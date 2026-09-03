// The Icon sets tab.
//
// The tab's whole reason to exist is showing nine sets *before* anything is
// downloaded, so the things worth testing are the ones that would quietly
// break that promise: a withdrawn theme vanishing instead of saying so, Apply
// being offered for art that is not on disk, and the picker listing sets you
// cannot actually switch to. None of those throw — they just leave a tab that
// looks fine and lies.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, iconsets, invoked, sets, failNext;

/// Four sets covering every state a card can be in: one downloaded and in use,
/// one available but not fetched, one that draws only wordmarks, and one gone
/// from the upstream list.
function fixture() {
  return [
    {
      name: "CodyWheel",
      dir: "codywheel-es-de",
      author: "Cody",
      variants: 3,
      icons: ["https://example.invalid/snes.png", "https://example.invalid/n64.png"],
      kinds: ["Hardware", "Styled text"],
      wordmarks_only: false,
      installed: 28,
      active: true,
      missing: false,
    },
    {
      name: "Diamond",
      dir: "diamond-es-de",
      author: "Dia",
      variants: 6,
      icons: ["https://example.invalid/snes.webp"],
      kinds: ["Hardware"],
      wordmarks_only: false,
      installed: 0,
      active: false,
      missing: false,
    },
    {
      name: "Razor",
      dir: "razor-es-de",
      author: "Wee",
      variants: 12,
      icons: ["https://example.invalid/snes.svg"],
      kinds: ["Styled text"],
      wordmarks_only: true,
      installed: 0,
      active: false,
      missing: false,
    },
    {
      name: "Meringue",
      dir: "",
      author: "",
      variants: 0,
      icons: [],
      kinds: [],
      wordmarks_only: false,
      installed: 0,
      active: false,
      missing: true,
    },
  ];
}

before(async () => {
  dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/" });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });

  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        if (cmd === "icon_sets") {
          if (failNext) {
            failNext = false;
            throw new Error("offline");
          }
          return sets;
        }
        if (cmd === "set_icon_set") return `set to ${args.dir}`;
        if (cmd === "install_icon_set") return `${args.dir}: 28 pictures`;
        if (cmd === "remove_icon_set") return `${args.dir} removed`;
        return null;
      },
      convertFileSrc: (p) => p,
    },
    event: { listen: async () => () => {}, emit: () => {} },
  };

  iconsets = await import(join(uiDir, "js/settings/iconsets.js"));
});

beforeEach(() => {
  invoked = [];
  sets = fixture();
  failNext = false;
});

/// Render the pane and wait for the async first draw to land.
async function render() {
  const box = dom.window.document.createElement("div");
  box.innerHTML = iconsets.html;
  dom.window.document.body.replaceChildren(box);
  iconsets.wire(box);
  await new Promise((r) => setTimeout(r, 0));
  return box;
}

describe("the icon sets tab", () => {
  test("draws a card for every set, including the one that is gone", async () => {
    const box = await render();
    assert.equal(box.querySelectorAll(".ic-card").length, 4);
    assert.equal(box.querySelectorAll(".ic-gone").length, 1, "the withdrawn set still appears");
    assert.match(box.querySelector(".ic-gone").textContent, /no longer in the ES-DE themes list/i);
  });

  test("shows each set's own console pictures without downloading anything", async () => {
    const box = await render();
    assert.equal(box.querySelectorAll(".ic-shots img").length, 4, "2 + 1 + 1 across the live sets");
    assert.equal(
      invoked.filter((i) => i.cmd === "install_icon_set").length,
      0,
      "opening the tab must fetch no theme",
    );
  });

  /// The whole reason the preview changed: screenshots showed the theme's
  /// interface, which is the one part of a theme this app never installs.
  test("the pictures come from the set's art, not from a screenshot field", async () => {
    const box = await render();
    const srcs = [...box.querySelectorAll(".ic-shots img")].map((i) => i.getAttribute("src"));
    assert.ok(
      srcs.every((s) => /snes|n64/.test(s)),
      `expected console art URLs, got ${srcs.join(", ")}`,
    );
  });

  /// Three of the nine draw only wordmarks. Finding that out by spending a
  /// download is the thing this warning exists to prevent.
  test("a wordmark-only set says so before you download it", async () => {
    const box = await render();
    const cards = [...box.querySelectorAll(".ic-card")];
    const razor = cards.find((c) => c.textContent.includes("Razor"));
    assert.match(razor.textContent, /names only/i);
    const iconic = cards.find((c) => c.textContent.includes("Diamond"));
    assert.doesNotMatch(iconic.textContent, /names only/i);
  });

  /// A set names its own looks and the card lists them, so what the Select
  /// button will cycle is visible before downloading.
  test("says which looks a set offers, in its own words", async () => {
    const box = await render();
    const cody = [...box.querySelectorAll(".ic-card")].find((c) =>
      c.textContent.includes("CodyWheel"),
    );
    assert.match(cody.textContent, /hardware/, "the looks are listed");
    assert.match(cody.textContent, /styled text/, "all of them, not just the first");
    assert.doesNotMatch(cody.textContent, /systemart/, "and never as a config key");
  });

  /// Apply on art that is not on disk would switch the grid to an empty folder.
  test("offers Download before it is fetched and Use after", async () => {
    const box = await render();
    const cards = [...box.querySelectorAll(".ic-card")];
    const downloaded = cards.find((c) => c.textContent.includes("CodyWheel"));
    const not = cards.find((c) => c.textContent.includes("Diamond"));

    assert.ok(downloaded.querySelector(".ic-apply"), "a downloaded set can be applied");
    assert.ok(downloaded.querySelector(".ic-remove"), "and removed");
    assert.equal(not.querySelector(".ic-apply"), null, "an unfetched set cannot be applied");
    assert.ok(not.querySelector(".ic-get"), "it offers a download instead");
  });

  test("the set in use is marked and cannot be re-applied", async () => {
    const box = await render();
    const on = box.querySelector(".ic-card.ic-on");
    assert.ok(on, "the active set is marked on the card, not only in the picker");
    assert.match(on.textContent, /CodyWheel/);
    assert.equal(on.querySelector(".ic-apply").disabled, true);
  });

  /// The picker switches what the grid draws, so an entry with nothing on disk
  /// would be a choice that silently does nothing.
  test("the picker lists only sets that are downloaded", async () => {
    const box = await render();
    const opts = [...box.querySelectorAll(".ic-active option")].map((o) => o.value);
    assert.deepEqual(opts, ["", "codywheel-es-de"]);
    assert.equal(box.querySelector(".ic-active").value, "codywheel-es-de", "and shows the active one");
  });

  test("downloading asks for that set and redraws", async () => {
    const box = await render();
    const get = box.querySelector(".ic-get");
    sets = fixture().map((s) =>
      s.dir === "diamond-es-de" ? { ...s, installed: 28 } : s,
    );
    get.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));

    const install = invoked.find((i) => i.cmd === "install_icon_set");
    assert.deepEqual(install?.args, { dir: "diamond-es-de" });
    assert.ok(
      invoked.filter((i) => i.cmd === "icon_sets").length >= 2,
      "the card has to redraw or it still says 'not downloaded'",
    );
  });

  test("choosing from the picker switches the grid", async () => {
    const box = await render();
    const picker = box.querySelector(".ic-active");
    picker.value = "";
    picker.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));
    assert.deepEqual(
      invoked.find((i) => i.cmd === "set_icon_set")?.args,
      { dir: "" },
      "an empty dir means the shared pool",
    );
  });

  /// The set list and the pictures both come from a compiled-in table now, so
  /// losing the network costs the authors' names, not the tab.
  test("says so when the set list cannot be read at all", async () => {
    failNext = true;
    const box = await render();
    assert.match(box.querySelector(".ic-grid").textContent, /could not reach/i);
  });

  test("the filter narrows the list without asking the backend again", async () => {
    const box = await render();
    const before = invoked.filter((i) => i.cmd === "icon_sets").length;
    const find = box.querySelector(".ic-find");
    find.value = "razor";
    find.dispatchEvent(new dom.window.Event("input", { bubbles: true }));

    const cards = [...box.querySelectorAll(".ic-card")];
    assert.equal(cards.length, 1, "one match");
    assert.match(cards[0].textContent, /Razor/);
    assert.equal(
      invoked.filter((i) => i.cmd === "icon_sets").length,
      before,
      "typing must not re-fetch the list",
    );
    assert.match(box.querySelector(".ic-count").textContent, /1 of 4/);
  });

  /// Fifteen of the published sets draw only wordmarks. Hiding them is the
  /// difference between a list worth scrolling and one that mostly is not.
  test("names-only sets can be hidden", async () => {
    const box = await render();
    const toggle = box.querySelector(".ic-nowords input");
    toggle.checked = true;
    toggle.dispatchEvent(new dom.window.Event("change", { bubbles: true }));

    const text = box.querySelector(".ic-grid").textContent;
    assert.doesNotMatch(text, /Razor/, "the wordmark-only set is gone");
    assert.match(text, /CodyWheel/, "the others stay");
  });

  test("a filter that matches nothing says so rather than going blank", async () => {
    const box = await render();
    const find = box.querySelector(".ic-find");
    find.value = "zzzz";
    find.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    assert.equal(box.querySelectorAll(".ic-card").length, 0);
    assert.match(box.querySelector(".ic-grid").textContent, /nothing matches/i);
  });

  test("a theme name with markup in it cannot inject into the card", async () => {
    sets = [
      {
        ...fixture()[0],
        name: '<img src=x onerror="window.__pwned=1">',
        author: "</div><script>window.__pwned=1</script>",
      },
    ];
    const box = await render();
    assert.equal(dom.window.__pwned, undefined);
    assert.equal(box.querySelectorAll(".ic-card").length, 1, "still one card, not a broken DOM");
  });
});

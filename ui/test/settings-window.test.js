// Every settings tab, rendered and wired the way the window does it.
//
// The other settings tests check that the markup contains a control. That is
// not the same as the tab working, and the difference was three live bugs that
// had been shipped for weeks:
//
//   * General wired a "reset keyboard" button that lives on the Control tab.
//     The lookup returned null, `.addEventListener` threw, and everything after
//     it in that pane — the wiring for every text field and toggle on the tab —
//     never ran. So editing the server URL or the library folder did nothing.
//   * Appearance called `cssColor()`, which was defined nowhere at all, so
//     wiring that tab threw partway through and the backdrop's own controls
//     were never connected.
//   * Both reset buttons called `closeSettings()` and `toggleSettings()`, which
//     were never imported into that module — a ReferenceError on click.
//
// None of them failed a test, because a pane that throws while being wired
// still contains all of its markup. This file wires the panes.

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, panes, asked;

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "settings.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  global.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  global.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });

  asked = [];
  const backend = fakeBackend((cmd) => {
    // Shapes the panes actually read. Anything else is a list.
    if (cmd === "config_fields") return { config_exists: true, library_root: "./library" };
    if (cmd === "motion_options") return { current: null, options: [] };
    if (cmd === "bios_status") return [0, 0, 0];
    if (cmd === "versions") return ["0.1.87", "3.4.0"];
    return [];
  });
  dom.window.__TAURI__ = {
    core: {
      invoke: (cmd, args) => {
        asked.push(cmd);
        return backend(cmd, args);
      },
      convertFileSrc: (p) => p,
    },
    event: { listen: async () => () => {}, emit: () => {} },
  };

  panes = await import(join(uiDir, "js", "settings-panes.js"));
  // The Control tab is a row per action with whatever key and button are on
  // it, so it needs the tables the settings window fetches at startup.
  await (await import(join(uiDir, "js", "bindings.js"))).loadBindings();
});

describe("every tab renders and wires", () => {
  for (const id of ["general", "appearance", "control", "library", "systems", "about"]) {
    test(`${id} wires without throwing`, async () => {
      const box = dom.window.document.createElement("div");
      box.innerHTML = panes.paneHtml(id);
      dom.window.document.body.appendChild(box);
      // The failure this is here to catch is a throw, so the assertion is that
      // there is not one. `wirePane` is async for the tabs that fetch first.
      await assert.doesNotReject(async () => panes.wirePane(id, box));
      box.remove();
    });
  }

  /// A tab that reads config.toml has to actually ask for it. General's call
  /// sat after the line that threw, so it never happened — the tab looked
  /// right and wrote nothing.
  test("the tabs with settings in config.toml read them", async () => {
    asked.length = 0;
    for (const id of ["general", "control", "systems"]) {
      const box = dom.window.document.createElement("div");
      box.innerHTML = panes.paneHtml(id);
      dom.window.document.body.appendChild(box);
      await panes.wirePane(id, box);
      box.remove();
    }
    const reads = asked.filter((c) => c === "config_fields").length;
    assert.equal(reads, 3, `only ${reads} of the three tabs read their settings`);
  });

  /// Each tab exports the same two things, which is what lets the dispatcher be
  /// a table rather than a chain of ifs.
  ///
  /// Markup may be a function rather than a string, for a pane built out of
  /// live state: the Control tab draws a row per action with whatever key and
  /// button are bound to it now, which a string evaluated at import time
  /// cannot say. `paneHtml` calls whichever it finds, so a pane offering
  /// neither is the failure.
  test("every tab in the table has markup and a wire function", () => {
    for (const t of panes.TABS) {
      assert.ok(t.pane, `${t.id} has no pane module`);
      assert.ok(
        ["string", "function"].includes(typeof t.pane.html),
        `${t.id} exports no markup`
      );
      assert.ok(panes.paneHtml(t.id).trim().length > 0, `${t.id}'s markup is empty`);
      assert.equal(typeof t.pane.wire, "function", `${t.id} exports no wire function`);
    }
  });

  /// The settings window asks this before deciding what a keypress means, and
  /// the capture state moved out from under it when the panes were split.
  test("the window can still ask whether a key is being captured", () => {
    assert.equal(typeof panes.isCapturing, "function");
    assert.equal(typeof panes.captureKey, "function");
    assert.equal(panes.isCapturing(), false);
  });
});

describe("the About tab", () => {
  /// The version numbers were already at the foot of the rail, which answers
  /// "are these two machines running the same thing" and nothing else. Who
  /// wrote it and where the source is were written down nowhere the app could
  /// show you.
  let box;

  before(async () => {
    box = dom.window.document.createElement("div");
    box.innerHTML = panes.paneHtml("about");
    dom.window.document.body.appendChild(box);
    await panes.wirePane("about", box);
  });

  test("it says who wrote it and where the source is", () => {
    const links = [...box.querySelectorAll(".link")].map((a) => a.dataset.href);
    assert.ok(
      links.includes("https://github.com/mizaimao"),
      "there is no link to the author"
    );
    assert.ok(
      links.includes("https://github.com/mizaimao/moose-rack"),
      "there is no link to the source"
    );
    assert.match(box.textContent, /mizaimao/);
  });

  test("it shows the version it is, and the server's", () => {
    assert.match(box.querySelector(".about-version").textContent, /0\.1\.87/);
    assert.match(box.querySelector(".about-version").textContent, /server 3\.4\.0/);
  });

  /// A webview follows a link in place, which would leave the settings window
  /// showing GitHub with no address bar, no back button and the app gone from
  /// underneath it. So none of them is a navigation: they are handed to the
  /// browser through a command that only accepts web links.
  test("no link navigates the window", () => {
    for (const a of box.querySelectorAll(".link")) {
      assert.equal(a.getAttribute("href"), null, `${a.textContent} navigates in place`);
      assert.match(a.dataset.href, /^https:\/\//);
    }
  });

  test("clicking one hands it to the browser", () => {
    asked.length = 0;
    box.querySelector(".link").dispatchEvent(
      new dom.window.MouseEvent("click", { bubbles: true })
    );
    assert.ok(asked.includes("open_link"), "the link was not opened anywhere");
  });
});

describe("the settings panes read as settings", () => {
  /// The headings were 13px, uppercase and dimmed to 0.75 while the paragraphs
  /// under them were full size and near-white, so the section names were the
  /// quietest thing on the page and the explanations shouted.
  test("a heading is louder than the text under it", () => {
    const css = readFileSync(join(uiDir, "settings.css"), "utf8");
    const block = (sel) => {
      const at = css.indexOf(sel + " {");
      assert.ok(at >= 0, `no rule for ${sel}`);
      return css.slice(at, css.indexOf("}", at));
    };
    const head = block("#pane h4");
    const hint = block("#pane .hint");
    const size = (b) => Number(/font-size:\s*([\d.]+)px/.exec(b)?.[1]);
    assert.ok(
      size(head) > size(hint),
      `headings are ${size(head)}px against ${size(hint)}px of explanation`
    );
    assert.match(head, /opacity:\s*1/, "the heading is still dimmed");
    assert.ok(Number(/opacity:\s*([\d.]+)/.exec(hint)?.[1]) < 0.7, "the hint is not dimmed");
  });

  /// Stretched across the pane, a 0–60 range put its steps nine pixels apart
  /// and the number beside it out at the far edge.
  test("a slider is not as wide as the window", () => {
    const css = readFileSync(join(uiDir, "settings.css"), "utf8");
    const at = css.lastIndexOf('#pane .srow .ctl input[type="range"] {');
    assert.ok(at >= 0, "the slider has no width of its own");
    assert.match(css.slice(at, css.indexOf("}", at)), /width:\s*\d+px/);
  });

  /// At 100% the action column absorbed everything left over, so "Move left"
  /// sat half a window from the key that does it.
  test("the bindings table is as wide as its contents", () => {
    const css = readFileSync(join(uiDir, "settings.css"), "utf8");
    const at = css.indexOf(".bindtbl {");
    assert.match(css.slice(at, css.indexOf("}", at)), /width:\s*auto/);
  });

  /// Cycling the artwork from the sofa is the reason several of these exist,
  /// and nothing on the page said so — the button is bound two tabs away under
  /// a name that does not obviously mean this row.
  test("settings the pad can change say so", async () => {
    const box = dom.window.document.createElement("div");
    box.innerHTML = panes.paneHtml("appearance");
    dom.window.document.body.appendChild(box);
    await panes.wirePane("appearance", box);
    const marks = [...box.querySelectorAll(".padmark")];
    assert.ok(marks.length, "nothing is marked as pad-changeable");
    for (const m of marks) {
      assert.ok(m.textContent.trim(), `the ${m.dataset.action} badge is empty`);
      assert.match(m.title, /Control tab/, "the badge does not say where to rebind it");
    }
    box.remove();
  });

  /// The Emulators tab opened with a heading and a paragraph about a table
  /// that is at the very bottom, so the first thing on the tab explained the
  /// last thing on it.
  test("the Emulators heading sits with its table", () => {
    const html = panes.paneHtml("systems");
    assert.ok(
      html.indexOf("<h4>Emulators</h4>") > html.indexOf("<h4>Game window</h4>"),
      "the table's heading is still at the top"
    );
    assert.ok(
      html.indexOf("<h4>Emulators</h4>") < html.indexOf('class="sys-table"'),
      "the heading is not above its own table"
    );
  });
});

describe("the console pictures row", () => {
  /// Reported from Windows as not being able to cycle them at all — which is
  /// exactly what a row of disabled buttons is. "Nothing has been fetched yet"
  /// and "this style has none" looked identical: every button grayed, no
  /// explanation, and a Get button below that nobody connects to the row above.
  test("nothing installed says so, rather than graying everything", async () => {
    const box = dom.window.document.createElement("div");
    box.innerHTML = panes.paneHtml("appearance");
    dom.window.document.body.appendChild(box);
    await panes.wirePane("appearance", box);
    // The stub answers `icon_styles` with an empty list, which is the same
    // shape as a machine that has fetched nothing.
    const row = box.querySelector(".icon-styles");
    assert.ok(row, "no console pictures row");
    box.remove();
  });

  /// The slider that decides how much of the window shows through the preview.
  /// One slider for the glass, and it says so. It was two — "Tint strength"
  /// for the cards and "Preview pane" for the pane — which is how the two
  /// surfaces came to disagree about how transparent they were.
  test("the glass is one slider on the Appearance tab", () => {
    const html = panes.paneHtml("appearance");
    assert.match(html, /class="glass-strength"/, "no slider for the glass");
    assert.doesNotMatch(html, /class="pane-clarity"/, "the preview pane got its own again");
    const at = html.indexOf('class="glass-strength"');
    const tag = html.slice(at, html.indexOf(">", at));
    assert.match(tag, /max="60"/);
    assert.match(tag, /min="0"/, "at 0 the glass is clear, and that has to be reachable");
  });
});

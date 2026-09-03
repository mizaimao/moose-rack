// Search inside the page you are looking at.
//
// Not the header's search box, which spans the library and takes you to a
// different screen. This one narrows what is already in front of you and
// leaves you where you are. It used to be drawn into the collections list —
// which is why it existed only there, and why it sat in the middle of a page
// of cards looking like something that had fallen out of a dialog.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, pf, el, tabs;

before(async () => {
  dom = new JSDOM(
    `<style>${readFileSync(join(uiDir, "style.css"), "utf8")}</style>` +
      readFileSync(join(uiDir, "index.html"), "utf8"),
    { url: "http://localhost/", pretendToBeVisual: true }
  );
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  global.CSS = dom.window.CSS;
  global.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  global.requestAnimationFrame = (f) => f();
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });
  dom.window.Element.prototype.scrollIntoView = function () {};
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  dom.window.IntersectionObserver = FakeObserver;
  global.IntersectionObserver = FakeObserver;
  dom.window.__TAURI__ = {
    core: {
      // Drawing a list selects a row, which paints the preview — and that
      // joins several of these arrays. A thinner stub throws after the test
      // has finished, as an unhandled rejection rather than a failure.
      invoke: async (cmd, args) =>
        cmd === "rom_detail"
          ? {
              id: args.id,
              name: `Game ${args.id}`,
              fs_name: "g.zip",
              platform: "gba",
              platform_slug: "gba",
              size_bytes: 1,
              downloaded: true,
              screenshots: [],
              genres: [],
              companies: [],
              franchises: [],
              game_modes: [],
              regions: [],
              alt_names: [],
              art: {},
            }
          : [],
      convertFileSrc: (p) => p,
    },
    event: { listen: async () => () => {}, emit: () => {} },
  };

  // The interface commands — bindings, ordering, the grid, the page filter —
  // are answered by the stand-in in backend.js. See the note at the top of
  // that file: it is deliberately naive, and the rules it stands in for are
  // asserted by `cargo test` against the real implementation.
  dom.window.__TAURI__.core.invoke = fakeBackend(dom.window.__TAURI__.core.invoke);
  pf = await import("../js/pagefilter.js");
  tabs = await import("../js/tabs.js");
  ({ el } = await import("../js/state.js"));
});

beforeEach(() => {
  el.list.innerHTML = `
    <div class="rows">
      <div class="row prow" data-slug="arcade"><span class="nm">Arcade</span></div>
      <div class="row prow" data-slug="gb"><span class="nm">Game Boy</span></div>
      <div class="row prow" data-slug="gba"><span class="nm">Game Boy Advance</span></div>
    </div>`;
  el.consoles.hidden = true;
  pf.clearPageFilter();
});

const visible = () =>
  [...el.list.querySelectorAll(".row")]
    .filter((n) => !n.classList.contains("filtered-out"))
    .map((n) => n.textContent.trim());

describe("filtering the page", () => {
  test("it keeps what matches and hides the rest", async () => {
    await pf.applyPageFilter("game boy");
    assert.deepEqual(visible(), ["Game Boy", "Game Boy Advance"]);
  });

  test("case does not matter, and neither does stray space", async () => {
    await pf.applyPageFilter("  ARCADE ");
    assert.deepEqual(visible(), ["Arcade"]);
  });

  test("emptying it puts everything back", async () => {
    await pf.applyPageFilter("arcade");
    pf.clearPageFilter();
    assert.equal(visible().length, 3);
  });

  /// A redraw builds fresh nodes that have never seen the filter. Without
  /// this, changing the order or coming back to a tab quietly undoes the
  /// search still sitting in the box.
  test("a redrawn list is filtered again", async () => {
    await pf.applyPageFilter("advance");
    assert.deepEqual(visible(), ["Game Boy Advance"]);
    el.list.innerHTML = `
      <div class="rows">
        <div class="row prow"><span class="nm">Arcade</span></div>
        <div class="row prow"><span class="nm">Game Boy Advance</span></div>
      </div>`;
    assert.equal(visible().length, 2, "the redraw should start unfiltered");
    await pf.refreshPageFilter();
    assert.deepEqual(visible(), ["Game Boy Advance"]);
  });

  /// Search is per screen. Text left in the box on the way to another tab
  /// means arriving at a list that is missing things for no visible reason.
  test("changing view clears it", async () => {
    const shell = await import("../js/shell.js");
    await pf.applyPageFilter("arcade");
    shell.enter({ title: "History" });
    assert.equal(pf.pageFilterText(), "");
    assert.equal(el.pageFilter.value, "");
  });

  test("it says what this page holds", () => {
    pf.setPageFilterLabel("27 collections");
    assert.match(el.pageFilter.placeholder, /27 collections/);
  });
});

describe("where the box sits", () => {
  /// The tab row runs the full width of the window and the preview is a column
  /// under its right-hand end, so left alone the box sits over the top of that
  /// column — text on one side of the seam, artwork on the other.
  test("it is in the tab row, before the two end buttons", () => {
    tabs.installTabs();
    const bar = dom.window.document.getElementById("page-filter");
    assert.equal(bar.parentElement?.id, "tabbar", "the box is not in the tab row");
    assert.ok(
      bar.nextElementSibling?.classList.contains("tabbar-end"),
      "Take offline and Hide info should be after it, at the very end"
    );
  });

  /// All the slack in the row is the box's left margin, so the box and the two
  /// buttons travel together against the right-hand edge. An auto margin on
  /// the buttons as well would split the slack between them and leave the box
  /// stranded in the middle of the row, which is where it was.
  test("the slack is in front of it, not between it and the buttons", () => {
    const css = readFileSync(join(uiDir, "style.css"), "utf8");
    const rule = (sel) => {
      const at = css.indexOf(sel + " {");
      assert.ok(at >= 0, `no rule for ${sel}`);
      return css.slice(at, css.indexOf("}", at));
    };
    assert.match(rule("#tabbar #page-filter"), /margin-left:\s*auto/);
    assert.doesNotMatch(
      rule("#tabbar .tabbar-end"),
      /margin-left:\s*auto/,
      "the buttons take half the slack, which strands the box mid-row"
    );
  });

  /// It does not dodge the preview column any more — that was the previous
  /// arrangement, and it put the box a long way from the edge on a wide
  /// window.
  test("nothing sets a right margin on it", () => {
    tabs.installTabs();
    const bar = dom.window.document.getElementById("page-filter");
    assert.equal(bar.style.marginRight, "", "something is still holding it off the edge");
  });
});

describe("pages with nothing to search", () => {
  /// History is three charts and the top of RomM browse is five groups you can
  /// read at a glance. A search box over either is a control that does
  /// nothing, which is worse than no control.
  let shell;

  before(async () => (shell = await import("../js/shell.js")));

  test("History has no filter box", () => {
    shell.enter({ title: "History", filter: false });
    assert.equal(el.pageFilterBar.hidden, true);
  });

  test("and a list brings it back", () => {
    shell.enter({ title: "Arcade", sort: true });
    assert.equal(el.pageFilterBar.hidden, false);
  });

  /// The same field as the search box above it. Two boxes that take typed
  /// text, one above the other, should not read as two different kinds of
  /// control — and the base `.filter` style is a solid panel meant for a box
  /// drawn into a list, which this one no longer is.
  test("the box matches the search box", () => {
    const css = readFileSync(join(uiDir, "style.css"), "utf8");
    const rule = (sel, last = false) => {
      const at = last ? css.lastIndexOf(sel + " {") : css.indexOf(sel + " {");
      assert.ok(at >= 0, `no rule for ${sel}`);
      return css.slice(at, css.indexOf("}", at));
    };
    const bg = (block) => /background:\s*([^;]+)/.exec(block)?.[1]?.trim();
    assert.equal(
      bg(rule("#tabbar #page-filter .filter", true)),
      bg(rule("#search")),
      "the two text boxes are painted differently"
    );
  });
});

describe("what the marks mean", () => {
  /// A symbol that appears on some cards and not others, and explains itself
  /// nowhere, is a symbol that makes people guess. Every mark the lists draw
  /// is named where it is drawn and listed on the help page.
  let library, state, keys;

  before(async () => {
    library = await import("../js/library.js");
    keys = await import("../js/keys.js");
    ({ state } = await import("../js/state.js"));
  });

  test("both states are drawn, and both say what they are", () => {
    state.layout = "grid";
    state.view = "roms";
    state.rows = [];
    library.renderRows(
      [
        { id: 1, name: "Here", size_bytes: 1, downloaded: true, platform: "gba" },
        { id: 2, name: "There", size_bytes: 1, downloaded: false, platform: "gba" },
      ],
      false
    );
    const marks = [...el.list.querySelectorAll(".mark")];
    assert.equal(marks.length, 2, "a game with no mark is a game with no answer");
    assert.match(marks[0].title, /On this machine/);
    assert.match(marks[1].title, /On the server/);
  });

  test("the help page lists them", () => {
    document.getElementById("shortcuts")?.remove();
    keys.runAction("help");
    const help = document.getElementById("shortcuts");
    assert.ok(help, "no help page");
    const text = help.textContent;
    for (const line of ["On this machine", "On the server", "Starred", "emulator"]) {
      assert.match(text, new RegExp(line), `the legend does not explain "${line}"`);
    }
    help.remove();
  });
});

describe("which section you are in", () => {
  /// Two attempts got this wrong the same way. Sticky and frosted, the heading
  /// floated over a game's cover; sticky and opaque, it still covered the row
  /// above it — a sticky heading is over the content by construction, and
  /// paint does not change that. Not sticky at all, and a console with more
  /// than a screenful of games loses its name entirely.
  ///
  /// The third way is to give it its own space: a strip above the list, out of
  /// the part that scrolls, with nothing underneath it to cover.
  test("the heading in the list does not float", () => {
    const css = readFileSync(join(uiDir, "style.css"), "utf8");
    const at = css.indexOf(".ghead {");
    const rule = css.slice(at, css.indexOf("}", at));
    assert.doesNotMatch(rule, /position:\s*sticky/, "it is over the content again");
    assert.doesNotMatch(rule, /background:/, "it is painting over what is behind it");
  });

  test("the strip is outside the list, with room of its own", () => {
    const css = readFileSync(join(uiDir, "style.css"), "utf8");
    const at = css.indexOf("#section-strip {");
    assert.ok(at >= 0, "there is no strip");
    const rule = css.slice(at, css.indexOf("}", at));
    assert.match(rule, /height:\s*\d+px/, "with no height of its own it would overlay again");
    assert.doesNotMatch(rule, /position:\s*(sticky|absolute|fixed)/);
    // In the page's own column, above <main>, so the list starts below it.
    const html = readFileSync(join(uiDir, "index.html"), "utf8");
    assert.ok(
      html.indexOf('id="section-strip"') < html.indexOf("<main>"),
      "the strip is inside the scrolling part"
    );
  });

  test("it names the section that has scrolled past, and nothing before that", async () => {
    const sections = await import("../js/sections.js");
    el.list.innerHTML = `
      <section class="pgroup"><div class="ghead">SNES <span class="gcount">4</span></div></section>
      <section class="pgroup"><div class="ghead">NES <span class="gcount">9</span></div></section>`;
    const heads = [...el.list.querySelectorAll(".ghead")];
    el.list.getBoundingClientRect = () => ({ top: 100 });
    // Both below the top of the list: nothing has been passed yet.
    heads[0].getBoundingClientRect = () => ({ top: 140 });
    heads[1].getBoundingClientRect = () => ({ top: 600 });
    sections.followSections();
    assert.equal(el.sectionStrip.textContent, "", "it named a section still on screen");
    assert.equal(el.sectionStrip.hidden, false, "the room for it should be kept");

    // Scrolled into the first section.
    heads[0].getBoundingClientRect = () => ({ top: 40 });
    el.list.dispatchEvent(new dom.window.Event("scroll"));
    assert.match(el.sectionStrip.textContent, /SNES/);

    // And into the second.
    heads[1].getBoundingClientRect = () => ({ top: 60 });
    el.list.dispatchEvent(new dom.window.Event("scroll"));
    assert.match(el.sectionStrip.textContent, /NES/);
  });

  /// A console's own game list is one section, and a strip naming it would say
  /// what the title bar already says.
  test("a list with one section has no strip", async () => {
    const sections = await import("../js/sections.js");
    el.list.innerHTML = `<div class="ghead">SNES</div>`;
    sections.followSections();
    assert.equal(el.sectionStrip.hidden, true);
  });
});

describe("the tab bar", () => {
  /// A tenth taller, which is the difference between a row of labels and a row
  /// you aim at.
  test("it is 42px", () => {
    const css = readFileSync(join(uiDir, "style.css"), "utf8");
    const at = css.indexOf("#tabbar {");
    assert.match(css.slice(at, css.indexOf("}", at)), /height:\s*42px/);
  });
});

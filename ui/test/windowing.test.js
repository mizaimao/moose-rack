// A long list draws only the band around the viewport.
//
// jsdom has no layout, so the page is given one: cards report a height, the
// grid reports its columns, and the list reports how much of itself is on
// screen. Without that every measurement is zero, the window decides there is
// nothing to window, and the whole thing quietly does not happen — which is
// also what it does in a real browser if the measuring ever breaks, so the
// first test here is that it is happening at all.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

const COLUMNS = 10;
const CARD_HEIGHT = 200;
const VIEWPORT = 800;
const TOTAL = 2506;

let dom, lib, state, el, visible, backend, keys;

const ROWS = Array.from({ length: TOTAL }, (_, i) => ({
  id: i + 1,
  name: `Game ${String(i).padStart(4, "0")}`,
  fs_name: `g${i}.zip`,
  platform: "arcade",
  size_bytes: 1,
  downloaded: true,
  favorite: false,
  rating: null, year: null, last_played: null, players: null,
}));

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/", pretendToBeVisual: true,
  });
  for (const k of ["window", "document", "localStorage", "CSS"])
    Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true });
  Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
  globalThis.requestAnimationFrame = (f) => dom.window.setTimeout(f, 0);
  dom.window.Element.prototype.scrollIntoView = function () {};
  class FO { observe() {} unobserve() {} disconnect() {} }
  dom.window.IntersectionObserver = FO;
  globalThis.IntersectionObserver = FO;

  // The layout jsdom does not have.
  Object.defineProperty(dom.window.HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return this.classList?.contains("gcard") || this.classList?.contains("row")
        ? CARD_HEIGHT
        : 0;
    },
  });
  Object.defineProperty(dom.window.HTMLElement.prototype, "offsetTop", {
    configurable: true, get: () => 0,
  });
  Object.defineProperty(dom.window.HTMLElement.prototype, "clientHeight", {
    configurable: true, get: () => VIEWPORT,
  });
  const real = dom.window.getComputedStyle.bind(dom.window);
  dom.window.getComputedStyle = (node, ...rest) => {
    const style = real(node, ...rest);
    if (node?.classList?.contains("gcards")) {
      return new Proxy(style, {
        get: (t, k) =>
          k === "gridTemplateColumns"
            ? Array.from({ length: COLUMNS }, () => "150px").join(" ")
            : k === "rowGap"
              ? "0px"
              : Reflect.get(t, k),
      });
    }
    return style;
  };
  globalThis.getComputedStyle = dom.window.getComputedStyle;

  backend = fakeBackend((cmd, args) => {
    if (cmd === "rom_covers") return [];
    if (cmd === "rom_detail")
      return { id: args.id, name: "x", fs_name: "g.zip", platform: "arcade",
               platform_slug: "arcade", size_bytes: 1, downloaded: true, screenshots: [],
               genres: [], companies: [], franchises: [], game_modes: [], regions: [],
               alt_names: [], art: {} };
    return [];
  });
  dom.window.__TAURI__ = {
    core: { invoke: backend, convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: () => {} },
  };
  backend.rows(ROWS);

  lib = await import("../js/library.js");
  visible = await import("../js/visible.js");
  keys = await import("../js/keys.js");
  ({ state, el } = await import("../js/state.js"));
  await (await import("../js/arrange.js")).arrangeCurrentList();
});

beforeEach(() => {
  state.view = "roms";
  state.platform = "arcade";
  state.layout = "grid";
  state.rows = ROWS;
  el.list.scrollTop = 0;
  lib.renderRows(ROWS, false);
});

const drawn = () => document.querySelectorAll("#list .gcard").length;
const spacers = () => [...document.querySelectorAll("#list .vspace")].map((n) => n.style.height);
/// A few turns of the loop. Moving the cursor asks the backend for the table
/// the first time, so the move lands a tick or two after the key.
const settle = async () => {
  for (let i = 0; i < 4; i++) await new Promise((r) => dom.window.setTimeout(r, 0));
};

/// Move the list, the way a wheel or a scrollbar would.
const scrollTo = (px) => {
  el.list.scrollTop = px;
  el.list.dispatchEvent(new dom.window.Event("scroll"));
};

describe("a two-thousand-row console", () => {
  /// The measurement is what makes any of this work, and a page where it comes
  /// back zero silently draws everything — which is the bug this whole file
  /// exists to catch.
  test("only a band of it is in the document", () => {
    assert.ok(drawn() > 0, "nothing was drawn at all");
    assert.ok(drawn() < TOTAL / 4, `${drawn()} of ${TOTAL} cards are in the document`);
  });

  test("the rows it is not drawing are stood in for", () => {
    scrollTo(20_000);
    const [before, after] = spacers();
    assert.ok(Number.parseFloat(before) > 0, "nothing holds the space above the band");
    assert.ok(Number.parseFloat(after) > 0, "nothing holds the space below the band");
  });

  /// The scrollbar has to be what it would have been with every row drawn, or
  /// the list jumps under the thumb.
  test("the page is the height it would have been", () => {
    for (const at of [0, 20_000, 45_000]) {
      scrollTo(at);
      const [before, after] = spacers().map(Number.parseFloat);
      const band = Math.ceil(drawn() / COLUMNS) * CARD_HEIGHT;
      const rows = Math.ceil(TOTAL / COLUMNS);
      assert.equal(before + band + after, rows * CARD_HEIGHT, `wrong height at ${at}`);
    }
  });

  test("scrolling draws what you scrolled to", () => {
    scrollTo(30_000);
    const at = [...document.querySelectorAll("#list .gcard")].map((n) => Number(n.dataset.at));
    const wanted = Math.floor(30_000 / CARD_HEIGHT) * COLUMNS;
    assert.ok(Math.min(...at) <= wanted && wanted <= Math.max(...at),
      `scrolled to row ${wanted}, drew ${Math.min(...at)}..${Math.max(...at)}`);
  });

  /// A short list is drawn whole — a window over it is machinery with nothing
  /// to do, and every existing behavior has to survive that path unchanged.
  test("a short list is left alone entirely", () => {
    const few = ROWS.slice(0, 50);
    backend.rows(few);
    state.rows = few;
    lib.renderRows(few, false);
    assert.equal(drawn(), 50);
    assert.equal(spacers().length, 0, "a short list was given spacers");
  });
});

describe("the cursor, through rows that are not drawn", () => {
  /// The whole reason navigation stopped measuring the page: most of the cards
  /// have no position, because they do not exist.
  test("it reaches the end of the list", async () => {
    lib.renderRows(ROWS, false);
    keys.HANDLERS.last();
    await settle();
    assert.equal(state.selected, ROWS[TOTAL - 1].id, "the end of the list is unreachable");
    assert.ok(document.querySelector(`.gcard[data-id="${ROWS[TOTAL - 1].id}"]`),
      "the last row was selected but never drawn");
  });

  test("and comes back to the front", async () => {
    keys.HANDLERS.last();
    await settle();
    keys.HANDLERS.first();
    await settle();
    assert.equal(state.selected, ROWS[0].id);
  });

  /// Down moves one row, which on a ten-wide grid is ten games.
  test("down moves a whole row", async () => {
    lib.renderRows(ROWS, false);
    const from = state.selected;
    keys.HANDLERS.down();
    await settle();
    const fromAt = ROWS.findIndex((r) => r.id === from);
    const toAt = ROWS.findIndex((r) => r.id === state.selected);
    assert.equal(toAt - fromAt, COLUMNS, `moved ${toAt - fromAt} games, not a row`);
  });

  /// The card carrying the highlight is thrown away every time the band moves.
  /// The cursor is a row, not a node.
  test("the highlight survives the band moving under it", async () => {
    keys.HANDLERS.last();
    await settle();
    const marked = document.querySelectorAll("#list .gcard.sel");
    assert.equal(marked.length, 1, `${marked.length} cards are highlighted`);
    assert.equal(Number(marked[0].dataset.id), state.selected);
  });
});

describe("the filter box over a windowed list", () => {
  /// Putting a class on the drawn nodes would search a few hundred games out
  /// of two and a half thousand — a filter that finds less the further down
  /// you have scrolled.
  test("it searches the whole list, not the band", async () => {
    const pf = await import("../js/pagefilter.js");
    const shown = await pf.applyPageFilter("Game 24");
    // Game 2400 through 2499 — a hundred rows, every one of them far past the
    // band that was drawn.
    assert.equal(shown, 100, `matched ${shown}`);
    assert.equal(visible.windowedList().total, 100, "the window kept rows the filter dropped");
  });

  test("clearing it puts the whole list back", async () => {
    const pf = await import("../js/pagefilter.js");
    await pf.applyPageFilter("Game 24");
    pf.clearPageFilter();
    assert.equal(visible.windowedList().total, TOTAL);
  });

  /// The cursor moves through what the filter left and nothing else.
  test("the cursor stays inside what is left", async () => {
    const pf = await import("../js/pagefilter.js");
    await pf.applyPageFilter("Game 24");
    keys.HANDLERS.last();
    await settle();
    const last = visible.windowedList().at(visible.windowedList().total - 1);
    assert.equal(state.selected, last.id, "the end of the filtered list is not its end");
  });
});

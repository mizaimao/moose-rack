// What acts on "the list on screen" has to mean the list that is on screen.
//
// `state.rows` and the cached arrangement are one shared answer to that
// question, and every screen that is not a list of games left them describing
// whichever console was open last. The sort, the filter, Random and the
// redraws Settings asks for all read them, so each acted on a list nobody
// could see.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, state, el, library, sort, filter, history, pictures;

const SNES = [
  { id: 1, name: "Zelda", platform: "snes", size_bytes: 1, downloaded: true },
  { id: 2, name: "Mario", platform: "snes", size_bytes: 1, downloaded: true },
];

/// A turn of the event loop, for handlers that redraw when the backend answers.
const settle = () => new Promise((r) => setTimeout(r, 0));

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
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

  const reply = async (cmd, args) => {
    if (cmd === "roms") return SNES.map((r) => ({ ...r }));
    if (cmd === "platforms") return [{ slug: "snes", name: "SNES", rom_count: 2, playable: true }];
    if (cmd === "play_history") return { sessions: 0 };
    if (cmd === "set_list_art") return "Miximages";
    if (cmd === "rom_detail") {
      return {
        id: args.id, name: `Game ${args.id}`, fs_name: "g.zip", platform: "snes",
        platform_slug: "snes", size_bytes: 1, downloaded: true, screenshots: [],
        genres: [], companies: [], franchises: [], game_modes: [], regions: [],
        alt_names: [], art: {},
      };
    }
    return [];
  };
  dom.window.__TAURI__ = {
    core: { invoke: fakeBackend(reply), convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: () => {} },
  };

  ({ state, el } = await import("../js/state.js"));
  library = await import("../js/library.js");
  sort = await import("../js/sort.js");
  filter = await import("../js/filter.js");
  history = await import("../js/history.js");
  pictures = await import("../js/pictures.js");
  // The tables the menus are built from, which the app fetches at startup.
  filter.setFilters((await sort.loadListControls()).filters);
});

beforeEach(() => {
  document.querySelector("#sort-menu")?.remove();
  document.querySelector(".ctx-menu")?.remove();
  state.collection = null;
});

describe("a screen that is not a list of games", () => {
  /// Open SNES, back out to the consoles, press S: the console grid was
  /// replaced by SNES's games under the title "Platforms".
  test("the console grid has no list left over to sort, filter or pick from", async () => {
    await library.showRoms("snes");
    assert.equal(state.rows.length, 2, "the console did not open");

    await library.showPlatforms();
    assert.deepEqual(state.rows, [], "SNES's games are still held as the list on screen");

    sort.openSortMenu();
    assert.equal(document.querySelector("#sort-menu"), null, "the sort menu opened over the consoles");
    assert.equal(await sort.cycleOrder(1), null, "the sort cycled a list that is not there");
    filter.openFilterMenu();
    assert.equal(document.querySelector(".ctx-menu"), null, "the filter menu opened over the consoles");
    assert.equal(library.randomGame(), null, "Random picked a game nobody can see");
    await settle();
    assert.ok(el.list.querySelector("[data-slug]"), "the consoles were drawn over");
  });

  /// Settings can change the game art or the Games layout while History is
  /// open, and each redraw painted the last console's games over the page.
  test("History is not painted over by a redraw meant for a list", async () => {
    await library.showRoms("snes");
    await history.showHistory();
    assert.deepEqual(state.rows, []);

    library.setLayout(state.layout === "grid" ? "list" : "grid");
    await pictures.useMiximage();
    await settle();
    assert.match(el.list.textContent, /Nothing recorded yet/, "History was replaced by a game list");
    assert.equal(el.list.querySelector("[data-id]"), null, "a game was drawn over History");
  });
});

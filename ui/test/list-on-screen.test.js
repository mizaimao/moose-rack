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

let dom, backend, state, el, library, sort, filter, history, pictures, collections, keys, detail;

/// Lets the held-back search answer.
let releaseSlow = () => {};

/// What `play_history` answers. Nothing recorded, unless a test says so.
let played = { sessions: 0 };

const SNES = [
  { id: 1, name: "Zelda", platform: "snes", size_bytes: 1, downloaded: true },
  { id: 2, name: "Mario", platform: "snes", size_bytes: 1, downloaded: true },
];

/// Games from two consoles: a collection, or what was played last.
const MIXED = [
  { id: 10, name: "Metroid", platform: "snes", size_bytes: 1, downloaded: true },
  { id: 11, name: "Golden Sun", platform: "gba", size_bytes: 1, downloaded: true },
  { id: 12, name: "Kirby", platform: "snes", size_bytes: 1, downloaded: true },
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
    if (cmd === "play_history") return played;
    if (cmd === "set_list_art") return "Miximages";
    if (cmd === "collection_roms") return MIXED.map((r) => ({ ...r }));
    if (cmd === "search") {
      // "slow" answers only when the test lets it, so a later search can
      // overtake it.
      if (args.term === "slow") await new Promise((done) => (releaseSlow = done));
      return [{ id: 100, name: args.term, platform: "snes", size_bytes: 1, downloaded: true }];
    }
    // The strip on the console screen asks for a handful; the page behind
    // its More button asks for 500.
    if (cmd === "recent_games") return args.limit === 500 ? MIXED.map((r) => ({ ...r })) : [];
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
  backend = fakeBackend(reply);
  dom.window.__TAURI__ = {
    core: { invoke: backend, convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: () => {} },
  };

  ({ state, el } = await import("../js/state.js"));
  library = await import("../js/library.js");
  sort = await import("../js/sort.js");
  filter = await import("../js/filter.js");
  history = await import("../js/history.js");
  pictures = await import("../js/pictures.js");
  collections = await import("../js/collections.js");
  keys = await import("../js/keys.js");
  detail = await import("../js/detail.js");
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

describe("a redraw keeps the shape the list was opened in", () => {
  const sections = () => el.list.querySelectorAll(".pgroup").length;

  /// Every redraw passed "is this a search" as whether to group, and
  /// Continue playing is a search that opens ungrouped, most recent first.
  /// Switching grid and list regrouped it by console and lost that order.
  test("Continue playing stays one list when the layout changes", async () => {
    await library.showAllRecent();
    assert.equal(sections(), 0, "Continue playing opened grouped");
    library.setLayout(state.layout === "grid" ? "list" : "grid");
    assert.equal(sections(), 0, "switching the layout grouped it by console");
  });

  /// And a collection opens grouped by console without being a search, so
  /// changing its order flattened it.
  test("a collection stays grouped when the order changes", async () => {
    await collections.showCollectionRoms("c1", "Mixed");
    assert.equal(sections(), 2, "the collection did not open grouped");
    // Selecting the first row asks the stand-in backend for lists of its
    // own, and it would take one of those for this list. Told again here.
    await settle();
    backend.rows(MIXED);
    await sort.cycleOrder(1);
    await settle();
    await settle();
    assert.equal(sections(), 2, "changing the order dropped the grouping");
  });
});

describe("the search box", () => {
  /// Emptying the box went to the open console or the console grid, whichever
  /// tab the search was typed in.
  test("emptying it goes back to the screen the search was typed over", async () => {
    await collections.showCollectionsIn("user", "My collections");
    await library.runSearch("zelda");
    assert.equal(state.view, "search");
    await library.runSearch("");
    assert.equal(state.view, "collections", "it left My collections");
    assert.match(el.title.textContent, /My collections/);
  });

  /// So does Back, one level at a time: the search, then the console.
  test("Back from a search goes to where it was typed", async () => {
    await library.showRoms("snes");
    await library.runSearch("zelda");
    // The pad's and the keyboard's Back; the header button runs the same walk.
    keys.runAction("back");
    await settle();
    await settle();
    assert.equal(state.view, "roms", "Back skipped the console the search was typed in");
    assert.equal(state.platform, "snes");
  });

  /// Answers can land out of order, and the late one was drawn over the
  /// search that replaced it.
  test("an answer overtaken by a later search is dropped", async () => {
    await library.showPlatforms();
    const slow = library.runSearch("slow");
    await settle();
    await library.runSearch("fast");
    assert.match(el.title.textContent, /fast/);
    releaseSlow();
    await slow;
    assert.match(el.title.textContent, /fast/, "the older search was drawn over the newer one");
    await library.runSearch("");
  });

  /// Escape emptied the box and went back, and the search waiting on the
  /// 200 ms timer then arrived anyway, under an empty box.
  test("Escape stops a search that is still waiting to run", async () => {
    await library.showPlatforms();
    keys.installKeys();
    el.search.value = "zelda";
    library.searchSoon("zelda");
    el.search.focus();
    el.search.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(state.view, "platforms", "the search ran after Escape");
    assert.equal(el.search.value, "");
  });
});

describe("History and the preview pane", () => {
  /// The game last selected stayed in the pane beside History, Play button
  /// and all, and in one pane the toggle that would close it was disabled.
  test("the last game's pane does not stay beside History", async () => {
    await library.showRoms("snes");
    await detail.selectRom(1);
    assert.ok(el.detail.querySelector("#play"), "the game never reached the pane");

    await history.showHistory();
    assert.equal(el.detail.hidden, true, "the last game's pane is still beside History");
    assert.equal(el.detail.querySelector("#play"), null, "and its Play button with it");
    assert.equal(state.selected, null, "a double-click on the page would still play it");
  });

  /// The games History lists can be picked, which opens the pane, so the
  /// toggle has to be able to put it away and bring it back.
  test("a game picked on History opens a pane the toggle can close", async () => {
    played = {
      sessions: 3,
      total_seconds: 7200,
      games: 1,
      platforms: [{ name: "SNES", seconds: 7200, spelled: "2h" }],
      top: [{ id: 1, name: "Zelda", platform: "snes", spelled: "2h" }],
      abandoned: [],
    };
    try {
      await history.showHistory();
      assert.equal(el.sidebarBtn.disabled, false, "the toggle is dead on History");
      await detail.selectRom(1);
      detail.setSidebar(false, { remember: false });
      assert.equal(el.detail.hidden, true, "the toggle did not close the pane");
      detail.setSidebar(true, { remember: false });
      assert.equal(el.detail.hidden, false, "the toggle could not bring it back");
    } finally {
      played = { sessions: 0 };
    }
  });
});

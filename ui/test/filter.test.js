// Narrowing a list, and picking out of it at random.
//
// The arcade console holds 2,506 games. Sorting them is not finding one: "by
// rating" still leaves 2,506 rows with the eleven you have played somewhere
// inside. Both of these are about the same problem and both are things every
// other frontend has.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, filter, library, state, el, backend, arrangeCurrentList;

/// A turn of the event loop, for the handlers that write through the backend
/// and redraw when it answers.
const settle = () => new Promise((r) => setTimeout(r, 0));

// `players` is the most a game supports, parsed from RomM's free text in Rust.
// Delta has none on purpose: two thirds of the real library has no player
// count, and that case decides whether the filter is useful or noise.
const ROWS = [
  { id: 1, name: "Alpha", downloaded: true, favorite: false, last_played: null, rating: 9, players: 1 },
  { id: 2, name: "Beta", downloaded: false, favorite: true, last_played: "2026-01-01", rating: 4, players: 2 },
  { id: 3, name: "Gamma", downloaded: true, favorite: true, last_played: "2026-02-02", rating: 8, players: 4 },
  { id: 4, name: "Delta", downloaded: false, favorite: false, last_played: null, rating: null, players: null },
];

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
  dom.window.__TAURI__ = {
    core: {
      // Selecting a row draws the preview, which joins several of these
      // arrays. A thinner stub throws after the test has finished, as an
      // unhandled rejection rather than a failure.
      invoke: async (cmd, args) =>
        cmd === "rom_detail"
          ? {
              id: args.id,
              name: `Game ${args.id}`,
              fs_name: "g.zip",
              platform: "arcade",
              platform_slug: "arcade",
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
  backend = fakeBackend(dom.window.__TAURI__.core.invoke);
  dom.window.__TAURI__.core.invoke = backend;
  filter = await import("../js/filter.js");
  library = await import("../js/library.js");
  ({ state, el } = await import("../js/state.js"));
  ({ arrangeCurrentList } = await import("../js/arrange.js"));
  // The tables the menu is built from, which the app fetches at startup.
  const { loadListControls } = await import("../js/sort.js");
  filter.setFilters((await loadListControls()).filters);
});

beforeEach(async () => {
  state.view = "roms";
  state.platform = "arcade";
  state.collection = null;
  state.rows = ROWS;
  state.layout = "list";
  // These tests hand rows to the page directly rather than through `roms`, so
  // the stand-in backend is told what the list holds and how it narrows.
  backend.rows(ROWS);
  backend.arrange((all) => all);
  await filter.clearFilters();
  await arrangeCurrentList();
  document.querySelector(".ctx-menu")?.remove();
});

// What each filter keeps, which of them cancel each other out, and that they
// belong to the list they were set on, are asserted in `gamefilter::tests` and
// `gamelist::tests` — against the implementation, rather than through a page.
// What is left here is the page: the button, the menu, and the empty result.

describe("the filter menu and button", () => {
  /// A filtered list looks exactly like a short one, and the filter itself is
  /// off screen in a menu — so the button has to say it is doing something.
  test("the button counts what is on", async () => {
    filter.refreshFilterButton();
    assert.match(el.filterBtn.textContent, /Filter/);
    assert.equal(el.filterBtn.classList.contains("on"), false);

    await filter.toggleFilter("fav");
    filter.refreshFilterButton();
    assert.match(el.filterBtn.textContent, /Filters · 1/);
    assert.equal(el.filterBtn.classList.contains("on"), true, "a filtered list looks unfiltered");
  });

  /// A filter is built out of two or three choices. A menu that shuts on each
  /// one turns that into four trips to the same button.
  test("choosing one leaves the menu open", async () => {
    filter.openFilterMenu();
    const item = [...document.querySelectorAll(".ctx-menu button")].find((b) =>
      b.textContent.includes("Starred")
    );
    assert.ok(item, "the menu does not offer the filters");
    item.click();
    await settle();
    assert.deepEqual(filter.activeFilters(), ["fav"]);
    assert.ok(document.querySelector(".ctx-menu"), "the menu shut after one choice");
  });

  /// An empty result is indistinguishable from an empty console, and the
  /// reason for it is hidden in a menu.
  test("an empty result says why, and offers the way out", async () => {
    // Two filters on and nothing left that passes them: the case that looks
    // exactly like an empty console unless the page says otherwise.
    await filter.toggleFilter("local");
    await filter.toggleFilter("great");
    backend.arrange(() => []);
    await arrangeCurrentList();
    library.renderRows([{ ...ROWS[3] }], false);
    const empty = document.getElementById("list").textContent;
    assert.match(empty, /matches the filters/, `unexplained empty list: ${empty}`);

    document.querySelector(".clear-filters").click();
    await settle();
    assert.deepEqual(filter.activeFilters(), [], "the way out did not clear them");
  });
});

describe("surprise me", () => {
  /// Nobody knows 2,506 arcade games, and scrolling until something looks
  /// familiar always lands in the same three letters of the alphabet.
  test("it picks something and puts the cursor on it", () => {
    library.renderRows(ROWS, false);
    const pick = library.randomGame();
    assert.ok(pick, "nothing was picked");
    assert.equal(state.selected, pick.id, "the cursor is somewhere else");
  });

  /// From what is shown, not from everything — so "surprise me out of the ones
  /// I have not played" is a question you can actually ask.
  test("it picks from what the filters left", async () => {
    await filter.toggleFilter("unplayed");
    // What "unplayed" keeps out of these four is settled in `gamefilter::tests`;
    // here it is given, and what is under test is that the button picks from
    // what was left rather than from everything.
    backend.arrange((all) => all.filter((r) => !r.last_played));
    await arrangeCurrentList();
    library.renderRows(ROWS, false);
    for (let i = 0; i < 20; i++) {
      const pick = library.randomGame();
      assert.ok(
        ["Alpha", "Delta"].includes(pick.name),
        `${pick.name} is filtered out but was offered`
      );
    }
  });

  test("an empty list is not an error", async () => {
    state.rows = [];
    backend.rows([]);
    await arrangeCurrentList();
    assert.equal(library.randomGame(), null);
  });
});

describe("random comes from this list and no other", () => {
  /// The point of it is "something from what I am looking at". Reaching across
  /// consoles, or past the filters, would make it a lucky dip through the
  /// whole library — which is not what a button on a console's page can mean.
  test("only rows from the list on screen", async () => {
    const arcade = [
      { id: 90, name: "Metal Slug", downloaded: true, favorite: false, last_played: null },
      { id: 91, name: "Pang", downloaded: true, favorite: false, last_played: null },
    ];
    state.rows = arcade;
    backend.rows(arcade);
    await arrangeCurrentList();
    library.renderRows(arcade, false);
    const ids = new Set();
    for (let i = 0; i < 30; i++) ids.add(library.randomGame().id);
    assert.deepEqual([...ids].sort(), [90, 91], "it picked something not in this list");
  });
});

// Walking into a folder, the way ES-DE does.
//
// A library that files its homebrew in `snes/Aftermarket` used to show one
// unplayable entry called Aftermarket with thirteen games hidden behind it.
// The scan records where each game sits and the whole console arrives in one
// call, so what is asserted here is the page: which games belong to the level
// you are on, which folders it offers, and that Back walks out one step at a
// time.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, library, state, el, trail, backend, arrangeCurrentList;

const row = (id, name, rel_dir) => ({
  id,
  name,
  rel_dir,
  fs_name: `${name}.sfc`,
  platform: "snes",
  size_bytes: 1,
  downloaded: true,
  favorite: false,
  last_played: null,
  rating: null,
  year: null,
  players: null,
});

// One console with games at three depths, which is what the card actually
// holds: `sfc/AdditionalRoms/Homebrew` is two levels below the top.
const ROWS = [
  row(1, "Super Mario World", ""),
  row(2, "Zelda", ""),
  row(3, "Witch n' Wiz", "Aftermarket"),
  row(4, "Corn Buster", "Aftermarket"),
  row(5, "Some Prototype", "AdditionalRoms/Prototypes"),
  row(6, "Some Homebrew", "AdditionalRoms/Homebrew"),
];

const dirsOnScreen = () =>
  [...document.querySelectorAll(".folders [data-dir]")].map((n) => n.dataset.dir);
const gamesOnScreen = () =>
  [...document.querySelectorAll(".rows > .row[data-id]")].map((n) => Number(n.dataset.id));

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
              fs_name: "g.sfc",
              platform: "snes",
              platform_slug: "snes",
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
  backend = fakeBackend(dom.window.__TAURI__.core.invoke);
  dom.window.__TAURI__.core.invoke = backend;
  library = await import("../js/library.js");
  ({ state, el, trail } = await import("../js/state.js"));
  ({ arrangeCurrentList } = await import("../js/arrange.js"));
});

beforeEach(async () => {
  state.view = "roms";
  state.platform = "snes";
  state.collection = null;
  state.rows = ROWS;
  state.layout = "list";
  state.folder = "";
  trail.length = 0;
  backend.rows(ROWS);
  backend.arrange((all) => all);
  await arrangeCurrentList();
});

describe("folders inside a console", () => {
  test("the top level shows its own games and a folder for each shelf", () => {
    library.renderRows(state.rows, false);
    assert.deepEqual(gamesOnScreen(), [1, 2]);
    assert.deepEqual(dirsOnScreen(), ["AdditionalRoms", "Aftermarket"]);
  });

  test("opening one shows what is inside it and nothing else", () => {
    library.renderRows(state.rows, false);
    library.openFolder("Aftermarket");
    assert.deepEqual(gamesOnScreen(), [3, 4]);
    assert.deepEqual(dirsOnScreen(), []);
  });

  // Two levels down is the case that catches a prefix match written as
  // `startsWith(folder)`: `AdditionalRoms` would then also swallow a sibling
  // called `AdditionalRomsOld`.
  test("a folder of folders offers the next level, not the games below it", () => {
    library.renderRows(state.rows, false);
    library.openFolder("AdditionalRoms");
    assert.deepEqual(gamesOnScreen(), []);
    assert.deepEqual(dirsOnScreen(), ["AdditionalRoms/Homebrew", "AdditionalRoms/Prototypes"]);

    library.openFolder("AdditionalRoms/Homebrew");
    assert.deepEqual(gamesOnScreen(), [6]);
  });

  test("back walks out one level at a time", () => {
    library.renderRows(state.rows, false);
    library.openFolder("AdditionalRoms");
    library.openFolder("AdditionalRoms/Homebrew");
    trail.pop()();
    assert.equal(state.folder, "AdditionalRoms");
    trail.pop()();
    assert.equal(state.folder, "");
    assert.deepEqual(gamesOnScreen(), [1, 2]);
  });

  // A console whose top level is nothing but folders is not an empty console.
  test("a level with folders and no games does not say there is nothing here", () => {
    state.rows = [row(7, "Only Nested", "Aftermarket")];
    backend.rows(state.rows);
    library.renderRows(state.rows, false);
    assert.deepEqual(dirsOnScreen(), ["Aftermarket"]);
    assert.equal(document.querySelector(".empty"), null);
  });
});

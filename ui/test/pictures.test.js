// The Select button: change whatever pictures are on screen.
//
// Worth its own file because the rules are all conditional and none of them
// fail loudly. Cycling onto a style with no pictures gives a grid of nothing,
// which reads as the button having broken the page; cycling the wrong kind for
// the view changes something the person is not looking at; and forgetting to
// tell the settings window leaves its dropdown disagreeing with the screen.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, ui, pictures, invoked, emitted, styles, listArt;

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  global.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });

  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        if (cmd === "icon_styles") return styles;
        if (cmd === "set_icon_style") return args.key;
        if (cmd === "list_art_options") return listArt;
        if (cmd === "set_list_art") return `Game list shows: ${args.value}`;
        if (cmd === "platforms") return [];
        return [];
      },
      convertFileSrc: (p) => p,
    },
    event: {
      listen: async () => () => {},
      emit: (name) => emitted.push(name),
    },
  };

  ui = await import("../js/state.js");
  pictures = await import("../js/pictures.js");
});

beforeEach(() => {
  invoked = [];
  emitted = [];
  styles = [
    { key: "logo", label: "Logos", available: 30, selected: true },
    { key: "consolegame", label: "Consoles", available: 12, selected: false },
    { key: "controller", label: "Controllers", available: 0, selected: false },
  ];
  listArt = [
    [
      ["physicalmedia", "Cartridge"],
      ["3dboxes", "3D box"],
      ["miximages", "Mix"],
    ],
    "physicalmedia",
  ];
  ui.state.rows = [];
});

const settle = () => new Promise((r) => dom.window.setTimeout(r, 0));

describe("the button that changes the pictures", () => {
  test("on the console grid it changes the console pictures", async () => {
    ui.state.view = "platforms";
    await pictures.cyclePictures();
    await settle();

    const set = invoked.find((c) => c.cmd === "set_icon_style");
    assert.ok(set, "nothing was changed");
    assert.equal(set.args.key, "consolegame", "it should step to the next style");
    assert.equal(
      invoked.some((c) => c.cmd === "set_list_art"),
      false,
      "it changed the game-list pictures from the console grid"
    );
  });

  test("inside a console it changes the game pictures", async () => {
    ui.state.view = "roms";
    ui.state.platform = "snes";
    await pictures.cyclePictures();
    await settle();

    const set = invoked.find((c) => c.cmd === "set_list_art");
    assert.ok(set, "nothing was changed");
    assert.equal(set.args.value, "3dboxes");
    assert.equal(invoked.some((c) => c.cmd === "set_icon_style"), false);
  });

  /// A style with no pictures draws a grid of nothing, which reads as the
  /// button having broken the page rather than as a style being empty.
  test("styles with no pictures are skipped", async () => {
    ui.state.view = "platforms";
    // Logos selected, consoles empty too: the only other usable style is gone,
    // so there is nothing to cycle to.
    styles[1].available = 0;
    await pictures.cyclePictures();
    await settle();
    assert.equal(
      invoked.some((c) => c.cmd === "set_icon_style"),
      false,
      "it switched to a style with no pictures in it"
    );
  });

  test("it wraps round rather than stopping at the end", async () => {
    ui.state.view = "platforms";
    styles = [
      { key: "logo", label: "Logos", available: 30, selected: false },
      { key: "consolegame", label: "Consoles", available: 12, selected: true },
    ];
    await pictures.cyclePictures();
    await settle();
    assert.equal(invoked.find((c) => c.cmd === "set_icon_style").args.key, "logo");
  });

  /// The settings window is a separate document and cannot see this one. Its
  /// dropdown would otherwise sit there showing whatever was selected when it
  /// opened, disagreeing with the screen behind it.
  test("the settings window is told, both ways round", async () => {
    ui.state.view = "platforms";
    await pictures.cyclePictures();
    await settle();
    assert.ok(emitted.includes("icons-changed"), `only emitted: ${emitted}`);

    emitted.length = 0;
    ui.state.view = "roms";
    await pictures.cyclePictures();
    await settle();
    assert.ok(emitted.includes("art-changed"), `only emitted: ${emitted}`);
  });

  /// A backend that cannot answer is not a reason to throw inside a button
  /// handler: the press is silent, but the page stays alive.
  test("a backend that refuses does not take the page down", async () => {
    ui.state.view = "platforms";
    styles = null; // makes the stub's response unusable
    await assert.doesNotReject(() => pictures.cyclePictures());
  });
});

describe("changing the pictures leaves the tab you are in alone", () => {
  /// Reported as "after pressing Select, the shoulder buttons stop working and
  /// only Library shows". What actually happened: the console grid was redrawn
  /// into whatever tab was open, and the tab machinery remembers where each
  /// section was left — so My collections went on showing the platform grid on
  /// every later visit, and after a few presses every tab showed the same
  /// screen.
  test("it does not draw the console grid into another section", async () => {
    ui.state.view = "collections";
    await pictures.cyclePictures();
    await settle();

    assert.equal(
      invoked.some((c) => c.cmd === "platforms"),
      false,
      "the console grid was drawn over a collections page"
    );
    // The setting still changed; only the redraw was wrong.
    assert.ok(invoked.some((c) => c.cmd === "set_icon_style"));
    assert.equal(ui.state.view, "collections", "the view was replaced");
  });

  test("and does redraw it when the console grid is what is showing", async () => {
    ui.state.view = "platforms";
    await pictures.cyclePictures();
    await settle();
    assert.ok(invoked.some((c) => c.cmd === "platforms"), "the grid was not redrawn");
  });
});

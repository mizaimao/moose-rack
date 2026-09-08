// Choosing the window instead of RetroArch, per game.
//
// There are two ways into the in-page emulator and they are not the same thing.
// One is a fallback: RetroArch has no core for this system, so the window
// offers to. The other is this — a deliberate choice on one game, made in the
// Core dropdown, when RetroArch *would* have worked.
//
// It is deliberately not stored through `set_game_core`. That writes a libretro
// core name into `config.toml` and every launch resolves against it, so a
// pseudo-core in there would be a lie the rest of the app has to keep reading.

import { test, describe, beforeEach } from "node:test";
import assert from "node:assert/strict";

const store = new Map();
globalThis.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
};

const { playHereWanted, setPlayHere, BROWSER_CHOICE, browserPlay } =
  await import("../js/ejs-systems.js");

describe("play this game in the window", () => {
  beforeEach(() => store.clear());

  test("nothing is chosen until it is chosen", () => {
    assert.equal(playHereWanted(12672), false);
  });

  test("the choice is remembered, and is per game", () => {
    setPlayHere(12672, true);
    assert.equal(playHereWanted(12672), true);
    assert.equal(playHereWanted(12673), false);
  });

  test("picking a real core takes it off again", () => {
    setPlayHere(12672, true);
    setPlayHere(12672, false);
    assert.equal(playHereWanted(12672), false);
  });

  test("a string id and a number id are the same game", () => {
    // The dropdown's handler gets whatever `selectRom` was given, and the
    // dataset hands out strings.
    setPlayHere("12672", true);
    assert.equal(playHereWanted(12672), true);
  });

  test("the marker is not a core name", () => {
    // It goes in the same <select> as real cores, and `set_game_core` must
    // never be handed it. Nothing libretro is called this.
    assert.match(BROWSER_CHOICE, /^__.*__$/);
    assert.notEqual(BROWSER_CHOICE, browserPlay("snes").core);
  });

  test("storage that throws is not a launch that fails", () => {
    // A private window cannot remember the choice. It must still make it for
    // this launch rather than throwing out of the change handler.
    const real = globalThis.localStorage.setItem;
    globalThis.localStorage.setItem = () => {
      throw new Error("quota");
    };
    try {
      assert.equal(setPlayHere(1, true), true);
    } finally {
      globalThis.localStorage.setItem = real;
    }
  });

  test("a corrupted store reads as nothing chosen", () => {
    store.set("playHere", "{not json");
    assert.equal(playHereWanted(12672), false);
  });
});

// Reading the save out of a running core.
//
// The interesting case is the one that is not a save at all: a core that has
// never loaded one still has SRAM, and it is all zeros. Reporting that as a
// save makes the first sync of a game you played on the handheld a *conflict* —
// both sides have something, they differ, and this browser has agreed to
// nothing yet. Safe, since nothing is overwritten, and wrong: an empty
// cartridge is not a copy to weigh against a real one.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { readSave } from "../js/browser-saves.js";

const core = (bytes) => ({
  getSaveFile: () => bytes,
  getSaveFilePath: () => "/data/saves/Snes9x/ActRaiser (USA).srm",
});

describe("reading a save", () => {
  test("a real save comes back with its file name", () => {
    const b = new Uint8Array(8192);
    b[42] = 1;
    const out = readSave(core(b));
    assert.equal(out.bytes.length, 8192);
    assert.equal(out.fileName, "ActRaiser (USA).srm");
  });

  test("an untouched cartridge is not a save", () => {
    assert.equal(readSave(core(new Uint8Array(8192))), null, "8 KB of zeros was offered as a save");
    assert.equal(readSave(core(new Uint8Array(0))), null);
    assert.equal(readSave(core(null)), null);
    assert.equal(readSave(core(undefined)), null);
  });

  /// One byte is enough. A save with a single flag set is a save.
  test("a nearly-empty save is still a save", () => {
    const b = new Uint8Array(8192);
    b[8191] = 0xff;
    assert.ok(readSave(core(b)), "a save with one byte set was discarded");
  });

  /// A core that throws rather than answering must not stop the game.
  test("a core that cannot be read is not a crash", () => {
    assert.equal(readSave({ getSaveFile: () => { throw new Error("no"); } }), null);
    assert.equal(readSave(null), null);
    assert.equal(readSave({}), null);
  });
});

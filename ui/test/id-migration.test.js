// Remembered games from before stable ids.
//
// Two things in a page's storage held positional game ids: the game the cursor
// was last on in each list, and the games set to play in the window. Those
// numbers name nothing now. The backend knows which game each one was once a
// scan or sync has placed it; this checks the page asks, rewrites what it gets
// an answer for, and leaves the rest alone.

import { test, describe, beforeEach } from "node:test";
import assert from "node:assert/strict";

const store = new Map();
globalThis.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
  key: (i) => [...store.keys()][i] ?? null,
  get length() { return store.size; },
};

const { adoptStableIds, isLegacy, FLOOR } = await import("../js/id-migration.js");

const STABLE = 4279940656588578; // Chrono Trigger on dev.lan

describe("remembered games move to stable ids", () => {
  beforeEach(() => store.clear());

  test("the cursor and the play-here choice follow their game", async () => {
    store.set("lastRom", JSON.stringify({ "roms:snes": 5653, "roms:gba": 1918, "search": STABLE }));
    store.set("playHere", JSON.stringify([5653, STABLE]));
    const asked = [];
    const state = { lastRom: JSON.parse(store.get("lastRom")) };
    const moved = await adoptStableIds(async (cmd, args) => {
      asked.push([cmd, args]);
      return { "5653": STABLE }; // 1918 not placed yet
    }, state);

    assert.equal(moved, 2);
    assert.deepEqual(asked, [["stable_ids", { ids: [5653, 1918] }]], "only old ids are asked about, once each");
    const lastRom = JSON.parse(store.get("lastRom"));
    assert.equal(lastRom["roms:snes"], STABLE);
    assert.equal(lastRom["roms:gba"], 1918, "an unplaced id waits for a later start");
    assert.equal(lastRom.search, STABLE, "a stable id is untouched");
    assert.deepEqual(JSON.parse(store.get("playHere")), [STABLE], "and two entries for one game become one");
    assert.equal(state.lastRom["roms:snes"], STABLE, "the in-memory copy state.js read at import is updated too");
  });

  test("EmulatorJS's per-game settings follow their game", async () => {
    store.set("ejs--10793-snes9x-ActRaiser (USA)-settings", '{"controlSettings":"mine"}');
    store.set("ejs-5653-snes9x-Chrono Trigger (USA)-settings", '{"cheats":"old"}');
    store.set(`ejs-${STABLE}-snes9x-Chrono Trigger (USA)-settings`, '{"cheats":"newer"}');
    store.set("ejs-settings", '{"volume":0.5}');
    const ACT = 3550463174075161;
    await adoptStableIds(async (cmd, { ids }) => {
      assert.deepEqual(ids.sort((a, b) => a - b), [-10793, 5653]);
      return { "-10793": ACT, "5653": STABLE };
    });
    assert.equal(store.get(`ejs-${ACT}-snes9x-ActRaiser (USA)-settings`), '{"controlSettings":"mine"}');
    assert.equal(store.has("ejs--10793-snes9x-ActRaiser (USA)-settings"), false);
    assert.equal(
      store.get(`ejs-${STABLE}-snes9x-Chrono Trigger (USA)-settings`),
      '{"cheats":"newer"}',
      "settings already under the new id win"
    );
    assert.equal(store.get("ejs-settings"), '{"volume":0.5}', "the global settings key is not a game's");
  });

  test("nothing old means no call at all", async () => {
    store.set("lastRom", JSON.stringify({ a: STABLE }));
    let called = false;
    assert.equal(await adoptStableIds(async () => { called = true; }), 0);
    assert.equal(called, false);
  });

  test("a backend that fails or does not know the command leaves storage alone", async () => {
    store.set("lastRom", JSON.stringify({ a: 12 }));
    for (const invoke of [async () => { throw new Error("unknown command"); }, async () => null]) {
      assert.equal(await adoptStableIds(invoke), 0);
      assert.equal(JSON.parse(store.get("lastRom")).a, 12);
    }
  });

  test("corrupt storage is not a reason for the page not to load", async () => {
    store.set("lastRom", "{not json");
    store.set("playHere", "[also not");
    assert.equal(await adoptStableIds(async () => ({})), 0);
  });

  test("the threshold agrees with the backend's", () => {
    assert.equal(FLOOR, 16777216);
    assert.ok(isLegacy(-10793) && isLegacy(5653));
    assert.ok(!isLegacy(STABLE) && !isLegacy(FLOOR));
  });
});

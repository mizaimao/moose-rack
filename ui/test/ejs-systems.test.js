// Every platform the library serves is either playable in a browser or refused
// for a stated reason. Nothing is allowed to be neither.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { browserPlay, shouldWarn, PLAYABLE, UNSUPPORTED } from "../js/ejs-systems.js";

/// The platform slugs `moose-service` reports today, from `/invoke/platforms`
/// against the live server. Written down so adding a platform to the library
/// fails here until somebody says what the browser should do with it.
const SERVED = [
  "arcade", "dc", "famicom", "gamegear", "gb", "gba", "gbc", "megadrive", "n64",
  "nds", "neo-geo-pocket", "neogeoaes", "nes", "ngc", "pcengine", "psx",
  "saturn", "sfc", "snes", "wonderswan", "wonderswancolor",
];

describe("what the browser can play", () => {
  test("every served platform is decided one way or the other", () => {
    const undecided = SERVED.filter((p) => {
      const r = browserPlay(p);
      return !r.core && !r.refuse;
    });
    assert.deepEqual(undecided, [], "these platforms have no verdict");
  });

  test("a refusal says why, and is a sentence rather than a slug", () => {
    for (const p of SERVED) {
      const r = browserPlay(p);
      if (!r.refuse) continue;
      assert.match(r.refuse, /[a-z] [a-z]/i, `${p}: ${r.refuse}`);
    }
  });

  /// GameCube and Dreamcast are the two with no core anywhere in the release.
  test("the two with no core are refused, not offered", () => {
    for (const p of ["ngc", "gc", "dc", "dreamcast"]) {
      assert.ok(browserPlay(p).refuse, `${p} was offered a core`);
      assert.equal(browserPlay(p).core, undefined);
    }
  });

  /// Cartridges are the whole point and must never be gated behind a warning.
  test("cartridges play, and are never warned about", () => {
    for (const p of ["nes", "snes", "sfc", "gb", "gbc", "gba", "megadrive",
                     "gamegear", "pcengine", "ngp", "wonderswan", "arcade"]) {
      assert.ok(PLAYABLE[p], `${p} should be playable`);
      assert.equal(shouldWarn(p, 4e6), false, `${p} at 4 MB should not warn`);
    }
  });

  /// A disc image crossing the network into memory is a question.
  test("discs are asked about before they start", () => {
    assert.equal(shouldWarn("psx", 302e6), true);
    assert.equal(shouldWarn("saturn"), true, "falls back to the measured typical size");
    assert.equal(shouldWarn("nes", 131072), false);
    // The rom's own size wins over the typical one: a small psx game is small.
    assert.equal(shouldWarn("psx", 12e6), false);
  });

  test("an unknown platform is refused rather than guessed at", () => {
    assert.ok(browserPlay("something_new").refuse);
    assert.ok(browserPlay("").refuse);
    assert.ok(browserPlay(undefined).refuse);
  });

  /// Both spellings occur: `neogeoaes` is the platform slug, `neogeo` the ES-DE
  /// directory, and the UI has been seen carrying either.
  test("the slugs with two spellings both resolve", () => {
    assert.equal(browserPlay("neogeoaes").core, "arcade");
    assert.equal(browserPlay("ngp").core, "ngp");
    assert.equal(browserPlay("neo-geo-pocket").core, "ngp");
    assert.equal(browserPlay("SNES").core, "snes", "case should not matter");
  });

  test("nothing is in both tables", () => {
    const both = Object.keys(PLAYABLE).filter((k) => k in UNSUPPORTED);
    assert.deepEqual(both, []);
  });
});

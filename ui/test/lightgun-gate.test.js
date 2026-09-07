// When the light-gun notice belongs in front of a launch.
//
// It is a modal and `launch` awaits it, so getting this wrong is not a cosmetic
// fault: a dialog nobody answers is a launch that never happens. And it opened
// on every SNES game, because the Super Scope exists and so every SNES game
// reports a gun — which is how a double-click in a browser came to do nothing
// at all while the same code path on the desktop was fine.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { askAboutLightGun } from "../js/lightgun-gate.js";

const ask = (o) => askAboutLightGun({ resolving: false, skipSync: false, mobile: false, native: true, ...o });

describe("the light-gun notice", () => {
  test("is shown on a desktop launch, which is what it describes", () => {
    assert.equal(ask({}), true);
  });

  /// The whole notice is about the desktop launch planner — the mouse aiming
  /// the gun, the gun taking the second controller port. Neither platform
  /// calls it.
  test("is not shown where nothing can launch a process for you", () => {
    assert.equal(ask({ native: false }), false, "a browser got the desktop notice");
    assert.equal(ask({ mobile: true }), false, "Android got the desktop notice");
    assert.equal(ask({ mobile: true, native: false }), false);
  });

  /// A retry has already answered it once, and a launch that skipped the sync
  /// is deliberately skipping the questions.
  test("is not repeated on a retry or a deliberate skip", () => {
    assert.equal(ask({ resolving: true }), false);
    assert.equal(ask({ skipSync: true }), false);
  });
});

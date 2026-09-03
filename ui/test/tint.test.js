// Picking a color out of box art.
//
// The canvas half needs a real graphics stack, which the test environment does
// not have. The half that decides *which* color, given the pixels, is plain
// arithmetic and is where every judgement lives — so that is what is tested.
//
// What it has to get right: a cover that is 90% black border and 10% red logo
// should read as red, not as very dark gray. A flat average gets that wrong,
// and a flat average is the obvious implementation.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { pick } from "../js/tint.js";

/// Build the pixel array `getImageData` would return, from a list of
/// `[r, g, b, count]` runs.
function pixels(...runs) {
  const out = [];
  for (const [r, g, b, count, a = 255] of runs) {
    for (let i = 0; i < count; i++) out.push(r, g, b, a);
  }
  return new Uint8ClampedArray(out);
}

const channels = (s) => s.split(" ").map(Number);

describe("choosing a cover's color", () => {
  test("a small bright mark beats a large dark background", () => {
    // The shape of most box art: a dark frame around a colorful logo.
    const [r, g, b] = channels(pick(pixels([10, 10, 12, 58], [220, 30, 40, 6])));
    assert.ok(r > g && r > b, `expected a red, got ${r} ${g} ${b}`);
    assert.ok(r > 150, `expected it bright enough to glow with, got ${r}`);
  });

  test("a gray cover gets no color rather than a made-up one", () => {
    assert.equal(pick(pixels([90, 90, 90, 64])), null);
    assert.equal(pick(pixels([12, 12, 14, 40], [200, 200, 205, 24])), null);
  });

  test("transparent padding around a cover is ignored", () => {
    // A tall cover letterboxed into a square leaves transparent bands. Counting
    // them would drag every color towards whatever the blank pixels hold.
    const opaque = pick(pixels([40, 90, 200, 32]));
    const padded = pick(pixels([40, 90, 200, 32], [255, 255, 255, 32, 0]));
    assert.equal(padded, opaque, "padding must not change the answer");
  });

  test("an empty or fully transparent image has no color", () => {
    assert.equal(pick(new Uint8ClampedArray([])), null);
    assert.equal(pick(pixels([255, 0, 0, 64, 0])), null);
  });

  test("hue survives the brightening", () => {
    // A deep blue cover has to come out blue, not washed to white by the lift.
    const [r, g, b] = channels(pick(pixels([8, 14, 60, 64])));
    assert.ok(b > r && b > g, `expected a blue, got ${r} ${g} ${b}`);
    assert.ok(b >= 200, `expected the lift to reach a usable level, got ${b}`);
  });

  test("the result is space separated, so it can carry an alpha", () => {
    // `rgb(r, g, b / .5)` is invalid; every use of this passes an alpha.
    const out = pick(pixels([200, 40, 40, 64]));
    assert.match(out, /^\d+ \d+ \d+$/);
  });
});

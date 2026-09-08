// Artwork is decoded off the main thread, and the app works when it cannot be.
//
// Measured in the desktop window on 2026-09-08, opening SNES: 49 covers cost
// 2,184ms of `createImageBitmap` and 1,834ms of fetching on the thread that
// runs the grid, the view transition and every animation. The first second
// after opening a console drew between one and four frames per 100ms. With the
// work moved to a worker it draws six -- 60fps -- from 300ms onward.
//
// jsdom has no `Worker` and no `OffscreenCanvas`, which is exactly the case
// that must not break: `decodeFitted` answers `null` and `fitted` falls back to
// the main-thread path that was there before.

import { test, describe, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { decodeFitted, stopDecoders } from "../js/decoder.js";

describe("the off-thread decoder", () => {
  beforeEach(() => stopDecoders());

  test("a browser without workers gets null, not a throw", async () => {
    assert.equal(typeof Worker, "undefined");
    assert.equal(await decodeFitted("http://example/a.png", 100, 100, 2), null);
  });

  test("null again on a second ask, without trying to start anything", async () => {
    // The pool is built once and remembered as unusable; asking a hundred times
    // while scrolling must not try a hundred times.
    assert.equal(await decodeFitted("http://example/a.png", 100, 100, 2), null);
    assert.equal(await decodeFitted("http://example/b.png", 100, 100, 2), null);
  });

  test("it never rejects, whatever it is given", async () => {
    for (const url of ["", "not a url", "://nonsense", null]) {
      assert.equal(await decodeFitted(url, 10, 10, 1), null, String(url));
    }
  });

  test("stopping it is safe when nothing was started", () => {
    stopDecoders();
    stopDecoders();
  });
});

describe("the decoder's worker program", () => {
  // The program is a string, so nothing here executes it. What can be checked
  // is that it still says the things the rest of the design depends on.
  test("it transfers the bitmap rather than copying it", async () => {
    const src = await import("node:fs/promises");
    const text = await src.readFile(new URL("../js/decoder.js", import.meta.url), "utf8");
    // The transfer list is the difference between a blit and a copy of every
    // pixel back across the thread boundary.
    assert.match(text, /postMessage\(\{ id, w, h, bitmap: out \}, \[out\]\)/);
    // And the full-size decode is closed inside the worker, not left for the
    // collector: that is the difference between a spike and a leak.
    assert.match(text, /bitmap\.close\(\)/);
    // Never enlarge, the same rule `fitpicture.js` states.
    assert.match(text, /Math\.min\(1,/);
  });
});

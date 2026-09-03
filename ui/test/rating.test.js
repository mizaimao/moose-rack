// The star rating.
//
// Here because the half star was U+2BE8 — a character almost no system font
// carries — so it rendered as the browser's missing-glyph box: a striped
// rectangle sitting in the middle of the row. Nothing in the code looked
// wrong, and nothing would have caught it except looking at the pixels or
// asserting on what characters are used.

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

/// util.js reaches state.js for the toast element, and state.js reads
/// localStorage at import time, so the page has to exist before the module
/// does. Loaded dynamically for that reason.
let starBar;

before(async () => {
  const dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
  });
  dom.window.__TAURI__ = {
    core: { invoke: () => Promise.resolve([]), convertFileSrc: (p) => p },
    event: { listen: () => Promise.resolve(() => {}) },
  };
  for (const k of ["window", "document", "navigator", "localStorage", "CSS"]) {
    Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true });
  }
  ({ starBar } = await import(join(uiDir, "js", "util.js")));
});

/// Everything outside the Basic Multilingual Plane's common ranges, plus the
/// specific characters that burned us. A star has to be a star.
const RISKY = ["\u2BE8", "\u2BE9", "\u2BEA", "\u2BEB", "\u00BD", "\uFFFD"];

const fillOf = (html) => {
  const m = html.match(/stars-on"[^>]*width:\s*([\d.]+)%/);
  return m ? Number(m[1]) : null;
};

describe("the star rating", () => {
  test("uses only characters a system font actually has", () => {
    for (const r of [0, 17, 50, 69, 99, 100]) {
      const html = starBar(r);
      for (const ch of RISKY) {
        assert.ok(!html.includes(ch), `rating ${r} emitted U+${ch.codePointAt(0).toString(16)}`);
      }
      // Five stars in each row, always the same character.
      assert.equal((html.match(/★/g) || []).length, 10, `rating ${r}`);
      assert.ok(!html.includes("☆"), "the empty row is the same glyph, dimmed");
    }
  });

  test("shows the score rather than the nearest half", () => {
    // 69/100 is 3.45 stars. Rounding to halves showed 3.5 and threw away the
    // difference between 69 and 71.
    assert.equal(fillOf(starBar(69)), 69);
    assert.equal(fillOf(starBar(71)), 71);
    assert.notEqual(fillOf(starBar(69)), fillOf(starBar(71)));
  });

  test("cannot overflow or go negative", () => {
    assert.equal(fillOf(starBar(140)), 100);
    assert.equal(fillOf(starBar(-20)), 0);
  });

  test("survives a missing or unparseable rating", () => {
    for (const bad of [null, undefined, "", "n/a", NaN]) {
      assert.equal(fillOf(starBar(bad)), 0, `${bad}`);
    }
  });

  test("says the score out loud for a screen reader", () => {
    assert.match(starBar(69), /aria-label="69 out of 100"/);
  });
});

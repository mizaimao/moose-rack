// The rapid-fire modifier choice.
//
// One button, chosen from several — not several at once. RetroArch 1.20.0
// binds the modifier once per player (`input_playerN_turbo`) and repeats one
// button (`input_turbo_default_button`), enforced in its own code with
// `if (id != remap_button) break;`. Several at once exists only in classic
// mode, which latches on face-button release and is the arrangement this one
// replaced.

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

let pane;

before(async () => {
  const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/" });
  global.window = dom.window;
  global.document = dom.window.document;
  global.localStorage = dom.window.localStorage;
  dom.window.__TAURI__ = { core: { invoke: async () => [] }, event: { listen: async () => () => {} } };
  pane = await import("../js/settings/emulators.js");
});

const mount = () => {
  const box = document.createElement("div");
  box.innerHTML = pane.html;
  return box;
};

describe("the rapid-fire modifier", () => {
  test("is one choice from several, not several at once", () => {
    const sel = mount().querySelector('[data-field="autofire"]');
    assert.ok(sel, "no modifier control");
    assert.equal(sel.multiple, false, "offering several at once is not possible in RetroArch");
    assert.deepEqual([...sel.options].map((o) => o.value), ["off", "lb", "rb", "y"]);
  });

  /// Y is not free the way the shoulders are — arcade cores map it to button
  /// D — and the label has to say so, or it reads as equivalent to LB and RB.
  test("the Y option says what else it sends", () => {
    const sel = mount().querySelector('[data-field="autofire"]');
    const y = [...sel.options].find((o) => o.value === "y");
    assert.match(y.textContent, /\bD\b/, `the Y option hides its side effect: ${y.textContent}`);
  });

  /// The fire button is not offered. It is RetroPad B — physical A, the
  /// primary fire in every arcade core — and fixed, which is also why no
  /// modifier can collide with it.
  test("there is no fire-button control", () => {
    assert.equal(mount().querySelector('[data-field="fire_button"]'), null);
  });
});

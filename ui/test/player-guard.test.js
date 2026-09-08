// One emulator at a time.
//
// A double-click is two `click` events and a `dblclick`, and anything that got
// two launches through would build two stages, two EmulatorJS instances, two
// audio contexts and two sets of key handlers on one page. That does not
// half-work; it looks like nothing working, which is exactly what a
// double-click did while the Play button was fine.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

let dom, playInBrowser, resetPlayer;

before(async () => {
  dom = new JSDOM("<!doctype html><html><body><div id=\"toast\"></div></body></html>", {
    url: "http://dev.lan/",
    pretendToBeVisual: true,
  });
  for (const k of ["window", "document", "localStorage", "CSS"])
    Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true });
  Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
  globalThis.requestAnimationFrame = (f) => dom.window.setTimeout(f, 0);
  // player.js listens on the window for the errors EmulatorJS throws inside
  // its own async loader, which never reach the promise it awaits.
  for (const k of ["addEventListener", "removeEventListener"])
    Object.defineProperty(globalThis, k, { value: dom.window[k].bind(dom.window), configurable: true });
  // state.js reads this at import time.
  dom.window.__TAURI__ = {
    core: { invoke: async () => ({}), convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: async () => {} },
  };
  // `confirmHeavy` asks in the page now rather than through `confirm()`, which
  // blocks the thread a game is running on. Answer it by pressing the button.
  Object.defineProperty(globalThis, "confirm", { value: () => true, configurable: true });
  const obs = new dom.window.MutationObserver(() => {
    dom.window.document.querySelector(".ejs-ask-row button")?.click();
  });
  obs.observe(dom.window.document.body, { childList: true, subtree: true });
  // jsdom does not fetch an external script, so neither `onload` nor `onerror`
  // ever fires and the promise `loadLoader` returns never settles. Answer for
  // it: this suite is about what the page does around the emulator, not about
  // the emulator arriving.
  const append = dom.window.document.body.appendChild.bind(dom.window.document.body);
  dom.window.document.body.appendChild = (n) => {
    const out = append(n);
    if (n.tagName === "SCRIPT") dom.window.setTimeout(() => n.onload?.(), 0);
    return out;
  };
  ({ playInBrowser, resetPlayer } = await import("../js/player.js"));
});

beforeEach(() => {
  resetPlayer();
});

const ROM = { id: -1, name: "ActRaiser", fs_name: "a.zip", platform_slug: "snes", size_bytes: 673604 };

describe("starting a game in the page", () => {
  test("a second start while one is running is refused", async () => {
    const first = playInBrowser(ROM);
    const second = await playInBrowser(ROM);
    await first;
    assert.equal(second, "Already playing");
    assert.equal(
      document.querySelectorAll("#ejs-stage").length,
      1,
      "two stages means two emulators on one page",
    );
  });

  test("a platform with no browser core never opens a stage", async () => {
    const out = await playInBrowser({ ...ROM, platform_slug: "ngc" });
    assert.match(out, /not playable/i);
    assert.equal(document.getElementById("ejs-stage"), null);
  });

  /// The box must never be a blank rectangle: that looks the same whether the
  /// core is downloading, a file is missing, or something threw.
  test("the stage says what it is doing", async () => {
    await playInBrowser(ROM);
    const note = document.querySelector("#ejs-stage .ejs-note");
    assert.ok(note, "no status line in the stage");
    assert.notEqual(note.textContent.trim(), "", "the stage is blank");
  });
});

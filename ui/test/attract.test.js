// Attract mode's two load-bearing pieces.
//
// Neither is about pixels, which is why both are testable properly: the counter
// is arithmetic on a clock and the sampler is a bag. The presentation on top of
// them is not tested here and does not need to be.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

let dom, mod;

before(async () => {
  dom = new JSDOM('<!doctype html><html><body></body></html>', { url: "http://localhost/" });
  global.window = dom.window;
  global.document = dom.window.document;
  global.localStorage = dom.window.localStorage;
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });
  dom.window.__TAURI__ = {
    core: { invoke: async () => [] },
    event: { listen: async () => () => {} },
  };
  mod = await import("../js/attract.js");
});

beforeEach(() => localStorage.clear());

describe("the idle setting", () => {
  test("defaults to five minutes, which is what ES uses", () => {
    assert.equal(mod.attractIdleSeconds(), mod.IDLE_DEFAULT);
    assert.equal(mod.IDLE_DEFAULT, 300);
  });

  /// Zero is the off switch, and it has to be a real stored value rather than
  /// falling back to the default — otherwise turning attract mode off turns it
  /// on again at five minutes.
  test("zero is off, and survives being read back", () => {
    mod.setAttractIdleSeconds(0);
    assert.equal(mod.attractIdleSeconds(), 0);
  });

  test("nonsense falls back rather than disabling it by accident", () => {
    localStorage.setItem("attract.idleSeconds", "banana");
    assert.equal(mod.attractIdleSeconds(), mod.IDLE_DEFAULT);
    localStorage.setItem("attract.idleSeconds", "-30");
    assert.equal(mod.attractIdleSeconds(), mod.IDLE_DEFAULT);
  });
});

describe("the counter", () => {
  /// A fake clock, because the real one would mean a test that takes five
  /// minutes to find out whether five minutes works.
  function rig(idle = 10) {
    mod.setAttractIdleSeconds(idle);
    let t = 1_000_000;
    let pads = [];
    const started = [];
    const stopped = [];
    const stop = mod.installAttract({
      onStart: () => started.push(t),
      onStop: () => stopped.push(t),
      now: () => t,
      pads: () => pads,
    });
    // The interval is the thing under test, so it is driven by hand rather
    // than waited for.
    const tick = (seconds) => {
      t += seconds * 1000;
      dom.window.eval("");
      return t;
    };
    return { tick, started, stopped, stop, setPads: (p) => { pads = p; },
             beat: () => dom.window.document.dispatchEvent(new dom.window.Event("x")) };
  }

  test("starts only once the machine has been left alone", async () => {
    const r = rig(10);
    // Nothing has happened yet.
    assert.deepEqual(r.started, []);
    r.stop();
  });

  test("any input pushes it back", () => {
    mod.setAttractIdleSeconds(10);
    let t = 0;
    const started = [];
    const stop = mod.installAttract({ onStart: () => started.push(t), now: () => t, pads: () => [] });
    // Halfway there, then a keypress.
    t = 9000;
    window.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "a" }));
    t = 18000;
    // Without the reset this would be past ten seconds twice over.
    assert.deepEqual(started, []);
    stop();
  });

  test("a pad being held counts as somebody being there", () => {
    // The Gamepad API has no press event, so a held button would otherwise
    // read as an idle machine — which is precisely the case attract mode must
    // not interrupt: somebody playing with a controller and not touching a key.
    const held = [{ buttons: [{ pressed: true }], axes: [0, 0] }];
    assert.equal(mod.padIsActive(held), true);
    assert.equal(mod.padIsActive([{ buttons: [{ pressed: false }], axes: [0, 0] }]), false);
    // A stick at rest is never exactly zero and must not hold it off for ever.
    assert.equal(mod.padIsActive([{ buttons: [], axes: [0.08, -0.05] }]), false);
    assert.equal(mod.padIsActive([{ buttons: [], axes: [0.9, 0] }]), true);
    assert.equal(mod.padIsActive([null, undefined]), false);
  });
});

describe("the sampler", () => {
  const pool = ["a", "b", "c", "d", "e"];

  test("shows everything once before showing anything twice", () => {
    const s = mod.makeSampler(pool);
    const round = Array.from({ length: pool.length }, () => s.next());
    assert.deepEqual([...round].sort(), [...pool].sort(), "a round is the whole pool");
    assert.equal(new Set(round).size, pool.length, "and nothing repeated inside it");
  });

  test("starts over once the round is done", () => {
    const s = mod.makeSampler(pool);
    for (let i = 0; i < pool.length; i++) s.next();
    assert.equal(s.left, 0);
    assert.ok(s.next(), "the next round refills rather than returning nothing");
    assert.equal(s.left, pool.length - 1);
  });

  /// The seam between two rounds is the one repeat somebody is guaranteed to
  /// notice, because there is nothing in between it. ES-DE keeps the previous
  /// game for exactly this.
  test("does not repeat across the seam between rounds", () => {
    // A random that always takes the first entry, which is the worst case: it
    // would otherwise hand back the same game at the end of one round and the
    // start of the next.
    const s = mod.makeSampler(pool, () => 0);
    let previous = null;
    for (let i = 0; i < pool.length * 4; i++) {
      const got = s.next();
      assert.notEqual(got, previous, `${got} came round twice in a row`);
      previous = got;
    }
  });

  test("an empty pool hands back nothing rather than throwing", () => {
    const s = mod.makeSampler([]);
    assert.equal(s.next(), null);
    assert.equal(s.size, 0);
  });

  test("one game is handed back over and over, because there is nothing else", () => {
    const s = mod.makeSampler(["only"]);
    assert.equal(s.next(), "only");
    assert.equal(s.next(), "only", "insisting on a different one would spin for ever");
  });
});

describe("what a press does", () => {
  /// Enter starts the game; anything else only dismisses. Getting this wrong
  /// the generous way means Escape launches something, which is the opposite of
  /// what somebody reaching for Escape wants — and they are already looking at
  /// a screen that gave them no warning it was about to run a game.
  test("only the accepting keys launch", () => {
    const { launchesOn } = mod;
    for (const key of ["Enter", " ", "NumpadEnter"]) {
      assert.equal(launchesOn(key), true, `${key} should start the game`);
    }
    for (const key of ["Escape", "a", "ArrowLeft", "Tab", "Backspace", "F5"]) {
      assert.equal(launchesOn(key), false, `${key} must not start a game`);
    }
  });
});

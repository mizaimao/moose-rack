// The browser's Back arrow, inside a single-page app.
//
// Without this the app has one history entry and Back leaves it, landing on
// the login page — which reads as being logged out rather than as having left.

import { test, describe, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { installHistoryNav, handlePop, resetHistoryNav } from "../js/history-nav.js";

/// A history that records what was pushed, and a listener registry, so a test
/// can fire `popstate` the way a browser does.
function fakeWindow() {
  const listeners = {};
  const pushes = [];
  const g = {
    history: { pushState: (s) => pushes.push(s), back: () => fire("popstate") },
    addEventListener: (name, fn) => ((listeners[name] ??= []).push(fn)),
  };
  function fire(name) {
    for (const fn of listeners[name] ?? []) fn({});
  }
  return { g, pushes, fire };
}

let saved;
beforeEach(() => {
  resetHistoryNav();
  saved = { history: globalThis.history, addEventListener: globalThis.addEventListener };
});

function install(opts) {
  const { g, pushes, fire } = fakeWindow();
  Object.defineProperty(globalThis, "history", { value: g.history, configurable: true });
  Object.defineProperty(globalThis, "addEventListener", { value: g.addEventListener, configurable: true });
  installHistoryNav(opts);
  return { pushes, fire };
}

describe("the browser's back arrow", () => {
  test("a spare entry is pushed so the first press is not already leaving", () => {
    const { pushes } = install({ back: () => {}, isTop: () => false });
    assert.equal(pushes.length, 1, "no spare entry was pushed");
  });

  test("a press steps the app back and leaves another spare behind", () => {
    let backs = 0;
    const { pushes, fire } = install({ back: () => backs++, isTop: () => false });
    fire("popstate");
    assert.equal(backs, 1, "the app did not step back");
    assert.equal(pushes.length, 2, "the next press has nothing to consume");
    fire("popstate");
    assert.equal(backs, 2);
    assert.equal(pushes.length, 3);
  });

  /// At the top there is nothing to go back to, and refusing would trap the
  /// tab: Back has to mean leave.
  test("at the top of the app it does not intervene", () => {
    let backs = 0;
    const { pushes, fire } = install({ back: () => backs++, isTop: () => true });
    fire("popstate");
    assert.equal(backs, 0, "it stepped back from the top");
    assert.equal(pushes.length, 1, "it put an entry back and trapped the tab");
  });

  /// The app can be somewhere on the way in and at the top on the way out.
  test("it stops intervening once the app reaches the top", () => {
    let depth = 2;
    const { pushes, fire } = install({ back: () => depth--, isTop: () => depth === 0 });
    fire("popstate");
    fire("popstate");
    assert.equal(depth, 0);
    const before = pushes.length;
    fire("popstate");
    assert.equal(pushes.length, before, "it kept the tab after reaching the top");
  });

  test("no history at all is survivable", () => {
    resetHistoryNav();
    Object.defineProperty(globalThis, "history", { value: undefined, configurable: true });
    assert.doesNotThrow(() => installHistoryNav({ back: () => {}, isTop: () => false }));
  });
});

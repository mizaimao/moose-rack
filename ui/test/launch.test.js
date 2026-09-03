// What happens between pressing play and the game appearing.
//
// That gap is several seconds on a cold start — a core check, a shader check, a
// BIOS check and a save negotiation with the server — and it used to say
// nothing at all, which reads as an app that has hung. The backend now
// announces each step, and this file holds the two things that can go wrong
// with that: the announcements not being listened for, and the listener
// outliving the launch.
//
// The second is the one worth a test. A launch that throws — a missing core, a
// save conflict, an unreachable server — takes a different path out, and a
// subscription left behind on that path means the next launch's progress is
// reported twice, then three times. It is invisible until it is absurd.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

// One DOM and one module graph for the file: state.js captures the Tauri
// bridge at import time, so a second page would leave it talking to the first.
let dom, mod, listeners, invoked, fail;

before(async () => {
  // The real page, because actions.js pulls in the gamepad loop and that
  // reaches for elements a stub page does not have.
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  // A bare global in the browser, a window property in jsdom. The launch path
  // measures the display refresh before it asks for the game, so without this
  // every launch fails before it starts.
  // Node's own `performance` is left alone: jsdom's delegates back to it, so
  // assigning one over the other recurses until the stack gives out.
  global.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });

  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        // The backend emits while the command is still running — that is the
        // whole point of it — so the stub does too.
        for (const fn of listeners) fn({ payload: "checking saves with the server…" });
        if (fail) throw new Error("no core installed");
        return "played for 3 minutes";
      },
    },
    event: {
      listen: async (name, fn) => {
        if (name === "launch-progress") {
          listeners.push(fn);
          return () => {
            listeners = listeners.filter((f) => f !== fn);
          };
        }
        return () => {};
      },
    },
  };

  mod = await import("../js/actions.js");
});

beforeEach(() => {
  listeners = [];
  invoked = [];
  fail = false;
});

describe("launching says what it is doing", () => {
  test("the phases the backend reports reach the screen", async () => {
    await mod.launch(7);
    // Two: the gun question, then the launch. A console with a light gun gets
    // a one-time notice before the game starts, so the launch path asks first.
    assert.deepEqual(
      invoked.map((i) => i.cmd),
      ["game_lightgun", "launch_rom"],
    );
    assert.match(
      dom.window.document.getElementById("toast").textContent,
      /played for 3 minutes/,
      "the result should be the last thing shown"
    );
  });

  test("the progress listener is dropped when the launch succeeds", async () => {
    await mod.launch(7);
    assert.equal(listeners.length, 0, "a subscription outlived its launch");
  });

  /// The path that actually leaked: a failed launch leaves through `catch`, and
  /// a listener released only after the happy-path await never gets released
  /// at all. Two failed launches and every later phase is reported twice.
  test("and dropped when the launch fails", async () => {
    fail = true;
    await mod.launch(7);
    assert.equal(
      listeners.length,
      0,
      "the failed launch kept its subscription — the next one reports twice"
    );
  });

  test("failing repeatedly does not stack up subscriptions", async () => {
    fail = true;
    await mod.launch(7);
    await mod.launch(7);
    await mod.launch(7);
    assert.equal(listeners.length, 0);
  });
});

// The save-conflict dialog.
//
// Worth testing in jsdom for the same reason the gamepad loop is: this path
// only runs when a save has genuinely diverged, which is rare and awkward to
// stage by hand, and a throw inside it would leave the launch blocked with
// nothing on screen. The behavior that matters is that cancelling does NOT
// launch and that an answered conflict is actually sent to the backend.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

// One page and one module graph for the whole file, as in gamepad.test.js and
// for the same reason: state.js captures the Tauri bridge and util.js captures
// element handles at import time. Building a fresh DOM per test leaves those
// pointing at the first one, so a later test silently talks to an earlier
// test's stub — which is exactly what happened while writing this.
let dom, mod, calls, failResolve;

before(async () => {
  dom = new JSDOM(
    '<!doctype html><html><body><footer id="toast" hidden></footer></body></html>',
    { url: "http://localhost/" }
  );
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  // `navigator` is getter-only in modern Node, so it is defined not assigned.
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });

  calls = [];
  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        calls.push({ cmd, args });
        if (failResolve) throw new Error("server said no");
        return "kept";
      },
    },
    event: { listen: async () => () => {} },
  };

  mod = await import("../js/conflicts.js");
});

beforeEach(() => {
  calls.length = 0;
  failResolve = false;
  document.querySelectorAll("#conflict-overlay").forEach((n) => n.remove());
});

const CONFLICT = {
  rom_id: 7,
  save_id: 12,
  file_name: "Zelda.srm",
  slot: "autosave",
  emulator: "snes9x",
  reason: "changed in both places",
  local_updated: "2026-08-06T10:00:00Z",
  local_bytes: 8192,
  server_updated: "2026-08-06T12:00:00Z",
};

describe("conflictsFrom", () => {
  test("finds the conflict list a refused launch carries", async () => {
    const got = mod.conflictsFrom(`SAVE_CONFLICT:${JSON.stringify([CONFLICT])}`);
    assert.equal(got.length, 1);
    assert.equal(got[0].file_name, "Zelda.srm");
  });

  test("an ordinary launch failure is not mistaken for a conflict", async () => {
    assert.equal(mod.conflictsFrom("RetroArch not found"), null);
    assert.equal(mod.conflictsFrom(new Error("not downloaded yet")), null);
    assert.equal(mod.conflictsFrom(undefined), null);
  });

  test("an empty or malformed list is not a conflict", async () => {
    // Opening a dialog with nothing in it would block the launch forever.
    assert.equal(mod.conflictsFrom("SAVE_CONFLICT:[]"), null);
    assert.equal(mod.conflictsFrom("SAVE_CONFLICT:{broken"), null);
  });
});

describe("the conflict dialog", () => {
  test("shows both sides with their dates, so which is newer is obvious", async () => {
    mod.askConflicts([CONFLICT]);

    const box = dom.window.document.querySelector("#conflict-overlay");
    assert.ok(box, "the dialog is on screen");
    const sides = [...box.querySelectorAll("[data-keep]")];
    assert.deepEqual(
      sides.map((s) => s.dataset.keep),
      ["local", "server"]
    );
    assert.match(box.textContent, /Zelda\.srm/);
    assert.match(box.textContent, /2026-08-06 10:00:00/, "local time shown");
    assert.match(box.textContent, /2026-08-06 12:00:00/, "server time shown");
  });

  test("choosing a side sends that choice and closes", async () => {
    const done = mod.askConflicts([CONFLICT]);

    dom.window.document.querySelector('[data-keep="server"]').click();
    assert.equal(await done, true, "answered");

    assert.equal(calls.length, 1);
    assert.equal(calls[0].cmd, "resolve_save_conflict");
    assert.deepEqual(calls[0].args, { fileName: "Zelda.srm", keep: "server" });
    assert.equal(
      dom.window.document.querySelector("#conflict-overlay"),
      null,
      "dialog closed"
    );
  });

  test("cancelling reports not-answered, so the caller must not launch", async () => {
    const done = mod.askConflicts([CONFLICT]);

    dom.window.document.querySelector(".conflict-cancel").click();
    assert.equal(await done, false, "cancelled");
    assert.equal(calls.length, 0, "nothing was written on either side");
  });

  test("Escape cancels — a pad user must never be trapped here", async () => {
    const done = mod.askConflicts([CONFLICT]);

    dom.window.document.dispatchEvent(
      new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })
    );
    assert.equal(await done, false);
  });

  test("arrows move between the choices and Enter takes one", async () => {
    const done = mod.askConflicts([CONFLICT]);
    const press = (key) =>
      dom.window.document.dispatchEvent(
        new dom.window.KeyboardEvent("keydown", { key, bubbles: true })
      );

    press("ArrowRight");
    assert.equal(dom.window.document.activeElement.dataset.keep, "server");
    press("ArrowLeft");
    assert.equal(dom.window.document.activeElement.dataset.keep, "local");
    press("Enter");
    assert.equal(await done, true);
    assert.equal(calls[0].args.keep, "local");
  });

  test("every conflict is asked about before the launch proceeds", async () => {
    const second = { ...CONFLICT, file_name: "Metroid.state1", slot: "slot1" };
    const done = mod.askConflicts([CONFLICT, second]);

    dom.window.document.querySelector('[data-keep="local"]').click();
    await new Promise((r) => setTimeout(r, 0));
    assert.ok(
      dom.window.document.querySelector("#conflict-overlay"),
      "still open for the second one"
    );
    assert.match(dom.window.document.body.textContent, /Metroid\.state1/);

    dom.window.document.querySelector('[data-keep="server"]').click();
    assert.equal(await done, true);
    assert.deepEqual(
      calls.map((c) => c.args.fileName),
      ["Zelda.srm", "Metroid.state1"]
    );
  });

  test("a failed resolve keeps the dialog open rather than launching anyway", async () => {
    failResolve = true;
    mod.askConflicts([CONFLICT]);

    dom.window.document.querySelector('[data-keep="local"]').click();
    await new Promise((r) => setTimeout(r, 0));

    assert.ok(
      dom.window.document.querySelector("#conflict-overlay"),
      "the save is still unresolved, so the question stays on screen"
    );
    assert.equal(
      dom.window.document.querySelector('[data-keep="local"]').disabled,
      false,
      "and can be tried again"
    );
  });
});

describe("the offline warning", () => {
  test("finds the reason a launch was refused for", () => {
    assert.equal(mod.offlineFrom("SAVE_OFFLINE:dns error"), "dns error");
    assert.equal(mod.offlineFrom("RetroArch not found"), null);
    // A conflict is a different question and must not be mistaken for this.
    assert.equal(mod.offlineFrom("SAVE_CONFLICT:[{}]"), null);
  });

  test("offers play-anyway and cancel, with cancel focused", () => {
    mod.askOffline("connection refused");
    const box = document.querySelector("#conflict-overlay");
    assert.ok(box);
    assert.match(box.textContent, /connection refused/);
    assert.match(box.textContent, /Play anyway/);
    // The safe answer is where a stray Enter lands.
    assert.equal(document.activeElement.dataset.go, "no");
  });

  test("play anyway resolves true, cancel resolves false", async () => {
    let done = mod.askOffline("server down");
    document.querySelector('[data-go="yes"]').click();
    assert.equal(await done, true);

    done = mod.askOffline("server down");
    document.querySelector('[data-go="no"]').click();
    assert.equal(await done, false);
  });

  test("Escape means cancel — never an accidental unsynced launch", async () => {
    const done = mod.askOffline("server down");
    document.dispatchEvent(
      new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })
    );
    assert.equal(await done, false);
    assert.equal(document.querySelector("#conflict-overlay"), null);
  });
});

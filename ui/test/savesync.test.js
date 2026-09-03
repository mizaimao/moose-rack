// The save-sync plan sheet.
//
// The whole point of this path is that nothing moves until somebody has looked
// at what would move, so what is worth pinning down is the refusals: that
// declining the plan does NOT call `sync_saves`, that an empty plan offers no
// button that would, and that conflicts coming back out of a finished run
// actually reach the picker instead of being reported at.
//
// Ordering and phrasing of the rows are not tested here — they are decided in
// `src/syncplan.rs` and asserted by `cargo test` against the real
// implementation. This file only asserts what the page does with them.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

// One page and one module graph for the whole file, as in conflicts.test.js:
// state.js captures the Tauri bridge at import time, so a fresh DOM per test
// would leave it pointing at the first one.
let dom, mod, calls, replies;

before(async () => {
  dom = new JSDOM(
    '<!doctype html><html><body><footer id="toast" hidden></footer></body></html>',
    { url: "http://localhost/" }
  );
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });

  calls = [];
  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        calls.push({ cmd, args });
        if (replies[cmd] instanceof Error) throw replies[cmd];
        return replies[cmd];
      },
    },
    event: { listen: async () => () => {} },
  };

  mod = await import("../js/savesync.js");
});

beforeEach(() => {
  calls.length = 0;
  replies = {};
  document.querySelectorAll("#conflict-overlay").forEach((n) => n.remove());
});

const PLAN = {
  agreed: 12,
  headline: "1 to conflict, 1 to pull, 2 to push",
  lines: [
    { action: "conflict", title: "Crash.srm", reason: "both changed", rom_id: 3, save_id: 3 },
    { action: "download", title: "Metroid.srm", reason: null, rom_id: 2, save_id: 2 },
    { action: "upload", title: "Aria.srm", reason: null, rom_id: 4, save_id: 4 },
    { action: "upload", title: "Zelda.srm", reason: null, rom_id: 1, save_id: 1 },
  ],
};

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

/// Wait for the sheet to be on screen. `syncSaves` awaits a command first, so
/// it is not there on the turn the call is made.
async function sheet() {
  for (let i = 0; i < 50; i += 1) {
    const box = document.querySelector(".sync-box");
    if (box) return box;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("the plan sheet never appeared");
}

const buttonSaying = (box, text) =>
  [...box.querySelectorAll("button")].find((b) => b.textContent === text);

describe("the plan is shown before anything moves", () => {
  test("declining leaves the saves alone", async () => {
    replies.sync_saves_plan = PLAN;
    const done = mod.syncSaves({ say: () => {} });
    const box = await sheet();
    buttonSaying(box, "Not now").click();
    await done;

    assert.deepEqual(calls.map((c) => c.cmd), ["sync_saves_plan"]);
  });

  test("Escape is a decline, not an accept", async () => {
    // The riskiest possible misreading of this dialog: a key pressed to get rid
    // of it starting the thing it was asking about.
    replies.sync_saves_plan = PLAN;
    const done = mod.syncSaves({ say: () => {} });
    await sheet();
    document.dispatchEvent(
      new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })
    );
    await done;

    assert.deepEqual(calls.map((c) => c.cmd), ["sync_saves_plan"]);
  });

  test("accepting runs the sync", async () => {
    replies.sync_saves_plan = PLAN;
    replies.sync_saves = { headline: "2 uploaded, 1 downloaded", notes: [], conflicts: [] };
    const done = mod.syncSaves({ say: () => {} });
    const box = await sheet();
    buttonSaying(box, "Carry this out").click();
    await done;

    assert.deepEqual(calls.map((c) => c.cmd), ["sync_saves_plan", "sync_saves"]);
  });

  test("every line in the plan gets a row", async () => {
    replies.sync_saves_plan = PLAN;
    const done = mod.syncSaves({ say: () => {} });
    const box = await sheet();
    assert.equal(box.querySelectorAll(".sy-row").length, 4);
    // The server's own words survive to the row that carries them.
    assert.match(box.querySelector(".sy-conflict").textContent, /both changed/);
    buttonSaying(box, "Not now").click();
    await done;
  });

  test("the focused button is the one that does nothing", async () => {
    // Enter on a dialog you have not read should not move somebody's saves.
    replies.sync_saves_plan = PLAN;
    const done = mod.syncSaves({ say: () => {} });
    const box = await sheet();
    assert.equal(document.activeElement.textContent, "Not now");
    buttonSaying(box, "Not now").click();
    await done;
  });
});

describe("an empty plan", () => {
  test("offers no button that would carry it out", async () => {
    // A live Apply on a plan with nothing in it is a button that does nothing,
    // which reads as broken rather than as finished.
    replies.sync_saves_plan = { agreed: 380, headline: "nothing to do — 380 already match", lines: [] };
    const done = mod.syncSaves({ say: () => {} });
    const box = await sheet();
    assert.equal(buttonSaying(box, "Carry this out"), undefined);
    assert.match(box.textContent, /380 already match/);
    buttonSaying(box, "Close").click();
    await done;
  });
});

describe("what comes back", () => {
  test("conflicts reach the picker instead of a status line", async () => {
    replies.sync_saves_plan = PLAN;
    replies.sync_saves = {
      headline: "1 in conflict",
      notes: [],
      conflicts: [CONFLICT],
    };
    const done = mod.syncSaves({ say: () => {} });
    buttonSaying(await sheet(), "Carry this out").click();

    // The picker is a second overlay, put up after the run returns.
    let picker = null;
    for (let i = 0; i < 50 && !picker; i += 1) {
      await new Promise((r) => setTimeout(r, 0));
      picker = document.querySelector(".conflict");
    }
    assert.ok(picker, "the conflict picker never appeared");
    assert.match(picker.textContent, /Zelda\.srm/);

    // Answered, so the promise settles rather than hanging this test.
    replies.resolve_save_conflict = "kept yours";
    picker.querySelector('[data-keep="local"]').click();
    await done;
    assert.ok(calls.some((c) => c.cmd === "resolve_save_conflict"));
  });

  test("a plan the server refuses is reported and moves nothing", async () => {
    replies.sync_saves_plan = new Error("no route to host");
    const said = [];
    await mod.syncSaves({ say: (m) => said.push(m) });

    assert.deepEqual(calls.map((c) => c.cmd), ["sync_saves_plan"]);
    assert.ok(said.some((m) => /no route to host/.test(m)), said);
  });
});

describe("two at once", () => {
  test("a second sync is refused while the first is up", async () => {
    // Both buttons reach the same function now, and two runs would race over
    // the same files.
    replies.sync_saves_plan = PLAN;
    const first = mod.syncSaves({ say: () => {} });
    await sheet();

    const said = [];
    assert.equal(await mod.syncSaves({ say: (m) => said.push(m) }), null);
    assert.ok(said.some((m) => /already running/.test(m)), said);

    buttonSaying(await sheet(), "Not now").click();
    await first;
    assert.deepEqual(calls.map((c) => c.cmd), ["sync_saves_plan"]);
  });
});

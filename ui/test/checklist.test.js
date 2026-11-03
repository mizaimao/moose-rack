// The launch checklist window: each line with its own size, and one button
// for the total of what's missing. See docs/launch-flow.html.

import { test, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
let dom, mod;

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), { url: "https://dev.lan/", pretendToBeVisual: true });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  dom.window.__TAURI__ = { core: { invoke: async () => null }, event: { listen: async () => () => {} } };
  mod = await import("../js/checklist.js");
});

const MB = 1024 * 1024;
const item = (label, ready, bytes) => ({ label, detail: ready ? "here" : "not here yet", ready, bytes });
const ico = (gameReady, emuReady, emuBytes = 31 * MB) => ({
  name: "Ico",
  platform: "ps2",
  game: item("Ico (USA).chd", gameReady, 646 * MB),
  emulator: item("PCSX2 v2.8.2", emuReady, emuBytes),
  ready: gameReady && emuReady,
  download_bytes: (gameReady ? 0 : 646 * MB) + (emuReady ? 0 : emuBytes),
  blocked: null,
});

test("the button is Play, Download with the total, Set up, or nothing", () => {
  assert.deepEqual(mod.buttonFor(ico(true, true)), { go: "play", text: "Play" });
  assert.deepEqual(mod.buttonFor(ico(false, false)), { go: "download", text: "Download 677.0 MB" });
  assert.deepEqual(mod.buttonFor(ico(true, false, 0)), { go: "download", text: "Set up" });
  assert.equal(mod.buttonFor({ ...ico(false, true), blocked: "no server" }), null);
});

test("each line shows its own size and the button the sum", async () => {
  const shown = mod.askChecklist(ico(false, false));
  const box = dom.window.document.querySelector(".checklist-box");
  const sizes = [...box.querySelectorAll(".cl-lines .size")].map((s) => s.textContent);
  assert.deepEqual(sizes, ["646.0 MB", "31.0 MB"]);
  assert.equal(box.querySelector(".cl-main").textContent, "Download 677.0 MB");
  assert.match(box.textContent, /Saves are checked with the server after you press Play/);
  dom.window.document.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "Enter" }));
  assert.equal(await shown, "download");
  assert.equal(dom.window.document.querySelector(".checklist-box"), null, "the window closed");
});

test("blocked shows why and offers only Cancel", async () => {
  const shown = mod.askChecklist({ ...ico(false, true), blocked: "The server isn't reachable, so the game can't be downloaded." });
  const box = dom.window.document.querySelector(".checklist-box");
  assert.equal(box.querySelector(".cl-main"), null);
  assert.match(box.querySelector(".cl-blocked").textContent, /server isn't reachable/);
  dom.window.document.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "Escape" }));
  assert.equal(await shown, "cancel");
});

#!/usr/bin/env node
// Drive the web UI in a real browser and say what happened.
//
//     node tools/browser-check.mjs [url] [--shot out.png]
//
// This exists because not having it cost a day. The jsdom suites run the app's
// modules and prove the code executes; they do not have a layout, a top layer,
// view transitions, or real hit-testing, and every one of those turned out to
// matter. Six times a fix looked right in jsdom, shipped, and did nothing --
// and the only witness was somebody at a keyboard being asked to try again.
//
// What it caught, in one run, after a day of guessing: the first click opens
// the detail pane, the grid reflows, the card moves out from under the pointer,
// and the second click lands on `<html>`. So `dblclick` fired outside the list
// and the delegated handler never ran at all. No error, no log, nothing to see
// from the server side.
//
// Needs Chrome. Headless, its own profile, muted, torn down on exit.
//
// Firefox was checked the same way once, over WebDriver BiDi -- it is the
// browser this was reported from, and the failure was hit-testing after a
// reflow, which is not an engine's opinion. Both agree. That check is not kept
// here because two protocols is twice the harness for the same answer; if you
// need it again, Firefox listens for BiDi on `--remote-debugging-port` and
// `input.performActions` sends a real double-click.

import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const url = args.find((a) => !a.startsWith("--")) ?? "http://dev.lan";
const shotAt = args.includes("--shot") ? args[args.indexOf("--shot") + 1] : null;
const PORT = 9222;

import { existsSync } from "node:fs";

// `require` is not defined in a module, which is how the first version of this
// found no browser on a machine with two.
const CHROME = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
].find((p) => existsSync(p));
if (!CHROME) {
  console.error("no Chrome found; this needs one to be a real browser");
  process.exit(2);
}

const profile = mkdtempSync(join(tmpdir(), "moose-browser-check-"));
const chrome = spawn(CHROME, [
  "--headless=new", `--remote-debugging-port=${PORT}`, "--no-first-run",
  "--no-default-browser-check", `--user-data-dir=${profile}`,
  // Headless is not silent. This starts a game, and a game has a title theme:
  // the first run played ActRaiser out of the speakers of somebody who had not
  // asked for it and could not see where it was coming from.
  "--mute-audio",
  "about:blank",
], { stdio: "ignore" });
const cleanup = () => {
  try { chrome.kill(); } catch {}
  try { rmSync(profile, { recursive: true, force: true }); } catch {}
};
process.on("exit", cleanup);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function cdp() {
  for (let i = 0; i < 40; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page;
    } catch {}
    await sleep(500);
  }
  throw new Error("Chrome never opened its debugging port");
}

const page = await cdp();
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
const logs = [];
await new Promise((r) => (ws.onopen = r));
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg.result ?? msg.error); pending.delete(msg.id); }
  if (msg.method === "Runtime.consoleAPICalled")
    logs.push(`${msg.params.type}: ` + msg.params.args.map((a) => a.value ?? a.description ?? "").join(" "));
  if (msg.method === "Runtime.exceptionThrown")
    logs.push("EXCEPTION: " + (msg.params.exceptionDetails.exception?.description ?? "").split("\n")[0]);
};
const send = (method, params = {}) =>
  new Promise((res) => { const n = ++id; pending.set(n, res); ws.send(JSON.stringify({ id: n, method, params })); });
const js = async (e) =>
  (await send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }))?.result?.value;

let failed = false;
const check = (label, got, want) => {
  const ok = want === undefined ? Boolean(got) : got === want;
  if (!ok) failed = true;
  console.log(`  ${ok ? "ok  " : "FAIL"} ${label}: ${got}`);
};

await send("Runtime.enable");
await send("Page.enable");
await send("Page.navigate", { url });
await sleep(9000);

console.log(`\n${url}`);
check("platform cards", await js("document.querySelectorAll('#list [data-slug]').length"));
await js(`document.querySelector("#list [data-slug='snes']")?.click()`);
await sleep(6000);
const cards = await js("document.querySelectorAll('#list .gcard[data-id]').length");
check("game cards", cards);
if (!cards) { cleanup(); process.exit(1); }

// A real double-click at real coordinates, dispatched by the browser.
const at = JSON.parse(await js(`(() => {
  const c = document.querySelector("#list .gcard[data-id]");
  const r = c.getBoundingClientRect();
  return JSON.stringify({ x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2), id: c.dataset.id });
})()`));
logs.length = 0;
for (const clickCount of [1, 2]) {
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: at.x, y: at.y, button: "left", clickCount, buttons: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: at.x, y: at.y, button: "left", clickCount, buttons: 0 });
  await sleep(70);
}
await sleep(10000);

console.log(`\ndouble-clicked ${at.id} at ${at.x},${at.y}`);
check("stage opened", await js("document.getElementById('ejs-stage') ? 'yes' : 'no'"), "yes");
check("emulator canvas", await js("document.querySelectorAll('#ejs-stage canvas').length"), 1);
check("shader picker", await js("document.querySelectorAll('#ejs-stage .ejs-shader option').length"), 5);

// And that the picker does something, which a missing method silently did not.
await js(`(() => { const s = document.querySelector("#ejs-stage .ejs-shader select");
  s.value = "crt-easymode.glslp"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`);
await sleep(3000);
check("shader applied", await js(`window.EJS_emulator?.getSettingValue?.("shader")`), "crt-easymode.glslp");

const errs = logs.filter((l) => l.startsWith("EXCEPTION"));
check("no exceptions", errs.length ? errs.join(" | ") : "none", "none");

if (shotAt) {
  const shot = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(shotAt, Buffer.from(shot.data, "base64"));
  console.log(`\n  screenshot: ${shotAt}`);
}
cleanup();
process.exit(failed ? 1 : 0);

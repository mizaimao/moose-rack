// Boot our Windows 98 disk in js-dos with no game, say Yes to the restart
// Windows asks for, and save the disk blocks that changed, so the registry it
// wrote can be read outside the browser.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync, appendFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
const [url, id, out] = process.argv.slice(2);
const PORT = 9334;
const profile = mkdtempSync(join(tmpdir(), "moose-w9x-cap-"));
const chrome = spawn("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", [
  "--headless=new", `--remote-debugging-port=${PORT}`, "--no-first-run", "--no-default-browser-check",
  `--user-data-dir=${profile}`, "--mute-audio", "--ignore-certificate-errors", "--window-size=1280,900", "about:blank",
], { stdio: "ignore" });
const cleanup = () => { try { chrome.kill(); } catch {} try { rmSync(profile, { recursive: true, force: true }); } catch {} };
process.on("exit", cleanup);
setTimeout(() => { console.log("hard timeout"); process.exit(3); }, 330_000);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let page;
for (let i = 0; i < 40 && !page; i++) { try { page = (await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json()).find((t) => t.type === "page"); } catch {} if (!page) await sleep(500); }
const ws = new WebSocket(page.webSocketDebuggerUrl, { maxPayload: 512 * 1024 * 1024 });
let n = 0; const pending = new Map(); const logs = [];
await new Promise((r) => (ws.onopen = r));
ws.onmessage = (m) => { const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg.result ?? msg.error); pending.delete(msg.id); }
  if (msg.method === "Runtime.exceptionThrown") logs.push("EXCEPTION: " + (msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text ?? "").split("\n")[0]); };
const send = (method, params = {}) => new Promise((res) => { const k = ++n; pending.set(k, res); ws.send(JSON.stringify({ id: k, method, params })); });
const js = async (e) => { const r = await send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true }); if (r?.exceptionDetails) return "ERR " + (r.exceptionDetails.exception?.description ?? r.exceptionDetails.text); return r?.result?.value; };
const snap = async (path) => { const s = await send("Page.captureScreenshot", { format: "png" }); writeFileSync(path, Buffer.from(s.data, "base64")); console.log("screenshot", path); };
const key = async (code, vk) => { for (const type of ["rawKeyDown", "keyUp"]) { await send("Input.dispatchKeyEvent", { type, code, key: code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk }); await sleep(120); } };
await send("Runtime.enable"); await send("Page.enable");
await send("Page.navigate", { url }); await sleep(7000);
console.log("start:", await js(`(async () => {
  const m = await import("/js/win9x-player.js");
  const g = m.parseW9x(await (await fetch("/rom?id=${id}")).text());
  const conf = g.conf.replace(/\\[autoexec\\][\\s\\S]*$/i, "[autoexec]\\nimgmount c c.vhd\\nBOOT -l c\\n");
  const base = new Uint8Array(await (await fetch(m.win9xUrl(g.base))).arrayBuffer());
  window.__base = base.slice();
  await new Promise((res, rej) => { const s = document.createElement("script"); s.src = "/jsdos/8.4.1/js-dos.js"; s.onload = res; s.onerror = rej; document.body.appendChild(s); });
  const el = document.createElement("div"); el.style.cssText = "position:fixed;inset:0;"; document.body.appendChild(el);
  Dos(el, { dosboxConf: conf, initFs: [{ path: "c.vhd", contents: base }], backend: "dosboxX", backendLocked: true,
    pathPrefix: "/jsdos/8.4.1/emulators/", autoStart: true, noCloud: true, noNetworking: true, thinSidebar: true,
    fsChanges: { local: false }, onEvent: (e, ci) => { if (e === "ci-ready") window.__ci = ci; } });
  return "base " + base.length + " bytes";
})()`));
await sleep(80_000);
console.log("ci:", await js("String(Boolean(window.__ci))"));
await snap(out + "-1.png");
// Click the screen so js-dos has the keyboard, then Enter: "Yes", restart.
await send("Input.dispatchMouseEvent", { type: "mousePressed", x: 640, y: 450, button: "left", clickCount: 1, buttons: 1 });
await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 640, y: 450, button: "left", clickCount: 1, buttons: 0 });
await sleep(1000);
await key("Enter", 13);
await sleep(100_000);
await snap(out + "-2.png");
console.log("diff:", await js(`(async () => {
  const now = await window.__ci.fsReadFile("c.vhd");
  const was = window.__base, S = 65536, parts = [];
  for (let o = 0; o < now.length; o += S) {
    const a = now.subarray(o, Math.min(o + S, now.length));
    const b = was.subarray(o, Math.min(o + S, was.length));
    let same = a.length === b.length;
    for (let i = 0; same && i < a.length; i++) if (a[i] !== b[i]) same = false;
    if (!same) parts.push(o);
  }
  window.__now = now; window.__parts = parts;
  return JSON.stringify({ length: now.length, was: was.length, changed: parts.length });
})()`));
const parts = JSON.parse(await js("JSON.stringify(window.__parts)"));
writeFileSync(out + ".patch", "");
for (const o of parts) {
  const b64 = await js(`(() => { const a = window.__now.subarray(${o}, ${o} + 65536); let s = ""; for (let i = 0; i < a.length; i += 8192) s += String.fromCharCode(...a.subarray(i, i + 8192)); return btoa(s); })()`);
  appendFileSync(out + ".patch", `${o} ${b64}\n`);
}
console.log("wrote", parts.length, "chunks to", out + ".patch");
for (const l of logs.slice(-10)) console.log(l);
cleanup(); process.exit(0);

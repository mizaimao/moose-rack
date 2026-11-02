// Play links: the web page hands a game it can't run to the desktop app as
// `moose-rack://play/<platform>/<file>`, and the desktop app starts it.
// The Rust side is `src/playlink.rs`; the expected address below is the same
// one its tests read.

import { test, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, links, web, desk;
const invoked = [];
const opened = [];
// Which backend the stub plays: the library service, or the desktop app.
let mode = "web";
let pending = [];
let library = {};
let windows = 0;
const downloaded = new Set();

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "https://dev.lan/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  global.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });

  const games = {
    7: { id: 7, name: "Super Game", platform_slug: "snes", fs_name: "Super Game (USA).sfc" },
    9: { id: 9, name: "Halo 3", platform_slug: "xbox360", fs_name: "Halo 3 (USA).iso" },
  };
  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        if (cmd === "status") return { can_launch: mode !== "web" };
        if (cmd === "rom_detail") return { ...games[args.id], downloaded: downloaded.has(args.id) };
        if (cmd === "take_play_link") return pending.shift() ?? null;
        if (cmd === "find_rom_by_file") return library[`${args.platform}/${args.file}`] ?? null;
        if (cmd === "launch_rom") return "Played";
        if (cmd === "download_rom") return "Downloaded";
        if (cmd === "launch_checklist") return checklistFor(args.id);
        if (cmd === "prepare_launch") {
          downloaded.add(args.id);
          return "downloaded";
        }
        return null;
      },
    },
    event: { listen: async () => () => {} },
  };

  // Press the checklist's main button (Play or Download) whenever it shows,
  // and count how often it does.
  new dom.window.MutationObserver(() => {
    const b = dom.window.document.querySelector("#conflict-overlay .cl-main");
    if (b && !b.dataset.counted) {
      b.dataset.counted = "1";
      windows++;
    }
    if (b) setTimeout(() => b.click(), 0);
  }).observe(dom.window.document.body, { childList: true });

  links = await import("../js/playlink.js");
  links.opener.open = (url) => opened.push(url);
  // Separate copies, because each remembers what `status` said the first time.
  web = await import("../js/actions.js?web");
  desk = await import("../js/actions.js?desk");
});

beforeEach(() => {
  windows = 0;
  invoked.length = 0;
  opened.length = 0;
  pending = [];
  library = {};
});

/// A checklist: the game is ready once it has been "downloaded".
function checklistFor(id) {
  const ready = downloaded.has(id);
  const item = (r) => ({ label: "x", detail: "y", ready: r, bytes: 1024 });
  return { name: "Game", platform: "ps2", game: item(ready), emulator: item(true), ready, download_bytes: ready ? 0 : 1024, blocked: null };
}

const toastText = () => dom.window.document.getElementById("toast").textContent;
const cmds = () => invoked.map((i) => i.cmd);

test("the link is the one src/playlink.rs reads", () => {
  assert.equal(
    links.playLink("ps2", "Ratchet & Clank (USA).chd"),
    "moose-rack://play/ps2/Ratchet%20%26%20Clank%20(USA).chd",
  );
});

test("on the web page, a game the browser can't run goes to the app", async () => {
  mode = "web";
  await web.launch(9);
  assert.deepEqual(opened, ["moose-rack://play/xbox360/Halo%203%20(USA).iso"]);
  assert.ok(!cmds().includes("launch_rom"), `launched on the server: ${cmds()}`);
  assert.match(toastText(), /Opening in Moose Rack/);
});

test("on the web page, a game the browser can run stays in the page", async () => {
  mode = "web";
  await web.launch(7);
  assert.deepEqual(opened, []);
});

function fakeWindow() {
  const handlers = {};
  return {
    handlers,
    addEventListener: (type, fn) => (handlers[type] = fn),
    removeEventListener: (type) => delete handlers[type],
    later: null,
    setTimeout(fn) {
      this.later = fn;
    },
  };
}

test("says so when nothing took the link", () => {
  const win = fakeWindow();
  const doc = { addEventListener() {}, removeEventListener() {}, hasFocus: () => true };
  const said = [];
  links.openInApp("xbox360", "Halo 3 (USA).iso", { toast: (m) => said.push(m), win, doc });
  win.later();
  assert.equal(said.length, 1);
  assert.match(said[0], /didn't open/);
});

test("stays quiet when the app or the browser's prompt took focus", () => {
  const win = fakeWindow();
  const doc = { addEventListener() {}, removeEventListener() {}, hasFocus: () => true };
  const said = [];
  links.openInApp("xbox360", "Halo 3 (USA).iso", { toast: (m) => said.push(m), win, doc });
  win.handlers.blur();
  win.later();
  assert.deepEqual(said, []);
});

test("on the desktop, a link that arrives downloads the game if needed, then starts it", async () => {
  mode = "desk";
  pending = [{ platform: "xbox360", file: "Halo 3 (USA).iso" }];
  library = { "xbox360/Halo 3 (USA).iso": 9 };
  await desk.watchPlayLinks();
  const order = cmds();
  assert.ok(order.includes("prepare_launch"), `the checklist didn't download first: ${order}`);
  const launched = invoked.find((i) => i.cmd === "launch_rom");
  assert.ok(launched, `no launch: ${order}`);
  assert.equal(launched.args.id, 9);
  assert.ok(order.indexOf("launch_checklist") < order.indexOf("prepare_launch"));
  assert.ok(order.indexOf("prepare_launch") < order.indexOf("launch_rom"));
  assert.ok(!order.includes("download_rom"), "the old download path ran too");
  assert.equal(windows, 1, "one window, for the download; none once it was ready");
  downloaded.delete(9);
  assert.ok(!order.includes("sync_library"));
});

test("on the desktop, a game already in the cache starts without a download", async () => {
  mode = "desk";
  pending = [{ platform: "snes", file: "Super Game (USA).sfc" }];
  library = { "snes/Super Game (USA).sfc": 7 };
  downloaded.add(7);
  try {
    await desk.watchPlayLinks();
  } finally {
    downloaded.delete(7);
  }
  assert.ok(!cmds().includes("prepare_launch"), `downloaded again: ${cmds()}`);
  assert.ok(cmds().includes("launch_checklist"));
  assert.equal(windows, 0, "nothing to fetch, so no window");
  assert.equal(invoked.find((i) => i.cmd === "launch_rom")?.args.id, 7);
});

test("on the desktop, a game it doesn't know updates the library first, then says so", async () => {
  mode = "desk";
  pending = [{ platform: "xbox360", file: "Unknown (USA).iso" }];
  await desk.watchPlayLinks();
  assert.ok(cmds().includes("sync_library"), `no library update: ${cmds()}`);
  assert.ok(!cmds().includes("launch_rom"));
  assert.match(toastText(), /Not in this library: Unknown \(USA\)\.iso/);
});

// Chrome on an Android phone, opening the library service.
//
// MOBILE is any Android user agent, and the Android launch path is for the
// app, the one with the Kotlin bridge. A phone's browser has no bridge, so
// pressing Play there went the app's way and failed with "this build has no
// way to start an emulator". In-page play was never reached.

import { test, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, mod;
const invoked = [];

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "https://dev.lan/",
    pretendToBeVisual: true,
  });
  // Set before any module reads it: state.js decides MOBILE at import.
  Object.defineProperty(dom.window.navigator, "userAgent", {
    value: "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/129.0 Mobile Safari/537.36",
    configurable: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  global.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });

  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        // The library service: it has RetroArch, and no screen to put it on.
        if (cmd === "status") return { can_launch: false };
        if (cmd === "rom_detail") return { id: 7, name: "Game", platform_slug: "snes" };
        return null;
      },
    },
    event: { listen: async () => () => {} },
  };

  mod = await import("../js/actions.js");
});

test("Play in an Android browser goes to the page, not to the app's bridge", async () => {
  await mod.launch(7);
  const cmds = invoked.map((i) => i.cmd);
  assert.ok(!cmds.includes("android_sync_before"), `went the app's way: ${cmds}`);
  assert.ok(!cmds.includes("android_launch_plan"), `went the app's way: ${cmds}`);
  assert.ok(cmds.includes("rom_detail"), `in-page play was not reached: ${cmds}`);
  assert.doesNotMatch(
    dom.window.document.getElementById("toast").textContent,
    /no way to start an emulator/,
  );
});

// Leaving Settings went to /index.html, which the service does not route, so
// the phone showed a 404 page instead of the library.
test("leaving Settings in a browser goes to the address the library is served at", async () => {
  const { libraryAddress } = await import("../js/settings.js");
  globalThis.__MOOSE_WEB = true;
  try {
    assert.equal(libraryAddress(), "/");
  } finally {
    delete globalThis.__MOOSE_WEB;
  }
  // The Android app loads its pages as files, and there it is index.html.
  assert.equal(libraryAddress(), "index.html");
  // And it is where the settings window actually goes.
  const src = readFileSync(join(uiDir, "js", "settings-window.js"), "utf8");
  assert.match(src, /location\.href = libraryAddress\(\)/);
});

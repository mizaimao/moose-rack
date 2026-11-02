// A window that gets no animation frames -- covered, on another desktop, a
// sleeping screen -- still launches. The refresh measurement waited for 24
// frames with no limit, so a play link that brought such a window up never
// reached launch_rom.

import { test, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const invoked = [];
let mod;

before(async () => {
  const dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "https://dev.lan/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  // No frames, ever.
  global.requestAnimationFrame = () => 0;
  dom.window.requestAnimationFrame = () => 0;
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });
  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        if (cmd === "status") return { can_launch: true };
        if (cmd === "launch_rom") return "Pac-Man exited cleanly";
        return null;
      },
    },
    event: { listen: async () => () => {} },
  };
  mod = await import("../js/actions.js");
});

test("a launch goes ahead without a refresh rate when the window gets no frames", async () => {
  const started = Date.now();
  await mod.launch(3);
  const launched = invoked.find((i) => i.cmd === "launch_rom");
  assert.ok(launched, `never launched: ${invoked.map((i) => i.cmd)}`);
  assert.equal(launched.args.refresh, null);
  assert.ok(Date.now() - started < 5000, "waited too long for frames");
});

// A game that plays in a standalone emulator the app hasn't installed yet:
// launch_rom answers EMULATOR_MISSING, the page asks, installs on a yes, and
// starts the same game again. Rust side: `launch_standalone` in
// src-tauri/src/lib.rs and src/standalone.rs.

import { test, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, mod;
const invoked = [];
let installed = false;

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

  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        if (cmd === "status") return { can_launch: true };
        if (cmd === "launch_rom") {
          if (!installed) throw "EMULATOR_MISSING:pcsx2:PCSX2:v2.8.2";
          return "Ico exited cleanly";
        }
        if (cmd === "install_emulator") {
          installed = true;
          return "installed PCSX2 v2.8.2";
        }
        return null;
      },
    },
    event: { listen: async () => () => {} },
  };
  mod = await import("../js/actions.js");
});

beforeEach(() => {
  invoked.length = 0;
  installed = false;
});

const cmds = () => invoked.map((i) => i.cmd);

/// Answer the dialog once it's on screen.
async function answer(go) {
  for (let i = 0; i < 200; i++) {
    const b = dom.window.document.querySelector(`#conflict-overlay [data-go="${go}"]`);
    if (b) {
      assert.match(dom.window.document.querySelector("#conflict-overlay h2").textContent, /PCSX2 isn't installed/);
      return b.click();
    }
    await new Promise((r) => setTimeout(r, 5));
  }
  assert.fail("the install question never appeared");
}

test("a yes installs the emulator and starts the game", async () => {
  const run = mod.launch(5);
  await answer("yes");
  await run;
  assert.deepEqual(
    cmds().filter((c) => ["launch_rom", "install_emulator"].includes(c)),
    ["launch_rom", "install_emulator", "launch_rom"],
  );
  assert.equal(invoked.find((i) => i.cmd === "install_emulator").args.id, "pcsx2");
  assert.match(dom.window.document.getElementById("toast").textContent, /exited cleanly/);
});

test("a no installs nothing", async () => {
  const run = mod.launch(5);
  await answer("no");
  await run;
  assert.ok(!cmds().includes("install_emulator"));
  assert.equal(cmds().filter((c) => c === "launch_rom").length, 1);
  assert.match(dom.window.document.getElementById("toast").textContent, /Launch cancelled/);
});

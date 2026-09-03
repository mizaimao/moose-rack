// The one tag in the top-right corner.
//
// It used to be four facts joined by dots — server, rom count, core count,
// disk usage — across the whole corner, three of which never change and none
// of which is read more than once a week. The one that matters is which server
// this is talking to, or that it is not talking to one.

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, status;

const STATUS = {
  configured: true,
  connected: true,
  server: "https://romm.lan:8080/",
  roms_cached: 2506,
  retroarch: true,
  cores_installed: 41,
  config_path: "/x/config.toml",
  data_dir: "/x",
  roms_dir: "/x/library",
  media_dir: "/x/media",
  crowded_folder: false,
};

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.localStorage = dom.window.localStorage;
  global.HTMLElement = dom.window.HTMLElement;
  global.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  global.requestAnimationFrame = (f) => f();
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });
  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd) =>
        cmd === "status" ? STATUS : cmd === "disk_usage" ? 210_000_000_000 : [],
      convertFileSrc: (p) => p,
    },
    event: { listen: async () => () => {}, emit: () => {} },
  };
  await import("../js/main.js");
  await new Promise((r) => setTimeout(r, 40));
  status = dom.window.document.getElementById("status");
});

describe("the server tag", () => {
  /// The scheme in front of a LAN address is six characters saying nothing,
  /// and the trailing slash is noise.
  test("it says which server, and nothing else", () => {
    assert.equal(status.textContent, "romm.lan:8080");
    assert.equal(status.dataset.state, "on");
  });

  /// "offline" and a server name are the same shape of word, and the
  /// difference between them is the entire point of the tag.
  test("the state has a color of its own, not just a word", () => {
    const css = readFileSync(join(uiDir, "style.css"), "utf8");
    for (const state of ["on", "off", "unset"]) {
      assert.ok(
        css.includes(`#status[data-state="${state}"]::before`),
        `nothing marks the ${state} state`
      );
    }
  });

  /// A panel of our own rather than a `title` attribute: the tooltip took a
  /// second to appear, went away on its own, could not be styled, and is why
  /// the details in it were effectively invisible.
  test("the rest is there on hover", () => {
    status.dispatchEvent(new dom.window.MouseEvent("pointerenter", { bubbles: true }));
    const card = dom.window.document.getElementById("status-card");
    assert.ok(card, "there is no panel");
    assert.equal(card.hidden, false, "the panel did not open");
    const text = card.textContent.replace(/\s+/g, " ");
    assert.match(text, /2506/, "the game count is not in it");
    assert.match(text, /41 cores/, "the cores are not in it");
    assert.match(text, /\/x\/library/, "the folders are not in it");
    // The tooltip it replaces has to go, or hovering gives both.
    assert.ok(!status.title, "the old tooltip is still attached as well");
  });

  /// Measured separately and filled in when it arrives, because measuring it
  /// walks every file in the library. On Android that is seconds, and a command
  /// that takes seconds there freezes the page for seconds — the IPC is served
  /// on the calling thread. So the row opens with a placeholder rather than the
  /// panel waiting on a disk walk.
  test("the disk figure arrives on its own and does not hold the panel up", async () => {
    const card = dom.window.document.getElementById("status-card");
    const cell = card.querySelector(".sc-disk");
    assert.ok(cell, "there is no cell for it");
    // A tick for the promise the card started when it was built.
    await new Promise((r) => setTimeout(r, 0));
    assert.match(cell.textContent, /195\.6 GB/, "the disk usage never arrived");
  });

  test("it closes again", () => {
    status.dispatchEvent(new dom.window.MouseEvent("pointerleave", { bubbles: true }));
    assert.equal(dom.window.document.getElementById("status-card").hidden, true);
  });
});

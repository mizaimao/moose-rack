// The shim `src-service/src/web.rs` injects, run rather than read.
//
// It is JavaScript that lives inside a Rust string, so nothing else in either
// test suite touches it: the Rust side can only assert that some substring is
// present, and the JS side never loads it. That gap is how `event` stayed a
// pair of do-nothing stubs while the desktop relied on it for six settings --
// the backdrop switch, its frame rate, glass tint, glass strength, collection
// art and attract mode. Every one of them silently did nothing in a browser.
//
// So this reads the constant out of the Rust source and evaluates it.

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

/// The body of `pub const SHIM: &str = r#"…"#;`.
function shimSource() {
  const rs = readFileSync(join(root, "src-service/src/web.rs"), "utf8");
  const start = rs.indexOf('pub const SHIM: &str = r#"');
  assert.notEqual(start, -1, "SHIM is not declared the way this test reads it");
  const from = rs.indexOf('"#', start + 26);
  return rs.slice(rs.indexOf("\n", start) + 1, from);
}

let dom, src;

before(() => {
  src = shimSource();
  dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://dev.lan:8001/",
    runScripts: "outside-only",
  });
});

describe("the web shim", () => {
  test("defines what state.js destructures at import time", () => {
    dom.window.eval(src);
    const core = dom.window.__TAURI__?.core;
    assert.ok(core, "window.__TAURI__.core is missing");
    assert.equal(typeof core.invoke, "function");
    assert.equal(typeof core.convertFileSrc, "function");
    assert.equal(typeof dom.window.__TAURI__.event.listen, "function");
    assert.equal(typeof dom.window.__TAURI__.event.emit, "function");
  });

  test("a local path becomes a URL the media route answers", () => {
    dom.window.eval(src);
    const url = dom.window.__TAURI__.core.convertFileSrc("/media/snes/A B (USA).png");
    assert.match(url, /^\/media\?path=/);
    assert.ok(url.includes("%20"), `spaces must be encoded: ${url}`);
  });

  /// Settings is a second tab. What it emits has to reach the library page, or
  /// the backdrop switch is a button that does nothing.
  test("an event emitted in one document reaches a listener in another", async () => {
    const library = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://dev.lan:8001/", runScripts: "outside-only",
    });
    const settings = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://dev.lan:8001/settings.html", runScripts: "outside-only",
    });
    // One channel implementation shared by both, the way one browser shares it
    // between two tabs of an origin. jsdom gives each document its own.
    const bus = new Set();
    for (const w of [library.window, settings.window]) {
      w.BroadcastChannel = class {
        constructor() { this.listeners = new Set(); bus.add(this); }
        addEventListener(_, fn) { this.listeners.add(fn); }
        postMessage(data) {
          for (const c of bus) if (c !== this) for (const fn of c.listeners) fn({ data });
        }
      };
      w.eval(src);
    }

    const seen = [];
    const unlisten = await library.window.__TAURI__.event.listen("backdrop-toggle", (e) =>
      seen.push(e.payload)
    );
    await settings.window.__TAURI__.event.emit("backdrop-toggle", true);
    assert.deepEqual(seen, [true], "the library page never heard the toggle");

    // And it can be turned off again.
    unlisten();
    await settings.window.__TAURI__.event.emit("backdrop-toggle", false);
    assert.deepEqual(seen, [true], "unlisten did not stop delivery");
  });

  /// A browser without BroadcastChannel must degrade, not throw: `state.js`
  /// destructures `listen` at module load, so a throw here is a blank page.
  test("without BroadcastChannel it still loads", async () => {
    const w = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://dev.lan:8001/", runScripts: "outside-only",
    }).window;
    delete w.BroadcastChannel;
    w.eval(src);
    const off = await w.__TAURI__.event.listen("x", () => {});
    await w.__TAURI__.event.emit("x", 1);
    off();
  });
});

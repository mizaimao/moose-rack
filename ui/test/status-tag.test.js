// What the tag in the top-right corner says.
//
// Three meanings, and the backend can only distinguish one of them. It reports
// the same `status` whether you are sitting at the machine or looking at it
// from across the house: a library on this disk and no upstream server. Only
// the browser knows which address was typed, so the split is decided in the UI
// and tested here rather than against a running service.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { statusTag, viewerIsRemote, secureLink, linkLabel } from "../js/status-tag.js";

const loc = (protocol, hostname) => ({ protocol, hostname });

describe("where the viewer is", () => {
  test("the desktop app is never remote", () => {
    // Tauri serves its own window over its own protocol, not over http.
    assert.equal(viewerIsRemote(loc("tauri:", "tauri.localhost")), false);
    assert.equal(viewerIsRemote(loc("asset:", "")), false);
    assert.equal(viewerIsRemote(loc("file:", "")), false);
    assert.equal(viewerIsRemote(undefined), false);
  });

  test("a browser on the machine itself is not remote", () => {
    for (const h of ["localhost", "127.0.0.1", "::1", "[::1]", "0.0.0.0", "app.localhost"]) {
      assert.equal(viewerIsRemote(loc("http:", h)), false, h);
    }
  });

  test("a browser anywhere else is", () => {
    for (const h of ["dev.lan", "192.168.1.10", "moose.home.arpa", "DEV.LAN"]) {
      assert.equal(viewerIsRemote(loc("http:", h)), true, h);
    }
    assert.equal(viewerIsRemote(loc("https:", "dev.lan")), true);
  });
});

describe("what the tag says", () => {
  const configured = { configured: true, connected: false, server: "" };

  test("a server it syncs from is named, wherever you are looking from", () => {
    const s = { configured: true, connected: true, server: "http://dev.lan:8001/" };
    for (const remote of [true, false]) {
      assert.deepEqual(statusTag(s, remote), { text: "dev.lan:8001", state: "on" });
    }
  });

  /// Sitting at the machine, with no server behind it. Nothing is wrong; there
  /// is simply nothing to be online with.
  test("offline when you are at the machine and there is no upstream", () => {
    assert.deepEqual(statusTag(configured, false), { text: "offline", state: "off" });
  });

  /// The case this whole service exists for, and the one that used to read
  /// "offline" while happily serving 11,867 games across the house.
  test("server mode when the same backend is viewed from elsewhere", () => {
    assert.deepEqual(statusTag(configured, true), { text: "server mode", state: "serving" });
  });

  test("its own state, so the dot can be blue rather than green or orange", () => {
    const states = new Set(
      [
        statusTag(configured, true).state,
        statusTag(configured, false).state,
        statusTag({ configured: true, connected: true, server: "x" }, false).state,
      ]
    );
    assert.equal(states.size, 3, "serving must not share a colour with on or off");
  });

  /// A missing config is the owner's problem and is not actionable by someone
  /// looking at the page from another machine -- for them the service plainly
  /// works, so telling them to go and create a file is noise.
  test("a remote viewer is not told to create a config file", () => {
    const s = { configured: false, connected: false, server: "" };
    assert.deepEqual(statusTag(s, false), { text: "no config.toml", state: "unset" });
    assert.deepEqual(statusTag(s, true), { text: "server mode", state: "serving" });
  });
});

describe("whether the link is encrypted", () => {
  const up = (server) => ({ configured: true, connected: true, server });

  /// The same service answers on both, so only the address bar can tell.
  test("in a browser, the page's own address decides", () => {
    assert.equal(secureLink(up(""), loc("https:", "dev.lan")), true);
    assert.equal(secureLink(up(""), loc("http:", "dev.lan")), false);
    assert.equal(secureLink(up(""), loc("http:", "localhost")), false);
  });

  test("in the desktop app, the server in config.toml decides", () => {
    const app = loc("tauri:", "tauri.localhost");
    assert.equal(secureLink(up("https://dev.lan"), app), true);
    assert.equal(secureLink(up("HTTPS://dev.lan/"), app), true);
    assert.equal(secureLink(up("http://dev.lan"), app), false);
  });

  /// "Not encrypted" would describe a link that does not exist.
  test("with no server there is nothing to describe", () => {
    const offline = { configured: true, connected: false, server: "https://dev.lan" };
    assert.equal(secureLink(offline, loc("tauri:", "tauri.localhost")), null);
    assert.equal(linkLabel(null), "no server");
  });

  test("the card says which, in words", () => {
    assert.equal(linkLabel(true), "Encrypted (HTTPS)");
    assert.equal(linkLabel(false), "Not encrypted (HTTP)");
  });

  test("an encrypted link gets the gold border, and it holds still on request", () => {
    const css = readFileSync(new URL("../style.css", import.meta.url), "utf8");
    assert.match(css, /#status\[data-secure="yes"\]::after\s*\{[^}]*animation: status-gold-glint/);
    assert.match(
      css,
      /prefers-reduced-motion: reduce\)\s*\{\s*#status\[data-secure="yes"\]::after \{ animation: none; \}/
    );
  });
});

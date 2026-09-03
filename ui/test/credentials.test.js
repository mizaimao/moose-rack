// The credentials dialog.
//
// The one part of the settings window where getting it wrong is not a cosmetic
// problem: a password in a form is on screen whenever the pane is, and a pane
// that saves on blur writes a half-typed token the moment focus moves. This
// checks the two rules that make it safe — nothing stored is ever shown, and
// nothing is written until Save — because both are invisible in a screenshot
// and neither had a test.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, creds, invoked;

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "settings.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  Object.defineProperty(global, "navigator", { value: dom.window.navigator, configurable: true });
  invoked = [];
  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        return "saved";
      },
      convertFileSrc: (p) => p,
    },
    event: { listen: async () => () => {}, emit: () => {} },
  };
  creds = await import("../js/credentials.js");
});

beforeEach(() => {
  invoked.length = 0;
  document.getElementById("conflict-overlay")?.remove();
});

const dialog = () => document.getElementById("conflict-overlay");

describe("editing a credential", () => {
  /// A settings pane has no reason to hand a stored password to a webview so
  /// it can draw it. The dialog says whether one is set; it never says what.
  test("what is already stored is never put on screen", () => {
    creds.credentialDialog({
      title: "Server",
      fields: [
        { field: "url", label: "Address", value: "http://dev.lan" },
        { field: "password", label: "Password", secret: true, isSet: true },
      ],
    });
    const html = dialog().innerHTML;
    assert.match(html, /http:\/\/dev\.lan/, "an ordinary field should keep its value");
    assert.doesNotMatch(html, /hunter2|value="[^"]*secret/i);
    const box = dialog().querySelector('[data-field="password"]');
    assert.equal(box.value, "", "the stored password was written into the field");
    assert.equal(box.type, "password", "it would be readable over a shoulder");
    // It says one is set without saying what it is.
    assert.match(box.placeholder, /already set/);
  });

  /// Nothing is written on the way past. The whole reason this is a dialog and
  /// not a row in the pane.
  test("typing writes nothing until Save", async () => {
    creds.credentialDialog({
      title: "Server",
      fields: [{ field: "password", label: "Password", secret: true }],
    });
    const box = dialog().querySelector('[data-field="password"]');
    box.value = "half-typed";
    box.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    box.dispatchEvent(new dom.window.Event("blur", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 10));
    assert.deepEqual(invoked, [], "a half-typed credential was saved on the way past");
  });

  test("Cancel leaves nothing behind", async () => {
    creds.credentialDialog({
      title: "Server",
      fields: [{ field: "password", label: "Password", secret: true }],
    });
    dialog().querySelector(".cred-cancel").click();
    await new Promise((r) => setTimeout(r, 10));
    assert.deepEqual(invoked, []);
  });
});

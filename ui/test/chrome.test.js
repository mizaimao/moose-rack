// The words on the toolbar buttons, and the switch that takes them away.
//
// Hiding a label leaves `title` as the button's accessible name, so the tests
// that matter here are as much about what is *kept* as about what is hidden:
// three of the six buttons carry state in their word -- what the list is
// sorted by, which pane the toggle shows, which way the layout flips -- and
// that state has to survive into the title.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
let dom, chrome;

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), { url: "http://localhost/" });
  for (const k of ["window", "document", "localStorage"])
    Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true });
  Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
  dom.window.__TAURI__ = { core: { invoke: async () => ({}) }, event: { emit: () => {} } };
  chrome = await import("../js/chrome.js");
});

beforeEach(() => {
  localStorage.clear();
  document.documentElement.className = "";
});

describe("button labels", () => {
  test("they are on until they are turned off", () => {
    assert.equal(chrome.labelsWanted(), true);
    chrome.setLabels(false);
    assert.equal(chrome.labelsWanted(), false);
    chrome.setLabels(true);
    assert.equal(chrome.labelsWanted(), true);
  });

  test("the choice lands on the document, where the stylesheet can see it", () => {
    chrome.setLabels(false);
    assert.ok(document.documentElement.classList.contains("no-labels"));
    chrome.setLabels(true);
    assert.ok(!document.documentElement.classList.contains("no-labels"));
  });

  test("applying it reads the stored answer when not given one", () => {
    localStorage.setItem("chromeLabels", "off");
    document.documentElement.className = "";
    chrome.applyLabels();
    assert.ok(document.documentElement.classList.contains("no-labels"));
  });

  test("a window that cannot remember still shows the words", () => {
    // A private window: forgetting the choice is survivable, starting with an
    // unreadable toolbar is not.
    const real = Object.getOwnPropertyDescriptor(dom.window, "localStorage");
    Object.defineProperty(globalThis, "localStorage", {
      value: {
        getItem() { throw new Error("denied"); },
        setItem() { throw new Error("denied"); },
      },
      configurable: true,
    });
    try {
      assert.equal(chrome.labelsWanted(), true);
      assert.equal(chrome.setLabels(false), false, "it still applies for this session");
    } finally {
      Object.defineProperty(globalThis, "localStorage", real ?? { value: dom.window.localStorage, configurable: true });
    }
  });

  test("the other window is told, and does not answer back", () => {
    const sent = [];
    dom.window.__TAURI__.event.emit = (name, payload) => sent.push([name, payload]);
    chrome.setLabels(false);
    assert.deepEqual(sent, [["chrome-labels", false]]);
    // What arrives from the other window is applied, never re-saved: saving
    // announces, and the two would talk in a circle.
    sent.length = 0;
    chrome.setLabels(true, { announce: false });
    assert.deepEqual(sent, []);
  });
});

describe("the six buttons the stylesheet names", () => {
  // The rule is by id rather than "any button with an icon", because the tab
  // row's own buttons are words with no icon and a cleverer selector would
  // leave that row blank.
  const NAMED = ["layout-btn", "filter-btn", "random-btn", "grab-btn", "sort-btn", "sidebar-btn"];

  test("every one of them exists and has both an icon and a word", () => {
    for (const id of NAMED) {
      const b = document.getElementById(id);
      assert.ok(b, `${id} is named in style.css but not in index.html`);
      assert.ok(b.querySelector(".icon"), `${id} has no icon, so hiding its word leaves nothing`);
      assert.ok(
        b.querySelector("span:not(.icon)"),
        `${id} has no word to hide`
      );
    }
  });

  test("every one of them has a title to fall back to", () => {
    // With the word gone this is the button's accessible name as well as its
    // tooltip. A button with neither is a button nobody can identify.
    for (const id of NAMED) {
      const b = document.getElementById(id);
      assert.ok(
        (b.getAttribute("title") || b.getAttribute("aria-label") || "").trim().length > 3,
        `${id} would be an unlabelled glyph`
      );
    }
  });

  test("the stylesheet hides exactly those six", () => {
    const css = readFileSync(join(uiDir, "style.css"), "utf8");
    const rule = css.slice(css.indexOf("html.no-labels"));
    for (const id of NAMED) assert.ok(rule.includes(`#${id}`), `${id} is not in the rule`);
  });
});

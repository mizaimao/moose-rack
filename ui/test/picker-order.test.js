// The order of the left column.
//
// The server hands consoles and collections back by size, so the column opened
// on "Arcade Fighting, 322 games" and buried "Best of nes" thirty rows down,
// with nothing on screen to say why or to change it.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, order;

before(async () => {
  dom = new JSDOM(`<style>${readFileSync(join(uiDir, "style.css"), "utf8")}</style><body></body>`, {
    url: "http://localhost/",
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.localStorage = dom.window.localStorage;
  global.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  // state.js reads these at import time, and this module reaches it through
  // the shared helpers.
  dom.window.__TAURI__ = {
    core: { invoke: async () => [], convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: () => {} },
  };

  // The interface commands — bindings, ordering, the grid, the page filter —
  // are answered by the stand-in in backend.js. See the note at the top of
  // that file: it is deliberately naive, and the rules it stands in for are
  // asserted by `cargo test` against the real implementation.
  dom.window.__TAURI__.core.invoke = fakeBackend(dom.window.__TAURI__.core.invoke);
  order = await import(join(uiDir, "js", "picker-order.js"));
});

beforeEach(async () => {
  dom.window.localStorage.clear();
  await order.setPickerOrder("collections", "name");
});

const consoles = [
  { name: "Arcade", rom_count: 322, playable: false },
  { name: "Nintendo 64", rom_count: 12, playable: true },
  { name: "Game Boy", rom_count: 90, playable: true },
];

const cols = [
  { name: "Arcade Fighting", rom_count: 500, local_count: 4 },
  { name: "Best of nes", rom_count: 12, local_count: 12 },
  { name: "Beta", rom_count: 90, local_count: 900 },
];

// Which orders each kind of list offers, what each one does to it, that
// favorites stay on top of any of them, and that the chosen one survives a
// restart are all asserted in `pickorder::tests` — against the implementation
// rather than through a page. What is left here is the bar above the column.

describe("the bar above the column", () => {
  /// The filter that used to sit beside this button is furniture of the tab
  /// row now — one box for every page rather than one for the only page that
  /// had it — so what is left here is the button alone.
  test("it is the order button, and nothing else", async () => {
    const doc = dom.window.document;
    await order.loadPickerOrders("collections");
    doc.body.innerHTML = order.pickerBar({ kind: "collections" });
    assert.ok(doc.querySelector(".pick-sort"), "the order button is gone");
    assert.equal(doc.querySelector("input"), null, "it still draws a filter box of its own");
  });

  test("the order button says which order it is in", async () => {
    await order.setPickerOrder("collections", "count");
    const doc = dom.window.document;
    doc.body.innerHTML = order.pickerBar({ kind: "collections" });
    assert.match(doc.querySelector(".pick-sort").textContent, /Most games/);
  });

  test("choosing an order redraws the list and relabels the button", async () => {
    const doc = dom.window.document;
    await order.loadPickerOrders("collections");
    doc.body.innerHTML = order.pickerBar({ kind: "collections" });
    let redrawn = 0;
    order.wirePickerBar(doc.body, "collections", () => redrawn++);
    doc.querySelector(".pick-sort").dispatchEvent(
      new dom.window.MouseEvent("click", { bubbles: true })
    );
    const item = [...doc.querySelectorAll(".ctx-menu button")].find((b) =>
      b.textContent.includes("Fewest games")
    );
    assert.ok(item, "the menu does not offer the orders");
    item.click();
    // The choice is written through the backend, so the label and the redraw
    // land on the turn after the click.
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(order.pickerOrder("collections").id, "fewest");
    assert.equal(redrawn, 1, "the list was not drawn again");
    // The bar is not part of the redraw, so without this the button went on
    // saying "Name" over a list sorted by something else.
    assert.match(doc.querySelector(".pick-sort").textContent, /Fewest games/);
  });
});

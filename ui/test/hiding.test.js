// Hiding things, checked against the real stylesheet.
//
// This file exists because of one bug that looked like three. `hidden` is
// implemented by the browser's own stylesheet as `[hidden] { display: none }`,
// and a user-agent rule loses to *any* author rule that sets `display` — no
// matter how weak. style.css sets one on every button that holds an icon
// beside a word, so Back, Grid and Hide info stayed on screen after the code
// had hidden them. The Grid button then redrew the last console you had open,
// which reads as a button that opens consoles by itself.
//
// None of that is visible from reading the JavaScript: the code sets `hidden`
// correctly and the element is genuinely hidden as far as the DOM is
// concerned. It has to be asked of the cascade.
//
// jsdom evaluates plain selectors but not `:has()`, so the assertions below
// cover the ID rules. That is enough to catch the regression: the fix is one
// declaration and either it is there or it is not.

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom;

/// Buttons the app hides depending on which screen is showing. Every one of
/// them carries an icon, a word, or both, which is what put them in the way of
/// the author `display` rules in the first place.
const TOGGLED = ["back", "layout-btn", "sidebar-btn", "zoom-wrap"];

before(() => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    pretendToBeVisual: true,
  });
  const style = dom.window.document.createElement("style");
  style.textContent = readFileSync(join(uiDir, "style.css"), "utf8");
  dom.window.document.head.appendChild(style);
});

const displayOf = (id) =>
  dom.window.getComputedStyle(dom.window.document.getElementById(id)).display;

describe("hidden actually hides", () => {
  for (const id of TOGGLED) {
    test(`${id} disappears when hidden`, () => {
      const node = dom.window.document.getElementById(id);
      node.hidden = true;
      assert.equal(
        displayOf(id),
        "none",
        `#${id} is still laid out while hidden — an author rule setting ` +
          `display is beating the browser's [hidden], so the screen shows a ` +
          `control that the code believes is gone`
      );
    });

    test(`${id} comes back when shown`, () => {
      const node = dom.window.document.getElementById(id);
      node.hidden = true;
      node.hidden = false;
      assert.notEqual(
        displayOf(id),
        "none",
        `#${id} stays invisible after being shown — a blanket display:none ` +
          `would hide these buttons for good`
      );
    });
  }
});

describe("the right-click menu is positioned by the stylesheet", () => {
  /// The menu shipped with `left` and `top` set inline and no `position` rule
  /// anywhere, so the browser laid it out in the normal document flow: it
  /// appeared past the end of the page, with the coordinates ignored. Nothing
  /// in the JavaScript was wrong and nothing reported anything — it simply
  /// looked like a right-click that did nothing.
  test("a menu given coordinates is taken out of the flow", () => {
    const menu = dom.window.document.createElement("div");
    menu.className = "ctx-menu";
    menu.style.left = "120px";
    menu.style.top = "90px";
    dom.window.document.body.appendChild(menu);

    const style = dom.window.getComputedStyle(menu);
    assert.equal(
      style.position,
      "fixed",
      "left and top mean nothing on a statically positioned element, so the " +
        "menu lands wherever the document flow puts it"
    );
    menu.remove();
  });
});

describe("the zoom slider changes the size at every step", () => {
  /// Reported as "multiple levels share the same icon size", which is what a
  /// grid of `minmax(X, 1fr)` columns does: the column count is
  /// floor(width / X) and the cards then stretch to fill, so the width drawn
  /// is row-width divided by column count and only moves when the count does.
  /// Most of the slider's travel drew identical cards.
  test("cards are a width, not a minimum they stretch past", () => {
    // Comments stripped first: the rule below is explained in a comment that
    // quotes the very pattern being asserted against, and matching that would
    // fail on the explanation rather than on the code.
    const css = readFileSync(join(uiDir, "style.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
    const gcards = css.slice(css.indexOf(".gcards {"), css.indexOf("}", css.indexOf(".gcards {")));
    assert.ok(
      gcards.includes("repeat(auto-fill, var(--gcard))"),
      `game cards still stretch to fill the row:\n${gcards}`
    );
    assert.ok(!/minmax\([^)]*1fr\)/.test(gcards), "a 1fr column is still in there");

    const grid = css.slice(css.indexOf(".grid {"), css.indexOf("}", css.indexOf(".grid {")));
    assert.ok(!/minmax\([^)]*1fr\)/.test(grid), "console cards still stretch");
  });

  /// A step so large that the slider has only a handful of usable positions is
  /// the other half of the same complaint.
  test("the slider moves in small enough steps to be continuous", () => {
    const html = readFileSync(join(uiDir, "index.html"), "utf8");
    const el = html.match(/<input id="zoom"[^>]*>/)[0];
    const num = (k) => Number(el.match(new RegExp(`${k}="(\\d+)"`))[1]);
    const steps = (num("max") - num("min")) / num("step");
    assert.ok(steps >= 50, `only ${steps} zoom positions, which is not continuous`);
  });
});

describe("it does not behave like a document", () => {
  /// Dragging across a game's name and watching it turn blue is the clearest
  /// tell that a window is a web view — nothing else on the desktop does that.
  test("text is not selectable", () => {
    const style = dom.window.getComputedStyle(dom.window.document.body);
    assert.equal(
      style.userSelect || style.webkitUserSelect,
      "none",
      "the library selects like a web page"
    );
  });

  /// Except where the text exists to be copied out: a path to paste somewhere,
  /// a field to type in.
  test("but the things you type in and copy from still are", () => {
    const input = dom.window.document.getElementById("search");
    const style = dom.window.getComputedStyle(input);
    assert.equal(
      style.userSelect || style.webkitUserSelect,
      "text",
      "the search field cannot be selected in"
    );
  });
});

describe("the left column is a list, whatever is in it", () => {
  /// Reported four times, and my first two attempts at it were CSS I never
  /// checked. The collections are drawn as cards — right for a page of them,
  /// wrong for a 240px column, where a grid two across leaves a stamp-sized
  /// picture above a name that does not fit. Asserted against the real
  /// stylesheet rather than by reading it.
  const inColumn = (html, sel) => {
    const doc = dom.window.document;
    doc.body.classList.add("columns");
    doc.getElementById("consoles").innerHTML = html;
    const node = doc.querySelector(`#consoles ${sel}`);
    const style = dom.window.getComputedStyle(node);
    return { node, style };
  };

  test("a collection is a line of text, not a card", () => {
    const { node, style } = inColumn(
      `<div class="grid"><div class="card" data-cid="1">
         <div class="logo mosaic"><span class="ph">Ar</span></div>
         <div class="name">Arcade Shmups Horizontal</div>
         <div class="meta">110 games<span class="here"> · 110 here</span></div>
       </div></div>`,
      ".card"
    );
    // Reported seven times. Twice I laid out the *inside* of the box and called
    // it fixed — first a row of three, then two lines beside the picture —
    // while `.card` went on drawing the box itself. What made it a card was
    // never the arrangement of the contents: it was the background, the border,
    // the 12px radius and the 14px of padding, all of which are asserted gone
    // here. A list is a line of text with nothing around it.
    assert.equal(style.display, "flex", "still not laid out as a line");
    // The eighth report, and the one thing neither earlier attempt asserted.
    // `.card` sets `flex-direction: column`; `display: flex` on its own
    // inherits that, so the name and the count went on stacking even after the
    // box around them was gone. A line is a line because it runs across.
    assert.equal(style.flexDirection, "row", "still stacked into two lines");
    assert.equal(style.borderRadius, "0px", "still has rounded corners");
    assert.equal(style.borderTopWidth || style.borderWidth, "0px", "still boxed in");
    assert.match(
      style.backgroundColor || "",
      /rgba\(0, 0, 0, 0\)|transparent|^$/,
      "still has a card's background"
    );
    assert.equal(style.padding, "5px 8px", "still padded like a card");

    const at = (sel) => dom.window.getComputedStyle(node.querySelector(sel));
    // A mosaic of covers at 22px is mush, and "Ar" stands in front of every one
    // of the eleven Arcade collections saying nothing the name does not.
    assert.equal(at(".logo").display, "none", "the stamp is still there");
    // The name gets the width; the count keeps only what it needs. "322 games ·
    // 322 here" is the same number twice for a collection fully downloaded, and
    // it was taking the room the name needed.
    assert.match(at(".name").flex, /^1 /, "the name does not take the width");
    assert.equal(at(".name").textOverflow, "ellipsis");
    assert.equal(at(".here").display, "none", "the count is still doubled");
  });

  test("and the grid of them becomes one per line", () => {
    const { style } = inColumn(
      `<div class="grid"><div class="card"></div><div class="card"></div></div>`,
      ".grid"
    );
    assert.equal(style.display, "flex");
    assert.equal(style.flexDirection, "column", "still a grid across the column");
  });

  /// "What is the green dot" is a fair question when the words that explain it
  /// have been cut off by a narrow column.
  test("the console rows drop the dot in the column and keep the words", () => {
    const { style } = inColumn(
      `<div class="rows"><div class="row prow" data-slug="snes">
         <span class="have"><span class="dot on"></span></span>
         <span class="nm">Super Nintendo</span>
         <span class="pf">snes</span>
         <span class="sz">50 games</span>
       </div></div>`,
      ".prow .have"
    );
    assert.equal(style.display, "none", "the unexplained dot is still there");
  });
});

// The artwork viewer, and the top bar's own furniture.
//
// Both are about things that move when they should not. The viewer's arrows
// were flex siblings of the picture, so they sat against whatever was on the
// stage — inwards for a cartridge, outwards for a wide screenshot — and
// stepping through a reel of mixed artwork meant chasing a button that moved
// every time it was pressed. The layout switch in the header had the same
// problem from the other side: it followed the end of the title, which is 200px
// longer for "Arcade Fighting — 322 games" than for "Platforms".

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const css = readFileSync(join(uiDir, "style.css"), "utf8");

let dom, lightbox, keys;

before(async () => {
  dom = new JSDOM(
    `<style>${css}</style>` + readFileSync(join(uiDir, "index.html"), "utf8"),
    { url: "http://localhost/", pretendToBeVisual: true }
  );
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  global.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  global.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });
  dom.window.__TAURI__ = {
    core: { invoke: async () => [], convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: () => {} },
  };
  lightbox = await import("../js/lightbox.js");
  keys = await import("../js/keys.js");
});

describe("the arrows stay where they are", () => {
  const style = (sel) => dom.window.getComputedStyle(dom.window.document.querySelector(sel));

  test("they are placed against the window, not against the picture", () => {
    for (const sel of ["#lightbox .lb-prev", "#lightbox .lb-next"]) {
      assert.equal(style(sel).position, "absolute", `${sel} still sits beside the artwork`);
      assert.equal(style(sel).top, "50%", `${sel} is not centered down the window`);
    }
    assert.equal(style("#lightbox .lb-prev").left, "14px");
    assert.equal(style("#lightbox .lb-next").right, "14px");
  });

  /// Both the same distance in, or a reel of stills has the cursor travelling
  /// further one way than the other.
  test("both are the same distance from their edge", () => {
    assert.equal(style("#lightbox .lb-prev").left, style("#lightbox .lb-next").right);
  });
});

describe("space plays and pauses", () => {
  /// The one key every video player on every platform agrees about. The
  /// alternative was aiming at a control bar that fades out.
  const stage = () => dom.window.document.querySelector("#lightbox .lb-stage");

  /// jsdom implements no media playback at all — `paused` is a getter with
  /// nothing behind it — so play and pause are stubbed and the flag is kept
  /// beside the element rather than on it.
  function fakeVideo() {
    const v = dom.window.document.createElement("video");
    const state = { paused: true };
    Object.defineProperty(v, "paused", { get: () => state.paused, configurable: true });
    v.play = () => {
      state.paused = false;
      return Promise.resolve();
    };
    v.pause = () => (state.paused = false || (state.paused = true));
    stage().replaceChildren(v);
    return v;
  }

  test("it toggles the video that is open", () => {
    const v = fakeVideo();
    assert.equal(lightbox.togglePlayback(), true, "nothing was toggled");
    assert.equal(v.paused, false, "space did not start it");
    lightbox.togglePlayback();
    assert.equal(v.paused, true, "space did not stop it");
  });

  /// A still has nothing to play, and swallowing the key there would make
  /// space dead on every other picture in the reel.
  test("on a still it does nothing and says so", () => {
    stage().replaceChildren(dom.window.document.createElement("img"));
    assert.equal(lightbox.togglePlayback(), false);
  });

  test("the key handler reaches it, and stops the page scrolling", () => {
    keys.installKeys();
    const v = fakeVideo();
    dom.window.document.getElementById("lightbox").hidden = false;
    const ev = new dom.window.KeyboardEvent("keydown", {
      key: " ",
      bubbles: true,
      cancelable: true,
    });
    dom.window.dispatchEvent(ev);
    assert.equal(v.paused, false, "space never reached the video");
    assert.equal(ev.defaultPrevented, true, "the page will scroll under the viewer");
    dom.window.document.getElementById("lightbox").hidden = true;
  });
});

describe("the layout switch keeps its place", () => {
  /// It sat immediately after the title, so it moved by the difference between
  /// "Platforms" and "Arcade Fighting — 322 games" every time the screen
  /// changed.
  test("the title takes a share of the bar, not the width of its words", () => {
    const h1 = dom.window.getComputedStyle(dom.window.document.querySelector("header h1"));
    assert.match(h1.flexBasis, /clamp|px|%/, "the title is still sized to its text");
    assert.equal(h1.textOverflow, "ellipsis", "a long name has nowhere to go");
  });

  /// Equal auto margins split the free space evenly, which is what puts it in
  /// the middle of the gap. Taking `margin-left: auto` on the search box as
  /// well would split it three ways and leave the switch a third of the way
  /// along.
  test("it is centered in the gap, and the search box does not take the slack", () => {
    const sw = dom.window.getComputedStyle(dom.window.document.getElementById("view-switch"));
    assert.equal(sw.marginLeft, "auto");
    assert.equal(sw.marginRight, "auto");
    const search = dom.window.getComputedStyle(dom.window.document.getElementById("search"));
    assert.notEqual(search.marginLeft, "auto", "the gap is split three ways");
  });

  /// Two unlabelled glyphs in a bar full of unlabelled glyphs say nothing
  /// about what they do, and this pair changes the whole shape of the window.
  /// Named for where you are rather than for the furniture: one is the
  /// arrangement you drive from a controller across the room, the other the
  /// one you drive with a mouse a foot away.
  test("each side says what it is", () => {
    const labels = [...dom.window.document.querySelectorAll("#view-switch button")].map((b) =>
      b.textContent.trim()
    );
    assert.deepEqual(labels, ["Sofa", "Desk"]);
  });

  test("it comes before the search box", () => {
    const header = dom.window.document.querySelector("header");
    const order = [...header.children].map((c) => c.id);
    assert.ok(
      order.indexOf("view-switch") < order.indexOf("search"),
      "the switch is not before the search box"
    );
  });
});

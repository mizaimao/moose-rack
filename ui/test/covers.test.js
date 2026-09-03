// Covers are fetched near the viewport and let go well away from it.
//
// The letting go is the whole of the memory problem: a cover is a few tens of
// kilobytes as a PNG and about 786 KB once decoded, and the version this
// replaces unobserved a card the moment its cover arrived — so every image the
// list had ever drawn stayed decoded for as long as the list was on screen.
// Measured 2026-08-20 at 578 MB of a ~671 MB total.
//
// Invisible from the outside: a list that holds every cover looks exactly like
// one that does not. It only shows up in `footprint`, which no test can read.
// So the observers are driven by hand here.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, lib, state, observers;

/// A stand-in that hands back the entries it was given, so a test can say
/// "these cards just came into view" or "these just left".
class Watcher {
  constructor(fn, opts) {
    this.fn = fn;
    this.margin = opts?.rootMargin ?? "0px";
    this.watched = new Set();
    observers.push(this);
  }
  observe(node) { this.watched.add(node); }
  unobserve(node) { this.watched.delete(node); }
  disconnect() { this.watched.clear(); }
  /// Report every card it is watching as entering, or leaving, the margin.
  fire(isIntersecting) {
    this.fn([...this.watched].map((target) => ({ target, isIntersecting })));
  }
}

/// The two observers, told apart by the distance each one works at.
///
/// By which is nearer rather than by a fixed number: the margins are a
/// fraction of the list's own height now, so the pixel values move with the
/// window and a test that names one is a test that breaks on a resize.
const px = (o) => Number(String(o.margin).replace("px", ""));
const both = () => {
  const last = observers.slice(-2);
  return last.sort((a, b) => px(a) - px(b));
};
const near = () => both()[0];
const far = () => both()[1];

const ROWS = Array.from({ length: 6 }, (_, i) => ({
  id: i + 1,
  name: `Game ${i} & Sons`,
  fs_name: `g${i}.zip`,
  platform: "arcade",
  size_bytes: 1,
  downloaded: true,
  favorite: i === 0,
  rating: null, year: null, last_played: null, players: null,
}));

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/", pretendToBeVisual: true,
  });
  for (const k of ["window", "document", "localStorage", "CSS"])
    Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true });
  Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
  globalThis.requestAnimationFrame = (f) => dom.window.setTimeout(f, 0);
  dom.window.Element.prototype.scrollIntoView = function () {};

  observers = [];
  dom.window.IntersectionObserver = Watcher;
  globalThis.IntersectionObserver = Watcher;

  const backend = fakeBackend((cmd, args) => {
    if (cmd === "rom_covers") return args.ids.map((id) => ({ id, cover: `/art/${id}.png` }));
    if (cmd === "rom_detail")
      return { id: 1, name: "x", fs_name: "g.zip", platform: "arcade", platform_slug: "arcade",
               size_bytes: 1, downloaded: true, screenshots: [], genres: [], companies: [],
               franchises: [], game_modes: [], regions: [], alt_names: [], art: {} };
    return [];
  });
  dom.window.__TAURI__ = {
    core: { invoke: backend, convertFileSrc: (p) => p },
    event: { listen: async () => () => {}, emit: () => {} },
  };
  backend.rows(ROWS);

  lib = await import("../js/library.js");
  ({ state } = await import("../js/state.js"));
  await (await import("../js/arrange.js")).arrangeCurrentList();
});

beforeEach(() => {
  observers.length = 0;
  state.view = "roms";
  state.platform = "arcade";
  state.layout = "grid";
  state.rows = ROWS;
  lib.renderRows(ROWS, false);
});

/// A turn of the loop: covers are fetched in a batch, 80ms behind the scroll.
/// Let the queued work run, then tell every picture on the page that it has
/// arrived.
///
/// jsdom does not fetch images, so an `<img>` there never fires `load` or
/// `error` and would sit in the in-flight count for ever. A browser always
/// answers one way or the other; this makes the stand-in do the same, which is
/// what the throttle in `pumpCovers` is written against.
const settle = async () => {
  await new Promise((r) => dom.window.setTimeout(r, 120));
  for (const img of document.querySelectorAll("#list img"))
    img.dispatchEvent(new dom.window.Event("load"));
  await new Promise((r) => dom.window.setTimeout(r, 20));
};

const art = (id) => document.querySelector(`.gcard[data-id="${id}"] .art`);
const hasImage = (id) => !!art(id)?.querySelector("img");

describe("covers near the viewport", () => {
  test("a card that comes into view gets its cover", async () => {
    assert.equal(hasImage(1), false, "drawn with an image before anything scrolled");
    near().fire(true);
    await settle();
    assert.equal(hasImage(1), true, "no cover arrived");
    assert.equal(art(1).querySelector("img").getAttribute("src"), "/art/1.png");
  });

  /// Replacing the whole of `.art` with the image took the star with it, so a
  /// starred game lost its star the moment its cover arrived.
  test("a starred game keeps its star once the cover lands", async () => {
    near().fire(true);
    await settle();
    assert.ok(art(1).querySelector(".star"), "the star went with the placeholder");
    assert.equal(art(2).querySelector(".star"), null, "an unstarred game grew one");
  });
});

describe("covers well away from it", () => {
  /// The fix for 578 MB. Nothing else in the app releases an image.
  test("a card that leaves the far margin gives its cover back", async () => {
    near().fire(true);
    await settle();
    assert.equal(hasImage(1), true);

    far().fire(false);
    assert.equal(hasImage(1), false, "the decoded image is still being held");
    assert.ok(art(1).querySelector(".ph"), "nothing was put back in its place");
  });

  /// The bug this is here for: `drawCover` puts a `<canvas>` on the card, not
  /// an `<img>`, and the release path went on testing for `IMG` after that
  /// changed. So nothing was ever released in the real app — only in this
  /// suite, where jsdom has no `createImageBitmap`, `fitted` bails, and every
  /// cover is the fallback `<img>`. The canvas has to be stood up by hand
  /// because the path that makes one cannot run here.
  test("a canvas cover is released too, not just an img", async () => {
    const card = document.querySelector('.gcard[data-id="1"]');
    card.dataset.loaded = "1";
    art(1).replaceChildren(document.createElement("canvas"));
    assert.ok(art(1).querySelector("canvas"), "the card did not start out holding a canvas");

    far().fire(false);
    assert.equal(art(1).querySelector("canvas"), null, "the canvas is still being held");
    assert.ok(art(1).querySelector(".ph"), "nothing was put back in its place");
    assert.equal(card.dataset.loaded, undefined, "the card cannot ask for its cover again");
  });

  test("what goes back is the placeholder it was drawn with", async () => {
    near().fire(true);
    await settle();
    far().fire(false);
    // Two letters, and the ampersand escaped exactly once.
    assert.equal(art(1).querySelector(".ph").textContent, "Ga");
    assert.ok(art(1).querySelector(".star"), "a starred game lost its star on the way back");
  });

  /// A card whose cover was released has to be able to ask for it again — the
  /// version this replaces unobserved on load, so it never could.
  test("scrolling back fetches it again", async () => {
    near().fire(true);
    await settle();
    far().fire(false);
    assert.equal(hasImage(1), false);

    near().fire(true);
    await settle();
    assert.equal(hasImage(1), true, "the cover never came back");
  });

  /// The two margins are different distances on purpose: a card one flick of
  /// the wheel off the top of the screen is about to be looked at again, and
  /// dropping its cover there would mean decoding it twice for nothing.
  test("the release margin is further out than the load margin", () => {
    assert.ok(px(far()) > px(near()) * 2, `${far().margin} is not far enough past ${near().margin}`);
  });

  /// The bug this replaced: a flat 1,600px release margin was two screens on
  /// a desktop window and eight on a 720-tall handheld — so the machine with
  /// the least memory held the most pictures. A decoded cover is four bytes a
  /// pixel whatever size it is drawn at, so the count of cards inside this
  /// margin *is* the memory. See `docs/memory-footprint.md`.
  test("the margins are a fraction of the screen, not a fixed distance", () => {
    const list = dom.window.document.getElementById("list");
    const height = (n) =>
      Object.defineProperty(list, "clientHeight", { value: n, configurable: true });
    // A desktop window, then a handheld's screen.
    height(900);
    lib.observeCovers();
    const tall = px(far());
    height(300);
    lib.observeCovers();
    assert.ok(
      px(far()) < tall,
      `a shorter list kept the same ${far().margin} margin, so it holds the same pile of pictures`
    );
  });

  /// This runs for every card leaving the margin, on a list of 2,506.
  test("a card with nothing to release is left alone", async () => {
    const before = art(3).innerHTML;
    far().fire(false);
    assert.equal(art(3).innerHTML, before, "the placeholder was rewritten over itself");
  });
});


/// A burst of `asset://` requests wedges the page: measured 2026-08-22, sixty
/// at once neither loaded nor failed — no `onload`, no `onerror`, nothing in
/// two minutes, the process perfectly still. The same sixty one after another
/// all succeeded. `flushCovers` was setting forty `src`s in a single pass.
///
/// So the number in the air is the thing to hold down, and it is the only part
/// of this a test can see.
describe("covers are not all asked for at once", () => {
  test("no more than six are in the air", async () => {
    // Far more cards than slots, all coming into view together.
    near().fire(true);
    await new Promise((r) => dom.window.setTimeout(r, 140));
    assert.ok(
      lib.coversInFlight() <= 6,
      `${lib.coversInFlight()} pictures in the air at once — a burst that size wedges`
    );
  });

  /// A slot must come back when the picture arrives, or the grid stops after
  /// the first six and never draws another — which is a worse bug than the one
  /// the throttle is for.
  test("answering the first six lets the rest through", async () => {
    near().fire(true);
    await new Promise((r) => dom.window.setTimeout(r, 140));
    const first = document.querySelectorAll("#list img").length;
    // Answer, repeatedly, the way a browser would as each arrives.
    for (let round = 0; round < 4; round++) {
      for (const img of document.querySelectorAll("#list img"))
        img.dispatchEvent(new dom.window.Event("load"));
      await new Promise((r) => dom.window.setTimeout(r, 20));
    }
    const after = document.querySelectorAll("#list img").length;
    assert.ok(
      after >= first,
      `the grid went backwards, ${first} pictures to ${after}`
    );
    assert.ok(lib.coversInFlight() <= 6, "the count ran away");
  });
});

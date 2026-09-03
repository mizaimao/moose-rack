// The controller path, exercised against the real index.html.
//
// This exists because "A and B do nothing" was reported five times and every
// diagnosis was made by reading the code, which was wrong every time. Reading
// cannot see an exception thrown inside a requestAnimationFrame callback: the
// next frame is already scheduled, so the loop keeps running and the failure is
// silent. These tests run the actual modules and assert on what happens.
//
//   npm test
//
// One page and one module graph for the whole file, because that is how the app
// runs: state.js caches element handles at import time, so the DOM has to exist
// first and cannot be swapped afterwards. Tests reset the list and the view
// instead of rebuilding the world.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

/// Selection class used across the UI. `sel`, not `selected` — worth naming
/// once here so a test never disagrees with library.js by a typo.
const SEL = "sel";

let dom, invoked, pads, ui, backend;

/// Minimal stand-ins for the backend. Shapes matter: the render path reads
/// `.length` off list responses and `.downloaded` off a detail, so returning a
/// bare null here surfaces as an unhandled rejection a frame later rather than
/// as a failed assertion.
function reply(cmd, args) {
  // Enough of a detail for the info pane to render: it joins several of these
  // arrays, so a thinner stub throws inside the pane rather than failing an
  // assertion here.
  if (cmd === "rom_detail")
    return {
      id: args.id,
      name: `Game ${args.id}`,
      fs_name: `game${args.id}.rom`,
      platform: "Super Nintendo",
      platform_slug: "snes",
      size_bytes: 1024,
      downloaded: false,
      files: [],
      screenshots: [],
      genres: [],
      companies: [],
      franchises: [],
      game_modes: [],
      regions: [],
      alt_names: [],
      art: {},
    };
  if (cmd === "status") return { configured: true };
  if (cmd === "recent_games") return recentGames;
  if (cmd === "download_estimate") return [estimateSummary, estimateFits, "note"];
  if (cmd === "game_states") return stateList;
  if (cmd === "collections_in")
    return [
      { id: "c1", name: "Best of snes", rom_count: 50, local_count: 20, sample_ids: [] },
      { id: "c2", name: "Arcade Fighting", rom_count: 322, local_count: 322, sample_ids: [] },
    ];
  if (cmd === "platforms")
    return [
      { slug: "snes", name: "Super Nintendo", rom_count: 12 },
      { slug: "psx", name: "PlayStation", rom_count: 40 },
      { slug: "megadrive", name: "Mega Drive", rom_count: 9 },
    ];
  return [];
}

/// What `game_states` should answer with, per test.
let stateList = [{ slot: "1", label: "Slot 1", when: "yesterday", resumable: true }];

/// What `recent_games` should answer with, per test.
let recentGames = [];
let estimateSummary = "10 game(s), about 1.0 GB";
let estimateFits = true;

/// Let the promise chain a UI action kicked off run to completion, so its
/// failures land inside the test that caused them.
const settle = () => new Promise((r) => dom.window.setTimeout(r, 0));

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });

  invoked = [];
  // The interface commands — bindings, ordering, the grid — are answered by
  // the stand-in in backend.js; everything else falls through to `reply`.
  backend = fakeBackend(reply);
  dom.window.__TAURI__ = {
    core: {
      invoke: (cmd, args) => {
        invoked.push({ cmd, args });
        return backend(cmd, args);
      },
      convertFileSrc: (p) => p,
    },
    event: { listen: () => Promise.resolve(() => {}), emit: () => {} },
  };

  pads = [];
  dom.window.navigator.getGamepads = () => pads;

  // defineProperty, not assignment: `navigator` is a getter-only global in
  // Node. `performance` is deliberately left alone — shadowing it makes
  // jsdom's own now() recurse until the stack blows.
  for (const k of ["window", "document", "navigator", "localStorage", "CSS"]) {
    Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true });
  }
  globalThis.requestAnimationFrame = (fn) => dom.window.setTimeout(fn, 0);
  // jsdom has no layout, so it does not implement this at all. The app calls it
  // whenever it moves the cursor; without a stand-in every such path throws.
  dom.window.Element.prototype.scrollIntoView = function () {};
  // jsdom has no IntersectionObserver, and the cover loader builds one on every
  // render. A stand-in that observes nothing is right for these tests: they are
  // about what is drawn, not about lazy loading.
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  dom.window.IntersectionObserver = FakeObserver;
  globalThis.IntersectionObserver = FakeObserver;

  const load = (m) => import(join(uiDir, "js", m));
  ui = {
    ...(await load("state.js")),
    ...(await load("keys.js")),
    ...(await load("bindings.js")),
    ...(await load("gamepad.js")),
    ...(await load("tabs.js")),
    ...(await load("library.js")),
    ...(await load("bulk.js")),
    ...(await load("lightbox.js")),
    ...(await load("shell.js")),
    ...(await load("sort.js")),
    ...(await load("filter.js")),
    ...(await load("arrange.js")),
  };
  // The tables come from the backend now, so nothing that reads a binding or
  // draws a sort menu works until they have arrived. The app does this in
  // main.js before it installs the keyboard or the pad.
  await ui.loadBindings();
  ui.setFilters((await ui.loadListControls()).filters);
});

beforeEach(async () => {
  invoked.length = 0;
  pads = [];
  recentGames = [];
  estimateFits = true;
  document.getElementById("bulk-overlay")?.remove();
  // A test that fails before its cleanup leaves its dialog in the document,
  // and the next test would then drive that one instead of its own.
  document.getElementById("conflict-overlay")?.remove();
  document.querySelector(".ctx-menu")?.remove();
  ui.closeLightbox();
  document.getElementById("list").innerHTML = "";
  ui.state.view = "platforms";
  ui.state.platform = null;
  ui.trail.length = 0;
  // One pane unless a test says otherwise: nearly everything in this file is
  // about the arrangement where a screen is replaced and Back brings it back.
  // Three columns has its own file.
  ui.setMode("single");
  ui.resetPad();
  // Flush anything the previous test left held. The loop only forgets a button
  // on a poll where it is not pressed, and a test does not poll on the way out.
  pads = [];
  ui.stepForTest();
  // The section is module state and survives between tests. A test that ends
  // in My collections would otherwise change what "back" means for the next
  // one, which is the kind of coupling that makes a suite lie.
  await ui.showSection("library");
  invoked.length = 0;
});

/// A gamepad with `down` pressed, shaped like the real thing.
/// A gamepad with `down` pressed, shaped like the real thing.
///
/// `analogue` gives a button a partial value, which is what a trigger reports:
/// `{ 7: 0.4 }` is the right trigger pulled a little under half way. A trigger
/// that far in is not "pressed" yet on most pads, so it is separate from
/// `down` rather than derived from it.
function pad(down = [], { mapping = "standard", axes = [0, 0], analogue = {} } = {}) {
  return {
    id: "Xbox Wireless Controller (STANDARD GAMEPAD Vendor: 045e Product: 0b13)",
    mapping,
    connected: true,
    axes,
    buttons: Array.from({ length: 17 }, (_, i) => ({
      pressed: down.includes(i),
      value: analogue[i] ?? (down.includes(i) ? 1 : 0),
    })),
  };
}

function cards(html) {
  document.getElementById("list").innerHTML = html;
  return [...document.querySelectorAll(".card")];
}

/// The slug of the console the UI last asked the backend for.
const openedPlatform = () =>
  [...invoked].reverse().find((c) => c.cmd === "roms")?.args.platform ?? null;

describe("pad bindings", () => {
  test("the default map sends the face buttons to open and back", () => {
    const map = ui.padMap();
    assert.equal(map[0], "activate", "bottom face button opens");
    assert.equal(map[1], "back", "right face button goes back");
    // Select cycles the pictures. It used to open settings — a second window
    // of text fields and tables that a pad cannot navigate, so the button
    // opened something you could then only leave again.
    assert.equal(map[8], "pictures", "Select should change the pictures");
  });

  test("every default binding names a handler that exists", () => {
    // The failure this guards against is invisible at runtime: runAction looks
    // the id up, finds nothing, and returns without a sound. A binding pointing
    // at a renamed handler is exactly "the button does nothing".
    for (const [index, action] of Object.entries(ui.padMap())) {
      assert.ok(
        Object.hasOwn(ui.HANDLERS, action),
        `button ${index} is bound to "${action}", which is not in HANDLERS`
      );
    }
  });

  /// Rebinding, the clearing of whatever held the button before, and the
  /// repair that keeps a direction from being left on nothing are asserted in
  /// `binds::tests` — against the implementation rather than through a page.
  /// What is left here is that the page can read the result.
  test("a rebind moves the action, and the page sees it", async () => {
    await ui.setPad("activate", 3);
    assert.equal(ui.padMap()[3], "activate");
    assert.equal(ui.padFor("activate"), 3);
    await ui.resetPad();
    assert.equal(ui.padMap()[0], "activate", "reset restores the defaults");
  });
});

describe("activate — the A button", () => {
  test("opens the focused console", async () => {
    cards(`<div class="card" data-slug="snes"></div><div class="card" data-slug="genesis"></div>`)[1]
      .classList.add(SEL);
    ui.runAction("activate");
    await settle();
    assert.equal(openedPlatform(), "genesis");
  });

  test("opens the first console when nothing is focused yet", async () => {
    cards(`<div class="card" data-slug="snes"></div>`);
    ui.runAction("activate");
    await settle();
    assert.equal(openedPlatform(), "snes", "a fresh grid must still respond to A");
  });

  test("opens the focused game", async () => {
    cards(`<div class="card" data-id="42"></div>`);
    ui.runAction("activate");
    await settle();
    assert.deepEqual(
      invoked[0],
      { cmd: "rom_detail", args: { id: 42 } },
      "A on a game asks the backend for its detail first"
    );
    // Not yet on disk, so it downloads first; a downloaded game launches
    // straight away. Either is "A did something", which is what is being
    // asserted -- the bug this guards against is A doing nothing at all.
    assert.ok(
      invoked.slice(1).some((c) => c.cmd === "download_rom" || c.cmd === "launch_rom"),
      `and then acts on it; got ${invoked.map((c) => c.cmd).join(" -> ")}`
    );
  });

  test("clicks a collection card, reusing the handler that knows its group", () => {
    let clicked = 0;
    cards(`<div class="card" data-cid="7"></div>`)[0].addEventListener("click", () => clicked++);
    ui.runAction("activate");
    assert.equal(clicked, 1);
  });

  test("does nothing, quietly, on an empty grid", () => {
    assert.doesNotThrow(() => ui.runAction("activate"));
    assert.equal(invoked.length, 0);
  });
});

describe("back — the B button", () => {
  test("is a no-op on the top-level grid", () => {
    ui.state.view = "platforms";
    assert.doesNotThrow(() => ui.runAction("back"));
    assert.equal(ui.state.view, "platforms");
  });

  test("returns to the grid from a console", async () => {
    cards(`<div class="card" data-slug="snes"></div>`);
    ui.runAction("activate");
    await settle();
    assert.equal(ui.state.view, "roms");
    ui.runAction("back");
    await settle();
    assert.equal(ui.state.view, "platforms", "B must climb out of a console");
  });

  test("unwinds one level at a time inside collections", () => {
    ui.state.view = "collection-roms";
    let popped = 0;
    ui.trail.push(() => popped++);
    ui.runAction("back");
    assert.equal(popped, 1, "the trail is walked, not skipped past");
    assert.equal(ui.trail.length, 0);
  });
});

describe("the poll loop", () => {
  // These call the poll loop's own translation of pad state to actions,
  // rather than re-deriving it, so the test fails if that logic drifts.
  const pressedActions = (...a) => ui.pressedActions(...a);

  test("a pressed A resolves to activate", () => {
    assert.deepEqual([...pressedActions([pad([0])], ui.padMap())], ["activate"]);
  });

  test("a pressed B resolves to back", () => {
    assert.deepEqual([...pressedActions([pad([1])], ui.padMap())], ["back"]);
  });

  test("an unbound button resolves to nothing", () => {
    assert.deepEqual([...pressedActions([pad([16])], ui.padMap())], []);
  });

  test("a button cleared by a rebind is skipped rather than dispatched as null", () => {
    const map = { ...ui.padMap(), 0: null };
    assert.deepEqual([...pressedActions([pad([0])], map)], []);
  });

  test("the stick moves along its dominant axis only", () => {
    // Pushed diagonally, reporting both would move the cursor twice in one
    // frame, which reads as it jumping around on its own.
    assert.deepEqual([...pressedActions([pad([], { axes: [-0.9, -0.7] })], ui.padMap())], ["left"]);
    assert.deepEqual([...pressedActions([pad([], { axes: [-0.7, -0.9] })], ui.padMap())], ["up"]);
  });

  test("a resting stick reports nothing", () => {
    assert.deepEqual([...pressedActions([pad([], { axes: [0.2, -0.3] })], ui.padMap())], []);
  });

  test("a disconnected slot is skipped", () => {
    assert.doesNotThrow(() => pressedActions([null, pad([0])], ui.padMap()));
    assert.deepEqual([...pressedActions([null, pad([0])], ui.padMap())], ["activate"]);
  });

  test("a pad reporting no standard mapping still works by index", () => {
    // WebKit does not always fill in `mapping`. The buttons are still in
    // standard order, so nothing should key off that field.
    assert.deepEqual([...pressedActions([pad([0], { mapping: "" })], ui.padMap())], ["activate"]);
  });
});

describe("the triggers scroll the list", () => {
  test("LT and RT are the list's scroll, not zoom or paging", () => {
    const map = ui.padMap();
    // The only two analogue controls on the pad besides the sticks, on a
    // screen whose main job is moving through two thousand games. Zoom is a
    // thing you set once and leave.
    assert.equal(map[6], "scrollUp", "LT scrolls up");
    assert.equal(map[7], "scrollDown", "RT scrolls down");
    // The stick clicks sort the list now. They were paging, which the d-pad
    // and the sticks already do a screen at a time, so pressing them looked
    // like nothing happening.
    assert.equal(map[10], "sortCycle");
    // The right stick click picks a game at random. It used to open the sort
    // menu, which a pad cannot navigate — a button that opens something you
    // can then only close again — and was left doing nothing at all. Random
    // is the one thing on this screen worth a button and needing no menu.
    assert.equal(map[11], "random", "the right stick click does nothing again");
    // The menu is still reachable by mouse and by key.
    assert.equal(ui.keyFor("sortMenu"), "s");
    // Paging is not gone, it is on the keyboard, where it has a key that says
    // what it does.
    assert.equal(ui.keyFor("pageUp"), "PageUp");
    assert.equal(ui.keyFor("pageDown"), "PageDown");
  });

  test("zoom stops at the slider's own limits", () => {
    const zoom = document.getElementById("zoom");
    document.getElementById("zoom-wrap").hidden = false;
    const max = Number(zoom.max);
    const min = Number(zoom.min);

    ui.state.zoom = max;
    ui.runAction("zoomIn");
    assert.equal(ui.state.zoom, max, "cannot grow past the top of the range");

    ui.state.zoom = min;
    ui.runAction("zoomOut");
    assert.equal(ui.state.zoom, min, "cannot shrink past the bottom");
  });

  test("nothing happens where there are no covers to resize", () => {
    document.getElementById("zoom-wrap").hidden = true;
    ui.state.zoom = 150;
    ui.runAction("zoomIn");
    assert.equal(ui.state.zoom, 150);
  });
});

describe("the lock after the emulator exits", () => {
  /// The quit combo is Select + A, and both are bound in here too. Coming back
  /// from RetroArch the pad can report nothing held for a frame or two before
  /// its real state arrives, so "wait for release" alone lifted the lock while
  /// the combo was still physically down — and the game relaunched itself.
  test("does not lift on the empty frame right after the emulator exits", () => {
    ui.ignorePadUntilReleased();
    const now = performance.now();
    assert.equal(
      ui.settleLifted(new Set(), now),
      false,
      "an empty frame immediately after the exit is the pad catching up, not a release"
    );
  });

  test("lifts once the pad is at rest and the floor has passed", () => {
    ui.ignorePadUntilReleased();
    const later = performance.now() + 500;
    assert.equal(ui.settleLifted(new Set(), later), true);
  });

  test("stays locked past the floor while a button is still held", () => {
    ui.ignorePadUntilReleased();
    const later = performance.now() + 500;
    assert.equal(
      ui.settleLifted(new Set(["activate"]), later),
      false,
      "the floor is a minimum, not a replacement for waiting"
    );
  });
});

describe("back at the top of a section", () => {
  /// Back did nothing at all from inside a console: with the crumb trail empty
  /// it asked for the section it was already in, and switching to the section
  /// you are already on is correctly a no-op. It has to reopen instead.
  test("reopens the section rather than asking to switch to it", async () => {
    cards(`<div class="card" data-slug="snes"></div>`);
    ui.runAction("activate");
    await settle();
    assert.equal(ui.state.view, "roms");

    await ui.resetSection();
    await settle();
    assert.equal(ui.state.view, "platforms", "Back must climb out of a console");
  });

  /// Back has to land on the front of the section you are in, not always on
  /// the platform grid. The tab bar says where you are and it has to stay true.
  test("lands on the front of the section you are actually in", async () => {
    await ui.showSection("mine");
    await settle();
    ui.state.view = "collection-roms";

    await ui.resetSection();
    await settle();
    assert.equal(ui.activeSection(), "mine", "Back must not tip you into the library");
    assert.notEqual(ui.state.view, "platforms");
  });
});

describe("grid / list on the consoles screen", () => {
  /// Pressing it there used to redraw `state.rows`, which still held the last
  /// console you opened — so the button appeared to open a console by itself.
  test("switching layout on the consoles screen does not open a console", async () => {
    cards(`<div class="card" data-slug="snes"></div>`);
    ui.runAction("activate");
    await settle();
    assert.equal(ui.state.view, "roms", "we opened one, so state.rows is populated");

    await ui.resetSection();
    await settle();
    assert.equal(ui.state.view, "platforms");

    const before = invoked.length;
    ui.setLayout("list");
    await settle();
    assert.equal(ui.state.view, "platforms", "the layout button must not open anything");
    assert.equal(
      invoked.slice(before).filter((c) => c.cmd === "roms").length,
      0,
      "no console was asked for"
    );
  });

  test("consoles render in both layouts, and stay reachable by keyboard", async () => {
    // The grid/list button is about the one-pane arrangement: in three columns
    // the console list is a 240px column and always a list, whatever the
    // button says.
    ui.setMode("single");
    ui.state.platforms = [
      { slug: "snes", name: "Super Nintendo", rom_count: 50, playable: true },
      { slug: "gb", name: "Game Boy", rom_count: 32, playable: false },
    ];
    ui.state.view = "platforms";

    // Counted inside the region the consoles were drawn into, not across the
    // document: the other arrangement's copy is still in the page and would be
    // counted too.
    const inPane = (sel) => document.getElementById("list").querySelectorAll(sel).length;
    const inColumn = (sel) => document.getElementById("consoles").querySelectorAll(sel).length;

    ui.setLayout("list");
    assert.equal(inPane(".prow"), 2);
    // The navigation code selects `.card, .gcard, .row, .tcard`; a console row
    // that is not one of those cannot be reached without a mouse.
    assert.equal(inPane(".row"), 2, "rows must be navigable");

    ui.setLayout("grid");
    assert.equal(inPane(".card"), 2);
    assert.equal(inPane(".prow"), 0);

    // And in a column it stays a list, because a grid two cards across in
    // 240px is neither a grid nor readable.
    ui.setMode("columns");
    ui.setLayout("grid");
    assert.equal(inColumn(".prow"), 2, "a grid in the column");
    assert.equal(inColumn(".card"), 0);
    ui.setMode("single");
  });

  test("opening a console from a row works, not just from a card", async () => {
    ui.setMode("single");
    ui.state.platforms = [{ slug: "gb", name: "Game Boy", rom_count: 32, playable: true }];
    ui.state.view = "platforms";
    ui.setLayout("list");

    // Inside the pane the consoles were drawn into: the left column still
    // holds the previous test's copy, and it comes first in the document.
    document.getElementById("list").querySelector(".prow").click();
    await settle();
    assert.equal(openedPlatform(), "gb");
  });
});

describe("the pad while a game is running", () => {
  // The obvious test — suspend, press the quit combo, assert nothing opened —
  // could not be made to fail with the guard removed. `step` returns early on
  // several checks before it reaches the dispatcher, and every attempt ended up
  // asserting on a poll that never ran. A test that passes either way is worse
  // than none, so this is left to the machine it actually affects.
  // Same story for "resuming re-arms the lock": it passed with the re-arm
  // removed, because settleLifted reads module state the previous test had
  // already left in the right shape. Removed rather than kept as decoration.
});

describe("continue playing", () => {
  test("the row is absent entirely when nothing has been played", async () => {
    recentGames = [];
    await ui.showPlatforms();
    await settle();
    assert.equal(document.querySelector(".recent"), null,
      "an empty row explaining itself is worse than no row");
  });

  test("recent games appear above the consoles, in the order given", async () => {
    recentGames = [
      { id: 7, name: "Chrono Trigger", platform: "snes", downloaded: true },
      { id: 8, name: "Gunstar Heroes", platform: "megadrive", downloaded: false },
    ];
    await ui.showPlatforms();
    await settle();
    const strip = document.querySelector(".recent");
    assert.ok(strip, "the row should be drawn");
    const names = [...strip.querySelectorAll(".gname")].map((n) => n.textContent);
    assert.deepEqual(names, ["Chrono Trigger", "Gunstar Heroes"]);
    // Above the consoles, not after them: it is the first thing you look at.
    assert.equal(document.getElementById("list").firstElementChild, strip);
  });
});

describe("take offline", () => {
  /// The size is asked for, not assumed. Counting what is already on disk is
  /// one filesystem call per game, and on a 2,500-game collection doing that on
  /// every checkbox froze the window for seconds.
  test("opening the dialog does not count anything", async () => {
    ui.askDownload({ platform: "snes" });
    await settle();
    assert.equal(
      invoked.filter((c) => c.cmd === "download_estimate").length,
      0,
      "nothing should be counted until asked"
    );
  });

  test("checking the size asks once, with the cheap defaults", async () => {
    ui.askDownload({ platform: "snes" });
    await settle();
    document.querySelector(".bulk-size").click();
    await settle();
    const asked = invoked.filter((c) => c.cmd === "download_estimate");
    assert.equal(asked.length, 1);
    assert.deepEqual(asked[0].args.choice.platforms, ["snes"], "the console on screen starts ticked");
    assert.equal(asked[0].args.choice.art, "minimal");
    assert.equal(asked[0].args.choice.videos, false, "videos must never be on by default");
    assert.equal(asked[0].args.choice.bios, false, "300 MB must never arrive unasked");
  });

  /// Nobody travels with one console, and choosing them one at a time meant
  /// running the whole dialog once per system — each size check against a disk
  /// the previous run had already eaten into, so the last always claimed to
  /// fit and did not.
  test("several systems go in one download", async () => {
    ui.askDownload({ platform: "snes" });
    await settle();
    const psx = [...document.querySelectorAll(".bulk-plat")].find((c) => c.value === "psx");
    psx.checked = true;
    document.querySelector(".bulk-size").click();
    await settle();
    const asked = invoked.filter((c) => c.cmd === "download_estimate").at(-1);
    assert.deepEqual(asked.args.choice.platforms.sort(), ["psx", "snes"]);
  });

  test("All and None move every tick at once", async () => {
    ui.askDownload({ platform: "snes" });
    await settle();
    document.querySelector(".bulk-all").click();
    assert.equal([...document.querySelectorAll(".bulk-plat:checked")].length, 3);
    document.querySelector(".bulk-none").click();
    assert.equal([...document.querySelectorAll(".bulk-plat:checked")].length, 0);
  });

  /// Somewhere with no server is exactly where a missing BIOS becomes a console
  /// that will not boot and cannot be fixed. It belongs in the pane people open
  /// before going there, not three tabs deep in settings.
  test("BIOS can be taken along with the games", async () => {
    ui.askDownload({ platform: "psx" });
    await settle();
    const bios = document.querySelector(".bulk-bios");
    assert.ok(bios, "no way to take BIOS files from the offline pane");
    bios.checked = true;
    document.querySelector(".bulk-size").click();
    await settle();
    assert.equal(
      invoked.filter((c) => c.cmd === "download_estimate").at(-1).args.choice.bios,
      true
    );
  });

  /// With nothing ticked the backend cannot tell "everything" from "nothing",
  /// so the dialog refuses rather than guessing.
  test("downloading nothing is refused before it is sent", async () => {
    ui.askDownload({});
    await settle();
    document.querySelector(".bulk-none").click();
    document.querySelector(".bulk-go").click();
    await settle();
    assert.equal(
      invoked.filter((c) => c.cmd === "download_set").length,
      0,
      "an empty download reached the backend"
    );
  });

  /// A figure computed before a checkbox moved is a wrong figure. It is cleared
  /// rather than silently left on screen next to different options.
  test("changing an option clears the figure instead of recomputing", async () => {
    ui.askDownload({ platform: "snes" });
    await settle();
    document.querySelector(".bulk-size").click();
    await settle();
    const before = invoked.filter((c) => c.cmd === "download_estimate").length;

    const box = document.querySelector(".bulk-videos");
    box.checked = true;
    // Bubbling, as a real change event does: the dialog listens once on the
    // panel rather than on each control.
    box.dispatchEvent(new window.Event("change", { bubbles: true }));
    await settle();

    assert.equal(
      invoked.filter((c) => c.cmd === "download_estimate").length,
      before,
      "a checkbox must not trigger the expensive count"
    );
    assert.ok(
      !document.querySelector(".bulk-est").textContent.includes("GB"),
      "the stale figure must not sit next to the new options"
    );
  });

  /// The whole point of asking the disk: refusing here costs nothing, refusing
  /// an hour in costs a half-written game and a full disk.
  test("a download that will not fit cannot be started", async () => {
    estimateFits = false;
    ui.askDownload({ platform: "psx" });
    await settle();
    document.querySelector(".bulk-size").click();
    await settle();
    assert.equal(document.querySelector(".bulk-go").disabled, true);
    assert.ok(document.querySelector(".bulk-space").classList.contains("bad"));
  });
});

describe("the lightbox and the controller", () => {
  /// The player used to swallow the pad completely. So the button that started
  /// a video could not stop it, and the only way out of a full-screen video was
  /// to find the mouse — on a machine being used from a sofa with a controller.
  test("the button that opened a video closes it again", () => {
    const y = Number(
      Object.entries(ui.padMap()).find(([, a]) => a === "video")?.[0]
    );
    assert.ok(Number.isInteger(y), "nothing is bound to the video action");

    ui.openLightbox([{ src: "x.mp4", kind: "video", caption: "Gameplay" }], 0);
    assert.equal(ui.isLightboxOpen(), true);

    // One poll with nothing held, as happens between letting go of whatever
    // opened the player and reaching for the next button.
    pads = [pad([])];
    ui.stepForTest();

    pads = [pad([y])];
    ui.stepForTest();
    assert.equal(ui.isLightboxOpen(), false, "the pad cannot close the player");
  });

  test("back closes it too", () => {
    const b = Number(Object.entries(ui.padMap()).find(([, a]) => a === "back")?.[0]);
    ui.openLightbox([{ src: "x.png", kind: "image", caption: "Box" }], 0);
    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([b])];
    ui.stepForTest();
    assert.equal(ui.isLightboxOpen(), false);
  });

  /// Zoom goes to whatever is in front of you: the picture on the stage when
  /// the player is open, the covers in the grid when it is not. It used to
  /// resize the grid invisibly while a video played over the top.
  test("zoom goes to the video, not the grid behind it", () => {
    const before = ui.state.zoom;
    ui.openLightbox([{ src: "x.mp4", kind: "video", caption: "Gameplay" }], 0);
    ui.runAction("zoomIn");

    assert.equal(ui.isLightboxOpen(), true, "zooming must not close it");
    assert.equal(ui.state.zoom, before, "the grid behind was resized instead");
    const scale = Number(document.getElementById("lightbox").style.getPropertyValue("--lb-zoom"));
    assert.ok(scale > 1, `the stage did not zoom (--lb-zoom = ${scale})`);
    ui.closeLightbox();
  });

  /// Coming back to a picture at whatever scale the last one was left at is
  /// disorienting, and nothing on screen explains it.
  test("zoom resets each time it opens", () => {
    ui.openLightbox([{ src: "x.mp4", kind: "video", caption: "Gameplay" }], 0);
    ui.zoomLightbox(1);
    ui.zoomLightbox(1);
    ui.closeLightbox();
    ui.openLightbox([{ src: "y.png", kind: "image", caption: "Box" }], 0);
    assert.equal(document.getElementById("lightbox").style.getPropertyValue("--lb-zoom"), "1");
  });
});

describe("A pauses what is playing", () => {
  /// The bottom button, the way it works in every media player on a console.
  /// It used to do nothing in there at all — the pad could open a video and
  /// close it, and had no way to stop it halfway.
  test("the bottom button toggles the video on the stage", () => {
    const a = Number(Object.entries(ui.padMap()).find(([, act]) => act === "activate")?.[0]);
    ui.openLightbox([{ src: "x.mp4", kind: "video", caption: "Gameplay" }], 0);

    const video = document.querySelector("#lightbox video");
    assert.ok(video, "no video on the stage");
    const state = { paused: true };
    Object.defineProperty(video, "paused", { get: () => state.paused, configurable: true });
    video.play = () => ((state.paused = false), Promise.resolve());
    video.pause = () => (state.paused = true);

    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([a])];
    ui.stepForTest();
    assert.equal(state.paused, false, "A did not start it");

    // Held down it must not toggle sixty times a second: one press, one
    // change.
    ui.stepForTest();
    assert.equal(state.paused, false, "holding A flickered it");

    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([a])];
    ui.stepForTest();
    assert.equal(state.paused, true, "A did not stop it");
    ui.closeLightbox();
  });
});

describe("holding Select rather than tapping it", () => {
  /// Tapping it steps through seven kinds of artwork. The miximage is the one
  /// people come back to — a screenshot, the box and the logo in one picture —
  /// and six presses to get home from the wrong end of the list is how a good
  /// control becomes an annoying one.
  const select = () =>
    Number(Object.entries(ui.padMap()).find(([, a]) => a === "pictures")?.[0]);

  test("two seconds of it goes straight to the miximage", async () => {
    const sel = select();
    assert.ok(Number.isInteger(sel), "nothing is bound to the pictures action");
    // Out of the settle window first.
    //
    // Closing a video with the pad leaves the loop settling — ignoring
    // everything until the buttons have been up for a moment, so that letting
    // go of the button that closed it does not also register in the library
    // underneath. That window is real time, and a press arriving inside it is
    // deliberately ignored, so the test has to wait it out rather than poll
    // once and assume.
    pads = [pad([])];
    ui.stepForTest();
    // The floor under that window is 200ms.
    await new Promise((r) => setTimeout(r, 260));
    ui.stepForTest();
    invoked.length = 0;

    pads = [pad([sel])];
    ui.stepForTest();
    // Not yet: a tap is still a tap.
    assert.equal(
      invoked.find((c) => c.cmd === "set_list_art")?.args.value,
      undefined,
      "the hold fired on the way down"
    );

    // Two seconds of the same press. The loop reads the clock rather than
    // counting frames, so the wait is real.
    await new Promise((r) => setTimeout(r, 2050));
    ui.stepForTest();
    assert.equal(
      invoked.find((c) => c.cmd === "set_list_art")?.args.value,
      "miximages",
      "holding Select did not reach the miximage"
    );

    // Once for the hold, not once a frame while the button stays down.
    invoked.length = 0;
    ui.stepForTest();
    ui.stepForTest();
    assert.equal(
      invoked.filter((c) => c.cmd === "set_list_art").length,
      0,
      "it fired again while the button was still down"
    );
    pads = [pad([])];
    ui.stepForTest();
  });

  /// And a tap still cycles, or the hold has cost the button its real job.
  test("a tap still steps to the next one", () => {
    invoked.length = 0;
    pads = [pad([select()])];
    ui.stepForTest();
    pads = [pad([])];
    ui.stepForTest();
    assert.ok(
      invoked.some((c) => c.cmd === "icon_styles" || c.cmd === "list_art_options"),
      "tapping Select no longer cycles the pictures"
    );
  });
});

describe("four controllers", () => {
  /// With four pads plugged in for a four-player game, every one of them moved
  /// the cursor in the library — so three people fidgeting with sticks made the
  /// menu unusable while player one tried to pick a game. In the emulator all
  /// four are players; out here, player one is in charge.
  test("only the first controller drives the menus", () => {
    const map = ui.padMap();
    const right = Number(Object.entries(map).find(([, a]) => a === "right")?.[0]);
    const down = Number(Object.entries(map).find(([, a]) => a === "down")?.[0]);

    // Player two pressing right, player one pressing nothing.
    const acts = ui.pressedActions([pad([]), pad([right]), pad([down])], map);
    assert.equal(acts.size, 0, `player two moved the cursor: ${[...acts]}`);

    // And player one still works with the others connected.
    assert.ok(ui.pressedActions([pad([right]), pad([down])], map).has("right"));
  });

  /// A disconnected pad leaves a null hole in the list rather than shrinking
  /// it, so "the first one" is not always index zero.
  test("a gap where a controller was does not stop the next one", () => {
    const map = ui.padMap();
    const right = Number(Object.entries(map).find(([, a]) => a === "right")?.[0]);
    assert.ok(ui.pressedActions([null, pad([right])], map).has("right"));
  });

  /// The stick is read from the same pad as the buttons. Reading it from all
  /// of them put the second player's resting drift into player one's cursor.
  test("the stick is only read from the first controller", () => {
    const map = ui.padMap();
    const acts = ui.pressedActions([pad([]), pad([], { axes: [1, 0] })], map);
    assert.equal(acts.has("right"), false, "another pad's stick moved the cursor");
  });
});

describe("walking the reel with a pad", () => {
  test("the d-pad steps through the artwork while a video plays", () => {
    const map = ui.padMap();
    const right = Number(Object.entries(map).find(([, a]) => a === "right")?.[0]);
    ui.openLightbox(
      [
        { src: "a.png", kind: "image", caption: "Mix" },
        { src: "b.mp4", kind: "video", caption: "Gameplay" },
      ],
      1
    );
    const shown = () =>
      document.querySelector("#lightbox figcaption").textContent.split(" — ")[0];
    assert.equal(shown(), "Gameplay");
    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([right])];
    ui.stepForTest();
    assert.equal(shown(), "Mix", "the d-pad did not change the picture");
    ui.closeLightbox();
  });

  test("the left stick does the same", () => {
    ui.openLightbox(
      [
        { src: "a.png", kind: "image", caption: "Mix" },
        { src: "b.mp4", kind: "video", caption: "Gameplay" },
      ],
      1
    );
    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([], { axes: [1, 0] })];
    ui.stepForTest();
    assert.equal(
      document.querySelector("#lightbox figcaption").textContent.split(" — ")[0],
      "Mix",
      "the stick did not change the picture"
    );
    ui.closeLightbox();
  });
});

// Which order a list opens in, what each one does to it, that favorites stay
// on top, and that an order belongs to the console it was set on are asserted
// in `gamesort::tests` and `gamelist::tests` — against the implementation
// rather than through a page.

describe("opening the player with a held button", () => {
  /// Y opens the player and Y closes it, and a button is held down across
  /// several polls. So the poll that first sees the player open must not treat
  /// the button that opened it as a fresh press — or the player opens and shuts
  /// in the same breath and the button looks dead.
  test("the button that opened the player does not immediately close it", () => {
    const y = Number(Object.entries(ui.padMap()).find(([, a]) => a === "video")?.[0]);
    pads = [pad([y])];
    // The press that opens it. The real path goes through playVideo, which is
    // async; the poll below is what would happen while the button is still down.
    ui.openLightbox([{ src: "x.mp4", kind: "video", caption: "Gameplay" }], 0);
    ui.stepForTest();
    assert.equal(ui.isLightboxOpen(), true, "the player shut as soon as it opened");

    // Released, then pressed again: now it should close.
    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([y])];
    ui.stepForTest();
    assert.equal(ui.isLightboxOpen(), false, "a second press should close it");
  });
});

describe("the menu on a game", () => {
  /// It offered "take this console offline", which on a game in Continue
  /// playing reads as an offer to download a whole platform — the opposite of
  /// the small local thing a right-click on one game should do. What it offers
  /// now is that game's own save states.
  test("offers this game's save states, not a platform download", async () => {
    document.getElementById("list").innerHTML =
      `<div class="gcards"><div class="gcard" data-id="7"></div></div>`;
    const card = document.querySelector(".gcard");
    ui.wireGame(card, 7);

    card.dispatchEvent(
      new window.MouseEvent("contextmenu", { bubbles: true, cancelable: true })
    );
    await settle();
    await settle();

    const menu = document.querySelector(".ctx-menu");
    assert.ok(menu, "no menu opened");
    const text = menu.textContent;
    assert.doesNotMatch(text, /offline/i, "still offering to download the console");
    assert.match(text, /play/i, "no way to start the game");
    assert.match(text, /Delete Slot 1/, "the game's own states are not offered");
    document.querySelector(".ctx-menu")?.remove();
  });

  /// Most games have no states, and leaving the entry out entirely made the
  /// menu look like it had lost the feature rather than had nothing to offer.
  test("a game with no states says so instead of showing nothing", async () => {
    stateList = [];
    document.getElementById("list").innerHTML =
      `<div class="gcards"><div class="gcard" data-id="9"></div></div>`;
    const card = document.querySelector(".gcard");
    ui.wireGame(card, 9);
    card.dispatchEvent(
      new window.MouseEvent("contextmenu", { bubbles: true, cancelable: true })
    );
    await settle();
    await settle();
    const menu = document.querySelector(".ctx-menu");
    assert.match(menu.textContent, /No save states/);
    assert.equal(menu.querySelector("button.dim").disabled, true);
    stateList = [{ slot: "1", label: "Slot 1", when: "yesterday", resumable: true }];
    document.querySelector(".ctx-menu")?.remove();
  });
});

describe("dialogs answer the controller", () => {
  /// A launch can stop dead on "your saves could not be checked — play
  /// anyway?", and that question was mouse-only. On a machine being used from
  /// a sofa that is a dead end, not a dialog.
  const openDialog = () => {
    const d = document.createElement("div");
    d.id = "conflict-overlay";
    d.innerHTML = `<button class="a" data-name="a">Play anyway</button>
                   <button class="b" data-name="b">Keep the server's</button>
                   <button data-go="no" class="cancel" data-name="cancel">Cancel</button>`;
    document.body.appendChild(d);
    return d;
  };

  test("the d-pad moves through the buttons and A presses one", () => {
    const map = ui.padMap();
    const down = Number(Object.entries(map).find(([, a]) => a === "down")?.[0]);
    const go = Number(Object.entries(map).find(([, a]) => a === "activate")?.[0]);
    const d = openDialog();
    let clicked = null;
    for (const b of d.querySelectorAll("button")) {
      b.addEventListener("click", () => (clicked = b.dataset.name));
    }

    pads = [pad([down])];
    ui.stepForTest();
    assert.ok(
      document.activeElement.classList.contains("pad-focus"),
      "nothing shows where the controller is"
    );
    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([down])];
    ui.stepForTest();

    pads = [pad([])];
    ui.stepForTest();
    pads = [pad([go])];
    ui.stepForTest();
    assert.equal(clicked, "b", `pressed ${clicked} instead of the second button`);
    d.remove();
  });

  test("B backs out through whatever the dialog calls cancelling", () => {
    const map = ui.padMap();
    const back = Number(Object.entries(map).find(([, a]) => a === "back")?.[0]);
    const d = openDialog();
    let cancelled = false;
    d.querySelector(".cancel").addEventListener("click", () => (cancelled = true));

    pads = [pad([back])];
    ui.stepForTest();
    assert.equal(cancelled, true, "the pad could not back out of the dialog");
    d.remove();
  });

  /// While a dialog is up it owns the pad: moving the cursor in the library
  /// behind it is how someone answers a question about a game they are no
  /// longer looking at.
  test("the library behind it does not move", () => {
    const map = ui.padMap();
    const down = Number(Object.entries(map).find(([, a]) => a === "down")?.[0]);
    document.getElementById("list").innerHTML =
      `<div class="gcards"><div class="gcard sel" data-id="1"></div><div class="gcard" data-id="2"></div></div>`;
    const d = openDialog();
    pads = [pad([down])];
    ui.stepForTest();
    assert.ok(
      document.querySelector('.gcard[data-id="1"]').classList.contains("sel"),
      "the selection moved in the library while a dialog was open"
    );
    d.remove();
  });
});

describe("search results group the twins together", () => {
  const heads = () =>
    [...document.querySelectorAll("#list .pgroup .gslug")].map((n) => n.textContent);

  const search = async (rows) => {
    ui.state.view = "search";
    ui.state.platform = null;
    ui.state.rows = rows;
    // These hand rows to the page directly rather than through `search`, so
    // the stand-in backend is told what the list holds.
    backend.rows(rows);
    await ui.arrangeCurrentList();
    ui.renderRows(rows, true);
  };

  const game = (id, platform, name) => ({
    id,
    name,
    fs_name: `${name}.rom`,
    platform,
    size_bytes: 1,
    downloaded: true,
    favorite: false,
  });

  /// The NES release and the Famicom one are the same game on the same
  /// hardware sold in two places. Ordering purely by hit count put eight other
  /// consoles between them.
  test("nes and famicom sit next to each other", async () => {
    await search([
      game(1, "psx", "Zelda-ish"),
      game(2, "psx", "Another"),
      game(3, "psx", "Third"),
      game(4, "nes", "Zelda"),
      game(5, "nes", "Zelda 2"),
      game(6, "famicom", "Zelda J"),
      game(7, "gba", "Zelda GBA"),
    ]);
    const order = heads();
    const nes = order.indexOf("nes");
    const fc = order.indexOf("famicom");
    assert.notEqual(nes, -1, `no nes group: ${order}`);
    assert.equal(fc, nes + 1, `famicom is not behind nes: ${order}`);
  });

  test("snes and sfc too, and the home-market name leads", async () => {
    await search([
      game(1, "sfc", "Mario J"),
      game(2, "snes", "Mario"),
      game(3, "snes", "Mario 2"),
      game(4, "megadrive", "Sonic"),
    ]);
    const order = heads();
    assert.equal(order.indexOf("sfc"), order.indexOf("snes") + 1, `${order}`);
  });

  /// A family's place in the list is decided by both halves together: three
  /// hits plus two beats a console with four, because it is one machine.
  test("a family is weighed as one console", async () => {
    await search([
      game(1, "psx", "A"),
      game(2, "psx", "B"),
      game(3, "psx", "C"),
      game(4, "psx", "D"),
      game(5, "nes", "E"),
      game(6, "nes", "F"),
      game(7, "nes", "G"),
      game(8, "famicom", "H"),
      game(9, "famicom", "I"),
    ]);
    const order = heads();
    assert.deepEqual(order.slice(0, 3), ["nes", "famicom", "psx"], `${order}`);
  });
});

describe("the triggers scroll by how hard they are pulled", () => {
  const map = () => ui.padMap();
  const lt = () => Number(Object.entries(map()).find(([, a]) => a === "scrollUp")?.[0]);
  const rt = () => Number(Object.entries(map()).find(([, a]) => a === "scrollDown")?.[0]);

  /// jsdom has no layout, so a real element's scrollTop is clamped to zero
  /// forever — nothing has a height, so nothing can scroll. An own property
  /// shadows the prototype's accessor and gives the list somewhere to record
  /// the movement, which is the thing under test.
  const scrollable = () => {
    const list = document.getElementById("list");
    let top = 1000;
    Object.defineProperty(list, "scrollTop", {
      configurable: true,
      get: () => top,
      set: (v) => (top = v),
    });
    return list;
  };

  /// One frame of the trigger pulled that far, straight into the reader —
  /// the poll loop around it has its own state (settling windows, repeat
  /// timers) that has nothing to do with how an analogue trigger is read.
  const pull = (index, value) => {
    const list = scrollable();
    const before = list.scrollTop;
    const consumed = ui.analogueScroll([pad([], { analogue: { [index]: value } })], map());
    return { moved: list.scrollTop - before, consumed };
  };

  test("a harder pull scrolls further than a light one", () => {
    const light = pull(rt(), 0.3).moved;
    const hard = pull(rt(), 1).moved;
    assert.ok(light > 0, "a light pull did nothing at all");
    assert.ok(hard > light * 2, `a full pull (${hard}) barely beat a light one (${light})`);
  });

  test("the left one goes the other way", () => {
    assert.ok(pull(lt(), 1).moved < 0, "the left trigger scrolled down");
  });

  /// A trigger at rest reports a small value on plenty of pads, and a list
  /// that creeps on its own is worse than one that does not move.
  test("a resting trigger does not creep", () => {
    assert.equal(pull(rt(), 0.03).moved, 0);
  });

  /// A trigger past its threshold reads as "pressed" as well, so the poll has
  /// to be told this one is already dealt with — otherwise it jumps a fixed
  /// step on every repeat on top of the smooth movement.
  test("a pulled trigger is reported as already handled", () => {
    assert.deepEqual([...pull(rt(), 1).consumed], ["scrollDown"]);
    assert.deepEqual([...pull(rt(), 0.02).consumed], [], "a resting trigger is not consumed");
  });
});

describe("the settings shortcut", () => {
  // The keyboard handler is attached by installKeys, which the app calls at
  // startup and this file otherwise has no reason to.
  before(() => ui.installKeys());

  /// Cmd+, on macOS, Ctrl+, everywhere else. The one shortcut every desktop
  /// application has, and this one did not.
  const press = (init) =>
    window.dispatchEvent(new window.KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));

  test("Cmd+, and Ctrl+, both open settings", () => {
    invoked.length = 0;
    press({ key: ",", metaKey: true });
    assert.ok(
      invoked.some((c) => c.cmd === "open_settings"),
      "Cmd+, did not open settings"
    );

    invoked.length = 0;
    press({ key: ",", ctrlKey: true });
    assert.ok(invoked.some((c) => c.cmd === "open_settings"), "Ctrl+, did not");
  });

  test("a bare comma does not", () => {
    invoked.length = 0;
    press({ key: "," });
    assert.equal(invoked.some((c) => c.cmd === "open_settings"), false);
  });
});

describe("take offline: collections as well as systems", () => {
  /// The pane could take whole systems, or the one collection you had open —
  /// and nothing in between. Picking four of your own collections for a trip
  /// meant opening the pane four times.
  test("there is a tab for them, listing what is not here yet", async () => {
    ui.askDownload({});
    await settle();
    const tabs = [...document.querySelectorAll(".bulk-tab")].map((t) => t.dataset.tab);
    assert.deepEqual(tabs, ["systems", "mine"]);
    const boxes = [...document.querySelectorAll(".bulk-coll")].map((c) => c.value);
    assert.deepEqual(boxes, ["c1", "c2"]);
    // 50 in it, 20 already here: what this costs is the other 30.
    assert.match(document.querySelector(".bulk-list[data-tab='mine']").textContent, /30 to get/);
  });

  test("ticking them sends them, alongside any systems", async () => {
    ui.askDownload({});
    await settle();
    document.querySelector('.bulk-plat[value="snes"]').checked = true;
    document.querySelector('.bulk-coll[value="c2"]').checked = true;
    invoked.length = 0;
    document.querySelector(".bulk-size").click();
    await settle();
    const est = invoked.find((c) => c.cmd === "download_estimate");
    assert.ok(est, "nothing was estimated");
    assert.deepEqual(est.args.choice.platforms, ["snes"]);
    assert.deepEqual(est.args.choice.collections, ["c2"]);
  });

  /// All and None act on the list you are looking at. Ticking every collection
  /// because you pressed All on the systems tab is a download nobody asked
  /// for.
  test("All only fills the tab you are on", async () => {
    ui.askDownload({});
    await settle();
    document.querySelector(".bulk-all").click();
    assert.equal(document.querySelectorAll(".bulk-plat:checked").length, 3);
    assert.equal(document.querySelectorAll(".bulk-coll:checked").length, 0);

    document.querySelector('.bulk-tab[data-tab="mine"]').click();
    document.querySelector(".bulk-all").click();
    assert.equal(document.querySelectorAll(".bulk-coll:checked").length, 2);
    document.getElementById("bulk-overlay")?.remove();
  });
});

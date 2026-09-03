// The save-state shelf, and the history page.
//
// The shelf is worth testing because its failure mode is quiet and expensive:
// a state is the only record of where you are in a game, it cannot be
// downloaded again, and the difference between "resume slot 3" and "start from
// the title screen" is invisible until an hour of play has gone. So what is
// checked here is that the slot actually travels to the backend, and that the
// autosave — which has no slot number and cannot be entered — is not offered as
// though it could be.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

/// Press a menu item the way a mouse presses it, and check the menu survived
/// the press.
///
/// `.click()` skips pointerdown, which is where this was broken for months:
/// the menu closed on the next pointerdown anywhere, so it came off the page
/// between the press and the release and no click ever reached the button.
/// jsdom will still deliver a click to a node that has been removed from the
/// document, so a test that only calls `.click()` passes against an app in
/// which every menu is dead — which is how "delete a save state" was reported
/// four times.
function pressItem(node) {
  node.dispatchEvent(new dom.window.MouseEvent("pointerdown", { bubbles: true }));
  assert.ok(node.isConnected, "the press itself dismissed the menu");
  for (const type of ["pointerup", "click"]) {
    node.dispatchEvent(new dom.window.MouseEvent(type, { bubbles: true }));
  }
}

let dom, detail, history, invoked, states, historyData;

/// Enough of a detail for the pane to render without reaching for something
/// that is not there; the shelf is the part under test.
const DETAIL = {
  id: 7,
  name: "Chrono Trigger",
  fs_name: "ct.sfc",
  platform: "Super Nintendo",
  size_bytes: 4_194_304,
  downloaded: true,
  screenshots: [],
  genres: [],
  companies: [],
  franchises: [],
  game_modes: [],
  regions: [],
  alt_names: [],
  art: {},
};

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.HTMLElement = dom.window.HTMLElement;
  global.localStorage = dom.window.localStorage;
  global.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
  Object.defineProperty(global, "navigator", {
    value: dom.window.navigator,
    configurable: true,
  });

  dom.window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        invoked.push({ cmd, args });
        if (cmd === "rom_detail") return DETAIL;
        if (cmd === "game_cores") return [];
        if (cmd === "game_states") return states;
        if (cmd === "play_history") return historyData;
        if (cmd === "launch_rom") return "played for 3 minutes";
        if (cmd === "game_video") return "/v/clip.mp4";
        if (cmd === "confirm_delete_state") return false;
        if (cmd === "delete_state") return "deleted Slot 3";
        if (cmd === "set_config_field") return "saved";
        return [];
      },
      convertFileSrc: (p) => `asset://${p}`,
    },
    event: { listen: async () => () => {} },
  };

  detail = await import("../js/detail.js");
  history = await import("../js/history.js");

  // The launch path measures the display over 24 animation frames before it
  // sends anything. Taken here, once, so a click in a test reaches the backend
  // in the same tick it would in an app that has been open for a second.
  const actions = await import("../js/actions.js");
  actions.warmRefresh();
  await frames(40);
});

/// Let `n` animation frames pass.
const frames = (n) =>
  new Promise((done) => {
    let left = n;
    const tick = () => (left-- > 0 ? dom.window.requestAnimationFrame(tick) : done());
    tick();
  });

beforeEach(() => {
  invoked = [];
  states = [];
  historyData = { total_seconds: 0, sessions: 0, games: 0, platforms: [], top: [], abandoned: [] };
});

const settle = () => new Promise((r) => dom.window.setTimeout(r, 0));

describe("the save-state shelf", () => {
  test("a game with no states shows no shelf at all", async () => {
    await detail.selectRom(7);
    await settle();
    assert.equal(document.querySelector(".shelf"), null, "an empty shelf is clutter");
  });

  test("each state is on the shelf, with its picture and how long ago", async () => {
    states = [
      {
        slot: "auto",
        label: "Where you left off",
        thumb: "/s/ct.state.auto.png",
        when: "2 hours ago",
        size_bytes: 900,
        core: "snes9x",
        resumable: false,
      },
      {
        slot: "3",
        label: "Slot 3",
        thumb: "/s/ct.state3.png",
        when: "6 days ago",
        size_bytes: 900,
        core: "snes9x",
        resumable: true,
      },
    ];
    await detail.selectRom(7);
    await settle();

    const shelf = [...document.querySelectorAll(".state")];
    assert.equal(shelf.length, 2);
    assert.match(shelf[0].textContent, /Where you left off/);
    assert.match(shelf[0].textContent, /2 hours ago/);
    assert.match(shelf[1].querySelector("img").src, /ct\.state3\.png/);
  });

  /// The autosave is the newest thing on the shelf and the one people will
  /// reach for. It has no slot number, so RetroArch cannot be told to enter it
  /// — `--entryslot auto` is not a thing. Offering it as a button that quietly
  /// starts the game from the beginning would be the worst outcome here.
  test("the autosave is shown but cannot be started from", async () => {
    states = [
      {
        slot: "auto",
        label: "Where you left off",
        thumb: null,
        when: "just now",
        size_bytes: 900,
        core: "snes9x",
        resumable: false,
      },
    ];
    await detail.selectRom(7);
    await settle();

    const auto = document.querySelector(".state");
    assert.ok(auto, "the autosave should still be visible");
    // Grayed, but not `disabled`: a disabled button fires no mouse events at
    // all, so marking it disabled also made it impossible to right-click —
    // and right-click is the only way to delete a state.
    assert.equal(auto.disabled, false, "a disabled button cannot be right-clicked");
    assert.ok(auto.classList.contains("noresume"), "it must not look like it works");

    auto.click();
    await settle();
    assert.equal(
      invoked.filter((c) => c.cmd === "launch_rom").length,
      0,
      "clicking it started the game — from the title screen, silently"
    );
  });

  /// The one this keeps failing on. A state that cannot be resumed still has
  /// to be deletable, and the only route to that is the right-click menu.
  test("the autosave can be right-clicked even though it cannot be started", async () => {
    states = [
      {
        slot: "auto",
        label: "Where you left off",
        thumb: null,
        when: "just now",
        size_bytes: 900,
        core: "FinalBurn Neo",
        resumable: false,
      },
    ];
    await detail.selectRom(7);
    await settle();

    const auto = document.querySelector(".state");
    auto.dispatchEvent(
      new dom.window.MouseEvent("contextmenu", { bubbles: true, cancelable: true })
    );
    await settle();

    const menu = document.querySelector(".ctx-menu");
    assert.ok(menu, "no menu on the one state that can only be deleted");
    assert.match(menu.textContent, /Delete/);
    pressItem(menu.querySelector("button"));
    await settle();
    assert.equal(
      invoked.find((c) => c.cmd === "delete_state")?.args.slot,
      "auto",
      "the autosave was not the state that got deleted"
    );
  });

  test("starting from a state sends that slot and no other", async () => {
    states = [
      {
        slot: "2",
        label: "Slot 2",
        thumb: null,
        when: "yesterday",
        size_bytes: 900,
        core: "snes9x",
        resumable: true,
      },
    ];
    await detail.selectRom(7);
    await settle();

    document.querySelector(".state").click();
    await settle();

    const launch = invoked.find((c) => c.cmd === "launch_rom");
    assert.ok(launch, "the shelf did not launch anything");
    assert.equal(launch.args.entrySlot, 2);
    assert.equal(launch.args.id, 7);
  });

  /// A state whose picture was never saved still has to be pickable. States
  /// made before thumbnails were switched on have none, and no amount of
  /// asking will produce one — the frame is not in the file.
  test("a state with no picture is still on the shelf", async () => {
    states = [
      {
        slot: "5",
        label: "Slot 5",
        thumb: null,
        when: "a month ago",
        size_bytes: 900,
        core: "snes9x",
        resumable: true,
      },
    ];
    await detail.selectRom(7);
    await settle();

    const btn = document.querySelector(".state");
    assert.equal(btn.querySelector("img"), null);
    assert.match(btn.querySelector(".state-blank").textContent, /5/);
    assert.equal(btn.disabled, false);
  });
});

describe("the history page", () => {
  /// A fresh install has nothing recorded, and three empty headings read as a
  /// broken page rather than a new one.
  test("with nothing recorded it says so rather than showing empty lists", async () => {
    await history.showHistory();
    await settle();
    assert.match(document.getElementById("list").textContent, /Nothing recorded yet/);
    assert.equal(document.querySelector(".hist-bar"), null);
  });

  test("consoles are drawn in proportion to the longest one", async () => {
    historyData = {
      total_seconds: 36_000,
      sessions: 12,
      games: 4,
      platforms: [
        { slug: "snes", name: "Super Nintendo", seconds: 27_000, spelled: "7 h 30 m", sessions: 8, games: 3 },
        { slug: "psx", name: "PlayStation", seconds: 9_000, spelled: "2 h 30 m", sessions: 4, games: 1 },
      ],
      top: [
        { id: 1, name: "Chrono Trigger", platform: "Super Nintendo", seconds: 20_000, spelled: "5 h 33 m", sessions: 5, last: "2026-08-01T10:00:00" },
      ],
      abandoned: [],
    };
    await history.showHistory();
    await settle();

    const widths = [...document.querySelectorAll(".hist-bar span")].map((b) => b.style.width);
    // The longest console fills its bar; everything else is measured against
    // it. Scaling against an absolute maximum leaves every console after the
    // first as an unreadable stub.
    assert.equal(widths[0], "100%");
    assert.match(widths[1], /^33\.3/);
  });

  test("the picked-up-and-put-down list only appears when there is one", async () => {
    historyData = {
      total_seconds: 3600,
      sessions: 4,
      games: 2,
      platforms: [{ slug: "snes", name: "Super Nintendo", seconds: 3600, spelled: "1 h 00 m", sessions: 4, games: 2 }],
      top: [],
      abandoned: [],
    };
    await history.showHistory();
    await settle();
    assert.doesNotMatch(document.getElementById("list").textContent, /Picked up and put down/);

    historyData.abandoned = [
      { id: 9, name: "Bounced Off", platform: "Super Nintendo", seconds: 900, spelled: "15 minutes", sessions: 3, last: null },
    ];
    await history.showHistory();
    await settle();
    const text = document.getElementById("list").textContent;
    assert.match(text, /Picked up and put down/);
    assert.match(text, /3 goes/);
  });
});

describe("browsing a game's media", () => {
  /// Each thing used to open on its own. Clicking the cart art gave you the
  /// cart art and nothing else; pressing Y gave you the video and nothing else.
  /// So the arrow keys had a set of one to walk through and looked broken —
  /// where ES-DE treats a game's media as one reel.
  test("opening one picture opens the whole reel, positioned on it", async () => {
    Object.assign(DETAIL, {
      art: { miximages: "/a/mix.png", "3dboxes": "/a/box.png", physicalmedia: "/a/cart.png" },
      cover: "/a/cover.png",
      screenshots: ["/a/s1.png"],
      has_video: true,
    });
    await detail.selectRom(7);
    await settle();

    const cart = document.querySelector('.artstrip figure[data-art="physicalmedia"]');
    assert.ok(cart, "the cart art is not in the strip");
    cart.click();
    await settle();

    const caption = document.querySelector("#lightbox figcaption").textContent;
    // "n of m" only appears when there is more than one thing to walk to.
    assert.match(caption, /Cart\/disc/);
    assert.match(caption, /of \d+/, `a set of one: ${caption}`);
  });

  /// The order is the strip's order, so stepping right lands on the picture to
  /// the right rather than somewhere the layout does not explain.
  test("the arrows walk the reel in the order it is drawn", async () => {
    await detail.selectRom(7);
    await settle();
    document.querySelector('.artstrip figure[data-art="miximages"]').click();
    await settle();

    const captionNow = () =>
      document.querySelector("#lightbox figcaption").textContent.split(" — ")[0];
    assert.equal(captionNow(), "Mix");

    const lb = await import("../js/lightbox.js");
    lb.stepLightbox(1);
    assert.equal(captionNow(), "3D box", "stepping right left the strip's order");
    lb.stepLightbox(-1);
    assert.equal(captionNow(), "Mix");
    // And it wraps, so holding a direction never dead-ends.
    lb.stepLightbox(-1);
    assert.notEqual(captionNow(), "Mix");
    lb.closeLightbox();
  });
});

describe("the video, where ES-DE puts it", () => {
  /// It was a button at the top of the pane and the last thing in the reel, so
  /// the arrows walked a dozen pictures before reaching it and could not reach
  /// it from the left at all. It is the first tile in the artwork strip now,
  /// and the first thing in the reel.
  test("it leads the artwork strip", async () => {
    Object.assign(DETAIL, {
      has_video: true,
      art: { miximages: "/a/mix.png", physicalmedia: "/a/cart.png" },
    });
    await detail.selectRom(7);
    await settle();
    const first = document.querySelector(".artstrip figure");
    assert.equal(first?.dataset.art, "video", "the strip still starts with a picture");
  });

  /// A tag, not a button: it says a video exists and waits to be asked. The
  /// manual and the trailer were links at the very foot of the pane, below the
  /// artwork, the save states and every metadata row, where nobody found them.
  test("what a game has is one row of tags near the top", async () => {
    Object.assign(DETAIL, { has_video: true, manual: "/a/m.pdf", youtube_id: "abc123" });
    await detail.selectRom(7);
    await settle();
    const tags = [...document.querySelectorAll("#detail .badges .badge")].map((b) =>
      b.textContent.trim()
    );
    const trailerIcon = () => document.querySelector("#trailer .icon");
    assert.deepEqual(tags, ["Video", "Manual", "Trailer"]);
    assert.equal(document.querySelector("#detail .extras"), null, "the old foot row is still there");
    // Not a play triangle: that one promises the video plays here, the way the
    // tag beside it does, and this one leaves the app for a browser.
    assert.ok(
      trailerIcon().classList.contains("icon-external"),
      "the trailer still wears a play icon"
    );
    // A webview follows a link in place, so YouTube would have replaced the
    // library with no address bar and no way back.
    const trailer = document.getElementById("trailer");
    assert.equal(trailer.tagName, "BUTTON", "the trailer still navigates this window");
    trailer.click();
    await settle();
    const call = invoked.find((c) => c.cmd === "open_link");
    assert.ok(call, "the trailer did not go out to the browser");
    assert.match(call.args.url, /youtube\.com\/watch\?v=abc123/);
  });

  test("pressing it opens the player", async () => {
    Object.assign(DETAIL, { has_video: true, art: { miximages: "/a/mix.png" } });
    await detail.selectRom(7);
    await settle();

    const btn = document.getElementById("playvid");
    assert.ok(btn, "no video tag for a game that has one");

    await detail.playVideo();
    await settle();

    const lb = await import("../js/lightbox.js");
    assert.equal(lb.isLightboxOpen(), true, "the player did not open");
    assert.ok(
      document.querySelector("#lightbox video"),
      "the player opened on something that is not the video"
    );
    // First in the reel, so one step left from it is the last picture rather
    // than nothing at all.
    assert.match(
      document.querySelector("#lightbox figcaption").textContent,
      /1 of|Gameplay/,
      "the video is not at the head of the reel"
    );
    lb.closeLightbox();
  });

  /// Y opens it, and the pane it used to live in can be hidden entirely. The
  /// game is the authority on whether there is a video, not the presence of a
  /// button on screen.
  test("it plays with the pane hidden", async () => {
    Object.assign(DETAIL, { has_video: true });
    await detail.selectRom(7);
    await settle();
    document.getElementById("playvid").remove();

    await detail.playVideo();
    await settle();
    const lb = await import("../js/lightbox.js");
    assert.equal(lb.isLightboxOpen(), true, "Y needs the button to be on screen");
    lb.closeLightbox();
  });
});

describe("the right-click menu", () => {
  /// It shipped with no stylesheet, so it was laid out in the normal flow and
  /// appeared past the end of the page — a menu that existed in the DOM and
  /// could not be seen or reached. Nothing about the JavaScript said so.
  test("right-clicking a save state opens a menu that is actually placed", async () => {
    states = [
      {
        slot: "2",
        label: "Slot 2",
        thumb: null,
        when: "yesterday",
        size_bytes: 900,
        core: "snes9x",
        resumable: true,
      },
    ];
    await detail.selectRom(7);
    await settle();

    const btn = document.querySelector(".state");
    const ev = new dom.window.MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 120,
      clientY: 90,
    });
    btn.dispatchEvent(ev);
    await settle();

    assert.equal(ev.defaultPrevented, true, "the browser's own menu was left to appear");
    const menu = document.querySelector(".ctx-menu");
    assert.ok(menu, "no menu opened");
    // Placed where the click was. That the rule making the placement mean
    // anything exists at all is checked against the stylesheet, in
    // hiding.test.js — that is the half that was missing.
    assert.equal(menu.style.left, "120px");
    assert.equal(menu.style.top, "90px");
    assert.match(menu.textContent, /Delete/);
  });

  test("choosing delete asks the backend for that slot", async () => {
    states = [
      {
        slot: "3",
        label: "Slot 3",
        thumb: null,
        when: "yesterday",
        size_bytes: 900,
        core: "snes9x",
        resumable: true,
      },
    ];
    await detail.selectRom(7);
    await settle();

    document.querySelector(".state").dispatchEvent(
      new dom.window.MouseEvent("contextmenu", { bubbles: true, cancelable: true })
    );
    await settle();
    pressItem(document.querySelector(".ctx-menu button"));
    await settle();

    const call = invoked.find((c) => c.cmd === "delete_state");
    assert.ok(call, "nothing was deleted");
    assert.equal(call.args.slot, "3");
    assert.equal(call.args.id, 7);
  });
});

describe("rapid fire, above the Play button", () => {
  /// It is a thing you change about the run you are about to start — set the
  /// rate, play a level, decide it wants to be slower — so it sits with Play
  /// rather than three windows away in Settings, and does not scroll off with
  /// the artwork.
  test("games that can have it get the control, pinned with Play", async () => {
    Object.assign(DETAIL, { autofire: "rb" });
    await detail.selectRom(7);
    await settle();

    const row = document.querySelector(".autofire-row");
    assert.ok(row, "no rapid-fire control");
    assert.ok(
      row.closest(".pinned"),
      "it is in the scrolling part, so it disappears as you read the game"
    );
    assert.ok(
      row.compareDocumentPosition(document.getElementById("play")) &
        dom.window.Node.DOCUMENT_POSITION_FOLLOWING,
      "it should sit above Play, not below it"
    );
    assert.equal(row.querySelectorAll(".af").length, 3, "three choices");
    assert.equal(row.querySelector(".af.on").dataset.af, "rb");
  });

  test("games it does not apply to get nothing at all", async () => {
    // Every shape "not applicable" can arrive in. A console game, a game with
    // no metadata, an older backend that does not send the field: all of them
    // must draw nothing rather than a row with no selection.
    for (const value of [null, undefined, "", "maybe", 0]) {
      Object.assign(DETAIL, { autofire: value });
      await detail.selectRom(7);
      await settle();
      assert.equal(
        document.querySelector(".autofire-row"),
        null,
        `a rapid-fire row appeared for autofire=${JSON.stringify(value)}`
      );
    }
  });

  /// "Off" and "not applicable" are different answers: one shows a control
  /// with Off selected, the other shows no control.
  test("off still shows the control, with off selected", async () => {
    Object.assign(DETAIL, { autofire: "off" });
    await detail.selectRom(7);
    await settle();
    assert.ok(document.querySelector(".autofire-row"), "the control vanished when turned off");
    assert.equal(document.querySelector(".af.on").dataset.af, "off");
  });

  test("choosing one saves it", async () => {
    Object.assign(DETAIL, { autofire: "off" });
    await detail.selectRom(7);
    await settle();
    document.querySelector('.af[data-af="lb"]').click();
    await settle();
    const saved = invoked.find((c) => c.cmd === "set_config_field");
    assert.ok(saved, "nothing was saved");
    assert.equal(saved.args.field, "autofire");
    assert.equal(saved.args.value, "lb");
    Object.assign(DETAIL, { autofire: null });
  });
});

describe("how fast the rapid fire goes", () => {
  test("the rate shows and steps by one", async () => {
    Object.assign(DETAIL, { autofire: "rb", autofire_hz: 5 });
    await detail.selectRom(7);
    await settle();

    assert.match(document.querySelector(".af-hz").textContent, /5 Hz/);
    document.querySelector('.af-step[data-hz="1"]').click();
    await settle();

    // For this run of the app, not for the file: five taps to go from six to
    // eleven would be five rewrites of config.toml.
    const saved = invoked.filter((c) => c.cmd === "set_autofire_hz").at(-1);
    assert.ok(saved, "the rate went nowhere");
    assert.equal(saved.args.hz, 6);
    assert.equal(
      invoked.filter((c) => c.cmd === "set_config_field" && c.args.field === "autofire_hz").length,
      0,
      "the rate wrote itself to disk"
    );
  });

  /// Shown next to "Off" as well. The rate is something you may want to set
  /// before switching on, and hiding it also made the row change width as you
  /// moved between the three.
  test("the rate is there even when rapid fire is off", async () => {
    Object.assign(DETAIL, { autofire: "off", autofire_hz: 5 });
    await detail.selectRom(7);
    await settle();
    assert.ok(document.querySelector(".autofire-row"), "the row should still be there");
    assert.ok(document.querySelector(".af-rate"), "the rate disappeared when off");
    assert.match(document.querySelector(".af-hz").textContent, /5 Hz/);
  });

  test("and can be set while off, ready for when it is turned on", async () => {
    Object.assign(DETAIL, { autofire: "off", autofire_hz: 5 });
    await detail.selectRom(7);
    await settle();
    document.querySelector('.af-step[data-hz="1"]').click();
    await settle();
    const saved = invoked.filter((c) => c.cmd === "set_autofire_hz").at(-1);
    assert.equal(saved.args.hz, 6);
    assert.match(document.querySelector(".af-hz").textContent, /6 Hz/);
  });

  /// Below one it would divide by zero inside the emulator, and above thirty
  /// it is faster than the game can read.
  test("it does not step past its limits", async () => {
    Object.assign(DETAIL, { autofire: "lb", autofire_hz: 1 });
    await detail.selectRom(7);
    await settle();
    invoked.length = 0;
    document.querySelector('.af-step[data-hz="-1"]').click();
    await settle();
    assert.equal(
      invoked.some((c) => c.cmd === "set_autofire_hz"),
      false,
      "it set a rate below one"
    );
    Object.assign(DETAIL, { autofire: null, autofire_hz: 6 });
  });
});

describe("changing rapid fire keeps your place", () => {
  /// The control is at the bottom of the pane, and rebuilding the whole pane
  /// to show the new value sent the reader back to the top of it — so every
  /// press of + threw away where they were.
  test("stepping the rate does not redraw the pane", async () => {
    Object.assign(DETAIL, { autofire: "rb", autofire_hz: 5 });
    await detail.selectRom(7);
    await settle();

    const before = invoked.filter((c) => c.cmd === "rom_detail").length;
    const scroll = document.querySelector("#detail .scroll");
    scroll.scrollTop = 120;

    document.querySelector('.af-step[data-hz="1"]').click();
    await settle();

    assert.equal(
      invoked.filter((c) => c.cmd === "rom_detail").length,
      before,
      "the whole pane was rebuilt, which is what loses the scroll position"
    );
    assert.match(document.querySelector(".af-hz").textContent, /6 Hz/, "the rate did not update");
  });

  /// Turning it off wrote a boolean, because the field was still on the list
  /// of settings that are booleans from when it was a toggle. The mode name is
  /// not "true", so it stored false — off. You could turn it off and never on.
  test("off and back on again", async () => {
    Object.assign(DETAIL, { autofire: "rb", autofire_hz: 5 });
    await detail.selectRom(7);
    await settle();

    document.querySelector('.af[data-af="off"]').click();
    await settle();
    assert.equal(document.querySelector(".af.on").dataset.af, "off");

    document.querySelector('.af[data-af="lb"]').click();
    await settle();
    assert.equal(
      document.querySelector(".af.on").dataset.af,
      "lb",
      "it could be turned off but not back on"
    );
    assert.ok(document.querySelector(".af-rate"), "the rate should come back");

    const saved = invoked.filter((c) => c.cmd === "set_config_field").map((c) => c.args.value);
    assert.deepEqual(saved.slice(-2), ["off", "lb"], `sent: ${saved}`);
    Object.assign(DETAIL, { autofire: null, autofire_hz: 5 });
  });
});

describe("closing the preview", () => {
  /// It is a third of the window when everything is side by side, and its
  /// toggle used to sit in the header among eight unrelated buttons. It is at
  /// the end of the tab row now — with what it acts on — which is also why the
  /// pane no longer carries a cross of its own: one control, in one place.
  test("the pane has no close button of its own", async () => {
    Object.assign(DETAIL, { has_video: false });
    await detail.selectRom(7);
    await settle();
    assert.equal(
      document.querySelector("#detail .pane-close"),
      null,
      "two controls for one thing"
    );
    assert.equal(document.getElementById("detail").hidden, false);
  });

  test("the toggle closes it", async () => {
    detail.setSidebar(false);
    await settle();
    assert.equal(document.getElementById("detail").hidden, true, "it did not close");
  });

  /// And the choice sticks. A preview that reopens itself on the next game is
  /// a button that appears not to have worked.
  test("it stays closed until asked back", async () => {
    detail.setSidebar(false);
    await detail.selectRom(7);
    await settle();
    assert.equal(document.getElementById("detail").hidden, true, "it came back on its own");

    detail.setSidebar(true);
    await detail.selectRom(7);
    await settle();
    assert.equal(document.getElementById("detail").hidden, false, "it will not come back");
  });
});

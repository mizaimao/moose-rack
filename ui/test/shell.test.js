// The seam a different layout is built through.
//
// The claim this file has to keep honest is that the views no longer know what
// the window looks like. That was not true before: every screen set the top bar
// itself — hide Back, show Grid, show the zoom slider unless we are in list
// mode — six or seven imperative lines repeated in each of six functions, each
// copy stating in code that this is a single-pane app with a back button.
// Forty-three of those lines, and every one of them would have had to change
// for a three-column layout.
//
// So the test builds a three-column skeleton — consoles down the left, games
// in the middle, preview on the right, no Back button at all because nothing
// is ever replaced — points the shell at it, and drives the ordinary view code
// against it. Nothing in ui/js knows this file exists.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let dom, shell, el;

/// Every command the page has asked the backend for, so a click can be checked
/// by what it went and did rather than by what it drew.
const asked = [];

/// A layout this app does not have. Deliberately missing things the current
/// one takes for granted: there is no #back, because a column that is always
/// on screen is never navigated away from.
const THREE_COLUMNS = `
  <main class="cols">
    <nav id="col-consoles"></nav>
    <section id="col-games"></section>
    <aside id="col-preview"></aside>
  </main>`;

before(async () => {
  dom = new JSDOM(readFileSync(join(uiDir, "index.html"), "utf8"), {
    url: "http://localhost/",
    pretendToBeVisual: true,
  });
  global.window = dom.window;
  global.document = dom.window.document;
  global.localStorage = dom.window.localStorage;
  // The views build selectors with CSS.escape. jsdom provides it on the
  // window; the modules reach for the bare global, as they do in a browser.
  global.CSS = dom.window.CSS;
  dom.window.__TAURI__ = {
    core: {
      // Selecting a row draws the info pane, which joins several of these
      // arrays — a thinner stub throws inside the pane, after the test has
      // finished, as an unhandled rejection rather than a failure.
      invoke: async (cmd, args) => (
        asked.push(cmd),
        cmd === "collection_groups"
          ? [{ group: "genre", label: "Genre", count: 3 }]
          : cmd === "collections_in"
            ? [
                { id: "c1", name: "First", rom_count: 5, local_count: 5, sample_ids: [11] },
                { id: "c2", name: "Second", rom_count: 7, local_count: 0, sample_ids: [12] },
              ]
            : cmd === "rom_covers"
              ? [
                  { id: 11, cover: "/art/11.png" },
                  { id: 12, cover: "/art/12.png" },
                ]
            : cmd === "platforms"
          ? [
              { slug: "arcade", name: "Arcade", rom_count: 9 },
              { slug: "gb", name: "Game Boy", rom_count: 4 },
            ]
          : cmd === "rom_detail"
          ? {
              id: args.id,
              name: `Game ${args.id}`,
              fs_name: "g.zip",
              platform: "arcade",
              platform_slug: "arcade",
              size_bytes: 1,
              downloaded: true,
              screenshots: [],
              genres: [],
              companies: [],
              franchises: [],
              game_modes: [],
              regions: [],
              alt_names: [],
              art: {},
            }
          : []),
      convertFileSrc: (p) => p,
    },
    event: { listen: async () => () => {} },
  };

  // The interface commands — bindings, ordering, the grid, the page filter —
  // are answered by the stand-in in backend.js. See the note at the top of
  // that file: it is deliberately naive, and the rules it stands in for are
  // asserted by `cargo test` against the real implementation.
  dom.window.__TAURI__.core.invoke = fakeBackend(dom.window.__TAURI__.core.invoke);
  // jsdom implements no layout, so it has no scrollIntoView — and the list
  // calls it to keep the cursor visible. A no-op is the honest stand-in: there
  // is nothing to scroll into view in a document with no viewport.
  dom.window.Element.prototype.scrollIntoView = function () {};

  shell = await import("../js/shell.js");
  ({ el } = await import("../js/state.js"));
});

beforeEach(() => {
  for (const role of ["primary", "aside", "nav"]) shell.setRegion(role, null);
  document.querySelector(".cols")?.remove();
});

describe("what a view asks for", () => {
  /// A view lists what it needs; everything else is hidden. Before, each view
  /// had to remember to turn off what the previous one turned on, and getting
  /// that wrong left a button on screen that acted on a game that was no
  /// longer there.
  test("anything not asked for is hidden", () => {
    for (const node of [el.back, el.layoutBtn, el.sidebarBtn, el.grabBtn, el.sortBtn]) {
      node.hidden = false;
    }
    shell.enter({ title: "Platforms", layout: true, grab: true });

    assert.equal(el.layoutBtn.hidden, false, "asked for and missing");
    assert.equal(el.grabBtn.hidden, false, "asked for and missing");
    assert.equal(el.back.hidden, true, "not asked for and still there");
    assert.equal(el.sortBtn.hidden, true);
    // Except the preview toggle, which is a fixture of the tab row it lives in
    // rather than a per-view button — see the test below.
    assert.equal(el.sidebarBtn.hidden, false);
    assert.equal(el.title.textContent, "Platforms");
  });

  /// The one conditional every view repeated and each got to decide for
  /// itself: the slider sizes covers, so it means nothing in a list.
  test("the zoom slider follows the covers, not the view", () => {
    shell.enter({ zoom: "grid", gridLayout: true });
    assert.equal(el.zoomWrap.hidden, false);
    shell.enter({ zoom: "grid", gridLayout: false });
    assert.equal(el.zoomWrap.hidden, true, "a zoom slider over a list of names");
    // A view that wants it regardless — the collection cards are cards too.
    shell.enter({ zoom: true, gridLayout: false });
    assert.equal(el.zoomWrap.hidden, false);
  });
});

describe("a layout this app does not have", () => {
  const threeColumns = () => {
    document.body.insertAdjacentHTML("beforeend", THREE_COLUMNS);
    shell.setRegion("nav", document.getElementById("col-consoles"));
    shell.setRegion("primary", document.getElementById("col-games"));
    shell.setRegion("games", document.getElementById("col-games"));
    shell.setRegion("aside", document.getElementById("col-preview"));
  };

  /// The point of the whole exercise: content goes where the shell says, and
  /// the view that produced it never learns where that was.
  test("views draw into whichever columns the shell has been given", () => {
    threeColumns();
    shell.paint("primary", "<div class='game'>Metal Slug</div>");
    shell.paint("nav", "<div class='console'>Arcade</div>");
    shell.paint("aside", "<div class='shot'>art</div>");

    assert.match(document.getElementById("col-games").innerHTML, /Metal Slug/);
    assert.match(document.getElementById("col-consoles").innerHTML, /Arcade/);
    assert.match(document.getElementById("col-preview").innerHTML, /art/);
    // And not into the one-column skeleton's list, which is still in the page.
    assert.equal(el.list.innerHTML, "");
  });

  /// A three-column layout has no Back button, because a column that is always
  /// on screen is never navigated away from. Every view still asks for one.
  test("asking for a button the layout does not have is not an error", () => {
    threeColumns();
    // The handles, not the nodes. `el` is built from getElementById once at
    // import, so a skeleton without these ids holds null — removing the
    // element from the page leaves the handle pointing at a detached node,
    // which still answers `.hidden` and proves nothing.
    const kept = { back: el.back, zoomWrap: el.zoomWrap, sortBtn: el.sortBtn };
    el.back = null;
    el.zoomWrap = null;
    el.sortBtn = null;
    try {
      assert.doesNotThrow(() =>
        shell.enter({ title: "Arcade", back: true, layout: true, zoom: "grid", gridLayout: true })
      );
      assert.doesNotThrow(() => shell.showZoom(true));
    } finally {
      Object.assign(el, kept);
    }
  });

  /// A region the layout does not offer is a region that is not shown, not a
  /// crash: a two-column arrangement with no preview should drop the preview,
  /// not take the window down with it.
  test("painting a region that does not exist is a no-op", () => {
    threeColumns();
    shell.setRegion("aside", null);
    // With no override it falls back to this skeleton's #detail; remove that
    // too, so the role genuinely resolves to nothing.
    const detail = el.detail;
    detail.remove();
    assert.doesNotThrow(() => shell.paint("aside", "<p>preview</p>"));
    document.body.appendChild(detail);
  });

  /// The end of the argument: a real view, unmodified, drawing into the middle
  /// column of a layout it has never heard of. If this needs the view to change
  /// then the seam is in the wrong place.
  test("an actual view renders into the middle column", async () => {
    threeColumns();
    const { renderRows } = await import("../js/library.js");
    const { state } = await import("../js/state.js");
    state.view = "roms";
    state.platform = "arcade";
    state.layout = "list";

    renderRows(
      [
        {
          id: 1,
          name: "Metal Slug",
          fs_name: "mslug.zip",
          platform: "arcade",
          size_bytes: 1,
          downloaded: true,
          favorite: false,
        },
      ],
      false
    );

    assert.match(
      document.getElementById("col-games").innerHTML,
      /Metal Slug/,
      "the game list did not reach the middle column"
    );
    assert.equal(el.list.innerHTML, "", "it went to the one-column skeleton instead");
  });

  test("a role with no mapping anywhere resolves to nothing", () => {
    assert.equal(shell.region("sidebar-that-does-not-exist"), null);
    assert.equal(shell.paint("sidebar-that-does-not-exist", "<p>x</p>"), null);
  });
});

describe("three columns, in the real page", () => {
  /// Not a made-up skeleton this time: the app's own index.html, switched to
  /// the other arrangement. The one element three columns adds is the left
  /// one; the middle and right are the same #list and #detail the single pane
  /// uses, which is why this is a handful of rules rather than a second UI.
  let library, state;

  before(async () => {
    library = await import("../js/library.js");
    ({ state } = await import("../js/state.js"));
  });

  beforeEach(() => {
    // The suite above points the roles at its own made-up columns, and those
    // overrides outlive it — this one is about the real page.
    for (const role of ["primary", "aside", "nav", "games", "picker"]) {
      shell.setRegion(role, null);
    }
    shell.setMode("single");
    el.consoles.innerHTML = "";
    el.list.innerHTML = "";
    state.platform = null;
  });

  test("the consoles get their own column, and the games keep the middle", () => {
    shell.setMode("columns");
    assert.equal(shell.region("picker"), el.consoles, "consoles did not move left");
    assert.equal(shell.region("games"), el.list, "the games left the middle");
    assert.equal(el.consoles.hidden, false, "the left column is still hidden");
    assert.ok(document.body.classList.contains("columns"));
  });

  test("in one pane the consoles and the games share the same element", () => {
    shell.setMode("single");
    assert.equal(shell.region("picker"), el.list);
    assert.equal(shell.region("games"), el.list);
    assert.equal(el.consoles.hidden, true, "the left column is showing in one pane");
  });

  /// The point of the arrangement: opening a console fills the middle and
  /// leaves the console list where it is. In one pane the same call replaces
  /// the screen.
  test("opening a console does not take the console list away", () => {
    shell.setMode("columns");
    el.consoles.innerHTML = `<div class="rows"><div class="prow" data-slug="arcade"></div></div>`;

    library.renderRows(
      [
        {
          id: 1,
          name: "Metal Slug",
          fs_name: "mslug.zip",
          platform: "arcade",
          size_bytes: 1,
          downloaded: true,
          favorite: false,
        },
      ],
      false
    );

    assert.match(el.list.innerHTML, /Metal Slug/, "the games did not reach the middle");
    assert.ok(
      el.consoles.querySelector('[data-slug="arcade"]'),
      "the console list was wiped by opening a console"
    );
  });

  /// Nothing is ever replaced, so there is nothing to go back to and the
  /// preview is a column rather than something to slide over the list.
  /// Reported missing twice. It moved to the tab row to sit with what it acts
  /// on, and a control that comes and goes from a row that never does is a
  /// control nobody finds. Once the preview has been closed it is also the
  /// only way back to it, so hiding it on any screen is how someone ends up
  /// with no preview and no button.
  /// On the console list there is nothing to preview, so the button is dead
  /// rather than gone — the third report of it being missing was it being
  /// hidden, and hiding it is what makes it unfindable.
  test("it is disabled where there is nothing to preview", () => {
    shell.setMode("single");
    shell.enter({ title: "Platforms", layout: true });
    assert.equal(el.sidebarBtn.hidden, false, "hidden again");
    assert.equal(el.sidebarBtn.disabled, true, "it still claims to do something");

    shell.enter({ title: "Arcade", sidebar: true, layout: true });
    assert.equal(el.sidebarBtn.disabled, false, "dead where there is a game to preview");

    // In Desk the preview is a column of the layout, so it is live everywhere.
    shell.setMode("columns");
    shell.enter({ title: "Platforms", layout: true });
    assert.equal(el.sidebarBtn.disabled, false);
  });

  test("Back is not offered, and the preview toggle never disappears", () => {
    for (const mode of ["columns", "single"]) {
      shell.setMode(mode);
      for (const view of [
        { title: "Arcade", back: true, sidebar: true, layout: true },
        { title: "Platforms", layout: true },
        { title: "History" },
      ]) {
        shell.enter(view);
        assert.equal(el.sidebarBtn.hidden, false, `${mode}: gone on ${view.title}`);
      }
    }

    shell.setMode("columns");
    shell.enter({ title: "Arcade", back: true, sidebar: true, layout: true });
    assert.equal(el.back.hidden, true, "a Back button with nowhere to go");
    assert.equal(el.layoutBtn.hidden, false, "grid/list still applies to the games");
    shell.setMode("single");
    shell.enter({ title: "Arcade", back: true, sidebar: true, layout: true });
    assert.equal(el.back.hidden, false);
  });

  test("the choice is remembered", () => {
    shell.chooseMode("columns");
    assert.equal(shell.storedMode(), "columns");
    shell.chooseMode("single");
    assert.equal(shell.storedMode(), "single");
    // Anything unrecognized is the one being worked on, which is the default
    // while three columns is being built.
    shell.chooseMode("nonsense");
    assert.equal(shell.shellMode(), "columns");
    shell.chooseMode("single");
  });
});

describe("each tab fills the left column with its own list", () => {
  /// The left column is not "the consoles" — it is whatever this tab is a list
  /// of. Library gives consoles, My collections gives collections, Browse
  /// gives the groups. It was drawing consoles in every tab because the role
  /// was named after the first thing that used it.
  let collections, history, state;

  before(async () => {
    collections = await import("../js/collections.js");
    history = await import("../js/history.js");
    ({ state } = await import("../js/state.js"));
  });

  beforeEach(() => {
    for (const role of ["primary", "aside", "nav", "games", "picker"]) {
      shell.setRegion(role, null);
    }
    shell.setMode("columns");
    el.consoles.innerHTML = "";
    el.list.innerHTML = "";
  });

  test("My collections puts the collections there, not the consoles", async () => {
    await collections.showCollectionsIn("user", "My collections");
    assert.equal(
      shell.region("picker"),
      el.consoles,
      "the picker is not the left column in three columns"
    );
    // The filter box is in the tab row now, not drawn into the list, so what
    // proves the collections landed here is the collections.
    assert.match(el.consoles.innerHTML, /cgrid|card/, "the collections did not reach the column");
  });

  test("Browse puts the groups there", async () => {
    await collections.showCollectionGroups({ exclude: ["user"] });
    // The stub answers with nothing, which is still an answer drawn into the
    // column rather than into the middle.
    assert.ok(el.consoles.innerHTML.length > 0, "the groups did not reach the column");
  });

  /// History is a page rather than a list with a detail beside it, so it owns
  /// no left column — and leaving the last tab's list there would be a list
  /// that acts on a screen that is gone.
  /// It used to empty the column, which left a 240px strip of nothing down the
  /// left with a drag handle beside it — a list that looks like it failed to
  /// load. There is nothing to pick from on this page, so there is no column.
  test("History takes the column away and draws into the middle", async () => {
    el.consoles.hidden = false;
    el.consoles.innerHTML = "<div class='prow'>Arcade</div>";
    await history.showHistory();
    assert.equal(el.consoles.hidden, true, "an empty column is still on screen");
    assert.match(el.list.innerHTML, /Nothing recorded yet|hist/, "History did not draw");
  });

  /// And gives it back. Hiding it in one view and forgetting to show it in the
  /// next is how a tab ends up with no list at all.
  test("the column comes back with the next tab", async () => {
    await history.showHistory();
    shell.enter({ title: "Library" });
    assert.equal(el.consoles.hidden, false, "Library has no column any more");
  });

  /// It set no view name at all, so the section machinery parked History under
  /// whatever was showing before — usually the console grid — and restored
  /// that instead every time the tab was opened. The tab simply never showed.
  test("History says which view it is", async () => {
    state.view = "platforms";
    await history.showHistory();
    assert.equal(state.view, "history");
  });
});

describe("three columns opens with all three filled", () => {
  /// A window that opens with two thirds of itself empty is asking to be told
  /// what it already knows: which console you were last in, or failing that
  /// the first one.
  let library, state, asked;

  before(async () => {
    library = await import("../js/library.js");
    ({ state } = await import("../js/state.js"));
  });

  beforeEach(() => {
    for (const role of ["primary", "aside", "nav", "games", "picker"]) {
      shell.setRegion(role, null);
    }
    shell.setMode("columns");
    el.consoles.innerHTML = "";
    el.list.innerHTML = "";
    state.platform = null;
    state.lastPlatform = null;
  });

  test("a console is opened without being clicked", async () => {
    await library.showPlatforms();
    assert.ok(state.platform, "no console was opened, so the middle is empty");
    assert.ok(el.consoles.innerHTML.length > 0, "the left column is empty");
  });

  /// The one you were last in, not merely the first alphabetically — coming
  /// back to where you were is the point of remembering it at all.
  test("it picks up where you left off", async () => {
    state.lastPlatform = "gb";
    await library.showPlatforms();
    assert.equal(state.platform, "gb");
  });

  /// The column stays on screen while the middle changes, so without a mark
  /// there is nothing to say which of thirty-five consoles you are looking at.
  test("the open console is marked in the column", async () => {
    await library.showPlatforms();
    const lit = el.consoles.querySelectorAll(".open");
    assert.equal(lit.length, 1, `${lit.length} consoles look open`);
    assert.equal(lit[0].dataset.slug, state.platform);
  });
});

describe("switching tabs in three columns", () => {
  /// Both reported together, and both the same cause: parking restores the
  /// screen a section was left on, which is right when a section *is* the
  /// screen. In three columns a tab owns two columns, so restoring only the
  /// screen left the other tab's list on the left — Library selected with
  /// collections beside it — and the previous tab's page still in the middle.
  let tabs, state, library;

  before(async () => {
    tabs = await import("../js/tabs.js");
    library = await import("../js/library.js");
    ({ state } = await import("../js/state.js"));
  });

  beforeEach(async () => {
    for (const role of ["primary", "aside", "nav", "games", "picker"]) {
      shell.setRegion(role, null);
    }
    shell.setMode("columns");
    el.consoles.innerHTML = "";
    el.list.innerHTML = "";
    state.platform = null;
    state.lastPlatform = null;
  });

  test("Library shows consoles again after a visit to My collections", async () => {
    await tabs.showSection("library", { force: true });
    assert.ok(
      el.consoles.querySelector("[data-slug]"),
      "the consoles are not in the column to begin with"
    );

    await tabs.showSection("mine");
    // The filter box moved to the tab row; the collections themselves are what
    // says the column changed hands.
    assert.ok(el.consoles.querySelector("#cgrid"), "the collections did not take the column");

    await tabs.showSection("library");
    assert.ok(
      el.consoles.querySelector("[data-slug]"),
      "Library was selected and the collections were still on the left"
    );
    assert.equal(el.consoles.querySelector(".filter"), null);
  });

  /// A page in the middle with a list of collections beside it is two tabs at
  /// once.
  test("the middle does not keep the last tab's contents", async () => {
    await tabs.showSection("history", { force: true });
    const wasHistory = el.list.innerHTML;
    assert.ok(wasHistory.length > 0, "History drew nothing to begin with");

    await tabs.showSection("browse");
    assert.notEqual(el.list.innerHTML, wasHistory, "the History page is still in the middle");
    // Browse now opens a group and then a collection, so the middle holds
    // that collection's games rather than a prompt — either way it is no
    // longer History's page.
    assert.doesNotMatch(el.list.innerHTML, /Nothing recorded yet|hist-/);
  });
});

describe("Browse and My collections fill the middle too", () => {
  let collections, state;

  before(async () => {
    collections = await import("../js/collections.js");
    ({ state } = await import("../js/state.js"));
  });

  beforeEach(() => {
    for (const role of ["primary", "aside", "nav", "games", "picker"]) {
      shell.setRegion(role, null);
    }
    shell.setMode("columns");
    el.consoles.innerHTML = "";
    el.list.innerHTML = "";
    state.lastCollection = null;
    state.lastGroup = null;
  });

  /// The handlers were attached to cards in the middle while the groups were
  /// drawn in the left column, so clicking one did nothing whatsoever.
  test("clicking a group in the column does something", async () => {
    await collections.showCollectionGroups({ exclude: [] });
    const card = el.consoles.querySelector(".card[data-group]");
    assert.ok(card, "no groups in the column");
    assert.ok(
      el.list.querySelector(".card"),
      "the group's collections did not fill the middle"
    );

    asked.length = 0;
    card.click();
    await new Promise((r) => setTimeout(r, 0));
    assert.ok(
      asked.includes("collections_in"),
      `the click did nothing — asked for: ${asked.join(", ") || "nothing"}`
    );
  });

  /// A tab that opens on "pick something on the left" has two thirds of the
  /// window doing nothing, which is the complaint Library already answered.
  test("My collections opens with a collection showing", async () => {
    await collections.showCollectionsIn("user", "My collections");
    assert.ok(state.lastCollection, "nothing was opened");
    assert.equal(
      el.list.innerHTML.includes("Pick a collection"),
      false,
      "the middle is still a prompt"
    );
  });

  /// Coming back to a tab should put you where you were, which is what the
  /// section machinery did before three columns took its job away.
  test("it returns to the collection you were in", async () => {
    await collections.showCollectionsIn("user", "My collections");
    state.lastCollection = { id: "c2", name: "Second" };

    await collections.showCollectionsIn("user", "My collections");
    assert.equal(state.lastCollection.id, "c2", "it went back to the first one");
    const open = el.consoles.querySelector(".card.open");
    assert.equal(open?.dataset.cid, "c2", "the column does not show which is open");
  });
});

describe("the left column can be resized", () => {
  /// 240px is a guess about somebody else's console names, and "Arcade Shmups
  /// Horizontal" does not fit in it.
  beforeEach(() => {
    shell.setMode("columns");
    document.getElementById("consoles-grip")?.remove();
    delete el.consoles.dataset.resizable;
    el.consoles.style.flexBasis = "";
    localStorage.removeItem("consolesWidth");
  });

  test("there is a handle beside it, and only in three columns", () => {
    shell.installColumnResizer();
    const grip = document.getElementById("consoles-grip");
    assert.ok(grip, "no handle to drag");
    assert.equal(
      grip.previousElementSibling,
      el.consoles,
      "the handle is not against the column"
    );
  });

  test("a remembered width is applied", () => {
    localStorage.setItem("consolesWidth", "330");
    shell.installColumnResizer();
    assert.equal(el.consoles.style.flexBasis, "330px");
  });

  /// Dragged to nothing it would be a column you cannot get back; dragged wide
  /// it would eat the games.
  test("it cannot be dragged away or over the games", () => {
    localStorage.setItem("consolesWidth", "10");
    shell.installColumnResizer();
    assert.equal(el.consoles.style.flexBasis, "160px", "the column can vanish");

    delete el.consoles.dataset.resizable;
    document.getElementById("consoles-grip")?.remove();
    localStorage.setItem("consolesWidth", "5000");
    shell.installColumnResizer();
    assert.equal(el.consoles.style.flexBasis, "520px", "the column can take the window");
  });

  test("installing twice does not leave two handles", () => {
    shell.installColumnResizer();
    shell.installColumnResizer();
    assert.equal(document.querySelectorAll("#consoles-grip").length, 1);
  });
});

describe("the order of the tabs", () => {
  let tabs;
  before(async () => (tabs = await import("../js/tabs.js")));

  /// Browse is the server's own groupings — 1,040 companies, every genre it
  /// knows — and it sat second from the left, in front of History and level
  /// with the collections you made yourself. It is the one you reach for
  /// least, so it goes to the end.
  test("RomM browse is last, and says whose collections it is", () => {
    const ids = tabs.SECTIONS.map((s) => s.id);
    assert.equal(ids.at(-1), "browse", "Browse is not the last tab");
    assert.deepEqual(ids, ["library", "mine", "history", "browse"]);
    assert.equal(
      tabs.SECTIONS.find((s) => s.id === "browse").label,
      "RomM browse",
      "the tab still calls itself Browse, which every tab here does"
    );
  });
});

describe("the layout switch in the header", () => {
  /// It was a dropdown in Settings, two windows away from the thing it
  /// changes — and it is a thing you flip back and forth to decide, not a
  /// setting. One pane is the default again: three columns was the default
  /// while it was being built, which is not a reason for anyone else to open
  /// the app in it.
  let shell, el;

  before(async () => {
    shell = await import("../js/shell.js");
    ({ el } = await import("../js/state.js"));
  });

  beforeEach(() => localStorage.removeItem("moose.shell"));

  test("one pane unless something says otherwise", () => {
    assert.equal(shell.storedMode(), "single");
    localStorage.setItem("moose.shell", "columns");
    assert.equal(shell.storedMode(), "columns", "a stored choice is ignored");
  });

  test("there are two buttons, one per arrangement", () => {
    const modes = [...(el.viewSwitch?.querySelectorAll("[data-mode]") ?? [])].map(
      (b) => b.dataset.mode
    );
    assert.deepEqual(modes, ["single", "columns"]);
  });

  /// A pair rather than a toggle, so the lit one has to be the arrangement you
  /// are actually in — whichever way the change arrived: this button, the
  /// dropdown in Settings, or the stored value at startup.
  test("the current arrangement is the lit one", () => {
    shell.setMode("columns");
    const lit = () =>
      [...el.viewSwitch.querySelectorAll("[data-mode]")]
        .filter((b) => b.classList.contains("on"))
        .map((b) => b.dataset.mode);
    assert.deepEqual(lit(), ["columns"]);
    shell.setMode("single");
    assert.deepEqual(lit(), ["single"]);
  });
});

describe("collections show their artwork", () => {
  /// Every collection in the app drew the two-letter placeholder, in both
  /// arrangements, and nothing ever said why: the loader looked up its
  /// container as `region(into)` where `into` is a parameter of the function
  /// that *calls* it. A ReferenceError on the first card, every time, caught
  /// by a `catch` meant for a failed cover lookup and thrown away.
  let collections, shell;

  before(async () => {
    collections = await import("../js/collections.js");
    shell = await import("../js/shell.js");
  });

  for (const mode of ["single", "columns"]) {
    test(`a cover reaches the card in ${mode}`, async () => {
      shell.setMode(mode);
      await collections.showCollectionsIn("user", "My collections");
      // The covers arrive a round trip after the cards are drawn.
      await new Promise((r) => setTimeout(r, 20));
      const card = document
        .querySelector('.card[data-cid="c1"] .logo img');
      assert.ok(card, "the card still shows the two-letter placeholder");
      assert.match(card.getAttribute("src"), /11\.png/);
    });
  }
});

describe("where the preview toggle lives", () => {
  /// In the header it was one of nine buttons among settings, search, sort and
  /// take-offline — none of which have anything to do with it — while the
  /// thing it opens and closes is a third of the window. It sits at the end of
  /// the tab row instead, beside what it acts on.
  let tabs, el;

  before(async () => {
    tabs = await import("../js/tabs.js");
    ({ el } = await import("../js/state.js"));
  });

  /// In a holder, which is what keeps them at the end of the row. The push
  /// right used to be a margin on Take offline — and that button is hidden on
  /// every screen with nothing to take offline, so the margin went with it and
  /// the preview toggle slid back up against the tabs.
  test("both are in a holder at the end, in that order", () => {
    tabs.installTabs();
    const end = el.tabbar.querySelector(".tabbar-end");
    assert.ok(end, "nothing holds them at the end");
    assert.equal(el.tabbar.lastElementChild, end, "something else is after them");
    assert.deepEqual(
      [...end.children].map((c) => c.id),
      ["grab-btn", "sidebar-btn"]
    );

    // And the holder stays put when the first button is not on screen.
    el.grabBtn.hidden = true;
    assert.equal(el.tabbar.lastElementChild, end);
    assert.equal(el.sidebarBtn.parentElement, end, "the toggle left the holder");
    el.grabBtn.hidden = false;
  });

  /// Moved rather than copied: the same element, so everything that hides it,
  /// relabels it or reads its state goes on working.
  test("there is only one of it", () => {
    tabs.installTabs();
    assert.equal(document.querySelectorAll("#sidebar-btn").length, 1, "the header kept a copy");
    assert.equal(document.querySelectorAll("#grab-btn").length, 1, "the header kept a copy");
  });

  /// Rebuilding the row must not throw it away — the tabs are redrawn whenever
  /// the sections change, and a control that vanishes on a redraw is worse
  /// than one in the wrong place.
  test("rebuilding the row keeps it", () => {
    tabs.installTabs();
    tabs.installTabs();
    assert.equal(el.tabbar.querySelectorAll(".tabbar-end").length, 1, "two holders");
    for (const btn of [el.sidebarBtn, el.grabBtn]) {
      assert.ok(btn.parentElement?.classList.contains("tabbar-end"));
      assert.equal(document.querySelectorAll(`#${btn.id}`).length, 1);
    }
  });
});

describe("bringing the preview back", () => {
  /// The pane is remembered as closed, which is right — and in Desk it left no
  /// way back. "Show info" on the console screen did nothing at all: the pane
  /// insisted on a selected game before it would appear, while the console
  /// screen's own line said the column should be there regardless. Whichever
  /// ran last won, and from a cold start with the preference off, neither
  /// showed it.
  let detail, shell, state, el;

  before(async () => {
    detail = await import("../js/detail.js");
    shell = await import("../js/shell.js");
    ({ state, el } = await import("../js/state.js"));
  });

  test("Show info opens the column even with nothing selected", () => {
    shell.setMode("columns");
    state.view = "platforms";
    state.selected = null;
    detail.setSidebar(false);
    assert.equal(el.detail.hidden, true);

    detail.setSidebar(true);
    assert.equal(el.detail.hidden, false, "the column will not come back");
  });

  /// The console screen shows the console under the cursor now, so the pane is
  /// meaningful there too — it was not, which is why pressing Show info on the
  /// Library screen did nothing at all.
  test("in Sofa it opens on the console screen as well", () => {
    shell.setMode("single");
    state.view = "platforms";
    state.selected = null;
    detail.setSidebar(true);
    assert.equal(el.detail.hidden, false, "nothing to show on the Library screen");

    state.view = "roms";
    state.selected = 7;
    detail.setSidebar(true);
    assert.equal(el.detail.hidden, false);
  });

  /// A game list with nothing selected is still nothing to preview: there it
  /// slides over the list rather than being part of it.
  test("and still waits when there is genuinely nothing", () => {
    shell.setMode("single");
    state.view = "roms";
    state.selected = null;
    detail.setSidebar(true);
    assert.equal(el.detail.hidden, true);
  });
});

describe("a console in the preview pane", () => {
  /// The Library screen had a preview that was always empty: nothing is
  /// selected there, so the pane stayed hidden and the button that opens it
  /// did nothing you could see — reported three times as the pane being gone.
  /// A console has facts of its own, and all of them lived in Settings.
  let detail, shell, state, el, asked;

  before(async () => {
    detail = await import("../js/detail.js");
    shell = await import("../js/shell.js");
    ({ state, el } = await import("../js/state.js"));
  });

  beforeEach(() => {
    state.platforms = [
      { slug: "arcade", name: "Arcade", rom_count: 2506, playable: true, logo: null },
      { slug: "3do", name: "3DO Interactive Multiplayer", rom_count: 12, playable: false, logo: null },
    ];
    state.view = "platforms";
    detail.setSidebar(true);
  });

  test("it shows the console's name and how many games it has", async () => {
    await detail.showPlatformInfo("arcade");
    assert.equal(el.detail.hidden, false, "the pane stayed shut");
    assert.match(el.detail.textContent, /Arcade/);
    assert.match(el.detail.textContent, /2506/);
  });

  /// The one fact that decides whether anything here will start, and it was
  /// only visible as a colored dot in the list — or in Settings, two windows
  /// away.
  test("a console with no emulator says so, in words", async () => {
    await detail.showPlatformInfo("3do");
    assert.match(el.detail.textContent, /none installed/i);
    assert.ok(el.detail.querySelector(".warn"), "no warning about a console that cannot start");
  });

  test("and one that can start does not cry wolf", async () => {
    await detail.showPlatformInfo("arcade");
    assert.equal(el.detail.querySelector(".warn"), null);
  });

  test("an unknown console leaves the pane alone", async () => {
    await detail.showPlatformInfo("arcade");
    const before = el.detail.innerHTML;
    await detail.showPlatformInfo("nothing-like-this");
    assert.equal(el.detail.innerHTML, before);
  });

  /// Take offline is on the pane as well as in the tab row: the pane is where
  /// you are looking when you decide a console is worth having on the plane.
  test("it offers to take the console offline", async () => {
    await detail.showPlatformInfo("arcade");
    assert.ok(el.detail.querySelector(".pf-grab"), "no way to download it from here");
  });
});

// The bindings table.
//
// Two stacked lists — every action once for the keyboard, then every action
// again for the controller — meant scrolling past twenty rows to compare a key
// with its button, and reading each action's name twice to do it. One row per
// action makes the comparison the layout. A regression to two lists has to fail
// here rather than be noticed by eye.

import { test, describe, before } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { JSDOM } from "jsdom";
import { fakeBackend } from "./backend.js";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");

let paneHtml, doc;

before(async () => {
  // settings-panes.js reaches state.js, which reads localStorage at import
  // time, so the page has to exist before the module does.
  const dom = new JSDOM(readFileSync(join(uiDir, "settings.html"), "utf8"), {
    url: "http://localhost/",
  });
  const backend = fakeBackend();
  dom.window.__TAURI__ = {
    core: { invoke: backend, convertFileSrc: (p) => p },
    event: { listen: () => Promise.resolve(() => {}), emit: () => {} },
  };
  for (const k of ["window", "document", "navigator", "localStorage", "CSS"]) {
    Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true });
  }
  ({ paneHtml } = await import(join(uiDir, "js", "settings-panes.js")));
  // The bindings table is drawn from what the backend says is bound, so the
  // pane has nothing to draw until they arrive.
  await (await import(join(uiDir, "js", "bindings.js"))).loadBindings();
  doc = new JSDOM(`<body>${paneHtml("control")}</body>`).window.document;
});

describe("keyboard and controller side by side", () => {
  test("every action is one row carrying both bindings", () => {
    const rows = [...doc.querySelectorAll("tr[data-id]")];
    assert.ok(rows.length > 10, `expected a row per action, got ${rows.length}`);
    for (const row of rows) {
      assert.ok(row.querySelector(".key-cell .set-key"), `${row.dataset.id}: no key button`);
      assert.ok(row.querySelector(".pad-cell .set-pad"), `${row.dataset.id}: no pad button`);
    }
  });

  test("the columns say what they are", () => {
    const heads = [...doc.querySelectorAll("th")].map((h) => h.textContent.trim());
    assert.deepEqual(heads, ["Action", "Keyboard", "Controller"]);
  });

  /// Both buttons now live in one row, so anything scoped to the row rather
  /// than the cell would hand the keyboard handler the controller's button and
  /// silently rebind the wrong one.
  test("the two buttons in a row are distinguishable by cell", () => {
    const row = doc.querySelector("tr[data-id]");
    const key = row.querySelector(".key-cell .set-key");
    const pad = row.querySelector(".pad-cell .set-pad");
    assert.ok(key && pad);
    assert.notEqual(key, pad);
    assert.equal(row.querySelectorAll(".set-key").length, 1, "one key button per row");
    assert.equal(row.querySelectorAll(".set-pad").length, 1, "one pad button per row");
  });

  /// Each action appears once. The old layout listed all of them twice, which
  /// is the thing being fixed.
  test("no action is listed twice", () => {
    const ids = [...doc.querySelectorAll("tr[data-id]")].map((r) => r.dataset.id);
    assert.equal(new Set(ids).size, ids.length, `duplicated rows: ${ids}`);
  });
});

describe("the settings tabs", () => {
  /// The BIOS control sat at the bottom of General under six unrelated
  /// headings — RetroArch, saves, server, achievements, ScreenScraper — and was
  /// simply not found. The things that go and fetch something now have their
  /// own tab.
  test("fetching things has its own tab, and it holds the BIOS control", async () => {
    const { TABS, paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    assert.ok(TABS.some((t) => t.id === "library"), "no library tab");

    const lib = new JSDOM(`<body>${paneHtml("library")}</body>`).window.document;
    assert.ok(lib.querySelector(".set-bios"), "BIOS control is not on the library tab");
    assert.ok(lib.querySelector(".set-bios-bar"), "and it needs a progress bar");
    assert.ok(lib.querySelector(".set-libsync"), "library sync belongs here too");
    assert.ok(lib.querySelector(".set-scrape"), "so does missing artwork");

    // And it is no longer buried in General.
    const gen = new JSDOM(`<body>${paneHtml("general")}</body>`).window.document;
    assert.equal(gen.querySelector(".set-bios"), null, "still duplicated in General");
  });

  /// "Systems" was a button in the top bar between Grid and Take offline —
  /// things you do to what is on screen — so it read as another view of the
  /// library rather than as configuration, and gave no clue what pressing it
  /// would do.
  test("per-system emulator and shader settings are a settings tab", async () => {
    const { TABS, paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    assert.ok(TABS.some((t) => t.id === "systems"), "no systems tab");
    const pane = new JSDOM(`<body>${paneHtml("systems")}</body>`).window.document;
    assert.ok(pane.querySelector(".sys-table"), "nowhere for the table to go");
  });

  /// The themes panel is gone: the app never rendered a theme, it only read
  /// the per-system pictures out of one. What is left is the picker for those
  /// pictures, which is the only part that changed what you see.
  test("console pictures are chosen in Appearance, with no theme gallery", async () => {
    const { paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    const pane = new JSDOM(`<body>${paneHtml("appearance")}</body>`).window.document;
    assert.ok(pane.querySelector(".icon-styles"), "no way to pick a picture style");
    assert.ok(pane.querySelector(".set-icons"), "no way to fetch any");
  });

  /// The tab was called "Systems", which said nothing: every tab in there is
  /// about systems of one kind or another, and the word gave no clue that this
  /// is where you choose which emulator runs a console.
  test("the emulator tab is named for what you change in it", async () => {
    const { TABS } = await import(join(uiDir, "js", "settings-panes.js"));
    const tab = TABS.find((t) => t.id === "systems");
    assert.ok(tab, "the tab is gone entirely");
    assert.notEqual(tab.label, "Systems", "still named after nothing in particular");
    assert.match(tab.label, /Emulator/i);
  });

  /// Four ports, and a way to say the other three pads are like the first.
  test("the control tab covers players beyond the first", async () => {
    const { paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    const pane = new JSDOM(`<body>${paneHtml("control")}</body>`).window.document;
    assert.ok(pane.querySelector(".pad-list"), "no list of connected controllers");
    assert.ok(
      pane.querySelector('[data-field="mirror_player_one"]'),
      "no way to bind players 2-4 like player 1"
    );
  });

  /// Two dropdowns whose only sensible settings were a matching pair. One
  /// palette now drives the glass and the shader backdrop together.
  test("one color scheme drives the glass and the backdrop", async () => {
    const { paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    const { SCHEMES } = await import(join(uiDir, "js", "backdrop.js"));
    const pane = new JSDOM(`<body>${paneHtml("appearance")}</body>`).window.document;

    assert.ok(pane.querySelector(".scheme-preset"), "no single scheme control");
    assert.equal(pane.querySelector(".glass-preset"), null, "the glass dropdown is still there");
    assert.equal(pane.querySelector(".bd-preset"), null, "the backdrop dropdown is still there");

    // The things that are not a color survive the merge.
    assert.ok(pane.querySelector(".glass-strength"), "tint strength was lost");
    assert.ok(pane.querySelector(".bd-speed"), "motion was lost");
    assert.ok(pane.querySelector(".bd-strength"), "brightness was lost");

    // The first scheme is the fallback when nothing is stored, which is every
    // new install — so it has to carry a glass color rather than being a
    // placeholder. A missing one used to be a crash before the first paint.
    assert.match(SCHEMES[0].glass ?? "", /^#[0-9a-f]{6}$/i, "the default scheme has no glass");

    // Every scheme carries all three colors, or picking it would leave one
    // surface on the last scheme's palette.
    for (const c of SCHEMES.filter((x) => x.id !== "custom")) {
      for (const k of ["glass", "low", "high"]) {
        assert.match(c[k] ?? "", /^#[0-9a-f]{6}$/i, `${c.id} has no ${k}`);
      }
    }
    // And custom leaves all three to the user.
    const custom = SCHEMES.find((x) => x.id === "custom");
    assert.ok(custom, "no way to set an unmatched pair");
    assert.equal(custom.glass, null);
  });

  /// Which screen a game opens on. The control is in the markup but starts
  /// hidden: with one display attached it is a question with one answer.
  test("the emulator tab can choose a screen for games", async () => {
    const { paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    const pane = new JSDOM(`<body>${paneHtml("systems")}</body>`).window.document;
    const row = pane.querySelector(".sys-screen");
    assert.ok(row, "no way to choose which screen a game opens on");
    assert.ok(pane.querySelector("select.game-display"), "no picker in the row");
    assert.equal(row.hidden, true, "it should stay hidden until there are two screens");
  });

  /// The black bars around a game are the window being the wrong shape, and
  /// the title bar is the other thing people mean by "border". Both are
  /// toggles rather than one, because they are different complaints.
  test("the game window has both of its borders as settings", async () => {
    const { paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    const pane = new JSDOM(`<body>${paneHtml("systems")}</body>`).window.document;
    assert.ok(pane.querySelector('[data-field="fit_window"]'), "no fit-to-game toggle");
    assert.ok(pane.querySelector('[data-field="window_decorations"]'), "no title-bar toggle");
  });

  /// Every tab the window offers has to render something. A tab whose id has
  /// no markup opens onto a blank pane, which is indistinguishable from a
  /// window that has broken.
  test("every tab renders a pane", async () => {
    const { TABS, paneHtml } = await import(join(uiDir, "js", "settings-panes.js"));
    for (const t of TABS) {
      assert.ok(
        (paneHtml(t.id) ?? "").trim().length > 0,
        `the ${t.id} tab renders nothing`
      );
    }
  });
});

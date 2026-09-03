// Do the settings panes' classes have any styling at all?
//
// The settings window is its own document — settings.html, `#pane`, styled by
// settings.css. The library window is index.html, styled by style.css. Writing
// a pane's rules into the wrong one fails *silently*: the markup renders, the
// behavior works, every test passes, and the elements simply come out with
// the browser's default look. That is how the Icon sets tab shipped with its
// screenshots at their natural 1280px width — the rules were in style.css
// under `#settings`, an id that exists in neither document.
//
// jsdom has no layout and no cascade, so nothing here can check what a rule
// *does*. What it can check is that a rule exists at all, in the file the
// window loads, which is the whole of the mistake being guarded against.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const uiDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (f) => readFileSync(join(uiDir, f), "utf8");

/// Every module, including the panes in their own directory.
function jsFiles() {
  const out = [];
  for (const f of readdirSync(join(uiDir, "js"))) {
    if (f.endsWith(".js")) out.push(join("js", f));
  }
  for (const f of readdirSync(join(uiDir, "js/settings"))) {
    if (f.endsWith(".js")) out.push(join("js/settings", f));
  }
  return out;
}

const settingsCss = read("settings.css");
const styleCss = read("style.css");
const settingsHtml = read("settings.html");
const indexHtml = read("index.html");

/// Class names appearing in a pane's markup, ignoring the ones that are only
/// there for JavaScript to find an element by.
function classesIn(source) {
  const out = new Set();
  for (const m of source.matchAll(/class="([^"$]+)"/g)) {
    for (const c of m[1].split(/\s+/)) {
      if (c && !c.includes("$")) out.add(c);
    }
  }
  return out;
}

describe("settings pane styling lives in the file the window loads", () => {
  /// Every layout class the Icon sets tab uses has to be styled in settings.css.
  /// `ic-get`, `ic-apply` and `ic-remove` are excluded: they are handles for the
  /// click listener, and the buttons take their look from `#pane button`.
  test("the icon sets tab is styled where the settings window can see it", () => {
    const hooks = new Set(["ic-get", "ic-apply", "ic-remove", "ic-active", "hint", "dim", "srow", "ctl"]);
    const used = [...classesIn(read("js/settings/iconsets.js"))].filter(
      (c) => c.startsWith("ic-") && !hooks.has(c),
    );
    assert.ok(used.length >= 5, `expected the tab to use several classes, saw ${used.length}`);
    for (const c of used) {
      assert.ok(
        settingsCss.includes(`.${c}`),
        `.${c} is used by the Icon sets tab but has no rule in settings.css` +
          (styleCss.includes(`.${c}`) ? " — it is in style.css, which that window does not load" : ""),
      );
    }
  });

  /// A rule scoped to an id that exists in no document is dead the moment it is
  /// written, and reads as working code until someone looks at the screen.
  ///
  /// An id counts as real if either HTML file declares it, or some module
  /// *builds* an element with it — `id="x"` inside a template string, or
  /// `.id = "x"`. Deliberately not `getElementById("x")`: that is a read, and
  /// a read of something nothing creates is exactly the dead branch that let
  /// `#settings` keep 18 rules after Settings moved to its own window.
  test("no stylesheet targets an id that nothing creates", () => {
    const ids = new Set();
    for (const html of [settingsHtml, indexHtml]) {
      for (const m of html.matchAll(/\bid="([^"]+)"/g)) ids.add(m[1]);
    }
    for (const f of jsFiles()) {
      const src = read(f);
      for (const m of src.matchAll(/\bid=\\?"([\w-]+)\\?"/g)) ids.add(m[1]);
      for (const m of src.matchAll(/\.id = "([\w-]+)"/g)) ids.add(m[1]);
    }

    const dead = new Set();
    for (const [file, css] of [
      ["style.css", styleCss],
      ["settings.css", settingsCss],
    ]) {
      // Every id in selector position, not only the ones starting a line.
      // Checking line starts alone hid `#themes-btn` for as long as it sat
      // second in a comma list, and it surfaced only when the dead selector
      // ahead of it was deleted.
      for (const rule of css.split("}")) {
        const sel = rule.split("{")[0].replace(/\/\*[\s\S]*?\*\//g, "");
        for (const m of sel.matchAll(/#([A-Za-z][\w-]*)/g)) {
          if (!ids.has(m[1])) dead.add(`${file} #${m[1]}`);
        }
      }
    }
    assert.deepEqual(
      [...dead],
      [],
      `these selectors match nothing in any document: ${[...dead].join(", ")}`,
    );
  });
});

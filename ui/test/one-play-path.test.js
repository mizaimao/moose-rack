// A double-click plays a game by pressing Play.
//
// It used to call `play(d)` itself, with its own freshly fetched detail. Both
// paths "called the same function" and behaved differently anyway — different
// `d`, different timing, a detail pane in a different state — and finding out
// why cost a working day. Two callers of one function is still two paths.
//
// So there is one path now: open the pane, press the button. Whatever the
// button does, a double-click does, including anything added to it later.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../js/library.js", import.meta.url), "utf8");

/// Code, not comments — the comment explaining the fix names what it replaced.
const code = src
  .split("\n")
  .filter((l) => !l.trim().startsWith("//") && !l.trim().startsWith("///"))
  .join("\n");

describe("double-click and the Play button", () => {
  test("a double-click presses the button rather than launching on its own", () => {
    assert.match(code, /pressPlay/, "the shared path is gone");
    // The two halves separately: a line between them is fine, and a window of
    // n characters is a test that breaks on a comment.
    const body = code.slice(code.indexOf("async function pressPlay"));
    const end = body.indexOf("\n}");
    const fn = body.slice(0, end);
    assert.match(fn, /getElementById\("play"\)/, "pressPlay no longer finds the button");
    assert.match(fn, /\.click\(\)/, "pressPlay no longer presses it");
  });

  /// The specific regression: a second launch path that fetches its own detail
  /// and calls `play` behind the pane's back.
  test("no dblclick handler calls play() with its own rom_detail", () => {
    const handlers = code.split('addEventListener("dblclick"');
    // The first chunk is everything before the first handler.
    for (const h of handlers.slice(1)) {
      const body = h.slice(0, h.indexOf("});"));
      assert.ok(
        !/play\(await invoke\("rom_detail"/.test(body),
        `a dblclick handler launches on its own again:\n${body.trim().slice(0, 200)}`,
      );
    }
  });

  /// A double-click sends two clicks first, and each used to re-render the
  /// detail pane -- replacing the very button the double-click was about to
  /// press, three view transitions deep.
  test("the second click of a double-click is not a selection", () => {
    assert.match(code, /ev\.detail > 1/, "the click handler still selects twice");
  });

  /// Continue playing is the one deliberate exception: it resumes from a save
  /// state rather than starting over, and the button cannot express that.
  test("Continue playing still resumes rather than restarting", () => {
    assert.match(code, /if \(resume\) return launch\(id, \{ resume: true \}\)/);
  });
});

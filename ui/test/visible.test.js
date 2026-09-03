// Which rows a window draws, and which it leaves to the spacers.
//
// The arithmetic only, with no page anywhere near it. This is the part that
// can be wrong in ways nobody sees: a band one row short is a list with a hole
// in it at exactly one scroll position, and a spacer half a row out is a grid
// that jumps as you scroll past it.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { slice, worthWindowing, THRESHOLD } from "../js/visible.js";

/// 2,506 arcade games, ten across, 200px a row, in an 800px window.
const arcade = (top, over = {}) =>
  slice({ total: 2506, columns: 10, rowHeight: 200, top, viewport: 800, overscan: 1.5, ...over });

/// Where the band sits, as whole rows.
const band = (s) => ({
  firstRow: s.first / 10,
  rows: Math.ceil(s.count / 10),
  before: s.before,
  after: s.after,
});

describe("the band that gets drawn", () => {
  test("at the top, it starts at the first row", () => {
    const s = arcade(0);
    assert.equal(s.first, 0);
    assert.equal(s.before, 0, "there is nothing above the first row to leave space for");
    assert.ok(s.count > 0);
  });

  /// The whole point: a fraction of the list, not all of it.
  test("it is a fraction of the list, not the list", () => {
    const s = arcade(20_000);
    assert.ok(s.count < 2506 / 4, `drew ${s.count} of 2,506`);
    assert.ok(s.count > 10, `drew only ${s.count}`);
  });

  /// Overscan is a guarantee rather than a tuning knob: the cursor's next stop
  /// has to be drawn already, whichever way it goes and however fast a held
  /// key repeats.
  test("a whole screen is drawn beyond the viewport in each direction", () => {
    const s = arcade(20_000);
    assert.ok(s.before <= 20_000 - 800, `only ${20_000 - s.before}px of overscan above`);
    const bottom = s.before + Math.ceil(s.count / 10) * 200;
    assert.ok(bottom >= 20_000 + 800 + 800, `only ${bottom - 20_800}px of overscan below`);
  });

  test("at the end, it stops at the last row", () => {
    const rows = Math.ceil(2506 / 10);
    const s = arcade(rows * 200);
    assert.equal(s.after, 0, "space was left below the last row");
    assert.equal(s.first + s.count, 2506, "the last row was not drawn");
  });

  /// A last row that is not full is still a row.
  test("a short last row is drawn whole", () => {
    const s = slice({ total: 2506, columns: 10, rowHeight: 200, top: 50_000, viewport: 800 });
    assert.equal(s.first + s.count, 2506);
    assert.equal(2506 % 10, 6, "this test is about the six left over");
  });
});

describe("the spacers", () => {
  /// The scrollbar, and every remembered scroll position, have to be what they
  /// would have been with every row drawn.
  test("together with the band they are the full height, at any position", () => {
    const rowHeight = 200;
    const total = Math.ceil(2506 / 10) * rowHeight;
    for (const top of [0, 137, 4_000, 20_000, 42_000, 60_000]) {
      const s = arcade(top);
      const drawn = Math.ceil(s.count / 10) * rowHeight;
      assert.equal(s.before + drawn + s.after, total, `wrong total height at ${top}`);
    }
  });

  /// Half a row of error is a grid that jumps by half a row as you scroll.
  test("they are always a whole number of rows", () => {
    for (const top of [0, 137, 4_321, 20_000, 59_999]) {
      const s = arcade(top);
      assert.equal(s.before % 200, 0, `before is not whole rows at ${top}`);
      assert.equal(s.after % 200, 0, `after is not whole rows at ${top}`);
    }
  });

  test("the band always starts on a row boundary", () => {
    for (const top of [0, 137, 4_321, 20_000, 59_999]) {
      assert.equal(arcade(top).first % 10, 0, `the band starts mid-row at ${top}`);
    }
  });
});

describe("the edges", () => {
  /// The list sitting below the top of the viewport — a strip of recently
  /// played above it, or the page scrolled up past it.
  test("a list not yet reached draws from its first row", () => {
    const s = arcade(-500);
    assert.equal(s.first, 0);
    assert.equal(s.before, 0);
  });

  test("an empty list draws nothing and asks for no space", () => {
    const s = slice({ total: 0, columns: 10, rowHeight: 200, top: 0, viewport: 800 });
    assert.equal(s.count, 0);
    assert.equal(s.before, 0);
    assert.equal(s.after, 0);
  });

  /// Before anything has been drawn there is no height to measure, and a
  /// window that draws nothing at that moment never gets a first card to
  /// measure from.
  test("with no row height yet, everything is drawn", () => {
    const s = slice({ total: 900, columns: 10, rowHeight: 0, top: 0, viewport: 800 });
    assert.equal(s.count, 900);
  });

  test("one column is a list, and behaves", () => {
    const s = slice({ total: 2506, columns: 1, rowHeight: 34, top: 10_000, viewport: 800 });
    assert.ok(s.count > 0 && s.count < 2506);
    assert.equal(s.before % 34, 0);
  });

  /// Nothing sane comes back from a zero or fractional column count, and both
  /// are reachable: the grid is measured off the browser, and a container that
  /// is not laid out yet reports neither.
  test("a nonsense column count does not divide by zero", () => {
    for (const columns of [0, -1, NaN, undefined]) {
      const s = slice({ total: 500, columns, rowHeight: 40, top: 0, viewport: 800 });
      assert.ok(Number.isFinite(s.count) && s.count > 0, `columns=${columns} drew ${s.count}`);
      assert.ok(Number.isFinite(s.before), `columns=${columns} gave a bad spacer`);
    }
  });
});

describe("when it is worth doing at all", () => {
  /// A window over a short list is machinery with nothing to do, and the
  /// threshold is well above a screenful at any zoom — so nothing anybody can
  /// see at once is ever windowed.
  test("short lists are drawn whole", () => {
    assert.equal(worthWindowing(35), false, "the console grid");
    assert.equal(worthWindowing(200), false, "a full page of search results");
    assert.equal(worthWindowing(THRESHOLD), false);
    assert.equal(worthWindowing(2506), true, "the arcade console");
  });
});

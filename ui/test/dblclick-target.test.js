// Which game a double-click meant.
//
// `dblclick` is dispatched on the nearest common ancestor of its two clicks,
// and the first click redraws the card it landed on — the selection class, the
// star, the cover arriving — so the second lands on a different node and the
// ancestor is the grid, which carries no `data-id`.
//
// Reading only the event target meant the handler returned without a word: no
// launch, no error, nothing in the console. That is the whole of "double-click
// does nothing", and it took a day and a screenshot of an empty console.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { doubleClickTarget } from "../js/dblclick-target.js";

describe("what a double-click meant", () => {
  test("the card it landed on, when it landed on one", () => {
    assert.equal(doubleClickTarget(7, 3), 7);
    // Ids are negative for anything found on this machine, and 0 is a real id
    // nowhere — but neither may be mistaken for "nothing".
    assert.equal(doubleClickTarget(-10793, null), -10793);
    assert.equal(doubleClickTarget(0, 9), 0);
  });

  /// The case that was broken, and the common one.
  test("the selected game, when it landed on the grid instead", () => {
    assert.equal(doubleClickTarget(null, 7), 7, "a double-click that missed did nothing");
    assert.equal(doubleClickTarget(undefined, -10793), -10793);
  });

  test("nothing, when nothing was hit and nothing is selected", () => {
    assert.equal(doubleClickTarget(null, null), null);
    assert.equal(doubleClickTarget(null, undefined), null);
    assert.equal(doubleClickTarget(undefined, undefined), null);
  });
});

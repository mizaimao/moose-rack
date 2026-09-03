// The right-click menu, and the press that chooses from it.
//
// Every menu in this app did nothing when you picked something out of it. Not
// the orders in the left column, not Delete on a save state — they opened,
// they looked right, and choosing an item was ignored. That is why the save
// state menu was reported four times as "not right": each fix made the menu
// appear correctly and none of them made it *work*.
//
// The cause is one line. The menu closed on the next `pointerdown` anywhere,
// and pressing a menu item is a pointerdown: the menu came off the page
// between the press and the release, so no click ever landed on the button
// that was pressed.

import { test, describe, before, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

let dom, menu;

before(async () => {
  dom = new JSDOM(`<body></body>`, { url: "http://localhost/", pretendToBeVisual: true });
  global.window = dom.window;
  global.document = dom.window.document;
  menu = await import("../js/menu.js");
});

beforeEach(() => menu.closeMenu());

/// The sequence a real press makes, in order. `.click()` on its own skips
/// straight to the end and hides exactly this bug.
///
/// The check in the middle is the whole test. jsdom will happily deliver a
/// click to a node that has been taken out of the document, so asserting only
/// that the action ran would pass against the broken code — a browser will
/// not: remove the pressed element before the release and the click is
/// retargeted to whatever is left, which is how the action was being lost.
function press(node) {
  node.dispatchEvent(new dom.window.MouseEvent("pointerdown", { bubbles: true }));
  const survived = node.isConnected;
  for (const type of ["pointerup", "click"]) {
    node.dispatchEvent(new dom.window.MouseEvent(type, { bubbles: true }));
  }
  return survived;
}

const settle = () => new Promise((r) => setTimeout(r, 5));

describe("choosing from a menu", () => {
  test("an item pressed the way a mouse presses it runs", async () => {
    let ran = 0;
    menu.showMenu([{ label: "Delete", run: () => ran++ }], 10, 10);
    // The watchers hook up a frame later, as they must — the press that opened
    // the menu is still in flight.
    await settle();
    const survived = press(document.querySelector(".ctx-menu button"));
    assert.ok(
      survived,
      "the menu was taken off the page by the press itself, so no click can land on it"
    );
    assert.equal(ran, 1, "the item was pressed and nothing happened");
  });

  test("and the menu is gone afterwards", async () => {
    menu.showMenu([{ label: "Delete", run: () => {} }], 10, 10);
    await settle();
    press(document.querySelector(".ctx-menu button"));
    assert.equal(document.querySelector(".ctx-menu"), null, "the menu stayed open");
  });

  test("pressing anywhere else dismisses it", async () => {
    let ran = 0;
    menu.showMenu([{ label: "Delete", run: () => ran++ }], 10, 10);
    await settle();
    press(document.body);
    assert.equal(document.querySelector(".ctx-menu"), null, "the menu will not go away");
    assert.equal(ran, 0, "dismissing it ran the item anyway");
  });

  /// Nothing is left watching the window afterwards, whether the menu was
  /// chosen from or clicked away. Otherwise every menu ever opened leaves a
  /// listener behind, and the tenth one closes on a press meant for the
  /// eleventh.
  test("a menu leaves nothing behind on the window", async () => {
    const hooks = [];
    const add = dom.window.addEventListener.bind(dom.window);
    const remove = dom.window.removeEventListener.bind(dom.window);
    dom.window.addEventListener = (t, f, o) => (hooks.push([t, f]), add(t, f, o));
    dom.window.removeEventListener = (t, f, o) => {
      const at = hooks.findIndex(([ht, hf]) => ht === t && hf === f);
      if (at >= 0) hooks.splice(at, 1);
      return remove(t, f, o);
    };
    try {
      menu.showMenu([{ label: "Delete", run: () => {} }], 10, 10);
      await settle();
      press(document.querySelector(".ctx-menu button"));
      assert.deepEqual(hooks, [], `left ${hooks.map(([t]) => t).join(", ")} on the window`);
    } finally {
      dom.window.addEventListener = add;
      dom.window.removeEventListener = remove;
    }
  });

  /// A menu that says why there is nothing to do is not a broken menu; a menu
  /// whose only item is unusable should still say something.
  test("a disabled item does nothing and does not close it", async () => {
    let ran = 0;
    menu.showMenu([{ label: "No save states", disabled: true, run: () => ran++ }], 10, 10);
    await settle();
    const b = document.querySelector(".ctx-menu button");
    assert.equal(b.disabled, true);
    b.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
    assert.equal(ran, 0);
  });
});

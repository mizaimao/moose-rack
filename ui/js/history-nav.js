// The browser's Back button, wired to the app's own Back.
//
// This is a single page. Every screen -- a console, a collection, a game -- is
// a redraw of the same document, so the browser's history holds one entry for
// the whole app and Back leaves it. On the desktop that never came up. In a
// browser it lands on the login page, which reads as having been logged out
// rather than as having left, and it is a habit nobody is going to unlearn.
//
// The app already knows how to step back: `trail` is a stack of functions and
// both the header button and the pad's Back press pop one. So rather than
// mirroring that stack into `history` -- four places push to it, and two
// sources of truth that must agree is how drift starts -- this keeps exactly
// one spare entry in front of the app and spends it on each press.
//
// At the top of the app there is nothing to go back to, and Back means what it
// says: leave. That is the one case where the login page is the right answer.

let atTop = () => false;
let goBack = null;
/// Set while we put an entry back, so the `popstate` that causes is ignored.
let restoring = false;

export function installHistoryNav({ back, isTop }) {
  goBack = back;
  atTop = isTop ?? (() => false);
  if (typeof history === "undefined" || typeof globalThis.addEventListener !== "function") {
    return;
  }
  // The spare. Without it the very first Back is already leaving.
  history.pushState({ moose: 1 }, "");
  globalThis.addEventListener("popstate", handlePop);
}

export function handlePop() {
  if (restoring) {
    restoring = false;
    return;
  }
  // Nothing left to go back to inside the app: let it go. Pressing Back at the
  // top of the library should leave, and refusing would trap the tab.
  if (atTop()) return;

  goBack?.();
  // Spend one, put one back, so the next press has something to consume.
  restoring = true;
  history.pushState({ moose: 1 }, "");
  // `pushState` does not fire `popstate`, so nothing will clear the flag.
  // Cleared here rather than in the listener that never runs.
  restoring = false;
}

/// Test seam.
export function resetHistoryNav() {
  goBack = null;
  atTop = () => false;
  restoring = false;
}

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
// It never leaves. The only entry behind this app is the login page, and being
// returned to a login you are already through reads as having been logged out.
// There is nothing useful on the other side of Back, so Back stays here; at the
// top of the library it simply does nothing, which is what the header button
// does there too.
//
// The tab is not trapped by that: closing it and typing an address both still
// work, and neither is what somebody reaching for Back is trying to do.

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
  // At the top there is nothing to step back to, so this is a no-op -- but the
  // entry is still put back below, because letting it go would land on the
  // login page.
  if (!atTop()) goBack?.();
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

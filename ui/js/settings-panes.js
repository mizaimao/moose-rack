// The Settings window's tabs.
//
// A table and two lookups. Each tab lives in its own file under settings/ and
// exports the same two things — the markup, and a function to wire it up — so
// adding a tab is a file and a line here.
//
// This was one 1,225-line module holding five unrelated panes, their markup,
// their behavior, and the key-capture state, which meant every change to any
// setting was a change to the same file. The panes had nothing to do with each
// other; the file was the only thing they shared.

import * as general from "./settings/general.js";
import * as appearance from "./settings/appearance.js";
import * as control from "./settings/control.js";
import * as library from "./settings/library.js";
import * as emulators from "./settings/emulators.js";
import * as iconsets from "./settings/iconsets.js";
import * as about from "./settings/about.js";
import { stopPadCapture } from "./settings/control.js";

/// The tabs, in the order they appear.
///
/// Order is deliberate: the two people reach for constantly first, then the one
/// that fetches things, then the per-console table nobody opens twice.
export const TABS = [
  { id: "general", label: "General", pane: general },
  { id: "appearance", label: "Appearance", pane: appearance },
  { id: "control", label: "Control", pane: control },
  // Its own tab because these are the things that go and fetch something, and
  // at the bottom of General under six unrelated headings the BIOS control was
  // simply not found.
  { id: "library", label: "Library", pane: library },
  // "Systems" said nothing: every tab in here is about systems of one kind or
  // another, and the word gave no clue that this is where you choose which
  // emulator runs a console. Named for the thing you come here to change.
  { id: "systems", label: "Emulators", pane: emulators },
  // Its own tab rather than a row in Appearance: the point of it is looking at
  // nine sets of artwork side by side, which needs the room.
  { id: "iconsets", label: "Icon sets", pane: iconsets },
  // Last, and the only tab with nothing to change on it.
  { id: "about", label: "About", pane: about },
];

/// Markup for one tab. Unknown ids return nothing rather than throwing, so a
/// stale saved tab cannot leave the window blank.
///
/// A pane may export its markup as a function instead of a string, for the one
/// that is built out of live state: the Control tab draws a row per action
/// with whatever key and button are bound to it now, and a string evaluated
/// when the module was imported cannot say that.
export function paneHtml(id) {
  const markup = TABS.find((t) => t.id === id)?.pane.html;
  return (typeof markup === "function" ? markup() : markup) ?? "";
}

/// Attach behavior to a rendered pane.
///
/// Every lookup inside a pane is scoped to `box`, so a pane only ever wires its
/// own controls and switching tabs cannot leave a listener pointing at an
/// element that has been removed.
export function wirePane(id, box) {
  stopPadCapture();
  return TABS.find((t) => t.id === id)?.pane.wire(box);
}

// The key and pad capture live with the Control tab, which is the only thing
// that captures anything. Re-exported because the settings *window* asks
// whether a capture is in progress before deciding what a keypress means.
export { isCapturing, captureKey, stopPadCapture } from "./settings/control.js";

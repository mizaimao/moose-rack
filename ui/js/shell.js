// Where a view is drawn, and what the window looks like while it is.
//
// This exists so the layout is one decision rather than forty-three. Every
// screen used to set the top bar itself — hide Back, show Grid, show the zoom
// slider unless we are in list mode, set the title — six or seven imperative
// lines repeated in each of six functions. That is not merely repetitive: each
// copy states, in code, that this is a single-pane app with a back button.
// Changing the shape of the window meant editing every view that had an
// opinion about it, and they all did.
//
// A view now *describes* what it needs and hands over its content. This module
// decides what that means for the window that is actually on screen. A
// different arrangement — consoles down the left, games in the middle, the
// preview on the right, nothing hidden and no Back at all — is a different
// implementation of `enter` and `paint`, not a change to any view.
//
// Nothing in here knows what a console or a game is, and no view knows what a
// column is. That is the whole point of the split.

import { el } from "./state.js";
import { clearPageFilter, setPageFilterExtra } from "./pagefilter.js";
import { followSections } from "./sections.js";

/// Where content goes, by role rather than by element.
///
/// `primary` is whatever holds the main list. `aside` is the detail beside it.
/// A three-column shell adds `nav` for the left column; a view that has
/// nothing to put there simply never asks for it, and asking for a region the
/// shell does not have is not an error — it is a region that is not shown.
/// One pane, or three columns.
///
/// The difference is entirely in where things are drawn and what the top bar
/// offers. In one pane, choosing a console *replaces* the screen and Back
/// brings it back. In three columns the consoles stay on the left, choosing
/// one fills the middle, and there is nothing to go back to — so no Back
/// button, and the preview is always there rather than something to toggle.
let mode = "single";

export function setMode(next) {
  mode = next === "single" ? "single" : "columns";
  if (el.consoles) el.consoles.hidden = mode !== "columns";
  document.body.classList.toggle("columns", mode === "columns");
  // The pair in the header, wherever the change came from — the button, the
  // dropdown in Settings, or the stored value at startup.
  for (const b of el.viewSwitch?.querySelectorAll("[data-mode]") ?? []) {
    b.classList.toggle("on", b.dataset.mode === mode);
  }
  return mode;
}

export function shellMode() {
  return mode;
}

const MODE_KEY = "moose.shell";

/// The arrangement chosen last time. Kept in localStorage rather than
/// config.toml because it is about this window rather than about the library,
/// and because the settings window has to be able to read it too.
export function storedMode() {
  // One pane unless asked otherwise. Three columns was the default while it
  // was being built, which is not a reason for anyone else to open the app in
  // it.
  return localStorage.getItem(MODE_KEY) === "columns" ? "columns" : "single";
}

/// Choose an arrangement and remember it. The other window is told, because
/// the picker lives there and the library is what changes.
export function chooseMode(next, { announce = true } = {}) {
  const applied = setMode(next);
  if (announce) {
    localStorage.setItem(MODE_KEY, applied);
    window.__TAURI__?.event?.emit?.("shell-mode", applied);
  }
  return applied;
}

const REGIONS = {
  // The list you pick from: consoles in the Library tab, collections in the
  // others. Its own column when there is one; otherwise the main pane, which
  // is what makes one set of view code serve both arrangements.
  picker: () => (mode === "columns" ? el.consoles : el.list),
  // The games. Always the main pane — in three columns that pane *is* the
  // middle column.
  games: () => el.list,
  primary: () => el.list,
  aside: () => el.detail,
};

const custom = new Map();

/// Point a role at a different element.
///
/// The seam a new layout is built through: a three-column shell registers its
/// own three, and every view carries on calling `paint("primary", …)`.
export function setRegion(role, node) {
  if (node) custom.set(role, node);
  else custom.delete(role);
}

export function region(role) {
  return custom.get(role) ?? REGIONS[role]?.() ?? null;
}

/// Draw `html` into a region. Missing regions are ignored rather than thrown
/// at: a layout without an aside is a layout, not a bug.
export function paint(role, html) {
  const node = region(role);
  if (!node) return null;
  node.innerHTML = html;
  return node;
}

/// The buttons in the top bar a view can ask for.
///
/// Everything is optional and everything defaults to hidden, so a view lists
/// what it needs rather than what it does not. `zoom: "grid"` means "only when
/// the covers are on screen", which is the one conditional every view repeated
/// and got to decide for itself.
const BUTTONS = ["back", "layout", "sidebar", "grab", "sort", "filter", "random"];

const HANDLES = {
  back: () => el.back,
  layout: () => el.layoutBtn,
  sidebar: () => el.sidebarBtn,
  grab: () => el.grabBtn,
  sort: () => el.sortBtn,
  filter: () => el.filterBtn,
  random: () => el.randomBtn,
};

/// Put the window into the state this view wants.
///
/// A window with no Back button, or no zoom slider, simply has nothing to set
/// — which is why every lookup tolerates a missing element instead of assuming
/// the skeleton this app happens to ship with.
export function enter({
  title = "",
  zoom = false,
  gridLayout = true,
  picker = true,
  filter = true,
  ...wants
} = {}) {
  // A page with no list has nothing to search. History is three charts, and
  // the top of RomM browse is five groups you can read at a glance — a box
  // over either of them is a control that does nothing, which is worse than no
  // control.
  if (el.pageFilterBar) el.pageFilterBar.hidden = !filter;
  // A view with nothing to pick from gets no column, rather than an empty one.
  if (el.consoles) el.consoles.hidden = mode !== "columns" || !picker;
  for (const name of BUTTONS) {
    const node = HANDLES[name]?.();
    if (!node) continue;
    // Back makes no sense once nothing is ever replaced: there is nowhere to
    // go back to.
    const suppressed = mode === "columns" && name === "back";
    // Sorting, filtering and picking at random are the same question — is
    // there a list of games here — so a view asks once rather than three
    // times and cannot end up offering two of the three.
    const asked = name === "filter" || name === "random" ? wants.sort : wants[name];
    // The preview toggle is a fixture of the tab row rather than a decision
    // each view makes. It moved down there to sit with what it acts on, and a
    // control that comes and goes from a row that never does is a control
    // nobody finds — reported missing twice. Once the preview has been closed
    // this is also the only way back to it, on every screen, so hiding it
    // anywhere is how someone ends up with no preview and no button.
    const wanted = asked || name === "sidebar";
    node.hidden = suppressed || !wanted;
    // Disabled rather than hidden where a screen genuinely has nothing to
    // preview: a control that vanishes from a row that never does is one
    // nobody finds again, and this one has been reported missing three times.
    // The console list is no longer such a screen — it previews the console
    // under the cursor — so it asks for the button like everywhere else.
    if (name === "sidebar") {
      const dead = !wants.sidebar && mode !== "columns";
      node.disabled = dead;
      node.title = dead ? "Nothing to preview on this screen" : "Show or hide the detail pane";
    }
  }
  if (el.zoomWrap) {
    // The one conditional worth keeping: the slider sizes covers, so it means
    // nothing in a list.
    el.zoomWrap.hidden = zoom === "grid" ? !gridLayout : !zoom;
  }
  // A filter typed for one screen means nothing on the next, and text left in
  // the box means arriving at a list that is missing things for no visible
  // reason. The button beside it belongs to the view too.
  clearPageFilter();
  setPageFilterExtra(null);
  // The strip belongs to the list that was on screen; the next view brings its
  // own, or none.
  followSections();
  if (el.title) {
    el.title.textContent = title;
    // It has a fixed share of the bar now, so a long console name ellipsises.
    el.title.title = title;
  }
}

/// Show or hide the zoom slider on its own.
///
/// Switching between grid and list is a live change rather than entering a
/// view, so it does not go through `enter` — but it is the same question about
/// the same control, and a layout without a slider still has to survive being
/// asked.
export function showZoom(on) {
  if (el.zoomWrap) el.zoomWrap.hidden = !on;
}

/// Empty the middle column, with a line saying what to do about it.
///
/// A tab that fills the left column has to clear the middle as well, or the
/// previous tab's contents sit there — History's page under a list of
/// collections, a console's games under another console's list. In one pane
/// there is only one place for content, so this is a no-op there.
export function resetGames(message) {
  if (mode !== "columns") return;
  const games = region("games");
  if (games) games.innerHTML = `<div class="empty">${message}</div>`;
}

/// Let the left column be dragged wider.
///
/// A fixed 240px is a guess about somebody else's console names, and
/// "Arcade Shmups Horizontal" does not fit in it. The width is remembered, and
/// bounded so the column cannot be dragged away entirely or over the games.
export function installColumnResizer() {
  const column = el.consoles;
  if (!column || column.dataset.resizable) return;
  column.dataset.resizable = "1";

  const grip = document.createElement("div");
  grip.id = "consoles-grip";
  grip.setAttribute("role", "separator");
  grip.setAttribute("aria-orientation", "vertical");
  grip.title = "Drag to resize";
  column.parentNode.insertBefore(grip, column.nextSibling);

  const apply = (px) => {
    const w = Math.max(160, Math.min(520, px | 0));
    column.style.flexBasis = `${w}px`;
    return w;
  };
  const saved = Number(localStorage.getItem("consolesWidth"));
  if (saved) apply(saved);

  let startX = 0;
  let startW = 0;
  // Dragging right widens it: it grows from its right edge, the opposite of
  // the preview on the other side.
  const onMove = (ev) => apply(startW + (ev.clientX - startX));
  const onUp = () => {
    document.removeEventListener("pointermove", onMove);
    document.removeEventListener("pointerup", onUp);
    grip.classList.remove("dragging");
    document.body.classList.remove("resizing-detail");
    localStorage.setItem("consolesWidth", String(column.getBoundingClientRect().width | 0));
  };
  grip.addEventListener("pointerdown", (ev) => {
    ev.preventDefault();
    startX = ev.clientX;
    startW = column.getBoundingClientRect().width;
    grip.classList.add("dragging");
    document.body.classList.add("resizing-detail");
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
  });
}

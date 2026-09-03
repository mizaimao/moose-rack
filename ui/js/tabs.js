// The top-level sections: Library, My collections, History, RomM browse.
//
// Three views that were previously reached through the same header button and a
// drill-down, which meant "my six hand-made collections" and "one thousand and
// forty companies" lived behind the same click and looked identical when you
// got there. They are different things and now sit side by side.
//
// Reachable with the shoulder buttons on a pad and Q/E on a keyboard, because
// this is the one piece of navigation you use constantly and it should not need
// the cursor.

import { el, state, trail, MOBILE } from "./state.js";
import { showPlatforms, backToPlatforms, showAllRecent } from "./library.js";
import { showCollectionGroups, showCollectionsIn } from "./collections.js";
import { showHistory } from "./history.js";
import { shellMode } from "./shell.js";

/// `user` and `smart` are collections someone made — by hand or as a saved
/// filter. Everything else RomM generates from metadata: genre, franchise,
/// company. Those are worth browsing but they are not *yours*, and the
/// distinction is the whole reason for two tabs rather than one.
export const SECTIONS = [
  {
    id: "library",
    label: "Library",
    // Coming out of a console plays the opening move in reverse. Everywhere
    // else there is nothing on screen to come out of, so there is nothing to
    // carry and it is a plain redraw.
    open: () => (state.view === "roms" ? backToPlatforms() : showPlatforms()),
  },
  {
    id: "mine",
    label: "My collections",
    // Straight into the list rather than a group picker: there is only one
    // group behind this, so the picker was a screen with a single card on it.
    open: () => showCollectionsIn("user", "My collections"),
  },
  // History on the desktop; Continue playing on Android.
  //
  // Same slot, because the thing you reach for on a handheld is the game you
  // were in the middle of, and it was three levels down: a strip on the library
  // page with a "More…" at the end of it. History — every session ever, with
  // durations — is a thing you sit down and read, which is a desktop activity.
  // The page behind Continue playing already existed as `showAllRecent`; only
  // the way in is new.
  MOBILE
    ? {
        id: "history",
        label: "Continue playing",
        open: () => showAllRecent(),
      }
    : {
        id: "history",
        label: "History",
        open: () => showHistory(),
      },
  {
    id: "browse",
    // "Browse" said nothing — every tab here browses something. These are the
    // server's own groupings: genres, franchises, companies, mirrored from
    // RomM rather than made by anyone. Naming it after where it comes from is
    // the difference between it and the tab above.
    label: "RomM browse",
    open: () => showCollectionGroups({ exclude: ["user"] }),
  },
];

let current = "library";

/// Where each section was when you left it: the crumb trail, the platform or
/// collection that was open, and how far down the list had scrolled.
///
/// Without this, switching tabs and switching back dropped you at the top of
/// the section's front page — so the shoulder buttons, which are meant to be
/// the cheapest navigation in the app, cost you your place every time.
const parked = new Map();

function park() {
  parked.set(current, {
    trail: [...trail],
    view: state.view,
    platform: state.platform,
    collection: state.collection,
    scroll: el.list.scrollTop,
    name: state.collectionName,
  });
}

/// Put a section back exactly as it was left. Returns false when there is
/// nothing parked, in which case the caller opens the section fresh.
async function unpark(id) {
  const was = parked.get(id);
  if (!was) return false;

  // Re-open whatever screen was showing rather than replaying the trail: the
  // data may have changed underneath, and a rebuild is cheap.
  const { showRoms, showPlatforms } = await import("./library.js");
  const { showCollectionRoms, showCollectionsIn, showCollectionGroups } =
    await import("./collections.js");

  if (was.view === "roms" && was.platform) await showRoms(was.platform);
  else if (was.view === "collection-roms" && was.collection)
    await showCollectionRoms(was.collection, was.name);
  else if (was.view === "collections" && id === "mine")
    await showCollectionsIn("user", "My collections");
  else if (was.view === "collections") await showCollectionGroups({ exclude: ["user"] });
  else if (was.view === "platforms") await showPlatforms();
  else return false;

  trail.length = 0;
  trail.push(...was.trail);
  // After the list has rendered, or the scroll target does not exist yet.
  requestAnimationFrame(() => {
    el.list.scrollTop = was.scroll;
  });
  return true;
}

export function activeSection() {
  return current;
}

/// Reopen the current section from the top, discarding where it was parked.
///
/// This is what Back does once the crumb trail is empty. It cannot go through
/// `showSection`, which returns immediately when asked for the section already
/// showing — correct for a tab press, and the reason Back did nothing at all
/// from inside a platform: the trail was empty, so it asked for the section it
/// was already in and got ignored.
export async function resetSection() {
  const section = SECTIONS.find((s) => s.id === current);
  if (!section) return;
  // Drop the parked position too. Otherwise the next visit to this tab would
  // restore the platform we just backed out of.
  parked.delete(current);
  state.collection = null;
  state.collectionName = null;
  trail.length = 0;
  await section.open();
}

/// Switch to a section by id. Safe to call with an unknown id.
export async function showSection(id, { force = false } = {}) {
  const section = SECTIONS.find((s) => s.id === id);
  if (!section) return;
  // Re-selecting the tab you are already on would park the section and then
  // immediately restore it, losing nothing but doing a pointless rebuild.
  // `force` is for the first call at startup, where there is nothing to park.
  if (id === current && !force) return;

  const from = SECTIONS.findIndex((s) => s.id === current);
  const to = SECTIONS.findIndex((s) => s.id === id);

  // Save where this section was before leaving it.
  park();
  current = id;
  paint();

  // Give the focus back to the page on Android.
  //
  // A tab holds keyboard focus after it is activated, which on a desktop is
  // correct — you tabbed to it, and the ring says where the keyboard is. Driven
  // by a pad nobody tabbed anywhere: the cursor is in the grid below, and a tab
  // row that still looks focused puts two highlights on screen at once, one of
  // which does not move. Nothing is lost by dropping it: keys.js listens on the
  // window rather than on whatever has focus.
  if (MOBILE) document.activeElement?.blur?.();

  // Parking restores the screen a section was left on, which is exactly right
  // when a section *is* the screen. In three columns it is not: a tab owns the
  // left column and the middle, and restoring only the screen left the other
  // tab's list on the left — Library selected with collections beside it —
  // or the previous tab's page still in the middle. Opening the tab fills both,
  // and where you were is remembered by the tab itself.
  const restored = shellMode() === "columns" ? false : await unpark(id);
  if (!restored) {
    state.collection = null;
    trail.length = 0;
    await section.open();
  }
  paint();
  if (!force) slide(to > from ? "right" : "left");
}

/// Play the page turn, in the direction the section moved.
///
/// Applied after the new content is in place rather than before: animating the
/// outgoing list would mean holding the old rows on screen while the new ones
/// load, and a bumper held down would queue transitions behind each other.
function slide(direction) {
  el.list.classList.remove("turn-left", "turn-right");
  // Forces the animation to restart when the same direction is played twice in
  // a row; without it the second press does nothing visible.
  void el.list.offsetWidth;
  el.list.classList.add(direction === "right" ? "turn-right" : "turn-left");
}

/// Move by `delta` through the sections, wrapping.
///
/// Wrapping because there are three of them and a pad user should not have to
/// remember which end they are at.
export function cycleSection(delta) {
  const at = SECTIONS.findIndex((s) => s.id === current);
  const next = (at + delta + SECTIONS.length) % SECTIONS.length;
  return showSection(SECTIONS[next].id);
}

function paint() {
  for (const btn of el.tabbar.querySelectorAll(".stab")) {
    const on = btn.dataset.id === current;
    btn.classList.toggle("active", on);
    btn.setAttribute("aria-selected", String(on));
  }
}

export function installTabs() {
  el.tabbar.replaceChildren();

  const lb = document.createElement("span");
  lb.className = "bumper";
  lb.textContent = "LB";
  el.tabbar.appendChild(lb);

  for (const s of SECTIONS) {
    const btn = document.createElement("button");
    btn.className = "stab";
    btn.dataset.id = s.id;
    btn.textContent = s.label;
    btn.setAttribute("role", "tab");
    btn.addEventListener("click", () => showSection(s.id));
    el.tabbar.appendChild(btn);
  }

  const rb = document.createElement("span");
  rb.className = "bumper";
  rb.textContent = "RB";
  el.tabbar.appendChild(rb);

  // The preview toggle, at the far right of this row rather than in the header.
  //
  // It belongs with what it acts on. Up there it was one of nine buttons in a
  // bar of unrelated things — settings, search, sort, take offline — and the
  // thing it opens and closes is a third of the window. Moved rather than
  // copied: it is the same element, so everything that hides or relabels it
  // goes on working.
  // Take offline first, then the preview toggle, both in a holder pinned to
  // the right-hand end.
  //
  // The push right used to be a margin on Take offline, which vanishes with
  // it: on any screen where there is nothing to take offline that button is
  // `hidden`, hidden means out of the layout, and the preview toggle slid back
  // up against RB in the middle of the row. A holder is always there whatever
  // is inside it.
  // The filter goes in before the end holder, so the two buttons stay at the
  // very right and the box sits clear of the preview column beneath it.
  if (el.pageFilterBar) el.tabbar.appendChild(el.pageFilterBar);

  const end = document.createElement("span");
  end.className = "tabbar-end";
  if (el.grabBtn) end.appendChild(el.grabBtn);
  if (el.sidebarBtn) end.appendChild(el.sidebarBtn);
  // Android has no header — it is a desktop object and `body.mobile` hides it
  // outright — so sync and settings would have no door at all. Settings has
  // been on Start, which is a thing you have to be told; this row is where you
  // are already looking.
  //
  // Moved into the end holder rather than added to SECTIONS. A section is
  // something the shoulder buttons cycle through, and on Android opening
  // settings navigates the whole webview to settings.html: as a section, one
  // press of RB past the last tab would take the app off the page. Whatever
  // these two are, they are not somewhere LB and RB should be able to land.
  if (MOBILE) {
    if (el.syncBtn) end.appendChild(el.syncBtn);
    if (el.settingsBtn) end.appendChild(el.settingsBtn);
  }
  el.tabbar.appendChild(end);

  paint();
}

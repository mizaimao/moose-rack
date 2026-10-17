// The arrangement of the list on screen: which rows survive the filters, in
// what order, and what the two header buttons should say.
//
// One cached answer, shared by sort.js and filter.js, because the backend
// computes both in one pass — narrowing 2,506 arcade games and then ordering
// what is left is one question, and asking it twice would sort rows that are
// about to be thrown away.
//
// Cached because `renderRows` is synchronous. Everything that can change the
// answer — opening a list, changing the order, toggling a filter — is already
// asynchronous, so the round trip happens there and the redraw reads the
// result.

import { state, invoke } from "./state.js";

/// Nothing decided yet. `ids: null` means "draw what you have": it is what the
/// backend answers when it is not holding this list, and drawing the rows in
/// the order they arrived is the honest response to that.
const UNARRANGED = {
  ids: null,
  order: "name",
  order_label: "Name",
  filters: [],
  sortable: false,
  filterable: false,
};

let current = UNARRANGED;

/// Which page is asking.
///
/// The backend keeps the list it last handed out, so a change of sort or
/// filter is answered with ids rather than the whole list travelling back. It
/// kept one for everybody, and moose-service is one backend for every browser
/// pointed at it: the second person to open a list replaced the first
/// person's, whose next sort then came back as ids from a list they were not
/// looking at. It keeps one per page now, under this. Not a secret, and not
/// kept across a reload; it only has to differ from the other pages open.
const PAGE = Math.random().toString(36).slice(2) + Date.now().toString(36);

/// Which list a question is about: the three fields the backend keys its
/// per-view memory on (see `gamelist::scope`), and the page asking.
export function listRef() {
  return listFor(state.view, state.platform, state.collection);
}

/// The same for a list that is not on screen yet. See `prefetchRoms`.
export function listFor(view, platform = null, collection = null) {
  return { view, platform: platform ?? null, collection: collection ?? null, page: PAGE };
}

export function arrangement() {
  return current;
}

export function applyArrangement(next) {
  current = next ?? UNARRANGED;
  return current;
}

/// Forget the list, for a screen that is not a list of games.
///
/// The console grid, History and a collections list hold no games, but
/// `state.rows` and the arrangement above went on describing whichever list
/// was open last. Everything that acts on "the list on screen" asks those two,
/// so pressing S on the console grid sorted SNES and drew its games under the
/// title "Platforms", R picked a game nobody could see, and changing the game
/// art in Settings painted the old console over History.
export function leaveList() {
  state.rows = [];
  applyArrangement(null);
}

/// Ask the backend how the list on screen should be drawn.
///
/// Called after every fetch that replaces `state.rows`, before the rows are
/// rendered. A view with nothing to sort still gets an answer, because the
/// header buttons hide themselves off `sortable` and `filterable`.
export async function arrangeCurrentList() {
  try {
    return applyArrangement(await invoke("arrange_list", { list: listRef() }));
  } catch (e) {
    // A list that cannot be arranged is still a list. Drawing it in the order
    // it arrived is better than not drawing it.
    console.warn("arranging the list:", e);
    return applyArrangement({ ...UNARRANGED, sortable: true, filterable: true });
  }
}

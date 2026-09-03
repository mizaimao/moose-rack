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

/// Which list a question is about. The three fields the backend keys its
/// per-view memory on — see `gamelist::scope`.
export function listRef() {
  return {
    view: state.view,
    platform: state.platform ?? null,
    collection: state.collection ?? null,
  };
}

export function arrangement() {
  return current;
}

export function applyArrangement(next) {
  current = next ?? UNARRANGED;
  return current;
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

// Narrowing a list down to the part you meant.
//
// The predicates, the opposing pairs and the per-view memory are in
// `src/gamefilter.rs`. What is left here is the menu, the button, and a cache
// of the backend's answer — `filtered()` is called from inside a redraw, which
// cannot await anything.
//
// The arcade console holds 2,506 games. Sorting them is not the same as
// finding one: "by rating" still leaves 2,506 rows, and the eleven you have
// actually played are somewhere in it. Every frontend has this and this one
// did not.

import { el, state, invoke } from "./state.js";
import { showMenu } from "./menu.js";
import { listRef, applyArrangement, arrangement } from "./arrange.js";

/// The filters, in the order the menu offers them. Filled by
/// `loadListControls` in sort.js, which fetches both tables in one call.
export let FILTERS = [];

export function setFilters(list) {
  FILTERS = list;
}

export function activeFilters() {
  return arrangement().filters;
}

export async function toggleFilter(id) {
  applyArrangement(await invoke("toggle_list_filter", { list: listRef(), filter: id }));
  return activeFilters();
}

export async function clearFilters() {
  applyArrangement(await invoke("clear_list_filters", { list: listRef() }));
}

/// Apply them, from the arrangement the backend last worked out.
///
/// The narrowing and the ordering are one answer — see `sorted` in sort.js,
/// which reads the same list of ids. This returns the rows that survived so
/// the "nothing matched" message can tell an empty console from an empty
/// filter.
export function filtered(rows) {
  const ids = arrangement().ids;
  if (!ids) return rows;
  const keep = new Set(ids);
  return rows.filter((r) => keep.has(r.id));
}

/// Whether this view has a list worth narrowing.
export function filterable() {
  return arrangement().filterable;
}

/// The button says how many are on, because a filtered list looks exactly like
/// a short one and there is nothing else on screen to say why.
export function refreshFilterButton() {
  if (!el.filterBtn) return;
  el.filterBtn.hidden = !filterable();
  const on = activeFilters().length;
  el.filterBtn.classList.toggle("on", on > 0);
  el.filterBtn
    .querySelector("span:not(.icon)")
    ?.replaceChildren(document.createTextNode(on ? `Filters · ${on}` : "Filter"));
}

/// The menu. Items are sticky: a filter is something you build up out of two
/// or three choices, and a menu that shuts on each one makes that four trips.
export function openFilterMenu(anchor) {
  if (!filterable()) return;
  const on = new Set(activeFilters());
  const items = FILTERS.map((f) => ({
    label: `${on.has(f.id) ? "✓" : "  "} ${f.label}`,
    sticky: true,
    run: () => {
      toggleFilter(f.id).then(redraw);
      return true;
    },
  }));
  if (on.size) {
    items.push(null, {
      label: "Clear filters",
      run: () => {
        clearFilters().then(redraw);
      },
    });
  }
  const at = (anchor ?? el.filterBtn)?.getBoundingClientRect();
  showMenu(items, at?.left ?? 40, (at?.bottom ?? 40) + 4);
}

/// Redraw the current list. Imported lazily for the same reason the sort does:
/// library.js draws the rows and this has to ask it to draw them again.
async function redraw() {
  const { renderRows } = await import("./library.js");
  if (state.rows.length) renderRows(state.rows, state.view === "search");
  refreshFilterButton();
  // Rebuild the menu in place so the ticks move as they are pressed.
  if (document.querySelector(".ctx-menu")) openFilterMenu();
}

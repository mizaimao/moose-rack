// How a list of games is ordered, and the menu that changes it.
//
// The orders themselves, the comparison and the per-view memory are in
// `src/gamesort.rs` and `src/gamelist.rs`. What is left here is the menu, the
// button, and a cache of the backend's answer — because `sorted()` is called
// from inside a redraw, which cannot await anything.
//
// Per view, and deliberately not saved. Sorting by rating to see what a
// console is famous for, and then finding every console still sorted that way
// a week later, is a setting that has outlived its question. It lasts as long
// as the app is open, which is how long the reason for it usually lasts.
//
// The console grid has no sort of its own: it is one screen of a couple of
// dozen tiles in a fixed order that people learn the shape of, and shuffling
// it would cost more than it gives.

import { el, state, invoke } from "./state.js";
import { escapeHtml } from "./util.js";
import { listRef, applyArrangement, arrangement, arrangeCurrentList } from "./arrange.js";

/// The orders, in the sequence the menu lists them. Filled by `loadListControls`.
export let ORDERS = [];

export async function loadListControls() {
  const controls = await invoke("list_controls");
  ORDERS = controls.orders;
  return controls;
}

export function currentOrder() {
  const at = arrangement();
  return ORDERS.find((o) => o.id === at.order) ?? ORDERS[0] ?? { id: "name", label: "Name" };
}

export async function setOrder(id) {
  applyArrangement(await invoke("set_list_order", { list: listRef(), order: id }));
}

/// The order a view starts in, if the user has not picked one for it yet.
///
/// Continue playing is ordered by *when you played it* or it is not a
/// continue-playing list — it arrived grouped by console, which threw that
/// away. Set rather than forced, so choosing something else still sticks.
export async function defaultOrder(id) {
  applyArrangement(
    await invoke("set_list_order", { list: listRef(), order: id, preferred: true })
  );
}

/// Order a copy of `rows` by the arrangement the backend last worked out.
///
/// Never in place: `state.rows` is what the page was given, and re-sorting it
/// repeatedly would compound rather than replace.
export function sorted(rows) {
  const ids = arrangement().ids;
  if (!ids) return [...rows];
  const by = new Map(rows.map((r) => [r.id, r]));
  return ids.map((id) => by.get(id)).filter(Boolean);
}

/// Whether this view has anything worth sorting.
export function sortable() {
  return arrangement().sortable;
}

let open = null;

/// The menu. Opened by the button in the header or by the right stick click.
export function openSortMenu(anchor) {
  if (open?.isConnected) {
    open.remove();
    open = null;
    return;
  }
  if (!sortable()) return;

  const menu = document.createElement("div");
  menu.id = "sort-menu";
  const now = currentOrder().id;
  menu.innerHTML = ORDERS.map(
    (o) =>
      `<button data-order="${o.id}" class="${o.id === now ? "on" : ""}">
         ${escapeHtml(o.label)}</button>`
  ).join("");

  const at = (anchor ?? el.sortBtn)?.getBoundingClientRect();
  menu.style.top = `${(at?.bottom ?? 40) + 4}px`;
  menu.style.right = `${Math.max(8, window.innerWidth - (at?.right ?? window.innerWidth))}px`;
  document.body.appendChild(menu);
  open = menu;

  const close = () => {
    menu.remove();
    open = null;
  };
  for (const b of menu.querySelectorAll("button")) {
    b.addEventListener("click", async () => {
      await setOrder(b.dataset.order);
      close();
      redraw();
    });
  }
  // Next frame, or the click that opened this closes it again.
  setTimeout(() => {
    window.addEventListener("pointerdown", close, { once: true });
  }, 0);
}

/// Step to the next order without opening anything, for the stick click.
///
/// Returns the new order's label for the toast, or null where the view has no
/// sort to step through.
export async function cycleOrder(delta = 1) {
  if (!sortable()) return null;
  const at = await invoke("cycle_list_order", { list: listRef(), delta });
  applyArrangement(at);
  redraw();
  return at.order_label;
}

/// Redraw the current list in the new order.
///
/// Imported lazily to keep this module out of a cycle: library.js draws the
/// rows and needs the sort, and the sort needs to ask it to draw again.
async function redraw() {
  const { renderRows } = await import("./library.js");
  if (state.rows.length) renderRows(state.rows, state.view === "search");
  el.sortBtn?.querySelector("span:not(.icon)")?.replaceChildren(
    document.createTextNode(currentOrder().label)
  );
}

/// Keep the header button's label and visibility in step with the view.
export function refreshSortButton() {
  if (!el.sortBtn) return;
  el.sortBtn.hidden = !sortable();
  el.sortBtn.querySelector("span:not(.icon)")?.replaceChildren(
    document.createTextNode(currentOrder().label)
  );
}

export { arrangeCurrentList };

// The order of the left column, and the little bar above it.
//
// The orders, the comparison and the storage are in `src/pickorder.rs`. What
// is left here is the bar, the menu it opens, and a cache of what the backend
// said — `pickerBar` builds markup and cannot await.
//
// Separate from sort.js, which orders games. The two answer different
// questions and want different answers: a game list sorted by rating is a
// question you asked about one console and stop caring about when you leave
// it, so that one is deliberately forgotten. The order of the column is the
// shape of the whole app — you learn where things are in it — so this one is
// remembered, in config.toml rather than in this document's own storage.

import { escapeHtml } from "./util.js";
import { showMenu } from "./menu.js";

const invoke = (...args) => window.__TAURI__.core.invoke(...args);

/// What each kind of list offers, and which of them is chosen. Filled by
/// `loadPickerOrders` before the bar is drawn.
const known = new Map();

/// Ask what this kind of list offers. Consoles offer nothing, and that is the
/// answer rather than an omission: thirty-five of them that never change is a
/// column you learn the shape of, which a button that reshuffles it works
/// against.
export async function loadPickerOrders(kind) {
  const controls = await invoke("picker_controls", { kind });
  known.set(kind, controls);
  return controls;
}

function controlsFor(kind) {
  return known.get(kind) ?? { orders: [], chosen: null, label: null };
}

/// The orders a kind offers, for the menu.
export function pickerOrders(kind) {
  return controlsFor(kind).orders;
}

/// The chosen order, as `{ id, label }`, or null for a kind with no choice.
export function pickerOrder(kind) {
  const at = controlsFor(kind);
  return at.chosen ? { id: at.chosen, label: at.label } : null;
}

export async function setPickerOrder(kind, id) {
  await invoke("set_picker_order", { kind, order: id });
  await loadPickerOrders(kind);
}

/// Which entries to draw, in what order.
///
/// A copy: the caller's array is what it was handed, and re-sorting it in
/// place would compound across redraws.
export async function sortPicker(kind, items) {
  const at = await invoke("sort_picker", { kind, rows: rowsOf(items) });
  known.set(kind, { orders: at.orders, chosen: at.chosen, label: at.label });
  return at.order.map((i) => items[i]);
}

/// The four facts an order can be built out of. Sent rather than the whole
/// entry, which carries sample cover ids the ordering has no use for.
function rowsOf(items) {
  return items.map((c) => ({
    name: c.name ?? "",
    rom_count: c.rom_count ?? 0,
    local_count: c.local_count ?? 0,
    is_favorite: !!c.is_favorite,
  }));
}

/// The button that says how a list is ordered.
///
/// It used to come with a filter box beside it, drawn into the top of the
/// list. The filter is furniture of the tab row now — one box for every page
/// rather than one for the only page that had it — so this is the button
/// alone, and it goes in the slot next to that box.
export function pickerBar({ kind }) {
  return `<div class="pickbar">
    <button class="pick-sort" type="button" title="Change the order of this list">
      <span>${escapeHtml(pickerOrder(kind)?.label ?? "Name")}</span>
    </button>
  </div>`;
}

/// Keep the button's own label in step. The list is redrawn by the caller, but
/// the bar above it is not part of that redraw — so without this the button
/// went on saying "Name" over a list sorted by size.
function relabel(root, kind) {
  const btn = root?.querySelector(".pick-sort span");
  if (btn) btn.textContent = pickerOrder(kind)?.label ?? "Name";
}

/// Wire the order button inside `root`. `redraw` is called after a change.
export function wirePickerBar(root, kind, redraw) {
  const btn = root?.querySelector(".pick-sort");
  if (!btn) return;
  btn.addEventListener("click", (ev) => {
    ev.stopPropagation();
    const now = pickerOrder(kind)?.id;
    const at = btn.getBoundingClientRect();
    showMenu(
      pickerOrders(kind).map((o) => ({
        label: o.id === now ? `✓ ${o.label}` : `   ${o.label}`,
        run: async () => {
          await setPickerOrder(kind, o.id);
          relabel(root, kind);
          redraw();
        },
      })),
      at.left,
      at.bottom + 4
    );
  });
}

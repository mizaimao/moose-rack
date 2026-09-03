// A stand-in for the Rust backend, for the tests that run in jsdom.
//
// These tests are about the *webview*: that the sort menu opens, that the
// filter button counts what is on, that a controller can answer a dialog. The
// decisions those things are made of moved to `src/` — which orders survive,
// what a filter keeps, where the cursor lands in a grid — and are asserted by
// `cargo test` against the real implementation.
//
// So this is deliberately simple, and deliberately not a second copy of any of
// it. Ordering here is the order it was given; filtering keeps everything.
// Nothing in `ui/test/` may assert an ordering or a predicate through this
// file — if a test would fail because this is naive, that test belongs in
// Rust.
//
// The one thing it cannot be naive about is the binding table: half the
// controller tests press button 0 and expect a game to open. That comes from
// `bindings.fixture.json`, which `binds::tests::the_webview_test_fixture_
// matches_these_tables` regenerates and checks — so a moved default button
// fails `cargo test` rather than quietly changing what these tests press.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const DEFAULTS = JSON.parse(readFileSync(join(here, "bindings.fixture.json"), "utf8"));

const clone = (v) => JSON.parse(JSON.stringify(v));

/// Build a fake `invoke`.
///
/// `reply` is the test's own answering machine for everything else — `roms`,
/// `rom_detail`, `platforms` and the rest — as either a `(cmd, args)` function
/// or an object keyed by command. The commands below are answered here and
/// never reach it, so a test file keeps the stub it already had.
export function fakeBackend(reply = () => []) {
  let bindings = clone(DEFAULTS);
  let page = { names: [], groups: [] };
  // Per view, exactly as the real one is scoped: `view:platform:collection`.
  const orders = new Map();
  const filters = new Map();
  const pickers = new Map();
  let rows = [];
  /// What the backend would have left after narrowing and ordering.
  ///
  /// A test that needs the list on screen to be a particular subset — the
  /// "surprise me" button picks from what the filters left, and there has to
  /// be something left out to prove it — says so here rather than relying on
  /// this file to reimplement the predicates.
  let keep = (all) => all;

  const scopeOf = (list) =>
    `${list?.view ?? ""}:${list?.platform ?? ""}:${list?.collection ?? ""}`;

  const arrangement = (list) => {
    const scope = scopeOf(list);
    const order = orders.get(scope) ?? "name";
    return {
      // The order they arrived in, and everything kept, unless the test has
      // said otherwise with `.arrange()`. See the note at the top: an ordering
      // assertion does not belong in a test that runs through this.
      ids: keep(rows, [...(filters.get(scope) ?? [])], order).map((r) => r.id),
      order,
      order_label: (ORDERS.find((o) => o.id === order) ?? ORDERS[0]).label,
      filters: [...(filters.get(scope) ?? [])],
      sortable: list?.view !== "platforms" && list?.view !== "systems",
      filterable:
        list?.view !== "platforms" && list?.view !== "systems" && list?.view !== "history",
    };
  };

  const ORDERS = [
    { id: "name", label: "Name", dir: 1 },
    { id: "rating", label: "Rating", dir: -1 },
    { id: "year", label: "Release year", dir: -1 },
    { id: "played", label: "Recently played", dir: -1 },
    { id: "size", label: "Size", dir: -1 },
    { id: "platform", label: "Console", dir: 1 },
  ];
  const FILTERS = [
    { id: "local", label: "On this machine" },
    { id: "missing", label: "Not downloaded" },
    { id: "fav", label: "Starred" },
    { id: "unplayed", label: "Never played" },
    { id: "played", label: "Played before" },
    { id: "great", label: "Rated 8 or better" },
    { id: "twoplayer", label: "Two players or more" },
  ];
  const PICKER_ORDERS = {
    collections: [
      { id: "name", label: "Name", dir: 1 },
      { id: "count", label: "Most games", dir: -1 },
      { id: "fewest", label: "Fewest games", dir: 1 },
      { id: "here", label: "Most downloaded", dir: -1 },
    ],
  };

  /// Remember whatever the test's own `roms`/`search`/`collection_roms`
  /// handler returned, so `arrange_list` has something to arrange.
  const remember = (result) => {
    if (Array.isArray(result) && result.every((r) => r && typeof r.id === "number")) {
      rows = result;
    }
    return result;
  };

  const own = {
    ui_bindings: () => clone(bindings),

    set_key_binding: ({ action, key }) => {
      for (const id of Object.keys(bindings.keys)) {
        if (key && id !== action && bindings.keys[id] === key) {
          bindings.keys[id] = null;
          bindings.key_labels[id] = "—";
        }
      }
      bindings.keys[action] = key ?? null;
      bindings.key_labels[action] = keyLabel(key);
      return clone(bindings);
    },

    set_pad_binding: ({ action, index }) => {
      for (const at of Object.keys(bindings.pad_map)) {
        if (bindings.pad_map[at] === action) bindings.pad_map[at] = null;
      }
      if (index !== null && index !== undefined) bindings.pad_map[index] = action;
      for (const a of bindings.actions) {
        bindings.pad_labels[a.id] = padLabel(indexOf(a.id));
      }
      return clone(bindings);
    },

    reset_bindings: () => {
      bindings = clone(DEFAULTS);
      return clone(bindings);
    },

    import_bindings: () => clone(bindings),

    list_controls: () => ({ orders: ORDERS, filters: FILTERS }),

    arrange_list: ({ list }) => arrangement(list),

    set_list_order: ({ list, order, preferred }) => {
      const scope = scopeOf(list);
      if (!preferred || !orders.has(scope)) orders.set(scope, order);
      return arrangement(list);
    },

    cycle_list_order: ({ list, delta }) => {
      const scope = scopeOf(list);
      const at = ORDERS.findIndex((o) => o.id === (orders.get(scope) ?? "name"));
      orders.set(scope, ORDERS[(at + delta + ORDERS.length) % ORDERS.length].id);
      return arrangement(list);
    },

    toggle_list_filter: ({ list, filter }) => {
      const scope = scopeOf(list);
      const on = new Set(filters.get(scope) ?? []);
      const opposite = {
        local: "missing", missing: "local", unplayed: "played", played: "unplayed",
      }[filter];
      if (on.has(filter)) on.delete(filter);
      else {
        on.add(filter);
        if (opposite) on.delete(opposite);
      }
      filters.set(scope, on);
      return arrangement(list);
    },

    clear_list_filters: ({ list }) => {
      filters.delete(scopeOf(list));
      return arrangement(list);
    },

    picker_controls: ({ kind }) => ({
      order: [],
      orders: PICKER_ORDERS[kind] ?? [],
      chosen: pickers.get(kind) ?? (PICKER_ORDERS[kind] ?? [])[0]?.id ?? null,
      label:
        (PICKER_ORDERS[kind] ?? []).find((o) => o.id === (pickers.get(kind) ?? ""))?.label ??
        (PICKER_ORDERS[kind] ?? [])[0]?.label ??
        null,
    }),

    sort_picker: ({ kind, rows: given }) => ({
      ...own.picker_controls({ kind }),
      order: given.map((_, i) => i),
    }),

    set_picker_order: ({ kind, order }) => {
      pickers.set(kind, order);
      return null;
    },

    set_page_names: ({ names, groups }) => {
      page = { names, groups: groups ?? [] };
      return null;
    },

    // The one predicate copied here, because it is one line and several tests
    // about the *box* need it to do something. The rule itself is asserted in
    // `pagefilter::tests`.
    page_filter: ({ query }) => {
      const want = String(query ?? "").trim().toLowerCase();
      const visible = page.names.map((n) => !want || n.toLowerCase().includes(want));
      return {
        visible,
        headings: page.groups.map(
          (g) => !!want && g.length > 0 && !g.some((i) => visible[i])
        ),
        shown: visible.filter(Boolean).length,
      };
    },

    // A grid that is uniform needs no measuring, so this one is not naive: a
    // windowed list navigates entirely through it, and a linear stand-in would
    // make every test about that navigation agree with itself and with
    // nothing else. That the arithmetic is right is asserted in
    // `gridnav::tests`, against the measured table on the same layout.
    grid_uniform: ({ count, columns }) => {
      const cols = Math.max(1, columns || 1);
      const rows = Math.ceil(count / cols);
      const lastIn = (r) => Math.min(count - r * cols, cols) - 1;
      const sideways = (step) =>
        Array.from({ length: count }, (_, i) => {
          const r = Math.floor(i / cols);
          return r * cols + Math.max(0, Math.min((i % cols) + step, lastIn(r)));
        });
      const vertical = (step) =>
        Array.from({ length: count }, (_, i) => {
          const target = Math.floor(i / cols) + step;
          if (target < 0 || target >= rows) return null;
          return target * cols + Math.min(i % cols, lastIn(target));
        });
      return {
        up: vertical(-1), down: vertical(1),
        left: sideways(-1), right: sideways(1),
        page_up: vertical(-3), page_down: vertical(3),
        first: count ? 0 : null,
        last: count ? count - 1 : null,
      };
    },

    // Linear, on purpose. Which card sits above which is geometry, and
    // `gridnav::tests` is where that is asserted — jsdom has no layout to
    // measure anyway, so every card here reports the same position.
    set_grid: ({ cards }) => {
      const n = cards.length;
      const before = (i) => (i > 0 ? i - 1 : null);
      const after = (i) => (i < n - 1 ? i + 1 : null);
      const table = (fn) => Array.from({ length: n }, (_, i) => fn(i));
      return {
        up: table(before),
        down: table(after),
        left: table(before),
        right: table(after),
        page_up: table(before),
        page_down: table(after),
        first: n ? 0 : null,
        last: n ? n - 1 : null,
      };
    },
  };

  function indexOf(action) {
    for (const [at, bound] of Object.entries(bindings.pad_map)) {
      if (bound === action) return Number(at);
    }
    return null;
  }

  function padLabel(index) {
    if (index === null) return "unset";
    return bindings.pad_buttons.find((b) => b.index === index)?.name ?? `button ${index}`;
  }

  function keyLabel(key) {
    if (!key) return "—";
    return (
      { ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓",
        Escape: "Esc", Backspace: "⌫", Enter: "⏎", " ": "Space",
        PageUp: "PgUp", PageDown: "PgDn" }[key] || key.toUpperCase()
    );
  }

  async function invoke(cmd, args = {}) {
    if (Object.hasOwn(own, cmd)) return own[cmd](args);
    const answer =
      typeof reply === "function"
        ? await reply(cmd, args)
        : Object.hasOwn(reply, cmd)
          ? await reply[cmd](args)
          : [];
    return remember(answer);
  }

  /// The rows the list on screen is holding, for tests that put them into
  /// `state.rows` directly rather than through a command.
  invoke.rows = (given) => {
    rows = given;
  };

  /// How this stand-in should narrow and order them. `(rows, filters, order)`
  /// in, the survivors out, in the order they should be drawn.
  invoke.arrange = (fn) => {
    keep = fn;
  };

  return invoke;
}

/// The default tables, for a test that wants to assert against them directly.
export const DEFAULT_BINDINGS = DEFAULTS;

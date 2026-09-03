// Browsing RomM's collections.
//
// These are the server's groupings, not ours — genres, franchises, companies,
// series and any hand-made lists, mirrored by `sync`. Three levels: groups →
// collections in a group → the games. ES-DE shows collections beside systems,
// so this sits next to the platform grid rather than inside it.

import { el, state, trail, invoke, convertFileSrc, listen } from "./state.js";
import { enter as enterView, region, resetGames } from "./shell.js";
import { escapeHtml } from "./util.js";
import { pickerBar, wirePickerBar, sortPicker, loadPickerOrders } from "./picker-order.js";
import { arrangeCurrentList, listRef } from "./arrange.js";
import {
  setPageFilterLabel, setPageFilterExtra, refreshPageFilter,
} from "./pagefilter.js";
import { renderRows, applyLayoutForView } from "./library.js";
import { restoreSidebar } from "./detail.js";
import { collectionArt } from "./collection-art.js";
import { shellMode } from "./shell.js";

function enter(fn) {
  trail.push(fn);
}

/// Light up the collection whose games are in the middle.
///
/// The column stays on screen while the middle changes, so without this there
/// is nothing to say which of twenty-seven you are looking at.
function markOpen(grid, cid) {
  for (const card of grid.querySelectorAll(".card")) {
    card.classList.toggle("open", card.dataset.cid === cid);
  }
}

/// The top bar both collection-browsing levels want.
///
/// `filter` is off for the top of Browse: five groups you can read at a glance
/// are not a list anybody needs to search.
function topBar(title, { filter = true } = {}) {
  state.view = "collections";
  applyLayoutForView("collections");
  restoreSidebar();
  state.platform = null;
  state.selected = null;
  el.detail.hidden = true;
  el.search.value = "";
  // The zoom slider stays: these are cards too, and sizing them is the same
  // want as sizing covers.
  enterView({ title, back: true, zoom: true, filter });
}

export async function showCollectionGroups({ exclude = [] } = {}) {
  trail.length = 0;
  topBar("RomM browse", { filter: false });
  resetGames("Pick a group on the left.");

  // `user` lives in its own tab now, so showing it here too would be the same
  // collections in two places with different names.
  const groups = (await invoke("collection_groups")).filter(
    (g) => !exclude.includes(g.group)
  );
  if (!groups.length) {
    region("picker").innerHTML = `<div class="empty">No collections on the server. Run a sync, or make one in RomM.</div>`;
    return;
  }

  region("picker").innerHTML = `<div class="grid">${groups
    .map(
      (g) => `
      <div class="card" data-group="${escapeHtml(g.group)}">
        <div class="logo"><span class="ph">${escapeHtml(g.label.slice(0, 2))}</span></div>
        <div class="name">${escapeHtml(g.label)}</div>
        <div class="meta">${g.count} collections</div>
      </div>`
    )
    .join("")}</div>`;

  // Attached inside the region the groups were drawn into. They were attached
  // to cards in the middle, which has none of them — so clicking a group in
  // the left column did nothing at all.
  const picker = region("picker");
  picker.querySelectorAll(".card").forEach((c) =>
    c.addEventListener("click", () => {
      const g = groups.find((x) => x.group === c.dataset.group);
      state.lastGroup = g.group;
      markGroup(picker, g.group);
      enter(showCollectionGroups);
      showCollectionsIn(g.group, g.label, { into: intoFor("browse") });
    })
  );
  picker.querySelector(".card")?.classList.add("sel");

  // Browse is three levels deep — groups, then collections, then games — and
  // there are two columns for lists. So the groups keep the column and their
  // collections go to the middle, rather than replacing the groups and leaving
  // no way to see that a group list existed at all.
  if (shellMode() === "columns" && groups.length) {
    const want = groups.find((g) => g.group === state.lastGroup) ?? groups[0];
    state.lastGroup = want.group;
    markGroup(picker, want.group);
    await showCollectionsIn(want.group, want.label, { into: "games" });
  }
}

/// Which region a group's collections belong in.
///
/// In My collections they are the top level and take the column. Reached
/// through Browse they are one level down, and the column already holds the
/// groups they came from.
function intoFor(section) {
  if (shellMode() !== "columns") return "picker";
  return section === "browse" ? "games" : "picker";
}

function markGroup(picker, group) {
  for (const card of picker.querySelectorAll("[data-group]")) {
    card.classList.toggle("open", card.dataset.group === group);
  }
}

/// The last group drawn, so a setting that changes how these look can redraw
/// the same grid rather than reloading the window.
let lastDrawn = null;

/// Draw the collections grid again, exactly as it is. Used when the picture
/// style changes: the cards are rebuilt, nothing else moves.
export function redrawCollections() {
  if (!lastDrawn) return;
  const { group, label, into } = lastDrawn;
  showCollectionsIn(group, label, { into });
}

export async function showCollectionsIn(group, label, { into = "picker" } = {}) {
  lastDrawn = { group, label, into };
  topBar(label || group);
  if (into === "picker") resetGames("Pick a collection on the left.");
  const items = await invoke("collections_in", { group });

  // 1,040 companies is not a browsable list, so filter locally. Kept separate
  // from the header search, which searches games rather than collections.
  // No filter box drawn into the list any more: that is the one in the tab
  // row now, where it is in the same place on every screen instead of only
  // this one, and where it does not sit in the middle of a page of cards.
  region(into).innerHTML = `<div class="grid" id="cgrid"></div>`;
  setPageFilterLabel(`${items.length} collections`);
  // The order button rides beside it. Which orders it offers, and which one
  // is on, come from config.toml — so the bar is drawn after that is known
  // rather than saying "Name" over a list ordered by something else.
  await loadPickerOrders("collections");
  const bar = document.createElement("span");
  bar.innerHTML = pickerBar({ kind: "collections" });
  setPageFilterExtra(bar.firstElementChild);
  wirePickerBar(document.getElementById("page-filter-extra"), "collections", () => draw(items));

  const grid = document.getElementById("cgrid");

  async function draw(unordered) {
    const list = await sortPicker("collections", unordered);
    grid.innerHTML = list
      .map(
        (c) => `
        <div class="card" data-cid="${escapeHtml(c.id)}">
          <div class="logo mosaic ${collectionArt()}"><span class="ph">${escapeHtml(c.name.slice(0, 2))}</span></div>
          <div class="name">${c.is_favorite ? "★ " : ""}${escapeHtml(c.name)}</div>
          <div class="meta">${c.rom_count} games${
            c.local_count ? `<span class="here"> · ${c.local_count} here</span>` : ""
          }</div>
        </div>`
      )
      .join("");

    grid.querySelectorAll(".card").forEach((card) =>
      card.addEventListener("click", () => {
        const c = list.find((x) => String(x.id) === card.dataset.cid);
        state.lastCollection = { id: c.id, name: c.name };
        markOpen(grid, card.dataset.cid);
        enter(() => showCollectionsIn(group, label));
        showCollectionRoms(c.id, c.name);
      })
    );
    // Leave a selection so the controller can act immediately, as the
    // platform grid does.
    grid.querySelector(".card")?.classList.add("sel");
    loadMosaics(list.slice(0, 60), grid);
    // The list is new; whatever is in the filter box has never seen it.
    refreshPageFilter();
  }

  await draw(items);

  // The middle belongs to a collection's games, and one of them may as well be
  // showing: the one you were last in, or the first. The same thing Library
  // does with consoles, and what makes returning to a tab put you back where
  // you were rather than at a prompt.
  if (shellMode() === "columns" && into === "picker" && items.length) {
    const want =
      items.find((c) => String(c.id) === String(state.lastCollection?.id)) ?? items[0];
    state.lastCollection = { id: want.id, name: want.name };
    markOpen(grid, String(want.id));
    await showCollectionRoms(want.id, want.name);
  }

  // Deliberately not focused: arrow keys and the controller should navigate
  // the grid straight away. Click the box, or press the search key, to filter.
}

export async function showCollectionRoms(id, name) {
  state.view = "collection-roms";
  applyLayoutForView("collection-roms");
  restoreSidebar();
  state.collection = id;
  // The collection's own name, kept apart from the rendered title. Restoring a
  // parked section used to hand the title back as the name, so "Arcade Sports —
  // 256 games" became the name and gained a second "— 256 games" every time.
  state.collectionName = name;
  el.search.value = "";

  state.rows = await invoke("collection_roms", { id: String(id), list: listRef() });
  await arrangeCurrentList();
  enterView({
    title: `${name} — ${state.rows.length} games`,
    back: true,
    layout: true,
    sidebar: true,
    grab: true,
    sort: true,
    zoom: "grid",
    gridLayout: state.layout === "grid",
  });
  // Collections mix platforms, so the list view needs the platform column.
  renderRows(state.rows, true);
}

/// Fill collection cards with a member's cover, through the same local cache
/// the game grids use — so this works offline and needs no server request.
async function loadMosaics(list, into) {
  // A collection with no sample ids — an older backend, or one the server has
  // not filled in — is a collection with no cover, not a crash halfway through
  // drawing the list.
  const style = collectionArt();
  if (style === "none") return;
  // How many members each card needs art for. One for a single cover, three
  // for the fan, four for the grid — asked for in one batch either way.
  const want = style === "fan" ? 3 : style === "tiles" ? 4 : 1;
  const ids = list.flatMap((c) => (c.sample_ids ?? []).slice(0, want));
  if (!ids.length) return;
  // Twice, deliberately. The first call answers from the filesystem alone and
  // returns in milliseconds, so the grid fills as fast as it can; the second
  // is allowed to go to the server for whatever was missing. Before this there
  // was one call that did both, and every card with no cached art held up
  // every card that had some.
  const paint = (covers) => {
    const byId = new Map(covers.map((c) => [c.id, c.cover]));
    for (const c of list) {
      const picks = (c.sample_ids ?? [])
        .slice(0, want)
        .map((id) => byId.get(id))
        .filter(Boolean);
      if (!picks.length) continue;
      // The grid the cards were actually drawn into, passed in.
      //
      // This read `region(into)`, and `into` is a parameter of the function
      // that *calls* this one — so it was a ReferenceError on the first card,
      // every time. The catch below is meant for a failed cover lookup and
      // swallowed that instead, so every collection in the app drew the
      // two-letter placeholder and nothing ever said why.
      const logo = into.querySelector(`.card[data-cid="${CSS.escape(String(c.id))}"] .logo`);
      if (!logo) continue;
      // The fan and the grid want every cover; a single is the first one.
      // Rendered back-to-front so the first pick is the one on top.
      logo.innerHTML = (style === "single" ? picks.slice(0, 1) : picks)
        .map((src, i) => `<img style="--i:${i}" src="${convertFileSrc(src)}" alt="" />`)
        .join("");
    }
  };

  try {
    paint(await invoke("rom_covers", { ids, localOnly: true }));
  } catch {
    // Placeholders are fine; artwork is not worth interrupting browsing for.
  }

  // The slow pass paints as it arrives rather than at the end.
  //
  // It fetches eight at a time and used to resolve only once every one of them
  // was in, so a screen of collections asking for eighty covers held the first
  // eight — which were ready almost at once — behind the eightieth. Each batch
  // is drawn as it lands.
  let stop;
  try {
    stop = await listen("covers-ready", ({ payload }) => {
      if (Array.isArray(payload)) paint(payload);
    });
    // If the grid has been left by the time this finishes, the cards it would
    // write into are gone and the lookups simply miss.
    paint(await invoke("rom_covers", { ids }));
  } catch {
    // As above.
  } finally {
    stop?.();
  }
}

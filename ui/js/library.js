// Platform grid, game grid/list, and lazy cover loading.

import { el, state, trail, invoke, convertFileSrc, rememberedRom, MOBILE } from "./state.js";
import { fitted, boxSize } from "./fitpicture.js";
import { resetNav, primeNav } from "./keys.js";
import { currentOrder, defaultOrder, refreshSortButton, sorted } from "./sort.js";
import { filtered, refreshFilterButton, activeFilters, clearFilters } from "./filter.js";
import { arrangeCurrentList, listRef } from "./arrange.js";
import { enter, region, showZoom, shellMode } from "./shell.js";
import { showMenu } from "./menu.js";
import { deleteState } from "./states.js";
import { human, escapeHtml, toast } from "./util.js";
import { setPageFilterLabel, refreshPageFilter } from "./pagefilter.js";
import { followSections } from "./sections.js";
import { play, restoreSidebar, selectRom, showPlatformInfo, withTransition } from "./detail.js";
import { download, launch } from "./actions.js";
import { installTilt } from "./tilt.js";
import { windowRows, stopWindowing, worthWindowing, windowedList } from "./visible.js";

export async function showPlatforms() {
  state.view = "platforms";
  applyLayoutForView("platforms");
  trail.length = 0;
  state.platform = null;
  state.selected = null;
  // In three columns the preview is a column, not something to hide, and the
  // middle is not emptied — going "back" to the consoles there is a thing that
  // does not happen.
  // In three columns the preview is a column rather than something to hide —
  // unless it has been closed, which is a choice that has to survive changing
  // screens or the button that closes it does nothing you can see.
  restoreSidebar();
  el.detail.hidden = shellMode() === "columns" ? !state.sidebar : true;
  enter({
    title: "Platforms",
    // Grid or list works here too. It was hidden on this screen, which is why
    // there was no way to see thirty-five consoles without scrolling past
    // thirty-five pictures of them.
    layout: true,
    // Offered here as well as inside a console: burying it one level down
    // means it is only found by someone who already knows it exists.
    grab: true,
    // The console screen has something to preview — the console under the
    // cursor — so the toggle belongs here too. Without this the button was
    // *disabled* on this screen while the pane it controls was still being
    // opened by every cursor move, so the panel appeared on any D-pad press
    // and there was no way to shut it.
    sidebar: true,
    zoom: "grid",
    gridLayout: state.layout === "grid",
  });
  coverObserver?.disconnect();

  const items = await invoke("platforms");
  for (const p of items) if (p.cover_aspect) state.aspects[p.slug] = p.cover_aspect;
  // Kept so switching layout can redraw without asking the backend again.
  state.platforms = items;

  renderPlatforms(items);
  if (shellMode() === "columns") {
    // Three columns, three columns' worth of content. A window that opens with
    // two thirds of itself empty until something is clicked is asking to be
    // told what it already knows: which console you were last in, or failing
    // that the first one. Choosing it fills the middle, and the middle fills
    // the preview.
    if (!state.platform) {
      const pick =
        items.find((p) => p.slug === state.lastPlatform)?.slug ?? items[0]?.slug;
      if (pick) {
        await showRoms(pick);
        markPlatform(pick);
        return;
      }
      region("games").innerHTML =
        `<div class="empty">Nothing here yet — sync with the server first.</div>`;
    }
    restorePlatformCursor();
    fitConsoleArt();
    return;
  }
  // Not on Android: it has a tab of its own there, and the library page is for
  // consoles. The strip costs most of a screen on a 469-point display, which is
  // the height that decides whether the console grid reads as a grid at all.
  if (!MOBILE) await showRecent();

  restorePlatformCursor();
  fitConsoleArt();
  // Fill the pane straight away if it is meant to be open. It only ever got
  // its contents from a cursor *move*, so on a fresh start with the toggle on
  // the column sat empty and hidden until the first D-pad press — which read
  // as the D-pad opening it rather than as it having been open all along.
  if (state.sidebar && state.view === "platforms") {
    const pick =
      state.platformShown
      ?? items.find((x) => x.slug === state.lastPlatform)?.slug
      ?? items[0]?.slug;
    if (pick) showPlatformInfo(pick);
  }
}

/// A row of the games you played most recently, above the consoles.
///
/// The timestamps are the server's, so this is the same list on every machine —
/// which is the point: the thing you were in the middle of is rarely on the
/// computer you are now sitting at. Absent entirely when nothing has been
/// played, rather than an empty row explaining itself.
/// How many recent games the strip will hold before it stops growing.
///
/// One row that scrolls sideways, up to about two screens' worth. Past that the
/// strip stops being a shortcut and becomes a second library above the real
/// one — so the rest go behind a button rather than off the side of a scroll
/// nobody would reach the end of.
const RECENT_IN_STRIP = 20;

async function showRecent() {
  let rows = [];
  try {
    // One more than fits, so the strip can tell whether there is a "more" to
    // show without a second call.
    rows = await invoke("recent_games", { limit: RECENT_IN_STRIP + 1 });
  } catch {
    return;
  }
  if (!rows.length) return;

  const overflow = rows.length > RECENT_IN_STRIP;
  const shown = rows.slice(0, RECENT_IN_STRIP);

  const strip = document.createElement("section");
  strip.className = "recent";
  strip.innerHTML =
    `<h2 class="ghead"><span class="gslug">Continue playing</span>` +
    (overflow ? `<button class="link recent-more">More…</button>` : "") +
    `</h2>` +
    `<div class="gcards">${shown.map((r) => `
       <div class="gcard" data-id="${r.id}" data-name="${escapeHtml(r.name.slice(0, 2))}">
         <div class="art"><span class="ph">${escapeHtml(r.name.slice(0, 2))}</span></div>
         <div class="gname">${escapeHtml(r.name)}</div>
         <div class="gmeta">${here(r)}${escapeHtml(r.platform)}</div>
       </div>`).join("")}</div>`;
  // Replace, never stack. This is prepended whenever the platform screen is
  // drawn, and drawing it twice — click More, come back, scroll — left two
  // strips one above the other, their headings running together as
  // "CONTINUE PLAYINGMORE…".
  el.list.querySelector(":scope > .recent")?.remove();
  el.list.prepend(strip);
  strip.querySelectorAll(".gcard").forEach((c) =>
    wireGame(c, Number(c.dataset.id), { resume: true })
  );
  // One listener on the row rather than one per card: the strip is rebuilt on
  // every draw of the platform screen.
  installTilt(strip.querySelector(".gcards"));
  strip.querySelector(".recent-more")?.addEventListener("click", showAllRecent);
  // Re-observe after prepending, so these cards get covers like any others.
  // The observer was set up before this row existed.
  observeCovers();
}

/// Everything you have played, as a page rather than a strip.
export async function showAllRecent() {
  state.view = "search";
  applyLayoutForView("search");
  restoreSidebar();
  state.platform = null;
  trail.length = 0;
  trail.push(() => showPlatforms());
  el.detail.hidden = !state.sidebar;
  enter({
    title: "Continue playing",
    back: true,
    layout: true,
    sidebar: true,
    sort: true,
    zoom: "grid",
    gridLayout: state.layout === "grid",
  });

  let rows = [];
  try {
    rows = await invoke("recent_games", { limit: 500, list: listRef() });
  } catch (e) {
    region("primary").innerHTML = `<div class="empty">${escapeHtml(String(e))}</div>`;
    return;
  }
  // Ungrouped by default. `recent_games` returns them most-recent-first, and
  // grouping by console threw that ordering away — which is the one thing a
  // list called "Continue playing" is ordered by. The sort button is offered
  // (`sort: true` above), so console grouping is still a keystroke away for
  // anyone who wants it; it is no longer what you get without asking.
  state.rows = rows;
  // Most recent first unless this view has already been sorted otherwise, and
  // grouped by console only when that is what was asked for.
  await defaultOrder("played");
  renderRows(rows, currentOrder().id === "platform");
}

/// Draw the consoles, in whichever layout is selected.
///
/// Both shapes carry the same three facts — name, how many games, whether a
/// core is installed — so switching layout changes the density and nothing
/// else. The list is for finding a console you can name; the grid is for
/// recognizing one you cannot.
/// Light up the console whose games are showing, in the left column.
///
/// The column stays on screen while the middle changes, so without this there
/// is nothing to say which of thirty-five consoles you are looking at.
function markPlatform(slug) {
  for (const node of region("picker").querySelectorAll("[data-slug]")) {
    node.classList.toggle("open", node.dataset.slug === slug);
  }
}

function renderPlatforms(items) {
  // Into the consoles region, which is its own column when there is one and
  // the main pane when there is not. The rest of this function does not know
  // or care which it got.
  const into = region("picker");
  if (!into) return;
  // A column is narrow, so the console cards are always a list there — a grid
  // of two-across cards in a 260px column is neither a grid nor readable.
  const asList = state.layout !== "grid" || shellMode() === "columns";
  // Already alphabetical: the `platforms` command orders them, because the
  // server hands them back by size and this grid is redrawn on a layout switch
  // and on every batch of covers that arrives. Left alone thereafter —
  // thirty-five consoles that never change are something you learn the shape
  // of, and a button that reshuffles them works against that.
  const ordered = items;
  into.innerHTML = asList
    ? `<div class="rows">${ordered.map(platformRow).join("")}</div>`
    : `<div class="grid">${ordered.map(platformCard).join("")}</div>`;
  setPageFilterLabel(`${items.length} consoles`);
  refreshPageFilter();

  into.querySelectorAll(".card, .prow").forEach((c) => {
    // Selection, not hover. The preview follows the cursor here exactly as it
    // does inside a console — a pane that changes as the pointer crosses the
    // list is a pane you cannot read, because reaching for it changes it.
    c.addEventListener("click", () => {
      if (shellMode() === "columns") {
        markPlatform(c.dataset.slug);
        showPlatformInfo(c.dataset.slug);
      }
      openPlatform(c.dataset.slug, c);
    });
  });
  if (shellMode() === "columns" && state.platform) markPlatform(state.platform);
  // Thirty-five consoles are never windowed, and leaving a window's scroll
  // listener attached to a list it no longer draws is a listener that runs on
  // every scroll of every screen after this one.
  stopWindowing();
  resetNav();
  primeNav();
}

/// Open a console, carrying its name up into the title bar.
///
/// The two are the same words in two places, so the browser is told they are
/// one element and moves it, rather than fading a grid out and a list in with
/// no thread between them. That thread is the whole point: it says where the
/// screen you are now looking at came from.
///
/// Falls back to a plain navigation wherever view transitions are missing or
/// the user has asked for less motion — `withTransition` handles both — so the
/// tags are cleaned up in a `finally` and never left on an element.
export async function openPlatform(slug, card) {
  const label = card?.querySelector(".name, .nm");
  if (label) label.style.viewTransitionName = "heading";
  try {
    await withTransition(async () => {
      await showRoms(slug);
      // Tagged inside the callback: the new snapshot is taken after this runs,
      // and the title only holds the console's name by then.
      el.title.style.viewTransitionName = "heading";
    });
  } finally {
    el.title.style.viewTransitionName = "";
    if (label) label.style.viewTransitionName = "";
  }
}

/// The same move in reverse, coming back out to the consoles.
///
/// Worth the symmetry: a transition that plays going in and not coming out
/// reads as a glitch rather than as a deliberate direction.
export async function backToPlatforms() {
  el.title.style.viewTransitionName = "heading";
  let label = null;
  try {
    await withTransition(async () => {
      await showPlatforms();
      label = el.list.querySelector(
        `[data-slug="${CSS.escape(state.lastPlatform ?? "")}"] .name, ` +
          `[data-slug="${CSS.escape(state.lastPlatform ?? "")}"] .nm`
      );
      if (label) label.style.viewTransitionName = "heading";
    });
  } finally {
    el.title.style.viewTransitionName = "";
    if (label) label.style.viewTransitionName = "";
  }
}

function platformCard(p) {
  return `
      <div class="card" data-slug="${p.slug}">
        <div class="logo">${
          p.logo
            ? `<img data-fit="1" class="${p.logo_wordmark ? "wordmark" : "art"}" src="${convertFileSrc(p.logo)}" alt="" />`
            : `<span class="wordtype">${escapeHtml(p.slug)}</span>`
        }</div>
        <div class="name">${escapeHtml(p.name)}</div>
        <div class="meta">
          <span class="dot ${p.playable ? "on" : ""}" title="${
            p.playable ? "An emulator for this console is installed" : "No emulator installed — games here will not start"
          }"></span>
          ${p.rom_count} games${p.playable ? "" : " · no core"}
        </div>
      </div>`;
}

function platformRow(p) {
  // `row` as well as `prow` so keyboard and pad navigation pick it up: the
  // movement code selects `.card, .gcard, .row, .tcard`, and a row that is not
  // one of those is unreachable without a mouse.
  // The dot says whether an emulator for this console is installed — green for
  // yes, gray for no. It is a fair signal on a wide row where the words "no
  // core" are beside it, and a riddle in a 240px column where they are not, so
  // the row says so in words and the title says it in full.
  return `
      <div class="row prow" data-slug="${p.slug}"
        title="${escapeHtml(p.name)} — ${p.rom_count} games${
          p.playable ? "" : ", but no emulator installed for it"
        }">
        <span class="have"><span class="dot ${p.playable ? "on" : ""}" title="${
          p.playable ? "An emulator for this console is installed" : "No emulator installed — games here will not start"
        }"></span></span>
        <span class="nm">${escapeHtml(p.name)}</span>
        <span class="pf">${escapeHtml(p.slug)}</span>
        <span class="sz">${p.rom_count} games${p.playable ? "" : " · no core"}</span>
      </div>`;
}

/// Put the cursor back on the console you were just in. Coming out of a game
/// list to find the selection reset to the top-left corner means re-finding
/// your place every single time.
function restorePlatformCursor() {
  const back = state.lastPlatform
    ? el.list.querySelector(`[data-slug="${CSS.escape(state.lastPlatform)}"]`)
    : null;
  if (back) {
    back.classList.add("sel");
    back.scrollIntoView({ block: "center" });
  } else {
    // Always leave something selected, so the controller's A button has a
    // target the moment the grid appears.
    el.list.querySelector(".card, .prow")?.classList.add("sel");
  }
}

export async function showRoms(slug) {
  state.view = "roms";
  applyLayoutForView("roms");
  // Read this system's artwork folders while the list is being drawn.
  //
  // Building that index means walking directories of several hundred files off
  // a card, and it used to land on whichever game the cursor touched first: on
  // the Retroid the first selection took 809ms and every one after it 30-60.
  // On Android an IPC call blocks the page while it runs, so that first press
  // was not a slow pane, it was a frozen app.
  //
  // Not awaited, and the backend spawns rather than blocking, so this costs the
  // list nothing.
  invoke("warm_media", { platform: slug }).catch(() => {});
  restoreSidebar();
  // The console list is a column of its own here and stays where it is; only
  // the middle changes. In one pane it has already been replaced by the time
  // this runs, which is the difference between the two arrangements in one
  // line.
  if (shellMode() === "columns" && !region("picker").children.length) {
    await showPlatforms();
  }
  state.platform = slug;
  state.lastPlatform = slug;
  localStorage.setItem("lastPlatform", slug);
  state.folder = "";
  el.search.value = "";
  state.rows = await invoke("roms", { platform: slug, list: listRef() });
  await arrangeCurrentList();
  enter({
    title: `${slug} — ${state.rows.length} games`,
    back: true,
    layout: true,
    sidebar: true,
    grab: true,
    sort: true,
    zoom: "grid",
    gridLayout: state.layout === "grid",
  });
  renderRows(state.rows, false);
}

export async function runSearch(term) {
  if (!term.trim()) {
    return state.platform ? showRoms(state.platform) : showPlatforms();
  }
  state.view = "search";
  applyLayoutForView("search");
  restoreSidebar();
  state.rows = await invoke("search", { term, list: listRef() });
  await arrangeCurrentList();
  const consoles = new Set(state.rows.map((r) => r.platform)).size;
  enter({
    title:
      `Search “${term}” — ${state.rows.length}` +
      (consoles > 1 ? ` across ${consoles} consoles` : ""),
    back: true,
    layout: true,
    sidebar: true,
    sort: true,
    zoom: "grid",
    gridLayout: state.layout === "grid",
  });
  renderRows(state.rows, true);
}

/// Click, double-click and right-click on one game.
///
/// Shared, because there are two places that draw games — the list, and the
/// "continue playing" strip above it — and they had drifted: the strip could be
/// selected and nothing else. Right-click there did nothing at all, and neither
/// did double-click, on cards that look exactly like the ones below where both
/// work.
/// Listen once, on the container, instead of on every game.
///
/// Attaching three listeners per row means 7,518 of them for the arcade list,
/// created and thrown away on every platform switch — which is what made
/// switching consoles feel slow. One listener on the container answers for all
/// of them, and survives the list being redrawn, so a redraw costs a string
/// and nothing else.
const WIRED = new WeakSet();

export function delegateGames(container) {
  if (!container || WIRED.has(container)) return;
  WIRED.add(container);

  const idOf = (ev) => {
    const node = ev.target.closest?.("[data-id]");
    return node && container.contains(node) ? Number(node.dataset.id) : null;
  };

  container.addEventListener("click", (ev) => {
    const dir = ev.target.closest?.("[data-dir]");
    if (dir && container.contains(dir)) return openFolder(dir.dataset.dir);
    const id = idOf(ev);
    if (id !== null) selectRom(id);
  });
  // Double-click is the shortcut for "just play it".
  container.addEventListener("dblclick", async (ev) => {
    const id = idOf(ev);
    if (id === null) return;
    ev.preventDefault();
    play(await invoke("rom_detail", { id }));
  });
  container.addEventListener("contextmenu", (ev) => {
    const id = idOf(ev);
    if (id === null) return;
    ev.preventDefault();
    gameMenu(id, ev.clientX, ev.clientY);
  });
}

/// One game's click, double-click and right-click, for a node that is not
/// inside a delegated container.
export function wireGame(node, id, { resume = false } = {}) {
  node.addEventListener("click", () => selectRom(id));
  // Double-click is the shortcut for "just play it".
  node.addEventListener("dblclick", async (ev) => {
    ev.preventDefault();
    // From Continue playing, carry on rather than start over. Everywhere else
    // a double-click means "play this", and a game you opened deliberately
    // from its console list is as likely to be a fresh run.
    if (resume) return launch(id, { resume: true });
    play(await invoke("rom_detail", { id }));
  });
  node.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    gameMenu(id, ev.clientX, ev.clientY);
  });
}

/// The right-click menu for one game.
async function gameMenu(id, x, y) {
  selectRom(id);
  let d;
  try {
    d = await invoke("rom_detail", { id });
  } catch {
    return;
  }
  // The game's save states, if it has any. This menu used to offer "take
  // this console offline" instead, which on a game in Continue playing reads
  // as an offer to download a whole platform — the opposite of the small,
  // local thing a right-click on one game should do. Taking a console
  // offline is on the toolbar, where it belongs.
  let states = [];
  try {
    states = await invoke("game_states", { id });
  } catch {
    states = [];
  }
  showMenu(
    [
      { label: d.downloaded ? "Play" : "Download and play", run: () => play(d) },
      d.downloaded ? null : { label: "Download", run: () => download(id, false) },
      // A rule between starting the game and destroying part of it.
      null,
      ...(states.length
        ? states.map((st) => ({
            label: `Delete ${st.label}${st.when ? ` — ${st.when}` : ""}`,
            danger: true,
            run: () => deleteState(id, st),
          }))
        : [{ label: "No save states", disabled: true }]),
    ],
    x,
    y
  );
}

/// Put the highlight back on the card the cursor is on.
///
/// A windowed list rebuilds its cards every time the band moves, so the node
/// carrying `.sel` is thrown away and replaced by one that does not. The
/// cursor is a row, not a node — this is what keeps the two in step.
function markSelected() {
  if (state.selected === null || state.selected === undefined) return;
  const want = String(state.selected);
  for (const node of el.list.querySelectorAll(".gcard, .row")) {
    node.classList.toggle("sel", node.dataset.id === want);
  }
}

/// Split a console's rows into what is on screen at the folder now open.
///
/// A game whose `rel_dir` is the current folder is drawn as a game; anything
/// deeper contributes its next path segment as a folder. Two levels down
/// (`AdditionalRoms/Homebrew`) that means the shelf shows `AdditionalRoms`
/// and nothing else until you open it, which is what ES-DE does.
function atFolder(rows) {
  const here = state.folder || "";
  const under = here ? `${here}/` : "";
  const games = [];
  const dirs = new Map();
  for (const r of rows) {
    const rel = r.rel_dir || "";
    if (rel === here) {
      games.push(r);
      continue;
    }
    if (!rel.startsWith(under)) continue;
    const next = rel.slice(under.length).split("/")[0];
    if (!next) continue;
    const seen = dirs.get(next) ?? { name: next, path: under + next, count: 0 };
    seen.count += 1;
    dirs.set(next, seen);
  }
  return {
    games,
    dirs: [...dirs.values()].sort((a, b) => a.name.localeCompare(b.name)),
  };
}

/// Walk into a folder, or back out of one.
///
/// Back is the trail the controller and the Back button already share, so one
/// press goes up one level and the press after that leaves the console.
export function openFolder(path) {
  const from = state.folder || "";
  if ((path || "") === from) return;
  // Only walking *in* leaves a crumb. Back calls `goToFolder` directly —
  // pushing here as well would drop a crumb pointing forward again, and Back
  // would bounce between two levels forever.
  trail.push(() => goToFolder(from));
  goToFolder(path);
}

function goToFolder(path) {
  state.folder = path || "";
  el.title.textContent = state.folder
    ? `${state.platform} / ${state.folder.split("/").join(" / ")}`
    : `${state.platform} — ${state.rows.length} games`;
  renderRows(state.rows, false);
}

function folderCardMarkup(d) {
  return `
      <div class="gcard folder" data-dir="${escapeHtml(d.path)}">
        <div class="art"><span class="icon icon-folder"></span></div>
        <div class="gname">${escapeHtml(d.name)}</div>
        <div class="gmeta">${d.count} game${d.count === 1 ? "" : "s"}</div>
      </div>`;
}

function folderRowMarkup(d) {
  return `
      <div class="row folder" data-dir="${escapeHtml(d.path)}">
        <span class="have"><span class="icon icon-folder"></span></span>
        <span class="nm">${escapeHtml(d.name)}</span>
        <span class="sz">${d.count} game${d.count === 1 ? "" : "s"}</span>
      </div>`;
}

/// The folders for this level, as one block above the games.
///
/// Drawn separately rather than mixed into the list because the games are
/// sorted, filtered and windowed and a folder is none of those things — it
/// belongs at the top whatever the list is ordered by, the way a file manager
/// does it.
function foldersMarkup(dirs) {
  if (!dirs.length) return "";
  const inner =
    state.layout === "grid"
      ? `<div class="gcards">${dirs.map(folderCardMarkup).join("")}</div>`
      : `<div class="rows">${dirs.map(folderRowMarkup).join("")}</div>`;
  return `<div class="folders">${inner}</div>`;
}

export function renderRows(unsorted, showPlatform) {
  // Ordered here rather than by whoever supplied the rows, so every path that
  // draws a list — a console, a collection, a search, a redraw after the order
  // changed — goes through the same comparison.
  // Narrowed first, then ordered. The other way round sorts rows that are
  // about to be thrown away, which on 2,506 games is most of the work.
  //
  // Folders come out before any of that. They are not sorted, filtered or
  // windowed with the games, and a console whose top level is nothing but
  // folders is not an empty console.
  const { games, dirs } =
    showPlatform || state.view !== "roms" ? { games: unsorted, dirs: [] } : atFolder(unsorted);
  const rows = sorted(filtered(games));
  refreshSortButton();
  refreshFilterButton();
  if (!rows.length && !dirs.length) {
    stopWindowing();
    // A filtered list that matches nothing looks exactly like a console with
    // no games in it, and the filter is off screen in a menu — so the empty
    // list has to say so, and offer the way out.
    region("games").innerHTML = activeFilters().length
      ? `<div class="empty">Nothing here matches the filters.
           <button class="link clear-filters">Clear them</button></div>`
      : `<div class="empty">Nothing here.</div>`;
    region("games")
      .querySelector(".clear-filters")
      ?.addEventListener("click", async () => {
        await clearFilters();
        renderRows(unsorted, showPlatform);
      });
    return;
  }
  // Search spans every console, so a flat list of 200 hits buries the ones you
  // meant. Grouping also lets each console keep its own cover shape, which a
  // single mixed grid cannot.
  resetNav();

  // A long flat list draws only the band around the viewport. Grouped results
  // are drawn whole: search is capped at 200 by the backend, and a grouped
  // collection is sections of a few hundred — below the threshold a window is
  // machinery with nothing to do. See `visible.js`.
  const window = !showPlatform && worthWindowing(rows.length);
  if (window) {
    const uniform = gridAspect(null);
    region("games").innerHTML =
      state.layout === "grid"
        ? `<div class="gcards"${uniform ? ` style="--ar:${uniform.toFixed(3)}"` : ""}></div>`
        : `<div class="rows"></div>`;
    windowRows({
      container: region("games").firstElementChild,
      scroller: el.list,
      rows,
      html: (r, at) =>
        state.layout === "grid"
          ? cardMarkup(r, uniform, at)
          : rowMarkup(r, showPlatform, at),
      // Every band change is a different set of nodes, so both of the things
      // that hold a map of the page have to be told: the cover observers, and
      // the cursor.
      onDraw: () => {
        if (state.layout === "grid") observeCovers();
        // The card the cursor was on has just been thrown away and drawn
        // again, without the class that says so.
        markSelected();
      },
    });
  } else {
    stopWindowing();
    region("games").innerHTML = showPlatform
      ? groupedMarkup(rows)
      : state.layout === "grid"
        ? gridMarkup(rows)
        : listMarkup(rows, showPlatform);
  }
  // After the games, so the windowed list keeps the container it was handed.
  if (dirs.length) region("games").insertAdjacentHTML("afterbegin", foldersMarkup(dirs));
  // Work out where the cursor can go while nobody is waiting on it, rather
  // than on the first arrow press after this.
  primeNav();

  delegateGames(region("games"));
  setPageFilterLabel(`${rows.length} games`);
  refreshPageFilter();
  followSections();

  if (state.layout === "grid") observeCovers();
  // Put the cursor back where it was in this list, falling back to the top.
  // Without this, every trip out to the platform grid and back means scrolling
  // to find your place again — which on a 2,500-game arcade list is the whole
  // journey a second time.
  if (rows.length) {
    const want = rememberedRom(rows) ?? rows[0].id;
    selectRom(want);
    let node = el.list.querySelector(`[data-id="${want}"]`);
    // Windowed, the remembered row is very often not drawn — being far down
    // the list is exactly why it was worth remembering. Ask the window for it,
    // which scrolls there and draws the band around it.
    if (!node) {
      const at = rows.findIndex((r) => r.id === want);
      node = windowedList()?.reveal(at) ?? null;
    }
    // `nearest` rather than `center`: if the remembered row is already on
    // screen, scrolling it to the middle moves the list for no reason.
    node?.scrollIntoView({ block: "nearest" });
  }
}

/// Search results, split into one section per console, biggest group first.
/// Consoles that are the same machine under two names, in the order they
/// should appear.
///
/// A search for "Zelda" turns up the NES release and the Famicom one, and they
/// belong beside each other — they are the same game on the same hardware,
/// sold in two places. Still separate headings, because the cartridges, the
/// regions and the dumps genuinely differ, but never with eight other consoles
/// between them.
const FAMILIES = [
  ["nes", "famicom"],
  ["snes", "sfc"],
];

/// The family a console belongs to, and where it sits within it. A console in
/// no family is a family of one.
function family(slug) {
  const at = FAMILIES.findIndex((f) => f.includes(slug));
  return at < 0 ? [slug, 0] : [FAMILIES[at][0], FAMILIES[at].indexOf(slug)];
}

function groupedMarkup(rows) {
  const groups = new Map();
  for (const r of rows) {
    if (!groups.has(r.platform)) groups.set(r.platform, []);
    groups.get(r.platform).push(r);
  }

  // Families are ordered by their combined weight, so a console with three
  // hits and its twin with two outrank one with four — they are one machine as
  // far as "how much of this search is about it" goes.
  const weight = new Map();
  for (const [slug, items] of groups) {
    const [key] = family(slug);
    weight.set(key, (weight.get(key) ?? 0) + items.length);
  }

  const ordered = [...groups.entries()].sort((a, b) => {
    const [ka, ia] = family(a[0]);
    const [kb, ib] = family(b[0]);
    if (ka !== kb) return weight.get(kb) - weight.get(ka) || ka.localeCompare(kb);
    // Inside a family, the order the family declares — the home-market name
    // first, which is the one most people search for.
    return ia - ib;
  });

  // One group is not a grouping. A collection that happens to hold games from a
  // single console got a heading repeating what the title already said, sitting
  // over the first row and cropping it.
  const single = ordered.length === 1;

  // Numbered across the whole result rather than within each section, so
  // `data-at` means the same thing here as it does in a flat list: where the
  // cursor is in what is on screen, top to bottom.
  let at = 0;
  return ordered
    .map(([platform, items]) => {
      const from = at;
      at += items.length;
      return `
      <section class="pgroup">
        ${single ? "" : `<h2 class="ghead">
          <span class="gslug">${escapeHtml(platform)}</span>
          <span class="gcount">${items.length}</span>
        </h2>`}
        ${state.layout === "grid"
          ? gridMarkup(items, platform, from)
          : listMarkup(items, false, from)}
      </section>`;
    })
    .join("");
}

/// One row.
///
/// `at` is its place in the whole list, not in what happens to be drawn — a
/// windowed list draws a band out of the middle, and the cursor moves through
/// rows that are not on the page. See `visible.js`.
function rowMarkup(r, showPlatform, at) {
  return `
      <div class="row${r.favorite ? " fav" : ""}" data-id="${r.id}" data-at="${at}">
        <span class="have">${here(r)}</span>
        <span class="nm">${r.favorite ? `<span class="star" title="Starred — in one of your starred collections">★</span>` : ""}${escapeHtml(r.name)}</span>
        ${showPlatform ? `<span class="pf">${r.platform}</span>` : ""}
        <span class="sz">${human(r.size_bytes)}</span>
      </div>`;
}

function listMarkup(rows, showPlatform, from = 0) {
  return `<div class="rows">${rows
    .map((r, i) => rowMarkup(r, showPlatform, from + i))
    .join("")}</div>`;
}

/// The mark that says a game is on this machine.
///
/// It was a bare "▣" with nothing to say what it meant — a symbol that appears
/// on some cards and not others and explains itself nowhere is a symbol that
/// makes people guess. It is a labelled icon now, and the games that are *not*
/// here say so too: "no mark" is not an answer anybody can read, especially on
/// a console where everything happens to be downloaded and the mark is on
/// every card.
function here(r) {
  return r.downloaded
    ? `<span class="mark here" title="On this machine — ready to play offline"><span class="icon icon-disk"></span></span>`
    : `<span class="mark away" title="On the server — downloads when you play it"><span class="icon icon-cloud"></span></span>`;
}

/// The ratio every card in a grid is shaped to, or null where the rows come
/// from more than one console and each card has to say for itself.
///
/// Grouped search passes its console in, so each section is uniform even
/// though the results as a whole are not. Uniformity is what lets a long list
/// be windowed at all — see `visible.js`.
function gridAspect(platform) {
  const slug = platform ?? (state.view !== "search" ? state.platform : null);
  return slug ? (state.aspects[slug] ?? null) : null;
}

/// One card. `at` is its place in the whole list; see `rowMarkup`.
function cardMarkup(r, uniform, at) {
  return `
      <div class="gcard" data-id="${r.id}" data-at="${at}" data-name="${escapeHtml(r.name.slice(0, 2))}"${
        r.favorite ? ` data-fav="1"` : ""
      }${
        !uniform && state.aspects[r.platform]
          ? ` style="--ar:${state.aspects[r.platform].toFixed(3)}"`
          : ""
      }>
        <div class="art"><span class="ph">${escapeHtml(r.name.slice(0, 2))}</span>${
          r.favorite
            ? `<span class="star" title="Starred — in one of your starred collections">★</span>`
            : ""
        }</div>
        <div class="gname">${escapeHtml(r.name)}</div>
        <div class="gmeta">${here(r)}${human(r.size_bytes)}</div>
      </div>`;
}

function gridMarkup(rows, platform, from = 0) {
  const uniform = gridAspect(platform);
  const style = uniform ? ` style="--ar:${uniform.toFixed(3)}"` : "";
  return `<div class="gcards"${style}>${rows
    .map((r, i) => cardMarkup(r, uniform, from + i))
    .join("")}</div>`;
}

/// Card size, applied as a CSS variable so both grids scale from one number.
/// Scroll the game list by `amount` pixels.
///
/// Its own function so the pad, the keyboard and anything else move the same
/// list in the same way — and so "which element actually scrolls" is answered
/// in one place rather than assumed in three.
export function scrollList(amount) {
  el.list.scrollTop += amount;
}

export function setZoom(px) {
  state.zoom = px;
  localStorage.setItem("zoom", String(px));
  el.zoom.value = String(px);
  document.documentElement.style.setProperty("--card", `${px}px`);
  // Wider cards mean fewer columns and a taller row, so a windowed list is
  // now drawing the wrong band and standing the wrong height in for the rest.
  // The resize listener does not fire for this: the window did not change, the
  // cards did.
  windowedList()?.remeasure();
  resetNav();
  primeNav();
}

/// Which stored layout the view on screen is using.
///
/// The console screen and the inside of a console are separate settings; search
/// results are a list of games, so they follow the games one.
export function layoutKeyForView(view = state.view) {
  return view === "platforms" ? "layoutPlatforms" : "layoutGames";
}

/// Point `state.layout` at the setting for the view being entered.
///
/// Called wherever `state.view` changes. Without it the console screen would
/// draw with whatever the last console used, which is exactly the coupling this
/// pair of settings exists to remove.
export function applyLayoutForView(view = state.view) {
  const next = state[layoutKeyForView(view)];
  if (state.layout !== next) state.layout = next;
  paintLayoutButton(next);
  showZoom(next === "grid");
}

/// The toggle in the header, showing what pressing it will do next.
function paintLayoutButton(now) {
  if (!el.layoutBtn) return;
  const label = el.layoutBtn.querySelector("span:not(.icon)");
  if (label) label.textContent = now === "grid" ? "List" : "Grid";
  const icon = el.layoutBtn.querySelector(".icon");
  if (icon) icon.className = `icon icon-${now === "grid" ? "list" : "grid"}`;
  el.layoutBtn.title = now === "grid" ? "Switch to list view" : "Switch to grid view";
}

/// Set the layout of the view currently on screen.
///
/// The button and the pad shortcut both come here, so which of the two settings
/// they change is decided by where you are rather than by the caller — press it
/// on the console screen and the console screen changes, press it inside a
/// console and that changes.
export function setLayout(next) {
  const key = layoutKeyForView();
  state[key] = next;
  state.layout = next;
  localStorage.setItem(key === "layoutPlatforms" ? "layout.platforms" : "layout", next);
  paintLayoutButton(next);
  // Only a grid has anything to resize, on the consoles screen as much as
  // anywhere else.
  showZoom(next === "grid");

  // Redraw whatever is actually on screen.
  //
  // This used to redraw `state.rows` regardless of the current view, and
  // `state.rows` still holds the last console you opened. So pressing the
  // button on the consoles screen replaced it with that console's games —
  // which looks exactly like the button opened a console by itself.
  if (state.view === "platforms") {
    renderPlatforms(state.platforms);
    restorePlatformCursor();
    fitConsoleArt();
  } else if (state.rows.length) {
    renderRows(state.rows, state.view === "search");
  }
}

// Covers load only for cards near the viewport, batched — opening a
// 2,400-game platform must not fire 2,400 requests — and are let go again
// once a card is well away from it.
//
// The letting go is the whole of the memory problem. A cover is a few tens of
// kilobytes as a PNG and about 786 KB once decoded into a bitmap, and the
// version this replaces unobserved a card the moment its cover arrived: every
// image the list had ever drawn stayed decoded for as long as the list was on
// screen. Measured on 2026-08-20, the WebKit content process sat at 578 MB of
// a ~671 MB total, and that was all of it. See docs/handheld-frontend.md.
let coverObserver;
/// The second observer, at a much larger margin than the first. Two of them,
/// not one, because loading and releasing want different distances: a card
/// just off the top of the screen is one flick of the wheel from being looked
/// at again, and dropping its cover there would mean fetching and decoding it
/// again on the way back. The gap between the two margins is the hysteresis.
let coverReleaser;
let coverQueue = [];
let coverTimer;

/// How far off screen a card has to be before its cover is let go, and how
/// close it has to come before one is asked for.
///
/// In screenfuls rather than pixels. A fixed 1,600px was two screens on the
/// window this was written against and eight on a handheld, which is exactly
/// backwards: the small screen, with the least memory, hoarded the most.
///
/// The numbers matter more than they look. Every card inside the release
/// margin is holding a decoded picture, and a decoded picture is four bytes a
/// pixel however small it is drawn — 4.9 MB apiece for a 1280x960 miximage.
/// At 1,600px that was around a hundred and thirty of them. See `memory-footprint.md`.
///
/// The gap between the two is the hysteresis: a card just off the top is one
/// flick from being looked at again, and dropping its cover there means
/// fetching and decoding it twice.
const LOAD_SCREENS = 0.4;
const RELEASE_SCREENS = 1.0;

/// Those two as pixel margins for the observers, measured against the list as
/// it is now. Re-read whenever the observers are rebuilt, which is every time
/// the list is redrawn — so a resized window gets the right numbers without
/// anything listening for it.
function coverMargins() {
  // A floor, because the list is measured before it has been laid out and a
  // zero-height screen would mean loading nothing at all.
  const screen = Math.max(el.list?.clientHeight ?? 0, 200);
  return {
    load: `${Math.round(screen * LOAD_SCREENS)}px`,
    release: `${Math.round(screen * RELEASE_SCREENS)}px`,
  };
}

let coverErrorShown = false;

/// The placeholder a card is drawn with, and goes back to when its cover is
/// released: the first two letters of its name, and the star if it has one.
function placeholder(card) {
  // `data-name` already holds only the two letters the placeholder draws, and
  // is already escaped — it went through `escapeHtml` on the way into the
  // attribute. Escaping it again would turn an `&` into `&amp;amp;`.
  const star = card.dataset.fav === "1"
    ? `<span class="star" title="Starred — in one of your starred collections">★</span>`
    : "";
  return `<span class="ph">${card.dataset.name ?? ""}</span>${star}`;
}

/// Whether this card is holding a decoded picture, as opposed to the two
/// letters it is drawn with before one arrives.
///
/// Both shapes count, and that is the whole point of the function. The release
/// path was written when a cover was an `<img>` and went on testing for one
/// after `drawCover` started putting a `<canvas>` in — so from the day the
/// artwork was drawn at tile size until this was found, **nothing was ever
/// released**. Every cover a browse touched was held until its card left the
/// window, and on a console short enough not to be windowed at all — fifteen
/// of Frank's twenty-four are under `visible.THRESHOLD` — that means all of
/// them, for as long as the console stayed open.
///
/// The tests did not catch it because jsdom has no `createImageBitmap`, so
/// `fitted` bails and every cover in the suite is the `<img>` fallback. There
/// is a test now that stands a canvas up by hand.
function holdsPicture(art) {
  const first = art?.firstElementChild;
  return first?.tagName === "IMG" || first?.tagName === "CANVAS";
}

export function observeCovers() {
  const margins = coverMargins();
  forgetPendingCovers();
  coverObserver?.disconnect();
  coverReleaser?.disconnect();
  coverQueue = [];
  coverObserver = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        // Still observed, not unobserved: a card whose cover has been released
        // has to be able to ask for it again. `loaded` is what stops the same
        // card being queued twice.
        if (e.target.dataset.loaded === "1") continue;
        // A folder card is a .gcard with no data-id, and there are folders in
        // any console whose ROMs are filed in subdirectories. Without this
        // `Number(undefined)` is NaN, JSON.stringify writes it as null, and the
        // backend rejects the whole batch on `ids` -- so one folder on screen
        // meant not a single cover loaded for the forty cards around it.
        if (!e.target.dataset.id) continue;
        e.target.dataset.loaded = "1";
        coverQueue.push(Number(e.target.dataset.id));
      }
      clearTimeout(coverTimer);
      coverTimer = setTimeout(flushCovers, 80);
    },
    { root: el.list, rootMargin: margins.load }
  );
  coverReleaser = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) continue;
        const art = e.target.querySelector(".art");
        // Only if there is something to let go of. Putting the placeholder
        // back over a placeholder is a write to the page for no reason, and
        // this runs for every card that leaves the margin.
        if (!holdsPicture(art)) continue;
        art.innerHTML = placeholder(e.target);
        delete e.target.dataset.loaded;
      }
    },
    { root: el.list, rootMargin: margins.release }
  );
  for (const c of el.list.querySelectorAll(".gcard")) {
    coverObserver.observe(c);
    coverReleaser.observe(c);
  }
}

/// How many pictures may be in the air at once.
///
/// Forty `src`s were being set in a single pass, and a burst of `asset://`
/// requests that size wedges: measured 2026-08-22, sixty at once neither
/// loaded nor failed — no `onload`, no `onerror`, nothing in two minutes, and
/// the process sitting perfectly still. The same sixty one after another all
/// succeeded, every time. Six is a browser's own per-host limit and is plenty
/// to keep a scrolling grid fed.
const AT_ONCE = 6;

let waiting = [];
let inFlight = 0;

/// Put a cover on a card, as soon as there is room to ask for one.
///
/// The card is written exactly as it was before — the `<img>` goes straight
/// into the page and fills in when it arrives. The only thing that changed is
/// how many are allowed to be asking at once.
function showCover(art, url, star) {
  waiting.push({ art, url, star });
  pumpCovers();
}

/// How many are in the air right now. Exported so a test can say so, since
/// the whole point of this is a number that must never go above `AT_ONCE`.
export function coversInFlight() {
  return inFlight;
}

function pumpCovers() {
  while (inFlight < AT_ONCE && waiting.length) {
    const { art, url, star } = waiting.shift();
    if (!art.isConnected) continue;
    inFlight += 1;
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      inFlight -= 1;
      pumpCovers();
    };
    // A request that never answers must not hold its place for ever. That is
    // the wedge this throttle is for: if it happens again the grid carries on
    // with one picture missing rather than stopping.
    setTimeout(finish, 10000);
    drawCover(art, url, star).finally(finish);
  }
}

/// Redraw the console pictures at the size the tiles show them.
///
/// The console screen was the one place still loading artwork the old way, and
/// it is thirty-four pictures at once: measured 2026-08-24, it costs about
/// 44 MB more than the same screen with them gone. They are hardware renders,
/// often a thousand pixels wide, shown in a box a hundred and fifty points
/// across.
///
/// Sequential, and after the tiles are in the page, because a burst of asset
/// requests wedges — the reason `AT_ONCE` exists.
export async function fitConsoleArt(root = el.list) {
  for (const img of [...root.querySelectorAll("img[data-fit]")]) {
    const url = img.currentSrc || img.src;
    const box = img.parentElement ?? img;
    const [w, h] = boxSize(box);
    if (!url || !(w > 0)) continue;
    const canvas = await fitted(url, w, h || w);
    if (!canvas || !img.isConnected) continue;
    canvas.className = img.className;
    canvas.style.maxWidth = "100%";
    canvas.style.maxHeight = "100%";
    img.replaceWith(canvas);
  }
}

/// Put one cover on one card, drawn at the size the card shows it.
///
/// The card holds a canvas rather than an `<img>` because an `<img>` keeps the
/// picture decoded at the *file's* resolution — 10.9 MB for one of Frank's
/// miximages, against 0.24 MB for the tile it is drawn into. See
/// `fitpicture.js`. Nothing about the file on disk changes; the lightbox and
/// the full-size views still open the original.
async function drawCover(art, url, star) {
  const [w, h] = boxSize(art);
  const canvas = await fitted(url, w, h);
  if (!art.isConnected) return;
  if (canvas) {
    art.replaceChildren(canvas);
    if (star) art.insertAdjacentHTML("beforeend", star);
    return;
  }
  // Whatever went wrong, a picture drawn the old way beats no picture.
  await new Promise((done) => {
    const img = document.createElement("img");
    img.alt = "";
    img.addEventListener("load", done);
    img.addEventListener("error", done);
    art.replaceChildren(img);
    if (star) art.insertAdjacentHTML("beforeend", star);
    img.src = url;
  });
}

/// Drop anything still queued. Called when the list is redrawn: those cards
/// are gone, and the ones that replace them will ask again.
export function forgetPendingCovers() {
  waiting = [];
}

async function flushCovers() {
  const ids = coverQueue.splice(0, 40);
  if (!ids.length) return;
  // A measuring switch, never set in normal use: with no artwork at all, what
  // is left while scrolling is the list itself. It is the only way to tell a
  // picture problem from a document problem.
  if (globalThis.__MOOSE_FLAGS?.includes("no-covers")) return;
  try {
    for (const { id, cover } of await invoke("rom_covers", { ids })) {
      if (!cover) continue;
      const card = el.list.querySelector(`.gcard[data-id="${id}"]`);
      const art = card?.querySelector(".art");
      // Gone since the batch was asked for — released again, or the list
      // redrawn under it. Dropping it is right; the card will ask again.
      if (!art || card.dataset.loaded !== "1") continue;
      const star = card.dataset.fav === "1"
        ? `<span class="star" title="Starred — in one of your starred collections">★</span>`
        : "";
      // The star is kept. Replacing the whole of `.art` with the image took it
      // away, so a starred game lost its star the moment its cover arrived.
      showCover(art, convertFileSrc(cover), star);
    }
  } catch (e) {
    // Placeholders stay — a failed batch is not worth interrupting browsing —
    // but it is said once. Swallowing this entirely meant a grid that fetched
    // no artwork at all looked exactly like a server with no artwork on it.
    if (!coverErrorShown) {
      coverErrorShown = true;
      toast(`Cover art is not loading — ${e}`, 8000);
    }
  }
  if (coverQueue.length) setTimeout(flushCovers, 30);
}

/// Pick something out of this list at random and put the cursor on it.
///
/// The reason every frontend has one: nobody knows 2,506 arcade games, and
/// "scroll until something looks familiar" always lands in the same three
/// letters of the alphabet. It picks from what is *shown* — the same rows the
/// filters left — so "surprise me from the ones I have not played" is a
/// question you can actually ask.
///
/// It selects rather than launches. A button that starts a game you have never
/// heard of, with no way to see what it is first, is a button people press
/// once.
export function randomGame() {
  const rows = sorted(filtered(state.rows));
  if (!rows.length) return null;
  const at = Math.floor(Math.random() * rows.length);
  const pick = rows[at];
  selectRom(pick.id);
  // Windowed, a game picked at random out of 2,506 is almost never one of the
  // hundred or so on the page — which is the point of the button. Ask the
  // window for it, which scrolls there and draws the band around it.
  const win = windowedList();
  const node =
    (win && el.list.contains(win.container) ? win.reveal(at) : null) ??
    region("games")?.querySelector(`[data-id="${pick.id}"]`);
  node?.scrollIntoView({ block: "center", behavior: "smooth" });
  markSelected();
  toast(pick.name);
  return pick;
}

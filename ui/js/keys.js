// Keyboard navigation, driven by the configurable bindings in bindings.js.
//
// Works off whatever is currently rendered rather than a parallel model: the
// selectable elements are `.card` (platforms), `.gcard`/`.row` (games) and
// `.tcard` (themes), so one implementation serves every view.

import { el, state, trail, invoke } from "./state.js";
import { selectRom, setSidebar, play, playVideo, showPlatformInfo } from "./detail.js";
import {
  showPlatforms, setLayout, setZoom, openPlatform, scrollList, randomGame,
} from "./library.js";
import { escapeHtml, toast } from "./util.js";
import {
  closeLightbox, zoomLightbox, stepLightbox, isLightboxOpen, togglePlayback,
} from "./lightbox.js";
import { cyclePictures } from "./pictures.js";
import { openSortMenu, cycleOrder } from "./sort.js";
import { openFilterMenu } from "./filter.js";
import { actions, actionFor, keyLabelFor, padMap, padLabelFor, padLabel } from "./bindings.js";
import { captureKey, isCapturing, settingsOpen, closeSettings, toggleSettings } from "./settings.js";
import { cycleSection, resetSection } from "./tabs.js";
import { windowedList } from "./visible.js";

function items() {
  // Not the ones the page filter has hidden: the cursor stepping onto a card
  // that is not on screen looks like the cursor disappearing.
  return [...el.list.querySelectorAll(".card, .gcard, .row, .tcard")].filter(
    (n) => !n.classList.contains("filtered-out")
  );
}

// Where the cursor goes next is worked out in `src/gridnav.rs`, from where the
// cards actually landed rather than from a column count. The version that
// replaced measured the first row and moved by plus or minus that number with
// the result clamped into range, and three things fell out of it: Up on the
// top row clamped to index 0, so you jumped to the first card instead of
// staying put; Down on the last row clamped to the final card the same way;
// and Left/Right ran off the end of a row into the next one. Grouped search
// results made it worse, since each console section can have its own card
// shape and one column count no longer described the page.
//
// The whole table of moves comes over once per rebuild, not one question per
// keypress. Reading `offsetTop` out of the page forces a layout, and a held
// direction repeats nine times a second across 2,506 arcade cards — a cursor
// that only moves once a round trip has come back reads as an app thinking
// about it. Looking a move up in the table is synchronous, which is what a
// cursor has to be.
//
// `offsetTop`/`offsetLeft` rather than getBoundingClientRect: they are
// relative to the layout, so the map stays valid while scrolling and only
// needs rebuilding when the list itself changes.
let shape = null;
let table = null;

export function resetNav() {
  shape = null;
  table = null;
}
window.addEventListener("resize", resetNav);

/// Fetch the table if the page has changed under it.
///
/// One call in flight at a time. A held direction repeats every 110ms, and on
/// a list this long the fetch can outlast that — without the guard, every
/// press while the first was still out would start another, each re-reading
/// 2,506 positions out of the page.
let inFlight = null;

async function syncGeometry(nodes) {
  // A windowed list is a uniform grid, and a uniform grid needs no measuring:
  // where a card sits is `index / columns`, so the table comes from two
  // numbers. That is not a shortcut — most of the cards are not drawn, so most
  // of them have no position to read, and the cursor has to be able to move
  // through them anyway. See src/gridnav.rs.
  const win = windowedList();
  if (win && el.list.contains(win.container)) {
    const now = `rows:${win.total}:${win.columns}`;
    if (shape === now && table) return;
    table = await invoke("grid_uniform", { count: win.total, columns: win.columns });
    shape = now;
    return;
  }
  const now = `${nodes.length}:${el.list.clientWidth}`;
  if (shape === now && table) return;
  if (!inFlight) {
    // Read every position in one go. Interleaving reads with anything that
    // writes to the page would force a fresh layout per card.
    const cards = nodes.map((n) => [n.offsetTop, n.offsetLeft, n.offsetWidth]);
    inFlight = invoke("set_grid", { cards })
      .then((t) => {
        table = t;
        shape = now;
      })
      .finally(() => {
        inFlight = null;
      });
  }
  await inFlight;
}

/// Work the table out now, before anybody presses anything.
///
/// Called after a list is drawn, from a timer rather than inline: by then the
/// browser has laid the page out for its own paint, so reading 2,506 positions
/// costs a lookup each instead of forcing a layout — and the 120KB the table
/// weighs crosses while nothing is waiting on it. Without this the whole cost
/// lands on the first arrow press after every redraw, which is exactly when
/// somebody is watching.
export function primeNav() {
  setTimeout(() => {
    const nodes = items();
    if (nodes.length) syncGeometry(nodes).catch(() => {});
  }, 0);
}

/// Move the cursor.
///
/// A null entry in the table means stay put, which is the whole point of the
/// geometry: running off the top of a grid leaves you where you were rather
/// than jumping to the first card.
async function step(direction) {
  const win = windowedList();
  const windowed = win && el.list.contains(win.container);
  const nodes = items();
  if (!windowed && !nodes.length) return;
  await syncGeometry(nodes);
  if (!table) return;

  const at = windowed ? cursorIn(win) : nodes.findIndex((n) => n.classList.contains("sel"));
  // Nothing selected yet: any direction lands on the first card, so a press on
  // a freshly drawn grid always does something.
  const to = at < 0 ? table.first : table[direction]?.[at];
  if (to === null || to === undefined) return;
  // Windowed, the row it lands on may not be drawn — that is the whole point
  // — so the window is asked for it, which scrolls there and draws the band
  // around it.
  focusNode(windowed ? win.reveal(to) : nodes[to]);
}

/// Where the cursor is in a windowed list.
///
/// Found by the game it is on rather than by the `.sel` class, because the
/// card carrying that class is thrown away every time the band moves. The row
/// survives; the node does not.
function cursorIn(win) {
  if (state.selected === null || state.selected === undefined) return -1;
  for (let i = 0; i < win.total; i += 1) {
    if (win.at(i)?.id === state.selected) return i;
  }
  return -1;
}

function focusNode(node) {
  if (!node) return;
  // Games own their selection (it drives the preview); a console highlights
  // itself and puts its own facts in the pane, which is what that pane shows
  // on the Library screen.
  if (node.dataset.id) selectRom(Number(node.dataset.id));
  else {
    items().forEach((n) => n.classList.toggle("sel", n === node));
    if (node.dataset.slug) showPlatformInfo(node.dataset.slug);
  }
  // Centred, not `nearest`.
  //
  // `nearest` scrolls by the smallest amount that brings the cursor into view,
  // which means that once it reaches the bottom edge it stays welded there:
  // every further press moves the list up by exactly one row and the cursor
  // never leaves the last line. On a handheld that line is also the one under
  // the gesture bar, so the thing you are pointing at is the thing you can
  // least see.
  //
  // Centring keeps it in the middle of the list with context above and below,
  // which is what you want when you are driving with a thumbstick rather than
  // reading. The browser clamps at both ends, so the first and last few items
  // sit where they can rather than forcing empty space.
  node.scrollIntoView({ block: "center", inline: "nearest" });
}

/// Left/right stop at the ends of their row rather than spilling into the
/// neighbouring one, which is what made this feel random. Up/down keep the
/// column you were in, matched on horizontal center rather than index — so a
/// short last row, a row of differently-shaped cards, or the next console's
/// section all land somewhere that looks directly above or below where you
/// were. All of that is decided in `src/gridnav.rs`; this picks a column of
/// the table it produced.
const move = (direction) => step(direction);

const edge = (last) => {
  const win = windowedList();
  if (win && el.list.contains(win.container)) {
    if (!win.total) return;
    return focusNode(win.reveal(last ? win.total - 1 : 0));
  }
  const nodes = items();
  if (!nodes.length) return;
  focusNode(nodes[last ? nodes.length - 1 : 0]);
};

/// Index of the selected card, or -1. Lost when linear navigation was replaced
/// with the grid-aware version below, while `activate` kept calling it — so
/// every press of Enter or A threw a ReferenceError, inside a rAF callback
/// where nothing surfaces it.
function focusedIndex(nodes) {
  return nodes.findIndex((n) => n.classList.contains("sel"));
}

function activate() {
  // The drawn nodes are enough here: the cursor is always on screen, so the
  // card it is on is always one of them.
  const nodes = items();
  if (!nodes.length) return;
  // With nothing selected, focusedIndex is -1 and `nodes[-1]` is undefined, so
  // this used to return silently — pressing A on a freshly opened grid did
  // nothing at all until you nudged the stick first. Fall back to the first
  // item so the button always does something.
  const node = nodes[focusedIndex(nodes)] ?? nodes[0];
  // Through openPlatform, not showRoms: the controller should get the same
  // transition the mouse does, and a click handler is not the place to keep it.
  if (node.dataset.slug) openPlatform(node.dataset.slug, node);
  else if (node.dataset.id) invoke("rom_detail", { id: Number(node.dataset.id) }).then(play);
  // A folder opens rather than plays, and the card already knows which one.
  else if (node.dataset.dir !== undefined) node.click();
  // Collection cards already carry the navigation in their click handler,
  // which knows the group it came from; reuse it rather than duplicating.
  else if (node.dataset.cid || node.dataset.group) node.click();
  else if (node.dataset.repo) node.querySelector('button[data-act="icons"]')?.click();
}

function goBack() {
  if (state.view === "platforms") return;
  el.search.value = "";
  // Same one-level-at-a-time walk the Back button does, so a controller
  // behaves identically inside collections.
  const up = trail.pop();
  if (up) return up();
  // The section you are in, not always the platform grid — Back from inside a
  // collection belongs in My collections, not the library.
  resetSection();
}

function toggleHelp() {
  const existing = document.getElementById("shortcuts");
  if (existing) return existing.remove();

  // Both columns, side by side. This listed the keyboard and then said
  // "Controller" over a live readout of button indices — which answers "is this
  // pad working" and not "what does this button do", the question somebody
  // opens a help page to ask. An action with a button and no key belongs here
  // too, so the filter is either-or rather than keyboard-only.
  const bound = actions()
    .map((a) => ({
      ...a,
      key: keyLabelFor(a.id),
      pad: padLabelFor(a.id),
    }))
    // "—" and "unset" are what the two labels say for an action nobody bound.
    .filter((a) => a.key !== "—" || a.pad !== "unset");

  const box = document.createElement("div");
  box.id = "shortcuts";
  box.innerHTML = `<div class="sc-panel">
    <h3>Controls</h3>
    <table class="sc-table">
      <thead><tr><th>Action</th><th>Keyboard</th><th>Controller</th></tr></thead>
      <tbody>${bound
        .map(
          (a) => `<tr>
            <td>${escapeHtml(a.label)}</td>
            <td>${a.key === "—" ? "<span class=\"dim\">—</span>" : escapeHtml(a.key)}</td>
            <td>${a.pad === "unset" ? "<span class=\"dim\">—</span>" : escapeHtml(a.pad)}</td>
          </tr>`
        )
        .join("")}</tbody>
    </table>
    <p class="pad-readout">No controller detected.</p>

    <h3>What the marks mean</h3>
    <table class="sc-table sc-marks">
      <tbody>
        <tr><td><span class="mark here"><span class="icon icon-disk"></span></span></td>
            <td>On this machine — ready to play with the server off</td></tr>
        <tr><td><span class="mark away"><span class="icon icon-cloud"></span></span></td>
            <td>On the server — downloads when you play it</td></tr>
        <tr><td><span class="star">★</span></td>
            <td>Starred, in one of your starred collections</td></tr>
        <tr><td><span class="dot on"></span></td>
            <td>An emulator for this console is installed</td></tr>
        <tr><td><span class="dot"></span></td>
            <td>No emulator — games on this console will not start</td></tr>
        <tr><td><span class="bumper">LB</span></td>
            <td>The shoulder buttons move between these tabs</td></tr>
      </tbody>
    </table>

    <p>Rebind these in Settings · Esc to close</p></div>`;
  box.addEventListener("click", () => box.remove());
  document.body.appendChild(box);

  // A live readout of what the pad actually reports, here rather than only in
  // Settings. "Button X does nothing" is unanswerable without it: the bindings
  // are by index, and an index that is not the one you think it is looks
  // exactly like an app that ignores the button.
  const out = box.querySelector(".pad-readout");
  // Every 60ms rather than every animation frame. The body of this rebuilds a
  // paragraph of markup, and doing that at the display's refresh rate is text
  // layout 120 times a second for something a thumb changes a few times.
  let last = "";
  const tick = () => {
    if (!box.isConnected) return;
    setTimeout(tick, 60);
    const pad = (navigator.getGamepads?.() ?? []).find(Boolean);
    if (!pad) return;
    // The index alone is not the answer. A rebind clears whatever else held
    // that button by writing null, so a button can be reported as pressed and
    // still be bound to nothing — which looks exactly like the app ignoring it.
    const map = padMap();
    const down = pad.buttons
      .map((b, i) => (b.pressed ? i : null))
      .filter((i) => i !== null)
      .map((i) => {
        const action = map[i];
        const label = action
          ? actions().find((a) => a.id === action)?.label || action
          : "<em>not bound</em>";
        return `${i} → ${label}`;
      });
    const axes = pad.axes
      .map((v, i) => (Math.abs(v) > 0.35 ? `axis${i}: ${v.toFixed(2)}` : null))
      .filter(Boolean);
    const html =
      `<strong>${escapeHtml(pad.id)}</strong><br>` +
      `mapping: ${pad.mapping || "(none reported)"} · ${pad.buttons.length} buttons<br>` +
      `pressed: <strong>${down.length ? down.join(" · ") : "nothing"}</strong>` +
      (axes.length ? `<br>${axes.join(" · ")}` : "") +
      `<br><span class="hint">A button that says "not bound" is why it does ` +
      `nothing — Settings · Control · Reset controller puts the defaults back.</span>`;
    if (html !== last) {
      last = html;
      out.innerHTML = html;
    }
  };
  tick();
}

export const HANDLERS = {
  // Movement is grid-aware now, so none of these need a column count: a list
  // is simply a grid one card wide and behaves correctly without a special case.
  left: () => move("left"),
  right: () => move("right"),
  up: () => move("up"),
  down: () => move("down"),
  pageUp: () => move("page_up"),
  pageDown: () => move("page_down"),
  first: () => edge(false),
  last: () => edge(true),
  activate,
  back: goBack,
  back2: goBack,
  search: () => el.search.focus(),
  help: toggleHelp,
  settings: toggleSettings,
  prevSection: () => cycleSection(-1),
  nextSection: () => cycleSection(1),
  download: () => {
    if (state.selected) import("./actions.js").then((m) => m.download(state.selected, false));
  },
  favorite: () => {
    if (state.selected)
      import("./actions.js").then((m) => m.toggleFavorite(state.selected));
  },
  layout: () => {
    // Wherever the button is offered, the binding works too.
    if (el.layoutBtn.hidden) return;
    setLayout(state.layout === "grid" ? "list" : "grid");
  },
  sidebar: () => {
    if (state.view === "roms" || state.view === "search") setSidebar(!state.sidebar);
  },
  video: playVideo,
  // A press worth of scrolling, for a digital trigger or a key. A pad with
  // analogue triggers never comes through here — see gamepad.js, which reads
  // how far they are pulled and scrolls by that instead.
  scrollUp: () => scrollList(-120),
  scrollDown: () => scrollList(120),
  pictures: cyclePictures,
  sortMenu: () => openSortMenu(),
  filterMenu: () => openFilterMenu(),
  random: () => randomGame(),
  sortCycle: () => {
    const now = cycleOrder(1);
    if (now) toast(`Sorted by ${now}`);
  },
  // The triggers zoom whatever is in front of you: the picture on the stage
  // when the lightbox is open, the covers in the grid when it is not.
  zoomIn: () => (isLightboxOpen() ? zoomLightbox(1) : nudgeZoom(1)),
  zoomOut: () => (isLightboxOpen() ? zoomLightbox(-1) : nudgeZoom(-1)),
};

/// Step the cover size, staying inside the slider's own range.
///
/// Reads the bounds off the slider rather than repeating them, so the pad and
/// the slider can never disagree about how big "biggest" is.
function nudgeZoom(direction) {
  if (el.zoomWrap.hidden) return;
  const step = Number(el.zoom.step) || 10;
  const min = Number(el.zoom.min) || 90;
  const max = Number(el.zoom.max) || 300;
  setZoom(Math.min(max, Math.max(min, (state.zoom || min) + direction * step)));
}

/// Run an action by id, as the keyboard would. Shared with the gamepad.
export function runAction(id) {
  const handler = HANDLERS[id];
  if (handler) handler();
}

/// Answer Android's Back button.
///
/// `MainActivity` asks this before it quits, and takes `true` to mean the press
/// was consumed. Everything else — including this function not existing yet,
/// because the page is still loading — means there was nowhere to go and the
/// app should close, which is what Back at the top of an Android app does.
///
/// The press is delivered as an Escape rather than by calling `goBack`
/// directly. Escape already picks its way through the lightbox, then the help
/// panel, then an open text field, then the view stack, in that order, and a
/// second implementation of that order would drift from this one.
///
/// Installed on every platform. Nothing but the Android activity ever calls it.
export function installAndroidBack() {
  window.__androidBack = () => {
    const somewhereToGo =
      isLightboxOpen() ||
      settingsOpen() ||
      document.getElementById("shortcuts") !== null ||
      state.view !== "platforms";
    if (!somewhereToGo) return false;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    return true;
  };
}

export function installKeys() {
  el.settingsBtn?.addEventListener("click", toggleSettings);
  // Loaded on the press rather than at startup: the plan sheet pulls in the
  // conflict picker behind it, and neither is wanted by a session that never
  // syncs.
  el.syncBtn?.addEventListener("click", async () => {
    const { syncSaves } = await import("./savesync.js");
    syncSaves();
  });

  window.addEventListener("keydown", (ev) => {
    // Rebinding swallows everything, so any key can be assigned.
    if (isCapturing() && captureKey(ev)) return;

    // Settings, the way every desktop opens settings: Cmd+, on macOS and
    // Ctrl+, everywhere else. Checked before anything else, including the
    // lightbox and the text fields, because it is the one shortcut people
    // expect to work from wherever they happen to be.
    if (ev.key === "," && (ev.metaKey || ev.ctrlKey) && !ev.altKey) {
      ev.preventDefault();
      toggleSettings();
      return;
    }

    // The lightbox owns the keyboard while it is open — but not entirely. The
    // button that opened it has to close it again: pressing Y to start a video
    // and then having to reach for the mouse to stop it is the worst kind of
    // half-finished. Zoom works in there too, on the same keys.
    if (!el.lb.hidden) {
      // Space, on anything with a video on it. It is the one key every video
      // player on every platform agrees about, and the alternative was aiming
      // at a control bar that fades out. Not bindable, for the same reason:
      // this is muscle memory rather than a preference.
      if (ev.key === " " && togglePlayback()) {
        ev.preventDefault();
        return;
      }
      const action = actionFor(ev.key);
      if (action === "video" || action === "back") {
        ev.preventDefault();
        closeLightbox();
      } else if (action === "zoomIn" || action === "zoomOut") {
        ev.preventDefault();
        zoomLightbox(action === "zoomIn" ? 1 : -1);
      } else if (action === "left" || action === "right") {
        // Walk the reel: artwork, screenshots, cover, video. Owned here rather
        // than in the lightbox because these are bindable, and a second handler
        // reading the raw arrow keys would step twice for anyone who left them
        // as they are and not at all for anyone who moved them.
        ev.preventDefault();
        stepLightbox(action === "right" ? 1 : -1);
      }
      return;
    }

    if (settingsOpen()) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        closeSettings();
      }
      return;
    }

    const help = document.getElementById("shortcuts");
    if (help) {
      ev.preventDefault();
      return help.remove();
    }

    // Any other text field (the collection filter) owns the keyboard outright,
    // otherwise typing a name would fire single-key shortcuts.
    const focused = document.activeElement;
    if (focused !== el.search && focused?.tagName === "INPUT" && focused.type !== "range") {
      if (ev.key === "Escape") focused.blur();
      return;
    }

    // Typing in the search box: only Esc and Enter mean anything.
    if (focused === el.search) {
      if (ev.key === "Escape") {
        el.search.value = "";
        el.search.blur();
        goBack();
      } else if (ev.key === "Enter") {
        el.search.blur();
      }
      return;
    }

    // Leave platform and browser shortcuts alone, except ⌘F for search.
    if (ev.metaKey || ev.ctrlKey) {
      if (ev.key === "f") {
        ev.preventDefault();
        el.search.focus();
      }
      return;
    }

    const action = actionFor(ev.key);
    const handler = action && HANDLERS[action];
    if (!handler) return;
    ev.preventDefault();
    handler();
  });
}

// Entry point: wire the header controls and load the first view.

import { el, state, trail, invoke, listen, MOBILE } from "./state.js";
import { askDownload } from "./bulk.js";
import { askConfigPatch } from "./conflicts.js";
import { openSortMenu } from "./sort.js";
import { openFilterMenu } from "./filter.js";
import { installPageFilter } from "./pagefilter.js";
import { chooseMode, storedMode, shellMode, installColumnResizer } from "./shell.js";
import { human, toast, escapeHtml } from "./util.js";
import { showPlatforms, showRoms, runSearch, setLayout, setZoom, renderRows, randomGame,
  applyLayoutForView, layoutKeyForView } from "./library.js";
import { setSidebar, installDetailResizer } from "./detail.js";
import { installTabs, showSection, resetSection, activeSection } from "./tabs.js";
import { installKeys, installAndroidBack } from "./keys.js";
import { installGamepad } from "./gamepad.js";
import { loadBindings } from "./bindings.js";
import { loadListControls } from "./sort.js";
import { setFilters } from "./filter.js";
import { warmRefresh } from "./actions.js";
import { redrawCollections } from "./collections.js";
import { installAttractScreen } from "./attract-screen.js";
import {
  startBackdrop, stopBackdrop, applyBackdropSettings,
  applyStoredGlassTint, setGlassTint, setGlassStrength,
  setBackdropFps,
} from "./backdrop.js";

el.back.addEventListener("click", () => {
  el.search.value = "";
  // Collections are three levels deep, so step back through them rather than
  // dropping straight to the platform grid.
  const up = trail.pop();
  if (up) return up();
  // Back at the top of a section returns to that section, not always to the
  // library — the tab bar says where you are and it should stay true.
  resetSection();
});

// Settings runs in its own document and cannot touch this one, so the artwork
// choice comes back as an event. Redrawn from the backend rather than from
// what is on screen: the images themselves have changed, not their layout.
listen("art-changed", () => {
  if (state.view === "platforms") return;
  if (state.rows.length) renderRows(state.rows, state.view === "search");
});

// The console pictures, which are the other half: `art-changed` deliberately
// skips the console grid because game artwork is not what changed there. This
// is the one that redraws it.
// Chosen in the settings window, which cannot reach this document.
// The console screen and the inside of a console keep separate layouts, and
// Settings can set either. Applied only when it is the one on screen; the other
// is stored and takes effect when you go there.
listen("layout-view", async ({ payload }) => {
  const { view, value } = payload || {};
  const key = view === "platforms" ? "layoutPlatforms" : "layoutGames";
  state[key] = value;
  if (layoutKeyForView() !== key) return;
  setLayout(value);
});

listen("shell-mode", async ({ payload }) => {
  chooseMode(String(payload), { announce: false });
  // Redrawn from scratch: the console list has moved to a different element,
  // and everything on screen was laid out for the old arrangement.
  await showSection(activeSection(), { force: true });
});

listen("icons-changed", () => {
  if (state.view === "platforms") showPlatforms();
});

// The library index has just been rebuilt, in the settings window, which
// cannot reach this document. The grid is drawn from the cache, so whatever is
// on screen has to be drawn again or the games that just arrived are not
// there — which is how a sync that worked looked like a sync that had not run.
listen("library-synced", async () => {
  if (state.view === "roms" && state.platform) return showRoms(state.platform);
  if (state.view === "platforms") return showPlatforms();
});

// The layout pair in the header. Redrawn from scratch, as the settings
// dropdown's own change is: the console list moves to a different element and
// everything on screen was laid out for the arrangement being left.
el.viewSwitch?.addEventListener("click", async (ev) => {
  const want = ev.target.closest("[data-mode]")?.dataset.mode;
  if (!want || want === shellMode()) return;
  chooseMode(want);
  await showSection(activeSection(), { force: true });
});

el.sortBtn.addEventListener("click", () => openSortMenu(el.sortBtn));
el.filterBtn?.addEventListener("click", () => openFilterMenu(el.filterBtn));
el.randomBtn?.addEventListener("click", () => randomGame());

el.zoom.addEventListener("input", (e) => setZoom(Number(e.target.value)));

el.layoutBtn.addEventListener("click", () =>
  setLayout(state.layout === "grid" ? "list" : "grid")
);

el.sidebarBtn.addEventListener("click", () => setSidebar(!state.sidebar));

// Whatever is on screen is what gets taken: a console page downloads that
// console, a collection page that collection.
// This is an application, not a page. The native menu offers "Open Image in
// New Window", "Copy Image" and "Share" for a console icon, none of which mean
// anything here, and it comes up over anything the app puts in its place.
//
// Text fields keep theirs: cut, copy, paste and the spelling menu are the
// system's job and there is no reason to reimplement them.
window.addEventListener("contextmenu", (ev) => {
  if (ev.target.closest("input, textarea, [contenteditable]")) return;
  ev.preventDefault();
});

el.grabBtn.addEventListener("click", () =>
  askDownload(
    state.view === "collection-roms" && state.collection
      ? { collection: state.collection, name: state.collectionName }
      : { platform: state.platform }
  )
);

let searchTimer;
el.search.addEventListener("input", (e) => {
  clearTimeout(searchTimer);
  const v = e.target.value;
  searchTimer = setTimeout(() => runSearch(v), 200);
});

// The backdrop's controls are in the Settings window and its canvas is here, so
// the two have to talk. Without this, toggling it there rendered a shader into
// the settings panel and left the library untouched.
listen("glass-tint", ({ payload }) => setGlassTint(payload, { announce: false }));
listen("glass-strength", ({ payload }) => setGlassStrength(payload, { announce: false }));
// The collection cards live here; the control is in the other window.
//
// Redraw that one grid rather than reloading. `location.reload()` was the first
// attempt and it threw away the whole session — scroll position, the open
// console, the cursor — to change a picture.
listen("collection-art", () => {
  if (state.view === "collections") redrawCollections();
});

// The draw loop reads the rate every frame, so this only has to store it —
// there is nothing to restart, including when it was Off.
listen("backdrop-fps", ({ payload }) => {
  setBackdropFps(payload, { announce: false });
});

listen("backdrop-toggle", ({ payload }) => {
  if (payload) startBackdrop();
  else stopBackdrop();
});
listen("backdrop-settings", ({ payload }) => {
  // Apply, never re-save: saving emits, and this window would then answer its
  // own message. That round trip is what made the backdrop flicker while a
  // color was being dragged.
  applyBackdropSettings(payload);
});

/// Anything worth watching gets a speed and a size, not just a moving bar.
///
/// Below a few megabytes it is over before it can be read, so the threshold
/// keeps a bar from flashing up with numbers nobody had time to see. A disc
/// image is a different matter: minutes of waiting with no idea whether it is
/// moving at 2 MB/s or 40.
const PROGRESS_DETAIL_BYTES = 5 * 1024 * 1024;

/// Rate is measured from the first tick of *this* transfer rather than from
/// zero, so a resumed download reports what it is doing now instead of
/// crediting itself with the part it did not transfer.
let dlStart = null;

listen("download-progress", ({ payload }) => {
  const [id, done, total] = payload;
  if (state.selected !== id) return;

  const prog = document.getElementById("prog");
  if (prog) {
    prog.hidden = false;
    prog.max = total || 1;
    prog.value = done;
  }

  const label = document.getElementById("prog-text");
  if (!label) return;
  if (total < PROGRESS_DETAIL_BYTES) {
    label.hidden = true;
    return;
  }

  const now = performance.now();
  if (!dlStart || done < dlStart.done) dlStart = { at: now, done };
  const seconds = (now - dlStart.at) / 1000;
  const rate = seconds > 0.4 ? (done - dlStart.done) / seconds : 0;
  const left = rate > 0 ? (total - done) / rate : null;

  label.hidden = false;
  label.textContent =
    `${human(done)} of ${human(total)}` +
    ` · ${total ? Math.round((done / total) * 100) : 0}%` +
    (rate > 0 ? ` · ${human(rate)}/s` : "") +
    (left !== null && left > 2 ? ` · ${formatEta(left)} left` : "");

  // A finished transfer is reported as (1, 1); clear the baseline so the next
  // one measures itself rather than inheriting this one's start.
  if (done >= total) dlStart = null;
});

function formatEta(seconds) {
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const m = Math.floor(seconds / 60);
  return m < 60 ? `${m}m ${Math.round(seconds % 60)}s` : `${Math.floor(m / 60)}h ${m % 60}m`;
}

/// The server without the parts nobody reads. `https://` in front of a LAN
/// address is six characters saying nothing, and the trailing slash is noise.
function bareUrl(url) {
  return String(url ?? "")
    .replace(/^https?:\/\//, "")
    .replace(/\/+$/, "");
}

/// What the tag says when you point at it.
///
/// Our own panel rather than a `title` attribute: the tooltip took a second to
/// appear, went away on its own, and could not be styled or read from a sofa —
/// which is why the details in it were effectively invisible.
function statusCard(s) {
  const card = document.createElement("div");
  card.id = "status-card";
  card.hidden = true;
  card.innerHTML = `
    <div class="sc-row"><span>Server</span><strong>${escapeHtml(
      s.configured ? bareUrl(s.server) : "not configured"
    )}</strong></div>
    <div class="sc-row"><span>Games</span><strong>${s.roms_cached}</strong></div>
    <div class="sc-row"><span>Emulator</span><strong>${
      s.retroarch ? `RetroArch · ${s.cores_installed} cores` : "not found"
    }</strong></div>
    <div class="sc-row"><span>On disk</span><strong class="sc-disk">…</strong></div>
    <hr>
    <div class="sc-path"><span>Config</span><code>${escapeHtml(s.config_path)}</code></div>
    <div class="sc-path"><span>Games</span><code>${escapeHtml(s.roms_dir)}</code></div>
    <div class="sc-path"><span>Artwork</span><code>${escapeHtml(s.media_dir)}</code></div>
    <div class="sc-path"><span>Data</span><code>${escapeHtml(s.data_dir)}</code></div>
    <p>Everything downloaded lives there. Deleting that folder reclaims all of
      it.</p>`;
  document.body.appendChild(card);

  // Asked for separately, and not waited on. Measuring it means walking every
  // file in the library — on a handheld with the library on a card that is
  // seconds — and on Android a command that takes seconds freezes the page for
  // seconds, because the IPC there is served on the calling thread.
  invoke("disk_usage")
    .then((bytes) => {
      const cell = card.querySelector(".sc-disk");
      if (cell) cell.textContent = human(bytes);
    })
    .catch(() => {
      const cell = card.querySelector(".sc-disk");
      if (cell) cell.textContent = "unknown";
    });

  const place = () => {
    const at = el.status.getBoundingClientRect();
    card.hidden = false;
    const box = card.getBoundingClientRect();
    card.style.top = `${at.bottom + 6}px`;
    // Pinned to the tag's right edge, which is the window's: opened leftward
    // it would otherwise hang off the side it is nearest to.
    card.style.left = `${Math.max(8, at.right - box.width)}px`;
  };
  el.status.addEventListener("pointerenter", place);
  el.status.addEventListener("focus", place);
  for (const ev of ["pointerleave", "blur"]) {
    el.status.addEventListener(ev, () => (card.hidden = true));
  }
  // It follows the tag, so it cannot stay behind when the window moves under
  // it — a panel left on screen over the library reads as something broken.
  window.addEventListener("scroll", () => (card.hidden = true), true);
  el.status.tabIndex = 0;
}

(async function init() {
  try {
    const s = await invoke("status");
    // "not set up" and "server unreachable" look identical otherwise — both
    // give an empty library — so say which it is rather than leaving the user
    // to guess whether the app is broken.
    // One tag, not four.
    //
    // "server · 2506 roms · 41 cores · 210 GB on disk" is four facts across the
    // whole top-right corner, three of which do not change and none of which is
    // read more than once a week. The one that matters — which server this is
    // talking to, or that it is not — is a word or two, and the rest is a
    // pointer away.
    const server = !s.configured
      ? "no config.toml"
      : s.connected
        ? bareUrl(s.server)
        : "offline";
    el.status.textContent = server;
    el.status.dataset.state = !s.configured ? "unset" : s.connected ? "on" : "off";
    // Both of these are only otherwise discovered by pressing play and having
    // it fail, which is a poor way to learn the app was never configured.
    if (s.crowded_folder) {
      // Before anything is created, not after. The app is about to write a
      // library folder, a cache and a config beside itself, and doing that in
      // someone's Downloads is how a launcher earns a reputation for mess.
      showFolderWarning(s);
    } else if (!s.configured) {
      // Naming the exact path it looked at: "it cannot find my config" is
      // otherwise unanswerable, and the answer is rarely the directory the
      // user expected.
      toast(`No config.toml at ${s.config_path} — copy config.example.toml there`);
    } else if (!s.retroarch && !window.MooseAndroid?.retroArchPackage) {
      toast("RetroArch not found — set its location in Settings");
    } else if (window.MooseAndroid?.retroArchPackage && !androidRetroArch()) {
      // Android has no location to set, so the desktop wording would send
      // somebody to a setting that is not there. Either the app is installed
      // or it is not.
      toast("RetroArch is not installed — install it and it will be found");
    }
    statusCard(s);
  } catch (e) {
    el.status.textContent = "backend error";
  }

  /// Whether RetroArch is on this Android device.
  ///
  /// The backend cannot answer this. It looks for a folder, and on Android
  /// RetroArch is a package — so `s.retroarch` is always false there and the
  /// desktop warning fired on every launch, telling the user to set a location
  /// that does not exist while RetroArch sat installed on the device.
  function androidRetroArch() {
    try {
      return !!window.MooseAndroid.retroArchPackage();
    } catch {
      return false;
    }
  }
  // Off by default: it is a preference, and starting a GPU loop uninvited on
  // someone's machine is not a decision this app should make for them.
  // Before anything is drawn: which arrangement decides where things go.
  chooseMode(storedMode(), { announce: false });
  if (localStorage.getItem("backdrop") === "on") startBackdrop();
  // Covers the preview pane too: it is a card, and takes the same --tint.
  // The build number under the traffic lights. `hiddenTitle` took the window
  // title away, and with it the one place a screenshot said which build it was.
  invoke("versions")
    .then(([client]) => {
      const el = document.getElementById("build");
      if (el && client) el.textContent = `v${client}`;
    })
    .catch(() => {});
  applyStoredGlassTint();
  setZoom(state.zoom);
  // Not `setLayout`: nothing has been chosen yet, this only points the state
  // and the button at the setting the opening view uses.
  applyLayoutForView();
  setSidebar(state.sidebar);
  installTabs();
  // The interface tables, before anything that reads them: the sort and filter
  // menus, the header buttons, the keyboard and the pad all resolve through
  // the backend now, and a menu drawn before the answer arrives is an empty
  // menu.
  await Promise.all([
    loadBindings().catch((e) => console.warn("loading bindings:", e)),
    loadListControls()
      .then((c) => setFilters(c.filters))
      .catch((e) => console.warn("loading list controls:", e)),
  ]);
  await showSection("library", { force: true });
  installPageFilter();
  installDetailResizer();
  installColumnResizer();
  // Android: no top bar, and the tab row becomes the top of the app. A class
  // rather than inline styles so the stylesheet keeps every rule about it.
  if (MOBILE) {
    document.body.classList.add("mobile");
    // On the root element as well as the body, because one rule needs to reach
    // `html` itself: the page background. See `html.mobile.backdrop-on`.
    document.documentElement.classList.add("mobile");
  }
  installKeys();
  // Last, and above everything it covers. The counter is one number at window
  // level — no view has to know attract mode exists — so this is the only place
  // in the app that mentions it.
  installAttractScreen();
  // Answers the Android Back button. Inert everywhere else.
  installAndroidBack();
  installGamepad();
  // Measure the display now. It costs 24 animation frames, and taken on demand
  // that wait landed between pressing play and the game being asked for.
  warmRefresh();

  // Last, and only once the library is on screen: the config file still works
  // whatever it says, so this is never a reason to hold up the app starting.
  checkConfig();
})();

/// Offer to bring config.toml up to date.
///
/// Every setting this app has renamed still loads through a compatibility path,
/// which is deliberate and invisible — a config can go on holding a password
/// that is never sent, and nothing on screen says so. Asked at startup because
/// that is when the file was read, and only when something can be changed
/// without a decision.
async function checkConfig() {
  try {
    const findings = await invoke("config_findings");
    if (!Array.isArray(findings) || !findings.length) return;
    if (!(await askConfigPatch(findings))) return;
    toast(await invoke("config_patch"), 12000);
  } catch (e) {
    // A config that cannot be read is already reported by the status line.
    // Failing to *check* it is not worth a dialog of its own.
    console.warn("config check:", e);
  }
}

/// "You have put me somewhere I am going to make a mess of."
///
/// Shown once, before the first sync or download creates anything. Dismissable
/// rather than blocking: it is advice, and someone who meant to put the app
/// there should not have to argue with it.
function showFolderWarning(status) {
  if (localStorage.getItem("folderWarningSeen") === "yes") return;

  const box = document.createElement("div");
  box.id = "conflict-overlay";
  box.innerHTML = `<div class="conflict-box">
      <header><span class="icon icon-info-on"></span><h2>Give me my own folder</h2></header>
      <p class="lead">This app writes everything beside its own executable —
        downloaded games, cover art, the library index, your config.</p>
      <p class="why">Right now that is:<br><code>${status.data_dir}</code><br>
        which already has ${status.folder_entries} other item${
          status.folder_entries === 1 ? "" : "s"
        } in it.</p>
      <p class="note">Move the app into a folder of its own and everything it
        creates stays together — and deleting that one folder reclaims all of
        it. Nothing has been written yet.</p>
      <div class="sides">
        <button class="side" data-ack="ok"><span class="who">Got it</span>
          <span class="when">do not show this again</span></button>
        <button class="side" data-ack="later"><span class="who">Remind me</span>
          <span class="when">ask again next launch</span></button>
      </div>
    </div>`;

  box.addEventListener("click", (ev) => {
    const btn = ev.target.closest("[data-ack]");
    if (!btn) return;
    if (btn.dataset.ack === "ok") localStorage.setItem("folderWarningSeen", "yes");
    box.remove();
  });
  document.addEventListener(
    "keydown",
    (ev) => {
      if (ev.key === "Escape" && box.isConnected) box.remove();
    },
    { once: true }
  );
  document.body.appendChild(box);
  box.querySelector("[data-ack]").focus();
}

// The sidebar: artwork, metadata, and the play/download actions.

import { el, state, invoke, convertFileSrc, rememberRom, MOBILE } from "./state.js";
import { fitted, boxSize } from "./fitpicture.js";
import { tintFor } from "./tint.js";
import { human, escapeHtml, row, starBar, toast } from "./util.js";
import { openLightbox, setOpenHook } from "./lightbox.js";
import { showMenu } from "./menu.js";
import { shellMode } from "./shell.js";
import { deleteState } from "./states.js";
import { launch, download } from "./actions.js";
import { askDownload } from "./bulk.js";

/// A video source the webview will actually play.
///
/// Everywhere but Android that is the file itself. There, a `<video>` pointed at
/// the asset protocol never starts: it sits at `readyState 0` with the network
/// still "loading" and fires neither `loadeddata` nor `error`, so the player
/// opens on a black rectangle for ever. `fetch` on the very same URL answers
/// 206 with the right bytes and the right content type — Chromium's media
/// loader simply does not go through the same path for a custom scheme.
///
/// So the bytes are fetched and handed over as a blob. These are ES-DE preview
/// clips, a few megabytes each, which is a reasonable thing to hold for as long
/// as the player is open — and the alternative, sending it out to another app,
/// takes you out of the library to watch it.
///
/// The URL is revoked when the player closes; see `releaseVideo`.
let heldVideo = null;
async function playableVideo(path) {
  releaseVideo();
  if (!MOBILE) return path;
  try {
    const res = await fetch(convertFileSrc(path));
    if (!res.ok) return path;
    heldVideo = URL.createObjectURL(await res.blob());
    return heldVideo;
  } catch {
    // Fall back to the path: on a build where this works, it works.
    return path;
  }
}

/// Let go of the bytes held for the player.
export function releaseVideo() {
  if (!heldVideo) return;
  URL.revokeObjectURL(heldVideo);
  heldVideo = null;
}

/// Hand something to another app on the device, on Android.
///
/// Returns true when it was taken, so the caller can fall through to the
/// in-window behaviour everywhere else. Three things need this and each fails
/// in the webview for its own reason — see `openExternal` in MainActivity.kt.
function handedToAndroid(target, mime) {
  const bridge = window.MooseAndroid;
  if (!MOBILE || !bridge?.openExternal || !target) return false;
  const failed = bridge.openExternal(target, mime);
  if (failed) toast(failed, 6000);
  return true;
}


let slideTimer;
setOpenHook(() => clearInterval(slideTimer));

/// Which pane the toggle is talking about right now.
///
/// The console screen and a game list show different things in the same
/// column — a console's description, or a game's artwork — and they are
/// separate decisions. One flag for both meant hiding the console panel and
/// then opening a game turned it back on, because the game screen's own answer
/// overwrote it and there was nothing left to restore on the way back.
export function sidebarScope() {
  return state.view === "platforms" ? "platforms" : "games";
}

const SIDEBAR_KEY = (scope) => `sidebar.${scope}`;

/// What this level was last left at. Defaults to shown, as before.
export function sidebarWanted(scope = sidebarScope()) {
  const v = localStorage.getItem(SIDEBAR_KEY(scope));
  // Migrate the single old flag the first time each level is asked for, so
  // nobody's existing preference is silently reset.
  if (v === null) return localStorage.getItem("sidebar") !== "off";
  return v !== "off";
}

/// Re-read the flag for whichever level is on screen, without announcing it.
///
/// Called when the view changes: the pane follows the level you moved to
/// rather than carrying the last level's answer with it.
export function restoreSidebar() {
  // Empty it first. The pane keeps whatever was last written into it, so
  // stepping from a game back to the console list left a game's artwork and
  // buttons sitting there until something on the new level was selected —
  // reported as "the platform info tab sometimes retains the game info".
  // Cheap: whichever level you land on refills it immediately.
  el.detail.innerHTML = "";
  state.platformShown = null;
  setSidebar(sidebarWanted(), { remember: false });
}

/// Show/hide the pane, honouring both the toggle and the current view.
export function setSidebar(on, { remember = true } = {}) {
  state.sidebar = on;
  if (remember) localStorage.setItem(SIDEBAR_KEY(sidebarScope()), on ? "on" : "off");
  // Named for what it shows rather than for what pressing it does. "Hide info"
  // on a screen full of consoles does not say *which* info, and the icon
  // already carries the on/off state.
  el.sidebarBtn.querySelector("span:not(.icon)").textContent =
    sidebarScope() === "platforms" ? "Platform info" : "Game info";
  el.sidebarBtn.querySelector(".icon").className = `icon icon-info-${on ? "on" : "off"}`;
  // In Desk the preview is a column of the layout, so "show it" means show it
  // — on the console list too, where it holds whatever was last selected.
  // This used to insist on a game being selected, which disagreed with the
  // console screen's own line and left the column shut with no way back:
  // pressing Show info there did nothing at all, because nothing was selected
  // *to* show.
  //
  // In Sofa it slides over the list, so it is only meaningful where there is
  // something under the cursor.
  const allowed =
    shellMode() === "columns" ||
    // The console screen has something to show now — the console under the
    // cursor — so the pane is meaningful there too. It was not, which is why
    // pressing Show info on the Library screen did nothing at all.
    state.view === "platforms" ||
    ((state.view === "roms" || state.view === "search" || state.view === "collection-roms") &&
      state.selected !== null);
  el.detail.hidden = !(on && allowed);
}

/// The console the pane is showing, when it is showing a console rather than a
/// game. Kept so the emulator lookup can be skipped on a repeat.
let systemsCache = null;

/// Show a console in the preview, instead of nothing.
///
/// The Library screen had a preview pane that was always empty: there is no
/// game selected there, so the pane was hidden and the button that opens it
/// appeared to do nothing at all. A console is a thing with facts about it —
/// how many games, how many are here, which emulator runs it, which shader it
/// gets, whether that emulator is even installed — and every one of those was
/// only findable in Settings.
export async function showPlatformInfo(slug) {
  const p = (state.platforms ?? []).find((x) => x.slug === slug);
  if (!p) return;
  state.platformShown = slug;
  el.detail.hidden = !state.sidebar;
  if (el.detail.hidden) return;

  // `portrait` rather than `logo`: the grid's artwork is cycled by Select and
  // the pane must not change under you while you are reading it.
  const pic = p.portrait ?? p.logo;
  const logo = pic
    ? `<img class="pcover${p.portrait ? "" : (p.logo_wordmark ? " wordmark" : "")}" src="${convertFileSrc(pic)}" alt="" />`
    : "";
  // The same four things ES-DE's themes put on a system: who made it, when it
  // arrived, what kind of machine it was, and a line about what it was for.
  const facts = [
    p.manufacturer ? row("Made by", p.manufacturer) : "",
    p.released ? row("Released", String(p.released)) : "",
    p.hardware ? row("Type", p.hardware) : "",
  ].join("");
  el.detail.innerHTML = `
    <div class="scroll">
      ${logo}
      <h2>${escapeHtml(p.name)}</h2>
      <div class="sub">${escapeHtml(p.slug)}</div>
      ${p.blurb ? `<p class="summary">${escapeHtml(p.blurb)}</p>` : ""}
      <dl>
        ${facts}
        <dt>Games</dt><dd>${p.rom_count}</dd>
        <dt>Emulator</dt><dd class="pf-core">${p.playable ? "…" : "none installed"}</dd>
        <dt>Shader</dt><dd class="pf-shader">…</dd>
      </dl>
      ${
        p.playable
          ? ""
          : `<p class="warn">No core for this console is installed, so nothing here
               will start. Settings → Emulators installs one.</p>`
      }
      <div class="pinned">
        <button class="pf-grab"><span class="icon icon-download"></span><span>Take offline</span></button>
      </div>
    </div>`;
  el.detail.querySelector(".pf-grab")?.addEventListener("click", () =>
    askDownload({ platform: slug })
  );

  // The emulator and shader come from the same place the Emulators tab reads,
  // so the two cannot disagree. Asked once and kept: it is a list of
  // thirty-five rows that only changes when a setting does.
  try {
    systemsCache ??= await invoke("systems");
  } catch {
    return;
  }
  if (state.platformShown !== slug) return;
  const sys = systemsCache.find((x) => x.slug === slug);
  if (!sys) return;
  const label = (list, want) => list?.find((o) => o.core === want || o.key === want)?.label;
  const core = el.detail.querySelector(".pf-core");
  if (core && p.playable) {
    core.textContent = label(sys.emulators, sys.core) ?? sys.core ?? "default";
  }
  const shader = el.detail.querySelector(".pf-shader");
  if (shader) shader.textContent = label(sys.shaders, sys.shader) ?? sys.shader ?? "none";
}

/// The game the pane is showing, kept so the video button can build the same
/// set of media the artwork clicks do. `playVideo` is reachable from a key and
/// a pad as well as the button, so it cannot be handed the detail as an
/// argument.
let currentDetail = null;

export async function selectRom(id) {
  // The card the cursor is moving to, tagged so the browser can carry it into
  // the detail pane's cover rather than cross-fading two unrelated images.
  // Tagged before the class changes, since the old tag has to be cleared in the
  // same frame or two elements claim the same name and the transition is
  // skipped entirely.
  const card = el.list.querySelector(`[data-id="${id}"] .art img`);
  const previous = document.querySelector('[style*="view-transition-name: cover"]');
  if (previous) previous.style.viewTransitionName = "";
  if (card) card.style.viewTransitionName = "cover";

  state.selected = id;
  rememberRom(id);
  el.list.querySelectorAll(".row, .gcard").forEach((r) =>
    r.classList.toggle("sel", Number(r.dataset.id) === id)
  );

  const [d, cores, states] = await Promise.all([
    invoke("rom_detail", { id }),
    // Never fatal: a missing core list should not blank the whole pane.
    invoke("game_cores", { id }).catch(() => []),
    // Nor a missing shelf. Reading it means walking the state folders, which
    // is fine per game and would not be fine if it blanked the pane on a
    // machine with no RetroArch.
    invoke("game_states", { id }).catch(() => []),
  ]);
  // A later click may have superseded this one; do not paint a stale game.
  if (state.selected !== id) return;
  currentDetail = d;
  const shots = d.screenshots || [];

  // No slideshow and no player at the top any more: the miximage below already
  // carries a screenshot, the box and the logo in one picture, so cycling
  // through screenshots above it was the same information twice.
  const top = "";

  // The other half of the morph: same name as the tagged card art, so the
  // browser treats them as one element moving rather than two fading.
  // Always the box, with or without a picture in it.
  //
  // The pane used to grow to whatever height the artwork happened to be, so
  // moving the cursor down a list resized the box and shoved every line of text
  // under it to a new place — once when the old picture went and again when the
  // new one arrived. On a handheld, where the cursor moves a game at a time,
  // that reads as the whole pane flickering.
  //
  // A fixed box means the text below never moves: the space is reserved before
  // the image has loaded, and a game with no artwork at all leaves the same gap
  // rather than closing it up.
  const cover = `<div class="coverbox">${
    d.cover
      ? `<img class="cover" data-fit="1" style="view-transition-name: cover" src="${convertFileSrc(
          d.cover
        )}" alt="" />`
      : `<span class="cover-none" aria-hidden="true"></span>`
  }</div>`;

  // What this game has, as three small tags rather than a button here and two
  // links at the very bottom of the pane.
  //
  // The video one is a label, not a player: it says a video exists and waits
  // to be asked, because ES-DE starting one by itself after a pause leaves you
  // muting the emulator to browse and then having no sound when you actually
  // want to watch something. Y opens it, and it is the first thing in the
  // artwork reel below — so this does not need to be the way in, only the way
  // you find out there is one.
  const badges = [
    d.has_video
      ? `<button class="badge" id="playvid" title="Play it (Y)">
           <span class="icon icon-play"></span><span>Video</span></button>`
      : "",
    d.manual
      ? `<button class="badge" id="manual" title="Read the manual">
           <span class="icon icon-book"></span><span>Manual</span></button>`
      : "",
    // Not a play glyph: this one is a YouTube link and opens the browser. A
    // play triangle beside the video tag promises the same thing happens, and
    // it does not — one plays here, the other leaves the app entirely.
    d.youtube_id
      ? `<button class="badge" id="trailer" title="Watch the trailer on YouTube — opens your browser"
           data-yt="${escapeHtml(d.youtube_id)}">
           <span class="icon icon-external"></span><span>Trailer</span></button>`
      : "",
  ].join("");
  // Beside the title on Android rather than on a row of their own.
  //
  // Three buttons carrying a word each cost a whole line of a 426-point column
  // for three things you press occasionally. As icons they fit in the space the
  // title was not using, and the title is clamped to two lines anyway. The
  // words stay in the `title` attribute, which is what a long press shows.
  const video = badges ? `<div class="badges">${badges}</div>` : "";
  const titleRow = MOBILE
    ? `<div class="titlerow"><h2>${escapeHtml(d.name)}</h2>${video}</div>`
    : `<h2>${escapeHtml(d.name)}</h2>`;

  el.detail.hidden = !state.sidebar;
  // The glow around the selection takes the cover's own color. Started here
  // rather than awaited: the pane must not wait on a canvas read, so the
  // color arrives a frame or two later and simply transitions in.
  applyTint(id, d.cover ? convertFileSrc(d.cover) : null);
  // Wrapped so the browser can match the tagged card art to the pane's cover
  // and move one into the other, instead of replacing the pane wholesale.
  await withTransition(() => {
    el.detail.innerHTML = `
    <div class="scroll">
      ${titleRow}
      <div class="sub">${escapeHtml(d.fs_name)}</div>
      ${top}
      ${cover}
      ${MOBILE ? "" : video}
      ${d.rating ? starBar(d.rating) : ""}
      ${d.summary ? `<p class="summary">${escapeHtml(d.summary)}</p>` : ""}
      <dl>
        ${row("Released", d.release_year)}
        ${row("Genre", d.genres.join(", "))}
        ${row("Developer", d.companies.join(", "))}
        ${row("Series", d.franchises.join(", "))}
        ${row("Players", d.player_count)}
        ${row("Modes", d.game_modes.join(", "))}
        ${row("Region", d.regions.join(", "))}
        ${row("Also known as", d.alt_names.join(" · "))}
        <dt>Platform</dt><dd>${d.platform}</dd>
        <dt>Size</dt><dd>${human(d.size_bytes)}</dd>
        <dt>Core</dt><dd>${corePicker(cores, d)}</dd>
        <dt>Local</dt><dd>${d.downloaded ? "yes" : "no"}</dd>
      </dl>
      ${stateShelf(states)}
      ${artStrip(d)}
    </div>
    <div class="pinned">
      ${autofireRow(d)}
      <div class="actions">
        <button class="primary" id="play">${d.downloaded ? "Play" : "Download & Play"}</button>
        <button class="ghost" id="dl" ${d.downloaded ? "disabled" : ""}>Download</button>
      </div>
      <progress id="prog" hidden></progress>
      <div id="prog-text" hidden></div>
    </div>`;
  });

  wireArtwork(d);
  wireShelf(d);
  wireAutofire(d);
  fitPictures(el.detail);

  document.getElementById("play").addEventListener("click", () => play(d));
  document.getElementById("dl").addEventListener("click", () => download(d.id, false));
  wireCorePicker(d.id);
}

/// The save states this game has, as pictures you can start from.
///
/// A state is the only record of where you actually are in a game — it cannot
/// be downloaded again and nothing else in the app knows what is in one. They
/// were being synced with the server already, which meant the app knew about
/// them and never showed one: the only way back into a state was to launch the
/// game, open RetroArch's menu, and guess at a slot number.
///
/// The picture is the point. RetroArch saves the frame beside the state, and
/// "the cave with the two doors" is a thing you recognize instantly where
/// "slot 3" is a thing you have to remember.
function stateShelf(states) {
  if (!states.length) return "";
  return `
    <div class="shelf">
      <div class="shelf-head">Save states</div>
      <div class="shelf-strip">
        ${states
          .map(
            (s) => `
          <button class="state ${s.resumable ? "" : "noresume"}"
            data-slot="${escapeHtml(s.slot)}"
            title="${s.resumable ? `Start from ${escapeHtml(s.label)}` : "RetroArch loads this one itself when you next play — right-click to delete it"}">
            ${
              s.thumb
                ? `<img src="${convertFileSrc(s.thumb)}" alt="" loading="lazy" />`
                : `<span class="state-blank">${escapeHtml(s.slot === "auto" ? "auto" : s.slot)}</span>`
            }
            <span class="state-label">${escapeHtml(s.label)}</span>
            <span class="state-when">${escapeHtml(s.when ?? "")}</span>
          </button>`
          )
          .join("")}
      </div>
    </div>`;
}

function wireShelf(d) {
  for (const btn of document.querySelectorAll(".state")) {
    // `noresume` rather than `disabled`. A disabled button fires no mouse
    // events at all — not click, not contextmenu — so marking the autosave
    // disabled also made it impossible to right-click, which is the only way
    // to delete one. It is grayed the same way and simply does not launch.
    if (!btn.classList.contains("noresume")) {
      btn.addEventListener("click", () => {
        // Same launch path as the Play button, so a state that needs a core
        // fetched, a BIOS file, or a save sync gets all of it.
        launch(d.id, { entrySlot: Number(btn.dataset.slot) });
      });
    }
    // Right-click, including on the autosave: it cannot be started from, but
    // it is a file like any other and clearing it out is a reasonable thing to
    // want.
    btn.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      stateMenu(d, btn, e.clientX, e.clientY);
    });
  }
}

/// Where the rapid fire lives, for games that can have it.
///
/// In the pinned strip directly above Play, rather than in Settings: this is a
/// thing you change *about the run you are about to start* — set the rate,
/// play a level, decide it wants to be slower — and a control three windows
/// away turns that into a trip. It does not scroll with the artwork for the same
/// reason.
///
/// Absent entirely for games it does not apply to, which is most of them.
/// The three the backend knows. Anything else — a value from a newer build, a
/// hand-edited config, a field that arrived as something unexpected — draws no
/// row at all rather than an empty one, because a control with nothing
/// selected is worse than no control.
const AUTOFIRE_MODES = ["off", "lb", "rb"];

function autofireRow(d) {
  // Games this does not apply to get nothing: it is only the arcade and Neo
  // Geo shooters, which is 879 of 2,668 arcade games and none of the consoles.
  if (!AUTOFIRE_MODES.includes(d.autofire)) return "";
  const opt = (key, label, title) =>
    `<button class="af ${d.autofire === key ? "on" : ""}" data-af="${key}"
       title="${escapeHtml(title)}">${escapeHtml(label)}</button>`;
  // Always here, including next to "Off". Hiding it meant the row changed
  // width as you moved between the three, and the rate is a thing you may want
  // to set before switching on rather than after — turning it on and finding
  // it too fast, then turning it off to slow it down, is a loop with no exit.
  const rate = `<span class="af-rate">
      <button class="af-step" data-hz="-1" title="Slower">−</button>
      <span class="af-hz">${d.autofire_hz ?? 6} Hz</span>
      <button class="af-step" data-hz="1" title="Faster">+</button>
    </span>`;
  return `
    <div class="autofire-row" title="Applies to every arcade shooter, not only this game">
      <span class="af-label">Rapid fire</span>
      ${opt("off", "Off", "The buttons behave as the cabinet did")}
      ${opt("lb", "Hold LB", "Hold LB on its own and the game fires at this rate. Do not hold the fire button as well — a real press overrides the repeat.")}
      ${opt("rb", "Hold RB", "The same on the other shoulder, for when your left hand is busy")}
      ${rate}
    </div>`;
}

/// Redraw just this row, in place.
///
/// Rebuilding the whole pane instead sent the reader back to the top of it,
/// which for a control that lives at the bottom means every press throws away
/// where you were. Nothing else on the pane depends on these two values, so
/// nothing else needs to be touched.
function repaintAutofire(d) {
  const row = document.querySelector(".autofire-row");
  if (!row) return;
  const fresh = document.createElement("div");
  fresh.innerHTML = autofireRow(d);
  const next = fresh.firstElementChild;
  if (!next) return;
  row.replaceWith(next);
  wireAutofire(d);
}

function wireAutofire(d) {
  for (const step of document.querySelectorAll(".autofire-row .af-step")) {
    step.addEventListener("click", async () => {
      // Clamped here as well as in the backend, so the number on screen never
      // shows something that was not stored.
      const want = Math.min(30, Math.max(1, (d.autofire_hz ?? 6) + Number(step.dataset.hz)));
      if (want === d.autofire_hz) return;
      try {
        // For this run of the app, not for the file. Five taps to go from six
        // to eleven would be five rewrites of config.toml, and a file
        // rewritten that often is one that eventually gets caught half
        // written. The number in Settings is the one you start with.
        await invoke("set_autofire_hz", { hz: want });
      } catch (e) {
        return toast(`Could not save — ${e}`, 8000);
      }
      d.autofire_hz = want;
      repaintAutofire(d);
    });
  }

  for (const btn of document.querySelectorAll(".autofire-row .af")) {
    btn.addEventListener("click", async () => {
      const want = btn.dataset.af;
      try {
        await invoke("set_config_field", { field: "autofire", value: want });
      } catch (e) {
        return toast(`Could not save — ${e}`, 8000);
      }
      d.autofire = want;
      repaintAutofire(d);
      toast(want === "off" ? "Rapid fire off" : `Rapid fire on ${want.toUpperCase()}`);
    });
  }
}

/// The right-click menu on a save state.
function stateMenu(d, btn, x, y) {
  const slot = btn.dataset.slot;
  showMenu(
    [
      {
        label: "Delete this state",
        danger: true,
        run: async () => {
          const gone = await deleteState(d.id, {
            slot,
            label: btn.querySelector(".state-label")?.textContent?.trim(),
          });
          // Redrawn from the backend rather than by removing the button here:
          // the shelf is what the folder says it is, not what this page
          // remembers.
          if (gone) await selectRom(d.id);
        },
      },
    ],
    x,
    y
  );
}

/// A dropdown of the cores this game can run under.
///
/// Per-game rather than per-platform because arcade romsets are mixed: the
/// platform default is a best guess, and individual games need to escape it.
function corePicker(cores, d) {
  if (!cores.length) {
    return d.core_label ? escapeHtml(d.core_label) : "<em>none installed</em>";
  }
  const pinned = cores.some((c) => c.pinned);
  return `<select id="core-pick" title="Core used to launch this game">
      <option value=""${pinned ? "" : " selected"}>Platform default</option>
      ${cores
        .map(
          (c) => `<option value="${escapeHtml(c.core)}"${c.pinned ? " selected" : ""}>
            ${escapeHtml(c.label)}${c.installed ? "" : " (not installed)"}${
              c.current && !c.pinned ? " — default" : ""
            }</option>`
        )
        .join("")}
    </select>`;
}

function wireCorePicker(id) {
  const pick = document.getElementById("core-pick");
  if (!pick) return;
  pick.addEventListener("change", async () => {
    try {
      const msg = await invoke("set_game_core", { id, core: pick.value });
      toast(msg);
    } catch (e) {
      toast(String(e));
    }
  });
}

/// Human labels for ES-DE media types, in the order worth showing.
const ART_ORDER = [
  ["miximages", "Mix"],
  ["3dboxes", "3D box"],
  ["backcovers", "Back"],
  ["titlescreens", "Title"],
  ["marquees", "Marquee"],
  ["physicalmedia", "Cart/disc"],
  ["fanart", "Fan art"],
];

/// Thumbnail row of everything ES-DE has for this game beyond the cover.
function artStrip(d) {
  const items = ART_ORDER.filter(([k]) => d.art && d.art[k]);
  if (!items.length && !d.has_video) return "";
  // The video first, the way ES-DE lays a game out: it is the thing you look
  // at, and it used to be a button somewhere else entirely, which left the
  // arrows walking a reel that started at the box art and never reached it.
  // No thumbnail because the file is not here yet — it is fetched when asked
  // for, and a tile that has to download 30MB to draw itself is not a
  // thumbnail.
  const vid = d.has_video
    ? `<figure data-art="video" class="vidtile" title="Gameplay video">
         <div class="vidthumb"><span class="icon icon-play"></span></div>
         <figcaption>Video</figcaption>
       </figure>`
    : "";
  return `<div class="artstrip">${vid}${items
    .map(
      ([k, label]) =>
        `<figure data-art="${k}" title="${label}">
           <img data-fit="1" src="${convertFileSrc(d.art[k])}" alt="${label}" loading="lazy" />
           <figcaption>${label}</figcaption>
         </figure>`
    )
    .join("")}</div>`;
}

/// Play the selected game's video, fetching it first if it is not here yet.
///
/// Exported because the controller reaches it too — this is the button ES-DE
/// buries in a sidebar, and burying it again behind a mouse would miss the
/// point.
export async function playVideo() {
  const id = state.selected;
  if (id === null) return;
  // The game is the authority on whether there is a video, not the presence of
  // a button. Y has to work whether or not the tag is on screen — it is a tag
  // now, and the pane it lives in can be hidden.
  if (!currentDetail?.has_video) return;
  const btn = document.getElementById("playvid");
  if (btn?.dataset.busy) return;

  if (btn) btn.dataset.busy = "1";
  const label = btn?.querySelector("span:not(.icon)");
  const was = label?.textContent;
  if (label) label.textContent = "Fetching…";
  try {
    const path = await invoke("game_video", { id });
    // Still the same game? A slow fetch must not open a video over whatever
    // was scrolled to in the meantime.
    if (state.selected !== id) return;
    // The whole reel, not just the video: the arrows are meant to walk from a
    // video into the artwork and back, which they cannot do in a set of one.
    const media = mediaSet(currentDetail ?? {}, await playableVideo(path));
    const at = media.findIndex((m) => m.id === "video");
    openLightbox(media, at < 0 ? 0 : at);
  } catch (e) {
    toast(String(e), 6000);
  } finally {
    if (btn) delete btn.dataset.busy;
    if (label && was) label.textContent = was;
  }
}

/// Everything this game has, as one set the arrows can walk through.
///
/// Built here rather than in the lightbox because this is where the list of
/// artwork kinds lives, and it has to stay in the order the strip draws them —
/// stepping right should land on the picture to the right.
///
/// Each thing used to open on its own. Clicking the cart art gave you the cart
/// art and nothing else, and pressing Y gave you the video and nothing else, so
/// the arrow keys had a set of one to walk through and appeared not to work.
/// ES-DE treats a game's media as one reel and so does this now.
///
/// `videoSrc` arrives separately: the video is fetched on demand, so its path
/// is not known until someone asks for it.
function mediaSet(d, videoSrc = null) {
  const items = [];
  // First, matching the strip below the details and matching ES-DE. It used to
  // be last, so "one more right" from the end of a dozen pictures was the
  // video and there was no way to reach it from the left at all.
  if (videoSrc) {
    // Already a URL when it is a blob — see `playableVideo`. `convertFileSrc`
    // on a `blob:` string would make a path out of it.
    const src = videoSrc.startsWith("blob:") ? videoSrc : convertFileSrc(videoSrc);
    items.push({ src, kind: "video", caption: "Gameplay", id: "video" });
  }
  for (const [kind, label] of ART_ORDER) {
    if (d.art?.[kind]) {
      items.push({ src: convertFileSrc(d.art[kind]), kind: "image", caption: label, id: kind });
    }
  }
  (d.screenshots || []).forEach((sh, i) =>
    items.push({
      src: convertFileSrc(sh),
      kind: "image",
      caption: `Screenshot ${i + 1}`,
      id: `screenshot-${i}`,
    })
  );
  if (d.cover) {
    items.push({ src: convertFileSrc(d.cover), kind: "image", caption: "Cover", id: "cover" });
  }
  return items;
}

/// Clicking artwork opens it full size, starting at what was clicked.
function wireArtwork(d) {
  const media = mediaSet(d);
  const openAt = (pred) => () => {
    const i = media.findIndex(pred);
    openLightbox(media, i < 0 ? 0 : i);
  };

  el.detail.querySelector(".shots")?.addEventListener("click", (ev) => {
    // Leave the slideshow's own arrows and dots alone.
    if (ev.target.closest(".nav, .dots")) return;
    const shown = el.detail.querySelector(".shots img.on");
    const idx = [...el.detail.querySelectorAll(".shots img")].indexOf(shown);
    openLightbox(media, Math.max(media.findIndex((m) => m.id === `screenshot-${idx}`), 0));
  });
  el.detail.querySelector("img.cover")?.addEventListener("click", openAt((m) => m.id === "cover"));
  document.getElementById("playvid")?.addEventListener("click", playVideo);
  el.detail.querySelectorAll(".artstrip figure").forEach((fig) =>
    // The video tile has no picture behind it — the file is fetched when it is
    // asked for — so it goes the same way Y does rather than through the reel,
    // which has no video in it until that fetch has happened.
    fig.addEventListener(
      "click",
      fig.dataset.art === "video" ? playVideo : openAt((m) => m.id === fig.dataset.art)
    )
  );
  document.getElementById("manual")?.addEventListener("click", () => {
    // No PDF viewer in Android's webview, so the lightbox would open on a blank
    // sheet. Out to whatever reads PDFs on the device instead.
    if (handedToAndroid(d.manual, "application/pdf")) return;
    openLightbox([{ src: convertFileSrc(d.manual), kind: "pdf", caption: "Manual" }], 0);
  });
  // Out to the browser rather than into this window. It was an <a target=
  // "_blank">, which in a webview is a navigation: YouTube would have replaced
  // the library, with no address bar and no way back.
  document.getElementById("trailer")?.addEventListener("click", (ev) => {
    const yt = ev.currentTarget.dataset.yt;
    const url = `https://www.youtube.com/watch?v=${encodeURIComponent(yt)}`;
    // `open_link` runs a desktop command and there is not one here: on Android
    // it fails with "No such file or directory (os error 2)", which is it
    // looking for `xdg-open`.
    if (handedToAndroid(url, "")) return;
    invoke("open_link", { url }).catch((e) => toast(`Could not open the trailer — ${e}`));
  });
}

function startSlideshow(count) {
  clearInterval(slideTimer);
  const box = document.getElementById("shots");
  if (!box) return;
  const imgs = [...box.querySelectorAll("img")];
  const dots = [...box.querySelectorAll(".dots span")];
  let i = 0;

  const show = (n) => {
    i = (n + count) % count;
    imgs.forEach((im, k) => im.classList.toggle("on", k === i));
    dots.forEach((dt, k) => dt.classList.toggle("on", k === i));
  };
  const auto = () => {
    clearInterval(slideTimer);
    slideTimer = setInterval(() => show(i + 1), 3500);
  };

  box.querySelector(".prev")?.addEventListener("click", () => { show(i - 1); auto(); });
  box.querySelector(".next")?.addEventListener("click", () => { show(i + 1); auto(); });
  dots.forEach((dt, k) => dt.addEventListener("click", () => { show(k); auto(); }));
  // Pause while the pointer is over the image, so it can be studied.
  box.addEventListener("mouseenter", () => clearInterval(slideTimer));
  box.addEventListener("mouseleave", auto);
  auto();
}

/// Enter / primary button: play, downloading first if needed.
export async function play(d) {
  if (d.downloaded) return launch(d.id);
  return download(d.id, true);
}

/// A drag handle between the list and the detail pane.
///
/// The pane was a fixed 360px, which cropped cover art and wrapped most
/// metadata lines. It is wider by default now and the width is remembered, so
/// this is set once rather than every session.
export function installDetailResizer() {
  const grip = document.createElement("div");
  grip.id = "detail-grip";
  grip.setAttribute("role", "separator");
  grip.setAttribute("aria-orientation", "vertical");
  grip.title = "Drag to resize";
  el.detail.parentNode.insertBefore(grip, el.detail);

  const saved = Number(localStorage.getItem("detailWidth"));
  if (saved) applyWidth(saved);

  let startX = 0;
  let startW = 0;

  const onMove = (ev) => {
    // Dragging left widens the pane: it grows from its left edge.
    applyWidth(startW + (startX - ev.clientX));
  };
  const onUp = () => {
    document.removeEventListener("pointermove", onMove);
    document.removeEventListener("pointerup", onUp);
    grip.classList.remove("dragging");
    document.body.classList.remove("resizing-detail");
    localStorage.setItem("detailWidth", String(el.detail.getBoundingClientRect().width | 0));
  };

  grip.addEventListener("pointerdown", (ev) => {
    ev.preventDefault();
    startX = ev.clientX;
    startW = el.detail.getBoundingClientRect().width;
    grip.classList.add("dragging");
    document.body.classList.add("resizing-detail");
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
  });

  // Double-click resets, so a pane dragged off screen is recoverable without
  // clearing storage by hand.
  grip.addEventListener("dblclick", () => {
    document.documentElement.style.removeProperty("--detail-w");
    localStorage.removeItem("detailWidth");
  });
}

function applyWidth(px) {
  // A narrow screen has no room for a remembered width.
  //
  // This writes an *inline* custom property on the root element, and an inline
  // property beats any stylesheet — so a width dragged out on a 27-inch
  // display followed the app onto a 833-point handheld and sat there,
  // untouched by the media query written for exactly that case. On anything
  // this narrow the stylesheet is the better judge, so the inline value is
  // removed and it decides.
  if (window.innerWidth <= 1100) {
    document.documentElement.style.removeProperty("--detail-w");
    return;
  }
  // Clamped in code as well as in CSS: the CSS bound stops it rendering wrong,
  // this stops a nonsense value being written to storage.
  const max = Math.min(window.innerWidth * 0.7, 900);
  const clamped = Math.max(300, Math.min(px, max));
  document.documentElement.style.setProperty("--detail-w", `${clamped}px`);
}

/// Paint the selection in the colors of the game's own box art.
///
/// Set on the two elements that show a selection — the pane and the selected
/// card or row — rather than on the document, so nothing else in the window
/// shifts color and there is nothing to unset when the selection moves.
///
/// Guarded by the id: a fast scroll starts one of these per game and they can
/// finish out of order, so a slow read for a game you have already scrolled
/// past must not repaint the one you are now on.
async function applyTint(id, url) {
  const color = await tintFor(url);
  if (state.selected !== id) return;

  const targets = [el.detail, el.list.querySelector(`[data-id="${id}"]`)];
  for (const node of targets) {
    if (!node) continue;
    if (color) node.style.setProperty("--pick", color);
    else node.style.removeProperty("--pick");
  }
}

/// Scroll the detail pane, for the controller's right stick.
///
/// The left stick moves the cursor through the list; the right one reads the
/// pane next to it, which is otherwise unreachable without a mouse.
/// Run `fn` inside a view transition when the browser has one.
///
/// The API is Chromium-and-newer-WebKit; where it is missing this is a plain
/// call, so the app behaves identically and simply does not animate. Nothing
/// downstream may depend on the transition having happened.
export function withTransition(fn) {
  const start = document.startViewTransition?.bind(document);
  if (!start || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    return Promise.resolve(fn());
  }
  return start(fn).finished.catch(() => {});
}

export function scrollDetail(amount) {
  const pane = el.detail?.querySelector(".scroll");
  if (!pane || el.detail.hidden) return false;
  pane.scrollTop += amount;
  return true;
}


/// Redraw every `data-fit` picture in `root` at the size it is shown.
///
/// The same trade as the grid: an `<img>` keeps the picture decoded at the
/// file's resolution however small it is drawn, and this pane shows one large
/// cover plus a gallery of every art type a game has — a dozen 1,280x960 files
/// at once on one that is well scraped. The file on disk is not touched, and
/// the lightbox still opens the original at full size.
///
/// Anything that will not convert is left exactly as it was.
async function fitPictures(root) {
  for (const img of [...root.querySelectorAll("img[data-fit]")]) {
    const url = img.currentSrc || img.src;
    if (!url) continue;
    // Measured against the box it is stretched to fill, not its own size.
    const box = img.parentElement ?? img;
    const [boxW, boxH] = boxSize(box);
    const width = boxW || img.clientWidth;
    if (!width) continue;
    // Inside a box of its own — the cover — the height is a real bound and has
    // to be respected, or the picture decides the pane's height and every game
    // is a different shape. Everywhere else the width is what constrains these,
    // so twice the width stands in for "no limit".
    const boxed = box.classList?.contains("coverbox");
    const canvas = await fitted(url, width, boxed && boxH ? boxH : width * 2);
    if (!canvas || !img.isConnected) continue;
    canvas.className = img.className;
    canvas.style.cssText = img.style.cssText;
    if (!boxed) {
      canvas.style.width = "100%";
      canvas.style.height = "auto";
    }
    // In a box, `fitted` has already sized it to fit and the stylesheet centres
    // it; overriding to 100% x auto here is what stretched it back out.
    img.replaceWith(canvas);
  }
}

// Playing a game in the page.
//
// The third way to start a game, beside `launch_rom` on the desktop and
// `launchAndroid` on the Thor. It exists because the web UI is the desktop UI
// served from a machine in another room: `launch_rom` would spawn RetroArch
// onto a monitor nobody is looking at, so the service refuses it, and the Play
// button had nothing left to do.
//
// EmulatorJS is vendored, not fetched from a CDN — see
// `scripts/fetch-emulatorjs.sh`. The service serves it from `/emulatorjs/`, so
// this works with the internet down, which is the point of a LAN library.
//
// Saves go through `browser-saves.js`, which goes through the same negotiate
// every other device uses. Measured first, in a real browser: EmulatorJS's own
// IndexedDB copy did not survive a reload -- 32 of 8192 bytes -- so the browser
// is not somewhere a save can be left, and syncing it is what makes playing
// here worth anything rather than a convenience.

import { browserPlay, shouldWarn } from "./ejs-systems.js";
import { presetsFor, chosenShader, rememberShader } from "./ejs-shaders.js";
import {
  syncOne,
  serverStates,
  pushState,
  pullState,
  localSaveIn,
  localSaveOut,
} from "./browser-saves.js";
import { ejsUrls } from "./ejs-base.js";
import { invoke } from "./state.js";
import { toast } from "./util.js";

/// Where EmulatorJS and the ROM come from. Two answers -- this service over
/// HTTP, or the desktop's own URI scheme -- so it is resolved per play rather
/// than being a constant. See `ejs-base.js`.
let urls = null;

/// Two frames, so an in-flight view transition is over before the DOM changes.
///
/// Raced against a timer, because `requestAnimationFrame` does not fire in a
/// window that is not being rendered -- a backgrounded tab, a minimised window,
/// or the offscreen window the desktop's own probe runs in, where this waited
/// for ever and the game never started. Two frames is 33ms when they come;
/// 250ms is late enough that a transition has finished anyway and early enough
/// that nobody waits.
function settled() {
  return Promise.race([
    new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))),
    new Promise((r) => setTimeout(r, 250)),
  ]);
}

/// Bytes, rendered the way the rest of the app renders them.
function mb(n) {
  return `${Math.round(Number(n) / 1e6)} MB`;
}

/// Ask before pulling a disc image across the network into a tab's memory.
///
/// A cartridge is a rounding error and is never asked about; a PlayStation
/// image is three hundred megabytes and starting one by accident is a bad
/// surprise on a phone. Resolved by `confirm` rather than a custom dialog
/// because this is the one question and the app has no modal of its own that
/// fits here.
async function confirmHeavy(rom) {
  const size = rom.size_bytes ?? 0;
  if (!shouldWarn(rom.platform_slug ?? rom.platform, size)) return true;
  return (
    (await ask(
      null,
      `${rom.name} is ${mb(size)}. Playing it here downloads all of that into ` +
        `this tab before the first frame.`,
      [["yes", "Play it"], ["", "Cancel"]]
    )) === "yes"
  );
}

/// Load EmulatorJS once, and only when somebody actually presses Play.
///
/// Its loader reads a set of `EJS_*` globals rather than taking arguments, and
/// it appends the emulator to the element named by `EJS_player`. So the globals
/// have to be set before the script is added, every time.
function loadLoader(dataBase) {
  return new Promise((resolve, reject) => {
    const existing = document.getElementById("ejs-loader");
    if (existing) return resolve();
    const s = document.createElement("script");
    s.id = "ejs-loader";
    s.src = `${dataBase}loader.js`;
    s.onload = () => resolve();
    s.onerror = () =>
      reject(
        new Error(
          "EmulatorJS is not installed on the server — run scripts/fetch-emulatorjs.sh"
        )
      );
    document.body.appendChild(s);
  });
}

/// The overlay the game runs in.
///
/// A `<dialog>`, opened with `showModal`, rather than a positioned div. The
/// clicks that get here start view transitions, and a running transition paints
/// a snapshot of the page in the **top layer** -- above any z-index, including
/// this one. A plain overlay created underneath it is invisible for the length
/// of the animation and, if a second transition skips the first, can stay that
/// way. `showModal` puts this in the top layer too, and the top layer stacks in
/// the order things entered it, so the newest is on top.
///
/// jsdom implements neither, which is why every harness run said this worked.
function openStage(title, shader, platformSlug, romId, core) {
  const stage = document.createElement("dialog");
  stage.id = "ejs-stage";
  stage.innerHTML = `
    <div class="ejs-bar">
      <button class="ejs-close" aria-label="Stop">Stop</button>
      <span class="ejs-title"></span>
      <button class="ejs-save-state" title="Freeze the game and keep it on the server">Save state</button>
      <label class="ejs-states">Load
        <select><option value="">…</option></select>
      </label>
      <label class="ejs-shader">Shader
        <select></select>
      </label>
    </div>
    <div class="ejs-frame"><div id="ejs-player"></div><div class="ejs-note"></div></div>`;
  stage.querySelector(".ejs-title").textContent = title;
  const sel = stage.querySelector(".ejs-shader select");
  for (const p of presetsFor(platformSlug)) {
    const o = document.createElement("option");
    o.value = p.id;
    o.textContent = p.label;
    if (p.note) o.title = p.note;
    sel.appendChild(o);
  }
  sel.value = shader;
  // `changeSettingOption`, which is what the emulator's own settings menu
  // calls: it routes "shader" through `handleSpecialOptions` to `enableShader`.
  // My first attempt guessed `setShader`, which does not exist -- so the picker
  // changed, nothing else did, and no error said why. Read out of the pinned
  // build rather than guessed at this time.
  sel.addEventListener("change", () => {
    rememberShader(sel.value);
    const emu = globalThis.EJS_emulator;
    try {
      emu.changeSettingOption("shader", sel.value || "none");
      note(stage, "");
    } catch (e) {
      // Say so rather than leaving a control that appears to do nothing.
      note(stage, `Shader will apply next time this game starts (${e?.message ?? e})`);
    }
  });
  // Save states are the server's, and on the desktop there is no server behind
  // this window. They are also not portable in the way a save is: SRAM is the
  // cartridge's battery and any core can read it, while a state is a snapshot
  // of one WebAssembly build's memory and RetroArch's snes9x cannot load one
  // written by EmulatorJS's. So the buttons come off rather than being wired to
  // something that would fail, or worse, half-work.
  if (urls?.desktop) {
    stage.querySelector(".ejs-save-state")?.remove();
    stage.querySelector(".ejs-states")?.remove();
  } else {
    wireStates(stage, romId, core);
  }
  document.body.appendChild(stage);
  // `showModal` where it exists; an open dialog is still a visible one where it
  // does not, and a game running is better than a correct stacking context.
  try {
    stage.showModal?.();
  } catch {
    stage.setAttribute("open", "");
  }
  if (!stage.hasAttribute("open")) stage.setAttribute("open", "");
  return stage;
}

/// Ask a question inside the stage, without freezing the game.
///
/// Not `confirm()`. A native dialog blocks the page's main thread, which here
/// means blocking a running emulator -- and in headless Chrome it blocks
/// forever, because nothing ever answers it. That is how the save sync came to
/// freeze the whole page the first time it met a conflict.
///
/// Resolves to the value of whichever button was pressed.
function ask(stage, message, choices) {
  return new Promise((resolve) => {
    const box = document.createElement("div");
    box.className = "ejs-ask";
    box.innerHTML = `<p></p><div class="ejs-ask-row"></div>`;
    box.querySelector("p").textContent = message;
    const row = box.querySelector(".ejs-ask-row");
    for (const [value, label] of choices) {
      const b = document.createElement("button");
      b.textContent = label;
      b.addEventListener("click", () => {
        box.remove();
        resolve(value);
      });
      row.appendChild(b);
    }
    (stage ?? document.body).appendChild(box);
  });
}

/// Say what is happening inside the stage.
///
/// A blank black rectangle is the worst possible failure: it looks the same
/// whether the core is still downloading, the page is missing a file, or
/// something threw. Every step writes here, so whatever goes wrong the box
/// itself says what it got to.
function note(stage, text, bad = false) {
  const n = stage?.querySelector(".ejs-note");
  if (!n) return;
  n.textContent = text;
  n.dataset.bad = bad ? "1" : "";
}

/// Report anything thrown while the game is starting.
///
/// EmulatorJS loads its own scripts from inside an async function of its own,
/// so a failure in there never reaches the promise this module awaits — it
/// surfaces as an unhandled rejection on the window and nothing else. Without
/// this the stage sits blank and the console is the only witness.
function watchForErrors(stage) {
  const onErr = (e) => note(stage, `Failed: ${e?.message ?? e?.reason?.message ?? e?.reason ?? e}`, true);
  globalThis.addEventListener("error", onErr);
  globalThis.addEventListener("unhandledrejection", onErr);
  return () => {
    globalThis.removeEventListener("error", onErr);
    globalThis.removeEventListener("unhandledrejection", onErr);
  };
}

/// The two state controls: make one, and bring one back.
///
/// Buttons rather than a timer, unlike the save. A state is a moment somebody
/// chose; taking one every two minutes would fill the shelf with moments
/// nobody picked, and `/api/states` answers no conflict because a freeze-frame
/// cannot be merged with another.
function wireStates(stage, romId, core) {
  const make = stage.querySelector(".ejs-save-state");
  const pick = stage.querySelector(".ejs-states select");

  const refresh = async () => {
    let states = [];
    try {
      states = await serverStates(romId);
    } catch {
      // A list that cannot be fetched is not a reason to stop playing.
    }
    pick.innerHTML = "";
    const first = document.createElement("option");
    first.value = "";
    first.textContent = states.length ? `${states.length} saved` : "none yet";
    pick.appendChild(first);
    for (const st of states) {
      const o = document.createElement("option");
      o.value = String(st.id);
      // The name is what it was saved as, which is a timestamp unless somebody
      // renamed it on disk. Shown as-is rather than reformatted: the file on
      // the server is the thing, and inventing a prettier label hides which.
      o.textContent = st.file_name.replace(/\.state$/, "");
      if (st.emulator) o.title = `made with ${st.emulator}`;
      pick.appendChild(o);
    }
  };

  make.addEventListener("click", async () => {
    const gm = globalThis.EJS_emulator?.gameManager;
    if (!gm) return;
    make.disabled = true;
    try {
      // The core that actually made it, not the system name we asked for.
      // EmulatorJS resolves `snes` to `snes9x`, and a state belongs to the
      // build that wrote it -- restoring a snes9x state into another SNES core
      // is a crash rather than a wrong picture. Falls back to the system name
      // when the emulator does not say.
      const made = globalThis.EJS_emulator?.coreName || core;
      const st = await pushState(gm, romId, { core: made });
      note(stage, `State saved — ${st.file_name}`);
      await refresh();
    } catch (e) {
      note(stage, `State not saved: ${e?.message ?? e}`, true);
    } finally {
      make.disabled = false;
      setTimeout(() => note(stage, ""), 4000);
    }
  });

  pick.addEventListener("change", async () => {
    const gm = globalThis.EJS_emulator?.gameManager;
    if (!gm || !pick.value) return;
    try {
      const n = await pullState(gm, pick.value);
      note(stage, `State loaded (${n} bytes)`);
    } catch (e) {
      note(stage, `State not loaded: ${e?.message ?? e}`, true);
    }
    setTimeout(() => note(stage, ""), 4000);
  });

  refresh();
}

/// How often a running game's save is pushed up.
///
/// Not only on the way out. A tab closes without warning -- a lid, a crash, a
/// phone deciding the page is old -- and a save written only on exit is a save
/// lost to any of those. Two minutes is short enough that the most anyone can
/// lose is two minutes.
const FLUSH_EVERY = 120_000;

let flushTimer = null;
let syncing = false;

/// Sync now, and never twice at once: a second pass while the first is still
/// negotiating would compare against a server the first is about to change.
async function flush(stage, romId, why) {
  const gm = globalThis.EJS_emulator?.gameManager;
  if (!gm || syncing) return;
  syncing = true;
  try {
    // On the desktop the save belongs to this machine and there is nothing to
    // negotiate with: it goes straight into the file RetroArch reads, and the
    // app's own `sync_saves` carries it to the server exactly as it does for a
    // game played in RetroArch. Negotiating here would make one machine two
    // devices and give one game two saves on one disk.
    if (urls?.desktop) {
      const out = await localSaveOut(gm, romId, invoke);
      if (out.action === "upload") {
        note(stage, `Save written (${out.bytes} bytes, ${why})`);
        setTimeout(() => note(stage, ""), 4000);
      }
      return;
    }
    const out = await syncOne(gm, romId, {
      // Never resolved silently. A save is hours of somebody's life and the
      // wrong pick is unrecoverable, so this asks and takes no answer as no.
      onConflict: async (op) =>
        ask(stage, `This game's save changed here and on the server. ${op.reason}`, [
          ["mine", "Keep this browser's"],
          ["theirs", "Keep the server's"],
          ["", "Leave both alone"],
        ]),
    });
    if (out.action === "upload") note(stage, `Save sent to the server (${why})`, false);
    if (out.action === "download") note(stage, "Save restored from the server");
    if (out.action === "conflict" && !out.resolved) note(stage, "Save conflict — left alone", true);
    if (out.action !== "no_op") setTimeout(() => note(stage, ""), 4000);
  } catch (e) {
    // A sync that cannot happen must not stop somebody playing.
    note(stage, `Save not synced: ${e?.message ?? e}`, true);
  } finally {
    syncing = false;
  }
}

/// Pull the server's copy in, then keep pushing ours back.
function startSaveSync(stage, romId) {
  if (urls?.desktop) {
    // In, not out: on the way in there is nothing in the core worth keeping,
    // and the file on disk is whatever RetroArch last wrote.
    localSaveIn(stage && globalThis.EJS_emulator?.gameManager, romId, invoke)
      .then((out) => {
        if (out?.action !== "download") return;
        note(stage, `Save loaded from this machine (${out.bytes} bytes)`);
        setTimeout(() => note(stage, ""), 4000);
      })
      .catch((e) => note(stage, `Save not loaded: ${e?.message ?? e}`, true));
  } else {
    flush(stage, romId, "start");
  }
  clearInterval(flushTimer);
  flushTimer = setInterval(() => flush(stage, romId, "autosave"), FLUSH_EVERY);
  // The two events that actually fire when somebody walks away. `pagehide` is
  // the one that survives a phone discarding the tab, where `beforeunload` is
  // not guaranteed to run at all.
  const onGone = () => flush(stage, romId, "leaving");
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") onGone();
  });
  globalThis.addEventListener("pagehide", onGone);
}

/// Take the game down and put the page back.
///
/// A reload rather than a teardown: EmulatorJS installs global state, an audio
/// context and its own key and gamepad handlers, and has no supported way to
/// remove them. Trying to unpick that by hand is how a second launch comes up
/// silent or with the pad captured by a game that is no longer on screen.
export async function stopPlaying() {
  clearInterval(flushTimer);
  // The last thing before the page goes: a reload throws the emulator away, and
  // with it any save that has not been sent.
  const romId = globalThis.__moosePlayingId;
  if (romId != null) await flush(null, romId, "stopping");
  starting = false;
  location.reload();
}

/// Test seam: forget that a game was started.
export function resetPlayer() {
  starting = false;
  document.getElementById("ejs-stage")?.remove();
}

/// True from the moment a start is accepted until the stage is gone.
///
/// Not `document.getElementById("ejs-stage")`: the stage is not built until
/// after the first await, so two calls in the same tick would both find no
/// stage and both go on to make one. A flag set before anything is awaited is
/// the only thing a second synchronous call can see.
let starting = false;

/// Play one game in the page. `rom` is a `rom_detail`.
export async function playInBrowser(rom) {
  // One at a time. A double-click is two `click` events and a `dblclick`, and
  // anything that got two launches through would build two stages, two
  // EmulatorJS instances, two audio contexts and two sets of key handlers on
  // one page. That does not half-work; it looks like nothing working.
  if (starting || document.getElementById("ejs-stage")) {
    return "Already playing";
  }
  starting = true;

  // Before the refusal check, so "EmulatorJS was never fetched" is not reported
  // as "this console cannot be played here" -- two different answers, and only
  // one of them has anything the user can do about it.
  urls = await ejsUrls(invoke);
  if (!urls.available) {
    starting = false;
    toast(
      "Playing in the window needs EmulatorJS — run scripts/fetch-emulatorjs.sh",
      8000
    );
    return "EmulatorJS is not installed";
  }

  const verdict = browserPlay(rom.platform_slug ?? rom.platform);
  if (verdict.refuse) {
    starting = false;
    toast(verdict.refuse, 6000);
    return "Not playable in a browser";
  }
  if (!(await confirmHeavy(rom))) {
    starting = false;
    return "Cancelled";
  }

  // `/rom`, not `/api/roms/{id}/content/`. Two id spaces live in that process:
  // /api/ numbers the scan it serves to clients, and the UI works in cache ids,
  // which are negative for anything found on this machine. Building an /api/
  // URL from a cache id 404s on every game, which is what it did.
  const url = urls.rom(rom.id);

  // Let the page settle first. The clicks that got here start view transitions,
  // and a transition puts a snapshot of the page in the top layer above
  // everything -- including a canvas created while it is running, which is how
  // an emulator ends up started and invisible. Two frames is enough for one to
  // finish; where the API is missing this costs nothing.
  await settled();

  // The viewer's own choice, else whatever this console is configured for.
  const shader = chosenShader(rom.shader, rom.platform_slug ?? rom.platform);
  const stage = openStage(rom.name, shader, rom.platform_slug ?? rom.platform, rom.id, verdict.core);
  stage.querySelector(".ejs-close").addEventListener("click", stopPlaying);

  const w = globalThis;
  w.EJS_player = "#ejs-player";
  w.EJS_core = verdict.core;
  w.EJS_gameUrl = url;
  w.EJS_gameName = rom.name;
  // Vendored, and the trailing slash matters: the loader concatenates rather
  // than resolving, so without it every core is fetched from one directory up.
  w.EJS_pathtodata = urls.data;
  w.EJS_startOnLoaded = true;
  // No phoning out. This is a LAN service and the whole reason the cores are
  // vendored; an ad frame would also be the only network call in the app that
  // is not to your own machine.
  // Capital A and U: that is what `loader.js` reads. `EJS_adUrl` is quietly
  // ignored, which is a fine way to keep an ad frame you thought you removed.
  w.EJS_AdUrl = "";
  w.EJS_alignStartButton = "center";
  // Its own bios/save directories would collide across games otherwise.
  w.EJS_gameID = rom.id;
  // For `stopPlaying`, which runs after the stage is gone.
  w.__moosePlayingId = rom.id;
  // EmulatorJS reads its start-up options from here. An empty string is a real
  // value meaning "none", so it is set either way rather than left undefined.
  w.EJS_defaultOptions = { ...(w.EJS_defaultOptions ?? {}), shader: shader || "none" };

  // Told by EmulatorJS itself rather than guessed at.
  w.EJS_ready = () => note(stage, "Core loaded — press start");
  w.EJS_onGameStart = () => {
    note(stage, "");
    // On `start`, deliberately after the emulator's own restore rather than
    // racing it. Strictly later cannot lose, and this was measured before it
    // was written: writing here reaches the running core, all 8192 bytes.
    startSaveSync(stage, rom.id);
  };

  const unwatch = watchForErrors(stage);
  note(stage, "Loading EmulatorJS…");
  try {
    await loadLoader(urls.data);
  } catch (e) {
    unwatch();
    stage.remove();
    starting = false;
    toast(String(e.message ?? e), 8000);
    return "Could not start";
  }
  note(stage, `Starting ${verdict.core}…`);
  // Left watching on purpose: the interesting failures happen after the loader
  // script has loaded, while it is fetching the core and the game.
  return `Playing ${rom.name}`;
}

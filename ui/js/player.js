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
// Saves are NOT wired up yet. EmulatorJS keeps its own IndexedDB filesystem and
// putting that through `/api/saves` is the next piece; until then this is for
// playing, and progress made here stays here. `docs/browser-emulation.md` says
// how it should go, and it must go through the negotiate that already exists
// rather than becoming a second sync.

import { browserPlay, shouldWarn } from "./ejs-systems.js";
import { toast } from "./util.js";

const EJS_PATH = "/emulatorjs/";

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
function confirmHeavy(rom) {
  const size = rom.size_bytes ?? 0;
  if (!shouldWarn(rom.platform_slug ?? rom.platform, size)) return true;
  return globalThis.confirm(
    `${rom.name} is ${mb(size)}. Playing it here downloads all of that into ` +
      `this tab before the first frame. Continue?`
  );
}

/// Load EmulatorJS once, and only when somebody actually presses Play.
///
/// Its loader reads a set of `EJS_*` globals rather than taking arguments, and
/// it appends the emulator to the element named by `EJS_player`. So the globals
/// have to be set before the script is added, every time.
function loadLoader() {
  return new Promise((resolve, reject) => {
    const existing = document.getElementById("ejs-loader");
    if (existing) return resolve();
    const s = document.createElement("script");
    s.id = "ejs-loader";
    s.src = `${EJS_PATH}data/loader.js`;
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
function openStage(title) {
  const stage = document.createElement("div");
  stage.id = "ejs-stage";
  stage.innerHTML = `
    <div class="ejs-bar">
      <button class="ejs-close" aria-label="Stop">Stop</button>
      <span class="ejs-title"></span>
    </div>
    <div class="ejs-frame"><div id="ejs-player"></div><div class="ejs-note"></div></div>`;
  stage.querySelector(".ejs-title").textContent = title;
  document.body.appendChild(stage);
  return stage;
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

/// Take the game down and put the page back.
///
/// A reload rather than a teardown: EmulatorJS installs global state, an audio
/// context and its own key and gamepad handlers, and has no supported way to
/// remove them. Trying to unpick that by hand is how a second launch comes up
/// silent or with the pad captured by a game that is no longer on screen.
export function stopPlaying() {
  location.reload();
}

/// Play one game in the page. `rom` is a `rom_detail`.
export async function playInBrowser(rom) {
  const verdict = browserPlay(rom.platform_slug ?? rom.platform);
  if (verdict.refuse) {
    toast(verdict.refuse, 6000);
    return "Not playable in a browser";
  }
  if (!confirmHeavy(rom)) return "Cancelled";

  // `/rom`, not `/api/roms/{id}/content/`. Two id spaces live in that process:
  // /api/ numbers the scan it serves to clients, and the UI works in cache ids,
  // which are negative for anything found on this machine. Building an /api/
  // URL from a cache id 404s on every game, which is what it did.
  const url = `/rom?id=${encodeURIComponent(rom.id)}`;

  const stage = openStage(rom.name);
  stage.querySelector(".ejs-close").addEventListener("click", stopPlaying);

  const w = globalThis;
  w.EJS_player = "#ejs-player";
  w.EJS_core = verdict.core;
  w.EJS_gameUrl = url;
  w.EJS_gameName = rom.name;
  // Vendored, and the trailing slash matters: the loader concatenates rather
  // than resolving, so without it every core is fetched from one directory up.
  w.EJS_pathtodata = EJS_PATH + "data/";
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

  // Told by EmulatorJS itself rather than guessed at.
  w.EJS_ready = () => note(stage, "Core loaded — press start");
  w.EJS_onGameStart = () => note(stage, "");

  const unwatch = watchForErrors(stage);
  note(stage, "Loading EmulatorJS…");
  try {
    await loadLoader();
  } catch (e) {
    unwatch();
    stage.remove();
    toast(String(e.message ?? e), 8000);
    return "Could not start";
  }
  note(stage, `Starting ${verdict.core}…`);
  // Left watching on purpose: the interesting failures happen after the loader
  // script has loaded, while it is fetching the core and the game.
  return `Playing ${rom.name}`;
}

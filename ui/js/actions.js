// Download and launch, kept separate from the pane that triggers them so the
// grid can call them on double-click without importing the whole sidebar.

import { state, invoke, listen, MOBILE } from "./state.js";
import { toast } from "./util.js";
import { askConflicts, conflictsFrom, askOffline, offlineFrom, askBios, biosFrom, noteLightGun } from "./conflicts.js";
import { suspendPad, resumePad } from "./gamepad.js";

/// Display refresh in Hz, measured rather than asked for: no web API reports
/// it, and Tauri's Monitor exposes size, position and scale factor but not
/// refresh rate. The median gap between animation frames is a good proxy —
/// median rather than mean so one hitched frame does not drag the estimate.
///
/// Cached after the first read: this costs ~24 frames, and a launch should not
/// wait on it twice. It is also taken at idle rather than on demand, because
/// the first launch of a session otherwise waited out those frames — most of a
/// second on a 30Hz-throttled window — before the request to start the game had
/// even been sent.
let refreshHz = null;

/// Take the measurement now, while nobody is waiting for it.
export function warmRefresh() {
  measureRefresh();
}

function measureRefresh(frames = 24) {
  if (refreshHz !== null) return Promise.resolve(refreshHz);
  return new Promise((resolve) => {
    const times = [];
    let last = performance.now();
    const tick = (now) => {
      times.push(now - last);
      last = now;
      if (times.length < frames) return requestAnimationFrame(tick);
      times.sort((a, b) => a - b);
      const median = times[times.length >> 1];
      // Guard against a throttled or backgrounded window, which reports
      // frame gaps far outside any real refresh rate.
      refreshHz = median > 0.5 && median < 40 ? Math.round(1000 / median) : null;
      resolve(refreshHz);
    };
    requestAnimationFrame(tick);
  });
}

/// True from the moment a launch starts until the emulator has exited and the
/// pad has settled.
///
/// `launch_rom` does not return until the game quits, so a second press in the
/// meantime used to start a second copy — two RetroArch windows over each
/// other, both holding the same save. A held A repeats slowly enough that it
/// took a deliberate double press, which is exactly what a controller invites.
let launching = false;

export function launchInFlight() {
  return launching;
}

/// Start a game on Android, where launching is asking another app.
///
/// Two steps rather than one because neither half can do the other's job. Rust
/// knows which core the game wants and where the ROM is; only Kotlin can see
/// which packages exist and start an Intent — Tauri does not hand the Android
/// context to Rust (tauri-apps/tauri#13267), so a Tauri command cannot reach
/// `startActivity`. So the backend plans, `MooseAndroid` starts, and this is the
/// seam between them.
///
/// Nothing is awaited. `startActivity` returns as soon as the request is
/// accepted, not when the game ends, so there is no exit code to report and no
/// moment to sync saves at — see `android_launch_plan` for the rest of what
/// that costs. The toast says the game is starting because that is the whole of
/// what is known.
/// The other half of an Android launch: RetroArch has given the screen back.
///
/// Called from Kotlin, which is the only side that can tell — the emulator is
/// another app and reports nothing when it stops. See `reportGameFinished` in
/// MainActivity.kt.
window.__gameFinished = async (id, seconds) => {
  try {
    const said = await invoke("android_after_play", { id, seconds });
    if (said) toast(said, 6000);
  } catch (e) {
    toast(`Saves were not uploaded — ${e}`, 8000);
  }
};

async function launchAndroid(id, skipSync = false) {
  const bridge = window.MooseAndroid;
  if (!bridge?.startEmulator) throw new Error("this build has no way to start an emulator");
  // Two things only Kotlin knows, and the config depends on both: which
  // RetroArch is installed decides where its own files are, and where we may
  // write decides whether it can read what we generate.
  // Pull what the server has that is newer, before the game opens it. Throws
  // for a conflict or an unreachable server, which `launch` already knows how
  // to ask about — the same two questions the desktop asks.
  const synced = skipSync ? "" : await invoke("android_sync_before", { id });
  const plan = await invoke("android_launch_plan", {
    id,
    retroarchPackage: bridge.retroArchPackage?.() ?? "",
    configDir: bridge.externalFilesDir?.() ?? "",
    pad: state.gamepad,
    // Decides how many subframes a strobe pass gets, same as on the desktop.
    refresh: await measureRefresh(),
  });
  const failed = bridge.startEmulator(JSON.stringify(plan));
  if (failed) throw new Error(failed);
  const via = plan.candidates?.[0]?.label;
  const notes = [...(plan.notes || []), synced].filter(Boolean);
  const said = notes.length ? ` — ${notes.join("; ")}` : "";
  return `Starting ${plan.name}${via ? ` in ${via}` : ""}…${said}`;
}

/// Launch, optionally picking up where the game was left.
///
/// `resume: true` means "carry on if there is something to carry on from" —
/// the newest resumable state is used, and a game with none simply starts. It
/// is not the same as `entrySlot`, which names one exact state the user
/// clicked; resume is a decision this function makes.
export async function launch(
  id,
  { resolving = false, skipSync = false, entrySlot = null, resume = false } = {}
) {
  if (resume && entrySlot === null) {
    try {
      const states = await invoke("game_states", { id });
      // Newest first is not guaranteed by the backend, so sort rather than
      // assume — and only slots that can actually be started from. RetroArch's
      // auto-save slot reports `resumable: false` because loading it is what
      // RetroArch does by itself, and starting *into* it double-loads.
      const best = states
        .filter((st) => st.resumable)
        .sort((a, b) => (b.when_epoch ?? 0) - (a.when_epoch ?? 0))[0];
      if (best) entrySlot = Number(best.slot);
    } catch {
      // No states, no RetroArch, no answer — start the game. A resume that
      // cannot be worked out is not a reason to refuse to play.
    }
  }
  // Say once, on the first gun console launched, that the mouse is the gun.
  // Before the guard below and before the pad goes quiet, because it is a
  // dialog the user answers rather than anything the launch depends on.
  //
  // Not on Android, where every word of it is wrong: there is no mouse to aim
  // with, and the light gun setting it is explaining is applied by the desktop
  // planner, which this platform does not use. It was a modal in front of every
  // Mega Drive, Master System, NES and PlayStation launch on the handheld,
  // asking about hardware that is not there.
  if (!resolving && !skipSync && !MOBILE) {
    try {
      // Shape-checked, not just truthy. `[]` is truthy in JavaScript, so a
      // backend — or a stub — answering with an empty array opened a dialog
      // with no text and no way to know what it was about, and the launch
      // waited on a promise nobody could resolve.
      const gun = await invoke("game_lightgun", { id });
      if (Array.isArray(gun) && gun.length === 2 && gun[1]) {
        await noteLightGun(gun[0], gun[1]);
      }
    } catch {
      // A console with no gun, or a backend that does not know: launch.
    }
  }
  // The retry paths below call back into this function on purpose, so they
  // pass through the guard rather than being stopped by it.
  if (launching && !resolving && !skipSync) return;
  launching = true;
  // And the pad goes quiet immediately, before anything is awaited: the press
  // that started this is still down, and the poll runs sixty times a second.
  suspendPad();
  // Declared out here so the error paths can drop the listener too: `const`
  // inside the try block is not in scope in the catch.
  let stop;
  try {
    toast("Launching…");
    // The connected pad's name picks which RetroArch autoconfig profile the
    // gamepad hotkeys are built from. Raw button indices differ per controller
    // and per OS, so guessing them is how "hold Select" ended up as "hold B".
    // The pad is very likely still held: this call does not return until the
    // emulator exits, and the way out of a game is a button combination.
    // Whatever is down belongs to that, not to us. (Suspended above as well,
    // before the first await — this one is the one that survives a retry.)
    suspendPad();
    stop = await listen("launch-progress", ({ payload }) => {
      toast(String(payload), 30_000);
    });
    // Android goes another way entirely: no process to spawn and nothing to
    // wait on. Everything above this line still applies — the guard, the pad,
    // the toast — because they are about this window, not about the emulator.
    const result = MOBILE
      ? await launchAndroid(id, skipSync)
      : await invoke("launch_rom", {
          id,
          pad: state.gamepad,
          refresh: await measureRefresh(),
          skipSync,
          entrySlot,
        });
    stop?.();
    toast(result);
  } catch (e) {
    stop?.();
    // A save that changed in two places stops the launch rather than picking a
    // winner. Ask, then start again — the second attempt syncs cleanly because
    // the conflict is gone by then.
    const conflicts = conflictsFrom(e);
    if (conflicts && !resolving) {
      const answered = await askConflicts(conflicts);
      if (!answered) return toast("Launch cancelled — saves left as they were");
      // `resolving` guards the retry: if it somehow conflicts again we report
      // it rather than reopening the dialog forever.
      return launch(id, { resolving: true, entrySlot });
    }
    // A BIOS this machine has not got and the server cannot supply. The
    // automatic fetch on the way in handles every other case, so reaching here
    // means there is nothing left to download — ask, because a core that
    // declares a BIOS does not always need one.
    //
    // `entrySlot` is carried through: it is which save state the launch was
    // resuming from, and dropping it would restart the game from the
    // beginning after the user said "play anyway".
    const bios = biosFrom(e);
    if (bios && !skipSync) {
      const go = await askBios(bios);
      if (!go) return toast("Launch cancelled");
      return launch(id, { skipSync: true, entrySlot });
    }
    // Saves could not be checked at all. Ask rather than deciding: starting
    // silently risks an hour on top of a stale save, and refusing would mean a
    // server being off stops you playing.
    const offline = offlineFrom(e);
    if (offline && !skipSync) {
      const go = await askOffline(offline);
      if (!go) return toast("Launch cancelled");
      return launch(id, { skipSync: true, entrySlot });
    }

    toast(`Launch failed — ${e}`, 8000);
  } finally {
    // Whatever happened — played, cancelled, failed — the pad is locked for a
    // moment on the way out. It is the same lock the emulator's own exit
    // combination needs: the buttons that quit the game are still down when
    // this window gets them back.
    launching = false;
    // In `finally`, not after the await: a launch that throws — a save
    // conflict, a missing core, an unreachable server — would otherwise leave
    // the controller ignored for good, with no way back but restarting.
    resumePad();
  }
}

/// Star a game, or take the star off.
///
/// The star is a collection on the server, not a mark on this machine, so
/// pressing it here is what makes the handheld and the phone agree — see
/// `moose_rack::favorites`.
///
/// The row is repainted from what the server actually did, not from what was
/// asked for. A star that lights up locally and never reached the server is
/// the worst outcome: it lies here and is invisible everywhere else.
export async function toggleFavorite(id) {
  if (!id) return;
  try {
    // Which way to turn it is decided in Rust, from the cache. What this page
    // last drew can be older than what another device has since done, so the
    // row is not the thing to ask.
    const now = await invoke("toggle_favorite", { id });
    paintStar(id, now);
    toast(now ? "Starred" : "Star removed");
  } catch (e) {
    toast(`Could not change the star — ${e}`, 8000);
  }
}

/// Put the star on the row, or take it off, without redrawing the list.
///
/// Redrawing would lose the scroll position and the cursor, and this changes
/// one glyph on one row.
function paintStar(id, starred) {
  for (const node of document.querySelectorAll(`[data-id="${id}"]`)) {
    node.classList.toggle("fav", starred);
    if (starred) node.dataset.fav = "1";
    else delete node.dataset.fav;
    const name = node.querySelector(".nm") || node.querySelector(".art");
    const existing = node.querySelector(".star");
    if (starred && !existing && name) {
      const star = document.createElement("span");
      star.className = "star";
      star.title = "Starred — in one of your starred collections";
      star.textContent = "★";
      name.prepend(star);
    } else if (!starred && existing) {
      existing.remove();
    }
  }
}

export async function download(id, thenPlay) {
  const prog = document.getElementById("prog");
  if (prog) prog.hidden = false;
  try {
    toast(await invoke("download_rom", { id }));
    // Refresh so "Local: yes" and the download marker update.
    const { selectRom } = await import("./detail.js");
    if (state.selected === id) await selectRom(id);
    if (thenPlay) await launch(id);
  } catch (e) {
    toast(`Download failed — ${e}`, 8000);
  } finally {
    if (prog) prog.hidden = true;
  }
}

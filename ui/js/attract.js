// Attract mode: the idle counter, and choosing what to show.
//
// Designed off EmulationStation's, read out of both forks — see
// docs/attract-mode.md. Two pieces do the work and everything else is
// presentation on top of them.
//
// **One counter, at window level.** ES accumulates `mTimeSinceLastInput` every
// frame and any input at all zeroes it. It lives above every view, so no screen
// has to know attract mode exists — which is the reason it is one counter
// rather than a hook in each of them.
//
// **Sampled without replacement.** Nothing repeats until every game with media
// has been shown. It is perhaps twenty lines and it is the difference between
// attract mode feeling curated and feeling broken: a naive random pick shows
// the same three games in a row often enough that people notice.

import { invoke } from "./state.js";

const IDLE_KEY = "attract.idleSeconds";

/// Five minutes, which is ES's own default, and **zero means off**.
///
/// Worth keeping rather than adding an enable switch beside it: one number
/// where zero is a real answer is a setting people can reason about, and it is
/// one thing to store rather than two that can disagree.
export const IDLE_DEFAULT = 300;

export function attractIdleSeconds() {
  // Read as text first, and that is not fussiness. `Number(null)` is 0, and 0
  // is a real answer here meaning off — so converting straight from a key that
  // has never been set turns attract mode off for everybody who has not
  // deliberately turned it on. The one value that must not be produced by
  // accident is the one that disables the feature.
  const raw = localStorage.getItem(IDLE_KEY);
  if (raw === null) return IDLE_DEFAULT;
  const n = Number(raw);
  return Number.isFinite(n) && n >= 0 ? n : IDLE_DEFAULT;
}

export function setAttractIdleSeconds(seconds) {
  const n = Math.max(0, Math.round(Number(seconds) || 0));
  localStorage.setItem(IDLE_KEY, String(n));
  return n;
}

/// A bag of games that hands each one out once before repeating any.
///
/// `remaining` is what has not been shown this round. Taking from it removes
/// it, and when it empties the round starts again — which is the whole trick.
///
/// `last` survives the refill, so the seam between two rounds does not repeat
/// either. ES-DE keeps `mPreviousGame` for exactly this: without it, the last
/// game of one round can be the first of the next, which is the one repeat a
/// viewer is guaranteed to notice because it happens with nothing in between.
export function makeSampler(pool, random = Math.random) {
  const all = [...pool];
  let remaining = [];
  let last = null;

  const refill = () => {
    remaining = [...all];
  };

  return {
    get size() {
      return all.length;
    },
    /// How many are left before this round starts over. For tests and for
    /// anything that wants to show progress.
    get left() {
      return remaining.length;
    },
    next() {
      if (!all.length) return null;
      if (!remaining.length) refill();
      let at = Math.floor(random() * remaining.length);
      // Re-rolled once, not looped: with one game in the pool there is nothing
      // else to hand back and insisting would spin for ever.
      if (all.length > 1 && remaining.length > 1 && remaining[at] === last) {
        at = (at + 1) % remaining.length;
      }
      const pick = remaining.splice(at, 1)[0];
      last = pick;
      return pick;
    },
  };
}

/// Which keys mean "start this" rather than "go away".
///
/// ES has `ScreenSaverControls`: with it on, input during attract mode launches
/// the game being shown; with it off, input only dismisses. Both frontends do
/// the launching, and it is the thing that makes this attract mode rather than
/// a screensaver.
///
/// Here rather than beside the overlay that uses it, because it is a decision
/// and not presentation — and because this module has no DOM behind it, so it
/// can be tested without standing up half the app. Getting it wrong the
/// generous way means Escape launches something, which is the opposite of what
/// somebody reaching for Escape wants, on a screen that gave them no warning it
/// was about to run a game.
const LAUNCH_KEYS = new Set(["Enter", " ", "NumpadEnter"]);

export function launchesOn(key) {
  return LAUNCH_KEYS.has(key);
}

/// Everything the counter watches. Movement included: a mouse crossing the
/// window is somebody at the machine, and a screensaver that ignores that is
/// the kind people disable rather than fix.
const SIGNALS = ["keydown", "pointerdown", "pointermove", "wheel", "touchstart"];

/// Whether any pad is being touched. Polled rather than listened for, because
/// the Gamepad API has no press event — and asked here rather than wired into
/// gamepad.js, so neither module has to know about the other.
export function padIsActive(pads) {
  for (const pad of pads || []) {
    if (!pad) continue;
    if (pad.buttons.some((b) => b?.pressed)) return true;
    // A stick at rest is not exactly zero, and a drifting one should not hold
    // attract mode off for ever.
    if (pad.axes.some((a) => Math.abs(a) > 0.35)) return true;
  }
  return false;
}

let running = null;

/// Start watching. `onStart` is called when the machine has been left alone
/// long enough; `onStop` when somebody comes back.
///
/// Idempotent, and returns a function that takes it all down again.
export function installAttract({ onStart, onStop, now = () => Date.now(),
                                 pads = () => navigator.getGamepads?.() ?? [] } = {}) {
  if (running) return running.stop;

  let lastInput = now();
  let active = false;

  const wake = () => {
    lastInput = now();
    if (active) {
      active = false;
      onStop?.();
    }
  };

  const check = () => {
    if (padIsActive(pads())) return wake();
    const idle = attractIdleSeconds();
    // Zero is off, and it has to be checked here rather than at install time:
    // somebody can turn it off while the counter is already running.
    if (idle <= 0 || active) return;
    if ((now() - lastInput) / 1000 >= idle) {
      active = true;
      onStart?.();
    }
  };

  for (const type of SIGNALS) {
    window.addEventListener(type, wake, { capture: true, passive: true });
  }
  // A second is fine. ES counts every frame because it is already rendering
  // every frame; this is not, and a five-minute timer does not need to be
  // accurate to sixteen milliseconds.
  const timer = setInterval(check, 1000);
  // Unreferenced where that means anything. In a browser this is a number and
  // the call does nothing; under Node it is a handle, and an interval that
  // nobody clears keeps the process alive — which is what happened the moment
  // this was wired into main.js, because two test files import that and the
  // whole suite then hung instead of finishing. A timer for a five-minute idle
  // check has no business deciding when a program may exit.
  timer?.unref?.();

  const stop = () => {
    clearInterval(timer);
    for (const type of SIGNALS) {
      window.removeEventListener(type, wake, { capture: true });
    }
    running = null;
  };
  running = { stop, wake, check };
  return stop;
}

/// The pool, fetched once and kept.
///
/// Not fetched at startup: attract mode may never trigger in a session, and the
/// walk it costs is a few dozen directories off a memory card. Asked for the
/// first time the counter runs out, which is five minutes in.
let pool = null;
let poolPromise = null;

export async function attractPool() {
  if (pool) return pool;
  if (!poolPromise) {
    poolPromise = invoke("attract_pool")
      .then((got) => {
        pool = Array.isArray(got) ? got : [];
        return pool;
      })
      .catch(() => {
        // Rebuildable: a pool that failed once should be asked for again the
        // next time rather than leaving attract mode dead for the session.
        poolPromise = null;
        return [];
      });
  }
  return poolPromise;
}

export function forgetAttractPool() {
  pool = null;
  poolPromise = null;
}

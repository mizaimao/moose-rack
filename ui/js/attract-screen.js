// Attract mode, on screen.
//
// The parts worth being careful about are in `attract.js` — the idle counter
// and the sampler — and both are tested there without a DOM. This is what draws
// them, and it is deliberately the thin half.
//
// Slideshow before video, and not only because it is simpler. On Android a
// `<video>` on the asset protocol never starts, so `detail.js` fetches the
// bytes and hands over a blob; that is fine for one clip somebody asked for and
// wrong for a cycle that swaps every ten seconds for hours, because every swap
// is a few megabytes read off the card into memory first. Stills cost a decode.
//
// What makes this attract mode rather than a screensaver is the last twenty
// lines: a press launches what is on screen.

import { el, invoke } from "./state.js";
import { attractPool, makeSampler, installAttract, launchesOn } from "./attract.js";
import { launch, launchInFlight } from "./actions.js";

/// ES's own numbers, from `ScreenSaverSwapImageTimeout` and `FADE_TIME`.
const SWAP_MS = 10_000;
const FADE_MS = 500;

const MOBILE = /\bAndroid\b/.test(navigator.userAgent);

let screen = null;
let sampler = null;
let swapTimer = 0;
let showing = null;

function build() {
  const box = document.createElement("div");
  box.id = "attract";
  box.innerHTML = `<div class="attract-art"></div>
    <div class="attract-info"><b></b><span></span></div>
    <p class="attract-hint">Press to play &middot; any other key to go back</p>`;
  document.body.appendChild(box);
  // Focusable, so the keys arrive here rather than at whatever had focus when
  // the machine was left alone.
  box.tabIndex = -1;
  box.focus({ preventScroll: true });
  return box;
}

function paint(pick) {
  if (!screen || !pick) return;
  showing = pick;
  const art = screen.querySelector(".attract-art");
  const img = document.createElement("img");
  img.alt = "";
  img.src = window.__TAURI__.core.convertFileSrc(pick.image);
  // The new one is laid over the old and the old is dropped once the new has
  // decoded. Swapping the src of one element shows a blank frame between two
  // pictures, which on a slideshow is the only frame anybody sees moving.
  img.addEventListener("load", () => {
    img.classList.add("in");
    for (const old of [...art.children]) {
      if (old !== img) setTimeout(() => old.remove(), FADE_MS);
    }
  }, { once: true });
  art.appendChild(img);
  screen.querySelector(".attract-info b").textContent = pick.name;
  screen.querySelector(".attract-info span").textContent = pick.platform;
}

function swap() {
  paint(sampler?.next());
  swapTimer = setTimeout(swap, SWAP_MS);
}

async function start() {
  // Never over a launch. Coming back from a game is the one moment the app is
  // idle for a long time without anybody having left, and covering the screen
  // then is the most annoying thing this feature could do.
  if (screen || launchInFlight()) return;
  const pool = (await attractPool()).filter((g) => g.image);
  if (!pool.length) return;
  // Checked again: fetching the pool takes long enough on a memory card that
  // somebody can have come back in the meantime.
  if (screen || launchInFlight()) return;

  sampler = makeSampler(pool);
  screen = build();
  requestAnimationFrame(() => screen.classList.add("on"));
  swap();
}

function stop() {
  clearTimeout(swapTimer);
  swapTimer = 0;
  const going = screen;
  screen = null;
  sampler = null;
  showing = null;
  if (!going) return;
  going.classList.remove("on");
  setTimeout(() => going.remove(), FADE_MS);
}

/// A press during attract mode: start what is on screen, or just leave.
///
/// Captured, and that is the whole reason this listener exists separately from
/// the counter's: the counter only needs to know something happened, while this
/// has to see *what* happened before the rest of the app acts on it. Without
/// the capture the key reaches the grid underneath first and moves the cursor
/// on a screen nobody can see.
function onKey(e) {
  if (!screen) return;
  const wanted = launchesOn(e.key);
  const game = showing;
  e.preventDefault();
  e.stopPropagation();
  stop();
  if (wanted && game) launch(game.id).catch(() => {});
}

function onPoint(e) {
  if (!screen) return;
  e.preventDefault();
  e.stopPropagation();
  stop();
}

export function installAttractScreen() {
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("pointerdown", onPoint, true);
  // Settings is its own window, so its preview button cannot just call `start`
  // — it would cover the settings page, which proves nothing.
  window.__TAURI__?.event?.listen?.("attract-now", () => start());
  return installAttract({ onStart: start, onStop: stop });
}

/// For the settings pane: show it now rather than waiting five minutes.
export async function previewAttract() {
  await start();
}

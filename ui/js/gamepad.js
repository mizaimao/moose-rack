// Controller support via the Gamepad API.
//
// WKWebView exposes the same Gamepad API Safari does, and macOS presents Xbox,
// DualShock/DualSense, Switch Pro and most 8BitDo pads as standard-mapping
// gamepads over Bluetooth or USB — so one mapping covers all of them without
// any per-vendor code or a native dependency.
//
// Buttons follow the W3C "standard" layout, which macOS reports for those
// controllers. Face-button *positions* are used, not labels: index 0 is the
// bottom face button (A on Xbox, Cross on PlayStation, B on a Nintendo
// layout), which is "confirm" on every platform.

import { el, state, MOBILE } from "./state.js";
import { runAction } from "./keys.js";
import { padMap } from "./bindings.js";
import { toast } from "./util.js";
import { scrollDetail } from "./detail.js";
import { scrollList } from "./library.js";
import { closeLightbox, stepLightbox, togglePlayback } from "./lightbox.js";
import { useMiximage } from "./pictures.js";

// Rebindable in Settings; padMap() layers the user's choices over the
// position-based defaults in bindings.js.

// Held directions repeat; one-shot buttons do not.
const REPEATABLE = new Set([
  "up", "down", "left", "right", "pageUp", "pageDown",
  // Holding a trigger sweeps the size rather than stepping once per press.
  "zoomIn", "zoomOut",
]);
const FIRST_REPEAT_MS = 380;
const REPEAT_MS = 110;
const STICK_DEADZONE = 0.55;

/// Below this a trigger is resting, or being brushed. Lower than the stick's
/// because a trigger is pulled deliberately and has no center to drift around.
const TRIGGER_DEADZONE = 0.06;

/// Pixels a frame at a full pull. About a screen every third of a second on a
/// laptop display — fast enough to cross a long list, slow enough to stop on
/// something.
const TRIGGER_TOP_SPEED = 34;

const held = new Map(); // action -> next fire time
let running = false;

/// Set while the pad must be ignored until every button is released.
///
/// The quit hotkey is Select + A, and both are bound here too — Select opens
/// Settings, A launches whatever is selected. So the moment RetroArch exited
/// and this window took focus again, the still-held buttons read as fresh
/// presses: the game relaunched and Settings opened behind it.
///
/// Waiting for release rather than a fixed timeout, because how long someone
/// holds a quit combo is not something to guess at.
///
/// Release alone turned out not to be enough. Coming back from RetroArch the
/// pad can report *nothing* held for a frame or two before the real state
/// arrives — the window regains focus first and the Gamepad API catches up
/// after — so the still-held quit combo read as released, the lock lifted, and
/// then the same buttons arrived as a fresh press. A short floor under the
/// lock covers that gap: input is ignored until the pad is at rest **and** the
/// floor has passed.
const SETTLE_FLOOR_MS = 200;

let settling = false;
let settleUntil = 0;

export function ignorePadUntilReleased() {
  settling = true;
  settleUntil = performance.now() + SETTLE_FLOOR_MS;
  held.clear();
}

/// Set for as long as a game is running.
///
/// Waiting for the pad to be released is not enough while the emulator is up.
/// The pad goes to rest a second after launch, the lock lifts, and this window
/// carries on polling behind the game for the whole session — so the buttons
/// pressed to quit are read here as they are pressed, and the app relaunches
/// the game it was told to leave.
///
/// It never showed on macOS, where the webview stops receiving gamepad events
/// once it loses focus. WebView2 on Windows keeps delivering them, so there the
/// entire session is one long stream of input nobody meant for this window.
let suspended = false;

export function suspendPad() {
  suspended = true;
  held.clear();
}

export function resumePad() {
  suspended = false;
  ignorePadUntilReleased();
}

/// Whether the lock may lift: the pad is at rest *and* the floor has passed.
///
/// Split out so the rule can be tested. The bug it encodes is invisible from
/// the outside — a lock that lifts one frame too early looks exactly like a
/// game relaunching itself.
export function settleLifted(pressed, now) {
  return pressed.size === 0 && now >= settleUntil;
}

function fire(action, now) {
  const due = held.get(action);
  if (due === undefined) {
    // Newly pressed: act immediately, then wait out the initial delay.
    held.set(action, now + FIRST_REPEAT_MS);
    runAction(action);
    return;
  }
  if (now < due) return;
  if (!REPEATABLE.has(action)) return;
  held.set(action, now + REPEAT_MS);
  runAction(action);
}

/// Translate raw pad state into the set of actions held this frame.
///
/// Split out of `poll` so it can be tested without a browser: this is where
/// "the A button does nothing" would live, and a bug here is otherwise
/// invisible, since poll runs inside requestAnimationFrame where a throw is
/// swallowed and the next frame is already queued.
/// The pad that drives the interface: the first one connected, and only that
/// one.
///
/// With four controllers plugged in for a four-player game, every one of them
/// used to move the cursor in the library — so three other people fidgeting
/// with sticks while player one tries to pick a game made the menu unusable.
/// In the emulator all four are players; out here, player one is in charge.
export function primaryPad(pads) {
  for (const pad of pads) {
    if (pad?.connected !== false && pad) return pad;
  }
  return null;
}

export function pressedActions(pads, map) {
  const pressed = new Set();
  const pad = primaryPad(pads);
  if (!pad) return pressed;

  for (const [index, action] of Object.entries(map)) {
    // A rebind clears the old slot by writing null, so skip those rather
    // than dispatching an action of null.
    if (action && pad.buttons[index]?.pressed) pressed.add(action);
  }
  // Right stick scrolls the detail pane. Axes 2 and 3 in the W3C standard
  // layout. Done here rather than as an action because it is continuous
  // rather than a repeat-fire keypress: how far you push decides how fast it
  // moves, which a discrete action cannot express.
  const ry = pad.axes[3] ?? 0;
  if (Math.abs(ry) > STICK_DEADZONE) {
    // Squared so a small push creeps and a full push moves properly.
    const speed = Math.sign(ry) * (Math.abs(ry) ** 2) * 26;
    scrollDetail(speed);
  }

  // Left stick doubles as a d-pad, but only along its dominant axis. Pushed
  // diagonally it would otherwise report left+up in the same frame and move
  // twice, which reads as the cursor jumping around on its own.
  const [x = 0, y = 0] = pad.axes;
  if (Math.abs(x) > Math.abs(y)) {
    if (x < -STICK_DEADZONE) pressed.add("left");
    if (x > STICK_DEADZONE) pressed.add("right");
  } else {
    if (y < -STICK_DEADZONE) pressed.add("up");
    if (y > STICK_DEADZONE) pressed.add("down");
  }

  return pressed;
}

/// Reported at most once. A throw inside a rAF callback does not stop the loop
/// — the next frame was already scheduled — so without this the controller just
/// silently stops responding, which is precisely how a broken poll survived
/// several rounds of being reported and "fixed".
let complained = false;

function poll() {
  if (!running) return;
  requestAnimationFrame(poll);
  // Nothing to read while the window is not on screen: a button pressed behind
  // another application is not aimed at this one, and the emulator owns the pad
  // anyway during the one case where this window is hidden and a pad is in use.
  if (document.hidden) return;
  try {
    step();
  } catch (e) {
    console.error("gamepad poll failed", e);
    if (!complained) {
      complained = true;
      toast(`Controller input error: ${e.message}`);
    }
  }
}

/// Whether the lightbox was open on the previous poll.
let wasLightboxOpen = false;

function step() {

  const map = padMap();

  // Opening or closing the lightbox is a change of context, and anything the
  // pad was holding belonged to the old one: a direction held while the
  // lightbox opens would otherwise arrive inside it with its repeat timer
  // already running, so the first real press does nothing.
  //
  // But the button that caused the change is usually still down — Y opens the
  // player and Y also closes it — so the held set is rebuilt from what is
  // pressed rather than emptied. Emptying it made the poll immediately after
  // opening treat that same held Y as a new press, and the player opened and
  // shut in the same breath.
  const lightboxOpen = !el.lb.hidden;
  if (lightboxOpen !== wasLightboxOpen) {
    wasLightboxOpen = lightboxOpen;
    const now = performance.now();
    held.clear();
    for (const action of pressedActions(navigator.getGamepads?.() ?? [], map)) {
      held.set(action, now + FIRST_REPEAT_MS);
    }
  }

  // The lightbox used to swallow the pad completely, which meant the button
  // that opened a video could not close it — the only way out was the mouse.
  // Four actions get through: the one that opened it, Back, and the triggers,
  // which zoom what is on the stage rather than the covers behind it.
  if (!el.lb.hidden) {
    const now = performance.now();
    const pressed = pressedActions(navigator.getGamepads?.() ?? [], map);
    for (const action of pressed) {
      if (action === "video" || action === "back") {
        // Edge-triggered through the same `held` map as everything else, or a
        // held button would close the lightbox and then act on the library
        // underneath it in the same press.
        if (!held.has(action)) {
          held.set(action, now);
          closeLightbox();
          // Settle, so lifting the button does not register anywhere else.
          settling = true;
        }
      } else if (action === "activate") {
        // The bottom button pauses whatever is playing, the way it does in
        // every media player on a console. Edge-triggered: held down it would
        // toggle sixty times a second.
        if (!held.has(action)) {
          held.set(action, now);
          togglePlayback();
        }
      } else if (action === "zoomIn" || action === "zoomOut") {
        fire(action, now);
      } else if (action === "left" || action === "right") {
        // Through `fire` so holding a direction repeats, the same as it does
        // in the grid. Walking a dozen pictures one press at a time is the
        // kind of thing that makes people reach for the mouse again.
        if (!held.has(action)) {
          held.set(action, now + FIRST_REPEAT_MS);
          stepLightbox(action === "right" ? 1 : -1);
        } else if (now >= held.get(action)) {
          held.set(action, now + REPEAT_MS);
          stepLightbox(action === "right" ? 1 : -1);
        }
      }
    }
    for (const action of [...held.keys()]) {
      if (!pressed.has(action)) held.delete(action);
    }
    return;
  }

  // A dialog has taken over the window: the save-conflict question, the
  // "saves are not syncing" warning, the take-offline pane, a right-click
  // menu. All of them were mouse-only, which on a machine being used from a
  // sofa means a launch can stop dead on a question that cannot be answered.
  //
  // The same four actions serve every one of them, because a dialog is a list
  // of buttons: move through them, press one, or back out.
  const dialog = openDialog();
  if (dialog) {
    const now = performance.now();
    const pressed = pressedActions(navigator.getGamepads?.() ?? [], map);
    const items = focusables(dialog);
    for (const action of pressed) {
      if (action === "activate") {
        if (!held.has(action)) {
          held.set(action, now);
          (focused(items) ?? items[0])?.click();
          settling = true;
        }
      } else if (action === "back" || action === "back2") {
        if (!held.has(action)) {
          held.set(action, now);
          dismiss(dialog, items);
          settling = true;
        }
      } else if (["up", "left", "down", "right"].includes(action)) {
        const step = action === "up" || action === "left" ? -1 : 1;
        if (!held.has(action)) {
          held.set(action, now + FIRST_REPEAT_MS);
          move(items, step);
        } else if (now >= held.get(action)) {
          held.set(action, now + REPEAT_MS);
          move(items, step);
        }
      }
    }
    for (const action of [...held.keys()]) {
      if (!pressed.has(action)) held.delete(action);
    }
    return;
  }

  // Settings is different: the pad must still be able to close it, or a
  // controller-only user is trapped the moment they open it. Everything else
  // in there is bound by pressing buttons, which reads the pad directly.
  if (document.getElementById("settings")) {
    const backIndex = Object.entries(map).find(([, a]) => a === "back")?.[0];
    const down = (navigator.getGamepads?.() ?? []).some(
      (p) => p && backIndex !== undefined && p.buttons[backIndex]?.pressed
    );
    if (down) {
      if (!held.has("closeSettings")) {
        held.set("closeSettings", performance.now());
        runAction("settings");
      }
    } else {
      held.delete("closeSettings");
    }
    return;
  }

  // Nothing here is ours while the emulator owns the screen.
  if (suspended) return;

  const now = performance.now();
  const pressed = pressedActions(navigator.getGamepads?.() ?? [], map);

  if (settling) {
    if (settleLifted(pressed, now)) settling = false;
    return;
  }

  // The triggers scroll the list, proportionally. Handled here rather than as
  // an action because it is continuous: how far the trigger is pulled decides
  // the speed, which a press-and-repeat cannot express. `analogueScroll`
  // reports which actions it consumed, so they do not also fire as presses —
  // a trigger past its threshold reads as "pressed" too, and without this it
  // would scroll a page per repeat on top of the smooth movement.
  const smooth = analogueScroll(navigator.getGamepads?.() ?? [], map);
  const long = longPress(pressed, now);
  for (const action of pressed) {
    if (!smooth.has(action) && !long.has(action)) fire(action, now);
  }
  // Release anything no longer held so the next press fires immediately.
  for (const action of [...held.keys()]) {
    if (!pressed.has(action)) held.delete(action);
  }
}

/// Hold a button rather than pressing it, for the one action worth two.
///
/// Only the picture cycle so far: it is seven long, and the miximage is the
/// one people come back to — a screenshot, the box and the logo in one
/// picture, which is the setting that suits every console at once. Six presses
/// to get home from the wrong end of a list is how a good control becomes an
/// annoying one.
///
/// Returns the actions this consumed, so a hold does not also fire as a press
/// on the way down. A tap is unaffected: `fire` is skipped only once the hold
/// has actually happened, which is two seconds after the button went down.
const HOLD_MS = 2000;
const holdStart = new Map();

function longPress(pressed, now) {
  const consumed = new Set();
  for (const [action, fn] of Object.entries(HOLD_ACTIONS)) {
    if (!pressed.has(action)) {
      holdStart.delete(action);
      continue;
    }
    const since = holdStart.get(action);
    if (since === undefined) {
      holdStart.set(action, now);
    } else if (since !== "done" && now - since >= HOLD_MS) {
      // Marked rather than deleted, so it fires once for the hold and not
      // again every frame the button stays down.
      holdStart.set(action, "done");
      fn();
    }
    if (since === "done") consumed.add(action);
  }
  return consumed;
}

const HOLD_ACTIONS = { pictures: () => useMiximage() };

/// Scroll the list by however hard the triggers are pulled.
///
/// Returns the actions it dealt with, so the caller does not fire them again
/// as ordinary presses.
///
/// A trigger on a standard pad reports a value from 0 to 1 as well as a
/// pressed flag, which is the only analogue control here besides the sticks —
/// and a list of two thousand games is exactly what an analogue control is
/// for. A pad whose triggers are digital reports 0 or 1 and falls through to
/// the press handler, which scrolls a fixed step.
export function analogueScroll(pads, map) {
  const done = new Set();
  const pad = primaryPad(pads);
  if (!pad) return done;

  let amount = 0;
  for (const [index, action] of Object.entries(map)) {
    if (action !== "scrollUp" && action !== "scrollDown") continue;
    const button = pad.buttons[index];
    // `value` where the pad reports one; a digital trigger only has `pressed`,
    // and is left to the press handler so it still does something.
    const pull = button?.value ?? 0;
    if (pull <= TRIGGER_DEADZONE) continue;
    // Squared, so a light pull creeps and a full pull moves properly. The same
    // shape the right stick uses on the info pane.
    const speed = pull ** 2 * TRIGGER_TOP_SPEED;
    amount += action === "scrollUp" ? -speed : speed;
    done.add(action);
  }
  if (amount) scrollList(amount);
  return done;
}

/// The modal currently over the window, if any.
///
/// Listed rather than detected, because "is this element a dialog" has no
/// answer a page can give: a right-click menu and a save-conflict question are
/// both just divs.
function openDialog() {
  return document.querySelector("#conflict-overlay, #bulk-overlay, .ctx-menu");
}

/// The things inside it a person could press, in the order they are drawn.
function focusables(dialog) {
  return [
    ...dialog.querySelectorAll(
      "button:not([disabled]), select:not([disabled]), input:not([disabled]):not([type=hidden])"
    ),
  ];
}

function focused(items) {
  return items.find((n) => n === document.activeElement);
}

/// Move the highlight, wrapping. A dialog is a short list and running off the
/// end of it is more annoying than coming round again.
function move(items, delta) {
  if (!items.length) return;
  const at = items.indexOf(document.activeElement);
  const next = items[(at + delta + items.length) % items.length] ?? items[0];
  next.focus();
  for (const n of items) n.classList.toggle("pad-focus", n === next);
}

/// Back out: whatever the dialog calls cancelling, or simply closing it.
function dismiss(dialog, items) {
  const cancel =
    dialog.querySelector(".bulk-cancel, [data-go=\"no\"], .cancel") ??
    // A right-click menu has no cancel; going back means it goes away.
    null;
  if (cancel) {
    cancel.click();
    return;
  }
  dialog.remove();
  void items;
}

/// Run one poll, for tests. The loop itself is driven by rAF, which a test
/// cannot wait on deterministically.
export function stepForTest() {
  step();
}

export function installGamepad() {
  if (!navigator.getGamepads) return;

  window.addEventListener("gamepadconnected", (ev) => {
    // id is a vendor string like "Xbox Wireless Controller (STANDARD GAMEPAD)".
    const name = ev.gamepad.id.replace(/\s*\(.*\)\s*$/, "").trim();
    // Not on Android: the handheld *is* the controller, so announcing that one
    // is attached is announcing that the device exists. It fired on every
    // launch, and it is the thing Frank saw as a white tint.
    if (!MOBILE) toast(`Controller connected — ${name || "gamepad"}`);
    state.gamepad = name;
    // Reveals the shoulder-button hints in the tab bar. Hidden without a pad,
    // where they would be advice for hardware that is not there.
    document.body.classList.add("has-pad");
    if (!running) {
      running = true;
      requestAnimationFrame(poll);
    }
  });

  window.addEventListener("gamepaddisconnected", () => {
    if (!(navigator.getGamepads() ?? []).some(Boolean)) {
      running = false;
      held.clear();
      state.gamepad = null;
      // The hints stay on Android: the built-in pad has not gone anywhere,
      // the browser has merely stopped reporting it.
      if (!MOBILE) {
        document.body.classList.remove("has-pad");
        toast("Controller disconnected");
      }
    }
  });

  // A pad paired before the window opened only appears once it is polled.
  if ((navigator.getGamepads() ?? []).some(Boolean)) {
    running = true;
    requestAnimationFrame(poll);
  }

  // Android: assume the pad, then look for it.
  //
  // `navigator.getGamepads()` deliberately returns nothing until a button has
  // been pressed — the browser will not tell a page what hardware is attached
  // until the user has interacted with it. On a desktop that is invisible: you
  // reach for the mouse. On a handheld the pad is the only input there is, so
  // the app sat there polling nothing and ignoring every press until one of
  // them happened to register, which reads as several seconds of a frozen app
  // on every launch.
  //
  // This device *is* a controller. So the poll starts immediately and the
  // shoulder hints are shown immediately, and the probe below keeps looking
  // until the browser admits what is plainly there. Nothing waits on it.
  if (MOBILE) {
    document.body.classList.add("has-pad");
    if (!running) {
      running = true;
      requestAnimationFrame(poll);
    }
    // The name is worth having — it picks the button labels — but it only
    // arrives with the first press, so it is watched for rather than waited on.
    const probe = setInterval(() => {
      const pad = (navigator.getGamepads() ?? []).find(Boolean);
      if (!pad) return;
      clearInterval(probe);
      state.gamepad = pad.id.replace(/\s*\(.*\)\s*$/, "").trim();
    }, 1000);
    // Two minutes of looking is plenty; a pad connected after that raises
    // `gamepadconnected` like any other.
    setTimeout(() => clearInterval(probe), 120000);
  }
}

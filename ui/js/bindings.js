// Keyboard and controller bindings — a window onto the backend, and no
// decisions of its own.
//
// The tables, the defaults, the repair rules and the storage all live in
// `src/binds.rs` now. They were here, which meant the TUI could not read a
// keybinding and an SDL front end would have had to reimplement every rule
// from the comments around them.
//
// Two things this file still does, and both are about speed. It holds the
// resolved tables in a variable so the accessors stay *synchronous*: the
// controller poll runs inside requestAnimationFrame, 120 times a second on
// this display, and `padMap()` there cannot await anything. And it reloads on
// `bindings-changed`, because the settings window is a separate document —
// rebinding there changes the file, and this document has to be told.

const invoke = (...args) => window.__TAURI__.core.invoke(...args);
// Read here rather than imported: this file deliberately depends on nothing,
// so that the settings window and the library window can both load it without
// dragging the rest of the app in behind it.
const MOBILE = /\bAndroid\b/.test(navigator.userAgent);
const emit = (name) => window.__TAURI__?.event?.emit?.(name);

/// The resolved tables, as `ui_bindings` returns them. Null until loaded.
///
/// Every accessor below reads this and never computes anything from it beyond
/// a lookup — the moment one of them starts deciding something, there are two
/// implementations of that decision again.
let table = null;

/// Sensible nothing, for the window between startup and the first load. An
/// accessor that throws there would take out whatever called it, and what
/// calls these is the key handler.
const EMPTY = {
  actions: [],
  pad_buttons: [],
  pad_map: {},
  keys: {},
  pad_labels: {},
  key_labels: {},
};

const now = () => table ?? EMPTY;

/// Fetch the tables. Called once at startup, before the keyboard or the pad is
/// installed, and again whenever something is rebound.
export async function loadBindings() {
  await adoptOldStorage();
  table = await invoke("ui_bindings");
  androidDefaults();
  return table;
}

/// Start opens Settings on Android.
///
/// The shared default gives Start the help card, on the reasoning that Settings
/// was a second window of text fields and tables a pad could not navigate — so
/// the button opened something you then could not use. That is no longer true
/// here: on Android Settings is a full page reached by navigation, driven by
/// the same directions as everything else, and Back leaves it. With the top bar
/// gone it is also the only way in.
///
/// A default, not an override: a button somebody has deliberately bound to
/// something else is left alone, so this cannot undo a choice.
function androidDefaults() {
  if (!MOBILE || !table?.pad_map) return;
  const START = 9;
  const bound = table.pad_map[START];
  if (!bound || bound === "help") table.pad_map[START] = "settings";
}

/// Hand over bindings a previous version left in this document's own storage.
///
/// They lived in localStorage, which the TUI cannot read and which the
/// settings window — being a second document — kept a separate copy of, synced
/// by listening for `storage` events. Moving them to config.toml is what
/// retires that; this is the one-way door people walking in from an older
/// build go through. The backend keeps anything already in the file, so this
/// cannot undo a rebind, and the keys are cleared afterwards so it happens
/// once.
const OLD_KEYS = "keybindings";
const OLD_PAD = "moose.pad";

async function adoptOldStorage() {
  let keys = null;
  let pad = null;
  try {
    keys = JSON.parse(localStorage.getItem(OLD_KEYS) || "null");
    pad = JSON.parse(localStorage.getItem(OLD_PAD) || "null");
  } catch {
    // Unparseable is the same as absent: there is nothing to carry over and
    // failing here would stop the app before it drew anything.
  }
  if (!keys && !pad) return;
  try {
    await invoke("import_bindings", { keys: keys ?? {}, pad: pad ?? {} });
    localStorage.removeItem(OLD_KEYS);
    localStorage.removeItem(OLD_PAD);
  } catch (e) {
    // Left in place to try again next launch rather than thrown away.
    console.warn("carrying over old bindings:", e);
  }
}

// Rebinding happens in the settings window, which is a separate document with
// its own copy of this module. The file is the truth; both copies re-read it.
if (typeof window !== "undefined") {
  window.__TAURI__?.event?.listen?.("bindings-changed", () => {
    invoke("ui_bindings").then((t) => (table = t)).catch(() => {});
  });
}

/// Every action, in the order Settings lists them.
export function actions() {
  return now().actions;
}

/// Controller buttons by W3C "standard mapping" index, with their names — 0 is
/// the bottom face button, which is A on Xbox, Cross on PlayStation and B on a
/// Nintendo pad.
export function padButtons() {
  return now().pad_buttons;
}

/// Button index -> action, with `null` where a rebind cleared the button.
///
/// The nulls are kept rather than dropped: "bound to nothing" and "not a
/// button on this pad" are different answers to why a press did nothing, and
/// the settings window has to be able to say which. Anything dispatching from
/// this has to skip them.
export function padMap() {
  return now().pad_map;
}

/// Which button currently triggers `action`, or null.
export function padFor(action) {
  for (const [index, bound] of Object.entries(now().pad_map)) {
    if (bound === action) return Number(index);
  }
  return null;
}

/// Current key for an action, or null when unbound.
export function keyFor(id) {
  return now().keys[id] ?? null;
}

/// Action bound to a pressed key, or null.
///
/// Case-insensitive for single characters so a binding works whether or not
/// Shift is held; anything longer is a named key where case is part of it.
export function actionFor(key) {
  const want = key.length === 1 ? key.toLowerCase() : key;
  for (const [id, bound] of Object.entries(now().keys)) {
    if (!bound) continue;
    if ((bound.length === 1 ? bound.toLowerCase() : bound) === want) return id;
  }
  return null;
}

/// What to print on the button that rebinds this action.
export function keyLabelFor(id) {
  return now().key_labels[id] ?? "—";
}

export function padLabelFor(id) {
  return now().pad_labels[id] ?? "unset";
}

/// The name of one controller button, for the live readout that answers "which
/// index is this button" — the question the bindings themselves cannot.
export function padLabel(index) {
  if (index === null || index === undefined) return "unset";
  return padButtons().find((b) => b.index === index)?.name ?? `button ${index}`;
}

/// Bind `action` to `index`, clearing whatever else held that button. A null
/// index unbinds.
export async function setPad(action, index) {
  table = await invoke("set_pad_binding", { action, index });
  emit("bindings-changed");
}

export async function setKey(id, key) {
  table = await invoke("set_key_binding", { action: id, key: key ?? null });
  emit("bindings-changed");
}

export async function resetAll() {
  table = await invoke("reset_bindings", { which: "keys" });
  emit("bindings-changed");
}

export async function resetPad() {
  table = await invoke("reset_bindings", { which: "pad" });
  emit("bindings-changed");
}

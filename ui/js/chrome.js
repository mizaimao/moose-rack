// The words on the toolbar buttons, and the switch that takes them away.
//
// Every one of those buttons is an icon and a word: List, Filter, Random, Name,
// Take offline, Game info. The words are what make the bar readable the first
// week and what makes it noisy afterwards, so this is a preference rather than
// a decision — the same shape as the grid/list and shell-mode settings, and it
// lives beside them in Settings.
//
// What is *not* optional is that the button still says what it is. Hiding a
// label leaves `title` as the accessible name, so every one of the six carries
// one, and the three whose word is a piece of state rather than a name --
// what the list is sorted by, which pane the toggle shows, which way the
// layout will flip -- put that state in the title too. See `paintChromeState`.

const KEY = "chromeLabels";

/// Whether the words are shown. On unless it has been turned off.
export function labelsWanted() {
  try {
    return localStorage.getItem(KEY) !== "off";
  } catch {
    // A private window shows them, which is the readable answer of the two.
    return true;
  }
}

/// Put the choice on the document, where the stylesheet can see it.
export function applyLabels(on = labelsWanted()) {
  document.documentElement.classList.toggle("no-labels", !on);
  return on;
}

/// Remember it, apply it here, and tell the other window.
///
/// `announce` is off when this *is* the answer to somebody else's message, or
/// the two windows would talk to each other in a circle.
export function setLabels(on, { announce = true } = {}) {
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    // Not remembered is not the same as not applied.
  }
  applyLabels(on);
  if (announce) window.__TAURI__?.event?.emit?.("chrome-labels", on);
  return on;
}

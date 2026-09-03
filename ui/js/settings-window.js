// The Settings window.
//
// Its own window rather than an overlay on the library. The old panel was one
// scrolling column holding emulator paths, sync buttons and both binding tables,
// which put the two longest things in the app between the user and everything
// else. Tabs down the left, one pane at a time, and the window is resizable.
//
// Runs in a separate document from index.html, so nothing here may touch the
// main window's DOM. The panes were written for that constraint: they render
// into whatever element they are given.

import { TABS, paneHtml, wirePane, isCapturing, captureKey } from "./settings-panes.js";
import { applyStoredGlassTint } from "./backdrop.js";
import { loadBindings } from "./bindings.js";

const tabsEl = document.getElementById("tabs");
const paneEl = document.getElementById("pane");
const toastEl = document.getElementById("settings-toast");

/// The tab last looked at, so reopening the window returns to where you were
/// rather than always to General.
const REMEMBERED = "settings-tab";

let current = null;

function show(id) {
  if (!TABS.some((t) => t.id === id)) id = TABS[0].id;
  current = id;
  localStorage.setItem(REMEMBERED, id);

  for (const btn of tabsEl.children) {
    const on = btn.dataset.id === id;
    btn.classList.toggle("active", on);
    btn.setAttribute("aria-selected", String(on));
  }

  paneEl.innerHTML = paneHtml(id);
  // Wiring happens after the markup is in the document: the panes look their
  // controls up by class, and an element that is not attached yet has no
  // computed style for the ones that read theme colors.
  wirePane(id, paneEl);
  paneEl.scrollTop = 0;
}

function buildTabs() {
  tabsEl.replaceChildren();
  for (const t of TABS) {
    const btn = document.createElement("button");
    btn.className = "tab";
    btn.dataset.id = t.id;
    btn.textContent = t.label;
    btn.setAttribute("role", "tab");
    btn.addEventListener("click", () => show(t.id));
    tabsEl.appendChild(btn);
  }
}

/// Up and down move between tabs, so this window is usable from a keyboard
/// without reaching for the mouse. Escape closes it.
///
/// Both are suppressed while a binding is being captured: the whole point of
/// that mode is that the next key press belongs to the binding, not to the UI.
document.addEventListener(
  "keydown",
  (ev) => {
    if (isCapturing()) {
      if (captureKey(ev)) return;
      return;
    }
    const idx = TABS.findIndex((t) => t.id === current);
    if (ev.key === "Escape") {
      ev.preventDefault();
      close();
    } else if (ev.key === "ArrowDown" && tabsEl.contains(document.activeElement)) {
      ev.preventDefault();
      show(TABS[Math.min(idx + 1, TABS.length - 1)].id);
      tabsEl.children[Math.min(idx + 1, TABS.length - 1)].focus();
    } else if (ev.key === "ArrowUp" && tabsEl.contains(document.activeElement)) {
      ev.preventDefault();
      show(TABS[Math.max(idx - 1, 0)].id);
      tabsEl.children[Math.max(idx - 1, 0)].focus();
    }
  },
  true
);

function close() {
  // On Android this page *is* the webview, reached by navigating rather than by
  // opening a window Tauri cannot close afterwards. Going back is a navigation
  // too. See settings.js for the two approaches this replaced.
  if (/\bAndroid\b/.test(navigator.userAgent)) {
    window.location.href = "index.html";
    return;
  }
  // A real window, which is every desktop build. getCurrentWindow rather than a
  // named lookup: this file is only ever loaded into the settings window, and
  // asking which window we are in cannot go stale.
  window.__TAURI__?.window?.getCurrentWindow?.().close();
}

/// Answer Android's Back button from inside the overlay.
///
/// On Android this page is the whole webview, so the activity asks it directly.
/// Always true: Back inside settings means leave settings, at every level.
window.__androidBack = () => {
  close();
  return true;
};

/// The panes call `toast` from util.js, which writes into the main window's
/// footer — an element this document does not have. Redirected here so a status
/// message lands in this window instead of vanishing.
window.addEventListener("settings-toast", (ev) => {
  toastEl.textContent = ev.detail;
  toastEl.hidden = false;
  clearTimeout(toastEl._t);
  toastEl._t = setTimeout(() => (toastEl.hidden = true), 4000);
});

/// Both version numbers at the foot of the rail.
///
/// Left hidden if the call fails rather than showing a blank or a guess: a
/// wrong version number is worse than none, since the whole point of it is to
/// answer "are these two machines running the same thing".
async function showVersions() {
  const box = document.getElementById("versions");
  if (!box) return;
  try {
    const [client, server] = await window.__TAURI__.core.invoke("versions");
    box.innerHTML =
      `Moose Rack <strong>${client}</strong>` +
      (server ? `<br>server <strong>${server}</strong>` : "");
    box.hidden = false;
  } catch {
    box.hidden = true;
  }
}

applyStoredGlassTint();
showVersions();
buildTabs();
// The bindings before the first pane, because the Control tab is a row per
// action with the key and button currently on it — and this window is a second
// document, so it has its own copy of them to fill.
loadBindings()
  .catch((e) => console.warn("loading bindings:", e))
  .finally(() => show(localStorage.getItem(REMEMBERED) || TABS[0].id));

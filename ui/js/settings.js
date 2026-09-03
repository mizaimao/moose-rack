// Opening the Settings window from the main one.
//
// Everything that used to be here — the panel markup, the binding tables, the
// capture logic — moved to settings-panes.js and now runs inside its own
// window. What is left is the door.

import { invoke } from "./state.js";
import { toast } from "./util.js";

/// Nothing in the main window captures keys any more; that happens in the
/// settings window, against its own document. Kept so keys.js has one less
/// thing to know about which window it is in.
export function isCapturing() {
  return false;
}

export function captureKey() {
  return false;
}

/// Android has no second window, so settings takes over this one.
///
/// Three ways were tried and only this one works.
///
/// A real second window is what desktop uses. Tauri will build one on Android
/// and then cannot dispose of it: `close()` never settles, `destroy()` and
/// `hide()` return quietly and change nothing. Settings became a room with no
/// door.
///
/// An iframe closes fine — it is an element — but Tauri's IPC does not reach
/// into one. `window.__TAURI__` is injected and looks right, `invoke` sends,
/// and the reply never comes back because responses only route to the top
/// frame. Every control that reads its value hung on that: the toggles kept
/// their "…" placeholder, the dropdowns stayed empty, the theme list read
/// forever, and the bindings table drew no rows. Worse than not closing,
/// because it looked like it worked.
///
/// So settings.html is loaded as the whole document. It is the top frame, so
/// invoke works, the dialog plugin works, and its own stylesheet applies —
/// nothing special-cased, the same page desktop opens in a window. It costs
/// the library page a reload when you come back, which on a handheld where
/// settings fills the screen anyway is a fair price for controls that work.
const MOBILE = /\bAndroid\b/.test(navigator.userAgent);

/// Never open in *this* document, on any platform: settings is always its own
/// page now — a window on desktop, the whole webview on Android.
export function settingsOpen() {
  return false;
}

export function closeSettings() {}

export async function toggleSettings() {
  if (MOBILE) {
    window.location.href = "settings.html";
    return;
  }
  try {
    await invoke("open_settings");
  } catch (e) {
    toast(`Could not open Settings — ${e}`, 6000);
  }
}

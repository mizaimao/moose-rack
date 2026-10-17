// Which drawing of the settings pane is the one on screen.
//
// Every tab is drawn into the same `#pane`, so "is my pane still there" cannot
// be asked of the element: `box.isConnected` is true for whichever tab is
// showing. The panes asked it anyway, and it never said no. An answer that
// came back after a tab switch wired the next tab's controls a second time,
// so a toggle saved twice and "Edit…" opened two dialogs, and each visit to
// Control started another controller readout that never stopped.
//
// So each drawing gets a signal, aborted when the next one starts. A slow
// answer checks it before touching the pane, and anything hung on something
// that outlives the pane (the window, a backend event, a timer) lets go when it
// fires.

import { listen } from "../state.js";

let current = null;

/// Start drawing the pane afresh, ending the drawing before it.
export function beginRender() {
  current?.abort();
  current = new AbortController();
  return current.signal;
}

/// `target.addEventListener`, removed again when the drawing ends.
///
/// Without a signal it is a plain listener, for a pane wired by hand.
export function listenWhile(signal, target, type, fn) {
  if (signal?.aborted) return;
  target.addEventListener(type, fn);
  signal?.addEventListener("abort", () => target.removeEventListener(type, fn), { once: true });
}

/// A backend event, heard for as long as the drawing lasts.
export async function onEventWhile(signal, event, fn) {
  if (signal?.aborted) return;
  const stop = await listen(event, fn);
  // The tab may have changed while the listener was being set up.
  if (signal?.aborted) return stop?.();
  signal?.addEventListener("abort", () => stop?.(), { once: true });
}

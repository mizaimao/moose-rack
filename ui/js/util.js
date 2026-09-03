// Formatting and small DOM helpers.

import { el } from "./state.js";

export function human(bytes) {
  if (!bytes) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(1)} ${units[i]}`;
}

export function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]
  );
}

/// Omit a definition row entirely when empty, rather than rendering a blank.
export function row(label, value) {
  if (value === null || value === undefined || value === "") return "";
  return `<dt>${label}</dt><dd>${escapeHtml(String(value))}</dd>`;
}

/// RomM stores ratings 0-100; show five stars plus the raw number.
/// Five stars filled to `rating` out of 100.
///
/// Two rows of the same five characters, one over the other, the top one
/// clipped to the score. Nothing else needs a glyph that might not exist: the
/// half star was U+2BE8, which almost no system font carries, so it rendered as
/// the browser's missing-glyph box — a striped rectangle sitting in the middle
/// of the row.
///
/// Clipping also gets the score right rather than near it. 69/100 is 3.45
/// stars; rounding to the nearest half showed 3.5 and threw away the
/// difference between 69 and 71.
export function starBar(rating) {
  const pct = Math.max(0, Math.min(100, Number(rating) || 0));
  return `<div class="rating">
      <span class="stars" role="img" aria-label="${Math.round(pct)} out of 100">
        <span class="stars-off">★★★★★</span>
        <span class="stars-on" style="width:${pct}%">★★★★★</span>
      </span>
      <span class="num">${Math.round(pct)}/100</span>
    </div>`;
}

let toastTimer;
export function toast(msg, ms = 4000) {
  // Missing element is not worth throwing over. This is called from async
  // click handlers, where a throw is swallowed by the promise and the handler
  // simply stops half-done — the failure mode that hid the A-button bug for
  // five rounds. Losing a status line is the smaller problem.
  if (!el.toast) {
    // The Settings window has its own document and no #toast footer, so a
    // status message there is dispatched for that window to place rather than
    // being dropped.
    if (typeof window !== "undefined" && document.getElementById("settings-toast")) {
      window.dispatchEvent(new CustomEvent("settings-toast", { detail: msg }));
      return;
    }
    console.warn("toast:", msg);
    return;
  }
  el.toast.textContent = msg;
  el.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.toast.hidden = true), ms);
}

/// A color from a CSS custom property, for the places that need a value
/// rather than a stylesheet — a `<input type="color">` cannot be given
/// `var(--bg)`, it needs six hex digits.
///
/// This was called in the Appearance pane and defined nowhere, so wiring that
/// tab threw a ReferenceError partway through and everything after it — the
/// backdrop's own controls — was never connected.
export function cssColor(name, fallback) {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  // Only the form a color input accepts. A token holding `color-mix(...)` or
  // a name is perfectly valid CSS and useless here.
  return /^#[0-9a-f]{6}$/i.test(raw) ? raw : fallback;
}

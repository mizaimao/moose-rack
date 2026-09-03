// Full-size artwork over a dimmed backdrop, inside the main window.
//
// In-window rather than a second OS window: no dock icon, no window
// management, and Esc closes it.

import { el, convertFileSrc } from "./state.js";

const lb = { items: [], index: 0, open: false, zoom: 1 };

/// How far in and out the zoom goes, and by how much per press.
const ZOOM_MIN = 0.5;
const ZOOM_MAX = 3;
const ZOOM_STEP = 0.15;

export function isLightboxOpen() {
  return lb.open;
}

/// Scale what is on the stage.
///
/// A transform rather than a width, because it applies identically to a video,
/// a still and a manual, and because scaling a video's width makes the browser
/// re-lay-out every frame it draws.
export function zoomLightbox(direction) {
  if (!lb.open) return;
  lb.zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, lb.zoom + direction * ZOOM_STEP));
  el.lb.style.setProperty("--lb-zoom", String(lb.zoom));
}

/// Called before opening so a running slideshow cannot swap the image out.
let onOpen = () => {};
export function setOpenHook(fn) {
  onOpen = fn;
}

export function openLightbox(items, index = 0) {
  if (!items.length) return;
  // Zoom resets per opening: coming back to a picture at whatever scale the
  // last one was left at is disorienting, and there is nothing on screen to
  // explain it.
  Object.assign(lb, { items, index, open: true, zoom: 1 });
  el.lb.style.setProperty("--lb-zoom", "1");
  el.lb.hidden = false;
  onOpen();
  render();
}

export function closeLightbox() {
  lb.open = false;
  el.lb.hidden = true;
  // Clear the stage so a playing video does not keep its audio going.
  el.lb.querySelector(".lb-stage").innerHTML = "";
  // And let go of the bytes, when the video was one this window is holding.
  // Android cannot play the file directly, so it is fetched into a blob; a blob
  // that is never revoked is a few megabytes kept for the life of the window,
  // once per video watched. Imported lazily because detail.js already imports
  // this module, and a static pair would be a cycle.
  import("./detail.js").then((d) => d.releaseVideo?.()).catch(() => {});
}

function step(delta) {
  if (!lb.open || lb.items.length < 2) return;
  lb.index = (lb.index + delta + lb.items.length) % lb.items.length;
  render();
}

function render() {
  const it = lb.items[lb.index];
  const stage = el.lb.querySelector(".lb-stage");
  stage.innerHTML =
    it.kind === "video"
      // Width, not just a maximum. These are 320x240 files more often than
      // not, and `max-width` never scales anything up — so the player opened
      // at the video's own size, a postage stamp in the middle of a 5K screen.
      ? `<video src="${it.src}" controls autoplay loop></video>`
      : it.kind === "pdf"
        // WKWebView renders PDFs natively, so an iframe is the whole viewer.
        ? `<iframe src="${it.src}" title="Manual"></iframe>`
        : `<img src="${it.src}" alt="" />`;
  el.lb.querySelector("figcaption").textContent =
    lb.items.length > 1
      ? `${it.caption} — ${lb.index + 1} of ${lb.items.length}`
      : it.caption;
  const multi = lb.items.length > 1;
  el.lb.querySelector(".lb-prev").disabled = !multi;
  el.lb.querySelector(".lb-next").disabled = !multi;
}

/// Step to the next or previous item. Exported so the pad and the keyboard go
/// through the same path the on-screen arrows do.
export function stepLightbox(delta) {
  step(delta);
}

el.lb.querySelector(".lb-close").addEventListener("click", closeLightbox);
el.lb.querySelector(".lb-prev").addEventListener("click", () => step(-1));
el.lb.querySelector(".lb-next").addEventListener("click", () => step(1));
// Clicking the backdrop closes; clicking the artwork itself does not.
el.lb.addEventListener("click", (ev) => {
  if (ev.target === el.lb || ev.target.tagName === "FIGURE") closeLightbox();
});
window.addEventListener("keydown", (ev) => {
  if (!lb.open) return;
  // Escape only. Left and right are bindable actions, so keys.js owns them —
  // handling them here as well would step twice per press for anyone who left
  // them on the arrow keys, and not at all for anyone who moved them.
  if (ev.key === "Escape") closeLightbox();
});

/// Play or pause whatever video is on the stage.
///
/// Returns false when there is nothing playing, so the caller can let the key
/// mean whatever it means elsewhere rather than swallowing it.
export function togglePlayback() {
  const video = document.querySelector("#lightbox video");
  if (!video) return false;
  if (video.paused) video.play().catch(() => {});
  else video.pause();
  return true;
}

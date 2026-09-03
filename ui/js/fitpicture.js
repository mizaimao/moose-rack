// Draw a picture at the size it is actually shown, and let go of the rest.
//
// Measured 2026-08-22, and this is the whole reason the file exists: WebKit
// decodes an image at the *file's* resolution, not the size it is drawn at.
// Twenty of Frank's 1,280x960 miximages, drawn into 200x150 boxes, cost
// 10.9 MB each — against 2.6 MB each for the same boxes filled from 325x600
// files. Same tiles, same layout, four times the memory, and the only thing
// that differed was the file. Ninety cards of that is around 980 MB, which is
// the arcade screen and the whole problem. See `docs/memory-footprint.md`.
//
// So: fetch the bytes, decode them once, draw the result into a canvas the
// size of the box, and close the decoded copy. What stays in memory is the
// canvas — a 200x150 box at 2x is 0.24 MB — instead of the decoded file.
//
// **Nothing here writes.** The file on disk is untouched, still exactly the
// bytes that were downloaded, and it is still what the lightbox and the
// full-size views open. This changes what is held in memory, not what is
// stored.

/// Never enlarge. A 224x256 titlescreen in a 400px box should stay 224x256 and
/// let the box letterbox it, the way `object-fit: contain` does — blowing it up
/// would cost four times the memory to show exactly the same detail.
const NEVER_UPSCALE = 1;

/// Draw `url` into a canvas that fits a `boxW` x `boxH` CSS-pixel box.
///
/// Returns the canvas, or `null` if anything at all went wrong — a caller that
/// gets `null` should fall back to a plain `<img>`, which is what the app did
/// before this and is never worse than showing nothing.
export async function fitted(url, boxW, boxH, dpr = window.devicePixelRatio || 1) {
  if (!(boxW > 0) || !(boxH > 0)) return null;
  if (typeof createImageBitmap !== "function") return null;
  // The measuring switch, so an A/B of this change needs one build and turns
  // one thing off. Never set in normal use.
  if (globalThis.__MOOSE_FLAGS?.includes("no-canvas")) return null;
  let bitmap = null;
  try {
    const response = await fetch(url);
    if (!response.ok) return null;
    const blob = await response.blob();
    // Decoded at full size on purpose. `createImageBitmap`'s `resizeWidth`
    // options would avoid even this, but Safari has historically ignored them
    // and a silently-ignored option is worse than none — it would look like it
    // worked. The full decode is a spike of one picture, not a resident set of
    // ninety, and `library.js` allows only six of these at a time.
    bitmap = await createImageBitmap(blob);
    const scale = Math.min(
      NEVER_UPSCALE,
      (boxW * dpr) / bitmap.width,
      (boxH * dpr) / bitmap.height
    );
    const w = Math.max(1, Math.round(bitmap.width * scale));
    const h = Math.max(1, Math.round(bitmap.height * scale));

    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(bitmap, 0, 0, w, h);

    // The canvas is in device pixels; the page lays out in CSS pixels.
    canvas.style.width = `${w / dpr}px`;
    canvas.style.height = `${h / dpr}px`;
    return canvas;
  } catch {
    return null;
  } finally {
    // The decoded copy goes now rather than whenever the collector gets round
    // to it. This one line is the difference between a spike and a leak.
    bitmap?.close?.();
  }
}

/// How big a box is, in CSS pixels, falling back to what CSS says it should be
/// when it has not been laid out yet.
export function boxSize(el) {
  const w = el.clientWidth;
  const h = el.clientHeight;
  if (w > 0 && h > 0) return [w, h];
  const css = el.ownerDocument?.defaultView?.getComputedStyle?.(el);
  return [parseFloat(css?.width) || 0, parseFloat(css?.height) || 0];
}

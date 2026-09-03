// The color of a game's box art, for the selection glow.
//
// One number per cover, worked out once and remembered. The cover is drawn
// into an 8x8 canvas and the 64 pixels are averaged — the browser does the
// scaling in the graphics driver, so the cost is one small draw call and 256
// bytes read back, not a walk over a 500x700 image. On this library that is
// under a millisecond per cover and never happens twice for the same game.
//
// Why a color rather than a border: a highlight in the game's own colors
// tells you which game is selected *and* something about the game, from the
// corner of your eye, without adding a single button or bar.
//
// ## Why the canvas is readable
//
// Reading pixels back from an image loaded off another origin taints the
// canvas and `getImageData` throws. Covers come through Tauri's asset
// protocol, which is a different origin from the page — but it answers with
// `Access-Control-Allow-Origin` set to this window's exact origin, so
// requesting the image with `crossOrigin = "anonymous"` satisfies the check and
// the canvas stays clean.
//
// That is a promise about someone else's code, so it is not assumed: the first
// SecurityError switches the whole feature off for the session rather than
// throwing once per cover. The interface falls back to the theme color, which
// is what it used before this existed.

/// url -> "r g b", or null for covers with no usable color.
const cache = new Map();

/// Cleared on the first sign that pixels cannot be read here.
let readable = true;

/// Small enough that the work is in the scaler, big enough that a cover is not
/// reduced to its background: an 8x8 keeps a logo or a character that fills a
/// reasonable part of the frame.
const GRID = 8;

let canvas = null;

function context() {
  if (!canvas) {
    canvas = document.createElement("canvas");
    canvas.width = GRID;
    canvas.height = GRID;
  }
  // `willReadFrequently` tells the browser to keep this canvas on the CPU;
  // without it every read forces a round trip back from the GPU.
  return canvas.getContext("2d", { willReadFrequently: true });
}

/// The dominant color of `url`, as an `"r g b"` triple, or null.
///
/// Never rejects. A cover that fails to load, or a color not worth using, is
/// null and the caller keeps the theme color.
export async function tintFor(url) {
  if (!url || !readable) return null;
  if (cache.has(url)) return cache.get(url);

  const value = await measure(url).catch(() => null);
  cache.set(url, value);
  return value;
}

function measure(url) {
  return new Promise((resolve) => {
    const img = new Image();
    // Must be set before `src`, or the request goes out without the CORS
    // headers and the canvas is tainted even though the server would have
    // allowed it.
    img.crossOrigin = "anonymous";
    img.onerror = () => resolve(null);
    img.onload = () => {
      try {
        const ctx = context();
        ctx.clearRect(0, 0, GRID, GRID);
        ctx.drawImage(img, 0, 0, GRID, GRID);
        resolve(pick(ctx.getImageData(0, 0, GRID, GRID).data));
      } catch {
        // Almost certainly a SecurityError from a tainted canvas. Whatever it
        // was, it will happen for every other cover too.
        readable = false;
        resolve(null);
      }
    };
    img.src = url;
  });
}

/// Average the pixels, weighted towards the colorful ones.
///
/// A flat average of box art is nearly always a muddy brown: most covers are
/// mostly background, and averaging a bright logo with a dark border gives
/// neither. Weighting by how far a pixel is from gray lets the color that
/// makes the cover recognizable win, which is the one worth showing.
export function pick(data) {
  let r = 0;
  let g = 0;
  let b = 0;
  let weight = 0;

  for (let i = 0; i < data.length; i += 4) {
    // Fully transparent pixels are padding around a non-square cover.
    if (data[i + 3] < 16) continue;
    const [pr, pg, pb] = [data[i], data[i + 1], data[i + 2]];
    const max = Math.max(pr, pg, pb);
    const min = Math.min(pr, pg, pb);
    // Distance from gray, and away from both black and white — a pixel at
    // either end carries no hue to borrow.
    const w = (max - min) / 255 + 0.15 * (1 - Math.abs(max + min - 255) / 255);
    r += pr * w;
    g += pg * w;
    b += pb * w;
    weight += w;
  }

  if (weight < 0.001) return null;
  return normalize(r / weight, g / weight, b / weight);
}

/// Lift the color to something that reads as light on a dark interface.
///
/// Hue is what identifies the game; brightness is what makes it visible. So the
/// color is taken apart, the hue kept exactly as measured, and only saturation
/// and lightness moved into a band that works as a glow.
///
/// Scaling the channels instead — the obvious version — cannot do this. A cover
/// that is mostly deep blue has every channel low, and multiplying by a bounded
/// factor leaves it dim; multiplying by an unbounded one blows the largest
/// channel to white and takes the hue with it.
function normalize(r, g, b) {
  const [h, s, l] = toHsl(r / 255, g / 255, b / 255);

  // Nearly gray. There is no hue here to borrow, and stretching what little
  // there is would invent a color the cover does not have.
  if (s < 0.15) return null;

  return toRgb(h, clamp(s, 0.45, 0.9), clamp(l, 0.55, 0.72))
    .map((c) => Math.round(c * 255))
    // Space separated, which is what `rgb(r g b / alpha)` wants — the comma
    // form cannot carry an alpha, and the alpha is how this is used everywhere.
    .join(" ");
}

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

function toHsl(r, g, b) {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d === 0) return [0, 0, l];

  const s = d / (1 - Math.abs(2 * l - 1));
  let h;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [(h * 60 + 360) % 360, s, l];
}

function toRgb(h, s, l) {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r, g, b] =
    h < 60 ? [c, x, 0]
    : h < 120 ? [x, c, 0]
    : h < 180 ? [0, c, x]
    : h < 240 ? [0, x, c]
    : h < 300 ? [x, 0, c]
    : [c, 0, x];
  return [r + m, g + m, b + m];
}

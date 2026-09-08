// Decoding artwork off the main thread.
//
// Measured in the desktop window on 2026-09-08, opening SNES: 49 covers, 2,184ms
// of `createImageBitmap` and 1,834ms of fetching, all of it on the thread that
// runs the view transition, the grid and every animation. The first second
// after opening a console ran at between one and four frames per hundred
// milliseconds. That is the app's single largest cost and it is not the
// animation's fault.
//
// The work itself is unchanged -- fetch the bytes, decode once, draw into a
// canvas the size of the tile, drop the full-size copy. See `fitpicture.js` for
// why that trade exists at all. What changes is *where*: a worker does the
// fetch, the decode and the downscale into an `OffscreenCanvas`, and transfers
// the finished `ImageBitmap` back. Drawing that into the page's canvas measured
// 0.00ms.
//
// Everything here is best-effort. A browser with no `OffscreenCanvas`, a worker
// that will not start, a fetch that fails -- all of them return `null` and the
// caller falls back to the main-thread path that was here before. Nothing may
// depend on this having worked.

/// The worker's whole program. Inline, because `ui/` is embedded in the desktop
/// binary and served by the service, and a separate file is one more thing that
/// has to be present in both.
const PROGRAM = `
self.onmessage = async (e) => {
  const { id, url, boxW, boxH, dpr } = e.data;
  let bitmap = null;
  try {
    const r = await fetch(url);
    if (!r.ok) return postMessage({ id, err: "http " + r.status });
    bitmap = await createImageBitmap(await r.blob());
    // Never enlarge: a 224x256 titlescreen in a 400px box stays 224x256 and
    // the box letterboxes it, the way object-fit: contain does.
    const scale = Math.min(1, (boxW * dpr) / bitmap.width, (boxH * dpr) / bitmap.height);
    const w = Math.max(1, Math.round(bitmap.width * scale));
    const h = Math.max(1, Math.round(bitmap.height * scale));
    const canvas = new OffscreenCanvas(w, h);
    const ctx = canvas.getContext("2d");
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(bitmap, 0, 0, w, h);
    const out = canvas.transferToImageBitmap();
    postMessage({ id, w, h, bitmap: out }, [out]);
  } catch (err) {
    postMessage({ id, err: String(err && err.message ? err.message : err) });
  } finally {
    // The full-size decode goes now rather than whenever the collector gets to
    // it. In a worker as on the main thread, this is the difference between a
    // spike and a leak.
    if (bitmap && bitmap.close) bitmap.close();
  }
};`;

/// Three. The work is mostly decode, which is CPU, and the machine this runs on
/// has other things to do -- including drawing the app. Six was the concurrency
/// the main-thread version used and it is not a target: the point is no longer
/// to keep a queue full, it is to stay out of the way.
const POOL = 3;

let pool = null;
let nextId = 1;
const waiting = new Map();

/// True when this browser can do any of it. Checked once.
function usable() {
  return (
    typeof Worker === "function" &&
    typeof OffscreenCanvas === "function" &&
    typeof createImageBitmap === "function" &&
    typeof URL?.createObjectURL === "function"
  );
}

function start() {
  if (pool) return pool;
  if (!usable()) return (pool = []);
  try {
    const url = URL.createObjectURL(new Blob([PROGRAM], { type: "text/javascript" }));
    pool = [];
    for (let i = 0; i < POOL; i += 1) {
      const w = new Worker(url);
      w.onmessage = (e) => {
        const settle = waiting.get(e.data.id);
        waiting.delete(e.data.id);
        settle?.(e.data.err ? null : e.data);
      };
      // A worker that dies takes its outstanding request with it. Answer it
      // rather than leaving a promise that never settles: the caller has a
      // perfectly good fallback and a card with no cover is worse than a
      // slightly slower one.
      w.onerror = () => {
        for (const [id, settle] of waiting) {
          waiting.delete(id);
          settle(null);
        }
      };
      pool.push(w);
    }
    // The blob can go as soon as the workers have been created from it.
    URL.revokeObjectURL(url);
  } catch {
    pool = [];
  }
  return pool;
}

/// Fetch, decode and downscale `url` to fit a `boxW` x `boxH` box, off thread.
///
/// Resolves to `{ bitmap, w, h }`, or `null` for "could not, use the other
/// path". Never rejects.
export function decodeFitted(url, boxW, boxH, dpr = 1) {
  const workers = start();
  if (!workers.length) return Promise.resolve(null);
  const id = nextId++;
  // Absolute, because a blob worker's own base URL is not the page's, and the
  // web build hands out relative `/media?path=` URLs.
  let absolute;
  try {
    absolute = new URL(url, location.href).href;
  } catch {
    return Promise.resolve(null);
  }
  return new Promise((resolve) => {
    waiting.set(id, resolve);
    // Round-robin. Nothing here knows which worker is busy, and asking would
    // cost more than the imbalance does.
    workers[id % workers.length].postMessage({ id, url: absolute, boxW, boxH, dpr });
  });
}

/// Test seam, and the way a page that is going away lets go of three threads.
export function stopDecoders() {
  for (const w of pool ?? []) w.terminate();
  pool = null;
  for (const [id, settle] of waiting) {
    waiting.delete(id);
    settle(null);
  }
}

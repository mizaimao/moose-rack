// Playing a Windows 9x game in the page, in js-dos.
//
// The other in-page player, beside `player.js`. EmulatorJS has nothing that
// runs Windows 98; js-dos's DOSBox-X build does, and eXoWin9x already ships
// every game as a DOSBox-X setup (docs/win9x.md).
//
// A game is three files. Its `.w9x`, from `/rom` like any other game: eXo's
// config for it, rewritten by tools/win9x_pack.py, with the names of the other
// two in its first lines. Its zip, from eXoWin9x where it sits, which js-dos
// unpacks itself. And the Windows 98 disk it boots, 254 MB, kept in this
// browser after the first time, because every game boots the same one.
//
// Nothing a game writes is kept. Its saves are on its D: disk, which here is a
// copy in this tab's memory, gone with the tab.

import { ejsUrls } from "./ejs-base.js";
import { ASK_ABOVE, OWN_PLAYER } from "./ejs-systems.js";

/// The js-dos release under `/jsdos/`, as assets/jsdos/MANIFEST.tsv pins it.
export const JSDOS = "8.4.1";

/// The Cache Storage bucket the Windows 98 disk is kept in.
const CACHE = "moose-win9x";

/// What a `.w9x` says, read from its comment lines.
///
/// `# key = value` for the files to fetch and their sizes, and files for the
/// page to write as `# file = <path>` followed by its lines as `# | <line>`.
/// The text itself is the DOSBox-X config; the comments are why DOSBox-X
/// reads past all of this.
export function parseW9x(text) {
  const head = {};
  const files = {};
  let file = null;
  for (const line of String(text).split(/\r?\n/)) {
    let m;
    if ((m = line.match(/^# file = (.+)$/))) {
      file = m[1];
      files[file] = [];
    } else if (file && (m = line.match(/^# \|(?: (.*))?$/))) {
      files[file].push(m[1] ?? "");
    } else if ((m = line.match(/^# (\w+) = (.+)$/))) {
      head[m[1]] = m[2];
      file = null;
    }
  }
  return {
    zip: head.zip,
    base: head.base,
    bytes: Number(head.bytes) || 0,
    unpacked: Number(head.unpacked) || 0,
    // CRLF, as the sheets eXo ships are written.
    files: Object.fromEntries(Object.entries(files).map(([k, v]) => [k, v.join("\r\n") + "\r\n"])),
    conf: String(text),
  };
}

/// Where the service hands out a file in eXoWin9x (`win9x_file`).
export function win9xUrl(path) {
  return `/win9x?path=${encodeURIComponent(path)}`;
}

function mb(n) {
  return `${Math.round(Number(n) / 1e6)} MB`;
}

/// Bytes, saying how far along they are.
///
/// Into one buffer of the announced size when there is one: a game zip is
/// hundreds of megabytes, and collecting chunks first would hold it twice.
async function fetchBytes(url, onProgress) {
  const res = await fetch(url, { credentials: "same-origin" });
  if (!res.ok) {
    throw new Error(`${res.status} for ${url}: ${(await res.text().catch(() => "")).slice(0, 200)}`);
  }
  const total = Number(res.headers.get("content-length")) || 0;
  if (!res.body || !total) return new Uint8Array(await res.arrayBuffer());
  const out = new Uint8Array(total);
  const reader = res.body.getReader();
  let got = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    if (got + value.length > total) throw new Error(`${url} is longer than it said`);
    out.set(value, got);
    got += value.length;
    onProgress?.(got, total);
  }
  if (got !== total) throw new Error(`${url} stopped at ${mb(got)} of ${mb(total)}`);
  return out;
}

/// The Cache Storage bucket, or nothing where the browser has none: a page
/// that is not a secure context, or a private window that refuses.
async function bucket() {
  try {
    return await globalThis.caches?.open(CACHE);
  } catch {
    return null;
  }
}

/// The Windows 98 disk, from this browser if it has it.
///
/// Kept by name, and a new build has a new name (`BASE` in win9x_pack.py), so
/// a disk in here is never stale. Any other name in the bucket is an older
/// build no game boots any more, and goes when the new one comes in.
async function baseDisk(url, onProgress) {
  const cache = await bucket();
  const hit = await cache?.match(url);
  if (hit) return new Uint8Array(await hit.arrayBuffer());
  const bytes = await fetchBytes(url, onProgress);
  try {
    const keep = new URL(url, location.href).href;
    for (const req of await cache.keys()) if (req.url !== keep) await cache.delete(req);
    await cache.put(url, new Response(bytes, { headers: { "content-type": "application/octet-stream" } }));
  } catch {
    // Not kept, so fetched again next time. Still plays.
  }
  return bytes;
}

/// The disk's size if it is not in this browser yet, for the question below.
async function baseStillToFetch(url) {
  if (await (await bucket())?.match(url)) return 0;
  try {
    const r = await fetch(url, { method: "HEAD", credentials: "same-origin" });
    // A 404's length is the length of "not found", not of a disk. The fetch
    // that follows says what went wrong.
    return r.ok ? Number(r.headers.get("content-length")) || 0 : 0;
  } catch {
    return 0;
  }
}

function loadJsdos() {
  if (globalThis.Dos) return Promise.resolve();
  const base = `/jsdos/${JSDOS}/`;
  const css = document.createElement("link");
  css.rel = "stylesheet";
  css.href = `${base}js-dos.css`;
  document.head.appendChild(css);
  return new Promise((resolve, reject) => {
    const s = document.createElement("script");
    s.src = `${base}js-dos.js`;
    s.onload = () => resolve();
    s.onerror = () => reject(new Error("js-dos is not installed on the server — run scripts/fetch-jsdos.sh"));
    document.body.appendChild(s);
  });
}

/// The overlay, the same `<dialog>` the EmulatorJS player uses and for the
/// same reasons (see `openStage` in player.js), with only Stop in its bar:
/// there are no save states or shaders to offer here.
function openStage(title) {
  const stage = document.createElement("dialog");
  stage.id = "ejs-stage";
  stage.innerHTML = `
    <div class="ejs-bar">
      <button class="ejs-close" aria-label="Stop">Stop</button>
      <span class="ejs-title"></span>
    </div>
    <div class="ejs-frame"><div class="ejs-note"></div></div>`;
  stage.querySelector(".ejs-title").textContent = title;
  // A reload, as in player.js: js-dos has no supported teardown either.
  stage.querySelector(".ejs-close").addEventListener("click", () => location.reload());
  // Escape is a key Windows games use. Stop is the way out.
  stage.addEventListener("cancel", (ev) => ev.preventDefault());
  keepOpen(stage);
  document.body.appendChild(stage);
  try {
    stage.showModal?.();
  } catch {
    stage.setAttribute("open", "");
  }
  if (!stage.hasAttribute("open")) stage.setAttribute("open", "");
  return stage;
}

/// Put the stage back if the browser closes it anyway.
///
/// Chrome honours a prevented `cancel` once: a second Escape, or Android's
/// Back, with no other input in between closes the dialog regardless. The game
/// went on running behind the library with no Stop button, and the library's
/// keys stayed off because the stage was still in the page.
export function keepOpen(stage) {
  stage.addEventListener("close", () => {
    if (!stage.isConnected) return;
    try {
      stage.showModal();
    } catch {
      stage.setAttribute("open", "");
    }
  });
}

function note(stage, text, bad = false) {
  const n = stage?.querySelector(".ejs-note");
  if (!n) return;
  n.textContent = text;
  n.dataset.bad = bad ? "1" : "";
}

/// Ask inside the page, without blocking it (see `ask` in player.js).
function ask(message, choices) {
  return new Promise((resolve) => {
    const box = document.createElement("div");
    box.className = "ejs-ask";
    box.innerHTML = `<p></p><div class="ejs-ask-row"></div>`;
    box.querySelector("p").textContent = message;
    const row = box.querySelector(".ejs-ask-row");
    for (const [value, label] of choices) {
      const b = document.createElement("button");
      b.textContent = label;
      b.addEventListener("click", () => {
        box.remove();
        resolve(value);
      });
      row.appendChild(b);
    }
    document.body.appendChild(box);
  });
}

/// The question before a big download, or `null` when there is nothing to ask.
export function heavyQuestion(name, zipBytes, baseBytes, unpacked, keeps = true) {
  if (zipBytes + baseBytes < ASK_ABOVE) return null;
  const parts = [`${name} is ${mb(zipBytes)}`];
  // Kept only where the browser has Cache Storage, which needs HTTPS.
  if (baseBytes) {
    parts.push(
      keeps
        ? `plus the ${mb(baseBytes)} Windows 98 disk this browser keeps after the first game`
        : `plus the ${mb(baseBytes)} Windows 98 disk, every time on a page that is not HTTPS`
    );
  }
  let q = `${parts.join(", ")}. Playing it here downloads that into this tab before Windows starts.`;
  // Measured nowhere yet: where a tab gives up depends on the browser and the
  // machine, so this says what is known and lets the person decide.
  if (unpacked > 1.5e9) q += ` It unpacks to ${mb(unpacked)}, which may be more than a tab can hold.`;
  return q;
}

let starting = false;

/// Play one Windows 9x game in the page. `rom` is a `rom_detail`.
export async function playWin9x(rom) {
  if (starting || document.getElementById("ejs-stage")) return "Already playing";
  starting = true;
  let stage = null;
  try {
    // Here rather than at the top, so the parsing above can be tested without
    // a page: state.js reads the document as it loads.
    const { invoke } = await import("./state.js");
    const urls = await ejsUrls(invoke);
    if (urls.desktop) return OWN_PLAYER.windows9x;

    const res = await fetch(urls.rom(rom.id), { credentials: "same-origin" });
    if (!res.ok) return `Could not read the game's .w9x: ${res.status}`;
    const game = parseW9x(await res.text());
    if (!game.zip || !game.base) return "This .w9x names no game or no Windows disk — pack it again with tools/win9x_pack.py";
    const zipUrl = win9xUrl(game.zip);
    const baseUrl = win9xUrl(game.base);

    const q = heavyQuestion(rom.name, game.bytes, await baseStillToFetch(baseUrl), game.unpacked, Boolean(await bucket()));
    if (q && (await ask(q, [["yes", "Play it"], ["", "Cancel"]])) !== "yes") return "Cancelled";

    stage = openStage(rom.name);
    const progress = (what) => (got, total) => note(stage, `${what}: ${mb(got)} of ${mb(total)}`);
    note(stage, "Windows 98…");
    const base = await baseDisk(baseUrl, progress("Windows 98 disk"));
    note(stage, "The game…");
    const zip = await fetchBytes(zipUrl, progress(rom.name));
    note(stage, "Starting js-dos…");
    await loadJsdos();

    // Made now, not with the stage: js-dos paints its own background at once,
    // and the note behind it is the only progress there is until then.
    const el = document.createElement("div");
    el.id = "ejs-player";
    stage.querySelector(".ejs-frame").appendChild(el);
    const text = new TextEncoder();
    const dos = globalThis.Dos(el, {
      dosboxConf: game.conf,
      // The disk as `c.vhd`, which is what the config mounts as C:. The zip as
      // a bundle, which js-dos unpacks into its root, so the paths the config
      // names are the zip's own. Then any corrected CD sheets, beside theirs.
      initFs: [
        { path: "c.vhd", contents: base },
        zip,
        ...Object.entries(game.files).map(([path, t]) => ({ path, contents: text.encode(t) })),
      ],
      backend: "dosboxX",
      backendLocked: true,
      pathPrefix: `/jsdos/${JSDOS}/emulators/`,
      autoStart: true,
      mouseCapture: true,
      thinSidebar: true,
      // Read in js-dos 8.4.1: with `local` false it loads no changes it saved
      // in this browser, so every start is the disk as the server has it.
      fsChanges: { local: false },
    });
    // No js-dos cloud for saves. In 8.4.1 this is a setter only, not an
    // option. Its network relay connects only when asked from its own menu.
    dos?.setNoCloud?.(true);
    return `Playing ${rom.name}`;
  } catch (e) {
    const msg = `Could not start: ${e?.message ?? e}`;
    if (stage) note(stage, msg, true);
    return msg;
  } finally {
    starting = false;
  }
}

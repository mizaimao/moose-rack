// "Take this with me" — download whole systems or a collection.
//
// The dialog exists because the choice is not obvious and the cost is not
// visible. Artwork is tens of kilobytes a game; a video is tens of megabytes.
// Ticking "videos" on a 2,400-game collection is the difference between a
// download that finishes before a flight and one that does not, and nothing on
// screen says so until the estimate moves.
//
// Systems are a checkbox list rather than a menu because nobody travels with
// one console. Choosing one at a time meant running this whole dialog once per
// system, each with a size check against a disk the previous run had already
// eaten into — so the last one always claimed to fit and did not.
//
// BIOS files sit here too, though they belong to no system in particular.
// Somewhere with no server is precisely where a missing BIOS turns into a
// console that will not boot and cannot be fixed, and this is the pane people
// open before going there.

import { invoke, listen } from "./state.js";
import { toast, escapeHtml } from "./util.js";

let open = null;

/// Ask what to take, then take it.
///
/// `what` is `{platform}`, `{collection, name}`, or nothing at all — opened
/// from the consoles page, where nothing has been chosen yet.
export async function askDownload(what = {}) {
  // `isConnected`, not just a non-null reference: if the overlay is removed by
  // anything other than its own close button, a bare null check would leave
  // this permanently convinced a dialog is up and refuse to open another.
  if (open?.isConnected) return;

  // A collection is a thing you pointed at, and asking again which systems it
  // covers would be a question about your own click. Otherwise: the list, with
  // whatever was on screen already ticked.
  let picker = "";
  if (!what.collection) {
    let list = [];
    let mine = [];
    try {
      // Both lists up front: the tabs switch between two things that are
      // already here, rather than fetching on the first click and leaving an
      // empty pane while it does.
      [list, mine] = await Promise.all([
        invoke("platforms"),
        invoke("collections_in", { group: "user" }).catch(() => []),
      ]);
    } catch {
      return toast("Could not read the platform list");
    }
    if (!list.length) return toast("Nothing to download yet — sync first");
    const tick = (cls, value, label, count, checked) => `
      <label class="bulk-sys">
        <input type="checkbox" class="${cls}" value="${escapeHtml(String(value))}"
          ${checked ? "checked" : ""} />
        <span>${escapeHtml(label)}</span>
        <em>${count}</em>
      </label>`;
    picker = `
      <div class="bulk-systems">
        <div class="bulk-systems-head">
          <span class="bulk-tabs">
            <button type="button" class="bulk-tab on" data-tab="systems">Systems</button>
            ${mine.length ? `<button type="button" class="bulk-tab" data-tab="mine">My collections</button>` : ""}
          </span>
          <span class="bulk-pick">
            <button type="button" class="link bulk-all">All</button>
            <button type="button" class="link bulk-none">None</button>
          </span>
        </div>
        <div class="bulk-list" data-tab="systems">
          ${list
            .map((p) => tick("bulk-plat", p.slug, p.name, p.rom_count, p.slug === what.platform))
            .join("")}
        </div>
        ${
          mine.length
            ? `<div class="bulk-list" data-tab="mine" hidden>
                 ${mine
                   .map((c) =>
                     tick("bulk-coll", c.id, c.name, `${c.rom_count - (c.local_count ?? 0)} to get`, false)
                   )
                   .join("")}
               </div>`
            : ""
        }
      </div>`;
  }
  const title = what.name ?? "your library";

  const box = document.createElement("div");
  box.id = "bulk-overlay";
  box.innerHTML = `
    <div class="bulk-panel" role="dialog" aria-label="Download ${escapeHtml(title)}">
      <h3>Take ${escapeHtml(title)} with you</h3>
      ${picker}
      <p class="bulk-est">Choose what to take, then check the size.</p>

      <label class="bulk-row">
        <span>Artwork</span>
        <select class="bulk-art">
          <option value="minimal" selected>Just what's shown (recommended)</option>
          <option value="full">Every picture</option>
          <option value="none">None</option>
        </select>
      </label>
      <p class="hint">Minimal is the two images the app draws: the game list's
        picture and the info pane's. Every picture adds box art, cartridge, 3D
        box, screenshots, title screens and marquees.</p>

      <label class="bulk-row bulk-check">
        <input type="checkbox" class="bulk-videos" />
        <span>Gameplay videos <em>— roughly 8 MB each</em></span>
      </label>
      <label class="bulk-row bulk-check">
        <input type="checkbox" class="bulk-manuals" />
        <span>Manuals</span>
      </label>
      <label class="bulk-row bulk-check">
        <input type="checkbox" class="bulk-bios" />
        <span>BIOS files <em>— PlayStation, Saturn, Dreamcast and friends will
          not boot without them</em></span>
      </label>

      <p class="bulk-space"></p>
      <div class="bulk-buttons">
        <button class="ghost bulk-cancel">Cancel</button>
        <button class="ghost bulk-size">Check size</button>
        <button class="primary bulk-go">Download</button>
      </div>
    </div>`;
  document.body.appendChild(box);
  open = box;

  const q = (s) => box.querySelector(s);
  const all = (s) => [...box.querySelectorAll(s)];
  const choice = () => ({
    platforms: all(".bulk-plat:checked").map((c) => c.value),
    collections: all(".bulk-coll:checked").map((c) => c.value),
    collection: what.collection ?? null,
    art: q(".bulk-art").value,
    videos: q(".bulk-videos").checked,
    manuals: q(".bulk-manuals").checked,
    bios: q(".bulk-bios").checked,
  });
  const close = () => {
    box.remove();
    open = null;
  };

  const stale = () => {
    q(".bulk-est").textContent = "Choose what to take, then check the size.";
    q(".bulk-space").textContent = "";
    q(".bulk-space").classList.remove("bad");
    q(".bulk-go").disabled = false;
  };

  async function refresh() {
    const est = q(".bulk-est");
    const btn = q(".bulk-size");
    btn.disabled = true;
    est.textContent = "Counting…";
    try {
      const [summary, fits, note] = await invoke("download_estimate", { choice: choice() });
      est.textContent = summary;
      q(".bulk-space").textContent = note;
      q(".bulk-space").classList.toggle("bad", !fits);
      // Refusing here rather than after an hour of transfer is the entire
      // point of asking the disk first.
      q(".bulk-go").disabled = !fits;
    } catch (e) {
      est.textContent = String(e);
      q(".bulk-go").disabled = true;
    } finally {
      btn.disabled = false;
    }
  }

  // Changing a box invalidates the last figure rather than recomputing it.
  // Counting what is already on disk means one filesystem call per game, and
  // doing that on every tick of a checkbox stalled the window for seconds.
  box.addEventListener("change", stale);
  // The tabs, and which list All/None act on: the one you are looking at.
  // Ticking every collection because you pressed All on the systems tab is a
  // download nobody asked for.
  const shownTab = () => box.querySelector(".bulk-tab.on")?.dataset.tab ?? "systems";
  const boxesHere = () => all(`.bulk-list[data-tab="${shownTab()}"] input`);
  for (const tab of all(".bulk-tab")) {
    tab.addEventListener("click", () => {
      all(".bulk-tab").forEach((t) => t.classList.toggle("on", t === tab));
      for (const list of all(".bulk-list")) list.hidden = list.dataset.tab !== tab.dataset.tab;
    });
  }

  q(".bulk-all")?.addEventListener("click", () => {
    boxesHere().forEach((c) => (c.checked = true));
    stale();
  });
  q(".bulk-none")?.addEventListener("click", () => {
    boxesHere().forEach((c) => (c.checked = false));
    stale();
  });
  q(".bulk-size").addEventListener("click", refresh);
  q(".bulk-cancel").addEventListener("click", close);
  box.addEventListener("keydown", (e) => e.key === "Escape" && close());

  q(".bulk-go").addEventListener("click", async () => {
    const go = q(".bulk-go");
    const picked = choice();
    // The one case the backend cannot report usefully: with nothing ticked it
    // has no idea whether you meant everything or nothing.
    if (!picked.platforms.length && !picked.collection) {
      return toast("Tick at least one system");
    }
    go.disabled = true;
    const est = q(".bulk-est");
    const stop = await listen("bulk-progress", ({ payload }) => {
      est.textContent = String(payload);
    });
    try {
      toast(await invoke("download_set", { choice: picked }), 8000);
      close();
    } catch (e) {
      est.textContent = String(e);
      go.disabled = false;
    } finally {
      stop?.();
    }
  });
}

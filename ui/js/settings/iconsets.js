// The Icon sets tab: look at an ES-DE theme's console art before fetching it.
//
// Its own tab because looking is the point. The Appearance tab's style picker
// offers five names and you find out what each looks like by choosing it and
// watching the grid change; here you see the pictures first.
//
// What is shown is each theme's *console artwork*, for consoles in this
// library, loaded a file at a time straight from the theme's repository. The
// tab first showed the authors' own screenshots and that was misleading: a
// screenshot is a picture of the theme's whole interface — menus, game grids,
// backgrounds — and the interface is the one part of a theme this app never
// installs. A preview has to show what the download actually gives you.
import { invoke, listen } from "../state.js";
import { toast, escapeHtml } from "../util.js";

export const html = `      <h4>Icon sets</h4>
      <p class="hint">Console pictures from ES-DE themes, shown with your own
        systems. Nothing is downloaded until you ask.</p>
      <div class="srow">
        <label>Drawing from</label>
        <div class="ctl">
          <select class="ic-active"><option value="">Shared pictures</option></select>
        </div>
      </div>
      <p class="hint">Shared pictures are the pool the Appearance tab fills from
        four themes at once, taking the best of each kind. A set keeps one
        designer's work together; anything it has no picture for falls back to
        the pool.</p>
      <div class="srow">
        <label>Find</label>
        <div class="ctl">
          <input class="ic-find" type="text" spellcheck="false" placeholder="theme name" />
          <label class="ic-nowords"><input type="checkbox" /> Hide names-only</label>
        </div>
      </div>
      <p class="hint ic-count"></p>
      <div class="ic-grid"></div>`;

/// One set's card, showing that theme's pictures of this library's consoles.
function card(s) {
  if (s.missing) {
    return `<div class="ic-card ic-gone">
      <div class="ic-name">${escapeHtml(s.name)}</div>
      <p class="hint">No longer in the ES-DE themes list.</p>
    </div>`;
  }
  // A picture that 404s hides itself rather than leaving a broken-image box:
  // the paths are recorded per theme and a theme is free to reorganize.
  const shots = s.icons.length
    ? `<div class="ic-shots">${s.icons
        .map(
          (u) =>
            `<img loading="lazy" src="${escapeHtml(u)}" alt=""
               onerror="this.style.display='none'" />`,
        )
        .join("")}</div>`
    : `<p class="hint">No console pictures recorded for this set.</p>`;

  const by = s.author ? ` — ${escapeHtml(s.author)}` : "";
  // The looks this set offers, in its own words. A theme decides how many it
  // has — one, or nine — and the Select button cycles exactly those.
  const kinds = (s.kinds ?? []).join(", ").toLowerCase();
  const state = s.installed
    ? `${s.installed} picture${s.installed === 1 ? "" : "s"} downloaded`
    : "not downloaded";

  // Three of the nine ship only wordmarks — a picture of each system's *name*
  // rather than the system. That is exactly what the console grid is meant to
  // avoid, so it is said before you spend a download finding out.
  const warn = s.wordmarks_only
    ? `<p class="hint ic-warn">Names only — this set draws each system's
         wordmark, not the hardware.</p>`
    : "";

  const actions = s.installed
    ? `<button class="ic-apply" data-dir="${escapeHtml(s.dir)}"${s.active ? " disabled" : ""}>
         ${s.active ? "In use" : "Use these"}</button>
       <button class="ic-remove" data-dir="${escapeHtml(s.dir)}">Remove</button>`
    : `<button class="ic-get" data-dir="${escapeHtml(s.dir)}">Download</button>`;

  return `<div class="ic-card${s.active ? " ic-on" : ""}">
    <div class="ic-name">${escapeHtml(s.name)}<span class="dim">${by}</span></div>
    ${shots}
    ${warn}
    <div class="ic-meta dim">${[kinds, state].filter(Boolean).join(" · ")}</div>
    <div class="ic-actions">${actions}</div>
  </div>`;
}

export function wire(box) {
  const grid = box.querySelector(".ic-grid");
  const picker = box.querySelector(".ic-active");
  const find = box.querySelector(".ic-find");
  const noWords = box.querySelector(".ic-nowords input");
  const count = box.querySelector(".ic-count");
  let sets = [];

  /// Redraw the cards from what is already loaded. Separate from `draw` so
  /// typing in the filter does not re-ask the backend on every keystroke.
  function paint() {
    const q = (find?.value ?? "").trim().toLowerCase();
    const shown = sets.filter(
      (s) =>
        (!q || s.name.toLowerCase().includes(q) || s.dir.includes(q)) &&
        !(noWords?.checked && s.wordmarks_only),
    );
    grid.innerHTML = shown.length
      ? shown.map(card).join("")
      : `<p class="hint">Nothing matches “${escapeHtml(q)}”.</p>`;
    if (count) {
      count.textContent =
        shown.length === sets.length
          ? `${sets.length} sets`
          : `${shown.length} of ${sets.length} sets`;
    }
  }

  find?.addEventListener("input", paint);
  noWords?.addEventListener("change", paint);

  async function draw() {
    grid.innerHTML = `<p class="hint">Reading the ES-DE themes list…</p>`;
    try {
      sets = (await invoke("icon_sets")) ?? [];
    } catch (e) {
      // Offline is the ordinary case here, not an error worth a toast: the tab
      // is the only thing that needs the network and it can say so in place.
      grid.innerHTML = `<p class="hint">Could not reach the ES-DE themes list — ${escapeHtml(
        String(e),
      )}</p>`;
      return;
    }
    paint();

    const active = sets.find((s) => s.active)?.dir ?? "";
    picker.innerHTML =
      `<option value="">Shared pictures</option>` +
      sets
        .filter((s) => s.installed)
        .map(
          (s) =>
            `<option value="${escapeHtml(s.dir)}"${
              s.dir === active ? " selected" : ""
            }>${escapeHtml(s.name)}</option>`,
        )
        .join("");
  }

  picker?.addEventListener("change", async () => {
    toast(await invoke("set_icon_set", { dir: picker.value }));
    await draw();
  });

  // One delegated listener rather than three per card, so a redraw cannot
  // leave a listener pointing at a button that no longer exists.
  grid?.addEventListener("click", async (e) => {
    const btn = e.target.closest("button");
    if (!btn) return;
    const dir = btn.dataset.dir;
    if (!dir) return;

    if (btn.classList.contains("ic-get")) {
      btn.disabled = true;
      const was = btn.textContent;
      btn.textContent = "Fetching…";
      const stop = await listen("icons-progress", ({ payload }) => {
        btn.textContent = String(payload);
      });
      try {
        toast(await invoke("install_icon_set", { dir }));
        await draw();
      } catch (err) {
        toast(String(err));
        btn.disabled = false;
        btn.textContent = was;
      } finally {
        stop?.();
      }
      return;
    }

    if (btn.classList.contains("ic-apply")) {
      toast(await invoke("set_icon_set", { dir }));
      await draw();
      return;
    }

    if (btn.classList.contains("ic-remove")) {
      toast(await invoke("remove_icon_set", { dir }));
      await draw();
    }
  });

  draw();
}

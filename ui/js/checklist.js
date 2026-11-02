// The launch checklist (docs/launch-flow.html): before a game starts, one
// window says whether the game file and the emulator are on this computer,
// each with its size, and offers Play, or Download with the total of what's
// missing. Saves are checked after Play, as their own step.

import { human, escapeHtml } from "./util.js";

/// What the button says for this checklist.
export function buttonFor(list) {
  if (list.blocked) return null;
  if (list.ready) return { go: "play", text: "Play" };
  if (list.download_bytes > 0) return { go: "download", text: `Download ${human(list.download_bytes)}` };
  // Nothing to fetch, only settings to apply on this computer.
  return { go: "download", text: "Set up" };
}

function line(title, item) {
  const size = item.bytes == null ? "" : human(item.bytes);
  return `<li class="${item.ready ? "ok" : "todo"}">
      <span class="mark"></span>
      <span><span class="what">${escapeHtml(title)}</span><br>
        <span class="detail">${escapeHtml(item.label)}: ${escapeHtml(item.detail)}</span></span>
      <span class="size">${escapeHtml(size)}</span>
    </li>`;
}

/// Show the checklist and wait for "play", "download" or "cancel".
export function askChecklist(list) {
  return new Promise((resolve) => {
    const main = buttonFor(list);
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    overlay.innerHTML = `<div class="conflict-box checklist-box">
        <header><h2>${escapeHtml(list.name)}</h2></header>
        <p class="lead">${escapeHtml(list.platform)}</p>
        <ol class="cl-lines">
          ${line("Game file", list.game)}
          ${line("Emulator", list.emulator)}
        </ol>
        ${list.blocked ? `<p class="cl-blocked">${escapeHtml(list.blocked)}</p>` : ""}
        <p class="note">Saves are checked with the server after you press Play.</p>
        <div class="cl-buttons">
          ${main ? `<button class="cl-main" data-go="${main.go}">${escapeHtml(main.text)}</button>` : ""}
          <button class="cl-cancel" data-go="cancel">Cancel</button>
        </div>
      </div>`;

    let settled = false;
    const finish = (go) => {
      if (settled) return;
      settled = true;
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(go);
    };
    const buttons = () => [...overlay.querySelectorAll("[data-go]")];
    let at = 0;
    const paint = () => buttons().forEach((b, i) => b.classList.toggle("sel", i === at));
    const onKey = (ev) => {
      // Every key stops here, as in the other launch questions: the library
      // behind listens on the window, and Enter would press Play on it.
      ev.stopPropagation();
      if (ev.key === "Escape") {
        ev.preventDefault();
        return finish("cancel");
      }
      if (ev.key === "ArrowLeft" || ev.key === "ArrowRight") {
        at = (at + 1) % buttons().length;
        paint();
        ev.preventDefault();
      }
      if (ev.key === "Enter") finish(buttons()[at]?.dataset.go ?? "cancel");
    };
    overlay.addEventListener("click", (ev) => {
      const b = ev.target.closest("[data-go]");
      if (b) finish(b.dataset.go);
    });
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    paint();
  });
}

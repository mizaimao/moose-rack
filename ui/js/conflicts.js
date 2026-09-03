// The save-conflict dialog: which copy do you want to keep?
//
// A conflict means the same save changed here and on the server since they last
// agreed. Nothing has been written, and the launch is refused until it is
// answered — playing on top of a save whose ownership is unresolved is how the
// loser gets overwritten for good on the way back out.
//
// Keyboard and pad reach this the same way they reach the rest of the app, so a
// controller-only user is never stuck at a dialog they cannot answer.

import { invoke } from "./state.js";
import { human, toast } from "./util.js";

/// A conflict the backend refused to guess about.
///
/// Shown side by side rather than as a sentence: "yours is newer" is the whole
/// decision, and a date next to a date makes that obvious in a way prose does
/// not.
function card(c) {
  const when = (t) => (t ? String(t).replace("T", " ").replace("Z", " UTC") : "unknown");
  const el = document.createElement("div");
  el.className = "conflict";
  el.innerHTML = `
    <h3>${escape(c.file_name)}</h3>
    <p class="why">${escape(c.reason || "This save changed here and on the server.")}</p>
    <div class="sides">
      <button class="side" data-keep="local">
        <span class="who">This machine</span>
        <span class="when">${escape(when(c.local_updated))}</span>
        <span class="size">${c.local_bytes ? human(c.local_bytes) : ""}</span>
      </button>
      <button class="side" data-keep="server">
        <span class="who">Server</span>
        <span class="when">${escape(when(c.server_updated))}</span>
        <span class="size"></span>
      </button>
    </div>
    <p class="note">The copy you do not keep is saved to
      <code>library/saves-backup/</code> first, so this is reversible.</p>
  `;
  return el;
}

function escape(s) {
  const d = document.createElement("div");
  d.textContent = s == null ? "" : String(s);
  return d.innerHTML;
}

/// Ask about each conflict in turn. Resolves true when every one was answered,
/// false if the user backed out — in which case the caller must not launch.
/// `lead` and `cancelLabel` are the two strings that are about *why* you are
/// being asked, and the answer is no longer always "a launch is waiting" — a
/// manual sync from the header reaches the same picker with nothing on hold.
/// Parameters rather than a second copy of the dialog: the choosing, the
/// keyboard grammar and the resolve call are the parts worth having once.
export function askConflicts(conflicts, {
  lead = "This save changed in two places. Choose which to keep — the launch is on hold until you do.",
  cancelLabel = "Cancel launch",
} = {}) {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    const box = document.createElement("div");
    box.className = "conflict-box";
    box.innerHTML = `<header>
        <span class="icon icon-info-on"></span>
        <h2>Save conflict</h2>
      </header>
      <p class="lead"></p>`;
    box.querySelector(".lead").textContent = lead;

    const list = document.createElement("div");
    box.appendChild(list);

    const cancel = document.createElement("button");
    cancel.className = "conflict-cancel";
    cancel.textContent = cancelLabel;
    box.appendChild(cancel);
    overlay.appendChild(box);

    let index = 0;
    let settled = false;

    const finish = (ok) => {
      if (settled) return;
      settled = true;
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(ok);
    };

    const show = () => {
      list.replaceChildren();
      if (index >= conflicts.length) return finish(true);
      const c = conflicts[index];
      const el = card(c);
      el.querySelectorAll("[data-keep]").forEach((btn) => {
        btn.addEventListener("click", async () => {
          btn.disabled = true;
          try {
            toast(
              await invoke("resolve_save_conflict", {
                fileName: c.file_name,
                keep: btn.dataset.keep,
              })
            );
            index += 1;
            show();
          } catch (e) {
            btn.disabled = false;
            // Staying open matters: the save is still unresolved, and closing
            // would leave the launch blocked with nothing on screen saying why.
            toast(`Could not resolve — ${e}`, 8000);
          }
        });
      });
      list.appendChild(el);
      el.querySelector("[data-keep]").focus();
    };

    // Left/right move between the two choices, Enter takes one, Esc cancels —
    // the same grammar as the rest of the app, so a pad works here too.
    const onKey = (e) => {
      const sides = [...list.querySelectorAll("[data-keep]")];
      if (!sides.length) return;
      const at = sides.indexOf(document.activeElement);
      if (e.key === "Escape") {
        e.preventDefault();
        finish(false);
      } else if (e.key === "ArrowRight" || e.key === "ArrowDown") {
        e.preventDefault();
        sides[Math.min(at + 1, sides.length - 1)].focus();
      } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
        e.preventDefault();
        sides[Math.max(at - 1, 0)].focus();
      } else if (e.key === "Enter" && at >= 0) {
        e.preventDefault();
        sides[at].click();
      }
      // Swallow everything else so the grid behind does not also act on it.
      e.stopPropagation();
    };

    cancel.addEventListener("click", () => finish(false));
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    show();
  });
}

/// Pull the conflict list out of the error a refused launch throws.
///
/// The backend cannot open a dialog and wait, so it refuses with the conflicts
/// attached and the frontend asks. A plain marker prefix rather than a typed
/// error because a Tauri command error is a string either way.
export function conflictsFrom(err) {
  const text = typeof err === "string" ? err : String(err?.message ?? err ?? "");
  const at = text.indexOf("SAVE_CONFLICT:");
  if (at === -1) return null;
  try {
    const parsed = JSON.parse(text.slice(at + "SAVE_CONFLICT:".length));
    return Array.isArray(parsed) && parsed.length ? parsed : null;
  } catch {
    return null;
  }
}

/// Pull the reason out of a launch refused because saves could not sync.
export function offlineFrom(err) {
  const text = typeof err === "string" ? err : String(err?.message ?? err ?? "");
  const at = text.indexOf("SAVE_OFFLINE:");
  return at === -1 ? null : text.slice(at + "SAVE_OFFLINE:".length).trim() || "unknown reason";
}

/// "Saves can't sync — play anyway?"
///
/// Asked rather than decided either way. Starting silently risks an hour on top
/// of a stale save; refusing would mean a server being off stops you playing at
/// all. Steam asks, and it is the right call.
export function askOffline(reason) {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    overlay.innerHTML = `<div class="conflict-box">
        <header><span class="icon icon-info-on"></span><h2>Saves are not syncing</h2></header>
        <p class="lead">Your saves could not be checked against the server.</p>
        <p class="why">${escape(reason)}</p>
        <p class="note">You can play, but progress will not be uploaded when you
          quit, and this device may not have the newest save.</p>
        <div class="sides">
          <button class="side" data-go="yes"><span class="who">Play anyway</span>
            <span class="when">saves stay on this machine</span></button>
          <button class="side" data-go="no"><span class="who">Cancel</span>
            <span class="when">do not launch</span></button>
        </div>
      </div>`;

    let settled = false;
    const finish = (ok) => {
      if (settled) return;
      settled = true;
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(ok);
    };

    const sides = () => [...overlay.querySelectorAll("[data-go]")];
    const onKey = (e) => {
      const list = sides();
      const at = list.indexOf(document.activeElement);
      if (e.key === "Escape") finish(false);
      else if (e.key === "ArrowRight" || e.key === "ArrowDown")
        list[Math.min(at + 1, list.length - 1)].focus();
      else if (e.key === "ArrowLeft" || e.key === "ArrowUp")
        list[Math.max(at - 1, 0)].focus();
      else if (e.key === "Enter" && at >= 0) list[at].click();
      else return;
      e.preventDefault();
      e.stopPropagation();
    };

    overlay.addEventListener("click", (e) => {
      const go = e.target.closest("[data-go]");
      if (go) finish(go.dataset.go === "yes");
    });
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    // Cancel is focused first: the safe answer should be the one a stray Enter
    // lands on.
    sides()[1].focus();
  });
}


/// Pull the file list out of a launch refused for a missing BIOS.
export function biosFrom(err) {
  const text = typeof err === "string" ? err : String(err?.message ?? err ?? "");
  const at = text.indexOf("BIOS_MISSING:");
  return at === -1 ? null : text.slice(at + "BIOS_MISSING:".length).trim() || "unknown file";
}

/// "This console needs a BIOS you have not got — play anyway?"
///
/// The app fetches BIOS files by itself on the way into every game, so this is
/// only ever the case it cannot fix: a file the server has not got either.
/// Asked rather than refused outright, because a core that declares a BIOS
/// does not always need one — several run fine without, and some run with
/// reduced compatibility rather than not at all. Refusing would stop a game
/// that would have worked.
/// Offer to bring config.toml up to date, once per set of findings.
///
/// Shown at startup because that is when the file was read, and only when
/// something in it can be updated without a decision. Everything else the check
/// found is listed too, so the dialog is a summary of the file rather than a
/// prompt with hidden context.
///
/// Dismissal is remembered against *what was found*, not a flag: fixing one
/// thing and finding a different one later should ask again, and saying no to
/// the same list twice should not.
const CONFIG_SEEN = "configPatchDeclined";

export function askConfigPatch(findings) {
  const fixable = findings.filter((f) => f.fixable);
  if (!fixable.length) return Promise.resolve(false);
  const signature = findings.map((f) => f.what).sort().join("|");
  if (localStorage.getItem(CONFIG_SEEN) === signature) return Promise.resolve(false);

  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    const rows = findings
      .map(
        (f) => `<li class="cfg-${escape(f.severity)}">
            <b>${escape(f.what)}</b> — ${escape(f.note)}
            ${f.fixable ? "" : '<i class="dim">left alone: your call</i>'}
          </li>`,
      )
      .join("");
    overlay.innerHTML = `<div class="conflict-box">
        <header><span class="icon icon-info-on"></span><h2>Your config file is out of date</h2></header>
        <p class="lead">${fixable.length} thing${fixable.length === 1 ? "" : "s"} in
          <code>config.toml</code> can be brought up to date. Everything still works —
          old settings are read through compatibility paths — but the file says
          things that are no longer true.</p>
        <ul class="cfg-list">${rows}</ul>
        <p class="note">A copy of the file as it is now goes to
          <code>config.toml.before-patch</code> first. Comments are kept.</p>
        <div class="sides">
          <button class="side" data-go="yes"><span class="who">Update it</span>
            <span class="when">keeps a backup</span></button>
          <button class="side" data-go="no"><span class="who">Leave it</span>
            <span class="when">do not ask about these again</span></button>
        </div>
      </div>`;

    let settled = false;
    const finish = (ok) => {
      if (settled) return;
      settled = true;
      if (!ok) localStorage.setItem(CONFIG_SEEN, signature);
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(ok);
    };
    const onKey = (ev) => {
      if (ev.key === "Escape") {
        ev.preventDefault();
        finish(false);
      }
    };
    for (const b of overlay.querySelectorAll("[data-go]")) {
      b.addEventListener("click", () => finish(b.dataset.go === "yes"));
    }
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    overlay.querySelector('[data-go="yes"]')?.focus();
  });
}

/// Remembered dismissal for the light gun notice.
const GUN_SEEN = "lightgunNoticeSeen";

/// Say once that the mouse is the gun, and how to stop it being one.
///
/// Shown on the first launch of a game on a console that has a gun, because
/// that is the only moment the information is wanted and the only moment
/// anybody would read it. The switch is on by default now: a gun game whose
/// trigger does nothing looks like a broken emulator, and the setting that
/// fixed it was a tick in a table three tabs away that said nothing about what
/// it did.
///
/// The cost is the other direction — on the NES, SNES and Mega Drive the gun
/// occupies player two's port — so this names the console it is about and says
/// where to turn it off. Resolves immediately once dismissed for good.
export function noteLightGun(platform, gunName) {
  if (localStorage.getItem(GUN_SEEN) === "yes") return Promise.resolve();
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    overlay.innerHTML = `<div class="conflict-box">
        <header><span class="icon icon-info-on"></span><h2>The mouse is your light gun</h2></header>
        <p class="lead">${escape(gunName)} games on ${escape(platform)} are ready
          to play: <b>aim with the mouse</b>, left button fires, right button
          shoots off-screen to reload.</p>
        <p class="note">This is on by default so gun games work without setting
          anything up. The catch: the gun sits in the <b>second controller
          port</b> on this console, so while it is on, a second pad will not
          work for two-player games.</p>
        <p class="why">Turn it off per console in Settings → Emulators, in the
          Light gun column.</p>
        <label class="gun-again"><input type="checkbox" /> Do not show this again</label>
        <div class="sides">
          <button class="side" data-go="yes"><span class="who">Got it</span>
            <span class="when">start the game</span></button>
        </div>
      </div>`;

    const finish = () => {
      if (overlay.querySelector(".gun-again input")?.checked) {
        localStorage.setItem(GUN_SEEN, "yes");
      }
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve();
    };
    const onKey = (ev) => {
      // Anything that means "go" — this dialog has nothing to decline.
      if (["Escape", "Enter", " "].includes(ev.key)) {
        ev.preventDefault();
        ev.stopPropagation();
        finish();
      }
    };
    overlay.querySelector("[data-go]")?.addEventListener("click", finish);
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    overlay.querySelector("[data-go]")?.focus();
  });
}

export function askBios(detail) {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    overlay.innerHTML = `<div class="conflict-box">
        <header><span class="icon icon-info-on"></span><h2>A BIOS file is missing</h2></header>
        <p class="lead">This console needs a BIOS file that is not on this
          machine, and the server does not have it either.</p>
        <p class="why">${escape(detail)}</p>
        <p class="note">Games usually show a black screen without it. Some cores
          run anyway with reduced compatibility, so this is worth a try before
          going looking for the file.</p>
        <div class="sides">
          <button class="side" data-go="yes"><span class="who">Play anyway</span>
            <span class="when">may show a black screen</span></button>
          <button class="side" data-go="no"><span class="who">Cancel</span>
            <span class="when">do not launch</span></button>
        </div>
      </div>`;

    let settled = false;
    const finish = (ok) => {
      if (settled) return;
      settled = true;
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(ok);
    };
    const sides = () => [...overlay.querySelectorAll("[data-go]")];
    let at = 0;
    const paint = () => sides().forEach((b, i) => b.classList.toggle("sel", i === at));
    const onKey = (ev) => {
      if (ev.key === "Escape") return finish(false);
      if (ev.key === "ArrowLeft" || ev.key === "ArrowRight") {
        at = at === 0 ? 1 : 0;
        paint();
        ev.preventDefault();
      }
      if (ev.key === "Enter") finish(sides()[at]?.dataset.go === "yes");
    };
    overlay.addEventListener("click", (ev) => {
      const b = ev.target.closest("[data-go]");
      if (b) finish(b.dataset.go === "yes");
    });
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    paint();
  });
}

// Save sync, shown before it happens.
//
// One button, not a push button and a pull button. The server decides the
// direction per save — some are newer here, some there, and a few are both —
// so "push" and "pull" are not choices anybody can make up front. Asking what
// *would* happen is, and it moves nothing.
//
// The same two-step the addon has had on the handheld since it was written, and
// for the same reason: a save is the only thing in this app that cannot be
// fetched again if it goes wrong.

import { invoke } from "./state.js";
import { toast } from "./util.js";
import { askConflicts } from "./conflicts.js";

/// Where a row goes, in words the row can be read by.
///
/// Deliberately not "upload"/"download": those name the transfer, and what
/// somebody looking at this list wants to know is which copy survives.
const WORDS = {
  conflict: { verb: "Changed in both places", cls: "sy-conflict" },
  download: { verb: "Comes from the server", cls: "sy-down" },
  upload: { verb: "Goes to the server", cls: "sy-up" },
};

function escape(s) {
  return String(s ?? "").replace(/[&<>"]/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]
  );
}

/// One line of the plan.
///
/// The reason comes from the server and is printed verbatim rather than
/// paraphrased — it knows things this machine does not, like when the two
/// copies last agreed.
function row(line) {
  const w = WORDS[line.action] ?? { verb: line.action, cls: "" };
  const el = document.createElement("div");
  el.className = `sy-row ${w.cls}`;
  el.innerHTML = `
    <span class="sy-what">${escape(w.verb)}</span>
    <span class="sy-title">${escape(line.title)}</span>
    ${line.reason ? `<span class="sy-why">${escape(line.reason)}</span>` : ""}
  `;
  return el;
}

/// Show the plan and wait for an answer. Resolves true if it should be carried
/// out.
///
/// Modal, like the conflict picker, and for the same reason: this is a question
/// with two answers and no useful third state where it sits in a corner while
/// you do something else that changes the saves underneath it.
function askPlan(review) {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.id = "conflict-overlay";
    const box = document.createElement("div");
    box.className = "conflict-box sync-box";
    const nothing = !review.lines.length;

    box.innerHTML = `<header>
        <span class="icon icon-sync"></span>
        <h2>${nothing ? "Nothing to sync" : "This is what would happen"}</h2>
      </header>
      <p class="lead">${escape(review.headline)}</p>`;

    const list = document.createElement("div");
    list.className = "sy-list";
    for (const line of review.lines) list.appendChild(row(line));
    box.appendChild(list);

    // Said plainly rather than left to be discovered. `negotiate` is saves
    // only; save states are compared against a local ledger and are not in
    // these rows, so a plan reading "nothing to sync" can still be followed by
    // four states moving. Better to say it than to look like a lie.
    const note = document.createElement("p");
    note.className = "conflict note";
    note.textContent =
      "Save states are checked separately and are not listed here. Nothing has moved yet.";
    box.appendChild(note);

    const buttons = document.createElement("div");
    buttons.className = "sy-buttons";
    const cancel = document.createElement("button");
    cancel.className = "conflict-cancel";
    cancel.textContent = nothing ? "Close" : "Not now";
    buttons.appendChild(cancel);

    // No "carry this out" when there is nothing to carry out. An empty plan
    // with a live Apply button is a button that does nothing, which reads as
    // broken rather than as finished.
    let go = null;
    if (!nothing) {
      go = document.createElement("button");
      go.className = "conflict-cancel sy-go";
      go.textContent = "Carry this out";
      buttons.appendChild(go);
    }
    box.appendChild(buttons);
    overlay.appendChild(box);

    let settled = false;
    const finish = (ok) => {
      if (settled) return;
      settled = true;
      overlay.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(ok);
    };

    // The same grammar as the conflict picker, so a pad reaches this too.
    const onKey = (e) => {
      const btns = [...buttons.querySelectorAll("button")];
      const at = btns.indexOf(document.activeElement);
      if (e.key === "Escape") {
        e.preventDefault();
        finish(false);
      } else if (e.key === "ArrowRight" || e.key === "ArrowDown") {
        e.preventDefault();
        btns[Math.min(at + 1, btns.length - 1)]?.focus();
      } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
        e.preventDefault();
        btns[Math.max(at - 1, 0)]?.focus();
      } else if (e.key === "Enter" && at >= 0) {
        e.preventDefault();
        btns[at].click();
      }
      e.stopPropagation();
    };

    cancel.addEventListener("click", () => finish(false));
    go?.addEventListener("click", () => finish(true));
    document.addEventListener("keydown", onKey, true);
    document.body.appendChild(overlay);
    // Opens on the safe answer. Enter on a dialog you have not read should not
    // move somebody's saves.
    cancel.focus();
  });
}

/// True while a sync is in flight anywhere in the app.
///
/// One flag rather than a disabled button, because there are two buttons now —
/// the header and the settings page — and a second sync would race the first
/// over the same files.
let running = false;

export function syncRunning() {
  return running;
}

/// The whole thing: ask, show, and carry out what was accepted.
///
/// `say` is how progress gets reported; the settings page passes a line under
/// its button, everywhere else it is a toast. Returns what happened, so a
/// caller can decide whether to redraw.
export async function syncSaves({ say = (m) => toast(m) } = {}) {
  if (running) {
    say("A sync is already running.");
    return null;
  }
  running = true;
  try {
    say("Asking the server what would change…");
    const review = await invoke("sync_saves_plan");
    if (!(await askPlan(review))) {
      say(review.lines.length ? "Nothing was transferred." : "");
      return null;
    }

    say("Syncing…");
    const run = await invoke("sync_saves");
    say(run.headline);

    // The picker the launch path already uses, in a room where there is no
    // launch to hold up. Reached at all for the first time: this used to come
    // back as a sentence in a status line with nothing to press.
    if (run.conflicts?.length) {
      await askConflicts(run.conflicts, {
        lead: `${run.conflicts.length} ${
          run.conflicts.length === 1 ? "save" : "saves"
        } changed in two places. Choose which copy to keep.`,
        cancelLabel: "Decide later",
      });
    }
    return run;
  } catch (e) {
    say(`Sync failed — ${e}`);
    return null;
  } finally {
    running = false;
  }
}

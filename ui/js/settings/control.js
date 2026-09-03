// The Control tab: the players, and every binding.
//
// The capture state lives here rather than in the dispatcher because this is
// the only tab that captures anything — and the settings window asks whether a
// capture is in progress before deciding what a keypress means.
import {
  actions, setKey, resetAll, keyLabelFor,
  padFor, setPad, resetPad, padLabel, padLabelFor,
} from "../bindings.js";
import { toast, escapeHtml } from "../util.js";
import { wireConfigFields } from "./fields.js";

/// Set while waiting for a keypress to assign, so the window's own key handler
/// gets out of the way.
let capturing = null;

/// Set while waiting for a controller button. The Gamepad API has no
/// button-down event, so binding one means polling until something is pressed.
let padCapture = null;

export const html = () => `      <h4>Players</h4>
      <p class="hint">Four ports, filled in the order pads connect. Only the
        first drives this app's menus.</p>
      <div class="pad-list">Looking for controllers…</div>
      <div class="srow">
        <label>Match player 1</label>
        <div class="ctl"><button data-field="mirror_player_one">…</button></div>
      </div>
      <p class="hint">Binds players 2–4 like player 1, since RetroArch gives an unknown pad
      none. Turn off only for genuinely different models.</p>

      <div class="srow">
        <label>Swap A and B</label>
        <div class="ctl"><button data-field="swap_ab">…</button></div>
      </div>
      <div class="srow">
        <label>Swap X and Y</label>
        <div class="ctl"><button data-field="swap_xy">…</button></div>
      </div>
      <p class="hint">For pads whose face buttons are lettered the other way
        round — a Nintendo layout read by software expecting an Xbox one, where
        Confirm and Back come out reversed. This changes how the app *reads* the
        pad and is not written into RetroArch's config, so a game still plays
        with the buttons the game expects. Turning it off puts the pad back
        exactly as it was.</p>

      <h4>Bindings</h4>
      <p class="hint">Click a key or a button to rebind it. Esc leaves it
        unset.</p>
      <p class="hint pad-live">No controller detected.</p>
      <table class="bindtbl">
        <thead><tr><th>Action</th><th>Keyboard</th><th>Controller</th></tr></thead>
        <tbody>
        ${actions()
          .map(
            (a) => `
          <tr data-id="${a.id}">
            <td class="bindname">${a.label}</td>
            <td class="key-cell"><button class="set-key ${keyLabelFor(a.id) === "—" ? "unset" : ""}">${keyLabelFor(a.id)}</button></td>
            <td class="pad-cell"><button class="set-pad ${padFor(a.id) === null ? "unset" : ""}">${padLabelFor(a.id)}</button></td>
          </tr>`
          )
          .join("")}
        </tbody>
      </table>
      <footer>
        <button class="set-reset">Reset keyboard</button>
        <button class="set-pad-reset">Reset controller</button>
      </footer>`;

export function wire(box) {
  box.querySelectorAll("tr[data-id]").forEach((row) => {
    const btn = row.querySelector(".key-cell .set-key");
    if (!btn) return;
    btn.addEventListener("click", () => {
      if (capturing) capturing.btn.textContent = keyLabelFor(capturing.id);
      capturing = { id: row.dataset.id, btn };
      btn.textContent = "press a key…";
      btn.classList.add("capturing");
    });
  });

  // "Reset keyboard" was wired by the General tab, which does not contain it.
  // That lookup returned null and threw, and everything after it in General —
  // including the wiring for every text field and toggle on that tab — never
  // ran. Splitting the panes into files is what made it visible: the button is
  // in this file and the handler was in another.
  box.querySelector(".set-reset")?.addEventListener("click", async () => {
    await resetAll();
    redraw(box);
    toast("Keyboard bindings reset");
  });

  box.querySelector(".set-pad-reset")?.addEventListener("click", async () => {
    await resetPad();
    redraw(box);
    toast("Controller bindings reset");
  });

  // Live readout of what the pad actually reports. The defaults assume the
  // W3C "standard" layout, but a pad that reports a different mapping puts the
  // face buttons at other indices — in which case the bindings look right and
  // nothing responds. This makes that visible instead of a guessing game.
  // Which pad is which player. The Gamepad API hands them back in connection
  // order and so does RetroArch, so the order here is the order in the game —
  // which is the only way to find out which controller is player three
  // without starting something four-player and pressing buttons.
  const list = box.querySelector(".pad-list");
  const drawList = () => {
    const pads = (navigator.getGamepads?.() ?? []).filter(Boolean);
    if (!pads.length) {
      list.innerHTML = `<p class="hint">Nothing connected — press a button on a
        controller to wake it.</p>`;
      return;
    }
    list.innerHTML = pads
      .slice(0, 4)
      .map(
        (p, i) => `
        <div class="pad-row">
          <span class="pad-num">P${i + 1}</span>
          <span class="pad-name">${escapeHtml(p.id.replace(/\s*\(.*\)\s*$/, "").trim() || "controller")}</span>
          ${i === 0 ? `<em>drives the menus</em>` : ""}
        </div>`
      )
      .join("") +
      (pads.length > 4
        ? `<p class="hint">${pads.length - 4} more connected than there are ports.</p>`
        : "");
  };
  drawList();
  window.addEventListener("gamepadconnected", drawList);
  window.addEventListener("gamepaddisconnected", drawList);

  const live = box.querySelector(".pad-live");
  // Twice the rate a button can be pressed and released is fast enough to look
  // instant, and 120 times a second is not twelve times better. This used to
  // rebuild the line on every animation frame, which meant re-measuring and
  // re-laying-out text at the display's refresh rate for a string that changes
  // when a thumb moves.
  let lastLine = "";
  const readPad = () => {
    // `box.isConnected`, not a lookup for an element id that no longer exists
    // anywhere in either document — that check was always false, so this
    // readout stopped after one frame and reported "no controller" for good.
    if (!box.isConnected) return;
    const pad = (navigator.getGamepads?.() ?? []).find(Boolean);
    const line = !pad
      ? "No controller detected — press a button to wake it."
      : `${pad.id} · mapping: ${pad.mapping || "(none reported)"} · ` +
        `${pad.buttons.length} buttons · pressed: ${
          pad.buttons
            .map((b, i) => (b.pressed ? i : null))
            .filter((i) => i !== null)
            .join(", ") || "none"
        }`;
    // Writing the same string still dirties the node and costs a relayout.
    if (line !== lastLine) {
      lastLine = line;
      live.textContent = line;
    }
    setTimeout(readPad, 60);
  };
  readPad();

  box.querySelectorAll("tr[data-id]").forEach((row) => {
    const btn = row.querySelector(".pad-cell .set-pad");
    if (!btn) return;
    btn.addEventListener("click", () => {
      stopPadCapture();
      btn.textContent = "press a button…";
      btn.classList.add("capturing");
      startPadCapture(row.dataset.id, btn);
    });
  });


  // The mirror toggle is a config.toml field like the ones in General.
  wireConfigFields(box);

}

export function stopPadCapture() {
  if (!padCapture) return;
  cancelAnimationFrame(padCapture.raf);
  padCapture.btn.classList.remove("capturing");
  padCapture.btn.textContent = padLabelFor(padCapture.id);
  padCapture = null;
}

function startPadCapture(id, btn) {
  // Ignore whatever is already held from the click that got us here, so the
  // action is not instantly bound to the button still under the user's thumb.
  const settled = new Set();
  const step = () => {
    const pads = navigator.getGamepads?.() ?? [];
    for (const pad of pads) {
      if (!pad) continue;
      for (let i = 0; i < pad.buttons.length; i++) {
        const down = pad.buttons[i]?.pressed;
        if (!down) {
          settled.add(i);
          continue;
        }
        if (!settled.has(i)) continue; // held since before we started
        padCapture = null;
        btn.classList.remove("capturing");
        setPad(id, i).then(redrawPadRows);
        return;
      }
    }
    if (padCapture) padCapture.raf = requestAnimationFrame(step);
  };
  padCapture = { id, btn, raf: requestAnimationFrame(step) };
}

/// Consume a keypress as a new binding. Returns true when handled.
export function captureKey(ev) {
  if (padCapture) {
    if (ev.key !== "Escape") return true;   // swallow keys while binding a pad
    ev.preventDefault();
    const { btn, id } = padCapture;
    padCapture = null;
    btn.classList.remove("capturing");
    setPad(id, null).then(redrawPadRows);
    return true;
  }
  if (!capturing) return false;
  ev.preventDefault();

  // Modifiers alone are not bindings.
  if (["Shift", "Control", "Alt", "Meta"].includes(ev.key)) return true;

  const key = ev.key === "Escape" ? null : ev.key;
  const { btn, id } = capturing;
  capturing = null;
  btn.classList.remove("capturing");
  // Every row, not just this one: a key can only drive one action, so binding
  // it here takes it away from whoever held it before.
  setKey(id, key).then(redrawKeyRows);
  return true;
}

function redrawKeyRows() {
  document.querySelectorAll("#settings tr[data-id]").forEach((row) => {
    const b = row.querySelector(".key-cell .set-key");
    if (!b || b.classList.contains("capturing")) return;
    const label = keyLabelFor(row.dataset.id);
    b.textContent = label;
    b.classList.toggle("unset", label === "—");
  });
}

function redrawPadRows() {
  document.querySelectorAll("#settings tr[data-id]").forEach((row) => {
    const b = row.querySelector(".pad-cell .set-pad");
    if (!b || b.classList.contains("capturing")) return;
    const i = padFor(row.dataset.id);
    b.textContent = padLabel(i);
    b.classList.toggle("unset", i === null);
  });
}

export function isCapturing() {
  return capturing !== null || padCapture !== null;
}

/// Rebuild this pane in place after a reset.
///
/// Both reset buttons used to call closeSettings() and toggleSettings(), which
/// were never imported here — so clicking either threw a ReferenceError and the
/// bindings were reset with nothing on screen changing to say so. Inside the
/// settings window there is no window to close and reopen anyway: redrawing the
/// pane is what "show me the new bindings" means.
function redraw(box) {
  box.innerHTML = html();
  wire(box);
}

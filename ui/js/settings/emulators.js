// The Emulators tab: which emulator runs each console, which shader it gets,
// which screen the game opens on, and the shape of its window.
import { invoke, MOBILE } from "../state.js";
import { toast, escapeHtml } from "../util.js";
import { wireConfigFields } from "./fields.js";

export const html = `${MOBILE ? "" : `      <h4>Game window</h4>
      <div class="srow">
        <label>Fit to the game</label>
        <div class="ctl"><button data-field="fit_window">…</button></div>
      </div>
      <p class="hint">A window shaped like the game has no room for black bars.
        Off fills the screen and lets RetroArch letterbox inside it.</p>
      <div class="srow">
        <label>Title bar</label>
        <div class="ctl"><button data-field="window_decorations">…</button></div>
      </div>
      <p class="hint">Off gives a clean edge, and leaves nothing to drag or
        click — the way out is then the pad combination or Escape.</p>

      <div class="srow sys-screen" hidden>
        <label>Open games on</label>
        <div class="ctl"><select class="game-display"></select></div>
      </div>
      <p class="hint sys-screen-hint" hidden>Automatic prefers an external
        screen: plugging a monitor in is a deliberate act, rarely done wanting
        the game on the laptop panel.</p>`}
      <!-- Nothing above this line exists on Android. RetroArch is started by an
           Intent and takes the whole screen: there is no window to shape, no
           title bar to remove, and no second display to choose. All three fed
           window_lines, which the Android launch does not generate at all, so
           they were three controls that wrote to config.toml and changed
           nothing. -->
      <div class="sys-motion"></div>

      <h4>Arcade rapid fire</h4>
      <div class="srow">
        <label>Auto-fire</label>
        <div class="ctl">
          <select class="cf-select" data-field="autofire">
            <option value="off">Off</option>
            <option value="lb">Hold LB — fire repeats while it is held</option>
            <option value="rb">Hold RB — the same on the other shoulder</option>
            <option value="y">Hold Y — sends button D as well</option>
          </select>
        </div>
      </div>
      <p class="hint">Arcade shooters only — 879 games here. A run of Metal Slug
        is a few thousand presses of a button the cabinet expected you to
        hammer.</p>
      <p class="hint">Hold the modifier on its own; let go and it stops. Do not hold the fire
      button too — a real press beats the repeat, giving one continuous shot.
      Nothing is remapped.</p>
      <p class="hint">One modifier only — RetroArch binds it once per player. LB and RB send
      nothing underneath them; Y also sends button D.</p>

      <div class="srow">
        <label>Save state on exit</label>
        <div class="ctl"><button data-field="save_state_on_exit"></button></div>
      </div>
      <p class="hint">Off by default. It writes to the <em>auto</em> slot, which Continue
      playing resumes from — so a brief look at a game would overwrite where
      you actually stopped.</p>

      <h4>Emulators</h4>
      <p class="hint">Which emulator runs each console, its shader, and whether
        a light gun takes port two. Applies to the next game you launch.</p>
      <details class="gun-help">
        <summary>How to play light gun games with the mouse</summary>
        <div>
          <p>Tick <b>Light gun</b> beside a console, then launch a gun game.
            Aim with the mouse; <b>left button</b> fires and <b>right button</b>
            shoots off-screen, which is how most gun games reload.</p>
          <p>The mouse is bound to the trigger on every launch, so there is
            nothing else to set up. What the tick does is tell the emulator a
            gun is plugged in — until then the game sees a joypad, however the
            mouse is bound, and shooting does nothing.</p>
          <p><b>Turn it off again afterwards.</b> On the NES, SNES and Mega
            Drive the gun goes in the <i>second</i> controller port, so leaving
            it on turns player two's pad into a gun and two-player games on
            that console stop working. The PlayStation's GunCon takes a port of
            its own, and arcade guns do not use a pad port at all.</p>
        </div>
      </details>
      <div class="sys-table">Loading…</div>

`;

/// The per-system table: emulator, shader and light gun for each console.
///
/// Rendered after the pane is on screen rather than as part of its markup,
/// because it needs two round trips — the systems themselves and the list of
/// motion shaders — and a tab that waited on those before drawing anything
/// would look like a tab that does not open.
export async function wire(box) {
  const table = box.querySelector(".sys-table");
  let rows, motion;
  try {
    [rows, motion] = await Promise.all([invoke("systems"), invoke("motion_options")]);
  } catch (e) {
    table.textContent = String(e);
    return;
  }
  // A slow answer that arrives after the tab was left has nowhere to go, and
  // writing to a detached node would leave the next tab looking fine while its
  // controls belong to a pane that is gone.
  if (!box.isConnected) return;

  await wireGameDisplay(box);
  // The two window toggles are config.toml fields, like the ones in General.
  wireConfigFields(box);
  box.querySelector(".sys-motion").innerHTML = motionMarkup(motion);
  table.innerHTML = `
    <table class="systbl">
      <thead>
        <tr><th>System</th><th>Games</th><th>Display</th><th>Emulator</th><th>Shader</th>
            ${MOBILE ? "" : '<th title="Aim with the mouse in light gun games">Light gun</th>'}</tr>
      </thead>
      <tbody>${rows.map(systemRow).join("")}</tbody>
    </table>`;

  for (const gun of box.querySelectorAll('input[data-field="lightgun"]')) {
    gun.addEventListener("change", async () => {
      const { slug, field } = gun.dataset;
      try {
        const said = await invoke("set_system_choice", {
          slug,
          field,
          value: gun.checked ? "on" : "off",
        });
        // Switching one on is the moment the explanation is wanted, and the
        // moment nobody goes looking for it — this is the one thing in the app
        // that does nothing visible until you know two facts about it.
        if (gun.checked) {
          box.querySelector(".gun-help")?.setAttribute("open", "");
          toast(`${said} — aim with the mouse, left button fires`, 9000);
        } else {
          toast(said);
        }
      } catch (e) {
        toast(String(e), 8000);
      }
    });
  }

  for (const sel of box.querySelectorAll(".sys-table select, .sys-motion select")) {
    sel.addEventListener("change", async () => {
      const { slug, field } = sel.dataset;
      try {
        // The motion layer is global, so it has its own command rather than a
        // per-system one. Everything else is keyed by platform slug.
        toast(
          field === "motion"
            ? await invoke("set_motion_shader", { value: sel.value })
            : await invoke("set_system_choice", { slug, field, value: sel.value })
        );
      } catch (e) {
        toast(String(e), 8000);
      }
    });
  }
}

/// Which screen a game opens on.
///
/// Hidden entirely with one display attached: a dropdown with a single entry is
/// a question that has one answer, and this is the settings pane rather than an
/// inventory of the machine.
async function wireGameDisplay(box) {
  const row = box.querySelector(".sys-screen");
  const hint = box.querySelector(".sys-screen-hint");
  const sel = box.querySelector(".game-display");
  if (!sel) return;

  let screens = [];
  try {
    screens = await invoke("game_displays");
  } catch {
    return;
  }
  if (!screens.length || !box.isConnected) return;

  row.hidden = false;
  hint.hidden = false;
  sel.innerHTML = screens
    .map(
      (d) =>
        `<option value="${escapeHtml(d.key)}" ${d.selected ? "selected" : ""}>
           ${escapeHtml(d.label)}</option>`
    )
    .join("");
  sel.addEventListener("change", async () => {
    try {
      toast(await invoke("set_config_field", { field: "game_display", value: sel.value }));
    } catch (e) {
      toast(`Could not save — ${e}`, 8000);
    }
  });
}

/// The strobe/BFI layer. Above the table and separate from it: it chains onto
/// whatever shader each system already uses rather than replacing one, so it
/// is not a per-system choice and should not look like a column.
function motionMarkup(motion) {
  if (!motion?.options?.length) return "";
  const options = motion.options
    .map(
      (o) =>
        `<option value="${o.path}" ${o.path === motion.current ? "selected" : ""}>${escapeHtml(
          o.label
        )} — ${escapeHtml(o.note)}</option>`
    )
    .join("");
  return `
    <div class="srow">
      <label>Motion layer</label>
      <div class="ctl">
        <select data-field="motion">
          <option value="none" ${!motion.current ? "selected" : ""}>Off</option>
          ${options}
        </select>
      </div>
    </div>
    <p class="hint">Blanks the screen between frames to cut LCD smearing. Needs a fixed
      refresh at a multiple of 60 — on a variable-refresh panel such as
      ProMotion it just flickers. CRT systems only.</p>`;
}

function systemRow(s) {
  const cores = s.emulators.length
    ? `<select data-slug="${s.slug}" data-field="core">${s.emulators
        .map(
          (e) =>
            `<option value="${e.core}" ${e.core === s.core ? "selected" : ""}>${escapeHtml(
              e.label
            )}${e.installed ? "" : " — not installed"}${e.is_default ? " (default)" : ""}</option>`
        )
        .join("")}</select>`
    : `<span class="dim">none known</span>`;

  const shaders = s.shaders.length
    ? `<select data-slug="${s.slug}" data-field="shader">
         <option value="none" ${!s.shader ? "selected" : ""}>None</option>
         ${s.shaders
           .map(
             (o) =>
               `<option value="${o.path}" ${o.path === s.shader ? "selected" : ""}>${escapeHtml(
                 o.label
               )}</option>`
           )
           .join("")}
       </select>`
    : `<span class="dim">no RetroArch</span>`;

  // Only for consoles that had a gun, and off by default: on most of them the
  // gun goes in the port a second pad would use, so leaving it on everywhere
  // would quietly break two-player games.
  //
  // Absent on Android. The gun is the mouse — the binds this writes are
  // input_player1_gun_trigger_mbtn and friends — and a handheld has no mouse to
  // aim with. Switching it on there took the port away from the pad and gave
  // back nothing that could shoot.
  const gun = s.gun
    ? `<label class="gun" title="${escapeHtml(s.gun)} in place of a pad — aim with the mouse, left button fires">
         <input type="checkbox" data-slug="${s.slug}" data-field="lightgun" ${s.gun_on ? "checked" : ""} />
         <span>${escapeHtml(s.gun)}</span>
       </label>`
    : `<span class="dim">—</span>`;

  return `<tr>
    <td class="sysname">${escapeHtml(s.name)}<div class="dim">${escapeHtml(s.slug)}</div></td>
    <td class="num">${s.rom_count}</td>
    <td><span class="badge ${s.display === "Handheld" ? "hh" : "crt"}">${escapeHtml(s.display)}</span></td>
    <td>${cores}</td>
    <td>${shaders}</td>
    ${MOBILE ? "" : `<td>${gun}</td>`}
  </tr>`;
}


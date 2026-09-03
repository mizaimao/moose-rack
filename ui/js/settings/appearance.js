// The Appearance tab: the pictures the lists draw, the console pictures, the
// glass, and the shader backdrop.
import { invoke, listen, convertFileSrc, MOBILE } from "../state.js";
import { toast, escapeHtml, cssColor } from "../util.js";
import { padFor, padLabelFor, keyLabelFor, actions } from "../bindings.js";
import {
  backdropSupported, backdropSettings, saveBackdropSettings,
  backdropWanted, setBackdropWanted, SCHEMES, ALL_SCHEMES, SCHEME_GROUPS,
  saveStyleSettings, styleSettings, clearStyleSettings, styleDefaults,
  glassTint, setGlassTint, glassStrength, setGlassStrength,
  backdropFps, setBackdropFps, BACKDROP_FPS_STEPS,
  BACKDROPS,
} from "../backdrop.js";
import { COLLECTION_ART, collectionArt, setCollectionArt } from "../collection-art.js";

export const html = `      <h4>Layout</h4>
      <div class="srow set-window-layout">
        <label>Window<span class="padmark" data-action="layout"></span></label>
        <div class="ctl">
          <select class="shell-mode">
            <option value="single">One pane — a screen at a time</option>
            <option value="columns">Three columns — consoles, games, preview</option>
          </select>
        </div>
      </div>
      <p class="hint set-window-layout">One pane at a time, with Back. Or three
        columns — consoles, games, preview — where nothing is ever replaced.</p>

      <div class="srow">
        <label>Consoles</label>
        <div class="ctl">
          <select class="view-layout" data-view="platforms">
            <option value="grid">Grid</option>
            <option value="list">List</option>
          </select>
        </div>
      </div>
      <div class="srow">
        <label>Games</label>
        <div class="ctl">
          <select class="view-layout" data-view="games">
            <option value="grid">Grid</option>
            <option value="list">List</option>
          </select>
        </div>
      </div>
      <p class="hint">Two settings, because they are two different questions. A
        dozen consoles are big pictures and read well as a grid; a console
        holding two thousand games reads better as a list. The button in the
        library changes whichever of the two you are looking at.</p>

      <h4>Artwork</h4>
      <div class="srow">
        <label>Game list shows<span class="padmark" data-action="pictures"></span></label>
        <div class="ctl"><select class="list-art"></select></div>
      </div>
      <p class="hint">What each game shows in the list and grid. Cartridge or
        disc keeps the grid even; anything missing falls back to the miximage,
        which is what the info pane always shows.</p>

      <h4>Console pictures</h4>
      <div class="srow">
        <label>Show<span class="padmark" data-action="pictures"></span></label>
        <div class="ctl"><div class="icon-styles"></div></div>
      </div>
      <p class="hint">What the console grid draws. Only styles with pictures are
        offered — a set that ships no controllers simply has no controllers to
        cycle to. Choose whose artwork in <b>Icon sets</b>.</p>
      <div class="srow">
        <label></label>
        <div class="ctl"><button class="set-icons">Get console pictures</button>
          <span class="set-icons-note"></span></div>
      </div>

      <div class="srow">
        <label>Collection picture</label>
        <div class="ctl"><select class="collection-art"></select></div>
      </div>
      <p class="hint collection-art-hint"></p>

      <!-- Not on Android. The icon in the launcher there is baked into the
           APK's mipmaps and set by the manifest, so nothing this app writes at
           runtime can change it and the row offered a choice that did nothing.
           wireAppIcons already leaves early when the holder is absent, so
           dropping the markup drops the app_icons round trip with it. -->
      ${MOBILE ? "" : `<h4>App icon</h4>
      <div class="srow">
        <label>Icon</label>
        <div class="ctl"><div class="app-icons"></div></div>
      </div>
      <p class="hint app-icons-note"></p>`}

      <h4>Color</h4>
      <div class="srow">
        <label>Scheme</label>
        <div class="ctl"><select class="scheme-preset"></select></div>
      </div>
      <p class="hint">One palette for the whole window — the glass and the
        backdrop behind it. Custom sets all three colors separately.</p>
      <div class="srow">
        <label>Glass</label>
        <div class="ctl"><input class="glass-strength" type="range" min="0" max="60" step="2" />
          <span class="glass-strength-val"></span></div>
      </div>
      <p class="hint">How solid every sheet of glass is — cards, selected row,
        cover art, preview pane. At 0 only the blur remains.</p>
      <p class="hint clarity-needs-backdrop">With the backdrop off there is nothing behind the glass
        to show, so this will look like it does little. Turn the backdrop on.</p>

      <div class="srow bd-custom">
        <label>Glass color</label>
        <div class="ctl"><input class="glass-custom" type="color" /></div>
      </div>

      <h4>Shader backdrop</h4>
      <div class="srow">
        <label>Enabled</label>
        <div class="ctl"><button class="set-backdrop">Shader backdrop: off</button></div>
      </div>
      <div class="srow">
        <label>Style</label>
        <div class="ctl"><select class="bd-style"></select></div>
      </div>
      <p class="hint bd-style-hint"></p>
      <p class="hint">Motion, brightness and color are remembered <em>per
        backdrop</em>. Anything you never change follows the shared setting.</p>
      <div class="srow">
        <label></label>
        <div class="ctl"><button class="bd-reset-style">Reset this backdrop</button>
          <span class="bd-style-state dim"></span></div>
      </div>
      <div class="srow">
        <label>Frame rate</label>
        <div class="ctl">
          <span class="stepper" role="group" aria-label="Backdrop frame rate">
            <button class="step-down" type="button" aria-label="Slower">&lsaquo;</button>
            <output class="bd-fps-val" aria-live="polite"></output>
            <button class="step-up" type="button" aria-label="Faster">&rsaquo;</button>
          </span>
        </div>
      </div>
      <p class="hint">How often the backdrop redraws. It is behind everything for
        as long as the app is open, so this is the setting that costs battery.
        <b>Off</b> draws it once and leaves it still.</p>

      <div class="srow">
        <label>Motion</label>
        <div class="ctl"><input class="bd-speed" type="range" min="0" max="700" step="5" />
          <input class="bd-speed-num num" type="number" min="0" step="1" />
          <span class="pct">%</span></div>
      </div>
      <div class="srow">
        <label>Brightness</label>
        <div class="ctl"><input class="bd-strength" type="range" min="0" max="300" step="5" />
          <input class="bd-strength-num num" type="number" min="0" step="1" />
          <span class="pct">%</span></div>
      </div>
      <div class="srow bd-custom">
        <label>Dark color</label>
        <div class="ctl"><input class="bd-low" type="color" />
          <button class="bd-low-reset">Use theme</button></div>
      </div>
      <div class="srow bd-custom">
        <label>Light color</label>
        <div class="ctl"><input class="bd-high" type="color" />
          <button class="bd-high-reset">Use theme</button></div>
      </div>
      <p class="hint">Drawn on the GPU behind the library — not behind this
        window. Motion at 0 holds it still.</p>
      <p class="hint set-backdrop-status"></p>

      <h4>Attract mode</h4>
      <div class="srow">
        <label>After</label>
        <div class="ctl">
          <input class="at-idle" type="number" min="0" max="3600" step="30" />
          <span class="dim">seconds idle</span>
          <button class="at-preview">Show it now</button>
        </div>
      </div>
      <p class="hint">Left alone this long, the app starts showing artwork from
        your library, and a press starts whatever is on screen. <b>0 turns it
        off.</b></p>
`;

/// Say which settings the controller can change without opening this window.
///
/// Cycling the artwork from the sofa is the reason several of these exist and
/// nothing here said so — the button is bound on the Control tab, two tabs
/// away, under a name ("Change the pictures") that does not obviously mean this
/// row. Read live rather than baked into the markup, so rebinding one is
/// reflected the next time this tab is opened.
function markPadControls(box) {
  for (const mark of box.querySelectorAll(".padmark")) {
    const id = mark.dataset.action;
    const action = actions().find((a) => a.id === id);
    const pad = padLabelFor(id);
    const key = keyLabelFor(id);
    if (pad === "unset" && key === "—") {
      // Nothing bound to it: a badge pointing at a button that does not exist
      // is worse than no badge.
      mark.remove();
      continue;
    }
    mark.innerHTML = `<span class="icon icon-pad"></span>${escapeHtml(
      pad === "unset" ? key : pad.split(" / ")[0]
    )}`;
    mark.title =
      `"${action?.label ?? id}" — changeable without opening this window.\n` +
      `Controller: ${pad}\nKeyboard: ${key === "—" ? "unset" : key}\n` +
      `Both are rebindable on the Control tab.`;
  }
}

export function wire(box) {
  // Three columns needs a window. This screen is 833 points wide and the
  // three-column shell wants roughly twice that before the middle column has
  // room for a name, so the option offered a layout that could not be drawn.
  if (/\bAndroid\b/.test(navigator.userAgent)) {
    box.querySelectorAll(".set-window-layout").forEach((n) => (n.hidden = true));
  }

  markPadControls(box);
  wireShellMode(box);
  wireViewLayouts(box);
  wireBackdropFps(box);
  wireAttract(box);
  wireIconStyles(box);
  wireAppIcons(box);

  // What the game lists draw. Populated from the backend rather than listed
  // here, so the names cannot drift from the directories they map to.
  const artSel = box.querySelector(".list-art");
  if (artSel) {
    invoke("list_art_options")
      .then(([choices, current]) => {
        artSel.innerHTML = choices
          .map(([k, label]) =>
            `<option value="${k}" ${k === current ? "selected" : ""}>${escapeHtml(label)}</option>`
          )
          .join("");
      })
      .catch(() => (artSel.disabled = true));
    // Same again for the game-list pictures, which Select cycles from inside a
    // console.
    listen("art-changed", () => {
      invoke("list_art_options")
        .then(([, current]) => {
          if (artSel.isConnected) artSel.value = current;
        })
        .catch(() => {});
    });
    artSel.addEventListener("change", async () => {
      try {
        toast(await invoke("set_list_art", { value: artSel.value }));
        // The library window redraws itself; this one cannot reach its DOM.
        window.__TAURI__?.event?.emit?.("art-changed");
      } catch (e) {
        toast(String(e), 8000);
      }
    });
  }

  // Shader backdrop. A switch rather than always-on: it runs a GPU loop for as
  // long as the app is open, which is not a cost to impose on someone who did
  // not ask for it.
  const bdBtn = box.querySelector(".set-backdrop");
  const bdStatus = box.querySelector(".set-backdrop-status");
  const paintBackdropButton = () => {
    const on = backdropWanted();
    bdBtn.textContent = `Shader backdrop: ${on ? "on" : "off"}`;
    bdBtn.classList.toggle("active", on);
  };
  if (!backdropSupported()) {
    bdBtn.disabled = true;
    bdBtn.textContent = "Shader backdrop: unavailable";
    bdStatus.textContent =
      "This machine's graphics driver does not offer WebGL2 to the app window.";
  } else {
    paintBackdropButton();
    bdBtn.addEventListener("click", () => {
      // Toggles the library window, not this one. This window has no library
      // behind it to put a backdrop on.
      const on = setBackdropWanted(!backdropWanted());
      bdStatus.textContent = on ? "On in the library window." : "";
      paintBackdropButton();
      // The preview pane's slider only means something with this on.
      box.querySelector(".clarity-needs-backdrop").hidden = on;
    });
  }

  // Live controls. Every change is applied to the running shader immediately —
  // a color picker whose result you cannot see is not usable.
  const cfg = backdropSettings();
  const speed = box.querySelector(".bd-speed");
  const speedNum = box.querySelector(".bd-speed-num");
  const strength = box.querySelector(".bd-strength");
  const strengthNum = box.querySelector(".bd-strength-num");
  const low = box.querySelector(".bd-low");
  const high = box.querySelector(".bd-high");

  // One scheme for both surfaces. The glass tint applies to this window
  // immediately as well as being announced, so the effect is visible on the
  // control that changed it.
  const schemeSel = box.querySelector(".scheme-preset");
  const glassCustom = box.querySelector(".glass-custom");
  const strengthEl = box.querySelector(".glass-strength");
  const strengthOut = box.querySelector(".glass-strength-val");

  // The styles, and what each one is. Switching applies to the library window
  // immediately, which is the only way to judge one.
  const styleEl = box.querySelector(".bd-style");
  const styleHint = box.querySelector(".bd-style-hint");
  if (styleEl) {
    const cfg = backdropSettings();
    styleEl.innerHTML = BACKDROPS.map(
      (b) =>
        `<option value="${escapeHtml(b.id)}" ${b.id === (cfg.style ?? "blobs") ? "selected" : ""}>${escapeHtml(b.label)}</option>`
    ).join("");
    const sayHint = () => {
      styleHint.textContent = BACKDROPS.find((b) => b.id === styleEl.value)?.hint ?? "";
    };
    sayHint();
    styleEl.addEventListener("change", () => {
      // Show the new style's own numbers before anything else happens.
      //
      // The sliders were left holding the previous style's values, so the
      // next nudge of one wrote *that* number onto the style just selected —
      // settings leaking from one backdrop into the next, and the reason a
      // newly chosen backdrop seemed to need a slider touched before it
      // looked right.
      showValues(saveBackdropSettings({ style: styleEl.value }));
      sayHint();
      sayStyleState();
    });
  }

  // Said out loud rather than left to be discovered: a slider whose effect
  // depends on another setting, with nothing saying so, reads as a slider that
  // does not work — which is exactly how it was reported.
  const sayBackdrop = () => {
    const note = box.querySelector(".clarity-needs-backdrop");
    if (note) note.hidden = backdropWanted();
  };
  sayBackdrop();

  // Per-backdrop overrides: say when the style has its own answers, and offer
  // to put it back on the shared ones.
  const resetBtn = box.querySelector(".bd-reset-style");
  const stateNote = box.querySelector(".bd-style-state");
  const sayStyleState = () => {
    const cur = backdropSettings().style;
    // Some shapes have no use for one of the controls. Static re-hashes its
    // whole field every frame at any speed, so the motion slider moves a
    // number and changes nothing you can see — disabled and labelled, rather
    // than left there looking broken.
    const shape = BACKDROPS.find((b) => b.id === cur);
    const noMotion = shape?.motion === false;
    if (speed) speed.disabled = noMotion;
    if (speedNum) speedNum.disabled = noMotion;
    if (speed?.closest(".srow")) {
      speed.closest(".srow").title = noMotion
        ? "This backdrop redraws completely every frame, so motion has no effect on it"
        : "";
    }
    const own = Object.keys(styleSettings(cur));
    const d = styleDefaults(cur);
    if (stateNote) {
      stateNote.textContent =
        `default ${Math.round(d.speed * 100)}% motion, ${Math.round(d.strength * 100)}% brightness`;
    }
    // Always available: it restores this backdrop's own defaults, which is
    // meaningful even when nothing has been changed yet.
    if (resetBtn) resetBtn.disabled = false;
  };
  sayStyleState();
  resetBtn?.addEventListener("click", () => {
    showValues(clearStyleSettings(backdropSettings().style));
    sayStyleState();
  });

  // What a collection card draws for a picture.
  const artSelEl = box.querySelector(".collection-art");
  if (artSelEl) {
    const hint = box.querySelector(".collection-art-hint");
    artSelEl.innerHTML = COLLECTION_ART
      .map(([id, label]) => `<option value="${id}">${escapeHtml(label)}</option>`)
      .join("");
    const sayArt = () => {
      const found = COLLECTION_ART.find(([id]) => id === artSelEl.value);
      if (hint) hint.textContent = found ? found[2] : "";
    };
    artSelEl.value = collectionArt();
    sayArt();
    artSelEl.addEventListener("change", () => {
      setCollectionArt(artSelEl.value);
      sayArt();
    });
  }

  if (strengthEl) {
    strengthEl.value = String(glassStrength());
    strengthOut.textContent = `${glassStrength()}%`;
    strengthEl.addEventListener("input", () => {
      strengthOut.textContent = `${setGlassStrength(strengthEl.value)}%`;
    });
  }

  // The individual pickers only mean anything on "custom"; beside a preset
  // they invite changing one and watching the preset put it back.
  const showCustom = () => {
    const on = schemeSel.value === "custom";
    box.querySelectorAll(".bd-custom").forEach((r) => (r.hidden = !on));
  };

  // Grouped: forty schemes in one flat list is a scroll, not a choice.
  schemeSel.innerHTML = SCHEME_GROUPS.map(
    ([label, items]) =>
      `<optgroup label="${escapeHtml(label)}">` +
      items.filter(Boolean).map((c) => `<option value="${c.id}">${escapeHtml(c.label)}</option>`).join("") +
      `</optgroup>`
  ).join("");
  schemeSel.value = ALL_SCHEMES.filter(Boolean).some((c) => c.id === cfg.preset) ? cfg.preset : "midnight";
  glassCustom.value = glassTint();
  showCustom();

  schemeSel.addEventListener("change", () => {
    // Every scheme, not only the two-color ones. This read `SCHEMES`, which
    // holds the pairs alone — so choosing any single color or spectrum found
    // nothing, fell into the "custom" branch below, and silently kept the
    // previous colors. The dropdown offered eighteen new schemes and none of
    // them did anything.
    const chosen = ALL_SCHEMES.filter(Boolean).find((c) => c.id === schemeSel.value);
    showCustom();
    if (!chosen || chosen.id === "custom") {
      // Keep whatever the three pickers already hold rather than blanking
      // them: "custom" means "leave this to me", not "start again".
      saveBackdropSettings({ preset: "custom" });
      return;
    }
    setGlassTint(chosen.glass);
    glassCustom.value = chosen.glass;
    low.value = chosen.low;
    high.value = chosen.high;
    saveBackdropSettings({ preset: chosen.id });
  });
  glassCustom.addEventListener("input", () => setGlassTint(glassCustom.value));

  // The slider and the number field are two views of one value, so each has
  // to write the other. The field takes anything: the slider's ceiling is where
  // it stops being useful, not where the shader stops working, and someone who
  // wants 900% motion on one backdrop should be able to type it.
  const showValues = (c) => {
    const sp = Math.round(c.speed * 100);
    const st = Math.round(c.strength * 100);
    if (document.activeElement !== speedNum) speedNum.value = String(sp);
    if (document.activeElement !== strengthNum) strengthNum.value = String(st);
    speed.value = String(Math.min(Number(speed.max), sp));
    strength.value = String(Math.min(Number(strength.max), st));
  };
  // A color input cannot show "unset", so an empty value reads back as the
  // theme's own color and the reset button is what clears it again.
  low.value = cfg.low || cssColor("--bg", "#0d0d12");
  high.value = cfg.high || cssColor("--accent", "#2e3358");
  showValues(cfg);

  const setSpeed = (pct) =>
    showValues(saveStyleSettings(backdropSettings().style, { speed: Math.max(0, pct) / 100 }));
  const setStrength = (pct) =>
    showValues(saveStyleSettings(backdropSettings().style, { strength: Math.max(0, pct) / 100 }));

  speed.addEventListener("input", () => setSpeed(Number(speed.value)));
  strength.addEventListener("input", () => setStrength(Number(strength.value)));
  // `change`, not `input`: typing "150" passes through 1 and 15 on the way, and
  // applying each would jerk the backdrop while the number is half-written.
  speedNum.addEventListener("change", () => setSpeed(Number(speedNum.value) || 0));
  strengthNum.addEventListener("change", () => setStrength(Number(strengthNum.value) || 0));
  low.addEventListener("input", () => saveBackdropSettings({ low: low.value }));
  high.addEventListener("input", () => saveBackdropSettings({ high: high.value }));
  box.querySelector(".bd-low-reset").addEventListener("click", () => {
    saveBackdropSettings({ low: "" });
    low.value = cssColor("--bg", "#0d0d12");
  });
  box.querySelector(".bd-high-reset").addEventListener("click", () => {
    saveBackdropSettings({ high: "" });
    high.value = cssColor("--accent", "#2e3358");
  });
}

/// Which pictures the console grid uses, and how to get more.
///
/// This was a whole top-level panel with a gallery of ES-DE themes,
/// screenshots, and a full-download button. The app never rendered a theme —
/// it only ever read the per-system artwork out of one — so all that offered
/// was a choice that changed nothing and a download measured in hundreds of
/// megabytes. What is left is the part that does change the screen.
/// One pane or three columns.
///
/// Applied to the library window rather than this one — the setting is about
/// where things are drawn over there, and this window has no list, no consoles
/// and no preview to rearrange.
function wireShellMode(box) {
  const sel = box.querySelector(".shell-mode");
  if (!sel) return;
  sel.value = localStorage.getItem("moose.shell") === "columns" ? "columns" : "single";
  sel.addEventListener("change", () => {
    localStorage.setItem("moose.shell", sel.value);
    window.__TAURI__?.event?.emit?.("shell-mode", sel.value);
    toast(sel.value === "columns" ? "Three columns" : "One pane");
  });
}

/// The backdrop's frame rate, as a stepper rather than a slider.
///
/// A ladder of eight values, because the useful choices are few and far apart —
/// between 28 and 30 there is nothing to see, while between 1 and 30 there is
/// everything. Both ends stop rather than wrap: an arrow that quietly takes 60
/// to 0 is a trap.
function wireBackdropFps(box) {
  const out = box.querySelector(".bd-fps-val");
  if (!out) return;
  const down = box.querySelector(".stepper .step-down");
  const up = box.querySelector(".stepper .step-up");

  const label = (n) => (n === 0 ? "Off" : `${n} fps`);
  const paint = () => {
    const now = backdropFps();
    const i = BACKDROP_FPS_STEPS.indexOf(now);
    out.textContent = label(now);
    down.disabled = i <= 0;
    up.disabled = i >= BACKDROP_FPS_STEPS.length - 1;
  };
  const step = (delta) => {
    const i = BACKDROP_FPS_STEPS.indexOf(backdropFps());
    const next = BACKDROP_FPS_STEPS[Math.min(BACKDROP_FPS_STEPS.length - 1, Math.max(0, i + delta))];
    setBackdropFps(next);
    paint();
    toast(`Backdrop: ${label(next)}`);
  };
  down.addEventListener("click", () => step(-1));
  up.addEventListener("click", () => step(1));
  paint();
}

/// Grid or list, separately for the console screen and for a console's games.
///
/// Stored under the same keys the library window uses and announced with an
/// event, which is how every other setting here reaches that window — it has
/// the list, this one does not.
function wireViewLayouts(box) {
  for (const sel of box.querySelectorAll(".view-layout")) {
    const view = sel.dataset.view;
    const key = view === "platforms" ? "layout.platforms" : "layout";
    sel.value = localStorage.getItem(key) === "list" ? "list" : "grid";
    sel.addEventListener("change", () => {
      localStorage.setItem(key, sel.value);
      window.__TAURI__?.event?.emit?.("layout-view", { view, value: sel.value });
      toast(`${view === "platforms" ? "Consoles" : "Games"}: ${sel.value}`);
    });
  }
}

/// Which picture the app itself wears.
///
/// Drawn from whatever `app_icons` returns rather than from a list here, so
/// shipping a third icon is a line in `src/appicon.rs` and a PNG — this file
/// does not learn about it.
async function wireAppIcons(box) {
  const holder = box.querySelector(".app-icons");
  const note = box.querySelector(".app-icons-note");
  if (!holder) return;

  let icons = [];
  try {
    icons = await invoke("app_icons");
  } catch {
    holder.textContent = "Could not read the icons this build ships";
    return;
  }
  if (!box.isConnected) return;

  holder.innerHTML = icons
    .map(
      (i) =>
        `<button class="app-icon ${i.selected ? "on" : ""}" data-id="${escapeHtml(i.id)}"
           title="${escapeHtml(i.label)}" aria-label="${escapeHtml(i.label)}">
           ${i.preview
             ? `<img src="${convertFileSrc(i.preview)}" alt="${escapeHtml(i.label)}" />`
             : `<span class="app-icon-missing"></span>`}
         </button>`
    )
    .join("");

  for (const b of holder.querySelectorAll(".app-icon")) {
    b.addEventListener("click", async () => {
      // Marked before the call rather than after: replacing the bundle's icon
      // takes a moment on macOS, and a picker that does not move when clicked
      // reads as broken.
      holder.querySelectorAll(".app-icon").forEach((x) => x.classList.toggle("on", x === b));
      try {
        const said = await invoke("set_app_icon", { id: b.dataset.id });
        if (note) note.textContent = said;
        toast(said);
      } catch (e) {
        if (note) note.textContent = String(e);
        toast(String(e));
      }
    });
  }
}

async function wireIconStyles(box) {
  const holder = box.querySelector(".icon-styles");
  const note = box.querySelector(".set-icons-note");
  if (!holder) return;

  const draw = async () => {
    let styles = [];
    try {
      styles = await invoke("icon_styles");
    } catch {
      holder.textContent = "Could not read the installed pictures";
      return;
    }
    if (!box.isConnected) return;
    // Nothing installed at all is a different answer from "this style has
    // none", and it looked identical: every button grayed out, no explanation,
    // and a Get button below that nobody connects to the row above. Reported
    // from Windows as not being able to cycle them at all, which is exactly
    // what a row of disabled buttons is.
    if (styles.length && styles.every((s) => !s.available)) {
      holder.innerHTML = `<p class="hint" style="margin:0">No console pictures have been
        fetched yet, so there is nothing to choose between. Press
        <strong>Get console pictures</strong> below — it takes a few hundred
        kilobytes from four ES-DE themes and then this row fills in.</p>`;
      return;
    }
    holder.innerHTML = styles
      .map(
        (s) =>
          `<button class="icon-style ${s.selected ? "on" : ""}" data-style="${escapeHtml(s.key)}"
             ${s.available ? "" : "disabled"}>${escapeHtml(s.label)}
             <em>${s.available}</em></button>`
      )
      .join("");
    for (const b of holder.querySelectorAll(".icon-style:not([disabled])")) {
      b.addEventListener("click", async () => {
        try {
          const label = await invoke("set_icon_style", { key: b.dataset.style });
          holder
            .querySelectorAll(".icon-style")
            .forEach((x) => x.classList.toggle("on", x === b));
          // The console grid is in the other window and cannot be reached from
          // here. Without this the choice only appeared after leaving the
          // console page and coming back, which reads as the setting not
          // having taken.
          window.__TAURI__?.event?.emit?.("icons-changed");
          toast(`Console pictures: ${label}`);
        } catch (e) {
          toast(String(e), 6000);
        }
      });
    }
  };
  await draw();

  // The pad changes this from the library window, which cannot reach this one.
  // Without the redraw the panel would sit there showing the style that was
  // selected when it opened, disagreeing with the screen behind it.
  listen("icons-changed", draw);

  box.querySelector(".set-icons")?.addEventListener("click", async (e) => {
    const btn = e.currentTarget;
    btn.disabled = true;
    // A picture per console per style, so it counts rather than going quiet —
    // a button that says nothing for a while is a button people press again.
    const stop = await listen("icons-progress", ({ payload }) => {
      note.textContent = String(payload);
    });
    try {
      const summary = await invoke("fetch_icons");
      note.textContent = summary.split("\n")[0];
      window.__TAURI__?.event?.emit?.("icons-changed");
      toast(summary, 9000);
      await draw();
    } catch (err) {
      note.textContent = "";
      toast(`Could not fetch pictures — ${err}`, 9000);
    } finally {
      stop?.();
      btn.disabled = false;
    }
  });
}

/// How long the app waits before it starts showing artwork, and a way to see it
/// without waiting.
///
/// The preview button is not a convenience. Five minutes is the default and
/// nobody is going to sit through it to find out whether the thing works, so
/// without this the feature can only be judged by changing a setting and going
/// away — which is how a broken screensaver ships.
function wireAttract(box) {
  const field = box.querySelector(".at-idle");
  if (!field) return;
  import("../attract.js").then(({ attractIdleSeconds, setAttractIdleSeconds }) => {
    field.value = String(attractIdleSeconds());
    field.addEventListener("change", () => {
      field.value = String(setAttractIdleSeconds(field.value));
      toast(Number(field.value) > 0
        ? `Attract mode after ${field.value} seconds`
        : "Attract mode off");
    });
  });
  box.querySelector(".at-preview")?.addEventListener("click", async () => {
    // Shown in the window that has the library in it, not this one: the
    // settings page is its own document and covering it would prove nothing.
    try {
      await window.__TAURI__.event.emit("attract-now");
      toast("Showing it in the main window");
    } catch {
      toast("Could not reach the main window", 6000);
    }
  });
}

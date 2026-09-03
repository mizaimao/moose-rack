// One button that changes whatever pictures are on screen.
//
// Which pictures depends on where you are, and that is the point: on the
// console grid there is one kind of picture worth changing (the console's), and
// inside a console there is another (the game's). A single button that means
// "show me these differently" needs no explanation in either place, where two
// buttons would need one in both.
//
// It replaced Select-opens-settings. Settings is a second window of text fields
// and tables that a controller cannot navigate, so that button opened something
// you could then only leave again.

import { state, invoke } from "./state.js";
import { toast } from "./util.js";
import { showPlatforms, renderRows } from "./library.js";

/// Views that are lists of games rather than lists of consoles.
const GAME_VIEWS = new Set(["roms", "search", "collection-roms"]);

/// Step to the next entry, wrapping. `-1` (not found) lands on the first,
/// which is what should happen when the current setting is one that has since
/// stopped being offered.
function next(values, current) {
  if (!values.length) return null;
  return values[(values.indexOf(current) + 1) % values.length];
}

/// Cycle the pictures for whatever is on screen.
export async function cyclePictures() {
  return GAME_VIEWS.has(state.view) ? cycleListArt() : cycleIconStyle();
}

/// The console grid: logos, consoles, controllers, hardware.
async function cycleIconStyle() {
  let styles;
  try {
    styles = await invoke("icon_styles");
  } catch (e) {
    return toast(String(e), 6000);
  }
  // An answer that is not a list is as unusable as no answer. Guarding the
  // shape as well as the call, because this runs from a controller button and
  // a throw here escapes into the poll loop rather than anywhere visible.
  if (!Array.isArray(styles)) return toast("Could not read the console pictures");
  // Only styles that have pictures. Cycling onto an empty one shows a grid of
  // nothing, which reads as the button having broken the page.
  const usable = styles.filter((s) => s.available > 0);
  if (usable.length < 2) {
    return toast("Only one set of console pictures is installed — Settings → Appearance");
  }
  const now = usable.find((s) => s.selected)?.key ?? usable[0].key;
  const want = next(usable.map((s) => s.key), now);
  try {
    const label = await invoke("set_icon_style", { key: want });
    // Only when the console grid is what is on screen.
    //
    // Redrawing it unconditionally put the platform grid inside whichever tab
    // was open — press Select in My collections and that tab was showing
    // consoles. The section machinery then remembered where each tab had been
    // left, so My collections went on showing the platform grid every time you
    // returned to it, and after a few presses every tab showed the same thing.
    if (state.view === "platforms") await showPlatforms();
    window.__TAURI__?.event?.emit?.("icons-changed");
    toast(`Console pictures: ${label}`);
  } catch (e) {
    toast(String(e), 6000);
  }
}

/// A list of games: cartridge, box, 3D box, mix, and so on.
async function cycleListArt() {
  let options, current;
  try {
    [options, current] = (await invoke("list_art_options")) ?? [];
  } catch (e) {
    return toast(String(e), 6000);
  }
  if (!Array.isArray(options)) return toast("Could not read the picture options");
  const keys = options.map(([k]) => k);
  const want = next(keys, current);
  if (!want) return;
  try {
    const label = await invoke("set_list_art", { value: want });
    if (state.rows.length) renderRows(state.rows, state.view === "search");
    window.__TAURI__?.event?.emit?.("art-changed");
    toast(String(label));
  } catch (e) {
    toast(String(e), 6000);
  }
}

/// The miximage, without cycling to find it.
///
/// Bound to a long press of the same button that cycles, because the cycle is
/// seven long and the miximage is the one people come back to: it carries a
/// screenshot, the box and the logo in one picture, so it is the setting that
/// works for every console at once. Six presses to get home from the wrong end
/// of the list is how a good control becomes an annoying one.
export async function useMiximage() {
  try {
    const label = await invoke("set_list_art", { value: "miximages" });
    if (state.rows.length) renderRows(state.rows, state.view === "search");
    window.__TAURI__?.event?.emit?.("art-changed");
    toast(String(label));
  } catch (e) {
    toast(String(e), 6000);
  }
}

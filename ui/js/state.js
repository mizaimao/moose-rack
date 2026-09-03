// Shared UI state and cached element handles.
//
// A plain mutable object rather than a store: the app is small and every view
// reads the same handful of fields.

export const state = {
  view: "platforms", // platforms | roms | search | themes | collections | collection-roms
  platform: null,
  /// The console last opened, so returning to the grid puts the cursor back on
  /// it instead of the top-left card.
  lastPlatform: localStorage.getItem("lastPlatform") || null,
  rows: [],
  selected: null,
  /// Platform slug -> cover aspect (w/h), measured by the backend.
  aspects: {},
  /// The folder open inside the current console: `""` at the top,
  /// `"Aftermarket"` or `"AdditionalRoms/Homebrew"` below it.
  ///
  /// ES-DE shows a subfolder as a folder you walk into. The scan records where
  /// each game sits, the whole console arrives in one call, and this is the
  /// only thing that changes when you open one — so walking the tree is a
  /// redraw, not a round trip.
  folder: "",
  /// The collection currently open, so its position can be remembered too.
  collection: null,
  /// Its name, separate from the rendered title. The title carries a count and
  /// re-deriving one from the other appends it twice.
  collectionName: null,
  /// The consoles as last fetched, so switching layout redraws them without
  /// another round trip.
  platforms: [],
  /// List key -> the rom id last selected in it. Coming back to a console or a
  /// collection puts the cursor where you left it rather than at the top, which
  /// otherwise means re-finding your place on every single trip in and out.
  lastRom: JSON.parse(localStorage.getItem("lastRom") || "{}"),
  /// The layout of whatever is on screen *now*.
  ///
  /// Derived, not stored: the two lines below are what persist, and this is
  /// whichever of them the current view uses. Kept as one field because the
  /// twenty-odd places that draw a list ask "grid or not" about the thing they
  /// are drawing, and none of them should have to know which view it belongs
  /// to. `applyLayoutForView` in library.js keeps it in step.
  layout: localStorage.getItem("layout") || "grid",
  /// The console screen, and the games inside a console: two settings, because
  /// they are two different questions. Consoles are a short list of big
  /// pictures and read well as a grid; a console holding two thousand games
  /// reads better as a list, and having one control both meant choosing which
  /// of the two to have wrong.
  ///
  /// `layout` is the older key and stays the games one, so nobody's existing
  /// choice is reset by this gaining a second half.
  layoutGames: localStorage.getItem("layout") || "grid",
  layoutPlatforms: localStorage.getItem("layout.platforms") || "grid",
  // Per level; see `sidebarScope` in detail.js. Seeded from the console
  // screen because that is what opens first.
  sidebar: (localStorage.getItem("sidebar.platforms")
    ?? localStorage.getItem("sidebar") ?? "on") !== "off",
  /// Base card width in px; the grid scales from this.
  zoom: Number(localStorage.getItem("zoom")) || 150,
  gamepad: null,
};

/// Where "back" goes, as a stack. Collections browse three levels deep while
/// every other view returns straight to the platform grid. Lives here rather
/// than in collections.js so library.js can clear it without an import cycle.
export const trail = [];

export const el = {
  tabbar: document.getElementById("tabbar"),
  list: document.getElementById("list"),
  consoles: document.getElementById("consoles"),
  detail: document.getElementById("detail"),
  title: document.getElementById("title"),
  back: document.getElementById("back"),
  search: document.getElementById("search"),
  status: document.getElementById("status"),
  toast: document.getElementById("toast"),
  layoutBtn: document.getElementById("layout-btn"),
  sortBtn: document.getElementById("sort-btn"),
  filterBtn: document.getElementById("filter-btn"),
  randomBtn: document.getElementById("random-btn"),
  grabBtn: document.getElementById("grab-btn"),
  sidebarBtn: document.getElementById("sidebar-btn"),
  lb: document.getElementById("lightbox"),
  settingsBtn: document.getElementById("settings-btn"),
  syncBtn: document.getElementById("sync-btn"),
  zoom: document.getElementById("zoom"),
  zoomWrap: document.getElementById("zoom-wrap"),
  viewSwitch: document.getElementById("view-switch"),
  pageFilter: document.getElementById("pfilter"),
  pageFilterBar: document.getElementById("page-filter"),
  sectionStrip: document.getElementById("section-strip"),
};

// Tauri globals, exposed once so modules do not each reach into window.
export const { invoke, convertFileSrc } = window.__TAURI__.core;
export const { listen } = window.__TAURI__.event;

/// Which list is on screen, as a key that survives a restart.
///
/// Search is deliberately excluded: the term changes, so a remembered position
/// in one search means nothing in the next.
export function listKey() {
  if (state.view === "roms" && state.platform) return `platform:${state.platform}`;
  if (state.view === "collection-roms" && state.collection) return `collection:${state.collection}`;
  return null;
}

/// Record the cursor position for the list currently on screen.
export function rememberRom(id) {
  const key = listKey();
  if (!key || id == null) return;
  state.lastRom[key] = id;
  try {
    localStorage.setItem("lastRom", JSON.stringify(state.lastRom));
  } catch {
    // A full or disabled localStorage costs the memory across restarts, not
    // within the session, so it is not worth failing a selection over.
  }
}

/// The rom to put the cursor on when a list is rendered, if it is still there.
export function rememberedRom(rows) {
  const key = listKey();
  if (!key) return null;
  const want = state.lastRom[key];
  return rows.some((r) => r.id === want) ? want : null;
}

/// Whether this is the Android build.
///
/// A handheld is not a small desktop. It has no window to drag, no title bar to
/// hide under, and a screen that has to spend every one of its 469 points on
/// the library rather than on chrome. The differences that follow from that are
/// gated on this rather than scattered as separate checks, so there is one
/// answer to change when the next device arrives.
export const MOBILE = /\bAndroid\b/.test(navigator.userAgent);

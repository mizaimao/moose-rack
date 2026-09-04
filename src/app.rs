//! Everything the app does, independent of how it is being asked.
//!
//! This is the backend. `src-tauri` wraps it in Tauri commands for the desktop
//! window; `src-service` wraps the same functions in HTTP for the server. The
//! logic is written once and lives here, so the two cannot drift.
//!
//! It moved out of `src-tauri/src/lib.rs` because that made it desktop-only by
//! accident rather than by design: nothing in `AppState` was ever Tauri-typed —
//! it holds a cache, a core map, an API client, some paths and some settings —
//! and the only thing keeping it there was where the file happened to be.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::{api, cache, config::Config, coremap::CoreMap, retroarch::RetroArch};
use std::path::Path;

/// Long-lived process state. The SQLite connection is not `Sync`, so it lives
/// behind a mutex and is only held for the duration of a query.
pub struct AppState {
    pub cache: Mutex<cache::Cache>,
    pub map: CoreMap,
    pub client: Option<Arc<api::Client>>,
    pub retroarch: Option<RetroArch>,
    pub roms_dir: PathBuf,
    pub media_dir: PathBuf,
    /// Artwork of a locally scanned ES-DE library. Keyed by ES-DE *system*
    /// name rather than RomM slug, so it needs its own lookup rather than
    /// being folded into `media_dir`.
    pub esde_media: Option<PathBuf>,
    pub theme_root: Option<String>,
    pub themes_dir: PathBuf,
    /// Bind players 2-4 like player 1. See config::ControllersCfg.
    pub mirror_players: bool,
    /// Shape the game window like the game, so it has no black bars.
    pub fit_window: bool,
    /// Keep the game window's title bar.
    pub window_decorations: bool,
    /// Behind mutexes so a choice made in the UI takes effect on the next
    /// launch rather than the next restart. `config.toml` stays the source of
    /// truth; these are the live copy.
    pub core_overrides: Mutex<std::collections::BTreeMap<String, String>>,
    pub core_per_game: Mutex<std::collections::BTreeMap<String, String>>,
    pub user_retroarch_cfg: PathBuf,
    pub shaders_enabled: bool,
    pub shader_overrides: Mutex<std::collections::BTreeMap<String, String>>,
    /// Systems switched over to a light gun, so gun games aim with the mouse.
    pub lightgun: Mutex<std::collections::BTreeMap<String, String>>,
    /// Which ES-DE artwork the list and the info pane draw.
    pub list_art: Mutex<String>,
    pub detail_art: String,
    /// Strobe/BFI pass chained onto CRT shaders, if the user enabled one.
    pub motion_shader: Mutex<Option<String>>,
    /// The look the grid draws — an id from the chosen set's own list, not one
    /// of three fixed kinds. Themes offer between one and nine.
    pub icon_look: Mutex<String>,
    /// A downloaded ES-DE icon set the grid draws from, or empty for the
    /// shared pool. Orthogonal to `icon_style`: whose art, versus which kind.
    pub icon_set: Mutex<String>,
    /// RetroAchievements, read once at startup from this project's config.toml
    /// — see `crate::achievements`.
    pub achievements: crate::achievements::Settings,
    /// Pull before a launch and push after it exits.
    pub auto_sync: bool,
    /// Conflicts awaiting the user's answer, so the resolve command can act on
    /// one by name rather than the UI having to hand the whole record back.
    pub pending_conflicts: Mutex<Vec<crate::savesync::SaveConflict>>,
    /// The rapid-fire rate for this run of the app, when it has been nudged.
    ///
    /// The +/- beside the control wrote config.toml on every press: five taps
    /// to go from six to eleven is five rewrites of the file, and a file
    /// rewritten that often is one that eventually gets caught half-written.
    /// The number in config.toml is the one you start with; moving it here is
    /// for the run you are about to have.
    pub autofire_hz: Mutex<Option<u32>>,
    /// Keys and buttons, resolved by `crate::binds` rather than by the
    /// page. `config.toml` stays the source of truth; this is the live copy,
    /// so a rebind takes effect on the next press rather than the next
    /// restart.
    pub bindings: Mutex<crate::binds::Bindings>,
    /// How the left column is ordered, per kind of list.
    pub picker_order: Mutex<crate::pickorder::PickerOrders>,
    /// The order and filters chosen per view, for this run only. See
    /// `crate::gamelist::Chosen` for why this one is memory and
    /// `picker_order` is a file.
    pub chosen: Mutex<crate::gamelist::Chosen>,
    /// The list last handed to a front end.
    ///
    /// Kept so `arrange_list` can answer "which of these, and in what order"
    /// without the whole list travelling back here: 2,506 rows leave for the
    /// arcade console, and every change of sort or filter would otherwise
    /// return all of them to be handed straight back.
    pub list_rows: Mutex<Vec<crate::gamelist::Row>>,
    /// Which view `list_rows` was filled for, so an arrangement asked for a
    /// different one is refused rather than answered with the wrong list.
    pub list_scope: Mutex<String>,
    /// The names on the page and the groups they sit under, for the filter box
    /// above them. Sent once per list rather than once per keystroke: there are
    /// 2,506 of them on the arcade console.
    pub page_names: Mutex<(Vec<String>, Vec<Vec<usize>>)>,
}

impl AppState {
    /// Build the state from `config.toml`, with no window involved.
    ///
    /// This was inline in the Tauri builder, so starting the backend meant
    /// starting a GUI. As a plain constructor the HTTP service builds exactly
    /// the state the desktop app does rather than an approximation of it.
    pub fn from_config() -> anyhow::Result<Self> {
    let cfg = Config::load().unwrap_or_default();
    let store = crate::cache::Cache::open(Path::new(crate::commands::CACHE_DB)).expect("opening metadata cache");
    // Archive verification depends on the server's exclusion lists; load the
    // cached copy before anything can download.
    crate::apply_cached_server_config(&store);
    // Never `expect` here. A release build has no console (see
    // `windows_subsystem` in `main.rs`), so a panic at startup is
    // completely silent: the icon bounces and nothing happens, with no way to
    // tell whether the app crashed or never ran.
    let map = CoreMap::load_or_embedded(Path::new(crate::commands::CORE_MAP));
    let client = cfg.server.client()
        .ok()
        .map(Arc::new);
    // A first run on Android has to end with a config, not with advice.
    //
    // Everywhere else the release notes say "copy config.example.toml to
    // config.toml" and that works. On Android the data directory is private:
    // the user cannot put a file in it, and the app showed a tab bar over an
    // empty screen with a message naming a file nobody could create. Measured
    // on a fresh Retroid Pocket Mini V2.
    //
    // Only when nothing is there. An existing config is the user's.
    if cfg!(target_os = "android")
        && crate::config_files::seed_config(Path::new("config.toml"))
    {
        eprintln!("no config.toml — wrote the documented template");
    }

    // On Android RetroArch is another app, so there is no path to search — it
    // is found by its config instead. Everything downstream of `state.retroarch`
    // depends on this: without it the shader list in Settings → Emulators was
    // empty and the tab said "no RetroArch" on a device that has it installed.
    let retroarch = if cfg!(target_os = "android") {
        RetroArch::locate_android()
    } else {
        RetroArch::locate_in(&cfg.retroarch.ordered_paths()).ok()
    }
    .map(|ra| ra.with_system_dir(Some(cfg.system_dir())));
    let roms_dir = cfg.local_roms_dir();
    let media_dir = PathBuf::from(&cfg.library.local_root).join("downloaded_media");

    // Artwork now comes from ES-DE alone. Anything fetched from RomM before
    // that goes, once, or the art chain would keep finding it and only the
    // games nobody had browsed yet would look consistent.
    match crate::media::drop_server_covers(&media_dir) {
        0 => {}
        n => eprintln!("cleared {n} cover(s) fetched from RomM; artwork now comes from ES-DE"),
    }

    // Icon sets fetched under a superseded art mapping, for the same reason:
    // the pictures are on disk in folders the current table does not use, so
    // the grid keeps drawing a controller where it says "Hardware".
    let fingerprints: std::collections::BTreeMap<String, String> =
        crate::iconart::table()
            .into_iter()
            .map(|(name, art)| (name, art.fingerprint()))
            .collect();
    for set in crate::theme::drop_stale_sets(&media_dir, &fingerprints) {
        eprintln!("re-fetch needed for icon set {set}: its pictures predate a corrected mapping");
    }

        Ok(AppState {
        cache: Mutex::new(store),
        map,
        client,
        retroarch,
        roms_dir,
        media_dir,
        esde_media: cfg.esde.media_dir(),
        theme_root: cfg.theme.root.clone(),
        themes_dir: cfg.themes_dir(),
        mirror_players: cfg.controllers.mirror_player_one,
        fit_window: cfg.retroarch.fit_window,
        window_decorations: cfg.retroarch.window_decorations,
        core_overrides: Mutex::new(cfg.cores.overrides.clone()),
        core_per_game: Mutex::new(cfg.cores.per_game.clone()),
        user_retroarch_cfg: cfg.user_retroarch_config(),
        shaders_enabled: cfg.shaders.enabled,
        shader_overrides: Mutex::new(cfg.shaders.by_platform.clone()),
        lightgun: Mutex::new(cfg.lightgun.by_platform.clone()),
        list_art: Mutex::new(cfg.media.list_art.clone()),
        detail_art: cfg.media.detail_art.clone(),
        motion_shader: Mutex::new(cfg.shaders.motion.clone()),
        // From config, not hardcoded: index 0 is `logo`, which is ES-DE's
        // wordmark art — a picture of the system's name. The grid wants
        // hardware, and the user's choice has to survive a restart.
        icon_look: Mutex::new(cfg.icons.style.clone()),
        icon_set: Mutex::new(cfg.icons.set.clone()),
        achievements: cfg.achievements.settings(),
        auto_sync: cfg.saves.auto_sync,
        pending_conflicts: Mutex::new(Vec::new()),
        autofire_hz: Mutex::new(None),
        bindings: Mutex::new(cfg.bindings.clone()),
        picker_order: Mutex::new(cfg.picker_order.clone()),
        chosen: Mutex::new(Default::default()),
        list_rows: Mutex::new(Vec::new()),
        list_scope: Mutex::new(String::new()),
        page_names: Mutex::new((Vec::new(), Vec::new())),
        })
    }
}

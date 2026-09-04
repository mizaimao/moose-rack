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
    /// Rescan an ES-DE tree into the metadata cache the UI reads.
    ///
    /// Three calls, and the CLI's `scan-esde` is the same three -- the rest of
    /// that command is printing. Shared because the fold at the end is easy to
    /// leave out and expensive to leave out: without `absorb_local_into_server`
    /// the scan sits beside whatever a server sync already found and every game
    /// both know about appears twice. That is 11,062 rows against 11,473, in a
    /// library of about 11,500.
    ///
    /// Returns the rows written and the rows folded.
    pub fn rescan(&self, layout: &crate::esde::Layout) -> anyhow::Result<(usize, usize)> {
        let mut store = self.cache.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let scan = scan_into(&mut store, layout, &self.map)?;
        Ok((scan.written, scan.folded))
    }

    /// Point the state at a library somewhere other than the config's.
    ///
    /// The service already knows where the library is -- it was told on the
    /// command line or in `moose-service.toml`, and it has scanned it. Without
    /// this the web UI would need a second config file repeating those paths,
    /// and the two would disagree the first time one was edited.
    ///
    /// `media_dir` is left alone: it is where the app *writes* -- art indexes,
    /// fetched icon sets -- and the ES-DE tree is not necessarily writable.
    pub fn point_at(&mut self, layout: &crate::esde::Layout) {
        self.roms_dir = layout.roms.clone();
        self.esde_media = Some(layout.media.clone());
    }

    pub fn from_config() -> anyhow::Result<Self> {
        Self::from_config_at(crate::config::path())
    }

    /// The same, from a named file.
    ///
    /// `Config::load` reads `config.toml` out of the working directory, which
    /// is fine for an app launched by its icon and wrong for a service whose
    /// working directory is whatever systemd set. The service passes the path
    /// it was given.
    pub fn from_config_at(path: &Path) -> anyhow::Result<Self> {
    let cfg = Config::load_from(path).unwrap_or_default();
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
    if cfg!(target_os = "android") && crate::config_files::seed_config(path) {
        eprintln!("no {} — wrote the documented template", path.display());
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

/// What a scan put into the cache, and what it found on the way.
pub struct Scan {
    pub written: usize,
    pub folded: usize,
    pub games: Vec<crate::esde::Game>,
    /// Systems with no platform mapping, so nothing from them was stored.
    pub skipped: Vec<String>,
}

/// Read an ES-DE tree into the metadata cache the UI reads.
///
/// The CLI's `scan-esde` is these three calls and a lot of printing; the
/// service is these three calls and none. Shared because the fold at the end is
/// easy to leave out and expensive to leave out: without
/// `absorb_local_into_server` the scan sits beside whatever a server sync
/// already found and every game both know about appears twice. That was 11,062
/// rows against 11,473, in a library of about 11,500.
pub fn scan_into(
    store: &mut cache::Cache,
    layout: &crate::esde::Layout,
    map: &CoreMap,
) -> anyhow::Result<Scan> {
    let (games, skipped) = crate::esde::scan(layout, map)?;
    let written = store.replace_from_esde(&games)?;
    // The scan knows a directory, not a console. Without this the grid reads
    // "snes snes 876 games", because the only thing that ever filled the
    // display name in was a server sync -- and a library served from an ES-DE
    // tree has no server to sync from.
    let names: Vec<(String, String)> = games
        .iter()
        .map(|g| g.platform_slug.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|slug| map.display_name(slug).map(|n| (slug.to_owned(), n.to_owned())))
        .collect();
    store.name_platforms(&names)?;
    let folded = store.absorb_local_into_server()?;
    Ok(Scan { written, folded, games, skipped })
}

#[cfg(test)]
mod scan_tests {
    use super::*;

    fn tree(name: &str) -> (PathBuf, crate::esde::Layout) {
        let root = std::env::temp_dir().join(format!("moose-rack-scan-{name}"));
        std::fs::remove_dir_all(&root).ok();
        let roms = root.join("ROMs");
        for f in ["snes/Chrono Trigger (USA).sfc", "snes/Super Metroid (USA).sfc"] {
            let p = roms.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        let layout = crate::esde::Layout::new(&root, Some(&roms));
        (root, layout)
    }

    fn map() -> CoreMap {
        serde_json::from_str(
            r#"{"default_core_by_server_platform": {"snes": "snes9x"},
                "systems": {"snes": {"server_platforms": ["snes"], "fullname": "Super Nintendo",
                            "extensions": [".sfc"], "emulators": []}}}"#,
        )
        .unwrap()
    }

    /// The grid said "snes snes 876 games".
    ///
    /// A scan knows the directory and nothing else, so it wrote `snes` as the
    /// display name too. On the desktop a server sync overwrote that with a
    /// real name and hid it; on a library served from an ES-DE tree there is no
    /// sync, and every console on the page was a lowercase directory.
    #[test]
    fn consoles_get_their_real_names_from_the_core_map() {
        let (root, layout) = tree("names");
        let mut store = cache::Cache::open(&root.join("cache.sqlite3")).unwrap();
        scan_into(&mut store, &layout, &map()).unwrap();
        let shown: Vec<String> =
            store.platforms().unwrap().into_iter().map(|p| p.display_name).collect();
        assert_eq!(shown, ["Super Nintendo"], "the console is still named after its folder");
        std::fs::remove_dir_all(&root).ok();
    }

    /// A name a sync supplied is not replaced by the table's fallback.
    #[test]
    fn a_name_already_set_is_left_alone() {
        let (root, layout) = tree("keepname");
        let mut store = cache::Cache::open(&root.join("cache.sqlite3")).unwrap();
        scan_into(&mut store, &layout, &map()).unwrap();
        store.name_platforms(&[("snes".into(), "Something Else".into())]).unwrap();
        let shown: Vec<String> =
            store.platforms().unwrap().into_iter().map(|p| p.display_name).collect();
        assert_eq!(shown, ["Super Nintendo"], "the fallback overwrote a name that was already set");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_scan_lands_in_the_cache_the_ui_reads() {
        let (root, layout) = tree("basic");
        let mut store = cache::Cache::open(&root.join("cache.sqlite3")).unwrap();
        let scan = scan_into(&mut store, &layout, &map()).unwrap();
        assert_eq!(scan.written, 2, "both games should be stored");
        assert_eq!(scan.games.len(), 2);
        assert_eq!(store.rom_count().unwrap(), 2);
        std::fs::remove_dir_all(&root).ok();
    }

    /// Twice is still two games. `replace_from_esde` replaces; a scan that
    /// appended would double the grid on every service restart, which is where
    /// this would show up now that the service rescans at startup.
    #[test]
    fn scanning_twice_does_not_double_the_library() {
        let (root, layout) = tree("twice");
        let mut store = cache::Cache::open(&root.join("cache.sqlite3")).unwrap();
        scan_into(&mut store, &layout, &map()).unwrap();
        scan_into(&mut store, &layout, &map()).unwrap();
        assert_eq!(store.rom_count().unwrap(), 2);
        std::fs::remove_dir_all(&root).ok();
    }

    /// Nobody may write the scan out longhand again.
    ///
    /// `scan-esde` did, and left out `absorb_local_into_server`, so the CLI's
    /// scan sat beside the rows a server sync had already stored: 11,062 and
    /// 11,473 in a library of about 11,500, every shared game drawn twice. The
    /// fold is one line and its absence is invisible until you count the grid.
    #[test]
    fn both_callers_go_through_scan_into() {
        for (name, src) in [
            ("src/main.rs", include_str!("main.rs")),
            ("src-service", include_str!("../src-service/src/main.rs")),
        ] {
            assert!(
                !src.contains("replace_from_esde("),
                "{name} scans by hand; it must call app::scan_into so the fold happens"
            );
        }
    }
}

//! Tauri GUI shell.
//!
//! Deliberately thin: every command here delegates to `moose_rack`, the same
//! crate the CLI and TUI use. If logic starts accumulating in this file it
//! belongs in the core crate instead.
//!
//! A library rather than a binary, because Android has no `main`. The APK is
//! Java that loads a `.so` and calls into it, so the entry point below is
//! `pub fn run` and `main.rs` is four lines that call it. Desktop is unchanged
//! by this — it still starts at `main` — but the shape is Android's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

mod iconsets;

use moose_rack::{
    api, cache, config::Config, coremap::{self, CoreMap}, download, media, retroarch::RetroArch,
    savesync, shaders, syncplan, theme, theme_remote, util,
};

const CACHE_DB: &str = "cache.sqlite3";
const CORE_MAP: &str = "data/esde-core-map.json";

/// Long-lived process state. The SQLite connection is not `Sync`, so it lives
/// behind a mutex and is only held for the duration of a query.
struct AppState {
    cache: Mutex<cache::Cache>,
    map: CoreMap,
    client: Option<Arc<api::Client>>,
    retroarch: Option<RetroArch>,
    roms_dir: PathBuf,
    media_dir: PathBuf,
    /// Artwork of a locally scanned ES-DE library. Keyed by ES-DE *system*
    /// name rather than RomM slug, so it needs its own lookup rather than
    /// being folded into `media_dir`.
    esde_media: Option<PathBuf>,
    theme_root: Option<String>,
    themes_dir: PathBuf,
    /// Bind players 2-4 like player 1. See config::ControllersCfg.
    mirror_players: bool,
    /// Shape the game window like the game, so it has no black bars.
    fit_window: bool,
    /// Keep the game window's title bar.
    window_decorations: bool,
    /// Behind mutexes so a choice made in the UI takes effect on the next
    /// launch rather than the next restart. `config.toml` stays the source of
    /// truth; these are the live copy.
    core_overrides: Mutex<std::collections::BTreeMap<String, String>>,
    core_per_game: Mutex<std::collections::BTreeMap<String, String>>,
    user_retroarch_cfg: PathBuf,
    shaders_enabled: bool,
    shader_overrides: Mutex<std::collections::BTreeMap<String, String>>,
    /// Systems switched over to a light gun, so gun games aim with the mouse.
    lightgun: Mutex<std::collections::BTreeMap<String, String>>,
    /// Which ES-DE artwork the list and the info pane draw.
    list_art: Mutex<String>,
    detail_art: String,
    /// Strobe/BFI pass chained onto CRT shaders, if the user enabled one.
    motion_shader: Mutex<Option<String>>,
    /// The look the grid draws — an id from the chosen set's own list, not one
    /// of three fixed kinds. Themes offer between one and nine.
    icon_look: Mutex<String>,
    /// A downloaded ES-DE icon set the grid draws from, or empty for the
    /// shared pool. Orthogonal to `icon_style`: whose art, versus which kind.
    icon_set: Mutex<String>,
    /// RetroAchievements, read once at startup from this project's config.toml
    /// — see `moose_rack::achievements`.
    achievements: moose_rack::achievements::Settings,
    /// Pull before a launch and push after it exits.
    auto_sync: bool,
    /// Conflicts awaiting the user's answer, so the resolve command can act on
    /// one by name rather than the UI having to hand the whole record back.
    pending_conflicts: Mutex<Vec<moose_rack::savesync::SaveConflict>>,
    /// The rapid-fire rate for this run of the app, when it has been nudged.
    ///
    /// The +/- beside the control wrote config.toml on every press: five taps
    /// to go from six to eleven is five rewrites of the file, and a file
    /// rewritten that often is one that eventually gets caught half-written.
    /// The number in config.toml is the one you start with; moving it here is
    /// for the run you are about to have.
    autofire_hz: Mutex<Option<u32>>,
    /// Keys and buttons, resolved by `moose_rack::binds` rather than by the
    /// page. `config.toml` stays the source of truth; this is the live copy,
    /// so a rebind takes effect on the next press rather than the next
    /// restart.
    bindings: Mutex<moose_rack::binds::Bindings>,
    /// How the left column is ordered, per kind of list.
    picker_order: Mutex<moose_rack::pickorder::PickerOrders>,
    /// The order and filters chosen per view, for this run only. See
    /// `moose_rack::gamelist::Chosen` for why this one is memory and
    /// `picker_order` is a file.
    chosen: Mutex<moose_rack::gamelist::Chosen>,
    /// The list last handed to a front end.
    ///
    /// Kept so `arrange_list` can answer "which of these, and in what order"
    /// without the whole list travelling back here: 2,506 rows leave for the
    /// arcade console, and every change of sort or filter would otherwise
    /// return all of them to be handed straight back.
    list_rows: Mutex<Vec<moose_rack::gamelist::Row>>,
    /// Which view `list_rows` was filled for, so an arrangement asked for a
    /// different one is refused rather than answered with the wrong list.
    list_scope: Mutex<String>,
    /// The names on the page and the groups they sit under, for the filter box
    /// above them. Sent once per list rather than once per keystroke: there are
    /// 2,506 of them on the arcade console.
    page_names: Mutex<(Vec<String>, Vec<Vec<usize>>)>,
}

#[derive(Serialize)]
struct PlatformView {
    slug: String,
    name: String,
    rom_count: i64,
    /// Whether a libretro core for this platform is actually installed.
    playable: bool,
    /// ES-DE theme art, if any has been installed locally.
    logo: Option<String>,
    /// True only for the `logo` style, whose art is a white-on-transparent
    /// wordmark and therefore needs inverting on a light page. Hardware and
    /// console art is full color and must not be touched.
    logo_wordmark: bool,
    /// A fixed picture of the machine for the info pane; see `portrait` above.
    portrait: Option<String>,
    /// Typical cover aspect (w/h) for this platform, so the grid can shape its
    /// cards instead of cropping. Null until enough covers are cached.
    cover_aspect: Option<f32>,
    /// What the machine was: maker, year, kind, and a line about it. The same
    /// four things ES-DE's themes show when you pick a system. Null for a
    /// platform we have nothing to say about, so the pane can leave it out
    /// rather than print blanks.
    manufacturer: Option<&'static str>,
    released: Option<u16>,
    hardware: Option<&'static str>,
    blurb: Option<&'static str>,
}

#[derive(Serialize)]
struct RomView {
    id: i64,
    name: String,
    fs_name: String,
    platform: String,
    size_bytes: i64,
    downloaded: bool,
    /// In a starred collection. Shown with a star and sorted to the top.
    favorite: bool,
    /// The three things a list can be ordered by that are not the name. Pulled
    /// out of the metadata blob here rather than in the page, because the page
    /// would have to parse the same JSON once per game on every redraw.
    rating: Option<f64>,
    year: Option<i32>,
    /// ISO timestamp, comparable as text. See util::iso_from_epoch.
    last_played: Option<String>,
    /// The most players the game supports, or `None` when nothing says.
    ///
    /// Parsed here for the same reason as `year` and `rating`: the field is
    /// free text inside the metadata blob, and the page would otherwise parse
    /// the same JSON once per game on every redraw.
    players: Option<u8>,
    /// Subfolder inside the ES-DE system directory, `""` at the top level.
    ///
    /// The page builds the folder view out of this. It arrives with the rest of
    /// the list, once, so walking in and out of a folder is a redraw rather
    /// than a round trip.
    rel_dir: String,
}

#[derive(Serialize)]
struct RomDetail {
    id: i64,
    name: String,
    fs_name: String,
    platform: String,
    /// The slug as well as the display name. The row of recent games holds
    /// games from several consoles, so anything acting on "this game's
    /// console" cannot read it off the page it is on.
    platform_slug: String,
    /// Auto-fire for this game: "off", "a" or "y" when the game is one that
    /// can have it, and absent when it is not.
    ///
    /// Absent and "off" are different answers and the pane needs both: absent
    /// means there is nothing to offer, "off" means there is and you have
    /// turned it down — which is the difference between showing no control and
    /// showing one with nothing selected.
    autofire: Option<String>,
    /// Shots a second, shown beside the three modes.
    autofire_hz: u32,
    size_bytes: i64,
    downloaded: bool,
    core: Option<String>,
    core_label: Option<String>,
    /// Local media, as `asset:` URLs the webview can load directly.
    cover: Option<String>,
    /// Present only when the video is already on this machine. The pane never
    /// downloads one: see `has_video`.
    video: Option<String>,
    /// Whether a gameplay video exists at all, local or on the server.
    ///
    /// Separate from `video` because the pane shows an indicator, not a
    /// player. A video is tens of megabytes against tens of kilobytes for
    /// every other kind of media a game has, and fetching one to find out
    /// whether it existed happened for every game the cursor touched.
    has_video: bool,
    /// Every screenshot we could resolve; the UI cycles through them.
    screenshots: Vec<String>,
    /// ES-DE artwork by type — 3dboxes, miximages, marquees, fanart and the
    /// rest. Far richer than RomM's own cover + one screenshot.
    art: std::collections::BTreeMap<String, String>,

    // Descriptive metadata, straight from RomM (which on this server got it
    // from the ES-DE gamelist import).
    summary: Option<String>,
    genres: Vec<String>,
    companies: Vec<String>,
    franchises: Vec<String>,
    game_modes: Vec<String>,
    player_count: Option<String>,
    /// 0-100 as RomM stores it.
    rating: Option<f64>,
    release_year: Option<i32>,
    alt_names: Vec<String>,
    regions: Vec<String>,
    manual: Option<String>,
    youtube_id: Option<String>,
}

/// Pull the library from the server into the local cache.
///
/// The GUI had no way to do this. The Windows release ships only the GUI, so a
/// fresh install there had an empty cache, nothing on screen, and no command to
/// fill it — which reads as "it cannot connect to the server" even though the
/// server is perfectly reachable. On macOS the cache was always populated by
/// the CLI beforehand, which is why it never showed up there.
///
/// Opens its own cache connection rather than using the shared one: `sync`
/// needs `&mut Cache` and awaits, and a `MutexGuard` cannot be held across an
/// await. Two SQLite connections to one file is fine, and the shared one sees
/// the committed rows immediately afterwards.
#[tauri::command]
async fn sync_library(app: tauri::AppHandle, state: State<'_, AppState>, full: bool) -> CmdResult<String> {
    let mut store = cache::Cache::open(Path::new(CACHE_DB)).map_err(err)?;

    // Pass one: this machine.
    //
    // Before the server and regardless of it. A library that exists on disk is
    // a library, and it used to be unreachable from here — this command began
    // by demanding a server and gave up if there was not one, so an install
    // with a full ES-DE folder and no RomM account showed an empty grid and
    // said only "no server configured".
    //
    // Failure here is reported and stepped over rather than returned. A missing
    // ROMs folder is the ordinary state of a fresh install, and it must not
    // stop the server pass that would have filled it.
    let _ = app.emit("sync-progress", "looking for games on this device…");
    let cfg = Config::load().unwrap_or_default();
    let layout = cfg.esde_layout();
    let local = match moose_rack::esde::scan(&layout, &state.map) {
        Ok((games, _skipped)) => store.replace_from_esde(&games).unwrap_or(0),
        Err(_) => 0,
    };

    // Pass two: the server, if there is one. Everything below is optional.
    let Some(client) = state.client.clone() else {
        // Nothing to fold — absorb only ever matches against server rows.
        let total = store.rom_count().unwrap_or(0);
        let _ = app.emit("sync-progress", "done");
        return Ok(if total == 0 {
            format!(
                "No games found under {} and no server configured.",
                layout.roms.display()
            )
        } else {
            format!("{total} games found on this device. No server configured.")
        });
    };

    let _ = app.emit("sync-progress", "checking the server…");
    // Refresh the settings that govern hashing before anything downloads.
    if let Ok(cfg) = client.config().await {
        store.save_server_config(&cfg).ok();
    }

    let _ = app.emit("sync-progress", "fetching the library…");
    let (platforms, upserted, incremental) = store.sync(&client, full).await.map_err(err)?;

    // Removals never appear in an incremental pull.
    let mut pruned = 0;
    if let Ok(ids) = client.rom_identifiers().await {
        pruned = store.prune_missing(&ids).unwrap_or(0);
    }

    let _ = app.emit("sync-progress", "collections…");
    let collections = match client.all_collections().await {
        Ok(items) => store.replace_collections(&items).unwrap_or(0),
        Err(_) => 0,
    };

    // Console pictures for the platform grid. Cheap — a few KB of vector art
    // each, only for platforms not already cached — and it means the grid has
    // real artwork without anyone downloading a theme for it.
    let _ = app.emit("sync-progress", "console pictures…");
    let icons = match client.platforms().await {
        Ok(list) => {
            let pairs: Vec<(String, String)> =
                list.iter().map(|p| (p.slug.clone(), p.fs_slug.clone())).collect();
            moose_rack::platformicon::ensure(&client, &state.media_dir, &pairs)
                .await
                .unwrap_or(0)
        }
        Err(_) => 0,
    };

    // A sync rewrites names from the server, so the real arcade titles go back
    // afterwards rather than before.
    let names = moose_rack::arcade::names(Path::new("data/arcade-names.json"));
    let renamed = store.apply_arcade_names(&names).unwrap_or(0);

    // Both passes are in now, so the games that are in both stop being two.
    // Matched on platform and file name — see `absorb_local_into_server`.
    let folded = store.absorb_local_into_server().unwrap_or(0);

    let total = store.rom_count().unwrap_or(0);
    let _ = app.emit("sync-progress", "done");
    Ok(format!(
        "{} sync: {total} games across {platforms} platforms ({upserted} updated{}{}{}{})",
        if incremental { "Incremental" } else { "Full" },
        if pruned > 0 { format!(", {pruned} removed") } else { String::new() },
        if collections > 0 { format!(", {collections} collections") } else { String::new() },
        if renamed > 0 { format!(", {renamed} arcade titles") } else { String::new() },
        // Said out loud because it is the number that explains why a local
        // scan of 900 games and a server of 900 games is not 1,800.
        if local > 0 {
            format!(", {local} found on this device of which {folded} matched the server")
        } else {
            String::new()
        },
    ) + &if icons > 0 { format!(", {icons} console pictures") } else { String::new() })
}

/// Open the Settings window, or focus it if it is already up.
///
/// A real second window rather than an overlay: settings are a place you go and
/// come back from, and the old panel put both binding tables — the longest
/// thing in the app — on top of the library you were trying to use. This one is
/// resizable and remembers nothing about the grid behind it.
#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> CmdResult<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    // Focus rather than open a second copy. Two settings windows would each
    // hold their own binding state and the last one saved would win.
    if let Some(existing) = app.get_webview_window("settings") {
        existing.set_focus().map_err(err)?;
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Settings")
        // Wider than it was: the bindings table is three columns now, and at
        // 1000 the action names wrapped to two lines each.
        .inner_size(1180.0, 780.0)
        // Below this the tab rail and a binding row stop fitting side by side.
        .min_inner_size(640.0, 460.0)
        .resizable(true)
        .build()
        .map_err(err)?;
    Ok(())
}

/// The config.toml values Settings can show and edit.
///
/// Deliberately a fixed list rather than "whatever is in the file". Not all of
/// config.toml is a setting — `cores.per_game` belongs in the game detail pane
/// and `[scraper]` is read by nothing. A field here means somebody decided it
/// belongs on screen.
#[derive(Serialize)]
struct ConfigFields {
    library_root: String,
    /// The ES-DE data directory — the one holding `gamelists/` and
    /// `downloaded_media/` — and the ROMs folder when it is somewhere else,
    /// which it usually is.
    ///
    /// On desktop these are typed once and forgotten. On Android they are the
    /// whole arrangement: ES-DE is a real app on the device with real folders,
    /// and pointing at them is what makes this app see the same library rather
    /// than a second copy hidden in its own private storage.
    esde_root: String,
    esde_roms: String,
    /// Where RetroArch is told to put battery saves and save states. Written
    /// into the per-launch config, so the folder chosen here is the folder the
    /// emulator actually uses.
    saves_root: String,
    server_url: String,
    server_username: String,
    /// Present or not, never the value. A settings pane has no reason to hand a
    /// credential back to the webview to display; the box shows "set" and lets
    /// you replace it.
    server_token_set: bool,
    achievements_enabled: bool,
    achievements_username: String,
    achievements_token_set: bool,
    achievements_hardcore: bool,
    shaders_enabled: bool,
    confirm_delete_state: bool,
    mirror_player_one: bool,
    /// Read the two face-button pairs the other way round. See
    /// `binds::pad_map_swapped`.
    swap_ab: bool,
    swap_xy: bool,
    fit_window: bool,
    window_decorations: bool,
    autofire: String,
    save_state_on_exit: bool,
    /// Present so the UI can say where it is writing, and warn when there is
    /// nothing to write to.
    config_path: String,
    config_exists: bool,
}

#[tauri::command]
fn config_fields() -> CmdResult<ConfigFields> {
    let cfg = Config::load().unwrap_or_default();
    Ok(ConfigFields {
        library_root: cfg.library.local_root.clone(),
        esde_root: cfg.esde.root.clone().unwrap_or_default(),
        esde_roms: cfg.esde.roms.clone().unwrap_or_default(),
        saves_root: cfg.saves.root.clone(),
        server_url: cfg.server.url.clone(),
        server_username: cfg.server.username.clone(),
        server_token_set: cfg.server.token.as_deref().is_some_and(|t| !t.trim().is_empty()),
        achievements_enabled: cfg.achievements.enabled,
        achievements_username: cfg.achievements.username.clone().unwrap_or_default(),
        achievements_token_set: cfg
            .achievements
            .token
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty()),
        achievements_hardcore: cfg.achievements.hardcore,
        shaders_enabled: cfg.shaders.enabled,
        confirm_delete_state: cfg.saves.confirm_delete_state,
        mirror_player_one: cfg.controllers.mirror_player_one,
        swap_ab: cfg.controllers.swap_ab,
        swap_xy: cfg.controllers.swap_xy,
        fit_window: cfg.retroarch.fit_window,
        window_decorations: cfg.retroarch.window_decorations,
        autofire: cfg.retroarch.autofire.clone(),
        save_state_on_exit: cfg.retroarch.save_state_on_exit,
        config_path: abs(Path::new("config.toml")),
        config_exists: Config::exists("config.toml"),
    })
}

/// Write one value into config.toml, or remove it when empty.
///
/// Goes through the same targeted TOML edit the per-system settings use, so the
/// hand-written comments explaining non-obvious choices survive — round-tripping
/// through a serialiser would delete every one of them.
///
/// Only the fields `config_fields` exposes are accepted. An allowlist rather
/// than passing a section and key straight through: this is called from a
/// webview, and "write any key into any table" is a larger door than this needs.
#[tauri::command]
fn set_config_field(field: String, value: String) -> CmdResult<String> {
    let (table, key) = match field.as_str() {
        "library_root" => ("library", "local_root"),
        // These two need more than a restart, and the message below says so:
        // the artwork root is read once at startup, and the ES-DE system name
        // that decides *which* artwork folder a game looks in is written only
        // by the local scan. Until both have happened a game falls back to
        // this app's own downloads.
        "esde_root" => ("esde", "root"),
        "esde_roms" => ("esde", "roms"),
        "saves_root" => ("saves", "root"),
        "server_url" => ("server", "url"),
        "server_token" => ("server", "token"),
        "server_username" => ("server", "username"),
        "scraper_ssid" => ("scraper", "ssid"),
        "scraper_sspassword" => ("scraper", "sspassword"),
        "achievements_enabled" => ("achievements", "enabled"),
        "achievements_username" => ("achievements", "username"),
        "achievements_token" => ("achievements", "token"),
        "achievements_hardcore" => ("achievements", "hardcore"),
        "shaders_enabled" => ("shaders", "enabled"),
        "confirm_delete_state" => ("saves", "confirm_delete_state"),
        "mirror_player_one" => ("controllers", "mirror_player_one"),
        "swap_ab" => ("controllers", "swap_ab"),
        "swap_xy" => ("controllers", "swap_xy"),
        "game_display" => ("retroarch", "game_display"),
        "fit_window" => ("retroarch", "fit_window"),
        "window_decorations" => ("retroarch", "window_decorations"),
        "autofire" => ("retroarch", "autofire"),
        "save_state_on_exit" => ("retroarch", "save_state_on_exit"),
        "autofire_hz" => ("retroarch", "autofire_hz"),
        other => return Err(format!("unknown setting {other}")),
    };

    // Booleans are TOML literals, not strings, so they cannot go through the
    // quoted-string writer.
    // Numbers, like booleans, are bare TOML literals: a quoted "5" is a string
    // and fails to deserialise into a number.
    if field == "autofire_hz" {
        let n: i64 = value.trim().parse().map_err(|_| format!("{value} is not a number"))?;
        moose_rack::config::set_table_number("config.toml", table, key, n.clamp(1, 30))
            .map_err(err)?;
        return Ok(format!("{n} shots a second"));
    }

    // Every field whose struct type is `bool`. Miss one and it is written as
    // `swap_ab = "true"` — a string where the struct wants a bool — and then
    // `toml::from_str` refuses *the whole file*. Since every caller of
    // `Config::load` does `unwrap_or_default()`, one quoted bool silently
    // reverts the entire app to its defaults: on the Thor it moved the library,
    // the artwork and the saves folder back to `./library/...` and the ES-DE
    // scan stopped finding anything, which reads as a stale cache rather than
    // an unreadable config.
    //
    // `autofire` and `game_display` are absent on purpose — those are strings
    // in the struct too.
    let boolean = matches!(
        field.as_str(),
        "achievements_enabled"
            | "achievements_hardcore"
            | "shaders_enabled"
            | "confirm_delete_state"
            | "mirror_player_one"
            | "swap_ab"
            | "swap_xy"
            | "fit_window"
            | "save_state_on_exit"
            | "window_decorations"
    );

    if value.trim().is_empty() && !boolean {
        moose_rack::config::clear_table_entry("config.toml", table, key).map_err(err)?;
        return Ok(format!("{key} cleared"));
    }
    if boolean {
        moose_rack::config::set_table_bool(
            "config.toml",
            table,
            key,
            value == "true" || value == "1",
        )
        .map_err(err)?;
    } else {
        moose_rack::config::set_table_entry("config.toml", table, key, value.trim())
            .map_err(err)?;
    }
    Ok(format!("{key} saved — restart to apply"))
}

/// Try the credentials without saving them.
///
/// The point of a Verify button is to answer "will this work" *before* it is
/// committed, so this builds a throwaway client from whatever is in the box
/// rather than from the running config.
#[tauri::command]
async fn verify_server(
    url: String,
    token: Option<String>,
    username: Option<String>,
    password: Option<String>,
) -> CmdResult<String> {
    let client = api::Client::with_auth(
        url.trim(),
        username.as_deref().unwrap_or_default(),
        password.as_deref().unwrap_or_default(),
        token.as_deref(),
    )
    .map_err(err)?;

    // Heartbeat first: it needs no credentials, so a failure here is the server
    // or the network rather than the token, and saying which saves a lot of
    // guessing.
    let version = match client.heartbeat().await {
        Ok(hb) => hb.system.version,
        Err(e) => {
            return Err(format!(
                "cannot reach {} — {}",
                url.trim(),
                e.to_string().lines().next().unwrap_or("no answer")
            ));
        }
    };

    // Then something that does need them.
    match client.me().await {
        Ok(me) => {
            let count = client.rom_count().await.unwrap_or(0);
            Ok(format!(
                "Connected to RomM {version} as {} — {count} games",
                me.username
            ))
        }
        Err(e) => Err(format!(
            "reached the server (RomM {version}) but the credentials were refused — {}",
            e.to_string().lines().next().unwrap_or("")
        )),
    }
}

/// Download the BIOS set from the server.
///
/// Optional and explicit: it is 300-odd MB on this server, and a launcher that
/// pulls that on first run without asking is a launcher people uninstall.
/// What a BIOS sync would do — total files, how many are here, bytes to fetch.
///
/// Its own command so the button can answer immediately instead of sitting
/// silent through a listing, which is indistinguishable from a control that
/// does nothing.
#[tauri::command]
async fn bios_status(state: State<'_, AppState>) -> CmdResult<(usize, usize, u64)> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
    moose_rack::bios::status(&client, &library_root).await.map_err(err)
}

#[tauri::command]
async fn sync_bios(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();

    let summary = moose_rack::bios::sync(&client, &library_root, |done, total, name| {
        let _ = app.emit("bios-progress", (done, total, name.to_owned()));
    })
    .await
    .map_err(err)?;

    let mut out = summary.headline();
    if !summary.notes.is_empty() {
        out.push('\n');
        out.push_str(&summary.notes.iter().take(6).cloned().collect::<Vec<_>>().join("\n"));
    }
    Ok(out)
}

/// Pull the useful bits out of RomM's merged `metadatum` blob.
fn meta_strings(meta: &Option<serde_json::Value>, key: &str) -> Vec<String> {
    meta.as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// This build's version, and the server's if we have ever spoken to it.
///
/// Both, because "which version am I on" is nearly always asked when something
/// behaves differently on two machines, and the answer is as often the server
/// as the client.
#[tauri::command]
fn versions(state: State<'_, AppState>) -> CmdResult<(String, Option<String>)> {
    let server = state.cache.lock().ok().and_then(|c| c.server_version());
    Ok((env!("CARGO_PKG_VERSION").to_owned(), server))
}

/// Whether this is something a browser should be handed.
///
/// Separate from the command so the rule can be tested without opening
/// anything. Deliberately a whitelist: `file:`, `javascript:` and every other
/// scheme a webview knows about are not links to a website.
fn is_web_link(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// Hand a web link to the browser.
///
/// The About tab has four of them and a webview follows a link in place, which
/// would turn the settings window into a browser with no address bar, no back
/// button and the app gone from underneath it.
///
/// Only http(s), and the URL is passed as an argument rather than through a
/// shell, so there is nothing here that can be talked into opening a file, a
/// script, or anything else with a scheme in front of it.
#[tauri::command]
fn open_link(url: String) -> CmdResult<()> {
    if !is_web_link(&url) {
        return Err("only web links can be opened".into());
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&url).spawn().map_err(err)?;
    Ok(())
}

/// One save state, as the shelf in the info pane shows it.
#[derive(Serialize)]
struct StateView {
    slot: String,
    label: String,
    /// Absolute path to the picture RetroArch saved with the state, if there is
    /// one. The page turns it into something it can load; states written before
    /// thumbnails were switched on have none and never will.
    thumb: Option<String>,
    when: Option<String>,
    size_bytes: u64,
    core: String,
    /// False for the autosave, which has no slot number to enter.
    resumable: bool,
    when_epoch: Option<u64>,
}

/// Check the RetroAchievements login.
///
/// Username and token only. A password would have to be held somewhere to
/// check with, and a status light is not worth a second secret — the token is
/// already stored because RetroArch needs it.
#[tauri::command]
async fn verify_achievements() -> CmdResult<moose_rack::achievements::Verified> {
    let cfg = Config::load().map_err(err)?;
    let user = cfg.achievements.username.clone().unwrap_or_default();
    let token = cfg.achievements.token.clone().unwrap_or_default();
    if user.trim().is_empty() || token.trim().is_empty() {
        return Ok(moose_rack::achievements::Verified {
            ok: false,
            user: None,
            error: Some(if user.trim().is_empty() {
                "no username set".into()
            } else {
                "no token set".to_string()
            }),
        });
    }
    Ok(moose_rack::achievements::verify(&user, &token).await)
}

/// The save states this game has.
#[tauri::command]
fn game_states(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<StateView>> {
    let Some(ra) = state.retroarch.as_ref() else {
        return Ok(Vec::new());
    };
    let cache = state.cache.lock().map_err(err)?;
    let Some(row) = cache.rom_by_id(id).map_err(err)? else {
        return Ok(Vec::new());
    };
    let now = std::time::SystemTime::now();
    Ok(moose_rack::states::shelf(&ra.root, &cache, &state.map, &row.fs_name)
        .map_err(err)?
        .into_iter()
        .map(|s| StateView {
            resumable: s.entry_slot().is_some(),
            when: s.modified.map(|t| moose_rack::states::ago(t, now)),
            // The raw time as well as the phrase. "3 days ago" cannot be
            // sorted, and picking the newest state to resume from is exactly
            // what the front end needs to do.
            when_epoch: s.modified,
            thumb: s.thumb.as_deref().map(moose_rack::util::webview_path),
            slot: s.slot,
            label: s.label,
            size_bytes: s.size,
            core: s.core,
        })
        .collect())
}

/// Delete one save state, keeping a copy in the backup folder.
///
/// Takes the slot rather than a path: a webview handing the backend a filename
/// to delete is a larger door than this needs, and the shelf already knows the
/// slot it drew.
#[tauri::command]
fn delete_state(state: State<'_, AppState>, id: i64, slot: String) -> CmdResult<String> {
    let ra = state.retroarch.as_ref().ok_or("RetroArch not found")?;
    let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
    let cache = state.cache.lock().map_err(err)?;
    let row = cache.rom_by_id(id).map_err(err)?.ok_or("no such game")?;
    let shelf = moose_rack::states::shelf(&ra.root, &cache, &state.map, &row.fs_name)
        .map_err(err)?;
    let found = shelf
        .into_iter()
        .find(|s| s.slot == slot)
        .ok_or_else(|| format!("no {slot} state for {}", row.name))?;
    let label = found.label.clone();
    moose_rack::states::remove(&library_root, id, &found).map_err(err)?;
    Ok(format!("deleted {label} — a copy is in the backups folder"))
}

/// Whether deleting a save state should ask first.
#[tauri::command]
fn confirm_delete_state() -> CmdResult<bool> {
    Ok(Config::load().unwrap_or_default().saves.confirm_delete_state)
}

/// Time played, by console and by game.
#[derive(Serialize)]
struct History {
    total_seconds: i64,
    sessions: i64,
    games: i64,
    platforms: Vec<PlatformTime>,
    top: Vec<GameTime>,
    /// Games opened more than once and still barely played.
    abandoned: Vec<GameTime>,
}

#[derive(Serialize)]
struct PlatformTime {
    slug: String,
    name: String,
    seconds: i64,
    spelled: String,
    sessions: i64,
    games: i64,
}

#[derive(Serialize)]
struct GameTime {
    id: i64,
    name: String,
    platform: String,
    seconds: i64,
    spelled: String,
    sessions: i64,
    last: Option<String>,
}

/// What the library has actually been used for.
///
/// Only sessions this app started. Anything played through ES-DE, on the
/// handheld, or before this existed is not here and cannot be — which is worth
/// saying on the page, because a total that looks like a lifetime figure and is
/// really a few weeks is a number that misleads.
#[tauri::command]
fn play_history(state: State<'_, AppState>) -> CmdResult<History> {
    let cache = state.cache.lock().map_err(err)?;
    let names: std::collections::HashMap<String, String> = cache
        .platforms()
        .map(|ps| ps.into_iter().map(|p| (p.fs_slug, p.display_name)).collect())
        .unwrap_or_default();
    let (total_seconds, sessions, games) = cache.play_totals().map_err(err)?;

    let platforms = cache
        .play_by_platform()
        .map_err(err)?
        .into_iter()
        .map(|(slug, seconds, sessions, games)| PlatformTime {
            name: names.get(&slug).cloned().unwrap_or_else(|| slug.clone()),
            spelled: moose_rack::util::spell_duration(seconds),
            slug,
            seconds,
            sessions,
            games,
        })
        .collect();

    let game = |(r, seconds, sessions, last): (cache::RomRow, i64, i64, String)| GameTime {
        id: r.id,
        name: r.name,
        platform: names.get(&r.platform_slug).cloned().unwrap_or(r.platform_slug),
        spelled: moose_rack::util::spell_duration(seconds),
        seconds,
        sessions,
        last: Some(last),
    };

    Ok(History {
        total_seconds,
        sessions,
        games,
        platforms,
        top: cache.play_by_game(12).map_err(err)?.into_iter().map(game).collect(),
        // Twice or more, under half an hour all told.
        abandoned: cache
            .abandoned(2, 1800, 12)
            .map_err(err)?
            .into_iter()
            .map(|(r, seconds, sessions)| GameTime {
                id: r.id,
                name: r.name,
                platform: names.get(&r.platform_slug).cloned().unwrap_or(r.platform_slug),
                spelled: moose_rack::util::spell_duration(seconds),
                seconds,
                sessions,
                last: None,
            })
            .collect(),
    })
}

/// The games played most recently, for the row at the top of the library.
///
/// Server timestamps, so the list is the same wherever you sign in — the point
/// is picking up where you left off, and that is rarely the machine you are
/// sitting at now.
#[tauri::command]
fn recent_games(
    state: State<'_, AppState>,
    limit: Option<usize>,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.recently_played(limit.unwrap_or(8)).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

/// What a bulk download should cover: what to take, and how much of it.
///
/// One struct rather than six parameters. The estimate and the download need
/// exactly the same set, so passing them separately meant two signatures that
/// had to be kept in step by hand — and the second of them had grown past what
/// the linter will accept.
#[derive(Debug, Clone, Deserialize)]
struct DownloadChoice {
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    collection: Option<String>,
    /// Ticked in the pane's own list. `collection` is the single one you
    /// pointed at before opening it; both are honoured, and a game in two of
    /// them is still one download.
    #[serde(default)]
    collections: Vec<String>,
    art: String,
    videos: bool,
    manuals: bool,
    bios: bool,
}

impl DownloadChoice {
    fn want(&self) -> moose_rack::bulk::Want {
        use moose_rack::bulk;
        bulk::Want {
            roms: true,
            art: match self.art.as_str() {
                "none" => bulk::Art::None,
                "full" => bulk::Art::Full,
                _ => bulk::Art::Minimal,
            },
            videos: self.videos,
            manuals: self.manuals,
        }
    }
}

/// Every game in the chosen systems and collection, in one list.
///
/// Systems are plural because the reason for taking a library offline is a
/// journey, and nobody travels with one console. Picking them one at a time
/// meant running the whole dialog once per system, each with its own size
/// check against a disk the previous run had already eaten into.
fn rows_for_choice(
    state: &State<'_, AppState>,
    platforms: &[String],
    collection: &Option<String>,
    collections: &[String],
) -> Result<Vec<cache::RomRow>, String> {
    let cache = state.cache.lock().map_err(err)?;
    let mut rows = Vec::new();
    for id in collection.iter().chain(collections.iter()) {
        rows.extend(cache.roms_in_collection(id).map_err(err)?);
    }
    for p in platforms {
        rows.extend(cache.roms_for(p).map_err(err)?);
    }
    if rows.is_empty() {
        return Err("nothing chosen".into());
    }
    // A game can be in a collection and in its system's list both, and paying
    // for it twice would overstate the download by however much they overlap.
    rows.sort_by_key(|r| r.id);
    rows.dedup_by_key(|r| r.id);
    Ok(rows)
}

/// What a bulk download would cost, before starting one.
///
/// Async, though it awaits nothing. A synchronous Tauri command runs on the
/// main thread, and this one asks the filesystem whether each game is already
/// here — one call per game, so on a 2,400-game console the window stopped
/// answering for seconds. Off the main thread it is the same work while the
/// interface keeps drawing.
#[tauri::command]
async fn download_estimate(
    state: State<'_, AppState>,
    choice: DownloadChoice,
) -> CmdResult<(String, bool, String)> {
    use moose_rack::{bulk, diskspace};

    let rows = rows_for_choice(&state, &choice.platforms, &choice.collection, &choice.collections)?;
    let want = choice.want();
    let mut est = bulk::estimate(&rows, want, |r| row_path(&state, r).is_some());
    // Asked of the server rather than averaged, because unlike artwork there is
    // a fixed set of these and it already knows which are here.
    let mut summary = est.describe();
    if choice.bios {
        let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
        if let Some(client) = state.client.clone()
            && let Ok((total, here, bytes)) = moose_rack::bios::status(&client, &library_root).await
        {
            est.media_bytes += bytes;
            summary = format!(
                "{}; plus {} BIOS file(s), {} already here",
                est.describe(),
                total - here,
                here
            );
        }
    }
    let (fits, note) = match diskspace::fits(&state.roms_dir, est.total()) {
        diskspace::Fit::Yes { available } => {
            (true, format!("{:.0} GB free", available as f64 / 1e9))
        }
        diskspace::Fit::No { available, short } => (
            false,
            format!(
                "only {:.0} GB free — {:.0} GB short, counting the {} GB this leaves spare",
                available as f64 / 1e9,
                short as f64 / 1e9,
                diskspace::MARGIN / 1_000_000_000,
            ),
        ),
        // Never a refusal: turning a failed syscall into "you cannot download"
        // would be a worse bug than the one the check exists to prevent.
        diskspace::Fit::Unknown => (true, "could not read free space".to_owned()),
    };
    Ok((summary, fits, note))
}

/// Download a whole platform or collection, with the media that was asked for.
///
/// Refuses up front when it would not fit. The failure without that check is
/// the worst kind: an hour of transfer, a full disk, and both a half-written
/// game and no room to clear it.
#[tauri::command]
async fn download_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    choice: DownloadChoice,
) -> CmdResult<String> {
    use moose_rack::{bulk, diskspace};

    let client = state.client.clone().ok_or("no server configured")?;
    let rows = rows_for_choice(&state, &choice.platforms, &choice.collection, &choice.collections)?;
    let want = choice.want();
    let est = bulk::estimate(&rows, want, |r| row_path(&state, r).is_some());
    if let diskspace::Fit::No { short, .. } = diskspace::fits(&state.roms_dir, est.total()) {
        return Err(format!(
            "not enough room — {:.1} GB short. Free some space or take less media.",
            short as f64 / 1e9
        ));
    }

    let media_root = state.media_dir.clone();
    let list_art = state.list_art.lock().map_err(err)?.clone();
    let mut games = 0usize;
    let total = rows.len();

    for (i, row) in rows.iter().enumerate() {
        if row_path(&state, row).is_none() {
            let members = if row.multi_file {
                client.member_hashes(row.id).await
            } else {
                Vec::new()
            };
            let target = moose_rack::download::Target {
                rom_id: row.id,
                members: &members,
                fs_name: &row.fs_name,
                platform_slug: &row.platform_slug,
                expected_size: (row.fs_size_bytes > 0).then_some(row.fs_size_bytes as u64),
                md5: row.md5_hash.as_deref(),
                sha1: row.sha1_hash.as_deref(),
                multi_file: row.multi_file,
            };
            if moose_rack::download::fetch(
                client.http(), client.base(), client.auth(), &target, &state.roms_dir, |_, _| {},
            )
            .await
            .is_ok()
            {
                games += 1;
            }
        }

        let stem = Path::new(&row.fs_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| row.fs_name.clone());
        // Whatever media was asked for. `ensure_*` skip anything already here,
        // so re-running after an interruption costs a stat rather than a fetch.
        if want.art != bulk::Art::None {
            let _ = media::ensure_art(
                Some(&client), &media_root, &row.platform_slug, &stem, &list_art,
            ).await;
            let _ = media::ensure_art(
                Some(&client), &media_root, &row.platform_slug, &stem, media::MIXIMAGES,
            ).await;
        }
        if want.art == bulk::Art::Full {
            for (kind, _) in media::ESDE_TYPES {
                if matches!(*kind, media::VIDEOS) {
                    continue;
                }
                let _ = media::ensure_esde(
                    Some(&client), &media_root, &row.platform_slug, &stem, kind,
                ).await;
            }
        }
        if want.videos {
            let _ = media::ensure_esde(
                Some(&client), &media_root, &row.platform_slug, &stem, media::VIDEOS,
            ).await;
        }
        if want.manuals {
            let _ = media::ensure_esde(
                Some(&client), &media_root, &row.platform_slug, &stem, "manuals",
            ).await;
        }

        if (i + 1) % 5 == 0 || i + 1 == total {
            let _ = app.emit("bulk-progress", format!("{}/{} — {games} downloaded", i + 1, total));
        }
    }
    // BIOS last, because it is the one part that is not per-game and the one
    // whose absence you only discover when a console refuses to boot somewhere
    // with no server to fetch from.
    let mut bios_note = String::new();
    if choice.bios {
        let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
        let _ = app.emit("bulk-progress", "BIOS files…".to_owned());
        match moose_rack::bios::sync(&client, &library_root, |done, got, name| {
            let _ = app.emit("bulk-progress", format!("BIOS {done}/{got} — {name}"));
        })
        .await
        {
            Ok(summary) => bios_note = format!("\n{}", summary.headline()),
            Err(e) => bios_note = format!("\nBIOS did not sync: {e}"),
        }
    }
    Ok(format!("{games} game(s) downloaded, {total} checked{bios_note}"))
}

#[tauri::command]
fn platforms(state: State<'_, AppState>) -> CmdResult<Vec<PlatformView>> {
    let cache = state.cache.lock().map_err(err)?;
    let rows = cache.platforms().map_err(err)?;
    // Consoles switched off in the library are not offered. ES-DE hides a
    // system by leaving a `noload.txt` in its directory, and a second frontend
    // over the same library has to agree with it.
    //
    // Filtered here rather than left to the scan, because the scan only decides
    // which games are found on the device: with a server configured the
    // platform has rows of its own and would come back into the grid with every
    // game in it still marked as somewhere else. Hiding a console means hiding
    // it.
    let switched_off = moose_rack::esde::switched_off_slugs(&state.roms_dir, &state.map);
    let rows: Vec<_> = rows.into_iter().filter(|p| !switched_off.contains(&p.fs_slug)).collect();
    // Read once for the whole grid rather than per platform: the lock would
    // otherwise be taken four times for each of thirty consoles.
    let set = state.icon_set.lock().map_err(err)?.clone();
    let look = state.icon_look.lock().map_err(err)?.clone();
    let views: Vec<PlatformView> = rows
        .into_iter()
        .map(|p| PlatformView {
            playable: resolve_core(&state, &p.fs_slug).is_some(),
            // A theme, if one is installed, then the console picture from the
            // server. The theme wins because installing one is a deliberate
            // choice and this is the fallback that means nobody has to.
            logo: theme::look_art(&state.media_dir, &p.fs_slug, &set, &look)
                .or_else(|| moose_rack::platformicon::installed(&state.media_dir, &p.fs_slug))
                .map(|p| moose_rack::util::webview_path(&p)),
            logo_wordmark: look.starts_with("styled-text")
                || (set.is_empty() && current_style(&state) == theme::IconStyle::Logo),
            // The info pane's picture, which does *not* follow the grid.
            //
            // Select cycles the grid's artwork — logo, console, controller —
            // and the pane was reading the same setting, so the console
            // portrait changed under you while you were reading about the
            // console. The pane wants a picture of the machine and always the
            // same one, so it asks for the hardware render and falls back to
            // the console-with-a-game before it settles for a wordmark.
            // The info pane wants a picture of the machine and always the same
            // one, so it asks for hardware by name rather than following the
            // grid's look.
            portrait: theme::look_art(&state.media_dir, &p.fs_slug, &set, "hardware")
                .or_else(|| theme::look_art(&state.media_dir, &p.fs_slug, &set, "systemart"))
                .or_else(|| theme::look_art(&state.media_dir, &p.fs_slug, &set, "consolegame"))
                .map(|p| moose_rack::util::webview_path(&p)),
            cover_aspect: media::cover_aspect(&state.media_dir, &p.fs_slug),
            manufacturer: moose_rack::platformfacts::of(&p.fs_slug).map(|f| f.manufacturer),
            released: moose_rack::platformfacts::of(&p.fs_slug).map(|f| f.released),
            hardware: moose_rack::platformfacts::of(&p.fs_slug).map(|f| f.hardware),
            blurb: moose_rack::platformfacts::of(&p.fs_slug).map(|f| f.blurb),
            slug: p.fs_slug,
            name: p.display_name,
            rom_count: p.rom_count,
        })
        .collect();
    // Alphabetically, here rather than in whatever draws them.
    //
    // The server hands these back by size, so every list in the app used to
    // open on whichever console happens to have the most ROMs in it. Sorted
    // once, at the source, because the console grid is redrawn on a layout
    // switch and on every batch of covers that arrives — and the order is not
    // something any of those redraws should have an opinion about.
    let order = moose_rack::pickorder::by_name(
        &views
            .iter()
            .map(|p| moose_rack::pickorder::PickerRow {
                name: p.name.clone(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    );
    let mut views: Vec<Option<PlatformView>> = views.into_iter().map(Some).collect();
    Ok(order.into_iter().filter_map(|i| views[i].take()).collect())
}

/// Shape cache rows for the list/grid, marking what is already on disk.
/// Turn cache rows into what a list draws, and remember them if asked.
///
/// `stash` is the scope the rows belong to — `roms:arcade:` and the like. A
/// list a front end will sort and filter passes one; the strip of recently
/// played games at the top of the console grid does not, because it is eight
/// rows in a fixed order and stashing them would displace the two and a half
/// thousand the middle column is showing.
fn to_views(
    state: &State<'_, AppState>,
    rows: Vec<cache::RomRow>,
    stash: Option<&str>,
) -> Vec<RomView> {
    // One query for the whole list rather than one per row.
    let favorites = state
        .cache
        .lock()
        .ok()
        .and_then(|c| c.favorite_ids().ok())
        .unwrap_or_default();
    let views: Vec<RomView> = rows
        .into_iter()
        .map(|r| {
            let meta: Option<serde_json::Value> =
                r.meta_json.as_deref().and_then(|m| serde_json::from_str(m).ok());
            RomView {
                favorite: favorites.contains(&r.id),
                downloaded: row_path(state, &r).is_some(),
                rating: meta
                    .as_ref()
                    .and_then(|m| m.get("average_rating"))
                    .and_then(|v| v.as_f64()),
                // RomM stores the release date as epoch milliseconds.
                year: meta
                    .as_ref()
                    .and_then(|m| m.get("first_release_date"))
                    .and_then(|v| v.as_f64())
                    .map(|ms| 1970 + (ms / 1000.0 / 31_556_952.0) as i32),
                players: meta
                    .as_ref()
                    .and_then(|m| m.get("player_count"))
                    .and_then(|v| v.as_str())
                    .and_then(cache::max_players),
                last_played: r.last_played.clone(),
                rel_dir: r.rel_dir.clone(),
                id: r.id,
                name: r.name,
                fs_name: r.fs_name,
                platform: r.platform_slug,
                size_bytes: r.fs_size_bytes,
            }
        })
        .collect();
    if let Some(scope) = stash
        && let (Ok(mut held), Ok(mut at)) = (state.list_rows.lock(), state.list_scope.lock())
    {
        *held = views.iter().map(RomView::as_row).collect();
        *at = scope.to_owned();
    }
    views
}

#[tauri::command]
fn roms(
    state: State<'_, AppState>,
    platform: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.roms_for(&platform).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

/// One collection group — `genre`, `franchise`, `user`, …
#[derive(serde::Serialize)]
struct GroupView {
    group: String,
    /// Human label; the raw group name is a server-side slug.
    label: String,
    count: i64,
}

#[derive(serde::Serialize)]
struct CollectionView {
    id: String,
    name: String,
    rom_count: i64,
    is_favorite: bool,
    /// A few member ROM ids — the card fetches their covers through the same
    /// local cache the game grids use, so this works offline too.
    sample_ids: Vec<i64>,
    /// How many of its games are downloaded here.
    ///
    /// The question a collection card cannot otherwise answer: "can I play any
    /// of this on a plane". Counted rather than stored, because a file can be
    /// deleted from under us and a stale count is worse than none.
    local_count: i64,
}

/// Plural label for a group, since the server's names are singular slugs.
fn group_label(group: &str) -> String {
    match group {
        "user" => "My collections".to_owned(),
        "smart" => "Smart collections".to_owned(),
        "collection" => "Series".to_owned(),
        "genre" => "Genres".to_owned(),
        "franchise" => "Franchises".to_owned(),
        "company" => "Companies".to_owned(),
        "mode" => "Player modes".to_owned(),
        "age_rating" => "Age ratings".to_owned(),
        // Unknown kinds still appear rather than being hidden — a RomM that
        // grows a new one should show up without a client change.
        other => {
            let mut c = other.replace('_', " ");
            c[..1].make_ascii_uppercase();
            c
        }
    }
}

#[tauri::command]
fn collection_groups(state: State<'_, AppState>) -> CmdResult<Vec<GroupView>> {
    let cache = state.cache.lock().map_err(err)?;
    Ok(cache
        .collection_groups()
        .map_err(err)?
        .into_iter()
        .map(|(group, count)| GroupView {
            label: group_label(&group),
            group,
            count,
        })
        .collect())
}

#[tauri::command]
fn collections_in(state: State<'_, AppState>, group: String) -> CmdResult<Vec<CollectionView>> {
    let cache = state.cache.lock().map_err(err)?;
    Ok(cache
        .collections_in(&group)
        .map_err(err)?
        .into_iter()
        .map(|c| CollectionView {
            local_count: cache
                .roms_in_collection(&c.id)
                .map(|rows| rows.iter().filter(|r| row_path(&state, r).is_some()).count() as i64)
                .unwrap_or(0),
            sample_ids: c.sample_ids,
            id: c.id,
            name: c.name,
            rom_count: c.rom_count,
            is_favorite: c.is_favorite,
        })
        .collect())
}

#[tauri::command]
fn collection_roms(
    state: State<'_, AppState>,
    id: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.roms_in_collection(&id).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

#[tauri::command]
fn search(
    state: State<'_, AppState>,
    term: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.search(&term, 200).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

#[tauri::command]
async fn rom_detail(state: State<'_, AppState>, id: i64) -> CmdResult<RomDetail> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let core = resolve_core_for(&state, &row.platform_slug, Some(&row.fs_name));
    let core_label = core
        .as_deref()
        .and_then(|c| state.map.label_for(c))
        .map(str::to_owned);

    // ES-DE files media under <media>/<platform>/<type>/<rom basename>.<ext>.
    let stem = Path::new(&row.fs_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| row.fs_name.clone());

    // Local ES-DE media covers only ~2% of this library, so fall back to the
    // server's artwork and cache it into the same tree.
    let client = state.client.clone();
    // A locally scanned ES-DE library keeps its artwork on the same disk as
    // the games, keyed by ES-DE system name. Nothing needs fetching there, so
    // the server client is dropped for those rows — otherwise every miss would
    // become a pointless request against a server this library did not come
    // from.
    let (scope_dir, scope_key) = media_scope(&state, &row);
    let media_root = scope_dir.to_path_buf();
    let media_key = scope_key.to_owned();
    let client = if state.esde_media.is_some() && row.esde_system.is_some() {
        None
    } else {
        client
    };
    let as_url =
        |p: Option<std::path::PathBuf>| p.map(|p| moose_rack::util::webview_path(&p));

    // ES-DE's own art, picked the way its Canvas theme picks it. RomM's cover
    // is no longer consulted: it is a second scrape from a different source,
    // and one game's art coming from one place and the next game's from
    // another is the inconsistency this replaces.
    let cover =
        media::ensure_art(client.as_deref(), &media_root, &media_key, &stem, &state.detail_art)
            .await;
    let screenshots = media::ensure_set(
        client.as_deref(), &media_root, &media_key, &stem,
        &row.screenshots(),
    ).await;
    // Only if it is already here. Downloading is what the play button does.
    let video = media::find_local(&media_root, &media_key, &stem, media::VIDEOS);
    let has_video = video.is_some()
        || media::video_exists(client.as_deref(), &media_root, &media_key, &stem).await;

    // Manuals are PDFs, which the webview renders natively.
    let manual = media::ensure(
        client.as_deref(), &media_root, &media_key, &stem,
        media::MANUALS, row.manual_path.as_deref(),
    ).await;

    // Everything ES-DE has for this game, fetched lazily and cached.
    let mut art = std::collections::BTreeMap::new();
    for (kind, _) in media::ESDE_TYPES {
        // Videos are fetched by the play button, never by looking at a game.
        if matches!(*kind, media::VIDEOS) {
            continue;
        }
        if let Some(p) = media::ensure_esde(
            client.as_deref(), &media_root, &media_key, &stem, kind,
        )
        .await
        {
            art.insert((*kind).to_owned(), moose_rack::util::webview_path(&p));
        }
    }

    let meta: Option<serde_json::Value> = row
        .meta_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    let json_list = |s: &Option<String>| -> Vec<String> {
        s.as_deref()
            .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
            .unwrap_or_default()
    };
    // RomM stores the release date as epoch milliseconds.
    let release_year = meta
        .as_ref()
        .and_then(|m| m.get("first_release_date"))
        .and_then(|v| v.as_f64())
        .map(|ms| 1970 + (ms / 1000.0 / 31_556_952.0) as i32);

    Ok(RomDetail {
        cover: as_url(cover),
        video: as_url(video),
        has_video,
        screenshots: screenshots
            .into_iter()
            .map(|p| moose_rack::util::webview_path(&p))
            .collect(),
        art,
        downloaded: row_path(&state, &row).is_some(),
        autofire: autofire_possible(&row)
            .then(|| moose_rack::tweaks::AutoFire::parse(&stored_autofire()).key().to_owned()),
        autofire_hz: autofire_hz(&state),
        platform_slug: row.platform_slug.clone(),
        id: row.id,
        name: row.name,
        fs_name: row.fs_name,
        platform: row.platform_slug,
        size_bytes: row.fs_size_bytes,
        core,
        core_label,
        summary: row.summary.clone().filter(|s| !s.is_empty()),
        genres: meta_strings(&meta, "genres"),
        companies: meta_strings(&meta, "companies"),
        franchises: meta_strings(&meta, "franchises"),
        game_modes: meta_strings(&meta, "game_modes"),
        player_count: meta
            .as_ref()
            .and_then(|m| m.get("player_count"))
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        rating: meta
            .as_ref()
            .and_then(|m| m.get("average_rating"))
            .and_then(|v| v.as_f64()),
        release_year,
        alt_names: json_list(&row.alt_names_json),
        regions: json_list(&row.regions_json),
        manual: manual.map(|p| moose_rack::util::webview_path(&p)),
        youtube_id: row.youtube_id.clone().filter(|s| !s.is_empty()),
    })
}

#[derive(Serialize, Clone)]
struct CoverView {
    id: i64,
    cover: Option<String>,
}

/// Resolve covers for a batch of ROMs.
///
/// The grid asks only for what has scrolled into view, and this bounds the
/// fan-out further, so browsing a 2,400-game platform never turns into 2,400
/// simultaneous requests.
#[tauri::command]
async fn rom_covers(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    local_only: Option<bool>,
) -> CmdResult<Vec<CoverView>> {
    const CONCURRENCY: usize = 8;

    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        ids.iter()
            .filter_map(|id| cache.rom_by_id(*id).ok().flatten())
            .collect::<Vec<_>>()
    };

    let list_art = state.list_art.lock().map_err(err)?.clone();
    // The cached answer, with no request behind it. A caller that wants the
    // grid filled *now* asks for this first: everything already on disk comes
    // back in a few milliseconds, and the misses are fetched by a second call
    // that can take as long as it likes because there is already something on
    // screen.
    if local_only.unwrap_or(false) {
        return Ok(rows
            .iter()
            .map(|row| {
                let stem = Path::new(&row.fs_name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| row.fs_name.clone());
                // Through `media_scope`, the same as `rom_detail`. This asked
                // `state.media_dir` and `platform_slug` directly, which is this
                // app's own download folder keyed by RomM's slug — and on a
                // device whose library is an ES-DE folder the artwork is not
                // there and is not keyed that way. Every cover came back null:
                // measured, the four samples behind each collection in My
                // Collections resolved to nothing, so every card in the tab drew
                // the two-letter placeholder.
                let (dir, key) = media_scope(&state, row);
                CoverView {
                    id: row.id,
                    cover: media::local_art(dir, key, &stem, &list_art)
                        .map(|p| moose_rack::util::webview_path(&p)),
                }
            })
            .collect());
    }
    let mut out = Vec::with_capacity(rows.len());
    // A steady stream of `CONCURRENCY` requests, not lockstep batches.
    //
    // This walked `rows.chunks(8)` and waited for all eight before starting the
    // next eight, so the cost was the sum of the slowest cover in each group
    // rather than the time to fetch them all. One cover the server is slow
    // about held seven finished ones and eight unstarted ones behind it. Over
    // wifi on a handheld that is the difference between a grid that fills and
    // one that arrives in visible steps — which is exactly what it looked like.
    //
    // A task is started the moment a slot frees, and each result is emitted as
    // it lands, so the page keeps filling continuously.
    let mut set = tokio::task::JoinSet::new();
    let mut queue = rows.iter();
    let mut pending = Vec::new();
    loop {
        // Fill every free slot.
        while set.len() < CONCURRENCY {
            let Some(row) = queue.next() else { break };
            let client = state.client.clone();
            // The same scope the fast pass above uses, so a cover found by one
            // is the cover fetched by the other.
            let (scope_dir, scope_key) = media_scope(&state, row);
            let media_root = scope_dir.to_path_buf();
            let (id, platform, fs_name) =
                (row.id, scope_key.to_owned(), row.fs_name.clone());
            let art = list_art.clone();
            set.spawn(async move {
                let stem = Path::new(&fs_name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or(fs_name);
                let cover =
                    media::ensure_art(client.as_deref(), &media_root, &platform, &stem, &art)
                        .await;
                CoverView { id, cover: cover.map(|p| moose_rack::util::webview_path(&p)) }
            });
        }
        let Some(res) = set.join_next().await else { break };
        if let Ok(v) = res {
            pending.push(v);
        }
        // Handed over in small groups as they land, rather than one event per
        // cover: eighty events would be eighty separate repaints of the same
        // grid. Flushed whenever the queue drains too, so the last few are not
        // left waiting for a group that will never fill.
        if pending.len() >= 4 || set.is_empty() {
            if pending.iter().any(|c| c.cover.is_some()) {
                let _ = app.emit("covers-ready", &pending);
            }
            out.append(&mut pending);
        }
    }
    out.append(&mut pending);
    // Keep what this batch learned, so scrolling back over the same cards — and
    // the next launch — costs nothing.
    for platform in rows.iter().map(|r| r.platform_slug.as_str()).collect::<std::collections::BTreeSet<_>>() {
        media::save_art_index(&state.media_dir, platform);
    }
    Ok(out)
}

/// Turn the star on this game the other way. Returns what it now is.
///
/// The star is a *collection* on the server, not a flag on the game, so this
/// reaches the handheld and the phone as well — see `moose_rack::favorites`.
///
/// Which way to turn it is decided here, from the cache, rather than passed in
/// by the page: the page knows what it last drew, and what it last drew can be
/// older than what another device has since done.
///
/// Three steps, because the cache is behind a lock this must not hold while it
/// waits on the network — `Cache` is not `Sync` and the future would not
/// compile. Work out where the star goes, do it on the server, write it down.
#[tauri::command]
async fn toggle_favorite(state: State<'_, AppState>, id: i64) -> CmdResult<bool> {
    let client = state
        .client
        .clone()
        .ok_or("no server connection — check config.toml")?;

    let (target, starred) = {
        let cache = state.cache.lock().map_err(err)?;
        let row = cache
            .rom_by_id(id)
            .map_err(err)?
            .ok_or_else(|| format!("no rom with id {id}"))?;
        let now = moose_rack::favorites::is_starred(&cache, id).map_err(err)?;
        (
            moose_rack::favorites::target(&cache, &row.platform_slug).map_err(err)?,
            !now,
        )
    };

    let landed = moose_rack::favorites::on_server(&client, target, id, starred)
        .await
        .map_err(err)?;
    let Some(landed) = landed else {
        // Unstarring something that was never in a list. Nothing failed, and
        // it is already what was wanted.
        return Ok(false);
    };

    let mut cache = state.cache.lock().map_err(err)?;
    moose_rack::favorites::record(&mut cache, &landed, id, starred).map_err(err)?;
    Ok(starred)
}

/// Download a ROM, emitting `download-progress` events as it goes.
#[tauri::command]
async fn download_rom(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> CmdResult<String> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let client = state
        .client
        .clone()
        .ok_or("no server connection — check config.toml")?;
    let roms_dir = state.roms_dir.clone();

    // Folder ROMs verify per member; the rom-level hash is not reproducible.
    let members = if row.multi_file {
        client.member_hashes(row.id).await
    } else {
        Vec::new()
    };

    let target = download::Target {
        rom_id: row.id,
        members: &members,
        fs_name: &row.fs_name,
        platform_slug: &row.platform_slug,
        expected_size: (row.fs_size_bytes > 0).then_some(row.fs_size_bytes as u64),
        md5: row.md5_hash.as_deref(),
        sha1: row.sha1_hash.as_deref(),
        multi_file: row.multi_file,
    };

    let mut last = std::time::Instant::now();
    let outcome = download::fetch(
        client.http(),
        client.base(),
        client.auth(),
        &target,
        &roms_dir,
        |done, total| {
            // Throttle: the webview does not need 60 events a second.
            if last.elapsed().as_millis() < 100 {
                return;
            }
            last = std::time::Instant::now();
            let _ = app.emit("download-progress", (id, done, total));
        },
    )
    .await
    .map_err(err)?;

    let _ = app.emit("download-progress", (id, 1u64, 1u64));
    Ok(match outcome {
        download::Outcome::AlreadyHave(p) => format!("already had {}", p.display()),
        download::Outcome::Downloaded { path, verified, .. } => {
            format!("downloaded {} ({})", path.display(), verified.describe())
        }
    })
}

/// Launch a ROM in RetroArch. Blocks until the emulator exits.
///
/// The artwork choices for a game list, and which one is in force.
#[tauri::command]
fn list_art_options(state: State<'_, AppState>) -> CmdResult<(Vec<(String, String)>, String)> {
    let current = state.list_art.lock().map_err(err)?.clone();
    let choices = media::LIST_ART_CHOICES
        .iter()
        .map(|(k, label)| ((*k).to_owned(), (*label).to_owned()))
        .collect();
    Ok((choices, current))
}

/// Change what the game lists draw.
///
/// The cached image for a game is keyed by kind, so switching does not throw
/// anything away — art already fetched for the old choice stays where it is and
/// is there again if you switch back.
#[tauri::command]
fn set_list_art(state: State<'_, AppState>, value: String) -> CmdResult<String> {
    if !media::LIST_ART_CHOICES.iter().any(|(k, _)| *k == value) {
        return Err(format!("unknown artwork type {value}"));
    }
    moose_rack::config::set_table_entry("config.toml", "media", "list_art", &value)
        .map_err(err)?;
    *state.list_art.lock().map_err(err)? = value.clone();
    Ok(format!("game lists now show {value}"))
}

/// Fetch artwork for games that have none, through the server's ScreenScraper
/// account.
///
/// Serial and unhurried on purpose: ScreenScraper throttles by account tier and
/// answers an exceeded allowance with a rejection rather than a picture, so a
/// run that hurries finishes fast and fetches nothing. The allowance being
/// spent is the server's, shared with anything else pointed at it.
#[tauri::command]
async fn scrape_missing(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    platform: Option<String>,
) -> CmdResult<String> {
    // The media listings are cached for the session, so anything written below
    // is invisible until they are dropped. See `esde::media_listing`.
    let _drop_media_cache = scopeguard_forget_media();

    use moose_rack::scrape;

    let client = state
        .client
        .clone()
        .ok_or("no server configured — set [server] in config.toml")?;
    let media_root = state.media_dir.clone();

    let todo = {
        let cache = state.cache.lock().map_err(err)?;
        scrape::missing(&cache, &media_root, platform.as_deref()).map_err(err)?
    };
    if todo.is_empty() {
        return Ok("every game already has artwork".to_owned());
    }

    let _ = app.emit("scrape-progress", format!("{} to look up…", todo.len()));
    let mut report = scrape::Report::default();
    for (i, row) in todo.iter().enumerate() {
        let _ = scrape::fill_one(&client, &media_root, row, false, &mut report).await;
        if (i + 1).is_multiple_of(10) || i + 1 == todo.len() {
            let _ = app.emit(
                "scrape-progress",
                format!("{}/{} — {} found", i + 1, todo.len(), report.fetched),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    // Those games have art now, so what the grid learned about them is wrong.
    for platform in todo.iter().map(|r| r.platform_slug.as_str()).collect::<std::collections::BTreeSet<_>>() {
        media::clear_art_index(&state.media_dir, platform);
    }
    Ok(report.describe())
}

/// Fetch a game's video and hand back its path, downloading it if needed.
///
/// Deliberately a separate command from `rom_detail`, and deliberately slow:
/// this is the one moment someone has asked for a video, so it is the one
/// moment worth spending tens of megabytes on.
#[tauri::command]
async fn game_video(state: State<'_, AppState>, id: i64) -> CmdResult<String> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let stem = Path::new(&row.fs_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| row.fs_name.clone());
    let (scope_dir, scope_key) = media_scope(&state, &row);
    let (media_root, media_key) = (scope_dir.to_path_buf(), scope_key.to_owned());
    let client = state.client.clone();

    media::ensure_esde(client.as_deref(), &media_root, &media_key, &stem, media::VIDEOS)
        .await
        .map(|p| moose_rack::util::webview_path(&p))
        .ok_or_else(|| format!("no video for {}", row.name))
}

/// The display the app is on, in the units RetroArch's window sizing uses.
///
/// Not a single number, because the two platforms disagree. Windows sizes
/// windows in device pixels, so a 4K screen at 150% scaling wants 3840. macOS
/// sizes them in points, so the same physical screen wants the logical figure —
/// pass it the device pixels there and the window is asked to be twice the size
/// of the display, which the compositor answers by clamping it to a shape
/// neither centered nor the size that was asked for.
///
/// Asked of the monitor rather than of the webview: `window.screen` reports CSS
/// pixels, which is the logical figure on both, so it is wrong on exactly one
/// of them and looks right while being tested on the other.
fn work_area(app: &tauri::AppHandle) -> Option<moose_rack::retroarch::Screen> {
    use tauri::Manager as _;

    // macOS first, and without the toolkit. A monitor arrives from Tauri as a
    // pixel size plus a scale factor, and dividing one by the other is only
    // correct when the display runs at its native resolution. This machine is
    // a 3024x1964 panel with a backing scale of 2 showing an 1800x1169
    // desktop, so that arithmetic gives 1512x982 — wrong by a third, and wrong
    // in a way that puts the game window somewhere nobody asked for.
    //
    // CoreGraphics reports points directly, in the space the window server and
    // therefore RetroArch use.
    let all = moose_rack::macdisplay::displays();
    if !all.is_empty() {
        let choice = moose_rack::macdisplay::Choice::parse(&state_game_display());
        // The main display's height is the origin of the vertical coordinate
        // space whichever screen the game lands on, so it travels separately.
        let primary = all
            .iter()
            .find(|d| d.main)
            .map(|d| d.bounds.height)
            .unwrap_or(all[0].bounds.height);
        let d = moose_rack::macdisplay::choose(&all, choice)?;
        return Some(moose_rack::retroarch::Screen {
            x: d.bounds.x as i32,
            y: d.bounds.y as i32,
            width: d.bounds.width as u32,
            height: d.bounds.height as u32,
            primary_height: primary as u32,
        });
    }

    // The monitor the library window is on, so launching from a laptop screen
    // with an external display attached sizes for the one being looked at.
    let monitor = app
        .get_webview_window("main")
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;

    let size = monitor.size();
    let at = monitor.position();
    let primary_height = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.size().height)
        .unwrap_or(size.height);
    Some(moose_rack::retroarch::Screen {
        x: at.x,
        y: at.y,
        width: size.width,
        height: size.height,
        primary_height,
    })
}

/// Whether this game gets auto-fire, and where.
///
/// One function so the badge in the info pane and the config written at launch
/// cannot drift: a cue that says a game has auto-fire when it does not is worse
/// than no cue.
fn autofire_for(row: &cache::RomRow) -> moose_rack::tweaks::AutoFire {
    use moose_rack::tweaks::AutoFire;
    if !autofire_possible(row) {
        return AutoFire::Off;
    }
    AutoFire::parse(&stored_autofire())
}

/// The rate for the next launch: whatever the pane was nudged to, or the one
/// in config.toml.
fn autofire_hz(state: &State<'_, AppState>) -> u32 {
    state
        .autofire_hz
        .lock()
        .ok()
        .and_then(|v| *v)
        .unwrap_or_else(|| Config::load().map(|c| c.retroarch.autofire_hz).unwrap_or(6))
}

/// Nudge the rate for this run of the app only.
///
/// Deliberately not written to disk: this is a control you press five times in
/// a row while looking at a game, and config.toml is not a scratchpad.
#[tauri::command]
fn set_autofire_hz(state: State<'_, AppState>, hz: u32) -> CmdResult<u32> {
    let hz = hz.clamp(1, 30);
    *state.autofire_hz.lock().map_err(err)? = Some(hz);
    Ok(hz)
}

/// The setting as it is on disk, not as it was when the window opened.
///
/// It was read once at startup and kept in AppState. Changing it from the info
/// pane wrote config.toml and then asked the backend what the setting was — and
/// got the old answer, so the highlight never moved and the choice looked like
/// it had not registered. The launch was using the stale value too.
fn stored_autofire() -> String {
    Config::load().unwrap_or_default().retroarch.autofire
}

/// Whether this game is one auto-fire applies to at all.
///
/// The cabinets that were built around a hammered button, and only those: a
/// "shooter" on a home console is as likely to be a light-gun game or a shmup
/// that already has auto-fire of its own.
fn autofire_possible(row: &cache::RomRow) -> bool {
    if !matches!(row.platform_slug.as_str(), "arcade" | "neogeoaes" | "neogeocd") {
        return false;
    }
    let meta = row.meta_json.as_deref().and_then(|m| serde_json::from_str(m).ok());
    meta_strings(&meta, "genres").iter().any(|g| g.to_lowercase().contains("shoot"))
}

/// The stored screen preference, read fresh so unplugging a monitor and
/// changing the setting both take effect on the next launch rather than the
/// next restart.
fn state_game_display() -> String {
    Config::load().unwrap_or_default().retroarch.game_display
}

/// The screens a game could open on, and which one is chosen.
#[derive(Serialize)]
struct DisplayView {
    key: String,
    label: String,
    selected: bool,
}

#[tauri::command]
fn game_displays() -> CmdResult<Vec<DisplayView>> {
    use moose_rack::macdisplay::{self, Choice};
    let all = macdisplay::displays();
    // One screen is not a choice, and a dropdown with a single entry is a
    // control that asks a question with one answer.
    if all.len() < 2 {
        return Ok(Vec::new());
    }
    let now = Choice::parse(&state_game_display());
    let mut out = vec![
        DisplayView {
            key: "auto".to_owned(),
            label: "Automatic — prefer an external screen".to_owned(),
            selected: now == Choice::PreferExternal,
        },
        DisplayView {
            key: "main".to_owned(),
            label: "The one with the menu bar".to_owned(),
            selected: now == Choice::Main,
        },
    ];
    out.extend(all.iter().enumerate().map(|(i, d)| DisplayView {
        key: i.to_string(),
        label: d.label(),
        selected: now == Choice::Index(i),
    }));
    Ok(out)
}

/// `pad` is the name the frontend's Gamepad API reports for the connected
/// controller, used to pick the RetroArch autoconfig profile the gamepad
/// hotkeys are derived from. Absent when nothing is connected.
#[tauri::command]
async fn launch_rom(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
    pad: Option<String>,
    refresh: Option<f32>,
    // Set by the retry after the user answered an offline warning.
    skip_sync: Option<bool>,
    // A save state to start in, chosen from the shelf in the info pane.
    entry_slot: Option<u32>,
) -> CmdResult<String> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let ra = state.retroarch.as_ref().ok_or("RetroArch not found")?;
    // row_path, not local_path: the grid marks a row downloaded with this, and
    // a launch that disagreed said "not downloaded yet" about a folder ROM
    // sitting right there. launch::plan resolves the folder to its playlist.
    let path = row_path(&state, &row).ok_or("not downloaded yet")?;
    // One shared planner for GUI, CLI and TUI — see launch.rs for why.
    let overrides = state.core_overrides.lock().map_err(err)?.clone();
    let per_game = state.core_per_game.lock().map_err(err)?.clone();
    let shader_overrides = state.shader_overrides.lock().map_err(err)?.clone();
    let motion = state.motion_shader.lock().map_err(err)?.clone();
    let lightgun = state.lightgun.lock().map_err(err)?.clone();
    let lib = state.roms_dir.parent().unwrap_or(Path::new("."));
    // Read per launch rather than held on AppState: it is a path the settings
    // window can change while the app is running, and a copy taken at startup
    // would send saves to the old folder until a restart.
    let saves_root = Config::load()
        .map(|c| moose_rack::util::expand_tilde(&c.saves.root))
        .unwrap_or_default();
    let req = moose_rack::launch::Request {
        saves_root: Some(&saves_root),
        fit_window: state.fit_window,
        window_decorations: state.window_decorations,
        // Only where the metadata says shooter, and only on the platforms
        // whose cabinets had a fire button: a "shooter" on the Mega Drive is
        // as likely to be a light-gun game or a shmup with its own auto-fire.
        autofire: autofire_for(&row),
        save_state_on_exit: Config::load()
            .map(|c| c.retroarch.save_state_on_exit)
            .unwrap_or(false),
        autofire_hz: autofire_hz(&state),
        mirror_players: state.mirror_players,
        entry_slot,
        rom: &path,
        platform: &row.platform_slug,
        fs_name: &row.fs_name,
        library_root: lib,
        user_cfg: &state.user_retroarch_cfg,
        shaders_enabled: state.shaders_enabled,
        shader_overrides: &shader_overrides,
        motion_shader: motion.as_deref(),
        refresh_hz: refresh,
        core_overrides: &overrides,
        core_per_game: &per_game,
        core_override: None,
        pad: pad.as_deref(),
        achievements: Some(&state.achievements),
        lightgun: &lightgun,
        screen: work_area(&app),
    };
    // Fetch what is missing before planning. `plan` only ever picks among cores
    // already on disk, so without this a fresh install — which has none — fails
    // with "no installed core" even when the buildbot has it.
    let wanted = coremap::resolve_core_for(
        &state.map,
        &overrides,
        &per_game,
        &row.platform_slug,
        Some(&row.fs_name),
        |_| true, // ignore what is installed; that is the point
    );
    // Say what is happening. Between pressing play and the emulator appearing
    // there are four things that can take a visible moment — fetching a core,
    // fetching a shader pack, fetching BIOS, and asking the server about saves
    // — and none of them used to announce itself. A window that goes quiet for
    // several seconds reads as one that has hung, and gives nothing to report
    // when it is slow.
    let say = |what: &str| {
        let _ = app.emit("launch-progress", what.to_owned());
    };
    let mut fetched = Vec::new();
    // Reuses the API client's HTTP stack; without a configured server there is
    // nothing to download from anyway.
    if let (Some(core), Some(api)) = (wanted.as_deref(), state.client.as_ref()) {
        let http = api.http();
        say("checking the emulator core…");
        match moose_rack::cores::ensure(http, ra, core).await {
            Ok(true) => fetched.push(format!("downloaded the {core} core")),
            Ok(false) => {}
            // Not fatal: an offline launch of an already-installed core should
            // still work, and `plan` reports the real problem if it does not.
            Err(e) => fetched.push(format!("could not fetch {core}: {e}")),
        }
        if state.shaders_enabled {
            say("checking shaders…");
            match shaders::ensure_pack(http, ra).await {
                Ok(true) => fetched.push("downloaded the shader pack".to_owned()),
                Ok(false) => {}
                Err(e) => fetched.push(format!("could not fetch shaders: {e}")),
            }
        }
        // BIOS, for the same reason as the core: telling someone to go and
        // install one is advice delivered at the exact moment they cannot see
        // why the screen is black. Only what this platform actually needs.
        let library_root = state.roms_dir.parent().unwrap_or(Path::new("."));
        say("checking BIOS files…");
        match moose_rack::bios::ensure(api, library_root, core, &row.platform_slug).await {
            Ok(0) => {}
            Ok(n) => fetched.push(format!("fetched {n} BIOS file(s)")),
            Err(e) => {
                // Refused rather than noted. This used to go into the launch
                // notes and start the game anyway, which meant the one case
                // the automatic fetch cannot fix — a file the server has not
                // got either — arrived as a black screen with the explanation
                // scrolled past behind it. The front end offers "play anyway",
                // because a core that wants a BIOS sometimes runs without one.
                if !skip_sync.unwrap_or(false) {
                    let want = moose_rack::bios::required_for(core, &row.platform_slug);
                    let dest = moose_rack::bios::system_dir(library_root);
                    let missing: Vec<String> = want
                        .into_iter()
                        .filter(|n| !dest.join(n).is_file())
                        .collect();
                    if !missing.is_empty() {
                        return Err(format!(
                            "BIOS_MISSING:{} needs {} — {}",
                            row.platform_slug,
                            missing.join(", "),
                            e
                        ));
                    }
                }
                fetched.push(format!("could not fetch BIOS: {e}"));
            }
        }
    }

    let plan = moose_rack::launch::plan(ra, &state.map, &req).map_err(err)?;

    // Steam-cloud shape: pull what the server has that is newer, play, then
    // push whatever changed. `plan.run` blocks until the emulator exits, so
    // those two moments are a real boundary rather than a guess about when
    // someone stopped playing.
    let mut notes = fetched;
    let pre = if skip_sync.unwrap_or(false) {
        // The user already said "play anyway" to an offline warning; asking
        // again on the retry would be a loop they cannot get out of.
        notes.push("saves: not synced — you chose to play anyway".to_owned());
        AutoSync::default()
    } else {
        say("checking saves with the server…");
        auto_sync(&state, ra, &row, savesync::When::BeforeLaunch).await
    };
    if let Some(note) = pre.note {
        notes.push(note);
    }
    // Could not sync at all. Steam asks rather than either blocking or starting
    // silently, and it is the right call: the save may be stale, which is worth
    // knowing before you put an hour into it, but being unable to play because
    // a server is off would be worse.
    if let Some(why) = pre.failed {
        return Err(format!("SAVE_OFFLINE:{why}"));
    }
    // A conflict stops the launch, as Steam does. Playing on top of a save
    // whose ownership is unresolved is how the loser gets overwritten for good
    // on the way back out — the one moment where continuing quietly is worse
    // than refusing.
    if !pre.conflicts.is_empty() {
        *state.pending_conflicts.lock().map_err(err)? = pre.conflicts.clone();
        return Err(format!(
            "SAVE_CONFLICT:{}",
            serde_json::to_string(&pre.conflicts).unwrap_or_default()
        ));
    }

    say("starting RetroArch…");
    let began = std::time::Instant::now();
    let started_at = moose_rack::util::now_iso();
    let status = plan.run(ra, false).map_err(err)?;

    // How long that took is the only record of it. RetroArch tells nobody, and
    // the server's `last_played` only moves when something tells the server —
    // which nothing here did, so playing a game on this machine used to leave
    // no trace on this machine at all.
    let seconds = began.elapsed().as_secs() as i64;
    if let Ok(cache) = state.cache.lock()
        && let Ok(true) = cache.record_play(row.id, &started_at, seconds)
    {
        notes.push(format!("played for {}", moose_rack::util::spell_duration(seconds)));
    }

    let post = if skip_sync.unwrap_or(false) {
        AutoSync::default()
    } else {
        auto_sync(&state, ra, &row, savesync::When::AfterExit).await
    };
    if let Some(note) = post.note {
        notes.push(note);
    }
    // After the fact there is nothing to ask: the game has already been played.
    // Say so in the notes so progress that has not left the machine is visible.
    if let Some(why) = post.failed {
        notes.push(format!("saves: NOT uploaded — {why}"));
    }
    if !post.conflicts.is_empty() {
        *state.pending_conflicts.lock().map_err(err)? = post.conflicts;
    }

    let prefix = if notes.is_empty() {
        String::new()
    } else {
        format!("{}; ", notes.join("; "))
    };
    Ok(if status.success() {
        format!("{prefix}{} exited cleanly", row.name)
    } else {
        format!("{prefix}{} exited with {status}", row.name)
    })
}

/// One thing on this device that could run the game.
#[derive(serde::Serialize)]
struct AndroidCandidate {
    /// `package/Activity`, ready for an explicit Intent. A leading dot on the
    /// activity is ES-DE's shorthand for "in this package" and is expanded
    /// where the Intent is built.
    component: String,
    /// The core's file name — `gambatte_libretro_android.so`. Not a path: the
    /// directory is RetroArch's own private one and depends on which of its two
    /// package names turns out to be installed, so it is completed on the
    /// Kotlin side where that is known.
    core_file: Option<String>,
    /// ES-DE's name for the emulator, for the toast.
    label: String,
}

/// Everything needed to start a game on Android.
#[derive(serde::Serialize)]
struct AndroidPlan {
    /// The game, so the Kotlin side can name it again when the emulator hands
    /// the screen back — it is the only thing that knows the game has ended.
    id: i64,
    /// Absolute path to the ROM.
    rom: String,
    name: String,
    /// The generated config, or `None` to let RetroArch use its own.
    config: Option<String>,
    /// Anything worth telling the user about what is and is not applied.
    notes: Vec<String>,
    /// Best first; the first one whose component is actually installed wins.
    candidates: Vec<AndroidCandidate>,
}

/// Whether a core file name is the Android build of `core`.
///
/// `opera` is `opera_libretro_android.so`. Compared on the part before
/// `_libretro` and case-insensitively, because the table's own file names are
/// not always the stem plus a suffix — `DoubleCherryGB_libretro_android.so`
/// differs from its stem in case alone, and a case-sensitive compare would
/// quietly drop the user's chosen core back to the default.
fn core_file_is(file: Option<&str>, core: &str) -> bool {
    file.and_then(|f| f.split("_libretro").next())
        .is_some_and(|stem| stem.eq_ignore_ascii_case(core))
}

/// What to start for a game on Android, and with what.
///
/// Android does not launch, it *asks*: RetroArch is a separate app reached by
/// an Intent, so nothing here spawns anything. This returns a plan and the
/// Kotlin side starts it — see `Bridge.startEmulator` in MainActivity.kt, and
/// docs/android-port.md Step 7.
///
/// `launch_rom` cannot serve this. It begins by asking for the RetroArch
/// install, and on Android there is none to find: the app is another package
/// and its cores live in `/data/data/com.retroarch.aarch64/cores`, which is
/// unreadable from here — measured, not assumed. Everything that follows from
/// having that install is unavailable too, and it is worth being plain about
/// what is therefore not happening on this platform yet: no core download, no
/// generated `retroarch.cfg`, no shader or BIOS fetch, no save sync, and no
/// play-time recorded, because `startActivity` returns at once rather than when
/// the game ends. RetroArch uses its own configuration, which on this device
/// already points at the card.
///
/// `retroarch_package` comes from `Bridge.retroArchPackage` and `config_dir`
/// from `Bridge.externalFilesDir`, because only the Kotlin side knows either:
/// which RetroArch is installed decides where its files are, and the generated
/// config has to be written somewhere RetroArch can read — this app's private
/// directory is not it.
///
/// Standalone emulators are offered as well as RetroArch, in ES-DE's own order.
///
/// They used to be dropped, because the manifest declared only the two
/// RetroArch packages and Android 11 makes anything undeclared invisible —
/// `getPackageInfo` throws for a package that is plainly installed. The
/// manifest now declares every package ES-DE knows, so the answer is real and
/// the Kotlin side picks the first that is actually here.
#[tauri::command]
fn android_launch_plan(
    state: State<'_, AppState>,
    id: i64,
    retroarch_package: String,
    config_dir: String,
    pad: Option<String>,
    refresh: Option<f32>,
) -> CmdResult<AndroidPlan> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let rom = row_path(&state, &row).ok_or("not downloaded yet")?;

    // The user's own choice of core, where they made one. The `resolve_core_for`
    // on `AppState` cannot answer here — it starts by unwrapping the RetroArch
    // locator and returns `None` the moment there isn't one — so the shared
    // resolution runs directly with the installed-check left open. Open is the
    // honest answer: which cores RetroArch has is inside its private directory,
    // and this app cannot look.
    let wanted = {
        let overrides = state.core_overrides.lock().map_err(err)?;
        let per_game = state.core_per_game.lock().map_err(err)?;
        coremap::resolve_core_for(
            &state.map,
            &overrides,
            &per_game,
            &row.platform_slug,
            Some(&row.fs_name),
            |_| true,
        )
    };

    let mut found = state.map.android_launches(&row.platform_slug);
    if found.is_empty() {
        // Two different problems, and saying the wrong one sends whoever reads
        // it to the wrong place. An unmapped platform is a gap in
        // `esde-core-map.json`; a mapped one with no libretro emulator is ES-DE
        // being right that nothing libretro runs it.
        return Err(if state.map.knows_platform(&row.platform_slug) {
            format!(
                "ES-DE runs {} in a standalone emulator, and this app does not \
                 start those yet — only RetroArch",
                row.platform_slug
            )
        } else {
            format!(
                "the core map has no entry for {}, so nothing here knows what \
                 would run it",
                row.platform_slug
            )
        });
    }
    // The chosen core ahead of the platform default. A stable sort, so the
    // order `android_launches` decided survives within each group.
    if let Some(core) = wanted.as_deref() {
        found.sort_by_key(|l| !core_file_is(l.core_file.as_deref(), core));
    }

    let (config, notes) = android_config(
        &state, &row, &rom, &retroarch_package, &config_dir, pad.as_deref(), refresh,
    );

    Ok(AndroidPlan {
        id,
        rom: rom.to_string_lossy().into_owned(),
        name: row.name.clone(),
        config,
        notes,
        candidates: found
            .into_iter()
            .map(|l| AndroidCandidate {
                component: l.component,
                core_file: l.core_file,
                label: l.label,
            })
            .collect(),
    })
}

/// Read this system's artwork directories now, so the first game does not.
///
/// The index behind `media::find_local` is built the first time a directory is
/// asked for, and building it means reading a folder of several hundred files
/// off a memory card. That cost lands on whichever game the cursor touches
/// first — and on Android an IPC call blocks the page for as long as the command
/// runs, so it is not a slow pane, it is a frozen app. Measured on a Retroid
/// Pocket Mini: 809ms for the first game in a list, then 67, 29, 26.
///
/// Spawned and not awaited on either side. The reply goes back immediately, so
/// the page never waits on it; the reading happens on a worker while the list is
/// being drawn and the user has not yet moved.
#[tauri::command]
async fn warm_media(state: State<'_, AppState>, platform: String) -> CmdResult<()> {
    let (dir, key) = {
        let cache = state.cache.lock().map_err(err)?;
        let row = cache
            .roms_for(&platform)
            .ok()
            .and_then(|mut v| v.pop());
        match row {
            // Through the same scope a real lookup uses, or the wrong tree is warmed.
            Some(row) => {
                let (d, k) = media_scope(&state, &row);
                (d.to_path_buf(), k.to_owned())
            }
            None => (state.media_dir.clone(), platform.clone()),
        }
    };
    tokio::task::spawn_blocking(move || {
        for (kind, _) in media::ESDE_TYPES {
            let _ = media::dir_index(&dir.join(&key).join(kind));
        }
    });
    Ok(())
}

/// Pull whatever the server has that is newer, before an Android launch.
///
/// The same two-sided sync the desktop does around `plan.run`, split in half
/// because Android has no such moment: `startActivity` returns when the request
/// is accepted, not when the game ends. So the pull happens here and the push
/// happens in [`android_after_play`], which the page calls when RetroArch hands
/// the screen back.
///
/// Returns a note for the toast, or an error the front end turns into the same
/// "play anyway" question the desktop asks. A conflict refuses, for the reason
/// it refuses everywhere: playing on top of a save whose ownership is unsettled
/// is how the loser gets overwritten for good.
#[tauri::command]
async fn android_sync_before(state: State<'_, AppState>, id: i64) -> CmdResult<String> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;
    let Some(ra) = state.retroarch.as_ref() else {
        return Ok(String::new());
    };
    let pre = auto_sync(&state, ra, &row, savesync::When::BeforeLaunch).await;
    if !pre.conflicts.is_empty() {
        *state.pending_conflicts.lock().map_err(err)? = pre.conflicts.clone();
        return Err(format!(
            "SAVE_CONFLICT:{}",
            serde_json::to_string(&pre.conflicts).unwrap_or_default()
        ));
    }
    if let Some(why) = pre.failed {
        return Err(format!("SAVE_OFFLINE:{why}"));
    }
    Ok(pre.note.unwrap_or_default())
}

/// Push whatever changed, after an Android game has been played.
///
/// `seconds` is how long RetroArch held the screen, measured on the Kotlin side
/// between starting the Intent and the activity coming back — there is nothing
/// else to measure it with, since the emulator is another app and tells nobody
/// when it stops.
///
/// Nothing here refuses: the game has already been played. A sync that could not
/// run is a note, not an error, because the alternative is telling someone their
/// progress is stuck after the fact.
#[tauri::command]
async fn android_after_play(
    state: State<'_, AppState>,
    id: i64,
    seconds: i64,
) -> CmdResult<String> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let mut notes = Vec::new();
    // `record_play` ignores anything under a minute, so a game glanced at and
    // closed does not become a play.
    if let Ok(cache) = state.cache.lock()
        && let Ok(true) = cache.record_play(row.id, &moose_rack::util::now_iso(), seconds)
    {
        notes.push(format!("played for {}", moose_rack::util::spell_duration(seconds)));
    }
    if let Some(ra) = state.retroarch.as_ref() {
        let post = auto_sync(&state, ra, &row, savesync::When::AfterExit).await;
        if let Some(note) = post.note {
            notes.push(note);
        }
        if let Some(why) = post.failed {
            notes.push(format!("saves: NOT uploaded — {why}"));
        }
        if !post.conflicts.is_empty() {
            *state.pending_conflicts.lock().map_err(err)? = post.conflicts;
        }
    }
    Ok(notes.join("; "))
}

/// Build the config RetroArch will be launched with, and say what is in it.
///
/// The whole of this app's per-launch settings — hotkeys, shader, saves, rapid
/// fire, achievements, light gun, core options — folded onto the user's own
/// `retroarch.cfg` and written somewhere RetroArch can read.
///
/// Merged rather than replaced, and that is the crux. The Intent takes
/// `CONFIGFILE` and nothing else, and that is the *complete* config: anything
/// the file leaves out falls back to RetroArch's built-in defaults, not to the
/// user's settings. Handing it our fragment alone would quietly reset their
/// video driver, their directories and every binding they have ever set. So
/// their file is the base, ours is laid over it, and their file is only ever
/// read — see `retroarch::merge_config`.
///
/// Never fatal. A config that could not be written means a launch with
/// RetroArch's own settings, which is exactly what every launch did before this
/// existed; refusing to start the game instead would be a worse trade.
fn android_config(
    state: &State<'_, AppState>,
    row: &cache::RomRow,
    rom: &Path,
    package: &str,
    config_dir: &str,
    pad: Option<&str>,
    // The measured display refresh, which decides how many subframes a strobe
    // pass gets. `None` is treated as "probably 120Hz or better".
    refresh: Option<f32>,
) -> (Option<String>, Vec<String>) {
    let mut notes = Vec::new();
    let out_dir = PathBuf::from(config_dir);
    // RetroArch's own files, by the name of the build that is installed. It
    // targets SDK 28, so it is still on legacy storage and reads these itself;
    // this app reaches them because it holds MANAGE_EXTERNAL_STORAGE.
    let ra = moose_rack::retroarch::RetroArch::android_app(&PathBuf::from(format!(
        "/storage/emulated/0/Android/data/{package}/files"
    )));
    let base = match std::fs::read_to_string(ra.data_dir().join("retroarch.cfg")) {
        Ok(text) => text,
        Err(e) => {
            notes.push(format!("using RetroArch's own settings — could not read its config: {e}"));
            return (None, notes);
        }
    };

    let cfg = Config::load().unwrap_or_default();
    let platform = row.platform_slug.as_str();
    let core = {
        let overrides = state.core_overrides.lock().ok();
        let per_game = state.core_per_game.lock().ok();
        match (overrides, per_game) {
            (Some(o), Some(g)) => {
                coremap::resolve_core_for(&state.map, &o, &g, platform, Some(&row.fs_name), |_| true)
            }
            _ => None,
        }
    }
    .unwrap_or_default();

    // The shader for this platform, if the pack is installed. `config_lines`
    // only ever emits a preset it has checked exists, so a device without the
    // pack gets no shader rather than a black screen.
    let preset = state
        .shaders_enabled
        .then(|| {
            let over = state.shader_overrides.lock().ok()?;
            moose_rack::shaders::preset_for(&over, platform)
        })
        .flatten();
    // A strobe pass has to be chained onto the platform's shader, because
    // RetroArch loads exactly one preset. The generated chain lands in the same
    // directory as the config, which is the one RetroArch can read.
    let motion = cfg
        .shaders
        .motion
        .as_deref()
        .filter(|m| !m.is_empty() && *m != "none")
        .filter(|_| moose_rack::shaders::display_of(platform) == moose_rack::shaders::Display::Crt);
    let chained =
        motion.and_then(|m| moose_rack::shaders::write_chained(&ra, &out_dir, preset.as_deref(), m));
    if motion.is_some() && chained.is_none() {
        notes.push("motion shader not installed — using the base shader alone".to_owned());
    }
    let shader_lines = match &chained {
        Some(p) => format!(
            "\n# Base shader with a motion pass chained on.\n\
             video_shader_enable = \"true\"\nvideo_shader = \"{}\"\n\
             video_shader_subframes = \"{}\"\n",
            p.display(),
            moose_rack::shaders::subframes_for(refresh)
        ),
        None => moose_rack::shaders::config_lines(&ra, preset.as_deref()),
    };
    // `config_lines` answers with an explicit `video_shader_enable = "false"`
    // when it cannot find the preset, which is the right thing to write and
    // says nothing to the user. RetroArch on Android ships no shader pack —
    // its `files/` holds a config and nothing else — so this is the ordinary
    // case here rather than an odd one, and silently getting no shader is
    // indistinguishable from the setting not working.
    if preset.is_some() && chained.is_none() && !shader_lines.contains("video_shader = ") {
        notes.push(format!(
            "no shader: RetroArch on this device has no shader pack, so {} could not be applied",
            moose_rack::shaders::label_of(preset.as_deref().unwrap_or_default())
        ));
    }

    let gun = state
        .lightgun
        .lock()
        .ok()
        .map(|g| g.get(platform).map(String::as_str) == Some("on"))
        .unwrap_or(false);

    // Tweaks write core options and remaps into `<root>/retroarch/` and point
    // RetroArch at them. On Android that root has to be shared storage: our own
    // files directory is private and RetroArch cannot read a word of it.
    // Make our preset the one RetroArch's own auto-loader finds first.
    //
    // `video_shader` does not load a shader. On the desktop `--set-shader` does
    // that, and there is no such flag in an Intent — so on Android the only
    // mechanism that applies a preset with content is the automatic one, which
    // `OVERRIDES` turns off. Measured on the Thor: same game, same config, with
    // `auto_shaders_enable = "false"` no shader at all and with it `"true"` a
    // shader appears.
    //
    // Turning it on alone is not enough and is worse than nothing, which is the
    // trap the comment in `OVERRIDES` describes. RetroArch looks for a preset in
    // four places — game, content directory, core, global — and takes the first.
    // Frank's device has a `global.slangp` of thirteen passes left behind by a
    // handheld; with auto-loading on and nothing of ours in a higher slot, that
    // is what every game came up in, and it looks exactly like our shader
    // working.
    //
    // So the chain is written into the *game* slot, which outranks all three of
    // his. His files are not touched: the `.opt` files holding his core settings
    // live in the same tree and are left alone, and the core and global presets
    // still apply to anything launched outside this app.
    let auto_shader = write_game_preset(&ra, &shader_lines, &core, rom);
    let extra = format!(
        "{}{}{}{}{}{}",
        auto_shader,
        shader_lines,
        ra.system_dir_line(),
        ra.prepare_tweaks(&out_dir, platform, &core, state_autofire()),
        moose_rack::achievements::config_lines(&state.achievements),
        moose_rack::lightgun::config_lines(platform, gun),
    );

    let saves_root = moose_rack::util::expand_tilde(&cfg.saves.root);
    let overlay = match ra.write_overrides_full(
        &out_dir,
        None,
        &extra,
        moose_rack::retroarch::Input {
            pad,
            mirror_players: state.mirror_players,
            autofire: state_autofire(),
            autofire_hz: autofire_hz(state),
            save_state_on_exit: cfg.retroarch.save_state_on_exit,
            saves_root: (!saves_root.as_os_str().is_empty()).then_some(saves_root.as_path()),
        },
    ) {
        Ok(path) => std::fs::read_to_string(path).unwrap_or_default(),
        Err(e) => {
            notes.push(format!("using RetroArch's own settings — {e}"));
            return (None, notes);
        }
    };

    let merged = moose_rack::retroarch::merge_config(&base, &overlay);
    let path = out_dir.join("launch.cfg");
    match std::fs::write(&path, merged) {
        Ok(()) => (Some(path.to_string_lossy().into_owned()), notes),
        Err(e) => {
            notes.push(format!("using RetroArch's own settings — could not write the config: {e}"));
            (None, notes)
        }
    }
}

/// Put this launch's shader where RetroArch will look for it, and say so.
///
/// Returns the config lines that switch the automatic loader on, or an empty
/// string when there is no shader to place — in which case auto-loading stays
/// off and his own presets do not creep in.
///
/// The preset is copied rather than referenced because RetroArch resolves an
/// auto preset by name, not by path: it looks for `<config>/<Core>/<game>.slangp`
/// and nothing else will do.
fn write_game_preset(
    ra: &moose_rack::retroarch::RetroArch,
    shader_lines: &str,
    core: &str,
    rom: &Path,
) -> String {
    // No shader for this platform: leave the automatic loader off, so a preset
    // of his does not arrive in place of the nothing we asked for.
    let Some(preset) = shader_lines
        .lines()
        .find_map(|l| l.strip_prefix("video_shader = "))
        .map(|v| v.trim().trim_matches('"'))
        .filter(|v| !v.is_empty())
    else {
        return String::new();
    };
    let (Some(dir), Some(stem)) =
        (ra.core_config_dir(core), rom.file_stem().and_then(|s| s.to_str()))
    else {
        return String::new();
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return String::new();
    }
    // Rewritten, not copied. A `.slangp` names its shaders relative to its own
    // directory, so the file that works in the shader tree refers to nothing
    // once it is sitting in RetroArch's config directory — and RetroArch fails
    // to load it silently, which looks exactly like the shader setting being
    // ignored. `handheld/ags001.slangp` says `shader0 = shaders/mgba/ags001.slang`
    // and that is what came out the other end.
    let Ok(parsed) = moose_rack::slangp::Preset::load(Path::new(preset)) else {
        return String::new();
    };
    let body = moose_rack::slangp::standalone(&parsed);
    let dest = dir.join(format!("{stem}.slangp"));
    if std::fs::write(&dest, body).is_err() {
        return String::new();
    }
    "\n# The shader for this game is in RetroArch's own game slot, which is the\n\
     # only thing that loads a preset with content when there is no command line.\n\
     auto_shaders_enable = \"true\"\n"
        .to_owned()
}

/// Clear the cached media listings when this value goes out of scope.
///
/// A scrape writes new artwork, and the listings are read once per directory
/// per session — so without this a picture fetched now would not appear until
/// the app was restarted. Tied to the scope rather than to a line at the end of
/// the function, because the function has several early returns.
fn scopeguard_forget_media() -> impl Drop {
    struct Forget;
    impl Drop for Forget {
        fn drop(&mut self) {
            moose_rack::esde::forget_media_listings();
            moose_rack::media::forget_dir_indexes();
        }
    }
    Forget
}

/// Rapid fire as configured, or off.
fn state_autofire() -> moose_rack::tweaks::AutoFire {
    Config::load()
        .map(|c| moose_rack::tweaks::AutoFire::parse(&c.retroarch.autofire))
        .unwrap_or_default()
}

/// What one half of the automatic sync produced.
#[derive(Default)]
struct AutoSync {
    /// A line for the launch notes, when anything happened.
    note: Option<String>,
    /// Saves changed on both sides. Non-empty means the user has to choose.
    conflicts: Vec<moose_rack::savesync::SaveConflict>,
    /// Set when the sync could not run at all — server down, DNS gone, timeout.
    /// Distinct from a conflict: nothing is wrong with the save, we simply do
    /// not know whether it is current.
    failed: Option<String>,
}

/// One half of the automatic sync for a single game.
///
/// A sync *failure* never stops anything: an unreachable server must not lock
/// the library, so the problem goes into the launch notes and play continues
/// offline. A *conflict* is different — that is a question only the user can
/// answer, and the caller refuses to launch until they have.
async fn auto_sync(
    state: &State<'_, AppState>,
    ra: &RetroArch,
    row: &cache::RomRow,
    when: savesync::When,
) -> AutoSync {
    if !state.auto_sync {
        return AutoSync::default();
    }
    let Some(client) = state.client.clone() else {
        return AutoSync::default();
    };
    let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();

    // The SQLite connection is not Sync, so the scan takes the lock and drops
    // it before any awaiting starts.
    let candidates = {
        let Ok(cache) = state.cache.lock() else {
            return AutoSync::default();
        };
        match savesync::scan_for_rom(&cache, &state.map, &ra.root, &row.fs_name) {
            Ok(c) => c,
            Err(e) => {
                return AutoSync {
                    failed: Some(format!("could not read the save folder: {e}")),
                    ..Default::default()
                };
            }
        }
    };

    match savesync::run_all(&client, &candidates, &ra.root, Path::new("."), &library_root).await {
        Ok(summary) => AutoSync {
            note: savesync::describe(when, &summary),
            conflicts: summary.conflicts,
            failed: None,
        },
        Err(e) => AutoSync {
            failed: Some(
                e.to_string().lines().next().unwrap_or("the server did not answer").to_owned(),
            ),
            ..Default::default()
        },
    }
}

/// Carry out the user's answer to a conflict, then report what happened.
///
/// Separate from `launch_rom` so the UI can present the choice and then launch
/// again: a Tauri command cannot block waiting for a webview dialog.
#[tauri::command]
async fn resolve_save_conflict(
    state: State<'_, AppState>,
    file_name: String,
    keep: moose_rack::savesync::Keep,
) -> CmdResult<String> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    let ra = state.retroarch.as_ref().ok_or("RetroArch not found")?;
    let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();

    let conflict = {
        let pending = state.pending_conflicts.lock().map_err(err)?;
        pending
            .iter()
            .find(|c| c.file_name == file_name)
            .cloned()
            .ok_or_else(|| format!("no pending conflict for {file_name}"))?
    };

    let outcome = savesync::resolve(
        &client,
        &conflict,
        keep,
        &ra.root,
        &library_root,
        Path::new("."),
    )
    .await
    .map_err(err)?;

    state
        .pending_conflicts
        .lock()
        .map_err(err)?
        .retain(|c| c.file_name != file_name);
    Ok(outcome)
}


/// Every ES-DE set that carries console art, previewed with its own pictures.
///
/// Driven by the art table rather than by matching names against the themes
/// list: the table is keyed by the same `reponame` the list uses, so the two
/// join directly and the prefix-matching that used to sit here — and that had
/// to special-case "Immersive (Revisited)" — is gone.
///
/// The pictures are raw URLs the webview loads itself, so the whole list can be
/// looked at without this process fetching a thing.
#[tauri::command]
async fn icon_sets(state: State<'_, AppState>) -> CmdResult<Vec<iconsets::IconSetView>> {
    // Names and authors are nice to have, not required: with no network the
    // tab still lists every set and still shows its pictures, because the
    // pictures come from the table.
    let listed = theme_remote::list_default().await.unwrap_or_default();
    let slugs: Vec<String> = {
        let cache = state.cache.lock().map_err(err)?;
        cache.platforms().map_err(err)?.into_iter().map(|p| p.fs_slug).collect()
    };
    let active = state.icon_set.lock().map_err(err)?.clone();
    // The consoles actually in this library, under the names ES-DE files them
    // by — a preview of systems the user does not own would be decoration.
    let systems = theme::preview_systems(&state.map, &slugs, 6);

    Ok(moose_rack::iconart::ordered()
        .into_iter()
        .map(|(dir, art)| {
            let entry = listed.iter().find(|t| t.dir_name() == dir);
            let look = art.best_look().map(|l| l.id.as_str()).unwrap_or("");
            iconsets::IconSetView {
                name: entry.map(|t| t.name.clone()).unwrap_or_else(|| iconsets::pretty(&dir)),
                author: entry.map(|t| t.author.clone()).unwrap_or_default(),
                variants: entry.map(|t| t.variants.len()).unwrap_or(0),
                icons: systems.iter().filter_map(|s| art.url(look, s)).collect(),
                kinds: art.looks.iter().map(|l| l.label.clone()).collect(),
                wordmarks_only: art.wordmarks_only(),
                installed: if theme::set_mapping(&state.media_dir, &dir).as_deref()
                    == Some(art.fingerprint().as_str())
                {
                    {
                        let ids: Vec<String> =
                            art.looks.iter().map(|l| l.id.clone()).collect();
                        theme::set_counts(&state.media_dir, &dir, &ids, &slugs)
                            .iter()
                            .map(|(_, n)| n)
                            .sum()
                    }
                } else {
                    // Fetched under a mapping since corrected, so the pictures
                    // are in the wrong folders. Offer it as a download again
                    // rather than as something already in hand.
                    0
                },
                active: active == dir,
                // Recorded here but gone from the published list — still
                // usable, since the pictures are fetched by path.
                missing: !listed.is_empty() && entry.is_none(),
                dir,
            }
        })
        .collect())
}

/// Download one set's console art, from the Icon sets tab.
#[tauri::command]
async fn install_icon_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    dir: String,
) -> CmdResult<String> {
    fetch_icon_set(&app, &state, &dir).await
}

/// Fetch a set's console pictures — the pictures themselves, not the theme.
///
/// One HTTP request per console per style, a few kilobytes each, straight from
/// the theme's repository over `raw.githubusercontent.com`. Anonymous: no
/// account, no token, and not the rate-limited API — `gh` is used to *build*
/// the art table offline, never to read it. Knowing where the files live means
/// never asking for the rest of the theme at all.
///
/// Shared by the Icon sets tab's Download and Appearance's "Get console
/// pictures", so the two cannot drift into fetching different things.
async fn fetch_icon_set(
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
    dir: &str,
) -> CmdResult<String> {
    let art = moose_rack::iconart::of(dir)
        .ok_or_else(|| format!("no artwork recorded for {dir}"))?;
    let slugs: Vec<String> = {
        let cache = state.cache.lock().map_err(err)?;
        cache.platforms().map_err(err)?.into_iter().map(|p| p.fs_slug).collect()
    };
    let http = state
        .client
        .as_ref()
        .map(|c| c.http().clone())
        .unwrap_or(util::http_client(None).map_err(err)?);

    // Start from nothing. A set fetched under an older mapping has pictures in
    // folders this one does not write, and leaving them means "Hardware" goes
    // on showing whatever the previous table filed there.
    let _ = theme::remove_set(&state.media_dir, dir);

    let wanted = theme::esde_names_for(&state.map, &slugs);
    let total: usize = art.looks.len() * wanted.len();
    let mut done = 0usize;
    let mut per_style: Vec<(String, usize)> = Vec::new();

    for look in &art.looks {
        let out = theme::set_dir(&state.media_dir, dir, &look.id);
        std::fs::create_dir_all(&out).map_err(err)?;
        let mut written = 0usize;
        for (slug, names) in &wanted {
            done += 1;
            if done.is_multiple_of(8) {
                let _ = app.emit("icons-progress", format!("{done} of {total}…"));
            }
            // A theme files a console under whichever ES-DE name it knows, so
            // try each rather than assuming our slug is it.
            for name in names {
                let Some(url) = art.url(&look.id, name) else { continue };
                let Ok(resp) = http.get(&url).send().await else { continue };
                if !resp.status().is_success() {
                    continue;
                }
                let Ok(bytes) = resp.bytes().await else { continue };
                if std::fs::write(out.join(format!("{slug}.{}", look.ext)), &bytes).is_ok() {
                    written += 1;
                }
                break;
            }
        }
        if written == 0 {
            // A style folder with nothing in it is one the Select button would
            // land on and show an empty grid. Better it does not exist: the
            // rotation offers what is there rather than being padded out.
            let _ = std::fs::remove_dir_all(&out);
        } else {
            per_style.push((look.label.to_lowercase(), written));
        }
    }

    if per_style.is_empty() {
        let _ = theme::remove_set(&state.media_dir, dir);
        return Err(format!("{dir}: no console pictures could be fetched"));
    }
    // Stamp what this was fetched under, so a corrected table can tell.
    let _ = theme::write_set_mapping(&state.media_dir, dir, &art.fingerprint());
    Ok(per_style.iter().map(|(l, n)| format!("{n} {l}")).collect::<Vec<_>>().join(", "))
}

#[derive(Serialize)]
struct ConfigFinding {
    severity: String,
    what: String,
    note: String,
    fixable: bool,
}

/// A newer published release, or nothing.
///
/// Deliberately reports rather than updates: replacing a running binary needs
/// signing, a rollback and a story for the half-written case. Knowing a new
/// version exists is the part that was missing.
#[tauri::command]
async fn check_update(
    state: State<'_, AppState>,
) -> CmdResult<Option<moose_rack::update::Update>> {
    let http = state
        .client
        .as_ref()
        .map(|c| c.http().clone())
        .unwrap_or(util::http_client(None).map_err(err)?);
    moose_rack::update::check(&http).await.map_err(err)
}

/// What in config.toml no longer says what it used to.
///
/// Read at startup and offered once, because the file is the one part of this
/// app nothing else explains: every renamed key still loads through a
/// compatibility path, so a config can go on carrying a password that is never
/// sent and a rapid-fire boolean that stopped being a boolean, with nothing on
/// screen saying so.
#[tauri::command]
fn config_findings() -> CmdResult<Vec<ConfigFinding>> {
    let Ok(text) = std::fs::read_to_string("config.toml") else { return Ok(Vec::new()) };
    Ok(moose_rack::configpatch::inspect(&text)
        .into_iter()
        .map(|f| ConfigFinding {
            severity: f.severity.to_string(),
            what: f.what,
            note: f.note,
            fixable: f.fix.is_some(),
        })
        .collect())
}

/// Apply the changes that follow from the old values, keeping a copy first.
///
/// The copy is not caution for its own sake: this file holds a server token,
/// and on an old enough config a RetroAchievements password. Losing either to a
/// bad edit would cost more than the stale keys ever did.
#[tauri::command]
fn config_patch() -> CmdResult<String> {
    let path = std::path::Path::new("config.toml");
    let text = std::fs::read_to_string(path).map_err(err)?;
    let (patched, applied) = moose_rack::configpatch::patch(&text);
    if applied.is_empty() {
        return Ok("Nothing to update".to_owned());
    }
    let backup = path.with_extension("toml.before-patch");
    std::fs::copy(path, &backup).map_err(err)?;
    std::fs::write(path, &patched).map_err(err)?;
    let had_password = applied.iter().any(|f| f.what.ends_with("password"));
    Ok(format!(
        "Updated {}: {}. The file as it was is in {}{}",
        applied.len(),
        applied.iter().map(|f| f.what.clone()).collect::<Vec<_>>().join(", "),
        backup.display(),
        if had_password { " — which still has the password in it, so delete it when you are happy" } else { "" }
    ))
}

/// The light gun a game's console has, if any, and whether it is switched on.
///
/// Asked once on the way into a launch so the app can say — once — that the
/// mouse is the trigger. Nothing about a gun is visible otherwise: it writes
/// binds into the launch config and shows up in the notes, which nobody reads
/// before the game starts.
#[tauri::command]
fn game_lightgun(state: State<'_, AppState>, id: i64) -> CmdResult<Option<(String, String)>> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let Some(name) = moose_rack::lightgun::label(&row.platform_slug) else {
        return Ok(None);
    };
    let off = state
        .lightgun
        .lock()
        .map_err(err)?
        .get(&row.platform_slug)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "false" | "no" | "0"))
        .unwrap_or(false);
    Ok((!off).then(|| (row.platform_slug.clone(), name.to_owned())))
}

/// Draw the console grid from one downloaded set, or from the shared pool when
/// `dir` is empty.
#[tauri::command]
fn set_icon_set(state: State<'_, AppState>, dir: String) -> CmdResult<String> {
    moose_rack::config::set_table_entry("config.toml", "icons", "set", &dir).map_err(err)?;
    *state.icon_set.lock().map_err(err)? = dir.clone();
    Ok(if dir.is_empty() {
        "Back to the shared pictures".to_owned()
    } else {
        format!("Console pictures from {dir}")
    })
}

/// Delete a downloaded set's art, and stop drawing from it if it was active.
#[tauri::command]
fn remove_icon_set(state: State<'_, AppState>, dir: String) -> CmdResult<String> {
    theme::remove_set(&state.media_dir, &dir).map_err(err)?;
    let mut active = state.icon_set.lock().map_err(err)?;
    if *active == dir {
        active.clear();
        drop(active);
        moose_rack::config::set_table_entry("config.toml", "icons", "set", "").map_err(err)?;
    }
    Ok(format!("{dir} removed"))
}

/// Fetch the console pictures for whichever set is selected.
///
/// Was: clone four hard-coded themes, strip their per-system art, delete the
/// checkouts — a minute of waiting and hundreds of megabytes to keep a few
/// hundred kilobytes, ignoring the set chosen in Icon sets entirely. The button
/// and the picker disagreed about what the grid should draw.
#[tauri::command]
async fn fetch_icons(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let chosen = state.icon_set.lock().map_err(err)?.clone();
    let set = if chosen.is_empty() { moose_rack::iconart::DEFAULT_SET.to_owned() } else { chosen.clone() };
    let summary = fetch_icon_set(&app, &state, &set).await?;

    // Choosing it as well as fetching it. Pressing "Get console pictures" and
    // seeing nothing change, because the grid was still on the shared pool, is
    // the confusion this answers.
    if chosen.is_empty() {
        moose_rack::config::set_table_entry("config.toml", "icons", "set", &set).map_err(err)?;
        *state.icon_set.lock().map_err(err)? = set.clone();
    }
    Ok(summary)
}

#[derive(Serialize)]
struct EmulatorOption {
    core: String,
    label: String,
    installed: bool,
    /// True for the core ES-DE would pick by default.
    is_default: bool,
}

#[derive(Clone, Serialize)]
struct ShaderOptionView {
    path: String,
    label: String,
    note: String,
}

#[derive(Serialize)]
struct SystemView {
    slug: String,
    name: String,
    rom_count: i64,
    display: String,
    /// Currently selected core and shader, whether defaulted or chosen.
    core: Option<String>,
    shader: Option<String>,
    emulators: Vec<EmulatorOption>,
    shaders: Vec<ShaderOptionView>,
    /// What this console's light gun was called, when it had one. `None` means
    /// no switch is offered for this system.
    gun: Option<String>,
    gun_on: bool,
}

/// Per-system configuration, ES-DE style: every alternative emulator the theme
/// data knows about, plus the shader presets this RetroArch can load.
#[tauri::command]
fn systems(state: State<'_, AppState>) -> CmdResult<Vec<SystemView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.platforms().map_err(err)?
    };
    let ra = state.retroarch.as_ref();

    // Twice, not once per console. `available` checks that each preset really
    // exists, and there are only two answers — one for televisions and one for
    // handhelds — so asking per system meant the same couple of dozen files
    // stat'd thirty-one times over. On the Thor, whose shader pack is on the
    // card, that was most of the three quarters of a second the Emulators tab
    // took to open.
    let options_for = |display| -> Vec<ShaderOptionView> {
        ra.map(|r| {
            shaders::available(r, display)
                .into_iter()
                .map(|o| ShaderOptionView {
                    path: o.path.to_owned(),
                    label: o.label.to_owned(),
                    note: o.note.to_owned(),
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let crt_shaders = options_for(shaders::Display::Crt);
    let handheld_shaders = options_for(shaders::Display::Handheld);

    Ok(rows
        .into_iter()
        .map(|p| {
            let slug = p.fs_slug;
            let default_core = state.map.default_core(&slug);

            // Alternatives come from the ES-DE extraction, so the list matches
            // what ES-DE itself offers for the system.
            let mut emulators: Vec<EmulatorOption> = state
                .map
                .alternatives(&slug)
                .into_iter()
                .map(|core| EmulatorOption {
                    label: state.map.label_for(core).unwrap_or(core).to_owned(),
                    installed: ra.is_some_and(|r| r.has_core(core)),
                    is_default: Some(core) == default_core,
                    core: core.to_owned(),
                })
                .collect();
            // Installed first, then ES-DE's own ordering.
            emulators.sort_by_key(|e| (!e.installed, !e.is_default));

            let display = shaders::display_of(&slug);
            let shader_list = match display {
                shaders::Display::Crt => crt_shaders.clone(),
                shaders::Display::Handheld => handheld_shaders.clone(),
            };

            SystemView {
                gun: moose_rack::lightgun::label(&slug).map(str::to_owned),
                gun_on: state
                    .lightgun
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&slug)
                    .is_some_and(|v| v.trim() == "on"),
                core: resolve_core(&state, &slug),
                shader: if state.shaders_enabled {
                    shaders::preset_for(
                        &state.shader_overrides.lock().unwrap_or_else(|e| e.into_inner()),
                        &slug,
                    )
                } else {
                    None
                },
                display: match display {
                    shaders::Display::Crt => "CRT",
                    shaders::Display::Handheld => "Handheld",
                }
                .to_owned(),
                name: p.display_name,
                rom_count: p.rom_count,
                emulators,
                shaders: shader_list,
                slug,
            }
        })
        .collect())
}

/// Where saves and save states live.
///
/// The `[saves] root` setting, which is the same folder written into the config
/// handed to RetroArch at launch — so what the emulator is told, what the save
/// sync reads, and what the settings page shows are one answer rather than
/// three.
///
/// Every one of these used to take the RetroArch *install* folder instead. That
/// is a reasonable guess for a portable desktop install and wrong everywhere
/// else: it ignored the setting whose entire job is this, and on Android there
/// is no install folder at all, so save sync failed with "RetroArch not found"
/// on a device where RetroArch was installed and the saves folder was set.
///
/// Falls back to the RetroArch root when the setting is somehow unreadable,
/// which keeps the old behaviour rather than inventing a new one.
fn saves_root(state: &State<'_, AppState>) -> PathBuf {
    Config::load()
        .map(|c| moose_rack::util::expand_tilde(&c.saves.root))
        .unwrap_or_else(|_| {
            state
                .retroarch
                .as_ref()
                .map(|ra| ra.root.clone())
                .unwrap_or_else(|| PathBuf::from("Saves"))
        })
}

/// What a sync would do, without doing any of it.
///
/// The first half of the two-step the addon has had all along: ask
/// `/api/sync/negotiate` what it would move, hand the rows back, and transfer
/// nothing until a person has looked at them. A save is the only thing in this
/// app that cannot be fetched again if it goes wrong, and the direction is not
/// a choice anyone can make up front — the server decides it per save, some one
/// way and some the other. So "what would happen" is the only question worth
/// asking first.
///
/// Saves only, because `negotiate` is saves only. Save states are compared
/// against a local ledger by `statesync` and are not in these rows; the page
/// says so rather than letting a plan reading "nothing to sync" be followed by
/// four states moving.
#[tauri::command]
async fn sync_saves_plan(state: State<'_, AppState>) -> CmdResult<syncplan::Review> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    let root = saves_root(&state);

    // Same shape as `sync_saves`: the cache is not Sync, so the scan takes the
    // lock and gives it back before anything is awaited.
    let candidates = {
        let cache = state.cache.lock().map_err(err)?;
        moose_rack::savesync::scan(&cache, &state.map, &root).map_err(err)?
    };

    let (states, _skipped) = moose_rack::savesync::client_states(&candidates);
    let identity = moose_rack::savesync::DeviceIdentity::ensure(&client, Path::new("."))
        .await
        .map_err(err)?;
    let plan = client.negotiate(&identity.device_id, &states).await.map_err(err)?;
    Ok(syncplan::Review::from_plan(&plan))
}

/// One game attract mode could show, and where its media is.
#[derive(Serialize)]
struct AttractPick {
    id: i64,
    name: String,
    platform: String,
    /// A still, following the same chain the grid uses.
    image: Option<String>,
    video: Option<String>,
}

/// Every game with media, for attract mode to draw from.
///
/// Built once and handed over whole. The alternative — asking each game whether
/// it has media as it comes up — is the thing ES-DE went out of its way to warn
/// about, in `generateImageList()`:
///
/// > To instead list all files recursively is much faster as this avoids
/// > `stat()` function calls which are very expensive on such problematic
/// > platforms.
///
/// It applies here harder than it does to them, because the Thor is one of
/// those platforms and this library is large. What makes it cheap is already
/// built: `media::find_local` goes through a per-directory index that is read
/// once and kept until that directory changes, so a lookup is a hash of a
/// lowercased stem rather than a trip to the card.
///
/// The lock is taken to copy the rows out and dropped before any of the media
/// work, and the walk itself runs on a worker. On Android an IPC call blocks
/// the page for as long as the command runs, so a command that reads a few
/// dozen directories has to not hold the interface while it does.
#[tauri::command]
async fn attract_pool(state: State<'_, AppState>) -> CmdResult<Vec<AttractPick>> {
    // The cache is not Sync, so what the media work needs is copied out and the
    // connection released before anything is awaited.
    let wanted: Vec<(i64, String, String, String, PathBuf, String)> = {
        let cache = state.cache.lock().map_err(err)?;
        cache
            .all_roms()
            .map_err(err)?
            .into_iter()
            .map(|row| {
                let (dir, key) = media_scope(&state, &row);
                let (dir, key) = (dir.to_path_buf(), key.to_owned());
                (row.id, row.name, row.platform_slug, row.fs_name, dir, key)
            })
            .collect()
    };

    tokio::task::spawn_blocking(move || {
        let mut pool = Vec::new();
        for (id, name, platform, fs_name, dir, key) in wanted {
            let stem = Path::new(&fs_name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| fs_name.clone());
            let image = media::ART_CHAIN
                .iter()
                .find_map(|kind| media::find_local(&dir, &key, &stem, kind));
            let video = media::find_local(&dir, &key, &stem, "videos");
            // A game with neither is a game attract mode has nothing to show
            // for, and carrying it would mean the sampler drawing blanks.
            if image.is_none() && video.is_none() {
                continue;
            }
            pool.push(AttractPick {
                id,
                name,
                platform,
                image: image.map(|p| moose_rack::util::webview_path(&p)),
                video: video.map(|p| moose_rack::util::webview_path(&p)),
            });
        }
        pool
    })
    .await
    .map_err(err)
}

/// What a finished sync did, and what it refused to guess about.
///
/// `sync_saves` used to answer with a string, which meant the conflicts it
/// found were flattened into a sentence and could not be acted on: the picker
/// that chooses between two copies existed and was only ever reached from a
/// launch. They come back as rows now, and the page hands them to the same
/// dialog.
#[derive(Serialize)]
struct SyncRun {
    headline: String,
    notes: Vec<String>,
    conflicts: Vec<moose_rack::savesync::SaveConflict>,
}

/// Sync saves and save states with the server.
///
/// An explicit action rather than something that happens on launch: a save is
/// the only thing here that cannot be fetched again if it goes wrong, so
/// overwriting one should be a decision, not a side effect.
#[tauri::command]
async fn sync_saves(state: State<'_, AppState>) -> CmdResult<SyncRun> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    // Not RetroArch's install folder: the saves folder. See `saves_root`.
    let root = saves_root(&state);

    // The cache is not Sync, so the scan takes the lock and releases it before
    // any awaiting starts. A future holding the connection across an await
    // cannot be spawned at all.
    let candidates = {
        let cache = state.cache.lock().map_err(err)?;
        moose_rack::savesync::scan(&cache, &state.map, &root).map_err(err)?
    };

    let summary = moose_rack::savesync::run_all(
        &client,
        &candidates,
        &root,
        Path::new("."),
        state.roms_dir.parent().unwrap_or(Path::new(".")),
    )
        .await
        .map_err(err)?;
    Ok(SyncRun {
        headline: summary.headline(),
        notes: summary.notes,
        conflicts: summary.conflicts,
    })
}

/// The motion (strobe/BFI) layer: what is installed, and what is selected.
///
/// Global rather than per-system: it depends on the display, not the console.
/// It chains *onto* whichever shader a system already uses rather than
/// replacing it, so picking one here leaves every per-system choice intact.
#[derive(Serialize)]
struct MotionView {
    current: Option<String>,
    options: Vec<ShaderOptionView>,
}

#[tauri::command]
fn motion_options(state: State<'_, AppState>) -> CmdResult<MotionView> {
    let installed: Vec<ShaderOptionView> = state
        .retroarch
        .as_ref()
        .map(|ra| {
            shaders::MOTION
                .iter()
                .filter(|o| shaders::resolve(ra, o.path).is_some())
                .map(|o| ShaderOptionView {
                    path: o.path.to_owned(),
                    label: o.label.to_owned(),
                    note: o.note.to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(MotionView {
        current: state.motion_shader.lock().map_err(err)?.clone(),
        options: installed,
    })
}

#[tauri::command]
fn set_motion_shader(state: State<'_, AppState>, value: String) -> CmdResult<String> {
    moose_rack::config::set_table_entry("config.toml", "shaders", "motion", &value)
        .map_err(err)?;
    let chosen = (value != "none" && !value.is_empty()).then_some(value);
    *state.motion_shader.lock().map_err(err)? = chosen.clone();
    Ok(match chosen {
        Some(v) => format!("motion layer: {v}"),
        None => "motion layer off".to_owned(),
    })
}

/// Persist a per-system core or shader choice to config.toml.
///
/// Written through a TOML edit rather than a full re-serialise so comments and
/// hand-written sections survive.
#[tauri::command]
fn set_system_choice(
    state: State<'_, AppState>,
    slug: String,
    field: String,
    value: String,
) -> CmdResult<String> {
    let table = match field.as_str() {
        "core" => "cores.overrides",
        "shader" => "shaders.by_platform",
        "lightgun" => "lightgun.by_platform",
        other => return Err(format!("unknown field {other}")),
    };
    moose_rack::config::set_table_entry("config.toml", table, &slug, &value).map_err(err)?;

    // Reflect it in the live copy too, so the next launch uses it without a
    // restart. config.toml remains authoritative on startup.
    match field.as_str() {
        "core" => {
            state.core_overrides.lock().map_err(err)?.insert(slug.clone(), value.clone());
        }
        "shader" => {
            state.shader_overrides.lock().map_err(err)?.insert(slug.clone(), value.clone());
        }
        "lightgun" => {
            state.lightgun.lock().map_err(err)?.insert(slug.clone(), value.clone());
        }
        _ => {}
    }
    Ok(format!("{slug}: {field} = {value}"))
}

/// Cores this game could run under, with the one in force marked.
///
/// Arcade is the reason this is per-game rather than per-platform: a mixed
/// romset has no single correct core, so individual games must be able to
/// escape the platform default.
#[derive(Serialize)]
struct CoreChoice {
    core: String,
    label: String,
    installed: bool,
    /// True for the core this game would launch with right now.
    current: bool,
    /// True when that is because of a per-game override rather than the
    /// platform default.
    pinned: bool,
}

#[tauri::command]
fn game_cores(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<CoreChoice>> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let current = resolve_core_for(&state, &row.platform_slug, Some(&row.fs_name));
    let pinned = state
        .core_per_game
        .lock()
        .map_err(err)?
        .contains_key(&moose_rack::config::game_key(&row.platform_slug, &row.fs_name));

    let mut cores: Vec<String> = state
        .map
        .alternatives(&row.platform_slug)
        .into_iter()
        .map(str::to_owned)
        .collect();
    // The platform default and whatever is in force are always offered, even
    // if ES-DE never listed them for this system.
    for extra in [state.map.default_core(&row.platform_slug).map(str::to_owned), current.clone()]
        .into_iter()
        .flatten()
    {
        if !cores.contains(&extra) {
            cores.push(extra);
        }
    }

    Ok(cores
        .into_iter()
        .map(|core| CoreChoice {
            label: state.map.label_for(&core).unwrap_or(&core).to_owned(),
            installed: state.retroarch.as_ref().is_some_and(|ra| ra.has_core(&core)),
            current: current.as_deref() == Some(core.as_str()),
            pinned: pinned && current.as_deref() == Some(core.as_str()),
            core,
        })
        .collect())
}

/// Pin a core to one game, or clear the pin with an empty `core`.
#[tauri::command]
fn set_game_core(state: State<'_, AppState>, id: i64, core: String) -> CmdResult<String> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let key = moose_rack::config::game_key(&row.platform_slug, &row.fs_name);
    if core.is_empty() {
        moose_rack::config::clear_table_entry("config.toml", "cores.per_game", &key)
            .map_err(err)?;
    } else {
        moose_rack::config::set_table_entry("config.toml", "cores.per_game", &key, &core)
            .map_err(err)?;
    }

    let mut live = state.core_per_game.lock().map_err(err)?;
    if core.is_empty() {
        live.remove(&key);
        // Clearing removes the hand-picked core, not the shipped one. For the
        // arcade romsets in the compiled-in table the platform default is a
        // core that was *measured* not to run them, so dropping all the way
        // back to it would be a broken state that returned on the next start
        // anyway, since load folds the table back in.
        if let Some(shipped) = moose_rack::config::arcade_core_map().remove(&key) {
            let msg = format!("{}: back to {shipped}, the core known to run it", row.name);
            live.insert(key, shipped);
            return Ok(msg);
        }
        return Ok(format!("{}: back to the {} default", row.name, row.platform_slug));
    }
    live.insert(key, core.clone());
    Ok(format!("{}: pinned to {core}", row.name))
}

/// Copy system logos out of an installed ES-DE theme into the media tree.
#[tauri::command]
fn install_theme_logos(state: State<'_, AppState>) -> CmdResult<String> {
    let slugs: Vec<String> = {
        let cache = state.cache.lock().map_err(err)?;
        cache.platforms().map_err(err)?.into_iter().map(|p| p.fs_slug).collect()
    };
    let themes = theme::discover_with(state.theme_root.as_deref(), Some(&state.themes_dir));
    if themes.is_empty() {
        return Err("no ES-DE themes found — install ES-DE or set [theme] root".into());
    }
    let n = theme::install(&themes, &state.map, &slugs, &state.media_dir).map_err(err)?;
    Ok(format!("installed {n} logos from {}", themes[0].name))
}

#[derive(Serialize)]
struct Status {
    server: String,
    connected: bool,
    /// False when there is no config.toml at all — a different problem from a
    /// server that will not answer, and worth saying so in the UI.
    configured: bool,
    retroarch: Option<String>,
    cores_installed: usize,
    roms_cached: i64,
    /// Absolute paths, shown in the UI so downloaded data is never a mystery.
    roms_dir: String,
    media_dir: String,
    /// Directory every relative path is resolved against, and therefore where
    /// `config.toml` is read from. Reported because "it cannot find my config"
    /// is otherwise unanswerable from inside the app.
    data_dir: String,
    config_path: String,
    /// True when the app has no config.toml and is sitting in a directory that
    /// already holds other things. It is about to create a library folder, a
    /// cache and a config here, and dropping all that into someone's Downloads
    /// or Desktop is rude.
    crowded_folder: bool,
    /// What else is in there, for the warning to name.
    folder_entries: usize,
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> CmdResult<Status> {
    let cache = state.cache.lock().map_err(err)?;
    Ok(Status {
        server: state
            .client
            .as_ref()
            .map(|c| c.base().to_owned())
            .unwrap_or_default(),
        connected: state.client.is_some(),
        configured: Config::exists("config.toml"),
        retroarch: state
            .retroarch
            .as_ref()
            .map(|r| r.root.display().to_string()),
        cores_installed: state
            .retroarch
            .as_ref()
            .map(|r| r.installed_cores().len())
            .unwrap_or(0),
        roms_cached: cache.rom_count().unwrap_or(0),
        roms_dir: abs(&state.roms_dir),
        media_dir: abs(&state.media_dir),
        data_dir: abs(Path::new(".")),
        config_path: abs(Path::new("config.toml")),
        crowded_folder: !Config::exists("config.toml") && neighbours() > 2,
        folder_entries: neighbours(),
    })
}

/// How much space the library and its artwork take up.
///
/// Its own command because it is the only slow thing `status` did, and on
/// Android slow is not merely slow. Tauri's IPC there is a `fetch` to a custom
/// protocol that the webview serves on the calling thread, so a command that
/// takes four seconds freezes the page for four seconds — the whole settings
/// window, mid-click.
///
/// And it had become four seconds. `roms_dir` is the ES-DE library now, so this
/// walks the card: 9,658 games, stat by stat, over FUSE. It used to be cheap
/// only because the config was failing to parse and the path fell back to an
/// empty `./library/roms`.
///
/// Nothing waits on it. The one place it is shown fills the figure in when it
/// arrives.
#[tauri::command]
async fn disk_usage(state: State<'_, AppState>) -> CmdResult<u64> {
    Ok(util::dir_size(&state.roms_dir) + util::dir_size(&state.media_dir))
}

/// Point the app at a RetroArch install by hand.
///
/// The automatic search only knows conventional locations — `/Applications`,
/// `C:/Program Files/RetroArch` and so on — which is no help when the install
/// lives somewhere like `E:/Emulators/RetroArch`. The path is verified before
/// it is written, so a typo fails here rather than at the next launch.
///
/// `state.retroarch` is resolved once at startup and is not behind a lock — it
/// is read inside an async command, where holding one across an await would be
/// a hazard — so the new path applies on the next launch of the app rather than
/// immediately. The returned string says so.
#[tauri::command]
fn set_retroarch_root(path: String) -> CmdResult<String> {
    let path = path.trim().to_owned();

    if path.is_empty() {
        // Empty means "go back to probing the usual places".
        moose_rack::config::clear_table_entry("config.toml", "retroarch", "root").map_err(err)?;
        return Ok("Cleared. The usual locations will be searched again after a restart.".into());
    }

    // Verify before writing, so a typo fails here rather than at the next launch.
    let found = RetroArch::locate(Some(&path)).map_err(|e| e.to_string())?;
    moose_rack::config::set_table_entry("config.toml", "retroarch", "root", &path)
        .map_err(err)?;

    Ok(format!(
        "Found {} with {} cores. Restart to use it.",
        found.binary.display(),
        found.installed_cores().len()
    ))
}

/// Which of the shared pool's three kinds best matches the chosen look.
///
/// Only for the pool under `_platforms/<kind>/`, which predates looks and still
/// has exactly hardware, controllers and wordmarks. A downloaded set is asked
/// for by look id and never comes through here.
fn current_style(state: &State<'_, AppState>) -> theme::IconStyle {
    let look = state.icon_look.lock().map(|l| l.clone()).unwrap_or_default();
    if look.starts_with("hardware") {
        theme::IconStyle::SystemArt
    } else if look.starts_with("controller") {
        theme::IconStyle::Controller
    } else {
        theme::IconStyle::Logo
    }
}

#[derive(Serialize)]
struct IconStyleView {
    key: String,
    label: String,
    /// How many of our platforms have art in this style.
    available: usize,
    selected: bool,
}

/// Every look available, with how many consoles each covers.
///
/// The chosen set's own looks first, then everything in the shared pool that
/// still holds pictures. Both, not either: the pool is what earlier versions
/// downloaded and it can hold looks no set offers — a user with `consolegame`
/// and `systemart_legacy` on disk had 24 pictures in each, and offering only
/// the set's two took those out of the rotation without deleting the files.
///
/// Nothing here is a fixed list. A theme with nine looks contributes nine.
#[tauri::command]
fn icon_styles(state: State<'_, AppState>) -> CmdResult<Vec<IconStyleView>> {
    let slugs: Vec<String> = {
        let cache = state.cache.lock().map_err(err)?;
        cache.platforms().map_err(err)?.into_iter().map(|p| p.fs_slug).collect()
    };
    let set = state.icon_set.lock().map_err(err)?.clone();
    let cur = state.icon_look.lock().map_err(err)?.clone();
    let mut out: Vec<IconStyleView> = Vec::new();

    if let Some(art) = moose_rack::iconart::of(&set) {
        let ids: Vec<String> = art.looks.iter().map(|l| l.id.clone()).collect();
        for (look, (_, available)) in
            art.looks.iter().zip(theme::set_counts(&state.media_dir, &set, &ids, &slugs))
        {
            out.push(IconStyleView {
                key: look.id.clone(),
                label: look.label.clone(),
                available,
                selected: look.id == cur,
            });
        }
    }

    for (key, available) in theme::pool_looks(&state.media_dir, &slugs) {
        // A pool folder whose name matches a look of the chosen set is the same
        // choice twice; the set's own wins because it is the one being drawn.
        if out.iter().any(|v| v.key == key) {
            continue;
        }
        out.push(IconStyleView {
            label: theme::pool_label(&key),
            selected: key == cur,
            key,
            available,
        });
    }
    Ok(out)
}

/// Draw the grid in one of the chosen set's looks.
#[tauri::command]
fn set_icon_style(state: State<'_, AppState>, key: String) -> CmdResult<String> {
    let set = state.icon_set.lock().map_err(err)?.clone();

    // A look belonging to the chosen set, or a folder in the shared pool that
    // actually holds pictures. Anything else is refused rather than stored: an
    // unknown id is a folder that does not exist, and the grid would go blank.
    let label = match moose_rack::iconart::of(&set).and_then(|a| a.look(&key).cloned()) {
        Some(look) => look.label,
        None => {
            let slugs: Vec<String> = {
                let cache = state.cache.lock().map_err(err)?;
                cache.platforms().map_err(err)?.into_iter().map(|p| p.fs_slug).collect()
            };
            if theme::pool_looks(&state.media_dir, &slugs).iter().any(|(k, _)| *k == key) {
                theme::pool_label(&key)
            } else {
                return Err(format!("{key} is not a look anything on this machine has"));
            }
        }
    };

    *state.icon_look.lock().map_err(err)? = key.clone();
    // Persist it, or the grid silently reverts on next launch.
    moose_rack::config::set_table_entry("config.toml", "icons", "style", &key).map_err(err)?;
    Ok(label)
}

/// One row in the app-icon picker.
#[derive(serde::Serialize)]
struct AppIconView {
    id: String,
    label: String,
    /// Absolute path to the preview picture, for `convertFileSrc`. Empty when
    /// the built files are missing, which the picker draws as a gap rather
    /// than a broken image.
    preview: String,
    selected: bool,
}

/// Where an icon's built files are: the bundle's resources once installed,
/// `assets/appicons/built` in a checkout.
///
/// Both are tried every time rather than deciding once, because a dev build run
/// from a checkout has a resource directory that exists and does not contain
/// them — so "which am I" is not a question with a stable answer.
fn appicon_dir(app: &tauri::AppHandle, id: &str) -> Option<PathBuf> {
    use tauri::Manager;
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        // Tauri maps `../assets/...` to `_up_/assets/...` inside Resources.
        roots.push(res.join("_up_").join("assets").join("appicons").join("built"));
        roots.push(res.join("assets").join("appicons").join("built"));
    }
    roots.push(PathBuf::from("assets/appicons/built"));
    roots.into_iter().map(|r| r.join(id)).find(|d| d.is_dir())
}

/// Every icon this build ships, with the chosen one marked.
#[tauri::command]
fn app_icons(app: tauri::AppHandle) -> CmdResult<Vec<AppIconView>> {
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    let chosen = moose_rack::appicon::chosen(cfg.appearance.app_icon.as_deref());
    Ok(moose_rack::appicon::ICONS
        .iter()
        .map(|icon| AppIconView {
            id: icon.id.to_string(),
            label: icon.label.to_string(),
            preview: appicon_dir(&app, icon.id)
                .map(|d| d.join(moose_rack::appicon::PREVIEW_NAME))
                .filter(|p| p.is_file())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            selected: icon.id == chosen.id,
        })
        .collect())
}

/// Wear a different icon.
///
/// Returns what to tell the user, which is not the same sentence everywhere:
/// the window icon changes under you on Windows and Linux, whereas macOS reads
/// the Dock icon out of the bundle once, so there the file is rewritten and the
/// change lands on the next launch.
#[tauri::command]
fn set_app_icon(app: tauri::AppHandle, id: String) -> CmdResult<String> {
    let icon = moose_rack::appicon::set(&id).map_err(err)?;
    let dir = appicon_dir(&app, icon.id)
        .ok_or_else(|| format!("{} has no built files — run scripts/build-appicons.sh", icon.id))?;
    Ok(apply_app_icon(&app, icon.id, &dir))
}

/// Put the icon on, as far as this OS allows, and say what happened.
///
/// Three arms, and they are `desktop`/`mobile` rather than `target_os` on
/// purpose. `not(target_os = "macos")` used to cover this, which put Android in
/// the Windows-and-Linux branch — and that branch calls `set_icon`, which Tauri
/// only gives a desktop window. It failed to compile, which was the lucky
/// outcome; the same pattern elsewhere in this app compiles and is wrong.
fn apply_app_icon(app: &tauri::AppHandle, id: &str, dir: &Path) -> String {
    let _ = (app, id, dir);

    #[cfg(all(desktop, not(target_os = "macos")))]
    {
        use tauri::Manager;
        let png = dir.join(moose_rack::appicon::WINDOW_NAME);
        if let Some(win) = app.get_webview_window("main") {
            match tauri::image::Image::from_path(&png) {
                Ok(img) => {
                    if win.set_icon(img).is_ok() {
                        return "Icon changed.".into();
                    }
                }
                Err(e) => return format!("Saved, but the icon would not load: {e}"),
            }
        }
        "Saved. It will be worn from the next launch.".into()
    }

    #[cfg(target_os = "macos")]
    {
        // macOS reads Contents/Resources/icon.icns when the bundle is launched
        // and never again, so the only honest way to change it is to replace
        // that file. Nothing else in the bundle is touched.
        match mac_bundle_icns() {
            Some(dest) => {
                let src = dir.join(moose_rack::appicon::icns_name(id));
                match std::fs::copy(&src, &dest) {
                    // Finder caches icons by bundle mtime; without this the old
                    // one can persist in the Dock for a surprisingly long time.
                    // `touch` rather than a crate: one line, already installed.
                    Ok(_) => {
                        if let Some(bundle) = dest.parent().and_then(|p| p.parent()) {
                            let _ = std::process::Command::new("/usr/bin/touch")
                                .arg(bundle)
                                .status();
                        }
                        "Icon changed — it will show after the app is restarted.".into()
                    }
                    Err(e) => format!(
                        "Saved, but the app bundle could not be written ({e}). \
                         The icon will be right in the next build."
                    ),
                }
            }
            // Running from `cargo run` rather than a bundle: nothing to rewrite.
            None => "Saved. It will be worn by the next build of the app.".into(),
        }
    }

    // Android has no window icon to set and no bundle to rewrite. The launcher
    // icon is baked into the APK at build time and an installed app cannot
    // change it, so the choice is remembered and nothing else can honestly be
    // claimed. Saying "Icon changed" here would be a lie the user can see.
    #[cfg(mobile)]
    {
        "Saved. Android takes its launcher icon from the installed app, so this \
         one will be worn by the next build."
            .into()
    }
}

/// `…/Moose Rack.app/Contents/Resources/icon.icns` for the running bundle, or
/// `None` when this is not a bundle at all.
#[cfg(target_os = "macos")]
fn mac_bundle_icns() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // Contents/MacOS/<exe> → Contents
    let contents = exe.parent()?.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let icns = contents.join("Resources").join("icon.icns");
    icns.is_file().then_some(icns)
}

/// How many things sit alongside the app, ignoring what the app itself puts
/// there.
///
/// The app's own executable does not count, and neither does anything it
/// created — a second run should not report the folder as crowded because of
/// the library it made on the first.
fn neighbours() -> usize {
    const OURS: &[&str] = &[
        "library", "cache.sqlite3", "config.toml", "data", "state.json",
        "states-seen.json", "crash.log", "saves-backup",
    ];
    let Ok(entries) = std::fs::read_dir(".") else { return 0 };
    entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.starts_with('.') || OURS.contains(&name.as_str()) {
                return false;
            }
            // The app itself, under any of the names it ships as.
            !(name.ends_with(".exe") && name.contains("moose"))
                && !name.starts_with("moose-rack")
                && !name.starts_with("moose-gui")
                && !name.starts_with("moose-rack-cli")
                && !name.ends_with(".app")
        })
        .count()
}

fn abs(p: &Path) -> String {
    // Through the same helper the pictures use: on Windows `canonicalize`
    // hands back \\?\C:\... and the status card would print that at somebody
    // as the answer to "where is my config".
    moose_rack::util::webview_path(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
}


// --- helpers -------------------------------------------------------------

/// Not a file-only test: a multi-disc game is a directory of discs plus the
/// `.m3u` that orders them, and RomM's own layout is what puts it there.
fn local_path(state: &State<'_, AppState>, platform: &str, fs_name: &str) -> Option<PathBuf> {
    let p = state.roms_dir.join(platform).join(fs_name);
    (p.is_file() || p.is_dir()).then_some(p)
}

/// Where a row's file actually is.
///
/// A locally scanned ES-DE game records its own absolute path — it lives on
/// whatever disk the library is on, which `<roms_dir>/<slug>/<file>` cannot
/// express.
fn row_path(state: &State<'_, AppState>, row: &cache::RomRow) -> Option<PathBuf> {
    if let Some(p) = row.local_path.as_deref().map(PathBuf::from)
        && (p.is_file() || p.is_dir())
    {
        return Some(p);
    }
    local_path(state, &row.platform_slug, &row.fs_name)
}

/// Artwork directory and key to look under.
///
/// ES-DE keys media by its own system name (`genesis`, `neogeo`), while the
/// downloaded RomM tree keys by slug — so the pair must travel together.
fn media_scope<'a>(state: &'a State<'_, AppState>, row: &'a cache::RomRow) -> (&'a Path, &'a str) {
    match (state.esde_media.as_deref(), row.esde_system.as_deref()) {
        (Some(dir), Some(system)) => (dir, system),
        _ => (state.media_dir.as_path(), row.platform_slug.as_str()),
    }
}

fn resolve_core(state: &State<'_, AppState>, platform: &str) -> Option<String> {
    resolve_core_for(state, platform, None)
}

/// As [`resolve_core`], honouring a per-game override when the file is known.
fn resolve_core_for(
    state: &State<'_, AppState>,
    platform: &str,
    fs_name: Option<&str>,
) -> Option<String> {
    let ra = state.retroarch.as_ref()?;
    let overrides = state.core_overrides.lock().ok()?;
    let per_game = state.core_per_game.lock().ok()?;
    coremap::resolve_core_for(&state.map, &overrides, &per_game, platform, fs_name, |c| {
        ra.has_core(c)
    })
}

/// Write panics somewhere findable before the process dies.
///
/// With no console attached, a panic in a release build produces no output at
/// all. This is the difference between "it does nothing when I double-click"
/// and a file naming the line that failed.
fn install_panic_log() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Beside the app, in the data directory `anchor_to_data_root` chose —
        // not in the home directory. Nothing is created: if that location is
        // not writable the panic still reaches stderr, which is no worse than
        // before and does not leave a folder behind.
        let path = PathBuf::from("crash.log");
        if std::fs::write(&path, format!("{info}\n")).is_ok() {
            let shown = path.canonicalize().unwrap_or(path);
            eprintln!("panic written to {}", shown.display());
        }
        previous(info);
    }));
}

/// The macOS menu bar, replaced only so the About panel says something.
///
/// The standard panel is what people press first — it is one item under the
/// app's own name — and it showed a version twice and nothing else. Who wrote
/// this and where the source is belong there rather than three clicks into a
/// settings window.
///
/// Rebuilding the app menu means rebuilding the standard items around it, so
/// they are all listed: without them, Cmd-Q, Cmd-C and Hide simply stop
/// existing, which is a far worse trade than a plain About panel.
#[cfg(target_os = "macos")]
fn install_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadataBuilder, MenuBuilder, SubmenuBuilder};

    let about = AboutMetadataBuilder::new()
        .name(Some("Moose Rack"))
        .version(Some(env!("CARGO_PKG_VERSION")))
        .credits(Some(
            "by mizaimao\n\ngithub.com/mizaimao/moose-rack\n\nIcons by Lucide (ISC)",
        ))
        .build();

    let app_menu = SubmenuBuilder::new(app, "Moose Rack")
        .about(Some(about))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    // Without an Edit menu the text fields in Settings lose cut, copy, paste
    // and select-all — on macOS those are menu items first and shortcuts
    // second.
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .separator()
        .close_window()
        .build()?;

    app.set_menu(MenuBuilder::new(app).items(&[&app_menu, &edit, &window]).build()?)?;
    Ok(())
}


// ---------------------------------------------------------------------------
// The interface logic, which lives in `moose_rack` rather than here.
//
// Sort orders, filter predicates, the order of the left column, page filtering
// and binding resolution were all written in the webview, which meant the TUI
// could not read a keybinding and an SDL front end would have had to
// reimplement every rule from the comments. The commands below are a thin
// window onto `moose_rack::{binds, gamelist, gamesort, gamefilter,
// pickorder, pagefilter, gridnav}` — no decision is taken in this file.
//
// One exception, deliberate: the controller poll. It runs inside
// `requestAnimationFrame`, 120 times a second, and a round trip per frame is
// not a thing that can be made fast enough. `ui/js/gamepad.js` keeps the
// arithmetic and `moose_rack::padpoll` is the definition it copies.
// ---------------------------------------------------------------------------

use moose_rack::{binds, gamefilter, gamelist, gamesort, gridnav, pagefilter, pickorder};

impl RomView {
    /// The nine facts a list needs to order and narrow itself.
    fn as_row(&self) -> gamelist::Row {
        gamelist::Row {
            id: self.id,
            name: self.name.clone(),
            platform: self.platform.clone(),
            downloaded: self.downloaded,
            favorite: self.favorite,
            rating: self.rating,
            year: self.year,
            last_played: self.last_played.clone(),
            size_bytes: self.size_bytes,
            players: self.players,
        }
    }
}

/// Which list a question is about. The three fields a front end already has.
#[derive(Debug, Deserialize)]
struct ListRef {
    view: String,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    collection: Option<String>,
}

impl ListRef {
    fn scope(&self) -> String {
        gamelist::scope(&self.view, self.platform.as_deref(), self.collection.as_deref())
    }
}

/// Everything a front end needs to draw and dispatch bindings.
///
/// One call rather than four, because the page caches the result and re-reads
/// it only when something is rebound — the previous version resolved the pad
/// map inside the poll loop, which was a storage read, a parse and a scan of
/// the result at 120Hz forever, and the app's largest idle cost.
#[derive(Serialize)]
struct BindingsView {
    actions: &'static [binds::Action],
    pad_buttons: &'static [binds::PadButton],
    /// Button index -> action, `null` where a rebind cleared the button.
    ///
    /// The nulls are kept rather than dropped because "bound to nothing" and
    /// "not a button on this pad" are different answers to why a press did
    /// nothing, and the settings window has to be able to say which.
    pad_map: std::collections::BTreeMap<u8, Option<String>>,
    /// Action -> key, `null` when unbound.
    keys: std::collections::BTreeMap<String, Option<String>>,
    /// Action -> the button's own name, for the help page.
    pad_labels: std::collections::BTreeMap<String, String>,
    key_labels: std::collections::BTreeMap<String, String>,
}

fn bindings_view(b: &binds::Bindings) -> BindingsView {
    // The face-button swap, applied where the pad is *read*.
    //
    // `pad_map_swapped` has existed since the setting did and was called by
    // nothing outside its own tests, so `[controllers] swap_ab` sat in
    // config.toml doing precisely nothing. Applied here rather than by
    // rebinding every action, which is the same fix done twenty times and
    // leaves the pad looking rebound to anyone who opens the list.
    //
    // Read per call rather than held: it is a setting somebody changes and
    // expects to take, and this is called again after every rebind anyway.
    let cfg = Config::load().unwrap_or_default();
    BindingsView {
        actions: binds::ACTIONS,
        pad_buttons: binds::PAD_BUTTONS,
        pad_map: b.pad_map_swapped(cfg.controllers.swap_ab, cfg.controllers.swap_xy),
        keys: binds::ACTIONS.iter().map(|a| (a.id.to_owned(), b.key_for(a.id))).collect(),
        pad_labels: binds::ACTIONS
            .iter()
            .map(|a| (a.id.to_owned(), binds::pad_label(b.pad_for(a.id))))
            .collect(),
        key_labels: binds::ACTIONS
            .iter()
            .map(|a| (a.id.to_owned(), binds::key_label(b.key_for(a.id).as_deref())))
            .collect(),
    }
}

/// Write the whole binding table back to `config.toml`.
///
/// All of it rather than the one key that changed, because a rebind clears
/// whichever key or button previously held the action — so a single press
/// changes two entries, and writing one of them leaves a config where two
/// things claim the same key.
///
/// Two passes over the file, not forty-five: `set_table_entries` takes the
/// whole table at once. Done a key at a time, one press of a rebind button was
/// forty-five read-modify-writes of config.toml.
fn save_bindings(b: &binds::Bindings) -> Result<(), String> {
    use moose_rack::config::set_table_entries;
    let keys: Vec<(String, Option<String>)> = binds::ACTIONS
        .iter()
        .map(|a| (a.id.to_owned(), b.keys.get(a.id).cloned()))
        .collect();
    set_table_entries("config.toml", "bindings.keys", &keys).map_err(err)?;

    let pad: Vec<(String, Option<String>)> = binds::PAD_BUTTONS
        .iter()
        .map(|p| {
            let index = p.index.to_string();
            let held = b.pad.get(&index).cloned();
            (index, held)
        })
        .collect();
    set_table_entries("config.toml", "bindings.pad", &pad).map_err(err)
}

#[tauri::command]
fn ui_bindings(state: State<'_, AppState>) -> CmdResult<BindingsView> {
    let b = state.bindings.lock().map_err(err)?;
    Ok(bindings_view(&b))
}

/// Bind a key to an action. A null key unbinds it.
#[tauri::command]
fn set_key_binding(
    state: State<'_, AppState>,
    action: String,
    key: Option<String>,
) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    b.set_key(&action, key.as_deref());
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

/// Bind a controller button to an action. A null index unbinds it.
#[tauri::command]
fn set_pad_binding(
    state: State<'_, AppState>,
    action: String,
    index: Option<u8>,
) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    b.set_pad(&action, index);
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

/// Put the defaults back. `which` is "keys" or "pad" — the two are reset
/// separately because they are rebound separately, and somebody undoing a
/// controller experiment rarely means to lose their keyboard as well.
#[tauri::command]
fn reset_bindings(state: State<'_, AppState>, which: String) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    match which.as_str() {
        "pad" => b.reset_pad(),
        _ => b.reset_keys(),
    }
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

/// Adopt bindings a previous version left in the webview's own storage.
///
/// Runs once. Before this the two tables lived in `localStorage`, which the
/// TUI cannot read and which the settings window — a second document — kept
/// its own copy of. Anything already in `config.toml` wins, so a second run
/// after somebody has rebound something cannot undo it.
#[tauri::command]
fn import_bindings(
    state: State<'_, AppState>,
    keys: std::collections::BTreeMap<String, Option<String>>,
    pad: std::collections::BTreeMap<String, Option<String>>,
) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    b.adopt(keys, pad);
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

/// The orders and filters a menu offers, with their labels.
#[derive(Serialize)]
struct ListControls {
    orders: &'static [gamesort::Order],
    filters: &'static [gamefilter::Filter],
}

#[tauri::command]
fn list_controls() -> ListControls {
    ListControls { orders: gamesort::ORDERS, filters: gamefilter::FILTERS }
}

/// Which rows to draw, in what order, and what the two buttons should say.
#[derive(Serialize)]
struct Arrangement {
    /// Row ids, narrowed and ordered. `null` when the backend is not holding
    /// this list — the caller then draws what it has rather than an order
    /// computed from somebody else's rows.
    ids: Option<Vec<i64>>,
    order: &'static str,
    order_label: &'static str,
    filters: Vec<String>,
    sortable: bool,
    filterable: bool,
}

fn arrangement(state: &State<'_, AppState>, list: &ListRef) -> Result<Arrangement, String> {
    let scope = list.scope();
    let chosen = state.chosen.lock().map_err(err)?;
    let order = chosen.order(&scope);
    let filters = chosen.filters(&scope);
    let held = state.list_rows.lock().map_err(err)?;
    let ids = (*state.list_scope.lock().map_err(err)? == scope)
        .then(|| gamelist::arrange(&held, order.id, &filters));
    Ok(Arrangement {
        ids,
        order: order.id,
        order_label: order.label,
        filters: filters.into_iter().collect(),
        sortable: gamelist::sortable(&list.view),
        filterable: gamelist::filterable(&list.view),
    })
}

#[tauri::command]
fn arrange_list(state: State<'_, AppState>, list: ListRef) -> CmdResult<Arrangement> {
    arrangement(&state, &list)
}

/// Choose an order for this list. `preferred` sets it only if nothing has been
/// chosen yet, which is how Continue playing opens most-recent-first without
/// overriding somebody who asked for something else.
#[tauri::command]
fn set_list_order(
    state: State<'_, AppState>,
    list: ListRef,
    order: String,
    preferred: Option<bool>,
) -> CmdResult<Arrangement> {
    {
        let mut chosen = state.chosen.lock().map_err(err)?;
        let scope = list.scope();
        if preferred.unwrap_or(false) {
            chosen.default_order(&scope, &order);
        } else {
            chosen.set_order(&scope, &order);
        }
    }
    arrangement(&state, &list)
}

/// Step to the next order without opening a menu, for the stick click.
#[tauri::command]
fn cycle_list_order(
    state: State<'_, AppState>,
    list: ListRef,
    delta: i32,
) -> CmdResult<Arrangement> {
    {
        let mut chosen = state.chosen.lock().map_err(err)?;
        let scope = list.scope();
        let next = gamesort::cycle(chosen.order(&scope).id, delta);
        chosen.set_order(&scope, next.id);
    }
    arrangement(&state, &list)
}

#[tauri::command]
fn toggle_list_filter(
    state: State<'_, AppState>,
    list: ListRef,
    filter: String,
) -> CmdResult<Arrangement> {
    state.chosen.lock().map_err(err)?.toggle_filter(&list.scope(), &filter);
    arrangement(&state, &list)
}

#[tauri::command]
fn clear_list_filters(state: State<'_, AppState>, list: ListRef) -> CmdResult<Arrangement> {
    state.chosen.lock().map_err(err)?.clear_filters(&list.scope());
    arrangement(&state, &list)
}

/// The left column: which entries to draw, in what order, and what the button
/// above them should say.
#[derive(Serialize)]
struct PickerArrangement {
    /// Indices into the rows that were handed over.
    order: Vec<usize>,
    /// The orders this kind of list offers. Empty for consoles, which get the
    /// alphabet and no button.
    orders: &'static [pickorder::PickerOrder],
    chosen: Option<&'static str>,
    label: Option<&'static str>,
}

#[tauri::command]
fn sort_picker(
    state: State<'_, AppState>,
    kind: String,
    rows: Vec<pickorder::PickerRow>,
) -> CmdResult<PickerArrangement> {
    let orders = state.picker_order.lock().map_err(err)?;
    let chosen = orders.get(&kind);
    Ok(PickerArrangement {
        order: pickorder::sort(&rows, chosen.map(|o| o.id)),
        orders: pickorder::orders_for(&kind),
        chosen: chosen.map(|o| o.id),
        label: chosen.map(|o| o.label),
    })
}

/// What a kind of list offers and which of them is chosen, without sorting
/// anything. The bar above the column is drawn before the column itself.
#[tauri::command]
fn picker_controls(state: State<'_, AppState>, kind: String) -> CmdResult<PickerArrangement> {
    let orders = state.picker_order.lock().map_err(err)?;
    let chosen = orders.get(&kind);
    Ok(PickerArrangement {
        order: Vec::new(),
        orders: pickorder::orders_for(&kind),
        chosen: chosen.map(|o| o.id),
        label: chosen.map(|o| o.label),
    })
}

#[tauri::command]
fn set_picker_order(state: State<'_, AppState>, kind: String, order: String) -> CmdResult<()> {
    state.picker_order.lock().map_err(err)?.set(&kind, &order);
    moose_rack::config::set_table_entry("config.toml", "picker_order", &kind, &order)
        .map_err(err)
}

/// Which entries on the page survive the text typed into the filter box, and
/// which group headings are left with nothing under them.
#[derive(Serialize)]
struct PageFilterResult {
    visible: Vec<bool>,
    headings: Vec<bool>,
    shown: usize,
}

#[tauri::command]
fn set_page_names(
    state: State<'_, AppState>,
    names: Vec<String>,
    groups: Option<Vec<Vec<usize>>>,
) -> CmdResult<()> {
    *state.page_names.lock().map_err(err)? = (names, groups.unwrap_or_default());
    Ok(())
}

#[tauri::command]
fn page_filter(state: State<'_, AppState>, query: String) -> CmdResult<PageFilterResult> {
    let held = state.page_names.lock().map_err(err)?;
    let (names, groups) = &*held;
    let visible = pagefilter::visible(names, &query);
    Ok(PageFilterResult {
        headings: pagefilter::empty_groups(groups, &visible, &query),
        shown: visible.iter().filter(|v| **v).count(),
        visible,
    })
}

/// Hand over where every card on the page was drawn — top, left, width — and
/// get back where each of them leads.
///
/// The whole table, once per rebuild, rather than a question per keypress. A
/// held direction repeats nine times a second, and a cursor that only moves
/// after a round trip reads as an app that is thinking about it. The geometry
/// is still decided in `gridnav`; the page only looks the answer up.
/// The same table for a grid that is uniform, from two numbers instead of
/// every card's position.
///
/// What a windowed list uses. Only a band of it is drawn, so most of the cards
/// have no position to measure — and the cursor still has to be able to move
/// through them. `gridnav::uniform` and `gridnav::moves` agree on any layout
/// where both apply; a test says so.
#[tauri::command]
fn grid_uniform(count: usize, columns: usize) -> gridnav::Moves {
    gridnav::uniform(count, columns)
}

#[tauri::command]
fn set_grid(cards: Vec<[f64; 3]>) -> gridnav::Moves {
    let cards: Vec<gridnav::Card> = cards
        .into_iter()
        .map(|[top, left, width]| gridnav::Card { top, left, width })
        .collect();
    gridnav::moves(&cards)
}

/// A line from the measuring script, on stdout where a shell can read it.
///
/// The page's own `console.log` goes to the webview's console and nowhere a
/// script can see it, so a scripted browse had no way of saying whether it had
/// run — which cost one measurement that looked flat and was actually a browse
/// that never happened. This is the whole of the reporting channel.
#[tauri::command]
fn measure_note(text: String) {
    if std::env::var_os("MOOSE_MEASURE").is_some() {
        println!("MEASURE {text}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
}

/// Start the app.
///
/// On desktop `main.rs` calls this. On Android nothing calls it from Rust:
/// `mobile_entry_point` generates the symbol the APK's Java half looks up
/// after `System.loadLibrary`, so this function *is* the entry point and
/// removing the attribute produces a library that loads and then does nothing.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_log();
    moose_rack::datadir::anchor();
    let cfg = Config::load().unwrap_or_default();
    let store = cache::Cache::open(Path::new(CACHE_DB)).expect("opening metadata cache");
    // Archive verification depends on the server's exclusion lists; load the
    // cached copy before anything can download.
    moose_rack::apply_cached_server_config(&store);
    // Never `expect` here. A release build has no console (see
    // `windows_subsystem` in `main.rs`), so a panic at startup is
    // completely silent: the icon bounces and nothing happens, with no way to
    // tell whether the app crashed or never ran.
    let map = CoreMap::load_or_embedded(Path::new(CORE_MAP));
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
        && moose_rack::config_files::seed_config(Path::new("config.toml"))
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
    match moose_rack::media::drop_server_covers(&media_dir) {
        0 => {}
        n => eprintln!("cleared {n} cover(s) fetched from RomM; artwork now comes from ES-DE"),
    }

    // Icon sets fetched under a superseded art mapping, for the same reason:
    // the pictures are on disk in folders the current table does not use, so
    // the grid keeps drawing a controller where it says "Hardware".
    let fingerprints: std::collections::BTreeMap<String, String> =
        moose_rack::iconart::table()
            .into_iter()
            .map(|(name, art)| (name, art.fingerprint()))
            .collect();
    for set in theme::drop_stale_sets(&media_dir, &fingerprints) {
        eprintln!("re-fetch needed for icon set {set}: its pictures predate a corrected mapping");
    }

    tauri::Builder::default()
        // Native folder picker for the RetroArch location setting.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
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
        .invoke_handler(tauri::generate_handler![
            measure_note,
            bios_status,
            download_set,
            recent_games,
            game_displays,
            game_states,
            verify_achievements,
            delete_state,
            confirm_delete_state,
            play_history,
            download_estimate,
            scrape_missing,
            list_art_options,
            set_list_art,
            game_video,
            versions,
            open_link,
            set_autofire_hz,
            platforms,
            roms,
            search,
            collection_groups,
            collections_in,
            collection_roms,
            rom_detail,
            download_rom,
            toggle_favorite,
            rom_covers,
            launch_rom,
            warm_media,
            android_launch_plan,
            android_sync_before,
            android_after_play,
            install_theme_logos,
            fetch_icons,
            icon_styles,
            set_icon_style,
            check_update,
            config_findings,
            config_patch,
            game_lightgun,
            icon_sets,
            install_icon_set,
            set_icon_set,
            remove_icon_set,
            set_retroarch_root,
            systems,
            sync_saves,
            sync_saves_plan,
            attract_pool,
            sync_library,
            sync_bios,
            resolve_save_conflict,
            motion_options,
            set_motion_shader,
            set_system_choice,
            game_cores,
            set_game_core,
            status,
            disk_usage,
            open_settings,
            config_fields,
            set_config_field,
            app_icons,
            set_app_icon,
            verify_server,
            ui_bindings,
            set_key_binding,
            set_pad_binding,
            reset_bindings,
            import_bindings,
            list_controls,
            arrange_list,
            set_list_order,
            cycle_list_order,
            toggle_list_filter,
            clear_list_filters,
            sort_picker,
            picker_controls,
            set_picker_order,
            set_page_names,
            page_filter,
            set_grid,
            grid_uniform
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            install_menu(app.handle())?;
            // The version in the title bar, so "which build is this" is
            // answerable from a screenshot. Set here rather than in
            // tauri.conf.json because the number lives in Cargo.toml and a
            // second copy of it in a config file is a copy that goes stale.
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_title(&format!("Moose Rack v{}", env!("CARGO_PKG_VERSION")));
            }
            // Windows and Linux draw a window icon and can be told a new one at
            // any time, so the chosen icon is put on at every launch. macOS
            // took it from the bundle before this code ran.
            {
                let cfg = moose_rack::config::Config::load().unwrap_or_default();
                let icon = moose_rack::appicon::chosen(cfg.appearance.app_icon.as_deref());
                if let Some(dir) = appicon_dir(app.handle(), icon.id) {
                    let _ = apply_app_icon(app.handle(), icon.id, &dir);
                }
            }
            // A scripted browse, for measuring what the app weighs.
            // See `measure_note` for how it reports back.
            //
            // `MOOSE_MEASURE=path/to/script.js` runs that file in the page a
            // few seconds after launch. Nothing ships enabled and the page
            // itself knows nothing about it: the alternative was asking Frank
            // to open a platform and scroll to the bottom while somebody
            // watched Activity Monitor, which is not a measurement anyone can
            // repeat.
            if let Ok(path) = std::env::var("MOOSE_MEASURE")
                && let Some(win) = app.get_webview_window("main")
            {
                // Out of sight, but not hidden: a hidden window stops being
                // rendered, the page never lays out, and the browse cannot
                // run. Off the side of the display keeps it drawing while
                // keeping it off whoever is using the machine — measuring
                // should not throw a window at them for minutes at a time.
                // Size it from the environment, because one of the things
                // worth weighing is whether the cost follows the window's area
                // rather than what is in it.
                let (w, h) = std::env::var("MOOSE_MEASURE_SIZE")
                    .ok()
                    .and_then(|s| {
                        let (w, h) = s.split_once('x')?;
                        Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
                    })
                    .unwrap_or((1460.0, 1046.0));
                let _ = win.set_size(tauri::LogicalSize::new(w, h));
                let _ = win.set_position(tauri::LogicalPosition::new(-4000.0, 200.0));
                match std::fs::read_to_string(&path) {
                    Ok(script) => {
                        // A switch the script can read, so an A/B needs one
                        // build and changes one thing.
                        let flags = std::env::var("MOOSE_MEASURE_FLAGS").unwrap_or_default();
                        let script = format!("window.__MOOSE_FLAGS = {flags:?};\n{script}");
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_secs(4));
                            if let Err(e) = win.eval(&script) {
                                eprintln!("measure: {e}");
                            }
                        });
                    }
                    Err(e) => eprintln!("measure: cannot read {path}: {e}"),
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("running tauri application");
}

#[cfg(test)]
mod tests {

    /// The About tab hands these to the system browser. A webview understands
    /// a great many schemes that are not websites, and this is the only thing
    /// standing between a link in the page and one of them.
    #[test]
    fn only_web_links_are_opened() {
        assert!(super::is_web_link("https://github.com/mizaimao/moose-rack"));
        assert!(super::is_web_link("http://192.168.1.2:8080"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "moose-rack://x",
            "  https://example.com",
            "",
        ] {
            assert!(!super::is_web_link(bad), "{bad} was treated as a web link");
        }
    }
}

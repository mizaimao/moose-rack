//! The backend, callable from anywhere.
//!
//! Every one of these was a `#[tauri::command]` in `src-tauri/src/lib.rs`, which
//! made them desktop-only by accident of where the file was rather than by
//! anything in them: 67 of the 84 took nothing but `&AppState` and plain
//! arguments. They live here now so the desktop window and the HTTP server call
//! the same function rather than two copies that drift.
//!
//! `src-tauri` keeps a one-line `#[tauri::command]` wrapper for each; the
//! service calls these directly. The seventeen that genuinely need a window --
//! native dialogs, opening a second window -- stayed behind.

#![allow(clippy::too_many_arguments)]

use crate::app::AppState;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::{
    api, cache, config::Config, coremap, media, retroarch::RetroArch, savesync, theme, util,
};

pub const CACHE_DB: &str = "cache.sqlite3";

pub const CORE_MAP: &str = "data/esde-core-map.json";

#[derive(Serialize)]
pub struct PlatformView {
    pub slug: String,
    pub name: String,
    pub rom_count: i64,
    /// Whether a libretro core for this platform is actually installed.
    pub playable: bool,
    /// ES-DE theme art, if any has been installed locally.
    pub logo: Option<String>,
    /// True only for the `logo` style, whose art is a white-on-transparent
    /// wordmark and therefore needs inverting on a light page. Hardware and
    /// console art is full color and must not be touched.
    pub logo_wordmark: bool,
    /// A fixed picture of the machine for the info pane; see `portrait` above.
    pub portrait: Option<String>,
    /// Typical cover aspect (w/h) for this platform, so the grid can shape its
    /// cards instead of cropping. Null until enough covers are cached.
    pub cover_aspect: Option<f32>,
    /// What the machine was: maker, year, kind, and a line about it. The same
    /// four things ES-DE's themes show when you pick a system. Null for a
    /// platform we have nothing to say about, so the pane can leave it out
    /// rather than print blanks.
    pub manufacturer: Option<&'static str>,
    pub released: Option<u16>,
    pub hardware: Option<&'static str>,
    pub blurb: Option<&'static str>,
}

#[derive(Serialize)]
pub struct RomView {
    pub id: i64,
    pub name: String,
    pub fs_name: String,
    pub platform: String,
    pub size_bytes: i64,
    pub downloaded: bool,
    /// In a starred collection. Shown with a star and sorted to the top.
    pub favorite: bool,
    /// The three things a list can be ordered by that are not the name. Pulled
    /// out of the metadata blob here rather than in the page, because the page
    /// would have to parse the same JSON once per game on every redraw.
    pub rating: Option<f64>,
    pub year: Option<i32>,
    /// ISO timestamp, comparable as text. See util::iso_from_epoch.
    pub last_played: Option<String>,
    /// The most players the game supports, or `None` when nothing says.
    ///
    /// Parsed here for the same reason as `year` and `rating`: the field is
    /// free text inside the metadata blob, and the page would otherwise parse
    /// the same JSON once per game on every redraw.
    pub players: Option<u8>,
    /// Subfolder inside the ES-DE system directory, `""` at the top level.
    ///
    /// The page builds the folder view out of this. It arrives with the rest of
    /// the list, once, so walking in and out of a folder is a redraw rather
    /// than a round trip.
    pub rel_dir: String,
}

#[derive(Serialize)]
pub struct RomDetail {
    pub id: i64,
    pub name: String,
    pub fs_name: String,
    pub platform: String,
    /// The slug as well as the display name. The row of recent games holds
    /// games from several consoles, so anything acting on "this game's
    /// console" cannot read it off the page it is on.
    pub platform_slug: String,
    /// Auto-fire for this game: "off", "a" or "y" when the game is one that
    /// can have it, and absent when it is not.
    ///
    /// Absent and "off" are different answers and the pane needs both: absent
    /// means there is nothing to offer, "off" means there is and you have
    /// turned it down — which is the difference between showing no control and
    /// showing one with nothing selected.
    pub autofire: Option<String>,
    /// Shots a second, shown beside the three modes.
    pub autofire_hz: u32,
    pub size_bytes: i64,
    pub downloaded: bool,
    pub core: Option<String>,
    pub core_label: Option<String>,
    /// Local media, as `asset:` URLs the webview can load directly.
    pub cover: Option<String>,
    /// Present only when the video is already on this machine. The pane never
    /// downloads one: see `has_video`.
    pub video: Option<String>,
    /// Whether a gameplay video exists at all, local or on the server.
    ///
    /// Separate from `video` because the pane shows an indicator, not a
    /// player. A video is tens of megabytes against tens of kilobytes for
    /// every other kind of media a game has, and fetching one to find out
    /// whether it existed happened for every game the cursor touched.
    pub has_video: bool,
    /// Every screenshot we could resolve; the UI cycles through them.
    pub screenshots: Vec<String>,
    /// ES-DE artwork by type — 3dboxes, miximages, marquees, fanart and the
    /// rest. Far richer than RomM's own cover + one screenshot.
    pub art: std::collections::BTreeMap<String, String>,

    // Descriptive metadata, straight from RomM (which on this server got it
    // from the ES-DE gamelist import).
    pub summary: Option<String>,
    pub genres: Vec<String>,
    pub companies: Vec<String>,
    pub franchises: Vec<String>,
    pub game_modes: Vec<String>,
    pub player_count: Option<String>,
    /// 0-100 as RomM stores it.
    pub rating: Option<f64>,
    pub release_year: Option<i32>,
    pub alt_names: Vec<String>,
    pub regions: Vec<String>,
    pub manual: Option<String>,
    pub youtube_id: Option<String>,
}

#[derive(Serialize)]
pub struct ConfigFields {
    pub library_root: String,
    /// The ES-DE data directory — the one holding `gamelists/` and
    /// `downloaded_media/` — and the ROMs folder when it is somewhere else,
    /// which it usually is.
    ///
    /// On desktop these are typed once and forgotten. On Android they are the
    /// whole arrangement: ES-DE is a real app on the device with real folders,
    /// and pointing at them is what makes this app see the same library rather
    /// than a second copy hidden in its own private storage.
    pub esde_root: String,
    pub esde_roms: String,
    /// Where RetroArch is told to put battery saves and save states. Written
    /// into the per-launch config, so the folder chosen here is the folder the
    /// emulator actually uses.
    pub saves_root: String,
    pub server_url: String,
    pub server_username: String,
    /// Present or not, never the value. A settings pane has no reason to hand a
    /// credential back to the webview to display; the box shows "set" and lets
    /// you replace it.
    pub server_token_set: bool,
    pub achievements_enabled: bool,
    pub achievements_username: String,
    pub achievements_token_set: bool,
    pub achievements_hardcore: bool,
    pub shaders_enabled: bool,
    pub confirm_delete_state: bool,
    pub mirror_player_one: bool,
    /// Read the two face-button pairs the other way round. See
    /// `crate::binds::pad_map_swapped`.
    pub swap_ab: bool,
    pub swap_xy: bool,
    pub fit_window: bool,
    pub window_decorations: bool,
    pub autofire: String,
    pub save_state_on_exit: bool,
    /// Present so the UI can say where it is writing, and warn when there is
    /// nothing to write to.
    pub config_path: String,
    pub config_exists: bool,
}

pub fn config_fields() -> CmdResult<ConfigFields> {
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
        config_path: abs(&crate::config::path()),
        config_exists: Config::exists(&crate::config::path_str()),
    })
}

pub fn set_config_field(field: String, value: String) -> CmdResult<String> {
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
        crate::config::set_table_number(&crate::config::path_str(), table, key, n.clamp(1, 30))
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
        crate::config::clear_table_entry(&crate::config::path_str(), table, key).map_err(err)?;
        return Ok(format!("{key} cleared"));
    }
    if boolean {
        crate::config::set_table_bool(
            &crate::config::path_str(),
            table,
            key,
            value == "true" || value == "1",
        )
        .map_err(err)?;
    } else {
        crate::config::set_table_entry(&crate::config::path_str(), table, key, value.trim())
            .map_err(err)?;
    }
    Ok(format!("{key} saved — restart to apply"))
}

pub async fn verify_server(
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

pub async fn bios_status(state: &AppState) -> CmdResult<(usize, usize, u64)> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
    crate::bios::status(&client, &library_root).await.map_err(err)
}

pub fn meta_strings(meta: &Option<serde_json::Value>, key: &str) -> Vec<String> {
    meta.as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

pub type CmdResult<T> = Result<T, String>;

pub fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub fn versions(state: &AppState) -> CmdResult<(String, Option<String>)> {
    let server = state.cache.lock().ok().and_then(|c| c.server_version());
    Ok((env!("CARGO_PKG_VERSION").to_owned(), server))
}

pub fn is_web_link(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

pub fn open_link(url: String) -> CmdResult<()> {
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

#[derive(Serialize)]
pub struct StateView {
    pub slot: String,
    pub label: String,
    /// Absolute path to the picture RetroArch saved with the state, if there is
    /// one. The page turns it into something it can load; states written before
    /// thumbnails were switched on have none and never will.
    pub thumb: Option<String>,
    pub when: Option<String>,
    pub size_bytes: u64,
    pub core: String,
    /// False for the autosave, which has no slot number to enter.
    pub resumable: bool,
    pub when_epoch: Option<u64>,
}

pub async fn verify_achievements() -> CmdResult<crate::achievements::Verified> {
    let cfg = Config::load().map_err(err)?;
    let user = cfg.achievements.username.clone().unwrap_or_default();
    let token = cfg.achievements.token.clone().unwrap_or_default();
    if user.trim().is_empty() || token.trim().is_empty() {
        return Ok(crate::achievements::Verified {
            ok: false,
            user: None,
            error: Some(if user.trim().is_empty() {
                "no username set".into()
            } else {
                "no token set".to_string()
            }),
        });
    }
    Ok(crate::achievements::verify(&user, &token).await)
}

pub fn game_states(state: &AppState, id: i64) -> CmdResult<Vec<StateView>> {
    let Some(ra) = state.retroarch.as_ref() else {
        return Ok(Vec::new());
    };
    let cache = state.cache.lock().map_err(err)?;
    let Some(row) = cache.rom_by_id(id).map_err(err)? else {
        return Ok(Vec::new());
    };
    let now = std::time::SystemTime::now();
    Ok(crate::states::shelf(&ra.root, &cache, &state.map, &row.fs_name)
        .map_err(err)?
        .into_iter()
        .map(|s| StateView {
            resumable: s.entry_slot().is_some(),
            when: s.modified.map(|t| crate::states::ago(t, now)),
            // The raw time as well as the phrase. "3 days ago" cannot be
            // sorted, and picking the newest state to resume from is exactly
            // what the front end needs to do.
            when_epoch: s.modified,
            thumb: s.thumb.as_deref().map(crate::util::webview_path),
            slot: s.slot,
            label: s.label,
            size_bytes: s.size,
            core: s.core,
        })
        .collect())
}

pub fn delete_state(state: &AppState, id: i64, slot: String) -> CmdResult<String> {
    let ra = state.retroarch.as_ref().ok_or("RetroArch not found")?;
    let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
    let cache = state.cache.lock().map_err(err)?;
    let row = cache.rom_by_id(id).map_err(err)?.ok_or("no such game")?;
    let shelf = crate::states::shelf(&ra.root, &cache, &state.map, &row.fs_name)
        .map_err(err)?;
    let found = shelf
        .into_iter()
        .find(|s| s.slot == slot)
        .ok_or_else(|| format!("no {slot} state for {}", row.name))?;
    let label = found.label.clone();
    crate::states::remove(&library_root, id, &found).map_err(err)?;
    Ok(format!("deleted {label} — a copy is in the backups folder"))
}

pub fn confirm_delete_state() -> CmdResult<bool> {
    Ok(Config::load().unwrap_or_default().saves.confirm_delete_state)
}

#[derive(Serialize)]
pub struct History {
    pub total_seconds: i64,
    pub sessions: i64,
    pub games: i64,
    pub platforms: Vec<PlatformTime>,
    pub top: Vec<GameTime>,
    /// Games opened more than once and still barely played.
    pub abandoned: Vec<GameTime>,
}

#[derive(Serialize)]
pub struct PlatformTime {
    pub slug: String,
    pub name: String,
    pub seconds: i64,
    pub spelled: String,
    pub sessions: i64,
    pub games: i64,
}

#[derive(Serialize)]
pub struct GameTime {
    pub id: i64,
    pub name: String,
    pub platform: String,
    pub seconds: i64,
    pub spelled: String,
    pub sessions: i64,
    pub last: Option<String>,
}

pub fn play_history(state: &AppState) -> CmdResult<History> {
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
            spelled: crate::util::spell_duration(seconds),
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
        spelled: crate::util::spell_duration(seconds),
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
                spelled: crate::util::spell_duration(seconds),
                seconds,
                sessions,
                last: None,
            })
            .collect(),
    })
}

pub fn recent_games(
    state: &AppState,
    limit: Option<usize>,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.recently_played(limit.unwrap_or(8)).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadChoice {
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub collection: Option<String>,
    /// Ticked in the pane's own list. `collection` is the single one you
    /// pointed at before opening it; both are honoured, and a game in two of
    /// them is still one download.
    #[serde(default)]
    pub collections: Vec<String>,
    pub art: String,
    pub videos: bool,
    pub manuals: bool,
    pub bios: bool,
}

impl DownloadChoice {
    pub fn want(&self) -> crate::bulk::Want {
        use crate::bulk;
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

pub fn rows_for_choice(
    state: &AppState,
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

pub async fn download_estimate(
    state: &AppState,
    choice: DownloadChoice,
) -> CmdResult<(String, bool, String)> {
    use crate::{bulk, diskspace};

    let rows = rows_for_choice(&state, &choice.platforms, &choice.collection, &choice.collections)?;
    let want = choice.want();
    let mut est = bulk::estimate(&rows, want, |r| row_path(&state, r).is_some());
    // Asked of the server rather than averaged, because unlike artwork there is
    // a fixed set of these and it already knows which are here.
    let mut summary = est.describe();
    if choice.bios {
        let library_root = state.roms_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
        if let Some(client) = state.client.clone()
            && let Ok((total, here, bytes)) = crate::bios::status(&client, &library_root).await
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

pub fn platforms(state: &AppState) -> CmdResult<Vec<PlatformView>> {
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
    let switched_off = crate::esde::switched_off_slugs(&state.roms_dir, &state.map);
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
                .or_else(|| crate::platformicon::installed(&state.media_dir, &p.fs_slug))
                .map(|p| crate::util::webview_path(&p)),
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
                .map(|p| crate::util::webview_path(&p)),
            cover_aspect: media::cover_aspect(&state.media_dir, &p.fs_slug),
            manufacturer: crate::platformfacts::of(&p.fs_slug).map(|f| f.manufacturer),
            released: crate::platformfacts::of(&p.fs_slug).map(|f| f.released),
            hardware: crate::platformfacts::of(&p.fs_slug).map(|f| f.hardware),
            blurb: crate::platformfacts::of(&p.fs_slug).map(|f| f.blurb),
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
    let order = crate::pickorder::by_name(
        &views
            .iter()
            .map(|p| crate::pickorder::PickerRow {
                name: p.name.clone(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    );
    let mut views: Vec<Option<PlatformView>> = views.into_iter().map(Some).collect();
    Ok(order.into_iter().filter_map(|i| views[i].take()).collect())
}

pub fn to_views(
    state: &AppState,
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

pub fn roms(
    state: &AppState,
    platform: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.roms_for(&platform).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

#[derive(serde::Serialize)]
pub struct GroupView {
    pub group: String,
    /// Human label; the raw group name is a server-side slug.
    pub label: String,
    pub count: i64,
}

#[derive(serde::Serialize)]
pub struct CollectionView {
    pub id: String,
    pub name: String,
    pub rom_count: i64,
    pub is_favorite: bool,
    /// A few member ROM ids — the card fetches their covers through the same
    /// local cache the game grids use, so this works offline too.
    pub sample_ids: Vec<i64>,
    /// How many of its games are downloaded here.
    ///
    /// The question a collection card cannot otherwise answer: "can I play any
    /// of this on a plane". Counted rather than stored, because a file can be
    /// deleted from under us and a stale count is worse than none.
    pub local_count: i64,
}

pub fn group_label(group: &str) -> String {
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

pub fn collection_groups(state: &AppState) -> CmdResult<Vec<GroupView>> {
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

pub fn collections_in(state: &AppState, group: String) -> CmdResult<Vec<CollectionView>> {
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

pub fn collection_roms(
    state: &AppState,
    id: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.roms_in_collection(&id).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

pub fn search(
    state: &AppState,
    term: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    let rows = {
        let cache = state.cache.lock().map_err(err)?;
        cache.search(&term, 200).map_err(err)?
    };
    Ok(to_views(&state, rows, list.map(|l| l.scope()).as_deref()))
}

pub async fn rom_detail(state: &AppState, id: i64) -> CmdResult<RomDetail> {
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
        |p: Option<std::path::PathBuf>| p.map(|p| crate::util::webview_path(&p));

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
            art.insert((*kind).to_owned(), crate::util::webview_path(&p));
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
            .map(|p| crate::util::webview_path(&p))
            .collect(),
        art,
        downloaded: row_path(&state, &row).is_some(),
        autofire: autofire_possible(&row)
            .then(|| crate::tweaks::AutoFire::parse(&stored_autofire()).key().to_owned()),
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
        manual: manual.map(|p| crate::util::webview_path(&p)),
        youtube_id: row.youtube_id.clone().filter(|s| !s.is_empty()),
    })
}

#[derive(Serialize, Clone)]
pub struct CoverView {
    pub id: i64,
    pub cover: Option<String>,
}

/// Artwork for a batch of games, cache first and network second.
///
/// `emit` is handed each group of covers as it lands. The desktop window turns
/// that into a `covers-ready` event so the grid fills while the fetch is still
/// running; the HTTP service passes a closure that does nothing, because a
/// single JSON response has nothing to fill progressively. That callback is the
/// only thing that ever needed a window here, so it is the only thing the
/// caller supplies.
pub async fn rom_covers(
    state: &AppState,
    ids: Vec<i64>,
    local_only: Option<bool>,
    emit: &(dyn Fn(&[CoverView]) + Send + Sync),
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
                // app's own download folder keyed by the upstream server's slug
                // — and on a device whose library is an ES-DE folder the artwork
                // is not there and is not keyed that way. Every cover came back
                // null: measured, the four samples behind each collection in My
                // Collections resolved to nothing, so every card in the tab drew
                // the two-letter placeholder.
                let (dir, key) = media_scope(state, row);
                CoverView {
                    id: row.id,
                    cover: media::local_art(dir, key, &stem, &list_art)
                        .map(|p| crate::util::webview_path(&p)),
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
    let mut pending: Vec<CoverView> = Vec::new();
    loop {
        // Fill every free slot.
        while set.len() < CONCURRENCY {
            let Some(row) = queue.next() else { break };
            let client = state.client.clone();
            // The same scope the fast pass above uses, so a cover found by one
            // is the cover fetched by the other.
            let (scope_dir, scope_key) = media_scope(state, row);
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
                CoverView { id, cover: cover.map(|p| crate::util::webview_path(&p)) }
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
                emit(&pending);
            }
            out.append(&mut pending);
        }
    }
    out.append(&mut pending);
    // Keep what this batch learned, so scrolling back over the same cards — and
    // the next launch — costs nothing.
    for platform in rows
        .iter()
        .map(|r| r.platform_slug.as_str())
        .collect::<std::collections::BTreeSet<_>>()
    {
        media::save_art_index(&state.media_dir, platform);
    }
    Ok(out)
}

pub async fn toggle_favorite(state: &AppState, id: i64) -> CmdResult<bool> {
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
        let now = crate::favorites::is_starred(&cache, id).map_err(err)?;
        (
            crate::favorites::target(&cache, &row.platform_slug).map_err(err)?,
            !now,
        )
    };

    let landed = crate::favorites::on_server(&client, target, id, starred)
        .await
        .map_err(err)?;
    let Some(landed) = landed else {
        // Unstarring something that was never in a list. Nothing failed, and
        // it is already what was wanted.
        return Ok(false);
    };

    let mut cache = state.cache.lock().map_err(err)?;
    crate::favorites::record(&mut cache, &landed, id, starred).map_err(err)?;
    Ok(starred)
}

pub fn list_art_options(state: &AppState) -> CmdResult<(Vec<(String, String)>, String)> {
    let current = state.list_art.lock().map_err(err)?.clone();
    let choices = media::LIST_ART_CHOICES
        .iter()
        .map(|(k, label)| ((*k).to_owned(), (*label).to_owned()))
        .collect();
    Ok((choices, current))
}

pub fn set_list_art(state: &AppState, value: String) -> CmdResult<String> {
    if !media::LIST_ART_CHOICES.iter().any(|(k, _)| *k == value) {
        return Err(format!("unknown artwork type {value}"));
    }
    crate::config::set_table_entry(&crate::config::path_str(), "media", "list_art", &value)
        .map_err(err)?;
    *state.list_art.lock().map_err(err)? = value.clone();
    Ok(format!("game lists now show {value}"))
}

pub async fn game_video(state: &AppState, id: i64) -> CmdResult<String> {
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
        .map(|p| crate::util::webview_path(&p))
        .ok_or_else(|| format!("no video for {}", row.name))
}

pub fn autofire_for(row: &cache::RomRow) -> crate::tweaks::AutoFire {
    use crate::tweaks::AutoFire;
    if !autofire_possible(row) {
        return AutoFire::Off;
    }
    AutoFire::parse(&stored_autofire())
}

pub fn autofire_hz(state: &AppState) -> u32 {
    state
        .autofire_hz
        .lock()
        .ok()
        .and_then(|v| *v)
        .unwrap_or_else(|| Config::load().map(|c| c.retroarch.autofire_hz).unwrap_or(6))
}

pub fn set_autofire_hz(state: &AppState, hz: u32) -> CmdResult<u32> {
    let hz = hz.clamp(1, 30);
    *state.autofire_hz.lock().map_err(err)? = Some(hz);
    Ok(hz)
}

pub fn stored_autofire() -> String {
    Config::load().unwrap_or_default().retroarch.autofire
}

pub fn autofire_possible(row: &cache::RomRow) -> bool {
    if !matches!(row.platform_slug.as_str(), "arcade" | "neogeoaes" | "neogeocd") {
        return false;
    }
    let meta = row.meta_json.as_deref().and_then(|m| serde_json::from_str(m).ok());
    meta_strings(&meta, "genres").iter().any(|g| g.to_lowercase().contains("shoot"))
}

pub fn state_game_display() -> String {
    Config::load().unwrap_or_default().retroarch.game_display
}

#[derive(Serialize)]
pub struct DisplayView {
    pub key: String,
    pub label: String,
    pub selected: bool,
}

pub fn game_displays() -> CmdResult<Vec<DisplayView>> {
    use crate::macdisplay::{self, Choice};
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

#[derive(serde::Serialize)]
pub struct AndroidCandidate {
    /// `package/Activity`, ready for an explicit Intent. A leading dot on the
    /// activity is ES-DE's shorthand for "in this package" and is expanded
    /// where the Intent is built.
    pub component: String,
    /// The core's file name — `gambatte_libretro_android.so`. Not a path: the
    /// directory is RetroArch's own private one and depends on which of its two
    /// package names turns out to be installed, so it is completed on the
    /// Kotlin side where that is known.
    pub core_file: Option<String>,
    /// ES-DE's name for the emulator, for the toast.
    pub label: String,
}

#[derive(serde::Serialize)]
pub struct AndroidPlan {
    /// The game, so the Kotlin side can name it again when the emulator hands
    /// the screen back — it is the only thing that knows the game has ended.
    pub id: i64,
    /// Absolute path to the ROM.
    pub rom: String,
    pub name: String,
    /// The generated config, or `None` to let RetroArch use its own.
    pub config: Option<String>,
    /// Anything worth telling the user about what is and is not applied.
    pub notes: Vec<String>,
    /// Best first; the first one whose component is actually installed wins.
    pub candidates: Vec<AndroidCandidate>,
}

pub fn core_file_is(file: Option<&str>, core: &str) -> bool {
    file.and_then(|f| f.split("_libretro").next())
        .is_some_and(|stem| stem.eq_ignore_ascii_case(core))
}

pub fn android_launch_plan(
    state: &AppState,
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

pub async fn warm_media(state: &AppState, platform: String) -> CmdResult<()> {
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

pub async fn android_sync_before(state: &AppState, id: i64) -> CmdResult<String> {
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

pub async fn android_after_play(
    state: &AppState,
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
        && let Ok(true) = cache.record_play(row.id, &crate::util::now_iso(), seconds)
    {
        notes.push(format!("played for {}", crate::util::spell_duration(seconds)));
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

pub fn android_config(
    state: &AppState,
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
    let ra = crate::retroarch::RetroArch::android_app(&PathBuf::from(format!(
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
            crate::shaders::preset_for(&over, platform)
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
        .filter(|_| crate::shaders::display_of(platform) == crate::shaders::Display::Crt);
    let chained =
        motion.and_then(|m| crate::shaders::write_chained(&ra, &out_dir, preset.as_deref(), m));
    if motion.is_some() && chained.is_none() {
        notes.push("motion shader not installed — using the base shader alone".to_owned());
    }
    let shader_lines = match &chained {
        Some(p) => format!(
            "\n# Base shader with a motion pass chained on.\n\
             video_shader_enable = \"true\"\nvideo_shader = \"{}\"\n\
             video_shader_subframes = \"{}\"\n",
            p.display(),
            crate::shaders::subframes_for(refresh)
        ),
        None => crate::shaders::config_lines(&ra, preset.as_deref()),
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
            crate::shaders::label_of(preset.as_deref().unwrap_or_default())
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
        crate::achievements::config_lines(&state.achievements),
        crate::lightgun::config_lines(platform, gun),
    );

    let saves_root = crate::util::expand_tilde(&cfg.saves.root);
    let overlay = match ra.write_overrides_full(
        &out_dir,
        None,
        &extra,
        crate::retroarch::Input {
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

    let merged = crate::retroarch::merge_config(&base, &overlay);
    let path = out_dir.join("launch.cfg");
    match std::fs::write(&path, merged) {
        Ok(()) => (Some(path.to_string_lossy().into_owned()), notes),
        Err(e) => {
            notes.push(format!("using RetroArch's own settings — could not write the config: {e}"));
            (None, notes)
        }
    }
}

pub fn write_game_preset(
    ra: &crate::retroarch::RetroArch,
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
    let Ok(parsed) = crate::slangp::Preset::load(Path::new(preset)) else {
        return String::new();
    };
    let body = crate::slangp::standalone(&parsed);
    let dest = dir.join(format!("{stem}.slangp"));
    if std::fs::write(&dest, body).is_err() {
        return String::new();
    }
    "\n# The shader for this game is in RetroArch's own game slot, which is the\n\
     # only thing that loads a preset with content when there is no command line.\n\
     auto_shaders_enable = \"true\"\n"
        .to_owned()
}

pub fn scopeguard_forget_media() -> impl Drop {
    struct Forget;
    impl Drop for Forget {
        fn drop(&mut self) {
            crate::esde::forget_media_listings();
            crate::media::forget_dir_indexes();
        }
    }
    Forget
}

pub fn state_autofire() -> crate::tweaks::AutoFire {
    Config::load()
        .map(|c| crate::tweaks::AutoFire::parse(&c.retroarch.autofire))
        .unwrap_or_default()
}

#[derive(Default)]
pub struct AutoSync {
    /// A line for the launch notes, when anything happened.
    pub note: Option<String>,
    /// Saves changed on both sides. Non-empty means the user has to choose.
    pub conflicts: Vec<crate::savesync::SaveConflict>,
    /// Set when the sync could not run at all — server down, DNS gone, timeout.
    /// Distinct from a conflict: nothing is wrong with the save, we simply do
    /// not know whether it is current.
    pub failed: Option<String>,
}

pub async fn auto_sync(
    state: &AppState,
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

pub async fn resolve_save_conflict(
    state: &AppState,
    file_name: String,
    keep: crate::savesync::Keep,
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

pub async fn icon_sets(state: &AppState) -> CmdResult<Vec<crate::iconsets::IconSetView>> {
    // Names and authors are nice to have, not required: with no network the
    // tab still lists every set and still shows its pictures, because the
    // pictures come from the table.
    let listed = crate::theme_remote::list_default().await.unwrap_or_default();
    let slugs: Vec<String> = {
        let cache = state.cache.lock().map_err(err)?;
        cache.platforms().map_err(err)?.into_iter().map(|p| p.fs_slug).collect()
    };
    let active = state.icon_set.lock().map_err(err)?.clone();
    // The consoles actually in this library, under the names ES-DE files them
    // by — a preview of systems the user does not own would be decoration.
    let systems = theme::preview_systems(&state.map, &slugs, 6);

    Ok(crate::iconart::ordered()
        .into_iter()
        .map(|(dir, art)| {
            let entry = listed.iter().find(|t| t.dir_name() == dir);
            let look = art.best_look().map(|l| l.id.as_str()).unwrap_or("");
            crate::iconsets::IconSetView {
                name: entry.map(|t| t.name.clone()).unwrap_or_else(|| crate::iconsets::pretty(&dir)),
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

#[derive(Serialize)]
pub struct ConfigFinding {
    pub severity: String,
    pub what: String,
    pub note: String,
    pub fixable: bool,
}

pub async fn check_update(
    state: &AppState,
) -> CmdResult<Option<crate::update::Update>> {
    let http = state
        .client
        .as_ref()
        .map(|c| c.http().clone())
        .unwrap_or(util::http_client(None).map_err(err)?);
    crate::update::check(&http).await.map_err(err)
}

pub fn config_findings() -> CmdResult<Vec<ConfigFinding>> {
    let Ok(text) = std::fs::read_to_string(crate::config::path()) else { return Ok(Vec::new()) };
    Ok(crate::configpatch::inspect(&text)
        .into_iter()
        .map(|f| ConfigFinding {
            severity: f.severity.to_string(),
            what: f.what,
            note: f.note,
            fixable: f.fix.is_some(),
        })
        .collect())
}

pub fn config_patch() -> CmdResult<String> {
    let path = crate::config::path();
    let path = path.as_path();
    let text = std::fs::read_to_string(path).map_err(err)?;
    let (patched, applied) = crate::configpatch::patch(&text);
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

pub fn game_lightgun(state: &AppState, id: i64) -> CmdResult<Option<(String, String)>> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let Some(name) = crate::lightgun::label(&row.platform_slug) else {
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

/// Fetch one icon set's console pictures into the media tree.
///
/// `progress` is called with a line like "48 of 320…". The desktop window turns
/// that into an `icons-progress` event; the HTTP service passes a closure that
/// does nothing, because a single response has no progress to report. That
/// callback was the only reason this needed a window.
pub async fn fetch_icon_set(
    state: &AppState,
    dir: &str,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> CmdResult<String> {
    let art = crate::iconart::of(dir).ok_or_else(|| format!("no artwork recorded for {dir}"))?;
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
                progress(&format!("{done} of {total}…"));
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

pub async fn install_icon_set(
    state: &AppState,
    dir: String,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> CmdResult<String> {
    fetch_icon_set(state, &dir, progress).await
}

/// Fetch the chosen set, or the default one if none has been chosen.
pub async fn fetch_icons(
    state: &AppState,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> CmdResult<String> {
    let chosen = state.icon_set.lock().map_err(err)?.clone();
    let set = if chosen.is_empty() { crate::iconart::DEFAULT_SET.to_owned() } else { chosen.clone() };
    let summary = fetch_icon_set(state, &set, progress).await?;

    // Choosing it as well as fetching it. Pressing "Get console pictures" and
    // seeing nothing change, because the grid was still on the shared pool, is
    // the confusion this answers.
    if chosen.is_empty() {
        crate::config::set_table_entry(&crate::config::path_str(), "icons", "set", &set)
            .map_err(err)?;
        *state.icon_set.lock().map_err(err)? = set.clone();
    }
    Ok(summary)
}

pub fn set_icon_set(state: &AppState, dir: String) -> CmdResult<String> {
    crate::config::set_table_entry(&crate::config::path_str(), "icons", "set", &dir).map_err(err)?;
    *state.icon_set.lock().map_err(err)? = dir.clone();
    Ok(if dir.is_empty() {
        "Back to the shared pictures".to_owned()
    } else {
        format!("Console pictures from {dir}")
    })
}

pub fn remove_icon_set(state: &AppState, dir: String) -> CmdResult<String> {
    theme::remove_set(&state.media_dir, &dir).map_err(err)?;
    let mut active = state.icon_set.lock().map_err(err)?;
    if *active == dir {
        active.clear();
        drop(active);
        crate::config::set_table_entry(&crate::config::path_str(), "icons", "set", "").map_err(err)?;
    }
    Ok(format!("{dir} removed"))
}

#[derive(Serialize)]
pub struct EmulatorOption {
    pub core: String,
    pub label: String,
    pub installed: bool,
    /// True for the core ES-DE would pick by default.
    pub is_default: bool,
}

#[derive(Clone, Serialize)]
pub struct ShaderOptionView {
    pub path: String,
    pub label: String,
    pub note: String,
}

#[derive(Serialize)]
pub struct SystemView {
    pub slug: String,
    pub name: String,
    pub rom_count: i64,
    pub display: String,
    /// Currently selected core and shader, whether defaulted or chosen.
    pub core: Option<String>,
    pub shader: Option<String>,
    pub emulators: Vec<EmulatorOption>,
    pub shaders: Vec<ShaderOptionView>,
    /// What this console's light gun was called, when it had one. `None` means
    /// no switch is offered for this system.
    pub gun: Option<String>,
    pub gun_on: bool,
}

pub fn systems(state: &AppState) -> CmdResult<Vec<SystemView>> {
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
            crate::shaders::available(r, display)
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
    let crt_shaders = options_for(crate::shaders::Display::Crt);
    let handheld_shaders = options_for(crate::shaders::Display::Handheld);

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

            let display = crate::shaders::display_of(&slug);
            let shader_list = match display {
                crate::shaders::Display::Crt => crt_shaders.clone(),
                crate::shaders::Display::Handheld => handheld_shaders.clone(),
            };

            SystemView {
                gun: crate::lightgun::label(&slug).map(str::to_owned),
                gun_on: state
                    .lightgun
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&slug)
                    .is_some_and(|v| v.trim() == "on"),
                core: resolve_core(&state, &slug),
                shader: if state.shaders_enabled {
                    crate::shaders::preset_for(
                        &state.shader_overrides.lock().unwrap_or_else(|e| e.into_inner()),
                        &slug,
                    )
                } else {
                    None
                },
                display: match display {
                    crate::shaders::Display::Crt => "CRT",
                    crate::shaders::Display::Handheld => "Handheld",
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

pub fn saves_root(state: &AppState) -> PathBuf {
    Config::load()
        .map(|c| crate::util::expand_tilde(&c.saves.root))
        .unwrap_or_else(|_| {
            state
                .retroarch
                .as_ref()
                .map(|ra| ra.root.clone())
                .unwrap_or_else(|| PathBuf::from("Saves"))
        })
}

pub async fn sync_saves_plan(state: &AppState) -> CmdResult<crate::syncplan::Review> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    let root = saves_root(&state);

    // Same shape as `sync_saves`: the cache is not Sync, so the scan takes the
    // lock and gives it back before anything is awaited.
    let candidates = {
        let cache = state.cache.lock().map_err(err)?;
        crate::savesync::scan(&cache, &state.map, &root).map_err(err)?
    };

    let (states, _skipped) = crate::savesync::client_states(&candidates);
    let identity = crate::savesync::DeviceIdentity::ensure(&client, Path::new("."))
        .await
        .map_err(err)?;
    let plan = client.negotiate(&identity.device_id, &states).await.map_err(err)?;
    Ok(crate::syncplan::Review::from_plan(&plan))
}

#[derive(Serialize)]
pub struct AttractPick {
    pub id: i64,
    pub name: String,
    pub platform: String,
    /// A still, following the same chain the grid uses.
    pub image: Option<String>,
    pub video: Option<String>,
}

pub async fn attract_pool(state: &AppState) -> CmdResult<Vec<AttractPick>> {
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
                image: image.map(|p| crate::util::webview_path(&p)),
                video: video.map(|p| crate::util::webview_path(&p)),
            });
        }
        pool
    })
    .await
    .map_err(err)
}

#[derive(Serialize)]
pub struct SyncRun {
    pub headline: String,
    pub notes: Vec<String>,
    pub conflicts: Vec<crate::savesync::SaveConflict>,
}

pub async fn sync_saves(state: &AppState) -> CmdResult<SyncRun> {
    let client = state.client.clone().ok_or("not connected to a server")?;
    // Not RetroArch's install folder: the saves folder. See `saves_root`.
    let root = saves_root(&state);

    // The cache is not Sync, so the scan takes the lock and releases it before
    // any awaiting starts. A future holding the connection across an await
    // cannot be spawned at all.
    let candidates = {
        let cache = state.cache.lock().map_err(err)?;
        crate::savesync::scan(&cache, &state.map, &root).map_err(err)?
    };

    let summary = crate::savesync::run_all(
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

#[derive(Serialize)]
pub struct MotionView {
    pub current: Option<String>,
    pub options: Vec<ShaderOptionView>,
}

pub fn motion_options(state: &AppState) -> CmdResult<MotionView> {
    let installed: Vec<ShaderOptionView> = state
        .retroarch
        .as_ref()
        .map(|ra| {
            crate::shaders::MOTION
                .iter()
                .filter(|o| crate::shaders::resolve(ra, o.path).is_some())
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

pub fn set_motion_shader(state: &AppState, value: String) -> CmdResult<String> {
    crate::config::set_table_entry(&crate::config::path_str(), "shaders", "motion", &value)
        .map_err(err)?;
    let chosen = (value != "none" && !value.is_empty()).then_some(value);
    *state.motion_shader.lock().map_err(err)? = chosen.clone();
    Ok(match chosen {
        Some(v) => format!("motion layer: {v}"),
        None => "motion layer off".to_owned(),
    })
}

pub fn set_system_choice(
    state: &AppState,
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
    crate::config::set_table_entry(&crate::config::path_str(), table, &slug, &value).map_err(err)?;

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

#[derive(Serialize)]
pub struct CoreChoice {
    pub core: String,
    pub label: String,
    pub installed: bool,
    /// True for the core this game would launch with right now.
    pub current: bool,
    /// True when that is because of a per-game override rather than the
    /// platform default.
    pub pinned: bool,
}

pub fn game_cores(state: &AppState, id: i64) -> CmdResult<Vec<CoreChoice>> {
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
        .contains_key(&crate::config::game_key(&row.platform_slug, &row.fs_name));

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

pub fn set_game_core(state: &AppState, id: i64, core: String) -> CmdResult<String> {
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;

    let key = crate::config::game_key(&row.platform_slug, &row.fs_name);
    if core.is_empty() {
        crate::config::clear_table_entry(&crate::config::path_str(), "cores.per_game", &key)
            .map_err(err)?;
    } else {
        crate::config::set_table_entry(&crate::config::path_str(), "cores.per_game", &key, &core)
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
        if let Some(shipped) = crate::config::arcade_core_map().remove(&key) {
            let msg = format!("{}: back to {shipped}, the core known to run it", row.name);
            live.insert(key, shipped);
            return Ok(msg);
        }
        return Ok(format!("{}: back to the {} default", row.name, row.platform_slug));
    }
    live.insert(key, core.clone());
    Ok(format!("{}: pinned to {core}", row.name))
}

pub fn install_theme_logos(state: &AppState) -> CmdResult<String> {
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
pub struct Status {
    pub server: String,
    pub connected: bool,
    /// False when there is no config.toml at all — a different problem from a
    /// server that will not answer, and worth saying so in the UI.
    pub configured: bool,
    pub retroarch: Option<String>,
    pub cores_installed: usize,
    pub roms_cached: i64,
    /// Absolute paths, shown in the UI so downloaded data is never a mystery.
    pub roms_dir: String,
    pub media_dir: String,
    /// Directory every relative path is resolved against, and therefore where
    /// `config.toml` is read from. Reported because "it cannot find my config"
    /// is otherwise unanswerable from inside the app.
    pub data_dir: String,
    pub config_path: String,
    /// True when the app has no config.toml and is sitting in a directory that
    /// already holds other things. It is about to create a library folder, a
    /// cache and a config here, and dropping all that into someone's Downloads
    /// or Desktop is rude.
    pub crowded_folder: bool,
    /// What else is in there, for the warning to name.
    pub folder_entries: usize,
}

pub fn status(state: &AppState) -> CmdResult<Status> {
    let cache = state.cache.lock().map_err(err)?;
    Ok(Status {
        server: state
            .client
            .as_ref()
            .map(|c| c.base().to_owned())
            .unwrap_or_default(),
        connected: state.client.is_some(),
        configured: Config::exists(&crate::config::path_str()),
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
        config_path: abs(&crate::config::path()),
        crowded_folder: !Config::exists(&crate::config::path_str()) && neighbours() > 2,
        folder_entries: neighbours(),
    })
}

pub async fn disk_usage(state: &AppState) -> CmdResult<u64> {
    Ok(util::dir_size(&state.roms_dir) + util::dir_size(&state.media_dir))
}

pub fn set_retroarch_root(path: String) -> CmdResult<String> {
    let path = path.trim().to_owned();

    if path.is_empty() {
        // Empty means "go back to probing the usual places".
        crate::config::clear_table_entry(&crate::config::path_str(), "retroarch", "root").map_err(err)?;
        return Ok("Cleared. The usual locations will be searched again after a restart.".into());
    }

    // Verify before writing, so a typo fails here rather than at the next launch.
    let found = RetroArch::locate(Some(&path)).map_err(|e| e.to_string())?;
    crate::config::set_table_entry(&crate::config::path_str(), "retroarch", "root", &path)
        .map_err(err)?;

    Ok(format!(
        "Found {} with {} cores. Restart to use it.",
        found.binary.display(),
        found.installed_cores().len()
    ))
}

pub fn current_style(state: &AppState) -> theme::IconStyle {
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
pub struct IconStyleView {
    pub key: String,
    pub label: String,
    /// How many of our platforms have art in this style.
    pub available: usize,
    pub selected: bool,
}

pub fn icon_styles(state: &AppState) -> CmdResult<Vec<IconStyleView>> {
    let slugs: Vec<String> = {
        let cache = state.cache.lock().map_err(err)?;
        cache.platforms().map_err(err)?.into_iter().map(|p| p.fs_slug).collect()
    };
    let set = state.icon_set.lock().map_err(err)?.clone();
    let cur = state.icon_look.lock().map_err(err)?.clone();
    let mut out: Vec<IconStyleView> = Vec::new();

    if let Some(art) = crate::iconart::of(&set) {
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

pub fn set_icon_style(state: &AppState, key: String) -> CmdResult<String> {
    let set = state.icon_set.lock().map_err(err)?.clone();

    // A look belonging to the chosen set, or a folder in the shared pool that
    // actually holds pictures. Anything else is refused rather than stored: an
    // unknown id is a folder that does not exist, and the grid would go blank.
    let label = match crate::iconart::of(&set).and_then(|a| a.look(&key).cloned()) {
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
    crate::config::set_table_entry(&crate::config::path_str(), "icons", "style", &key).map_err(err)?;
    Ok(label)
}

#[derive(serde::Serialize)]
pub struct AppIconView {
    pub id: String,
    pub label: String,
    /// Absolute path to the preview picture, for `convertFileSrc`. Empty when
    /// the built files are missing, which the picker draws as a gap rather
    /// than a broken image.
    pub preview: String,
    pub selected: bool,
}

#[cfg(target_os = "macos")]
pub fn mac_bundle_icns() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // Contents/MacOS/<exe> → Contents
    let contents = exe.parent()?.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let icns = contents.join("Resources").join("icon.icns");
    icns.is_file().then_some(icns)
}

pub fn neighbours() -> usize {
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

pub fn abs(p: &Path) -> String {
    // Through the same helper the pictures use: on Windows `canonicalize`
    // hands back \\?\C:\... and the status card would print that at somebody
    // as the answer to "where is my config".
    crate::util::webview_path(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
}

pub fn local_path(state: &AppState, platform: &str, fs_name: &str) -> Option<PathBuf> {
    let p = state.roms_dir.join(platform).join(fs_name);
    (p.is_file() || p.is_dir()).then_some(p)
}

pub fn row_path(state: &AppState, row: &cache::RomRow) -> Option<PathBuf> {
    if let Some(p) = row.local_path.as_deref().map(PathBuf::from)
        && (p.is_file() || p.is_dir())
    {
        return Some(p);
    }
    local_path(state, &row.platform_slug, &row.fs_name)
}

pub fn media_scope<'a>(state: &'a AppState, row: &'a cache::RomRow) -> (&'a Path, &'a str) {
    match (state.esde_media.as_deref(), row.esde_system.as_deref()) {
        (Some(dir), Some(system)) => (dir, system),
        _ => (state.media_dir.as_path(), row.platform_slug.as_str()),
    }
}

pub fn resolve_core(state: &AppState, platform: &str) -> Option<String> {
    resolve_core_for(state, platform, None)
}

pub fn resolve_core_for(
    state: &AppState,
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

pub fn install_panic_log() {
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

impl RomView {
    /// The nine facts a list needs to order and narrow itself.
    fn as_row(&self) -> crate::gamelist::Row {
        crate::gamelist::Row {
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

#[derive(Debug, Deserialize)]
pub struct ListRef {
    pub view: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub collection: Option<String>,
}

impl ListRef {
    fn scope(&self) -> String {
        crate::gamelist::scope(&self.view, self.platform.as_deref(), self.collection.as_deref())
    }
}

#[derive(Serialize)]
pub struct BindingsView {
    pub actions: &'static [crate::binds::Action],
    pub pad_buttons: &'static [crate::binds::PadButton],
    /// Button index -> action, `null` where a rebind cleared the button.
    ///
    /// The nulls are kept rather than dropped because "bound to nothing" and
    /// "not a button on this pad" are different answers to why a press did
    /// nothing, and the settings window has to be able to say which.
    pub pad_map: std::collections::BTreeMap<u8, Option<String>>,
    /// Action -> key, `null` when unbound.
    pub keys: std::collections::BTreeMap<String, Option<String>>,
    /// Action -> the button's own name, for the help page.
    pub pad_labels: std::collections::BTreeMap<String, String>,
    pub key_labels: std::collections::BTreeMap<String, String>,
}

pub fn bindings_view(b: &crate::binds::Bindings) -> BindingsView {
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
        actions: crate::binds::ACTIONS,
        pad_buttons: crate::binds::PAD_BUTTONS,
        pad_map: b.pad_map_swapped(cfg.controllers.swap_ab, cfg.controllers.swap_xy),
        keys: crate::binds::ACTIONS.iter().map(|a| (a.id.to_owned(), b.key_for(a.id))).collect(),
        pad_labels: crate::binds::ACTIONS
            .iter()
            .map(|a| (a.id.to_owned(), crate::binds::pad_label(b.pad_for(a.id))))
            .collect(),
        key_labels: crate::binds::ACTIONS
            .iter()
            .map(|a| (a.id.to_owned(), crate::binds::key_label(b.key_for(a.id).as_deref())))
            .collect(),
    }
}

pub fn save_bindings(b: &crate::binds::Bindings) -> Result<(), String> {
    use crate::config::set_table_entries;
    let keys: Vec<(String, Option<String>)> = crate::binds::ACTIONS
        .iter()
        .map(|a| (a.id.to_owned(), b.keys.get(a.id).cloned()))
        .collect();
    set_table_entries(&crate::config::path_str(), "bindings.keys", &keys).map_err(err)?;

    let pad: Vec<(String, Option<String>)> = crate::binds::PAD_BUTTONS
        .iter()
        .map(|p| {
            let index = p.index.to_string();
            let held = b.pad.get(&index).cloned();
            (index, held)
        })
        .collect();
    set_table_entries(&crate::config::path_str(), "bindings.pad", &pad).map_err(err)
}

pub fn ui_bindings(state: &AppState) -> CmdResult<BindingsView> {
    let b = state.bindings.lock().map_err(err)?;
    Ok(bindings_view(&b))
}

pub fn set_key_binding(
    state: &AppState,
    action: String,
    key: Option<String>,
) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    b.set_key(&action, key.as_deref());
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

pub fn set_pad_binding(
    state: &AppState,
    action: String,
    index: Option<u8>,
) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    b.set_pad(&action, index);
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

pub fn reset_bindings(state: &AppState, which: String) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    match which.as_str() {
        "pad" => b.reset_pad(),
        _ => b.reset_keys(),
    }
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

pub fn import_bindings(
    state: &AppState,
    keys: std::collections::BTreeMap<String, Option<String>>,
    pad: std::collections::BTreeMap<String, Option<String>>,
) -> CmdResult<BindingsView> {
    let mut b = state.bindings.lock().map_err(err)?;
    b.adopt(keys, pad);
    save_bindings(&b)?;
    Ok(bindings_view(&b))
}

#[derive(Serialize)]
pub struct ListControls {
    pub orders: &'static [crate::gamesort::Order],
    pub filters: &'static [crate::gamefilter::Filter],
}

pub fn list_controls() -> ListControls {
    ListControls { orders: crate::gamesort::ORDERS, filters: crate::gamefilter::FILTERS }
}

#[derive(Serialize)]
pub struct Arrangement {
    /// Row ids, narrowed and ordered. `null` when the backend is not holding
    /// this list — the caller then draws what it has rather than an order
    /// computed from somebody else's rows.
    pub ids: Option<Vec<i64>>,
    pub order: &'static str,
    pub order_label: &'static str,
    pub filters: Vec<String>,
    pub sortable: bool,
    pub filterable: bool,
}

pub fn arrangement(state: &AppState, list: &ListRef) -> Result<Arrangement, String> {
    let scope = list.scope();
    let chosen = state.chosen.lock().map_err(err)?;
    let order = chosen.order(&scope);
    let filters = chosen.filters(&scope);
    let held = state.list_rows.lock().map_err(err)?;
    let ids = (*state.list_scope.lock().map_err(err)? == scope)
        .then(|| crate::gamelist::arrange(&held, order.id, &filters));
    Ok(Arrangement {
        ids,
        order: order.id,
        order_label: order.label,
        filters: filters.into_iter().collect(),
        sortable: crate::gamelist::sortable(&list.view),
        filterable: crate::gamelist::filterable(&list.view),
    })
}

pub fn arrange_list(state: &AppState, list: ListRef) -> CmdResult<Arrangement> {
    arrangement(&state, &list)
}

pub fn set_list_order(
    state: &AppState,
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

pub fn cycle_list_order(
    state: &AppState,
    list: ListRef,
    delta: i32,
) -> CmdResult<Arrangement> {
    {
        let mut chosen = state.chosen.lock().map_err(err)?;
        let scope = list.scope();
        let next = crate::gamesort::cycle(chosen.order(&scope).id, delta);
        chosen.set_order(&scope, next.id);
    }
    arrangement(&state, &list)
}

pub fn toggle_list_filter(
    state: &AppState,
    list: ListRef,
    filter: String,
) -> CmdResult<Arrangement> {
    state.chosen.lock().map_err(err)?.toggle_filter(&list.scope(), &filter);
    arrangement(&state, &list)
}

pub fn clear_list_filters(state: &AppState, list: ListRef) -> CmdResult<Arrangement> {
    state.chosen.lock().map_err(err)?.clear_filters(&list.scope());
    arrangement(&state, &list)
}

#[derive(Serialize)]
pub struct PickerArrangement {
    /// Indices into the rows that were handed over.
    pub order: Vec<usize>,
    /// The orders this kind of list offers. Empty for consoles, which get the
    /// alphabet and no button.
    pub orders: &'static [crate::pickorder::PickerOrder],
    pub chosen: Option<&'static str>,
    pub label: Option<&'static str>,
}

pub fn sort_picker(
    state: &AppState,
    kind: String,
    rows: Vec<crate::pickorder::PickerRow>,
) -> CmdResult<PickerArrangement> {
    let orders = state.picker_order.lock().map_err(err)?;
    let chosen = orders.get(&kind);
    Ok(PickerArrangement {
        order: crate::pickorder::sort(&rows, chosen.map(|o| o.id)),
        orders: crate::pickorder::orders_for(&kind),
        chosen: chosen.map(|o| o.id),
        label: chosen.map(|o| o.label),
    })
}

pub fn picker_controls(state: &AppState, kind: String) -> CmdResult<PickerArrangement> {
    let orders = state.picker_order.lock().map_err(err)?;
    let chosen = orders.get(&kind);
    Ok(PickerArrangement {
        order: Vec::new(),
        orders: crate::pickorder::orders_for(&kind),
        chosen: chosen.map(|o| o.id),
        label: chosen.map(|o| o.label),
    })
}

pub fn set_picker_order(state: &AppState, kind: String, order: String) -> CmdResult<()> {
    state.picker_order.lock().map_err(err)?.set(&kind, &order);
    crate::config::set_table_entry(&crate::config::path_str(), "picker_order", &kind, &order)
        .map_err(err)
}

#[derive(Serialize)]
pub struct PageFilterResult {
    pub visible: Vec<bool>,
    pub headings: Vec<bool>,
    pub shown: usize,
}

pub fn set_page_names(
    state: &AppState,
    names: Vec<String>,
    groups: Option<Vec<Vec<usize>>>,
) -> CmdResult<()> {
    *state.page_names.lock().map_err(err)? = (names, groups.unwrap_or_default());
    Ok(())
}

pub fn page_filter(state: &AppState, query: String) -> CmdResult<PageFilterResult> {
    let held = state.page_names.lock().map_err(err)?;
    let (names, groups) = &*held;
    let visible = crate::pagefilter::visible(names, &query);
    Ok(PageFilterResult {
        headings: crate::pagefilter::empty_groups(groups, &visible, &query),
        shown: visible.iter().filter(|v| **v).count(),
        visible,
    })
}

pub fn grid_uniform(count: usize, columns: usize) -> crate::gridnav::Moves {
    crate::gridnav::uniform(count, columns)
}

pub fn set_grid(cards: Vec<[f64; 3]>) -> crate::gridnav::Moves {
    let cards: Vec<crate::gridnav::Card> = cards
        .into_iter()
        .map(|[top, left, width]| crate::gridnav::Card { top, left, width })
        .collect();
    crate::gridnav::moves(&cards)
}
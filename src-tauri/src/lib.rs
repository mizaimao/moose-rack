//! The desktop shell.
//!
//! The backend lives in `moose_rack::commands` so the HTTP service calls the
//! same functions rather than a copy. What stays here is what genuinely needs a
//! window: native dialogs, opening the settings window, emitting events to the
//! webview, and the Tauri builder itself.
//!
//! Everything under the wrappers heading is one line. If you are adding a
//! command, write it in `moose_rack::commands` and wrap it here -- logic in this
//! file is logic the server cannot reach.

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


use moose_rack::{
    api, cache, config::Config, coremap::{self, CoreMap}, download, media, retroarch::RetroArch,
    savesync, shaders, syncplan, theme, theme_remote, util,
};


use moose_rack::app::AppState;
use moose_rack::commands::*;

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

#[tauri::command]
async fn download_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    choice: DownloadChoice,
) -> CmdResult<String> {
    use moose_rack::{bulk, diskspace};

    let client = state.client.clone().ok_or("no server configured")?;
    let rows = rows_for_choice(&*state, &choice.platforms, &choice.collection, &choice.collections)?;
    let want = choice.want();
    let est = bulk::estimate(&rows, want, |r| row_path(&*state, r).is_some());
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
        if row_path(&*state, row).is_none() {
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
                let (dir, key) = media_scope(&*state, row);
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
            let (scope_dir, scope_key) = media_scope(&*state, row);
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
    let path = row_path(&*state, &row).ok_or("not downloaded yet")?;
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
        autofire_hz: autofire_hz(&*state),
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
        auto_sync(&*state, ra, &row, savesync::When::BeforeLaunch).await
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
        auto_sync(&*state, ra, &row, savesync::When::AfterExit).await
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

#[tauri::command]
async fn install_icon_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    dir: String,
) -> CmdResult<String> {
    fetch_icon_set(&app, &state, &dir).await
}

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

#[tauri::command]
fn set_app_icon(app: tauri::AppHandle, id: String) -> CmdResult<String> {
    let icon = moose_rack::appicon::set(&id).map_err(err)?;
    let dir = appicon_dir(&app, icon.id)
        .ok_or_else(|| format!("{} has no built files — run scripts/build-appicons.sh", icon.id))?;
    Ok(apply_app_icon(&app, icon.id, &dir))
}

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

#[tauri::command]
fn measure_note(text: String) {
    if std::env::var_os("MOOSE_MEASURE").is_some() {
        println!("MEASURE {text}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_log();
    moose_rack::datadir::anchor();
    // Built by `moose_rack::app::AppState::from_config`, so the desktop window
    // and the HTTP service start from the same state rather than two setups
    // that drift.
    let state = AppState::from_config().expect("building app state");
    tauri::Builder::default()
        .manage(state)
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

// ---- wrappers over moose_rack::commands ----

#[tauri::command]
fn config_fields() -> CmdResult<ConfigFields> {
    moose_rack::commands::config_fields()
}

#[tauri::command]
fn set_config_field(field: String, value: String) -> CmdResult<String> {
    moose_rack::commands::set_config_field(field, value)
}

#[tauri::command]
async fn verify_server(
    url: String,
    token: Option<String>,
    username: Option<String>,
    password: Option<String>,
) -> CmdResult<String> {
    moose_rack::commands::verify_server(url, token, username, password).await
}

#[tauri::command]
async fn bios_status(state: State<'_, AppState>) -> CmdResult<(usize, usize, u64)> {
    moose_rack::commands::bios_status(&*state).await
}

#[tauri::command]
fn versions(state: State<'_, AppState>) -> CmdResult<(String, Option<String>)> {
    moose_rack::commands::versions(&*state)
}

#[tauri::command]
fn open_link(url: String) -> CmdResult<()> {
    moose_rack::commands::open_link(url)
}

#[tauri::command]
async fn verify_achievements() -> CmdResult<moose_rack::achievements::Verified> {
    moose_rack::commands::verify_achievements().await
}

#[tauri::command]
fn game_states(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<StateView>> {
    moose_rack::commands::game_states(&*state, id)
}

#[tauri::command]
fn delete_state(state: State<'_, AppState>, id: i64, slot: String) -> CmdResult<String> {
    moose_rack::commands::delete_state(&*state, id, slot)
}

#[tauri::command]
fn confirm_delete_state() -> CmdResult<bool> {
    moose_rack::commands::confirm_delete_state()
}

#[tauri::command]
fn play_history(state: State<'_, AppState>) -> CmdResult<History> {
    moose_rack::commands::play_history(&*state)
}

#[tauri::command]
fn recent_games(
    state: State<'_, AppState>,
    limit: Option<usize>,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::recent_games(&*state, limit, list)
}

#[tauri::command]
async fn download_estimate(
    state: State<'_, AppState>,
    choice: DownloadChoice,
) -> CmdResult<(String, bool, String)> {
    moose_rack::commands::download_estimate(&*state, choice).await
}

#[tauri::command]
fn platforms(state: State<'_, AppState>) -> CmdResult<Vec<PlatformView>> {
    moose_rack::commands::platforms(&*state)
}

#[tauri::command]
fn roms(
    state: State<'_, AppState>,
    platform: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::roms(&*state, platform, list)
}

#[tauri::command]
fn collection_groups(state: State<'_, AppState>) -> CmdResult<Vec<GroupView>> {
    moose_rack::commands::collection_groups(&*state)
}

#[tauri::command]
fn collections_in(state: State<'_, AppState>, group: String) -> CmdResult<Vec<CollectionView>> {
    moose_rack::commands::collections_in(&*state, group)
}

#[tauri::command]
fn collection_roms(
    state: State<'_, AppState>,
    id: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::collection_roms(&*state, id, list)
}

#[tauri::command]
fn search(
    state: State<'_, AppState>,
    term: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::search(&*state, term, list)
}

#[tauri::command]
async fn rom_detail(state: State<'_, AppState>, id: i64) -> CmdResult<RomDetail> {
    moose_rack::commands::rom_detail(&*state, id).await
}

#[tauri::command]
async fn toggle_favorite(state: State<'_, AppState>, id: i64) -> CmdResult<bool> {
    moose_rack::commands::toggle_favorite(&*state, id).await
}

#[tauri::command]
fn list_art_options(state: State<'_, AppState>) -> CmdResult<(Vec<(String, String)>, String)> {
    moose_rack::commands::list_art_options(&*state)
}

#[tauri::command]
fn set_list_art(state: State<'_, AppState>, value: String) -> CmdResult<String> {
    moose_rack::commands::set_list_art(&*state, value)
}

#[tauri::command]
async fn game_video(state: State<'_, AppState>, id: i64) -> CmdResult<String> {
    moose_rack::commands::game_video(&*state, id).await
}

#[tauri::command]
fn set_autofire_hz(state: State<'_, AppState>, hz: u32) -> CmdResult<u32> {
    moose_rack::commands::set_autofire_hz(&*state, hz)
}

#[tauri::command]
fn game_displays() -> CmdResult<Vec<DisplayView>> {
    moose_rack::commands::game_displays()
}

#[tauri::command]
fn android_launch_plan(
    state: State<'_, AppState>,
    id: i64,
    retroarch_package: String,
    config_dir: String,
    pad: Option<String>,
    refresh: Option<f32>,
) -> CmdResult<AndroidPlan> {
    moose_rack::commands::android_launch_plan(&*state, id, retroarch_package, config_dir, pad, refresh)
}

#[tauri::command]
async fn warm_media(state: State<'_, AppState>, platform: String) -> CmdResult<()> {
    moose_rack::commands::warm_media(&*state, platform).await
}

#[tauri::command]
async fn android_sync_before(state: State<'_, AppState>, id: i64) -> CmdResult<String> {
    moose_rack::commands::android_sync_before(&*state, id).await
}

#[tauri::command]
async fn android_after_play(
    state: State<'_, AppState>,
    id: i64,
    seconds: i64,
) -> CmdResult<String> {
    moose_rack::commands::android_after_play(&*state, id, seconds).await
}

#[tauri::command]
async fn resolve_save_conflict(
    state: State<'_, AppState>,
    file_name: String,
    keep: moose_rack::savesync::Keep,
) -> CmdResult<String> {
    moose_rack::commands::resolve_save_conflict(&*state, file_name, keep).await
}

#[tauri::command]
async fn icon_sets(state: State<'_, AppState>) -> CmdResult<Vec<moose_rack::iconsets::IconSetView>> {
    moose_rack::commands::icon_sets(&*state).await
}

#[tauri::command]
async fn check_update(
    state: State<'_, AppState>,
) -> CmdResult<Option<moose_rack::update::Update>> {
    moose_rack::commands::check_update(&*state).await
}

#[tauri::command]
fn config_findings() -> CmdResult<Vec<ConfigFinding>> {
    moose_rack::commands::config_findings()
}

#[tauri::command]
fn config_patch() -> CmdResult<String> {
    moose_rack::commands::config_patch()
}

#[tauri::command]
fn game_lightgun(state: State<'_, AppState>, id: i64) -> CmdResult<Option<(String, String)>> {
    moose_rack::commands::game_lightgun(&*state, id)
}

#[tauri::command]
fn set_icon_set(state: State<'_, AppState>, dir: String) -> CmdResult<String> {
    moose_rack::commands::set_icon_set(&*state, dir)
}

#[tauri::command]
fn remove_icon_set(state: State<'_, AppState>, dir: String) -> CmdResult<String> {
    moose_rack::commands::remove_icon_set(&*state, dir)
}

#[tauri::command]
fn systems(state: State<'_, AppState>) -> CmdResult<Vec<SystemView>> {
    moose_rack::commands::systems(&*state)
}

#[tauri::command]
async fn sync_saves_plan(state: State<'_, AppState>) -> CmdResult<syncplan::Review> {
    moose_rack::commands::sync_saves_plan(&*state).await
}

#[tauri::command]
async fn attract_pool(state: State<'_, AppState>) -> CmdResult<Vec<AttractPick>> {
    moose_rack::commands::attract_pool(&*state).await
}

#[tauri::command]
async fn sync_saves(state: State<'_, AppState>) -> CmdResult<SyncRun> {
    moose_rack::commands::sync_saves(&*state).await
}

#[tauri::command]
fn motion_options(state: State<'_, AppState>) -> CmdResult<MotionView> {
    moose_rack::commands::motion_options(&*state)
}

#[tauri::command]
fn set_motion_shader(state: State<'_, AppState>, value: String) -> CmdResult<String> {
    moose_rack::commands::set_motion_shader(&*state, value)
}

#[tauri::command]
fn set_system_choice(
    state: State<'_, AppState>,
    slug: String,
    field: String,
    value: String,
) -> CmdResult<String> {
    moose_rack::commands::set_system_choice(&*state, slug, field, value)
}

#[tauri::command]
fn game_cores(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<CoreChoice>> {
    moose_rack::commands::game_cores(&*state, id)
}

#[tauri::command]
fn set_game_core(state: State<'_, AppState>, id: i64, core: String) -> CmdResult<String> {
    moose_rack::commands::set_game_core(&*state, id, core)
}

#[tauri::command]
fn install_theme_logos(state: State<'_, AppState>) -> CmdResult<String> {
    moose_rack::commands::install_theme_logos(&*state)
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> CmdResult<Status> {
    moose_rack::commands::status(&*state)
}

#[tauri::command]
async fn disk_usage(state: State<'_, AppState>) -> CmdResult<u64> {
    moose_rack::commands::disk_usage(&*state).await
}

#[tauri::command]
fn set_retroarch_root(path: String) -> CmdResult<String> {
    moose_rack::commands::set_retroarch_root(path)
}

#[tauri::command]
fn icon_styles(state: State<'_, AppState>) -> CmdResult<Vec<IconStyleView>> {
    moose_rack::commands::icon_styles(&*state)
}

#[tauri::command]
fn set_icon_style(state: State<'_, AppState>, key: String) -> CmdResult<String> {
    moose_rack::commands::set_icon_style(&*state, key)
}

#[tauri::command]
fn ui_bindings(state: State<'_, AppState>) -> CmdResult<BindingsView> {
    moose_rack::commands::ui_bindings(&*state)
}

#[tauri::command]
fn set_key_binding(
    state: State<'_, AppState>,
    action: String,
    key: Option<String>,
) -> CmdResult<BindingsView> {
    moose_rack::commands::set_key_binding(&*state, action, key)
}

#[tauri::command]
fn set_pad_binding(
    state: State<'_, AppState>,
    action: String,
    index: Option<u8>,
) -> CmdResult<BindingsView> {
    moose_rack::commands::set_pad_binding(&*state, action, index)
}

#[tauri::command]
fn reset_bindings(state: State<'_, AppState>, which: String) -> CmdResult<BindingsView> {
    moose_rack::commands::reset_bindings(&*state, which)
}

#[tauri::command]
fn import_bindings(
    state: State<'_, AppState>,
    keys: std::collections::BTreeMap<String, Option<String>>,
    pad: std::collections::BTreeMap<String, Option<String>>,
) -> CmdResult<BindingsView> {
    moose_rack::commands::import_bindings(&*state, keys, pad)
}

#[tauri::command]
fn list_controls() -> ListControls {
    moose_rack::commands::list_controls()
}

#[tauri::command]
fn arrange_list(state: State<'_, AppState>, list: ListRef) -> CmdResult<Arrangement> {
    moose_rack::commands::arrange_list(&*state, list)
}

#[tauri::command]
fn set_list_order(
    state: State<'_, AppState>,
    list: ListRef,
    order: String,
    preferred: Option<bool>,
) -> CmdResult<Arrangement> {
    moose_rack::commands::set_list_order(&*state, list, order, preferred)
}

#[tauri::command]
fn cycle_list_order(
    state: State<'_, AppState>,
    list: ListRef,
    delta: i32,
) -> CmdResult<Arrangement> {
    moose_rack::commands::cycle_list_order(&*state, list, delta)
}

#[tauri::command]
fn toggle_list_filter(
    state: State<'_, AppState>,
    list: ListRef,
    filter: String,
) -> CmdResult<Arrangement> {
    moose_rack::commands::toggle_list_filter(&*state, list, filter)
}

#[tauri::command]
fn clear_list_filters(state: State<'_, AppState>, list: ListRef) -> CmdResult<Arrangement> {
    moose_rack::commands::clear_list_filters(&*state, list)
}

#[tauri::command]
fn sort_picker(
    state: State<'_, AppState>,
    kind: String,
    rows: Vec<moose_rack::pickorder::PickerRow>,
) -> CmdResult<PickerArrangement> {
    moose_rack::commands::sort_picker(&*state, kind, rows)
}

#[tauri::command]
fn picker_controls(state: State<'_, AppState>, kind: String) -> CmdResult<PickerArrangement> {
    moose_rack::commands::picker_controls(&*state, kind)
}

#[tauri::command]
fn set_picker_order(state: State<'_, AppState>, kind: String, order: String) -> CmdResult<()> {
    moose_rack::commands::set_picker_order(&*state, kind, order)
}

#[tauri::command]
fn set_page_names(
    state: State<'_, AppState>,
    names: Vec<String>,
    groups: Option<Vec<Vec<usize>>>,
) -> CmdResult<()> {
    moose_rack::commands::set_page_names(&*state, names, groups)
}

#[tauri::command]
fn page_filter(state: State<'_, AppState>, query: String) -> CmdResult<PageFilterResult> {
    moose_rack::commands::page_filter(&*state, query)
}

#[tauri::command]
fn grid_uniform(count: usize, columns: usize) -> moose_rack::gridnav::Moves {
    moose_rack::commands::grid_uniform(count, columns)
}

#[tauri::command]
fn set_grid(cards: Vec<[f64; 3]>) -> moose_rack::gridnav::Moves {
    moose_rack::commands::set_grid(cards)
}

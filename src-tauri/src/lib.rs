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

use tauri::{Emitter, Manager, State};

use moose_rack::media;


use moose_rack::app::AppState;
use moose_rack::commands::*;

use moose_rack::busy::{self, Busy};

#[tauri::command]
async fn sync_library(app: tauri::AppHandle, state: State<'_, AppState>, full: bool) -> CmdResult<String> {
    moose_rack::commands::sync_library(&state, full, |what| {
        let _ = app.emit("sync-progress", what.to_owned());
    })
    .await
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
    let summary = moose_rack::bios::sync(&client, &state.system_dir(), &state.legacy_bios, |done, total, name| {
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
            let folder = row.folder();
            let target = moose_rack::download::Target {
                rom_id: row.id,
                members: &members,
                fs_name: &row.fs_name,
                folder: &folder,
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
        let _ = app.emit("bulk-progress", "BIOS files…".to_owned());
        match moose_rack::bios::sync(&client, &state.system_dir(), &state.legacy_bios, |done, got, name| {
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
    // The window's only part in this is the progress event; the fetching itself
    // is shared with the server.
    moose_rack::commands::rom_covers(&state, ids, local_only, &move |batch| {
        let _ = app.emit("covers-ready", batch);
    })
    .await
}

#[tauri::command]
async fn download_rom(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> CmdResult<String> {
    let mut last = std::time::Instant::now();
    let said = moose_rack::commands::download_game(&state, id, |done, total| {
        // Throttle: the webview does not need 60 events a second.
        if last.elapsed().as_millis() < 100 {
            return;
        }
        last = std::time::Instant::now();
        let _ = app.emit("download-progress", (id, done, total));
    })
    .await;
    let _ = app.emit("download-progress", (id, 1u64, 1u64));
    said
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

// Each argument is a key in the page's `invoke("launch_rom", {...})`, so they
// stay separate rather than being folded into a struct the page would have to
// match.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn launch_rom(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
    pad: Option<String>,
    refresh: Option<f32>,
    skip_sync: Option<bool>,
    skip_bios: Option<bool>,
    entry_slot: Option<u32>,
    busy: State<'_, Busy>,
) -> CmdResult<String> {
    // The whole sequence is shared with the other front ends; this window adds
    // where the game goes and how progress is shown.
    let opts = moose_rack::play::Options {
        pad,
        refresh,
        screen: work_area(&app),
        skip_sync,
        skip_bios,
        entry_slot,
    };
    let say = |what: &str| {
        let _ = app.emit("launch-progress", what.to_owned());
    };
    moose_rack::play::launch(&state, &busy, id, opts, say).await
}


/// The launch checklist: is the game here, is the emulator ready.
#[tauri::command]
async fn launch_checklist(state: State<'_, AppState>, id: i64) -> CmdResult<moose_rack::play::Checklist> {
    moose_rack::play::checklist(&state, id).await
}

/// The checklist's Download button: fetch whatever is missing, then the page
/// checks again.
#[tauri::command]
async fn prepare_launch(app: tauri::AppHandle, state: State<'_, AppState>, id: i64) -> CmdResult<String> {
    let say = |what: &str| {
        let _ = app.emit("launch-progress", what.to_owned());
    };
    moose_rack::play::prepare(&state, id, say).await
}

/// Download and set up a standalone emulator, for the page's "Install it?".
#[tauri::command]
async fn install_emulator(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<String> {
    use moose_rack::standalone;
    let emu = standalone::by_id(&id).ok_or_else(|| format!("no emulator called {id}"))?;
    let dir = standalone::dir(&state.library_root, emu);
    let http = moose_rack::util::http_client(None).map_err(err)?;
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    standalone::install(&http, emu, &dir, |done, total| {
        if last.elapsed().as_millis() < 250 {
            return;
        }
        last = std::time::Instant::now();
        let pct = (done * 100).checked_div(total).unwrap_or(0);
        let _ = app.emit("launch-progress", format!("downloading {} {}… {pct}%", emu.name, emu.version));
    })
    .await
    .map_err(err)?;
    Ok(format!("installed {} {}", emu.name, emu.version))
}

#[tauri::command]
async fn install_icon_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    dir: String,
) -> CmdResult<String> {
    moose_rack::commands::install_icon_set(&state, dir, &progress(app)).await
}

/// The window's part in a fetch: saying how far along it is.
fn progress(app: tauri::AppHandle) -> impl Fn(&str) + Send + Sync {
    move |line: &str| {
        let _ = app.emit("icons-progress", line.to_owned());
    }
}

#[tauri::command]
async fn fetch_icons(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    moose_rack::commands::fetch_icons(&state, &progress(app)).await
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
    use tauri::menu::{AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};

    // Our own Quit rather than the standard one: the standard item ends the
    // process straight away, with no event the app can answer, so Cmd+Q in
    // the middle of a game would have skipped the save upload. This one asks
    // `may_quit` first. (The Dock's Quit still goes the standard way.)
    let quit = MenuItemBuilder::with_id("quit", "Quit Moose Rack")
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;
    app.on_menu_event(|app, event| {
        if event.id() == "quit" && may_quit(app) {
            app.exit(0);
        }
    });

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
        .item(&quit)
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

// ---------------------------------------------------------------------------
// EmulatorJS in the desktop window.
//
// The web UI plays a game in the page when the machine holding the library is
// not the machine in front of you. The desktop has the same need for a
// different reason: RetroArch may have no core for this system, or not be
// installed at all, and the window should still be able to run a cartridge.
//
// The web build gets EmulatorJS over HTTP from the service. There is no HTTP
// server behind the desktop window, and the 296 MB of vendored cores must not
// be embedded in the binary -- they are not even in git. So the window reads
// them off disk through a URI scheme of its own: `moose://localhost/data/...`
// for EmulatorJS, `moose://localhost/rom/<id>` for the game.
//
// A scheme, rather than the asset protocol that serves artwork, because
// EmulatorJS builds its own URLs by appending to `EJS_pathtodata`, and the
// asset protocol's percent-encoded whole-path form cannot be appended to.
// ---------------------------------------------------------------------------

/// Where EmulatorJS is, decided once. `None` means it was never fetched, which
/// is a normal state for a fresh clone -- see `scripts/fetch-emulatorjs.sh`.
fn ejs_dir() -> Option<&'static std::path::Path> {
    static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let config = moose_rack::config::path();
        let candidates =
            moose_rack::ejs::desktop_candidates(exe.as_deref(), &cwd, Some(config.as_path()));
        moose_rack::ejs::locate(candidates.iter().map(std::path::PathBuf::as_path))
    })
    .as_deref()
}

/// The base the page should build EmulatorJS URLs from, or `None` when playing
/// in the window is not available on this install.
///
/// Windows has no custom schemes in WebView2 and Tauri maps them onto
/// `http://<scheme>.localhost`; everywhere else it is a real scheme. The rule
/// lives here rather than in the page because it is a fact about the host.
#[tauri::command]
async fn local_save(state: State<'_, AppState>, id: i64) -> CmdResult<Option<moose_rack::commands::LocalSave>> {
    moose_rack::commands::local_save(&state, id)
}

#[tauri::command]
fn put_local_save(state: State<'_, AppState>, id: i64, data: String) -> CmdResult<String> {
    moose_rack::commands::put_local_save(&state, id, data)
}

#[tauri::command]
fn browser_play_base() -> Option<String> {
    ejs_dir()?;
    Some(if cfg!(windows) {
        "http://moose.localhost/".to_string()
    } else {
        "moose://localhost/".to_string()
    })
}

/// A response with the headers every reply from this scheme needs.
///
/// `Access-Control-Allow-Origin` because the page is `tauri://localhost` and
/// this is a different origin: without it EmulatorJS's core fetches are blocked
/// and the emulator hangs on "Downloading core" with nothing in the console.
fn ejs_reply(
    status: u16,
    content_type: &str,
    extra: Vec<(&'static str, String)>,
    body: Vec<u8>,
) -> tauri::http::Response<Vec<u8>> {
    let mut b = tauri::http::Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .header("Cross-Origin-Resource-Policy", "cross-origin")
        // Without this the page can read the body of a 206 but not the header
        // saying which bytes it got: `Content-Range` is not CORS-safelisted, so
        // a cross-origin fetch hides it. Verified in the window -- the ranges
        // were right and `content-range` read back as null.
        .header("Access-Control-Expose-Headers", "Content-Range, Content-Length, Accept-Ranges")
        .header("Content-Type", content_type);
    for (k, v) in extra {
        b = b.header(k, v);
    }
    b.body(body).unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

fn ejs_error(status: u16, why: &str) -> tauri::http::Response<Vec<u8>> {
    ejs_reply(status, "text/plain", Vec::new(), why.as_bytes().to_vec())
}

/// One file, through the tested reader in `moose_rack::ejs`.
fn ejs_file(path: &std::path::Path, range: Option<&str>) -> tauri::http::Response<Vec<u8>> {
    let r = moose_rack::ejs::read_range(path, range);
    ejs_reply(r.status, r.content_type, r.headers, r.body)
}

/// Serve `moose://localhost/...` to the window.
fn ejs_serve(app: &tauri::AppHandle, req: tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let range = req
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let path = req.uri().path().to_string();
    let Some(root) = ejs_dir() else {
        return ejs_error(503, "EmulatorJS is not installed -- run scripts/fetch-emulatorjs.sh");
    };
    if let Some(rest) = path.strip_prefix("/rom/") {
        let Ok(id) = rest.parse::<i64>() else {
            return ejs_error(400, "no id");
        };
        use tauri::Manager as _;
        let state = app.state::<AppState>();
        let row = state
            .cache
            .lock()
            .ok()
            .and_then(|c| c.rom_by_id(id).ok().flatten());
        let Some(row) = row else {
            return ejs_error(404, "no such game");
        };
        let Some(file) = moose_rack::commands::row_path(&state, &row) else {
            return ejs_error(404, "not on this machine");
        };
        if !file.is_file() {
            return ejs_error(409, "this game is a folder, not a file");
        }
        return ejs_file(&file, range.as_deref());
    }
    match moose_rack::ejs::resolve(root, &path) {
        Some(file) => ejs_file(&file, range.as_deref()),
        None => ejs_error(404, "not found"),
    }
}

/// A play link that has arrived and the page hasn't picked up yet.
///
/// Held as well as announced: a link that starts the app arrives before the
/// page has loaded and is listening, so the page asks for it once it's ready,
/// and again each time `play-link` says another one came in.
#[derive(Default)]
struct PendingPlay(std::sync::Mutex<Option<moose_rack::playlink::PlayLink>>);

/// Keep the last valid play link among `urls` and tell the page.
///
/// The last one, because two clicks before the page answers mean the second
/// game. Anything that doesn't parse is dropped without a word: any web page
/// can fire a `moose-rack://` address.
#[cfg(desktop)]
fn take_links(app: &tauri::AppHandle, urls: impl IntoIterator<Item = String>) {
    let Some(link) = urls.into_iter().filter_map(|u| moose_rack::playlink::parse(&u)).last() else {
        return;
    };
    eprintln!("play link: {}/{}", link.platform, link.file);
    if let Ok(mut slot) = app.state::<PendingPlay>().0.lock() {
        *slot = Some(link);
    }
    show_main(app);
    let _ = app.emit_to("main", "play-link", ());
}

/// Bring the main window to the front, as when a link or a second launch asks
/// for the app.
#[cfg(desktop)]
fn show_main(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn take_play_link(pending: State<'_, PendingPlay>) -> Option<moose_rack::playlink::PlayLink> {
    pending.0.lock().ok()?.take()
}

#[tauri::command]
async fn find_rom_by_file(state: State<'_, AppState>, platform: String, file: String) -> CmdResult<Option<i64>> {
    moose_rack::commands::find_rom_by_file(&state, platform, file)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_log();
    moose_rack::datadir::anchor();
    // Built by `moose_rack::app::AppState::from_config`, so the desktop window
    // and the HTTP service start from the same state rather than two setups
    // that drift.
    let state = AppState::from_config().expect("building app state");
    let builder = tauri::Builder::default();
    // Play links. Single-instance has to be the first plugin registered: a
    // link opened on Windows or Linux starts a second copy of the app, and
    // this is what turns that into a link for the copy already running.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main(app)))
        .plugin(tauri_plugin_deep_link::init());
    builder
        // The native folder picker behind every Browse button in Settings.
        // Granted in capabilities/default.json, but a permission for a plugin
        // that was never registered still fails at runtime: f47a312 dropped
        // this line and every Browse button went dead with nothing at build
        // time to say so. `the_plugins_granted_are_the_plugins_registered`
        // below is what catches that now.
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(PendingPlay::default())
        .manage(busy::Busy::default())
        // Closing the main window quits the app, so it asks `may_quit` first.
        // The settings window closes freely.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && !may_quit(window.app_handle())
            {
                api.prevent_close();
            }
        })
        // EmulatorJS and the ROM it plays, read off disk. See `ejs_serve`.
        .register_uri_scheme_protocol("moose", |ctx, req| ejs_serve(ctx.app_handle(), req))
        .invoke_handler(tauri::generate_handler![
            measure_note,
            browser_play_base,
            take_play_link,
            install_emulator,
            launch_checklist,
            prepare_launch,
            find_rom_by_file,
            stable_ids,
            local_save,
            put_local_save,
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
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                // macOS takes the scheme from the bundle's Info.plist. Windows
                // and Linux need it written down at run time, because the app
                // runs portable rather than from an installer (on Windows a
                // key under HKEY_CURRENT_USER\Software\Classes).
                #[cfg(any(windows, target_os = "linux"))]
                if let Err(e) = app.deep_link().register_all() {
                    eprintln!("play links: cannot register {}://: {e}", moose_rack::playlink::SCHEME);
                }
                // A link that started the app arrives here; one opened while it
                // runs arrives through on_open_url.
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    take_links(app.handle(), urls.iter().map(|u| u.to_string()));
                }
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    take_links(&handle, event.urls().iter().map(|u| u.to_string()));
                });
            }
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
                // Off the side by default. `MOOSE_MEASURE_POS=x,y` puts it back
                // on the display, which the emulator needs: a window that is
                // not rendered gets no `requestAnimationFrame`, and EmulatorJS
                // draws every frame from one. Weighing the app wants it out of
                // sight; watching a game actually run needs it in sight.
                let (px, py) = std::env::var("MOOSE_MEASURE_POS")
                    .ok()
                    .and_then(|s| {
                        let (x, y) = s.split_once(',')?;
                        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                    })
                    .unwrap_or((-4000.0, 200.0));
                let _ = win.set_position(tauri::LogicalPosition::new(px, py));
                // And in front, when it was asked for on the display. WebKit
                // gives a window it does not consider visible no animation
                // frames at all, so a frame-timing measurement taken behind the
                // terminal that started it reads zero frames rather than slow
                // ones.
                if std::env::var_os("MOOSE_MEASURE_POS").is_some() {
                    let _ = win.set_focus();
                }
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
        .build(tauri::generate_context!())
        .expect("building tauri application")
        .run(|app, event| {
            // Any other way out: Quit in the menu, or `exit` from code. Only
            // while the main window is still there to show why; once it has
            // closed, `may_quit` has already said yes.
            if let tauri::RunEvent::ExitRequested { api, .. } = event
                && app.get_webview_window("main").is_some()
                && !may_quit(app)
            {
                api.prevent_exit();
            }
        });
}

/// Whether the app may quit now. When it may not, says why in a dialog and
/// returns false; "Quit anyway" on the upload question quits from there.
fn may_quit(app: &tauri::AppHandle) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let hold = app.state::<busy::Busy>().hold();
    if let Some(why) = &hold {
        eprintln!("quit held: {why:?}");
    }
    match hold {
        None => true,
        Some(busy::Hold::Playing(name)) => {
            app.dialog()
                .message(format!(
                    "{name} is still running. Quit the game first: Moose Rack uploads its saves when it ends."
                ))
                .title("Moose Rack")
                .kind(MessageDialogKind::Info)
                .show(|_| {});
            false
        }
        Some(busy::Hold::Uploading) => {
            let handle = app.clone();
            app.dialog()
                .message("Saves are still uploading to the server. If you quit now, the next sync picks them up.")
                .title("Moose Rack")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom("Quit anyway".into(), "Keep open".into()))
                .show(move |quit| {
                    if quit {
                        handle.state::<busy::Busy>().leave();
                        handle.exit(0);
                    }
                });
            false
        }
    }
}

// ---- wrappers over moose_rack::commands ----

#[tauri::command]
async fn config_fields() -> CmdResult<ConfigFields> {
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
    moose_rack::commands::bios_status(&state).await
}

#[tauri::command]
async fn versions(state: State<'_, AppState>) -> CmdResult<(String, Option<String>)> {
    moose_rack::commands::versions(&state)
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
async fn game_states(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<StateView>> {
    moose_rack::commands::game_states(&state, id)
}

#[tauri::command]
fn delete_state(state: State<'_, AppState>, id: i64, slot: String) -> CmdResult<String> {
    moose_rack::commands::delete_state(&state, id, slot)
}

#[tauri::command]
fn confirm_delete_state() -> CmdResult<bool> {
    moose_rack::commands::confirm_delete_state()
}

#[tauri::command]
async fn play_history(state: State<'_, AppState>) -> CmdResult<History> {
    moose_rack::commands::play_history(&state)
}

#[tauri::command]
async fn recent_games(
    state: State<'_, AppState>,
    limit: Option<usize>,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::recent_games(&state, limit, list)
}

#[tauri::command]
async fn download_estimate(
    state: State<'_, AppState>,
    choice: DownloadChoice,
) -> CmdResult<(String, bool, String)> {
    moose_rack::commands::download_estimate(&state, choice).await
}

#[tauri::command]
async fn stable_ids(state: State<'_, AppState>, ids: Vec<i64>) -> CmdResult<moose_rack::commands::StableIds> {
    moose_rack::commands::stable_ids(&state, ids)
}

#[tauri::command]
async fn platforms(state: State<'_, AppState>) -> CmdResult<Vec<PlatformView>> {
    moose_rack::commands::platforms(&state)
}

#[tauri::command]
async fn roms(
    state: State<'_, AppState>,
    platform: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::roms(&state, platform, list)
}

#[tauri::command]
async fn collection_groups(state: State<'_, AppState>) -> CmdResult<Vec<GroupView>> {
    moose_rack::commands::collection_groups(&state)
}

#[tauri::command]
async fn collections_in(state: State<'_, AppState>, group: String) -> CmdResult<Vec<CollectionView>> {
    moose_rack::commands::collections_in(&state, group)
}

#[tauri::command]
async fn collection_roms(
    state: State<'_, AppState>,
    id: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::collection_roms(&state, id, list)
}

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    term: String,
    list: Option<ListRef>,
) -> CmdResult<Vec<RomView>> {
    moose_rack::commands::search(&state, term, list)
}

#[tauri::command]
async fn rom_detail(state: State<'_, AppState>, id: i64) -> CmdResult<RomDetail> {
    moose_rack::commands::rom_detail(&state, id).await
}

#[tauri::command]
async fn toggle_favorite(state: State<'_, AppState>, id: i64) -> CmdResult<bool> {
    moose_rack::commands::toggle_favorite(&state, id).await
}

#[tauri::command]
async fn list_art_options(state: State<'_, AppState>) -> CmdResult<(Vec<(String, String)>, String)> {
    moose_rack::commands::list_art_options(&state)
}

#[tauri::command]
fn set_list_art(state: State<'_, AppState>, value: String) -> CmdResult<String> {
    moose_rack::commands::set_list_art(&state, value)
}

#[tauri::command]
async fn game_video(state: State<'_, AppState>, id: i64) -> CmdResult<String> {
    moose_rack::commands::game_video(&state, id).await
}

#[tauri::command]
fn set_autofire_hz(state: State<'_, AppState>, hz: u32) -> CmdResult<u32> {
    moose_rack::commands::set_autofire_hz(&state, hz)
}

#[tauri::command]
async fn game_displays() -> CmdResult<Vec<DisplayView>> {
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
    moose_rack::commands::android_launch_plan(&state, id, retroarch_package, config_dir, pad, refresh)
}

#[tauri::command]
async fn warm_media(state: State<'_, AppState>, platform: String) -> CmdResult<()> {
    moose_rack::commands::warm_media(&state, platform).await
}

#[tauri::command]
async fn android_sync_before(state: State<'_, AppState>, id: i64) -> CmdResult<String> {
    moose_rack::commands::android_sync_before(&state, id).await
}

#[tauri::command]
async fn android_after_play(
    state: State<'_, AppState>,
    id: i64,
    seconds: i64,
) -> CmdResult<String> {
    moose_rack::commands::android_after_play(&state, id, seconds).await
}

#[tauri::command]
async fn resolve_save_conflict(
    state: State<'_, AppState>,
    file_name: String,
    keep: moose_rack::savesync::Keep,
    busy: State<'_, busy::Busy>,
) -> CmdResult<String> {
    let _uploading = busy.uploading();
    moose_rack::commands::resolve_save_conflict(&state, file_name, keep).await
}

#[tauri::command]
async fn icon_sets(state: State<'_, AppState>) -> CmdResult<Vec<moose_rack::iconsets::IconSetView>> {
    moose_rack::commands::icon_sets(&state).await
}

#[tauri::command]
async fn check_update(
    state: State<'_, AppState>,
) -> CmdResult<Option<moose_rack::update::Update>> {
    moose_rack::commands::check_update(&state).await
}

#[tauri::command]
async fn config_findings() -> CmdResult<Vec<ConfigFinding>> {
    moose_rack::commands::config_findings()
}

#[tauri::command]
fn config_patch() -> CmdResult<String> {
    moose_rack::commands::config_patch()
}

#[tauri::command]
async fn game_lightgun(state: State<'_, AppState>, id: i64) -> CmdResult<Option<(String, String)>> {
    moose_rack::commands::game_lightgun(&state, id)
}

#[tauri::command]
fn set_icon_set(state: State<'_, AppState>, dir: String) -> CmdResult<String> {
    moose_rack::commands::set_icon_set(&state, dir)
}

#[tauri::command]
fn remove_icon_set(state: State<'_, AppState>, dir: String) -> CmdResult<String> {
    moose_rack::commands::remove_icon_set(&state, dir)
}

#[tauri::command]
async fn systems(state: State<'_, AppState>) -> CmdResult<Vec<SystemView>> {
    moose_rack::commands::systems(&state)
}

#[tauri::command]
async fn sync_saves_plan(state: State<'_, AppState>) -> CmdResult<moose_rack::syncplan::Review> {
    moose_rack::commands::sync_saves_plan(&state).await
}

#[tauri::command]
async fn attract_pool(state: State<'_, AppState>) -> CmdResult<Vec<AttractPick>> {
    moose_rack::commands::attract_pool(&state).await
}

#[tauri::command]
async fn sync_saves(state: State<'_, AppState>, busy: State<'_, busy::Busy>) -> CmdResult<SyncRun> {
    let _uploading = busy.uploading();
    moose_rack::commands::sync_saves(&state).await
}

#[tauri::command]
async fn motion_options(state: State<'_, AppState>) -> CmdResult<MotionView> {
    moose_rack::commands::motion_options(&state)
}

#[tauri::command]
fn set_motion_shader(state: State<'_, AppState>, value: String) -> CmdResult<String> {
    moose_rack::commands::set_motion_shader(&state, value)
}

#[tauri::command]
fn set_system_choice(
    state: State<'_, AppState>,
    slug: String,
    field: String,
    value: String,
) -> CmdResult<String> {
    moose_rack::commands::set_system_choice(&state, slug, field, value)
}

#[tauri::command]
async fn game_cores(state: State<'_, AppState>, id: i64) -> CmdResult<Vec<CoreChoice>> {
    moose_rack::commands::game_cores(&state, id)
}

#[tauri::command]
fn set_game_core(state: State<'_, AppState>, id: i64, core: String) -> CmdResult<String> {
    moose_rack::commands::set_game_core(&state, id, core)
}

#[tauri::command]
fn install_theme_logos(state: State<'_, AppState>) -> CmdResult<String> {
    moose_rack::commands::install_theme_logos(&state)
}

#[tauri::command]
async fn status(state: State<'_, AppState>) -> CmdResult<Status> {
    moose_rack::commands::status(&state)
}

#[tauri::command]
async fn disk_usage(state: State<'_, AppState>) -> CmdResult<u64> {
    moose_rack::commands::disk_usage(&state).await
}

#[tauri::command]
fn set_retroarch_root(path: String) -> CmdResult<String> {
    moose_rack::commands::set_retroarch_root(path)
}

#[tauri::command]
async fn icon_styles(state: State<'_, AppState>) -> CmdResult<Vec<IconStyleView>> {
    moose_rack::commands::icon_styles(&state)
}

#[tauri::command]
fn set_icon_style(state: State<'_, AppState>, key: String) -> CmdResult<String> {
    moose_rack::commands::set_icon_style(&state, key)
}

#[tauri::command]
async fn ui_bindings(state: State<'_, AppState>) -> CmdResult<BindingsView> {
    moose_rack::commands::ui_bindings(&state)
}

#[tauri::command]
fn set_key_binding(
    state: State<'_, AppState>,
    action: String,
    key: Option<String>,
) -> CmdResult<BindingsView> {
    moose_rack::commands::set_key_binding(&state, action, key)
}

#[tauri::command]
fn set_pad_binding(
    state: State<'_, AppState>,
    action: String,
    index: Option<u8>,
) -> CmdResult<BindingsView> {
    moose_rack::commands::set_pad_binding(&state, action, index)
}

#[tauri::command]
fn reset_bindings(state: State<'_, AppState>, which: String) -> CmdResult<BindingsView> {
    moose_rack::commands::reset_bindings(&state, which)
}

#[tauri::command]
fn import_bindings(
    state: State<'_, AppState>,
    keys: std::collections::BTreeMap<String, Option<String>>,
    pad: std::collections::BTreeMap<String, Option<String>>,
) -> CmdResult<BindingsView> {
    moose_rack::commands::import_bindings(&state, keys, pad)
}

#[tauri::command]
async fn list_controls() -> ListControls {
    moose_rack::commands::list_controls()
}

#[tauri::command]
async fn arrange_list(state: State<'_, AppState>, list: ListRef) -> CmdResult<Arrangement> {
    moose_rack::commands::arrange_list(&state, list)
}

#[tauri::command]
fn set_list_order(
    state: State<'_, AppState>,
    list: ListRef,
    order: String,
    preferred: Option<bool>,
) -> CmdResult<Arrangement> {
    moose_rack::commands::set_list_order(&state, list, order, preferred)
}

#[tauri::command]
fn cycle_list_order(
    state: State<'_, AppState>,
    list: ListRef,
    delta: i32,
) -> CmdResult<Arrangement> {
    moose_rack::commands::cycle_list_order(&state, list, delta)
}

#[tauri::command]
fn toggle_list_filter(
    state: State<'_, AppState>,
    list: ListRef,
    filter: String,
) -> CmdResult<Arrangement> {
    moose_rack::commands::toggle_list_filter(&state, list, filter)
}

#[tauri::command]
fn clear_list_filters(state: State<'_, AppState>, list: ListRef) -> CmdResult<Arrangement> {
    moose_rack::commands::clear_list_filters(&state, list)
}

#[tauri::command]
async fn sort_picker(
    state: State<'_, AppState>,
    kind: String,
    rows: Vec<moose_rack::pickorder::PickerRow>,
) -> CmdResult<PickerArrangement> {
    moose_rack::commands::sort_picker(&state, kind, rows)
}

#[tauri::command]
async fn picker_controls(state: State<'_, AppState>, kind: String) -> CmdResult<PickerArrangement> {
    moose_rack::commands::picker_controls(&state, kind)
}

#[tauri::command]
fn set_picker_order(state: State<'_, AppState>, kind: String, order: String) -> CmdResult<()> {
    moose_rack::commands::set_picker_order(&state, kind, order)
}

#[tauri::command]
fn set_page_names(
    state: State<'_, AppState>,
    names: Vec<String>,
    groups: Option<Vec<Vec<usize>>>,
) -> CmdResult<()> {
    moose_rack::commands::set_page_names(&state, names, groups)
}

#[tauri::command]
async fn page_filter(
    state: State<'_, AppState>,
    query: String,
    names: Option<Vec<String>>,
    groups: Option<Vec<Vec<usize>>>,
) -> CmdResult<PageFilterResult> {
    moose_rack::commands::page_filter(&state, query, names, groups)
}

#[tauri::command]
async fn grid_uniform(count: usize, columns: usize) -> moose_rack::gridnav::Moves {
    moose_rack::commands::grid_uniform(count, columns)
}

#[tauri::command]
fn set_grid(cards: Vec<[f64; 3]>) -> moose_rack::gridnav::Moves {
    moose_rack::commands::set_grid(cards)
}

#[cfg(test)]
mod tests {
    /// Every plugin a capability grants has to be registered on the builder.
    ///
    /// A capability naming an unregistered plugin builds cleanly and fails only
    /// when the page calls it, so the Browse buttons in Settings were dead from
    /// f47a312 until someone pressed one. Read from the source because the
    /// builder cannot be run in a test without a window.
    #[test]
    fn the_plugins_granted_are_the_plugins_registered() {
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let source = include_str!("lib.rs");
        let granted = caps["permissions"].as_array().unwrap().iter().filter_map(|p| {
            let (plugin, _) = p.as_str()?.split_once(':')?;
            (plugin != "core").then_some(plugin)
        });
        for plugin in granted {
            let init = format!(".plugin(tauri_plugin_{}::init())", plugin.replace('-', "_"));
            assert!(source.contains(&init), "{plugin} is granted but `{init}` is not on the builder");
        }
    }
}

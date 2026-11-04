//! Play: everything between pressing Play and the game ending, shared by every
//! front end. The desktop app (`src-tauri`) and the Slint one call this; each
//! passes in what only its window knows (the screen, the pad, the refresh
//! rate) and shows progress its own way. See `docs/launch-flow.html`.
//!
//! The stops that need an answer come back as errors with a prefix the page
//! reads (`SAVE_CONFLICT:`, `SAVE_OFFLINE:`, `BIOS_MISSING:`,
//! `EMULATOR_MISSING:`), as they did when this lived in the desktop app.

use std::path::Path;

use crate::app::AppState;
use crate::busy::Busy;
use crate::commands::*;
use crate::config::Config;
use crate::{cache, coremap, savesync, shaders};

/// What the window knows about this launch.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// The connected pad's name, which picks RetroArch's autoconfig profile.
    pub pad: Option<String>,
    /// The display's refresh rate, measured by the page.
    pub refresh: Option<f32>,
    /// Where the game window goes.
    pub screen: Option<crate::retroarch::Screen>,
    /// Set by the retry after the user answered an offline warning.
    pub skip_sync: Option<bool>,
    /// Set by the retry after the user said "play anyway" to a missing BIOS.
    /// Separate from `skip_sync`: that answer is about the BIOS, and taking it
    /// as "skip the save sync" lost the sync on both sides of the game.
    pub skip_bios: Option<bool>,
    /// A save state to start in, chosen from the shelf in the info pane.
    pub entry_slot: Option<u32>,
}

/// Play the game with this id, start to finish. Returns the line to show when
/// it ends ("Ico exited cleanly; 1 save uploaded").
pub async fn launch(
    state: &AppState,
    busy: &Busy,
    id: i64,
    opts: Options,
    say: impl Fn(&str) + Send + Sync,
) -> CmdResult<String> {
    let Options { pad, refresh, screen, skip_sync, skip_bios, entry_slot } = opts;
    let row = {
        let cache = state.cache.lock().map_err(err)?;
        cache.rom_by_id(id).map_err(err)?
    }
    .ok_or_else(|| format!("no rom with id {id}"))?;
    eprintln!("launch: {} ({})", row.name, row.platform_slug);

    // Systems the app plays with a standalone emulator of its own (PCSX2 for
    // the PS2) never touch RetroArch.
    if let Some(emu) = crate::standalone::for_platform(&row.platform_slug) {
        let rom = row_path(state, &row).ok_or("not downloaded yet")?;
        return launch_standalone(state, busy, emu, &row, &rom, skip_sync.unwrap_or(false), &say).await;
    }

    let found = retroarch(state).ok_or("RetroArch not found")?;
    let ra = &found;
    // row_path, not local_path: the grid marks a row downloaded with this, and
    // a launch that disagreed said "not downloaded yet" about a folder ROM
    // sitting right there. launch::plan resolves the folder to its playlist.
    let path = row_path(state, &row).ok_or("not downloaded yet")?;
    // One shared planner for GUI, CLI and TUI — see launch.rs for why.
    let overrides = state.core_overrides.lock().map_err(err)?.clone();
    let per_game = state.core_per_game.lock().map_err(err)?.clone();
    let shader_overrides = state.shader_overrides.lock().map_err(err)?.clone();
    let motion = state.motion_shader.lock().map_err(err)?.clone();
    let lightgun = state.lightgun.lock().map_err(err)?.clone();
    // The app's library folder, where the CLI already put these. The folder
    // above the ROMs, which this was, is the home folder for `~/ROMs` and the
    // root of the card on a handheld.
    let lib = state.library_root.as_path();
    // Read per launch rather than held on AppState: it is a path the settings
    // window can change while the app is running, and a copy taken at startup
    // would send saves to the old folder until a restart.
    let saves_root = Config::load()
        .map(|c| crate::util::expand_tilde(&c.saves.root))
        .unwrap_or_default();
    let req = crate::launch::Request {
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
        autofire_hz: autofire_hz(state),
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
        screen,
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
    let mut fetched = Vec::new();
    // Reuses the API client's HTTP stack; without a configured server there is
    // nothing to download from anyway.
    if let (Some(core), Some(api)) = (wanted.as_deref(), state.client.as_ref()) {
        let http = api.http();
        say("checking the emulator core…");
        match crate::cores::ensure(http, ra, core).await {
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
        let bios_dir = state.system_dir();
        say("checking BIOS files…");
        match crate::bios::ensure(api, &bios_dir, &state.legacy_bios, core, &row.platform_slug).await {
            Ok(0) => {}
            Ok(n) => fetched.push(format!("fetched {n} BIOS file(s)")),
            Err(e) => {
                // Refused rather than noted. This used to go into the launch
                // notes and start the game anyway, which meant the one case
                // the automatic fetch cannot fix — a file the server has not
                // got either — arrived as a black screen with the explanation
                // scrolled past behind it. The front end offers "play anyway",
                // because a core that wants a BIOS sometimes runs without one.
                if !skip_bios.unwrap_or(false) {
                    let want = crate::bios::required_for(core, &row.platform_slug);
                    let missing: Vec<String> = want
                        .into_iter()
                        .filter(|n| !bios_dir.join(n).is_file())
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

    // The BIOS folder as it is now. `state.retroarch` was pointed at it at
    // startup only if it existed then, and on a fresh machine the fetch above
    // is what creates it: the first Neo Geo launch would otherwise download
    // aes.zip and still tell RetroArch to look somewhere else.
    let ra = &ra.clone().with_system_dir(Some(state.system_dir()));
    let plan = crate::launch::plan(ra, &state.map, &req).map_err(err)?;

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
        auto_sync(state, &row, savesync::When::BeforeLaunch).await
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

    let record = open_record(
        state,
        &row,
        skip_sync.unwrap_or(false),
        retroarch_config_lines(&plan, state),
        crate::launchrecord::Data {
            game_md5: row.md5_hash.clone(),
            emulator: format!("RetroArch, {}", plan.core),
            emulator_md5: crate::launchrecord::md5_of(&ra.core_path(&plan.core)),
            bios: bios_of(&state.system_dir(), crate::bios::required_for(&plan.core, &row.platform_slug)),
        },
    );
    if let Ok(cmd) = plan.command(ra, false) {
        eprintln!("launch: {}", crate::retroarch::render(&cmd));
    }
    say("starting RetroArch…");
    let began = std::time::Instant::now();
    let started_at = crate::util::now_iso();
    // The app refuses to quit from here until the saves have gone up: see `busy`.
    let status = {
        let _playing = busy.playing(&row.name);
        plan.run(ra, false)
    };
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            close_record(state, record, format!("did not start: {e}"), 0, None).await;
            return Err(err(e));
        }
    };

    // How long that took is the only record of it. RetroArch tells nobody, and
    // the server's `last_played` only moves when something tells the server —
    // which nothing here did, so playing a game on this machine used to leave
    // no trace on this machine at all.
    let seconds = began.elapsed().as_secs() as i64;
    if let Ok(cache) = state.cache.lock()
        && let Ok(true) = cache.record_play(row.id, &started_at, seconds)
    {
        notes.push(format!("played for {}", crate::util::spell_duration(seconds)));
    }

    let post = if skip_sync.unwrap_or(false) {
        AutoSync::default()
    } else {
        let _uploading = busy.uploading();
        auto_sync(state, &row, savesync::When::AfterExit).await
    };
    let saves_note = post.note.clone().or_else(|| post.failed.clone().map(|w| format!("NOT uploaded: {w}")));
    if let Some(note) = post.note {
        notes.push(note);
    }
    // After the fact there is nothing to ask: the game has already been played.
    // Say so in the notes so progress that has not left the machine is visible.
    if let Some(why) = post.failed {
        notes.push(format!("saves: NOT uploaded — {why}"));
    }
    close_record(state, record, status.to_string(), seconds, saves_note).await;
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


/// `launch` for a standalone emulator: the same steps as a RetroArch game
/// (BIOS, saves before, play, saves after), with the emulator's own setup.
///
/// An emulator that isn't installed yet comes back as `EMULATOR_MISSING:`, so
/// the page can offer to install it and try again.
async fn launch_standalone(
    state: &AppState,
    busy: &Busy,
    emu: &'static crate::standalone::Emulator,
    row: &cache::RomRow,
    rom: &Path,
    skip_sync: bool,
    say: &(impl Fn(&str) + Send + Sync),
) -> CmdResult<String> {
    use crate::standalone;
    let dir = standalone::dir(&state.library_root, emu);
    let Some(program) = standalone::program(&dir) else {
        return Err(format!("EMULATOR_MISSING:{}:{}:{}", emu.id, emu.name, emu.version));
    };

    let mut notes = Vec::new();
    let bios_dir = state.system_dir().join("PS2");
    if let Some(api) = state.client.as_ref() {
        say("checking BIOS files…");
        match standalone::fetch_bios(api, &bios_dir).await {
            Ok(0) => {}
            Ok(n) => notes.push(format!("fetched {n} BIOS file(s)")),
            Err(e) => notes.push(format!("could not fetch BIOS: {e}")),
        }
    }
    // PCSX2 starts nothing without one, so this is a refusal rather than the
    // "play anyway" question a RetroArch core gets.
    if standalone::pick_bios(&bios_dir).is_none() {
        return Err(format!(
            "No PS2 BIOS. Put one in the server's bios/PS2 folder, or in {}",
            bios_dir.display()
        ));
    }
    let family = emu.family.unwrap_or_default();
    standalone::configure(&dir, &bios_dir).map_err(err)?;

    if skip_sync {
        notes.push("saves: not synced — you chose to play anyway".to_owned());
    } else {
        say("checking saves with the server…");
        let pre = auto_sync_units(state, family, savesync::When::BeforeLaunch).await;
        if let Some(why) = pre.failed {
            return Err(format!("SAVE_OFFLINE:{why}"));
        }
        if !pre.conflicts.is_empty() {
            *state.pending_conflicts.lock().map_err(err)? = pre.conflicts.clone();
            return Err(format!(
                "SAVE_CONFLICT:{}",
                serde_json::to_string(&pre.conflicts).unwrap_or_default()
            ));
        }
        notes.extend(pre.note);
    }

    let record = open_record(
        state,
        row,
        skip_sync,
        standalone::config_lines(&bios_dir),
        crate::launchrecord::Data {
            game_md5: row.md5_hash.clone(),
            emulator: format!("{} {}", emu.name, emu.version),
            emulator_md5: crate::launchrecord::md5_of(&program),
            bios: bios_of(&bios_dir, standalone::pick_bios(&bios_dir).into_iter().collect()),
        },
    );
    say(&format!("starting {}…", emu.name));
    let began = std::time::Instant::now();
    let started_at = crate::util::now_iso();
    let status = {
        let _playing = busy.playing(&row.name);
        standalone::command(&program, rom).status()
    }
    .map_err(|e| {
        // PCSX2's macOS build is Intel-only, so Apple silicon runs it through
        // Rosetta, which a fresh Mac may not have.
        if e.raw_os_error() == Some(86) {
            format!("{} needs Rosetta on this Mac. Install it with: softwareupdate --install-rosetta", emu.name)
        } else {
            format!("starting {}: {e}", emu.name)
        }
    })?;
    eprintln!("launch: {} ended: {status}", emu.name);
    let seconds = began.elapsed().as_secs() as i64;
    if let Ok(cache) = state.cache.lock()
        && let Ok(true) = cache.record_play(row.id, &started_at, seconds)
    {
        notes.push(format!("played for {}", crate::util::spell_duration(seconds)));
    }

    let mut saves_note = None;
    if !skip_sync {
        eprintln!("launch: uploading saves");
        let post = {
            let _uploading = busy.uploading();
            auto_sync_units(state, family, savesync::When::AfterExit).await
        };
        saves_note = post.note.clone().or_else(|| post.failed.clone().map(|w| format!("NOT uploaded: {w}")));
        notes.extend(post.note);
        if let Some(why) = post.failed {
            notes.push(format!("saves: NOT uploaded — {why}"));
        }
        if !post.conflicts.is_empty() {
            *state.pending_conflicts.lock().map_err(err)? = post.conflicts;
        }
    }

    close_record(state, record, status.to_string(), seconds, saves_note).await;
    let prefix = if notes.is_empty() { String::new() } else { format!("{}; ", notes.join("; ")) };
    eprintln!("launch: done");
    Ok(if status.success() {
        format!("{prefix}{} exited cleanly", row.name)
    } else {
        format!("{prefix}{} exited with {status}", row.name)
    })
}

/// One line of the launch checklist.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Item {
    /// What it is: the game's file name, or the emulator and its version.
    pub label: String,
    /// The state in words: "in the cache, hash checked", "not on this computer yet".
    pub detail: String,
    pub ready: bool,
    /// Its size, whether or not it has to be fetched. `None` when nobody can say.
    pub bytes: Option<u64>,
}

/// What stands between this game and Play. See `docs/launch-flow.html`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Checklist {
    /// The game's name and console, for the window's heading.
    pub name: String,
    pub platform: String,
    pub game: Item,
    pub emulator: Item,
    /// Both lines ready: the button is Play.
    pub ready: bool,
    /// What the Download button fetches: the sizes of the lines that aren't
    /// ready, added up. A line with no known size adds nothing.
    pub download_bytes: u64,
    /// Something is missing and Download can't fetch it (no server, no
    /// RetroArch). The window shows this and offers only Cancel.
    pub blocked: Option<String>,
}

fn row_for(state: &AppState, id: i64) -> CmdResult<cache::RomRow> {
    let cache = state.cache.lock().map_err(err)?;
    cache.rom_by_id(id).map_err(err)?.ok_or_else(|| format!("no rom with id {id}"))
}

/// The game line: present, and its hash checked against the library's.
///
/// A file is hashed once and then remembered by size and date (`verified`),
/// so only the first check of a big disc takes a moment.
fn game_item(state: &AppState, row: &cache::RomRow) -> Item {
    let bytes = (row.fs_size_bytes > 0).then_some(row.fs_size_bytes as u64);
    let item = |ready: bool, detail: &str| Item { label: row.fs_name.clone(), detail: detail.to_owned(), ready, bytes };
    let Some(path) = row_path(state, row) else {
        return item(false, "not on this computer yet");
    };
    // Folder ROMs (multi-disc) are checked member by member when they
    // download; there is no single hash to check here.
    if path.is_dir() {
        return item(true, "in the cache");
    }
    if crate::verified::is_current(&state.library_root, &path) {
        return item(true, "in the cache, hash checked");
    }
    let no_hash = row.md5_hash.as_deref().unwrap_or("").is_empty() && row.sha1_hash.as_deref().unwrap_or("").is_empty();
    if no_hash {
        // A local library file the server has no hash for: nothing to check
        // it against, and nothing better to download instead.
        return item(true, "in the cache");
    }
    let folder = row.folder();
    let target = crate::download::Target {
        rom_id: row.id,
        members: &[],
        fs_name: &row.fs_name,
        folder: &folder,
        expected_size: bytes,
        md5: row.md5_hash.as_deref(),
        sha1: row.sha1_hash.as_deref(),
        multi_file: false,
    };
    match crate::download::verify(&path, &target) {
        Ok(_) => {
            crate::verified::remember(&state.library_root, &path);
            item(true, "in the cache, hash checked")
        }
        Err(_) => item(false, "in the cache, but it doesn't match the library; downloads again"),
    }
}

/// BIOS files the server has that `missing` names, with their total size.
async fn server_bios_bytes(state: &AppState, wanted: impl Fn(&crate::api::Firmware) -> bool) -> Option<u64> {
    let client = state.client.as_ref()?;
    let list = client.firmware().await.ok()?;
    Some(list.iter().filter(|f| wanted(f)).map(|f| f.file_size_bytes.max(0) as u64).sum())
}

/// The emulator line, and what blocks Download when it can't fix this line.
async fn emulator_item(state: &AppState, row: &cache::RomRow) -> (Item, Option<String>) {
    if let Some(emu) = crate::standalone::for_platform(&row.platform_slug) {
        use crate::standalone;
        let dir = standalone::dir(&state.library_root, emu);
        let bios_dir = state.system_dir().join("PS2");
        let label = format!("{} {}", emu.name, emu.version);
        let installed = standalone::program(&dir).is_some()
            && standalone::installed_version(&dir).as_deref() == Some(emu.version);
        let bios = standalone::pick_bios(&bios_dir).is_some();
        let settings = installed && standalone::configured(&dir, &bios_dir);
        let mut bytes = if installed { 0 } else { standalone::download_size(emu) };
        if !bios {
            let have = |f: &crate::api::Firmware| {
                std::path::Path::new(&f.file_name).file_name().is_some_and(|n| bios_dir.join(n).is_file())
            };
            bytes += server_bios_bytes(state, |f| standalone::is_ps2_bios(f) && !have(f)).await.unwrap_or(0);
        }
        let detail = match (installed, bios, settings) {
            (true, true, true) => "with BIOS and its settings".to_owned(),
            (false, _, _) => "not on this computer yet; comes with BIOS and settings".to_owned(),
            (true, false, _) => "installed; BIOS not here yet".to_owned(),
            (true, true, false) => "installed; settings to apply".to_owned(),
        };
        let ready = installed && bios && settings;
        let blocked = (!ready && (!installed || !bios) && state.client.is_none())
            .then(|| "The server isn't reachable, so the emulator can't be fetched.".to_owned());
        let item = Item { label, detail, ready, bytes: Some(if ready { standalone::download_size(emu) } else { bytes }) };
        return (item, blocked);
    }

    let (overrides, per_game) = match (state.core_overrides.lock(), state.core_per_game.lock()) {
        (Ok(o), Ok(p)) => (o.clone(), p.clone()),
        _ => Default::default(),
    };
    let Some(core) = coremap::resolve_core_for(
        &state.map,
        &overrides,
        &per_game,
        &row.platform_slug,
        Some(&row.fs_name),
        |_| true,
    ) else {
        let item = Item { label: "RetroArch".to_owned(), detail: format!("no core for {}", row.platform_slug), ready: false, bytes: None };
        return (item, Some(format!("RetroArch has no core for {}.", row.platform_slug)));
    };
    // Not found: Download installs the app's own copy (`install_retroarch`).
    let ra = retroarch(state);
    let http = crate::util::http_client(Some(std::time::Duration::from_secs(10))).ok();
    let mut bytes = 0;
    let mut unknown = false;
    let version = match (&ra, &http) {
        (None, Some(h)) => crate::retroarch_install::latest_available(h).await.ok(),
        _ => None,
    };
    if ra.is_none() {
        match (&http, &version) {
            (Some(h), Some(v)) => match crate::retroarch_install::download_size(h, v).await {
                Some(n) => bytes += n,
                None => unknown = true,
            },
            _ => unknown = true,
        }
    }
    let label = match &version {
        Some(v) => format!("RetroArch {v}, {core}"),
        None => format!("RetroArch, {core}"),
    };
    let has_core = ra.as_ref().is_some_and(|ra| ra.has_core(&core));
    let bios_dir = state.system_dir();
    let missing_bios: Vec<String> = crate::bios::required_for(&core, &row.platform_slug)
        .into_iter()
        .filter(|n| !bios_dir.join(n).is_file())
        .collect();
    if !has_core {
        match &http {
            Some(h) => match crate::cores::download_size(h, &core).await {
                Some(n) => bytes += n,
                None => unknown = true,
            },
            None => unknown = true,
        }
    }
    if !missing_bios.is_empty() {
        let wanted = |f: &crate::api::Firmware| {
            std::path::Path::new(&f.file_name)
                .file_name()
                .is_some_and(|n| missing_bios.iter().any(|m| n.to_string_lossy() == m.as_str()))
        };
        bytes += server_bios_bytes(state, wanted).await.unwrap_or(0);
    }
    let ready = has_core && missing_bios.is_empty();
    let detail = match (has_core, missing_bios.is_empty()) {
        _ if ra.is_none() && missing_bios.is_empty() => "not on this computer yet; comes with its core".to_owned(),
        _ if ra.is_none() => format!("not on this computer yet; comes with its core and BIOS: {}", missing_bios.join(", ")),
        (true, true) => "core and BIOS here".to_owned(),
        (false, true) => "core not on this computer yet".to_owned(),
        (true, false) => format!("BIOS not here yet: {}", missing_bios.join(", ")),
        (false, false) => format!("core and BIOS not here yet: {}", missing_bios.join(", ")),
    };
    // RetroArch and its cores come from libretro's site; only BIOS needs the server.
    let blocked = (!missing_bios.is_empty() && state.client.is_none())
        .then(|| "The server isn't reachable, so the BIOS can't be fetched.".to_owned());
    let item = Item { label, detail, ready, bytes: if ready || unknown { None } else { Some(bytes) } };
    (item, blocked)
}

/// Check the game and the emulator for this launch.
pub async fn checklist(state: &AppState, id: i64) -> CmdResult<Checklist> {
    let row = row_for(state, id)?;
    let game = game_item(state, &row);
    let (emulator, emu_blocked) = emulator_item(state, &row).await;
    let missing = |i: &Item| if i.ready { 0 } else { i.bytes.unwrap_or(0) };
    let download_bytes = missing(&game) + missing(&emulator);
    let blocked = if !game.ready && state.client.is_none() {
        Some("The server isn't reachable, so the game can't be downloaded.".to_owned())
    } else {
        emu_blocked
    };
    Ok(Checklist {
        name: row.name.clone(),
        platform: row.platform_slug.clone(),
        ready: game.ready && emulator.ready,
        game,
        emulator,
        download_bytes,
        blocked,
    })
}

/// The Download button: fetch everything the checklist found missing, check
/// every hash, and apply the emulator's settings. The window then checks
/// again.
pub async fn prepare(state: &AppState, id: i64, say: impl Fn(&str) + Send + Sync) -> CmdResult<String> {
    let row = row_for(state, id)?;
    let mut done = Vec::new();

    if !game_item(state, &row).ready {
        let name = row.fs_name.clone();
        let mut last = 0u64;
        let said = download_game(state, id, |got, total| {
            let pct = (got * 100).checked_div(total).unwrap_or(0);
            if pct != last {
                last = pct;
                say(&format!("downloading {name}… {pct}%"));
            }
        })
        .await?;
        done.push(said);
    }

    if let Some(emu) = crate::standalone::for_platform(&row.platform_slug) {
        use crate::standalone;
        let dir = standalone::dir(&state.library_root, emu);
        if standalone::program(&dir).is_none() || standalone::installed_version(&dir).as_deref() != Some(emu.version) {
            let http = crate::util::http_client(None).map_err(err)?;
            let mut last = 0u64;
            standalone::install(&http, emu, &dir, |got, total| {
                let pct = (got * 100).checked_div(total).unwrap_or(0);
                if pct != last {
                    last = pct;
                    say(&format!("downloading {} {}… {pct}%", emu.name, emu.version));
                }
            })
            .await
            .map_err(err)?;
            done.push(format!("installed {} {}", emu.name, emu.version));
        }
        let bios_dir = state.system_dir().join("PS2");
        if let Some(api) = state.client.as_ref() {
            say("fetching BIOS files…");
            let n = standalone::fetch_bios(api, &bios_dir).await.map_err(err)?;
            if n > 0 {
                done.push(format!("fetched {n} BIOS file(s)"));
            }
        }
        say(&format!("setting up {}…", emu.name));
        standalone::configure(&dir, &bios_dir).map_err(err)?;
        return Ok(done.join("; "));
    }

    let ra = match retroarch(state) {
        Some(ra) => ra,
        None => {
            let (ra, version) = install_retroarch(state, &say).await?;
            done.push(format!("installed RetroArch {version}"));
            ra
        }
    };
    let ra = &ra;
    let http = crate::util::http_client(None).map_err(err)?;
    let api = state.client.as_ref().ok_or("not connected to a server")?;
    let (overrides, per_game) = {
        let o = state.core_overrides.lock().map_err(err)?.clone();
        let p = state.core_per_game.lock().map_err(err)?.clone();
        (o, p)
    };
    if let Some(core) = coremap::resolve_core_for(&state.map, &overrides, &per_game, &row.platform_slug, Some(&row.fs_name), |_| true) {
        say("fetching the emulator core…");
        if crate::cores::ensure(&http, ra, &core).await.map_err(err)? {
            done.push(format!("downloaded the {core} core"));
        }
        say("fetching BIOS files…");
        let n = crate::bios::ensure(api, &state.system_dir(), &state.legacy_bios, &core, &row.platform_slug)
            .await
            .map_err(err)?;
        if n > 0 {
            done.push(format!("fetched {n} BIOS file(s)"));
        }
    }
    Ok(done.join("; "))
}

/// Open a launch record. Written before the emulator starts, so a launch
/// that never comes back still left a trace (`launchrecord`).
fn open_record(
    state: &AppState,
    row: &cache::RomRow,
    offline: bool,
    config: Vec<String>,
    data: crate::launchrecord::Data,
) -> crate::launchrecord::Record {
    use crate::launchrecord::*;
    let started_at = crate::util::now_iso();
    let identity = savesync::DeviceIdentity::current(Path::new("."));
    let rec = Record {
        id: new_id(&started_at),
        device_id: identity.map(|i| i.device_id),
        device_name: savesync::hostname(),
        started_at,
        ended_at: None,
        game: Game { platform: row.platform_slug.clone(), file: row.fs_name.clone() },
        code: code(),
        config: ConfigVersion { hash: config_hash(&config) },
        data,
        machine: machine(),
        offline: offline || state.client.is_none(),
        outcome: None,
    };
    if let Err(e) = save(&state.library_root, &rec) {
        eprintln!("launch: could not write the launch record: {e:#}");
    }
    rec
}

/// Close the record with how the game ended, then upload every finished
/// record waiting on this computer, this one included.
async fn close_record(
    state: &AppState,
    mut rec: crate::launchrecord::Record,
    exit: String,
    seconds: i64,
    saves: Option<String>,
) {
    use crate::launchrecord::*;
    rec.ended_at = Some(crate::util::now_iso());
    rec.outcome = Some(Outcome { exit, seconds, saves });
    if let Err(e) = save(&state.library_root, &rec) {
        eprintln!("launch: could not finish the launch record: {e:#}");
        return;
    }
    if let Some(client) = state.client.as_ref() {
        let n = upload_pending(client, &state.library_root).await;
        eprintln!("launch: record {} written, {n} uploaded", rec.id);
    }
}

/// Each named BIOS in `dir` with its md5, for a launch record.
fn bios_of(dir: &Path, names: Vec<String>) -> Vec<crate::launchrecord::Bios> {
    names
        .into_iter()
        .map(|name| crate::launchrecord::Bios { md5: crate::launchrecord::md5_of(&dir.join(&name)), name })
        .collect()
}

/// The settings a RetroArch launch was given, as lines for the record's config
/// hash: the core, the shader preset by name, and the overrides file the plan
/// wrote, with this computer's paths and window geometry left out.
fn retroarch_config_lines(plan: &crate::launch::Plan, state: &AppState) -> Vec<String> {
    let mut lines = vec![format!("core = {}", plan.core)];
    if let Some(name) = plan.shader.as_deref().and_then(|s| s.file_name()) {
        lines.push(format!("shader = {}", name.to_string_lossy()));
    }
    let root = std::path::absolute(&state.library_root).unwrap_or_else(|_| state.library_root.clone());
    let root = root.to_string_lossy().into_owned();
    const PER_DEVICE: &[&str] = &["window_pos", "_width", "_height", "refresh_rate", "monitor_index", "directory", "_path"];
    if let Some(text) = plan.overrides.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) {
        for l in text.lines().map(str::trim) {
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let key = l.split('=').next().unwrap_or("").trim();
            if PER_DEVICE.iter().any(|d| key.contains(d)) || l.contains(&root) {
                continue;
            }
            lines.push(l.to_owned());
        }
    }
    lines
}

/// RetroArch: the copy found at startup, or the app's own one if the
/// checklist's Download has installed it since.
fn retroarch(state: &AppState) -> Option<crate::retroarch::RetroArch> {
    state.retroarch.clone().or_else(|| {
        let own = crate::standalone::retroarch_dir(&state.library_root);
        crate::retroarch::RetroArch::locate_in(&[own.to_string_lossy().into_owned()])
            .ok()
            .map(|ra| ra.with_system_dir(Some(state.system_dir())))
    })
}

/// Install RetroArch into `<library>/emulators/retroarch`, portable, so its
/// config, cores and shaders stay in that folder. Returns it and its version.
async fn install_retroarch(
    state: &AppState,
    say: &(impl Fn(&str) + Send + Sync),
) -> CmdResult<(crate::retroarch::RetroArch, String)> {
    let http = crate::util::http_client(None).map_err(err)?;
    let version = crate::retroarch_install::latest_available(&http).await.map_err(err)?;
    let dir = crate::standalone::retroarch_dir(&state.library_root);
    let mut last = 101;
    crate::retroarch_install::install(&http, &version, &dir, |got, total| {
        let pct = (got * 100).checked_div(total).unwrap_or(0);
        if pct != last {
            last = pct;
            say(&format!("downloading RetroArch {version}… {pct}%"));
        }
    })
    .await
    .map_err(err)?;
    let ra = retroarch(state).ok_or_else(|| format!("RetroArch {version} installed, but it isn't in {}", dir.display()))?;
    Ok((ra, version))
}

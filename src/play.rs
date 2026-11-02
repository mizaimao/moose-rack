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

    let ra = state.retroarch.as_ref().ok_or("RetroArch not found")?;
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

    say("starting RetroArch…");
    let began = std::time::Instant::now();
    let started_at = crate::util::now_iso();
    // The app refuses to quit from here until the saves have gone up: see `busy`.
    let status = {
        let _playing = busy.playing(&row.name);
        plan.run(ra, false).map_err(err)?
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
    let seconds = began.elapsed().as_secs() as i64;
    if let Ok(cache) = state.cache.lock()
        && let Ok(true) = cache.record_play(row.id, &started_at, seconds)
    {
        notes.push(format!("played for {}", crate::util::spell_duration(seconds)));
    }

    if !skip_sync {
        let post = {
            let _uploading = busy.uploading();
            auto_sync_units(state, family, savesync::When::AfterExit).await
        };
        notes.extend(post.note);
        if let Some(why) = post.failed {
            notes.push(format!("saves: NOT uploaded — {why}"));
        }
        if !post.conflicts.is_empty() {
            *state.pending_conflicts.lock().map_err(err)? = post.conflicts;
        }
    }

    let prefix = if notes.is_empty() { String::new() } else { format!("{}; ", notes.join("; ")) };
    Ok(if status.success() {
        format!("{prefix}{} exited cleanly", row.name)
    } else {
        format!("{prefix}{} exited with {status}", row.name)
    })
}

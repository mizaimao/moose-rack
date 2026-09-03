//! Deterministic "does this core actually run this game?" test.
//!
//! The previous attempt at this launched a game, waited a few seconds, killed
//! it, and called it a pass if nothing had crashed. That gave contradictory
//! results on identical runs — the same game passed three times and failed a
//! fourth — and a default was once changed on the strength of it. None of that
//! evidence was worth anything.
//!
//! This is the honest version, built on three RetroArch features:
//!
//! * `--max-frames=N` runs exactly N frames and exits. No timers, no killing.
//! * `--log-file` + `--verbose` record what the core actually did.
//! * null audio and video drivers keep it quiet and stop it rendering.
//!
//! It is **not** headless, though. `video_driver = "null"` silences rendering,
//! but on macOS RetroArch's Cocoa frontend creates its window before the video
//! driver is chosen, so a window still appears for every probe. Probing N games
//! against M cores opens N*M windows, so this is gated behind an explicit flag
//! and never run unasked.
//!
//! **The exit code is 0 whether or not the content loaded**, which is the trap
//! the old method fell into. The verdict comes from the log alone.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::retroarch::RetroArch;

/// What a single (game, core) probe concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The core loaded the content and ran frames, with no complaint about the
    /// romset.
    Ran,
    /// The core started the game but said files were missing from the romset.
    ///
    /// Its own category because it is neither a pass nor a refusal, and calling
    /// it either is wrong in a way that matters: the emulator is up, something
    /// is on screen, and the game is running with pieces absent — wrong
    /// graphics, no sound, a crash three levels in. Counting these as successes
    /// is how a set gets called healthy while being quietly broken.
    RanWithMissingFiles,
    /// A BIOS or device romset, which is not a game and cannot be played.
    IsBios,
    /// The core refused the content — wrong system, missing romset files,
    /// version mismatch.
    RefusedContent,
    /// The core itself would not load (missing, wrong architecture).
    CoreFailed,
    /// RetroArch never reached a verdict; treat as unknown, not as a pass.
    Inconclusive,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Ran => "ran",
            Verdict::RanWithMissingFiles => "ran, romset incomplete",
            Verdict::IsBios => "not a game (BIOS)",
            Verdict::RefusedContent => "refused content",
            Verdict::CoreFailed => "core failed to load",
            Verdict::Inconclusive => "inconclusive",
        }
    }

    pub fn ok(self) -> bool {
        self == Verdict::Ran
    }
}

#[derive(Debug, Clone)]
pub struct Probe {
    pub core: String,
    pub verdict: Verdict,
    /// The line the verdict was read from, for when a result looks wrong.
    pub evidence: String,
    pub seconds: f64,
}

/// Config silencing audio and rendering.
///
/// Does NOT prevent the window on macOS — see the module docs. Written per run
/// rather than kept around: it must never be mistaken for the user's own
/// RetroArch config.
const HEADLESS: &str = "\
# Written by `moose-rack probe`. Nothing here should reach a real session.
#
# config_save_on_exit MUST stay first and MUST stay false. RetroArch defaults it
# to true, and an --appendconfig value is written back into the user's
# retroarch.cfg on exit — an earlier version of this file omitted it and
# permanently set the user's video, audio and menu drivers to \"null\", which
# silently broke every game until someone noticed.
config_save_on_exit = \"false\"
video_driver = \"null\"
audio_driver = \"null\"
menu_driver = \"null\"
# Input too, and for a reason that only shows on a machine without a desktop:
# RetroArch picks an input driver from the video driver, and with video nulled
# it finds none and exits with \"Cannot initialize input driver\" -- *after* the
# core has loaded the game and reported its geometry. On macOS the Cocoa
# frontend supplies one regardless, so this passed there and failed everywhere
# else, marking every MAME game a refusal when the frontend was what died.
input_driver = \"null\"
joypad_driver = \"null\"
video_fullscreen = \"false\"
# A probe must not inherit or write save state, or it would pollute real saves.
savestate_auto_load = \"false\"
savestate_auto_save = \"false\"
";

/// Run one game under one core and report what happened.
///
/// `frames` at 60 fps: 180 is three seconds, enough for a core to load a
/// romset and start emulating, short enough to probe hundreds of games.
pub fn probe_one(ra: &RetroArch, rom: &Path, core: &str, frames: u32, scratch: &Path) -> Result<Probe> {
    std::fs::create_dir_all(scratch)
        .with_context(|| format!("creating {}", scratch.display()))?;
    let cfg = scratch.join("headless.cfg");
    std::fs::write(&cfg, HEADLESS)?;
    let log = scratch.join("probe.log");
    let _ = std::fs::remove_file(&log);

    let started = std::time::Instant::now();
    let out = Command::new(&ra.binary)
        .arg(format!("--appendconfig={}", cfg.display()))
        .arg(format!("--max-frames={frames}"))
        .arg("--verbose")
        .arg(format!("--log-file={}", log.display()))
        .arg("-L")
        .arg(core)
        .arg(rom)
        .output()
        .with_context(|| format!("running RetroArch for {core}"))?;
    let seconds = started.elapsed().as_secs_f64();

    // Deliberately ignoring out.status: RetroArch exits 0 even when the core
    // refused the content. Only the log distinguishes the two.
    let text = std::fs::read_to_string(&log).unwrap_or_else(|_| {
        String::from_utf8_lossy(&out.stderr).into_owned()
    });
    let (verdict, evidence) = read_verdict(&text);
    Ok(Probe { core: core.to_owned(), verdict, evidence, seconds })
}

/// Read a verdict out of a RetroArch log.
///
/// Order matters: a failure line is conclusive even though the log also
/// contains the "Content ran for" line that a success would produce.
/// The name the core itself knows the game by, from its startup line.
///
/// FBNeo announces `Driver X was successfully started : game's full name is Y`.
/// That Y is the arcade database's own title, which is the honest thing to
/// check a library's labelling against — it comes from the emulator that will
/// run it, not from a scraper.
pub fn core_title(log: &str) -> Option<String> {
    log.lines()
        .find_map(|l| l.split("game's full name is ").nth(1))
        .map(|s| s.trim().trim_end_matches('.').to_owned())
}

/// Phrases each core uses when a romset is incomplete.
///
/// Taken from the core binaries rather than guessed. The FBNeo one is the case
/// that matters most: the game starts, so every signal short of reading this
/// line says success.
const MISSING_FILE_MARKERS: &[&str] = &[
    // FBNeo
    "romsets is missing files",
    "Missing files, aborting",
    // FBNeo again, and the reason this list is not just the two lines above:
    // when a romset is incomplete FBNeo often says nothing about "missing
    // files" at all, and instead names each absent chip one line at a time:
    //
    //   [FBNeo] ROM at index 0 with name d8-d15.1m and CRC 0x9121d1b6 is required
    //
    // The game still starts, still reports geometry, and still runs frames, so
    // without this marker the probe calls it a clean pass. `maniacsq` passed
    // that way and then showed the missing-files screen when launched for real.
    "[FBNeo] ROM at index",
    // MAME 2003+ and MAME
    "Required files are missing",
    "NOT FOUND",
    "WRONG CHECKSUM",
];

pub fn read_verdict(log: &str) -> (Verdict, String) {
    let mut ran_line = None;
    let mut missing = None;
    let mut started = None;

    for line in log.lines() {
        // A BIOS is not a game. FBNeo says so outright, and without this they
        // read as refusals and look like broken dumps.
        if line.contains("Bioses aren't meant to be launched this way") {
            return (Verdict::IsBios, line.trim().to_owned());
        }
        if MISSING_FILE_MARKERS.iter().any(|m| line.contains(m)) {
            missing = Some(line.trim().to_owned());
        }
        if line.contains("was successfully started") {
            started = Some(line.trim().to_owned());
        }
        if line.contains("[Content]: Failed to load content")
            || line.contains("Failed to load content")
        {
            return (Verdict::RefusedContent, line.trim().to_owned());
        }
        if line.contains("Failed to open libretro core")
            || line.contains("Failed to load libretro core")
            || line.contains("[Core]: Failed to load")
        {
            return (Verdict::CoreFailed, line.trim().to_owned());
        }
        // MAME cores never say "successfully started"; the signal that they
        // loaded the game is the geometry they report back to the frontend.
        if line.contains("[Core]: Geometry:") {
            started.get_or_insert_with(|| line.trim().to_owned());
        }
        if line.contains("Content ran for a total of") {
            ran_line = Some(line.trim().to_owned());
        }
    }

    // An incomplete romset outranks everything below it. The game may well have
    // started; that is exactly the case this verdict exists to name.
    if let Some(l) = missing {
        return (Verdict::RanWithMissingFiles, l);
    }

    // The core's own word beats the clock. RetroArch reports runtime in whole
    // seconds, so on a fast machine sixty frames round to "00 seconds" — and
    // reading that as a refusal marks every healthy game on a quick host as
    // broken. Which is exactly what happened the first time this ran on a
    // server rather than a laptop.
    if let Some(l) = started {
        return (Verdict::Ran, l);
    }

    match ran_line {
        // No such claim from the core, and nothing emulated: a refusal the core
        // did not log explicitly.
        Some(l) if l.contains("00 hours, 00 minutes, 00 seconds") => {
            (Verdict::RefusedContent, l)
        }
        Some(l) => (Verdict::Ran, l),
        None => (Verdict::Inconclusive, "no verdict line in the log".to_owned()),
    }
}

/// Try every candidate core against one game, in order.
pub fn probe_cores(
    ra: &RetroArch,
    rom: &Path,
    cores: &[String],
    frames: u32,
    scratch: &Path,
) -> Result<Vec<Probe>> {
    if !rom.is_file() && !rom.is_dir() {
        bail!("not downloaded: {}", rom.display());
    }
    let mut out = Vec::new();
    for core in cores {
        if !ra.has_core(core) {
            out.push(Probe {
                core: core.clone(),
                verdict: Verdict::CoreFailed,
                evidence: "core not installed".to_owned(),
                seconds: 0.0,
            });
            continue;
        }
        out.push(probe_one(ra, rom, core, frames, scratch)?);
    }
    Ok(out)
}

/// Scratch directory for probe artefacts, inside the visible library folder
/// rather than a hidden system temp dir.
pub fn scratch_dir(library_root: &str) -> PathBuf {
    PathBuf::from(library_root).join("probe")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case that has been miscounted repeatedly: the game *starts*, the
    /// runtime is non-zero, RetroArch is happy — and the romset is incomplete.
    /// Every signal short of reading the core's own line says success.
    #[test]
    fn a_game_that_starts_with_files_missing_is_not_a_pass() {
        let log = "[libretro INFO] [FBNeo] Driver kof98 was successfully started : game's full name is The King of Fighters '98\n\
             [libretro INFO] [FBNeo] This game is known but one of your romsets is missing files for THIS VERSION of FBNeo.\n\
             [INFO] [Core]: Content ran for a total of: 00 hours, 00 minutes, 03 seconds.";
        let (v, _) = read_verdict(log);
        assert_eq!(v, Verdict::RanWithMissingFiles);
        assert!(!v.ok(), "an incomplete romset must never count as running");
    }

    #[test]
    fn mames_wording_for_the_same_thing_is_caught_too() {
        for line in [
            "[libretro INFO] Required files are missing, the game cannot be run.",
            "[libretro INFO] [MAME 2003+] gfx1.rom     NOT FOUND",
            "[libretro INFO] WRONG CHECKSUM for rom foo.bin",
        ] {
            let log = format!("{line}\n[INFO] [Core]: Content ran for a total of: 00 hours, 00 minutes, 04 seconds.");
            assert_eq!(read_verdict(&log).0, Verdict::RanWithMissingFiles, "{line}");
        }
    }

    #[test]
    fn a_clean_run_is_still_a_pass() {
        let log = "[libretro INFO] [FBNeo] Driver kovsh was successfully started : game's full name is Knights of Valour Super Heroes\n\
             [libretro INFO] [FBNeo] No missing files, proceeding\n\
             [INFO] [Core]: Content ran for a total of: 00 hours, 00 minutes, 01 seconds.";
        let (v, _) = read_verdict(log);
        assert_eq!(v, Verdict::Ran);
        assert!(v.ok());
    }

    /// "No missing files, proceeding" contains the word missing. Matching it
    /// would turn every healthy romset into a failure.
    #[test]
    fn the_reassuring_message_is_not_read_as_a_complaint() {
        let log = "[libretro INFO] [FBNeo] No missing files, proceeding\n\
             [INFO] [Core]: Content ran for a total of: 00 hours, 00 minutes, 02 seconds.";
        assert_eq!(read_verdict(log).0, Verdict::Ran);
    }

    #[test]
    fn a_bios_is_reported_as_a_bios_rather_than_a_broken_game() {
        let log = "[libretro INFO] [FBNeo] Bioses aren't meant to be launched this way";
        let (v, _) = read_verdict(log);
        assert_eq!(v, Verdict::IsBios);
        assert!(!v.ok());
    }

    /// Straight from a headless server run. Sixty frames took under a second,
    /// so RetroArch rounded the runtime to zero — and the old rule called a
    /// perfectly good launch a refusal. On a slower laptop this never showed.
    #[test]
    fn a_core_that_says_it_started_is_believed_over_a_rounded_clock() {
        let log = "[libretro INFO] [FBNeo] No missing files, proceeding\n\
             [libretro INFO] [FBNeo] Driver mslug was successfully started : game's full name is Metal Slug - Super Vehicle-001\n\
             [INFO] [Core]: Content ran for a total of: 00 hours, 00 minutes, 00 seconds.";
        assert_eq!(read_verdict(log).0, Verdict::Ran);
    }

    /// But a missing romset still outranks the core's claim to have started —
    /// that is the whole point of the stricter rule.
    #[test]
    fn starting_does_not_excuse_an_incomplete_romset() {
        let log = "[libretro INFO] [FBNeo] Driver x was successfully started : game's full name is X\n\
             [libretro INFO] [FBNeo] This game is known but one of your romsets is missing files for THIS VERSION of FBNeo.";
        assert_eq!(read_verdict(log).0, Verdict::RanWithMissingFiles);
    }

    #[test]
    fn zero_runtime_is_still_a_refusal() {
        let log = "[INFO] [Core]: Content ran for a total of: 00 hours, 00 minutes, 00 seconds.";
        assert_eq!(read_verdict(log).0, Verdict::RefusedContent);
    }

    /// The core knows what the game is really called, which is the honest thing
    /// to check a library's labelling against.
    #[test]
    fn the_cores_own_title_is_recoverable_from_the_log() {
        let log = "[libretro INFO] [FBNeo] Driver mslug was successfully started : game's full name is Metal Slug - Super Vehicle-001";
        assert_eq!(core_title(log).as_deref(), Some("Metal Slug - Super Vehicle-001"));
        assert_eq!(core_title("nothing here"), None);
    }
}

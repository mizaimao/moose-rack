//! `moose-fastlaunch` — start a game without waiting for Python.
//!
//! Measured on the Flip, 2026-08-28: a warm GBA launch is 4.26 s, of which
//! RetroArch is 0.83 s and `configgen` is 3.43 s. That 3.43 s is 272 Python
//! files imported from cold on every launch (1.08 s), a daemon restarted and
//! waited on for no reason (0.93 s), and about 1.4 s of config work.
//!
//! This program does the config work natively for the twelve libretro systems
//! that are actually played here, and hands everything else back.
//!
//! # The safety story
//!
//! There is exactly one: **when in doubt, exec the Python launcher.** An
//! unknown system, a non-libretro emulator, a core that is not on disk, a
//! `knulli.conf` that will not parse — all of them end in
//! [`fall_back`], which costs the 3.4 s we were trying to save and is
//! otherwise indistinguishable from not having installed this at all.
//!
//! # What is not done yet
//!
//! Generating `retroarchcustom.cfg`. That file is 114 KB and about 3,200
//! lines, and reproducing it exactly is the bulk of the remaining work. It is
//! deliberately not guessed at here: `--plan` prints what this program has
//! resolved so it can be diffed against what configgen decided, and the
//! generation lands only once that diff is clean across every system. Until
//! then this binary resolves and falls back, which is correct but not yet
//! fast. See `docs/fast-launch.md`.

mod args;
mod conf;
mod resolve;

use std::os::unix::process::CommandExt as _;
use std::process::Command;

/// Where we are installed. EmulationStation calls this path, from the
/// `<command>` in `es_systems.cfg`, so it is the one we must occupy.
const INSTALLED_AS: &str = "/usr/bin/emulatorlauncher";

/// KNULLI's own launcher, moved aside by the installer.
///
/// **Not** `/usr/bin/emulatorlauncher`. That is where *we* live, and execing
/// it would be execing ourselves — an unbounded fork on the only path that
/// starts games, on a device you would then need working in order to fix it.
/// The installer renames the Python shim here first; if it is missing we fail
/// loudly rather than guessing, because a loud failure is one reboot away from
/// stock and a quiet one is not.
const STOCK_LAUNCHER: &str = "/usr/bin/emulatorlauncher.stock";

/// `knulli.conf`, the single source of user settings.
const CONF_PATH: &str = "/userdata/system/knulli.conf";

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = args::Args::parse(argv.clone());

    // `--plan` is the differential-testing hook: resolve, print, do not
    // launch. Kept out of the argv handed to the fallback.
    let dry_run = argv.iter().any(|a| a == "--plan");

    let plan = build_plan(&parsed);

    if dry_run {
        match plan {
            Some(p) => println!(
                "system={}\ncore={}\ncore_path={}\nrom={}",
                p.system,
                p.core,
                p.core_path,
                parsed.rom.as_deref().unwrap_or("")
            ),
            None => println!("fallback"),
        }
        return std::process::ExitCode::SUCCESS;
    }

    // Until config generation lands, a successful plan still falls back — it
    // has proved it *could* handle the launch, not that it can produce the
    // config RetroArch needs. Better slow than subtly wrong.
    let _ = plan;
    fall_back(&parsed.argv)
}

/// Resolve the launch, returning `None` for anything we will not handle.
fn build_plan(parsed: &args::Args) -> Option<resolve::Plan> {
    let system = parsed.system.as_deref()?;
    let game = parsed.rom_file_name()?;

    // A `knulli.conf` we cannot read is not an error worth reporting: it is a
    // reason to let the Python launcher, which has its own opinions about
    // defaults, deal with it.
    let text = std::fs::read_to_string(CONF_PATH).ok()?;
    let conf = conf::Conf::parse(&text, system, game);

    resolve::plan(&conf, &|path| std::path::Path::new(path).exists())
}

/// Hand the launch to KNULLI's Python launcher, unchanged.
///
/// `exec` rather than spawn-and-wait: EmulationStation is watching this
/// process, and an extra process in the middle would mean its exit code and
/// its signals had to be forwarded correctly. Replacing ourselves means there
/// is nothing to forward.
fn fall_back(argv: &[String]) -> std::process::ExitCode {
    if !std::path::Path::new(STOCK_LAUNCHER).exists() {
        eprintln!(
            "moose-fastlaunch: {STOCK_LAUNCHER} is missing — refusing to exec \
             {INSTALLED_AS}, which is this program. Reboot to get stock back."
        );
        return std::process::ExitCode::FAILURE;
    }

    let err = Command::new(STOCK_LAUNCHER)
        .args(argv.iter().filter(|a| *a != "--plan"))
        .exec();

    // exec only returns on failure.
    eprintln!("moose-fastlaunch: could not exec {STOCK_LAUNCHER}: {err}");
    std::process::ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stock_launcher_is_an_absolute_path() {
        // It is exec'd, so a relative path would resolve against whatever
        // directory ES happened to leave us in.
        assert!(STOCK_LAUNCHER.starts_with('/'));
        assert!(CONF_PATH.starts_with('/'));
    }

    #[test]
    fn we_never_exec_ourselves() {
        // The collision that matters is not the binary's name, it is the
        // path it occupies: installed as /usr/bin/emulatorlauncher, a
        // fallback pointing at /usr/bin/emulatorlauncher is an unbounded
        // fork on the only path that starts games. The first version of
        // this test compared against the wrong thing and would have shipped
        // exactly that.
        assert_ne!(
            STOCK_LAUNCHER, INSTALLED_AS,
            "the fallback must not be the path we are installed at"
        );
        assert!(
            !STOCK_LAUNCHER.ends_with("moose-fastlaunch"),
            "nor this program by name"
        );
    }

    #[test]
    fn plan_flag_is_stripped_from_the_fallback_argv() {
        let argv = ["-system".to_string(), "gba".to_string(), "--plan".to_string()];
        let passed: Vec<&String> = argv.iter().filter(|a| *a != "--plan").collect();
        assert_eq!(passed.len(), 2, "--plan is ours, not the Python launcher's");
    }

    #[test]
    fn build_plan_refuses_without_a_system_or_rom() {
        let a = args::Args::parse(vec!["-rom", "/roms/gba/x.gba"]);
        assert!(build_plan(&a).is_none(), "no -system");
        let a = args::Args::parse(vec!["-system", "gba"]);
        assert!(build_plan(&a).is_none(), "no -rom");
    }
}

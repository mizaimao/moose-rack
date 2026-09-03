//! Which emulator and core, and whether we are allowed to take the fast path.
//!
//! The bias throughout is towards refusing. Every `None` returned here ends
//! in `exec emulatorlauncher`, which costs the 3.4 s we were trying to save
//! and is otherwise exactly what would have happened anyway. A wrong `Some`
//! costs a game launched with the wrong core, which is the failure mode this
//! whole design exists to avoid.

use crate::conf::Conf;

/// What the fast path needs to know to build a RetroArch command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub system: String,
    pub core: String,
    /// Absolute path to the libretro core.
    pub core_path: String,
}

/// The systems this launcher will handle, and the core KNULLI defaults to.
///
/// Only libretro systems that are actually played on this device are listed.
/// The table is a floor, not the answer: an explicit `<system>.core=` in
/// `knulli.conf` always wins over it. Its job is to let a system launch when
/// the user has never pinned a core, without guessing for systems nobody
/// here plays.
///
/// Read off the device on 2026-08-28. If KNULLI changes a default, the worst
/// case is that this disagrees and a game launches on the wrong core — which
/// is why `docs/fast-launch.md` requires the differential test to be re-run
/// after a KNULLI update, and why the table is small enough to check by eye.
const DEFAULT_CORES: &[(&str, &str)] = &[
    ("dreamcast", "flycastvl"),
    ("fbneo", "fbneo"),
    ("gb", "gambatte"),
    ("gba", "vba-m"),
    ("gbc", "gambatte"),
    ("megadrive", "genesisplusgx"),
    ("n64", "mupen64plus-next"),
    ("neogeo", "geolith"),
    ("nes", "fceumm"),
    ("psx", "pcsx_rearmed"),
    ("snes", "snes9x"),
];

/// Where KNULLI keeps libretro cores.
pub const CORE_DIR: &str = "/usr/lib/libretro";

/// Decide whether this launch can be handled natively.
///
/// `core_exists` is injected rather than called directly so the decision can
/// be tested without a device and without a filesystem full of `.so` files.
pub fn plan(conf: &Conf, core_exists: &dyn Fn(&str) -> bool) -> Option<Plan> {
    let system = conf.system();
    if system.is_empty() {
        return None;
    }

    // An explicit emulator that is not libretro is somebody else's job:
    // standalone PPSSPP, flycast, amiberry and the other 50-odd all want
    // evmapy set up and a different command line entirely.
    match conf.get("emulator") {
        None | Some("libretro") => {}
        Some(_) => return None,
    }

    // A core the user pinned wins. Otherwise fall back to the table, and if
    // the system is not in it, refuse — an unknown system is exactly the case
    // where guessing is worse than being slow.
    let core = conf.get("core").map(str::to_string).or_else(|| {
        DEFAULT_CORES
            .iter()
            .find(|(s, _)| *s == system)
            .map(|(_, c)| (*c).to_string())
    })?;

    if core.is_empty() || core.contains('/') {
        // A core name is a bare identifier. A path here means something has
        // gone wrong upstream, and building `/usr/lib/libretro/../..` out of
        // it would be worse than refusing.
        return None;
    }

    let core_path = format!("{CORE_DIR}/{core}_libretro.so");
    if !core_exists(&core_path) {
        return None;
    }

    Some(Plan {
        system: system.to_string(),
        core,
        core_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every core exists.
    fn all(_: &str) -> bool {
        true
    }
    /// No core exists.
    fn none(_: &str) -> bool {
        false
    }

    fn conf(text: &str, system: &str) -> Conf {
        Conf::parse(text, system, "game.rom")
    }

    #[test]
    fn uses_the_pinned_core() {
        let c = conf("gba.core=vba-m\n", "gba");
        let p = plan(&c, &all).expect("should plan");
        assert_eq!(p.core, "vba-m");
        assert_eq!(p.core_path, "/usr/lib/libretro/vba-m_libretro.so");
    }

    #[test]
    fn falls_back_to_the_default_table() {
        let p = plan(&conf("", "nes"), &all).expect("nes has a default");
        assert_eq!(p.core, "fceumm");
    }

    #[test]
    fn a_pinned_core_beats_the_table() {
        let c = conf("gba.core=mgba\n", "gba");
        assert_eq!(plan(&c, &all).unwrap().core, "mgba", "table is only a floor");
    }

    #[test]
    fn game_scope_reaches_the_plan() {
        // The whole point of the conf layering: one game on a different core.
        let text = "gba.core=vba-m\ngba[\"Special.gba\"].core=mgba\n";
        let c = Conf::parse(text, "gba", "Special.gba");
        assert_eq!(plan(&c, &all).unwrap().core, "mgba");
        let c = Conf::parse(text, "gba", "Ordinary.gba");
        assert_eq!(plan(&c, &all).unwrap().core, "vba-m");
    }

    #[test]
    fn refuses_an_unknown_system() {
        assert_eq!(plan(&conf("", "amiga500"), &all), None);
    }

    #[test]
    fn refuses_a_non_libretro_emulator() {
        // This is the case that would have broken all 55 standalone
        // emulators if the evmapy guard had been unconditional.
        let c = conf("dreamcast.emulator=flycast\ndreamcast.core=flycast\n", "dreamcast");
        assert_eq!(plan(&c, &all), None);
    }

    #[test]
    fn accepts_an_explicit_libretro_emulator() {
        let c = conf("n64.emulator=libretro\nn64.core=mupen64plus-next\n", "n64");
        assert_eq!(plan(&c, &all).unwrap().core, "mupen64plus-next");
    }

    #[test]
    fn refuses_when_the_core_file_is_missing() {
        assert_eq!(plan(&conf("gba.core=vba-m\n", "gba"), &none), None);
    }

    #[test]
    fn refuses_a_core_name_that_is_a_path() {
        let c = conf("gba.core=../../bin/sh\n", "gba");
        assert_eq!(plan(&c, &all), None, "a core name is not a path");
    }

    #[test]
    fn refuses_an_empty_core_or_system() {
        assert_eq!(plan(&conf("gba.core=\n", "gba"), &all), None);
        assert_eq!(plan(&conf("", ""), &all), None);
    }

    #[test]
    fn every_default_is_a_plausible_core_name() {
        for (system, core) in DEFAULT_CORES {
            assert!(!system.is_empty() && !core.is_empty());
            assert!(!core.contains('/'), "{core} looks like a path");
            assert!(
                !core.ends_with("_libretro"),
                "{core} already carries the suffix the path builder adds"
            );
        }
    }

    #[test]
    fn the_default_table_is_sorted_and_unique() {
        // Sorted so a human can check it against the device by eye, unique so
        // a duplicate cannot silently shadow the entry below it.
        let names: Vec<&str> = DEFAULT_CORES.iter().map(|(s, _)| *s).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted, "keep DEFAULT_CORES sorted and free of duplicates");
    }
}

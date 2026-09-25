//! Which KNULLI this build was checked against, and what to do when it moves.
//!
//! Every patch here is a bet about a file somebody else ships: that
//! `knulli.conf` is read first-wins, that the idle hooks are two and are named
//! `mode` and `extendedmode`, that `lid-control` exists and reads `system.lid`,
//! that ES's logo is at that path on the squashfs. None of that is promised by
//! KNULLI, and an OS update is free to move any of it.
//!
//! When it does, a patch does not fail loudly — it writes a key nothing reads
//! any more, and reports itself on. `never-sleep` did exactly that for weeks.
//! So the build records the version it was checked against and says so, and
//! refuses to apply anything against a KNULLI it has never seen.
//!
//! Refusing is the useful behaviour rather than a warning nobody reads: a
//! patcher applied against the wrong OS leaves markers in files, and undoing
//! that needs the same wrong build. `--anyway` is there for when the person at
//! the keyboard knows better.

use crate::patch::Paths;

/// The KNULLI these patches were read against and tested on.
///
/// Read off the device with `knulli-version`, which prints this file plus a
/// flag or two about what is customised; the flags are not part of the
/// version and are not recorded here.
///
/// **Bump this only after checking the patches against the new image**, not to
/// make the warning go away. What to re-read is in `docs/knulli-addon.md`.
pub const BUILT_FOR: &str = "scarab 2026/05/10 22:54";

/// What the device is running, against what this build knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The KNULLI this build was checked against.
    Same,
    /// A different one. The patches may write keys nothing reads.
    Moved(String),
    /// No version file — not a KNULLI, or a build that stopped shipping one.
    Unknown,
}

/// The version string on the device, trimmed. `None` when the file is absent.
pub fn installed(paths: &Paths) -> Option<String> {
    let text = std::fs::read_to_string(paths.knulli_version()).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

pub fn check(paths: &Paths) -> Verdict {
    match installed(paths) {
        None => Verdict::Unknown,
        Some(v) if v == BUILT_FOR => Verdict::Same,
        Some(v) => Verdict::Moved(v),
    }
}

impl Verdict {
    /// One line for `--status` and for the top of the patches tab.
    pub fn line(&self) -> String {
        match self {
            Verdict::Same => format!("{BUILT_FOR} — the one these patches were made for"),
            Verdict::Moved(found) => {
                format!("{found} — these patches were made for {BUILT_FOR}; update moose-patch")
            }
            Verdict::Unknown => {
                format!("not found — these patches were made for {BUILT_FOR}")
            }
        }
    }

    /// Whether applying should go ahead without being asked twice.
    ///
    /// `Unknown` is allowed: the tests run against a scratch directory with no
    /// squashfs in it, and so does anyone trying the patcher somewhere new. A
    /// KNULLI that says it is a *different* KNULLI is the case worth stopping.
    pub fn safe_to_apply(&self) -> bool {
        !matches!(self, Verdict::Moved(_))
    }

    /// What to print when it is not safe and `--anyway` was not given.
    pub fn refusal(&self) -> String {
        format!(
            "this KNULLI is {}\nmoose-patch was checked against {BUILT_FOR}, and a patch \
             written for another image can silently set a key nothing reads.\nUpdate \
             moose-patch, or pass --anyway if you know the difference does not matter.",
            match self {
                Verdict::Moved(found) => found.clone(),
                _ => "not the one this build knows".into(),
            }
        )
    }

    /// The same, for the screen, where there is no `--anyway` to offer.
    pub fn refusal_on_screen(&self) -> String {
        let found = match self {
            Verdict::Moved(found) => found.as_str(),
            _ => "not one this build knows",
        };
        format!(
            "This KNULLI is {found}. moose-patch was checked against {BUILT_FOR}, and a patch \
             written for another image can set a key nothing reads. Update moose-patch first. \
             Nothing was written."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("moose-knulli-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("usr/share/knulli")).unwrap();
        Paths::new(dir)
    }

    fn write(paths: &Paths, text: &str) {
        std::fs::write(paths.knulli_version(), text).unwrap();
    }

    #[test]
    fn the_version_we_were_built_for_is_the_same_one() {
        let paths = scratch("same");
        write(&paths, &format!("{BUILT_FOR}\n"));
        assert_eq!(check(&paths), Verdict::Same);
        assert!(check(&paths).safe_to_apply());
    }

    #[test]
    fn an_updated_knulli_stops_an_apply() {
        // The whole point. An image that is not the one the patches were read
        // against must not be written to without somebody saying so.
        let paths = scratch("moved");
        write(&paths, "scarab 2026/11/01 09:00\n");
        assert_eq!(check(&paths), Verdict::Moved("scarab 2026/11/01 09:00".into()));
        assert!(!check(&paths).safe_to_apply());
        assert!(check(&paths).refusal().contains("2026/11/01"));
        assert!(check(&paths).refusal().contains(BUILT_FOR), "say what it wanted");
    }

    #[test]
    fn no_version_file_is_not_a_refusal() {
        // Every test in this crate runs against a scratch directory with no
        // squashfs in it. If a missing file blocked applying, none of them
        // could apply anything.
        let paths = scratch("absent");
        assert_eq!(check(&paths), Verdict::Unknown);
        assert!(check(&paths).safe_to_apply());
    }

    #[test]
    fn the_flags_knulli_version_prints_are_not_part_of_it() {
        // `knulli-version` appends "[c]" and friends for a custom.sh and the
        // like. Comparing against that would report every customised device as
        // a different OS — so the file is read, not the command.
        let paths = scratch("flags");
        write(&paths, &format!("{BUILT_FOR}\n"));
        assert_eq!(installed(&paths).as_deref(), Some(BUILT_FOR));
    }
}

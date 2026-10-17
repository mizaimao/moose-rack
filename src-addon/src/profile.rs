//! What you had, written down.
//!
//! This is the answer to *"even on a newly installed KNULLI we can easily
//! configure and recover all of those customized settings"*. The profile is
//! one small text file naming the option each patch was sitting at; the binary
//! carries everything else. So recovery on a device that has never seen this
//! app is: copy two files across, run `moose-patch --restore`.
//!
//! Options are stored **by name, not by index**. An index would silently mean
//! something different the moment a new option is added to a patch, and the
//! failure would be a device configured wrongly rather than an error.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;

use crate::patch::{Patch, Paths, State};

/// Read the profile, if there is one. A missing file is not an error — it is
/// a device that has not been set up yet.
///
/// Any other failure is. Read as empty, a profile that could not be read was
/// saved over on the next apply with "never saved" for every patch that reads
/// as changed, and the one file a reflashed device is restored from was gone.
pub fn load(paths: &Paths) -> Result<BTreeMap<String, String>> {
    let path = paths.profile();
    match fs::read_to_string(&path) {
        Ok(text) => Ok(parse(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((id, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"');
        out.insert(id.trim().to_string(), value.to_string());
    }
    out
}

/// The patches a save could not read a setting off, and what it did instead.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Saved {
    /// Read as `changed`, so the line from the profile before stays, as
    /// `id = option`.
    pub kept: Vec<String>,
    /// Read as `changed` with nothing saved before to keep.
    pub missing: Vec<String>,
}

/// The profile for `patches` as they read now. `before` is the profile being
/// replaced: a patch that reads as `changed` keeps its line from there.
///
/// It used to be dropped. The profile is rewritten after every on-screen
/// apply, and ES rewriting never-sleep's line is enough to make it read as
/// changed, so one apply of anything else took never-sleep out of the file a
/// reflashed device is restored from.
pub fn render(patches: &[Patch], before: &BTreeMap<String, String>) -> (String, Saved) {
    let mut out = String::from(
        "# moose-patch — what this device is set to.\n\
         # Options are named rather than numbered, so adding one to a patch\n\
         # later cannot silently change what an old profile means.\n\n",
    );
    let mut saved = Saved::default();
    for patch in patches {
        match patch.state() {
            State::At(i) => {
                if let Some(choice) = patch.choices.get(i) {
                    out.push_str(&format!("{} = \"{}\"\n", patch.id, choice.name));
                }
            }
            State::Changed => match before.get(patch.id) {
                Some(name) => {
                    out.push_str(&format!(
                        "# {} reads as changed on the device; this is what was saved before.\n\
                         {} = \"{name}\"\n",
                        patch.id, patch.id
                    ));
                    saved.kept.push(format!("{} = {name}", patch.id));
                }
                None => {
                    out.push_str(&format!(
                        "# {} reads as changed on the device and was never saved.\n",
                        patch.id
                    ));
                    saved.missing.push(patch.id.to_string());
                }
            },
        }
    }
    // Lines for patches this build does not have, from a profile a newer build
    // wrote. They go back out as they came in, so running an older build for a
    // while does not cost the newer one its settings.
    let foreign: Vec<_> = before
        .iter()
        .filter(|(id, _)| !patches.iter().any(|p| p.id == id.as_str()))
        .collect();
    if !foreign.is_empty() {
        out.push_str("\n# Not patches this build knows; kept as they were.\n");
        for (id, name) in foreign {
            out.push_str(&format!("{id} = \"{name}\"\n"));
        }
    }
    (out, saved)
}

/// Write down where every patch currently sits. What it could not read off
/// the device is in the answer, so the caller can say so.
///
/// A profile that is there and cannot be read stops the save, and the file is
/// left as it is.
pub fn save(paths: &Paths, patches: &[Patch]) -> Result<Saved> {
    let before = load(paths)?;
    let path = paths.profile();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let (text, saved) = render(patches, &before);
    crate::patch::write_through(&path, text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(saved)
}

/// What `restore` did, so it can be printed rather than guessed at.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Restored {
    pub applied: Vec<String>,
    pub already: Vec<String>,
    /// Named in the profile but not in this build, or naming an option that no
    /// longer exists. Reported rather than ignored: a profile that half works
    /// silently is worse than one that says which half.
    pub unknown: Vec<String>,
    /// Tried and could not. One patch failing must not abandon the rest — on
    /// a freshly flashed device that would mean a single read-only mount
    /// costs you every other setting you were restoring.
    pub failed: Vec<String>,
}

/// Put the device back to what the profile says. This is the whole point of
/// the file.
pub fn restore(paths: &Paths, patches: &[Patch]) -> Result<Restored> {
    let wanted = load(paths)?;
    let mut out = Restored::default();
    for (id, name) in wanted {
        let Some(patch) = patches.iter().find(|p| p.id == id) else {
            out.unknown.push(id);
            continue;
        };
        let Some(index) = patch.choices.iter().position(|c| c.name == name) else {
            out.unknown.push(format!("{id} = {name}"));
            continue;
        };
        if patch.state() == State::At(index) {
            out.already.push(id);
            continue;
        }
        match patch.apply(index) {
            Ok(()) => out.applied.push(format!("{id} = {name}")),
            Err(e) => out.failed.push(format!("{id} = {name}: {e:#}")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue;

    /// With KNULLI's own trigger file, which hotkey-app is written from.
    fn scratch(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("moose-profile-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let paths = Paths::new(dir);
        fs::create_dir_all(paths.stock_triggers().parent().unwrap()).unwrap();
        fs::write(paths.stock_triggers(), "KEY_POWER 1  /usr/bin/power-button\n").unwrap();
        paths
    }

    #[test]
    fn a_device_can_be_rebuilt_from_one_file() {
        // The whole promise: set a device up, keep the profile, and a machine
        // that has never seen this app ends up identical.
        let first = scratch("source");
        let all = catalogue::all(&first);
        for patch in &all {
            let last = patch.choices.len() - 1;
            patch.apply(last).unwrap();
        }
        save(&first, &all).unwrap();
        let profile = fs::read_to_string(first.profile()).unwrap();

        let fresh = scratch("target");
        fs::create_dir_all(fresh.profile().parent().unwrap()).unwrap();
        fs::write(fresh.profile(), &profile).unwrap();

        let theirs = catalogue::all(&fresh);
        let done = restore(&fresh, &theirs).unwrap();
        assert!(done.unknown.is_empty(), "{:?}", done.unknown);
        assert_eq!(done.applied.len(), all.len());

        for patch in &theirs {
            let last = patch.choices.len() - 1;
            assert_eq!(
                patch.state(),
                State::At(last),
                "{} did not come back",
                patch.id
            );
        }
    }

    #[test]
    fn restoring_twice_is_a_no_op() {
        let paths = scratch("twice");
        let all = catalogue::all(&paths);
        all.iter().find(|p| p.id == "hotkeys").unwrap().apply(1).unwrap();
        save(&paths, &all).unwrap();

        let first = restore(&paths, &all).unwrap();
        assert!(first.applied.is_empty(), "already there: {:?}", first.applied);
        let again = restore(&paths, &all).unwrap();
        assert_eq!(first, again);
    }

    #[test]
    #[cfg(unix)]
    fn one_patch_failing_does_not_abandon_the_rest() {
        // This is not hypothetical: /boot on the device is mounted read-only,
        // and the first restore attempt lost every setting that came after it
        // alphabetically.
        use std::os::unix::fs::PermissionsExt;
        let paths = scratch("keeps-going");
        let all = catalogue::all(&paths);

        // Make the directory the es-logo patch writes into refuse writes.
        let blocked = paths.blank_logo();
        let dir = blocked.parent().unwrap().to_path_buf();
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(paths.profile().parent().unwrap()).unwrap();
        fs::write(
            paths.profile(),
            "es-logo = \"ON\"\nnever-sleep = \"ON\"\nhotkeys = \"ON\"\n",
        )
        .unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

        let done = restore(&paths, &all).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(done.failed.len(), 1, "{done:?}");
        assert!(
            done.applied.iter().any(|s| s.starts_with("hotkeys")),
            "the rest still had to be applied: {done:?}"
        );
        assert!(
            done.applied.iter().any(|s| s.starts_with("never-sleep")),
            "{done:?}"
        );
    }

    #[test]
    fn an_option_that_no_longer_exists_is_reported_not_ignored() {
        // A profile from a newer build, or a hand-edited one. Applying nothing
        // and saying so beats applying something adjacent.
        let paths = scratch("stale");
        fs::create_dir_all(paths.profile().parent().unwrap()).unwrap();
        fs::write(paths.profile(), "hotkeys = \"sideways\"\nnosuch = \"ON\"\n").unwrap();
        let done = restore(&paths, &catalogue::all(&paths)).unwrap();
        assert!(done.applied.is_empty());
        assert_eq!(done.unknown.len(), 2);
    }

    #[test]
    fn the_profile_names_options_rather_than_numbering_them() {
        let paths = scratch("names");
        let all = catalogue::all(&paths);
        all.iter().find(|p| p.id == "shaders").unwrap().apply(2).unwrap();
        let (text, _) = render(&all, &BTreeMap::new());
        assert!(
            text.contains("shaders = \"shimmerless plain\""),
            "should record the name:\n{text}"
        );
    }

    #[test]
    fn a_patch_that_reads_changed_keeps_what_was_saved_before() {
        // ES rewrites never-sleep's line and it reads as changed. The next
        // save, after any on-screen apply, used to drop it from the profile,
        // and a reflashed device came back without it.
        let paths = scratch("keeps-changed");
        let all = catalogue::all(&paths);
        let never_sleep = all.iter().find(|p| p.id == "never-sleep").unwrap();
        never_sleep.apply(1).unwrap();
        save(&paths, &all).unwrap();

        let conf = paths.knulli_conf();
        let text = fs::read_to_string(&conf).unwrap();
        fs::write(&conf, text.replace("extendedmode=none", "extendedmode=suspend")).unwrap();
        assert_eq!(never_sleep.state(), State::Changed);

        let saved = save(&paths, &all).unwrap();
        assert_eq!(saved.kept, vec!["never-sleep = ON"]);
        let profile = fs::read_to_string(paths.profile()).unwrap();
        assert!(profile.contains("never-sleep = \"ON\""), "{profile}");
        assert!(profile.contains("never-sleep reads as changed"), "{profile}");

        // And a restore puts it back.
        restore(&paths, &all).unwrap();
        assert_eq!(never_sleep.state(), State::At(1));
    }

    /// A profile written by a newer build names patches this one does not
    /// have. A save here used to walk only the patches it knows and drop the
    /// rest, so going back to the newer build found them gone.
    #[test]
    fn a_save_keeps_lines_for_patches_this_build_does_not_know() {
        let paths = scratch("foreign");
        let all = catalogue::all(&paths);
        fs::create_dir_all(paths.profile().parent().unwrap()).unwrap();
        fs::write(paths.profile(), "from-a-newer-build = \"ON\"\n").unwrap();

        save(&paths, &all).unwrap();
        let profile = fs::read_to_string(paths.profile()).unwrap();
        assert!(profile.contains("from-a-newer-build = \"ON\""), "{profile}");
        assert_eq!(
            load(&paths).unwrap().get("from-a-newer-build").map(String::as_str),
            Some("ON")
        );
        // Saving again does not write it twice.
        save(&paths, &all).unwrap();
        let again = fs::read_to_string(paths.profile()).unwrap();
        assert_eq!(again.matches("from-a-newer-build").count(), 1, "{again}");
    }

    /// A profile that is there but cannot be read is not an empty one. Read as
    /// empty, the save after it wrote "never saved" over every setting in it.
    #[test]
    fn a_profile_that_cannot_be_read_is_not_saved_over() {
        let paths = scratch("unreadable");
        let all = catalogue::all(&paths);
        fs::create_dir_all(paths.profile().parent().unwrap()).unwrap();
        let bytes = b"hotkeys = \"ON\"\nnever-sleep = \"\xff\"\n";
        fs::write(paths.profile(), bytes).unwrap();

        assert!(load(&paths).is_err());
        assert!(save(&paths, &all).is_err(), "saved over a profile it could not read");
        assert_eq!(fs::read(paths.profile()).unwrap(), bytes);
        assert!(restore(&paths, &all).is_err());

        // No file at all is still a device that has not been set up.
        fs::remove_file(paths.profile()).unwrap();
        assert!(load(&paths).unwrap().is_empty());
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let map = parse("# a note\n\nhotkeys = \"ON\"\n  \nbezels=\"off\"\n");
        assert_eq!(map.get("hotkeys").map(String::as_str), Some("ON"));
        assert_eq!(map.get("bezels").map(String::as_str), Some("off"));
        assert_eq!(map.len(), 2);
    }
}

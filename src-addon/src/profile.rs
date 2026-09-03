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
pub fn load(paths: &Paths) -> BTreeMap<String, String> {
    let text = fs::read_to_string(paths.profile()).unwrap_or_default();
    parse(&text)
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

pub fn render(patches: &[Patch]) -> String {
    let mut out = String::from(
        "# moose-patch — what this device is set to.\n\
         # Options are named rather than numbered, so adding one to a patch\n\
         # later cannot silently change what an old profile means.\n\n",
    );
    for patch in patches {
        if let State::At(i) = patch.state()
            && let Some(choice) = patch.choices.get(i)
        {
            out.push_str(&format!("{} = \"{}\"\n", patch.id, choice.name));
        }
    }
    out
}

/// Write down where every patch currently sits.
pub fn save(paths: &Paths, patches: &[Patch]) -> Result<()> {
    let path = paths.profile();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&path, render(patches))
        .with_context(|| format!("writing {}", path.display()))
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
    let wanted = load(paths);
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

    fn scratch(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("moose-profile-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Paths::new(dir)
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
        let text = render(&all);
        assert!(
            text.contains("shaders = \"shimmerless plain\""),
            "should record the name:\n{text}"
        );
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let map = parse("# a note\n\nhotkeys = \"ON\"\n  \nbezels=\"off\"\n");
        assert_eq!(map.get("hotkeys").map(String::as_str), Some("ON"));
        assert_eq!(map.get("bezels").map(String::as_str), Some("off"));
        assert_eq!(map.len(), 2);
    }
}

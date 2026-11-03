//! Game files whose hash has been checked, remembered by size and modification
//! time, so the launch checklist hashes a game once rather than on every Play.
//! A 4 GB disc takes seconds to hash; a stamp takes a `stat`.
//!
//! Kept in `<library>/verified.json`. Losing it costs one re-hash per game.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Stamp {
    size: u64,
    /// Seconds since the epoch.
    mtime: i64,
}

fn store(library_root: &Path) -> PathBuf {
    library_root.join("verified.json")
}

fn load(library_root: &Path) -> BTreeMap<String, Stamp> {
    std::fs::read(store(library_root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some(Stamp { size: m.len(), mtime })
}

fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether `path` was checked and hasn't changed since.
pub fn is_current(library_root: &Path, path: &Path) -> bool {
    let Some(now) = stamp(path) else { return false };
    load(library_root).get(&key(path)) == Some(&now)
}

/// Remember that `path`, as it is now, passed its hash check.
pub fn remember(library_root: &Path, path: &Path) {
    let Some(now) = stamp(path) else { return };
    let mut all = load(library_root);
    all.insert(key(path), now);
    if let Ok(json) = serde_json::to_vec_pretty(&all) {
        std::fs::create_dir_all(library_root).ok();
        let tmp = store(library_root).with_extension("json.part");
        if std::fs::write(&tmp, json).is_ok() {
            std::fs::rename(&tmp, store(library_root)).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checked_file_stays_checked_until_it_changes() {
        let lib = std::env::temp_dir().join("moose-verified");
        let _ = std::fs::remove_dir_all(&lib);
        std::fs::create_dir_all(&lib).unwrap();
        let game = lib.join("Ico (USA).chd");
        std::fs::write(&game, b"disc").unwrap();
        assert!(!is_current(&lib, &game));
        remember(&lib, &game);
        assert!(is_current(&lib, &game));
        std::fs::write(&game, b"a different disc").unwrap();
        assert!(!is_current(&lib, &game), "a changed size means a new check");
        assert!(!is_current(&lib, &lib.join("missing.chd")));
    }
}

//! Rotating local backups, taken before anything overwrites a save.
//!
//! Automatic sync is what makes this necessary. While syncing was a deliberate
//! action, a bad outcome was something you had chosen and could see; now a
//! download lands on launch and an upload leaves on exit without anyone
//! watching, so a corrupt or wrong-side-wins save can replace good progress
//! before it is noticed.
//!
//! Ten copies per `(rom, slot)`, oldest evicted:
//!
//! ```text
//! <library>/saves-backup/<rom_id>/<slot>/<unix millis>-<file name>
//! ```
//!
//! Deliberately plain files under the visible library folder rather than an
//! archive or a database — recovery is a copy in Finder, with no tooling and
//! nothing to learn at the moment when something has already gone wrong.
//!
//! Timestamps come from the file's own mtime rather than the clock, so a
//! restored backup keeps the identity it had and re-backing-up the same bytes
//! twice is a no-op.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// How many copies to keep per (rom, slot).
///
/// Ten covers the realistic case — noticing within a few sessions that a save
/// went backwards — without turning a 32 KB memory card into unbounded growth.
pub const KEEP: usize = 10;

/// Directory holding every backup, under the visible library folder.
pub fn root(library_root: &Path) -> PathBuf {
    library_root.join("saves-backup")
}

fn slot_dir(library_root: &Path, rom_id: i64, slot: &str) -> PathBuf {
    // A slot is a short token this project generates (`slot3`, `auto`,
    // `autosave`), but it reaches here from parsed filenames, so it is
    // sanitised rather than trusted as a path component.
    let safe: String = slot
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let safe = if safe.is_empty() { "unslotted".to_owned() } else { safe };
    root(library_root).join(rom_id.to_string()).join(safe)
}

/// Milliseconds since the epoch for a file's mtime, or 0 if unknown.
fn mtime_millis(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One stored backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub path: PathBuf,
    /// Epoch millis taken from the original file's mtime.
    pub stamp: u64,
    /// The save's own file name, as it would be restored.
    pub file_name: String,
}

/// Copy `save` aside before it is overwritten, and evict the oldest.
///
/// Returns `Ok(None)` when there was nothing to back up — a download for a save
/// this device does not have yet is the common case, and is not a failure.
///
/// Backing up the same bytes twice is a no-op: the stamp is the file's mtime,
/// so an unchanged file maps onto the copy already stored.
pub fn keep(library_root: &Path, rom_id: i64, slot: &str, save: &Path) -> Result<Option<PathBuf>> {
    if !save.is_file() {
        return Ok(None);
    }
    let file_name = save
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "save".to_owned());

    let dir = slot_dir(library_root, rom_id, slot);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let dest = dir.join(format!("{}-{file_name}", mtime_millis(save)));
    if dest.exists() {
        // Same file, same mtime: already held.
        return Ok(Some(dest));
    }
    std::fs::copy(save, &dest)
        .with_context(|| format!("copying {} to {}", save.display(), dest.display()))?;

    prune(&dir)?;
    Ok(Some(dest))
}

/// Drop the oldest copies until at most [`KEEP`] remain.
fn prune(dir: &Path) -> Result<()> {
    let mut found = list_dir(dir);
    if found.len() <= KEEP {
        return Ok(());
    }
    // Newest first, so everything past KEEP is the oldest.
    found.sort_by(|a, b| b.stamp.cmp(&a.stamp).then_with(|| b.path.cmp(&a.path)));
    for old in found.into_iter().skip(KEEP) {
        std::fs::remove_file(&old.path).ok();
    }
    Ok(())
}

fn list_dir(dir: &Path) -> Vec<Backup> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let (stamp, file_name) = name.split_once('-')?;
            Some(Backup {
                stamp: stamp.parse().ok()?,
                file_name: file_name.to_owned(),
                path: e.path(),
            })
        })
        .collect()
}

/// Every kept copy for one (rom, slot), newest first.
pub fn list(library_root: &Path, rom_id: i64, slot: &str) -> Vec<Backup> {
    let mut found = list_dir(&slot_dir(library_root, rom_id, slot));
    found.sort_by(|a, b| b.stamp.cmp(&a.stamp).then_with(|| b.path.cmp(&a.path)));
    found
}

/// Put a kept copy back, backing up whatever it replaces first.
///
/// Restoring is itself an overwrite, so the file being replaced is kept too —
/// otherwise restoring the wrong copy would be the one action with no undo.
pub fn restore(library_root: &Path, rom_id: i64, slot: &str, backup: &Path, dest: &Path) -> Result<()> {
    keep(library_root, rom_id, slot, dest)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::copy(backup, dest)
        .with_context(|| format!("restoring {} to {}", backup.display(), dest.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-savebackup-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a save with a controlled mtime, so ordering is deterministic
    /// rather than dependent on how fast the test runs.
    fn write_save(path: &Path, body: &[u8], millis: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(millis);
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(t).unwrap();
    }

    #[test]
    fn a_save_is_copied_aside_before_it_is_overwritten() {
        let dir = scratch("basic");
        let save = dir.join("saves/snes9x/Zelda.srm");
        write_save(&save, b"original", 1000);

        let kept = keep(&dir, 7, "autosave", &save).unwrap().expect("something to keep");
        assert!(kept.is_file());
        assert_eq!(std::fs::read(&kept).unwrap(), b"original");

        // The overwrite the backup exists for.
        write_save(&save, b"replaced", 2000);
        assert_eq!(std::fs::read(&save).unwrap(), b"replaced");
        assert_eq!(
            std::fs::read(&list(&dir, 7, "autosave")[0].path).unwrap(),
            b"original",
            "the previous contents are still recoverable"
        );
    }

    /// Nothing to back up is the ordinary case on a first download, not an
    /// error — it must not abort the sync that called it.
    #[test]
    fn a_missing_save_is_not_an_error() {
        let dir = scratch("missing");
        assert_eq!(keep(&dir, 7, "autosave", &dir.join("nope.srm")).unwrap(), None);
        assert!(list(&dir, 7, "autosave").is_empty());
    }

    /// Ten kept, oldest evicted. The cap is the whole point: without it a save
    /// synced on every launch grows without bound.
    #[test]
    fn only_the_ten_newest_are_kept() {
        let dir = scratch("rotate");
        let save = dir.join("saves/snes9x/Zelda.srm");

        for i in 1..=15u64 {
            write_save(&save, format!("version {i}").as_bytes(), i * 1000);
            keep(&dir, 7, "autosave", &save).unwrap();
        }

        let kept = list(&dir, 7, "autosave");
        assert_eq!(kept.len(), KEEP, "capped at {KEEP}");
        assert_eq!(
            std::fs::read(&kept[0].path).unwrap(),
            b"version 15",
            "newest first"
        );
        assert_eq!(
            std::fs::read(&kept[KEEP - 1].path).unwrap(),
            b"version 6",
            "versions 1-5 evicted, 6-15 kept"
        );
    }

    /// The stamp is the file's mtime, so syncing an unchanged save repeatedly
    /// does not consume the whole history with identical copies.
    #[test]
    fn backing_up_unchanged_bytes_twice_keeps_one_copy() {
        let dir = scratch("idempotent");
        let save = dir.join("saves/snes9x/Zelda.srm");
        write_save(&save, b"same", 5000);

        for _ in 0..5 {
            keep(&dir, 7, "autosave", &save).unwrap();
        }
        assert_eq!(list(&dir, 7, "autosave").len(), 1);
    }

    /// Slots and ROMs have separate histories. Sharing one would let a busy
    /// autosave slot evict the manual save you were keeping deliberately.
    #[test]
    fn each_rom_and_slot_rotates_independently() {
        let dir = scratch("scoped");
        let a = dir.join("a.srm");
        let b = dir.join("b.state1");
        write_save(&a, b"rom seven", 1000);
        write_save(&b, b"rom eight", 1000);

        keep(&dir, 7, "autosave", &a).unwrap();
        keep(&dir, 7, "slot1", &b).unwrap();
        keep(&dir, 8, "autosave", &b).unwrap();

        assert_eq!(list(&dir, 7, "autosave").len(), 1);
        assert_eq!(list(&dir, 7, "slot1").len(), 1);
        assert_eq!(list(&dir, 8, "autosave").len(), 1);
        assert!(list(&dir, 9, "autosave").is_empty());
    }

    /// Slots reach this from parsed filenames, so a hostile or merely odd one
    /// must not escape the backup tree.
    #[test]
    fn a_slot_name_cannot_escape_the_backup_directory() {
        let dir = scratch("traversal");
        let save = dir.join("Zelda.srm");
        write_save(&save, b"x", 1000);

        let kept = keep(&dir, 7, "../../etc/passwd", &save).unwrap().unwrap();
        assert!(
            kept.starts_with(root(&dir)),
            "{} escaped the backup root",
            kept.display()
        );
        assert!(!kept.to_string_lossy().contains(".."));
    }

    /// Restoring is an overwrite too, so the file it replaces is kept first.
    /// Restoring the wrong copy would otherwise be the one move with no undo.
    #[test]
    fn restoring_backs_up_what_it_replaces() {
        let dir = scratch("restore");
        let save = dir.join("saves/snes9x/Zelda.srm");
        write_save(&save, b"good", 1000);
        keep(&dir, 7, "autosave", &save).unwrap();

        write_save(&save, b"ruined", 2000);
        keep(&dir, 7, "autosave", &save).unwrap();

        // Put the older copy back.
        let history = list(&dir, 7, "autosave");
        assert_eq!(history.len(), 2);
        let older = history.iter().min_by_key(|b| b.stamp).unwrap().path.clone();
        restore(&dir, 7, "autosave", &older, &save).unwrap();

        assert_eq!(std::fs::read(&save).unwrap(), b"good", "restored");
        assert!(
            list(&dir, 7, "autosave").iter().any(|b| {
                std::fs::read(&b.path).map(|v| v == b"ruined").unwrap_or(false)
            }),
            "the version restoring replaced is still recoverable"
        );
    }

    /// Backups live in the visible library folder, so recovery is a file copy
    /// and deleting that one folder still reclaims everything.
    #[test]
    fn backups_live_under_the_visible_library_folder() {
        let dir = scratch("location");
        let save = dir.join("Zelda.srm");
        write_save(&save, b"x", 1000);
        let kept = keep(&dir, 7, "autosave", &save).unwrap().unwrap();

        assert!(kept.starts_with(dir.join("saves-backup")));
        // By component, not by a literal separator: `contains("/7/")` passes on
        // macOS and fails on Windows, which is exactly how it failed in CI.
        assert!(
            kept.components().any(|c| c.as_os_str() == "7"),
            "keyed by rom id: {}",
            kept.display()
        );
        assert!(kept.file_name().unwrap().to_string_lossy().ends_with("-Zelda.srm"));
    }
}

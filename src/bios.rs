//! BIOS files, fetched from the server into the visible library folder.
//!
//! Neo Geo, PlayStation, the MAME family and a dozen others refuse to start
//! without one, and the failure is opaque: the core loads, the screen stays
//! black or shows a line about a missing file, and nothing says which file or
//! where it should go.
//!
//! RomM keeps the BIOS set alongside the games, which is the point — the same
//! server that has the ROMs has the files needed to run them, so a second
//! machine is one sync away rather than a manual hunt.
//!
//! Everything lands flat in `<library>/system/`, not in the folders the server
//! groups them under. RetroArch looks in exactly one directory and does not
//! recurse, so `bios/3do/panafz1.bin` on the server has to become
//! `system/panafz1.bin` here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use serde::Deserialize;

use crate::api::{Client, Firmware};

/// The BIOS manifest, compiled in.
///
/// Same reasoning as the core map: a Windows install is one loose executable,
/// and a feature that needs a file nobody shipped is a feature that does not
/// work there.
pub const MANIFEST: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/bios-manifest.json"));

/// One BIOS the manifest knows about: which file, and what needs it.
#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    /// File name as RetroArch expects it in the system directory.
    pub path: String,
    #[serde(default)]
    pub desc: String,
    /// False for BIOS that only unlock optional behavior.
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub cores: Vec<String>,
    #[serde(default)]
    pub systems: Vec<String>,
}

fn manifest() -> Vec<Entry> {
    serde_json::from_str(MANIFEST).unwrap_or_default()
}

/// The BIOS files a launch of `core` on `platform` needs.
///
/// Matched on either, because the manifest describes both and neither alone is
/// enough: `neogeo.zip` is listed against the arcade *systems* while
/// `geolith`'s own BIOS is listed against the *core*.
///
/// Only the required ones. Optional BIOS unlock extra behavior rather than
/// making a game run, and pulling the whole set on the way into a game is the
/// opposite of what an automatic fetch should feel like.
pub fn required_for(core: &str, platform: &str) -> Vec<String> {
    manifest()
        .into_iter()
        .filter(|e| e.required)
        .filter(|e| {
            e.cores.iter().any(|c| c == core) || e.systems.iter().any(|s| s == platform)
        })
        .map(|e| e.path)
        .collect()
}

/// Fetch any BIOS this launch needs and does not have.
///
/// Called on the way into a game rather than offered as a button: a missing
/// BIOS produces a black screen and a line in a log nobody reads, so "install
/// it yourself" is advice given at the exact moment the user cannot see why.
///
/// Returns how many were fetched. Never fatal — an unreachable server should
/// not stop a game that might well run without one.
pub async fn ensure(
    client: &Client,
    library_root: &Path,
    core: &str,
    platform: &str,
) -> Result<usize> {
    let wanted = required_for(core, platform);
    if wanted.is_empty() {
        return Ok(0);
    }
    let dest = system_dir(library_root);
    let missing: Vec<String> = wanted
        .into_iter()
        .filter(|name| !dest.join(name).is_file())
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }

    std::fs::create_dir_all(&dest)
        .with_context(|| format!("creating {}", dest.display()))?;
    let available = client.firmware().await?;
    let mut got = 0;

    for name in &missing {
        // The server's own name for it may carry a path; match on the leaf.
        let Some(fw) = available.iter().find(|f| {
            Path::new(&f.file_name)
                .file_name()
                .is_some_and(|n| n.to_string_lossy() == name.as_str())
        }) else {
            continue;
        };
        if let Ok(bytes) = client.firmware_content(fw.id, &fw.file_name).await {
            // This one stays flat on purpose: it fetches a *named* BIOS a core
            // asked for, and RetroArch looks for that name in `system/` itself.
            let part = dest.join(format!("{name}.part"));
            if std::fs::write(&part, &bytes).is_ok()
                && std::fs::rename(&part, dest.join(name)).is_ok()
            {
                got += 1;
            } else {
                std::fs::remove_file(&part).ok();
            }
        }
    }
    Ok(got)
}

/// What a sync did.
#[derive(Debug, Default)]
pub struct Summary {
    pub downloaded: usize,
    pub already_had: usize,
    pub failed: usize,
    pub bytes: u64,
    pub notes: Vec<String>,
}

impl Summary {
    pub fn headline(&self) -> String {
        if self.downloaded == 0 && self.failed == 0 {
            return format!("BIOS already complete ({} files)", self.already_had);
        }
        let mut s = format!(
            "{} downloaded ({}), {} already present",
            self.downloaded,
            crate::util::human(self.bytes),
            self.already_had
        );
        if self.failed > 0 {
            s.push_str(&format!(", {} failed", self.failed));
        }
        s
    }
}

/// Where BIOS live locally.
pub fn system_dir(library_root: &Path) -> PathBuf {
    library_root.join("system")
}

/// Is this file already here and intact?
///
/// Size first because it is free, then the hash — a truncated download has the
/// wrong size and a corrupted one does not, and only the second needs reading
/// the file back.
fn already_have(path: &Path, want: &Firmware) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if want.file_size_bytes > 0 && meta.len() != want.file_size_bytes as u64 {
        return false;
    }
    match want.md5_hash.as_deref().filter(|h| !h.is_empty()) {
        Some(want_md5) => crate::download::hash_file(path)
            .map(|(md5, _)| md5.eq_ignore_ascii_case(want_md5))
            .unwrap_or(false),
        // No published hash: size is all there is to go on.
        None => true,
    }
}

/// Download every BIOS the server has that is not already here.
///
/// `progress` is called with `(done, total, name)` so a frontend can show which
/// file it is on — the set is 67 files on this server and a bare spinner says
/// nothing about how far through it is.
/// What a BIOS sync would do, without doing any of it.
///
/// The button that starts one used to sit silent for as long as the listing
/// took, which reads as a control that does nothing. Asking first costs one
/// request and answers the only questions worth asking: is this already done,
/// and how much would it fetch.

/// Where one firmware file belongs under `system/`.
///
/// The server reports a `file_path` — `mame`, `pcsx2/Langs/de_DE` — and the tree
/// is reproduced, because a flat directory silently loses files: the real set
/// has 3,339 entries under 2,738 distinct names, so flattening overwrites 601 of
/// them. `config.ini` alone appears 199 times, once per MSX machine.
///
/// Neither half is trusted as a path. A server that answers `../../.ssh` or an
/// absolute path is answering something this must not write to, so every
/// component is checked and `..`, `.`, empty and rooted segments are dropped.
/// The name contributes its leaf only.
pub fn local_path(dest: &Path, file_path: Option<&str>, file_name: &str) -> Option<PathBuf> {
    let leaf = Path::new(file_name).file_name()?.to_string_lossy().into_owned();
    if leaf.is_empty() {
        return None;
    }
    let mut out = dest.to_path_buf();
    if let Some(rel) = file_path {
        for part in rel.split(['/', '\\']) {
            let part = part.trim();
            if part.is_empty() || part == "." || part == ".." {
                continue;
            }
            // Anything that is not a plain name is not a path component here.
            if Path::new(part).components().count() != 1 {
                continue;
            }
            out.push(part);
        }
    }
    Some(out.join(leaf))
}

pub async fn status(client: &Client, library_root: &Path) -> Result<(usize, usize, u64)> {
    let list = client
        .firmware()
        .await
        .context("listing BIOS files (does the token have firmware.read?)")?;
    let dest = system_dir(library_root);

    let mut have = 0;
    let mut bytes = 0u64;
    for fw in &list {
        if fw.file_name.is_empty() {
            continue;
        }
        let Some(path) = local_path(&dest, fw.file_path.as_deref(), &fw.file_name) else {
            continue;
        };
        if already_have(&path, fw) {
            have += 1;
        } else {
            bytes += fw.file_size_bytes.max(0) as u64;
        }
    }
    Ok((list.len(), have, bytes))
}

pub async fn sync(
    client: &Client,
    library_root: &Path,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<Summary> {
    let list = client
        .firmware()
        .await
        .context("listing BIOS files (does the token have firmware.read?)")?;

    let dest = system_dir(library_root);
    std::fs::create_dir_all(&dest)
        .with_context(|| format!("creating {}", dest.display()))?;

    let mut summary = Summary::default();
    let total = list.len();

    for (i, fw) in list.iter().enumerate() {
        if fw.file_name.is_empty() {
            continue;
        }
        // Neither the name nor the reported path is trusted; see `local_path`.
        let Some(path) = local_path(&dest, fw.file_path.as_deref(), &fw.file_name) else {
            continue;
        };
        let leaf = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                continue;
            }
        }
        progress(i + 1, total, &leaf);

        if already_have(&path, fw) {
            summary.already_had += 1;
            continue;
        }

        match client.firmware_content(fw.id, &fw.file_name).await {
            Ok(bytes) => {
                // Written to a temporary name and renamed, so an interrupted
                // download cannot leave a half a BIOS looking like a whole one
                // — which fails in the emulator rather than here.
                let part = dest.join(format!("{leaf}.part"));
                if let Err(e) = std::fs::write(&part, &bytes) {
                    summary.failed += 1;
                    summary.notes.push(format!("{leaf}: {e}"));
                    continue;
                }
                match std::fs::rename(&part, &path) {
                    Ok(()) => {
                        summary.downloaded += 1;
                        summary.bytes += bytes.len() as u64;
                    }
                    Err(e) => {
                        std::fs::remove_file(&part).ok();
                        summary.failed += 1;
                        summary.notes.push(format!("{leaf}: {e}"));
                    }
                }
            }
            Err(e) => {
                summary.failed += 1;
                summary.notes.push(format!(
                    "{leaf}: {}",
                    e.to_string().lines().next().unwrap_or("failed")
                ));
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-bios-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fw(name: &str, size: i64, md5: Option<&str>) -> Firmware {
        Firmware {
            id: 1,
            file_name: name.to_owned(),
            file_size_bytes: size,
            md5_hash: md5.map(str::to_owned),
            sha1_hash: None,
            file_path: None,
            is_verified: true,
        }
    }

    /// Everything goes flat into one directory. RetroArch looks in exactly one
    /// place and does not recurse, so reproducing the server's `bios/3do/`
    /// grouping would hide every file from the emulator that needs it.
    #[test]
    fn bios_land_flat_in_the_system_directory() {
        let root = Path::new("/lib");
        assert_eq!(system_dir(root), Path::new("/lib/system"));
    }

    /// A file already here with the right size and hash is not fetched again —
    /// this runs against 67 files and most of them will not have changed.
    #[test]
    fn an_intact_file_is_recognized() {
        let dir = scratch("intact");
        let path = dir.join("bios.bin");
        std::fs::write(&path, b"hello").unwrap();
        // md5 of "hello"
        let md5 = "5d41402abc4b2a76b9719d911017c592";

        assert!(already_have(&path, &fw("bios.bin", 5, Some(md5))));
        assert!(!already_have(&path, &fw("bios.bin", 5, Some("0".repeat(32).as_str()))));
    }

    /// Wrong size is enough on its own, and is checked first because it costs
    /// nothing — a truncated download does not need its hash computing.
    #[test]
    fn a_truncated_file_is_refetched() {
        let dir = scratch("short");
        let path = dir.join("bios.bin");
        std::fs::write(&path, b"hel").unwrap();
        assert!(!already_have(&path, &fw("bios.bin", 5, None)));
    }

    /// Without a published hash there is nothing to check but the size, which
    /// must still be enough to avoid downloading the whole set every run.
    #[test]
    fn a_file_with_no_published_hash_is_accepted_on_size() {
        let dir = scratch("nohash");
        let path = dir.join("bios.bin");
        std::fs::write(&path, b"hello").unwrap();
        assert!(already_have(&path, &fw("bios.bin", 5, None)));
        assert!(!already_have(&path, &fw("bios.bin", 999, None)));
    }

    /// A file that is not there at all is not "already had".
    #[test]
    fn a_missing_file_is_not_mistaken_for_a_present_one() {
        let dir = scratch("absent");
        assert!(!already_have(&dir.join("nope.bin"), &fw("nope.bin", 5, None)));
    }

    #[test]
    fn the_headline_says_nothing_happened_when_nothing_did() {
        let s = Summary { already_had: 67, ..Default::default() };
        assert_eq!(s.headline(), "BIOS already complete (67 files)");
        let s = Summary { downloaded: 3, already_had: 64, bytes: 1024, failed: 1, ..Default::default() };
        assert!(s.headline().contains("3 downloaded"));
        assert!(s.headline().contains("1 failed"));
    }

    /// The manifest is compiled in, so it must parse. Infallible by
    /// construction only if something checks — a broken file would otherwise
    /// silently mean "no BIOS is ever required" and every Neo Geo game would
    /// fail with a black screen and no fetch attempted.
    #[test]
    fn the_embedded_manifest_parses_and_is_populated() {
        let all = manifest();
        assert!(all.len() > 100, "got {}", all.len());
        assert!(all.iter().any(|e| e.path == "neogeo.zip"));
    }

    /// The case Frank hit: a Neo Geo game will not start without these, and
    /// they are listed against the arcade systems rather than the core.
    #[test]
    fn neo_geo_needs_its_bios_whichever_way_it_is_launched() {
        for (core, platform) in [("geolith", "neogeoaes"), ("fbneo", "arcade"), ("mame2003_plus", "mame")] {
            let need = required_for(core, platform);
            assert!(
                need.iter().any(|n| n == "neogeo.zip"),
                "{core}/{platform} should need neogeo.zip, got {need:?}"
            );
        }
    }

    /// Matched on the core as well as the system, because neither alone covers
    /// the manifest: some entries name only cores.
    #[test]
    fn a_core_only_entry_is_found_by_its_core() {
        let by_core = required_for("geolith", "something-else");
        assert!(!by_core.is_empty(), "geolith has required BIOS of its own");
    }

    /// A platform that needs nothing asks for nothing — otherwise every launch
    /// of every console would hit the network for a list it cannot use.
    #[test]
    fn a_platform_with_no_bios_requires_none() {
        assert!(required_for("snes9x", "snes").is_empty());
        assert!(required_for("fceumm", "nes").is_empty());
    }

    /// Only the required ones. Optional BIOS unlock extra behavior rather than
    /// making a game run, and fetching the lot on the way into a game is the
    /// opposite of what an automatic fetch should feel like.
    #[test]
    fn optional_bios_are_not_fetched_on_launch() {
        let all = manifest();
        let optional: Vec<&Entry> = all.iter().filter(|e| !e.required).collect();
        assert!(!optional.is_empty(), "the manifest has optional entries to exclude");

        for platform in ["arcade", "mame", "psx", "nes", "snes"] {
            let need = required_for("any", platform);
            for e in &optional {
                if e.systems.iter().any(|s| s == platform) {
                    assert!(
                        !need.contains(&e.path),
                        "{} is optional and must not be fetched on launch",
                        e.path
                    );
                }
            }
        }
    }
}
#[cfg(test)]
mod local_path_tests {
    use super::local_path;
    use std::path::Path;

    #[test]
    fn a_sub_path_is_reproduced() {
        let p = local_path(Path::new("/lib/system"), Some("mame"), "neogeo.zip").unwrap();
        assert_eq!(p, Path::new("/lib/system/mame/neogeo.zip"));
    }

    #[test]
    fn no_sub_path_lands_at_the_top() {
        let p = local_path(Path::new("/lib/system"), None, "scph1001.bin").unwrap();
        assert_eq!(p, Path::new("/lib/system/scph1001.bin"));
    }

    /// The real collision: 199 `config.ini`, one per MSX machine. Flattened they
    /// are one file; with the tree they are 199.
    #[test]
    fn same_name_under_two_directories_is_two_files() {
        let a = local_path(Path::new("/s"), Some("Machines/COL - ColecoVision"), "config.ini");
        let b = local_path(Path::new("/s"), Some("Machines/COL - Spectravideo SVI-603"), "config.ini");
        assert_ne!(a, b);
    }

    /// A server-supplied path is not trusted. None of these may escape `dest`.
    #[test]
    fn traversal_is_stripped_not_honoured() {
        for evil in ["../../.ssh", "..", "./..", "/etc", "a/../../b"] {
            let p = local_path(Path::new("/lib/system"), Some(evil), "x.bin").unwrap();
            assert!(
                p.starts_with("/lib/system"),
                "{evil} escaped to {}",
                p.display()
            );
            assert!(!p.to_string_lossy().contains(".."), "{evil} left a .. in {}", p.display());
        }
    }

    /// A name carrying a path contributes its leaf only.
    #[test]
    fn a_name_with_directories_in_it_keeps_only_the_leaf() {
        let p = local_path(Path::new("/s"), Some("mame"), "../../evil/neogeo.zip").unwrap();
        assert_eq!(p, Path::new("/s/mame/neogeo.zip"));
    }

    #[test]
    fn an_empty_name_is_no_path_at_all() {
        assert!(local_path(Path::new("/s"), None, "").is_none());
    }
}

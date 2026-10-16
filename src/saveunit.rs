//! A save unit: one folder an emulator treats as one save.
//!
//! The design is in `docs/emulator-saves.md`. This is the part the app and
//! moose-service share, so the two cannot disagree on what a unit's hash is or
//! which zips are acceptable: the naming rules, the hash, packing a folder into
//! a zip and putting one back over a folder.
//!
//! The hash is the one [`crate::savehash`] computes for a zip, md5 over the
//! sorted `name:md5(bytes)` lines, taken here over the folder's files. A folder
//! and the zip [`pack`] makes of it hash the same, so the server can hash what
//! it was sent without unpacking it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use md5::Digest as _;

/// The save formats with an adapter. A family is a format rather than an app:
/// PCSX2, ARMSX2 and NetherSX2 all read one another's folder cards.
pub const FAMILIES: &[&str] = &["pcsx2", "ppsspp"];

/// Units bigger than this are left alone unless a device raises it. PSP
/// install data lives in SAVEDATA beside the saves and looks like one; the
/// 845 MB of ULUS10509DAT is the case this exists for.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

pub fn valid_family(family: &str) -> bool {
    FAMILIES.contains(&family)
}

/// A key is the folder's own name: one path segment, nothing a Windows or SMB
/// file system would refuse, no leading dot (temporary folders here use one).
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && !key.starts_with('.')
        && !key.ends_with(' ')
        && !key.ends_with('.')
        && key
            .chars()
            .all(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// Files that are the OS's rather than the emulator's, and never travel:
/// dotfiles (`.DS_Store`, the `._` twins macOS leaves on exFAT cards) and
/// Windows' `Thumbs.db` and `desktop.ini`.
fn is_junk(name: &str) -> bool {
    name.starts_with('.') || name.eq_ignore_ascii_case("thumbs.db") || name.eq_ignore_ascii_case("desktop.ini")
}

/// What a unit looks like on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub hash: String,
    pub bytes: u64,
    pub files: usize,
    /// The newest file's mtime, as seconds since the epoch.
    pub modified: i64,
}

/// Every file in `dir` that travels, as (name relative to `dir` with `/`
/// separators, full path), sorted by name the way the hash sorts them.
/// Symlinks are skipped rather than followed.
pub fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
        for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if is_junk(&name) {
                continue;
            }
            let kind = e.file_type()?;
            let path = e.path();
            if kind.is_dir() {
                walk(base, &path, out)?;
            } else if kind.is_file() {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                let rel = rel.iter().map(|c| c.to_string_lossy()).collect::<Vec<_>>().join("/");
                out.push((rel, path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Hash, size and age of the folder, reading every file once.
pub fn summarize(dir: &Path) -> Result<Summary> {
    let mut lines = Vec::new();
    let mut bytes = 0;
    let mut modified = 0;
    let all = files(dir)?;
    for (rel, path) in &all {
        let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut h = md5::Md5::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            bytes += n as u64;
        }
        lines.push(format!("{rel}:{}", hex::encode(h.finalize())));
        let m = f.metadata()?.modified().ok();
        if let Some(secs) = m.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
            modified = modified.max(secs.as_secs() as i64);
        }
    }
    Ok(Summary {
        hash: hex::encode(md5::Md5::digest(lines.join("\n").as_bytes())),
        bytes,
        files: all.len(),
        modified,
    })
}

/// The folder as a zip, one entry per file in [`files`], so that [`check`] on
/// the result gives the folder's [`summarize`] hash.
pub fn pack(dir: &Path) -> Result<Vec<u8>> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(false);
    for (rel, path) in files(dir)? {
        w.start_file(rel.as_str(), opts)?;
        let mut f = std::fs::File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        std::io::copy(&mut f, &mut w)?;
    }
    Ok(w.finish()?.into_inner())
}

/// The zip's hash, once every entry has been checked to be a plain file with a
/// relative name [`unpack`] can write. Anything else is refused whole: a unit
/// is put back all or nothing.
pub fn check(zip_bytes: &[u8]) -> Result<String> {
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).context("not a zip")?;
    let mut any = false;
    for i in 0..a.len() {
        let e = a.by_index(i)?;
        if e.is_dir() {
            continue;
        }
        if e.is_symlink() || !e.is_file() {
            bail!("{} is not a plain file", e.name());
        }
        let Some(rel) = e.enclosed_name() else {
            bail!("{} would land outside the folder", e.name());
        };
        // `enclosed_name` makes `/etc/passwd` relative rather than refusing
        // it. Safe, but `pack` never writes such a name, so it is refused.
        let raw = e.name();
        if raw.starts_with('/') || raw.contains('\\') || raw.contains(':') || rel.iter().any(|c| is_junk(&c.to_string_lossy())) {
            bail!("{} is not a name a unit holds", e.name());
        }
        any = true;
    }
    if !any {
        bail!("the zip holds no files");
    }
    crate::savehash::manifest(&mut a)
}

/// Replace the folder `target` with the zip's contents.
///
/// The zip is unpacked beside `target` first, then swapped in by two renames,
/// so a failure at any point leaves the old folder or the new one, never part
/// of each. Backing up the old folder is the caller's job, before this.
pub fn unpack(zip_bytes: &[u8], target: &Path) -> Result<()> {
    check(zip_bytes)?;
    let parent = target.parent().context("a unit folder has a parent")?;
    let name = target.file_name().context("a unit folder has a name")?.to_string_lossy().into_owned();
    std::fs::create_dir_all(parent)?;
    let incoming = parent.join(format!(".{name}.incoming"));
    let replaced = parent.join(format!(".{name}.replaced"));
    for stale in [&incoming, &replaced] {
        if stale.exists() {
            std::fs::remove_dir_all(stale)?;
        }
    }

    let mut a = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))?;
    let written = (|| -> Result<()> {
        std::fs::create_dir_all(&incoming)?;
        for i in 0..a.len() {
            let mut e = a.by_index(i)?;
            if e.is_dir() {
                continue;
            }
            let rel = e.enclosed_name().context("checked above")?;
            let out = incoming.join(rel);
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut e, &mut f)?;
            f.flush()?;
        }
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(e.context(format!("unpacking into {}", incoming.display())));
    }

    let had_old = target.exists();
    if had_old {
        std::fs::rename(target, &replaced).with_context(|| format!("moving {} aside", target.display()))?;
    }
    if let Err(e) = std::fs::rename(&incoming, target) {
        if had_old {
            let _ = std::fs::rename(&replaced, target);
        }
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(anyhow::Error::from(e).context(format!("putting {} in place", target.display())));
    }
    if had_old {
        std::fs::remove_dir_all(&replaced)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("moose-unit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn psp_save(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("PARAM.SFO"), b"\0PSF sfo").unwrap();
        std::fs::write(dir.join("DATA.BIN"), vec![7u8; 4096]).unwrap();
        std::fs::write(dir.join(".DS_Store"), b"finder").unwrap();
    }

    #[test]
    fn a_folder_and_its_zip_hash_the_same() {
        let d = scratch("same");
        let save = d.join("UCUS98653DATA00");
        psp_save(&save);
        std::fs::create_dir_all(save.join("sub")).unwrap();
        std::fs::write(save.join("sub/inner.bin"), b"x").unwrap();
        let s = summarize(&save).unwrap();
        assert_eq!(s.files, 3, "the .DS_Store is not part of it");
        assert_eq!(s.bytes, 8 + 4096 + 1);
        let zip = pack(&save).unwrap();
        assert_eq!(check(&zip).unwrap(), s.hash);

        // And it is the hash savehash gives the same zip on disk, which is
        // what moose-service used to compute for a zipped save.
        let on_disk = d.join("unit.zip");
        std::fs::write(&on_disk, &zip).unwrap();
        assert_eq!(crate::savehash::compute(&on_disk).unwrap(), s.hash);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_changed_byte_changes_the_hash() {
        let d = scratch("changed");
        let save = d.join("UCUS98653DATA00");
        psp_save(&save);
        let before = summarize(&save).unwrap().hash;
        std::fs::write(save.join("DATA.BIN"), vec![8u8; 4096]).unwrap();
        assert_ne!(summarize(&save).unwrap().hash, before);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn unpacking_replaces_the_folder_whole() {
        let d = scratch("unpack");
        let from = d.join("from/BASLUS-20035");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("_pcsx2_index"), b"idx").unwrap();
        std::fs::write(from.join("BASLUS-20035"), b"new save").unwrap();
        let zip = pack(&from).unwrap();

        let target = d.join("card.ps2/BASLUS-20035");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("BASLUS-20035"), b"old save").unwrap();
        std::fs::write(target.join("stale-file"), b"gone after").unwrap();

        unpack(&zip, &target).unwrap();
        assert_eq!(std::fs::read(target.join("BASLUS-20035")).unwrap(), b"new save");
        assert!(!target.join("stale-file").exists(), "the old folder is replaced, not merged");
        assert_eq!(summarize(&target).unwrap().hash, summarize(&from).unwrap().hash);
        let left: Vec<_> = std::fs::read_dir(d.join("card.ps2")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("BASLUS-20035")], "no temporary folder is left");
        std::fs::remove_dir_all(&d).ok();
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn a_zip_that_would_write_outside_its_folder_is_refused() {
        for bad in ["../escape", "/etc/passwd", "a/../../b", ".hidden", "dir/.DS_Store", "C:/x", "a\\b"] {
            assert!(check(&zip_of(&[("ok", b"1"), (bad, b"2")])).is_err(), "{bad}");
        }
        assert!(check(&zip_of(&[])).is_err(), "an empty zip is not a save");
        assert!(check(b"not a zip").is_err());

        let d = scratch("refused");
        let target = d.join("UCUS98653DATA00");
        psp_save(&target);
        let before = summarize(&target).unwrap();
        assert!(unpack(&zip_of(&[("../escape", b"x")]), &target).is_err());
        assert_eq!(summarize(&target).unwrap(), before, "a refused zip leaves the folder alone");
        assert!(!d.join("escape").exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn keys_are_one_plain_segment() {
        for good in ["UCUS98653DATA00", "BASLUS-20035", "BASCUS-97465RATCHET", "BISLPM-65248", "ULUS10509DAT"] {
            assert!(valid_key(good), "{good}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", "C:", "x?", "trailing.", "trailing ", "nul\0"] {
            assert!(!valid_key(bad), "{bad:?}");
        }
        assert!(valid_family("pcsx2") && valid_family("ppsspp") && !valid_family("dolphin"));
    }
}

//! ROM download with resume and hash verification.
//!
//! Transfers run at wire speed (~115 MB/s measured, PLAN.md §3), so resume
//! exists for reliability rather than to rescue a slow transfer: a dropped
//! connection part-way through a 1.5 GB disc image should not start over.
//!
//! Bytes land in `<name>.part` and are only renamed into place once the hash
//! matches, so a partial file can never be mistaken for a complete ROM.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
// md5 and sha1 both implement `digest::Digest`, so importing the trait once
// brings `::digest()` into scope for both.
use md5::Digest as _;

/// Outcome of a download attempt.
#[derive(Debug)]
pub enum Outcome {
    /// Already present and verified; nothing transferred.
    AlreadyHave(PathBuf),
    Downloaded {
        path: PathBuf,
        bytes: u64,
        resumed_from: u64,
        verified: Verified,
    },
}

#[derive(Debug, PartialEq)]
pub enum Verified {
    Md5,
    Sha1,
    /// Server published no hash for this ROM — size was all we could check.
    SizeOnly,
    /// Folder ROM: `checked` of `total` unpacked files matched their own md5.
    /// The two differ when the server listed no hash for some member, which
    /// is worth saying rather than reporting a clean bill of health.
    Members { checked: usize, total: usize },
}

/// What we need to fetch and check one ROM.
pub struct Target<'a> {
    pub rom_id: i64,
    /// For folder ROMs: `(file_name, md5)` per member, used to verify each
    /// file after unpacking.
    pub members: &'a [(String, String)],
    pub fs_name: &'a str,
    pub platform_slug: &'a str,
    pub expected_size: Option<u64>,
    pub md5: Option<&'a str>,
    pub sha1: Option<&'a str>,
    /// Folder ROM (multi-disc). The server zips the folder for transfer, so
    /// the bytes on the wire are not the ROM and must be unpacked.
    pub multi_file: bool,
}

fn urlencode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn hash_file(path: &Path) -> Result<(String, String)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut md5 = md5::Md5::new();
    let mut sha1 = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        md5.update(&buf[..n]);
        sha1.update(&buf[..n]);
    }
    Ok((hex::encode(md5.finalize()), hex::encode(sha1.finalize())))
}

/// Fallback exclusion lists, used only until the server's are fetched.
///
/// These are RomM 5.0.0's defaults. They are configurable per deployment, so
/// the live values from `/api/config` take precedence — see [`set_exclusions`].
const FALLBACK_NAMES: &[&str] = &[
    ".DS_Store", ".localized", ".Trashes", ".stfolder", "@SynoResource",
    "gamelist.xml", "metadata.pegasus.txt",
];
const FALLBACK_EXTS: &[&str] =
    &["db", "ini", "tmp", "bak", "lock", "log", "cache", "crdownload"];

/// Exclusions in force for archive hashing.
static EXCLUSIONS: std::sync::OnceLock<(Vec<String>, Vec<String>)> = std::sync::OnceLock::new();

/// Adopt the server's exclusion lists. Call once, before verifying downloads.
///
/// Ignored if already set — the first caller (startup) wins, so a later refresh
/// cannot change hashing semantics mid-run.
pub fn set_exclusions(names: Vec<String>, exts: Vec<String>) {
    if names.is_empty() && exts.is_empty() {
        return;
    }
    let _ = EXCLUSIONS.set((names, exts));
}

fn exclusions() -> (Vec<String>, Vec<String>) {
    EXCLUSIONS.get().cloned().unwrap_or_else(|| {
        (
            FALLBACK_NAMES.iter().map(|s| s.to_string()).collect(),
            FALLBACK_EXTS.iter().map(|s| s.to_string()).collect(),
        )
    })
}

fn excluded(name: &str) -> bool {
    let (names, exts) = exclusions();
    let base = name.rsplit('/').next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    exts.iter().any(|e| lower.ends_with(&format!(".{e}")))
        || names.iter().any(|n| base == n)
}

/// Reproduce RomM's composite archive hash.
///
/// RomM does not hash the archive bytes, nor any single member: it streams the
/// decompressed contents of **every** eligible member, in ASCII order of the
/// internal path, through one running digest. From
/// `handler/filesystem/roms_handler.py`:
///
/// ```python
/// for name, size, chunks in ARCHIVE_READERS[rom_ext](...):
///     for chunk in chunks:
///         md5_h.update(chunk)     # one hash across all members
/// ```
///
/// That is why a single-member zip appears to match "the file inside" (with one
/// member the concatenation *is* that member) while multi-member romsets match
/// nothing simpler.
pub fn hash_archive_composite(path: &Path) -> Option<(String, String)> {
    let mut md5 = md5::Md5::new();
    let mut sha1 = sha1::Sha1::new();
    let mut any = false;

    if let Ok(file) = std::fs::File::open(path)
        && let Ok(mut zip) = zip::ZipArchive::new(std::io::BufReader::new(file))
    {
        let mut names: Vec<String> = Vec::new();
        for i in 0..zip.len() {
            if let Ok(e) = zip.by_index(i)
                && !e.is_dir()
            {
                names.push(e.name().to_owned());
            }
        }
        // ASCII order of the full internal path, as the server sorts.
        names.sort();
        for name in names {
            if excluded(&name) {
                continue;
            }
            let Ok(mut entry) = zip.by_name(&name) else { continue };
            let mut buf = Vec::new();
            if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                md5.update(&buf);
                sha1.update(&buf);
                any = true;
            }
        }
        return any.then(|| (hex::encode(md5.finalize()), hex::encode(sha1.finalize())));
    }

    let mut archive = sevenz_rust2::ArchiveReader::open(path, Default::default()).ok()?;
    let mut names: Vec<String> = archive
        .archive()
        .files
        .iter()
        .filter(|e| e.has_stream())
        .map(|e| e.name().to_owned())
        .collect();
    names.sort();
    for name in names {
        if excluded(&name) {
            continue;
        }
        if let Ok(buf) = archive.read_file(&name) {
            md5.update(&buf);
            sha1.update(&buf);
            any = true;
        }
    }
    any.then(|| (hex::encode(md5.finalize()), hex::encode(sha1.finalize())))
}

/// Per-member hashes, for diagnostics (`hashcheck`).
pub fn hash_archive_members(path: &Path) -> Vec<(String, String, String)> {
    let mut out = Vec::new();

    if let Ok(file) = std::fs::File::open(path)
        && let Ok(mut zip) = zip::ZipArchive::new(std::io::BufReader::new(file))
    {
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else { continue };
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_owned();
            let mut buf = Vec::new();
            if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                out.push((
                    name,
                    hex::encode(md5::Md5::digest(&buf)),
                    hex::encode(sha1::Sha1::digest(&buf)),
                ));
            }
        }
        return out;
    }

    // 7z: same idea, different container.
    if let Ok(mut archive) = sevenz_rust2::ArchiveReader::open(path, Default::default()) {
        let names: Vec<String> = archive
            .archive()
            .files
            .iter()
            .filter(|e| e.has_stream())
            .map(|e| e.name().to_owned())
            .collect();
        for name in names {
            if let Ok(buf) = archive.read_file(&name) {
                out.push((
                    name,
                    hex::encode(md5::Md5::digest(&buf)),
                    hex::encode(sha1::Sha1::digest(&buf)),
                ));
            }
        }
    }
    out
}

/// Compare against whichever hash the server published. Returns which one was
/// used, or an error describing the mismatch.
impl Verified {
    /// How the file was checked, in words, for whichever front-end is asking.
    pub fn describe(&self) -> String {
        match *self {
            Verified::Md5 => "md5 verified".to_owned(),
            Verified::Sha1 => "sha1 verified".to_owned(),
            Verified::SizeOnly => "size only — server published no hash".to_owned(),
            Verified::Members { checked, total } if checked == total => {
                format!("all {total} files md5-checked")
            }
            Verified::Members { checked, total } => format!(
                "{checked} of {total} files md5-checked — server listed no hash for the rest"
            ),
        }
    }
}

fn verify(path: &Path, target: &Target<'_>) -> Result<Verified> {
    let want_md5 = target.md5.filter(|h| !h.is_empty());
    let want_sha1 = target.sha1.filter(|h| !h.is_empty());
    if want_md5.is_none() && want_sha1.is_none() {
        return Ok(Verified::SizeOnly);
    }

    let (got_md5, got_sha1) = hash_file(path)?;
    let matches = |md5: &str, sha1: &str| match (want_md5, want_sha1) {
        (Some(w), _) if md5.eq_ignore_ascii_case(w) => Some(Verified::Md5),
        (None, Some(w)) if sha1.eq_ignore_ascii_case(w) => Some(Verified::Sha1),
        _ => None,
    };

    if let Some(v) = matches(&got_md5, &got_sha1) {
        return Ok(v);
    }
    // Archives: RomM hashes the concatenated contents of every member.
    if let Some((c_md5, c_sha1)) = hash_archive_composite(path)
        && let Some(v) = matches(&c_md5, &c_sha1)
    {
        return Ok(v);
    }
    let members = hash_archive_members(path);

    let want = want_md5.unwrap_or_else(|| want_sha1.unwrap_or(""));
    bail!(
        "hash mismatch: server {want}, downloaded {got_md5}{}",
        if members.is_empty() {
            String::new()
        } else {
            format!(" (composite of {} archive member(s) also checked)", members.len())
        }
    )
}

/// First file named `name` anywhere under `dir`.
fn find_by_name(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(hit) = find_by_name(&p, name) {
                return Some(hit);
            }
        } else if p.file_name().is_some_and(|f| f == name) {
            return Some(p);
        }
    }
    None
}

/// Verify each file of an unpacked folder ROM against its own hash.
///
/// The rom-level `md5_hash` is a composite hashed in `os.walk` order on the
/// server — filesystem-dependent, so no client can reproduce it. Per-member
/// hashes are exact, and identify *which* file is wrong rather than just that
/// something is.
///
/// Returns `(verified, files on disk)`, or the first mismatch.
pub fn verify_members(dir: &Path, members: &[(String, String)]) -> Result<(usize, usize)> {
    let mut checked = 0;
    for (name, want_md5) in members {
        if want_md5.is_empty() {
            continue;
        }
        // The server names members by leaf; a zip may nest them a level deep,
        // so fall back to a search before calling it missing.
        let direct = dir.join(name);
        let path = if direct.is_file() {
            direct
        } else {
            match find_by_name(dir, name) {
                Some(p) => p,
                None => bail!("missing after unpack: {name}"),
            }
        };
        let (got, _) = hash_file(&path)?;
        if got.eq_ignore_ascii_case(want_md5) {
            checked += 1;
            continue;
        }
        // A member may itself be an archive, and RomM hashes an archive's
        // contents rather than its bytes — the same rule as the single-file
        // path, applied one level down.
        if let Some((c_md5, _)) = hash_archive_composite(&path)
            && c_md5.eq_ignore_ascii_case(want_md5)
        {
            checked += 1;
            continue;
        }
        bail!("{name}: md5 mismatch (server {want_md5}, unpacked {got})");
    }
    let listed: std::collections::HashSet<&str> =
        members.iter().map(|(n, _)| n.as_str()).collect();
    Ok((checked, count_members(dir, &listed)))
}

/// Files under `dir` that ought to have been verified.
///
/// An `.m3u` the server never listed is one RomM synthesised into the zip for
/// multi-disc launching — it has no counterpart on disk there, so counting it
/// as unverified would report a shortfall that does not exist.
fn count_members(dir: &Path, listed: &std::collections::HashSet<&str>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                return count_members(&p, listed);
            }
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let generated = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("m3u"))
                && !listed.contains(name.as_str());
            usize::from(!generated)
        })
        .sum()
}

/// Extract a downloaded folder-ROM zip into `dest`, flattening any wrapper
/// directory the archive carries.
fn unpack_folder_rom(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .with_context(|| format!("reading {}", zip_path.display()))?;

    // A folder ROM downloaded before this function existed is sitting there as
    // the raw zip, under the folder's own name and with no extension. That is a
    // *file* where a directory has to go, so removing only directories left it
    // in place and `create_dir_all` then failed with "File exists" — the one
    // copy that most needed replacing was the one that could not be.
    if dest.is_dir() {
        std::fs::remove_dir_all(dest).ok();
    } else if dest.exists() {
        std::fs::remove_file(dest).ok();
    }
    std::fs::create_dir_all(dest)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        // Keep only the leaf: entries may be prefixed with the folder name,
        // and we already have a directory for it.
        let name = Path::new(entry.name())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let out = dest.join(&name);
        let mut w = std::fs::File::create(&out)
            .with_context(|| format!("writing {}", out.display()))?;
        std::io::copy(&mut entry, &mut w)?;
    }
    Ok(())
}

/// Download `target` into `<library_roms>/<platform>/<fs_name>`.
///
/// `auth` is a complete `Authorization` header value, so this works with a
/// bearer token or Basic without knowing which.
///
/// `progress` is called with `(downloaded_so_far, total_or_0)`.
pub async fn fetch(
    http: &reqwest::Client,
    base_url: &str,
    auth: &str,
    target: &Target<'_>,
    library_roms: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<Outcome> {
    // A multi-disc playlist whose discs were never indexed is a stub on this
    // server; downloading it yields a file that cannot launch. See PLAN.md §3.
    if target.fs_name.ends_with(".m3u")
        && target.expected_size.is_some_and(|s| s < 4096)
    {
        bail!(
            "{} is a {}-byte playlist stub — its disc images were never indexed by RomM, \
             so there is nothing to download. Fix server-side with convert-to-folder.",
            target.fs_name,
            target.expected_size.unwrap_or(0)
        );
    }

    let dir = library_roms.join(target.platform_slug);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let final_path = dir.join(target.fs_name);
    let part_path = dir.join(format!("{}.part", target.fs_name));

    if target.multi_file {
        // Per-member hashes make this a real check rather than a presence
        // test: a folder with a corrupted file refetches instead of being
        // reported as verified.
        if final_path.is_dir()
            && std::fs::read_dir(&final_path).map(|d| d.count() > 0).unwrap_or(false)
            && verify_members(&final_path, target.members).is_ok()
        {
            return Ok(Outcome::AlreadyHave(final_path));
        }
    } else if final_path.is_file() {
        // Trust it only if it still matches; otherwise fall through and refetch.
        if verify(&final_path, target).is_ok() {
            return Ok(Outcome::AlreadyHave(final_path));
        }
        std::fs::remove_file(&final_path).ok();
    }

    let resume_from = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
    let url = format!(
        "{}/api/roms/{}/content/{}",
        base_url.trim_end_matches('/'),
        target.rom_id,
        urlencode_path(target.fs_name)
    );

    let mut req = http
        .get(&url)
        .header("Authorization", auth);
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={resume_from}-"));
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GET {url} -> {status}");
    }

    // 206 means the server honoured Range; 200 means it ignored it and is
    // sending the whole file, so any partial data must be discarded.
    let appending = status == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0;
    let started_at = if appending { resume_from } else { 0 };

    let total = target
        .expected_size
        .unwrap_or_else(|| resp.content_length().unwrap_or(0) + started_at);

    let mut file = if appending {
        std::fs::OpenOptions::new().append(true).open(&part_path)?
    } else {
        std::fs::File::create(&part_path)?
    };

    let mut written = started_at;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading response body")?;
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
        progress(written, total);
    }
    file.flush()?;
    drop(file);

    // A folder ROM is transferred as a zip of its contents, so the bytes on
    // the wire never equal `fs_size_bytes` (which is the sum of the files).
    // Zip overhead alone makes them differ by a few hundred bytes.
    if let Some(expected) = target.expected_size
        && !target.multi_file
        && written != expected
    {
        bail!(
            "size mismatch: expected {expected} bytes, got {written} \
             (partial left at {})",
            part_path.display()
        );
    }

    let verified = if target.multi_file {
        // The server hashed the folder's contents; we hold a zip of them.
        Verified::SizeOnly
    } else {
        match verify(&part_path, target) {
            Ok(v) => v,
            Err(e) => {
                // Keep the bad file for inspection rather than silently deleting.
                bail!("{e}\n  bad download left at {}", part_path.display());
            }
        }
    };

    // Folder ROMs arrive as a zip of the directory. Unpack it so the layout
    // matches the server's and the .m3u inside points at real neighbours.
    if target.multi_file {
        unpack_folder_rom(&part_path, &final_path)?;
        std::fs::remove_file(&part_path).ok();
        // Now the files exist individually, each can be checked properly.
        let verified = match verify_members(&final_path, target.members) {
            Ok((0, _)) => Verified::SizeOnly,
            Ok((checked, total)) => Verified::Members { checked, total },
            Err(e) => {
                bail!("{e}\n  unpacked files left at {}", final_path.display());
            }
        };
        return Ok(Outcome::Downloaded {
            path: final_path,
            bytes: written,
            resumed_from: started_at,
            verified,
        });
    }

    std::fs::rename(&part_path, &final_path)
        .with_context(|| format!("renaming into {}", final_path.display()))?;

    Ok(Outcome::Downloaded {
        path: final_path,
        bytes: written,
        resumed_from: started_at,
        verified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-download-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("creating a temp dir");
        dir
    }

    fn write(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, body).unwrap();
        p
    }

    /// A zip whose members are written in the given order, so a test can prove
    /// the hash is taken in *name* order rather than storage order.
    fn zip_of(path: &Path, members: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = Default::default();
        for (name, body) in members {
            zip.start_file(*name, opts).unwrap();
            std::io::Write::write_all(&mut zip, body).unwrap();
        }
        zip.finish().unwrap();
    }

    fn md5_of(bytes: &[u8]) -> String {
        hex::encode(md5::Md5::digest(bytes))
    }

    /// The exclusion rule decides which bytes feed the archive hash, so getting
    /// it wrong does not error — it silently computes a hash that will never
    /// match the server's, for every archive containing such a file.
    #[test]
    fn exclusions_match_on_the_basename_and_are_case_insensitive_on_extension() {
        // Nothing about an ordinary ROM is excluded.
        assert!(!excluded("Sonic.md"));
        assert!(!excluded("roms/Sonic.md"));

        // Names match exactly, anywhere in the tree.
        assert!(excluded(".DS_Store"));
        assert!(excluded("some/nested/dir/.DS_Store"));
        assert!(excluded("gamelist.xml"));

        // ...but only exactly. A name that merely ends with an excluded one is
        // a real file and its bytes belong in the hash.
        assert!(!excluded("mygamelist.xml"));

        // Extensions are compared lowercased, since archives carry both forms.
        assert!(excluded("Thumbs.DB"));
        assert!(excluded("notes.log"));
        assert!(excluded("NOTES.LOG"));
        assert!(!excluded("save.srm"));
    }

    /// Setting the exclusions to nothing must not wipe the fallbacks — an empty
    /// `/api/config` response would otherwise change how every archive hashes.
    #[test]
    fn empty_server_exclusions_are_ignored() {
        set_exclusions(Vec::new(), Vec::new());
        assert!(excluded(".DS_Store"), "the fallback list must survive");
    }

    /// RomM streams every eligible member through ONE digest, in ASCII order of
    /// the internal path. With a single member the concatenation is that member,
    /// which is why a one-file zip looks like it matches "the file inside".
    #[test]
    fn a_single_member_archive_hashes_as_its_contents() {
        let dir = scratch("composite-one");
        let zip_path = dir.join("one.zip");
        zip_of(&zip_path, &[("game.md", b"abcdef")]);

        let (md5, _) = hash_archive_composite(&zip_path).expect("a zip we just wrote");
        assert_eq!(md5, md5_of(b"abcdef"));
    }

    /// The ordering rule, which is the part that cannot be guessed: members are
    /// concatenated by sorted name, not by the order they sit in the archive.
    #[test]
    fn members_hash_in_name_order_not_storage_order() {
        let dir = scratch("composite-order");
        let zip_path = dir.join("set.zip");
        // Deliberately stored z-then-a.
        zip_of(&zip_path, &[("z.bin", b"ZZZ"), ("a.bin", b"AAA")]);

        let (md5, _) = hash_archive_composite(&zip_path).unwrap();
        assert_eq!(md5, md5_of(b"AAAZZZ"), "sorted by name: a.bin then z.bin");
        assert_ne!(md5, md5_of(b"ZZZAAA"), "not the order they were written in");
    }

    /// An excluded member contributes nothing, so an archive with junk in it
    /// hashes identically to the same archive without.
    #[test]
    fn excluded_members_do_not_reach_the_digest() {
        let dir = scratch("composite-excluded");
        let clean = dir.join("clean.zip");
        let dirty = dir.join("dirty.zip");
        zip_of(&clean, &[("game.md", b"payload")]);
        zip_of(&dirty, &[("game.md", b"payload"), (".DS_Store", b"junk")]);

        assert_eq!(
            hash_archive_composite(&clean).unwrap().0,
            hash_archive_composite(&dirty).unwrap().0
        );
    }

    /// Per-member verification is what makes a folder ROM checkable at all, and
    /// it must name the file that is wrong rather than just failing.
    #[test]
    fn member_verification_names_the_file_that_does_not_match() {
        let dir = scratch("members-bad");
        write(&dir, "disc1.chd", b"one");
        write(&dir, "disc2.chd", b"two");

        let members = vec![
            ("disc1.chd".to_owned(), md5_of(b"one")),
            ("disc2.chd".to_owned(), md5_of(b"WRONG")),
        ];
        let err = verify_members(&dir, &members).expect_err("disc2 does not match").to_string();
        assert!(err.contains("disc2.chd"), "must say which file: {err}");
        assert!(!err.contains("disc1.chd"), "and not blame the good one: {err}");
    }

    /// The server names members by leaf, but a zip may nest them a level deep.
    /// Falling back to a search is what stops that being reported as missing.
    #[test]
    fn a_nested_member_is_found_rather_than_called_missing() {
        let dir = scratch("members-nested");
        write(&dir, "Shenmue/disc1.chd", b"one");

        let members = vec![("disc1.chd".to_owned(), md5_of(b"one"))];
        let (checked, _) = verify_members(&dir, &members).expect("found one level down");
        assert_eq!(checked, 1);
    }

    /// A member the server listed no hash for is skipped, not treated as a
    /// failure — `checked` and `total` differing is what reports that honestly.
    #[test]
    fn members_without_a_published_hash_are_skipped_not_failed() {
        let dir = scratch("members-nohash");
        write(&dir, "a.bin", b"a");
        write(&dir, "b.bin", b"b");

        let members = vec![
            ("a.bin".to_owned(), md5_of(b"a")),
            ("b.bin".to_owned(), String::new()),
        ];
        let (checked, total) = verify_members(&dir, &members).unwrap();
        assert_eq!((checked, total), (1, 2));
        assert!(
            Verified::Members { checked, total }.describe().contains("no hash for the rest"),
            "the shortfall has to be visible, not rounded up to a clean bill of health"
        );
    }

    /// RomM synthesises an .m3u into the zip for multi-disc launching that has
    /// no counterpart on the server. Counting it would report a shortfall that
    /// does not exist, and make every folder ROM look partly unverified.
    #[test]
    fn a_generated_playlist_is_not_counted_as_an_unverified_member() {
        let dir = scratch("members-m3u");
        write(&dir, "disc1.chd", b"one");
        write(&dir, "Game.m3u", b"disc1.chd\n");

        let members = vec![("disc1.chd".to_owned(), md5_of(b"one"))];
        let (checked, total) = verify_members(&dir, &members).unwrap();
        assert_eq!(
            (checked, total),
            (1, 1),
            "the .m3u the server never listed is generated, not missing"
        );
        assert_eq!(
            Verified::Members { checked, total }.describe(),
            "all 1 files md5-checked"
        );

        // An .m3u the server DID list is a real file and does count.
        let listed = std::collections::HashSet::from(["Game.m3u"]);
        assert_eq!(count_members(&dir, &listed), 2);
    }

    /// Filenames reach the server as a URL path segment. A raw space or `#`
    /// truncates or misroutes the request, and this library is full of both.
    #[test]
    fn filenames_are_percent_encoded_for_the_content_url() {
        assert_eq!(
            urlencode_path("Final Fantasy VII (USA) (Disc 1).chd"),
            "Final%20Fantasy%20VII%20%28USA%29%20%28Disc%201%29.chd"
        );
        assert_eq!(urlencode_path("Blow'em Out!.zip"), "Blow%27em%20Out%21.zip");
        // Unreserved characters must pass through untouched.
        assert_eq!(urlencode_path("a-b_c.d~e"), "a-b_c.d~e");
    }

    /// The words shown after a download are the user's only evidence of what
    /// was actually checked, so "verified" must never overstate.
    #[test]
    fn describe_does_not_overstate_what_was_checked() {
        assert_eq!(Verified::Md5.describe(), "md5 verified");
        assert_eq!(
            Verified::SizeOnly.describe(),
            "size only — server published no hash"
        );
        assert_eq!(
            Verified::Members { checked: 3, total: 3 }.describe(),
            "all 3 files md5-checked"
        );
        assert!(Verified::Members { checked: 2, total: 3 }
            .describe()
            .contains("2 of 3"));
    }
}

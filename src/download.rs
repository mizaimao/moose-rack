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
    /// The directory under the ROMs folder, from `RomRow::folder`.
    pub folder: &'a Path,
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
/// Feed everything `r` yields into both digests, a chunk at a time.
///
/// Archive members were read whole into memory first, which for a zipped disc
/// image is a buffer the size of the disc on a handheld with 2 GB of RAM.
fn digest_into(r: &mut impl std::io::Read, md5: &mut md5::Md5, sha1: &mut sha1::Sha1) -> std::io::Result<()> {
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        md5.update(&buf[..n]);
        sha1.update(&buf[..n]);
    }
}

/// How many files an archive holds, from its index alone.
fn archive_member_count(path: &Path) -> usize {
    if let Ok(file) = std::fs::File::open(path)
        && let Ok(mut zip) = zip::ZipArchive::new(std::io::BufReader::new(file))
    {
        return (0..zip.len()).filter(|&i| zip.by_index(i).is_ok_and(|e| !e.is_dir())).count();
    }
    sevenz_rust2::ArchiveReader::open(path, Default::default())
        .map(|a| a.archive().files.iter().filter(|e| e.has_stream()).count())
        .unwrap_or(0)
}

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
            // A member that cannot be read leaves a digest of part of it,
            // which matches nothing; say there is no answer instead.
            digest_into(&mut entry, &mut md5, &mut sha1).ok()?;
            any = true;
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
            let (mut md5, mut sha1) = (md5::Md5::new(), sha1::Sha1::new());
            if digest_into(&mut entry, &mut md5, &mut sha1).is_ok() {
                out.push((name, hex::encode(md5.finalize()), hex::encode(sha1.finalize())));
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
        // "Size only" has to mean the size was looked at. It was not, so any
        // file already at the path, an empty one or an older revision, was
        // reported as here and verified.
        let len = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?.len();
        if len == 0 {
            bail!("{} is empty", path.display());
        }
        if let Some(want) = target.expected_size
            && len != want
        {
            bail!("size mismatch: server {want} bytes, here {len}");
        }
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
    // Counted from the index: reading every member a second time only to
    // print how many there were doubled the cost of a mismatch.
    let members = archive_member_count(path);

    let want = want_md5.unwrap_or_else(|| want_sha1.unwrap_or(""));
    bail!(
        "hash mismatch: server {want}, downloaded {got_md5}{}",
        if members == 0 {
            String::new()
        } else {
            format!(" (composite of {members} archive member(s) also checked)")
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
///
/// `dest` is a fresh folder beside the real one; [`swap_in`] puts it in place
/// once its files have checked out, so a bad or interrupted download never
/// costs the copy that was already there.
fn unpack_folder_rom(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .with_context(|| format!("reading {}", zip_path.display()))?;

    if dest.exists() {
        std::fs::remove_dir_all(dest)
            .with_context(|| format!("clearing {}", dest.display()))?;
    }
    std::fs::create_dir_all(dest)?;

    // The paths inside, kept. Only a wrapper folder every file sits in is
    // dropped, since `dest` is that folder already. Keeping just each file's
    // name flattened a PS3 or Wii U game's PS3_GAME/USRDIR/... tree, and two
    // files of one name in different folders overwrote each other.
    let mut files: Vec<(usize, PathBuf)> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else {
            bail!("{} would land outside the folder", entry.name());
        };
        files.push((i, rel));
    }
    let wrapper = files.first().and_then(|(_, p)| {
        let first = p.components().next()?;
        let shared = files
            .iter()
            .all(|(_, p)| p.components().count() > 1 && p.components().next() == Some(first));
        shared.then(|| PathBuf::from(first.as_os_str()))
    });
    for (i, rel) in files {
        let rel = match &wrapper {
            Some(w) => rel.strip_prefix(w).map(Path::to_path_buf).unwrap_or(rel),
            None => rel,
        };
        let out = dest.join(&rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let mut entry = zip.by_index(i)?;
        let mut w = std::fs::File::create(&out)
            .with_context(|| format!("writing {}", out.display()))?;
        std::io::copy(&mut entry, &mut w)?;
    }
    Ok(())
}

/// Put the unpacked folder `incoming` where `dest` is, by renames.
///
/// The old copy is moved aside first and only removed once the new one is in
/// place. A folder ROM downloaded before folders were unpacked is sitting at
/// `dest` as the raw zip, a *file*, and is replaced the same way.
fn swap_in(incoming: &Path, dest: &Path) -> Result<()> {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let replaced = dest.with_file_name(format!(".{name}.replaced"));
    if replaced.is_dir() {
        std::fs::remove_dir_all(&replaced).ok();
    } else if replaced.exists() {
        std::fs::remove_file(&replaced).ok();
    }
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(dest, &replaced)
            .with_context(|| format!("moving {} aside", dest.display()))?;
    }
    if let Err(e) = std::fs::rename(incoming, dest) {
        if had_old {
            let _ = std::fs::rename(&replaced, dest);
        }
        return Err(e).with_context(|| format!("moving the download into {}", dest.display()));
    }
    if replaced.is_dir() {
        std::fs::remove_dir_all(&replaced).ok();
    } else if replaced.exists() {
        std::fs::remove_file(&replaced).ok();
    }
    Ok(())
}

/// A name from the server that is one plain file or folder name: no `/`,
/// no `..`, no drive or root, as this machine's paths read it. Anything else
/// would be written outside the folder it was meant for.
fn plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none()
}

/// A folder under the ROMs folder made only of plain names.
fn plain_folder(folder: &Path) -> bool {
    folder.components().next().is_some()
        && folder.components().all(|c| match c {
            std::path::Component::Normal(n) => plain_name(&n.to_string_lossy()),
            _ => false,
        })
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

    if !plain_name(target.fs_name) || !plain_folder(target.folder) {
        bail!(
            "refusing to download {:?} into {:?}: not a plain file and folder name",
            target.fs_name,
            target.folder
        );
    }
    let dir = library_roms.join(target.folder);
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
    } else if final_path.is_file() && verify(&final_path, target).is_ok() {
        // Trust it only if it still matches; otherwise refetch. The copy that
        // does not match stays where it is until the new one has been checked:
        // the rename at the end replaces it, and a download that fails leaves
        // it alone rather than leaving nothing.
        return Ok(Outcome::AlreadyHave(final_path));
    }

    let mut resume_from = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
    // A `.part` as long as the whole file, or longer, is one that arrived and
    // then failed its size or hash check. Resuming it asks for bytes past the
    // end, which the server answers with 416, and every later try did the
    // same. It starts again instead.
    if resume_from > 0 && target.expected_size.is_some_and(|size| resume_from >= size) {
        std::fs::remove_file(&part_path).ok();
        resume_from = 0;
    }
    let url = format!(
        "{}/api/roms/{}/content/{}",
        base_url.trim_end_matches('/'),
        target.rom_id,
        urlencode_path(target.fs_name)
    );

    let get = |from: u64| {
        let mut req = http.get(&url).header("Authorization", auth);
        if from > 0 {
            req = req.header("Range", format!("bytes={from}-"));
        }
        req.send()
    };
    let mut resp = get(resume_from).await.with_context(|| format!("GET {url}"))?;
    // The same, when the size was not known up front: the server says the
    // range is past the end, so the partial file is no use.
    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && resume_from > 0 {
        std::fs::remove_file(&part_path).ok();
        resume_from = 0;
        resp = get(0).await.with_context(|| format!("GET {url}"))?;
    }
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
        let incoming = dir.join(format!(".{}.incoming", target.fs_name));
        unpack_folder_rom(&part_path, &incoming)?;
        std::fs::remove_file(&part_path).ok();
        // Now the files exist individually, each can be checked properly,
        // before anything already here is touched.
        let verified = match verify_members(&incoming, target.members) {
            Ok((0, _)) => Verified::SizeOnly,
            Ok((checked, total)) => Verified::Members { checked, total },
            Err(e) => {
                bail!("{e}\n  unpacked files left at {}", incoming.display());
            }
        };
        swap_in(&incoming, &final_path)?;
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

    /// A server that answers one request with `status` and `body`, and hands
    /// back the request it got.
    fn serve_once(status: &'static str, body: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(head.as_bytes()).unwrap();
            sock.write_all(&body).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (base, handle)
    }

    fn target<'a>(fs_name: &'a str, folder: &'a Path, size: Option<u64>, md5: Option<&'a str>) -> Target<'a> {
        Target {
            rom_id: 7,
            members: &[],
            fs_name,
            folder,
            expected_size: size,
            md5,
            sha1: None,
            multi_file: false,
        }
    }

    /// A local copy that does not match the server's is replaced only by a
    /// download that arrived and checked out. It used to be deleted before the
    /// request went out, so a failed download left no copy at all.
    #[tokio::test]
    async fn a_copy_that_does_not_match_survives_a_failed_download() {
        let roms = scratch("survives");
        let folder = PathBuf::from("snes");
        let mine = write(&roms.join("snes"), "Game.sfc", b"my own copy");
        let (base, server) = serve_once("500 Internal Server Error", b"no".to_vec());
        crate::util::install_tls();
        let wanted = md5_of(b"the server's copy");
        let t = target("Game.sfc", &folder, Some(17), Some(&wanted));
        assert!(fetch(&reqwest::Client::new(), &base, "", &t, &roms, |_, _| {}).await.is_err());
        server.join().unwrap();
        assert_eq!(std::fs::read(&mine).unwrap(), b"my own copy");
    }

    /// A `.part` as long as the whole file is a download that arrived and then
    /// failed its check. Resuming it asks for bytes past the end, which the
    /// server refuses with 416, so every later try failed the same way.
    #[tokio::test]
    async fn a_full_length_part_starts_over_instead_of_asking_past_the_end() {
        let roms = scratch("full-part");
        let folder = PathBuf::from("snes");
        let body = b"sixteen bytes!!!".to_vec();
        write(&roms.join("snes"), "Game.sfc.part", b"sixteen bad byte");
        let (base, server) = serve_once("200 OK", body.clone());
        crate::util::install_tls();
        let t = target("Game.sfc", &folder, Some(body.len() as u64), None);
        fetch(&reqwest::Client::new(), &base, "", &t, &roms, |_, _| {})
            .await
            .expect("the download starts over and completes");
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(!request.contains("range:"), "asked for a range again: {request}");
        assert_eq!(std::fs::read(roms.join("snes/Game.sfc")).unwrap(), body);
        assert!(!roms.join("snes/Game.sfc.part").exists());
    }

    /// Names come from the server and are joined onto the ROMs folder, so one
    /// with a folder in it would write, and on a mismatch replace, a file
    /// somewhere else.
    #[tokio::test]
    async fn a_name_that_leaves_the_folder_is_refused() {
        let roms = scratch("escape");
        let snes = PathBuf::from("snes");
        let up = PathBuf::from("../elsewhere");
        for t in [
            target("../Game.sfc", &snes, None, None),
            target("a/Game.sfc", &snes, None, None),
            target("..", &snes, None, None),
            target("Game.sfc", &up, None, None),
        ] {
            // Nothing listens on port 9: a request at all would be an error of
            // a different kind, and the message says which.
            let e = fetch(&reqwest::Client::new(), "http://127.0.0.1:9", "", &t, &roms, |_, _| {})
                .await
                .unwrap_err()
                .to_string();
            assert!(e.contains("refusing"), "{e}");
        }
    }

    /// With no hash published, "already here" still has to be the right size:
    /// an empty file or another revision at the path was counted as verified.
    #[test]
    fn with_no_hash_the_size_still_has_to_match() {
        let dir = scratch("size-only");
        let folder = PathBuf::from("snes");
        let ok = write(&dir, "Game.sfc", b"twelve bytes");
        assert!(matches!(verify(&ok, &target("Game.sfc", &folder, Some(12), None)), Ok(Verified::SizeOnly)));
        assert!(verify(&ok, &target("Game.sfc", &folder, Some(13), None)).is_err());
        let empty = write(&dir, "Empty.sfc", b"");
        assert!(verify(&empty, &target("Empty.sfc", &folder, None, None)).is_err());
    }

    /// A folder game keeps its own folders; only a wrapper around everything
    /// goes, because the destination is that folder already.
    #[test]
    fn a_folder_rom_keeps_its_inner_folders() {
        let dir = scratch("nested");
        let zip_path = dir.join("game.zip");
        zip_of(&zip_path, &[
            ("Game/PS3_GAME/USRDIR/EBOOT.BIN", b"a"),
            ("Game/PS3_GAME/PARAM.SFO", b"b"),
            ("Game/PS3_DISC.SFB", b"c"),
        ]);
        let dest = dir.join("out");
        unpack_folder_rom(&zip_path, &dest).unwrap();
        assert!(dest.join("PS3_GAME/USRDIR/EBOOT.BIN").is_file());
        assert!(dest.join("PS3_GAME/PARAM.SFO").is_file());
        assert!(dest.join("PS3_DISC.SFB").is_file());

        // No shared wrapper: kept as it is.
        let zip_path = dir.join("flat.zip");
        zip_of(&zip_path, &[("disc1/track.bin", b"1"), ("disc2/track.bin", b"2"), ("game.m3u", b"m")]);
        let dest = dir.join("flat");
        unpack_folder_rom(&zip_path, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("disc1/track.bin")).unwrap(), b"1");
        assert_eq!(std::fs::read(dest.join("disc2/track.bin")).unwrap(), b"2");
    }

    /// The unpacked folder replaces the old one only once it is complete, and
    /// a raw zip left at the folder's name by an old build is replaced too.
    #[test]
    fn an_unpacked_folder_is_swapped_in_whole() {
        let dir = scratch("swap");
        let dest = dir.join("Game");
        write(&dest, "old.bin", b"old");
        let incoming = dir.join(".Game.incoming");
        write(&incoming, "new.bin", b"new");
        swap_in(&incoming, &dest).unwrap();
        assert!(dest.join("new.bin").is_file());
        assert!(!dest.join("old.bin").exists());
        assert!(!incoming.exists());
        assert!(!dir.join(".Game.replaced").exists());

        let raw = write(&dir, "Disc", b"a zip from before folders were unpacked");
        let incoming = dir.join(".Disc.incoming");
        write(&incoming, "disc.cue", b"cue");
        swap_in(&incoming, &raw).unwrap();
        assert!(raw.join("disc.cue").is_file());
    }
}

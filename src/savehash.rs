//! Byte-exact port of RomM 5.0.0 `assets_handler.compute_content_hash`.
//!
//! This is the single most consequential function in the sync path. The server
//! compares this hash to decide `no_op`; if ours disagrees by even one byte,
//! `no_op` never fires and the entire save set re-uploads on every run,
//! forever. RomM itself shipped that bug — there is a recovery task in their
//! repo for it.
//!
//! Reference implementation (Python):
//!
//! ```python
//! def compute_content_hash(path):
//!     if zipfile.is_zipfile(path):
//!         with zipfile.ZipFile(path, "r") as zf:
//!             parts = []
//!             for name in sorted(zf.namelist()):      # sorted() is load-bearing
//!                 if not name.endswith("/"):          # skip directory entries
//!                     parts.append(f"{name}:{hashlib.md5(zf.read(name)).hexdigest()}")
//!             return hashlib.md5("\n".join(parts).encode()).hexdigest()
//!     h = hashlib.md5()
//!     with open(path, "rb") as f:
//!         while chunk := f.read(8192):
//!             h.update(chunk)
//!     return h.hexdigest()
//! ```
//!
//! Notes on fidelity:
//! * Python sorts by Unicode code point; Rust's `str` ordering is by UTF-8
//!   bytes, which yields the same order because UTF-8 preserves code-point
//!   ordering. Equivalent.
//! * The 8192-byte chunking does not affect the digest; we stream in 1 MiB.
//! * `usedforsecurity=False` on the server is a policy flag only.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use md5::Digest as _;

/// Whether the file parses as a zip archive.
///
/// Python's `zipfile.is_zipfile` scans for the End of Central Directory
/// record; the `zip` crate's reader does the same thing, so opening
/// successfully is the equivalent test.
fn is_zip(path: &Path) -> bool {
    std::fs::File::open(path)
        .ok()
        .is_some_and(|f| zip::ZipArchive::new(std::io::BufReader::new(f)).is_ok())
}

fn md5_hex(bytes: &[u8]) -> String {
    hex::encode(md5::Md5::digest(bytes))
}

/// Content hash as RomM computes it. Zip files hash their manifest; everything
/// else is a plain MD5 of the raw bytes.
pub fn compute(path: &Path) -> Result<String> {
    if is_zip(path) {
        return zip_manifest_hash(path);
    }
    let mut f =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = md5::Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn zip_manifest_hash(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .with_context(|| format!("reading zip {}", path.display()))?;

    // Collect names first: sorting must happen over the whole namelist before
    // any entry is read, exactly as `sorted(zf.namelist())` does.
    let mut names: Vec<String> = (0..archive.len())
        .map(|i| Ok(archive.by_index(i)?.name().to_owned()))
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    names.sort();

    let mut parts: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        if name.ends_with('/') {
            continue; // directory entry
        }
        let mut entry = archive.by_name(&name)?;
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        parts.push(format!("{name}:{}", md5_hex(&bytes)));
    }

    Ok(md5_hex(parts.join("\n").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn plain_file_is_raw_md5() {
        let dir = std::env::temp_dir().join("moose-rack-hash-test-plain");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.srm");
        std::fs::write(&p, b"hello world").unwrap();
        // md5("hello world")
        assert_eq!(compute(&p).unwrap(), "5eb63bbbe01eeed093cb22bb8f5acdc3");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zip_hashes_manifest_not_bytes() {
        let dir = std::env::temp_dir().join("moose-rack-hash-test-zip");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.zip");
        {
            let f = std::fs::File::create(&p).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            // Deliberately written out of order, to prove sorting happens.
            w.start_file("b.txt", opts).unwrap();
            w.write_all(b"bbb").unwrap();
            w.start_file("a.txt", opts).unwrap();
            w.write_all(b"aaa").unwrap();
            w.finish().unwrap();
        }
        let expect = md5_hex(
            format!("a.txt:{}\nb.txt:{}", md5_hex(b"aaa"), md5_hex(b"bbb")).as_bytes(),
        );
        assert_eq!(compute(&p).unwrap(), expect);
        std::fs::remove_dir_all(&dir).ok();
    }
}

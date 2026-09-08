//! Where the vendored EmulatorJS lives, and what a webview may ask it for.
//!
//! The web UI gets EmulatorJS from the service, which serves `assets/emulatorjs`
//! over `/emulatorjs/`. The desktop window has no HTTP server behind it, so it
//! reads the same directory off disk through a custom URI scheme. Both need the
//! same two answers -- where the directory is, and whether a requested path is
//! inside it -- so both come from here.
//!
//! The directory is 296 MB of vendored WebAssembly and is deliberately not in
//! git (`scripts/fetch-emulatorjs.sh` puts it there). A build without it is a
//! normal state, not a broken one: the desktop simply does not offer to play in
//! the window, and says why.

use std::path::{Path, PathBuf};

/// The one file that proves a directory really is an EmulatorJS unpack.
const MARKER: &str = "data/loader.js";

/// The first candidate that holds EmulatorJS, or `None`.
///
/// Order is caller's order. Nothing here searches upwards or guesses: a wrong
/// directory that happens to exist is worse than none, because the emulator
/// would fail later with a 404 for a core instead of up front with "not
/// installed".
pub fn locate<'a>(candidates: impl IntoIterator<Item = &'a Path>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .find(|dir| dir.join(MARKER).is_file())
        .map(Path::to_path_buf)
}

/// Where a desktop build should look, in order.
///
/// `exe` is the running binary. Inside a macOS bundle that is
/// `Moose Rack.app/Contents/MacOS/moose-gui`, so `../Resources/emulatorjs` is
/// where a bundled copy would land; outside one it is `target/release`, and the
/// working directory is the repo.
pub fn desktop_candidates(exe: Option<&Path>, cwd: &Path, config: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = exe.and_then(Path::parent) {
        out.push(dir.join("../Resources/emulatorjs"));
        out.push(dir.join("emulatorjs"));
    }
    // Beside the config file, which is the repo when the app is run from it.
    // The working directory is not: an app launched from the Finder or by
    // `open` gets `/`, and that is the normal way anybody starts it.
    if let Some(dir) = config.and_then(Path::parent) {
        out.push(dir.join("assets/emulatorjs"));
    }
    out.push(cwd.join("assets/emulatorjs"));
    out
}

/// Resolve one `/data/...` request against the EmulatorJS directory.
///
/// Returns `None` for anything that escapes it. The check is on the *request*,
/// before touching the filesystem, because a canonicalised comparison cannot
/// tell a symlink the unpack ships from one a request invented -- and
/// EmulatorJS is third-party bytes fetched by a script.
pub fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = url_path.trim_start_matches('/');
    let rel = rel.strip_prefix("data/")?;
    let mut out = root.join("data");
    for part in rel.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ if part.contains('\\') => return None,
            _ => out.push(part),
        }
    }
    Some(out)
}

/// What to say a file is. EmulatorJS refuses to instantiate a `.wasm` served as
/// `application/octet-stream`, and a `.js` module served as text never runs.
pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "js" => "text/javascript",
        "css" => "text/css",
        "html" => "text/html",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// One reply: everything a webview needs, with no webview types in it.
///
/// Split out from the scheme handler in `src-tauri` so the byte arithmetic --
/// which is the part that goes wrong -- can be tested without building a window.
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

/// Read a file, honouring one `Range` header.
///
/// EmulatorJS asks for the tail of a zip before it asks for the rest, and a 200
/// to a range request makes it start the whole download again -- the same
/// reason the service serves ROMs through `ServeFile`.
pub fn read_range(path: &Path, range: Option<&str>) -> Reply {
    use std::io::{Read, Seek, SeekFrom};
    let err = |status, why: &str| Reply {
        status,
        content_type: "text/plain",
        headers: Vec::new(),
        body: why.as_bytes().to_vec(),
    };
    let Ok(mut f) = std::fs::File::open(path) else {
        return err(404, "not found");
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let content_type = content_type(path);
    let Some(spec) = range.and_then(|r| r.trim().strip_prefix("bytes=")) else {
        let mut body = Vec::with_capacity(len as usize);
        if f.read_to_end(&mut body).is_err() {
            return err(500, "unreadable");
        }
        return Reply {
            status: 200,
            content_type,
            headers: vec![("Accept-Ranges", "bytes".into())],
            body,
        };
    };
    // One range only. A multi-range request needs a multipart body, nothing
    // here asks for one, and half-answering it with the first range would be a
    // wrong answer rather than an unsupported one.
    let (from, to) = spec.split_once('-').unwrap_or((spec, ""));
    let last = len.saturating_sub(1);
    let (start, end) = if from.is_empty() {
        // `bytes=-500`: the final 500 bytes, which is how a zip's directory is
        // fetched.
        let want: u64 = to.parse().unwrap_or(0);
        (len.saturating_sub(want.min(len)), last)
    } else {
        let start: u64 = from.parse().unwrap_or(0);
        (start, to.parse().unwrap_or(last).min(last))
    };
    if len == 0 || start > end || start > last {
        return Reply {
            status: 416,
            content_type: "text/plain",
            headers: vec![("Content-Range", format!("bytes */{len}"))],
            body: Vec::new(),
        };
    }
    let mut body = vec![0u8; (end - start + 1) as usize];
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut body).is_err() {
        return err(500, "unreadable");
    }
    Reply {
        status: 206,
        content_type,
        headers: vec![
            ("Accept-Ranges", "bytes".into()),
            ("Content-Range", format!("bytes {start}-{end}/{len}")),
        ],
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-ejs-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unpack(dir: &Path) {
        std::fs::create_dir_all(dir.join("data/cores")).unwrap();
        std::fs::write(dir.join(MARKER), "// loader").unwrap();
    }

    #[test]
    fn locate_skips_a_directory_that_is_not_an_unpack() {
        let tmp = scratch("locate");
        let empty = tmp.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let real = tmp.join("real");
        unpack(&real);
        let found = locate([empty.as_path(), real.as_path()]);
        assert_eq!(found, Some(real));
    }

    #[test]
    fn locate_is_none_when_nothing_is_installed() {
        let tmp = scratch("none");
        assert_eq!(locate([tmp.as_path()]), None);
    }

    #[test]
    fn a_bundle_looks_beside_itself_before_the_working_directory() {
        let exe = PathBuf::from("/Applications/Moose Rack.app/Contents/MacOS/moose-gui");
        let c = desktop_candidates(Some(&exe), Path::new("/repo"), None);
        assert_eq!(
            c.first().unwrap(),
            Path::new("/Applications/Moose Rack.app/Contents/MacOS/../Resources/emulatorjs")
        );
        assert_eq!(c.last().unwrap(), Path::new("/repo/assets/emulatorjs"));
    }

    #[test]
    fn the_config_file_says_where_the_repo_is() {
        // An app started from the Finder has `/` as its working directory, so
        // `assets/emulatorjs` relative to it is `/assets/emulatorjs`. The
        // config file is the one thing that always names the real tree.
        let c = desktop_candidates(
            Some(Path::new("/Applications/Moose Rack.app/Contents/MacOS/moose-gui")),
            Path::new("/"),
            Some(Path::new("/Users/frank/Projects/moose-rack/config.toml")),
        );
        assert!(c.contains(&PathBuf::from("/Users/frank/Projects/moose-rack/assets/emulatorjs")));
    }

    #[test]
    fn resolve_stays_under_data() {
        let root = Path::new("/ejs");
        assert_eq!(resolve(root, "/data/loader.js"), Some(PathBuf::from("/ejs/data/loader.js")));
        assert_eq!(
            resolve(root, "data/cores/snes9x-wasm.data"),
            Some(PathBuf::from("/ejs/data/cores/snes9x-wasm.data"))
        );
    }

    #[test]
    fn resolve_refuses_to_leave_the_directory() {
        let root = Path::new("/ejs");
        for bad in [
            "/data/../../etc/passwd",
            "/data/cores/../../../../etc/passwd",
            "/data/..",
            "/etc/passwd",
            "/rom/12",
        ] {
            assert_eq!(resolve(root, bad), None, "{bad} should not resolve");
        }
    }

    #[test]
    fn resolve_refuses_a_backslash_segment() {
        // Windows treats `..\` as a parent too, and a segment that survived the
        // `..` check by carrying its own separator would walk out of the tree.
        assert_eq!(resolve(Path::new("/ejs"), "/data/..\\..\\secret"), None);
    }

    #[test]
    fn wasm_is_not_octet_stream() {
        assert_eq!(content_type(Path::new("a/b/core.wasm")), "application/wasm");
        assert_eq!(content_type(Path::new("loader.js")), "text/javascript");
        assert_eq!(content_type(Path::new("snes9x-wasm.data")), "application/octet-stream");
    }

    #[test]
    fn a_whole_file_is_a_200_that_says_it_takes_ranges() {
        let dir = scratch("whole");
        let p = dir.join("core.wasm");
        std::fs::write(&p, b"0123456789").unwrap();
        let r = read_range(&p, None);
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"0123456789");
        assert_eq!(r.content_type, "application/wasm");
        assert!(r.headers.iter().any(|(k, v)| *k == "Accept-Ranges" && v == "bytes"));
    }

    #[test]
    fn a_range_is_a_206_with_only_those_bytes() {
        let dir = scratch("range");
        let p = dir.join("game.sfc");
        std::fs::write(&p, b"0123456789").unwrap();
        let r = read_range(&p, Some("bytes=2-5"));
        assert_eq!(r.status, 206);
        assert_eq!(r.body, b"2345");
        assert!(r.headers.iter().any(|(k, v)| *k == "Content-Range" && v == "bytes 2-5/10"));
    }

    #[test]
    fn an_open_range_runs_to_the_end() {
        let dir = scratch("open");
        let p = dir.join("game.sfc");
        std::fs::write(&p, b"0123456789").unwrap();
        let r = read_range(&p, Some("bytes=7-"));
        assert_eq!(r.status, 206);
        assert_eq!(r.body, b"789");
        assert!(r.headers.iter().any(|(_, v)| v == "bytes 7-9/10"));
    }

    #[test]
    fn a_suffix_range_is_the_tail() {
        // `bytes=-4` means the last four bytes, not "up to byte 4". Reading it
        // the other way hands a zip reader the header when it asked for the
        // central directory, which fails as a corrupt archive rather than as a
        // bad range.
        let dir = scratch("suffix");
        let p = dir.join("game.zip");
        std::fs::write(&p, b"0123456789").unwrap();
        let r = read_range(&p, Some("bytes=-4"));
        assert_eq!(r.status, 206);
        assert_eq!(r.body, b"6789");
        assert!(r.headers.iter().any(|(_, v)| v == "bytes 6-9/10"));
    }

    #[test]
    fn a_range_past_the_end_is_416_and_not_a_panic() {
        let dir = scratch("past");
        let p = dir.join("game.sfc");
        std::fs::write(&p, b"0123456789").unwrap();
        assert_eq!(read_range(&p, Some("bytes=99-200")).status, 416);
        let empty = dir.join("empty.sfc");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(read_range(&empty, Some("bytes=0-0")).status, 416);
    }

    #[test]
    fn a_missing_file_is_404() {
        assert_eq!(read_range(Path::new("/nope/nothing.js"), None).status, 404);
    }
}

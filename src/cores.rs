//! Downloading libretro cores from the buildbot.
//!
//! Cores ship as `<stem>_libretro.dylib.zip`, one zip per core, rebuilt
//! nightly. Download and unzip is the whole procedure — see PLAN.md §6.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

const BUILDBOT: &str = "https://buildbot.libretro.com/nightly";

/// Buildbot path segment for the host we're running on.
///
/// Not every core is built for `arm64` (Dolphin has been absent), so callers
/// should be prepared to fall back to `x86_64` under Rosetta.
pub fn platform_segment() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "apple/osx/arm64",
        ("macos", "x86_64") => "apple/osx/x86_64",
        ("linux", "x86_64") => "linux/x86_64",
        ("linux", "aarch64") => "linux/arm64",
        // The buildbot ships MSVC and MinGW trees for Windows; the MinGW one is
        // what the official RetroArch download links against.
        ("windows", "x86_64") => "windows/x86_64",
        ("windows", "x86") => "windows/x86",
        (os, arch) => bail!("no buildbot mapping for {os}/{arch}"),
    })
}

/// Shared-library extension for the host.
pub fn lib_extension() -> &'static str {
    match std::env::consts::OS {
        "macos" => "dylib",
        "windows" => "dll",
        _ => "so",
    }
}

pub fn core_filename(core: &str) -> String {
    format!("{core}_libretro.{}", lib_extension())
}

fn core_url(segment: &str, core: &str) -> String {
    format!("{BUILDBOT}/{segment}/latest/{}.zip", core_filename(core))
}

/// Download one core and extract it into `dest_dir`.
///
/// Returns the number of bytes written. Existing files are overwritten, so a
/// re-run upgrades to the current nightly.
pub async fn install(
    client: &reqwest::Client,
    core: &str,
    dest_dir: &Path,
    segment: &str,
) -> Result<u64> {
    let url = core_url(segment, core);
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;

    if !resp.status().is_success() {
        bail!("{} -> HTTP {}", url, resp.status());
    }

    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("downloading {url}"))?;

    // The archive holds exactly the one shared library.
    let reader = std::io::Cursor::new(&bytes);
    let mut zip = zip::ZipArchive::new(reader)
        .with_context(|| format!("{core}: response was not a zip ({} bytes)", bytes.len()))?;

    let wanted = core_filename(core);
    let mut written = 0u64;
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("creating {}", dest_dir.display()))?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        // Ignore any path component in the archive; cores are flat.
        let name = Path::new(entry.name())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name != wanted {
            continue;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        let out = dest_dir.join(&name);
        std::fs::write(&out, &buf).with_context(|| format!("writing {}", out.display()))?;
        written = buf.len() as u64;
    }

    if written == 0 {
        bail!("{core}: archive did not contain {wanted}");
    }
    Ok(written)
}

/// Install `core` if this RetroArch does not already have it.
///
/// Called on the launch path: a fresh install has no cores at all, and failing
/// with "core not installed" when the buildbot has it one HTTP request away is
/// a poor first run. Returns true when something was actually fetched.
///
/// Errors are deliberately not swallowed — a silent failure here surfaces later
/// as the same confusing "core not installed", which is what this exists to
/// avoid.
pub async fn ensure(
    client: &reqwest::Client,
    ra: &crate::retroarch::RetroArch,
    core: &str,
) -> Result<bool> {
    if ra.has_core(core) {
        return Ok(false);
    }
    let dir = ra.cores_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    install(client, core, &dir, platform_segment()?).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name on disk and the name in the download URL are the same string,
    /// and both are built from the host's library extension. Getting it wrong
    /// does not fail at build time or at startup: the download 404s, or the
    /// core installs under a name `has_core` will never look for, and the app
    /// reports "core not installed" for a core sitting right there.
    #[test]
    fn a_core_is_named_for_the_host_it_will_run_on() {
        let name = core_filename("mgba");
        assert!(name.starts_with("mgba_libretro."), "{name}");
        let expected = if cfg!(target_os = "macos") {
            "dylib"
        } else if cfg!(target_os = "windows") {
            "dll"
        } else {
            "so"
        };
        assert_eq!(name, format!("mgba_libretro.{expected}"));
    }

    /// The URL is the filename hung off the buildbot's per-host tree. Both
    /// halves have to be right, and a wrong one is a 404 at the moment someone
    /// is trying to play something.
    #[test]
    fn the_download_url_carries_the_host_tree_and_the_host_extension() {
        let url = core_url("apple/osx/arm64", "mgba");
        assert_eq!(
            url,
            format!("{BUILDBOT}/apple/osx/arm64/latest/{}.zip", core_filename("mgba"))
        );
        // No doubled or missing separators, which is the usual way a hand-built
        // URL goes wrong.
        assert!(!url.contains("//latest"), "{url}");
        assert!(url.starts_with("https://"), "{url}");
    }

    /// This machine has to have a mapping, or nothing can be installed on it.
    #[test]
    fn the_host_this_is_running_on_has_a_buildbot_tree() {
        let seg = platform_segment().expect("no buildbot mapping for this host");
        assert!(!seg.is_empty());
        assert!(!seg.starts_with('/') && !seg.ends_with('/'), "{seg}");
        // The three we build for, each with the shape their tree really has.
        let expected_prefix = if cfg!(target_os = "macos") {
            "apple/osx/"
        } else if cfg!(target_os = "windows") {
            "windows/"
        } else {
            "linux/"
        };
        assert!(seg.starts_with(expected_prefix), "{seg} is not under {expected_prefix}");
    }
}

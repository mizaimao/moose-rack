//! Downloading and installing RetroArch itself.
//!
//! Cores come from the nightly buildbot (see `cores.rs`); the emulator comes
//! from the stable tree:
//!
//! ```text
//! https://buildbot.libretro.com/stable/<version>/apple/osx/x86_64/RetroArch.dmg
//! https://buildbot.libretro.com/stable/<version>/windows/x86_64/RetroArch.7z
//! ```
//!
//! There is no separate `arm64` macOS directory — the `x86_64` path serves a
//! universal disk image (~171 MB), which is what Apple Silicon machines get.
//!
//! Installs land next to our other data rather than in `/Applications`, so the
//! whole thing stays inside the one deletable folder, and we set up portable
//! mode so it never touches an existing RetroArch's config.

use std::path::{Path, PathBuf};
// Every use of Command here is macOS-only (hdiutil, cp -R, xattr); importing it
// unconditionally is an unused import on Linux and Windows.
#[cfg(target_os = "macos")]
use std::process::Command;

use anyhow::{Context, Result, bail};

const STABLE_BASE: &str = "https://buildbot.libretro.com/stable";

/// Stable releases we know the layout of, newest first.
///
/// Probed in order rather than scraped: the directory listing is HTML and
/// parsing it to pick a version is more fragile than trying a short list.
const KNOWN_VERSIONS: &[&str] = &["1.22.2", "1.22.1", "1.22.0", "1.21.0", "1.20.0"];

/// Where the download lives for this OS, and how to unpack it.
fn artifact(version: &str) -> Result<(String, &'static str)> {
    Ok(match std::env::consts::OS {
        // Universal build; there is no arm64-specific directory.
        "macos" => (
            format!("{STABLE_BASE}/{version}/apple/osx/x86_64/RetroArch.dmg"),
            "dmg",
        ),
        "windows" => (
            format!("{STABLE_BASE}/{version}/windows/x86_64/RetroArch.7z"),
            "7z",
        ),
        // Linux has no single official binary: RetroArch ships through distro
        // packages, Flatpak and AppImage, and each puts cores and config
        // somewhere different. Naming them beats a generic failure.
        "linux" => bail!(
            "RetroArch is not downloaded automatically on Linux — install it with your \n\
             package manager, Flatpak (`flatpak install flathub org.libretro.RetroArch`),\n\
             or an AppImage, then set [retroarch] root in config.toml if it is not found."
        ),
        os => bail!("no RetroArch download configured for {os}; install it manually"),
    })
}

/// Newest version whose artifact actually exists.
pub async fn latest_available(http: &reqwest::Client) -> Result<String> {
    for v in KNOWN_VERSIONS {
        let (url, _) = artifact(v)?;
        if let Ok(resp) = http.head(&url).send().await
            && resp.status().is_success()
        {
            return Ok((*v).to_owned());
        }
    }
    bail!("no known RetroArch release is downloadable right now")
}

/// Only `install_dmg` shells out, and that is macOS-only, so this would be dead
/// code elsewhere.
#[cfg(target_os = "macos")]
fn run(cmd: &mut Command, what: &str) -> Result<String> {
    let out = cmd.output().with_context(|| format!("running {what}"))?;
    if !out.status.success() {
        bail!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Mount a .dmg, copy RetroArch.app out, unmount.
///
/// The mount point is parsed from `hdiutil`'s output rather than guessed,
/// because the volume name varies between releases.
#[cfg(target_os = "macos")]
fn install_dmg(dmg: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let out = run(
        Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-readonly"])
            .arg(dmg),
        "hdiutil attach",
    )?;
    let mount = out
        .lines()
        .filter_map(|l| l.split('\t').next_back())
        .map(str::trim)
        .find(|p| p.starts_with("/Volumes/"))
        .map(PathBuf::from)
        .context("could not find the mount point in hdiutil output")?;

    // Always detach, even if the copy fails.
    let result = (|| -> Result<PathBuf> {
        let app = std::fs::read_dir(&mount)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .context("no .app inside the disk image")?;

        std::fs::create_dir_all(dest_dir)?;
        let target = dest_dir.join(app.file_name().unwrap_or_default());
        if target.exists() {
            std::fs::remove_dir_all(&target).ok();
        }
        // `cp -R` preserves the bundle's symlinks and permissions, which a
        // naive recursive copy would flatten and break code signing.
        run(
            Command::new("cp").arg("-R").arg(&app).arg(dest_dir),
            "cp -R",
        )?;
        Ok(target)
    })();

    run(
        Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mount),
        "hdiutil detach",
    )
    .ok();
    result
}

#[cfg(not(target_os = "macos"))]
fn install_dmg(_dmg: &Path, _dest_dir: &Path) -> Result<PathBuf> {
    bail!("disk images can only be mounted on macOS")
}

/// macOS marks downloads as quarantined; Gatekeeper then refuses to launch
/// them from another process. Clearing the attribute is what the user would
/// otherwise do by right-clicking Open once.
#[cfg(target_os = "macos")]
fn clear_quarantine(path: &Path) {
    let _ = Command::new("xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(path)
        .output();
}

#[cfg(not(target_os = "macos"))]
fn clear_quarantine(_path: &Path) {}

/// Download and install RetroArch into `dest_dir`, returning the install root.
///
/// `progress` receives `(downloaded, total)`.
pub async fn install(
    http: &reqwest::Client,
    version: &str,
    dest_dir: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    use futures_util::StreamExt;

    let (url, kind) = artifact(version)?;
    std::fs::create_dir_all(dest_dir)?;
    let tmp = dest_dir.join(format!("retroarch-download.{kind}"));

    let resp = http.get(&url).send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("GET {url} -> {}", resp.status());
    }
    let total = resp.content_length().unwrap_or(0);

    let mut file = std::fs::File::create(&tmp)?;
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading download")?;
        std::io::Write::write_all(&mut file, &chunk)?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    drop(file);

    let installed = match kind {
        "dmg" => install_dmg(&tmp, dest_dir)?,
        "7z" => {
            let target = dest_dir.join("RetroArch");
            if target.exists() {
                std::fs::remove_dir_all(&target).ok();
            }
            sevenz_rust2::decompress_file(&tmp, &target)
                .with_context(|| format!("extracting {}", tmp.display()))?;
            target
        }
        other => bail!("no handler for a .{other} artifact"),
    };

    std::fs::remove_file(&tmp).ok();
    clear_quarantine(&installed);

    // Portable mode: RetroArch keeps config, saves and cores beside the app
    // rather than in the user's home, so this install cannot disturb another.
    let marker = dest_dir.join("portable.txt");
    if !marker.exists() {
        std::fs::write(&marker, "").ok();
    }

    Ok(dest_dir.to_path_buf())
}

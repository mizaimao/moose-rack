//! ES-DE theme downloader.
//!
//! Reads the official themes list and downloads themes as zip archives.
//! No `git` involved -- Windows ships none, so cloning meant that platform
//! could not install a theme at all. Themes land in
//! our own directory (`<library>/themes/`) rather than `~/ES-DE/themes`, so
//! downloading one here never disturbs an existing ES-DE install — and it
//! stays inside the single deletable folder all our data lives in.
//!
//! Cloning is `--depth 1`: several themes carry hundreds of megabytes of
//! artwork and there is no reason to fetch their history.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const THEMES_LIST_URL: &str =
    "https://gitlab.com/es-de/themes/themes-list/-/raw/master/themes.json";

#[derive(Debug, Deserialize)]
struct ThemesList {
    #[serde(default)]
    themes: Vec<RemoteTheme>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTheme {
    pub name: String,
    /// Directory name the theme is cloned into; also its stable identifier.
    #[serde(default)]
    pub reponame: String,
    pub url: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(default)]
    pub color_schemes: Vec<String>,
    #[serde(default)]
    pub screenshots: Vec<Screenshot>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Screenshot {
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub caption: String,
}

/// Base for screenshot paths, which are relative to the themes-list repo.
const SCREENSHOT_BASE: &str =
    "https://gitlab.com/es-de/themes/themes-list/-/raw/master/";

impl RemoteTheme {
    /// Directory name to clone into, falling back to a slug of the name.
    pub fn dir_name(&self) -> String {
        if !self.reponame.is_empty() {
            return self.reponame.clone();
        }
        self.name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect()
    }

    /// Absolute URL of the first screenshot, for preview thumbnails.
    ///
    /// Hosted on gitlab.com, not the RomM server — previewing a theme is
    /// inherently a network operation, since the next step clones it.
    pub fn screenshot_url(&self) -> Option<String> {
        self.screenshot_urls().into_iter().next()
    }

    /// Every screenshot the author published, for a preview that shows more
    /// than one screen of a theme.
    pub fn screenshot_urls(&self) -> Vec<String> {
        self.screenshots
            .iter()
            .filter(|s| !s.image.is_empty())
            .map(|s| format!("{SCREENSHOT_BASE}{}", s.image))
            .collect()
    }

    /// Loose match on name or reponame, for CLI lookup.
    pub fn matches(&self, needle: &str) -> bool {
        let n = needle.to_ascii_lowercase();
        self.name.to_ascii_lowercase().contains(&n)
            || self.reponame.to_ascii_lowercase().contains(&n)
    }
}

/// Fetch the official themes list with a throwaway client.
///
/// Themes are browsable with no RomM server configured, so this does not
/// depend on the API client existing.
pub async fn list_default() -> Result<Vec<RemoteTheme>> {
    let http = crate::util::http_client(None).context("building HTTP client")?;
    list(&http).await
}

/// Fetch the official themes list.
pub async fn list(http: &reqwest::Client) -> Result<Vec<RemoteTheme>> {
    let resp = http
        .get(THEMES_LIST_URL)
        .send()
        .await
        .with_context(|| format!("GET {THEMES_LIST_URL}"))?;
    if !resp.status().is_success() {
        bail!("{THEMES_LIST_URL} -> {}", resp.status());
    }
    let parsed: ThemesList = resp.json().await.context("parsing themes.json")?;
    Ok(parsed.themes)
}

/// Candidate archive URLs for a theme's git URL, best first.
///
/// Themes were cloned with `git`, which meant a Windows install could not
/// download a single one — Windows ships no git, and the failure surfaced as
/// "running git (is it installed?)" at the moment someone was trying to make
/// the app look nicer.
///
/// Both hosts in the official list serve a zip of any branch over plain HTTP,
/// so nothing external is needed. The branch is unknown, hence a list: `master`
/// first because that is what the ES-DE themes use, then `main`.
pub fn archive_urls(git_url: &str) -> Vec<String> {
    let base = git_url.trim_end_matches('/').trim_end_matches(".git");
    let name = base.rsplit('/').next().unwrap_or("theme");
    let mut out = Vec::new();
    for branch in ["master", "main"] {
        if base.contains("gitlab.") {
            out.push(format!("{base}/-/archive/{branch}/{name}-{branch}.zip"));
        } else if base.contains("github.") {
            out.push(format!("{base}/archive/refs/heads/{branch}.zip"));
        }
    }
    out
}

/// Unpack a theme zip into `dest`, dropping the wrapper directory.
///
/// Both hosts wrap everything in one top-level folder named after the repo and
/// branch. Keeping it would put the theme one level deeper than every path in
/// the theme's own XML expects.
fn unpack(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context("reading the downloaded theme archive")?;

    if dest.exists() {
        std::fs::remove_dir_all(dest).ok();
    }
    std::fs::create_dir_all(dest)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(path) = entry.enclosed_name() else {
            // A path escaping the archive root. Skipped rather than trusted.
            continue;
        };
        // Strip the wrapper directory.
        let mut parts = path.components();
        parts.next();
        let rel: PathBuf = parts.collect();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).ok();
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut w = std::fs::File::create(&out)
            .with_context(|| format!("writing {}", out.display()))?;
        std::io::copy(&mut entry, &mut w)?;
    }
    Ok(())
}

/// Download a theme into `themes_dir`, replacing any existing copy.
///
/// Returns the installed path and whether it was new.
pub async fn install(
    http: &reqwest::Client,
    theme: &RemoteTheme,
    themes_dir: &Path,
) -> Result<(PathBuf, bool)> {
    std::fs::create_dir_all(themes_dir)
        .with_context(|| format!("creating {}", themes_dir.display()))?;
    let dest = themes_dir.join(theme.dir_name());
    let fresh = !dest.is_dir();

    let candidates = archive_urls(&theme.url);
    if candidates.is_empty() {
        bail!(
            "{} is hosted somewhere this cannot download from ({})",
            theme.name,
            theme.url
        );
    }

    let mut last = String::new();
    for url in &candidates {
        let resp = match http.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        if !resp.status().is_success() {
            // A 404 here just means the branch is named the other thing.
            last = format!("{url} -> {}", resp.status());
            continue;
        }
        let bytes = resp.bytes().await.context("downloading the theme")?;
        unpack(&bytes, &dest)?;
        return Ok((dest, fresh));
    }
    bail!("could not download {}: {last}", theme.name)
}

/// Delete an installed theme.
pub fn remove(dir_name: &str, themes_dir: &Path) -> Result<()> {
    let dest = themes_dir.join(dir_name);
    if !dest.is_dir() {
        bail!("{} is not installed", dir_name);
    }
    std::fs::remove_dir_all(&dest).with_context(|| format!("removing {}", dest.display()))?;
    Ok(())
}

/// Recursive size on disk, for reporting what a theme costs.
pub use crate::util::dir_size as size_of;

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of this module's rewrite: no `git`, so a theme URL has
    /// to become a plain HTTP archive URL. Getting the shape wrong means a 404
    /// and a theme that will not install.
    #[test]
    fn a_gitlab_url_becomes_an_archive_download() {
        let urls = archive_urls("https://gitlab.com/es-de/themes/modern-es-de.git");
        assert_eq!(
            urls[0],
            "https://gitlab.com/es-de/themes/modern-es-de/-/archive/master/modern-es-de-master.zip"
        );
        // Branch is unknown up front, so both are offered.
        assert!(urls[1].contains("/archive/main/modern-es-de-main.zip"));
    }

    /// The official list carries a few GitHub-hosted themes, which use a
    /// completely different archive path.
    #[test]
    fn a_github_url_uses_the_other_archive_shape() {
        let urls = archive_urls("https://github.com/someone/a-theme");
        assert_eq!(urls[0], "https://github.com/someone/a-theme/archive/refs/heads/master.zip");
        assert_eq!(urls[1], "https://github.com/someone/a-theme/archive/refs/heads/main.zip");
    }

    /// A trailing slash or a missing .git must not change the repo name, or the
    /// archive file name is wrong and the download 404s.
    #[test]
    fn the_repo_name_survives_trailing_punctuation() {
        for u in [
            "https://gitlab.com/es-de/themes/slate.git",
            "https://gitlab.com/es-de/themes/slate/",
            "https://gitlab.com/es-de/themes/slate",
        ] {
            assert!(archive_urls(u)[0].ends_with("/slate-master.zip"), "{u}");
        }
    }

    /// Somewhere neither shape fits gets no candidates, so `install` can say so
    /// rather than requesting a URL it invented.
    #[test]
    fn an_unknown_host_offers_nothing() {
        assert!(archive_urls("https://example.com/themes/whatever.git").is_empty());
    }

    /// Both hosts wrap the tree in one directory named after the repo and
    /// branch. Keeping it would put every file one level deeper than the
    /// theme's own XML expects.
    #[test]
    fn the_wrapper_directory_is_stripped() {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = Default::default();
            zip.start_file("slate-master/theme.xml", opts).unwrap();
            std::io::Write::write_all(&mut zip, b"<theme/>").unwrap();
            zip.start_file("slate-master/_inclusions/colors.xml", opts).unwrap();
            std::io::Write::write_all(&mut zip, b"<colors/>").unwrap();
            zip.finish().unwrap();
        }

        let dir = std::env::temp_dir().join("moose-rack-theme-unpack");
        std::fs::remove_dir_all(&dir).ok();
        unpack(&buf, &dir).unwrap();

        assert!(dir.join("theme.xml").is_file(), "theme.xml at the top");
        assert!(dir.join("_inclusions/colors.xml").is_file(), "nested files kept");
        assert!(!dir.join("slate-master").exists(), "wrapper gone");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Re-downloading replaces rather than merging, so a theme that dropped a
    /// file upstream does not keep a stale copy of it.
    #[test]
    fn reinstalling_replaces_the_previous_copy() {
        let dir = std::env::temp_dir().join("moose-rack-theme-replace");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stale.xml"), b"old").unwrap();

        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = Default::default();
            zip.start_file("t-master/theme.xml", opts).unwrap();
            std::io::Write::write_all(&mut zip, b"<theme/>").unwrap();
            zip.finish().unwrap();
        }
        unpack(&buf, &dir).unwrap();

        assert!(dir.join("theme.xml").is_file());
        assert!(!dir.join("stale.xml").exists(), "the old copy is gone");
        std::fs::remove_dir_all(&dir).ok();
    }
}

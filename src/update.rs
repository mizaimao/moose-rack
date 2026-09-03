//! Is there a newer release than the one running?
//!
//! The app publishes GitHub releases and has never mentioned that any exist, so
//! the only way to find out was to go and look. This asks, and says so once.
//!
//! ## Deliberately not an updater
//!
//! It reports and links; it does not download, replace or restart anything.
//! Self-replacing binaries need code signing, a rollback path and a story for
//! the half-written case, and none of that is worth carrying for a tool one
//! person runs on three machines. Knowing there is a new version is the part
//! that was missing.
//!
//! ## The version scheme is not semver
//!
//! This project counts 0.1.9 -> 0.1.10, and bumps the patch number by however
//! many things a batch contained — 0.2.302 to 0.2.439 is one afternoon.
//! Comparison is therefore numeric per component, never lexicographic: as
//! strings, "0.2.439" sorts before "0.2.99".

use anyhow::{Context, Result};
use serde::Deserialize;

/// Releases live here. Not configurable: this is where *this* app updates from,
/// and a build pointing somewhere else is a different app.
const LATEST: &str = "https://api.github.com/repos/mizaimao/moose-rack/releases/latest";

/// What is running now, from the crate version at build time.
pub fn running() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Update {
    /// `0.2.440`, without the `v`.
    pub version: String,
    /// What is running, so a caller can print both without asking twice.
    pub running: String,
    pub url: String,
    /// The release's title, when it has one worth showing.
    pub title: String,
}

/// Split a version into numbers, ignoring a leading `v` and anything after a
/// dash. A component that is not a number sorts as 0 rather than failing: a
/// tag nobody expected should not stop the check working.
fn parts(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('-')
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// True when `candidate` is a later version than `current`.
///
/// Numeric per component, because this project's numbers are not semver and
/// string comparison gets them wrong: "0.2.439" < "0.2.99" as text, and the
/// 439 is the newer one.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (parts(candidate), parts(current));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The newest published release, or `None` when it is not newer than this one.
///
/// Drafts and prereleases are skipped: `releases/latest` already excludes them,
/// and the guard is here so a change to that endpoint cannot start offering
/// someone a build that was never announced.
pub async fn check(http: &reqwest::Client) -> Result<Option<Update>> {
    let resp = http
        .get(LATEST)
        // GitHub refuses an API request with no user agent.
        .header("User-Agent", format!("moose-rack/{}", running()))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("GET {LATEST}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("{LATEST} -> {}", resp.status());
    }
    let rel: GhRelease = resp.json().await.context("reading the release")?;
    Ok(newer_than(&rel, running()))
}

/// The comparison, split out so it can be tested without a network.
fn newer_than(rel: &GhRelease, current: &str) -> Option<Update> {
    if rel.draft || rel.prerelease {
        return None;
    }
    let version = rel.tag_name.trim_start_matches(['v', 'V']).to_owned();
    if !is_newer(&version, current) {
        return None;
    }
    Some(Update {
        title: if rel.name.is_empty() { version.clone() } else { rel.name.clone() },
        version,
        running: current.to_owned(),
        url: if rel.html_url.is_empty() {
            "https://github.com/mizaimao/moose-rack/releases".to_owned()
        } else {
            rel.html_url.clone()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            html_url: "https://example.invalid/r".into(),
            name: String::new(),
            prerelease: false,
            draft: false,
        }
    }

    /// The one that matters: this project's numbers are not semver, and as
    /// text "0.2.439" sorts before "0.2.99".
    #[test]
    fn versions_compare_as_numbers_not_as_text() {
        assert!(is_newer("0.2.440", "0.2.439"));
        assert!(is_newer("0.2.439", "0.2.99"), "439 is later than 99");
        assert!(!is_newer("0.2.99", "0.2.439"));
        assert!(is_newer("0.1.10", "0.1.9"), "the scheme goes 0.1.9 -> 0.1.10");
        assert!(!is_newer("0.1.9", "0.1.10"));
    }

    #[test]
    fn the_same_version_is_not_an_update() {
        assert!(!is_newer("0.2.440", "0.2.440"));
        assert!(!is_newer("v0.2.440", "0.2.440"), "a leading v is not a difference");
    }

    #[test]
    fn a_shorter_version_is_padded_rather_than_mismatched() {
        assert!(is_newer("0.3", "0.2.440"));
        assert!(!is_newer("0.2", "0.2.0"));
        assert!(is_newer("1", "0.9.9"));
    }

    /// A tag nobody expected should leave the check working, not break it.
    #[test]
    fn an_unparseable_tag_does_not_panic() {
        assert!(!is_newer("nightly", "0.2.440"));
        assert!(is_newer("0.2.441-rc1", "0.2.440"), "the suffix is ignored, the number is not");
    }

    #[test]
    fn a_newer_release_is_reported_with_both_versions() {
        let u = newer_than(&rel("v0.2.450"), "0.2.440").unwrap();
        assert_eq!(u.version, "0.2.450");
        assert_eq!(u.running, "0.2.440");
        assert_eq!(u.url, "https://example.invalid/r");
    }

    #[test]
    fn an_older_or_equal_release_is_nothing_to_report() {
        assert!(newer_than(&rel("v0.2.440"), "0.2.440").is_none());
        assert!(newer_than(&rel("v0.2.1"), "0.2.440").is_none());
    }

    /// `releases/latest` already excludes these; the guard is so a change at
    /// GitHub's end cannot start offering an unannounced build.
    #[test]
    fn drafts_and_prereleases_are_never_offered() {
        let mut d = rel("v9.9.9");
        d.draft = true;
        assert!(newer_than(&d, "0.2.440").is_none());
        let mut p = rel("v9.9.9");
        p.prerelease = true;
        assert!(newer_than(&p, "0.2.440").is_none());
    }

    /// A release with no `html_url` still has to lead somewhere.
    #[test]
    fn a_release_with_no_link_falls_back_to_the_releases_page() {
        let mut r = rel("v9.9.9");
        r.html_url = String::new();
        let u = newer_than(&r, "0.2.440").unwrap();
        assert!(u.url.ends_with("/releases"), "{}", u.url);
    }

    /// The running version has to be a version, or every comparison is against
    /// nothing and the check silently never fires.
    #[test]
    fn the_running_version_parses() {
        let p = parts(running());
        assert_eq!(p.len(), 3, "expected three components in {}", running());
        assert!(p.iter().any(|n| *n > 0), "{} looks empty", running());
    }
}

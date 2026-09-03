//! Console pictures for the platform grid, taken from your own RomM server.
//!
//! RomM ships a drawing of every console it knows about and serves them at
//! `/assets/platforms/<slug>.svg` — a 1000x1000 illustration of the actual
//! hardware, not the system's name set as type. They need no authentication,
//! they are already on a machine on this network, and being vector art they
//! stay sharp at whatever size the grid is zoomed to.
//!
//! This is why the app no longer asks anyone to download an ES-DE theme to get
//! pictures on the platform cards. A theme is a few hundred megabytes fetched
//! from a stranger's git host to obtain art that the server we are already
//! talking to hands over in a few kilobytes each.
//!
//! Not every platform has one. The server had 28 of 35 when this was written —
//! `saturn`, `sms`, `naomi`, `famicom`, `pico`, `g-and-w` and `easyrpg` were
//! missing. Those fall back to the styled name, so a gap is a plain-looking
//! card rather than an empty one.
//!
//! ## Which slug
//!
//! Two different slugs are in play and they do not always agree: RomM's own
//! (`sms`, `genesis`, `sfam`) and the folder name the files live under
//! (`mastersystem`, `megadrive`, `sfc`). The icons are published under the
//! first; everything else in this app keys off the second. So the fetch reads
//! RomM's slug and the file is *saved* under the folder name, and no later
//! lookup has to know that the two ever differed.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::api::Client;

/// Where the cache lives, under the media root.
const DIR: &str = "platform-icons";

/// A cached icon for a platform, if one was fetched.
///
/// Keyed by the folder name (`mastersystem`), which is what the rest of the
/// app carries around.
pub fn installed(media_root: &Path, fs_slug: &str) -> Option<PathBuf> {
    let path = media_root.join(DIR).join(format!("{}.svg", safe(fs_slug)));
    path.is_file().then_some(path)
}

/// Fetch any icons not already cached.
///
/// `platforms` is `(server_slug, fs_slug)`. Returns how many new files landed.
///
/// A platform with no icon on the server is remembered as a miss, so a library
/// with seven of them does not re-ask for all seven on every sync. The marker
/// is an empty file, which `installed` rejects because it only accepts an SVG
/// it can actually name — see `safe`.
pub async fn ensure(
    client: &Client,
    media_root: &Path,
    platforms: &[(String, String)],
) -> Result<usize> {
    let dir = media_root.join(DIR);
    std::fs::create_dir_all(&dir)?;

    let mut fetched = 0;
    for (slug, fs_slug) in platforms {
        let file = dir.join(format!("{}.svg", safe(fs_slug)));
        let miss = dir.join(format!("{}.absent", safe(fs_slug)));
        if file.is_file() || miss.is_file() {
            continue;
        }

        let url = format!("{}/assets/platforms/{}.svg", client.base(), slug);
        let res = client
            .http()
            .get(&url)
            .header("Authorization", client.auth())
            .send()
            .await;

        match res {
            Ok(r) if r.status().is_success() => {
                let body = r.bytes().await?;
                // A 200 carrying an HTML error page would otherwise be written
                // out as an .svg and render as nothing.
                if body.starts_with(b"<?xml") || body.starts_with(b"<svg") {
                    std::fs::write(&file, &body)?;
                    fetched += 1;
                } else {
                    let _ = std::fs::write(&miss, []);
                }
            }
            // 404 is the ordinary case for a console the icon set does not
            // cover, not a failure worth reporting.
            Ok(_) => {
                let _ = std::fs::write(&miss, []);
            }
            // A network error is not a miss: the server may be back next time,
            // so nothing is recorded and it will be retried.
            Err(_) => {}
        }
    }
    Ok(fetched)
}

/// Keep a slug to one path component.
///
/// Slugs come from the server, and a platform called `../../etc` would
/// otherwise write outside the cache.
fn safe(slug: &str) -> String {
    slug.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_cannot_escape_the_cache_directory() {
        assert_eq!(safe("../../etc/passwd"), "______etc_passwd");
        assert_eq!(safe("neo-geo-pocket"), "neo-geo-pocket");
    }

    /// The miss marker and the icon share a stem, so the marker must not be
    /// mistaken for an icon.
    #[test]
    fn an_absent_marker_is_not_offered_as_an_icon() {
        let dir = std::env::temp_dir().join("moose-rack-icon-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        std::fs::write(dir.join(DIR).join("saturn.absent"), []).unwrap();

        assert!(installed(&dir, "saturn").is_none());

        std::fs::write(dir.join(DIR).join("saturn.svg"), b"<svg/>").unwrap();
        assert!(installed(&dir, "saturn").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

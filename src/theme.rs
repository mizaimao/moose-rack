//! ES-DE theme support — system logos for the platform grid.
//!
//! Reuses the themes ES-DE already has installed rather than fetching icons
//! from IGDB or TheGamesDB. RomM's `url_logo` points at those external CDNs,
//! not at your own server, so using it would mean a network round trip to a
//! third party for every console icon. Themes are local, offline, SVG, and
//! already match the frontend you use.
//!
//! Themes disagree on where per-system art lives. Three conventions are
//! checked directly, and anything else is found by a depth-limited sweep for
//! a directory whose name contains "logo":
//!
//! ```text
//! <theme>/<system>/images/<style>.svg     slate-es-de, most community themes
//! <theme>/system/logos/<system>.svg       linear-es-de
//! <theme>/_inc/system-logo/<system>.svg   canvas-es-de
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::util::expand_tilde;

use crate::coremap::CoreMap;

/// Where ES-DE keeps themes, in probe order.
const THEME_ROOTS: &[&str] = &[
    // User-installed themes take precedence over the bundled set, and these
    // two locations are the same on every platform.
    "~/ES-DE/themes",
    "~/.emulationstation/themes",
    // The bundled set ships inside the application, which is where the
    // platforms diverge.
    #[cfg(target_os = "macos")]
    "~/Data/Games/Emulators/ES-DE.app/Contents/Resources/themes",
    #[cfg(target_os = "macos")]
    "/Applications/ES-DE.app/Contents/Resources/themes",
    #[cfg(target_os = "windows")]
    "C:/Program Files/ES-DE/themes",
    #[cfg(target_os = "windows")]
    "~/ES-DE/Application Data/themes",
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    "/usr/share/es-de/themes",
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    "/app/share/es-de/themes",
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    "~/.local/share/es-de/themes",
];

// svg first (crisp at any size), then the raster formats themes use.
const ICON_EXTENSIONS: &[&str] = &["svg", "webp", "png", "jpg"];

/// Which piece of per-system art to use for the platform grid.
///
/// Three kinds, in the order the Select button cycles them.
///
/// Styled text leads and is the default because it is the one kind nearly
/// every theme draws: across 142 surveyed artwork directories, 85 hold a
/// wordmark, 27 a controller and only 11 the console. Leading with hardware
/// meant leading with the rarest thing on offer.
///
/// There were five. `consolegame` and `systemart_legacy` are gone: one theme
/// in fifty-four ships legacy art, and `consolegame` is a slate idiom almost
/// nobody follows. Both were usually empty, and an empty style in the rotation
/// is a grid of nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconStyle {
    /// The system's name as a styled wordmark. Widest coverage, and the
    /// default. Keyed `logo` because that is what ES-DE themes call it.
    Logo,
    /// The system's controller.
    Controller,
    /// The console itself.
    SystemArt,
}

impl IconStyle {
    pub const ALL: [IconStyle; 3] = [Self::Logo, Self::Controller, Self::SystemArt];

    pub fn key(self) -> &'static str {
        match self {
            Self::Logo => "logo",
            Self::Controller => "controller",
            Self::SystemArt => "systemart",
        }
    }

    /// What the picker and the Select toast call it.
    ///
    /// "Logos" said nothing about what you would see. The art is the system's
    /// name set as a wordmark — styled text — and calling it that is the
    /// difference between a choice and a guess.
    pub fn label(self) -> &'static str {
        match self {
            Self::Logo => "Styled text",
            Self::Controller => "Controllers",
            Self::SystemArt => "Hardware",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.to_ascii_lowercase();
        // The two dropped styles map onto what replaced them rather than
        // failing: a config saying `systemart_legacy` was choosing hardware
        // art, and should keep getting hardware art.
        match s.as_str() {
            "systemart_legacy" => return Some(Self::SystemArt),
            "consolegame" => return Some(Self::SystemArt),
            _ => {}
        }
        Self::ALL.into_iter().find(|v| v.key() == s)
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub path: PathBuf,
}

/// As [`discover`], but also searching a downloaded-themes directory first.
pub fn discover_with(extra_root: Option<&str>, downloaded: Option<&Path>) -> Vec<Theme> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(d) = downloaded {
        roots.push(d.to_path_buf());
    }
    if let Some(e) = extra_root {
        roots.push(expand_tilde(e));
    }
    roots.extend(THEME_ROOTS.iter().map(|r| expand_tilde(r)));

    let mut out: Vec<Theme> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || out.iter().any(|t| t.name == name) {
                continue;
            }
            out.push(Theme { name, path });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// RomM platform slug -> candidate ES-DE system names, best first.
///
/// The two disagree on several names (`ngc`/`gc`, `dc`/`dreamcast`,
/// `neogeoaes`/`neogeo`, `neo-geo-pocket`/`ngp`), and the mapping is already
/// recorded in the extracted core map, so derive it rather than restate it.
///
/// Returns every candidate rather than one, because ROM-hack systems claim the
/// same slug as their base system — `genh` and `msu-md` both map to
/// `megadrive` — and only the base system has theme art. Ordering puts an
/// exact slug match first, then the base system, then the hack variants.
pub fn slug_to_esde(map: &CoreMap) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (esde_name, system) in &map.systems {
        for platform in &system.server_platforms {
            out.entry(platform.clone()).or_default().push(esde_name.clone());
        }
    }
    for (slug, names) in out.iter_mut() {
        // Exact match wins; otherwise shorter names are the base systems
        // ("megadrive" over "msu-md" is length-ambiguous, so exact-match and
        // the slug fallback below carry the real weight).
        names.sort_by_key(|n| (n != slug, n.len(), n.clone()));
        if !names.contains(slug) {
            names.push(slug.clone());
        }
    }
    out
}

/// Every ES-DE name each of our platforms might be filed under, in the order
/// worth trying.
///
/// A theme names its files for the ES-DE system, not for our slug, and the two
/// disagree often enough to matter — `megadrive` against `genesis`, `pcengine`
/// against `tg16`. Trying each in turn is the difference between a set that
/// installs thirty pictures and one that installs twelve.
pub fn esde_names_for(map: &CoreMap, slugs: &[String]) -> Vec<(String, Vec<String>)> {
    let names = slug_to_esde(map);
    slugs
        .iter()
        .map(|slug| {
            let mut cands = names.get(slug).cloned().unwrap_or_default();
            if !cands.contains(slug) {
                cands.push(slug.clone());
            }
            (slug.clone(), cands)
        })
        .collect()
}

/// A handful of this library's consoles to preview a set with.
///
/// The user's own systems rather than a fixed list: a preview showing consoles
/// they do not own is decoration, and the whole complaint that produced this
/// function was a preview that showed something other than what you get.
pub fn preview_systems(map: &CoreMap, slugs: &[String], want: usize) -> Vec<String> {
    esde_names_for(map, slugs)
        .into_iter()
        .filter_map(|(_, names)| names.into_iter().next())
        .take(want)
        .collect()
}

fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}

/// Locate one style of per-system art within a theme.
pub fn art_for(theme: &Theme, esde_system: &str, style: IconStyle) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for ext in ICON_EXTENSIONS {
        // slate-es-de and many community themes keep every style side by side.
        candidates.push(
            theme.path.join(esde_system).join("images").join(format!("{}.{ext}", style.key())),
        );
    }
    // Hardware renders live in their own top-level directories rather than
    // per-system folders: modern-es-de uses art/ and art_legacy/, linear-es-de
    // uses system/systemart/.
    if style == IconStyle::SystemArt {
        for ext in ICON_EXTENSIONS {
            candidates.push(theme.path.join("art").join(format!("{esde_system}.{ext}")));
            candidates
                .push(theme.path.join("system").join("systemart").join(format!("{esde_system}.{ext}")));
            // The older renders, now that they are not a style of their own:
            // better a classic picture of the console than none.
            candidates.push(theme.path.join("art_legacy").join(format!("{esde_system}.{ext}")));
        }
    }
    if style == IconStyle::Logo {
        for ext in ICON_EXTENSIONS {
            // linear-es-de
            candidates.push(theme.path.join("system").join("logos").join(format!("{esde_system}.{ext}")));
            // canvas-es-de and relatives
            candidates.push(theme.path.join("_inc").join("system-logo").join(format!("{esde_system}.{ext}")));
        }
    }
    let found = first_existing(&candidates);
    match (found, style) {
        (Some(p), _) => Some(p),
        // Only the sweep knows about unusual layouts, and it only knows logos.
        (None, IconStyle::Logo) => logo_by_sweep(theme, esde_system),
        (None, _) => None,
    }
}

/// Fallback: look for `<system>.<ext>` inside any directory whose name mentions
/// "logo".
///
/// Depth-limited because theme repositories run to hundreds of megabytes and a
/// full walk per system would be slow; logo directories are always shallow.
fn logo_by_sweep(theme: &Theme, esde_system: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with('.') {
                continue;
            }
            if name.contains("logo") {
                out.push(path.clone());
            }
            walk(&path, depth - 1, out);
        }
    }

    let mut dirs = Vec::new();
    walk(&theme.path, 3, &mut dirs);
    for dir in dirs {
        for ext in ICON_EXTENSIONS {
            let p = dir.join(format!("{esde_system}.{ext}"));
            if p.is_file() {
                return p.canonicalize().ok();
            }
        }
    }
    None
}

/// Resolve logos for every platform slug, using the first theme that has one.
///
/// Falling through theme by theme rather than requiring a single complete
/// theme means a sparse custom theme still contributes what it has.
pub fn logos(themes: &[Theme], map: &CoreMap, slugs: &[String]) -> BTreeMap<String, PathBuf> {
    art(themes, map, slugs, IconStyle::Logo)
}

/// As [`logos`], for a specific art style.
pub fn art(
    themes: &[Theme],
    map: &CoreMap,
    slugs: &[String],
    style: IconStyle,
) -> BTreeMap<String, PathBuf> {
    let names = slug_to_esde(map);
    let mut out = BTreeMap::new();
    'slug: for slug in slugs {
        let candidates = names
            .get(slug)
            .cloned()
            .unwrap_or_else(|| vec![slug.clone()]);
        for theme in themes {
            for esde in &candidates {
                if let Some(p) = art_for(theme, esde, style) {
                    out.insert(slug.clone(), p);
                    continue 'slug;
                }
            }
        }
    }
    out
}

/// Copy resolved logos into the local media tree so the UI has a stable place
/// to read from and works even if ES-DE is later moved or uninstalled.
pub fn install(
    themes: &[Theme],
    map: &CoreMap,
    slugs: &[String],
    media_root: &Path,
) -> Result<usize> {
    let mut total = 0;
    for style in IconStyle::ALL {
        total += install_style(themes, map, slugs, media_root, style)?;
    }
    Ok(total)
}

/// Where one set's art lives: `_platforms/sets/<set>/<style>/`.
///
/// A named set is one ES-DE theme's own artwork, kept apart from the shared
/// pool under `_platforms/<style>/` that `fetch_icons` fills from four themes
/// at once. Both exist because they answer different questions: the pool is
/// "give me the best hardware render available for this console from anywhere",
/// and a set is "show me this designer's work, all of it, together".
pub fn set_dir(media_root: &Path, set: &str, look: &str) -> PathBuf {
    media_root.join("_platforms").join("sets").join(set).join(look)
}

/// The sets that are actually on this machine, and how many pictures each has.
///
/// `iconart::set_ids()` is the shipped catalog — every set the app knows how to
/// download. Offering that as the choice list means picking one that was never
/// downloaded, and the screen then draws nothing with no way to tell why. What
/// belongs in the list is what has files under it.
pub fn installed_sets(media_root: &Path) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = crate::iconart::set_ids()
        .into_iter()
        .filter_map(|id| {
            let n = count_pictures(&media_root.join("_platforms").join("sets").join(&id));
            (n > 0).then_some((id, n))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Every picture under a set, across all the looks it keeps side by side.
fn count_pictures(set: &Path) -> usize {
    std::fs::read_dir(set)
        .into_iter()
        .flatten()
        .flatten()
        .map(|look| {
            std::fs::read_dir(look.path())
                .into_iter()
                .flatten()
                .flatten()
                .filter(|f| {
                    f.path()
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| ICON_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
                        .unwrap_or(false)
                })
                .count()
        })
        .sum()
}

/// How many pictures a named set holds in each of the looks it offers.
pub fn set_counts(media_root: &Path, set: &str, looks: &[String], slugs: &[String]) -> Vec<(String, usize)> {
    looks
        .iter()
        .map(|look| {
            let dir = set_dir(media_root, set, look);
            let n = slugs
                .iter()
                .filter(|slug| {
                    ICON_EXTENSIONS.iter().any(|ext| dir.join(format!("{slug}.{ext}")).is_file())
                })
                .count();
            (look.clone(), n)
        })
        .collect()
}

/// Every look in the shared pool that actually holds pictures.
///
/// Enumerated from disk rather than from a fixed list of kinds. The list used
/// to be an enum, and shrinking it from five to three deleted two looks a user
/// already had 24 pictures for — `consolegame` and `systemart_legacy` — from
/// their rotation, without deleting the files. Whatever is on disk is offered.
///
/// `sets/` is skipped: those belong to a chosen set and are offered by name.
pub fn pool_looks(media_root: &Path, slugs: &[String]) -> Vec<(String, usize)> {
    let base = media_root.join("_platforms");
    let Ok(rd) = std::fs::read_dir(&base) else { return Vec::new() };
    let mut out: Vec<(String, usize)> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != "sets")
        .map(|name| {
            let dir = base.join(&name);
            let n = slugs
                .iter()
                .filter(|slug| {
                    ICON_EXTENSIONS.iter().any(|ext| dir.join(format!("{slug}.{ext}")).is_file())
                })
                .count();
            (name, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    out.sort();
    out
}

/// A readable name for a pool look. Falls back to the folder name, because the
/// pool can hold anything an older build or a hand-copied theme put there.
pub fn pool_label(key: &str) -> String {
    match key {
        "systemart" => "Hardware".to_owned(),
        "systemart_legacy" => "Hardware (classic)".to_owned(),
        "consolegame" => "Console with a game".to_owned(),
        "controller" => "Controllers".to_owned(),
        "logo" => "Styled text".to_owned(),
        other => {
            let mut c = other.replace(['_', '-'], " ");
            if let Some(f) = c.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            c
        }
    }
}

/// The picture for one console in one look, wherever that look lives.
///
/// A look is a folder name: inside the chosen set first, then the shared pool.
/// Anything the look has no picture for falls back to whatever else is
/// downloaded, because no theme draws every system and a hole in the grid
/// reads as a failed download.
pub fn look_art(media_root: &Path, slug: &str, set: &str, look: &str) -> Option<PathBuf> {
    let base = media_root.join("_platforms");
    let mut dirs: Vec<PathBuf> = Vec::new();
    if !set.is_empty() && !look.is_empty() {
        dirs.push(base.join("sets").join(set).join(look));
    }
    if !look.is_empty() {
        dirs.push(base.join(look));
    }
    // Then anything else this set downloaded, then the rest of the pool.
    if !set.is_empty()
        && let Ok(rd) = std::fs::read_dir(base.join("sets").join(set))
    {
        let mut rest: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        rest.sort();
        dirs.extend(rest);
    }
    if let Ok(rd) = std::fs::read_dir(&base) {
        let mut rest: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "sets"))
            .collect();
        rest.sort();
        dirs.extend(rest);
    }
    for d in dirs {
        for ext in ICON_EXTENSIONS {
            let p = d.join(format!("{slug}.{ext}"));
            if p.is_file() {
                return p.canonicalize().ok();
            }
        }
    }
    None
}

/// Delete every downloaded set whose art was fetched under a different mapping.
///
/// Run at startup. Correcting the art table is not enough on its own: the
/// pictures are already on disk, filed under whatever the old table said, and
/// they keep being drawn. The first corrected table filed Iconic's controllers
/// as hardware and its wordmarks as controllers, so the console grid showed a
/// controller under "Hardware" until the folders went.
///
/// Deleting rather than remapping, because the two do not line up: a style the
/// new table has no directory for would be left behind with nothing to replace
/// it. Re-fetching a set costs a few hundred kilobytes.
pub fn drop_stale_sets(media_root: &Path, current: &BTreeMap<String, String>) -> Vec<String> {
    let root = media_root.join("_platforms").join("sets");
    let Ok(entries) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut dropped = Vec::new();
    for e in entries.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        // A set no longer in the table is left alone: it is not wrong, just
        // unrecognized, and deleting a user's pictures on that basis would be
        // presumptuous.
        let Some(want) = current.get(&name) else { continue };
        if set_mapping(media_root, &name).as_deref() != Some(want.as_str())
            && std::fs::remove_dir_all(e.path()).is_ok()
        {
            dropped.push(name);
        }
    }
    dropped
}

/// The mapping a downloaded set was fetched under, if it recorded one.
///
/// Absent means it predates the record, which is itself a mismatch: those are
/// exactly the downloads made under the mapping that filed controllers as
/// hardware.
pub fn set_mapping(media_root: &Path, set: &str) -> Option<String> {
    std::fs::read_to_string(media_root.join("_platforms").join("sets").join(set).join("mapping.txt"))
        .ok()
}

/// Record the mapping a set was fetched under.
pub fn write_set_mapping(media_root: &Path, set: &str, fingerprint: &str) -> std::io::Result<()> {
    let dir = media_root.join("_platforms").join("sets").join(set);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("mapping.txt"), fingerprint)
}

/// Delete a named set's art. The theme checkout is already long gone; this is
/// the few hundred kilobytes of SVG it left behind.
pub fn remove_set(media_root: &Path, set: &str) -> std::io::Result<()> {
    let dir = media_root.join("_platforms").join("sets").join(set);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Install one art style into `_platforms/<style>/`.
///
/// Styles are kept side by side so switching in the UI needs no re-download.
pub fn install_style(
    themes: &[Theme],
    map: &CoreMap,
    slugs: &[String],
    media_root: &Path,
    style: IconStyle,
) -> Result<usize> {
    let dir = media_root.join("_platforms").join(style.key());
    std::fs::create_dir_all(&dir)?;
    let mut n = 0;
    for (slug, src) in art(themes, map, slugs, style) {
        let ext = src
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "svg".into());
        let dst = dir.join(format!("{slug}.{ext}"));
        if std::fs::copy(&src, &dst).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// Installed art for a platform in the requested style, falling back to
/// `Logo` when that style has no art for this system.
pub fn installed_logo(media_root: &Path, slug: &str, style: IconStyle) -> Option<PathBuf> {
    installed_logo_from(media_root, slug, style, "", "")
}

/// The same, restricted to a named set first.
///
/// A set that has no picture for this console falls through to the shared pool
/// rather than leaving a hole in the grid: no ES-DE theme draws every system,
/// and a chosen set going blank for the three consoles it skipped would read as
/// the download having failed.
pub fn installed_logo_from(
    media_root: &Path,
    slug: &str,
    style: IconStyle,
    set: &str,
    look: &str,
) -> Option<PathBuf> {
    let base = media_root.join("_platforms");
    // The chosen look first, then anything else the set downloaded: a theme
    // draws most systems in every look it offers, but not all of them, and a
    // hole in the grid reads as a failed download.
    if !set.is_empty() {
        let sets_dir = media_root.join("_platforms").join("sets").join(set);
        let mut tried: Vec<PathBuf> = Vec::new();
        if !look.is_empty() {
            tried.push(sets_dir.join(look));
        }
        if let Ok(rd) = std::fs::read_dir(&sets_dir) {
            let mut others: Vec<PathBuf> =
                rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            others.sort();
            tried.extend(others);
        }
        for d in tried {
            for ext in ICON_EXTENSIONS {
                let p = d.join(format!("{slug}.{ext}"));
                if p.is_file() {
                    return p.canonicalize().ok();
                }
            }
        }
    }
    // Then the shared pool, which still has the three old kinds.
    let mut styles = vec![style];
    if style != IconStyle::Logo {
        styles.push(IconStyle::Logo);
    }
    for s in styles {
        for ext in ICON_EXTENSIONS {
            let p = base.join(s.key()).join(format!("{slug}.{ext}"));
            if p.is_file() {
                return p.canonicalize().ok();
            }
        }
    }
    None
}

/// How many platforms have art installed for each style.
pub fn installed_counts(media_root: &Path, slugs: &[String]) -> Vec<(IconStyle, usize)> {
    IconStyle::ALL
        .into_iter()
        .map(|style| {
            let base = media_root.join("_platforms").join(style.key());
            let n = slugs
                .iter()
                .filter(|slug| {
                    ICON_EXTENSIONS
                        .iter()
                        .any(|ext| base.join(format!("{slug}.{ext}")).is_file())
                })
                .count();
            (style, n)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch theme tree. `files` are paths relative to the theme root.
    fn theme_with(name: &str, files: &[&str]) -> (PathBuf, Theme) {
        let root = std::env::temp_dir().join(format!("moose-rack-theme-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        for f in files {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"<svg/>").unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        (root.clone(), Theme { name: name.to_owned(), path: root })
    }

    /// Themes disagree about where per-system art lives, and the three
    /// conventions below are all in use by themes people actually have
    /// installed. Reading only the first is how a theme ends up looking as
    /// though it has no icons at all.
    #[test]
    fn art_is_found_under_each_layout_themes_actually_use() {
        // slate-es-de and most community themes.
        let (_, slate) = theme_with("slate", &["snes/images/logo.svg"]);
        assert!(art_for(&slate, "snes", IconStyle::Logo).is_some());

        // linear-es-de.
        let (_, linear) = theme_with("linear", &["system/logos/snes.svg"]);
        assert!(art_for(&linear, "snes", IconStyle::Logo).is_some());

        // canvas-es-de and relatives.
        let (_, canvas) = theme_with("canvas", &["_inc/system-logo/snes.svg"]);
        assert!(art_for(&canvas, "snes", IconStyle::Logo).is_some());
    }

    /// modern-es-de keeps hardware renders in art/ and older ones in
    /// art_legacy/. The classic set used to be a style of its own and no
    /// longer is — one theme in fifty-four ships it, which is not enough to
    /// earn a place in the rotation. It stays as a fallback instead, so a
    /// theme carrying only the older renders still shows the console.
    #[test]
    fn the_classic_renders_stand_in_for_hardware_rather_than_being_their_own_style() {
        let (_, only_legacy) = theme_with("legacy", &["art_legacy/snes.png"]);
        assert!(
            art_for(&only_legacy, "snes", IconStyle::SystemArt).is_some(),
            "a theme with only the older renders must still draw the console"
        );

        // Both present: the current render wins.
        let (_, both) = theme_with("both", &["art/snes.png", "art_legacy/snes.png"]);
        let got = art_for(&both, "snes", IconStyle::SystemArt).unwrap();
        assert!(
            got.components().any(|c| c.as_os_str() == "art"),
            "the current render is preferred over art_legacy, got {}",
            got.display()
        );
    }

    /// Exactly three, in the order the Select button walks them, with styled
    /// text first because it is the kind nearly every theme actually draws.
    #[test]
    fn there_are_three_styles_and_styled_text_leads() {
        assert_eq!(IconStyle::ALL.len(), 3);
        assert_eq!(IconStyle::ALL[0], IconStyle::Logo, "styled text is the default");
        let keys: Vec<&str> = IconStyle::ALL.iter().map(|s| s.key()).collect();
        assert_eq!(keys, ["logo", "controller", "systemart"]);
    }

    /// "Logos" told nobody what they would see. The labels are what the picker
    /// and the Select toast print, and they have to name the picture.
    #[test]
    fn the_labels_say_what_the_picture_is() {
        assert_eq!(IconStyle::Logo.label(), "Styled text");
        assert_eq!(IconStyle::Controller.label(), "Controllers");
        assert_eq!(IconStyle::SystemArt.label(), "Hardware");
    }

    /// A config written before the two styles were dropped still has to load,
    /// and has to keep drawing a picture of the console rather than falling
    /// back to a wordmark.
    /// Whatever is on disk is offered. The list of kinds used to be an enum,
    /// and cutting it from five to three took `consolegame` and
    /// `systemart_legacy` out of the rotation while leaving 24 pictures of each
    /// sitting in the library.
    #[test]
    fn every_pool_folder_with_pictures_is_offered() {
        let media = std::env::temp_dir().join("moose-rack-pool-looks");
        let _ = std::fs::remove_dir_all(&media);
        let base = media.join("_platforms");
        for (folder, files) in [
            ("logo", &["snes", "nes"][..]),
            ("consolegame", &["snes"][..]),
            ("systemart_legacy", &["nes"][..]),
            // Something no build ever wrote — hand-copied, and still offered.
            ("my-own-art", &["snes"][..]),
            // A folder with no picture for any console we have.
            ("empty", &[][..]),
        ] {
            std::fs::create_dir_all(base.join(folder)).unwrap();
            for f in files {
                std::fs::write(base.join(folder).join(format!("{f}.svg")), "x").unwrap();
            }
        }
        // Sets belong to a chosen set and are offered by name, not as pool looks.
        std::fs::create_dir_all(base.join("sets").join("x").join("hardware")).unwrap();

        let slugs = vec!["snes".to_owned(), "nes".to_owned()];
        let got = pool_looks(&media, &slugs);
        assert_eq!(
            got,
            vec![
                ("consolegame".to_owned(), 1),
                ("logo".to_owned(), 2),
                ("my-own-art".to_owned(), 1),
                ("systemart_legacy".to_owned(), 1),
            ],
            "empty folders and sets/ are left out; everything else is offered"
        );
    }

    /// A look is a folder name, found in the chosen set first and the shared
    /// pool second, so a set and the pool can both offer one called `logo`.
    #[test]
    fn a_looks_own_folder_wins_over_the_pool() {
        let media = std::env::temp_dir().join("moose-rack-look-art");
        let _ = std::fs::remove_dir_all(&media);
        let base = media.join("_platforms");
        std::fs::create_dir_all(base.join("logo")).unwrap();
        std::fs::write(base.join("logo").join("snes.svg"), "pool").unwrap();
        let set = base.join("sets").join("meringue-es-de").join("styled-text");
        std::fs::create_dir_all(&set).unwrap();
        std::fs::write(set.join("snes.svg"), "set").unwrap();

        let from_set = look_art(&media, "snes", "meringue-es-de", "styled-text").unwrap();
        assert_eq!(std::fs::read_to_string(from_set).unwrap(), "set");

        // A pool look asked for by name, with a set chosen, still comes from
        // the pool — both are on offer at once.
        let from_pool = look_art(&media, "snes", "meringue-es-de", "logo").unwrap();
        assert_eq!(std::fs::read_to_string(from_pool).unwrap(), "pool");

        // A console the chosen look has no picture for falls back rather than
        // leaving a hole.
        std::fs::write(base.join("logo").join("nes.svg"), "pool-nes").unwrap();
        assert!(look_art(&media, "nes", "meringue-es-de", "styled-text").is_some());
    }

    /// Correcting the art table is not enough: art fetched under the old one
    /// is already on disk, in folders the new table does not write, and it
    /// goes on being drawn. This is what clears it.
    #[test]
    fn a_set_fetched_under_an_old_mapping_is_dropped() {
        let media = std::env::temp_dir().join("moose-rack-stale-sets");
        let _ = std::fs::remove_dir_all(&media);
        let sets = media.join("_platforms").join("sets");

        // Fetched under the mapping that filed controllers as hardware.
        std::fs::create_dir_all(sets.join("iconic-es-de").join("systemart")).unwrap();
        write_set_mapping(&media, "iconic-es-de", "systemart=_inc/systems/system.webp").unwrap();
        // Fetched under the current one.
        std::fs::create_dir_all(sets.join("razor-es-de").join("logo")).unwrap();
        write_set_mapping(&media, "razor-es-de", "logo=system/logos.svg").unwrap();
        // Downloaded before mappings were recorded at all.
        std::fs::create_dir_all(sets.join("meringue-es-de").join("logo")).unwrap();
        // Not in the table — somebody else's, and not ours to delete.
        std::fs::create_dir_all(sets.join("hand-made").join("logo")).unwrap();

        let current = BTreeMap::from([
            ("iconic-es-de".to_owned(), "logo=_inc/systems/carousel-icons.webp".to_owned()),
            ("razor-es-de".to_owned(), "logo=system/logos.svg".to_owned()),
            ("meringue-es-de".to_owned(), "logo=x.svg".to_owned()),
        ]);
        let mut dropped = drop_stale_sets(&media, &current);
        dropped.sort();

        assert_eq!(dropped, ["iconic-es-de", "meringue-es-de"]);
        assert!(!sets.join("iconic-es-de").exists(), "the stale set is gone");
        assert!(sets.join("razor-es-de").exists(), "a current set is kept");
        assert!(sets.join("hand-made").exists(), "an unrecognized set is not ours to delete");
    }

    /// An unstamped download is a stale one: those are exactly the fetches made
    /// before the mapping was recorded, under the table that was wrong.
    #[test]
    fn a_set_with_no_mapping_counts_as_stale() {
        let media = std::env::temp_dir().join("moose-rack-stale-unstamped");
        let _ = std::fs::remove_dir_all(&media);
        std::fs::create_dir_all(media.join("_platforms").join("sets").join("x-es-de")).unwrap();
        assert_eq!(set_mapping(&media, "x-es-de"), None);
        let current = BTreeMap::from([("x-es-de".to_owned(), "logo=a.svg".to_owned())]);
        assert_eq!(drop_stale_sets(&media, &current), ["x-es-de"]);
    }

    #[test]
    fn the_dropped_styles_still_parse_to_hardware() {
        assert_eq!(IconStyle::parse("systemart_legacy"), Some(IconStyle::SystemArt));
        assert_eq!(IconStyle::parse("consolegame"), Some(IconStyle::SystemArt));
        assert_eq!(IconStyle::parse("nonsense"), None);
    }

    /// A style with no picture for a system falls back to the logo, which
    /// every theme has most of. Without that a console grid switched to
    /// "controllers" would be full of holes rather than mixed.
    #[test]
    fn a_missing_style_falls_back_to_the_logo() {
        let media = std::env::temp_dir().join("moose-rack-theme-installed");
        let _ = std::fs::remove_dir_all(&media);
        let base = media.join("_platforms");
        std::fs::create_dir_all(base.join("logo")).unwrap();
        std::fs::write(base.join("logo/snes.svg"), b"<svg/>").unwrap();

        assert!(installed_logo(&media, "snes", IconStyle::Controller).is_some());
        // And a system with nothing at all is still nothing — the fallback is
        // to the logo, not to another system's picture.
        assert!(installed_logo(&media, "dreamcast", IconStyle::Controller).is_none());
    }

    /// The count beside each style in Settings is what tells you a style is
    /// worth switching to. Counting a system that has no file, or missing one
    /// that does, makes that number a lie.
    #[test]
    fn the_installed_count_matches_what_is_on_disk() {
        let media = std::env::temp_dir().join("moose-rack-theme-counts");
        let _ = std::fs::remove_dir_all(&media);
        let base = media.join("_platforms");
        std::fs::create_dir_all(base.join("logo")).unwrap();
        std::fs::create_dir_all(base.join("controller")).unwrap();
        for slug in ["snes", "nes", "gba"] {
            std::fs::write(base.join(format!("logo/{slug}.svg")), b"<svg/>").unwrap();
        }
        std::fs::write(base.join("controller/snes.png"), b"x").unwrap();

        let slugs: Vec<String> =
            ["snes", "nes", "gba", "psx"].iter().map(|s| s.to_string()).collect();
        let counts = installed_counts(&media, &slugs);
        let of = |style: IconStyle| {
            counts.iter().find(|(s, _)| *s == style).map(|(_, n)| *n).unwrap()
        };
        assert_eq!(of(IconStyle::Logo), 3, "three logos were written");
        assert_eq!(of(IconStyle::Controller), 1);
        assert_eq!(of(IconStyle::SystemArt), 0);
    }

    /// Any extension the themes use, not only SVG. Several ship webp, and a
    /// theme whose art is all webp would otherwise read as empty.
    #[test]
    fn art_is_found_whatever_image_format_the_theme_ships() {
        for ext in ICON_EXTENSIONS {
            let (_, t) = theme_with(&format!("fmt-{ext}"), &[&format!("snes/images/logo.{ext}")]);
            assert!(art_for(&t, "snes", IconStyle::Logo).is_some(), "{ext} was not found");
        }
    }

    /// The style keys are written into config.toml and read back, so they have
    /// to survive the round trip — and each has to be distinct, or two styles
    /// would share a directory under `_platforms/`.
    #[test]
    fn every_style_has_a_distinct_key_that_parses_back() {
        let mut seen = std::collections::BTreeSet::new();
        for style in IconStyle::ALL {
            assert!(seen.insert(style.key()), "{} is used twice", style.key());
            assert_eq!(IconStyle::parse(style.key()), Some(style));
        }
        assert_eq!(IconStyle::parse("nonsense"), None);
    }
}

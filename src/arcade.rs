//! Real titles for arcade romsets.
//!
//! RomM names a ROM from its metadata match, and falls back to the filename
//! when nothing matches. For arcade that fallback is the romset short name, so
//! the library shows `kof98`, `samsho4`, `tophuntr` — 63 of 152 Neo Geo games
//! and 345 of 2,413 arcade ones.
//!
//! The DATs already downloaded for core-coverage analysis carry a
//! `<description>` per romset, which is the real title. `tools/arcade_names.py`
//! flattens FBNeo's and MAME's into `data/arcade-names.json`, and this applies
//! it wherever RomM left a bare romset name behind.

use std::collections::BTreeMap;
use std::path::Path;

/// Platforms whose filenames are romset short names.
pub const ARCADE_PLATFORMS: &[&str] = &["arcade", "mame", "neogeoaes"];

/// Load the romset → title map. Absent file is not an error: the names simply
/// stay as they are.
pub fn names(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// True when RomM clearly had no metadata and fell back to the file name.
pub fn is_bare_romset(name: &str, fs_name: &str) -> bool {
    let stem = fs_name.rsplit_once('.').map_or(fs_name, |(s, _)| s);
    name.is_empty() || name.eq_ignore_ascii_case(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This decides which titles get overwritten by the DAT map. Too eager and
    /// it destroys a real title RomM matched correctly; too shy and the library
    /// keeps showing `kof98`.
    #[test]
    fn a_name_matching_its_own_filename_is_a_fallback_not_a_title() {
        assert!(is_bare_romset("kof98", "kof98.zip"));
        assert!(is_bare_romset("", "kof98.zip"), "no name at all");
        // RomM's fallback preserves the stem's case, but a differently-cased
        // match is still the same fallback.
        assert!(is_bare_romset("KOF98", "kof98.zip"));
        // No extension to strip.
        assert!(is_bare_romset("tophuntr", "tophuntr"));
    }

    /// A real title must survive, or the renaming pass corrupts the metadata it
    /// is supposed to be improving.
    #[test]
    fn a_real_title_is_left_alone() {
        assert!(!is_bare_romset("The King of Fighters '98", "kof98.zip"));
        assert!(!is_bare_romset("Metal Slug", "mslug.zip"));
        // A title that merely starts with the stem is still a title.
        assert!(!is_bare_romset("kof98 (Rev A)", "kof98.zip"));
    }

    /// Only the last dot separates the extension: arcade filenames contain
    /// dots, and stripping from the first would compare against a truncated
    /// stem and miss the match.
    #[test]
    fn only_the_final_extension_is_stripped() {
        assert!(is_bare_romset("sf2.ce", "sf2.ce.zip"));
    }
}

//! One id per game, the same on every machine.
//!
//! Games used to be numbered by their position in a scan: the server counted
//! `1, 2, 3` through its library and the desktop counted `-1, -2, -3` through
//! its own. Two numberings of the same games, and neither was stable. Adding
//! one file to a folder shifted every id after it, and the server keeps saves
//! on disk under `<saves>/<rom_id>/`. Checked on dev.lan on 2026-09-17: all
//! seven saves filed under server ids sat under ids that by then named other
//! games. Chrono Trigger's save was filed under a Game Boy Color racer.
//!
//! The id is now derived from where the game is, and nothing else:
//! the ES-DE system folder, the folders inside it, and the file name. Two
//! machines holding the same library in the same layout agree on every id
//! without talking to each other, and a rescan cannot move one.
//!
//! Not platform and file name: that pair is not unique. Measured on this
//! library, `sfc/AdditionalRoms/Public Domain` and `sfc/AdditionalRoms/Homebrew`
//! both hold an `Astrohawk (World) (Unl).zip`.
//!
//! Renaming or moving a file gives it a new id. That matches what every
//! emulator already does with saves, which are named after the ROM file.

use sha1::{Digest, Sha1};
use unicode_normalization::UnicodeNormalization;

/// Largest id a JavaScript number holds exactly. The web UI and the desktop
/// window both pass ids through JSON, and an id above this is silently rounded
/// on the way into the page. That already happened once, to save ids.
pub const JS_SAFE: i64 = (1 << 53) - 1;

/// Every stable id is at least this.
///
/// The positional ids never exceeded the size of a library, so anything smaller
/// in magnitude is an id from before this change: a client on an old build, a
/// save folder on disk, a stored preference. `is_legacy` tells them apart
/// without a table, and it is exact, because `game_id` never returns less.
pub const FLOOR: i64 = 1 << 24;

/// The id of the game at `system/rel_dir/fs_name`.
///
/// `rel_dir` is the path inside the system folder, empty at the top. Either
/// separator is accepted, because a Windows scan produces backslashes and the
/// same library scanned there has to agree with a Linux server.
pub fn game_id(system: &str, rel_dir: &str, fs_name: &str) -> i64 {
    id_of("game", &key(system, rel_dir, fs_name))
}

/// A platform's id, from its slug.
pub fn platform_id(slug: &str) -> i64 {
    id_of("platform", slug)
}

/// True for an id from the positional scheme.
pub fn is_legacy(id: i64) -> bool {
    id.unsigned_abs() < FLOOR as u64
}

/// The string the id is taken from, which is also the thing to print when two
/// games are found to share one.
pub fn key(system: &str, rel_dir: &str, fs_name: &str) -> String {
    let clean = |s: &str| s.replace('\\', "/").trim_matches('/').to_owned();
    let (system, rel_dir) = (clean(system), clean(rel_dir));
    let path = if rel_dir.is_empty() {
        format!("{system}/{fs_name}")
    } else {
        format!("{system}/{rel_dir}/{fs_name}")
    };
    // NFC. macOS hands back file names in whatever form they were written, and
    // a name copied through another tool can arrive decomposed, so `Pokémon`
    // on the SSD and `Pokémon` on the server can be different bytes for the
    // same file.
    path.nfc().collect()
}

fn id_of(kind: &str, text: &str) -> i64 {
    // SHA-1 rather than the standard library's hasher, whose output is allowed
    // to change between Rust releases and would renumber every library on
    // whichever machine upgraded first.
    let mut h = Sha1::new();
    h.update(kind.as_bytes());
    h.update([0u8]);
    h.update(text.as_bytes());
    let digest = h.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    let n = (u64::from_be_bytes(first) as i64) & JS_SAFE;
    if n < FLOOR { n + FLOOR } else { n }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_file_gets_the_same_id_every_time() {
        let a = game_id("snes", "", "Chrono Trigger (USA).sfc");
        assert_eq!(a, game_id("snes", "", "Chrono Trigger (USA).sfc"));
    }

    /// A fixed value, so a change to the derivation fails here rather than
    /// silently renumbering every library and detaching every save.
    #[test]
    fn the_derivation_does_not_change() {
        assert_eq!(game_id("snes", "", "Chrono Trigger (USA).sfc"), 6_738_819_705_334_126);
    }

    #[test]
    fn ids_stay_inside_what_javascript_can_hold() {
        for i in 0..5000 {
            let id = game_id("arcade", "", &format!("game{i}.zip"));
            assert!((FLOOR..=JS_SAFE).contains(&id), "{id}");
            assert!(!is_legacy(id));
        }
    }

    #[test]
    fn the_same_name_in_two_folders_is_two_games() {
        // Both of these exist in the real library.
        assert_ne!(
            game_id("sfc", "AdditionalRoms/Public Domain", "Astrohawk (World) (Unl).zip"),
            game_id("sfc", "AdditionalRoms/Homebrew", "Astrohawk (World) (Unl).zip"),
        );
        assert_ne!(game_id("snes", "", "X.zip"), game_id("sfc", "", "X.zip"));
    }

    #[test]
    fn a_windows_scan_agrees_with_a_linux_one() {
        assert_eq!(
            game_id("sfc", "AdditionalRoms\\Homebrew", "a.zip"),
            game_id("sfc", "AdditionalRoms/Homebrew", "a.zip"),
        );
        assert_eq!(game_id("sfc", "/Homebrew/", "a.zip"), game_id("sfc", "Homebrew", "a.zip"));
    }

    #[test]
    fn a_decomposed_name_is_the_same_file() {
        let composed = "Pok\u{e9}mon Red (USA).zip";
        let decomposed = "Poke\u{301}mon Red (USA).zip";
        assert_ne!(composed, decomposed);
        assert_eq!(game_id("gb", "", composed), game_id("gb", "", decomposed));
    }

    #[test]
    fn every_old_id_is_recognised_as_old() {
        for old in [1, 42, 11_062, -1, -10_793, -12_000] {
            assert!(is_legacy(old), "{old}");
        }
    }

    #[test]
    fn a_game_and_a_platform_with_the_same_name_do_not_collide() {
        assert_ne!(platform_id("snes"), id_of("game", "snes"));
    }
}

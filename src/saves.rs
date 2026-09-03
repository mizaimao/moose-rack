//! Stage 5a — scan local save/state files into `ClientSaveState` candidates.
//!
//! Layout assumed is RetroArch's "sort by core" option:
//!
//! ```text
//! <root>/saves/<Core Display Name>/<rom basename>.srm
//! <root>/states/<Core Display Name>/<rom basename>.state[N][.auto]
//! ```
//!
//! Two things here are load-bearing and easy to get wrong:
//!
//! * **Slot names must be stable and non-null.** Saves pair on
//!   `(rom_id, slot)`, and a null slot is treated as an archival upload that
//!   *always* negotiates as `upload` — so unstable slots accumulate duplicates
//!   without bound.
//! * **`rom_id` resolution must be platform-scoped.** This library has
//!   `nes`+`famicom`, `snes`+`sfc` and `arcade`+`mame` as separate platforms
//!   with overlapping filenames; `airattck.zip` genuinely exists in both
//!   `arcade` and `mame`. Ambiguity is reported, never guessed.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cache::Cache;
use crate::coremap::CoreMap;
use crate::platform::SaveLayout;
use crate::savehash;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Kind {
    Save,
    State,
}

// Several fields exist for the negotiate payload (stage 5b) rather than for
// today's `scan` output; ClientSaveState needs size and content_hash.
#[allow(dead_code)]
#[derive(Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub kind: Kind,
    /// Core display name as it appeared in the directory, e.g. `"melonDS DS"`.
    pub core_dir: String,
    /// Resolved libretro core stem, e.g. `"melondsds"`.
    pub core: Option<String>,
    /// ROM basename the emulator derived the save name from.
    pub rom_base: String,
    pub slot: String,
    pub size: u64,
    pub content_hash: String,
    pub resolution: Resolution,
    /// False when another file claims the same `(rom_id, slot)` and comes from
    /// a more current core. Saves pair on `(rom_id, slot)`, so syncing both
    /// would have them overwrite each other on every run.
    pub canonical: bool,
    /// Set on non-canonical entries: the core that won.
    pub superseded_by: Option<String>,
}

impl Candidate {
    /// The RomM platform this resolved to, when it resolved at all.
    pub fn platform(&self) -> Option<&str> {
        match &self.resolution {
            Resolution::Resolved { platform, .. } => Some(platform),
            _ => None,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum Resolution {
    /// Exactly one ROM matched, on one platform.
    Resolved { rom_id: i64, platform: String, fs_name: String },
    /// The same filename exists on more than one candidate platform.
    Ambiguous(Vec<(i64, String, String)>),
    /// No ROM matched — orphan save, or the ROM is not on the server.
    Unmatched,
    /// The core directory did not map to a known libretro core.
    UnknownCore,
}

/// Normalize a core display name for matching: lowercase, alphanumerics only.
/// RetroArch's desktop display names differ from ES-DE's labels — `"MAME"` vs
/// `"MAME - Current"`, `"melonDS DS"` vs `melondsds` — so compare on a reduced
/// form rather than exact strings.
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Files that live in the save tree but are not saves.
fn is_ignorable(rel: &Path, file_name: &str) -> bool {
    // MAME writes its own config under <core>/mame/cfg/.
    if rel.components().any(|c| c.as_os_str() == "cfg") {
        return true;
    }
    // PCSX2 memory-card internals.
    if file_name.starts_with("_pcsx2") || file_name.starts_with('.') {
        return true;
    }
    matches!(
        Path::new(file_name)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("cfg" | "ini" | "txt" | "ico" | "icn" | "sys")
    )
}

/// Does this filename belong in the `states` tree rather than `saves`?
///
/// Shares its rule with [`split_slot`] below, which is the authority on the
/// naming: anything carrying `.state` is a save state, everything else
/// (`.srm`, `.sav`) is a game save. A download filed under the wrong one is
/// invisible to the next scan.
pub fn is_state_name(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().contains(".state")
}

/// Split a save filename into (rom basename, slot).
///
/// RetroArch conventions, plus what this collection actually contains:
/// `.srm`/`.sav` are game saves; `.state` is manual slot 0; `.stateN` is
/// slot N; `.state.auto` is the autosave slot. Opera writes `<name>.0.srm`
/// under `per_game/`.
fn split_slot(file_name: &str) -> Option<(String, String)> {
    let lower = file_name.to_ascii_lowercase();

    if let Some(stem) = lower.strip_suffix(".state.auto") {
        return Some((file_name[..stem.len()].to_owned(), "auto".into()));
    }
    if let Some(pos) = lower.rfind(".state") {
        let suffix = &lower[pos + ".state".len()..];
        let slot = if suffix.is_empty() {
            "slot0".to_owned()
        } else if suffix.chars().all(|c| c.is_ascii_digit()) {
            format!("slot{suffix}")
        } else {
            return None;
        };
        return Some((file_name[..pos].to_owned(), slot));
    }
    for ext in [".srm", ".sav"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            // Opera: "<name>.0.srm" — drop the trailing disc/save index.
            let base = &file_name[..stem.len()];
            let base = base
                .rsplit_once('.')
                .filter(|(_, tail)| tail.chars().all(|c| c.is_ascii_digit()))
                .map(|(head, _)| head)
                .unwrap_or(base);
            return Some((base.to_owned(), "autosave".into()));
        }
    }
    None
}

/// Candidate platforms for a core stem, from the ES-DE core map.
fn platforms_for_core(map: &CoreMap, core: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (platform, default) in &map.default_core_by_server_platform {
        if default == core && !out.contains(platform) {
            out.push(platform.clone());
        }
    }
    for platform in map.platforms_with_core(core) {
        if !out.contains(&platform) {
            out.push(platform);
        }
    }
    out
}

/// A `saves/<system>/` folder, on a device that files by platform.
///
/// Returns the core to record and the platforms to resolve against. The core
/// is **not in the path** on this layout — configgen aims RetroArch at a
/// per-system folder and the core it picked is nowhere on disk. So the answer
/// is the core this device would run for that system, which is right unless a
/// single game has been given a different core by hand in EmulationStation.
/// That case is undetectable from the filesystem; it is a known limit, not an
/// oversight.
fn system_folder(map: &CoreMap, folder: &str) -> (Option<String>, Vec<String>) {
    // Device spellings first: Batocera calls the whole arcade library `fbneo`,
    // and RomM calls it `arcade`.
    let device = crate::platform::current();
    let slug = device
        .system_aliases()
        .iter()
        .find(|(dir, _)| dir.eq_ignore_ascii_case(folder))
        .map(|(_, slug)| (*slug).to_string())
        .unwrap_or_else(|| folder.to_string());

    // What this hardware runs, where it differs from the shipped default —
    // the Flip runs pcsx_rearmed for PSX, not SwanStation.
    let core = device
        .default_cores()
        .iter()
        .find(|(platform, _)| platform.eq_ignore_ascii_case(&slug))
        .map(|(_, core)| (*core).to_string())
        .or_else(|| map.default_core(&slug).map(str::to_string));

    // Every platform this folder collects, not just the one it is named
    // after: KNULLI keeps Super Famicom in `snes`, and a save searched for
    // under `snes` alone comes back unmatched.
    let platforms = device.platforms_in_folder(&slug);
    (core, platforms)
}

fn resolve(cache: &Cache, platforms: &[String], rom_base: &str) -> Result<Resolution> {
    let mut hits: Vec<(i64, String, String)> = Vec::new();
    for platform in platforms {
        for rom in cache.roms_for(platform)? {
            let stem = Path::new(&rom.fs_name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&rom.fs_name);
            if stem.eq_ignore_ascii_case(rom_base) {
                hits.push((rom.id, platform.clone(), rom.fs_name.clone()));
            }
        }
    }
    Ok(match hits.len() {
        0 => Resolution::Unmatched,
        1 => {
            let (rom_id, platform, fs_name) = hits.pop().unwrap();
            Resolution::Resolved { rom_id, platform, fs_name }
        }
        _ => Resolution::Ambiguous(hits),
    })
}

/// Walk `<root>/saves` and `<root>/states`.
pub fn scan(root: &Path, cache: &Cache, map: &CoreMap) -> Result<Vec<Candidate>> {
    scan_matching(root, cache, map, &|_| true)
}

/// The device's own layout, unless a caller says otherwise.
fn layout_here() -> SaveLayout {
    crate::platform::current().save_layout()
}

/// As [`scan`], considering only saves whose ROM basename passes `keep`.
///
/// Automatic sync runs either side of every launch, and a full scan hashes the
/// contents of every save in the tree — which is the expensive part and grows
/// with the library. Filtering before hashing makes a per-game sync proportional
/// to that game rather than to everything ever played.
///
/// The filter is on the ROM basename rather than the file, so every core's copy
/// of the same game is still seen together and [`mark_canonical`] can resolve a
/// collision between them.
pub fn scan_matching(
    root: &Path,
    cache: &Cache,
    map: &CoreMap,
    keep: &dyn Fn(&str) -> bool,
) -> Result<Vec<Candidate>> {
    scan_matching_with(root, cache, map, keep, layout_here())
}

/// As [`scan_matching`], against a stated layout rather than this machine's.
///
/// Both shapes exist on real devices and a scanner that only knows one finds
/// nothing on the other, so both are exercised by the tests.
pub fn scan_matching_with(
    root: &Path,
    cache: &Cache,
    map: &CoreMap,
    keep: &dyn Fn(&str) -> bool,
    layout: SaveLayout,
) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for (sub, kind) in [("saves", Kind::Save), ("states", Kind::State)] {
        let base = root.join(sub);
        if !base.is_dir() {
            continue;
        }
        collect(&base, &base, kind, cache, map, keep, layout, &mut out)?;
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    mark_canonical(&mut out, map);
    Ok(out)
}

/// Saves belonging to one ROM, matched on its file stem the way an emulator
/// derives a save name from the content it loaded.
pub fn scan_for_stem(
    root: &Path,
    cache: &Cache,
    map: &CoreMap,
    stem: &str,
) -> Result<Vec<Candidate>> {
    scan_matching(root, cache, map, &|base| base.eq_ignore_ascii_case(stem))
}

/// Resolve `(rom_id, slot)` collisions between cores.
///
/// The same game saved under two cores maps to one server-side pairing — this
/// collection has Castlevania under both `melonDS` and `melonDS DS`. The
/// platform's default core wins, because that is what ES-DE launches and
/// therefore what future saves will come from. The loser is kept in the scan
/// output but flagged, never silently dropped.
fn mark_canonical(candidates: &mut [Candidate], map: &CoreMap) {
    use std::collections::HashMap;

    // (rom_id, slot, kind) -> index of the current best candidate.
    let mut best: HashMap<(i64, String, bool), usize> = HashMap::new();
    let mut losers: Vec<(usize, String)> = Vec::new();

    for i in 0..candidates.len() {
        let Resolution::Resolved { rom_id, platform, .. } = &candidates[i].resolution else {
            continue;
        };
        let key = (*rom_id, candidates[i].slot.clone(), candidates[i].kind == Kind::Save);
        let is_default = map.default_core(platform) == candidates[i].core.as_deref();

        match best.get(&key).copied() {
            None => {
                best.insert(key, i);
            }
            Some(prev) => {
                let prev_is_default = match &candidates[prev].resolution {
                    Resolution::Resolved { platform, .. } => {
                        map.default_core(platform) == candidates[prev].core.as_deref()
                    }
                    _ => false,
                };
                // Only displace the incumbent if it is not already the default.
                if is_default && !prev_is_default {
                    losers.push((prev, candidates[i].core.clone().unwrap_or_default()));
                    best.insert(key, i);
                } else {
                    losers.push((i, candidates[prev].core.clone().unwrap_or_default()));
                }
            }
        }
    }

    for (idx, winner) in losers {
        candidates[idx].canonical = false;
        candidates[idx].superseded_by = Some(winner);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect(
    base: &Path,
    dir: &Path,
    dir_kind: Kind,
    cache: &Cache,
    map: &CoreMap,
    keep: &dyn Fn(&str) -> bool,
    layout: SaveLayout,
    out: &mut Vec<Candidate>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(base, &path, dir_kind, cache, map, keep, layout, out)?;
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let rel = path.strip_prefix(base).unwrap_or(&path);
        if is_ignorable(rel, &file_name) {
            continue;
        }
        let Some((rom_base, slot)) = split_slot(&file_name) else {
            continue;
        };
        // Before hashing: the digest is the expensive part of a scan.
        if !keep(&rom_base) {
            continue;
        }

        // What the first component under saves/ or states/ actually means
        // depends on the device — see `SaveLayout`.
        let first = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_default();

        // And so does what kind of file this is. RetroArch keeps states in
        // their own tree, so the folder settles it; Batocera puts both in one
        // folder, so only the name can.
        let kind = match layout {
            SaveLayout::ByCore => dir_kind,
            SaveLayout::BySystem => {
                if is_state_name(&file_name) {
                    Kind::State
                } else {
                    Kind::Save
                }
            }
        };

        let (core, platforms) = match layout {
            SaveLayout::ByCore => {
                let core = map.core_by_display_name(&first, normalize);
                let platforms = core
                    .as_deref()
                    .map(|c| platforms_for_core(map, c))
                    .unwrap_or_default();
                (core, platforms)
            }
            SaveLayout::BySystem => system_folder(map, &first),
        };

        let resolution = if core.is_none() {
            Resolution::UnknownCore
        } else if platforms.is_empty() {
            Resolution::Unmatched
        } else {
            resolve(cache, &platforms, &rom_base)?
        };

        let core_dir = first;
        out.push(Candidate {
            size: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            content_hash: savehash::compute(&path)?,
            path,
            kind,
            core_dir,
            core,
            rom_base,
            slot,
            resolution,
            canonical: true,
            superseded_by: None,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Game saves. The slot is a constant because a `.srm` has no slot of
    /// its own, and it must still never be empty — see the module docs on what
    /// a null slot does to the server's pairing.
    #[test]
    fn game_saves_split_into_a_basename_and_a_stable_slot() {
        assert_eq!(
            split_slot("Chrono Trigger.srm"),
            Some(("Chrono Trigger".to_owned(), "autosave".to_owned()))
        );
        assert_eq!(
            split_slot("Zelda.sav"),
            Some(("Zelda".to_owned(), "autosave".to_owned()))
        );
        // The extension match is case-insensitive, but the basename keeps the
        // case it had — it is matched against the ROM filename later.
        assert_eq!(
            split_slot("ZELDA.SRM"),
            Some(("ZELDA".to_owned(), "autosave".to_owned()))
        );
    }

    /// `.state` is slot 0, `.stateN` is slot N, `.state.auto` is the autosave.
    /// Getting the numbering wrong pairs a save against the wrong server slot.
    #[test]
    fn save_states_carry_their_slot_number() {
        assert_eq!(
            split_slot("Metroid.state"),
            Some(("Metroid".to_owned(), "slot0".to_owned()))
        );
        assert_eq!(
            split_slot("Metroid.state3"),
            Some(("Metroid".to_owned(), "slot3".to_owned()))
        );
        // Two digits must not be truncated to one.
        assert_eq!(
            split_slot("Metroid.state10"),
            Some(("Metroid".to_owned(), "slot10".to_owned()))
        );
        assert_eq!(
            split_slot("Metroid.state.auto"),
            Some(("Metroid".to_owned(), "auto".to_owned()))
        );
    }

    /// Opera writes `<name>.0.srm`. That trailing index is a disc number, not
    /// part of the game's name, and leaving it on means the save never matches
    /// the ROM it belongs to.
    #[test]
    fn an_opera_disc_index_is_not_part_of_the_basename() {
        assert_eq!(
            split_slot("Lemmings.0.srm"),
            Some(("Lemmings".to_owned(), "autosave".to_owned()))
        );
        // A dotted segment that is not numeric is part of the name and stays.
        assert_eq!(
            split_slot("Sonic.CD.srm"),
            Some(("Sonic.CD".to_owned(), "autosave".to_owned()))
        );
    }

    /// A game whose title contains dots or brackets must survive intact, since
    /// the basename is what gets matched against the ROM filename.
    #[test]
    fn punctuation_in_a_title_survives_the_split() {
        assert_eq!(
            split_slot("Final Fantasy VII (USA) (Disc 1).state2"),
            Some(("Final Fantasy VII (USA) (Disc 1)".to_owned(), "slot2".to_owned()))
        );
    }

    /// Anything unrecognized is skipped rather than guessed at. A wrong slot is
    /// worse than no candidate: it pairs against something on the server.
    #[test]
    fn unrecognized_names_produce_no_candidate() {
        for name in ["notes.txt", "Game.stateX", "Game.state-1", "Game", "retroarch.cfg"] {
            assert_eq!(split_slot(name), None, "{name} should not parse as a save");
        }
    }

    /// Whatever `split_slot` accepts, the slot it returns must be non-empty and
    /// deterministic. Unstable slots accumulate duplicates on the server
    /// without bound, which is the failure this guards.
    #[test]
    fn every_recognized_save_gets_a_non_empty_stable_slot() {
        let names = [
            "A.srm", "A.sav", "A.state", "A.state0", "A.state9", "A.state.auto", "A.0.srm",
        ];
        for name in names {
            let (_, slot) = split_slot(name).unwrap_or_else(|| panic!("{name} should parse"));
            assert!(!slot.is_empty(), "{name} produced an empty slot");
            assert_eq!(split_slot(name).unwrap().1, slot, "{name} must be deterministic");
        }
    }

    /// The states/saves split has to agree with `split_slot`, or a downloaded
    /// file lands in the tree the next scan does not look in and is lost.
    #[test]
    fn the_state_test_agrees_with_the_slot_parser() {
        for name in ["Game.state", "Game.state5", "Game.state.auto", "GAME.STATE"] {
            assert!(is_state_name(name), "{name} belongs in states/");
        }
        for name in ["Game.srm", "Game.sav", "Game.0.srm"] {
            assert!(!is_state_name(name), "{name} belongs in saves/");
        }
    }

    /// The save tree also holds files that are not saves. Uploading a MAME
    /// config or a PCSX2 memory-card internal as if it were one is noise at
    /// best and a wrong restore at worst.
    #[test]
    fn non_save_files_in_the_tree_are_ignored() {
        assert!(is_ignorable(Path::new("MAME/mame/cfg/default.cfg"), "default.cfg"));
        assert!(is_ignorable(Path::new("PCSX2/_pcsx2_lastsave"), "_pcsx2_lastsave"));
        assert!(is_ignorable(Path::new("x/.DS_Store"), ".DS_Store"));
        assert!(is_ignorable(Path::new("x/settings.ini"), "settings.ini"));
        assert!(is_ignorable(Path::new("x/readme.TXT"), "readme.TXT"));

        assert!(!is_ignorable(Path::new("Snes9x/Zelda.srm"), "Zelda.srm"));
        assert!(!is_ignorable(Path::new("Snes9x/Zelda.state1"), "Zelda.state1"));
    }

    /// RetroArch's save directories are named after the core's *display* name,
    /// which never matches the libretro stem exactly. Comparison happens on a
    /// reduced form of both.
    #[test]
    fn core_directory_names_reduce_to_a_comparable_form() {
        assert_eq!(normalize("MAME - Current"), "mamecurrent");
        assert_eq!(normalize("melonDS DS"), "melondsds");
        assert_eq!(normalize("Snes9x"), "snes9x");
        assert_eq!(normalize("Beetle PSX HW"), "beetlepsxhw");
    }

    fn map() -> CoreMap {
        serde_json::from_str(
            r#"{
              "default_core_by_server_platform": {"nds": "melondsds"},
              "systems": {"nds": {
                "server_platforms": ["nds"],
                "emulators": [
                  {"label": "melonDS DS", "kind": "libretro", "core": "melondsds"},
                  {"label": "melonDS",    "kind": "libretro", "core": "melonds"}
                ]}}
            }"#,
        )
        .unwrap()
    }

    fn candidate(core: &str, slot: &str, kind: Kind, rom_id: i64) -> Candidate {
        Candidate {
            path: PathBuf::from(format!("/ra/saves/{core}/Game.srm")),
            kind,
            core_dir: core.to_owned(),
            core: Some(core.to_owned()),
            rom_base: "Game".to_owned(),
            slot: slot.to_owned(),
            size: 1,
            content_hash: "h".to_owned(),
            resolution: Resolution::Resolved {
                rom_id,
                platform: "nds".to_owned(),
                fs_name: "Game.nds".to_owned(),
            },
            canonical: true,
            superseded_by: None,
        }
    }

    /// The real collision in this library: the same game saved under both
    /// `melonDS` and `melonDS DS`. They map to one server-side pairing, so
    /// syncing both would have them overwrite each other on every single run.
    /// The platform's default core wins, because that is what future saves will
    /// come from.
    #[test]
    fn the_default_core_wins_a_slot_collision() {
        let mut c = vec![
            candidate("melonds", "autosave", Kind::Save, 1),
            candidate("melondsds", "autosave", Kind::Save, 1),
        ];
        mark_canonical(&mut c, &map());

        assert!(!c[0].canonical, "the non-default core loses");
        assert_eq!(c[0].superseded_by.as_deref(), Some("melondsds"));
        assert!(c[1].canonical, "the platform default wins");
        assert!(c[1].superseded_by.is_none());
    }

    /// Order must not decide the winner. Listing the default first previously
    /// left it beaten by whatever came after it.
    #[test]
    fn the_default_still_wins_when_it_is_seen_first() {
        let mut c = vec![
            candidate("melondsds", "autosave", Kind::Save, 1),
            candidate("melonds", "autosave", Kind::Save, 1),
        ];
        mark_canonical(&mut c, &map());

        assert!(c[0].canonical, "the default must not be displaced by a later entry");
        assert!(!c[1].canonical);
        assert_eq!(c[1].superseded_by.as_deref(), Some("melondsds"));
    }

    /// The loser is flagged, never dropped. `moose-rack scan` reports it so
    /// the user can see why a save is not being synced.
    #[test]
    fn a_superseded_save_is_kept_and_explained() {
        let mut c = vec![
            candidate("melonds", "autosave", Kind::Save, 1),
            candidate("melondsds", "autosave", Kind::Save, 1),
        ];
        mark_canonical(&mut c, &map());
        assert_eq!(c.len(), 2, "nothing is silently discarded");
        assert!(c.iter().any(|x| !x.canonical && x.superseded_by.is_some()));
    }

    /// Different slots, different games and the save/state split are all
    /// separate pairings — collapsing any of them would discard a real save.
    #[test]
    fn only_a_genuine_collision_is_treated_as_one() {
        let mut c = vec![
            candidate("melonds", "slot1", Kind::State, 1),
            candidate("melondsds", "slot2", Kind::State, 1),
            candidate("melondsds", "slot1", Kind::State, 2),
            // Same rom, same slot name, but a game save rather than a state.
            candidate("melonds", "slot1", Kind::Save, 1),
        ];
        mark_canonical(&mut c, &map());
        assert!(
            c.iter().all(|x| x.canonical),
            "none of these share a (rom, slot, kind) pairing"
        );
    }

    /// An unresolved save has no rom_id to pair on, so it cannot collide with
    /// anything and must be left alone rather than compared.
    #[test]
    fn unresolved_saves_are_not_drawn_into_collision_handling() {
        let mut unmatched = candidate("melonds", "autosave", Kind::Save, 1);
        unmatched.resolution = Resolution::Unmatched;
        let mut c = vec![unmatched, candidate("melondsds", "autosave", Kind::Save, 1)];
        mark_canonical(&mut c, &map());
        assert!(c.iter().all(|x| x.canonical));
    }

    /// A core's platforms come from the map, and the default mapping is
    /// consulted as well as the alternatives list — a core that is only ever a
    /// default would otherwise resolve to no platforms and match nothing.
    #[test]
    fn a_cores_platforms_include_both_defaults_and_alternatives() {
        let m = map();
        assert_eq!(platforms_for_core(&m, "melondsds"), ["nds"]);
        assert_eq!(platforms_for_core(&m, "melonds"), ["nds"], "an alternative counts too");
        assert!(platforms_for_core(&m, "nothing").is_empty());
    }
}

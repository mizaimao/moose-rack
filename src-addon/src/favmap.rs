//! Which game on the card is which game on the server.
//!
//! The server knows a game by a number. EmulationStation knows it by a path.
//! Everything in [`crate::favsync`] works in numbers, so something has to hold
//! the two together, and this is it.
//!
//! The join is the path inside the system folder, which is safe here for the
//! same reason it is safe for saves: these ROMs were copied from one source,
//! so the same game has the same file name and the same subfolder everywhere.
//! The subfolder matters: `snes/Aftermarket/` sits beside `snes/`, and the
//! library holds files of one name in both. What is *not* the same is the
//! system folder — the server files SNES under `sfc` and the handheld under
//! `snes` — so the platform's own folder mapping does that half.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use moose_rack::cache::Cache;
use moose_rack::platform::Platform;

/// A game the server knows about that is also on this card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    pub rom_id: i64,
    /// The folder under the ROMs root — `snes`, not `sfc`.
    pub folder: String,
    /// The folder inside that, `""` at the top: `Aftermarket`.
    pub rel_dir: String,
    /// The file in it, as ES names it.
    pub file: String,
}

impl Known {
    /// Where the game is inside its system folder, as ES writes it in a
    /// gamelist's `<path>` (without the `./`).
    pub fn rel_path(&self) -> String {
        if self.rel_dir.is_empty() {
            self.file.clone()
        } else {
            format!("{}/{}", self.rel_dir, self.file)
        }
    }

    pub fn full_path(&self, roms_root: &Path) -> PathBuf {
        roms_root.join(&self.folder).join(self.rel_path())
    }
}

/// A server's `rel_dir`, if it is a plain path of folder names.
///
/// It is joined onto the card, so `..`, a root or a drive would reach outside
/// the system folder. A row carrying one is left off the card rather than
/// followed there.
fn plain_rel_dir(rel_dir: &str) -> Option<String> {
    let parts: Vec<&str> = rel_dir.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    parts
        .iter()
        .all(|p| *p != "." && *p != ".." && !p.contains(':'))
        .then(|| parts.join("/"))
}

/// Every game the cache knows that is actually sitting on this card.
///
/// Checked against the filesystem rather than trusted from the cache: the
/// card holds a subset of the library, and a star can only be set on a game
/// ES can see. Games that are not here are simply absent from the result,
/// which is what stops [`crate::favsync::reconcile`] reading them as
/// unstarred.
pub fn on_card(cache: &Cache, platform: &dyn Platform, roms_root: &Path) -> anyhow::Result<Vec<Known>> {
    let mut out = Vec::new();
    // One directory listing per folder, not one `exists()` per game: an exFAT
    // card with nine thousand arcade ROMs makes the second unbearable.
    let mut listed: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for rom in cache.all_roms()? {
        let folder = platform.save_folder(&rom.platform_slug);
        let Some(rel_dir) = plain_rel_dir(&rom.rel_dir) else { continue };
        let here = listed.entry((folder.clone(), rel_dir.clone())).or_insert_with(|| {
            std::fs::read_dir(roms_root.join(&folder).join(&rel_dir))
                .map(|d| {
                    d.filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        });
        if let Some(file) = as_named_on_card(here, &rom.fs_name) {
            out.push(Known { rom_id: rom.id, folder, rel_dir, file });
        }
    }
    Ok(out)
}

/// What the card calls a game the server calls `fs_name`.
///
/// Usually the same thing. Multi-disc games are not: RomM holds one rom named
/// `Final Fantasy VII (USA)` with the discs inside it, and Batocera files the
/// discs in a *hidden* `.Final Fantasy VII (USA)/` folder with a playlist
/// beside it. What ES shows, and therefore what ES stars, is the playlist.
///
/// Matching only the plain name silently skipped every multi-disc game — they
/// were starred on both sides and read as being on neither, so unstarring one
/// on the handheld would never have travelled.
fn as_named_on_card(here: &BTreeSet<String>, fs_name: &str) -> Option<String> {
    if here.contains(fs_name) {
        return Some(fs_name.to_owned());
    }
    let playlist = format!("{fs_name}.m3u");
    here.contains(&playlist).then_some(playlist)
}

/// The games on this card, by id — what `reconcile` needs for `known`.
pub fn ids(known: &[Known]) -> BTreeSet<i64> {
    known.iter().map(|k| k.rom_id).collect()
}

/// Card games grouped by the folder they live in.
pub fn by_folder(known: &[Known]) -> BTreeMap<String, Vec<Known>> {
    let mut out: BTreeMap<String, Vec<Known>> = BTreeMap::new();
    for k in known {
        out.entry(k.folder.clone()).or_default().push(k.clone());
    }
    out
}

/// Look a game up by the system folder and the path inside it that ES named
/// it with. Two games of one file name in `snes/` and `snes/Aftermarket/` are
/// two keys.
pub fn by_file(known: &[Known]) -> BTreeMap<(String, String), i64> {
    known
        .iter()
        .map(|k| ((k.folder.clone(), k.rel_path()), k.rom_id))
        .collect()
}

/// Where ES keeps its two kinds of list.
#[derive(Clone, Debug)]
pub struct EsPaths {
    pub roms: PathBuf,
    pub collections: PathBuf,
    pub settings: PathBuf,
}

impl EsPaths {
    /// The KNULLI layout.
    pub fn knulli() -> Self {
        Self::under(Path::new("/userdata"))
    }

    /// The same layout under some other root, which is what the tests use.
    pub fn under(userdata: &Path) -> Self {
        let es = userdata.join("system/configs/emulationstation");
        Self {
            roms: userdata.join("roms"),
            collections: es.join("collections"),
            settings: es.join("es_settings.cfg"),
        }
    }

    /// One system's gamelist.
    pub fn gamelist(&self, folder: &str) -> PathBuf {
        self.roms.join(folder).join("gamelist.xml")
    }

    /// One collection's membership file.
    pub fn collection(&self, name: &str) -> PathBuf {
        self.collections.join(crate::eslist::CollectionFile::file_name(name))
    }

    /// `(system folder, path inside it)` for an absolute ROM path from a
    /// collection file, or `None` for one outside the ROMs folder.
    pub fn locate(&self, rom: &Path) -> Option<(String, String)> {
        let rest = rom.strip_prefix(&self.roms).ok()?;
        let mut parts = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
        let folder = parts.next()?;
        let inside = parts.collect::<Vec<_>>().join("/");
        (!inside.is_empty()).then_some((folder, inside))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(folders: &[(&str, &[&str])]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-favmap-{}", folders.len()));
        let _ = std::fs::remove_dir_all(&dir);
        for (folder, files) in folders {
            let d = dir.join("roms").join(folder);
            std::fs::create_dir_all(&d).unwrap();
            for f in *files {
                std::fs::write(d.join(f), b"rom").unwrap();
            }
        }
        dir
    }

    #[test]
    fn the_server_folder_and_the_card_folder_are_not_the_same_name() {
        // The server files SNES under `sfc`; the handheld under `snes`. Get
        // this wrong and every SNES star silently matches nothing.
        let p = moose_rack::platform::knulli::Knulli;
        assert_eq!(p.save_folder("sfc"), "snes");
        assert_eq!(p.save_folder("famicom"), "nes");
        assert_eq!(p.save_folder("gb"), "gb");
    }

    #[test]
    fn a_game_is_only_known_if_it_is_really_on_the_card() {
        let root = card(&[("snes", &["Chrono Trigger (USA).sfc"])]);
        let paths = EsPaths::under(&root);
        assert!(paths.roms.join("snes/Chrono Trigger (USA).sfc").exists());
        assert!(!paths.roms.join("snes/Secret of Mana (USA).sfc").exists());
    }

    #[test]
    fn a_multi_disc_game_is_found_by_the_playlist_es_actually_shows() {
        // The server has one rom, `Final Fantasy VII (USA)`. The card has a
        // hidden folder of discs and a playlist beside it, and the playlist is
        // what ES lists and what ES stars.
        let here: BTreeSet<String> = [
            "Final Fantasy VII (USA).m3u".to_owned(),
            "Chrono Trigger (USA).sfc".to_owned(),
        ]
        .into();
        assert_eq!(
            as_named_on_card(&here, "Final Fantasy VII (USA)").as_deref(),
            Some("Final Fantasy VII (USA).m3u")
        );
        // and a single-file game is still matched as itself
        assert_eq!(
            as_named_on_card(&here, "Chrono Trigger (USA).sfc").as_deref(),
            Some("Chrono Trigger (USA).sfc")
        );
        // something genuinely absent stays absent
        assert_eq!(as_named_on_card(&here, "Tony Hawks Pro Skater 2 (USA).chd"), None);
    }

    #[test]
    fn the_hidden_disc_folder_is_never_what_gets_starred() {
        // `.Final Fantasy VII (USA)` is on the card too, and ES hides it. A
        // star written against the folder would sit in the gamelist doing
        // nothing.
        let here: BTreeSet<String> = [
            ".Final Fantasy VII (USA)".to_owned(),
            "Final Fantasy VII (USA).m3u".to_owned(),
        ]
        .into();
        assert_eq!(
            as_named_on_card(&here, "Final Fantasy VII (USA)").as_deref(),
            Some("Final Fantasy VII (USA).m3u")
        );
    }

    /// `snes/Aftermarket/` holds games too, and the library has file names in
    /// both it and `snes/`. Matching on the name alone never found the
    /// subfolder games, and took a twin at the top for the one below.
    #[test]
    fn a_game_in_a_subfolder_and_its_twin_at_the_top_are_two_games() {
        let dir = std::env::temp_dir().join("moose-favmap-subfolders");
        let _ = std::fs::remove_dir_all(&dir);
        let roms = dir.join("roms");
        std::fs::create_dir_all(roms.join("snes/Aftermarket")).unwrap();
        for f in ["snes/Foo (USA).sfc", "snes/Aftermarket/Foo (USA).sfc", "snes/Aftermarket/Bar (USA).sfc"] {
            std::fs::write(roms.join(f), b"rom").unwrap();
        }
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(dir.join("etc/Foo (USA).sfc"), b"not a rom").unwrap();
        let game = |rel_dir: &str, fs_name: &str| moose_rack::esde::Game {
            platform_slug: "sfc".into(),
            system: "snes".into(),
            name: fs_name.into(),
            fs_name: fs_name.into(),
            rel_dir: rel_dir.into(),
            ..Default::default()
        };
        let mut cache = Cache::open(&dir.join("cache.sqlite3")).unwrap();
        cache
            .replace_from_esde(&[
                game("", "Foo (USA).sfc"),
                game("Aftermarket", "Foo (USA).sfc"),
                game("Aftermarket", "Bar (USA).sfc"),
                // Not on this card, and a path that would leave the folder.
                game("../../etc", "Foo (USA).sfc"),
            ])
            .unwrap();
        let known = on_card(&cache, &moose_rack::platform::knulli::Knulli, &roms).unwrap();
        let mut paths: Vec<String> = known.iter().map(|k| format!("{}/{}", k.folder, k.rel_path())).collect();
        paths.sort();
        assert_eq!(paths, ["snes/Aftermarket/Bar (USA).sfc", "snes/Aftermarket/Foo (USA).sfc", "snes/Foo (USA).sfc"]);
        let index = by_file(&known);
        let top = index[&("snes".to_owned(), "Foo (USA).sfc".to_owned())];
        let below = index[&("snes".to_owned(), "Aftermarket/Foo (USA).sfc".to_owned())];
        assert_ne!(top, below, "the twins became one game");
        assert_eq!(top, moose_rack::gameid::game_id("snes", "", "Foo (USA).sfc"));
        assert_eq!(below, moose_rack::gameid::game_id("snes", "Aftermarket", "Foo (USA).sfc"));
        let k = known.iter().find(|k| k.rom_id == below).unwrap();
        assert_eq!(k.full_path(&roms), roms.join("snes/Aftermarket/Foo (USA).sfc"));
        // And a collection file's absolute path finds its way back.
        let es = EsPaths::under(&dir);
        assert_eq!(
            es.locate(&es.roms.join("snes/Aftermarket/Foo (USA).sfc")),
            Some(("snes".to_owned(), "Aftermarket/Foo (USA).sfc".to_owned()))
        );
        assert_eq!(es.locate(Path::new("/elsewhere/snes/Foo.sfc")), None);
    }

    #[test]
    fn known_games_group_and_index_the_way_the_writers_need_them() {
        let known = vec![
            Known { rom_id: 1, folder: "snes".into(), rel_dir: String::new(), file: "A.sfc".into() },
            Known { rom_id: 2, folder: "snes".into(), rel_dir: String::new(), file: "B.sfc".into() },
            Known { rom_id: 3, folder: "gb".into(), rel_dir: String::new(), file: "C.gb".into() },
        ];
        assert_eq!(ids(&known), [1, 2, 3].into());
        let grouped = by_folder(&known);
        assert_eq!(grouped["snes"].len(), 2);
        assert_eq!(grouped["gb"].len(), 1);
        assert_eq!(by_file(&known)[&("gb".to_owned(), "C.gb".to_owned())], 3);
    }

    #[test]
    fn the_es_layout_is_where_it_actually_is_on_a_knulli_card() {
        let p = EsPaths::knulli();
        assert_eq!(p.gamelist("snes"), Path::new("/userdata/roms/snes/gamelist.xml"));
        assert_eq!(
            p.collection("Arcade Fighting"),
            Path::new("/userdata/system/configs/emulationstation/collections/custom-Arcade Fighting.cfg")
        );
        assert_eq!(
            p.settings,
            Path::new("/userdata/system/configs/emulationstation/es_settings.cfg")
        );
    }

    #[test]
    fn a_rom_path_is_absolute_because_that_is_what_a_collection_file_holds() {
        // ES-DE writes %ROMPATH%; Batocera's ES writes the whole path, and the
        // files already on this card are absolute.
        let k = Known { rom_id: 1, folder: "fbneo".into(), rel_dir: String::new(), file: "64street.zip".into() };
        assert_eq!(
            k.full_path(Path::new("/userdata/roms")),
            Path::new("/userdata/roms/fbneo/64street.zip")
        );
    }
}

//! Collections, as text files.
//!
//! One file per list, one game per line, `#` for comments. That shape was
//! chosen in `docs/library-service.md` for a specific reason: the 2,662
//! memberships here are the only thing in this library nobody can rebuild, and a
//! list you can read, diff, edit and restore without a server running is a list
//! that survives the server.
//!
//!     # ★ Best of nes
//!     # 86 games, exported 2026-09-03 from RomM
//!     Castlevania (USA)
//!     Contra (USA)
//!
//! Membership is by name, which is the trade this shape makes. A rename breaks a
//! line, and the alternative — an id — breaks on every rebuild instead. A broken
//! line is visible in a text file and fixable with an editor; a dangling id is
//! neither. Unmatched names are reported rather than dropped.

use serde::Serialize;

#[derive(Debug, Serialize, PartialEq)]
pub struct Collection {
    /// A string because RomM's virtual collections used base64 ids and the
    /// client's field is typed for it. Ours is the file's stem.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub rom_ids: Vec<i64>,
    pub rom_count: i64,
    pub is_favorite: bool,
    pub is_virtual: bool,
    pub is_smart: bool,
}

/// What a file asked for that the library does not have.
#[derive(Debug, PartialEq)]
pub struct Unmatched {
    pub collection: String,
    pub name: String,
}

/// Read every `.txt` in `dir` into a collection, resolving names against
/// `by_name`.
///
/// Matching is case-insensitive and ignores surrounding whitespace, because
/// these files are meant to be edited by hand and a trailing space should not
/// silently drop a game.
pub fn load(
    dir: &std::path::Path,
    by_name: &std::collections::HashMap<(String, String), i64>,
) -> (Vec<Collection>, Vec<Unmatched>) {
    let mut out = Vec::new();
    let mut missing = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (out, missing);
    };
    let mut paths: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    paths.sort();

    for p in paths {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let (mut ids, mut name) = (Vec::new(), stem.clone());
        // The first comment is the collection's own name, which can differ from
        // the file stem once a character had to be replaced to make a path.
        if let Some(first) = text.lines().next().and_then(|l| l.strip_prefix('#')) {
            let t = first.trim();
            if !t.is_empty() {
                name = t.to_owned();
            }
        }
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // A trailing `    # title` is the human-readable name, kept beside
            // the matchable one. Split on the run of spaces before the hash, not
            // on a bare `#`, because a game may legitimately contain one --
            // `Vs. Super Mario Bros.` does not, but `#1 Club` would.
            let (key, alt) = match line.find("    #") {
                Some(i) => (line[..i].trim_end(), Some(line[i + 5..].trim())),
                None => (line, None),
            };
            // `platform/name`. The platform is not decoration: without it
            // `Arcade Classics` resolved "Contra" to the *Famicom* Contra,
            // because a name alone is not unique across a library that holds
            // both the arcade original and its home port.
            let (plat, key) = match key.split_once('/') {
                Some((p, n)) => (p.trim().to_lowercase(), n.trim()),
                None => (String::new(), key),
            };
            // Both halves are tried, because neither side names games one way.
            // ES-DE takes `<name>` from the gamelist when a scraper filled it in
            // and falls back to the file stem when it did not, so a list keyed
            // on either alone matches part of the library and misses the rest --
            // measured at 466 of 2,662 one way and 103 the other.
            let hit = by_name
                .get(&(plat.clone(), key.to_lowercase()))
                .or_else(|| alt.and_then(|a| by_name.get(&(plat.clone(), a.to_lowercase()))));
            match hit {
                Some(id) => ids.push(*id),
                None => missing.push(Unmatched {
                    collection: name.clone(),
                    name: key.to_owned(),
                }),
            }
        }
        ids.sort_unstable();
        ids.dedup();
        out.push(Collection {
            rom_count: ids.len() as i64,
            rom_ids: ids,
            // The star is what sorts these to the top of a listing, and it is
            // also what marks them as the curated ones.
            is_favorite: name.starts_with('★'),
            is_virtual: false,
            is_smart: false,
            description: None,
            id: stem,
            name,
        });
    }
    (out, missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lib() -> HashMap<(String, String), i64> {
        [("nes", "castlevania (usa)", 1i64), ("nes", "contra (usa)", 2), ("nes", "metroid (usa)", 3)]
            .into_iter()
            .map(|(p, k, v)| ((p.to_owned(), k.to_owned()), v))
            .collect()
    }

    fn write(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn names_resolve_to_ids_and_comments_are_skipped() {
        let d = tempdir::TempDir::new("c").unwrap();
        write(d.path(), "best.txt", "# ★ Best of nes\n# 2 games\nnes/Castlevania (USA)\nnes/Contra (USA)\n");
        let (cols, missing) = load(d.path(), &lib());
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "★ Best of nes");
        assert_eq!(cols[0].rom_ids, vec![1, 2]);
        assert_eq!(cols[0].rom_count, 2);
        assert!(missing.is_empty());
    }

    /// The star marks the curated ones, and it is what the client shows as a
    /// favourite.
    #[test]
    fn a_starred_list_is_a_favourite() {
        let d = tempdir::TempDir::new("c").unwrap();
        write(d.path(), "a.txt", "# ★ Best of nes\nnes/Contra (USA)\n");
        write(d.path(), "b.txt", "# Arcade Puzzle\nnes/Contra (USA)\n");
        let (cols, _) = load(d.path(), &lib());
        assert!(cols.iter().find(|c| c.name.contains("Best")).unwrap().is_favorite);
        assert!(!cols.iter().find(|c| c.name.contains("Arcade")).unwrap().is_favorite);
    }

    /// These are edited by hand. A trailing space or different case must not
    /// silently drop a game.
    #[test]
    fn matching_forgives_case_and_whitespace() {
        let d = tempdir::TempDir::new("c").unwrap();
        write(d.path(), "a.txt", "# x\n  nes/CASTLEVANIA (usa)  \n\tnes/Contra (USA)\n\n");
        let (cols, missing) = load(d.path(), &lib());
        assert_eq!(cols[0].rom_ids, vec![1, 2]);
        assert!(missing.is_empty());
    }

    /// A name the library does not have is reported, never dropped: that is the
    /// rot worth seeing, and dropping it makes a shrinking list look healthy.
    #[test]
    fn an_unmatched_name_is_reported_not_swallowed() {
        let d = tempdir::TempDir::new("c").unwrap();
        write(d.path(), "a.txt", "# ★ Best of nes\nnes/Contra (USA)\nnes/Game That Left (USA)\n");
        let (cols, missing) = load(d.path(), &lib());
        assert_eq!(cols[0].rom_ids, vec![2], "only the one that resolved");
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "Game That Left (USA)");
        assert_eq!(missing[0].collection, "★ Best of nes");
    }

    #[test]
    fn the_same_game_twice_is_counted_once() {
        let d = tempdir::TempDir::new("c").unwrap();
        write(d.path(), "a.txt", "# x\nnes/Contra (USA)\nnes/Contra (USA)\n");
        let (cols, _) = load(d.path(), &lib());
        assert_eq!(cols[0].rom_ids, vec![2]);
        assert_eq!(cols[0].rom_count, 1);
    }

    #[test]
    fn a_missing_or_empty_directory_is_not_an_error() {
        let d = tempdir::TempDir::new("c").unwrap();
        assert_eq!(load(d.path(), &lib()).0.len(), 0);
        assert_eq!(load(&d.path().join("nope"), &lib()).0.len(), 0);
    }

    /// Only `.txt`, so a stray README or a `.bak` left by an editor does not
    /// become a collection.
    #[test]
    fn only_txt_files_count() {
        let d = tempdir::TempDir::new("c").unwrap();
        write(d.path(), "a.txt", "# real\nnes/Contra (USA)\n");
        write(d.path(), "README.md", "# not a collection\nnes/Contra (USA)\n");
        write(d.path(), "a.txt.bak", "# nor this\nnes/Contra (USA)\n");
        let (cols, _) = load(d.path(), &lib());
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "real");
    }

    /// The export writes `filename    # Display Title`, so the title stays
    /// readable without being what is matched on.
    #[test]
    fn a_trailing_title_comment_is_not_part_of_the_name() {
        let d = tempdir::TempDir::new("c").unwrap();
        write(d.path(), "a.txt", "# x\nnes/Contra (USA)    # Contra\nnes/Castlevania (USA)\n");
        let (cols, missing) = load(d.path(), &lib());
        assert_eq!(cols[0].rom_ids, vec![1, 2]);
        assert!(missing.is_empty(), "the comment must not break the match");
    }

    /// A game whose name starts with `#` used to be swallowed as a comment.
    /// The `platform/` prefix fixed that for free: the line no longer begins
    /// with a hash, so it is read as an entry.
    #[test]
    fn a_hash_in_the_name_itself_survives() {
        let d = tempdir::TempDir::new("c").unwrap();
        let mut l = lib();
        l.insert(("nes".into(), "#1 club (usa)".into()), 9);
        write(d.path(), "a.txt", "# x\nnes/#1 Club (USA)\n");
        let (cols, missing) = load(d.path(), &l);
        assert_eq!(cols[0].rom_ids, vec![9], "the platform prefix rescues it");
        assert!(missing.is_empty());
    }

    /// Neither side names games one way, so a line carries both and either may
    /// be the one the library knows.
    #[test]
    fn the_title_comment_is_a_fallback_when_the_filename_does_not_match() {
        let d = tempdir::TempDir::new("c").unwrap();
        let mut l: HashMap<(String,String), i64> = HashMap::new();
        // The library knows this game only by its display title.
        l.insert(("nes".to_owned(), "contra".to_owned()), 7i64);
        write(d.path(), "a.txt", "# x\nnes/Contra (USA)    # Contra\n");
        let (cols, missing) = load(d.path(), &l);
        assert_eq!(cols[0].rom_ids, vec![7], "fell back to the title");
        assert!(missing.is_empty());
    }

    /// The filename is tried first, so a library that knows both does not
    /// depend on which one the scraper happened to write.
    #[test]
    fn the_filename_wins_when_both_are_known() {
        let d = tempdir::TempDir::new("c").unwrap();
        let mut l: HashMap<(String,String), i64> = HashMap::new();
        l.insert(("nes".to_owned(), "contra (usa)".to_owned()), 1i64);
        l.insert(("nes".to_owned(), "contra".to_owned()), 2i64);
        write(d.path(), "a.txt", "# x\nnes/Contra (USA)    # Contra\n");
        let (cols, _) = load(d.path(), &l);
        assert_eq!(cols[0].rom_ids, vec![1]);
    }

    /// The bug this format had: a name alone is not unique across a library
    /// holding both the arcade original and its home port, so `Arcade Classics`
    /// resolved "Contra" to the Famicom one.
    #[test]
    fn the_same_name_on_two_platforms_is_two_games() {
        let d = tempdir::TempDir::new("c").unwrap();
        let mut l: HashMap<(String, String), i64> = HashMap::new();
        l.insert(("arcade".into(), "contra".into()), 100);
        l.insert(("famicom".into(), "contra".into()), 200);
        write(d.path(), "a.txt", "# Arcade Classics\narcade/Contra\n");
        let (cols, missing) = load(d.path(), &l);
        assert_eq!(cols[0].rom_ids, vec![100], "must not take the famicom one");
        assert!(missing.is_empty());
        // And a platform the library does not have resolves to nothing rather
        // than to whatever shares the name.
        write(d.path(), "b.txt", "# Best of megadrive\nmegadrive/Contra\n");
        let (cols, missing) = load(d.path(), &l);
        let md = cols.iter().find(|c| c.name.contains("megadrive")).unwrap();
        assert!(md.rom_ids.is_empty(), "no platform match, no membership");
        assert_eq!(missing.len(), 1);
    }

    /// The real files, parsed. Not a fixture: these 27 lists are the only thing
    /// in this library nobody can rebuild, and a format change that silently
    /// stops reading them is the failure worth catching here.
    #[test]
    fn the_committed_collections_all_parse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().join("data/collections");
        if !dir.is_dir() {
            return; // not a checkout with the data
        }
        // Every line resolves against a library that knows every name in them,
        // so anything unmatched is a parse failure rather than a missing game.
        let mut lib: HashMap<(String, String), i64> = HashMap::new();
        let mut n = 0i64;
        for f in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = f.path();
            if p.extension().is_none_or(|e| e != "txt") { continue }
            for line in std::fs::read_to_string(&p).unwrap().lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') { continue }
                let key = line.split("    #").next().unwrap().trim();
                let Some((plat, name)) = key.split_once('/') else {
                    panic!("{}: line has no platform prefix: {line}", p.display());
                };
                n += 1;
                lib.insert((plat.to_lowercase(), name.to_lowercase()), n);
            }
        }
        let (cols, missing) = load(&dir, &lib);
        assert_eq!(cols.len(), 27, "27 curated lists");
        assert!(missing.is_empty(), "unparsed lines: {:?}", &missing[..missing.len().min(5)]);
        let total: i64 = cols.iter().map(|c| c.rom_count).sum();
        assert_eq!(total, n, "every line became a membership");
        assert_eq!(cols.iter().filter(|c| c.is_favorite).count(), 9, "nine starred lists");
    }
}

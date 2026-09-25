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
            let Some(entry) = Entry::parse(line) else { continue };
            match entry.resolve(by_name) {
                Some(id) => ids.push(id),
                None => missing.push(Unmatched {
                    collection: name.clone(),
                    name: entry.key.to_owned(),
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

/// One game line of a list file, split into what is matched on.
struct Entry<'a> {
    platform: String,
    key: &'a str,
    title: Option<&'a str>,
}

impl<'a> Entry<'a> {
    /// `None` for a blank line or a comment.
    fn parse(line: &'a str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        // A trailing `    # title` is the human-readable name, kept beside
        // the matchable one. Split on the run of spaces before the hash, not
        // on a bare `#`, because a game may legitimately contain one --
        // `Vs. Super Mario Bros.` does not, but `#1 Club` would.
        let (key, title) = match line.find("    #") {
            Some(i) => (line[..i].trim_end(), Some(line[i + 5..].trim())),
            None => (line, None),
        };
        // `platform/name`. The platform is not decoration: without it
        // `Arcade Classics` resolved "Contra" to the *Famicom* Contra,
        // because a name alone is not unique across a library that holds
        // both the arcade original and its home port.
        let (platform, key) = match key.split_once('/') {
            Some((p, n)) => (p.trim().to_lowercase(), n.trim()),
            None => (String::new(), key),
        };
        Some(Self { platform, key, title })
    }

    /// Both halves are tried, because neither side names games one way.
    /// ES-DE takes `<name>` from the gamelist when a scraper filled it in and
    /// falls back to the file stem when it did not, so a list keyed on either
    /// alone matches part of the library and misses the rest -- measured at
    /// 466 of 2,662 one way and 103 the other.
    fn resolve(&self, by_name: &std::collections::HashMap<(String, String), i64>) -> Option<i64> {
        let p = &self.platform;
        by_name
            .get(&(p.clone(), self.key.to_lowercase()))
            .or_else(|| self.title.and_then(|t| by_name.get(&(p.clone(), t.to_lowercase()))))
            .copied()
    }
}

/// A game, as the name table and a list line see it.
pub struct Named<'a> {
    pub platform: &'a str,
    /// The folder inside the system directory, `""` at the top.
    pub rel_dir: &'a str,
    pub name: &'a str,
    pub fs_name: &'a str,
    pub id: i64,
}

fn stem(fs_name: &str) -> &str {
    fs_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(fs_name)
}

fn under(rel_dir: &str, name: &str) -> String {
    let rel_dir = rel_dir.trim_matches('/');
    if rel_dir.is_empty() { name.to_owned() } else { format!("{rel_dir}/{name}") }
}

/// Name -> id, from whatever rows are given.
///
/// Keyed on both the display name and the file stem, because ES-DE uses the
/// gamelist `<name>` where a scraper filled one in and the stem where it did
/// not. On one key alone this resolved 466 of 2,662 memberships; on the other,
/// 103. The platform is part of the key because a name is not unique across a
/// library holding arcade *Contra* and Famicom *Contra* -- un-prefixed,
/// `Arcade Classics` resolved to the Famicom one and looked entirely plausible.
///
/// Then, in a second pass so they never take a key the first pass gave out,
/// the forms a written line falls back to when the stem is ambiguous: the
/// folder and stem (`Aftermarket/Foo`), the file name with its extension, and
/// both. `snes/` and `snes/Aftermarket/` can hold a file of the same name, and
/// a list written from the Flip has to be able to say which one it meant.
pub fn name_table<'a>(
    rows: impl IntoIterator<Item = Named<'a>>,
) -> std::collections::HashMap<(String, String), i64> {
    let rows: Vec<Named<'a>> = rows.into_iter().collect();
    let mut out = std::collections::HashMap::new();
    for g in &rows {
        let p = g.platform.to_lowercase();
        out.entry((p.clone(), g.name.to_lowercase())).or_insert(g.id);
        out.entry((p, stem(g.fs_name).to_lowercase())).or_insert(g.id);
    }
    for g in &rows {
        let p = g.platform.to_lowercase();
        for key in [under(g.rel_dir, stem(g.fs_name)), g.fs_name.to_owned(), under(g.rel_dir, g.fs_name)] {
            out.entry((p.clone(), key.to_lowercase())).or_insert(g.id);
        }
    }
    out
}

/// The line that puts `g` in a list: `platform/key    # Title`.
///
/// The key is the plainest one that reads back as this game and no other --
/// the stem, as the export wrote them, unless a game of the same name elsewhere
/// in the system already answers to it. `None` when nothing names it uniquely,
/// which takes two systems sharing a platform and holding the same file.
pub fn line_for(g: &Named, by_name: &std::collections::HashMap<(String, String), i64>) -> Option<String> {
    let p = g.platform.to_lowercase();
    let candidates = [
        stem(g.fs_name).to_owned(),
        under(g.rel_dir, stem(g.fs_name)),
        g.fs_name.to_owned(),
        under(g.rel_dir, g.fs_name),
    ];
    let key = candidates
        .into_iter()
        .find(|k| by_name.get(&(p.clone(), k.to_lowercase())) == Some(&g.id))?;
    Some(format!("{}/{key}    # {}", g.platform, g.name))
}

/// What to do to a list's membership.
pub enum Change {
    /// Lines to append for games not already in it, keyed by game id.
    Add(Vec<(i64, String)>),
    /// Every line naming one of these games goes.
    Remove(std::collections::BTreeSet<i64>),
}

/// Rewrite one list file so its next load has the membership asked for.
///
/// Comments, blank lines, lines that no longer resolve and the order of what
/// stays are all kept as they were; new games go at the end. A list is
/// something a person edits by hand, and a write from a handheld should read in
/// a diff as the one or two lines it changed. Written through a temporary file
/// beside it, named so `load` never takes a half-written one for a list.
///
/// Returns whether the file changed.
pub fn edit(
    path: &std::path::Path,
    change: &Change,
    by_name: &std::collections::HashMap<(String, String), i64>,
) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let crlf = text.contains("\r\n");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let present: std::collections::BTreeSet<i64> =
        lines.iter().filter_map(|l| Entry::parse(l)?.resolve(by_name)).collect();
    let before = lines.len();
    match change {
        Change::Add(new) => {
            let mut added = std::collections::BTreeSet::new();
            for (id, line) in new {
                if !present.contains(id) && added.insert(*id) {
                    lines.push(line.clone());
                }
            }
            if lines.len() == before {
                return Ok(false);
            }
        }
        Change::Remove(gone) => {
            lines.retain(|l| {
                Entry::parse(l).and_then(|e| e.resolve(by_name)).is_none_or(|id| !gone.contains(&id))
            });
            if lines.len() == before {
                return Ok(false);
            }
        }
    }
    let nl = if crlf { "\r\n" } else { "\n" };
    let mut body = lines.join(nl);
    body.push_str(nl);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // `.name.txt.tmp`: not a `.txt`, so a crash between the two steps leaves
    // nothing `load` would read as a second copy of the list.
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(true)
}

/// Put the curated lists into the metadata cache the web UI reads.
///
/// `/api/collections` already serves these, but that answers the *client*,
/// which keys its own cache by the ids it syncs. The UI running in this process
/// reads the cache directly and knew nothing about them: a server holding all 27
/// lists showed an empty Collections tab.
///
/// Resolved a second time rather than reusing the ids from the API scan. Those
/// number the scan; the cache numbers its own rows, and they are not the same
/// numbering. Same parser, different name table -- which is the whole reason
/// membership is by name and not by id.
pub fn into_cache(
    store: &mut moose_rack::cache::Cache,
    dir: &std::path::Path,
) -> anyhow::Result<(usize, usize)> {
    let rows = store.all_roms()?;
    let table = name_table(rows.iter().map(|r| Named {
        platform: &r.platform_slug,
        rel_dir: &r.rel_dir,
        name: &r.name,
        fs_name: &r.fs_name,
        id: r.id,
    }));
    let (cols, unmatched) = load(dir, &table);
    let items: Vec<moose_rack::api::Collection> = cols
        .into_iter()
        .map(|c| moose_rack::api::Collection {
            id: c.id,
            name: c.name,
            description: c.description,
            rom_count: c.rom_ids.len() as i64,
            rom_ids: c.rom_ids,
            is_favorite: c.is_favorite,
            is_virtual: false,
            is_smart: false,
            kind: None,
            path_covers_small: Vec::new(),
        })
        .collect();
    let n = store.replace_collections(&items)?;
    Ok((n, unmatched.len()))
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

#[cfg(test)]
mod cache_tests {
    use super::*;

    /// The lists reach the UI's own store, keyed to the ids that store uses.
    ///
    /// The tab was empty on a server holding all 27 lists: `/api/collections`
    /// answers the client and the UI here does not go through the client. Two
    /// stores, and only one of them was being filled.
    #[test]
    fn the_lists_land_in_the_cache_the_ui_reads() {
        let dir = std::env::temp_dir().join("moose-col-cache-test");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("lists")).unwrap();
        std::fs::write(
            dir.join("lists/best.txt"),
            "# Best of NES\n# 2 games\nnes/Castlevania (USA)\nnes/Nothing Here\n",
        )
        .unwrap();

        let mut store = moose_rack::cache::Cache::open(&dir.join("cache.sqlite3")).unwrap();
        let games = vec![
            moose_rack::esde::Game {
                platform_slug: "nes".into(),
                system: "nes".into(),
                name: "Castlevania (USA)".into(),
                fs_name: "Castlevania (USA).zip".into(),
                ..Default::default()
            },
        ];
        store.replace_from_esde(&games).unwrap();

        let (n, missing) = into_cache(&mut store, &dir.join("lists")).unwrap();
        assert_eq!(n, 1, "one list");
        assert_eq!(missing, 1, "the line naming a game that is not here is reported, not dropped");

        // The membership must point at the cache's own id, which is not the
        // scan's numbering -- resolving once and reusing those ids is the bug
        // this exists to prevent.
        let id = store.all_roms().unwrap()[0].id;
        let members = store.roms_in_collection("best").unwrap();
        assert_eq!(members.len(), 1, "the list opens with its game in it");
        assert_eq!(members[0].id, id);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Both resolvers key names the same way, because they resolve the same
    /// files against different ids.
    #[test]
    fn the_name_table_keys_on_platform_name_and_stem() {
        let t = name_table([Named {
            platform: "NES",
            rel_dir: "",
            name: "Contra",
            fs_name: "Contra (USA).zip",
            id: 7,
        }]);
        assert_eq!(t.get(&("nes".into(), "contra".into())), Some(&7));
        assert_eq!(t.get(&("nes".into(), "contra (usa)".into())), Some(&7));
        // The platform is part of the key: arcade Contra is not this one.
        assert_eq!(t.get(&("arcade".into(), "contra".into())), None);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    fn g<'a>(rel_dir: &'a str, fs_name: &'a str, name: &'a str, id: i64) -> Named<'a> {
        Named { platform: "snes", rel_dir, name, fs_name, id }
    }

    /// Two games with one file name, one at the top and one in a subfolder,
    /// the way `snes/Aftermarket/` sits beside `snes/` on the SSD.
    fn twins() -> Vec<Named<'static>> {
        vec![
            g("", "Foo (USA).sfc", "Foo", 1),
            g("Aftermarket", "Foo (USA).sfc", "Foo", 2),
            g("", "Bar (USA).sfc", "Bar", 3),
        ]
    }

    /// A line written for a game has to read back as that game. The stem is
    /// what the export used; a twin in a subfolder needs its folder in front,
    /// or the list names the other one.
    #[test]
    fn a_written_line_reads_back_as_the_game_it_was_written_for() {
        let t = name_table(twins());
        for game in twins() {
            let line = line_for(&game, &t).unwrap();
            let back = Entry::parse(&line).unwrap().resolve(&t);
            assert_eq!(back, Some(game.id), "{line} reads as another game");
        }
        assert_eq!(line_for(&twins()[2], &t).unwrap(), "snes/Bar (USA)    # Bar");
        assert_eq!(line_for(&twins()[1], &t).unwrap(), "snes/Aftermarket/Foo (USA)    # Foo");
    }

    /// The second pass must not take a key the first pass would have given
    /// out, or a list that resolved before this change resolves differently.
    #[test]
    fn the_extra_keys_never_change_what_an_existing_line_means() {
        // Game 1's file name, with its extension, is game 2's display name.
        let t = name_table([
            g("", "Odd.Name.sfc", "Odd", 1),
            g("", "Other (USA).sfc", "Odd.Name.sfc", 2),
        ]);
        assert_eq!(t.get(&("snes".into(), "odd.name.sfc".into())), Some(&2));
    }

    #[test]
    fn adding_appends_and_keeps_every_comment_and_the_order() {
        let d = tempdir::TempDir::new("e").unwrap();
        let p = d.path().join("best.txt");
        let t = name_table(twins());
        std::fs::write(&p, "# ★ Best of snes\n# 1 game\nsnes/Bar (USA)    # Bar\n\nsnes/Gone (USA)\n").unwrap();
        let add = Change::Add(vec![
            (3, line_for(&twins()[2], &t).unwrap()),
            (2, line_for(&twins()[1], &t).unwrap()),
            (2, line_for(&twins()[1], &t).unwrap()),
        ]);
        assert!(edit(&p, &add, &t).unwrap());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "# ★ Best of snes\n# 1 game\nsnes/Bar (USA)    # Bar\n\nsnes/Gone (USA)\n\
             snes/Aftermarket/Foo (USA)    # Foo\n",
            "already there once, asked for twice: one new line, at the end"
        );
        assert!(!edit(&p, &add, &t).unwrap(), "rewrote a list that already said this");
        assert!(!d.path().join(".best.txt.tmp").exists(), "left its temporary behind");
    }

    #[test]
    fn removing_takes_only_the_lines_for_those_games() {
        let d = tempdir::TempDir::new("e").unwrap();
        let p = d.path().join("best.txt");
        let t = name_table(twins());
        std::fs::write(&p, "# x\nsnes/Foo (USA)\nsnes/Aftermarket/Foo (USA)\r\nsnes/Bar (USA)    # Bar\n").unwrap();
        assert!(edit(&p, &Change::Remove([2].into()), &t).unwrap());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "# x\r\nsnes/Foo (USA)\r\nsnes/Bar (USA)    # Bar\r\n",
            "the twin at the top stays; a file that used CRLF keeps it"
        );
        assert!(!edit(&p, &Change::Remove([2].into()), &t).unwrap());
    }

    /// A list that cannot be read is an error, never an empty list to write
    /// over.
    #[test]
    fn a_list_that_cannot_be_read_is_not_rewritten() {
        let d = tempdir::TempDir::new("e").unwrap();
        let p = d.path().join("bad.txt");
        std::fs::write(&p, b"# x\nsnes/Bar (USA)\n\xff\xfe\n").unwrap();
        let t = name_table(twins());
        assert!(edit(&p, &Change::Add(vec![(1, "snes/Foo (USA)".into())]), &t).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"# x\nsnes/Bar (USA)\n\xff\xfe\n");
    }
}

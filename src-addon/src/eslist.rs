//! What EmulationStation thinks is a favourite, and what it thinks a
//! collection is.
//!
//! ES keeps the two ideas in two different places, neither of which is the
//! server:
//!
//! * `/userdata/roms/<system>/gamelist.xml` — a `<favorite>true</favorite>`
//!   inside the `<game>` block, beside the scraped description and artwork.
//! * `/userdata/system/configs/emulationstation/collections/custom-<name>.cfg`
//!   — one absolute ROM path per line.
//!
//! **The gamelist is edited as text, not as XML.** It holds everything the
//! scraper found — descriptions, ratings, release dates, RetroAchievements
//! hashes — and a parse-and-rewrite would quietly drop any tag this program
//! has never heard of. Reading 633 games and writing back 633 games is how you
//! lose a library's worth of scraping to a tag you forgot. So the only bytes
//! that move are the ones inside the `<favorite>` element.
//!
//! Every file here is copied aside before it is rewritten -- see [`back_up`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// How many copies of each file [`back_up`] keeps. The same number the save
/// backups keep.
pub const KEEP: usize = 10;

/// Copy `file` aside before it is overwritten, and drop the oldest copies.
///
/// `<backups>/<folder>/<mtime millis>-<name>`, where `folder` is the directory
/// the file sits in: `snes/…-gamelist.xml`, `collections/…-custom-X.cfg`.
/// Plain files, so putting one back is a `cp` over ssh. Keyed on the file's
/// own mtime, so backing up bytes that have not changed since the last copy
/// writes nothing. A file that does not exist yet has nothing to keep.
///
/// An error here stops the write it was protecting.
pub fn back_up(backups: &Path, file: &Path) -> Result<()> {
    let meta = match std::fs::metadata(file) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = file
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "top".into());
    let dir = backups.join(folder);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis());
    let dest = dir.join(format!("{stamp}-{name}"));
    if !dest.exists() {
        std::fs::copy(file, &dest)
            .with_context(|| format!("backing up {} to {}", file.display(), dest.display()))?;
    }
    // Only this file's copies: one folder holds every collection's.
    let mut kept: Vec<(u128, PathBuf)> = std::fs::read_dir(&dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let (stamp, rest) = n.split_once('-')?;
            if rest != name {
                return None;
            }
            Some((stamp.parse::<u128>().ok()?, e.path()))
        })
        .collect();
    kept.sort_by(|a, b| b.cmp(a));
    for (_, old) in kept.into_iter().skip(KEEP) {
        std::fs::remove_file(&old).ok();
    }
    Ok(())
}

/// Write `body` to `path` through a temporary beside it, after backing up
/// what is there.
fn replace(path: &Path, body: &str, backups: &Path) -> Result<()> {
    back_up(backups, path)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".moose");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// One system's `gamelist.xml`, held as the text it is.
pub struct Gamelist {
    path: PathBuf,
    text: String,
    dirty: bool,
}

/// Where a `<game>` block sits, and what it says.
struct Entry {
    /// The ROM's path inside the system folder, with ES's leading `./` taken
    /// off: `Tetris.gb`, or `Aftermarket/Foo.sfc` for one in a subfolder.
    file: String,
    /// Byte range of the whole `<game>…</game>` block.
    block: (usize, usize),
    /// Byte range of the `<favorite>…</favorite>` element, when there is one.
    favorite: Option<(usize, usize)>,
    is_favorite: bool,
}

impl Gamelist {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Ok(Self { path: path.to_path_buf(), text, dirty: false })
    }

    /// An empty list, for a system ES has never scraped.
    pub fn empty(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            text: "<?xml version=\"1.0\"?>\n<gameList>\n</gameList>\n".into(),
            dirty: true,
        }
    }

    /// The list, or `None` when the system has none yet.
    ///
    /// Only "not found" is `None`. Anything else -- a byte that is not UTF-8,
    /// an I/O error -- is an error: read as an empty list it would be a list
    /// with no stars, and every star on the server would look unstarred here.
    pub fn load_if_present(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(Self { path: path.to_path_buf(), text, dirty: false })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn load_or_empty(path: &Path) -> Result<Self> {
        Ok(Self::load_if_present(path)?.unwrap_or_else(|| Self::empty(path)))
    }

    /// Where the next `<game>` block opens, at or after `from`.
    ///
    /// With or without attributes: ES writes `<game id="1234" source="ScreenScraper">`
    /// for a game it scraped itself, and looking for the bare `<game>` alone
    /// read every one of those as absent -- starred on the card and never seen.
    /// `<gameList>` is not one.
    fn next_game(&self, mut from: usize) -> Option<usize> {
        while let Some(i) = self.text[from..].find("<game") {
            let start = from + i;
            let after = self.text[start + "<game".len()..].chars().next();
            if after.is_some_and(|c| c == '>' || c.is_whitespace()) {
                return Some(start);
            }
            from = start + "<game".len();
        }
        None
    }

    /// Every `<game>` block, in the order they appear.
    fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(start) = self.next_game(from) {
            let Some(end) = self.text[start..].find("</game>").map(|i| i + start + "</game>".len())
            else {
                break;
            };
            let block = &self.text[start..end];
            if let Some(file) = tag_value(block, "path") {
                let favorite = tag_span(block, "favorite")
                    .map(|(a, b)| (a + start, b + start));
                out.push(Entry {
                    file: file.trim_start_matches("./").to_owned(),
                    block: (start, end),
                    favorite,
                    is_favorite: tag_value(block, "favorite")
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true")),
                });
            }
            from = end;
        }
        out
    }

    /// The ROMs ES has starred, as paths inside the system folder.
    pub fn favorites(&self) -> BTreeSet<String> {
        self.entries()
            .into_iter()
            .filter(|e| e.is_favorite)
            .map(|e| e.file)
            .collect()
    }

    /// Every ROM file name this list mentions.
    pub fn known(&self) -> BTreeSet<String> {
        self.entries().into_iter().map(|e| e.file).collect()
    }

    /// Star a game, or unstar it. Says whether anything moved.
    ///
    /// A game ES has never scraped has no block to edit, so a minimal one is
    /// added; ES fills in the rest the next time it scrapes. Unstarring takes
    /// the element out rather than writing `false`, which is how ES itself
    /// leaves an unstarred game.
    pub fn set_favorite(&mut self, file: &str, starred: bool) -> bool {
        let Some(entry) = self.entries().into_iter().find(|e| e.file == file) else {
            return if starred { self.append_game(file) } else { false };
        };
        if entry.is_favorite == starred {
            return false;
        }
        match (entry.favorite, starred) {
            // Present and wrong: rewrite just the element.
            (Some((a, b)), _) => {
                let replacement =
                    if starred { "<favorite>true</favorite>" } else { "" };
                self.text.replace_range(a..b, replacement);
                if !starred {
                    self.tidy_blank_line(a);
                }
            }
            // Absent and wanted: put it in front of the closing tag, indented
            // the way the tags above it are.
            (None, true) => {
                // The start of the line `</game>` sits on, not the tag: the
                // closing tag has its own indentation in front of it, and
                // inserting at the tag puts the new element after it.
                let close = entry.block.1 - "</game>".len();
                let line = self.text[..close].rfind('\n').map_or(close, |i| i + 1);
                let indent = block_indent(&self.text, entry.block);
                self.text
                    .insert_str(line, &format!("{indent}<favorite>true</favorite>\n"));
            }
            (None, false) => return false,
        }
        self.dirty = true;
        true
    }

    /// A `<game>` block for something ES has not scraped.
    fn append_game(&mut self, file: &str) -> bool {
        let Some(close) = self.text.rfind("</gameList>") else {
            return false;
        };
        let base = file.rsplit_once('/').map_or(file, |(_, name)| name);
        let name = base.rsplit_once('.').map_or(base, |(stem, _)| stem);
        let block = format!(
            "\t<game>\n\t\t<path>./{}</path>\n\t\t<name>{}</name>\n\t\t<favorite>true</favorite>\n\t</game>\n",
            escape(file),
            escape(name),
        );
        self.text.insert_str(close, &block);
        self.dirty = true;
        true
    }

    /// After lifting an element out, the line it was on is left as whitespace.
    fn tidy_blank_line(&mut self, at: usize) {
        let start = self.text[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[at..].find('\n').map_or(self.text.len(), |i| at + i + 1);
        if self.text[start..end].trim().is_empty() {
            self.text.replace_range(start..end, "");
        }
    }

    pub fn changed(&self) -> bool {
        self.dirty
    }

    /// Write it back, but only if something moved, after copying the old one
    /// into `backups`.
    ///
    /// Through a temporary file in the same directory: ES reads these on a
    /// timer, and a half-written gamelist is a system that opens empty.
    pub fn save(&self, backups: &Path) -> Result<bool> {
        if !self.dirty {
            return Ok(false);
        }
        replace(&self.path, &self.text, backups)?;
        Ok(true)
    }
}

/// The text inside the first `<tag>…</tag>` of a block.
fn tag_value(block: &str, tag: &str) -> Option<String> {
    let (a, b) = tag_span(block, tag)?;
    let open = format!("<{tag}>");
    Some(unescape(&block[a + open.len()..b - tag.len() - 3]))
}

/// Where the first `<tag>…</tag>` of a block starts and ends.
fn tag_span(block: &str, tag: &str) -> Option<(usize, usize)> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let a = block.find(&open)?;
    let b = block[a..].find(&close)? + a + close.len();
    Some((a, b))
}

/// The whitespace the tags inside a block are indented with.
fn block_indent(text: &str, block: (usize, usize)) -> String {
    let inner = &text[block.0..block.1];
    inner
        .find("<path>")
        .map(|at| {
            let line = inner[..at].rfind('\n').map_or(0, |i| i + 1);
            inner[line..at].to_owned()
        })
        .filter(|s| s.chars().all(char::is_whitespace) && !s.is_empty())
        .unwrap_or_else(|| "\t\t".into())
}

/// Text for inside an element.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Text for inside a `"`-quoted attribute. A bare `"` there ends the value
/// early, and ES then cannot parse the rest of `es_settings.cfg`.
fn escape_attr(s: &str) -> String {
    escape(s).replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// --- Custom collections -----------------------------------------------------

/// The name ES knows a collection by.
///
/// ES takes it from the file name, `custom-<name>.cfg`, and lists the ones to
/// show in `CollectionSystemsCustom` separated by commas, with no escaping for
/// either. So `/`, which would make the file a directory, and `,`, which would
/// split one name into two in the setting, become `-` in both places. The two
/// have to agree or the file is written and never shown.
pub fn es_name(collection: &str) -> String {
    collection.replace(['/', ','], "-")
}

/// One `custom-<name>.cfg`: absolute ROM paths, one per line.
pub struct CollectionFile {
    path: PathBuf,
    pub entries: BTreeSet<PathBuf>,
    /// The file as it was read, so comments, blank lines and the order of the
    /// entries survive a save.
    lines: Vec<String>,
}

impl CollectionFile {
    /// What ES calls the file for a collection of this name.
    pub fn file_name(collection: &str) -> String {
        format!("custom-{}.cfg", es_name(collection))
    }

    fn is_entry(line: &str) -> bool {
        let l = line.trim();
        !l.is_empty() && !l.starts_with('#')
    }

    /// The file, or an empty one when there is none yet. Any other failure to
    /// read it is an error: read as empty, it would be a list with no games,
    /// and every one of them would look taken out here.
    pub fn load(path: &Path) -> Result<Self> {
        let lines: Vec<String> = match std::fs::read_to_string(path) {
            Ok(text) => text.lines().map(str::to_owned).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let entries = lines
            .iter()
            .filter(|l| Self::is_entry(l))
            .map(|l| PathBuf::from(l.trim()))
            .collect();
        Ok(Self { path: path.to_path_buf(), entries, lines })
    }

    /// Write it back, only when it differs from what is there, after copying
    /// the old one into `backups`.
    ///
    /// Lines keep their places: an entry taken out loses its line, one put in
    /// goes at the end, and comments and blank lines stay where they were.
    pub fn save(&mut self, backups: &Path) -> Result<bool> {
        let mut out: Vec<String> = Vec::new();
        let mut written = BTreeSet::new();
        for line in &self.lines {
            if Self::is_entry(line) {
                let p = PathBuf::from(line.trim());
                if !self.entries.contains(&p) || !written.insert(p) {
                    continue;
                }
            }
            out.push(line.clone());
        }
        for p in &self.entries {
            if !written.contains(p) {
                out.push(p.to_string_lossy().into_owned());
            }
        }
        if out == self.lines {
            return Ok(false);
        }
        let mut body = out.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        replace(&self.path, &body, backups)?;
        self.lines = out;
        Ok(true)
    }
}

/// Collections ES will actually show.
///
/// A `custom-*.cfg` alone is invisible: its name has to be listed in
/// `CollectionSystemsCustom` in `es_settings.cfg` as well, which is the step
/// that gets forgotten and makes a correct file look like a broken one.
pub fn enabled_collections(settings: &str) -> Vec<String> {
    setting_value(settings, "CollectionSystemsCustom")
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Rewrite `CollectionSystemsCustom` so every named collection is shown.
///
/// `wanted` are server names; each goes in under [`es_name`], the name its file
/// has. Returns the new file when it had to change, `None` when it already
/// said so.
pub fn show_collections(settings: &str, wanted: &[String]) -> Option<String> {
    let mut names: BTreeSet<String> = enabled_collections(settings).into_iter().collect();
    let before = names.len();
    names.extend(wanted.iter().map(|w| es_name(w)));
    if names.len() == before {
        return None;
    }
    let joined = names.into_iter().collect::<Vec<_>>().join(",");
    Some(set_setting(settings, "CollectionSystemsCustom", &joined))
}

/// `es_settings.cfg` updated so these collections show, or `None` when it
/// already shows them all.
///
/// A missing file starts from an empty `<config>`. Any other failure to read it
/// is an error: read as empty, the rewrite would be a settings file holding
/// one line, and ES would come up with every other setting at its default.
pub fn settings_showing(path: &Path, wanted: &[String]) -> Result<Option<String>> {
    if wanted.is_empty() {
        return Ok(None);
    }
    let settings = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "<?xml version=\"1.0\"?>\n<config>\n</config>\n".to_owned()
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(show_collections(&settings, wanted))
}

/// Write what [`settings_showing`] returned, after backing up the old file.
pub fn write_settings(path: &Path, body: &str, backups: &Path) -> Result<()> {
    replace(path, body, backups)
}

/// ES settings are `<string name="Key" value="..." />` lines.
fn setting_value(settings: &str, key: &str) -> Option<String> {
    let needle = format!("name=\"{key}\"");
    let at = settings.find(&needle)?;
    let rest = &settings[at + needle.len()..];
    let v = rest.find("value=\"")? + "value=\"".len();
    let end = rest[v..].find('"')? + v;
    Some(unescape(&rest[v..end]))
}

fn set_setting(settings: &str, key: &str, value: &str) -> String {
    let needle = format!("name=\"{key}\"");
    let line = format!("\t<string name=\"{key}\" value=\"{}\" />", escape_attr(value));
    let Some(at) = settings.find(&needle) else {
        // Not there at all: add it before the closing tag.
        return match settings.rfind("</config>") {
            Some(close) => {
                let mut out = settings.to_owned();
                out.insert_str(close, &format!("{line}\n"));
                out
            }
            None => format!("{settings}{line}\n"),
        };
    };
    let start = settings[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = settings[at..].find('\n').map_or(settings.len(), |i| at + i);
    let mut out = settings.to_owned();
    out.replace_range(start..end, &line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block shaped like the ones actually on the handheld — tabs, scraped
    /// tags, RetroAchievements hashes and all.
    const REAL: &str = "<?xml version=\"1.0\"?>\n<gameList>\n\
\t<game>\n\
\t\t<path>./Avenging Spirit.gb</path>\n\
\t\t<name>Avenging Spirit</name>\n\
\t\t<desc>A ghost grabbing bodies.</desc>\n\
\t\t<image>./images/Avenging Spirit-image.png</image>\n\
\t\t<rating>0.74</rating>\n\
\t\t<favorite>true</favorite>\n\
\t\t<cheevosHash>E88EAB57AB4614966748280BF3C97F52</cheevosHash>\n\
\t</game>\n\
\t<game>\n\
\t\t<path>./Tetris.gb</path>\n\
\t\t<name>Tetris</name>\n\
\t\t<desc>Blocks.</desc>\n\
\t</game>\n\
</gameList>\n";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-eslist-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Somewhere for the tests that are not about backups to put theirs.
    fn bk() -> PathBuf {
        std::env::temp_dir().join("moose-eslist-backups")
    }

    fn loaded(name: &str, text: &str) -> Gamelist {
        let p = scratch(name).join("gamelist.xml");
        std::fs::write(&p, text).unwrap();
        Gamelist::load(&p).unwrap()
    }

    #[test]
    fn it_reads_the_stars_that_are_there() {
        let list = loaded("reads", REAL);
        assert_eq!(list.favorites(), ["Avenging Spirit.gb".to_owned()].into());
        assert_eq!(list.known().len(), 2);
    }

    #[test]
    fn everything_scraped_survives_a_star() {
        // The reason this is text surgery. A parse-and-rewrite drops any tag
        // the program has not heard of, and <cheevosHash> is one nobody would
        // think to keep — losing it costs a re-scrape of the whole library.
        let mut list = loaded("survives", REAL);
        assert!(list.set_favorite("Tetris.gb", true));
        assert!(list.text.contains("<cheevosHash>E88EAB57AB4614966748280BF3C97F52</cheevosHash>"));
        assert!(list.text.contains("<desc>A ghost grabbing bodies.</desc>"));
        assert!(list.text.contains("<image>./images/Avenging Spirit-image.png</image>"));
        assert_eq!(
            list.favorites(),
            ["Avenging Spirit.gb".to_owned(), "Tetris.gb".to_owned()].into()
        );
    }

    #[test]
    fn unstarring_takes_the_tag_out_and_leaves_no_blank_line() {
        let mut list = loaded("unstar", REAL);
        assert!(list.set_favorite("Avenging Spirit.gb", false));
        assert!(list.favorites().is_empty());
        assert!(!list.text.contains("<favorite>"));
        assert!(!list.text.contains("\n\n"), "left a hole where the tag was");
        // and the rest of the block is untouched
        assert!(list.text.contains("<rating>0.74</rating>"));
    }

    #[test]
    fn a_star_goes_in_at_the_indentation_of_its_neighbours() {
        let mut list = loaded("indent", REAL);
        list.set_favorite("Tetris.gb", true);
        assert!(
            list.text.contains("\t\t<favorite>true</favorite>\n\t</game>"),
            "not indented like the tags above it:\n{}",
            list.text
        );
    }

    #[test]
    fn setting_what_is_already_set_changes_nothing() {
        // A sync runs over every game every time. If agreeing counted as a
        // change, every run would rewrite nine gamelists for no reason.
        let mut list = loaded("noop", REAL);
        assert!(!list.set_favorite("Avenging Spirit.gb", true));
        assert!(!list.set_favorite("Tetris.gb", false));
        assert!(!list.changed());
        assert!(!list.save(&bk()).unwrap());
    }

    #[test]
    fn a_game_es_never_scraped_still_gets_starred() {
        let mut list = loaded("unscraped", REAL);
        assert!(list.set_favorite("Kirby's Dream Land.gb", true));
        assert!(list.favorites().contains("Kirby's Dream Land.gb"));
        assert!(list.text.contains("<name>Kirby's Dream Land</name>"));
        assert!(list.text.ends_with("</gameList>\n"), "block went outside the list");
    }

    #[test]
    fn a_name_with_an_ampersand_does_not_break_the_file() {
        let mut list = loaded("amp", REAL);
        list.set_favorite("Tom & Jerry.gb", true);
        assert!(list.text.contains("./Tom &amp; Jerry.gb"));
        // and it reads back as it went in
        assert!(list.favorites().contains("Tom & Jerry.gb"));
    }

    #[test]
    fn a_system_with_no_gamelist_at_all_can_still_be_starred() {
        let p = scratch("fresh").join("gamelist.xml");
        let mut list = Gamelist::load_or_empty(&p).unwrap();
        assert!(list.set_favorite("Super Mario Land.gb", true));
        assert!(list.save(&bk()).unwrap());
        assert_eq!(
            Gamelist::load(&p).unwrap().favorites(),
            ["Super Mario Land.gb".to_owned()].into()
        );
    }

    #[test]
    fn a_save_lands_whole_or_not_at_all() {
        // ES re-reads these on a timer; a half-written one is a system that
        // opens empty.
        let p = scratch("atomic").join("gamelist.xml");
        std::fs::write(&p, REAL).unwrap();
        let mut list = Gamelist::load(&p).unwrap();
        list.set_favorite("Tetris.gb", true);
        assert!(list.save(&bk()).unwrap());
        assert!(!p.with_extension("xml.moose").exists(), "left its temporary behind");
        assert_eq!(Gamelist::load(&p).unwrap().favorites().len(), 2);
    }

    #[test]
    fn a_collection_file_is_paths_one_per_line() {
        let dir = scratch("coll");
        let p = dir.join(CollectionFile::file_name("Arcade Fighting"));
        assert_eq!(p.file_name().unwrap(), "custom-Arcade Fighting.cfg");
        std::fs::write(&p, "/userdata/roms/fbneo/64street.zip\n\n# note\n/userdata/roms/fbneo/aodk.zip\n").unwrap();
        let mut c = CollectionFile::load(&p).unwrap();
        assert_eq!(c.entries.len(), 2, "blank and commented lines are not games");
        c.entries.insert(PathBuf::from("/userdata/roms/fbneo/aliencha.zip"));
        assert!(c.save(&bk()).unwrap());
        assert_eq!(CollectionFile::load(&p).unwrap().entries.len(), 3);
        assert!(!c.save(&bk()).unwrap(), "rewrote a file that already said this");
    }

    #[test]
    fn a_collection_named_with_a_slash_does_not_become_a_directory() {
        assert_eq!(
            CollectionFile::file_name("Shmups / Vertical"),
            "custom-Shmups - Vertical.cfg"
        );
    }

    #[test]
    fn a_collection_file_nobody_has_made_yet_reads_as_empty() {
        let p = scratch("missing").join("custom-Nothing.cfg");
        assert!(CollectionFile::load(&p).unwrap().entries.is_empty());
    }

    const SETTINGS: &str = "<?xml version=\"1.0\"?>\n<config>\n\
\t<string name=\"CollectionSystemsCustom\" value=\"Arcade Fighting,Arcade Maze\" />\n\
\t<string name=\"ThemeSet\" value=\"knulli\" />\n\
</config>\n";

    #[test]
    fn a_collection_file_is_invisible_until_es_is_told_to_show_it() {
        // The forgotten step: a perfectly good custom-*.cfg shows nothing at
        // all until its name is in this one setting.
        assert_eq!(
            enabled_collections(SETTINGS),
            vec!["Arcade Fighting".to_owned(), "Arcade Maze".to_owned()]
        );
        let out = show_collections(SETTINGS, &["★ Best of snes".to_owned()]).unwrap();
        assert!(out.contains("Arcade Fighting"), "dropped one that was already shown");
        assert!(out.contains("★ Best of snes"));
        assert!(out.contains("<string name=\"ThemeSet\" value=\"knulli\" />"), "ate another setting");
    }

    #[test]
    fn telling_es_what_it_already_shows_rewrites_nothing() {
        assert!(show_collections(SETTINGS, &["Arcade Maze".to_owned()]).is_none());
    }

    /// ES writes `<game id="…" source="…">` for a game it scraped itself.
    /// Looking only for the bare tag read every one of those as absent: a
    /// star on the card that never reached the server, and a star from the
    /// server that got a second block for a game that already had one.
    #[test]
    fn a_game_block_with_attributes_is_still_a_game() {
        let text = "<?xml version=\"1.0\"?>\n<gameList>\n\
\t<game id=\"4231\" source=\"ScreenScraper.fr\">\n\
\t\t<path>./Tetris.gb</path>\n\
\t\t<name>Tetris</name>\n\
\t\t<favorite>true</favorite>\n\
\t</game>\n\
\t<game\tid=\"12\">\n\
\t\t<path>./Dr. Mario.gb</path>\n\
\t</game>\n\
</gameList>\n";
        let mut list = loaded("attrs", text);
        assert_eq!(list.favorites(), ["Tetris.gb".to_owned()].into());
        assert_eq!(list.known().len(), 2);
        assert!(list.set_favorite("Dr. Mario.gb", true));
        assert_eq!(list.text.matches("<path>./Dr. Mario.gb</path>").count(), 1, "added a second block");
        assert!(list.set_favorite("Tetris.gb", false));
        assert_eq!(list.favorites(), ["Dr. Mario.gb".to_owned()].into());
        // `<gameList>` itself is not a game.
        assert!(loaded("attrs-empty", "<gameList>\n</gameList>\n").known().is_empty());
    }

    /// A game in `snes/Aftermarket/` is `./Aftermarket/Foo.sfc` to ES, and
    /// its star goes on that path, not on a same-named game at the top.
    #[test]
    fn a_game_in_a_subfolder_is_starred_by_its_path() {
        let text = "<gameList>\n\t<game>\n\t\t<path>./Foo.sfc</path>\n\t</game>\n</gameList>\n";
        let mut list = loaded("subfolder", text);
        assert!(list.set_favorite("Aftermarket/Foo.sfc", true));
        assert_eq!(list.favorites(), ["Aftermarket/Foo.sfc".to_owned()].into());
        assert!(list.text.contains("<path>./Aftermarket/Foo.sfc</path>"));
        assert!(list.text.contains("<name>Foo</name>"), "the name is the file's, not the folder's");
    }

    /// A gamelist that cannot be read is an error. Read as empty it has no
    /// stars, and every star the server has for this system reads as taken
    /// off here.
    #[test]
    fn an_unreadable_gamelist_is_an_error_not_an_empty_list() {
        let p = scratch("bad-gamelist").join("gamelist.xml");
        std::fs::write(&p, b"<gameList>\n\xff\xfe</gameList>\n").unwrap();
        assert!(Gamelist::load_if_present(&p).is_err());
        assert!(Gamelist::load_or_empty(&p).is_err());
        assert!(Gamelist::load_if_present(&p.with_file_name("none.xml")).unwrap().is_none());
    }

    /// Somebody's notes in a `custom-*.cfg` are still there after a sync,
    /// and so is the order.
    #[test]
    fn comments_and_order_in_a_collection_file_survive_a_save() {
        let dir = scratch("coll-comments");
        let p = dir.join("custom-Arcade Maze.cfg");
        std::fs::write(
            &p,
            "# picked by hand\n/userdata/roms/fbneo/pacman.zip\n\n# the hard ones\n/userdata/roms/fbneo/digdug.zip\n/userdata/roms/fbneo/amidar.zip\n",
        )
        .unwrap();
        let mut c = CollectionFile::load(&p).unwrap();
        c.entries.remove(Path::new("/userdata/roms/fbneo/digdug.zip"));
        c.entries.insert(PathBuf::from("/userdata/roms/fbneo/alibaba.zip"));
        assert!(c.save(&bk()).unwrap());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "# picked by hand\n/userdata/roms/fbneo/pacman.zip\n\n# the hard ones\n/userdata/roms/fbneo/amidar.zip\n/userdata/roms/fbneo/alibaba.zip\n"
        );
    }

    /// A cfg that cannot be read is an error, never an empty collection.
    #[test]
    fn an_unreadable_collection_file_is_an_error() {
        let p = scratch("coll-bad").join("custom-X.cfg");
        std::fs::write(&p, b"/userdata/roms/fbneo/\xff.zip\n").unwrap();
        assert!(CollectionFile::load(&p).is_err());
    }

    /// A collection name with a quote in it went into the attribute bare, and
    /// ES could not read the rest of `es_settings.cfg`.
    #[test]
    fn a_quote_in_a_collection_name_is_escaped_in_the_settings() {
        let settings = "<?xml version=\"1.0\"?>\n<config>\n\
\t<string name=\"CollectionSystemsCustom\" value=\"Best &quot;Hard&quot; Ones,Arcade Maze\" />\n\
</config>\n";
        assert_eq!(
            enabled_collections(settings),
            vec!["Best \"Hard\" Ones".to_owned(), "Arcade Maze".to_owned()]
        );
        let out = show_collections(settings, &["Say \"Hi\"".to_owned()]).unwrap();
        assert!(
            out.contains("value=\"Arcade Maze,Best &quot;Hard&quot; Ones,Say &quot;Hi&quot;\""),
            "{out}"
        );
        assert_eq!(enabled_collections(&out).len(), 3);
    }

    /// ES splits the setting on commas with no escape, and takes a
    /// collection's name from its file. A comma in a server name becomes `-`
    /// in both, so the file written is the one the setting shows.
    #[test]
    fn a_comma_in_a_collection_name_does_not_split_it_in_two() {
        let out = show_collections(SETTINGS, &["Shmups, Vertical".to_owned()]).unwrap();
        let shown = enabled_collections(&out);
        assert_eq!(shown.len(), 3, "{shown:?}");
        assert!(shown.contains(&"Shmups- Vertical".to_owned()));
        assert_eq!(CollectionFile::file_name("Shmups, Vertical"), "custom-Shmups- Vertical.cfg");
    }

    /// A settings file that cannot be read must not be rewritten as one line.
    #[test]
    fn settings_that_cannot_be_read_are_not_rewritten() {
        let dir = scratch("settings-bad");
        let p = dir.join("es_settings.cfg");
        std::fs::write(&p, b"<config>\n\t<string name=\"A\" value=\"\xff\" />\n</config>\n").unwrap();
        assert!(settings_showing(&p, &["Arcade Maze".to_owned()]).is_err());
        // Missing is a fresh file, and a well-formed one.
        let fresh = settings_showing(&dir.join("none.cfg"), &["Arcade Maze".to_owned()]).unwrap().unwrap();
        assert!(fresh.contains("<config>") && fresh.contains("</config>"), "{fresh}");
        assert_eq!(enabled_collections(&fresh), vec!["Arcade Maze".to_owned()]);
    }

    /// Every rewrite keeps the old file, ten deep, like the save backups.
    #[test]
    fn a_rewrite_keeps_the_old_file_and_the_oldest_copies_go() {
        let dir = scratch("backup");
        let backups = dir.join("es-backup");
        let p = dir.join("snes").join("gamelist.xml");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        // Nothing there yet: nothing to keep, and not an error.
        back_up(&backups, &p).unwrap();
        assert!(!backups.exists() || std::fs::read_dir(backups.join("snes")).map_or(0, |d| d.count()) == 0);

        let t0 = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for i in 0..12u64 {
            std::fs::write(&p, format!("<gameList>{i}</gameList>")).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(t0 + std::time::Duration::from_secs(i)).unwrap();
            back_up(&backups, &p).unwrap();
        }
        // Unchanged since the last copy: no new one.
        back_up(&backups, &p).unwrap();
        let mut kept: Vec<String> = std::fs::read_dir(backups.join("snes"))
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        kept.sort();
        assert_eq!(kept.len(), KEEP);
        assert!(kept.contains(&"<gameList>11</gameList>".to_owned()));
        assert!(!kept.contains(&"<gameList>0</gameList>".to_owned()), "the oldest should have gone");

        // And a save goes through it.
        let before = "<gameList>\n\t<game>\n\t\t<path>./A.sfc</path>\n\t</game>\n</gameList>\n";
        std::fs::write(&p, before).unwrap();
        let mut list = Gamelist::load(&p).unwrap();
        assert!(list.set_favorite("A.sfc", true));
        assert!(list.save(&backups).unwrap());
        assert_ne!(std::fs::read_to_string(&p).unwrap(), before, "setup: the save changed nothing");
        let copies: Vec<String> = std::fs::read_dir(backups.join("snes"))
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert!(copies.iter().any(|c| c == before), "the file a save replaced was not kept");
    }

    #[test]
    fn a_settings_file_without_the_line_gets_one() {
        let bare = "<?xml version=\"1.0\"?>\n<config>\n\t<string name=\"ThemeSet\" value=\"knulli\" />\n</config>\n";
        let out = show_collections(bare, &["Arcade Maze".to_owned()]).unwrap();
        assert!(out.contains("name=\"CollectionSystemsCustom\" value=\"Arcade Maze\""));
        assert!(out.contains("</config>"), "lost the closing tag");
        assert_eq!(enabled_collections(&out), vec!["Arcade Maze".to_owned()]);
    }
}

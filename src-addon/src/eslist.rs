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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One system's `gamelist.xml`, held as the text it is.
pub struct Gamelist {
    path: PathBuf,
    text: String,
    dirty: bool,
}

/// Where a `<game>` block sits, and what it says.
struct Entry {
    /// The ROM's file name, with ES's leading `./` taken off.
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

    pub fn load_or_empty(path: &Path) -> Result<Self> {
        if path.exists() { Self::load(path) } else { Ok(Self::empty(path)) }
    }

    /// Every `<game>` block, in the order they appear.
    fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(start) = self.text[from..].find("<game>").map(|i| i + from) {
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

    /// The ROM file names ES has starred.
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
        let name = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
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

    /// Write it back, but only if something moved.
    ///
    /// Through a temporary file in the same directory: ES reads these on a
    /// timer, and a half-written gamelist is a system that opens empty.
    pub fn save(&self) -> Result<bool> {
        if !self.dirty {
            return Ok(false);
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let tmp = self.path.with_extension("xml.moose");
        std::fs::write(&tmp, &self.text)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("replacing {}", self.path.display()))?;
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

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

// --- Custom collections -----------------------------------------------------

/// One `custom-<name>.cfg`: absolute ROM paths, one per line.
pub struct CollectionFile {
    path: PathBuf,
    pub entries: BTreeSet<PathBuf>,
}

impl CollectionFile {
    /// What ES calls the file for a collection of this name.
    pub fn file_name(collection: &str) -> String {
        // `/` would make it a directory, and ES has no escaping for it.
        format!("custom-{}.cfg", collection.replace('/', "-"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let entries = match std::fs::read_to_string(path) {
            Ok(text) => text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(PathBuf::from)
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { path: path.to_path_buf(), entries })
    }

    /// Write it back, sorted, only when it differs from what is there.
    pub fn save(&self) -> Result<bool> {
        let body = self
            .entries
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        let body = if body.is_empty() { body } else { format!("{body}\n") };
        if std::fs::read_to_string(&self.path).is_ok_and(|old| old == body) {
            return Ok(false);
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let tmp = self.path.with_extension("cfg.moose");
        std::fs::write(&tmp, &body).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("replacing {}", self.path.display()))?;
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
/// Returns the new file when it had to change, `None` when it already said so.
pub fn show_collections(settings: &str, wanted: &[String]) -> Option<String> {
    let mut names: BTreeSet<String> = enabled_collections(settings).into_iter().collect();
    let before = names.len();
    names.extend(wanted.iter().cloned());
    if names.len() == before {
        return None;
    }
    let joined = names.into_iter().collect::<Vec<_>>().join(",");
    Some(set_setting(settings, "CollectionSystemsCustom", &joined))
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
    let line = format!("\t<string name=\"{key}\" value=\"{}\" />", escape(value));
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
        assert!(!list.save().unwrap());
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
        assert!(list.save().unwrap());
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
        assert!(list.save().unwrap());
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
        assert!(c.save().unwrap());
        assert_eq!(CollectionFile::load(&p).unwrap().entries.len(), 3);
        assert!(!c.save().unwrap(), "rewrote a file that already said this");
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

    #[test]
    fn a_settings_file_without_the_line_gets_one() {
        let bare = "<?xml version=\"1.0\"?>\n<config>\n\t<string name=\"ThemeSet\" value=\"knulli\" />\n</config>\n";
        let out = show_collections(bare, &["Arcade Maze".to_owned()]).unwrap();
        assert!(out.contains("name=\"CollectionSystemsCustom\" value=\"Arcade Maze\""));
        assert!(out.contains("</config>"), "lost the closing tag");
        assert_eq!(enabled_collections(&out), vec!["Arcade Maze".to_owned()]);
    }
}

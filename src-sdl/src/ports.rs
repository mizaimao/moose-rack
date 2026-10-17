// Ports and Tools, as KNULLI actually defines them.
//
// The first version of this file was wrong twice over, and both mistakes came
// from guessing at a structure instead of reading it:
//
//   * **Ports and Tools are `<group>`s, not folders.** `/userdata/roms/tools`
//     holds nothing but a readme; the Tools *group* is `odcommander` and
//     `vaixterm`, each its own system with its own directory. Scanning the
//     folder found an empty folder and reported, wrongly, that there was
//     nothing there — while the device plainly showed two.
//   * **Every member has its own extensions.** `.sh` is only what the `ports`
//     system itself uses. Its siblings use `.wad`, `.game`, `.rtcw`, `.odc`,
//     `.vxt`, `.zip` and more, so a scan looking for shell scripts finds one
//     system out of ten.
//
// So this reads `es_systems.cfg` — the same table KNULLI's own front end reads —
// and takes the group membership, the paths and the extensions from it.

use std::path::{Path, PathBuf};

/// One entry a group offers.
pub struct Port {
    pub name: String,
    pub path: PathBuf,
    /// Which system it belongs to. `emulatorlauncher` is told this, and the
    /// members of a group are different systems.
    pub system: String,
    /// Box art or a screenshot, when the gamelist named one that exists.
    pub image: Option<PathBuf>,
}

/// One system out of `es_systems.cfg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System {
    pub name: String,
    pub path: PathBuf,
    /// Lower-case, without the dot.
    pub extensions: Vec<String>,
    pub group: Option<String>,
}

/// The groups worth showing beside the consoles, and what to call them.
///
/// Not every group: `megadrive` and `nes` are groups too, and those are
/// consoles the library already knows about. These three hold the things that
/// are not RomM platforms at all.
pub const GROUPS: [(&str, &str); 3] = [
    ("ports", "Ports"),
    ("tools", "Tools"),
    ("emulators", "Emulators"),
];

/// Parse `es_systems.cfg`.
///
/// Hand-rolled for the same reason `esde.rs` hand-rolls its gamelist reader: the
/// format is flat, and one dependency for six fields is not a trade worth making
/// on a device with a gigabyte of memory.
pub fn systems(cfg: &Path) -> Vec<System> {
    let Ok(text) = std::fs::read_to_string(cfg) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in text.split("<system>").skip(1) {
        let block = block.split("</system>").next().unwrap_or(block);
        let (Some(name), Some(path)) = (tag(block, "name"), tag(block, "path")) else {
            continue;
        };
        let extensions = tag(block, "extension")
            .unwrap_or_default()
            .split_whitespace()
            .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        out.push(System {
            name,
            path: PathBuf::from(path),
            extensions,
            group: tag(block, "group"),
        });
    }
    out
}

/// Everything in one group, across every system that belongs to it.
///
/// Sorted by name, because a group is a flat list to whoever is looking at it —
/// that Doom and Abuse come from different systems is a launching detail, not
/// something to organize the screen around.
pub fn scan_group(all: &[System], group: &str) -> Vec<Port> {
    let mut out: Vec<Port> = all
        .iter()
        .filter(|s| s.group.as_deref() == Some(group) || s.name == group)
        .flat_map(scan_system)
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// One system's entries.
fn scan_system(system: &System) -> Vec<Port> {
    let names = gamelist_names(&system.path.join("gamelist.xml"));
    std::fs::read_dir(&system.path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            if !system.extensions.contains(&ext) {
                return None;
            }
            let file = path.file_name()?.to_str()?.to_owned();
            let stem = path.file_stem()?.to_str()?.to_owned();
            let (name, image) = names
                .iter()
                .find(|(p, _, _)| *p == file)
                .map(|(_, n, i)| (n.clone(), i.clone()))
                .unwrap_or((stem, None));
            Some(Port {
                name,
                system: system.name.clone(),
                image: image
                    .map(|i| system.path.join(i.trim_start_matches("./")))
                    .filter(|p| p.is_file()),
                path,
            })
        })
        .collect()
}

/// `(file name, display name, image)` out of a gamelist.
fn gamelist_names(path: &Path) -> Vec<(String, String, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in game_blocks(&text) {
        let Some(raw) = tag(block, "path") else {
            continue;
        };
        let file = raw.rsplit(['/', '\\']).next().unwrap_or(&raw).to_owned();
        let name = tag(block, "name").unwrap_or_else(|| file.clone());
        out.push((file, name, tag(block, "image")));
    }
    out
}

/// The inside of every `<game>` block, with or without attributes.
///
/// ES writes `<game id="1234" source="ScreenScraper">` for a game it scraped
/// itself, so splitting on the bare `<game>` skipped every one of those and
/// showed the file name instead of the scraped name and picture. The same fix
/// as `next_game` in moose-patch's `eslist.rs`. `<gameList>` is not one.
fn game_blocks(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("<game") {
        let start = from + i + "<game".len();
        from = start;
        if !text[start..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c.is_whitespace())
        {
            continue;
        }
        let Some(open_end) = text[start..].find('>').map(|i| start + i + 1) else {
            break;
        };
        // `<game id="1" />` has nothing inside, and reading on to the next
        // `</game>` would take the following game's tags for its own.
        if text[..open_end].ends_with("/>") {
            continue;
        }
        let Some(end) = text[open_end..].find("</game>").map(|i| open_end + i) else {
            break;
        };
        out.push(&text[open_end..end]);
        from = end + "</game>".len();
    }
    out
}

fn tag(block: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    let value = unescape(block[start..end].trim());
    (!value.is_empty()).then_some(value)
}

/// The five entities ES writes. `&amp;` goes last, so `&amp;lt;` comes out as
/// the text `&lt;` and not as `<`.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Launch one, the way KNULLI does.
///
/// Through `emulatorlauncher` rather than by running the file — most of these
/// are not executable at all. A `.wad` is data for prboom and a `.game` is data
/// for abuse; only the `ports` system's own entries are shell scripts, which is
/// the other half of the mistake this file used to make.
pub fn launch(port: &Port) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("emulatorlauncher")
        .arg("-system")
        .arg(&port.system)
        .arg("-rom")
        .arg(&port.path)
        .status()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three real systems out of the Flip's own `es_systems.cfg`, with the ROM
    /// paths rewritten so the test can point them at a scratch directory.
    const FRAGMENT: &str = include_str!("../tests/data/es_systems_fragment.cfg");

    fn fixture(root: &Path) -> Vec<System> {
        let text = FRAGMENT.replace("ROMS", &root.to_string_lossy());
        let path = root.join("es_systems.cfg");
        std::fs::write(&path, text).unwrap();
        systems(&path)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-groups-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The real file parses into systems with their group and extensions.
    #[test]
    fn the_devices_own_table_parses() {
        let dir = scratch("parse");
        let all = fixture(&dir);
        let by = |n: &str| all.iter().find(|s| s.name == n).cloned();

        let od = by("odcommander").expect("odcommander is in the fragment");
        assert_eq!(od.group.as_deref(), Some("tools"), "OD-Commander is a Tool");
        assert_eq!(od.extensions, ["odc"], "the leading dot was not stripped");

        let vx = by("vaixterm").expect("vaixterm is in the fragment");
        assert_eq!(vx.group.as_deref(), Some("tools"));

        let pr = by("prboom").expect("prboom is in the fragment");
        assert_eq!(pr.group.as_deref(), Some("ports"));
        assert!(
            pr.extensions.contains(&"wad".to_owned()),
            "{:?}",
            pr.extensions
        );
    }

    /// Tools is OD-Commander and VaixTerm — two systems, not the empty `tools`
    /// folder.
    ///
    /// This is the bug the file was rewritten for: scanning
    /// `/userdata/roms/tools` finds a readme and reports Tools empty, while the
    /// device plainly shows two.
    #[test]
    fn tools_is_two_systems_and_not_the_empty_tools_folder() {
        let dir = scratch("tools");
        let all = fixture(&dir);
        for (system, file) in [
            ("odcommander", "odcommander.odc"),
            ("vaixterm", "vaixterm.vxt"),
        ] {
            let d = dir.join(system);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(file), "x").unwrap();
        }
        // An empty `tools` folder, exactly as the device has it.
        std::fs::create_dir_all(dir.join("tools")).unwrap();
        std::fs::write(dir.join("tools/_info.txt"), "readme").unwrap();

        let found = scan_group(&all, "tools");
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["odcommander", "vaixterm"],
            "Tools came back empty again"
        );
        assert_eq!(
            found[0].system, "odcommander",
            "the launcher needs the member system"
        );
    }

    /// Each system's own extensions are honored. `.sh` is only the `ports`
    /// system's; a scan looking for shell scripts finds one system in ten.
    #[test]
    fn each_system_matches_its_own_extensions() {
        let dir = scratch("exts");
        let all = fixture(&dir);
        let d = dir.join("prboom");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("doom.wad"), "x").unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        std::fs::write(d.join("run.sh"), "x").unwrap();

        let found = scan_group(&all, "ports");
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["doom"], "a .sh in prboom is not a prboom game");
    }

    /// A gamelist name and picture win over the file name, and a picture that is
    /// not on disk is no picture.
    #[test]
    fn the_gamelist_names_what_it_can() {
        let dir = scratch("gamelist");
        let all = fixture(&dir);
        let d = dir.join("odcommander");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("odcommander.odc"), "x").unwrap();
        std::fs::write(
            d.join("gamelist.xml"),
            "<gameList><game><path>./odcommander.odc</path>\
             <name>OD-Commander</name><image>./art.png</image></game></gameList>",
        )
        .unwrap();

        let found = scan_group(&all, "tools");
        assert_eq!(
            found[0].name, "OD-Commander",
            "the gamelist name was ignored"
        );
        assert_eq!(found[0].image, None, "a picture that is not there was kept");

        std::fs::write(d.join("art.png"), "x").unwrap();
        assert!(
            scan_group(&all, "tools")[0].image.is_some(),
            "a real picture was dropped"
        );
    }

    /// A block ES scraped itself opens `<game id=".." source="..">`, and its
    /// text is escaped. Both used to come out wrong: the block was skipped, so
    /// the Port showed its file name, and a name with `&` in it showed `&amp;`.
    #[test]
    fn a_scraped_block_with_attributes_and_entities_is_read() {
        let dir = scratch("gamelist-attrs");
        let all = fixture(&dir);
        let d = dir.join("odcommander");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Tom & Jerry.odc"), "x").unwrap();
        std::fs::write(d.join("vaix.odc"), "x").unwrap();
        std::fs::write(
            d.join("gamelist.xml"),
            "<gameList>\n\
             \t<game id=\"4231\" source=\"ScreenScraper\">\n\
             \t\t<path>./Tom &amp; Jerry.odc</path>\n\
             \t\t<name>Tom &amp; Jerry &lt;Demo&gt;</name>\n\
             \t</game>\n\
             \t<game\tid=\"12\">\n\
             \t\t<path>./vaix.odc</path>\n\
             \t\t<name>Vaix</name>\n\
             \t</game>\n\
             </gameList>\n",
        )
        .unwrap();

        let found = scan_group(&all, "tools");
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Tom & Jerry <Demo>", "Vaix"]);
    }

    /// `<gameList>` is not a game, and an empty `<game />` does not take the
    /// next block's tags.
    #[test]
    fn only_real_game_blocks_are_read() {
        let text = "<gameList>\n<game id=\"1\" />\n<game><path>./b.sh</path></game>\n</gameList>";
        let blocks = game_blocks(text);
        assert_eq!(blocks, ["<path>./b.sh</path>"]);
    }

    /// A group nothing belongs to is empty rather than an error, and an
    /// unreadable table is no systems rather than a panic.
    #[test]
    fn a_missing_table_or_group_is_simply_empty() {
        assert!(systems(Path::new("/nonexistent/es_systems.cfg")).is_empty());
        let dir = scratch("none");
        let all = fixture(&dir);
        assert!(scan_group(&all, "nothing-is-in-this").is_empty());
    }
}

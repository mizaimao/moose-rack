//! Reading a local ES-DE library instead of a RomM server.
//!
//! The app was built around a remote server: sync metadata, download ROMs,
//! fetch artwork. An ES-DE install on a local disk already has all three, so
//! nothing needs downloading — only indexing.
//!
//! ES-DE's layout, and the one trap in it:
//!
//! ```text
//! <roms>/<system>/<game files>
//! <esde>/gamelists/<system>/gamelist.xml        names, genre, players, rating
//! <esde>/downloaded_media/<system>/<type>/<game stem>.<ext>
//! ```
//!
//! **`<system>` is ES-DE's name, not a RomM slug** — `dreamcast` vs `dc`,
//! `neogeo` vs `neogeoaes`, `megadrive` vs `megadrive`. The mapping already
//! exists in `data/esde-core-map.json`, which is what makes this cheap: the
//! same file that tells us which core runs a platform also tells us which
//! ES-DE system feeds it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::coremap::CoreMap;

/// ES-DE system directories the core map does not cover.
///
/// The map is built from ES-DE's Android system list, and a real install does
/// not match it exactly: `genesis` is the same console the map calls
/// `megadrive` — 942 games on this card, and the single biggest thing a
/// missing alias would drop. The rest are systems RomM has platforms for but
/// the Android export never listed.
const SYSTEM_ALIASES: &[(&str, &str)] = &[
    ("genesis", "megadrive"),
    ("megadrive", "megadrive"),
    ("gameandwatch", "g-and-w"),
    ("n3ds", "new-nintendo-3ds"),
    ("pico-8", "pico8"),
    ("easyrpg", "easyrpg"),
    ("ps2", "ps2"),
    ("ps3", "ps3"),
    ("switch", "switch"),
    ("wii", "wii"),
    ("wiiu", "wiiu"),
    ("saturn", "saturn"),
    ("xbox360", "xbox360"),
    ("naomi", "naomi"),
];

/// Directories inside the ROMs folder that are not systems.
///
/// A BIOS folder scanned as a system would invent 176 phantom "games" and,
/// worse, hand them to a core that cannot run them.
const NOT_SYSTEMS: &[&str] = &["0_BIOS", "bios", "ports", "SourcePorts", "Ports"];

/// One game found on disk.
#[derive(Debug, Clone, Default)]
pub struct Game {
    /// RomM platform slug, so the rest of the app is unchanged.
    pub platform_slug: String,
    /// ES-DE system directory name, needed to find its media.
    pub system: String,
    pub name: String,
    pub fs_name: String,
    /// Where this sits inside the system directory: `""` at the top,
    /// `"Aftermarket"` or `"AdditionalRoms/Homebrew"` below it.
    ///
    /// ES-DE walks into subfolders and shows them as folders, so a library
    /// that files its homebrew that way has games several levels down. This is
    /// the path that makes them findable again — the front ends draw a folder
    /// per distinct value, and the gamelist and media lookups key off it.
    pub rel_dir: String,
    /// Absolute path — ES-DE libraries live wherever the user put them, so
    /// nothing may be assumed about a layout relative to the project.
    pub path: PathBuf,
    pub size_bytes: i64,
    pub summary: Option<String>,
    pub genres: Vec<String>,
    pub players: Option<String>,
    /// 0–100, converted from ES-DE's 0.0–1.0 so it matches RomM's scale.
    pub rating: Option<f64>,
    pub release_year: Option<i32>,
}

/// Where the pieces of an ES-DE install are.
#[derive(Debug, Clone)]
pub struct Layout {
    pub roms: PathBuf,
    pub gamelists: PathBuf,
    pub media: PathBuf,
}

impl Layout {
    /// Derive the layout from an ES-DE data directory, allowing an explicit
    /// ROMs directory since ES-DE keeps that separate and configurable.
    pub fn new(esde_root: &Path, roms: Option<&Path>) -> Self {
        Self {
            roms: roms.map(Path::to_path_buf).unwrap_or_else(|| esde_root.join("ROMs")),
            gamelists: esde_root.join("gamelists"),
            media: esde_root.join("downloaded_media"),
        }
    }
}

/// Metadata for one game out of a gamelist.
#[derive(Default, Clone)]
struct Entry {
    name: Option<String>,
    desc: Option<String>,
    genre: Option<String>,
    players: Option<String>,
    rating: Option<f64>,
    year: Option<i32>,
}

/// Parse `gamelist.xml` into `path stem -> metadata`.
///
/// Hand-rolled rather than pulled in as an XML dependency: the format is flat,
/// and the only awkward part is that `<path>` is relative and usually prefixed
/// `./`, so the prefix is stripped and the extension dropped for matching.
///
/// Keyed by the path relative to the system directory, not by file name.
/// `<path>./Aftermarket/Blow'em Out!.zip` and a top-level `Blow'em Out!.zip`
/// are two different games, and keying on the file name alone let one
/// overwrite the other's name and description. The bare name is inserted as a
/// second key when it is still free, so a gamelist that writes a nested game
/// without its folder is still matched.
fn parse_gamelist(path: &Path) -> BTreeMap<String, Entry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for chunk in text.split("<game>").skip(1) {
        let body = chunk.split("</game>").next().unwrap_or("");
        let field = |tag: &str| -> Option<String> {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            let s = body.find(&open)? + open.len();
            let e = body[s..].find(&close)? + s;
            let v = unescape(body[s..e].trim());
            (!v.is_empty()).then_some(v)
        };
        let Some(p) = field("path") else { continue };
        let rel = p.replace('\\', "/").trim_start_matches("./").to_owned();
        let key = rel.rsplit_once('.').map_or(rel.clone(), |(s, _)| s.to_owned());
        let file = key.rsplit('/').next().unwrap_or(&key).to_owned();
        let entry = Entry {
            name: field("name"),
            desc: field("desc"),
            genre: field("genre"),
            players: field("players"),
            // ES-DE stores 0.0–1.0; the rest of the app uses RomM's 0–100.
            rating: field("rating").and_then(|r| r.parse::<f64>().ok()).map(|r| r * 100.0),
            // "19940101T000000"
            year: field("releasedate")
                .and_then(|d| d.get(..4).and_then(|y| y.parse().ok())),
        };
        if file != key {
            out.entry(file).or_insert_with(|| entry.clone());
        }
        out.insert(key, entry);
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Files ES-DE keeps beside games that are not games.
fn is_game_file(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') {
        return false;
    }
    // ES-DE's own bookkeeping, which is not filtered by the extension list
    // below: a disabled `noload.txt` is renamed rather than deleted, and
    // `noload.txt.masked-off` has the extension `masked-off`. One turned up in
    // the `mame` directory and the library listed a game called "noload.txt".
    if name.to_ascii_lowercase().starts_with("noload.") {
        return false;
    }
    !matches!(
        p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("xml" | "txt" | "srm" | "state" | "cfg" | "dat" | "db")
    )
}

/// Whether a system directory has been switched off in the library.
///
/// ES-DE's own convention: an empty file called `noload.txt` beside the games.
/// It is how a console you are not playing gets hidden without moving several
/// hundred files, and a second frontend reading the same library has to honour
/// it or the two disagree about what the library contains.
///
/// Exactly that name, which is what makes turning the marker *off* work:
/// renaming it to `noload.txt.masked-off` is how it gets disabled, and that
/// must not go on matching. Two of the directories on Frank's card are in
/// exactly that state.
fn is_switched_off(dir: &Path) -> bool {
    dir.join("noload.txt").is_file()
}

/// The RomM slugs of every system the library has switched off.
///
/// Read from disk on each call rather than remembered, so adding or removing a
/// marker shows up the next time the grid is drawn instead of after a sync. It
/// is one `stat` per system directory, against a list that is a few dozen long.
///
/// Separate from [`scan`] because the two answer different questions. `scan`
/// decides which *games* exist and a switched-off system contributes none; this
/// decides which *platforms* to show, which matters even when the games came
/// from the server rather than the card — otherwise a console hidden on the
/// device reappears in the grid the moment there is a library behind it.
pub fn switched_off_slugs(roms: &Path, map: &CoreMap) -> BTreeSet<String> {
    let sys_to_slug = slug_map(map);
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(roms).into_iter().flatten().flatten() {
        let dir = entry.path();
        if !dir.is_dir() || !is_switched_off(&dir) {
            continue;
        }
        if let Some(slug) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|system| sys_to_slug.get(system))
        {
            out.insert((*slug).to_owned());
        }
    }
    out
}

/// ES-DE system directory name -> RomM slug, most specific source last.
fn slug_map(map: &CoreMap) -> BTreeMap<&str, &str> {
    let mut sys_to_slug: BTreeMap<&str, &str> = BTreeMap::new();
    for (system, def) in &map.systems {
        if let Some(slug) = def.server_platforms.first() {
            sys_to_slug.insert(system.as_str(), slug.as_str());
        }
    }
    // Aliases win: they describe the directory names a real install uses.
    for (system, slug) in SYSTEM_ALIASES {
        sys_to_slug.insert(system, slug);
    }
    // The device's own spellings win over both. Batocera names its directories
    // itself, and the shipped core map was built from an ES-DE *Android*
    // export, so on KNULLI `fbneo` is the whole arcade library under a name
    // nothing else here knows.
    for (system, slug) in crate::platform::current().system_aliases() {
        sys_to_slug.insert(system, slug);
    }
    sys_to_slug
}

/// Files that are never a game, whatever folder they turn up in.
///
/// [`is_game_file`] is a deny list, which is right for a system directory: it
/// holds games and little else. One level down that stops being true.
/// `arcade/fbneo` is full of `.nv` battery files, `famicom/FCEUmm` of
/// `.state.auto`, `easyrpg/Imgs` of screenshots — recursing without this
/// listed every one of them as a game.
const NEVER_GAMES: &[&str] = &[
    "auto", "bak", "bmp", "cfg", "dat", "db", "fs", "ini", "jpeg", "jpg", "ldb", "lmt", "lmu",
    "log", "m3u", "mid", "nv", "ogg", "png", "rtc", "sav", "srm", "state", "txt", "wav", "webp",
    "xml", "xyz",
];

/// Whether a file below the top level counts as a game.
///
/// The system's own extension list wins outright, so `dreamcast/*.dat` is
/// still a game even though `.dat` is junk everywhere else. A spelling ES-DE
/// does not list falls through and is kept — `n3ds/eShop` is full of `.zcci`,
/// and an allow list alone would have thrown the lot away.
///
/// `.m3u` is the reason the list wins rather than merely helping. RomM writes
/// one into every folder it serves, so `snes/Aftermarket/Aftermarket.m3u` sits
/// beside the games naming all thirteen as discs. It is listed for `psx` and
/// `dreamcast`, where a playlist really is how a game starts, and nowhere
/// else.
fn nested_game_file(path: &Path, exts: &[String]) -> bool {
    if !is_game_file(path) {
        return false;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) else {
        return false;
    };
    if exts.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext)) {
        return true;
    }
    !NEVER_GAMES.contains(&ext.as_str())
}

/// `Shenmue (USA) (Disc 2)` -> `Shenmue (USA)`, and `(Disk 2)`, `(CD 2)`,
/// `(Side A)` the same way. `None` when there is no marker at all.
///
/// A file named for nothing but its disc — `disc1.chd` — reduces to the empty
/// string rather than `None`, so a folder of those reads as one game.
///
/// The label has to be short and alphanumeric, or `Tales of the Disc Golfer`
/// would reduce to something that matches nothing.
fn strip_disc_marker(stem: &str) -> Option<String> {
    let lower = stem.to_ascii_lowercase();
    // A folder whose files are named only by their disc: `disc1.chd`,
    // `cd 2.bin`. There is no title to strip, so they all reduce to nothing
    // and land on the same base — which is the answer, since a folder named
    // that way holds one game by construction.
    let bare = lower.replace([' ', '_', '-'], "");
    for kind in ["disc", "disk", "cd", "side"] {
        if let Some(n) = bare.strip_prefix(kind)
            && !n.is_empty()
            && n.len() <= 2
            && n.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Some(String::new());
        }
    }
    for kind in ["disc", "disk", "cd", "side"] {
        let open = format!("({kind} ");
        let mut from = 0;
        while let Some(i) = lower[from..].find(&open) {
            let at = from + i;
            let Some(len) = lower[at..].find(')') else { break };
            let end = at + len + 1;
            let label = stem[at + open.len()..end - 1].trim();
            if !label.is_empty()
                && label.len() <= 2
                && label.chars().all(|c| c.is_ascii_alphanumeric())
            {
                let joined = format!("{}{}", &stem[..at], &stem[end..]);
                return Some(joined.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            from = at + 1;
        }
    }
    None
}

/// What a directory inside a system folder turns out to be.
enum Folder {
    /// The directory itself is the game. EasyRPG works this way.
    Game,
    /// Discs of one game, which RomM also serves as a single folder ROM.
    MultiDisc,
    /// A shelf of unrelated games. ES-DE walks into these, and so do we.
    Shelf,
}

/// Tell the three apart.
///
/// What separates `psx/Final Fantasy VII (USA)/`, whose files are all named
/// after the folder, from `psx/MultiDisk/`, whose members carry disc numbers
/// too but are five different games and named after none of them.
fn classify(dir: &Path, exts: &[String]) -> Folder {
    // `RPG_RT.ldb` is the marker EasyRPG itself looks for. Descending would
    // list the maps and graphics inside and lose the game entirely.
    if dir.join("RPG_RT.ldb").is_file() || dir.join("RPG_RT.exe").is_file() {
        return Folder::Game;
    }
    let own = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut bases = BTreeSet::new();
    let mut named_after_the_folder = true;
    let mut files = 0;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        // A multi-disc game is flat. Anything with folders under it is a shelf.
        if path.is_dir() {
            return Folder::Shelf;
        }
        if !nested_game_file(&path, exts) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        // The playlist is how the folder launches, not one of its discs.
        if name.to_ascii_lowercase().ends_with(".m3u") {
            continue;
        }
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        files += 1;
        named_after_the_folder &= stem.to_ascii_lowercase().starts_with(&own);
        bases.insert(strip_disc_marker(stem).unwrap_or_else(|| stem.to_owned()));
    }
    if files == 0 {
        return Folder::Shelf;
    }
    // RomM names a folder ROM after the game and everything inside after the
    // folder, which is the strongest signal there is and the one that survives
    // an odd member: `Lunar - Silver Star Story Complete (USA)` holds two discs
    // and a bonus `(The Making of)` disc with no number on it at all.
    if named_after_the_folder && !own.is_empty() {
        return Folder::MultiDisc;
    }
    // Failing that, files named for nothing but their disc — `disc1.chd`,
    // `disc2.chd` — which reduce to the same empty base.
    if files >= 2 && bases.len() == 1 { Folder::MultiDisc } else { Folder::Shelf }
}

/// The fixed part of a walk: everything that does not change as it descends.
struct Walk<'a> {
    system: &'a str,
    slug: &'a str,
    meta: &'a BTreeMap<String, Entry>,
    exts: &'a [String],
}

impl Walk<'_> {
    fn game(&self, path: &Path, rel_dir: &str, is_dir: bool) -> Game {
        let fs_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let stem = fs_name.rsplit_once('.').map_or(fs_name.clone(), |(s, _)| s.to_owned());
        // The gamelist is keyed by path relative to the system directory, so a
        // game in a subfolder has to be looked up under that path.
        let key = if rel_dir.is_empty() { stem.clone() } else { format!("{rel_dir}/{stem}") };
        let e = self.meta.get(&key);
        let size = if is_dir {
            dir_size(path)
        } else {
            std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0)
        };
        Game {
            platform_slug: self.slug.to_owned(),
            system: self.system.to_owned(),
            name: e.and_then(|e| e.name.clone()).unwrap_or_else(|| stem.clone()),
            fs_name,
            rel_dir: rel_dir.to_owned(),
            path: path.to_owned(),
            size_bytes: size,
            summary: e.and_then(|e| e.desc.clone()),
            genres: e
                .and_then(|e| e.genre.clone())
                .map(|g| g.split([',', '/']).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default(),
            players: e.and_then(|e| e.players.clone()),
            rating: e.and_then(|e| e.rating),
            release_year: e.and_then(|e| e.year),
        }
    }
}

/// One directory, then the shelves beneath it.
///
/// A shelf that turns out to hold nothing contributes nothing, which is what
/// drops `arcade/fbneo` and `famicom/FCEUmm` without having to name them.
fn walk(w: &Walk, dir: &Path, rel_dir: &str, out: &mut Vec<Game>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(fs_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        // A leading dot means hidden, and the device means it. Batocera files
        // multi-disc games as `.Final Fantasy VII (USA)/` with the `.m3u`
        // beside it, and scanning the folder too listed every one of them
        // twice, once properly and once with a dot in front and no way to start
        // it.
        if fs_name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            match classify(&path, w.exts) {
                Folder::Shelf => {
                    let sub = if rel_dir.is_empty() {
                        fs_name
                    } else {
                        format!("{rel_dir}/{fs_name}")
                    };
                    walk(w, &path, &sub, out);
                }
                // A folder game and a multi-disc folder are both one entry,
                // the way RomM serves them.
                Folder::Game | Folder::MultiDisc => out.push(w.game(&path, rel_dir, true)),
            }
            continue;
        }
        // The top level keeps the permissive rule it has always had; below it
        // the system's extension list decides, because that is where the
        // emulator's own bookkeeping lives.
        let keep = if rel_dir.is_empty() {
            is_game_file(&path)
        } else {
            nested_game_file(&path, w.exts)
        };
        if keep {
            out.push(w.game(&path, rel_dir, false));
        }
    }
}

/// Scan an ES-DE library into games, keyed to RomM platform slugs.
///
/// Systems the core map does not know are skipped rather than guessed at — a
/// wrong slug would put games under a platform whose core cannot run them.
pub fn scan(layout: &Layout, map: &CoreMap) -> Result<(Vec<Game>, Vec<String>)> {
    if !layout.roms.is_dir() {
        bail!("no ROMs directory at {}", layout.roms.display());
    }

    let sys_to_slug = slug_map(map);
    let ignored = crate::platform::current().ignored_systems();

    let mut out = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&layout.roms)
        .with_context(|| format!("reading {}", layout.roms.display()))?
        .flatten()
    {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(system) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // `NOT_SYSTEMS` is what is never a system anywhere; `ignored` is what
        // this device has and does not want. Both are silent — unlike an
        // unknown directory, which is reported so a missing alias can be found.
        if NOT_SYSTEMS.contains(&system) || ignored.contains(&system) || system.starts_with('.')
        {
            continue;
        }
        // Switched off in the library itself. Silent, like `ignored` above — a
        // system hidden on purpose is not a problem to report.
        if is_switched_off(&dir) {
            continue;
        }
        let Some(slug) = sys_to_slug.get(system) else {
            skipped.push(system.to_owned());
            continue;
        };

        let meta = parse_gamelist(&layout.gamelists.join(system).join("gamelist.xml"));
        let exts = map.systems.get(system).map(|s| s.extensions.as_slice()).unwrap_or(&[]);
        let w = Walk { system, slug, meta: &meta, exts };
        walk(&w, &dir, "", &mut out);
    }
    out.sort_by(|a, b| {
        (&a.platform_slug, &a.rel_dir, &a.name).cmp(&(&b.platform_slug, &b.rel_dir, &b.name))
    });
    skipped.sort();
    Ok((out, skipped))
}

fn dir_size(p: &Path) -> i64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten()
        .map(|e| {
            let q = e.path();
            if q.is_dir() { dir_size(&q) } else { std::fs::metadata(&q).map(|m| m.len() as i64).unwrap_or(0) }
        })
        .sum()
}

/// Find one piece of artwork for a game.
///
/// ES-DE names media after the game's file stem, so this needs the stem and the
/// *ES-DE system* name — not the RomM slug, which is why `Game` carries both.
pub fn media_path(layout: &Layout, system: &str, stem: &str, kind: &str) -> Option<PathBuf> {
    const EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "mp4", "webm", "mkv", "pdf"];
    // ES-DE mirrors the ROM folder structure under each media type, so a game
    // in `snes/Aftermarket` has its cover in `covers/Aftermarket`. `stem` may
    // therefore carry a subfolder, and the listing has to be read from that
    // directory rather than the top one.
    let (sub, stem) = stem.rsplit_once('/').unwrap_or(("", stem));
    let mut dir = layout.media.join(system).join(kind);
    if !sub.is_empty() {
        dir = dir.join(sub);
    }
    let names = media_listing(&dir);
    EXTS.iter().find_map(|e| {
        let file = format!("{stem}.{e}");
        names.contains(&file.to_lowercase()).then(|| dir.join(file))
    })
}

type MediaListings = BTreeMap<PathBuf, std::sync::Arc<std::collections::HashSet<String>>>;
static MEDIA_LISTING_CACHE: std::sync::OnceLock<std::sync::Mutex<MediaListings>> =
    std::sync::OnceLock::new();

/// Every file in one media directory, lowercased, read once.
///
/// This used to be eight `is_file` calls per art type per game — around eighty
/// for one row of the info pane, each a stat over FUSE on a memory card.
/// Measured on the Thor: `rom_detail` took 650-750ms, and moving the cursor one
/// game was that long behind the press.
///
/// One `read_dir` per directory instead, kept for the session. The directories
/// are per system and per art type, so there are a few dozen of them and each
/// is read the first time a game needs it.
///
/// Lowercased because the card is case-insensitive and the gamelist's spelling
/// of a name does not always match the file's.
fn media_listing(dir: &Path) -> std::sync::Arc<std::collections::HashSet<String>> {
    use std::collections::HashSet;
    use std::sync::Arc;
    let cache = MEDIA_LISTING_CACHE.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()));

    if let Ok(map) = cache.lock()
        && let Some(hit) = map.get(dir)
    {
        return hit.clone();
    }
    let names: HashSet<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(|n| n.to_lowercase()))
        .collect();
    let names = Arc::new(names);
    if let Ok(mut map) = cache.lock() {
        map.insert(dir.to_path_buf(), names.clone());
    }
    names
}

/// Forget the media listings, so newly scraped artwork is seen.
///
/// Called after anything writes into the media tree. Without it a picture
/// downloaded during the session would not appear until a restart, which is the
/// one way a cache like this becomes a bug rather than a saving.
pub fn forget_media_listings() {
    // Rebuilt lazily on the next lookup; clearing is enough.
    if let Some(cache) = MEDIA_LISTING_CACHE.get()
        && let Ok(mut map) = cache.lock()
    {
        map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-esde-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(super) fn touch(path: &Path, body: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    pub(super) fn map() -> CoreMap {
        serde_json::from_str(
            r#"{
              "default_core_by_server_platform": {"snes": "snes9x", "megadrive": "genesisgx"},
              "systems": {
                "snes":      {"server_platforms": ["snes"],
                              "extensions": [".sfc", ".smc", ".zip"], "emulators": []},
                "megadrive": {"server_platforms": ["megadrive"], "emulators": []}
              }
            }"#,
        )
        .unwrap()
    }

    /// A shelf is walked into; a multi-disc game is not.
    ///
    /// Both are directories inside a system folder and RomM serves both as a
    /// single folder ROM, so the only thing telling them apart is what is
    /// inside: three files that reduce to one name, or thirteen that do not.
    #[test]
    fn a_shelf_is_walked_into_and_a_multi_disc_game_is_not() {
        let root = scratch("shelves");
        let roms = root.join("ROMs");

        touch(&roms.join("snes/Top Level Game.sfc"), b"x");
        // What RomM leaves behind: the games, and a playlist naming all of
        // them as if they were discs of one.
        touch(&roms.join("snes/Aftermarket/Witch n' Wiz.zip"), b"x");
        touch(&roms.join("snes/Aftermarket/Corn Buster.zip"), b"x");
        touch(&roms.join("snes/Aftermarket/Aftermarket.m3u"), b"x");
        // Two levels down, which the card really has under `sfc`.
        touch(&roms.join("snes/AdditionalRoms/Homebrew/Someone's Demo.sfc"), b"x");
        // A real multi-disc game: one name once the disc number is stripped.
        touch(&roms.join("snes/Chrono Trigger (USA)/Chrono Trigger (USA) (Disc 1).sfc"), b"x");
        touch(&roms.join("snes/Chrono Trigger (USA)/Chrono Trigger (USA) (Disc 2).sfc"), b"x");
        // Disc numbers throughout, but five different games — a shelf.
        touch(&roms.join("snes/MultiDisk/Colony Wars (USA) (Disc 1).sfc"), b"x");
        touch(&roms.join("snes/MultiDisk/Heart of Darkness (USA) (Disc 1).sfc"), b"x");
        // The emulator's own bookkeeping, which is not a game anywhere.
        touch(&roms.join("snes/support/dino.nv"), b"x");
        touch(&roms.join("snes/support/B-Wings.state.auto"), b"x");

        let layout = Layout::new(&root, Some(&roms));
        let (games, _) = scan(&layout, &map()).unwrap();
        let mut found: Vec<(String, String)> =
            games.iter().map(|g| (g.rel_dir.clone(), g.name.clone())).collect();
        found.sort();

        assert_eq!(
            found,
            vec![
                ("".to_owned(), "Chrono Trigger (USA)".to_owned()),
                ("".to_owned(), "Top Level Game".to_owned()),
                ("AdditionalRoms/Homebrew".to_owned(), "Someone's Demo".to_owned()),
                ("Aftermarket".to_owned(), "Corn Buster".to_owned()),
                ("Aftermarket".to_owned(), "Witch n' Wiz".to_owned()),
                ("MultiDisk".to_owned(), "Colony Wars (USA) (Disc 1)".to_owned()),
                ("MultiDisk".to_owned(), "Heart of Darkness (USA) (Disc 1)".to_owned()),
            ],
            "the shelves are walked into, the multi-disc game stays one entry, \
             the playlist RomM wrote is not a game, and `support` contributes nothing"
        );
    }

    /// A bonus disc with no number on it does not turn a game into a shelf.
    ///
    /// `Lunar - Silver Star Story Complete (USA)` ships two discs and a
    /// `(The Making of)` disc. Reading disc numbers alone, the odd one out made
    /// the whole folder look like a shelf and the game became four entries.
    #[test]
    fn a_folder_named_after_its_game_stays_one_game() {
        let root = scratch("bonus-disc");
        let roms = root.join("ROMs");
        let game = "Lunar - Silver Star Story Complete (USA)";
        touch(&roms.join(format!("snes/{game}/{game} (Disc 1).sfc")), b"x");
        touch(&roms.join(format!("snes/{game}/{game} (Disc 2).sfc")), b"x");
        touch(&roms.join(format!("snes/{game}/{game} (The Making of).sfc")), b"x");
        touch(&roms.join(format!("snes/{game}/{game}.m3u")), b"x");

        let layout = Layout::new(&root, Some(&roms));
        let (games, _) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].fs_name, game);
    }

    /// The gamelist keys off the path, not the file name.
    ///
    /// Two games called `Foo` — one at the top, one in a folder — used to share
    /// a key, and whichever the parser read second won both names.
    #[test]
    fn a_nested_game_takes_its_name_from_its_own_gamelist_entry() {
        let root = scratch("nested-gamelist");
        let roms = root.join("ROMs");
        touch(&roms.join("snes/Foo.sfc"), b"x");
        touch(&roms.join("snes/Aftermarket/Foo.sfc"), b"x");
        touch(
            &root.join("gamelists/snes/gamelist.xml"),
            br#"<gameList>
                 <game><path>./Foo.sfc</path><name>The Top One</name></game>
                 <game><path>./Aftermarket/Foo.sfc</path><name>The Nested One</name></game>
               </gameList>"#,
        );

        let layout = Layout::new(&root, Some(&roms));
        let (games, _) = scan(&layout, &map()).unwrap();
        let named = |rel: &str| {
            games.iter().find(|g| g.rel_dir == rel).map(|g| g.name.clone()).unwrap()
        };
        assert_eq!(named(""), "The Top One");
        assert_eq!(named("Aftermarket"), "The Nested One");
    }

    /// EasyRPG games are directories, and descending would list their maps and
    /// lose the game.
    #[test]
    fn a_folder_with_an_rpg_maker_marker_is_the_game() {
        let root = scratch("rpgmaker");
        let roms = root.join("ROMs");
        touch(&roms.join("snes/Ib/RPG_RT.ldb"), b"x");
        touch(&roms.join("snes/Ib/Map0001.lmu"), b"x");

        let layout = Layout::new(&root, Some(&roms));
        let (games, _) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Ib");
        assert_eq!(games[0].rel_dir, "");
    }

    #[test]
    fn disc_markers_are_stripped_but_titles_are_not() {
        assert_eq!(strip_disc_marker("Shenmue (USA) (Disc 2)").as_deref(), Some("Shenmue (USA)"));
        assert_eq!(
            strip_disc_marker("Metal Gear Solid (USA) (Disc 1) (Rev 1)").as_deref(),
            Some("Metal Gear Solid (USA) (Rev 1)")
        );
        assert_eq!(strip_disc_marker("Panzer Dragoon Saga (Disk 3)").as_deref(), Some("Panzer Dragoon Saga"));
        assert_eq!(strip_disc_marker("Some Game (Side A)").as_deref(), Some("Some Game"));
        // No marker at all, and a title that merely contains the word.
        assert_eq!(strip_disc_marker("Super Mario World"), None);
        assert_eq!(strip_disc_marker("Tales of the Disc Golfer"), None);
    }

    /// The KNULLI scheme, end to end through a real scan.
    ///
    /// Measured on the Flip on 2026-08-24: `fbneo` holds 2,504 zips and is the
    /// arcade library; `wswan`, `wswanc` and `gamecube` are empty and not
    /// wanted. Without the scheme's alias the whole arcade set scans to
    /// nothing, and `esde::scan` reports that as a skipped directory rather
    /// than as an error — which is exactly why it went unnoticed.
    #[cfg(feature = "knulli")]
    #[test]
    fn on_knulli_fbneo_is_the_arcade_library_and_the_unwanted_are_silent() {
        let root = scratch("knulli-scan");
        let roms = root.join("roms");
        touch(&roms.join("fbneo/sf2.zip"), b"rom");
        touch(&roms.join("wswan/gunpey.ws"), b"rom");
        touch(&roms.join("gamecube/melee.iso"), b"rom");
        touch(&roms.join("nowhere/mystery.bin"), b"rom");

        let layout = Layout::new(&root, Some(&roms));
        let (games, skipped) = scan(&layout, &map()).unwrap();

        assert_eq!(
            games.iter().map(|g| g.platform_slug.as_str()).collect::<Vec<_>>(),
            ["arcade"],
            "fbneo is the arcade library and nothing else should have scanned"
        );
        assert!(
            !skipped.iter().any(|s| s == "wswan" || s == "gamecube"),
            "hidden means silent, not reported: {skipped:?}"
        );
        assert!(
            skipped.iter().any(|s| s == "nowhere"),
            "a genuinely unknown directory must still be reported, or the next \
             missing alias is invisible too: {skipped:?}"
        );
    }

    /// ES-DE keeps its ROMs directory separate and configurable, so it is not
    /// safe to assume `<root>/ROMs`.
    #[test]
    fn an_explicit_roms_directory_overrides_the_default() {
        let default = Layout::new(Path::new("/esde"), None);
        assert_eq!(default.roms, Path::new("/esde/ROMs"));
        assert_eq!(default.gamelists, Path::new("/esde/gamelists"));
        assert_eq!(default.media, Path::new("/esde/downloaded_media"));

        let custom = Layout::new(Path::new("/esde"), Some(Path::new("/Volumes/SD/games")));
        assert_eq!(custom.roms, Path::new("/Volumes/SD/games"));
        assert_eq!(
            custom.gamelists,
            Path::new("/esde/gamelists"),
            "only the ROMs location moves"
        );
    }

    /// The gamelist is the only source of real titles. Rating is rescaled from
    /// ES-DE's 0.0–1.0 to RomM's 0–100, and the year is the first four
    /// characters of a timestamp — both silent if wrong.
    #[test]
    fn a_gamelist_entry_is_parsed_into_metadata() {
        let dir = scratch("gamelist");
        let path = dir.join("gamelist.xml");
        touch(
            &path,
            br#"<?xml version="1.0"?>
            <gameList>
              <game>
                <path>./Chrono Trigger (USA).sfc</path>
                <name>Chrono Trigger</name>
                <desc>A tale of time travel &amp; friendship</desc>
                <genre>Role-Playing</genre>
                <players>1</players>
                <rating>0.95</rating>
                <releasedate>19950811T000000</releasedate>
              </game>
            </gameList>"#,
        );

        let meta = parse_gamelist(&path);
        let e = meta.get("Chrono Trigger (USA)").expect("keyed by file stem");
        assert_eq!(e.name.as_deref(), Some("Chrono Trigger"));
        assert_eq!(
            e.desc.as_deref(),
            Some("A tale of time travel & friendship"),
            "entities must be unescaped"
        );
        assert_eq!(e.rating, Some(95.0), "0.95 becomes 95, not 0.95");
        assert_eq!(e.year, Some(1995));
        assert_eq!(e.players.as_deref(), Some("1"));
    }

    /// `<path>` is relative and prefixed in a couple of ways; all of them have
    /// to reduce to the same key or the metadata never joins to the file.
    #[test]
    fn game_paths_reduce_to_a_file_stem_however_they_are_written() {
        let dir = scratch("gamelist-paths");
        let path = dir.join("gamelist.xml");
        touch(
            &path,
            br#"<gameList>
              <game><path>./Sonic.md</path><name>Dotted</name></game>
              <game><path>Sub/Dir/Streets of Rage.md</path><name>Nested</name></game>
              <game><path>.\Windows\Golden Axe.md</path><name>Backslashes</name></game>
            </gameList>"#,
        );
        let meta = parse_gamelist(&path);
        assert_eq!(meta.get("Sonic").and_then(|e| e.name.clone()).as_deref(), Some("Dotted"));
        assert_eq!(
            meta.get("Streets of Rage").and_then(|e| e.name.clone()).as_deref(),
            Some("Nested")
        );
        assert_eq!(
            meta.get("Golden Axe").and_then(|e| e.name.clone()).as_deref(),
            Some("Backslashes")
        );
    }

    /// A game with almost no metadata must not poison the whole gamelist, and
    /// empty tags count as absent rather than as an empty title.
    #[test]
    fn sparse_and_empty_fields_are_treated_as_missing() {
        let dir = scratch("gamelist-sparse");
        let path = dir.join("gamelist.xml");
        touch(
            &path,
            br#"<gameList>
              <game><path>./A.sfc</path><name></name><rating></rating></game>
              <game><path>./B.sfc</path><name>Has a name</name></game>
            </gameList>"#,
        );
        let meta = parse_gamelist(&path);
        assert!(meta.get("A").unwrap().name.is_none(), "an empty tag is not a title");
        assert!(meta.get("A").unwrap().rating.is_none());
        assert_eq!(meta.get("B").unwrap().name.as_deref(), Some("Has a name"));
    }

    /// A missing gamelist is ordinary — plenty of systems have none — and must
    /// yield no metadata rather than failing the scan.
    #[test]
    fn a_missing_gamelist_is_not_an_error() {
        assert!(parse_gamelist(Path::new("/nonexistent/gamelist.xml")).is_empty());
    }

    /// Saves, configs and dotfiles sit beside games in an ES-DE tree. Indexing
    /// them invents games that cannot launch.
    #[test]
    fn non_game_files_beside_the_roms_are_not_indexed() {
        for name in ["gamelist.xml", "notes.txt", "Zelda.srm", "Zelda.state", "es.cfg", ".DS_Store"] {
            assert!(!is_game_file(Path::new(name)), "{name} is not a game");
        }
        for name in ["Zelda.sfc", "Sonic.md", "Game.chd", "Game.zip"] {
            assert!(is_game_file(Path::new(name)), "{name} is a game");
        }
    }

    /// The alias that matters most: a real install calls it `genesis` where the
    /// map says `megadrive`. Without the alias this drops the single largest
    /// system on the card.
    #[test]
    fn a_real_installs_system_names_are_aliased_to_the_map() {
        let dir = scratch("scan-alias");
        touch(&dir.join("ROMs/genesis/Sonic.md"), b"rom");
        let layout = Layout::new(&dir, None);

        let (games, skipped) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].platform_slug, "megadrive", "genesis is megadrive");
        assert_eq!(games[0].system, "genesis", "but its media still lives under genesis");
        assert!(skipped.is_empty());
    }

    /// A BIOS folder scanned as a system invents phantom games and hands them
    /// to a core that cannot run them.
    #[test]
    fn bios_and_ports_directories_are_not_systems() {
        let dir = scratch("scan-bios");
        touch(&dir.join("ROMs/snes/Zelda.sfc"), b"rom");
        touch(&dir.join("ROMs/0_BIOS/scph1001.bin"), b"bios");
        touch(&dir.join("ROMs/bios/neogeo.zip"), b"bios");
        touch(&dir.join("ROMs/ports/doom.sh"), b"port");
        touch(&dir.join("ROMs/.hidden/x.bin"), b"junk");
        let layout = Layout::new(&dir, None);

        let (games, skipped) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 1, "only the real system");
        assert_eq!(games[0].name, "Zelda");
        assert!(
            !skipped.iter().any(|s| s == "0_BIOS" || s == "bios" || s == "ports"),
            "these are excluded outright, not reported as unmapped systems"
        );
    }

    /// ES-DE hides a system by dropping `noload.txt` in its directory, and
    /// un-hides it by renaming that file rather than deleting it. Both halves
    /// matter: on Frank's card eight systems are hidden this way and two more
    /// carry a `noload.txt.masked-off` that must not still count.
    #[test]
    fn a_system_marked_noload_is_not_scanned() {
        let dir = scratch("scan-noload");
        touch(&dir.join("ROMs/snes/Zelda.sfc"), b"rom");
        touch(&dir.join("ROMs/megadrive/Sonic.md"), b"rom");
        touch(&dir.join("ROMs/megadrive/noload.txt"), b"");
        let layout = Layout::new(&dir, None);

        let (games, skipped) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 1, "the hidden system contributes nothing");
        assert_eq!(games[0].name, "Zelda");
        assert!(!skipped.iter().any(|s| s == "megadrive"), "hidden on purpose, not unmapped");
    }

    /// The platform list asks a different question from the scan: with a server
    /// behind it a switched-off console has rows of its own, so hiding it has to
    /// be decided from the library rather than from what the scan returned.
    #[test]
    fn switched_off_systems_are_reported_as_slugs_for_the_platform_list() {
        let dir = scratch("scan-switched-off");
        touch(&dir.join("ROMs/snes/Zelda.sfc"), b"rom");
        touch(&dir.join("ROMs/megadrive/Sonic.md"), b"rom");
        touch(&dir.join("ROMs/megadrive/noload.txt"), b"");
        touch(&dir.join("ROMs/gamegear/Sonic.gg"), b"rom");
        touch(&dir.join("ROMs/gamegear/noload.txt.masked-off"), b"");

        let off = switched_off_slugs(&dir.join("ROMs"), &map());
        assert!(off.contains("megadrive"), "hidden by the marker");
        assert!(!off.contains("gamegear"), "its marker has been disabled by renaming");
        assert!(!off.contains("snes"), "never had one");
    }

    /// The marker is turned off by renaming, so the system comes back — and
    /// the renamed marker is not itself a game.
    #[test]
    fn a_masked_off_noload_marker_restores_the_system_without_becoming_a_game() {
        let dir = scratch("scan-noload-masked");
        touch(&dir.join("ROMs/megadrive/Sonic.md"), b"rom");
        touch(&dir.join("ROMs/megadrive/noload.txt.masked-off"), b"");
        touch(&dir.join("ROMs/megadrive/systeminfo.txt"), b"");
        let layout = Layout::new(&dir, None);

        let (games, _) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 1, "one game, not three");
        assert_eq!(games[0].name, "Sonic");
    }

    /// An unknown system is reported rather than guessed at: a wrong slug puts
    /// games under a platform whose core cannot run them.
    #[test]
    fn an_unmapped_system_is_skipped_and_named() {
        let dir = scratch("scan-unknown");
        touch(&dir.join("ROMs/snes/Zelda.sfc"), b"rom");
        touch(&dir.join("ROMs/vectrex/Minestorm.bin"), b"rom");
        let layout = Layout::new(&dir, None);

        let (games, skipped) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(skipped, ["vectrex"], "reported so the user can add a mapping");
    }

    /// A directory inside a system is a multi-disc game, which ES-DE treats as
    /// one entry — the same shape as RomM's folder ROMs.
    #[test]
    fn a_multi_disc_directory_counts_as_one_game_and_is_sized_whole() {
        let dir = scratch("scan-folder");
        touch(&dir.join("ROMs/snes/Solo.sfc"), b"12345");
        touch(&dir.join("ROMs/snes/Shenmue (USA)/disc1.chd"), b"aaa");
        touch(&dir.join("ROMs/snes/Shenmue (USA)/disc2.chd"), b"bb");
        let layout = Layout::new(&dir, None);

        let (games, _) = scan(&layout, &map()).unwrap();
        assert_eq!(games.len(), 2, "the folder is one game, not two files");
        let folder = games.iter().find(|g| g.fs_name == "Shenmue (USA)").expect("folder game");
        assert_eq!(folder.size_bytes, 5, "summed across the folder's contents");
        let solo = games.iter().find(|g| g.fs_name == "Solo.sfc").unwrap();
        assert_eq!(solo.size_bytes, 5);
    }

    /// The gamelist supplies the title; without one the file stem stands in, so
    /// a game is never nameless.
    #[test]
    fn scanned_games_take_their_title_from_the_gamelist_or_the_filename() {
        let dir = scratch("scan-names");
        touch(&dir.join("ROMs/snes/ct.sfc"), b"rom");
        touch(&dir.join("ROMs/snes/unnamed.sfc"), b"rom");
        touch(
            &dir.join("gamelists/snes/gamelist.xml"),
            br#"<gameList><game><path>./ct.sfc</path><name>Chrono Trigger</name>
                <genre>RPG, Adventure</genre></game></gameList>"#,
        );
        let layout = Layout::new(&dir, None);

        let (games, _) = scan(&layout, &map()).unwrap();
        let named = games.iter().find(|g| g.fs_name == "ct.sfc").unwrap();
        assert_eq!(named.name, "Chrono Trigger");
        assert_eq!(named.genres, ["RPG", "Adventure"], "split and trimmed");
        let plain = games.iter().find(|g| g.fs_name == "unnamed.sfc").unwrap();
        assert_eq!(plain.name, "unnamed", "falls back to the stem");
    }

    /// A missing ROMs directory is worth saying plainly — it is the single most
    /// likely thing to be misconfigured.
    #[test]
    fn scanning_without_a_roms_directory_says_so() {
        let dir = scratch("scan-noroms");
        let layout = Layout::new(&dir, None);
        let err = scan(&layout, &map()).expect_err("no ROMs dir").to_string();
        assert!(err.contains("ROMs"), "got: {err}");
    }

    /// Media is keyed by ES-DE system name and found by probing extensions,
    /// because ES-DE stores whatever the scraper produced.
    #[test]
    fn media_is_located_by_probing_the_extensions_esde_uses() {
        let dir = scratch("media");
        let layout = Layout::new(&dir, None);
        touch(&dir.join("downloaded_media/snes/covers/Zelda.jpg"), b"img");

        assert_eq!(
            media_path(&layout, "snes", "Zelda", "covers"),
            Some(dir.join("downloaded_media/snes/covers/Zelda.jpg"))
        );
        assert_eq!(media_path(&layout, "snes", "Zelda", "videos"), None);
        assert_eq!(media_path(&layout, "snes", "Missing", "covers"), None);
    }
}

#[cfg(test)]
mod hidden {
    use super::tests::{map, scratch, touch};
    use super::*;

    /// A leading dot means hidden, and the device means it.
    ///
    /// Batocera files multi-disc games as `.Final Fantasy VII (USA)/` with the
    /// `.m3u` beside it, and its own front end skips the folder. Scanning it
    /// anyway listed every one of those games twice — once properly, once with
    /// a dot in front of the name and no way to start it.
    #[test]
    fn hidden_entries_are_not_games() {
        let dir = scratch("hidden");
        let roms = dir.join("roms");
        let psx = roms.join("snes");
        std::fs::create_dir_all(&psx).unwrap();
        touch(&psx.join("Super Mario World (USA).sfc"), b"x");
        touch(&psx.join("Chrono Trigger (USA).sfc"), b"x");
        std::fs::create_dir_all(psx.join(".Chrono Trigger (USA)")).unwrap();
        touch(&psx.join(".Chrono Trigger (USA)/disc1.sfc"), b"x");
        touch(&psx.join(".hidden thing.sfc"), b"x");

        let layout = Layout::new(&dir, Some(&roms));
        let (games, _) = scan(&layout, &map()).unwrap();
        let mut names: Vec<&str> = games.iter().map(|g| g.fs_name.as_str()).collect();
        names.sort();
        assert_eq!(
            names,
            ["Chrono Trigger (USA).sfc", "Super Mario World (USA).sfc"],
            "a hidden entry was scanned"
        );
    }
}

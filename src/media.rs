//! Artwork resolution: local ES-DE media first, server as fallback.
//!
//! Only ~2% of this library has local ES-DE media (it was staged for a 239-game
//! test subset), while the server has covers for most ROMs. Anything fetched is
//! written into the *same* ES-DE-shaped tree, so there is one lookup path and
//! imported and fetched art are interchangeable:
//!
//! ```text
//! <media_root>/<platform>/covers/<rom basename>.<ext>
//! <media_root>/<platform>/screenshots/<rom basename>.<ext>
//! <media_root>/<platform>/videos/<rom basename>.mp4
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::api;

/// Every ES-DE media type, with the extensions each realistically uses.
///
/// The server exposes these at
/// `/assets/romm/resources/esde-media/<platform>/<type>/<rom basename>.<ext>`
/// — symlinks into the ROM library from RomM's own resources tree, which is a
/// mounted volume and already web-served. Far richer than RomM's own metadata,
/// which carries one cover and one screenshot.
pub const ESDE_TYPES: &[(&str, &[&str])] = &[
    ("covers", &["png", "jpg", "webp"]),
    ("backcovers", &["png", "jpg", "webp"]),
    ("3dboxes", &["png", "jpg", "webp"]),
    ("miximages", &["png", "jpg", "webp"]),
    ("screenshots", &["png", "jpg", "webp"]),
    ("titlescreens", &["png", "jpg", "webp"]),
    ("marquees", &["png", "jpg", "webp"]),
    ("fanart", &["png", "jpg", "webp"]),
    ("physicalmedia", &["png", "jpg", "webp"]),
    ("videos", &["mp4", "webm", "mkv"]),
    ("manuals", &["pdf"]),
];

/// Path prefix the media symlinks live under, relative to the server root.
const ESDE_BASE: &str = "/assets/romm/resources/esde-media";

/// What to show for a game, in the order ES-DE's Canvas theme asks for it.
///
/// ES-DE themes name a *list* of media types and take the first that exists;
/// Canvas's default gamelist asks for `miximage, cover`. Screenshot is on the
/// end here because Canvas's larger-font variants fall back to it, and because
/// something drawn from the game itself beats nothing.
///
/// Miximage first is not a stylistic preference, it is the only one of the
/// three that is a consistent shape. On this library every miximage is
/// 1280x960 whatever the console, while the covers run from 0.73 to 1.37 —
/// portrait on the NES, landscape on the SNES, square on the Game Boy Advance —
/// so a grid built on covers is ragged no matter how carefully they were
/// scraped. That raggedness is what the covers looked wrong for.
pub const ART_CHAIN: &[&str] = &[MIXIMAGES, COVERS, SCREENSHOTS];

/// The art types a game list can be set to show, in the order Settings offers
/// them, with the label to show for each.
pub const LIST_ART_CHOICES: &[(&str, &str)] = &[
    (PHYSICALMEDIA, "Cartridge or disc"),
    (MIXIMAGES, "Miximage"),
    (COVERS, "Box art"),
    ("3dboxes", "Box art (3D)"),
    (TITLESCREENS, "Title screen"),
    (SCREENSHOTS, "Screenshot"),
    (MARQUEES, "Marquee"),
];

/// The chain to try when a game list is set to `preferred`.
///
/// A preference is not a promise: no console has every type, and one of them
/// cannot have the default at all. Cartridge art does not exist for arcade
/// machines — they have no cartridge — so an arcade grid asking for one would
/// be empty on every row no matter how completely it had been scraped. The
/// fallbacks are what keep a choice from emptying a console.
pub fn art_chain(preferred: &str) -> Vec<&str> {
    let mut chain = vec![preferred];
    for kind in ART_CHAIN {
        if !chain.contains(kind) {
            chain.push(kind);
        }
    }
    chain
}

/// What a platform's art lookups have already learned.
///
/// Two facts, both cheap to record and expensive to rediscover:
///
/// * which games have no ES-DE art at all, so they are asked about once
///   instead of on every scroll past them
/// * which ES-DE directory this platform's media actually lives in, so the
///   alias that never matches stops being tried first
///
/// Without this, one screen of arcade cards costs about 1,900 requests —
/// nineteen per game, because a miss walks four media types across two
/// directory names across three extensions before giving up — and scrolling
/// back over the same cards costs the same again. Measured on this library:
/// 4.2 seconds for a hundred cards on a quiet LAN, and it is the whole reason
/// the grid looks like the server is not answering.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ArtIndex {
    /// File stems with no ES-DE art of any kind.
    no_art: std::collections::HashSet<String>,
    /// The ES-DE directory that has actually produced art here.
    dir: Option<String>,
}

/// Per-platform indexes, loaded once and shared. Behind a mutex because the
/// grid resolves eight cards at a time.
static INDEX: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, ArtIndex>>,
> = std::sync::OnceLock::new();

fn index_path(media_root: &Path, platform: &str) -> std::path::PathBuf {
    media_root.join(platform).join(".art-index.json")
}

fn with_index<T>(
    media_root: &Path,
    platform: &str,
    f: impl FnOnce(&mut ArtIndex) -> T,
) -> T {
    let cell = INDEX.get_or_init(Default::default);
    let mut map = cell.lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(platform.to_owned()).or_insert_with(|| {
        std::fs::read_to_string(index_path(media_root, platform))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    });
    f(entry)
}

/// Persist what has been learned. Cheap enough to call after a batch of cards.
///
/// Best-effort throughout: this is an optimisation, and a media root that
/// cannot be written to should mean a slower grid, not a broken one.
pub fn save_art_index(media_root: &Path, platform: &str) {
    let Some(cell) = INDEX.get() else { return };
    let map = cell.lock().unwrap_or_else(|e| e.into_inner());
    let Some(entry) = map.get(platform) else { return };
    let path = index_path(media_root, platform);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(entry) {
        let _ = std::fs::write(path, text);
    }
}

/// Forget what a platform has learned, after new art has been fetched for it.
pub fn clear_art_index(media_root: &Path, platform: &str) {
    if let Some(cell) = INDEX.get() {
        cell.lock().unwrap_or_else(|e| e.into_inner()).remove(platform);
    }
    let _ = std::fs::remove_file(index_path(media_root, platform));
}

/// ES-DE system directories to search for a RomM platform's media.
///
/// The two naming schemes agree almost everywhere, and where they do not it is
/// silent: the media is on the server, the app asks for a directory that does
/// not exist, and the console simply has no artwork. Arcade is the case here —
/// ES-DE scrapes MAME romsets under `mame`, RomM calls the platform `arcade` —
/// and it is the largest console in this library.
pub fn esde_dirs(platform: &str) -> Vec<&str> {
    let mut out = vec![platform];
    out.extend(match platform {
        "arcade" => ["mame"].as_slice(),
        "mame" => ["arcade"].as_slice(),
        _ => [].as_slice(),
    });
    out
}

/// As [`esde_dirs`], with the one that has actually worked here tried first.
///
/// Arcade is why: its media lives under `mame`, so trying `arcade` first means
/// every single lookup pays for a directory that has never once answered.
fn esde_dirs_learned(media_root: &Path, platform: &str) -> Vec<String> {
    let mut dirs: Vec<String> = esde_dirs(platform).into_iter().map(str::to_owned).collect();
    if let Some(known) = with_index(media_root, platform, |i| i.dir.clone())
        && let Some(at) = dirs.iter().position(|d| *d == known)
    {
        dirs.swap(0, at);
    }
    dirs
}

/// ES-DE media subdirectory names, in the order the UI prefers them.
pub const MIXIMAGES: &str = "miximages";
pub const PHYSICALMEDIA: &str = "physicalmedia";
pub const COVERS: &str = "covers";
pub const SCREENSHOTS: &str = "screenshots";
pub const VIDEOS: &str = "videos";
pub const TITLESCREENS: &str = "titlescreens";
pub const MARQUEES: &str = "marquees";
/// Thumbnails for the grid. Not an ES-DE directory — ES-DE has no thumb
/// concept — but kept in the same tree so one delete clears everything.
pub const COVERS_THUMB: &str = "covers_thumb";
pub const MANUALS: &str = "manuals";

/// Look for an already-present media file, whatever its extension.
pub fn find_local(media_root: &Path, platform: &str, stem: &str, kind: &str) -> Option<PathBuf> {
    let dir = media_root.join(platform).join(kind);
    let file = dir_index(&dir).get(&stem.to_lowercase())?.clone();
    dir.join(file).canonicalize().ok()
}

type DirIndex = std::sync::Arc<BTreeMap<String, String>>;
/// The index, and the directory's modification time when it was read.
type CachedDir = (Option<std::time::SystemTime>, DirIndex);
static DIR_CACHE: std::sync::OnceLock<std::sync::Mutex<BTreeMap<PathBuf, CachedDir>>> =
    std::sync::OnceLock::new();

/// When the directory last changed, or `None` if it cannot be asked.
fn dir_stamp(dir: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(dir).ok()?.modified().ok()
}

/// How recently a directory has to have changed before its listing is treated
/// as still moving.
///
/// A modification time is only as fine as the filesystem keeps it, and several
/// keep it to the second. Two writes inside one tick leave the stamp identical,
/// so "has this directory changed" answers no when it has — and the remembered
/// listing is served with the new file missing from it. On APFS the timestamps
/// are fine enough to hide this entirely; on the Linux runner it failed the
/// first time it ran.
///
/// It is not only a test's problem. A scrape landing in the same tick as a
/// lookup would leave that picture invisible for the rest of the session, which
/// is exactly the bug the cache was written not to have.
///
/// So a directory touched within this window is read rather than remembered.
/// While art is arriving that means a walk per lookup, which is the cost of
/// being right during the one second it matters; a directory nobody is writing
/// to is cached as before, which is every directory almost all of the time.
const SETTLED_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether `stamp` is old enough for a remembered listing to be trusted.
fn settled(stamp: Option<std::time::SystemTime>) -> bool {
    stamp
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age >= SETTLED_AFTER)
}

/// One media directory, indexed by lowercased file stem, read once.
///
/// The info pane asks about roughly ten kinds of artwork for every game the
/// cursor lands on, and each ask was a full `read_dir` scanned linearly. On the
/// Thor the Mega Drive artwork folders hold about nine hundred files each and
/// sit on a memory card, so that was ten directory walks over FUSE per press:
/// `rom_detail` measured 650-750ms, which is what "the panel lags behind the
/// cursor" was.
///
/// Read once per directory per session instead. There are a few dozen of them —
/// one per system per art type — so the whole library costs a few dozen walks
/// rather than ten per keypress.
///
/// Lowercased on both sides because the card is case-insensitive and the
/// gamelist's spelling of a title does not always match the file's.
pub fn dir_index(dir: &Path) -> DirIndex {
    let cache = DIR_CACHE.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()));
    // One `stat` to decide whether the remembered listing is still true, rather
    // than walking the directory again. Adding or removing a file changes a
    // directory's modification time, so this catches a scrape that lands while
    // the app is running — and it is what stops the cache being a bug.
    let stamp = dir_stamp(dir);
    if settled(stamp)
        && let Ok(map) = cache.lock()
        && let Some((seen, hit)) = map.get(dir)
        && *seen == stamp
    {
        return hit.clone();
    }
    let mut index = BTreeMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let (Some(stem), Some(name)) = (
            path.file_stem().and_then(|s| s.to_str()),
            path.file_name().and_then(|s| s.to_str()),
        ) else {
            continue;
        };
        index.insert(stem.to_lowercase(), name.to_owned());
    }
    let index = std::sync::Arc::new(index);
    // Not remembered while it is still moving: caching a listing read during
    // the window would pin it there once the window passed.
    if settled(stamp)
        && let Ok(mut map) = cache.lock()
    {
        map.insert(dir.to_path_buf(), (stamp, index.clone()));
    }
    index
}

/// Forget the indexes, so artwork written during this session is seen.
///
/// Without it a picture fetched by a scrape would not appear until a restart,
/// which is how a cache like this turns into a bug.
pub fn forget_dir_indexes() {
    if let Some(cache) = DIR_CACHE.get()
        && let Ok(mut map) = cache.lock()
    {
        map.clear();
    }
}

/// Percent-encode a server path, preserving `/` and `?`.
///
/// RomM emits cover URLs like `.../big.png?ts=2026-07-30 00:45:10` — the raw
/// space in the timestamp makes the request line invalid, so it must be
/// encoded before use.
fn encode_path(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => out.push(b as char),
            b'-' | b'_' | b'.' | b'~' | b'/' | b'?' | b'=' | b'&' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn extension_of(server_path: &str) -> &str {
    let no_query = server_path.split('?').next().unwrap_or(server_path);
    match no_query.rsplit_once('.') {
        Some((_, ext)) if ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) => ext,
        _ => "png",
    }
}

/// Fetch one artwork file from the server and store it in the media tree.
pub async fn fetch(
    client: &api::Client,
    server_path: &str,
    media_root: &Path,
    platform: &str,
    stem: &str,
    kind: &str,
) -> Result<PathBuf> {
    let url = format!(
        "{}/{}",
        client.base().trim_end_matches('/'),
        encode_path(server_path.trim_start_matches('/'))
    );

    let resp = client
        .http()
        .get(&url)
        .header("Authorization", client.auth())
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;

    if !resp.status().is_success() {
        bail!("GET {url} -> {}", resp.status());
    }
    let bytes = resp.bytes().await.context("reading artwork body")?;
    if bytes.is_empty() {
        bail!("empty artwork response from {url}");
    }

    let dir = media_root.join(platform).join(kind);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{stem}.{}", extension_of(server_path)));
    std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    path.canonicalize().or(Ok(path))
}

/// Fetch one ES-DE media file from the server, trying the extensions that
/// type actually uses.
///
/// This path needs no authentication (RomM serves `/assets` openly), so it
/// works even before the client has credentials.
pub async fn ensure_esde(
    client: Option<&api::Client>,
    media_root: &Path,
    platform: &str,
    stem: &str,
    kind: &str,
) -> Option<PathBuf> {
    if let Some(local) = find_local(media_root, platform, stem, kind) {
        return Some(local);
    }
    let client = client?;
    let exts = ESDE_TYPES.iter().find(|(k, _)| *k == kind).map(|(_, e)| *e)?;

    for dir in esde_dirs_learned(media_root, platform) {
        for ext in exts {
            let server_path = format!("{ESDE_BASE}/{dir}/{kind}/{stem}.{ext}");
            // Saved under the platform, not the directory it was found in, so
            // every later lookup finds it without knowing about the aliasing.
            match fetch(client, &server_path, media_root, platform, stem, kind).await {
                Ok(p) => {
                    // Remember which name answered, so the alias that never
                    // does stops being tried first.
                    with_index(media_root, platform, |i| i.dir = Some(dir.clone()));
                    return Some(p);
                }
                // A miss here is ordinary: not every game has every media
                // type, and we are probing extensions.
                Err(_) => continue,
            }
        }
    }
    None
}

/// Everything this app scraped itself, as paths relative to the media root.
///
/// The rest of `downloaded_media` is a copy of what is already on the server —
/// re-fetchable, and pointless to send back. These files are the ones that
/// exist nowhere else, so anything that later pushes artwork onto the server's
/// ES-DE tree needs to know which they are, and a folder of mixed origins
/// cannot say.
///
/// A plain list, at the root of the media folder, so it can be handed to rsync
/// or read by eye without this app.
const SCRAPED_MANIFEST: &str = ".scraped.txt";

/// Note a file this app fetched from ScreenScraper.
///
/// Appends rather than rewrites: a run is thousands of files and holding the
/// whole list in memory to rewrite it each time buys nothing. Duplicates are
/// possible if a game is scraped twice and are the reader's problem — a sorted
/// unique is one `sort -u` away, and losing an entry matters more than
/// repeating one.
pub fn record_scraped(media_root: &Path, platform: &str, kind: &str, file_name: &str) {
    use std::io::Write as _;
    let path = media_root.join(SCRAPED_MANIFEST);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{platform}/{kind}/{file_name}");
    }
}

/// Marker recording that the cache has been cleared of RomM-sourced artwork.
const ESDE_ONLY_MARKER: &str = ".art-from-esde-2";

/// Throw away artwork downloaded from RomM before the app moved to ES-DE.
///
/// The cache keeps art by *kind*, not by where it came from, so a cover fetched
/// from RomM last week and one scraped from ScreenScraper sit side by side in
/// `covers/` and cannot be told apart. Leaving them means the art chain finds
/// the RomM copy first for every game already browsed — the mixing this change
/// exists to stop, made invisible by the fact that it only affects games you
/// happened to look at before.
///
/// So those directories go once. `covers` and `screenshots` refill from ES-DE
/// on demand; `covers_thumb` is no longer part of the chain at all and refills
/// never. Nothing here is authored — every file is a copy of something on the
/// server.
///
/// Screenshots matter as much as covers and were missed the first time. They
/// are the last link of the art chain, so a game with a RomM screenshot cached
/// counted as having artwork: it showed a screenshot where the rest of the
/// console showed cartridges, and — worse — the "find missing artwork" pass
/// skipped it, because from the outside it looked like a game that already had
/// some. That is why the marker has a number: a machine that ran the first
/// version has to run this one too.
///
/// Runs once per media root, recorded by a marker file, so it does not delete a
/// freshly rebuilt cache on the next launch. Returns how many files went.
pub fn drop_server_covers(media_root: &Path) -> usize {
    let marker = media_root.join(ESDE_ONLY_MARKER);
    if marker.exists() || !media_root.is_dir() {
        return 0;
    }

    let mut removed = 0;
    let Ok(platforms) = std::fs::read_dir(media_root) else {
        return 0;
    };
    for platform in platforms.flatten() {
        for kind in [COVERS, COVERS_THUMB, SCREENSHOTS] {
            let dir = platform.path().join(kind);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.path().is_file() && std::fs::remove_file(entry.path()).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    let _ = std::fs::write(&marker, b"artwork comes from ES-DE only; see media.rs
");
    removed
}

/// The image to show for a game: the first thing in [`ART_CHAIN`] that exists.
///
/// Only ES-DE's own media. RomM's cover is deliberately not consulted — it is a
/// second scrape from a different source, and a library that shows one game's
/// art from one place and the next game's from another is exactly the
/// inconsistency this replaces.
/// Whatever is already on this machine, without asking the server anything.
///
/// The network half of `ensure_art` is what makes a grid of collection cards
/// take a second or two to fill: the cards are drawn at once and then every
/// one that has no cached art waits on its own request. This answers from the
/// filesystem alone, so a caller can paint what it has immediately and go back
/// for the rest afterwards.
pub fn local_art(media_root: &Path, platform: &str, stem: &str, preferred: &str) -> Option<PathBuf> {
    art_chain(preferred)
        .into_iter()
        .find_map(|kind| find_local(media_root, platform, stem, kind))
}

pub async fn ensure_art(
    client: Option<&api::Client>,
    media_root: &Path,
    platform: &str,
    stem: &str,
    preferred: &str,
) -> Option<PathBuf> {
    // Anything already here is free, and has to be checked before the index is
    // consulted: a game recorded as having none may since have been scraped.
    for kind in art_chain(preferred) {
        if let Some(p) = find_local(media_root, platform, stem, kind) {
            return Some(p);
        }
    }
    // Asked about once, not on every scroll past it.
    if with_index(media_root, platform, |i| i.no_art.contains(stem)) {
        return None;
    }

    for kind in art_chain(preferred) {
        if let Some(p) = ensure_esde(client, media_root, platform, stem, kind).await {
            return Some(p);
        }
    }
    // Only worth recording when there was a server to ask. Offline, everything
    // would look absent and the whole library would be written off.
    if client.is_some() {
        with_index(media_root, platform, |i| i.no_art.insert(stem.to_owned()));
    }
    None
}

/// Whether a gameplay video exists, without downloading one.
///
/// Videos are tens of megabytes and every other kind of media here is tens of
/// kilobytes, so fetching one just to find out whether it exists costs more
/// than everything else a game shows put together — and until now that happened
/// for every game the cursor touched. A HEAD answers the same question for the
/// price of a header.
pub async fn video_exists(
    client: Option<&api::Client>,
    media_root: &Path,
    platform: &str,
    stem: &str,
) -> bool {
    if find_local(media_root, platform, stem, VIDEOS).is_some() {
        return true;
    }
    let Some(client) = client else {
        return false;
    };
    let exts = ESDE_TYPES.iter().find(|(k, _)| *k == VIDEOS).map(|(_, e)| *e);
    for dir in esde_dirs(platform) {
        for ext in exts.unwrap_or(&[]) {
            let url = format!("{}{ESDE_BASE}/{dir}/{VIDEOS}/{stem}.{ext}", client.base());
            let ok = client
                .http()
                .head(&url)
                .header("Authorization", client.auth())
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
            if ok {
                return true;
            }
        }
    }
    false
}

/// Grid thumbnail: the small cover if the server has one, otherwise whatever
/// full-size cover is already local (an imported ES-DE one, typically).
pub async fn ensure_thumb(
    client: Option<&api::Client>,
    media_root: &Path,
    platform: &str,
    stem: &str,
    small: Option<&str>,
    large: Option<&str>,
) -> Option<PathBuf> {
    if let Some(p) = find_local(media_root, platform, stem, COVERS_THUMB) {
        return Some(p);
    }
    // An already-downloaded full cover beats spending a request on a thumb.
    if let Some(p) = find_local(media_root, platform, stem, COVERS) {
        return Some(p);
    }
    if small.is_some()
        && let Some(p) = ensure(client, media_root, platform, stem, COVERS_THUMB, small).await {
            return Some(p);
        }
    ensure(client, media_root, platform, stem, COVERS, large).await
}

/// Resolve a set of screenshots: any already local, plus anything the server
/// has that we do not.
///
/// Extras are stored as `<stem>-2.jpg`, `<stem>-3.jpg` … because the ES-DE
/// convention only names one screenshot per game, and we do not want to
/// collide with an imported one.
pub async fn ensure_set(
    client: Option<&api::Client>,
    media_root: &Path,
    platform: &str,
    stem: &str,
    server_paths: &[String],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for (i, server) in server_paths.iter().enumerate() {
        let name = if i == 0 {
            stem.to_owned()
        } else {
            format!("{stem}-{}", i + 1)
        };
        if let Some(p) = ensure(
            client,
            media_root,
            platform,
            &name,
            SCREENSHOTS,
            Some(server.as_str()),
        )
        .await
        {
            out.push(p);
        }
    }
    // No server list at all (or nothing fetched): fall back to whatever is on
    // disk under the plain stem.
    if out.is_empty()
        && let Some(p) = find_local(media_root, platform, stem, SCREENSHOTS)
    {
        out.push(p);
    }
    out
}

/// Local file if present, else fetch from the server and cache it.
///
/// Returns `None` when there is nothing local *and* the server has no artwork
/// for this ROM — a normal outcome, not an error.
pub async fn ensure(
    client: Option<&api::Client>,
    media_root: &Path,
    platform: &str,
    stem: &str,
    kind: &str,
    server_path: Option<&str>,
) -> Option<PathBuf> {
    if let Some(local) = find_local(media_root, platform, stem, kind) {
        return Some(local);
    }
    let (client, server_path) = (client?, server_path?);
    if server_path.is_empty() {
        return None;
    }
    match fetch(client, server_path, media_root, platform, stem, kind).await {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("artwork fetch failed for {platform}/{stem} ({kind}): {e}");
            None
        }
    }
}


/// Pixel dimensions of a PNG or JPEG, without decoding the image.
///
/// Only the header is read: PNG keeps them in IHDR, JPEG in the SOFn marker.
/// Enough to work out a cover's aspect ratio cheaply.
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 8];
    f.read_exact(&mut head).ok()?;

    if head == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        let mut ihdr = [0u8; 16];
        f.read_exact(&mut ihdr).ok()?;
        let w = u32::from_be_bytes(ihdr[8..12].try_into().ok()?);
        let h = u32::from_be_bytes(ihdr[12..16].try_into().ok()?);
        return (w > 0 && h > 0).then_some((w, h));
    }

    if head[0] == 0xFF && head[1] == 0xD8 {
        f.seek(SeekFrom::Start(2)).ok()?;
        let mut b = [0u8; 1];
        loop {
            // Scan to the next marker.
            while f.read_exact(&mut b).is_ok() && b[0] != 0xFF {}
            let mut marker = 0xFFu8;
            while marker == 0xFF {
                f.read_exact(&mut b).ok()?;
                marker = b[0];
            }
            // SOF0..SOF15, skipping the non-frame markers in that range.
            if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                let mut sof = [0u8; 7];
                f.read_exact(&mut sof).ok()?;
                let h = u16::from_be_bytes([sof[3], sof[4]]) as u32;
                let w = u16::from_be_bytes([sof[5], sof[6]]) as u32;
                return (w > 0 && h > 0).then_some((w, h));
            }
            let mut len = [0u8; 2];
            f.read_exact(&mut len).ok()?;
            let skip = u16::from_be_bytes(len).saturating_sub(2) as i64;
            f.seek(SeekFrom::Current(skip)).ok()?;
        }
    }
    None
}

/// Typical cover aspect (width / height) for a platform, from the covers we
/// already hold.
///
/// Box art varies enormously by system — measured here, PSP UMD cases are 0.58
/// and SNES boxes 1.37 — so a single grid ratio crops most of them. Sampling
/// real files keeps this correct without a hardcoded table to maintain.
///
/// Returns `None` until enough covers are cached to be confident.
pub fn cover_aspect(media_root: &Path, platform: &str) -> Option<f32> {
    let mut ratios: Vec<f32> = Vec::new();
    // The same chain the grid draws from, in the same order. Measuring
    // `covers` while the grid shows miximages would size every card to the
    // wrong shape, and it would be a stale measurement of art nothing renders.
    for kind in ART_CHAIN.iter().copied() {
        let dir = media_root.join(platform).join(kind);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten().take(40) {
            if let Some((w, h)) = image_size(&entry.path())
                && h > 0
            {
                ratios.push(w as f32 / h as f32);
            }
        }
        if ratios.len() >= 8 {
            break;
        }
    }
    if ratios.len() < 3 {
        return None;
    }
    // Median: a few odd covers (a wide promo shot among box art) should not
    // drag the whole grid.
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(ratios[ratios.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-media-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The artwork index is read once and then trusted — but only until the
    /// directory changes.
    ///
    /// It exists because the info pane asks about ten kinds of artwork per game
    /// and each ask was a full directory walk: on the Thor, whose media sits on
    /// a memory card in folders of ~900 files, `rom_detail` took 650-750ms and
    /// the pane visibly lagged the cursor. A cache fixes that and introduces the
    /// opposite risk, which is what this pins down: a picture that arrives
    /// during the session has to be found without a restart.
    #[test]
    fn the_artwork_index_is_reused_until_the_directory_changes() {
        let dir = scratch("dir-index");
        let art = dir.join("snes").join("covers");
        std::fs::create_dir_all(&art).unwrap();
        std::fs::write(art.join("Zelda.png"), b"x").unwrap();

        assert_eq!(
            find_local(&dir, "snes", "Zelda", "covers"),
            Some(art.join("Zelda.png").canonicalize().unwrap()),
            "the file that is there is found"
        );
        assert_eq!(find_local(&dir, "snes", "Mario", "covers"), None);

        // Scraped mid-session. The directory's modification time moves, so the
        // remembered listing is known to be stale without walking it again.
        std::fs::write(art.join("Mario.png"), b"x").unwrap();
        assert_eq!(
            find_local(&dir, "snes", "Mario", "covers"),
            Some(art.join("Mario.png").canonicalize().unwrap()),
            "a picture that arrived after the first look is still found"
        );
    }

    /// The card is case-insensitive and a gamelist's spelling of a title does
    /// not always match the file's, so the index is keyed on a lowercased stem.
    #[test]
    fn artwork_is_found_whatever_the_case_of_the_name() {
        let dir = scratch("dir-index-case");
        let art = dir.join("snes").join("covers");
        std::fs::create_dir_all(&art).unwrap();
        std::fs::write(art.join("Chrono Trigger.png"), b"x").unwrap();

        assert!(find_local(&dir, "snes", "chrono trigger", "covers").is_some());
        assert!(find_local(&dir, "snes", "CHRONO TRIGGER", "covers").is_some());
    }

    /// A directory that is not there is an empty index, not a panic — most
    /// systems have no `manuals` folder at all.
    #[test]
    fn a_missing_artwork_directory_is_simply_empty() {
        let dir = scratch("dir-index-absent");
        assert_eq!(find_local(&dir, "snes", "Zelda", "manuals"), None);
    }

    /// A PNG header only: signature, then the IHDR chunk whose first two fields
    /// are the dimensions. Enough for `image_size`, which never decodes pixels.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v
    }

    /// A JPEG reduced to SOI followed by an SOF0 frame header, which is where
    /// the dimensions live — height first, then width.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v
    }

    /// RomM emits cover URLs carrying a timestamp with a raw space in it, which
    /// makes the HTTP request line invalid. `/` and `?` have to survive, or the
    /// path stops addressing anything.
    #[test]
    fn a_server_path_is_encoded_without_destroying_its_structure() {
        let raw = "/assets/romm/resources/roms/2/42/cover/big.png?ts=2026-07-30 00:45:10";
        let got = encode_path(raw);
        assert!(!got.contains(' '), "the raw space must be gone: {got}");
        assert!(got.contains("%20"));
        assert!(got.starts_with("/assets/romm/"), "slashes are preserved: {got}");
        assert!(got.contains("?ts="), "the query still addresses the same asset: {got}");
        assert!(got.contains("00%3A45%3A10") || got.contains("00:45:10"));
    }

    /// The stored file's extension comes from the server path, and the query
    /// string is not part of it — `big.png?ts=…` must not become `png?ts=…`.
    #[test]
    fn the_extension_ignores_the_query_string() {
        assert_eq!(extension_of("/x/big.png?ts=2026-07-30 00:45:10"), "png");
        assert_eq!(extension_of("/x/cover.jpg"), "jpg");
        assert_eq!(extension_of("/x/clip.webm"), "webm");
    }

    /// Anything that does not look like an extension falls back to png rather
    /// than producing a file named after half a URL.
    #[test]
    fn an_unusable_extension_falls_back_to_png() {
        assert_eq!(extension_of("/x/no-extension-here"), "png");
        // Too long to be an extension — this is a path segment with a dot.
        assert_eq!(extension_of("/x/file.somethinglong"), "png");
        // Not alphanumeric.
        assert_eq!(extension_of("/x/file.p n g"), "png");
    }

    /// Imported ES-DE art and art fetched from the server share one tree, so
    /// the lookup must match on stem regardless of which extension landed.
    #[test]
    fn local_media_is_found_by_stem_whatever_the_extension() {
        let root = scratch("find-local");
        let dir = root.join("snes").join(COVERS);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Zelda.webp"), b"img").unwrap();

        assert!(find_local(&root, "snes", "Zelda", COVERS).is_some());
        // A different game, a different platform and a different media type
        // must all miss rather than returning the wrong file.
        assert!(find_local(&root, "snes", "Zeld", COVERS).is_none());
        assert!(find_local(&root, "nes", "Zelda", COVERS).is_none());
        assert!(find_local(&root, "snes", "Zelda", VIDEOS).is_none());
    }

    /// Dimensions are read from the header alone. Getting the PNG field order
    /// wrong transposes every cover; in JPEG the two are stored height-first,
    /// which is the easy mistake.
    #[test]
    fn image_dimensions_are_read_from_the_header_of_both_formats() {
        let dir = scratch("image-size");
        let p = dir.join("a.png");
        std::fs::write(&p, png(600, 800)).unwrap();
        assert_eq!(image_size(&p), Some((600, 800)));

        let j = dir.join("b.jpg");
        std::fs::write(&j, jpeg(320, 240)).unwrap();
        assert_eq!(image_size(&j), Some((320, 240)), "JPEG stores height before width");
    }

    /// Anything that is not a readable image must decline rather than return a
    /// nonsense ratio that reshapes the whole grid.
    #[test]
    fn a_file_that_is_not_an_image_has_no_size() {
        let dir = scratch("image-size-bad");
        let txt = dir.join("notes.txt");
        std::fs::write(&txt, b"not an image at all").unwrap();
        assert_eq!(image_size(&txt), None);
        assert_eq!(image_size(&dir.join("missing.png")), None);

        // A PNG claiming zero height would divide by zero downstream.
        let zero = dir.join("zero.png");
        std::fs::write(&zero, png(100, 0)).unwrap();
        assert_eq!(image_size(&zero), None);
    }

    /// Box art varies enormously by system, so the grid measures rather than
    /// assumes. The median is used so one odd wide promo shot cannot reshape
    /// every card on the page.
    #[test]
    fn the_cover_aspect_is_the_median_and_ignores_an_outlier() {
        let root = scratch("aspect");
        let dir = root.join("snes").join(COVERS);
        std::fs::create_dir_all(&dir).unwrap();
        // Four portrait covers at 0.5, and one very wide outlier.
        for i in 0..4 {
            std::fs::write(dir.join(format!("box{i}.png")), png(500, 1000)).unwrap();
        }
        std::fs::write(dir.join("promo.png"), png(4000, 1000)).unwrap();

        let aspect = cover_aspect(&root, "snes").expect("five covers is enough");
        assert!(
            (aspect - 0.5).abs() < 0.01,
            "the median must ignore the 4.0 outlier, got {aspect}"
        );
    }

    /// Too few covers means no confident answer, and the grid keeps its default
    /// rather than shaping itself around one or two files.
    #[test]
    fn too_few_covers_yields_no_aspect_at_all() {
        let root = scratch("aspect-few");
        let dir = root.join("snes").join(COVERS);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(cover_aspect(&root, "snes"), None, "nothing cached yet");

        std::fs::write(dir.join("a.png"), png(500, 1000)).unwrap();
        std::fs::write(dir.join("b.png"), png(500, 1000)).unwrap();
        assert_eq!(cover_aspect(&root, "snes"), None, "two is not enough to be confident");

        std::fs::write(dir.join("c.png"), png(500, 1000)).unwrap();
        assert!(cover_aspect(&root, "snes").is_some(), "three is");
    }

    /// Every declared media type needs at least one extension to probe, or
    /// `ensure_esde` silently never fetches it.
    #[test]
    fn every_media_type_has_extensions_to_try() {
        for (kind, exts) in ESDE_TYPES {
            assert!(!exts.is_empty(), "{kind} has no extensions and could never be fetched");
        }
        // The named constants must all be types that actually exist in the
        // table, or a lookup by that name finds nothing.
        for kind in [COVERS, SCREENSHOTS, VIDEOS, MANUALS] {
            assert!(
                ESDE_TYPES.iter().any(|(k, _)| *k == kind),
                "{kind} is referenced by name but not declared"
            );
        }
    }

    /// With no client there is nothing to fetch from, so these must resolve
    /// from disk alone rather than erroring — this is the offline path.
    #[tokio::test]
    async fn artwork_resolves_offline_from_local_files_only() {
        let root = scratch("offline");
        let dir = root.join("snes").join(COVERS);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Zelda.png"), b"img").unwrap();

        assert!(ensure(None, &root, "snes", "Zelda", COVERS, None).await.is_some());
        assert!(ensure(None, &root, "snes", "Missing", COVERS, None).await.is_none());
        // A local full cover is preferred over spending a request on a thumb.
        assert!(ensure_thumb(None, &root, "snes", "Zelda", None, None).await.is_some());
        assert!(ensure_esde(None, &root, "snes", "Zelda", COVERS).await.is_some());
    }

    /// Extra screenshots are stored as `<stem>-2`, `<stem>-3` so a fetched set
    /// cannot overwrite an imported ES-DE screenshot sharing the plain stem.
    #[tokio::test]
    async fn a_screenshot_set_falls_back_to_the_plain_stem_when_offline() {
        let root = scratch("shot-set");
        let dir = root.join("snes").join(SCREENSHOTS);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Zelda.png"), b"img").unwrap();

        let got = ensure_set(None, &root, "snes", "Zelda", &[]).await;
        assert_eq!(got.len(), 1, "the local screenshot is still found with no server list");
    }

    /// Arcade is the whole reason this exists: ES-DE scrapes MAME romsets
    /// under `mame` while RomM calls the platform `arcade`, and the mismatch
    /// is silent — the art is on the server and the console just looks
    /// unscraped.
    #[test]
    fn arcade_media_is_looked_for_under_mame_as_well() {
        assert!(esde_dirs("arcade").contains(&"mame"), "{:?}", esde_dirs("arcade"));
        assert!(esde_dirs("mame").contains(&"arcade"), "{:?}", esde_dirs("mame"));
        // The platform's own name always comes first, so an exact match is
        // never passed over in favour of an alias.
        assert_eq!(esde_dirs("arcade")[0], "arcade");
        assert_eq!(esde_dirs("snes"), vec!["snes"]);
    }

    /// Miximage first is the point. It is the only one of the three that is a
    /// consistent shape across consoles, so putting covers ahead of it would
    /// reintroduce the ragged grid this replaced.
    #[test]
    fn the_art_chain_prefers_the_one_consistent_shape() {
        assert_eq!(ART_CHAIN[0], MIXIMAGES);
        assert!(ART_CHAIN.contains(&COVERS));
        assert!(!ART_CHAIN.contains(&COVERS_THUMB), "thumbs are RomM-sourced");
    }

    #[test]
    fn clearing_server_artwork_leaves_everything_else_alone() {
        let root = std::env::temp_dir().join("moose-rack-media-purge");
        let _ = std::fs::remove_dir_all(&root);
        for (kind, name) in [
            (COVERS, "a.png"),
            (COVERS_THUMB, "a.png"),
            (MIXIMAGES, "a.png"),
            (SCREENSHOTS, "a.png"),
            ("videos", "a.mp4"),
        ] {
            let dir = root.join("snes").join(kind);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(name), b"x").unwrap();
        }

        assert_eq!(drop_server_covers(&root), 3, "covers, thumbs and screenshots go");
        assert!(!root.join("snes").join(COVERS).join("a.png").exists());
        assert!(!root.join("snes").join(COVERS_THUMB).join("a.png").exists());
        // Screenshots are the last link of the art chain, so a stale RomM one
        // counted as artwork and hid the game from the fill-in pass.
        assert!(!root.join("snes").join(SCREENSHOTS).join("a.png").exists());
        // Everything else is ES-DE's and has to survive; miximages in
        // particular is what the grid now draws from.
        assert!(root.join("snes").join(MIXIMAGES).join("a.png").exists());
        assert!(root.join("snes").join("videos").join("a.mp4").exists());

        // Second run must do nothing: a rebuilt cache is the normal state on
        // every launch after the first, and clearing it each time would mean
        // re-downloading the whole library's artwork daily.
        std::fs::write(root.join("snes").join(COVERS).join("b.png"), b"x").unwrap();
        assert_eq!(drop_server_covers(&root), 0);
        assert!(root.join("snes").join(COVERS).join("b.png").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A preference cannot be a promise. Arcade machines have no cartridge, so
    /// an arcade grid asking for one would be empty on every row however
    /// completely it had been scraped.
    #[test]
    fn a_choice_that_a_console_cannot_satisfy_still_falls_back() {
        let chain = art_chain(PHYSICALMEDIA);
        assert_eq!(chain[0], PHYSICALMEDIA, "the choice is honoured first");
        assert!(chain.contains(&MIXIMAGES), "and something always follows it");
    }

    /// Choosing a type that is already a fallback must not list it twice, or
    /// every miss costs two round trips to the same missing file.
    #[test]
    fn choosing_a_fallback_type_does_not_duplicate_it() {
        let chain = art_chain(MIXIMAGES);
        assert_eq!(chain[0], MIXIMAGES);
        assert_eq!(chain.iter().filter(|k| **k == MIXIMAGES).count(), 1, "{chain:?}");
    }

    /// Settings offers these by name; a label with no directory behind it is a
    /// choice that silently shows nothing.
    #[test]
    fn every_offered_choice_is_a_real_media_directory() {
        for (kind, label) in LIST_ART_CHOICES {
            assert!(
                ESDE_TYPES.iter().any(|(k, _)| k == kind),
                "{label} maps to {kind}, which ES-DE does not have"
            );
        }
        assert_eq!(LIST_ART_CHOICES[0].0, PHYSICALMEDIA, "cartridge art is the default");
    }

    /// The symptom this exists for: one screen of arcade cards costs about
    /// nineteen requests per game, and scrolling back over the same cards used
    /// to cost the same again.
    #[tokio::test]
    async fn a_game_with_no_art_is_only_asked_about_once() {
        let root = std::env::temp_dir().join("moose-rack-art-index");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("snes")).unwrap();

        // No client: nothing can be asked, and nothing may be recorded either.
        assert!(ensure_art(None, &root, "snes", "Nope", MIXIMAGES).await.is_none());
        assert!(
            !with_index(&root, "snes", |i| i.no_art.contains("Nope")),
            "offline, every game looks absent — writing that down would blank the library"
        );

        // Recorded by hand, since a real miss needs a server to have answered.
        with_index(&root, "snes", |i| i.no_art.insert("Nope".to_owned()));
        save_art_index(&root, "snes");
        assert!(root.join("snes").join(".art-index.json").is_file());

        // Art that turns up later has to win over the record of its absence,
        // or a scrape would fill in files the grid then refuses to look at.
        std::fs::create_dir_all(root.join("snes").join(MIXIMAGES)).unwrap();
        std::fs::write(root.join("snes").join(MIXIMAGES).join("Nope.png"), b"x").unwrap();
        assert!(ensure_art(None, &root, "snes", "Nope", MIXIMAGES).await.is_some());

        clear_art_index(&root, "snes");
        assert!(!root.join("snes").join(".art-index.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Arcade media lives under `mame`, so trying `arcade` first means every
    /// lookup pays for a directory that has never once answered.
    #[test]
    fn the_directory_that_answers_gets_tried_first() {
        let root = std::env::temp_dir().join("moose-rack-art-dirhint");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        assert_eq!(esde_dirs_learned(&root, "arcade")[0], "arcade", "the platform's own name by default");
        with_index(&root, "arcade", |i| i.dir = Some("mame".to_owned()));
        assert_eq!(esde_dirs_learned(&root, "arcade")[0], "mame");
        // Both are still tried; the hint reorders, it does not exclude.
        assert_eq!(esde_dirs_learned(&root, "arcade").len(), 2);

        clear_art_index(&root, "arcade");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The rest of the media folder is a copy of what the server already has,
    /// so a later push needs to know which files exist nowhere else. A folder
    /// of mixed origins cannot say, and getting this wrong means either
    /// uploading the server's own art back to it or losing the scraped art.
    #[test]
    fn the_manifest_records_a_path_the_media_root_can_be_walked_with() {
        let root = std::env::temp_dir().join("moose-rack-scraped-manifest");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        record_scraped(&root, "megadrive", COVERS, "Road Rash (USA, Europe).png");
        record_scraped(&root, "arcade", PHYSICALMEDIA, "colony7.png");

        let text = std::fs::read_to_string(root.join(SCRAPED_MANIFEST)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "appended, not overwritten: {lines:?}");
        assert_eq!(lines[0], "megadrive/covers/Road Rash (USA, Europe).png");
        // Relative to the media root, so it joins onto either end of a copy.
        for line in &lines {
            assert!(!line.starts_with('/'), "{line} is absolute");
            assert_eq!(line.split('/').count(), 3, "{line} is not platform/kind/file");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

//! Local metadata cache.
//!
//! A full cold pull is only ~8 seconds (PLAN.md §3), so this exists for offline
//! browsing and instant navigation rather than to work around slowness. After
//! the first sync it goes incremental via `updated_after`.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::api;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS platforms (
    id            INTEGER PRIMARY KEY,
    fs_slug       TEXT NOT NULL UNIQUE,
    display_name  TEXT,
    rom_count     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS roms (
    id             INTEGER PRIMARY KEY,
    platform_slug  TEXT NOT NULL,
    name           TEXT,
    fs_name        TEXT NOT NULL,
    fs_size_bytes  INTEGER,
    md5_hash       TEXT,
    sha1_hash      TEXT,
    crc_hash       TEXT,
    updated_at     TEXT,
    cover_path     TEXT,
    screenshot_path TEXT,
    screenshots_json TEXT,
    cover_small_path TEXT,
    summary        TEXT,
    meta_json      TEXT,
    alt_names_json TEXT,
    regions_json   TEXT,
    manual_path    TEXT,
    youtube_id     TEXT,
    multi_file     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS roms_platform ON roms(platform_slug);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- Collections mirror the server rather than being a local invention: this is a
-- RomM client, so whatever RomM groups games into is what we show.
CREATE TABLE IF NOT EXISTS collections (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    grp         TEXT NOT NULL,
    description TEXT,
    rom_count   INTEGER NOT NULL DEFAULT 0,
    is_favorite INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS collection_roms (
    collection_id TEXT NOT NULL,
    rom_id        INTEGER NOT NULL,
    PRIMARY KEY (collection_id, rom_id)
);
CREATE INDEX IF NOT EXISTS collection_roms_rom ON collection_roms(rom_id);
-- Every session, one row.
--
-- Kept here rather than on the game because the interesting questions are about
-- the shape of the sessions, not their sum: a game opened eleven times for four
-- minutes each is a different thing from one played twice for an afternoon, and
-- a single "hours played" column cannot tell them apart. The server has no
-- equivalent, so this is the only record there is.
CREATE TABLE IF NOT EXISTS plays (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    rom_id     INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    seconds    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS plays_rom ON plays(rom_id);
-- Play history recorded against ids from before `gameid`, waiting for the scan
-- or sync that says which game each one was. See `adopt_stable_ids`.
CREATE TABLE IF NOT EXISTS id_migration (
    old_id        INTEGER PRIMARY KEY,
    platform_slug TEXT,
    fs_name       TEXT,
    esde_system   TEXT,
    rel_dir       TEXT,
    -- Set when one old id turned up naming two different games: an older build
    -- reused the number. Nothing is attributed to it then.
    ambiguous     INTEGER NOT NULL DEFAULT 0
);
-- Every old id placed so far, with the game it named. Kept, not consumed, so
-- anything else on this machine still keyed by old ids -- save backups, the
-- state ledger, a browser's remembered selection -- can be moved later.
CREATE TABLE IF NOT EXISTS id_moves (
    old_id        INTEGER PRIMARY KEY,
    new_id        INTEGER NOT NULL,
    platform_slug TEXT NOT NULL,
    fs_name       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS collections_grp ON collections(grp);
"#;

/// `(old id, platform slug, file name, system folder, relative dir)`, one row of
/// `id_migration`.
type Pending = (i64, String, String, Option<String>, Option<String>);

pub struct Cache {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct CollectionRow {
    pub id: String,
    pub name: String,
    /// `user`, `smart`, or a virtual kind such as `genre` / `franchise`.
    pub group: String,
    pub description: Option<String>,
    pub rom_count: i64,
    pub is_favorite: bool,
    /// A few member ROM ids, so the card can show real cover art through the
    /// same local cache the game grids use.
    pub sample_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct PlatformRow {
    pub fs_slug: String,
    pub display_name: String,
    pub rom_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct RomRow {
    /// Used for `/api/roms/{id}/content/{fs_name}`.
    pub id: i64,
    pub platform_slug: String,
    pub name: String,
    pub fs_name: String,
    pub fs_size_bytes: i64,
    pub md5_hash: Option<String>,
    pub sha1_hash: Option<String>,
    /// Server-relative artwork paths, if the server has any.
    pub cover_path: Option<String>,
    pub screenshot_path: Option<String>,
    /// Every screenshot the server has, JSON-encoded. Games range from 0 to 12.
    pub screenshots_json: Option<String>,
    pub cover_small_path: Option<String>,
    pub summary: Option<String>,
    /// RomM's merged metadata: genres, companies, player count, rating…
    pub meta_json: Option<String>,
    pub alt_names_json: Option<String>,
    pub regions_json: Option<String>,
    pub manual_path: Option<String>,
    pub youtube_id: Option<String>,
    pub multi_file: bool,
    /// When this game was last played, as an ISO timestamp. Written locally
    /// after every session and also filled in by the server, so it sorts and
    /// compares as text either way.
    pub last_played: Option<String>,
    /// ES-DE system directory this came from, when the library was scanned
    /// from a local ES-DE tree. Artwork there is keyed by ES-DE system name,
    /// not by RomM slug, so the two cannot be used interchangeably.
    pub esde_system: Option<String>,
    /// Subfolder inside the system directory, `""` at the top level.
    ///
    /// Server rows are always empty: RomM has no folders, it has one ROM per
    /// folder. Only a local ES-DE scan fills this in.
    pub rel_dir: String,
    /// Absolute path for a locally scanned game.
    pub local_path: Option<String>,
}

/// Columns every `RomRow` query selects, in order.
/// Every game in a starred collection.
///
/// Two ways in, because there are two ways to star something. RomM flags a
/// collection it considers a favorite, and a person marks one by putting a
/// star in the name — which is what happened on this library. Reading both
/// means the app agrees with whichever the user did.
const FAVORITE_ROMS: &str = "SELECT cr.rom_id FROM collection_roms cr \
                              JOIN collections c ON c.id = cr.collection_id \
                              WHERE c.is_favorite = 1 OR c.name LIKE '★%'";

/// What a per-system starred collection is called on this library.
///
/// The nine "★ Best of …" lists were made by hand on the server and are
/// already mirrored onto the handheld game-for-game, so a star added here has
/// to land in the same place rather than start a tenth convention beside them.
/// The suffix is the RomM platform slug — `megadrive`, `neogeoaes` — because
/// that is what the existing names use.
pub fn star_name(platform: &str) -> String {
    format!("★ Best of {platform}")
}

const ROM_COLUMNS: &str = "id, platform_slug, COALESCE(NULLIF(name, ''), fs_name), \
                           fs_name, COALESCE(fs_size_bytes, 0), md5_hash, sha1_hash, \
                           cover_path, screenshot_path, screenshots_json, \
                           cover_small_path, summary, meta_json, alt_names_json, \
                           regions_json, manual_path, youtube_id, \
                           COALESCE(multi_file, 0), esde_system, local_path, \
                           last_played, COALESCE(rel_dir, '')";

/// Hide a server row that the local scan has walked into.
///
/// RomM has no folders. It indexed `snes/Aftermarket` as one ROM with thirteen
/// files, and the scan now lists those thirteen games and draws a folder — so
/// without this the shelf appears twice, once as a folder you can open and
/// once as a single unplayable game beside it.
///
/// Only ever hides the container. The proper cure is rescanning on the RomM
/// side so the server indexes the games individually.
const NOT_A_WALKED_SHELF: &str = "NOT (roms.from_server = 1 AND roms.from_scan = 0 AND COALESCE(roms.multi_file, 0)      AND EXISTS (SELECT 1 FROM roms AS l WHERE l.from_scan = 1                    AND l.platform_slug = roms.platform_slug                    AND (l.rel_dir = roms.fs_name OR l.rel_dir LIKE roms.fs_name || '/%')))";

/// Whether a row is something to put on screen.
///
/// A leading dot means hidden, and it means it on the card as well as in the
/// app. Batocera files multi-disc games as `.Final Fantasy VII (USA)/` with the
/// `.m3u` beside it, and its own front end skips the folder — so the same game
/// appeared twice, once properly and once with a dot in front of the name and
/// no way to start it.
///
/// The scan no longer picks them up, but a cache filled before that fix still
/// holds them, and a cache synced from a server can hold anything. Filtered
/// here, where every list this app draws passes through, so both front ends get
/// it from one rule rather than two.
pub fn shown(row: &RomRow) -> bool {
    !row.fs_name.starts_with('.')
}

fn rom_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RomRow> {
    Ok(RomRow {
        id: r.get(0)?,
        platform_slug: r.get(1)?,
        name: r.get(2)?,
        fs_name: r.get(3)?,
        fs_size_bytes: r.get(4)?,
        md5_hash: r.get(5)?,
        sha1_hash: r.get(6)?,
        cover_path: r.get(7)?,
        screenshot_path: r.get(8)?,
        screenshots_json: r.get(9)?,
        cover_small_path: r.get(10)?,
        summary: r.get(11)?,
        meta_json: r.get(12)?,
        alt_names_json: r.get(13)?,
        regions_json: r.get(14)?,
        manual_path: r.get(15)?,
        youtube_id: r.get(16)?,
        // Read tolerantly: the migration adds columns as TEXT, so an older
        // cache stores this as "0"/"1" while a freshly created one stores an
        // integer. Both must work without forcing a rebuild.
        multi_file: match r.get_ref(17)? {
            rusqlite::types::ValueRef::Integer(i) => i != 0,
            rusqlite::types::ValueRef::Text(b) => {
                !matches!(std::str::from_utf8(b).unwrap_or("0"), "0" | "" | "false")
            }
            _ => false,
        },
        esde_system: r.get(18)?,
        local_path: r.get(19)?,
        last_played: r.get(20)?,
        rel_dir: r.get(21).unwrap_or_default(),
    })
}

impl RomRow {
    /// Where this game belongs under a library's ROMs folder: its ES-DE system
    /// folder and the path inside that, which is exactly what its id is taken
    /// from. A download written here is found by the next scan as the same
    /// game. Platform alone for a row from a server too old to say.
    ///
    /// Both values come from the server and are joined onto a local folder a
    /// download is written into, so each piece must be a plain name: no `..`,
    /// no `.`, no drive or root. A row that fails that is filed by platform
    /// instead, which is where a download went before this existed.
    pub fn folder(&self) -> std::path::PathBuf {
        fn plain(part: &str) -> bool {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.contains(':')
                && std::path::Path::new(part).components().count() == 1
                && matches!(std::path::Path::new(part).components().next(), Some(std::path::Component::Normal(_)))
        }
        let parts: Vec<&str> = self.rel_dir.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
        match self.esde_system.as_deref().filter(|s| !s.is_empty()) {
            Some(system) if plain(system) && parts.iter().all(|p| plain(p)) => {
                let mut p = std::path::PathBuf::from(system);
                p.extend(parts);
                p
            }
            _ => std::path::PathBuf::from(&self.platform_slug),
        }
    }

    /// Where a download used to go, `<platform>/<file>`. Still looked at, so a
    /// game fetched by an older build counts as here.
    pub fn legacy_path(&self, roms: &Path) -> std::path::PathBuf {
        roms.join(&self.platform_slug).join(&self.fs_name)
    }

    /// Server-side screenshot paths, newest schema first, falling back to the
    /// single-path column for caches written before the list was stored.
    pub fn screenshots(&self) -> Vec<String> {
        if let Some(json) = &self.screenshots_json
            && let Ok(v) = serde_json::from_str::<Vec<String>>(json)
            && !v.is_empty()
        {
            return v;
        }
        self.screenshot_path.clone().into_iter().collect()
    }
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening cache at {}", path.display()))?;
        conn.execute_batch(SCHEMA).context("creating schema")?;
        // Older caches predate the artwork columns; add them in place rather
        // than forcing a full resync.
        for (col, ty) in [
            ("cover_path", "TEXT"), ("screenshot_path", "TEXT"),
            ("screenshots_json", "TEXT"), ("cover_small_path", "TEXT"),
            ("summary", "TEXT"), ("meta_json", "TEXT"), ("alt_names_json", "TEXT"),
            ("regions_json", "TEXT"), ("manual_path", "TEXT"),
            ("youtube_id", "TEXT"), ("multi_file", "INTEGER NOT NULL DEFAULT 0"),
            // ES-DE libraries live wherever the user put them, so a game's
            // location cannot be derived from <roms>/<slug>/<fs_name>.
            ("local_path", "TEXT"), ("esde_system", "TEXT"),
            // When the server last saw this game played. Drives the row of
            // recent games, and comes from the server rather than being
            // recorded here, so it follows you between machines.
            ("last_played", "TEXT"),
            // Where the game sits inside its system directory. Empty at the
            // top, `Aftermarket` or `AdditionalRoms/Homebrew` below it — the
            // folders ES-DE walks into and the front ends now draw.
            ("rel_dir", "TEXT"),
            // Who knows about this game: a scan of this machine's disk, the
            // server, or both. The sign of the id used to say this -- local
            // rows were negative -- and could only say one of the two, which
            // is why the same game was stored twice and folded together
            // afterwards. With one id per game it is one row with two flags.
            ("from_scan", "INTEGER NOT NULL DEFAULT 0"),
            ("from_server", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            let _ = conn.execute(&format!("ALTER TABLE roms ADD COLUMN {col} {ty}"), []);
        }
        let _ = conn.execute("ALTER TABLE id_migration ADD COLUMN ambiguous INTEGER NOT NULL DEFAULT 0", []);
        let cache = Self { conn };
        cache.adopt_stable_ids()?;
        Ok(cache)
    }

    /// Move a cache written under positional ids onto stable ones, once.
    ///
    /// Everything keyed by a game id here is either rebuilt by the next scan
    /// and sync -- the rows, collection membership, platforms -- or is play
    /// history, which nothing else holds. So the rebuildable part is emptied
    /// and the sync watermark dropped so the next pull is a full one, and each
    /// old id that has plays against it is written down with enough to find the
    /// game again. `apply_id_migration` finishes the job once rows exist.
    ///
    /// Rows under old ids are cleared on every open, not only the first. An
    /// older build still installed alongside -- the Flip, a copy of the app
    /// not yet updated -- can sync into this file again and write positional
    /// ids back into it, and those would sit here beside the stable rows for
    /// the same games.
    fn adopt_stable_ids(&self) -> Result<()> {
        let floor = crate::gameid::FLOOR;
        if self.meta_get("id_scheme").as_deref() == Some("stable") {
            // An old id already placed, now naming a different game, was reused
            // by an older build. Its earlier placement is dropped so it is
            // worked out again for the game it names now.
            self.conn.execute(
                "DELETE FROM id_moves WHERE old_id IN (
                     SELECT r.id FROM roms r JOIN id_moves m ON m.old_id = r.id
                      WHERE r.id > -?1 AND r.id < ?1
                        AND (m.platform_slug <> r.platform_slug OR m.fs_name <> r.fs_name))",
                [floor],
            )?;
            // One still waiting, now naming a different game: the plays under
            // it could be either, so none are attributed.
            self.conn.execute(
                "UPDATE id_migration SET ambiguous = 1 WHERE old_id IN (
                     SELECT r.id FROM roms r JOIN id_migration m ON m.old_id = r.id
                      WHERE r.id > -?1 AND r.id < ?1
                        AND (m.platform_slug <> r.platform_slug OR m.fs_name <> r.fs_name))",
                [floor],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO id_migration(old_id, platform_slug, fs_name, esde_system, rel_dir)
                      SELECT id, platform_slug, fs_name, esde_system, rel_dir FROM roms
                       WHERE id > -?1 AND id < ?1",
                [floor],
            )?;
            self.conn.execute("DELETE FROM roms WHERE id > -?1 AND id < ?1", [floor])?;
            // A row with neither flag was written by an older build syncing from
            // an updated server: the id is stable, but that build knows nothing
            // of ownership. It came from the server, so it is the server's --
            // left unflagged, the next scan would delete it as nobody's.
            self.conn.execute("UPDATE roms SET from_server = 1 WHERE from_scan = 0 AND from_server = 0", [])?;
            return Ok(());
        }
        self.conn.execute_batch(
            "BEGIN;
             INSERT OR IGNORE INTO id_migration(old_id, platform_slug, fs_name, esde_system, rel_dir)
                  SELECT id, platform_slug, fs_name, esde_system, rel_dir FROM roms;
             DELETE FROM roms;
             DELETE FROM collection_roms;
             DELETE FROM collections;
             DELETE FROM platforms;
             DELETE FROM meta WHERE key = 'roms_updated_through';
             INSERT OR REPLACE INTO meta(key, value) VALUES ('id_scheme', 'stable');
             COMMIT;",
        )?;
        Ok(())
    }

    /// Point play history recorded under old ids at the games they were.
    ///
    /// By location where the old row knew it (a local scan always did), and by
    /// platform and file name otherwise -- but only where that names exactly
    /// one game, because it is not unique: `Astrohawk (World) (Unl).zip` is in
    /// two folders of `sfc`. An old id that cannot be resolved yet is kept for
    /// the next pass rather than guessed at.
    pub fn apply_id_migration(&self) -> Result<usize> {
        let pending: Vec<Pending> = {
            let mut stmt = self.conn.prepare(
                "SELECT old_id, platform_slug, fs_name, esde_system, rel_dir FROM id_migration",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let ambiguous: std::collections::HashSet<i64> = self
            .conn
            .prepare("SELECT old_id FROM id_migration WHERE ambiguous = 1")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        let mut moved = 0;
        for (old, slug, fs_name, system, rel_dir) in pending {
            if ambiguous.contains(&old) {
                continue;
            }
            let new: Option<i64> = match (&system, &rel_dir) {
                (Some(system), Some(rel_dir)) => {
                    let id = crate::gameid::game_id(system, rel_dir, &fs_name);
                    self.conn
                        .query_row("SELECT id FROM roms WHERE id = ?1", [id], |r| r.get(0))
                        .ok()
                }
                _ => {
                    let ids: Vec<i64> = self
                        .conn
                        .prepare("SELECT id FROM roms WHERE platform_slug = ?1 AND fs_name = ?2")?
                        .query_map(params![slug, fs_name], |r| r.get(0))?
                        .collect::<std::result::Result<_, _>>()?;
                    (ids.len() == 1).then(|| ids[0])
                }
            };
            if let Some(new) = new {
                self.conn.execute("UPDATE plays SET rom_id = ?1 WHERE rom_id = ?2", params![new, old])?;
                self.conn.execute(
                    "INSERT OR REPLACE INTO id_moves(old_id, new_id, platform_slug, fs_name) VALUES (?1, ?2, ?3, ?4)",
                    params![old, new, slug, fs_name],
                )?;
                self.conn.execute("DELETE FROM id_migration WHERE old_id = ?1", [old])?;
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Whether anything still keyed by an old id that has not been placed can
    /// be given up on.
    ///
    /// True once a full pull from the server has completed under stable ids --
    /// from then on the server's copy is the one to use, and whatever could not
    /// be translated is rebuilt from it -- or once nothing is left waiting that a
    /// later scan or sync could still place. Frank's rule: an old id that cannot
    /// be translated is not kept around; the server's version replaces it.
    pub fn id_migration_settled(&self) -> Result<bool> {
        if self.meta_get("id_scheme_settled").as_deref() == Some("1") {
            return Ok(true);
        }
        let waiting: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM id_migration WHERE ambiguous = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(waiting == 0)
    }

    /// Every old id placed so far, and the stable id of the game it named.
    pub fn id_moves(&self) -> Result<std::collections::BTreeMap<i64, i64>> {
        Ok(self
            .conn
            .prepare("SELECT old_id, new_id FROM id_moves")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?)
    }

    /// How many games this machine has on disk that the server also lists.
    pub fn on_both(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM roms WHERE from_scan = 1 AND from_server = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Replace the stored collections wholesale.
    ///
    /// Virtual collections are recomputed by the server from scratch and their
    /// ids are derived from name+type, so a rename silently orphans the old
    /// row. Rebuilding is cheaper and more correct than reconciling.
    pub fn replace_collections(&mut self, items: &[api::Collection]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM collection_roms", [])?;
        tx.execute("DELETE FROM collections", [])?;
        {
            let mut ins = tx.prepare(
                "INSERT OR REPLACE INTO collections
                 (id, name, grp, description, rom_count, is_favorite)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut link = tx.prepare(
                "INSERT OR IGNORE INTO collection_roms(collection_id, rom_id) VALUES (?1, ?2)",
            )?;
            for c in items {
                // Trust the member list over the server's count: the two
                // disagree when a member rom has since been deleted.
                ins.execute(params![
                    c.id,
                    c.name,
                    c.group(),
                    c.description,
                    c.rom_ids.len() as i64,
                    c.is_favorite as i64,
                ])?;
                for rom_id in &c.rom_ids {
                    link.execute(params![c.id, rom_id])?;
                }
            }
        }
        tx.commit()?;
        Ok(items.len())
    }

    /// Give the consoles their real names.
    ///
    /// A scan only knows the directory -- `snes`, `gbc` -- so it writes that as
    /// the display name and the grid reads "snes snes 876 games". A server sync
    /// used to paper over this by overwriting the row with a name it fetched;
    /// with no server there is nothing to overwrite it, which is what a library
    /// served from a local ES-DE tree now is.
    ///
    /// Only where a name is actually known, and never over one a sync supplied:
    /// the table is the fallback, not the authority.
    pub fn name_platforms(&mut self, names: &[(String, String)]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut n = 0;
        {
            let mut up = tx.prepare(
                "UPDATE platforms SET display_name = ?2
                 WHERE fs_slug = ?1 AND COALESCE(NULLIF(display_name, ''), fs_slug) = fs_slug",
            )?;
            for (slug, name) in names {
                n += up.execute(params![slug, name])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Collection groups present, with how many collections each holds.
    ///
    /// Counts only collections that still have at least one ROM we know about,
    /// so a group cannot advertise entries that open empty.
    pub fn collection_groups(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.grp, COUNT(*) FROM collections c
             WHERE EXISTS (SELECT 1 FROM collection_roms cr JOIN roms r ON r.id = cr.rom_id
                           WHERE cr.collection_id = c.id)
             GROUP BY c.grp ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Collections in one group, largest first, skipping any that would open
    /// empty against the ROMs actually in the cache.
    pub fn collections_in(&self, group: &str) -> Result<Vec<CollectionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.name, c.grp, c.description,
                    (SELECT COUNT(*) FROM collection_roms cr JOIN roms r ON r.id = cr.rom_id
                     WHERE cr.collection_id = c.id) AS live,
                    c.is_favorite,
                    (SELECT group_concat(rom_id) FROM
                       (SELECT cr.rom_id FROM collection_roms cr JOIN roms r ON r.id = cr.rom_id
                        WHERE cr.collection_id = c.id LIMIT 4))
             FROM collections c
             WHERE c.grp = ?1 AND live > 0
             ORDER BY live DESC, c.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([group], |r| {
                Ok(CollectionRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    group: r.get(2)?,
                    description: r.get(3)?,
                    rom_count: r.get(4)?,
                    is_favorite: r.get::<_, i64>(5)? != 0,
                    sample_ids: r
                        .get::<_, Option<String>>(6)?
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|s| s.parse().ok())
                        .collect(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// ROMs belonging to one collection.
    /// Every game in a collection group, each listed once.
    ///
    /// `DISTINCT` matters: the "My collections" group holds overlapping lists —
    /// a game can be in both Arcade Fighting and Arcade Essentials — and
    /// without it the same download would be queued twice.
    pub fn roms_in_group(&self, grp: &str) -> Result<Vec<RomRow>> {
        // A subquery rather than a join: `collections` carries `id`, `name`
        // and `description` too, and joining it puts those in scope alongside
        // the same names on `roms`, which SQLite rejects as ambiguous.
        let sql = format!(
            "SELECT {ROM_COLUMNS} FROM roms WHERE id IN ( \
                 SELECT cr.rom_id FROM collection_roms cr \
                 JOIN collections c ON c.id = cr.collection_id \
                 WHERE c.grp = ?1) \
             ORDER BY 2, 3 COLLATE NOCASE"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([grp], rom_from_row)?
            .filter(|r| r.as_ref().map(shown).unwrap_or(true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn roms_in_collection(&self, id: &str) -> Result<Vec<RomRow>> {
        let sql = format!(
            "SELECT {ROM_COLUMNS} FROM roms r
             JOIN collection_roms cr ON cr.rom_id = r.id
             WHERE cr.collection_id = ?1
             ORDER BY COALESCE(NULLIF(r.name, ''), r.fs_name) COLLATE NOCASE"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([id], rom_from_row)?
            .filter(|r| r.as_ref().map(shown).unwrap_or(true))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Replace bare romset names with the real titles from the DAT map.
    ///
    /// Run after a sync, because a sync rewrites `name` from the server and
    /// would otherwise put `kof98` back.
    pub fn apply_arcade_names(
        &mut self,
        names: &std::collections::BTreeMap<String, String>,
    ) -> Result<usize> {
        if names.is_empty() {
            return Ok(0);
        }
        let rows: Vec<(i64, String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, COALESCE(name, ''), fs_name FROM roms WHERE platform_slug IN
                 (SELECT value FROM json_each(?1))",
            )?;
            let list = serde_json::to_string(crate::arcade::ARCADE_PLATFORMS)?;
            stmt.query_map([list], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let tx = self.conn.transaction()?;
        let mut n = 0;
        {
            let mut up = tx.prepare("UPDATE roms SET name = ?1 WHERE id = ?2")?;
            for (id, name, fs_name) in rows {
                if !crate::arcade::is_bare_romset(&name, &fs_name) {
                    continue;
                }
                let stem = fs_name.rsplit_once('.').map_or(fs_name.as_str(), |(s, _)| s);
                if let Some(title) = names.get(stem)
                    && !title.eq_ignore_ascii_case(&name)
                {
                    up.execute(params![title, id])?;
                    n += 1;
                }
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Replace the *locally scanned* half of the library.
    ///
    /// Wholesale for local rows and untouched for everything else: there is no
    /// server watermark to diff against, and a local scan is fast enough that
    /// reconciling would be more code for no gain. Collections are left alone —
    /// they belong to RomM and mean nothing here.
    ///
    /// Every game gets its stable id (see `gameid`), so a game the server also
    /// lists is the same row, not a second one to be folded in later. The scan
    /// answers for what only the disk knows -- where the file is, which system
    /// folder named it, the folders above it -- and takes the description only
    /// for games the server does not describe.
    ///
    /// Games no longer on disk lose the scan's claim: the row goes if the
    /// server does not list it either, and otherwise stays with no local path.
    pub fn replace_from_esde(&mut self, games: &[crate::esde::Game]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE roms SET from_scan = 0", [])?;
        let mut written = 0usize;
        {
            let mut ins = tx.prepare(
                "INSERT INTO roms (id, platform_slug, name, fs_name, fs_size_bytes,
                                   summary, meta_json, local_path, esde_system, multi_file,
                                   rel_dir, from_scan)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1)
                 ON CONFLICT(id) DO UPDATE SET
                    local_path    = excluded.local_path,
                    esde_system   = excluded.esde_system,
                    rel_dir       = excluded.rel_dir,
                    multi_file    = excluded.multi_file,
                    fs_size_bytes = excluded.fs_size_bytes,
                    from_scan     = 1,
                    -- The server's description where it gave one. Its rows
                    -- carry artwork paths and collection membership that a
                    -- gamelist does not, so a scan must not overwrite them.
                    platform_slug = CASE WHEN roms.from_server = 1 THEN roms.platform_slug ELSE excluded.platform_slug END,
                    name          = CASE WHEN roms.from_server = 1 THEN roms.name ELSE excluded.name END,
                    summary       = CASE WHEN roms.from_server = 1 THEN COALESCE(roms.summary, excluded.summary) ELSE excluded.summary END,
                    meta_json     = CASE WHEN roms.from_server = 1 THEN COALESCE(roms.meta_json, excluded.meta_json) ELSE excluded.meta_json END",
            )?;
            // OR IGNORE, not OR REPLACE: a platform the server already knows
            // about keeps its own id, name and count. This only has to make
            // sure a locally-found system has *a* row to hang games off.
            let mut plat = tx.prepare(
                "INSERT OR IGNORE INTO platforms (id, fs_slug, display_name, rom_count)
                 VALUES (?1, ?2, ?3, 0)",
            )?;
            let mut counts: std::collections::BTreeMap<&str, i64> = Default::default();
            let mut seen: std::collections::HashMap<i64, &crate::esde::Game> = Default::default();

            for g in games {
                let id = crate::gameid::game_id(&g.system, &g.rel_dir, &g.fs_name);
                // One in a hundred million for a library this size, and loud
                // when it happens: the second game would otherwise overwrite
                // the first's row and nobody would know which went missing.
                if let Some(first) = seen.get(&id) {
                    eprintln!(
                        "two games share id {id}: {} and {} -- only the first is listed",
                        crate::gameid::key(&first.system, &first.rel_dir, &first.fs_name),
                        crate::gameid::key(&g.system, &g.rel_dir, &g.fs_name),
                    );
                    continue;
                }
                seen.insert(id, g);
                let meta = serde_json::json!({
                    "genres": g.genres,
                    "player_count": g.players,
                    "average_rating": g.rating,
                    // `release_year`, not `first_release_date`: the latter is RomM's key
                    // and means epoch milliseconds. Writing a year under it made
                    // every scanned game report 1970 -- see `year_from_meta`.
                    "release_year": g.release_year,
                });
                ins.execute(params![
                    id,
                    g.platform_slug,
                    g.name,
                    g.fs_name,
                    g.size_bytes,
                    g.summary,
                    meta.to_string(),
                    g.path.to_string_lossy(),
                    g.system,
                    i64::from(g.path.is_dir()),
                    g.rel_dir,
                ])?;
                written += 1;
                *counts.entry(g.platform_slug.as_str()).or_default() += 1;
            }
            for slug in counts.keys() {
                plat.execute(params![crate::gameid::platform_id(slug), slug, slug])?;
            }
        }
        tx.execute("DELETE FROM roms WHERE from_scan = 0 AND from_server = 0", [])?;
        tx.execute(
            "UPDATE roms SET local_path = NULL WHERE from_scan = 0 AND from_server = 1",
            [],
        )?;
        // The count the grid shows is how many games are known, from either
        // source, so it is counted rather than assumed. Only the systems this
        // scan touched: a platform with nothing local keeps whatever the server
        // said about it.
        {
            let mut set = tx.prepare(
                "UPDATE platforms SET rom_count =
                     (SELECT COUNT(*) FROM roms WHERE roms.platform_slug = platforms.fs_slug)
                 WHERE fs_slug = ?1",
            )?;
            let mut seen: std::collections::BTreeSet<&str> = Default::default();
            for g in games {
                if seen.insert(g.platform_slug.as_str()) {
                    set.execute(params![g.platform_slug])?;
                }
            }
        }
        tx.commit()?;
        self.apply_id_migration()?;
        Ok(written)
    }

    pub fn collection_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM collections", [], |r| r.get(0))?)
    }

    fn meta_get(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .ok()
    }

    fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Remember server settings that change how we interpret its data, so a
    /// download verifies identically when the server is unreachable.
    pub fn save_server_config(&self, cfg: &api::ServerConfig) -> Result<()> {
        self.meta_set("excluded_files", &serde_json::to_string(&cfg.default_excluded_files)?)?;
        self.meta_set("excluded_exts", &serde_json::to_string(&cfg.default_excluded_extensions)?)?;
        self.meta_set("skip_hash", &cfg.skip_hash_calculation.to_string())?;
        Ok(())
    }

    /// `(excluded_files, excluded_extensions)` as last seen, if ever fetched.
    pub fn server_exclusions(&self) -> Option<(Vec<String>, Vec<String>)> {
        let files = serde_json::from_str(&self.meta_get("excluded_files")?).ok()?;
        let exts = serde_json::from_str(&self.meta_get("excluded_exts")?).ok()?;
        Some((files, exts))
    }

    pub fn server_version(&self) -> Option<String> {
        self.meta_get("server_version")
    }

    pub fn set_server_version(&self, v: &str) -> Result<()> {
        self.meta_set("server_version", v)
    }

    /// High-water mark of `updated_at` across everything we've stored.
    ///
    /// Using the max row timestamp rather than "now" avoids losing rows to
    /// clock skew between this machine and the server.
    pub fn watermark(&self) -> Option<String> {
        self.meta_get("roms_updated_through")
    }

    pub fn rom_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM roms", [], |r| r.get(0))?)
    }

    pub fn platforms(&self) -> Result<Vec<PlatformRow>> {
        // Count from the roms we actually hold, so the UI never promises rows
        // it cannot show.
        let mut stmt = self.conn.prepare(
            "SELECT p.fs_slug,
                    COALESCE(NULLIF(p.display_name, ''), p.fs_slug),
                    (SELECT COUNT(*) FROM roms r WHERE r.platform_slug = p.fs_slug)
             FROM platforms p
             -- By display name. It was ordered by ROM count, which put arcade
             -- and megadrive first and scattered everything else with no
             -- visible logic; alphabetical means a console is where you expect
             -- it. COLLATE NOCASE so casing does not split the order.
             ORDER BY 2 COLLATE NOCASE ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PlatformRow {
                    fs_slug: r.get(0)?,
                    display_name: r.get(1)?,
                    rom_count: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().filter(|p| p.rom_count > 0).collect())
    }

    /// Games you have starred, as a set of ids.
    ///
    /// RomM has no per-game favorite of its own — a favorite there is a
    /// *collection*, either one the server has flagged or one you named with a
    /// star, which is what the "★ Best of …" collections on this library are.
    /// So a game counts as a favorite when it is in one of those, and this
    /// stays true whether the starring happened here or on the web.
    pub fn favorite_ids(&self) -> Result<std::collections::HashSet<i64>> {
        let mut stmt = self.conn.prepare(FAVORITE_ROMS)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
        Ok(ids)
    }

    // --- Starring, written straight through --------------------------------
    //
    // The star has to light up the moment it is pressed, and a full
    // `replace_collections` means asking the server for every collection it
    // has. So the one row that changed is changed here too, and the next sync
    // overwrites it with the truth. If the server call failed, nothing below
    // is reached and the cache still says what the server says.

    /// The collection a star lives in, if this library already has one.
    ///
    /// Flagged first, named second — `is_favorite` is RomM's own idea of the
    /// thing, and a name is only how somebody spelt it when the server had no
    /// flag to offer. `platform` picks between per-system starred lists, which
    /// is how this library is arranged; a library with one starred collection
    /// for everything matches it whatever the platform.
    pub fn starred_collection(&self, platform: &str) -> Result<Option<(String, String)>> {
        let wanted = star_name(platform);
        let mut stmt = self.conn.prepare(
            "SELECT id, name FROM collections
             WHERE grp = 'user' AND (is_favorite = 1 OR name LIKE '★%')
             ORDER BY (name = ?1) DESC, is_favorite DESC, name COLLATE NOCASE
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![wanted])?;
        // Only an exact per-platform match, or a library-wide starred list,
        // will do. "★ Best of nes" must never catch a star on a SNES game.
        while let Some(r) = rows.next()? {
            let (id, name): (String, String) = (r.get(0)?, r.get(1)?);
            if name == wanted || !name.starts_with("★ Best of ") {
                return Ok(Some((id, name)));
            }
        }
        Ok(None)
    }

    /// Record a collection the server has just made, so the star has somewhere
    /// to point before the next full sync.
    pub fn remember_collection(&mut self, c: &api::Collection) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO collections
             (id, name, grp, description, rom_count, is_favorite)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![c.id, c.name, c.group(), c.description, c.rom_ids.len() as i64, c.is_favorite as i64],
        )?;
        Ok(())
    }

    /// How many games a collection claims, as recorded here.
    pub fn collection_size(&self, collection_id: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT rom_count FROM collections WHERE id = ?1",
            params![collection_id],
            |r| r.get(0),
        )?)
    }

    /// Put one game in a collection, or take it out, and keep the count right.
    pub fn set_membership(&mut self, collection_id: &str, rom_id: i64, member: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        if member {
            tx.execute(
                "INSERT OR IGNORE INTO collection_roms(collection_id, rom_id) VALUES (?1, ?2)",
                params![collection_id, rom_id],
            )?;
        } else {
            tx.execute(
                "DELETE FROM collection_roms WHERE collection_id = ?1 AND rom_id = ?2",
                params![collection_id, rom_id],
            )?;
        }
        // Counted, not incremented: a repeated star would otherwise inflate it.
        tx.execute(
            "UPDATE collections SET rom_count =
               (SELECT COUNT(*) FROM collection_roms WHERE collection_id = ?1)
             WHERE id = ?1",
            params![collection_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The games played most recently, newest first.
    ///
    /// Server-side timestamps, so this is the same list on every machine — the
    /// point of it is picking up where you left off, and "where you left off"
    /// is rarely the machine you are now sitting at.
    pub fn recently_played(&self, limit: usize) -> Result<Vec<RomRow>> {
        let sql = format!(
            "SELECT {ROM_COLUMNS} FROM roms \
             WHERE last_played IS NOT NULL AND last_played <> '' \
             ORDER BY last_played DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([limit as i64], rom_from_row)?
            .filter(|r| r.as_ref().map(shown).unwrap_or(true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record one finished session, and mark the game played.
    ///
    /// `last_played` is also set locally. It used to come only from the server,
    /// which meant playing a game on this machine changed nothing on the
    /// "continue playing" row until a sync happened to bring back a timestamp
    /// the server had no reason to have — so the row was often a list of what
    /// somebody else's machine had been doing.
    ///
    /// Sessions under a minute are dropped. Starting a game and quitting
    /// straight back out is a thing people do constantly — wrong game, wrong
    /// controller, checking it runs — and counting those makes "eleven
    /// sessions" mean nothing.
    pub fn record_play(&self, rom_id: i64, started_at: &str, seconds: i64) -> Result<bool> {
        if seconds < 60 {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO plays(rom_id, started_at, seconds) VALUES (?1, ?2, ?3)",
            rusqlite::params![rom_id, started_at, seconds],
        )?;
        self.conn.execute(
            "UPDATE roms SET last_played = ?1 WHERE id = ?2",
            rusqlite::params![started_at, rom_id],
        )?;
        Ok(true)
    }

    /// Time played per console, longest first.
    pub fn play_by_platform(&self) -> Result<Vec<(String, i64, i64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.platform_slug, SUM(p.seconds), COUNT(*), COUNT(DISTINCT p.rom_id)              FROM plays p JOIN roms r ON r.id = p.rom_id              GROUP BY r.platform_slug ORDER BY 2 DESC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Time played per game, longest first: `(rom, seconds, sessions, last)`.
    pub fn play_by_game(&self, limit: usize) -> Result<Vec<(RomRow, i64, i64, String)>> {
        let sql = format!(
            "SELECT {ROM_COLUMNS}, t.secs, t.runs, t.last FROM roms              JOIN (SELECT rom_id, SUM(seconds) secs, COUNT(*) runs, MAX(started_at) last                    FROM plays GROUP BY rom_id) t ON t.rom_id = roms.id              ORDER BY t.secs DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        // By name, not by position. ROM_COLUMNS holds a COALESCE with commas
        // inside it, so counting separators to find where the extra columns
        // start gives a number several too high.
        let rows = stmt
            .query_map([limit as i64], |r| {
                Ok((rom_from_row(r)?, r.get("secs")?, r.get("runs")?, r.get("last")?))
            })?
            .filter(|t| t.as_ref().map(|(row, ..)| shown(row)).unwrap_or(true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Games picked up more than once and never really played.
    ///
    /// The definition is deliberately narrow: opened on at least `runs`
    /// separate occasions, and under `under` seconds in total. Something you
    /// came back to and still bounced off — which is a more interesting list
    /// than "games you started once", because that one is just your library.
    pub fn abandoned(&self, runs: i64, under: i64, limit: usize) -> Result<Vec<(RomRow, i64, i64)>> {
        let sql = format!(
            "SELECT {ROM_COLUMNS}, t.secs, t.runs FROM roms              JOIN (SELECT rom_id, SUM(seconds) secs, COUNT(*) runs FROM plays                    GROUP BY rom_id) t ON t.rom_id = roms.id              WHERE t.runs >= ?1 AND t.secs < ?2 ORDER BY t.runs DESC, t.secs ASC LIMIT ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![runs, under, limit as i64], |r| {
                Ok((rom_from_row(r)?, r.get("secs")?, r.get("runs")?))
            })?
            .filter(|t| t.as_ref().map(|(row, ..)| shown(row)).unwrap_or(true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Total seconds and session count across everything.
    pub fn play_totals(&self) -> Result<(i64, i64, i64)> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(seconds), 0), COUNT(*), COUNT(DISTINCT rom_id) FROM plays",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    /// Every game, ordered as the platform pages order them.
    pub fn all_roms(&self) -> Result<Vec<RomRow>> {
        let sql =
            format!("SELECT {ROM_COLUMNS} FROM roms WHERE {NOT_A_WALKED_SHELF} \
                     ORDER BY 2, 3 COLLATE NOCASE");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], rom_from_row)?
            .filter(|r| r.as_ref().map(shown).unwrap_or(true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn roms_for(&self, platform_slug: &str) -> Result<Vec<RomRow>> {
        // Favorites first, then alphabetical within each group. A console
        // page is a wall of a few hundred names; the handful you actually play
        // being at the top is the difference between browsing and searching.
        let sql = format!(
            "SELECT {ROM_COLUMNS} FROM roms WHERE platform_slug = ?1 AND {NOT_A_WALKED_SHELF} \
             ORDER BY (id IN ({FAVORITE_ROMS})) DESC, 3 COLLATE NOCASE"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([platform_slug], rom_from_row)?
            .filter(|r| r.as_ref().map(shown).unwrap_or(true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The platform a file on disk belongs to, by exact path.
    ///
    /// Path inference expects `<roms>/<slug>/<file>`, which an ES-DE library
    /// does not satisfy: its directories are ES-DE system names (`dreamcast`,
    /// `neogeo`), not RomM slugs. Asking the index is exact and works for any
    /// layout.
    pub fn platform_for_path(&self, path: &Path) -> Option<String> {
        let p = path.to_string_lossy().to_string();
        self.conn
            .query_row(
                "SELECT platform_slug FROM roms WHERE local_path = ?1 LIMIT 1",
                [&p],
                |r| r.get(0),
            )
            .ok()
    }

    /// One row from a platform, for anything that needs the *shape* of a
    /// platform's entries rather than its contents.
    ///
    /// `warm_media` wanted a single row to work out which media folder to read,
    /// and was calling `roms_for` to get it: 861 rows built and filtered, with
    /// the cache lock held, to look at one. Measured in the desktop window on
    /// 2026-09-08, that was the reason opening SNES took 140ms before anything
    /// moved -- `roms` and `arrange_list` were queued behind it on the mutex.
    ///
    /// Same `WHERE` as `roms_for`, so it picks from the same set. No ordering:
    /// any row of the platform answers the question, and sorting several
    /// hundred to take the first is the cost being removed.
    pub fn any_rom_for(&self, platform_slug: &str) -> Result<Option<RomRow>> {
        let sql = format!(
            "SELECT {ROM_COLUMNS} FROM roms WHERE platform_slug = ?1 AND {NOT_A_WALKED_SHELF} LIMIT 1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map([platform_slug], rom_from_row)?;
        Ok(match rows.next() {
            Some(r) => Some(r?),
            None => None,
        })
    }

    pub fn rom_by_id(&self, id: i64) -> Result<Option<RomRow>> {
        let sql = format!("SELECT {ROM_COLUMNS} FROM roms WHERE id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map([id], rom_from_row)?;
        Ok(match rows.next() {
            Some(r) => Some(r?),
            None => None,
        })
    }

    /// Case-insensitive search over display name and filename.
    pub fn search(&self, needle: &str, limit: usize) -> Result<Vec<RomRow>> {
        let sql = format!(
            "SELECT {ROM_COLUMNS} FROM roms \
             WHERE name LIKE ?1 OR fs_name LIKE ?1 \
             ORDER BY 3 COLLATE NOCASE LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let pattern = format!("%{needle}%");
        let rows = stmt
            .query_map(params![pattern, limit as i64], rom_from_row)?
            .filter(|r| r.as_ref().map(shown).unwrap_or(true))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Drop cached roms the server no longer has.
    ///
    /// Incremental sync only ever learns about additions and changes, so
    /// without this a deleted rom lingers forever — as happened when 18
    /// multi-disc playlist stubs were replaced by folder ROMs and both showed
    /// up in the UI.
    ///
    /// Only the server's claim is withdrawn. A game this machine also found on
    /// disk stays, with `from_server` cleared; only a row the server alone knew
    /// about is deleted.
    pub fn prune_missing(&mut self, live_ids: &[i64]) -> Result<usize> {
        if live_ids.is_empty() {
            return Ok(0);
        }
        // A server still on positional ids lists ids that name nothing here,
        // and pruning against that list would delete every server row.
        if live_ids.iter().any(|id| crate::gameid::is_legacy(*id)) {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS live_ids(id INTEGER PRIMARY KEY);
             DELETE FROM live_ids;",
        )?;
        {
            let mut stmt = tx.prepare("INSERT OR IGNORE INTO live_ids(id) VALUES(?1)")?;
            for id in live_ids {
                stmt.execute([id])?;
            }
        }
        tx.execute(
            "UPDATE roms SET from_server = 0
              WHERE from_server = 1 AND from_scan = 1 AND id NOT IN (SELECT id FROM live_ids)",
            [],
        )?;
        let removed = tx.execute(
            "DELETE FROM roms
              WHERE from_server = 1 AND from_scan = 0 AND id NOT IN (SELECT id FROM live_ids)",
            [],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// Pull platforms and ROMs from the server into the cache.
    ///
    /// Returns `(platforms, roms_upserted, was_incremental)`.
    /// The one statement that lands a RomM row in the cache.
    ///
    /// A `const` so the regression test below runs the real SQL rather
    /// than a copy that can drift away from it.
    const ROM_UPSERT: &str = "INSERT INTO roms(id, platform_slug, name, fs_name,
                                          fs_size_bytes, md5_hash, sha1_hash,
                                          crc_hash, updated_at, cover_path,
                                          screenshot_path, screenshots_json,
                                          cover_small_path, summary, meta_json,
                                          alt_names_json, regions_json,
                                          manual_path, youtube_id, multi_file,
                                          last_played, esde_system, rel_dir,
                                          from_server)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,
                                ?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,1)
                         ON CONFLICT(id) DO UPDATE SET
                            platform_slug = excluded.platform_slug,
                            name          = excluded.name,
                            fs_name       = excluded.fs_name,
                            fs_size_bytes = excluded.fs_size_bytes,
                            md5_hash      = excluded.md5_hash,
                            sha1_hash     = excluded.sha1_hash,
                            crc_hash      = excluded.crc_hash,
                            updated_at    = excluded.updated_at,
                            cover_path    = excluded.cover_path,
                            screenshot_path = excluded.screenshot_path,
                            screenshots_json = excluded.screenshots_json,
                            cover_small_path = excluded.cover_small_path,
                            summary        = excluded.summary,
                            meta_json      = excluded.meta_json,
                            alt_names_json = excluded.alt_names_json,
                            regions_json   = excluded.regions_json,
                            manual_path    = excluded.manual_path,
                            youtube_id     = excluded.youtube_id,
                            multi_file     = excluded.multi_file,
                            -- Where the server keeps the file. The same values
                            -- the id is derived from, so they agree with a scan
                            -- of the same library by construction. COALESCE for
                            -- a server too old to send them.
                            esde_system    = COALESCE(excluded.esde_system, roms.esde_system),
                            rel_dir        = COALESCE(excluded.rel_dir, roms.rel_dir),
                            from_server    = 1,
                            -- Only a scan of this machine knows this. The
                            -- server sends NULL, and COALESCE keeps the path.
                            local_path     = COALESCE(excluded.local_path, roms.local_path),
                            -- Only when the server has one. An incremental
                            -- pull can return a row with no per-user block,
                            -- and letting that null out the timestamp would
                            -- empty the recent list on every sync.
                            last_played    = COALESCE(excluded.last_played, roms.last_played)";

    pub async fn sync(
        &mut self,
        client: &api::Client,
        force_full: bool,
    ) -> Result<(usize, usize, bool)> {
        let platforms = client.platforms().await?;
        {
            let tx = self.conn.transaction()?;
            for p in &platforms {
                // A slug can come back under a different id: deleting a platform
                // on the server and letting a scan recreate it renumbers it, and
                // the upsert below only resolves a conflict on `id`, leaving the
                // UNIQUE on `fs_slug` to fail. Drop the stale row first.
                //
                // Safe because `roms` keys off `platform_slug`, not this id, so
                // nothing downstream is orphaned by the renumbering.
                tx.execute(
                    "DELETE FROM platforms WHERE fs_slug = ?1 AND id <> ?2",
                    params![p.fs_slug, p.id],
                )?;
                tx.execute(
                    "INSERT INTO platforms(id, fs_slug, display_name, rom_count)
                     VALUES(?1, ?2, ?3, ?4)
                     ON CONFLICT(id) DO UPDATE SET
                        fs_slug = excluded.fs_slug,
                        display_name = excluded.display_name,
                        rom_count = excluded.rom_count",
                    params![
                        p.id,
                        p.fs_slug,
                        p.name.clone().unwrap_or_default(),
                        p.rom_count
                    ],
                )?;
            }
            tx.commit()?;
        }

        let since = if force_full { None } else { self.watermark() };
        let mut offset = 0u32;
        let mut upserted = 0usize;
        let mut high = since.clone().unwrap_or_default();

        loop {
            let page = client.roms(None, 500, offset, since.as_deref()).await?;
            if page.items.is_empty() {
                break;
            }
            // A server still numbering games by position. Its ids move when
            // its library changes and would land on nothing here, so nothing
            // is taken from it; the fix is on the server.
            if let Some(old) = page.items.iter().find(|r| crate::gameid::is_legacy(r.id)) {
                anyhow::bail!(
                    "the server numbers games the old way (it sent id {} for {}); \
                     update moose-service on it before syncing",
                    old.id,
                    old.fs_name
                );
            }
            let n = page.items.len();
            {
                let tx = self.conn.transaction()?;
                for rom in &page.items {
                    if let Some(ts) = &rom.updated_at
                        && ts.as_str() > high.as_str()
                    {
                        high = ts.clone();
                    }
                    // The server keeps the row when a file disappears and
                    // raises `missing_from_fs` instead of deleting it, which is
                    // right for the server and wrong for this cache: a cached
                    // game is one the grid offers and the launcher will try to
                    // open. Drop it here so a game that is gone stops being
                    // offered — and drop any row we already had, because a file
                    // deleted after the last sync arrives as an *update*, not as
                    // an absence.
                    if rom.missing_from_fs {
                        tx.execute(
                            "UPDATE roms SET from_server = 0 WHERE id = ?1 AND from_scan = 1",
                            params![rom.id],
                        )?;
                        tx.execute(
                            "DELETE FROM roms WHERE id = ?1 AND from_scan = 0",
                            params![rom.id],
                        )?;
                        continue;
                    }
                    tx.execute(
                        Self::ROM_UPSERT,
                        params![
                            rom.id,
                            rom.platform_fs_slug.clone().unwrap_or_default(),
                            rom.name.clone().unwrap_or_default(),
                            rom.fs_name,
                            rom.fs_size_bytes,
                            rom.md5_hash,
                            rom.sha1_hash,
                            rom.crc_hash,
                            rom.updated_at,
                            rom.path_cover_large,
                            rom.merged_screenshots.first(),
                            serde_json::to_string(&rom.merged_screenshots).ok(),
                            rom.path_cover_small,
                            rom.summary,
                            rom.metadatum.as_ref().and_then(|m| serde_json::to_string(m).ok()),
                            serde_json::to_string(&rom.alternative_names).ok(),
                            serde_json::to_string(&rom.regions).ok(),
                            rom.path_manual,
                            rom.youtube_video_id,
                            rom.has_multiple_files as i64,
                            rom.rom_user.as_ref().and_then(|u| u.last_played.clone()),
                            rom.esde_system,
                            rom.rel_dir,
                        ],
                    )?;
                }
                tx.commit()?;
            }
            upserted += n;
            offset += n as u32;
            if page.total > 0 && offset as i64 >= page.total {
                break;
            }
        }

        if !high.is_empty() {
            self.meta_set("roms_updated_through", &high)?;
        }
        self.apply_id_migration()?;
        // A full pull is everything the server has. Past this point an old id
        // it did not place names nothing the server knows, and the server's
        // copy of anything is what gets used.
        if since.is_none() {
            self.meta_set("id_scheme_settled", "1")?;
        }
        Ok((platforms.len(), upserted, since.is_some()))
    }
}

/// The most players a game supports, from RomM's free-text `player_count`.
///
/// The field is whatever the metadata source wrote: this library holds `"2"`,
/// `"1-2"`, `"1-4"`, `"1-8"` and `"8+"`. The useful question is "can someone
/// else play too", so the answer is the largest number in the string.
///
/// `None` for absent or unreadable, and that is not the same as one player:
/// two thirds of this library has no player count at all, and a filter that
/// treated unknown as single-player would quietly hide most of it.
pub fn max_players(raw: &str) -> Option<u8> {
    let mut best: Option<u8> = None;
    let mut cur = String::new();
    // Walk the digits rather than splitting on a separator, because the
    // separator varies: "1-2", "1 - 4", "2 players", "8+".
    for c in raw.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            cur.push(c);
            continue;
        }
        if !cur.is_empty() {
            if let Ok(n) = cur.parse::<u8>() {
                best = Some(best.map_or(n, |b: u8| b.max(n)));
            }
            cur.clear();
        }
    }
    best.filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cache on disk rather than in memory: `open` runs the migration path
    /// too, which is where the tolerant column reads below come from.
    fn cache(name: &str) -> Cache {
        let dir = std::env::temp_dir().join(format!("moose-rack-cache-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Cache::open(&dir.join("c.sqlite3")).expect("opening a fresh cache")
    }

    fn add_platform(c: &Cache, id: i64, slug: &str, display: &str) {
        c.conn
            .execute(
                "INSERT INTO platforms(id, fs_slug, display_name, rom_count) VALUES(?1,?2,?3,0)",
                params![id, slug, display],
            )
            .unwrap();
    }

    /// Sessions, and what they add up to.
    ///
    /// The SQL here is the whole feature: a mistake in a JOIN or a GROUP BY
    /// does not fail, it produces a plausible number, and a plausible wrong
    /// number about how you spent a year is worse than no number.
    /// Every shape this library actually holds, taken from the real column.
    #[test]
    fn the_player_count_is_read_as_the_most_it_supports() {
        for (raw, want) in [
            ("1", Some(1)),
            ("2", Some(2)),
            ("1-2", Some(2)),
            ("1-4", Some(4)),
            ("1-8", Some(8)),
            ("8+", Some(8)),
            ("4", Some(4)),
            ("1 - 3", Some(3)),
            ("2 players", Some(2)),
        ] {
            assert_eq!(max_players(raw), want, "{raw:?}");
        }
    }

    /// Unknown is not one player. Two thirds of this library has no player
    /// count, and calling those single-player would hide most of it from a
    /// filter that is supposed to reveal things.
    #[test]
    fn an_absent_or_unreadable_count_is_unknown_not_one() {
        assert_eq!(max_players(""), None);
        assert_eq!(max_players("unknown"), None);
        assert_eq!(max_players("0"), None, "zero players is not a fact about a game");
    }

    #[test]
    fn play_time_adds_up_per_game_and_per_console() {
        let c = cache("plays");
        add_platform(&c, 1, "snes", "Super Nintendo");
        add_platform(&c, 2, "psx", "PlayStation");
        add_rom(&c, 10, "snes", "Chrono Trigger", "ct.sfc");
        add_rom(&c, 11, "snes", "Super Metroid", "sm.sfc");
        add_rom(&c, 20, "psx", "Vagrant Story", "vs.bin");

        c.record_play(10, "2026-01-01T10:00:00", 3600).unwrap();
        c.record_play(10, "2026-01-02T10:00:00", 1800).unwrap();
        c.record_play(11, "2026-01-03T10:00:00", 600).unwrap();
        c.record_play(20, "2026-01-04T10:00:00", 7200).unwrap();

        let by_platform = c.play_by_platform().unwrap();
        // PlayStation first: two hours beats one and a half, and the ordering
        // is what the page is for.
        assert_eq!(by_platform[0].0, "psx");
        assert_eq!(by_platform[0].1, 7200);
        assert_eq!(by_platform[1], ("snes".to_owned(), 6000, 3, 2));

        let by_game = c.play_by_game(10).unwrap();
        assert_eq!(by_game[0].0.id, 20);
        assert_eq!(by_game[1].0.name, "Chrono Trigger");
        assert_eq!(by_game[1].1, 5400, "two sessions on one game must sum");
        assert_eq!(by_game[1].2, 2, "and count as two");
        assert_eq!(by_game[1].3, "2026-01-02T10:00:00", "the later of the two");

        assert_eq!(c.play_totals().unwrap(), (13_200, 4, 3));
    }

    /// Starting a game and quitting straight back out is something people do
    /// constantly — wrong game, wrong controller, checking it runs. Counting
    /// those makes a session count mean nothing.
    #[test]
    fn a_glance_at_a_game_is_not_a_session() {
        let c = cache("plays-short");
        add_platform(&c, 1, "snes", "Super Nintendo");
        add_rom(&c, 10, "snes", "Chrono Trigger", "ct.sfc");

        assert!(!c.record_play(10, "2026-01-01T10:00:00", 12).unwrap());
        assert!(!c.record_play(10, "2026-01-01T10:01:00", 59).unwrap());
        assert!(c.record_play(10, "2026-01-01T10:02:00", 60).unwrap());
        assert_eq!(c.play_totals().unwrap(), (60, 1, 1));
    }

    /// Playing something here has to show up on the "continue playing" row
    /// here. It used to wait on the server sending back a timestamp it had no
    /// reason to have, so the row showed what other machines had been doing.
    #[test]
    fn playing_a_game_marks_it_played_without_asking_the_server() {
        let c = cache("plays-recent");
        add_platform(&c, 1, "snes", "Super Nintendo");
        add_rom(&c, 10, "snes", "Chrono Trigger", "ct.sfc");
        assert!(c.recently_played(5).unwrap().is_empty());

        c.record_play(10, "2026-01-01T10:00:00", 900).unwrap();
        let recent = c.recently_played(5).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, 10);
        // The timestamp comes back on the row, not only into the ordering.
        // These columns are read by position, so adding one to the list
        // without adding it to the reader shifts every field after it —
        // quietly, into a field of the same type.
        assert_eq!(recent[0].last_played.as_deref(), Some("2026-01-01T10:00:00"));
    }

    fn local_game(slug: &str, system: &str, name: &str, file: &str) -> crate::esde::Game {
        crate::esde::Game {
            platform_slug: slug.into(),
            system: system.into(),
            name: name.into(),
            fs_name: file.into(),
            rel_dir: String::new(),
            path: std::path::PathBuf::from(format!("/ES-DE/ROMs/{system}/{file}")),
            size_bytes: 42,
            summary: None,
            genres: vec![],
            players: None,
            rating: None,
            release_year: None,
        }
    }

    /// A row the way a server sync writes it: through the real upsert, with a
    /// server's stable id and location.
    fn server_row(c: &Cache, id: i64, slug: &str, name: &str, file: &str, system: &str, rel: &str) {
        c.conn
            .execute(
                Cache::ROM_UPSERT,
                params![
                    id, slug, name, file, 512i64,
                    None::<String>, None::<String>, None::<String>, None::<String>,
                    Some("/cover.png"), None::<String>, None::<String>, None::<String>,
                    Some("From the server."), None::<String>, None::<String>, None::<String>,
                    None::<String>, None::<String>, 0i64, None::<String>,
                    Some(system), Some(rel),
                ],
            )
            .unwrap();
    }

    fn ids(c: &Cache) -> Vec<i64> {
        c.conn
            .prepare("SELECT id FROM roms ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn flags(c: &Cache, id: i64) -> (i64, i64) {
        c.conn
            .query_row("SELECT from_scan, from_server FROM roms WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
    }

    /// A scanned game's id is the one every other machine derives for the same
    /// file, not a position in this scan.
    #[test]
    fn a_scanned_game_gets_its_stable_id() {
        let mut c = cache("stable-scan-ids");
        c.replace_from_esde(&[
            local_game("ps2", "ps2", "Some PS2 Game", "g.iso"),
            local_game("snes", "snes", "Chrono Trigger", "ct.sfc"),
        ])
        .unwrap();
        let mut want = vec![
            crate::gameid::game_id("ps2", "", "g.iso"),
            crate::gameid::game_id("snes", "", "ct.sfc"),
        ];
        want.sort();
        assert_eq!(ids(&c), want);
    }

    /// Two files with one name in different folders are two games. Keyed on
    /// platform and file name they were one, and the second scan overwrote the
    /// first; both of these are in the real library.
    #[test]
    fn the_same_name_in_two_folders_is_two_rows() {
        let mut c = cache("same-name-two-folders");
        let mut a = local_game("sfc", "sfc", "Astrohawk", "Astrohawk (World) (Unl).zip");
        a.rel_dir = "AdditionalRoms/Public Domain".into();
        let mut b = a.clone();
        b.rel_dir = "AdditionalRoms/Homebrew".into();
        c.replace_from_esde(&[a, b]).unwrap();
        assert_eq!(c.rom_count().unwrap(), 2);
    }

    /// A game downloaded to where `folder` says is the same game, with the
    /// same id, when this machine next scans. Downloads used to go to
    /// `<platform>/<file>`, which dropped the system folder and every folder
    /// inside it: the game came back under a different id, and two files with
    /// one name in different folders landed on the same path.
    #[test]
    fn a_download_is_found_again_under_the_servers_id() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-download-home");
        std::fs::remove_dir_all(&dir).ok();
        let (system, rel, file) = ("sfc", "AdditionalRoms/Homebrew", "Astrohawk (World) (Unl).zip");
        let server_id = crate::gameid::game_id(system, rel, file);

        let c = cache("download-home");
        server_row(&c, server_id, "snes", "Astrohawk", file, system, rel);
        let row = c.rom_by_id(server_id).unwrap().unwrap();
        assert_eq!(row.folder(), Path::new("sfc/AdditionalRoms/Homebrew"));

        // Put the file where a download would, then scan the tree.
        let roms = dir.join("ROMs");
        std::fs::create_dir_all(roms.join(row.folder())).unwrap();
        std::fs::write(roms.join(row.folder()).join(file), b"rom").unwrap();
        let layout = crate::esde::Layout::new(&dir, Some(&roms));
        let (games, _) = crate::esde::scan(&layout, &crate::coremap::CoreMap::embedded()).unwrap();
        let found = games.iter().find(|g| g.fs_name == file).expect("the scan finds the download");
        assert_eq!(crate::gameid::game_id(&found.system, &found.rel_dir, &found.fs_name), server_id);
    }

    /// The folder comes from the server and a download is written into it, so
    /// a value that would climb out of the ROMs folder is not used.
    #[test]
    fn a_server_cannot_point_a_download_outside_the_roms_folder() {
        let c = cache("folder-escape");
        for (system, rel) in [
            ("..", ""),
            ("snes", "../../.ssh"),
            ("/etc", ""),
            ("snes", "a/./b"),
            ("C:", "x"),
            ("snes", "ok\\..\\up"),
        ] {
            let id = crate::gameid::game_id(system, rel, "g.sfc");
            server_row(&c, id, "snes", "G", "g.sfc", system, rel);
            let row = c.rom_by_id(id).unwrap().unwrap();
            assert_eq!(row.folder(), Path::new("snes"), "{system:?} {rel:?}");
        }
        let id = crate::gameid::game_id("sfc", "AdditionalRoms/Homebrew", "g.sfc");
        server_row(&c, id, "snes", "G", "g.sfc", "sfc", "AdditionalRoms/Homebrew");
        assert_eq!(c.rom_by_id(id).unwrap().unwrap().folder(), Path::new("sfc/AdditionalRoms/Homebrew"));
    }

    /// An older build syncing from an updated server writes stable-id rows
    /// with neither ownership flag. They are the server's; left unflagged, the
    /// next scan deleted them.
    #[test]
    fn rows_an_old_build_synced_are_kept_as_the_servers() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-unflagged");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite3");
        let id = crate::gameid::game_id("gba", "", "kirby.gba");
        {
            let c = Cache::open(&path).unwrap();
            c.conn
                .execute(
                    "INSERT INTO roms(id, platform_slug, name, fs_name) VALUES (?1, 'gba', 'Kirby', 'kirby.gba')",
                    [id],
                )
                .unwrap();
        }
        let mut c = Cache::open(&path).unwrap();
        assert_eq!(flags(&c, id), (0, 1));
        c.replace_from_esde(&[]).unwrap();
        assert!(c.rom_by_id(id).unwrap().is_some(), "a scan does not delete it");
    }

    #[test]
    fn pruning_against_a_positional_list_deletes_nothing() {
        let mut c = cache("prune-legacy-list");
        let id = crate::gameid::game_id("snes", "", "a.sfc");
        server_row(&c, id, "snes", "A", "a.sfc", "snes", "");
        assert_eq!(c.prune_missing(&[1, 2, 3]).unwrap(), 0);
        assert!(c.rom_by_id(id).unwrap().is_some());
    }

    /// The same round trip through the real downloader: a server row fetched
    /// over HTTP by `download::fetch`, then a scan of the folder it wrote. The
    /// test above writes to `folder()` by hand, which would still pass if the
    /// downloader put the file somewhere else.
    #[tokio::test]
    async fn a_real_download_scans_back_to_the_servers_id() {
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join("moose-rack-cache-test-real-download");
        std::fs::remove_dir_all(&dir).ok();
        let (system, rel, file) = ("sfc", "AdditionalRoms/Homebrew", "Astrohawk (World) (Unl).zip");
        let body = b"not really a rom".to_vec();

        // One request, answered with the file. Enough for a fresh download.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = body.clone();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                served.len()
            );
            sock.write_all(head.as_bytes()).unwrap();
            sock.write_all(&served).unwrap();
            request
        });

        let server_id = crate::gameid::game_id(system, rel, file);
        let c = cache("real-download");
        server_row(&c, server_id, "snes", "Astrohawk", file, system, rel);
        let row = c.rom_by_id(server_id).unwrap().unwrap();
        let folder = row.folder();
        let target = crate::download::Target {
            rom_id: row.id,
            members: &[],
            fs_name: &row.fs_name,
            folder: &folder,
            expected_size: Some(body.len() as u64),
            md5: None,
            sha1: None,
            multi_file: false,
        };
        let roms = dir.join("ROMs");
        crate::util::install_tls();
        crate::download::fetch(&reqwest::Client::new(), &base, "", &target, &roms, |_, _| {})
            .await
            .expect("the download completes");
        let request = server.join().unwrap();
        assert!(request.starts_with(&format!("GET /api/roms/{server_id}/content/")), "{request}");

        let layout = crate::esde::Layout::new(&dir, Some(&roms));
        let (games, _) = crate::esde::scan(&layout, &crate::coremap::CoreMap::embedded()).unwrap();
        let found = games.iter().find(|g| g.fs_name == file).expect("the scan finds what was downloaded");
        assert_eq!(crate::gameid::game_id(&found.system, &found.rel_dir, &found.fs_name), server_id);
    }

    /// A real scan of a tree with folders inside a system, compared with the id
    /// written the way a Linux server writes it. On macOS and Linux this checks
    /// the scan; on the Windows CI runner the scanner sees backslashes, and this
    /// is the test that says the two still agree.
    #[test]
    fn a_scan_on_this_platform_agrees_with_a_linux_servers_id() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-platform-scan");
        std::fs::remove_dir_all(&dir).ok();
        let roms = dir.join("ROMs");
        let nested = roms.join("snes").join("AdditionalRoms").join("Homebrew");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("g.sfc"), b"x").unwrap();
        let layout = crate::esde::Layout::new(&dir, Some(&roms));
        let (games, _) = crate::esde::scan(&layout, &crate::coremap::CoreMap::embedded()).unwrap();
        let g = games.iter().find(|g| g.fs_name == "g.sfc").expect("scanned");
        assert_eq!(
            crate::gameid::game_id(&g.system, &g.rel_dir, &g.fs_name),
            crate::gameid::game_id("snes", "AdditionalRoms/Homebrew", "g.sfc"),
            "scanned as {:?} / {:?}",
            g.system,
            g.rel_dir
        );
    }

    #[test]
    fn a_row_from_an_older_server_falls_back_to_its_platform() {
        let c = cache("folder-fallback");
        add_rom(&c, crate::gameid::game_id("x", "", "g.sfc"), "snes", "G", "g.sfc");
        let row = c.rom_by_id(crate::gameid::game_id("x", "", "g.sfc")).unwrap().unwrap();
        assert_eq!(row.folder(), Path::new("snes"));
    }

    /// A server sync must not blank the path to a game that is on this disk.
    #[test]
    fn a_sync_leaves_a_local_path_where_it_found_one() {
        let mut c = cache("sync-keeps-local-path");
        c.replace_from_esde(&[local_game("snes", "snes", "Chrono Trigger", "ct.sfc")]).unwrap();
        let id = crate::gameid::game_id("snes", "", "ct.sfc");
        server_row(&c, id, "snes", "Chrono Trigger", "ct.sfc", "snes", "");

        let r = c.rom_by_id(id).unwrap().expect("the upserted row");
        assert_eq!(
            r.local_path.as_deref(),
            Some("/ES-DE/ROMs/snes/ct.sfc"),
            "the server has no opinion about local files and must not erase one"
        );
        assert_eq!(r.esde_system.as_deref(), Some("snes"));
    }

    /// The same game found on disk and on the server is one row, in either
    /// order, with no folding step. Both sides derive the id from where the
    /// file is. The server's description wins, and the disk supplies the path.
    #[test]
    fn a_game_found_on_disk_and_on_the_server_is_one_game() {
        let id = crate::gameid::game_id("snes", "", "ct.sfc");

        let mut scan_first = cache("one-game-scan-first");
        scan_first.replace_from_esde(&[local_game("snes", "snes", "CT (gamelist)", "ct.sfc")]).unwrap();
        server_row(&scan_first, id, "snes", "Chrono Trigger", "ct.sfc", "snes", "");

        let mut sync_first = cache("one-game-sync-first");
        server_row(&sync_first, id, "snes", "Chrono Trigger", "ct.sfc", "snes", "");
        sync_first.replace_from_esde(&[local_game("snes", "snes", "CT (gamelist)", "ct.sfc")]).unwrap();

        for c in [&scan_first, &sync_first] {
            assert_eq!(c.rom_count().unwrap(), 1);
            assert_eq!(flags(c, id), (1, 1));
            let r = c.rom_by_id(id).unwrap().unwrap();
            assert_eq!(r.name, "Chrono Trigger", "the server names it");
            assert_eq!(r.cover_path.as_deref(), Some("/cover.png"), "and keeps its artwork");
            assert_eq!(r.local_path.as_deref(), Some("/ES-DE/ROMs/snes/ct.sfc"), "the disk says where");
        }
    }

    /// What the server stops listing and what the disk stops holding are
    /// separate claims. Withdrawing one leaves a game the other still vouches
    /// for; a game nobody vouches for goes.
    #[test]
    fn each_side_withdraws_only_its_own_claim() {
        let mut c = cache("claims");
        let both = crate::gameid::game_id("snes", "", "both.sfc");
        let server_only = crate::gameid::game_id("snes", "", "server.sfc");
        server_row(&c, both, "snes", "Both", "both.sfc", "snes", "");
        server_row(&c, server_only, "snes", "Server", "server.sfc", "snes", "");
        c.replace_from_esde(&[
            local_game("snes", "snes", "Both", "both.sfc"),
            local_game("snes", "snes", "Mine", "mine.sfc"),
        ])
        .unwrap();
        let mine = crate::gameid::game_id("snes", "", "mine.sfc");

        // The server now lists nothing of these.
        c.prune_missing(&[crate::gameid::game_id("snes", "", "elsewhere.sfc")]).unwrap();
        assert_eq!(flags(&c, both), (1, 0), "still on disk");
        assert_eq!(flags(&c, mine), (1, 0), "never on the server");
        assert!(c.rom_by_id(server_only).unwrap().is_none(), "nobody vouches for it");

        // And the disk loses one.
        c.replace_from_esde(&[local_game("snes", "snes", "Mine", "mine.sfc")]).unwrap();
        assert!(c.rom_by_id(both).unwrap().is_none());
        assert_eq!(c.rom_count().unwrap(), 1);
    }

    /// A game the server still lists keeps its row when it leaves this disk,
    /// but stops claiming a local file.
    #[test]
    fn leaving_the_disk_drops_the_path_not_the_game() {
        let mut c = cache("left-disk");
        let id = crate::gameid::game_id("snes", "", "ct.sfc");
        server_row(&c, id, "snes", "Chrono Trigger", "ct.sfc", "snes", "");
        c.replace_from_esde(&[local_game("snes", "snes", "CT", "ct.sfc")]).unwrap();
        c.replace_from_esde(&[]).unwrap();
        let r = c.rom_by_id(id).unwrap().expect("still listed by the server");
        assert_eq!(r.local_path, None);
        assert_eq!(flags(&c, id), (0, 1));
    }

    /// Play history is the one thing in this cache nothing else has, so moving
    /// to stable ids must carry it across rather than start the history over.
    #[test]
    fn play_history_survives_the_move_to_stable_ids() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-adopt");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite3");
        {
            // A cache as the old build left it: a local row at -3 and a server
            // row at 42, each played, and no scheme marker.
            let c = Cache::open(&path).unwrap();
            c.conn.execute_batch(
                "DELETE FROM meta WHERE key = 'id_scheme';
                 INSERT INTO roms(id, platform_slug, name, fs_name, esde_system, rel_dir)
                      VALUES (-3, 'snes', 'CT', 'ct.sfc', 'snes', '');
                 INSERT INTO roms(id, platform_slug, name, fs_name)
                      VALUES (42, 'gba', 'Kirby', 'kirby.gba');
                 INSERT INTO plays(rom_id, started_at, seconds) VALUES (-3, '2026-01-01', 600);
                 INSERT INTO plays(rom_id, started_at, seconds) VALUES (42, '2026-01-02', 300);
                 INSERT INTO plays(rom_id, started_at, seconds) VALUES (42, '2026-01-03', 60);",
            ).unwrap();
        }
        let mut c = Cache::open(&path).unwrap();
        assert_eq!(c.rom_count().unwrap(), 0, "rows are rebuilt, not kept under old ids");

        // The scan resolves the local game by location, the sync the other by
        // platform and name.
        c.replace_from_esde(&[local_game("snes", "snes", "CT", "ct.sfc")]).unwrap();
        let kirby = crate::gameid::game_id("gba", "", "kirby.gba");
        server_row(&c, kirby, "gba", "Kirby", "kirby.gba", "gba", "");
        c.apply_id_migration().unwrap();

        let plays = |id: i64| -> i64 {
            c.conn.query_row("SELECT COUNT(*) FROM plays WHERE rom_id = ?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(plays(crate::gameid::game_id("snes", "", "ct.sfc")), 1);
        assert_eq!(plays(kirby), 2);
        let left: i64 = c.conn.query_row("SELECT COUNT(*) FROM id_migration", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "nothing left waiting");

        // And it only happens once.
        drop(c);
        let c = Cache::open(&path).unwrap();
        assert_eq!(c.rom_count().unwrap(), 2, "a second open does not wipe it again");
    }

    /// An old build syncing into a cache that has already moved over writes
    /// positional ids again. The next open clears them, and keeps their plays
    /// waiting to be matched.
    #[test]
    fn rows_an_old_build_writes_later_are_cleared_on_open() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-late-legacy");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite3");
        let stable = crate::gameid::game_id("snes", "", "ct.sfc");
        {
            let c = Cache::open(&path).unwrap();
            server_row(&c, stable, "snes", "CT", "ct.sfc", "snes", "");
            c.conn.execute_batch(
                "INSERT INTO roms(id, platform_slug, name, fs_name) VALUES (12, 'snes', 'CT', 'ct.sfc');
                 INSERT INTO plays(rom_id, started_at, seconds) VALUES (12, '2026-02-01', 90);",
            ).unwrap();
        }
        let c = Cache::open(&path).unwrap();
        assert_eq!(ids(&c), vec![stable], "the positional row is gone, the stable one kept");
        c.apply_id_migration().unwrap();
        let on_stable: i64 = c.conn
            .query_row("SELECT COUNT(*) FROM plays WHERE rom_id = ?1", [stable], |r| r.get(0))
            .unwrap();
        assert_eq!(on_stable, 1, "its play went to the game it was");
    }

    /// Placing an old id is recorded, not consumed, so the other stores keyed by
    /// game id can be moved after the plays have been.
    #[test]
    fn every_placed_old_id_is_remembered() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-moves-kept");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite3");
        {
            let c = Cache::open(&path).unwrap();
            // Unplayed as well: backups and the state ledger key on games that
            // were never timed.
            c.conn.execute_batch(
                "DELETE FROM meta WHERE key = 'id_scheme';
                 INSERT INTO roms(id, platform_slug, name, fs_name, esde_system, rel_dir)
                      VALUES (-3, 'snes', 'CT', 'ct.sfc', 'snes', '');",
            ).unwrap();
        }
        let mut c = Cache::open(&path).unwrap();
        c.replace_from_esde(&[local_game("snes", "snes", "CT", "ct.sfc")]).unwrap();
        let moves = c.id_moves().unwrap();
        assert_eq!(moves.get(&-3), Some(&crate::gameid::game_id("snes", "", "ct.sfc")));
    }

    /// An older build that reuses an old number for a different game, while
    /// plays under that number are still waiting, must not have them handed to
    /// either game.
    #[test]
    fn a_reused_old_id_attributes_nothing() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-reused");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite3");
        {
            let c = Cache::open(&path).unwrap();
            c.conn.execute_batch(
                "DELETE FROM meta WHERE key = 'id_scheme';
                 INSERT INTO roms(id, platform_slug, name, fs_name) VALUES (-5, 'gba', 'Kirby', 'kirby.gba');
                 INSERT INTO plays(rom_id, started_at, seconds) VALUES (-5, '2026-01-01', 60);",
            ).unwrap();
        }
        // Waiting on -5 = Kirby. Now an old build writes -5 = Zelda and plays it.
        {
            let c = Cache::open(&path).unwrap();
            c.conn.execute_batch(
                "INSERT INTO roms(id, platform_slug, name, fs_name) VALUES (-5, 'gba', 'Zelda', 'zelda.gba');
                 INSERT INTO plays(rom_id, started_at, seconds) VALUES (-5, '2026-02-01', 90);",
            ).unwrap();
        }
        let c = Cache::open(&path).unwrap();
        let kirby = crate::gameid::game_id("gba", "", "kirby.gba");
        let zelda = crate::gameid::game_id("gba", "", "zelda.gba");
        server_row(&c, kirby, "gba", "Kirby", "kirby.gba", "gba", "");
        server_row(&c, zelda, "gba", "Zelda", "zelda.gba", "gba", "");
        c.apply_id_migration().unwrap();
        let on = |id: i64| -> i64 {
            c.conn.query_row("SELECT COUNT(*) FROM plays WHERE rom_id = ?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(on(kirby), 0, "not merged into Kirby");
        assert_eq!(on(zelda), 0, "nor into Zelda");
        assert_eq!(on(-5), 2, "left where they were");
    }

    /// Settled once nothing is waiting, or once a full pull has happened even
    /// if something is.
    #[test]
    fn the_migration_settles_when_nothing_can_still_be_placed() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-settled");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite3");
        {
            let c = Cache::open(&path).unwrap();
            c.conn.execute_batch(
                "DELETE FROM meta WHERE key = 'id_scheme';
                 INSERT INTO roms(id, platform_slug, name, fs_name) VALUES (-5, 'gba', 'Gone', 'gone.gba');",
            ).unwrap();
        }
        let c = Cache::open(&path).unwrap();
        assert!(!c.id_migration_settled().unwrap(), "-5 could still be placed by a sync");
        c.meta_set("id_scheme_settled", "1").unwrap();
        assert!(c.id_migration_settled().unwrap(), "after a full pull it is final");

        let fresh = cache("settled-fresh");
        assert!(fresh.id_migration_settled().unwrap(), "nothing waiting is settled");
    }

    /// An old id that names more than one game is left waiting, not guessed.
    #[test]
    fn an_ambiguous_old_id_is_not_guessed() {
        let dir = std::env::temp_dir().join("moose-rack-cache-test-ambiguous");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite3");
        {
            let c = Cache::open(&path).unwrap();
            c.conn.execute_batch(
                "DELETE FROM meta WHERE key = 'id_scheme';
                 INSERT INTO roms(id, platform_slug, name, fs_name)
                      VALUES (7, 'sfc', 'Astrohawk', 'Astrohawk (World) (Unl).zip');
                 INSERT INTO plays(rom_id, started_at, seconds) VALUES (7, '2026-01-01', 60);",
            ).unwrap();
        }
        let mut c = Cache::open(&path).unwrap();
        let mut a = local_game("sfc", "sfc", "Astrohawk", "Astrohawk (World) (Unl).zip");
        a.rel_dir = "AdditionalRoms/Public Domain".into();
        let mut b = a.clone();
        b.rel_dir = "AdditionalRoms/Homebrew".into();
        c.replace_from_esde(&[a, b]).unwrap();
        let waiting: i64 = c.conn.query_row("SELECT COUNT(*) FROM id_migration", [], |r| r.get(0)).unwrap();
        assert_eq!(waiting, 1);
        let moved: i64 = c.conn.query_row("SELECT COUNT(*) FROM plays WHERE rom_id <> 7", [], |r| r.get(0)).unwrap();
        assert_eq!(moved, 0);
    }

    /// Every column read by position, checked in one place.
    ///
    /// `ROM_COLUMNS` and `rom_from_row` are two lists that have to stay in the
    /// same order, and nothing enforces it: an insertion in the middle shifts
    /// every field after it by one, and where the types happen to line up the
    /// result is a row full of plausible values in the wrong places. That is
    /// not a failure anything reports.
    #[test]
    fn every_column_lands_in_the_field_it_belongs_to() {
        let c = cache("columns");
        add_platform(&c, 1, "snes", "Super Nintendo");
        c.conn
            .execute(
                "INSERT INTO roms(id, platform_slug, name, fs_name, fs_size_bytes, md5_hash, \
                 sha1_hash, cover_path, screenshot_path, summary, manual_path, youtube_id, \
                 multi_file, esde_system, local_path, last_played) \
                 VALUES(7,'snes','Chrono Trigger','ct.sfc',4194304,'themd5','thesha1', \
                 '/c.png','/s.png','A summary.','/m.pdf','vid123',1,'snes','/here/ct.sfc', \
                 '2026-02-03T04:05:06')",
                [],
            )
            .unwrap();

        let r = c.rom_by_id(7).unwrap().expect("the row that was just inserted");
        assert_eq!(r.id, 7);
        assert_eq!(r.platform_slug, "snes");
        assert_eq!(r.name, "Chrono Trigger");
        assert_eq!(r.fs_name, "ct.sfc");
        assert_eq!(r.fs_size_bytes, 4_194_304);
        assert_eq!(r.md5_hash.as_deref(), Some("themd5"));
        assert_eq!(r.sha1_hash.as_deref(), Some("thesha1"));
        assert_eq!(r.cover_path.as_deref(), Some("/c.png"));
        assert_eq!(r.screenshot_path.as_deref(), Some("/s.png"));
        assert_eq!(r.summary.as_deref(), Some("A summary."));
        assert_eq!(r.manual_path.as_deref(), Some("/m.pdf"));
        assert_eq!(r.youtube_id.as_deref(), Some("vid123"));
        assert!(r.multi_file);
        assert_eq!(r.esde_system.as_deref(), Some("snes"));
        assert_eq!(r.local_path.as_deref(), Some("/here/ct.sfc"));
        assert_eq!(r.last_played.as_deref(), Some("2026-02-03T04:05:06"));
    }

    /// "Started twice and bounced off" is a narrower question than "started
    /// once", which is just the library. A game played for an afternoon is not
    /// abandoned however many times it was opened, and one opened once is not
    /// yet a pattern.
    #[test]
    fn abandoned_means_came_back_to_and_still_bounced_off() {
        let c = cache("plays-abandoned");
        add_platform(&c, 1, "snes", "Super Nintendo");
        add_rom(&c, 10, "snes", "Bounced Off", "a.sfc");
        add_rom(&c, 11, "snes", "Played Properly", "b.sfc");
        add_rom(&c, 12, "snes", "Opened Once", "c.sfc");

        for i in 0..3 {
            c.record_play(10, &format!("2026-01-0{}T10:00:00", i + 1), 300).unwrap();
        }
        for i in 0..3 {
            c.record_play(11, &format!("2026-02-0{}T10:00:00", i + 1), 5000).unwrap();
        }
        c.record_play(12, "2026-03-01T10:00:00", 200).unwrap();

        let got = c.abandoned(2, 1800, 10).unwrap();
        let names: Vec<&str> = got.iter().map(|(r, _, _)| r.name.as_str()).collect();
        assert_eq!(names, ["Bounced Off"]);
        assert_eq!(got[0].1, 900);
        assert_eq!(got[0].2, 3);
    }

    /// A row as a server sync leaves it. Marked as the server's, because the
    /// pruning and ownership rules act on that flag and not on the id.
    fn add_rom(c: &Cache, id: i64, slug: &str, name: &str, fs_name: &str) {
        c.conn
            .execute(
                "INSERT INTO roms(id, platform_slug, name, fs_name, fs_size_bytes, from_server)
                 VALUES(?1,?2,?3,?4,0,1)",
                params![id, slug, name, fs_name],
            )
            .unwrap();
    }

    /// The grid must never advertise a platform that opens empty. The count
    /// comes from the roms actually held, not the server's figure, because the
    /// two disagree the moment anything is pruned.
    #[test]
    fn platforms_with_no_roms_are_not_offered() {
        let c = cache("empty-platforms");
        add_platform(&c, 1, "snes", "Super Nintendo");
        add_platform(&c, 2, "dc", "Dreamcast");
        add_rom(&c, 10, "snes", "Chrono Trigger", "ct.sfc");

        let got = c.platforms().unwrap();
        assert_eq!(got.len(), 1, "dreamcast holds nothing and must not appear");
        assert_eq!(got[0].fs_slug, "snes");
        assert_eq!(got[0].rom_count, 1);
    }

    /// Alphabetical by display name, case-insensitively. It was ordered by ROM
    /// count, which put the two biggest systems first and scattered the rest
    /// with no visible logic.
    #[test]
    fn platforms_are_ordered_by_name_regardless_of_case() {
        let c = cache("platform-order");
        for (i, (slug, display)) in
            [("z", "atari"), ("a", "Nintendo"), ("m", "Sega")].iter().enumerate()
        {
            add_platform(&c, i as i64 + 1, slug, display);
            add_rom(&c, i as i64 + 100, slug, "g", "g.bin");
        }
        let names: Vec<String> =
            c.platforms().unwrap().into_iter().map(|p| p.display_name).collect();
        assert_eq!(names, ["atari", "Nintendo", "Sega"], "lowercase must not sort last");
    }

    /// Incremental sync never learns about deletions, so pruning is the only
    /// thing that removes a stale row.
    #[test]
    fn pruning_drops_exactly_what_the_server_no_longer_has() {
        let mut c = cache("prune");
        let [kept, gone, also] = [
            crate::gameid::game_id("snes", "", "kept.sfc"),
            crate::gameid::game_id("snes", "", "gone.sfc"),
            crate::gameid::game_id("snes", "", "gone2.sfc"),
        ];
        add_rom(&c, kept, "snes", "Kept", "kept.sfc");
        add_rom(&c, gone, "snes", "Gone", "gone.sfc");
        add_rom(&c, also, "snes", "Also gone", "gone2.sfc");

        assert_eq!(c.prune_missing(&[kept]).unwrap(), 2);
        assert_eq!(c.rom_count().unwrap(), 1);
        assert!(c.rom_by_id(kept).unwrap().is_some());
    }

    /// The guard that matters most: an empty id list means the server call
    /// failed, not that the server has nothing. Without this, one failed
    /// request would empty the entire library.
    #[test]
    fn pruning_against_an_empty_list_deletes_nothing() {
        let mut c = cache("prune-empty");
        add_rom(&c, 1, "snes", "Kept", "kept.sfc");
        assert_eq!(c.prune_missing(&[]).unwrap(), 0);
        assert_eq!(c.rom_count().unwrap(), 1, "an empty list must never wipe the cache");
    }

    /// Only bare romset names on arcade platforms are replaced. A real title is
    /// left alone, and a same-named file on another platform is not touched.
    #[test]
    fn arcade_renaming_only_touches_bare_romsets_on_arcade_platforms() {
        let mut c = cache("arcade-names");
        add_rom(&c, 1, "arcade", "kof98", "kof98.zip");
        add_rom(&c, 2, "arcade", "Metal Slug", "mslug.zip");
        add_rom(&c, 3, "snes", "kof98", "kof98.zip");

        let names = std::collections::BTreeMap::from([
            ("kof98".to_owned(), "The King of Fighters '98".to_owned()),
            ("mslug".to_owned(), "Metal Slug".to_owned()),
        ]);
        assert_eq!(c.apply_arcade_names(&names).unwrap(), 1);

        assert_eq!(c.rom_by_id(1).unwrap().unwrap().name, "The King of Fighters '98");
        assert_eq!(c.rom_by_id(2).unwrap().unwrap().name, "Metal Slug", "already correct");
        assert_eq!(c.rom_by_id(3).unwrap().unwrap().name, "kof98", "not an arcade platform");
    }

    /// An older cache stores this as TEXT because the migration adds columns
    /// loosely; a fresh one stores an integer. Reading it strictly would make
    /// every folder ROM in an existing cache look single-file, and download it
    /// as an unusable zip.
    #[test]
    fn multi_file_reads_from_both_the_old_and_new_column_types() {
        let c = cache("multifile");
        add_rom(&c, 1, "psx", "Int", "a.chd");
        add_rom(&c, 2, "psx", "Text", "b.chd");
        add_rom(&c, 3, "psx", "Zero", "c.chd");
        c.conn.execute("UPDATE roms SET multi_file = 1 WHERE id = 1", []).unwrap();
        c.conn.execute("UPDATE roms SET multi_file = '1' WHERE id = 2", []).unwrap();
        c.conn.execute("UPDATE roms SET multi_file = '0' WHERE id = 3", []).unwrap();

        assert!(c.rom_by_id(1).unwrap().unwrap().multi_file, "integer 1");
        assert!(c.rom_by_id(2).unwrap().unwrap().multi_file, "text \"1\"");
        assert!(!c.rom_by_id(3).unwrap().unwrap().multi_file, "text \"0\"");
    }

    /// A row with no display name falls back to its filename, or the UI shows
    /// a blank tile that cannot be identified or searched for.
    #[test]
    fn a_nameless_rom_falls_back_to_its_filename() {
        let c = cache("noname");
        add_rom(&c, 1, "snes", "", "Actraiser (USA).sfc");
        assert_eq!(c.rom_by_id(1).unwrap().unwrap().name, "Actraiser (USA).sfc");
    }

    /// Search covers the filename as well as the title, because half this
    /// library is known by one and half by the other.
    #[test]
    fn search_matches_title_or_filename_case_insensitively() {
        let c = cache("search");
        add_rom(&c, 1, "snes", "Chrono Trigger", "ct.sfc");
        add_rom(&c, 2, "arcade", "kof98", "kof98.zip");

        assert_eq!(c.search("chrono", 10).unwrap().len(), 1, "title, wrong case");
        assert_eq!(c.search("ct.sfc", 10).unwrap().len(), 1, "filename");
        assert_eq!(c.search("KOF", 10).unwrap().len(), 1);
        assert_eq!(c.search("nothing here", 10).unwrap().len(), 0);
    }

    /// The newest schema stores every screenshot; older caches stored one. A
    /// cache written before the list existed must still show its screenshot.
    #[test]
    fn screenshots_prefer_the_list_and_fall_back_to_the_single_column() {
        let c = cache("shots");
        add_rom(&c, 1, "snes", "Both", "a.sfc");
        add_rom(&c, 2, "snes", "Legacy", "b.sfc");
        c.conn
            .execute(
                "UPDATE roms SET screenshots_json = '[\"/one.png\",\"/two.png\"]',
                                 screenshot_path = '/old.png' WHERE id = 1",
                [],
            )
            .unwrap();
        c.conn
            .execute("UPDATE roms SET screenshot_path = '/old.png' WHERE id = 2", [])
            .unwrap();

        assert_eq!(
            c.rom_by_id(1).unwrap().unwrap().screenshots(),
            ["/one.png", "/two.png"]
        );
        assert_eq!(c.rom_by_id(2).unwrap().unwrap().screenshots(), ["/old.png"]);
    }

    /// An empty stored list must not shadow the legacy column, or a row that
    /// synced before the list existed shows no artwork at all.
    #[test]
    fn an_empty_screenshot_list_falls_back_rather_than_showing_nothing() {
        let c = cache("shots-empty");
        add_rom(&c, 1, "snes", "Empty list", "a.sfc");
        c.conn
            .execute(
                "UPDATE roms SET screenshots_json = '[]', screenshot_path = '/old.png'
                 WHERE id = 1",
                [],
            )
            .unwrap();
        assert_eq!(c.rom_by_id(1).unwrap().unwrap().screenshots(), ["/old.png"]);
    }

    /// Collections come from the server as JSON, and the two families disagree
    /// on the type of `id` — hand-made ones use a number, virtual ones a base64
    /// string. Both have to land in the same table.
    #[test]
    fn collections_accept_both_numeric_and_string_ids() {
        let numeric: crate::api::Collection =
            serde_json::from_str(r#"{"id": 5, "name": "Favorites", "rom_ids": [1]}"#).unwrap();
        assert_eq!(numeric.id, "5");
        assert_eq!(numeric.group(), "user");

        let virt: crate::api::Collection = serde_json::from_str(
            r#"{"id": "eyJuYW1lIjoiUlBHIn0", "name": "RPG", "is_virtual": true,
                "type": "genre", "rom_ids": [1]}"#,
        )
        .unwrap();
        assert_eq!(virt.id, "eyJuYW1lIjoiUlBHIn0");
        assert_eq!(virt.group(), "genre", "a virtual collection groups by its type");
    }

    /// A collection whose members are all gone would open empty, so it is not
    /// offered — and neither is a group left with nothing in it.
    #[test]
    fn collections_that_would_open_empty_are_hidden() {
        let mut c = cache("collections");
        add_rom(&c, 1, "snes", "Chrono Trigger", "ct.sfc");

        let live: crate::api::Collection =
            serde_json::from_str(r#"{"id": 1, "name": "Live", "rom_ids": [1]}"#).unwrap();
        // Every member of this one was pruned from the cache.
        let dead: crate::api::Collection =
            serde_json::from_str(r#"{"id": 2, "name": "Dead", "rom_ids": [999]}"#).unwrap();
        c.replace_collections(&[live, dead]).unwrap();

        let groups = c.collection_groups().unwrap();
        assert_eq!(groups, [("user".to_owned(), 1)], "only the collection with a live member");

        let items = c.collections_in("user").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Live");
        assert_eq!(items[0].sample_ids, [1], "sample ids drive the cover mosaic");
    }

    /// Replacing is wholesale: virtual collection ids are derived from name and
    /// type, so a rename orphans the old row rather than updating it.
    #[test]
    fn replacing_collections_clears_the_previous_set() {
        let mut c = cache("collections-replace");
        add_rom(&c, 1, "snes", "Game", "g.sfc");

        let first: crate::api::Collection =
            serde_json::from_str(r#"{"id": 1, "name": "Old name", "rom_ids": [1]}"#).unwrap();
        c.replace_collections(&[first]).unwrap();
        let renamed: crate::api::Collection =
            serde_json::from_str(r#"{"id": 2, "name": "New name", "rom_ids": [1]}"#).unwrap();
        c.replace_collections(&[renamed]).unwrap();

        let items = c.collections_in("user").unwrap();
        assert_eq!(items.len(), 1, "the orphaned row must be gone, not accumulated");
        assert_eq!(items[0].name, "New name");
    }

    /// An ES-DE library lives wherever the user put it, so a game's location
    /// cannot be derived from <roms>/<slug>/<file>. The index is asked instead.
    #[test]
    fn a_scanned_games_platform_is_found_by_its_absolute_path() {
        let c = cache("path-lookup");
        add_rom(&c, 1, "genesis", "Sonic", "sonic.md");
        c.conn
            .execute(
                "UPDATE roms SET local_path = '/Volumes/SD/ROMs/megadrive/sonic.md' WHERE id = 1",
                [],
            )
            .unwrap();

        assert_eq!(
            c.platform_for_path(Path::new("/Volumes/SD/ROMs/megadrive/sonic.md")).as_deref(),
            Some("genesis")
        );
        assert_eq!(c.platform_for_path(Path::new("/nowhere/sonic.md")), None);
    }

    /// The exclusion lists govern archive hashing, so they must survive a
    /// restart with the server unreachable — otherwise an offline verify uses
    /// different rules from the download that produced the file.
    #[test]
    fn server_exclusions_round_trip_for_offline_use() {
        let c = cache("server-config");
        assert!(c.server_exclusions().is_none(), "nothing known before the first sync");

        c.save_server_config(&crate::api::ServerConfig {
            default_excluded_files: vec!["custom.nfo".to_owned()],
            default_excluded_extensions: vec!["sav".to_owned()],
            skip_hash_calculation: false,
        })
        .unwrap();

        let (files, exts) = c.server_exclusions().expect("stored");
        assert_eq!(files, ["custom.nfo"]);
        assert_eq!(exts, ["sav"]);
    }

    /// The row of recent games is only useful if it survives a sync. An
    /// incremental pull can return a game with no per-user block at all, and
    /// letting that overwrite the timestamp empties the list every time.
    #[test]
    fn a_sync_without_per_user_data_does_not_forget_when_a_game_was_played() {
        let c = cache("last-played");
        add_platform(&c, 1, "snes", "Super Nintendo");
        add_rom(&c, 10, "snes", "Chrono Trigger", "ct.sfc");
        c.conn
            .execute("UPDATE roms SET last_played = '2026-08-01T10:00:00' WHERE id = 10", [])
            .unwrap();
        assert_eq!(c.recently_played(5).unwrap().len(), 1);

        // What an incremental sync does when the server sends no rom_user.
        c.conn
            .execute(
                "INSERT INTO roms(id, platform_slug, name, fs_name, fs_size_bytes, last_played)
                 VALUES(10,'snes','Chrono Trigger','ct.sfc',0,NULL)
                 ON CONFLICT(id) DO UPDATE SET
                    last_played = COALESCE(excluded.last_played, roms.last_played)",
                [],
            )
            .unwrap();
        assert_eq!(
            c.recently_played(5).unwrap().len(),
            1,
            "the timestamp must survive a sync that did not mention it"
        );
    }

    #[test]
    fn recent_games_come_back_newest_first_and_never_the_unplayed() {
        let c = cache("recent-order");
        add_platform(&c, 1, "snes", "Super Nintendo");
        for (id, name, when) in [
            (1, "Older", Some("2026-01-01T00:00:00")),
            (2, "Newer", Some("2026-08-01T00:00:00")),
            (3, "Never", None),
        ] {
            add_rom(&c, id, "snes", name, &format!("{name}.sfc"));
            if let Some(w) = when {
                c.conn
                    .execute("UPDATE roms SET last_played = ?1 WHERE id = ?2", rusqlite::params![w, id])
                    .unwrap();
            }
        }
        let got = c.recently_played(10).unwrap();
        assert_eq!(got.len(), 2, "a game never played has no place in a recent list");
        assert_eq!(got[0].name, "Newer");
        assert_eq!(got[1].name, "Older");
    }

    #[test]
    fn the_recent_list_honours_its_limit() {
        let c = cache("recent-limit");
        add_platform(&c, 1, "snes", "Super Nintendo");
        for i in 1..=8 {
            add_rom(&c, i, "snes", &format!("Game {i}"), &format!("g{i}.sfc"));
            c.conn
                .execute(
                    "UPDATE roms SET last_played = ?1 WHERE id = ?2",
                    rusqlite::params![format!("2026-08-{:02}T00:00:00", i), i],
                )
                .unwrap();
        }
        assert_eq!(c.recently_played(3).unwrap().len(), 3);
    }
}

#[cfg(test)]
mod hiding {
    use super::*;

    /// A leading dot means hidden, in the app as well as on the card.
    ///
    /// The scan stopped picking these up, but a cache filled before that still
    /// holds them and a synced cache can hold anything — so the rule lives
    /// where every list passes through, and both front ends get it from one
    /// One row, from the same set `roms_for` lists.
    ///
    /// The point of it is what it does *not* do -- build and sort every row of
    /// a console to look at one -- and that cannot be asserted directly. What
    /// can be is that it answers from the same set, so warming picks a real
    /// entry's media folder and not a hidden one's.
    #[test]
    fn any_rom_for_answers_from_the_listed_set() {
        let dir = std::env::temp_dir().join("moose-rack-any-rom");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let mut cache = Cache::open(&dir.join("cache.sqlite3")).unwrap();
        let game = |id: i64, fs_name: &str| crate::esde::Game {
            platform_slug: "snes".into(),
            system: "snes".into(),
            fs_name: fs_name.into(),
            name: fs_name.trim_end_matches(".sfc").into(),
            path: std::path::PathBuf::from(format!("/userdata/roms/snes/{fs_name}")),
            size_bytes: 1000 + id,
            ..Default::default()
        };
        cache
            .replace_from_esde(&[game(1, "ActRaiser (USA).sfc"), game(2, "Chrono Trigger (USA).sfc")])
            .unwrap();

        let one = cache.any_rom_for("snes").unwrap().expect("a row");
        let listed = cache.roms_for("snes").unwrap();
        assert!(
            listed.iter().any(|r| r.id == one.id),
            "any_rom_for returned a row roms_for does not list"
        );
        assert_eq!(one.platform_slug, "snes");
        assert_eq!(cache.any_rom_for("nothing-here").unwrap().map(|r| r.id), None);
    }

    /// place rather than each carrying its own version.
    #[test]
    fn hidden_rows_are_not_listed() {
        let dir = std::env::temp_dir().join("moose-rack-hiding");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let mut cache = Cache::open(&dir.join("cache.sqlite3")).unwrap();

        let game = |id: i64, fs_name: &str| crate::esde::Game {
            platform_slug: "psx".into(),
            system: "psx".into(),
            fs_name: fs_name.into(),
            name: fs_name.trim_end_matches(".chd").into(),
            path: std::path::PathBuf::from(format!("/userdata/roms/psx/{fs_name}")),
            size_bytes: 1000 + id,
            ..Default::default()
        };
        cache
            .replace_from_esde(&[
                game(1, "40 Winks (USA).chd"),
                game(2, ".Final Fantasy VII (USA)"),
            ])
            .unwrap();

        let listed = cache.roms_for("psx").unwrap();
        let names: Vec<&str> = listed.iter().map(|r| r.fs_name.as_str()).collect();
        assert_eq!(names, ["40 Winks (USA).chd"], "a hidden row was listed");

        assert!(shown(&RomRow {
            fs_name: "Chrono Trigger.sfc".into(),
            ..listed[0].clone()
        }));
        assert!(!shown(&RomRow {
            fs_name: ".hidden".into(),
            ..listed[0].clone()
        }));
    }
}

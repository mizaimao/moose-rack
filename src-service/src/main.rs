//! The library service — an ES-DE tree, served over HTTP.
//!
//! This is the thing that replaces RomM. It answers the endpoints `src/api.rs`
//! already calls, so pointing the app at it is a change of `[server] url` and
//! nothing else: no client code moves, and the two can be run side by side and
//! compared.
//!
//! ## What it is not
//!
//! It owns no database of its own. The filesystem is the truth and the index is
//! a cache you can delete: every route below is answered from a scan of the
//! ES-DE tree, so `rm` on the cache costs a rescan and nothing else. That is the
//! first of the five rules in `docs/library-service.md`, and it is the one the
//! others depend on.
//!
//! ## Identity
//!
//! Rows are numbered from the scan order, which is stable for an unchanged tree
//! and is *not* a durable id. RomM's ids were durable and that is precisely what
//! made a rename a migration. Nothing here should be stored against a row id;
//! the content hash is the identity, and `inventory.db` already holds one for
//! every file on the SSD.
//!
//!     moose-service --root /home/frank/moose-library/ES-DE \
//!                   --roms /home/frank/moose-library/ROMs

mod auth;
mod collections;
mod web;
mod saves;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::{
    extract::{Path as AxPath, Query, State},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use clap::Parser;
use moose_rack::{coremap::CoreMap, esde};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeFile;

/// The file form of the flags below.
///
/// Every field is optional and a flag of the same name wins over it, so a config
/// can set the seven paths that never change and a flag can override one for a
/// single run without editing anything.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    library: LibraryPaths,
    #[serde(default)]
    server: ServerCfg,
    /// Who may ask. Absent or empty leaves the service open -- see `auth`.
    #[serde(default)]
    auth: auth::AuthConfig,
}

#[derive(Debug, Default, Deserialize)]
struct LibraryPaths {
    root: Option<String>,
    roms: Option<String>,
    media: Option<String>,
    collections: Option<String>,
    firmware: Option<String>,
    inventory: Option<String>,
    saves: Option<String>,
    ui: Option<String>,
    /// The app's own `config.toml`, for the parts of the UI that are not the
    /// library: theme, cores, achievements. Optional -- without it the defaults
    /// apply and the library still comes from the paths above.
    app_config: Option<String>,
    /// Where EmulatorJS was unpacked. Defaults to `assets/emulatorjs` beside
    /// the `ui` directory, which is where the fetch script puts it.
    emulatorjs: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ServerCfg {
    bind: Option<String>,
    port: Option<u16>,
}

/// Read a config, if there is one.
///
/// Missing is not an error: the flags alone are still a complete way to run
/// this. A *malformed* one is, because silently falling back to defaults would
/// serve the wrong library and look like it worked.
fn load_config(path: &std::path::Path) -> Result<FileConfig> {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

#[derive(Parser)]
#[command(about = "Serve an ES-DE library over the API the app already speaks")]
struct Args {
    /// Config file. Flags below override whatever it sets.
    #[arg(long, env = "MOOSE_SERVICE_CONFIG", default_value = "moose-service.toml")]
    config: String,
    /// ES-DE data directory: the one holding gamelists/ and downloaded_media/
    #[arg(long)]
    root: Option<String>,
    /// ROMs directory, if it is not <root>/ROMs
    #[arg(long)]
    roms: Option<String>,
    /// Artwork directory, if it is not <root>/downloaded_media
    #[arg(long)]
    media: Option<String>,
    /// Address to bind. Use this to restrict the interface as well as the port.
    #[arg(long, env = "MOOSE_SERVICE_BIND", default_value = "0.0.0.0:8001")]
    bind: String,
    /// The app's `ui/` directory. Without it there is no web interface.
    #[arg(long, env = "MOOSE_SERVICE_UI")]
    ui: Option<String>,
    /// Collections directory of .txt lists. Defaults to <root>/collections.
    #[arg(long, env = "MOOSE_SERVICE_COLLECTIONS")]
    collections: Option<String>,
    /// BIOS / firmware directory. Without it /api/firmware is an empty list.
    #[arg(long, env = "MOOSE_SERVICE_FIRMWARE")]
    firmware: Option<String>,
    /// Where saves live. Defaults to <root>/saves.
    #[arg(long, env = "MOOSE_SERVICE_SAVES")]
    saves: Option<String>,
    /// inventory.db, for the hashes. Without it downloads are size-checked only.
    #[arg(long, env = "MOOSE_SERVICE_INVENTORY")]
    inventory: Option<String>,
    /// Port only, overriding whatever `--bind` says. The common case is wanting
    /// a different port on the same interface, and rewriting the whole address
    /// to do that is a good way to bind to localhost by accident.
    #[arg(short, long, env = "MOOSE_SERVICE_PORT")]
    port: Option<u16>,
    /// Hash a password for `[[auth.users]]`, print it, and exit. The plain
    /// password is never stored anywhere; paste the printed line into the config.
    #[arg(long, value_name = "PASSWORD")]
    hash_password: Option<String>,
}

/// The scan, held for the process lifetime.
///
/// Rescanning is cheap — 11,473 games in about three seconds — but not free, and
/// nothing here mutates it yet. When writes arrive this becomes a lock rather
/// than an Arc.
struct Library {
    games: Vec<esde::Game>,
    /// The curated lists, resolved to ids at startup.
    collections: Vec<collections::Collection>,
    /// The BIOS set, flattened.
    ///
    /// Flattened on purpose: the tree has `mame/`, `fbneo/` and so on, but
    /// RetroArch wants every BIOS in one system directory, so the sub-path is
    /// reported and not reproduced. A wrong BIOS breaks emulation silently,
    /// which is why the hashes travel with the listing.
    firmware: Vec<Firmware>,
    /// Saves, and the per-device bookkeeping that makes a conflict detectable.
    ///
    /// A Mutex rather than an RwLock: writes are rare and a save upload must
    /// not interleave with the negotiate that decided to send it.
    sync: std::sync::Mutex<SyncState>,
    /// `(system, relative path)` -> `(md5, sha1, crc32)`, out of inventory.db.
    ///
    /// The client already knows how to verify a download — `verify()` in
    /// download.rs hashes what it got and compares. It falls back to a size
    /// check only when the server publishes nothing, which is what made the
    /// first transfers here unverified. The hashes were computed once over
    /// 1.76 TB; not serving them was the whole gap.
    hashes: std::collections::HashMap<(String, String), (Option<String>, Option<String>, Option<String>)>,
}

/// Device registrations and what each last agreed with the server.
///
/// Persisted as JSON beside the saves. Losing it is not fatal — every save
/// becomes a conflict on the next differing sync, which is the safe direction to
/// fail in.
#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    devices: std::collections::HashMap<String, String>,
    /// `device\0rom_id\0file` -> hash last agreed. Flattened because JSON keys
    /// cannot be tuples.
    seen: std::collections::HashMap<String, String>,
}

struct SyncState {
    store: saves::SaveStore,
    /// Save *states* -- emulator freeze-frames. A separate store because they
    /// sync by a different rule: `crate::statesync` decides what to send before
    /// it sends, so there is no conflict to answer and nothing to merge.
    states: saves::StateStore,
    path: std::path::PathBuf,
    data: Persisted,
}

impl SyncState {
    fn seen_map(&self) -> saves::Seen {
        self.data
            .seen
            .iter()
            .filter_map(|(k, v)| {
                let mut it = k.split('\0');
                let d = it.next()?.to_owned();
                let r: i64 = it.next()?.parse().ok()?;
                Some(((d, r, it.next()?.to_owned()), v.clone()))
            })
            .collect()
    }

    fn agree(&mut self, device: &str, rom_id: i64, file: &str, hash: &str) {
        self.data
            .seen
            .insert(format!("{device}\0{rom_id}\0{file}"), hash.to_owned());
        self.flush();
    }

    fn flush(&self) {
        if let Ok(j) = serde_json::to_vec_pretty(&self.data) {
            let _ = std::fs::write(&self.path, j);
        }
    }
}

#[derive(Serialize, Clone)]
struct Firmware {
    id: i64,
    file_name: String,
    file_size_bytes: i64,
    md5_hash: Option<String>,
    sha1_hash: Option<String>,
    /// Where it sits under the firmware root, e.g. `mame`. Reported so a
    /// duplicate name is explicable, not so it can be recreated.
    file_path: Option<String>,
    /// True only when a hash was actually computed. Claiming verification
    /// without one would be worse than admitting none.
    is_verified: bool,
    #[serde(skip)]
    abs: std::path::PathBuf,
}

/// Walk a BIOS tree into a flat list, hashing as it goes.
///
/// Hashed once at startup rather than per request: the set is ~170 files and a
/// few GB, and a BIOS that silently differs is the failure this exists to catch.
fn scan_firmware(root: &std::path::Path) -> Vec<Firmware> {
    use md5::Digest as _;
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
            if name.starts_with('.') {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            let md5 = hex::encode(md5::Md5::digest(&bytes));
            let sha1 = hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes));
            let rel = p
                .parent()
                .and_then(|par| par.strip_prefix(root).ok())
                .map(|r| r.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty());
            out.push(Firmware {
                id: saves::save_id(0, &p.to_string_lossy()),
                file_name: name.to_owned(),
                file_size_bytes: bytes.len() as i64,
                md5_hash: Some(md5),
                sha1_hash: Some(sha1),
                file_path: rel,
                is_verified: true,
                abs: p,
            });
        }
    }
    out.sort_by(|a, b| (&a.file_path, &a.file_name).cmp(&(&b.file_path, &b.file_name)));
    out
}

async fn firmware(State(lib): State<Arc<Library>>) -> Json<Vec<Firmware>> {
    Json(lib.firmware.clone())
}

async fn firmware_content(
    State(lib): State<Arc<Library>>,
    AxPath((id, _name)): AxPath<(i64, String)>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let Some(f) = lib.firmware.iter().find(|f| f.id == id) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    match tower::ServiceExt::oneshot(ServeFile::new(&f.abs), req).await {
        Ok(r) => r.into_response(),
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Serialize)]
struct HeartbeatSystem {
    #[serde(rename = "VERSION")]
    version: String,
}

#[derive(Serialize)]
struct Heartbeat {
    #[serde(rename = "SYSTEM")]
    system: HeartbeatSystem,
}

#[derive(Serialize)]
struct ServerConfig {
    #[serde(rename = "DEFAULT_EXCLUDED_FILES")]
    files: Vec<String>,
    #[serde(rename = "DEFAULT_EXCLUDED_EXTENSIONS")]
    exts: Vec<String>,
    #[serde(rename = "SKIP_HASH_CALCULATION")]
    skip_hash: bool,
}

#[derive(Serialize)]
struct User {
    id: i64,
    username: String,
    role: String,
}

#[derive(Serialize)]
struct Platform {
    id: i64,
    fs_slug: String,
    slug: String,
    name: Option<String>,
    rom_count: i64,
}

#[derive(Serialize)]
struct Rom {
    id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    md5_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha1_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crc_hash: Option<String>,
    name: Option<String>,
    fs_name: String,
    missing_from_fs: bool,
    fs_size_bytes: Option<i64>,
    platform_fs_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Serialize)]
struct RomPage {
    items: Vec<Rom>,
    total: i64,
}

#[derive(Deserialize)]
struct Page {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

fn default_limit() -> usize {
    50
}

/// The scan index is the id, one-based.
///
/// One-based because zero is what a missing field deserializes to, and a game
/// that silently becomes "game 0" is the kind of bug that takes an evening.
fn to_rom(i: usize, g: &esde::Game, lib: &Library) -> Rom {
    // The dump's hash, not the container's: the client hashes the file it just
    // wrote, which is the zip, so the container hash is the one that compares.
    let key = (g.system.clone(), rel_of(g));
    let (md5, sha1, crc) = lib.hashes.get(&key).cloned().unwrap_or((None, None, None));
    Rom {
        id: i as i64 + 1,
        md5_hash: md5,
        sha1_hash: sha1,
        crc_hash: crc,
        name: Some(g.name.clone()),
        fs_name: g.fs_name.clone(),
        // The scan only reports files it found, so anything listed exists. RomM
        // needed this flag because its rows outlived their files.
        missing_from_fs: false,
        fs_size_bytes: Some(g.size_bytes),
        platform_fs_slug: Some(g.platform_slug.clone()),
        summary: g.summary.clone(),
    }
}

/// The game's path relative to its system directory, which is how inventory.db
/// keys its rows.
fn rel_of(g: &esde::Game) -> String {
    if g.rel_dir.is_empty() {
        g.fs_name.clone()
    } else {
        format!("{}/{}", g.rel_dir, g.fs_name)
    }
}

/// Load `(system, path) -> hashes` out of inventory.db.
///
/// Container hashes, because the client hashes the file it wrote. Missing rows
/// are not an error: a game the inventory has not seen simply gets no hash and
/// falls back to a size check, which is what happens today for everything.
fn load_hashes(
    path: &str,
) -> Result<std::collections::HashMap<(String, String), (Option<String>, Option<String>, Option<String>)>>
{
    let conn = rusqlite::Connection::open(path)?;
    let mut out = std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT system, path, container_md5, container_sha1, container_crc32 \
         FROM files WHERE status = 'ok'",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            (r.get::<_, String>(0)?, r.get::<_, String>(1)?),
            (r.get(2)?, r.get(3)?, r.get(4)?),
        ))
    })?;
    for row in rows {
        let (k, v) = row?;
        out.insert(k, v);
    }
    Ok(out)
}

/// A page for a person who typed the address into a browser.
///
/// There was nothing here and `/` answered 404, which reads as "the server is
/// down" rather than "you asked for a path that does not exist". Everything on
/// it is already public through the API; it is a signpost, not an interface.
async fn index(State(lib): State<Arc<Library>>) -> axum::response::Html<String> {
    let platforms = {
        let mut m: std::collections::BTreeMap<&str, usize> = Default::default();
        for g in &lib.games {
            *m.entry(g.platform_slug.as_str()).or_default() += 1;
        }
        m
    };
    let saves = lib.sync.lock().map(|s| s.store.list(None).len()).unwrap_or(0);
    let cols: i64 = lib.collections.iter().map(|c| c.rom_count).sum();
    let rows = platforms
        .iter()
        .map(|(p, n)| format!("<tr><td>{p}</td><td align=right>{n}</td></tr>"))
        .collect::<String>();
    let tmpl = r#"<!doctype html><meta charset=utf-8><title>Moose Rack</title>
<style>body{font:14px/1.5 system-ui;margin:3rem auto;max-width:40rem;padding:0 1rem}
td{padding:.1rem .8rem .1rem 0}code{background:#eee;padding:.1rem .3rem}</style>
<h1>Moose Rack</h1>
<p>Library service {v}. This is the API host; the app talks to it.</p>
<p><b>{games}</b> games &middot; <b>{plat}</b> platforms &middot; <b>{fw}</b> firmware files
&middot; <b>{cn}</b> collections ({cm} memberships) &middot; <b>{saves}</b> saves</p>
<table>{rows}</table>
<p>Point the app's <code>[server] url</code> here. Routes live under <code>/api/</code>:
<a href="/api/heartbeat">heartbeat</a>,
<a href="/api/platforms">platforms</a>,
<a href="/api/collections">collections</a>.</p>
"#;
    axum::response::Html(
        tmpl.replace("{v}", env!("CARGO_PKG_VERSION"))
            .replace("{games}", &lib.games.len().to_string())
            .replace("{plat}", &platforms.len().to_string())
            .replace("{fw}", &lib.firmware.len().to_string())
            .replace("{cn}", &lib.collections.len().to_string())
            .replace("{cm}", &cols.to_string())
            .replace("{saves}", &saves.to_string())
            .replace("{rows}", &rows),
    )
}

async fn heartbeat() -> Json<Heartbeat> {
    Json(Heartbeat {
        system: HeartbeatSystem {
            // The client compares this against the release it was verified
            // against and warns on a mismatch. Ours is the crate version.
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
    })
}

async fn config(State(lib): State<Arc<Library>>) -> Json<ServerConfig> {
    Json(ServerConfig {
        // Nothing is excluded here: the tree is the library, and a file that
        // should not be in it should not be on disk. RomM needed these because
        // it scanned directories it did not own.
        files: vec![],
        exts: vec![],
        // True only when no inventory was loaded. The client reads this as
        // "size checks are all you get" and says so out loud, which is the
        // right thing for it to do -- but it should only hear it when it is
        // true, or a corrupt transfer goes unnoticed.
        skip_hash: lib.hashes.is_empty(),
    })
}

async fn users_me() -> Json<User> {
    Json(User {
        id: 1,
        username: "local".to_owned(),
        role: "admin".to_owned(),
    })
}

async fn platforms(State(lib): State<Arc<Library>>) -> Json<Vec<Platform>> {
    let mut seen: std::collections::BTreeMap<&str, (i64, &str)> = Default::default();
    for g in &lib.games {
        let e = seen.entry(g.platform_slug.as_str()).or_insert((0, g.system.as_str()));
        e.0 += 1;
    }
    Json(
        seen.iter()
            .enumerate()
            .map(|(i, (slug, (count, system)))| Platform {
                id: i as i64 + 1,
                fs_slug: (*slug).to_owned(),
                slug: (*slug).to_owned(),
                name: Some((*system).to_owned()),
                rom_count: *count,
            })
            .collect(),
    )
}

async fn roms(State(lib): State<Arc<Library>>, Query(p): Query<Page>) -> Json<RomPage> {
    let items = lib
        .games
        .iter()
        .enumerate()
        .skip(p.offset)
        .take(p.limit)
        .map(|(i, g)| to_rom(i, g, &lib))
        .collect();
    Json(RomPage {
        items,
        total: lib.games.len() as i64,
    })
}

/// Every id the service currently has.
///
/// The client's only way to notice a deletion: `updated_after` reports changes
/// and never removals. Cheap — one array of ints.
///
/// Its absence is not a 404. Without this route `/api/roms/{id}` matches, the
/// literal "identifiers" goes to the i64 extractor and the answer is 400 --
/// which is what a first sync against this service got, and why the client
/// silently skipped pruning. Order does not matter: axum prefers a static
/// segment to a dynamic one however they are registered. The test asserts the
/// route exists, having been checked to fail when it is removed.
async fn rom_identifiers(State(lib): State<Arc<Library>>) -> Json<Vec<i64>> {
    Json((1..=lib.games.len() as i64).collect())
}

/// The curated lists.
///
/// Still empty rather than 404 when there is no directory: the client reads a
/// missing endpoint as an error and an empty list as "none yet".
/// RomM generated collections by genre, franchise and so on -- 1,931 of them
/// against 27 hand-made ones. There is no equivalent here and there should not
/// be: the point of the text files is that a list is something a person decided.
///
/// Empty rather than absent because the GUI asks for all three on every load and
/// wraps each in `unwrap_or_default`. A 404 is survivable but logs an error
/// forever; an empty list is the honest answer.
async fn no_collections() -> Json<Vec<serde_json::Value>> {
    Json(vec![])
}

async fn no_kinds() -> Json<Vec<String>> {
    Json(vec![])
}

async fn serve_collections(State(lib): State<Arc<Library>>) -> axum::response::Response {
    Json(&lib.collections).into_response()
}

/// The bytes of a game.
///
/// `GET /api/roms/{id}/content/{name}` — the name is in the path because that
/// is the shape the client already builds, and it is ignored here: the id
/// decides which file is served. Checking it would only let a stale client name
/// turn into a 404 for a file that is present.
///
/// Range is handled by `ServeFile`, which matters more than it looks. The
/// client resumes a part-file by sending `Range: bytes=N-` and treats 200 as
/// "the server ignored me, throw the partial away". Getting this wrong does not
/// fail, it silently re-downloads gigabytes.
async fn rom_content(
    State(lib): State<Arc<Library>>,
    AxPath((id, _name)): AxPath<(i64, String)>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let Ok(idx) = usize::try_from(id - 1) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Some(game) = lib.games.get(idx) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    match tower::ServiceExt::oneshot(ServeFile::new(&game.path), req).await {
        Ok(r) => r.into_response(),
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
struct DeviceReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
}

/// Register a device, or hand back the one already registered under this name.
///
/// `allow_existing` is why the client sends a name at all: minting a fresh id
/// every call would give each one empty bookkeeping, and every save would then
/// look like a first-time upload.
async fn register_device(
    State(lib): State<Arc<Library>>,
    Json(req): Json<DeviceReq>,
) -> Json<serde_json::Value> {
    let name = req.name.or(req.hostname).unwrap_or_else(|| "unnamed".into());
    let mut st = lib.sync.lock().unwrap();
    let id = st
        .data
        .devices
        .iter()
        .find(|(_, n)| **n == name)
        .map(|(i, _)| i.clone())
        .unwrap_or_else(|| {
            let id = format!("{:016x}", saves::save_id(0, &name));
            st.data.devices.insert(id.clone(), name.clone());
            st.flush();
            id
        });
    Json(serde_json::json!({ "id": id, "name": name }))
}

#[derive(Deserialize)]
struct SavesQuery {
    #[serde(default)]
    rom_id: Option<i64>,
}

async fn list_saves(
    State(lib): State<Arc<Library>>,
    Query(q): Query<SavesQuery>,
) -> Json<Vec<saves::ServerSave>> {
    Json(lib.sync.lock().unwrap().store.list(q.rom_id))
}

#[derive(Deserialize)]
struct NegotiateReq {
    device_id: String,
    #[serde(default)]
    saves: Vec<saves::ClientSaveState>,
}

async fn negotiate(
    State(lib): State<Arc<Library>>,
    Json(req): Json<NegotiateReq>,
) -> Json<saves::SyncPlan> {
    let st = lib.sync.lock().unwrap();
    let server = st.store.list(None);
    let mut plan = saves::plan(&req.device_id, &req.saves, &server, &st.seen_map());
    // A session id the client can quote back. Nothing is reserved by it -- it
    // exists so `complete_session` has something to close.
    plan.session_id = Some(1);
    Json(plan)
}

async fn save_content(
    State(lib): State<Arc<Library>>,
    AxPath(id): AxPath<i64>,
) -> axum::response::Response {
    match lib.sync.lock().unwrap().store.read(id) {
        Some(b) => ([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], b).into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct StatesQuery {
    rom_id: Option<i64>,
}

async fn list_states(
    State(lib): State<Arc<Library>>,
    Query(q): Query<StatesQuery>,
) -> Json<Vec<saves::ServerState>> {
    Json(lib.sync.lock().unwrap().states.list(q.rom_id))
}

async fn state_content(
    State(lib): State<Arc<Library>>,
    AxPath(id): AxPath<i64>,
) -> axum::response::Response {
    match lib.sync.lock().unwrap().states.read(id) {
        Some(b) => {
            ([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], b).into_response()
        }
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct StateUploadQuery {
    rom_id: i64,
    #[serde(default)]
    emulator: Option<String>,
}

/// Take the state we are given.
///
/// No overwrite flag and no 409, unlike `/api/saves`: `src/api.rs` says so in
/// as many words, and the reason is that a freeze-frame belongs to one emulator
/// build and cannot be merged with another. The decision not to send is made
/// before the call.
async fn upload_state(
    State(lib): State<Arc<Library>>,
    Query(q): Query<StateUploadQuery>,
    mut form: axum::extract::Multipart,
) -> axum::response::Response {
    let mut file_name = String::new();
    let mut bytes: Vec<u8> = Vec::new();
    while let Ok(Some(field)) = form.next_field().await {
        // `stateFile`, the name `upload_state` sends. Anything else is ignored
        // rather than guessed at.
        if field.name() == Some("stateFile") {
            file_name = field.file_name().unwrap_or_default().to_owned();
            bytes = field.bytes().await.unwrap_or_default().to_vec();
        }
    }
    // A path separator in the name would write outside the store.
    let safe = std::path::Path::new(&file_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    if safe.is_empty() || safe.starts_with('.') {
        return (axum::http::StatusCode::BAD_REQUEST, "no usable stateFile").into_response();
    }
    let store = lib.sync.lock().unwrap();
    match store.states.write(q.rom_id, &safe, q.emulator.as_deref(), &bytes) {
        Ok(st) => Json(st).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct UploadQuery {
    rom_id: i64,
    device_id: String,
    #[serde(default)]
    overwrite: Option<bool>,
}

/// Accept a save, unless the server's copy moved since this device last agreed.
///
/// Refusing is the point: 409 is how the client discovers a conflict at all.
/// `overwrite=true` is not a retry, it is the user having been shown the
/// conflict and chosen.
async fn upload_save(
    State(lib): State<Arc<Library>>,
    Query(q): Query<UploadQuery>,
    mut form: axum::extract::Multipart,
) -> axum::response::Response {
    let mut name = String::new();
    let mut bytes = Vec::new();
    while let Ok(Some(field)) = form.next_field().await {
        if field.name() == Some("saveFile") {
            name = field.file_name().unwrap_or("save.srm").to_owned();
            bytes = field.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        }
    }
    if name.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "no saveFile part").into_response();
    }

    let mut st = lib.sync.lock().unwrap();
    let existing = st.store.list(Some(q.rom_id)).into_iter().find(|s| s.file_name == name);
    if !q.overwrite.unwrap_or(false) {
        if let Some(cur) = &existing {
            let agreed = st
                .data
                .seen
                .get(&format!("{}\0{}\0{}", q.device_id, q.rom_id, name))
                .cloned();
            if agreed.as_deref() != cur.content_hash.as_deref() {
                return (
                    axum::http::StatusCode::CONFLICT,
                    format!(
                        "the server copy of {name} changed since this device last agreed \
                         (server {}, last agreed {})",
                        cur.content_hash.clone().unwrap_or_default(),
                        agreed.unwrap_or_else(|| "never".into())
                    ),
                )
                    .into_response();
            }
        }
    }

    match st.store.write(q.rom_id, &name, &bytes) {
        Ok(s) => {
            let h = s.content_hash.clone().unwrap_or_default();
            st.agree(&q.device_id, q.rom_id, &name, &h);
            Json(s).into_response()
        }
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn complete_session() -> axum::http::StatusCode {
    axum::http::StatusCode::OK
}

async fn rom_by_id(
    State(lib): State<Arc<Library>>,
    AxPath(id): AxPath<i64>,
) -> Result<Json<Rom>, axum::http::StatusCode> {
    let idx = usize::try_from(id - 1).map_err(|_| axum::http::StatusCode::NOT_FOUND)?;
    lib.games
        .get(idx)
        .map(|g| Json(to_rom(idx, g, &lib)))
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}

/// What the whole service is protected by.
#[derive(Default)]
pub struct Guard {
    pub cfg: auth::AuthConfig,
    pub sessions: auth::Sessions,
}

/// Routes that answer before anybody has proved who they are.
///
/// `heartbeat` so "server down" and "wrong credentials" can be told apart --
/// without it a bad token looks exactly like an unplugged machine. `login` for
/// obvious reasons, and `__shim.js` because the login page is served by the
/// same document machinery as the app and would otherwise fail to script.
fn is_public(path: &str) -> bool {
    matches!(path, "/api/heartbeat" | "/login" | "/logout" | "/__shim.js")
}

/// Identify the caller, or refuse.
///
/// The identity is put in the request's extensions so a handler can gate on it
/// -- `/invoke/` is one route serving eighty commands and the split between
/// them is not something a router can express.
async fn require_auth(
    State(guard): State<std::sync::Arc<Guard>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if guard.cfg.open() || is_public(req.uri().path()) {
        req.extensions_mut().insert(auth::Identity::Owner);
        return next.run(req).await;
    }
    let header = req.headers().get(axum::http::header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let cookie = req.headers().get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok());
    let who = auth::from_header(&guard.cfg, header).or_else(|| {
        auth::session_from_cookies(cookie).and_then(|id| guard.sessions.get(id))
    });
    let Some(who) = who else {
        // A browser gets the login page rather than the platform box: a `Basic`
        // challenge cannot carry a token, which is how the owner signs in.
        let wants_html = req
            .headers()
            .get(axum::http::header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("text/html"));
        if wants_html && !req.uri().path().starts_with("/api/") {
            return axum::response::Redirect::to("/login").into_response();
        }
        return (axum::http::StatusCode::UNAUTHORIZED, "who are you?").into_response();
    };
    // `/invoke/` is one route serving eighty commands, and the line between
    // reading the library and changing it runs between the commands rather
    // than between the routes. Decided here rather than in the handler: this
    // runs before the router resolves anything, so the rule holds however the
    // handler is later rewritten, and it can be tested against any router.
    if !who.is_owner() {
        if let Some(cmd) = req.uri().path().strip_prefix("/invoke/") {
            if auth::owner_only(cmd) {
                return (
                    axum::http::StatusCode::FORBIDDEN,
                    format!("{cmd} is the owner's to do"),
                )
                    .into_response();
            }
        }
    }
    req.extensions_mut().insert(who);
    next.run(req).await
}

/// Swap a token or a username and password for a session cookie.
async fn login(
    State(guard): State<std::sync::Arc<Guard>>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let (token, user, pass) = (s("token"), s("username"), s("password"));
    // The shared account, when the owner has switched it on. No password to
    // check because there is none: the button is the credential, and the whole
    // point is that everyone using it is the same reader.
    if body.get("guest").and_then(|v| v.as_bool()) == Some(true) {
        if !guard.cfg.guest {
            return (axum::http::StatusCode::UNAUTHORIZED, "no").into_response();
        }
        let who = auth::Identity::User(auth::AuthConfig::GUEST.to_owned());
        let id = guard.sessions.open(who);
        let cookie =
            format!("{}={id}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000", auth::COOKIE);
        return (
            [(axum::http::header::SET_COOKIE, cookie)],
            Json(serde_json::json!({ "ok": true, "owner": false, "name": auth::AuthConfig::GUEST })),
        )
            .into_response();
    }
    let header = if !token.is_empty() {
        format!("Bearer {token}")
    } else {
        format!(
            "Basic {}",
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                format!("{user}:{pass}")
            )
        )
    };
    let Some(who) = auth::from_header(&guard.cfg, Some(&header)) else {
        // One message for a bad token and a bad password, so neither says
        // which of the two exists.
        return (axum::http::StatusCode::UNAUTHORIZED, "no").into_response();
    };
    let owner = who.is_owner();
    let name = who.name().to_owned();
    let id = guard.sessions.open(who);
    // HttpOnly so a script on the page cannot read it; SameSite=Lax so it is
    // not sent from another site. Not Secure: this is plain HTTP on a LAN, and
    // a Secure cookie would simply never be stored.
    let cookie = format!("{}={id}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000", auth::COOKIE);
    (
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true, "owner": owner, "name": name })),
    )
        .into_response()
}

async fn logout(
    State(guard): State<std::sync::Arc<Guard>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    if let Some(id) =
        auth::session_from_cookies(headers.get(axum::http::header::COOKIE).and_then(|v| v.to_str().ok()))
    {
        guard.sessions.close(id);
    }
    let cookie = format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0", auth::COOKIE);
    ([(axum::http::header::SET_COOKIE, cookie)], axum::response::Redirect::to("/login"))
        .into_response()
}

/// The sign-in page.
///
/// Deliberately not part of `ui/`: it has to render before anything is
/// authorised, and the app's own modules all call `invoke` on the way up.
const LOGIN_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Moose Rack</title>
<style>
 :root { color-scheme: dark }
 body { margin:0; min-height:100vh; display:grid; place-items:center;
        background:#14161a; color:#e8eaed;
        font:14px/1.5 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif }
 form { width:min(92vw,320px); display:grid; gap:10px }
 h1 { font-size:17px; margin:0 0 6px; font-weight:600 }
 p.hint { margin:0 0 10px; color:#9aa0a6; font-size:12px }
 input { padding:9px 11px; border-radius:7px; border:1px solid #2a2e35;
         background:#1b1e24; color:inherit; font:inherit; width:100% ; box-sizing:border-box }
 input:focus { outline:2px solid #4c8dff; outline-offset:1px; border-color:transparent }
 button { padding:9px 11px; border-radius:7px; border:0; background:#4c8dff; color:#fff;
          font:inherit; font-weight:600; cursor:pointer }
 button:disabled { opacity:.6; cursor:default }
 button.ghost { background:transparent; border:1px solid #2a2e35; color:#c8ccd2; font-weight:500 }
 button.ghost:hover { border-color:#3a4049; color:#e8eaed }
 .sep { display:flex; align-items:center; gap:10px; color:#666; font-size:11px;
        text-transform:uppercase; letter-spacing:.08em }
 .sep::before,.sep::after { content:""; flex:1; height:1px; background:#2a2e35 }
 .err { color:#ff8a80; font-size:12px; min-height:1.4em }
</style>
<form id="f">
  <h1>Moose Rack</h1>
  <p class="hint">Sign in with your token, or with a username and password.</p>
  <input name="token" type="password" placeholder="Token" autocomplete="off">
  <div class="sep">or</div>
  <input name="username" placeholder="Username" autocomplete="username">
  <input name="password" type="password" placeholder="Password" autocomplete="current-password">
  <button>Sign in</button>
  <!--GUEST-->
  <div class="err" id="e"></div>
</form>
<script>
document.getElementById("f").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const f = ev.target, b = f.querySelector("button"), e = document.getElementById("e");
  b.disabled = true; e.textContent = "";
  const body = Object.fromEntries(new FormData(f));
  const r = await fetch("/login", {
    method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
  });
  if (r.ok) { location.href = "/"; return; }
  b.disabled = false;
  e.textContent = "Not recognised.";
});
const g = document.getElementById("g");
if (g) g.addEventListener("click", async () => {
  g.disabled = true;
  const r = await fetch("/login", {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ guest: true }),
  });
  if (r.ok) { location.href = "/"; return; }
  g.disabled = false;
  document.getElementById("e").textContent = "Guest access is off.";
});
</script>
"#;

async fn login_page(State(guard): State<std::sync::Arc<Guard>>) -> axum::response::Html<String> {
    // Drawn only when it will work. A button that answers 401 is worse than no
    // button: it reads as the service being broken rather than as a door that
    // was never opened.
    let guest = if guard.cfg.guest {
        r#"<div class="sep">or</div>
  <button type="button" id="g" class="ghost">Continue as guest</button>"#
    } else {
        ""
    };
    axum::response::Html(LOGIN_PAGE.replace("<!--GUEST-->", guest))
}

/// The routes, as a function so tests can build one without a socket.
/// The web UI, if a `ui/` directory was given.
///
/// A separate Router with its own state, merged in: the API answers `src/api.rs`
/// and this answers the app's own IPC, and conflating them would make each
/// harder to read.
fn web_app(st: Arc<web::WebState>) -> Router {
    let dir = st.ui_dir.clone();
    Router::new()
        .route("/", get(web::index))
        .route("/__shim.js", get(web::shim))
        .route("/invoke/{cmd}", axum::routing::post(web::invoke))
        .nest_service("/js", tower_http::services::ServeDir::new(dir.join("js")))
        .nest_service("/icons", tower_http::services::ServeDir::new(dir.join("icons")))
        .route_service("/style.css", tower_http::services::ServeFile::new(dir.join("style.css")))
        .route_service("/settings.css", tower_http::services::ServeFile::new(dir.join("settings.css")))
        // Not `ServeFile`: the settings page loads the same modules and needs
        // the shim too, so it goes out rewritten like `index.html`.
        .route("/settings.html", get(web::settings))
        // Artwork by absolute path, which is the shape `convertFileSrc` hands
        // the page. Confined to the media roots -- see `resolve_media`.
        .route("/media", get(web::media))
        // EmulatorJS, vendored by scripts/fetch-emulatorjs.sh. Served from here
        // rather than referenced on a CDN: this is a LAN library and it has to
        // play with the internet down. Absent when nobody has run the script,
        // and the page says so rather than failing blankly.
        .nest_service(
            "/emulatorjs",
            tower_http::services::ServeDir::new(st.emulatorjs.clone()),
        )
        .with_state(st)
}

fn app(lib: Arc<Library>, media_dir: std::path::PathBuf, with_index: bool) -> Router {
        let base = Router::new();
        // The status page is a fallback. When a UI is served, `/` is the app --
        // somebody typing the address wants the thing, not a summary of it.
        let base = if with_index { base.route("/", get(index)) } else { base };
        base
            .route("/api/heartbeat", get(heartbeat))
            .route("/api/config", get(config))
            .route("/api/users/me", get(users_me))
            .route("/api/platforms", get(platforms))
            .route("/api/roms", get(roms))
            .route("/api/roms/identifiers", get(rom_identifiers))
            .route("/api/roms/{id}", get(rom_by_id))
            .route("/api/roms/{id}/content/{*name}", get(rom_content))
            .route("/api/collections", get(serve_collections))
            .route("/api/collections/smart", get(no_collections))
            .route("/api/collections/virtual", get(no_collections))
            .route("/api/collections/virtual/identifiers", get(no_kinds))
        .route("/api/firmware", get(firmware))
        .route("/api/firmware/{id}/content/{*name}", get(firmware_content))
        .route("/api/devices", axum::routing::post(register_device))
        .route("/api/saves", get(list_saves).post(upload_save))
        .route("/api/saves/{id}/content", get(save_content))
        .route("/api/states", get(list_states).post(upload_state))
        .route("/api/states/{id}/content", get(state_content))
        .route("/api/sync/negotiate", axum::routing::post(negotiate))
        .route("/api/sync/sessions/{id}/complete", axum::routing::post(complete_session))
            // Artwork straight off the tree. ES-DE and Skraper already scraped it;
            // re-serving it through a database would gain nothing.
            //
            // Two mounts for one directory. `media.rs` builds artwork URLs itself
            // from a hardcoded `/assets/romm/resources/esde-media`, so serving that
            // path is what makes an unmodified client show covers at all. The
            // neutral mount is what the client should use once that constant is
            // retired, and having both means that can happen without a flag day.
            .nest_service(
                "/assets/romm/resources/esde-media",
                tower_http::services::ServeDir::new(media_dir.clone()),
            )
            .nest_service("/assets/media", tower_http::services::ServeDir::new(media_dir))
            .with_state(lib)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Before the library is even looked at: this is a calculator, not a server.
    if let Some(plain) = args.hash_password.as_deref() {
        println!("{}", auth::hash_password(plain));
        return Ok(());
    }
    let cfg = load_config(std::path::Path::new(&args.config))?;
    // A flag beats the file; the file beats nothing. `or` reads in that order.
    let pick = |flag: Option<String>, file: &Option<String>| flag.or_else(|| file.clone());
    let root = pick(args.root.clone(), &cfg.library.root).context(
        "no --root and no [library] root in the config: nothing to serve",
    )?;
    let roms = pick(args.roms.clone(), &cfg.library.roms);
    let media = pick(args.media.clone(), &cfg.library.media);
    if std::path::Path::new(&args.config).exists() {
        println!("config     {}", args.config);
    }
    let layout = esde::Layout::new(
        std::path::Path::new(&root),
        roms.as_deref().map(std::path::Path::new),
    )
    .with_media(media.as_deref().map(std::path::Path::new));

    println!("roms       {}", layout.roms.display());
    println!("gamelists  {}", layout.gamelists.display());
    println!("media      {}", layout.media.display());

    let map = CoreMap::embedded();
    let started = std::time::Instant::now();
    let (games, skipped) = esde::scan(&layout, &map)?;
    println!(
        "scanned    {} games in {:.1}s",
        games.len(),
        started.elapsed().as_secs_f64()
    );
    if !skipped.is_empty() {
        println!("skipped    {}", skipped.join(", "));
    }

    let media_dir = layout.media.clone();
    let hashes = match pick(args.inventory.clone(), &cfg.library.inventory).as_deref() {
        Some(p) => match load_hashes(p) {
            Ok(h) => {
                println!("hashes     {} rows from {p}", h.len());
                h
            }
            // Not fatal: the library still serves, downloads just fall back to
            // a size check. Saying so beats refusing to start.
            Err(e) => {
                eprintln!("hashes     could not read {p}: {e} -- size checks only");
                Default::default()
            }
        },
        None => {
            println!("hashes     none (--inventory not given) -- size checks only");
            Default::default()
        }
    };
    let saves_root = pick(args.saves.clone(), &cfg.library.saves)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(&root).join("saves"));
    std::fs::create_dir_all(&saves_root)?;
    let state_path = saves_root.join("sync-state.json");
    let data: Persisted = std::fs::read(&state_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    println!(
        "saves      {} ({} devices known)",
        saves_root.display(),
        data.devices.len()
    );
    let firmware = match pick(args.firmware.clone(), &cfg.library.firmware).as_deref() {
        Some(f) => {
            let list = scan_firmware(std::path::Path::new(f));
            println!("firmware   {} files from {f}", list.len());
            list
        }
        None => {
            println!("firmware   none (--firmware not given)");
            Vec::new()
        }
    };
    // Name -> id, from the scan, so a list resolves without a database. The
    // keying rule lives in `collections` because the web UI resolves the same
    // lists against different ids and the two must agree on what a name is.
    let by_name = collections::name_table(
        games
            .iter()
            .enumerate()
            .map(|(i, g)| (g.platform_slug.as_str(), g.name.as_str(), g.fs_name.as_str(), i as i64 + 1)),
    );
    let col_dir = pick(args.collections.clone(), &cfg.library.collections)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(&root).join("collections"));
    let (cols, unmatched) = collections::load(&col_dir, &by_name);
    // No `fav_ids` here. Favourites reach the client as membership of a
    // starred collection -- `/api/collections` carries `is_favorite` and
    // `src/api.rs` has no per-rom flag to put one in. One was computed and
    // dropped for several commits, which reads as a half-finished feature
    // rather than as a thing that is already answered elsewhere.

    println!(
        "collections {} lists, {} memberships from {}",
        cols.len(),
        cols.iter().map(|c| c.rom_count).sum::<i64>(),
        col_dir.display()
    );
    if !unmatched.is_empty() {
        // Named, not counted: a line that stopped resolving is the rot this
        // shape exists to make visible.
        eprintln!("collections {} names did not resolve:", unmatched.len());
        for u in unmatched.iter().take(10) {
            eprintln!("            {} / {}", u.collection, u.name);
        }
    }
    let lib = Arc::new(Library {
        games,
        collections: cols,
        hashes,
        firmware,
        sync: std::sync::Mutex::new(SyncState {
            store: saves::SaveStore::new(&saves_root),
            states: saves::StateStore::new(saves_root.join("_states")),
            path: state_path,
            data,
        }),
    });

    let ui_path = pick(args.ui.clone(), &cfg.library.ui);
    let mut app = app(lib, media_dir, ui_path.is_none());

    // The UI, when there is one. Its `/` replaces the status page: a person who
    // types the address wants the app, not a summary of it.
    if let Some(ui) = ui_path.as_deref() {
        let ui_dir = std::path::PathBuf::from(ui);
        // The same constructor the desktop app uses.
        let app_cfg = cfg
            .library
            .app_config
            .clone()
            .unwrap_or_else(|| "config.toml".to_owned());
        let mut state = moose_rack::app::AppState::from_config_at(std::path::Path::new(&app_cfg))
            .context("building app state for the web UI")?;
        // One source of truth for where the library is: this service was told,
        // and the state follows. Otherwise `config.toml` would have to repeat
        // the paths, and the two would disagree the first time one was edited.
        state.point_at(&layout);
        // This process has RetroArch on it and no one in front of it. Saying so
        // is what lets the web UI offer to play in the page instead.
        state.serve_only();
        println!("ui         {} (app config {app_cfg})", ui_dir.display());
        // The filesystem is the truth and the cache is derived -- the same rule
        // the API answers by. Scanning at startup costs a second on top of the
        // scan that already happened and means the page can never show a
        // library that is no longer there.
        match state.rescan(&layout) {
            Ok((n, folded)) => println!(
                "ui cache   {n} games{}",
                if folded > 0 { format!(", {folded} folded into synced rows") } else { String::new() }
            ),
            // Not fatal. The API half is unaffected, and a UI listing a stale
            // cache beats a service that refuses to start.
            Err(e) => eprintln!("ui cache   not rebuilt: {e}"),
        }
        // The lists again, against the ids the cache just wrote. Without this
        // the Collections tab is empty on a server serving all 27 of them --
        // `/api/collections` answers the client, and the UI here does not go
        // through the client.
        match state
            .cache
            .lock()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .and_then(|mut c| collections::into_cache(&mut c, &col_dir))
        {
            Ok((n, miss)) => println!(
                "ui lists   {n} collections{}",
                if miss > 0 { format!(", {miss} names did not resolve") } else { String::new() }
            ),
            Err(e) => eprintln!("ui lists   not loaded: {e}"),
        }
        let emulatorjs = cfg
            .library
            .emulatorjs
            .clone()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                // `<repo>/ui` -> `<repo>/assets/emulatorjs`.
                ui_dir
                    .parent()
                    .map(|r| r.join("assets/emulatorjs"))
                    .unwrap_or_else(|| std::path::PathBuf::from("assets/emulatorjs"))
            });
        match std::fs::read_dir(emulatorjs.join("data/cores")) {
            Ok(rd) => println!("emulatorjs {} ({} cores)", emulatorjs.display(), rd.count()),
            // Not fatal, and named: the Play button in a browser is the only
            // thing that needs it, and it says so itself when pressed.
            Err(_) => println!(
                "emulatorjs not installed at {} -- run scripts/fetch-emulatorjs.sh",
                emulatorjs.display()
            ),
        }
        app = web_app(Arc::new(web::WebState { state, ui_dir, emulatorjs })).merge(app);
    }

    // Everything above is routes; this is who may reach them. Applied to the
    // merged router rather than to each half, so a route added to either is
    // covered by construction rather than by remembering.
    let guard = std::sync::Arc::new(Guard {
        cfg: cfg.auth.clone(),
        sessions: auth::Sessions::default(),
    });
    println!("auth       {}", guard.cfg.describe());
    if guard.cfg.open() {
        println!("           anyone who can reach the port has full access");
    }
    app = app
        .merge(
            Router::new()
                .route("/login", get(login_page).post(login))
                .route("/logout", get(logout).post(logout))
                .with_state(guard.clone()),
        )
        .layer(axum::middleware::from_fn_with_state(guard.clone(), require_auth));

    let bind = cfg.server.bind.clone().unwrap_or(args.bind.clone());
    let mut addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("bind {bind:?} is not host:port"))?;
    // Flag, then file, then whatever `bind` already said.
    if let Some(p) = args.port.or(cfg.server.port) {
        addr.set_port(p);
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("listening  http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    /// A two-game ES-DE tree on disk, because the scan reads real files and a
    /// mock of it would test the mock.
    fn fixture(dir: &std::path::Path) -> (Arc<Library>, std::path::PathBuf) {
        let roms = dir.join("ROMs/nes");
        let lists = dir.join("gamelists/nes");
        let media = dir.join("downloaded_media/nes/miximages");
        for d in [&roms, &lists, &media] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(roms.join("Alpha (USA).zip"), b"alpha-bytes").unwrap();
        std::fs::write(roms.join("Beta (USA).zip"), b"beta").unwrap();
        std::fs::write(media.join("Alpha (USA).png"), b"PNG").unwrap();
        // A BIOS tree with a subdirectory, because the real one has mame/ and
        // fbneo/ and the flattening is the part worth testing.
        std::fs::create_dir_all(dir.join("bios/mame")).unwrap();
        std::fs::write(dir.join("bios/scph1001.bin"), b"psx-bios").unwrap();
        std::fs::write(dir.join("bios/mame/neogeo.zip"), b"neogeo").unwrap();
        std::fs::write(
            lists.join("gamelist.xml"),
            r#"<?xml version="1.0"?><gameList>
                 <game><path>./Alpha (USA).zip</path><name>Alpha</name></game>
                 <game><path>./Beta (USA).zip</path><name>Beta</name></game>
               </gameList>"#,
        )
        .unwrap();

        let layout = esde::Layout::new(dir, Some(&dir.join("ROMs")));
        let (games, _) = esde::scan(&layout, &CoreMap::embedded()).unwrap();
        let saves_root = dir.join("saves");
        std::fs::create_dir_all(&saves_root).unwrap();
        let lib = Library {
            games,
            collections: Vec::new(),
            hashes: Default::default(),
            firmware: scan_firmware(&dir.join("bios")),
            sync: std::sync::Mutex::new(SyncState {
                store: saves::SaveStore::new(&saves_root),
                states: saves::StateStore::new(saves_root.join("_states")),
                path: saves_root.join("sync-state.json"),
                data: Default::default(),
            }),
        };
        (Arc::new(lib), layout.media)
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
        let r = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn built() -> (tempdir::TempDir, Router) {
        let d = tempdir::TempDir::new("svc").unwrap();
        let (lib, media) = fixture(d.path());
        let r = app(lib, media, true);
        (d, r)
    }

    /// The same router the binary builds, guard included.
    ///
    /// Built through the real middleware rather than by calling `from_header`
    /// directly: the whole question is whether the routes are actually behind
    /// it, and a unit test of the checker cannot answer that.
    fn guarded(cfg: auth::AuthConfig) -> (tempdir::TempDir, Router) {
        let d = tempdir::TempDir::new("svc-auth").unwrap();
        let (lib, media) = fixture(d.path());
        let guard = std::sync::Arc::new(Guard { cfg, sessions: auth::Sessions::default() });
        let r = app(lib, media, true)
            .merge(
                Router::new()
                    // Qualified: this module has its own `get` helper for
                    // issuing requests, which shadows the router builder.
                    .route("/login", axum::routing::get(login_page).post(login))
                    .route("/logout", axum::routing::get(logout).post(logout))
                    .with_state(guard.clone()),
            )
            .layer(axum::middleware::from_fn_with_state(guard, require_auth));
        (d, r)
    }

    fn locked() -> auth::AuthConfig {
        auth::AuthConfig {
            token: Some("owner-secret".into()),
            users: vec![auth::User {
                name: "guest".into(),
                password: auth::hash_password("hunter2"),
            }],
            guest: false,
        }
    }

    /// Send a request with whatever headers a test wants.
    async fn req(app: &Router, method: &str, path: &str, headers: &[(&str, &str)]) -> (StatusCode, String) {
        let mut b = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let r = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn basic(user: &str, pass: &str) -> String {
        use base64::Engine as _;
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
        )
    }

    /// With credentials configured, an anonymous request gets nothing.
    ///
    /// Every route, not a sample: the guard is a layer on the merged router
    /// precisely so a route added later cannot miss it, and this is what proves
    /// that held.
    #[tokio::test]
    async fn without_credentials_the_library_is_closed() {
        let (_d, app) = guarded(locked());
        for path in [
            "/", "/api/roms", "/api/platforms", "/api/collections", "/api/firmware",
            "/api/config", "/api/users/me", "/api/saves", "/api/roms/1",
            "/api/roms/identifiers", "/media?path=/etc/passwd",
        ] {
            let (s, _) = req(&app, "GET", path, &[]).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{path} answered without credentials");
        }
    }

    /// Heartbeat stays open, or a bad token is indistinguishable from a machine
    /// that is switched off.
    #[tokio::test]
    async fn the_heartbeat_answers_anyone() {
        let (_d, app) = guarded(locked());
        let (s, _) = req(&app, "GET", "/api/heartbeat", &[]).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_token_opens_everything_and_a_wrong_one_opens_nothing() {
        let (_d, app) = guarded(locked());
        let ok = [("authorization", "Bearer owner-secret")];
        let (s, _) = req(&app, "GET", "/api/roms", &ok).await;
        assert_eq!(s, StatusCode::OK);
        for bad in ["Bearer wrong", "Bearer ", "Bearer owner-secre", "owner-secret"] {
            let (s, _) = req(&app, "GET", "/api/roms", &[("authorization", bad)]).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{bad:?} got in");
        }
    }

    #[tokio::test]
    async fn a_username_and_password_open_the_library() {
        let (_d, app) = guarded(locked());
        let (s, _) = req(&app, "GET", "/api/roms", &[("authorization", &basic("guest", "hunter2"))]).await;
        assert_eq!(s, StatusCode::OK);
        for (u, pw) in [("guest", "wrong"), ("nobody", "hunter2"), ("", "")] {
            let (s, _) = req(&app, "GET", "/api/roms", &[("authorization", &basic(u, pw))]).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{u}:{pw} got in");
        }
    }

    /// Configure nothing and nothing is checked -- so upgrading the binary does
    /// not lock somebody out of their own library.
    #[tokio::test]
    async fn an_empty_auth_section_leaves_the_service_open() {
        let (_d, app) = guarded(auth::AuthConfig::default());
        let (s, _) = req(&app, "GET", "/api/roms", &[]).await;
        assert_eq!(s, StatusCode::OK);
    }

    /// A browser cannot send a token in a header, so it trades one for a cookie.
    #[tokio::test]
    async fn login_returns_a_cookie_that_then_works() {
        let (_d, app) = guarded(locked());
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"owner-secret"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let set = r.headers().get("set-cookie").unwrap().to_str().unwrap().to_owned();
        assert!(set.contains("HttpOnly"), "a script must not be able to read it: {set}");
        assert!(set.contains("SameSite=Lax"), "{set}");
        let cookie = set.split(';').next().unwrap().to_owned();

        let (s, _) = req(&app, "GET", "/api/roms", &[("cookie", &cookie)]).await;
        assert_eq!(s, StatusCode::OK, "the cookie login did not carry");

        // And a made-up one does not.
        let (s, _) = req(&app, "GET", "/api/roms", &[("cookie", "moose_session=invented")]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_bad_login_is_refused_without_saying_which_half_was_wrong() {
        let (_d, app) = guarded(locked());
        for body in [
            r#"{"token":"nope"}"#,
            r#"{"username":"guest","password":"nope"}"#,
            r#"{"username":"nobody","password":"hunter2"}"#,
            r#"{}"#,
        ] {
            let r = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/login")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{body} was accepted");
            assert!(r.headers().get("set-cookie").is_none(), "{body} got a session");
        }
    }

    /// A browser asking for a page is sent to sign in; a client asking for JSON
    /// gets a 401 it can report.
    #[tokio::test]
    async fn a_browser_is_redirected_and_a_client_is_told() {
        let (_d, app) = guarded(locked());
        let (s, _) = req(&app, "GET", "/", &[("accept", "text/html")]).await;
        assert_eq!(s, StatusCode::SEE_OTHER, "a browser should land on the login page");
        let (s, _) = req(&app, "GET", "/api/roms", &[("accept", "text/html")]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "an API route must never redirect");
    }

    /// The login page renders before anyone has proved anything.
    #[tokio::test]
    async fn the_login_page_is_public() {
        let (_d, app) = guarded(locked());
        let (s, body) = req(&app, "GET", "/login", &[]).await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("Sign in"), "{body:.80}");
    }

    /// A guest may read the library through the UI's own IPC and may not
    /// change it. One route, eighty commands, and the line runs between them.
    #[tokio::test]
    async fn a_guest_may_read_through_invoke_but_not_write() {
        let (_d, app) = guarded(locked());
        let post = |auth: String, cmd: &str| {
            let app = app.clone();
            let uri = format!("/invoke/{cmd}");
            async move {
                app.oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("authorization", auth)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
            }
        };
        // The write is refused for the guest and reaches the backend for the
        // owner. `set_list_art` with no argument is a 400 from the command
        // itself -- which is the point: it got past the guard.
        assert_eq!(post(basic("guest", "hunter2"), "set_list_art").await, StatusCode::FORBIDDEN);
        assert_ne!(
            post("Bearer owner-secret".into(), "set_list_art").await,
            StatusCode::FORBIDDEN,
            "the owner was refused their own settings"
        );
        // And an unknown command is owner-only, so a guest cannot probe for
        // one that was added without being classified.
        assert_eq!(post(basic("guest", "hunter2"), "brand_new").await, StatusCode::FORBIDDEN);
    }

    /// The gap a last audit before decommissioning found.
    ///
    /// `src/api.rs` calls `/api/states` and `/api/states/{id}/content`, and
    /// `statesync::run` is reached from every save sync -- so save states were
    /// silently unsynced while saves worked. Nothing reported it because the
    /// client asks for states after saves and a 404 there is not fatal.
    #[tokio::test]
    async fn save_states_upload_list_and_download() {
        let (_d, app) = built();
        let body = concat!(
            "--X\r\n",
            "Content-Disposition: form-data; name=\"stateFile\"; filename=\"Game.state1\"\r\n\r\n",
            "frozen\r\n",
            "--X--\r\n"
        );
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/states?rom_id=1&emulator=snes9x")
                    .header("content-type", "multipart/form-data; boundary=X")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let up: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(up["rom_id"], 1);
        assert_eq!(up["file_name"], "Game.state1");
        assert_eq!(up["emulator"], "snes9x");
        let id = up["id"].as_i64().unwrap();

        let (s, body) = get(&app, "/api/states?rom_id=1").await;
        assert_eq!(s, StatusCode::OK);
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["id"], id);

        let (s, bytes) = get(&app, &format!("/api/states/{id}/content")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(bytes, "frozen");

        // Another game's states are not this game's.
        let (_, other) = get(&app, "/api/states?rom_id=2").await;
        assert_eq!(other, "[]");
    }

    /// A filename with a path in it must not write outside the store.
    #[tokio::test]
    async fn an_uploaded_state_cannot_escape_its_directory() {
        let (_d, app) = built();
        for name in ["../../escape.state", "/etc/passwd", ".hidden"] {
            let body = format!(
                "--X\r\nContent-Disposition: form-data; name=\"stateFile\"; filename=\"{name}\"\r\n\r\nx\r\n--X--\r\n"
            );
            let r = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/states?rom_id=1")
                        .header("content-type", "multipart/form-data; boundary=X")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            // Either refused outright, or reduced to a bare name inside the
            // store -- never a path.
            if r.status() == StatusCode::OK {
                let v: serde_json::Value = serde_json::from_slice(
                    &axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap(),
                )
                .unwrap();
                let got = v["file_name"].as_str().unwrap();
                assert!(!got.contains('/'), "{name} was stored as {got}");
                assert!(!got.starts_with('.'), "{name} was stored as {got}");
            }
        }
    }

    fn with_guest() -> auth::AuthConfig {
        let mut c = locked();
        c.guest = true;
        c
    }

    /// A POST whose response cookie a test needs. Named apart from the
    /// existing `post_json`, which takes a `Value` and returns no headers.
    async fn post_login(app: &Router, uri: &str, body: &str) -> (StatusCode, Option<String>, String) {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = r.status();
        let cookie = r
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .map(|c| c.split(';').next().unwrap().to_owned());
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        (status, cookie, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The shared account: one button, no password, and it reads the library.
    #[tokio::test]
    async fn a_guest_can_sign_in_with_the_button() {
        let (_d, app) = guarded(with_guest());
        let (s, cookie, body) = post_login(&app, "/login", r#"{"guest":true}"#).await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains(r#""owner":false"#), "{body}");
        let cookie = cookie.expect("no session cookie");
        let (s, _) = req(&app, "GET", "/api/roms", &[("cookie", &cookie)]).await;
        assert_eq!(s, StatusCode::OK, "a guest could not read the library");
    }

    /// And is still not the owner.
    #[tokio::test]
    async fn a_guest_may_sync_saves_and_may_not_change_settings() {
        let (_d, app) = guarded(with_guest());
        let (_, cookie, _) = post_login(&app, "/login", r#"{"guest":true}"#).await;
        let cookie = cookie.unwrap();
        let (s, _) = req(&app, "POST", "/invoke/sync_saves", &[("cookie", &cookie)]).await;
        assert_ne!(s, StatusCode::FORBIDDEN, "saves are shared in that account");
        let (s, _) = req(&app, "POST", "/invoke/set_list_art", &[("cookie", &cookie)]).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    /// Off unless asked for. The button is not drawn and the door does not open.
    #[tokio::test]
    async fn guest_access_is_refused_when_it_is_not_switched_on() {
        let (_d, app) = guarded(locked());
        let (s, cookie, _) = post_login(&app, "/login", r#"{"guest":true}"#).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert!(cookie.is_none(), "a session was opened for a guest that is off");
        let (_, page) = req(&app, "GET", "/login", &[]).await;
        assert!(!page.contains("Continue as guest"), "the button is drawn but does nothing");
    }

    #[tokio::test]
    async fn the_guest_button_appears_only_when_it_works() {
        let (_d, app) = guarded(with_guest());
        let (_, page) = req(&app, "GET", "/login", &[]).await;
        assert!(page.contains("Continue as guest"), "the button was not drawn");
    }

    #[tokio::test]
    async fn the_scan_is_what_the_api_reports() {
        let (_d, app) = built();
        let (s, body) = get(&app, "/api/roms").await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["total"], 2);
        assert_eq!(v["items"].as_array().unwrap().len(), 2);
    }

    /// The bug a real sync found. With no such route `/api/roms/{id}` matches,
    /// "identifiers" reaches the i64 extractor and the answer is 400, so the
    /// client cannot list ids and skips pruning -- its only way to notice a
    /// deletion. Verified to fail when the route is removed.
    #[tokio::test]
    async fn identifiers_is_not_swallowed_by_the_id_route() {
        let (_d, app) = built();
        let (s, body) = get(&app, "/api/roms/identifiers").await;
        assert_eq!(s, StatusCode::OK, "identifiers must not hit the i64 extractor");
        assert_eq!(serde_json::from_str::<Vec<i64>>(&body).unwrap(), vec![1, 2]);
    }

    #[tokio::test]
    async fn ids_are_one_based_and_out_of_range_is_404() {
        let (_d, app) = built();
        assert_eq!(get(&app, "/api/roms/1").await.0, StatusCode::OK);
        // Zero is what a missing field deserializes to; it must not resolve.
        assert_eq!(get(&app, "/api/roms/0").await.0, StatusCode::NOT_FOUND);
        assert_eq!(get(&app, "/api/roms/99").await.0, StatusCode::NOT_FOUND);
    }

    /// The client reads an empty list as "none yet" and a 404 as an error, and
    /// an ES-DE tree genuinely has no collections.
    #[tokio::test]
    async fn collections_is_empty_not_missing() {
        let (_d, app) = built();
        let (s, body) = get(&app, "/api/collections").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body, "[]");
    }

    /// Saying "no hashes" when there are none is right; saying it when there are
    /// would leave every download size-checked.
    #[tokio::test]
    async fn skip_hash_tracks_whether_an_inventory_was_loaded() {
        let d = tempdir::TempDir::new("svc").unwrap();
        let (lib, media) = fixture(d.path());
        let (s, body) = get(&app(lib, media.clone(), true), "/api/config").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()
            ["SKIP_HASH_CALCULATION"], true, "no inventory -> true");

        let (mut lib2, _) = fixture(d.path());
        let l = Arc::get_mut(&mut lib2).unwrap();
        l.hashes.insert(
            ("nes".into(), "Alpha (USA).zip".into()),
            (Some("abc".into()), None, None),
        );
        let (_, body) = get(&app(lib2, media, true), "/api/config").await;
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()
            ["SKIP_HASH_CALCULATION"], false, "inventory loaded -> false");
    }

    #[tokio::test]
    async fn a_hash_is_served_when_the_inventory_has_one() {
        let d = tempdir::TempDir::new("svc").unwrap();
        let (mut lib, media) = fixture(d.path());
        Arc::get_mut(&mut lib).unwrap().hashes.insert(
            ("nes".into(), "Alpha (USA).zip".into()),
            (Some("deadbeef".into()), Some("cafe".into()), None),
        );
        let (_, body) = get(&app(lib, media, true), "/api/roms").await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let alpha = v["items"].as_array().unwrap().iter()
            .find(|i| i["fs_name"] == "Alpha (USA).zip").unwrap();
        assert_eq!(alpha["md5_hash"], "deadbeef");
        assert_eq!(alpha["sha1_hash"], "cafe");
        // A game the inventory has not seen gets no hash rather than a wrong one.
        let beta = v["items"].as_array().unwrap().iter()
            .find(|i| i["fs_name"] == "Beta (USA).zip").unwrap();
        assert!(beta.get("md5_hash").is_none(), "unknown file must publish no hash");
    }

    #[tokio::test]
    async fn content_serves_the_bytes_and_honours_range() {
        let (_d, app) = built();
        let (s, body) = get(&app, "/api/roms/1/content/Alpha%20(USA).zip").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(body, "alpha-bytes");

        // 206 with a correct tail, or the client discards its partial and
        // re-downloads the whole file without ever reporting a problem.
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/roms/1/content/Alpha%20(USA).zip")
                    .header("Range", "bytes=6-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"bytes");
    }

    #[tokio::test]
    async fn platform_counts_come_from_the_scan() {
        let (_d, app) = built();
        let (_, body) = get(&app, "/api/platforms").await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let nes = v.as_array().unwrap().iter().find(|p| p["fs_slug"] == "nes").unwrap();
        assert_eq!(nes["rom_count"], 2);
    }

    async fn post_json(app: &Router, uri: &str, body: serde_json::Value) -> (StatusCode, String) {
        let r = app.clone().oneshot(
            Request::builder().method("POST").uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let s = r.status();
        let b = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        (s, String::from_utf8_lossy(&b).into_owned())
    }

    /// A multipart body with one `saveFile` part, built by hand so the test does
    /// not depend on a client library to describe the wire format.
    async fn post_save(app: &Router, uri: &str, name: &str, bytes: &[u8]) -> (StatusCode, String) {
        let b = "X-BOUND";
        let mut body = Vec::new();
        body.extend_from_slice(format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"saveFile\"; filename=\"{name}\"\r\n\r\n"
        ).as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{b}--\r\n").as_bytes());
        let r = app.clone().oneshot(
            Request::builder().method("POST").uri(uri)
                .header("content-type", format!("multipart/form-data; boundary={b}"))
                .body(Body::from(body)).unwrap()).await.unwrap();
        let s = r.status();
        let rb = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        (s, String::from_utf8_lossy(&rb).into_owned())
    }

    /// Minting a fresh id per call would give each one empty bookkeeping, and
    /// every save would then look like a first-time upload.
    #[tokio::test]
    async fn registering_the_same_device_twice_returns_one_id() {
        let (_d, app) = built();
        let (s1, b1) = post_json(&app, "/api/devices", serde_json::json!({"name": "flip"})).await;
        let (_, b2) = post_json(&app, "/api/devices", serde_json::json!({"name": "flip"})).await;
        assert_eq!(s1, StatusCode::OK);
        let id = |b: &str| serde_json::from_str::<serde_json::Value>(b).unwrap()["id"].clone();
        assert_eq!(id(&b1), id(&b2), "same name must not mint a second device");
        let (_, b3) = post_json(&app, "/api/devices", serde_json::json!({"name": "mac"})).await;
        assert_ne!(id(&b1), id(&b3), "different names are different devices");
    }

    #[tokio::test]
    async fn an_uploaded_save_can_be_listed_and_read_back() {
        let (_d, app) = built();
        let (s, body) = post_save(&app, "/api/saves?rom_id=1&device_id=dev", "a.srm", b"save-bytes").await;
        assert_eq!(s, StatusCode::OK, "{body}");
        let saved: serde_json::Value = serde_json::from_str(&body).unwrap();
        let id = saved["id"].as_i64().unwrap();

        let (_, listed) = get(&app, "/api/saves?rom_id=1").await;
        let v: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);

        let (s2, content) = get(&app, &format!("/api/saves/{id}/content")).await;
        assert_eq!(s2, StatusCode::OK);
        assert_eq!(content, "save-bytes");
    }

    /// Refusing is the point: 409 is how the client discovers a conflict at all.
    #[tokio::test]
    async fn a_second_device_overwriting_blind_is_refused() {
        let (_d, app) = built();
        let (s1, _) = post_save(&app, "/api/saves?rom_id=1&device_id=alice", "a.srm", b"from-alice").await;
        assert_eq!(s1, StatusCode::OK);
        // Bob never agreed on alice's bytes, so his upload must not land.
        let (s2, body) = post_save(&app, "/api/saves?rom_id=1&device_id=bob", "a.srm", b"from-bob").await;
        assert_eq!(s2, StatusCode::CONFLICT, "{body}");
        // and the bytes are untouched
        let (_, listed) = get(&app, "/api/saves?rom_id=1").await;
        let v: serde_json::Value = serde_json::from_str(&listed).unwrap();
        let id = v[0]["id"].as_i64().unwrap();
        assert_eq!(get(&app, &format!("/api/saves/{id}/content")).await.1, "from-alice");
    }

    /// Overwrite is not a retry — it carries out a decision already shown to a
    /// person.
    #[tokio::test]
    async fn overwrite_true_lands_after_a_conflict() {
        let (_d, app) = built();
        post_save(&app, "/api/saves?rom_id=1&device_id=alice", "a.srm", b"from-alice").await;
        let (s, _) = post_save(
            &app, "/api/saves?rom_id=1&device_id=bob&overwrite=true", "a.srm", b"from-bob").await;
        assert_eq!(s, StatusCode::OK);
        let (_, listed) = get(&app, "/api/saves?rom_id=1").await;
        let id = serde_json::from_str::<serde_json::Value>(&listed).unwrap()[0]["id"].as_i64().unwrap();
        assert_eq!(get(&app, &format!("/api/saves/{id}/content")).await.1, "from-bob");
    }

    /// The device that just uploaded has agreed on those bytes, so its next
    /// negotiate is a no-op rather than a conflict with itself.
    #[tokio::test]
    async fn uploading_records_agreement_so_the_next_sync_is_quiet() {
        let (_d, app) = built();
        post_save(&app, "/api/saves?rom_id=1&device_id=alice", "a.srm", b"bytes").await;
        let hash = "b1946ac92492d2347c6235b4d2611184"; // md5 of "bytes\n"? no: of "bytes"
        let (_, listed) = get(&app, "/api/saves?rom_id=1").await;
        let server_hash = serde_json::from_str::<serde_json::Value>(&listed).unwrap()[0]
            ["content_hash"].as_str().unwrap().to_owned();
        let _ = hash;
        let (s, body) = post_json(&app, "/api/sync/negotiate", serde_json::json!({
            "device_id": "alice",
            "saves": [{"rom_id": 1, "file_name": "a.srm", "content_hash": server_hash,
                       "updated_at": "now", "file_size_bytes": 5}]
        })).await;
        assert_eq!(s, StatusCode::OK);
        let plan: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(plan["total_no_op"], 1, "same bytes both sides");
        assert_eq!(plan["total_conflict"], 0);
    }

    #[tokio::test]
    async fn negotiate_tells_a_new_device_to_download() {
        let (_d, app) = built();
        post_save(&app, "/api/saves?rom_id=1&device_id=alice", "a.srm", b"bytes").await;
        let (_, body) = post_json(&app, "/api/sync/negotiate", serde_json::json!({
            "device_id": "flip", "saves": []
        })).await;
        let plan: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(plan["total_download"], 1);
        assert_eq!(plan["operations"][0]["action"], "download");
    }

    /// The tree has subdirectories; RetroArch wants one flat system directory.
    /// So the sub-path is reported and not reproduced.
    #[tokio::test]
    async fn firmware_is_flattened_but_remembers_where_it_came_from() {
        let (_d, app) = built();
        let (s, body) = get(&app, "/api/firmware").await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let all = v.as_array().unwrap();
        assert_eq!(all.len(), 2);
        let nested = all.iter().find(|f| f["file_name"] == "neogeo.zip").unwrap();
        assert_eq!(nested["file_path"], "mame", "sub-path is reported");
        let top = all.iter().find(|f| f["file_name"] == "scph1001.bin").unwrap();
        assert!(top["file_path"].is_null(), "a top-level file has no sub-path");
    }

    /// A wrong BIOS breaks emulation silently, so the hashes travel with the
    /// listing rather than being fetched separately.
    #[tokio::test]
    async fn firmware_carries_hashes_and_serves_its_bytes() {
        let (_d, app) = built();
        let (_, body) = get(&app, "/api/firmware").await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let f = v.as_array().unwrap().iter()
            .find(|f| f["file_name"] == "scph1001.bin").unwrap();
        // md5 and sha1 of "psx-bios", computed with python rather than copied
        // out of this code's own output -- otherwise the test only asserts the
        // implementation agrees with itself.
        assert_eq!(f["md5_hash"], "3400cfcc9a4e5a91adae128ed69ffab3");
        assert_eq!(f["sha1_hash"], "5be58ba07567dcbe81b3a1c4e5af279a2e1d3dcd");
        assert_eq!(f["file_size_bytes"], 8);
        assert_eq!(f["is_verified"], true);
        let id = f["id"].as_i64().unwrap();
        let (s, bytes) = get(&app, &format!("/api/firmware/{id}/content/scph1001.bin")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(bytes, "psx-bios");
    }

    #[tokio::test]
    async fn an_unknown_firmware_id_is_404() {
        let (_d, app) = built();
        assert_eq!(get(&app, "/api/firmware/999/content/x.bin").await.0, StatusCode::NOT_FOUND);
    }

    /// `/` answered 404, which reads as "the server is down" rather than "that
    /// path does not exist". A person who types the address gets a page.
    #[tokio::test]
    async fn the_root_is_a_page_not_a_404() {
        let (_d, app) = built();
        let (s, body) = get(&app, "/").await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.contains("Moose Rack"), "should name itself");
        assert!(body.contains("/api/heartbeat"), "should point at the API");
        assert!(body.contains("nes"), "should list what it holds");
    }

    /// The GUI asks for all three on every load. A 404 is survivable -- each is
    /// wrapped in `unwrap_or_default` -- but it logs an error forever, and the
    /// honest answer is that there are none.
    #[tokio::test]
    async fn the_generated_collection_endpoints_answer_empty() {
        let (_d, app) = built();
        for uri in ["/api/collections/smart", "/api/collections/virtual?type=genre",
                    "/api/collections/virtual/identifiers"] {
            let (s, body) = get(&app, uri).await;
            assert_eq!(s, StatusCode::OK, "{uri}");
            assert_eq!(body, "[]", "{uri}");
        }
    }

    /// A missing config is fine -- the flags alone are a complete way to run
    /// this -- but a malformed one must not fall back to defaults and serve the
    /// wrong library while looking like it worked.
    #[test]
    fn a_missing_config_is_defaults_and_a_broken_one_is_an_error() {
        let d = tempdir::TempDir::new("cfg").unwrap();
        let absent = d.path().join("nope.toml");
        assert!(load_config(&absent).is_ok(), "missing is not an error");

        let bad = d.path().join("bad.toml");
        std::fs::write(&bad, "[library\nroot = ").unwrap();
        let e = load_config(&bad).unwrap_err().to_string();
        assert!(e.contains("parsing"), "{e}");
    }

    #[test]
    fn the_config_supplies_paths_and_a_port() {
        let d = tempdir::TempDir::new("cfg").unwrap();
        let f = d.path().join("s.toml");
        std::fs::write(&f, r#"
[library]
root = "/lib/ES-DE"
roms = "/lib/ROMs"
firmware = "/lib/bios"
[server]
port = 9999
"#).unwrap();
        let c = load_config(&f).unwrap();
        assert_eq!(c.library.root.as_deref(), Some("/lib/ES-DE"));
        assert_eq!(c.library.roms.as_deref(), Some("/lib/ROMs"));
        assert_eq!(c.library.firmware.as_deref(), Some("/lib/bios"));
        assert_eq!(c.server.port, Some(9999));
        // Unset stays unset rather than becoming an empty string.
        assert!(c.library.media.is_none());
    }

    /// The shipped example must parse, or it is documentation for something
    /// that does not work.
    #[test]
    fn the_example_config_parses() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().join("moose-service.example.toml");
        if !p.exists() { return }
        let c = load_config(&p).expect("the shipped example must parse");
        assert!(c.library.root.is_some(), "the example should set a root");
    }
}

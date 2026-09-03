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

#[derive(Parser)]
#[command(about = "Serve an ES-DE library over the API the app already speaks")]
struct Args {
    /// ES-DE data directory: the one holding gamelists/ and downloaded_media/
    #[arg(long)]
    root: String,
    /// ROMs directory, if it is not <root>/ROMs
    #[arg(long)]
    roms: Option<String>,
    /// Artwork directory, if it is not <root>/downloaded_media
    #[arg(long)]
    media: Option<String>,
    /// Address to bind. Use this to restrict the interface as well as the port.
    #[arg(long, env = "MOOSE_SERVICE_BIND", default_value = "0.0.0.0:8001")]
    bind: String,
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
}

/// The scan, held for the process lifetime.
///
/// Rescanning is cheap — 11,473 games in about three seconds — but not free, and
/// nothing here mutates it yet. When writes arrive this becomes a lock rather
/// than an Arc.
struct Library {
    games: Vec<esde::Game>,
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

/// Collections, which an ES-DE tree does not have.
///
/// Empty rather than 404: the client treats a missing endpoint as an error and
/// an empty list as "none yet", and the second is the truth. `library-service.md`
/// makes these text files under `collections/`; until that exists there are none.
async fn collections() -> Json<Vec<serde_json::Value>> {
    Json(vec![])
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

/// The routes, as a function so tests can build one without a socket.
fn app(lib: Arc<Library>, media_dir: std::path::PathBuf) -> Router {
        Router::new()
            .route("/api/heartbeat", get(heartbeat))
            .route("/api/config", get(config))
            .route("/api/users/me", get(users_me))
            .route("/api/platforms", get(platforms))
            .route("/api/roms", get(roms))
            .route("/api/roms/identifiers", get(rom_identifiers))
            .route("/api/roms/{id}", get(rom_by_id))
            .route("/api/roms/{id}/content/{*name}", get(rom_content))
            .route("/api/collections", get(collections))
        .route("/api/devices", axum::routing::post(register_device))
        .route("/api/saves", get(list_saves).post(upload_save))
        .route("/api/saves/{id}/content", get(save_content))
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
    let layout = esde::Layout::new(
        std::path::Path::new(&args.root),
        args.roms.as_deref().map(std::path::Path::new),
    )
    .with_media(args.media.as_deref().map(std::path::Path::new));

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
    let hashes = match args.inventory.as_deref() {
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
    let saves_root = args
        .saves
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(&args.root).join("saves"));
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
    let lib = Arc::new(Library {
        games,
        hashes,
        sync: std::sync::Mutex::new(SyncState {
            store: saves::SaveStore::new(&saves_root),
            path: state_path,
            data,
        }),
    });

    let app = app(lib, media_dir);

    let mut addr: SocketAddr = args
        .bind
        .parse()
        .with_context(|| format!("--bind {:?} is not host:port", args.bind))?;
    if let Some(p) = args.port {
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
            hashes: Default::default(),
            sync: std::sync::Mutex::new(SyncState {
                store: saves::SaveStore::new(&saves_root),
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
        let r = app(lib, media);
        (d, r)
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
        let (s, body) = get(&app(lib, media.clone()), "/api/config").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap()
            ["SKIP_HASH_CALCULATION"], true, "no inventory -> true");

        let (mut lib2, _) = fixture(d.path());
        let l = Arc::get_mut(&mut lib2).unwrap();
        l.hashes.insert(
            ("nes".into(), "Alpha (USA).zip".into()),
            (Some("abc".into()), None, None),
        );
        let (_, body) = get(&app(lib2, media), "/api/config").await;
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
        let (_, body) = get(&app(lib, media), "/api/roms").await;
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
}

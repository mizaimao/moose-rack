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
    /// `(system, relative path)` -> `(md5, sha1, crc32)`, out of inventory.db.
    ///
    /// The client already knows how to verify a download — `verify()` in
    /// download.rs hashes what it got and compares. It falls back to a size
    /// check only when the server publishes nothing, which is what made the
    /// first transfers here unverified. The hashes were computed once over
    /// 1.76 TB; not serving them was the whole gap.
    hashes: std::collections::HashMap<(String, String), (Option<String>, Option<String>, Option<String>)>,
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
/// Registered before `/api/roms/{id}`, because otherwise axum hands the literal
/// "identifiers" to the i64 extractor and answers 400. That is exactly what a
/// first sync against this service did.
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
    let lib = Arc::new(Library { games, hashes });

    let app = Router::new()
        .route("/api/heartbeat", get(heartbeat))
        .route("/api/config", get(config))
        .route("/api/users/me", get(users_me))
        .route("/api/platforms", get(platforms))
        .route("/api/roms", get(roms))
        .route("/api/roms/identifiers", get(rom_identifiers))
        .route("/api/roms/{id}", get(rom_by_id))
        .route("/api/roms/{id}/content/{*name}", get(rom_content))
        .route("/api/collections", get(collections))
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
        .with_state(lib);

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

//! RomM API client.
//!
//! Findings that shape this file are in PLAN.md §3. The two that bite:
//! the list param is `platform_ids` (an array — unknown params are silently
//! ignored, so a typo pages the whole library), and connection reuse is worth
//! ~300x, which `reqwest`'s pool gives us for free as long as one Client is
//! shared.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde::Deserialize;

// Mirrors the server schema rather than only the fields read today.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: String,
}

/// The server's per-user record for a game.
///
/// `last_played` lives here rather than on the game because it is about a
/// person, not a file — which is also why the recent list is the same on every
/// machine signed in as you.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RomUser {
    #[serde(default)]
    pub last_played: Option<String>,
}

/// Every ScreenScraper media URL the server resolved for a game.
///
/// These are the server's own — it built them, it stores them, and it serves
/// them back on `/api/roms/{id}`. Reading them is reading your own library.
/// Nothing here is assembled by this app.
///
/// Present only for games the server actually matched against ScreenScraper. A
/// game it identified some other way, or not at all, has none of these.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SsMedia {
    #[serde(default)]
    pub box2d_url: Option<String>,
    #[serde(default)]
    pub box2d_back_url: Option<String>,
    #[serde(default)]
    pub box3d_url: Option<String>,
    #[serde(default)]
    pub physical_url: Option<String>,
    #[serde(default)]
    pub miximage_url: Option<String>,
    #[serde(default)]
    pub screenshot_url: Option<String>,
    #[serde(default)]
    pub title_screen_url: Option<String>,
    #[serde(default)]
    pub marquee_url: Option<String>,
    #[serde(default)]
    pub fanart_url: Option<String>,
    #[serde(default)]
    pub manual_url: Option<String>,
    #[serde(default)]
    pub video_url: Option<String>,
}

/// A game as the server knows it, for the fields the scraper needs.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RomMedia {
    #[serde(default)]
    pub ss_metadata: Option<SsMedia>,
}

/// One identification candidate from the server's metadata lookup.
#[derive(Debug, Clone, Deserialize)]
pub struct Match {
    #[serde(default)]
    pub name: Option<String>,
    /// ScreenScraper's game id, when it recognized the file.
    #[serde(default)]
    pub ss_id: Option<i64>,
    /// A complete, ready-to-fetch ScreenScraper media URL, built by the server.
    /// Empty when ScreenScraper had no match or is not enabled there.
    #[serde(default)]
    pub ss_url_cover: String,
}

// Mirrors the server schema rather than only the fields read today.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct Platform {
    pub id: i64,
    pub fs_slug: String,
    /// RomM's own slug, which is not always the folder name — `sms` against
    /// `mastersystem`, `genesis` against `megadrive`. The console pictures
    /// under `/assets/platforms/` are keyed by this one.
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub rom_count: i64,
}

// Mirrors the server schema rather than only the fields read today.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct Rom {
    pub id: i64,
    /// Per-user state the server keeps, including when this was last played.
    #[serde(default)]
    pub rom_user: Option<RomUser>,
    /// Display title, e.g. `"'88 Games"`. Distinct from `fs_name`.
    #[serde(default)]
    pub name: Option<String>,
    pub fs_name: String,
    /// The server scanned and could not find this file on disk.
    ///
    /// RomM does not delete the row when a ROM disappears — it keeps the entry
    /// and its metadata and raises this flag, which is the right call for a
    /// library you might restore from backup. It does mean a plain sync pulls
    /// in games that cannot be launched: 84 of them here after a cleanup, each
    /// of which would sit in the grid looking playable and fail at launch.
    #[serde(default)]
    pub missing_from_fs: bool,
    #[serde(default)]
    pub fs_size_bytes: Option<i64>,
    #[serde(default)]
    pub platform_fs_slug: Option<String>,
    #[serde(default)]
    pub md5_hash: Option<String>,
    #[serde(default)]
    pub sha1_hash: Option<String>,
    #[serde(default)]
    pub crc_hash: Option<String>,
    /// Drives incremental sync via the `updated_after` query param.
    #[serde(default)]
    pub updated_at: Option<String>,
    /// Server-side artwork, e.g. `/assets/romm/resources/roms/2/42/cover/big.png?ts=...`.
    /// Note the timestamp query contains a raw space and must be encoded.
    #[serde(default)]
    pub path_cover_large: Option<String>,
    /// Thumbnail variant — averages ~71 KB against ~278 KB for the large one,
    /// which matters a lot when a grid shows hundreds at once.
    #[serde(default)]
    pub path_cover_small: Option<String>,
    #[serde(default)]
    pub merged_screenshots: Vec<String>,

    // Descriptive metadata. `metadatum` is RomM's merged view across whatever
    // sources were enabled; on this server that is the ES-DE gamelist import.
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub metadatum: Option<serde_json::Value>,
    #[serde(default)]
    pub alternative_names: Vec<String>,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub path_manual: Option<String>,
    #[serde(default)]
    pub youtube_video_id: Option<String>,
    /// True for folder ROMs (multi-disc games). The content endpoint serves
    /// these as a zip of the folder, not the ROM itself.
    #[serde(default)]
    pub has_multiple_files: bool,
    /// Populated only when the request asks for `with_files=true`.
    #[serde(default)]
    pub files: Vec<RomFile>,
}

/// One file inside a multi-file (folder) ROM.
///
/// Mirrors the server schema rather than only the fields read today.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct RomFile {
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub file_size_bytes: Option<u64>,
    #[serde(default)]
    pub md5_hash: Option<String>,
    #[serde(default)]
    pub sha1_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RomPage {
    #[serde(default)]
    pub items: Vec<Rom>,
    #[serde(default)]
    pub total: i64,
}

/// Server-side settings that change how we must treat its data.
///
/// Fetched rather than assumed: the exclusion lists feed our archive-hash
/// reproduction, and they are configurable per deployment and per RomM
/// version. Hardcoding them means silently computing the wrong hash the day
/// someone edits `config.yml`.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub default_excluded_files: Vec<String>,
    #[serde(default)]
    pub default_excluded_extensions: Vec<String>,
    /// When true the server stores no hashes at all, so downloads can only be
    /// size-checked. Worth surfacing rather than discovering per-file.
    #[serde(default)]
    pub skip_hash_calculation: bool,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(rename = "DEFAULT_EXCLUDED_FILES", default)]
    files: Vec<String>,
    #[serde(rename = "DEFAULT_EXCLUDED_EXTENSIONS", default)]
    exts: Vec<String>,
    #[serde(rename = "SKIP_HASH_CALCULATION", default)]
    skip_hash: bool,
}

#[derive(Debug, Deserialize)]
pub struct Heartbeat {
    #[serde(rename = "SYSTEM")]
    pub system: HeartbeatSystem,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatSystem {
    #[serde(rename = "VERSION", default)]
    pub version: String,
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    auth: String,
}

impl Client {
    pub fn new(base_url: &str, username: &str, password: &str) -> Result<Self> {
        Self::with_auth(base_url, username, password, None)
    }

    /// As [`Self::new`], preferring a bearer token when one is configured.
    ///
    /// A RomM client token carries explicit scopes and can be revoked from the
    /// server without changing the account password, so it is strictly better
    /// than Basic for a device that keeps its credentials on disk. Basic stays
    /// supported because existing configs use it.
    pub fn with_auth(
        base_url: &str,
        username: &str,
        password: &str,
        token: Option<&str>,
    ) -> Result<Self> {
        if base_url.is_empty() {
            bail!("server.url is empty — copy config.example.toml to config.toml");
        }
        let token = token.map(str::trim).filter(|t| !t.is_empty());
        if token.is_none() && username.is_empty() && password.is_empty() {
            bail!(
                "no credentials — set [server] token (preferred) or username and \
                 password in config.toml"
            );
        }
        let http = crate::util::http_client(None).context("building HTTP client")?;
        // The complete header value, so no call site has to know which scheme
        // is in use. Getting that wrong is silent: the server just 401s.
        let auth = match token {
            Some(t) => format!("Bearer {t}"),
            None => format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD
                    .encode(format!("{username}:{password}"))
            ),
        };
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_owned(),
            auth,
        })
    }

    /// Shared HTTP client — reuse it so connections stay pooled.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The complete `Authorization` header value — `Bearer …` or `Basic …`
    /// depending on what was configured. Callers issuing their own requests
    /// (streaming downloads, artwork) send this verbatim rather than
    /// reconstructing a scheme they would have to keep in sync.
    pub fn auth(&self) -> &str {
        &self.auth
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let hint = if status == reqwest::StatusCode::FORBIDDEN {
                "\n  403 usually means the token lacks a scope. This client needs \
                 roms.read and platforms.read."
            } else {
                ""
            };
            bail!("GET {url} -> {status}{hint}\n  {}", body.chars().take(300).collect::<String>());
        }
        resp.json::<T>()
            .await
            .with_context(|| format!("decoding response from {url}"))
    }

    pub async fn me(&self) -> Result<User> {
        self.get_json("/api/users/me").await
    }

    pub async fn platforms(&self) -> Result<Vec<Platform>> {
        self.get_json("/api/platforms").await
    }

    /// One page of ROMs. `platform_id` is sent as `platform_ids` — the API
    /// ignores unknown params silently, and `platform_id` is not a real one.
    ///
    /// `updated_after` (ISO-8601) makes this incremental: after a first full
    /// pull, later syncs return only what changed.
    pub async fn roms(
        &self,
        platform_id: Option<i64>,
        limit: u32,
        offset: u32,
        updated_after: Option<&str>,
    ) -> Result<RomPage> {
        let mut path = format!("/api/roms?limit={limit}&offset={offset}");
        if let Some(id) = platform_id {
            path.push_str(&format!("&platform_ids={id}"));
        }
        if let Some(ts) = updated_after {
            path.push_str(&format!("&updated_after={}", urlencode(ts)));
        }
        self.get_json(&path).await
    }

    /// One ROM with its member files.
    ///
    /// Folder ROMs need this: the rom-level hash is a composite computed in
    /// filesystem order on the server and cannot be reproduced elsewhere, but
    /// each member carries its own md5, which verifies precisely.
    pub async fn rom_with_files(&self, id: i64) -> Result<Rom> {
        self.get_json(&format!("/api/roms/{id}?with_files=true")).await
    }

    /// `(file_name, md5)` for each member of a folder ROM.
    ///
    /// Tolerant by design: this only ever *strengthens* verification, so a
    /// server that cannot answer costs us the per-file check rather than the
    /// download. (`/api/roms/{id}/files` returns 500 on 5.0.0; the rom
    /// endpoint with `with_files=true` is the working route.)
    pub async fn member_hashes(&self, id: i64) -> Vec<(String, String)> {
        match self.rom_with_files(id).await {
            Ok(rom) => rom
                .files
                .into_iter()
                .filter_map(|f| Some((f.file_name, f.md5_hash?)))
                .filter(|(n, m)| !n.is_empty() && !m.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Every ROM id the server currently has.
    ///
    /// Cheap (one array of ints for ~10k roms) and the only way to notice
    /// deletions: `updated_after` reports changes, never removals.
    pub async fn rom_identifiers(&self) -> Result<Vec<i64>> {
        self.get_json("/api/roms/identifiers").await
    }

    /// Server settings that affect how we interpret its data.
    pub async fn config(&self) -> Result<ServerConfig> {
        let raw: RawConfig = self.get_json("/api/config").await?;
        Ok(ServerConfig {
            default_excluded_files: raw.files,
            default_excluded_extensions: raw.exts,
            skip_hash_calculation: raw.skip_hash,
        })
    }

    /// Unauthenticated; also the cheapest reachability check.
    pub async fn heartbeat(&self) -> Result<Heartbeat> {
        self.get_json("/api/heartbeat").await
    }

    /// Total ROM count, without pulling any rows we don't need.
    pub async fn rom_count(&self) -> Result<i64> {
        Ok(self.roms(None, 1, 0, None).await?.total)
    }
}

/// Minimal percent-encoding for query values (timestamps contain `:` and `+`).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A RomM collection: hand-made, smart (a saved filter), or virtual
/// (auto-grouped by genre, franchise, company…).
///
/// One struct covers all three because the server returns the same shape for
/// each; only `is_virtual`/`is_smart` and the presence of `type` differ.
#[derive(Debug, Clone, Deserialize)]
pub struct Collection {
    /// Hand-made collections have a numeric id, virtual ones a base64 string.
    /// Kept as text so the two can share a table.
    #[serde(deserialize_with = "id_as_string")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub rom_ids: Vec<i64>,
    #[serde(default)]
    pub rom_count: i64,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub is_virtual: bool,
    #[serde(default)]
    pub is_smart: bool,
    /// Virtual collections only: `genre`, `franchise`, `company`, …
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// Covers of the first few members, for a mosaic thumbnail.
    #[serde(default)]
    pub path_covers_small: Vec<String>,
}

impl Collection {
    /// Which of the three kinds this is, as a stable string for the cache.
    pub fn group(&self) -> &str {
        match () {
            _ if self.is_virtual => self.kind.as_deref().unwrap_or("virtual"),
            _ if self.is_smart => "smart",
            _ => "user",
        }
    }
}

/// Accepts either a JSON number or string, since the two collection families
/// disagree on the type of `id`.
fn id_as_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    })
}

impl Client {
    /// Hand-made collections.
    pub async fn collections(&self) -> Result<Vec<Collection>> {
        self.get_json("/api/collections").await
    }

    /// Saved-filter collections.
    pub async fn smart_collections(&self) -> Result<Vec<Collection>> {
        self.get_json("/api/collections/smart").await
    }

    /// Auto-grouped collections of one kind.
    pub async fn virtual_collections(&self, kind: &str) -> Result<Vec<Collection>> {
        self.get_json(&format!("/api/collections/virtual?type={kind}")).await
    }

    /// Which virtual kinds this server actually has.
    ///
    /// Read from the identifiers list rather than hard-coded, so a RomM that
    /// grows a new kind appears without a client change. Each identifier is
    /// base64 of `{"name": ..., "type": ...}`.
    pub async fn virtual_kinds(&self) -> Result<Vec<String>> {
        let ids: Vec<String> = self.get_json("/api/collections/virtual/identifiers").await?;
        let mut kinds: Vec<String> = Vec::new();
        for id in ids {
            // Pad: the server emits unpadded base64, and both alphabets occur.
            let padded = format!("{id}{}", "=".repeat((4 - id.len() % 4) % 4));
            let Some(raw) = base64::engine::general_purpose::URL_SAFE
                .decode(&padded)
                .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&padded))
                .ok()
            else {
                continue;
            };
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw)
                && let Some(k) = v.get("type").and_then(|t| t.as_str())
                && !kinds.iter().any(|s| s == k)
            {
                kinds.push(k.to_owned());
            }
        }
        Ok(kinds)
    }

    /// Every collection the server has, of all three families.
    ///
    /// Tolerant per family: a server with smart collections disabled, or a
    /// virtual kind that errors, costs that group rather than the whole sync.
    /// Ask the server to identify a game against its metadata sources.
    ///
    /// The same call RomM's own web interface makes when it offers you match
    /// candidates, and the reason this app needs no ScreenScraper credentials:
    /// the server holds them and hands back ready-made media URLs for whichever
    /// sources it has enabled.
    ///
    /// `rom_id` matters as much as the name — the server has the file's hashes
    /// and matches on those first, which is how an arcade romset called
    /// `pkscram.zip` finds its real title.
    /// The ScreenScraper media the server already holds for a game.
    pub async fn rom_media(&self, rom_id: i64) -> Result<SsMedia> {
        let rom: RomMedia = self.get_json(&format!("/api/roms/{rom_id}")).await?;
        Ok(rom.ss_metadata.unwrap_or_default())
    }

    /// Tell the server which ScreenScraper game a file is.
    ///
    /// The server then resolves and stores ScreenScraper's whole media set for
    /// it — the same thing RomM's own "match" button does, and the only way to
    /// get more than a single box for a game it identified from somewhere else.
    /// Reversible from RomM with `unmatch_metadata`.
    ///
    /// `Ok(false)` means the token is not allowed to write. That is not an
    /// error worth stopping a run for: the caller falls back to the one URL it
    /// can get without writing, and says so once at the end rather than 2,700
    /// times.
    pub async fn set_screenscraper_match(&self, rom_id: i64, ss_id: i64) -> Result<bool> {
        let form = reqwest::multipart::Form::new().text("ss_id", ss_id.to_string());
        let resp = self
            .http
            .put(format!("{}/api/roms/{rom_id}", self.base))
            .header("Authorization", self.auth())
            .multipart(form)
            .send()
            .await
            .with_context(|| format!("matching rom {rom_id} to ScreenScraper {ss_id}"))?;

        if resp.status() == reqwest::StatusCode::FORBIDDEN
            || resp.status() == reqwest::StatusCode::UNAUTHORIZED
        {
            return Ok(false);
        }
        if !resp.status().is_success() {
            bail!("PUT /api/roms/{rom_id} -> {}", resp.status());
        }
        Ok(true)
    }

    /// Replace a game's cover on the server with an image we hold.
    ///
    /// So the RomM web interface, and anything else pointed at the server,
    /// shows the same picture this app does rather than whatever earlier
    /// scrape happened to win. The server re-encodes what it is given.
    ///
    /// `Ok(false)` for a refusal, matching `set_screenscraper_match`: a token
    /// without write permission should slow nothing down and stop nothing.
    pub async fn upload_cover(&self, rom_id: i64, file_name: &str, bytes: Vec<u8>) -> Result<bool> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name.to_owned())
            .mime_str("image/png")?;
        let resp = self
            .http
            .put(format!("{}/api/roms/{rom_id}", self.base))
            .header("Authorization", self.auth())
            .multipart(reqwest::multipart::Form::new().part("artwork", part))
            .send()
            .await
            .with_context(|| format!("uploading cover for rom {rom_id}"))?;

        if resp.status() == reqwest::StatusCode::FORBIDDEN
            || resp.status() == reqwest::StatusCode::UNAUTHORIZED
        {
            return Ok(false);
        }
        if !resp.status().is_success() {
            bail!("PUT /api/roms/{rom_id} (artwork) -> {}", resp.status());
        }
        Ok(true)
    }

    pub async fn identify(&self, rom_id: i64, name: &str) -> Result<Vec<Match>> {
        let path = format!(
            "/api/search/roms?rom_id={rom_id}&search_by=name&search_term={}",
            urlencode(name)
        );
        self.get_json(&path).await
    }

    pub async fn all_collections(&self) -> Result<Vec<Collection>> {
        let mut out = self.collections().await.unwrap_or_default();
        out.extend(self.smart_collections().await.unwrap_or_default());
        for kind in self.virtual_kinds().await.unwrap_or_default() {
            out.extend(self.virtual_collections(&kind).await.unwrap_or_default());
        }
        Ok(out)
    }

    // --- Changing a collection ----------------------------------------------
    //
    // Only hand-made collections can be changed. Smart ones are a stored
    // filter and virtual ones are grouped on the fly, so the server has
    // nowhere to record a membership for either — adding to one is not a thing
    // that can succeed, and `favorites` refuses before it asks.

    /// Put games into a collection. Already-present ids are not an error.
    pub async fn add_roms_to_collection(&self, id: &str, rom_ids: &[i64]) -> Result<()> {
        self.edit_collection_roms(reqwest::Method::POST, id, rom_ids).await
    }

    /// Take games out of a collection. Absent ids are not an error.
    pub async fn remove_roms_from_collection(&self, id: &str, rom_ids: &[i64]) -> Result<()> {
        self.edit_collection_roms(reqwest::Method::DELETE, id, rom_ids).await
    }

    /// Both membership calls: same path, same body, opposite verbs.
    ///
    /// An empty list returns without asking. The server accepts it, but a
    /// no-op round trip per unchanged game is what makes a sync of two hundred
    /// games feel broken.
    async fn edit_collection_roms(
        &self,
        method: reqwest::Method,
        id: &str,
        rom_ids: &[i64],
    ) -> Result<()> {
        if rom_ids.is_empty() {
            return Ok(());
        }
        let url = format!("{}/api/collections/{}/roms", self.base, urlencode(id));
        let resp = self
            .http
            .request(method.clone(), &url)
            .header("Authorization", &self.auth)
            .json(&serde_json::json!({ "rom_ids": rom_ids }))
            .send()
            .await
            .with_context(|| format!("{method} {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let hint = if status == reqwest::StatusCode::FORBIDDEN {
                "\n  403 here usually means the token lacks collections.write."
            } else {
                ""
            };
            bail!(
                "{method} {url} -> {status}{hint}\n  {}",
                body.chars().take(300).collect::<String>()
            );
        }
        Ok(())
    }

    /// Make a new hand-made collection and return it.
    ///
    /// Multipart rather than JSON because that is what the endpoint declares —
    /// it shares a handler with the one that takes a cover image.
    pub async fn create_collection(&self, name: &str, is_favorite: bool) -> Result<Collection> {
        let url = format!(
            "{}/api/collections?is_favorite={}",
            self.base,
            if is_favorite { "true" } else { "false" }
        );
        let form = reqwest::multipart::Form::new().text("name", name.to_owned());
        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.auth)
            .multipart(form)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!(
                "POST {url} -> {status}\n  {}",
                body.chars().take(300).collect::<String>()
            );
        }
        resp.json().await.with_context(|| format!("decoding {url}"))
    }
}

// --- Saves and sync ---------------------------------------------------------
//
// The protocol, read out of the running server (`/backend/endpoints/saves.py`)
// rather than inferred from the spec, which documents none of it:
//
// * `POST /api/saves` with `overwrite=false` (the default) returns **409** when
//   the server's copy changed since this device last synced. That is real
//   conflict detection, so we always upload with overwrite off and surface the
//   409 rather than clobbering another machine's progress.
// * The server dedupes on `content_hash`: an identical save is discarded and
//   the existing record returned. This is why `savehash.rs` had to reproduce
//   RomM's hash byte-for-byte — get it wrong and every save re-uploads forever.
// * `autocleanup` keeps only the newest `autocleanup_limit` saves per
//   (rom, slot) and deletes the rest. Only applies when a slot is given.
// * `/api/sync/negotiate` covers **saves only** — its payload has no states
//   array. States have their own endpoints and no negotiation.

/// A device registered with the server; sync is scoped to one.
///
/// `POST /api/devices` answers with `device_id` while `GET /api/devices` lists
/// `id`, so both spellings are accepted rather than silently deserialising to
/// nothing.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    #[serde(alias = "device_id")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Save {
    pub id: i64,
    pub rom_id: i64,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub file_size_bytes: i64,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub emulator: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// What the client believes it has, for one save.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientSaveState {
    pub rom_id: i64,
    pub file_name: String,
    pub slot: Option<String>,
    pub emulator: Option<String>,
    pub content_hash: String,
    pub updated_at: String,
    pub file_size_bytes: i64,
}

/// What the server tells us to do about one save.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SyncOperation {
    /// `upload`, `download`, `conflict` or `no_op`.
    pub action: String,
    pub rom_id: i64,
    #[serde(default)]
    pub save_id: Option<i64>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub emulator: Option<String>,
    /// Why the server chose this action — worth showing verbatim.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub server_content_hash: Option<String>,
    #[serde(default)]
    pub server_updated_at: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SyncPlan {
    #[serde(default)]
    pub session_id: Option<i64>,
    #[serde(default)]
    pub operations: Vec<SyncOperation>,
    #[serde(default)]
    pub total_upload: i64,
    #[serde(default)]
    pub total_download: i64,
    #[serde(default)]
    pub total_conflict: i64,
    #[serde(default)]
    pub total_no_op: i64,
}

/// Upload rejected because the server's copy moved on since our last sync.
#[derive(Debug)]
pub struct Conflict {
    pub detail: String,
}

impl Client {
    async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.auth)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("POST {url} -> {status}\n  {}", body.chars().take(300).collect::<String>());
        }
        resp.json::<T>().await.with_context(|| format!("decoding {url}"))
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        self.get_json("/api/devices").await
    }

    /// Register this machine, or return its existing registration.
    ///
    /// **`hostname` is load-bearing.** The server deduplicates on a fingerprint
    /// of `(mac_address, hostname, platform)` — verified in
    /// `/backend/endpoints/device.py`. It ignores `client_device_identifier`
    /// on create entirely. Registering without a hostname fingerprints as
    /// all-nulls, matches nothing, and mints a brand new device every call;
    /// each new device starts with empty sync bookkeeping, so every save then
    /// looks like a first-time upload.
    pub async fn register_device(&self, name: &str, hostname: &str) -> Result<Device> {
        self.post_json(
            "/api/devices",
            &serde_json::json!({
                "name": name,
                "hostname": hostname,
                "platform": std::env::consts::OS,
                "client": "moose-rack",
                "client_version": env!("CARGO_PKG_VERSION"),
                "allow_existing": true,
            }),
        )
        .await
    }

    /// Saves the server holds, optionally for one ROM.
    pub async fn saves(&self, rom_id: Option<i64>) -> Result<Vec<Save>> {
        let path = match rom_id {
            Some(id) => format!("/api/saves?rom_id={id}"),
            None => "/api/saves".to_owned(),
        };
        self.get_json(&path).await
    }

    /// Ask the server what to do, given everything we hold locally.
    pub async fn negotiate(&self, device_id: &str, saves: &[ClientSaveState]) -> Result<SyncPlan> {
        self.post_json(
            "/api/sync/negotiate",
            &serde_json::json!({ "device_id": device_id, "saves": saves }),
        )
        .await
    }

    /// Download one save's bytes.
    pub async fn save_content(&self, save_id: i64, device_id: &str) -> Result<Vec<u8>> {
        let url = format!("{}/api/saves/{save_id}/content?device_id={device_id}", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {url} -> {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }

    /// Tell the server a download landed, so its per-device bookkeeping stays
    /// accurate and the file is not offered again next time.
    pub async fn confirm_download(&self, save_id: i64) -> Result<()> {
        let url = format!("{}/api/saves/{save_id}/downloaded", self.base);
        self.http
            .post(&url)
            .header("Authorization", &self.auth)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        Ok(())
    }

    /// Upload a save. `Ok(Err(Conflict))` means the server refused because its
    /// copy is newer — a real outcome to show the user, not an error.
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_save(
        &self,
        rom_id: i64,
        file_name: &str,
        bytes: Vec<u8>,
        slot: Option<&str>,
        emulator: Option<&str>,
        device_id: &str,
        session_id: Option<i64>,
        overwrite: bool,
    ) -> Result<std::result::Result<Save, Conflict>> {
        let mut url = format!("/api/saves?rom_id={rom_id}&device_id={device_id}");
        // Overwrite is off for ordinary uploads: refusing is what makes the
        // server detect a conflict at all. It goes on only to carry out a
        // decision the user has already been shown and made.
        if overwrite {
            url.push_str("&overwrite=true");
        }
        if let Some(s) = slot {
            url.push_str(&format!("&slot={}", urlencode(s)));
        }
        if let Some(e) = emulator {
            url.push_str(&format!("&emulator={}", urlencode(e)));
        }
        if let Some(s) = session_id {
            url.push_str(&format!("&session_id={s}"));
        }
        let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_owned());
        let form = reqwest::multipart::Form::new().part("saveFile", part);

        let full = format!("{}{}", self.base, url);
        let resp = self
            .http
            .post(&full)
            .header("Authorization", &self.auth)
            .multipart(form)
            .send()
            .await
            .with_context(|| format!("POST {full}"))?;

        if resp.status() == reqwest::StatusCode::CONFLICT {
            let detail = resp.text().await.unwrap_or_default();
            return Ok(Err(Conflict { detail }));
        }
        if !resp.status().is_success() {
            let s = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("POST {full} -> {s}\n  {}", body.chars().take(300).collect::<String>());
        }
        Ok(Ok(resp.json().await.context("decoding uploaded save")?))
    }

    /// Close a sync session so the server stops counting it as in flight.
    pub async fn complete_session(&self, session_id: i64) -> Result<()> {
        let url = format!("{}/api/sync/sessions/{session_id}/complete", self.base);
        self.http
            .post(&url)
            .header("Authorization", &self.auth)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `updated_after` carries an ISO-8601 timestamp, whose `:` and `+` are
    /// both meaningful in a query string. Sending them raw makes the server
    /// read a different instant — or none — and the API ignores parameters it
    /// cannot parse *silently*, so the symptom is a full re-sync every time
    /// rather than an error.
    #[test]
    fn query_values_are_encoded_so_a_timestamp_survives() {
        assert_eq!(
            urlencode("2026-08-06T15:44:55+00:00"),
            "2026-08-06T15%3A44%3A55%2B00%3A00"
        );
        // Unreserved characters must pass through, or every URL grows noise.
        assert_eq!(urlencode("abc-DEF_123.x~y"), "abc-DEF_123.x~y");
        assert_eq!(urlencode("a b"), "a%20b");
    }

    /// A save slot or emulator name reaches the server as a query value too.
    #[test]
    fn slot_and_emulator_names_are_encoded() {
        assert_eq!(urlencode("state 3"), "state%203");
        assert_eq!(urlencode("mame2003-plus"), "mame2003-plus");
    }

    /// An unset server is a configuration problem the user can fix, and saying
    /// so beats a connection error against the empty string.
    #[test]
    fn an_empty_server_url_is_refused_with_advice() {
        // Matched rather than `expect_err`: that needs `Client: Debug`, and
        // deriving one would print the encoded credential in any debug output.
        let err = match Client::new("", "u", "p") {
            Ok(_) => panic!("an empty server url must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("config.toml"), "should say how to fix it: {err}");
    }

    /// A trailing slash in the configured URL would otherwise produce `//api/…`
    /// on every request.
    #[test]
    fn a_trailing_slash_on_the_server_url_is_trimmed() {
        let c = Client::new("http://dev.lan/", "u", "p").unwrap();
        assert_eq!(c.base(), "http://dev.lan");
    }

    /// `auth()` is the complete header value, not a bare credential. Streaming
    /// downloads and artwork fetches send it verbatim, so if it were just the
    /// base64 every one of those call sites would have to hardcode `Basic` —
    /// which is exactly how a bearer token silently keeps sending Basic.
    #[test]
    fn basic_credentials_are_encoded_once_for_reuse() {
        let c = Client::new("http://dev.lan", "user", "pass").unwrap();
        // "user:pass" in base64, as a ready-to-send header value.
        assert_eq!(c.auth(), "Basic dXNlcjpwYXNz");
    }

    /// A configured token wins over username/password, and produces a bearer
    /// header rather than Basic.
    #[test]
    fn a_token_is_preferred_and_sent_as_a_bearer() {
        let c = Client::with_auth("http://dev.lan", "user", "pass", Some("tok_abc")).unwrap();
        assert_eq!(c.auth(), "Bearer tok_abc");

        // Blank or whitespace-only is not a token; fall back to Basic rather
        // than sending "Bearer " and getting a 401 with no explanation.
        for empty in [Some(""), Some("   "), None] {
            let c = Client::with_auth("http://dev.lan", "user", "pass", empty).unwrap();
            assert_eq!(c.auth(), "Basic dXNlcjpwYXNz", "for {empty:?}");
        }
    }

    /// No credential of any kind is a configuration mistake worth naming, not
    /// an empty Basic header that 401s on every request.
    #[test]
    fn no_credentials_at_all_is_refused_with_advice() {
        let err = match Client::with_auth("http://dev.lan", "", "", None) {
            Ok(_) => panic!("must not build a client with no credentials"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("token"), "should mention the token option: {err}");
    }

    /// The three collection families share one struct because the server
    /// returns one shape. `group` is what tells them apart in the cache, and
    /// virtual has to win: a virtual collection can also carry `is_smart`.
    #[test]
    fn collection_grouping_puts_virtual_before_smart_before_user() {
        let parse = |s: &str| serde_json::from_str::<Collection>(s).unwrap();

        assert_eq!(parse(r#"{"id": 1, "name": "Mine"}"#).group(), "user");
        assert_eq!(
            parse(r#"{"id": 2, "name": "Filter", "is_smart": true}"#).group(),
            "smart"
        );
        assert_eq!(
            parse(r#"{"id": "x", "name": "RPG", "is_virtual": true, "type": "genre"}"#).group(),
            "genre"
        );
        assert_eq!(
            parse(r#"{"id": "x", "name": "Both", "is_virtual": true, "is_smart": true,
                      "type": "franchise"}"#)
                .group(),
            "franchise",
            "virtual wins, and takes its name from `type`"
        );
        // A virtual collection with no type still has to land somewhere.
        assert_eq!(
            parse(r#"{"id": "x", "name": "Odd", "is_virtual": true}"#).group(),
            "virtual"
        );
    }

    /// Absent fields must default rather than fail the whole page: one rom with
    /// no cover would otherwise cost the entire sync.
    #[test]
    fn a_sparse_rom_still_deserialises() {
        let rom: Rom = serde_json::from_str(r#"{"id": 1, "fs_name": "game.zip"}"#).unwrap();
        assert_eq!(rom.fs_name, "game.zip");
        assert!(rom.name.is_none());
        assert!(!rom.has_multiple_files);
        assert!(rom.merged_screenshots.is_empty());
        assert!(rom.files.is_empty());
    }

    /// `POST /api/devices` answers with `device_id` while `GET /api/devices`
    /// lists `id`. Accepting only one spelling deserialises to nothing and the
    /// device is re-registered on every sync.
    #[test]
    fn a_device_is_read_under_either_id_spelling() {
        let created: Device =
            serde_json::from_str(r#"{"device_id": "abc", "name": "mac"}"#).unwrap();
        assert_eq!(created.id, "abc");
        let listed: Device = serde_json::from_str(r#"{"id": "abc"}"#).unwrap();
        assert_eq!(listed.id, "abc");
    }

    /// The server's config keys are SCREAMING_CASE and nothing like our field
    /// names; a rename here silently empties the exclusion lists and changes
    /// how every archive hashes.
    #[test]
    fn server_config_maps_from_its_screaming_case_keys() {
        let raw = r#"{"DEFAULT_EXCLUDED_FILES": ["gamelist.xml"],
                      "DEFAULT_EXCLUDED_EXTENSIONS": ["db"],
                      "SKIP_HASH_CALCULATION": true}"#;
        let parsed: RawConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.files, ["gamelist.xml"]);
        assert_eq!(parsed.exts, ["db"]);
        assert!(parsed.skip_hash);
    }
}

// --- Save states ------------------------------------------------------------
//
// A separate family of endpoints from `/api/saves`, and deliberately simpler:
// no slot, no device_id, no negotiate. `/api/sync/negotiate` covers saves only
// — its payload has no states array — so the comparison for these has to be
// done client-side. See `crate::statesync`.

/// One save state on the server.
///
/// Note the absence of `content_hash`: unlike a save, the server publishes no
/// digest for a state, so "has this changed" has to be answered from size and
/// timestamp instead.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SaveState {
    pub id: i64,
    pub rom_id: i64,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub file_size_bytes: i64,
    #[serde(default)]
    pub emulator: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl Client {
    /// States the server holds for one ROM.
    pub async fn states(&self, rom_id: i64) -> Result<Vec<SaveState>> {
        self.get_json(&format!("/api/states?rom_id={rom_id}")).await
    }

    /// Download one state's bytes.
    pub async fn state_content(&self, state_id: i64) -> Result<Vec<u8>> {
        let url = format!("{}/api/states/{state_id}/content", self.base);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {url} -> {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }

    /// Upload a state. There is no overwrite flag and no conflict response —
    /// the server takes what it is given, which is why the decision about
    /// whether to send has to be made before calling this.
    pub async fn upload_state(
        &self,
        rom_id: i64,
        file_name: &str,
        bytes: Vec<u8>,
        emulator: Option<&str>,
    ) -> Result<SaveState> {
        let mut url = format!("/api/states?rom_id={rom_id}");
        if let Some(e) = emulator {
            url.push_str(&format!("&emulator={}", urlencode(e)));
        }
        let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_owned());
        let form = reqwest::multipart::Form::new().part("stateFile", part);

        let full = format!("{}{}", self.base, url);
        let resp = self
            .http
            .post(&full)
            .header("Authorization", &self.auth)
            .multipart(form)
            .send()
            .await
            .with_context(|| format!("POST {full}"))?;
        if !resp.status().is_success() {
            let st = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("POST {full} -> {st}\n  {}", body.chars().take(300).collect::<String>());
        }
        resp.json().await.context("decoding uploaded state")
    }
}

// --- Firmware (BIOS) --------------------------------------------------------
//
// RomM keeps the BIOS set alongside the games, which is what makes a second
// machine cheap to set up: the same server that has the ROMs has the files
// Neo Geo, PlayStation and the MAME family refuse to start without.
//
// Needs the `firmware.read` scope on the token.

/// One BIOS file on the server.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Firmware {
    pub id: i64,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub file_size_bytes: i64,
    #[serde(default)]
    pub md5_hash: Option<String>,
    #[serde(default)]
    pub sha1_hash: Option<String>,
    /// Where it sits on the server, e.g. `bios/3do`. Not reproduced locally —
    /// RetroArch wants every BIOS flat in one system directory.
    #[serde(default)]
    pub file_path: Option<String>,
    /// The server checked it against a known-good hash.
    #[serde(default)]
    pub is_verified: bool,
}

impl Client {
    /// Every BIOS the server holds.
    pub async fn firmware(&self) -> Result<Vec<Firmware>> {
        self.get_json("/api/firmware").await
    }

    /// One BIOS file's bytes.
    pub async fn firmware_content(&self, id: i64, file_name: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/api/firmware/{id}/content/{}",
            self.base,
            urlencode(file_name)
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let hint = if status == reqwest::StatusCode::FORBIDDEN {
                "\n  403 usually means the token lacks the firmware.read scope."
            } else {
                ""
            };
            bail!("GET {url} -> {status}{hint}");
        }
        Ok(resp.bytes().await?.to_vec())
    }
}

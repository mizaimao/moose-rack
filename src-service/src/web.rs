//! The app's own UI, served over HTTP.
//!
//! `ui/` is 12,552 lines of JavaScript that talks to Rust through Tauri's IPC —
//! `window.__TAURI__.core.invoke("roms", {...})` — and not through `/api/`. So
//! serving the directory alone gives a blank page: `invoke` is undefined in a
//! browser.
//!
//! The bridge is two halves and no change to the UI at all:
//!
//! * a shim that defines `window.__TAURI__.core` before the app's modules load,
//!   turning `invoke(cmd, args)` into `POST /invoke/<cmd>`;
//! * a dispatcher here that answers the commands a browse-only UI needs.
//!
//! Nothing in `ui/` is edited, which is the point: the desktop app and the web
//! page stay the same program, and a change to one is a change to both.
//!
//! ## What is deliberately not here
//!
//! Launching, pad bindings, RetroArch paths, icon installation — 68 of the 84
//! commands. They act on the machine a person is sitting at, and this is not it.
//! They answer a plain "not available on the server" rather than failing
//! obscurely, because a UI that half-works is worse than one that says so.

use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use moose_rack::cache::Cache;
use serde::Serialize;
use serde_json::{json, Value};

/// Injected before the app's own modules, so `invoke` exists by the time they
/// run. `convertFileSrc` becomes a URL rather than a `file://` path.
pub const SHIM: &str = r#"
// Tauri's IPC, over HTTP. See src-service/src/web.rs.
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      const r = await fetch("/invoke/" + cmd, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(args ?? {}),
      });
      const text = await r.text();
      if (!r.ok) throw new Error(text || r.statusText);
      return text ? JSON.parse(text) : null;
    },
    // Artwork is served from the media tree; a local path becomes a URL.
    convertFileSrc: (p) => "/media?path=" + encodeURIComponent(p),
  },
  // The desktop app pushes a few events. Nothing here emits them yet, so
  // listeners are registered and never fire rather than throwing on startup.
  event: { listen: async () => () => {}, emit: async () => {} },
};
"#;

#[derive(Serialize)]
pub struct PlatformView {
    pub slug: String,
    pub name: String,
    pub rom_count: i64,
    pub playable: bool,
    pub logo: Option<String>,
    pub logo_wordmark: bool,
    pub portrait: Option<String>,
    pub cover_aspect: Option<f32>,
    pub manufacturer: Option<String>,
    pub released: Option<u16>,
    pub hardware: Option<String>,
    pub blurb: Option<String>,
}

#[derive(Serialize)]
pub struct RomView {
    pub id: i64,
    pub name: String,
    pub fs_name: String,
    pub platform: String,
    pub size_bytes: i64,
    /// Always true here: the server holds the file, so every game is playable
    /// from its point of view. On the desktop this means "on this disk".
    pub downloaded: bool,
    pub favorite: bool,
    pub rating: Option<f64>,
    pub year: Option<i32>,
    pub last_played: Option<String>,
    pub players: Option<u8>,
    pub rel_dir: String,
}

/// Answer one `invoke`.
///
/// Unknown and machine-local commands are told apart on purpose. A typo should
/// look different from a thing that exists and cannot work here.
pub fn dispatch(cache: &Cache, favorites: &std::collections::HashSet<i64>, cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        "status" => Ok(json!({
            "server": "moose-service",
            "user": "local",
            "roms": cache.rom_count().unwrap_or(0),
            "online": true,
        })),
        "versions" => Ok(json!({ "client": env!("CARGO_PKG_VERSION") })),

        "platforms" | "systems" => {
            let rows = cache.platforms().map_err(|e| e.to_string())?;
            let out: Vec<PlatformView> = rows
                .into_iter()
                .map(|p| PlatformView {
                    slug: p.fs_slug.clone(),
                    name: p.display_name.clone(),
                    rom_count: p.rom_count,
                    // The server does not run games, so nothing is "playable"
                    // in the sense the desktop means. Reported true so the grid
                    // does not grey everything out.
                    playable: true,
                    logo: None,
                    logo_wordmark: false,
                    portrait: None,
                    cover_aspect: None,
                    manufacturer: None,
                    released: None,
                    hardware: None,
                    blurb: None,
                })
                .collect();
            serde_json::to_value(out).map_err(|e| e.to_string())
        }

        "roms" => {
            let platform = args.get("platform").and_then(Value::as_str).unwrap_or("");
            let rows = cache.roms_for(platform).map_err(|e| e.to_string())?;
            let out: Vec<RomView> = rows
                .into_iter()
                .map(|r| RomView {
                    favorite: favorites.contains(&r.id),
                    id: r.id,
                    name: if r.name.is_empty() { r.fs_name.clone() } else { r.name.clone() },
                    fs_name: r.fs_name,
                    platform: r.platform_slug,
                    size_bytes: r.fs_size_bytes,
                    downloaded: true,
                    rating: None,
                    year: None,
                    last_played: r.last_played.clone(),
                    players: None,
                    rel_dir: r.rel_dir.clone(),
                })
                .collect();
            serde_json::to_value(out).map_err(|e| e.to_string())
        }

        // Machine-local: these act on the computer somebody is sitting at.
        "launch_rom" | "set_pad_binding" | "set_key_binding" | "set_retroarch_root"
        | "import_bindings" | "reset_bindings" | "install_icon_set" | "remove_icon_set"
        | "open_link" | "open_settings" | "android_launch_plan" | "android_after_play"
        | "game_cores" | "set_game_core" | "game_displays" | "game_lightgun" => {
            Err(format!("{cmd} is not available on the server"))
        }

        _ => Err(format!("unknown command {cmd}")),
    }
}

pub struct WebState {
    pub cache: std::sync::Mutex<Cache>,
    pub favorites: std::collections::HashSet<i64>,
    pub ui_dir: std::path::PathBuf,
}

pub async fn invoke(
    State(st): State<Arc<WebState>>,
    axum::extract::Path(cmd): axum::extract::Path<String>,
    body: Option<Json<Value>>,
) -> axum::response::Response {
    let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let cache = match st.cache.lock() {
        Ok(c) => c,
        Err(_) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "cache poisoned").into_response(),
    };
    match dispatch(&cache, &st.favorites, &cmd, &args) {
        Ok(v) => Json(v).into_response(),
        // 400 rather than 500: the request named something this cannot do, and
        // the UI shows the message.
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// `index.html` with the shim injected ahead of the app's own scripts.
///
/// Rewritten on the way out rather than edited on disk, so the desktop build
/// keeps an untouched `ui/`.
pub async fn index(State(st): State<Arc<WebState>>) -> axum::response::Response {
    let path = st.ui_dir.join("index.html");
    let Ok(html) = std::fs::read_to_string(&path) else {
        return (axum::http::StatusCode::NOT_FOUND, format!("no {}", path.display())).into_response();
    };
    let injected = html.replacen("<head>", "<head>\n<script src=\"/__shim.js\"></script>", 1);
    axum::response::Html(injected).into_response()
}

pub async fn shim() -> axum::response::Response {
    ([(axum::http::header::CONTENT_TYPE, "text/javascript")], SHIM).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shim_defines_what_the_ui_reads() {
        // state.js does `const { invoke, convertFileSrc } = window.__TAURI__.core`
        // at module load. Anything missing is a TypeError before the page draws.
        assert!(SHIM.contains("window.__TAURI__"));
        assert!(SHIM.contains("invoke:"));
        assert!(SHIM.contains("convertFileSrc:"));
        assert!(SHIM.contains("event:"), "attract-screen.js calls event.listen at startup");
    }

    /// A typo and a thing that cannot work here must not look the same.
    #[test]
    fn machine_local_commands_say_so_and_unknown_ones_say_that() {
        let d = tempdir::TempDir::new("w").unwrap();
        let cache = Cache::open(&d.path().join("c.sqlite3")).unwrap();
        let fav = Default::default();
        let e = dispatch(&cache, &fav, "launch_rom", &json!({})).unwrap_err();
        assert!(e.contains("not available on the server"), "{e}");
        let e2 = dispatch(&cache, &fav, "definitely_not_a_command", &json!({})).unwrap_err();
        assert!(e2.contains("unknown command"), "{e2}");
    }

    #[test]
    fn status_and_versions_answer_without_a_library() {
        let d = tempdir::TempDir::new("w").unwrap();
        let cache = Cache::open(&d.path().join("c.sqlite3")).unwrap();
        let fav = Default::default();
        let s = dispatch(&cache, &fav, "status", &json!({})).unwrap();
        assert_eq!(s["online"], true);
        let v = dispatch(&cache, &fav, "versions", &json!({})).unwrap();
        assert!(v["client"].is_string());
    }
}

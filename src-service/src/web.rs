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
//! * a dispatcher here that hands each command to `moose_rack::commands`.
//!
//! Nothing in `ui/` is edited, which is the point: the desktop app and the web
//! page stay the same program, and a change to one is a change to both.
//!
//! ## What is deliberately not here
//!
//! Launching, ROM downloads, library sync, the Android hand-off and the app's
//! own dock icon. They act on the machine a person is sitting at, and this is
//! not it. They answer a plain "not available on the server" rather than failing
//! obscurely, because a UI that half-works is worse than one that says so.

use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use moose_rack::app::AppState;
use serde_json::{json, Value};

/// Injected before the app's own modules, so `invoke` exists by the time they
/// run. `convertFileSrc` becomes a URL rather than a `file://` path.
///
/// Two commands are answered here rather than over the wire. `open_settings`
/// and `open_link` open a window on the machine that runs the *backend*, which
/// in a browser is the wrong machine entirely — a click here would raise a
/// window on the server, in a room nobody is in. The browser's own `window.open`
/// is the same act on the right machine, so the adapter does it. That is not a
/// second implementation: there is nothing of the library in either.
pub const SHIM: &str = r#"
// Tauri's IPC, over HTTP. See src-service/src/web.rs.
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      // Opening a window belongs to whichever machine has the screen.
      if (cmd === "open_settings") { window.open("/settings.html", "_blank"); return null; }
      if (cmd === "open_link") { window.open(args?.url, "_blank", "noopener"); return null; }
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
  // Tauri's cross-window events, over BroadcastChannel.
  //
  // Settings is a separate window on the desktop and a separate tab here, and
  // the two talk: the shader backdrop's switch, its frame rate, the glass tint
  // and strength, and the collection art all live in Settings while the thing
  // they change is drawn by the library page. Stubbed out, every one of those
  // silently did nothing -- you turn the backdrop on and the library goes on
  // looking exactly as it did.
  //
  // BroadcastChannel is the same thing for same-origin documents, so `ui/` is
  // unchanged. It does not deliver to the sender, where Tauri does; the one
  // place that matters, `setBackdropWanted`, already acts on its own window
  // directly, and not looping back is the safer of the two differences.
  event: (() => {
    const chan = "BroadcastChannel" in window ? new BroadcastChannel("moose-rack") : null;
    const handlers = new Map();
    chan?.addEventListener("message", (m) => {
      const { event, payload } = m.data || {};
      for (const fn of handlers.get(event) ?? []) {
        try { fn({ event, payload }); } catch (e) { console.warn(event, e); }
      }
    });
    return {
      listen: async (event, fn) => {
        if (!handlers.has(event)) handlers.set(event, new Set());
        handlers.get(event).add(fn);
        return () => handlers.get(event)?.delete(fn);
      },
      emit: async (event, payload) => chan?.postMessage({ event, payload }),
    };
  })(),
};
"#;

/// Tauri's own argument convention: JavaScript sends `localOnly`, Rust names it
/// `local_only`. The desktop build gets that translation from the `#[command]`
/// macro. Doing it here rather than per command is what lets an arm be one line.
fn snake(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Every key of a flat argument object, renamed. Values are untouched: a
/// `ListRef` inside has its own serde names and is not ours to rewrite.
fn normalise(args: &Value) -> Value {
    match args.as_object() {
        Some(map) => Value::Object(
            map.iter().map(|(k, v)| (snake(k), v.clone())).collect(),
        ),
        None => args.clone(),
    }
}

/// Answer one `invoke` by calling the shared backend.
///
/// There is no logic here on purpose. Every arm is
/// `moose_rack::commands::<name>` -- the same function the desktop window calls
/// through its Tauri wrapper -- so the two cannot drift. A handler written here
/// would be a second implementation of something that already exists.
///
/// `j!` serialises a `CmdResult`, `v!` a plain return value. A shared closure
/// would be monomorphised to the first command's type and refuse every other
/// one, and `p!` reads one argument by the name the UI sends it under.
pub async fn dispatch(state: &AppState, cmd: &str, args: &Value) -> Result<Value, String> {
    use moose_rack::commands as c;
    let args = normalise(args);
    macro_rules! j {
        ($e:expr) => {
            $e.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
        };
    }
    macro_rules! v {
        ($e:expr) => {
            serde_json::to_value($e).map_err(|e| e.to_string())
        };
    }
    // A missing key is `null`, which is what `Option<T>` deserialises from, so
    // optional arguments need no special case and a required one names itself
    // in the error rather than arriving as a zero.
    macro_rules! p {
        ($k:expr) => {
            serde_json::from_value(args.get($k).cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("{}: {e}", $k))?
        };
    }
    match cmd {
        // Library
        "status" => j!(c::status(state)),
        "versions" => j!(c::versions(state)),
        "platforms" => j!(c::platforms(state)),
        "systems" => j!(c::systems(state)),
        "roms" => j!(c::roms(state, p!("platform"), p!("list"))),
        "recent_games" => j!(c::recent_games(state, p!("limit"), p!("list"))),
        "search" => j!(c::search(state, p!("term"), p!("list"))),
        "collection_groups" => j!(c::collection_groups(state)),
        "collections_in" => j!(c::collections_in(state, p!("group"))),
        "collection_roms" => j!(c::collection_roms(state, p!("id"), p!("list"))),
        "play_history" => j!(c::play_history(state)),
        "attract_pool" => j!(c::attract_pool(state).await),

        // One game
        "rom_detail" => j!(c::rom_detail(state, p!("id")).await),
        "toggle_favorite" => j!(c::toggle_favorite(state, p!("id")).await),
        "game_states" => j!(c::game_states(state, p!("id"))),
        "delete_state" => j!(c::delete_state(state, p!("id"), p!("slot"))),
        "confirm_delete_state" => j!(c::confirm_delete_state()),
        "game_cores" => j!(c::game_cores(state, p!("id"))),
        "set_game_core" => j!(c::set_game_core(state, p!("id"), p!("core"))),
        "game_lightgun" => j!(c::game_lightgun(state, p!("id"))),
        "game_displays" => j!(c::game_displays()),
        "game_video" => j!(c::game_video(state, p!("id")).await),

        // Artwork. The desktop build passes a closure that emits `covers-ready`
        // so the grid fills as they land; one JSON response has nothing to fill
        // progressively, so this one does nothing.
        "rom_covers" => j!(c::rom_covers(state, p!("ids"), p!("local_only"), &|_| {}).await),
        "warm_media" => j!(c::warm_media(state, p!("platform")).await),

        // Settings
        "config_fields" => j!(c::config_fields()),
        "set_config_field" => j!(c::set_config_field(p!("field"), p!("value"))),
        "config_findings" => j!(c::config_findings()),
        "config_patch" => j!(c::config_patch()),
        "list_art_options" => j!(c::list_art_options(state)),
        "set_list_art" => j!(c::set_list_art(state, p!("value"))),
        "motion_options" => j!(c::motion_options(state)),
        "set_motion_shader" => j!(c::set_motion_shader(state, p!("value"))),
        "set_autofire_hz" => j!(c::set_autofire_hz(state, p!("hz"))),
        "set_system_choice" => {
            j!(c::set_system_choice(state, p!("slug"), p!("field"), p!("value")))
        }
        "icon_styles" => j!(c::icon_styles(state)),
        "set_icon_style" => j!(c::set_icon_style(state, p!("key"))),
        "icon_sets" => j!(c::icon_sets(state).await),
        "set_icon_set" => j!(c::set_icon_set(state, p!("dir"))),
        "remove_icon_set" => j!(c::remove_icon_set(state, p!("dir"))),
        // Fetching console pictures writes into the server's own media tree,
        // which is a thing a server should do -- these were refused for a
        // while only because they sat next to the dock-icon commands, which
        // really are about the machine somebody is sitting at.
        "install_icon_set" => j!(c::install_icon_set(state, p!("dir"), &|_| {}).await),
        "fetch_icons" => j!(c::fetch_icons(state, &|_| {}).await),
        "bios_status" => j!(c::bios_status(state).await),
        "disk_usage" => j!(c::disk_usage(state).await),
        "check_update" => j!(c::check_update(state).await),
        "verify_achievements" => j!(c::verify_achievements().await),
        "verify_server" => {
            j!(c::verify_server(p!("url"), p!("token"), p!("username"), p!("password")).await)
        }

        // Controls. These write the config of whichever machine runs the
        // backend, which for a browser pointed at it is the server's own — the
        // same single-user config the desktop app edits.
        "ui_bindings" => j!(c::ui_bindings(state)),
        "set_key_binding" => j!(c::set_key_binding(state, p!("action"), p!("key"))),
        "set_pad_binding" => j!(c::set_pad_binding(state, p!("action"), p!("index"))),
        "reset_bindings" => j!(c::reset_bindings(state, p!("which"))),
        "import_bindings" => j!(c::import_bindings(state, p!("keys"), p!("pad"))),
        "list_controls" => v!(c::list_controls()),

        // Ordering and layout
        "arrange_list" => j!(c::arrange_list(state, p!("list"))),
        "set_list_order" => {
            j!(c::set_list_order(state, p!("list"), p!("order"), p!("preferred")))
        }
        "cycle_list_order" => j!(c::cycle_list_order(state, p!("list"), p!("delta"))),
        "toggle_list_filter" => j!(c::toggle_list_filter(state, p!("list"), p!("filter"))),
        "clear_list_filters" => j!(c::clear_list_filters(state, p!("list"))),
        "sort_picker" => j!(c::sort_picker(state, p!("kind"), p!("rows"))),
        "picker_controls" => j!(c::picker_controls(state, p!("kind"))),
        "set_picker_order" => j!(c::set_picker_order(state, p!("kind"), p!("order"))),
        "set_page_names" => j!(c::set_page_names(state, p!("names"), p!("groups"))),
        "page_filter" => j!(c::page_filter(state, p!("query"))),
        "grid_uniform" => v!(c::grid_uniform(p!("count"), p!("columns"))),
        "set_grid" => v!(c::set_grid(p!("cards"))),

        // Saves
        "sync_saves_plan" => j!(c::sync_saves_plan(state).await),
        "sync_saves" => j!(c::sync_saves(state).await),
        "resolve_save_conflict" => {
            j!(c::resolve_save_conflict(state, p!("file_name"), p!("keep")).await)
        }

        // Things that act on the machine somebody is sitting at. Worded
        // differently from "unknown" so a typo does not look like a design limit.
        //
        // The downloads and syncs are here for a second reason: they copy a
        // library *from* a server *to* local storage, and this is the server.
        // There is nowhere for them to put anything.
        "launch_rom" | "set_retroarch_root" | "open_link" | "open_settings"
        | "sync_library" | "sync_bios" | "download_rom" | "download_set"
        | "download_estimate" | "scrape_missing" | "app_icons" | "set_app_icon"
        | "android_launch_plan" | "android_sync_before" | "android_after_play" => {
            Err(format!("{cmd} is not available on the server"))
        }

        _ => Err(format!("unknown command {cmd}")),
    }
}

pub struct WebState {
    /// The same state the desktop window holds, built by the same constructor.
    pub state: AppState,
    pub ui_dir: std::path::PathBuf,
}

pub async fn invoke(
    State(st): State<Arc<WebState>>,
    axum::extract::Path(cmd): axum::extract::Path<String>,
    body: Option<Json<Value>>,
) -> axum::response::Response {
    let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    match dispatch(&st.state, &cmd, &args).await {
        Ok(v) => Json(v).into_response(),
        // 400 rather than 500: the request named something this cannot do, and
        // the UI shows the message.
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// The directories `/media` will read from, and nothing else.
///
/// The commands hand the UI absolute paths because the desktop webview can load
/// a file by name. Over HTTP that same path is a request for any file on the
/// server, so it is checked against the roots the artwork actually lives under.
pub fn media_roots(state: &AppState) -> Vec<std::path::PathBuf> {
    let mut roots = vec![state.media_dir.clone(), state.themes_dir.clone()];
    roots.extend(state.esde_media.clone());
    roots.extend(state.theme_root.as_deref().map(std::path::PathBuf::from));
    roots
}

/// Resolve a requested path against the allowed roots, or refuse.
///
/// Both sides are canonicalised first. Comparing the strings as given would let
/// `<media>/../../etc/passwd` through while looking like it starts with the
/// media directory, and would refuse a legitimate path that reached the same
/// file by a symlink the scan itself followed.
pub fn resolve_media(roots: &[std::path::PathBuf], asked: &str) -> Option<std::path::PathBuf> {
    let full = std::fs::canonicalize(asked).ok()?;
    if !full.is_file() {
        return None;
    }
    roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .any(|r| full.starts_with(&r))
        .then_some(full)
}

/// One artwork, video or manual, by the absolute path a command returned.
pub async fn media(
    State(st): State<Arc<WebState>>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    req: axum::http::Request<axum::body::Body>,
) -> axum::response::Response {
    let Some(asked) = q.get("path") else {
        return (axum::http::StatusCode::BAD_REQUEST, "no path").into_response();
    };
    let Some(full) = resolve_media(&media_roots(&st.state), asked) else {
        // One answer for "outside the roots" and for "not there". Telling them
        // apart would report whether an arbitrary path exists on the server.
        return (axum::http::StatusCode::NOT_FOUND, "no such media").into_response();
    };
    // Through `ServeFile` for Range: a video the UI scrubs is a range request,
    // and a 200 to one of those makes the browser refetch the whole file.
    use tower::ServiceExt as _;
    match tower_http::services::ServeFile::new(full).oneshot(req).await {
        Ok(r) => r.into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `index.html` with the shim injected ahead of the app's own scripts.
///
/// Rewritten on the way out rather than edited on disk, so the desktop build
/// keeps an untouched `ui/`.
pub async fn index(State(st): State<Arc<WebState>>) -> axum::response::Response {
    page(&st.ui_dir, "index.html")
}

/// The settings window, which in a browser is a second tab of the same app.
pub async fn settings(State(st): State<Arc<WebState>>) -> axum::response::Response {
    page(&st.ui_dir, "settings.html")
}

fn page(dir: &std::path::Path, name: &str) -> axum::response::Response {
    let path = dir.join(name);
    let Ok(html) = std::fs::read_to_string(&path) else {
        return (axum::http::StatusCode::NOT_FOUND, format!("no {}", path.display()))
            .into_response();
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

    /// The command names this file answers, read out of the match itself.
    fn arms() -> std::collections::BTreeSet<String> {
        let src = include_str!("web.rs");
        let body = &src[src.find("match cmd {").unwrap()..src.find("pub struct WebState").unwrap()];
        let mut out = std::collections::BTreeSet::new();
        for line in body.lines() {
            let l = line.trim();
            // A multi-line pattern continues on lines that open with `|`, and
            // leaving those out is how six refusals looked like omissions.
            if !l.starts_with('"') && !l.starts_with('|') {
                continue;
            }
            // `"a" | "b" => ...`, its continuation lines, and `"a" => ...`.
            let head = l.split("=>").next().unwrap_or("");
            for part in head.split('|') {
                let p = part.trim().trim_matches(|c| c == '"' || c == ' ');
                if !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    out.insert(p.to_owned());
                }
            }
        }
        out
    }

    #[test]
    fn the_shim_defines_what_the_ui_reads() {
        // state.js does `const { invoke, convertFileSrc } = window.__TAURI__.core`
        // at module load. Anything missing is a TypeError before the page draws.
        assert!(SHIM.contains("window.__TAURI__"));
        assert!(SHIM.contains("invoke:"));
        assert!(SHIM.contains("convertFileSrc:"));
        assert!(SHIM.contains("event:"), "attract-screen.js calls event.listen at startup");
        // Not a stub: Settings and the library page are two documents that have
        // to talk, and `ui/test/shim.test.js` proves this one does.
        assert!(SHIM.contains("BroadcastChannel"), "cross-window events are stubbed out");
    }

    /// Opening a window is done by the browser, not asked of the server.
    ///
    /// Sent over the wire these would raise a window on the machine running the
    /// service, which is not the machine looking at the page.
    #[test]
    fn the_shim_opens_windows_itself() {
        assert!(SHIM.contains(r#"cmd === "open_settings""#));
        assert!(SHIM.contains("/settings.html"));
        assert!(SHIM.contains(r#"cmd === "open_link""#));
    }

    /// A typo and a thing that cannot work here must not look the same. Checked
    /// on the source rather than by calling, because building an AppState needs a
    /// real library and this is about wording.
    #[test]
    fn machine_local_commands_say_so_and_unknown_ones_say_that() {
        let src = include_str!("web.rs");
        assert!(src.contains("is not available on the server"));
        assert!(src.contains("unknown command"));
        assert!(src.contains("\"launch_rom\""), "launching must stay desktop-only");
    }

    /// Every arm must call `moose_rack::commands`, or it is a second
    /// implementation of something that already exists.
    #[test]
    fn no_command_is_implemented_here() {
        let src = include_str!("web.rs");
        let body = &src[src.find("match cmd {").unwrap()..src.find("pub struct WebState").unwrap()];
        for line in body.lines() {
            let l = line.trim();
            if l.contains("j!(") || l.contains("v!(") {
                assert!(l.contains("c::"), "arm does not delegate: {l}");
            }
        }
    }

    /// Nothing the UI calls may fall through to "unknown command".
    ///
    /// This is the test that catches the real failure: the page loads, the shim
    /// works, and one tab is empty because a command nobody thought about
    /// answers 400. Read from `ui/` so adding a call there fails here.
    #[test]
    fn every_command_the_ui_invokes_has_an_arm() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("ui/js");
        let mut called = std::collections::BTreeSet::new();
        for e in std::fs::read_dir(&ui).expect("ui/js") {
            let p = e.unwrap().path();
            if p.extension().is_none_or(|x| x != "js") {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            // `invoke("name"` and `invoke(\n  "name"`, both of which occur.
            for (i, _) in src.match_indices("invoke(") {
                let rest = src[i + "invoke(".len()..].trim_start();
                if let Some(r) = rest.strip_prefix('"') {
                    if let Some(end) = r.find('"') {
                        called.insert(r[..end].to_owned());
                    }
                }
            }
        }
        assert!(called.len() > 50, "only found {} invokes; the scan broke", called.len());
        let have = arms();
        let missing: Vec<_> = called.difference(&have).collect();
        assert!(missing.is_empty(), "the UI calls these and dispatch does not answer: {missing:?}");
    }

    /// Tauri renames JS arguments; so must this, or `localOnly` arrives as a
    /// missing `local_only` and every cover is fetched over the network.
    #[test]
    fn arguments_are_renamed_the_way_tauri_renames_them() {
        assert_eq!(snake("localOnly"), "local_only");
        assert_eq!(snake("id"), "id");
        assert_eq!(snake("fileName"), "file_name");
        let n = normalise(&json!({"localOnly": true, "ids": [1, 2]}));
        assert_eq!(n["local_only"], json!(true));
        assert_eq!(n["ids"], json!([1, 2]));
        // Values are left alone: a ListRef has its own field names.
        let nested = normalise(&json!({"list": {"someKey": 1}}));
        assert_eq!(nested["list"]["someKey"], json!(1));
    }

    /// The paths in a command's answer are absolute and come from the server's
    /// own disk. Serving them by name without this check serves any file.
    #[test]
    fn media_is_confined_to_its_roots() {
        let tmp = std::env::temp_dir().join(format!("moose-media-{}", std::process::id()));
        let root = tmp.join("media");
        std::fs::create_dir_all(root.join("snes")).unwrap();
        std::fs::write(root.join("snes/a.png"), b"x").unwrap();
        std::fs::write(tmp.join("secret"), b"x").unwrap();
        let roots = vec![root.clone()];

        assert!(resolve_media(&roots, root.join("snes/a.png").to_str().unwrap()).is_some());
        // Outside, plainly.
        assert!(resolve_media(&roots, tmp.join("secret").to_str().unwrap()).is_none());
        // Outside, by a path that starts inside. This is the one that gets
        // through when the check is a string prefix.
        let dots = root.join("snes/../../secret");
        assert!(resolve_media(&roots, dots.to_str().unwrap()).is_none());
        // A directory is not media.
        assert!(resolve_media(&roots, root.join("snes").to_str().unwrap()).is_none());
        assert!(resolve_media(&roots, "/nope/nothing.png").is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }
}

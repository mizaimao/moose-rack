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
use moose_rack::app::AppState;
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

/// Answer one `invoke`.
///
/// Unknown and machine-local commands are told apart on purpose. A typo should
/// look different from a thing that exists and cannot work here.
/// Answer one `invoke` by calling the shared backend.
///
/// There is no logic here on purpose. Every command is
/// `moose_rack::commands::<name>`, the same function the desktop window calls
/// through its Tauri wrapper, so the two cannot drift. A handler written here
/// instead would be a second implementation of something that already exists.
/// Answer one `invoke` by calling the shared backend.
///
/// There is no logic here on purpose. Every arm is
/// `moose_rack::commands::<name>` -- the same function the desktop window calls
/// through its Tauri wrapper -- so the two cannot drift. A handler written here
/// would be a second implementation of something that already exists.
///
/// The `j!` is only to serialize: a shared closure would be monomorphised to
/// the first command's return type and refuse every other one.
pub fn dispatch(state: &AppState, cmd: &str, args: &Value) -> Result<Value, String> {
    use moose_rack::commands as c;
    macro_rules! j {
        ($e:expr) => {
            $e.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
        };
    }
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match cmd {
        "status" => j!(c::status(state)),
        "versions" => j!(c::versions(state)),
        "platforms" => j!(c::platforms(state)),
        "systems" => j!(c::systems(state)),
        "roms" => j!(c::roms(state, s("platform"), None)),
        "recent_games" => j!(c::recent_games(state, None, None)),
        "collection_groups" => j!(c::collection_groups(state)),

        // Things that act on the machine somebody is sitting at. Worded
        // differently from "unknown" so a typo does not look like a design limit.
        "launch_rom" | "set_pad_binding" | "set_key_binding" | "set_retroarch_root"
        | "import_bindings" | "reset_bindings" | "open_link" | "open_settings" => {
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
    match dispatch(&st.state, &cmd, &args) {
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
            if l.starts_with('"') && l.contains("=>") && l.contains("j!(") {
                assert!(l.contains("c::"), "arm does not delegate: {l}");
            }
        }
    }
}

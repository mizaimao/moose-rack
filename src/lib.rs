//! Shared backend for the CLI/TUI (`src/main.rs`) and the Tauri GUI
//! (`src-tauri/`). Everything the GUI needs already lives here — the GUI adds
//! a window, not a second implementation.

pub mod achievements;
pub mod api;
pub mod appicon;
pub mod binds;
pub mod bios;
pub mod arcade;
pub mod aspect;
pub mod bulk;
pub mod cache;
pub mod config;
pub mod configpatch;
// No longer test-only: it now carries the template the app seeds a first run
// with, which on Android is the only way a config can appear at all.
pub mod config_files;
pub mod coremap;
pub mod coverage;
pub mod cores;
pub mod datadir;
pub mod diskspace;
pub mod download;
pub mod esde;
pub mod favorites;
pub mod gamefilter;
pub mod gamelist;
pub mod focusring;
pub mod gamesort;
pub mod gridnav;
pub mod iconart;
pub mod launch;
pub mod layout;
pub mod lightgun;
pub mod macdisplay;
pub mod media;
pub mod platform;
pub mod platformfacts;
pub mod padprofile;
pub mod platformicon;
pub mod players;
pub mod padpoll;
pub mod pagefilter;
pub mod parity;
pub mod pickorder;
pub mod probe;
pub mod retroarch;
pub mod retroarch_install;
pub mod savehash;
pub mod shaders;
pub mod slangp;
pub mod theme;
pub mod tweaks;
pub mod theme_remote;
pub mod rowwindow;
pub mod savebackup;
pub mod script;
pub mod scrape;
pub mod saves;
pub mod savesync;
pub mod states;
pub mod statesync;
pub mod syncplan;
pub mod tui;
pub mod update;
pub mod util;

/// RomM release this client's server-specific behavior was verified against.
///
/// Archive hashing, param names and the `/api/config` shape were all read out
/// of this version. A different server may still work, but the mismatch is
/// worth surfacing rather than debugging from first principles again.
pub const VERIFIED_AGAINST: &str = "5.1.0";

/// Load cached server settings into the modules that need them.
///
/// Called at startup by every frontend so archive verification behaves the
/// same in the CLI, TUI and GUI — and offline, where `/api/config` is
/// unreachable but the last-known values are still correct.
pub fn apply_cached_server_config(store: &cache::Cache) {
    if let Some((files, exts)) = store.server_exclusions() {
        download::set_exclusions(files, exts);
    }
}

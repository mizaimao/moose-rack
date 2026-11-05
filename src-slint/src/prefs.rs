//! The window's own choices, kept between launches in `ui.toml` beside the
//! config: menu sounds, icon size, Sofa or Desk, grid or list, and the sort
//! order. What the core owns, the automatic save sync, goes to config.toml
//! through `moose_rack::commands::set_auto_sync` instead.
//!
//! A few `key = value` lines, read leniently: a missing or unreadable file, or
//! a line that makes no sense, leaves that choice at its default.

use std::path::PathBuf;
use std::time::Duration;

use slint::ComponentHandle;

use crate::AppWindow;

pub struct Prefs {
    pub sounds: bool,
    pub icon_size: f32,
    pub desk: bool,
    pub list: bool,
    /// "name", "year" or "played".
    pub sort: String,
}

fn path() -> PathBuf {
    moose_rack::config::path().with_file_name("ui.toml")
}

/// What was kept last time, or None when there is nothing to go on.
pub fn load() -> Option<Prefs> {
    let text = std::fs::read_to_string(path()).ok()?;
    let mut p = Prefs { sounds: true, icon_size: 150.0, desk: true, list: false, sort: "name".into() };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "sounds" => p.sounds = value != "false",
            "icon_size" => p.icon_size = value.parse().unwrap_or(p.icon_size),
            "layout" => p.desk = value != "sofa",
            "view" => p.list = value == "list",
            "sort" => p.sort = value.to_owned(),
            _ => {}
        }
    }
    Some(p)
}

fn save(p: &Prefs) {
    let text = format!(
        "# Moose Rack's window: kept between launches, safe to delete.\n\
         sounds = {}\nicon_size = {}\nlayout = \"{}\"\nview = \"{}\"\nsort = \"{}\"\n",
        p.sounds,
        p.icon_size.round(),
        if p.desk { "desk" } else { "sofa" },
        if p.list { "list" } else { "grid" },
        p.sort,
    );
    if let Err(e) = std::fs::write(path(), text) {
        eprintln!("settings: couldn't keep them in {}: {e}", path().display());
    }
}

thread_local! {
    static PENDING: slint::Timer = slint::Timer::default();
}

/// Keep the window's choices, half a second after the last change, so dragging
/// the icon size slider writes once rather than every step.
pub fn changed(ui: &AppWindow, sort: &'static str) {
    let weak = ui.as_weak();
    PENDING.with(|t| {
        t.start(slint::TimerMode::SingleShot, Duration::from_millis(500), move || {
            let Some(ui) = weak.upgrade() else { return };
            save(&Prefs {
                sounds: ui.get_opt_sounds(),
                icon_size: ui.get_zoom(),
                desk: ui.get_mode() == 1,
                list: ui.get_list_view(),
                sort: sort.to_owned(),
            });
        })
    });
}

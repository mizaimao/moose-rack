//! Where the cursor goes for each action `moose_rack::binds` names, whether it
//! came from a key or a pad button.
//!
//! The window keeps the cursor in `focus-zone` and `focus-index` (see Zone in
//! app.slint); this moves it, and acts through the window's own callbacks so a
//! press and a click do exactly the same thing.

use slint::{Model, SharedString};

use crate::AppWindow;

// The zones, numbered as Zone in app.slint numbers them.
pub const HEADER: i32 = 0;
pub const CONSOLES: i32 = 1;
pub const GRID: i32 = 2;
pub const DETAIL: i32 = 3;
pub const SETTINGS: i32 = 4;
pub const SOFA: i32 = 5;

/// The header's controls, left to right: Sofa/Desk, search, grid or list,
/// filter, random, sort, sync, settings.
const HEADER_LAST: i32 = 7;
/// Settings' rows: three toggles, theme, icon size, Done.
const SETTINGS_LAST: i32 = 5;
/// The detail's actions: Play, the download button, the star.
const DETAIL_LAST: i32 = 2;

const ZOOM_MIN: f32 = 90.0;
const ZOOM_MAX: f32 = 300.0;

/// Carry out one action. False when it meant nothing where the cursor is, so a
/// key can fall through to whatever else wants it.
pub fn dispatch(ui: &AppWindow, action: &str) -> bool {
    let zone = ui.get_focus_zone();
    let at = ui.get_focus_index();
    let desk = ui.get_mode() == 1;
    let count = ui.get_games().row_count() as i32;
    let cols = ui.get_grid_cols().max(1);
    let sel = ui.get_selected_index();
    let go = |zone: i32, index: i32| {
        ui.set_focus_zone(zone);
        ui.set_focus_index(index);
    };

    match action {
        "up" => match zone {
            GRID if sel >= cols => ui.set_selected_index(sel - cols),
            GRID => go(HEADER, 1),
            CONSOLES if ui.get_platform_index() > 0 => ui.invoke_platform_selected(ui.get_platform_index() - 1),
            CONSOLES => go(HEADER, 0),
            DETAIL => go(HEADER, 1),
            SETTINGS if at > 0 => go(SETTINGS, at - 1),
            _ => return false,
        },
        "down" => match zone {
            GRID if sel / cols < (count - 1) / cols => ui.set_selected_index((sel + cols).min(count - 1)),
            CONSOLES if ui.get_platform_index() + 1 < ui.get_platforms().row_count() as i32 => {
                ui.invoke_platform_selected(ui.get_platform_index() + 1)
            }
            HEADER => go(GRID, 0),
            SETTINGS if at < SETTINGS_LAST => go(SETTINGS, at + 1),
            _ => return false,
        },
        "left" => match zone {
            GRID if sel % cols > 0 => ui.set_selected_index(sel - 1),
            GRID if desk => go(CONSOLES, 0),
            DETAIL | SOFA | HEADER if at > 0 => go(zone, at - 1),
            DETAIL => go(GRID, 0),
            SETTINGS => adjust(ui, at, -1),
            _ => return false,
        },
        "right" => match zone {
            GRID if sel % cols < cols - 1 && sel + 1 < count => ui.set_selected_index(sel + 1),
            GRID if desk => go(DETAIL, 0),
            CONSOLES => go(GRID, 0),
            DETAIL | SOFA if at < DETAIL_LAST => go(zone, at + 1),
            HEADER if at < HEADER_LAST => go(HEADER, at + 1),
            SETTINGS => adjust(ui, at, 1),
            _ => return false,
        },
        "activate" => match zone {
            GRID if count == 0 => return false,
            GRID if desk => ui.invoke_play(sel),
            GRID => {
                ui.set_sofa_detail_open(true);
                go(SOFA, 0);
            }
            CONSOLES => go(GRID, 0),
            DETAIL | SOFA => detail_action(ui, at),
            HEADER => header_action(ui, at),
            SETTINGS => settings_action(ui, at),
            _ => return false,
        },
        "back" | "back2" => match zone {
            SETTINGS => {
                ui.set_settings_open(false);
                go(GRID, 0);
            }
            SOFA => {
                ui.set_sofa_detail_open(false);
                go(GRID, 0);
            }
            HEADER | DETAIL | CONSOLES => go(GRID, 0),
            _ if !ui.get_query().is_empty() => {
                ui.set_query(SharedString::new());
                ui.invoke_search_edited(SharedString::new());
            }
            _ => return false,
        },
        "prevSection" | "nextSection" => {
            ui.invoke_cycle_tab(if action == "prevSection" { -1 } else { 1 });
            ui.set_sofa_detail_open(false);
            go(GRID, 0);
        }
        "favorite" if count > 0 => ui.invoke_favorite_toggled(sel),
        "search" => ui.invoke_focus_search(),
        "settings" => {
            ui.set_settings_open(true);
            go(SETTINGS, 0);
        }
        "random" => {
            ui.invoke_random_pick();
            go(GRID, 0);
        }
        "sortCycle" | "sortMenu" => ui.invoke_sort_toggled(),
        "layout" => ui.set_list_view(!ui.get_list_view()),
        "first" if count > 0 => ui.set_selected_index(0),
        "last" if count > 0 => ui.set_selected_index(count - 1),
        "pageUp" | "scrollUp" if count > 0 => ui.set_selected_index((sel - cols * 3).max(0)),
        "pageDown" | "scrollDown" if count > 0 => ui.set_selected_index((sel + cols * 3).min(count - 1)),
        "zoomIn" => ui.set_zoom((ui.get_zoom() + 10.0).min(ZOOM_MAX)),
        "zoomOut" => ui.set_zoom((ui.get_zoom() - 10.0).max(ZOOM_MIN)),
        "download" if count > 0 => ui.invoke_start_download(),
        "help" | "video" | "pictures" | "filterMenu" | "sidebar" => {
            ui.invoke_show_toast(format!("\"{action}\" is not in the prototype yet").into())
        }
        _ => return false,
    }
    true
}

fn detail_action(ui: &AppWindow, at: i32) {
    let sel = ui.get_selected_index();
    let Some(game) = ui.get_games().row_data(sel as usize) else { return };
    match at {
        0 => ui.invoke_play(sel),
        1 if game.installed => ui.invoke_removed(sel),
        1 => ui.invoke_start_download(),
        _ => ui.invoke_favorite_toggled(sel),
    }
}

fn header_action(ui: &AppWindow, at: i32) {
    match at {
        0 => {
            ui.set_mode(1 - ui.get_mode());
            ui.set_sofa_detail_open(false);
        }
        1 => ui.invoke_focus_search(),
        2 => ui.set_list_view(!ui.get_list_view()),
        3 => ui.invoke_show_toast("Filters are not in the prototype yet".into()),
        4 => ui.invoke_random_pick(),
        5 => ui.invoke_sort_toggled(),
        6 => ui.invoke_show_toast("Saves synced (pretend)".into()),
        _ => {
            ui.set_settings_open(true);
            ui.set_focus_zone(SETTINGS);
            ui.set_focus_index(0);
        }
    }
}

fn settings_action(ui: &AppWindow, at: i32) {
    match at {
        0 => ui.set_opt_autosync(!ui.get_opt_autosync()),
        1 => ui.set_opt_hidden(!ui.get_opt_hidden()),
        2 => ui.set_opt_sounds(!ui.get_opt_sounds()),
        3 => ui.set_opt_theme((ui.get_opt_theme() + 1) % 3),
        4 => {}
        _ => {
            ui.set_settings_open(false);
            ui.set_focus_zone(GRID);
            ui.set_focus_index(0);
        }
    }
}

/// Left and right on a Settings row change its value where it has one.
fn adjust(ui: &AppWindow, at: i32, step: i32) {
    match at {
        3 => ui.set_opt_theme((ui.get_opt_theme() + step).clamp(0, 2)),
        4 => ui.set_zoom((ui.get_zoom() + 10.0 * step as f32).clamp(ZOOM_MIN, ZOOM_MAX)),
        _ => {}
    }
}

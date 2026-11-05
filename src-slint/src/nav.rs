//! Where the cursor goes for each action `moose_rack::binds` names, whether it
//! came from a key or a pad button.
//!
//! The window keeps the cursor in `focus-zone` and `focus-index` (see Zone in
//! app.slint); this moves it, and acts through the window's own callbacks so a
//! press and a click do exactly the same thing.

use std::cell::RefCell;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::AppWindow;

// The zones, numbered as Zone in app.slint numbers them.
pub const HEADER: i32 = 0;
pub const CONSOLES: i32 = 1;
pub const GRID: i32 = 2;
pub const DETAIL: i32 = 3;
pub const SETTINGS: i32 = 4;
pub const SOFA: i32 = 5;
// 6 is the Options menu, which `menu-open` stands for here.

thread_local! {
    /// What each row of the open Options menu does, as `menu_pick` names it.
    static MENU_ITEMS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

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
    note_header(ui);
    let done = act(ui, action);
    note_header(ui);
    done
}

/// The header remembers which of its controls had the cursor, so going up to
/// it lands there again (rule 2 in docs/controller-ux.md).
fn note_header(ui: &AppWindow) {
    if ui.get_focus_zone() == HEADER {
        ui.set_header_at(ui.get_focus_index());
    }
}

fn act(ui: &AppWindow, action: &str) -> bool {
    // The launch checklist takes every action while it is open: nothing behind
    // it moves, as in the desktop app's launch questions.
    if ui.get_checklist_open() {
        let has_main = !ui.get_checklist_button().is_empty() && ui.get_checklist_progress().is_empty();
        match action {
            "left" | "right" if has_main => ui.set_checklist_at(1 - ui.get_checklist_at()),
            "activate" if has_main && ui.get_checklist_at() == 0 => ui.invoke_checklist_go(),
            "activate" | "back" | "back2" => ui.invoke_checklist_cancel(),
            _ => {}
        }
        return true;
    }
    // So does the Options menu.
    if ui.get_menu_open() {
        let at = ui.get_focus_index();
        let rows = MENU_ITEMS.with(|m| m.borrow().len()) as i32;
        match action {
            "up" if at > 0 => ui.set_focus_index(at - 1),
            "down" if at + 1 < rows => ui.set_focus_index(at + 1),
            "activate" => menu_pick(ui, at),
            "back" | "back2" | "options" => ui.invoke_close_menu(),
            _ => {}
        }
        return true;
    }
    // Settings takes everything but moving, changing, closing and Start, so
    // nothing behind it moves and the hint bar can show just those.
    if ui.get_settings_open()
        && !matches!(action, "up" | "down" | "left" | "right" | "activate" | "back" | "back2" | "settings")
    {
        return true;
    }
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
    let header = ui.get_header_at();

    match action {
        "up" => match zone {
            GRID if sel >= cols => ui.set_selected_index(sel - cols),
            CONSOLES if ui.get_platform_index() > 0 => ui.invoke_platform_selected(ui.get_platform_index() - 1),
            GRID | CONSOLES | DETAIL => go(HEADER, header),
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
            SETTINGS => ui.invoke_close_settings(),
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
        // The star is for the game under the cursor, so only where there is one.
        "favorite" if count > 0 && matches!(zone, GRID | DETAIL | SOFA) => ui.invoke_favorite_toggled(sel),
        "options" => open_menu(ui, zone),
        "search" => ui.invoke_focus_search(),
        // Start opens Settings and closes it again, as the menu button does in
        // Steam's Big Picture and ES-DE.
        "settings" if ui.get_settings_open() => ui.invoke_close_settings(),
        "settings" => ui.invoke_open_settings(),
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

/// Open the Options menu for where the cursor is: the game's own actions first
/// when it is on one, then the list's (rule 5 in docs/controller-ux.md).
fn open_menu(ui: &AppWindow, zone: i32) {
    let sel = ui.get_selected_index();
    let game = if matches!(zone, GRID | DETAIL | SOFA) { ui.get_games().row_data(sel.max(0) as usize) } else { None };
    let mut items: Vec<(&'static str, &str)> = Vec::new();
    if let Some(g) = &game {
        items.push(if g.installed { ("remove", "Remove from this device") } else { ("download", "Take offline") });
        items.push(("favorite", if g.favorite { "Remove from favorites" } else { "Add to favorites" }));
    }
    items.push(("search", "Search"));
    items.push(("sort", if ui.get_sort_label() == "Year" { "Sort by name" } else { "Sort by year" }));
    items.push(("layout", if ui.get_list_view() { "Show as grid" } else { "Show as list" }));
    items.push(("random", "Random game"));
    ui.set_menu_title(game.map(|g| g.title).unwrap_or_else(|| "Options".into()));
    let labels: Vec<SharedString> = items.iter().map(|&(_, label)| label.into()).collect();
    ui.set_menu_items(ModelRc::new(VecModel::from(labels)));
    MENU_ITEMS.with(|m| *m.borrow_mut() = items.into_iter().map(|(id, _)| id).collect());
    ui.invoke_open_menu();
}

/// Do what a row of the Options menu says, after closing it so the cursor is
/// back where it was.
pub fn menu_pick(ui: &AppWindow, row: i32) {
    let Some(id) = MENU_ITEMS.with(|m| m.borrow().get(row.max(0) as usize).copied()) else { return };
    ui.invoke_close_menu();
    let sel = ui.get_selected_index();
    match id {
        "download" => ui.invoke_start_download(),
        "remove" => ui.invoke_removed(sel),
        "favorite" => ui.invoke_favorite_toggled(sel),
        "search" => ui.invoke_focus_search(),
        "sort" => ui.invoke_sort_toggled(),
        "layout" => ui.set_list_view(!ui.get_list_view()),
        "random" => {
            ui.invoke_random_pick();
            ui.set_focus_zone(GRID);
            ui.set_focus_index(0);
        }
        _ => {}
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
        _ => ui.invoke_open_settings(),
    }
}

fn settings_action(ui: &AppWindow, at: i32) {
    match at {
        0 => ui.set_opt_autosync(!ui.get_opt_autosync()),
        1 => ui.set_opt_hidden(!ui.get_opt_hidden()),
        2 => ui.set_opt_sounds(!ui.get_opt_sounds()),
        3 => ui.set_opt_theme((ui.get_opt_theme() + 1) % 3),
        4 => {}
        _ => ui.invoke_close_settings(),
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

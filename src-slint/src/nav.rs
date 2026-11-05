//! Where the cursor goes for each action `moose_rack::binds` names, whether it
//! came from a key or a pad button.
//!
//! The window keeps the cursor in `focus-zone` and `focus-index` (see Zone in
//! app.slint); this moves it, and acts through the window's own callbacks so a
//! press and a click do exactly the same thing.

use std::cell::{Cell, RefCell};

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::AppWindow;
use crate::sound::Cue;

// The zones, numbered as Zone in app.slint numbers them.
pub const HEADER: i32 = 0;
pub const CONSOLES: i32 = 1;
pub const GRID: i32 = 2;
pub const DETAIL: i32 = 3;
pub const SETTINGS: i32 = 4;
pub const SOFA: i32 = 5;
// 6 is the Options menu, which `menu-open` stands for here.
pub const KEYBOARD: i32 = 7;

/// The on-screen keyboard's bottom row (indices 40 to 43): Space, Delete,
/// Clear and Done, each under these columns of the rows above.
const OSK_WIDE: [(i32, i32); 4] = [(0, 4), (4, 6), (6, 8), (8, 10)];
const OSK_SPACE: i32 = 40;
const OSK_DELETE: i32 = 41;
const OSK_CLEAR: i32 = 42;
const OSK_DONE: i32 = 43;
/// Where the keyboard opens: on q, then wherever it was left.
const OSK_START: i32 = 10;

thread_local! {
    /// What each row of the open Options menu does, as `menu_pick` names it.
    static MENU_ITEMS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    /// The letters the list's titles start with, in list order, and which one
    /// the menu's "Jump to letter" row is on.
    static LETTERS: RefCell<(Vec<char>, usize)> = const { RefCell::new((Vec::new(), 0)) };
    /// The keyboard key the cursor was on when it closed, and the column it
    /// was last in above the bottom row, so Up from a wide key goes back there.
    static OSK_AT: Cell<i32> = const { Cell::new(OSK_START) };
    static OSK_COL: Cell<i32> = const { Cell::new(0) };
}

/// Search with the pad alone (rule 7): the on-screen keyboard while a
/// controller is connected, the text box otherwise.
pub fn open_search(ui: &AppWindow) {
    if !ui.get_pad_connected() {
        ui.invoke_focus_search();
        return;
    }
    ui.set_osk_open(true);
    ui.set_focus_zone(KEYBOARD);
    ui.set_focus_index(OSK_AT.get());
}

fn close_osk(ui: &AppWindow) {
    let at = ui.get_focus_index();
    OSK_AT.set(if at == OSK_DONE { OSK_START } else { at });
    // Only spaces is no search, and should not leave Clear search behind.
    if ui.get_query().trim().is_empty() && !ui.get_query().is_empty() {
        osk_edit(ui, String::clear);
    }
    ui.set_osk_open(false);
    ui.set_focus_zone(GRID);
    ui.set_focus_index(0);
}

/// One press on the keyboard. The results narrow behind it as each key types.
fn osk(ui: &AppWindow, action: &str) {
    let at = ui.get_focus_index();
    let wide = at >= OSK_SPACE;
    let col = at % 10;
    let to = match action {
        "left" if (wide && at > OSK_SPACE) || (!wide && col > 0) => at - 1,
        "right" if (wide && at < OSK_DONE) || (!wide && col < 9) => at + 1,
        "up" if wide => {
            let (from, to) = OSK_WIDE[(at - OSK_SPACE) as usize];
            let c = OSK_COL.get();
            30 + if (from..to).contains(&c) { c } else { from }
        }
        "up" if at >= 10 => at - 10,
        "down" if !wide && at < 30 => at + 10,
        "down" if !wide => OSK_SPACE + OSK_WIDE.iter().position(|&(from, to)| (from..to).contains(&col)).unwrap_or(0) as i32,
        "activate" => return osk_press(ui, at),
        "favorite" => return osk_press(ui, OSK_DELETE),
        "options" => return osk_press(ui, OSK_SPACE),
        "settings" | "back" | "back2" => return close_osk(ui),
        _ => return,
    };
    if to < OSK_SPACE {
        OSK_COL.set(to % 10);
    }
    ui.set_focus_index(to);
}

/// Press a keyboard key, by its index, from the pad or a click.
pub fn osk_press(ui: &AppWindow, key: i32) {
    match key {
        OSK_SPACE => osk_edit(ui, |q| q.push(' ')),
        OSK_DELETE => osk_edit(ui, |q| {
            q.pop();
        }),
        OSK_CLEAR => osk_edit(ui, String::clear),
        OSK_DONE => close_osk(ui),
        _ => {
            if let Some(k) = ui.get_osk_keys().row_data(key.max(0) as usize) {
                osk_edit(ui, |q| q.push_str(&k));
            }
        }
    }
}

/// A key typed on a real keyboard while the on-screen one is open types into
/// the search too. False for the keys that move and press.
pub fn osk_key(ui: &AppWindow, name: &str) -> bool {
    if name == "Backspace" {
        osk_press(ui, OSK_DELETE);
        return true;
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => {
            osk_edit(ui, |q| q.extend(c.to_lowercase()));
            true
        }
        _ => false,
    }
}

fn osk_edit(ui: &AppWindow, edit: impl FnOnce(&mut String)) {
    let mut query = ui.get_query().to_string();
    edit(&mut query);
    ui.set_query(query.as_str().into());
    ui.invoke_search_edited(query.into());
}

/// The letter a title is filed under when sorted by name: the first character,
/// capitalised, or # for anything that isn't a plain letter.
fn letter_of(title: &str) -> char {
    match title.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
        _ => '#',
    }
}

fn letter_label(letter: char) -> String {
    format!("Jump to letter   \u{2039} {letter} \u{203a}")
}

/// Left and right on the "Jump to letter" row change its letter.
fn step_letter(ui: &AppWindow, row: i32, step: i32) {
    let label = LETTERS.with(|l| {
        let (letters, at) = &mut *l.borrow_mut();
        if letters.is_empty() {
            return None;
        }
        *at = (*at as i32 + step).clamp(0, letters.len() as i32 - 1) as usize;
        Some(letter_label(letters[*at]))
    });
    if let Some(label) = label {
        ui.get_menu_items().set_row_data(row as usize, label.into());
    }
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
    // Every press that did something says so (rule 9), unless Settings has
    // turned the sounds off.
    if done && ui.get_opt_sounds() {
        crate::sound::play(match action {
            "activate" => Cue::Confirm,
            "back" | "back2" => Cue::Back,
            _ => Cue::Move,
        });
    }
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
        let on_letter = MENU_ITEMS.with(|m| m.borrow().get(at as usize) == Some(&"letter"));
        match action {
            "up" if at > 0 => ui.set_focus_index(at - 1),
            "down" if at + 1 < rows => ui.set_focus_index(at + 1),
            "left" | "right" if on_letter => step_letter(ui, at, if action == "left" { -1 } else { 1 }),
            "activate" => menu_pick(ui, at),
            "back" | "back2" | "options" => ui.invoke_close_menu(),
            _ => {}
        }
        return true;
    }
    // And the on-screen keyboard.
    if ui.get_osk_open() {
        osk(ui, action);
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
    let page = ui.get_grid_page_rows().max(1) * cols;

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
        "search" => open_search(ui),
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
        // A page is the rows on screen, in the same column (rule 6).
        "pageUp" | "scrollUp" if count > 0 => ui.set_selected_index((sel - page).max(0)),
        "pageDown" | "scrollDown" if count > 0 => ui.set_selected_index((sel + page).min(count - 1)),
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
    // Jumping by letter, for a list sorted by name (rule 6), as ES-DE's
    // options menu does it: left and right pick the letter.
    let by_name = ui.get_sort_label() != "Year";
    let games = ui.get_games();
    let letter_row;
    if by_name && games.row_count() > 0 {
        let mut letters: Vec<char> = Vec::new();
        for g in games.iter() {
            let l = letter_of(&g.title);
            if !letters.contains(&l) {
                letters.push(l);
            }
        }
        let here = games.row_data(sel.max(0) as usize).map(|g| letter_of(&g.title)).unwrap_or('#');
        let at = letters.iter().position(|&l| l == here).unwrap_or(0);
        letter_row = letter_label(letters[at]);
        LETTERS.with(|l| *l.borrow_mut() = (letters, at));
        items.push(("letter", &letter_row));
    }
    items.push(("search", "Search"));
    items.push(("sort", if by_name { "Sort by year" } else { "Sort by name" }));
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
        "search" => open_search(ui),
        "sort" => ui.invoke_sort_toggled(),
        "layout" => ui.set_list_view(!ui.get_list_view()),
        "letter" => {
            let Some(letter) = LETTERS.with(|l| l.borrow().0.get(l.borrow().1).copied()) else { return };
            if let Some(row) = ui.get_games().iter().position(|g| letter_of(&g.title) == letter) {
                ui.set_selected_index(row as i32);
                ui.set_focus_zone(GRID);
                ui.set_focus_index(0);
            }
        }
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
        1 => open_search(ui),
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

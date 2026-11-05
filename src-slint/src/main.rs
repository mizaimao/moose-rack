//! The Slint front end for Moose Rack.
//!
//! It shows the real library through the shared core (`moose_rack`), read
//! from the same cache and kept current by the same library update as the
//! desktop app. With no config to build that from, or with
//! `MOOSE_SLINT_SAMPLE=1` (and always for `MOOSE_SLINT_SNAPSHOT`, so pictures
//! stay comparable), it shows the sample instead. See README.md.

mod library;
mod model;
mod nav;
#[cfg(not(target_os = "android"))]
mod pad;
mod prefs;
mod sample;
mod sound;

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use moose_rack::app::AppState;
use moose_rack::{binds, padpoll};
use slint::{Color, Model, ModelRc, SharedString, VecModel};

use model::{Platform, hash, human};

slint::include_modules!();

/// What the grid is showing, and the choices that decide it.
struct State {
    platform: usize,
    tab: i32,
    query: String,
    /// The order lists are in: name, year, or recently played.
    sort: Sort,
    /// Each row of the grid's model, as (platform, game) indices.
    shown: Vec<(usize, usize)>,
    /// Each row's game id. Unlike `shown` it still holds after the library
    /// update has replaced `platforms`.
    shown_ids: Vec<i64>,
    /// Which list `shown` is (`list_key`); none for search results.
    shown_key: Option<String>,
    /// The game the cursor was last on in each list, so coming back to a
    /// console or tab lands there again (rule 2 in docs/controller-ux.md).
    remembered: std::collections::HashMap<String, i64>,
    /// The filter menu's switches: every list shows only games on this
    /// device, only favorites, or both.
    only_device: bool,
    only_favorites: bool,
}

/// The tabs, in the order the tab row shows them. Recent is next to Platforms,
/// so recently played games are one press away (rule 6), as the old app's
/// strip above the consoles and Steam's Recent Games shelf put them up front.
const TAB_CONSOLES: i32 = 0;
const TAB_RECENT: i32 = 1;
const TAB_FAVORITES: i32 = 2;
const TAB_DEVICE: i32 = 3;

/// The orders Sort by offers, as the old app's sort menu (`moose_rack::gamesort`).
#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Name,
    Year,
    Played,
}

impl Sort {
    const ALL: [Sort; 3] = [Sort::Name, Sort::Year, Sort::Played];

    /// How `ui.toml` writes it.
    fn id(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Year => "year",
            Sort::Played => "played",
        }
    }

    fn from_id(id: &str) -> Sort {
        Sort::ALL.into_iter().find(|s| s.id() == id).unwrap_or(Sort::Name)
    }

    fn label(self) -> &'static str {
        match self {
            Sort::Name => "Name",
            Sort::Year => "Year",
            Sort::Played => "Recently played",
        }
    }
}

/// Where the cursor lands when the grid's list is rebuilt.
enum Land {
    /// Where it last was in this list, or the top.
    Remembered,
    /// On the same game, or on the same row when that game has left the list.
    Same,
    /// On this row, or the last: the next game after one that left the list.
    Row(usize),
}

/// Everything the window's callbacks share. It lives on the window's thread;
/// work done elsewhere (the library update, covers) hands its result back
/// with `upgrade_in_event_loop`.
struct App {
    platforms: Vec<Platform>,
    state: State,
    /// The shared core's state, when there is a real library.
    lib: Option<Arc<AppState>>,
    /// Where the core's network work runs.
    runtime: Option<Arc<tokio::runtime::Runtime>>,
    /// The rows on screen in the grid, first and last, as the grid reports them.
    visible: (usize, usize),
    /// Games whose cover has been asked for and not dropped since.
    requested: std::collections::HashSet<i64>,
    /// What is running: the window won't close while a game is (`moose_rack::busy`).
    busy: Arc<moose_rack::busy::Busy>,
    /// The game the checklist window is about, while it is open.
    checking: Option<i64>,
    /// Why the last library update failed, until one succeeds.
    library_error: Option<String>,
    /// Counts selection changes, so details that arrive late for a game the
    /// cursor has left are dropped.
    detail_gen: u64,
    /// Counts changes of the game pictures, so covers of the old kind still
    /// on their way when the kind changed are dropped.
    art_gen: u64,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.borrow_mut().as_mut().map(f))
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    // Pictures use the sample so they stay comparable, unless MOOSE_SLINT_REAL
    // asks for the real library in them.
    let sample_only = std::env::var_os("MOOSE_SLINT_SAMPLE").is_some()
        || (std::env::var_os("MOOSE_SLINT_SNAPSHOT").is_some() && std::env::var_os("MOOSE_SLINT_REAL").is_none());
    let lib = if sample_only { None } else { library::open() };
    let platforms = match &lib {
        Some(state) => library::load(state),
        None => sample::platforms(),
    };
    let runtime = lib
        .as_ref()
        .and_then(|_| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().ok())
        .map(Arc::new);
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            platforms,
            state: State {
                platform: 0,
                tab: 0,
                query: String::new(),
                sort: Sort::Name,
                shown: Vec::new(),
                shown_ids: Vec::new(),
                shown_key: None,
                remembered: Default::default(),
                only_device: false,
                only_favorites: false,
            },
            lib: lib.clone(),
            runtime,
            visible: (0, NEAR),
            requested: Default::default(),
            busy: Default::default(),
            checking: None,
            library_error: None,
            detail_gen: 0,
            art_gen: 0,
        })
    });
    // MOOSE_SLINT_PLATFORM=gba opens on that console, for measuring one.
    if let Ok(slug) = std::env::var("MOOSE_SLINT_PLATFORM") {
        with_app(|a| a.state.platform = a.platforms.iter().position(|p| p.slug == slug).unwrap_or(0));
    }
    // The window's choices from last time (prefs.rs). Pictures and scripted
    // tests start from the defaults and keep nothing, so they stay comparable
    // and leave no trace.
    // MOOSE_SLINT_KEEP_PREFS lets a scripted test check them.
    let keep_prefs = std::env::var_os("MOOSE_SLINT_SNAPSHOT").is_none()
        && (std::env::var_os("MOOSE_SLINT_TEST_KEYS").is_none() || std::env::var_os("MOOSE_SLINT_KEEP_PREFS").is_some());
    if let Some(p) = prefs::load().filter(|_| keep_prefs) {
        ui.set_opt_sounds(p.sounds);
        ui.set_zoom(p.icon_size.clamp(90.0, 300.0));
        ui.set_mode(if p.desk { 1 } else { 0 });
        ui.set_list_view(p.list);
        with_app(|a| a.state.sort = Sort::from_id(&p.sort));
    }
    if keep_prefs {
        ui.on_prefs_changed({
            let weak = ui.as_weak();
            move || {
                let sort = with_app(|a| a.state.sort.id()).unwrap_or("name");
                prefs::changed(&weak.unwrap(), sort);
            }
        });
    }
    // The save sync belongs to the core (config.toml); the switch shows it and
    // changes it there, taking effect at once.
    if let Some(lib) = &lib {
        ui.set_opt_autosync(lib.auto_sync.load(std::sync::atomic::Ordering::Relaxed));
    }
    ui.on_autosync_changed({
        let weak = ui.as_weak();
        let lib = lib.clone();
        move |on| {
            let Some(lib) = &lib else { return };
            if lib.auto_sync.load(std::sync::atomic::Ordering::Relaxed) == on {
                return;
            }
            if let Err(e) = moose_rack::commands::set_auto_sync(lib, on) {
                weak.unwrap().invoke_show_toast(format!("Couldn't change the save sync: {e}").into());
            }
        }
    });
    ui.on_pictures_step({
        let weak = ui.as_weak();
        move |which, by| {
            let ui = weak.unwrap();
            if which == 0 { step_list_art(&ui, by) } else { step_icon_style(&ui, by) }
        }
    });
    ui.on_get_console_pictures({
        let weak = ui.as_weak();
        move || get_console_pictures(&weak.unwrap())
    });
    show_picture_labels(&ui);
    show_platforms(&ui);
    refresh(&ui, Land::Remembered);
    // Sofa opens on the consoles, as the old app opened on its Platforms page.
    if ui.get_mode() == 0 {
        ui.set_sofa_consoles(true);
        ui.set_focus_zone(nav::CONSOLES);
    }
    fetch_covers(&ui);
    if lib.is_some() {
        update_library(&ui);
    }
    // Test switches, for driving it where nobody is at the screen:
    // MOOSE_SLINT_TEST_PLAY=ps2/Crazy Taxi (USA).chd presses Play on that game
    // two seconds in, and MOOSE_SLINT_TEST_ACCEPT presses the checklist's main
    // button when it shows.
    if let (Ok(target), Some(state)) = (std::env::var("MOOSE_SLINT_TEST_PLAY"), lib.clone()) {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_secs(2), move || {
            let Some(ui) = weak.upgrade() else { return };
            let (platform, file) = target.split_once('/').unwrap_or(("", target.as_str()));
            match moose_rack::commands::find_rom_by_file(&state, platform.to_owned(), file.to_owned()) {
                Ok(Some(id)) => play_game(&ui, id),
                _ => eprintln!("test play: {target} is not in the library"),
            }
        });
    }
    // MOOSE_SLINT_TEST_KEYS=right,down,settings,back presses those actions
    // (the names `moose_rack::binds` uses) one by one, prints where the cursor
    // is after each, and quits; `type:zelda` types into search, and `pad` acts
    // as if a controller were connected. For checking the controller rules in
    // docs/controller-ux.md.
    let _test_keys = std::env::var("MOOSE_SLINT_TEST_KEYS").ok().map(|keys| test_keys(&ui, keys));

    ui.on_platform_selected({
        let weak = ui.as_weak();
        move |i: i32| {
            let ui = weak.unwrap();
            with_app(|a| {
                a.state.platform = i as usize;
                a.state.tab = 0;
                a.state.query.clear();
            });
            ui.set_tab_index(0);
            ui.set_query(SharedString::new());
            refresh(&ui, Land::Remembered);
            fetch_covers(&ui);
        }
    });

    ui.on_tab_selected({
        let weak = ui.as_weak();
        move |i: i32| {
            let ui = weak.unwrap();
            with_app(|a| {
                a.state.tab = i;
                a.state.query.clear();
            });
            ui.set_query(SharedString::new());
            refresh(&ui, Land::Remembered);
            fetch_covers(&ui);
        }
    });

    ui.on_visible_range({
        let weak = ui.as_weak();
        move |first: i32, last: i32| {
            let ui = weak.unwrap();
            let now = (first.max(0) as usize, last.max(0) as usize);
            if with_app(|a| std::mem::replace(&mut a.visible, now) != now).unwrap_or(false) {
                fetch_covers(&ui);
            }
        }
    });

    ui.on_search_edited({
        let weak = ui.as_weak();
        move |text: SharedString| {
            let ui = weak.unwrap();
            with_app(|a| a.state.query = text.trim().to_string());
            refresh(&ui, Land::Remembered);
        }
    });

    ui.on_sort_toggled({
        let weak = ui.as_weak();
        move || {
            let ui = weak.unwrap();
            with_app(|a| {
                let at = Sort::ALL.iter().position(|&o| o == a.state.sort).unwrap_or(0);
                a.state.sort = Sort::ALL[(at + 1) % Sort::ALL.len()];
            });
            refresh(&ui, Land::Same);
        }
    });

    // Sort by's choice, by its place in `Sort::ALL`.
    ui.on_sort_chosen({
        let weak = ui.as_weak();
        move |i| {
            let ui = weak.unwrap();
            with_app(|a| a.state.sort = Sort::ALL.get(i.max(0) as usize).copied().unwrap_or(Sort::Name));
            refresh(&ui, Land::Same);
        }
    });

    ui.on_random_pick({
        let weak = ui.as_weak();
        move || {
            let ui = weak.unwrap();
            let pick = with_app(|a| {
                if a.state.shown.is_empty() {
                    return None;
                }
                let i = (random() % a.state.shown.len() as u64) as usize;
                let (pi, gi) = a.state.shown[i];
                Some((i, a.platforms[pi].games[gi].title.clone()))
            })
            .flatten();
            if let Some((i, title)) = pick {
                ui.set_selected_index(i as i32);
                ui.invoke_show_toast(format!("Random pick: {title}").into());
            }
        }
    });

    ui.on_play({
        let weak = ui.as_weak();
        move |i: i32| {
            let ui = weak.unwrap();
            if let Some(id) = with_app(|a| a.state.shown.get(i as usize).map(|&(pi, gi)| a.platforms[pi].games[gi].id)).flatten() {
                play_game(&ui, id);
            }
        }
    });

    ui.on_selection_changed({
        let weak = ui.as_weak();
        move |_row: i32| request_detail(&weak.unwrap())
    });

    ui.on_checklist_go({
        let weak = ui.as_weak();
        move || checklist_download(&weak.unwrap())
    });

    ui.on_checklist_cancel({
        let weak = ui.as_weak();
        move || {
            let ui = weak.unwrap();
            let fetching = !ui.get_checklist_progress().is_empty();
            with_app(|a| a.checking = None);
            ui.set_checklist_open(false);
            ui.invoke_show_toast(if fetching { "The download carries on; press Play again when it's done" } else { "Launch cancelled" }.into());
        }
    });

    // Not while a game runs: its saves go up when it ends, as in the desktop app.
    ui.window().on_close_requested({
        let weak = ui.as_weak();
        move || {
            let running = with_app(|a| a.busy.hold()).flatten();
            match running {
                Some(moose_rack::busy::Hold::Playing(name)) => {
                    if let Some(ui) = weak.upgrade() {
                        ui.invoke_show_toast(format!("{name} is still running. Quit the game first: its saves go up when it ends.").into());
                    }
                    slint::CloseRequestResponse::KeepWindowShown
                }
                _ => slint::CloseRequestResponse::HideWindow,
            }
        }
    });

    ui.on_favorite_toggled({
        let weak = ui.as_weak();
        move |i: i32| {
            let ui = weak.unwrap();
            let toggled = with_app(|a| {
                let &(pi, gi) = a.state.shown.get(i as usize)?;
                let g = &mut a.platforms[pi].games[gi];
                g.favorite = !g.favorite;
                // The real star lives on the server, in the favorites list.
                if let (Some(lib), Some(rt)) = (a.lib.clone(), a.runtime.clone()) {
                    let id = g.id;
                    rt.spawn(async move {
                        if let Err(e) = moose_rack::commands::toggle_favorite(&lib, id).await {
                            eprintln!("favorite: {e}");
                        }
                    });
                }
                Some(a.state.tab == TAB_FAVORITES)
            })
            .flatten();
            match toggled {
                // Unstarring on the Favorites tab takes the game off it, and
                // the cursor moves on to the next one.
                Some(true) => refresh(&ui, Land::Row(i as usize)),
                Some(false) => redraw_row(&ui, i as usize),
                None => {}
            }
        }
    });

    ui.on_download_finished({
        let weak = ui.as_weak();
        move |i: i32| {
            let ui = weak.unwrap();
            if let Some(title) = set_installed(i as usize, true) {
                redraw_row(&ui, i as usize);
                ui.invoke_show_toast(format!("{title} is on this device").into());
            }
        }
    });

    ui.on_removed({
        let weak = ui.as_weak();
        move |i: i32| {
            let ui = weak.unwrap();
            if let Some(title) = set_installed(i as usize, false) {
                if with_app(|a| a.state.tab == TAB_DEVICE).unwrap_or(false) {
                    refresh(&ui, Land::Row(i as usize));
                } else {
                    redraw_row(&ui, i as usize);
                }
                ui.invoke_show_toast(format!("Removed {title} from this device").into());
            }
        }
    });

    // Keys and pad buttons both become the core's actions, so a rebinding made
    // in the app's Settings would mean the same thing here.
    let bindings = prototype_bindings();
    ui.on_key({
        let weak = ui.as_weak();
        let bindings = bindings.clone();
        move |name| {
            let ui = weak.unwrap();
            // With the on-screen keyboard open, typing types.
            if ui.get_osk_open() && nav::osk_key(&ui, &name) {
                return true;
            }
            bindings.action_for(&name).is_some_and(|action| nav::dispatch(&ui, action))
        }
    });
    ui.on_osk_pressed({
        let weak = ui.as_weak();
        move |key| nav::osk_press(&weak.unwrap(), key)
    });
    ui.on_ask_chosen({
        let weak = ui.as_weak();
        move |choice| answered(&weak.unwrap(), choice)
    });
    ui.on_go_back({
        let weak = ui.as_weak();
        move || {
            nav::go_back(&weak.unwrap());
        }
    });
    ui.on_console_opened({
        let weak = ui.as_weak();
        move |i| nav::open_console(&weak.unwrap(), i)
    });
    ui.on_menu_picked({
        let weak = ui.as_weak();
        move |row| nav::menu_pick(&weak.unwrap(), row)
    });
    #[cfg(not(target_os = "android"))]
    let _pads = read_pads(&ui, &bindings);

    if let Ok(path) = std::env::var("MOOSE_SLINT_SNAPSHOT") {
        snapshot(&ui, path, std::env::var("MOOSE_SLINT_SCENE").unwrap_or_default());
    }

    ui.run()
}

/// The app's bindings with three buttons given jobs the hint bar shows: the
/// left face button stars a game, the top one searches (it plays the gameplay
/// video in the app), and Start opens Settings (the shortcut list in the app),
/// as "," does on the keyboard. Confirm and back are swapped on top of these;
/// see `read_pads`.
fn prototype_bindings() -> binds::Bindings {
    let mut bindings = binds::Bindings::default();
    // Steam's library, as Frank chose on 2026-11-05 (rule 5 in
    // docs/controller-ux.md): X (the left face button) filters, Y (the top
    // one) sorts, the Menu button (Start) opens Options for the game, and the
    // Steam button (Guide; Xbox, PS) opens the main menu, here Settings.
    // Search is in the header, as in Steam, and in Options.
    bindings.set_pad("filterMenu", Some(2));
    bindings.set_pad("sortMenu", Some(3));
    bindings.set_pad("options", Some(9));
    bindings.set_pad("settings", Some(16));
    // View (Select) keeps the app's own job: the game pictures, as in the
    // old app.
    bindings.set_key("settings", Some(","));
    bindings
}

/// Poll the controllers every frame on the window's own thread, and turn what
/// is held into actions with the core's repeat timing and stick dead zone.
/// Returns the timer, which stops when dropped.
#[cfg(not(target_os = "android"))]
fn read_pads(ui: &AppWindow, bindings: &binds::Bindings) -> Option<slint::Timer> {
    let mut pads = match pad::Pads::new() {
        Ok(pads) => pads,
        Err(e) => {
            eprintln!("controllers: {e}");
            return None;
        }
    };
    // Confirm is the right face button and back the bottom one, on every pad:
    // A on Nintendo, Circle on PlayStation, B on Xbox. Frank's call
    // (2026-10-31), the same place whatever is printed there, and the core's
    // own A/B swap does it.
    let map = bindings.pad_map_swapped(true, false);
    let mut repeat = padpoll::Repeat::default();
    let mut connected = 0;
    let start = Instant::now();
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(16), move || {
        let Some(ui) = weak.upgrade() else { return };
        for change in pads.changes() {
            match change {
                pad::Change::Connected(info) => {
                    println!("controller connected: {} ({})", info.name, info.kind);
                    ui.invoke_show_toast(format!("{} connected: {}", info.kind, info.name).into());
                    show_pad(&ui, &info);
                    connected += 1;
                }
                pad::Change::Disconnected(name) => {
                    println!("controller disconnected: {name}");
                    ui.invoke_show_toast(format!("{name} disconnected").into());
                    connected -= 1;
                    if connected == 0 {
                        show_keyboard(&ui);
                    }
                }
            }
        }
        let frame = pads.frame();
        let pressed = padpoll::pressed_actions(&frame.buttons, &frame.axes, &map);
        let now = start.elapsed().as_secs_f64() * 1000.0;
        match PAD_GATE.get() {
            PadGate::Open => {}
            PadGate::Suspended => {
                repeat.clear();
                return;
            }
            PadGate::Settling(until) => {
                let until = until.saturating_duration_since(start).as_secs_f64() * 1000.0;
                if !padpoll::settle_lifted(&pressed, now, until) {
                    repeat.clear();
                    return;
                }
                PAD_GATE.set(PadGate::Open);
            }
        }
        repeat.release(&pressed);
        for action in &pressed {
            if repeat.fire_accelerating(action, now) {
                nav::dispatch(&ui, action);
            }
        }
    });
    Some(timer)
}

#[cfg(not(target_os = "android"))]
fn show_pad(ui: &AppWindow, info: &pad::Info) {
    ui.set_pad_connected(true);
    ui.set_pad_name(info.name.clone().into());
    ui.set_pad_kind(info.kind.into());
    // `faces` is bottom, right, left, top; the jobs are as `read_pads` maps them.
    ui.set_glyph_confirm(info.faces[1].into());
    ui.set_glyph_back(info.faces[0].into());
    ui.set_glyph_left(info.faces[2].into());
    ui.set_glyph_top(info.faces[3].into());
    ui.set_shoulder_left(info.shoulders[0].into());
    ui.set_shoulder_right(info.shoulders[1].into());
    ui.set_start_label(info.start.into());
}

/// The hints go back to the keys that do the same things.
#[cfg(not(target_os = "android"))]
fn show_keyboard(ui: &AppWindow) {
    ui.set_pad_connected(false);
    ui.set_glyph_confirm("Enter".into());
    ui.set_glyph_back("Esc".into());
    ui.set_glyph_left("F".into());
    ui.set_glyph_top("S".into());
    ui.set_shoulder_left("Q".into());
    ui.set_shoulder_right("E".into());
    ui.set_start_label("O".into());
}

/// `MOOSE_SLINT_SNAPSHOT=out.png` sets up one scene, saves the window as a PNG
/// and quits, so the look can be checked on a machine nobody is watching.
/// `MOOSE_SLINT_SCENE` picks it: desk (the default), sofa, detail, list,
/// settings, download, header, consoles, actions, options, keyboard, recent,
/// tiles (Sofa's consoles), offline, conflict, ps5, switch, moved (the cursor
/// two cards on) or pictures (the Game pictures menu).
fn snapshot(ui: &AppWindow, path: String, scene: String) {
    match scene.as_str() {
        "sofa" => ui.set_mode(0),
        "detail" => {
            ui.set_mode(0);
            ui.set_sofa_detail_open(true);
        }
        "list" => ui.set_list_view(true),
        "settings" => {
            ui.set_settings_open(true);
            ui.set_focus_zone(nav::SETTINGS);
            ui.set_focus_index(2);
        }
        "header" => {
            ui.set_focus_zone(nav::HEADER);
            ui.set_focus_index(4);
        }
        "consoles" => ui.set_focus_zone(nav::CONSOLES),
        "options" => {
            nav::dispatch(ui, "options");
        }
        "tiles" => {
            ui.set_mode(0);
            ui.set_sofa_consoles(true);
            ui.set_focus_zone(nav::CONSOLES);
        }
        "recent" => {
            ui.set_tab_index(TAB_RECENT);
            ui.invoke_tab_selected(TAB_RECENT);
        }
        "offline" => stopped(ui, 0, Default::default(), "SAVE_OFFLINE:the server didn't answer (dev.lan:8080)"),
        "conflict" => {
            let c = moose_rack::savesync::SaveConflict {
                rom_id: 0,
                save_id: None,
                file_name: "Ico (USA).ps2".into(),
                slot: None,
                emulator: None,
                reason: Some("Both copies changed since the last sync.".into()),
                local_path: None,
                local_updated: Some("2026-11-04T21:12:09Z".into()),
                local_bytes: 8_388_608,
                server_updated: Some("2026-11-03T18:40:55Z".into()),
            };
            ASKING.with(|q| *q.borrow_mut() = Some(Asking::Conflict { id: 0, opts: Default::default(), conflicts: vec![c] }));
            ask_next(ui);
        }
        "keyboard" => {
            ui.set_pad_connected(true);
            nav::open_search(ui);
            // i and c: "ic", which finds Ico in the sample.
            for key in [17, 32] {
                nav::osk_press(ui, key);
            }
            nav::dispatch(ui, "right");
        }
        "actions" => {
            ui.set_focus_zone(nav::DETAIL);
            ui.set_focus_index(0);
        }
        // The cursor two cards on, to see nothing is left lit behind it.
        // After the first layout: until then the grid has no columns, and
        // Right leaves it for the info pane.
        "moved" => {
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(400), move || {
                let Some(ui) = weak.upgrade() else { return };
                nav::dispatch(&ui, "right");
                nav::dispatch(&ui, "right");
            });
        }
        "pictures" => {
            nav::dispatch(ui, "pictures");
        }
        #[cfg(not(target_os = "android"))]
        "ps5" | "switch" => {
            let (faces, shoulders, start, kind, name) = if scene == "ps5" {
                (["cross", "circle", "square", "triangle"], ["L1", "R1"], "Options", "PS5", "DualSense Wireless Controller")
            } else {
                (["B", "A", "Y", "X"], ["L", "R"], "+", "Switch Pro", "Pro Controller")
            };
            show_pad(&ui, &pad::Info { name: name.into(), kind, faces, shoulders, start });
        }
        "download" => {
            ui.set_selected_index(1);
            ui.set_download_index(1);
            ui.set_download_progress(0.4);
        }
        _ => {}
    }
    let weak = ui.as_weak();
    // MOOSE_SLINT_SNAPSHOT_WAIT gives a real library time to update and draw
    // its covers first.
    let wait = std::env::var("MOOSE_SLINT_SNAPSHOT_WAIT").ok().and_then(|s| s.parse().ok()).unwrap_or(1200);
    slint::Timer::single_shot(std::time::Duration::from_millis(wait), move || {
        let ui = weak.unwrap();
        let saved = ui.window().take_snapshot().map_err(std::io::Error::other).and_then(|frame| save_png(&path, &frame));
        if let Err(e) = saved {
            eprintln!("snapshot: {e}");
        }
        let _ = slint::quit_event_loop();
    });
}

fn save_png(path: &str, frame: &slint::SharedPixelBuffer<slint::Rgba8Pixel>) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, frame.width(), frame.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(frame.as_bytes()).map_err(std::io::Error::other)
}

thread_local! {
    /// Console pictures already read, by file, so redrawing the consoles
    /// doesn't read them again. A few dozen small files.
    static LOGOS: RefCell<std::collections::HashMap<String, slint::Image>> = RefCell::new(Default::default());
}

/// A console picture, read once. SVG, PNG and WebP, as the themes ship them.
fn logo_image(path: &str) -> Option<slint::Image> {
    LOGOS.with(|cache| {
        if let Some(image) = cache.borrow().get(path) {
            return Some(image.clone());
        }
        let image = slint::Image::load_from_path(std::path::Path::new(path)).ok()?;
        cache.borrow_mut().insert(path.to_owned(), image.clone());
        Some(image)
    })
}

/// The consoles column, and Sofa's console tiles.
fn show_platforms(ui: &AppWindow) {
    let rows: Vec<PlatformItem> = with_app(|a| {
        a.platforms
            .iter()
            .map(|p| {
                let logo = p.logo.as_deref().and_then(logo_image);
                PlatformItem {
                    name: p.name.as_str().into(),
                    short: p.short.as_str().into(),
                    count: p.games.len() as i32,
                    tint: rgb(p.rgb),
                    has_logo: logo.is_some(),
                    logo: logo.unwrap_or_default(),
                }
            })
            .collect()
    })
    .unwrap_or_default();
    ui.set_platforms(ModelRc::new(VecModel::from(rows)));
}

/// The console picture styles that have pictures, in the core's order. A
/// style with none would turn every tile back into letters, so it isn't
/// offered, as the old app didn't.
pub(crate) fn icon_styles() -> Vec<moose_rack::commands::IconStyleView> {
    let Some(lib) = with_app(|a| a.lib.clone()).flatten() else { return Vec::new() };
    moose_rack::commands::icon_styles(&lib).unwrap_or_default().into_iter().filter(|s| s.available > 0).collect()
}

/// Settings' picture rows say what is chosen now.
fn show_picture_labels(ui: &AppWindow) {
    let now = list_art();
    let game = moose_rack::media::LIST_ART_CHOICES
        .iter()
        .find(|&&(key, _)| Some(key) == now.as_deref())
        .map_or("Cartridge or disc", |&(_, label)| label);
    ui.set_game_pictures(game.into());
    let styles = icon_styles();
    match styles.iter().find(|s| s.selected).or(styles.first()) {
        Some(s) => {
            ui.set_console_pictures(s.label.as_str().into());
            ui.set_console_pictures_hint(format!("{} consoles have this picture", s.available).into());
        }
        None => {
            ui.set_console_pictures("None yet".into());
            ui.set_console_pictures_hint("Get them below first".into());
        }
    }
}

/// Re-read every console's picture from the core, after the style or the
/// set changed.
fn reload_logos(ui: &AppWindow) {
    let Some(lib) = with_app(|a| a.lib.clone()).flatten() else { return };
    let Ok(list) = moose_rack::commands::platforms(&lib) else { return };
    let logos: std::collections::HashMap<String, Option<String>> = list.into_iter().map(|p| (p.slug, p.logo)).collect();
    with_app(|a| {
        for p in &mut a.platforms {
            p.logo = logos.get(&p.slug).cloned().flatten();
        }
    });
    show_platforms(ui);
}

/// The next or previous game picture, from Settings.
pub(crate) fn step_list_art(ui: &AppWindow, by: i32) {
    let choices = moose_rack::media::LIST_ART_CHOICES;
    let now = list_art();
    let at = choices.iter().position(|&(key, _)| Some(key) == now.as_deref()).unwrap_or(0) as i32;
    let next = (at + by).rem_euclid(choices.len() as i32) as usize;
    let (key, label) = choices[next];
    set_list_art(ui, key, label);
}

/// The next or previous console picture style, from Settings. With none to
/// step through, it offers to get them.
pub(crate) fn step_icon_style(ui: &AppWindow, by: i32) {
    let styles = icon_styles();
    if styles.is_empty() {
        ui.invoke_show_toast("No console pictures yet: Get console pictures, below".into());
        return;
    }
    let at = styles.iter().position(|s| s.selected).unwrap_or(0) as i32;
    let next = (at + by).rem_euclid(styles.len() as i32) as usize;
    set_icon_style(ui, &styles[next].key);
}

/// Draw the consoles in another style, kept in config.toml by the core.
pub(crate) fn set_icon_style(ui: &AppWindow, key: &str) {
    let Some(lib) = with_app(|a| a.lib.clone()).flatten() else { return };
    match moose_rack::commands::set_icon_style(&lib, key.to_owned()) {
        Ok(label) => {
            reload_logos(ui);
            show_picture_labels(ui);
            ui.invoke_show_toast(format!("Console pictures: {label}").into());
        }
        Err(e) => ui.invoke_show_toast(e.into()),
    }
}

/// "Get console pictures", as the old app's Appearance pane had it: the
/// chosen set's pictures, or the default set's when none was chosen
/// (`commands::fetch_icons`), counted out as they arrive.
pub(crate) fn get_console_pictures(ui: &AppWindow) {
    if ui.get_console_pictures_busy() {
        return;
    }
    let Some((lib, rt)) = with_app(|a| a.lib.clone().zip(a.runtime.clone())).flatten() else {
        ui.invoke_show_toast("No library to get console pictures for".into());
        return;
    };
    ui.set_console_pictures_busy(true);
    let weak = ui.as_weak();
    rt.spawn(async move {
        let progress = {
            let weak = weak.clone();
            move |what: &str| {
                let what = format!("Console pictures: {what}");
                let _ = weak.upgrade_in_event_loop(move |ui| ui.invoke_show_toast(what.into()));
            }
        };
        let result = moose_rack::commands::fetch_icons(&lib, &progress).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_console_pictures_busy(false);
            match result {
                Ok(summary) => {
                    // Files at the same paths may have been replaced.
                    LOGOS.with(|cache| cache.borrow_mut().clear());
                    reload_logos(&ui);
                    show_picture_labels(&ui);
                    ui.invoke_show_toast(summary.lines().next().unwrap_or("Console pictures fetched").to_owned().into());
                }
                Err(e) => ui.invoke_show_toast(format!("Couldn't get console pictures: {e}").into()),
            }
        });
    });
}

/// Runs MOOSE_SLINT_TEST_KEYS: one step every 200 ms, the first a second in.
fn test_keys(ui: &AppWindow, keys: String) -> slint::Timer {
    // Quiet: these run on machines nobody is listening to.
    ui.set_opt_sounds(false);
    let steps: Vec<String> = keys.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
    let weak = ui.as_weak();
    let mut next = 0usize;
    let started = Instant::now();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
        let Some(ui) = weak.upgrade() else { return };
        if started.elapsed() < Duration::from_secs(1) {
            return;
        }
        let Some(step) = steps.get(next) else {
            let _ = slint::quit_event_loop();
            return;
        };
        next += 1;
        match step.strip_prefix("type:") {
            Some(text) => {
                ui.set_query(text.into());
                ui.invoke_search_edited(text.into());
            }
            // As if a controller were plugged in.
            None if step == "pad" => ui.set_pad_connected(true),
            // The offline question, as if a launch had stopped on it.
            None if step == "ask" => stopped(&ui, 0, Default::default(), "SAVE_OFFLINE:test"),
            None => {
                nav::dispatch(&ui, step);
            }
        }
        let sel = ui.get_selected_index();
        let title = ui.get_games().row_data(sel.max(0) as usize).map(|g| g.title.to_string()).unwrap_or_default();
        println!(
            "{step:>12}  zone {} at {}  header {}  settings {}  ask {}  mode {} list {} sort {}  {} | {sel} {title}",
            ui.get_focus_zone(),
            ui.get_focus_index(),
            ui.get_header_at(),
            ui.get_settings_open(),
            if ui.get_ask_open() { ui.get_ask_at() } else { -1 },
            ui.get_mode(),
            ui.get_list_view(),
            ui.get_sort_label(),
            ui.get_heading(),
        );
    });
    timer
}

/// The filter menu's switches: only on this device, only favorites.
fn filters() -> (bool, bool) {
    with_app(|a| (a.state.only_device, a.state.only_favorites)).unwrap_or_default()
}

/// Flip one of the filter menu's switches ("device" or "favorites") and show
/// the list it leaves, the cursor staying on its game if that is still in it.
fn toggle_filter(ui: &AppWindow, which: &str) -> (bool, bool) {
    with_app(|a| match which {
        "device" => a.state.only_device = !a.state.only_device,
        _ => a.state.only_favorites = !a.state.only_favorites,
    });
    refresh(ui, Land::Same);
    filters()
}

/// Rebuild the grid from the state, with the cursor where `land` says.
fn refresh(ui: &AppWindow, land: Land) {
    let at = ui.get_selected_index().max(0) as usize;
    let Some((heading, rows, cursor, sort, platform)) = with_app(|a| {
        let s = &mut a.state;
        let platforms = &a.platforms;
        // The list being left remembers its game.
        let was = s.shown_ids.get(at).copied();
        if let (Some(key), Some(id)) = (s.shown_key.take(), was) {
            s.remembered.insert(key, id);
        }
        if s.platform >= platforms.len() {
            s.platform = 0;
        }
        let query = s.query.to_lowercase();
        let mut shown: Vec<(usize, usize)> = Vec::new();
        for (pi, p) in platforms.iter().enumerate() {
            for (gi, g) in p.games.iter().enumerate() {
                let wanted = if !query.is_empty() {
                    g.title.to_lowercase().contains(&query)
                } else {
                    match s.tab {
                        TAB_RECENT => g.last_played.is_some(),
                        TAB_FAVORITES => g.favorite,
                        TAB_DEVICE => g.installed,
                        _ => pi == s.platform,
                    }
                };
                let passes = (!s.only_device || g.installed) && (!s.only_favorites || g.favorite);
                if wanted && passes {
                    shown.push((pi, gi));
                }
            }
        }
        let game = |&(pi, gi): &(usize, usize)| &platforms[pi].games[gi];
        // Recent is always newest first; it is what the tab is for.
        let sort = if s.tab == TAB_RECENT && query.is_empty() { Sort::Played } else { s.sort };
        shown.sort_by_cached_key(|k| game(k).title.to_lowercase());
        match sort {
            Sort::Name => {}
            Sort::Year => shown.sort_by_key(|k| game(k).year),
            // Newest first, then never played, each in name order (stable sort).
            Sort::Played => shown.sort_by(|a, b| game(b).last_played.cmp(&game(a).last_played)),
        }
        let heading = if !query.is_empty() {
            format!("Results for \"{}\"", s.query)
        } else {
            match s.tab {
                TAB_RECENT => "Recently played".to_string(),
                TAB_FAVORITES => "Favorites".to_string(),
                TAB_DEVICE => "On this device".to_string(),
                _ => match (platforms.get(s.platform), &a.library_error) {
                    (Some(p), _) => p.name.clone(),
                    (None, Some(why)) => format!("Couldn't load the library: {why}"),
                    (None, None) => "Updating the library…".to_owned(),
                },
            }
        };
        // A filtered list says so, or games would seem to be missing.
        let heading = if s.only_device || s.only_favorites { format!("{heading} (filtered)") } else { heading };
        let rows: Vec<GameItem> = shown.iter().map(|&(pi, gi)| item(&platforms[pi], gi)).collect();
        let ids: Vec<i64> = shown.iter().map(|k| game(k).id).collect();
        let key = if !query.is_empty() {
            None
        } else {
            match s.tab {
                TAB_RECENT => Some("recent".to_owned()),
                TAB_FAVORITES => Some("favorites".to_owned()),
                TAB_DEVICE => Some("device".to_owned()),
                _ => platforms.get(s.platform).map(|p| format!("console:{}", p.slug)),
            }
        };
        let row_of = |id: i64| ids.iter().position(|&x| x == id);
        let last = ids.len().saturating_sub(1);
        let cursor = match land {
            Land::Remembered => key.as_ref().and_then(|k| s.remembered.get(k)).and_then(|&id| row_of(id)).unwrap_or(0),
            Land::Same => was.and_then(row_of).unwrap_or(at.min(last)),
            Land::Row(row) => row.min(last),
        };
        s.shown = shown;
        s.shown_ids = ids;
        s.shown_key = key;
        // A new list starts at the top; the grid reports where it really is.
        a.visible = (0, NEAR);
        (heading, rows, cursor, sort, s.platform)
    }) else {
        return;
    };
    ui.set_heading(heading.into());
    ui.set_sort_label(sort.label().into());
    ui.set_platform_index(platform as i32);
    ui.set_art_aspect(art_aspect());
    ui.set_games(ModelRc::new(VecModel::from(rows)));
    ui.set_selected_index(cursor as i32);
    request_detail(ui);
}

/// The shape of the cards on show: the console's art shape on its own page,
/// tall box shape elsewhere. Kept between a tall box and a wide cartridge.
fn art_aspect() -> f32 {
    with_app(|a| {
        let on_console = a.state.tab == TAB_CONSOLES && a.state.query.is_empty();
        let shape = if on_console { a.platforms.get(a.state.platform).and_then(|p| p.aspect) } else { None };
        shape.unwrap_or(0.75).clamp(0.6, 1.6)
    })
    .unwrap_or(0.75)
}

/// Draw one row of the grid again, after its game changed.
fn redraw_row(ui: &AppWindow, row: usize) {
    if let Some(Some(it)) = with_app(|a| a.state.shown.get(row).map(|&(pi, gi)| item(&a.platforms[pi], gi))) {
        if row < ui.get_games().row_count() {
            ui.get_games().set_row_data(row, it);
        }
    }
}

/// Mark the game on this row downloaded or not; its title, if there was one.
fn set_installed(row: usize, installed: bool) -> Option<String> {
    with_app(|a| {
        let &(pi, gi) = a.state.shown.get(row)?;
        let g = &mut a.platforms[pi].games[gi];
        g.installed = installed;
        Some(g.title.clone())
    })
    .flatten()
}

fn item(p: &Platform, gi: usize) -> GameItem {
    let g = &p.games[gi];
    GameItem {
        title: g.title.as_str().into(),
        platform: p.name.as_str().into(),
        badge: p.short.as_str().into(),
        year: g.year.map(|y| y.to_string()).unwrap_or_default().into(),
        size: human(g.bytes).into(),
        tint: tint(p, &g.title),
        installed: g.installed,
        favorite: g.favorite,
        cover: g.cover.clone().unwrap_or_default(),
        has_cover: g.cover.is_some(),
    }
}

/// Update the library in the background, then show what it found. The
/// cache is shown first, so the window never waits on the network.
fn update_library(ui: &AppWindow) {
    let Some((lib, rt)) = with_app(|a| a.lib.clone().zip(a.runtime.clone())).flatten() else { return };
    let weak = ui.as_weak();
    rt.spawn(async move {
        let progress = weak.clone();
        let said = moose_rack::commands::sync_library(&lib, false, move |what| {
            let what = what.to_owned();
            eprintln!("library: {what}");
            let _ = progress.upgrade_in_event_loop(move |ui| ui.invoke_show_toast(format!("Library: {what}").into()));
        })
        .await;
        match &said {
            Ok(line) => eprintln!("library: {line}"),
            Err(e) => eprintln!("library: update failed: {e}"),
        }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let fresh = library::load(&lib);
            with_app(|a| {
                // Stay on the same console, found by name, as the list may
                // have gained or lost consoles before it.
                let slug = a.platforms.get(a.state.platform).map(|p| p.slug.clone());
                // Keep the covers already drawn and the shapes already measured:
                // the fresh list has neither, and a cover marked as asked for
                // would never be asked for again.
                let mut fresh = fresh;
                let mut kept: std::collections::HashMap<i64, slint::Image> = std::collections::HashMap::new();
                let mut shapes: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
                for p in a.platforms.drain(..) {
                    if let Some(s) = p.aspect {
                        shapes.insert(p.slug.clone(), s);
                    }
                    kept.extend(p.games.into_iter().filter_map(|g| Some((g.id, g.cover?))));
                }
                for p in &mut fresh {
                    if p.aspect.is_none() {
                        p.aspect = shapes.get(&p.slug).copied();
                    }
                    for g in &mut p.games {
                        g.cover = kept.remove(&g.id);
                    }
                }
                a.requested = fresh.iter().flat_map(|p| p.games.iter()).filter(|g| g.cover.is_some()).map(|g| g.id).collect();
                a.platforms = fresh;
                a.state.platform = slug.and_then(|s| a.platforms.iter().position(|p| p.slug == s)).unwrap_or(0);
            });
            let empty = with_app(|a| {
                a.library_error = said.as_ref().err().map(|e| plain_error(e));
                a.platforms.is_empty()
            })
            .unwrap_or(true);
            show_platforms(&ui);
            // The update runs in the background: the cursor stays on its game.
            refresh(&ui, Land::Same);
            fetch_covers(&ui);
            match said {
                Ok(line) => {
                    // "Synced 20:41": when this device last heard from the server.
                    let now = moose_rack::util::now_iso();
                    ui.set_status(format!("Synced {}", now.get(11..16).unwrap_or("")).into());
                    ui.invoke_show_toast(line.into())
                }
                Err(e) => ui.invoke_show_toast(format!("Library update failed: {}", plain_error(&e)).into()),
            }
            // Nothing to show yet: try again, so allowing the network in System
            // Settings, or the server coming back, fills the window by itself.
            if empty {
                let again = ui.as_weak();
                slint::Timer::single_shot(Duration::from_secs(15), move || {
                    if let Some(ui) = again.upgrade() {
                        update_library(&ui);
                    }
                });
            }
        });
    });
}

/// How many games either side of the screen get their cover loaded.
const NEAR: usize = 24;
/// How far from the screen a loaded cover is kept before it's dropped.
const KEEP: usize = 72;
/// Covers are shrunk to this many pixels wide, about a card on a Retina
/// screen, before they reach the window: full-size art is several times that.
const COVER_WIDTH: u32 = 360;

/// Load the covers of the games on screen and near it that have none yet, the
/// cache first and the server second as the desktop app does, and drop the
/// ones far from the screen. Each batch is shrunk on a background thread and
/// drawn as it arrives.
fn fetch_covers(ui: &AppWindow) {
    let Some((lib, rt, ids, dropped, art_gen)) = with_app(|a| {
        let (first, last) = a.visible;
        let near = first.saturating_sub(NEAR)..=last + NEAR;
        let keep = first.saturating_sub(KEEP)..=last + KEEP;
        let kept: std::collections::HashSet<(usize, usize)> =
            a.state.shown.iter().enumerate().filter(|(row, _)| keep.contains(row)).map(|(_, &k)| k).collect();
        // Drop covers far from the screen, on this console or any other.
        let mut dropped = Vec::new();
        for (pi, p) in a.platforms.iter_mut().enumerate() {
            for (gi, g) in p.games.iter_mut().enumerate() {
                if !kept.contains(&(pi, gi)) && (g.cover.is_some() || a.requested.contains(&g.id)) {
                    g.cover = None;
                    a.requested.remove(&g.id);
                    if let Some(row) = a.state.shown.iter().position(|&k| k == (pi, gi)) {
                        dropped.push(row);
                    }
                }
            }
        }
        let mut ids = Vec::new();
        for (row, &(pi, gi)) in a.state.shown.iter().enumerate() {
            let g = &a.platforms[pi].games[gi];
            if near.contains(&row) && g.cover.is_none() && a.requested.insert(g.id) {
                ids.push(g.id);
            }
        }
        a.lib.clone().zip(a.runtime.clone()).map(|(l, r)| (l, r, ids, dropped, a.art_gen))
    })
    .flatten() else {
        return;
    };
    for row in dropped {
        redraw_row(ui, row);
    }
    if ids.is_empty() {
        return;
    }
    let weak = ui.as_weak();
    rt.spawn(async move {
        let shrink = |found: Vec<(i64, String)>, weak: slint::Weak<AppWindow>| {
            tokio::task::spawn_blocking(move || {
                let pictures: Vec<(i64, slint::SharedPixelBuffer<slint::Rgba8Pixel>)> =
                    found.into_iter().filter_map(|(id, path)| Some((id, shrunk(&path)?))).collect();
                let _ = weak.upgrade_in_event_loop(move |ui| apply_covers(&ui, art_gen, pictures));
            });
        };
        let each = weak.clone();
        let emit = move |batch: &[moose_rack::commands::CoverView]| {
            let found: Vec<(i64, String)> = batch.iter().filter_map(|c| Some((c.id, c.cover.clone()?))).collect();
            shrink(found, each.clone());
        };
        if let Ok(all) = moose_rack::commands::rom_covers(&lib, ids, None, &emit).await {
            let found: Vec<(i64, String)> = all.into_iter().filter_map(|c| Some((c.id, c.cover?))).collect();
            shrink(found, weak);
        }
    });
}

/// The game picture in use: "physicalmedia", "covers", "miximages"...
pub(crate) fn list_art() -> Option<String> {
    let lib = with_app(|a| a.lib.clone()).flatten()?;
    moose_rack::commands::list_art_options(&lib).ok().map(|(_, now)| now)
}

/// Show `value` on every game's card from now on, as the old app's Pictures
/// button did (ui/js/pictures.js): kept in config.toml by the core, and every
/// cover drawn again in the new kind. Each console's card shape is measured
/// again from the new pictures, since box art and cartridges differ.
pub(crate) fn set_list_art(ui: &AppWindow, value: &str, label: &str) {
    let Some(lib) = with_app(|a| a.lib.clone()).flatten() else {
        ui.invoke_show_toast("No library to change the pictures of".into());
        return;
    };
    if let Err(e) = moose_rack::commands::set_list_art(&lib, value.to_owned()) {
        ui.invoke_show_toast(e.into());
        return;
    }
    with_app(|a| {
        a.art_gen += 1;
        a.requested.clear();
        for p in &mut a.platforms {
            p.aspect = None;
            for g in &mut p.games {
                g.cover = None;
            }
        }
    });
    refresh(ui, Land::Same);
    fetch_covers(ui);
    show_picture_labels(ui);
    ui.invoke_show_toast(format!("Game pictures: {label}").into());
}

/// A cover file, shrunk to `COVER_WIDTH` wide, ready for the window.
fn shrunk(path: &str) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    shrunk_to(path, COVER_WIDTH)
}

/// A picture file, shrunk to at most `width` pixels wide.
fn shrunk_to(path: &str, width: u32) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let picture = image::open(path).ok()?;
    let small = if picture.width() > width { picture.thumbnail(width, u32::MAX) } else { picture };
    let rgba = small.to_rgba8();
    Some(slint::SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}

/// Draw covers that arrived, if their games are still wanted.
fn apply_covers(ui: &AppWindow, art_gen: u64, pictures: Vec<(i64, slint::SharedPixelBuffer<slint::Rgba8Pixel>)>) {
    let rows = with_app(|a| {
        let mut rows = Vec::new();
        // The pictures changed kind while these were on their way.
        if art_gen != a.art_gen {
            return (rows, false);
        }
        for (id, pixels) in pictures {
            // Dropped while it was on its way: the screen has moved on.
            if !a.requested.contains(&id) {
                continue;
            }
            let at = a.platforms.iter().enumerate().find_map(|(pi, p)| {
                p.games.iter().position(|g| g.id == id).map(|gi| (pi, gi))
            });
            let Some((pi, gi)) = at else { continue };
            a.platforms[pi].games[gi].cover = Some(slint::Image::from_rgba8(pixels));
            if let Some(row) = a.state.shown.iter().position(|&k| k == (pi, gi)) {
                rows.push(row);
            }
        }
        // A console whose art shape isn't known yet takes the middle shape of
        // its first six covers.
        let mut reshaped = false;
        for p in &mut a.platforms {
            if p.aspect.is_some() {
                continue;
            }
            let mut shapes: Vec<f32> = p
                .games
                .iter()
                .filter_map(|g| g.cover.as_ref())
                .map(|c| c.size())
                .filter(|s| s.height > 0)
                .map(|s| s.width as f32 / s.height as f32)
                .collect();
            if shapes.len() >= 6 {
                shapes.sort_by(f32::total_cmp);
                p.aspect = Some(shapes[shapes.len() / 2]);
                reshaped = true;
            }
        }
        (rows, reshaped)
    })
    .unwrap_or_default();
    let (rows, reshaped) = rows;
    if reshaped {
        ui.set_art_aspect(art_aspect());
    }
    for row in rows {
        redraw_row(ui, row);
    }
}

/// The platform's colour, pulled toward one of a few hues by the title, so the
/// stand-in covers on one shelf are related but not all the same.
fn tint(p: &Platform, title: &str) -> Color {
    const HUES: [(u8, u8, u8); 8] = [
        (220, 70, 70),
        (230, 150, 40),
        (70, 170, 90),
        (40, 150, 200),
        (120, 90, 220),
        (200, 70, 160),
        (60, 120, 220),
        (180, 160, 60),
    ];
    let h = HUES[(hash(title) % HUES.len() as u64) as usize];
    let mix = |a: u8, b: u8| ((a as u16 * 55 + b as u16 * 45) / 100) as u8;
    rgb((mix(p.rgb.0, h.0), mix(p.rgb.1, h.1), mix(p.rgb.2, h.2)))
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::from_rgb_u8(r, g, b)
}

fn random() -> u64 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    hash(&nanos.to_string())
}

/// Play: the shared launch checklist first (docs/launch-flow.html). When the
/// game and its emulator are both here the game starts with no window; when
/// something is missing the checklist window shows it, with its size.
fn play_game(ui: &AppWindow, id: i64) {
    let Some((lib, rt, busy)) = with_app(|a| Some((a.lib.clone()?, a.runtime.clone()?, a.busy.clone()))).flatten() else {
        ui.invoke_show_toast("Playing needs the real library; this is the sample".into());
        return;
    };
    if let Some(moose_rack::busy::Hold::Playing(name)) = busy.hold() {
        ui.invoke_show_toast(format!("{name} is already running").into());
        return;
    }
    let weak = ui.as_weak();
    rt.spawn(async move {
        let list = moose_rack::play::checklist(&lib, id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| match list {
            Ok(list) if list.ready => start_launch(&ui, id),
            Ok(list) => show_checklist(&ui, id, &list),
            // A check that can't run is no reason to refuse: the launch says
            // what is really wrong.
            Err(_) => start_launch(&ui, id),
        });
    });
}

/// Open the checklist window for this game, or bring it up to date.
fn show_checklist(ui: &AppWindow, id: i64, list: &moose_rack::play::Checklist) {
    let platform = with_app(|a| {
        a.checking = Some(id);
        a.platforms.iter().find(|p| p.slug == list.platform).map(|p| p.name.clone())
    })
    .flatten()
    .unwrap_or_else(|| list.platform.clone());
    let line = |title: &str, item: &moose_rack::play::Item| CheckLine {
        title: title.into(),
        detail: format!("{}: {}", item.label, item.detail).into(),
        size: item.bytes.map(human).unwrap_or_default().into(),
        ready: item.ready,
    };
    let button = if list.blocked.is_some() {
        String::new()
    } else if list.download_bytes > 0 {
        format!("Download {}", human(list.download_bytes))
    } else {
        "Set up".to_owned()
    };
    ui.set_checklist_game(list.name.as_str().into());
    ui.set_checklist_platform(platform.into());
    ui.set_checklist_lines(ModelRc::new(VecModel::from(vec![line("Game file", &list.game), line("Emulator", &list.emulator)])));
    ui.set_checklist_button(button.into());
    ui.set_checklist_blocked(list.blocked.clone().unwrap_or_default().into());
    ui.set_checklist_progress(SharedString::new());
    ui.set_checklist_at(0);
    ui.set_checklist_open(true);
    if std::env::var_os("MOOSE_SLINT_TEST_ACCEPT").is_some() && list.blocked.is_none() {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(1500), move || {
            if let Some(ui) = weak.upgrade() {
                eprintln!("test: pressing {}", ui.get_checklist_button());
                checklist_download(&ui);
            }
        });
    }
}

/// The checklist's Download button: fetch everything missing through the
/// shared core, then check again; ready means the game starts.
fn checklist_download(ui: &AppWindow) {
    let Some((lib, rt, id)) = with_app(|a| Some((a.lib.clone()?, a.runtime.clone()?, a.checking?))).flatten() else { return };
    ui.set_checklist_progress("Starting…".into());
    let weak = ui.as_weak();
    rt.spawn(async move {
        let progress = weak.clone();
        let fetched = moose_rack::play::prepare(&lib, id, move |what| {
            let what = what.to_owned();
            let _ = progress.upgrade_in_event_loop(move |ui| ui.set_checklist_progress(what.into()));
        })
        .await;
        let list = moose_rack::play::checklist(&lib, id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            // Cancelled while it fetched: say it's done, start nothing.
            let still_asked = with_app(|a| a.checking == Some(id)).unwrap_or(false);
            mark_installed(&ui, id, list.as_ref().is_ok_and(|l| l.game.ready));
            match (fetched, list) {
                (Err(e), _) => {
                    eprintln!("checklist: download failed: {e}");
                    ui.set_checklist_progress(SharedString::new());
                    ui.invoke_show_toast(format!("Download failed: {e}").into());
                }
                (Ok(_), Ok(list)) if list.ready && still_asked => {
                    with_app(|a| a.checking = None);
                    ui.set_checklist_open(false);
                    start_launch(&ui, id);
                }
                (Ok(_), Ok(list)) if still_asked => show_checklist(&ui, id, &list),
                (Ok(said), _) => ui.invoke_show_toast(if said.is_empty() { "Ready".to_owned() } else { said }.into()),
            }
        });
    });
}

/// Mark a game downloaded or not wherever it is shown.
fn mark_installed(ui: &AppWindow, id: i64, installed: bool) {
    let row = with_app(|a| {
        let (pi, gi) = a.platforms.iter().enumerate().find_map(|(pi, p)| p.games.iter().position(|g| g.id == id).map(|gi| (pi, gi)))?;
        a.platforms[pi].games[gi].installed = installed;
        a.state.shown.iter().position(|&k| k == (pi, gi))
    })
    .flatten();
    if let Some(row) = row {
        redraw_row(ui, row);
    }
}

/// Whether the controller drives this window, as the desktop app decides it
/// (ui/js/gamepad.js, `suspendPad` and `resumePad`).
///
/// Off from the moment a launch starts: SDL reads the pad even with RetroArch
/// in front (it has to, this window has no SDL window of its own to focus), so
/// without this every button pressed in the game also moved the cursor here,
/// and the combination that quits the game launched it again behind it.
/// Back on once the game has ended, the pad is at rest and the short floor
/// in `padpoll::settle_lifted` has passed, because the buttons that quit are
/// still held when this window gets them back.
#[derive(Clone, Copy)]
#[cfg_attr(target_os = "android", allow(dead_code))]
enum PadGate {
    Open,
    /// A launch is running: the emulator has the pad.
    Suspended,
    /// Back from a game: ignored until nothing is held and this has passed.
    Settling(Instant),
}

thread_local! {
    static PAD_GATE: std::cell::Cell<PadGate> = const { std::cell::Cell::new(PadGate::Open) };
}

/// Lifts the pad lock when a launch thread ends however it ends: played,
/// stopped on a question, failed, or panicked. A lock left on would leave the
/// controller dead until the app restarts.
struct ResumePad(slint::Weak<AppWindow>);

impl Drop for ResumePad {
    fn drop(&mut self) {
        let _ = self.0.upgrade_in_event_loop(|_| {
            let floor = Duration::from_millis(padpoll::SETTLE_FLOOR_MS as u64);
            PAD_GATE.set(PadGate::Settling(Instant::now() + floor));
        });
    }
}

/// Start the game through the shared launch (`moose_rack::play::launch`):
/// saves checked, the emulator run and waited for, saves uploaded, the launch
/// recorded. On a thread of its own, because the emulator run blocks until the
/// game ends.
fn start_launch(ui: &AppWindow, id: i64) {
    let opts = moose_rack::play::Options {
        // The controller in hand, so RetroArch's hotkeys follow its autoconfig
        // profile, as the desktop app passes the pad its page sees.
        pad: ui.get_pad_connected().then(|| ui.get_pad_name().to_string()),
        ..Default::default()
    };
    start_launch_with(ui, id, opts);
}

/// Launch with the answers given so far (play while saves can't sync, play
/// without the BIOS). A stop that needs an answer comes back as a question.
fn start_launch_with(ui: &AppWindow, id: i64, opts: moose_rack::play::Options) {
    let Some((lib, busy)) = with_app(|a| Some((a.lib.clone()?, a.busy.clone()))).flatten() else { return };
    let weak = ui.as_weak();
    // Quiet at once, before the thread starts: the press that launched is
    // still down.
    PAD_GATE.set(PadGate::Suspended);
    let resume = ResumePad(weak.clone());
    std::thread::spawn(move || {
        let _resume = resume;
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        let progress = weak.clone();
        let say = move |what: &str| {
            let what = what.to_owned();
            let _ = progress.upgrade_in_event_loop(move |ui| ui.invoke_show_toast(what.into()));
        };
        let result = rt.block_on(moose_rack::play::launch(&lib, &busy, id, opts.clone(), say));
        let _ = weak.upgrade_in_event_loop(move |ui| match result {
            Ok(line) => {
                mark_played(&ui, id);
                ui.invoke_show_toast(line.into());
            }
            Err(e) => stopped(&ui, id, opts, &e),
        });
    });
}

/// A game just played goes to the front of Recent at once, without waiting
/// for the next library update. The time is read back from the cache, where
/// the core wrote it, so its rule holds here too: a session under a minute
/// is not a play (`Cache::record_play`).
fn mark_played(ui: &AppWindow, id: i64) {
    let shown = with_app(|a| {
        let played = a.lib.as_ref()?.cache.lock().ok()?.rom_by_id(id).ok()??.last_played;
        for g in a.platforms.iter_mut().flat_map(|p| p.games.iter_mut()).filter(|g| g.id == id) {
            g.last_played = played.clone();
        }
        Some(a.state.tab == TAB_RECENT || a.state.sort == Sort::Played)
    })
    .flatten()
    .unwrap_or(false);
    if shown {
        refresh(ui, Land::Same);
    }
}

/// What a question is about, kept until it is answered.
enum Asking {
    /// Which copy of each conflicting save to keep, one at a time.
    Conflict { id: i64, opts: moose_rack::play::Options, conflicts: Vec<moose_rack::savesync::SaveConflict> },
    /// Play while saves can't be checked with the server?
    Offline { id: i64, opts: moose_rack::play::Options },
    /// Play with a BIOS missing?
    Bios { id: i64, opts: moose_rack::play::Options },
}

thread_local! {
    static ASKING: RefCell<Option<Asking>> = const { RefCell::new(None) };
}

/// A launch stopped. The stops that need an answer are asked, as the desktop
/// app's dialogs ask them (ui/js/conflicts.js), and as Steam asks about cloud
/// saves rather than guessing; anything else is said.
fn stopped(ui: &AppWindow, id: i64, opts: moose_rack::play::Options, e: &str) {
    if e.starts_with("SAVE_CONFLICT:") {
        let conflicts = with_app(|a| a.lib.as_ref().and_then(|l| l.pending_conflicts.lock().ok().map(|c| c.clone())))
            .flatten()
            .unwrap_or_default();
        if !conflicts.is_empty() {
            ASKING.with(|q| *q.borrow_mut() = Some(Asking::Conflict { id, opts, conflicts }));
            ask_next(ui);
            return;
        }
    } else if let Some(why) = e.strip_prefix("SAVE_OFFLINE:") {
        ASKING.with(|q| *q.borrow_mut() = Some(Asking::Offline { id, opts }));
        ask(
            ui,
            "Saves are not syncing",
            "Your saves could not be checked against the server.",
            why.trim(),
            "You can play, but progress won't be uploaded when you quit, and this device may not have the newest save.",
            ("Play anyway", "saves stay on this device"),
            ("Cancel", "don't launch"),
        );
        return;
    } else if let Some(what) = e.strip_prefix("BIOS_MISSING:") {
        ASKING.with(|q| *q.borrow_mut() = Some(Asking::Bios { id, opts }));
        ask(
            ui,
            "A BIOS file is missing",
            "This console needs a BIOS file that isn't on this device, and the server doesn't have it either.",
            what.trim(),
            "Games usually show a black screen without it. Some emulators run anyway, so it's worth a try.",
            ("Play anyway", "may show a black screen"),
            ("Cancel", "don't launch"),
        );
        return;
    }
    ui.invoke_show_toast(format!("Launch failed: {e}").into());
}

/// Put a question up, on its first answer.
fn ask(ui: &AppWindow, title: &str, lead: &str, detail: &str, note: &str, a: (&str, &str), b: (&str, &str)) {
    ui.set_ask_title(title.into());
    ui.set_ask_lead(lead.into());
    ui.set_ask_detail(detail.into());
    ui.set_ask_note(note.into());
    ui.set_ask_a(a.0.into());
    ui.set_ask_a_sub(a.1.into());
    ui.set_ask_b(b.0.into());
    ui.set_ask_b_sub(b.1.into());
    ui.set_ask_at(0);
    ui.set_ask_open(true);
}

/// Ask about the next unanswered save conflict, or launch once there are none.
fn ask_next(ui: &AppWindow) {
    let next = ASKING.with(|q| match &*q.borrow() {
        Some(Asking::Conflict { conflicts, .. }) => conflicts.first().cloned(),
        _ => None,
    });
    let Some(c) = next else {
        ui.set_ask_open(false);
        if let Some(Asking::Conflict { id, opts, .. }) = ASKING.with(|q| q.borrow_mut().take()) {
            start_launch_with(ui, id, opts);
        }
        return;
    };
    let when = |t: &Option<String>| match t {
        Some(t) => format!("saved {}", t.replace('T', " ").replace('Z', " UTC")),
        None => "date unknown".to_owned(),
    };
    let here = match c.local_bytes {
        0 => when(&c.local_updated),
        n => format!("{}, {}", when(&c.local_updated), human(n as u64)),
    };
    ask(
        ui,
        "Which save do you want to keep?",
        &format!("{} changed on this device and on the server since they last matched. The game waits until you choose.", c.file_name),
        c.reason.as_deref().unwrap_or(""),
        "The copy you don't keep is backed up to library/saves-backup/ first, so this can be undone.",
        ("This device", &here),
        ("Server", &when(&c.server_updated)),
    );
}

/// The answer to the open question: 0 or 1 for its two choices, -1 to cancel.
fn answered(ui: &AppWindow, choice: i32) {
    let asking = ASKING.with(|q| q.borrow_mut().take());
    match asking {
        Some(Asking::Conflict { id, opts, conflicts }) if choice >= 0 => {
            let Some(c) = conflicts.first().cloned() else { return };
            let keep = if choice == 0 { moose_rack::savesync::Keep::Local } else { moose_rack::savesync::Keep::Server };
            let Some((lib, rt)) = with_app(|a| Some((a.lib.clone()?, a.runtime.clone()?))).flatten() else { return };
            ASKING.with(|q| *q.borrow_mut() = Some(Asking::Conflict { id, opts, conflicts }));
            let weak = ui.as_weak();
            rt.spawn(async move {
                let said = moose_rack::commands::resolve_save_conflict(&lib, c.file_name.clone(), keep).await;
                let _ = weak.upgrade_in_event_loop(move |ui| match said {
                    Ok(line) => {
                        ui.invoke_show_toast(line.into());
                        ASKING.with(|q| {
                            if let Some(Asking::Conflict { conflicts, .. }) = &mut *q.borrow_mut() {
                                conflicts.retain(|x| x.file_name != c.file_name);
                            }
                        });
                        ask_next(&ui);
                    }
                    // The save is still unresolved, so the question stays up.
                    Err(e) => ui.invoke_show_toast(format!("Couldn't keep that copy: {e}").into()),
                });
            });
        }
        Some(Asking::Offline { id, mut opts }) if choice == 0 => {
            ui.set_ask_open(false);
            opts.skip_sync = Some(true);
            start_launch_with(ui, id, opts);
        }
        Some(Asking::Bios { id, mut opts }) if choice == 0 => {
            ui.set_ask_open(false);
            opts.skip_bios = Some(true);
            start_launch_with(ui, id, opts);
        }
        Some(Asking::Conflict { .. }) => {
            ui.set_ask_open(false);
            ui.invoke_show_toast("Not launched: the save is still waiting for a choice".into());
        }
        _ => {
            ui.set_ask_open(false);
            ui.invoke_show_toast("Not launched".into());
        }
    }
}

/// A library update failure in words someone can act on.
fn plain_error(e: &str) -> String {
    if e.contains("os error 65") {
        // macOS 15 and later: the app hasn't been allowed onto the LAN.
        "macOS hasn't let Moose Rack onto your local network. Allow it in System Settings > Privacy & Security > Local Network.".to_owned()
    } else if e.contains("tcp connect error") || e.contains("timed out") {
        "the server didn't answer. Moose Rack tries again every 15 seconds.".to_owned()
    } else {
        e.lines().next().unwrap_or(e).to_owned()
    }
}

/// Load the selected game's details for the pane, a moment after the cursor
/// stops, so running down a list doesn't fetch every game it passes.
fn request_detail(ui: &AppWindow) {
    let row = ui.get_selected_index().max(0) as usize;
    let picked = with_app(|a| {
        a.detail_gen += 1;
        let turn = a.detail_gen;
        a.state.shown.get(row).map(|&(pi, gi)| (a.platforms[pi].games[gi].id, turn))
    })
    .flatten();
    clear_detail(ui);
    let Some((id, turn)) = picked else { return };
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(150), move || {
        if with_app(|a| a.detail_gen == turn).unwrap_or(false)
            && let Some(ui) = weak.upgrade()
        {
            load_detail(&ui, id, turn);
        }
    });
}

fn clear_detail(ui: &AppWindow) {
    ui.set_detail_has_art(false);
    ui.set_detail_summary(SharedString::new());
    ui.set_detail_meta(SharedString::new());
    ui.set_detail_emulator(SharedString::new());
    ui.set_detail_shots(ModelRc::default());
}

/// The details from the shared core (`commands::rom_detail`): the fuller
/// artwork the old app showed in its pane, the description, a line of facts,
/// the emulator, and up to three screenshots. Pictures are shrunk off the
/// window's thread.
fn load_detail(ui: &AppWindow, id: i64, turn: u64) {
    let Some((lib, rt)) = with_app(|a| a.lib.clone().zip(a.runtime.clone())).flatten() else { return };
    let weak = ui.as_weak();
    rt.spawn(async move {
        let Ok(d) = moose_rack::commands::rom_detail(&lib, id).await else { return };
        let emulator = moose_rack::standalone::for_platform(&d.platform_slug)
            .map(|e| e.name.to_owned())
            .unwrap_or_else(|| "RetroArch".to_owned());
        let mut facts: Vec<String> = Vec::new();
        if let Some(year) = d.release_year {
            facts.push(year.to_string());
        }
        if !d.genres.is_empty() {
            facts.push(d.genres.iter().take(2).cloned().collect::<Vec<_>>().join(", "));
        }
        if let Some(players) = d.player_count.as_deref().filter(|p| !p.is_empty()) {
            facts.push(if players == "1" { "1 player".to_owned() } else { format!("{players} players") });
        }
        if let Some(maker) = d.companies.first() {
            facts.push(maker.clone());
        }
        let summary = d.summary.clone().unwrap_or_default();
        let (cover, shots) = (d.cover.clone(), d.screenshots.clone());
        let (art, pictures) = tokio::task::spawn_blocking(move || {
            let art = cover.as_deref().and_then(|p| shrunk_to(p, 640));
            let pictures: Vec<_> = shots.iter().take(3).filter_map(|p| shrunk_to(p, 320)).collect();
            (art, pictures)
        })
        .await
        .unwrap_or((None, Vec::new()));
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !with_app(|a| a.detail_gen == turn).unwrap_or(false) {
                return;
            }
            ui.set_detail_has_art(art.is_some());
            if let Some(pixels) = art {
                ui.set_detail_art(slint::Image::from_rgba8(pixels));
            }
            ui.set_detail_summary(summary.into());
            ui.set_detail_meta(facts.join("  ·  ").into());
            ui.set_detail_emulator(emulator.into());
            let shots: Vec<slint::Image> = pictures.into_iter().map(slint::Image::from_rgba8).collect();
            ui.set_detail_shots(ModelRc::new(VecModel::from(shots)));
        });
    });
}

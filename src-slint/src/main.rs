//! A Slint prototype of the Moose Rack front end.
//!
//! It draws the building blocks of the current UI over sample data, so the
//! look, the feel and the memory cost can be judged before anything is ported.
//! The rules a real port keeps in `moose_rack` (sorting, filtering, the grid
//! cursor) are done here in a few lines instead; see README.md.

mod sample;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use slint::{Color, Model, ModelRc, SharedString, VecModel};

use sample::Platform;

slint::include_modules!();

/// What the grid is showing, and the choices that decide it.
struct State {
    platform: usize,
    tab: i32,
    query: String,
    by_year: bool,
    favorites: HashSet<&'static str>,
    installed: HashSet<&'static str>,
    /// Each row of the grid's model, as (platform, game) indices.
    shown: Vec<(usize, usize)>,
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let platforms = Rc::new(sample::platforms());
    let state = Rc::new(RefCell::new(initial_state(&platforms)));

    let rows: Vec<PlatformItem> = platforms
        .iter()
        .map(|p| PlatformItem {
            name: p.name.into(),
            short: p.short.into(),
            count: p.games.len() as i32,
            tint: rgb(p.rgb),
        })
        .collect();
    ui.set_platforms(ModelRc::new(VecModel::from(rows)));
    refresh(&ui, &platforms, &mut state.borrow_mut(), None);

    ui.on_platform_selected({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move |i: i32| {
            let ui = weak.unwrap();
            let mut s = s.borrow_mut();
            s.platform = i as usize;
            s.tab = 0;
            s.query.clear();
            ui.set_tab_index(0);
            ui.set_query(SharedString::new());
            refresh(&ui, &p, &mut s, None);
        }
    });

    ui.on_tab_selected({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move |i: i32| {
            let ui = weak.unwrap();
            let mut s = s.borrow_mut();
            s.tab = i;
            s.query.clear();
            ui.set_query(SharedString::new());
            refresh(&ui, &p, &mut s, None);
        }
    });

    ui.on_search_edited({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move |text: SharedString| {
            let ui = weak.unwrap();
            let mut s = s.borrow_mut();
            s.query = text.trim().to_string();
            refresh(&ui, &p, &mut s, None);
        }
    });

    ui.on_sort_toggled({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move || {
            let ui = weak.unwrap();
            let mut s = s.borrow_mut();
            s.by_year = !s.by_year;
            let keep = s.shown.get(ui.get_selected_index() as usize).copied();
            refresh(&ui, &p, &mut s, keep);
        }
    });

    ui.on_random_pick({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move || {
            let ui = weak.unwrap();
            let s = s.borrow();
            if s.shown.is_empty() {
                return;
            }
            let i = (random() % s.shown.len() as u64) as usize;
            ui.set_selected_index(i as i32);
            let (pi, gi) = s.shown[i];
            ui.invoke_show_toast(format!("Random pick: {}", p[pi].games[gi].title).into());
        }
    });

    ui.on_play({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move |i: i32| {
            let ui = weak.unwrap();
            let s = s.borrow();
            if let Some(&(pi, gi)) = s.shown.get(i as usize) {
                let title = p[pi].games[gi].title;
                ui.invoke_show_toast(format!("Starting {title} (the prototype launches nothing)").into());
            }
        }
    });

    ui.on_favorite_toggled({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move |i: i32| {
            let ui = weak.unwrap();
            let mut s = s.borrow_mut();
            let Some(&(pi, gi)) = s.shown.get(i as usize) else { return };
            let title = p[pi].games[gi].title;
            if !s.favorites.remove(title) {
                s.favorites.insert(title);
            }
            if s.tab == 1 {
                // Unstarring on the Favorites tab takes the game off it.
                refresh(&ui, &p, &mut s, None);
            } else {
                ui.get_games().set_row_data(i as usize, item(&p[pi], gi, &s));
            }
        }
    });

    ui.on_download_finished({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move |i: i32| {
            let ui = weak.unwrap();
            let mut s = s.borrow_mut();
            let Some(&(pi, gi)) = s.shown.get(i as usize) else { return };
            let title = p[pi].games[gi].title;
            s.installed.insert(title);
            ui.get_games().set_row_data(i as usize, item(&p[pi], gi, &s));
            ui.invoke_show_toast(format!("{title} is on this device").into());
        }
    });

    ui.on_removed({
        let (weak, p, s) = (ui.as_weak(), platforms.clone(), state.clone());
        move |i: i32| {
            let ui = weak.unwrap();
            let mut s = s.borrow_mut();
            let Some(&(pi, gi)) = s.shown.get(i as usize) else { return };
            let title = p[pi].games[gi].title;
            s.installed.remove(title);
            if s.tab == 2 {
                refresh(&ui, &p, &mut s, None);
            } else {
                ui.get_games().set_row_data(i as usize, item(&p[pi], gi, &s));
            }
            ui.invoke_show_toast(format!("Removed {title} from this device").into());
        }
    });

    if let Ok(path) = std::env::var("MOOSE_SLINT_SNAPSHOT") {
        snapshot(&ui, path, std::env::var("MOOSE_SLINT_SCENE").unwrap_or_default());
    }

    ui.run()
}

/// `MOOSE_SLINT_SNAPSHOT=out.png` sets up one scene, saves the window as a PNG
/// and quits, so the look can be checked on a machine nobody is watching.
/// `MOOSE_SLINT_SCENE` picks it: desk (the default), sofa, detail, list,
/// settings or download.
fn snapshot(ui: &AppWindow, path: String, scene: String) {
    match scene.as_str() {
        "sofa" => ui.set_mode(0),
        "detail" => {
            ui.set_mode(0);
            ui.set_sofa_detail_open(true);
        }
        "list" => ui.set_list_view(true),
        "settings" => ui.set_settings_open(true),
        "download" => {
            ui.set_selected_index(1);
            ui.set_download_index(1);
            ui.set_download_progress(0.4);
        }
        _ => {}
    }
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(1200), move || {
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

/// A starting point with something in every tab: a few favorites, and about a
/// third of the library already downloaded.
fn initial_state(platforms: &[Platform]) -> State {
    let mut installed = HashSet::new();
    for p in platforms {
        for g in &p.games {
            if hash(g.title) % 3 == 0 {
                installed.insert(g.title);
            }
        }
    }
    let favorites = ["Ratchet & Clank: Up Your Arsenal", "Sly 2: Band of Thieves", "Okami", "Jet Set Radio Future", "Super Metroid"]
        .into_iter()
        .collect();
    State { platform: 0, tab: 0, query: String::new(), by_year: false, favorites, installed, shown: Vec::new() }
}

/// Rebuild the grid from the state. `keep` is a game to leave the cursor on, if
/// it is still in the list.
fn refresh(ui: &AppWindow, platforms: &[Platform], s: &mut State, keep: Option<(usize, usize)>) {
    let query = s.query.to_lowercase();
    let mut shown: Vec<(usize, usize)> = Vec::new();
    for (pi, p) in platforms.iter().enumerate() {
        for (gi, g) in p.games.iter().enumerate() {
            let wanted = if !query.is_empty() {
                g.title.to_lowercase().contains(&query)
            } else {
                match s.tab {
                    1 => s.favorites.contains(g.title),
                    2 => s.installed.contains(g.title),
                    _ => pi == s.platform,
                }
            };
            if wanted {
                shown.push((pi, gi));
            }
        }
    }
    if s.by_year {
        shown.sort_by_key(|&(pi, gi)| (platforms[pi].games[gi].year, platforms[pi].games[gi].title));
    } else {
        shown.sort_by_key(|&(pi, gi)| platforms[pi].games[gi].title.to_lowercase());
    }

    let heading = if !query.is_empty() {
        format!("Results for \"{}\"", s.query)
    } else {
        match s.tab {
            1 => "Favorites".to_string(),
            2 => "On this device".to_string(),
            _ => platforms[s.platform].name.to_string(),
        }
    };

    let rows: Vec<GameItem> = shown.iter().map(|&(pi, gi)| item(&platforms[pi], gi, s)).collect();
    let cursor = keep.and_then(|k| shown.iter().position(|&x| x == k)).unwrap_or(0);
    s.shown = shown;

    ui.set_heading(heading.into());
    ui.set_sort_label(if s.by_year { "Year" } else { "Name" }.into());
    ui.set_platform_index(s.platform as i32);
    ui.set_games(ModelRc::new(VecModel::from(rows)));
    ui.set_selected_index(cursor as i32);
}

fn item(p: &Platform, gi: usize, s: &State) -> GameItem {
    let g = &p.games[gi];
    GameItem {
        title: g.title.into(),
        platform: p.name.into(),
        badge: p.short.into(),
        year: g.year.to_string().into(),
        size: size_of(p, g.title).into(),
        tint: tint(p, g.title),
        installed: s.installed.contains(g.title),
        favorite: s.favorites.contains(g.title),
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

fn size_of(p: &Platform, title: &str) -> String {
    let mb = p.typical_mb as u64 * (60 + hash(title) % 80) / 100;
    if mb >= 1000 {
        format!("{:.1} GB", mb as f64 / 1000.0)
    } else {
        format!("{} MB", mb.max(1))
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::from_rgb_u8(r, g, b)
}

/// FNV-1a: stable across runs, which `DefaultHasher` does not promise.
fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn random() -> u64 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    hash(&nanos.to_string())
}

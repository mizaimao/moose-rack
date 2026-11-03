//! The real library, through the shared core: the same consoles, games and
//! favorites the desktop app shows, read from the same cache and filled by the
//! same library update (`moose_rack::commands`).

use std::sync::Arc;

use moose_rack::app::AppState;

use crate::model::{Game, Platform, hash};

/// The app's state from its config, as the desktop app builds it. `None` with
/// a reason on stderr when there's no config to build it from.
pub fn open() -> Option<Arc<AppState>> {
    moose_rack::datadir::anchor();
    match AppState::from_config() {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            eprintln!("library: no app state, showing the sample: {e:#}");
            None
        }
    }
}

/// Every console with games, and its games, out of the cache.
pub fn load(state: &AppState) -> Vec<Platform> {
    let Ok(list) = moose_rack::commands::platforms(state) else { return Vec::new() };
    list.into_iter()
        .filter(|p| p.rom_count > 0)
        .map(|p| {
            let games = moose_rack::commands::roms(state, p.slug.clone(), None)
                .unwrap_or_default()
                .into_iter()
                .map(|r| Game {
                    id: r.id,
                    title: r.name,
                    year: r.year,
                    bytes: r.size_bytes.max(0) as u64,
                    installed: r.downloaded,
                    favorite: r.favorite,
                    cover: None,
                })
                .collect();
            // The full name, as the desktop app shows it: "Super Nintendo", not "snes".
            let name = state.map.display_name(&p.slug).map(str::to_owned).unwrap_or(p.name);
            Platform { short: short_name(&p.slug), rgb: colour_for(&p.slug), slug: p.slug, name, games }
        })
        .collect()
}

/// The letters on a card's band.
fn short_name(slug: &str) -> String {
    let known = [
        ("ps2", "PS2"), ("ps3", "PS3"), ("psx", "PS1"), ("psp", "PSP"), ("snes", "SNES"), ("sfc", "SFC"),
        ("nes", "NES"), ("famicom", "FC"), ("n64", "N64"), ("gba", "GBA"), ("gb", "GB"), ("gbc", "GBC"),
        ("nds", "NDS"), ("megadrive", "MD"), ("genesis", "GEN"), ("arcade", "ARC"), ("dc", "DC"),
        ("ngc", "GC"), ("saturn", "SAT"), ("pcengine", "PCE"), ("gamegear", "GG"), ("mastersystem", "SMS"),
        ("neo-geo-pocket", "NGP"), ("neogeoaes", "NEO"), ("wonderswan", "WS"), ("wonderswancolor", "WSC"),
        ("3do", "3DO"), ("dos", "DOS"), ("windows9x", "WIN"), ("xbox", "XBOX"), ("switch", "NSW"),
    ];
    known
        .iter()
        .find(|(s, _)| *s == slug)
        .map(|(_, short)| (*short).to_owned())
        .unwrap_or_else(|| slug.chars().take(5).collect::<String>().to_uppercase())
}

/// A colour of its own per console, the same every run.
fn colour_for(slug: &str) -> (u8, u8, u8) {
    const PALETTE: [(u8, u8, u8); 10] = [
        (52, 84, 196), (190, 60, 60), (60, 140, 90), (150, 80, 190), (200, 120, 40),
        (40, 140, 170), (170, 70, 130), (90, 110, 60), (120, 100, 200), (180, 150, 50),
    ];
    PALETTE[(hash(slug) % PALETTE.len() as u64) as usize]
}

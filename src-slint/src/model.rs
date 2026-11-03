//! What the window shows: consoles and their games. Filled from the real
//! library (`library.rs`) or, for screenshots and offline, the sample
//! (`sample.rs`); the rest of the app doesn't know which.

pub struct Platform {
    pub slug: String,
    pub name: String,
    /// The few letters on a card's band: "PS2", "SNES".
    pub short: String,
    pub rgb: (u8, u8, u8),
    pub games: Vec<Game>,
}

pub struct Game {
    /// The library's id, which the shared core's commands take.
    pub id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub bytes: u64,
    pub installed: bool,
    pub favorite: bool,
    /// The cover, once it's loaded. Until then the card draws a stand-in.
    pub cover: Option<slint::Image>,
}

/// FNV-1a: stable across runs, which `DefaultHasher` does not promise.
pub fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// A size as the app writes it: "646 MB", "4.2 GB".
pub fn human(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if bytes == 0 {
        String::new()
    } else if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{} MB", mb.round().max(1.0) as u64)
    }
}

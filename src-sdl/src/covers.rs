// Box art: found on disk, decoded once, and let go again.
//
// The letting go is the point. A cover is a few tens of kilobytes as a PNG and
// about 786 KB once decoded, and the webview held every one it had ever drawn
// — 578 MB of a 671 MB process, measured on 2026-08-20. That is merely
// wasteful on a Mac. On a 1 GB handheld it is the whole machine.
//
// So this is bounded, and the bound is a count of textures rather than a
// count of bytes: every card on screen is the same size, so a texture is the
// same size as its neighbours and counting them is counting memory.

use crate::gfx::{Gfx, Texture};
use moose_rack::media;
use sdl2::image::LoadSurface;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How many decoded covers to hold.
///
/// A screenful is a few dozen; this is enough for a screen either side of
/// where the cursor is, so scrolling back never re-decodes. At roughly 786 KB
/// apiece it is about 150 MB at the limit, which is why the handheld will want
/// this lower — and why it is one number rather than scattered.
const LIMIT: usize = 192;

/// What a cover can be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Drawn, and in `held`.
    Ready,
    /// Looked for and not there. Remembered so it is not looked for again on
    /// every frame — a filesystem miss is cheap and 2,506 of them a frame is
    /// not.
    None,
}

pub struct Covers {
    media_root: PathBuf,
    /// Where the ROMs are, for artwork the device scraped beside them.
    roms_root: PathBuf,
    /// Where the device keeps front-end themes, which carry console pictures.
    theme_dirs: Vec<PathBuf>,
    /// Which game artwork to prefer, from `[media] list_art`.
    look: String,
    /// Which console picture the grid draws, from `[icons] style`, and which
    /// installed set it comes from — `[icons] set`, empty for none.
    console_look: String,
    console_set: String,
    held: HashMap<i64, Texture>,
    known: HashMap<i64, State>,
    /// The order things were asked for, oldest first. A plain queue rather
    /// than a proper LRU: what is asked for is what is on screen, so the
    /// oldest entry is the furthest from the cursor by construction.
    order: Vec<i64>,
}

impl Covers {
    pub fn new(
        media_root: PathBuf,
        roms_root: PathBuf,
        look: String,
        icons: (String, String),
    ) -> Self {
        Covers {
            media_root,
            roms_root,
            theme_dirs: moose_rack::platform::current().theme_dirs(),
            look,
            console_look: icons.0,
            console_set: icons.1,
            held: HashMap::new(),
            known: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// How many decoded pictures are being held. The number worth watching
    /// while scrolling, since it is the one that used to grow without limit.
    #[allow(dead_code)]
    pub fn holding(&self) -> usize {
        self.held.len()
    }

    /// A console's own picture — the hardware render the grid draws.
    ///
    /// Kept in the same cache as the covers, under a key no game can have.
    /// There are three dozen consoles against two and a half thousand games,
    /// so they cost nothing to hold and are on screen constantly.
    pub fn console(&mut self, gfx: &Gfx, slug: &str) -> Option<&Texture> {
        let (look, set) = (self.console_look.clone(), self.console_set.clone());
        // A stable negative id per slug: the cache is keyed by rom id, and no
        // rom has one.
        let key = -(slug
            .bytes()
            .fold(1i64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as i64))
            .abs()
            | 1);
        if let Some(State::None) = self.known.get(&key) {
            return None;
        }
        if self.known.contains_key(&key) {
            return self.held.get(&key);
        }
        let path = moose_rack::theme::look_art(&self.media_root, slug, &set, &look)
            .or_else(|| moose_rack::theme::look_art(&self.media_root, slug, &set, "systemart"))
            .or_else(|| moose_rack::theme::look_art(&self.media_root, slug, &set, "consolegame"))
            .or_else(|| moose_rack::platformicon::installed(&self.media_root, slug))
            .or_else(|| self.in_a_theme(slug));
        let Some(path) = path else {
            self.known.insert(key, State::None);
            return None;
        };
        match self.load(gfx, &path) {
            Some(texture) => {
                self.held.insert(key, texture);
                self.known.insert(key, State::Ready);
                self.held.get(&key)
            }
            None => {
                self.known.insert(key, State::None);
                None
            }
        }
    }

    /// The cover for a game, decoding it if it is on disk and not yet held.
    ///
    /// `stem` is the ROM's filename without its extension, which is what the
    /// artwork is named after — see `media::local_art`.
    /// A picture named by path rather than found by convention.
    ///
    /// For ports, whose artwork the gamelist points straight at — there is no
    /// platform and no ROM stem to derive it from. `key` is the cache slot;
    /// callers use negative numbers so these cannot collide with a ROM id.
    pub fn by_path(&mut self, gfx: &Gfx, key: i64, path: &std::path::Path) -> Option<&Texture> {
        match self.known.get(&key) {
            Some(State::None) => return None,
            Some(State::Ready) => return self.held.get(&key),
            None => {}
        }
        match self.load(gfx, path) {
            Some(texture) => {
                self.make_room();
                self.held.insert(key, texture);
                self.known.insert(key, State::Ready);
                self.order.push(key);
                self.held.get(&key)
            }
            None => {
                self.known.insert(key, State::None);
                None
            }
        }
    }

    /// A picture this app draws itself, made once and kept.
    ///
    /// For the signal symbol, which is pixels computed from a strength rather
    /// than a file read from anywhere. Held in the same cache as the artwork
    /// because it is the same kind of thing — a texture that is expensive to
    /// make and cheap to keep — and under a key nothing else uses.
    pub fn made(
        &mut self,
        gfx: &Gfx,
        key: i64,
        make: impl FnOnce() -> (u32, u32, Vec<u8>),
    ) -> Option<&Texture> {
        if !self.known.contains_key(&key) {
            let (w, h, pixels) = make();
            self.held.insert(key, gfx.upload_rgba(w, h, &pixels));
            self.known.insert(key, State::Ready);
        }
        self.held.get(&key)
    }

    pub fn get(&mut self, gfx: &Gfx, id: i64, platform: &str, stem: &str) -> Option<&Texture> {
        match self.known.get(&id) {
            Some(State::None) => return None,
            Some(State::Ready) => {
                // Asked for again: it stays.
                if let Some(at) = self.order.iter().position(|&held| held == id) {
                    let moved = self.order.remove(at);
                    self.order.push(moved);
                }
                return self.held.get(&id);
            }
            None => {}
        }

        let path = media::local_art(&self.media_root, platform, stem, &self.look)
            .or_else(|| self.beside_the_rom(platform, stem));
        let Some(path) = path else {
            self.known.insert(id, State::None);
            return None;
        };
        match self.load(gfx, &path) {
            Some(texture) => {
                self.make_room();
                self.held.insert(id, texture);
                self.known.insert(id, State::Ready);
                self.order.push(id);
                self.held.get(&id)
            }
            None => {
                // On disk but not decodable — a truncated download, or
                // something that is not an image. Remembered as absent so it
                // is not retried every frame.
                self.known.insert(id, State::None);
                None
            }
        }
    }

    /// A console picture out of a front-end theme already on the device.
    ///
    /// Asked for five times, and each time the answer was "there is no icon set
    /// here" — which is true and beside the point. A KNULLI install has themes
    /// in `/userdata/themes`, and every one of them draws these systems: the
    /// pictures are at `<theme>/_inc/systems/<look>/<slug>.webp`, which is the
    /// same shape as the sets this app downloads. Nothing needed fetching; we
    /// were looking in one folder and they were in another.
    ///
    /// Art before icons before logos: a rendered machine says which console it
    /// is at a glance, and a wordmark has to be read. SVG is skipped — it is a
    /// logo format here, and the raster ones are what the grid wants.
    fn in_a_theme(&self, slug: &str) -> Option<PathBuf> {
        for root in &self.theme_dirs {
            let Ok(themes) = std::fs::read_dir(root) else {
                continue;
            };
            let mut themes: Vec<PathBuf> = themes.flatten().map(|t| t.path()).collect();
            themes.sort();
            for theme in themes {
                let systems = theme.join("_inc").join("systems");
                if !systems.is_dir() {
                    continue;
                }
                let mut looks: Vec<PathBuf> = std::fs::read_dir(&systems)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|d| d.path())
                    .collect();
                // Directory names vary by theme — `carousel-icons-art`,
                // `artwork (modern)`, `systemart`. Sorting puts "art" before
                // "icons" before "logos" often enough, and the explicit
                // preference below does the rest.
                looks.sort_by_key(|p| {
                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                    let rank = if name.contains("art") {
                        0
                    } else if name.contains("icon") {
                        1
                    } else {
                        2
                    };
                    (rank, name)
                });
                for look in looks {
                    for ext in ["webp", "png", "jpg"] {
                        let at = look.join(format!("{slug}.{ext}"));
                        if at.is_file() {
                            return Some(at);
                        }
                    }
                }
            }
        }
        None
    }

    /// Artwork the device scraped for itself, next to the ROM.
    ///
    /// A library built by scanning the card has no media folder of ours to draw
    /// from, and every cover came back empty — which on a handheld with 98
    /// scraped pictures per system is not "no artwork", it is looking in the
    /// wrong place. Batocera keeps them at `<system>/images/<stem>-image.png`
    /// beside the games, which is where its own front end reads them from.
    ///
    /// The suffixes are tried in the order a scraper writes them: the box, then
    /// a screen, then whatever else it managed.
    fn beside_the_rom(&self, platform: &str, stem: &str) -> Option<PathBuf> {
        let dir = self.roms_root.join(platform).join("images");
        if !dir.is_dir() {
            return None;
        }
        for suffix in ["-image", "-box2dfront", "-thumb", "-boxart", "-screenshot", ""] {
            for ext in ["png", "jpg", "jpeg", "webp"] {
                let at = dir.join(format!("{stem}{suffix}.{ext}"));
                if at.is_file() {
                    return Some(at);
                }
            }
        }
        None
    }

    /// Read the file and hand its pixels to the GPU.
    ///
    /// SDL_image decodes — it is already a package on the handheld and knows
    /// every format a scraper produces — but the texture is ours, on our
    /// context. `into_rgba` because a decoder returns whatever the file was,
    /// and the uploader wants one layout.
    fn load(&self, gfx: &Gfx, path: &Path) -> Option<Texture> {
        let surface = sdl2::surface::Surface::from_file(path).ok()?;
        let rgba = surface
            .convert_format(sdl2::pixels::PixelFormatEnum::ABGR8888)
            .ok()?;
        let (w, h) = (rgba.width(), rgba.height());
        let pitch = rgba.pitch() as usize;
        let bytes = rgba.without_lock()?;
        // Rows can be padded. Copied tightly, because the uploader is told
        // rows are not.
        let mut tight = Vec::with_capacity(w as usize * h as usize * 4);
        for row in 0..h as usize {
            let from = row * pitch;
            tight.extend_from_slice(&bytes[from..from + w as usize * 4]);
        }
        Some(gfx.upload_rgba(w, h, &tight))
    }

    /// Drop the oldest until there is room for one more.
    fn make_room(&mut self) {
        while self.held.len() >= LIMIT {
            let Some(oldest) = self.order.first().copied() else {
                break;
            };
            self.order.remove(0);
            self.held.remove(&oldest);
            // Forgotten rather than marked absent: it was there a moment ago
            // and scrolling back has to find it again.
            self.known.remove(&oldest);
        }
    }
}

/// The bound is what makes this safe on a 1 GB machine, so it is checked
/// rather than trusted — at compile time, since it is a constant and a test
/// that can never fail at runtime is not a test.
///
/// Too small and scrolling back re-decodes; too large and the handheld is
/// gone. At roughly 786 KB a cover, 192 is about 150 MB.
const _: () = assert!(LIMIT >= 96 && LIMIT <= 256);

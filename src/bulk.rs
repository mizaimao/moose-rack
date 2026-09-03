//! Planning a bulk download: what it would fetch, and how big that is.
//!
//! Separated from the fetching because the estimate is the useful part. "25.6
//! GB" against "31 GB" is the difference between a download that finishes
//! before a flight and one that does not, and it is not guessable from a game
//! count — a Mega Drive cartridge and a PlayStation disc differ by three orders
//! of magnitude.
//!
//! Artwork is not one thing and the sizes are not close. A game's static art is
//! tens of kilobytes per type; its video is tens of megabytes. Fetching
//! everything for a 2,400-game collection is two orders of magnitude more
//! transfer than fetching what is actually shown, and nearly all of it would
//! never be looked at. So the choices are separate, and videos have their own,
//! because they are the one that changes the total by a factor of ten.

use crate::cache::RomRow;

/// How much artwork to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    /// Nothing. For a ROM-only run.
    None,
    /// The two images the interface actually draws: the one the game list
    /// shows, and the miximage in the info pane. Everything visible, nothing
    /// else.
    Minimal,
    /// Every static type ES-DE has — box front and back, 3D box, cartridge,
    /// screenshot, title screen, marquee, fanart. For an info pane that works
    /// with no server.
    Full,
}

/// What a run should fetch.
#[derive(Debug, Clone, Copy)]
pub struct Want {
    pub roms: bool,
    pub art: Art,
    pub videos: bool,
    pub manuals: bool,
}

impl Default for Want {
    /// The defaults are the cheap ones. Anything that multiplies the download
    /// has to be asked for, because the person ticking these boxes cannot see
    /// the difference between a type that costs 40 KB a game and one that costs
    /// 40 MB until the estimate updates.
    fn default() -> Self {
        Self { roms: true, art: Art::Minimal, videos: false, manuals: false }
    }
}

/// Bytes a single game's media costs, by kind.
///
/// Measured off this library rather than guessed: a miximage averages 350 KB, a
/// cartridge render 200 KB, a video 8 MB. They are averages, and the estimate
/// says so — the point is the order of magnitude, which is what the choice
/// turns on.
const ART_EACH: u64 = 350_000;
const FULL_ART_EACH: u64 = 1_800_000;
const VIDEO_EACH: u64 = 8_000_000;
const MANUAL_EACH: u64 = 400_000;

/// What a download would cost, before it starts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    pub games: usize,
    /// Games whose ROM is already on this machine and will be skipped.
    pub roms_present: usize,
    pub rom_bytes: u64,
    pub media_bytes: u64,
}

impl Estimate {
    pub fn total(&self) -> u64 {
        self.rom_bytes + self.media_bytes
    }

    /// Deliberately not a precise figure. Media sizes are averages and the
    /// server may hold fewer types than a game could have, so presenting this
    /// to three significant figures would be false precision about a number
    /// that exists to answer "does this fit".
    pub fn describe(&self) -> String {
        format!(
            "{} game(s), about {:.1} GB ({:.1} GB of games, {:.1} GB of media){}",
            self.games,
            self.total() as f64 / 1e9,
            self.rom_bytes as f64 / 1e9,
            self.media_bytes as f64 / 1e9,
            if self.roms_present > 0 {
                format!("; {} already downloaded", self.roms_present)
            } else {
                String::new()
            },
        )
    }
}

/// Estimate a run over `rows`. `present` says whether a game's ROM is here.
pub fn estimate(rows: &[RomRow], want: Want, present: impl Fn(&RomRow) -> bool) -> Estimate {
    let mut e = Estimate { games: rows.len(), ..Default::default() };
    for row in rows {
        if want.roms {
            if present(row) {
                e.roms_present += 1;
            } else {
                e.rom_bytes += row.fs_size_bytes.max(0) as u64;
            }
        }
        e.media_bytes += match want.art {
            Art::None => 0,
            // Two images: what the list draws and what the pane shows.
            Art::Minimal => ART_EACH * 2,
            Art::Full => FULL_ART_EACH,
        };
        if want.videos {
            e.media_bytes += VIDEO_EACH;
        }
        if want.manuals {
            e.media_bytes += MANUAL_EACH;
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize, each: i64) -> Vec<RomRow> {
        (0..n)
            .map(|i| RomRow {
                id: i as i64,
                platform_slug: "snes".into(),
                name: format!("Game {i}"),
                fs_name: format!("game{i}.sfc"),
                fs_size_bytes: each,
                ..Default::default()
            })
            .collect()
    }

    /// The reason videos are a separate box: they change the answer by an order
    /// of magnitude, and someone ticking boxes cannot see that until the number
    /// moves.
    #[test]
    fn videos_dominate_the_total_when_asked_for() {
        let r = rows(100, 1_000_000);
        let without = estimate(&r, Want::default(), |_| false);
        let with = estimate(&r, Want { videos: true, ..Want::default() }, |_| false);
        assert!(
            with.total() > without.total() * 5,
            "videos should dwarf the rest: {} vs {}",
            with.total(),
            without.total()
        );
    }

    #[test]
    fn the_default_takes_games_and_the_two_images_that_are_shown() {
        let w = Want::default();
        assert!(w.roms);
        assert_eq!(w.art, Art::Minimal);
        assert!(!w.videos, "videos must never arrive unasked");
        assert!(!w.manuals);
    }

    /// A game already on disk is not downloaded again, and the estimate has to
    /// say so — otherwise a mostly-complete library reports the full size and
    /// looks impossible.
    #[test]
    fn games_already_here_are_not_counted_towards_the_download() {
        let r = rows(10, 1_000_000);
        let all = estimate(&r, Want::default(), |_| false);
        let none = estimate(&r, Want::default(), |_| true);
        assert_eq!(all.rom_bytes, 10_000_000);
        assert_eq!(none.rom_bytes, 0);
        assert_eq!(none.roms_present, 10);
        // Media is still fetched for a game whose ROM is present: having the
        // game does not mean having its artwork.
        assert_eq!(none.media_bytes, all.media_bytes);
    }

    #[test]
    fn asking_for_no_art_costs_nothing_beyond_the_games() {
        let r = rows(50, 2_000_000);
        let e = estimate(&r, Want { art: Art::None, ..Want::default() }, |_| false);
        assert_eq!(e.media_bytes, 0);
        assert_eq!(e.total(), 100_000_000);
    }

    #[test]
    fn the_full_set_costs_more_than_the_two_shown_images() {
        let r = rows(10, 0);
        let min = estimate(&r, Want::default(), |_| false);
        let full = estimate(&r, Want { art: Art::Full, ..Want::default() }, |_| false);
        assert!(full.media_bytes > min.media_bytes);
    }

    #[test]
    fn the_summary_says_what_is_already_here() {
        let r = rows(4, 1_000_000);
        assert!(estimate(&r, Want::default(), |_| true).describe().contains("4 already downloaded"));
        assert!(!estimate(&r, Want::default(), |_| false).describe().contains("already"));
    }
}

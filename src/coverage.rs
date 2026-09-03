//! Which games have artwork, and which of the rest could ever get any.
//!
//! "Missing art" is three different situations wearing one label, and treating
//! them alike makes the number useless:
//!
//! * **Not fetched yet.** The art exists somewhere and nobody has asked for it.
//!   Worth a scrape run.
//! * **Not in the database.** ScreenScraper has no entry for this file, and no
//!   amount of scraping will produce one. Reporting these as missing is
//!   reporting a permanent fact as an outstanding task.
//! * **Never going to be.** Romhacks, translations, unlicensed carts and
//!   prototypes. ScreenScraper catalogs released games; a patched ROM is not
//!   one, and counting them against coverage means the number can never reach
//!   100% and so stops meaning anything.
//!
//! The third is decided from the filename, because that is where the scene
//! conventions put it. No-Intro and TOSEC both tag in brackets — `(Unl)`,
//! `(Hack)`, `(Proto)`, `(Aftermarket)` — and those tags are the only
//! machine-readable statement of what a dump is. It is not a perfect signal: an
//! untagged hack reads as official and a legitimate game with `(Demo)` in its
//! real title would read as unofficial. It is the signal that exists.

use std::collections::BTreeMap;

/// What kind of dump a file is, as far as its name admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A released game. Should have artwork; if it has none, that is a gap.
    Official,
    /// A hack, translation, unlicensed cart, prototype or demo. May have
    /// artwork, will often never have any, and should not count against a
    /// coverage figure either way.
    Unofficial,
}

/// Bracketed tags that mean "not a released game".
///
/// Matched inside brackets only. `(Unl)` is a tag; a game called `Unlimited
/// Saga` is not, and matching bare substrings would catch it.
const UNOFFICIAL_TAGS: &[&str] = &[
    "unl", "hack", "aftermarket", "homebrew", "pirate", "proto", "prototype",
    "beta", "sample", "demo", "test program", "unreleased", "translation",
    "bootleg", "program",
];

/// Files known to be unrunnable whatever we do, from `data/unavailable-roms.json`.
///
/// Compiled in rather than read at run time: it is a fact about a romset, not a
/// setting, and a missing file should not silently turn thirteen permanent
/// failures back into thirteen outstanding tasks.
const UNAVAILABLE: &str = include_str!("../data/unavailable-roms.json");

/// Whether this file is one nothing can run.
///
/// These are counted with the hacks and prototypes rather than as gaps. The zip
/// holds a different regional revision than the driver of that name expects, so
/// no amount of scraping or configuration reaches them — only a different dump
/// does, and that is not a task this app can close.
pub fn unavailable(platform: &str, fs_name: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(UNAVAILABLE)
        .ok()
        .and_then(|v| v.get(platform)?.get(fs_name).map(|_| true))
        .unwrap_or(false)
}

/// Classify a file by the tags in its name.
pub fn classify_in(platform: &str, fs_name: &str) -> Kind {
    if unavailable(platform, fs_name) {
        return Kind::Unofficial;
    }
    classify(fs_name)
}

pub fn classify(fs_name: &str) -> Kind {
    let mut inside = String::new();
    let mut depth = 0i32;
    for c in fs_name.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
                inside.push('\u{1}'); // separator, so tags cannot run together
            }
            ')' | ']' => depth = (depth - 1).max(0),
            c if depth > 0 => inside.extend(c.to_lowercase()),
            _ => {}
        }
    }
    // Tags are comma separated inside one bracket: `(USA, Unl)`.
    for tag in inside.split(['\u{1}', ',']) {
        let tag = tag.trim();
        if UNOFFICIAL_TAGS.contains(&tag) {
            return Kind::Unofficial;
        }
        // `(Hack by Someone)`, `(Beta 2)`, `(Proto 1)` — the first word carries
        // it, and a trailing qualifier does not change what the dump is.
        if let Some(first) = tag.split(' ').next()
            && UNOFFICIAL_TAGS.contains(&first)
            && tag.split(' ').count() <= 4
        {
            return Kind::Unofficial;
        }
    }
    Kind::Official
}

/// Coverage for one platform.
#[derive(Debug, Default, Clone, Copy)]
pub struct Row {
    pub official: usize,
    pub official_with_art: usize,
    pub unofficial: usize,
    pub unofficial_with_art: usize,
    /// Of the official games with no art, how many the server could not find
    /// in ScreenScraper. Only set by a probing run.
    pub not_in_database: usize,
    /// How many were probed at all, so a partial probe cannot be read as a
    /// complete one.
    pub probed: usize,
}

impl Row {
    /// Percentage of released games that have artwork. The figure that means
    /// something: hacks and prototypes are excluded, so 100% is reachable.
    pub fn percent(&self) -> f64 {
        if self.official == 0 {
            return 100.0;
        }
        self.official_with_art as f64 * 100.0 / self.official as f64
    }

    /// Released games with no art that were not ruled out by a probe — the
    /// ones a scrape run could still do something about.
    pub fn worth_scraping(&self) -> usize {
        (self.official - self.official_with_art).saturating_sub(self.not_in_database)
    }
}

/// Totals across every platform.
pub fn totals(rows: &BTreeMap<String, Row>) -> Row {
    let mut t = Row::default();
    for r in rows.values() {
        t.official += r.official;
        t.official_with_art += r.official_with_art;
        t.unofficial += r.unofficial;
        t.unofficial_with_art += r.unofficial_with_art;
        t.not_in_database += r.not_in_database;
        t.probed += r.probed;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_games_are_official_however_they_are_tagged() {
        for name in [
            "Sonic the Hedgehog 2 (World).zip",
            "Chrono Trigger (USA).sfc",
            "Alex Kidd in the Enchanted Castle (USA, Europe) (Rev A).zip",
            "Final Fantasy VI (Japan) (Rev 1) [!].sfc",
        ] {
            assert_eq!(classify(name), Kind::Official, "{name}");
        }
    }

    #[test]
    fn hacks_and_unlicensed_carts_are_not_counted_against_coverage() {
        for name in [
            "Battle of Red Cliffs, The (China) (Unl).zip",
            "Beggar Prince (World) (Unl).zip",
            "Beast Ball (USA) (Proto).zip",
            "Some Game (USA) (Beta).zip",
            "Super Mario Bros (Hack by Someone).nes",
            "Rockman 4 (Japan) (Translation).nes",
            "Thing (Aftermarket).zip",
        ] {
            assert_eq!(classify(name), Kind::Unofficial, "{name}");
        }
    }

    /// The trap in matching these: the tags are ordinary words. A title that
    /// merely contains one is a released game, and getting this wrong quietly
    /// removes real games from the count.
    #[test]
    fn a_title_that_merely_contains_a_tag_word_is_still_official() {
        for name in [
            "Unlimited Saga (USA).zip",
            "Demolition Man (USA).zip",
            "Prototype Hunter (USA).zip",
            "Beta Wolf (Japan).zip",
            "Hackers (Europe).zip",
        ] {
            assert_eq!(classify(name), Kind::Official, "{name}");
        }
    }

    /// A percentage that can never reach 100 is a percentage nobody reads.
    #[test]
    fn the_figure_ignores_dumps_that_could_never_have_art() {
        let r = Row { official: 10, official_with_art: 10, unofficial: 90, ..Default::default() };
        assert_eq!(r.percent(), 100.0, "hacks must not hold the number down");
    }

    #[test]
    fn a_probe_only_discounts_what_it_actually_ruled_out() {
        let r = Row {
            official: 100,
            official_with_art: 60,
            not_in_database: 25,
            probed: 40,
            ..Default::default()
        };
        assert_eq!(r.worth_scraping(), 15, "40 missing, 25 of them impossible");

        // Nothing probed means nothing ruled out — every gap still counts.
        let r = Row { official: 100, official_with_art: 60, ..Default::default() };
        assert_eq!(r.worth_scraping(), 40);
    }

    /// The thirteen arcade romsets nothing can run. They are a permanent fact
    /// about a dump, not an outstanding task, and counting them as gaps means
    /// the number never closes and stops being read.
    #[test]
    fn romsets_nothing_can_run_are_not_counted_as_gaps() {
        assert!(unavailable("arcade", "avengers.zip"));
        assert!(unavailable("arcade", "vball.zip"));
        assert_eq!(classify_in("arcade", "avengers.zip"), Kind::Unofficial);
        // Only for the platform they were recorded under, and only for the
        // exact file: a same-named dump on another system is a different game.
        assert!(!unavailable("megadrive", "avengers.zip"));
        assert!(!unavailable("arcade", "mslug.zip"));
        assert_eq!(classify_in("arcade", "mslug.zip"), Kind::Official);
    }
}

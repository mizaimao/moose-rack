//! Filling in artwork for games ES-DE never scraped.
//!
//! ## Why this does not talk to ScreenScraper itself
//!
//! ScreenScraper rejects every request that does not carry a *developer*
//! credential, and those are issued to registered applications, not to people.
//! An account of your own is not enough. So a client like this one has three
//! ways to get art, and only one of them is honest:
//!
//! * Take another application's credential — ES-DE's, say, or RomM's — out of
//!   its binary or its URLs and make our own calls under its name. That is
//!   impersonating someone else's software to a service that trusted it, and
//!   when it is noticed the key is revoked for *their* users, who did nothing.
//!   Not an option, however convenient.
//! * Register this app with ScreenScraper and wait for its own credential.
//!   Correct, and slow.
//! * Ask the server, which already has one.
//!
//! The third is what happens here, and it needs nothing new. A RomM server with
//! ScreenScraper configured answers `/api/search/roms` with ready-made media
//! URLs — the same call and the same URLs its own web interface uses when it
//! offers you match candidates. Fetching one is being an ordinary client of
//! your own server. Nothing is borrowed and nothing is pretended.
//!
//! The full set comes from the same place. A game the server has matched
//! against ScreenScraper carries `ss_metadata`: every media URL it resolved —
//! 2D box, back, 3D box, cartridge, miximage, screenshot, title screen,
//! marquee, fanart, manual, video. Those are the server's own URLs, stored on
//! the server, served back on `/api/roms/{id}`, so reading them is reading your
//! own library. Nothing is assembled here, which is the line that matters:
//! rewriting the `media=` parameter to invent requests the server never made
//! would be the first option wearing a different hat.
//!
//! A game the server matched some other way — IGDB, LaunchBox, or nothing — has
//! no `ss_metadata`, and then all that is available is the single 2D box URL
//! the search endpoint builds on demand. So the result is uneven by design, and
//! honest about which games got what.
//!
//! ## Where the art goes
//!
//! Into the same local cache the ES-DE tree is copied to, each type filed under
//! the ES-DE directory that means the same thing — so a scraped game is
//! indistinguishable from one ES-DE scraped, and the art chain, the card
//! sizing and the info pane all work on it without knowing where it came from.

use anyhow::Result;
use std::path::Path;

use crate::api::Client;
use crate::cache::{Cache, RomRow};
use crate::media;

/// What a run did.
#[derive(Debug, Default, Clone, Copy)]
pub struct Report {
    /// Games that already had artwork and were not touched.
    pub had_art: usize,
    pub fetched: usize,
    /// Identified by the server, but ScreenScraper had no picture.
    pub no_art: usize,
    /// The server could not identify the file at all.
    pub unmatched: usize,
    pub failed: usize,
    /// Games that got only a box because the server would not accept the
    /// match. Counted rather than reported per game: it is one cause with one
    /// fix, and saying it 2,700 times is not 2,700 pieces of information.
    pub box_only: usize,
    /// Set the first time the server refused a write.
    pub needs_write_scope: bool,
    /// Covers pushed back to the server, replacing whatever it had.
    pub covers_pushed: usize,
}

impl Report {
    pub fn describe(&self) -> String {
        let mut out = format!(
            "{} fetched, {} already had art, {} not on ScreenScraper, {} unidentified{}",
            self.fetched,
            self.had_art,
            self.no_art,
            self.unmatched,
            if self.failed > 0 { format!(", {} failed", self.failed) } else { String::new() },
        );
        if self.covers_pushed > 0 {
            out.push_str(&format!("; {} cover(s) pushed to the server", self.covers_pushed));
        }
        if self.needs_write_scope {
            out.push_str(&format!(
                "\n{} game(s) got the box only. The rest of the set — cartridge, \
                 miximage, marquee — needs the server to record the match, and \
                 this token may not write. Add `roms.write` to it in RomM under \
                 Settings, Client tokens, then run this again.",
                self.box_only
            ));
        }
        out
    }
}

/// Games with no artwork of any kind, in the order they would be shown.
///
/// Checked against the local cache only, which is deliberate: a HEAD per game
/// per media type against the server would be seven requests each before any
/// work started, and the caller is about to ask the server about every one of
/// them anyway.
pub fn missing(cache: &Cache, media_root: &Path, platform: Option<&str>) -> Result<Vec<RomRow>> {
    let rows = match platform {
        Some(p) => cache.roms_for(p)?,
        None => cache.all_roms()?,
    };
    Ok(rows
        .into_iter()
        .filter(|r| {
            let stem = stem_of(&r.fs_name);
            !media::ART_CHAIN
                .iter()
                .any(|k| media::find_local(media_root, &r.platform_slug, &stem, k).is_some())
        })
        .collect())
}

/// Which ES-DE directory each of the server's ScreenScraper URLs belongs in.
///
/// Named pairs rather than a guess at the far end: `physical_url` is a
/// cartridge and ES-DE calls that `physicalmedia`, and getting one of these
/// wrong files a picture where nothing will ever look for it.
fn urls_for(m: &crate::api::SsMedia, videos: bool) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut add = |kind: &'static str, url: &Option<String>| {
        if let Some(u) = url.as_ref().filter(|u| !u.is_empty()) {
            out.push((kind, u.clone()));
        }
    };
    add(media::PHYSICALMEDIA, &m.physical_url);
    add(media::MIXIMAGES, &m.miximage_url);
    add(media::COVERS, &m.box2d_url);
    add("backcovers", &m.box2d_back_url);
    add("3dboxes", &m.box3d_url);
    add(media::SCREENSHOTS, &m.screenshot_url);
    add(media::TITLESCREENS, &m.title_screen_url);
    add(media::MARQUEES, &m.marquee_url);
    add("fanart", &m.fanart_url);
    add("manuals", &m.manual_url);
    // Videos are tens of megabytes against tens of kilobytes for everything
    // else here, so they are the one type that has to be asked for.
    if videos {
        add(media::VIDEOS, &m.video_url);
    }
    out
}

/// Fetch artwork for one game. Returns whether anything landed.
///
/// Every step is allowed to come back empty. A romset with no ScreenScraper
/// entry, a game the server cannot identify, a match with no picture attached —
/// all ordinary, and none of them a reason to stop a run of two thousand.
pub async fn fill_one(
    client: &Client,
    media_root: &Path,
    row: &RomRow,
    videos: bool,
    report: &mut Report,
) -> Result<bool> {
    let stem = stem_of(&row.fs_name);

    // The ES-DE tree wins, and it is checked against the *server*, not just
    // what happens to be cached here. `missing` only looks locally, because
    // seven HEADs per game before any work started would cost more than the
    // run; that makes it a prefilter, and this is the real test. Without it a
    // console fully scraped in ES-DE would get ScreenScraper boxes written over
    // the top of art nobody had happened to browse yet.
    if media::ensure_art(Some(client), media_root, &row.platform_slug, &stem, media::MIXIMAGES)
        .await
        .is_some()
    {
        report.had_art += 1;
        return Ok(false);
    }

    // Everything the server already resolved, which is the whole set.
    let media_urls = match client.rom_media(row.id).await {
        Ok(m) => urls_for(&m, videos),
        Err(_) => Vec::new(),
    };

    // Nothing stored means the server identified this game some other way —
    // IGDB, LaunchBox — or not at all. Ask it what ScreenScraper thinks the
    // file is, and then tell it, so it resolves and keeps the whole set. That
    // is the same thing RomM's own match button does, and the only route to
    // more than a single box.
    let media_urls = if !media_urls.is_empty() {
        media_urls
    } else {
        let matches = match client.identify(row.id, &row.name).await {
            Ok(m) => m,
            Err(_) => {
                report.failed += 1;
                return Ok(false);
            }
        };
        let Some(hit) = matches
            .iter()
            .filter(|m| {
                m.name.as_deref().is_none_or(|n| plausible_match(&row.name, n))
                    || m.name.as_deref().is_none_or(|n| plausible_match(&stem, n))
            })
            .find(|m| m.ss_id.is_some() || !m.ss_url_cover.is_empty())
        else {
            if matches.is_empty() {
                report.unmatched += 1;
            } else {
                report.no_art += 1;
            }
            return Ok(false);
        };

        let mut full = Vec::new();
        if let Some(ss_id) = hit.ss_id {
            match client.set_screenscraper_match(row.id, ss_id).await {
                // Recorded. Read back what it resolved, which is the whole set.
                Ok(true) => {
                    if let Ok(m) = client.rom_media(row.id).await {
                        full = urls_for(&m, videos);
                    }
                }
                // Not allowed to write. Carry on with the one URL that needs no
                // permission, and let the report say why once.
                Ok(false) => {
                    report.needs_write_scope = true;
                }
                Err(_) => {}
            }
        }
        if full.is_empty() {
            if hit.ss_url_cover.is_empty() {
                report.no_art += 1;
                return Ok(false);
            }
            report.box_only += 1;
            vec![(media::COVERS, hit.ss_url_cover.clone())]
        } else {
            full
        }
    };

    let mut landed = false;
    // Kept so the server's own copy can be replaced with the same picture.
    let mut cover: Option<Vec<u8>> = None;
    for (kind, url) in media_urls {
        // Straight to ScreenScraper, at the address the server gave us, and
        // with no Authorization header: that is our server's credential, not
        // theirs.
        let bytes = match client.http().get(&url).send().await {
            Ok(r) if r.status().is_success() => r.bytes().await.unwrap_or_default(),
            _ => continue,
        };
        // A throttled or unknown request comes back as 200 with a line of
        // text, so size is the honest test of whether this is a picture.
        if bytes.len() < 1024 {
            continue;
        }
        let dir = media_root.join(&row.platform_slug).join(kind);
        std::fs::create_dir_all(&dir)?;
        let file_name = format!("{stem}.{}", extension_for(kind, &bytes));
        std::fs::write(dir.join(&file_name), &bytes)?;
        media::record_scraped(media_root, &row.platform_slug, kind, &file_name);
        if kind == media::COVERS {
            cover = Some(bytes.to_vec());
        }
        landed = true;
    }

    if !landed {
        report.no_art += 1;
        return Ok(false);
    }
    report.fetched += 1;

    // Put the same picture on the server, so the RomM web interface and any
    // other client show what this app shows rather than whichever earlier
    // scrape happened to win. Never fatal: the art is already on disk here, and
    // failing to share it is not a reason to call the game a failure.
    if let Some(bytes) = cover {
        match client.upload_cover(row.id, &format!("{stem}.png"), bytes).await {
            Ok(true) => report.covers_pushed += 1,
            Ok(false) => report.needs_write_scope = true,
            Err(_) => {}
        }
    }
    Ok(true)
}

/// The extension to save under, read from the bytes rather than the URL.
///
/// ScreenScraper's media endpoint carries the type in a query parameter, not in
/// a path, so the URL says nothing about what came back — and `find_local`
/// matches on the stem and needs the real extension to be there.
fn extension_for(kind: &str, bytes: &[u8]) -> &'static str {
    if kind == media::VIDEOS {
        return "mp4";
    }
    if bytes.starts_with(b"%PDF") {
        return "pdf";
    }
    if bytes.starts_with(b"\x89PNG") {
        return "png";
    }
    "jpg"
}

/// Whether a candidate the server offered is plausibly the same game.
///
/// The server returns candidates, not answers, and the top one is sometimes a
/// different game. It is always the same shape of mistake: a sequel matched to
/// the original, because the original's name is a prefix of the sequel's and
/// whatever scores the match does not care about the numeral on the end. On one
/// console this put Road Rash's box on Road Rash II, PGA Tour Golf's on III,
/// King of the Monsters' on 2 — and then pushed each one to the server as that
/// game's cover, which is worse than leaving it blank.
///
/// So the numeral has to agree. Both bare, or both the same: `Road Rash` and
/// `Road Rash II` do not match, and neither do `HardBall '94` and `HardBall
/// III`. Names that share no opening word are rejected outright, which is what
/// catches the pairs that are not sequels at all.
fn plausible_match(rom_name: &str, candidate: &str) -> bool {
    let (a, b) = (normalize(rom_name), normalize(candidate));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if edition(&a) != edition(&b) {
        return false;
    }
    // The first word carries most of a title's identity, and a candidate that
    // does not share it is not a near miss.
    a.split(' ').next() == b.split(' ').next()
}

/// Lowercase, without the bracketed region and revision tags, and without
/// punctuation — so `Boogerman: A Pick and Flick Adventure` and
/// `Boogerman - A Pick and Flick Adventure` are the same string.
fn normalize(name: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() || c == '\'' => out.extend(c.to_lowercase()),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The thing that tells one entry in a series from another: a trailing number,
/// a roman numeral, or a year. `None` for a title that has none.
fn edition(normalized: &str) -> Option<String> {
    let last = normalized.split(' ').next_back()?;
    let is_roman = !last.is_empty() && last.chars().all(|c| "ivx".contains(c));
    let is_number = last.chars().all(|c| c.is_ascii_digit()) && !last.is_empty();
    let is_year = last.starts_with('\'') && last.len() == 3;
    (is_roman || is_number || is_year).then(|| last.to_owned())
}

fn stem_of(fs_name: &str) -> String {
    Path::new(fs_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| fs_name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_that_found_nothing_still_says_what_it_looked_at() {
        let r = Report { had_art: 12, ..Default::default() };
        assert!(r.describe().contains("0 fetched"));
        assert!(r.describe().contains("12 already had art"));
        // Failures are only mentioned when there were some: a clean run should
        // not read as though something went wrong.
        assert!(!r.describe().contains("failed"));
        assert!(Report { failed: 3, ..Default::default() }.describe().contains("3 failed"));
    }

    #[test]
    fn a_name_with_no_extension_still_yields_a_stem() {
        assert_eq!(stem_of("Sonic (USA).md"), "Sonic (USA)");
        assert_eq!(stem_of("pkscram"), "pkscram");
        assert_eq!(stem_of("Game.v1.2.zip"), "Game.v1.2");
    }

    use crate::api::SsMedia;

    fn full() -> SsMedia {
        let u = |s: &str| Some(s.to_owned());
        SsMedia {
            box2d_url: u("b2d"),
            box2d_back_url: u("back"),
            box3d_url: u("b3d"),
            physical_url: u("cart"),
            miximage_url: u("mix"),
            screenshot_url: u("shot"),
            title_screen_url: u("title"),
            marquee_url: u("marquee"),
            fanart_url: u("fan"),
            manual_url: u("manual"),
            video_url: u("video"),
        }
    }

    /// Every directory this writes into has to be one ES-DE actually uses, or
    /// the picture lands somewhere nothing will ever look for it — and it looks
    /// exactly like a game that failed to scrape.
    #[test]
    fn every_media_type_lands_in_a_real_esde_directory() {
        for (kind, _) in urls_for(&full(), true) {
            assert!(
                media::ESDE_TYPES.iter().any(|(k, _)| *k == kind),
                "{kind} is not an ES-DE media directory"
            );
        }
    }

    /// The full set is the point of reading the server's stored metadata: a
    /// single box would have been available without it.
    #[test]
    fn the_whole_set_is_taken_including_the_cartridge() {
        let got: Vec<&str> = urls_for(&full(), false).into_iter().map(|(k, _)| k).collect();
        for want in [
            media::PHYSICALMEDIA,
            media::MIXIMAGES,
            media::COVERS,
            "3dboxes",
            media::MARQUEES,
            media::TITLESCREENS,
        ] {
            assert!(got.contains(&want), "{want} missing from {got:?}");
        }
    }

    /// A video is tens of megabytes against tens of kilobytes for every
    /// picture, so it is the one type that must never arrive unasked. Getting
    /// this backwards turns a 2,700-game run into a hundred-gigabyte one.
    #[test]
    fn a_video_is_never_fetched_unless_it_was_asked_for() {
        let without: Vec<&str> = urls_for(&full(), false).into_iter().map(|(k, _)| k).collect();
        assert!(!without.contains(&media::VIDEOS), "{without:?}");

        let with: Vec<&str> = urls_for(&full(), true).into_iter().map(|(k, _)| k).collect();
        assert!(with.contains(&media::VIDEOS));
    }

    /// A game with only some media must yield only those, not empty strings
    /// standing in for the rest.
    #[test]
    fn absent_and_empty_urls_are_both_skipped() {
        let partial = SsMedia {
            box2d_url: Some("b2d".to_owned()),
            physical_url: Some(String::new()),
            ..Default::default()
        };
        let got: Vec<&str> = urls_for(&partial, true).into_iter().map(|(k, _)| k).collect();
        assert_eq!(got, vec![media::COVERS], "an empty string is not a URL");
    }

    /// ScreenScraper's media endpoint names the type in a query parameter, not
    /// in a path, so the URL says nothing about what came back. The bytes do,
    /// and `find_local` needs the real extension to find the file again.
    #[test]
    fn the_extension_comes_from_the_bytes_not_the_url() {
        assert_eq!(extension_for(media::COVERS, b"\x89PNG\r\n"), "png");
        assert_eq!(extension_for(media::COVERS, b"\xff\xd8\xff\xe0"), "jpg");
        assert_eq!(extension_for("manuals", b"%PDF-1.4"), "pdf");
        // A video is not sniffed: mp4 has no magic at offset zero worth
        // trusting, and the endpoint only ever returns one.
        assert_eq!(extension_for(media::VIDEOS, b"\x00\x00\x00 ftyp"), "mp4");
    }

    /// The prefilter is local-only by design, so anything cached under any link
    /// of the art chain takes a game out of the list. That is what let a stale
    /// RomM screenshot hide a game from the very pass meant to fix it.
    #[test]
    fn anything_already_cached_takes_a_game_out_of_the_list() {
        let root = std::env::temp_dir().join("moose-rack-scrape-missing");
        let _ = std::fs::remove_dir_all(&root);
        for kind in media::ART_CHAIN {
            std::fs::create_dir_all(root.join("snes").join(kind)).unwrap();
        }
        let has = |kind: &str| {
            media::find_local(&root, "snes", "Game", kind).is_some()
        };
        assert!(!has(media::SCREENSHOTS));
        std::fs::write(root.join("snes").join(media::SCREENSHOTS).join("Game.jpg"), b"x").unwrap();
        assert!(has(media::SCREENSHOTS), "a screenshot counts as artwork here");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A permission problem is one cause with one fix. Reported once, with the
    /// fix in it — not as 2,700 identical failures, and not silently as a run
    /// that simply produced worse art than it should have.
    #[test]
    fn a_token_that_cannot_write_is_explained_once_and_actionably() {
        let r = Report { fetched: 900, box_only: 900, needs_write_scope: true, ..Default::default() };
        let msg = r.describe();
        assert!(msg.contains("900 fetched"), "{msg}");
        assert!(msg.contains("roms.write"), "the message has to name the scope: {msg}");
        assert!(msg.contains("Client tokens"), "and where to set it: {msg}");
        // Once, not per game.
        assert_eq!(msg.matches("roms.write").count(), 1, "{msg}");
    }

    /// A run where the server accepted every match must not mention scopes at
    /// all: a warning that fires on success trains you to ignore it.
    #[test]
    fn a_clean_run_says_nothing_about_permissions() {
        let r = Report { fetched: 40, ..Default::default() };
        let msg = r.describe();
        assert!(!msg.contains("roms.write"), "{msg}");
        assert!(!msg.contains("box only"), "{msg}");
    }

    /// Pushing covers back is worth reporting, and a run that pushed none must
    /// not claim to have.
    #[test]
    fn covers_pushed_back_are_counted_only_when_there_were_some() {
        let r = Report { fetched: 12, covers_pushed: 12, ..Default::default() };
        assert!(r.describe().contains("12 cover(s) pushed"), "{}", r.describe());
        assert!(!Report { fetched: 12, ..Default::default() }.describe().contains("pushed"));
    }

    /// The five real collisions from one Mega Drive run. Each put another
    /// game's box on the shelf and pushed it to the server as that game's
    /// cover, which is worse than leaving the card blank.
    #[test]
    fn a_sequel_is_never_matched_to_the_game_it_follows() {
        for (rom, candidate) in [
            ("Road Rash II (USA, Europe)", "Road Rash"),
            ("PGA Tour Golf III (USA, Europe)", "PGA Tour Golf"),
            ("HardBall III (USA)", "HardBall '94"),
            ("King of the Monsters 2 (USA)", "King of the Monsters"),
            ("Chavez II (USA)", "Boxing Legends of the Ring"),
        ] {
            assert!(!plausible_match(rom, candidate), "{rom} accepted {candidate}");
            // And not the other way round either.
            assert!(!plausible_match(candidate, rom), "{candidate} accepted {rom}");
        }
    }

    /// Rejecting too much is the other failure, and it looks like a scraper
    /// that simply does not work. Punctuation and region tags differ between
    /// the filename and every metadata source; none of that is a mismatch.
    #[test]
    fn the_same_game_written_differently_still_matches() {
        for (rom, candidate) in [
            ("Boogerman - A Pick and Flick Adventure (USA)", "Boogerman: A Pick and Flick Adventure"),
            ("Road Rash (USA, Europe)", "Road Rash"),
            ("Sonic The Hedgehog 2 (World)", "Sonic the Hedgehog 2"),
            ("Streets of Rage II (Japan)", "Streets of Rage II"),
            ("Aladdin (USA) (Rev A)", "Aladdin"),
        ] {
            assert!(plausible_match(rom, candidate), "{rom} rejected {candidate}");
        }
    }

    #[test]
    fn the_numeral_is_what_separates_one_entry_from_the_next() {
        assert_eq!(edition(&normalize("Road Rash II")), Some("ii".to_owned()));
        assert_eq!(edition(&normalize("King of the Monsters 2")), Some("2".to_owned()));
        assert_eq!(edition(&normalize("HardBall '94")), Some("'94".to_owned()));
        assert_eq!(edition(&normalize("Road Rash")), None);
        // A title merely ending in a word is not an edition.
        assert_eq!(edition(&normalize("Streets of Rage")), None);
    }

    #[test]
    fn region_tags_and_punctuation_fall_away() {
        assert_eq!(normalize("Boogerman: A Pick and Flick Adventure (USA) [!]"),
                   "boogerman a pick and flick adventure");
        assert_eq!(normalize("Road Rash (USA, Europe)"), "road rash");
    }
}

//! The network, on a thread of its own.
//!
//! The menu draws at 640×480 on a device with four slow cores; a sync that
//! blocked the loop would look exactly like the app having hung, and this one
//! has looked hung enough times already. So everything here runs on a worker
//! and reports back through a channel the interface drains once a frame.
//!
//! `sync::Stage` is the vocabulary. This file only produces those states — it
//! makes no decisions, which is why the decisions are all testable without a
//! server.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use moose_rack::cache::Cache;
use moose_rack::config::Config;
use moose_rack::coremap::CoreMap;

use crate::sync::{Review, Stage, Stars};

/// Where the library index lives.
///
/// Matching a save to a server rom id needs the cache, and building one means
/// pulling the whole library — 7,883 rows on this device. The archived front
/// end already did that, so its database is looked for before anything is
/// rebuilt. A wrong guess here is not fatal: an absent cache means every save
/// resolves to nothing and the plan comes back empty, which reads as "sync is
/// broken" — so the search order is written down rather than left to luck.
pub fn find_cache(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// The places worth looking, in order.
pub fn cache_search_path(app_dir: &Path) -> Vec<PathBuf> {
    vec![
        // Ours, once the addon builds its own.
        app_dir.join("cache.sqlite3"),
        // The archived front end's, on this device.
        app_dir.join("../moose-rack/cache.sqlite3"),
        PathBuf::from("/userdata/system/moose-rack/cache.sqlite3"),
    ]
}

/// What the worker sends back.
#[derive(Debug)]
pub enum Message {
    /// Something to show while waiting. Not progress in the counted sense —
    /// scanning and negotiating have no total until they finish.
    Note(String),
    Plan(Box<Review>),
    /// A favourites-and-collections plan, shown before anything moves.
    Stars(Box<crate::favrun::Plan>),
    /// A sync ran. `conflicts` are the saves that changed on both sides:
    /// nothing was written for those and they still need a person.
    Finished {
        moved: usize,
        note: String,
        conflicts: Vec<moose_rack::savesync::SaveConflict>,
    },
    Failed(String),
}

/// A running job. Dropping it does not cancel the thread; the channel simply
/// stops being read, which is the right behaviour when the app is closing.
pub struct Job {
    rx: Receiver<Message>,
}

impl Job {
    /// Everything the worker has said since the last look.
    ///
    /// Drains rather than taking one, so a burst of notes cannot leave the
    /// interface a frame behind the truth.
    pub fn drain(&self) -> Vec<Message> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(m) => out.push(m),
                Err(TryRecvError::Empty) => break,
                // The thread finished and dropped its end. Nothing more is
                // coming, which is not an error.
                Err(TryRecvError::Disconnected) => break,
            }
        }
        out
    }
}

/// Everything both jobs need before they can talk to the server.
///
/// Split out because the two used to be one function with a boolean, and the
/// half that scans is the half most likely to be wrong on a new device — it
/// wants to fail in one place with one message.
struct Ready {
    candidates: Vec<moose_rack::saves::Candidate>,
}

fn prepare(
    app_dir: &Path,
    ra_root: &Path,
    say: &dyn Fn(Message),
) -> Result<Ready, String> {
    let Some(cache_path) = find_cache(&cache_search_path(app_dir)) else {
        return Err("no library index — the save list cannot be matched to the server".into());
    };
    say(Message::Note("reading the library index".into()));
    let cache = Cache::open(&cache_path).map_err(|e| format!("opening the index: {e:#}"))?;
    let map = CoreMap::load_or_embedded(&app_dir.join("data/esde-core-map.json"));

    say(Message::Note("scanning saves".into()));
    let candidates = moose_rack::savesync::scan(&cache, &map, ra_root)
        .map_err(|e| format!("scanning saves: {e:#}"))?;
    // The cache and the map have done their work — the candidates carry the
    // resolved rom ids from here on, and the SQLite handle must not be held
    // across an await.
    drop(cache);
    Ok(Ready { candidates })
}

/// Favourites and collections, both ways.
///
/// One job for looking and for doing, because the looking is the expensive
/// part — reading nine gamelists off an exFAT card and asking the server for
/// every collection — and doing it twice to carry out what it just worked out
/// would double the wait for no gain. `carry_out` decides which.
///
/// Unlike the save sync this does *not* re-negotiate before acting. There is
/// no server-side session to go stale, and re-reading would only widen the
/// window in which the card changes under us.
pub fn stars(cfg: &Config, app_dir: &Path, carry_out: bool) -> Job {
    let (tx, rx) = channel();
    let server = cfg.server.url.clone();
    let username = cfg.server.username.clone();
    let password = cfg.server.password.clone();
    let token = cfg.server.token.clone();
    let app_dir = app_dir.to_path_buf();

    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let say = move |m: Message| {
            let _ = tx2.send(m);
        };
        let Some(cache_path) = find_cache(&cache_search_path(&app_dir)) else {
            return say(Message::Failed(
                "no library index — the stars cannot be matched to the server".into(),
            ));
        };
        let cache = match Cache::open(&cache_path) {
            Ok(c) => c,
            Err(e) => return say(Message::Failed(format!("opening the index: {e:#}"))),
        };
        let platform = moose_rack::platform::current();
        let es = crate::favmap::EsPaths::knulli();

        say(Message::Note("looking at what is on the card".into()));
        let known = match crate::favmap::on_card(&cache, platform, &es.roms) {
            Ok(k) => k,
            Err(e) => return say(Message::Failed(format!("reading the card: {e:#}"))),
        };

        let baseline_path = app_dir.join("favorites-baseline.json");
        let mut baseline = crate::favsync::Baseline::load(&baseline_path);

        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        say(Message::Note("asking the server for its collections".into()));
        let result: Result<Message, String> = runtime.block_on(async {
            let client = moose_rack::api::Client::with_auth(
                &server,
                &username,
                &password,
                token.as_deref().filter(|t| !t.is_empty()),
            )
            .map_err(|e| format!("{e:#}"))?;
            let plan =
                crate::favrun::plan(&client, &cache, &es, &known, &baseline, platform)
                    .await
                    .map_err(|e| format!("{e:#}"))?;
            if !carry_out {
                return Ok(Message::Stars(Box::new(plan)));
            }
            say(Message::Note("applying".into()));
            let report = crate::favrun::carry_out(&client, &es, &known, &plan, &mut baseline)
                .await
                .map_err(|e| format!("{e:#}"))?;
            // The baseline is saved after the work, never before: a run that
            // died halfway must be worked out again, not recorded as agreed.
            if let Err(e) = baseline.save(&baseline_path) {
                return Err(format!("saving what was agreed: {e:#}"));
            }
            let shown = crate::favrun::show_all(&es, &plan).unwrap_or(false);
            let mut note = match (report.applied_here, report.sent) {
                (0, 0) => "nothing to change".to_owned(),
                (0, n) => format!("{n} sent"),
                (n, 0) => format!("{n} applied here"),
                (a, b) => format!("{a} applied here, {b} sent"),
            };
            if shown {
                note.push_str(" — EmulationStation told to show them");
            }
            if !report.failed.is_empty() {
                note.push_str(&format!(" ({} failed)", report.failed.len()));
            }
            Ok(Message::Finished {
                moved: report.applied_here + report.sent,
                note,
                conflicts: Vec::new(),
            })
        });
        match result {
            Ok(m) => say(m),
            Err(e) => say(Message::Failed(e)),
        }
    });

    Job { rx }
}

/// Carry out what the plan said.
///
/// It negotiates again rather than replaying the plan it was shown. That is
/// deliberate: minutes may have passed, another device may have pushed, and
/// the server is the one that knows. The plan a person accepted is a
/// statement of intent, not a set of instructions to execute blind.
pub fn carry_out(cfg: &Config, ra_root: &Path, app_dir: &Path, library_root: &Path) -> Job {
    let (tx, rx) = channel();
    let server = cfg.server.url.clone();
    let username = cfg.server.username.clone();
    let password = cfg.server.password.clone();
    let token = cfg.server.token.clone();
    let ra_root = ra_root.to_path_buf();
    let app_dir = app_dir.to_path_buf();
    let library_root = library_root.to_path_buf();

    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let say = move |m: Message| {
            let _ = tx2.send(m);
        };
        let ready = match prepare(&app_dir, &ra_root, &say) {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(e)),
        };
        say(Message::Note("syncing".into()));
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        let result = runtime.block_on(async {
            let client = moose_rack::api::Client::with_auth(
                &server,
                &username,
                &password,
                token.as_deref(),
            )?;
            moose_rack::savesync::run_all(
                &client,
                &ready.candidates,
                &ra_root,
                &app_dir,
                &library_root,
            )
            .await
        });
        match result {
            Ok(summary) => say(Message::Finished {
                moved: summary.uploaded + summary.downloaded,
                note: summary.headline(),
                conflicts: summary.conflicts,
            }),
            Err(e) => say(Message::Failed(format!("{e:#}"))),
        }
    });

    Job { rx }
}

/// Rebuild this device's list of games from the server.
///
/// Matching a save to a server save is done by rom id, and ids are the
/// server's to change: a rescan there renumbers everything. The index this
/// device inherited from the archived front end had Chrono Trigger as 6985
/// where the server now says 9272, so every save the Flip described was a
/// game the server did not recognise and every one came back "upload this,
/// it is new".
///
/// A **full** pull, into our own file. An incremental one keys on id, so the
/// stale rows would survive alongside the new ones and a save could match
/// either — which is worse than not matching at all.
pub fn refresh_index(cfg: &Config, app_dir: &Path) -> Job {
    let (tx, rx) = channel();
    let server = cfg.server.url.clone();
    let username = cfg.server.username.clone();
    let password = cfg.server.password.clone();
    let token = cfg.server.token.clone();
    let path = app_dir.join("cache.sqlite3");

    std::thread::spawn(move || {
        let say = move |m: Message| {
            let _ = tx.send(m);
        };
        if server.trim().is_empty() {
            return say(Message::Failed("no server in config.toml".into()));
        }
        // Start clean. Anything left from a previous index is by definition
        // numbered the old way.
        let _ = std::fs::remove_file(&path);
        say(Message::Note("rebuilding the game list".into()));

        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        // `Cache::sync` answers (platforms, roms, was-incremental) — not
        // (upserted, removed), which is how the first run reported "36 games
        // listed, 9371 dropped" for a refresh that had in fact pulled 9,371
        // games across 36 platforms.
        let result: anyhow::Result<(usize, usize)> = runtime.block_on(async {
            let client = moose_rack::api::Client::with_auth(
                &server,
                &username,
                &password,
                token.as_deref(),
            )?;
            let mut cache = Cache::open(&path)?;
            let (platforms, roms, _) = cache.sync(&client, true).await?;
            Ok((platforms, roms))
        });

        match result {
            Ok((platforms, roms)) => say(Message::Finished {
                moved: roms,
                note: format!("{roms} games across {platforms} systems"),
                conflicts: Vec::new(),
            }),
            Err(e) => say(Message::Failed(format!("{e:#}"))),
        }
    });

    Job { rx }
}

/// Take everything the server holds, whatever it thinks this device has.
///
/// `negotiate` will not offer a save this device already took — correctly, and
/// that is the same rule that stops a deleted save coming back. So a device
/// being set up from scratch, or one whose card was wiped, has no way in
/// through the ordinary path. This is that way in: list, fetch, place.
///
/// It overwrites. Everything it replaces goes through `savebackup` first, the
/// same as any download.
pub fn pull_all(cfg: &Config, ra_root: &Path, app_dir: &Path, library_root: &Path) -> Job {
    let (tx, rx) = channel();
    let server = cfg.server.url.clone();
    let username = cfg.server.username.clone();
    let password = cfg.server.password.clone();
    let token = cfg.server.token.clone();
    let ra_root = ra_root.to_path_buf();
    let app_dir = app_dir.to_path_buf();
    let library_root = library_root.to_path_buf();

    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let say = move |m: Message| {
            let _ = tx2.send(m);
        };
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        say(Message::Note("listing what the server holds".into()));

        let result: anyhow::Result<usize> = runtime.block_on(async {
            let client = moose_rack::api::Client::with_auth(
                &server,
                &username,
                &password,
                token.as_deref(),
            )?;
            let identity =
                moose_rack::savesync::DeviceIdentity::ensure(&client, &app_dir).await?;
            let saves = client.saves(None).await?;
            let total = saves.len();
            let mut done = 0;
            for save in saves {
                let platform = client
                    .rom_with_files(save.rom_id)
                    .await
                    .ok()
                    .and_then(|r| r.platform_fs_slug)
                    .filter(|s| !s.is_empty());
                let dest = moose_rack::savesync::download_path(
                    &ra_root,
                    &save.file_name,
                    moose_rack::savesync::destination(
                        save.emulator.as_deref(),
                        platform.as_deref(),
                    ),
                );
                let bytes = client.save_content(save.id, &identity.device_id).await?;
                if let Some(dir) = dest.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let slot = save.slot.as_deref().unwrap_or("unslotted");
                let _ = moose_rack::savebackup::keep(&library_root, save.rom_id, slot, &dest);
                std::fs::write(&dest, bytes)?;
                // Tell the server it landed.
                //
                // Without this its per-device bookkeeping still says this
                // device does not hold the save, and the very next negotiate
                // offers to *push* all of them back — which is exactly what
                // happened the first time this ran.
                if let Err(e) = client.confirm_download(save.id).await {
                    eprintln!("could not confirm {}: {e:#}", save.file_name);
                }
                done += 1;
                say(Message::Note(format!("{done} of {total}")));
            }
            Ok(done)
        });

        match result {
            Ok(moved) => say(Message::Finished {
                moved,
                note: format!("{moved} pulled"),
                conflicts: Vec::new(),
            }),
            Err(e) => say(Message::Failed(format!("{e:#}"))),
        }
    });

    Job { rx }
}

/// Ask the server what a sync would do. Moves nothing.
///
/// This is the whole of the first stage: scan what is on the card, hand it to
/// `/api/sync/negotiate`, and hand back the plan for a person to look at.
pub fn negotiate(cfg: &Config, ra_root: &Path, app_dir: &Path) -> Job {
    let (tx, rx) = channel();
    let server = cfg.server.url.clone();
    let username = cfg.server.username.clone();
    let password = cfg.server.password.clone();
    let token = cfg.server.token.clone();
    let ra_root = ra_root.to_path_buf();
    let app_dir = app_dir.to_path_buf();

    std::thread::spawn(move || {
        let say = move |m: Message| {
            // A closed channel means the app moved on. Nothing to report to.
            let _ = tx.send(m);
        };
        if server.trim().is_empty() {
            return say(Message::Failed("no server in config.toml".into()));
        }
        let ready = match prepare(&app_dir, &ra_root, &say) {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(e)),
        };

        let (states, skipped) = moose_rack::savesync::client_states(&ready.candidates);
        say(Message::Note(format!(
            "{} saves found, {skipped} unmatched — asking the server",
            states.len()
        )));

        // The client is async and this thread is not, so it gets a runtime of
        // its own. One current-thread runtime, not the multi-threaded one:
        // there is exactly one request in flight and four slow cores to leave
        // alone.
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        let result = runtime.block_on(async {
            let client = moose_rack::api::Client::with_auth(
                &server,
                &username,
                &password,
                token.as_deref(),
            )?;
            let identity =
                moose_rack::savesync::DeviceIdentity::ensure(&client, &app_dir).await?;
            client.negotiate(&identity.device_id, &states).await
        });

        match result {
            Ok(plan) => say(Message::Plan(Box::new(Review::from_plan(&plan)))),
            Err(e) => say(Message::Failed(format!("{e:#}"))),
        }
    });

    Job { rx }
}

/// Fold everything the worker has said into the stage the interface draws.
///
/// Separate from the thread so the whole of it is testable: the interface's
/// behaviour on a burst of notes, on a plan, and on a failure is decided here
/// and nowhere else.
pub fn apply(
    stage: &mut Stage,
    held: &mut Vec<moose_rack::savesync::SaveConflict>,
    messages: Vec<Message>,
) {
    for message in messages {
        *stage = match message {
            Message::Note(note) => Stage::Asking { note },
            Message::Plan(review) => Stage::Ready(*review),
            Message::Failed(why) => Stage::Failed(why),
            Message::Finished { moved, note, conflicts } => {
                let count = conflicts.len();
                // Kept beside the app rather than inside the stage: they are
                // what the next decision is made from, and a stage that is
                // cheap to clone and compare is worth more than one that
                // carries them.
                *held = conflicts;
                Stage::Done { moved, conflicts: count, note }
            }
            // Belongs to the other job. The caller knows which job it is
            // draining and routes it to `apply_stars`; reaching here means a
            // stars message arrived on the save channel, which nothing sends.
            Message::Stars(_) => continue,
        };
    }
}

/// The same fold, for the favourites job.
///
/// The plan is kept beside the stage rather than inside it, for the reason the
/// conflicts are: the stage is compared every frame to decide whether anything
/// changed, and a plan holding every collection on the server is not something
/// to compare at 60Hz.
pub fn apply_stars(
    stars: &mut Stars,
    held: &mut Option<crate::favrun::Plan>,
    messages: Vec<Message>,
) {
    for message in messages {
        *stars = match message {
            Message::Note(note) => Stars::Asking(note),
            Message::Stars(plan) => {
                let stage = Stars::Ready { headline: plan.headline(), moves: plan.total() };
                *held = Some(*plan);
                stage
            }
            Message::Failed(why) => Stars::Failed(why),
            Message::Finished { note, .. } => {
                // Carried out: the plan it was made from is spent.
                *held = None;
                Stars::Done(note)
            }
            Message::Plan(_) => continue,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_word_wins() {
        // Notes arrive faster than frames. Showing the first of a burst would
        // leave the line a step behind what the worker is actually doing.
        let mut stage = Stage::Idle;
        apply(
            &mut stage,
            &mut Vec::new(),
            vec![
                Message::Note("reading the library index".into()),
                Message::Note("scanning saves".into()),
            ],
        );
        assert_eq!(stage.note(), "scanning saves");
    }

    #[test]
    fn a_plan_arriving_after_notes_is_what_is_shown() {
        let mut stage = Stage::Idle;
        apply(
            &mut stage,
            &mut Vec::new(),
            vec![
                Message::Note("scanning saves".into()),
                Message::Plan(Box::default()),
            ],
        );
        assert!(matches!(stage, Stage::Ready(_)));
        assert!(!stage.is_busy(), "a plan means the worker is done");
    }

    #[test]
    fn a_failure_is_not_left_looking_busy() {
        // The state that mattered: a stage stuck on "asking the server" after
        // the request already failed is indistinguishable from a hang.
        let mut stage = Stage::Asking { note: "asking the server".into() };
        apply(&mut stage, &mut Vec::new(), vec![Message::Failed("no route to host".into())]);
        assert_eq!(stage.note(), "failed: no route to host");
        assert!(!stage.is_busy());
    }

    #[test]
    fn nothing_said_changes_nothing() {
        let mut stage = Stage::Asking { note: "scanning saves".into() };
        apply(&mut stage, &mut Vec::new(), vec![]);
        assert_eq!(stage, Stage::Asking { note: "scanning saves".into() });
    }

    #[test]
    fn the_index_is_looked_for_where_it_actually_is() {
        // The archived front end's database is the one this device has, and
        // rebuilding it means pulling 7,883 rows over wifi.
        let dir = std::env::temp_dir().join("moose-cache-search");
        let _ = std::fs::remove_dir_all(&dir);
        let app = dir.join("moose-patch");
        std::fs::create_dir_all(app.join("../moose-rack")).unwrap();

        assert_eq!(find_cache(&cache_search_path(&app)), None, "nothing to find yet");

        let theirs = app.join("../moose-rack/cache.sqlite3");
        std::fs::write(&theirs, b"x").unwrap();
        assert_eq!(find_cache(&cache_search_path(&app)), Some(theirs));

        // Ours wins once it exists.
        let ours = app.join("cache.sqlite3");
        std::fs::write(&ours, b"x").unwrap();
        assert_eq!(find_cache(&cache_search_path(&app)), Some(ours));
    }
}

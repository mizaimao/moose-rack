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
    /// What went wrong in a run that otherwise finished: transfers that did
    /// not happen. Sent before `Finished`, so it can be put in front of the
    /// person instead of shrinking to a count in the headline.
    Report(Vec<String>),
    Failed(String),
}

/// Every note of a run, into the log. moose-patch.log recorded each button
/// press and nothing about what a press did; now a sync says what moved.
fn log(summary: &moose_rack::savesync::Summary) {
    for note in &summary.notes {
        eprintln!("  {note}");
    }
    eprintln!("sync: {}", summary.headline());
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

/// Where the gamelists and collection files are copied before a rewrite.
pub fn es_backups(app_dir: &Path) -> PathBuf {
    app_dir.join("es-backup")
}

/// Favourites and collections: look at what a sync would do.
///
/// The looking is the expensive part — reading nine gamelists off an exFAT
/// card and asking the server for every collection — and the plan it hands
/// back is what [`stars_apply`] carries out, so the second press does not do
/// it again.
///
/// The one thing it writes is the baseline for lists that already agree. Both
/// sides hold those already, and recording it here is what lets a baseline
/// from before stable ids heal when there is nothing to carry out.
pub fn stars(cfg: &Config, app_dir: &Path) -> Job {
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
            // In full in the log: the panel has room for the names only.
            for u in &plan.unread {
                eprintln!("stars: not read: {u}");
            }
            if crate::favrun::record_agreement(&plan, &mut baseline)
                && let Err(e) = baseline.save(&baseline_path)
            {
                return Err(format!("saving what was agreed: {e:#}"));
            }
            Ok(Message::Stars(Box::new(plan)))
        });
        match result {
            Ok(m) => say(m),
            Err(e) => say(Message::Failed(e)),
        }
    });

    Job { rx }
}

/// Carry out the plan that was shown, as it was shown.
///
/// Not worked out again: what a person accepted is what runs. Refused while
/// EmulationStation is running and the plan writes a file it keeps in memory,
/// because ES would write its own copy back over it on its next exit. From
/// L2+R2 ES is already stopped; opened as a Port, it is not.
pub fn stars_apply(
    cfg: &Config,
    app_dir: &Path,
    es: crate::favmap::EsPaths,
    plan: crate::favrun::Plan,
) -> Job {
    use crate::es::Frontend as _;
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
        match plan.writes_card(&es) {
            Ok(true) if crate::es::Device::default().es_running() => {
                return say(Message::Failed(
                    "EmulationStation is running and would write its own copy over this — \
                     open moose-patch with L2+R2, or run moose-patch --stars-apply over ssh"
                        .into(),
                ));
            }
            Ok(_) => {}
            Err(e) => return say(Message::Failed(format!("{e:#}"))),
        }
        let baseline_path = app_dir.join("favorites-baseline.json");
        let mut baseline = crate::favsync::Baseline::load(&baseline_path);
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        say(Message::Note("applying".into()));
        let result: Result<Message, String> = runtime.block_on(async {
            let client = moose_rack::api::Client::with_auth(
                &server,
                &username,
                &password,
                token.as_deref().filter(|t| !t.is_empty()),
            )
            .map_err(|e| format!("{e:#}"))?;
            // No `--anyway` from the screen: a mass unstar is refused here and
            // can only be pushed through over ssh, where it is typed.
            let backups = es_backups(&app_dir);
            let report = crate::favrun::carry_out(&client, &es, &plan, &mut baseline, &backups, false)
                .await
                .map_err(|e| format!("{e:#}"))?;
            // The baseline is saved after the work, never before: a run that
            // died halfway must be worked out again, not recorded as agreed.
            if let Err(e) = baseline.save(&baseline_path) {
                return Err(format!("saving what was agreed: {e:#}"));
            }
            let note = report.summary();
            let failed = crate::favrun::failures(&plan, &report);
            if failed.is_empty() {
                return Ok(Message::Finished {
                    moved: report.applied_here + report.sent,
                    note,
                    conflicts: Vec::new(),
                });
            }
            for f in &failed {
                eprintln!("stars: failed: {f}");
            }
            Err(format!("{note}; {} list(s) did not sync: {}", failed.len(), failed.join("; ")))
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
            Ok(summary) => {
                log(&summary);
                if !summary.problems.is_empty() {
                    say(Message::Report(summary.problems.clone()));
                }
                say(Message::Finished {
                    moved: summary.uploaded + summary.downloaded,
                    note: summary.headline(),
                    conflicts: summary.conflicts,
                })
            }
            Err(e) => {
                eprintln!("sync failed: {e:#}");
                say(Message::Failed(format!("{e:#}")))
            }
        }
    });

    Job { rx }
}

/// Settle conflicts the person has decided, one way or the other.
///
/// The on-screen half of `--sync --keep`: the same `savesync::resolve`, which
/// backs up whatever it replaces and is the only path that sends `overwrite`.
pub fn resolve(
    cfg: &Config,
    ra_root: &Path,
    app_dir: &Path,
    library_root: &Path,
    decided: Vec<(moose_rack::savesync::SaveConflict, moose_rack::savesync::Keep)>,
) -> Job {
    let (tx, rx) = channel();
    let server = cfg.server.url.clone();
    let username = cfg.server.username.clone();
    let password = cfg.server.password.clone();
    let token = cfg.server.token.clone();
    let ra_root = ra_root.to_path_buf();
    let app_dir = app_dir.to_path_buf();
    let library_root = library_root.to_path_buf();

    std::thread::spawn(move || {
        let say = move |m: Message| {
            let _ = tx.send(m);
        };
        say(Message::Note(format!("settling {} conflict(s)", decided.len())));
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        let client = match moose_rack::api::Client::with_auth(&server, &username, &password, token.as_deref()) {
            Ok(c) => c,
            Err(e) => return say(Message::Failed(format!("{e:#}"))),
        };
        let mut problems = Vec::new();
        let mut settled = 0;
        for (conflict, keep) in &decided {
            let done = runtime.block_on(moose_rack::savesync::resolve(
                &client,
                conflict,
                *keep,
                &ra_root,
                &library_root,
                &app_dir,
            ));
            match done {
                Ok(message) => {
                    eprintln!("  {message}");
                    settled += 1;
                }
                Err(e) => {
                    eprintln!("  {}: FAILED -- {e:#}", conflict.file_name);
                    problems.push(format!("{}: {e:#}", conflict.file_name));
                }
            }
        }
        eprintln!("conflicts: {settled} settled, {} failed", problems.len());
        if !problems.is_empty() {
            say(Message::Report(problems.clone()));
        }
        say(Message::Finished {
            moved: settled,
            note: format!("{settled} conflict(s) settled{}", if problems.is_empty() {
                String::new()
            } else {
                format!(", {} failed", problems.len())
            }),
            conflicts: Vec::new(),
        });
    });

    Job { rx }
}

/// Rebuild this device's list of games from the server.
///
/// Matching a save to a server save is done by game id. Ids are stable now,
/// derived from where the game's file is, so a rescan no longer renumbers
/// them; what the index goes stale on is games added to or moved on the
/// server. Before stable ids it was worse: the index this device inherited
/// had Chrono Trigger as 6985 where the server said 9272, and every save came
/// back "upload this, it is new".
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
        // Built beside the old one and swapped in only when complete. It used
        // to delete the old index first, so a Wi-Fi drop part way left a
        // partial one, and every save in the systems it was missing went
        // unreported -- which the server reads as "this device does not have
        // it" and offers to overwrite.
        let building = path.with_file_name("cache.sqlite3.building");
        let _ = std::fs::remove_file(&building);
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
            let mut cache = Cache::open(&building)?;
            let (platforms, roms, _) = cache.sync(&client, true).await?;
            drop(cache);
            std::fs::rename(&building, &path)?;
            Ok((platforms, roms))
        });
        if result.is_err() {
            let _ = std::fs::remove_file(&building);
        }

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

/// Take everything the server holds, saves and states, over whatever is here.
///
/// For a card set up again, where the ordinary sync -- which moves only what
/// changed since this device last agreed with the server -- has nothing to go
/// on. It overwrites; everything replaced is backed up first. The work is
/// `savesync::pull_all`, the same download path the sync uses.
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
        let say = move |m: Message| {
            let _ = tx.send(m);
        };
        say(Message::Note("taking everything the server holds".into()));
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => return say(Message::Failed(format!("starting the network: {e}"))),
        };
        let result = runtime.block_on(async {
            let client = moose_rack::api::Client::with_auth(&server, &username, &password, token.as_deref())?;
            moose_rack::savesync::pull_all(&client, &ra_root, &app_dir, &library_root).await
        });
        match result {
            Ok(summary) => {
                log(&summary);
                if !summary.problems.is_empty() {
                    say(Message::Report(summary.problems.clone()));
                }
                say(Message::Finished {
                    moved: summary.downloaded,
                    note: summary.headline(),
                    conflicts: Vec::new(),
                })
            }
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
            let plan = client.negotiate(&identity.device_id, &states).await?;
            let mut review = Review::from_plan(&plan);
            // States too: the server's plan covers saves only, so a plan built
            // from it alone said "nothing to do" while states differed.
            let (lines, agreed) =
                moose_rack::statesync::preview(&client, &ready.candidates, &app_dir).await?;
            review.add_states(lines, agreed);
            anyhow::Ok(review)
        });

        match result {
            Ok(review) => say(Message::Plan(Box::new(review))),
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
            // Taken out by the caller before the fold, and shown.
            Message::Report(_) => continue,
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
            Message::Plan(_) | Message::Report(_) => continue,
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

    /// Audit item 24. The second press planned again and carried out the new
    /// plan, not the one on screen. The server here is nowhere, so a worker
    /// that asked it for the collections again would fail; this one only
    /// writes the star it was shown.
    #[test]
    fn the_second_press_carries_out_the_plan_that_was_shown() {
        use crate::favmap::{EsPaths, Known};
        use crate::favrun::{Held, Item, Plan};
        use crate::favsync::Move;

        let dir = std::env::temp_dir().join("moose-worker-stars-apply");
        let _ = std::fs::remove_dir_all(&dir);
        let es = EsPaths::under(&dir);
        std::fs::create_dir_all(es.roms.join("snes")).unwrap();
        std::fs::write(
            es.gamelist("snes"),
            "<gameList>\n\t<game>\n\t\t<path>./Chrono Trigger (USA).sfc</path>\n\t</game>\n</gameList>\n",
        )
        .unwrap();
        let plan = Plan {
            items: vec![Item {
                id: "34".into(),
                name: "★ Best of snes".into(),
                held: Held::Stars(vec!["snes".into()]),
                moves: vec![Move::StarHere(1)],
                agreed: [1i64].into(),
            }],
            known: vec![Known {
                rom_id: 1,
                folder: "snes".into(),
                rel_dir: String::new(),
                file: "Chrono Trigger (USA).sfc".into(),
            }],
            ..Plan::default()
        };
        let mut cfg = Config::default();
        cfg.server.url = "http://nowhere.invalid".into();
        cfg.server.username = "flip".into();
        cfg.server.password = "pw".into();
        let job = stars_apply(&cfg, &dir, es.clone(), plan);

        let mut stars = Stars::Asking("starting".into());
        let mut held = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while stars.is_busy() && std::time::Instant::now() < deadline {
            apply_stars(&mut stars, &mut held, job.drain());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(matches!(stars, Stars::Done(_)), "{stars:?}");
        let list = crate::eslist::Gamelist::load(&es.gamelist("snes")).unwrap();
        assert!(list.favorites().contains("Chrono Trigger (USA).sfc"));
        let baseline = crate::favsync::Baseline::load(&dir.join("favorites-baseline.json"));
        assert_eq!(baseline.of("34"), [1i64].into());
        assert!(es_backups(&dir).join("snes").is_dir(), "the gamelist was not backed up");
    }

    /// A list that did not sync is named on the panel, with why. It used to
    /// read "1 sent (1 failed)" and the log said nothing more.
    #[test]
    fn a_list_that_did_not_sync_is_named_not_counted() {
        use crate::favmap::EsPaths;
        use crate::favrun::{Held, Item, Plan};
        use crate::favsync::Move;

        let dir = std::env::temp_dir().join("moose-worker-stars-failed");
        let _ = std::fs::remove_dir_all(&dir);
        let es = EsPaths::under(&dir);
        let plan = Plan {
            items: vec![Item {
                id: "45".into(),
                name: "Arcade Fighting".into(),
                held: Held::File,
                moves: vec![Move::StarOnServer(1)],
                agreed: [1i64].into(),
            }],
            unread: vec!["Arcade Maze: reading custom-Arcade Maze.cfg: invalid UTF-8".into()],
            ..Plan::default()
        };
        let mut cfg = Config::default();
        cfg.server.url = "http://nowhere.invalid".into();
        cfg.server.username = "flip".into();
        cfg.server.password = "pw".into();
        let job = stars_apply(&cfg, &dir, es, plan);
        let mut stars = Stars::Asking("starting".into());
        let mut held = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while stars.is_busy() && std::time::Instant::now() < deadline {
            apply_stars(&mut stars, &mut held, job.drain());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let Stars::Failed(why) = &stars else { panic!("{stars:?}") };
        assert!(why.contains("2 list(s) did not sync"), "{why}");
        assert!(why.contains("Arcade Fighting: POST"), "{why}");
        assert!(why.contains("Arcade Maze: reading"), "{why}");
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

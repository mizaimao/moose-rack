//! Save states, which the server treats as a different thing from saves.
//!
//! `/api/saves` and `/api/states` are separate families, and until now every
//! `.state` file was being posted to the saves one — filed on the server as if
//! a freeze-frame snapshot were an in-game save.
//!
//! States are simpler and harder at the same time. Simpler because there is no
//! slot and no device: a state is just a file belonging to a ROM. Harder
//! because none of the conflict machinery exists for them —
//! `/api/sync/negotiate` covers saves only, `POST /api/states` has no overwrite
//! flag and never returns 409, and the server publishes no content hash. So the
//! server cannot tell us what changed and will not refuse anything.
//!
//! That means the comparison happens here, and needs a memory of what was last
//! agreed. Without one, "my copy differs from the server's" cannot distinguish
//! *I played and it did not* from *it changed and I did not*, and picking wrong
//! either uploads over someone else's progress or overwrites your own.
//!
//! The ledger is that memory: the hash of each state as of the last successful
//! sync, kept next to the device identity.
//!
//!   local == ledger, server != ledger  ->  the server moved. Download.
//!   local != ledger, server == ledger  ->  we moved. Upload.
//!   both differ                        ->  a genuine conflict. Ask.
//!   neither differs                    ->  nothing to do.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::Client;
use crate::savesync::{SaveConflict, Summary};
use crate::saves::{Candidate, Kind, Resolution};

const LEDGER: &str = "states-seen.json";

/// What each state looked like when it last agreed with the server.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// `"<rom_id>/<file name>"` -> local content hash at the last sync.
    #[serde(default)]
    seen: BTreeMap<String, String>,
    /// The same key -> the server's fingerprint at the last sync. The server
    /// publishes no hash for a state, so size and timestamp stand in.
    #[serde(default)]
    server: BTreeMap<String, String>,
}

fn key(rom_id: i64, file_name: &str) -> String {
    format!("{rom_id}/{file_name}")
}

/// Size and timestamp as one comparable string, since there is no hash.
fn fingerprint(state: &crate::api::SaveState) -> String {
    format!(
        "{}:{}",
        state.file_size_bytes,
        state.updated_at.as_deref().unwrap_or("")
    )
}

impl Ledger {
    /// Rekey entries recorded under old positional game ids onto stable ones.
    ///
    /// Losing an entry is safe -- the state is asked about again rather than
    /// assumed -- but it turns every synced state into a question, so entries
    /// whose old id has a known game are moved. The rest are kept for later.
    ///
    /// With `settled`, an old-id entry that has no translation is dropped
    /// instead: the migration has given up on it, so the state is compared
    /// with the server's afresh on the next sync and the server's copy is used.
    pub fn remap(&mut self, moves: &std::collections::BTreeMap<i64, i64>, settled: bool) -> usize {
        let mut changed = 0;
        for map in [&mut self.seen, &mut self.server] {
            let old = std::mem::take(map);
            for (key, value) in old {
                let rekeyed = key.split_once('/').and_then(|(id, file)| {
                    let id: i64 = id.parse().ok()?;
                    let new = moves.get(&id).filter(|_| crate::gameid::is_legacy(id))?;
                    Some(format!("{new}/{file}"))
                });
                let legacy = key
                    .split_once('/')
                    .and_then(|(id, _)| id.parse::<i64>().ok())
                    .is_some_and(crate::gameid::is_legacy);
                match rekeyed {
                    Some(k) => {
                        changed += 1;
                        map.entry(k).or_insert(value);
                    }
                    None if legacy && settled => changed += 1,
                    None => {
                        map.insert(key, value);
                    }
                }
            }
        }
        changed
    }

    /// Load, rekey and save the ledger in `dir`, when there is anything to move.
    pub fn adopt_stable_ids(
        dir: &Path,
        moves: &std::collections::BTreeMap<i64, i64>,
        settled: bool,
    ) -> Result<usize> {
        let mut ledger = Self::load(dir);
        let changed = ledger.remap(moves, settled);
        if changed > 0 {
            ledger.save(dir)?;
        }
        Ok(changed)
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(LEDGER))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(LEDGER);
        let body = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
    }

    fn record(&mut self, rom_id: i64, file_name: &str, local_hash: &str, server: Option<&str>) {
        let k = key(rom_id, file_name);
        self.seen.insert(k.clone(), local_hash.to_owned());
        match server {
            Some(f) => {
                self.server.insert(k, f.to_owned());
            }
            None => {
                self.server.remove(&k);
            }
        }
    }
}

/// What to do about one state.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Upload,
    Download,
    /// Both sides moved since they last agreed.
    Conflict,
}

/// The decision, given what each side looks like now and what was last agreed.
///
/// Pure, and the only place the rule lives — every mistake this module could
/// make that costs someone a save is a mistake in this function.
pub fn decide(
    local_hash: Option<&str>,
    server_print: Option<&str>,
    ledger_local: Option<&str>,
    ledger_server: Option<&str>,
) -> Action {
    match (local_hash, server_print) {
        // Only one side has it at all.
        (Some(_), None) => Action::Upload,
        (None, Some(_)) => Action::Download,
        (None, None) => Action::Nothing,
        (Some(l), Some(s)) => {
            // Never synced before and both exist: we cannot tell who is
            // authoritative, so we do not guess.
            let (Some(kl), Some(ks)) = (ledger_local, ledger_server) else {
                return Action::Conflict;
            };
            match (l != kl, s != ks) {
                (false, false) => Action::Nothing,
                (true, false) => Action::Upload,
                (false, true) => Action::Download,
                (true, true) => Action::Conflict,
            }
        }
    }
}

/// Sync the save states among `candidates` for whichever ROMs they resolve to.
///
/// Battery saves in the list are ignored — they go through
/// [`crate::savesync`], which has the server's own negotiation behind it.
pub async fn run(
    client: &Client,
    candidates: &[Candidate],
    library_root: &Path,
    data_dir: &Path,
) -> Result<Summary> {
    let mut summary = Summary::default();
    let mut ledger = Ledger::load(data_dir);

    // Only canonical, resolved states: an unmatched one has no rom_id to file
    // it under, and a superseded one would fight the file that beat it.
    let mine: Vec<(&Candidate, i64)> = candidates
        .iter()
        .filter(|c| c.kind == Kind::State && c.canonical)
        .filter_map(|c| match &c.resolution {
            Resolution::Resolved { rom_id, .. } => Some((c, *rom_id)),
            _ => None,
        })
        .collect();

    // One listing per ROM rather than per state.
    let mut rom_ids: Vec<i64> = mine.iter().map(|(_, id)| *id).collect();
    rom_ids.sort_unstable();
    rom_ids.dedup();

    let mut remote: BTreeMap<i64, Vec<crate::api::SaveState>> = BTreeMap::new();
    for id in &rom_ids {
        match client.states(*id).await {
            Ok(list) => {
                remote.insert(*id, list);
            }
            Err(e) => {
                                summary.fail(format!("could not list states for rom {id}: {e:#}"));
            }
        }
    }

    for (c, rom_id) in &mine {
        let file_name = c
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let k = key(*rom_id, &file_name);
        let server = remote
            .get(rom_id)
            .and_then(|list| list.iter().find(|s| s.file_name == file_name));
        let print = server.map(fingerprint);

        match decide(
            Some(&c.content_hash),
            print.as_deref(),
            ledger.seen.get(&k).map(String::as_str),
            ledger.server.get(&k).map(String::as_str),
        ) {
            Action::Nothing => summary.unchanged += 1,
            Action::Upload => {
                let bytes = match std::fs::read(&c.path) {
                    Ok(b) => b,
                    Err(e) => {
                                                summary.fail(format!("could not read {file_name}: {e}"));
                        continue;
                    }
                };
                match client
                    .upload_state(*rom_id, &file_name, bytes, c.core.as_deref())
                    .await
                {
                    Ok(saved) => {
                        summary.uploaded += 1;
                        summary.notes.push(format!("uploaded state {file_name}"));
                        // The fingerprint has to come from the same endpoint the
                        // next run will compare against. POST and GET report
                        // updated_at differently, so recording the upload
                        // response made the following sync believe the server
                        // had moved and download what it had just sent.
                        let print = match client.states(*rom_id).await {
                            Ok(list) => list
                                .iter()
                                .find(|s| s.file_name == file_name)
                                .map(fingerprint)
                                .unwrap_or_else(|| fingerprint(&saved)),
                            Err(_) => fingerprint(&saved),
                        };
                        ledger.record(*rom_id, &file_name, &c.content_hash, Some(&print));
                    }
                    Err(e) => {
                                                summary.fail(format!("could not upload {file_name}: {e:#}"));
                    }
                }
            }
            Action::Download => {
                let Some(server) = server else { continue };
                match client.state_content(server.id).await {
                    Ok(bytes) => {
                        // Into the file that is there. Working the folder out
                        // again from the core and the server's slug put a state
                        // where nothing reads it wherever the two mappings
                        // disagree, and the ledger then recorded it as taken.
                        let dest = c.path.clone();
                        // Same rule as a save: nothing is overwritten without a
                        // copy of what was there first.
                        if let Err(e) =
                            crate::savebackup::keep(library_root, *rom_id, &c.slot, &dest)
                        {
                            summary.notes.push(format!("could not back up {file_name}: {e}"));
                        }
                        match crate::savesync::write_atomically(&dest, &bytes) {
                            Ok(()) => {
                                summary.downloaded += 1;
                                summary.notes.push(format!("downloaded state {file_name}"));
                                let hash = crate::savehash::compute(&dest).unwrap_or_default();
                                ledger.record(*rom_id, &file_name, &hash, Some(&fingerprint(server)));
                            }
                            Err(e) => {
                                                                summary.fail(format!("could not write {}: {e:#}", dest.display()));
                            }
                        }
                    }
                    Err(e) => {
                                                summary.fail(format!("could not download {file_name}: {e:#}"));
                    }
                }
            }
            Action::Conflict => summary.conflicts.push(SaveConflict {
                rom_id: *rom_id,
                save_id: server.map(|s| s.id),
                slot: Some(c.slot.clone()),
                emulator: c.core.clone().or_else(|| Some(c.core_dir.clone())),
                reason: Some(
                    "this save state changed here and on the server since they last agreed"
                        .to_owned(),
                ),
                local_updated: Some(crate::savesync::rfc3339(mtime_secs(&c.path))),
                local_bytes: c.size as i64,
                local_path: Some(c.path.clone()),
                server_updated: server.and_then(|s| s.updated_at.clone()),
                file_name,
            }),
        }
    }

    // States that exist only on the server. Until now nothing brought these
    // down: the loop above walks this device's files, so a state made on
    // another device never arrived.
    match client.all_states().await {
        Ok(all) => {
            for (s, dest) in incoming(&all, candidates, &ledger) {
                match client.state_content(s.id).await {
                    Ok(bytes) => match crate::savesync::write_atomically(&dest, &bytes) {
                        Ok(()) => {
                            summary.downloaded += 1;
                            summary.notes.push(format!("downloaded state {}", s.file_name));
                            let hash = crate::savehash::compute(&dest).unwrap_or_default();
                            ledger.record(s.rom_id, &s.file_name, &hash, Some(&fingerprint(s)));
                        }
                        Err(e) => {
                                                        summary.fail(format!("could not write {}: {e:#}", dest.display()));
                        }
                    },
                    Err(e) => {
                                                summary.fail(format!("could not download {}: {e:#}", s.file_name));
                    }
                }
            }
        }
        Err(e) => {
                        summary.fail(format!("could not list the server's states: {e:#}"));
        }
    }

    // Best effort: losing the ledger costs one round of extra comparison, not
    // any data, so it must not fail the sync that just succeeded.
    if let Err(e) = ledger.save(data_dir) {
        summary.notes.push(format!("could not record state sync: {e}"));
    }
    Ok(summary)
}

/// Take every state the server holds, over whatever is here.
///
/// The states half of [`crate::savesync::pull_all`]. A state goes where this
/// device keeps that system's states, worked out from the server's platform
/// for the game; one the server cannot place is reported, not guessed at.
pub async fn pull_all(
    client: &Client,
    ra_root: &Path,
    library_root: &Path,
    data_dir: &Path,
) -> Result<Summary> {
    let mut summary = Summary::default();
    let mut ledger = Ledger::load(data_dir);
    for s in client.all_states().await? {
        let platform = match crate::platform::current().save_layout() {
            crate::platform::SaveLayout::ByCore => None,
            crate::platform::SaveLayout::BySystem => {
                match client.rom_with_files(s.rom_id).await.ok().and_then(|r| r.platform_fs_slug) {
                    Some(p) if !p.is_empty() => Some(p),
                    _ => {
                        summary.fail(format!("{}: the server did not say which system it is", s.file_name));
                        continue;
                    }
                }
            }
        };
        let dest = crate::savesync::download_path(
            ra_root,
            &s.file_name,
            crate::savesync::destination(s.emulator.as_deref(), platform.as_deref()),
        );
        let bytes = match client.state_content(s.id).await {
            Ok(b) => b,
            Err(e) => {
                summary.fail(format!("could not download {}: {e:#}", s.file_name));
                continue;
            }
        };
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        if let Err(e) = crate::savebackup::keep(library_root, s.rom_id, "unslotted", &dest) {
            summary.notes.push(format!("could not back up {}: {e:#}", dest.display()));
        }
        match crate::savesync::write_atomically(&dest, &bytes) {
            Ok(()) => {
                summary.downloaded += 1;
                summary.notes.push(format!("downloaded state {}", dest.display()));
                let hash = crate::savehash::compute(&dest).unwrap_or_default();
                ledger.record(s.rom_id, &s.file_name, &hash, Some(&fingerprint(&s)));
            }
            Err(e) => summary.fail(format!("could not write {}: {e:#}", dest.display())),
        }
    }
    if let Err(e) = ledger.save(data_dir) {
        summary.notes.push(format!("could not record state sync: {e}"));
    }
    Ok(summary)
}

/// What a state sync would do, without doing any of it.
///
/// The same decision `run` makes, from one listing of the server's states.
/// Returns the lines worth showing and how many states already agree.
pub async fn preview(
    client: &Client,
    candidates: &[Candidate],
    data_dir: &Path,
) -> Result<(Vec<crate::syncplan::Line>, usize)> {
    use crate::syncplan::{Action as Shown, Line};
    let ledger = Ledger::load(data_dir);
    let all = client.all_states().await?;
    let mut lines = Vec::new();
    let mut agreed = 0;
    for c in candidates.iter().filter(|c| c.kind == Kind::State && c.canonical) {
        let Resolution::Resolved { rom_id, .. } = &c.resolution else { continue };
        let file_name = c.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let k = key(*rom_id, &file_name);
        let server = all.iter().find(|s| s.rom_id == *rom_id && s.file_name == file_name);
        let print = server.map(fingerprint);
        let shown = match decide(
            Some(&c.content_hash),
            print.as_deref(),
            ledger.seen.get(&k).map(String::as_str),
            ledger.server.get(&k).map(String::as_str),
        ) {
            Action::Nothing => {
                agreed += 1;
                continue;
            }
            Action::Upload => Shown::Upload,
            Action::Download => Shown::Download,
            Action::Conflict => Shown::Conflict,
        };
        lines.push(Line { action: shown, title: file_name, reason: None, rom_id: *rom_id, save_id: server.map(|s| s.id) });
    }
    for (s, _) in incoming(&all, candidates, &ledger) {
        lines.push(Line {
            action: Shown::Download,
            title: s.file_name.clone(),
            reason: Some("only on the server".into()),
            rom_id: s.rom_id,
            save_id: Some(s.id),
        });
    }
    Ok((lines, agreed))
}

/// Server states to bring down, and where each one goes.
///
/// Only for games already played here -- a save or state for the game exists
/// on this device -- and into the same folder as that file, which is where
/// this device's emulator keeps that game's states. A state this device had
/// agreed on and no longer holds was deleted here, and is not brought back.
pub fn incoming<'a>(
    server: &'a [crate::api::SaveState],
    candidates: &[Candidate],
    ledger: &Ledger,
) -> Vec<(&'a crate::api::SaveState, PathBuf)> {
    let resolved = |c: &Candidate| match &c.resolution {
        Resolution::Resolved { rom_id, .. } => Some(*rom_id),
        _ => None,
    };
    let mut out = Vec::new();
    for s in server {
        let here = candidates
            .iter()
            .filter(|c| resolved(c) == Some(s.rom_id))
            .collect::<Vec<_>>();
        let Some(beside) = here.first() else { continue };
        let held = here.iter().any(|c| {
            c.kind == Kind::State && c.path.file_name().is_some_and(|n| n.to_string_lossy() == s.file_name)
        });
        if held || ledger.seen.contains_key(&key(s.rom_id, &s.file_name)) {
            continue;
        }
        out.push((s, beside.path.with_file_name(&s.file_name)));
    }
    out
}

fn mtime_secs(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Carry out a decision about one save-state conflict.
///
/// States need their own path: the saves endpoints would file a freeze-frame as
/// an in-game save, which is the bug this module exists to fix — and doing it
/// while *resolving* a state conflict is an easy way to reintroduce it.
///
/// The ledger is updated either way, or the same conflict is reported again on
/// the next run and the answer never sticks.
pub async fn resolve_one(
    client: &Client,
    conflict: &SaveConflict,
    keep: crate::savesync::Keep,
    ra_root: &Path,
    library_root: &Path,
    data_dir: &Path,
) -> Result<String> {
    let mut ledger = Ledger::load(data_dir);

    let message = match keep {
        crate::savesync::Keep::Local => {
            let path = conflict
                .local_path
                .clone()
                .context("no local copy to keep — nothing to upload")?;
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            client
                .upload_state(
                    conflict.rom_id,
                    &conflict.file_name,
                    bytes,
                    conflict.emulator.as_deref(),
                )
                .await?;
            let hash = crate::savehash::compute(&path).unwrap_or_default();
            let print = server_print(client, conflict.rom_id, &conflict.file_name).await;
            ledger.record(conflict.rom_id, &conflict.file_name, &hash, print.as_deref());
            format!("{}: kept this machine's copy", conflict.file_name)
        }
        crate::savesync::Keep::Server => {
            let state_id = conflict
                .save_id
                .context("the server did not name a state to download")?;
            let bytes = client.state_content(state_id).await?;
            let dest = keep_server_target(conflict, ra_root);
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            let slot = conflict.slot.as_deref().unwrap_or("unslotted");
            crate::savebackup::keep(library_root, conflict.rom_id, slot, &dest).ok();
            crate::savesync::write_atomically(&dest, &bytes)?;

            let hash = crate::savehash::compute(&dest).unwrap_or_default();
            let print = server_print(client, conflict.rom_id, &conflict.file_name).await;
            ledger.record(conflict.rom_id, &conflict.file_name, &hash, print.as_deref());
            format!("{}: kept the server's copy", dest.display())
        }
    };

    ledger.save(data_dir)?;
    Ok(message)
}

/// Where the server's copy goes when someone keeps it.
///
/// The conflicting local file, whenever the conflict names one. Working it out
/// by core instead wrote the state to `states/<core>/` on the Flip, which
/// nothing reads; the ledger then recorded the server's hash, the rejected
/// local state no longer matched it, and the next sync uploaded it over the
/// copy that had just been chosen.
pub fn keep_server_target(conflict: &SaveConflict, ra_root: &Path) -> PathBuf {
    match &conflict.local_path {
        Some(p) => p.clone(),
        None => crate::savesync::download_path(
            ra_root,
            &conflict.file_name,
            crate::savesync::destination(conflict.emulator.as_deref(), None),
        ),
    }
}

/// The server's fingerprint for one state, read from the listing.
///
/// Always the listing, never an upload response: the two report `updated_at`
/// differently, and recording the upload's version made the next sync believe
/// the server had moved and download what it had just sent.
async fn server_print(client: &Client, rom_id: i64, file_name: &str) -> Option<String> {
    client
        .states(rom_id)
        .await
        .ok()?
        .iter()
        .find(|s| s.file_name == file_name)
        .map(fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_entries_follow_their_game_to_its_stable_id() {
        let mut l = Ledger::default();
        l.seen.insert("-12/Game.state1".into(), "h".into());
        l.server.insert("-12/Game.state1".into(), "10:t".into());
        l.seen.insert("-99/Other.state".into(), "x".into());
        l.seen.insert("20000009/Kept.state".into(), "k".into());
        let moves = [(-12i64, 20_000_001i64)].into_iter().collect();
        assert_eq!(l.remap(&moves, false), 2);
        assert_eq!(l.seen.get("20000001/Game.state1").map(String::as_str), Some("h"));
        assert_eq!(l.server.get("20000001/Game.state1").map(String::as_str), Some("10:t"));
        assert!(l.seen.contains_key("-99/Other.state"), "unplaced ids wait");
        assert!(l.seen.contains_key("20000009/Kept.state"), "stable keys untouched");
        assert_eq!(l.remap(&moves, false), 0, "idempotent");
        // Once the migration has settled, what could not be translated goes and
        // the server's copy is used on the next sync.
        assert_eq!(l.remap(&moves, true), 1);
        assert!(!l.seen.contains_key("-99/Other.state"));
        assert!(l.seen.contains_key("20000009/Kept.state"));
    }

    /// One side only. Nothing to weigh up: copy it to the other.
    #[test]
    fn a_state_only_one_side_has_is_simply_copied() {
        assert_eq!(decide(Some("a"), None, None, None), Action::Upload);
        assert_eq!(decide(None, Some("1:t"), None, None), Action::Download);
        assert_eq!(decide(None, None, None, None), Action::Nothing);
    }

    /// Both sides, unchanged since they last agreed.
    #[test]
    fn matching_states_do_nothing() {
        assert_eq!(decide(Some("a"), Some("1:t"), Some("a"), Some("1:t")), Action::Nothing);
    }

    /// Played here, untouched there. This is the ordinary case after a session
    /// and must not be mistaken for a conflict, or every game would stop to ask
    /// a question with an obvious answer.
    #[test]
    fn a_state_changed_only_here_uploads() {
        assert_eq!(decide(Some("b"), Some("1:t"), Some("a"), Some("1:t")), Action::Upload);
    }

    /// Played on another machine, untouched here.
    #[test]
    fn a_state_changed_only_there_downloads() {
        assert_eq!(decide(Some("a"), Some("2:u"), Some("a"), Some("1:t")), Action::Download);
    }

    /// Both moved. The one case where guessing loses somebody's progress.
    #[test]
    fn both_sides_moving_is_a_conflict() {
        assert_eq!(decide(Some("b"), Some("2:u"), Some("a"), Some("1:t")), Action::Conflict);
    }

    /// Never synced before, and both sides already have a copy. There is no
    /// basis to call either one authoritative, so it is asked rather than
    /// guessed — the first sync is exactly when a wrong guess is most likely.
    #[test]
    fn a_first_sync_with_both_sides_populated_asks() {
        assert_eq!(decide(Some("a"), Some("1:t"), None, None), Action::Conflict);
        assert_eq!(decide(Some("a"), Some("1:t"), Some("a"), None), Action::Conflict);
        assert_eq!(decide(Some("a"), Some("1:t"), None, Some("1:t")), Action::Conflict);
    }

    /// The server publishes no hash for a state, so size and timestamp stand in
    /// for one. Either changing has to count as the server having moved.
    #[test]
    fn the_server_fingerprint_uses_both_size_and_time() {
        let state = |bytes, at: &str| crate::api::SaveState {
            id: 1,
            rom_id: 7,
            file_name: "Game.state".to_owned(),
            file_size_bytes: bytes,
            emulator: None,
            updated_at: Some(at.to_owned()),
        };
        let base = fingerprint(&state(100, "2026-08-06T10:00:00Z"));
        assert_ne!(base, fingerprint(&state(101, "2026-08-06T10:00:00Z")), "size");
        assert_ne!(base, fingerprint(&state(100, "2026-08-06T11:00:00Z")), "time");
        assert_eq!(base, fingerprint(&state(100, "2026-08-06T10:00:00Z")));
    }

    /// The ledger is what makes "who moved" answerable, so it has to survive a
    /// restart intact.
    #[test]
    fn the_ledger_round_trips() {
        let dir = std::env::temp_dir().join("moose-rack-statesync-ledger");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        let mut l = Ledger::default();
        l.record(7, "Game.state", "hash-a", Some("100:t"));
        l.record(8, "Other.state1", "hash-b", None);
        l.save(&dir).unwrap();

        let back = Ledger::load(&dir);
        assert_eq!(back.seen.get("7/Game.state").map(String::as_str), Some("hash-a"));
        assert_eq!(back.server.get("7/Game.state").map(String::as_str), Some("100:t"));
        assert_eq!(back.seen.get("8/Other.state1").map(String::as_str), Some("hash-b"));
        assert_eq!(back.server.get("8/Other.state1"), None, "no server copy recorded");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A missing or corrupt ledger must not stop a sync — it costs one round of
    /// extra comparison, not any data.
    #[test]
    fn a_broken_ledger_is_treated_as_empty() {
        let dir = std::env::temp_dir().join("moose-rack-statesync-broken");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Ledger::load(&dir).seen.is_empty(), "absent");

        std::fs::write(dir.join(LEDGER), b"{not json").unwrap();
        assert!(Ledger::load(&dir).seen.is_empty(), "corrupt");
        std::fs::remove_dir_all(&dir).ok();
    }


    fn local(path: &str, rom_id: i64, kind: Kind) -> Candidate {
        Candidate {
            path: std::path::PathBuf::from(path),
            kind,
            core_dir: "snes".into(),
            core: Some("snes9x".into()),
            rom_base: "ActRaiser (USA)".into(),
            slot: "state1".into(),
            size: 10,
            content_hash: "h".into(),
            resolution: Resolution::Resolved { rom_id, platform: "snes".into(), fs_name: "ActRaiser (USA).zip".into() },
            canonical: true,
            superseded_by: None,
        }
    }

    fn on_server(id: i64, rom_id: i64, name: &str) -> crate::api::SaveState {
        crate::api::SaveState {
            id,
            rom_id,
            file_name: name.into(),
            file_size_bytes: 10,
            emulator: None,
            updated_at: Some("1".into()),
        }
    }

    /// A state made on another device used to stay on the server: the sync
    /// only walked this device's own files.
    #[test]
    fn a_state_only_on_the_server_comes_down_beside_the_games_save() {
        let here = [local("/userdata/saves/snes/ActRaiser (USA).srm", 7, Kind::Save)];
        let server = [on_server(1, 7, "ActRaiser (USA).state1")];
        let got = incoming(&server, &here, &Ledger::default());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, std::path::PathBuf::from("/userdata/saves/snes/ActRaiser (USA).state1"));
    }

    #[test]
    fn nothing_comes_down_for_a_game_never_played_here_or_already_held() {
        let here = [local("/userdata/saves/snes/ActRaiser (USA).state1", 7, Kind::State)];
        // Held already: the ordinary three-way rule handles it.
        assert!(incoming(&[on_server(1, 7, "ActRaiser (USA).state1")], &here, &Ledger::default()).is_empty());
        // A game with nothing on this device.
        assert!(incoming(&[on_server(2, 8, "Other.state1")], &here, &Ledger::default()).is_empty());
    }

    /// Deleted here after it last agreed with the server: gone on purpose.
    #[test]
    fn a_state_deleted_here_is_not_brought_back() {
        let here = [local("/userdata/saves/snes/ActRaiser (USA).srm", 7, Kind::Save)];
        let mut ledger = Ledger::default();
        ledger.record(7, "ActRaiser (USA).state2", "h", Some("p"));
        assert!(incoming(&[on_server(3, 7, "ActRaiser (USA).state2")], &here, &ledger).is_empty());
    }

    /// Keeping the server's copy writes into the conflicting file, which is the
    /// one the emulator reads.
    #[test]
    fn keeping_the_servers_state_writes_the_local_file() {
        let c = SaveConflict {
            rom_id: 7,
            save_id: Some(1),
            file_name: "ActRaiser (USA).state1".into(),
            slot: None,
            emulator: Some("snes9x".into()),
            reason: None,
            local_path: Some("/userdata/saves/snes/ActRaiser (USA).state1".into()),
            local_updated: None,
            local_bytes: 0,
            server_updated: None,
        };
        assert_eq!(
            keep_server_target(&c, Path::new("/userdata")),
            std::path::PathBuf::from("/userdata/saves/snes/ActRaiser (USA).state1")
        );
    }
}

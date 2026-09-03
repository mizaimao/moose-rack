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
use std::path::Path;

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
    ra_root: &Path,
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
                summary.notes.push(format!("could not list states for rom {id}: {e}"));
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
                        summary.notes.push(format!("could not read {file_name}: {e}"));
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
                    Err(e) => summary.notes.push(format!("could not upload {file_name}: {e}")),
                }
            }
            Action::Download => {
                let Some(server) = server else { continue };
                match client.state_content(server.id).await {
                    Ok(bytes) => {
                        let dest = crate::savesync::download_path(
                            ra_root,
                            &file_name,
                            crate::savesync::destination(
                                c.core.as_deref().or(Some(&c.core_dir)),
                                c.platform(),
                            ),
                        );
                        if let Some(dir) = dest.parent() {
                            std::fs::create_dir_all(dir).ok();
                        }
                        // Same rule as a save: nothing is overwritten without a
                        // copy of what was there first.
                        if let Err(e) =
                            crate::savebackup::keep(library_root, *rom_id, &c.slot, &dest)
                        {
                            summary.notes.push(format!("could not back up {file_name}: {e}"));
                        }
                        match std::fs::write(&dest, &bytes) {
                            Ok(()) => {
                                summary.downloaded += 1;
                                summary.notes.push(format!("downloaded state {file_name}"));
                                let hash = crate::savehash::compute(&dest).unwrap_or_default();
                                ledger.record(*rom_id, &file_name, &hash, Some(&fingerprint(server)));
                            }
                            Err(e) => summary
                                .notes
                                .push(format!("could not write {}: {e}", dest.display())),
                        }
                    }
                    Err(e) => summary.notes.push(format!("could not download {file_name}: {e}")),
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

    // Best effort: losing the ledger costs one round of extra comparison, not
    // any data, so it must not fail the sync that just succeeded.
    if let Err(e) = ledger.save(data_dir) {
        summary.notes.push(format!("could not record state sync: {e}"));
    }
    Ok(summary)
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
            // A state conflict carries no platform, so this files by core.
            // Correct on RetroArch's layout; on Batocera's, states are out of
            // scope for now by decision, not by accident.
            let dest = crate::savesync::download_path(
                ra_root,
                &conflict.file_name,
                crate::savesync::destination(conflict.emulator.as_deref(), None),
            );
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            let slot = conflict.slot.as_deref().unwrap_or("unslotted");
            crate::savebackup::keep(library_root, conflict.rom_id, slot, &dest).ok();
            std::fs::write(&dest, &bytes)
                .with_context(|| format!("writing {}", dest.display()))?;

            let hash = crate::savehash::compute(&dest).unwrap_or_default();
            let print = server_print(client, conflict.rom_id, &conflict.file_name).await;
            ledger.record(conflict.rom_id, &conflict.file_name, &hash, print.as_deref());
            format!("{}: kept the server's copy", dest.display())
        }
    };

    ledger.save(data_dir)?;
    Ok(message)
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
}

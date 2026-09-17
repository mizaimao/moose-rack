//! Save sync: the one thing a filesystem cannot do on its own.
//!
//! Browsing and downloading are questions about a tree, and the tree can answer
//! them. "Which of these three machines has the newest copy of this save, and
//! did two of them change it since they last agreed" is not — it needs somewhere
//! that remembers what each device last saw. That is the whole reason a service
//! exists rather than a network share.
//!
//! Saves are still files. `<root>/saves/<rom_key>/<file_name>`, with the
//! content hash computed from the bytes and the timestamp taken from the file,
//! so deleting the index costs a rescan and nothing else.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// What the client believes it holds for one save.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ClientSaveState {
    pub rom_id: i64,
    pub file_name: String,
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub emulator: Option<String>,
    pub content_hash: String,
    pub updated_at: String,
    #[serde(default)]
    pub file_size_bytes: i64,
}

/// What the server holds for one save.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerSave {
    pub id: i64,
    pub rom_id: i64,
    pub file_name: String,
    pub file_size_bytes: i64,
    pub content_hash: Option<String>,
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub emulator: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncOperation {
    /// `upload`, `download`, `conflict` or `no_op`.
    pub action: String,
    pub rom_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emulator: Option<String>,
    /// Why this action was chosen. The client shows it verbatim, so it is
    /// addressed to a person deciding what to do about a conflict.
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncPlan {
    pub session_id: Option<i64>,
    pub operations: Vec<SyncOperation>,
    pub total_upload: i64,
    pub total_download: i64,
    pub total_conflict: i64,
    pub total_no_op: i64,
}

/// What a device last agreed with the server about, per save.
///
/// This is the bookkeeping that makes a conflict detectable. Without it the only
/// question you can ask is "are these different", and the answer to that is the
/// same whether one side changed or both did.
pub type Seen = HashMap<(String, i64, String), String>;

fn key(device: &str, rom_id: i64, file: &str) -> (String, i64, String) {
    (device.to_owned(), rom_id, file.to_owned())
}

/// Decide what should happen to every save, given both sides and what this
/// device last saw.
///
/// The rules, and why each exists:
///
/// * Hashes equal — `no_op`. Nothing to move, whatever the timestamps say.
/// * Only the client has it — `upload`.
/// * Only the server has it — `download`.
/// * Both, different, and the device last saw the server's current hash — the
///   client changed alone, so `upload`.
/// * Both, different, and the device last saw the client's current hash — the
///   server changed alone, so `download`.
/// * Both, different, and the device saw neither — both moved since they
///   agreed, so `conflict`. Never resolved silently: a save is hours of
///   somebody's life and the wrong pick is unrecoverable.
/// * Both, different, and nothing was ever seen — `conflict` as well. A first
///   sync that finds two different copies has no basis to choose.
pub fn plan(device: &str, client: &[ClientSaveState], server: &[ServerSave], seen: &Seen) -> SyncPlan {
    let by_key: HashMap<(i64, &str), &ServerSave> =
        server.iter().map(|s| ((s.rom_id, s.file_name.as_str()), s)).collect();
    let client_keys: std::collections::HashSet<(i64, &str)> =
        client.iter().map(|c| (c.rom_id, c.file_name.as_str())).collect();

    let mut ops = Vec::new();

    for c in client {
        let last = seen.get(&key(device, c.rom_id, &c.file_name));
        match by_key.get(&(c.rom_id, c.file_name.as_str())) {
            None => ops.push(op("upload", c.rom_id, None, &c.file_name, c, None,
                "the server does not have this save")),
            Some(s) => {
                let same = s.content_hash.as_deref() == Some(c.content_hash.as_str());
                if same {
                    ops.push(op("no_op", c.rom_id, Some(s.id), &c.file_name, c, Some(s),
                        "both sides hold the same bytes"));
                } else if last.map(|h| Some(h.as_str()) == s.content_hash.as_deref()).unwrap_or(false) {
                    ops.push(op("upload", c.rom_id, Some(s.id), &c.file_name, c, Some(s),
                        "changed here since this device last agreed with the server"));
                } else if last.map(|h| h == &c.content_hash).unwrap_or(false) {
                    ops.push(op("download", c.rom_id, Some(s.id), &c.file_name, c, Some(s),
                        "changed on the server since this device last agreed"));
                } else {
                    ops.push(op("conflict", c.rom_id, Some(s.id), &c.file_name, c, Some(s),
                        "both copies changed since they last agreed; pick one"));
                }
            }
        }
    }

    for s in server {
        if !client_keys.contains(&(s.rom_id, s.file_name.as_str())) {
            ops.push(SyncOperation {
                action: "download".into(),
                rom_id: s.rom_id,
                save_id: Some(s.id),
                file_name: Some(s.file_name.clone()),
                slot: s.slot.clone(),
                emulator: s.emulator.clone(),
                reason: "this device does not have this save".into(),
                server_content_hash: s.content_hash.clone(),
                server_updated_at: s.updated_at.clone(),
            });
        }
    }

    let count = |a: &str| ops.iter().filter(|o| o.action == a).count() as i64;
    SyncPlan {
        session_id: None,
        total_upload: count("upload"),
        total_download: count("download"),
        total_conflict: count("conflict"),
        total_no_op: count("no_op"),
        operations: ops,
    }
}

#[allow(clippy::too_many_arguments)]
fn op(
    action: &str,
    rom_id: i64,
    save_id: Option<i64>,
    file: &str,
    c: &ClientSaveState,
    s: Option<&ServerSave>,
    reason: &str,
) -> SyncOperation {
    SyncOperation {
        action: action.into(),
        rom_id,
        save_id,
        file_name: Some(file.to_owned()),
        slot: c.slot.clone(),
        emulator: c.emulator.clone(),
        reason: reason.into(),
        server_content_hash: s.and_then(|s| s.content_hash.clone()),
        server_updated_at: s.and_then(|s| s.updated_at.clone()),
    }
}


/// Saves on disk, at `<root>/<rom_id>/<file_name>`.
///
/// One directory per game rather than one flat pile, because two games can
/// legitimately hold `battery.srm` and flattening them would silently make one
/// overwrite the other.
pub struct SaveStore {
    root: std::path::PathBuf,
}

/// A stable id for a save, derived from what identifies it.
///
/// Derived rather than allocated so it survives the index being deleted --
/// which is rule one, and would otherwise renumber every save and invalidate
/// every device's bookkeeping. Positive because the client stores it as an i64
/// and a negative id reads as an error elsewhere in this codebase.
///
/// And no wider than 2^53 - 1, because one of the clients is a browser.
/// JavaScript has one number type and it is a double: an id above that is
/// parsed to the nearest representable value and sent back as a different
/// number. Measured -- `4585479350140525600` came back as a 404, because the
/// id the browser quoted was not the id the server had issued.
///
/// Safe to change: these are derived on every scan and the only thing stored
/// against a save is the `seen` bookkeeping, which is keyed by device, rom and
/// file name rather than by this.
const JS_SAFE: i64 = (1i64 << 53) - 1;

pub fn save_id(rom_id: i64, file_name: &str) -> i64 {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(rom_id.to_le_bytes());
    h.update(b"\0");
    h.update(file_name.as_bytes());
    let d = h.finalize();
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    (i64::from_le_bytes(b) & JS_SAFE).max(1)
}

fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

impl SaveStore {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self, rom_id: i64) -> std::path::PathBuf {
        self.root.join(rom_id.to_string())
    }

    /// Every save, or just one game's.
    ///
    /// Hashes are computed from the bytes on every call. That is the cost of
    /// the filesystem being the truth: a save edited by something other than
    /// this service is still described correctly.
    pub fn list(&self, rom_id: Option<i64>) -> Vec<ServerSave> {
        let mut out = Vec::new();
        let dirs: Vec<std::path::PathBuf> = match rom_id {
            Some(id) => vec![self.dir(id)],
            None => std::fs::read_dir(&self.root)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
                .unwrap_or_default(),
        };
        for d in dirs {
            let Some(rid) = d.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<i64>().ok())
            else {
                continue;
            };
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
                let Ok(bytes) = std::fs::read(&p) else { continue };
                let updated = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                out.push(ServerSave {
                    id: save_id(rid, name),
                    rom_id: rid,
                    file_name: name.to_owned(),
                    file_size_bytes: bytes.len() as i64,
                    content_hash: Some(md5_hex(&bytes)),
                    slot: None,
                    emulator: None,
                    updated_at: updated.map(|s| format!("{s}")),
                });
            }
        }
        out.sort_by(|a, b| (a.rom_id, &a.file_name).cmp(&(b.rom_id, &b.file_name)));
        out
    }

    pub fn write(&self, rom_id: i64, file_name: &str, bytes: &[u8]) -> std::io::Result<ServerSave> {
        let d = self.dir(rom_id);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join(file_name), bytes)?;
        Ok(self
            .list(Some(rom_id))
            .into_iter()
            .find(|s| s.file_name == file_name)
            .expect("just written"))
    }

    /// The bytes of one save, found by id.
    ///
    /// By id rather than by name because that is what the client holds after a
    /// negotiate, and it is the id that is stable across a rescan.
    pub fn read(&self, id: i64) -> Option<Vec<u8>> {
        self.list(None)
            .into_iter()
            .find(|s| s.id == id)
            .and_then(|s| std::fs::read(self.dir(s.rom_id).join(&s.file_name)).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(rom: i64, file: &str, hash: &str) -> ClientSaveState {
        ClientSaveState {
            rom_id: rom,
            file_name: file.into(),
            slot: None,
            emulator: None,
            content_hash: hash.into(),
            updated_at: "2026-09-03T00:00:00Z".into(),
            file_size_bytes: 1,
        }
    }

    fn s(id: i64, rom: i64, file: &str, hash: &str) -> ServerSave {
        ServerSave {
            id,
            rom_id: rom,
            file_name: file.into(),
            file_size_bytes: 1,
            content_hash: Some(hash.into()),
            slot: None,
            emulator: None,
            updated_at: Some("2026-09-03T00:00:00Z".into()),
        }
    }

    fn actions(p: &SyncPlan) -> Vec<&str> {
        p.operations.iter().map(|o| o.action.as_str()).collect()
    }

    #[test]
    fn identical_bytes_are_a_no_op_whatever_the_clock_says() {
        let mut client = c(1, "a.srm", "aaa");
        client.updated_at = "1999-01-01T00:00:00Z".into();
        let p = plan("dev", &[client], &[s(10, 1, "a.srm", "aaa")], &Seen::new());
        assert_eq!(actions(&p), ["no_op"]);
        assert_eq!(p.total_no_op, 1);
    }

    #[test]
    fn only_the_client_has_it_so_upload() {
        let p = plan("dev", &[c(1, "a.srm", "aaa")], &[], &Seen::new());
        assert_eq!(actions(&p), ["upload"]);
        assert_eq!(p.total_upload, 1);
    }

    #[test]
    fn only_the_server_has_it_so_download() {
        let p = plan("dev", &[], &[s(10, 1, "a.srm", "aaa")], &Seen::new());
        assert_eq!(actions(&p), ["download"]);
        assert_eq!(p.total_download, 1);
    }

    /// The device last agreed on the server's current bytes, so only this side
    /// moved. Uploading is safe.
    #[test]
    fn changed_here_alone_is_an_upload() {
        let mut seen = Seen::new();
        seen.insert(key("dev", 1, "a.srm"), "server-hash".into());
        let p = plan("dev", &[c(1, "a.srm", "new-local")], &[s(10, 1, "a.srm", "server-hash")], &seen);
        assert_eq!(actions(&p), ["upload"]);
    }

    /// The device last agreed on the bytes it still holds, so the server moved
    /// and this side did not.
    #[test]
    fn changed_on_the_server_alone_is_a_download() {
        let mut seen = Seen::new();
        seen.insert(key("dev", 1, "a.srm"), "local-hash".into());
        let p = plan("dev", &[c(1, "a.srm", "local-hash")], &[s(10, 1, "a.srm", "server-moved")], &seen);
        assert_eq!(actions(&p), ["download"]);
    }

    /// Both moved since they last agreed. This must never be resolved silently:
    /// a save is hours of somebody's life and the wrong pick is unrecoverable.
    #[test]
    fn both_changed_is_a_conflict() {
        let mut seen = Seen::new();
        seen.insert(key("dev", 1, "a.srm"), "the-old-one".into());
        let p = plan("dev", &[c(1, "a.srm", "local-moved")], &[s(10, 1, "a.srm", "server-moved")], &seen);
        assert_eq!(actions(&p), ["conflict"]);
        assert_eq!(p.total_conflict, 1);
    }

    /// A first sync that meets two different copies has no basis to choose.
    #[test]
    fn differing_with_no_history_is_a_conflict_not_a_guess() {
        let p = plan("dev", &[c(1, "a.srm", "local")], &[s(10, 1, "a.srm", "server")], &Seen::new());
        assert_eq!(actions(&p), ["conflict"]);
    }

    /// Bookkeeping is per device: another machine having agreed says nothing
    /// about this one.
    #[test]
    fn another_devices_history_does_not_count_as_ours() {
        let mut seen = Seen::new();
        seen.insert(key("other-device", 1, "a.srm"), "server-hash".into());
        let p = plan("dev", &[c(1, "a.srm", "local")], &[s(10, 1, "a.srm", "server-hash")], &seen);
        assert_eq!(actions(&p), ["conflict"], "someone else's agreement is not ours");
    }

    #[test]
    fn a_mixed_plan_counts_each_kind() {
        let mut seen = Seen::new();
        seen.insert(key("dev", 2, "b.srm"), "srv-b".into());
        let p = plan(
            "dev",
            &[c(1, "a.srm", "same"), c(2, "b.srm", "local-b"), c(3, "c.srm", "only-local")],
            &[s(10, 1, "a.srm", "same"), s(11, 2, "b.srm", "srv-b"), s(12, 4, "d.srm", "only-server")],
            &seen,
        );
        assert_eq!(p.total_no_op, 1);
        assert_eq!(p.total_upload, 2, "b changed here, c is new here");
        assert_eq!(p.total_download, 1, "d is only on the server");
        assert_eq!(p.total_conflict, 0);
    }

    /// The same file name under two different games is two saves.
    #[test]
    fn saves_are_keyed_by_rom_as_well_as_name() {
        let p = plan(
            "dev",
            &[c(1, "a.srm", "x")],
            &[s(10, 2, "a.srm", "x")],
            &Seen::new(),
        );
        assert_eq!(actions(&p), ["upload", "download"], "different games, not the same save");
    }

    fn store() -> (tempdir::TempDir, SaveStore) {
        let d = tempdir::TempDir::new("saves").unwrap();
        let s = SaveStore::new(d.path());
        (d, s)
    }

    #[test]
    fn a_written_save_comes_back_with_its_hash_and_size() {
        let (_d, st) = store();
        let w = st.write(7, "battery.srm", b"hello").unwrap();
        assert_eq!(w.rom_id, 7);
        assert_eq!(w.file_size_bytes, 5);
        assert_eq!(w.content_hash.as_deref(), Some("5d41402abc4b2a76b9719d911017c592"));
        assert_eq!(st.list(Some(7)).len(), 1);
    }

    /// Two games may both hold `battery.srm`. Flattening them would make one
    /// silently overwrite the other.
    #[test]
    fn the_same_name_under_two_games_is_two_saves() {
        let (_d, st) = store();
        st.write(1, "battery.srm", b"one").unwrap();
        st.write(2, "battery.srm", b"two").unwrap();
        let all = st.list(None);
        assert_eq!(all.len(), 2);
        assert_ne!(all[0].id, all[1].id, "ids must not collide across games");
        assert_eq!(st.read(all[0].id).unwrap(), b"one");
        assert_eq!(st.read(all[1].id).unwrap(), b"two");
    }

    /// Ids are derived, not allocated, so deleting and rebuilding the index does
    /// not renumber saves and invalidate every device's bookkeeping.
    #[test]
    fn ids_are_stable_and_positive() {
        let a = save_id(42, "battery.srm");
        assert_eq!(a, save_id(42, "battery.srm"), "same input, same id");
        assert_ne!(a, save_id(43, "battery.srm"));
        assert_ne!(a, save_id(42, "other.srm"));
        assert!(a > 0, "a negative id reads as an error elsewhere");
    }

    #[test]
    fn rewriting_a_save_changes_its_hash_but_not_its_id() {
        let (_d, st) = store();
        let first = st.write(3, "a.srm", b"before").unwrap();
        let second = st.write(3, "a.srm", b"after").unwrap();
        assert_eq!(first.id, second.id);
        assert_ne!(first.content_hash, second.content_hash);
        assert_eq!(st.read(second.id).unwrap(), b"after");
    }

    #[test]
    fn listing_an_empty_or_missing_store_is_not_an_error() {
        let (_d, st) = store();
        assert!(st.list(None).is_empty());
        assert!(st.list(Some(99)).is_empty());
        assert!(st.read(12345).is_none());
    }

    /// The round trip that matters: what `list` reports is what `plan` consumes.
    #[test]
    fn a_stored_save_matching_the_client_plans_as_no_op() {
        let (_d, st) = store();
        let s = st.write(5, "a.srm", b"same-bytes").unwrap();
        let c = ClientSaveState {
            rom_id: 5,
            file_name: "a.srm".into(),
            slot: None,
            emulator: None,
            content_hash: s.content_hash.clone().unwrap(),
            updated_at: "whenever".into(),
            file_size_bytes: 10,
        };
        let p = plan("dev", &[c], &st.list(None), &Seen::new());
        assert_eq!(actions(&p), ["no_op"]);
    }
}

// --- Save states ------------------------------------------------------------
//
// A save *state* is a freeze-frame of the emulator; a save is the cartridge's
// own battery-backed memory. They sync differently and `crate::statesync`
// handles them separately for a reason -- a state belongs to one emulator and
// often to one version of it, so there is no merging to be done and no conflict
// to resolve. The server takes what it is given.
//
// Same store, a different root, because the shapes are the same: files under a
// directory named for the rom id, ids derived from the name so they survive a
// rescan. `emulator` is the one field saves do not carry, and it is remembered
// in the file name rather than beside it -- see `state_file_name`.

/// One state as the client reads it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ServerState {
    pub id: i64,
    pub rom_id: i64,
    pub file_name: String,
    pub file_size_bytes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emulator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// States live beside saves, under their own directory.
pub struct StateStore {
    root: std::path::PathBuf,
}

impl StateStore {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self, rom_id: i64) -> std::path::PathBuf {
        self.root.join(rom_id.to_string())
    }

    /// Where the emulator is recorded.
    ///
    /// In a sibling file rather than in the name, because the name is what the
    /// client matches on when it decides whether it already has this state --
    /// decorating it would make every state look new to a client that had it.
    fn emu_path(&self, rom_id: i64, file_name: &str) -> std::path::PathBuf {
        self.dir(rom_id).join(format!(".{file_name}.emulator"))
    }

    pub fn list(&self, rom_id: Option<i64>) -> Vec<ServerState> {
        let mut out = Vec::new();
        let dirs: Vec<std::path::PathBuf> = match rom_id {
            Some(id) => vec![self.dir(id)],
            None => std::fs::read_dir(&self.root)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
                .unwrap_or_default(),
        };
        for d in dirs {
            let Some(rid) =
                d.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<i64>().ok())
            else {
                continue;
            };
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
                // The sidecars are bookkeeping, not states.
                if name.starts_with('.') && name.ends_with(".emulator") {
                    continue;
                }
                let Ok(meta) = std::fs::metadata(&p) else { continue };
                let updated = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                out.push(ServerState {
                    id: save_id(rid, name),
                    rom_id: rid,
                    file_name: name.to_owned(),
                    file_size_bytes: meta.len() as i64,
                    emulator: std::fs::read_to_string(self.emu_path(rid, name))
                        .ok()
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty()),
                    updated_at: updated.map(|s| format!("{s}")),
                });
            }
        }
        out.sort_by(|a, b| (a.rom_id, &a.file_name).cmp(&(b.rom_id, &b.file_name)));
        out
    }

    /// Take what we are given. No conflict answer: `statesync` decides whether
    /// to send before it calls, because a freeze-frame cannot be merged.
    pub fn write(
        &self,
        rom_id: i64,
        file_name: &str,
        emulator: Option<&str>,
        bytes: &[u8],
    ) -> std::io::Result<ServerState> {
        let d = self.dir(rom_id);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join(file_name), bytes)?;
        match emulator {
            Some(e) if !e.is_empty() => std::fs::write(self.emu_path(rom_id, file_name), e)?,
            // An upload with no emulator clears a stale one rather than leaving
            // the previous uploader's name on somebody else's state.
            _ => {
                let _ = std::fs::remove_file(self.emu_path(rom_id, file_name));
            }
        }
        self.list(Some(rom_id))
            .into_iter()
            .find(|s| s.file_name == file_name)
            .ok_or_else(|| std::io::Error::other("state vanished after writing"))
    }

    pub fn read(&self, id: i64) -> Option<Vec<u8>> {
        self.list(None)
            .into_iter()
            .find(|s| s.id == id)
            .and_then(|s| std::fs::read(self.dir(s.rom_id).join(&s.file_name)).ok())
    }
}

#[cfg(test)]
mod id_tests {
    use super::*;

    /// One of the clients is a browser, and JavaScript has one number type.
    ///
    /// An id above 2^53 - 1 is parsed to the nearest double and quoted back as
    /// a different number, so the download 404s: the browser asks for a save
    /// the server never issued. Found by running the sync in a real browser --
    /// `4585479350140525600` went out and something else came back.
    #[test]
    fn every_id_survives_a_round_trip_through_javascript() {
        const MAX: i64 = (1i64 << 53) - 1;
        for rom in [-10793i64, -1, 1, 7, 999_999] {
            for name in ["a.srm", "ActRaiser (USA).srm", "", "x".repeat(200).as_str()] {
                let id = save_id(rom, name);
                assert!(id > 0, "{rom}/{name}: ids must stay positive");
                assert!(id <= MAX, "{rom}/{name}: {id} cannot be represented in a browser");
                // What `JSON.parse` would make of it, which is what comes back.
                assert_eq!(id as f64 as i64, id, "{rom}/{name}: {id} changes value as a double");
            }
        }
    }

    /// Still derived, so deleting the index and rebuilding gives the same ids.
    #[test]
    fn the_same_save_gets_the_same_id() {
        assert_eq!(save_id(-10793, "ActRaiser (USA).srm"), save_id(-10793, "ActRaiser (USA).srm"));
        assert_ne!(save_id(-10793, "a.srm"), save_id(-10793, "b.srm"));
        assert_ne!(save_id(1, "a.srm"), save_id(2, "a.srm"));
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    fn store(name: &str) -> (tempdir::TempDir, StateStore) {
        let d = tempdir::TempDir::new(name).unwrap();
        let s = StateStore::new(d.path().join("states"));
        (d, s)
    }

    #[test]
    fn a_state_round_trips_with_its_emulator() {
        let (_d, s) = store("st");
        let w = s.write(7, "Game.state1", Some("snes9x"), b"frozen").unwrap();
        assert_eq!(w.rom_id, 7);
        assert_eq!(w.file_name, "Game.state1");
        assert_eq!(w.file_size_bytes, 6);
        assert_eq!(w.emulator.as_deref(), Some("snes9x"));
        assert_eq!(s.read(w.id).as_deref(), Some(&b"frozen"[..]));
        assert_eq!(s.list(Some(7)), vec![w.clone()]);
        assert_eq!(s.list(None), vec![w]);
        assert_eq!(s.list(Some(8)), vec![]);
    }

    /// The sidecar must never be listed as a state of its own, or every state
    /// appears twice and the second one is four bytes of emulator name.
    #[test]
    fn the_emulator_sidecar_is_not_a_state() {
        let (_d, s) = store("st-side");
        s.write(1, "A.state", Some("mesen"), b"x").unwrap();
        let names: Vec<_> = s.list(None).into_iter().map(|x| x.file_name).collect();
        assert_eq!(names, ["A.state"]);
    }

    /// Ids are derived, so deleting the index and rebuilding it gives the same
    /// numbers -- the rule the whole service is built on.
    #[test]
    fn ids_survive_a_rebuild() {
        let (_d, s) = store("st-id");
        let a = s.write(3, "X.state", None, b"one").unwrap();
        let again = s.list(None)[0].clone();
        assert_eq!(a.id, again.id);
        assert_eq!(a.id, save_id(3, "X.state"));
    }

    /// Re-uploading replaces, and an upload with no emulator does not inherit
    /// the last one's.
    #[test]
    fn uploading_again_replaces_and_clears_a_stale_emulator() {
        let (_d, s) = store("st-re");
        let first = s.write(2, "S.state", Some("mupen"), b"aa").unwrap();
        let second = s.write(2, "S.state", None, b"bbbb").unwrap();
        assert_eq!(first.id, second.id, "the same file is the same state");
        assert_eq!(second.file_size_bytes, 4);
        assert_eq!(second.emulator, None, "the previous emulator stuck to a new state");
        assert_eq!(s.list(None).len(), 1);
    }

    #[test]
    fn an_unknown_id_reads_nothing() {
        let (_d, s) = store("st-none");
        s.write(1, "A.state", None, b"x").unwrap();
        assert_eq!(s.read(999), None);
    }
}

/// Moving saves and states stored under positional game ids to stable ones.
///
/// Until `gameid`, a game's id was its position in the scan, and saves live on
/// disk at `<saves>/<rom_id>/`. Checked on dev.lan on 2026-09-17: all seven
/// saves filed under server ids sat under ids that by then named other games,
/// Chrono Trigger's under a Game Boy Color racer. Nothing had been overwritten
/// yet; the next sync would have paired them with the wrong games.
///
/// A save carries its game's name, because every emulator names it after the
/// ROM file: `Chrono Trigger (USA).srm`. So each old folder goes to the game
/// whose file has that stem, when exactly one game does. States are named by
/// time, not game, and follow the save folder with the same old id; the two
/// were written under the same numbering. Anything that does not resolve to a
/// single game is left where it is and reported, never guessed.
pub mod legacy {
    use std::collections::{BTreeMap, HashMap};
    use std::path::Path;

    use moose_rack::gameid;

    /// Marker written when the move is done, holding the report.
    pub const MARKER: &str = ".stable-ids.json";

    #[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
    pub struct Report {
        /// Old id to new id, for everything that moved.
        pub moved: BTreeMap<i64, i64>,
        /// Old id to why it stayed.
        pub left: BTreeMap<i64, String>,
    }

    fn stem(name: &str) -> String {
        let name = moose_rack::savesync::local_name(name);
        let p = Path::new(&name);
        // `.state1`, `.state.auto`: everything from the first save-ish dot.
        let base = p.file_name().and_then(|s| s.to_str()).unwrap_or(&name);
        for ext in [".state", ".srm", ".sav", ".rtc"] {
            if let Some(at) = base.find(ext) {
                return base[..at].to_owned();
            }
        }
        p.file_stem().and_then(|s| s.to_str()).unwrap_or(base).to_owned()
    }

    fn legacy_dirs(root: &Path) -> Vec<(i64, std::path::PathBuf)> {
        let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
        let mut out: Vec<_> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let id: i64 = e.file_name().to_str()?.parse().ok()?;
                gameid::is_legacy(id).then(|| (id, e.path()))
            })
            .collect();
        out.sort();
        out
    }

    /// Work out what would move, without moving anything.
    ///
    /// `games` is every game the server lists, as `(stable id, file name)`.
    pub fn plan(saves_root: &Path, games: &[(i64, String)]) -> Report {
        let mut by_stem: HashMap<String, Vec<i64>> = HashMap::new();
        // Case-folded too, for a save and a ROM that disagree only in case.
        // Found on dev.lan: `Kirby & the Amazing Mirror (USA).srm` beside
        // `Kirby & The Amazing Mirror (USA).zip`. Used only when the exact name
        // matches nothing, and still only when it names a single game.
        let mut by_folded: HashMap<String, Vec<i64>> = HashMap::new();
        for (id, fs_name) in games {
            by_stem.entry(stem(fs_name)).or_default().push(*id);
            by_folded.entry(stem(fs_name).to_lowercase()).or_default().push(*id);
        }
        let mut report = Report::default();
        for (old, dir) in legacy_dirs(saves_root) {
            let names: Vec<String> = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().is_file())
                        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                        .filter(|n| !n.starts_with('.'))
                        .collect()
                })
                .unwrap_or_default();
            let mut targets: Vec<i64> = Vec::new();
            let mut why = None;
            for n in &names {
                let found = by_stem.get(&stem(n)).or_else(|| by_folded.get(&stem(n).to_lowercase()));
                match found.map(Vec::as_slice) {
                    Some([one]) => targets.push(*one),
                    Some(many) => {
                        why = Some(format!("{n} matches {} games", many.len()));
                    }
                    None => why = Some(format!("no game is named like {n}")),
                }
            }
            targets.sort();
            targets.dedup();
            match (why, targets.as_slice()) {
                (None, [new]) => {
                    report.moved.insert(old, *new);
                }
                (None, []) => {
                    report.left.insert(old, "no save files in it".into());
                }
                (None, _) => {
                    report.left.insert(old, "its files name different games".into());
                }
                (Some(reason), _) => {
                    report.left.insert(old, reason);
                }
            }
        }
        // States carry no game name. They go with the save folder of the same
        // old id, or they stay.
        for (old, _) in legacy_dirs(&saves_root.join("_states")) {
            if !report.moved.contains_key(&old) && !report.left.contains_key(&old) {
                report.left.insert(old, "states only, and states are not named after a game".into());
            }
        }
        report
    }

    /// Move one directory's files into another, keeping anything that would
    /// collide where it was.
    fn merge_into(from: &Path, to: &Path) -> std::io::Result<Vec<String>> {
        std::fs::create_dir_all(to)?;
        let mut clashes = Vec::new();
        for e in std::fs::read_dir(from)?.filter_map(|e| e.ok()) {
            let target = to.join(e.file_name());
            if target.exists() {
                clashes.push(e.file_name().to_string_lossy().into_owned());
                continue;
            }
            std::fs::rename(e.path(), target)?;
        }
        if clashes.is_empty() {
            std::fs::remove_dir(from).ok();
        }
        Ok(clashes)
    }

    /// Do it, once. Returns `None` when it has already been done.
    pub fn apply(saves_root: &Path, games: &[(i64, String)]) -> std::io::Result<Option<Report>> {
        let marker = saves_root.join(MARKER);
        if marker.exists() {
            return Ok(None);
        }
        let mut report = plan(saves_root, games);
        let moves: Vec<(i64, i64)> = report.moved.iter().map(|(a, b)| (*a, *b)).collect();
        for (old, new) in moves {
            for sub in ["", "_states"] {
                let base = if sub.is_empty() { saves_root.to_path_buf() } else { saves_root.join(sub) };
                let from = base.join(old.to_string());
                if !from.is_dir() {
                    continue;
                }
                let clashes = merge_into(&from, &base.join(new.to_string()))?;
                if !clashes.is_empty() {
                    report.left.insert(
                        old,
                        format!("{} already existed under {new}: {}", if sub.is_empty() { "saves" } else { "states" }, clashes.join(", ")),
                    );
                }
            }
        }
        std::fs::write(&marker, serde_json::to_vec_pretty(&report).unwrap_or_default())?;
        Ok(Some(report))
    }

    /// Rewrite `device\0rom_id\0file` bookkeeping keys onto the new ids.
    ///
    /// Keys under an old id that did not move are dropped: that id names
    /// nothing any more, and a device that lost its record of agreeing is asked
    /// about the save next time rather than assumed to agree.
    pub fn remap_seen(seen: &mut HashMap<String, String>, moved: &BTreeMap<i64, i64>) -> usize {
        let old: Vec<(String, String)> = seen.drain().collect();
        let mut changed = 0;
        for (key, hash) in old {
            let mut parts = key.splitn(3, '\0');
            let (Some(device), Some(rom), Some(file)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let Ok(rom) = rom.parse::<i64>() else { continue };
            if !gameid::is_legacy(rom) {
                seen.insert(key, hash);
                continue;
            }
            if let Some(new) = moved.get(&rom) {
                seen.insert(format!("{device}\0{new}\0{file}"), hash);
                changed += 1;
            }
        }
        changed
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn tree(name: &str) -> std::path::PathBuf {
            let d = std::env::temp_dir().join(format!("moose-legacy-{name}"));
            std::fs::remove_dir_all(&d).ok();
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        fn put(root: &Path, rel: &str) {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }

        const CT: i64 = 20_000_001;
        const RIPTIDE: i64 = 20_000_002;
        const ASTRO_A: i64 = 20_000_003;
        const ASTRO_B: i64 = 20_000_004;

        fn games() -> Vec<(i64, String)> {
            vec![
                (CT, "Chrono Trigger (USA).zip".into()),
                (RIPTIDE, "Rip-Tide Racer (Europe).zip".into()),
                (ASTRO_A, "Astrohawk (World) (Unl).zip".into()),
                (ASTRO_B, "Astrohawk (World) (Unl).zip".into()),
            ]
        }

        /// The live case: Chrono Trigger's save under the id that now names
        /// Rip-Tide Racer goes to Chrono Trigger, not to Rip-Tide Racer.
        #[test]
        fn a_save_goes_to_the_game_it_is_named_after() {
            let root = tree("named");
            put(&root, "5653/Chrono Trigger (USA).srm");
            let r = apply(&root, &games()).unwrap().unwrap();
            assert_eq!(r.moved.get(&5653), Some(&CT));
            assert!(root.join(format!("{CT}/Chrono Trigger (USA).srm")).is_file());
            assert!(!root.join("5653").exists());
        }

        #[test]
        fn states_follow_the_save_folder_with_the_same_old_id() {
            let root = tree("states");
            put(&root, "-10793/Chrono Trigger (USA).srm");
            put(&root, "_states/-10793/2026-09-08T04-25-32-465Z.state");
            put(&root, "_states/-10793/.2026-09-08T04-25-32-465Z.state.emulator");
            apply(&root, &games()).unwrap();
            assert!(root.join(format!("_states/{CT}/2026-09-08T04-25-32-465Z.state")).is_file());
            assert!(root.join(format!("_states/{CT}/.2026-09-08T04-25-32-465Z.state.emulator")).is_file());
        }

        #[test]
        fn a_name_two_games_share_is_left_alone() {
            let root = tree("ambiguous");
            put(&root, "12/Astrohawk (World) (Unl).srm");
            let r = apply(&root, &games()).unwrap().unwrap();
            assert!(r.moved.is_empty());
            assert!(r.left[&12].contains("matches 2 games"), "{:?}", r.left);
            assert!(root.join("12/Astrohawk (World) (Unl).srm").is_file(), "not moved");
        }

        #[test]
        fn a_save_named_after_no_game_is_left_alone() {
            let root = tree("unknown");
            put(&root, "6411/a-plumber-for-all-seasons_2021-11-22.srm");
            let r = apply(&root, &games()).unwrap().unwrap();
            assert!(r.left.contains_key(&6411));
            assert!(root.join("6411/a-plumber-for-all-seasons_2021-11-22.srm").is_file());
        }

        #[test]
        fn it_runs_once() {
            let root = tree("once");
            put(&root, "5653/Chrono Trigger (USA).srm");
            assert!(apply(&root, &games()).unwrap().is_some());
            put(&root, "7/Chrono Trigger (USA).srm");
            assert!(apply(&root, &games()).unwrap().is_none());
            assert!(root.join("7").exists(), "a second start does not move anything");
        }

        #[test]
        fn a_file_already_under_the_new_id_is_not_overwritten() {
            let root = tree("clash");
            put(&root, "5653/Chrono Trigger (USA).srm");
            std::fs::create_dir_all(root.join(CT.to_string())).unwrap();
            std::fs::write(root.join(format!("{CT}/Chrono Trigger (USA).srm")), b"newer").unwrap();
            let r = apply(&root, &games()).unwrap().unwrap();
            assert_eq!(std::fs::read(root.join(format!("{CT}/Chrono Trigger (USA).srm"))).unwrap(), b"newer");
            assert!(root.join("5653/Chrono Trigger (USA).srm").is_file(), "the old one kept aside");
            assert!(r.left[&5653].contains("already existed"));
        }

        #[test]
        fn a_save_that_differs_from_its_rom_only_in_case_still_goes_home() {
            let root = tree("case");
            put(&root, "1918/chrono trigger (usa).srm");
            let r = apply(&root, &games()).unwrap().unwrap();
            assert_eq!(r.moved.get(&1918), Some(&CT));
        }

        #[test]
        fn a_romm_stamped_name_still_matches() {
            let root = tree("stamped");
            put(&root, "44/Chrono Trigger (USA) [2026-08-06_23-06-01].srm");
            let r = apply(&root, &games()).unwrap().unwrap();
            assert_eq!(r.moved.get(&44), Some(&CT));
        }

        #[test]
        fn bookkeeping_follows_the_move_and_forgets_what_did_not() {
            let mut seen: HashMap<String, String> = [
                ("dev\u{0}5653\u{0}Chrono Trigger (USA).srm", "h1"),
                ("dev\u{0}6411\u{0}plumber.srm", "h2"),
                (&format!("dev\u{0}{CT}\u{0}Already.srm") as &str, "h3"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
            let moved: BTreeMap<i64, i64> = [(5653, CT)].into_iter().collect();
            assert_eq!(remap_seen(&mut seen, &moved), 1);
            assert_eq!(seen.get(&format!("dev\u{0}{CT}\u{0}Chrono Trigger (USA).srm")).map(String::as_str), Some("h1"));
            assert!(!seen.keys().any(|k| k.contains("\u{0}6411\u{0}")), "an unmoved old id names nothing");
            assert!(seen.contains_key(&format!("dev\u{0}{CT}\u{0}Already.srm")), "stable keys untouched");
        }
    }
}

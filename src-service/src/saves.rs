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
pub fn save_id(rom_id: i64, file_name: &str) -> i64 {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(rom_id.to_le_bytes());
    h.update(b"\0");
    h.update(file_name.as_bytes());
    let d = h.finalize();
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    (i64::from_le_bytes(b) & i64::MAX).max(1)
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

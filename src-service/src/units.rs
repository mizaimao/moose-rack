//! Save units: folders an emulator treats as one save, kept whole.
//!
//! `docs/emulator-saves.md` has the design. A unit is keyed by
//! `(family, key)`, the save format and the folder's own name, and travels as
//! a zip made by `moose_rack::saveunit::pack`. The rules for what moves are the
//! save rules, through [`crate::saves::decide_held`].
//!
//! On disk:
//!
//!     <root>/<family>/<key>/current.zip
//!     <root>/<family>/<key>/unit.json
//!     <root>/<family>/<key>/history/<unix seconds>-<hash8>.zip
//!
//! Unlike saves, units keep history: replacing a card folder loses more than
//! replacing one `.srm`, and an emulator that is still running can write an old
//! copy back over a new one (see "The running emulator" in the doc).

use std::path::PathBuf;

use moose_rack::saveunit;
use serde::{Deserialize, Serialize};

/// Replaced copies kept per unit, the same count `savebackup.rs` keeps on a
/// device.
pub const HISTORY: usize = 10;

/// What a device reports for one unit it holds.
#[derive(Debug, Clone, Deserialize)]
pub struct ClientUnit {
    pub family: String,
    pub key: String,
    pub content_hash: String,
}

/// What the server holds for one unit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerUnit {
    pub id: i64,
    pub family: String,
    pub key: String,
    pub content_hash: String,
    pub size: u64,
    /// Unix seconds, as a string, the way saves report it.
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UnitOperation {
    /// `upload`, `download`, `conflict`, `no_op` or `skip`.
    pub action: String,
    pub family: String,
    pub key: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UnitPlan {
    pub operations: Vec<UnitOperation>,
    pub total_upload: usize,
    pub total_download: usize,
    pub total_conflict: usize,
    pub total_no_op: usize,
    pub total_skip: usize,
}

/// The ledger key for a unit, in the same `seen` map saves use. A save key's
/// second field is a number, so the two can never collide.
pub fn seen_key(device: &str, family: &str, key: &str) -> String {
    format!("{device}\0unit:{family}\0{key}")
}

/// Decide what happens to every unit in `families`, given what the device
/// holds and `last`, what it last agreed on per (family, key).
///
/// Units in families the device did not ask about are not mentioned: a phone
/// with no PS2 emulator is never told to download PS2 saves. A unit only the
/// server holds is a download, unless this device agreed on it before; then
/// it was deleted here, and that is respected rather than undone.
pub fn plan(
    families: &[String],
    client: &[ClientUnit],
    server: &[ServerUnit],
    last: &dyn Fn(&str, &str) -> Option<String>,
) -> UnitPlan {
    let held: std::collections::HashSet<(&str, &str)> =
        client.iter().map(|c| (c.family.as_str(), c.key.as_str())).collect();
    let mut ops = Vec::new();
    let with = |action: &str, reason: &str, family: &str, key: &str, s: Option<&ServerUnit>| UnitOperation {
        action: action.into(),
        family: family.into(),
        key: key.into(),
        reason: reason.into(),
        unit_id: s.map(|s| s.id),
        server_content_hash: s.map(|s| s.content_hash.clone()),
        server_size: s.map(|s| s.size),
        server_updated_at: s.map(|s| s.updated_at.clone()),
    };
    for c in client {
        let s = server.iter().find(|s| s.family == c.family && s.key == c.key);
        match s {
            None => ops.push(with("upload", "the server does not have this save", &c.family, &c.key, None)),
            Some(s) => {
                let agreed = last(&c.family, &c.key);
                let (action, reason) =
                    crate::saves::decide_held(&c.content_hash, Some(&s.content_hash), agreed.as_deref());
                ops.push(with(action, reason, &c.family, &c.key, Some(s)));
            }
        }
    }
    for s in server {
        if !families.contains(&s.family) || held.contains(&(s.family.as_str(), s.key.as_str())) {
            continue;
        }
        if last(&s.family, &s.key).is_some() {
            ops.push(with("skip", "deleted on this device since it last agreed; kept on the server", &s.family, &s.key, Some(s)));
        } else {
            ops.push(with("download", "this device does not have this save", &s.family, &s.key, Some(s)));
        }
    }
    let count = |a: &str| ops.iter().filter(|o| o.action == a).count();
    UnitPlan {
        total_upload: count("upload"),
        total_download: count("download"),
        total_conflict: count("conflict"),
        total_no_op: count("no_op"),
        total_skip: count("skip"),
        operations: ops,
    }
}

/// Derived from the name, like a save id, and as narrow, for the browser.
pub fn unit_id(family: &str, key: &str) -> i64 {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(b"unit\0");
    h.update(family.as_bytes());
    h.update(b"\0");
    h.update(key.as_bytes());
    let d = h.finalize();
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[..8]);
    (i64::from_le_bytes(b) & ((1i64 << 53) - 1)).max(1)
}

pub struct UnitStore {
    root: PathBuf,
}

/// Why a unit could not be stored: the request's fault, or the disk's.
#[derive(Debug)]
pub enum WriteError {
    Refused(String),
    Io(std::io::Error),
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e)
    }
}

fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)
}

impl UnitStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self, family: &str, key: &str) -> PathBuf {
        self.root.join(family).join(key)
    }

    /// Every unit, or one family's.
    pub fn list(&self, family: Option<&str>) -> Vec<ServerUnit> {
        let families: Vec<&str> = match family {
            Some(f) => vec![f],
            None => saveunit::FAMILIES.to_vec(),
        };
        let mut out = Vec::new();
        for f in families {
            let Ok(rd) = std::fs::read_dir(self.root.join(f)) else { continue };
            for e in rd.flatten() {
                let Ok(meta) = std::fs::read(e.path().join("unit.json")) else { continue };
                if let Ok(u) = serde_json::from_slice::<ServerUnit>(&meta) {
                    out.push(u);
                }
            }
        }
        out.sort_by(|a, b| (&a.family, &a.key).cmp(&(&b.family, &b.key)));
        out
    }

    pub fn get(&self, family: &str, key: &str) -> Option<ServerUnit> {
        let meta = std::fs::read(self.dir(family, key).join("unit.json")).ok()?;
        serde_json::from_slice(&meta).ok()
    }

    /// One unit and its current zip, by id.
    pub fn read(&self, id: i64) -> Option<(ServerUnit, Vec<u8>)> {
        let u = self.list(None).into_iter().find(|u| u.id == id)?;
        let bytes = std::fs::read(self.dir(&u.family, &u.key).join("current.zip")).ok()?;
        Some((u, bytes))
    }

    /// Store `zip` as the unit's current copy, keeping the one it replaces.
    ///
    /// The zip is checked whole first, with the same rules the device applies
    /// before unpacking it, so nothing is stored that a device would refuse.
    pub fn write(&self, family: &str, key: &str, zip: &[u8]) -> Result<ServerUnit, WriteError> {
        if !saveunit::valid_family(family) {
            return Err(WriteError::Refused(format!("no such save family: {family}")));
        }
        if !saveunit::valid_key(key) {
            return Err(WriteError::Refused(format!("not a usable unit name: {key:?}")));
        }
        let hash = saveunit::check(zip).map_err(|e| WriteError::Refused(format!("{e:#}")))?;
        let dir = self.dir(family, key);
        let history = dir.join("history");
        std::fs::create_dir_all(&history)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let current = dir.join("current.zip");
        if let Some(old) = self.get(family, key)
            && old.content_hash != hash
            && current.is_file()
        {
            let short: String = old.content_hash.chars().take(8).collect();
            std::fs::rename(&current, history.join(format!("{now:011}-{short}.zip")))?;
            let mut kept: Vec<PathBuf> = std::fs::read_dir(&history)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "zip"))
                .collect();
            kept.sort();
            for gone in kept.iter().rev().skip(HISTORY) {
                let _ = std::fs::remove_file(gone);
            }
        }
        write_atomically(&current, zip)?;
        let unit = ServerUnit {
            id: unit_id(family, key),
            family: family.to_owned(),
            key: key.to_owned(),
            content_hash: hash,
            size: zip.len() as u64,
            updated_at: now.to_string(),
        };
        let meta = serde_json::to_vec_pretty(&unit).map_err(std::io::Error::other)?;
        write_atomically(&dir.join("unit.json"), &meta)?;
        Ok(unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn unit(family: &str, key: &str, hash: &str) -> ServerUnit {
        ServerUnit {
            id: unit_id(family, key),
            family: family.into(),
            key: key.into(),
            content_hash: hash.into(),
            size: 1,
            updated_at: "0".into(),
        }
    }

    fn held(family: &str, key: &str, hash: &str) -> ClientUnit {
        ClientUnit { family: family.into(), key: key.into(), content_hash: hash.into() }
    }

    fn actions(p: &UnitPlan) -> Vec<(&str, &str)> {
        p.operations.iter().map(|o| (o.key.as_str(), o.action.as_str())).collect()
    }

    #[test]
    fn units_follow_the_save_rules() {
        let fam = vec!["ppsspp".to_string()];
        let server = [unit("ppsspp", "SAME", "a"), unit("ppsspp", "MINE", "a"), unit("ppsspp", "THEIRS", "b"), unit("ppsspp", "BOTH", "b")];
        let client = [held("ppsspp", "SAME", "a"), held("ppsspp", "MINE", "z"), held("ppsspp", "THEIRS", "a"), held("ppsspp", "BOTH", "z"), held("ppsspp", "NEW", "n")];
        let last = |_: &str, k: &str| match k {
            "MINE" | "THEIRS" | "BOTH" => Some("a".to_string()),
            _ => None,
        };
        let p = plan(&fam, &client, &server, &last);
        assert_eq!(
            actions(&p),
            [("SAME", "no_op"), ("MINE", "upload"), ("THEIRS", "download"), ("BOTH", "conflict"), ("NEW", "upload")]
        );
    }

    #[test]
    fn a_device_hears_only_about_the_families_it_asked_for() {
        let server = [unit("pcsx2", "BASLUS-20035", "a"), unit("ppsspp", "UCUS98653DATA00", "b")];
        let p = plan(&["ppsspp".to_string()], &[], &server, &|_, _| None);
        assert_eq!(actions(&p), [("UCUS98653DATA00", "download")]);
    }

    #[test]
    fn a_unit_deleted_here_is_not_brought_back() {
        let server = [unit("ppsspp", "UCUS98653DATA00", "b")];
        let p = plan(&["ppsspp".to_string()], &[], &server, &|_, _| Some("b".into()));
        assert_eq!(actions(&p), [("UCUS98653DATA00", "skip")]);
    }

    #[test]
    fn a_replaced_unit_goes_to_history_and_history_is_bounded() {
        let d = tempdir::TempDir::new("units").unwrap();
        let store = UnitStore::new(d.path());
        let first = store.write("ppsspp", "UCUS98653DATA00", &zip_of(&[("DATA.BIN", b"0")])).unwrap();
        // Writing the same bytes again keeps no copy: nothing was replaced.
        store.write("ppsspp", "UCUS98653DATA00", &zip_of(&[("DATA.BIN", b"0")])).unwrap();
        let history = d.path().join("ppsspp/UCUS98653DATA00/history");
        assert_eq!(std::fs::read_dir(&history).unwrap().count(), 0);

        for i in 1..=(HISTORY + 3) {
            std::thread::sleep(std::time::Duration::from_millis(2));
            let z = zip_of(&[("DATA.BIN", i.to_string().as_bytes())]);
            // Distinct names even within one second: the hash is in them.
            store.write("ppsspp", "UCUS98653DATA00", &z).unwrap();
        }
        assert!(std::fs::read_dir(&history).unwrap().count() <= HISTORY);
        let now = store.get("ppsspp", "UCUS98653DATA00").unwrap();
        assert_ne!(now.content_hash, first.content_hash);
        let (u, bytes) = store.read(now.id).unwrap();
        assert_eq!(u, now);
        assert_eq!(saveunit::check(&bytes).unwrap(), now.content_hash);
    }

    #[test]
    fn nothing_unusable_is_stored() {
        let d = tempdir::TempDir::new("units-bad").unwrap();
        let store = UnitStore::new(d.path());
        let ok = zip_of(&[("DATA.BIN", b"0")]);
        assert!(matches!(store.write("dolphin", "GALE01", &ok), Err(WriteError::Refused(_))));
        assert!(matches!(store.write("ppsspp", "../x", &ok), Err(WriteError::Refused(_))));
        assert!(matches!(store.write("ppsspp", "K", b"not a zip"), Err(WriteError::Refused(_))));
        assert!(matches!(store.write("ppsspp", "K", &zip_of(&[("../x", b"1")])), Err(WriteError::Refused(_))));
        assert!(store.list(None).is_empty());
    }
}

//! Launch records: what ran, so a launch can be audited and repeated. See
//! `docs/server-and-client.md`, "Launch records".
//!
//! A record holds three versions, and the same three mean the same launch:
//!
//! - code: the Moose Rack build (version and git commit, `+dirty` when built
//!   from uncommitted changes);
//! - config: a hash over the settings the app owns for this game's emulator,
//!   with this computer's paths left out;
//! - data: hashes of the game file, the emulator build and the BIOS.
//!
//! Plus the machine, for context. Each record is written to
//! `<library>/launches/<id>.json` when the game starts, finished when it ends,
//! and uploaded to the server; one written offline waits there and goes up
//! with the next launch that can reach the server. Uploaded ones move to
//! `launches/sent/`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    /// The server's id for this device, when it has one.
    pub device_id: Option<String>,
    pub device_name: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub game: Game,
    pub code: Code,
    pub config: ConfigVersion,
    pub data: Data,
    pub machine: Machine,
    /// Played without checking saves with the server.
    pub offline: bool,
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub platform: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Code {
    pub app: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigVersion {
    /// md5 over the app-owned settings, as sorted `key = value` lines.
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Data {
    pub game_md5: Option<String>,
    /// "PCSX2 v2.8.2", or "RetroArch, gambatte".
    pub emulator: String,
    /// md5 of the emulator program, or of the RetroArch core.
    pub emulator_md5: Option<String>,
    pub bios: Vec<Bios>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bios {
    pub name: String,
    pub md5: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub chip: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub exit: String,
    pub seconds: i64,
    /// What the save sync after the game said, when it said anything.
    pub saves: Option<String>,
}

/// This build.
pub fn code() -> Code {
    Code { app: env!("CARGO_PKG_VERSION").to_owned(), commit: env!("MOOSE_COMMIT").to_owned() }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// This computer, as far as a record needs it.
pub fn machine() -> Machine {
    let (os_version, chip) = match std::env::consts::OS {
        "macos" => (
            run("sw_vers", &["-productVersion"]),
            run("sysctl", &["-n", "machdep.cpu.brand_string"]),
        ),
        "windows" => (
            run("cmd", &["/C", "ver"]),
            std::env::var("PROCESSOR_IDENTIFIER").ok(),
        ),
        _ => (run("uname", &["-r"]), None),
    };
    Machine {
        os: std::env::consts::OS.to_owned(),
        os_version: os_version.unwrap_or_default(),
        arch: std::env::consts::ARCH.to_owned(),
        chip: chip.unwrap_or_default(),
    }
}

/// md5 of a file, or `None` when it can't be read.
pub fn md5_of(path: &Path) -> Option<String> {
    crate::download::hash_file(path).ok().map(|(md5, _)| md5)
}

/// md5 over `key = value` lines, sorted, so the order settings were written in
/// doesn't change the version.
pub fn config_hash(lines: &[String]) -> String {
    use md5::Digest as _;
    let mut sorted = lines.to_vec();
    sorted.sort();
    hex::encode(md5::Md5::digest(sorted.join("\n").as_bytes()))
}

/// A new id: when it started, plus a few characters so two launches in the
/// same second don't collide.
pub fn new_id(started_at: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let salt = nanos ^ std::process::id().rotate_left(16);
    let stamp: String = started_at.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("{stamp}-{:06x}", salt & 0xff_ffff)
}

pub fn dir(library_root: &Path) -> PathBuf {
    library_root.join("launches")
}

/// Write the record where it waits for upload. Replaces an earlier copy of the
/// same id, which is how the closing half lands on the opening half.
pub fn save(library_root: &Path, rec: &Record) -> anyhow::Result<PathBuf> {
    let dir = dir(library_root);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", rec.id));
    let part = path.with_extension("json.part");
    std::fs::write(&part, serde_json::to_vec_pretty(rec)?)?;
    std::fs::rename(&part, &path)?;
    Ok(path)
}

/// Records written here and not uploaded yet, finished ones only: a record
/// still open belongs to a game that is running.
pub fn pending(library_root: &Path) -> Vec<(PathBuf, Record)> {
    let Ok(rd) = std::fs::read_dir(dir(library_root)) else { return Vec::new() };
    let mut out: Vec<(PathBuf, Record)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let rec: Record = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
            rec.outcome.is_some().then_some((p, rec))
        })
        .collect();
    out.sort_by(|a, b| a.1.started_at.cmp(&b.1.started_at));
    out
}

/// Upload every finished record waiting here. Returns how many went up.
/// Stops at the first failure: the server being away is the usual reason, and
/// the rest wait for the next try.
pub async fn upload_pending(client: &crate::api::Client, library_root: &Path) -> usize {
    let sent = dir(library_root).join("sent");
    let mut n = 0;
    for (path, rec) in pending(library_root) {
        if client.post_launch(&rec).await.is_err() {
            break;
        }
        std::fs::create_dir_all(&sent).ok();
        if let Some(name) = path.file_name() {
            std::fs::rename(&path, sent.join(name)).ok();
        }
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, finished: bool) -> Record {
        Record {
            id: id.to_owned(),
            device_id: Some("dev-1".into()),
            device_name: "macserver".into(),
            started_at: "2026-11-02T03:00:00".into(),
            ended_at: finished.then(|| "2026-11-02T03:30:00".into()),
            game: Game { platform: "ps2".into(), file: "Crazy Taxi (USA).chd".into() },
            code: code(),
            config: ConfigVersion { hash: config_hash(&["UI.ConfirmShutdown = false".into()]) },
            data: Data { game_md5: Some("abc".into()), emulator: "PCSX2 v2.8.2".into(), emulator_md5: None, bios: vec![] },
            machine: machine(),
            offline: false,
            outcome: finished.then(|| Outcome { exit: "exit status: 0".into(), seconds: 1800, saves: None }),
        }
    }

    #[test]
    fn the_config_hash_ignores_the_order_settings_came_in() {
        let a = config_hash(&["b = 2".into(), "a = 1".into()]);
        let b = config_hash(&["a = 1".into(), "b = 2".into()]);
        assert_eq!(a, b);
        assert_ne!(a, config_hash(&["a = 1".into(), "b = 3".into()]));
    }

    #[test]
    fn only_finished_records_wait_for_upload() {
        let lib = std::env::temp_dir().join("moose-launchrecord");
        let _ = std::fs::remove_dir_all(&lib);
        save(&lib, &record("open-one", false)).unwrap();
        save(&lib, &record("done-one", true)).unwrap();
        let waiting: Vec<String> = pending(&lib).into_iter().map(|(_, r)| r.id).collect();
        assert_eq!(waiting, ["done-one"]);
        // Closing the open one replaces it in place.
        save(&lib, &record("open-one", true)).unwrap();
        assert_eq!(pending(&lib).len(), 2);
    }

    #[test]
    fn a_record_reads_back_as_written() {
        let rec = record("x", true);
        let json = serde_json::to_string(&rec).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), rec);
        assert!(!rec.code.commit.is_empty());
    }

    #[test]
    fn ids_sort_by_time_and_stay_plain() {
        let id = new_id("2026-11-02T03:00:00");
        assert!(id.starts_with("20261102T030000-"), "{id}");
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}

//! Save synchronisation: the glue between [`crate::saves`] and [`crate::api`].
//!
//! Both ends were already written and neither was ever called, so save sync has
//! never run. This module is the missing middle: scan the RetroArch save tree,
//! tell the server what we have, and carry out whatever it asks for.
//!
//! Runs automatically either side of a launch — pull before, push after — and
//! on demand through `sync-saves`. `[saves] auto_sync` turns the automatic half
//! off.
//!
//! This was deliberately manual for a long time, on the grounds that a save is
//! the one thing here that cannot be re-downloaded if it goes wrong. That was
//! really an argument against automatic sync *without a way back*: every
//! overwrite now copies the previous file into a rotating backup first (see
//! [`crate::savebackup`]), so a bad outcome is recoverable rather than final.
//!
//! A conflict is never resolved silently. When both sides changed since the
//! last sync, nothing is written and the launch is refused until the user says
//! which copy to keep — playing on top of a save whose ownership is unsettled
//! is how the loser gets overwritten for good on the way back out.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::{Client, ClientSaveState};
use crate::cache::Cache;
use crate::coremap::CoreMap;
use crate::platform::SaveLayout;
use crate::saves::{self, Candidate, Resolution};

/// This machine's identity with the server, persisted beside the cache.
///
/// The server issues the id; we only remember it. Without that the server sees
/// a new device on every run and cannot tell "this machine already has that
/// save" from "another machine is uploading a different one".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub device_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

const STATE_FILE: &str = "state.json";

impl DeviceIdentity {
    fn load(dir: &Path) -> Option<Self> {
        let raw = std::fs::read_to_string(dir.join(STATE_FILE)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(STATE_FILE);
        let body = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
    }

    /// The stored identity, registering with the server the first time.
    pub async fn ensure(client: &Client, dir: &Path) -> Result<Self> {
        if let Some(existing) = Self::load(dir) {
            return Ok(existing);
        }
        let host = hostname();
        let device = client
            .register_device(&host, &host)
            .await
            .context("registering this device with the server")?;
        let identity = Self { device_id: device.id, name: device.name.or(Some(host)) };
        identity.save(dir)?;
        Ok(identity)
    }
}

/// Best-effort machine name, for the device list on the server.
fn hostname() -> String {
    for key in ["HOSTNAME", "COMPUTERNAME", "NAME"] {
        if let Some(v) = std::env::var_os(key)
            && !v.is_empty()
        {
            return v.to_string_lossy().into_owned();
        }
    }
    // gethostname without a dependency: the file exists on Linux, and the two
    // environment variables above cover macOS and Windows.
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// What a sync did, for reporting. Nothing here is a failure on its own —
/// conflicts and skips are normal and worth showing rather than burying.
#[derive(Debug, Default, Serialize)]
pub struct Summary {
    pub uploaded: usize,
    pub downloaded: usize,
    /// Saves changed on both sides since the last sync. Nothing is written for
    /// these; they carry enough detail for the user to choose.
    pub conflicts: Vec<SaveConflict>,
    pub unchanged: usize,
    /// Local files the server was not told about, and why.
    pub skipped: usize,
    /// Transfers that were attempted and did not happen. Counted, not just
    /// noted: a run where every transfer failed used to read "Saves already in
    /// sync", because the headline only looked at what had moved.
    pub failed: usize,
    /// One line each, in the order they happened.
    pub notes: Vec<String>,
    /// The failures among `notes`, on their own, to be put in front of the
    /// person rather than left in a log.
    pub problems: Vec<String>,
}

impl Summary {
    /// Something that should have moved and did not.
    pub fn fail(&mut self, what: String) {
        self.failed += 1;
        self.notes.push(what.clone());
        self.problems.push(what);
    }
}

/// A save that changed on both sides, described well enough to choose between.
#[derive(Debug, Clone, Serialize)]
pub struct SaveConflict {
    pub rom_id: i64,
    pub save_id: Option<i64>,
    pub file_name: String,
    pub slot: Option<String>,
    pub emulator: Option<String>,
    /// The server's own words for why it refused to pick.
    pub reason: Option<String>,
    /// Absolute path of the local copy, when one was found.
    pub local_path: Option<PathBuf>,
    pub local_updated: Option<String>,
    pub local_bytes: i64,
    pub server_updated: Option<String>,
}

/// Which copy to keep. There is no third option that keeps both: they are the
/// same `(rom, slot)` as far as the server is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Keep {
    /// Upload this machine's copy over the server's.
    Local,
    /// Download the server's copy over this machine's.
    Server,
}

impl Summary {
    pub fn headline(&self) -> String {
        if self.uploaded + self.downloaded + self.conflicts.len() + self.failed == 0 {
            return format!("Saves already in sync ({} checked)", self.unchanged);
        }
        let mut parts = Vec::new();
        if self.uploaded > 0 {
            parts.push(format!("{} uploaded", self.uploaded));
        }
        if self.downloaded > 0 {
            parts.push(format!("{} downloaded", self.downloaded));
        }
        if !self.conflicts.is_empty() {
            parts.push(format!("{} in conflict", self.conflicts.len()));
        }
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        parts.join(", ")
    }
}

/// Turn scanner output into what the server wants to hear about.
///
/// Only canonical, ROM-matched candidates are sent. An ambiguous or unmatched
/// save has no `rom_id` to pair on, and a non-canonical one would fight the
/// file that beat it for the same `(rom_id, slot)` on every run.
pub fn client_states(candidates: &[Candidate]) -> (Vec<ClientSaveState>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0;

    for c in candidates {
        // Save states go to /api/states, a different family of endpoints with
        // no slot and no negotiation. Sending them here filed a freeze-frame
        // snapshot on the server as if it were an in-game save. See
        // crate::statesync, which handles them.
        if c.kind == saves::Kind::State {
            continue;
        }
        let Resolution::Resolved { rom_id, .. } = c.resolution else {
            skipped += 1;
            continue;
        };
        if !c.canonical {
            skipped += 1;
            continue;
        }
        out.push(ClientSaveState {
            rom_id,
            file_name: c
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            slot: Some(c.slot.clone()),
            emulator: c.core.clone().or_else(|| Some(c.core_dir.clone())),
            content_hash: c.content_hash.clone(),
            updated_at: modified_rfc3339(&c.path),
            file_size_bytes: c.size as i64,
        });
    }
    (out, skipped)
}

/// Where a save the server sent us should land.
#[derive(Debug, Clone)]
pub enum Destination<'a> {
    /// RetroArch's own layout: `saves/<core folder>/…`, states in `states/`.
    /// Both spellings are offered because neither is reliable on its own —
    /// see [`download_path`].
    Core { core: Option<&'a str> },
    /// Batocera and KNULLI: `saves/<platform>/…`, with states in that same
    /// folder rather than a sibling `states/`. Owned, because the folder is
    /// not always the server's slug — see `Platform::save_folder`.
    System(String),
}

/// The folder RetroArch itself reads, which is not the name the server uses
/// and not reliably the name in the core map either.
///
/// RetroArch names the folder after the core's own `corename`. The server
/// stores the core's *stem* — `pcsx_rearmed` — and the shipped map carries
/// ES-DE's label, which is `PCSX ReARMed` where the real folder is
/// `PCSX-ReARMed`, and `Snes9x - Current` where it is `Snes9x`. Writing
/// either of those makes a folder the emulator never looks in, and the save
/// is silently ignored while our own scanner reports it present.
///
/// So the folder that is already there wins. `normalize` ignores case and
/// punctuation, which is exactly the difference between all three spellings.
fn core_folder(dir: &Path, core: Option<&str>) -> Option<String> {
    let core = core.filter(|s| !s.is_empty())?;
    let wanted = saves::normalize(core);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if saves::normalize(&name) == wanted {
                return Some(name);
            }
        }
    }
    // Nothing there yet, so the server's own spelling it is. RetroArch will
    // read from it once it writes a save of its own there.
    Some(core.to_string())
}

/// Where this device would file a save for one platform, given the core the
/// server recorded.
///
/// The core is what RetroArch needs and the platform is what Batocera needs;
/// which one is used depends on the device, not on the save.
pub fn destination<'a>(core: Option<&'a str>, platform: Option<&'a str>) -> Destination<'a> {
    let device = crate::platform::current();
    match (device.save_layout(), platform) {
        // Through the device's own naming: RomM keeps `sfam` apart from
        // `snes` and this handheld has only `snes`, so the server's slug is
        // not always a folder anything reads.
        (SaveLayout::BySystem, Some(slug)) => Destination::System(device.save_folder(slug)),
        _ => Destination::Core { core },
    }
}

/// The name the *emulator* will look for, given the name the server stores.
///
/// RomM stamps every save it keeps: `Chrono Trigger (USA).srm` is filed as
/// `Chrono Trigger (USA) [2026-08-06_23-06-01].srm`, which is how several
/// versions of one save coexist under one `(rom, slot)`. That stamp is the
/// server's bookkeeping and means nothing to RetroArch, which derives a save
/// name from the content it loaded — so a download that keeps it lands a file
/// no emulator will ever open, and which our own scanner cannot match back to
/// a ROM either.
///
/// Only the stamp is removed. Game names are full of brackets — `[!]`,
/// `[T-En by ...]` — so the shape is matched exactly rather than "the last
/// bracketed thing".
pub fn local_name(server_name: &str) -> String {
    let Some(open) = server_name.rfind(" [") else {
        return server_name.to_string();
    };
    let rest = &server_name[open + 2..];
    let Some(close) = rest.find(']') else {
        return server_name.to_string();
    };
    if !is_stamp(&rest[..close]) {
        return server_name.to_string();
    }
    format!("{}{}", &server_name[..open], &rest[close + 1..])
}

/// `2026-08-06_23-06-01`, and nothing else.
fn is_stamp(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    bytes.iter().enumerate().all(|(i, b)| match i {
        4 | 7 => *b == b'-',
        10 => *b == b'_',
        13 | 16 => *b == b'-',
        _ => b.is_ascii_digit(),
    })
}

/// Mirrors the device's own layout, which is what the scanner reads back.
/// Without it a download lands somewhere the emulator never looks.
pub fn download_path(root: &Path, file_name: &str, dest: Destination<'_>) -> PathBuf {
    let file_name = &local_name(file_name);
    match dest {
        // One folder per system, states included — configgen points both
        // `savefile_directory` and `savestate_directory` at it.
        Destination::System(slug) => root.join("saves").join(&slug).join(file_name),
        Destination::Core { core } => {
            let kind = if saves::is_state_name(file_name) { "states" } else { "saves" };
            let dir = root.join(kind);
            match core_folder(&dir, core) {
                Some(folder) => dir.join(folder).join(file_name),
                None => dir.join(file_name),
            }
        }
    }
}

/// Scan the save tree. Split from [`run`] so a caller holding a lock on the
/// cache can drop it before any awaiting starts — the SQLite connection is not
/// `Sync`, so a future holding one across an await cannot be spawned.
pub fn scan(cache: &Cache, map: &CoreMap, ra_root: &Path) -> Result<Vec<Candidate>> {
    saves::scan(ra_root, cache, map).context("scanning the save tree")
}

/// As [`scan`], for one game only — what the automatic sync either side of a
/// launch uses.
///
/// `fs_name` is the ROM's filename; emulators derive a save name from its stem,
/// which is what the scanner matches on.
pub fn scan_for_rom(
    cache: &Cache,
    map: &CoreMap,
    ra_root: &Path,
    fs_name: &str,
) -> Result<Vec<Candidate>> {
    let stem = Path::new(fs_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(fs_name);
    saves::scan_for_stem(ra_root, cache, map, stem)
        .with_context(|| format!("scanning saves for {stem}"))
}

/// When the automatic sync runs.
///
/// Both halves are separate because they fail differently. A pull that cannot
/// reach the server should not stop you playing offline; a push that fails has
/// progress sitting on the machine that still needs to leave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Before the emulator starts: take whatever the server has that is newer.
    BeforeLaunch,
    /// After it exits: send whatever changed while playing.
    AfterExit,
}

impl When {
    pub fn label(self) -> &'static str {
        match self {
            Self::BeforeLaunch => "pulled before launch",
            Self::AfterExit => "pushed after exit",
        }
    }
}

/// Report of one automatic half-sync, for the launch notes.
///
/// Never an error to the caller: a sync problem must not stop a game starting
/// or be the last thing that happens after one ends. Anything that went wrong
/// is in `notes` and shown, which is the difference between "sync is broken"
/// and "nothing happened and nobody said why".
pub fn describe(when: When, summary: &Summary) -> Option<String> {
    let moved = match when {
        When::BeforeLaunch => summary.downloaded,
        When::AfterExit => summary.uploaded,
    };
    if moved == 0 && summary.conflicts.is_empty() {
        return None;
    }
    let mut line = format!("saves: {moved} {}", when.label());
    if !summary.conflicts.is_empty() {
        line.push_str(&format!(
            ", {} in conflict and left alone — run `sync-saves` to look",
            summary.conflicts.len()
        ));
    }
    Some(line)
}

/// One save on the server, and a file here that the filesystem treats as the
/// same file although its name is spelled with different case.
///
/// The server compares names exactly, because its disk does. A card formatted
/// exFAT, APFS in its default form, NTFS and an Android SD card do not: there
/// `Kirby & the Amazing Mirror (USA).srm` and `Kirby & The Amazing Mirror
/// (USA).srm` are one file. So the server offered its lowercase copy as a
/// download "this device does not have", the download landed on the Flip's own
/// newer save and replaced it, and nothing called it a conflict because by the
/// server's reckoning the two had never met. It then re-uploaded under the
/// other spelling, which left both on the server and a pull on every sync
/// after that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Twin {
    /// Same bytes under both spellings. Nothing to move; the local file takes
    /// the server's spelling so both sides name it the same from now on.
    Same { local: PathBuf, server_name: String },
    /// Different bytes. A conflict: neither side is allowed to replace the
    /// other without somebody choosing.
    Differ { candidate: usize },
}

/// Whether a file named `other_name` beside `existing` would be `existing`.
///
/// True only on a filesystem that folds case: the other spelling resolves, yet
/// no directory entry carries it. On a case-sensitive disk the lookup fails,
/// or finds a genuinely separate file with an entry of its own, and the answer
/// is false -- which is what keeps this inert on ext4.
pub fn occupies(existing: &Path, other_name: &str) -> bool {
    if existing.file_name().is_some_and(|n| n == other_name) {
        return false;
    }
    if !existing.with_file_name(other_name).exists() {
        return false;
    }
    let Some(dir) = existing.parent() else {
        return false;
    };
    std::fs::read_dir(dir)
        .map(|entries| !entries.flatten().any(|e| e.file_name() == other_name))
        .unwrap_or(false)
}

/// Every download in `plan` that would land on a differently spelled local
/// save, keyed by its index in `plan`.
///
/// `folds` answers [`occupies`] and is a parameter so the decision can be
/// tested on any disk. Only saves resolved to the same game are considered, so
/// two games whose names happen to differ only in case are never paired.
pub fn case_twins(
    plan: &[crate::api::SyncOperation],
    candidates: &[Candidate],
    folds: &dyn Fn(&Path, &str) -> bool,
) -> std::collections::HashMap<usize, Twin> {
    let mut out = std::collections::HashMap::new();
    for (i, op) in plan.iter().enumerate() {
        if op.action != "download" {
            continue;
        }
        let Some(server_name) = op.file_name.as_deref().map(local_name) else {
            continue;
        };
        let folded = server_name.to_lowercase();
        let found = candidates.iter().position(|c| {
            c.kind == saves::Kind::Save
                && matches!(c.resolution, Resolution::Resolved { rom_id, .. } if rom_id == op.rom_id)
                && c.path.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy();
                    n != server_name && n.to_lowercase() == folded
                })
                && folds(&c.path, &server_name)
        });
        let Some(at) = found else {
            continue;
        };
        let same = op.server_content_hash.as_deref() == Some(candidates[at].content_hash.as_str());
        out.insert(
            i,
            if same {
                Twin::Same { local: candidates[at].path.clone(), server_name }
            } else {
                Twin::Differ { candidate: at }
            },
        );
    }
    out
}

/// Rename a file to a name that differs only in case.
///
/// Through a temporary name, because on a filesystem that folds case a direct
/// rename finds the target already there -- it is the same file -- and
/// Linux's `rename` then does nothing and reports success.
pub fn respell(path: &Path, new_name: &str) -> Result<PathBuf> {
    let target = path.with_file_name(new_name);
    let step = path.with_file_name(format!(".{new_name}.respelling"));
    std::fs::rename(path, &step)
        .with_context(|| format!("renaming {} aside", path.display()))?;
    std::fs::rename(&step, &target)
        .with_context(|| format!("renaming it to {new_name}"))?;
    Ok(target)
}

/// The local save the server means by `(rom_id, name)`.
///
/// By game as well as name. The same file name exists under more than one
/// system -- 374 of them on the server -- and matching on the name alone sent
/// whichever sorted first: a Game Boy Color upload could carry the Game Boy
/// Advance file's bytes, on every sync.
fn local_for<'a>(candidates: &'a [Candidate], rom_id: i64, name: &str) -> Option<&'a Candidate> {
    candidates.iter().find(|c| {
        c.kind == saves::Kind::Save
            && matches!(c.resolution, Resolution::Resolved { rom_id: r, .. } if r == rom_id)
            && c.path.file_name().is_some_and(|n| n.to_string_lossy() == name)
    })
}

/// Negotiate and carry out the plan for an already-scanned tree.
pub async fn run(
    client: &Client,
    candidates: &[Candidate],
    ra_root: &Path,
    data_dir: &Path,
    library_root: &Path,
) -> Result<Summary> {
    let identity = DeviceIdentity::ensure(client, data_dir).await?;
    let mut summary = Summary::default();

    let (states, skipped) = client_states(candidates);
    summary.skipped = skipped;
    if skipped > 0 {
        summary
            .notes
            .push(format!("{skipped} local file(s) skipped: no matching ROM, or superseded"));
    }

    let plan = client
        .negotiate(&identity.device_id, &states)
        .await
        .context("asking the server what to sync")?;

    // What this device told the server it holds. A download of anything else
    // is, by the server's reckoning, a file the device lacks -- which is only
    // true if nothing is at the target. See `Landed::Occupied`.
    let reported: std::collections::HashSet<(i64, String)> =
        states.iter().map(|s| (s.rom_id, s.file_name.clone())).collect();

    // Downloads that would land on a local save spelled differently. The
    // upload the server asks for alongside each one is the same save seen from
    // this side, so it is skipped too: the pair is settled once, below.
    let twins = case_twins(&plan.operations, candidates, &occupies);
    let paired: std::collections::HashSet<PathBuf> = twins
        .values()
        .map(|t| match t {
            Twin::Same { local, .. } => local.clone(),
            Twin::Differ { candidate } => candidates[*candidate].path.clone(),
        })
        .collect();

    for (at, op) in plan.operations.iter().enumerate() {
        if let Some(twin) = twins.get(&at) {
            match twin {
                Twin::Same { local, server_name } => match respell(local, server_name) {
                    Ok(_) => {
                        summary.unchanged += 1;
                        summary.notes.push(format!("{server_name}: already here, spelled differently; renamed to match"));
                    }
                    Err(e) => summary.notes.push(format!("{server_name}: could not rename the local copy: {e:#}")),
                },
                Twin::Differ { candidate } => {
                    let c = &candidates[*candidate];
                    let here = c.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let name = op.file_name.clone().unwrap_or_default();
                    summary.conflicts.push(SaveConflict {
                        rom_id: op.rom_id,
                        save_id: op.save_id,
                        slot: op.slot.clone().or_else(|| Some(c.slot.clone())),
                        emulator: op.emulator.clone().or_else(|| c.core.clone()),
                        reason: Some(format!(
                            "this device keeps it as {here} and the server as {name}, and they differ"
                        )),
                        local_updated: Some(modified_rfc3339(&c.path)),
                        local_bytes: c.size as i64,
                        local_path: Some(c.path.clone()),
                        server_updated: op.server_updated_at.clone(),
                        file_name: name,
                    });
                }
            }
            continue;
        }
        match op.action.as_str() {
            "download" => {
                let Some(save_id) = op.save_id else {
                    summary.notes.push("server asked for a download with no save id".to_owned());
                    continue;
                };
                let name = op.file_name.clone().unwrap_or_else(|| format!("save-{save_id}"));
                let was_reported = reported.contains(&(op.rom_id, local_name(&name)));
                match download_one(
                    client,
                    ra_root,
                    save_id,
                    &name,
                    op.emulator.as_deref(),
                    &identity,
                    library_root,
                    op.rom_id,
                    op.slot.as_deref(),
                    Guard::Unreported { reported: was_reported, server_hash: op.server_content_hash.as_deref() },
                )
                .await
                {
                    Ok(Landed::Written { path, note }) => {
                        summary.downloaded += 1;
                        summary.notes.push(format!("downloaded {}", path.display()));
                        summary.notes.extend(note);
                    }
                    Ok(Landed::Occupied { path, same: true }) => {
                        summary.unchanged += 1;
                        summary.notes.push(format!(
                            "{name}: already here with the server's bytes, left as it is ({})",
                            path.display()
                        ));
                    }
                    Ok(Landed::Occupied { path, same: false }) => {
                        summary.conflicts.push(SaveConflict {
                            rom_id: op.rom_id,
                            save_id: op.save_id,
                            slot: op.slot.clone(),
                            emulator: op.emulator.clone(),
                            reason: Some(
                                "a save is already here that this device never reported, and it \
                                 differs from the server's"
                                    .to_owned(),
                            ),
                            local_updated: Some(modified_rfc3339(&path)),
                            local_bytes: std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0),
                            local_path: Some(path),
                            server_updated: op.server_updated_at.clone(),
                            file_name: name,
                        });
                    }
                    Err(e) => {
                                                summary.fail(format!("could not download {name}: {e:#}"));
                    }
                }
            }
            "upload" => {
                let name = op.file_name.clone().unwrap_or_default();
                let Some(c) = local_for(candidates, op.rom_id, &name) else {
                                        summary.fail(format!("server asked to upload {name}, which is not here"));
                    continue;
                };
                if paired.contains(&c.path) {
                    continue;
                }
                match upload_one(client, c, op.rom_id, &identity, plan.session_id, false).await {
                    Ok(true) => {
                        summary.uploaded += 1;
                        summary.notes.push(format!("uploaded {name}"));
                    }
                    // A 409 is the server declining because its copy moved on
                    // since we last synced — the same situation `negotiate`
                    // calls a conflict, discovered a step later. Described the
                    // same way so it is resolvable rather than just reported.
                    Ok(false) => {
                        summary.conflicts.push(SaveConflict {
                            rom_id: op.rom_id,
                            save_id: op.save_id,
                            slot: op.slot.clone().or_else(|| Some(c.slot.clone())),
                            emulator: op.emulator.clone().or_else(|| c.core.clone()),
                            reason: Some("the server's copy changed since this device last synced".to_owned()),
                            local_updated: Some(modified_rfc3339(&c.path)),
                            local_bytes: c.size as i64,
                            local_path: Some(c.path.clone()),
                            server_updated: op.server_updated_at.clone(),
                            file_name: name.clone(),
                        });
                        summary.notes.push(format!("{name}: server copy moved on, left alone"));
                    }
                    Err(e) => {
                                                summary.fail(format!("could not upload {name}: {e:#}"));
                    }
                }
            }
            "conflict" => {
                let name = op.file_name.clone().unwrap_or_default();
                let local = local_for(candidates, op.rom_id, &name);
                summary.conflicts.push(SaveConflict {
                    rom_id: op.rom_id,
                    save_id: op.save_id,
                    slot: op.slot.clone().or_else(|| local.map(|c| c.slot.clone())),
                    emulator: op.emulator.clone().or_else(|| local.and_then(|c| c.core.clone())),
                    reason: op.reason.clone(),
                    local_updated: local.map(|c| modified_rfc3339(&c.path)),
                    local_bytes: local.map(|c| c.size as i64).unwrap_or(0),
                    local_path: local.map(|c| c.path.clone()),
                    server_updated: op.server_updated_at.clone(),
                    file_name: name,
                });
            }
            "no_op" => summary.unchanged += 1,
            other => {
                                summary.fail(format!("unknown action {other:?} from the server"));
            }
        }
    }

    // Closing the session is what lets the server release its locks; a failure
    // here has not lost anything, so it is reported rather than propagated.
    if let Some(id) = plan.session_id
        && let Err(e) = client.complete_session(id).await
    {
        summary.notes.push(format!("could not close the sync session: {e}"));
    }

    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
/// Carry out a decision about one conflict.
///
/// The only place `overwrite=true` is ever sent, and only for a conflict the
/// user has been shown and answered. Both directions back up the file they
/// replace first, so choosing wrong is recoverable by copying a file back
/// rather than being the end of that save.
pub async fn resolve(
    client: &Client,
    conflict: &SaveConflict,
    keep: Keep,
    ra_root: &Path,
    library_root: &Path,
    data_dir: &Path,
) -> Result<String> {
    // A save state goes through the states endpoints. Resolving one here would
    // post a freeze-frame to /api/saves, which is exactly the bug statesync
    // exists to fix.
    if saves::is_state_name(&conflict.file_name) {
        return crate::statesync::resolve_one(
            client, conflict, keep, ra_root, library_root, data_dir,
        )
        .await;
    }

    let identity = DeviceIdentity::ensure(client, data_dir).await?;
    let slot = conflict.slot.as_deref().unwrap_or("unslotted");

    match keep {
        Keep::Local => {
            let path = conflict
                .local_path
                .clone()
                .context("no local copy to keep — nothing to upload")?;
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let result = client
                .upload_save(
                    conflict.rom_id,
                    &conflict.file_name,
                    bytes,
                    conflict.slot.as_deref(),
                    conflict.emulator.as_deref(),
                    &identity.device_id,
                    None,
                    true,
                )
                .await?;
            match result {
                Ok(_) => {
                    // A conflict between two spellings of one file: now the
                    // bytes agree, make the names agree too, or the next sync
                    // pairs them up all over again.
                    let server_name = local_name(&conflict.file_name);
                    if path.file_name().is_some_and(|n| n.to_string_lossy() != server_name)
                        && occupies(&path, &server_name)
                    {
                        respell(&path, &server_name)?;
                    }
                    Ok(format!("{}: kept this machine's copy", conflict.file_name))
                }
                // Refused even with overwrite on: the server moved again
                // between being asked and being answered. Saying so beats
                // reporting a success that did not happen.
                Err(c) => anyhow::bail!(
                    "{}: the server refused the upload ({})",
                    conflict.file_name,
                    c.detail.chars().take(200).collect::<String>()
                ),
            }
        }
        Keep::Server => {
            let save_id = conflict
                .save_id
                .context("the server did not name a save to download")?;
            // Someone chose the server's copy, so overwriting is the point.
            // Written to the local file the conflict named when there is one:
            // that is the file the emulator reads, whatever the server's
            // spelling or folder would work out to.
            let landed = download_one(
                client,
                ra_root,
                save_id,
                &conflict.file_name,
                conflict.emulator.as_deref(),
                &identity,
                library_root,
                conflict.rom_id,
                Some(slot),
                match conflict.local_path.as_deref() {
                    Some(p) => Guard::Into(p),
                    None => Guard::Overwrite,
                },
            )
            .await?;
            match landed {
                Landed::Written { path, note } => {
                    let tail = note.map(|n| format!(" ({n})")).unwrap_or_default();
                    Ok(format!("{}: kept the server's copy{tail}", path.display()))
                }
                Landed::Occupied { path, .. } => {
                    anyhow::bail!("{}: not written", path.display())
                }
            }
        }
    }
}

/// Where a download may write, and what it must check first.
pub enum Guard<'a> {
    /// A planned download. When the device did not report the save, anything
    /// already at the target is a local file the server never saw.
    Unreported { reported: bool, server_hash: Option<&'a str> },
    /// Someone chose the server's copy: write over whatever is there.
    Overwrite,
    /// As `Overwrite`, into this exact file.
    Into(&'a Path),
}

/// What became of one download.
#[derive(Debug)]
pub enum Landed {
    /// Written at `path`. `note` says anything that went wrong afterwards.
    Written { path: PathBuf, note: Option<String> },
    /// Not written: a save the device never reported is already at `path`.
    /// `same` when its bytes are the server's.
    ///
    /// The server offers a save as "this device does not have it" whenever
    /// the device did not report it, and a device does not report a save it
    /// cannot match to a game -- an incomplete game index after a dropped
    /// refresh, a renamed ROM. The download then landed on that save, with a
    /// backup but no conflict. Every game in a system the index was missing
    /// could lose its progress in one sync.
    Occupied { path: PathBuf, same: bool },
}

#[allow(clippy::too_many_arguments)]
async fn download_one(
    client: &Client,
    root: &Path,
    save_id: i64,
    file_name: &str,
    emulator: Option<&str>,
    identity: &DeviceIdentity,
    library_root: &Path,
    rom_id: i64,
    slot: Option<&str>,
    guard: Guard<'_>,
) -> Result<Landed> {
    let path = match guard {
        Guard::Into(p) => p.to_path_buf(),
        _ => {
            // The platform, for a device that files by system.
            //
            // Asked of the server rather than the local cache: the whole point
            // of a download is that there may be nothing local for this game
            // yet, and the cache is behind a mutex that deliberately is not
            // held across an await. `platform_fs_slug`, not `platform_slug`:
            // the first is the library's folder name.
            //
            // A failure is an error, not a fallback. Without the platform the
            // save was written to the top of the saves folder, where nothing
            // reads it, and counted as downloaded.
            let platform = match crate::platform::current().save_layout() {
                SaveLayout::ByCore => None,
                SaveLayout::BySystem => {
                    let rom = client
                        .rom_with_files(rom_id)
                        .await
                        .context("asking the server which system this game is")?;
                    Some(
                        rom.platform_fs_slug
                            .filter(|s| !s.is_empty())
                            .context("the server did not say which system this game is")?,
                    )
                }
            };
            download_path(root, file_name, destination(emulator, platform.as_deref()))
        }
    };

    if let Some(stop) = occupied(&path, &guard)? {
        return Ok(stop);
    }

    let bytes = client.save_content(save_id, &identity.device_id).await?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    // Whatever is being replaced goes into the rotating backup first. This is
    // the only irreversible step in a sync, and it stops being something the
    // user consciously chose the moment syncing runs on its own.
    let slot = slot.unwrap_or("unslotted");
    let mut note = None;
    if let Err(e) = crate::savebackup::keep(library_root, rom_id, slot, &path) {
        // A failed backup must not cost the download, but it must be said.
        note = Some(format!("could not back up {} before overwriting: {e:#}", path.display()));
    }
    write_atomically(&path, &bytes)?;
    // Only after the bytes are safely on disk: telling the server first would
    // have it believe we hold a save we failed to write.
    if let Err(e) = client.confirm_download(save_id, &identity.device_id).await {
        let n = format!(
            "{} is written, but the server did not record it, so the next change here may \
             read as a conflict: {e:#}",
            path.display()
        );
        note = Some(match note {
            Some(first) => format!("{first}; {n}"),
            None => n,
        });
    }
    Ok(Landed::Written { path, note })
}

/// Whether a planned download must stop because a save the device never
/// reported is already where it would land.
pub fn occupied(path: &Path, guard: &Guard<'_>) -> Result<Option<Landed>> {
    let Guard::Unreported { reported: false, server_hash } = guard else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let here = crate::savehash::compute(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(Landed::Occupied { same: *server_hash == Some(here.as_str()), path: path.to_path_buf() }))
}

/// Write through a temporary file in the same folder, then rename.
///
/// A sync stopped part way -- the app quit, the battery died -- used to leave
/// a truncated save where the old one was.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.part"));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Take every save the server holds, over whatever is here.
///
/// For a device being set up again: a card that was wiped, or a new one. The
/// ordinary sync is the wrong tool there, because it only moves what changed
/// since this device last agreed with the server. Everything replaced is
/// backed up first, and a save whose system the server cannot name is
/// reported rather than written where nothing reads it.
pub async fn pull_all(
    client: &Client,
    ra_root: &Path,
    data_dir: &Path,
    library_root: &Path,
) -> Result<Summary> {
    let identity = DeviceIdentity::ensure(client, data_dir).await?;
    let mut summary = Summary::default();
    for save in client.saves(None).await? {
        match download_one(
            client,
            ra_root,
            save.id,
            &save.file_name,
            save.emulator.as_deref(),
            &identity,
            library_root,
            save.rom_id,
            save.slot.as_deref(),
            Guard::Overwrite,
        )
        .await
        {
            Ok(Landed::Written { path, note }) => {
                summary.downloaded += 1;
                summary.notes.push(format!("downloaded {}", path.display()));
                summary.notes.extend(note);
            }
            Ok(Landed::Occupied { .. }) => summary.unchanged += 1,
            Err(e) => summary.fail(format!("could not download {}: {e:#}", save.file_name)),
        }
    }
    match crate::statesync::pull_all(client, ra_root, library_root, data_dir).await {
        Ok(states) => {
            summary.downloaded += states.downloaded;
            summary.failed += states.failed;
            summary.notes.extend(states.notes);
            summary.problems.extend(states.problems);
        }
        Err(e) => summary.fail(format!("save states did not come down: {e:#}")),
    }
    Ok(summary)
}

/// Returns false when the server refused because its copy moved on.
async fn upload_one(
    client: &Client,
    candidate: &Candidate,
    rom_id: i64,
    identity: &DeviceIdentity,
    session_id: Option<i64>,
    overwrite: bool,
) -> Result<bool> {
    let bytes = std::fs::read(&candidate.path)
        .with_context(|| format!("reading {}", candidate.path.display()))?;
    let name = candidate
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let result = client
        .upload_save(
            rom_id,
            &name,
            bytes,
            Some(&candidate.slot),
            candidate.core.as_deref().or(Some(&candidate.core_dir)),
            &identity.device_id,
            session_id,
            overwrite,
        )
        .await?;
    Ok(result.is_ok())
}

/// A file's mtime as RFC 3339 UTC, which is the format the server compares on.
///
/// Done by hand rather than with a date crate: this is the only place the
/// project needs one, and the conversion is a well-known closed form.
fn modified_rfc3339(path: &Path) -> String {
    let secs = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    rfc3339(secs)
}

/// Seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Howard Hinnant's days-from-civil, inverted. Exact for any date this will
/// ever see, and shorter than taking on a date dependency for one call site.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Sync saves and save states together.
///
/// The two go to different endpoints and are compared in completely different
/// ways — saves through the server's own `negotiate`, states by a local ledger
/// because the server offers nothing to negotiate with — but nobody calling
/// this cares about that. One call, one summary.
pub async fn run_all(
    client: &Client,
    candidates: &[Candidate],
    ra_root: &Path,
    data_dir: &Path,
    library_root: &Path,
) -> Result<Summary> {
    let mut summary = run(client, candidates, ra_root, data_dir, library_root).await?;

    // States are best effort against the saves half: failing to sync a
    // freeze-frame should not discard a successful game-save sync.
    match crate::statesync::run(client, candidates, library_root, data_dir).await {
        Ok(states) => {
            summary.uploaded += states.uploaded;
            summary.downloaded += states.downloaded;
            summary.unchanged += states.unchanged;
            summary.failed += states.failed;
            summary.conflicts.extend(states.conflicts);
            summary.notes.extend(states.notes);
            summary.problems.extend(states.problems);
        }
        Err(e) => {
                        summary.fail(format!("save states did not sync: {e:#}"));
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A save that keeps the server's stamp is a save no emulator opens.
    ///
    /// Found by pulling 13 saves onto the device and looking at where they
    /// landed: every one was named `… [2026-08-06_23-06-01].srm`, and
    /// RetroArch derives its save name from the content it loaded.
    #[test]
    fn the_servers_version_stamp_comes_off() {
        assert_eq!(
            local_name("Chrono Trigger (USA) [2026-08-06_23-06-01].srm"),
            "Chrono Trigger (USA).srm"
        );
        assert_eq!(
            local_name("Kirby & the Amazing Mirror (USA) [2026-08-06_23-06-01].srm"),
            "Kirby & the Amazing Mirror (USA).srm"
        );
    }

    /// Game names are full of brackets and none of the others may be touched.
    #[test]
    fn only_the_stamp_comes_off() {
        assert_eq!(
            local_name("Donkey Kong Country (U) (V1.2) [!] [2026-08-15_23-55-09].srm"),
            "Donkey Kong Country (U) (V1.2) [!].srm",
            "the [!] is part of the game's name"
        );
        for untouched in [
            "Zelda [!].srm",
            "Nameless Game The (Japan) (T-En by Nagato and Ryusui) (n).srm",
            "Crash Bandicoot (USA).srm",
            "weird [not-a-stamp].srm",
            "Game [2026-08-06].srm",
            "Game [2026-08-06_23-06-01x].srm",
        ] {
            assert_eq!(local_name(untouched), untouched, "{untouched} was altered");
        }
    }

    #[test]
    fn a_download_strips_the_stamp_wherever_it_lands() {
        assert_eq!(
            download_path(
                Path::new("/userdata"),
                "Chrono Trigger (USA) [2026-08-06_23-06-01].srm",
                Destination::System("snes".into()),
            ),
            Path::new("/userdata/saves/snes/Chrono Trigger (USA).srm")
        );
    }

    /// A pull must land in the folder the *emulator* reads.
    ///
    /// RetroArch names the folder after the core's own `corename`; the server
    /// stores the stem. `pcsx_rearmed` and `PCSX-ReARMed` are the same core
    /// and different directories, and writing the wrong one leaves a save the
    /// emulator never loads while our own scanner reports it present — which
    /// is the worst kind of failure, silent and self-confirming.
    #[test]
    fn a_download_joins_the_core_folder_that_is_already_there() {
        let dir = std::env::temp_dir().join("moose-rack-dl-folder");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("saves/PCSX-ReARMed")).unwrap();

        let path = download_path(
            &dir,
            "Crash Bandicoot (USA).srm",
            Destination::Core { core: Some("pcsx_rearmed") },
        );
        assert_eq!(path, dir.join("saves/PCSX-ReARMed/Crash Bandicoot (USA).srm"));
    }

    #[test]
    fn with_no_folder_yet_the_servers_spelling_is_used() {
        let dir = std::env::temp_dir().join("moose-rack-dl-fresh");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("saves")).unwrap();
        let path = download_path(&dir, "Game.srm", Destination::Core { core: Some("mgba") });
        assert_eq!(path, dir.join("saves/mgba/Game.srm"));
    }

    /// Batocera files by platform and keeps states in that same folder, so a
    /// download must not go looking for a `states/` tree that is not there.
    #[test]
    fn a_by_system_device_files_everything_under_the_platform() {
        let root = Path::new("/userdata");
        assert_eq!(
            download_path(root, "Crash Bandicoot (USA).srm", Destination::System("psx".into())),
            Path::new("/userdata/saves/psx/Crash Bandicoot (USA).srm")
        );
        assert_eq!(
            download_path(root, "Kirby (USA).state1", Destination::System("gba".into())),
            Path::new("/userdata/saves/gba/Kirby (USA).state1"),
            "states share the platform folder on this layout"
        );
    }

    /// The format the server compares timestamps in. A wrong date here does
    /// not error — it silently makes every local save look older or newer than
    /// it is, which decides who wins a sync.
    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
        // A leap day, which is where a hand-rolled conversion goes wrong.
        assert_eq!(rfc3339(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    /// Save states and game saves live in different trees. Putting one in
    /// the other's directory means the next scan never finds it again.
    #[test]
    fn downloads_land_where_the_scanner_will_find_them() {
        let root = Path::new("/ra");
        assert_eq!(
            download_path(root, "Game.state", Destination::Core { core: Some("snes9x") }),
            Path::new("/ra/states/snes9x/Game.state")
        );
        assert_eq!(
            download_path(root, "Game.state3", Destination::Core { core: Some("snes9x") }),
            Path::new("/ra/states/snes9x/Game.state3")
        );
        assert_eq!(
            download_path(root, "Game.srm", Destination::Core { core: Some("snes9x") }),
            Path::new("/ra/saves/snes9x/Game.srm")
        );
    }

    /// RetroArch nests by core; a download without one would land at the top of
    /// the tree where the scanner does not look.
    #[test]
    fn a_missing_emulator_does_not_produce_a_stray_directory() {
        assert_eq!(
            download_path(Path::new("/ra"), "Game.srm", Destination::Core { core: None }),
            Path::new("/ra/saves/Game.srm")
        );
        assert_eq!(
            download_path(Path::new("/ra"), "Game.srm", Destination::Core { core: Some("") }),
            Path::new("/ra/saves/Game.srm"),
            "an empty core name must not create an unnamed directory"
        );
    }

    /// A conflict with just enough shape to count as one.
    fn conflict(file_name: &str) -> SaveConflict {
        SaveConflict {
            rom_id: 7,
            save_id: Some(1),
            file_name: file_name.to_owned(),
            slot: Some("autosave".to_owned()),
            emulator: Some("snes9x".to_owned()),
            reason: Some("both sides changed".to_owned()),
            local_path: Some(PathBuf::from("/ra/saves/snes9x/Zelda.srm")),
            local_updated: Some("2026-08-06T10:00:00Z".to_owned()),
            local_bytes: 8192,
            server_updated: Some("2026-08-06T12:00:00Z".to_owned()),
        }
    }

    fn candidate(name: &str, resolution: Resolution, canonical: bool) -> Candidate {
        Candidate {
            path: PathBuf::from(format!("/ra/saves/snes9x/{name}")),
            kind: saves::Kind::Save,
            core_dir: "Snes9x".to_owned(),
            core: Some("snes9x".to_owned()),
            rom_base: "Game".to_owned(),
            slot: "srm".to_owned(),
            size: 8192,
            content_hash: "abc123".to_owned(),
            resolution,
            canonical,
            superseded_by: None,
        }
    }

    /// Only saves that pair with a ROM are offered. An unmatched one has no
    /// rom_id, and a superseded one would fight the file that beat it for the
    /// same (rom_id, slot) on every single sync.
    #[test]
    fn only_canonical_matched_saves_are_offered() {
        let resolved = || Resolution::Resolved {
            rom_id: 7,
            platform: "snes".to_owned(),
            fs_name: "Game.sfc".to_owned(),
        };
        let candidates = vec![
            candidate("Game.srm", resolved(), true),
            candidate("Loser.srm", resolved(), false),
            candidate("Orphan.srm", Resolution::Unmatched, true),
            candidate("Unknown.srm", Resolution::UnknownCore, true),
            candidate("Two.srm", Resolution::Ambiguous(vec![]), true),
        ];

        let (states, skipped) = client_states(&candidates);
        assert_eq!(states.len(), 1, "only the canonical, resolved one");
        assert_eq!(skipped, 4);
        assert_eq!(states[0].rom_id, 7);
        assert_eq!(states[0].file_name, "Game.srm");
        assert_eq!(states[0].content_hash, "abc123");
        assert_eq!(states[0].emulator.as_deref(), Some("snes9x"));
    }

    /// The scanner names the core directory even when it cannot resolve it to
    /// a libretro stem. Sending nothing would lose the pairing entirely.
    #[test]
    fn an_unresolved_core_falls_back_to_its_directory_name() {
        let mut c = candidate(
            "Game.srm",
            Resolution::Resolved {
                rom_id: 7,
                platform: "snes".to_owned(),
                fs_name: "Game.sfc".to_owned(),
            },
            true,
        );
        c.core = None;
        let (states, _) = client_states(&[c]);
        assert_eq!(states[0].emulator.as_deref(), Some("Snes9x"));
    }


    /// The launch note is the only place an automatic sync is visible, so it
    /// must say nothing when nothing happened and must never hide a conflict.
    #[test]
    fn an_automatic_sync_reports_only_what_it_did() {
        let quiet = Summary { unchanged: 4, ..Default::default() };
        assert_eq!(describe(When::BeforeLaunch, &quiet), None, "silent when idle");

        let pulled = Summary { downloaded: 2, ..Default::default() };
        assert_eq!(
            describe(When::BeforeLaunch, &pulled).unwrap(),
            "saves: 2 pulled before launch"
        );

        // An upload is not a pull: reporting the wrong direction would tell
        // someone their progress left the machine when it did not.
        assert_eq!(describe(When::BeforeLaunch, &Summary { uploaded: 3, ..Default::default() }), None);
        assert_eq!(
            describe(When::AfterExit, &Summary { uploaded: 3, ..Default::default() }).unwrap(),
            "saves: 3 pushed after exit"
        );
    }

    /// A conflict is the one outcome that needs the user, so it is surfaced
    /// even when nothing moved -- otherwise an automatic sync that quietly
    /// declined to act looks exactly like one that had nothing to do.
    #[test]
    fn a_conflict_is_always_surfaced() {
        let stuck = Summary { conflicts: vec![conflict("Zelda.srm")], ..Default::default() };
        let note = describe(When::AfterExit, &stuck).expect("a conflict must be reported");
        assert!(note.contains("1 in conflict"), "{note}");
        assert!(note.contains("left alone"), "and that nothing was written: {note}");
        assert!(note.contains("sync-saves"), "and how to look: {note}");
    }

    #[test]
    fn the_headline_says_nothing_happened_when_nothing_did() {
        let s = Summary { unchanged: 12, ..Default::default() };
        assert_eq!(s.headline(), "Saves already in sync (12 checked)");
        let s = Summary {
            uploaded: 2,
            conflicts: vec![conflict("Zelda.srm")],
            ..Default::default()
        };
        assert_eq!(s.headline(), "2 uploaded, 1 in conflict");
    }


    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-savesync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// What this disk does with case, asked without the code under test.
    fn disk_folds_case(dir: &Path) -> bool {
        std::fs::write(dir.join("Probe"), b"").unwrap();
        let folds = dir.join("probe").exists();
        std::fs::remove_file(dir.join("Probe")).unwrap();
        folds
    }

    fn twin_save(path: &str, rom_id: i64, hash: &str) -> Candidate {
        Candidate {
            path: PathBuf::from(path),
            kind: saves::Kind::Save,
            core_dir: "gba".into(),
            core: Some("mgba".into()),
            rom_base: Path::new(path).file_stem().unwrap().to_string_lossy().into_owned(),
            slot: "unslotted".into(),
            size: 32768,
            content_hash: hash.into(),
            resolution: Resolution::Resolved {
                rom_id,
                platform: "gba".into(),
                fs_name: "rom.zip".into(),
            },
            canonical: true,
            superseded_by: None,
        }
    }

    fn op(action: &str, rom_id: i64, name: &str, hash: Option<&str>) -> crate::api::SyncOperation {
        crate::api::SyncOperation {
            action: action.into(),
            rom_id,
            save_id: Some(1),
            file_name: Some(name.into()),
            slot: None,
            emulator: None,
            reason: None,
            server_content_hash: hash.map(str::to_owned),
            server_updated_at: None,
        }
    }

    const THE: &str = "/card/saves/gba/Kirby & The Amazing Mirror (USA).srm";
    const THE_NAME: &str = "Kirby & The Amazing Mirror (USA).srm";
    const LOWER: &str = "Kirby & the Amazing Mirror (USA).srm";

    /// The Flip's own Kirby save was replaced by the server's older copy this
    /// way on 2026-09-24. Take `folds(...)` out of `case_twins` and the first
    /// assertion fails: the download goes through as a plain download.
    #[test]
    fn a_download_spelled_differently_is_never_written_over_a_local_save() {
        let local = twin_save(THE, 7, "newer");
        let plan = vec![
            op("upload", 7, THE_NAME, None),
            op("download", 7, LOWER, Some("older")),
        ];
        let folds = |_: &Path, _: &str| true;
        let twins = case_twins(&plan, std::slice::from_ref(&local), &folds);
        assert_eq!(twins.get(&1), Some(&Twin::Differ { candidate: 0 }), "a conflict, not a download");
        assert_eq!(twins.len(), 1);

        // Same bytes: nothing moves either way, and the local file takes the
        // server's spelling so the pair is not offered again.
        let same = vec![op("upload", 7, THE_NAME, None), op("download", 7, LOWER, Some("newer"))];
        assert_eq!(
            case_twins(&same, std::slice::from_ref(&local), &folds).get(&1),
            Some(&Twin::Same { local: PathBuf::from(THE), server_name: LOWER.into() })
        );
    }

    /// On a disk that keeps case the two spellings are two files, and a
    /// download writes the one it names. Nothing changes there.
    #[test]
    fn a_case_sensitive_disk_pairs_nothing() {
        let local = twin_save(THE, 7, "newer");
        let plan = vec![op("download", 7, LOWER, Some("older"))];
        let keeps = |_: &Path, _: &str| false;
        assert!(case_twins(&plan, &[local], &keeps).is_empty());
    }

    #[test]
    fn only_the_same_game_is_ever_paired() {
        let folds = |_: &Path, _: &str| true;
        // Another game whose name happens to differ only in case.
        let other = twin_save(THE, 8, "h");
        assert!(case_twins(&[op("download", 7, LOWER, Some("h"))], &[other], &folds).is_empty());
        // A save state is not a save.
        let mut state = twin_save(THE, 7, "h");
        state.kind = saves::Kind::State;
        assert!(case_twins(&[op("download", 7, LOWER, Some("h"))], &[state], &folds).is_empty());
        // The exact name is a normal download, handled by the server's own rules.
        let exact = twin_save(THE, 7, "h");
        assert!(case_twins(&[op("download", 7, THE_NAME, Some("x"))], &[exact], &folds).is_empty());
    }

    /// Against the real disk under the test, whichever kind it is: the Mac and
    /// Windows fold case, the Linux CI runner does not.
    #[test]
    fn another_spelling_occupies_the_file_only_where_the_disk_folds_case() {
        let dir = scratch("occupies");
        let folds = disk_folds_case(&dir);
        let real = dir.join(THE_NAME);
        std::fs::write(&real, b"the flip's save").unwrap();

        assert_eq!(occupies(&real, LOWER), folds);
        assert!(!occupies(&real, THE_NAME), "its own name is not another spelling");
        assert!(!occupies(&real, "Kirby & the Amazing Mirror (Europe).srm"));
        if !folds {
            // Two genuinely separate files: neither is the other.
            std::fs::write(dir.join(LOWER), b"another file").unwrap();
            assert!(!occupies(&real, LOWER));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A direct rename to a case variant is a silent no-op on Linux over a
    /// folding filesystem, so `respell` goes through a temporary name.
    #[test]
    fn respelling_changes_the_name_and_keeps_the_bytes() {
        let dir = scratch("respell");
        let real = dir.join(THE_NAME);
        std::fs::write(&real, b"progress").unwrap();

        let moved = respell(&real, LOWER).unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![LOWER.to_owned()], "one file, spelled the server's way");
        assert_eq!(std::fs::read(moved).unwrap(), b"progress");
        let _ = std::fs::remove_dir_all(&dir);
    }


    /// The download that overwrote every save in a system after an incomplete
    /// game index: the device did not report them, so the server offered its
    /// copies as new. Delete the `occupied` call in `download_one`, or make it
    /// ignore `reported`, and this is what goes through.
    #[test]
    fn a_download_stops_at_a_save_the_device_never_reported() {
        let dir = scratch("occupied");
        let here = dir.join("Metroid Fusion (USA).srm");
        std::fs::write(&here, b"the flip's progress").unwrap();
        let ours = crate::savehash::compute(&here).unwrap();

        let unreported = Guard::Unreported { reported: false, server_hash: Some("the server's") };
        match occupied(&here, &unreported).unwrap() {
            Some(Landed::Occupied { same: false, path }) => assert_eq!(path, here),
            other => panic!("expected a conflict, got {other:?}"),
        }
        let same = Guard::Unreported { reported: false, server_hash: Some(ours.as_str()) };
        assert!(matches!(occupied(&here, &same).unwrap(), Some(Landed::Occupied { same: true, .. })));

        // Reported by the device: the server's rule decides, and it said download.
        let reported = Guard::Unreported { reported: true, server_hash: Some("the server's") };
        assert!(occupied(&here, &reported).unwrap().is_none());
        // Nothing there: a genuinely new save.
        assert!(occupied(&dir.join("new.srm"), &unreported).unwrap().is_none());
        // Chosen by a person.
        assert!(occupied(&here, &Guard::Overwrite).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two systems, one file name. The upload for the Game Boy Color game must
    /// send the Game Boy Color file.
    #[test]
    fn the_local_save_is_found_by_game_as_well_as_name() {
        let gba = twin_save("/card/saves/gba/Tony Hawk's Pro Skater 3 (USA).srm", 1, "gba bytes");
        let gbc = twin_save("/card/saves/gbc/Tony Hawk's Pro Skater 3 (USA).srm", 2, "gbc bytes");
        let all = [gba, gbc];
        let found = local_for(&all, 2, "Tony Hawk's Pro Skater 3 (USA).srm").unwrap();
        assert_eq!(found.content_hash, "gbc bytes");
        assert!(local_for(&all, 3, "Tony Hawk's Pro Skater 3 (USA).srm").is_none());
    }

    #[test]
    fn a_failure_is_never_headlined_as_in_sync() {
        let s = Summary { unchanged: 300, failed: 2, ..Default::default() };
        assert_eq!(s.headline(), "2 failed");
    }

    #[test]
    fn an_atomic_write_leaves_no_part_file() {
        let dir = scratch("atomic");
        let path = dir.join("a.srm");
        std::fs::write(&path, b"old").unwrap();
        write_atomically(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

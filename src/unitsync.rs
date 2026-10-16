//! Syncing save units with the server: the standalone emulators' saves.
//!
//! `docs/emulator-saves.md` has the design and [`crate::saveunit`] the format.
//! Each `[[saves.units]]` in config.toml is a [`Source`], the place one
//! emulator keeps its saves on this device. A conflict joins the save
//! conflicts as a [`SaveConflict`] named `family/key`, a name no save file
//! has, so the same picker and the same `--keep` answer it through
//! [`resolve`].

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::api::{ClientUnit, Client, UnitOperation};
use crate::config::UnitSourceCfg;
use crate::saveunit;
use crate::savesync::{DeviceIdentity, Keep, SaveConflict, Summary};
use crate::syncplan::{Action, Line};

/// Where one family's units live on this device.
#[derive(Debug, Clone)]
pub struct Source {
    pub family: String,
    /// The folder whose subfolders are the units: `PSP/SAVEDATA`, or a
    /// folder memory card.
    pub dir: PathBuf,
    pub max_bytes: u64,
    pub skip: Vec<String>,
}

impl Source {
    pub fn open(cfg: &UnitSourceCfg) -> Result<Self> {
        let root = crate::util::expand_tilde(&cfg.root);
        let dir = match cfg.family.as_str() {
            "ppsspp" => {
                let nested = root.join("PSP").join("SAVEDATA");
                if nested.is_dir() {
                    nested
                } else if root.is_dir() && root.file_name().is_some_and(|n| n == "SAVEDATA") {
                    root
                } else {
                    bail!("{} has no PSP/SAVEDATA folder", root.display());
                }
            }
            "pcsx2" => {
                if root.is_file() {
                    bail!(
                        "{} is a file memory card, which holds every game in one file and cannot \
                         be synced by game. PCSX2 can convert it to a folder card in its memory \
                         card settings.",
                        root.display()
                    );
                }
                if !root.is_dir() {
                    bail!("{} does not exist", root.display());
                }
                if !root.join("_pcsx2_superblock").is_file() {
                    let cards: Vec<String> = std::fs::read_dir(&root)
                        .map(|rd| {
                            rd.flatten()
                                .map(|e| e.file_name().to_string_lossy().into_owned())
                                .filter(|n| n.to_ascii_lowercase().ends_with(".ps2"))
                                .collect()
                        })
                        .unwrap_or_default();
                    bail!(
                        "{} is not a folder memory card{}",
                        root.display(),
                        if cards.is_empty() { String::new() } else { format!("; name one of: {}", cards.join(", ")) }
                    );
                }
                root
            }
            other => bail!("no save adapter for {other:?}; there is one for {}", saveunit::FAMILIES.join(", ")),
        };
        Ok(Source {
            family: cfg.family.clone(),
            dir,
            max_bytes: cfg.max_mb.map(|mb| mb * 1024 * 1024).unwrap_or(saveunit::DEFAULT_MAX_BYTES),
            skip: cfg.skip.clone(),
        })
    }

    /// The units here, with anything not synced counted and said in `summary`.
    pub fn scan(&self, summary: &mut Summary) -> Vec<Held> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            summary.fail(format!("could not read {}", self.dir.display()));
            return out;
        };
        let mut entries: Vec<_> = rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let key = e.file_name().to_string_lossy().into_owned();
            // Ours (`.x.incoming`), the OS's, and PCSX2's own card files.
            if key.starts_with('.') || key.starts_with("_pcsx2") || self.skip.contains(&key) {
                continue;
            }
            if !saveunit::valid_key(&key) {
                summary.skipped += 1;
                summary.notes.push(format!("{}/{key}: not a name every device can hold, so not synced", self.family));
                continue;
            }
            let summary_of = match saveunit::summarize(&e.path()) {
                Ok(s) => s,
                Err(err) => {
                    summary.fail(format!("could not read {}/{key}: {err:#}", self.family));
                    continue;
                }
            };
            if summary_of.files == 0 {
                continue;
            }
            if summary_of.bytes > self.max_bytes {
                summary.skipped += 1;
                summary.notes.push(format!(
                    "{}/{key}: {} MB, over the {} MB limit for one save, so not synced",
                    self.family,
                    summary_of.bytes / (1024 * 1024),
                    self.max_bytes / (1024 * 1024)
                ));
                continue;
            }
            out.push(Held { key, path: e.path(), summary: summary_of });
        }
        out
    }
}

/// One unit this device holds.
#[derive(Debug, Clone)]
pub struct Held {
    pub key: String,
    pub path: PathBuf,
    pub summary: saveunit::Summary,
}

/// The configured sources. One per family: two PS2 cards on one device would
/// both answer for the same game, and nothing could say which is meant.
pub fn sources(cfgs: &[UnitSourceCfg], summary: &mut Summary) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    for cfg in cfgs {
        if out.iter().any(|s| s.family == cfg.family) {
            summary.fail(format!("a second [[saves.units]] for {} was ignored: one per device", cfg.family));
            continue;
        }
        match Source::open(cfg) {
            Ok(s) => out.push(s),
            Err(e) => summary.fail(format!("{} saves not synced: {e:#}", cfg.family)),
        }
    }
    out
}

/// Whether the family's emulator is running, where this platform can tell.
///
/// A running PCSX2 holds the card in memory and writes it back on its own
/// schedule, so a save put in under it can be overwritten with the old one.
/// Android lets an app see only its own processes, so there this is always
/// false and the plan sheet asks for the emulator to be closed instead.
fn running(family: &str) -> bool {
    #[cfg(all(unix, not(target_os = "android")))]
    {
        let name = match family {
            "pcsx2" => "pcsx2",
            "ppsspp" => "ppsspp",
            _ => return false,
        };
        std::process::Command::new("pgrep")
            .args(["-i", name])
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(not(all(unix, not(target_os = "android"))))]
    {
        let _ = family;
        false
    }
}

fn label(family: &str, key: &str) -> String {
    format!("{family}/{key}")
}

/// `family/key` back from a conflict's file name, when it names a unit.
pub fn unit_of(file_name: &str) -> Option<(&str, &str)> {
    let (family, key) = file_name.split_once('/')?;
    (saveunit::valid_family(family) && saveunit::valid_key(key)).then_some((family, key))
}

fn as_conflict(family: &str, key: &str, reason: Option<String>, held: &Held, server_updated: Option<&str>) -> SaveConflict {
    SaveConflict {
        rom_id: 0,
        save_id: Some(saveunit::unit_id(family, key)),
        file_name: label(family, key),
        slot: None,
        emulator: Some(family.to_owned()),
        reason,
        local_path: Some(held.path.clone()),
        local_updated: Some(crate::savesync::rfc3339(held.summary.modified)),
        local_bytes: held.summary.bytes as i64,
        server_updated: server_updated
            .and_then(|s| s.parse::<i64>().ok())
            .map(crate::savesync::rfc3339),
    }
}

/// Upload `held`. False when the server refused because its copy moved.
async fn upload(client: &Client, family: &str, held: &Held, device_id: &str, overwrite: bool) -> Result<bool> {
    let zip = saveunit::pack(&held.path)?;
    let hash = saveunit::check(&zip)?;
    match client.upload_unit(family, &held.key, zip, device_id, overwrite).await? {
        Ok(stored) if stored.content_hash != hash => {
            bail!("the server stored a different copy of {} than it was sent", label(family, &held.key))
        }
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Fetch the server's copy and put it at `target`, backing up what is there.
async fn download(
    client: &Client,
    family: &str,
    key: &str,
    expect: Option<&str>,
    target: &Path,
    device_id: &str,
    library_root: &Path,
) -> Result<()> {
    let id = saveunit::unit_id(family, key);
    let zip = client.unit_content(id).await?;
    let hash = saveunit::check(&zip).with_context(|| format!("the server's copy of {}", label(family, key)))?;
    if let Some(want) = expect
        && want != hash
    {
        bail!("{} changed on the server while it was being fetched; sync again", label(family, key));
    }
    crate::savebackup::keep_folder(library_root, family, key, target)?;
    saveunit::unpack(&zip, target)?;
    client.confirm_unit(id, device_id).await
}

/// Ask the server and do what it says, into `summary`.
pub async fn run(
    client: &Client,
    sources: &[Source],
    data_dir: &Path,
    library_root: &Path,
    summary: &mut Summary,
) -> Result<()> {
    let Some((identity, held, plan)) = negotiate(client, sources, data_dir, summary).await? else {
        return Ok(());
    };
    for op in plan {
        let Some(src) = sources.iter().find(|s| s.family == op.family) else { continue };
        let name = label(&op.family, &op.key);
        let local = held.iter().find(|(f, h)| *f == op.family && h.key == op.key).map(|(_, h)| h);
        match (op.action.as_str(), local) {
            ("upload", Some(h)) => match upload(client, &op.family, h, &identity.device_id, false).await {
                Ok(true) => {
                    summary.uploaded += 1;
                    summary.notes.push(format!("uploaded {name}"));
                }
                Ok(false) => summary.conflicts.push(as_conflict(&op.family, &op.key, op.reason.clone(), h, None)),
                Err(e) => summary.fail(format!("could not upload {name}: {e:#}")),
            },
            ("download", _) => {
                let target = src.dir.join(&op.key);
                match download(
                    client,
                    &op.family,
                    &op.key,
                    op.server_content_hash.as_deref(),
                    &target,
                    &identity.device_id,
                    library_root,
                )
                .await
                {
                    Ok(()) => {
                        summary.downloaded += 1;
                        summary.notes.push(format!("downloaded {name}"));
                    }
                    Err(e) => summary.fail(format!("could not download {name}: {e:#}")),
                }
            }
            ("conflict", Some(h)) => summary.conflicts.push(as_conflict(
                &op.family,
                &op.key,
                op.reason.clone(),
                h,
                op.server_updated_at.as_deref(),
            )),
            ("no_op", _) => summary.unchanged += 1,
            ("skip", _) => {
                summary.skipped += 1;
                summary.notes.push(format!("{name}: {}", op.reason.as_deref().unwrap_or("skipped")));
            }
            (other, _) => summary.fail(format!("{name}: the server asked for {other:?}, which this device cannot do")),
        }
    }
    Ok(())
}

type Negotiated = (DeviceIdentity, Vec<(String, Held)>, Vec<UnitOperation>);

/// Scan every source the emulator is not running for, and ask the server.
/// `None` when there is nothing to ask about.
async fn negotiate(
    client: &Client,
    sources: &[Source],
    data_dir: &Path,
    summary: &mut Summary,
) -> Result<Option<Negotiated>> {
    let mut held = Vec::new();
    let mut families = Vec::new();
    for s in sources {
        if running(&s.family) {
            summary.fail(format!("{} is running, so its saves were left alone; close it and sync again", s.family));
            continue;
        }
        families.push(s.family.clone());
        held.extend(s.scan(summary).into_iter().map(|h| (s.family.clone(), h)));
    }
    if families.is_empty() {
        return Ok(None);
    }
    let identity = DeviceIdentity::ensure(client, data_dir).await?;
    let units: Vec<ClientUnit> = held
        .iter()
        .map(|(f, h)| ClientUnit { family: f.clone(), key: h.key.clone(), content_hash: h.summary.hash.clone() })
        .collect();
    let plan = client.negotiate_units(&identity.device_id, &families, &units).await?;
    Ok(Some((identity, held, plan.operations)))
}

/// The plan for the plan sheet: its lines, how many already match, and what
/// the person should be told that is not a line.
pub async fn preview(
    client: &Client,
    sources: &[Source],
    data_dir: &Path,
) -> Result<(Vec<Line>, usize, Vec<String>)> {
    let mut scratch = Summary::default();
    let Some((_, held, plan)) = negotiate(client, sources, data_dir, &mut scratch).await? else {
        return Ok((Vec::new(), 0, scratch.problems));
    };
    let mut lines = Vec::new();
    let mut agreed = 0;
    for op in plan {
        let action = match op.action.as_str() {
            "upload" => Action::Upload,
            "download" => Action::Download,
            "conflict" => Action::Conflict,
            "no_op" => {
                agreed += 1;
                continue;
            }
            _ => continue,
        };
        let local = held.iter().find(|(f, h)| *f == op.family && h.key == op.key).map(|(_, h)| h);
        let name = label(&op.family, &op.key);
        let title = match local.and_then(|h| title(&op.family, &h.path)) {
            Some(t) => format!("{t} ({name})"),
            None => name,
        };
        lines.push(Line {
            action,
            title,
            reason: op.reason.filter(|r| !r.trim().is_empty()),
            rom_id: 0,
            save_id: op.unit_id,
        });
    }
    // Problems found while scanning belong in the sheet too, or it reads
    // "nothing to do" while nothing was checked.
    let mut notes = scratch.problems;
    if !lines.is_empty() {
        notes.push(
            "Close PPSSPP and the PS2 emulator before carrying this out: one still running can \
             write its old copy back over what arrives."
                .into(),
        );
    }
    Ok((lines, agreed, notes))
}

/// Settle one unit conflict, keeping one side. The other is backed up first
/// when it is this device's, and kept in the server's history when it is the
/// server's.
pub async fn resolve(
    client: &Client,
    conflict: &SaveConflict,
    keep: Keep,
    library_root: &Path,
    data_dir: &Path,
) -> Result<String> {
    let (family, key) = unit_of(&conflict.file_name).context("not a save unit")?;
    let path = conflict.local_path.as_deref().context("the conflict names no local copy")?;
    let identity = DeviceIdentity::ensure(client, data_dir).await?;
    match keep {
        Keep::Local => {
            let held = Held { key: key.to_owned(), path: path.to_owned(), summary: saveunit::summarize(path)? };
            upload(client, family, &held, &identity.device_id, true).await?;
            Ok(format!("kept this device's {}", conflict.file_name))
        }
        Keep::Server => {
            download(client, family, key, None, path, &identity.device_id, library_root).await?;
            Ok(format!("kept the server's {}", conflict.file_name))
        }
    }
}

/// The game's own name for a save, where the format carries one: PARAM.SFO's
/// `SAVEDATA_TITLE`, else `TITLE`, for PSP saves.
pub fn title(family: &str, dir: &Path) -> Option<String> {
    if family != "ppsspp" {
        return None;
    }
    let sfo = std::fs::read(dir.join("PARAM.SFO")).ok()?;
    let fields = sfo_fields(&sfo)?;
    let pick = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
    pick("SAVEDATA_TITLE").or_else(|| pick("TITLE"))
}

/// The UTF-8 string fields of a PARAM.SFO.
fn sfo_fields(b: &[u8]) -> Option<Vec<(String, String)>> {
    let u16_at = |o: usize| Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?));
    let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize);
    if b.get(..4)? != b"\0PSF" {
        return None;
    }
    let (keys, data, count) = (u32_at(8)?, u32_at(12)?, u32_at(16)?);
    let cstr = |o: usize, max: usize| {
        let s = b.get(o..(o + max).min(b.len()))?;
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Some(String::from_utf8_lossy(&s[..end]).into_owned())
    };
    let mut out = Vec::new();
    for i in 0..count.min(256) {
        let at = 20 + i * 16;
        let (key_off, fmt, len) = (u16_at(at)? as usize, u16_at(at + 2)?, u32_at(at + 4)?);
        if fmt == 0x0204 {
            out.push((cstr(keys + key_off, 64)?, cstr(data + u32_at(at + 12)?, len)?));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("moose-unitsync-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cfg(family: &str, root: &Path) -> UnitSourceCfg {
        UnitSourceCfg { family: family.into(), root: root.to_string_lossy().into_owned(), max_mb: None, skip: vec![] }
    }

    /// A PARAM.SFO with the given string fields, laid out as the PSP does.
    fn sfo(fields: &[(&str, &str)]) -> Vec<u8> {
        let mut keys = Vec::new();
        let mut data = Vec::new();
        let mut index = Vec::new();
        for (k, v) in fields {
            let (ko, dof) = (keys.len() as u16, data.len() as u32);
            keys.extend_from_slice(k.as_bytes());
            keys.push(0);
            let mut val = v.as_bytes().to_vec();
            val.push(0);
            index.extend_from_slice(&ko.to_le_bytes());
            index.extend_from_slice(&0x0204u16.to_le_bytes());
            index.extend_from_slice(&(val.len() as u32).to_le_bytes());
            index.extend_from_slice(&(val.len() as u32).to_le_bytes());
            index.extend_from_slice(&dof.to_le_bytes());
            data.extend_from_slice(&val);
        }
        let key_start = 20 + index.len();
        let data_start = key_start + keys.len();
        let mut out = b"\0PSF".to_vec();
        out.extend_from_slice(&0x0101u32.to_le_bytes());
        out.extend_from_slice(&(key_start as u32).to_le_bytes());
        out.extend_from_slice(&(data_start as u32).to_le_bytes());
        out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        out.extend(index);
        out.extend(keys);
        out.extend(data);
        out
    }

    #[test]
    fn a_memory_stick_finds_its_savedata_and_reads_titles() {
        let d = scratch("stick");
        let save = d.join("PSP/SAVEDATA/UCUS98653DATA00");
        std::fs::create_dir_all(&save).unwrap();
        std::fs::write(save.join("PARAM.SFO"), sfo(&[("CATEGORY", "MS"), ("SAVEDATA_TITLE", "Size Matters"), ("TITLE", "R&C")])).unwrap();
        std::fs::write(save.join("DATA.BIN"), b"x").unwrap();
        std::fs::create_dir_all(d.join("PSP/SAVEDATA/.stfolder.removed-1")).unwrap();

        let src = Source::open(&cfg("ppsspp", &d)).unwrap();
        assert_eq!(src.dir, d.join("PSP/SAVEDATA"));
        let mut s = Summary::default();
        let held = src.scan(&mut s);
        assert_eq!(held.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), ["UCUS98653DATA00"]);
        assert_eq!(title("ppsspp", &held[0].path).as_deref(), Some("Size Matters"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn install_data_over_the_limit_is_left_alone_and_said_so() {
        let d = scratch("big");
        let big = d.join("PSP/SAVEDATA/ULUS10509DAT");
        std::fs::create_dir_all(&big).unwrap();
        std::fs::write(big.join("DATA.BIN"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        let mut c = cfg("ppsspp", &d);
        c.max_mb = Some(1);
        let mut s = Summary::default();
        assert!(Source::open(&c).unwrap().scan(&mut s).is_empty());
        assert_eq!(s.skipped, 1);
        assert!(s.notes[0].contains("ULUS10509DAT: 2 MB, over the 1 MB limit"), "{:?}", s.notes);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_folder_card_gives_one_unit_per_game_and_keeps_its_own_files() {
        let d = scratch("card");
        let card = d.join("Mcd001.ps2");
        std::fs::create_dir_all(card.join("BASLUS-20035")).unwrap();
        std::fs::write(card.join("_pcsx2_superblock"), b"sb").unwrap();
        std::fs::write(card.join("BASLUS-20035/BASLUS-20035"), b"save").unwrap();
        std::fs::write(card.join("BASLUS-20035/_pcsx2_index"), b"idx").unwrap();
        std::fs::create_dir_all(card.join("_pcsx2_meta")).unwrap();

        let src = Source::open(&cfg("pcsx2", &card)).unwrap();
        let mut s = Summary::default();
        let held = src.scan(&mut s);
        assert_eq!(held.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), ["BASLUS-20035"]);
        assert_eq!(held[0].summary.files, 2, "the game's own index travels with it");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_file_card_or_a_memcards_folder_is_refused_with_the_way_out() {
        let d = scratch("filecard");
        std::fs::write(d.join("Mcd001.ps2"), vec![0u8; 16]).unwrap();
        let e = Source::open(&cfg("pcsx2", &d.join("Mcd001.ps2"))).unwrap_err().to_string();
        assert!(e.contains("file memory card") && e.contains("convert"), "{e}");
        let e = Source::open(&cfg("pcsx2", &d)).unwrap_err().to_string();
        assert!(e.contains("name one of: Mcd001.ps2"), "{e}");
        let e = Source::open(&cfg("dolphin", &d)).unwrap_err().to_string();
        assert!(e.contains("no save adapter"), "{e}");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn only_one_source_per_family() {
        let d = scratch("twice");
        std::fs::create_dir_all(d.join("PSP/SAVEDATA")).unwrap();
        let mut s = Summary::default();
        let got = sources(&[cfg("ppsspp", &d), cfg("ppsspp", &d)], &mut s);
        assert_eq!(got.len(), 1);
        assert_eq!(s.failed, 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_unit_conflict_is_named_so_resolve_can_find_it() {
        assert_eq!(unit_of("ppsspp/UCUS98653DATA00"), Some(("ppsspp", "UCUS98653DATA00")));
        assert_eq!(unit_of("pcsx2/BASLUS-20035"), Some(("pcsx2", "BASLUS-20035")));
        // An ordinary save never looks like one.
        assert_eq!(unit_of("Final Fantasy VII (USA).srm"), None);
        assert_eq!(unit_of("snes/Foo.srm"), None);
        assert_eq!(unit_of("ppsspp/../x"), None);
    }
}

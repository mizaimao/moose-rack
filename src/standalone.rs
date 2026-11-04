//! Standalone emulators the desktop app installs, sets up and starts itself,
//! for systems RetroArch can't run well. PCSX2 for the PS2 first. See
//! `docs/server-and-client.md`, "Play, step by step".
//!
//! Each one lives in `<library>/emulators/<id>/` in its own portable mode, so
//! its settings, BIOS choice and memory cards stay in that folder and never
//! touch a copy installed some other way. The version is pinned: an update is
//! a change to this file, not something that happens on its own.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub struct Emulator {
    /// Folder name under `emulators/`, and the id the front end asks for.
    pub id: &'static str,
    pub name: &'static str,
    /// The release tag the download comes from.
    pub version: &'static str,
    /// Library platform slugs it plays.
    pub platforms: &'static [&'static str],
    /// Save unit family (`unitsync`), when its saves sync as units.
    pub family: Option<&'static str>,
    /// Download size in bytes of the pinned release: (macOS, Windows). Shown
    /// in the launch checklist before anything is fetched.
    pub sizes: (u64, u64),
}

pub const PCSX2: Emulator = Emulator {
    id: "pcsx2",
    name: "PCSX2",
    version: "v2.8.2",
    platforms: &["ps2"],
    family: Some("pcsx2"),
    sizes: (24_377_200, 25_670_075),
};

pub const ALL: &[&Emulator] = &[&PCSX2];

/// The folder card the app gives PCSX2. A folder rather than an 8 MB file, so
/// each game's save syncs on its own (`docs/emulator-saves.md`).
pub const CARD: &str = "Moose.ps2";

/// The formatted header of a PS2 folder card, from one PCSX2 made.
///
/// PCSX2's own "new folder card" writes an empty one, and every game then asks
/// to format the card. Formatting would also hide saves that sync brought in
/// before the first boot. The header is the standard 8 MB card layout and holds
/// nothing about the card it came from; PCSX2 rebuilds the rest of the card
/// from the game folders each time it loads it.
const CARD_SUPERBLOCK: &[u8] = include_bytes!("../data/pcsx2-folder-card-superblock.bin");

/// The emulator the app runs for this platform on this OS, if there is one.
pub fn for_platform(platform: &str) -> Option<&'static Emulator> {
    if !matches!(std::env::consts::OS, "macos" | "windows") {
        return None;
    }
    ALL.iter().copied().find(|e| e.platforms.contains(&platform))
}

pub fn by_id(id: &str) -> Option<&'static Emulator> {
    ALL.iter().copied().find(|e| e.id == id)
}

/// Where it is installed.
pub fn dir(library_root: &Path, emu: &Emulator) -> PathBuf {
    library_root.join("emulators").join(emu.id)
}

/// Where the app keeps its own RetroArch, portable (`play::install_retroarch`).
pub fn retroarch_dir(library_root: &Path) -> PathBuf {
    library_root.join("emulators").join("retroarch")
}

/// The pinned release's download size on this OS.
pub fn download_size(emu: &Emulator) -> u64 {
    if std::env::consts::OS == "windows" { emu.sizes.1 } else { emu.sizes.0 }
}

/// The version installed in `dir`, as the app recorded it when it installed it.
pub fn installed_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(VERSION_FILE)).ok().map(|s| s.trim().to_owned())
}

/// Beside the program: which pinned release the app put there.
const VERSION_FILE: &str = ".moose-version";

/// The program to start, if it's installed.
pub fn program(dir: &Path) -> Option<PathBuf> {
    let p = match std::env::consts::OS {
        "macos" => dir.join("PCSX2.app/Contents/MacOS/PCSX2"),
        _ => dir.join("pcsx2-qt.exe"),
    };
    p.is_file().then_some(p)
}

/// The release file for this OS and the version pinned above.
fn asset(emu: &Emulator) -> Result<(String, &'static str)> {
    let v = emu.version;
    let name = match std::env::consts::OS {
        "macos" => (format!("pcsx2-{v}-macos-Qt.tar.xz"), "tar.xz"),
        "windows" => (format!("pcsx2-{v}-windows-x64-Qt.7z"), "7z"),
        os => bail!("{} is not set up for {os} yet", emu.name),
    };
    Ok((format!("https://github.com/PCSX2/pcsx2/releases/download/{v}/{}", name.0), name.1))
}

/// Download and unpack the pinned release into `dir`.
///
/// Only the program is replaced. Settings, memory cards and anything else
/// PCSX2 wrote beside it stay. `progress` gets `(downloaded, total)`.
pub async fn install(
    http: &reqwest::Client,
    emu: &Emulator,
    dir: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    use futures_util::StreamExt;

    let (url, kind) = asset(emu)?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let download = dir.join(format!(".download.{kind}"));
    let resp = http.get(&url).send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("GET {url} -> {}", resp.status());
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&download)?;
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading download")?;
        std::io::Write::write_all(&mut file, &chunk)?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    drop(file);

    let staging = dir.join(".incoming");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).ok();
    }
    std::fs::create_dir_all(&staging)?;
    unpack(&download, kind, &staging)?;
    place(&staging, dir)?;
    std::fs::remove_dir_all(&staging).ok();
    std::fs::remove_file(&download).ok();

    // Portable mode: PCSX2 looks for this beside the .app on macOS and beside
    // the .exe on Windows (`EmuFolders::SetAppRoot`).
    let marker = dir.join("portable.txt");
    if !marker.exists() {
        std::fs::write(&marker, "")?;
    }
    std::fs::write(dir.join(VERSION_FILE), emu.version)?;
    program(dir).with_context(|| format!("{} unpacked, but its program isn't in {}", emu.name, dir.display()))
}

fn unpack(archive: &Path, kind: &str, into: &Path) -> Result<()> {
    match kind {
        // The system's tar reads xz on macOS; nothing to vendor for it.
        "tar.xz" => {
            let out = std::process::Command::new("tar")
                .arg("-xf")
                .arg(archive)
                .arg("-C")
                .arg(into)
                .output()
                .context("running tar")?;
            if !out.status.success() {
                bail!("unpacking {}: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim());
            }
        }
        "7z" => sevenz_rust2::decompress_file(archive, into)
            .with_context(|| format!("unpacking {}", archive.display()))?,
        other => bail!("no unpacker for .{other}"),
    }
    Ok(())
}

/// Move the unpacked program into `dir`.
fn place(staging: &Path, dir: &Path) -> Result<()> {
    if std::env::consts::OS == "macos" {
        // The archive holds one .app, named after the version.
        let app = find(staging, &|p| p.extension().is_some_and(|e| e == "app") && p.is_dir())
            .with_context(|| format!("no .app in the {} download", PCSX2.name))?;
        let target = dir.join("PCSX2.app");
        if target.exists() {
            std::fs::remove_dir_all(&target).with_context(|| format!("removing the old {}", target.display()))?;
        }
        std::fs::rename(&app, &target).with_context(|| format!("moving {} into place", app.display()))?;
        return Ok(());
    }
    let exe = find(staging, &|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("pcsx2-qt.exe")))
        .with_context(|| "no pcsx2-qt.exe in the download".to_owned())?;
    lay_over(exe.parent().unwrap_or(staging), dir)
}

/// The first path under `dir`, two levels deep at most, that `pick` accepts.
fn find(dir: &Path, pick: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let level: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    if let Some(p) = level.iter().find(|p| pick(p)) {
        return Some(p.clone());
    }
    level.iter().filter(|p| p.is_dir()).find_map(|d| {
        std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).find(|p| pick(p))
    })
}

/// Copy everything under `from` into `to`, replacing same-named files.
fn lay_over(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            lay_over(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).with_context(|| format!("writing {}", dst.display()))?;
        }
    }
    Ok(())
}

/// The BIOS to use from `bios_dir`: the first of these that is there, else
/// any `.bin`. US first, since most of the library is US discs.
pub fn pick_bios(bios_dir: &Path) -> Option<String> {
    const ORDER: &[&str] = &["SCPH-39001.bin", "SCPH-70004.BIN", "SCPH-30004R.bin", "SCPH-10000.bin"];
    let names: Vec<String> = std::fs::read_dir(bios_dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ORDER
        .iter()
        .find_map(|want| names.iter().find(|n| n.eq_ignore_ascii_case(want)).cloned())
        .or_else(|| {
            let mut bins: Vec<&String> =
                names.iter().filter(|n| n.to_ascii_lowercase().ends_with(".bin")).collect();
            bins.sort();
            bins.first().map(|s| (*s).clone())
        })
}

/// Whether a server BIOS entry is one of the PS2's: it sits in a `PS2` folder
/// on the server (`bios/PS2/SCPH-39001.bin`).
pub fn is_ps2_bios(fw: &crate::api::Firmware) -> bool {
    fw.file_path
        .as_deref()
        .is_some_and(|p| p.split(['/', '\\']).any(|part| part.eq_ignore_ascii_case("ps2")))
}

/// Fetch the server's PS2 BIOS files that `bios_dir` hasn't got. The `.NVM`
/// and `.MEC` beside each BIOS come too: PCSX2 keeps the console's language
/// and clock settings in them. Returns how many were fetched.
pub async fn fetch_bios(client: &crate::api::Client, bios_dir: &Path) -> Result<usize> {
    let list = client.firmware().await.context("listing the server's BIOS files")?;
    std::fs::create_dir_all(bios_dir).with_context(|| format!("creating {}", bios_dir.display()))?;
    let mut got = 0;
    for fw in list.iter().filter(|f| is_ps2_bios(f)) {
        let Some(leaf) = Path::new(&fw.file_name).file_name() else { continue };
        let dest = bios_dir.join(leaf);
        if dest.is_file() {
            continue;
        }
        let bytes = client.firmware_content(fw.id, &fw.file_name).await?;
        let part = dest.with_extension("part");
        std::fs::write(&part, &bytes)?;
        std::fs::rename(&part, &dest)?;
        got += 1;
    }
    Ok(got)
}

/// The settings the app owns in `inis/PCSX2.ini`. Anything else in the file
/// is left as PCSX2 or the user wrote it.
pub fn settings(bios_dir: &Path, bios: Option<&str>) -> Vec<(&'static str, String, String)> {
    let mut out: Vec<(&'static str, String, String)> = Vec::new();
    let mut set = |section: &'static str, key: &str, value: &str| {
        out.push((section, key.to_owned(), value.to_owned()));
    };
    // Without the right version PCSX2 offers to reset everything on start.
    set("UI", "SettingsVersion", "1");
    set("UI", "SetupWizardIncomplete", "false");
    set("UI", "ConfirmShutdown", "false");
    set("UI", "StartFullscreen", "true");
    set("UI", "HideMouseCursor", "true");
    // The version is pinned by the app; PCSX2 asking to update would undo that.
    set("AutoUpdater", "CheckAtStartup", "false");
    set("Folders", "Bios", &bios_dir.to_string_lossy());
    if let Some(b) = bios {
        set("Filenames", "BIOS", b);
    }
    set("MemoryCards", "Slot1_Enable", "true");
    set("MemoryCards", "Slot1_Filename", CARD);
    // Off: PCSX2 would make an ordinary 8 MB card for slot 2, and saves on it
    // wouldn't sync.
    set("MemoryCards", "Slot2_Enable", "false");
    set("EmuCore", "EnableFastBoot", "true");
    set("EmuCore", "SaveStateOnShutdown", "false");
    set("InputSources", "SDL", "true");

    // Both pads, by position: SDL's south button is Cross, whatever the
    // controller prints on it. The same table PCSX2's own automatic mapping
    // uses (`s_sdl_generic_binding_button_mapping`).
    for (pad, sdl) in [("Pad1", "SDL-0"), ("Pad2", "SDL-1")] {
        set(pad, "Type", "DualShock2");
        for (key, input) in [
            ("Up", "DPadUp"),
            ("Right", "DPadRight"),
            ("Down", "DPadDown"),
            ("Left", "DPadLeft"),
            ("Triangle", "FaceNorth"),
            ("Circle", "FaceEast"),
            ("Cross", "FaceSouth"),
            ("Square", "FaceWest"),
            ("Select", "Back"),
            ("Start", "Start"),
            ("L1", "LeftShoulder"),
            ("R1", "RightShoulder"),
            ("L2", "+LeftTrigger"),
            ("R2", "+RightTrigger"),
            ("L3", "LeftStick"),
            ("R3", "RightStick"),
            ("LUp", "-LeftY"),
            ("LRight", "+LeftX"),
            ("LDown", "+LeftY"),
            ("LLeft", "-LeftX"),
            ("RUp", "-RightY"),
            ("RRight", "+RightX"),
            ("RDown", "+RightY"),
            ("RLeft", "-RightX"),
            ("LargeMotor", "LargeMotor"),
            ("SmallMotor", "SmallMotor"),
        ] {
            set(pad, key, &format!("{sdl}/{input}"));
        }
    }

    // Hold Select, as in RetroArch games (`padprofile::HOTKEYS`): bottom face
    // quits, top face opens the menu, shoulders load and save a state, the
    // right trigger fast-forwards while held.
    let combo = |input: &str| format!("SDL-0/Back & SDL-0/{input}");
    set("Hotkeys", "ShutdownVM", &combo("FaceSouth"));
    set("Hotkeys", "OpenPauseMenu", &combo("FaceNorth"));
    set("Hotkeys", "LoadStateFromSlot", &combo("LeftShoulder"));
    set("Hotkeys", "SaveStateToSlot", &combo("RightShoulder"));
    set("Hotkeys", "HoldTurbo", &combo("+RightTrigger"));
    set("Hotkeys", "PreviousSaveStateSlot", &combo("DPadLeft"));
    set("Hotkeys", "NextSaveStateSlot", &combo("DPadRight"));
    out
}

/// The app-owned settings as `Section.Key = value` lines, for a launch record's
/// config version. The BIOS folder is this computer's path and is left out.
pub fn config_lines(bios_dir: &Path) -> Vec<String> {
    settings(bios_dir, pick_bios(bios_dir).as_deref())
        .into_iter()
        .filter(|(s, k, _)| !(*s == "Folders" && k == "Bios"))
        .map(|(s, k, v)| format!("{s}.{k} = {v}"))
        .collect()
}

/// `text` with each `(section, key, value)` set, everything else kept.
///
/// A key the app owns is written once: a repeat of it (PCSX2 allows several
/// bindings per key) is dropped, so the app's binding is the only one.
pub fn set_ini(text: &str, values: &[(&'static str, String, String)]) -> String {
    // Sections in order, each with its lines. The lines before any header
    // belong to an unnamed first section.
    let mut sections: Vec<(Option<String>, Vec<String>)> = vec![(None, Vec::new())];
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            sections.push((Some(t[1..t.len() - 1].trim().to_owned()), Vec::new()));
        } else if let Some(last) = sections.last_mut() {
            last.1.push(line.to_owned());
        }
    }
    let key_of = |line: &str| -> Option<String> {
        let t = line.trim();
        if t.starts_with(';') || t.starts_with('#') {
            return None;
        }
        t.split_once('=').map(|(k, _)| k.trim().to_owned())
    };
    for (section, key, value) in values {
        let line = format!("{key} = {value}");
        let at = sections.iter().position(|(n, _)| n.as_deref() == Some(section));
        let lines = match at {
            Some(i) => &mut sections[i].1,
            None => {
                sections.push((Some((*section).to_owned()), Vec::new()));
                &mut sections.last_mut().expect("just pushed").1
            }
        };
        let mut placed = false;
        lines.retain_mut(|l| {
            if key_of(l).as_deref() != Some(key.as_str()) {
                return true;
            }
            if placed {
                return false;
            }
            placed = true;
            *l = line.clone();
            true
        });
        if !placed {
            // After the last line with something on it, so a blank line
            // between sections stays between sections.
            let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
            lines.insert(end, line);
        }
    }
    let mut out = String::new();
    for (name, lines) in &sections {
        if let Some(n) = name {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(&format!("[{n}]\n"));
        }
        let end = lines.iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
        for l in &lines[..end] {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// Write the app's settings into PCSX2's ini, and make sure the memory card
/// exists and is formatted. Returns the card's folder.
pub fn configure(dir: &Path, bios_dir: &Path) -> Result<PathBuf> {
    // In full: PCSX2 reads a relative folder against its own data folder, so
    // `./library/...` from the app's point of view pointed nowhere.
    let bios_dir = &full(bios_dir);
    let inis = dir.join("inis");
    std::fs::create_dir_all(&inis)?;
    let ini = inis.join("PCSX2.ini");
    let old = std::fs::read_to_string(&ini).unwrap_or_default();
    let want = settings(bios_dir, pick_bios(bios_dir).as_deref());
    if !holds(&old, &want) {
        std::fs::write(&ini, set_ini(&old, &want)).with_context(|| format!("writing {}", ini.display()))?;
    }
    let card = dir.join("memcards").join(CARD);
    seed_card(&card)?;
    Ok(card)
}

/// `path` from the root, as PCSX2 needs it.
fn full(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `text` already holds every one of `values`, whatever else is in it
/// and however it's laid out. PCSX2 rewrites its ini when it quits (two blank
/// lines between sections, its window position added), so comparing text
/// called every launch after the first a change, and the checklist asked to
/// set up again.
pub fn holds(text: &str, values: &[(&'static str, String, String)]) -> bool {
    let mut section = String::new();
    let mut have: std::collections::HashMap<(String, String), String> = std::collections::HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_owned();
        } else if let Some((k, v)) = line.split_once('=') {
            have.entry((section.clone(), k.trim().to_owned())).or_insert_with(|| v.trim().to_owned());
        }
    }
    values.iter().all(|(s, k, v)| have.get(&((*s).to_owned(), k.clone())) == Some(v))
}

/// Whether PCSX2's settings already hold everything the app owns and the
/// card is formatted: what `configure` would leave unchanged.
pub fn configured(dir: &Path, bios_dir: &Path) -> bool {
    let bios_dir = &full(bios_dir);
    let old = std::fs::read_to_string(dir.join("inis/PCSX2.ini")).unwrap_or_default();
    let want = settings(bios_dir, pick_bios(bios_dir).as_deref());
    let card = dir.join("memcards").join(CARD).join("_pcsx2_superblock");
    holds(&old, &want) && std::fs::read(card).is_ok_and(|b| b.starts_with(b"Sony PS2 Memory Card Format"))
}

/// Create the folder card, or format one PCSX2 left unformatted, without
/// touching the game folders inside it.
pub fn seed_card(card: &Path) -> Result<()> {
    std::fs::create_dir_all(card).with_context(|| format!("creating {}", card.display()))?;
    let sb = card.join("_pcsx2_superblock");
    let formatted = std::fs::read(&sb).is_ok_and(|b| b.starts_with(b"Sony PS2 Memory Card Format"));
    if !formatted {
        std::fs::write(&sb, CARD_SUPERBLOCK).with_context(|| format!("writing {}", sb.display()))?;
    }
    Ok(())
}

/// The command that plays `rom`: fullscreen, no PCSX2 window around it, and
/// PCSX2 quits when the game is shut down.
pub fn command(program: &Path, rom: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.args(["-nogui", "-fullscreen", "--"]).arg(rom);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("moose-standalone-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn set_ini_replaces_adds_and_keeps_the_rest() {
        let old = "[UI]\nTheme = darkfusion\nConfirmShutdown = true\n\n[EmuCore]\nEnableCheats = true\n";
        let out = set_ini(
            old,
            &[
                ("UI", "ConfirmShutdown".into(), "false".into()),
                ("UI", "SettingsVersion".into(), "1".into()),
                ("Hotkeys", "ShutdownVM".into(), "SDL-0/Back & SDL-0/FaceSouth".into()),
            ],
        );
        assert_eq!(
            out,
            "[UI]\nTheme = darkfusion\nConfirmShutdown = false\nSettingsVersion = 1\n\n\
             [EmuCore]\nEnableCheats = true\n\n[Hotkeys]\nShutdownVM = SDL-0/Back & SDL-0/FaceSouth\n"
        );
    }

    #[test]
    fn settings_still_count_after_pcsx2_rewrites_the_file() {
        let values = settings(Path::new("/lib/bios/PS2"), Some("SCPH-39001.bin"));
        let ours = set_ini("", &values);
        // What PCSX2 leaves when it quits: two blank lines between sections,
        // and its window position added.
        let rewritten = ours.replace("\n\n[", "\n\n\n[").replacen(
            "HideMouseCursor = true\n",
            "HideMouseCursor = true\nMainWindowGeometry = AdnQywAD\nMainWindowState = AAAA\n",
            1,
        );
        assert_ne!(rewritten, ours);
        assert!(holds(&rewritten, &values));
        // A value that really changed does count.
        let changed = rewritten.replace("ConfirmShutdown = false", "ConfirmShutdown = true");
        assert!(!holds(&changed, &values));
        // So does a binding of ours that isn't the first one any more.
        let shadowed = rewritten.replace("Cross = SDL-0/FaceSouth", "Cross = Keyboard/Z\nCross = SDL-0/FaceSouth");
        assert!(!holds(&shadowed, &values));
    }

    #[test]
    fn set_ini_leaves_one_binding_for_a_key_it_owns() {
        let old = "[Pad1]\nCross = Keyboard/Z\nCross = SDL-0/FaceEast\nCircle = Keyboard/X\n";
        let out = set_ini(old, &[("Pad1", "Cross".into(), "SDL-0/FaceSouth".into())]);
        assert_eq!(out, "[Pad1]\nCross = SDL-0/FaceSouth\nCircle = Keyboard/X\n");
    }

    #[test]
    fn set_ini_is_stable() {
        let values = settings(Path::new("/lib/bios/PS2"), Some("SCPH-39001.bin"));
        let once = set_ini("", &values);
        assert_eq!(set_ini(&once, &values), once);
        assert!(once.contains("[UI]\nSettingsVersion = 1\n"));
        assert!(once.contains("Bios = /lib/bios/PS2\n"));
        assert!(once.contains("Slot1_Filename = Moose.ps2\n"));
        assert!(once.contains("Slot2_Enable = false\n"));
    }

    #[test]
    fn the_pad_is_bound_by_position() {
        let values = settings(Path::new("/b"), None);
        let get = |s: &str, k: &str| {
            values.iter().find(|(sec, key, _)| *sec == s && key == k).map(|(_, _, v)| v.as_str())
        };
        assert_eq!(get("Pad1", "Cross"), Some("SDL-0/FaceSouth"));
        assert_eq!(get("Pad1", "Circle"), Some("SDL-0/FaceEast"));
        assert_eq!(get("Pad2", "Triangle"), Some("SDL-1/FaceNorth"));
        assert_eq!(get("Pad1", "L2"), Some("SDL-0/+LeftTrigger"));
        assert_eq!(get("Pad1", "LUp"), Some("SDL-0/-LeftY"));
        assert_eq!(get("Hotkeys", "ShutdownVM"), Some("SDL-0/Back & SDL-0/FaceSouth"));
        assert_eq!(get("Filenames", "BIOS"), None, "no BIOS named when none is there");
    }

    #[test]
    fn pick_bios_prefers_the_us_one_then_any() {
        let d = tmp("bios");
        for f in ["SCPH-10000.bin", "SCPH-39001.bin", "SCPH-39001.NVM"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        assert_eq!(pick_bios(&d).as_deref(), Some("SCPH-39001.bin"));
        std::fs::remove_file(d.join("SCPH-39001.bin")).unwrap();
        assert_eq!(pick_bios(&d).as_deref(), Some("SCPH-10000.bin"));
        std::fs::remove_file(d.join("SCPH-10000.bin")).unwrap();
        std::fs::write(d.join("odd.bin"), b"x").unwrap();
        assert_eq!(pick_bios(&d).as_deref(), Some("odd.bin"));
        assert_eq!(pick_bios(&d.join("missing")), None);
    }

    #[test]
    fn a_new_card_is_formatted_and_an_old_one_keeps_its_saves() {
        let d = tmp("card");
        let card = d.join(CARD);
        seed_card(&card).unwrap();
        let sb = std::fs::read(card.join("_pcsx2_superblock")).unwrap();
        assert_eq!(sb.len(), 8192);
        assert!(sb.starts_with(b"Sony PS2 Memory Card Format 1.2.0.0"));

        // An unformatted card, as PCSX2's own "new card" leaves it, with a
        // save that sync brought in.
        std::fs::write(card.join("_pcsx2_superblock"), b"").unwrap();
        std::fs::create_dir_all(card.join("BASCUS-97268RATCHET")).unwrap();
        std::fs::write(card.join("BASCUS-97268RATCHET/save"), b"progress").unwrap();
        seed_card(&card).unwrap();
        assert!(std::fs::read(card.join("_pcsx2_superblock")).unwrap().starts_with(b"Sony PS2"));
        assert_eq!(std::fs::read(card.join("BASCUS-97268RATCHET/save")).unwrap(), b"progress");

        // A formatted one is left exactly as it is.
        let mut custom = CARD_SUPERBLOCK.to_vec();
        custom[0x154] = 0x7f;
        std::fs::write(card.join("_pcsx2_superblock"), &custom).unwrap();
        seed_card(&card).unwrap();
        assert_eq!(std::fs::read(card.join("_pcsx2_superblock")).unwrap(), custom);
    }

    #[test]
    fn configure_writes_the_ini_and_the_card() {
        let d = tmp("configure");
        let bios = d.join("bios");
        std::fs::create_dir_all(&bios).unwrap();
        std::fs::write(bios.join("SCPH-70004.BIN"), b"x").unwrap();
        std::fs::create_dir_all(d.join("inis")).unwrap();
        std::fs::write(d.join("inis/PCSX2.ini"), "[UI]\nTheme = darkfusion\n").unwrap();
        let card = configure(&d, &bios).unwrap();
        let ini = std::fs::read_to_string(d.join("inis/PCSX2.ini")).unwrap();
        assert!(ini.contains("Theme = darkfusion"));
        assert!(ini.contains("BIOS = SCPH-70004.BIN"));
        assert_eq!(card, d.join("memcards").join(CARD));
        assert!(card.join("_pcsx2_superblock").is_file());
    }

    #[test]
    fn the_command_plays_fullscreen_with_no_window_around_it() {
        let cmd = command(Path::new("/e/PCSX2"), Path::new("/r/Ico (USA).chd"));
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-nogui", "-fullscreen", "--", "/r/Ico (USA).chd"]);
    }

    #[test]
    fn ps2_is_pcsx2_where_the_app_runs_it() {
        let found = for_platform("ps2").map(|e| e.id);
        if matches!(std::env::consts::OS, "macos" | "windows") {
            assert_eq!(found, Some("pcsx2"));
        } else {
            assert_eq!(found, None);
        }
        assert!(for_platform("snes").is_none());
        assert!(by_id("pcsx2").is_some());
    }
}

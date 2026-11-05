//! Standalone emulators the desktop app installs, sets up and starts itself,
//! for systems RetroArch can't run well: PCSX2 for the PS2, PPSSPP for the PSP
//! (Frank, 2026-11-05: PPSSPP itself, not its RetroArch core). See
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
    /// Download size in bytes of the pinned release: (macOS, Windows, Linux).
    /// Shown in the launch checklist before anything is fetched.
    pub sizes: (u64, u64, u64),
    /// The BIOS folder it needs under the system folder and the server's
    /// `bios/` (`PS2`), or None when it needs none.
    pub bios: Option<&'static str>,
}

pub const PCSX2: Emulator = Emulator {
    id: "pcsx2",
    name: "PCSX2",
    version: "v2.8.2",
    platforms: &["ps2"],
    family: Some("pcsx2"),
    sizes: (24_377_200, 25_670_075, 60_119_544),
    bios: Some("PS2"),
};

/// Its saves sync as the `ppsspp` family the Android handhelds already use, so
/// a PSP save carries between the Nova and here.
pub const PPSSPP: Emulator = Emulator {
    id: "ppsspp",
    name: "PPSSPP",
    version: "v1.20.4",
    platforms: &["psp"],
    family: Some("ppsspp"),
    sizes: (37_389_023, 20_390_334, 50_689_748),
    bios: None,
};

pub const ALL: &[&Emulator] = &[&PCSX2, &PPSSPP];

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
    // Desktop Linux on x86_64, which is what these are fetched for: not
    // KNULLI, which is `target_os = "linux"` on an ARM handheld.
    let scheme = crate::platform::current().scheme();
    let desktop_linux = scheme == "linux" && std::env::consts::ARCH == "x86_64";
    if !(matches!(scheme, "macos" | "windows") || desktop_linux) {
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
    match std::env::consts::OS {
        "windows" => emu.sizes.1,
        "linux" => emu.sizes.2,
        _ => emu.sizes.0,
    }
}

/// The version installed in `dir`, as the app recorded it when it installed it.
pub fn installed_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(VERSION_FILE)).ok().map(|s| s.trim().to_owned())
}

/// Beside the program: which pinned release the app put there.
const VERSION_FILE: &str = ".moose-version";

/// The Linux download, kept under one name whatever the version.
fn linux_appimage(emu: &Emulator) -> String {
    format!("{}.AppImage", emu.name)
}

/// Where the emulator keeps its settings and saves in its portable mode.
///
/// PCSX2 (`inis/`, `memcards/`): beside the program on macOS and Windows. On
/// Linux the program is an AppImage, which runs from a read-only mount, so
/// PCSX2 2.8 uses a `PCSX2` folder beside the AppImage instead
/// (`EmuFolders::SetDataDirectory`, the `APPIMAGE` case).
///
/// PPSSPP 1.20 (its "memory stick": `PSP/SYSTEM/ppsspp.ini`, `PSP/SAVEDATA`):
/// a `memstick` folder beside the exe on Windows, which the zip is portable by
/// (no `installed.txt`). On macOS and Linux it is `$HOME/.config/ppsspp`
/// (`NativeApp.cpp`, `DarwinFileSystemServices.mm`), so `command` starts it
/// with HOME set to its own folder.
pub fn data_root(emu: &Emulator, dir: &Path) -> PathBuf {
    match (emu.id, std::env::consts::OS) {
        ("pcsx2", "linux") => dir.join("PCSX2"),
        ("ppsspp", "windows") => dir.join("memstick"),
        ("ppsspp", _) => dir.join(".config").join("ppsspp"),
        _ => dir.to_path_buf(),
    }
}

/// PCSX2's memory card folder, which the save sync reads and writes.
pub fn card_dir(dir: &Path) -> PathBuf {
    data_root(&PCSX2, dir).join("memcards").join(CARD)
}

/// What the save sync reads and writes for this emulator: PCSX2's folder card,
/// PPSSPP's memory stick (which holds `PSP/SAVEDATA`).
pub fn save_root(emu: &Emulator, dir: &Path) -> PathBuf {
    match emu.id {
        "pcsx2" => card_dir(dir),
        _ => data_root(emu, dir),
    }
}

/// The program to start, if it's installed.
pub fn program(emu: &Emulator, dir: &Path) -> Option<PathBuf> {
    let p = match (emu.id, std::env::consts::OS) {
        (_, "linux") => dir.join(linux_appimage(emu)),
        ("pcsx2", "macos") => dir.join("PCSX2.app/Contents/MacOS/PCSX2"),
        ("pcsx2", _) => dir.join("pcsx2-qt.exe"),
        ("ppsspp", "macos") => dir.join("PPSSPPSDL.app/Contents/MacOS/PPSSPPSDL"),
        ("ppsspp", _) => dir.join("PPSSPPWindows64.exe"),
        _ => return None,
    };
    p.is_file().then_some(p)
}

/// The release file for this OS and the version pinned above.
fn asset(emu: &Emulator) -> Result<(String, &'static str)> {
    let v = emu.version;
    let x86_64 = std::env::consts::ARCH == "x86_64";
    let (file, kind) = match (emu.id, std::env::consts::OS) {
        ("pcsx2", "macos") => (format!("pcsx2-{v}-macos-Qt.tar.xz"), "tar.xz"),
        ("pcsx2", "windows") => (format!("pcsx2-{v}-windows-x64-Qt.7z"), "7z"),
        ("pcsx2", "linux") if x86_64 => (format!("pcsx2-{v}-linux-appimage-x64-Qt.AppImage"), "AppImage"),
        ("ppsspp", "macos") => (format!("PPSSPPSDL-macOS-{v}.zip"), "zip"),
        ("ppsspp", "windows") => (format!("PPSSPP-{v}-Windows-x64.zip"), "zip"),
        ("ppsspp", "linux") if x86_64 => (format!("PPSSPP-{v}-anylinux-x86_64.AppImage"), "AppImage"),
        (_, os) => bail!("{} is not set up for {os} yet", emu.name),
    };
    let repo = if emu.id == "ppsspp" { "hrydgard/ppsspp" } else { "PCSX2/pcsx2" };
    Ok((format!("https://github.com/{repo}/releases/download/{v}/{file}"), kind))
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

    if kind == "AppImage" {
        // Linux: the AppImage is the program; nothing to unpack. Portable mode
        // there is on the command line (see `command`).
        let target = dir.join(linux_appimage(emu));
        std::fs::rename(&download, &target).with_context(|| format!("moving {} into place", target.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                .with_context(|| format!("making {} runnable", target.display()))?;
        }
    } else {
        let staging = dir.join(".incoming");
        if staging.exists() {
            std::fs::remove_dir_all(&staging).ok();
        }
        std::fs::create_dir_all(&staging)?;
        unpack(&download, kind, &staging)?;
        place(emu, &staging, dir)?;
        std::fs::remove_dir_all(&staging).ok();
        std::fs::remove_file(&download).ok();

        // Portable mode: PCSX2 looks for this beside the .app on macOS and
        // beside the .exe on Windows (`EmuFolders::SetAppRoot`). PPSSPP's zip
        // is portable as it comes on Windows, and HOME makes it so elsewhere.
        let marker = dir.join("portable.txt");
        if emu.id == "pcsx2" && !marker.exists() {
            std::fs::write(&marker, "")?;
        }
    }
    std::fs::write(dir.join(VERSION_FILE), emu.version)?;
    program(emu, dir).with_context(|| format!("{} unpacked, but its program isn't in {}", emu.name, dir.display()))
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
        // macOS: `ditto`, which keeps the symlinks inside an app bundle's
        // frameworks that a plain unzip flattens, and with them the signature.
        "zip" if cfg!(target_os = "macos") => {
            let out = std::process::Command::new("ditto")
                .args(["-x", "-k"])
                .arg(archive)
                .arg(into)
                .output()
                .context("running ditto")?;
            if !out.status.success() {
                bail!("unpacking {}: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim());
            }
        }
        "zip" => {
            let file = std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
            zip::ZipArchive::new(file)
                .and_then(|mut z| z.extract(into))
                .with_context(|| format!("unpacking {}", archive.display()))?;
        }
        other => bail!("no unpacker for .{other}"),
    }
    Ok(())
}

/// Move the unpacked program into `dir`.
fn place(emu: &Emulator, staging: &Path, dir: &Path) -> Result<()> {
    if std::env::consts::OS == "macos" {
        // The archive holds one .app (PCSX2 names it after the version); it
        // goes in under the name `program` looks for.
        let app = find(staging, &|p| p.extension().is_some_and(|e| e == "app") && p.is_dir())
            .with_context(|| format!("no .app in the {} download", emu.name))?;
        let name = if emu.id == "ppsspp" { "PPSSPPSDL.app" } else { "PCSX2.app" };
        let target = dir.join(name);
        if target.exists() {
            std::fs::remove_dir_all(&target).with_context(|| format!("removing the old {}", target.display()))?;
        }
        std::fs::rename(&app, &target).with_context(|| format!("moving {} into place", app.display()))?;
        return Ok(());
    }
    let want = if emu.id == "ppsspp" { "PPSSPPWindows64.exe" } else { "pcsx2-qt.exe" };
    let exe = find(staging, &|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(want)))
        .with_context(|| format!("no {want} in the download"))?;
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
pub fn config_lines(emu: &Emulator, bios_dir: &Path) -> Vec<String> {
    owned(emu, bios_dir)
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

/// The settings the app owns in PPSSPP's `ppsspp.ini`: no first-run screen
/// and no offer to update the pinned version. Fullscreen and leaving to the
/// app from the pause menu are on its command line (`command`), not saved.
pub fn ppsspp_settings() -> Vec<(&'static str, String, String)> {
    vec![
        ("General", "FirstRun".to_owned(), "False".to_owned()),
        ("General", "CheckForNewVersion".to_owned(), "False".to_owned()),
    ]
}

/// The settings the app owns for this emulator.
fn owned(emu: &Emulator, bios_dir: &Path) -> Vec<(&'static str, String, String)> {
    match emu.id {
        "ppsspp" => ppsspp_settings(),
        _ => {
            let bios_dir = &full(bios_dir);
            settings(bios_dir, pick_bios(bios_dir).as_deref())
        }
    }
}

/// The emulator's own settings file.
fn ini_path(emu: &Emulator, dir: &Path) -> PathBuf {
    match emu.id {
        "ppsspp" => data_root(emu, dir).join("PSP/SYSTEM/ppsspp.ini"),
        _ => data_root(emu, dir).join("inis/PCSX2.ini"),
    }
}

/// Write the app's settings into the emulator's ini, and make sure its save
/// folder exists: PCSX2's memory card, formatted, or PPSSPP's PSP/SAVEDATA.
/// Returns what the save sync reads (`save_root`).
pub fn configure(emu: &Emulator, dir: &Path, bios_dir: &Path) -> Result<PathBuf> {
    let ini = ini_path(emu, dir);
    if let Some(parent) = ini.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let old = std::fs::read_to_string(&ini).unwrap_or_default();
    let want = owned(emu, bios_dir);
    if !holds(&old, &want) {
        std::fs::write(&ini, set_ini(&old, &want)).with_context(|| format!("writing {}", ini.display()))?;
    }
    if emu.id == "pcsx2" {
        seed_card(&card_dir(dir))?;
    } else {
        let saves = data_root(emu, dir).join("PSP/SAVEDATA");
        std::fs::create_dir_all(&saves).with_context(|| format!("creating {}", saves.display()))?;
    }
    Ok(save_root(emu, dir))
}

/// `path` from the root, as PCSX2 needs it: it reads a relative folder
/// against its own data folder, so `./library/...` pointed nowhere.
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

/// Whether the emulator's settings already hold everything the app owns and
/// its save folder is ready: what `configure` would leave unchanged.
pub fn configured(emu: &Emulator, dir: &Path, bios_dir: &Path) -> bool {
    let old = std::fs::read_to_string(ini_path(emu, dir)).unwrap_or_default();
    let ready = if emu.id == "pcsx2" {
        std::fs::read(card_dir(dir).join("_pcsx2_superblock")).is_ok_and(|b| b.starts_with(b"Sony PS2 Memory Card Format"))
    } else {
        data_root(emu, dir).join("PSP/SAVEDATA").is_dir()
    };
    holds(&old, &owned(emu, bios_dir)) && ready
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

/// The command that plays `rom` in `emu`, installed in `dir`.
///
/// PCSX2: fullscreen, no window around it, and it quits when the game is shut
/// down. On Linux portable mode is `-portable`: no file beside an AppImage's
/// program can be read from inside its mount.
///
/// PPSSPP: fullscreen, and Exit in its pause menu quits back to the app
/// rather than to PPSSPP's own game browser. On macOS and Linux HOME (and on
/// Linux XDG_CONFIG_HOME) is its folder, which is what makes it portable
/// there (`data_root`).
pub fn command(emu: &Emulator, program: &Path, rom: &Path, dir: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    if emu.id == "ppsspp" {
        if std::env::consts::OS != "windows" {
            let home = full(dir);
            cmd.env("HOME", &home);
            if std::env::consts::OS == "linux" {
                cmd.env("XDG_CONFIG_HOME", home.join(".config"));
            }
        }
        cmd.args(["--fullscreen", "--pause-menu-exit"]).arg(full(rom));
        return cmd;
    }
    if std::env::consts::OS == "linux" {
        cmd.arg("-portable");
    }
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
        let root = data_root(&PCSX2, &d);
        std::fs::create_dir_all(root.join("inis")).unwrap();
        std::fs::write(root.join("inis/PCSX2.ini"), "[UI]\nTheme = darkfusion\n").unwrap();
        let card = configure(&PCSX2, &d, &bios).unwrap();
        let ini = std::fs::read_to_string(root.join("inis/PCSX2.ini")).unwrap();
        assert!(ini.contains("Theme = darkfusion"));
        assert!(ini.contains("BIOS = SCPH-70004.BIN"));
        assert_eq!(card, root.join("memcards").join(CARD));
        assert!(configured(&PCSX2, &d, &bios), "configure's result reads back as configured");
        assert!(card.join("_pcsx2_superblock").is_file());
    }

    /// PPSSPP's settings go in its memory stick, PSP/SAVEDATA is made for the
    /// save sync, and the user's own settings in the ini stay.
    #[test]
    fn ppsspp_is_set_up_in_its_own_memory_stick() {
        let d = tmp("ppsspp");
        let ini = data_root(&PPSSPP, &d).join("PSP/SYSTEM/ppsspp.ini");
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(&ini, "[Graphics]\nInternalResolution = 3\n").unwrap();
        assert!(!configured(&PPSSPP, &d, &d));
        let saves = configure(&PPSSPP, &d, &d).unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("InternalResolution = 3"), "the user's setting was lost:\n{text}");
        assert!(text.contains("FirstRun = False") && text.contains("CheckForNewVersion = False"));
        assert_eq!(saves, data_root(&PPSSPP, &d));
        assert!(saves.join("PSP/SAVEDATA").is_dir(), "the save sync needs PSP/SAVEDATA");
        assert!(configured(&PPSSPP, &d, &d));
    }

    /// PPSSPP keeps its data under HOME on macOS and Linux, so the command
    /// points HOME at its own folder; it plays fullscreen and its pause menu's
    /// Exit comes back to the app.
    #[test]
    fn ppsspp_runs_fullscreen_with_its_home_in_its_folder() {
        let cmd = command(&PPSSPP, Path::new("/e/ppsspp/PPSSPP"), Path::new("/r/Daxter (USA).iso"), Path::new("/e/ppsspp"));
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let rom = full(Path::new("/r/Daxter (USA).iso")).to_string_lossy().into_owned();
        assert_eq!(args, ["--fullscreen", "--pause-menu-exit", rom.as_str()]);
        let home = cmd.get_envs().find(|(k, _)| *k == "HOME").and_then(|(_, v)| v);
        if std::env::consts::OS == "windows" {
            assert_eq!(home, None, "Windows is portable by its memstick folder, not HOME");
        } else {
            assert_eq!(home, Some(std::ffi::OsStr::new("/e/ppsspp")));
        }
    }

    #[test]
    fn the_command_plays_fullscreen_with_no_window_around_it() {
        let cmd = command(&PCSX2, Path::new("/e/PCSX2"), Path::new("/r/Ico (USA).chd"), Path::new("/e"));
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let want: &[&str] = if std::env::consts::OS == "linux" {
            &["-portable", "-nogui", "-fullscreen", "--", "/r/Ico (USA).chd"]
        } else {
            &["-nogui", "-fullscreen", "--", "/r/Ico (USA).chd"]
        };
        assert_eq!(args, want);
    }

    #[test]
    fn ps2_is_pcsx2_where_the_app_runs_it() {
        let found = for_platform("ps2").map(|e| e.id);
        let scheme = crate::platform::current().scheme();
        if matches!(scheme, "macos" | "windows") || (scheme == "linux" && std::env::consts::ARCH == "x86_64") {
            assert_eq!(found, Some("pcsx2"));
        } else {
            assert_eq!(found, None);
        }
        assert!(for_platform("snes").is_none());
        assert!(by_id("pcsx2").is_some());
        let psp = for_platform("psp").map(|e| e.id);
        assert_eq!(psp, found.map(|_| "ppsspp"), "PSP is PPSSPP wherever PS2 is PCSX2");
    }
}

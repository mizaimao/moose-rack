//! Applying, reverting, and — the one that earns its keep — reading back.
//!
//! Every patch here is a list of **steps**, and every step is one of two
//! things: a marked block inside a text config, or a file we own. That is not
//! a simplification for its own sake. It came out of placing all of these by
//! hand first, and finding that KNULLI already provides a `/userdata`
//! override for nearly everything — so the work was never the change, it was
//! knowing which file survives an upgrade.
//!
//! Because a step can be asked whether it is already satisfied, a patch can
//! report which of its options the device is actually sitting at, or that it
//! is at none of them because an update moved the ground. That third answer
//! is why this is not a list of booleans.
//!
//! Reverting is not a special path. "off" is an option like any other, and its
//! steps are "clear that block" and "put that file back", so the machinery
//! that applies is the machinery that undoes.

use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// Where everything lives. A struct rather than constants so the tests can
/// point the whole engine at a temporary directory and watch what it writes.
#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Paths { root: PathBuf::from("/") }
    }
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Paths { root: root.into() }
    }

    fn at(&self, rest: &str) -> PathBuf {
        self.root.join(rest)
    }

    /// The one file KNULLI reads for almost every setting, and the one that
    /// survives an upgrade.
    pub fn knulli_conf(&self) -> PathBuf {
        self.at("userdata/system/knulli.conf")
    }

    /// Which KNULLI this is. One line, e.g. `scarab 2026/05/10 22:54`.
    ///
    /// On the squashfs, so an OS update replaces it — which is the point. See
    /// `crate::knulli`.
    pub fn knulli_version(&self) -> PathBuf {
        self.at("usr/share/knulli/knulli.version")
    }

    pub fn boot_custom(&self) -> PathBuf {
        self.at("boot/boot-custom.sh")
    }

    pub fn user_startup(&self) -> PathBuf {
        self.at("userdata/system/custom.sh")
    }

    pub fn trigger_conf(&self) -> PathBuf {
        self.at("userdata/system/configs/multimedia_keys.conf")
    }

    pub fn es_input(&self) -> PathBuf {
        self.at("userdata/system/configs/emulationstation/es_input.cfg")
    }

    pub fn decoration(&self, rest: &str) -> PathBuf {
        self.at(&format!("userdata/decorations/moose/systems/{rest}"))
    }

    pub fn shader(&self, rest: &str) -> PathBuf {
        self.at(&format!("userdata/shaders/moose/{rest}"))
    }

    /// A shader *set*. configgen looks here before the stock library, and
    /// resolves the set's presets relative to `userdata/shaders` — which is
    /// how the loaded preset ends up in our folder, which is what keeps
    /// Hotkey + D-pad cycling to four presets instead of seven hundred.
    pub fn shaderset(&self, name: &str) -> PathBuf {
        self.at(&format!("userdata/shaders/configs/{name}/rendering-defaults.yml"))
    }

    /// The blank logo, **on /boot**.
    ///
    /// Not in /userdata, which is where it started and where it did not work:
    /// the hook that installs it runs as `S00bootcustom`, and `S02resize` is
    /// what mounts /userdata. At S00 there is no /userdata to read from, so
    /// the hook found nothing and EmulationStation came back with its own
    /// logo after every reboot. /boot is already mounted — the init script
    /// reads the hook itself from there — and has 2.1 GB spare.
    pub fn blank_logo(&self) -> PathBuf {
        self.at("boot/moose-blank-logo.png")
    }

    /// The image EmulationStation draws while it loads. On the squashfs, so
    /// replacing it only lasts until the next boot — which is what
    /// `boot-custom.sh` is for.
    pub fn es_logo(&self) -> PathBuf {
        self.at("usr/share/emulationstation/resources/logo.png")
    }

    /// The flag that turns the evmapy guard on, **on /boot**.
    ///
    /// Same reason as the blank logo and the GPU marker: the hook runs as
    /// `S00bootcustom` and `S02resize` is what mounts /userdata, so a flag
    /// kept there is one the hook cannot read.
    pub fn evmapy_flag(&self) -> PathBuf {
        self.at("boot/moose-evmapy-guard")
    }

    /// Which Mali blob the boot hook should install, **on /boot**.
    ///
    /// Same reason as the blank logo: the hook runs as `S00bootcustom` and
    /// `S02resize` is what mounts /userdata, so a marker kept there is one the
    /// hook cannot read. This switcher had never once worked at boot.
    pub fn gpu_choice(&self) -> PathBuf {
        self.at("boot/moose-gpu")
    }

    /// Where the hook expects to find a driver blob. Too big to compile in —
    /// 43 MB and 56 MB — so they are placed once and referred to by name.
    pub fn gpu_blob(&self, which: &str) -> PathBuf {
        self.at(&format!("boot/moose-libmali-{which}.so"))
    }

    /// KNULLI's own trigger file, which the one in /userdata replaces.
    pub fn stock_triggers(&self) -> PathBuf {
        self.at("etc/triggerhappy/triggers.d/multimedia_keys.conf")
    }

    /// EmulationStation's own settings. Read, never written — the one thing
    /// wanted from it is which way round the face buttons are.
    pub fn es_settings(&self) -> PathBuf {
        self.at("userdata/system/configs/emulationstation/es_settings.cfg")
    }

    /// Where a file we replaced is kept, so "off" can put the original back.
    ///
    /// **Not beside the file.** They used to live as `<name>.moose-orig` next
    /// to the original, which is fine for a config file and wrong for a
    /// directory something else enumerates: RetroArch cycles the shader folder
    /// by listing it, and found the backups sitting in there. One directory,
    /// outside everything we manage, and on real storage so a backup of
    /// something in /usr survives the reboot that would restore it anyway.
    pub fn backup_for(&self, path: &Path) -> PathBuf {
        let flat: String = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .chars()
            .map(|c| if c == '/' || c == '\\' { '_' } else { c })
            .collect();
        self.at(&format!("userdata/system/moose-patch/backups/{flat}"))
    }

    pub fn profile(&self) -> PathBuf {
        self.at("userdata/system/moose-patch/profile.toml")
    }
}

/// The marker a block is wrapped in. Rewriting between a matched pair is what
/// makes a config patch idempotent, and what makes "off" possible without
/// remembering what the file looked like before.
fn open_marker(id: &str) -> String {
    format!("## moose-patch: {id}")
}
fn close_marker(id: &str) -> String {
    format!("## moose-patch: {id} end")
}

/// The body between the markers, if the block is there at all.
pub fn read_block(text: &str, id: &str) -> Option<String> {
    let open = open_marker(id);
    let close = close_marker(id);
    let mut inside = false;
    let mut body = Vec::new();
    for line in text.lines() {
        if line.trim() == close {
            return Some(body.join("\n"));
        }
        if inside {
            body.push(line);
        }
        // Checked after the close, so a one-line block cannot swallow its own
        // terminator: `close` starts with `open`, and the trim-compare above
        // would otherwise match the opener first.
        if line.trim() == open {
            inside = true;
        }
    }
    None
}

/// What a block wrote over, kept on the line that displaced it.
///
/// Distinct from the opening marker — `read_block` and `clear_block` compare a
/// trimmed line for equality with `## moose-patch: <id>`, and this is longer,
/// so it can never be mistaken for one.
fn hidden_marker(id: &str) -> String {
    format!("## moose-patch: {id} hid: ")
}

/// The `key` of a `key=value` line, ignoring comments and blanks.
fn key_of(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let key = line.split_once('=')?.0.trim();
    (!key.is_empty()).then_some(key)
}

/// The value KNULLI's reader takes for `key`: the first live `key=value` line
/// from the top of the file. See `hide_keys` for why it is the first.
pub fn effective<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find(|line| key_of(line) == Some(key))
        .and_then(|line| line.split_once('='))
        .map(|(_, value)| value.trim())
}

/// Every key in `body` reads, in `text`, as the value `body` gives it.
///
/// The block being in the file is not enough. ES keeps knulli.conf in memory
/// and writes the whole thing back, and a line it puts above ours wins.
fn in_force(text: &str, body: &str) -> bool {
    body.lines()
        .filter_map(key_of)
        .all(|key| effective(text, key) == effective(body, key))
}

/// A text file, or `None` when there is no such file.
///
/// Anything else that goes wrong is an error, including a file that is not
/// UTF-8. This used to be `read_to_string(..).unwrap_or_default()`, which made
/// one stray byte in knulli.conf, or an EIO, into an empty file, and the write
/// that followed left nothing in it but our block.
pub fn read_text(path: &Path) -> Result<Option<String>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    String::from_utf8(bytes).map(Some).map_err(|e| {
        anyhow!(
            "{} is not UTF-8 (at byte {}); it was left as it is",
            path.display(),
            e.utf8_error().valid_up_to()
        )
    })
}

/// Comment out any earlier line setting a key this block also sets.
///
/// **`knulli.conf` is first-wins, not last-wins.** `knulli-settings-get` scans
/// from the top and returns the first match, so a block appended at the end of
/// the file that repeats a key already up there is read by nothing. That is
/// not a theory: `never-sleep` wrote `system.batterysaver.extendedmode=none`
/// for weeks under a `=suspend` on line 319, the app said it was on, and the
/// handheld went on suspending after fifteen minutes.
///
/// The displaced line is kept verbatim on its marker so `unhide` can put it
/// back exactly, which is what makes turning a patch off a real undo rather
/// than a guess at what KNULLI's default was.
fn hide_keys(text: &str, id: &str, keys: &[&str]) -> String {
    if keys.is_empty() {
        return text.to_owned();
    }
    let marker = hidden_marker(id);
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match key_of(line) {
            Some(key) if keys.contains(&key) => out.push(format!("{marker}{line}")),
            _ => out.push(line.to_owned()),
        }
    }
    let mut text = out.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Put back every line this block hid.
fn unhide(text: &str, id: &str) -> String {
    let marker = hidden_marker(id);
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        match line.trim_start().strip_prefix(marker.as_str()) {
            Some(original) => out.push(original),
            None => out.push(line),
        }
    }
    let mut text = out.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// Put a block in, replacing any previous one. Appends when there was none.
pub fn set_block(text: &str, id: &str, body: &str) -> Result<String> {
    let cleared = clear_block(text, id)?;
    // Hidden before the block is appended, so the block's own lines — which
    // set the very keys being hidden — are not swept up by it.
    let keys: Vec<&str> = body.lines().filter_map(key_of).collect();
    let cleared = hide_keys(&cleared, id, &keys);
    let mut out = cleared.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&open_marker(id));
    out.push('\n');
    out.push_str(body.trim_end());
    out.push('\n');
    out.push_str(&close_marker(id));
    out.push('\n');
    Ok(out)
}

/// Take a block out, markers and all, leaving the rest of the file alone.
///
/// An opening marker with no end marker after it is refused. Everything below
/// it would otherwise count as the block and go with it, and in knulli.conf
/// that can be most of the file.
pub fn clear_block(text: &str, id: &str) -> Result<String> {
    // Whatever this block wrote over comes back first. Turning a patch off has
    // to leave the file as KNULLI had it, not merely without our lines.
    let text = &unhide(text, id);
    let open = open_marker(id);
    let close = close_marker(id);
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim() == close {
            inside = false;
            continue;
        }
        if line.trim() == open {
            inside = true;
            continue;
        }
        if !inside {
            out.push(line);
        }
    }
    if inside {
        bail!("`{open}` has no `{close}` after it, so where the block ends is unknown");
    }
    let mut text = out.join("\n");
    while text.ends_with("\n\n\n") {
        text.pop();
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// What kind of file a block lives in. It decides what "the block is there"
/// has to mean before a patch can call itself on.
#[derive(Clone, Debug)]
pub enum Form {
    /// A script, such as custom.sh. The block is the whole of the change.
    Script,
    /// knulli.conf: `key=value` lines, read first-wins. On means every key in
    /// the block is the value KNULLI's reader would find, not only that the
    /// block is in the file.
    Settings,
    /// A file in /userdata that *replaces* one KNULLI ships rather than adding
    /// to it: `multimedia_keys.conf`, whose /etc copy holds the volume, power
    /// and lid keys. So it is always that file, as it is now, plus our block.
    ///
    /// On rebuilds it from the /etc copy at every apply. Off deletes it when
    /// what is left is that copy, so /etc is read again. And it reads as on
    /// only while the rest of it matches /etc, so a KNULLI update that changes
    /// the /etc copy shows as `changed` instead of being shadowed by a frozen
    /// copy of the old one. Nothing on the device is edited by hand, so
    /// rebuilding the file loses nothing.
    Seeded(PathBuf),
}

/// Lines in a `key=value` file that point at something of ours: a key ending
/// in `key_suffix` whose value starts with `value_prefix`.
#[derive(Clone, Debug)]
pub struct NamedIn {
    pub conf: PathBuf,
    pub key_suffix: &'static str,
    pub value_prefix: &'static str,
}

impl NamedIn {
    /// Whether any live line still names one. Every live line counts, not
    /// only the first-wins one: configgen reads knulli.conf its own way, and
    /// a file anything may still load has to stay.
    fn still_named(&self) -> Result<bool> {
        let Some(text) = read_text(&self.conf)? else { return Ok(false) };
        Ok(text.lines().any(|line| {
            let Some(key) = key_of(line) else { return false };
            let value = line.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
            key.ends_with(self.key_suffix) && value.starts_with(self.value_prefix)
        }))
    }
}

/// One thing a patch does.
#[derive(Clone, Debug)]
pub enum Step {
    /// A marked block in a text config. `None` means the block should not be
    /// there — which is how "off" is written.
    Block { file: PathBuf, id: String, body: Option<String>, form: Form },
    /// A file this app owns, at a path nothing else writes to. `None` means it
    /// should not be there, and whatever was there before comes back — from
    /// `backup`, which is deliberately not inside the directory `path` lives
    /// in.
    Place { path: PathBuf, bytes: Option<&'static [u8]>, backup: PathBuf },
    /// Our file laid over one KNULLI may ship: `es_input.cfg`, ES's
    /// `logo.png`. `ours` is what we put there and `on` says whether it should
    /// be there.
    ///
    /// Off never deletes a file we did not put there. It puts the backup back
    /// if the file is ours (or gone), deletes it only if it is byte for byte
    /// ours and there is no backup, and otherwise leaves it alone. So a stock
    /// file reads as off, which is what a fresh install is.
    Cover { path: PathBuf, ours: &'static [u8], on: bool, backup: PathBuf },
    /// A file of ours that more than one patch can need at once: the shader
    /// presets and sets, which the global shader and every per-system shader
    /// name. `want` places it. Otherwise it is removed unless `named` still
    /// finds a line pointing at one of ours, so no option is ever left naming
    /// a set that was deleted from under it.
    Shared { path: PathBuf, bytes: &'static [u8], want: bool, named: NamedIn },
}

/// Write, even if the filesystem says no.
///
/// `/boot` on this device is mounted read-only, and `boot-custom.sh` lives
/// there because it is the only hook that runs before EmulationStation. So a
/// patch that has to write there gets one retry with the mount flipped, and
/// the mount is put back afterwards whatever happens.
pub(crate) fn write_through(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    match write_whole(path, bytes) {
        Err(e) if e.kind() == ErrorKind::ReadOnlyFilesystem => {}
        other => return other,
    }
    let Some(point) = mount_point_of(path) else {
        return write_whole(path, bytes);
    };
    let _ = remount(&point, "rw");
    let result = write_whole(path, bytes);
    let _ = remount(&point, "ro");
    result
}

/// Write to a new file beside `path` and rename it over, so whatever reads the
/// file, and whatever is left after a power cut, is the old one or the new
/// one and never the first half of the new one.
///
/// The temporary name starts with a dot, so a folder something lists (the
/// shader presets) does not show it for the moment it exists.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "no file name"))?;
    let temp = path.with_file_name(format!(".{}.moose-tmp", name.to_string_lossy()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // A script has to stay executable. exFAT and vfat may refuse a chmod,
        // and they fix the mode at mount time anyway.
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&temp, meta.permissions());
        }
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Remove, even if the filesystem says no.
///
/// The mirror of [`write_through`], and it was missing: turning the graphics
/// driver back to stock means deleting the marker on /boot, `fs::remove_file`
/// returned `EROFS`, and the switch silently stayed where it was.
fn remove_through(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == ErrorKind::ReadOnlyFilesystem => {}
        other => return other,
    }
    let Some(point) = mount_point_of(path) else {
        return fs::remove_file(path);
    };
    let _ = remount(&point, "rw");
    let result = fs::remove_file(path);
    let _ = remount(&point, "ro");
    result
}

fn remount(point: &str, how: &str) -> std::io::Result<()> {
    std::process::Command::new("mount")
        .args(["-o", &format!("remount,{how}"), point])
        .status()
        .map(|_| ())
}

/// The longest mount point in /proc/mounts that this path sits under.
fn mount_point_of(path: &Path) -> Option<String> {
    let mounts = fs::read_to_string("/proc/mounts").ok()?;
    let target = path.to_str()?;
    mounts
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|point| *point != "/" && target.starts_with(point))
        .max_by_key(|point| point.len())
        .map(str::to_string)
}

/// A file's bytes, or `None` when there is no such file.
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Put the backup of `path` back, and drop it.
fn put_back(path: &Path, backup: &Path) -> Result<()> {
    let original = fs::read(backup).with_context(|| format!("reading {}", backup.display()))?;
    write_through(path, &original).with_context(|| format!("restoring {}", path.display()))?;
    fs::remove_file(backup).ok();
    Ok(())
}

/// Two texts that differ at most in how they end.
fn same_text(a: &str, b: &str) -> bool {
    a.trim_end() == b.trim_end()
}

/// Put `bytes` at `path`, keeping whatever was there the first time.
fn lay_down(path: &Path, bytes: &[u8], backup: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    // Keep whatever was there the first time, and only the first time: a
    // second apply must not back up our own copy over the real original. A
    // backup that fails stops the apply; writing on without one would lose
    // the original for good.
    if path.exists() && !backup.exists() {
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::copy(path, backup)
            .with_context(|| format!("backing up {} to {}", path.display(), backup.display()))?;
    }
    write_through(path, bytes).with_context(|| format!("writing {}", path.display()))
}

impl Step {
    /// Is the device already like this?
    ///
    /// A file that cannot be read is never "already like this". It reads as
    /// `changed`, and applying it stops with the reason.
    pub fn satisfied(&self) -> bool {
        match self {
            Step::Block { file, id, body, form } => {
                let Ok(text) = read_text(file) else { return false };
                let text = text.unwrap_or_default();
                // An unterminated block is not "absent", whatever read_block
                // makes of it.
                let Ok(rest) = clear_block(&text, id) else { return false };
                match (read_block(&text, id), body) {
                    (None, None) => true,
                    (Some(found), Some(want)) if found.trim() == want.trim() => match form {
                        Form::Script => true,
                        Form::Settings => in_force(&text, want),
                        Form::Seeded(seed) => {
                            matches!(read_text(seed), Ok(Some(stock)) if same_text(&rest, &stock))
                        }
                    },
                    _ => false,
                }
            }
            Step::Place { path, bytes, .. } => match bytes {
                Some(want) => fs::read(path).map(|got| got == *want).unwrap_or(false),
                None => !path.exists(),
            },
            Step::Cover { path, ours, on, .. } => match read_bytes(path) {
                Ok(now) => (now.as_deref() == Some(*ours)) == *on,
                Err(_) => false,
            },
            Step::Shared { path, bytes, want, named } => {
                if *want {
                    fs::read(path).map(|got| got == *bytes).unwrap_or(false)
                } else {
                    !path.exists() || named.still_named().unwrap_or(false)
                }
            }
        }
    }

    pub fn apply(&self) -> Result<()> {
        match self {
            Step::Block { file, id, body, form } => {
                let current = read_text(file)?;
                let next = match (form, body) {
                    // Rebuilt from the /etc copy as it is now, every time. A
                    // copy that cannot be read stops everything: this file
                    // replaces that one, so our lines alone would take every
                    // key it carries.
                    (Form::Seeded(seed), Some(body)) => {
                        let stock = read_text(seed)?.with_context(|| {
                            format!(
                                "{} is missing, and {} replaces it rather than adding to it; \
                                 nothing was written",
                                seed.display(),
                                file.display()
                            )
                        })?;
                        set_block(&stock, id, body)?
                    }
                    (Form::Seeded(seed), None) => {
                        let Some(text) = current.as_deref() else { return Ok(()) };
                        let rest = clear_block(text, id)?;
                        // Nothing left but KNULLI's own file: remove it, so
                        // /etc is what triggerhappy reads, this image's and
                        // every later one's.
                        if read_text(seed)?.is_some_and(|stock| same_text(&rest, &stock)) {
                            return remove_through(file)
                                .with_context(|| format!("removing {}", file.display()));
                        }
                        rest
                    }
                    (_, Some(body)) => set_block(current.as_deref().unwrap_or(""), id, body)?,
                    (_, None) => match current.as_deref() {
                        // Nothing to take a block out of. Creating an empty
                        // file here would be a change nobody asked for.
                        None => return Ok(()),
                        Some(text) => clear_block(text, id)?,
                    },
                };
                if current.as_deref() == Some(next.as_str()) {
                    return Ok(());
                }
                if let Some(parent) = file.parent() {
                    fs::create_dir_all(parent).ok();
                }
                write_through(file, next.as_bytes())
                    .with_context(|| format!("writing {}", file.display()))
            }
            Step::Place { path, bytes, backup } => match bytes {
                Some(bytes) => lay_down(path, bytes, backup),
                None => {
                    if backup.exists() {
                        put_back(path, backup)?;
                    } else if path.exists() {
                        remove_through(path)
                            .with_context(|| format!("removing {}", path.display()))?;
                    }
                    Ok(())
                }
            },
            Step::Cover { path, ours, on, backup } => {
                if *on {
                    return lay_down(path, ours, backup);
                }
                let now = read_bytes(path)?;
                let is_ours = now.as_deref() == Some(*ours);
                if (is_ours || now.is_none()) && backup.exists() {
                    put_back(path, backup)
                } else if is_ours {
                    remove_through(path).with_context(|| format!("removing {}", path.display()))
                } else {
                    // Somebody else's: KNULLI's own, or one ES wrote since.
                    Ok(())
                }
            }
            Step::Shared { path, bytes, want, named } => {
                if *want {
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).ok();
                    }
                    return write_through(path, bytes)
                        .with_context(|| format!("writing {}", path.display()));
                }
                if !path.exists() || named.still_named()? {
                    return Ok(());
                }
                remove_through(path).with_context(|| format!("removing {}", path.display()))
            }
        }
    }
}

/// One setting a patch can be at.
#[derive(Clone, Debug)]
pub struct Choice {
    pub name: String,
    pub steps: Vec<Step>,
}

/// Where a patch currently is.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum State {
    /// Sitting exactly at one of its options.
    At(usize),
    /// At none of them. Either somebody edited the file by hand, or KNULLI
    /// shipped an update and the ground moved. Worth saying out loud rather
    /// than quietly overwriting.
    Changed,
}

#[derive(Clone, Debug)]
pub struct Patch {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub choices: Vec<Choice>,
}

impl Patch {
    /// Whether applying `index` writes a file EmulationStation keeps in memory
    /// and writes back: knulli.conf, or anything under its own config folder.
    ///
    /// Opened as a Port, the window runs with ES still up underneath it, and a
    /// change to one of these lasts only until ES next saves.
    pub fn writes_what_es_holds(&self, index: usize, paths: &Paths) -> bool {
        let es_dir = paths.es_settings();
        let es_dir = es_dir.parent();
        self.choices.get(index).is_some_and(|choice| {
            choice.steps.iter().any(|step| {
                let target = match step {
                    Step::Block { file, .. } => file,
                    Step::Place { path, .. } | Step::Cover { path, .. } | Step::Shared { path, .. } => path,
                };
                *target == paths.knulli_conf() || es_dir.is_some_and(|d| target.starts_with(d))
            })
        })
    }

    pub fn state(&self) -> State {
        self.choices
            .iter()
            .position(|c| c.steps.iter().all(Step::satisfied))
            .map(State::At)
            .unwrap_or(State::Changed)
    }

    pub fn option_names(&self) -> Vec<String> {
        self.choices.iter().map(|c| c.name.clone()).collect()
    }

    /// Run one option's steps in order. Stops at the first failure and says
    /// which step it was, because a half-applied patch that reports success
    /// is worse than one that fails loudly.
    pub fn apply(&self, index: usize) -> Result<()> {
        let choice = self
            .choices
            .get(index)
            .with_context(|| format!("{} has no option {index}", self.id))?;
        for (n, step) in choice.steps.iter().enumerate() {
            step.apply()
                .with_context(|| format!("{} step {}", self.id, n + 1))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As the catalogue builds them: the backup lives outside the directory
    /// the file is in.
    fn placed(root: &Path, path: PathBuf, bytes: Option<&'static [u8]>) -> Step {
        let backup = Paths::new(root).backup_for(&path);
        Step::Place { path, bytes, backup }
    }

    /// A block in knulli.conf, as the catalogue builds them.
    fn conf_block(file: &Path, id: &str, body: Option<&str>) -> Step {
        Step::Block {
            file: file.to_path_buf(),
            id: id.into(),
            body: body.map(str::to_string),
            form: Form::Settings,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-patch-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_boot_hook_only_reads_from_partitions_it_can_see() {
        // S00bootcustom runs before S02resize, which is what mounts
        // /userdata. Anything the hook needs at boot must be somewhere already
        // mounted — /boot, which is where it reads itself from. Keeping the
        // blank logo in /userdata meant the hook silently did nothing and the
        // KNULLI logo was back after every reboot.
        let paths = Paths::new("/");
        for needed_at_boot in [
            paths.blank_logo(),
            paths.boot_custom(),
            paths.gpu_choice(),
            paths.gpu_blob("wayland"),
        ] {
            assert!(
                needed_at_boot.starts_with("/boot"),
                "{} is read by the S00 hook and must be on /boot",
                needed_at_boot.display()
            );
        }
    }

    #[test]
    fn a_block_round_trips() {
        let text = "keep=1\n";
        let with = set_block(text, "hotkeys", "a=1\nb=2").unwrap();
        assert_eq!(read_block(&with, "hotkeys").as_deref(), Some("a=1\nb=2"));
        assert!(with.contains("keep=1"), "the rest of the file must survive");
        let without = clear_block(&with, "hotkeys").unwrap();
        assert_eq!(read_block(&without, "hotkeys"), None);
        assert!(without.contains("keep=1"));
    }

    /// The lines a settings reader would actually act on: not blank, not
    /// commented out.
    fn live(text: &str) -> Vec<&str> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }

    #[test]
    fn a_key_already_in_the_file_is_the_one_that_wins() {
        // knulli.conf is FIRST-wins. `knulli-settings-get` scans from the top
        // and stops. A block appended at the end repeating a key from line 319
        // is read by nothing — which is exactly what `never-sleep` did, while
        // the app cheerfully reported it as on.
        let before = "system.batterysaver.mode=dim\n\
                      system.batterysaver.extendedmode=suspend\n\
                      system.power.led=1\n";
        let after = set_block(before, "power", "system.batterysaver.extendedmode=none\n").unwrap();

        // The old line is no longer a setting…
        let set: Vec<&str> = live(&after).into_iter().filter(|l| l.contains("extendedmode")).collect();
        assert_eq!(set, vec!["system.batterysaver.extendedmode=none"]);
        // …and the keys we did not touch are untouched.
        assert!(live(&after).contains(&"system.batterysaver.mode=dim"));
        assert!(live(&after).contains(&"system.power.led=1"));
    }

    #[test]
    fn turning_it_off_puts_the_original_line_back_exactly() {
        // Not "delete our lines" — restore what KNULLI had. Guessing the
        // default is how a device ends up in a state its owner never chose.
        let before = "system.batterysaver.extendedmode=suspend\n\
                      system.batterysaver.chargingbypass=0\n";
        let on = set_block(before, "power", "system.batterysaver.extendedmode=none\n").unwrap();
        let off = clear_block(&on, "power").unwrap();
        assert_eq!(off, before, "did not come back to what it was");
    }

    #[test]
    fn hiding_survives_being_applied_over_and_over() {
        // Every apply runs set_block again. If each one hid the line it had
        // already hidden, the markers would nest and the original would be
        // unrecoverable.
        let before = "system.batterysaver.extendedmode=suspend\n";
        let mut text = before.to_owned();
        for _ in 0..3 {
            text = set_block(&text, "power", "system.batterysaver.extendedmode=none\n").unwrap();
        }
        assert_eq!(text.matches("hid:").count(), 1, "stacked its own markers");
        assert_eq!(clear_block(&text, "power").unwrap(), before);
    }

    #[test]
    fn a_hidden_line_is_not_mistaken_for_the_blocks_own_marker() {
        // `read_block` matches a trimmed line against "## moose-patch: power".
        // If the hiding marker could equal that, a block would swallow the
        // line it hid and lose it.
        let text = set_block(
            "system.batterysaver.extendedmode=suspend\n",
            "power",
            "system.batterysaver.extendedmode=none\n",
        )
        .unwrap();
        assert_eq!(
            read_block(&text, "power").as_deref(),
            Some("system.batterysaver.extendedmode=none")
        );
    }

    #[test]
    fn one_block_does_not_disturb_what_another_hid() {
        let before = "system.batterysaver.extendedmode=suspend\n\
                      system.batterysaver.chargingbypass=0\n";
        let a = set_block(before, "power", "system.batterysaver.extendedmode=none\n").unwrap();
        let b = set_block(&a, "charge", "system.batterysaver.chargingbypass=1\n").unwrap();
        // Taking one off leaves the other's work standing.
        let off = clear_block(&b, "power").unwrap();
        assert!(off.contains("system.batterysaver.extendedmode=suspend"), "did not restore");
        let set: Vec<&str> =
            live(&off).into_iter().filter(|l| l.contains("chargingbypass")).collect();
        assert_eq!(set, vec!["system.batterysaver.chargingbypass=1"], "clobbered the other");
    }

    #[test]
    fn a_block_that_sets_nothing_hides_nothing() {
        // Comments and blanks are not settings. A body of pure commentary must
        // not silently disable a line somewhere above it.
        let before = "system.power.led=1\n";
        let after = set_block(before, "note", "# just a comment\n\n").unwrap();
        assert!(live(&after).contains(&"system.power.led=1"), "hid a key it never set");
    }

    #[test]
    fn applying_twice_does_not_stack_blocks() {
        // knulli.conf is read by a parser that takes the last value it sees.
        // Two copies of a block is how a revert silently fails.
        let once = set_block("keep=1\n", "shaders", "x=1").unwrap();
        let twice = set_block(&once, "shaders", "x=1").unwrap();
        assert_eq!(twice.matches("## moose-patch: shaders\n").count(), 1);
        assert_eq!(once, twice, "a second apply should change nothing");
    }

    #[test]
    fn changing_a_block_replaces_it_rather_than_appending() {
        let once = set_block("keep=1\n", "shaders", "set=a").unwrap();
        let twice = set_block(&once, "shaders", "set=b").unwrap();
        assert_eq!(read_block(&twice, "shaders").as_deref(), Some("set=b"));
        assert!(!twice.contains("set=a"));
    }

    #[test]
    fn clearing_a_block_that_was_never_there_is_harmless() {
        assert_eq!(clear_block("keep=1\n", "nope").unwrap(), "keep=1\n");
    }

    #[test]
    fn a_config_that_cannot_be_read_is_left_as_it_is() {
        // One byte that is not UTF-8 used to read as an empty file, and the
        // write after it left knulli.conf holding our block and nothing else:
        // Wi-Fi, cores, every setting gone.
        let dir = scratch("unreadable");
        let file = dir.join("knulli.conf");
        let before = b"wifi.ssid=home\ncaf\xe9=1\ngba=mgba\n".to_vec();
        fs::write(&file, &before).unwrap();

        let on = Step::Block {
            file: file.clone(),
            id: "power".into(),
            body: Some("a=1".into()),
            form: Form::Settings,
        };
        let err = on.apply().expect_err("applied over a file it could not read");
        assert!(format!("{err:#}").contains("UTF-8"), "{err:#}");
        assert_eq!(fs::read(&file).unwrap(), before, "the file was written");

        // And it reads as at no option, rather than as "off".
        let off = conf_block(&file, "power", None);
        assert!(!off.satisfied());
        assert!(!on.satisfied());
    }

    #[test]
    fn a_missing_seed_writes_nothing() {
        // The /userdata trigger file replaces the /etc one. Created with only
        // our two lines, volume, power and the lid stop working.
        let dir = scratch("no-seed");
        let file = dir.join("userdata/multimedia_keys.conf");
        let step = Step::Block {
            file: file.clone(),
            id: "hotkey".into(),
            body: Some("BTN_TR2+BTN_TL2 1 /bin/true".into()),
            form: Form::Seeded(dir.join("etc/multimedia_keys.conf")),
        };
        assert!(step.apply().is_err());
        assert!(!file.exists(), "created the file with only our lines in it");
    }

    #[test]
    fn a_block_with_no_end_marker_is_refused_not_cut_to_the_end_of_the_file() {
        let text = "keep=1\n## moose-patch: power\nsystem.batterysaver.mode=dim\ngba=mgba\n";
        assert!(clear_block(text, "power").is_err());
        assert!(set_block(text, "power", "x=1").is_err());

        let dir = scratch("no-end-marker");
        let file = dir.join("knulli.conf");
        fs::write(&file, text).unwrap();
        let off = conf_block(&file, "power", None);
        assert!(off.apply().is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), text, "the lines below it went");
        assert!(!off.satisfied(), "a broken block is not an absent one");
    }

    #[test]
    fn a_write_replaces_the_file_rather_than_rewriting_it_in_place() {
        // Written beside and renamed over, so a reader or a power cut sees the
        // old file or the new one. A second link to the old file is how to
        // tell from outside: rewritten in place, it would change too.
        let dir = scratch("atomic");
        let file = dir.join("knulli.conf");
        let link = dir.join("old-link");
        fs::write(&file, "keep=1\n").unwrap();
        fs::hard_link(&file, &link).unwrap();

        conf_block(&file, "x", Some("a=1")).apply().unwrap();

        assert!(fs::read_to_string(&file).unwrap().contains("a=1"));
        assert_eq!(fs::read_to_string(&link).unwrap(), "keep=1\n");
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "a temporary file was left behind: {names:?}");
    }

    #[test]
    #[cfg(unix)]
    fn a_backup_that_fails_stops_the_write() {
        // Writing on without the backup would lose the original for good.
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("backup-fails");
        let path = dir.join("es_input.cfg");
        fs::write(&path, b"KNULLI's own").unwrap();
        let backup = Paths::new(&dir).backup_for(&path);
        let backups = backup.parent().unwrap().to_path_buf();
        fs::create_dir_all(&backups).unwrap();
        fs::set_permissions(&backups, fs::Permissions::from_mode(0o555)).unwrap();

        let result = placed(&dir, path.clone(), Some(b"ours")).apply();
        fs::set_permissions(&backups, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"KNULLI's own");
    }

    #[test]
    fn a_block_step_reads_its_own_state() {
        let dir = scratch("block-state");
        let file = dir.join("knulli.conf");
        fs::write(&file, "keep=1\n").unwrap();

        let on = conf_block(&file, "hotkeys", Some("a=1"));
        let off = conf_block(&file, "hotkeys", None);

        assert!(off.satisfied(), "no block means off is already true");
        assert!(!on.satisfied());
        on.apply().unwrap();
        assert!(on.satisfied());
        assert!(!off.satisfied());
        off.apply().unwrap();
        assert!(off.satisfied(), "off has to be able to undo on");
    }

    #[test]
    fn the_reader_takes_the_first_live_line() {
        let text = "# system.power.led=9\n\
                    ## moose-patch: power hid: system.power.led=8\n\
                    system.power.led = 1\n\
                    system.power.led=2\n";
        assert_eq!(effective(text, "system.power.led"), Some("1"));
        assert_eq!(effective(text, "system.lid"), None);
    }

    #[test]
    fn a_block_is_on_only_while_the_reader_would_find_its_values() {
        // ES holds knulli.conf in memory and writes all of it back. A line it
        // puts above our block wins, first-wins, while the block itself is
        // untouched. Reading only the block called that on.
        let dir = scratch("in-force");
        let file = dir.join("knulli.conf");
        fs::write(&file, "system.batterysaver.extendedmode=suspend\ngba=mgba\n").unwrap();
        let on = conf_block(&file, "power", Some("system.batterysaver.extendedmode=none"));
        let off = conf_block(&file, "power", None);
        on.apply().unwrap();
        assert!(on.satisfied());

        let text = fs::read_to_string(&file).unwrap();
        fs::write(&file, format!("system.batterysaver.extendedmode=suspend\n{text}")).unwrap();
        assert!(!on.satisfied(), "the reader finds =suspend first");
        assert!(!off.satisfied(), "and the block is still there, so it is not off either");

        // A script is not a settings file: there the block is the change.
        let script = dir.join("custom.sh");
        let step = Step::Block {
            file: script.clone(),
            id: "wifi".into(),
            body: Some("X=1".into()),
            form: Form::Script,
        };
        step.apply().unwrap();
        let text = fs::read_to_string(&script).unwrap();
        fs::write(&script, format!("X=2\n{text}")).unwrap();
        assert!(step.satisfied());
    }

    #[test]
    fn placing_a_file_keeps_the_original_and_gives_it_back() {
        // This is the whole reason "off" is safe on es_input.cfg and on the
        // ES logo: what KNULLI shipped has to come back, not vanish.
        let dir = scratch("place");
        let path = dir.join("logo.png");
        fs::write(&path, b"the original").unwrap();

        let on = placed(&dir, path.clone(), Some(b"ours"));
        let off = placed(&dir, path.clone(), None);

        on.apply().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"ours");
        assert!(on.satisfied());

        off.apply().unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"the original",
            "reverting must restore what was there, not delete it"
        );
        assert!(
            !Paths::new(&dir).backup_for(&path).exists(),
            "and clean up after itself"
        );
    }

    #[test]
    fn a_second_apply_does_not_overwrite_the_backup() {
        let dir = scratch("backup-once");
        let path = dir.join("logo.png");
        fs::write(&path, b"the original").unwrap();
        let on = placed(&dir, path.clone(), Some(b"ours"));
        on.apply().unwrap();
        on.apply().unwrap();
        placed(&dir, path.clone(), None).apply().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"the original");
    }

    #[test]
    fn a_backup_never_lands_in_the_directory_it_came_from() {
        // Found on the device, not in the code: RetroArch cycles the shader
        // folder by listing it, and the backups were sitting in there among
        // the presets. Anything that enumerates a directory we manage must
        // see only what we meant to put there.
        let dir = scratch("backup-elsewhere");
        let managed = dir.join("userdata/shaders/moose");
        fs::create_dir_all(&managed).unwrap();
        let preset = managed.join("1-shimmerless.glslp");
        fs::write(&preset, b"the old one").unwrap();

        placed(&dir, preset.clone(), Some(b"ours")).apply().unwrap();

        let left: Vec<String> = fs::read_dir(&managed)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["1-shimmerless.glslp"], "stray files: {left:?}");

        // And the original is still recoverable from wherever it went.
        placed(&dir, preset.clone(), None).apply().unwrap();
        assert_eq!(fs::read(&preset).unwrap(), b"the old one");
    }

    #[test]
    fn taking_a_patch_off_goes_through_the_same_door_as_putting_it_on() {
        // Both directions have to cope with a read-only mount. Only the write
        // side did, so "graphics driver: back to stock" deleted nothing on
        // /boot and the device came back on the other driver, twice.
        let dir = scratch("remove-through");
        let path = dir.join("marker");
        fs::write(&path, b"wayland").unwrap();
        placed(&dir, path.clone(), None).apply().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_file_we_added_is_removed_rather_than_restored() {
        let dir = scratch("place-new");
        let path = dir.join("added.conf");
        placed(&dir, path.clone(), Some(b"x")).apply().unwrap();
        assert!(path.exists());
        placed(&dir, path.clone(), None).apply().unwrap();
        assert!(!path.exists(), "nothing was there before, so nothing after");
    }

    #[test]
    fn a_patch_reports_which_option_it_is_at() {
        let dir = scratch("patch-state");
        let file = dir.join("knulli.conf");
        fs::write(&file, "keep=1\n").unwrap();

        let patch = Patch {
            id: "shaders",
            title: "Shaders",
            detail: "",
            choices: vec![
                Choice {
                    name: "off".into(),
                    steps: vec![Step::Block {
                        file: file.clone(),
                        id: "shaders".into(),
                        body: None,
                        form: Form::Settings,
                    }],
                },
                Choice {
                    name: "ON".into(),
                    steps: vec![Step::Block {
                        file: file.clone(),
                        id: "shaders".into(),
                        body: Some("set=a".into()),
                        form: Form::Settings,
                    }],
                },
            ],
        };

        assert_eq!(patch.state(), State::At(0));
        patch.apply(1).unwrap();
        assert_eq!(patch.state(), State::At(1));
        patch.apply(0).unwrap();
        assert_eq!(patch.state(), State::At(0), "and back again");
    }

    #[test]
    fn a_hand_edited_block_reads_as_changed() {
        // The case this whole enum exists for: KNULLI ships an update, or
        // somebody edits the file, and what is there is not any of our
        // options. Overwriting silently would be the wrong answer.
        let dir = scratch("patch-changed");
        let file = dir.join("knulli.conf");
        fs::write(&file, set_block("keep=1\n", "shaders", "set=SOMETHING ELSE").unwrap()).unwrap();

        let patch = Patch {
            id: "shaders",
            title: "Shaders",
            detail: "",
            choices: vec![Choice {
                name: "ON".into(),
                steps: vec![Step::Block {
                    file,
                    id: "shaders".into(),
                    body: Some("set=a".into()),
                    form: Form::Settings,
                }],
            }],
        };
        assert_eq!(patch.state(), State::Changed);
    }
}

//! Locating and launching a RetroArch install.
//!
//! Deliberately does not assume `/Applications/RetroArch.app`: this machine's
//! install lives elsewhere and runs in portable mode, which is the layout we
//! target. See PLAN.md §6 for how portable mode resolves directories.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::padprofile;
use crate::util::expand_tilde;

/// Seed for the user's own RetroArch settings.
///
/// Everything is commented out: this file is appended last at launch, so an
/// uncommented line takes effect immediately and silently changing someone's
/// controls would be worse than doing nothing.
const USER_CONFIG_TEMPLATE: &str = r#"# Your RetroArch settings, applied every time this app launches a game.
#
# Same format as retroarch.cfg: key = "value". Appended AFTER our defaults, so
# anything here wins. Your own retroarch.cfg is never modified.
#
# Find the exact key for a setting by changing it once in RetroArch's menu and
# diffing its retroarch.cfg, or see docs.libretro.com.

# ---- Video filters / shaders ----
# video_shader_enable = "true"
# video_shader = "~/Data/Games/Emulators/RetroArch/shaders/shaders_glsl/crt/crt-geom.glslp"
# video_smooth = "false"
# video_scale_integer = "true"

# ---- Windows: window blacks out while being resized ----
# These three used to be forced on Windows and made it worse on an Nvidia card
# with G-Sync: d3d11 with "sync to exact content framerate" settles on a
# different refresh rate every launch. Try them one at a time if resizing still
# flashes, starting with the driver.
# video_driver = "d3d11"
# vrr_runloop_enable = "true"
# video_max_swapchain_images = "2"

# ---- Controls ----
# Player 1, RetroPad button -> your device's button index.
# input_player1_a_btn = "1"
# input_player1_b_btn = "0"
# input_player1_x_btn = "3"
# input_player1_y_btn = "2"
# input_player1_l_btn = "4"
# input_player1_r_btn = "5"
# input_player1_start_btn = "9"
# input_player1_select_btn = "8"
# input_player1_up_btn = "h0up"
# input_player1_down_btn = "h0down"
# input_player1_left_btn = "h0left"
# input_player1_right_btn = "h0right"

# ---- Hotkeys ----
# input_enable_hotkey_btn = "8"
# input_exit_emulator_btn = "9"
# input_menu_toggle_btn = "2"

# ---- Anything else ----
# audio_latency = "64"
# fastforward_ratio = "3.0"
"#;

/// A located RetroArch install.
#[derive(Debug)]
pub struct RetroArch {
    /// Set when BIOS should be read from somewhere other than
    /// `<root>/system`; see [`Self::with_system_dir`].
    pub system_override: Option<PathBuf>,
    /// Directory containing `RetroArch.app`. In portable mode this is also the
    /// root for `cores/`, `saves/`, `states/`, `system/`, `config/`.
    pub root: PathBuf,
    pub binary: PathBuf,
    /// True when `portable.txt` sits beside the bundle, meaning RetroArch keeps
    /// everything under `root` instead of `~/Documents` + `~/Library`.
    pub portable: bool,
}

// Where RetroArch usually lives, in probe order, now lives in
// `platform::current().retroarch_roots()`.
//
// The shape of an install differs enough between targets that one list will not
// do: macOS ships an `.app` bundle, Windows a directory with `retroarch.exe`,
// desktop Linux a system prefix. That much `target_os` could express. What it
// could not is KNULLI, which *is* `target_os = "linux"` and has exactly one
// root, or Android, where RetroArch is a separate app reached by Intent rather
// than a binary on a path at all.

/// Fold a per-launch override fragment into a complete RetroArch config.
///
/// Android needs this because its Intent has no `--appendconfig`. All it takes
/// is `CONFIGFILE`, and that is the *whole* config: whatever the file does not
/// mention falls back to RetroArch's built-in defaults, not to the user's
/// settings. Handing it our fragment alone would silently reset the video
/// driver, the directories and every binding the user has ever set.
///
/// So the user's own `retroarch.cfg` is the base and our fragment is laid over
/// it. Keys we set replace theirs in place; keys we do not set are left exactly
/// as they were, comments and order included, so the file we hand over is
/// theirs with our changes in it. Anything of ours the base has never heard of
/// is appended at the end under a heading.
///
/// Their file is only ever read. The result is written somewhere else, and
/// `config_save_on_exit = "false"` in the fragment stops RetroArch writing back
/// over even that copy.
///
/// Only `key = value` lines are taken from the fragment; its comments explain
/// the desktop arrangement and would be misleading in a merged file.
pub fn merge_config(base: &str, overlay: &str) -> String {
    let mut ours: Vec<(String, String)> = Vec::new();
    for line in overlay.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        // A key with a space in it is not a key; skip rather than corrupt.
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }
        ours.push((key.to_owned(), value.trim().to_owned()));
    }
    // One entry per key, the last one winning — which is how RetroArch reads a
    // file with a repeated key, and how the fragment is built: a later block
    // deliberately overrides an earlier one.
    //
    // Deduplicated here rather than at the point of use, because the two halves
    // of the merge disagreed about it otherwise: the replacement below consumed
    // only the last, and the append at the end then wrote the *earlier* value
    // back out — after it. `auto_shaders_enable` came out both "true" and
    // "false", in that order, and the file ended with the one we did not mean.
    {
        let mut seen = std::collections::BTreeSet::new();
        let mut deduped: Vec<(String, String)> = Vec::with_capacity(ours.len());
        for (key, value) in ours.into_iter().rev() {
            if seen.insert(key.clone()) {
                deduped.push((key, value));
            }
        }
        deduped.reverse();
        ours = deduped;
    }

    let mut used = vec![false; ours.len()];
    let mut out = String::with_capacity(base.len() + overlay.len());
    for line in base.lines() {
        let key = line.split_once('=').map(|(k, _)| k.trim());
        // Last one wins, matching how RetroArch reads a file with a repeated
        // key — so a fragment that sets something twice behaves the same here.
        let hit = key.and_then(|k| ours.iter().rposition(|(ok, _)| ok == k));
        match hit {
            Some(i) => {
                used[i] = true;
                out.push_str(&format!("{} = {}
", ours[i].0, ours[i].1));
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }

    let mut first = true;
    for (i, (key, value)) in ours.iter().enumerate() {
        if used[i] {
            continue;
        }
        if first {
            out.push_str("
# ---- Added by moose-rack for this launch ----
");
            first = false;
        }
        out.push_str(&format!("{key} = {value}
"));
    }
    out
}

/// Executable name for the host, and where it sits under the install root.
///
/// Returns `(subpath of the binary, subpath of the "root" we should record)`.
/// macOS hides the binary inside the bundle and treats the *containing*
/// directory as the root, because that is where `portable.txt` and `cores/`
/// live. The other two put the executable in the root itself.
/// First path that exists, preferring `primary`; falls back to the first
/// `primary` entry so callers always get a usable path to report.
fn first_existing(primary: &[PathBuf], extra: &[PathBuf]) -> PathBuf {
    primary
        .iter()
        .chain(extra.iter())
        .find(|p| p.is_dir())
        .cloned()
        .unwrap_or_else(|| primary[0].clone())
}

fn binary_candidates(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    #[cfg(target_os = "macos")]
    {
        let bundle = if root.extension().is_some_and(|e| e == "app") {
            root.to_path_buf()
        } else {
            root.join("RetroArch.app")
        };
        let recorded = bundle.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
        vec![(bundle.join("Contents/MacOS/RetroArch"), recorded)]
    }
    #[cfg(target_os = "windows")]
    {
        vec![(root.join("retroarch.exe"), root.to_path_buf())]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // A distro install puts the binary in <prefix>/bin but keeps cores and
        // config elsewhere, so both layouts are worth trying.
        vec![
            (root.join("bin").join("retroarch"), root.to_path_buf()),
            (root.join("retroarch"), root.to_path_buf()),
        ]
    }
}

/// How much of the display the game window fills.
///
/// Not all of it. A window exactly the size of the work area is the same size
/// as a maximised one, and on both macOS and Windows that ends up flush against
/// an edge with no way to grab the title bar — so a sliver is left, which also
/// makes it obvious at a glance that this is a window and the desktop is still
/// behind it.
const SCREEN_FILL: f32 = 0.94;

/// Everything about the controllers, for one launch.
///
/// One struct because they always travel together and there is nothing to be
/// gained from passing four things four times — the linter is right that eight
/// positional arguments is a signature nobody reads correctly.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The connected pad's reported name, which picks the autoconfig profile.
    pub pad: Option<&'a str>,
    /// Bind players 2-4 like player 1.
    pub mirror_players: bool,
    pub autofire: crate::tweaks::AutoFire,
    /// Shots a second, when auto-fire is on.
    pub autofire_hz: u32,
    /// Write a save state when the game exits.
    pub save_state_on_exit: bool,
    /// Where RetroArch should keep battery saves and save states.
    ///
    /// `None` leaves RetroArch's own choice alone, which is what every launch
    /// did before this existed — and what it does on a machine where the
    /// setting has never been touched.
    pub saves_root: Option<&'a Path>,
}

impl Default for Input<'_> {
    fn default() -> Self {
        Self {
            pad: None,
            mirror_players: true,
            autofire: crate::tweaks::AutoFire::Off,
            autofire_hz: 5,
            save_state_on_exit: false,
            saves_root: None,
        }
    }
}

/// Where the game window should go: the monitor's own origin and size, in the
/// units the desktop uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    /// Top-left of this monitor, in the desktop's coordinates, y counting down.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Height of the primary monitor. Needed only on macOS, where window
    /// coordinates count up from the bottom of *that* screen — see below.
    pub primary_height: u32,
}

/// Space the menu bar takes at the top of a macOS screen.
///
/// A window placed under it is pushed down by the window server, which then
/// makes the bottom of the window hang off the screen. Leaving the room costs
/// nothing and is the difference between "as tall as the screen" and "as tall
/// as the screen, with the bottom inch missing".
const MENU_BAR: u32 = 38;

/// Open the game window in the top-left of the screen the library is on, as
/// tall as that screen allows.
///
/// Two things here were found by measuring rather than by reading, because both
/// are silent when wrong.
///
/// The first is the setting names. `video_window_width` and
/// `video_window_height` are what the documentation says, and RetroArch 1.20
/// does not have them -- they are absent from a config it wrote itself, which
/// writes every setting it knows. Asking for a 2406-wide window through those
/// produced a 720-wide one, because the size came from `video_scale` and
/// nothing had been overridden at all. The keys that work are
/// `video_windowed_position_width` and `_height`, and they are only read when
/// `video_window_save_positions` is on.
///
/// The second is what the coordinates mean. Asking for y = 0 on an
/// 1169-point-tall screen put the window's *bottom* at the bottom of the
/// screen: RetroArch passes the value to Cocoa, whose origin is the bottom-left
/// of the primary display with y counting upwards. So "the top of this monitor"
/// is not 0, and on a second monitor it is not even a positive number in the
/// obvious direction. The arithmetic below is that conversion, and it is why
/// the primary monitor's height has to travel with the rest.
pub fn window_lines(
    screen: Option<Screen>,
    aspect: Option<crate::aspect::Shape>,
    decorations: bool,
) -> String {
    let Some(s) = screen else {
        return String::new();
    };
    if s.width == 0 || s.height == 0 {
        return String::new();
    }

    // As tall as the screen allows, less the menu bar. Width is left a little
    // short of the full screen: a window exactly as wide as the display has no
    // edge left to grab, and every console here is squarer than a modern
    // monitor anyway, so the extra width would be black bars.
    let (top_gap, bottom_gap) = if cfg!(target_os = "macos") {
        (MENU_BAR, 0)
    } else {
        (0, 0)
    };
    // Negative coordinates do not survive the trip. Asking for x = -378 — the
    // left edge of a monitor sitting up and to the left of the built-in one —
    // put the window at x = 2142, most of it off the right-hand side of the
    // desktop, which is the "still at the right edge" this kept producing.
    // Asking for x = 0 on the same monitor landed exactly at 0. So the setting
    // is read as unsigned somewhere between here and the window.
    //
    // Clamping at zero rather than refusing: on a screen that starts left of
    // the origin, x = 0 is still a point on that screen, so the window opens
    // where it was asked to open — just not flush against the left edge. The
    // width comes down to match, or it would run off the far side.
    let left = s.x.max(0);
    let lost_x = (left - s.x).max(0) as u32;
    let usable_w = s.width.saturating_sub(lost_x);

    // Same for the vertical, in Cocoa's terms: the distance from the bottom of
    // the primary display up to the bottom edge of the window.
    let want_y = if cfg!(target_os = "macos") {
        s.primary_height as i32 - s.y - s.height as i32
    } else {
        s.y
    };
    let bottom = want_y.max(0);
    let lost_y = (bottom - want_y).max(0) as u32;
    let usable_h = s.height.saturating_sub(lost_y);

    let mut h = usable_h.saturating_sub(top_gap + bottom_gap);
    let mut w = (usable_w as f32 * SCREEN_FILL) as u32;

    // Shaped like the game, when the platform has one shape. RetroArch keeps
    // the picture's proportions inside whatever window it is given, so a
    // window of the wrong shape is a window with black bars in it — and on a
    // maximised one those bars are large. Giving it a window of the right shape
    // leaves nothing over to put a bar in.
    let mut shape = String::new();
    if let Some(a) = aspect {
        let (fw, fh) = crate::aspect::fit(w, h, a.ratio);
        w = fw;
        h = fh;

        // And tell RetroArch to draw at that same shape.
        //
        // Sizing the window alone was half a fix. The default here is square
        // pixel, which is the frame buffer's shape rather than the
        // television's: a Neo Geo frame is 320x224, which is 10:7, and it was
        // meant to be seen as 4:3. So the window was 4:3, the picture was
        // 10:7, and RetroArch did the only thing it could and put bars in the
        // difference. They agree now, and there is no difference to fill.
        //
        // Index 20 is "Config", which means "use video_aspect_ratio" — exact
        // for any ratio, where the numbered entries only cover a fixed list.
        // Only where every game on the platform really is this shape. Arcade
        // is sized to a 4:3 cabinet but drawn at whatever the game is, because
        // a rotated vertical shooter in that same cabinet is 3:4 and forcing
        // it to 4:3 would stretch it rather than letterbox it.
        if a.exact {
            shape = format!(
            "\n# Draw at the same shape as the window, so there is nothing left\n\
             # over to letterbox. 20 is \"Config\": use the ratio below.\n\
             aspect_ratio_index = \"20\"\n\
             video_aspect_ratio = \"{a:.6}\"\n\
             video_aspect_ratio_auto = \"false\"\n",
                a = a.ratio
            );
        }
    }

    // A screen so far off to the left that clamping leaves nothing worth
    // opening. Better to write no geometry at all and let RetroArch use its own
    // than to ask for a window nobody could play in.
    if w < 640 || h < 480 {
        return String::new();
    }

    let x = left;
    let y = bottom;

    let title_bar = if decorations {
        ""
    } else {
        "# No title bar. The way out is the controller combination or Escape.\n\
         video_window_show_decorations = \"false\"\n"
    };
    format!(
        "{shape}{title_bar}\n# Top-left of the screen the library is on, as tall as it goes.\n\
         video_fullscreen = \"false\"\n\
         # On: this is what makes the size below be read at all.\n\
         video_window_save_positions = \"true\"\n\
         video_windowed_position_width = \"{w}\"\n\
         video_windowed_position_height = \"{h}\"\n\
         video_windowed_position_x = \"{x}\"\n\
         video_windowed_position_y = \"{y}\"\n\
         # The names the documentation gives, which RetroArch 1.20 does not\n\
         # have. Kept for builds that do.\n\
         video_window_custom_size_enable = \"true\"\n\
         video_window_width = \"{w}\"\n\
         video_window_height = \"{h}\"\n\
         # And the ceiling, which is separate and silently wins. A RetroArch\n\
         # set up on a 1080p monitor keeps auto_width_max = 1920 forever, so on\n\
         # a 4K screen the window lands at a quarter of the area no matter what\n\
         # size was asked for -- which looks exactly like the size being\n\
         # ignored.\n\
         video_window_auto_width_max = \"{w}\"\n\
         video_window_auto_height_max = \"{h}\"\n"
    )
}


impl RetroArch {
    /// Locate an install. `configured` wins; otherwise probe known roots.
    /// RetroArch as it exists on Android: another app, not an install we found.
    ///
    /// There is no binary to run — launching is an Intent — but everything else
    /// this type does is about *files*, and those are all real: `retroarch.cfg`,
    /// `config/<Core>/`, `shaders/`. They live in RetroArch's own external
    /// files directory, which this app can read because it holds
    /// MANAGE_EXTERNAL_STORAGE and RetroArch targets SDK 28 and so is still on
    /// legacy storage.
    ///
    /// `portable` so `data_dir()` is the root: on Android there is no second
    /// location the way a desktop install has one beside the bundle.
    ///
    /// `binary` is empty on purpose. Nothing here spawns a process, and an
    /// invented path would be a thing that looks runnable and is not.
    pub fn android_app(files_dir: &Path) -> Self {
        Self {
            system_override: None,
            root: files_dir.to_path_buf(),
            binary: PathBuf::new(),
            portable: true,
        }
    }

    /// Find RetroArch on an Android device.
    ///
    /// Not a path search: there is no binary to find. What identifies an
    /// install here is its config, and that sits in its external files
    /// directory under its own package name — readable because this app holds
    /// MANAGE_EXTERNAL_STORAGE and RetroArch targets SDK 28.
    ///
    /// Both spellings, `aarch64` first, because a device may carry either
    /// build. Probing the filesystem rather than asking the package manager
    /// keeps this in Rust: Tauri does not hand the Android context to Rust
    /// (tauri-apps/tauri#13267), and every caller of this type is Rust.
    ///
    /// Without this `state.retroarch` was `None` on Android, and everything
    /// derived from it went quietly empty — the shader list in
    /// Settings → Emulators said "no RetroArch" on a device with 2,037 presets
    /// on the card.
    pub fn locate_android() -> Option<Self> {
        for package in ["com.retroarch.aarch64", "com.retroarch"] {
            let files = PathBuf::from(format!("/storage/emulated/0/Android/data/{package}/files"));
            if files.join("retroarch.cfg").is_file() {
                return Some(Self::android_app(&files));
            }
        }
        None
    }

    pub fn locate(configured: Option<&str>) -> Result<Self> {
        Self::locate_in(&configured.map(str::to_owned).into_iter().collect::<Vec<_>>())
    }

    /// Try each path in order and take the first that holds RetroArch.
    ///
    /// An empty list means "probe the usual places". Ordering is the point:
    /// it lets a portable build take precedence over a system one without
    /// uninstalling anything.
    pub fn locate_in(paths: &[String]) -> Result<Self> {
        let mut tried: Vec<PathBuf> = Vec::new();

        let candidates: Vec<PathBuf> = if paths.is_empty() {
            crate::platform::current().retroarch_roots().iter().map(|c| expand_tilde(c)).collect()
        } else {
            paths.iter().map(|p| expand_tilde(p)).collect()
        };

        for root in candidates {
            for (binary, recorded) in binary_candidates(&root) {
                tried.push(binary.clone());
                if !binary.is_file() {
                    continue;
                }
                // portable.txt is a macOS and Windows mechanism; on Linux
                // RetroArch always follows XDG and ignores the marker
                // entirely (PLAN.md §6).
                let portable = cfg!(not(target_os = "linux"))
                    && recorded.join("portable.txt").is_file();
                return Ok(Self {
                    root: recorded,
                    binary,
                    portable,
                    system_override: None,
                });
            }
        }

        bail!(
            "could not find RetroArch. Tried:\n{}\nSet [retroarch] root in config.toml.",
            tried
                .iter()
                .map(|p| format!("  {}", p.display()))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    /// Directory holding `*_libretro.<dylib|dll|so>`.
    ///
    /// Only correct for builds with `HAVE_UPDATE_CORES`, which is what the
    /// official download is; App Store builds keep cores inside the bundle.
    /// Verified against this machine's 1.20.0 install.
    pub fn cores_dir(&self) -> PathBuf {
        // Distro packages drop cores in a system library directory instead of
        // the user's data folder, and which directories those are is a
        // property of the device — KNULLI has exactly one, holding 99 cores.
        let extra: Vec<PathBuf> =
            crate::platform::current().core_dirs().iter().map(PathBuf::from).collect();
        first_existing(&[self.data_dir().join("cores")], &extra)
    }

    /// RetroArch's user-data root: where `retroarch.cfg`, `cores/`, `system/`
    /// and `config/` all hang off.
    ///
    /// In portable mode that is the install directory. Otherwise it is the
    /// platform's own location, which is the part that differs: macOS uses
    /// Application Support, Windows uses APPDATA, and Linux follows XDG —
    /// where `portable.txt` is ignored outright, so the marker is never even
    /// consulted there.
    pub fn data_dir(&self) -> PathBuf {
        if self.portable {
            return self.root.clone();
        }
        crate::platform::current().retroarch_data_dir(&self.root)
    }

    /// One value out of RetroArch's own `retroarch.cfg`.
    ///
    /// Its settings are the truth about where its files are; anything derived
    /// from the install root is a guess that happens to be right on a desktop.
    fn config_value(&self, key: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.data_dir().join("retroarch.cfg"))
            .or_else(|_| std::fs::read_to_string(self.root.join("retroarch.cfg")))
            .ok()?;
        text.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
        })
    }

    /// Where per-core settings live: `<config>/<Core>/<Core>.opt`.
    ///
    /// RetroArch's own `rgui_config_directory` first, for the same reason
    /// `shaders_dir` asks: it does not have to be inside the install and on a
    /// handheld it usually is not. On the Thor it is
    /// `/storage/emulated/0/RetroArch/config` while the install lives under
    /// `Android/data`, so deriving it from the root looked in an empty place and
    /// found none of the per-core settings — or the presets that quietly
    /// override ours.
    pub fn config_dir(&self) -> PathBuf {
        if let Some(dir) = self.config_value("rgui_config_directory").filter(|d| !d.is_empty()) {
            let dir = PathBuf::from(dir);
            if dir.is_dir() {
                return dir;
            }
        }
        self.data_dir().join("config")
    }

    /// RetroArch's display name for `core`, as its config directory spells it.
    ///
    /// There is no table that could be complete: the name comes from the core's
    /// own `.info` file, which on Android sits in a directory this app cannot
    /// read. So the directories RetroArch has already made are the source, and
    /// they are matched loosely — `genesis_plus_gx` against `Genesis Plus GX`,
    /// `mupen64plus_next` against `Mupen64Plus-Next` — because the only
    /// difference is punctuation and case.
    ///
    /// The curated list in `tweaks` wins where it has an answer: it carries the
    /// handful whose display name is not a respelling of the core id at all,
    /// like `fbneo` for "FinalBurn Neo".
    pub fn core_config_dir(&self, core: &str) -> Option<PathBuf> {
        let root = self.config_dir();
        if let Some(known) = crate::tweaks::core_dir_name(core) {
            return Some(root.join(known));
        }
        let squashed = |s: &str| {
            s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
        };
        let want = squashed(core);
        std::fs::read_dir(&root).ok()?.flatten().find_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            (e.path().is_dir() && squashed(name) == want).then(|| root.join(name))
        })
    }

    /// Root of the shader trees (`shaders_slang`, `shaders_glsl`).
    ///
    /// RetroArch's own `video_shader_dir` is asked first, because it is the
    /// only answer that is always right: the pack does not have to live inside
    /// the install and on a handheld it usually does not. On the Thor it is on
    /// the card — `/storage/A2FC-A9FB/Saves/shaders` — with 2,037 presets in
    /// it, while `files/shaders` does not exist at all, so guessing from the
    /// install root reported no shader pack on a device that has one.
    pub fn shaders_dir(&self) -> PathBuf {
        if let Some(dir) = self.config_value("video_shader_dir").filter(|d| !d.is_empty()) {
            let dir = PathBuf::from(dir);
            if dir.is_dir() {
                return dir;
            }
        }
        first_existing(
            &[self.data_dir().join("shaders"), self.root.join("shaders")],
            &[
                PathBuf::from("/usr/share/libretro/shaders"),
                PathBuf::from("/usr/local/share/libretro/shaders"),
            ],
        )
    }

    pub fn core_path(&self, core: &str) -> PathBuf {
        self.cores_dir()
            .join(format!("{core}_libretro.{}", crate::cores::lib_extension()))
    }

    pub fn has_core(&self, core: &str) -> bool {
        self.core_path(core).is_file()
    }

    /// Core stems currently installed.
    pub fn installed_cores(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.cores_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| {
                        n.strip_suffix(&format!("_libretro.{}", crate::cores::lib_extension()))
                    })
                    .map(str::to_owned)
            })
            .collect();
        out.sort();
        out
    }

    /// Settings we force at launch, written to a file passed via
    /// `--appendconfig`.
    ///
    /// A RetroArch tuned for a handheld makes poor assumptions on a desktop
    /// launched from another app. These are overlaid for the session only —
    /// the user's own `retroarch.cfg` is never modified.
    const OVERRIDES: &str = "\
# Generated by moose-rack. Applied per launch via --appendconfig.
# Your own retroarch.cfg is not modified.

# Without this RetroArch starts paused whenever its window is not focused,
# which is always true when launched from another application: the game never
# starts and a pause icon sits in the corner.
pause_nonactive = \"false\"

# Bezel/border overlays are for handhelds; on a desktop they show up as
# decorative bars around a small screen.
input_overlay_enable = \"false\"

# A saved custom viewport (e.g. 1920x1440) distorts a 160x144 handheld game in
# a resizable window. Let the core state its own aspect.
aspect_ratio_index = \"21\"
video_aspect_ratio_auto = \"true\"

# Belt and braces: never let a launch from here rewrite the user's config.
config_save_on_exit = \"false\"

# The shader chosen here is the shader that runs.
#
# RetroArch looks for a preset of its own beside each core -- config/<Core>/
# <Core>.slangp -- and that one wins over `video_shader` without saying so.
# One left behind by a handheld, or by pressing \"save core preset\" once, meant
# every NES game came up in crt-royale no matter what this app asked for, and
# every attempt to change it in Settings did nothing. Those files are left
# alone; they are just not consulted.
auto_shaders_enable = \"false\"

# Mouse wired to the light gun controls.
#
# RetroArch keeps gun binds apart from pad binds and ships them unbound, so a
# gun game aims with the pointer -- that part is read directly -- and then the
# trigger does nothing. These cost nothing when no gun is in use: they only
# apply to a port a core has been told holds a gun, which is off unless it is
# switched on for that system in Settings -> Emulators.
#
# Left button fires, right button shoots off-screen (how most gun games
# reload), middle is Start.
input_player1_mouse_index = \"0\"
input_player1_gun_trigger_mbtn = \"1\"
input_player1_gun_offscreen_shot_mbtn = \"2\"
input_player1_gun_start_mbtn = \"3\"
# A save state with no picture is a slot number, and a slot number is not
# something anybody remembers. RetroArch writes the frame beside the state when
# asked; nothing can produce one after the fact.
savestate_thumbnail_enable = \"true\"

input_player2_mouse_index = \"0\"
input_player2_gun_trigger_mbtn = \"1\"
input_player2_gun_offscreen_shot_mbtn = \"2\"
input_player2_gun_start_mbtn = \"3\"

";

    /// Windows-only additions, appended to `OVERRIDES` there.
    ///
    /// Empty of video settings, and that is the fix rather than an omission.
    ///
    /// This block used to force `video_driver = "d3d11"`, `vrr_runloop_enable`
    /// and a two-image swapchain, to stop a window blacking out while being
    /// resized on a 144 Hz display. It did not stop it. What it did do was
    /// take a machine already set to d3d12 and move it to d3d11, which is the
    /// one driver with a known VRR problem: on an Nvidia card with G-Sync,
    /// d3d11 plus "sync to exact content framerate" -- which is what
    /// `vrr_runloop_enable` is -- negotiates a refresh rate several Hz below
    /// the display's, differently on each launch (libretro/RetroArch#14513).
    /// A timing figure that changes per launch is a driver reinitialisation
    /// waiting to happen, and a reinitialisation is a black screen.
    ///
    /// So the whole block is gone. RetroArch's own config decides the video
    /// driver, as it did before, and `retroarch-user.cfg` carries the old
    /// settings commented out for anyone whose display did want them.
    #[cfg(target_os = "windows")]
    const OVERRIDES_OS: &str = "";

    #[cfg(not(target_os = "windows"))]
    const OVERRIDES_OS: &str = "";
    /// As above, appending the user's own settings last so they win.
    ///
    /// RetroArch applies `--appendconfig` entries in order, later overriding
    /// earlier, so anything in `user_config` beats our defaults. That is how a
    /// pinned button map or video filter survives without editing RetroArch's
    /// own config or opening its menu.
    pub fn write_overrides_with(
        &self,
        dir: &Path,
        user_config: Option<&Path>,
    ) -> Result<PathBuf> {
        self.write_overrides_full(dir, user_config, "", Input::default())
    }

    /// As above, with `extra` (per-platform shader settings) inserted before
    /// the user's file — so the user can still override even those.
    pub fn write_overrides_full(
        &self,
        dir: &Path,
        user_config: Option<&Path>,
        extra: &str,
        input: Input<'_>,
    ) -> Result<PathBuf> {
        let Input {
            pad, mirror_players, autofire, autofire_hz, save_state_on_exit, saves_root,
        } = input;
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        let mut body = Self::OVERRIDES.to_owned();
        body.push_str(Self::OVERRIDES_OS);
        body.push_str(&self.hotkeys(pad));
        body.push_str(&self.players(pad, mirror_players));
        body.push_str(&self.autofire(pad, autofire, autofire_hz));
        // Only ever written when asked for. `savestate_auto_save` is one of
        // RetroArch's stickier settings — it writes to the *auto* slot, which
        // is the slot this app resumes from, so leaving it on means a game you
        // glanced at overwrites where you actually stopped.
        body.push_str(&format!(
            "\n# ---- Save state on exit ----\nsavestate_auto_save = \"{}\"\n",
            if save_state_on_exit { "true" } else { "false" }
        ));
        // Where the game's saves land.
        //
        // Written into the same per-launch file as everything else, so it is
        // one setting rather than a folder in this app's settings that the
        // emulator has never heard of — which is what it would be otherwise.
        // `sort_savefiles_enable` **on**, and that is a correction.
        //
        // It used to be off, which meant RetroArch read and wrote
        // `saves/<game>.srm` flat while this app's sync wrote and scanned
        // `saves/<core>/<game>.srm`. So a save pulled from the server was
        // never loaded by the game, and a save the game wrote was never seen
        // by the sync — silently, in both directions, and the symptom was a
        // title screen offering no save to continue from.
        //
        // Per-core is the half that has to win: the core is part of a save's
        // identity on the server, and a flat tree throws it away with no way
        // to get it back. It is also what RetroArch does by default.
        //
        // `sort_savefiles_by_content_enable` is written off rather than left
        // alone: it is a subfolder per *game*, a third layout nothing here
        // reads, and a user's own retroarch.cfg turning it on would break the
        // agreement above without anything saying so.
        if let Some(root) = saves_root {
            let saves = root.join("saves");
            let states = root.join("states");
            // Created here because RetroArch does not create a directory it was
            // told to use; it reports a write error at the moment a game is
            // saved, which is the worst moment to find out.
            let _ = std::fs::create_dir_all(&saves);
            let _ = std::fs::create_dir_all(&states);
            body.push_str(&format!(
                "\n# ---- Saves ----\n\
                 savefile_directory = \"{}\"\n\
                 savestate_directory = \"{}\"\n\
                 sort_savefiles_enable = \"true\"\n\
                 sort_savestates_enable = \"true\"\n\
                 sort_savefiles_by_content_enable = \"false\"\n\
                 sort_savestates_by_content_enable = \"false\"\n",
                saves.display(),
                states.display(),
            ));
        }
        body.push_str(extra);

        if let Some(user) = user_config {
            match std::fs::read_to_string(user) {
                Ok(extra) => {
                    body.push_str(&format!(
                        "\n# ---- from {} (yours; overrides everything above) ----\n",
                        user.display()
                    ));
                    body.push_str(&extra);
                    if !extra.ends_with('\n') {
                        body.push('\n');
                    }
                }
                Err(e) => eprintln!("warning: could not read {}: {e}", user.display()),
            }
        }

        let path = dir.join("retroarch-overrides.cfg");
        std::fs::write(&path, &body)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    /// The pad profile these settings should be generated from.
    ///
    /// One place, because the three callers below each did this lookup and each
    /// was a chance for them to disagree about which pad they were describing —
    /// a config whose hotkeys come from one profile and whose rapid fire comes
    /// from another.
    ///
    /// The driver is asked for rather than assumed: the same controller is
    /// numbered differently under each, so a profile from the wrong directory
    /// is valid, plausible and completely wrong.
    fn pad_profile(&self, device: Option<&str>) -> Option<padprofile::PadProfile> {
        let driver = padprofile::configured_driver(&[
            self.data_dir().join("retroarch.cfg"),
            self.root.join("retroarch.cfg"),
        ]);
        // Android has no per-pad profile to find, and that is not a gap. Its
        // input driver takes raw `KeyEvent` codes, which the operating system
        // fixes — `BUTTON_A` is 96 on every device ever shipped — so the
        // numbers are the same for every controller and RetroArch writes no
        // autoconfig file to read.
        //
        // Keyed off the driver rather than a `cfg`, so it can be exercised on
        // any host and so what decides it is the same thing RetroArch itself
        // decided.
        if driver.as_deref() == Some("android") {
            return Some(padprofile::android());
        }
        let roots = [self.root.join("autoconfig"), self.data_dir().join("autoconfig")];
        padprofile::find_with_driver(&roots, device, driver.as_deref())
            .or_else(|| padprofile::known(device))
    }

    /// The controller hotkey block for `device` (the connected pad's reported
    /// name, if the frontend knows it).
    ///
    /// Generated per launch rather than shipped as fixed numbers: RetroArch
    /// hotkeys take raw driver indices, which differ per controller *and* per
    /// operating system. See [`crate::padprofile`].
    pub fn hotkeys(&self, device: Option<&str>) -> String {
        // Only ever a profile RetroArch itself wrote, or one built in for a pad
        // somebody has actually reported.
        //
        // There used to be a generic Xbox table behind these, used whenever
        // nothing matched. That is worse than nothing. A hotkey index that is
        // wrong does not fail quietly — it binds the modifier to a button or
        // stick used constantly in play, so the menu opens or the game quits
        // while you are moving. A guess is only safe when being wrong is
        // cheap, and here it is not.
        // Both places RetroArch keeps them: beside the binary for a portable
        // install, and in its user-data directory for a normal one. The second
        // is where anything it learned about the connected pad ends up, so
        // searching only the first found shipped defaults and missed the
        // profile that is actually in use.
        match self.pad_profile(device) {
            Some(profile) => padprofile::hotkey_block(&profile),
            None => padprofile::no_profile_note(
                &[self.root.join("autoconfig"), self.data_dir().join("autoconfig")],
                device,
            ),
        }
    }

    /// Auto-fire: the shot button repeats while held.
    ///
    /// Arcade shooters were built around a cabinet button you hammered, and a
    /// run of Metal Slug is a few thousand presses. Every home port of these
    /// games since has offered auto-fire; the arcade originals cannot, because
    /// the hardware had no such thing.
    ///
    /// The arrangement, and why it is this way round: the bottom face button
    /// becomes the repeating one, because that is the one already under the
    /// thumb and the one that hurts. Single shots move to the top face button,
    /// which on a Neo Geo four-button layout is button D — unused by Metal
    /// Slug and by most of the run-and-gun games, so nothing is displaced. A
    /// game that does use all four still works: the top button keeps sending
    /// the shot, it simply no longer sends D.
    ///
    /// RetroArch's "single button (hold)" turbo mode is what does the
    /// repeating: it pulses `input_turbo_default_button` — RetroPad B, which
    /// every arcade core maps to the primary fire — while the turbo button is
    /// held.
    pub fn autofire(&self, device: Option<&str>, on: crate::tweaks::AutoFire, hz: u32) -> String {
        use crate::tweaks::AutoFire;
        if on == AutoFire::Off {
            return String::new();
        }
        let Some(profile) = self.pad_profile(device) else {
            // Nothing known about the pad means nothing written, and rapid
            // fire silently not happening looks exactly like rapid fire not
            // working — which is a different problem with a different fix.
            eprintln!(
                "rapid fire: no autoconfig profile for {}, so nothing was written",
                device.unwrap_or("(no pad)")
            );
            return String::new();
        };
        // The modifier, and only the modifier. Nothing else is bound, moved
        // or cleared: that is the whole point of this arrangement.
        let Some(which) = on.physical() else { unreachable!("Off returned above") };
        let Some(hold) = profile.get(which) else {
            eprintln!("rapid fire: this pad's profile has no {which:?} button");
            return String::new();
        };

        // One press-and-release in frames, and how much of it the button is
        // down. Shared with the Flip's rapid-fire patch — see turbo_timing for
        // why it rounds and why the hold is capped rather than halved.
        let hz = hz.clamp(1, 30);
        let (period, duty) = crate::tweaks::turbo_timing(hz);

        eprintln!("rapid fire: hold {which:?}, {hz} shots a second (turbo mode 3, period {period})");
        format!(
            "\n# ---- Rapid fire ----\n\
             # Mode 3 is \"single button (hold)\": while the button below is\n\
             # held, RetroPad B — the fire button in every arcade core —\n\
             # repeats at the rate set here. Let go and it stops.\n\
             #\n\
             # Hold it *on its own*. RetroArch reports the repeat only on\n\
             # frames where the button is not physically pressed — the real\n\
             # press wins — so holding the modifier and the fire button\n\
             # together gives one continuous shot and no repeat at all. That\n\
             # is why every arrangement that put the repeat on the fire button\n\
             # itself was unplayable, and it is not something a config can fix.\n\
             #\n\
             # Not mode 0. RetroArch calls that one \"classic\" and its own\n\
             # documentation describes holding the modifier and a face button\n\
             # together, but what it actually does is *latch*: the face button\n\
             # is flagged as turbo and stays flagged after everything is\n\
             # released. That is a toggle, and a toggle is dangerous in the\n\
             # games this exists for — an end-of-game countdown that takes one\n\
             # press per second empties in a second when something is holding\n\
             # the button down for you and you have forgotten it is on.\n\
             #\n\
             # This mode was unusable when the modifier *was* the fire button:\n\
             # that button still sent its own shot, so holding it was one\n\
             # continuous press with the repeat on top. On a shoulder there is\n\
             # nothing underneath — LB and RB are unbound in these cores — so\n\
             # the pulse is the only thing the game sees.\n\
             #\n\
             # A modifier rather than a mode, because the arrangements that put\n\
             # the repeat *on* a face button cannot work: that button still\n\
             # sends its own shot, so holding it is one continuous press with\n\
             # the repeat pulsing on top — and in a game where holding fire\n\
             # charges a shot, that is not rapid fire at all. Moving the fire\n\
             # button out of the way instead means remapping the thing the\n\
             # player's hands already know.\n\
             #\n\
             # The rate is a period in frames — how long one press-release\n\
             # cycle lasts — so it runs backwards from shots per second and\n\
             # the conversion happens here rather than in anyone's head.\n\
             input_turbo_mode = \"3\"\n\
             input_turbo_default_button = \"0\"\n\
             input_turbo_period = \"{period}\"\n\
             input_duty_cycle = \"{duty}\"\n\
             {}\n",
            hold.line("player1_turbo"),
        )
    }

    /// The player-port block: how many ports, the stick standing in for the
    /// d-pad on each, and optionally players 2-4 bound like player 1.
    ///
    /// Resolved from the same profile as [`Self::hotkeys`] and by the same
    /// route, because mirroring is only meaningful against the pad the
    /// frontend can actually see.
    pub fn players(&self, device: Option<&str>, mirror: bool) -> String {
        crate::players::config_lines(self.pad_profile(device).as_ref(), mirror)
    }

    /// Which of the keys we just wrote a core's own override file also sets.
        ///
    /// RetroArch applies `config/<Core Name>/<Core Name>.cfg` after everything
    /// passed with `--appendconfig`, so anything in there silently wins. That
    /// is invisible from this side — the config we write is correct and the
    /// emulator behaves as though it is not — and it cost ten rounds of
    /// chasing a rapid fire setting that was being replaced three lines at a
    /// time.
    ///
    /// Compares by key rather than against a list of interesting settings, so
    /// a clash in anything we set is reported rather than only the ones
    /// somebody thought to check.
    pub fn override_clash(&self, core_label: Option<&str>, ours: &Path) -> Vec<String> {
        let Some(label) = core_label else {
            return Vec::new();
        };
        let theirs = self.config_dir().join(label).join(format!("{label}.cfg"));
        let (Ok(ours), Ok(theirs)) =
            (std::fs::read_to_string(ours), std::fs::read_to_string(theirs))
        else {
            return Vec::new();
        };
        // Values, not just key names. A core override that happens to say the
        // same thing we do is not a conflict, and reporting it as one would
        // switch off every override file for a launch that had no argument
        // with any of them.
        let settings = |text: &str| -> Vec<(String, String)> {
            text.lines()
                .filter(|l| !l.trim_start().starts_with('#'))
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.trim().to_owned(), v.trim().trim_matches('"').to_owned()))
                .filter(|(k, _)| !k.is_empty())
                .collect()
        };
        let mine = settings(&ours);
        let mut clash: Vec<String> = settings(&theirs)
            .into_iter()
            .filter(|(k, v)| mine.iter().any(|(mk, mv)| mk == k && mv != v))
            .map(|(k, _)| k)
            .collect();
        clash.sort();
        clash.dedup();
        clash
    }

/// Write a starter user-settings file if none exists yet.
    ///
    /// Seeded with commented examples rather than active values: silently
    /// changing someone's controls is worse than leaving them alone.
    pub fn ensure_user_config(path: &Path) -> Result<bool> {
        if path.exists() {
            return Ok(false);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(path, USER_CONFIG_TEMPLATE)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(true)
    }

    /// RetroArch's own system directory, where cores look for BIOS files.
    pub fn system_dir(&self) -> PathBuf {
        self.system_override.clone().unwrap_or_else(|| self.root.join("system"))
    }

    /// Point BIOS lookups at a folder we control instead of RetroArch's own.
    ///
    /// The canonical BIOS set is kept on the server and synced into the visible
    /// library folder, so a second machine gets an identical set by copying one
    /// directory. RetroArch's own `system/` is left alone.
    ///
    /// An earlier attempt at this set `system_directory` to the *ROM's* folder
    /// and broke every arcade launch, because the MAME BIOS romsets were not
    /// there. The folder handed to this must be a superset — arcade BIOS
    /// included — or the same breakage returns.
    pub fn with_system_dir(mut self, dir: Option<PathBuf>) -> Self {
        // Absolute, always: RetroArch resolves a relative `system_directory`
        // against its own working directory rather than ours.
        self.system_override = dir
            .filter(|d| d.is_dir())
            .map(|d| d.canonicalize().unwrap_or(d));
        self
    }

    /// Copy known BIOS sets sitting beside a ROM into RetroArch's system
    /// directory.
    ///
    /// MAME-family cores look for BIOS only in the system directory, so a
    /// `neogeo.zip` downloaded alongside the games is invisible to them —
    /// "Neo Geo BIOS required" even though the file is right there. Copying is
    /// cheap (a couple of MB) and leaves the download layout untouched.
    pub fn install_bios(&self, rom_dir: &Path) -> Result<usize> {
        const BIOS: &[&str] = &["neogeo.zip", "neocdz.zip", "pgm.zip", "decocass.zip"];
        let dest = self.system_dir();
        std::fs::create_dir_all(&dest).ok();
        let mut n = 0;
        for name in BIOS {
            let src = rom_dir.join(name);
            let dst = dest.join(name);
            if src.is_file() && !dst.is_file() && std::fs::copy(&src, &dst).is_ok() {
                n += 1;
            }
        }
        Ok(n)
    }
    /// As [`Self::launch_command`], additionally appending an override config.
    pub fn launch_command_with(
        &self,
        core: &str,
        rom: &Path,
        fullscreen: bool,
        overrides: Option<&Path>,
    ) -> Result<Command> {
        self.launch_command_full(core, rom, fullscreen, overrides, None, None)
    }

    /// The full form, including an explicit shader preset.
    ///
    /// `--set-shader` rather than the `video_shader` config key. Writing the
    /// key into an `--appendconfig` file looked right and did nothing: for
    /// RetroArch that key is remembered *state*, and what actually loads a
    /// preset with content is this flag, which its own help describes as
    /// "loaded each time content is loaded, effectively overrides automatic
    /// shader presets". Passing an empty string disables shaders explicitly,
    /// so a preset set for the previous game cannot leak into this one.
    pub fn launch_command_full(
        &self,
        core: &str,
        rom: &Path,
        fullscreen: bool,
        overrides: Option<&Path>,
        shader: Option<&Path>,
        entry_slot: Option<u32>,
    ) -> Result<Command> {
        let core_path = self.core_path(core);
        if !core_path.is_file() {
            bail!(
                "core not installed: {}\n  expected at {}",
                core,
                core_path.display()
            );
        }
        if !rom.is_file() {
            bail!("ROM not found: {}", rom.display());
        }
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-L").arg(&core_path).arg(rom);
        if let Some(cfg) = overrides {
            cmd.arg("--appendconfig").arg(cfg);
        }
        match shader {
            Some(p) => {
                cmd.arg(format!("--set-shader={}", p.display()));
            }
            None => {
                cmd.arg("--set-shader=");
            }
        }
        if fullscreen {
            cmd.arg("-f");
        }
        // Start in a save state rather than at the title screen. RetroArch
        // takes the slot number, not a path, which is why the shelf keeps
        // RetroArch's own slot names rather than inventing its own.
        if let Some(slot) = entry_slot {
            cmd.arg(format!("--entryslot={slot}"));
        }
        Ok(cmd)
    }

    /// Write project-local core options and remaps for this platform, and
    /// return the config lines that point RetroArch at them.
    ///
    /// Returns an empty string when the platform needs neither, so the
    /// redirect stays off and every other core keeps using the user's own
    /// per-core settings untouched.
    /// Config line pointing BIOS lookups at our folder, when one is set.
    pub fn system_dir_line(&self) -> String {
        match &self.system_override {
            Some(d) => format!(
                "\n# BIOS come from the library folder, synced from the server, so a\n                 # second machine gets an identical set by copying one directory.\n                 system_directory = \"{}\"\n",
                d.display()
            ),
            None => String::new(),
        }
    }

    pub fn prepare_tweaks(
        &self,
        library_root: &Path,
        platform: &str,
        core: &str,
        autofire: crate::tweaks::AutoFire,
    ) -> String {
        let opts = crate::tweaks::core_options(platform, core);
        let remap = crate::tweaks::remap_with(platform, core, autofire);
        let Some(label) = crate::tweaks::core_dir_name(core) else {
            return String::new();
        };
        // Note the order: a core with neither options nor a remap still has to
        // reach the cleanup below, because "nothing to write" is exactly the
        // case where a file from an older version is left applying itself.
        // Returning early here is what let the arcade remap survive.
        let nothing_to_write = opts.is_empty() && remap.is_empty();
        let dir = library_root.join("retroarch");
        if std::fs::create_dir_all(&dir).is_err() {
            return String::new();
        }
        // Absolute: RetroArch resolves a relative path against its own working
        // directory, not ours, so "./library/..." would silently miss.
        let dir = dir.canonicalize().unwrap_or(dir);

        // Seed from the user's own options for this core so their choices —
        // palette, aspect, sound quality — survive the redirect. Only the keys
        // we care about are then overwritten.
        let user_opt = self
            .config_dir()
            .join(label)
            .join(format!("{label}.opt"));
        let mut lines: Vec<String> = std::fs::read_to_string(&user_opt)
            .map(|s| s.lines().map(str::to_owned).filter(|l| !l.trim().is_empty()).collect())
            .unwrap_or_default();
        for (k, v) in opts {
            lines.retain(|l| l.split('=').next().is_none_or(|key| key.trim() != *k));
            lines.push(format!("{k} = \"{v}\""));
        }
        lines.sort();
        let opts_path = dir.join("core-options.cfg");
        if !opts.is_empty() && std::fs::write(&opts_path, lines.join("\n") + "\n").is_err() {
            return String::new();
        }

        let remaps_dir = dir.join("remaps");
        let core_dir = remaps_dir.join(label);
        let rmp = core_dir.join(format!("{label}.rmp"));
        if remap.is_empty() {
            // Deleted, not merely skipped. Writing nothing leaves whatever was
            // written last time, and RetroArch keeps applying it: an arcade
            // remap from a superseded design — `input_player1_btn_b = "-1"`,
            // `input_player1_btn_x = "0"` — outlived the code that made it and
            // moved the fire button from A to Y on every launch for weeks.
            // Nothing in the app wrote those lines any more, which is exactly
            // why nobody found them by reading it.
            let _ = std::fs::remove_file(&rmp);
        } else if std::fs::create_dir_all(&core_dir).is_ok() {
            let _ = std::fs::write(&rmp, remap.join("\n") + "\n");
        }
        if nothing_to_write {
            return String::new();
        }

        [
                "",
                "# Project-local core options and remaps, so the user's own",
                "# config/<Core>/<Core>.opt is never touched. global_core_options",
                "# is required: without it the per-core file wins and this is ignored.",
                "global_core_options = \"true\"",
                &format!("core_options_path = \"{}\"", opts_path.display()),
                &format!("input_remapping_directory = \"{}\"", remaps_dir.display()),
                "",
            ]
            .join("\n").to_string()
    }
    /// As [`Self::launch`], with an override config appended.
    pub fn launch_with(
        &self,
        core: &str,
        rom: &Path,
        fullscreen: bool,
        overrides: Option<&Path>,
    ) -> Result<std::process::ExitStatus> {
        self.launch_full(core, rom, fullscreen, overrides, None, None)
    }

    /// As [`Self::launch_with`], with an explicit shader preset.
    pub fn launch_full(
        &self,
        core: &str,
        rom: &Path,
        fullscreen: bool,
        overrides: Option<&Path>,
        shader: Option<&Path>,
        entry_slot: Option<u32>,
    ) -> Result<std::process::ExitStatus> {
        let mut cmd =
            self.launch_command_full(core, rom, fullscreen, overrides, shader, entry_slot)?;
        let status = cmd
            .status()
            .with_context(|| format!("spawning {}", self.binary.display()))?;
        Ok(status)
    }
}

/// Render a command the way a shell would accept it.
pub fn render(cmd: &Command) -> String {
    let quote = |s: &str| {
        if s.contains([' ', '\'', '"', '(', ')', '!']) {
            format!("{:?}", s)
        } else {
            s.to_owned()
        }
    };
    let mut parts = vec![quote(&cmd.get_program().to_string_lossy())];
    parts.extend(cmd.get_args().map(|a| quote(&a.to_string_lossy())));
    parts.join(" ")
}

#[cfg(test)]
mod tests {

    /// The merge exists because `CONFIGFILE` replaces the whole config, so
    /// anything the file omits reverts to RetroArch's defaults rather than to
    /// the user's settings. Everything not ours has to survive untouched.
    #[test]
    fn merging_replaces_our_keys_and_keeps_every_other_line() {
        let base = "# their comment\n\
                    video_driver = \"vulkan\"\n\
                    fps_show = \"false\"\n\
                    input_player1_a_btn = \"97\"\n";
        let overlay = "# ours, and this comment must not travel\n\
                       fps_show = \"true\"\n\
                       savefile_directory = \"/card/Saves/saves\"\n";

        let out = super::merge_config(base, overlay);
        assert!(out.contains("# their comment"), "their comments stay");
        assert!(out.contains("video_driver = \"vulkan\""), "untouched settings stay");
        assert!(out.contains("input_player1_a_btn = \"97\""), "their binds stay");
        assert!(out.contains("fps_show = \"true\""), "ours wins");
        assert!(!out.contains("fps_show = \"false\""), "and theirs is gone, not duplicated");
        assert!(out.contains("savefile_directory = \"/card/Saves/saves\""), "new keys appended");
        assert!(!out.contains("must not travel"), "our comments are not carried over");
    }

    /// A replaced key keeps its position, so the merged file still reads like
    /// the user's own — and a key is replaced, never added twice.
    /// A key set twice in the fragment has to leave the file once, with the
    /// later value. The first version consumed only the last of them and then
    /// appended the earlier one at the end, so the file finished with the value
    /// that had been overridden — `auto_shaders_enable` came out "false" after
    /// something had deliberately set it "true".
    #[test]
    fn a_key_set_twice_in_the_fragment_lands_once_with_the_later_value() {
        let out = super::merge_config(
            "auto_shaders_enable = \"x\"\nother = \"keep\"\n",
            "auto_shaders_enable = \"false\"\nauto_shaders_enable = \"true\"\n",
        );
        assert_eq!(out.matches("auto_shaders_enable").count(), 1, "once, not twice");
        assert!(out.contains("auto_shaders_enable = \"true\""), "the later value wins");
        assert!(out.contains("other = \"keep\""));
    }

    #[test]
    fn merging_replaces_in_place_rather_than_appending_a_second_copy() {
        let base = "a = \"1\"\nb = \"2\"\nc = \"3\"\n";
        let out = super::merge_config(base, "b = \"changed\"\n");
        assert_eq!(out, "a = \"1\"\nb = \"changed\"\nc = \"3\"\n");
        assert_eq!(out.matches("b = ").count(), 1, "one b, not two");
    }

    /// The saves folder chosen in settings has to reach RetroArch.
    ///
    /// It is a setting about where *the emulator* writes, so a folder this app
    /// merely remembers is a folder that does nothing: RetroArch keeps using
    /// its own, the saves land somewhere else, and the sync looks in the wrong
    /// place. The only thing that makes it real is these two keys in the file
    /// handed over at launch.
    /// The launch and the sync have to mean the same thing by "where a save
    /// lives", and for a long time they did not.
    ///
    /// RetroArch was told to write `saves/<game>.srm` flat while
    /// `savesync::download_path` wrote `saves/<core>/<game>.srm`. A save
    /// pulled from the server was therefore never loaded by the game and a
    /// save the game wrote was never seen by the sync — in both directions,
    /// silently. The symptom was a title screen with nothing to continue from,
    /// which looks like a corrupt save rather than two halves disagreeing.
    #[test]
    fn the_launch_files_saves_the_same_way_the_sync_reads_them() {
        let dir = std::env::temp_dir().join("moose-rack-ra-sort-test");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let saves = dir.join("MySaves");
        let path = fake(&dir)
            .write_overrides_full(
                &dir,
                None,
                "",
                Input { saves_root: Some(&saves), ..Input::default() },
            )
            .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();

        assert_eq!(val(&body, "sort_savefiles_enable"), "true");
        assert_eq!(val(&body, "sort_savestates_enable"), "true");
        // Per *content* stays off: that is a folder per game, a third layout
        // nothing in this project reads.
        assert_eq!(val(&body, "sort_savefiles_by_content_enable"), "false");

        // And the two agree in fact, not just in spirit: where the sync puts a
        // download is a `saves/<core>/` path under the same root.
        let landed = crate::savesync::download_path(
            &saves,
            "Chrono Trigger (USA).srm",
            crate::savesync::Destination::Core { core: Some("snes9x") },
        );
        assert_eq!(landed, saves.join("saves/snes9x/Chrono Trigger (USA).srm"));
    }

    #[test]
    fn the_chosen_saves_folder_is_written_into_the_launch_config() {
        let dir = std::env::temp_dir().join("moose-rack-ra-saves-test");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let saves = dir.join("MySaves");

        let path = fake(&dir)
            .write_overrides_full(
                &dir,
                None,
                "",
                Input { saves_root: Some(&saves), ..Input::default() },
            )
            .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();

        assert!(
            body.contains(&format!("savefile_directory = \"{}\"", saves.join("saves").display())),
            "battery saves must be pointed at the chosen folder:\n{body}"
        );
        assert!(
            body.contains(&format!("savestate_directory = \"{}\"", saves.join("states").display())),
            "and so must save states:\n{body}"
        );
        // RetroArch does not create these, and reports the failure at the
        // moment somebody saves.
        assert!(saves.join("saves").is_dir(), "the folders are made up front");
        assert!(saves.join("states").is_dir());
    }

    /// Nothing chosen means nothing said, so RetroArch keeps its own default.
    #[test]
    fn no_saves_folder_leaves_retroarch_alone() {
        let dir = std::env::temp_dir().join("moose-rack-ra-saves-none");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = fake(&dir).write_overrides_with(&dir, None).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("savefile_directory"));
        assert!(!body.contains("savestate_directory"));
    }

    use super::*;

    /// A RetroArch whose files are all inside `root`.
    ///
    /// `portable` because otherwise `data_dir()` answers with the *host's*
    /// real location — on this Mac that is `~/Library/Application Support/
    /// RetroArch`, so tests that write a core override were writing into the
    /// developer's own RetroArch install. Portable keeps every path under the
    /// scratch directory, which also makes the fixture answer the same way
    /// under every device scheme rather than only under the host's.
    fn fake(root: &Path) -> RetroArch {
        RetroArch {
            root: root.to_path_buf(),
            binary: root.join("retroarch"),
            portable: true,
            system_override: None,
        }
    }

    /// Give a scratch RetroArch an autoconfig profile, in every driver
    /// directory this OS searches, so the test works the same on all three.
    ///
    /// Needed now that no profile means no hotkeys: a test that wants hotkeys
    /// has to supply the thing they are derived from, which is also a more
    /// honest reflection of how this works in reality.
    fn with_autoconfig(root: &Path) {
        const PROFILE: &str = r#"
input_driver = "test"
input_device = "Xbox Wireless Controller"
input_b_btn = "0"
input_a_btn = "1"
input_y_btn = "2"
input_x_btn = "3"
input_l_btn = "4"
input_r_btn = "5"
input_select_btn = "6"
input_start_btn = "7"
input_up_btn = "h0up"
input_down_btn = "h0down"
input_left_btn = "h0left"
input_right_btn = "h0right"
input_l2_axis = "+2"
input_r2_axis = "+5"
"#;
        for driver in ["mfi", "hid", "xinput", "dinput", "sdl2", "udev", "linuxraw"] {
            let dir = root.join("autoconfig").join(driver);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("xbox.cfg"), PROFILE).unwrap();
        }
    }

    /// Each test gets its own directory under the OS temp dir, named for the
    /// test, so a failure leaves inspectable output and runs cannot collide.
    /// Where RetroArch keeps its shaders and its per-core settings is decided
    /// by RetroArch, not by where the install happens to be.
    ///
    /// This is the bug that made "the shader setting does nothing" on the Thor:
    /// its pack is on a memory card at `Saves/shaders` with 2,037 presets in it,
    /// while `<install>/shaders` does not exist — so deriving the path from the
    /// root reported no shader pack on a device that has one. The config
    /// directory had the same shape of problem, and there it meant missing the
    /// per-core presets that silently override ours.
    #[test]
    fn the_shader_and_config_directories_follow_retroarchs_own_settings() {
        let dir = scratch("ra-dirs");
        let elsewhere = dir.join("card");
        std::fs::create_dir_all(elsewhere.join("shaders")).unwrap();
        std::fs::create_dir_all(elsewhere.join("config")).unwrap();
        std::fs::write(
            dir.join("retroarch.cfg"),
            format!(
                "video_shader_dir = \"{}\"\nrgui_config_directory = \"{}\"\n",
                elsewhere.join("shaders").display(),
                elsewhere.join("config").display()
            ),
        )
        .unwrap();

        let ra = fake(&dir);
        assert_eq!(ra.shaders_dir(), elsewhere.join("shaders"), "its own setting wins");
        assert_eq!(ra.config_dir(), elsewhere.join("config"));
    }

    /// A setting naming a directory that is not there is not an answer.
    #[test]
    fn a_missing_directory_in_the_config_falls_back_to_the_install() {
        let dir = scratch("ra-dirs-missing");
        std::fs::create_dir_all(dir.join("shaders")).unwrap();
        std::fs::write(
            dir.join("retroarch.cfg"),
            "video_shader_dir = \"/nowhere/at/all\"\n",
        )
        .unwrap();
        assert_eq!(fake(&dir).shaders_dir(), dir.join("shaders"));
    }

    /// RetroArch names a core's directory after its *display* name, and there is
    /// no table that could be complete — the name comes from the core's own
    /// `.info` file, which on Android is somewhere this app cannot read. So the
    /// directories RetroArch has already made are the source, matched loosely.
    #[test]
    fn a_cores_config_directory_is_found_across_case_and_punctuation() {
        let dir = scratch("ra-coredir");
        let cfg = dir.join("config");
        for name in ["Genesis Plus GX", "Mupen64Plus-Next", "Snes9x"] {
            std::fs::create_dir_all(cfg.join(name)).unwrap();
        }
        std::fs::write(dir.join("retroarch.cfg"), format!("rgui_config_directory = \"{}\"\n", cfg.display()))
            .unwrap();
        let ra = fake(&dir);

        assert_eq!(ra.core_config_dir("genesis_plus_gx"), Some(cfg.join("Genesis Plus GX")));
        assert_eq!(ra.core_config_dir("mupen64plus_next"), Some(cfg.join("Mupen64Plus-Next")));
        assert_eq!(ra.core_config_dir("snes9x"), Some(cfg.join("Snes9x")));
        // Nothing there and nothing curated: better to say so than to invent a
        // directory and write a preset RetroArch will never look at.
        assert_eq!(ra.core_config_dir("nothing_like_this"), None);
        // The curated list wins where a display name is not a respelling at all.
        assert_eq!(ra.core_config_dir("fbneo"), Some(cfg.join("FinalBurn Neo")));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moose-rack-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("creating a temp dir");
        dir
    }

    /// The user's own file is appended last, because `--appendconfig` gives the
    /// final assignment of a key. This is the promise the README makes: we never
    /// modify their retroarch.cfg and never win an argument with it.
    /// One monitor, at the desktop origin.
    fn screen(width: u32, height: u32) -> Screen {
        Screen { x: 0, y: 0, width, height, primary_height: height }
    }

    fn val(out: &str, key: &str) -> String {
        out.lines()
            .find(|l| l.starts_with(&format!("{key} = ")))
            .and_then(|l| l.split('"').nth(1))
            .unwrap_or_else(|| panic!("{key} is not written:\n{out}"))
            .to_owned()
    }

    /// The keys the documentation names do not exist in RetroArch 1.20, and a
    /// window asked for through them came out at 720 pixels wide because the
    /// size fell back to `video_scale`. These are the ones that work, and they
    /// are only read when saved positions are on.
    #[test]
    fn the_size_is_written_under_the_names_retroarch_actually_reads() {
        let out = window_lines(Some(screen(2560, 1440)), None, true);
        assert_eq!(val(&out, "video_window_save_positions"), "true");
        assert_eq!(val(&out, "video_windowed_position_width"), "2406");
        // The full height of the screen less the menu bar, not a fraction of
        // it: vertical space is the thing being maximised.
        assert_eq!(
            val(&out, "video_windowed_position_height"),
            if cfg!(target_os = "macos") { "1402" } else { "1440" }
        );
        // And the documented names too, for builds that have them.
        assert_eq!(val(&out, "video_window_custom_size_enable"), "true");
        assert_eq!(val(&out, "video_window_width"), "2406");
    }

    /// Top-left, and as tall as the screen allows.
    ///
    /// The vertical coordinate is the part that cannot be reasoned out: asking
    /// for y = 0 on an 1169-tall screen put the window's *bottom* at the bottom
    /// of the screen, because RetroArch hands the number to Cocoa, whose origin
    /// is the bottom-left of the primary display. On the primary monitor that
    /// makes the top of the screen y = 0 only because the window is exactly as
    /// tall as the space below the menu bar.
    #[test]
    #[cfg(target_os = "macos")]
    fn the_window_sits_under_the_menu_bar_at_the_top_left() {
        let out = window_lines(Some(screen(1800, 1169)), None, true);
        assert_eq!(val(&out, "video_windowed_position_x"), "0");
        assert_eq!(val(&out, "video_windowed_position_height"), "1131");
        // 1169 - 0 - (1131 + 38) = 0: the window's bottom edge on the bottom
        // of the screen, its top just under the menu bar.
        assert_eq!(val(&out, "video_windowed_position_y"), "0");
    }

    /// A monitor above the primary one has negative coordinates going down and
    /// positive ones going up. Getting this backwards puts the window off the
    /// bottom of the desktop, which is where it went.
    ///
    /// The horizontal is the other half: a monitor that starts left of the
    /// origin cannot be addressed by its own left edge, because a negative x
    /// does not survive — asking for -378 put the window at 2142, most of it
    /// off the right of the desktop. Zero is still a point on that screen, so
    /// that is where it goes, with the width brought in to match.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_monitor_above_and_left_of_the_primary_one_is_still_reachable() {
        let out = window_lines(Some(Screen {
            x: -380,
            y: -1440,
            width: 2560,
            height: 1440,
            primary_height: 1169,
        }), None, true);
        assert_eq!(val(&out, "video_windowed_position_x"), "0");
        // 2560 wide starting 380 to the left of the origin leaves 2180 usable.
        assert_eq!(val(&out, "video_windowed_position_width"), "2049");
        assert_eq!(val(&out, "video_windowed_position_height"), "1402");
        // 1169 - (-1440) - 1440 = 1169: the window's bottom edge level with the
        // top of the primary screen, which is the bottom of this one.
        assert_eq!(val(&out, "video_windowed_position_y"), "1169");
    }

    /// A screen entirely left of the origin still gets a window, and the
    /// arithmetic must not underflow on the way — these are unsigned widths.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_screen_far_off_to_the_left_does_not_underflow() {
        let out = window_lines(Some(Screen {
            x: -3000,
            y: 0,
            width: 1920,
            height: 1080,
            primary_height: 1169,
        }), None, true);
        assert_eq!(out, "", "a window nobody could play in is not worth asking for");
    }

    /// Everywhere else y counts down from the top, so a monitor's own origin
    /// is already the answer.
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn elsewhere_the_monitor_origin_is_the_position() {
        let out = window_lines(Some(Screen {
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440,
            primary_height: 1080,
        }), None, true);
        assert_eq!(val(&out, "video_windowed_position_x"), "1920");
        assert_eq!(val(&out, "video_windowed_position_y"), "0");
        assert_eq!(val(&out, "video_windowed_position_height"), "1440");
    }

    /// The ceiling is a separate setting that silently wins. RetroArch keeps
    /// whatever `auto_width_max` it was last configured with, so one set up on
    /// a 1080p monitor caps every window at 1920x1080 forever — and on a 4K
    /// screen that is a quarter of the area, which reads as the requested size
    /// being ignored rather than capped.
    #[test]
    fn the_size_is_written_alongside_the_ceiling_that_would_cap_it() {
        let out = window_lines(Some(screen(3840, 2160)), None, true);
        assert_eq!(
            val(&out, "video_windowed_position_width"),
            val(&out, "video_window_auto_width_max"),
            "a ceiling below the requested size caps it"
        );
        assert_eq!(
            val(&out, "video_windowed_position_height"),
            val(&out, "video_window_auto_height_max")
        );
    }

    /// A window the exact size of the display is a maximised one with no title
    /// bar left to grab. It has to come in from the edges.
    #[test]
    fn the_window_is_a_little_smaller_than_the_display() {
        let out = window_lines(Some(screen(1000, 1000)), None, true);
        let w: u32 = val(&out, "video_windowed_position_width").parse().unwrap();
        assert!(w < 1000, "width {w} fills the display, leaving no edge to grab");
        assert!(w > 850, "width {w} wastes the screen");
        // Height is the one thing that is not held back: "as much vertical
        // space as the screen has" is the whole point of the placement.
        let h: u32 = val(&out, "video_windowed_position_height").parse().unwrap();
        assert!(h >= 1000 - super::MENU_BAR, "height {h} is not the full screen");
    }

    /// The black bars are the window being the wrong shape, and on a maximised
    /// window they are large: a 3:2 handheld game on a 16:10 laptop screen
    /// leaves a column down each side wider than the game was tall on the
    /// original hardware. A window shaped like the game has nothing over.
    #[test]
    fn a_window_shaped_like_the_game_leaves_no_room_for_a_bar() {
        // A wide screen, where a maximised window and a 3:2 game plainly
        // disagree. On a 16:10 laptop the two happen to land within a few
        // percent of each other, which would make this pass on a coincidence.
        let bare = window_lines(Some(screen(2560, 1440)), None, true);
        let fitted = window_lines(Some(screen(2560, 1440)), crate::aspect::of("gba"), true);

        let read = |out: &str, k: &str| -> f32 { val(out, k).parse().unwrap() };
        let shape = |out: &str| {
            read(out, "video_windowed_position_width") / read(out, "video_windowed_position_height")
        };
        assert!((shape(&fitted) - 1.5).abs() < 0.01, "{fitted}");
        // And it is genuinely different from the unshaped one, or the test is
        // passing on a coincidence of this particular screen.
        assert!((shape(&bare) - 1.5).abs() > 0.05, "{bare}");
        // Still inside the screen.
        assert!(read(&fitted, "video_windowed_position_width") <= 2560.0);
        assert!(read(&fitted, "video_windowed_position_height") <= 1440.0);
    }

    /// A platform nobody has mapped is left exactly as it was: no shaping of
    /// the window, and no ratio forced on the picture.
    #[test]
    fn a_platform_with_no_one_shape_is_left_alone() {
        let a = window_lines(Some(screen(1800, 1169)), crate::aspect::of("some-new-thing"), true);
        let b = window_lines(Some(screen(1800, 1169)), None, true);
        assert_eq!(a, b);
        assert!(!b.contains("aspect_ratio_index"), "{b}");
    }

    /// Sizing the window was half the job. The picture is drawn at whatever
    /// ratio RetroArch is set to, and the default is the frame buffer's shape
    /// rather than the television's — a Neo Geo frame is 320x224, which is
    /// 10:7, shown as 4:3. So a 4:3 window held a 10:7 picture and RetroArch
    /// put bars in the difference, which is exactly what a fitted window is
    /// supposed to remove.
    #[test]
    fn the_picture_is_drawn_at_the_shape_the_window_was_given() {
        let out = window_lines(Some(screen(1800, 1169)), crate::aspect::of("neogeoaes"), true);
        assert_eq!(val(&out, "aspect_ratio_index"), "20", "20 is \"use the ratio below\"");
        let asked: f32 = val(&out, "video_aspect_ratio").parse().unwrap();
        assert!((asked - 4.0 / 3.0).abs() < 0.001, "{out}");

        // And the window really is that shape, or the two still disagree.
        let w: f32 = val(&out, "video_windowed_position_width").parse().unwrap();
        let h: f32 = val(&out, "video_windowed_position_height").parse().unwrap();
        assert!((w / h - asked).abs() < 0.01, "window {w}x{h} is not {asked}");
    }

    /// RetroArch loads a preset of its own from `config/<Core>/<Core>.slangp`
    /// and that one wins over `video_shader` without saying so anywhere. One
    /// left behind by a handheld, or by pressing "save core preset" once,
    /// meant every NES game came up in crt-royale whatever this app asked for
    /// — and every attempt to change it in Settings appeared to do nothing.
    #[test]
    fn the_shader_we_choose_is_not_overruled_by_one_retroarch_finds_itself() {
        let dir = scratch("auto-shaders");
        let body =
            std::fs::read_to_string(fake(&dir).write_overrides_with(&dir, None).unwrap()).unwrap();
        assert!(
            body.contains("auto_shaders_enable = \"false\""),
            "a preset beside the core will quietly replace ours:\n{body}"
        );
    }

    /// Arcade gets the cabinet's shape for its window and nothing forced on
    /// the picture. Both halves matter: a window the width of the screen puts
    /// a black column down each side of every horizontal game, and a forced
    /// 4:3 would stretch every vertical one.
    #[test]
    fn arcade_is_given_a_cabinet_shaped_window_but_no_forced_ratio() {
        let out = window_lines(Some(screen(2560, 1440)), crate::aspect::of("arcade"), true);
        let w: f32 = val(&out, "video_windowed_position_width").parse().unwrap();
        let h: f32 = val(&out, "video_windowed_position_height").parse().unwrap();
        assert!((w / h - 4.0 / 3.0).abs() < 0.01, "window {w}x{h} is not a cabinet");
        assert!(
            !out.contains("aspect_ratio_index"),
            "a vertical shooter would be stretched to fit:\n{out}"
        );
    }

    /// Auto-fire moves two buttons and nothing else. Getting only half of it
    /// written would be worse than none: a shot button that repeats, with
    /// The bug that put the fire button on Y for weeks.
    ///
    /// A remap written by a superseded design outlived the code that made it.
    /// `prepare_tweaks` only ever *wrote* remaps, so a core it no longer has
    /// anything to say about kept whatever was on disk — and RetroArch went on
    /// applying `input_player1_btn_b = "-1"` on every launch. Nothing in the
    /// app wrote that line any more, which is exactly why reading the app
    /// never found it.
    ///
    /// Put the bug back — make the `remap.is_empty()` branch do nothing
    /// instead of deleting — and this fails.
    #[test]
    fn a_remap_the_app_no_longer_wants_is_deleted_not_left_behind() {
        let dir = scratch("stale-remap");
        let ra = fake(&dir);
        let lib = dir.join("lib");

        // An arcade remap from the old arrangement, exactly as found on disk.
        let stale = lib.join("retroarch/remaps/FinalBurn Neo/FinalBurn Neo.rmp");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::write(
            &stale,
            "input_player1_btn_b = \"-1\"\ninput_player1_btn_x = \"0\"\n",
        )
        .unwrap();
        assert!(stale.exists());

        // fbneo has core options but no remap, so this is the "nothing to
        // write" path that used to return early.
        ra.prepare_tweaks(&lib, "arcade", "fbneo", crate::tweaks::AutoFire::LeftBumper);

        assert!(
            !stale.exists(),
            "the stale arcade remap survived, so fire stays on the wrong button"
        );
    }

    /// A core that still wants a remap keeps it — the cleanup must not be a
    /// blanket delete.
    #[test]
    fn a_remap_the_app_still_wants_is_written() {
        let dir = scratch("live-remap");
        let lib = dir.join("lib");
        fake(&dir).prepare_tweaks(&lib, "nes", "fceumm", crate::tweaks::AutoFire::Off);
        let rmp = lib.join("retroarch/remaps/FCEUmm/FCEUmm.rmp");
        let body = std::fs::read_to_string(&rmp).expect("FCEUmm still wants its X/Y swap");
        assert!(body.contains("input_player1_btn_x"), "the swap was not written: {body}");
    }

    /// RetroArch 1.20.0 calls this `input_duty_cycle`. The *variable* behind it
    /// is `input_turbo_duty_cycle`, which is what the previous reading latched
    /// onto — so the app wrote a key RetroArch has never had and the duty
    /// cycle silently never applied. From `configuration.c`:
    ///
    ///     SETTING_UINT("input_duty_cycle", &settings->uints.input_turbo_duty_cycle, ...)
    #[test]
    fn the_duty_cycle_uses_the_key_retroarch_actually_reads() {
        let dir = scratch("duty-key");
        with_autoconfig(&dir);
        let out = fake(&dir).autofire(
            Some("Xbox Wireless Controller"),
            crate::tweaks::AutoFire::LeftBumper,
            5,
        );
        assert!(
            out.lines().any(|l| l.starts_with("input_duty_cycle")),
            "the duty cycle is on a key RetroArch ignores: {out}"
        );
        assert!(
            !out.contains("input_turbo_duty_cycle"),
            "the key that does nothing is back: {out}"
        );
    }



    /// Y as the modifier is a real arrangement, not a fallback to a shoulder.
    #[test]
    fn the_top_button_can_be_the_modifier() {
        let dir = scratch("top-modifier");
        with_autoconfig(&dir);
        let out = fake(&dir).autofire(
            Some("Xbox Wireless Controller"),
            crate::tweaks::AutoFire::Top,
            5,
        );
        assert!(out.contains("input_player1_turbo"), "no modifier bound: {out}");
        assert!(out.contains("input_turbo_mode = \"3\""), "not the hold mode: {out}");
    }

    /// Off unless asked for, and written either way.
    ///
    /// `savestate_auto_save` writes to RetroArch's *auto* slot, which is the
    /// slot Continue playing resumes from — so leaving it on means a game you
    /// glanced at overwrites where you actually stopped. Written explicitly as
    /// "false" rather than omitted, because the user's own retroarch.cfg may
    /// have it on and silence would let that through.
    #[test]
    fn a_state_on_exit_is_written_only_when_it_was_asked_for() {
        let dir = scratch("save-on-exit");
        with_autoconfig(&dir);
        let line = |on: bool| {
            fake(&dir)
                .write_overrides_full(
                    &dir.join("out"),
                    None,
                    "",
                    crate::retroarch::Input { save_state_on_exit: on, ..Default::default() },
                )
                .map(|p| std::fs::read_to_string(p).unwrap())
                .unwrap()
                .lines()
                .find(|l| l.starts_with("savestate_auto_save"))
                .map(str::to_owned)
                .expect("the key was not written at all")
        };
        assert!(line(true).contains("true"), "asked for and not written");
        assert!(line(false).contains("false"), "not asked for and not disabled");
    }

    /// nowhere left to fire a single shot from, is a game you cannot aim in.
    #[test]
    fn autofire_rate_is_the_nearest_whole_frame_to_what_was_asked() {
        let dir = scratch("autofire-rate");
        with_autoconfig(&dir);
        let period = |hz: u32| {
            let out = fake(&dir).autofire(
                Some("Xbox Wireless Controller"),
                crate::tweaks::AutoFire::LeftBumper,
                hz,
            );
            out.lines()
                .find(|l| l.starts_with("input_turbo_period"))
                .and_then(|l| l.split('"').nth(1))
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
        };
        // The ones that divide 60 exactly.
        assert_eq!(period(4), 15, "four shots a second is 15 frames");
        assert_eq!(period(5), 12);
        assert_eq!(period(10), 6);
        // And the ones that do not. Truncating 60/7 gives 8 frames, which is
        // 7.5 shots a second — every rate that does not divide 60 was drifting
        // fast the same way.
        assert_eq!(period(7), 9, "7 Hz is 8.57 frames, so 9");
        assert_eq!(period(9), 7);
        // The button is down for half the cycle, and never for zero frames.
        for hz in 1..=30 {
            let p = period(hz);
            assert!(p >= 2, "{hz} Hz gave a period of {p}");
        }
    }

    /// The file that beat us for ten rounds.
    ///
    /// RetroArch applies `config/<Core>/<Core>.cfg` after everything passed
    /// with `--appendconfig`, so three lines in there — turbo mode 2, single
    /// button *toggle* — replaced the mode we asked for on every launch. The
    /// config we wrote was correct and the emulator behaved as though it was
    /// not, with nothing anywhere saying why.
    #[test]
    fn a_cores_own_override_is_noticed_key_by_key() {
        let dir = scratch("override-clash");
        let ra = fake(&dir);
        let core_cfg = ra.config_dir().join("Geolith");
        std::fs::create_dir_all(&core_cfg).unwrap();
        std::fs::write(
            core_cfg.join("Geolith.cfg"),
            "input_turbo_mode = \"2\"\ninput_turbo_period = \"6\"\nvideo_smooth = \"true\"\n",
        )
        .unwrap();

        let ours = dir.join("ours.cfg");
        std::fs::write(
            &ours,
            "# a comment mentioning input_turbo_mode\ninput_turbo_mode = \"3\"\n\
             input_turbo_period = \"15\"\ninput_player1_turbo_btn = \"10\"\n",
        )
        .unwrap();

        let clash = ra.override_clash(Some("Geolith"), &ours);
        assert_eq!(clash, vec!["input_turbo_mode", "input_turbo_period"]);
        // Only what both set *differently*: their video_smooth is theirs to
        // keep, and our turbo button is not contested.
        assert!(!clash.iter().any(|k| k == "video_smooth" || k == "input_player1_turbo_btn"));

        // Agreement is not a conflict. Switching off every override file over
        // a setting both sides already agree on would throw away whatever else
        // is in there for nothing.
        std::fs::write(
            core_cfg.join("Geolith.cfg"),
            "input_turbo_mode = \"3\"\ninput_turbo_period = \"15\"\n",
        )
        .unwrap();
        assert!(ra.override_clash(Some("Geolith"), &ours).is_empty());

        // A core with no override of its own, and a core we have no name for.
        assert!(ra.override_clash(Some("SwanStation"), &ours).is_empty());
        assert!(ra.override_clash(None, &ours).is_empty());
    }

    /// Nothing is moved, cleared or swapped. That is the whole reason this
    /// arrangement replaced the last one: putting the repeat *on* a face
    /// button meant either a held shot underneath the repeat or remapping the
    /// fire button somewhere the player's hands do not expect, and a game
    /// where holding fire charges — Pulstar — survives neither.
    #[test]
    fn rapid_fire_moves_no_buttons_at_all() {
        for mode in [crate::tweaks::AutoFire::LeftBumper, crate::tweaks::AutoFire::RightBumper] {
            assert!(
                crate::tweaks::remap_with("arcade", "fbneo", mode).is_empty(),
                "{mode:?} remapped something it did not need to"
            );
        }
    }

    /// A tap, at every rate.
    ///
    /// The button was held for half the cycle, which at slow rates is a long
    /// press — 250ms at 2 shots a second. Games with a charge shot read that
    /// as charging, so the slowest settings on a Neo Geo shooter spent their
    /// time winding up a beam instead of firing.
    #[test]
    fn the_button_is_never_held_long_enough_to_read_as_held() {
        let dir = scratch("autofire-duty");
        with_autoconfig(&dir);
        let field = |hz: u32, key: &str| {
            fake(&dir)
                .autofire(Some("Xbox Wireless Controller"), crate::tweaks::AutoFire::LeftBumper, hz)
                .lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split('"').nth(1))
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
        };
        for hz in 1..=30 {
            let period = field(hz, "input_turbo_period");
            let duty = field(hz, "input_duty_cycle");
            assert!(duty >= 1, "{hz} Hz never presses the button");
            assert!(duty <= 4, "{hz} Hz holds it for {duty} frames, which reads as held");
            assert!(period > duty, "{hz} Hz never lets the button go: {duty}/{period}");
        }
        // The fast end is unchanged: one frame down, one up.
        assert_eq!(field(30, "input_turbo_period"), 2);
        assert_eq!(field(30, "input_duty_cycle"), 1);
    }

    /// Held, not latched.
    ///
    /// Mode 0 — "classic" — reads like the right one and is not: it flags the
    /// face button as turbo and leaves it flagged after everything is
    /// released, which is a toggle. In a game whose end-of-round countdown
    /// takes one press per second, a toggle nobody remembers leaving on
    /// empties the clock in a second.
    #[test]
    fn rapid_fire_is_a_shoulder_modifier_and_binds_only_that() {
        let dir = scratch("autofire");
        with_autoconfig(&dir);
        let out = fake(&dir).autofire(
            Some("Xbox Wireless Controller"),
            crate::tweaks::AutoFire::LeftBumper,
            5,
        );

        assert!(out.contains("input_turbo_mode = \"3\""), "held, not latched: {out}");
        assert!(out.contains("input_player1_turbo_btn"), "{out}");
        // Mode 3 pulses one button: RetroPad B, the fire button in every
        // arcade core.
        assert!(out.contains("input_turbo_default_button = \"0\""), "{out}");
        // Nothing else is bound. A line moving a face button here would be the
        // old arrangement coming back in through the config file.
        assert!(!out.contains("input_player1_b_btn"), "{out}");
        assert!(!out.contains("input_player1_x_btn"), "{out}");
    }

    /// The two shoulders write the same block with a different button, and
    /// they are different buttons — a mode that bound the same one twice would
    /// look like a choice and be none.
    #[test]
    fn the_other_shoulder_is_a_different_button() {
        let dir = scratch("autofire-rb");
        with_autoconfig(&dir);
        let btn = |mode| {
            fake(&dir)
                .autofire(Some("Xbox Wireless Controller"), mode, 5)
                .lines()
                .find(|l| l.starts_with("input_player1_turbo"))
                .map(str::to_owned)
                .unwrap_or_default()
        };
        let l = btn(crate::tweaks::AutoFire::LeftBumper);
        let r = btn(crate::tweaks::AutoFire::RightBumper);
        assert!(!l.is_empty() && !r.is_empty(), "{l} / {r}");
        assert_ne!(l, r, "both shoulders bound the same button");
    }

    /// A pad nothing is known about gets nothing. Guessing a button index here
    /// does not fail quietly — it binds fire to whatever that index happens to
    /// be on the pad in someone's hands.
    #[test]
    fn autofire_writes_nothing_for_a_pad_with_no_profile() {
        let dir = scratch("autofire-unknown");
        assert_eq!(
            fake(&dir).autofire(Some("Some Unheard-of Pad"), crate::tweaks::AutoFire::LeftBumper, 5),
            ""
        );
    }

    /// Nothing known about the display means nothing written: the emulator's
    /// own window settings are the user's, and a guess would overwrite them.
    #[test]
    fn an_unknown_display_leaves_the_window_settings_alone() {
        assert_eq!(window_lines(None, None, true), "");
        assert_eq!(window_lines(Some(screen(0, 1080)), None, true), "");
        assert_eq!(window_lines(Some(screen(1920, 0)), None, true), "");
    }

    #[test]
    fn the_users_own_settings_come_last() {
        let dir = scratch("user-last");
        with_autoconfig(&dir);
        let user = dir.join("mine.cfg");
        std::fs::write(&user, "video_driver = \"vulkan\"\n").unwrap();

        let path = fake(&dir)
            .write_overrides_full(&dir, Some(&user), "video_smooth = \"true\"\n", Input::default())
            .unwrap();
        let body = std::fs::read_to_string(path).unwrap();

        let ours = body.find("input_enable_hotkey_btn").expect("our hotkeys");
        let extra = body.find("video_smooth").expect("the per-platform extra");
        let theirs = body.find("vulkan").expect("the user's line");
        assert!(ours < extra, "per-platform settings layer over ours");
        assert!(extra < theirs, "the user's file layers over everything");
    }

    /// A missing user config is a warning, not a failed launch — the file is
    /// optional and someone deleting it should not be unable to play.
    #[test]
    fn a_missing_user_config_does_not_fail_the_launch() {
        let dir = scratch("missing-user");
        with_autoconfig(&dir);
        let path = fake(&dir)
            .write_overrides_full(&dir, Some(&dir.join("nope.cfg")), "", Input::default())
            .unwrap();
        assert!(std::fs::read_to_string(path).unwrap().contains("input_enable_hotkey_btn"));
    }

    /// The hotkeys are generated from the pad profile, not hardcoded.
    ///
    /// The layout is asserted symbolically: which *button* does what, with the
    /// index looked up the same way the generator looks it up. Pinning literal
    /// numbers here is what let the wrong ones ship — they looked right next to
    /// a matching comment, and were the browser's indices rather than
    /// RetroArch's.
    #[test]
    fn the_hotkeys_come_from_the_pad_profile() {
        use crate::padprofile;

        let dir = scratch("hotkeys");
        with_autoconfig(&dir);
        let body = std::fs::read_to_string(
            fake(&dir).write_overrides_full(&dir, None, "", Input::default()).unwrap(),
        )
        .unwrap();

        // The profile written above is what the block must be derived from.
        let profile = padprofile::find(&dir, None).expect("the autoconfig we just wrote");
        assert!(
            body.contains(&profile.get(padprofile::MODIFIER).unwrap().line("enable_hotkey")),
            "Select must be bound as the modifier"
        );
        for (action, button, _) in padprofile::HOTKEYS {
            let bind = profile.get(*button).expect("fallback covers every hotkey");
            assert!(
                body.contains(&bind.line(action)),
                "{action} should be on {button:?}, as {}",
                bind.line(action)
            );
        }

        // The two settings that make quit survivable.
        assert!(body.contains("quit_press_twice = \"true\""), "quit must confirm");
        assert!(
            body.contains("config_save_on_exit = \"false\""),
            "RetroArch must not write our layered settings back into the user\'s config"
        );
    }

    /// Quit must never sit on the same button as the modifier, and must never
    /// be reachable without it. Holding B and tapping A is an ordinary thing to
    /// do in a game; it should not end one.
    #[test]
    fn quit_is_not_reachable_during_normal_play() {
        use crate::padprofile::{self, Physical};

        let dir = scratch("quit-safety");
        with_autoconfig(&dir);
        let body = std::fs::read_to_string(
            fake(&dir).write_overrides_full(&dir, None, "", Input::default()).unwrap(),
        )
        .unwrap();

        let profile = padprofile::find(&dir, None).expect("the autoconfig we just wrote");
        let modifier = profile.get(padprofile::MODIFIER).unwrap();
        let quit = profile.get(Physical::A).unwrap();
        assert_ne!(modifier.value, quit.value);
        assert!(body.contains(&modifier.line("enable_hotkey")));
        // The face buttons must not be the modifier under any profile.
        for face in [Physical::A, Physical::B, Physical::X, Physical::Y] {
            let bind = profile.get(face).unwrap();
            assert_ne!(
                bind.value, modifier.value,
                "{face:?} is used constantly in play and cannot be the hotkey modifier"
            );
        }
    }

    /// Every emitted line must be a `key = value` assignment, a comment, or
    /// blank. A stray escaped newline once joined the platform block onto the
    /// preceding line, which RetroArch parses as one malformed setting.
    #[test]
    fn every_emitted_line_parses_as_a_setting() {
        let dir = scratch("well-formed");
        let path = fake(&dir).write_overrides_full(&dir, None, "", Input::default()).unwrap();
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, _) = line.split_once(" = ").unwrap_or_else(|| panic!("malformed: {line}"));
            assert!(
                key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "key {key:?} has characters RetroArch will not accept"
            );
        }
    }

    /// Locating reports what it tried rather than a bare "not found", because
    /// the usual cause is an install somewhere unusual (a second drive) and the
    /// list is what tells the user to go set the path in Settings.
    #[test]
    fn a_failed_locate_names_the_paths_it_tried() {
        let missing = scratch("no-retroarch").join("nowhere");
        let err = RetroArch::locate_in(&[missing.display().to_string()])
            .expect_err("there is no RetroArch there")
            .to_string();
        assert!(err.contains(&missing.display().to_string()), "got: {err}");
        assert!(err.contains("config.toml"), "should say how to fix it: {err}");
    }

    /// The starter user config is written once and never overwritten — it holds
    /// the user's own settings after the first run.
    #[test]
    fn the_starter_user_config_is_never_overwritten() {
        let path = scratch("starter").join("user.cfg");
        assert!(RetroArch::ensure_user_config(&path).unwrap(), "written the first time");
        std::fs::write(&path, "input_driver = \"mine\"\n").unwrap();
        assert!(!RetroArch::ensure_user_config(&path).unwrap(), "left alone the second time");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "input_driver = \"mine\"\n");
    }

    /// The change this file exists to guard: with nothing to derive hotkeys
    /// from, none are written.
    ///
    /// A wrong index does not fail quietly — it puts the modifier on a button or
    /// stick used constantly in play, so the menu opens or the game quits while
    /// you are moving. That is exactly what a generic fallback table produced on
    /// a pad it did not describe.
    #[test]
    fn no_profile_means_no_hotkeys_rather_than_a_guess() {
        let dir = scratch("no-profile");
        let body = std::fs::read_to_string(
            fake(&dir).write_overrides_full(&dir, None, "", Input { pad: Some("Some Unusual Pad"), ..Input::default() }).unwrap(),
        )
        .unwrap();

        assert!(
            !body.contains("input_enable_hotkey"),
            "no modifier may be invented: {body}"
        );
        assert!(
            !body.contains("input_exit_emulator"),
            "and certainly not quit"
        );
        // But it must say why, and where it looked.
        assert!(body.contains("No profile matched"), "{body}");
        assert!(body.contains("autoconfig"), "names the directory: {body}");
        assert!(body.contains("run RetroArch once"), "and how to fix it: {body}");
    }

    /// The note is comments only, so it cannot change any setting by accident.
    #[test]
    fn the_no_profile_note_binds_nothing() {
        let dir = scratch("no-profile-lines");
        let body = std::fs::read_to_string(
            fake(&dir).write_overrides_full(&dir, None, "", Input::default()).unwrap(),
        )
        .unwrap();
        // Only the hotkey keys: `input_overlay_enable` and friends are ordinary
        // base settings and have nothing to do with a pad profile.
        let mut names: Vec<String> = vec!["enable_hotkey".to_owned()];
        names.extend(padprofile::HOTKEYS.iter().map(|(a, _, _)| (*a).to_owned()));

        let bound: Vec<&str> = body
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter(|l| names.iter().any(|n| l.contains(&format!("input_{n}_"))))
            .collect();
        assert!(bound.is_empty(), "no hotkey may be bound: {bound:?}");
    }
}
//! Turns a physical gamepad button into the binding RetroArch expects for it.
//!
//! Hotkeys in `retroarch.cfg` (`input_exit_emulator_btn = "0"`) are *raw driver
//! indices*, not the browser's standard-mapping indices and not anything
//! portable. The same Xbox controller reports:
//!
//! | button | macOS (mfi) | Linux (udev) | Windows (dinput) |
//! |--------|-------------|--------------|------------------|
//! | A      | 0           | 0            | 0                |
//! | B      | 8           | 1            | 1                |
//! | Select | 2           | 6            | 6                |
//! | D-pad  | 4..7        | hat `h0up`   | hat `h0up`       |
//! | RT     | axis `+5`   | axis `+5`    | axis `-2`        |
//!
//! So a hardcoded number is wrong on two platforms out of three, and wrong
//! again on the next controller. RetroArch already solves this: it ships an
//! autoconfig profile per controller per driver, mapping raw inputs to the
//! abstract RetroPad. Reading that file and inverting it gives the correct raw
//! binding for whatever is actually plugged in.
//!
//! Note the RetroPad naming is SNES-style, so it does *not* line up with the
//! labels printed on an Xbox pad: RetroPad `b` is the bottom button (Xbox A),
//! RetroPad `a` is the right one (Xbox B). [`Physical`] exists so the rest of
//! the code can say "the A button" and mean the one the user's thumb is on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A button as labelled on the controller in the user's hands, Xbox naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Physical {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    LT,
    RT,
    Select,
    Start,
    L3,
    R3,
    Up,
    Down,
    Left,
    Right,
}

impl Physical {
    /// The RetroPad key this button maps to in an autoconfig profile.
    ///
    /// The face buttons are the whole reason this function exists: RetroArch
    /// names them after a SNES pad, so they are crossed over relative to an
    /// Xbox one. Getting this backwards is what bound "quit" to a button
    /// people press during normal play.
    /// RetroPad's own name for this button, which is what a config key uses.
    pub fn retropad(self) -> &'static str {
        match self {
            Self::A => "b",     // bottom face button
            Self::B => "a",     // right face button
            Self::X => "y",     // left face button
            Self::Y => "x",     // top face button
            Self::LB => "l",
            Self::RB => "r",
            Self::LT => "l2",
            Self::RT => "r2",
            Self::Select => "select",
            Self::Start => "start",
            Self::L3 => "l3",
            Self::R3 => "r3",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// How RetroArch wants a binding written: `input_<action>_btn` for buttons and
/// hats, `input_<action>_axis` for triggers reported as an axis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub suffix: &'static str,
    pub value: String,
}

impl Binding {
    /// The config line for `action`, e.g. `input_exit_emulator_btn = "0"`.
    pub fn line(&self, action: &str) -> String {
        format!("input_{action}_{} = \"{}\"", self.suffix, self.value)
    }
}

/// One autoconfig profile: RetroPad key -> raw binding.
#[derive(Debug, Clone, Default)]
pub struct PadProfile {
    pub device: String,
    pub driver: String,
    binds: BTreeMap<String, Binding>,
}

impl PadProfile {
    /// Parse an autoconfig `.cfg`. Unknown keys are ignored; the file also
    /// carries `_label` entries and comments we have no use for.
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();

        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').to_owned();
            if value.is_empty() || value == "nul" {
                continue;
            }

            match key {
                "input_device" => out.device = value,
                "input_driver" => out.driver = value,
                // `_label` keys describe the same binding in words; skipping
                // them keeps `input_a_btn_label` out of the `a` slot.
                _ if key.ends_with("_label") => {}
                _ => {
                    let Some(rest) = key.strip_prefix("input_") else {
                        continue;
                    };
                    if let Some(name) = rest.strip_suffix("_btn") {
                        out.binds.insert(
                            name.to_owned(),
                            Binding { suffix: "btn", value },
                        );
                    } else if let Some(name) = rest.strip_suffix("_axis") {
                        out.binds.insert(
                            name.to_owned(),
                            Binding { suffix: "axis", value },
                        );
                    }
                }
            }
        }

        out
    }

    /// The raw binding for a physical button, if this pad has one. Absent for,
    /// say, L3/R3 on a pad without stick clicks.
    pub fn get(&self, button: Physical) -> Option<&Binding> {
        self.binds.get(button.retropad())
    }

    fn is_empty(&self) -> bool {
        self.binds.is_empty()
    }
}

/// Autoconfig subdirectories to search, best driver for this OS first.
///
/// RetroArch keeps one directory per input driver and the profiles inside are
/// not interchangeable — the same controller has different indices in each,
/// which is the entire problem this module solves.
const DRIVER_DIRS: &[&str] = if cfg!(target_os = "macos") {
    &["mfi", "hid"]
} else if cfg!(target_os = "windows") {
    // xinput first: it is RetroArch's default input driver on Windows and was
    // missing from this list entirely, so a machine using it found no profile
    // and silently fell through to the built-in table.
    &["xinput", "dinput", "hid", "sdl2"]
} else {
    &["udev", "sdl2", "linuxraw"]
};

/// Find the profile for `device` under `<root>/autoconfig`.
///
/// `device` is the name the frontend reports for the connected pad (the
/// Gamepad API's `id`, e.g. "Xbox Wireless Controller"). Matching is loose
/// because that string and RetroArch's `input_device` rarely agree exactly:
/// vendor/product suffixes, underscores for spaces, and "Wireless Controller"
/// against "Xbox Wireless Controller" are all normal.
pub fn find(root: &Path, device: Option<&str>) -> Option<PadProfile> {
    find_in(&[root.join("autoconfig")], device)
}

/// As [`find`], searching several autoconfig roots in order.
///
/// A portable RetroArch keeps `autoconfig/` beside the binary; a normal Windows
/// or Linux install ships defaults there but writes anything it learns to its
/// user-data directory instead. Searching only the install directory found the
/// shipped profiles and none of the ones RetroArch had actually chosen for the
/// connected pad — which are the ones that are right.
pub fn find_in(roots: &[std::path::PathBuf], device: Option<&str>) -> Option<PadProfile> {
    find_with_driver(roots, device, None)
}

/// As [`find_in`], searching `driver`'s directory before any other.
///
/// This is the whole ballgame. The same controller has different button numbers
/// under every input driver, and RetroArch picks the driver — so the order to
/// search is not a preference, it is a fact recorded in RetroArch's own config
/// as `input_joypad_driver`.
///
/// Guessing it is how the hotkey modifier ended up on a left stick: this machine
/// runs `sdl2`, the search tried `xinput` and `dinput` first, found a perfectly
/// valid profile in the wrong driver's directory, and used its indices.
pub fn find_with_driver(
    roots: &[std::path::PathBuf],
    device: Option<&str>,
    driver: Option<&str>,
) -> Option<PadProfile> {
    // The configured driver first, then the usual order as a fallback for an
    // install that never recorded one.
    let mut order: Vec<&str> = Vec::new();
    if let Some(d) = driver.map(str::trim).filter(|d| !d.is_empty()) {
        order.push(d);
    }
    order.extend(DRIVER_DIRS.iter().copied().filter(|d| Some(*d) != driver));

    roots.iter().find_map(|dir| find_one(dir, device, &order))
}

/// RetroArch's own answer to "which driver is this pad on".
///
/// Read from its config rather than assumed. `input_driver` is the keyboard and
/// mouse; `input_joypad_driver` is the pad, and it is the one that decides
/// button numbering.
pub fn configured_driver(config_paths: &[std::path::PathBuf]) -> Option<String> {
    for path in config_paths {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("input_joypad_driver") else { continue };
            let Some((_, value)) = rest.split_once('=') else { continue };
            let value = value.trim().trim_matches('"').to_owned();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn find_one(dir: &Path, device: Option<&str>, order: &[&str]) -> Option<PadProfile> {
    let wanted = device.map(normalize);

    for driver in order {
        let sub = dir.join(driver);
        let Ok(entries) = std::fs::read_dir(&sub) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "cfg"))
            .collect();
        // Read order from the filesystem is arbitrary; sorting makes the
        // chosen profile the same on every machine and every run.
        files.sort();

        // A driver with exactly one profile is that driver's answer for every
        // pad — macOS ships a single MFi profile covering Xbox, DualSense,
        // Switch Pro and 8BitDo alike, so there is nothing to match against.
        if files.len() == 1
            && wanted.is_none()
            && let Some(p) = read(&files[0])
        {
            return Some(p);
        }

        let Some(wanted) = wanted.as_deref() else {
            if let Some(p) = files.first().and_then(|f| read(f)) {
                return Some(p);
            }
            continue;
        };

        let mut best: Option<(usize, PadProfile)> = None;
        for file in &files {
            let Some(profile) = read(file) else { continue };
            let stem = file.file_stem().unwrap_or_default().to_string_lossy();
            for candidate in [normalize(&profile.device), normalize(&stem)] {
                let Some(score) = overlap(wanted, &candidate) else {
                    continue;
                };
                if best.as_ref().is_none_or(|(b, _)| score > *b) {
                    best = Some((score, profile.clone()));
                }
            }
        }
        if let Some((_, profile)) = best {
            return Some(profile);
        }
        // Single-profile driver, but the name did not match: still better than
        // nothing, since it is the only thing this driver can produce.
        if files.len() == 1
            && let Some(p) = read(&files[0])
        {
            return Some(p);
        }
    }

    None
}

fn read(path: &Path) -> Option<PadProfile> {
    let profile = PadProfile::parse(&std::fs::read_to_string(path).ok()?);
    (!profile.is_empty()).then_some(profile)
}

/// Lowercase, and reduce anything that is not a letter or digit to a single
/// space, so "Xbox_Wireless_Controller" and "Xbox Wireless Controller (STANDARD
/// GAMEPAD Vendor: 045e)" compare on their words.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_owned()
}

/// How many words the two names share, or None if they share nothing
/// meaningful. "controller" alone is not meaningful — nearly every profile
/// contains it, so matching on it picks an arbitrary pad.
fn overlap(a: &str, b: &str) -> Option<usize> {
    const NOISE: &[&str] = &[
        "controller", "gamepad", "wireless", "wired", "standard", "vendor",
        "product", "pad", "usb", "bluetooth", "the", "for",
    ];
    let words: Vec<&str> = b.split(' ').filter(|w| w.len() > 2).collect();
    let shared: Vec<&&str> = words.iter().filter(|w| a.split(' ').any(|x| x == **w)).collect();
    if shared.iter().any(|w| !NOISE.contains(w)) {
        Some(shared.len())
    } else {
        None
    }
}

/// Built-in profiles for pads that do not follow their platform's usual
/// numbering, matched by name.
///
/// A fresh RetroArch install can have no `autoconfig/` directory at all, in
/// which case there is nothing to read and the generic table below is used. That
/// table is right for an Xbox pad and wrong for anything that numbers its
/// buttons differently — and when it is wrong the *modifier* moves, so every
/// hotkey either does nothing or fires on the wrong button.
///
/// Each entry is `(name fragment, profile)`. Kept deliberately short: this is
/// for pads someone has actually reported, not a database.
const KNOWN: &[(&str, &str)] = &[
    // 8BitDo Ultimate 2 in Xbox mode. Reported from RetroArch's own binding
    // screen: Select and Start are the reverse of the usual Xbox order, and the
    // triggers are on axes 4 and 5 rather than a shared axis 2.
    #[cfg(target_os = "windows")]
    (
        "8bitdo",
        r#"
        input_driver = "xinput"
        input_device = "8BitDo Ultimate 2 (built-in)"
        input_b_btn = "0"
        input_a_btn = "1"
        input_y_btn = "2"
        input_x_btn = "3"
        input_l_btn = "4"
        input_r_btn = "5"
        input_start_btn = "6"
        input_select_btn = "7"
        input_l3_btn = "8"
        input_r3_btn = "9"
        input_up_btn = "h0up"
        input_down_btn = "h0down"
        input_left_btn = "h0left"
        input_right_btn = "h0right"
        input_l2_axis = "+4"
        input_r2_axis = "+5"
        "#,
    ),
];

/// A built-in profile for `device`, if one is known for it.
pub fn known(device: Option<&str>) -> Option<PadProfile> {
    let want = normalize(device?);
    KNOWN
        .iter()
        .find(|(fragment, _)| want.replace(' ', "").contains(fragment))
        .map(|(_, text)| PadProfile::parse(text))
}

/// Built-in profiles, used when `<root>/autoconfig` is missing or has nothing
/// for this pad. Taken from RetroArch's own shipped files for an Xbox
/// controller on each platform, which is the common case by a wide margin.
pub fn fallback() -> PadProfile {
    PadProfile::parse(if cfg!(target_os = "macos") {
        // mfi: the single profile macOS uses for every supported pad.
        r#"
        input_driver = "mfi"
        input_device = "mFi Controller (built-in default)"
        input_b_btn = "0"
        input_y_btn = "1"
        input_select_btn = "2"
        input_start_btn = "3"
        input_up_btn = "4"
        input_down_btn = "5"
        input_left_btn = "6"
        input_right_btn = "7"
        input_a_btn = "8"
        input_x_btn = "9"
        input_l_btn = "10"
        input_r_btn = "11"
        input_l3_btn = "14"
        input_r3_btn = "15"
        input_l2_axis = "+4"
        input_r2_axis = "+5"
        "#
    } else if cfg!(target_os = "windows") {
        r#"
        input_driver = "dinput"
        input_device = "XInput Controller (built-in default)"
        input_b_btn = "0"
        input_a_btn = "1"
        input_y_btn = "2"
        input_x_btn = "3"
        input_l_btn = "4"
        input_r_btn = "5"
        input_select_btn = "6"
        input_start_btn = "7"
        input_l3_btn = "8"
        input_r3_btn = "9"
        input_up_btn = "h0up"
        input_down_btn = "h0down"
        input_left_btn = "h0left"
        input_right_btn = "h0right"
        input_l2_axis = "+2"
        input_r2_axis = "-2"
        "#
    } else {
        r#"
        input_driver = "udev"
        input_device = "Xbox Wireless Controller (built-in default)"
        input_b_btn = "0"
        input_a_btn = "1"
        input_y_btn = "2"
        input_x_btn = "3"
        input_l_btn = "4"
        input_r_btn = "5"
        input_select_btn = "6"
        input_start_btn = "7"
        input_l3_btn = "9"
        input_r3_btn = "10"
        input_up_btn = "h0up"
        input_down_btn = "h0down"
        input_left_btn = "h0left"
        input_right_btn = "h0right"
        input_l2_axis = "+2"
        input_r2_axis = "+5"
        "#
    })
}

/// The shipped hotkey layout, in terms of buttons the user can see.
///
/// Every entry fires only while the modifier is held, so these keep their
/// normal in-game meaning.
pub const MODIFIER: Physical = Physical::Select;

pub const HOTKEYS: &[(&str, Physical, &str)] = &[
    ("exit_emulator", Physical::A, "quit (asks twice)"),
    // Left face toggles the shader, right face shows the FPS.
    //
    // By position, not by letter, because the letters move: RetroPad names the
    // faces after a SNES pad and an Xbox pad crosses them over, so "B" is the
    // bottom button on one device and the right one on another. Frank's Thor is
    // Nintendo-labelled — its Y is the left face and its A is the right — and
    // this is the arrangement he asked for on it.
    ("shader_toggle", Physical::X, "shaders on/off"),
    ("fps_toggle", Physical::B, "FPS counter"),
    ("menu_toggle", Physical::Y, "RetroArch menu"),
    ("load_state", Physical::LB, "load state"),
    ("save_state", Physical::RB, "save state"),
    ("hold_fast_forward", Physical::RT, "fast-forward while held"),
    ("shader_prev", Physical::Up, "previous shader"),
    ("shader_next", Physical::Down, "next shader"),
    ("state_slot_decrease", Physical::Left, "previous save slot"),
    ("state_slot_increase", Physical::Right, "next save slot"),
];

/// The pad every Android device has, as an autoconfig profile.
///
/// On Android the numbers are not per-controller. RetroArch's `android` input
/// driver takes raw `KeyEvent` codes, and those are fixed by the operating
/// system — `BUTTON_A` is 96 on every device that has ever shipped, whatever
/// pad is attached. That is the opposite of every other platform, where an
/// index is meaningless without knowing which controller and which driver
/// produced it, and it is why this can be a table at all.
///
/// Checked against Frank's Thor, which had three hotkeys already set by hand:
/// `input_enable_hotkey_btn = "109"` (SELECT), `input_exit_emulator_btn = "96"`
/// (BUTTON_A) and `input_menu_toggle_btn = "100"` (BUTTON_Y) — and against its
/// four saved face binds, which agree with all four rows below.
///
/// Written as an autoconfig file and parsed rather than built by hand, so it
/// goes through exactly the same path as a profile read off disk. RetroPad
/// names the face buttons after a SNES pad, so `b` is the *bottom* one.
///
/// The d-pad is a **hat**, and that is not a guess. The Thor's pad lists
/// `rawAxis=16` and `rawAxis=17` with a range of -1..1 — `ABS_HAT0X` and
/// `ABS_HAT0Y` — so the directions arrive as axis movement, not as key presses.
///
/// Keycodes were tried first, on the strength of the device also advertising a
/// `DPAD` source. That source is the one Android synthesises for navigation; the
/// raw events underneath are still the hat, so `input_up_btn = "19"` matched
/// nothing and the shader hotkeys did nothing at all. RetroArch spells a hat
/// `h0up` and friends, in the `_btn` field.
const ANDROID_PAD: &str = "\
input_device = \"Android gamepad\"
input_driver = \"android\"
input_b_btn = \"96\"
input_a_btn = \"97\"
input_y_btn = \"99\"
input_x_btn = \"100\"
input_l_btn = \"102\"
input_r_btn = \"103\"
input_l2_btn = \"104\"
input_r2_btn = \"105\"
input_l3_btn = \"106\"
input_r3_btn = \"107\"
input_start_btn = \"108\"
input_select_btn = \"109\"
input_up_btn = \"h0up\"
input_down_btn = \"h0down\"
input_left_btn = \"h0left\"
input_right_btn = \"h0right\"
";

/// The Android pad profile. See [`ANDROID_PAD`].
pub fn android() -> PadProfile {
    PadProfile::parse(ANDROID_PAD)
}

/// Render the hotkey block for `profile`.
///
/// Buttons the pad does not report are skipped with a note rather than emitted
/// as a guess: a wrong index is worse than a missing hotkey, because it fires
/// during play.
pub fn hotkey_block(profile: &PadProfile) -> String {
    let mut out = String::from(
        "\n# ---- Controller hotkeys ----\n\
         # RetroArch ships keyboard hotkeys but binds none for a gamepad, so a\n\
         # handheld user has no way out of a game without a keyboard.\n\
         #\n\
         # Indices below are raw driver values, read from RetroArch's own\n\
         # autoconfig profile for this pad — they are not portable between\n\
         # controllers or operating systems, which is why they are generated\n\
         # per launch rather than shipped as fixed numbers.\n",
    );
    out.push_str(&format!(
        "# Profile: {} ({})\n",
        if profile.device.is_empty() { "unknown" } else { &profile.device },
        if profile.driver.is_empty() { "unknown driver" } else { &profile.driver },
    ));

    let Some(modifier) = profile.get(MODIFIER) else {
        // Without the modifier every hotkey below would fire on a bare press.
        out.push_str(
            "# This pad reports no Select button, so no hotkeys are bound: without\n\
             # a modifier they would trigger during normal play.\n",
        );
        return out;
    };
    out.push_str(&format!(
        "{}   # Back / Select / Minus — hold for all of these\n\n",
        modifier.line("enable_hotkey")
    ));

    for (action, button, note) in HOTKEYS {
        match profile.get(*button) {
            Some(bind) => {
                out.push_str(&format!("{:<38} # + {button:?} -> {note}\n", bind.line(action)))
            }
            None => out.push_str(&format!("# no {button:?} on this pad, so no {note}\n")),
        }
    }

    out.push_str(
        "\n# Quit asks once rather than dropping the game instantly — losing\n\
         # unsaved progress to a stray press is not recoverable.\n\
         quit_press_twice = \"true\"\n\
         \n\
         # Opening the menu should pause, not leave the game running underneath.\n\
         menu_pause_libretro = \"true\"\n",
    );
    out
}

/// What to write when no profile could be found.
///
/// Comments only — deliberately no bindings. RetroArch will still map the pad
/// for *play* from its own autoconfig; what is missing is only the hotkey
/// layer, and missing it costs a shortcut. Guessing it costs a game.
///
/// The note says which directory was searched, because the usual cause is a
/// RetroArch that has never been run and therefore has no `autoconfig/` at all.
pub fn no_profile_note(roots: &[std::path::PathBuf], device: Option<&str>) -> String {
    format!(
        "\n# ---- Controller hotkeys ----\n\
         # None bound. RetroArch takes raw driver button indices for these, and\n\
         # they differ per controller and per operating system, so they are read\n\
         # out of RetroArch's own autoconfig profile for the connected pad.\n\
         #\n\
         # No profile matched{}.\n\
         # Searched:\n{}\n\
         #\n\
         # If that directory does not exist, run RetroArch once so it writes its\n\
         # configuration, then launch again. Guessing the indices instead would\n\
         # put the modifier on a button or stick used during play, which quits\n\
         # games mid-session -- so nothing is written rather than something\n\
         # probably wrong.\n",
        match device {
            Some(d) => format!(" for {d:?}"),
            None => String::new(),
        },
        roots
            .iter()
            .map(|r| format!("#   {}", r.display()))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[cfg(test)]
mod tests {

    /// The Android table has to agree with what RetroArch itself wrote on a
    /// real device — these three were already in Frank's config, set by hand
    /// through RetroArch's own menu, and they are the check that the keycodes
    /// below are the ones the driver actually sees.
    #[test]
    fn the_android_pad_matches_what_the_device_reports() {
        let p = android();
        assert_eq!(p.get(Physical::Select).unwrap().value, "109", "SELECT");
        assert_eq!(p.get(Physical::A).unwrap().value, "96", "bottom face, BUTTON_A");
        assert_eq!(p.get(Physical::Y).unwrap().value, "100", "top face, BUTTON_Y");
        // RetroPad names the faces after a SNES pad, so these are crossed over.
        assert_eq!(p.get(Physical::B).unwrap().value, "97", "right face, BUTTON_B");
        assert_eq!(p.get(Physical::X).unwrap().value, "99", "left face, BUTTON_X");
    }

    /// The d-pad carries the shader hotkeys, and the face buttons are arranged
    /// by position rather than by letter — the Thor is Nintendo-labelled, so its
    /// Y is the left face and its A is the right one.
    #[test]
    fn the_android_pad_binds_the_dpad_and_the_faces_by_position() {
        let p = android();
        // A hat, not keycodes: the pad reports ABS_HAT0X/Y, so key 19 never comes.
        assert_eq!(p.get(Physical::Up).unwrap().value, "h0up");
        assert_eq!(p.get(Physical::Down).unwrap().value, "h0down");
        assert_eq!(p.get(Physical::Left).unwrap().value, "h0left");
        assert_eq!(p.get(Physical::Right).unwrap().value, "h0right");

        let block = hotkey_block(&p);
        assert!(block.contains("input_shader_prev_btn = \"h0up\""), "up cycles shaders back");
        assert!(block.contains("input_shader_next_btn = \"h0down\""), "down cycles shaders on");
        assert!(block.contains("input_state_slot_decrease_btn = \"h0left\""), "left picks a slot");
        assert!(block.contains("input_state_slot_increase_btn = \"h0right\""), "right picks a slot");
        // Left face is the shader toggle, right face is the FPS counter.
        assert!(block.contains("input_shader_toggle_btn = \"99\""), "left face, BUTTON_X");
        assert!(block.contains("input_fps_toggle_btn = \"97\""), "right face, BUTTON_B");
        // Unchanged, and confirmed on the device.
        assert!(block.contains("input_exit_emulator_btn = \"96\""), "bottom face quits");
        assert!(block.contains("input_menu_toggle_btn = \"100\""), "top face opens the menu");
    }
    use super::*;

    /// RetroArch's real mfi profile — the one macOS uses for every pad.
    const MFI: &str = r#"
input_driver = "mfi"
input_device = "mFi Controller"
input_b_btn = "0"
input_y_btn = "1"
input_select_btn = "2"
input_start_btn = "3"
input_up_btn = "4"
input_down_btn = "5"
input_left_btn = "6"
input_right_btn = "7"
input_a_btn = "8"
input_x_btn = "9"
input_l_btn = "10"
input_r_btn = "11"
input_l3_btn = "14"
input_r3_btn = "15"
input_l2_axis = "+4"
input_r2_axis = "+5"
input_b_btn_label = "A/Cross"
input_a_btn_label = "B/Circle"
"#;

    /// RetroArch's udev profile for an Xbox pad on Linux. Same controller,
    /// different numbers, and the d-pad is a hat rather than four buttons.
    const UDEV: &str = r#"
input_driver = "udev"
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

    /// The face buttons are the trap: RetroPad is named after a SNES pad, so
    /// `input_b_btn` is the button an Xbox controller prints "A" on. Reading
    /// it the obvious way binds quit to a button used constantly in play,
    /// which is exactly the bug this module was written to fix.
    #[test]
    fn face_buttons_are_read_by_position_not_by_letter() {
        let p = PadProfile::parse(MFI);
        assert_eq!(p.get(Physical::A).unwrap().value, "0", "Xbox A is RetroPad b");
        assert_eq!(p.get(Physical::B).unwrap().value, "8", "Xbox B is RetroPad a");
        assert_eq!(p.get(Physical::X).unwrap().value, "1", "Xbox X is RetroPad y");
        assert_eq!(p.get(Physical::Y).unwrap().value, "9", "Xbox Y is RetroPad x");
    }

    /// The numbers Frank read off RetroArch's own binding screen, macOS + Xbox.
    #[test]
    fn the_mfi_profile_matches_what_retroarch_reports() {
        let p = PadProfile::parse(MFI);
        for (button, expected) in [
            (Physical::A, "0"),
            (Physical::X, "1"),
            (Physical::Select, "2"),
            (Physical::Start, "3"),
            (Physical::Up, "4"),
            (Physical::Down, "5"),
            (Physical::Left, "6"),
            (Physical::Right, "7"),
            (Physical::B, "8"),
            (Physical::Y, "9"),
            (Physical::LB, "10"),
            (Physical::RB, "11"),
        ] {
            assert_eq!(p.get(button).unwrap().value, expected, "{button:?}");
        }
    }

    /// The same physical button has a different index per driver. A fixed
    /// number cannot be right on more than one platform.
    #[test]
    fn the_same_button_differs_between_drivers() {
        let (mfi, udev) = (PadProfile::parse(MFI), PadProfile::parse(UDEV));
        assert_eq!(mfi.get(Physical::B).unwrap().value, "8");
        assert_eq!(udev.get(Physical::B).unwrap().value, "1");
        assert_eq!(mfi.get(Physical::Select).unwrap().value, "2");
        assert_eq!(udev.get(Physical::Select).unwrap().value, "6");
    }

    /// A hat d-pad goes in a `_btn` key as `h0up`; a trigger axis needs
    /// `_axis`. Writing either into the wrong key silently does nothing.
    #[test]
    fn hats_and_axes_get_the_key_retroarch_expects() {
        let p = PadProfile::parse(UDEV);
        let up = p.get(Physical::Up).unwrap();
        assert_eq!(up.line("shader_prev"), "input_shader_prev_btn = \"h0up\"");
        let rt = p.get(Physical::RT).unwrap();
        assert_eq!(rt.line("hold_fast_forward"), "input_hold_fast_forward_axis = \"+5\"");
    }

    #[test]
    fn label_keys_do_not_overwrite_bindings() {
        let p = PadProfile::parse(MFI);
        assert_eq!(p.get(Physical::A).unwrap().value, "0", "not \"A/Cross\"");
    }

    /// The generated block must bind the modifier and must not bind quit to a
    /// bare button. This is the regression that made games exit mid-play.
    #[test]
    fn quit_is_never_reachable_without_the_modifier() {
        for text in [MFI, UDEV] {
            let profile = PadProfile::parse(text);
            let block = hotkey_block(&profile);
            let modifier = profile.get(MODIFIER).unwrap();
            assert!(
                block.contains(&modifier.line("enable_hotkey")),
                "the Select modifier must be bound"
            );
            let exit = profile.get(Physical::A).unwrap();
            assert!(block.contains(&exit.line("exit_emulator")));
            assert_ne!(
                modifier.value,
                exit.value,
                "the modifier and quit must not be the same button"
            );
        }
    }

    /// Guards against the previous behavior, where standard-mapping indices
    /// were written out verbatim: on mfi, index 8 is B and index 0 is A, so
    /// `enable_hotkey = 8` plus `exit = 0` meant B+A quit the game.
    #[test]
    fn the_old_standard_mapping_indices_are_gone() {
        let block = hotkey_block(&PadProfile::parse(MFI));
        assert!(
            !block.contains("input_enable_hotkey_btn = \"8\""),
            "8 is the B button on mfi, not Select — this was the accidental-exit bug"
        );
        assert!(block.contains("input_enable_hotkey_btn = \"2\""));
    }

    /// A pad with no Select button gets no hotkeys at all, rather than a set
    /// that fires on bare presses.
    #[test]
    fn a_pad_without_a_modifier_gets_no_hotkeys() {
        let block = hotkey_block(&PadProfile::parse("input_b_btn = \"0\"\n"));
        assert!(!block.contains("input_exit_emulator"));
        assert!(block.contains("no hotkeys are bound"));
    }

    /// A missing button is skipped, not guessed at.
    #[test]
    fn a_missing_button_is_noted_rather_than_invented() {
        let block = hotkey_block(&PadProfile::parse(
            "input_select_btn = \"2\"\ninput_b_btn = \"0\"\n",
        ));
        assert!(block.contains("input_exit_emulator_btn = \"0\""));
        assert!(!block.contains("input_save_state"), "no RB on this pad");
        assert!(block.contains("no RB on this pad"));
    }

    #[test]
    fn the_built_in_fallback_covers_every_hotkey() {
        let p = fallback();
        assert!(p.get(MODIFIER).is_some());
        for (action, button, _) in HOTKEYS {
            assert!(p.get(*button).is_some(), "fallback is missing {button:?} for {action}");
        }
    }

    #[test]
    fn device_names_match_across_punctuation_and_suffixes() {
        assert!(overlap(&normalize("Xbox Wireless Controller"), &normalize("Xbox_Wireless_Controller")).is_some());
        assert!(overlap(
            &normalize("Xbox Wireless Controller (STANDARD GAMEPAD Vendor: 045e Product: 0b13)"),
            &normalize("Xbox Wireless Controller")
        )
        .is_some());
        // "Controller" alone is shared by nearly every profile and must not
        // count as a match, or an arbitrary pad wins.
        assert!(overlap(&normalize("DualSense Wireless Controller"), &normalize("Xbox Controller")).is_none());
    }

    /// The reported failure: an 8BitDo Ultimate 2 on Windows numbers Select and
    /// Start the reverse of an Xbox pad, so the generic table put the hotkey
    /// modifier on Start. Every hotkey then needed the wrong button held, which
    /// reads as "hotkeys do not work on Windows".
    #[test]
    #[cfg(target_os = "windows")]
    fn a_known_pad_beats_the_generic_table() {
        let p = known(Some("8BitDo Ultimate 2 Wireless Controller")).expect("a known pad");
        assert_eq!(p.get(MODIFIER).unwrap().value, "7", "Select is 7 on this pad");
        assert_eq!(p.get(Physical::Start).unwrap().value, "6");
        assert_eq!(p.get(Physical::RT).unwrap().suffix, "axis");
        assert_eq!(p.get(Physical::RT).unwrap().value, "+5");

        // The generic table disagrees, which is the whole reason the entry
        // exists.
        assert_ne!(
            fallback().get(MODIFIER).unwrap().value,
            p.get(MODIFIER).unwrap().value
        );
    }

    /// Matching is on the name, so an unrelated pad still gets the generic
    /// table rather than someone else's numbers.
    #[test]
    fn an_unknown_pad_matches_nothing() {
        assert!(known(Some("Xbox Wireless Controller")).is_none());
        assert!(known(Some("DualSense Wireless Controller")).is_none());
        assert!(known(None).is_none());
    }

    /// Every built-in entry must cover the modifier and every hotkey, or it is
    /// worse than the generic table it replaces.
    #[test]
    fn every_known_profile_covers_the_full_hotkey_set() {
        for (name, text) in KNOWN {
            let p = PadProfile::parse(text);
            assert!(p.get(MODIFIER).is_some(), "{name} has no Select");
            for (action, button, _) in HOTKEYS {
                assert!(p.get(*button).is_some(), "{name} is missing {button:?} for {action}");
            }
        }
    }

    /// The reported failure, as a test. This machine runs `input_joypad_driver
    /// = "sdl2"`, and searching xinput/dinput first found a valid profile in
    /// the wrong driver's directory — same pad, different numbers, modifier on
    /// a stick.
    #[test]
    fn the_configured_driver_is_searched_first() {
        let dir = std::env::temp_dir().join("moose-rack-pad-driver");
        std::fs::remove_dir_all(&dir).ok();

        // One directory this OS searches by default, and one it does not — the
        // point being that a configured driver is used even when it is not in
        // the built-in order, which is the case on the machine that reported
        // this.
        let usual = DRIVER_DIRS[0];
        for (driver, select) in [(usual, "6"), ("sdl2", "4")] {
            let sub = dir.join("autoconfig").join(driver);
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(
                sub.join("pad.cfg"),
                format!(
                    "input_device = \"Test Pad\"\ninput_driver = \"{driver}\"\n\
                     input_select_btn = \"{select}\"\ninput_b_btn = \"0\"\n"
                ),
            )
            .unwrap();
        }

        let roots = [dir.join("autoconfig")];
        let picked = find_with_driver(&roots, Some("Test Pad"), Some("sdl2")).unwrap();
        assert_eq!(picked.driver, "sdl2", "the configured driver wins");
        assert_eq!(picked.get(MODIFIER).unwrap().value, "4");

        // With nothing configured, the usual order applies and finds the other
        // one — so an install that never recorded a driver still works.
        let fallback = find_with_driver(&roots, Some("Test Pad"), None).unwrap();
        assert_eq!(fallback.driver, usual);
        assert_eq!(fallback.get(MODIFIER).unwrap().value, "6");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `input_driver` is the keyboard and mouse. Reading it instead of
    /// `input_joypad_driver` picks the wrong directory on any machine where
    /// they differ — which is this one: dinput for input, sdl2 for the pad.
    #[test]
    fn the_pad_driver_is_read_not_the_keyboard_one() {
        let dir = std::env::temp_dir().join("moose-rack-pad-cfg");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("retroarch.cfg");
        std::fs::write(
            &cfg,
            "input_driver = \"dinput\"\ninput_joypad_driver = \"sdl2\"\n",
        )
        .unwrap();

        assert_eq!(configured_driver(std::slice::from_ref(&cfg)).as_deref(), Some("sdl2"));

        // A config without one is not an error; the usual order applies.
        let bare = dir.join("bare.cfg");
        std::fs::write(&bare, "video_driver = \"d3d11\"\n").unwrap();
        assert_eq!(configured_driver(&[bare]), None);
        // Nor is a missing file.
        assert_eq!(configured_driver(&[dir.join("nope.cfg")]), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
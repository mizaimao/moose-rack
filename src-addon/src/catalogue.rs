//! Every patch, as data.
//!
//! The bodies and the files are compiled in. That is the whole point of the
//! addon: *"even on a newly installed KNULLI we can easily configure and
//! recover all of those customized settings"* — which only works if recovery
//! is one binary and one profile, not a binary plus a directory of assets to
//! remember.
//!
//! Each entry's `detail` names the file it writes and says why that file and
//! not the obvious one. Every one of these was placed by hand first, and the
//! hard part was never the change.

use crate::patch::{Choice, Patch, Paths, Step};

const HOTKEYS: &str = include_str!("../assets/hotkeys.conf");
const SHADERS_LCD: &str = include_str!("../assets/shaders.conf");
const SHADERS_PLAIN: &str = include_str!("../assets/shaders-plain.conf");
const SHADERS_ZFAST: &str = include_str!("../assets/shaders-zfast.conf");
const POWER: &str = include_str!("../assets/power.conf");
const CHARGE_AWAKE: &str = include_str!("../assets/charge-awake.conf");
const WIFI_AWAKE: &str = include_str!("../assets/wifi-awake.sh");

const SHADER_1: &[u8] = include_bytes!("../../device/retroarch-shaders/1-sharp-shimmerless.glslp");
const SHADER_2: &[u8] =
    include_bytes!("../../device/retroarch-shaders/2-sharp-shimmerless-scanlines.glslp");
const SHADER_3: &[u8] = include_bytes!("../../device/retroarch-shaders/3-sharp-shimmerless-lcd.glslp");
const SHADER_4: &[u8] = include_bytes!("../../device/retroarch-shaders/4-zfast-crt.glslp");

const SET_LCD: &[u8] = include_bytes!("../assets/shadersets/lcd.yml");
const SET_PLAIN: &[u8] = include_bytes!("../assets/shadersets/plain.yml");
const SET_ZFAST: &[u8] = include_bytes!("../assets/shadersets/zfast.yml");

const BEZEL_PNG: &[u8] = include_bytes!("../../device/gba-bezel/systems/gba-4_3.png");
const BEZEL_INFO: &[u8] = include_bytes!("../../device/gba-bezel/systems/gba-4_3.info");

const BLANK_LOGO: &[u8] = include_bytes!("../../device/splash/blank-logo.png");
const BOOT_HOOK: &[u8] = include_bytes!("../../device/splash/boot-custom.sh");
/// Presence is the switch; the hook reads nothing out of it.
const EVMAPY_FLAG: &[u8] = b"moose-patch: evmapy guard on\n";
const ES_INPUT: &[u8] = include_bytes!("../../device/hotkey/es_input.cfg");
const TRIGGERS: &str = include_str!("../../device/hotkey/multimedia_keys.append");

/// Clears the framebuffer, where `S03system-splash` leaves the KNULLI logo.
const CLEAR_FB: &str = r#"case "$1" in
  start)
    if [ -e /dev/fb0 ] && [ -r /sys/class/graphics/fb0/virtual_size ]; then
      W=$(cut -d, -f1 /sys/class/graphics/fb0/virtual_size)
      H=$(cut -d, -f2 /sys/class/graphics/fb0/virtual_size)
      dd if=/dev/zero of=/dev/fb0 bs=4096 count=$(( W * H * 4 / 4096 )) 2>/dev/null
    fi
    ;;
esac"#;

fn block(paths: &Paths, id: &str, body: Option<&str>) -> Step {
    Step::Block {
        file: paths.knulli_conf(),
        id: id.into(),
        body: body.map(str::to_string),
        seed: None,
    }
}

fn startup(paths: &Paths, id: &str, body: Option<&str>) -> Step {
    Step::Block {
        file: paths.user_startup(),
        id: id.into(),
        body: body.map(str::to_string),
        seed: None,
    }
}

fn place(paths: &Paths, path: std::path::PathBuf, bytes: Option<&'static [u8]>) -> Step {
    let backup = paths.backup_for(&path);
    Step::Place { path, bytes, backup }
}

/// The four presets, plus whichever set is being chosen. Every option lays
/// down all four, so the cycle is the same list whichever one you start from.
fn shader_files(paths: &Paths, set: Option<(&str, &'static [u8])>) -> Vec<Step> {
    let presets: [(&str, Option<&'static [u8]>); 4] = match set {
        Some(_) => [
            ("1-sharp-shimmerless.glslp", Some(SHADER_1)),
            ("2-sharp-shimmerless-scanlines.glslp", Some(SHADER_2)),
            ("3-sharp-shimmerless-lcd.glslp", Some(SHADER_3)),
            ("4-zfast-crt.glslp", Some(SHADER_4)),
        ],
        None => [
            ("1-sharp-shimmerless.glslp", None),
            ("2-sharp-shimmerless-scanlines.glslp", None),
            ("3-sharp-shimmerless-lcd.glslp", None),
            ("4-zfast-crt.glslp", None),
        ],
    };
    let mut steps: Vec<Step> = presets
        .iter()
        .map(|(name, bytes)| place(paths, paths.shader(name), *bytes))
        .collect();
    for (name, body) in [
        ("moose-lcd", SET_LCD),
        ("moose-plain", SET_PLAIN),
        ("moose-zfast", SET_ZFAST),
    ] {
        steps.push(place(paths, paths.shaderset(name), set.map(|_| body)));
    }
    steps
}

fn on_off(name_on: &str, on: Vec<Step>, off: Vec<Step>) -> Vec<Choice> {
    vec![
        Choice { name: "off".into(), steps: off },
        Choice { name: name_on.into(), steps: on },
    ]
}

/// One system's bezel. `ours` is whether we carry artwork for it — only GBA,
/// so far, and only because the stock one is a washed-out khaki.
fn bezel(paths: &Paths, slug: &'static str, name: &'static str, ours: bool) -> Patch {
    let id: &'static str = Box::leak(format!("bezel-{slug}").into_boxed_str());
    let title: &'static str = Box::leak(format!("{name} bezel").into_boxed_str());
    let detail: &'static str = Box::leak(
        format!(
            "{slug}.bezel in knulli.conf. KNULLI's own artwork lives on the squashfs and \
             does not survive an upgrade; ours goes in /userdata/decorations, which does. \
             The game keeps the full width either way — the border only fills the strip \
             this system's picture leaves empty on a 4:3 screen."
        )
        .into_boxed_str(),
    );

    let mut choices = vec![
        Choice {
            name: "off".into(),
            steps: vec![
                block(paths, &format!("bezel-{slug}"), None),
                place(paths, paths.decoration(&format!("{slug}-4_3.png")), None),
                place(paths, paths.decoration(&format!("{slug}-4_3.info")), None),
            ],
        },
        Choice {
            name: "KNULLI".into(),
            steps: vec![
                block(paths, &format!("bezel-{slug}"), Some(&format!("{slug}.bezel=default-knulli"))),
                place(paths, paths.decoration(&format!("{slug}-4_3.png")), None),
                place(paths, paths.decoration(&format!("{slug}-4_3.info")), None),
            ],
        },
    ];
    if ours {
        choices.push(Choice {
            name: "silver".into(),
            steps: vec![
                block(paths, &format!("bezel-{slug}"), Some(&format!("{slug}.bezel=moose"))),
                place(paths, paths.decoration(&format!("{slug}-4_3.png")), Some(BEZEL_PNG)),
                place(paths, paths.decoration(&format!("{slug}-4_3.info")), Some(BEZEL_INFO)),
            ],
        });
    }
    Patch { id, title, detail, choices }
}

/// One system's shader, over the global set.
fn shader(paths: &Paths, slug: &'static str, name: &'static str) -> Patch {
    let id: &'static str = Box::leak(format!("shader-{slug}").into_boxed_str());
    let title: &'static str = Box::leak(format!("{name} shader").into_boxed_str());
    let detail: &'static str = Box::leak(
        format!(
            "{slug}.shaderset in knulli.conf, which wins over the global one. Worth setting \
             when a bezel already draws an LCD grid, because two grids stacked look dirty. \
             Left at 'follow global' there is no {slug} line in the file at all."
        )
        .into_boxed_str(),
    );
    Patch {
        id,
        title,
        detail,
        choices: vec![
            Choice {
                name: "follow global".into(),
                steps: vec![block(paths, &format!("shader-{slug}"), None)],
            },
            Choice {
                name: "shimmerless plain".into(),
                steps: vec![block(
                    paths,
                    &format!("shader-{slug}"),
                    Some(&format!("{slug}.shaderset=moose-plain")),
                )],
            },
            Choice {
                name: "shimmerless + LCD".into(),
                steps: vec![block(
                    paths,
                    &format!("shader-{slug}"),
                    Some(&format!("{slug}.shaderset=moose-lcd")),
                )],
            },
            Choice {
                name: "none".into(),
                steps: vec![block(
                    paths,
                    &format!("shader-{slug}"),
                    Some(&format!("{slug}.shaderset=none")),
                )],
            },
        ],
    }
}

/// The arcade systems on this card. `mame` has no ROMs and `neogeocd` is not a
/// folder here, so writing lines for either would be noise in a file configgen
/// re-reads at every launch of every system.
const ARCADE: [&str; 2] = ["fbneo", "neogeo"];

/// What the Flip's own controller config calls each button.
///
/// Read off `retroarchcustom.cfg` rather than reasoned about. The letters here
/// are the letters printed on the plastic — see the warning at the top of
/// docs/handover.md, which cost two rounds of getting this exactly backwards.
/// RetroPad B is the primary fire in every arcade core, and on this device
/// that is the button printed **B**, not the one printed A.
const BTN_A: u8 = 0;
const BTN_Y: u8 = 3;
const BTN_L1: u8 = 4;
const BTN_R1: u8 = 5;

/// One `<system>.retroarch.<key>=<value>` line per arcade system.
///
/// configgen copies anything under a `retroarch.` prefix straight into
/// retroarchcustom.cfg, and it does it at the *end* of writeLibretroConfig —
/// after the controller bindings — so these win over what it just wrote. That
/// is the only reason a rebind is possible from here at all.
fn arcade_lines(pairs: &[(&str, String)]) -> String {
    let mut out = String::new();
    for system in ARCADE {
        for (key, value) in pairs {
            out.push_str(&format!("{system}.retroarch.{key}={value}\n"));
        }
    }
    out
}

/// The block for one modifier button.
///
/// `remap` is the pair of lines that move a face button's normal job out of the
/// way, and it is empty for the shoulders because nothing is in the way there.
fn rapid_fire_body(btn: u8, remap: &[(&str, String)]) -> String {
    let mut pairs = vec![
        // Mode 3 is "single button (hold)": while the modifier is down,
        // RetroPad B repeats. Not mode 0 — RetroArch calls that one "classic"
        // and it *latches*, leaving the button flagged after everything is
        // released. A latch nobody remembers is dangerous in exactly the games
        // this exists for.
        ("input_turbo_mode", "3".to_string()),
        ("input_turbo_default_button", "0".to_string()),
        ("input_player1_turbo_btn", btn.to_string()),
    ];
    pairs.extend(remap.iter().map(|(k, v)| (*k, v.clone())));
    arcade_lines(&pairs)
}

/// Hold a button, the shot repeats.
///
/// The rate lives in its own patch and its own block. That is not tidiness:
/// `set_block` removes a block and appends the new one at the end of the file,
/// so re-applying this patch would move it *below* the rate block — and
/// configgen reads knulli.conf last-wins. Two blocks that set the same key can
/// therefore swap which one wins simply because you changed the other one.
/// Disjoint key sets are what make the two independent.
fn rapid_fire(paths: &Paths) -> Patch {
    // Holding the modifier on its own is not a style note. RetroArch reports
    // the repeat only on frames where the button is not physically pressed —
    // the real press wins — so a modifier that also sends the fire button gives
    // one continuous shot and no repeat at all.
    let shoulder = |btn: u8| vec![block(paths, "rapid-fire", Some(&rapid_fire_body(btn, &[])))];

    Patch {
        id: "rapid-fire",
        title: "Rapid fire",
        detail: "fbneo and neogeo input keys in knulli.conf, passed through to \
                 retroarchcustom.cfg by configgen. Hold the button and the shot repeats; let \
                 go and it stops. Hold it on its own — RetroArch drops the repeat on any \
                 frame the button is physically pressed, so a modifier that also fires gives \
                 one long shot and no repeat. The rate is the next patch along.",
        choices: vec![
            Choice { name: "off".into(), steps: vec![block(paths, "rapid-fire", None)] },
            Choice { name: "hold L1".into(), steps: shoulder(BTN_L1) },
            Choice { name: "hold R1".into(), steps: shoulder(BTN_R1) },
            // Y is not free the way the shoulders are: it is RetroPad Y, which
            // these cores map to Neo Geo C, so holding it sends C underneath
            // the repeat. Harmless in a game that does not use C.
            Choice { name: "hold Y".into(), steps: shoulder(BTN_Y) },
            // A is not free either, and unlike Y it cannot be left alone: it is
            // RetroPad A, which these cores map to Neo Geo B — jump. Holding
            // the modifier would hold jump for as long as you were firing.
            //
            // So A's normal job moves to Y: RetroPad A is rebound to the button
            // printed Y, and RetroPad Y is cleared off it so one press does not
            // send both. The cost is Neo Geo C, which nothing reaches while
            // this option is on.
            Choice {
                name: "hold A".into(),
                steps: vec![block(
                    paths,
                    "rapid-fire",
                    Some(&rapid_fire_body(
                        BTN_A,
                        &[
                            ("input_player1_a_btn", BTN_Y.to_string()),
                            ("input_player1_y_btn", "nul".to_string()),
                        ],
                    )),
                )],
            },
        ],
    }
}

/// How fast it repeats, one shot a second at a time.
///
/// Six is the setting to use. It cannot be option zero — the catalogue's rule
/// is that option zero touches nothing, so that a bare device reads as
/// untouched rather than as misconfigured — so "off" here means no period line
/// at all and RetroArch's own default, which is a six-frame cycle, about ten a
/// second, and faster than most of these games read input.
fn rapid_fire_rate(paths: &Paths) -> Patch {
    let mut choices =
        vec![Choice { name: "off".into(), steps: vec![block(paths, "rapid-fire-rate", None)] }];
    for hz in 1..=12u32 {
        // The same arithmetic the desktop uses, from the same function, so the
        // two cannot drift into feeling different at the same number.
        let (period, duty) = moose_rack::tweaks::turbo_timing(hz);
        // The rate you asked for, written next to the frames it became.
        //
        // Not decoration. A period is whole frames, so the top of this range is
        // quantised: 11 and 12 a second are both a 5-frame cycle, and without
        // this line the two options would be byte-identical blocks — `state`
        // would match the first one and the menu would read 11 after you chose
        // 12. configgen ignores comments, so it costs nothing.
        let body = format!(
            "# {hz} a second — a {period}-frame cycle, button down for {duty} of them\n{}",
            arcade_lines(&[
                ("input_turbo_period", period.to_string()),
                ("input_duty_cycle", duty.to_string()),
            ])
        );
        choices.push(Choice {
            name: format!("{hz} a second"),
            steps: vec![block(paths, "rapid-fire-rate", Some(&body))],
        });
    }
    Patch {
        id: "rapid-fire-rate",
        title: "Rapid fire rate",
        detail: "input_turbo_period and input_duty_cycle, in frames. Six a second is the one \
                 to use. The button is held for at most four frames whatever the rate: half \
                 of a slow cycle is a quarter-second press, and a quarter of a second is a \
                 held button to any game that charges a shot — Pulstar and Blazing Star spent \
                 the slow settings winding up instead of firing. A cycle is whole frames, so \
                 the fast end is coarse — 11 and 12 a second are the same five frames — and \
                 the block says which rate was asked for.",
        choices,
    }
}

/// The patches tab, in the order it is drawn.
pub fn all(paths: &Paths) -> Vec<Patch> {
    vec![
        Patch {
            id: "hotkeys",
            title: "Hotkeys",
            detail: "Writes the RetroArch hotkey block into knulli.conf. It has to go there \
                     rather than into RetroArch's own menu, because configgen rewrites \
                     retroarch.cfg at every single launch — which is why changes made inside \
                     RetroArch never stick.",
            choices: on_off(
                "ON",
                vec![block(paths, "hotkeys", Some(HOTKEYS))],
                vec![block(paths, "hotkeys", None)],
            ),
        },
        Patch {
            id: "shaders",
            title: "Shaders",
            detail: "global.shaderset in knulli.conf, pointed at our own set in \
                     /userdata/shaders. That is not vanity: configgen resolves a set's \
                     presets relative to that directory, and RetroArch cycles the folder of \
                     the preset it loaded — so a stock set makes Hotkey + D-pad walk all \
                     seven hundred presets in the library, most of which this handheld \
                     cannot afford. Ours holds four, all cheap.",
            choices: vec![
                Choice {
                    name: "off".into(),
                    steps: {
                        let mut s = vec![block(paths, "shaders", None)];
                        s.extend(shader_files(paths, None));
                        s
                    },
                },
                Choice {
                    name: "shimmerless + LCD/CRT".into(),
                    steps: {
                        let mut s = vec![block(paths, "shaders", Some(SHADERS_LCD))];
                        s.extend(shader_files(paths, Some(("moose-lcd", SET_LCD))));
                        s
                    },
                },
                Choice {
                    name: "shimmerless plain".into(),
                    steps: {
                        let mut s = vec![block(paths, "shaders", Some(SHADERS_PLAIN))];
                        s.extend(shader_files(paths, Some(("moose-plain", SET_PLAIN))));
                        s
                    },
                },
                Choice {
                    name: "zfast".into(),
                    steps: {
                        let mut s = vec![block(paths, "shaders", Some(SHADERS_ZFAST))];
                        s.extend(shader_files(paths, Some(("moose-zfast", SET_ZFAST))));
                        s
                    },
                },
            ],
        },
        // Bezels, one system at a time.
        //
        // Only these three have 4:3 artwork, and that is not an oversight in
        // KNULLI: a 4:3 console game already fills a 4:3 screen, so there is
        // nowhere for a border to go that is not the picture. The handhelds
        // are narrower than the screen and have real space going spare.
        bezel(paths, "gba", "Game Boy Advance", true),
        bezel(paths, "gb", "Game Boy", false),
        bezel(paths, "gbc", "Game Boy Color", false),

        // And the shader, per system, over the global one.
        //
        // Not `<system>.smooth` — knulli.conf says of it, in its own words,
        // "Is overidden if using a shader set", so a bilinear-filter row would
        // do nothing at all while any shader set is on.
        shader(paths, "gba", "Game Boy Advance"),
        shader(paths, "gb", "Game Boy"),
        shader(paths, "gbc", "Game Boy Color"),

        rapid_fire(paths),
        rapid_fire_rate(paths),

        Patch {
            id: "hotkey-app",
            title: "L2+R2 opens this app",
            detail: "Two lines in /userdata/system/configs/multimedia_keys.conf, which \
                     S50triggerhappy prefers over anything in /etc — and /etc is on the tmpfs \
                     overlay, so a rule there would be gone by the next boot. The file is \
                     seeded from KNULLI's own first, because the /userdata one replaces it \
                     rather than adding to it, and the volume and power keys live in there.",
            choices: on_off(
                "ON",
                vec![Step::Block {
                    file: paths.trigger_conf(),
                    id: "hotkey".into(),
                    body: Some(TRIGGERS.to_string()),
                    seed: Some(paths.stock_triggers()),
                }],
                vec![Step::Block {
                    file: paths.trigger_conf(),
                    id: "hotkey".into(),
                    body: None,
                    seed: Some(paths.stock_triggers()),
                }],
            ),
        },
        Patch {
            id: "es-shoulders",
            title: "ES ignores L2/R2",
            detail: "A 24-line es_input.cfg holding only this device's pad, with l2 and r2 \
                     dropped, so the shoulder combo does not also page the menu about. \
                     Copying KNULLI's file across whole does not work: 291 pad definitions, \
                     and EmulationStation will not start.",
            choices: on_off(
                "ON",
                vec![place(paths, paths.es_input(), Some(ES_INPUT))],
                vec![place(paths, paths.es_input(), None)],
            ),
        },
        Patch {
            id: "es-logo",
            title: "Hide the loading logo",
            detail: "A black 1280x720 PNG over resources/logo.png, put back at every boot by \
                     /boot/boot-custom.sh because /usr is a tmpfs and is stock again each \
                     time. The blank itself lives on /boot too: that hook runs as S00 and \
                     S02resize is what mounts /userdata, so a blank kept there is one the \
                     hook cannot see. EmulationStation draws that file whenever it is loading — every \
                     game launch and every return from one — and there is no setting for it.",
            choices: on_off(
                "ON",
                vec![
                    place(paths, paths.blank_logo(), Some(BLANK_LOGO)),
                    place(paths, paths.boot_custom(), Some(BOOT_HOOK)),
                    place(paths, paths.es_logo(), Some(BLANK_LOGO)),
                ],
                vec![
                    place(paths, paths.blank_logo(), None),
                    place(paths, paths.es_logo(), None),
                ],
            ),
        },
        Patch {
            id: "launch-evmapy",
            title: "Don't restart evmapy on every launch",
            detail: "0.93 s off every game launch, measured — 3.43 s to 2.50 s, three runs \
                     each side. batocera-evmapy start kills the daemon, touches a flag and \
                     blocks on inotifywait until it comes back; it is a process round trip, \
                     not work. configgen writes a per-device .json into /var/run/evmapy \
                     before calling start, and libretro.keys asks only for a lightgun combo, \
                     so a libretro launch with no gun writes nothing and then waits for a \
                     daemon with no job. The guard is that test and nothing more, so the 54 \
                     standalone emulators that do declare player mappings are untouched. \
                     /usr is a tmpfs, so /boot/boot-custom.sh puts the line back each boot.",
            choices: on_off(
                "ON",
                vec![
                    place(paths, paths.evmapy_flag(), Some(EVMAPY_FLAG)),
                    place(paths, paths.boot_custom(), Some(BOOT_HOOK)),
                ],
                vec![place(paths, paths.evmapy_flag(), None)],
            ),
        },
        Patch {
            id: "boot-splash",
            title: "Clear the boot splash",
            detail: "Zeroes /dev/fb0 from /userdata/system/custom.sh, which S99userservices \
                     runs after S03system-splash has painted the KNULLI logo there and left \
                     it. Only visible when nothing else is drawing — but then it is the whole \
                     screen.",
            choices: on_off(
                "ON",
                vec![startup(paths, "splash", Some(CLEAR_FB))],
                vec![startup(paths, "splash", None)],
            ),
        },
        Patch {
            id: "never-sleep",
            title: "Never sleep",
            detail: "system.batterysaver.extendedmode in knulli.conf. Never suspends, on \
                     battery or plugged in — suspending after 15 minutes idle drops the network \
                     and reads as a dead device. Dimming stays either way. If you only want \
                     this while it is charging, use \"Awake while charging\" instead and leave \
                     this off.",
            choices: on_off(
                "ON",
                vec![block(paths, "power", Some(POWER))],
                vec![block(paths, "power", None)],
            ),
        },
        Patch {
            id: "charge-awake",
            title: "Awake while charging",
            detail: "system.batterysaver.chargingbypass in knulli.conf. Plugged in, it stops \
                     dimming and stops suspending — KNULLI drops a pause file whenever the \
                     battery is not discharging, and both idle hooks check for it. On battery \
                     nothing changes, so it still looks after itself in your bag. Closing the \
                     lid and pressing power both still suspend: neither goes anywhere near \
                     that file. No script and no service — the OS already does this, it is \
                     just turned off.",
            choices: on_off(
                "ON",
                vec![block(paths, "charge-awake", Some(CHARGE_AWAKE))],
                vec![block(paths, "charge-awake", None)],
            ),
        },
        Patch {
            id: "wifi-awake",
            title: "Keep Wi-Fi awake",
            detail: "Turns off wireless power saving from /userdata/system/custom.sh. Costs \
                     battery; buys a device that answers when you reach for it.",
            choices: on_off(
                "ON",
                vec![startup(paths, "wifi", Some(WIFI_AWAKE))],
                vec![startup(paths, "wifi", None)],
            ),
        },
        Patch {
            id: "gpu",
            title: "Graphics driver",
            detail: "Writes which Mali blob /boot/boot-custom.sh installs at boot. Marker and \
                     blobs both live on /boot because that hook runs as S00 and /userdata is \
                     not mounted until S02 — which is why this switch never once worked. The \
                     blobs are 43 and 56 MB, too big to carry in here, so they are placed on \
                     /boot once; without them this setting is remembered and does nothing. \
                     The stock one has no Wayland support, the g24p0 one does, and the \
                     emulators behave identically on both.",
            choices: vec![
                Choice {
                    // No marker at all: the hook does nothing without one and
                    // /usr is the stock image at every boot, so this is what
                    // an untouched device looks like.
                    name: "stock".into(),
                    steps: vec![place(paths, paths.gpu_choice(), None)],
                },
                Choice {
                    name: "wayland".into(),
                    steps: vec![
                        place(paths, paths.gpu_choice(), Some(b"wayland\n")),
                        place(paths, paths.boot_custom(), Some(BOOT_HOOK)),
                    ],
                },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::State;

    fn scratch(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("moose-catalogue-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Paths::new(dir)
    }

    #[test]
    fn a_fresh_device_reads_as_the_first_option_everywhere() {
        // The convention every other part of this leans on: **option 0 is the
        // one that touches nothing**. "off" for most, "follow global" for the
        // per-system shaders, "stock" for the driver. A bare device must read
        // as that, or a new install looks like a corrupted one and `--restore`
        // has nothing sane to fall back to.
        let paths = scratch("fresh");
        for patch in all(&paths) {
            assert_eq!(
                patch.state(),
                State::At(0),
                "{} should read as '{}' on a bare device",
                patch.id,
                patch.choices[0].name
            );
        }
    }

    #[test]
    fn every_patch_applies_and_reads_itself_back() {
        // The round trip that matters: after apply, `state` must agree. If it
        // does not, the menu shows a change still queued that has already
        // happened, and pressing A again would run it forever.
        let paths = scratch("roundtrip");
        for patch in all(&paths) {
            for (i, choice) in patch.choices.iter().enumerate() {
                patch.apply(i).unwrap_or_else(|e| {
                    panic!("{} option {} ({}) failed: {e:#}", patch.id, i, choice.name)
                });
                assert_eq!(
                    patch.state(),
                    State::At(i),
                    "{} did not read back as {}",
                    patch.id,
                    choice.name
                );
            }
        }
    }

    #[test]
    fn turning_everything_on_then_back_leaves_no_trace() {
        // A full revert has to be a full revert. Anything left behind is
        // something the next person has to find by hand — and knulli.conf is
        // read last-wins, so a stray block is a setting still in force.
        let paths = scratch("revert");
        let patches = all(&paths);
        for patch in &patches {
            patch.apply(patch.choices.len() - 1).unwrap();
        }
        for patch in &patches {
            patch.apply(0).unwrap();
            assert_eq!(patch.state(), State::At(0), "{} would not go back", patch.id);
        }
        let conf = std::fs::read_to_string(paths.knulli_conf()).unwrap_or_default();
        assert!(
            !conf.contains("## moose-patch:"),
            "knulli.conf still holds our blocks:\n{conf}"
        );
        let startup = std::fs::read_to_string(paths.user_startup()).unwrap_or_default();
        assert!(
            !startup.contains("## moose-patch:"),
            "custom.sh still holds our blocks:\n{startup}"
        );
    }

    /// The lines one option of `rapid-fire` puts in knulli.conf.
    fn rapid_fire_conf(paths: &Paths, option: &str) -> String {
        let patches = all(paths);
        let patch = patches.iter().find(|p| p.id == "rapid-fire").expect("no rapid-fire patch");
        let i = patch
            .choices
            .iter()
            .position(|c| c.name == option)
            .unwrap_or_else(|| panic!("rapid-fire has no option {option:?}"));
        patch.apply(i).unwrap();
        std::fs::read_to_string(paths.knulli_conf()).unwrap_or_default()
    }

    /// Holding A would otherwise hold jump.
    ///
    /// The modifier has to send nothing of its own — RetroArch drops the repeat
    /// on any frame the button is physically down, and more to the point these
    /// cores map RetroPad A to Neo Geo B. Holding it to fire would hold jump
    /// for the whole burst. So A's normal job moves to the button printed Y.
    #[test]
    fn hold_a_moves_a_out_of_the_way_and_lands_it_on_y() {
        let paths = scratch("rapid-fire-a");
        let conf = rapid_fire_conf(&paths, "hold A");

        for system in ARCADE {
            // A is the modifier.
            assert!(
                conf.contains(&format!("{system}.retroarch.input_player1_turbo_btn={BTN_A}")),
                "{system}: A is not the modifier:\n{conf}"
            );
            // And what A used to do now happens on Y.
            assert!(
                conf.contains(&format!("{system}.retroarch.input_player1_a_btn={BTN_Y}")),
                "{system}: normal-paced A did not land on Y:\n{conf}"
            );
            // Y stops sending its own button, or one press would send both.
            assert!(
                conf.contains(&format!("{system}.retroarch.input_player1_y_btn=nul")),
                "{system}: Y still sends Y as well:\n{conf}"
            );
        }
    }

    /// The remap belongs to `hold A` and to nothing else.
    ///
    /// A rebind left behind by a different option is a controller that is
    /// silently wrong in every arcade game, which is a far worse bug than rapid
    /// fire not working — you would not know to look here.
    #[test]
    fn only_hold_a_rebinds_anything() {
        for option in ["off", "hold L1", "hold R1", "hold Y"] {
            let paths = scratch(&format!("rapid-fire-{}", option.replace(' ', "-")));
            let conf = rapid_fire_conf(&paths, option);
            assert!(
                !conf.contains("input_player1_a_btn"),
                "{option} rebound A:\n{conf}"
            );
            assert!(
                !conf.contains("input_player1_y_btn"),
                "{option} rebound Y:\n{conf}"
            );
        }
    }

    /// Turning it off puts the face buttons back.
    ///
    /// `hold A` is the only option that touches a binding, so it is the only
    /// one whose revert has anything to undo. Leaving the rebind behind would
    /// mean A does nothing and Y does A's job, forever, with rapid fire showing
    /// as off.
    #[test]
    fn switching_off_after_hold_a_leaves_no_rebind() {
        let paths = scratch("rapid-fire-a-off");
        rapid_fire_conf(&paths, "hold A");
        let conf = rapid_fire_conf(&paths, "off");
        assert!(!conf.contains("input_player1_a_btn"), "A is still rebound:\n{conf}");
        assert!(!conf.contains("input_player1_y_btn"), "Y is still cleared:\n{conf}");
        assert!(!conf.contains("input_turbo_mode"), "turbo is still on:\n{conf}");
    }

    /// The modifier is a different RetroPad button from the one that repeats.
    ///
    /// RetroPad B — `input_turbo_default_button=0` — is what pulses, and on
    /// this device that is the button printed B. Every option here has to hold
    /// something else, or the physical press wins on every frame and there is
    /// no repeat at all.
    #[test]
    fn no_option_holds_the_button_that_repeats() {
        let paths = scratch("rapid-fire-modifier");
        let patches = all(&paths);
        let patch = patches.iter().find(|p| p.id == "rapid-fire").unwrap();
        for choice in patch.choices.iter().skip(1) {
            let i = patch.choices.iter().position(|c| c.name == choice.name).unwrap();
            patch.apply(i).unwrap();
            let conf = std::fs::read_to_string(paths.knulli_conf()).unwrap();
            assert!(
                !conf.contains("input_player1_turbo_btn=1"),
                "{} holds the button that repeats:\n{conf}",
                choice.name
            );
            assert!(
                conf.contains("input_turbo_default_button=0"),
                "{} does not pulse RetroPad B:\n{conf}",
                choice.name
            );
        }
    }

    /// Twelve rates, each a different block.
    ///
    /// A period is whole frames, so 11 and 12 a second both come out as five —
    /// and two options that write byte-identical blocks are two options the
    /// menu cannot tell apart. It read back as 11 after choosing 12 until the
    /// requested rate went into the block as a comment.
    #[test]
    fn every_rate_is_its_own_setting() {
        let paths = scratch("rapid-fire-rates");
        let patches = all(&paths);
        let patch = patches.iter().find(|p| p.id == "rapid-fire-rate").unwrap();

        assert_eq!(patch.choices.len(), 13, "off, then one a second up to twelve");
        for (i, hz) in (1..=12u32).enumerate() {
            assert_eq!(patch.choices[i + 1].name, format!("{hz} a second"));
        }

        let mut seen: Vec<String> = Vec::new();
        for (i, choice) in patch.choices.iter().enumerate().skip(1) {
            patch.apply(i).unwrap();
            assert_eq!(patch.state(), State::At(i), "{} did not read back", choice.name);
            let conf = std::fs::read_to_string(paths.knulli_conf()).unwrap();
            assert!(!seen.contains(&conf), "{} wrote a block already used", choice.name);
            seen.push(conf);
        }
    }

    /// The handheld and the desktop agree on what a rate means.
    ///
    /// Both write `input_turbo_period` and `input_duty_cycle`, from the same
    /// function, so "six a second" cannot come to mean two different speeds.
    #[test]
    fn the_rate_is_the_desktops_arithmetic() {
        let paths = scratch("rapid-fire-timing");
        let patches = all(&paths);
        let patch = patches.iter().find(|p| p.id == "rapid-fire-rate").unwrap();
        for hz in 1..=12u32 {
            let i = patch.choices.iter().position(|c| c.name == format!("{hz} a second")).unwrap();
            patch.apply(i).unwrap();
            let conf = std::fs::read_to_string(paths.knulli_conf()).unwrap();
            let (period, duty) = moose_rack::tweaks::turbo_timing(hz);
            assert!(
                conf.contains(&format!("fbneo.retroarch.input_turbo_period={period}")),
                "{hz} a second is not {period} frames:\n{conf}"
            );
            assert!(
                conf.contains(&format!("fbneo.retroarch.input_duty_cycle={duty}")),
                "{hz} a second does not hold for {duty} frames:\n{conf}"
            );
            assert!(duty <= 4, "{hz} a second holds the button for {duty} frames");
            assert!(period > duty, "{hz} a second never lets the button go");
        }
    }

    /// Both arcade systems, every time.
    ///
    /// neogeo is a separate folder with a separate core (geolith), so a patch
    /// that only wrote fbneo lines would work in Metal Slug from the arcade
    /// folder and do nothing at all from the Neo Geo one.
    #[test]
    fn both_arcade_systems_get_every_line() {
        let paths = scratch("rapid-fire-systems");
        let conf = rapid_fire_conf(&paths, "hold L1");
        for system in ARCADE {
            assert!(
                conf.contains(&format!("{system}.retroarch.input_turbo_mode=3")),
                "{system} was left out:\n{conf}"
            );
        }
    }

    #[test]
    fn no_two_patches_write_the_same_block() {
        // Sharing a marker would mean one patch silently overwriting the
        // other's settings, and both reporting the same state. Within a single
        // patch the repeats are the point — that is how "off" undoes "on" —
        // so the check is per (file, block) across *different* patches.
        let paths = scratch("unique");
        let mut ids = std::collections::BTreeSet::new();
        let mut owner: std::collections::BTreeMap<(std::path::PathBuf, String), &str> =
            Default::default();

        for patch in all(&paths) {
            assert!(ids.insert(patch.id.to_string()), "duplicate patch id {}", patch.id);
            for choice in &patch.choices {
                for step in &choice.steps {
                    if let crate::patch::Step::Block { file, id, .. } = step {
                        let key = (file.clone(), id.clone());
                        match owner.get(&key) {
                            Some(first) if *first != patch.id => panic!(
                                "{} and {first} both write block '{id}' in {}",
                                patch.id,
                                file.display()
                            ),
                            _ => {
                                owner.insert(key, patch.id);
                            }
                        }
                    }
                }
            }
        }
        assert!(owner.len() >= 12, "expected a block per config patch, saw {}", owner.len());
    }

    #[test]
    fn every_shader_option_keeps_the_cycle_inside_our_folder() {
        // RetroArch cycles the directory of the preset it loaded. Name a stock
        // set here and Hotkey + D-pad walks the whole library — seven hundred
        // presets, most of them far too heavy for this handheld. That is a
        // regression you cannot see from the code, only from the device, so
        // it gets a test.
        let paths = scratch("shader-cycle");
        for patch in all(&paths) {
            if !(patch.id == "shaders" || patch.id.starts_with("shader-")) {
                continue;
            }
            for choice in &patch.choices {
                for step in &choice.steps {
                    let crate::patch::Step::Block { body: Some(body), .. } = step else {
                        continue;
                    };
                    for line in body.lines() {
                        let Some((_, set)) = line.trim().split_once("shaderset=") else {
                            continue;
                        };
                        assert!(
                            set == "none" || set.starts_with("moose-"),
                            "{} / {} points at '{set}', which lives in the stock library",
                            patch.id,
                            choice.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn choosing_a_shader_lays_down_the_whole_cycle() {
        // All four presets, whichever option you pick, so the list you cycle
        // is the same list every time. And "off" takes them all away again.
        let paths = scratch("shader-files");
        let shaders = all(&paths).into_iter().find(|p| p.id == "shaders").unwrap();

        shaders.apply(1).unwrap();
        let present = |n: &str| paths.shader(n).exists();
        assert!(present("1-sharp-shimmerless.glslp"));
        assert!(present("4-zfast-crt.glslp"));
        assert!(paths.shaderset("moose-lcd").exists(), "the set itself must be written");

        shaders.apply(0).unwrap();
        assert!(!present("1-sharp-shimmerless.glslp"), "off leaves nothing behind");
        assert!(!paths.shaderset("moose-lcd").exists());
    }

    #[test]
    fn only_the_systems_with_artwork_get_a_bezel_row() {
        // gba, gb and gbc, and no others. A 4:3 console game already fills a
        // 4:3 screen, so a border there costs picture and buys nothing —
        // which is also why KNULLI ships no 4:3 artwork for them.
        let paths = scratch("bezel-rows");
        let rows: Vec<&str> = all(&paths)
            .iter()
            .map(|p| p.id)
            .filter(|id| id.starts_with("bezel-"))
            .map(|id| Box::leak(id.to_string().into_boxed_str()) as &str)
            .collect();
        assert_eq!(rows, vec!["bezel-gba", "bezel-gb", "bezel-gbc"]);
    }

    #[test]
    fn the_trigger_file_keeps_knullis_own_keys() {
        // The trap: /userdata/system/configs/multimedia_keys.conf *replaces*
        // the one in /etc. Creating it with only our block would take volume,
        // power and the lid switch away.
        let paths = scratch("triggers");
        let stock = paths.stock_triggers();
        std::fs::create_dir_all(stock.parent().unwrap()).unwrap();
        std::fs::write(&stock, "KEY_VOLUMEUP 1  /usr/bin/volume-button volup\n").unwrap();

        let patch = all(&paths).into_iter().find(|p| p.id == "hotkey-app").unwrap();
        patch.apply(1).unwrap();

        let written = std::fs::read_to_string(paths.trigger_conf()).unwrap();
        assert!(written.contains("KEY_VOLUMEUP"), "volume keys were lost:\n{written}");
        assert!(written.contains("BTN_TR2"));
    }

    #[test]
    fn hiding_the_logo_gives_es_its_own_file_back() {
        // If "off" deleted logo.png instead of restoring it, ES would be
        // missing a resource it loads unconditionally.
        let paths = scratch("logo");
        let logo = paths.es_logo();
        std::fs::create_dir_all(logo.parent().unwrap()).unwrap();
        std::fs::write(&logo, b"the real KNULLI beetle").unwrap();

        let patch = all(&paths).into_iter().find(|p| p.id == "es-logo").unwrap();
        patch.apply(1).unwrap();
        assert_eq!(std::fs::read(&logo).unwrap(), BLANK_LOGO);
        patch.apply(0).unwrap();
        assert_eq!(std::fs::read(&logo).unwrap(), b"the real KNULLI beetle");
    }

    #[test]
    fn two_patches_sharing_one_file_do_not_tread_on_each_other() {
        // never-sleep and hotkeys both live in knulli.conf; boot-splash and
        // wifi-awake both live in custom.sh. Applying one must not disturb
        // the other's block.
        let paths = scratch("shared");
        let patches = all(&paths);
        let get = |id: &str| patches.iter().find(|p| p.id == id).unwrap();

        get("hotkeys").apply(1).unwrap();
        get("never-sleep").apply(1).unwrap();
        get("charge-awake").apply(1).unwrap();
        assert_eq!(get("never-sleep").state(), State::At(1), "charging clobbered sleep");
        assert_eq!(get("charge-awake").state(), State::At(1));
        assert_eq!(get("hotkeys").state(), State::At(1), "sleep clobbered hotkeys");

        get("boot-splash").apply(1).unwrap();
        get("wifi-awake").apply(1).unwrap();
        assert_eq!(get("boot-splash").state(), State::At(1), "wifi clobbered splash");

        // And taking one away leaves the other.
        get("never-sleep").apply(0).unwrap();
        assert_eq!(get("hotkeys").state(), State::At(1));
    }

    /// What the settings reader would act on, in file order.
    fn live(text: &str) -> Vec<&str> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }

    #[test]
    fn the_power_patches_beat_the_values_knulli_ships_with() {
        // The whole failure was here, not in the block: a stock knulli.conf
        // already sets both of these near the top, and the reader takes the
        // first one it meets. A patch appended underneath changed nothing
        // while reporting itself as on.
        let paths = scratch("power-stock");
        // A stock file has to exist for there to be anything to shadow.
        std::fs::create_dir_all(paths.knulli_conf().parent().unwrap()).unwrap();
        std::fs::write(
            paths.knulli_conf(),
            "system.power.led=1\n\
             system.batterysaver.mode=dim\n\
             system.batterysaver.extendedmode=suspend\n\
             system.batterysaver.chargingbypass=0\n",
        )
        .unwrap();
        let patches = all(&paths);
        let get = |id: &str| patches.iter().find(|p| p.id == id).unwrap();

        get("charge-awake").apply(1).unwrap();
        let text = std::fs::read_to_string(paths.knulli_conf()).unwrap();
        let set: Vec<&str> =
            live(&text).into_iter().filter(|l| l.contains("chargingbypass")).collect();
        assert_eq!(
            set,
            vec!["system.batterysaver.chargingbypass=1"],
            "the shipped =0 is still the one the reader would find first"
        );
        // Suspending on battery is untouched — that is the point of this one.
        assert!(live(&text).contains(&"system.batterysaver.extendedmode=suspend"));

        // And off puts KNULLI's own value back, rather than deleting the line.
        get("charge-awake").apply(0).unwrap();
        let text = std::fs::read_to_string(paths.knulli_conf()).unwrap();
        assert!(live(&text).contains(&"system.batterysaver.chargingbypass=0"));
    }
}


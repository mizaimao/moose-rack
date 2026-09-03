// Settings, as data rather than as a screen.
//
// The desktop has 38 of these across seven panes, and a survey of them found
// that all but a handful are one of four shapes: on or off, one of a fixed set,
// a number in a range, or a line of text. So the screen is one list widget and
// this table, and adding a setting is a row here rather than markup, a wiring
// function and a backend allowlist entry.
//
// What is deliberately *not* here: the four the desktop has that mean nothing on
// a handheld — the window arrangement, the title bar, fit-to-window, and where
// RetroArch is installed, which on KNULLI is `/usr` and decided by the platform
// scheme. A setting that cannot do anything is worse than a missing one,
// because you try it.
//
// Writing goes through `moose_rack::config`, which is the same three functions
// the desktop app's Tauri commands call. There is no second writer and no second
// idea of what a field is called.

use anyhow::Context;
use moose_rack::config::{self, Config};

/// The file this front end reads and writes.
///
/// Not `config.toml`. The desktop app owns that name, and on a development Mac
/// both run out of the same directory — so a value stepped in the preview was
/// editing the real desktop settings, silently, one press at a time. On the
/// device there is only ever one app and the name is simply explicit about
/// which one it belongs to.
///
/// Same format and same `Config` type: this is a second *file*, not a second
/// idea of what a setting is.
pub const FILE: &str = "moose-sdl.toml";

/// Read the front end's config, falling back to the desktop's if it has none
/// yet.
///
/// The fallback is read-only and happens once: a first run on a machine that
/// already has a server and an account configured should not start from
/// nothing. Everything written afterwards goes to [`FILE`], so the two never
/// drift into each other.
pub fn load() -> (Config, &'static str) {
    if Config::exists(FILE) {
        return (
            Config::load_from(std::path::Path::new(FILE)).unwrap_or_default(),
            FILE,
        );
    }
    if Config::exists("config.toml") {
        return (
            Config::load_from(std::path::Path::new("config.toml")).unwrap_or_default(),
            "config.toml",
        );
    }
    (Config::default(), FILE)
}

/// What kind of control a setting is.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// On or off.
    Toggle(bool),
    /// One of a fixed set. `at` indexes `options`, which is `(value, label)`.
    ///
    /// Owned rather than borrowed because the emulator rows are built from the
    /// core map at run time — which cores a console has is not something that
    /// can be written down in the source.
    Choice {
        at: usize,
        options: Vec<(String, String)>,
    },
    /// A number in a range, stepped with left and right.
    Number {
        value: i64,
        min: i64,
        max: i64,
        step: i64,
        unit: &'static str,
    },
    /// A line of text, filled in with the on-screen keyboard.
    Text { value: String, secret: bool },
    /// Shown, not changed. The help says where to change it.
    ReadOnly(String),
    /// Does something rather than holding a value.
    Action,
    /// A pad binding, changed by pressing the button you want.
    Binding(Option<u8>),
}

/// One row on a settings screen.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The `[table] key` this writes to, as `table.key`. Empty for actions and
    /// for anything the config does not own.
    pub field: &'static str,
    pub label: &'static str,
    /// One sentence, shown beside the list. Says what it does, or where to
    /// change it when it cannot be changed here.
    pub help: &'static str,
    pub kind: Kind,
}

impl Entry {
    /// What the right-hand column shows.
    pub fn value(&self) -> String {
        match &self.kind {
            Kind::Toggle(on) => if *on { "On" } else { "Off" }.to_owned(),
            Kind::Choice { at, options } => options
                .get(*at)
                .map(|(_, label)| (*label).to_owned())
                .unwrap_or_default(),
            Kind::Number { value, unit, .. } => format!("{value}{unit}"),
            Kind::Text { value, secret } => {
                if value.is_empty() {
                    "not set".to_owned()
                } else if *secret {
                    "\u{2022}".repeat(value.chars().count().min(12))
                } else {
                    value.clone()
                }
            }
            Kind::ReadOnly(v) => v.clone(),
            Kind::Action => "\u{203a}".to_owned(),
            Kind::Binding(b) => moose_rack::binds::pad_label(*b),
        }
    }

    /// Whether left and right change this in place.
    ///
    /// Sliders only. A toggle flips on A and a choice opens a list — including
    /// them here is what put arrows beside rows those presses did not move, and
    /// taught the wrong control on every screen.
    pub fn steps(&self) -> bool {
        matches!(self.kind, Kind::Number { .. })
    }

    /// Step the value. `dir` is -1 or 1. Returns what to write, if anything.
    ///
    /// Toggles ignore the direction — there are two states and both directions
    /// reach the other one, which is what every settings list does and what
    /// stops "left" being a dead press on half the rows.
    pub fn step(&mut self, dir: i64) -> Option<Written> {
        match &mut self.kind {
            Kind::Toggle(on) => {
                *on = !*on;
                Some(Written::Bool(*on))
            }
            Kind::Choice { at, options } => {
                let n = options.len();
                if n == 0 {
                    return None;
                }
                *at = (*at as i64 + dir).rem_euclid(n as i64) as usize;
                Some(Written::Text(options[*at].0.clone()))
            }
            Kind::Number {
                value,
                min,
                max,
                step,
                ..
            } => {
                *value = (*value + dir * *step).clamp(*min, *max);
                Some(Written::Number(*value))
            }
            _ => None,
        }
    }
}

/// A value on its way to `config.toml`.
#[derive(Debug, Clone, PartialEq)]
pub enum Written {
    Bool(bool),
    Number(i64),
    Text(String),
}

/// The appearance settings as they stand right now, for the renderer.
///
/// Read back off the entries rather than out of the file: the file has just been
/// written and re-reading it to find out what we set is a round trip for an
/// answer already in hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub backdrop: String,
    pub scheme: String,
    pub speed: f32,
    pub strength: f32,
    pub glass: i64,
    pub animations: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            backdrop: "blobs".into(),
            scheme: "midnight".into(),
            speed: 1.0,
            strength: 1.0,
            glass: 30,
            animations: true,
        }
    }
}

/// What the Appearance pane currently says.
pub fn look(panes: &[Pane]) -> Look {
    let mut out = Look::default();
    let Some(pane) = panes.iter().find(|p| p.id == "appearance") else {
        return out;
    };
    for e in &pane.entries {
        match (e.field, &e.kind) {
            ("appearance.backdrop", Kind::Choice { at, options }) => {
                out.backdrop = options
                    .get(*at)
                    .map(|(v, _)| v.clone())
                    .unwrap_or(out.backdrop);
            }
            ("appearance.scheme", Kind::Choice { at, options }) => {
                out.scheme = options
                    .get(*at)
                    .map(|(v, _)| v.clone())
                    .unwrap_or(out.scheme);
            }
            ("appearance.backdrop_speed", Kind::Number { value, .. }) => {
                out.speed = *value as f32 / 100.0;
            }
            ("appearance.backdrop_strength", Kind::Number { value, .. }) => {
                out.strength = *value as f32 / 100.0;
            }
            ("appearance.glass", Kind::Number { value, .. }) => out.glass = *value,
            ("appearance.animations", Kind::Toggle(on)) => out.animations = *on,
            _ => {}
        }
    }
    out
}

/// A group of settings, which is one screen.
pub struct Pane {
    pub id: &'static str,
    pub label: &'static str,
    /// One line saying what is in here, for the root menu's side panel.
    /// A list of eight names with a number beside each says nothing about
    /// which one holds the thing you came to change.
    pub blurb: &'static str,
    pub entries: Vec<Entry>,
}

/// The Device pane, built from whatever the platform scheme actually offers.
///
/// A row per thing the hardware has, and no row for a thing it does not — on a
/// Mac there is no backlight and no battery, so the pane is short rather than
/// full of controls that do nothing. Which files these read is
/// `moose_rack::platform`, measured on the Flip.
fn device_entries() -> Vec<Entry> {
    let p = moose_rack::platform::current();
    let mut out = Vec::new();

    if p.wifi().is_some() {
        out.push(Entry {
            field: "",
            label: "Wi-Fi",
            help: "Join a network. Scanning uses the device's own knulli-wifi.",
            kind: Kind::Action,
        });
    }
    if let Some(b) = p.brightness() {
        // Read the real level, so the row opens on where the screen actually
        // is rather than snapping it to a default the moment you press right.
        let now = std::fs::read_to_string(&b.path)
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or((b.max / 2) as i64);
        out.push(Entry {
            field: "device.brightness",
            label: "Brightness",
            help: "Screen backlight. Left and right change it straight away.",
            kind: Kind::Number {
                value: now,
                min: 0,
                max: b.max as i64,
                step: (b.max / 16).max(1) as i64,
                unit: "",
            },
        });
    }
    if let Some(b) = p.battery() {
        let charge = std::fs::read_to_string(&b.capacity)
            .ok()
            .map(|s| format!("{}%", s.trim()))
            .unwrap_or_else(|| "unknown".into());
        out.push(Entry {
            field: "",
            label: "Battery",
            help: "The gauge on this unit reports itself as uncalibrated, so treat the percentage as rough.",
            kind: Kind::ReadOnly(charge),
        });
    }
    if out.is_empty() {
        out.push(Entry {
            field: "",
            label: "Backlight, battery, Wi-Fi",
            help: "This build has no backlight, battery or radio to offer — it is running on a desktop. On the handheld these are the controls for them.",
            kind: Kind::ReadOnly("Not on this machine".to_owned()),
        });
    }
    out
}

/// One row per console, offering the cores that console actually has.
///
/// Ours, not KNULLI's. This front end replaces the KNULLI front end and launches
/// RetroArch itself through `launch::plan`, so configgen never sees the choice
/// and `knulli.conf` is not consulted — which is just as well, because
/// configgen takes a core name verbatim with no validation and a name that does
/// not exist becomes a failed launch rather than an error.
///
/// The defaults are the picks settled on the device: `pcsx_rearmed` for PSX
/// rather than SwanStation, `flycastvl` rather than full Flycast, and Neo Geo
/// fixed to `geolith` because the ROMs here are geolith's.
fn emulator_entries(
    consoles: &[(String, String)],
    map: &moose_rack::coremap::CoreMap,
    overrides: &std::collections::BTreeMap<String, String>,
) -> Vec<Entry> {
    consoles
        .iter()
        .filter_map(|(slug, name)| {
            let mut cores: Vec<&str> = map.alternatives(slug);
            if let Some(d) = map.default_core(slug)
                && !cores.contains(&d)
            {
                cores.insert(0, d);
            }
            if cores.len() < 2 {
                // One core is not a choice. A row that cannot change is noise
                // on a screen you came to to change something.
                return None;
            }
            // What you chose, then what this device should run, then the
            // shipped default. The middle one is the point: the core map is
            // ES-DE's and assumes a desktop.
            let device_pick = moose_rack::platform::current()
                .default_cores()
                .iter()
                .find(|(p, _)| *p == slug)
                .map(|(_, c)| *c);
            let chosen = overrides
                .get(slug)
                .map(String::as_str)
                .or(device_pick)
                .or_else(|| map.default_core(slug))
                .unwrap_or(cores[0]);
            // A device pick the map does not list is still offered — it is the
            // one that runs here.
            if let Some(pick) = device_pick
                && !cores.contains(&pick)
            {
                cores.insert(0, pick);
            }
            let at = cores.iter().position(|c| *c == chosen).unwrap_or(0);
            let options: Vec<(String, String)> = cores
                .iter()
                .map(|c| {
                    let label = map.label_for(c).unwrap_or(c);
                    ((*c).to_owned(), label.to_owned())
                })
                .collect();
            Some(Entry {
                // Leaked because an entry names its field for the life of the
                // program and there are a few dozen of them, once, at startup.
                field: Box::leak(format!("cores_overrides.{slug}").into_boxed_str()),
                label: Box::leak(name.clone().into_boxed_str()),
                help: "Which core runs this console. The default is what the device was measured to run well.",
                kind: Kind::Choice { at, options },
            })
        })
        .collect()
}

/// One row per action, showing the pad button it is on.
///
/// Pad only. A handheld has no keyboard to bind, and a keyboard column here
/// would be a column of the same dash 29 times — the key bindings still exist
/// and are still edited in the desktop app, which is where somebody with a
/// keyboard is sitting.
///
/// Not every action earns a row: the ones with no pad button and no sensible
/// one to give them are the desktop's, and offering to bind them here says the
/// device can do something it cannot.
fn binding_entries(binds: &moose_rack::binds::Bindings) -> Vec<Entry> {
    moose_rack::binds::ACTIONS
        .iter()
        .filter(|a| !matches!(a.id, "search" | "help" | "sortMenu" | "filterMenu"))
        .map(|a| Entry {
            field: Box::leak(format!("bindings_pad.{}", a.id).into_boxed_str()),
            label: a.label,
            help: "Press A, then press the button you want. B clears it, and the \
                   Start button leaves without changing anything.",
            kind: Kind::Binding(binds.pad_for(a.id)),
        })
        .collect()
}

/// Build every pane from the config as it stands.
///
/// Read once when the tab is opened rather than held live: a settings screen
/// that re-reads the file while you are on it can show a value you did not set,
/// and nothing else writes that file while this app is running.
pub fn panes(
    cfg: &Config,
    consoles: &[(String, String)],
    map: &moose_rack::coremap::CoreMap,
) -> Vec<Pane> {
    let mut emulators = emulator_entries(consoles, map, &cfg.cores.overrides);
    emulators.extend(launch_entries(cfg));
    let mut control = vec![
        Entry {
            field: "controllers.swap_ab",
            label: "Swap A and B",
            help: "For moving around this app only. Some pads report the bottom face button as B; this corrects it without touching what a game sees.",
            kind: Kind::Toggle(cfg.controllers.swap_ab),
        },
        Entry {
            field: "controllers.swap_xy",
            label: "Swap X and Y",
            help: "The same for the other pair. Never written into RetroArch.",
            kind: Kind::Toggle(cfg.controllers.swap_xy),
        },
        Entry {
            field: "controllers.mirror_player_one",
            label: "Match player 1",
            help: "Give players 2 to 4 the same bindings as player 1.",
            kind: Kind::Toggle(cfg.controllers.mirror_player_one),
        },
    ];
    control.extend(binding_entries(&cfg.bindings));

    // The desktop's panes, in the desktop's order, plus Device — which the
    // desktop has no equivalent of because a laptop has no backlight you would
    // set from an app. Named the same on purpose: a settings screen that
    // reorganises everything is one you have to relearn per machine.
    vec![
        Pane {
            id: "general",
            label: "General",
            blurb: "The RomM server and the accounts games are looked up against, plus what happens when a game closes.",
            entries: vec![
                Entry {
                    field: "server.url",
                    label: "RomM server",
                    help: "Where the library is synced from.",
                    kind: Kind::Text {
                        value: cfg.server.url.clone(),
                        secret: false,
                    },
                },
                Entry {
                    field: "server.username",
                    label: "Server login",
                    help: "Your RomM account. Leave empty if you use a token.",
                    kind: Kind::Text {
                        value: cfg.server.username.clone(),
                        secret: false,
                    },
                },
                Entry {
                    field: "server.token",
                    label: "Server token",
                    help: "A RomM client token — better than a password on a device that keeps it on a card.",
                    kind: Kind::Text {
                        value: cfg.server.token.clone().unwrap_or_default(),
                        secret: true,
                    },
                },
                Entry {
                    field: "achievements.enabled",
                    label: "RetroAchievements",
                    help: "Track achievements while you play.",
                    kind: Kind::Toggle(cfg.achievements.enabled),
                },
                Entry {
                    field: "achievements.username",
                    label: "Account",
                    help: "Your RetroAchievements username.",
                    kind: Kind::Text {
                        value: cfg.achievements.username.clone().unwrap_or_default(),
                        secret: false,
                    },
                },
                Entry {
                    field: "achievements.token",
                    label: "Token",
                    help: "From your RetroAchievements settings page. Not your password.",
                    kind: Kind::Text {
                        value: cfg.achievements.token.clone().unwrap_or_default(),
                        secret: true,
                    },
                },
                Entry {
                    field: "achievements.hardcore",
                    label: "Hardcore mode",
                    help: "No save states and no rewind, for the achievements that require it.",
                    kind: Kind::Toggle(cfg.achievements.hardcore),
                },
                Entry {
                    field: "saves.confirm_delete_state",
                    label: "Ask before deleting",
                    help: "Confirm before a save state is thrown away.",
                    kind: Kind::Toggle(cfg.saves.confirm_delete_state),
                },
                // No ScreenScraper rows. The desktop writes `[scraper]` and
                // nothing in the codebase reads it — its own dialog says "not
                // used by the app yet". A row for a setting that cannot do
                // anything is worse than a missing one, because you try it.
                Entry {
                    field: "",
                    label: "Sync saves now",
                    help: "Compare your saves and save states with the server. Not wired up on this front end yet.",
                    kind: Kind::Action,
                },
            ],
        },
        Pane {
            id: "appearance",
            label: "Appearance",
            blurb: "The color scheme, the moving backdrop behind everything, how much glass the panels have, and what the lists show.",
            entries: vec![
                Entry {
                    field: "media.list_art",
                    label: "Game list shows",
                    help: "Which picture a game is drawn with.",
                    kind: Kind::Choice {
                        at: art_at(&cfg.media.list_art),
                        options: art_options(),
                    },
                },
                Entry {
                    field: "media.detail_art",
                    label: "Show",
                    help: "Which picture the pane beside the list uses.",
                    kind: Kind::Choice {
                        at: art_at(&cfg.media.detail_art),
                        options: art_options(),
                    },
                },
                Entry {
                    field: "appearance.backdrop",
                    label: "Backdrop",
                    help: "Which shader draws behind everything. The webview keeps its own in the browser, so this one is the handheld's.",
                    kind: Kind::Choice {
                        at: backdrop_at(&cfg.appearance.backdrop),
                        options: backdrop_options(),
                    },
                },
                Entry {
                    field: "appearance.scheme",
                    label: "Color scheme",
                    help: "The colors the backdrop and the glass are drawn in. The same nine the desktop offers.",
                    kind: Kind::Choice {
                        at: scheme_at(&cfg.appearance.scheme),
                        options: scheme_options(),
                    },
                },
                Entry {
                    field: "appearance.backdrop_speed",
                    label: "Speed",
                    help: "How fast the backdrop moves, against the style's own pace.",
                    kind: Kind::Number {
                        value: cfg.appearance.backdrop_speed,
                        min: 0,
                        max: 300,
                        step: 10,
                        unit: "%",
                    },
                },
                Entry {
                    field: "appearance.backdrop_strength",
                    label: "Strength",
                    help: "How strongly the backdrop is drawn. Zero is a plain dark screen.",
                    kind: Kind::Number {
                        value: cfg.appearance.backdrop_strength,
                        min: 0,
                        max: 200,
                        step: 10,
                        unit: "%",
                    },
                },
                Entry {
                    field: "appearance.animations",
                    label: "Animations",
                    help: "Whether pages slide as you move between tabs, and whatever else moves later. One switch for all of it — turning animation off should not mean hunting for six more switches.",
                    kind: Kind::Toggle(cfg.appearance.animations),
                },
                Entry {
                    field: "appearance.glass",
                    label: "Glass",
                    help: "How frosted the panels are. The blur is a shader, so this costs nothing to raise.",
                    kind: Kind::Number {
                        value: cfg.appearance.glass,
                        min: 0,
                        max: 60,
                        step: 5,
                        unit: "",
                    },
                },
                Entry {
                    field: "icons.style",
                    label: "Console picture",
                    help: "How a console is drawn on the Library screen.",
                    kind: choice(icon_style_options(&cfg.media_dir(), &cfg.icons.set), &cfg.icons.style),
                },
                Entry {
                    field: "shaders.motion",
                    label: "Motion layer",
                    help: "A shader pass over the game itself — black-frame insertion and the like.",
                    kind: choice(
                        motion_options(),
                        cfg.shaders.motion.as_deref().unwrap_or(""),
                    ),
                },
                Entry {
                    field: "shaders.enabled",
                    label: "Shaders",
                    help: "Apply the shader chain when a game launches.",
                    kind: Kind::Toggle(cfg.shaders.enabled),
                },
            ],
        },
        Pane {
            id: "control",
            label: "Control",
            blurb: "Which button does what in this app, plus the two face-button swaps for pads that report them the other way round.",
            entries: control,
        },
        Pane {
            id: "library",
            label: "Library",
            blurb: "Where the ROMs live, whether RomM's own collections appear beside yours, and what to fetch.",
            entries: vec![
                Entry {
                    field: "library.local_root",
                    label: "Folder",
                    help: "Set by the device image. Change it in moose-sdl.toml on the card.",
                    kind: Kind::ReadOnly(cfg.library.local_root.clone()),
                },
                Entry {
                    field: "library.server_collections",
                    label: "RomM collections",
                    help: "Show the collections RomM generates by company, genre and franchise alongside your own.",
                    kind: Kind::Toggle(cfg.library.server_collections),
                },
                Entry {
                    field: "",
                    label: "Fetch game list",
                    help: "Pull the library from the server. Not wired up on this front end yet — see the Syncing tab.",
                    kind: Kind::Action,
                },
                Entry {
                    field: "",
                    label: "BIOS files",
                    help: "Check which consoles are missing the files they need. Not wired up yet.",
                    kind: Kind::Action,
                },
                Entry {
                    field: "",
                    label: "Missing artwork",
                    help: "Find games with no picture. Not wired up yet.",
                    kind: Kind::Action,
                },
            ],
        },
        Pane {
            id: "emulators",
            label: "Emulators",
            blurb: "Which core runs each console, and what RetroArch is told to do when a game starts and stops.",
            entries: emulators,
        },
        Pane {
            id: "iconsets",
            label: "Icon sets",
            blurb: "The console pictures the grid draws, and which downloaded set they come from.",
            entries: vec![
                Entry {
                    field: "icons.set",
                    label: "Drawing from",
                    help: "Which set the console pictures come from.",
                    kind: choice(icon_set_options(&cfg.media_dir()), &cfg.icons.set),
                },
                Entry {
                    field: "icons.style",
                    label: "Style",
                    help: "How a console is drawn within that set.",
                    kind: choice(icon_style_options(&cfg.media_dir(), &cfg.icons.set), &cfg.icons.style),
                },
                Entry {
                    field: "",
                    label: "Get pictures",
                    help: "Download a set of console pictures over Wi-Fi. Pick which one; it fetches only the consoles your library actually has, and replaces whatever that set had before.",
                    kind: Kind::Action,
                },
            ],
        },
        Pane {
            id: "device",
            label: "Device",
            blurb: "Screen brightness, Wi-Fi, and the rest of what this machine itself does.",
            entries: device_entries(),
        },
        Pane {
            id: "exit",
            label: "Exit",
            blurb: "Leave Moose Rack and go back to the device's own menu.",
            entries: vec![Entry {
                field: "",
                label: "Back to EmulationStation",
                help: "Closes Moose Rack. The device's own front end comes back where it left off.",
                kind: Kind::Action,
            }],
        },
        Pane {
            id: "about",
            label: "About",
            blurb: "This build, and what it is running on.",
            entries: about_entries(),
        },
    ]
}

/// The About page: this build, then what it is running on.
///
/// The machine facts are the ones KNULLI's own System Information page shows,
/// read the same way it reads them. A Mac has no thermal zone and no board
/// name in the same place; those lines are simply absent there rather than
/// showing a zero.
fn about_entries() -> Vec<Entry> {
    let mut entries = vec![
                Entry {
                    field: "",
                    label: "Version",
                    help: "This build.",
                    kind: Kind::ReadOnly(env!("CARGO_PKG_VERSION").to_owned()),
                },
                Entry {
                    field: "",
                    label: "Scheme",
                    help: "Which platform scheme this build selected.",
                    kind: Kind::ReadOnly(moose_rack::platform::current().scheme().to_owned()),
                },
                Entry {
                    field: "",
                    label: "By",
                    help: "Who wrote it.",
                    kind: Kind::ReadOnly("mizaimao".to_owned()),
                },
                Entry {
                    field: "",
                    label: "Source",
                    help: "github.com/mizaimao/moose-rack",
                    kind: Kind::ReadOnly("moose-rack".to_owned()),
                },
    ];
    entries.extend(crate::sysinfo::facts().into_iter().map(|(label, value)| Entry {
        field: "",
        label,
        help: "Read from this machine.",
        kind: Kind::ReadOnly(value),
    }));
    entries
}

/// How a game is launched — the desktop's Emulators pane minus the two that
/// mean nothing without a window.
///
/// "Fit to the game" and "Title bar" are gone: there is no window to fit and no
/// bar to hide.
fn launch_entries(cfg: &Config) -> Vec<Entry> {
    vec![
        Entry {
            field: "retroarch.save_state_on_exit",
            label: "Save state on exit",
            help: "Write a save state when a game is closed, so it reopens where you left it.",
            kind: Kind::Toggle(cfg.retroarch.save_state_on_exit),
        },
        Entry {
            field: "retroarch.autofire_hz",
            label: "Auto-fire speed",
            help: "Shots a second while auto-fire is held.",
            kind: Kind::Number {
                value: cfg.retroarch.autofire_hz as i64,
                min: 1,
                max: 30,
                step: 1,
                unit: "/s",
            },
        },
        Entry {
            field: "retroarch.autofire",
            label: "Auto-fire",
            help: "Hold to repeat, or press once to toggle repeating.",
            kind: Kind::Choice {
                at: autofire_at(Some(cfg.retroarch.autofire.as_str())),
                options: autofire_options(),
            },
        },
    ]
}

/// The backdrops this front end can draw, from the shader table itself — so a
/// style added there appears here without a second list to keep in step.
fn backdrop_options() -> Vec<(String, String)> {
    moose_sdl_styles()
        .iter()
        .map(|(id, label)| ((*id).to_owned(), (*label).to_owned()))
        .collect()
}

/// A choice already positioned on the value it holds.
///
/// One helper because getting this wrong is silent: an index that does not match
/// the stored value opens the list on the wrong row and, worse, shows the wrong
/// answer on the settings screen itself.
fn choice(options: Vec<(String, String)>, current: &str) -> Kind {
    // A list of one is not a choice.
    //
    // It looks like one — the row has the chevron, pressing A opens a sheet —
    // and then the sheet holds a single line and closes on the value that was
    // already there. Shown rather than offered, so the screen stops promising
    // something it cannot do. What makes the choice real is elsewhere: for
    // icon sets it is downloading a second one.
    if options.len() < 2 {
        let label = options
            .first()
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| "Nothing to choose from".to_owned());
        return Kind::ReadOnly(label);
    }
    let at = options.iter().position(|(v, _)| v == current).unwrap_or(0);
    Kind::Choice { at, options }
}

/// The icon sets the shipped table describes.
fn icon_set_options(media_root: &std::path::Path) -> Vec<(String, String)> {
    let installed = moose_rack::theme::installed_sets(media_root);
    if installed.is_empty() {
        // Nothing downloaded. One row saying so beats a list of names that all
        // draw a blank screen.
        return vec![(String::new(), "None downloaded".to_owned())];
    }
    std::iter::once((String::new(), "Off".to_owned()))
        .chain(installed.into_iter().map(|(id, n)| {
            let label = id.trim_end_matches("-es-de").replace('-', " ");
            (id, format!("{label} ({n})"))
        }))
        .collect()
}

/// The looks one set offers. A set with no table entry offers nothing rather
/// than a free-text box.
fn icon_style_options(media_root: &std::path::Path, set: &str) -> Vec<(String, String)> {
    // Same rule as the sets themselves: a look with no pictures under it is a
    // choice that draws a blank screen, so it is not offered. An empty set id
    // means none is chosen, and then there are no looks to pick between.
    if set.is_empty() {
        return vec![(String::new(), "None".to_owned())];
    }
    let Some(art) = moose_rack::iconart::of(set) else {
        return vec![(String::new(), "None".to_owned())];
    };
    let looks: Vec<(String, String)> = art
        .looks
        .iter()
        .filter(|l| {
            moose_rack::theme::set_dir(media_root, set, &l.id)
                .read_dir()
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
        })
        .map(|l| (l.id.clone(), l.label.clone()))
        .collect();
    if looks.is_empty() {
        return vec![(String::new(), "None".to_owned())];
    }
    looks
}

fn motion_options() -> Vec<(String, String)> {
    std::iter::once((String::new(), "Off".to_owned()))
        .chain(
            moose_rack::shaders::MOTION
                .iter()
                .map(|o| (o.path.to_owned(), o.label.to_owned())),
        )
        .collect()
}

fn scheme_options() -> Vec<(String, String)> {
    crate::backdrop::SCHEMES
        .iter()
        .map(|s| (s.id.to_owned(), s.label.to_owned()))
        .collect()
}

fn scheme_at(current: &str) -> usize {
    crate::backdrop::SCHEMES
        .iter()
        .position(|s| s.id == current)
        .unwrap_or(0)
}

fn backdrop_at(current: &str) -> usize {
    moose_sdl_styles()
        .iter()
        .position(|(id, _)| *id == current)
        .unwrap_or(0)
}

/// Indirection so `settings` does not depend on the renderer's module layout.
fn moose_sdl_styles() -> &'static [(&'static str, &'static str)] {
    crate::backdrop::STYLE_LIST
}

fn art_options() -> Vec<(String, String)> {
    ["box", "title", "screenshot", "logo", "none"]
        .iter()
        .map(|k| ((*k).to_owned(), k.replace('_', " ")))
        .collect()
}

fn art_at(current: &str) -> usize {
    art_options()
        .iter()
        .position(|(k, _)| k == current)
        .unwrap_or(0)
}

fn autofire_options() -> Vec<(String, String)> {
    vec![
        (String::new(), "Off".to_owned()),
        ("hold".to_owned(), "Hold".to_owned()),
        ("toggle".to_owned(), "Toggle".to_owned()),
    ]
}

fn autofire_at(current: Option<&str>) -> usize {
    let now = current.unwrap_or("");
    autofire_options()
        .iter()
        .position(|(k, _)| k == now)
        .unwrap_or(0)
}

/// Write one setting to [`FILE`].
///
/// The same three functions the desktop's Tauri command calls, so a value set
/// here and a value set there are written the same way into the same format —
/// into this front end's own file. The field is `table.key`; anything without a
/// dot is not ours to write.
pub fn write(field: &str, value: &Written) -> anyhow::Result<()> {
    write_to(FILE, field, value)
}

/// The same, into a named file.
///
/// Split out so the wiring audit can round-trip every setting through a scratch
/// file: change it, read the file back, and check the value actually came back
/// changed. A field name that does not match anything in `Config` writes
/// happily into a table nothing reads, which looks exactly like it worked —
/// that is the failure this exists to catch.
pub fn write_to(file: &str, field: &str, value: &Written) -> anyhow::Result<()> {
    let Some((table, key)) = field.split_once('.') else {
        anyhow::bail!("{field} is not a config field");
    };
    // `device.*` is the hardware, not the config file. Backlight is the one
    // setting where the file would be the wrong place entirely: it has to take
    // effect as you hold the button, and the device already remembers it.
    if table == "device" {
        return apply_device(key, value);
    }
    // Only now, once it is settled that something is actually going to be
    // written. Seeding before the hardware branch created a config file for a
    // brightness change that never touches one — including while the tests ran.
    seed_into(file)?;
    if table == "bindings_pad" {
        let Written::Text(v) = value else {
            anyhow::bail!("a binding is a button number or nothing");
        };
        let button = if v.is_empty() {
            None
        } else {
            Some(v.parse::<u8>()?)
        };
        return set_pad_binding(key, button);
    }
    let table = toml_table(table);
    match value {
        Written::Bool(v) => config::set_table_bool(file, table, key, *v),
        Written::Number(v) => config::set_table_number(file, table, key, *v),
        Written::Text(v) => config::set_table_entry(file, table, key, v),
    }
}

/// Move an action to another pad button, and save it.
///
/// Through `Bindings::set_pad` rather than by writing the key directly: that
/// method also clears whatever else was on the button, and two actions on one
/// button is a pad where one of them silently stops working.
pub fn set_pad_binding(action: &str, button: Option<u8>) -> anyhow::Result<()> {
    seed()?;
    let mut cfg = Config::load_from(std::path::Path::new(FILE)).unwrap_or_default();
    cfg.bindings.set_pad(action, button);
    // Say so rather than let the healing quietly move things around. The map
    // cannot be stranded any more, but a rebind that silently lands somewhere
    // else is its own confusion — better to refuse the press and explain.
    if let Some(stranded) = cfg.bindings.stranded() {
        anyhow::bail!("that would leave {stranded} on no button");
    }
    let entries: Vec<(String, Option<String>)> = cfg
        .bindings
        .pad
        .iter()
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect();
    config::set_table_entries(FILE, "bindings.pad", &entries).context("saving the pad bindings")
}

/// Make sure this front end's file exists before writing one setting into it.
///
/// Copied from `config.toml` when there is one. Without this the first setting
/// changed on the handheld created a file holding *only* that setting — and
/// since [`load`] prefers this file whole, every other value silently reverted
/// to its default. One press on the settings screen and the server address was
/// gone.
fn seed() -> anyhow::Result<()> {
    seed_into(FILE)
}

fn seed_into(file: &str) -> anyhow::Result<()> {
    if Config::exists(file) {
        return Ok(());
    }
    if Config::exists("config.toml") {
        std::fs::copy("config.toml", file)
            .with_context(|| format!("seeding {file} from config.toml"))?;
    }
    Ok(())
}

/// The TOML table a field's prefix means.
///
/// `[cores.overrides]` is a sub-table and `table.key` cannot spell a second
/// dot, so the field says `cores_overrides` and this puts it back. Get it wrong
/// and every core choice lands in a `[cores_overrides]` table nothing reads —
/// which looks exactly like it worked.
fn toml_table(prefix: &str) -> &str {
    match prefix {
        "cores_overrides" => "cores.overrides",
        other => other,
    }
}

/// Apply a hardware setting, now.
///
/// Through the vendor wrapper where there is one — `knulli-brightness` also
/// handles whatever else the board couples to the backlight — and by writing
/// the sysfs node when there is not.
fn apply_device(key: &str, value: &Written) -> anyhow::Result<()> {
    let Written::Number(n) = value else {
        anyhow::bail!("device.{key} takes a number");
    };
    match key {
        "brightness" => {
            let Some(b) = moose_rack::platform::current().brightness() else {
                anyhow::bail!("this device has no backlight");
            };
            if let Some(helper) = b.helper
                && std::process::Command::new(helper)
                    .arg(n.to_string())
                    .status()
                    .is_ok_and(|s| s.success())
            {
                return Ok(());
            }
            std::fs::write(&b.path, n.to_string())
                .with_context(|| format!("writing {}", b.path.display()))
        }
        other => anyhow::bail!("nothing applies device.{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A couple of real consoles and the shipped core map, so the emulator
    /// rows in these tests are the ones the app builds.
    fn built() -> Vec<Pane> {
        let cfg = Config::default();
        let consoles = [
            ("megadrive".to_owned(), "Sega Mega Drive".to_owned()),
            ("snes".to_owned(), "Super Nintendo".to_owned()),
        ];
        panes(&cfg, &consoles, &moose_rack::coremap::CoreMap::embedded())
    }

    fn entry(kind: Kind) -> Entry {
        Entry {
            field: "a.b",
            label: "L",
            help: "H",
            kind,
        }
    }

    /// A toggle flips whichever way you press, because a settings list where
    /// half the rows ignore "left" is a list you stop trusting.
    #[test]
    fn a_toggle_flips_in_either_direction() {
        let mut e = entry(Kind::Toggle(false));
        assert_eq!(e.step(1), Some(Written::Bool(true)));
        assert_eq!(e.value(), "On");
        assert_eq!(e.step(-1), Some(Written::Bool(false)));
        assert_eq!(e.value(), "Off");
    }

    /// A choice wraps, so the last option is one press left of the first.
    #[test]
    fn a_choice_wraps_both_ways() {
        let mut e = entry(Kind::Choice {
            at: 0,
            options: autofire_options(),
        });
        assert_eq!(e.value(), "Off");
        assert_eq!(e.step(-1), Some(Written::Text("toggle".into())));
        assert_eq!(e.value(), "Toggle");
        e.step(1);
        assert_eq!(e.value(), "Off");
    }

    /// A number stops at its ends rather than wrapping — brightness rolling
    /// from full to off in one press is a dark screen and a hunt for the
    /// button.
    #[test]
    fn a_number_clamps_and_does_not_wrap() {
        let mut e = entry(Kind::Number {
            value: 250,
            min: 0,
            max: 255,
            step: 16,
            unit: "",
        });
        e.step(1);
        assert_eq!(e.value(), "255");
        e.step(1);
        assert_eq!(e.value(), "255", "brightness wrapped past full");
        for _ in 0..40 {
            e.step(-1);
        }
        assert_eq!(e.value(), "0");
    }

    /// Text and read-only rows are not stepped; left and right do nothing on
    /// them rather than half-changing something.
    #[test]
    fn text_and_read_only_do_not_step() {
        let mut t = entry(Kind::Text {
            value: "x".into(),
            secret: false,
        });
        assert_eq!(t.step(1), None);
        assert!(!t.steps());
        let mut r = entry(Kind::ReadOnly("/userdata/roms".into()));
        assert_eq!(r.step(1), None);
        assert!(!r.steps());
    }

    /// A secret is never shown, and an empty field says so rather than looking
    /// like a blank value somebody set on purpose.
    #[test]
    fn a_secret_is_dots_and_an_empty_field_says_so() {
        assert_eq!(
            entry(Kind::Text {
                value: String::new(),
                secret: true
            })
            .value(),
            "not set"
        );
        let e = entry(Kind::Text {
            value: "abcd".into(),
            secret: true,
        });
        assert_eq!(e.value(), "\u{2022}\u{2022}\u{2022}\u{2022}");
        // Long secrets do not draw a hundred dots across the pane.
        let long = entry(Kind::Text {
            value: "x".repeat(64),
            secret: true,
        });
        assert_eq!(long.value().chars().count(), 12);
    }

    /// Every field either names a `table.key` the writer can reach, or is
    /// empty. A typo here writes nothing and reports success, which is the
    /// quietest way for a settings screen to be broken.
    #[test]
    fn every_field_is_writable_or_deliberately_not() {
        for pane in built() {
            for e in pane.entries {
                if e.field.is_empty() {
                    assert!(
                        !e.steps() || matches!(e.kind, Kind::Number { .. }),
                        "{} has no field but is editable as a config value",
                        e.label
                    );
                    continue;
                }
                assert!(
                    e.field.split_once('.').is_some(),
                    "{} names {:?}, which is not table.key",
                    e.label,
                    e.field
                );
            }
        }
    }

    /// The panes the handheld has, and not the ones it does not. Window
    /// arrangement, the title bar and fit-to-window are desktop questions.
    #[test]
    fn no_pane_offers_a_setting_the_handheld_cannot_honour() {
        let labels: Vec<String> = built()
            .iter()
            .flat_map(|p| p.entries.iter().map(|e| e.label.to_lowercase()))
            .collect();
        for absent in ["window", "title bar", "fit to the game"] {
            assert!(
                !labels.iter().any(|l| l.contains(absent)),
                "{absent:?} means nothing on a device with no windows"
            );
        }
    }

    /// `device.*` is the hardware and must never reach the config file.
    ///
    /// Backlight in a file is the wrong place twice over: it has to take effect
    /// while the button is held, and the device already remembers it across a
    /// reboot. A row that wrote it to TOML would look like it worked and change
    /// nothing on screen.
    #[test]
    fn device_settings_do_not_go_to_the_config_file() {
        // No backlight on the machine the tests run on, so this reports that
        // rather than writing `[device] brightness` into moose-sdl.toml.
        let err = write("device.brightness", &Written::Number(100)).unwrap_err();
        assert!(
            format!("{err:#}").contains("backlight"),
            "expected a hardware error, got: {err:#}"
        );
        assert!(
            !std::path::Path::new(FILE).exists() || {
                let raw = std::fs::read_to_string(FILE).unwrap_or_default();
                !raw.contains("[device]")
            }
        );
    }

    /// A hardware key nothing knows how to apply is an error, not a silent
    /// success.
    #[test]
    fn an_unknown_device_setting_is_refused() {
        assert!(write("device.nonsense", &Written::Number(1)).is_err());
        assert!(write("device.brightness", &Written::Bool(true)).is_err());
    }

    /// A console with a real choice of cores gets a row; one with a single
    /// core does not.
    ///
    /// A row that cannot change is noise on a screen you came to to change
    /// something.
    #[test]
    fn emulators_offers_only_consoles_with_a_choice() {
        let emu = built()
            .into_iter()
            .find(|p| p.id == "emulators")
            .expect("an Emulators pane");
        assert!(!emu.entries.is_empty(), "no console offered a core choice");
        // The per-console rows; the pane also carries the launch settings.
        for e in emu
            .entries
            .iter()
            .filter(|e| e.field.starts_with("cores_overrides."))
        {
            let Kind::Choice { options, .. } = &e.kind else {
                panic!("{} is not a choice", e.label);
            };
            assert!(
                options.len() >= 2,
                "{} has one option and should not be a row",
                e.label
            );
        }
    }

    /// The sub-table is spelled back out before anything is written.
    ///
    /// Checked on the translation rather than by calling `write`, which would
    /// create a real config file while the tests run.
    #[test]
    fn the_overrides_sub_table_is_spelled_back_out() {
        assert_eq!(toml_table("cores_overrides"), "cores.overrides");
        assert_eq!(toml_table("library"), "library");
        assert_eq!(toml_table("server"), "server");
    }

    /// Controls are captured, not stepped.
    ///
    /// Stepping through button names to bind a control is a worse control than
    /// the problem it avoids. `Binding` is its own kind precisely so the screen
    /// cannot fall back to left and right on it.
    #[test]
    fn controls_are_captured_rather_than_stepped() {
        let pane = built()
            .into_iter()
            .find(|p| p.id == "control")
            .expect("a Control pane");
        let bindings: Vec<&Entry> = pane
            .entries
            .iter()
            .filter(|e| e.field.starts_with("bindings_pad."))
            .collect();
        assert!(!bindings.is_empty(), "no bindings in the Control pane");
        for e in bindings {
            assert!(
                matches!(e.kind, Kind::Binding(_)),
                "{} is not a binding, so it would be stepped",
                e.label
            );
            assert!(
                !e.steps(),
                "{} can still be changed with left and right",
                e.label
            );
        }
    }

    /// An unbound control says so; a bound one names its button.
    #[test]
    fn a_binding_reads_as_the_button_it_is_on() {
        assert_eq!(
            entry(Kind::Binding(None)).value(),
            moose_rack::binds::pad_label(None)
        );
        let bound = entry(Kind::Binding(Some(0)));
        assert_eq!(bound.value(), moose_rack::binds::pad_label(Some(0)));
        assert_ne!(bound.value(), "", "a bound button drew nothing");
    }

    /// Appearance is not three rows any more.
    ///
    /// The backdrop, its speed and strength, and the glass were missing because
    /// the desktop keeps them in the browser's local storage rather than in
    /// config.toml — so there was nothing to copy across, and shipping three
    /// rows without saying that was the wrong answer twice.
    #[test]
    fn appearance_can_actually_change_how_it_looks() {
        let pane = built()
            .into_iter()
            .find(|p| p.id == "appearance")
            .expect("an Appearance pane");
        let labels: Vec<&str> = pane.entries.iter().map(|e| e.label).collect();
        for wanted in ["Backdrop", "Speed", "Strength", "Glass"] {
            assert!(
                labels.contains(&wanted),
                "Appearance has no {wanted:?}: {labels:?}"
            );
        }
        assert!(pane.entries.len() >= 6, "still a stub: {labels:?}");
    }

    /// Nothing is a text box unless a person genuinely has to type it.
    ///
    /// The audit Frank asked for, as a test rather than a reading. A text box on
    /// a handheld costs the on-screen keyboard and a dozen presses, so it is
    /// only right for a value nobody can enumerate — an address, a login, a
    /// token. Everything else is a name from a list this app already holds, and
    /// a list is one press.
    #[test]
    fn only_genuinely_free_text_is_a_text_box() {
        // The complete set, with why. Anything else typed in is a bug.
        const TYPED: &[&str] = &[
            "server.url",
            "server.username",
            "server.token",
            "achievements.username",
            "achievements.token",
        ];
        for pane in built() {
            for e in pane.entries {
                if matches!(e.kind, Kind::Text { .. }) {
                    assert!(
                        TYPED.contains(&e.field),
                        "{} ({}) is a text box and its values are a fixed list",
                        e.label,
                        e.field
                    );
                }
            }
        }
    }

    /// A list must open on the value it is holding.
    ///
    /// An index that does not match the stored value is silent twice: the
    /// settings row shows the wrong answer, and the list opens on the wrong row.
    #[test]
    fn every_list_is_positioned_on_its_own_value() {
        for pane in built() {
            for e in &pane.entries {
                let Kind::Choice { at, options } = &e.kind else {
                    continue;
                };
                assert!(
                    options.is_empty() || *at < options.len(),
                    "{} points past its own options",
                    e.label
                );
                assert!(
                    !options.is_empty() || e.field.is_empty(),
                    "{} is a list with nothing in it",
                    e.label
                );
            }
        }
    }

    /// The renderer is driven by what the pane says, not by the file.
    ///
    /// A setting that writes `moose-sdl.toml` and changes nothing on screen is
    /// not wired up, it is a note to self. `look` is what closes that: the loop
    /// compares it against what it last drew and rebuilds when they differ.
    #[test]
    fn look_reads_the_appearance_pane_back() {
        let mut panes = built();
        let now = look(&panes);
        assert_eq!(now.backdrop, "blobs");
        assert_eq!(now.scheme, "midnight");
        assert_eq!(now.speed, 1.0, "100% is a multiplier of one");
        assert_eq!(now.strength, 1.0);

        // Choose a different backdrop, the way the list does.
        let pane = panes.iter_mut().find(|p| p.id == "appearance").unwrap();
        let entry = pane
            .entries
            .iter_mut()
            .find(|e| e.field == "appearance.backdrop")
            .unwrap();
        let Kind::Choice { at, options } = &mut entry.kind else {
            panic!("not a choice")
        };
        *at = options
            .iter()
            .position(|(v, _)| v == "towers")
            .expect("towers is a backdrop");

        assert_eq!(
            look(&panes).backdrop,
            "towers",
            "the pane changed and look did not"
        );
    }

    /// A slider moves the number the renderer is given, not just the label.
    #[test]
    fn the_sliders_reach_the_renderer() {
        let mut panes = built();
        let pane = panes.iter_mut().find(|p| p.id == "appearance").unwrap();
        for (field, expect) in [
            ("appearance.backdrop_strength", 0.0),
            ("appearance.glass", 0.0),
        ] {
            let e = pane.entries.iter_mut().find(|e| e.field == field).unwrap();
            let Kind::Number { value, .. } = &mut e.kind else {
                panic!("{field} is not a number")
            };
            *value = 0;
            let _ = expect;
        }
        let now = look(&panes);
        assert_eq!(now.strength, 0.0, "strength zero must reach the shader");
        assert_eq!(now.glass, 0, "glass zero must reach the blur");
    }

    /// And they are the same shaders, character for character.
    ///
    /// The lists matching is not the same as the pictures matching. Towers was
    /// fixed on the web side — a step() at the horizon left the floor at 0.13
    /// on one side and 0 on the other, with a dark band sitting on a bright
    /// line between them — and the fix never reached here, so the handheld drew
    /// the bug for as long as the two files have existed. Nothing said a word:
    /// both compiled, both ran, and the test below only ever compared names.
    ///
    /// Compares the compiled-in bodies against the text of `ui/js/backdrop.js`,
    /// so there is no second parser of this crate's own source to keep right.
    #[test]
    fn the_shader_bodies_are_the_same_text_in_both_front_ends() {
        let js = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/js/backdrop.js");
        let Ok(text) = std::fs::read_to_string(js) else {
            return;
        };
        let mut checked = 0;
        for style in crate::backdrop::STYLES {
            let head = format!("id: \"{}\",", style.id);
            let Some(at) = text.find(&head) else {
                panic!("{} is not in ui/js/backdrop.js", style.id);
            };
            let open = "body: `\n";
            let from = text[at..].find(open).expect("that style has no body") + at + open.len();
            let to = from + text[from..].find("`,\n").expect("that body never ends");
            // Trimmed at both ends, and only there. A Rust raw string is
            // closed on its own line and a JS template literal is closed on the
            // last line of the shader, so every body here ends with a newline
            // that its twin does not have — a difference in how the two
            // languages end a string, not in what the GPU is given. Everything
            // between the ends is compared exactly.
            assert_eq!(
                text[from..to].trim(),
                style.body.trim(),
                "the {} shader has drifted between the two front ends",
                style.id
            );
            checked += 1;
        }
        assert!(checked >= 8, "only {checked} bodies were compared");
    }

    /// Both front ends offer the same backdrops and the same color schemes.
    ///
    /// The SDL renderer shipped with three styles and no schemes against the
    /// webview's eleven and nine, which is most of what "the gap is still huge"
    /// meant. The shader bodies are identical text in both, so there is no
    /// reason for the lists to differ.
    ///
    /// Read out of `ui/js/backdrop.js` rather than counted here. This asserted
    /// a literal eleven and named eight ids, so adding a style to the webview
    /// failed it with "the webview has eleven backdrop styles" — a message
    /// about the number this test was written with, pointing at neither the
    /// style that was added nor the file it was added to.
    #[test]
    fn the_backdrops_and_schemes_match_the_webviews() {
        let js = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/js/backdrop.js");
        let Ok(text) = std::fs::read_to_string(js) else {
            // The SDL frontend is built on the handheld from this crate alone.
            // No webview to compare against is not a failure.
            return;
        };
        // Everything between BACKDROPS and its closing bracket, so the schemes
        // further down the file are not read as styles.
        let list = text
            .split_once("export const BACKDROPS = [")
            .and_then(|(_, rest)| rest.split_once("\n];"))
            .map(|(inner, _)| inner)
            .unwrap_or_default();
        let webview: Vec<&str> = list
            .lines()
            .filter_map(|l| l.trim().strip_prefix("id: \""))
            .filter_map(|l| l.split_once('"'))
            .map(|(id, _)| id)
            .collect();
        assert!(
            webview.len() >= 8,
            "only found {} styles in ui/js/backdrop.js; the parsing above has gone stale",
            webview.len()
        );
        assert_eq!(
            crate::backdrop::SCHEMES.len(),
            9,
            "the webview has nine named schemes, plus a custom one the pad drops"
        );
        for id in &webview {
            assert!(
                crate::backdrop::STYLE_LIST.iter().any(|(s, _)| s == id),
                "{id} is in the webview and not here"
            );
        }
        for (id, _) in crate::backdrop::STYLE_LIST {
            assert!(
                webview.contains(id),
                "{id} is here and not in the webview"
            );
        }
        for id in [
            "midnight", "frost", "abyss", "moss", "ember", "rust", "wine", "plum", "slate",
        ] {
            assert!(
                crate::backdrop::SCHEMES.iter().any(|s| s.id == id),
                "{id} is in the webview and not here"
            );
        }
    }

    /// Every ported style still compiles as a shader body, in the sense that it
    /// is not empty and mentions the one variable the frame is built from.
    #[test]
    fn every_ported_style_has_a_body_that_sets_the_colour() {
        for style in crate::backdrop::STYLES {
            assert!(
                style.body.contains("base"),
                "{} never assigns base, so it would draw nothing",
                style.id
            );
        }
    }

    /// The backdrop list is the renderer's own, so a style added to the shader
    /// table cannot go missing from the setting that picks it.
    #[test]
    fn the_backdrop_list_comes_from_the_renderer() {
        let opts = backdrop_options();
        assert_eq!(opts.len(), crate::backdrop::STYLE_LIST.len());
        assert_eq!(backdrop_at("aurora"), 1);
        assert_eq!(
            scheme_at("nonsense"),
            0,
            "an unknown scheme is the first, not a panic"
        );
        assert_eq!(
            backdrop_at("nonsense"),
            0,
            "an unknown style is the first, not a panic"
        );
    }

    /// Everything the desktop can change, this can change too — or there is a
    /// reason written down for why it cannot.
    ///
    /// The list on the left is the desktop's `set_config_field` table, verbatim.
    /// This is what stops the gap reopening: a setting added there and not here
    /// fails, rather than being noticed months later by somebody looking for it.
    #[test]
    fn nothing_the_desktop_can_set_is_quietly_missing() {
        // (field, why it is absent here) — an empty reason means it must exist.
        const DESKTOP: &[(&str, &str)] = &[
            ("library.local_root", ""),
            ("server.url", ""),
            ("server.token", ""),
            ("server.username", ""),
            ("achievements.enabled", ""),
            ("achievements.username", ""),
            ("achievements.token", ""),
            ("achievements.hardcore", ""),
            ("shaders.enabled", ""),
            ("saves.confirm_delete_state", ""),
            ("controllers.mirror_player_one", ""),
            ("retroarch.autofire", ""),
            ("retroarch.autofire_hz", ""),
            ("retroarch.save_state_on_exit", ""),
            ("icons.set", ""),
            ("icons.style", ""),
            ("media.list_art", ""),
            ("shaders.motion", ""),
            ("scraper.ssid", "written by the desktop and read by nothing"),
            (
                "scraper.sspassword",
                "written by the desktop and read by nothing",
            ),
            ("retroarch.game_display", "no second display on a handheld"),
            ("retroarch.fit_window", "no window to fit"),
            ("retroarch.window_decorations", "no title bar"),
            ("appearance.app_icon", "no dock to put an icon in"),
        ];

        let fields: Vec<&str> = built()
            .iter()
            .flat_map(|p| p.entries.iter().map(|e| e.field))
            .collect();
        for (field, why) in DESKTOP {
            let here = fields.contains(field);
            if why.is_empty() {
                assert!(here, "the desktop can set {field:?} and this cannot");
            } else {
                assert!(!here, "{field:?} is here but was excluded because: {why}");
            }
        }
    }

    /// The panes are the desktop's, in the desktop's order.
    ///
    /// This drifted once already: the first version invented Device, Library,
    /// Accounts, Emulators, Controls and About, which is a settings screen you
    /// have to relearn per machine for no reason. Two additions: Device,
    /// because a laptop has no backlight you would set from an app, and Exit,
    /// because a laptop has a window with a close button on it.
    #[test]
    fn the_panes_match_the_desktops() {
        let ids: Vec<_> = built().iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "general",
                "appearance",
                "control",
                "library",
                "emulators",
                "iconsets",
                "device",
                // Not a desktop pane: a windowed app is closed with its window,
                // and a handheld has neither a window nor a visible way out.
                "exit",
                "about"
            ],
            "the handheld's settings no longer line up with the desktop's"
        );
    }

    /// The settings the desktop has that this device cannot honor are absent,
    /// and the ones it can are present.
    ///
    /// Named individually because "38 rows" is not a check: a pane can be the
    /// right length and hold the wrong things.
    #[test]
    fn the_desktops_settings_are_here_except_the_ones_about_windows() {
        let labels: Vec<String> = built()
            .iter()
            .flat_map(|p| p.entries.iter().map(|e| e.label.to_lowercase()))
            .collect();
        let has = |what: &str| labels.iter().any(|l| l.contains(what));

        for wanted in [
            "romm server",
            "retroachievements",
            "hardcore",
            "ask before deleting",
            "game list shows",
            "match player 1",
            "folder",
            "save state on exit",
            "auto-fire",
            "drawing from",
            "version",
        ] {
            assert!(has(wanted), "the desktop has {wanted:?} and this does not");
        }
        for absent in ["window", "title bar", "fit to the game"] {
            assert!(
                !has(absent),
                "{absent:?} means nothing on a device with no windows"
            );
        }
    }

    /// Refusing a rebind that would strand an essential action.
    ///
    /// The healing in `binds` means the pad can no longer be locked out, but a
    /// press that quietly puts the action somewhere else is its own confusion.
    /// The screen says no instead.
    #[test]
    fn a_rebind_that_would_strand_confirm_is_refused() {
        let mut b = moose_rack::binds::Bindings::default();
        // Confirm's button taken by a direction, and that direction's own
        // button cleared — Frank's config, as it happened.
        b.pad.insert("0".into(), "up".into());
        b.pad.insert("12".into(), String::new());
        assert_eq!(
            b.stranded(),
            Some("activate"),
            "the arrangement that locked the pad out was not reported"
        );

        assert_eq!(
            moose_rack::binds::Bindings::default().stranded(),
            None,
            "the defaults strand nothing"
        );
    }

    /// This front end never writes the desktop's file.
    ///
    /// On a development Mac both run from the same directory, and before the
    /// split a value stepped in the preview edited the real desktop config one
    /// press at a time with nothing said.
    #[test]
    fn settings_are_written_to_this_front_ends_own_file() {
        assert_ne!(FILE, "config.toml");
        assert!(
            FILE.ends_with(".toml"),
            "still a TOML file, just a different one"
        );
    }

    /// The writer refuses a field it cannot place, rather than writing it into
    /// some default table.
    #[test]
    fn the_writer_refuses_a_field_with_no_table() {
        assert!(write("nodot", &Written::Bool(true)).is_err());
    }
}

#[cfg(test)]
mod wiring {
    use super::*;

    /// A list of one is shown, not offered.
    ///
    /// It used to be a Choice: the row had a chevron, pressing A opened a sheet
    /// holding a single line, and picking it changed nothing. Three settings
    /// were in that state on a machine with no icon sets downloaded.
    #[test]
    fn a_choice_of_one_is_not_a_choice() {
        let one = choice(vec![("a".into(), "Only one".into())], "a");
        assert!(
            matches!(&one, Kind::ReadOnly(v) if v == "Only one"),
            "{one:?}"
        );
        let none = choice(Vec::new(), "");
        assert!(matches!(none, Kind::ReadOnly(_)), "{none:?}");

        let two = choice(
            vec![("a".into(), "A".into()), ("b".into(), "B".into())],
            "b",
        );
        assert!(
            matches!(two, Kind::Choice { at: 1, .. }),
            "two options stopped being a choice"
        );
    }

    /// Every setting that claims a config field is actually read back.
    ///
    /// This is the audit. A settings row is "wired" when changing it changes
    /// what the app reads, and the way to know that is to change it and read.
    /// The failure it catches is silent: `write` will happily put
    /// `library.folder = "x"` into the file whether or not `Config` has a
    /// `folder` in `[library]`, the row will show the new value until the
    /// screen is rebuilt, and then it reverts with no error anywhere.
    ///
    /// `device.*` is excluded because it is hardware rather than a file, and
    /// `bindings_pad.*` because it goes through `Bindings::set_pad` and has its
    /// own tests. Everything else round-trips.
    #[test]
    fn every_setting_that_names_a_field_is_read_back() {
        let dir = std::env::temp_dir().join("moose-rack-wiring");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("moose-sdl.toml").to_string_lossy().into_owned();
        std::fs::write(&file, "").unwrap();

        let consoles = vec![("snes".to_owned(), "Super Nintendo".to_owned())];
        let map = moose_rack::coremap::CoreMap::embedded();
        let load = || Config::load_from(std::path::Path::new(&file)).unwrap_or_default();

        let mut checked = 0;
        let mut unwired: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        // Indices rather than the entries themselves: the panes are rebuilt
        // after each write, so a borrow taken before it would be of the old
        // ones.
        let count = |cfg: &Config| {
            panes(cfg, &consoles, &map)
                .iter()
                .map(|p| p.entries.len())
                .sum::<usize>()
        };
        let total = count(&load());
        for i in 0..total {
            let cfg = load();
            let all: Vec<(String, Kind)> = panes(&cfg, &consoles, &map)
                .into_iter()
                .flat_map(|p| p.entries)
                .map(|e| (e.field.to_owned(), e.kind))
                .collect();
            let (field, kind) = all[i].clone();
            if field.is_empty() {
                skipped.push(format!("(no field) {kind:?}"));
                continue;
            }
            if field.starts_with("device.") || field.starts_with("bindings_pad.") {
                skipped.push(format!("{field} — hardware or binding, tested elsewhere"));
                continue;
            }
            // A value this setting is definitely not on right now.
            let want = match &kind {
                Kind::Toggle(on) => Written::Bool(!on),
                Kind::Number { value, min, max, .. } => {
                    Written::Number(if *value == *min { *max } else { *min })
                }
                Kind::Text { .. } => Written::Text("moose-rack-wiring-probe".to_owned()),
                Kind::Choice { at, options } if options.len() > 1 => {
                    let other = if *at == 0 { 1 } else { 0 };
                    Written::Text(options[other].0.clone())
                }
                // A choice of one and the rows that hold no value: nothing to
                // change, so nothing to check.
                _ => {
                    skipped.push(format!("{field} — nothing else to change it to"));
                    continue;
                }
            };
            write_to(&file, &field, &want).unwrap_or_else(|e| panic!("{field}: {e}"));

            let after = load();
            let now = panes(&after, &consoles, &map)
                .into_iter()
                .flat_map(|p| p.entries)
                .find(|e| e.field == field)
                .map(|e| e.kind);
            let stuck = match (&want, &now) {
                (Written::Bool(v), Some(Kind::Toggle(on))) => on != v,
                (Written::Number(v), Some(Kind::Number { value, .. })) => value != v,
                (Written::Text(v), Some(Kind::Text { value, .. })) => value != v,
                (Written::Text(v), Some(Kind::Choice { at, options })) => {
                    options.get(*at).map(|o| &o.0) != Some(v)
                }
                _ => true,
            };
            if stuck {
                unwired.push(field.clone());
            }
            checked += 1;
        }

        // What was passed over, and why, so the number that was actually
        // proved is visible rather than implied.
        eprintln!("wired and checked: {checked}");
        eprintln!("not checked: {skipped:#?}");
        assert!(
            unwired.is_empty(),
            "{} of {checked} settings write somewhere nothing reads: {unwired:#?}",
            unwired.len()
        );
        assert!(checked > 20, "only {checked} settings were checked");
    }
}

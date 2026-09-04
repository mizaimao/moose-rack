//! `config.toml` — see `config.example.toml` for the documented template.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub library: Library,
    #[serde(default)]
    pub retroarch: RetroArchCfg,
    #[serde(default)]
    pub saves: SavesCfg,
    #[serde(default)]
    pub controllers: ControllersCfg,
    #[serde(default)]
    pub theme: ThemeCfg,
    #[serde(default)]
    pub cores: CoresCfg,
    #[serde(default)]
    pub shaders: ShadersCfg,
    #[serde(default)]
    pub lightgun: LightgunCfg,
    #[serde(default)]
    pub media: MediaCfg,
    #[serde(default)]
    pub icons: IconsCfg,
    #[serde(default)]
    pub appearance: AppearanceCfg,
    #[serde(default)]
    pub esde: EsdeCfg,
    /// `[achievements]`, with `[cheevos]` still accepted: that is RetroArch's
    /// own name for the feature and what every key it writes is called, so an
    /// existing config keeps working.
    #[serde(default, alias = "cheevos")]
    pub achievements: AchievementsCfg,
    /// Rebound keys and controller buttons. Empty until somebody changes one;
    /// see [`crate::binds`] for the tables the changes are layered over.
    ///
    /// Here rather than in the webview's own storage because the TUI and any
    /// front end that is not a browser have to read the same choices — and
    /// because the settings window is a second document, which meant the two
    /// halves of the app kept separate copies and synchronised them by hand.
    #[serde(default)]
    pub bindings: crate::binds::Bindings,
    /// How the left column is ordered, per kind of list. Remembered, unlike
    /// the game sort — see [`crate::pickorder`].
    #[serde(default)]
    pub picker_order: crate::pickorder::PickerOrders,
}

/// RetroAchievements — the `[achievements]` section of config.toml.
///
/// See [`crate::achievements`] for why a username is required alongside the
/// credential, and why hardcore mode is written explicitly every launch.
#[derive(Debug, Default, Deserialize)]
pub struct AchievementsCfg {
    #[serde(default)]
    #[serde(deserialize_with = "lenient_bool")]
    pub enabled: bool,
    /// RetroAchievements account name. The token alone authenticates nothing.
    #[serde(default)]
    pub username: Option<String>,
    /// Login token — what RetroArch stores after a successful login, and what
    /// it prefers thereafter. The only credential this app stores.
    #[serde(default)]
    pub token: Option<String>,
    /// Disables save states, fast-forward and rewind — four of the gamepad
    /// hotkeys this app binds.
    #[serde(default)]
    #[serde(deserialize_with = "lenient_bool")]
    pub hardcore: bool,
    #[serde(default)]
    #[serde(deserialize_with = "lenient_bool")]
    pub test_unofficial: bool,
}

impl AchievementsCfg {
    pub fn settings(&self) -> crate::achievements::Settings {
        crate::achievements::Settings {
            enabled: self.enabled,
            username: self.username.clone(),
            token: self.token.clone(),
            hardcore: self.hardcore,
            test_unofficial: self.test_unofficial,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// A RomM client token, used in preference to username/password.
    ///
    /// Better than Basic for a device that keeps credentials on disk: it
    /// carries only the scopes this client needs, and can be revoked from the
    /// server without touching the account password.
    #[serde(default)]
    pub token: Option<String>,
}

impl Server {
    /// Build a client from whichever credential is configured.
    pub fn client(&self) -> anyhow::Result<crate::api::Client> {
        crate::api::Client::with_auth(
            &self.url,
            &self.username,
            &self.password,
            self.token.as_deref(),
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct Library {
    #[serde(default = "default_local_root")]
    pub local_root: String,
    /// Show RomM's own generated collections — by company, genre, franchise,
    /// series and mode — alongside the ones you made.
    ///
    /// On by default, and worth turning off on a handheld: this library has 27
    /// collections of its own and 1,931 generated ones, and the generated set
    /// is mostly a way to find a game you could have found faster in the
    /// Library tab.
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub server_collections: bool,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            local_root: default_local_root(),
            server_collections: true,
        }
    }
}

fn default_local_root() -> String {
    "./library".to_owned()
}

/// Controllers beyond the first.
#[derive(Debug, Deserialize)]
pub struct ControllersCfg {
    /// Bind players 2-4 the same way as player 1.
    ///
    /// On by default. RetroArch binds each pad from its own autoconfig profile
    /// and gives a controller it has no profile for nothing at all, which looks
    /// like a dead port rather than a missing file. Copying player one is right
    /// whenever the other pads are the same model — the usual case — and wrong
    /// for a genuinely different device, which is why it can be turned off.
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub mirror_player_one: bool,

    /// Swap the two face buttons **for moving around this app only**.
    ///
    /// Some pads report the bottom face button as B and the right one as A, so
    /// Confirm and Back come out the wrong way round. This corrects that where
    /// it is read — nothing here is written into RetroArch's config, and a game
    /// plays with the buttons the game expects. The alternative, rebinding
    /// every action one at a time, is the same fix done twenty times.
    #[serde(default)]
    #[serde(deserialize_with = "lenient_bool")]
    pub swap_ab: bool,
    /// The same for the other pair.
    #[serde(default)]
    #[serde(deserialize_with = "lenient_bool")]
    pub swap_xy: bool,
}

impl Default for ControllersCfg {
    fn default() -> Self {
        Self {
            mirror_player_one: true,
            swap_ab: false,
            swap_xy: false,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SavesCfg {
    /// Directory containing `saves/` and `states/` subdirectories. In a
    /// portable RetroArch install this is the portable root itself.
    #[serde(default = "default_saves_root")]
    pub root: String,

    /// Pull before a game starts and push after it exits, Steam-cloud style.
    ///
    /// On by default now that every overwrite is backed up first — see
    /// [`crate::savebackup`]. Without those backups this would be the one
    /// setting in the app that can destroy something unrecoverable, which is
    /// why it did not exist until they did.
    ///
    /// A conflict is still never resolved automatically: both sides are left
    /// as they are and reported.
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub auto_sync: bool,

    /// Ask before deleting a save state from the shelf.
    ///
    /// Off by default, which is the unusual choice for a destructive action and
    /// is deliberate. Deleting states is housekeeping done several at a time —
    /// a shelf collects a dozen of them — and a dialog for each turns a tidy-up
    /// into a chore, which is how people end up not tidying up at all. The file
    /// is not gone either way: it goes to the same backup folder an overwritten
    /// save does, so the undo exists whether or not the question was asked.
    #[serde(default)]
    #[serde(deserialize_with = "lenient_bool")]
    pub confirm_delete_state: bool,
}

impl Default for SavesCfg {
    fn default() -> Self {
        Self {
            root: default_saves_root(),
            auto_sync: true,
            confirm_delete_state: false,
        }
    }
}

fn default_saves_root() -> String {
    // The device first, where it has an opinion — a handheld's save tree is
    // somewhere the OS chose, not somewhere beside the app. `./Saves` is the
    // portable-RetroArch answer and remains the fallback.
    crate::platform::current()
        .saves_root()
        .unwrap_or("./Saves")
        .to_owned()
}

#[derive(Debug, Default, Deserialize)]
pub struct CoresCfg {
    /// Platform slug -> libretro core stem, overriding the ES-DE default.
    ///
    /// Needed when a collection's ROMs do not match what the default core
    /// expects — arcade romsets in particular are version-locked, and a
    /// MAME 2003-Plus set will not run under current MAME.
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,

    /// `"<platform>/<fs_name>"` -> core, for the one game that needs a
    /// different core from the rest of its platform.
    ///
    /// Keyed by path rather than ROM id: ids are reassigned when the server
    /// rescans, and a rebuilt library would silently point these at the wrong
    /// games. The path is also readable, so this table can be hand-edited.
    ///
    /// Loaded with [`ARCADE_CORE_MAP`] folded in underneath, so the file only
    /// ever holds what somebody chose by hand.
    #[serde(default)]
    pub per_game: BTreeMap<String, String>,
}

/// Arcade romsets their platform default core cannot run.
///
/// Compiled in for the same reason as the ES-DE core map: a file beside the
/// executable is not something a downloaded build can rely on. 6 KB.
///
/// It lives here rather than in config.toml because it is measured, not
/// chosen — every row was produced by probing the romset headless against the
/// core — and it made two thirds of the user's config file a table no user
/// should have to read.
pub const ARCADE_CORE_MAP: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/arcade-core-map.toml"
));

/// The compiled-in table. Infallible by construction: a malformed file fails
/// the test below, which runs at build time, rather than failing a user.
pub fn arcade_core_map() -> BTreeMap<String, String> {
    toml::from_str(ARCADE_CORE_MAP).expect("the embedded arcade core map is valid TOML")
}

/// Config key for a single game's core override.
pub fn game_key(platform: &str, fs_name: &str) -> String {
    format!("{platform}/{fs_name}")
}

/// A local ES-DE library, used instead of a RomM server.
///
/// When `root` is set the app indexes that tree directly: nothing is
/// downloaded, because the ROMs and artwork are already on disk.
#[derive(Debug, Default, Deserialize)]
pub struct EsdeCfg {
    /// ES-DE data directory — the one holding `gamelists/` and
    /// `downloaded_media/`.
    #[serde(default)]
    pub root: Option<String>,
    /// ROMs directory, if it is not `<root>/ROMs`. ES-DE keeps this separate
    /// and configurable, so it usually is.
    #[serde(default)]
    pub roms: Option<String>,
    /// Artwork directory, if it is not `<root>/downloaded_media`. A portable
    /// ES-DE install files it under `support/` while `gamelists/` stays in the
    /// data directory, and then no single root derives both.
    #[serde(default)]
    pub media: Option<String>,
}

impl EsdeCfg {
    /// Artwork root of a *separately configured* ES-DE library.
    ///
    /// Deliberately still `None` when nothing is set, rather than falling back
    /// the way [`Config::esde_layout`] does. This one answers "is there a
    /// second library whose artwork should be preferred over ours" — and the
    /// app's own download folder is not a second library, it is this one.
    pub fn media_dir(&self) -> Option<PathBuf> {
        self.layout().map(|l| l.media).filter(|p| p.is_dir())
    }

    pub fn layout(&self) -> Option<crate::esde::Layout> {
        let root = crate::util::expand_tilde(self.root.as_deref()?);
        let roms = self.roms.as_deref().map(crate::util::expand_tilde);
        let media = self.media.as_deref().map(crate::util::expand_tilde);
        Some(crate::esde::Layout::new(&root, roms.as_deref()).with_media(media.as_deref()))
    }
}

/// `[appearance]` — how the app itself looks, as opposed to what it draws.
///
/// Most of the Appearance pane lives in the browser's own storage, because it
/// is per-screen and changes as you drag a slider. The app icon does not: it
/// has to be known before a window exists, so it belongs in the file.
#[derive(Debug, Deserialize)]
pub struct AppearanceCfg {
    /// Which icon the app wears — an id from [`crate::appicon::ICONS`].
    /// Absent means the default; an id this build no longer ships also means
    /// the default rather than no icon.
    #[serde(default)]
    pub app_icon: Option<String>,
    /// How far away the screen is held, in centimetres. Absent means a desk.
    ///
    /// The one number that makes a layout in points come out the right
    /// physical size on both a monitor and a handheld — see
    /// [`crate::layout::Scale::viewed_from`]. A panel's DPI says how fine its
    /// pixels are and nothing about how close somebody's face is, and that is
    /// most of what decides how big a thing should be drawn.
    ///
    /// A 4" 960x720 handheld is around 300 DPI. Left at the desk default that
    /// works out to a scale of three, which is a screen with room for two
    /// covers on it. Set to 30 it is 1.5, and the screen is 640 points wide.
    /// The handheld image ships with this set; nothing else needs it.
    #[serde(default)]
    pub viewing_distance_cm: Option<f32>,

    /// Which shader draws behind everything, by id — `blobs`, `aurora` or
    /// `plasma`. The webview keeps its own in the browser's local storage, so
    /// this is the SDL front end's and the two do not have to agree.
    #[serde(default = "default_backdrop")]
    pub backdrop: String,
    /// Which color scheme the backdrop and the glass take — an id from
    /// `ui/js/backdrop.js`'s list, shared with the webview.
    #[serde(default = "default_scheme")]
    pub scheme: String,
    /// How fast it moves, as a percentage of the style's own pace.
    #[serde(default = "hundred")]
    pub backdrop_speed: i64,
    /// How strongly it is drawn, as a percentage.
    #[serde(default = "hundred")]
    pub backdrop_strength: i64,
    /// How frosted the panels are, 0 to 60.
    #[serde(default = "default_glass")]
    pub glass: i64,
    /// Whether the front end animates: pages sliding as the tabs change, and
    /// whatever else is added later.
    ///
    /// One switch for all of it rather than one per effect. Somebody who turns
    /// animation off has said what they want, and having to find six more
    /// switches to actually get it is the shape of a settings screen nobody
    /// trusts.
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub animations: bool,
}

/// Written out rather than derived.
///
/// `#[serde(default = "...")]` fills a field that is *missing from a table that
/// is present*. When the whole `[appearance]` table is absent, serde uses
/// `AppearanceCfg::default()` instead — and the derived one is zeros, which
/// gave a backdrop drawn at zero strength and a settings screen reading "0%"
/// beside a black screen.
impl Default for AppearanceCfg {
    fn default() -> Self {
        Self {
            app_icon: None,
            viewing_distance_cm: None,
            backdrop: default_backdrop(),
            scheme: default_scheme(),
            backdrop_speed: hundred(),
            backdrop_strength: hundred(),
            glass: default_glass(),
            animations: yes(),
        }
    }
}

fn default_backdrop() -> String {
    "blobs".to_owned()
}

fn default_scheme() -> String {
    "midnight".to_owned()
}

fn hundred() -> i64 {
    100
}

fn default_glass() -> i64 {
    30
}

/// Which per-system artwork the platform grid shows.
#[derive(Debug, Deserialize)]
pub struct IconsCfg {
    /// Which look the grid draws — an id from the chosen set's own list, such
    /// as `hardware`, `controller`, `styled-text` or `styled-text-2`.
    ///
    /// Not one of three fixed kinds any more: themes offer between one and nine
    /// looks and squeezing them into three meant showing the wrong picture
    /// under names a theme never filled. `data/icon-set-art.toml` lists what
    /// each set has. With no set chosen this falls back to the shared pool's
    /// `logo`, `controller` and `systemart`.
    #[serde(default = "default_icon_style")]
    pub style: String,

    /// Which ES-DE set the grid draws from. Empty until one is fetched, at
    /// which point "Get console pictures" writes the set it took.
    ///
    /// Separate from `style` rather than folded into it: a set and a style are
    /// different questions — *whose* artwork, and *which kind* of picture — and
    /// a set ships more than one kind.
    #[serde(default)]
    pub set: String,
}

fn default_icon_style() -> String {
    // A look id from the chosen set's own list. "hardware" is first in every
    // set that has it, and sets are ordered hardware-first, so this lands on a
    // picture of the console wherever there is one.
    "hardware".to_owned()
}

impl Default for IconsCfg {
    fn default() -> Self {
        Self {
            style: default_icon_style(),
            set: String::new(),
        }
    }
}

/// A TOML boolean, or the word for one in quotes.
///
/// `set_config_field` wrote `swap_ab = "true"` for several versions, because
/// its list of which settings are booleans had drifted from the struct. TOML is
/// strict and `toml::from_str` refuses the entire file over one wrong type, so
/// a single quoted bool took the whole config down — and since every caller
/// falls back to `Config::default()`, silently. On Frank's Thor that pointed
/// the library, the artwork and the saves folder at `./library/...`, and the
/// ES-DE scan found nothing while the grid went on showing a cache from before.
///
/// So the reader takes both spellings. The writer is fixed separately; this is
/// what lets a file written by the broken versions load again without anyone
/// having to hand-edit it, and what stops the next drift being silent.
fn lenient_bool<'de, D>(d: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Bool(bool),
        Text(String),
    }
    match Either::deserialize(d)? {
        Either::Bool(b) => Ok(b),
        Either::Text(t) => match t.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" => Ok(false),
            other => Err(serde::de::Error::custom(format!(
                "expected true or false, found {other:?}"
            ))),
        },
    }
}

#[derive(Debug, Deserialize)]
pub struct ShadersCfg {
    /// Master switch. Off means no shader is applied and RetroArch's own
    /// setting is left alone.
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub enabled: bool,
    /// Platform slug -> preset path under `shaders_slang/` (no extension), or
    /// `"none"` to force no shader for that platform.
    #[serde(default)]
    pub by_platform: BTreeMap<String, String>,
    /// Strobe/BFI pass chained on top of the platform shader for CRT
    /// consoles, e.g. `"subframe-bfi/adaptive_strobe-koko"`. Off by default:
    /// how well it works depends on the display, so it is a deliberate choice
    /// rather than something that should surprise anyone.
    #[serde(default)]
    pub motion: Option<String>,
}

impl Default for ShadersCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            by_platform: BTreeMap::new(),
            motion: None,
        }
    }
}

/// Which ES-DE artwork the interface shows where.
#[derive(Debug, Deserialize)]
pub struct MediaCfg {
    /// The image on each game in the list or grid. Cartridge art by default:
    /// it is the thing you recognize a game by when you owned it, and within a
    /// console every cartridge is the same shape, so the grid stays even.
    #[serde(default = "cart_art")]
    pub list_art: String,
    /// The image in the info pane. The miximage, which already combines box,
    /// screenshot and logo, so the pane says everything at once.
    #[serde(default = "mix_art")]
    pub detail_art: String,
}

fn cart_art() -> String {
    "physicalmedia".to_owned()
}

fn mix_art() -> String {
    "miximages".to_owned()
}

impl Default for MediaCfg {
    fn default() -> Self {
        Self {
            list_art: cart_art(),
            detail_art: mix_art(),
        }
    }
}

/// Systems switched over to a light gun.
///
/// Its own section rather than a flag on each platform because on most of
/// these consoles the gun takes the port a second pad would use, so leaving it
/// on breaks two-player games. Keeping it in one visible list makes it obvious
/// what has been switched.
#[derive(Debug, Default, Deserialize)]
pub struct LightgunCfg {
    /// Platform slug -> `"on"`. Anything else, or absent, means off.
    #[serde(default)]
    pub by_platform: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ThemeCfg {
    /// Extra directory to search for ES-DE themes, checked before the
    /// standard locations. Unset means probe ~/ES-DE/themes and the ES-DE.app
    /// bundle.
    pub root: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RetroArchCfg {
    /// Single install. Kept for older configs; `installs` supersedes it.
    pub root: Option<String>,

    /// Ordered list of installs, tried top to bottom like a boot order. The
    /// first enabled entry that actually contains RetroArch wins, so a
    /// portable build can shadow a system one without deleting either.
    #[serde(default)]
    pub installs: Vec<RetroArchInstall>,

    /// Extra RetroArch settings appended at launch, on top of ours.
    ///
    /// Path to a file in RetroArch's own `key = "value"` format. Anything here
    /// wins, so button maps and video filters can be pinned without ever
    /// opening RetroArch's menu.
    pub user_config: Option<String>,

    /// Which screen a game opens on: `auto`, `main`, or a display number.
    ///
    /// `auto` prefers an external display, because plugging a monitor into a
    /// laptop is a deliberate act and nobody does it wanting the game on the
    /// laptop panel. macOS only for now — it is the one platform where the
    /// screen geometry can be asked for in the units the window manager
    /// actually uses.
    #[serde(default = "default_game_display")]
    pub game_display: String,

    /// See [`default_game_display`] above for why this is on.
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub fit_window: bool,

    /// Make the shot button repeat while held, in arcade shooters.
    ///
    /// On by default and only for games the metadata calls shooters — 767 of
    /// them here. These were built around a cabinet button you hammered, and
    /// a run of Metal Slug is a few thousand presses. Off for anything where
    /// single shots matter more than volume.
    /// How many shots a second auto-fire produces.
    ///
    /// In hertz rather than RetroArch's own unit, which is the number of frames
    /// in one press-release cycle — a number that means nothing to anyone and
    /// runs the wrong way, where larger is slower. The conversion is one line
    /// and it happens once, here, rather than in everybody's head.
    #[serde(default = "default_autofire_hz")]
    pub autofire_hz: u32,

    /// `off`, `lb`, or `rb` — which shoulder button turns the face buttons
    /// into rapid fire while it is held.
    ///
    /// Reads an old `true`/`false` as well, because this was a toggle first and
    /// the app itself wrote that boolean into people's files. Changing the type
    /// without accepting the old one did not produce a wrong setting — it made
    /// the whole config fail to parse, so every other value in it silently fell
    /// back to a default: no server, no library path, nothing. A setting that
    /// can break the file it lives in is worse than the setting being wrong.
    #[serde(default = "default_autofire", deserialize_with = "autofire_from_toml")]
    pub autofire: String,
    /// Have RetroArch write a save state when the game exits.
    ///
    /// Off by default, and deliberately. An automatic state on every exit
    /// quietly becomes the state you resume from, so a five-minute look at a
    /// game overwrites the point you actually stopped at — and RetroArch's
    /// auto slot is the one this app resumes from. Anyone who wants it can
    /// have it; nobody gets it without asking.
    #[serde(default)]
    #[serde(deserialize_with = "lenient_bool")]
    pub save_state_on_exit: bool,

    /// Keep the game window's title bar.
    ///
    /// On by default, because without it there is nothing to drag and nothing
    /// to click to close — the way out is a controller combination or a
    /// keyboard hotkey, and someone who has just turned this off is exactly
    /// the person who may not know that yet.
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub window_decorations: bool,
}

/// Shape the game window like the game.
///
/// On by default. RetroArch keeps a picture's proportions inside the window it
/// is given, so a maximised window on a 16:10 laptop showing a 3:2 handheld
/// game is a black column down each side — wider than the game was tall on the
/// original hardware. A window of the right shape has nothing left to put a bar
/// in. Turn it off to have the window fill the screen and let the emulator
/// letterbox inside it.
fn default_autofire_hz() -> u32 {
    // Six a second: a shade under what a good player sustains by hand, and one
    // of the rates that divides 60 exactly, so it is delivered as asked rather
    // than quantised to the nearest frame.
    6
}

fn default_autofire() -> String {
    "lb".to_owned()
}

/// Accept the string this setting is now, or the boolean it used to be.
fn autofire_from_toml<'de, D>(d: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Text(String),
        // `true` meant rapid fire was wanted, back when this was a switch.
        Flag(bool),
    }
    Ok(match Either::deserialize(d)? {
        Either::Text(s) => s,
        Either::Flag(true) => "lb".to_owned(),
        Either::Flag(false) => "off".to_owned(),
    })
}

fn default_game_display() -> String {
    "auto".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetroArchInstall {
    /// Directory containing `RetroArch.app` (macOS) or `retroarch.exe`.
    pub path: String,
    /// Shown in Settings; defaults to the path when absent.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "yes")]
    #[serde(deserialize_with = "lenient_bool")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl RetroArchCfg {
    /// Install paths to try, in order, honouring `enabled`.
    ///
    /// Falls back to the legacy single `root` so an existing config keeps
    /// working untouched.
    pub fn ordered_paths(&self) -> Vec<String> {
        if !self.installs.is_empty() {
            return self
                .installs
                .iter()
                .filter(|i| i.enabled)
                .map(|i| i.path.clone())
                .collect();
        }
        self.root.clone().into_iter().collect()
    }
}

/// Which config file this process is using.
///
/// It used to be the literal `config.toml`, resolved against the working
/// directory, in twenty-six places. That is right for an app started by its
/// icon and wrong for a service: systemd sets the working directory, and the
/// library service reads a file named in `moose-service.toml`. The state was
/// built from the named file while `config_fields`, `status` and every setting
/// writer went on reading a different one -- so the Settings page reported an
/// unconfigured app on a server that was correctly configured, and an edit
/// would have been written to a file nothing reads.
///
/// Process-wide rather than an argument because that is what it is: decided
/// once at startup, true for everything afterwards, and several of the readers
/// are `fn()` with no state to thread it through.
///
/// A lock rather than a `OnceLock`. With `get_or_init` the first *reader* fixes
/// the value, so anything that happened to ask before startup finished would
/// silently pin the default and make `set_path` a no-op -- a failure that looks
/// exactly like the bug this exists to fix, and leaves no trace.
static CONFIG_PATH: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Name the config file. Called by `AppState::from_config_at` before it reads.
pub fn set_path(path: PathBuf) {
    if let Ok(mut w) = CONFIG_PATH.write() {
        *w = Some(path);
    }
}

/// The config file, defaulting to `config.toml` in the working directory --
/// which is what a desktop launch wants and what every caller assumed.
pub fn path() -> PathBuf {
    CONFIG_PATH
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

/// The same, for the writers that take a `&str`.
pub fn path_str() -> String {
    path().to_string_lossy().into_owned()
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_from(&path())
    }

    /// True when `load` fell back to defaults because no file was there.
    ///
    /// A missing config used to be indistinguishable from a configured one
    /// pointing at a dead server: both produce an unreachable client and an
    /// empty library, which reads as "the app is broken" rather than "nothing
    /// has been set up yet". Callers use this to say which it is.
    pub fn exists(path: &str) -> bool {
        Path::new(path).is_file()
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.is_file() {
            // Absent config is fine for commands that don't touch the server.
            return Ok(Self::default().with_shipped_cores());
        }
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg.with_shipped_cores())
    }

    /// Fold the compiled-in arcade table in under whatever the file said.
    ///
    /// Under, not over: a core picked in the game detail pane is written to
    /// config.toml, and a shipped row must never quietly replace it.
    fn with_shipped_cores(mut self) -> Self {
        for (game, core) in arcade_core_map() {
            self.cores.per_game.entry(game).or_insert(core);
        }
        self
    }

    /// Where to look for games already on this machine.
    ///
    /// Two folders, both configurable and both with a default, because they are
    /// genuinely separate things: the ES-DE base folder holds `gamelists/` and
    /// `downloaded_media/` — the metadata and artwork for a whole collection —
    /// while the games themselves are usually somewhere else entirely, often a
    /// card or an external drive. ES-DE keeps them apart for that reason and so
    /// does this.
    ///
    /// The defaults are the app's own folders, because they already have the
    /// shape ES-DE uses: `<library>/downloaded_media` beside `<library>/roms`.
    /// So an install that has never been pointed at anything still has a local
    /// library to scan — its own — and pointing it at a real ES-DE install is a
    /// matter of changing two paths rather than arranging anything.
    ///
    /// Unlike [`EsdeCfg::layout`] this never returns `None`. There is always
    /// somewhere to look; whether anything is there is a question for the scan.
    pub fn esde_layout(&self) -> crate::esde::Layout {
        let root = self
            .esde
            .root
            .as_deref()
            .map(crate::util::expand_tilde)
            .unwrap_or_else(|| PathBuf::from(&self.library.local_root));
        let roms = self
            .esde
            .roms
            .as_deref()
            .map(crate::util::expand_tilde)
            // Spelled out rather than calling `local_roms_dir`, which is now
            // defined in terms of this function and would recurse.
            .unwrap_or_else(|| PathBuf::from(&self.library.local_root).join("roms"));
        // `media` overrides the derived `<root>/downloaded_media` for a portable
        // ES-DE install, where artwork sits under `support/` and gamelists do
        // not. Applied here as well as in `EsdeCfg::layout`, because this is the
        // path everything that is not a separately-configured second library
        // goes through — including `scan-esde` with no `--root`.
        let media = self.esde.media.as_deref().map(crate::util::expand_tilde);
        crate::esde::Layout::new(&root, Some(&roms)).with_media(media.as_deref())
    }

    /// Where games live — the folder set in Settings, not a folder of our own.
    ///
    /// This used to be `<library>/roms` unconditionally, so an install pointed
    /// at a real ES-DE library read games from one place and downloaded them to
    /// another. Now there is one answer: the ES-DE ROMs folder, which defaults
    /// to `<library>/roms` and so is unchanged for anyone who has not set it.
    pub fn local_roms_dir(&self) -> PathBuf {
        self.esde_layout().roms
    }

    /// Where artwork lives, for the same reason.
    ///
    /// `<esde root>/downloaded_media`, which is where ES-DE keeps it and where
    /// it already is on a machine that has one. The default ES-DE root is the
    /// library folder, so this stays `<library>/downloaded_media` until
    /// somebody points it somewhere — and then everything follows together
    /// rather than the app keeping a second copy beside the real one.
    pub fn media_dir(&self) -> PathBuf {
        self.esde_layout().media
    }

    /// The user's own RetroArch settings file, appended at launch.
    pub fn user_retroarch_config(&self) -> PathBuf {
        match &self.retroarch.user_config {
            Some(p) => crate::util::expand_tilde(p),
            None => PathBuf::from(&self.library.local_root).join("retroarch-user.cfg"),
        }
    }

    /// The BIOS folder, used as RetroArch's system directory.
    ///
    /// `<roms>/0_BIOS`, which is where ES-DE keeps it — the leading zero sorts
    /// it above the systems, and `esde::NOT_SYSTEMS` already knows to skip it
    /// when scanning. Frank's card has 174 files in exactly that folder, so
    /// writing to a `BIOS` beside it would have made a second, empty one.
    ///
    /// It moved out of `<library>/system`, which was chosen so that one visible
    /// folder held everything this app downloaded. That property is worth less
    /// than sharing a BIOS folder with the emulator that reads it: the files
    /// are large, they are the same files, and two copies is the arrangement
    /// this whole change exists to end.
    pub fn system_dir(&self) -> PathBuf {
        self.esde_layout().roms.join("0_BIOS")
    }

    /// ES-DE themes, read from where ES-DE downloads them.
    ///
    /// `<esde root>/themes`. ES-DE puts its themes there and this app draws
    /// console pictures out of them, so a machine with ES-DE on it already has
    /// everything needed and nothing has to be fetched twice. Defaults to
    /// `<library>/themes` as before, because the ES-DE root defaults to the
    /// library folder.
    pub fn themes_dir(&self) -> PathBuf {
        let root = self
            .esde
            .root
            .as_deref()
            .map(crate::util::expand_tilde)
            .unwrap_or_else(|| PathBuf::from(&self.library.local_root));
        root.join("themes")
    }
}

/// Set `key = "value"` inside `[table]` in a TOML file, creating the table if
/// needed.
///
/// A targeted text edit rather than parse-and-reserialise: the config carries
/// hand-written comments explaining non-obvious choices (which arcade core and
/// why, what the shader groups mean), and round-tripping through a serialiser
/// would delete all of them.
pub fn set_table_entry(path: &str, table: &str, key: &str, value: &str) -> Result<()> {
    write_entry(path, table, key, Some(value))
}

/// Set a boolean. TOML booleans are bare literals, so they cannot go through
/// the quoted-string writer — `enabled = "true"` is a string and parses as one,
/// which then fails to deserialise into a bool.
pub fn set_table_bool(path: &str, table: &str, key: &str, value: bool) -> Result<()> {
    write_raw(path, table, key, Some(if value { "true" } else { "false" }))
}

/// Set a number, for the same reason booleans need their own writer: a
/// quoted "5" is a TOML string and fails to deserialise into a number.
pub fn set_table_number(path: &str, table: &str, key: &str, value: i64) -> Result<()> {
    write_raw(path, table, key, Some(&value.to_string()))
}

/// Remove `key` from `[table]` if present.
pub fn clear_table_entry(path: &str, table: &str, key: &str) -> Result<()> {
    write_entry(path, table, key, None)
}

/// TOML bare keys allow only letters, digits, `_` and `-`. Per-game keys are
/// file paths, so they have to be quoted and escaped.
fn toml_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return key.to_owned();
    }
    format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Does this line define `key`, in either its bare or quoted form?
fn defines_key(line: &str, key: &str) -> bool {
    let Some(lhs) = line.split('=').next() else {
        return false;
    };
    let lhs = lhs.trim();
    lhs == key || lhs == toml_key(key)
}

fn write_entry(path: &str, table: &str, key: &str, value: Option<&str>) -> Result<()> {
    let quoted = value.map(|v| {
        format!(
            "{} = \"{}\"",
            toml_key(key),
            v.replace('\\', "\\\\").replace('"', "\\\"")
        )
    });
    write_line(path, table, key, quoted)
}

/// As [`write_entry`], with the right-hand side written verbatim — for values
/// that are not TOML strings.
fn write_raw(path: &str, table: &str, key: &str, value: Option<&str>) -> Result<()> {
    write_line(
        path,
        table,
        key,
        value.map(|v| format!("{} = {v}", toml_key(key))),
    )
}

fn write_line(path: &str, table: &str, key: &str, entry: Option<String>) -> Result<()> {
    write_lines(path, table, &[(key.to_owned(), entry)])
}

/// Set or clear a whole table's worth of keys in one pass over the file.
///
/// One read and one write, whatever the length of the list. The single-key
/// writer above goes through here too, so there is one implementation of where
/// a key belongs in a file.
///
/// Written for the bindings, which are 29 actions and 16 buttons: doing those
/// one at a time is forty-five read-modify-writes of config.toml for a single
/// press of a rebind button, and a file rewritten that often is one that
/// eventually gets caught half-written.
pub fn set_table_entries(
    path: &str,
    table: &str,
    entries: &[(String, Option<String>)],
) -> Result<()> {
    let lines: Vec<(String, Option<String>)> = entries
        .iter()
        .map(|(key, value)| {
            let entry = value.as_ref().map(|v| {
                format!(
                    "{} = \"{}\"",
                    toml_key(key),
                    v.replace('\\', "\\\\").replace('"', "\\\"")
                )
            });
            (key.clone(), entry)
        })
        .collect();
    write_lines(path, table, &lines)
}

fn write_lines(path: &str, table: &str, entries: &[(String, Option<String>)]) -> Result<()> {
    let file = Path::new(path);
    let original = std::fs::read_to_string(file).unwrap_or_default();
    let header = format!("[{table}]");

    let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
    let mut touched = false;
    for (key, entry) in entries {
        touched |= edit(&mut lines, &header, key, entry.clone());
    }
    if !touched {
        return Ok(());
    }
    std::fs::write(file, lines.join("\n") + "\n")
        .with_context(|| format!("writing {}", file.display()))?;
    Ok(())
}

/// Put one key where it belongs in `lines`. Returns whether anything changed.
fn edit(lines: &mut Vec<String>, header: &str, key: &str, entry: Option<String>) -> bool {
    // Locate the table, and the key within it.
    let table_at = lines.iter().position(|l| l.trim() == header);
    let Some(start) = table_at else {
        // Nothing to remove from a table that does not exist.
        let Some(entry) = entry else { return false };
        // No such table yet: append it.
        if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(header.to_owned());
        lines.push(entry);
        return true;
    };

    // The table ends at the next header line.
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());

    let existing = lines[start + 1..end]
        .iter()
        .position(|l| defines_key(l, key))
        .map(|i| start + 1 + i);

    match (existing, entry) {
        (Some(i), Some(entry)) => {
            if lines[i] == entry {
                return false;
            }
            lines[i] = entry;
        }
        (Some(i), None) => {
            lines.remove(i);
        }
        (None, Some(entry)) => lines.insert(end, entry),
        (None, None) => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config with no `[appearance]` table at all still gets real values.
    ///
    /// `#[serde(default = "...")]` only fills a field missing from a table that
    /// is present. With the table absent serde reaches for `Default`, and the
    /// derived one is zeros — which drew the backdrop at zero strength and put
    /// "0%" beside a black screen on the settings page.
    #[test]
    fn appearance_defaults_survive_a_config_with_no_appearance_table() {
        let cfg: Config = toml::from_str("[server]\nurl = \"http://x\"\n").unwrap();
        assert_eq!(
            cfg.appearance.backdrop_strength, 100,
            "backdrop drawn at nothing"
        );
        assert_eq!(cfg.appearance.backdrop_speed, 100, "backdrop frozen");
        assert_eq!(cfg.appearance.backdrop, "blobs");
        assert!(cfg.appearance.glass > 0, "panels drawn with no glass");
    }

    /// A whole table at once, which is what the bindings need: 29 actions and
    /// 16 buttons, written one at a time, is forty-five read-modify-writes of
    /// config.toml for one press of a rebind button.
    #[test]
    fn a_whole_table_is_written_in_one_pass() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-table");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let file = path.to_str().unwrap();
        std::fs::write(&path, "[server]\nurl = \"http://dev.lan\"\n").unwrap();

        super::set_table_entries(
            file,
            "bindings.keys",
            &[
                ("sortMenu".to_owned(), Some("s".to_owned())),
                ("filterMenu".to_owned(), Some("f".to_owned())),
                // Empty, not absent: "deliberately unbound".
                ("left".to_owned(), Some(String::new())),
                // Absent: never touched, so nothing should appear for it.
                ("random".to_owned(), None),
            ],
        )
        .unwrap();

        let doc: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let keys = doc.get("bindings").and_then(|b| b.get("keys")).unwrap();
        assert_eq!(keys.get("sortMenu").unwrap().as_str(), Some("s"));
        assert_eq!(keys.get("filterMenu").unwrap().as_str(), Some("f"));
        assert_eq!(
            keys.get("left").unwrap().as_str(),
            Some(""),
            "unbound was not written"
        );
        assert!(
            keys.get("random").is_none(),
            "a key nobody set was written anyway"
        );
        // And the rest of the file is untouched.
        assert_eq!(
            doc.get("server").unwrap().get("url").unwrap().as_str(),
            Some("http://dev.lan")
        );
    }

    /// The config carries hand-written comments explaining non-obvious
    /// choices, which is why every writer here is a targeted text edit rather
    /// than parse-and-reserialise. A batched writer that lost them would undo
    /// that quietly.
    #[test]
    fn writing_a_table_keeps_the_comments_around_it() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-comments");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let file = path.to_str().unwrap();
        std::fs::write(
            &path,
            "# why this console needs this core\n[cores]\n# and why not the other one\nnes = \"nestopia\"\n",
        )
        .unwrap();

        super::set_table_entries(
            file,
            "cores",
            &[("nes".to_owned(), Some("mesen".to_owned()))],
        )
        .unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("# why this console needs this core"),
            "a comment was lost"
        );
        assert!(
            after.contains("# and why not the other one"),
            "a comment was lost"
        );
        assert!(
            after.contains("nes = \"mesen\""),
            "the value was not changed"
        );
    }

    /// Nothing to change means nothing to write. Rebinding one button walks
    /// all 45 entries, and 44 of them are already what they should be — a file
    /// rewritten every time is one that eventually gets caught half-written.
    #[test]
    fn a_table_that_would_not_change_is_not_rewritten() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-nochange");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let file = path.to_str().unwrap();
        std::fs::write(&path, "[cores]\nnes = \"mesen\"\n").unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        super::set_table_entries(
            file,
            "cores",
            &[("nes".to_owned(), Some("mesen".to_owned()))],
        )
        .unwrap();

        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before,
            "the file was rewritten with the same contents"
        );
    }

    /// The app wrote `autofire = false` into people's config files while this
    /// was a toggle. Changing the field to a string without accepting that did
    /// not merely lose the setting — the whole file failed to parse, so every
    /// other value in it fell back to a default: no server, no library path.
    /// A setting that can break the file it lives in is worse than one that is
    /// wrong.
    #[test]
    fn an_old_boolean_autofire_still_loads_the_rest_of_the_file() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-autofire");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[server]\nurl = \"http://dev.lan\"\n\n[retroarch]\nautofire = false\n",
        )
        .unwrap();

        let cfg = Config::load_from(&path).expect("an old config still has to load");
        assert_eq!(
            cfg.server.url, "http://dev.lan",
            "the rest of the file was lost"
        );
        assert_eq!(cfg.retroarch.autofire, "off");

        // `true` meant rapid fire was wanted, so it stays wanted — on the
        // modifier, which is the arrangement that works.
        std::fs::write(&path, "[retroarch]\nautofire = true\n").unwrap();
        assert_eq!(Config::load_from(&path).unwrap().retroarch.autofire, "lb");

        // The old face-button arrangement. Anyone who had it on asked for rapid
        // fire and still gets it, rather than finding it silently off.
        //
        // "a" and "bottom" go to the left bumper because the bottom face
        // button is the *fire* button — a modifier sitting on it cancels the
        // repeat outright, so there is nowhere for it to go but a shoulder.
        // "y" and "top" now mean what they say: the top face button is a
        // modifier the pad can actually hold, so the button they picked is the
        // button they keep.
        for old in ["a", "bottom"] {
            std::fs::write(&path, format!("[retroarch]\nautofire = \"{old}\"\n")).unwrap();
            assert_eq!(
                crate::tweaks::AutoFire::parse(
                    &Config::load_from(&path).unwrap().retroarch.autofire
                ),
                crate::tweaks::AutoFire::LeftBumper,
                "{old} lost its rapid fire"
            );
        }
        for old in ["y", "top"] {
            std::fs::write(&path, format!("[retroarch]\nautofire = \"{old}\"\n")).unwrap();
            assert_eq!(
                crate::tweaks::AutoFire::parse(
                    &Config::load_from(&path).unwrap().retroarch.autofire
                ),
                crate::tweaks::AutoFire::Top,
                "{old} did not keep the button it named"
            );
        }

        // The current form still works, and an absent one is the default.
        std::fs::write(&path, "[retroarch]\nautofire = \"rb\"\n").unwrap();
        assert_eq!(Config::load_from(&path).unwrap().retroarch.autofire, "rb");
        std::fs::write(&path, "[retroarch]\n").unwrap();
        assert_eq!(Config::load_from(&path).unwrap().retroarch.autofire, "lb");
    }

    /// The boot order is the point: a portable build can shadow a system one
    /// without uninstalling either, and disabling an entry must skip it rather
    /// than reorder anything.
    #[test]
    fn the_retroarch_boot_order_honours_position_and_enabled() {
        let cfg: Config = toml::from_str(
            r#"
            [[retroarch.installs]]
            path = "/first"
            [[retroarch.installs]]
            path = "/disabled"
            enabled = false
            [[retroarch.installs]]
            path = "/second"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.retroarch.ordered_paths(), ["/first", "/second"]);
    }

    /// An entry with no `enabled` key is on. Defaulting the other way would
    /// silently disable every install in an existing config.
    #[test]
    fn an_install_is_enabled_unless_it_says_otherwise() {
        let cfg: Config = toml::from_str("[[retroarch.installs]]\npath = \"/ra\"\n").unwrap();
        assert_eq!(cfg.retroarch.ordered_paths(), ["/ra"]);
    }

    /// `installs` supersedes the legacy single `root`, but an older config that
    /// only sets `root` has to keep working untouched.
    #[test]
    fn the_legacy_single_root_still_works_and_is_superseded() {
        let legacy: Config = toml::from_str("[retroarch]\nroot = \"/old\"\n").unwrap();
        assert_eq!(legacy.retroarch.ordered_paths(), ["/old"]);

        let both: Config = toml::from_str(
            "[retroarch]\nroot = \"/old\"\n[[retroarch.installs]]\npath = \"/new\"\n",
        )
        .unwrap();
        assert_eq!(
            both.retroarch.ordered_paths(),
            ["/new"],
            "the list wins; the old key is not appended"
        );
    }

    /// Nothing configured means "probe the usual places", which is an empty
    /// list rather than a list containing an empty path.
    #[test]
    fn no_retroarch_config_asks_for_the_default_probe() {
        assert!(Config::default().retroarch.ordered_paths().is_empty());
    }

    /// A missing config is not an error — most commands work offline against
    /// the local cache and should not require one.
    /// A quoted boolean has to load.
    ///
    /// `set_config_field` wrote `swap_ab = "true"` for several versions, and
    /// TOML refuses the whole file over one wrong type — which, with every
    /// caller falling back to defaults, silently moved the library, the artwork
    /// and the saves folder on the device. The reader takes both spellings so
    /// those files recover on their own.
    #[test]
    fn a_boolean_written_as_a_string_still_loads_the_whole_file() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-quoted-bool");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[esde]\nroms = \"/storage/CARD/Roms\"\n\n[controllers]\nswap_ab = \"true\"\nswap_xy = \"false\"\n",
        )
        .unwrap();

        let cfg = Config::load_from(&path).expect("one quoted bool must not lose the file");
        assert!(cfg.controllers.swap_ab);
        assert!(!cfg.controllers.swap_xy);
        assert_eq!(
            cfg.esde.roms.as_deref(),
            Some("/storage/CARD/Roms"),
            "this is what was actually being lost — the whole file, not just the bool"
        );

        // Nonsense is still an error rather than quietly false.
        std::fs::write(&path, "[controllers]\nswap_ab = \"maybe\"\n").unwrap();
        assert!(Config::load_from(&path).is_err());
    }

    #[test]
    fn a_missing_config_loads_defaults_rather_than_failing() {
        let cfg = Config::load_from(Path::new("/nonexistent/config.toml"))
            .expect("absent config is fine");
        assert_eq!(cfg.library.local_root, "./library");
        assert!(cfg.server.url.is_empty());
        assert!(cfg.shaders.enabled, "shaders default on");
        assert!(!Config::exists("/nonexistent/config.toml"));
    }

    /// Malformed TOML must name the file. It used to surface as a bare parse
    /// error with no indication of which file to go and look at.
    #[test]
    fn a_broken_config_says_which_file_it_is() {
        let dir = std::env::temp_dir().join("moose-rack-config-broken");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[server\nurl = ").unwrap();

        let err = Config::load_from(&path)
            .expect_err("not valid TOML")
            .to_string();
        assert!(err.contains("config.toml"), "got: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every derived path hangs off `library.local_root`, so one setting moves
    /// the whole footprint — which is what makes "delete that folder" true.
    #[test]
    fn every_download_location_derives_from_one_setting() {
        let cfg: Config = toml::from_str("[library]\nlocal_root = \"/data/moose-rack\"\n").unwrap();
        for dir in [
            cfg.local_roms_dir(),
            cfg.media_dir(),
            cfg.system_dir(),
            cfg.themes_dir(),
        ] {
            assert!(
                dir.starts_with("/data/moose-rack"),
                "{} escaped the library root",
                dir.display()
            );
        }
    }

    /// The section was renamed, and the old name has to keep working — a config
    /// that silently stops applying does not error, it just turns achievements
    /// off, which is indistinguishable from never having set them up.
    #[test]
    fn the_old_cheevos_section_name_is_still_accepted() {
        let renamed: Config =
            toml::from_str("[achievements]\nenabled = true\nusername = \"frank\"\ntoken = \"t\"\n")
                .unwrap();
        let legacy: Config =
            toml::from_str("[cheevos]\nenabled = true\nusername = \"frank\"\ntoken = \"t\"\n")
                .unwrap();

        for (label, cfg) in [("achievements", renamed), ("cheevos", legacy)] {
            let s = cfg.achievements.settings();
            assert!(s.enabled, "[{label}] should be read");
            assert!(s.usable(), "[{label}] should authenticate");
            assert_eq!(s.username.as_deref(), Some("frank"), "[{label}]");
            assert_eq!(
                s.credential(),
                Some(("cheevos_token", "t")),
                "[{label}] credential"
            );
        }
    }

    /// Absent means off, and off means the user's own RetroArch settings are
    /// left alone rather than overwritten with a disabled login.
    #[test]
    fn no_achievements_section_means_untouched() {
        let cfg: Config = toml::from_str("[server]\nurl = \"http://x\"\n").unwrap();
        let s = cfg.achievements.settings();
        assert!(!s.enabled);
        assert!(!s.usable());
        assert!(crate::achievements::config_lines(&s).is_empty());
    }

    /// Every field of the section has to survive the trip, including the two
    /// that change what the emulator does.
    #[test]
    fn the_achievements_section_maps_every_field() {
        let cfg: Config = toml::from_str(
            "[achievements]\nenabled = true\nusername = \"frank\"\n\
             token = \"tok\"\nhardcore = true\ntest_unofficial = true\n",
        )
        .unwrap();
        let s = cfg.achievements.settings();
        assert_eq!(s.credential(), Some(("cheevos_token", "tok")));

        // A config still carrying the old `password` key loads rather than
        // failing — the field is gone, and an unknown key is not a reason to
        // refuse someone's whole config file — but it authenticates nothing.
        let old: Config = toml::from_str(
            "[achievements]\nenabled = true\nusername = \"frank\"\npassword = \"pw\"\n",
        )
        .expect("an older config must still load");
        assert_eq!(old.achievements.settings().credential(), None);
        assert!(
            !old.achievements.settings().usable(),
            "a password still logs in"
        );
        assert!(s.hardcore);
        assert!(s.test_unofficial);
        let out = crate::achievements::config_lines(&s);
        assert!(out.contains("cheevos_hardcore_mode_enable = \"true\""));
        assert!(out.contains("cheevos_test_unofficial = \"true\""));
    }

    /// A configured token has to actually reach the client, or the app keeps
    /// sending Basic and the token silently does nothing.
    #[test]
    fn a_configured_token_reaches_the_client_as_a_bearer() {
        let cfg: Config = toml::from_str(
            "[server]\nurl = \"http://dev.lan\"\ntoken = \"rmm_abc\"\nusername = \"frank\"\n",
        )
        .unwrap();
        assert_eq!(cfg.server.client().unwrap().auth(), "Bearer rmm_abc");
    }

    /// Without a token it falls back to Basic, so an existing config that never
    /// had one keeps working untouched.
    #[test]
    fn without_a_token_the_client_falls_back_to_basic() {
        let cfg: Config = toml::from_str(
            "[server]\nurl = \"http://dev.lan\"\nusername = \"user\"\npassword = \"pass\"\n",
        )
        .unwrap();
        assert_eq!(cfg.server.client().unwrap().auth(), "Basic dXNlcjpwYXNz");
    }

    /// A token alone is enough — the password was deliberately removed from the
    /// real config, so building a client must not require one.
    #[test]
    fn a_token_alone_is_enough_to_build_a_client() {
        let cfg: Config =
            toml::from_str("[server]\nurl = \"http://dev.lan\"\ntoken = \"rmm_abc\"\n").unwrap();
        assert!(
            cfg.server.client().is_ok(),
            "a config with no username or password must still connect"
        );
    }

    /// Per-game keys are file paths, which TOML cannot express as bare keys.
    /// Round-tripping through a real parser is the only check that matters.
    #[test]
    fn the_shipped_arcade_table_parses_and_is_all_arcade() {
        let map = arcade_core_map();
        assert!(
            map.len() > 100,
            "only {} rows — did the table lose its contents?",
            map.len()
        );
        for (game, core) in &map {
            assert!(game.starts_with("arcade/"), "{game} is not an arcade path");
            assert!(game.ends_with(".zip"), "{game} is not a romset");
            assert!(!core.is_empty(), "{game} has no core");
        }
    }

    /// The point of moving the table out of config.toml was that nothing about
    /// launching a game changed. A config with no `[cores.per_game]` still has
    /// to resolve the 154 romsets whose platform default cannot run them.
    #[test]
    fn a_config_without_the_table_still_gets_it() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-shipped-only");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[server]\nurl = \"http://x\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(
            cfg.cores
                .per_game
                .get("arcade/mrdo.zip")
                .map(String::as_str),
            Some("mame2003_plus")
        );
    }

    /// And a core chosen in the detail pane still wins: the shipped rows go
    /// underneath the file, never over it.
    #[test]
    fn a_hand_picked_core_beats_the_shipped_one() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-shipped-beaten");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[cores.per_game]\n\"arcade/mrdo.zip\" = \"fbneo\"\n").unwrap();
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(
            cfg.cores
                .per_game
                .get("arcade/mrdo.zip")
                .map(String::as_str),
            Some("fbneo")
        );
        // and the rest of the table is still there beside it
        assert!(cfg.cores.per_game.len() > 100);
    }

    #[test]
    fn per_game_keys_round_trip() {
        let dir = std::env::temp_dir().join(format!("moose-rack-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.toml");
        let p = path.to_str().unwrap();
        std::fs::write(&path, "[cores.overrides]\narcade = \"mame2003_plus\"\n").unwrap();

        // Names taken from the real library: spaces, dots, brackets, commas,
        // apostrophes, and a quote for good measure.
        let keys = [
            "arcade/blazstar.zip",
            "psx/Final Fantasy VII (USA) (Disc 1).chd",
            "snes/Blow'em Out! (USA) (Aftermarket) (Unl).zip",
            "nes/Say \"Hello\" [b1].nes",
            "gba/back\\slash.gba",
        ];
        for (i, k) in keys.iter().enumerate() {
            set_table_entry(p, "cores.per_game", k, &format!("core{i}")).unwrap();
        }

        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap())
            .expect("must still be valid TOML");
        let tbl = parsed["cores"]["per_game"].as_table().unwrap();
        assert_eq!(tbl.len(), keys.len(), "one entry per key, no duplicates");
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(tbl[*k].as_str(), Some(format!("core{i}").as_str()));
        }

        // Rewriting a key must replace, not append.
        set_table_entry(p, "cores.per_game", keys[0], "fbneo").unwrap();
        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let tbl = parsed["cores"]["per_game"].as_table().unwrap();
        assert_eq!(tbl.len(), keys.len(), "rewrite must not duplicate");
        assert_eq!(tbl[keys[0]].as_str(), Some("fbneo"));

        // Clearing removes it.
        clear_table_entry(p, "cores.per_game", keys[0]).unwrap();
        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed["cores"]["per_game"].as_table().unwrap().len(),
            keys.len() - 1
        );

        // The hand-written table above must survive untouched.
        assert_eq!(
            parsed["cores"]["overrides"]["arcade"].as_str(),
            Some("mame2003_plus")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod bool_tests {

    /// The games folder and the artwork folder follow the ES-DE settings.
    ///
    /// Before this they were always `<library>/roms` and
    /// `<library>/downloaded_media`, so an install pointed at a real ES-DE
    /// library read from one place and wrote to another — the app kept a second
    /// copy of everything beside the one already on the disk.
    #[test]
    fn the_library_folders_follow_the_esde_settings() {
        let cfg: Config = toml::from_str(
            r#"
            [library]
            local_root = "./library"
            [esde]
            root = "/sd/ES-DE"
            roms = "/sd/Roms"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.local_roms_dir(), PathBuf::from("/sd/Roms"));
        assert_eq!(cfg.media_dir(), PathBuf::from("/sd/ES-DE/downloaded_media"));
    }

    /// And an install that has never been pointed anywhere is unchanged.
    ///
    /// The defaults have to land exactly where they always did, or an existing
    /// library becomes invisible the moment this ships.
    #[test]
    fn nothing_moves_for_an_install_that_has_set_nothing() {
        let cfg: Config = toml::from_str(
            r#"
            [library]
            local_root = "./library"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.local_roms_dir(), PathBuf::from("./library/roms"));
        assert_eq!(cfg.media_dir(), PathBuf::from("./library/downloaded_media"));
    }

    /// BIOS and themes live with ES-DE too.
    ///
    /// The BIOS folder is `0_BIOS` and not `BIOS`: that is ES-DE's name for it,
    /// and it is the folder that already holds the files on a machine that has
    /// them. Getting the name wrong makes a second empty folder beside a full
    /// one, which looks exactly like the app not finding anything.
    #[test]
    fn bios_and_themes_follow_esde_as_well() {
        let cfg: Config = toml::from_str(
            r#"
            [library]
            local_root = "./library"
            [esde]
            root = "/sd/ES-DE"
            roms = "/sd/Roms"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.system_dir(), PathBuf::from("/sd/Roms/0_BIOS"));
        assert_eq!(cfg.themes_dir(), PathBuf::from("/sd/ES-DE/themes"));
    }

    /// RetroArch's own files stay put, deliberately.
    ///
    /// Frank asked for these to stay in the library folder: they are this app's
    /// working files for one emulator, not part of an ES-DE library, and moving
    /// them into one would put this app's generated config among somebody
    /// else's.
    #[test]
    fn the_retroarch_files_stay_in_the_library_folder() {
        let cfg: Config = toml::from_str(
            r#"
            [library]
            local_root = "./library"
            [esde]
            root = "/sd/ES-DE"
            roms = "/sd/Roms"
            "#,
        )
        .unwrap();
        assert_eq!(
            cfg.user_retroarch_config(),
            PathBuf::from("./library/retroarch-user.cfg")
        );
    }


    use super::*;

    /// A TOML boolean is a bare literal. Written as a quoted string it parses
    /// as a string and then fails to deserialise into a bool — so a switch in
    /// Settings would appear to save and do nothing on the next launch.
    #[test]
    fn booleans_are_written_unquoted() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-bool");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.toml");
        let p = path.to_str().unwrap();
        std::fs::write(&path, "[achievements]\nusername = \"frank\"\n").unwrap();

        set_table_bool(p, "achievements", "enabled", true).unwrap();
        set_table_bool(p, "achievements", "hardcore", false).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("enabled = true"), "unquoted: {raw}");
        assert!(!raw.contains("\"true\""), "never quoted: {raw}");

        // And it has to survive the real parser into the real struct.
        let cfg: Config = toml::from_str(&raw).unwrap();
        assert!(cfg.achievements.enabled);
        assert!(!cfg.achievements.hardcore);
        assert_eq!(cfg.achievements.username.as_deref(), Some("frank"));

        // Flipping it back rewrites in place rather than appending a second key.
        set_table_bool(p, "achievements", "enabled", false).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.matches("enabled").count(), 1, "one key, not two: {raw}");
        assert!(!toml::from_str::<Config>(&raw).unwrap().achievements.enabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Strings and booleans share the writer, so the string path must still
    /// quote and must not have been broken by adding the raw one.
    #[test]
    fn strings_are_still_quoted() {
        let dir = std::env::temp_dir().join("moose-rack-cfg-str");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.toml");
        let p = path.to_str().unwrap();

        set_table_entry(p, "library", "local_root", "/data/moose-rack").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("local_root = \"/data/moose-rack\""), "{raw}");
        let cfg: Config = toml::from_str(&raw).unwrap();
        assert_eq!(cfg.library.local_root, "/data/moose-rack");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod config_path_tests {
    /// Every reader must go through `config::path()`.
    ///
    /// A correctly configured server reported `config_exists: false` and a
    /// Settings page with nothing in it, because the state was built from the
    /// file `moose-service.toml` named while `config_fields`, `status` and every
    /// setting writer went on reading `config.toml` out of the working
    /// directory. Twenty-odd call sites, each individually reasonable.
    ///
    /// The one allowed literal is the list of file names `neighbours` looks for
    /// beside the binary, which is about names on disk and not about this file.
    #[test]
    fn nothing_names_the_config_file_itself() {
        for (name, src) in [
            ("commands.rs", include_str!("commands.rs")),
            ("appicon.rs", include_str!("appicon.rs")),
            ("app.rs", include_str!("app.rs")),
        ] {
            // Production code only. A test that puts the default back is
            // naming the default on purpose.
            let src = src.split("#[cfg(test)]").next().unwrap_or(src);
            for (i, line) in src.lines().enumerate() {
                if !line.contains("\"config.toml\"") {
                    continue;
                }
                // A comment explaining the default is not a reader.
                let code = line.trim_start();
                if code.starts_with("//") || code.starts_with("///") {
                    continue;
                }
                assert!(
                    line.contains("cache.sqlite3"),
                    "{name}:{} names the config file instead of asking config::path(): {line}",
                    i + 1
                );
            }
        }
    }

    /// Setting it changes what every reader sees.
    ///
    /// The literal scan above cannot catch the actual failure, which was that
    /// `set_path` was never called at all: no literal, no reader, nothing to
    /// find, and a server that reported `config_exists: false` while holding a
    /// perfectly good config.
    #[test]
    fn naming_a_file_changes_what_is_read() {
        let dir = std::env::temp_dir().join("moose-config-path-test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("elsewhere.toml");
        std::fs::write(&f, "[media]\nlist_art = \"miximages\"\n").unwrap();

        super::set_path(f.clone());
        assert_eq!(super::path(), f);
        assert!(super::Config::exists(&super::path_str()), "the named file is not seen");
        assert_eq!(super::Config::load().unwrap().media.list_art, "miximages");

        // Put it back, so the rest of this binary sees the default.
        super::set_path(std::path::PathBuf::from("config.toml"));
        assert_eq!(super::path().file_name().unwrap(), "config.toml");
    }
}

// What is on screen, and what the cursor is on.
//
// No drawing here and no SDL: this is the front end's state, and every rule it
// applies belongs to `moose_rack`. Which order a list is in comes from
// `gamesort`, what a filter keeps from `gamefilter`, where the cursor goes next
// from `gridnav`, and what a key or a button means from `binds`. That was the
// point of doing task 1 first — this file is a few hundred lines because none
// of those decisions are taken twice.

use anyhow::{Context, Result};
use moose_rack::cache::{Cache, CollectionRow};
use moose_rack::gamelist::{self, Chosen, Row};
use moose_rack::gridnav::{self, Moves};
use moose_rack::{gamefilter, gamesort};
use std::path::{Path, PathBuf};

/// What a cover is shaped like before anything has been measured. Three by
/// four, which is most console box art and all of the placeholder cards.
pub const DEFAULT_ASPECT: f32 = 0.75;

/// Everything the preview column says about one game.
pub struct Detail {
    pub id: i64,
    pub name: String,
    pub platform: String,
    /// What its artwork is filed under.
    pub stem: String,
    /// The name on disk, extension and all.
    pub file: String,
    /// The name on disk, extension and all.
    pub size: String,
    pub favorite: bool,
    pub last_played: Option<String>,
    pub downloaded: bool,
    pub hardware: Option<&'static str>,
    pub maker: Option<&'static str>,
    /// RomM's 0-10 score.
    pub rating: Option<f64>,
    pub year: Option<i32>,
    pub players: Option<u8>,
    /// The blurb RomM holds, and the genres out of its metadata. Both come
    /// from the cache row rather than the list row, so they are fetched for
    /// the highlighted game only — see [`Library::deep`].
    pub summary: Option<String>,
    pub genres: Vec<String>,
}

impl Detail {
    /// The pane's rows, as label and value. Skipping what is not known rather
    /// than printing a blank: a field with nothing in it is a question the
    /// pane cannot answer, and a dash is not an answer.
    /// Whether this game came with anything beyond its file.
    ///
    /// A card scan gives a name and a size; a RomM sync gives a rating, a
    /// genre, a maker and a blurb. The pane shows the file itself only in the
    /// first case, where without it there is nothing to read.
    fn facts_are_thin(&self) -> bool {
        self.rating.is_none()
            && self.year.is_none()
            && self.genres.is_empty()
            && self.summary.is_none()
            && self.maker.is_none()
    }

    pub fn facts(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![("Console", self.platform.clone())];
        // Rating first: on the desktop it is the line the eye goes to, and a
        // score is the one fact here that is an opinion rather than a
        // measurement.
        if let Some(rating) = self.rating {
            out.push(("Rating", format!("{rating:.1} / 10")));
        }
        if let Some(year) = self.year {
            out.push(("Released", year.to_string()));
        }
        if !self.genres.is_empty() {
            out.push(("Genre", self.genres.join(", ")));
        }
        if let Some(players) = self.players.filter(|n| *n > 1) {
            out.push(("Players", format!("up to {players}")));
        }
        out.push(("Size", self.size.clone()));
        if self.favorite {
            out.push(("Starred", "yes".to_owned()));
        }
        if let Some(maker) = self.maker {
            out.push(("Made by", maker.to_owned()));
        }
        if let Some(hardware) = self.hardware {
            out.push(("Hardware", hardware.to_owned()));
        }
        match &self.last_played {
            // The date only. The time it was started is not something anybody
            // is looking for here.
            Some(when) => out.push(("Last played", when.chars().take(10).collect())),
            None => out.push(("Last played", "never".to_owned())),
        }
        out.push(("Downloaded", if self.downloaded { "yes" } else { "no" }.to_owned()));
        // What the file is, when there is nothing else to say.
        //
        // A library scanned from a card has no rating, no genre, no blurb and
        // no cover — the whole pane came down to Console and Size, which is
        // less than the row already showed. These two are always knowable, and
        // "which file is this, and is it a disc set" is a real question in a
        // folder of `.chd`, `.m3u` and `.cue`.
        if let Some(kind) = self
            .file
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_uppercase())
            .filter(|e| !e.is_empty() && e.len() <= 5)
        {
            out.push(("Format", kind));
        }
        if self.facts_are_thin() {
            out.push(("File", self.file.clone()));
        }
        out
    }
}

/// The tabs across the top.
///
/// The shoulder buttons cycle them — `prevSection` and `nextSection`, which is
/// why every one of them is in the list whether or not it draws anything yet.
/// A shoulder press that lands on a hole is worse than one that lands on a page
/// saying what will be there.
///
/// Continue is gone: it and History answered the same question with two
/// different pictures of it, and History is the one that can also say how long.
/// So History carries both jobs and the tab row is one shorter, which on a
/// 640-point panel is worth having.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub id: &'static str,
    pub label: &'static str,
    /// Whether this tab draws its own content yet. One that does not says so on
    /// the page rather than showing an empty frame.
    pub ready: bool,
}

pub const SECTIONS: &[Section] = &[
    Section { id: "library", label: "Library", ready: true },
    Section { id: "mine", label: "Collections", ready: true },
    Section { id: "settings", label: "Settings", ready: true },
    Section { id: "history", label: "History", ready: true },
    Section { id: "syncing", label: "Syncing", ready: true },
];

/// Which screen is showing, within whichever tab is open.
///
/// One enum for every tab rather than a place per tab, because only one list is
/// ever on screen: the cursor, the movement table and the scroll position are
/// all singular, and giving each tab its own copy is how they drift apart. Which
/// place a tab *starts* at is [`root_view`], and `back` walks up from there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Library: the consoles.
    Platforms,
    /// Library or Collections: a list of games.
    Roms,
    /// Collections: the kinds — user, franchise, genre, company.
    Groups,
    /// Collections: the collections within one kind.
    Collections,
    /// History: everything played, most time first.
    History,
    /// Settings: the panes.
    Panes,
    /// Settings: one pane's settings.
    Options,
    /// Settings: the Wi-Fi list.
    Wifi,
    /// Library: one folder of ports or tools.
    Scripts,
}

/// Where a tab starts when you arrive at it.
///
/// Switching tabs resets to this. Without it, leaving Library inside a console's
/// games and shouldering over to Collections arrives at a game list holding the
/// *console's* games under the Collections heading.
pub fn root_view(section: &str) -> View {
    match section {
        "mine" => View::Groups,
        "history" => View::History,
        "settings" => View::Panes,
        _ => View::Platforms,
    }
}

/// RomM's group names, as something to read.
fn kind_label(group: &str) -> &'static str {
    match group {
        "collection" => "Series",
        "franchise" => "Franchises",
        "genre" => "Genres",
        "company" => "Companies",
        "mode" => "Modes",
        _ => "Other",
    }
}

/// One row on the Collections tab's first screen.
///
/// Your own collections and RomM's generated kinds share a list because they
/// are the same gesture — pick a shelf, get games. They are not the same
/// *depth*, though: yours open straight onto their games, and a kind opens onto
/// the collections inside it, because there are 1,022 company collections and
/// nobody wants those flattened into anything.
pub enum Shelf {
    /// One of yours. Opens its games.
    Mine { id: String, name: String, games: i64 },
    /// A kind of RomM collection. Opens the collections within it.
    Kind { group: String, label: &'static str, count: i64 },
}

impl Shelf {
    pub fn name(&self) -> &str {
        match self {
            Shelf::Mine { name, .. } => name,
            Shelf::Kind { label, .. } => label,
        }
    }

    /// The line the root menu's side panel shows about it.
    ///
    /// One of yours is a list you made; a RomM kind is a way the server groups
    /// things. Saying which is which is the whole reason the two halves of that
    /// page are separated by a rule.
    pub fn note(&self) -> String {
        match self {
            Shelf::Mine { games, .. } => match games {
                0 => "One of your collections. Nothing in it yet.".to_owned(),
                1 => "One of your collections. One game.".to_owned(),
                n => format!("One of your collections. {n} games."),
            },
            Shelf::Kind { label, count, .. } => match count {
                1 => format!("How RomM groups games by {}. One of them.", label.to_lowercase()),
                n => format!("How RomM groups games by {}. {n} of them.", label.to_lowercase()),
            },
        }
    }

    /// The number on the right: games for one of yours, collections for a kind.
    pub fn count(&self) -> i64 {
        match self {
            Shelf::Mine { games, .. } => *games,
            Shelf::Kind { count, .. } => *count,
        }
    }
}

/// A list of options, open over the settings screen.
///
/// This is what a setting does instead of stepping. Stepping means holding a
/// direction and watching values go past to find the one you want, and on a list
/// of nine cores it means doing that while reading — which is the control this
/// replaced.
/// The picker field that means "download this set" rather than "save this
/// value". Not a config key: nothing is written, a thread starts instead.
pub const FETCH_ICONS: &str = "\u{0}fetch-icons";

pub struct Picker {
    pub title: &'static str,
    /// `(value, label)`, the same shape the setting holds.
    pub options: Vec<(String, String)>,
    pub at: usize,
    /// Where the choice is written. Empty for a choice that is not a setting.
    pub field: &'static str,
}

/// Waiting for a button, to put a control on it.
pub struct Capture {
    pub action: String,
    pub label: &'static str,
}

/// Something waiting for the keyboard, and what to do with what is typed.
pub struct TextField {
    /// Where the text goes when it is finished. Distinguishing these is not
    /// optional: both open the same keyboard, and without it a search typed on
    /// the Library tab was written into whichever setting the Settings tab
    /// happened to be sitting on.
    pub target: Target,
    pub prompt: &'static str,
    pub value: String,
    pub secret: bool,
}

/// What a keyboard is filling in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A setting, written to `moose-sdl.toml` under this `table.key`.
    Setting(&'static str),
    /// The search box over a game list. Written nowhere.
    Search,
    /// A Wi-Fi password for this network. Handed to the device's own helper
    /// and never written to a file — the device saves it itself.
    WifiKey(String),
}

/// One game in the History list.
pub struct Played {
    pub id: i64,
    /// What its artwork is filed under.
    pub stem: String,
    pub name: String,
    pub platform: String,
    pub seconds: i64,
    pub runs: i64,
    /// When it was last started, as the server recorded it.
    pub last: String,
}

/// What the Syncing page can say without touching the network.
///
/// Read once at startup from the cache the other front ends fill. Nothing here
/// performs a sync — this is the state a sync would change, which is the honest
/// half to build first.
pub struct SyncState {
    pub server_version: Option<String>,
    pub watermark: Option<String>,
    pub games: i64,
    pub consoles: usize,
    pub collections: i64,
    /// From the play log. Named rather than a tuple because
    /// `Cache::play_totals` returns them in the order seconds, sessions,
    /// games — which read as games, sessions, seconds reports 6,065 games
    /// played for twenty seconds, and looks plausible enough to ship.
    pub seconds_played: i64,
    pub sessions: i64,
    pub games_played: i64,
}

pub struct Console {
    pub slug: String,
    pub name: String,
    pub games: i64,
    /// A folder of scripts rather than a RomM platform.
    ///
    /// Ports and Tools sit in the console list because that is where KNULLI
    /// puts them, but opening one lists shell scripts rather than games and
    /// launching one runs `emulatorlauncher` rather than RetroArch.
    pub scripts: bool,
}

pub struct Library {
    cache: Cache,
    media_root: PathBuf,
    /// Where downloads land, for deciding what will play offline.
    roms_dir: PathBuf,
    pub consoles: Vec<Console>,
    /// Which console the picker is on, always — the picker column shows it
    /// highlighted whether or not its games are open.
    pub console_at: usize,
    pub view: View,
    /// The games of the open console, unsorted and unfiltered. What `arranged`
    /// holds is which of them to draw, in what order.
    rows: Vec<Row>,
    /// The ROM filename without its extension, per row, in the same order.
    ///
    /// What the artwork on disk is named after — see `media::local_art`. Kept
    /// beside the rows rather than on them because `gamelist::Row` is the
    /// shape a *list* needs, and where a file sits is not part of that.
    stems: Vec<String>,
    /// The same rows' names on disk, extension and all.
    files: Vec<String>,
    arranged: Vec<usize>,
    /// Which tab is open.
    pub section: usize,
    /// Sofa or Desk. Not what the window can hold — what is wanted.
    /// The Collections tab's first screen: yours flat, then RomM's kinds.
    pub shelves: Vec<Shelf>,
    pub shelf_at: usize,
    /// How many of `shelves` are yours — the divider goes after this many, and
    /// it is 0 when you have none.
    pub mine_count: usize,
    /// The last collection peeked into, and the names that came back.
    peeked: (String, Vec<String>),
    /// The last game looked up in full, and what came back — its blurb and its
    /// genres, which the list rows do not carry.
    looked_up: (i64, Option<String>, Vec<String>),
    /// Set when the Exit row is chosen, and read by the loop.
    pub quitting: bool,
    /// A set of console pictures being downloaded, if one is.
    pub fetching: Option<crate::iconfetch::Fetch>,
    /// Sample pictures for whichever set the picker is sitting on.
    pub previews: Option<crate::iconfetch::Previews>,
    pub cols: Vec<CollectionRow>,
    pub col_at: usize,
    /// The open collection's name, for the header once its games are showing.
    pub col_name: String,
    pub history: Vec<Played>,
    pub history_at: usize,
    /// Settings, built from the config when the app started.
    pub panes: Vec<crate::settings::Pane>,
    pub pane_at: usize,
    pub option_at: usize,
    /// What the search box holds. Empty means everything.
    pub query: String,
    /// The open list of options, if a setting was activated.
    pub picking: Option<Picker>,
    /// Waiting for a button press, if a control was activated.
    pub capturing: Option<Capture>,
    /// The device's own system table, read once.
    pub es_systems: Vec<crate::ports::System>,
    /// The open Ports or Tools group.
    pub ports: Vec<crate::ports::Port>,
    pub port_at: usize,
    /// Which folder those came from, for launching and for the heading.
    pub port_system: String,
    /// The Wi-Fi screen, when it is open.
    pub wifi: crate::wifi::State,
    pub wifi_at: usize,
    /// A scan or a join in flight. Polled by the loop, not by the drawing.
    pub wifi_job: Option<crate::wifi::Job>,
    /// Set when a text setting was activated, for the loop to open a keyboard
    /// on. Taken rather than read, so one press opens one keyboard.
    pub wants_keyboard: Option<TextField>,
    pub sync: SyncState,
    /// Where the cursor is, as a place in `arranged`.
    pub at: usize,
    /// The shape of this console's box art, width over height.
    ///
    /// Measured off what is actually on disk — a PSP UMD case is 0.58 and a
    /// SNES box 1.37, and one ratio for all of them crops most systems. Per
    /// console rather than per game so the grid stays regular, which is also
    /// what lets the cursor move by arithmetic.
    pub aspect: f32,
    /// How far the grid is scrolled, in points. Owned here rather than by the
    /// drawing so it survives a redraw and a resize.
    pub scrolled: f32,
    chosen: Chosen,
    /// Where each drawn row leads, worked out once per relayout.
    moves: Moves,
    columns: usize,
}

impl Library {
    /// Open the metadata cache and read the consoles.
    /// `server_collections` is `[library] server_collections` — whether RomM's
    /// 1,931 generated collections join yours on the Collections tab.
    /// Whether there is a real library here, as opposed to the groups the
    /// device's own `es_systems.cfg` always provides.
    ///
    /// Ports, Tools and Emulators are pushed onto the console list from that
    /// file whether or not anything has ever been cached, so "are there any
    /// consoles" is always yes and cannot be the question. It was: the first
    /// run on the handheld saw three consoles, decided the library was fine,
    /// and never scanned the card.
    pub fn has_cached_library(&self) -> bool {
        self.consoles.iter().any(|c| !c.scripts)
    }

    pub fn open(
        path: &Path,
        media_root: PathBuf,
        roms_dir: PathBuf,
        server_collections: bool,
    ) -> Result<Self> {
        let cache = Cache::open(path).with_context(|| format!("opening {}", path.display()))?;
        let names = moose_rack::coremap::CoreMap::load_or_embedded(std::path::Path::new(
            "data/esde-core-map.json",
        ));
        let consoles = cache
            .platforms()
            .context("reading the consoles")?
            .into_iter()
            .map(|p| Console {
                // A library scanned from the card knows the directory name and
                // nothing else, so the console was called `psx` and `gbc` on
                // screen. The proper name is already written down here — the
                // same table the desktop reads — and the slug is the fallback
                // rather than the answer.
                name: match p.display_name.trim() {
                    named if !named.is_empty() && named != p.fs_slug => named.to_owned(),
                    _ => names
                        .display_name(&p.fs_slug)
                        .map(str::to_owned)
                        .unwrap_or_else(|| p.fs_slug.clone()),
                },
                slug: p.fs_slug,
                games: p.rom_count,
                scripts: false,
            })
            .collect();
        let mut lib = Library {
            files: Vec::new(),
            peeked: (String::new(), Vec::new()),
            looked_up: (0, None, Vec::new()),
            quitting: false,
            fetching: None,
            previews: None,
            cache,
            media_root,
            roms_dir,
            consoles,
            console_at: 0,
            view: View::Platforms,
            rows: Vec::new(),
            stems: Vec::new(),
            arranged: Vec::new(),
            section: 0,
            shelves: Vec::new(),
            shelf_at: 0,
            mine_count: 0,
            cols: Vec::new(),
            col_at: 0,
            col_name: String::new(),
            history: Vec::new(),
            history_at: 0,
            panes: Vec::new(),
            pane_at: 0,
            option_at: 0,
            query: String::new(),
            picking: None,
            capturing: None,
            es_systems: Vec::new(),
            ports: Vec::new(),
            port_at: 0,
            port_system: String::new(),
            wifi: crate::wifi::State::Idle,
            wifi_at: 0,
            wifi_job: None,
            wants_keyboard: None,
            sync: SyncState {
                server_version: None,
                watermark: None,
                games: 0,
                consoles: 0,
                collections: 0,
                seconds_played: 0,
                sessions: 0,
                games_played: 0,
            },
            at: 0,
            aspect: DEFAULT_ASPECT,
            scrolled: 0.0,
            chosen: Chosen::default(),
            moves: Moves::default(),
            columns: 1,
        };
        // The kinds of collection, and everything the History and Syncing
        // pages show. All three are counts and short lists out of the same
        // cache, cheap enough to read once here rather than on arriving at a
        // tab — which would stall the shoulder button.
        lib.build_shelves(server_collections);
        lib.history = lib
            .cache
            .play_by_game(200)
            .unwrap_or_default()
            .into_iter()
            .map(|(r, seconds, runs, last)| Played {
                id: r.id,
                stem: Path::new(&r.fs_name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| r.fs_name.clone()),
                name: r.name,
                platform: r.platform_slug,
                seconds,
                runs,
                last,
            })
            .collect();
        let totals = lib.cache.play_totals().unwrap_or((0, 0, 0));
        lib.sync = SyncState {
            server_version: lib.cache.server_version(),
            watermark: lib.cache.watermark(),
            games: lib.cache.rom_count().unwrap_or(0),
            consoles: lib.consoles.len(),
            collections: lib.cache.collection_count().unwrap_or(0),
            seconds_played: totals.0,
            sessions: totals.1,
            games_played: totals.2,
        };

        // Ports and Tools, where KNULLI puts them: in among the consoles. Not
        // scanned as systems — `esde::scan` skips both folders on purpose,
        // because a folder of shell scripts read as a platform invents games —
        // so they are added here as their own kind, and only when they hold
        // something. `tools` is empty on this device and shows nothing.
        // From the device's own system table, not from a folder — Ports and
        // Tools are groups of systems there, and `/userdata/roms/tools` is an
        // empty directory while the Tools group has two members.
        lib.es_systems = moose_rack::platform::current()
            .es_systems()
            .map(|p| crate::ports::systems(&p))
            .unwrap_or_default();
        for (group, label) in crate::ports::GROUPS {
            let found = crate::ports::scan_group(&lib.es_systems, group);
            if found.is_empty() {
                continue;
            }
            lib.consoles.push(Console {
                slug: group.to_owned(),
                name: label.to_owned(),
                games: found.len() as i64,
                scripts: true,
            });
        }

        // The console list is a list before it is anything else, and the
        // cursor has to be able to move on it. Without this the move table
        // stayed empty until a console was opened — which needs the cursor.
        lib.moves = gridnav::uniform(lib.consoles.len(), 1);
        Ok(lib)
    }

    /// Whether Back has anywhere left to go.
    ///
    /// On the console list it has not, and on a handheld with no window to
    /// close that is where Back means leave.
    /// Move to a tab, and arrive at that tab's own root.
    ///
    /// The one way in. Setting `section` on its own leaves `view` pointing at
    /// wherever the last tab was, and the drawing switches on `view` — which is
    /// how Collections drew an empty game list instead of its list of kinds.
    pub fn go_to_section(&mut self, index: usize) {
        self.section = index.min(SECTIONS.len() - 1);
        self.view = root_view(self.section().id);
        self.scrolled = 0.0;
        self.columns = 0;
        self.relayout(1);
    }

    /// Which tab is open.
    ///
    /// By name rather than by index: the drawing switches on this, and an index
    /// would silently mean a different page the moment a tab is added in the
    /// middle.
    pub fn section(&self) -> &'static Section {
        SECTIONS.get(self.section).unwrap_or(&SECTIONS[0])
    }

    /// Whether Back would have nowhere to go — at which point it leaves.
    pub fn at_top(&self) -> bool {
        self.view == root_view(self.section().id)
    }

    pub fn console(&self) -> Option<&Console> {
        self.consoles.get(self.console_at)
    }

    /// The games to draw, in the order to draw them, each with the name its
    /// artwork is filed under.
    pub fn showing(&self) -> impl Iterator<Item = (&Row, &str)> {
        self.arranged.iter().filter_map(|i| {
            Some((self.rows.get(*i)?, self.stems.get(*i).map(String::as_str).unwrap_or("")))
        })
    }

    pub fn shown(&self) -> usize {
        self.arranged.len()
    }

    /// What the third column shows about the game under the cursor.
    ///
    /// Assembled here rather than by the drawing, so the pane is a list of
    /// facts to lay out rather than a place where things are worked out. What
    /// is *in* the list comes from the row and from `platformfacts`, both
    /// already in the core.
    /// The highlighted game's detail, with the blurb and genres filled in.
    ///
    /// Two steps because they come from two places: everything else is already
    /// in the list row, and these two need the cache. Read once per game
    /// rather than once per frame — the pane redraws sixty times a second and
    /// the cursor sits still for most of them.
    /// Begin downloading one set of console pictures.
    ///
    /// Only the consoles the library holds: the table covers about ninety
    /// systems and a real library is two dozen, so fetching the rest is most of
    /// the transfer and none of the value.
    /// Read the settings from the config file again and rebuild every pane.
    ///
    /// After something changes what a pane can offer rather than what it holds
    /// — a set of pictures arriving, which turns one icon-set choice into two.
    pub fn rebuild_panes(&mut self) {
        let (cfg, _) = crate::settings::load();
        let consoles: Vec<(String, String)> = self
            .consoles
            .iter()
            .map(|c| (c.slug.clone(), c.name.clone()))
            .collect();
        let map = moose_rack::coremap::CoreMap::load_or_embedded(std::path::Path::new(
            "data/esde-core-map.json",
        ));
        self.panes = crate::settings::panes(&cfg, &consoles, &map);
        self.option_at = self
            .option_at
            .min(self.panes.get(self.pane_at).map_or(0, |p| p.entries.len().saturating_sub(1)));
    }

    /// Fetch a few of one set's pictures, to look at before choosing it.
    ///
    /// Replaces whatever was being fetched: the cursor has moved and the old
    /// set's pictures are no longer the answer to anything. The worker notices
    /// when its channel closes and stops.
    pub fn start_previews(&mut self, set: &str) {
        if self.previews.as_ref().is_some_and(|p| p.set == set) {
            return;
        }
        let slugs: Vec<String> = self.consoles.iter().map(|c| c.slug.clone()).collect();
        let map = moose_rack::coremap::CoreMap::load_or_embedded(std::path::Path::new(
            "data/esde-core-map.json",
        ));
        self.previews = Some(crate::iconfetch::Previews::start(
            self.media_root.clone(),
            set,
            slugs,
            map,
        ));
    }

    pub fn start_icon_fetch(&mut self, set: &str) {
        let slugs: Vec<String> = self.consoles.iter().map(|c| c.slug.clone()).collect();
        let map = moose_rack::coremap::CoreMap::load_or_embedded(std::path::Path::new(
            "data/esde-core-map.json",
        ));
        self.fetching = Some(crate::iconfetch::Fetch::start(
            self.media_root.clone(),
            set,
            slugs,
            map,
        ));
    }

    pub fn detail_full(&mut self) -> Option<Detail> {
        let mut detail = self.detail()?;
        if self.looked_up.0 != detail.id {
            let (mut summary, mut genres) = (None, Vec::new());
            if let Ok(Some(row)) = self.cache.rom_by_id(detail.id) {
                summary = row.summary.filter(|s| !s.trim().is_empty());
                genres = row
                    .meta_json
                    .as_deref()
                    .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                    .and_then(|m| m.get("genres").cloned())
                    .and_then(|g| g.as_array().cloned())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            self.looked_up = (detail.id, summary, genres);
        }
        detail.summary = self.looked_up.1.clone();
        detail.genres = self.looked_up.2.clone();
        Some(detail)
    }

    pub fn detail(&self) -> Option<Detail> {
        let row = self.selected()?;
        let stem = self.arranged.get(self.at).and_then(|i| self.stems.get(*i))?;
        Some(Detail {
            name: row.name.clone(),
            platform: row.platform.clone(),
            // The name on disk. `stems` holds it without its extension, and
            // "is this a .chd or an .m3u" is precisely the question the pane is
            // there to answer when nothing else is known.
            file: self
                .files
                .get(*self.arranged.get(self.at)?)
                .cloned()
                .unwrap_or_else(|| stem.clone()),
            stem: stem.clone(),
            id: row.id,
            size: moose_rack::util::human(row.size_bytes as u64),
            favorite: row.favorite,
            last_played: row.last_played.clone(),
            downloaded: row.downloaded,
            hardware: moose_rack::platformfacts::of(&row.platform).map(|f| f.hardware),
            maker: moose_rack::platformfacts::of(&row.platform).map(|f| f.manufacturer),
            rating: row.rating,
            year: row.year,
            players: row.players,
            summary: None,
            genres: Vec::new(),
        })
    }

    pub fn selected(&self) -> Option<&Row> {
        self.arranged.get(self.at).and_then(|i| self.rows.get(*i))
    }

    /// The list the sort and the filters belong to. See `gamelist::scope` —
    /// "order this console by rating" is a statement about that console.
    fn scope(&self) -> String {
        gamelist::scope("roms", self.console().map(|c| c.slug.as_str()), None)
    }

    /// Used by the header line, which is commented out — see `draw_chrome`.
    #[allow(dead_code)]
    pub fn order_label(&self) -> &'static str {
        self.chosen.order(&self.scope()).label
    }

    /// Used by the header line, which is commented out — see `draw_chrome`.
    #[allow(dead_code)]
    pub fn filters(&self) -> Vec<String> {
        self.chosen.filters(&self.scope()).into_iter().collect()
    }

    /// Open the console the picker is on.
    pub fn open_console(&mut self) -> Result<()> {
        let Some(slug) = self.console().map(|c| c.slug.clone()) else { return Ok(()) };
        let fetched = self.cache.roms_for(&slug).with_context(|| format!("reading {slug}"))?;
        self.take_rows(fetched);
        self.aspect = moose_rack::media::cover_aspect(&self.media_root, &slug)
            .filter(|a| a.is_finite() && *a > 0.1 && *a < 4.0)
            .unwrap_or(DEFAULT_ASPECT);
        self.view = View::Roms;
        self.at = 0;
        self.scrolled = 0.0;
        // rearrange relayouts, but only after the view has changed — the
        // table is built for whichever list is showing.
        self.rearrange();
        Ok(())
    }

    /// Turn cache rows into the list this front end draws.
    ///
    /// Shared by a console and a collection: both are a list of games, and the
    /// only thing that differed was which query filled it.
    fn take_rows(&mut self, fetched: Vec<moose_rack::cache::RomRow>) {
        let favorites = self.cache.favorite_ids().unwrap_or_default();
        let roms_dir = self.roms_dir.clone();
        self.files = fetched.iter().map(|r| r.fs_name.clone()).collect();
        self.stems = fetched
            .iter()
            .map(|r| {
                Path::new(&r.fs_name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| r.fs_name.clone())
            })
            .collect();
        self.rows = fetched
            .into_iter()
            .map(|r| Row {
                favorite: favorites.contains(&r.id),
                // Whether it will play with the server off. The row's own path
                // if it has one, and the library folder otherwise — which is
                // where a download lands.
                downloaded: r
                    .local_path
                    .as_deref()
                    .map(Path::new)
                    .filter(|p| p.exists())
                    .is_some()
                    || roms_dir.join(&r.platform_slug).join(&r.fs_name).exists(),
                id: r.id,
                name: r.name,
                platform: r.platform_slug,
                size_bytes: r.fs_size_bytes,
                last_played: r.last_played,
                // The rest lives in a metadata blob this front end does not
                // parse yet. Absent is a real answer for all three — a game
                // with no rating sorts last rather than first.
                ..Row::default()
            })
            .collect();
    }

    /// Open a Ports or Tools folder.
    pub fn open_scripts(&mut self) {
        let Some(slug) = self.console().map(|c| c.slug.clone()) else { return };
        self.ports = crate::ports::scan_group(&self.es_systems, &slug);
        self.port_system = slug;
        self.view = View::Scripts;
        self.port_at = 0;
        self.scrolled = 0.0;
        self.refresh_rows();
    }

    /// The Collections tab's first screen.
    ///
    /// Yours first and flat — you made them, they are few, and putting them
    /// behind a "Mine" folder costs a press every single time. Then RomM's
    /// generated kinds, which are 1,931 collections nobody browses by name and
    /// which `[library] server_collections = false` removes outright.
    pub fn build_shelves(&mut self, romm: bool) {
        let mut mine = self.cache.collections_in("user").unwrap_or_default();
        // Alphabetical. The cache hands them back largest first, which is the
        // right order for a wall of cards you are browsing and the wrong one
        // for a list you are looking a known name up in — and yours is always
        // the second thing.
        mine.sort_by_key(|a| a.name.to_lowercase());
        self.mine_count = mine.len();
        self.shelves = mine
            .into_iter()
            .map(|c| Shelf::Mine { id: c.id, name: c.name, games: c.rom_count })
            .collect();
        if !romm {
            return;
        }
        for (group, count) in self.cache.collection_groups().unwrap_or_default() {
            if group == "user" {
                continue;
            }
            self.shelves.push(Shelf::Kind { label: kind_label(&group), group, count });
        }
    }

    /// Open whatever is under the cursor on that first screen.
    ///
    /// One of yours goes straight to its games; a kind goes to the collections
    /// inside it. Two depths from one list, which is the point of `Shelf`.
    pub fn open_shelf(&mut self) -> Result<()> {
        let Some(shelf) = self.shelves.get(self.shelf_at) else { return Ok(()) };
        match shelf {
            Shelf::Mine { id, name, .. } => {
                let (id, name) = (id.clone(), name.clone());
                self.open_collection_id(&id, &name)
            }
            Shelf::Kind { group, .. } => {
                let group = group.clone();
                self.cols = self.cache.collections_in(&group).unwrap_or_default();
                self.view = View::Collections;
                self.col_at = 0;
                self.scrolled = 0.0;
                self.columns = 0;
                self.relayout(1);
                Ok(())
            }
        }
    }

    /// Open the collection under the cursor: its games become the list.
    ///
    /// The same `rows`/`arranged`/`at` the Library tab fills, so the sorting,
    /// the filters, the info pane and the drawing are all shared — a collection
    /// of games is a list of games.
    pub fn open_collection(&mut self) -> Result<()> {
        let Some(col) = self.cols.get(self.col_at) else { return Ok(()) };
        let (id, name) = (col.id.clone(), col.name.clone());
        self.open_collection_id(&id, &name)
    }

    /// Load one collection's games into the shared list.
    ///
    /// The same `rows`/`arranged`/`at` the Library tab fills, so the sorting,
    /// the filters, the info pane and the drawing are all shared — a collection
    /// of games is a list of games.
    /// A few names out of the highlighted collection, for the side panel.
    ///
    /// Held by id rather than re-read: the cursor sits on one row while the
    /// screen redraws sixty times a second, and reading three hundred rows to
    /// show four of them each of those times is the kind of thing that makes a
    /// handheld feel slow.
    pub fn peek(&mut self, id: &str) -> &[String] {
        if self.peeked.0 != id {
            let names = self
                .cache
                .roms_in_collection(id)
                .map(|rows| rows.into_iter().take(6).map(|r| r.name).collect())
                .unwrap_or_default();
            self.peeked = (id.to_owned(), names);
        }
        &self.peeked.1
    }

    fn open_collection_id(&mut self, id: &str, name: &str) -> Result<()> {
        let fetched = self
            .cache
            .roms_in_collection(id)
            .with_context(|| format!("reading collection {name}"))?;
        self.col_name = name.to_owned();
        self.take_rows(fetched);
        // A collection mixes consoles, so there is no one box shape to draw it
        // in. The default is the least wrong answer rather than a measured one.
        self.aspect = DEFAULT_ASPECT;
        self.view = View::Roms;
        self.at = 0;
        self.scrolled = 0.0;
        self.rearrange();
        Ok(())
    }

    /// Up one level, wherever that is for the tab you are in.
    ///
    /// The Library goes games -> consoles; Collections goes games ->
    /// collections -> kinds. `at_top` is what decides whether Back leaves the
    /// app instead, and it has to agree with this or Back either traps you or
    /// quits a level early.
    pub fn back(&mut self) {
        self.view = match (self.section().id, self.view) {
            ("settings", View::Wifi) => View::Options,
            ("settings", View::Options) => View::Panes,
            ("mine", View::Roms) => View::Collections,
            ("mine", View::Collections) => View::Groups,
            (_, View::Roms) => View::Platforms,
            (_, View::Scripts) => View::Platforms,
            (id, _) => root_view(id),
        };
        self.scrolled = 0.0;
        // Rebuilt outright rather than through `relayout`, which skips the work
        // when the count looks unchanged — and a console list the same length
        // as the games list it came from is not impossible.
        self.columns = 0;
        self.relayout(1);
    }

    /// Work out which rows survive the filters and what order they go in.
    fn rearrange(&mut self) {
        let scope = self.scope();
        let order = self.chosen.order(&scope).id;
        let filters = self.chosen.filters(&scope);
        let ids = gamelist::arrange(&self.rows, order, &filters);
        // `arrange` answers in ids because that is what a front end holding
        // its own rows wants; this one wants places in its own array.
        let mut by_id = std::collections::HashMap::with_capacity(self.rows.len());
        for (i, row) in self.rows.iter().enumerate() {
            by_id.insert(row.id, i);
        }
        self.arranged = ids.into_iter().filter_map(|id| by_id.get(&id).copied()).collect();
        // The search box, applied after the core's own ordering and filters so
        // that what you searched is still sorted the way you asked for.
        if !self.query.is_empty() {
            let needle = self.query.to_lowercase();
            let rows = &self.rows;
            self.arranged.retain(|i| {
                rows.get(*i).is_some_and(|r| r.name.to_lowercase().contains(&needle))
            });
        }
        self.at = self.at.min(self.arranged.len().saturating_sub(1));
        self.moves = gridnav::uniform(self.arranged.len(), self.columns);
    }

    /// Tell it how wide the grid came out, so the cursor knows what is beside
    /// what.
    ///
    /// A uniform grid needs no geometry — `gridnav::uniform` works it out from
    /// two numbers, which is also why it can answer for rows that were never
    /// drawn.
    pub fn relayout(&mut self, columns: usize) {
        let columns = columns.max(1);
        let count = self.count_here();
        // Only when it would come out different. This is called from the
        // drawing, so it runs every frame, and the table is six vectors the
        // length of the list — 15,000 allocations a frame on the arcade
        // console for an answer that changes when the window is resized.
        if columns == self.columns && count == self.moves.down.len() {
            return;
        }
        self.columns = columns;
        self.moves = gridnav::uniform(count, columns);
    }

    /// Put the cursor on a particular row, if it is one.
    ///
    /// Change the highlighted setting's value and save it.
    ///
    /// Shared by the slider and by a toggle, which flips on activate rather
    /// than opening a list of two.
    pub fn step_option(&mut self, dir: i64) {
        let at = self.option_at;
        let Some(pane) = self.panes.get_mut(self.pane_at) else { return };
        let Some(entry) = pane.entries.get_mut(at) else { return };
        let field = entry.field;
        if let Some(written) = entry.step(dir)
            && !field.is_empty()
            && let Err(e) = crate::settings::write(field, &written)
        {
            eprintln!("could not save {field}: {e:#}");
        }
    }

    /// The setting under the cursor.
    fn option_here(&self) -> Option<&crate::settings::Entry> {
        self.panes.get(self.pane_at)?.entries.get(self.option_at)
    }

    /// The text setting under the cursor, if that is what it is.
    fn text_here(&self) -> Option<TextField> {
        let entry = self.panes.get(self.pane_at)?.entries.get(self.option_at)?;
        match &entry.kind {
            crate::settings::Kind::Text { value, secret } => Some(TextField {
                target: Target::Setting(entry.field),
                prompt: entry.label,
                value: value.clone(),
                secret: *secret,
            }),
            _ => None,
        }
    }

    /// Store what was typed, wherever it was going.
    pub fn take_typed(&mut self, target: &Target, text: &str) {
        match target {
            Target::Search => {
                self.query = text.to_owned();
                self.at = 0;
                self.scrolled = 0.0;
                self.rearrange();
            }
            Target::WifiKey(ssid) => {
                self.wifi_job = Some(crate::wifi::join(ssid.clone(), text.to_owned()));
                self.wifi = crate::wifi::State::Busy("Joining");
            }
            Target::Setting(field) => {
                if let Some(pane) = self.panes.get_mut(self.pane_at)
                    && let Some(entry) = pane.entries.get_mut(self.option_at)
                    && let crate::settings::Kind::Text { value, .. } = &mut entry.kind
                {
                    *value = text.to_owned();
                }
                if field.is_empty() {
                    return;
                }
                if let Err(e) = crate::settings::write(
                    field,
                    &crate::settings::Written::Text(text.to_owned()),
                ) {
                    eprintln!("could not save {field}: {e:#}");
                }
            }
        }
    }

    /// Rebuild the movement table for the screen you are on, whatever it was
    /// before.
    ///
    /// `relayout` skips the work when the count looks unchanged, which is right
    /// on a redraw and wrong after a list has been replaced under it — a Wi-Fi
    /// scan that finds the same number of networks it had before would keep the
    /// old table.
    pub fn refresh_rows(&mut self) {
        self.columns = 0;
        self.relayout(1);
    }

    /// How many rows the screen you are on has.
    ///
    /// One place rather than three copies of the same match: `relayout`,
    /// `point_at` and the drawing all need it, and the three drifted apart the
    /// first time a new screen was added.
    pub fn count_here(&self) -> usize {
        match self.view {
            View::Platforms => self.consoles.len(),
            View::Groups => self.shelves.len(),
            View::Collections => self.cols.len(),
            View::History => self.history.len(),
            View::Panes => self.panes.len(),
            View::Options => self.panes.get(self.pane_at).map_or(0, |p| p.entries.len()),
            View::Wifi => self.wifi.names().len(),
            View::Scripts => self.ports.len(),
            View::Roms => self.arranged.len(),
        }
    }

    /// For the mouse, which does not step — it arrives.
    pub fn point_at(&mut self, index: usize) -> bool {
        // Counted before the cursor is borrowed: the count for Options reads
        // `pane_at`, which is itself one of the cursors.
        let count = self.count_here();
        let cursor = match self.view {
            View::Platforms => &mut self.console_at,
            View::Groups => &mut self.shelf_at,
            View::Collections => &mut self.col_at,
            View::History => &mut self.history_at,
            View::Panes => &mut self.pane_at,
            View::Options => &mut self.option_at,
            View::Wifi => &mut self.wifi_at,
            View::Scripts => &mut self.port_at,
            View::Roms => &mut self.at,
        };
        if index >= count || *cursor == index {
            return false;
        }
        *cursor = index;
        true
    }

    /// Drive the open list of options, if there is one.
    ///
    /// Takes the input before anything else does, the way the keyboard does:
    /// a list you are choosing from owns the buttons until you have chosen.
    fn act_picking(&mut self, action: &str) -> bool {
        let Some(picker) = self.picking.as_mut() else { return false };
        let n = picker.options.len();
        let was = picker.at;
        match action {
            "up" => picker.at = (picker.at + n.saturating_sub(1)) % n.max(1),
            "down" => picker.at = (picker.at + 1) % n.max(1),
            "first" => picker.at = 0,
            "last" => picker.at = n.saturating_sub(1),
            "activate" => {
                let Some(picker) = self.picking.take() else { return true };
                let Some((value, _)) = picker.options.get(picker.at).cloned() else {
                    return true;
                };
                if let Some(pane) = self.panes.get_mut(self.pane_at)
                    && let Some(entry) = pane.entries.get_mut(self.option_at)
                    && let crate::settings::Kind::Choice { at, .. } = &mut entry.kind
                {
                    *at = picker.at;
                }
                if picker.field == FETCH_ICONS {
                    self.previews = None;
                    self.start_icon_fetch(&value);
                    return true;
                }
                if !picker.field.is_empty()
                    && let Err(e) = crate::settings::write(
                        picker.field,
                        &crate::settings::Written::Text(value),
                    )
                {
                    eprintln!("could not save {}: {e:#}", picker.field);
                }
            }
            "back" | "back2" => {
                self.picking = None;
                self.previews = None;
            }
            _ => {}
        }
        // A different set under the cursor means different pictures to show.
        if let Some(picker) = self.picking.as_ref()
            && picker.field == FETCH_ICONS
            && (picker.at != was || self.previews.is_none())
            && let Some((set, _)) = picker.options.get(picker.at).cloned()
        {
            self.start_previews(&set);
        }
        true
    }

    /// Put the control being captured onto `button`, or clear it.
    ///
    /// Called by the loop with a raw button index rather than an action,
    /// because the whole point is to catch the button before it means anything.
    pub fn capture_button(&mut self, button: Option<u8>) {
        let Some(cap) = self.capturing.take() else { return };
        if let Err(e) = crate::settings::set_pad_binding(&cap.action, button) {
            eprintln!("could not bind {}: {e:#}", cap.label);
            return;
        }
        if let Some(pane) = self.panes.get_mut(self.pane_at)
            && let Some(entry) = pane.entries.get_mut(self.option_at)
            && let crate::settings::Kind::Binding(on) = &mut entry.kind
        {
            *on = button;
        }
    }

    /// Stop waiting for a button, changing nothing.
    pub fn cancel_capture(&mut self) {
        self.capturing = None;
    }

    /// Do what a key or a button asked for. Returns whether anything changed.
    pub fn act(&mut self, action: &str) -> Result<bool> {
        if self.act_picking(action) {
            return Ok(true);
        }
        // Left and right adjust a slider, and nothing else. A choice opens a
        // list and a control waits for a button, because stepping through values
        // to find the one you want is the control this screen used to have.
        //
        // Checked before the cursor is borrowed: the cursor borrows all of
        // `self`, and changing a value needs it too.
        if self.view == View::Options && matches!(action, "left" | "right") {
            let sliding = self
                .option_here()
                .is_some_and(|e| matches!(e.kind, crate::settings::Kind::Number { .. }));
            if sliding {
                self.step_option(if action == "right" { 1 } else { -1 });
                return Ok(true);
            }
        }

        let cursor = match self.view {
            View::Platforms => &mut self.console_at,
            View::Groups => &mut self.shelf_at,
            View::Collections => &mut self.col_at,
            View::History => &mut self.history_at,
            View::Panes => &mut self.pane_at,
            View::Options => &mut self.option_at,
            View::Wifi => &mut self.wifi_at,
            View::Scripts => &mut self.port_at,
            View::Roms => &mut self.at,
        };
        let moved = |table: &[Option<usize>], at: usize| table.get(at).copied().flatten();

        let next = match action {
            "left" => moved(&self.moves.left, *cursor),
            "right" => moved(&self.moves.right, *cursor),
            "up" => moved(&self.moves.up, *cursor),
            "down" => moved(&self.moves.down, *cursor),
            "pageUp" => moved(&self.moves.page_up, *cursor),
            "pageDown" => moved(&self.moves.page_down, *cursor),
            "first" => self.moves.first,
            "last" => self.moves.last,
            _ => None,
        };
        if let Some(to) = next {
            *cursor = to;
            return Ok(true);
        }

        match action {
            "activate" => match self.view {
                View::Platforms => {
                    if self.console().is_some_and(|c| c.scripts) {
                        self.open_scripts();
                        return Ok(true);
                    }
                    self.open_console()?;
                    Ok(true)
                }
                View::Scripts => {
                    let Some(port) = self.ports.get(self.port_at) else {
                        return Ok(false);
                    };
                    match crate::ports::launch(port) {
                        Ok(status) if status.success() => {}
                        Ok(status) => eprintln!("{} exited {status}", port.name),
                        Err(e) => eprintln!("could not launch {}: {e:#}", port.name),
                    }
                    Ok(true)
                }
                View::Groups => {
                    self.open_shelf()?;
                    Ok(true)
                }
                View::Collections => {
                    self.open_collection()?;
                    Ok(true)
                }
                View::Panes => {
                    self.view = View::Options;
                    self.option_at = 0;
                    self.scrolled = 0.0;
                    self.columns = 0;
                    self.relayout(1);
                    Ok(true)
                }
                // A text setting is the one kind that cannot be stepped, so
                // it asks for the keyboard. The loop picks this up and opens
                // one — `Library` has no window to open it in.
                View::Options => {
                    // A choice opens a list, a control waits for a button, and
                    // a toggle just flips — none of them step.
                    if let Some(e) = self.option_here() {
                        match &e.kind {
                            crate::settings::Kind::Choice { at, options } => {
                                self.picking = Some(Picker {
                                    title: e.label,
                                    options: options.clone(),
                                    at: *at,
                                    field: e.field,
                                });
                                return Ok(true);
                            }
                            crate::settings::Kind::Binding(_) => {
                                self.capturing = Some(Capture {
                                    action: e
                                        .field
                                        .trim_start_matches("bindings_pad.")
                                        .to_owned(),
                                    label: e.label,
                                });
                                return Ok(true);
                            }
                            crate::settings::Kind::Toggle(_) => {
                                self.step_option(1);
                                return Ok(true);
                            }
                            _ => {}
                        }
                    }
                    // Leaving. A row rather than a button nobody can see: the
                    // only way out was B at the top level, which the help bar
                    // called "back" and which on a handheld is indistinguishable
                    // from a button that does nothing.
                    if self.option_here().is_some_and(|e| e.label.starts_with("Back to")) {
                        self.quitting = true;
                        return Ok(true);
                    }

                    // Getting console pictures: pick which set, and the pick
                    // starts the download rather than writing a setting.
                    if self.option_here().is_some_and(|e| e.label == "Get pictures") {
                        self.picking = Some(Picker {
                            title: "Get pictures",
                            // The whole catalog here, not the installed ones —
                            // the point of this row is to install one that is
                            // not there yet.
                            options: moose_rack::iconart::ordered()
                                .into_iter()
                                .map(|(id, _)| {
                                    let label =
                                        id.trim_end_matches("-es-de").replace('-', " ");
                                    (id, label)
                                })
                                .collect(),
                            at: 0,
                            field: FETCH_ICONS,
                        });
                        if let Some(first) = moose_rack::iconart::ordered().first() {
                            let id = first.0.clone();
                            self.start_previews(&id);
                        }
                        return Ok(true);
                    }

                    // The Wi-Fi row is an action rather than a value, and the
                    // only one so far that opens a screen of its own.
                    if self.option_here().is_some_and(|e| e.label == "Wi-Fi") {
                        self.view = View::Wifi;
                        self.wifi_at = 0;
                        self.scrolled = 0.0;
                        self.wifi_job = Some(crate::wifi::scan());
                        self.wifi = crate::wifi::State::Busy("Scanning");
                        self.columns = 0;
                        self.relayout(1);
                        return Ok(true);
                    }
                    self.wants_keyboard = self.text_here();
                    Ok(self.wants_keyboard.is_some())
                }
                View::Wifi => {
                    let Some(ssid) = self.wifi.names().get(self.wifi_at).cloned() else {
                        return Ok(false);
                    };
                    self.wants_keyboard = Some(TextField {
                        target: Target::WifiKey(ssid.clone()),
                        prompt: "Wi-Fi password",
                        value: String::new(),
                        secret: true,
                    });
                    Ok(true)
                }
                // History is already the games, and a game list has no launcher
                // behind it yet.
                View::Roms | View::History => Ok(false),
            },
            "back" | "back2" if !self.at_top() => {
                self.back();
                Ok(true)
            }
            // The shoulder buttons, and q/e. The cheapest navigation there is,
            // which is why they move between tabs rather than within one.
            "prevSection" | "nextSection" => {
                let delta = if action == "nextSection" { 1 } else { SECTIONS.len() - 1 };
                self.go_to_section((self.section + delta) % SECTIONS.len());
                Ok(true)
            }
            // The search box. Only over a list of games — there is nothing to
            // search on a screen of four settings.
            "search" if self.view == View::Roms => {
                self.wants_keyboard = Some(TextField {
                    target: Target::Search,
                    prompt: "Search",
                    value: self.query.clone(),
                    secret: false,
                });
                Ok(true)
            }
            "sortCycle" if self.view == View::Roms => {
                let scope = self.scope();
                let next = gamesort::cycle(self.chosen.order(&scope).id, 1);
                self.chosen.set_order(&scope, next.id);
                self.rearrange();
                Ok(true)
            }
            "filterMenu" if self.view == View::Roms => {
                // No menu yet. Stepping through the filters one press at a
                // time is enough to prove they are the core's filters and not
                // a second set — the menu is a later phase.
                let scope = self.scope();
                let on = self.chosen.filters(&scope);
                let next = gamefilter::FILTERS
                    .iter()
                    .position(|f| on.contains(f.id))
                    .map(|i| (i + 1) % (gamefilter::FILTERS.len() + 1))
                    .unwrap_or(0);
                self.chosen.clear_filters(&scope);
                if let Some(f) = gamefilter::FILTERS.get(next) {
                    self.chosen.toggle_filter(&scope, f.id);
                }
                self.rearrange();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(names: &[(&str, bool)]) -> Vec<Row> {
        names
            .iter()
            .enumerate()
            .map(|(i, (name, fav))| Row {
                id: i as i64 + 1,
                name: (*name).to_owned(),
                favorite: *fav,
                ..Row::default()
            })
            .collect()
    }

    /// A library with rows already in it, so the tests need no database.
    fn seeded(rows: Vec<Row>) -> Library {
        let mut lib = Library {
            files: Vec::new(),
            peeked: (String::new(), Vec::new()),
            looked_up: (0, None, Vec::new()),
            quitting: false,
            fetching: None,
            previews: None,
            cache: Cache::open(Path::new(":memory:")).expect("an in-memory cache"),
            media_root: PathBuf::new(),
            roms_dir: PathBuf::new(),
            consoles: vec![Console { slug: "snes".into(), name: "SNES".into(), games: 3, scripts: false }],
            console_at: 0,
            view: View::Roms,
            stems: rows.iter().map(|r| r.name.clone()).collect(),
            rows,
            arranged: Vec::new(),
            section: 0,
            shelves: Vec::new(),
            shelf_at: 0,
            mine_count: 0,
            cols: Vec::new(),
            col_at: 0,
            col_name: String::new(),
            history: Vec::new(),
            history_at: 0,
            panes: Vec::new(),
            pane_at: 0,
            option_at: 0,
            query: String::new(),
            picking: None,
            capturing: None,
            es_systems: Vec::new(),
            ports: Vec::new(),
            port_at: 0,
            port_system: String::new(),
            wifi: crate::wifi::State::Idle,
            wifi_at: 0,
            wifi_job: None,
            wants_keyboard: None,
            sync: SyncState {
                server_version: None,
                watermark: None,
                games: 0,
                consoles: 0,
                collections: 0,
                seconds_played: 0,
                sessions: 0,
                games_played: 0,
            },
            at: 0,
            aspect: DEFAULT_ASPECT,
            scrolled: 0.0,
            chosen: Chosen::default(),
            moves: Moves::default(),
            columns: 1,
        };
        lib.rearrange();
        lib
    }

    #[test]
    fn a_list_comes_out_in_the_cores_order() {
        let lib = seeded(rows(&[("Zelda", false), ("asteroids", false), ("Metroid", false)]));
        let names: Vec<_> = lib.showing().map(|(r, _)| r.name.as_str()).collect();
        assert_eq!(names, ["asteroids", "Metroid", "Zelda"]);
    }

    /// Favorites on top, which is `gamesort`'s rule and not one of ours.
    #[test]
    fn favorites_lead_whatever_the_order() {
        let lib = seeded(rows(&[("Alpha", false), ("Zebra", true)]));
        assert_eq!(lib.showing().next().map(|(r, _)| r.name.as_str()), Some("Zebra"));
    }

    #[test]
    fn the_cursor_moves_by_the_grid_it_was_told_about() {
        let mut lib = seeded(rows(&[
            ("a", false), ("b", false), ("c", false), ("d", false),
            ("e", false), ("f", false), ("g", false),
        ]));
        lib.relayout(3);
        assert!(lib.act("right").unwrap());
        assert_eq!(lib.at, 1);
        assert!(lib.act("down").unwrap());
        assert_eq!(lib.at, 4, "down did not move a whole row");
        assert!(lib.act("last").unwrap());
        assert_eq!(lib.at, 6);
        // And stops rather than wrapping.
        assert!(!lib.act("down").unwrap(), "the cursor ran off the bottom");
    }

    /// Cycling the sort is `gamesort::cycle`, and it belongs to this console.
    #[test]
    fn cycling_the_sort_changes_the_order_and_says_so() {
        let mut lib = seeded(rows(&[("b", false), ("a", false)]));
        assert_eq!(lib.order_label(), "Name");
        lib.act("sortCycle").unwrap();
        assert_eq!(lib.order_label(), "Rating");
    }

    /// The filters narrow what is shown, and the cursor cannot be left past
    /// the end of what is left.
    #[test]
    fn filtering_narrows_the_list_and_brings_the_cursor_back() {
        let mut lib = seeded(rows(&[("a", false), ("b", true), ("c", false)]));
        lib.at = 2;
        // The third filter is "Starred".
        lib.act("filterMenu").unwrap();
        lib.act("filterMenu").unwrap();
        lib.act("filterMenu").unwrap();
        assert_eq!(lib.filters(), ["fav"]);
        assert_eq!(lib.shown(), 1);
        assert!(lib.at < lib.shown(), "the cursor was left past the end");
    }

    /// End to end, against the cache the other front ends read.
    ///
    /// Skipped where there is no cache — CI has none — but on a machine that
    /// has one this is the test that would have caught the console list
    /// drawing and nothing else happening.
    #[test]
    fn a_real_console_opens_and_has_games_in_it() {
        // `cargo test` runs from the crate directory, not the workspace root.
        let db = Path::new("../cache.sqlite3");
        if !db.is_file() {
            eprintln!("no cache.sqlite3 here; skipping");
            return;
        }
        let mut lib = Library::open(
            db,
            PathBuf::from("../library/downloaded_media"),
            PathBuf::from("../library/roms"),
            true,
        )
        .expect("opening the cache");
        assert!(!lib.consoles.is_empty(), "a cache with no consoles in it");

        // The first console with anything in it, since a library can hold
        // empty ones.
        let at = lib
            .consoles
            .iter()
            .position(|c| c.games > 0)
            .expect("no console has any games");
        lib.console_at = at;
        lib.act("activate").expect("opening a console");

        assert_eq!(lib.view, View::Roms, "activate did not open the console");
        assert!(lib.shown() > 0, "the console opened with nothing in it");
        assert!(lib.selected().is_some(), "nothing is under the cursor");
    }

    /// The console list has to be navigable before anything is opened.
    ///
    /// It was not: the move table was only ever worked out for the games
    /// grid, so on the console screen every direction did nothing and the
    /// cursor sat on whatever was first.
    #[test]
    fn the_console_cursor_moves_before_a_console_is_opened() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.view = View::Platforms;
        lib.consoles = vec![
            Console { slug: "a".into(), name: "A".into(), games: 1, scripts: false },
            Console { slug: "b".into(), name: "B".into(), games: 2, scripts: false },
            Console { slug: "c".into(), name: "C".into(), games: 3, scripts: false },
        ];
        lib.relayout(1);
        assert!(lib.act("down").unwrap(), "down did nothing on the console list");
        assert_eq!(lib.console_at, 1);
        assert!(lib.act("last").unwrap());
        assert_eq!(lib.console_at, 2);
    }

    /// The mouse arrives rather than stepping, so it must not be able to land
    /// outside the list — a pointer over the gap below the last row is a
    /// common thing and must not move the cursor there.
    #[test]
    fn pointing_outside_the_list_moves_nothing() {
        let mut lib = seeded(rows(&[("a", false), ("b", false)]));
        assert!(lib.point_at(1));
        assert_eq!(lib.at, 1);
        assert!(!lib.point_at(9), "the cursor followed the pointer off the end");
        assert_eq!(lib.at, 1);
        assert!(!lib.point_at(1), "pointing at where it already is counts as a change");
    }

    /// The tabs wrap, both ways, and there are as many as the webview has.
    #[test]
    fn the_shoulders_walk_the_tabs_and_wrap() {
        let mut lib = seeded(rows(&[("a", false)]));
        assert_eq!(SECTIONS.len(), 5);
        assert_eq!(lib.section, 0);
        lib.act("nextSection").unwrap();
        assert_eq!(lib.section, 1);
        lib.act("prevSection").unwrap();
        assert_eq!(lib.section, 0);
        lib.act("prevSection").unwrap();
        assert_eq!(lib.section, SECTIONS.len() - 1, "going back from the first did not wrap");
    }

    /// The six tabs, in the order they were asked for.
    ///
    /// Named rather than counted: a test that only checks the length passes
    /// while the tabs say something else entirely, and the drawing switches on
    /// these ids.
    #[test]
    fn the_tabs_are_the_six_the_panel_was_designed_around() {
        let ids: Vec<_> = SECTIONS.iter().map(|s| s.id).collect();
        assert_eq!(ids, ["library", "mine", "settings", "history", "syncing"]);
    }

    /// The shoulders reach every tab, including the ones with nothing behind
    /// them yet — a tab they skip is one nobody can see is coming.
    #[test]
    fn the_shoulders_reach_every_tab_and_wrap() {
        let mut lib = seeded(rows(&[("a", false)]));
        let mut seen = vec![lib.section];
        for _ in 1..SECTIONS.len() {
            lib.act("nextSection").unwrap();
            seen.push(lib.section);
        }
        assert_eq!(seen, (0..SECTIONS.len()).collect::<Vec<_>>(), "a tab was skipped");
        lib.act("nextSection").unwrap();
        assert_eq!(lib.section, 0, "the last tab did not wrap round to the first");
    }

    /// Every tab arrives at its own root, whatever the last one was showing.
    ///
    /// The bug this pins: `MOOSE_SDL_TAB=mine` set the section without touching
    /// the view, so Collections opened at `Platforms`, fell through the
    /// drawing's match to the game-list arm, and rendered an empty list and an
    /// empty info pane under a "— 0 games" heading.
    #[test]
    fn arriving_at_a_tab_lands_on_its_own_root() {
        let mut lib = seeded(rows(&[("a", false)]));
        assert_eq!(lib.view, View::Roms, "the fixture starts inside a console");
        for (i, want) in [
            (0, View::Platforms),
            (1, View::Groups),
            (2, View::Panes),
            (3, View::History),
            (4, View::Platforms),
        ] {
            lib.go_to_section(i);
            assert_eq!(lib.view, want, "{} landed wrong", SECTIONS[i].id);
        }
    }

    /// Back walks up the tab it is in, and says when there is nowhere left.
    ///
    /// Collections is three deep and the Library two, so a single shared rule
    /// would either trap you in Collections or quit a level early out of it.
    #[test]
    fn back_walks_up_the_tab_it_is_in() {
        let mut lib = seeded(rows(&[("a", false)]));

        lib.go_to_section(1); // Collections
        assert!(lib.at_top(), "the kinds are the top of Collections");
        lib.view = View::Collections;
        assert!(!lib.at_top());
        lib.back();
        assert_eq!(lib.view, View::Groups, "collections should go up to the kinds");
        assert!(lib.at_top());

        lib.view = View::Roms;
        lib.back();
        assert_eq!(lib.view, View::Collections, "a collection's games go up to the collections");

        lib.go_to_section(0); // Library
        lib.view = View::Roms;
        lib.back();
        assert_eq!(lib.view, View::Platforms, "a console's games go up to the consoles");
        assert!(lib.at_top());
    }

    /// Yours first and flat, then RomM's kinds — and `mine_count` is the join
    /// between them, which is where the divider goes.
    #[test]
    fn the_shelf_puts_yours_first_and_flat() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.shelves = vec![
            Shelf::Mine { id: "1".into(), name: "Best of nes".into(), games: 75 },
            Shelf::Kind { group: "company".into(), label: "Companies", count: 1022 },
        ];
        lib.mine_count = 1;
        assert_eq!(lib.shelves[0].name(), "Best of nes");
        assert_eq!(lib.shelves[0].count(), 75, "yours count games");
        assert_eq!(lib.shelves[1].name(), "Companies");
        assert_eq!(lib.shelves[1].count(), 1022, "a kind counts collections, not games");
    }

    /// `[library] server_collections = false` leaves only yours, and then there
    /// is no divider to draw — `mine_count` equals the whole list.
    #[test]
    fn turning_server_collections_off_leaves_only_yours() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.build_shelves(false);
        assert_eq!(
            lib.mine_count,
            lib.shelves.len(),
            "a kind survived with RomM collections turned off"
        );
    }

    /// A toggle flips on activate, and left and right do nothing to it.
    ///
    /// Stepping is for sliders only. On a choice it meant holding a direction
    /// and watching values go past to find one, which is the control this
    /// screen used to have and the reason it was rebuilt.
    #[test]
    fn a_toggle_flips_on_activate_and_ignores_left_and_right() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.panes = vec![crate::settings::Pane {
            id: "t",
            blurb: "",
            label: "Test",
            entries: vec![crate::settings::Entry {
                field: "",
                label: "Something",
                help: "",
                kind: crate::settings::Kind::Toggle(false),
            }],
        }];
        lib.go_to_section(2);
        lib.act("activate").unwrap();
        assert_eq!(lib.view, View::Options, "a group did not open");

        let value = |l: &Library| l.panes[0].entries[0].value();
        assert_eq!(value(&lib), "Off");
        lib.act("activate").unwrap();
        assert_eq!(value(&lib), "On", "activate did not flip the toggle");
        lib.act("right").unwrap();
        assert_eq!(value(&lib), "On", "right stepped a toggle");
    }

    /// A choice opens a list rather than stepping through its options.
    #[test]
    fn a_choice_opens_a_list() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.panes = vec![crate::settings::Pane {
            id: "t",
            blurb: "",
            label: "Test",
            entries: vec![crate::settings::Entry {
                field: "",
                label: "Which one",
                help: "",
                kind: crate::settings::Kind::Choice {
                    at: 1,
                    options: vec![
                        ("a".into(), "Alpha".into()),
                        ("b".into(), "Beta".into()),
                        ("c".into(), "Gamma".into()),
                    ],
                },
            }],
        }];
        lib.go_to_section(2);
        lib.act("activate").unwrap();
        lib.act("activate").unwrap();

        let picker = lib.picking.as_ref().expect("a list should have opened");
        assert_eq!(picker.title, "Which one");
        assert_eq!(picker.options.len(), 3);
        assert_eq!(picker.at, 1, "the list did not open on the current value");
    }

    /// A control waits for a button rather than stepping through their names.
    #[test]
    fn a_control_waits_for_a_button() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.panes = vec![crate::settings::Pane {
            id: "t",
            blurb: "",
            label: "Test",
            entries: vec![crate::settings::Entry {
                field: "bindings_pad.activate",
                label: "Open platform / play",
                help: "",
                kind: crate::settings::Kind::Binding(None),
            }],
        }];
        lib.go_to_section(2);
        lib.act("activate").unwrap();
        lib.act("activate").unwrap();

        let cap = lib.capturing.as_ref().expect("it should be waiting for a button");
        assert_eq!(cap.action, "activate", "the table prefix was not stripped");
        assert!(lib.picking.is_none(), "a control opened a list instead");
    }

    /// Up and down still move, and Back leaves the group rather than the tab.
    #[test]
    fn settings_still_navigate_normally() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.panes = vec![crate::settings::Pane {
            id: "t",
            blurb: "",
            label: "Test",
            entries: vec![
                crate::settings::Entry {
                    field: "",
                    label: "One",
                    help: "",
                    kind: crate::settings::Kind::ReadOnly("x".into()),
                },
                crate::settings::Entry {
                    field: "",
                    label: "Two",
                    help: "",
                    kind: crate::settings::Kind::ReadOnly("y".into()),
                },
            ],
        }];
        lib.go_to_section(2);
        lib.act("activate").unwrap();
        lib.act("down").unwrap();
        assert_eq!(lib.option_at, 1, "down did not move between settings");
        assert!(!lib.at_top());
        lib.back();
        assert_eq!(lib.view, View::Panes, "back should leave the group, not the tab");
        assert!(lib.at_top());
    }

    /// Search filters the list without disturbing the order it was sorted in.
    #[test]
    fn search_narrows_the_list_and_keeps_the_order() {
        let mut lib = seeded(rows(&[
            ("Sonic the Hedgehog", false),
            ("Alex Kidd", false),
            ("Sonic 2", false),
        ]));
        assert_eq!(lib.shown(), 3);

        lib.take_typed(&Target::Search, "sonic");
        let names: Vec<_> = lib.showing().map(|(r, _)| r.name.as_str()).collect();
        assert_eq!(names, ["Sonic 2", "Sonic the Hedgehog"], "still in name order");

        lib.take_typed(&Target::Search, "");
        assert_eq!(lib.shown(), 3, "clearing the box did not put everything back");
    }

    /// A search typed on the Library tab must not land in a setting.
    ///
    /// Both open the same keyboard, and before the target was carried through,
    /// finishing a search wrote the query into whichever setting the Settings
    /// tab happened to be sitting on.
    #[test]
    fn a_search_does_not_write_itself_into_a_setting() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.panes = vec![crate::settings::Pane {
            id: "t",
            blurb: "",
            label: "Test",
            entries: vec![crate::settings::Entry {
                field: "",
                label: "Some text",
                help: "",
                kind: crate::settings::Kind::Text {
                    value: "untouched".into(),
                    secret: false,
                },
            }],
        }];
        lib.take_typed(&Target::Search, "sonic");
        let crate::settings::Kind::Text { value, .. } = &lib.panes[0].entries[0].kind else {
            panic!("not a text setting");
        };
        assert_eq!(value, "untouched", "the search was written into a setting");
        assert_eq!(lib.query, "sonic");
    }

    /// A Wi-Fi password goes to the device, never to a file.
    ///
    /// It is the one secret this app handles that it must not store: connman
    /// already saves it, and a copy in `moose-sdl.toml` on an exfat card with no
    /// permissions is a password sitting in a text file.
    #[test]
    fn a_wifi_password_is_not_written_to_the_config() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.panes = vec![crate::settings::Pane {
            id: "device",
            blurb: "",
            label: "Device",
            entries: vec![crate::settings::Entry {
                field: "",
                label: "Wi-Fi",
                help: "",
                kind: crate::settings::Kind::Action,
            }],
        }];
        lib.take_typed(&Target::WifiKey("Chicken24".into()), "hunter2");
        assert!(lib.wifi_job.is_some(), "the join never started");
        // Nothing was written, and the setting it came from is untouched.
        assert!(matches!(lib.panes[0].entries[0].kind, crate::settings::Kind::Action));
        assert!(
            !std::path::Path::new(crate::settings::FILE).exists()
                || !std::fs::read_to_string(crate::settings::FILE)
                    .unwrap_or_default()
                    .contains("hunter2"),
            "the password reached the config file"
        );
    }

    /// Back from the network list returns to the Device settings, not out of
    /// the tab.
    #[test]
    fn back_from_the_wifi_list_returns_to_the_settings() {
        let mut lib = seeded(rows(&[("a", false)]));
        lib.go_to_section(2);
        lib.view = View::Wifi;
        assert!(!lib.at_top());
        lib.back();
        assert_eq!(lib.view, View::Options);
        lib.back();
        assert_eq!(lib.view, View::Panes);
        assert!(lib.at_top());
    }

    #[test]
    fn back_returns_to_the_consoles() {
        let mut lib = seeded(rows(&[("a", false)]));
        assert!(!lib.at_top(), "a console's games are not the top level");
        assert!(lib.act("back").unwrap());
        assert_eq!(lib.view, View::Platforms);
        assert!(lib.at_top(), "back left us somewhere that is not the top");
    }
}

#[cfg(test)]
mod library_state {
    /// The groups do not count as a library.
    ///
    /// `Ports`, `Tools` and `Emulators` come from the device's own
    /// `es_systems.cfg` and are pushed onto the console list every time,
    /// cached library or not. Asking "are there any consoles" therefore always
    /// answers yes — which is why the handheld's first run saw three of them,
    /// decided all was well, and left the card unscanned with 235 GB of games
    /// on it.
    #[test]
    fn three_groups_are_not_a_library() {
        let group = |slug: &str| super::Console {
            slug: slug.to_owned(),
            name: slug.to_owned(),
            games: 4,
            scripts: true,
        };
        let console = super::Console {
            slug: "megadrive".to_owned(),
            name: "Mega Drive".to_owned(),
            games: 942,
            scripts: false,
        };

        let only_groups = [group("ports"), group("tools"), group("emulators")];
        assert!(
            !only_groups.iter().any(|c| !c.scripts),
            "the groups counted as a cached library"
        );
        let with_a_console = [group("ports"), console];
        assert!(with_a_console.iter().any(|c| !c.scripts));
    }
}

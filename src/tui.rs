//! Stage 2 — terminal library browser.
//!
//! Drill-down: platforms → games. Reads only the local SQLite cache, so it is
//! usable with the server unreachable. Enter launches a game when the ROM is
//! present locally.

use std::io::{Stdout, stdout};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};

use crate::api;
use crate::cache::{Cache, PlatformRow, RomRow};
use crate::coremap::CoreMap;
use crate::download;
use crate::retroarch::RetroArch;
use crate::util::human;

/// Live state of a download, shared between the worker task and the UI.
#[derive(Default)]
struct Progress {
    done: u64,
    total: u64,
    label: String,
    finished: Option<Result<String, String>>,
}

type Term = Terminal<CrosstermBackend<Stdout>>;

/// What an automatic sync decided about a launch.
enum SyncOutcome {
    /// Carry on; the string is a note worth showing.
    Ok(Option<String>),
    /// Do not launch — a conflict needs answering first.
    Blocked(String),
}

#[derive(PartialEq)]
enum View {
    Platforms,
    Roms,
}

pub struct App {
    view: View,
    platforms: Vec<PlatformRow>,
    roms: Vec<RomRow>,
    /// Indices into `roms` surviving the current filter.
    filtered: Vec<usize>,
    platform_state: ListState,
    rom_state: ListState,
    filter: String,
    searching: bool,
    status: String,
    local_roms: PathBuf,
    ra: Option<RetroArch>,
    map: CoreMap,
    core_overrides: std::collections::BTreeMap<String, String>,
    core_per_game: std::collections::BTreeMap<String, String>,
    user_ra_cfg: PathBuf,
    /// Where RetroArch is told to keep saves and states, from `[saves] root`.
    saves_root: PathBuf,
    shaders_enabled: bool,
    achievements: crate::achievements::Settings,
    auto_sync: bool,
    /// Own connection to the metadata cache, for resolving saves to ROMs.
    ///
    /// A second connection rather than borrowing the caller's: `App` outlives
    /// the borrow, and SQLite is happy with two readers of the same file.
    /// `None` when it could not be opened, which only costs the sync.
    save_cache: Option<Cache>,
    shader_overrides: std::collections::BTreeMap<String, String>,
    quit: bool,
    /// Set while a download is in flight; cleared once the UI reports it.
    progress: Option<Arc<Mutex<Progress>>>,
    /// Set when a download was started by Enter rather than `d`: launch this
    /// ROM once the bytes land.
    launch_when_done: Option<RomRow>,
    client: Option<Arc<api::Client>>,
    rt: tokio::runtime::Handle,
}

impl App {
    pub fn new(
        cache: &Cache,
        local_roms: PathBuf,
        ra: Option<RetroArch>,
        map: CoreMap,
        client: Option<Arc<api::Client>>,
        rt: tokio::runtime::Handle,
    ) -> Result<Self> {
        // Alphabetically, through the same ordering the GUI's console column
        // uses. The cache hands these back in the server's order, which is by
        // size — so this list used to open on whichever console happened to
        // have the most ROMs in it.
        let platforms = {
            let rows = cache.platforms()?;
            let order = crate::pickorder::by_name(
                &rows
                    .iter()
                    .map(|p| crate::pickorder::PickerRow {
                        name: p.display_name.clone(),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>(),
            );
            let mut rows: Vec<Option<PlatformRow>> = rows.into_iter().map(Some).collect();
            order.into_iter().filter_map(|i| rows[i].take()).collect::<Vec<_>>()
        };
        let mut platform_state = ListState::default();
        if !platforms.is_empty() {
            platform_state.select(Some(0));
        }
        let status = match &ra {
            Some(r) => format!("RetroArch: {}", r.root.display()),
            None => "RetroArch not found — launching disabled".to_owned(),
        };
        Ok(Self {
            view: View::Platforms,
            platforms,
            roms: Vec::new(),
            filtered: Vec::new(),
            platform_state,
            rom_state: ListState::default(),
            filter: String::new(),
            searching: false,
            status,
            local_roms,
            ra,
            map,
            core_overrides: crate::config::Config::load()
                .map(|c| c.cores.overrides)
                .unwrap_or_default(),
            core_per_game: crate::config::Config::load()
                .map(|c| c.cores.per_game)
                .unwrap_or_default(),
            user_ra_cfg: crate::config::Config::load()
                .map(|c| c.user_retroarch_config())
                .unwrap_or_default(),
            saves_root: crate::config::Config::load()
                .map(|c| crate::util::expand_tilde(&c.saves.root))
                .unwrap_or_default(),
            shaders_enabled: crate::config::Config::load()
                .map(|c| c.shaders.enabled)
                .unwrap_or(true),
            shader_overrides: crate::config::Config::load()
                .map(|c| c.shaders.by_platform)
                .unwrap_or_default(),
            achievements: crate::config::Config::load()
                .map(|c| c.achievements.settings())
                .unwrap_or_default(),
            auto_sync: crate::config::Config::load()
                .map(|c| c.saves.auto_sync)
                .unwrap_or(true),
            save_cache: Cache::open(Path::new("cache.sqlite3")).ok(),
            quit: false,
            progress: None,
            launch_when_done: None,
            client,
            rt,
        })
    }

    fn selected_platform(&self) -> Option<&PlatformRow> {
        self.platforms.get(self.platform_state.selected()?)
    }

    fn selected_rom(&self) -> Option<&RomRow> {
        let i = *self.filtered.get(self.rom_state.selected()?)?;
        self.roms.get(i)
    }

    /// Local path for a ROM, if it was staged into `library/`.
    /// A folder ROM is a directory, so this cannot be a file-only test: a
    /// multi-disc game that had downloaded fine still reported itself missing.
    /// `launch::plan` resolves the directory to the playlist inside it.
    fn local_path(&self, rom: &RomRow) -> Option<PathBuf> {
        let p = self.local_roms.join(&rom.platform_slug).join(&rom.fs_name);
        (p.is_file() || p.is_dir()).then_some(p)
    }

    fn apply_filter(&mut self) {
        // The same rule the GUI's page filter uses — case and stray space
        // ignored, matching anywhere in the name rather than at the start. The
        // filename as well as the title, which is the one thing this list can
        // usefully search that a grid of covers cannot.
        self.filtered = self
            .roms
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                crate::pagefilter::matches(&r.name, &self.filter)
                    || crate::pagefilter::matches(&r.fs_name, &self.filter)
            })
            .map(|(i, _)| i)
            .collect();
        self.rom_state
            .select(if self.filtered.is_empty() { None } else { Some(0) });
    }

    fn enter_platform(&mut self, cache: &Cache) -> Result<()> {
        let Some(p) = self.selected_platform().cloned() else {
            return Ok(());
        };
        self.roms = cache.roms_for(&p.fs_slug)?;
        self.filter.clear();
        self.apply_filter();
        self.view = View::Roms;
        Ok(())
    }

    fn move_selection(&mut self, delta: isize) {
        let (state, len) = match self.view {
            View::Platforms => (&mut self.platform_state, self.platforms.len()),
            View::Roms => (&mut self.rom_state, self.filtered.len()),
        };
        if len == 0 {
            return;
        }
        let cur = state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1);
        state.select(Some(next as usize));
    }

    /// Resolve the core for a ROM, preferring the mapped default but falling
    /// back to any installed alternative.
    fn resolve_core(&self, ra: &RetroArch, platform: &str) -> Option<String> {
        self.resolve_core_for(ra, platform, None)
    }

    /// As `resolve_core`, honouring a per-game override when the file is known.
    fn resolve_core_for(
        &self,
        ra: &RetroArch,
        platform: &str,
        fs_name: Option<&str>,
    ) -> Option<String> {
        crate::coremap::resolve_core_for(
            &self.map,
            &self.core_overrides,
            &self.core_per_game,
            platform,
            fs_name,
            |c| ra.has_core(c),
        )
    }

    /// Enter: play. Downloads first if the ROM is not local yet.
    fn play(&mut self, term: &mut Term) -> Result<()> {
        let Some(rom) = self.selected_rom().cloned() else {
            return Ok(());
        };
        if self.local_path(&rom).is_some() {
            return self.launch_rom(&rom, term);
        }
        if self.client.is_none() {
            self.status = "not downloaded, and no server connection".into();
            return Ok(());
        }
        self.launch_when_done = Some(rom);
        self.start_download();
        Ok(())
    }

    fn launch_rom(&mut self, rom: &RomRow, term: &mut Term) -> Result<()> {
        let rom = rom.clone();
        let Some(ra) = &self.ra else {
            self.status = "RetroArch not found".into();
            return Ok(());
        };
        let Some(path) = self.local_path(&rom) else {
            self.status = format!("not downloaded: {}", rom.fs_name);
            return Ok(());
        };

        // Hand the terminal back before spawning, or the emulator and the TUI
        // fight over it; restore afterwards.
        // Overrides neutralise handheld-oriented settings (pause-when-unfocused,
        // bezel overlays) without touching the user's retroarch.cfg.
        let cfg_core_overrides = self.core_overrides.clone();
        let req = crate::launch::Request {
            autofire: crate::tweaks::AutoFire::Off,
            save_state_on_exit: false,
            autofire_hz: 5,
            window_decorations: true,
            fit_window: true,
            mirror_players: true,
            entry_slot: None,
            rom: &path,
            platform: &rom.platform_slug,
            fs_name: &rom.fs_name,
            library_root: self.local_roms.parent().unwrap_or(Path::new(".")),
            user_cfg: &self.user_ra_cfg,
            shaders_enabled: self.shaders_enabled,
            shader_overrides: &self.shader_overrides,
            core_overrides: &cfg_core_overrides,
            core_per_game: &self.core_per_game,
            core_override: None,
            // No display detection here either; motion shaders are a GUI
            // setting and the TUI leaves them off.
            motion_shader: None,
            refresh_hz: None,
            // The TUI has no gamepad detection, so hotkeys fall back to the
            // best profile for this OS's input driver.
            pad: None,
            achievements: Some(&self.achievements),
            // The TUI has no Systems screen to switch it on from.
            lightgun: &std::collections::BTreeMap::new(),
            // No way to ask a terminal how big the display is; leave the
            // emulator's own window settings alone.
            screen: None,
            saves_root: Some(&self.saves_root),
        };
        let plan = match crate::launch::plan(ra, &self.map, &req) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("{}: {e}", rom.name);
                return Ok(());
            }
        };

        // Cores, shaders and BIOS, fetched if missing. Same as the other two
        // frontends: a missing BIOS is a black screen, and the TUI has even
        // less room than the GUI to explain one.
        if let (Some(client), Ok(cfg)) = (self.client.clone(), crate::config::Config::load()) {
            let core = self.resolve_core_for(ra, &rom.platform_slug, Some(&rom.fs_name));
            let lib = std::path::PathBuf::from(&cfg.library.local_root);
            let root = ra.root.clone();
            let ra_for_fetch = crate::retroarch::RetroArch::locate_in(&[root.display().to_string()]);
            if let (Some(core), Ok(ra2)) = (core, ra_for_fetch) {
                let slug = rom.platform_slug.clone();
                self.rt.block_on(async {
                    let _ = crate::cores::ensure(client.http(), &ra2, &core).await;
                    let _ = crate::bios::ensure(&client, &lib, &core, &slug).await;
                    if cfg.shaders.enabled {
                        let _ = crate::shaders::ensure_pack(client.http(), &ra2).await;
                    }
                });
            }
        }

        // Pull before, push after — the same automatic sync the GUI does. A
        // conflict refuses the launch here too, but the TUI has no dialog to
        // resolve it in, so it says where to.
        match self.sync_saves(&rom, crate::savesync::When::BeforeLaunch) {
            SyncOutcome::Blocked(msg) => {
                self.status = msg;
                return Ok(());
            }
            SyncOutcome::Ok(Some(note)) => self.status = note,
            SyncOutcome::Ok(None) => {}
        }

        restore_terminal(term)?;
        let result = plan.run(ra, false);
        setup_terminal(term)?;

        let played = match result {
            Ok(s) if s.success() => format!("{} exited cleanly", rom.name),
            Ok(s) => format!("{} exited with {}", rom.name, s),
            Err(e) => format!("launch failed: {e}"),
        };
        // After the fact there is nothing to ask — the game has been played, so
        // anything that went wrong is reported rather than blocking.
        self.status = match self.sync_saves(&rom, crate::savesync::When::AfterExit) {
            SyncOutcome::Ok(Some(note)) => format!("{played} — {note}"),
            SyncOutcome::Blocked(msg) => format!("{played} — {msg}"),
            SyncOutcome::Ok(None) => played,
        };
        Ok(())
    }

    /// One half of the automatic save sync, run on the tokio runtime.
    ///
    /// Blocking is fine here: the TUI is idle at this point either way, and a
    /// launch is about to take over the terminal.
    fn sync_saves(&self, rom: &RomRow, when: crate::savesync::When) -> SyncOutcome {
        if !self.auto_sync {
            return SyncOutcome::Ok(None);
        }
        let (Some(client), Some(ra)) = (self.client.clone(), self.ra.as_ref()) else {
            return SyncOutcome::Ok(None);
        };
        let library_root = self.local_roms.parent().unwrap_or(Path::new(".")).to_path_buf();

        let Some(cache) = self.save_cache.as_ref() else {
            return SyncOutcome::Ok(None);
        };
        let candidates = match crate::savesync::scan_for_rom(
            cache,
            &self.map,
            &ra.root,
            &rom.fs_name,
        ) {
            Ok(c) => c,
            Err(e) => return SyncOutcome::Ok(Some(format!("saves: could not scan ({e})"))),
        };

        let root = ra.root.clone();
        let result = self.rt.block_on(async move {
            crate::savesync::run_all(&client, &candidates, &root, Path::new("."), &library_root).await
        });

        match result {
            Ok(summary) if !summary.conflicts.is_empty() && when == crate::savesync::When::BeforeLaunch => {
                SyncOutcome::Blocked(format!(
                    "save conflict on {} — resolve it in the app, or run `moose-rack sync-saves`",
                    summary.conflicts[0].file_name
                ))
            }
            Ok(summary) => SyncOutcome::Ok(crate::savesync::describe(when, &summary)),
            // Not fatal: a server that is off must not stop you playing.
            Err(e) => SyncOutcome::Ok(Some(format!(
                "saves NOT synced ({}) — progress stays on this machine",
                e.to_string().lines().next().unwrap_or("server unreachable")
            ))),
        }
    }

    /// Kick off a download on the tokio runtime. The UI keeps redrawing while
    /// it runs; progress arrives through the shared `Progress`.
    fn start_download(&mut self) {
        if self.progress.is_some() {
            self.status = "a download is already running".into();
            return;
        }
        let Some(rom) = self.selected_rom().cloned() else { return };
        let Some(client) = self.client.clone() else {
            self.status = "no server connection — check config.toml".into();
            return;
        };
        if self.local_path(&rom).is_some() {
            self.status = format!("{} is already downloaded", rom.name);
            return;
        }

        let shared = Arc::new(Mutex::new(Progress {
            total: rom.fs_size_bytes.max(0) as u64,
            label: rom.name.clone(),
            ..Default::default()
        }));
        self.progress = Some(shared.clone());

        let roms_dir = self.local_roms.clone();
        self.rt.spawn(async move {
            // Folder ROMs verify per member; the rom-level hash is a
            // filesystem-ordered composite we cannot reproduce.
            let members = if rom.multi_file {
                client.member_hashes(rom.id).await
            } else {
                Vec::new()
            };

            let target = download::Target {
                rom_id: rom.id,
                members: &members,
                fs_name: &rom.fs_name,
                platform_slug: &rom.platform_slug,
                expected_size: (rom.fs_size_bytes > 0).then_some(rom.fs_size_bytes as u64),
                md5: rom.md5_hash.as_deref(),
                sha1: rom.sha1_hash.as_deref(),
                multi_file: rom.multi_file,
            };
            let sink = shared.clone();
            let result = download::fetch(
                client.http(),
                client.base(),
                client.auth(),
                &target,
                &roms_dir,
                |done, total| {
                    if let Ok(mut p) = sink.lock() {
                        p.done = done;
                        if total > 0 {
                            p.total = total;
                        }
                    }
                },
            )
            .await;

            if let Ok(mut p) = shared.lock() {
                p.finished = Some(match result {
                    Ok(download::Outcome::Downloaded { verified, .. }) => {
                        Ok(format!("{} downloaded — {}", rom.name, verified.describe()))
                    }
                    Ok(_) => Ok(format!("{} downloaded", rom.name)),
                    Err(e) => Err(format!("{}: {e}", rom.name)),
                });
            }
        });
        self.status = "starting download…".into();
    }

    /// Retire a finished download. Returns a ROM to launch when the download
    /// was started by Enter and succeeded.
    fn poll_download(&mut self) -> Option<RomRow> {
        let done = self
            .progress
            .as_ref()
            .and_then(|p| p.lock().ok().and_then(|g| g.finished.clone()));
        let outcome = done?;
        self.progress = None;
        match outcome {
            Ok(msg) => {
                self.status = msg;
                // Only auto-launch if this download was an Enter, not a `d`.
                self.launch_when_done.take()
            }
            Err(msg) => {
                self.status = format!("download failed — {msg}");
                self.launch_when_done = None;
                None
            }
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers, cache: &Cache, term: &mut Term)
        -> Result<()>
    {
        // Search box swallows most keys while active.
        if self.searching {
            match code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.filter.clear();
                    self.apply_filter();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.apply_filter();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.apply_filter();
                }
                _ => {}
            }
            return Ok(());
        }

        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::Home => self.move_selection(isize::MIN / 2),
            KeyCode::End => self.move_selection(isize::MAX / 2),
            KeyCode::Char('d') if self.view == View::Roms => self.start_download(),
            KeyCode::Char('/') if self.view == View::Roms => {
                self.searching = true;
                self.filter.clear();
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => match self.view {
                View::Platforms => self.enter_platform(cache)?,
                View::Roms => self.play(term)?,
            },
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') if self.view == View::Roms => {
                self.view = View::Platforms;
                self.filter.clear();
            }
            _ => {}
        }
        Ok(())
    }
}

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(if app.searching { 3 } else { 0 }),
            Constraint::Length(if app.progress.is_some() { 3 } else { 0 }),
            Constraint::Length(3),
        ])
        .split(f.area());

    match app.view {
        View::Platforms => draw_platforms(f, app, chunks[0]),
        View::Roms => draw_roms(f, app, chunks[0]),
    }

    if app.searching {
        let p = Paragraph::new(Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(app.filter.clone()),
            Span::styled("█", Style::default().fg(Color::DarkGray)),
        ]))
        .block(Block::default().borders(Borders::ALL).title(" search "));
        f.render_widget(p, chunks[1]);
    }

    if let Some(shared) = &app.progress
        && let Ok(p) = shared.lock()
    {
        let ratio = if p.total > 0 {
            (p.done as f64 / p.total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(format!(
                " downloading {} ",
                p.label.chars().take(48).collect::<String>()
            )))
            .gauge_style(Style::default().fg(Color::Green))
            .ratio(ratio)
            .label(format!("{} / {}", human(p.done), human(p.total)));
        f.render_widget(gauge, chunks[2]);
    }

    let help = match app.view {
        View::Platforms => "↑↓/jk move   ⏎ open   q quit",
        View::Roms => "↑↓/jk move   ⏎ play (downloads first)   d download only   / search   esc back   q quit",
    };
    let footer = Paragraph::new(vec![
        Line::from(Span::styled(
            app.status.clone(),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(help, Style::default().fg(Color::DarkGray))),
    ])
    .wrap(Wrap { trim: true })
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[3]);
}

fn draw_platforms(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let total: i64 = app.platforms.iter().map(|p| p.rom_count).sum();
    let items: Vec<ListItem> = app
        .platforms
        .iter()
        .map(|p| {
            let playable = app
                .ra
                .as_ref()
                .and_then(|ra| app.resolve_core(ra, &p.fs_slug))
                .is_some();
            let mark = if playable { "●" } else { "○" };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {mark} "),
                    Style::default().fg(if playable { Color::Green } else { Color::DarkGray }),
                ),
                Span::raw(format!("{:<24}", p.display_name)),
                Span::styled(
                    format!("{:>6}", p.rom_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(format!(
            " platforms — {} games   ● = core installed ",
            total
        )))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
    f.render_stateful_widget(list, area, &mut app.platform_state);
}

fn draw_roms(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let slug = app
        .selected_platform()
        .map(|p| p.display_name.clone())
        .unwrap_or_default();

    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .filter_map(|&i| app.roms.get(i))
        .map(|r| {
            let local = app.local_path(r).is_some();
            let mark = if local { "▣" } else { "·" };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {mark} "),
                    Style::default().fg(if local { Color::Green } else { Color::DarkGray }),
                ),
                Span::raw(format!("{:<52}", truncate(&r.name, 52))),
                Span::styled(
                    format!("{:>10}", human(r.fs_size_bytes.max(0) as u64)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let title = format!(
        " {slug} — {} of {} shown   ▣ = downloaded ",
        app.filtered.len(),
        app.roms.len()
    );
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
    f.render_stateful_widget(list, area, &mut app.rom_state);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn setup_terminal(term: &mut Term) -> Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    term.hide_cursor()?;
    term.clear()?;
    Ok(())
}

fn restore_terminal(term: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    Ok(())
}

pub fn run(
    cache: &Cache,
    local_roms: &Path,
    ra: Option<RetroArch>,
    map: CoreMap,
    client: Option<Arc<api::Client>>,
    rt: tokio::runtime::Handle,
) -> Result<()> {
    let mut app = App::new(cache, local_roms.to_path_buf(), ra, map, client, rt)?;
    if app.platforms.is_empty() {
        anyhow::bail!("cache is empty — run `sync` first");
    }

    let mut term = Terminal::new(CrosstermBackend::new(stdout()))?;
    setup_terminal(&mut term)?;

    // Keep the terminal restorable even if drawing panics.
    let result = (|| -> Result<()> {
        while !app.quit {
            if let Some(rom) = app.poll_download() {
                app.launch_rom(&rom, &mut term)?;
            }
            term.draw(|f| draw(f, &mut app))?;
            // Timeout rather than a blocking read, so the progress gauge keeps
            // animating while a download runs in the background.
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                app.on_key(key.code, key.modifiers, cache, &mut term)?;
            }
        }
        Ok(())
    })();

    restore_terminal(&mut term)?;
    result
}

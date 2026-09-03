//! moose-patch — the addon.
//!
//! Two tabs, a list of dials, and one deliberate moment where everything you
//! turned actually happens. See `docs/knulli-addon.md` for what it is for and
//! `model` for why nothing applies as you touch it.
//!
//! It also runs without a window:
//!
//!     moose-patch --status     what every patch is currently at
//!     moose-patch --restore    put the device back to the saved profile
//!
//! `--restore` is what a freshly installed KNULLI needs, and it is deliberately
//! reachable over ssh — a device that has just been reflashed has no way to
//! launch a windowed app until the patches that make that possible are on.

use anyhow::{Context, Result};
use moose_patch::model::{App, Kind, Overlay, Tab};
use moose_patch::patch::{Patch, Paths, State};
use moose_patch::sync::Stage;
use moose_patch::worker;
use moose_patch::{catalogue, profile, rows, ui};
use moose_sdl::gfx::Gfx;
use moose_sdl::input;
use moose_sdl::text;
use sdl2::controller::Button;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;

/// What the pad and the keyboard both turn into, so the loop below reads the
/// same whichever one you are holding.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Press {
    Up,
    Down,
    Left,
    Right,
    Accept,
    Back,
    Detail,
    TabLeft,
    TabRight,
    Quit,
}

fn from_key(key: Keycode) -> Option<Press> {
    Some(match key {
        Keycode::Up | Keycode::W => Press::Up,
        Keycode::Down | Keycode::S => Press::Down,
        Keycode::Left | Keycode::A => Press::Left,
        Keycode::Right | Keycode::D => Press::Right,
        Keycode::Return | Keycode::Space => Press::Accept,
        Keycode::Backspace => Press::Back,
        Keycode::X => Press::Detail,
        Keycode::Q => Press::TabLeft,
        Keycode::E => Press::TabRight,
        Keycode::Escape => Press::Quit,
        _ => return None,
    })
}

/// Raw joystick numbers, for a pad SDL could not describe.
///
/// Only consulted when no controller opened. The numbering is the one
/// `es_input.cfg` gives this family of handhelds — 0 is the button printed A,
/// 1 the one printed B — which is also what RetroArch is configured with.
fn from_joy_button(index: u8, swapped: bool) -> Option<Press> {
    let (accept, back) = if swapped { (1, 0) } else { (0, 1) };
    Some(match index {
        i if i == accept => Press::Accept,
        i if i == back => Press::Back,
        2 | 3 => Press::Detail,
        4 => Press::TabLeft,
        5 => Press::TabRight,
        _ => return None,
    })
}

fn from_hat(state: sdl2::joystick::HatState) -> Option<Press> {
    use sdl2::joystick::HatState as H;
    Some(match state {
        H::Up => Press::Up,
        H::Down => Press::Down,
        H::Left => Press::Left,
        H::Right => Press::Right,
        _ => return None,
    })
}

/// Whether to swap the two face buttons that confirm and cancel.
///
/// Normally **no**, and the reasoning matters because I got it wrong once.
///
/// `es_input.cfg`'s letters are the letters *printed on the plastic*: that
/// file is written by EmulationStation's controller wizard, which asks you to
/// press A, then B, and records whatever you pressed. Vendor defaults are made
/// the same way. `moose_sdl::input` maps ES's `a` to SDL's `a`, so SDL's
/// `Button::A` already *is* the button printed A, whatever position it sits in
/// and whatever the kernel calls its scancode.
///
/// What misled me: EmulationStation carries `InvertButtons`, and the Flip has
/// it set. That is a **preference in ES's own interface** — "I would rather
/// confirm with the other button" — not a statement about the hardware.
/// Reading it as one made A quit the app, which is the bug this replaces.
///
/// So the swap is only ever an explicit choice, from the same setting the
/// desktop app uses, and off unless asked for.
fn buttons_swapped(cfg: &moose_rack::config::Config) -> bool {
    cfg.controllers.swap_ab
}

fn from_button(button: Button, swapped: bool) -> Option<Press> {
    let (accept, back) = if swapped {
        (Button::B, Button::A)
    } else {
        (Button::A, Button::B)
    };
    Some(match button {
        Button::DPadUp => Press::Up,
        Button::DPadDown => Press::Down,
        Button::DPadLeft => Press::Left,
        Button::DPadRight => Press::Right,
        b if b == accept => Press::Accept,
        b if b == back => Press::Back,
        Button::X | Button::Y => Press::Detail,
        Button::LeftShoulder => Press::TabLeft,
        Button::RightShoulder => Press::TabRight,
        _ => return None,
    })
}

fn open_window(video: &sdl2::VideoSubsystem) -> Result<sdl2::video::Window, String> {
    // The device's screen exactly. On a desktop it is a small window, which is
    // the point: the last front end was drawn at four times the size and every
    // judgement made about it was wrong.
    let (w, h) = (ui::PANEL.0, ui::PANEL.1);
    // Not `fullscreen_desktop`. On this handheld SDL's kmsdrm backend owns the
    // whole screen anyway, and asking for a fullscreen mode change on top of
    // that is a mode this app has to hand back cleanly when it exits — which
    // is one more thing to get wrong between the app quitting and
    // EmulationStation drawing again.
    let mut builder = video.window("moose-patch", w, h);
    builder
        .position_centered()
        .opengl()
        .build()
        .map_err(|e| e.to_string())
}

fn main() -> Result<()> {
    moose_rack::datadir::anchor();
    let paths = Paths::default();
    let patches = catalogue::all(&paths);

    match std::env::args().nth(1).as_deref() {
        Some("--status") => return status(&patches),
        Some("--restore") => return restore(&paths, &patches),
        Some("--plan") => return sync_cli(false),
        Some("--sync") => return sync_cli(true),
        Some("--pull-all") => return pull_all_cli(),
        Some("--refresh") => return refresh_cli(),
        Some("--saves") => return saves_cli(),
        Some("--apply") => return apply_cli(&patches, std::env::args().nth(2)),
        Some("--stars") => return stars_cli(false),
        Some("--stars-apply") => return stars_cli(true),
        Some("--save") => {
            profile::save(&paths, &patches)?;
            println!("wrote {}", paths.profile().display());
            return Ok(());
        }
        Some(other) if other.starts_with("--") => {
            eprintln!(
                "moose-patch [--status | --apply <id>=<option> | --plan | --sync \
                 | --refresh | --pull-all | --stars | --stars-apply | --restore | --save]"
            );
            std::process::exit(2);
        }
        _ => {}
    }
    window(&paths, &patches)
}

/// Ask the server what a sync would do, and print it. Moves nothing.
///
/// The same worker the interface uses, drained to the end instead of once a
/// frame — so this exercises the real path over ssh, on the device, without a
/// window. Every sync bug so far has been found by looking rather than by
/// reasoning, and this is the cheapest way to look.
/// Every save on the card and what it resolved to.
///
/// The plan only says how many were unmatched. When one is, the question is
/// always *which* and *why*, and this is the difference between knowing and
/// guessing at it.
fn saves_cli() -> Result<()> {
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    let ra_root = moose_rack::util::expand_tilde(&cfg.saves.root);
    let app_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let Some(cache_path) = worker::find_cache(&worker::cache_search_path(&app_dir)) else {
        anyhow::bail!("no library index — run --refresh first");
    };
    let cache = moose_rack::cache::Cache::open(&cache_path)?;
    let map = moose_rack::coremap::CoreMap::load_or_embedded(
        &app_dir.join("data/esde-core-map.json"),
    );
    for c in moose_rack::savesync::scan(&cache, &map, &ra_root)? {
        let name = c.path.file_name().unwrap_or_default().to_string_lossy();
        println!(
            "{name}\n    folder={} core={:?} slot={} canonical={} -> {:?}",
            c.core_dir, c.core, c.slot, c.canonical, c.resolution
        );
    }
    Ok(())
}

/// Rebuild the game list. Everything else depends on it being current.
fn refresh_cli() -> Result<()> {
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    let app_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    drain_to_end(worker::refresh_index(&cfg, &app_dir))
}

/// Take everything the server holds. For a device being set up, or one whose
/// card was wiped — `negotiate` will not re-offer a save this device already
/// took, which is the same rule that stops a deleted save coming back.
fn pull_all_cli() -> Result<()> {
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    let ra_root = moose_rack::util::expand_tilde(&cfg.saves.root);
    let library_root = moose_rack::util::expand_tilde(&cfg.library.local_root);
    let app_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let job = worker::pull_all(&cfg, &ra_root, &app_dir, &library_root);
    drain_to_end(job)
}

fn sync_cli(carry_out: bool) -> Result<()> {
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    let ra_root = moose_rack::util::expand_tilde(&cfg.saves.root);
    let app_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    println!("saves under {}", ra_root.display());

    let job = if carry_out {
        let library_root = moose_rack::util::expand_tilde(&cfg.library.local_root);
        worker::carry_out(&cfg, &ra_root, &app_dir, &library_root)
    } else {
        worker::negotiate(&cfg, &ra_root, &app_dir)
    };
    drain_to_end(job)
}

/// Turn one patch to one of its options, from a shell.
///
/// The window is the way a person does this. This is how it gets *checked* on
/// the device — over ssh, against the real /userdata, without a screen. Every
/// patch bug so far has been found by applying one and then reading the file
/// it claimed to write, and doing that needed a controller in hand until now.
fn apply_cli(patches: &[Patch], arg: Option<String>) -> Result<()> {
    let Some(arg) = arg else {
        anyhow::bail!("--apply needs <id>=<option>, e.g. --apply charge-awake=ON");
    };
    let (id, wanted) = arg
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("--apply needs <id>=<option>, not {arg:?}"))?;
    let Some(patch) = patches.iter().find(|p| p.id == id) else {
        anyhow::bail!(
            "no patch called {id:?} — try one of: {}",
            patches.iter().map(|p| p.id).collect::<Vec<_>>().join(", ")
        );
    };
    let options = patch.option_names();
    let Some(index) = options.iter().position(|o| o.eq_ignore_ascii_case(wanted)) else {
        anyhow::bail!("{id} has no option {wanted:?} — it has: {}", options.join(", "));
    };
    patch.apply(index)?;
    // Read back, rather than reporting what was asked for. The two disagreeing
    // is the whole class of bug this exists to catch.
    println!("{id} -> {}", match patch.state() {
        State::At(i) => options[i].clone(),
        State::Changed => "changed (does not match any option)".into(),
    });
    Ok(())
}

/// Favourites and collections: what the card and the server disagree about.
///
/// Without `--stars-apply` this only looks — it reads ES's gamelists and the
/// server's collections and prints the difference. Deciding by looking is the
/// point: every difference here is a star somebody set, and a plan that moves
/// nothing can be checked against what they remember doing.
fn stars_cli(carry_out: bool) -> Result<()> {
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    let app_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let Some(cache_path) = worker::find_cache(&worker::cache_search_path(&app_dir)) else {
        anyhow::bail!("no library index — run --refresh first");
    };
    let cache = moose_rack::cache::Cache::open(&cache_path)?;
    let platform = moose_rack::platform::current();
    let es = moose_patch::favmap::EsPaths::knulli();

    let known = moose_patch::favmap::on_card(&cache, platform, &es.roms)?;
    println!(
        "{} of the server's games are on this card, under {}",
        known.len(),
        es.roms.display()
    );

    let baseline_path = app_dir.join("favorites-baseline.json");
    let mut baseline = moose_patch::favsync::Baseline::load(&baseline_path);

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let client = moose_rack::api::Client::with_auth(
            &cfg.server.url,
            &cfg.server.username,
            &cfg.server.password,
            cfg.server.token.as_deref().filter(|t| !t.is_empty()),
        )?;
        let plan =
            moose_patch::favrun::plan(&client, &cache, &es, &known, &baseline, platform).await?;
        println!("{}", plan.headline());
        // Every collection, agreed or not. "Nothing to do" is only worth
        // anything if you can see that both sides were actually read.
        for s in &plan.surveyed {
            let flag = if s.here == s.reachable { " " } else { "!" };
            println!(
                "  {flag} {:<28} card {:>4}   server {:>4} ({} of them on this card)",
                s.name, s.here, s.server, s.reachable
            );
        }
        for item in &plan.items {
            let held = match &item.held {
                moose_patch::favrun::Held::Stars(f) => format!("stars in {}", f.join(", ")),
                moose_patch::favrun::Held::File => "collection file".into(),
            };
            println!("  {} ({held})", item.name);
            for m in &item.moves {
                let (what, id) = match m {
                    moose_patch::favsync::Move::StarHere(i) => ("star here", i),
                    moose_patch::favsync::Move::UnstarHere(i) => ("unstar here", i),
                    moose_patch::favsync::Move::StarOnServer(i) => ("star on server", i),
                    moose_patch::favsync::Move::UnstarOnServer(i) => ("unstar on server", i),
                };
                let name = cache
                    .rom_by_id(*id)
                    .ok()
                    .flatten()
                    .map(|r| r.name)
                    .unwrap_or_else(|| format!("rom {id}"));
                println!("    {what:<16} {name}");
            }
        }
        if !carry_out {
            println!("\nnothing moved — run --stars-apply to carry this out");
            return Ok(());
        }
        let report =
            moose_patch::favrun::carry_out(&client, &es, &known, &plan, &mut baseline).await?;
        baseline.save(&baseline_path)?;
        if moose_patch::favrun::show_all(&es, &plan)? {
            println!("told EmulationStation to show the collections it was hiding");
        }
        println!(
            "{} applied here ({} files rewritten), {} sent",
            report.applied_here, report.files_written, report.sent
        );
        for failure in &report.failed {
            println!("  failed: {failure}");
        }
        anyhow::Ok(())
    })
}

/// Follow one job to its end, printing each new thing it says.
fn drain_to_end(job: worker::Job) -> Result<()> {
    let mut stage = Stage::default();
    let mut conflicts = Vec::new();
    let mut last = String::new();
    loop {
        worker::apply(&mut stage, &mut conflicts, job.drain());
        let note = stage.note();
        if note != last {
            println!("{note}");
            last = note;
        }
        match &stage {
            Stage::Ready(review) => {
                for line in &review.lines {
                    let why = line.reason.as_deref().unwrap_or("");
                    println!("  {:<9} {} {why}", line.action.label(), line.title);
                }
                return Ok(());
            }
            Stage::Done { moved, conflicts: n, .. } => {
                println!("moved {moved}, {n} conflict(s)");
                for c in &conflicts {
                    println!("  conflict  {}  {}", c.file_name, c.reason.as_deref().unwrap_or("both sides changed"));
                }
                return Ok(());
            }
            Stage::Failed(_) => std::process::exit(1),
            _ => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
}

fn status(patches: &[Patch]) -> Result<()> {
    // The server first, because "it cannot reach RomM" and "no patch is on"
    // are different problems and this is the one line that tells them apart.
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    if cfg.server.url.is_empty() {
        println!("{:<16} not configured", "server");
    } else {
        let auth = if cfg.server.token.is_some() {
            "token"
        } else if !cfg.server.password.is_empty() {
            "password"
        } else {
            "no credential"
        };
        println!("{:<16} {} ({auth})", "server", cfg.server.url);
    }
    println!();
    for patch in patches {
        let at = match patch.state() {
            State::At(i) => patch.choices[i].name.clone(),
            State::Changed => "changed — not at any known setting".into(),
        };
        println!("{:<16} {at}", patch.id);
    }
    Ok(())
}

fn restore(paths: &Paths, patches: &[Patch]) -> Result<()> {
    let done = profile::restore(paths, patches)?;
    for line in &done.applied {
        println!("applied  {line}");
    }
    for id in &done.already {
        println!("already  {id}");
    }
    for line in &done.failed {
        eprintln!("failed   {line}");
    }
    for id in &done.unknown {
        // Not an error that stops the rest: a profile from a newer build
        // should still restore everything this one understands.
        eprintln!("unknown  {id}");
    }
    if done.applied.is_empty() && done.already.is_empty() && done.unknown.is_empty() {
        eprintln!("nothing to restore — no profile at {}", paths.profile().display());
    }
    if !done.failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn window(paths: &Paths, patches: &[Patch]) -> Result<()> {
    let sdl = sdl2::init()
        .map_err(anyhow::Error::msg)
        .context("starting SDL")?;
    let video = sdl
        .video()
        .map_err(anyhow::Error::msg)
        .context("opening the display")?;
    {
        // Fixed before the window is made; SDL will not change them after.
        let attr = video.gl_attr();
        if cfg!(any(target_os = "android", target_os = "linux")) {
            attr.set_context_profile(sdl2::video::GLProfile::GLES);
            attr.set_context_version(3, 0);
        } else {
            attr.set_context_profile(sdl2::video::GLProfile::Core);
            attr.set_context_version(3, 3);
        }
    }

    let win = open_window(&video)
        .map_err(anyhow::Error::msg)
        .context("opening a window")?;
    let _context = win
        .gl_create_context()
        .map_err(anyhow::Error::msg)
        .context("creating an OpenGL context")?;
    win.gl_set_context_to_current().map_err(anyhow::Error::msg)?;

    let mut gfx = unsafe { Gfx::new(&video) }.context("setting up the renderer")?;
    let _ = video.gl_set_swap_interval(sdl2::video::SwapInterval::VSync);
    let mut painter = text::Painter::new().context("finding fonts")?;

    // The pad, through the same code the front end used — the GUID on this
    // family of handhelds is shared by four different devices, so the mapping
    // has to come from the OS's own es_input.cfg rather than SDL's database.
    let controllers = sdl.game_controller().map_err(anyhow::Error::msg)?;
    let joysticks = sdl.joystick().map_err(anyhow::Error::msg)?;
    let pads = input::Pads::open_first(&controllers, &joysticks);
    // A pad SDL cannot describe sends joystick events and no controller ones.
    // Listening only for controller events would look exactly like the app
    // having frozen: it draws, and nothing you press does anything.
    let raw_pad = !pads.is_open();
    if raw_pad {
        eprintln!("no controller mapping — falling back to raw joystick buttons");
        for index in 0..joysticks.num_joysticks().unwrap_or(0) {
            let _ = joysticks.open(index);
        }
    }

    // The server, if this device has been given credentials. Read once: the
    // sync tab shows where it would talk to, and says so plainly when nowhere.
    let cfg = moose_rack::config::Config::load().unwrap_or_default();
    let server = (!cfg.server.url.is_empty()).then(|| cfg.server.url.clone());

    // Which button confirms. See `buttons_swapped`.
    let swapped = buttons_swapped(&cfg);
    println!("confirm/cancel swapped: {swapped}");

    let stage = Stage::default();
    let stars = moose_patch::sync::Stars::default();
    let mut app = App {
        tab: Tab::Patches,
        sync: rows::sync(server.as_deref(), &stage.note(), &stars.note()),
        stage,
        stars,
        star_plan: None,
        conflicts: Vec::new(),
        patches: rows::patches(patches),
        overlay: Overlay::None,
        should_quit: false,
    };
    let mut view = ui::Ui::default();
    let mut events = sdl.event_pump().map_err(anyhow::Error::msg)?;
    let mut job: Option<Running> = None;

    // Where the saves are and where our own files live. Both are wanted only
    // when a sync starts, but reading them once keeps the loop free of it.
    let ra_root = moose_rack::util::expand_tilde(&cfg.saves.root);
    let library_root = moose_rack::util::expand_tilde(&cfg.library.local_root);
    let app_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    while !app.should_quit {
        // A menu has nothing moving in it, so it waits for something to happen
        // rather than burning a core drawing the same frame. On a handheld
        // that is the difference between warm and not.
        let woke = events.wait_event_timeout(250);
        let arrived: Vec<Event> = woke.into_iter().chain(events.poll_iter()).collect();

        for event in arrived {
            let press = match event {
                Event::Quit { .. } => Some(Press::Quit),
                Event::KeyDown { keycode: Some(key), repeat: false, .. } => from_key(key),
                Event::ControllerButtonDown { button, .. } => from_button(button, swapped),
                Event::JoyButtonDown { button_idx, .. } if raw_pad => {
                    from_joy_button(button_idx, swapped)
                }
                Event::JoyHatMotion { state, .. } if raw_pad => from_hat(state),
                _ => None,
            };
            let Some(press) = press else { continue };
            // Every press, in the log. When a button did nothing, the question
            // is always whether the app never saw it or saw it and ignored it,
            // and one line settles that without a second trip to the device.
            eprintln!("press {press:?}");
            if let Some(request) = act(&mut app, &mut view, press, paths, patches) {
                job = Some(match request {
                    Request::Negotiate => {
                        Running::Saves(worker::negotiate(&cfg, &ra_root, &app_dir))
                    }
                    Request::CarryOut => Running::Saves(worker::carry_out(
                        &cfg,
                        &ra_root,
                        &app_dir,
                        &library_root,
                    )),
                    Request::Refresh => Running::Saves(worker::refresh_index(&cfg, &app_dir)),
                    Request::Stars => Running::Stars(worker::stars(&cfg, &app_dir, false)),
                    Request::StarsApply => Running::Stars(worker::stars(&cfg, &app_dir, true)),
                });
            }
        }

        // Anything the worker has said since the last frame. Which fold it
        // goes through is decided by which job is running, not by the message:
        // the two syncs report the same kinds of thing and only the caller
        // knows whose news it is.
        if let Some(running) = &job {
            let messages = running.job().drain();
            if !messages.is_empty() {
                match running {
                    Running::Saves(_) => {
                        worker::apply(&mut app.stage, &mut app.conflicts, messages);
                        if !app.stage.is_busy() {
                            job = None;
                        }
                    }
                    Running::Stars(_) => {
                        worker::apply_stars(&mut app.stars, &mut app.star_plan, messages);
                        if !app.stars.is_busy() {
                            job = None;
                        }
                    }
                }
            }
        }
        let note = app.stage.note();
        app.sync.set_fact("status", &note);
        let stars_note = app.stars.note();
        app.sync.set_note("stars", &stars_note);

        gfx.resize(ui::PANEL.0 as f32, ui::PANEL.1 as f32);
        view.draw(&gfx, &mut painter, &app);
        win.gl_swap_window();
    }
    Ok(())
}

/// Carry out the queue. Everything that failed is left showing as still
/// pending, which is the honest thing for the menu to say afterwards.
fn run_queue(app: &mut App, paths: &Paths, patches: &[Patch]) {
    for (id, index) in app.orders() {
        let Some(patch) = patches.iter().find(|p| p.id == id) else { continue };
        match patch.apply(index) {
            Ok(()) => {
                for page in [&mut app.sync, &mut app.patches] {
                    if let Some(row) = page.rows.iter_mut().find(|r| r.id == id) {
                        row.settle();
                    }
                }
            }
            Err(e) => eprintln!("{id}: {e:#}"),
        }
    }
    if let Err(e) = profile::save(paths, patches) {
        eprintln!("could not write the profile: {e:#}");
    }
}

/// Which sync the running job belongs to.
///
/// Both report through the same channel, and both say "Note" and "Failed", so
/// nothing in a message identifies whose it is. The caller started it and is
/// the only one that knows.
enum Running {
    Saves(worker::Job),
    Stars(worker::Job),
}

impl Running {
    fn job(&self) -> &worker::Job {
        match self {
            Running::Saves(j) | Running::Stars(j) => j,
        }
    }
}

/// Something only the outside world can do. Returned rather than performed,
/// so every button press stays testable without a network or a window.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Request {
    /// Ask the server what a sync would do. Moves nothing.
    Negotiate,
    /// Do it. Only reachable from a plan that has been shown and accepted.
    CarryOut,
    /// Rebuild the list of games. Everything else is matched through it.
    Refresh,
    /// Ask what the favourites and collections sync would do. Moves nothing.
    Stars,
    /// Do it.
    StarsApply,
}

/// One press. Split out so the whole of the app's behaviour can be exercised
/// without a window — see the tests at the foot of this file.
fn act(
    app: &mut App,
    view: &mut ui::Ui,
    press: Press,
    paths: &Paths,
    patches: &[Patch],
) -> Option<Request> {
    if press == Press::Quit {
        app.should_quit = true;
        return None;
    }
    match app.overlay {
        Overlay::Applying { .. } => {}

        Overlay::Detail => {
            if matches!(press, Press::Back | Press::Accept | Press::Detail) {
                app.overlay = Overlay::None;
            }
        }

        Overlay::ConfirmAction { .. } => match press {
            Press::Accept => {
                let id = app.page().selected().map(|r| r.id.clone()).unwrap_or_default();
                app.overlay = Overlay::None;
                if id == "refresh" {
                    if app.stage.is_busy() {
                        return None;
                    }
                    app.stage = Stage::Asking { note: "starting".into() };
                    return Some(Request::Refresh);
                }
                if id == "check" {
                    // Refusing while one is already in flight: two syncs would
                    // race on the same files.
                    if app.stage.is_busy() {
                        return None;
                    }
                    // Which of the two stages this press means is decided in
                    // one place — `Stage::next_step` — so the prompt, the help
                    // line and this cannot drift apart.
                    return match app.stage.next_step() {
                        Some("carry this out") => {
                            app.stage = Stage::Asking { note: "starting".into() };
                            Some(Request::CarryOut)
                        }
                        Some(_) => {
                            app.stage = Stage::Asking { note: "starting".into() };
                            Some(Request::Negotiate)
                        }
                        None => None,
                    };
                }
                if id == "stars" {
                    if app.stars.is_busy() {
                        return None;
                    }
                    // Same two-step as the saves, decided in one place so the
                    // prompt and the handler cannot disagree.
                    return match app.stars.next_step() {
                        Some("carry this out") => {
                            app.stars = moose_patch::sync::Stars::Asking("starting".into());
                            Some(Request::StarsApply)
                        }
                        Some(_) => {
                            app.stars = moose_patch::sync::Stars::Asking("starting".into());
                            Some(Request::Stars)
                        }
                        None => None,
                    };
                }
                eprintln!("sync action '{id}' is not wired up yet");
            }
            Press::Back => app.overlay = Overlay::None,
            _ => {}
        },

        Overlay::ConfirmApply => match press {
            Press::Accept => {
                run_queue(app, paths, patches);
                app.overlay = Overlay::None;
            }
            Press::Back => app.overlay = Overlay::None,
            _ => {}
        },

        Overlay::ConfirmDiscard => match press {
            Press::Accept => {
                app.sync.revert_all();
                app.patches.revert_all();
                app.should_quit = true;
            }
            Press::Back => app.overlay = Overlay::None,
            _ => {}
        },

        Overlay::None => match press {
            Press::Up => app.page_mut().step(-1),
            Press::Down => app.page_mut().step(1),
            Press::Left => app.page_mut().turn_selected(-1),
            Press::Right => app.page_mut().turn_selected(1),
            Press::TabLeft => {
                app.next_tab(-1);
                view.first = 0;
            }
            Press::TabRight => {
                app.next_tab(1);
                view.first = 0;
            }
            Press::Detail => {
                if app.page().selected().is_some_and(|r| !r.detail.is_empty()) {
                    app.overlay = Overlay::Detail;
                }
            }
            Press::Accept => {
                // A means "do what this tab is for". On sync that is the one
                // row under the cursor — those are actions, with nothing to
                // reconcile, so they ask once and go. On patches it is
                // everything queued.
                let action = match app.page().selected() {
                    Some(row) if matches!(row.kind, Kind::Action { .. }) => {
                        Some(row.title.clone())
                    }
                    _ => None,
                };
                if let Some(title) = action {
                    app.overlay = Overlay::ConfirmAction { title };
                } else if !app.queue().is_empty() {
                    app.overlay = Overlay::ConfirmApply;
                }
            }
            Press::Back => {
                if app.queue().is_empty() {
                    app.should_quit = true;
                } else {
                    app.overlay = Overlay::ConfirmDiscard;
                }
            }
            Press::Quit => app.should_quit = true,
        },
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole app pointed at a temporary directory, so pressing buttons in
    /// these tests really does write files and read them back.
    fn fixture(name: &str) -> (App, ui::Ui, Paths, Vec<Patch>) {
        let dir = std::env::temp_dir().join(format!("moose-main-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::new(dir);
        let patches = catalogue::all(&paths);
        let app = App {
            tab: Tab::Patches,
            sync: rows::sync(None, "not synced yet", "not checked yet"),
            stage: Stage::default(),
            stars: moose_patch::sync::Stars::default(),
            star_plan: None,
            conflicts: Vec::new(),
            patches: rows::patches(&patches),
            overlay: Overlay::None,
            should_quit: false,
        };
        (app, ui::Ui::default(), paths, patches)
    }

    fn press(f: &mut (App, ui::Ui, Paths, Vec<Patch>), p: Press) -> Option<Request> {
        act(&mut f.0, &mut f.1, p, &f.2, &f.3)
    }

    #[test]
    fn sdls_a_is_the_button_printed_a_and_confirms() {
        // `es_input.cfg` records the letters printed on the plastic, because
        // that is what its wizard asks you to press, and moose_sdl maps ES's
        // `a` to SDL's `a`. So no swap is the right default, whatever position
        // the button sits in or the kernel calls its scancode.
        //
        // Getting this backwards made A quit the app. The press log said so:
        // two Accepts, then two Backs, then "exited 0".
        assert_eq!(from_button(Button::A, false), Some(Press::Accept));
        assert_eq!(from_button(Button::B, false), Some(Press::Back));
        // And the explicit setting, for a device that really is the other way.
        assert_eq!(from_button(Button::B, true), Some(Press::Accept));
        assert_eq!(from_button(Button::A, true), Some(Press::Back));
        // The d-pad is unaffected either way.
        assert_eq!(from_button(Button::DPadUp, true), Some(Press::Up));
        assert_eq!(from_joy_button(0, false), Some(Press::Accept));
        assert_eq!(from_joy_button(0, true), Some(Press::Back));
    }

    #[test]
    fn the_swap_is_off_unless_the_config_asks() {
        // Specifically *not* read from EmulationStation's `InvertButtons`.
        // That is a preference in its interface, not a fact about the pad, and
        // treating it as one is what broke A.
        let cfg = moose_rack::config::Config::default();
        assert!(!buttons_swapped(&cfg));
    }

    #[test]
    fn a_on_the_sync_tab_runs_that_row_rather_than_the_patch_queue() {
        // The two tabs mean different things by A. Queuing a patch change and
        // then pressing A on "Push saves up" must not apply the patch — that
        // is a different tab's business and a nasty surprise.
        let mut f = fixture("sync-a");
        press(&mut f, Press::Right);
        assert_eq!(f.0.queue().len(), 1);

        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down); // past refresh, onto "see what would sync"
        assert_eq!(f.0.tab, Tab::Sync);
        press(&mut f, Press::Accept);
        match &f.0.overlay {
            Overlay::ConfirmAction { title } => assert!(title.contains("would sync")),
            other => panic!("expected the action prompt, got {other:?}"),
        }

        // Cancelling leaves both the queue and the device untouched.
        press(&mut f, Press::Back);
        assert_eq!(f.0.overlay, Overlay::None);
        assert_eq!(f.0.queue().len(), 1);
        assert!(!f.2.knulli_conf().exists());
    }

    #[test]
    fn accepting_the_sync_row_asks_the_server_and_moves_nothing() {
        // The two-stage rule: pressing A here produces a *request for a plan*.
        // Nothing is transferred, and the patches queue is untouched.
        let mut f = fixture("sync-start");
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down); // past the refresh row, onto "see what would sync"
        assert_eq!(press(&mut f, Press::Accept), None, "the prompt comes first");
        assert_eq!(press(&mut f, Press::Accept), Some(Request::Negotiate));
        assert!(f.0.stage.is_busy());
        assert!(!f.2.knulli_conf().exists());
    }

    #[test]
    fn a_plan_in_hand_turns_the_same_button_into_carry_it_out() {
        // The two stages, from one row. Which one this press means is decided
        // by `Stage::next_step` and nowhere else.
        use moose_patch::sync::Review;
        let mut f = fixture("sync-two-stage");
        f.0.stage = Stage::Ready(Review {
            lines: vec![moose_patch::sync::Line {
                action: moose_patch::sync::Action::Upload,
                title: "Zelda.srm".into(),
                reason: None,
                rom_id: 1,
                save_id: None,
            }],
            agreed: 0,
        });
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down); // past refresh, onto "see what would sync"
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), Some(Request::CarryOut));
    }

    #[test]
    fn an_empty_plan_offers_to_look_again_rather_than_run_nothing() {
        use moose_patch::sync::Review;
        let mut f = fixture("sync-empty-plan");
        f.0.stage = Stage::Ready(Review { lines: vec![], agreed: 380 });
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down); // past refresh, onto "see what would sync"
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), Some(Request::Negotiate));
    }

    #[test]
    fn the_game_list_can_be_rebuilt_from_its_own_row() {
        // The row that unblocks everything else: saves are matched to games by
        // the server's id, and a rescan there renumbers them all.
        let mut f = fixture("refresh");
        press(&mut f, Press::TabLeft);
        assert_eq!(
            f.0.page().selected().map(|r| r.id.as_str()),
            Some("refresh"),
            "refresh is the first thing to do on a device, so it is where the cursor lands"
        );
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), Some(Request::Refresh));
        assert!(f.0.stage.is_busy());
    }

    #[test]
    fn a_second_press_will_not_start_a_second_sync() {
        // Two in flight would race on the same files, and the second would
        // overwrite what the first had just written.
        let mut f = fixture("sync-twice");
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down); // past the refresh row, onto "see what would sync"
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), Some(Request::Negotiate));
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), None, "still busy");
    }

    #[test]
    fn a_is_wired_to_the_favourites_row() {
        // The row exists and A does something on it. "A is not wired to
        // apply" happened once already; a row that draws and does nothing is
        // indistinguishable from a broken app.
        let mut f = fixture("stars-wired");
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down);
        press(&mut f, Press::Down); // onto "Sync favourites and collections"
        assert_eq!(
            f.0.page().selected().map(|r| r.id.as_str()),
            Some("stars"),
            "the row moved — this test is pressing A on something else"
        );
        press(&mut f, Press::Accept); // the confirmation
        assert_eq!(press(&mut f, Press::Accept), Some(Request::Stars));
        assert!(f.0.stars.is_busy());
    }

    #[test]
    fn looking_at_the_stars_comes_before_moving_them() {
        // First press asks; only a plan with something in it turns the second
        // press into one that writes.
        let mut f = fixture("stars-two-step");
        f.0.stars = moose_patch::sync::Stars::Ready { headline: "2 to send".into(), moves: 2 };
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down);
        press(&mut f, Press::Down);
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), Some(Request::StarsApply));

        // and a plan with nothing in it offers to look again, not to run
        let mut f = fixture("stars-empty-plan");
        f.0.stars =
            moose_patch::sync::Stars::Ready { headline: "nothing to do".into(), moves: 0 };
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down);
        press(&mut f, Press::Down);
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), Some(Request::Stars));
    }

    #[test]
    fn the_two_syncs_do_not_block_each_other() {
        // Separate stages: looking at the saves must not make the favourites
        // row unpressable, and the other way round.
        let mut f = fixture("stars-independent");
        f.0.stage = Stage::Asking { note: "scanning saves".into() };
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down);
        press(&mut f, Press::Down);
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Accept), Some(Request::Stars));
    }

    #[test]
    fn cancelling_the_sync_prompt_asks_for_nothing() {
        let mut f = fixture("sync-cancel");
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down); // past the refresh row, onto "see what would sync"
        press(&mut f, Press::Accept);
        assert_eq!(press(&mut f, Press::Back), None);
        assert_eq!(f.0.overlay, Overlay::None);
        assert!(!f.0.stage.is_busy());
    }

    #[test]
    fn b_leaves_straight_away_when_nothing_is_queued() {
        let mut f = fixture("leave");
        press(&mut f, Press::Back);
        assert!(f.0.should_quit);
    }

    #[test]
    fn b_asks_first_when_something_is_queued() {
        // The whole point of queuing: work in hand must not vanish because a
        // thumb found the wrong button.
        let mut f = fixture("ask");
        press(&mut f, Press::Right);
        press(&mut f, Press::Back);
        assert_eq!(f.0.overlay, Overlay::ConfirmDiscard);
        assert!(!f.0.should_quit);
    }

    #[test]
    fn a_never_applies_without_confirming() {
        let mut f = fixture("confirm");
        press(&mut f, Press::Right);
        press(&mut f, Press::Accept);
        assert_eq!(f.0.overlay, Overlay::ConfirmApply);
        assert_eq!(f.0.queue().len(), 1, "nothing may be applied yet");
        // And nothing has been written.
        assert!(!f.2.knulli_conf().exists());
    }

    #[test]
    fn confirming_writes_the_files_and_empties_the_queue() {
        // End to end: a press of A really does put the block in knulli.conf.
        let mut f = fixture("apply");
        press(&mut f, Press::Right);
        press(&mut f, Press::Accept);
        press(&mut f, Press::Accept);
        assert!(f.0.queue().is_empty());
        let conf = std::fs::read_to_string(f.2.knulli_conf()).unwrap();
        assert!(conf.contains("## moose-patch: hotkeys"), "{conf}");
        assert_eq!(f.3[0].state(), State::At(1));
    }

    #[test]
    fn cancelling_keeps_the_queue_and_writes_nothing() {
        let mut f = fixture("cancel");
        press(&mut f, Press::Right);
        press(&mut f, Press::Accept);
        press(&mut f, Press::Back);
        assert_eq!(f.0.queue().len(), 1);
        assert!(!f.2.knulli_conf().exists());
    }

    #[test]
    fn applying_writes_a_profile_that_can_rebuild_the_device() {
        let mut f = fixture("profile");
        press(&mut f, Press::Right);
        press(&mut f, Press::Accept);
        press(&mut f, Press::Accept);
        let saved = std::fs::read_to_string(f.2.profile()).unwrap();
        assert!(saved.contains("hotkeys = \"ON\""), "{saved}");
    }

    #[test]
    fn discarding_puts_every_dial_back() {
        let mut f = fixture("discard");
        press(&mut f, Press::Right);
        press(&mut f, Press::Down);
        press(&mut f, Press::Right);
        assert_eq!(f.0.queue().len(), 2);
        press(&mut f, Press::Back);
        press(&mut f, Press::Accept);
        assert!(f.0.queue().is_empty());
        assert!(f.0.should_quit);
    }

    #[test]
    fn the_queue_survives_changing_tabs() {
        // Turning a dial on one tab and wandering to the other must not
        // quietly drop it — the counter says "not applied", and it has to
        // mean it.
        let mut f = fixture("tabs");
        press(&mut f, Press::Right);
        press(&mut f, Press::TabLeft);
        press(&mut f, Press::Down); // past refresh, onto "see what would sync"
        assert_eq!(f.0.tab, Tab::Sync);
        assert_eq!(f.0.queue().len(), 1);
    }

    #[test]
    fn x_opens_the_detail_and_b_closes_it() {
        let mut f = fixture("detail");
        press(&mut f, Press::Detail);
        assert_eq!(f.0.overlay, Overlay::Detail);
        press(&mut f, Press::Back);
        assert_eq!(f.0.overlay, Overlay::None);
        assert!(!f.0.should_quit, "closing a panel is not leaving the app");
    }

    #[test]
    fn dials_do_nothing_while_a_panel_is_up() {
        let mut f = fixture("behind");
        press(&mut f, Press::Detail);
        press(&mut f, Press::Right);
        assert!(f.0.queue().is_empty(), "the list is not live behind a panel");
    }

    #[test]
    fn a_second_visit_opens_at_what_the_first_one_did() {
        // Apply, then rebuild the menu the way starting the app again would.
        // The row has to come back already on, or every session proposes the
        // same change forever.
        let mut f = fixture("again");
        press(&mut f, Press::Right);
        press(&mut f, Press::Accept);
        press(&mut f, Press::Accept);

        let fresh = rows::patches(&catalogue::all(&f.2));
        let row = fresh.rows.iter().find(|r| r.id == "hotkeys").unwrap();
        assert_eq!(row.value(), "ON");
        assert!(!row.pending());
    }
}

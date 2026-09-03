// The SDL front end: a window, a loop, and input.
//
// Phase one of docs/handheld-frontend.md task 3. There is nothing to look at
// yet beyond proof that the parts fit — what this establishes is the shape
// everything after it is written against:
//
//   * The window is **resizable**, and the layout answers. Not a fixed
//     backbuffer scaled up. On the handheld the size is settled once at
//     startup; on a desktop somebody drags an edge, and doing that is the
//     fastest way to find every place a pixel got hardcoded.
//   * Nothing is measured in pixels. A card is 150 *points*; `moose_rack::
//     layout` turns points into pixels for whatever panel it lands on. The
//     moment a number in a view means pixels, that view has picked a screen.
//   * Input resolves through `moose_rack::binds`, so a rebind made in the
//     desktop app is a rebind here. The pad's deadzones and repeat timings
//     come from `moose_rack::padpoll`, which the webview only ever had a
//     copy of.
//
// It opens at 960x720 on purpose. That is the handheld, and living at 4:3
// every day is what stops the constrained case being a surprise at the end.

use crate::gfx::{Gfx, Rgba};
use anyhow::{Context, Result};
use moose_rack::layout::{Edges, Rect, Scale, Size, Viewport};
use moose_rack::{padpoll, rowwindow};
use sdl2::controller::Button;
use sdl2::event::{Event, WindowEvent};
use std::collections::BTreeSet;
use std::path::PathBuf;

mod backdrop;
mod covers;
mod gfx;
mod glass;
mod input;
mod keyboard;
mod library;
mod ports;
mod rescan;
mod settings;
mod status;
mod iconfetch;
mod sysinfo;
mod text;
mod wifi;

/// The handheld's panel, in points — and the size every layout below is
/// written against.
///
/// The Miyoo Flip is 640x480. That is not a window size, it is the *unit*: on
/// the device one point is one pixel, and on this Mac the window is opened four
/// times larger and `scale_for` divides back down, so the layout is 640x480
/// points either way and what is on screen here is what will be on screen
/// there. Getting that wrong is how a design that reads fine on a desk turns
/// out to be four words per line on the device.
const PANEL: (u32, u32) = (640, 480);

/// How much bigger than the panel the preview window is, in each direction.
///
/// Four, because at 1:1 a 640x480 window on a retina Mac is a postage stamp
/// nobody can judge a layout in. Asked for as *points*, so the window is
/// requested at half this and the display's own doubling supplies the rest —
/// on a retina Mac that lands on 2560x1920 pixels, four device pixels per panel
/// pixel. It does not have to land exactly: `viewport` divides whatever
/// drawable it gets by 640, so the layout is a 640-point panel either way and
/// only the physical size of the preview changes.
const PREVIEW: u32 = 3;

/// How many pixels one layout point gets.
///
/// The panel is 640x480 pixels and that does not change. This is how much of it
/// one element takes: at 1 the layout is 640x480 points and a 12-point label is
/// twelve pixels tall; at 2 it is 320x240 points and the same label is
/// twenty-four.
///
/// One and a half, for the handheld. A 3.5" screen held at arm's length is not
/// a monitor at arm's length, and the layout this front end started from was a
/// desktop one measured in the same numbers. Two was tried and is too much: six
/// rows of a list, and titles cut to "Castle of…".
///
/// Nothing is upscaled at any value — the type is *rasterised* at the size the
/// scale asks for, so it is bigger and exactly as sharp.
///
/// A fractional scale costs one thing and it is worth naming. Points land on
/// half-pixels: a rectangle edge at 10.5 pixels is drawn across two, so a
/// hairline rule comes out as two grey pixels rather than one bright one. Text
/// is unaffected — glyphs are rasterised at their real size and their position
/// is rounded — and the panels are drawn by a shader that antialiases its own
/// edges anyway. So the penalty is confined to rules and outlines, and 1.5 is
/// the friendliest fraction there is: everything even lands whole.
const UI_SCALE: f32 = 1.5;

/// The same, overridable without a rebuild: `MOOSE_SDL_UI_SCALE=1.5`.
///
/// Because the right number is a judgement about a screen, made by looking at
/// that screen, and a rebuild-and-reinstall between each look is how a
/// judgement like that gets made once and badly.
fn ui_scale() -> f32 {
    std::env::var("MOOSE_SDL_UI_SCALE")
        .ok()
        .and_then(|n| n.parse::<f32>().ok())
        .filter(|n| (0.5..=4.0).contains(n))
        .unwrap_or(UI_SCALE)
}

/// How long to wait for something to happen before looking around anyway.
///
/// Fifty milliseconds: twenty wake-ups a second, each of which does nothing but
/// check a clock. Long enough that the process is asleep; short enough that a
/// button press is never noticeably late.
const IDLE_WAIT_MS: u32 = 50;

/// How long to wait while a button is held down.
///
/// A pad reports what it is holding when asked, not by sending anything, so a
/// held direction produces no events at all — the loop has to come round on its
/// own for auto-repeat to keep firing. Eight milliseconds is finer than the
/// repeat rate and still a hundred and twenty times less work than not waiting.
const HELD_WAIT_MS: u32 = 8;

/// How long a frame may stand before it is redrawn regardless.
const STALE_MS: f64 = 500.0;

/// What the backdrop is actually redrawn at.
///
/// Per style rather than one number for all of them: at one frame a second the
/// app costs half a percent of a core, and whether that looks like one frame a
/// second is entirely a question about which backdrop is behind it.
///
/// `MOOSE_SDL_MOTION_FPS=4` overrides it, which is how the table was arrived at.
fn motion_fps(style: &str, speed: f32) -> f64 {
    std::env::var("MOOSE_SDL_MOTION_FPS")
        .ok()
        .and_then(|n| n.parse().ok())
        .filter(|n: &f64| *n > 0.0)
        .unwrap_or_else(|| backdrop::needed_fps(style, speed))
}

/// How long a page takes to slide in when the tab changes.
///
/// Short. A transition is there to say *which way* you moved, and one long
/// enough to be admired is one you wait for every time.
const SLIDE_MS: f64 = 180.0;

/// What the pad is actually doing, printed.
///
/// `MOOSE_SDL_BENCH=pads`. Two rounds of "the controls do not work" went by on
/// guesses — first that SDL had no mapping, then that the mapping was right and
/// something else was wrong — and a guess is what this replaces. It says which
/// mapping SDL settled on, what the joystick reports underneath it, and which
/// buttons are down right now, for twenty seconds.
///
/// Both layers, because the interesting failures are disagreements between
/// them: a mapping that names a button the pad does not have, or a pad that
/// reports a button pressed that nobody is touching.
fn bench_pads(controller: &sdl2::GameControllerSubsystem, joystick: &sdl2::JoystickSubsystem, events: &mut sdl2::EventPump) {
    let entries = input::es_input_mappings();
    println!("{} pads described in es_input.cfg", entries.len());
    let count = joystick.num_joysticks().unwrap_or(0);
    println!("{count} joystick(s)");

    let mut pads = Vec::new();
    let mut sticks = Vec::new();
    for i in 0..count {
        if let Ok(js) = joystick.open(i) {
            println!(
                "  [{i}] {} — guid {} — {} buttons, {} axes, {} hats",
                js.name(),
                js.guid(),
                js.num_buttons(),
                js.num_axes(),
                js.num_hats()
            );
            let shared = entries.iter().filter(|e| e.guid == js.guid().to_string()).count();
            println!("       {shared} pad(s) in es_input.cfg share that guid");
            match input::best_mapping(&entries, &js.guid().to_string(), &js.name(), js.num_hats() > 0) {
                Some(m) => {
                    println!("       chose: {m}");
                    let _ = controller.add_mapping(&m);
                }
                None => println!("       none of them fits; leaving SDL's own"),
            }
            sticks.push(js);
        }
        if controller.is_game_controller(i) {
            if let Ok(pad) = controller.open(i) {
                println!("       opens as a controller: {}", pad.name());
                println!("       mapping in use: {}", pad.mapping());
                pads.push(pad);
            }
        } else {
            println!("       SDL has no controller mapping for it");
        }
    }

    println!("\nwatching for 20 seconds — press things\n");
    let mut was = String::new();
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 20 {
        for event in events.poll_iter() {
            if matches!(event, Event::Quit { .. }) {
                return;
            }
        }
        let mut now = String::new();
        for js in &sticks {
            let down: Vec<String> = (0..js.num_buttons())
                .filter(|b| js.button(*b).unwrap_or(false))
                .map(|b| b.to_string())
                .collect();
            let moved: Vec<String> = (0..js.num_axes())
                .filter_map(|a| {
                    let v = js.axis(a).unwrap_or(0);
                    (v.saturating_abs() > 8000).then(|| format!("a{a}={v}"))
                })
                .collect();
            if !down.is_empty() || !moved.is_empty() {
                now += &format!("raw buttons [{}] axes [{}]  ", down.join(" "), moved.join(" "));
            }
        }
        for pad in &pads {
            let named: Vec<&str> = [
                (Button::A, "A"), (Button::B, "B"), (Button::X, "X"), (Button::Y, "Y"),
                (Button::LeftShoulder, "L1"), (Button::RightShoulder, "R1"),
                (Button::Back, "Back"), (Button::Start, "Start"), (Button::Guide, "Guide"),
                (Button::LeftStick, "L3"), (Button::RightStick, "R3"),
                (Button::DPadUp, "Up"), (Button::DPadDown, "Down"),
                (Button::DPadLeft, "Left"), (Button::DPadRight, "Right"),
            ]
            .iter()
            .filter(|(b, _)| pad.button(*b))
            .map(|(_, n)| *n)
            .collect();
            if !named.is_empty() {
                now += &format!("controller [{}]", named.join(" "));
            }
        }
        if now != was {
            println!("{}", if now.is_empty() { "(nothing)".to_owned() } else { now.clone() });
            was = now;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

/// The floor: one picture, redrawn every frame, and nothing else.
///
/// `MOOSE_SDL_BENCH=image` — a still frame put on screen at the display's rate
/// with no shader, no blur, no text and no frame-skipping. It answers the
/// question every other measurement is relative to: what does it cost this
/// machine merely to hand SDL a window and put pixels in it?
///
/// Without it a number like "19%" means nothing. If the floor is 15 there is
/// almost nothing left to win; if it is 2 the drawing is the whole cost.
/// Reported rather than guessed, because guessing is what got the last round's
/// optimization pointed at the wrong thing.
fn bench_image(window: &sdl2::video::Window, gfx: &mut Gfx, events: &mut sdl2::EventPump) {
    // One texture, uploaded once. A gradient rather than a solid: a driver may
    // notice that a constant color needs no sampling.
    let (w, h) = (PANEL.0, PANEL.1);
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.extend_from_slice(&[
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                128,
                255,
            ]);
        }
    }
    let picture = gfx.upload_rgba(w, h, &pixels);
    let mut frames = 0u64;
    let start = std::time::Instant::now();
    loop {
        for event in events.poll_iter() {
            if matches!(event, Event::Quit { .. }) {
                return;
            }
        }
        let (dw, dh) = window.drawable_size();
        let (ox, oy, zoom) = panel_box(dw as f32, dh as f32);
        gfx.resize_at(0.0, 0.0, dw as f32, dh as f32);
        gfx.clear(paint::BACKGROUND);
        gfx.image(
            &picture,
            ox,
            oy,
            w as f32 * zoom,
            h as f32 * zoom,
            Rgba::WHITE,
        );
        window.gl_swap_window();
        frames += 1;
        if frames.is_multiple_of(300) {
            let secs = start.elapsed().as_secs_f64();
            println!("bench: {frames} frames in {secs:.1}s, {:.0} fps", frames as f64 / secs);
        }
    }
}

/// How much each backdrop style changes in a second.
///
/// `MOOSE_SDL_BENCH=motion`. Drawing at one frame a second costs almost nothing,
/// and whether it *looks* like one frame a second depends entirely on the style:
/// a slow drift is the same picture either way, and something with a fast edge
/// in it steps visibly. This measures which is which instead of leaving it to
/// be argued about — the average level a pixel moves between two frames a
/// second apart, at the style's own pace.
fn bench_motion(video: &sdl2::VideoSubsystem, window: &sdl2::video::Window, gfx: &mut Gfx) {
    let (w, h) = (PANEL.0, PANEL.1);
    let target = match unsafe { gfx::Offscreen::new(w, h) } {
        Ok(t) => t,
        Err(e) => {
            eprintln!("no offscreen: {e:#}");
            return;
        }
    };
    let mut rows: Vec<(String, f64, f64)> = Vec::new();
    for style in backdrop::STYLE_LIST {
        let Ok(mut art) = (unsafe { backdrop::Backdrop::build(video, style.0) }) else {
            continue;
        };
        art.scheme = *backdrop::scheme("midnight");
        let shot = |gfx: &mut Gfx, at: f32| -> Vec<u8> {
            unsafe {
                gfx.draw_onto(&target, |gfx| {
                    gfx.clear(paint::BACKGROUND);
                    art.draw(w as f32, h as f32, at);
                });
            }
            let mut raw = vec![0u8; (w * h * 4) as usize];
            unsafe {
                gl::BindFramebuffer(gl::FRAMEBUFFER, target.frame_id());
                gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
                gl::ReadPixels(
                    0,
                    0,
                    w as i32,
                    h as i32,
                    gl::RGBA,
                    gl::UNSIGNED_BYTE,
                    raw.as_mut_ptr() as *mut _,
                );
                gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            }
            raw
        };
        // A second apart: how far this style travels between two frames if it
        // is drawn once a second.
        let a = shot(gfx, 10.0);
        let b = shot(gfx, 11.0);
        // Mean *and* worst. A few stars crossing the screen move almost no
        // average level and are the most obvious thing on it, so an average on
        // its own would call the jumpiest style the smoothest.
        let step = |x: &[u8], y: &[u8]| -> (f64, f64) {
            let n = x.len() / 4 * 3;
            let mut sum = 0u64;
            let mut worst = 0u64;
            for (p, q) in x.chunks(4).zip(y.chunks(4)) {
                for i in 0..3 {
                    let d = (p[i] as i32 - q[i] as i32).unsigned_abs() as u64;
                    sum += d;
                    worst = worst.max(d);
                }
            }
            (sum as f64 / n as f64, worst as f64)
        };
        let (mean_s, worst_s) = step(&a, &b);
        rows.push((style.1.to_owned(), mean_s, worst_s));
    }
    window.gl_swap_window();
    rows.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    println!("{:<22} {:>10} {:>10}", "style", "mean/s", "worst/s");
    for (label, mean, worst) in rows {
        println!("{label:<22} {mean:>10.2} {worst:>10.0}");
    }
}

/// Everything that arrived: what the wait woke for, then what was queued
/// behind it.
///
/// One line, and a function only so it can be tested. The bug it exists to
/// prevent is not a hard one to see once it is named — the event that ends the
/// wait is *consumed* by the wait — and it was still shipped, because the loop
/// reads perfectly well without it and the symptom is "the mouse stopped
/// working" rather than anything about events.
fn collect<E>(woke: Option<E>, queued: impl IntoIterator<Item = E>) -> Vec<E> {
    woke.into_iter().chain(queued).collect()
}

/// Where the cursor is drawn, which is not always where it is.
///
/// The cursor jumped: pressing down moved a bright bar one row instantly, and
/// on a list of two thousand games that is the whole screen changing with
/// nothing to say which way it went. It slides now — over 90 milliseconds,
/// which is short enough that it never delays anything and long enough that the
/// eye follows the bar rather than re-finding it.
///
/// Only the drawing. What is *selected* changes the instant the button is
/// pressed, because a settings screen that acts on a value 90 milliseconds late
/// is a settings screen that feels broken.
#[derive(Default)]
struct Gliding {
    /// Where it was drawn last frame.
    from: Option<Rect>,
    /// Where it is going, and when it set off.
    to: Option<Rect>,
    started: f64,
}

impl Gliding {
    /// How long the slide lasts.
    const MS: f64 = 90.0;

    /// The rectangle to draw this frame, given where the cursor really is.
    fn at(&mut self, target: Rect, now: f64, animate: bool) -> Rect {
        if !animate {
            self.from = Some(target);
            self.to = Some(target);
            return target;
        }
        let same = |a: Option<Rect>, b: Rect| {
            a.map(|r| (r.x - b.x).abs() < 0.5 && (r.y - b.y).abs() < 0.5 && (r.h - b.h).abs() < 0.5)
                .unwrap_or(false)
        };
        if !same(self.to, target) {
            // Set off from wherever it is *now*, not from where the last move
            // was going — interrupting a slide half way should carry on from
            // there rather than snap back.
            self.from = Some(self.showing(now));
            self.to = Some(target);
            self.started = now;
        }
        self.showing(now)
    }

    fn showing(&self, now: f64) -> Rect {
        let (Some(from), Some(to)) = (self.from, self.to) else {
            return self.to.or(self.from).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        };
        let t = ((now - self.started) / Self::MS).clamp(0.0, 1.0) as f32;
        // Eased out, like the page transition: quick away, settling in.
        let e = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
        Rect::new(
            from.x + (to.x - from.x) * e,
            from.y + (to.y - from.y) * e,
            from.w + (to.w - from.w) * e,
            from.h + (to.h - from.h) * e,
        )
    }

    /// Whether it still has somewhere to get to, so the loop keeps drawing.
    fn moving(&self, now: f64) -> bool {
        self.to.is_some() && now - self.started < Self::MS
    }
}

/// A page on its way out, and which way it is going.
struct Slide {
    started: f64,
}

fn main() -> Result<()> {
    // Where the app's files are. Config, cache and library are all addressed
    // relative to one directory, and which one depends on how this was
    // started — see `moose_rack::datadir`. Without it `cache.sqlite3` is
    // opened relative to whatever directory the binary was launched from, and
    // `Cache::open` *creates* the file, so instead of failing it makes an
    // empty database and shows a library of nothing.
    moose_rack::datadir::anchor();

    // The renderer has to be the GL one, because the backdrop is a shader and
    // shaders need a context to be in. SDL picks Metal first on macOS and
    // would give us one we cannot draw into; the handheld has only GL anyway,
    // so this makes both machines the same rather than special-casing one.
    let sdl = sdl2::init()
        .map_err(anyhow::Error::msg)
        .context("starting SDL")?;
    let video = sdl
        .video()
        .map_err(anyhow::Error::msg)
        .context("opening the display")?;

    // Ask for the context the shader needs, *before* the window is made —
    // SDL fixes the attributes at creation and there is no changing them
    // after. Left alone, macOS hands back a legacy 2.1 context and the shader
    // fails with "version '330' is not supported", which is a true statement
    // about a context nobody asked for.
    //
    // Core 3.3 on a desktop and GLES 3.0 on the handheld: the two dialects the
    // backdrop is written in, and the only difference between them is the
    // version line.
    {
        let attr = video.gl_attr();
        if cfg!(any(target_os = "android", target_os = "linux")) {
            attr.set_context_profile(sdl2::video::GLProfile::GLES);
            attr.set_context_version(3, 0);
        } else {
            attr.set_context_profile(sdl2::video::GLProfile::Core);
            attr.set_context_version(3, 3);
        }
    }

    let window = open_window(&video)
        .map_err(anyhow::Error::msg)
        .context("opening a window")?;

    // The config, read once. Everything the front end needs from it is settled
    // before the first frame; nothing below re-reads a file.
    // This front end's own file — see `settings::FILE` for why it is not
    // `config.toml`.
    let (config, config_from) = settings::load();
    println!("config: {config_from}");

    // The context, ours. Everything below draws through it.
    let _context = window
        .gl_create_context()
        .map_err(anyhow::Error::msg)
        .context("creating an OpenGL context")?;
    window
        .gl_set_context_to_current()
        .map_err(anyhow::Error::msg)?;
    let mut gfx = unsafe { Gfx::new(&video) }.context("setting up the renderer")?;
    println!(
        "gl: {} · {}",
        unsafe { gfx::reported(gl::VERSION) },
        unsafe { gfx::reported(gl::SHADING_LANGUAGE_VERSION) }
    );
    // Vertical sync, so a menu does not run the fan up.
    let _ = video.gl_set_swap_interval(sdl2::video::SwapInterval::VSync);

    let mut painter = text::Painter::new().context("finding fonts")?;
    check_fonts(&mut painter);

    // The shader backdrop, underneath everything. Built after the renderer, so
    // the context it compiles against is the one the renderer made current.
    //
    // Not fatal: a machine whose driver will not compile it still gets a
    // library, and the message says why rather than the window being black.
    // The frosted panels. Not fatal either: a driver that will not draw into a
    // texture still gets a library, with flat panels instead of glass.
    let mut frosted = match unsafe { glass::Glass::new(PANEL.0, PANEL.1) } {
        Ok(mut g) => {
            g.strength = config.appearance.glass as f32 / 20.0;
            Some(g)
        }
        Err(e) => {
            eprintln!("no glass: {e:#}");
            None
        }
    };

    // The style and its tuning come from the config, so the Appearance settings
    // are the ones that draw this rather than a second set that agrees by
    // coincidence.
    let mut backdrop =
        match unsafe { backdrop::Backdrop::build(&video, &config.appearance.backdrop) } {
            Ok(mut b) => {
                b.scheme = *backdrop::scheme(&config.appearance.scheme);
                b.speed = config.appearance.backdrop_speed as f32 / 100.0;
                b.strength = config.appearance.backdrop_strength as f32 / 100.0;
                println!("backdrop: {}", b.style_label());
                Some(b)
            }
            Err(e) => {
                eprintln!("no backdrop: {e:#}");
                None
            }
        };

    // Box art, from the same folder the other front ends fill. Nothing is
    // fetched: what is on disk is drawn and what is not is a flat card.
    let mut art = covers::Covers::new(
        config.media_dir(),
        // Where the games actually are. On a handheld the device fixes this —
        // `/userdata/roms` — and the config's answer is a default nobody set,
        // which is the wrong place to look for artwork scraped beside a ROM.
        moose_rack::platform::current()
            .default_library()
            .map(|l| l.roms)
            .unwrap_or_else(|| config.local_roms_dir()),
        config.media.list_art.clone(),
        (config.icons.style.clone(), config.icons.set.clone()),
    );

    // The library, straight out of the metadata cache the other front ends
    // read. Nothing is fetched and nothing is written.
    let mut lib = library::Library::open(
        std::path::Path::new("cache.sqlite3"),
        config.media_dir(),
        config.local_roms_dir(),
        config.library.server_collections,
    )
    .context("opening the library")?;
    // Nothing in the cache: build one from the card.
    //
    // This used to print a warning telling you to run a command at a terminal,
    // which on a handheld is a sentence with nowhere to type it. Every device
    // this front end is for arrives with ROMs already on it and no cache at
    // all, so the first run makes one — no server, no network, just the ROM
    // directories the device already has and the core map that says which
    // console each of them is.
    //
    // Only when it is empty. A library that has been synced from RomM knows
    // things a scan of the card cannot — cover art, ratings, play time — and
    // replacing it every launch would throw all of that away.
    let mut rescan = if !lib.has_cached_library()
        && moose_rack::platform::current().default_library().is_some()
    {
        println!("no cached library; looking at what is on the card");
        Some(rescan::Rescan::start(std::path::PathBuf::from("cache.sqlite3")))
    } else {
        None
    };
    println!("{} consoles", lib.consoles.len());
    // The consoles this library actually holds, so the Emulators pane offers
    // rows for those and not for the whole core map.
    let consoles: Vec<(String, String)> = lib
        .consoles
        .iter()
        .map(|c| (c.slug.clone(), c.name.clone()))
        .collect();
    let core_map = moose_rack::coremap::CoreMap::load_or_embedded(std::path::Path::new(
        "data/esde-core-map.json",
    ));
    lib.panes = settings::panes(&config, &consoles, &core_map);
    start_at(&mut lib);

    // The device's screen, as a texture. Everything is drawn here and this is
    // what the window shows, magnified.
    let panel = unsafe { gfx::Offscreen::new(PANEL.0, PANEL.1) }
        .context("this driver will not draw into a 640x480 texture")?;
    panel.texture.nearest();
    // The page being left behind, held while it slides off.
    //
    // A copy rather than a re-render: the old page cannot be drawn again once
    // the state has moved on, and it is already sitting in a texture. Nothing
    // is spent here until a tab actually changes.
    let leaving = unsafe { gfx::Offscreen::new(PANEL.0, PANEL.1) }
        .context("this driver will not draw into a 640x480 texture")?;
    leaving.texture.nearest();
    let mut slide: Option<Slide> = None;
    let mut was_section = lib.section;

    let mut screen = viewport(&window);
    say_where_we_are(&screen);

    // The bindings the desktop app writes. Resolved once — the loop below runs
    // at the display's refresh rate, and this is a scan of two tables.
    let bindings = config.bindings.clone();
    // Read through the face-button swap, which is this app's own and goes no
    // further — see .
    let pad_map = bindings.pad_map_swapped(config.controllers.swap_ab, config.controllers.swap_xy);

    let controller = sdl.game_controller().map_err(anyhow::Error::msg)?;
    let joysticks = sdl.joystick().map_err(anyhow::Error::msg)?;
    let mut pads = input::Pads::open_first(&controller, &joysticks);
    let mut repeat = padpoll::Repeat::default();

    let timer = sdl
        .timer()
        .map_err(anyhow::Error::msg)
        .context("starting the clock")?;
    let mut events = sdl.event_pump().map_err(anyhow::Error::msg)?;
    // The floor, when asked for: a window and one texture, nothing else. The
    // number every other measurement here is relative to.
    match std::env::var("MOOSE_SDL_BENCH").as_deref() {
        Ok("image") => {
            bench_image(&window, &mut gfx, &mut events);
            return Ok(());
        }
        Ok("motion") => {
            bench_motion(&video, &window, &mut gfx);
            return Ok(());
        }
        Ok("pads") => {
            let joystick = sdl.joystick().map_err(anyhow::Error::msg)?;
            bench_pads(&controller, &joystick, &mut events);
            return Ok(());
        }
        _ => {}
    }
    // What the pad is holding, so the drawing can show that input arrived.
    let mut held: BTreeSet<String> = BTreeSet::new();

    // Where each thing was drawn last frame, so the mouse knows what it is
    // over. Filled by the drawing and read by the next event — which is the
    // only order that works, since what is on screen is decided while drawing
    // it.
    let mut hits: Hits = Hits::default();
    // What the pointer is over, which is not what is chosen.
    let mut hover: Option<usize> = None;

    // The open text field, when something asked for one. While it is here the
    // pad drives the keyboard and nothing else — a field you are typing into
    // owns the input until it is finished with.
    let mut typing: Option<keyboard::Keyboard> = demo_keyboard();
    // The corner: clock, signal, charge. Read on a timer, not per frame.
    let mut status = status::Status::default();
    // What the renderer is currently set to, so a change can be spotted without
    // rebuilding a shader every frame.
    // Seeded from what the renderer was actually built with, not from the
    // settings entries. Reading the entries here makes the two agree by
    // definition — including when they do not, which is how a change made
    // before the loop starts was written to the file and never drawn.
    let mut showing = settings::Look {
        backdrop: config.appearance.backdrop.clone(),
        scheme: config.appearance.scheme.clone(),
        speed: config.appearance.backdrop_speed as f32 / 100.0,
        strength: config.appearance.backdrop_strength as f32 / 100.0,
        glass: config.appearance.glass,
        animations: config.appearance.animations,
    };

    let mut frames = 0u32;

    // Whether anything has happened that the screen does not already show.
    //
    // The loop used to draw every frame forever: on this machine that was 75%
    // of a core to display a settings list that had not changed in a minute,
    // and on a handheld it is the battery. A frame here is a few hundred
    // separate draw calls, so the cheapest one is the one not made.
    //
    // Starts true — the first frame has never been drawn.
    let mut dirty = true;
    let mut drawn_at = 0.0f64;
    // Where the cursor is drawn, which lags where it is by a few frames.
    let mut glide = Gliding::default();
    // The corner as the screen last showed it. The clock changing is a reason
    // to redraw and nothing else notices it.
    let mut corner = status.parts();

    'running: loop {
        let now = ticks(&timer);

        // Wait for something rather than spin.
        //
        // How long depends on what is due next. A moving backdrop is due at its
        // own rate; everything else is due when it happens, and the timeout is
        // only there so the clock, a finishing download and a held button still
        // get their turn.
        //
        // Skipping a frame without waiting is not the same as waiting: the
        // first version of this returned to the top of the loop, found nothing
        // to do, and spun there — which cost *more* than drawing every frame.
        let animating = backdrop.is_some() && showing.speed > 0.0;
        // How often this backdrop actually needs redrawing. Per style, from
        // measurement — see `backdrop::needed_fps`.
        let motion = motion_fps(&showing.backdrop, showing.speed);
        // How long there is to wait before something is due.
        //
        // A held button is the short one: nothing arrives while a direction is
        // down — the state is polled, not sent — so the loop has to come back
        // often enough for auto-repeat to feel like auto-repeat. Not "not at
        // all", which is what it was, and which spun a core for as long as a
        // direction was held.
        let wait = if dirty {
            0
        } else if !held.is_empty() {
            HELD_WAIT_MS
        } else if animating {
            ((1000.0 / motion - (now - drawn_at)).max(1.0) as u32).min(IDLE_WAIT_MS)
        } else {
            IDLE_WAIT_MS
        };
        // Waiting *takes* the event it woke for. Dropping it — which is what
        // the first version of this did — means the first press of any
        // interaction is swallowed, and since the app is asleep between
        // interactions the first press is every press. The mouse and the pad
        // both stopped working, and neither symptom pointed at the event loop.
        let woke = (wait > 0)
            .then(|| events.wait_event_timeout(wait))
            .flatten();
        let arrived = collect(woke, events.poll_iter());

        for event in arrived {
            // Every event is a reason to look again. Being wrong in this
            // direction costs one frame; being wrong the other way leaves a
            // key press with nothing on screen to show for it.
            dirty = true;
            match event {
                Event::Quit { .. } => break 'running,
                Event::Window {
                    win_event: WindowEvent::SizeChanged(..),
                    ..
                }
                | Event::Window {
                    win_event: WindowEvent::Moved(..),
                    ..
                } => {
                    // Moved as well as resized: dragging a window between a
                    // retina display and a plain one changes the scale
                    // without changing the size in points.
                    screen = viewport(&window);
                    say_where_we_are(&screen);
                }
                // The mouse. ES-DE makes this hard and it is the complaint
                // Frank had about it; here it is the same cursor the pad
                // moves, pointed at instead of stepped to.
                // Moving the pointer lights a row up. It does not *choose*
                // it: choosing scrolls the list to keep the choice on screen,
                // the scroll moves a different row under the pointer, and the
                // next motion event chooses that one — so the grid ran away
                // from the mouse. `.row:hover` in the stylesheet is a
                // background color and nothing else, for the same reason.
                Event::MouseMotion { x, y, .. } => {
                    let (x, y) = pointer(&window, x, y);
                    hover = hits.at(x, y);
                }
                Event::MouseButtonDown {
                    mouse_btn, x, y, ..
                } => match mouse_btn {
                    sdl2::mouse::MouseButton::Left => {
                        let (x, y) = pointer(&window, x, y);
                        if let Some(action) = hits.button_at(x, y) {
                            act(&mut lib, action);
                        } else if let Some(tab) = hits.tab_at(x, y) {
                            lib.go_to_section(tab);
                        } else if let Some(at) = hits.at(x, y) {
                            lib.point_at(at);
                            // A click opens what it is on, which is what a
                            // double-click does in the webview — but there is
                            // nothing else a click on a console could mean.
                            act(&mut lib, "activate");
                        }
                    }
                    // The right button goes back, the way it does in every
                    // file manager.
                    sdl2::mouse::MouseButton::Right => {
                        if lib.at_top() {
                            break 'running;
                        }
                        act(&mut lib, "back");
                    }
                    _ => {}
                },
                Event::MouseWheel { y, .. } => {
                    // A wheel notch is a row, not a pixel: the cursor is what
                    // scrolls here, and a list that moves without it leaves
                    // the selection off screen.
                    let step = if y > 0 { "up" } else { "down" };
                    for _ in 0..y.unsigned_abs().min(4) {
                        act(&mut lib, step);
                    }
                }
                Event::KeyDown {
                    keycode: Some(key),
                    repeat: false,
                    ..
                } => {
                    // Escape is bound to Back, not to quit — it was hardcoded
                    // here in the first commit and never taken out, so going
                    // back from a console closed the app instead. The window
                    // closes the window; Cmd-Q and Alt-F4 do what they always
                    // do.
                    if let Some(action) = input::action_for_key(&bindings, key) {
                        if let Some(kb) = typing.as_mut() {
                            match kb.act(action) {
                                keyboard::Outcome::Done => {
                                    let text = kb.text.clone();
                                    if let Some(target) = kb.target.clone() {
                                        lib.take_typed(&target, &text);
                                    }
                                    typing = None;
                                }
                                keyboard::Outcome::Cancelled => typing = None,
                                keyboard::Outcome::Typing => {}
                            }
                            continue;
                        }
                        // Back at the top level leaves, which is what Back
                        // means on a handheld with no window to close and no
                        // Cmd-Q to press. Nothing is lost by it: this browses.
                        if matches!(action, "back" | "back2") && lib.at_top() {
                            break 'running;
                        }
                        act(&mut lib, action);
                    }
                }
                _ => {}
            }
        }

        // The pad, read whole rather than as events: how far a stick is pushed
        // and how long a button has been down are states, not edges, and
        // `padpoll` is written against them.
        // While a control is being captured, a button is a button and not
        // whatever it is bound to — that is the whole point of capturing.
        if lib.capturing.is_some()
            && let Some(button) = pads.any_button()
        {
            match button {
                // B clears, Start leaves it alone. Everything else binds.
                1 => lib.capture_button(None),
                9 => lib.cancel_capture(),
                other => lib.capture_button(Some(other)),
            }
            repeat.release(&BTreeSet::new());
            continue;
        }

        let pressed = pads.pressed(&pad_map);
        for action in &pressed {
            if repeat.fire(action, now) {
                if let Some(kb) = typing.as_mut() {
                    match kb.act(action) {
                        keyboard::Outcome::Done => {
                            let text = kb.text.clone();
                            if let Some(target) = kb.target.clone() {
                                lib.take_typed(&target, &text);
                            }
                            typing = None;
                        }
                        keyboard::Outcome::Cancelled => typing = None,
                        keyboard::Outcome::Typing => {}
                    }
                    continue;
                }
                // B at the top level does nothing now.
                //
                // It used to quit, and the help bar called it "back" — so the
                // one irreversible action on the device was on an unlabelled
                // button, next to a label saying it did something else. Leaving
                // is a row in Settings, where it says what it does.
                if matches!(action.as_str(), "back" | "back2") && lib.at_top() {
                    continue;
                }
                act(&mut lib, action);
                if lib.quitting {
                    break 'running;
                }
                dirty = true;
            }
        }
        repeat.release(&pressed);
        held.clone_from(&pressed);

        if typing.is_none()
            && let Some(kb) = keyboard_wanted(&mut lib)
        {
            typing = Some(kb);
        }
        // A scan or a join, finishing on its thread. Polled here rather than
        // in the drawing so a slow helper never holds up a frame.
        if let Some(job) = lib.wifi_job.as_ref()
            && let Some(state) = job.poll()
        {
            lib.wifi = state;
            lib.wifi_job = None;
            lib.wifi_at = 0;
            lib.refresh_rows();
            dirty = true;
        }
        // The panel tint follows the color scheme, like the webview's does.
        let g = backdrop::glass_of(&showing.scheme);
        let glass_tint = Rgba(g[0] * 0.55, g[1] * 0.55, g[2] * 0.62, 0.30);

        // A setting is not a setting until it does something. The Appearance
        // pane writes the file; this is what makes the screen change while you
        // are looking at it.
        let want = settings::look(&lib.panes);
        if want != showing {
            if want.backdrop != showing.backdrop {
                match unsafe { backdrop::Backdrop::build(&video, &want.backdrop) } {
                    Ok(b) => backdrop = Some(b),
                    Err(e) => eprintln!("no backdrop {}: {e:#}", want.backdrop),
                }
            }
            if let Some(g) = frosted.as_mut() {
                // 0 to 60 on the slider, the same range the webview uses, onto
                // how far each tap reaches.
                g.strength = want.glass as f32 / 20.0;
            }
            if let Some(b) = backdrop.as_mut() {
                b.scheme = *backdrop::scheme(&want.scheme);
                b.speed = want.speed;
                b.strength = want.strength;
            }
            showing = want;
            dirty = true;
        }

        status.poll(now as u64);
        if status.parts() != corner {
            corner = status.parts();
            dirty = true;
        }

        // A download reports on its own row, and the row has to be redrawn to
        // say so.
        if lib.fetching.is_some() {
            dirty = true;
        }
        // The card being read, on its first run. When it lands the library is
        // reopened, because every list on every tab was built from a cache that
        // has just been replaced.
        let scanning = rescan.as_mut().map(|j| j.note()).unwrap_or_default();
        if let Some(job) = rescan.as_mut() {
            job.poll();
            if job.finished() {
                if let rescan::Progress::Failed(why) = job.poll() {
                    eprintln!("scan: {why}");
                }
                println!("{}", job.note());
                match library::Library::open(
                    std::path::Path::new("cache.sqlite3"),
                    config.media_dir(),
                    config.local_roms_dir(),
                    config.library.server_collections,
                ) {
                    Ok(mut fresh) => {
                        // What the screen was showing, kept: a scan finishing
                        // is not a reason for the cursor to jump to the top of
                        // another tab.
                        fresh.go_to_section(lib.section);
                        fresh.panes = std::mem::take(&mut lib.panes);
                        lib = fresh;
                    }
                    Err(e) => eprintln!("reopening the library: {e:#}"),
                }
                rescan = None;
            }
            dirty = true;
        }

        // Sample pictures arriving one at a time, each of which changes the
        // sheet.
        if let Some(previews) = lib.previews.as_mut() {
            let before = previews.found.len();
            if previews.poll().len() != before {
                dirty = true;
            }
        }

        // A backstop against an invalidation nobody thought of. Half a second
        // of a stale screen is invisible on a settings list and costs two
        // frames a second when the app is otherwise asleep — which is the
        // trade this whole arrangement is making.
        if now - drawn_at > STALE_MS {
            dirty = true;
        }
        // A tab changed: keep what is on screen and slide it out.
        if lib.section != was_section {
            if showing.animations {
                unsafe {
                    gfx.draw_onto(&leaving, |gfx| {
                        gfx.image_part(
                            &panel.texture,
                            0.0,
                            0.0,
                            PANEL.0 as f32,
                            PANEL.1 as f32,
                            (0.0, 1.0, 1.0, 0.0),
                            gfx::Rgba::WHITE,
                        );
                    });
                }
                slide = Some(Slide { started: now });
            }
            was_section = lib.section;
            dirty = true;
        }
        // A slide is motion, so it draws every frame it lasts. So is the
        // cursor on its way to a new row.
        if slide.is_some() || glide.moving(now) {
            dirty = true;
        }
        // The selection pulses on the console grid, which is motion like any
        // other — see the outline in `draw_consoles`.
        if showing.animations && lib.section().id == "library" && lib.view == library::View::Platforms {
            dirty = true;
        }

        // A moving backdrop redraws on its own clock rather than the display's.
        if !dirty && (!animating || now - drawn_at < 1000.0 / motion) {
            continue;
        }
        dirty = false;
        drawn_at = now;
        let (dw, dh) = window.drawable_size();
        let (ox, oy, zoom) = panel_box(dw as f32, dh as f32);

        // The backdrop is drawn twice, deliberately. Once small, into a
        // texture that is then blurred — which is what the panels sample —
        // and once at full size behind everything. Reading the finished frame
        // back would save the second draw and cannot be done before the frame
        // exists, which is the whole reason `backdrop-filter` is expensive in
        // a browser too.
        let seconds = now as f32 / 1000.0;
        if let (Some(glass), Some(backdrop)) = (&mut frosted, &backdrop) {
            unsafe {
                let _ = glass.resize(screen.width_px as u32, screen.height_px as u32);
                let (w, h) = glass.blurred_size();
                glass.capture(&mut gfx, |_| backdrop.draw(w as f32, h as f32, seconds));
            }
        }

        // The whole page, into 640x480 pixels — the device's own count.
        //
        // Drawing straight at the window and scaling the *layout* instead is
        // what made this a preview of a screen nobody has: the boxes landed in
        // the right places and every glyph was rasterized four times larger
        // than the handheld will ever draw it. Whatever comes out of here is
        // what comes out on the Flip.
        unsafe {
            gfx.draw_onto(&panel, |gfx| {
                gfx.clear(paint::BACKGROUND);
                if let Some(backdrop) = &backdrop {
                    backdrop.draw(screen.width_px, screen.height_px, seconds);
                }
                hits = draw(
                    gfx,
                    frosted.as_ref(),
                    &mut painter,
                    &mut art,
                    &mut lib,
                    &screen,
                    hover,
                    typing.as_ref(),
                    &status,
                    glass_tint,
                    &scanning,
                    &mut glide,
                    now,
                    showing.animations,
                );
            });
        }

        // And out to the window, magnified with no smoothing, letterboxed.
        gfx.resize_at(0.0, 0.0, dw as f32, dh as f32);
        gfx.clear(paint::BACKGROUND);
        let (pw, ph) = (PANEL.0 as f32 * zoom, PANEL.1 as f32 * zoom);
        // Bottom row first: a texture drawn into through a framebuffer comes
        // out with GL's origin, and everything else here is measured from the
        // top left. Sampling it upside down puts the two back in agreement.
        let flip = (0.0, 1.0, 1.0, 0.0);
        // Where the tab row stops and the page begins, as a fraction of the
        // panel. The two halves of the transition are different things: the
        // tabs slide, because the movement is what says which way you went, and
        // the page under them fades, because a whole screenful of list sliding
        // past is motion nobody asked to read.
        let split = size::TABS / PANEL.1 as f32;
        let band = |from: f32, to: f32| (0.0, 1.0 - from, 1.0, 1.0 - to);
        match slide.as_ref() {
            Some(s) => {
                let t = ((now - s.started) / SLIDE_MS).clamp(0.0, 1.0) as f32;
                // Eased out: fast at the start, settling at the end, which is
                // how a thing with weight arrives.
                let eased = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
                if t >= 1.0 {
                    slide = None;
                    gfx.image_part(&panel.texture, ox, oy, pw, ph, flip, gfx::Rgba::WHITE);
                } else {
                    let (top_h, body_h) = (ph * split, ph * (1.0 - split));
                    let body_y = oy + top_h;

                    // The tab row does not move.
                    //
                    // It used to slide, on the reasoning that the movement says
                    // which way you went. On a handheld it does not: the row is
                    // twenty-six points of a 480-point screen, the thing that
                    // says which tab you are on is the mark under it, and a
                    // strip of text sliding past above a page that is fading is
                    // two animations disagreeing about what happened.
                    gfx.image_part(
                        &panel.texture,
                        ox,
                        oy,
                        pw,
                        top_h,
                        band(0.0, split),
                        gfx::Rgba::WHITE,
                    );

                    // The page, fading: the old one under, the new one over it
                    // at rising opacity. Both in place — nothing here moves.
                    let body_band = band(split, 1.0);
                    gfx.image_part(
                        &leaving.texture,
                        ox,
                        body_y,
                        pw,
                        body_h,
                        body_band,
                        gfx::Rgba::WHITE,
                    );
                    gfx.image_part(
                        &panel.texture,
                        ox,
                        body_y,
                        pw,
                        body_h,
                        body_band,
                        gfx::Rgba(1.0, 1.0, 1.0, eased),
                    );
                }
            }
            None => {
                gfx.image_part(&panel.texture, ox, oy, pw, ph, flip, gfx::Rgba::WHITE);
            }
        }
        window.gl_swap_window();

        // A portrait, and then out. Taken after a handful of frames rather
        // than the first: covers are decoded as they are asked for, so frame
        // one is a picture of an empty cache.
        if let Some((path, _)) = shot_wanted() {
            frames += 1;
            // Longer than eight when asked: taking a portrait is one job, and
            // watching what the process weighs while it draws is another.
            let want: u32 = std::env::var("MOOSE_SDL_FRAMES")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(8);
            if frames >= want {
                unsafe { gl::Finish() };
                save_shot(&window, &path).with_context(|| format!("saving {path}"))?;
                println!("shot: {path}");
                break 'running;
            }
        }
    }
    Ok(())
}

/// Where the pointer is, in the pixels everything is drawn in.
///
/// SDL reports the mouse in *window* coordinates and we draw in *drawable*
/// ones, and on a retina display those differ by two — so a click near the
/// middle of the window lands a quarter of the way in, and one in the bottom
/// half lands nowhere at all. Not the layout's scale, which is about how big
/// things should look; this is about which pixels the platform means.
fn pointer(window: &sdl2::video::Window, x: i32, y: i32) -> (f32, f32) {
    let (dw, dh) = window.drawable_size();
    let (ww, wh) = window.size();
    let across = if ww > 0 { dw as f32 / ww as f32 } else { 1.0 };
    let down = if wh > 0 { dh as f32 / wh as f32 } else { 1.0 };
    // Into the panel's own pixels. Everything is drawn 640x480 and then
    // magnified into the middle of the window, so a click has to come back the
    // other way through both — the letterbox offset and the zoom — or the
    // pointer lands a quarter of the screen from where it looks.
    let (ox, oy, zoom) = panel_box(dw as f32, dh as f32);
    (
        (x as f32 * across - ox) / zoom,
        (y as f32 * down - oy) / zoom,
    )
}

/// A portrait of one frame, written to a file, and then nothing.
///
/// `MOOSE_SDL_SHOT=/tmp/a.png moose-sdl` draws the interface once and saves it.
/// It exists because the alternative was Frank opening the app and describing
/// what looked wrong, and a description is not a pixel — the console grid
/// spilling out of its panel took four rounds of that, and one look at a file
/// to find.
///
/// `MOOSE_SDL_SIZE=1460x1046` sets the window; the default is the handheld's.
fn shot_wanted() -> Option<(String, (u32, u32))> {
    let path = std::env::var("MOOSE_SDL_SHOT").ok()?;
    let size = std::env::var("MOOSE_SDL_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once(['x', 'X'])?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or((PANEL.0 * PREVIEW / 2, PANEL.1 * PREVIEW / 2));
    Some((path, size))
}

/// Read the finished frame back and write it out.
///
/// GL hands rows back bottom-up, so they are flipped — the sort of thing that
/// makes a picture look fine until you read a word in it.
fn save_shot(window: &sdl2::video::Window, path: &str) -> Result<()> {
    use sdl2::image::SaveSurface;
    let (w, h) = window.drawable_size();
    let mut flipped = vec![0u8; (w * h * 4) as usize];
    unsafe {
        gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
        gl::ReadPixels(
            0,
            0,
            w as i32,
            h as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            flipped.as_mut_ptr() as *mut _,
        );
    }
    let stride = (w * 4) as usize;
    let mut rows = Vec::with_capacity(flipped.len());
    for row in (0..h as usize).rev() {
        rows.extend_from_slice(&flipped[row * stride..row * stride + stride]);
    }
    let surface = sdl2::surface::Surface::from_data(
        &mut rows,
        w,
        h,
        stride as u32,
        sdl2::pixels::PixelFormatEnum::ABGR8888,
    )
    .map_err(anyhow::Error::msg)?;
    surface.save(path).map_err(anyhow::Error::msg)?;
    Ok(())
}

fn open_window(video: &sdl2::VideoSubsystem) -> Result<sdl2::video::Window, String> {
    let shot = shot_wanted();
    let (w, h) = shot
        .as_ref()
        .map(|(_, size)| *size)
        .unwrap_or((PANEL.0 * PREVIEW / 2, PANEL.1 * PREVIEW / 2));
    let mut builder = video.window("Moose Rack", w, h);
    // Taking a portrait should not throw a window at whoever is using the
    // machine, so it happens off screen.
    if shot.is_some() {
        builder.hidden();
    }
    builder
        .position_centered()
        .resizable()
        // Retina and the like. Without this the window is described in points
        // by the platform and drawn at half the resolution it could be.
        .allow_highdpi()
        .opengl()
        .build()
        .map_err(|e| e.to_string())
}

/// Which tab and console to open on, for previews.
///
/// `MOOSE_SDL_TAB=library MOOSE_SDL_OPEN=megadrive` opens that console's games;
/// `MOOSE_SDL_TAB=mine MOOSE_SDL_OPEN=user` opens that kind of collection. Only
/// useful for judging a screen without pressing anything to get to it — the app
/// opens on the first tab otherwise.
fn start_at(lib: &mut library::Library) {
    if let Ok(want) = std::env::var("MOOSE_SDL_TAB")
        && let Some(i) = library::SECTIONS.iter().position(|s| s.id == want)
    {
        lib.go_to_section(i);
    }
    let Ok(open) = std::env::var("MOOSE_SDL_OPEN") else {
        return;
    };
    // What "open" names depends on the tab: a console in the Library, a kind
    // of collection in Collections.
    let err = if lib.section().id == "settings" {
        lib.pane_at = lib.panes.iter().position(|p| p.id == open).unwrap_or(0);
        lib.act("activate").map(|_| ())
    } else if lib.section().id == "mine" {
        lib.shelf_at = lib
            .shelves
            .iter()
            .position(|sh| sh.name().eq_ignore_ascii_case(&open))
            .unwrap_or(0);
        lib.open_shelf()
    } else {
        let at = open
            .parse::<usize>()
            .ok()
            .or_else(|| lib.consoles.iter().position(|c| c.slug == open))
            .unwrap_or(0);
        lib.console_at = at.min(lib.consoles.len().saturating_sub(1));
        // Through `activate`, not `open_console`: Ports and Tools are consoles
        // that open a list of scripts instead, and a preview that takes a
        // different path from the app is a preview of something else.
        lib.act("activate").map(|_| ())
    };
    if let Err(e) = err {
        eprintln!("MOOSE_SDL_OPEN={open}: {e:#}");
    }
    // Replay a few presses, for judging a screen that takes some getting to —
    // a list of options is three presses in and cannot be screenshotted
    // otherwise. `MOOSE_SDL_ACT=down,down,activate`, in the app's own action
    // names, through the app's own dispatcher.
    if let Ok(script) = std::env::var("MOOSE_SDL_ACT") {
        for action in script.split(',').map(str::trim).filter(|a| !a.is_empty()) {
            if let Err(e) = lib.act(action) {
                eprintln!("MOOSE_SDL_ACT {action}: {e:#}");
            }
        }
    }
}

/// Milliseconds since SDL started, which is what `padpoll` counts in.
///
/// The subsystem is held rather than asked for each frame: `Sdl::timer`
/// initializes it, and doing that sixty times a second for a number is work
/// for nothing — and if it ever failed, time would silently stop and the
/// backdrop would freeze with nothing to say why.
fn ticks(timer: &sdl2::TimerSubsystem) -> f64 {
    timer.ticks64() as f64
}

/// What the window is, in points.
///
/// The *drawable* size, not the window size: on a retina display those differ
/// by the backing scale, and the drawable is the one with pixels in it.
fn viewport(_window: &sdl2::video::Window) -> Viewport {
    // 640x480 at one to one, whatever the window is.
    //
    // It used to be the panel multiplied by the preview scale, which made the
    // *layout* right and the *drawing* a lie: a 12-point label rasterized at
    // 4x is a 48-pixel glyph, sharp and fully legible, and on the device it is
    // a 12-pixel glyph and may not be legible at all. Everything is drawn into
    // a 640x480 texture now, exactly as many pixels as the Flip has, and that
    // texture is what gets magnified — so what is on this screen is the device
    // screen enlarged rather than a different screen entirely.
    Viewport::new(PANEL.0 as f32, PANEL.1 as f32, Scale::new(ui_scale()))
}

/// Where the panel sits in the window, and how big a pixel is.
///
/// A whole-number scale, and letterboxed. `Scale::new` snaps to quarter steps
/// and clamps at four, so dividing the drawable by 640 gave 696 points on a
/// 2784-pixel window — a preview of a screen the device does not have, with a
/// different number of panes. Fitting the panel and leaving the rest of the
/// window blank is the only arrangement where what is on screen here is what
/// will be on screen there.
fn panel_box(w: f32, h: f32) -> (f32, f32, f32) {
    let fit = (w / PANEL.0 as f32)
        .min(h / PANEL.1 as f32)
        .floor()
        .max(1.0);
    let (pw, ph) = (PANEL.0 as f32 * fit, PANEL.1 as f32 * fit);
    (((w - pw) / 2.0).max(0.0), ((h - ph) / 2.0).max(0.0), fit)
}

/// Say what the machine can and cannot draw, once, at startup.
///
/// Both halves matter on hardware we did not build. A machine with no CJK face
/// turns every Japanese title into a row of empty boxes, and that does not look
/// like a missing font — it looks like the names are wrong, which is a bug
/// report about the library. And a card width that cuts every name short is
/// worth knowing before squinting at a 4" screen to find out.
fn check_fonts(painter: &mut text::Painter) {
    println!("{} faces installed", painter.faces());
    for probe in ["Metroid", "ゼルダの伝説", "Pokémon"] {
        if !painter.can_draw(probe) {
            eprintln!("warning: no installed face can draw {probe:?} — it will be drawn as boxes");
        }
    }
    // Which face each of the shared scripts is handed to, and whether it is
    // the one asked for. Chinese, Japanese and Korean share code points whose
    // correct shapes differ; `moose_rack::script` reads the title and names
    // the family, and this says whether the machine had it. A handheld with
    // only a pan-CJK fallback installed will say so here rather than quietly
    // drawing Japanese titles in Chinese forms.
    for (what, sample) in [
        ("Latin etc.", "Metroid"),
        ("Japanese", "ゼルダの伝説"),
        ("Chinese (S)", "塞尔达传说"),
        ("Chinese (T)", "薩爾達傳說"),
        ("Korean", "젤다의 전설"),
    ] {
        let asked = painter.family_for(sample).unwrap_or("(any)").to_owned();
        if let Some(face) = painter.face_for(sample) {
            let note = if face == asked || asked == "(any)" {
                ""
            } else {
                "  <- not what was asked for"
            };
            println!("  {what:<12} asked {asked:<20} got {face}{note}");
        }
    }
    // A title long enough that it must be cut, and one short enough that it
    // must not. Both, because an ellipsis that never appears and one that
    // always does are the same bug from opposite sides.
    let long = "Mortal Kombat II: The Very Long Subtitle Nobody Asked For, Special Edition";
    let at_card = |t: &str| text::Spec::new(t, size::LABEL, 1.0).wrapped(size::CARD, 2);
    if !painter.is_clipped(&at_card(long)) {
        eprintln!("warning: a title far too long for a card was not cut short");
    }
    if painter.is_clipped(&at_card("Metroid")) {
        eprintln!("warning: a short title was cut short");
    }
}

fn say_where_we_are(screen: &Viewport) {
    println!(
        "{:.0}x{:.0}px at {:.2}x -> {:.0}x{:.0}pt, {:.2}:1, {:?}",
        screen.width_px,
        screen.height_px,
        screen.scale.factor(),
        screen.width(),
        screen.height(),
        screen.aspect(),
        screen.panes(),
    );
}

/// Hand an action to the library, and say if anything came of it.
///
/// A binding that resolves to nothing is printed rather than swallowed: an
/// action nobody handles looks exactly like a button that is not working.
fn act(lib: &mut library::Library, action: &str) {
    match lib.act(action) {
        Ok(true) => {}
        Ok(false) => println!("nothing does {action} yet"),
        Err(e) => eprintln!("{action}: {e:#}"),
    }
}

/// A keyboard for whichever setting asked for one, if any.
///
/// Taken rather than read: one activation opens one keyboard, and leaving the
/// request in place would reopen it the moment the last one closed.
fn keyboard_wanted(lib: &mut library::Library) -> Option<keyboard::Keyboard> {
    let want = lib.wants_keyboard.take()?;
    Some(keyboard::Keyboard::new(want.prompt, &want.value, want.secret).filling(want.target))
}

/// A grid of cards, in points, and the columns the window has room for.
///
/// Not a design. It is the smallest thing that is wrong on screen if the units
/// are wrong: cards that change physical size when the window is dragged
/// between displays, or a column that appears at the wrong width.
/// The colors, in one place.
mod paint {
    use crate::gfx::Rgba;

    pub const BACKGROUND: Rgba = Rgba::rgb(14, 15, 20);
    /// Furniture: darker than a panel, so it does not compete with artwork.
    pub const BAR: Rgba = Rgba(0.05, 0.05, 0.08, 0.72);
    pub const CARD: Rgba = Rgba(0.14, 0.15, 0.21, 0.62);
    /// A console tile: a wash, not a plate, and no frost behind it.
    ///
    /// The picture on it is a machine rendered on nothing, so what should be
    /// behind it is the backdrop — moving, and its own color. Glass here reads
    /// as grey: a blur of a dark ground with the saturation pushed is grey
    /// whatever the scheme is, and a shelf of hardware turns into a shelf of
    /// grey rectangles with hardware printed on them.
    pub const TILE: Rgba = Rgba(0.10, 0.11, 0.16, 0.30);
    pub const CURSOR: Rgba = Rgba::rgb(96, 140, 210);
    pub const TEXT: Rgba = Rgba::rgb(232, 234, 242);
    pub const DIM: Rgba = Rgba::rgb(150, 154, 168);
    pub const FAINT: Rgba = Rgba::rgb(104, 108, 124);
    /// The plate a mark sits on, so it reads against artwork of any color.
    /// `.row:hover` — the pointer saying where it is, which is not the same
    /// as the cursor saying what is chosen.
    pub const HOVER: Rgba = Rgba(1.0, 1.0, 1.0, 0.07);
    pub const STAR: Rgba = Rgba::rgb(240, 200, 90);
    /// A hairline between two halves of one list. Faint on purpose: it is
    /// separating things, not labelling them.
    /// Behind a keyboard: dark enough that the page under it stops competing.
    pub const SCRIM: Rgba = Rgba(0.02, 0.02, 0.04, 0.95);
    /// The keyboard's own panel. Opaque, not the translucent BAR — a game list
    /// showing through the keys is exactly what makes them hard to read.
    pub const SHEET: Rgba = Rgba::rgb(18, 19, 26);
    pub const RULE: Rgba = Rgba(1.0, 1.0, 1.0, 0.13);
    /// On this machine, and on the server.
    pub const HERE: Rgba = Rgba::rgb(120, 200, 140);
    pub const AWAY: Rgba = Rgba::rgb(150, 160, 180);
}

/// The sizes, in points. Nothing here is a pixel.
mod size {
    use moose_rack::layout::Edges;

    // Everything here is in panel points, and the panel is 640x480. That is
    // the whole budget: a 42-point tab row is a *tenth of the screen height*,
    // which is what the desk-sized numbers these replaced were spending.
    pub const GAP: f32 = 6.0;
    /// The tab row. Six tabs across 640 points, so ~104 each.
    pub const TABS: f32 = 26.0;
    /// Air either side of a tab's own name, and between one tab and the next.
    /// Names set solid read as one long word at this size.
    pub const TAB_PAD: f32 = 7.0;
    pub const TAB_GAP: f32 = 5.0;
    /// How big a tab's name is. The same as a title, because a tab row is the
    /// most important line on the screen and used to be set smaller than the
    /// list under it.
    pub const TAB_TEXT: f32 = 13.0;
    /// The line under the tabs saying where you are. Off — see `draw_chrome`.
    #[allow(dead_code)]
    pub const HEADER: f32 = 20.0;
    /// What sits there instead: air.
    pub const HEADER_GAP: f32 = 6.0;
    /// The strip along the bottom saying what the buttons do.
    pub const HELP: f32 = 18.0;
    pub const CARD: f32 = 96.0;
    pub const LABEL: f32 = 12.0;
    pub const TITLE: f32 = 13.0;
    /// A list row. Tall enough to touch, short enough that a 434-point body
    /// shows sixteen of them.
    pub const ROW: f32 = 26.0;
    /// How many consoles across, when there is room for a grid at all.
    ///
    /// A count rather than a width. The tile takes whatever the panel divided
    /// by this comes to, so the same grid is the same shape on any screen —
    /// three across on a handheld is three across on a desk, drawn bigger.
    pub const GRID_COLUMNS: usize = 3;
    /// And how many down. Three by two on a handheld: six consoles at a glance,
    /// each big enough to recognise by its picture rather than by reading it.
    pub const GRID_ROWS: usize = 2;
    /// How much smaller a card is than its place in the grid.
    ///
    /// Air, not spacing: the grid's own gap separates the slots, and this makes
    /// the card sit inside its slot rather than fill it. Six cards edge to edge
    /// read as one panel with lines drawn on it.
    pub const TILE_INSET: f32 = 7.0;
    /// Below this much room per tile the grid becomes a list. Not the tile
    /// size — there is no fixed tile size any more.
    pub const TILE_MIN: f32 = 84.0;
    /// Two lines of console name — "Nintendo Entertainment System" needs
    /// both — then the game count under it, then the tile's own padding.
    /// Reserving less is what made the count bleed out of the tile and sit
    /// under the row below.
    pub const TILE_CAPTION: f32 = LABEL * 1.3 * 2.0 + 4.0 + 14.0 + 10.0;
    /// How round a corner is. Generous on purpose: at this element scale a
    /// six-point radius on a hundred-point card is very nearly a square, and
    /// the cards on the device's own theme are visibly rounded.
    pub const ROUND: f32 = 11.0;
    pub const ROUND_SMALL: f32 = 7.0;
    pub const PAD: Edges = Edges::all(GAP);
}

/// Where each thing was drawn, so the mouse knows what it is over.
///
/// Rebuilt every frame, because that is when the answer is known, and in
/// pixels because that is what the pointer arrives in.
#[derive(Default)]
pub struct Hits {
    rows: Vec<(Rect, usize)>,
    tabs: Vec<(Rect, usize)>,
    /// Header controls, each remembered as the action it fires.
    ///
    /// One list rather than one field per button: a control in the header is
    /// a name and a rectangle, and everything it does is already an `act`
    /// case because the pad has to be able to do it too. Adding a button is
    /// then a call, not a field, a variant and a branch.
    buttons: Vec<(Rect, &'static str)>,
}

impl Hits {
    fn row(&mut self, at: Rect, index: usize) {
        self.rows.push((at, index));
    }

    fn tab(&mut self, at: Rect, index: usize) {
        self.tabs.push((at, index));
    }

    fn button_at(&self, x: f32, y: f32) -> Option<&'static str> {
        self.buttons
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, a)| *a)
    }

    fn at(&self, x: f32, y: f32) -> Option<usize> {
        self.rows
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, i)| *i)
    }

    /// Checked before the rows, because the tab row is drawn over them.
    fn tab_at(&self, x: f32, y: f32) -> Option<usize> {
        self.tabs
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, i)| *i)
    }
}

/// Everything one frame needs to draw itself, so no function takes eight
/// arguments and no call site gets two of them the wrong way round.
struct Frame<'a> {
    gfx: &'a Gfx,
    glass: Option<&'a glass::Glass>,
    painter: &'a mut text::Painter,
    art: &'a mut covers::Covers,
    screen: &'a Viewport,
    /// Which row the pointer is over, if any. Separate from the cursor: see
    /// the motion handler.
    hover: Option<usize>,
    hits: Hits,
    /// What panels are tinted with, from the color scheme. A constant here was
    /// the same gray whichever scheme was chosen.
    glass_tint: Rgba,
    /// Where the cursor is drawn, sliding towards where it actually is.
    glide: &'a mut Gliding,
    /// The loop's clock, for that slide.
    now: f64,
    /// Whether to animate at all — the one switch in Appearance.
    animate: bool,
}

impl Frame<'_> {
    /// Points to pixels. The one conversion in the file.
    fn px(&self, r: Rect) -> Rect {
        let s = self.screen.scale.factor();
        Rect::new(r.x * s, r.y * s, r.w * s, r.h * s)
    }

    fn whole(&self) -> (f32, f32) {
        (self.screen.width_px, self.screen.height_px)
    }

    /// A pane of frosted glass, or a flat one where the machine has no glass.
    fn pane(&self, at: Rect, tint: Rgba, round: f32) {
        let at = self.px(at);
        let round = round * self.screen.scale.factor();
        let whole = self.whole();
        // Lit from above, the way a sheet of glass is: the tint is stronger at
        // the bottom than the top. A flat tint over a blur reads as a gray slab,
        // which is what these were.
        let bottom = Rgba(tint.0, tint.1, tint.2, (tint.3 * 1.45).min(1.0));
        self.gfx.faded(bottom, || {
            self.gfx.rounded(round, || match self.glass {
                Some(glass) => glass.panel(self.gfx, whole, at.x, at.y, at.w, at.h, tint),
                None => self.gfx.fill(at, tint),
            })
        });
    }

    fn fill(&self, at: Rect, color: Rgba, round: f32) {
        let at = self.px(at);
        let gfx = self.gfx;
        gfx.rounded(round * self.screen.scale.factor(), || gfx.fill(at, color));
    }

    fn outline(&self, at: Rect, thickness: f32, color: Rgba, round: f32) {
        let at = self.px(at);
        let s = self.screen.scale.factor();
        let gfx = self.gfx;
        gfx.rounded(round * s, || gfx.outline(at, thickness * s, color));
    }

    fn spec(&self, text: impl Into<String>, size: f32) -> text::Spec {
        text::Spec::new(text, size, self.screen.scale.factor())
    }

    fn wrapped(&self, text: impl Into<String>, size: f32, width: f32, lines: u16) -> text::Spec {
        self.spec(text, size).wrapped(width, lines)
    }

    /// A label at the top left of a box. Returns its height, in points.
    fn label(&mut self, spec: &text::Spec, at: Rect, color: Rgba) -> f32 {
        let at = self.px(at);
        let h = self.painter.put(self.gfx, spec, at, color);
        self.screen.scale.pt(h)
    }

    fn label_right(&mut self, spec: &text::Spec, at: Rect, color: Rgba) {
        let at = self.px(at);
        self.painter.put_right(self.gfx, spec, at, color);
    }

    fn label_centered(&mut self, spec: &text::Spec, at: Rect, color: Rgba) {
        let at = self.px(at);
        self.painter.put_centered(self.gfx, spec, at, color);
    }

    /// The pointer's mark on a row, drawn under everything else in it.
    fn hovering(&self, index: usize, at: Rect, round: f32) {
        if self.hover == Some(index) {
            self.fill(at, paint::HOVER, round);
        }
    }

    /// A header control: a pill with a word in it, and a rectangle the mouse
    /// can find it by.
    ///
    /// The pad reaches these through their bindings, so the button carries the
    /// action name rather than a closure — one string is the label, the hit
    /// and the behavior.
    ///
    /// A console's own picture, in a box.
    /// Says whether there was one, so a caller can give the space to something
    /// else rather than reserving it for a picture that does not exist.
    fn console_art(&mut self, slug: &str, at: Rect, round: f32) -> bool {
        let at = self.px(at);
        let round = round * self.screen.scale.factor();
        let gfx = self.gfx;
        match self.art.console(gfx, slug) {
            Some(picture) => {
                gfx.rounded(round, || gfx.picture(picture, at, Rgba::WHITE));
                true
            }
            None => false,
        }
    }

    /// The signal symbol, made once per strength and held.
    ///
    /// Kept in the cover cache under a key no rom and no console can have.
    /// There are five of them and they are on screen constantly, so making the
    /// pixels every frame would be the one thing in the corner that costs
    /// anything.
    fn signal(&mut self, bars: u8, at: Rect) {
        let key = -(2_000_000 + bars as i64);
        let at = self.px(at);
        let gfx = self.gfx;
        let texture = self.art.made(gfx, key, || {
            let (w, h) = status::WIFI_SIZE;
            (w, h, status::wifi_pixels(bars))
        });
        if let Some(texture) = texture {
            gfx.picture(texture, at, paint::DIM);
        }
    }

    /// The battery symbol, made once per state and held.
    ///
    /// Keyed by charge and by whether it is plugged in, so a hundred and one
    /// possible pictures — which sounds like a lot and is 22x12 pixels each,
    /// made only as they are reached, and the charge moves in whole percent.
    fn charge(&mut self, percent: u8, charging: bool, at: Rect) {
        let key = -(3_000_000 + percent as i64 * 2 + charging as i64);
        let at = self.px(at);
        let gfx = self.gfx;
        let texture = self.art.made(gfx, key, || {
            let (w, h) = status::BATTERY_SIZE;
            (w, h, status::battery_pixels(percent, charging))
        });
        if let Some(texture) = texture {
            // Green when plugged in, and the usual grey otherwise — the one
            // fact worth reading across the room.
            let tint = if charging { paint::HERE } else { paint::DIM };
            gfx.picture(texture, at, tint);
        }
    }

    /// A game's cover, in a box. Says whether there was one, so the caller can
    /// put a pane of glass there instead.
    /// A picture named by path — a port's artwork, which the gamelist points
    /// straight at rather than filing under a platform and a stem.
    fn picture(&mut self, key: i64, path: &std::path::Path, at: Rect, round: f32) -> bool {
        let at = self.px(at);
        let round = round * self.screen.scale.factor();
        let gfx = self.gfx;
        match self.art.by_path(gfx, key, path) {
            Some(art) => {
                gfx.rounded(round, || gfx.picture(art, at, Rgba::WHITE));
                true
            }
            None => false,
        }
    }

    fn cover(&mut self, id: i64, platform: &str, stem: &str, at: Rect, round: f32) -> bool {
        let at = self.px(at);
        let round = round * self.screen.scale.factor();
        let gfx = self.gfx;
        match self.art.get(gfx, id, platform, stem) {
            Some(cover) => {
                gfx.rounded(round, || gfx.picture(cover, at, Rgba::WHITE));
                true
            }
            None => false,
        }
    }
}

/// One frame.
///
/// Fourteen arguments, and clippy is right that it is too many — but the fix is
/// a struct holding every borrow the drawing needs, which is a refactor to make
/// deliberately rather than on the way to a release.
#[allow(clippy::too_many_arguments)]
///
/// The page divides itself: a tab row, a header, and a body split into as many
/// columns as the window can hold. Nothing below adds a gap to an offset.
fn draw(
    gfx: &Gfx,
    frosted: Option<&glass::Glass>,
    painter: &mut text::Painter,
    art: &mut covers::Covers,
    lib: &mut library::Library,
    screen: &Viewport,
    hover: Option<usize>,
    typing: Option<&keyboard::Keyboard>,
    status: &status::Status,
    glass_tint: Rgba,
    scanning: &str,
    glide: &mut Gliding,
    now: f64,
    animate: bool,
) -> Hits {
    let mut f = Frame {
        gfx,
        glass: frosted,
        painter,
        art,
        screen,
        hover,
        hits: Hits::default(),
        glass_tint,
        glide,
        now,
        animate,
    };
    let page = Rect::new(0.0, 0.0, screen.width(), screen.height());

    let boxes = page.column(
        0.0,
        &[
            Size::Fixed(size::TABS),
            // A gap where the "where you are" line used to be. `size::HEADER`
            // is what to put back if it is ever wanted again — see
            // `draw_chrome`, where the line itself is commented out.
            Size::Fixed(size::HEADER_GAP),
            Size::Grow(1.0),
            Size::Fixed(size::HELP),
        ],
    );
    let [tabs, header, body, help] =
        <[Rect; 4]>::try_from(boxes).unwrap_or([Rect::new(0.0, 0.0, 0.0, 0.0); 4]);

    // One page at a time, chosen by the tab. There is no Sofa-or-Desk here and
    // no column count to work out: the panel is 640 points wide and that is
    // room for one list and one pane beside it, so the arrangement is not a
    // question the window gets to answer differently on different days.
    match library::SECTIONS[lib.section.min(library::SECTIONS.len() - 1)].id {
        // A scan in progress owns the Library page: there is nothing on it yet
        // and a blank screen is the one thing that cannot say why.
        "library" if !scanning.is_empty() => {
            let spec = f.wrapped(scanning, size::TITLE, body.w * 0.7, 3);
            f.label_centered(&spec, body, paint::DIM);
        }
        "library" => draw_library(&mut f, lib, body.inset(size::PAD)),
        "mine" => draw_collections(&mut f, lib, body.inset(size::PAD)),
        "history" => draw_history(&mut f, lib, body.inset(size::PAD)),
        "syncing" => draw_syncing(&mut f, lib, body.inset(size::PAD)),
        "settings" => draw_settings(&mut f, lib, body.inset(size::PAD)),
        // Until Settings is built, its tab is where the keyboard can be looked
        // at — it is the only screen that will ask for one.
        other => draw_unbuilt(&mut f, other, body),
    }

    // What the buttons do here. Said rather than left to be found by pressing
    // everything, which is what a settings screen with no help bar teaches.
    draw_help(&mut f, hints_for(lib, typing.is_some()), help);

    // The keyboard sits over whatever asked for it.
    if let Some(picker) = lib.picking.as_ref() {
        let shots = lib.previews.as_ref().map(|p| p.found.clone()).unwrap_or_default();
        draw_picker_sheet(&mut f, picker, &shots, page);
    } else if let Some(cap) = lib.capturing.as_ref() {
        draw_capture(&mut f, cap, page);
    }
    if let Some(kb) = typing {
        draw_keyboard(&mut f, kb, page);
    }

    draw_chrome(&mut f, lib, tabs, header, status);
    f.hits
}

/// The Library tab: consoles, then one console's games with a pane beside them.
///
/// Two screens rather than two columns. 640 points will not hold a console
/// picker *and* a game list *and* an info pane, so the picker is the whole page
/// until a console is opened and then gets out of the way — which is also how
/// the handheld wants to be read, one thing at a time.
fn draw_library(f: &mut Frame, lib: &mut library::Library, area: Rect) {
    if lib.consoles.is_empty() {
        let spec = f.wrapped(
            "No consoles in this library.\nRun `moose-rack sync` to fill it.",
            size::TITLE,
            area.w * 0.8,
            2,
        );
        f.label_centered(&spec, area, paint::DIM);
        return;
    }

    if lib.view == library::View::Scripts {
        // A port is a name and, if it was scraped, a picture. No size, no
        // console, no star — none of those mean anything for a shell script.
        let [list, aside] = <[Rect; 2]>::try_from(
            area.cols(size::GAP, &[8, 4]),
        )
        .unwrap();
        let rows: Vec<_> = lib
            .ports
            .iter()
            .map(|p| (p.name.clone(), String::new()))
            .collect();
        draw_picker(f, list, &rows, lib.port_at, rows.len());

        f.pane(aside, f.glass_tint, size::ROUND);
        if let Some(port) = lib.ports.get(lib.port_at) {
            let inner = aside.inset(size::PAD);
            let (art, below) = inner.split_top(inner.w * 0.62);
            // Negative keys: the cover cache is keyed by ROM id and a port has
            // none, so these take slots no ROM can reach.
            let key = -(lib.port_at as i64 + 1);
            let drawn = port
                .image
                .as_deref()
                .is_some_and(|p| f.picture(key, p, art, size::ROUND_SMALL));
            if !drawn {
                f.pane(art, paint::CARD, size::ROUND_SMALL);
            }
            let title = f.wrapped(&port.name, size::TITLE, below.w, 3);
            f.label(
                &title,
                below.inset(Edges {
                    top: 8.0,
                    ..Edges::default()
                }),
                paint::TEXT,
            );
        }
        return;
    }
    if lib.view != library::View::Roms {
        draw_consoles(f, lib, area, true);
        return;
    }

    // The games, and the facts about the one under the cursor. The list is
    // always a list here — a wall of covers and a sidebar do not both fit, and
    // between the two the list is what answers "what is in this console".
    // Half and half.
    //
    // The list gave two thirds to names and a column of file sizes, and the
    // pane got what was left. A size is not why anybody is looking at a game
    // list — it is in the pane, where the questions about one game live — and
    // the room it took is better spent on the pane being readable.
    let [list, aside] = <[Rect; 2]>::try_from(area.cols(size::GAP, &[6, 6])).unwrap();
    draw_game_list(f, lib, list);
    f.pane(aside, f.glass_tint, size::ROUND);
    draw_detail(f, lib, aside.inset(size::PAD));
}

/// The Collections tab: kinds, then collections, then games.
///
/// Three levels because the data is three levels and the middle one is not
/// optional — there are 1,024 company collections in this library, which is not
/// a list anybody scrolls to the bottom of.
fn draw_collections(f: &mut Frame, lib: &mut library::Library, area: Rect) {
    match lib.view {
        library::View::Groups => {
            let rows: Vec<MenuRow> = lib
                .shelves
                .iter()
                .map(|sh| MenuRow {
                    title: sh.name().to_owned(),
                    count: sh.count().to_string(),
                    note: sh.note(),
                    inside: Vec::new(),
                })
                .collect();
            // The rule sits between yours and RomM's, and only if there is
            // something on both sides of it.
            let rule = (lib.mine_count > 0 && lib.mine_count < rows.len())
                .then_some(lib.mine_count);
            // Only the highlighted one is looked into: it is the only row the
            // side panel has room to describe.
            let id = match lib.shelves.get(lib.shelf_at) {
                Some(library::Shelf::Mine { id, .. }) => Some(id.clone()),
                _ => None,
            };
            let mut rows = rows;
            if let (Some(id), Some(row)) = (id, rows.get_mut(lib.shelf_at)) {
                row.inside = lib.peek(&id).to_vec();
            }
            draw_menu(
                f,
                area,
                &rows,
                lib.shelf_at,
                rule,
                "No collections yet.\nThey come from the RomM server; a library scanned from the card has none.",
            );
        }
        library::View::Collections => {
            let rows: Vec<MenuRow> = lib
                .cols
                .iter()
                .map(|c| MenuRow {
                    title: c.name.clone(),
                    count: c.rom_count.to_string(),
                    note: match c.rom_count {
                        1 => "One game.".to_owned(),
                        n => format!("{n} games."),
                    },
                    inside: Vec::new(),
                })
                .collect();
            let id = lib.cols.get(lib.col_at).map(|c| c.id.clone());
            let mut rows = rows;
            if let (Some(id), Some(row)) = (id, rows.get_mut(lib.col_at)) {
                row.inside = lib.peek(&id).to_vec();
            }
            draw_menu(f, area, &rows, lib.col_at, None, "Nothing in this group.");
        }
        // A collection of games is a list of games: same list, same info pane.
        library::View::Roms => {
            let [list, aside] = <[Rect; 2]>::try_from(
                area.cols(size::GAP, &[8, 4]),
            )
            .unwrap();
            draw_game_list(f, lib, list);
            f.pane(aside, f.glass_tint, size::ROUND);
            draw_detail(f, lib, aside.inset(size::PAD));
        }
        // Not reachable: `go_to_section` puts this tab at `Groups` and only
        // `back`/`activate` move it. Drawing the root beats drawing nothing.
        _ => draw_picker(f, area, &[], 0, 0),
    }
}

/// The History tab: what was actually played, most time first.
///
/// Not the same list as Continue. Continue answers "what was I doing"; this
/// answers "what have I played", which wants time and counts rather than
/// artwork, and reads as a table.
fn draw_history(f: &mut Frame, lib: &mut library::Library, area: Rect) {
    if lib.history.is_empty() {
        let spec = f.wrapped(
            "Nothing played yet.\nPlay time arrives with a sync.",
            size::TITLE,
            area.w * 0.8,
            2,
        );
        f.label_centered(&spec, area, paint::DIM);
        return;
    }
    let [list, aside] =
        <[Rect; 2]>::try_from(area.cols(size::GAP, &[8, 4]))
            .unwrap();

    let step = size::ROW;
    lib.relayout(1);
    let fits = (list.h / step).floor().max(1.0) as usize;
    let first = lib.history_at.saturating_sub(fits.saturating_sub(1));

    for (offset, game) in lib.history.iter().enumerate().skip(first).take(fits) {
        let line = Rect::new(
            list.x,
            list.y + (offset - first) as f32 * step,
            list.w,
            step,
        );
        if line.bottom() > list.bottom() {
            break;
        }
        f.hits.row(f.px(line), offset);
        let on = offset == lib.history_at;
        f.hovering(offset, line, size::ROUND_SMALL);
        if on {
            f.pane(line, paint::CURSOR, size::ROUND_SMALL);
        }

        // Name and time played. The date moved to the pane: it is the least
        // useful of the three at a glance and was taking a column from the
        // titles, which are what you are reading down.
        let inside = line.inset(Edges::xy(8.0, 5.0));
        let [title, time] =
            <[Rect; 2]>::try_from(inside.row(8.0, &[Size::Grow(1.0), Size::Fixed(56.0)])).unwrap();
        let spec = f.wrapped(&game.name, size::LABEL, title.w, 1);
        f.label(&spec, title, if on { paint::TEXT } else { paint::DIM });
        let spec = f.spec(played_for(game.seconds), 10.0);
        f.label_right(&spec, time, if on { paint::TEXT } else { paint::DIM });
    }

    // What the highlighted game is, in full.
    f.pane(aside, f.glass_tint, size::ROUND);
    if let Some(game) = lib.history.get(lib.history_at) {
        let inner = aside.inset(size::PAD);
        let (art, below) = inner.split_top(inner.w * 0.62);
        let key = -(lib.history_at as i64 + 1_000_001);
        if !f.cover(game.id, &game.platform, &game.stem, art, size::ROUND_SMALL) {
            f.pane(art, paint::CARD, size::ROUND_SMALL);
        }
        let _ = key;
        let title = f.wrapped(&game.name, size::TITLE, below.w, 2);
        let used = f.label(
            &title,
            below.inset(Edges {
                top: 8.0,
                ..Edges::default()
            }),
            paint::TEXT,
        );

        let facts = [
            ("Console", game.platform.clone()),
            ("Played", played_for(game.seconds)),
            (
                "Times",
                if game.runs == 1 {
                    "once".to_owned()
                } else {
                    format!("{}", game.runs)
                },
            ),
            ("Last", game.last.chars().take(10).collect::<String>()),
        ];
        for (i, (label, value)) in facts.iter().enumerate() {
            let line = Rect::new(
                below.x,
                below.y + used + 16.0 + i as f32 * 18.0,
                below.w,
                16.0,
            );
            let [name, val] =
                <[Rect; 2]>::try_from(line.row(6.0, &[Size::Fixed(58.0), Size::Grow(1.0)]))
                    .unwrap();
            let spec = f.spec(*label, size::LABEL);
            f.label(&spec, name, paint::FAINT);
            let spec = f.spec(value.as_str(), size::LABEL);
            f.label(&spec, val, paint::DIM);
        }
    }
}

/// The Syncing tab: the state a sync would change.
///
/// Nothing here performs one — that needs the network on a thread the draw loop
/// does not have yet. What it does is stop the tab being a lie: everything shown
/// is read from the same cache the other front ends sync into, so it says
/// truthfully how far behind this device is.
fn draw_syncing(f: &mut Frame, lib: &library::Library, area: Rect) {
    let s = &lib.sync;
    let (games, sessions, seconds) = (s.games_played, s.sessions, s.seconds_played);
    let facts: [(&str, String); 6] = [
        (
            "Server",
            s.server_version
                .clone()
                .unwrap_or_else(|| "not recorded".into()),
        ),
        (
            "Library synced through",
            s.watermark
                .as_ref()
                .map(|w| w.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "never".into()),
        ),
        ("Games in the cache", s.games.to_string()),
        ("Consoles", s.consoles.to_string()),
        ("Collections", s.collections.to_string()),
        (
            "Play recorded",
            if sessions == 0 {
                "none".to_owned()
            } else {
                format!(
                    "{games} games · {sessions} sessions · {}",
                    played_for(seconds)
                )
            },
        ),
    ];

    let (top, rest) = area.split_top(24.0);
    let spec = f.spec("WHAT THIS DEVICE HOLDS", 10.0);
    f.label(&spec, top, paint::FAINT);

    let table = rest.inset(Edges {
        top: 4.0,
        ..Edges::default()
    });
    for (i, (label, value)) in facts.iter().enumerate() {
        let line = Rect::new(table.x, table.y + i as f32 * 22.0, table.w.min(420.0), 20.0);
        let [name, val] =
            <[Rect; 2]>::try_from(line.row(10.0, &[Size::Fixed(150.0), Size::Grow(1.0)])).unwrap();
        let spec = f.spec(*label, size::LABEL);
        f.label(&spec, name, paint::FAINT);
        let spec = f.spec(value.as_str(), size::LABEL);
        f.label(&spec, val, paint::TEXT);
    }

    let note = Rect::new(
        table.x,
        table.y + facts.len() as f32 * 22.0 + 14.0,
        table.w.min(430.0),
        60.0,
    );
    let spec = f.wrapped(
        "Pulling the library and pushing saves is not wired up here yet. Sync from \
         the desktop app and this page will follow.",
        size::LABEL,
        note.w,
        3,
    );
    f.label(&spec, note, paint::DIM);
}

/// A plain list of name and count — the shape both Collections levels take.
fn draw_picker(f: &mut Frame, area: Rect, rows: &[(String, String)], at: usize, total: usize) {
    draw_rule_list(f, area, rows, at, total, None)
}

/// One row of a root menu: what it is, what is in it, and a line about it.
///
/// The three are separated because they are drawn differently — a name is
/// read, a count is glanced at, and the note is only read once, on the row the
/// cursor is on.
pub struct MenuRow {
    pub title: String,
    pub count: String,
    pub note: String,
    /// What is inside, for the side panel — a few of the names, so the panel
    /// says something about *this* collection rather than about collections.
    pub inside: Vec<String>,
}

/// A root menu: Settings' list of panes, Collections' list of collections.
///
/// These two pages were a name at the left edge and a grey number at the right
/// with two thirds of the screen left over, which is a list rather than a
/// screen. Three things change that, and they are the three every menu on a
/// device like this has:
///
///   * **Rows are cards.** Text laid straight onto the backdrop has nothing to
///     tell the eye where one row stops. A faint plate per row does.
///   * **The count is a chip, not digits at the far edge.** A number 500 points
///     from the name it belongs to is not read as belonging to it.
///   * **The leftover space carries the description.** The note for the row the
///     cursor is on, which is what the empty two thirds was there for.
fn draw_menu(
    f: &mut Frame,
    area: Rect,
    rows: &[MenuRow],
    at: usize,
    rule_before: Option<usize>,
    empty_note: &str,
) {
    if rows.is_empty() {
        // Say why, not just that.
        //
        // A library built by scanning the card has no collections in it and
        // never will: a collection is something the RomM server keeps, and the
        // card holds files. "Nothing here." on a device that has just found
        // 7,891 games reads as a fault.
        let spec = f.wrapped(empty_note, size::TITLE, area.w * 0.7, 3);
        f.label_centered(&spec, area, paint::DIM);
        return;
    }
    // Seven columns and five: wide enough for a long collection name, and a
    // side wide enough for a sentence about it.
    let [list, aside] = <[Rect; 2]>::try_from(area.cols(size::GAP, &[7, 5])).unwrap();

    let step = size::ROW + 4.0;
    let room = list.h - 10.0 - rule_before.map_or(0.0, |_| size::GAP * 1.6);
    let fits = (room / step).floor().max(1.0) as usize;
    let first = at
        .saturating_sub(fits.saturating_sub(1))
        .min(rows.len().saturating_sub(1));
    let shift = |offset: usize| match rule_before {
        Some(r) if offset >= r && r > first => size::GAP * 1.6,
        _ => 0.0,
    };
    // The column is a panel of its own, full height, with the cards inside it.
    //
    // Centering the cards in the empty column was the other way to stop eight
    // rows hanging from the top of a screen with four hundred points of nothing
    // under them, and it reads worse: a floating stack beside a full-height
    // side panel has nothing holding it. A container has the two columns agree.
    f.pane(list, paint::BAR, size::ROUND);
    let list = list.inset(Edges::all(5.0));
    let top = list.y;

    for (offset, row) in rows.iter().enumerate().skip(first).take(fits) {
        let slot = Rect::new(
            list.x,
            top + (offset - first) as f32 * step + shift(offset),
            list.w,
            step,
        );
        if slot.bottom() > list.bottom() {
            break;
        }
        if rule_before == Some(offset) && offset > first {
            f.fill(
                Rect::new(list.x, slot.y - size::GAP * 0.8, list.w, 1.0),
                paint::RULE,
                0.0,
            );
        }
        // The card, with the gap between cards taken out of the slot rather
        // than added to it — so the rows keep their spacing and the list keeps
        // its count.
        let card = Rect::new(slot.x, slot.y + 2.0, slot.w, slot.h - 4.0);
        f.hits.row(f.px(card), offset);
        let on = offset == at;
        f.hovering(offset, card, size::ROUND_SMALL);
        f.pane(
            card,
            if on { paint::CURSOR } else { paint::CARD },
            size::ROUND_SMALL,
        );

        let inside = card.inset(Edges::xy(10.0, 4.0));
        let [name, chip] = <[Rect; 2]>::try_from(inside.cols(6.0, &[9, 3])).unwrap();
        let spec = f.wrapped(&row.title, size::LABEL, name.w, 1);
        f.label(&spec, name, if on { paint::TEXT } else { paint::DIM });
        if !row.count.is_empty() {
            let spec = f.spec(row.count.as_str(), 10.0);
            let (text_w, _) = f.painter.measure(f.gfx, &spec);
            let w = (f.screen.scale.pt(text_w as f32) + 12.0).max(24.0);
            let plate = Rect::new(
                chip.right() - w,
                chip.y + (chip.h - 15.0) / 2.0,
                w,
                15.0,
            );
            f.fill(plate, if on { paint::HOVER } else { paint::BAR }, 7.0);
            f.label_centered(&spec, plate, if on { paint::TEXT } else { paint::DIM });
        }
    }

    // The side: what the highlighted row is.
    f.pane(aside, f.glass_tint, size::ROUND);
    let Some(row) = rows.get(at) else { return };
    let inner = aside.inset(Edges::all(10.0));
    let spec = f.wrapped(&row.title, size::TITLE, inner.w, 2);
    let (_, title_h) = f.painter.measure(f.gfx, &spec);
    f.label(&spec, inner, paint::TEXT);
    let after = inner.y + f.screen.scale.pt(title_h as f32) + 8.0;
    let body = Rect::new(inner.x, after, inner.w, inner.bottom() - after);
    let mut y = body.y;
    if !row.note.is_empty() {
        let spec = f.wrapped(&row.note, size::LABEL, body.w, 6);
        let (_, h) = f.painter.measure(f.gfx, &spec);
        f.label(&spec, Rect { y, ..body }, paint::DIM);
        y += f.screen.scale.pt(h as f32) + 10.0;
    }
    for name in &row.inside {
        let line = Rect::new(body.x, y, body.w, 15.0);
        if line.bottom() > body.bottom() {
            break;
        }
        // A dot, then the name — so a list of six titles reads as a list and
        // not as a paragraph that has lost its punctuation.
        f.fill(Rect::new(line.x + 1.0, line.y + 6.0, 3.0, 3.0), paint::FAINT, 1.5);
        let spec = f.wrapped(name, 10.0, line.w - 10.0, 1);
        f.label(&spec, Rect { x: line.x + 10.0, ..line }, paint::FAINT);
        y += 15.0;
    }
}

fn draw_rule_list(
    f: &mut Frame,
    area: Rect,
    rows: &[(String, String)],
    at: usize,
    total: usize,
    rule_before: Option<usize>,
) {
    if rows.is_empty() {
        let spec = f.spec("Nothing here.", size::TITLE);
        f.label_centered(&spec, area, paint::DIM);
        return;
    }
    let step = size::ROW;
    // The rule's own space comes off the budget before the rows are counted.
    // Left out, the last row is pushed past the bottom and dropped — and when
    // that row is the cursor, the screen has no cursor on it at all.
    let room = area.h - rule_before.map_or(0.0, |_| size::GAP * 1.6);
    let fits = (room / step).floor().max(1.0) as usize;
    let first = at
        .saturating_sub(fits.saturating_sub(1))
        .min(total.saturating_sub(1));
    // The rule occupies space of its own, so everything below it shifts down
    // rather than the rule being drawn over the row that follows.
    let shift = |offset: usize| match rule_before {
        Some(r) if offset >= r && r > first => size::GAP * 1.6,
        _ => 0.0,
    };

    for (offset, (name, count)) in rows.iter().enumerate().skip(first).take(fits) {
        let line = Rect::new(
            area.x,
            area.y + (offset - first) as f32 * step + shift(offset),
            area.w,
            step,
        );
        if line.bottom() > area.bottom() {
            break;
        }
        if rule_before == Some(offset) && offset > first {
            f.fill(
                Rect::new(area.x, line.y - size::GAP * 0.8, area.w, 1.0),
                paint::RULE,
                0.0,
            );
        }
        f.hits.row(f.px(line), offset);
        let on = offset == at;
        f.hovering(offset, line, size::ROUND_SMALL);
        if on {
            f.pane(line, paint::CURSOR, size::ROUND_SMALL);
        }
        let inside = line.inset(Edges::xy(8.0, 5.0));
        let [title, n] = <[Rect; 2]>::try_from(inside.cols(8.0, &[9, 3])).unwrap();
        let spec = f.wrapped(name, size::LABEL, title.w, 1);
        f.label(&spec, title, if on { paint::TEXT } else { paint::DIM });
        let spec = f.spec(count.as_str(), 10.0);
        f.label_right(&spec, n, paint::FAINT);
    }
}

/// Seconds as something short enough for a column.
fn played_for(seconds: i64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

/// A keyboard with something in it, for judging the layout.
///
/// `MOOSE_SDL_KEYBOARD="RetroAchievements token"` or `=secret:Wi-Fi password`.
/// Only a preview: the real one is opened by a settings field.
fn demo_keyboard() -> Option<keyboard::Keyboard> {
    let want = std::env::var("MOOSE_SDL_KEYBOARD").ok()?;
    let (secret, prompt) = match want.strip_prefix("secret:") {
        Some(rest) => (true, rest.to_owned()),
        None => (false, want),
    };
    let mut kb = keyboard::Keyboard::new(prompt, if secret { "hunter2" } else { "frank" }, secret);
    kb.row = 1;
    kb.col = 3;
    Some(kb)
}

/// What to say the buttons do, for whatever is on screen.
fn hints_for(lib: &library::Library, typing: bool) -> &'static [(&'static str, &'static str)] {
    if typing {
        return &[("A", "type"), ("B", "delete"), ("L R", "page")];
    }
    if lib.capturing.is_some() {
        return &[("any", "bind"), ("B", "clear"), ("Start", "cancel")];
    }
    if lib.picking.is_some() {
        return &[("A", "choose"), ("B", "cancel")];
    }
    match lib.view {
        library::View::Options => &[
            ("A", "change"),
            ("< >", "slider"),
            ("B", "back"),
            ("L R", "tabs"),
        ],
        library::View::Roms => &[
            ("A", "play"),
            ("B", "back"),
            ("/", "search"),
            ("L R", "tabs"),
        ],
        _ => &[("A", "open"), ("B", "back"), ("L R", "tabs")],
    }
}

/// A sheet of options, over the settings screen.
///
/// The shape a mature settings menu uses: the choices are on screen at once and
/// the current one is marked, so choosing is reading and pressing rather than
/// holding a direction until the right value goes past.
fn draw_picker_sheet(f: &mut Frame, picker: &library::Picker, previews: &[PathBuf], page: Rect) {
    f.fill(page, paint::SCRIM, 0.0);

    let step = size::ROW;
    // As many as the screen has room for, rather than a number picked in
    // advance. Nine was fine for the schemes and hid two of the eleven
    // backdrops — and a list you can only see the top of reads as a list that
    // is missing things.
    let head = 46.0 + size::GAP * 2.0;
    let fits = ((page.h - size::TABS - size::HEADER_GAP - size::HELP - head) / step)
        .floor()
        .max(3.0) as usize;
    let rows = picker.options.len().min(fits);
    let sheet_h = rows as f32 * step + head;
    let sheet = Rect::new(page.x, page.bottom() - sheet_h, page.w, sheet_h);
    f.fill(sheet, paint::SHEET, 0.0);

    let inner = sheet.inset(size::PAD);
    let (head_rect, body) = inner.split_top(24.0);
    let spec = f.spec(picker.title, size::TITLE);
    f.label(&spec, head_rect, paint::TEXT);

    // Choosing a set of pictures by reading nine names is not choosing. The
    // desktop shows each one drawn in its own artwork; so does this, for
    // whichever is under the cursor.
    let showing_art = picker.field == library::FETCH_ICONS;
    let (list, samples) = if showing_art {
        let [list, samples] = <[Rect; 2]>::try_from(body.cols(size::GAP, &[5, 7])).unwrap();
        (list, Some(samples))
    } else {
        (body, None)
    };

    // Keep the highlighted option on screen when the list is longer than the
    // sheet — the core map offers nine cores for some consoles.
    let first = picker.at.saturating_sub(rows.saturating_sub(1));
    if picker.options.len() > rows {
        let spec = f.spec(
            format!("{} of {}", picker.at + 1, picker.options.len()),
            10.0,
        );
        f.label_right(&spec, head_rect, paint::FAINT);
    }
    for (i, (_, label)) in picker.options.iter().enumerate().skip(first).take(rows) {
        let line = Rect::new(list.x, list.y + (i - first) as f32 * step, list.w, step);
        let on = i == picker.at;
        if on {
            f.pane(line, paint::CURSOR, size::ROUND_SMALL);
        }
        let inside = line.inset(Edges::xy(8.0, 5.0));
        let spec = f.wrapped(label, size::LABEL, inside.w, 1);
        f.label(&spec, inside, if on { paint::TEXT } else { paint::DIM });
    }

    // The samples, in two rows of three.
    let Some(samples) = samples else { return };
    f.pane(samples, paint::CARD, size::ROUND_SMALL);
    let grid = samples.inset(Edges::all(6.0));
    if previews.is_empty() {
        let spec = f.spec("Fetching a look at it…", size::LABEL);
        f.label_centered(&spec, grid, paint::FAINT);
        return;
    }
    let (across, down) = (3usize, 2usize);
    let cell_w = (grid.w - size::GAP * (across - 1) as f32) / across as f32;
    let cell_h = (grid.h - size::GAP * (down - 1) as f32) / down as f32;
    for (i, path) in previews.iter().take(across * down).enumerate() {
        let cell = Rect::new(
            grid.x + (i % across) as f32 * (cell_w + size::GAP),
            grid.y + (i / across) as f32 * (cell_h + size::GAP),
            cell_w,
            cell_h,
        );
        // A key no ROM can have and no console picture uses, so a preview
        // never collides with real artwork in the cache.
        let key = -(1_000_000 + i as i64);
        f.picture(key, path, cell, size::ROUND_SMALL);
    }
}

/// Waiting for a button, so a control can be put on it.
fn draw_capture(f: &mut Frame, cap: &library::Capture, page: Rect) {
    f.fill(page, paint::SCRIM, 0.0);
    let (top, rest) = page.inset(size::PAD).split_top(page.h * 0.42);
    let spec = f.spec(cap.label, 20.0);
    f.label_centered(&spec, top, paint::TEXT);
    let spec = f.wrapped(
        "Press the button you want. B clears it, Start leaves it alone.",
        size::TITLE,
        rest.w.min(380.0),
        3,
    );
    f.label_centered(&spec, Rect { h: 60.0, ..rest }, paint::DIM);
}

/// What the buttons do, along the bottom.
///
/// Permanent, the way a settings menu that expects a pad has it — a screen
/// whose controls you have to guess is one where left and right get pressed on
/// everything to find out what moves.
fn draw_help(f: &mut Frame, hints: &[(&str, &str)], area: Rect) {
    let mut x = area.x + size::GAP;
    for (button, what) in hints {
        let spec = f.spec(*button, 10.0);
        let (w, _) = f.painter.measure(f.gfx, &spec);
        let w = f.screen.scale.pt(w as f32);
        f.label(&spec, Rect { x, w, ..area }, paint::TEXT);
        x += w + 4.0;

        let spec = f.spec(*what, 10.0);
        let (w, _) = f.painter.measure(f.gfx, &spec);
        let w = f.screen.scale.pt(w as f32);
        f.label(&spec, Rect { x, w, ..area }, paint::FAINT);
        x += w + size::GAP * 1.6;
    }
}

/// The on-screen keyboard, over whatever asked for it.
///
/// Drawn as a sheet across the bottom two thirds rather than a dialog in the
/// middle: the field being filled has to stay visible while you type into it,
/// and on a 480-point screen a centered box leaves room for neither.
fn draw_keyboard(f: &mut Frame, kb: &keyboard::Keyboard, page: Rect) {
    // Block what is behind it. A keyboard you can read the library through is a
    // keyboard you keep losing the cursor on, and the page underneath is not
    // something you can act on while a field is open anyway.
    f.fill(page, paint::SCRIM, 0.0);

    // Laptop proportions: the key area is about two and a half times as wide as
    // it is tall, so keys come out wider than they are deep. Filling the height
    // instead gave 85-point rows — keys the size of postage stamps stacked into
    // a wall, which is nothing like a keyboard and read as one.
    let key_h = 34.0;
    let rows = keyboard::ROWS + 1;
    let keys_h = rows as f32 * key_h + size::GAP * (rows - 1) as f32;
    let sheet_h = keys_h + 52.0 + size::GAP * 2.0;
    let sheet = Rect::new(page.x, page.bottom() - sheet_h, page.w, sheet_h);
    f.fill(sheet, paint::SHEET, 0.0);

    let inner = sheet.inset(size::PAD);
    let (top, keys) = inner.split_top(52.0);

    // What is being asked for, and what has been typed so far.
    let prompt = f.spec(kb.prompt.as_str(), size::LABEL);
    f.label(&prompt, Rect { h: 15.0, ..top }, paint::FAINT);
    let field = Rect {
        y: top.y + 17.0,
        h: 26.0,
        ..top
    };
    f.pane(field, paint::CARD, size::ROUND_SMALL);
    let shown = kb.shown();
    let text = f.spec(
        if shown.is_empty() {
            "…"
        } else {
            shown.as_str()
        },
        size::TITLE,
    );
    f.label(
        &text,
        field.inset(Edges::xy(8.0, 5.0)),
        if shown.is_empty() {
            paint::FAINT
        } else {
            paint::TEXT
        },
    );

    let grid = kb.grid();
    let cell_w = (keys.w - size::GAP * (keyboard::COLS - 1) as f32) / keyboard::COLS as f32;
    let row_y = |row: usize| keys.y + row as f32 * (key_h + size::GAP);

    for (row, letters) in grid.iter().enumerate().take(keyboard::ROWS) {
        for col in 0..keyboard::COLS {
            let cell = Rect::new(
                keys.x + col as f32 * (cell_w + size::GAP),
                row_y(row),
                cell_w,
                key_h,
            );
            let on = !kb.on_actions() && kb.row == row && kb.col == col;
            f.pane(
                cell,
                if on { paint::CURSOR } else { paint::CARD },
                size::ROUND_SMALL,
            );
            let ch: String = letters.chars().nth(col).into_iter().collect();
            let spec = f.spec(ch.as_str(), size::TITLE);
            f.label_centered(&spec, cell, if on { paint::TEXT } else { paint::DIM });
        }
    }

    // The action row. Space is drawn wide because that is what a thumb expects,
    // and stays one cell to the cursor.
    let weights = [1.4, 1.2, 3.4, 1.4, 1.6];
    let total: f32 = weights.iter().sum();
    let usable = keys.w - size::GAP * (weights.len() - 1) as f32;
    let mut x = keys.x;
    for (i, action) in keyboard::ACTIONS.iter().enumerate() {
        let w = usable * weights[i] / total;
        let cell = Rect::new(x, row_y(keyboard::ROWS), w, key_h);
        let on = kb.action() == Some(*action);
        f.pane(
            cell,
            if on { paint::CURSOR } else { paint::CARD },
            size::ROUND_SMALL,
        );
        let spec = f.spec(action.label(), size::LABEL);
        f.label_centered(&spec, cell, if on { paint::TEXT } else { paint::DIM });
        x += w + size::GAP;
    }
}

/// The Settings tab: the panes, then one pane's settings.
///
/// Two levels, like Collections. The settings themselves are a list with the
/// value on the right and one sentence of explanation beside it — because a
/// setting whose name you have to guess the meaning of is a setting nobody
/// touches, and on a handheld there is no tooltip to hover for.
fn draw_settings(f: &mut Frame, lib: &mut library::Library, area: Rect) {
    if lib.view == library::View::Wifi {
        let (top, list) = area.split_top(22.0);
        let spec = f.spec(lib.wifi.note().as_str(), size::LABEL);
        f.label(&spec, top, paint::FAINT);
        let names = lib.wifi.names();
        let joined = match &lib.wifi {
            wifi::State::Networks { joined, .. } => joined.clone(),
            _ => None,
        };
        let rows: Vec<_> = names
            .iter()
            .map(|n| {
                let mark = if joined.as_deref() == Some(n.as_str()) {
                    "saved"
                } else {
                    ""
                };
                (n.clone(), mark.to_owned())
            })
            .collect();
        draw_picker(f, list, &rows, lib.wifi_at, rows.len());
        return;
    }

    if lib.view == library::View::Panes {
        let rows: Vec<MenuRow> = lib
            .panes
            .iter()
            .map(|p| MenuRow {
                title: p.label.to_owned(),
                count: p.entries.len().to_string(),
                note: p.blurb.to_owned(),
                inside: p.entries.iter().take(6).map(|e| e.label.to_owned()).collect(),
            })
            .collect();
        draw_menu(f, area, &rows, lib.pane_at, None, "No settings.");
        return;
    }

    // A download in flight reports on its own row, and when it lands the panes
    // are rebuilt — the set that was just fetched is a real choice now, and the
    // list of installed sets is read when the panes are built.
    if let Some(fetch) = lib.fetching.as_mut() {
        let note = {
            fetch.poll();
            // Named, because the row says "Get pictures" and what is arriving
            // is one particular set out of nine.
            format!("{} · {}", fetch.set.trim_end_matches("-es-de"), fetch.note())
        };
        let landed = fetch.finished();
        if let Some(pane) = lib.panes.get_mut(lib.pane_at)
            && let Some(entry) = pane.entries.iter_mut().find(|e| e.label == "Get pictures")
        {
            entry.kind = settings::Kind::ReadOnly(note);
        }
        if landed {
            lib.fetching = None;
            lib.rebuild_panes();
        }
    }

    let Some(pane) = lib.panes.get(lib.pane_at) else {
        return;
    };
    // About has nothing to explain. Every row on it is a fact with no help to
    // give and no value to change, so a column repeating the row's own label
    // is furniture — the page takes the whole width instead.
    let plain = pane.id == "about";
    let (list, aside) = if plain {
        (area, Rect::new(0.0, 0.0, 0.0, 0.0))
    } else {
        let [list, aside] = <[Rect; 2]>::try_from(area.cols(size::GAP, &[8, 4])).unwrap();
        (list, aside)
    };

    // The column is a panel, like the root menus.
    //
    // Not decoration: at the resolution the device actually has, a moving
    // backdrop passes straight through unhighlighted rows and the value at the
    // end of a line stops being readable. A settings screen is the one place
    // where every row has to be legible at once.
    f.pane(list, paint::BAR, size::ROUND);
    let list = list.inset(Edges::all(5.0));

    let step = size::ROW;
    let fits = (list.h / step).floor().max(1.0) as usize;
    let first = lib.option_at.saturating_sub(fits.saturating_sub(1));

    for (offset, entry) in pane.entries.iter().enumerate().skip(first).take(fits) {
        let line = Rect::new(
            list.x,
            list.y + (offset - first) as f32 * step,
            list.w,
            step,
        );
        if line.bottom() > list.bottom() {
            break;
        }
        f.hits.row(f.px(line), offset);
        let on = offset == lib.option_at;
        f.hovering(offset, line, size::ROUND_SMALL);
        if on {
            f.pane(line, paint::CURSOR, size::ROUND_SMALL);
        }
        let inside = line.inset(Edges::xy(8.0, 5.0));
        let [name, value] =
            <[Rect; 2]>::try_from(inside.row(8.0, &[Size::Grow(1.0), Size::Fixed(110.0)])).unwrap();
        let spec = f.wrapped(entry.label, size::LABEL, name.w, 1);
        f.label(&spec, name, if on { paint::TEXT } else { paint::DIM });

        // Only a slider shows arrows, because only a slider is changed with
        // them. A toggle flips on A and a choice opens a list, and drawing
        // arrows on those is how the screen taught the wrong control.
        let sliding = entry.steps();
        let shown = if on && sliding {
            format!("\u{2039} {} \u{203a}", entry.value())
        } else {
            entry.value()
        };
        let spec = f.spec(shown.as_str(), 11.0);
        f.label_right(
            &spec,
            value,
            match &entry.kind {
                settings::Kind::ReadOnly(_) => paint::FAINT,
                _ if on => paint::TEXT,
                _ => paint::DIM,
            },
        );
    }

    // What the highlighted setting does. Not on About, which has no column.
    if plain {
        return;
    }
    f.pane(aside, f.glass_tint, size::ROUND);
    if let Some(entry) = pane.entries.get(lib.option_at) {
        let inner = aside.inset(size::PAD);
        let title = f.wrapped(entry.label, size::TITLE, inner.w, 2);
        let used = f.label(&title, inner, paint::TEXT);
        let body = Rect {
            y: inner.y + used + 8.0,
            h: inner.h - used - 8.0,
            ..inner
        };
        let help = f.wrapped(entry.help, size::LABEL, body.w, 8);
        f.label(&help, body, paint::DIM);
    }
}

/// A tab that is in the row but has nothing behind it yet.
///
/// It says which one and that it is coming, rather than drawing an empty
/// frame. The shoulder buttons cycle every tab whether or not it is built, so
/// landing here has to be legible — an empty page reads as a crash.
fn draw_unbuilt(f: &mut Frame, id: &str, area: Rect) {
    let (what, why) = match id {
        "mine" => (
            "Collections",
            "Your RomM collections. The data is already in the cache; this page is not built yet.",
        ),
        "history" => ("History", "Everything played, and when. Not built yet."),
        "syncing" => (
            "Syncing",
            "Pulling the library and pushing saves. Nothing here yet — this is the one that has to be written from scratch.",
        ),
        "settings" => (
            "Settings",
            "Bindings, artwork and the library path. Not built yet.",
        ),
        _ => ("Not built", "Nothing here yet."),
    };
    // Title and sentence as one block near the middle, not one at a third of
    // the way down and the other at two thirds with the page between them.
    let inner = area.inset(size::PAD);
    let (top, rest) = inner.split_top(inner.h * 0.46);
    let spec = f.spec(what, 20.0);
    f.label_centered(&spec, top, paint::DIM);
    let spec = f.wrapped(why, size::LABEL, rest.w.min(330.0), 4);
    f.label_centered(&spec, Rect { h: 54.0, ..rest }, paint::FAINT);
}

/// The tab row and the header, across the top.
fn draw_chrome(
    f: &mut Frame,
    lib: &library::Library,
    tabs: Rect,
    header: Rect,
    status: &status::Status,
) {
    f.pane(
        Rect::new(tabs.x, tabs.y, tabs.w, tabs.h + header.h),
        paint::BAR,
        0.0,
    );

    // How much of the row the corner will want, measured before the tabs are
    // laid out. It is drawn last and has to be known first, or the tab row
    // sizes itself against space the clock is about to take.
    let status_width: f32 = status
        .parts()
        .iter()
        .map(|part| {
            let w = match part {
                status::Part::Text(text) => {
                    let spec = f.spec(text.as_str(), 10.0);
                    let (w, _) = f.painter.measure(f.gfx, &spec);
                    f.screen.scale.pt(w as f32)
                }
                status::Part::Wifi(_) => f.screen.scale.pt(status::WIFI_SIZE.0 as f32),
                status::Part::Battery { .. } => {
                    f.screen.scale.pt(status::BATTERY_SIZE.0 as f32)
                }
            };
            w + size::GAP
        })
        .sum::<f32>()
        // Air between the last tab and the first thing in the corner. Without
        // it they are merely adjacent, and "Syncing 6:01 PM" reads as one line
        // of text rather than two different things.
        + size::GAP * 2.0;

    // Tabs: their own size, inside their own half of the row.
    //
    // Two rules, and the first one used to be missing. The row is *split*: the
    // corner takes what it measured above and the tabs get the rest, so a long
    // name cannot slide under the clock however many tabs there are. On this
    // panel that is 640 pixels less about 120 for the corner — a measurement,
    // not a number written down, because the corner grows a charging bolt and
    // loses a Wi-Fi symbol depending on the moment.
    //
    // And the type does not shrink to fit. It did, and the answer was a tab row
    // set two points smaller than everything around it — legible, and obviously
    // the loser of an argument. When the names do not fit, the row scrolls to
    // keep the current one in view, which is what a row of tabs on a small
    // screen has always done.
    let room = tabs.w - size::GAP * 2.0 - status_width;
    let font = size::TAB_TEXT;
    let widths: Vec<f32> = library::SECTIONS
        .iter()
        .map(|s| {
            let spec = f.spec(s.label, font);
            let (w, _) = f.painter.measure(f.gfx, &spec);
            f.screen.scale.pt(w as f32) + size::TAB_PAD * 2.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + size::TAB_GAP * (widths.len() - 1) as f32;

    // Where the row starts, so the chosen tab is inside the room there is.
    // Zero when everything fits, which is the usual case and must not shift.
    let mut offset = 0.0f32;
    if total > room {
        let before: f32 = widths[..lib.section]
            .iter()
            .map(|w| w + size::TAB_GAP)
            .sum();
        let here = widths[lib.section];
        // Far enough left that the chosen tab's right edge is inside, and never
        // so far that the row pulls away from its own start.
        offset = (before + here - room).clamp(0.0, total - room);
    }

    let left = tabs.x + size::GAP;
    let mut x = left - offset;
    for (i, section) in library::SECTIONS.iter().enumerate() {
        let width = widths[i];
        let slot = Rect::new(x, tabs.y, width, tabs.h);
        x += width + size::TAB_GAP;
        // Outside its half of the row: not drawn at all rather than drawn over
        // the clock.
        // Both edges. Testing only where a tab *starts* lets one that starts
        // inside and ends outside draw anyway — which is "Syncing" printed over
        // the clock, and it is what the split was supposed to prevent.
        if slot.right() < left || slot.right() > left + room {
            continue;
        }
        let spec = f.spec(section.label, font);
        let on = i == lib.section;
        if on {
            // The glow, then the bar.
            //
            // Stacked translucent rectangles rather than a blur pass: three
            // draws against a whole extra framebuffer, and at this size the
            // difference is not visible. Each is wider, taller and fainter than
            // the last, which is what a light source spilling past its own edge
            // looks like.
            for step in (1..=6).rev() {
                let spread = step as f32 * 1.6;
                let fade = 1.0 / (step as f32 * step as f32);
                let glow = Rgba(
                    paint::CURSOR.0,
                    paint::CURSOR.1,
                    paint::CURSOR.2,
                    0.30 * fade,
                );
                let h = 3.0 + spread * 1.6;
                f.fill(
                    Rect::new(
                        slot.x - spread * 0.5,
                        slot.bottom() - 1.5 - h / 2.0,
                        slot.w + spread,
                        h,
                    ),
                    glow,
                    h / 2.0,
                );
            }
            f.fill(
                Rect::new(slot.x, slot.bottom() - 3.0, slot.w, 3.0),
                paint::CURSOR,
                2.0,
            );
        }
        f.label_centered(
            &spec,
            slot,
            // A tab that does nothing yet is dimmer rather than missing.
            if on {
                paint::TEXT
            } else if section.ready {
                paint::DIM
            } else {
                paint::FAINT
            },
        );
        f.hits.tab(f.px(slot), i);
    }

    // The corner every device has: clock, then signal, then charge. Laid out
    // from the right edge inwards so the clock keeps its place as the other two
    // appear and disappear — a time that shuffles sideways when the charger
    // goes in is the sort of thing you notice every time.
    let mut right = tabs.right() - size::GAP;
    for part in status.parts().iter().rev() {
        let w = match part {
            status::Part::Text(text) => {
                let spec = f.spec(text.as_str(), 10.0);
                let (w, _) = f.painter.measure(f.gfx, &spec);
                let w = f.screen.scale.pt(w as f32);
                let slot = Rect::new(right - w, tabs.y, w, tabs.h);
                f.label_centered(&spec, slot, paint::DIM);
                w
            }
            // Arcs, not bars. Drawn at exactly the size it was made, so its
            // pixels are its pixels — an icon of this size resampled by even a
            // fraction is a smear rather than a symbol.
            status::Part::Wifi(bars) => {
                let (iw, ih) = status::WIFI_SIZE;
                let (w, h) = (f.screen.scale.pt(iw as f32), f.screen.scale.pt(ih as f32));
                let at = Rect::new(right - w, tabs.y + (tabs.h - h) / 2.0, w, h);
                f.signal(*bars, at);
                w
            }
            status::Part::Battery { percent, charging } => {
                let (iw, ih) = status::BATTERY_SIZE;
                let (w, h) = (f.screen.scale.pt(iw as f32), f.screen.scale.pt(ih as f32));
                let at = Rect::new(right - w, tabs.y + (tabs.h - h) / 2.0, w, h);
                f.charge(*percent, *charging, at);
                w
            }
        };
        right = right - w - size::GAP;
    }

    // The line that used to sit between the tabs and the page — where you are
    // on the left, how the list is arranged on the right — is off. It said
    // things the screen already showed, and on a 640-point panel it cost
    // twenty points of height to do it. What is there instead is a gap.
    //
    // Kept rather than deleted: turning it back on is uncommenting this and
    // restoring `size::HEADER` in the page split.
    let _ = header;
    // // Where you are on the left, how the list is arranged on the right.
    // let inner = header.inset(Edges::xy(size::GAP, 0.0));
    // // What the line under the tabs says. One useful fact per screen, not a
    // // count of the furniture — "8 groups" told you how the settings happened to
    // // be filed, which is not something anybody wants to know.
    // let here = match (lib.section().id, lib.view) {
    //     // Left as it was: Frank said this one already reads right.
    //     ("library", library::View::Platforms) => format!("{} consoles", lib.consoles.len()),
    //     ("library", library::View::Roms) => match lib.console() {
    //         Some(c) if !lib.query.is_empty() => {
    //             format!(
    //                 "{} — {} matching \u{201c}{}\u{201d}",
    //                 c.name,
    //                 lib.shown(),
    //                 lib.query
    //             )
    //         }
    //         Some(c) => format!("{} — {} games", c.name, lib.shown()),
    //         None => String::new(),
    //     },
    //     ("library", library::View::Scripts) => match lib.console() {
    //         Some(c) => format!("{} — {}", c.name, lib.ports.len()),
    //         None => String::new(),
    //     },
    //     ("mine", library::View::Groups) => match (lib.mine_count, lib.shelves.len()) {
    //         (0, n) => format!("{n} from RomM"),
    //         (n, all) if n == all => format!("{n} you made"),
    //         (n, all) => format!("{n} you made · {} from RomM", all - n),
    //     },
    //     ("mine", library::View::Collections) => format!("{} collections", lib.cols.len()),
    //     ("mine", _) => format!("{} — {} games", lib.col_name, lib.shown()),
    //     ("history", _) => {
    //         let (games, secs) = (lib.sync.games_played, lib.sync.seconds_played);
    //         format!("{games} games · {}", played_for(secs))
    //     }
    //     ("settings", library::View::Panes) => "How this device behaves".to_owned(),
    //     ("settings", library::View::Wifi) => "Wi-Fi".to_owned(),
    //     ("settings", _) => lib
    //         .panes
    //         .get(lib.pane_at)
    //         .map(|p| p.label.to_owned())
    //         .unwrap_or_default(),
    //     (_, _) => lib.section().label.to_owned(),
    // };
    // let title = f.spec(here, size::TITLE);
    // let (_, th) = f.painter.measure(f.gfx, &title);
    // let centered =
    //     Rect::new(inner.x, inner.y, inner.w, inner.h).center(inner.w, f.screen.scale.pt(th as f32));
    // f.label(
    //     &title,
    //     Rect {
    //         x: inner.x,
    //         ..centered
    //     },
    //     paint::TEXT,
    // );
    //
    // // How the games are ordered, when there are games. No grid/list control:
    // // on this panel it is a list, and a button offering the arrangement that
    // // does not fit is a button that makes the screen worse.
    // if lib.view == library::View::Roms {
    //     let filters = lib.filters();
    //     let arranged = if filters.is_empty() {
    //         lib.order_label().to_owned()
    //     } else {
    //         format!("{}  ·  {}", lib.order_label(), filters.join(" + "))
    //     };
    //     let spec = f.spec(arranged, 11.0);
    //     f.label_right(&spec, centered, paint::DIM);
    // }
}

/// The consoles: a grid of tiles where there is room, a list where there is
/// not — the same two shapes the webview switches between.
fn draw_consoles(f: &mut Frame, lib: &mut library::Library, area: Rect, focused: bool) {
    if area.w < size::TILE_MIN * 2.0 + size::GAP {
        let fits = (area.h / size::ROW).floor().max(1.0) as usize;
        let first = lib.console_at.saturating_sub(fits.saturating_sub(1));
        for (offset, console) in lib.consoles.iter().enumerate().skip(first).take(fits) {
            let slot = Rect::new(
                area.x,
                area.y + (offset - first) as f32 * size::ROW,
                area.w,
                size::ROW,
            );
            f.hits.row(f.px(slot), offset);
            let on = offset == lib.console_at;
            f.hovering(offset, slot, size::ROUND_SMALL);
            if on {
                match focused {
                    true => f.fill(slot, paint::CURSOR, size::ROUND_SMALL),
                    false => f.pane(slot, paint::CARD, size::ROUND_SMALL),
                }
            }
            let [name, count] = <[Rect; 2]>::try_from(
                slot.inset(Edges::xy(8.0, 5.0))
                    .row(6.0, &[Size::Grow(1.0), Size::Fixed(46.0)]),
            )
            .unwrap();
            let spec = f.wrapped(&console.name, size::TITLE, name.w, 1);
            f.label(&spec, name, if on { paint::TEXT } else { paint::DIM });
            let spec = f.spec(console.games.to_string(), 11.0);
            f.label_right(&spec, count, if on { paint::TEXT } else { paint::FAINT });
        }
        return;
    }

    // The grid is a count of tiles, not a tile size.
    //
    // It used to be `area.fits(GAP, TILE)` — how many 148-point tiles go in the
    // width — which is a desktop question. On a 320-point panel the answer is
    // two, and two enormous consoles filling the screen is not a shelf. What is
    // wanted is a *shape*: three across, and as many rows as fit that.
    //
    // So the tile takes its size from the room divided by the count, and the
    // count comes down on a panel too narrow to hold it rather than the tile
    // staying fixed and the grid emptying out.
    let across = size::GRID_COLUMNS.min(lib.consoles.len().max(1)).max(1);
    lib.relayout(across);
    let grid = area.tracks(size::GAP, across);
    // Rows come from the count too, not from the tile.
    //
    // Both halves have to be a count or neither is: deriving the height from
    // the width gave a tile 149 points tall in 258 points of room, which is one
    // row and a gap — three consoles on a screen that was asked for six. The
    // room divided by the rows wanted is the height, and whatever is left after
    // the caption is the picture.
    let want = size::GRID_ROWS;
    let tile_h = ((area.h - size::GAP * (want - 1) as f32) / want as f32).max(size::TILE_MIN);
    // How many of those actually fit. Normally `want`; fewer only when the
    // height floor kicked in on a very short panel.
    //
    // The gap is added to the room as well as to the tile, or the arithmetic
    // eats its own tail: a height derived from two rows, divided by that height
    // plus a gap, is always a little under two and floors to one. Which is what
    // it did — two rows asked for, three consoles drawn.
    let rows = (((area.h + size::GAP) / (tile_h + size::GAP)).floor() as usize).clamp(1, want);
    let first_row = (lib.console_at / across).saturating_sub(rows.saturating_sub(2));

    for (offset, console) in lib.consoles.iter().enumerate() {
        let row = offset / across;
        if row < first_row || row >= first_row + rows {
            continue;
        }
        let slot = grid.cell(offset - first_row * across, tile_h);
        // The card is smaller than the space it sits in, and centred there.
        //
        // Filling the slot edge to edge made six cards that touched, which
        // reads as one panel divided by lines rather than as six things. The
        // grid still decides where they go — three across, two down — and the
        // card just takes less of it.
        let tile = slot.inset(Edges::all(size::TILE_INSET));
        f.hits.row(f.px(tile), offset);
        let on = offset == lib.console_at;

        // Barely there, and only the glass behind it.
        //
        // A console tile is mostly its own picture — a machine rendered on
        // nothing, with the backdrop showing through. A solid plate under it
        // turns a shelf of hardware into a shelf of grey rectangles with
        // hardware printed on them.
        f.fill(tile, paint::TILE, size::ROUND);
        f.hovering(offset, tile, size::ROUND);
        if on {
            // A breathing outline, the way `canvas-es` marks its selection.
            //
            // A still 2-point border says "this one" and stops saying it after
            // a second; a slow pulse keeps saying it, and on a shelf of six
            // cards it is what the eye comes back to. Two seconds a breath —
            // any faster and it is a warning light.
            let phase = if f.animate {
                (f.now / 1000.0 * std::f64::consts::TAU / 2.0).sin() as f32
            } else {
                0.0
            };
            let lit = 0.72 + 0.28 * phase;
            let glow = Rgba(paint::CURSOR.0, paint::CURSOR.1, paint::CURSOR.2, lit);
            // The halo first, wider and much fainter, then the line itself.
            for step in (1..=3).rev() {
                let spread = step as f32 * 1.6;
                let fade = 0.16 * lit / (step as f32 * step as f32);
                f.outline(
                    tile.inset(Edges::all(-spread)),
                    2.0,
                    Rgba(paint::CURSOR.0, paint::CURSOR.1, paint::CURSOR.2, fade),
                    size::ROUND + spread,
                );
            }
            f.outline(tile, 2.0, glow, size::ROUND);
        }

        let inner = tile.inset(Edges::all(8.0));
        // The picture takes what the caption does not want. A fixed art height
        // inside a tile whose height is now computed is the one number that
        // would still not scale.
        let (art, caption) = inner.split_top((inner.h - size::TILE_CAPTION).max(inner.h * 0.4));
        f.console_art(&console.slug, art, 0.0);

        let name = f.wrapped(&console.name, size::LABEL, caption.w, 2);
        let used = f.label(&name, caption, if on { paint::TEXT } else { paint::DIM });
        let under = Rect {
            y: caption.y + used + 4.0,
            h: 14.0,
            ..caption
        };
        let [dot, count] =
            <[Rect; 2]>::try_from(under.row(4.0, &[Size::Fixed(9.0), Size::Grow(1.0)])).unwrap();
        let spec = f.spec("●", 9.0);
        f.label(&spec, dot, paint::HERE);
        let spec = f.spec(format!("{} games", console.games), 11.0);
        f.label(&spec, count, paint::FAINT);
    }
}

/// One console's games.
fn draw_game_list(f: &mut Frame, lib: &mut library::Library, area: Rect) {
    // The bar is drawn once, under all the rows, at wherever it has slid to.
    //
    // It used to be drawn inside the loop, on the chosen row — so the bar
    // slid and the text did not, and half the highlight arrived before the
    // other half. A row is text; the cursor is one thing moving behind it.
    let step = size::ROW;
    lib.relayout(1);

    let (shown, at, was) = (lib.shown(), lib.at, lib.scrolled);
    let ask = |top: f32| rowwindow::Ask::new(shown, 1, step, top, area.h);
    let top = rowwindow::scroll_to(at, ask(was)).unwrap_or(was);
    lib.scrolled = top;
    let band = rowwindow::band(ask(top));

    let rows: Vec<_> = lib
        .showing()
        .enumerate()
        .skip(band.first)
        .take(band.count)
        .map(|(i, (r, _))| (i, r.name.clone(), r.favorite, r.downloaded))
        .collect();

    // Where the chosen row will land, before anything is drawn over it.
    for (i, ..) in rows.iter() {
        if *i == lib.at {
            let line = Rect::new(area.x, area.y + *i as f32 * step - top, area.w, step);
            if line.bottom() >= area.y && line.bottom() <= area.bottom() {
                let at = f.glide.at(line, f.now, f.animate);
                f.pane(at, paint::CURSOR, size::ROUND_SMALL);
            }
        }
    }

    for (i, name, favorite, downloaded) in rows {
        let line = Rect::new(area.x, area.y + i as f32 * step - top, area.w, step);
        if line.bottom() < area.y || line.bottom() > area.bottom() {
            continue;
        }
        f.hits.row(f.px(line), i);
        let on = i == lib.at;
        f.hovering(i, line, size::ROUND_SMALL);


        // No console column. Every row in this list is the console named in
        // the header, so it said the same word ninety-four times down the page
        // and took ninety points the titles needed — "Alex Kidd in the..." was
        // being cut off to print "megadrive" beside it.
        let inside = line.inset(Edges::xy(8.0, 5.0));
        // No size column either. It was sixty-six points of every row saying
        // how big a file is, which is a thing you look up once about one game
        // and never scan down a list for — so it lives in the pane now, where
        // the questions about one game are answered.
        let [mark, title] =
            <[Rect; 2]>::try_from(inside.row(8.0, &[Size::Fixed(12.0), Size::Grow(1.0)])).unwrap();

        let spec = f.spec(if downloaded { "\u{25cf}" } else { "\u{25cb}" }, 11.0);
        f.label(
            &spec,
            mark,
            if downloaded { paint::HERE } else { paint::AWAY },
        );

        // The star sits before the name rather than beside it, so a starred
        // game and a plain one line their names up anyway.
        let mut left = title;
        if favorite {
            let spec = f.spec("\u{2605}", 11.0);
            let (w, _) = f.painter.measure(f.gfx, &spec);
            let w = f.screen.scale.pt(w as f32) + 5.0;
            f.label(&spec, left, paint::STAR);
            left = Rect {
                x: left.x + w,
                w: left.w - w,
                ..left
            };
        }
        let spec = f.wrapped(&name, size::LABEL, left.w, 1);
        f.label(&spec, left, if on { paint::TEXT } else { paint::DIM });    }
}

/// The preview column: the cover, the name, and what is known about it.
fn draw_detail(f: &mut Frame, lib: &mut library::Library, area: Rect) {
    let Some(detail) = lib.detail_full() else {
        return;
    };
    // The picture takes its shape from the cover and its limit from the pane.
    //
    // Height from the aspect alone was fine when the column was a third of the
    // screen; at half the width the same rule makes it tall enough to push the
    // name and every fact off the bottom. Half the pane, at most.
    let tall = (area.w / lib.aspect.clamp(0.3, 3.0)).min(area.h * 0.52);
    let (art, rest) = area.split_top(tall);
    // The game's own cover if there is one, and the console's picture if not.
    //
    // And if neither — which is every game in a library scanned from a card
    // with no artwork on it — the space is not reserved at all. It used to be:
    // two thirds of the pane held an empty grey box, and the facts underneath
    // were pushed off the bottom, so the column showed a blank rectangle,
    // Console and Size. That is what "the side bar is meaningless" was.
    let drew = f.cover(
        detail.id,
        &detail.platform,
        &detail.stem,
        art,
        size::ROUND_SMALL,
    ) || f.console_art(&detail.platform, art, size::ROUND_SMALL);
    let rest = if drew { rest } else { area };

    // The one place a title is not cut short: the card had to fit it into 150
    // points and this column is where the whole thing goes.
    let below = rest.inset(Edges {
        top: size::GAP,
        ..Edges::default()
    });
    let name = f.wrapped(&detail.name, size::TITLE, below.w, 4);
    let used = f.label(&name, below, paint::TEXT);
    let mut y = below.y + used + size::GAP;

    for (label, value) in detail.facts() {
        let line = Rect::new(below.x, y, below.w, 16.0);
        if line.bottom() > area.bottom() {
            break;
        }
        let [left, right] = <[Rect; 2]>::try_from(line.cols(6.0, &[5, 7])).unwrap();
        let spec = f.spec(label, 11.0);
        f.label(&spec, left, paint::FAINT);
        let spec = f.wrapped(&value, 11.0, right.w, 1);
        f.label(&spec, right, paint::DIM);
        y += 17.0;
    }

    // The blurb, under a rule, in whatever room is left.
    //
    // Last because it is the one thing here of no fixed length: the facts are
    // a line each and this is a paragraph, so it takes the remainder rather
    // than pushing anything off the bottom.
    let Some(summary) = detail.summary.as_deref() else {
        return;
    };
    y += size::GAP;
    if y + 24.0 > area.bottom() {
        return;
    }
    f.fill(Rect::new(below.x, y, below.w, 1.0), paint::RULE, 0.0);
    y += size::GAP + 2.0;
    let room = area.bottom() - y;
    // As many lines as fit, and no more: a paragraph cut off mid-word at the
    // panel edge is worse than one that stops at a line.
    let lines = ((room / 14.0).floor() as u16).clamp(1, 12);
    let spec = f.wrapped(summary, 10.0, below.w, lines);
    f.label(&spec, Rect::new(below.x, y, below.w, room), paint::DIM);
}

#[cfg(test)]
mod loop_tests {
    use super::collect;

    /// The event that ended the wait is handled, not thrown away.
    ///
    /// Between interactions this app is asleep, so the event that wakes it is
    /// the first press of whatever somebody is doing. Losing it loses every
    /// first press — which is what "the mouse is not working" turned out to be,
    /// and the pad with it.
    #[test]
    fn the_event_that_woke_us_is_not_lost() {
        assert_eq!(collect(Some("click"), ["motion"]), ["click", "motion"]);
        assert_eq!(collect(Some("click"), []), ["click"]);
        assert_eq!(collect(None::<&str>, ["motion"]), ["motion"]);
        assert!(collect(None::<&str>, []).is_empty());
    }
}

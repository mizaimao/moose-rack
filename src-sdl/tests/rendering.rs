// What the screen actually gets.
//
// Written after Frank asked to stop being the test harness, and he was right
// to: three of the last four bugs — a backdrop painted over, a shader in the
// wrong dialect, every label drawn as a solid block — were invisible to
// `cargo test` and obvious in a screenshot. The suite was green through all of
// them.
//
// So this opens a real window, hidden, draws into it, and reads the pixels
// back. It is slower than a unit test and it needs a display, which CI has
// none of — so it skips there rather than failing. On a machine with a screen
// it is the difference between "the code compiles" and "the picture is right".

use std::sync::OnceLock;

/// A hidden window and a context, made once for the whole file.
///
/// SDL does not like being initialised twice in a process, and `cargo test`
/// runs these on threads of one process.
struct Screen {
    _sdl: sdl2::Sdl,
    video: sdl2::VideoSubsystem,
    window: sdl2::video::Window,
    _context: sdl2::video::GLContext,
}

// SDL is not Sync, and this is only ever touched from the one test thread
// below. `--test-threads=1` is not enough on its own to convince the compiler.
unsafe impl Sync for Screen {}
unsafe impl Send for Screen {}

/// The size MOOSE_SDL_DUMP renders at: the handheld's panel, so what is looked
/// at is what will be seen.
const DUMP: (u32, u32) = (640, 480);
const WIDTH: u32 = 200;
const HEIGHT: u32 = 120;

fn screen() -> Option<&'static Screen> {
    static ONCE: OnceLock<Option<Screen>> = OnceLock::new();
    ONCE.get_or_init(|| match build_screen() {
        Ok(screen) => Some(screen),
        Err(why) => {
            // Said out loud, and in detail. The first version of this returned
            // `None` on any failure and every check quietly skipped — so the
            // suite reported nine passes while the screen was full of grey
            // blocks, which is worse than having no tests at all.
            eprintln!("no display for the rendering checks: {why}");
            None
        }
    })
    .as_ref()
}

fn build_screen() -> Result<Screen, String> {
    let sdl = sdl2::init().map_err(|e| format!("init: {e}"))?;
    let video = sdl.video().map_err(|e| format!("video: {e}"))?;
    {
        let attr = video.gl_attr();
        attr.set_context_profile(sdl2::video::GLProfile::Core);
        attr.set_context_version(3, 3);
    }
    let window = video
        .window("test", WIDTH, HEIGHT)
        .opengl()
        .hidden()
        .build()
        .map_err(|e| format!("window: {e}"))?;
    let context = window.gl_create_context().map_err(|e| format!("context: {e}"))?;
    window.gl_set_context_to_current().map_err(|e| format!("current: {e}"))?;
    gl::load_with(|name| video.gl_get_proc_address(name) as *const _);
    Ok(Screen { _sdl: sdl, video, window, _context: context })
}

/// Read the frame back, as rows of RGBA from the top down.
///
/// GL hands them back bottom-up, which is the sort of thing that makes a test
/// pass while the picture is upside down.
fn read_pixels() -> Vec<u8> {
    let mut flipped = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    unsafe {
        gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
        gl::ReadPixels(
            0,
            0,
            WIDTH as i32,
            HEIGHT as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            flipped.as_mut_ptr() as *mut _,
        );
    }
    let row = WIDTH as usize * 4;
    let mut out = vec![0u8; flipped.len()];
    for y in 0..HEIGHT as usize {
        let from = (HEIGHT as usize - 1 - y) * row;
        out[y * row..(y + 1) * row].copy_from_slice(&flipped[from..from + row]);
    }
    out
}

/// The same read as `read_pixels`, off an offscreen target rather than the
/// window, and flipped the same way: GL hands rows back bottom-up.
///
/// # Safety
///
/// A context must be current and `target` must belong to it.
unsafe fn read_target(target: &moose_sdl::gfx::Offscreen) -> Vec<u8> {
    let (w, h) = (DUMP.0 as usize, DUMP.1 as usize);
    let mut flipped = vec![0u8; w * h * 4];
    unsafe {
        gl::BindFramebuffer(gl::FRAMEBUFFER, target.frame_id());
        gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
        gl::ReadPixels(
            0, 0, w as i32, h as i32,
            gl::RGBA, gl::UNSIGNED_BYTE,
            flipped.as_mut_ptr() as *mut _,
        );
        gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
    }
    let row = w * 4;
    let mut out = vec![0u8; flipped.len()];
    for y in 0..h {
        let from = (h - 1 - y) * row;
        out[y * row..(y + 1) * row].copy_from_slice(&flipped[from..from + row]);
    }
    out
}

fn pixel(frame: &[u8], x: u32, y: u32) -> (u8, u8, u8) {
    let at = (y as usize * WIDTH as usize + x as usize) * 4;
    (frame[at], frame[at + 1], frame[at + 2])
}

/// Everything a test wants: a renderer on a live context.
fn with_gfx(body: impl FnOnce(&mut moose_sdl::gfx::Gfx)) -> bool {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return false;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    body(&mut gfx);
    screen.window.gl_swap_window();
    true
}


/// Everything, in order, on the main thread.
///
/// Not a `#[test]`, and the crate says `harness = false`, for two reasons that
/// both bit: a GL context belongs to the thread that made it current and
/// libtest runs tests on several, and on macOS SDL's video subsystem must be
/// initialised on the real main thread or it reports "No available video
/// device". Under the harness this file reported nine passes while the screen
/// was full of grey blocks — every check was skipping.
fn main() {
    if screen().is_none() {
        // A skip, and a loud one. CI has no display; a developer's machine
        // does, and a silent skip there is worse than no test.
        eprintln!("SKIPPED: the rendering checks did not run");
        return;
    }
    a_filled_rectangle_reaches_the_screen();
    a_coverage_texture_is_not_a_solid_block();
    an_image_keeps_its_own_colours();
    an_odd_width_is_not_sheared();
    every_style_compiles();
    the_jitter_table_can_be_remeasured();
    the_backdrop_covers_the_frame();
    the_backdrop_moves_over_time();
    the_backdrop_does_not_depend_on_how_many_pixels_it_has();
    a_drawn_word_has_gaps_in_it();
    a_wrapped_label_at_retina_scale_has_gaps_too();
    a_label_after_the_backdrop_still_has_gaps();
    the_glass_actually_blurs();
    a_panel_shows_what_is_behind_it();
    rounded_corners_follow_the_quad_not_the_texture();
    println!("all rendering checks passed");
}

fn a_filled_rectangle_reaches_the_screen() {
    let mut frame = Vec::new();
    if !with_gfx(|gfx| {
        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        gfx.rect(50.0, 30.0, 100.0, 60.0, moose_sdl::gfx::Rgba::rgb(255, 0, 0));
        frame = read_pixels();
    }) {
        return;
    }
    assert_eq!(pixel(&frame, 100, 60), (255, 0, 0), "the middle of the rectangle");
    assert_eq!(pixel(&frame, 10, 10), (0, 0, 0), "outside it");
    // And the right way up: the rectangle starts a quarter down, not up.
    assert_eq!(pixel(&frame, 100, 10), (0, 0, 0), "drawn upside down");
}

/// The bug this file was written for.
///
/// A label is a coverage mask — white with the glyph's coverage as its alpha —
/// tinted at draw time. Drawn as a solid block it means the alpha never
/// reached the shader, and every name in the library becomes a grey rectangle
/// the size of the word. Which is exactly what happened, and what a green
/// suite said nothing about.
fn a_coverage_texture_is_not_a_solid_block() {
    let mut frame = Vec::new();
    if !with_gfx(|gfx| {
        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        // Left half opaque, right half transparent. Eight wide so the row is
        // not a multiple of four bytes either — which is the other thing that
        // silently shears an upload.
        let coverage: Vec<u8> = (0..8 * 8)
            .map(|i| if i % 8 < 4 { 255 } else { 0 })
            .collect();
        let texture = gfx.upload_coverage(8, 8, &coverage);
        gfx.image(&texture, 0.0, 0.0, 160.0, 80.0, moose_sdl::gfx::Rgba::rgb(0, 255, 0));
        frame = read_pixels();
    }) {
        return;
    }
    assert_eq!(pixel(&frame, 20, 40), (0, 255, 0), "the covered half did not draw");
    assert_eq!(
        pixel(&frame, 140, 40),
        (0, 0, 0),
        "the transparent half drew anyway — the coverage never became alpha, \
         so every label is a solid block"
    );
}

/// A picture is drawn as it is, not tinted by whatever was last set.
fn an_image_keeps_its_own_colours() {
    let mut frame = Vec::new();
    if !with_gfx(|gfx| {
        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        // Four pixels: red, green, blue, white.
        #[rustfmt::skip]
        let pixels: Vec<u8> = vec![
            255, 0, 0, 255,   0, 255, 0, 255,
            0, 0, 255, 255,   255, 255, 255, 255,
        ];
        let texture = gfx.upload_rgba(2, 2, &pixels);
        gfx.image(&texture, 0.0, 0.0, 200.0, 120.0, moose_sdl::gfx::Rgba::WHITE);
        frame = read_pixels();
    }) {
        return;
    }
    assert_eq!(pixel(&frame, 25, 15), (255, 0, 0), "top left");
    assert_eq!(pixel(&frame, 175, 15), (0, 255, 0), "top right");
    assert_eq!(pixel(&frame, 25, 105), (0, 0, 255), "bottom left");
}

/// Rows that are not a multiple of four bytes wide.
///
/// GL unpacks with four-byte alignment unless told otherwise, and a text
/// raster is exactly as wide as the word in it — so an odd width shears the
/// image diagonally, which reads as a font problem rather than an upload one.
fn an_odd_width_is_not_sheared() {
    let mut frame = Vec::new();
    if !with_gfx(|gfx| {
        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        // Three wide: the first column opaque, the rest not.
        let coverage: Vec<u8> = (0..3 * 3).map(|i| if i % 3 == 0 { 255 } else { 0 }).collect();
        let texture = gfx.upload_coverage(3, 3, &coverage);
        gfx.image(&texture, 0.0, 0.0, 180.0, 120.0, moose_sdl::gfx::Rgba::rgb(255, 255, 255));
        frame = read_pixels();
    }) {
        return;
    }
    // The left third is lit on every row. Sheared, the lit column walks across.
    for y in [20, 60, 100] {
        assert_eq!(pixel(&frame, 20, y).0, 255, "row {y} lost its first column");
        assert_eq!(pixel(&frame, 100, y).0, 0, "row {y} is sheared");
    }
}

/// Every style compiles, and every style draws something.
///
/// Only two of them were ever built here — blobs and plasma — so a GLSL mistake
/// anywhere else reached the device untouched. It is not a small class of
/// mistake: `programFor` returns null on a failed compile and the style simply
/// never switches, with nothing on screen to say why, which is how Sweep and
/// Starfield both shipped broken. There is no GLSL compiler on a developer
/// machine to check against; this hidden context is the one there is.
///
/// It also dumps a frame per style when MOOSE_SDL_DUMP names a directory, which
/// is how a new style gets looked at without opening the app: raw RGBA at the
/// size below, top row first.
fn every_style_compiles() {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let dump = std::env::var("MOOSE_SDL_DUMP").ok();

    for (id, label) in moose_sdl::backdrop::STYLE_LIST {
        let backdrop = unsafe { moose_sdl::backdrop::Backdrop::build(&screen.video, id) }
            .unwrap_or_else(|e| panic!("{label} ({id}) does not compile: {e}"));

        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        // Not at time zero. Several of these start from a symmetry they leave
        // within a second, and a style judged at its first frame is judged on
        // the one frame that is not representative of it.
        unsafe { backdrop.draw(WIDTH as f32, HEIGHT as f32, 9.0) };
        let frame = read_pixels();
        screen.window.gl_swap_window();

        let lit = frame.chunks(4).filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0).count();
        assert!(
            lit > (WIDTH * HEIGHT) as usize / 8,
            "{label} ({id}) lit {lit} pixels of {}; it compiled and drew nothing",
            WIDTH * HEIGHT
        );

        if let Some(dir) = &dump {
            // Made rather than required to exist: this is a debugging aid, and
            // failing the whole rendering suite because a scratch directory was
            // cleaned up is not what it is for.
            std::fs::create_dir_all(dir).expect("the dump directory should be creatable");
            // Onto an offscreen target at the panel's size, not into this
            // file's 200x120 window. The checks above only need enough pixels
            // to count lit ones; a person looking at a backdrop to decide
            // whether the fog has enough structure in it needs the size it will
            // actually be seen at, and at 200 across every wisp is one pixel.
            let target =
                unsafe { moose_sdl::gfx::Offscreen::new(DUMP.0, DUMP.1) }.expect("a target");
            // Several moments, not one. These drift, wrap and tumble on periods
            // of tens of seconds, and a style judged on a single frame is judged
            // on whichever arrangement that second happened to hold.
            for (n, at) in [9.0f32, 40.0, 90.0, 150.0].iter().enumerate() {
                let shot = unsafe {
                    gfx.draw_onto(&target, |g| {
                        g.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
                        backdrop.draw(DUMP.0 as f32, DUMP.1 as f32, *at);
                    });
                    read_target(&target)
                };
                let path = std::path::Path::new(dir).join(format!("{id}-{n}.rgba"));
                std::fs::write(&path, &shot).expect("the dump directory should be writable");
            }
        }
    }
}

/// Print what STYLE_JITTER should say, when MOOSE_SDL_JITTER is set.
///
/// The table decides how often each style is redrawn on the handheld and it has
/// to be measured rather than guessed. The measurement already existed as
/// MOOSE_SDL_BENCH=motion, which needs the app running and a window on screen —
/// so in practice the table went stale whenever a style was tuned, and a style
/// whose worst pixel halved went on costing twice the frames it needed.
///
/// The same arithmetic, on the hidden context this file already has: the worst
/// level any one pixel moves between two frames drawn a second apart, at the
/// style's own pace and the scheme the handheld opens on. Mean is printed too
/// and is not what the table holds — a handful of bright points crossing a dark
/// field shift almost no average and are the most obvious thing on screen.
///
/// Silent unless asked. It is a measurement, not a check: what the numbers
/// should be is a judgement about frame rates, and asserting last week's
/// reading would fail every time a style was legitimately made calmer.
fn the_jitter_table_can_be_remeasured() {
    if std::env::var("MOOSE_SDL_JITTER").is_err() {
        return;
    }
    let Some(screen) = screen() else { return };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    // The panel's size, not this file's: a thin fast edge can cross a pixel at
    // 640x480 and fall between two at 200x120.
    let (w, h) = (640u32, 480u32);
    let target = unsafe { moose_sdl::gfx::Offscreen::new(w, h) }.expect("an offscreen target");

    println!("{:<12} {:>9} {:>9}", "style", "mean/s", "worst/s");
    for (id, _) in moose_sdl::backdrop::STYLE_LIST {
        let Ok(mut art) = (unsafe { moose_sdl::backdrop::Backdrop::build(&screen.video, id) })
        else {
            continue;
        };
        art.scheme = *moose_sdl::backdrop::scheme("midnight");
        let shot = |gfx: &mut moose_sdl::gfx::Gfx, at: f32| -> Vec<u8> {
            unsafe {
                gfx.draw_onto(&target, |g| {
                    g.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
                    art.draw(w as f32, h as f32, at);
                });
                let mut raw = vec![0u8; (w * h * 4) as usize];
                gl::BindFramebuffer(gl::FRAMEBUFFER, target.frame_id());
                gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
                gl::ReadPixels(
                    0, 0, w as i32, h as i32,
                    gl::RGBA, gl::UNSIGNED_BYTE,
                    raw.as_mut_ptr() as *mut _,
                );
                gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
                raw
            }
        };
        let a = shot(&mut gfx, 10.0);
        let b = shot(&mut gfx, 11.0);
        let (mut sum, mut worst) = (0u64, 0u64);
        for (p, q) in a.chunks(4).zip(b.chunks(4)) {
            for i in 0..3 {
                let step = (p[i] as i32 - q[i] as i32).unsigned_abs() as u64;
                sum += step;
                worst = worst.max(step);
            }
        }
        let n = (a.len() / 4 * 3) as f64;
        println!("{id:<12} {:>9.2} {worst:>9}", sum as f64 / n);
    }
}

/// The backdrop is a shader on the same context, and it has to actually put
/// something on the screen — the last one ran perfectly and was painted over.
fn the_backdrop_covers_the_frame() {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let backdrop = unsafe { moose_sdl::backdrop::Backdrop::build(&screen.video, "blobs") }
        .expect("the backdrop should compile on a context we asked for");

    gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
    unsafe { backdrop.draw(WIDTH as f32, HEIGHT as f32, 1.0) };
    let frame = read_pixels();
    screen.window.gl_swap_window();

    let lit = frame.chunks(4).filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0).count();
    assert!(
        lit > (WIDTH * HEIGHT) as usize / 2,
        "the backdrop lit {lit} pixels of {}; it is not covering the frame",
        WIDTH * HEIGHT
    );
}

/// The backdrop looks the same however many pixels it is given.
///
/// This is a promise, not an observation. The panel is 640x480 today and the
/// window is whatever it is, and one day there will be other sizes; a backdrop
/// whose pattern is tied to a pixel count is one that has to be re-judged every
/// time that changes. It is also the bug that hid behind the glass: the blurred
/// copy is drawn at a quarter size, and if the shader cared about that, every
/// panel would show a different pattern from the page it sits on.
///
/// Drawn at two sizes with the same aspect and the same time, boiled down to a
/// small grid, and compared. Boiled down because the two are not the same
/// number of pixels and cannot be compared pixel for pixel — what has to match
/// is the picture, not the sampling of it.
fn the_backdrop_does_not_depend_on_how_many_pixels_it_has() {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    let backdrop = unsafe { moose_sdl::backdrop::Backdrop::build(&screen.video, "blobs") }
        .expect("the backdrop should compile");

    // 200x120 and 100x60 — half in each direction, so the same shape.
    let mut coarse = |w: u32, h: u32| -> Vec<u8> {
        gfx.resize_at(0.0, 0.0, w as f32, h as f32);
        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        unsafe { backdrop.draw(w as f32, h as f32, 3.0) };
        let mut raw = vec![0u8; (w * h * 4) as usize];
        unsafe {
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
        }
        // Ten by six cells, each the average of whatever fell in it.
        let (cx, cy) = (10usize, 6usize);
        let mut cells = vec![0u8; cx * cy];
        for (i, cell) in cells.iter_mut().enumerate() {
            let (gx, gy) = (i % cx, i / cx);
            let (x0, x1) = (gx * w as usize / cx, (gx + 1) * w as usize / cx);
            let (y0, y1) = (gy * h as usize / cy, (gy + 1) * h as usize / cy);
            let mut sum = 0u64;
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * w as usize + x) * 4;
                    sum += raw[at] as u64 + raw[at + 1] as u64 + raw[at + 2] as u64;
                    n += 3;
                }
            }
            *cell = (sum / n.max(1)) as u8;
        }
        cells
    };

    let big = coarse(200, 120);
    let small = coarse(100, 60);
    screen.window.gl_swap_window();

    let worst = big
        .iter()
        .zip(&small)
        .map(|(a, b)| (*a as i32 - *b as i32).abs())
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 12,
        "the same backdrop drawn at two sizes differs by {worst} levels; \
         it is tied to the pixel count rather than to the shape of the screen"
    );
}

/// And it moves. A still backdrop is what a frozen clock looks like, and the
/// clock has been frozen twice.
fn the_backdrop_moves_over_time() {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let backdrop = unsafe { moose_sdl::backdrop::Backdrop::build(&screen.video, "plasma") }
        .expect("the backdrop should compile");

    let at = |seconds: f32| {
        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        unsafe { backdrop.draw(WIDTH as f32, HEIGHT as f32, seconds) };
        read_pixels()
    };
    let first = at(0.0);
    // Two seconds apart, which at the shipped speed is plainly different and
    // not a rounding difference.
    let later = at(2.0);
    screen.window.gl_swap_window();

    let changed = first.iter().zip(&later).filter(|(a, b)| a.abs_diff(**b) > 4).count();
    assert!(
        changed > first.len() / 20,
        "only {changed} of {} bytes changed over two seconds; the backdrop is not moving",
        first.len()
    );
}

/// A word, all the way through: shaped, rasterised, uploaded, drawn, read
/// back.
///
/// The one that matters. Every layer below it can be right and the label still
/// arrive as a rectangle — which is what happened — because nothing else
/// checks that the *gaps between the letters* are still gaps.
fn a_drawn_word_has_gaps_in_it() {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let mut painter = moose_sdl::text::Painter::new().expect("fonts");

    gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
    let spec = moose_sdl::text::Spec::new("IIIIIIII", 40.0, 1.0);
    let (w, h) = painter.measure(&gfx, &spec);
    painter.draw(&gfx, &spec, 10.0, 10.0, moose_sdl::gfx::Rgba::rgb(255, 255, 255));
    let frame = read_pixels();
    screen.window.gl_swap_window();

    assert!(w > 4 && h > 4, "the label measured {w}x{h}");
    // Inside the label's own box: some pixels lit, and some not. A solid block
    // has none of the second, and that is exactly how it looked.
    let mut lit = 0;
    let mut dark = 0;
    for y in 10..(10 + h).min(HEIGHT) {
        for x in 10..(10 + w).min(WIDTH) {
            if pixel(&frame, x, y).0 > 40 { lit += 1 } else { dark += 1 }
        }
    }
    assert!(lit > 0, "nothing was drawn at all");
    assert!(
        dark > lit / 4,
        "{lit} lit against {dark} dark inside the label — it is a solid block, \
         not letters"
    );
}

/// The same word the way the interface actually asks for it: wrapped to a
/// card's width, over two lines, at a retina scale.
///
/// Standalone it was fine and on screen it was a block, so the difference is
/// in the asking.
fn a_wrapped_label_at_retina_scale_has_gaps_too() {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let mut painter = moose_sdl::text::Painter::new().expect("fonts");

    gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
    let spec = moose_sdl::text::Spec::new("Metroid", 13.0, 2.0).wrapped(150.0, 2);
    let (w, h) = painter.measure(&gfx, &spec);
    painter.draw(&gfx, &spec, 4.0, 4.0, moose_sdl::gfx::Rgba::rgb(200, 200, 210));
    let frame = read_pixels();
    screen.window.gl_swap_window();

    let mut lit = 0;
    let mut dark = 0;
    for y in 4..(4 + h).min(HEIGHT) {
        for x in 4..(4 + w).min(WIDTH) {
            if pixel(&frame, x, y).0 > 40 { lit += 1 } else { dark += 1 }
        }
    }
    assert!(lit > 0, "nothing drawn");
    assert!(dark > lit / 4, "{lit} lit against {dark} dark — a solid block");
}

/// A label drawn *after* the backdrop.
///
/// The one the screenshot showed and every test above missed: the backdrop
/// turns blending off for its own quad — it is opaque and covers the frame —
/// and left it off. Everything drawn after it then ignored its alpha, so every
/// name in the library became a solid rectangle the colour of its tint.
///
/// Nothing here is about the backdrop. It is about one drawing step being able
/// to change the state another depends on, which is a thing a renderer has to
/// be robust to rather than a rule everyone remembers.
fn a_label_after_the_backdrop_still_has_gaps() {
    let Some(screen) = screen() else {
        eprintln!("no display here; skipping");
        return;
    };
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let backdrop = unsafe { moose_sdl::backdrop::Backdrop::build(&screen.video, "blobs") }
        .expect("the backdrop");
    let mut painter = moose_sdl::text::Painter::new().expect("fonts");

    gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
    unsafe { backdrop.draw(WIDTH as f32, HEIGHT as f32, 1.0) };

    let spec = moose_sdl::text::Spec::new("IIIIIIII", 40.0, 1.0);
    let (w, h) = painter.measure(&gfx, &spec);
    painter.draw(&gfx, &spec, 10.0, 10.0, moose_sdl::gfx::Rgba::rgb(255, 255, 255));
    let frame = read_pixels();
    screen.window.gl_swap_window();

    let mut lit = 0;
    let mut dark = 0;
    for y in 10..(10 + h).min(HEIGHT) {
        for x in 10..(10 + w).min(WIDTH) {
            if pixel(&frame, x, y).0 > 230 { lit += 1 } else { dark += 1 }
        }
    }
    assert!(lit > 0, "nothing was drawn at all");
    assert!(
        dark > lit / 4,
        "{lit} lit against {dark} dark — the label is a solid block, because \
         something before it left blending off"
    );
}

/// A blur that does not blur.
///
/// Measured rather than looked at: a sharp edge has a big jump between
/// neighbouring pixels and a blurred one does not, so the largest step along a
/// row is the whole of the test. The effect is the app's look, and "it renders
/// something" is not the same as "it is soft".
fn the_glass_actually_blurs() {
    let screen = screen().expect("checked");
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let mut glass = unsafe { moose_sdl::glass::Glass::new(WIDTH, HEIGHT) }.expect("glass");
    unsafe { glass.resize(WIDTH, HEIGHT) }.expect("resized");

    // Half black, half white, with one hard edge down the middle.
    let (bw, bh) = glass.blurred_size();
    unsafe {
        glass.capture(&mut gfx, |g| {
            g.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
            g.rect(bw as f32 / 2.0, 0.0, bw as f32 / 2.0, bh as f32, moose_sdl::gfx::Rgba::WHITE);
        });
    }

    // Draw the blurred copy back out at full size and read the edge.
    gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
    gfx.image(glass.blurred(), 0.0, 0.0, WIDTH as f32, HEIGHT as f32, moose_sdl::gfx::Rgba::WHITE);
    let soft = read_pixels();

    // And the same edge with no blur at all, for something to compare against.
    gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
    gfx.rect(WIDTH as f32 / 2.0, 0.0, WIDTH as f32 / 2.0, HEIGHT as f32, moose_sdl::gfx::Rgba::WHITE);
    let sharp = read_pixels();
    screen.window.gl_swap_window();

    let biggest_step = |frame: &[u8]| {
        (1..WIDTH)
            .map(|x| pixel(frame, x, 60).0.abs_diff(pixel(frame, x - 1, 60).0))
            .max()
            .unwrap_or(0)
    };
    let (soft_step, sharp_step) = (biggest_step(&soft), biggest_step(&sharp));
    assert!(sharp_step > 200, "the unblurred edge is not an edge ({sharp_step})");
    assert!(
        soft_step < sharp_step / 3,
        "the blurred edge steps by {soft_step} against {sharp_step} unblurred — \
         it is not blurred"
    );
}

/// A panel shows what is behind *itself*, not the same picture everywhere.
///
/// The difference between glass and wallpaper, and the easiest thing to get
/// wrong: sampling the whole blurred texture into every panel looks plausible
/// on one panel and obviously wrong on two.
fn a_panel_shows_what_is_behind_it() {
    let screen = screen().expect("checked");
    let mut gfx = unsafe { moose_sdl::gfx::Gfx::new(&screen.video) }.expect("a renderer");
    gfx.resize(WIDTH as f32, HEIGHT as f32);
    let mut glass = unsafe { moose_sdl::glass::Glass::new(WIDTH, HEIGHT) }.expect("glass");
    unsafe { glass.resize(WIDTH, HEIGHT) }.expect("resized");

    let (bw, bh) = glass.blurred_size();
    unsafe {
        glass.capture(&mut gfx, |g| {
            g.clear(moose_sdl::gfx::Rgba::rgb(255, 0, 0));
            g.rect(bw as f32 / 2.0, 0.0, bw as f32 / 2.0, bh as f32, moose_sdl::gfx::Rgba::rgb(0, 0, 255));
        });
    }

    // Two panels, one over each half, and a tint that lets the blur through.
    gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
    let whole = (WIDTH as f32, HEIGHT as f32);
    let clear = moose_sdl::gfx::Rgba(0.0, 0.0, 0.0, 0.0);
    glass.panel(&gfx, whole, 10.0, 10.0, 60.0, 100.0, clear);
    glass.panel(&gfx, whole, 130.0, 10.0, 60.0, 100.0, clear);
    let frame = read_pixels();
    screen.window.gl_swap_window();

    let left = pixel(&frame, 40, 60);
    let right = pixel(&frame, 160, 60);
    assert!(left.0 > left.2, "the left panel is not showing the red behind it: {left:?}");
    assert!(right.2 > right.0, "the right panel is showing the same as the left: {right:?}");
}

/// Corner rounding belongs to the quad, not to the texture it samples.
///
/// The two are the same thing until something draws a *piece* of a texture,
/// which is exactly what a frosted panel does: it takes the part of the
/// blurred backdrop that sits behind it. The shader was measuring the corner
/// from the texture coordinate, so every glass panel had its alpha cut along a
/// diagonal somewhere in the middle of itself — black wedges at the bottom of
/// every console tile, which is what Frank saw and reported.
fn rounded_corners_follow_the_quad_not_the_texture() {
    let mut frame = Vec::new();
    if !with_gfx(|gfx| {
        gfx.clear(moose_sdl::gfx::Rgba::rgb(0, 0, 0));
        // A solid white picture, so anything missing is the corner cut and not
        // the picture.
        let texture = gfx.upload_rgba(2, 2, &[255u8; 16]);
        // The bottom-right quarter of it, which is the sort of window a panel
        // near the corner of the screen asks for.
        gfx.rounded(8.0, || {
            gfx.image_part(
                &texture,
                40.0,
                20.0,
                120.0,
                80.0,
                (0.5, 0.5, 1.0, 1.0),
                moose_sdl::gfx::Rgba::WHITE,
            )
        });
        frame = read_pixels();
    }) {
        return;
    }
    // Every edge's midpoint is inside a rounded rectangle, whatever the
    // radius. A cut across the middle takes at least one of them.
    for (x, y, edge) in
        [(100, 22, "top"), (100, 97, "bottom"), (42, 60, "left"), (157, 60, "right")]
    {
        assert_eq!(
            pixel(&frame, x, y),
            (255, 255, 255),
            "the {edge} edge was cut: the corner rounding is measuring the texture \
             instead of the quad"
        );
    }
    // And the corners themselves are still cut, or this passes with rounding
    // switched off entirely.
    assert_eq!(pixel(&frame, 41, 21), (0, 0, 0), "the top-left corner was not rounded");
}

//! The screen's geometry, asked of macOS directly.
//!
//! The window manager, the frontend and the emulator have to agree on one
//! coordinate space, and going through the toolkit did not get us there. Tauri
//! reports a monitor as a pixel size plus a scale factor, which is the right
//! answer only when the display is running at its native resolution. On a
//! MacBook set to a scaled mode — which is the default on these machines — the
//! panel is 3024x1964, the backing scale factor is still 2, and the desktop is
//! actually 1800x1169 points. Dividing one by the other gives 1512x982: a
//! number that is wrong by a third, in a way that is invisible until a window
//! lands somewhere strange.
//!
//! `CGDisplayBounds` reports points, in the same top-left-origin space the
//! window server uses, with no scale factor involved. That is what Cocoa lays
//! windows out in, and therefore what RetroArch's window position means.

/// A display's bounds in points, top-left origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One attached display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    pub bounds: Bounds,
    /// The laptop's own panel. Everything else is something plugged in.
    pub builtin: bool,
    /// Holds the menu bar, and is the origin of the coordinate space.
    pub main: bool,
}

impl Display {
    /// What to call it in a menu: the kind, and the size, which is how people
    /// tell two external monitors apart.
    pub fn label(&self) -> String {
        format!(
            "{} — {}x{}",
            if self.builtin { "Built-in" } else { "External" },
            self.bounds.width as u32,
            self.bounds.height as u32,
        )
    }
}

/// Which display a game should open on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// An external if one is attached, otherwise the built-in.
    ///
    /// The default, because an external display is a deliberate act: nobody
    /// plugs a monitor into a laptop and then wants the game on the laptop
    /// screen. It is also usually the bigger and the faster of the two.
    PreferExternal,
    /// Whichever holds the menu bar.
    Main,
    /// A particular one, by position in the list.
    Index(usize),
}

impl Choice {
    pub fn parse(s: &str) -> Self {
        match s {
            "main" => Self::Main,
            "auto" | "external" => Self::PreferExternal,
            other => other.parse().ok().map(Self::Index).unwrap_or(Self::PreferExternal),
        }
    }

    pub fn key(self) -> String {
        match self {
            Self::PreferExternal => "auto".to_owned(),
            Self::Main => "main".to_owned(),
            Self::Index(i) => i.to_string(),
        }
    }
}

/// Pick a display out of `all` according to `choice`.
///
/// Never returns nothing when there is anything attached: a preference naming a
/// screen that has since been unplugged falls back rather than leaving the game
/// with nowhere to open, which would be a black window on a machine whose only
/// fault is that a cable came out.
pub fn choose(all: &[Display], choice: Choice) -> Option<Display> {
    if all.is_empty() {
        return None;
    }
    let main = || all.iter().find(|d| d.main).copied().or_else(|| all.first().copied());
    match choice {
        Choice::Main => main(),
        Choice::Index(i) => all.get(i).copied().or_else(main),
        Choice::PreferExternal => all
            .iter()
            .find(|d| !d.builtin)
            .copied()
            .or_else(main),
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Bounds;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    // SAFETY contract for both: they take a display id and return by value,
    // touch no memory we own, and are safe to call from any thread.
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayIsBuiltin(display: u32) -> i32;
        fn CGDisplayIsMain(display: u32) -> i32;
    }

    /// Every attached display, main one first.
    ///
    /// Ordered so that a stored index means the same thing across a session,
    /// and so "the first external" is a stable idea rather than whatever order
    /// the window server happens to hand back.
    pub fn displays() -> Vec<super::Display> {
        const MAX: u32 = 16;
        let mut ids = [0u32; MAX as usize];
        let mut count = 0u32;
        // SAFETY: `ids` has room for MAX entries, which is what is promised,
        // and `count` receives how many were written.
        let err = unsafe { CGGetActiveDisplayList(MAX, ids.as_mut_ptr(), &mut count) };
        if err != 0 {
            return Vec::new();
        }
        let mut out: Vec<super::Display> = ids[..count as usize]
            .iter()
            .filter_map(|&id| {
                // SAFETY: as above; these take a display id by value.
                let r = unsafe { CGDisplayBounds(id) };
                if r.size.width <= 0.0 || r.size.height <= 0.0 {
                    return None;
                }
                Some(super::Display {
                    bounds: Bounds {
                        x: r.origin.x,
                        y: r.origin.y,
                        width: r.size.width,
                        height: r.size.height,
                    },
                    builtin: unsafe { CGDisplayIsBuiltin(id) } != 0,
                    main: unsafe { CGDisplayIsMain(id) } != 0,
                })
            })
            .collect();
        out.sort_by_key(|d| !d.main);
        out
    }

    /// The display holding the menu bar.
    ///
    /// Deliberately the main display rather than "whichever screen the library
    /// window is on". Picking the second one means agreeing with the window
    /// server about which screen that is *and* about where it starts, and
    /// getting either wrong puts the game window off the side of the desktop —
    /// which is what kept happening. The main display is the one every part of
    /// the system already agrees about.
    pub fn main_display() -> Option<Bounds> {
        // SAFETY: see the extern block. No arguments are pointers.
        let r = unsafe { CGDisplayBounds(CGMainDisplayID()) };
        if r.size.width <= 0.0 || r.size.height <= 0.0 {
            return None;
        }
        Some(Bounds {
            x: r.origin.x,
            y: r.origin.y,
            width: r.size.width,
            height: r.size.height,
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Bounds;
    pub fn main_display() -> Option<Bounds> {
        None
    }
    pub fn displays() -> Vec<super::Display> {
        Vec::new()
    }
}

pub use imp::{displays, main_display};

#[cfg(test)]
mod tests {
    use super::*;

    /// Points, not pixels. A Retina panel reports thousands of pixels across
    /// and a desktop of about two thousand points; a four-figure width in the
    /// five thousands would mean we are reading the framebuffer instead.
    #[test]
    #[cfg(target_os = "macos")]
    fn the_main_display_is_measured_in_points() {
        let b = main_display().expect("macOS always has a main display");
        assert!(b.width > 640.0 && b.width < 4000.0, "{b:?} looks like pixels");
        assert!(b.height > 480.0 && b.height < 3000.0, "{b:?} looks like pixels");
        // The main display is the origin of the coordinate space by definition.
        assert_eq!((b.x, b.y), (0.0, 0.0));
    }

    fn screen(builtin: bool, main: bool, w: f64) -> Display {
        Display {
            bounds: Bounds { x: 0.0, y: 0.0, width: w, height: 1000.0 },
            builtin,
            main,
        }
    }

    /// Plugging a monitor into a laptop is a deliberate act, and nobody does it
    /// wanting the game on the laptop screen.
    #[test]
    fn an_external_display_wins_by_default() {
        let all = [screen(true, true, 1800.0), screen(false, false, 2560.0)];
        assert_eq!(choose(&all, Choice::PreferExternal).unwrap().bounds.width, 2560.0);
        // With nothing plugged in there is only one answer, and it is not none.
        let solo = [screen(true, true, 1800.0)];
        assert_eq!(choose(&solo, Choice::PreferExternal).unwrap().bounds.width, 1800.0);
    }

    #[test]
    fn asking_for_the_main_display_gets_it_even_with_an_external_attached() {
        let all = [screen(true, true, 1800.0), screen(false, false, 2560.0)];
        assert_eq!(choose(&all, Choice::Main).unwrap().bounds.width, 1800.0);
    }

    /// A cable coming out should not leave a game with nowhere to open. The
    /// stored preference names a screen by position, and positions move.
    #[test]
    fn a_choice_naming_a_display_that_is_gone_falls_back() {
        let all = [screen(true, true, 1800.0)];
        assert_eq!(choose(&all, Choice::Index(3)).unwrap().bounds.width, 1800.0);
        assert!(choose(&[], Choice::Index(0)).is_none());
        assert!(choose(&[], Choice::PreferExternal).is_none());
    }

    /// The setting round-trips through config.toml as text, and anything
    /// unrecognized means the default rather than a panic or a blank screen.
    #[test]
    fn the_setting_survives_being_written_and_read_back() {
        for c in [Choice::PreferExternal, Choice::Main, Choice::Index(2)] {
            assert_eq!(Choice::parse(&c.key()), c);
        }
        assert_eq!(Choice::parse("nonsense"), Choice::PreferExternal);
        assert_eq!(Choice::parse(""), Choice::PreferExternal);
        // "external" was the name before "auto"; it means the same thing.
        assert_eq!(Choice::parse("external"), Choice::PreferExternal);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn there_is_nothing_to_ask_off_macos() {
        assert!(main_display().is_none());
    }
}

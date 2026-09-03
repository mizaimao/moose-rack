//! What shape each console's picture is.
//!
//! A window that does not match gets black bars, and on a maximised window
//! those bars are large — a Game Boy Advance is 3:2 and a modern laptop screen
//! is nearer 16:10, so a full-height window leaves a black column down each
//! side that is wider than the game is tall on the original hardware.
//!
//! RetroArch cannot fix that from inside: it is told to keep the picture's
//! shape, so all it can do is put bars in the space it was given. The fix is to
//! give it less space, of the right shape.
//!
//! These are the *display* aspects, not the pixel counts. A SNES frame is
//! 256x224, which is 8:7, and it was meant to be seen on a 4:3 television —
//! using the pixel count would make every game on the system slightly too tall.
//! Handhelds are the other way round: their pixels really are square, because
//! the panel was part of the machine.

/// A platform's shape, and whether every game on it really has that shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// Width divided by height.
    pub ratio: f32,
    /// True when the whole platform shares this shape, so the picture can be
    /// drawn at it as well as the window being sized to it.
    ///
    /// False for arcade. A cabinet was a 4:3 television, so a 4:3 window is a
    /// far better fit than a window the width of the screen — but a rotated
    /// vertical shooter is 3:4 in the same cabinet, and forcing it to 4:3
    /// would not letterbox it, it would stretch it. Those games keep their own
    /// shape inside the window and are pillarboxed, which is what they were on
    /// a horizontal screen anyway.
    pub exact: bool,
}

/// The shape of `platform`'s picture, as width divided by height.
///
/// `None` for anything whose games disagree with each other. Arcade is the
/// clear case — a vertical shooter and a driving cabinet are both "arcade" and
/// they are not the same shape — and guessing there would rotate half the
/// library into a letterbox.
pub fn of(platform: &str) -> Option<Shape> {
    let exact = !matches!(platform, "arcade" | "mame" | "fbneo");
    let ratio = match platform {
        // Handhelds: square pixels, so the panel's own resolution is the shape.
        "gb" | "gbc" | "gamegear" => 160.0 / 144.0,
        "gba" => 240.0 / 160.0,
        "neo-geo-pocket" | "ngp" => 160.0 / 152.0,
        "wonderswan" | "wonderswancolor" => 224.0 / 144.0,
        "psp" => 480.0 / 272.0,
        // Two screens, one above the other, and the window has to hold both.
        "nds" => 256.0 / 384.0,

        // Televisions. All 4:3 whatever the frame buffer says.
        "nes" | "famicom" | "snes" | "sfc" | "megadrive" | "mastersystem" | "pcengine"
        | "n64" | "psx" | "saturn" | "dc" | "ngc" | "3do" | "neogeoaes" | "neogeo" => 4.0 / 3.0,

        // Arcade cabinets of this era were 4:3 televisions turned into
        // furniture, and the Neo Geo is the same hardware in a cabinet or a
        // console. A vertical shooter in a rotated cabinet is the exception,
        // and it was pillarboxed on any horizontal screen too — there is no
        // way to know which from the platform alone, and 4:3 is right for the
        // large majority.
        "arcade" | "mame" | "fbneo" | "neogeocd" => 4.0 / 3.0,

        // Anything not listed: no single answer.
        _ => return None,
    };
    Some(Shape { ratio, exact })
}

/// Fit the largest `aspect`-shaped box inside `width` x `height`.
///
/// Whichever dimension has to give, gives. Nothing is ever scaled up past the
/// space available, so the result always fits.
pub fn fit(width: u32, height: u32, aspect: f32) -> (u32, u32) {
    // `is_finite` as well as positive: an aspect arriving as NaN would make
    // every comparison below false and hand back a zero-sized window.
    if width == 0 || height == 0 || aspect <= 0.0 || !aspect.is_finite() {
        return (width, height);
    }
    let by_width = (width as f32 / aspect) as u32;
    if by_width <= height {
        (width, by_width)
    } else {
        ((height as f32 * aspect) as u32, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: a window shaped like the game has nothing left over to
    /// put a bar in.
    #[test]
    fn a_fitted_window_has_the_games_shape() {
        let (w, h) = fit(1692, 1048, of("gba").unwrap().ratio);
        // 3:2, and inside the space it was given.
        assert!((w as f32 / h as f32 - 1.5).abs() < 0.01, "{w}x{h}");
        assert!(w <= 1692 && h <= 1048);
        // Height was the constraint here, so the width comes in and the height
        // is used in full.
        assert_eq!(h, 1048);
    }

    #[test]
    fn whichever_dimension_has_to_give_gives() {
        // A wide screen and a 4:3 game: the width comes in.
        let (w, h) = fit(2560, 1440, 4.0 / 3.0);
        assert_eq!((w, h), (1920, 1440));
        // A tall window and a wide game: the height comes in.
        let (w, h) = fit(1000, 1000, 2.0);
        assert_eq!((w, h), (1000, 500));
    }

    /// Televisions were 4:3 whatever the frame buffer said. Using the pixel
    /// count instead would make every SNES game slightly too tall — 8:7 rather
    /// than 4:3 — which is wrong in a way that looks almost right.
    #[test]
    fn console_games_are_shaped_for_the_television_not_the_framebuffer() {
        for p in ["snes", "nes", "megadrive", "psx", "n64"] {
            assert!((of(p).unwrap().ratio - 4.0 / 3.0).abs() < 0.001, "{p}");
        }
    }

    #[test]
    fn handhelds_keep_their_own_panels_shape() {
        assert!((of("gba").unwrap().ratio - 1.5).abs() < 0.001);
        assert!((of("gb").unwrap().ratio - 160.0 / 144.0).abs() < 0.001);
        // Two screens stacked, so taller than it is wide.
        assert!(of("nds").unwrap().ratio < 1.0);
    }

    /// Cabinets were 4:3 televisions in furniture. A rotated vertical shooter
    /// is the exception and was pillarboxed on any horizontal screen anyway —
    /// there is nothing in the platform to tell the two apart, and leaving
    /// arcade unshaped meant a window as wide as the screen with a black
    /// column down each side of every game in it.
    /// A rotated shooter in the same cabinet is 3:4, and forcing it to 4:3
    /// would stretch it rather than letterbox it. So arcade gets the window
    /// shape but not the drawing ratio.
    #[test]
    fn arcade_is_shaped_but_its_games_are_not_all_that_shape() {
        assert!(!of("arcade").unwrap().exact);
        assert!(!of("fbneo").unwrap().exact);
        // A console's games really are all the one shape.
        assert!(of("snes").unwrap().exact);
        assert!(of("neogeoaes").unwrap().exact);
    }

    #[test]
    fn arcade_cabinets_are_four_by_three() {
        for p in ["arcade", "mame", "fbneo", "neogeoaes", "neogeocd"] {
            assert!((of(p).unwrap().ratio - 4.0 / 3.0).abs() < 0.001, "{p}");
        }
    }

    #[test]
    fn a_platform_nobody_has_mapped_has_no_answer() {
        assert_eq!(of("some-new-console"), None);
        assert_eq!(of(""), None);
    }

    #[test]
    fn nothing_is_scaled_up_or_divided_by_zero() {
        assert_eq!(fit(0, 100, 1.5), (0, 100));
        assert_eq!(fit(100, 0, 1.5), (100, 0));
        assert_eq!(fit(100, 100, 0.0), (100, 100));
        let (w, h) = fit(800, 600, 4.0 / 3.0);
        assert!(w <= 800 && h <= 600);
    }
}

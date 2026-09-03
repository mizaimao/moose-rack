// How big things are drawn, and how many columns the window has room for.
//
// One front end has to serve a 4" 960x720 handheld and a 27" desktop, and a
// layout written in pixels cannot do both: the same 150 pixels is a comfortable
// cover on one and a postage stamp on the other. So nothing is written in
// pixels. A card is 150 *points*, and a single scale turns points into pixels
// for whatever it is being drawn on.
//
// Pixels are a measurement here, never a design unit. The moment a number in a
// view means pixels, that view has picked a screen.

use serde::{Deserialize, Serialize};

/// Pixels per point.
///
/// 96 dots per inch is one pixel per point — the baseline every desktop
/// toolkit uses, and what a 1x display has always been.
pub const BASELINE_DPI: f32 = 96.0;

/// Sensible bounds. Below the first, text stops being legible; above the
/// second, something has reported nonsense — a display that claims 2,000 DPI
/// is a driver bug, not a screen, and it should not blow the layout up.
const MIN_SCALE: f32 = 0.5;
const MAX_SCALE: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Scale(f32);

impl Default for Scale {
    fn default() -> Self {
        Scale(1.0)
    }
}

impl Scale {
    /// One point is one pixel.
    pub const NONE: Scale = Scale(1.0);

    /// Whatever was asked for, clamped and snapped.
    ///
    /// Snapped to quarter steps because a scale of 1.37 puts every border and
    /// every glyph on a fraction of a pixel, which reads as a soft, slightly
    /// wrong picture rather than as a bug anybody reports.
    pub fn new(factor: f32) -> Self {
        if !factor.is_finite() {
            return Scale(1.0);
        }
        Scale((factor * 4.0).round() / 4.0).clamped()
    }

    fn clamped(self) -> Self {
        Scale(self.0.clamp(MIN_SCALE, MAX_SCALE))
    }

    /// From what the display says about itself.
    ///
    /// A starting point, not an answer. DPI describes the panel and not how
    /// far away somebody's face is, and that is most of what decides how big a
    /// thing should be: a handheld is held at arm's length or closer, so the
    /// same physical size that is comfortable on a monitor is wastefully large
    /// on it. Hence [`Scale::viewed_from`], and hence the config override — a
    /// handheld should state its scale rather than infer one.
    pub fn from_dpi(dpi: f32) -> Self {
        Scale::new(dpi / BASELINE_DPI)
    }

    /// From the panel and how far away it is held, in the same unit.
    ///
    /// A 27" monitor at 60cm and a 4" handheld at 30cm want the same *angular*
    /// size, which is half the physical size on the closer one. This is that
    /// correction, and it is why the handheld does not simply get its DPI
    /// ratio: a 4" 960x720 panel is around 300 DPI, which would be a scale of
    /// three and a screen with room for two cards on it.
    pub fn viewed_from(dpi: f32, distance_cm: f32) -> Self {
        const REFERENCE_CM: f32 = 60.0;
        // Zero, negative, or NaN — none of which is a distance. `<= 0.0` is
        // false for NaN, so it is spelled out rather than negated.
        if distance_cm <= 0.0 || distance_cm.is_nan() {
            return Scale::from_dpi(dpi);
        }
        Scale::new((dpi / BASELINE_DPI) * (distance_cm / REFERENCE_CM))
    }

    pub fn factor(self) -> f32 {
        self.0
    }

    /// Points to pixels, for handing a number to a renderer.
    pub fn px(self, points: f32) -> f32 {
        points * self.0
    }

    /// Pixels to points, for taking a number back off one — a window size, a
    /// mouse position, a measured glyph.
    pub fn pt(self, pixels: f32) -> f32 {
        pixels / self.0
    }
}

/// The drawable area, in the units a layout is written in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width_px: f32,
    pub height_px: f32,
    pub scale: Scale,
}

impl Viewport {
    pub fn new(width_px: f32, height_px: f32, scale: Scale) -> Self {
        Viewport { width_px, height_px, scale }
    }

    pub fn width(self) -> f32 {
        self.scale.pt(self.width_px)
    }

    pub fn height(self) -> f32 {
        self.scale.pt(self.height_px)
    }

    /// Wider than it is tall, and by how much. 4:3 is 1.33, 16:9 is 1.78.
    pub fn aspect(self) -> f32 {
        if self.height_px > 0.0 { self.width_px / self.height_px } else { 1.0 }
    }

    pub fn panes(self) -> Panes {
        Panes::fitting(self.width())
    }
}

/// How many columns there is room for.
///
/// Room for, not how many are shown: which arrangement to use is a preference,
/// and this is the ceiling it lives under. A window that cannot hold three
/// columns must not be given three however the preference reads, and a
/// handheld never can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Panes {
    /// The list you pick from, or the games, or the preview — one at a time,
    /// with a back button. Every handheld, and a narrow window.
    One,
    /// The picker beside the games.
    Two,
    /// Picker, games, preview.
    Three,
}

impl Panes {
    /// Measured off the stylesheet the webview uses, so the two agree about
    /// when a window is wide enough: the picker column is 260 points and the
    /// preview 320, and the middle needs at least two 150-point covers with
    /// 14-point gaps between and around them — about 340. That is 920 for
    /// three, and 600 for two. Rounded up, because a layout that only just
    /// fits is one nobody enjoys.
    pub const TWO_AT: f32 = 660.0;
    pub const THREE_AT: f32 = 1000.0;

    pub fn fitting(width_points: f32) -> Panes {
        if width_points >= Self::THREE_AT {
            Panes::Three
        } else if width_points >= Self::TWO_AT {
            Panes::Two
        } else {
            Panes::One
        }
    }

    pub fn count(self) -> usize {
        match self {
            Panes::One => 1,
            Panes::Two => 2,
            Panes::Three => 3,
        }
    }

    /// The most this window can give, and no more than was asked for.
    pub fn at_most(self, wanted: Panes) -> Panes {
        self.min(wanted)
    }
}

/// A box, in points, measured from the top left.
///
/// Everything a view places is one of these, and nothing computes a position
/// by adding gaps to offsets — that is what this file exists to stop. A
/// hand-placed interface is one where every new element costs ten lines of
/// arithmetic and every change to a margin costs twenty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Space around the inside of a box: top, right, bottom, left, as CSS names
/// them and in the order CSS names them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const fn all(n: f32) -> Self {
        Edges { top: n, right: n, bottom: n, left: n }
    }
    pub const fn xy(x: f32, y: f32) -> Self {
        Edges { top: y, right: x, bottom: y, left: x }
    }
}

/// How much of a row or column one child wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    /// Exactly this many points.
    Fixed(f32),
    /// A share of what is left after the fixed ones. Two `Grow(1.0)` split it
    /// evenly; `Grow(2.0)` beside `Grow(1.0)` takes two thirds.
    Grow(f32),
}

/// The grid the column widths in this app are expressed against.
///
/// Twelve because it divides by two, three, four and six, which covers every
/// split a list-and-pane layout wants. Borrowed from the stylesheets on
/// purpose: hand-written widths are how a value column came out 110 points on
/// one screen and 56 on another for no reason anybody could state, and how each
/// of them had to be guessed again when the panel changed size.
pub const COLUMNS: u16 = 12;

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The same box, smaller by `edges` on each side. Never inside out: a
    /// padding wider than the box gives a box of nothing, not a negative one.
    pub fn inset(self, edges: Edges) -> Self {
        Rect {
            x: self.x + edges.left,
            y: self.y + edges.top,
            w: (self.w - edges.left - edges.right).max(0.0),
            h: (self.h - edges.top - edges.bottom).max(0.0),
        }
    }

    /// Take `h` points off the top, and what is left.
    pub fn split_top(self, h: f32) -> (Rect, Rect) {
        let h = h.clamp(0.0, self.h);
        (Rect { h, ..self }, Rect { y: self.y + h, h: self.h - h, ..self })
    }

    pub fn split_bottom(self, h: f32) -> (Rect, Rect) {
        let h = h.clamp(0.0, self.h);
        (Rect { y: self.bottom() - h, h, ..self }, Rect { h: self.h - h, ..self })
    }

    /// Take `w` points off the left, and what is left.
    pub fn split_left(self, w: f32) -> (Rect, Rect) {
        let w = w.clamp(0.0, self.w);
        (Rect { w, ..self }, Rect { x: self.x + w, w: self.w - w, ..self })
    }

    pub fn split_right(self, w: f32) -> (Rect, Rect) {
        let w = w.clamp(0.0, self.w);
        (Rect { x: self.right() - w, w, ..self }, Rect { w: self.w - w, ..self })
    }

    /// A box of this size, centered in this one.
    pub fn center(self, w: f32, h: f32) -> Self {
        Rect { x: self.x + (self.w - w) / 2.0, y: self.y + (self.h - h) / 2.0, w, h }
    }

    /// A box of this shape, as large as fits, centered. What a picture that
    /// must not be distorted goes in.
    pub fn fit(self, aspect: f32) -> Self {
        let aspect = if aspect > 0.0 { aspect } else { 1.0 };
        let (w, h) = if self.w / self.h > aspect {
            (self.h * aspect, self.h)
        } else {
            (self.w, self.w / aspect)
        };
        self.center(w, h)
    }

    /// Lay children left to right, with `gap` between them.
    ///
    /// Fixed children take what they ask for; the rest share what is left in
    /// proportion to their weight. A row that does not fit gives its growing
    /// children nothing rather than negative widths.
    /// Split across a twelve-column grid.
    ///
    /// `spans` are column counts, and what is left over goes to the last one —
    /// so `cols(gap, &[8, 4])` is two thirds and one third whatever the panel
    /// is, rather than a width somebody measured once.
    pub fn cols(self, gap: f32, spans: &[u16]) -> Vec<Rect> {
        let sizes: Vec<Size> = spans.iter().map(|n| Size::Grow(*n as f32)).collect();
        self.row(gap, &sizes)
    }

    pub fn row(self, gap: f32, children: &[Size]) -> Vec<Rect> {
        let widths = share(self.w, gap, children);
        let mut out = Vec::with_capacity(children.len());
        let mut x = self.x;
        for w in widths {
            out.push(Rect { x, w, ..self });
            x += w + gap;
        }
        out
    }

    /// Lay children top to bottom.
    pub fn column(self, gap: f32, children: &[Size]) -> Vec<Rect> {
        let heights = share(self.h, gap, children);
        let mut out = Vec::with_capacity(children.len());
        let mut y = self.y;
        for h in heights {
            out.push(Rect { y, h, ..self });
            y += h + gap;
        }
        out
    }

    /// The twelve-column grid, which is the one piece of layout every page on
    /// the web is built out of and the reason those pages are quick to change.
    ///
    /// `span(from, count)` is the box covering those columns — `span(0, 8)`
    /// beside `span(8, 4)` is the two-thirds/one-third split every layout
    /// starts as.
    pub fn columns(self, gap: f32) -> Columns {
        Columns { rect: self, gap, count: 12 }
    }

    /// The same, with a different number of tracks — for the grids that are
    /// not twelve, like a wall of covers.
    pub fn tracks(self, gap: f32, count: usize) -> Columns {
        Columns { rect: self, gap, count: count.max(1) }
    }

    /// How many tracks of `each` points fit across, with `gap` between.
    ///
    /// What a wall of covers asks: not "how wide is a card" but "how many".
    pub fn fits(self, gap: f32, each: f32) -> usize {
        if each <= 0.0 {
            return 1;
        }
        (((self.w + gap) / (each + gap)).floor() as usize).max(1)
    }
}

/// A grid of equal tracks across a box.
#[derive(Debug, Clone, Copy)]
pub struct Columns {
    rect: Rect,
    gap: f32,
    count: usize,
}

impl Columns {
    pub fn track(self) -> f32 {
        let gaps = self.gap * (self.count - 1) as f32;
        ((self.rect.w - gaps) / self.count as f32).max(0.0)
    }

    /// The box covering `count` tracks starting at `from`.
    pub fn span(self, from: usize, count: usize) -> Rect {
        let track = self.track();
        let from = from.min(self.count);
        let count = count.min(self.count - from).max(1);
        Rect {
            x: self.rect.x + from as f32 * (track + self.gap),
            w: track * count as f32 + self.gap * (count - 1) as f32,
            ..self.rect
        }
    }

    /// Where the `n`th cell of a wrapping grid goes, given a row height.
    ///
    /// For a wall of covers: the cursor's index in, a box out, and no caller
    /// dividing by a column count.
    pub fn cell(self, index: usize, height: f32) -> Rect {
        let (row, col) = (index / self.count, index % self.count);
        let cell = self.span(col, 1);
        Rect { y: self.rect.y + row as f32 * (height + self.gap), h: height, ..cell }
    }
}

/// Hand out one axis between children.
fn share(total: f32, gap: f32, children: &[Size]) -> Vec<f32> {
    if children.is_empty() {
        return Vec::new();
    }
    let gaps = gap * (children.len() - 1) as f32;
    let fixed: f32 = children
        .iter()
        .map(|c| match c {
            Size::Fixed(n) => *n,
            Size::Grow(_) => 0.0,
        })
        .sum();
    let weight: f32 = children
        .iter()
        .map(|c| match c {
            Size::Grow(n) => n.max(0.0),
            Size::Fixed(_) => 0.0,
        })
        .sum();
    let spare = (total - gaps - fixed).max(0.0);
    children
        .iter()
        .map(|c| match c {
            Size::Fixed(n) => n.max(0.0),
            Size::Grow(n) if weight > 0.0 => spare * n.max(0.0) / weight,
            Size::Grow(_) => 0.0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twelve columns, and the split holds whatever the panel is.
    ///
    /// The point of the grid: a hand-written width is right on one screen and
    /// arbitrary on every other, and has to be guessed again each time the
    /// layout moves.
    #[test]
    fn columns_split_by_share_not_by_measurement() {
        let wide = Rect::new(0.0, 0.0, 1200.0, 100.0);
        let cols = wide.cols(0.0, &[8, 4]);
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].w, 800.0);
        assert_eq!(cols[1].w, 400.0);
        assert_eq!(cols[1].x, 800.0, "the second column does not start where the first ends");

        // The same shares on a panel half the size.
        let narrow = Rect::new(0.0, 0.0, 600.0, 100.0);
        let cols = narrow.cols(0.0, &[8, 4]);
        assert_eq!(cols[0].w, 400.0);
        assert_eq!(cols[1].w, 200.0);
    }

    /// The gap comes out of the columns, not out of the rectangle.
    #[test]
    fn the_gap_sits_between_the_columns() {
        let r = Rect::new(0.0, 0.0, 120.0, 10.0);
        let cols = r.cols(20.0, &[6, 6]);
        assert_eq!(cols[0].w, 50.0);
        assert_eq!(cols[1].x, 70.0);
        assert_eq!(cols[1].right(), 120.0, "the split overflowed its own rectangle");
    }

    /// The handheld, at the size the whole exercise is about.
    const POCKET: (f32, f32) = (960.0, 720.0);

    #[test]
    fn points_and_pixels_are_the_same_thing_at_one() {
        let s = Scale::NONE;
        assert_eq!(s.px(150.0), 150.0);
        assert_eq!(s.pt(150.0), 150.0);
    }

    #[test]
    fn points_survive_a_round_trip_through_pixels() {
        for factor in [0.5, 1.0, 1.25, 2.0, 3.0] {
            let s = Scale::new(factor);
            assert!((s.pt(s.px(150.0)) - 150.0).abs() < 0.001, "at {factor}");
        }
    }

    /// A scale of 1.37 puts every border on a fraction of a pixel, which reads
    /// as a soft picture rather than as a bug anybody reports.
    #[test]
    fn a_scale_is_snapped_to_a_quarter() {
        assert_eq!(Scale::new(1.37).factor(), 1.25);
        assert_eq!(Scale::new(1.6).factor(), 1.5);
        assert_eq!(Scale::new(2.0).factor(), 2.0);
    }

    /// A display that claims 2,000 DPI is a driver bug, not a screen.
    #[test]
    fn nonsense_does_not_blow_the_layout_up() {
        assert_eq!(Scale::from_dpi(20_000.0).factor(), MAX_SCALE);
        assert_eq!(Scale::from_dpi(1.0).factor(), MIN_SCALE);
        // Neither of these is a large scale — they are a number that was
        // never computed. Falling back to 1 draws something; clamping to the
        // maximum draws one enormous card and looks deliberate.
        assert_eq!(Scale::new(f32::NAN).factor(), 1.0);
        assert_eq!(Scale::new(f32::INFINITY).factor(), 1.0);
    }

    #[test]
    fn a_plain_desktop_display_is_unscaled() {
        assert_eq!(Scale::from_dpi(96.0), Scale::NONE);
    }

    /// The correction that stops a handheld getting a scale of three.
    ///
    /// A 4" 960x720 panel is about 300 DPI, and at arm's length that would
    /// leave a screen 320 points wide — room for two covers. Held at half the
    /// distance of a monitor, it wants half the physical size.
    #[test]
    fn a_handheld_is_held_closer_and_is_scaled_for_it() {
        let naive = Scale::from_dpi(300.0);
        let held = Scale::viewed_from(300.0, 30.0);
        assert!(held.factor() < naive.factor(), "holding it closer made things bigger");
        assert_eq!(held.factor(), 1.5);

        let screen = Viewport::new(POCKET.0, POCKET.1, held);
        assert_eq!(screen.width(), 640.0);
        assert!(screen.width() > 4.0 * 150.0, "no room for a row of covers");
    }

    /// A monitor at the reference distance is unaffected by the correction.
    #[test]
    fn a_desktop_at_arms_length_is_left_alone() {
        assert_eq!(Scale::viewed_from(96.0, 60.0), Scale::NONE);
        assert_eq!(Scale::viewed_from(220.0, 60.0), Scale::from_dpi(220.0));
    }

    #[test]
    fn a_distance_of_nothing_falls_back_rather_than_dividing_by_zero() {
        assert_eq!(Scale::viewed_from(96.0, 0.0), Scale::from_dpi(96.0));
        assert_eq!(Scale::viewed_from(96.0, -5.0), Scale::from_dpi(96.0));
    }

    /// The one that matters: the handheld is never given three columns.
    #[test]
    fn the_handheld_is_one_pane() {
        let screen = Viewport::new(POCKET.0, POCKET.1, Scale::viewed_from(300.0, 30.0));
        assert_eq!(screen.panes(), Panes::One);
    }

    #[test]
    fn a_desktop_window_earns_its_columns_by_width() {
        let at = |w: f32| Viewport::new(w, 900.0, Scale::NONE).panes();
        assert_eq!(at(500.0), Panes::One);
        assert_eq!(at(800.0), Panes::Two);
        assert_eq!(at(1400.0), Panes::Three);
        // And the boundaries are where they say they are.
        assert_eq!(at(Panes::TWO_AT), Panes::Two);
        assert_eq!(at(Panes::TWO_AT - 1.0), Panes::One);
        assert_eq!(at(Panes::THREE_AT), Panes::Three);
        assert_eq!(at(Panes::THREE_AT - 1.0), Panes::Two);
    }

    /// A retina window is the same *size* as a plain one, in points — which is
    /// the whole reason for the unit. Twice the pixels must not mean twice the
    /// columns.
    #[test]
    fn twice_the_pixels_is_not_twice_the_layout() {
        let plain = Viewport::new(1200.0, 800.0, Scale::NONE);
        let retina = Viewport::new(2400.0, 1600.0, Scale::new(2.0));
        assert_eq!(plain.width(), retina.width());
        assert_eq!(plain.panes(), retina.panes());
        assert!((plain.aspect() - retina.aspect()).abs() < 0.001);
    }

    /// Room for is not the same as shown. A wide window with the preference
    /// set to one pane gets one; a narrow one asking for three still gets one.
    #[test]
    fn the_window_is_a_ceiling_and_the_preference_lives_under_it() {
        assert_eq!(Panes::Three.at_most(Panes::One), Panes::One);
        assert_eq!(Panes::One.at_most(Panes::Three), Panes::One);
        assert_eq!(Panes::Three.at_most(Panes::Three), Panes::Three);
        assert_eq!(Panes::Two.at_most(Panes::Three), Panes::Two);
    }
}

#[cfg(test)]
mod boxes {
    use super::*;

    const PAGE: Rect = Rect::new(0.0, 0.0, 1200.0, 800.0);

    #[test]
    fn a_row_gives_the_fixed_ones_what_they_asked_for() {
        let out = PAGE.row(10.0, &[Size::Fixed(260.0), Size::Grow(1.0), Size::Fixed(320.0)]);
        assert_eq!(out[0].w, 260.0);
        assert_eq!(out[2].w, 320.0);
        // 1200 less two gaps less the two fixed.
        assert_eq!(out[1].w, 1200.0 - 20.0 - 260.0 - 320.0);
        // And they sit end to end with the gap between.
        assert_eq!(out[1].x, out[0].right() + 10.0);
        assert_eq!(out[2].x, out[1].right() + 10.0);
        assert_eq!(out[2].right(), 1200.0);
    }

    #[test]
    fn growing_children_split_what_is_left_by_weight() {
        let out = PAGE.row(0.0, &[Size::Grow(2.0), Size::Grow(1.0)]);
        assert_eq!(out[0].w, 800.0);
        assert_eq!(out[1].w, 400.0);
    }

    /// A window dragged narrower than its own furniture. Negative widths are
    /// how a layout starts drawing things inside out.
    #[test]
    fn a_row_that_does_not_fit_gives_nothing_rather_than_less_than_nothing() {
        let narrow = Rect::new(0.0, 0.0, 100.0, 40.0);
        let out = narrow.row(10.0, &[Size::Fixed(260.0), Size::Grow(1.0)]);
        assert_eq!(out[0].w, 260.0, "a fixed child still asks for its size");
        assert_eq!(out[1].w, 0.0, "the growing one went negative");
    }

    #[test]
    fn a_column_stacks_downwards() {
        let out = PAGE.column(6.0, &[Size::Fixed(42.0), Size::Fixed(38.0), Size::Grow(1.0)]);
        assert_eq!(out[0].y, 0.0);
        assert_eq!(out[1].y, 48.0);
        assert_eq!(out[2].y, 92.0);
        assert_eq!(out[2].bottom(), 800.0);
        // Full width, all of them: a column divides one axis and leaves the
        // other alone.
        assert!(out.iter().all(|r| r.w == 1200.0));
    }

    #[test]
    fn insetting_never_turns_a_box_inside_out() {
        let small = Rect::new(0.0, 0.0, 10.0, 10.0);
        let out = small.inset(Edges::all(40.0));
        assert_eq!((out.w, out.h), (0.0, 0.0));
    }

    #[test]
    fn splitting_takes_from_the_edge_it_says() {
        let (top, rest) = PAGE.split_top(42.0);
        assert_eq!((top.y, top.h), (0.0, 42.0));
        assert_eq!((rest.y, rest.h), (42.0, 758.0));

        let (right, rest) = PAGE.split_right(320.0);
        assert_eq!(right.x, 880.0);
        assert_eq!(rest.w, 880.0);

        let (bottom, rest) = PAGE.split_bottom(30.0);
        assert_eq!(bottom.y, 770.0);
        assert_eq!(rest.h, 770.0);
    }

    /// More than there is. A split has to clamp or the two halves overlap.
    #[test]
    fn splitting_further_than_the_box_goes_clamps() {
        let (top, rest) = PAGE.split_top(9_000.0);
        assert_eq!(top.h, 800.0);
        assert_eq!(rest.h, 0.0);
    }

    /// The two-thirds/one-third split every page starts as.
    #[test]
    fn twelve_columns_span_the_way_the_web_does() {
        let grid = PAGE.columns(10.0);
        let main = grid.span(0, 8);
        let side = grid.span(8, 4);
        assert!((main.w - (grid.track() * 8.0 + 70.0)).abs() < 0.01);
        assert_eq!(side.right(), 1200.0);
        assert!(side.x > main.right(), "the two spans overlap");
        assert!((side.x - main.right() - 10.0).abs() < 0.01, "the gap between them is wrong");
    }

    #[test]
    fn a_span_cannot_run_off_the_grid() {
        let grid = PAGE.columns(10.0);
        assert_eq!(grid.span(10, 9).right(), 1200.0);
        assert!(grid.span(0, 99).w <= 1200.0);
        assert!(grid.span(99, 1).w > 0.0, "a span past the end vanished");
    }

    /// What a wall of covers asks: how many fit, then where the nth goes.
    #[test]
    fn a_wrapping_grid_places_cells_by_index() {
        let area = Rect::new(100.0, 50.0, 700.0, 600.0);
        let across = area.fits(14.0, 150.0);
        assert_eq!(across, 4);
        let grid = area.tracks(14.0, across);
        let first = grid.cell(0, 200.0);
        assert_eq!((first.x, first.y), (100.0, 50.0));
        // The fifth wraps to the second row.
        let fifth = grid.cell(4, 200.0);
        assert_eq!(fifth.x, 100.0);
        assert_eq!(fifth.y, 50.0 + 214.0);
        // And the last of a row ends flush with the area.
        assert!((grid.cell(3, 200.0).right() - 800.0).abs() < 0.01);
    }

    /// A picture that must not be distorted, in a box that is not its shape.
    #[test]
    fn fitting_keeps_the_shape_and_centers_what_is_left() {
        // A tall box art in a wide box: letterboxed left and right.
        let box_ = Rect::new(0.0, 0.0, 200.0, 100.0);
        let art = box_.fit(0.75);
        assert_eq!(art.h, 100.0);
        assert!((art.w - 75.0).abs() < 0.01);
        assert!((art.x - 62.5).abs() < 0.01, "not centered");

        // And the other way round.
        let wide = Rect::new(0.0, 0.0, 100.0, 200.0).fit(1.37);
        assert_eq!(wide.w, 100.0);
        assert!(wide.y > 0.0);
    }

    #[test]
    fn a_point_is_inside_a_box_or_it_is_not() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(10.0, 20.0), "the top left corner is inside");
        assert!(!r.contains(110.0, 40.0), "the right edge is outside");
        assert!(!r.contains(9.0, 40.0));
        assert!(r.contains(109.0, 69.0));
    }

    /// The whole point: a page laid out in one expression, and every piece of
    /// it in the right place without a single addition in the caller.
    #[test]
    fn a_whole_page_lays_itself_out() {
        let [tabs, header, body] = <[Rect; 3]>::try_from(
            PAGE.column(0.0, &[Size::Fixed(42.0), Size::Fixed(38.0), Size::Grow(1.0)]),
        )
        .unwrap();
        let [picker, games, aside] = <[Rect; 3]>::try_from(
            body.row(14.0, &[Size::Fixed(260.0), Size::Grow(1.0), Size::Fixed(320.0)]),
        )
        .unwrap();

        assert_eq!(tabs.h, 42.0);
        assert_eq!(header.y, 42.0);
        assert_eq!(body.y, 80.0);
        assert_eq!(picker.x, 0.0);
        assert_eq!(aside.right(), 1200.0);
        assert!(games.w > 500.0);
        // Nothing overlaps and nothing is left over.
        assert_eq!(games.x, picker.right() + 14.0);
        assert_eq!(aside.x, games.right() + 14.0);
    }
}

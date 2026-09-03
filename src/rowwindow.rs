// Which rows of a long list to draw, and how much space to leave for the rest.
//
// The arcade console is 2,506 games. Drawing all of them is the thing
// `docs/parked.md` called "the only thing in the app that is slow rather than
// missing", and on a 1 GB handheld it is not slow so much as fatal. So a band
// around the viewport is drawn and two spacers stand in for everything else,
// each exactly the height of the rows it replaces — the scrollbar and every
// remembered position stay what they would have been with the whole list
// there.
//
// This is `ui/js/visible.js` ported, as `docs/handheld-frontend.md` task 3
// says to do early: a 1 GB handheld needs it more than a Mac does, and the
// webview's copy has fourteen tests behind it that are worth keeping rather
// than rediscovering against a 4" screen.
//
// It works because a grid of covers is uniform — every column the same width,
// every card the same shape — which is also what lets `crate::gridnav::uniform`
// move the cursor through rows that were never drawn.

/// Below this many rows, draw the lot.
///
/// Well above a screenful at any size, so nothing anybody can see at once is
/// ever windowed. The point is the two-thousand-row case.
pub const THRESHOLD: usize = 400;

/// How far beyond the viewport to keep drawn, as a multiple of its height, in
/// each direction.
///
/// Not a tuning knob so much as a guarantee: a whole screen of overscan means
/// the cursor's next stop is already drawn, whichever way it goes and however
/// fast a held key repeats. Paging moves three rows, well inside it.
pub const OVERSCAN: f32 = 1.5;

/// Which rows to draw, and how much empty space to leave either side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// The first item to draw. Always the start of a row.
    pub first: usize,
    pub count: usize,
    /// Height of the space above and below, in the same unit as `row_height`.
    pub before: f32,
    pub after: f32,
    /// How many rows the whole list comes to.
    pub rows: usize,
}

/// What the window is being asked about.
#[derive(Debug, Clone, Copy)]
pub struct Ask {
    pub total: usize,
    pub columns: usize,
    pub row_height: f32,
    /// How far the list's first row has been scrolled past. Negative while the
    /// list is still below the top of the viewport.
    pub top: f32,
    pub viewport: f32,
    pub overscan: f32,
}

impl Ask {
    pub fn new(total: usize, columns: usize, row_height: f32, top: f32, viewport: f32) -> Self {
        Ask { total, columns, row_height, top, viewport, overscan: OVERSCAN }
    }
}

/// Whether a list this long is worth windowing at all.
pub fn worth_it(total: usize) -> bool {
    total > THRESHOLD
}

/// The band to draw.
///
/// Everything is in whole rows, so the band always starts on a row boundary
/// and the spacers are always an exact number of rows. Half a row of error is
/// a grid that jumps by half a row as it is scrolled.
pub fn band(ask: Ask) -> Band {
    let columns = ask.columns.max(1);
    let rows = ask.total.div_ceil(columns);
    // A height of zero, or of NaN — both mean nothing has been measured yet,
    // and `<= 0.0` is false for NaN, so it is spelled out.
    if ask.total == 0 || ask.row_height <= 0.0 || ask.row_height.is_nan() {
        return Band { first: 0, count: ask.total, before: 0.0, after: 0.0, rows };
    }

    let margin = ask.viewport * ask.overscan;
    let first_row = (((ask.top - margin) / ask.row_height).floor() as isize)
        .clamp(0, rows.saturating_sub(1) as isize) as usize;
    let last_row = (((ask.top + ask.viewport + margin) / ask.row_height).ceil() as isize)
        .clamp(first_row as isize + 1, rows as isize) as usize;

    let first = first_row * columns;
    Band {
        first,
        count: (ask.total - first).min((last_row - first_row) * columns),
        before: first_row as f32 * ask.row_height,
        after: (rows - last_row) as f32 * ask.row_height,
        rows,
    }
}

/// Where to scroll so that `index` is on screen, given where it is now.
///
/// Returns `None` when it already is, because scrolling to somewhere you are
/// already looking moves the list under the reader for no reason.
pub fn scroll_to(index: usize, ask: Ask) -> Option<f32> {
    // A height of zero, or of NaN — both mean nothing has been measured yet,
    // and `<= 0.0` is false for NaN, so it is spelled out.
    if ask.total == 0 || ask.row_height <= 0.0 || ask.row_height.is_nan() {
        return None;
    }
    let row = (index / ask.columns.max(1)) as f32;
    let top = row * ask.row_height;
    let bottom = top + ask.row_height;
    if top < ask.top {
        Some(top)
    } else if bottom > ask.top + ask.viewport {
        Some(bottom - ask.viewport)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2,506 arcade games, ten across, 200 units a row, in an 800 unit window.
    fn arcade(top: f32) -> Band {
        band(Ask::new(2506, 10, 200.0, top, 800.0))
    }

    #[test]
    fn at_the_top_it_starts_at_the_first_row() {
        let b = arcade(0.0);
        assert_eq!(b.first, 0);
        assert_eq!(b.before, 0.0, "space was left above the first row");
        assert!(b.count > 0);
    }

    /// The whole point: a fraction of the list, not the list.
    #[test]
    fn it_is_a_fraction_of_the_list() {
        let b = arcade(20_000.0);
        assert!(b.count < 2506 / 4, "drew {} of 2,506", b.count);
        assert!(b.count > 10, "drew only {}", b.count);
    }

    /// Overscan is a guarantee rather than a knob: the cursor's next stop has
    /// to be drawn already, whichever way it goes.
    #[test]
    fn a_whole_screen_is_drawn_beyond_the_viewport_each_way() {
        let b = arcade(20_000.0);
        assert!(b.before <= 20_000.0 - 800.0, "only {} above", 20_000.0 - b.before);
        let bottom = b.before + (b.count.div_ceil(10)) as f32 * 200.0;
        assert!(bottom >= 20_000.0 + 1600.0, "only {} below", bottom - 20_800.0);
    }

    #[test]
    fn at_the_end_it_stops_at_the_last_row() {
        let rows = 2506_usize.div_ceil(10);
        let b = arcade(rows as f32 * 200.0);
        assert_eq!(b.after, 0.0, "space was left below the last row");
        assert_eq!(b.first + b.count, 2506, "the last row was not drawn");
    }

    /// A last row that is not full is still a row. 2,506 over ten leaves six.
    #[test]
    fn a_short_last_row_is_drawn_whole() {
        let b = band(Ask::new(2506, 10, 200.0, 50_000.0, 800.0));
        assert_eq!(b.first + b.count, 2506);
    }

    /// The scrollbar, and every remembered position, must be what they would
    /// have been with every row drawn.
    #[test]
    fn the_page_is_the_height_it_would_have_been() {
        let total = 2506_usize.div_ceil(10) as f32 * 200.0;
        for top in [0.0, 137.0, 4_000.0, 20_000.0, 42_000.0, 60_000.0] {
            let b = arcade(top);
            let drawn = b.count.div_ceil(10) as f32 * 200.0;
            assert_eq!(b.before + drawn + b.after, total, "wrong height at {top}");
        }
    }

    /// Half a row of error is a grid that jumps as it is scrolled past.
    #[test]
    fn the_spacers_are_always_whole_rows() {
        for top in [0.0, 137.0, 4_321.0, 20_000.0, 59_999.0] {
            let b = arcade(top);
            assert_eq!(b.before % 200.0, 0.0, "before is not whole rows at {top}");
            assert_eq!(b.after % 200.0, 0.0, "after is not whole rows at {top}");
            assert_eq!(b.first % 10, 0, "the band starts mid-row at {top}");
        }
    }

    /// The list sitting below the top of the viewport — something above it, or
    /// the page scrolled up past it.
    #[test]
    fn a_list_not_yet_reached_draws_from_its_first_row() {
        let b = arcade(-500.0);
        assert_eq!(b.first, 0);
        assert_eq!(b.before, 0.0);
    }

    #[test]
    fn an_empty_list_draws_nothing_and_asks_for_no_space() {
        let b = band(Ask::new(0, 10, 200.0, 0.0, 800.0));
        assert_eq!(b.count, 0);
        assert_eq!(b.before, 0.0);
        assert_eq!(b.after, 0.0);
    }

    /// Before anything is drawn there is no height to measure, and a window
    /// that draws nothing then never gets a first card to measure from.
    #[test]
    fn with_no_row_height_yet_everything_is_drawn() {
        assert_eq!(band(Ask::new(900, 10, 0.0, 0.0, 800.0)).count, 900);
    }

    #[test]
    fn one_column_is_a_list_and_behaves() {
        let b = band(Ask::new(2506, 1, 34.0, 10_000.0, 800.0));
        assert!(b.count > 0 && b.count < 2506);
        assert_eq!(b.before % 34.0, 0.0);
    }

    /// Both are reachable: the grid is measured off whatever drew it, and a
    /// container that is not laid out yet reports neither.
    #[test]
    fn nonsense_does_not_divide_by_zero() {
        for columns in [0, 1] {
            let b = band(Ask::new(500, columns, 40.0, 0.0, 800.0));
            assert!(b.count > 0, "columns={columns} drew nothing");
            assert!(b.before.is_finite());
        }
    }

    /// A window over a short list is machinery with nothing to do.
    #[test]
    fn short_lists_are_drawn_whole() {
        assert!(!worth_it(35), "the console list");
        assert!(!worth_it(THRESHOLD));
        assert!(worth_it(2506), "the arcade console");
    }

    /// Scrolling to somewhere you are already looking moves the list under
    /// the reader for no reason.
    #[test]
    fn a_row_already_on_screen_is_left_alone() {
        let ask = Ask::new(2506, 10, 200.0, 2_000.0, 800.0);
        // Rows 10 to 13 are on screen at top=2000.
        assert_eq!(scroll_to(105, ask), None);
        // Above it.
        assert_eq!(scroll_to(5, ask), Some(0.0));
        // Below it.
        assert_eq!(scroll_to(2500, ask), Some(250.0 * 200.0 + 200.0 - 800.0));
    }
}

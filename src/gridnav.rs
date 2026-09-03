// Moving a cursor around a grid of cards.
//
// Derived from where things actually landed, not from a column count. The
// version this replaces measured the first row and then moved by plus or minus
// that number with the result clamped into range, and three things fell out of
// it: Up on the top row clamped to index 0, so you jumped to the first card
// instead of staying put; Down on the last row clamped to the final card the
// same way; and Left/Right ran off the end of a row into the next one. Grouped
// search results made it worse, since each console section can have its own
// card shape and one column count no longer described the page.
//
// Geometry in, index out. A webview measures `offsetTop`/`offsetLeft`; SDL
// knows where it drew each card; a TUI has a character grid. None of them has
// to agree on anything but the arithmetic here.

use serde::Serialize;

/// How many rows a page step moves.
pub const PAGE: i32 = 3;

/// Where one card sits, in whatever units the caller lays out in.
///
/// Positions relative to the layout rather than the viewport, so the map stays
/// valid while scrolling and only needs rebuilding when the list itself
/// changes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Card {
    pub top: f64,
    pub left: f64,
    pub width: f64,
}

/// A few units of jitter is normal between cards of differing height, so cards
/// within this of each other count as the same row.
const ROW_TOLERANCE: f64 = 6.0;

/// Group the cards into rows, top to bottom and left to right within each.
pub fn rows(cards: &[Card]) -> Vec<Vec<usize>> {
    let mut buckets: Vec<(f64, Vec<usize>)> = Vec::new();
    for (i, card) in cards.iter().enumerate() {
        match buckets.iter_mut().find(|(top, _)| (top - card.top).abs() <= ROW_TOLERANCE) {
            Some((_, members)) => members.push(i),
            None => buckets.push((card.top, vec![i])),
        }
    }
    buckets.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, members) in &mut buckets {
        members.sort_by(|&a, &b| cards[a].left.total_cmp(&cards[b].left));
    }
    buckets.into_iter().map(|(_, members)| members).collect()
}

/// Where the cursor is, as a row and a column into [`rows`].
fn locate(grid: &[Vec<usize>], selected: usize) -> Option<(usize, usize)> {
    grid.iter()
        .enumerate()
        .find_map(|(r, row)| row.iter().position(|&i| i == selected).map(|c| (r, c)))
}

/// One step sideways from `(r, c)`.
fn step_x(grid: &[Vec<usize>], (r, c): (usize, usize), step: i32) -> usize {
    let row = &grid[r];
    row[(c as i32 + step).clamp(0, row.len() as i32 - 1) as usize]
}

/// One step up or down from `(r, c)`, or `None` at the edges.
fn step_y(
    cards: &[Card],
    grid: &[Vec<usize>],
    (r, c): (usize, usize),
    step: i32,
) -> Option<usize> {
    let target = r as i32 + step;
    if target < 0 || target >= grid.len() as i32 {
        return None;
    }
    let from = cards[grid[r][c]];
    let want = from.left + from.width / 2.0;
    grid[target as usize]
        .iter()
        .copied()
        .min_by(|&a, &b| {
            let d = |i: usize| (cards[i].left + cards[i].width / 2.0 - want).abs();
            d(a).total_cmp(&d(b))
        })
}

/// Where every card's neighbours are, worked out once for a whole page.
///
/// A table rather than a question per keypress. Front ends that have to ask
/// across a boundary — the webview, over Tauri's IPC — cannot afford a round
/// trip in the middle of a cursor move: a held direction repeats nine times a
/// second, and a cursor that arrives after the hop reads as an app that is
/// thinking about it. The geometry still lives here; only the asking moved.
///
/// Every entry is the index to land on, or `None` for stay put.
#[derive(Debug, Default, Serialize)]
pub struct Moves {
    pub up: Vec<Option<usize>>,
    pub down: Vec<Option<usize>>,
    pub left: Vec<Option<usize>>,
    pub right: Vec<Option<usize>>,
    /// [`PAGE`] rows at a time. Not three single steps: each of those would
    /// re-derive the column from where the last one landed, so a short row on
    /// the way past would drag the cursor sideways.
    pub page_up: Vec<Option<usize>>,
    pub page_down: Vec<Option<usize>>,
    /// The two ends of the page, for the jump-to-first and jump-to-last keys,
    /// and for the first press on a grid with nothing selected yet.
    pub first: Option<usize>,
    pub last: Option<usize>,
}

pub fn moves(cards: &[Card]) -> Moves {
    let grid = rows(cards);
    let mut at = vec![(0usize, 0usize); cards.len()];
    for (r, row) in grid.iter().enumerate() {
        for (c, &i) in row.iter().enumerate() {
            at[i] = (r, c);
        }
    }
    let sideways = |step: i32| -> Vec<Option<usize>> {
        (0..cards.len()).map(|i| Some(step_x(&grid, at[i], step))).collect()
    };
    let vertical = |step: i32| -> Vec<Option<usize>> {
        (0..cards.len()).map(|i| step_y(cards, &grid, at[i], step)).collect()
    };
    Moves {
        up: vertical(-1),
        down: vertical(1),
        left: sideways(-1),
        right: sideways(1),
        page_up: vertical(-PAGE),
        page_down: vertical(PAGE),
        first: edge(cards.len(), false),
        last: edge(cards.len(), true),
    }
}

/// Left or right within the current row.
///
/// Stops at the ends rather than spilling into the neighbouring row, which is
/// what made this feel random. With nothing selected it lands on the first
/// card, so a direction always does something on a freshly drawn grid.
pub fn move_x(cards: &[Card], selected: Option<usize>, step: i32) -> Option<usize> {
    let grid = rows(cards);
    let first = *grid.first()?.first()?;
    let Some(at) = selected.and_then(|s| locate(&grid, s)) else {
        return Some(first);
    };
    Some(step_x(&grid, at, step))
}

/// Up or down a row, keeping the column you were in.
///
/// Matched on horizontal center rather than index, so a short last row, a row
/// of differently-shaped cards, or the next console's section all land
/// somewhere that looks directly above or below where you were.
///
/// Returns `None` at the edges — staying put, rather than clamping to the
/// first or last card. Note that this applies to paging as well as to single
/// steps: PageDown within three rows of the bottom stays where it is rather
/// than jumping to the end. That is the behavior this replaces, kept
/// deliberately; `first`/`last` are the way to reach the ends.
pub fn move_y(cards: &[Card], selected: Option<usize>, step: i32) -> Option<usize> {
    let grid = rows(cards);
    let first = *grid.first()?.first()?;
    let Some(at) = selected.and_then(|s| locate(&grid, s)) else {
        return Some(first);
    };
    step_y(cards, &grid, at, step)
}

/// Where each row leads in a grid that is uniform: every card the same size,
/// laid out left to right and wrapping at `columns`.
///
/// No geometry at all, because none is needed — position is `index / columns`
/// and `index % columns`. That matters for more than tidiness: a windowed list
/// draws a band around the viewport and the cursor still has to move through
/// the rows that are not on the page, which cannot be measured because they do
/// not exist. It also means the front end sends two numbers instead of every
/// card's position.
///
/// The answers match [`moves`] on the same layout — see the test.
pub fn uniform(count: usize, columns: usize) -> Moves {
    let cols = columns.max(1);
    let rows = count.div_ceil(cols);
    let sideways = |step: i32| -> Vec<Option<usize>> {
        (0..count)
            .map(|i| {
                let (r, c) = (i / cols, i % cols);
                // The last row is usually short, so its right-hand end is not
                // at `cols - 1`.
                let last = (count - r * cols).min(cols) - 1;
                Some(r * cols + (c as i32 + step).clamp(0, last as i32) as usize)
            })
            .collect()
    };
    let vertical = |step: i32| -> Vec<Option<usize>> {
        (0..count)
            .map(|i| {
                let (r, c) = (i / cols, i % cols);
                let target = r as i32 + step;
                if target < 0 || target >= rows as i32 {
                    return None;
                }
                let target = target as usize;
                // Landing on a short last row: the column you were in may not
                // exist there, so take the nearest that does — which is what
                // matching on horizontal center comes to when every card is
                // the same width.
                let last = (count - target * cols).min(cols) - 1;
                Some(target * cols + c.min(last))
            })
            .collect()
    };
    Moves {
        up: vertical(-1),
        down: vertical(1),
        left: sideways(-1),
        right: sideways(1),
        page_up: vertical(-PAGE),
        page_down: vertical(PAGE),
        first: edge(count, false),
        last: edge(count, true),
    }
}

/// The first or last card on the page, in the order they were handed over.
pub fn edge(count: usize, last: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(if last { count - 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three rows of four, then a short last row of two — the shape that broke
    /// every clamped implementation of this.
    fn grid_4x3_plus_2() -> Vec<Card> {
        let mut cards = Vec::new();
        for r in 0..4 {
            let wide = if r == 3 { 2 } else { 4 };
            for c in 0..wide {
                cards.push(Card { top: r as f64 * 100.0, left: c as f64 * 50.0, width: 40.0 });
            }
        }
        cards
    }

    #[test]
    fn cards_are_grouped_into_the_rows_they_were_drawn_in() {
        let grid = rows(&grid_4x3_plus_2());
        assert_eq!(grid.len(), 4);
        assert_eq!(grid[0], [0, 1, 2, 3]);
        assert_eq!(grid[3], [12, 13], "the short last row was padded out");
    }

    /// A few px of jitter between cards of differing height is normal.
    #[test]
    fn a_little_jitter_is_still_one_row() {
        let cards = vec![
            Card { top: 0.0, left: 0.0, width: 40.0 },
            Card { top: 4.0, left: 50.0, width: 40.0 },
            Card { top: 40.0, left: 0.0, width: 40.0 },
        ];
        assert_eq!(rows(&cards), vec![vec![0, 1], vec![2]]);
    }

    /// Cards do not always arrive in reading order — a grouped search result
    /// is drawn section by section.
    #[test]
    fn rows_read_top_to_bottom_and_left_to_right_whatever_the_order_in() {
        let cards = vec![
            Card { top: 100.0, left: 50.0, width: 40.0 },
            Card { top: 0.0, left: 50.0, width: 40.0 },
            Card { top: 0.0, left: 0.0, width: 40.0 },
        ];
        assert_eq!(rows(&cards), vec![vec![2, 1], vec![0]]);
    }

    /// The bug that made this feel random: running off the end of a row into
    /// the next one.
    #[test]
    fn left_and_right_stop_at_the_ends_of_their_row() {
        let cards = grid_4x3_plus_2();
        assert_eq!(move_x(&cards, Some(3), 1), Some(3), "right spilled into the next row");
        assert_eq!(move_x(&cards, Some(4), -1), Some(4), "left spilled into the previous row");
        assert_eq!(move_x(&cards, Some(1), 1), Some(2));
    }

    /// Up on the top row used to clamp to index 0, so you jumped to the first
    /// card instead of staying where you were.
    #[test]
    fn up_and_down_stay_put_at_the_edges() {
        let cards = grid_4x3_plus_2();
        assert_eq!(move_y(&cards, Some(2), -1), None, "up from the top row moved");
        assert_eq!(move_y(&cards, Some(13), 1), None, "down from the last row moved");
    }

    /// Matched on horizontal center, so a short last row lands under the card
    /// you were on rather than at an index that no longer exists.
    #[test]
    fn a_short_last_row_is_entered_below_where_you_were() {
        let cards = grid_4x3_plus_2();
        assert_eq!(move_y(&cards, Some(9), 1), Some(13), "column 1 did not land under itself");
        // Column 3 has nothing below it; the nearest card is the end of the
        // short row rather than nothing at all.
        assert_eq!(move_y(&cards, Some(11), 1), Some(13));
    }

    /// Pressing a direction on a freshly drawn grid has to do something.
    #[test]
    fn nothing_selected_lands_on_the_first_card() {
        let cards = grid_4x3_plus_2();
        assert_eq!(move_x(&cards, None, 1), Some(0));
        assert_eq!(move_y(&cards, None, 1), Some(0));
    }

    #[test]
    fn an_empty_page_moves_nowhere_rather_than_failing() {
        assert_eq!(move_x(&[], None, 1), None);
        assert_eq!(move_y(&[], None, 1), None);
        assert_eq!(edge(0, false), None);
    }

    #[test]
    fn the_ends_are_the_ends_of_the_page_not_of_a_row() {
        assert_eq!(edge(14, false), Some(0));
        assert_eq!(edge(14, true), Some(13));
    }

    /// The table a front end navigates by has to give the same answers as
    /// asking one move at a time — it is the same page, worked out in advance.
    #[test]
    fn the_table_agrees_with_asking_one_move_at_a_time() {
        let cards = grid_4x3_plus_2();
        let table = moves(&cards);
        for i in 0..cards.len() {
            assert_eq!(table.left[i], move_x(&cards, Some(i), -1), "left from {i}");
            assert_eq!(table.right[i], move_x(&cards, Some(i), 1), "right from {i}");
            assert_eq!(table.up[i], move_y(&cards, Some(i), -1), "up from {i}");
            assert_eq!(table.down[i], move_y(&cards, Some(i), 1), "down from {i}");
            assert_eq!(table.page_up[i], move_y(&cards, Some(i), -PAGE), "page up from {i}");
            assert_eq!(table.page_down[i], move_y(&cards, Some(i), PAGE), "page down from {i}");
        }
        assert_eq!(table.first, Some(0));
        assert_eq!(table.last, Some(cards.len() - 1));
    }

    /// The two ways of working the table out have to agree, or the cursor
    /// behaves differently on a long list from a short one — which is exactly
    /// the sort of difference nobody attributes to windowing.
    #[test]
    fn the_uniform_table_matches_the_measured_one() {
        for (count, columns) in [(14usize, 4usize), (40, 10), (41, 10), (7, 1), (3, 9)] {
            let cards: Vec<Card> = (0..count)
                .map(|i| Card {
                    top: (i / columns) as f64 * 200.0,
                    left: (i % columns) as f64 * 160.0,
                    width: 150.0,
                })
                .collect();
            let measured = moves(&cards);
            let derived = uniform(count, columns);
            assert_eq!(derived.up, measured.up, "up, {count} over {columns}");
            assert_eq!(derived.down, measured.down, "down, {count} over {columns}");
            assert_eq!(derived.left, measured.left, "left, {count} over {columns}");
            assert_eq!(derived.right, measured.right, "right, {count} over {columns}");
            assert_eq!(derived.page_up, measured.page_up, "page up, {count} over {columns}");
            assert_eq!(derived.page_down, measured.page_down, "page down, {count} over {columns}");
            assert_eq!(derived.first, measured.first);
            assert_eq!(derived.last, measured.last);
        }
    }

    /// The case windowing exists for: a table over rows that were never drawn.
    #[test]
    fn a_uniform_table_covers_rows_nothing_measured() {
        let table = uniform(2506, 10);
        assert_eq!(table.down.len(), 2506);
        assert_eq!(table.down[0], Some(10));
        assert_eq!(table.up[0], None, "up from the first row moved");
        assert_eq!(table.down[2505], None, "down from the last row moved");
        // 2,506 over ten leaves six on the last row. Column 8 has nothing
        // below it, so it lands on the end of the short row rather than off it.
        assert_eq!(table.down[2498], Some(2505));
    }

    #[test]
    fn an_empty_page_has_an_empty_table() {
        let table = moves(&[]);
        assert!(table.up.is_empty());
        assert_eq!(table.first, None);
        assert_eq!(table.last, None);
    }

    /// A list is a grid one card wide, and needs no special case.
    #[test]
    fn a_single_column_list_behaves_like_a_grid_one_wide() {
        let cards: Vec<Card> = (0..5)
            .map(|r| Card { top: r as f64 * 40.0, left: 0.0, width: 300.0 })
            .collect();
        assert_eq!(move_y(&cards, Some(0), 1), Some(1));
        assert_eq!(move_x(&cards, Some(2), 1), Some(2), "a one-wide row has nowhere to go sideways");
    }
}



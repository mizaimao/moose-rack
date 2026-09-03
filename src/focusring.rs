// Reaching everything on a screen with a direction pad.
//
// The cursor today walks a list, because a list is a column of identical boxes
// and "the next one" is obvious. Everything that is not a list — the tab row,
// the buttons in the header, the sort and filter menus, the settings window —
// is reachable only with a mouse. On a desktop that is fine. On an Android
// handheld there is no pointer at all, so anything the pad cannot reach may as
// well not be drawn: see `docs/parked.md`.
//
// So this is the other half of navigation. Given where everything is on the
// screen, it answers "if I press left from here, what should light up" for
// every thing on it at once — a table, computed when the screen is drawn and
// then walked without asking again, which is the same bargain `gridnav` makes
// and for the same reason (`docs/handheld-frontend.md`: nothing per keypress
// crosses a boundary).
//
// `gridnav` is not this. It assumes a wall of equal cards in rows and answers
// in rows and columns. A screen is not a grid: a tab is 90 points wide beside
// one that is 40, the preview column is one tall box beside eighty small ones,
// and there is no row that contains both.

use serde::{Deserialize, Serialize};

/// One thing that can be focused, and where it is.
///
/// The id is whatever the caller wants to get back — an element id in the
/// webview, an index in the SDL front end. Nothing here reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    #[serde(default)]
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Spot {
    pub fn new(id: &str, x: f64, y: f64, w: f64, h: f64) -> Self {
        Spot { id: id.to_owned(), x, y, w, h }
    }

    fn right(&self) -> f64 {
        self.x + self.w
    }

    fn bottom(&self) -> f64 {
        self.y + self.h
    }

    fn mid_x(&self) -> f64 {
        self.x + self.w / 2.0
    }

    fn mid_y(&self) -> f64 {
        self.y + self.h / 2.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    pub fn name(self) -> &'static str {
        match self {
            Dir::Up => "up",
            Dir::Down => "down",
            Dir::Left => "left",
            Dir::Right => "right",
        }
    }
}

/// How much harder it is to drift sideways than to travel forwards.
///
/// Pressing right should land on the thing to the right, not on the thing
/// slightly right and a long way down that happens to be a few points nearer.
/// Thirteen is what Android's own focus search uses, and it is high enough
/// that a row of buttons walks along itself rather than wandering into the
/// list underneath.
const SIDEWAYS_COST: f64 = 13.0;

/// Where each direction leads from each spot.
///
/// Four arrays the same length as the spots, each holding the index to land on
/// or `None` for "nothing that way, stay put".
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Ring {
    pub up: Vec<Option<usize>>,
    pub down: Vec<Option<usize>>,
    pub left: Vec<Option<usize>>,
    pub right: Vec<Option<usize>>,
}

impl Ring {
    pub fn step(&self, from: usize, dir: Dir) -> Option<usize> {
        let list = match dir {
            Dir::Up => &self.up,
            Dir::Down => &self.down,
            Dir::Left => &self.left,
            Dir::Right => &self.right,
        };
        list.get(from).copied().flatten()
    }
}

/// The whole table, computed once for a screen.
pub fn ring(spots: &[Spot]) -> Ring {
    let each = |dir: Dir| (0..spots.len()).map(|i| nearest(spots, i, dir)).collect();
    Ring {
        up: each(Dir::Up),
        down: each(Dir::Down),
        left: each(Dir::Left),
        right: each(Dir::Right),
    }
}

/// What lies `dir` of `from`, or `None`.
///
/// A candidate has to be genuinely in that direction — its leading edge past
/// the leading edge of where we are — and then the nearest one wins, counting
/// sideways drift much more heavily than forward distance. Anything whose
/// perpendicular span overlaps ours beats anything whose does not, however
/// close, because a button directly above is what "up" means even when
/// something diagonal is nearer in a straight line.
pub fn nearest(spots: &[Spot], from: usize, dir: Dir) -> Option<usize> {
    let here = spots.get(from)?;
    let mut best: Option<(bool, f64, usize)> = None;
    for (i, spot) in spots.iter().enumerate() {
        if i == from {
            continue;
        }
        if !beyond(here, spot, dir) {
            continue;
        }
        let major = forward(here, spot, dir).max(0.0);
        let minor = sideways(here, spot, dir);
        let lined_up = overlaps(here, spot, dir);
        // The drift is what gets multiplied, not the distance. Written the
        // other way round first — which is how Android writes it, because
        // Android gives absolute priority to anything in the beam and only
        // falls back to this — and with nothing in the beam it means "close in
        // any direction wins", so pressing right from the last tab dived into
        // a card two rows down instead of crossing to the header button.
        let score = major * major + SIDEWAYS_COST * minor * minor;
        let better = match &best {
            None => true,
            // Lined up always wins. Between two that are, or two that are not,
            // the lower score wins.
            Some((was_lined_up, was_score, _)) => match (lined_up, was_lined_up) {
                (true, false) => true,
                (false, true) => false,
                _ => score < *was_score,
            },
        };
        if better {
            best = Some((lined_up, score, i));
        }
    }
    best.map(|(_, _, i)| i)
}

/// Is `there` far enough that way to count?
///
/// Measured on the leading edge rather than the center: a tall preview column
/// beside a short card has a center far below it and is still plainly to the
/// right.
fn beyond(here: &Spot, there: &Spot, dir: Dir) -> bool {
    match dir {
        Dir::Left => there.x < here.x && there.right() <= here.right(),
        Dir::Right => there.right() > here.right() && there.x >= here.x,
        Dir::Up => there.y < here.y && there.bottom() <= here.bottom(),
        Dir::Down => there.bottom() > here.bottom() && there.y >= here.y,
    }
}

/// How far along the direction of travel, edge to edge.
fn forward(here: &Spot, there: &Spot, dir: Dir) -> f64 {
    match dir {
        Dir::Left => here.x - there.right(),
        Dir::Right => there.x - here.right(),
        Dir::Up => here.y - there.bottom(),
        Dir::Down => there.y - here.bottom(),
    }
}

/// How far off the line of travel, center to center.
fn sideways(here: &Spot, there: &Spot, dir: Dir) -> f64 {
    match dir {
        Dir::Left | Dir::Right => (here.mid_y() - there.mid_y()).abs(),
        Dir::Up | Dir::Down => (here.mid_x() - there.mid_x()).abs(),
    }
}

/// Do the two overlap across the direction of travel — is one actually above
/// the other, rather than diagonally off?
fn overlaps(here: &Spot, there: &Spot, dir: Dir) -> bool {
    match dir {
        Dir::Left | Dir::Right => there.y < here.bottom() && here.y < there.bottom(),
        Dir::Up | Dir::Down => there.x < here.right() && here.x < there.right(),
    }
}

/// Where the ring starts when nothing is focused yet: the top-left-most spot,
/// reading order, which is where a person's eye already is.
pub fn first(spots: &[Spot]) -> Option<usize> {
    spots
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

/// The spot under a point, if any. The mouse and the ring agree about what is
/// where, so pointing at something and stepping to it land in the same place.
pub fn at(spots: &[Spot], x: f64, y: f64) -> Option<usize> {
    spots
        .iter()
        .position(|s| x >= s.x && x < s.right() && y >= s.y && y < s.bottom())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A screen shaped like the app's: a tab row, two header buttons on the
    /// right, a grid of six cards, and a tall preview column beside them.
    fn screen() -> Vec<Spot> {
        vec![
            Spot::new("tab-library", 10.0, 0.0, 90.0, 40.0),
            Spot::new("tab-mine", 100.0, 0.0, 130.0, 40.0),
            Spot::new("tab-history", 230.0, 0.0, 80.0, 40.0),
            Spot::new("btn-sort", 700.0, 45.0, 70.0, 30.0),
            Spot::new("btn-layout", 780.0, 45.0, 70.0, 30.0),
            Spot::new("card-0", 10.0, 90.0, 150.0, 200.0),
            Spot::new("card-1", 170.0, 90.0, 150.0, 200.0),
            Spot::new("card-2", 330.0, 90.0, 150.0, 200.0),
            Spot::new("card-3", 10.0, 300.0, 150.0, 200.0),
            Spot::new("card-4", 170.0, 300.0, 150.0, 200.0),
            Spot::new("card-5", 330.0, 300.0, 150.0, 200.0),
            Spot::new("preview", 700.0, 90.0, 150.0, 410.0),
        ]
    }

    fn go(spots: &[Spot], from: &str, dir: Dir) -> String {
        let i = spots.iter().position(|s| s.id == from).expect("no such spot");
        match nearest(spots, i, dir) {
            Some(to) => spots[to].id.clone(),
            None => "—".to_owned(),
        }
    }

    #[test]
    fn a_row_of_tabs_walks_along_itself() {
        let s = screen();
        assert_eq!(go(&s, "tab-library", Dir::Right), "tab-mine");
        assert_eq!(go(&s, "tab-mine", Dir::Right), "tab-history");
        assert_eq!(go(&s, "tab-mine", Dir::Left), "tab-library");
        assert_eq!(go(&s, "tab-library", Dir::Left), "—", "walked off the left edge");
    }

    /// The bug this weighting is for: from the middle tab, "right" must not
    /// dive into a header button that is further away but roughly level.
    #[test]
    fn right_does_not_dive_into_another_row() {
        let s = screen();
        assert_eq!(go(&s, "tab-history", Dir::Right), "btn-sort");
        assert_eq!(go(&s, "tab-library", Dir::Right), "tab-mine");
    }

    #[test]
    fn the_grid_still_behaves_like_a_grid() {
        let s = screen();
        assert_eq!(go(&s, "card-0", Dir::Right), "card-1");
        assert_eq!(go(&s, "card-1", Dir::Right), "card-2");
        assert_eq!(go(&s, "card-0", Dir::Down), "card-3");
        assert_eq!(go(&s, "card-4", Dir::Up), "card-1");
        assert_eq!(go(&s, "card-5", Dir::Down), "—");
    }

    /// The point of the whole thing: from a card, the preview column and the
    /// header are reachable without a mouse.
    #[test]
    fn everything_is_reachable_from_a_card() {
        let s = screen();
        assert_eq!(go(&s, "card-2", Dir::Right), "preview");
        assert_eq!(go(&s, "card-0", Dir::Up), "tab-library");
        assert_eq!(go(&s, "preview", Dir::Up), "btn-sort");
    }

    /// A tall box beside a short one is still to the right of it, even though
    /// its center is a long way below. Measured on edges for this reason.
    #[test]
    fn a_tall_column_is_beside_the_card_not_below_it() {
        let s = screen();
        assert_eq!(go(&s, "card-5", Dir::Right), "preview");
        assert_eq!(go(&s, "preview", Dir::Left), "card-2");
    }

    /// Nothing is stranded: from every spot on the screen, every other spot is
    /// reachable in some number of steps. A control that can be drawn and not
    /// arrived at is the whole failure this exists to prevent.
    #[test]
    fn no_control_is_stranded() {
        let s = screen();
        let table = ring(&s);
        for start in 0..s.len() {
            let mut seen = vec![false; s.len()];
            let mut queue = vec![start];
            seen[start] = true;
            while let Some(at) = queue.pop() {
                for dir in Dir::ALL {
                    if let Some(next) = table.step(at, dir)
                        && !seen[next]
                    {
                        seen[next] = true;
                        queue.push(next);
                    }
                }
            }
            let stranded: Vec<&str> = s
                .iter()
                .zip(&seen)
                .filter(|(_, got)| !**got)
                .map(|(spot, _)| spot.id.as_str())
                .collect();
            assert!(
                stranded.is_empty(),
                "from {}: cannot reach {stranded:?}",
                s[start].id
            );
        }
    }

    /// Within a grid, every step must be reversible: press right then left and
    /// be back where you started.
    ///
    /// Only within the grid. Across a whole screen it cannot hold and should
    /// not be asked for — three tabs sit above two columns of cards, so some
    /// card has to answer "up" with a tab that does not answer "down" with it.
    /// Demanding it everywhere would mean carrying the column you entered from
    /// as state, which is a different feature and not obviously a better one.
    #[test]
    fn stepping_back_through_the_grid_returns_where_you_came_from() {
        let s: Vec<Spot> = screen().into_iter().filter(|s| s.id.starts_with("card-")).collect();
        let table = ring(&s);
        let back = |d: Dir| match d {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        };
        for i in 0..s.len() {
            for dir in Dir::ALL {
                let Some(there) = table.step(i, dir) else { continue };
                let home = table.step(there, back(dir));
                assert_eq!(
                    home,
                    Some(i),
                    "{} --{}--> {} but coming back lands on {:?}",
                    s[i].id,
                    dir.name(),
                    s[there].id,
                    home.map(|h| s[h].id.as_str())
                );
            }
        }
    }

    #[test]
    fn an_empty_screen_answers_nothing_rather_than_panicking() {
        assert_eq!(nearest(&[], 0, Dir::Up), None);
        assert_eq!(first(&[]), None);
        assert_eq!(ring(&[]), Ring::default());
    }

    #[test]
    fn the_ring_starts_at_the_top_left() {
        let s = screen();
        assert_eq!(s[first(&s).unwrap()].id, "tab-library");
    }

    #[test]
    fn the_pointer_and_the_ring_agree() {
        let s = screen();
        assert_eq!(s[at(&s, 200.0, 20.0).unwrap()].id, "tab-mine");
        assert_eq!(s[at(&s, 250.0, 150.0).unwrap()].id, "card-1");
        assert_eq!(s[at(&s, 400.0, 150.0).unwrap()].id, "card-2");
        assert_eq!(at(&s, 600.0, 600.0), None, "found something in empty space");
    }
}

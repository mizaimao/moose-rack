// How a list of games is ordered.
//
// Per view, and deliberately not saved — see [`crate::gamelist::Chosen`] for
// why. This module is the orders themselves and the comparison, which is the
// part every front end needs and the part that was living in the webview.

use crate::gamelist::{Row, name_cmp};
use serde::Serialize;
use std::cmp::Ordering;

/// One way to order a list.
///
/// `dir` is 1 for ascending. Where the key is a thing you want *more* of — a
/// rating, a year, a size, a recent play — it is -1, because the interesting
/// end of those is the top.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Order {
    pub id: &'static str,
    pub label: &'static str,
    pub dir: i8,
}

/// The orders, in the sequence a menu lists them.
pub const ORDERS: &[Order] = &[
    Order { id: "name", label: "Name", dir: 1 },
    Order { id: "rating", label: "Rating", dir: -1 },
    Order { id: "year", label: "Release year", dir: -1 },
    Order { id: "played", label: "Recently played", dir: -1 },
    Order { id: "size", label: "Size", dir: -1 },
    // Only meaningful where the rows come from more than one console — a
    // search, or Continue playing. Inside a console every row has the same key
    // and this degrades to the name tie-break, which is harmless.
    Order { id: "platform", label: "Console", dir: 1 },
];

pub fn by_id(id: &str) -> Option<&'static Order> {
    ORDERS.iter().find(|o| o.id == id)
}

/// Step to the next order, for the stick click that has no menu behind it.
pub fn cycle(from: &str, delta: i32) -> &'static Order {
    let at = ORDERS.iter().position(|o| o.id == from).unwrap_or(0) as i32;
    let len = ORDERS.len() as i32;
    &ORDERS[(at + delta).rem_euclid(len) as usize]
}

/// Compare two rows under one order.
///
/// Favorites stay on top whatever the order, which is what they are for. A
/// game with nothing to compare sorts last rather than first: an unrated game
/// is not a bad one, and a screen that opens on the unknowns is a screen that
/// answers nothing. Name breaks the tie, so two games with the same rating do
/// not swap places between one redraw and the next.
pub fn compare(a: &Row, b: &Row, order_id: &str) -> Ordering {
    if a.favorite != b.favorite {
        return if a.favorite { Ordering::Less } else { Ordering::Greater };
    }
    let order = by_id(order_id).unwrap_or(&ORDERS[0]);
    let keyed = match order.id {
        "name" => name_cmp(&a.name, &b.name),
        // `missing` is -1 rather than 0, so an unrated game sorts below a game
        // rated zero rather than alongside it.
        "rating" => cmp_f64(a.rating.unwrap_or(-1.0), b.rating.unwrap_or(-1.0)),
        "year" => a.year.unwrap_or(-1).cmp(&b.year.unwrap_or(-1)),
        // ISO timestamps, comparable as text. Never played is the empty
        // string, which sorts below every real date.
        "played" => a.last_played.as_deref().unwrap_or("").cmp(b.last_played.as_deref().unwrap_or("")),
        "size" => a.size_bytes.cmp(&b.size_bytes),
        "platform" => name_cmp(&a.platform, &b.platform),
        _ => Ordering::Equal,
    };
    let keyed = if order.dir < 0 { keyed.reverse() } else { keyed };
    keyed.then_with(|| name_cmp(&a.name, &b.name))
}

/// A total order over ratings. `f64` has no `Ord` because of NaN, which cannot
/// reach here — every rating is either a number the server sent or the -1 that
/// stands in for none.
fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// Order a list in place. Callers hold references, so the rows themselves are
/// never moved and no caller's own array is disturbed.
pub fn sort(rows: &mut [&Row], order_id: &str) {
    rows.sort_by(|a, b| compare(a, b, order_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, rating: Option<f64>, year: Option<i32>) -> Row {
        Row { name: name.to_owned(), rating, year, ..Row::default() }
    }

    fn names<'a>(rows: &'a [Row], order: &str) -> Vec<&'a str> {
        let mut refs: Vec<&Row> = rows.iter().collect();
        sort(&mut refs, order);
        refs.into_iter().map(|r| r.name.as_str()).collect()
    }

    /// Ported from `ui/test/gamepad.test.js`, "sorting a list".
    #[test]
    fn alphabetical_by_default() {
        let rows = vec![row("Zelda", None, None), row("asteroids", None, None), row("Metroid", None, None)];
        assert_eq!(names(&rows, "name"), ["asteroids", "Metroid", "Zelda"]);
    }

    /// An unrated game is not a bad one, and a screen that opens on the
    /// unknowns is a screen that answers nothing.
    #[test]
    fn rating_sorts_high_to_low_with_the_unrated_last() {
        let rows = vec![
            row("Middling", Some(5.0), None),
            row("Unknown", None, None),
            row("Great", Some(9.0), None),
            row("Zero", Some(0.0), None),
        ];
        assert_eq!(names(&rows, "rating"), ["Great", "Middling", "Zero", "Unknown"]);
    }

    #[test]
    fn a_missing_year_sorts_last_too() {
        let rows = vec![row("New", None, Some(2001)), row("Undated", None, None), row("Old", None, Some(1987))];
        assert_eq!(names(&rows, "year"), ["New", "Old", "Undated"]);
    }

    /// What favorites are for.
    #[test]
    fn favorites_stay_on_top_of_any_order() {
        let mut starred = row("Zebra", Some(1.0), None);
        starred.favorite = true;
        let rows = vec![row("Alpha", Some(9.0), None), starred];
        for order in ORDERS {
            assert_eq!(names(&rows, order.id)[0], "Zebra", "not first under {}", order.id);
        }
    }

    /// Two games with the same key must not swap places between one redraw and
    /// the next, which is what the name tie-break is for.
    #[test]
    fn equal_keys_fall_back_to_the_name() {
        let rows = vec![row("Beta", Some(8.0), None), row("Alpha", Some(8.0), None)];
        assert_eq!(names(&rows, "rating"), ["Alpha", "Beta"]);
        // And it is stable across repeats, not merely once.
        assert_eq!(names(&rows, "rating"), names(&rows, "rating"));
    }

    #[test]
    fn never_played_sorts_below_every_real_date() {
        let mut played = Row { name: "Played".to_owned(), ..Row::default() };
        played.last_played = Some("2026-01-01T00:00:00Z".to_owned());
        let rows = vec![Row { name: "Never".to_owned(), ..Row::default() }, played];
        assert_eq!(names(&rows, "played"), ["Played", "Never"]);
    }

    #[test]
    fn cycling_wraps_in_both_directions() {
        assert_eq!(cycle("name", 1).id, "rating");
        assert_eq!(cycle("platform", 1).id, "name", "the last order did not wrap");
        assert_eq!(cycle("name", -1).id, "platform", "stepping back did not wrap");
    }

    /// The order id comes from a saved choice and from a menu, so a stale or
    /// mistyped one must degrade to the default rather than throw the list
    /// into an arbitrary arrangement.
    #[test]
    fn an_unknown_order_falls_back_to_the_first() {
        let rows = vec![row("Zelda", None, None), row("asteroids", None, None)];
        assert_eq!(names(&rows, "nonsense"), ["asteroids", "Zelda"]);
    }
}


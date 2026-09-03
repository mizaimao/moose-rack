// Narrowing a list down to the part you meant.
//
// The arcade console holds 2,506 games. Sorting them is not the same as
// finding one: "by rating" still leaves 2,506 rows, and the eleven you have
// actually played are somewhere in it.
//
// Built out of what a row already carries — downloaded, starred, rated, its
// year, whether it has ever been played — so nothing has to be fetched and the
// filtering is instant on a list of any size. Genre is missing on purpose: the
// RomM browse tab is genres, done properly, as collections from the server.

use crate::gamelist::Row;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Filter {
    pub id: &'static str,
    pub label: &'static str,
}

/// The filters, in the order a menu offers them.
///
/// Several can be on at once and they all have to pass — "downloaded" and
/// "never played" together is the list of things taking up disk that you have
/// not touched, which is a question worth asking and cannot be asked any other
/// way.
pub const FILTERS: &[Filter] = &[
    Filter { id: "local", label: "On this machine" },
    Filter { id: "missing", label: "Not downloaded" },
    Filter { id: "fav", label: "Starred" },
    Filter { id: "unplayed", label: "Never played" },
    Filter { id: "played", label: "Played before" },
    // 8/10 on RomM's scale. A "good games" filter with no number attached is a
    // filter nobody can predict the results of.
    Filter { id: "great", label: "Rated 8 or better" },
    // The first filter here about the *game* rather than about your
    // relationship to it. "Somebody is coming over" is a real question and the
    // answer was buried in a metadata blob.
    Filter { id: "twoplayer", label: "Two players or more" },
];

/// Does this row survive that filter?
///
/// An unknown player count is excluded, not assumed: two thirds of a real
/// library has no player count, and treating those as two-player would make
/// the filter meaningless. It exists to narrow, and the honest answer to "we
/// do not know" is to leave the row out.
///
/// An id nothing recognizes keeps everything, so a filter saved by a newer
/// build cannot empty a list on an older one.
pub fn keeps(id: &str, row: &Row) -> bool {
    match id {
        "local" => row.downloaded,
        "missing" => !row.downloaded,
        "fav" => row.favorite,
        "unplayed" => row.last_played.is_none(),
        "played" => row.last_played.is_some(),
        "great" => row.rating.unwrap_or(-1.0) >= 8.0,
        "twoplayer" => row.players.unwrap_or(0) >= 2,
        _ => true,
    }
}

/// Pairs that cannot both be true.
///
/// Choosing one clears the other rather than leaving an empty list and no clue
/// why — "on this machine" plus "not downloaded" matches nothing, ever.
pub fn opposite(id: &str) -> Option<&'static str> {
    match id {
        "local" => Some("missing"),
        "missing" => Some("local"),
        "unplayed" => Some("played"),
        "played" => Some("unplayed"),
        _ => None,
    }
}

/// Which rows survive. Every filter has to pass; no filters means every row.
pub fn retain(rows: &[Row], active: &BTreeSet<String>) -> Vec<usize> {
    (0..rows.len())
        .filter(|&i| active.iter().all(|f| keeps(f, &rows[i])))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same four rows `ui/test/filter.test.js` uses. Delta has no player
    /// count on purpose: two thirds of the real library has none, and that
    /// case decides whether the filter is useful or noise.
    fn rows() -> Vec<Row> {
        vec![
            Row { id: 1, name: "Alpha".into(), downloaded: true, favorite: false, last_played: None, rating: Some(9.0), players: Some(1), ..Row::default() },
            Row { id: 2, name: "Beta".into(), downloaded: false, favorite: true, last_played: Some("2026-01-01".into()), rating: Some(4.0), players: Some(2), ..Row::default() },
            Row { id: 3, name: "Gamma".into(), downloaded: true, favorite: true, last_played: Some("2026-02-02".into()), rating: Some(8.0), players: Some(4), ..Row::default() },
            Row { id: 4, name: "Delta".into(), downloaded: false, favorite: false, last_played: None, rating: None, players: None, ..Row::default() },
        ]
    }

    fn names(on: &[&str]) -> Vec<String> {
        let rows = rows();
        let active: BTreeSet<String> = on.iter().map(|s| (*s).to_owned()).collect();
        retain(&rows, &active).into_iter().map(|i| rows[i].name.clone()).collect()
    }

    #[test]
    fn nothing_on_leaves_the_list_alone() {
        assert_eq!(names(&[]).len(), 4);
    }

    #[test]
    fn each_one_keeps_what_it_says() {
        assert_eq!(names(&["local"]), ["Alpha", "Gamma"]);
        assert_eq!(names(&["fav"]), ["Beta", "Gamma"]);
        assert_eq!(names(&["unplayed"]), ["Alpha", "Delta"]);
        assert_eq!(names(&["great"]), ["Alpha", "Gamma"], "an unrated game counted as good");
        assert_eq!(names(&["twoplayer"]), ["Beta", "Gamma"], "a one-player game got through");
    }

    /// A game nothing says a player count for is not a two-player game.
    /// Assuming otherwise would let two thirds of the library through and make
    /// the filter worthless.
    #[test]
    fn an_unknown_player_count_is_not_counted_as_two_players() {
        let kept = names(&["twoplayer"]);
        assert!(!kept.contains(&"Delta".to_owned()), "a game with no player count got through");
        assert!(!kept.contains(&"Alpha".to_owned()), "a one-player game got through");
    }

    /// The reason several can be on at once: "downloaded and never played" is
    /// the list of things taking up disk you have not touched, and there is no
    /// other way to ask it.
    #[test]
    fn two_of_them_both_have_to_pass() {
        assert_eq!(names(&["local", "unplayed"]), ["Alpha"]);
    }

    #[test]
    fn opposites_are_paired_both_ways() {
        assert_eq!(opposite("local"), Some("missing"));
        assert_eq!(opposite("missing"), Some("local"));
        assert_eq!(opposite("played"), Some("unplayed"));
        assert_eq!(opposite("unplayed"), Some("played"));
        assert_eq!(opposite("fav"), None, "starred has no opposite to cancel");
    }

    /// Ported from `ui/test/filter.test.js`, "opposites cancel rather than
    /// emptying the list".
    #[test]
    fn choosing_one_opposite_clears_the_other() {
        let mut chosen = crate::gamelist::Chosen::default();
        chosen.toggle_filter("roms:arcade:", "local");
        let on = chosen.toggle_filter("roms:arcade:", "missing");
        assert_eq!(on.into_iter().collect::<Vec<_>>(), ["missing"]);
    }

    /// A filter saved by a newer build must not empty the list on an older one.
    #[test]
    fn an_unknown_filter_keeps_everything() {
        assert_eq!(names(&["genre:shmup"]).len(), 4);
    }

    /// Ported from `ui/test/filter.test.js`, "they belong to the list they
    /// were set on".
    #[test]
    fn filters_belong_to_the_list_they_were_set_on() {
        let mut chosen = crate::gamelist::Chosen::default();
        chosen.toggle_filter("roms:arcade:", "fav");
        assert!(chosen.filters("roms:gb:").is_empty(), "the next console inherited them");
        assert_eq!(chosen.filters("roms:arcade:").len(), 1, "and lost them coming back");
    }

    /// Filtering and ordering in one pass, which is what a front end asks for.
    #[test]
    fn arrange_narrows_then_orders() {
        let rows = rows();
        let on: BTreeSet<String> = ["local"].iter().map(|s| (*s).to_owned()).collect();
        // Gamma is starred, so it leads whatever the order says.
        assert_eq!(crate::gamelist::arrange(&rows, "name", &on), [3, 1]);
    }
}

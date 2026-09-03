// What a list of games knows about itself: the row shape, which view it is,
// and the ordering and narrowing a person has chosen for it.
//
// The shape is shared because sorting and filtering want the same nine facts
// and neither should have to fetch anything: a filter that has to ask the
// server which games are downloaded is a filter that is slow on the console
// that needs it most — arcade, at 2,506 games.
//
// See [`crate::gamesort`] for the orders and [`crate::gamefilter`] for the
// filters. Both are driven from here.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// One game as a list draws it.
///
/// Every field is something a row already carries by the time it reaches a
/// front end — see `RomView` in the Tauri layer, which is this plus the two
/// identifiers a list needs to act on a row.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Row {
    pub id: i64,
    pub name: String,
    /// Console slug. Only meaningful where the rows come from more than one —
    /// a search, or Continue playing.
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub downloaded: bool,
    /// In a starred collection.
    #[serde(default)]
    pub favorite: bool,
    /// RomM's 0-10 scale.
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub year: Option<i32>,
    /// ISO timestamp, comparable as text.
    #[serde(default)]
    pub last_played: Option<String>,
    #[serde(default)]
    pub size_bytes: i64,
    /// The most players the game supports, or `None` when nothing says.
    #[serde(default)]
    pub players: Option<u8>,
}

/// Which list is on screen, as the key the chosen order and filters hang off.
///
/// Keyed by what is being looked at rather than globally: "sort this console
/// by rating" and "show me what I have not played here" are both statements
/// about *this* console. Finding every other console still sorted and filtered
/// that way is a setting that has outlived its question.
pub fn scope(view: &str, platform: Option<&str>, collection: Option<&str>) -> String {
    format!(
        "{view}:{}:{}",
        platform.unwrap_or_default(),
        collection.unwrap_or_default()
    )
}

/// Whether this view has anything worth sorting.
///
/// The console grid has no sort of its own: it is one screen of a couple of
/// dozen tiles in a fixed order that people learn the shape of, and shuffling
/// it would cost more than it gives.
pub fn sortable(view: &str) -> bool {
    view != "platforms" && view != "systems"
}

/// Whether this view has a list worth narrowing.
pub fn filterable(view: &str) -> bool {
    sortable(view) && view != "history"
}

/// Compare two titles the way a person reading a list would.
///
/// Case-insensitively first, so "Zelda" and "asteroids" land where they are
/// looked for rather than in ASCII order with every capital ahead of every
/// lower case. Raw bytes break the tie after that, so the order is total and
/// two games differing only in case cannot swap places between redraws.
///
/// This is an approximation of the `localeCompare` the webview used, and it
/// agrees with it across the Latin alphabet. It does not implement collation
/// for Japanese titles, which sort by code point here — a real ordering, just
/// not the one a Japanese reader would choose. Left as an approximation
/// deliberately: the alternative is carrying ICU.
pub fn name_cmp(a: &str, b: &str) -> Ordering {
    // Folded lazily, character by character. Lowercasing into two Strings is
    // the obvious way to write this and it allocates twice per comparison —
    // which, sorting 2,506 arcade games, is a hundred thousand allocations for
    // one press of the sort button.
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
        .then_with(|| a.cmp(b))
}

/// The order and filters chosen per view, for this run of the app only.
///
/// A map in memory, not a file — the forgetting is the feature. Sorting by
/// rating to see what a console is famous for, and then finding every console
/// still sorted that way a week later, is a setting that has outlived the
/// question that produced it. The order of the *column*, which is the shape of
/// the whole app rather than one question about one console, is remembered:
/// see [`crate::pickorder`].
#[derive(Debug, Default)]
pub struct Chosen {
    order: BTreeMap<String, String>,
    filters: BTreeMap<String, BTreeSet<String>>,
}

impl Chosen {
    /// The order for this scope, falling back to the first one offered.
    pub fn order(&self, scope: &str) -> &'static crate::gamesort::Order {
        self.order
            .get(scope)
            .and_then(|id| crate::gamesort::by_id(id))
            .unwrap_or(&crate::gamesort::ORDERS[0])
    }

    pub fn set_order(&mut self, scope: &str, id: &str) {
        self.order.insert(scope.to_owned(), id.to_owned());
    }

    /// The order a view starts in, if nobody has picked one for it yet.
    ///
    /// Continue playing is ordered by *when you played it* or it is not a
    /// continue-playing list — it arrives grouped by console, which throws
    /// that away. Set rather than forced, so choosing something else sticks.
    pub fn default_order(&mut self, scope: &str, id: &str) {
        self.order.entry(scope.to_owned()).or_insert_with(|| id.to_owned());
    }

    pub fn filters(&self, scope: &str) -> BTreeSet<String> {
        self.filters.get(scope).cloned().unwrap_or_default()
    }

    pub fn toggle_filter(&mut self, scope: &str, id: &str) -> BTreeSet<String> {
        let on = self.filters.entry(scope.to_owned()).or_default();
        if !on.remove(id) {
            on.insert(id.to_owned());
            // Choosing one clears its opposite rather than leaving an empty
            // list and no clue why — "on this machine" plus "not downloaded"
            // matches nothing, ever.
            if let Some(other) = crate::gamefilter::opposite(id) {
                on.remove(other);
            }
        }
        on.clone()
    }

    pub fn clear_filters(&mut self, scope: &str) {
        self.filters.remove(scope);
    }
}

/// Narrow, then order. Returns the ids to draw, in the order to draw them.
///
/// Ids rather than rows because the caller already has the rows: this answers
/// "which of these, and in what order", which is a few kilobytes rather than
/// the whole list travelling back to a front end that was handed it a moment
/// ago.
pub fn arrange(rows: &[Row], order_id: &str, filters: &BTreeSet<String>) -> Vec<i64> {
    let kept = crate::gamefilter::retain(rows, filters);
    let mut kept: Vec<&Row> = kept.into_iter().map(|i| &rows[i]).collect();
    crate::gamesort::sort(&mut kept, order_id);
    kept.into_iter().map(|r| r.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_belongs_to_the_view_it_was_set_on() {
        assert_eq!(scope("roms", Some("arcade"), None), "roms:arcade:");
        assert_ne!(
            scope("roms", Some("arcade"), None),
            scope("roms", Some("gb"), None),
            "two consoles share one set of filters"
        );
    }

    /// Ported from `ui/test/filter.test.js`, "the console grid has nothing to
    /// filter", and `ui/test/gamepad.test.js`, "the console grid has no sort".
    #[test]
    fn the_console_grid_has_neither_a_sort_nor_a_filter() {
        assert!(!sortable("platforms"));
        assert!(!filterable("platforms"));
        assert!(sortable("roms"));
        assert!(filterable("roms"));
        // History is a log, not a library — there is nothing in it to narrow.
        assert!(sortable("history"));
        assert!(!filterable("history"));
    }

    #[test]
    fn titles_read_in_the_order_a_person_looks_for_them() {
        assert_eq!(name_cmp("asteroids", "Zelda"), Ordering::Less);
        assert_eq!(name_cmp("Alpha", "Alpha"), Ordering::Equal);
        // Total, so two games differing only in case cannot swap places
        // between one redraw and the next.
        assert_ne!(name_cmp("alpha", "Alpha"), Ordering::Equal);
    }

    /// The forgetting is the feature — but only between views, not between
    /// visits to the same one within a session.
    #[test]
    fn an_order_is_remembered_for_its_own_list_and_no_other() {
        let mut c = Chosen::default();
        c.set_order("roms:arcade:", "rating");
        assert_eq!(c.order("roms:arcade:").id, "rating");
        assert_eq!(c.order("roms:gb:").id, "name", "the next console inherited it");
    }

    /// Continue playing sets its own default; choosing something else sticks.
    #[test]
    fn a_default_order_yields_to_a_choice() {
        let mut c = Chosen::default();
        c.default_order("history::", "played");
        assert_eq!(c.order("history::").id, "played");
        c.set_order("history::", "name");
        c.default_order("history::", "played");
        assert_eq!(c.order("history::").id, "name", "the default overrode a choice");
    }
}

// The order of the left column.
//
// Separate from [`crate::gamesort`], which orders games. The two answer
// different questions and want different answers: a game list sorted by rating
// is a question you asked about one console and stop caring about when you
// leave it, so that one is deliberately forgotten. The order of the column is
// the shape of the whole app — you learn where things are in it — so this one
// is remembered, on disk.
//
// It also had no order anybody chose. The server hands consoles and
// collections back by size, so the column opened on "Arcade Fighting, 322" and
// buried "Best of nes" thirty rows down, with no way to say otherwise.

use crate::gamelist::name_cmp;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// One entry in the left column, of either kind.
///
/// The two kinds share the idea but not the fields: a console knows whether an
/// emulator is installed, a collection knows how much of it is downloaded, and
/// neither knows the other's. The unused ones are simply zero.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PickerRow {
    pub name: String,
    #[serde(default)]
    pub rom_count: i64,
    /// How many of its games are on this machine.
    #[serde(default)]
    pub local_count: i64,
    #[serde(default)]
    pub is_favorite: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct PickerOrder {
    pub id: &'static str,
    pub label: &'static str,
    pub dir: i8,
}

/// Orders offered for collections, in the sequence the menu lists them.
pub const COLLECTION_ORDERS: &[PickerOrder] = &[
    PickerOrder { id: "name", label: "Name", dir: 1 },
    PickerOrder { id: "count", label: "Most games", dir: -1 },
    PickerOrder { id: "fewest", label: "Fewest games", dir: 1 },
    PickerOrder { id: "here", label: "Most downloaded", dir: -1 },
];

/// What a kind of list may be ordered by.
///
/// Consoles get nothing, and that is the answer rather than an omission. There
/// are thirty-five of them and they do not change, so the column is something
/// you learn the shape of — which a button that reshuffles it works against.
/// Collections are different: there are twenty-seven, they arrive from the
/// server in size order, and which of them you want at the top depends on what
/// you are doing.
pub fn orders_for(kind: &str) -> &'static [PickerOrder] {
    match kind {
        "collections" => COLLECTION_ORDERS,
        _ => &[],
    }
}

/// The chosen order per kind, remembered across restarts.
///
/// Name, not size. The server's own order is by count, which is why every list
/// in this app used to open on whichever console happens to have the most ROMs
/// in it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PickerOrders(#[serde(default)] pub BTreeMap<String, String>);

impl PickerOrders {
    /// The order for a kind, or the first one it offers. `None` for a kind
    /// with no orders at all, which is how a caller knows not to draw a button.
    pub fn get(&self, kind: &str) -> Option<&'static PickerOrder> {
        let offered = orders_for(kind);
        self.0
            .get(kind)
            .and_then(|id| offered.iter().find(|o| o.id == id))
            .or_else(|| offered.first())
    }

    pub fn set(&mut self, kind: &str, id: &str) {
        self.0.insert(kind.to_owned(), id.to_owned());
    }
}

/// Alphabetically, for the lists that get no choice.
pub fn by_name(rows: &[PickerRow]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| name_cmp(&rows[a].name, &rows[b].name));
    order
}

/// Which entries to draw, in the order to draw them.
///
/// Indices rather than rows, so the caller's own list is never disturbed —
/// re-sorting it in place would compound across redraws instead of replacing.
pub fn sort(rows: &[PickerRow], order_id: Option<&str>) -> Vec<usize> {
    let Some(order) = order_id.and_then(|id| COLLECTION_ORDERS.iter().find(|o| o.id == id)) else {
        return by_name(rows);
    };
    let mut out: Vec<usize> = (0..rows.len()).collect();
    out.sort_by(|&a, &b| {
        let (a, b) = (&rows[a], &rows[b]);
        // Favorites first whatever else is chosen — a starred collection is
        // one you said you wanted at hand.
        if a.is_favorite != b.is_favorite {
            return if a.is_favorite { Ordering::Less } else { Ordering::Greater };
        }
        let keyed = match order.id {
            "count" | "fewest" => a.rom_count.cmp(&b.rom_count),
            "here" => a.local_count.cmp(&b.local_count),
            _ => name_cmp(&a.name, &b.name),
        };
        let keyed = if order.dir < 0 { keyed.reverse() } else { keyed };
        // A stable tie-break, or two collections with the same count swap
        // places between one redraw and the next.
        keyed.then_with(|| name_cmp(&a.name, &b.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, rom_count: i64, local_count: i64) -> PickerRow {
        PickerRow { name: name.to_owned(), rom_count, local_count, is_favorite: false }
    }

    /// The same rows `ui/test/picker-order.test.js` uses.
    fn cols() -> Vec<PickerRow> {
        vec![row("Alpha", 500, 5), row("Best of nes", 12, 12), row("Beta", 90, 900)]
    }

    fn names(rows: &[PickerRow], order: Option<&str>) -> Vec<String> {
        sort(rows, order).into_iter().map(|i| rows[i].name.clone()).collect()
    }

    /// Thirty-five consoles that never change is a column you learn the shape
    /// of, and a button that reshuffles it works against that.
    #[test]
    fn consoles_are_alphabetical_with_no_order_to_choose() {
        let consoles = vec![row("Nintendo 64", 0, 0), row("Arcade", 0, 0), row("Game Boy", 0, 0)];
        let sorted: Vec<&str> = by_name(&consoles).into_iter().map(|i| consoles[i].name.as_str()).collect();
        assert_eq!(sorted, ["Arcade", "Game Boy", "Nintendo 64"]);
        assert!(orders_for("platforms").is_empty(), "the consoles kept a menu");
    }

    #[test]
    fn collections_start_under_name_not_size() {
        assert_eq!(PickerOrders::default().get("collections").map(|o| o.id), Some("name"));
    }

    /// Unlike the game sort, which is deliberately forgotten: the order of a
    /// game list is a question about one console, the order of the column is
    /// the shape of the app.
    #[test]
    fn the_chosen_order_is_remembered() {
        let mut orders = PickerOrders::default();
        orders.set("collections", "count");
        assert_eq!(orders.get("collections").map(|o| o.id), Some("count"));

        let toml = toml::to_string(&orders).expect("serialising");
        let back: PickerOrders = toml::from_str(&toml).expect("parsing");
        assert_eq!(back.get("collections").map(|o| o.id), Some("count"), "it was forgotten on reload");
    }

    #[test]
    fn most_games_first_and_fewest_the_other_way() {
        let cols = cols();
        assert_eq!(
            sort(&cols, Some("count")).into_iter().map(|i| cols[i].rom_count).collect::<Vec<_>>(),
            [500, 90, 12]
        );
        assert_eq!(
            sort(&cols, Some("fewest")).into_iter().map(|i| cols[i].rom_count).collect::<Vec<_>>(),
            [12, 90, 500]
        );
    }

    #[test]
    fn what_is_downloaded_can_come_first() {
        let cols = cols();
        assert_eq!(names(&cols, Some("here"))[0], "Beta");
    }

    /// A starred collection is one you said you wanted at hand, so it stays at
    /// the top whatever else is chosen.
    #[test]
    fn favorites_stay_on_top_of_any_order() {
        let starred = vec![
            PickerRow { name: "Zebra".into(), rom_count: 1, local_count: 0, is_favorite: true },
            row("Alpha", 500, 500),
        ];
        for order in COLLECTION_ORDERS {
            assert_eq!(names(&starred, Some(order.id))[0], "Zebra", "not first under {}", order.id);
        }
    }

    /// An order id saved by a newer build, or a stale one, must not scramble
    /// the column — it falls back to the alphabet.
    #[test]
    fn an_unknown_order_falls_back_to_the_alphabet() {
        let cols = cols();
        assert_eq!(names(&cols, Some("nonsense")), ["Alpha", "Best of nes", "Beta"]);
    }
}

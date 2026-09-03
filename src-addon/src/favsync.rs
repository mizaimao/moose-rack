//! Making the handheld's stars and the server's stars agree.
//!
//! Two mutable sides, no server-side negotiation to lean on — `/api/sync` is
//! saves only. So the rule has to be decided here.
//!
//! **Remember what the last sync agreed on.** With a baseline, every
//! difference explains itself: a game starred here that the baseline has not
//! seen was starred here since, and belongs on the server; a game the baseline
//! has that this side has lost was unstarred here, and should go from the
//! server too. Without one, the only safe move is to merge everything, and
//! unstarring never travels — you take a star off on the handheld and the next
//! sync puts it straight back.
//!
//! And because a star is a *boolean*, a three-way merge has no conflicts to
//! resolve. There are only two values: if both sides moved away from the
//! baseline, they both moved to the same place, and they already agree.

use std::collections::{BTreeMap, BTreeSet};

/// One thing that has to happen for the two sides to match.
///
/// **Declaration order is display order** — `derive(Ord)` takes it from here
/// and `reconcile` sorts by it. Arrivals before departures, and this device
/// before the server, so the rows that change what you see on the handheld are
/// the ones at the top.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Move {
    /// The server has it; star it on this device.
    StarHere(i64),
    /// The server lost it; take the star off here.
    UnstarHere(i64),
    /// Starred here since the last sync; put it on the server.
    StarOnServer(i64),
    /// Unstarred here since the last sync; take it off the server.
    UnstarOnServer(i64),
}

impl Move {
    pub fn rom_id(self) -> i64 {
        match self {
            Move::StarHere(id)
            | Move::UnstarHere(id)
            | Move::StarOnServer(id)
            | Move::UnstarOnServer(id) => id,
        }
    }

    /// Does carrying this out change the server?
    pub fn touches_server(self) -> bool {
        matches!(self, Move::StarOnServer(_) | Move::UnstarOnServer(_))
    }
}

/// What to do about one collection, given both sides and what they last agreed.
///
/// `known` is the set of games this device actually has. A star on the server
/// for a game that is not on the card is left alone rather than reported as
/// missing — the card holds a subset of the library, and treating "not here"
/// as "unstarred here" would strip the server of every star for every game the
/// handheld does not carry.
pub fn reconcile(
    here: &BTreeSet<i64>,
    server: &BTreeSet<i64>,
    baseline: &BTreeSet<i64>,
    known: &BTreeSet<i64>,
) -> Vec<Move> {
    let mut moves = Vec::new();
    for id in here.union(server).copied().collect::<BTreeSet<_>>() {
        let on_here = here.contains(&id);
        let on_server = server.contains(&id);
        let was = baseline.contains(&id);
        match (on_here, on_server) {
            (true, true) | (false, false) => {}
            // Only the server has it.
            (false, true) if !known.contains(&id) => {}
            (false, true) if was => moves.push(Move::UnstarOnServer(id)),
            (false, true) => moves.push(Move::StarHere(id)),
            // Only this device has it.
            (true, false) if was => moves.push(Move::UnstarHere(id)),
            (true, false) => moves.push(Move::StarOnServer(id)),
        }
    }
    // Grouped by what happens, the way the save plan groups its lines: a
    // person reading this wants "three coming down, one going up", not a list
    // interleaved by an id they never see.
    moves.sort();
    moves
}

/// What the baseline becomes once a set of moves has been carried out.
///
/// Written *after* the work, not before: a run that dies halfway leaves the
/// old baseline, and the next run works out the same moves again. Saving it
/// first would record agreement that never happened, and the half that did not
/// run would look like a deliberate unstarring next time — which deletes.
pub fn settled(here: &BTreeSet<i64>, server: &BTreeSet<i64>, moves: &[Move]) -> BTreeSet<i64> {
    let mut out: BTreeSet<i64> = here.union(server).copied().collect();
    for m in moves {
        match m {
            Move::StarHere(_) | Move::StarOnServer(_) => {}
            Move::UnstarHere(id) | Move::UnstarOnServer(id) => {
                out.remove(id);
            }
        }
    }
    out
}

/// The baseline for every collection, as it goes to disk.
///
/// Plain JSON in the addon's own directory. It is a cache of an agreement, not
/// data — losing it costs one over-merged sync, not a star.
#[derive(Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Baseline {
    #[serde(default)]
    pub agreed: BTreeMap<String, BTreeSet<i64>>,
}

impl Baseline {
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let tmp = path.with_extension("json.moose");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn of(&self, collection: &str) -> BTreeSet<i64> {
        self.agreed.get(collection).cloned().unwrap_or_default()
    }

    /// Has this collection ever been synced?
    ///
    /// Asked separately from `of`, because an empty baseline and a missing one
    /// mean opposite things: the first says "we agreed there were none", the
    /// second says "we have never looked".
    pub fn seen(&self, collection: &str) -> bool {
        self.agreed.contains_key(collection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[i64]) -> BTreeSet<i64> {
        ids.iter().copied().collect()
    }

    /// Everything on the card, for the tests that are not about that.
    fn all() -> BTreeSet<i64> {
        set(&[1, 2, 3, 4, 5, 6, 7, 8, 9])
    }

    #[test]
    fn sides_that_agree_produce_no_work() {
        assert!(reconcile(&set(&[1, 2]), &set(&[1, 2]), &set(&[1, 2]), &all()).is_empty());
    }

    #[test]
    fn a_star_added_on_the_web_arrives_on_the_handheld() {
        let moves = reconcile(&set(&[1]), &set(&[1, 2]), &set(&[1]), &all());
        assert_eq!(moves, vec![Move::StarHere(2)]);
    }

    #[test]
    fn a_star_added_on_the_handheld_reaches_the_server() {
        let moves = reconcile(&set(&[1, 2]), &set(&[1]), &set(&[1]), &all());
        assert_eq!(moves, vec![Move::StarOnServer(2)]);
    }

    #[test]
    fn unstarring_on_the_handheld_travels_instead_of_coming_back() {
        // The whole reason for the baseline. Without it, "here has 1, server
        // has 1 and 2" reads as "the server knows something we do not", and
        // the star you just took off is put straight back on.
        let moves = reconcile(&set(&[1]), &set(&[1, 2]), &set(&[1, 2]), &all());
        assert_eq!(moves, vec![Move::UnstarOnServer(2)]);
    }

    #[test]
    fn unstarring_on_the_web_travels_too() {
        let moves = reconcile(&set(&[1, 2]), &set(&[1]), &set(&[1, 2]), &all());
        assert_eq!(moves, vec![Move::UnstarHere(2)]);
    }

    #[test]
    fn both_sides_starring_the_same_game_is_not_a_conflict() {
        // Only two values, so "both changed" means both changed to the same
        // thing. There is nothing to ask a person about.
        assert!(reconcile(&set(&[1]), &set(&[1]), &set(&[]), &all()).is_empty());
    }

    #[test]
    fn both_sides_unstarring_the_same_game_is_not_a_conflict_either() {
        assert!(reconcile(&set(&[]), &set(&[]), &set(&[1]), &all()).is_empty());
    }

    #[test]
    fn a_star_for_a_game_this_card_does_not_carry_is_left_alone() {
        // The card holds a subset of the library. Reading "not on this card"
        // as "unstarred here" would strip the server of every star for every
        // game the handheld does not have — which, on a 4GB card, is most of
        // them.
        let known = set(&[1]);
        let moves = reconcile(&set(&[1]), &set(&[1, 500]), &set(&[1, 500]), &known);
        assert!(moves.is_empty(), "wanted to touch a game that is not here: {moves:?}");
    }

    #[test]
    fn the_very_first_sync_merges_rather_than_deletes() {
        // No baseline: nothing can be read as a deletion, so both sides keep
        // everything they have and each gains what the other had.
        let moves = reconcile(&set(&[1, 2]), &set(&[2, 3]), &set(&[]), &all());
        assert_eq!(moves, vec![Move::StarHere(3), Move::StarOnServer(1)]);
    }

    #[test]
    fn what_is_agreed_afterwards_is_what_both_sides_will_hold() {
        let here = set(&[1, 2]);
        let server = set(&[2, 3]);
        let moves = reconcile(&here, &server, &set(&[]), &all());
        assert_eq!(settled(&here, &server, &moves), set(&[1, 2, 3]));

        // and a run that removed things records the removal
        let here = set(&[1]);
        let server = set(&[1, 2]);
        let moves = reconcile(&here, &server, &set(&[1, 2]), &all());
        assert_eq!(settled(&here, &server, &moves), set(&[1]));
    }

    #[test]
    fn a_baseline_that_has_never_been_written_is_not_an_empty_one() {
        // "We agreed there were none" and "we have never looked" lead to
        // opposite decisions about every star on the server.
        let b = Baseline::default();
        assert!(!b.seen("34"));
        let mut b = Baseline::default();
        b.agreed.insert("34".into(), BTreeSet::new());
        assert!(b.seen("34"));
        assert!(b.of("34").is_empty());
    }

    #[test]
    fn a_baseline_survives_a_round_trip() {
        let dir = std::env::temp_dir().join("moose-favsync-baseline");
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("favorites.json");
        let mut b = Baseline::default();
        b.agreed.insert("34".into(), set(&[1, 2, 3]));
        b.save(&p).unwrap();
        assert_eq!(Baseline::load(&p).of("34"), set(&[1, 2, 3]));
        assert!(!p.with_extension("json.moose").exists(), "left its temporary behind");
    }

    #[test]
    fn a_missing_or_corrupt_baseline_reads_as_never_synced() {
        // Losing it costs one over-merged sync. Refusing to run because a
        // cache file is unreadable costs the feature.
        let dir = std::env::temp_dir().join("moose-favsync-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("favorites.json");
        assert!(!Baseline::load(&p).seen("34"));
        std::fs::write(&p, "{not json").unwrap();
        assert!(!Baseline::load(&p).seen("34"));
    }

    #[test]
    fn only_two_of_the_four_moves_reach_the_server() {
        assert!(Move::StarOnServer(1).touches_server());
        assert!(Move::UnstarOnServer(1).touches_server());
        assert!(!Move::StarHere(1).touches_server());
        assert!(!Move::UnstarHere(1).touches_server());
    }
}

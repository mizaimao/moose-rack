//! The stars sync, end to end.
//!
//! [`crate::favsync`] decides *what* should happen, [`crate::eslist`] can
//! write ES's files and [`crate::favmap`] joins numbers to paths. This puts
//! the three together and is the only part that talks to the server.
//!
//! Which of ES's two kinds of list a collection belongs to is decided by its
//! name, because that is how this card is already arranged:
//!
//! * `★ Best of snes` — the stars in `/userdata/roms/snes/gamelist.xml`
//! * `★ Favourites` — the stars in every gamelist on the card
//! * anything else — `collections/custom-<name>.cfg`
//!
//! Only hand-made collections take part. A smart collection is a stored filter
//! and a virtual one is grouped on the fly; neither has a membership the
//! server could write, so a plan that included them would fail on every row.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use moose_rack::api::{Client, Collection};
use moose_rack::cache::Cache;
use moose_rack::platform::Platform;

use crate::eslist::{CollectionFile, Gamelist};
use crate::favmap::{EsPaths, Known};
use crate::favsync::{Baseline, Move, reconcile, settled};

/// Where a collection lives on the handheld.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Held {
    /// ES's own stars, in these system folders. One folder for a per-system
    /// list; every folder on the card for the library-wide one.
    Stars(Vec<String>),
    /// A `custom-<name>.cfg`.
    File,
}

/// The prefix this library uses for its per-system starred lists.
const PER_SYSTEM: &str = "★ Best of ";

/// The one list that means every star on the card, whatever the system.
///
/// By exact name. It used to be "any list the server marks `is_favorite`",
/// and moose-service marks every list whose name starts with ★. A tenth
/// starred list with another name -- `★ Shmups` -- would have been synced
/// against every star in every gamelist on the card, in both directions.
pub const ALL_STARS: &str = "★ Favourites";

/// More unstars than this in one run are refused without `--anyway`.
pub const MAX_UNSTARS: usize = 20;

/// Taking more than half a list's games is refused too, once it is more than
/// this many. Two is an ordinary afternoon; half of a list at once is what a
/// gamelist that failed to read, or a card swapped for another, looks like.
const FEW: usize = 2;

/// Which kind of ES list a collection maps onto.
///
/// `folders` is what the card actually has, so the library-wide list covers
/// exactly the systems present rather than every system the server knows.
///
/// The platform is passed in rather than read from `platform::current()`: a
/// test built on this Mac *is* the macOS platform, so a global lookup would
/// have the tests agreeing with a mapping the handheld never uses.
pub fn held_as(c: &Collection, folders: &[String], platform: &dyn Platform) -> Held {
    if let Some(slug) = c.name.strip_prefix(PER_SYSTEM) {
        // `★ Best of sfc` on the server is `snes` on the card.
        return Held::Stars(vec![platform.save_folder(slug.trim())]);
    }
    if c.name == ALL_STARS {
        return Held::Stars(folders.to_vec());
    }
    Held::File
}

/// What one collection needs doing.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub held: Held,
    pub moves: Vec<Move>,
    /// What the baseline becomes once `moves` have been carried out.
    pub agreed: BTreeSet<i64>,
}

impl Item {
    pub fn to_server(&self) -> Vec<i64> {
        self.pick(|m| matches!(m, Move::StarOnServer(_)))
    }
    pub fn off_server(&self) -> Vec<i64> {
        self.pick(|m| matches!(m, Move::UnstarOnServer(_)))
    }
    fn pick(&self, f: impl Fn(&Move) -> bool) -> Vec<i64> {
        self.moves.iter().filter(|m| f(m)).map(|m| m.rom_id()).collect()
    }
    /// Stars this takes off, on either side.
    pub fn unstars(&self) -> usize {
        self.moves
            .iter()
            .filter(|m| matches!(m, Move::UnstarHere(_) | Move::UnstarOnServer(_)))
            .count()
    }
}

/// What one collection looked like on each side, whether or not it needs work.
///
/// Kept so "nothing to do" can be *checked* rather than believed. A matcher
/// that finds nothing on either side reports agreement just as loudly as one
/// that works, and the two are indistinguishable from the headline alone.
#[derive(Clone, Debug)]
pub struct Survey {
    pub name: String,
    pub here: usize,
    pub server: usize,
    /// How many of the server's are on this card at all.
    pub reachable: usize,
}

/// The whole plan, ready to be shown before anything moves, and complete
/// enough to be carried out later exactly as it was shown.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub items: Vec<Item>,
    /// Collections that already agree — counted, never listed.
    pub agreeing: usize,
    /// Every collection looked at, in the order they were looked at.
    pub surveyed: Vec<Survey>,
    /// Collections that need no work, and what they agree on.
    ///
    /// Recorded every time, even though nothing moves. Without it the
    /// baseline only ever learns about lists that happened to differ, so the
    /// *first* star taken off an agreeing list reads as a list never synced —
    /// and a never-synced list merges, which puts the star straight back. And
    /// a baseline left from before stable game ids, holding numbers that name
    /// nothing now, is replaced on the first run rather than kept for ever.
    pub already: Vec<(String, BTreeSet<i64>)>,
    /// The games on the card the plan was made against. Carrying it out needs
    /// their paths, and looking them up again could find a different card.
    pub known: Vec<Known>,
    /// Lists left out because their card side could not be read, with why.
    /// Never read as empty: that is every star in the list taken off.
    pub unread: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn total(&self) -> usize {
        self.items.iter().map(|i| i.moves.len()).sum()
    }

    /// One line for the top of the panel.
    pub fn headline(&self) -> String {
        let mut line = if self.is_empty() {
            match self.agreeing {
                0 => "no collections to sync".into(),
                n => format!("nothing to do — {n} lists already match"),
            }
        } else {
            let here = self
                .items
                .iter()
                .flat_map(|i| &i.moves)
                .filter(|m| !m.touches_server())
                .count();
            let there = self.total() - here;
            match (here, there) {
                (0, n) => format!("{n} to send"),
                (n, 0) => format!("{n} to apply here"),
                (a, b) => format!("{a} to apply here, {b} to send"),
            }
        };
        if !self.unread.is_empty() {
            let names: Vec<&str> = self
                .unread
                .iter()
                .map(|u| u.split_once(": ").map_or(u.as_str(), |(n, _)| n))
                .collect();
            line.push_str(&format!(" — could not read {}", names.join(", ")));
        }
        if self.too_many_unstars().is_some() {
            line.push_str(" — too many unstars, refused");
        }
        line
    }

    /// Why this plan must not be carried out without `--anyway`, if it must
    /// not.
    ///
    /// Every way this sync has gone wrong so far looked like a list emptying
    /// on one side: a gamelist block it could not parse, a file it could not
    /// read, a baseline full of ids from before a renumbering. Each would have
    /// been a mass unstar sent to the server and written to the card.
    pub fn too_many_unstars(&self) -> Option<String> {
        let total: usize = self.items.iter().map(Item::unstars).sum();
        // `agreed` is the list after the run, so before it held that plus
        // what the run takes off.
        let worst = self
            .items
            .iter()
            .filter(|i| i.unstars() > FEW && i.unstars() > i.agreed.len())
            .max_by_key(|i| i.unstars());
        if total <= MAX_UNSTARS && worst.is_none() {
            return None;
        }
        let mut why = format!("refused: this would take {total} stars off at once");
        if let Some(i) = worst {
            why.push_str(&format!(
                ", {} of the {} in {}",
                i.unstars(),
                i.unstars() + i.agreed.len(),
                i.name
            ));
        }
        why.push_str(". If that is meant, run moose-patch --stars-apply --anyway over ssh");
        Some(why)
    }

    /// Whether carrying this out writes a file EmulationStation keeps in
    /// memory: a gamelist, a collection file, or `es_settings.cfg`.
    pub fn writes_card(&self, es: &EsPaths) -> Result<bool> {
        if self.items.iter().any(|i| i.moves.iter().any(|m| !m.touches_server())) {
            return Ok(true);
        }
        Ok(crate::eslist::settings_showing(&es.settings, &self.files())?.is_some())
    }

    /// The lists held as collection files.
    fn files(&self) -> Vec<String> {
        self.items.iter().filter(|i| i.held == Held::File).map(|i| i.name.clone()).collect()
    }
}

/// Work out what would happen. Changes nothing.
pub async fn plan(
    client: &Client,
    cache: &Cache,
    es: &EsPaths,
    known: &[Known],
    baseline: &Baseline,
    platform: &dyn Platform,
) -> Result<Plan> {
    let _ = cache;
    let collections = client.collections().await.context("asking for the collections")?;
    Ok(plan_from(collections, es, known, baseline, platform))
}

/// [`plan`], given the server's collections.
pub fn plan_from(
    collections: Vec<Collection>,
    es: &EsPaths,
    known: &[Known],
    baseline: &Baseline,
    platform: &dyn Platform,
) -> Plan {
    let folders: Vec<String> = crate::favmap::by_folder(known).into_keys().collect();
    let on_card = crate::favmap::ids(known);
    let index = crate::favmap::by_file(known);

    let mut plan = Plan { known: known.to_vec(), ..Plan::default() };
    for c in collections {
        // Smart and virtual ones arrive from other endpoints, but a server
        // that starts mixing them in must not be trusted to keep them out.
        if c.is_smart || c.is_virtual {
            continue;
        }
        let held = held_as(&c, &folders, platform);
        let here = match &held {
            Held::Stars(in_folders) => stars_on_card(es, in_folders, &index),
            Held::File => members_of_file(es, &c.name, &index),
        };
        let here = match here {
            Ok(h) => h,
            Err(e) => {
                plan.unread.push(format!("{}: {e:#}", c.name));
                continue;
            }
        };
        let server: BTreeSet<i64> = c.rom_ids.iter().copied().collect();
        let base = if baseline.seen(&c.id) { baseline.of(&c.id) } else { BTreeSet::new() };
        let moves = reconcile(&here, &server, &base, &on_card);
        plan.surveyed.push(Survey {
            name: c.name.clone(),
            here: here.len(),
            server: server.len(),
            reachable: server.intersection(&on_card).count(),
        });
        let agreed = settled(&here, &server, &moves, &on_card);
        if moves.is_empty() {
            plan.agreeing += 1;
            plan.already.push((c.id.clone(), agreed));
            continue;
        }
        plan.items.push(Item { id: c.id, name: c.name, held, moves, agreed });
    }
    // Biggest first: the lists with real work in them are the ones worth
    // reading, and a plan is scrolled past, not studied.
    plan.items.sort_by(|a, b| b.moves.len().cmp(&a.moves.len()).then(a.name.cmp(&b.name)));
    plan
}

/// Write down what the lists that already agree agree on. Says whether the
/// baseline changed, so a caller knows whether to save it.
///
/// Safe from a look as well as a run: both sides already hold this, so there
/// is nothing a person has to accept first.
pub fn record_agreement(plan: &Plan, baseline: &mut Baseline) -> bool {
    let mut changed = false;
    for (id, agreed) in &plan.already {
        if baseline.agreed.get(id) != Some(agreed) {
            baseline.agreed.insert(id.clone(), agreed.clone());
            changed = true;
        }
    }
    changed
}

/// The rom ids ES has starred, across a set of system folders.
///
/// A folder with no gamelist has no stars. A gamelist that cannot be read is
/// an error for the list, not an empty one.
fn stars_on_card(
    es: &EsPaths,
    folders: &[String],
    index: &BTreeMap<(String, String), i64>,
) -> Result<BTreeSet<i64>> {
    let mut out = BTreeSet::new();
    for folder in folders {
        let Some(list) = Gamelist::load_if_present(&es.gamelist(folder))? else { continue };
        for file in list.favorites() {
            if let Some(id) = index.get(&(folder.clone(), file)) {
                out.insert(*id);
            }
        }
    }
    Ok(out)
}

/// The rom ids a `custom-*.cfg` lists. A missing file is an empty list; one
/// that cannot be read is an error.
fn members_of_file(
    es: &EsPaths,
    name: &str,
    index: &BTreeMap<(String, String), i64>,
) -> Result<BTreeSet<i64>> {
    let file = CollectionFile::load(&es.collection(name))?;
    Ok(file.entries.iter().filter_map(|p| index.get(&es.locate(p)?).copied()).collect())
}

/// What actually happened.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Report {
    pub applied_here: usize,
    pub sent: usize,
    pub files_written: usize,
    /// `es_settings.cfg` was changed so ES shows the collection files.
    pub shown: bool,
    /// One line per list that did not go through, saying which and why.
    pub failed: Vec<String>,
}

impl Report {
    /// The line for the panel and the log.
    pub fn summary(&self) -> String {
        let mut note = match (self.applied_here, self.sent) {
            (0, 0) => "nothing to change".to_owned(),
            (0, n) => format!("{n} sent"),
            (n, 0) => format!("{n} applied here"),
            (a, b) => format!("{a} applied here, {b} sent"),
        };
        if self.shown {
            note.push_str(" — EmulationStation told to show them");
        }
        note
    }
}

/// Every list that did not sync, with why: the ones that could not be read
/// and the ones that did not go through. The CLI exits 1 when this is not
/// empty, and the panel names them.
pub fn failures<'a>(plan: &'a Plan, report: &'a Report) -> Vec<&'a str> {
    plan.unread.iter().chain(&report.failed).map(String::as_str).collect()
}

/// Carry a plan out, as it was shown.
///
/// Refused outright when [`Plan::too_many_unstars`] objects and `anyway` is
/// not set: nothing is sent and nothing written.
///
/// The server goes first for each collection. If it refuses, that collection
/// is left entirely alone and its baseline is not moved, so the next run works
/// the same moves out again — which is what makes a failed sync a retry rather
/// than a silent divergence. Every file rewritten is copied into `backups`
/// first.
pub async fn carry_out(
    client: &Client,
    es: &EsPaths,
    plan: &Plan,
    baseline: &mut Baseline,
    backups: &Path,
    anyway: bool,
) -> Result<Report> {
    if !anyway && let Some(why) = plan.too_many_unstars() {
        bail!(why);
    }
    let mut report = Report::default();
    // The lists that already agree, written down first. They cost no requests
    // and no file writes, and recording them is what makes the next unstar
    // travel instead of coming back.
    record_agreement(plan, baseline);
    let by_id: BTreeMap<i64, &Known> = plan.known.iter().map(|k| (k.rom_id, k)).collect();
    let mut files = Vec::new();
    for item in &plan.items {
        if let Err(e) = send(client, item).await {
            report.failed.push(format!("{}: {e:#}", item.name));
            continue;
        }
        report.sent += item.to_server().len() + item.off_server().len();

        match apply_here(es, &by_id, item, backups) {
            Ok(written) => {
                report.files_written += written;
                report.applied_here += item.moves.iter().filter(|m| !m.touches_server()).count();
                baseline.agreed.insert(item.id.clone(), item.agreed.clone());
                if item.held == Held::File {
                    files.push(item.name.clone());
                }
            }
            Err(e) => report.failed.push(format!("{}: {e:#}", item.name)),
        }
    }
    match show(es, &files, backups) {
        Ok(shown) => report.shown = shown,
        Err(e) => report.failed.push(format!("showing the collections in EmulationStation: {e:#}")),
    }
    Ok(report)
}

async fn send(client: &Client, item: &Item) -> Result<()> {
    client.add_roms_to_collection(&item.id, &item.to_server()).await?;
    client.remove_roms_from_collection(&item.id, &item.off_server()).await?;
    Ok(())
}

/// Write the handheld's half. Returns how many files were rewritten.
fn apply_here(
    es: &EsPaths,
    by_id: &BTreeMap<i64, &Known>,
    item: &Item,
    backups: &Path,
) -> Result<usize> {
    let wanted: Vec<(&Known, bool)> = item
        .moves
        .iter()
        .filter(|m| !m.touches_server())
        .filter_map(|m| by_id.get(&m.rom_id()).map(|k| (*k, matches!(m, Move::StarHere(_)))))
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }

    match &item.held {
        Held::Stars(_) => {
            // Grouped by folder so each gamelist is read and written once,
            // however many of its games moved.
            let mut written = 0;
            let mut folders: BTreeMap<String, Vec<(&Known, bool)>> = BTreeMap::new();
            for (k, on) in wanted {
                folders.entry(k.folder.clone()).or_default().push((k, on));
            }
            for (folder, games) in folders {
                let mut list = Gamelist::load_or_empty(&es.gamelist(&folder))?;
                for (k, on) in games {
                    list.set_favorite(&k.rel_path(), on);
                }
                if list.save(backups)? {
                    written += 1;
                }
            }
            Ok(written)
        }
        Held::File => {
            let mut file = CollectionFile::load(&es.collection(&item.name))?;
            for (k, on) in wanted {
                let full = k.full_path(&es.roms);
                if on {
                    file.entries.insert(full);
                } else {
                    file.entries.remove(&full);
                }
            }
            Ok(file.save(backups)? as usize)
        }
    }
}

/// Make sure every collection file just synced is one ES will actually show.
///
/// It touches a setting rather than a list, and it is the step that gets
/// forgotten: a correct `custom-*.cfg` shows nothing at all until its name is
/// in `CollectionSystemsCustom`.
fn show(es: &EsPaths, names: &[String], backups: &Path) -> Result<bool> {
    let Some(next) = crate::eslist::settings_showing(&es.settings, names)? else {
        return Ok(false);
    };
    crate::eslist::write_settings(&es.settings, &next, backups)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection(id: &str, name: &str, favorite: bool, roms: &[i64]) -> Collection {
        Collection {
            id: id.into(),
            name: name.into(),
            description: None,
            rom_ids: roms.to_vec(),
            rom_count: roms.len() as i64,
            is_favorite: favorite,
            is_virtual: false,
            is_smart: false,
            kind: None,
            path_covers_small: Vec::new(),
        }
    }

    /// The handheld's mapping, named explicitly — see `held_as`.
    const FLIP: moose_rack::platform::knulli::Knulli = moose_rack::platform::knulli::Knulli;

    fn nowhere() -> Client {
        Client::with_auth("http://nowhere.invalid", "u", "p", None).unwrap()
    }

    #[test]
    fn a_best_of_list_is_the_stars_in_one_gamelist() {
        let folders = vec!["snes".to_owned(), "gb".to_owned()];
        assert_eq!(
            held_as(&collection("34", "★ Best of snes", false, &[]), &folders, &FLIP),
            Held::Stars(vec!["snes".to_owned()])
        );
    }

    #[test]
    fn a_best_of_list_named_for_the_servers_slug_finds_the_cards_folder() {
        // The server calls it `sfc`; the card calls it `snes`. This is the
        // same mapping the saves use, and getting it wrong means the stars
        // land in a gamelist that does not exist.
        let folders = vec!["snes".to_owned()];
        assert_eq!(
            held_as(&collection("34", "★ Best of sfc", false, &[]), &folders, &FLIP),
            Held::Stars(vec!["snes".to_owned()])
        );
    }

    #[test]
    fn an_ordinary_collection_is_a_file() {
        let folders = vec!["fbneo".to_owned()];
        assert_eq!(
            held_as(&collection("45", "Arcade Fighting", false, &[]), &folders, &FLIP),
            Held::File
        );
    }

    #[test]
    fn a_library_wide_favourites_list_covers_every_system_on_the_card() {
        let folders = vec!["snes".to_owned(), "gb".to_owned()];
        assert_eq!(
            held_as(&collection("7", ALL_STARS, true, &[]), &folders, &FLIP),
            Held::Stars(folders)
        );
    }

    /// moose-service marks every list whose name starts with ★ as a
    /// favourite. That made any new starred list the all-favourites list, to
    /// be synced against every star on the card.
    #[test]
    fn another_starred_list_is_its_own_collection_not_every_star() {
        let folders = vec!["snes".to_owned(), "gb".to_owned()];
        for name in ["★ Shmups", "Favourites", "★ favourites"] {
            assert_eq!(
                held_as(&collection("8", name, true, &[]), &folders, &FLIP),
                Held::File,
                "{name}"
            );
        }
    }

    fn item(name: &str, held: Held, moves: Vec<Move>, agreed: &[i64]) -> Item {
        Item {
            id: name.into(),
            name: name.into(),
            held,
            moves,
            agreed: agreed.iter().copied().collect(),
        }
    }

    #[test]
    fn the_headline_says_which_way_things_are_going() {
        let mut plan = Plan::default();
        plan.items.push(item(
            "★ Best of snes",
            Held::File,
            vec![Move::StarHere(1), Move::StarOnServer(2), Move::StarOnServer(3)],
            &[],
        ));
        assert_eq!(plan.headline(), "1 to apply here, 2 to send");
        assert_eq!(plan.total(), 3);

        let mut only_up = Plan::default();
        only_up.items.push(item("x", Held::File, vec![Move::StarOnServer(2)], &[]));
        assert_eq!(only_up.headline(), "1 to send");
    }

    #[test]
    fn a_plan_with_nothing_in_it_says_how_many_lists_agreed() {
        let plan = Plan { agreeing: 27, ..Plan::default() };
        assert_eq!(plan.headline(), "nothing to do — 27 lists already match");
        assert_eq!(Plan::default().headline(), "no collections to sync");
    }

    #[test]
    fn an_item_separates_what_goes_up_from_what_comes_off() {
        let i = item(
            "x",
            Held::File,
            vec![
                Move::StarHere(1),
                Move::StarOnServer(2),
                Move::UnstarOnServer(3),
                Move::UnstarHere(4),
            ],
            &[],
        );
        assert_eq!(i.to_server(), vec![2]);
        assert_eq!(i.off_server(), vec![3]);
        assert_eq!(i.unstars(), 2);
    }

    // --- planning ------------------------------------------------------------

    fn card(name: &str) -> (EsPaths, Vec<Known>) {
        // Named per test: these run in parallel, and one shared directory
        // means each one deletes the card the others are reading.
        let dir = std::env::temp_dir().join(format!("moose-favrun-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let es = EsPaths::under(&dir);
        std::fs::create_dir_all(es.roms.join("snes")).unwrap();
        std::fs::create_dir_all(&es.collections).unwrap();
        std::fs::write(
            es.gamelist("snes"),
            "<?xml version=\"1.0\"?>\n<gameList>\n\t<game>\n\t\t<path>./Chrono Trigger (USA).sfc</path>\n\t\t<name>Chrono Trigger</name>\n\t\t<desc>Time travel.</desc>\n\t</game>\n</gameList>\n",
        )
        .unwrap();
        let known = vec![
            snes(1, "", "Chrono Trigger (USA).sfc"),
            snes(2, "", "Secret of Mana (USA).sfc"),
        ];
        (es, known)
    }

    fn snes(rom_id: i64, rel_dir: &str, file: &str) -> Known {
        Known { rom_id, folder: "snes".into(), rel_dir: rel_dir.into(), file: file.into() }
    }

    fn backups(es: &EsPaths) -> std::path::PathBuf {
        es.roms.parent().unwrap().join("es-backup")
    }

    fn set_star(es: &EsPaths, folder: &str, rel_path: &str, on: bool) {
        let mut list = Gamelist::load_or_empty(&es.gamelist(folder)).unwrap();
        list.set_favorite(rel_path, on);
        list.save(&backups(es)).unwrap();
    }

    /// Audit item 18. A list that agreed kept whatever baseline it had for
    /// ever, so one written under the old positional ids -- numbers that name
    /// nothing now -- was never replaced, and an unstar on that list merged
    /// back instead of travelling.
    #[test]
    fn a_list_that_agrees_refreshes_a_stale_baseline() {
        let (es, known) = card("stale");
        set_star(&es, "snes", "Chrono Trigger (USA).sfc", true);
        let lists = || vec![collection("34", "★ Best of snes", true, &[1, 900])];
        let mut baseline = Baseline::default();
        baseline.agreed.insert("34".into(), [5653i64, 812].into());
        let plan = plan_from(lists(), &es, &known, &baseline, &FLIP);
        assert!(plan.items.is_empty(), "{:?}", plan.items);
        assert!(record_agreement(&plan, &mut baseline));
        assert_eq!(
            baseline.of("34"),
            [1i64].into(),
            "900 is not on this card and must not be recorded"
        );
        // Recorded once; the next look has nothing to change.
        let plan = plan_from(lists(), &es, &known, &baseline, &FLIP);
        assert!(!record_agreement(&plan, &mut baseline));

        // And now the unstar travels.
        set_star(&es, "snes", "Chrono Trigger (USA).sfc", false);
        let plan = plan_from(lists(), &es, &known, &baseline, &FLIP);
        assert_eq!(plan.items[0].moves, vec![Move::UnstarOnServer(1)]);
    }

    /// A `custom-*.cfg` that could not be read was an empty list: every game
    /// in it read as taken off here, and went off the server.
    #[test]
    fn a_collection_file_that_cannot_be_read_leaves_its_list_out() {
        let (es, known) = card("unreadable");
        std::fs::write(es.collection("Arcade Fighting"), b"/userdata/roms/snes/\xff.sfc\n").unwrap();
        let mut baseline = Baseline::default();
        baseline.agreed.insert("45".into(), [1i64, 2].into());
        let lists = vec![collection("45", "Arcade Fighting", false, &[1, 2])];
        let plan = plan_from(lists, &es, &known, &baseline, &FLIP);
        assert!(
            plan.items.is_empty(),
            "planned moves from a file it could not read: {:?}",
            plan.items
        );
        assert!(plan.already.is_empty(), "recorded agreement for it");
        assert_eq!(plan.unread.len(), 1);
        assert!(plan.unread[0].starts_with("Arcade Fighting: reading "), "{}", plan.unread[0]);
        assert!(plan.headline().contains("could not read Arcade Fighting"), "{}", plan.headline());
    }

    #[test]
    fn a_gamelist_that_cannot_be_read_leaves_its_list_out() {
        let (es, known) = card("unreadable-gamelist");
        std::fs::write(es.gamelist("snes"), b"<gameList>\xff</gameList>").unwrap();
        let mut baseline = Baseline::default();
        baseline.agreed.insert("34".into(), [1i64].into());
        let lists = vec![collection("34", "★ Best of snes", true, &[1])];
        let plan = plan_from(lists, &es, &known, &baseline, &FLIP);
        assert!(plan.items.is_empty());
        assert_eq!(plan.unread.len(), 1);
    }

    /// A starred game in a subfolder, and its twin at the top that is not.
    #[test]
    fn a_star_in_a_subfolder_is_that_game_and_not_its_twin() {
        let (es, mut known) = card("subfolder");
        std::fs::create_dir_all(es.roms.join("snes/Aftermarket")).unwrap();
        known.push(snes(3, "Aftermarket", "Chrono Trigger (USA).sfc"));
        set_star(&es, "snes", "Aftermarket/Chrono Trigger (USA).sfc", true);
        let lists = vec![collection("34", "★ Best of snes", true, &[])];
        let plan = plan_from(lists, &es, &known, &Baseline::default(), &FLIP);
        assert_eq!(plan.items[0].moves, vec![Move::StarOnServer(3)]);

        // And from a collection file, by its absolute path.
        let path = es.roms.join("snes/Aftermarket/Chrono Trigger (USA).sfc");
        std::fs::write(es.collection("Hacks"), format!("{}\n", path.display())).unwrap();
        let lists = vec![collection("9", "Hacks", false, &[])];
        let plan = plan_from(lists, &es, &known, &Baseline::default(), &FLIP);
        assert_eq!(plan.items[0].moves, vec![Move::StarOnServer(3)]);
    }

    // --- carrying it out -------------------------------------------------------

    #[tokio::test]
    async fn a_list_that_already_agrees_is_still_written_down() {
        // Otherwise the baseline only ever learns about lists that differed,
        // and the first star taken off an agreeing list looks like a list
        // that has never been synced — which merges, and puts it back.
        let mut baseline = Baseline::default();
        let (es, known) = card("already");
        let plan = Plan {
            agreeing: 1,
            already: vec![("34".to_owned(), [1i64, 2].into())],
            known,
            ..Plan::default()
        };
        let report =
            carry_out(&nowhere(), &es, &plan, &mut baseline, &backups(&es), false).await.unwrap();
        assert_eq!(report, Report::default(), "did work for a list with none to do");
        assert!(baseline.seen("34"));
        assert_eq!(baseline.of("34"), [1i64, 2].into());
    }

    /// A plan whose items only touch the card, so carrying it out needs no
    /// server.
    fn card_plan(known: Vec<Known>, items: Vec<Item>) -> Plan {
        Plan { items, known, ..Plan::default() }
    }

    fn best_of_snes(moves: Vec<Move>, agreed: &[i64]) -> Item {
        item("★ Best of snes", Held::Stars(vec!["snes".into()]), moves, agreed)
    }

    async fn run(es: &EsPaths, plan: &Plan, anyway: bool) -> Result<Report> {
        carry_out(&nowhere(), es, plan, &mut Baseline::default(), &backups(es), anyway).await
    }

    #[tokio::test]
    async fn a_star_from_the_server_lands_in_the_gamelist_and_keeps_the_scraping() {
        let (es, known) = card("star-lands");
        let plan = card_plan(known, vec![best_of_snes(vec![Move::StarHere(1)], &[1])]);
        let report = run(&es, &plan, false).await.unwrap();
        assert_eq!(report.files_written, 1);
        let list = Gamelist::load(&es.gamelist("snes")).unwrap();
        assert!(list.favorites().contains("Chrono Trigger (USA).sfc"));
        assert!(list.known().len() == 1, "invented a second block for a game it already had");
        // The gamelist it replaced was kept.
        let kept = std::fs::read_dir(backups(&es).join("snes")).unwrap().count();
        assert_eq!(kept, 1, "no backup of the gamelist that was rewritten");
    }

    #[tokio::test]
    async fn one_gamelist_is_written_once_however_many_of_its_games_moved() {
        let (es, known) = card("written-once");
        let moves = vec![Move::StarHere(1), Move::StarHere(2)];
        let plan = card_plan(known, vec![best_of_snes(moves, &[1, 2])]);
        let report = run(&es, &plan, false).await.unwrap();
        assert_eq!(report.files_written, 1, "wrote the file twice");
        assert_eq!(Gamelist::load(&es.gamelist("snes")).unwrap().favorites().len(), 2);
    }

    #[tokio::test]
    async fn a_collection_from_the_server_becomes_a_file_of_absolute_paths() {
        let (es, known) = card("as-file");
        let plan =
            card_plan(known, vec![item("Arcade Fighting", Held::File, vec![Move::StarHere(2)], &[2])]);
        run(&es, &plan, false).await.unwrap();
        let file = CollectionFile::load(&es.collection("Arcade Fighting")).unwrap();
        assert!(file.entries.contains(&es.roms.join("snes/Secret of Mana (USA).sfc")));
    }

    #[tokio::test]
    async fn the_collections_a_plan_touches_are_the_ones_es_is_told_to_show() {
        let (es, known) = card("shown");
        std::fs::write(
            &es.settings,
            "<?xml version=\"1.0\"?>\n<config>\n\t<string name=\"ThemeSet\" value=\"knulli\" />\n</config>\n",
        )
        .unwrap();
        let plan = card_plan(
            known,
            vec![
                item("Arcade Fighting", Held::File, vec![Move::StarHere(1)], &[1]),
                best_of_snes(vec![Move::StarHere(1)], &[1]),
            ],
        );
        assert!(plan.writes_card(&es).unwrap());
        let report = run(&es, &plan, false).await.unwrap();
        assert!(report.shown);
        let settings = std::fs::read_to_string(&es.settings).unwrap();
        let shown = crate::eslist::enabled_collections(&settings);
        assert_eq!(shown, vec!["Arcade Fighting".to_owned()]);
        assert!(
            !settings.contains("★ Best of snes"),
            "a starred list is not a custom collection — it is the stars themselves"
        );
        assert!(!run(&es, &plan, false).await.unwrap().shown, "rewrote settings that already said this");
    }

    /// Nothing is sent and nothing written when a plan takes off more than
    /// twenty stars, or more than half a list, unless told to.
    #[tokio::test]
    async fn a_mass_unstar_is_refused_without_anyway() {
        let (es, _) = card("mass-unstar");
        let known: Vec<Known> = (1..=30).map(|i| snes(i, "", &format!("G{i}.sfc"))).collect();
        for k in &known {
            set_star(&es, "snes", &k.file, true);
        }
        let before = std::fs::read_to_string(es.gamelist("snes")).unwrap();
        let unstars: Vec<Move> = (1..=21).map(Move::UnstarHere).collect();
        let rest: Vec<i64> = (22..=30).collect();
        let mut plan = card_plan(known, vec![best_of_snes(unstars.clone(), &rest)]);
        let mut baseline = Baseline::default();
        let e = carry_out(&nowhere(), &es, &plan, &mut baseline, &backups(&es), false)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("21 stars off"), "{e}");
        assert!(e.contains("--anyway"), "{e}");
        assert_eq!(std::fs::read_to_string(es.gamelist("snes")).unwrap(), before, "wrote the card");
        assert!(baseline.agreed.is_empty());
        assert!(plan.headline().contains("refused"), "{}", plan.headline());

        // Three of four in one list is more than half of it.
        let three = (1..=3).map(Move::UnstarOnServer).collect();
        plan.items = vec![item("Arcade Maze", Held::File, three, &[4])];
        let e = run(&es, &plan, false).await.unwrap_err().to_string();
        assert!(e.contains("3 of the 4 in Arcade Maze"), "{e}");

        // Two is an ordinary edit.
        plan.items = vec![item("Arcade Maze", Held::File, (1..=2).map(Move::UnstarHere).collect(), &[])];
        assert!(plan.too_many_unstars().is_none());

        // Told to, it goes.
        plan.items = vec![best_of_snes(unstars, &rest)];
        run(&es, &plan, true).await.unwrap();
        assert_eq!(Gamelist::load(&es.gamelist("snes")).unwrap().favorites().len(), 9);
    }

    /// A list the server refused is named with the reason, and its baseline
    /// does not move.
    #[tokio::test]
    async fn a_list_that_failed_is_named_with_why() {
        let (es, known) = card("failed");
        let plan =
            card_plan(known, vec![item("Arcade Fighting", Held::File, vec![Move::StarOnServer(1)], &[1])]);
        let mut baseline = Baseline::default();
        let report =
            carry_out(&nowhere(), &es, &plan, &mut baseline, &backups(&es), false).await.unwrap();
        assert_eq!(report.failed.len(), 1);
        assert!(
            report.failed[0].starts_with("Arcade Fighting: POST http://nowhere.invalid/api/collections/"),
            "{}",
            report.failed[0]
        );
        assert!(!baseline.seen("Arcade Fighting"), "moved the baseline of a list that did not go through");
    }
}

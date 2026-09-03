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
//! * `Arcade Fighting` — `collections/custom-Arcade Fighting.cfg`
//!
//! Only hand-made collections take part. A smart collection is a stored filter
//! and a virtual one is grouped on the fly; neither has a membership the
//! server could write, so a plan that included them would fail on every row.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
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
    /// list; every folder on the card for a library-wide one.
    Stars(Vec<String>),
    /// A `custom-<name>.cfg`.
    File,
}

/// The prefix this library uses for its per-system starred lists.
const PER_SYSTEM: &str = "★ Best of ";

/// Which kind of ES list a collection maps onto.
///
/// `folders` is what the card actually has, so a library-wide favourites list
/// covers exactly the systems present rather than every system RomM knows.
///
/// The platform is passed in rather than read from `platform::current()`: a
/// test built on this Mac *is* the macOS platform, so a global lookup would
/// have the tests agreeing with a mapping the handheld never uses.
pub fn held_as(c: &Collection, folders: &[String], platform: &dyn Platform) -> Held {
    match c.name.strip_prefix(PER_SYSTEM) {
        // `★ Best of sfc` on the server is `snes` on the card.
        Some(slug) => Held::Stars(vec![platform.save_folder(slug.trim())]),
        None if c.is_favorite => Held::Stars(folders.to_vec()),
        None => Held::File,
    }
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

/// The whole plan, ready to be shown before anything moves.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub items: Vec<Item>,
    /// Collections that already agree — counted, never listed.
    pub agreeing: usize,
    /// Every collection looked at, in the order they were looked at.
    pub surveyed: Vec<Survey>,
    /// Collections that need no work, and what they agree on.
    ///
    /// Recorded even though nothing moves. Without this the baseline only
    /// ever learns about lists that happened to differ, so the *first* star
    /// taken off an already-agreeing list reads as a list never synced — and
    /// a never-synced list merges, which puts the star straight back.
    pub already: Vec<(String, BTreeSet<i64>)>,
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
        if self.is_empty() {
            return match self.agreeing {
                0 => "no collections to sync".into(),
                n => format!("nothing to do — {n} lists already match"),
            };
        }
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
    let folders: Vec<String> = crate::favmap::by_folder(known).into_keys().collect();
    let on_card = crate::favmap::ids(known);
    let index = crate::favmap::by_file(known);

    let mut plan = Plan::default();
    for c in client.collections().await.context("asking for the collections")? {
        // Smart and virtual ones arrive from other endpoints, but a server
        // that starts mixing them in must not be trusted to keep them out.
        if c.is_smart || c.is_virtual {
            continue;
        }
        let held = held_as(&c, &folders, platform);
        let here = match &held {
            Held::Stars(in_folders) => stars_on_card(es, in_folders, &index)?,
            Held::File => members_of_file(es, &c.name, &index),
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
        if moves.is_empty() {
            plan.agreeing += 1;
            if !baseline.seen(&c.id) {
                plan.already.push((c.id.clone(), settled(&here, &server, &[])));
            }
            continue;
        }
        let agreed = settled(&here, &server, &moves);
        plan.items.push(Item { id: c.id, name: c.name, held, moves, agreed });
    }
    // Biggest first: the lists with real work in them are the ones worth
    // reading, and a plan is scrolled past, not studied.
    plan.items.sort_by(|a, b| b.moves.len().cmp(&a.moves.len()).then(a.name.cmp(&b.name)));
    let _ = cache;
    Ok(plan)
}

/// The rom ids ES has starred, across a set of system folders.
fn stars_on_card(
    es: &EsPaths,
    folders: &[String],
    index: &std::collections::BTreeMap<(String, String), i64>,
) -> Result<BTreeSet<i64>> {
    let mut out = BTreeSet::new();
    for folder in folders {
        let path = es.gamelist(folder);
        if !path.exists() {
            continue;
        }
        for file in Gamelist::load(&path)?.favorites() {
            if let Some(id) = index.get(&(folder.clone(), file)) {
                out.insert(*id);
            }
        }
    }
    Ok(out)
}

/// The rom ids a `custom-*.cfg` lists.
fn members_of_file(
    es: &EsPaths,
    name: &str,
    index: &std::collections::BTreeMap<(String, String), i64>,
) -> BTreeSet<i64> {
    let Ok(file) = CollectionFile::load(&es.collection(name)) else {
        return BTreeSet::new();
    };
    file.entries
        .iter()
        .filter_map(|p| {
            let folder = p.parent()?.file_name()?.to_string_lossy().into_owned();
            let file = p.file_name()?.to_string_lossy().into_owned();
            index.get(&(folder, file)).copied()
        })
        .collect()
}

/// What actually happened.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Report {
    pub applied_here: usize,
    pub sent: usize,
    pub files_written: usize,
    pub failed: Vec<String>,
}

/// Carry a plan out.
///
/// The server goes first for each collection. If it refuses, that collection
/// is left entirely alone and its baseline is not moved, so the next run works
/// the same moves out again — which is what makes a failed sync a retry rather
/// than a silent divergence.
pub async fn carry_out(
    client: &Client,
    es: &EsPaths,
    known: &[Known],
    plan: &Plan,
    baseline: &mut Baseline,
) -> Result<Report> {
    let mut report = Report::default();
    // The lists that already agree, written down first. They cost no requests
    // and no file writes, and recording them is what makes the next unstar
    // travel instead of coming back.
    for (id, agreed) in &plan.already {
        baseline.agreed.insert(id.clone(), agreed.clone());
    }
    for item in &plan.items {
        if let Err(e) = send(client, item).await {
            report.failed.push(format!("{}: {e:#}", item.name));
            continue;
        }
        report.sent += item.to_server().len() + item.off_server().len();

        match apply_here(es, known, item) {
            Ok(written) => {
                report.files_written += written;
                report.applied_here +=
                    item.moves.iter().filter(|m| !m.touches_server()).count();
                baseline.agreed.insert(item.id.clone(), item.agreed.clone());
            }
            Err(e) => report.failed.push(format!("{}: {e:#}", item.name)),
        }
    }
    Ok(report)
}

async fn send(client: &Client, item: &Item) -> Result<()> {
    client.add_roms_to_collection(&item.id, &item.to_server()).await?;
    client.remove_roms_from_collection(&item.id, &item.off_server()).await?;
    Ok(())
}

/// Write the handheld's half. Returns how many files were rewritten.
fn apply_here(es: &EsPaths, known: &[Known], item: &Item) -> Result<usize> {
    let by_id: std::collections::BTreeMap<i64, &Known> =
        known.iter().map(|k| (k.rom_id, k)).collect();
    let wanted: Vec<(&Known, bool)> = item
        .moves
        .iter()
        .filter(|m| !m.touches_server())
        .filter_map(|m| {
            by_id
                .get(&m.rom_id())
                .map(|k| (*k, matches!(m, Move::StarHere(_))))
        })
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }

    match &item.held {
        Held::Stars(_) => {
            // Grouped by folder so each gamelist is read and written once,
            // however many of its games moved.
            let mut written = 0;
            let mut folders: std::collections::BTreeMap<String, Vec<(&Known, bool)>> =
                Default::default();
            for (k, on) in wanted {
                folders.entry(k.folder.clone()).or_default().push((k, on));
            }
            for (folder, games) in folders {
                let path = es.gamelist(&folder);
                let mut list = Gamelist::load_or_empty(&path)?;
                for (k, on) in games {
                    list.set_favorite(&k.file, on);
                }
                if list.save()? {
                    written += 1;
                }
            }
            Ok(written)
        }
        Held::File => {
            let path = es.collection(&item.name);
            let mut file = CollectionFile::load(&path)?;
            for (k, on) in wanted {
                let full = k.full_path(&es.roms);
                if on {
                    file.entries.insert(full);
                } else {
                    file.entries.remove(&full);
                }
            }
            Ok(file.save()? as usize)
        }
    }
}

/// Make sure every collection with a file is one ES will actually show.
///
/// Separate from the sync because it touches a setting rather than a list, and
/// because it is the step that gets forgotten: a correct `custom-*.cfg` shows
/// nothing at all until its name is in `CollectionSystemsCustom`.
pub fn show_all(es: &EsPaths, plan: &Plan) -> Result<bool> {
    let wanted: Vec<String> = plan
        .items
        .iter()
        .filter(|i| i.held == Held::File)
        .map(|i| i.name.clone())
        .collect();
    if wanted.is_empty() {
        return Ok(false);
    }
    let settings = std::fs::read_to_string(&es.settings).unwrap_or_default();
    let Some(next) = crate::eslist::show_collections(&settings, &wanted) else {
        return Ok(false);
    };
    write_atomically(&es.settings, &next)?;
    Ok(true)
}

fn write_atomically(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("cfg.moose");
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
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
            held_as(&collection("7", "Favourites", true, &[]), &folders, &FLIP),
            Held::Stars(folders)
        );
    }

    #[test]
    fn the_headline_says_which_way_things_are_going() {
        let mut plan = Plan::default();
        plan.items.push(Item {
            id: "1".into(),
            name: "★ Best of snes".into(),
            held: Held::File,
            moves: vec![Move::StarHere(1), Move::StarOnServer(2), Move::StarOnServer(3)],
            agreed: BTreeSet::new(),
        });
        assert_eq!(plan.headline(), "1 to apply here, 2 to send");
        assert_eq!(plan.total(), 3);

        let mut only_up = Plan::default();
        only_up.items.push(Item {
            id: "1".into(),
            name: "x".into(),
            held: Held::File,
            moves: vec![Move::StarOnServer(2)],
            agreed: BTreeSet::new(),
        });
        assert_eq!(only_up.headline(), "1 to send");
    }

    #[test]
    fn a_plan_with_nothing_in_it_says_how_many_lists_agreed() {
        let plan =
            Plan { items: Vec::new(), agreeing: 27, surveyed: Vec::new(), already: Vec::new() };
        assert_eq!(plan.headline(), "nothing to do — 27 lists already match");
        assert_eq!(Plan::default().headline(), "no collections to sync");
    }

    #[test]
    fn an_item_separates_what_goes_up_from_what_comes_off() {
        let item = Item {
            id: "1".into(),
            name: "x".into(),
            held: Held::File,
            moves: vec![
                Move::StarHere(1),
                Move::StarOnServer(2),
                Move::UnstarOnServer(3),
                Move::UnstarHere(4),
            ],
            agreed: BTreeSet::new(),
        };
        assert_eq!(item.to_server(), vec![2]);
        assert_eq!(item.off_server(), vec![3]);
    }

    // --- writing the handheld's half ---------------------------------------

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
            Known { rom_id: 1, folder: "snes".into(), file: "Chrono Trigger (USA).sfc".into() },
            Known { rom_id: 2, folder: "snes".into(), file: "Secret of Mana (USA).sfc".into() },
        ];
        (es, known)
    }

    #[tokio::test]
    async fn a_list_that_already_agrees_is_still_written_down() {
        // Otherwise the baseline only ever learns about lists that differed,
        // and the first star taken off an agreeing list looks like a list
        // that has never been synced — which merges, and puts it back.
        let mut baseline = Baseline::default();
        let plan = Plan {
            items: Vec::new(),
            agreeing: 1,
            surveyed: Vec::new(),
            already: vec![("34".to_owned(), [1i64, 2].into())],
        };
        let (es, known) = card("already");
        let client = moose_rack::api::Client::with_auth("http://nowhere.invalid", "u", "p", None)
            .unwrap();
        let report = carry_out(&client, &es, &known, &plan, &mut baseline).await.unwrap();
        assert_eq!(report, Report::default(), "did work for a list with none to do");
        assert!(baseline.seen("34"));
        assert_eq!(baseline.of("34"), [1i64, 2].into());
    }

    #[test]
    fn a_star_from_the_server_lands_in_the_gamelist_and_keeps_the_scraping() {
        let (es, known) = card("star-lands");
        let item = Item {
            id: "34".into(),
            name: "★ Best of snes".into(),
            held: Held::Stars(vec!["snes".into()]),
            moves: vec![Move::StarHere(1)],
            agreed: BTreeSet::new(),
        };
        assert_eq!(apply_here(&es, &known, &item).unwrap(), 1);
        let list = Gamelist::load(&es.gamelist("snes")).unwrap();
        assert!(list.favorites().contains("Chrono Trigger (USA).sfc"));
        assert!(list.known().len() == 1, "invented a second block for a game it already had");
    }

    #[test]
    fn one_gamelist_is_written_once_however_many_of_its_games_moved() {
        let (es, known) = card("written-once");
        let item = Item {
            id: "34".into(),
            name: "★ Best of snes".into(),
            held: Held::Stars(vec!["snes".into()]),
            moves: vec![Move::StarHere(1), Move::StarHere(2)],
            agreed: BTreeSet::new(),
        };
        assert_eq!(apply_here(&es, &known, &item).unwrap(), 1, "wrote the file twice");
        assert_eq!(Gamelist::load(&es.gamelist("snes")).unwrap().favorites().len(), 2);
    }

    #[test]
    fn a_collection_from_the_server_becomes_a_file_of_absolute_paths() {
        let (es, known) = card("as-file");
        let item = Item {
            id: "45".into(),
            name: "Arcade Fighting".into(),
            held: Held::File,
            moves: vec![Move::StarHere(2)],
            agreed: BTreeSet::new(),
        };
        assert_eq!(apply_here(&es, &known, &item).unwrap(), 1);
        let file = CollectionFile::load(&es.collection("Arcade Fighting")).unwrap();
        assert!(file.entries.contains(&es.roms.join("snes/Secret of Mana (USA).sfc")));
    }

    #[test]
    fn moves_that_only_touch_the_server_write_nothing_on_the_card() {
        let (es, known) = card("server-only");
        let item = Item {
            id: "34".into(),
            name: "★ Best of snes".into(),
            held: Held::Stars(vec!["snes".into()]),
            moves: vec![Move::StarOnServer(1), Move::UnstarOnServer(2)],
            agreed: BTreeSet::new(),
        };
        assert_eq!(apply_here(&es, &known, &item).unwrap(), 0);
        assert!(Gamelist::load(&es.gamelist("snes")).unwrap().favorites().is_empty());
    }

    #[test]
    fn the_collections_a_plan_touches_are_the_ones_es_is_told_to_show() {
        let (es, _) = card("shown");
        std::fs::write(
            &es.settings,
            "<?xml version=\"1.0\"?>\n<config>\n\t<string name=\"ThemeSet\" value=\"knulli\" />\n</config>\n",
        )
        .unwrap();
        let plan = Plan {
            items: vec![
                Item {
                    id: "45".into(),
                    name: "Arcade Fighting".into(),
                    held: Held::File,
                    moves: vec![Move::StarHere(1)],
                    agreed: BTreeSet::new(),
                },
                Item {
                    id: "34".into(),
                    name: "★ Best of snes".into(),
                    held: Held::Stars(vec!["snes".into()]),
                    moves: vec![Move::StarHere(1)],
                    agreed: BTreeSet::new(),
                },
            ],
            agreeing: 0,
            surveyed: Vec::new(),
            already: Vec::new(),
        };
        assert!(show_all(&es, &plan).unwrap());
        let settings = std::fs::read_to_string(&es.settings).unwrap();
        let shown = crate::eslist::enabled_collections(&settings);
        assert_eq!(shown, vec!["Arcade Fighting".to_owned()]);
        assert!(
            !settings.contains("★ Best of snes"),
            "a starred list is not a custom collection — it is the stars themselves"
        );
        assert!(!show_all(&es, &plan).unwrap(), "rewrote settings that already said this");
    }
}

//! Starring a game so every device agrees about it.
//!
//! RomM has no per-game favorite. What it has is collections, one of which may
//! be flagged `is_favorite` — so "star this game" means "put it in that
//! collection", and the star is shared between devices for free, because the
//! collection is on the server.
//!
//! This library spells it a second way: nine hand-made `★ Best of …` lists,
//! one per system, already mirrored onto the handheld game-for-game. Both
//! spellings are honoured, and [`crate::cache::Cache::starred_collection`]
//! decides which one a given game belongs to.
//!
//! The order here is always **server first, cache second**. A star that lit up
//! locally and never reached the server is worse than one that failed to light
//! up: the first lies to you on this machine and is invisible on every other,
//! the second you can see and press again.

use anyhow::{Context, Result, bail};

use crate::api::{Client, Collection};
use crate::cache::{Cache, star_name};

/// Whether a game is starred, according to the cache.
pub fn is_starred(cache: &Cache, rom_id: i64) -> Result<bool> {
    Ok(cache.favorite_ids()?.contains(&rom_id))
}

/// Where a star for a game on this platform goes.
///
/// Worked out from the cache in one step, separately from the server call, so
/// a caller holding the cache behind a lock can let go of it before it starts
/// awaiting. Holding it across the request is not merely rude — `Cache` is not
/// `Sync`, and the future would not compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The list exists already.
    Have { id: String, name: String },
    /// This system has no starred list yet; starring makes one.
    Missing { platform: String },
}

pub fn target(cache: &Cache, platform: &str) -> Result<Target> {
    Ok(match cache.starred_collection(platform)? {
        Some((id, name)) => Target::Have { id, name },
        None => Target::Missing { platform: platform.to_owned() },
    })
}

/// What the server did, and what the cache should now record.
#[derive(Debug)]
pub struct Landed {
    /// The collection the game went into, or came out of.
    pub id: String,
    /// Set when the collection had to be made, so the caller can remember it.
    pub created: Option<Collection>,
}

/// Star a game on the server, or unstar it.
///
/// `Ok(None)` means there was nothing to do: unstarring a game whose system
/// has no starred list at all. Unstarring never creates one — an empty list
/// nobody asked for is not an improvement on no list.
pub async fn on_server(
    client: &Client,
    target: Target,
    rom_id: i64,
    starred: bool,
) -> Result<Option<Landed>> {
    let (id, name, created) = match target {
        Target::Have { id, name } => (id, name, None),
        Target::Missing { .. } if !starred => return Ok(None),
        Target::Missing { platform } => {
            if platform.trim().is_empty() {
                bail!("cannot star a game with no platform — there is no list to put it in");
            }
            let made = client
                .create_collection(&star_name(&platform), true)
                .await
                .with_context(|| format!("making a starred list for {platform}"))?;
            (made.id.clone(), made.name.clone(), Some(made))
        }
    };

    if starred {
        client
            .add_roms_to_collection(&id, &[rom_id])
            .await
            .with_context(|| format!("starring rom {rom_id} into {name}"))?;
    } else {
        client
            .remove_roms_from_collection(&id, &[rom_id])
            .await
            .with_context(|| format!("unstarring rom {rom_id} from {name}"))?;
    }
    Ok(Some(Landed { id, created }))
}

/// Write down what the server just did.
///
/// Always after the server, never before. A star that lit up locally and never
/// reached the server is worse than one that failed to light: the first lies
/// to you here and is invisible everywhere else, the second you can see and
/// press again.
pub fn record(cache: &mut Cache, landed: &Landed, rom_id: i64, starred: bool) -> Result<()> {
    if let Some(made) = &landed.created {
        cache.remember_collection(made)?;
    }
    cache.set_membership(&landed.id, rom_id, starred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Collection;

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

    /// Named per test: these run in parallel, and a directory shared between
    /// two of them means each wipes the database the other is reading.
    fn cache_with(name: &str, items: &[Collection]) -> Cache {
        let dir = std::env::temp_dir().join(format!("moose-rack-fav-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cache = Cache::open(&dir.join("cache.sqlite3")).unwrap();
        cache.replace_collections(items).unwrap();
        cache
    }

    #[test]
    fn a_star_lands_in_the_list_for_that_system() {
        // The whole point of the per-system arrangement: a SNES game must not
        // fall into "★ Best of nes" just because that list was found first.
        let cache = cache_with("per-system", &[
            collection("32", "★ Best of nes", false, &[1]),
            collection("34", "★ Best of snes", false, &[2]),
        ]);
        assert_eq!(cache.starred_collection("snes").unwrap().unwrap().0, "34");
        assert_eq!(cache.starred_collection("nes").unwrap().unwrap().0, "32");
    }

    #[test]
    fn a_system_with_no_list_of_its_own_gets_nothing_by_mistake() {
        // Not "★ Best of nes" — that would quietly file every Dreamcast game
        // under NES. Returning nothing is what makes `set` create the right
        // list instead.
        let cache = cache_with("no-list", &[collection("32", "★ Best of nes", false, &[1])]);
        assert!(cache.starred_collection("dreamcast").unwrap().is_none());
    }

    #[test]
    fn one_starred_list_for_everything_catches_every_system() {
        // A library that keeps a single "Favourites" is just as valid, and
        // there is no per-system list to prefer over it.
        let cache = cache_with("one-list", &[collection("7", "Favourites", true, &[1])]);
        for platform in ["snes", "dreamcast", "gba"] {
            assert_eq!(cache.starred_collection(platform).unwrap().unwrap().0, "7");
        }
    }

    #[test]
    fn the_servers_own_flag_beats_a_name_somebody_typed() {
        let cache = cache_with("flag-wins", &[
            collection("1", "★ hand-typed", false, &[1]),
            collection("7", "Favourites", true, &[2]),
        ]);
        assert_eq!(cache.starred_collection("snes").unwrap().unwrap().0, "7");
    }

    #[test]
    fn starring_shows_up_at_once_and_unstarring_takes_it_away() {
        // Write-through: the star has to light the moment it is pressed, not
        // at the next full sync.
        let mut cache = cache_with("write-through", &[collection("34", "★ Best of snes", false, &[2])]);
        assert!(!is_starred(&cache, 99).unwrap());
        cache.set_membership("34", 99, true).unwrap();
        assert!(is_starred(&cache, 99).unwrap());
        cache.set_membership("34", 99, false).unwrap();
        assert!(!is_starred(&cache, 99).unwrap());
    }

    #[test]
    fn the_count_is_recounted_rather_than_nudged() {
        // Pressing star twice on the same game must not make the list claim
        // two more members than it has.
        let mut cache = cache_with("recount", &[collection("34", "★ Best of snes", false, &[2])]);
        cache.set_membership("34", 99, true).unwrap();
        cache.set_membership("34", 99, true).unwrap();
        assert_eq!(
            cache.collection_size("34").unwrap(),
            2,
            "one existing member plus one starred"
        );
    }

    #[test]
    fn a_smart_or_virtual_list_is_never_the_target() {
        // Neither can hold a membership — the server has nowhere to write it —
        // so a star aimed at one would fail every time.
        let mut smart = collection("s1", "★ Recently added", false, &[1]);
        smart.is_smart = true;
        let mut virt = collection("v1", "★ Platformers", false, &[2]);
        virt.is_virtual = true;
        let cache = cache_with("smart-virtual", &[smart, virt]);
        assert!(cache.starred_collection("nes").unwrap().is_none());
    }
}

//! Turning the catalogue into something with a cursor in it.
//!
//! The patches tab is built by *reading the device*, not by declaring what
//! ought to be true. A row opens at whatever `state()` reports, so a device
//! somebody else set up, or one that has just taken a KNULLI update, tells you
//! where it actually stands the moment you open the app.

use crate::model::{Page, Row};
use crate::patch::{Patch, State};

/// The sync tab: status you can read at the top, then the things you can set
/// going.
pub fn sync(server: Option<&str>, status: &str, stars: &str) -> Page {
    Page::new(vec![
        Row::fact("server", "Server", server.unwrap_or("not configured")),
        Row::fact("status", "Status", status),
        // One action, not a push button and a pull button.
        //
        // The server decides direction per save — some are newer here, some
        // there, and a few are both — so "push" and "pull" are not choices a
        // person can sensibly make up front. Asking what *would* happen is,
        // and it moves nothing.
        Row::action(
            "refresh",
            "Refresh the game list",
            "Rebuilds this device's list of your games from the server. Saves are matched to \
             games by the server's own id, and a rescan there renumbers everything — so a \
             stale list makes every save look like a game the server has never heard of. \
             Do this first on a new device, and again if syncing claims everything is new.",
            "—",
        ),
        Row::action(
            "check",
            "See what would sync",
            "Scans the saves on this card, hands the list to the server, and shows what it \
             would do — which way each save would move, and where both sides changed since \
             the last sync. Nothing is transferred until you accept the plan.",
            "—",
        ),
        Row::action(
            "stars",
            "Sync favourites and collections",
            "Matches the games you have starred here against the collections on the server, both \
             ways. A star added on the web arrives here; one added here is sent back. What was \
             agreed last time is remembered, so taking a star off travels too instead of coming \
             straight back. Shows what it would do before it does anything.",
            stars,
        ),
        Row::action(
            "offline",
            "Take games offline",
            "Downloads chosen games from the server onto the card so they play with no \
             network. Not wired up yet.",
            "—",
        ),
    ])
}

/// The patches tab, read back off the device.
pub fn patches(patches: &[Patch]) -> Page {
    Page::new(
        patches
            .iter()
            .map(|patch| {
                let live = match patch.state() {
                    State::At(i) => Some(i),
                    State::Changed => None,
                };
                Row::dial(patch.id, patch.title, patch.detail, patch.option_names(), live)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue;
    use crate::patch::Paths;

    fn scratch(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("moose-rows-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Paths::new(dir)
    }

    #[test]
    fn nothing_is_queued_before_anything_is_touched() {
        // Opening the app must not propose a single change, whatever it finds.
        let paths = scratch("fresh");
        assert!(patches(&catalogue::all(&paths)).pending().is_empty());
        assert!(sync(None, "not synced yet", "not checked yet").pending().is_empty());
    }

    #[test]
    fn the_menu_opens_at_what_the_device_actually_is() {
        // Apply something behind the app's back, then build the menu: the row
        // has to come up already showing it, or the first thing the user does
        // is turn a dial that was already where they wanted it.
        let paths = scratch("reads-back");
        let all = catalogue::all(&paths);
        let hotkeys = all.iter().find(|p| p.id == "hotkeys").unwrap();
        hotkeys.apply(1).unwrap();

        let page = patches(&catalogue::all(&paths));
        let row = page.rows.iter().find(|r| r.id == "hotkeys").unwrap();
        assert_eq!(row.value(), "ON");
        assert!(!row.pending());
    }

    #[test]
    fn a_file_edited_behind_our_back_shows_as_changed() {
        let paths = scratch("drifted");
        let all = catalogue::all(&paths);
        all.iter().find(|p| p.id == "hotkeys").unwrap().apply(1).unwrap();
        // Somebody, or an update, rewrites the block.
        let conf = paths.knulli_conf();
        let text = std::fs::read_to_string(&conf).unwrap();
        std::fs::write(&conf, text.replace("global.retroarch", "# global.retroarch")).unwrap();

        let page = patches(&catalogue::all(&paths));
        let row = page.rows.iter().find(|r| r.id == "hotkeys").unwrap();
        assert!(row.adrift());
        assert_eq!(row.value(), "changed");
        assert!(!row.pending(), "still not queued until it is chosen");
    }

    #[test]
    fn every_patch_says_what_it_changes() {
        // The detail line is the only place the file it writes is recorded,
        // and an empty one makes the row unundoable by hand.
        let paths = scratch("details");
        for row in patches(&catalogue::all(&paths)).rows {
            assert!(
                row.detail.len() > 40,
                "{} needs a detail line saying what it touches",
                row.id
            );
        }
    }

    #[test]
    fn the_sync_tab_opens_on_something_you_can_press() {
        // Its first two rows are facts; the cursor has to have skipped them.
        assert!(
            sync(None, "not synced yet", "not checked yet")
                .selected()
                .is_some_and(|r| r.selectable())
        );
    }
}

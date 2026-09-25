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
             games through it, so a game missing from the list is a save that cannot sync. \
             Do this first on a new device, and again after games are added to the server. \
             The old list is kept until the new one is complete.",
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
            "conflicts",
            "Settle conflicts",
            "Saves changed here and on the server since they last agreed. Nothing was written \
             for these. Pick a side for each: ← keeps this device's, → keeps the server's. \
             Whichever copy is replaced is backed up first.",
            "none",
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
        // No "take games offline" here, on purpose. Pulling ROMs down is too
        // heavy for this device -- a quad A55 on Wi-Fi filling an exFAT card --
        // and the card is filled from the SSD instead (docs/card-prep.md). The
        // row sat here unwired for weeks and was pressed eight times.
    ])
}

/// The patches tab, read back off the device.
///
/// `knulli` is which OS this is, as a fact at the top. A patch is a bet about
/// files KNULLI ships, and an update can move any of them — so the version the
/// patches were read against belongs beside them, where somebody choosing one
/// can see it, and not only in `--status`.
pub fn patches(patches: &[Patch], knulli: &str) -> Page {
    let mut rows = vec![Row::fact("knulli", "KNULLI", knulli)];
    rows.extend(patches.iter().map(|patch| {
        let live = match patch.state() {
            State::At(i) => Some(i),
            State::Changed => None,
        };
        Row::dial(patch.id, patch.title, patch.detail, patch.option_names(), live)
    }));
    Page::new(rows)
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
        assert!(patches(&catalogue::all(&paths), "test image").pending().is_empty());
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

        let page = patches(&catalogue::all(&paths), "test image");
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

        let page = patches(&catalogue::all(&paths), "test image");
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
        // Facts are exempt: a fact is the thing it says, and there is nothing
        // to undo by hand at two in the morning.
        for row in patches(&catalogue::all(&paths), "test image").rows {
            if matches!(row.kind, crate::model::Kind::Fact { .. }) {
                continue;
            }
            assert!(
                row.detail.len() > 40,
                "{} needs a detail line saying what it touches",
                row.id
            );
        }
    }

    #[test]
    fn the_flip_is_never_offered_game_downloads() {
        let page = sync(None, "not synced yet", "not checked yet");
        assert!(
            page.rows.iter().all(|r| r.id != "offline"),
            "taking games offline is too heavy for the handheld"
        );
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

    #[test]
    fn the_patches_tab_says_which_knulli_and_does_not_open_on_it() {
        // The version is a fact, so the cursor must land past it -- and it has
        // to be there, because a patch list with no OS beside it is what let
        // `never-sleep` report itself on against a moved image.
        let paths = scratch("knulli-row");
        let page = patches(&catalogue::all(&paths), "scarab 2026/05/10 22:54");
        assert_eq!(page.rows[0].id, "knulli");
        assert_eq!(page.rows[0].value(), "scarab 2026/05/10 22:54");
        assert!(page.selected().is_some_and(|r| r.selectable()));
        assert_ne!(page.selected().map(|r| r.id.as_str()), Some("knulli"));
    }
}

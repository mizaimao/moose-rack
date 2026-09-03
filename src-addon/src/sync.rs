//! Syncing, as a state machine the interface can draw.
//!
//! Nothing here talks to the network. The worker thread does that and hands
//! back one of these states; keeping the two apart is what lets the whole of
//! the behaviour — including "the server said conflict" — be tested without a
//! server.
//!
//! The shape is fixed by one rule: **nothing moves until the plan has been
//! shown and accepted.** `negotiate` tells us what would happen before a byte
//! is transferred, so there is no reason to act first and report afterwards,
//! and every reason not to on a device whose only copy of a save is local.
//!
//! The plan itself — [`Action`], [`Line`], [`Review`] — moved to
//! `moose_rack::syncplan` when the desktop and Android app grew the same
//! review step. Both frontends show one plan and it is sorted and phrased in
//! one place; two copies of "which order do the rows go in" is two copies that
//! can disagree about which save you were looking at. Re-exported here so this
//! module still reads as the whole of syncing from the addon's side.

pub use moose_rack::syncplan::{Action, Line, Review};

/// Where a sync has got to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// Never asked.
    #[default]
    Idle,
    /// Scanning, or waiting on the server. `note` says which.
    Asking { note: String },
    /// A plan, waiting for a person.
    Ready(Review),
    /// Carrying it out.
    Running { done: usize, total: usize },
    /// Finished. `conflicts` is a count rather than the conflicts themselves:
    /// those are held beside the app so this stays cheap to clone and compare.
    Done { moved: usize, conflicts: usize, note: String },
    Failed(String),
}

impl Stage {
    /// What A does next, in words, or `None` when A does nothing.
    ///
    /// One place decides this so the help line, the prompt and the handler
    /// cannot drift apart — which is how "A is not wired to apply" happened.
    pub fn next_step(&self) -> Option<&'static str> {
        match self {
            Stage::Ready(review) if review.is_empty() => Some("see what would sync"),
            Stage::Ready(_) => Some("carry this out"),
            Stage::Idle | Stage::Done { .. } | Stage::Failed(_) => Some("see what would sync"),
            // Asking, Running: the worker has it.
            _ => None,
        }
    }

    /// Can a person start something right now?
    ///
    /// False while the network is busy, which is what stops a second press
    /// starting a second sync over the top of the first.
    pub fn is_busy(&self) -> bool {
        matches!(self, Stage::Asking { .. } | Stage::Running { .. })
    }

    /// The line under the tab, whatever state it is in.
    pub fn note(&self) -> String {
        match self {
            Stage::Idle => "not synced yet".into(),
            Stage::Asking { note } => note.clone(),
            Stage::Ready(review) => review.headline(),
            Stage::Running { done, total } => format!("{done} of {total}"),
            Stage::Done { note, .. } => note.clone(),
            Stage::Failed(why) => format!("failed: {why}"),
        }
    }
}

/// Where the favourites-and-collections sync has got to.
///
/// Its own type rather than a second use of [`Stage`]: that one's `Ready`
/// carries a save review, and the two syncs are independent — you can look at
/// what the stars would do without touching the saves, and the sync tab draws
/// both lines at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Stars {
    #[default]
    Idle,
    Asking(String),
    /// A plan, waiting for a person. `moves` is zero when both sides agree.
    Ready { headline: String, moves: usize },
    Done(String),
    Failed(String),
}

impl Stars {
    /// What A does next, in words, or `None` while the worker has it.
    ///
    /// One place decides this, so the help line and the handler cannot drift
    /// apart — the mistake that made A do nothing on the patches tab.
    pub fn next_step(&self) -> Option<&'static str> {
        match self {
            Stars::Ready { moves, .. } if *moves > 0 => Some("carry this out"),
            Stars::Ready { .. } | Stars::Idle | Stars::Done(_) | Stars::Failed(_) => {
                Some("see what would sync")
            }
            Stars::Asking(_) => None,
        }
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Stars::Asking(_))
    }

    pub fn note(&self) -> String {
        match self {
            Stars::Idle => "not checked yet".into(),
            Stars::Asking(note) => note.clone(),
            Stars::Ready { headline, .. } => headline.clone(),
            Stars::Done(note) => note.clone(),
            Stars::Failed(why) => format!("failed: {why}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moose_rack::api::{SyncOperation, SyncPlan};

    fn op(action: &str, name: &str, rom_id: i64) -> SyncOperation {
        SyncOperation {
            action: action.into(),
            rom_id,
            save_id: Some(rom_id),
            file_name: Some(name.into()),
            slot: Some("autosave".into()),
            emulator: Some("mgba".into()),
            reason: None,
            server_content_hash: None,
            server_updated_at: None,
        }
    }

    fn plan(ops: Vec<SyncOperation>, no_op: i64) -> SyncPlan {
        SyncPlan {
            session_id: Some(1),
            operations: ops,
            total_upload: 0,
            total_download: 0,
            total_conflict: 0,
            total_no_op: no_op,
        }
    }

    #[test]
    fn a_stage_with_an_empty_plan_offers_to_look_again_not_to_run_it() {
        // Carrying out nothing is not a thing to offer. Before, the help line
        // and the handler each decided this for themselves.
        let empty = Stage::Ready(Review::from_plan(&plan(vec![], 12)));
        assert_eq!(empty.next_step(), Some("see what would sync"));

        let something = Stage::Ready(Review::from_plan(&plan(vec![op("upload", "a.srm", 1)], 0)));
        assert_eq!(something.next_step(), Some("carry this out"));
    }

    #[test]
    fn nothing_is_offered_while_the_worker_is_busy() {
        assert_eq!(Stage::Asking { note: "scanning".into() }.next_step(), None);
        assert_eq!(Stage::Running { done: 1, total: 4 }.next_step(), None);
    }

    #[test]
    fn after_a_run_you_can_look_again() {
        // Including after a failure — a sync that failed on a dropped wifi
        // should be one button away from being retried, not a restart.
        let done = Stage::Done { moved: 4, conflicts: 1, note: "4 moved".into() };
        assert_eq!(done.next_step(), Some("see what would sync"));
        assert_eq!(
            Stage::Failed("no route to host".into()).next_step(),
            Some("see what would sync")
        );
    }

    #[test]
    fn a_busy_stage_refuses_to_start_another() {
        // Two syncs at once would race on the same files. The interface asks
        // this before it lets A do anything.
        assert!(!Stage::Idle.is_busy());
        assert!(Stage::Asking { note: "asking the server".into() }.is_busy());
        assert!(Stage::Running { done: 1, total: 9 }.is_busy());
        assert!(!Stage::Ready(Review::default()).is_busy());
        assert!(!Stage::Failed("no route to host".into()).is_busy());
        assert!(!Stage::Done { moved: 0, conflicts: 0, note: "done".into() }.is_busy());
    }

    #[test]
    fn every_stage_has_something_to_say() {
        // The sync tab always shows a line. "Nothing at all" reads as frozen.
        for stage in [
            Stage::Idle,
            Stage::Asking { note: "scanning saves".into() },
            Stage::Ready(Review::default()),
            Stage::Running { done: 2, total: 7 },
            Stage::Done { moved: 3, conflicts: 0, note: "3 saves pushed".into() },
            Stage::Failed("no route to host".into()),
        ] {
            assert!(!stage.note().is_empty(), "{stage:?} said nothing");
        }
        assert_eq!(Stage::Running { done: 2, total: 7 }.note(), "2 of 7");
        assert_eq!(
            Stage::Failed("no route to host".into()).note(),
            "failed: no route to host"
        );
    }

    #[test]
    fn the_stars_stage_offers_to_look_before_it_offers_to_act() {
        assert_eq!(Stars::Idle.next_step(), Some("see what would sync"));
        assert_eq!(
            Stars::Ready { headline: "2 to send".into(), moves: 2 }.next_step(),
            Some("carry this out")
        );
        // A plan with nothing in it is not a thing to carry out.
        assert_eq!(
            Stars::Ready { headline: "nothing to do".into(), moves: 0 }.next_step(),
            Some("see what would sync")
        );
        assert_eq!(Stars::Asking("looking".into()).next_step(), None);
    }

    #[test]
    fn the_stars_stage_always_has_something_to_say() {
        for stage in [
            Stars::Idle,
            Stars::Asking("reading the card".into()),
            Stars::Ready { headline: "1 to send".into(), moves: 1 },
            Stars::Done("1 sent".into()),
            Stars::Failed("no route to host".into()),
        ] {
            assert!(!stage.note().is_empty(), "{stage:?} said nothing");
        }
        assert_eq!(
            Stars::Failed("no route to host".into()).note(),
            "failed: no route to host"
        );
    }

    #[test]
    fn a_second_press_while_it_is_working_starts_nothing() {
        assert!(Stars::Asking("looking".into()).is_busy());
        assert!(!Stars::Idle.is_busy());
        assert!(!Stars::Ready { headline: "x".into(), moves: 1 }.is_busy());
    }
}

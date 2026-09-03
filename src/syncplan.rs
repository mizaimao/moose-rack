//! A save sync, described before it happens.
//!
//! `/api/sync/negotiate` answers what *would* move without moving anything, and
//! this turns that answer into rows a person can read. It lives here rather
//! than in either frontend because both of them show the same plan: the addon
//! on the handheld and the app on desktop and Android. Two copies of "which
//! order do the rows go in" is two copies that can disagree about which save
//! you were looking at.
//!
//! Nothing here talks to the network, so the whole of it — including "the
//! server said conflict" — is testable without a server.
//!
//! **Saves only.** `negotiate` has no opinion about save states; those are
//! compared against a local ledger by [`crate::statesync`] and are not in this
//! plan. Whoever shows it has to say so, or a plan reading "nothing to sync"
//! followed by four states moving looks like a lie.

use serde::Serialize;

use crate::api::{SyncOperation, SyncPlan};

/// What the server decided about one save.
///
/// **Declaration order is the display order** — `derive(Ord)` takes it from
/// here, and [`Review::from_plan`] sorts by it. Conflicts first because they
/// are the only rows that need a decision; pulls before pushes because a pull
/// overwrites something local and deserves the closer look.
#[derive(Copy, Clone, PartialEq, Eq, Debug, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Both sides moved since the last sync. Needs a person.
    Conflict,
    /// Theirs is newer.
    Download,
    /// Ours is newer.
    Upload,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Conflict => "conflict",
            Action::Upload => "push",
            Action::Download => "pull",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "upload" => Action::Upload,
            "download" => Action::Download,
            "conflict" => Action::Conflict,
            // `no_op` is the server saying "already agreed". Worth counting,
            // never worth a row.
            _ => return None,
        })
    }
}

/// One line of the plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    pub action: Action,
    pub title: String,
    /// The server's own words for why, when it gave any. Shown verbatim
    /// rather than paraphrased — it knows things this device does not.
    pub reason: Option<String>,
    pub rom_id: i64,
    pub save_id: Option<i64>,
}

/// A plan, ready to be shown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Review {
    pub lines: Vec<Line>,
    pub agreed: usize,
}

impl Review {
    /// Conflicts first, then pulls, then pushes.
    ///
    /// Not alphabetical: the only rows that need a decision are the conflicts,
    /// and a list that buries them under two hundred agreed pushes is a list
    /// nobody reads to the bottom of.
    pub fn from_plan(plan: &SyncPlan) -> Self {
        let mut lines: Vec<Line> = plan
            .operations
            .iter()
            .filter_map(Self::line_of)
            .collect();
        lines.sort_by(|a, b| a.action.cmp(&b.action).then(a.title.cmp(&b.title)));
        Review { lines, agreed: plan.total_no_op.max(0) as usize }
    }

    fn line_of(op: &SyncOperation) -> Option<Line> {
        let action = Action::parse(&op.action)?;
        Some(Line {
            action,
            title: op
                .file_name
                .clone()
                .unwrap_or_else(|| format!("rom {}", op.rom_id)),
            reason: op.reason.clone().filter(|r| !r.trim().is_empty()),
            rom_id: op.rom_id,
            save_id: op.save_id,
        })
    }

    pub fn count(&self, action: Action) -> usize {
        self.lines.iter().filter(|l| l.action == action).count()
    }

    /// Nothing to do. Distinct from "not asked yet".
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// One line for the top of the panel.
    pub fn headline(&self) -> String {
        if self.is_empty() {
            return match self.agreed {
                0 => "nothing to sync".into(),
                n => format!("nothing to do — {n} already match"),
            };
        }
        let mut parts = Vec::new();
        for action in [Action::Conflict, Action::Download, Action::Upload] {
            let n = self.count(action);
            if n > 0 {
                parts.push(format!("{n} to {}", action.label()));
            }
        }
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn conflicts_come_first() {
        // The only rows needing a decision are the conflicts. A list that
        // buries them under two hundred agreed pushes is one nobody reads to
        // the bottom of.
        let review = Review::from_plan(&plan(
            vec![
                op("upload", "Zelda.srm", 1),
                op("download", "Metroid.srm", 2),
                op("conflict", "Crash.srm", 3),
                op("upload", "Aria.srm", 4),
            ],
            0,
        ));
        let order: Vec<Action> = review.lines.iter().map(|l| l.action).collect();
        assert_eq!(
            order,
            vec![Action::Conflict, Action::Download, Action::Upload, Action::Upload]
        );
        // and alphabetical within a group
        assert_eq!(review.lines[2].title, "Aria.srm");
    }

    #[test]
    fn no_op_is_counted_and_never_shown() {
        // "Already agreed" is worth a number so the user can tell a working
        // sync from one that found nothing — but it is not worth 380 rows.
        let review = Review::from_plan(&plan(vec![op("no_op", "Same.srm", 1)], 380));
        assert!(review.is_empty());
        assert_eq!(review.agreed, 380);
        assert_eq!(review.headline(), "nothing to do — 380 already match");
    }

    #[test]
    fn an_empty_plan_says_so_rather_than_looking_broken() {
        let review = Review::from_plan(&plan(vec![], 0));
        assert_eq!(review.headline(), "nothing to sync");
    }

    #[test]
    fn the_headline_counts_each_kind() {
        let review = Review::from_plan(&plan(
            vec![
                op("upload", "a.srm", 1),
                op("upload", "b.srm", 2),
                op("download", "c.srm", 3),
                op("conflict", "d.srm", 4),
            ],
            5,
        ));
        assert_eq!(review.headline(), "1 to conflict, 1 to pull, 2 to push");
        assert_eq!(review.count(Action::Upload), 2);
    }

    #[test]
    fn an_operation_with_no_filename_still_names_something() {
        // Never an empty row: a blank line in a confirmation is worse than an
        // ugly one, because there is nothing to decide about.
        let mut o = op("upload", "", 77);
        o.file_name = None;
        let review = Review::from_plan(&plan(vec![o], 0));
        assert_eq!(review.lines[0].title, "rom 77");
    }

    #[test]
    fn the_servers_reason_survives_but_blank_ones_do_not() {
        let mut o = op("conflict", "Crash.srm", 1);
        o.reason = Some("both changed since 2026-08-20".into());
        let mut blank = op("upload", "Zelda.srm", 2);
        blank.reason = Some("   ".into());
        let review = Review::from_plan(&plan(vec![o, blank], 0));
        assert_eq!(
            review.lines[0].reason.as_deref(),
            Some("both changed since 2026-08-20")
        );
        assert_eq!(review.lines[1].reason, None);
    }

    /// The web side reads these rows, so the names it reads by are part of the
    /// contract rather than an implementation detail.
    #[test]
    fn a_line_serialises_under_the_names_the_page_looks_for() {
        let review = Review::from_plan(&plan(vec![op("conflict", "Crash.srm", 3)], 2));
        let json = serde_json::to_value(&review).unwrap();
        assert_eq!(json["agreed"], 2);
        assert_eq!(json["lines"][0]["action"], "conflict");
        assert_eq!(json["lines"][0]["title"], "Crash.srm");
        assert_eq!(json["lines"][0]["rom_id"], 3);
    }
}

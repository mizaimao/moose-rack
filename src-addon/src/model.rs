//! What the menu holds, apart from how it is drawn.
//!
//! The shape of this file is one decision: **nothing happens when you turn a
//! dial.** A row remembers what is true on the device and what you have
//! turned it to, and the difference between those two is the work queued up.
//! Applying is a separate, deliberate act at the end.
//!
//! That is not decoration. Half of these patches stop EmulationStation
//! starting if they are wrong, and the device is then a black screen with no
//! menu to undo them from. Queuing means one confirmation covers the whole
//! batch, and it means backing out is free right up until the moment it is not.

/// Which half of the app you are looking at.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Tab {
    Sync,
    Patches,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Sync, Tab::Patches];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Sync => "SYNC",
            Tab::Patches => "PATCHES",
        }
    }
}

/// What a row does when you press A on it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A setting with named options, cycled left and right. Not a checkbox:
    /// the graphics driver has two names rather than an on and an off, and
    /// bezels have a list of packs.
    ///
    /// `live` is `None` when the device is at none of them — KNULLI shipped an
    /// update, or somebody edited the file. That is shown rather than papered
    /// over, and it is deliberately *not* queued on sight: the app should not
    /// propose work the moment it opens.
    Choice {
        options: Vec<String>,
        live: Option<usize>,
        picked: usize,
        touched: bool,
    },
    /// Something to read. No options, never pending.
    Fact { value: String },
    /// An action — push saves, pull saves. Runs on its own rather than
    /// joining the queue, because there is nothing to reconcile.
    Action { note: String },
}

/// One line.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    pub title: String,
    /// One sentence, shown when X is pressed. Says what the patch actually
    /// changes and where, because "on" is not enough to undo something by
    /// hand at two in the morning.
    pub detail: String,
    pub kind: Kind,
}

impl Row {
    pub fn choice(
        id: &str,
        title: &str,
        detail: &str,
        options: &[&str],
        live: usize,
    ) -> Self {
        Row::dial(
            id,
            title,
            detail,
            options.iter().map(|s| (*s).to_string()).collect(),
            Some(live),
        )
    }

    pub fn dial(
        id: &str,
        title: &str,
        detail: &str,
        options: Vec<String>,
        live: Option<usize>,
    ) -> Self {
        Row {
            id: id.into(),
            title: title.into(),
            detail: detail.into(),
            kind: Kind::Choice {
                options,
                live,
                picked: live.unwrap_or(0),
                touched: false,
            },
        }
    }

    /// Which option is dialled up, for whoever has to go and do it.
    pub fn picked(&self) -> Option<usize> {
        match &self.kind {
            Kind::Choice { picked, .. } => Some(*picked),
            _ => None,
        }
    }

    /// The device is at none of this row's options.
    pub fn adrift(&self) -> bool {
        matches!(&self.kind, Kind::Choice { live: None, .. })
    }

    pub fn fact(id: &str, title: &str, value: &str) -> Self {
        Row {
            id: id.into(),
            title: title.into(),
            detail: String::new(),
            kind: Kind::Fact { value: value.into() },
        }
    }

    pub fn action(id: &str, title: &str, detail: &str, note: &str) -> Self {
        Row {
            id: id.into(),
            title: title.into(),
            detail: detail.into(),
            kind: Kind::Action { note: note.into() },
        }
    }

    /// Turned away from what the device is actually doing.
    pub fn pending(&self) -> bool {
        match &self.kind {
            Kind::Choice { live: Some(live), picked, .. } => live != picked,
            // Adrift: queued only once you have actually chosen where it
            // should land, so opening the app proposes nothing.
            Kind::Choice { live: None, touched, .. } => *touched,
            _ => false,
        }
    }

    /// Only rows you can put a cursor on. A fact is not one of them — landing
    /// on a line that does nothing when you press a button reads as the app
    /// having stopped responding.
    pub fn selectable(&self) -> bool {
        !matches!(self.kind, Kind::Fact { .. })
    }

    /// What the right-hand column reads.
    pub fn value(&self) -> &str {
        match &self.kind {
            Kind::Choice { options, live: None, touched: false, .. } => {
                let _ = options;
                "changed"
            }
            Kind::Choice { options, picked, .. } => {
                options.get(*picked).map(String::as_str).unwrap_or("")
            }
            Kind::Fact { value } => value,
            Kind::Action { note } => note,
        }
    }

    /// What it read before you touched it, if that is different.
    pub fn was(&self) -> Option<&str> {
        match &self.kind {
            Kind::Choice { options, live: Some(live), picked, .. } if live != picked => {
                options.get(*live).map(String::as_str)
            }
            Kind::Choice { live: None, touched: true, .. } => Some("changed"),
            _ => None,
        }
    }

    /// Cycling wraps. With two options that is the only sane behaviour, and
    /// with six it saves a long walk back.
    pub fn turn(&mut self, by: i32) {
        if let Kind::Choice { options, picked, touched, .. } = &mut self.kind {
            if options.is_empty() {
                return;
            }
            let n = options.len() as i32;
            *picked = (((*picked as i32 + by) % n + n) % n) as usize;
            *touched = true;
        }
    }

    /// After the script has run and the device really is like this.
    pub fn settle(&mut self) {
        if let Kind::Choice { live, picked, touched, .. } = &mut self.kind {
            *live = Some(*picked);
            *touched = false;
        }
    }

    /// Put the dial back where the device is.
    pub fn revert(&mut self) {
        if let Kind::Choice { live, picked, touched, .. } = &mut self.kind {
            *picked = live.unwrap_or(0);
            *touched = false;
        }
    }
}

/// A tab's worth of rows, and where the cursor is in them.
#[derive(Clone, Debug)]
pub struct Page {
    pub rows: Vec<Row>,
    pub cursor: usize,
}

impl Page {
    pub fn new(rows: Vec<Row>) -> Self {
        let cursor = rows.iter().position(Row::selectable).unwrap_or(0);
        Page { rows, cursor }
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    /// Moves to the next row that can hold a cursor, skipping the facts.
    /// Stops at the ends rather than wrapping — a list that jumps from the
    /// bottom back to the top hides how long it is.
    pub fn step(&mut self, by: i32) {
        let mut at = self.cursor as i32;
        loop {
            at += by;
            if at < 0 || at as usize >= self.rows.len() {
                return;
            }
            if self.rows[at as usize].selectable() {
                self.cursor = at as usize;
                return;
            }
        }
    }

    pub fn turn_selected(&mut self, by: i32) {
        if let Some(row) = self.rows.get_mut(self.cursor) {
            row.turn(by);
        }
    }

    /// Update a fact in place, keeping the cursor where it was.
    ///
    /// Rebuilding the page would be simpler and would also throw the cursor
    /// back to the top every time the status line changed — which, during a
    /// sync, is several times a second.
    pub fn set_fact(&mut self, id: &str, value: &str) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.id == id)
            && let Kind::Fact { value: slot } = &mut row.kind
            && slot != value
        {
            *slot = value.to_string();
        }
    }

    /// The line under an action row — what it last did, or how it is getting on.
    ///
    /// The twin of `set_fact`: facts and actions keep their text in different
    /// places, and the sync tab has both.
    pub fn set_note(&mut self, id: &str, note: &str) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.id == id)
            && let Kind::Action { note: slot } = &mut row.kind
            && slot != note
        {
            *slot = note.to_string();
        }
    }

    pub fn pending(&self) -> Vec<&Row> {
        self.rows.iter().filter(|r| r.pending()).collect()
    }

    pub fn revert_all(&mut self) {
        for row in &mut self.rows {
            row.revert();
        }
    }

    pub fn settle_all(&mut self) {
        for row in &mut self.rows {
            row.settle();
        }
    }
}

/// What is on screen on top of the menu, if anything.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Overlay {
    None,
    /// A is pressed and there is a queue. Lists it and waits.
    ConfirmApply,
    /// B is pressed with a queue outstanding. Leaving would throw it away, so
    /// it asks instead of silently discarding work.
    ConfirmDiscard,
    /// A on a sync row. Those are actions, not dials — there is nothing to
    /// queue, so they ask once and then run.
    ConfirmAction { title: String },
    /// X on a row.
    Detail,
    /// The scripts are running, one after another.
    Applying { done: usize, total: usize },
}

/// The whole app, minus the drawing.
pub struct App {
    pub tab: Tab,
    /// Where syncing has got to. Lives here so the sync tab can draw it, and
    /// is only ever written by `worker::apply`.
    pub stage: crate::sync::Stage,
    /// Saves that changed on both sides. Nothing was written for these and
    /// they still need a person; held here rather than in the stage so the
    /// stage stays cheap to clone and compare.
    pub conflicts: Vec<moose_rack::savesync::SaveConflict>,
    /// Where the favourites-and-collections sync has got to. Its own stage
    /// because it runs independently of the saves — see `sync::Stars`.
    pub stars: crate::sync::Stars,
    /// The favourites plan a person is being shown, held beside the stage for
    /// the same reason the conflicts are: the stage is compared every frame.
    pub star_plan: Option<crate::favrun::Plan>,
    pub sync: Page,
    pub patches: Page,
    pub overlay: Overlay,
    pub should_quit: bool,
}

impl App {
    pub fn page(&self) -> &Page {
        match self.tab {
            Tab::Sync => &self.sync,
            Tab::Patches => &self.patches,
        }
    }

    pub fn page_mut(&mut self) -> &mut Page {
        match self.tab {
            Tab::Sync => &mut self.sync,
            Tab::Patches => &mut self.patches,
        }
    }

    /// Everything queued, across both tabs — switching tabs does not abandon
    /// what you dialled up on the other one.
    pub fn queue(&self) -> Vec<&Row> {
        self.sync
            .rows
            .iter()
            .chain(self.patches.rows.iter())
            .filter(|r| r.pending())
            .collect()
    }

    /// The queue as instructions rather than borrows, so it can be carried
    /// out while the rows themselves are being written back.
    pub fn orders(&self) -> Vec<(String, usize)> {
        self.queue()
            .iter()
            .filter_map(|r| r.picked().map(|i| (r.id.clone(), i)))
            .collect()
    }

    pub fn next_tab(&mut self, by: i32) {
        let n = Tab::ALL.len() as i32;
        let at = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0) as i32;
        self.tab = Tab::ALL[(((at + by) % n + n) % n) as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Page {
        Page::new(vec![
            Row::fact("f", "Server", "connected"),
            Row::choice("a", "Hotkeys", "", &["ON", "OFF"], 0),
            Row::choice("b", "Driver", "", &["stock", "wayland"], 1),
        ])
    }

    #[test]
    fn the_cursor_starts_below_a_fact() {
        // A fact cannot be selected, so a page that opens with one has to put
        // the cursor on the first row that can be.
        assert_eq!(page().cursor, 1);
    }

    #[test]
    fn stepping_skips_facts_and_stops_at_the_ends() {
        let mut p = page();
        p.step(-1);
        assert_eq!(p.cursor, 1, "should not land on the fact above");
        p.step(1);
        assert_eq!(p.cursor, 2);
        p.step(1);
        assert_eq!(p.cursor, 2, "should not run off the bottom");
    }

    #[test]
    fn turning_wraps_both_ways() {
        let mut row = Row::choice("a", "t", "", &["ON", "OFF"], 0);
        row.turn(1);
        assert_eq!(row.value(), "OFF");
        row.turn(1);
        assert_eq!(row.value(), "ON", "two options should wrap round");
        row.turn(-1);
        assert_eq!(row.value(), "OFF", "and wrap backwards");
    }

    #[test]
    fn pending_is_the_difference_from_the_device() {
        let mut row = Row::choice("a", "t", "", &["ON", "OFF"], 0);
        assert!(!row.pending());
        row.turn(1);
        assert!(row.pending());
        assert_eq!(row.was(), Some("ON"), "the queue has to say what it was");
        row.settle();
        assert!(!row.pending(), "settling is what applying leaves behind");
        assert_eq!(row.was(), None);
    }

    #[test]
    fn reverting_puts_the_dial_back() {
        let mut p = page();
        p.turn_selected(1);
        assert_eq!(p.pending().len(), 1);
        p.revert_all();
        assert!(p.pending().is_empty());
    }

    #[test]
    fn a_row_the_device_has_drifted_from_says_so_without_queueing_itself() {
        // KNULLI shipped an update and the file is not any of our options.
        // The menu must show that, and must not propose a fix on sight —
        // opening the app should never leave work sitting in the queue.
        let mut row = Row::dial("a", "t", "", vec!["ON".into(), "off".into()], None);
        assert!(row.adrift());
        assert_eq!(row.value(), "changed");
        assert!(!row.pending(), "nothing may be queued before it is touched");

        row.turn(1);
        assert!(row.pending(), "choosing where it should land queues it");
        assert_eq!(row.was(), Some("changed"));
        row.settle();
        assert!(!row.adrift(), "applying puts it back on a known option");
    }

    #[test]
    fn a_fact_is_never_queued() {
        let mut row = Row::fact("f", "Server", "connected");
        row.turn(1);
        assert!(!row.pending());
        assert_eq!(row.value(), "connected");
    }
}

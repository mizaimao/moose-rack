// An on-screen keyboard, for the fields a handheld cannot otherwise fill.
//
// Written here rather than borrowed. ES-DE has one and its Android port is not
// open source; the desktop one is GPL and pulling it in would put another name
// in LICENSES.md for something that is four arrays and a cursor. This is the
// four arrays and the cursor.
//
// What needs it, and nothing else does: the RetroAchievements account, the
// ScreenScraper account, the Wi-Fi password, and the RomM server if it is ever
// typed here rather than put on the card. Everything else in Settings is a
// toggle, a choice from a fixed set, or a number — all of which a d-pad does
// better than a keyboard would.
//
// The layout is a fixed 10x4 grid plus an action row, because a grid is what
// makes the movement arithmetic instead of a table of special cases. Keys that
// would want to be wider — space especially — get their width in the *drawing*
// and stay one cell to the cursor.

/// Which set of characters the keys are showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Lower,
    Upper,
    Symbols,
}

/// The character grid, ten wide and four tall, per page.
///
/// Ten columns because that is what fits `qwertyuiop` without splitting the row
/// somewhere nobody expects. The rows are padded to ten so every position is a
/// real key — a ragged grid means "down" from the end of one row lands on
/// nothing, which is the bug every hand-rolled keyboard has.
const LOWER: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl-", "zxcvbnm_.@"];
const UPPER: [&str; 4] = ["!\"#$%&'()*", "QWERTYUIOP", "ASDFGHJKL+", "ZXCVBNM<>?"];
const SYMBOLS: [&str; 4] = ["1234567890", "~`|\\/:;,\"'", "[]{}()<>=^", "!?@#$%&*_-"];

pub const COLS: usize = 10;
pub const ROWS: usize = 4;

/// The buttons under the grid. One cell each to the cursor, whatever their
/// drawn width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Shift,
    Symbols,
    Space,
    Backspace,
    Done,
}

pub const ACTIONS: [Action; 5] = [
    Action::Shift,
    Action::Symbols,
    Action::Space,
    Action::Backspace,
    Action::Done,
];

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Shift => "Shift",
            Action::Symbols => "#+=",
            Action::Space => "Space",
            Action::Backspace => "Del",
            Action::Done => "OK",
        }
    }
}

/// What a keypress did, for the caller to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Still typing.
    Typing,
    /// Done was pressed — take the text.
    Done,
    /// Back was pressed with nothing to delete — the field is abandoned.
    Cancelled,
}

/// One text field being filled in.
pub struct Keyboard {
    /// What this keyboard is filling in, so the caller does not have to
    /// remember what it opened one for.
    pub target: Option<crate::library::Target>,
    /// What is being asked for, drawn above the field.
    pub prompt: String,
    /// What has been typed.
    pub text: String,
    /// Dots instead of characters. On for anything called a password or token —
    /// a handheld is a thing you use in a room with other people in it.
    pub secret: bool,
    pub page: Page,
    /// Cursor, as a row in `0..=ROWS`. Row `ROWS` is the action row.
    pub row: usize,
    pub col: usize,
}

impl Keyboard {
    pub fn new(prompt: impl Into<String>, initial: &str, secret: bool) -> Self {
        Self {
            target: None,
            prompt: prompt.into(),
            text: initial.to_owned(),
            secret,
            page: Page::Lower,
            row: 1,
            col: 0,
        }
    }

    /// Say what this keyboard is filling in.
    pub fn filling(mut self, target: crate::library::Target) -> Self {
        self.target = Some(target);
        self
    }

    /// The characters currently on the keys.
    pub fn grid(&self) -> [&'static str; 4] {
        match self.page {
            Page::Lower => LOWER,
            Page::Upper => UPPER,
            Page::Symbols => SYMBOLS,
        }
    }

    /// The character at a grid position, if there is one.
    pub fn key_at(&self, row: usize, col: usize) -> Option<char> {
        self.grid().get(row)?.chars().nth(col)
    }

    /// Whether the cursor is on the action row rather than the grid.
    pub fn on_actions(&self) -> bool {
        self.row >= ROWS
    }

    /// The action under the cursor, when it is on the action row.
    pub fn action(&self) -> Option<Action> {
        self.on_actions()
            .then(|| ACTIONS[self.col.min(ACTIONS.len() - 1)])
    }

    /// What to show in the field: the text, or a dot for each character.
    pub fn shown(&self) -> String {
        if self.secret {
            "\u{2022}".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }

    /// Drive the keyboard with one of the app's actions.
    ///
    /// The same action names the rest of the front end uses, so the pad and the
    /// keyboard both reach it without a second binding table.
    pub fn act(&mut self, action: &str) -> Outcome {
        match action {
            "left" => self.step(-1, 0),
            "right" => self.step(1, 0),
            "up" => self.step(0, -1),
            "down" => self.step(0, 1),
            "activate" => return self.press(),
            // Back deletes, which is what every phone does with the button next
            // to the keyboard. With nothing left to delete it leaves — so a
            // field opened by mistake costs two presses, not a trip through a
            // menu.
            "back" | "back2" => {
                if self.text.is_empty() {
                    return Outcome::Cancelled;
                }
                self.text.pop();
            }
            // The shoulders switch pages without moving the cursor off the key
            // it is on, which is what makes a capital letter one press and a
            // move rather than three.
            "prevSection" | "nextSection" => self.cycle_page(),
            _ => {}
        }
        Outcome::Typing
    }

    /// Press whatever is under the cursor.
    fn press(&mut self) -> Outcome {
        if let Some(action) = self.action() {
            match action {
                Action::Shift => {
                    self.page = match self.page {
                        Page::Lower => Page::Upper,
                        _ => Page::Lower,
                    }
                }
                Action::Symbols => {
                    self.page = match self.page {
                        Page::Symbols => Page::Lower,
                        _ => Page::Symbols,
                    }
                }
                Action::Space => self.text.push(' '),
                Action::Backspace => {
                    self.text.pop();
                }
                Action::Done => return Outcome::Done,
            }
            return Outcome::Typing;
        }
        if let Some(c) = self.key_at(self.row, self.col) {
            self.text.push(c);
            // A capital is one letter, not a mode. Anything else and every
            // name typed here comes out in CAPITALS.
            if self.page == Page::Upper {
                self.page = Page::Lower;
            }
        }
        Outcome::Typing
    }

    fn cycle_page(&mut self) {
        self.page = match self.page {
            Page::Lower => Page::Upper,
            Page::Upper => Page::Symbols,
            Page::Symbols => Page::Lower,
        };
    }

    /// Move the cursor, wrapping around every edge.
    ///
    /// Wrapping rather than stopping because there is no pointer to jump with:
    /// getting from `1` to `OK` should not mean holding down for four rows and
    /// right for nine columns.
    fn step(&mut self, dx: isize, dy: isize) {
        let rows = ROWS + 1;
        if dy != 0 {
            let was_grid = !self.on_actions();
            let next = (self.row as isize + dy).rem_euclid(rows as isize) as usize;
            // Coming into the action row from a 10-wide grid, land on the
            // button under where the cursor was rather than resetting to the
            // first one.
            self.col = if next >= ROWS && was_grid {
                (self.col * ACTIONS.len()) / COLS
            } else if next < ROWS && self.on_actions() {
                (self.col * COLS) / ACTIONS.len()
            } else {
                self.col
            };
            self.row = next;
        }
        let width = if self.on_actions() {
            ACTIONS.len()
        } else {
            COLS
        };
        if dx != 0 {
            self.col = (self.col as isize + dx).rem_euclid(width as isize) as usize;
        }
        self.col = self.col.min(width - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb() -> Keyboard {
        Keyboard::new("RetroAchievements token", "", false)
    }

    fn type_all(k: &mut Keyboard, actions: &[&str]) {
        for a in actions {
            k.act(a);
        }
    }

    /// Every position in the grid is a real key.
    ///
    /// A ragged layout is how a hand-rolled keyboard gets a hole in it: "down"
    /// from the end of a long row lands on a position the short row below does
    /// not have, and the press does nothing with no way to tell why.
    #[test]
    fn every_grid_position_has_a_key_on_every_page() {
        for page in [Page::Lower, Page::Upper, Page::Symbols] {
            let mut k = kb();
            k.page = page;
            for row in 0..ROWS {
                assert_eq!(
                    k.grid()[row].chars().count(),
                    COLS,
                    "{page:?} row {row} is not {COLS} wide"
                );
                for col in 0..COLS {
                    assert!(
                        k.key_at(row, col).is_some(),
                        "{page:?} {row},{col} is a hole"
                    );
                }
            }
        }
    }

    /// Typing a capital returns to lower case, so a name is not shouted.
    #[test]
    fn shift_applies_to_one_letter_only() {
        let mut k = kb();
        k.page = Page::Upper;
        k.row = 1;
        k.col = 0; // Q
        k.act("activate");
        assert_eq!(k.text, "Q");
        assert_eq!(
            k.page,
            Page::Lower,
            "shift stayed on and would capitalize the rest"
        );
        k.act("activate");
        assert_eq!(k.text, "Qq");
    }

    /// Back deletes while there is text, and leaves once there is not.
    #[test]
    fn back_deletes_then_leaves() {
        let mut k = kb();
        k.text = "ab".into();
        assert_eq!(k.act("back"), Outcome::Typing);
        assert_eq!(k.text, "a");
        assert_eq!(k.act("back"), Outcome::Typing);
        assert_eq!(k.text, "");
        assert_eq!(
            k.act("back"),
            Outcome::Cancelled,
            "an empty field should let go"
        );
    }

    /// The cursor wraps at every edge, including between the grid and the
    /// buttons under it.
    #[test]
    fn the_cursor_wraps_rather_than_sticking() {
        let mut k = kb();
        k.row = 0;
        k.col = 0;
        k.act("left");
        assert_eq!(k.col, COLS - 1, "left from the first column did not wrap");
        k.col = 0;
        k.act("up");
        assert!(
            k.on_actions(),
            "up from the top row should reach the buttons"
        );
        k.act("down");
        assert_eq!(k.row, 0, "down from the buttons should return to the top");
    }

    /// Moving between a ten-wide grid and a five-wide button row keeps the
    /// cursor roughly where it was, rather than snapping to the first button.
    #[test]
    fn dropping_onto_the_buttons_keeps_your_place() {
        let mut k = kb();
        k.row = ROWS - 1;
        k.col = 9; // far right of the grid
        k.act("down");
        assert_eq!(
            k.action(),
            Some(Action::Done),
            "the rightmost key should reach OK"
        );

        k.row = ROWS - 1;
        k.col = 0;
        k.act("down");
        assert_eq!(
            k.action(),
            Some(Action::Shift),
            "the leftmost key should reach Shift"
        );
    }

    /// A password is never drawn. The device is used in rooms with other people
    /// in them, and a token on screen is a token on screen.
    #[test]
    fn a_secret_is_shown_as_dots_but_kept_whole() {
        let mut k = Keyboard::new("Wi-Fi password", "", true);
        k.text = "hunter2".into();
        assert_eq!(k.shown(), "\u{2022}".repeat(7));
        assert_eq!(k.text, "hunter2", "the real text must survive being hidden");
    }

    /// Done hands the text back; nothing else does.
    #[test]
    fn only_done_finishes() {
        let mut k = kb();
        k.text = "frank".into();
        k.row = ROWS;
        k.col = 0; // Shift
        assert_eq!(k.act("activate"), Outcome::Typing);
        k.col = ACTIONS.len() - 1; // OK
        assert_eq!(k.act("activate"), Outcome::Done);
        assert_eq!(k.text, "frank");
    }

    /// The shoulders reach every page and come back round.
    #[test]
    fn the_shoulders_cycle_the_pages() {
        let mut k = kb();
        assert_eq!(k.page, Page::Lower);
        type_all(&mut k, &["nextSection"]);
        assert_eq!(k.page, Page::Upper);
        type_all(&mut k, &["nextSection"]);
        assert_eq!(k.page, Page::Symbols);
        type_all(&mut k, &["nextSection"]);
        assert_eq!(k.page, Page::Lower);
    }

    /// A token is the reason this exists: letters, digits and punctuation, all
    /// reachable without leaving the pad.
    #[test]
    fn a_realistic_token_can_be_typed() {
        let mut k = kb();
        // Every character of a token like this has to exist somewhere.
        for c in "aZ9_-.@".chars() {
            let found = [Page::Lower, Page::Upper, Page::Symbols].iter().any(|p| {
                let mut probe = kb();
                probe.page = *p;
                (0..ROWS).any(|r| (0..COLS).any(|col| probe.key_at(r, col) == Some(c)))
            });
            assert!(found, "{c:?} cannot be typed on any page");
        }
        let _ = &mut k;
    }
}

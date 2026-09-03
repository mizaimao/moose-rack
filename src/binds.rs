// Keyboard and controller bindings: the tables, the resolution, the storage.
//
// Moved out of `ui/js/bindings.js`, which had no DOM in it at all — it was 251
// lines of pure decision-making sitting in the one front end that happened to
// be written first. The TUI could not read a keybinding, and an SDL front end
// would have had to reimplement the healing rules below from the comments.
//
// Two things live here and they are deliberately not merged: which key or
// button a person has *chosen* (`Bindings`, on disk), and which key or button
// an action *ends up on* once defaults and repair are folded in (`pad_map`,
// `key_for`). Front ends ask for the second and never compute it themselves.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One thing the app can be told to do.
///
/// Order here is the order Settings lists them in.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    /// The key it starts on, or `None` for deliberately unbound.
    ///
    /// D/G/I/T are unbound out of the box: bare letters are easy to hit by
    /// accident, so those actions are opt-in via Settings.
    pub fallback: Option<&'static str>,
}

macro_rules! action {
    ($id:literal, $label:literal, $fallback:expr) => {
        Action {
            id: $id,
            label: $label,
            fallback: $fallback,
        }
    };
}

pub const ACTIONS: &[Action] = &[
    action!("left", "Move left", Some("ArrowLeft")),
    action!("right", "Move right", Some("ArrowRight")),
    action!("up", "Move up", Some("ArrowUp")),
    action!("down", "Move down", Some("ArrowDown")),
    action!("first", "Jump to first", Some("Home")),
    action!("last", "Jump to last", Some("End")),
    action!("pageUp", "Page up", Some("PageUp")),
    action!("pageDown", "Page down", Some("PageDown")),
    action!("activate", "Open platform / play", Some("Enter")),
    action!("back", "Go back", Some("Escape")),
    action!("back2", "Go back (alternate)", Some("Backspace")),
    action!("search", "Focus search", Some("/")),
    action!("help", "Shortcut list", Some("?")),
    action!("download", "Download without playing", None),
    // The glyph it draws, so the key and the mark on the row are the same
    // thing. Every other letter worth having was taken: f is the filter, s
    // the sort, r random.
    action!("favorite", "Star this game", Some("*")),
    action!("layout", "Toggle grid / list", None),
    action!("sidebar", "Toggle info pane", None),
    action!("settings", "Open settings", None),
    action!("prevSection", "Previous section", Some("q")),
    action!("nextSection", "Next section", Some("e")),
    action!("scrollUp", "Scroll the list up", None),
    action!("scrollDown", "Scroll the list down", None),
    action!("zoomIn", "Bigger covers", Some("+")),
    action!("zoomOut", "Smaller covers", Some("-")),
    action!("video", "Play gameplay video", Some("v")),
    action!("pictures", "Change the pictures", None),
    action!("sortCycle", "Next sort order", None),
    action!("sortMenu", "Sort by…", Some("s")),
    action!("filterMenu", "Filter this list…", Some("f")),
    action!("random", "Surprise me", Some("r")),
];

/// Controller buttons, by W3C "standard mapping" index.
///
/// Separate from the keyboard table because the two are rebound independently
/// — and because an index means nothing without a name: 0 is the bottom face
/// button, which is A on Xbox, Cross on PlayStation and B on a Nintendo pad.
///
/// SDL_GameController numbers its buttons differently, so a front end reading
/// SDL translates its own button into this index before asking anything here.
/// The index is the vocabulary; where it came from is the front end's problem.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PadButton {
    pub index: u8,
    pub name: &'static str,
}

pub const PAD_BUTTONS: &[PadButton] = &[
    PadButton {
        index: 0,
        name: "A / Cross (bottom face)",
    },
    PadButton {
        index: 1,
        name: "B / Circle (right face)",
    },
    PadButton {
        index: 2,
        name: "X / Square (left face)",
    },
    PadButton {
        index: 3,
        name: "Y / Triangle (top face)",
    },
    PadButton {
        index: 4,
        name: "L1 / LB",
    },
    PadButton {
        index: 5,
        name: "R1 / RB",
    },
    PadButton {
        index: 6,
        name: "L2 / LT",
    },
    PadButton {
        index: 7,
        name: "R2 / RT",
    },
    PadButton {
        index: 8,
        name: "Select / Share",
    },
    PadButton {
        index: 9,
        name: "Start / Options",
    },
    PadButton {
        index: 10,
        name: "L3 (left stick)",
    },
    PadButton {
        index: 11,
        name: "R3 (right stick)",
    },
    PadButton {
        index: 12,
        name: "D-pad up",
    },
    PadButton {
        index: 13,
        name: "D-pad down",
    },
    PadButton {
        index: 14,
        name: "D-pad left",
    },
    PadButton {
        index: 15,
        name: "D-pad right",
    },
];

/// Defaults, chosen by position rather than label so they read correctly on
/// every controller family.
///
/// The shoulders move between sections — the navigation you use constantly,
/// and the one thing that should never need the cursor. The triggers scroll
/// the list, and how hard you pull decides how fast; they were zoom, which is
/// a thing you set once and then leave, a poor use of the only two analogue
/// controls on the pad on a screen whose main job is moving through two
/// thousand games.
///
/// The top face button plays the gameplay video: the one thing ES-DE has that
/// is genuinely hard to find. Select cycles the pictures rather than opening
/// settings, which is a second window full of text fields and tables a pad
/// cannot navigate — so the button opened something you then could not use and
/// could only leave again.
///
/// The left stick click steps through the sort orders. The right one picks a
/// game at random: the one thing on this screen worth a button, needing no
/// menu, answering the question a 2,506-game arcade list actually poses.
pub const PAD_FALLBACK: &[(u8, &str)] = &[
    (0, "activate"),
    (1, "back"),
    (3, "video"),
    (4, "prevSection"),
    (5, "nextSection"),
    (6, "scrollUp"),
    (7, "scrollDown"),
    (8, "pictures"),
    (9, "help"),
    (10, "sortCycle"),
    (11, "random"),
    (12, "up"),
    (13, "down"),
    (14, "left"),
    (15, "right"),
];

/// Actions the app is unusable without.
///
/// Anything else can be left unbound on purpose — plenty of people never want
/// a themes button on their pad. These five are different: with a direction
/// missing you cannot reach half the grid, and with Confirm missing you cannot
/// open anything at all.
const ESSENTIAL: &[&str] = &["up", "down", "left", "right", "activate", "back"];

/// What a person has chosen, layered over the tables above.
///
/// An entry present but empty is not the same as an entry absent: empty means
/// "deliberately cleared", which is what a rebind writes over the button that
/// used to hold the action, and it has to survive a reload or the default
/// comes straight back and the rebind looks like it did nothing.
///
/// TOML has no null, hence the empty string rather than an `Option`. The
/// distinction is restored at the edges of this module and nowhere else has to
/// know about it.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Bindings {
    /// action id -> key, `""` meaning deliberately unbound.
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
    /// button index -> action id, `""` meaning cleared by a rebind.
    #[serde(default)]
    pub pad: BTreeMap<String, String>,
}

impl Bindings {
    /// The resolved controller map: index -> action, or `None` where a rebind
    /// cleared the button.
    ///
    /// `None` is kept rather than dropped because the settings window draws
    /// this table, and "bound to nothing" and "not a button on this pad" are
    /// different answers to why a press did nothing.
    ///
    /// Callers dispatching from it must skip the `None`s. That is the whole of
    /// the bug behind "a button cleared by a rebind is dispatched as null".
    pub fn pad_map(&self) -> BTreeMap<u8, Option<String>> {
        let mut map: BTreeMap<u8, Option<String>> = PAD_FALLBACK
            .iter()
            .map(|(i, a)| (*i, Some((*a).to_owned())))
            .collect();
        for (index, action) in &self.pad {
            let Ok(index) = index.parse::<u8>() else {
                continue;
            };
            map.insert(
                index,
                if action.is_empty() {
                    None
                } else {
                    Some(action.clone())
                },
            );
        }

        // Rebinding clears whichever button previously held that action. If
        // that leaves an essential action with no button at all, the pad is
        // broken rather than customized — a direction that does nothing looks
        // exactly like an app ignoring the button, and there is nothing on
        // screen to say otherwise.
        //
        // Two passes, and the second one is the important half. Healing only
        // into a *free* button is polite and insufficient: bind Up onto
        // Confirm's button and clear Up's own, and Confirm has nowhere to go
        // home to. The pad then has no way to open anything, no way to reach
        // the settings screen that could fix it, and no way out. Being locked
        // out of the app is strictly worse than losing a binding somebody
        // chose, so the second pass takes the button back.
        // Three passes, in order of politeness. Healing only into a *free* home
        // button is polite and insufficient: put Up onto Confirm's button and
        // clear Up's own, and Confirm has nowhere to go home to. The pad then
        // cannot open anything, cannot reach the settings screen that would fix
        // it, and cannot get out.
        //
        //   1. its own default button, if nothing is on it
        //   2. any button nothing is on
        //   3. its own default button, evicting whatever is there
        //
        // Three exists so that two can promise never to steal a button somebody
        // chose. Being locked out is worse than an odd placement, and worse
        // again than losing one binding — but it is the last resort, not the
        // first.
        for pass in 0..3 {
            for action in ESSENTIAL {
                if map.values().any(|a| a.as_deref() == Some(*action)) {
                    continue;
                }
                let home = PAD_FALLBACK
                    .iter()
                    .find(|(_, a)| a == action)
                    .map(|(i, _)| *i);
                let free = |at: &u8| map.get(at).map(Option::is_none).unwrap_or(true);
                let onto = match pass {
                    0 => home.filter(free),
                    1 => PAD_BUTTONS.iter().map(|b| b.index).find(|i| free(i)),
                    _ => home,
                };
                if let Some(onto) = onto {
                    map.insert(onto, Some((*action).to_owned()));
                }
            }
        }
        map
    }

    /// The pad map with the face buttons swapped as asked.
    ///
    /// Applied to the *map*, not to the bindings — the swap is how this app
    /// reads the pad, so it never reaches the file, never reaches RetroArch,
    /// and a game still plays with the buttons the game expects. Turning it off
    /// restores the pad exactly; nothing was rewritten to turn it on.
    pub fn pad_map_swapped(&self, ab: bool, xy: bool) -> BTreeMap<u8, Option<String>> {
        let mut map = self.pad_map();
        for (on, (l, r)) in [(ab, (0u8, 1u8)), (xy, (2u8, 3u8))] {
            if !on {
                continue;
            }
            let (a, b) = (
                map.get(&l).cloned().flatten(),
                map.get(&r).cloned().flatten(),
            );
            map.insert(l, b);
            map.insert(r, a);
        }
        map
    }

    /// Which button currently triggers `action`, or `None`.
    pub fn pad_for(&self, action: &str) -> Option<u8> {
        self.pad_map()
            .into_iter()
            .find(|(_, a)| a.as_deref() == Some(action))
            .map(|(i, _)| i)
    }

    /// Bind `action` to `index`, clearing whatever else held that button.
    /// A `None` index unbinds.
    pub fn set_pad(&mut self, action: &str, index: Option<u8>) {
        for (i, a) in self.pad_map() {
            if a.as_deref() == Some(action) {
                self.pad.insert(i.to_string(), String::new());
            }
        }
        if let Some(index) = index {
            self.pad.insert(index.to_string(), action.to_owned());
        }
    }

    /// An essential action the chosen bindings leave with no button, if any.
    ///
    /// Reported *before* `pad_map` heals it, so a settings screen can refuse
    /// the rebind and say why rather than accepting it and quietly putting the
    /// action somewhere else.
    pub fn stranded(&self) -> Option<&'static str> {
        let mut chosen: BTreeMap<u8, Option<&str>> =
            PAD_FALLBACK.iter().map(|(i, a)| (*i, Some(*a))).collect();
        for (index, action) in &self.pad {
            let Ok(index) = index.parse::<u8>() else {
                continue;
            };
            chosen.insert(
                index,
                if action.is_empty() {
                    None
                } else {
                    Some(action)
                },
            );
        }
        ESSENTIAL
            .iter()
            .find(|action| !chosen.values().any(|a| a == &Some(**action)))
            .copied()
    }

    pub fn reset_pad(&mut self) {
        self.pad.clear();
    }

    /// Current key for an action, or `None` when unbound.
    pub fn key_for(&self, id: &str) -> Option<String> {
        if let Some(chosen) = self.keys.get(id) {
            return if chosen.is_empty() {
                None
            } else {
                Some(chosen.clone())
            };
        }
        ACTIONS
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| a.fallback)
            .map(str::to_owned)
    }

    /// Action bound to a pressed key, or `None`.
    ///
    /// Case-insensitive for single characters so a binding works whether or
    /// not Shift is held; anything longer is a named key (`ArrowLeft`) where
    /// case is part of the name.
    pub fn action_for(&self, key: &str) -> Option<&'static str> {
        let want = fold(key);
        ACTIONS
            .iter()
            .find(|a| self.key_for(a.id).map(|k| fold(&k)) == Some(want.clone()))
            .map(|a| a.id)
    }

    pub fn set_key(&mut self, id: &str, key: Option<&str>) {
        // A key can only drive one action; clear whoever held it.
        if let Some(key) = key {
            for a in ACTIONS {
                if a.id != id && self.key_for(a.id).as_deref() == Some(key) {
                    self.keys.insert(a.id.to_owned(), String::new());
                }
            }
        }
        self.keys
            .insert(id.to_owned(), key.unwrap_or_default().to_owned());
    }

    pub fn reset_keys(&mut self) {
        self.keys.clear();
    }

    /// Take on bindings from somewhere else, without overruling what is
    /// already here.
    ///
    /// For the one-way door out of the webview's own storage, where these
    /// lived before the file did — see `import_bindings` in the Tauri layer.
    /// Anything already set wins, so running it a second time after somebody
    /// has rebound something cannot undo them, and an action or a button this
    /// build does not have is dropped rather than written back out.
    ///
    /// `None` means "deliberately unbound", which is what the old storage
    /// wrote over a button a rebind had cleared, and it has to survive the
    /// move or the default comes straight back.
    pub fn adopt(
        &mut self,
        keys: impl IntoIterator<Item = (String, Option<String>)>,
        pad: impl IntoIterator<Item = (String, Option<String>)>,
    ) {
        for (action, key) in keys {
            if ACTIONS.iter().any(|a| a.id == action) {
                self.keys
                    .entry(action)
                    .or_insert_with(|| key.unwrap_or_default());
            }
        }
        for (index, action) in pad {
            if index.parse::<u8>().is_ok() {
                self.pad
                    .entry(index)
                    .or_insert_with(|| action.unwrap_or_default());
            }
        }
    }
}

/// Lowercase a single character, leave named keys alone.
fn fold(key: &str) -> String {
    if key.chars().count() == 1 {
        key.to_lowercase()
    } else {
        key.to_owned()
    }
}

/// Human label for a key, e.g. `ArrowLeft` -> `←`.
pub fn key_label(key: Option<&str>) -> String {
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return "—".to_owned();
    };
    match key {
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "Escape" => "Esc",
        "Backspace" => "⌫",
        "Enter" => "⏎",
        " " => "Space",
        "PageUp" => "PgUp",
        "PageDown" => "PgDn",
        other => return other.to_uppercase(),
    }
    .to_owned()
}

/// Human label for a controller button.
pub fn pad_label(index: Option<u8>) -> String {
    let Some(index) = index else {
        return "unset".to_owned();
    };
    PAD_BUTTONS
        .iter()
        .find(|b| b.index == index)
        .map(|b| b.name.to_owned())
        .unwrap_or_else(|| format!("button {index}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `ui/test/gamepad.test.js`, "pad bindings".
    #[test]
    fn the_default_map_sends_the_face_buttons_to_open_and_back() {
        let map = Bindings::default().pad_map();
        assert_eq!(
            map[&0].as_deref(),
            Some("activate"),
            "bottom face button opens"
        );
        assert_eq!(
            map[&1].as_deref(),
            Some("back"),
            "right face button goes back"
        );
        // Select cycles the pictures. It used to open settings — a second
        // window of text fields and tables that a pad cannot navigate.
        assert_eq!(
            map[&8].as_deref(),
            Some("pictures"),
            "Select should change the pictures"
        );
    }

    /// The failure this guards against is invisible at runtime: a front end
    /// looks the id up, finds nothing, and returns without a sound. A binding
    /// pointing at a renamed action is exactly "the button does nothing".
    #[test]
    fn every_default_binding_names_an_action_that_exists() {
        for (index, action) in PAD_FALLBACK {
            assert!(
                ACTIONS.iter().any(|a| a.id == *action),
                "button {index} is bound to \"{action}\", which is not an action"
            );
        }
    }

    #[test]
    fn a_rebind_moves_the_action_and_frees_the_old_button() {
        let mut b = Bindings::default();
        b.set_pad("activate", Some(3));
        assert_eq!(b.pad_map()[&3].as_deref(), Some("activate"));
        assert_eq!(
            b.pad_map()[&0],
            None,
            "the old button is cleared, not left dangling"
        );
        b.reset_pad();
        assert_eq!(
            b.pad_map()[&0].as_deref(),
            Some("activate"),
            "reset restores the defaults"
        );
    }

    #[test]
    fn pad_for_reports_where_an_action_currently_lives() {
        let mut b = Bindings::default();
        assert_eq!(b.pad_for("activate"), Some(0));
        b.set_pad("activate", Some(2));
        assert_eq!(b.pad_for("activate"), Some(2));
    }

    /// A direction that does nothing looks exactly like an app ignoring the
    /// button. Unbinding one puts it back rather than leaving the pad broken.
    #[test]
    fn an_essential_action_cannot_be_left_on_no_button() {
        let mut b = Bindings::default();
        b.set_pad("up", None);
        assert_eq!(b.pad_for("up"), Some(12), "the d-pad lost its up");

        // But healing never steals a button somebody deliberately assigned.
        let mut b = Bindings::default();
        b.set_pad("up", None);
        b.set_pad("random", Some(12));
        assert_eq!(
            b.pad_map()[&12].as_deref(),
            Some("random"),
            "the deliberate bind was stolen"
        );
    }

    /// The face-button swap is how this app reads the pad, and nothing more.
    ///
    /// It must not touch the stored bindings: a game plays with the buttons the
    /// game expects, and turning the swap off has to restore the pad exactly
    /// rather than leave a rewritten file behind.
    #[test]
    fn swapping_the_face_buttons_changes_only_how_this_app_reads_them() {
        let b = Bindings::default();
        let plain = b.pad_map();
        let (a0, b1) = (plain[&0].clone(), plain[&1].clone());
        assert_eq!(a0.as_deref(), Some("activate"));
        assert_eq!(b1.as_deref(), Some("back"));

        let swapped = b.pad_map_swapped(true, false);
        assert_eq!(swapped[&0], b1, "the bottom button did not become Back");
        assert_eq!(swapped[&1], a0, "the right button did not become Confirm");
        assert_eq!(
            swapped.get(&2),
            plain.get(&2),
            "the other pair moved as well"
        );

        assert_eq!(
            b.pad,
            Bindings::default().pad,
            "the swap rewrote the bindings"
        );
        assert_eq!(
            b.pad_map_swapped(false, false),
            plain,
            "off is not the plain map"
        );
    }

    /// Both pairs at once, and X/Y on its own.
    #[test]
    fn each_pair_swaps_independently() {
        let b = Bindings::default();
        let plain = b.pad_map();
        // Button 2 has no default binding, so this also checks that swapping
        // with an unbound button gives the pair "nothing" rather than panicking.
        let xy = b.pad_map_swapped(false, true);
        assert_eq!(xy[&2], plain[&3], "X did not take Y's action");
        assert_eq!(
            xy[&3],
            plain.get(&2).cloned().flatten(),
            "Y kept an action X never had"
        );
        assert_eq!(xy[&0], plain[&0], "A/B moved when only X/Y was asked for");

        let both = b.pad_map_swapped(true, true);
        assert_eq!(both[&0], plain[&1]);
        assert_eq!(both[&3], plain.get(&2).cloned().flatten());
    }

    /// The lock-out, as a test.
    ///
    /// Frank's own config after a few presses on the settings screen: Up moved
    /// onto Confirm's button, and Up's own button cleared by the rebind. Confirm
    /// then had no button, its home was taken, and healing only into a free home
    /// gave up — leaving a pad that could not open anything, could not reach the
    /// screen that would fix it, and could not leave.
    #[test]
    fn a_rebind_onto_confirms_button_cannot_strand_confirm() {
        let mut b = Bindings::default();
        b.pad.insert("0".into(), "up".into());
        b.pad.insert("12".into(), String::new());

        let map = b.pad_map();
        assert!(
            map.values().any(|a| a.as_deref() == Some("activate")),
            "nothing opens anything: {map:?}"
        );
        assert!(
            b.pad_for("up").is_some(),
            "up went missing while confirm was rescued"
        );
    }

    /// Whatever is in the file, every essential action comes out on a button.
    ///
    /// The blunt version of the rule, because the interesting cases are the ones
    /// nobody thought of — a hand-edited file, a half-finished rebind, a config
    /// copied from another device.
    #[test]
    fn no_arrangement_of_the_file_can_strand_an_essential() {
        for (button, action) in [
            ("0", "up"),
            ("1", "up"),
            ("12", "activate"),
            ("13", "back"),
            ("0", ""),
        ] {
            let mut b = Bindings::default();
            b.pad.insert(button.into(), action.into());
            // And clear everything else, which is the worst a file can be.
            for i in 0..16u8 {
                b.pad.entry(i.to_string()).or_insert_with(String::new);
            }
            let map = b.pad_map();
            for essential in ESSENTIAL {
                assert!(
                    map.values().any(|a| a.as_deref() == Some(*essential)),
                    "{essential:?} has no button with {button}={action:?}: {map:?}"
                );
            }
        }
    }

    /// Non-essential actions stay unbound, because plenty of people never want
    /// a themes button on their pad.
    #[test]
    fn an_ordinary_action_stays_unbound() {
        let mut b = Bindings::default();
        b.set_pad("video", None);
        assert_eq!(b.pad_for("video"), None);
    }

    #[test]
    fn a_key_only_drives_one_action() {
        let mut b = Bindings::default();
        b.set_key("random", Some("s"));
        assert_eq!(b.action_for("s"), Some("random"));
        assert_eq!(
            b.key_for("sortMenu"),
            None,
            "the old owner kept the key too"
        );
    }

    /// A binding works whether or not Shift is held; a named key keeps its case.
    #[test]
    fn letters_are_case_insensitive_and_named_keys_are_not() {
        let b = Bindings::default();
        assert_eq!(b.action_for("S"), Some("sortMenu"));
        assert_eq!(b.action_for("s"), Some("sortMenu"));
        assert_eq!(b.action_for("ArrowLeft"), Some("left"));
        assert_eq!(b.action_for("arrowleft"), None);
    }

    /// Deliberately unbound out of the box, and it has to stay that way: bare
    /// letters are easy to hit by accident.
    #[test]
    fn the_opt_in_actions_start_on_no_key() {
        let b = Bindings::default();
        for id in ["download", "layout", "sidebar", "settings", "pictures"] {
            assert_eq!(b.key_for(id), None, "{id} is bound out of the box");
        }
    }

    /// An empty entry means "cleared", not "absent" — or the default comes
    /// straight back on reload and the rebind looks like it did nothing.
    #[test]
    fn a_cleared_binding_survives_a_round_trip() {
        let mut b = Bindings::default();
        b.set_key("left", None);
        let toml = toml::to_string(&b).expect("serialising bindings");
        let back: Bindings = toml::from_str(&toml).expect("parsing bindings");
        assert_eq!(back.key_for("left"), None, "the arrow key came back");
    }

    /// The one-way door out of the webview's own storage. Getting this wrong
    /// means somebody's rebinds are silently thrown away on the launch that
    /// moves them, and there is nothing on screen to say so.
    #[test]
    fn bindings_are_adopted_from_the_old_storage() {
        let mut b = Bindings::default();
        b.adopt(
            [("sortMenu".to_owned(), Some("z".to_owned()))],
            [("3".to_owned(), Some("activate".to_owned()))],
        );
        assert_eq!(b.key_for("sortMenu").as_deref(), Some("z"));
        assert_eq!(b.pad_map()[&3].as_deref(), Some("activate"));
    }

    /// Running it again after somebody has rebound something must not undo
    /// them: the file is the truth by then.
    #[test]
    fn adopting_never_overrules_what_is_already_set() {
        let mut b = Bindings::default();
        b.set_key("sortMenu", Some("q"));
        b.set_pad("video", Some(2));
        b.adopt(
            [("sortMenu".to_owned(), Some("z".to_owned()))],
            [("2".to_owned(), Some("random".to_owned()))],
        );
        assert_eq!(
            b.key_for("sortMenu").as_deref(),
            Some("q"),
            "an old key overruled a new one"
        );
        assert_eq!(
            b.pad_map()[&2].as_deref(),
            Some("video"),
            "an old button overruled a new one"
        );
    }

    /// "Deliberately unbound" is what the old storage wrote over a button a
    /// rebind had cleared. Losing that on the way across puts the default
    /// straight back, and the rebind looks like it never happened.
    #[test]
    fn an_unbound_entry_survives_being_adopted() {
        let mut b = Bindings::default();
        // Button 3 is the gameplay video, which nobody has to have.
        b.adopt([("video".to_owned(), None)], [("3".to_owned(), None)]);
        assert_eq!(b.key_for("video"), None, "the key came back");
        assert_eq!(b.pad_map()[&3], None, "the button came back");
    }

    /// Except where it would leave the pad broken.
    ///
    /// Somebody arriving with Confirm unbound in the old storage would
    /// otherwise land on a controller that opens nothing, with nothing on
    /// screen to say why — the same repair that protects a rebind protects
    /// this, and the two have to agree.
    #[test]
    fn adopting_cannot_leave_an_essential_action_on_no_button() {
        let mut b = Bindings::default();
        b.adopt([], [("0".to_owned(), None)]);
        assert_eq!(
            b.pad_map()[&0].as_deref(),
            Some("activate"),
            "the pad arrived with nothing to open a game with"
        );
    }

    /// A newer build's action, or a button index that is not one, would
    /// otherwise be written back into config.toml and sit there forever.
    #[test]
    fn nonsense_is_dropped_rather_than_carried_over() {
        let mut b = Bindings::default();
        b.adopt(
            [("teleport".to_owned(), Some("t".to_owned()))],
            [("banana".to_owned(), Some("back".to_owned()))],
        );
        assert!(
            b.keys.is_empty(),
            "an unknown action was carried over: {:?}",
            b.keys
        );
        assert!(
            b.pad.is_empty(),
            "a nonsense button was carried over: {:?}",
            b.pad
        );
    }

    #[test]
    fn labels_are_human() {
        assert_eq!(key_label(Some("ArrowLeft")), "←");
        assert_eq!(key_label(Some("s")), "S");
        assert_eq!(key_label(None), "—");
        assert_eq!(pad_label(Some(0)), "A / Cross (bottom face)");
        assert_eq!(pad_label(Some(99)), "button 99");
        assert_eq!(pad_label(None), "unset");
    }

    /// The webview's tests run against a stand-in backend rather than this
    /// one, and its copy of the default tables is a fixture on disk. This
    /// keeps the two in step: a new action or a moved default button fails
    /// here, with the fix being to regenerate the file.
    ///
    /// Run `UPDATE_FIXTURES=1 cargo test` to write it.
    #[test]
    fn the_webview_test_fixture_matches_these_tables() {
        let b = Bindings::default();
        let want = serde_json::json!({
            "actions": ACTIONS.iter().map(|a| serde_json::json!({
                "id": a.id, "label": a.label,
            })).collect::<Vec<_>>(),
            "pad_buttons": PAD_BUTTONS.iter().map(|p| serde_json::json!({
                "index": p.index, "name": p.name,
            })).collect::<Vec<_>>(),
            "pad_map": b.pad_map(),
            "keys": ACTIONS.iter().map(|a| (a.id, b.key_for(a.id))).collect::<BTreeMap<_, _>>(),
            "pad_labels": ACTIONS.iter()
                .map(|a| (a.id, pad_label(b.pad_for(a.id))))
                .collect::<BTreeMap<_, _>>(),
            "key_labels": ACTIONS.iter()
                .map(|a| (a.id, key_label(b.key_for(a.id).as_deref())))
                .collect::<BTreeMap<_, _>>(),
        });
        let want = serde_json::to_string_pretty(&want).expect("serialising the fixture") + "\n";

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/ui/test/bindings.fixture.json");
        if std::env::var("UPDATE_FIXTURES").is_ok() {
            std::fs::write(path, &want).expect("writing the fixture");
            return;
        }
        // Line endings normalized on both sides. Git checks this file out with
        // CRLF on Windows unless told otherwise, so a byte-for-byte comparison
        // fails there and only there — which is a red CI run about nothing,
        // and the fix it suggests would produce a file that then fails
        // everywhere else.
        let have = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            have.replace("\r\n", "\n"),
            want.replace("\r\n", "\n"),
            "ui/test/bindings.fixture.json is out of step with these tables — \
             run UPDATE_FIXTURES=1 cargo test to bring it up to date"
        );
    }

    /// Every action Settings offers has to be reachable, and no id may appear
    /// twice — a duplicate row rebinds whichever copy the click landed on.
    #[test]
    fn no_action_is_listed_twice() {
        let mut seen = std::collections::BTreeSet::new();
        for a in ACTIONS {
            assert!(seen.insert(a.id), "{} is listed twice", a.id);
        }
    }
}

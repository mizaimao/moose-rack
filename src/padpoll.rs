// Reading a controller: deadzones, repeat timings, and the locks around a
// game launch.
//
// This is the one module here the Tauri front end does not call. The webview
// polls the Gamepad API inside `requestAnimationFrame`, 120 times a second on
// this display, and a round trip into the backend per frame is not a thing
// that can be made fast enough. `ui/js/gamepad.js` keeps its own copy of the
// arithmetic below and this is the definition it is a copy of — so when a
// deadzone or a repeat delay is argued about, it is argued about once.
//
// Everything here takes plain numbers. The W3C standard mapping and
// SDL_GameController disagree about which integer is the bottom face button,
// so a front end translates into the indices [`crate::binds::PAD_BUTTONS`]
// names before asking anything here.

use std::collections::{BTreeMap, BTreeSet};

/// How long a held button waits before it starts repeating, and how fast it
/// repeats after that. Long enough that a deliberate single press never
/// doubles; short enough that holding a direction crosses a list.
pub const FIRST_REPEAT_MS: f64 = 380.0;
pub const REPEAT_MS: f64 = 110.0;

/// Below this a stick is resting, or drifting. Sticks wear and center badly,
/// and a cursor that creeps on its own is worse than one that needs a firmer
/// push.
pub const STICK_DEADZONE: f64 = 0.55;

/// Below this a trigger is resting, or being brushed. Lower than the stick's
/// because a trigger is pulled deliberately and has no center to drift around.
pub const TRIGGER_DEADZONE: f64 = 0.06;

/// Pixels a frame at a full pull. About a screen every third of a second on a
/// laptop display — fast enough to cross a long list, slow enough to stop on
/// something.
pub const TRIGGER_TOP_SPEED: f64 = 34.0;

/// The same idea for the right stick over the info pane, which is shorter.
pub const STICK_SCROLL_SPEED: f64 = 26.0;

/// A minimum on the lock after an emulator exits. See [`settle_lifted`].
pub const SETTLE_FLOOR_MS: f64 = 200.0;

/// How long a button must be held to count as a hold rather than a press.
pub const HOLD_MS: f64 = 2000.0;

/// Held directions repeat; one-shot buttons do not.
///
/// A held Confirm that repeated would launch the same game over and over; a
/// held direction that did not would make a 2,506-game list unusable.
pub fn repeatable(action: &str) -> bool {
    matches!(
        action,
        "up" | "down" | "left" | "right" | "pageUp" | "pageDown" | "zoomIn" | "zoomOut"
    )
}

/// The left stick as a d-pad, along its dominant axis only.
///
/// Pushed diagonally it would otherwise report left and up in the same frame
/// and move twice, which reads as the cursor jumping around on its own.
pub fn stick_direction(x: f64, y: f64) -> Option<&'static str> {
    if x.abs() > y.abs() {
        if x < -STICK_DEADZONE {
            return Some("left");
        }
        if x > STICK_DEADZONE {
            return Some("right");
        }
    } else {
        if y < -STICK_DEADZONE {
            return Some("up");
        }
        if y > STICK_DEADZONE {
            return Some("down");
        }
    }
    None
}

/// How fast a pulled trigger scrolls, in pixels a frame.
///
/// Squared, so a light pull creeps and a full pull moves properly. A list of
/// two thousand games is exactly what an analogue control is for. A pad whose
/// triggers are digital reports 0 or 1 and falls through to the press handler,
/// which scrolls a fixed step — so it still does something.
pub fn trigger_scroll(pull: f64) -> f64 {
    if pull <= TRIGGER_DEADZONE {
        return 0.0;
    }
    pull * pull * TRIGGER_TOP_SPEED
}

/// The same shape for the right stick over the info pane, signed by direction.
pub fn stick_scroll(value: f64) -> f64 {
    if value.abs() <= STICK_DEADZONE {
        return 0.0;
    }
    value.signum() * value * value * STICK_SCROLL_SPEED
}

/// Which actions the pad is asking for this frame.
///
/// `buttons` is indexed by the numbers [`crate::binds::PAD_BUTTONS`] names;
/// `map` is a resolved [`crate::binds::Bindings::pad_map`], whose `None`
/// entries are buttons a rebind cleared and must be skipped rather than
/// dispatched.
pub fn pressed_actions(
    buttons: &[bool],
    axes: &[f64],
    map: &BTreeMap<u8, Option<String>>,
) -> BTreeSet<String> {
    let mut pressed = BTreeSet::new();
    for (index, action) in map {
        let Some(action) = action else { continue };
        if buttons.get(*index as usize).copied().unwrap_or(false) {
            pressed.insert(action.clone());
        }
    }
    if let Some(dir) = stick_direction(
        axes.first().copied().unwrap_or(0.0),
        axes.get(1).copied().unwrap_or(0.0),
    ) {
        pressed.insert(dir.to_owned());
    }
    pressed
}

/// Whether the lock after an emulator exits may lift.
///
/// The quit hotkey is Select + A, and both are bound in the library too — so
/// the moment the emulator exits and this window takes focus again, the
/// still-held buttons read as fresh presses: the game relaunches and settings
/// open behind it.
///
/// Waiting for release alone turned out not to be enough. Coming back the pad
/// can report *nothing* held for a frame or two before its real state arrives,
/// so the still-held combo read as released, the lock lifted, and the same
/// buttons then arrived as a fresh press. A short floor under the lock covers
/// that gap: input is ignored until the pad is at rest **and** the floor has
/// passed.
pub fn settle_lifted(pressed: &BTreeSet<String>, now: f64, settle_until: f64) -> bool {
    pressed.is_empty() && now >= settle_until
}

/// The repeat state machine: which held actions should act this frame.
#[derive(Debug, Default)]
pub struct Repeat {
    /// action -> the time it may next fire.
    held: BTreeMap<String, f64>,
}

impl Repeat {
    /// Whether `action` should act now. Newly pressed acts immediately and
    /// then waits out the initial delay; a held one acts again only if it is
    /// [`repeatable`].
    pub fn fire(&mut self, action: &str, now: f64) -> bool {
        match self.held.get(action).copied() {
            None => {
                self.held.insert(action.to_owned(), now + FIRST_REPEAT_MS);
                true
            }
            Some(due) if now >= due && repeatable(action) => {
                self.held.insert(action.to_owned(), now + REPEAT_MS);
                true
            }
            Some(_) => false,
        }
    }

    /// Whether `action` should act now, once only, until it is released.
    ///
    /// For the places a repeat would be wrong however long the button is down:
    /// a held Confirm inside a dialog would press the button sixty times a
    /// second, and a held Back would close the dialog and then act on the
    /// library underneath it in the same press.
    pub fn once(&mut self, action: &str, now: f64) -> bool {
        if self.held.contains_key(action) {
            return false;
        }
        self.held.insert(action.to_owned(), now);
        true
    }

    /// Forget anything no longer held, so the next press fires immediately.
    pub fn release(&mut self, pressed: &BTreeSet<String>) {
        self.held.retain(|action, _| pressed.contains(action));
    }

    /// Drop everything.
    ///
    /// For a change of context — a player opening over the library — where
    /// what the pad was holding belonged to the old one. Callers usually want
    /// [`Self::rebuild`] instead.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Restart from what is pressed right now, as though it had just been.
    ///
    /// The button that caused a change of context is usually still down — the
    /// same button opens the video player and closes it — so emptying the held
    /// set made the very next poll treat that held button as a new press, and
    /// the player opened and shut in the same breath.
    pub fn rebuild(&mut self, pressed: &BTreeSet<String>, now: f64) {
        self.held = pressed
            .iter()
            .map(|a| (a.clone(), now + FIRST_REPEAT_MS))
            .collect();
    }
}

/// Hold a button rather than pressing it, for the one action worth two.
///
/// Only the picture cycle so far: it is seven long, and the miximage is the
/// one people come back to — a screenshot, the box and the logo in one
/// picture, which is the setting that suits every console at once. Six presses
/// to get home from the wrong end of a list is how a good control becomes an
/// annoying one.
#[derive(Debug, Default)]
pub struct Hold {
    since: BTreeMap<String, Option<f64>>,
}

/// What a frame of held buttons produced.
pub struct Held {
    /// Holds that completed this frame. Each fires once, not once per frame
    /// for as long as the button stays down.
    pub fired: Vec<String>,
    /// Actions a completed hold has taken over, so the caller does not also
    /// fire them as an ordinary press on the way down. A tap is unaffected:
    /// an action is only consumed once the hold has actually happened.
    pub consumed: BTreeSet<String>,
}

impl Hold {
    pub fn poll(&mut self, pressed: &BTreeSet<String>, watched: &[&str], now: f64) -> Held {
        let mut out = Held { fired: Vec::new(), consumed: BTreeSet::new() };
        for action in watched {
            if !pressed.contains(*action) {
                self.since.remove(*action);
                continue;
            }
            match self.since.get(*action).copied() {
                None => {
                    self.since.insert((*action).to_owned(), Some(now));
                }
                Some(Some(started)) if now - started >= HOLD_MS => {
                    // Marked rather than removed, so it fires once for the
                    // hold and not again every frame the button stays down.
                    self.since.insert((*action).to_owned(), None);
                    out.fired.push((*action).to_owned());
                    out.consumed.insert((*action).to_owned());
                }
                Some(None) => {
                    out.consumed.insert((*action).to_owned());
                }
                Some(Some(_)) => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binds::Bindings;

    fn buttons(down: &[usize]) -> Vec<bool> {
        let mut b = vec![false; 17];
        for i in down {
            b[*i] = true;
        }
        b
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Ported from `ui/test/gamepad.test.js`, "the poll loop".
    #[test]
    fn the_face_buttons_resolve_to_open_and_back() {
        let map = Bindings::default().pad_map();
        assert_eq!(pressed_actions(&buttons(&[0]), &[], &map), set(&["activate"]));
        assert_eq!(pressed_actions(&buttons(&[1]), &[], &map), set(&["back"]));
    }

    #[test]
    fn an_unbound_button_resolves_to_nothing() {
        let map = Bindings::default().pad_map();
        assert!(pressed_actions(&buttons(&[16]), &[], &map).is_empty());
    }

    /// A rebind clears the old button by writing an empty binding over it. A
    /// front end dispatching that as an action is the bug this guards.
    #[test]
    fn a_button_cleared_by_a_rebind_is_skipped() {
        let mut b = Bindings::default();
        b.set_pad("activate", Some(3));
        assert!(pressed_actions(&buttons(&[0]), &[], &b.pad_map()).is_empty());
    }

    /// Pushed diagonally, reporting both would move the cursor twice in one
    /// frame, which reads as it jumping around on its own.
    #[test]
    fn the_stick_moves_along_its_dominant_axis_only() {
        assert_eq!(stick_direction(-0.9, -0.7), Some("left"));
        assert_eq!(stick_direction(-0.7, -0.9), Some("up"));
        assert_eq!(stick_direction(0.9, 0.7), Some("right"));
        assert_eq!(stick_direction(0.7, 0.9), Some("down"));
    }

    #[test]
    fn a_resting_stick_reports_nothing() {
        assert_eq!(stick_direction(0.2, -0.3), None);
        assert_eq!(stick_direction(0.0, 0.0), None);
    }

    /// A pad reporting fewer buttons than the map names must not be read past
    /// its end — a poll that fails is a controller that silently stops working.
    #[test]
    fn a_short_button_list_is_not_read_off_the_end() {
        let map = Bindings::default().pad_map();
        assert_eq!(pressed_actions(&[true], &[], &map), set(&["activate"]));
        assert!(pressed_actions(&[], &[], &map).is_empty());
    }

    /// Ported from "the triggers scroll by how hard they are pulled".
    #[test]
    fn a_harder_pull_scrolls_further_than_a_light_one() {
        let light = trigger_scroll(0.3);
        let hard = trigger_scroll(1.0);
        assert!(light > 0.0, "a light pull did nothing at all");
        assert!(hard > light * 2.0, "a full pull ({hard}) barely beat a light one ({light})");
    }

    /// A trigger at rest reports a small value on plenty of pads, and a list
    /// that creeps on its own is worse than one that does not move.
    #[test]
    fn a_resting_trigger_does_not_creep() {
        assert_eq!(trigger_scroll(0.03), 0.0);
        assert_eq!(stick_scroll(0.2), 0.0);
    }

    #[test]
    fn the_right_stick_scrolls_both_ways() {
        assert!(stick_scroll(-0.9) < 0.0);
        assert!(stick_scroll(0.9) > 0.0);
    }

    /// Ported from "the lock after the emulator exits".
    #[test]
    fn the_lock_does_not_lift_on_the_empty_frame_after_an_exit() {
        let until = 1000.0 + SETTLE_FLOOR_MS;
        assert!(
            !settle_lifted(&BTreeSet::new(), 1000.0, until),
            "an empty frame immediately after the exit is the pad catching up, not a release"
        );
        assert!(settle_lifted(&BTreeSet::new(), 1500.0, until));
        assert!(
            !settle_lifted(&set(&["activate"]), 1500.0, until),
            "the floor is a minimum, not a replacement for waiting"
        );
    }

    #[test]
    fn a_new_press_acts_at_once_then_waits_out_the_delay() {
        let mut r = Repeat::default();
        assert!(r.fire("down", 0.0), "the first press did nothing");
        assert!(!r.fire("down", 100.0), "it repeated before the initial delay");
        assert!(r.fire("down", FIRST_REPEAT_MS), "it never started repeating");
        assert!(!r.fire("down", FIRST_REPEAT_MS + 10.0));
        assert!(r.fire("down", FIRST_REPEAT_MS + REPEAT_MS));
    }

    /// A held Confirm that repeated would launch the same game over and over.
    #[test]
    fn a_one_shot_button_never_repeats_however_long_it_is_held() {
        let mut r = Repeat::default();
        assert!(r.fire("activate", 0.0));
        for t in [FIRST_REPEAT_MS, 5_000.0, 60_000.0] {
            assert!(!r.fire("activate", t), "Confirm repeated at {t}ms");
        }
    }

    #[test]
    fn releasing_lets_the_next_press_fire_at_once() {
        let mut r = Repeat::default();
        r.fire("activate", 0.0);
        r.release(&BTreeSet::new());
        assert!(r.fire("activate", 10.0), "the button had to be waited out after release");
    }

    /// The same button opens the video player and closes it, so it is still
    /// down when the context changes. Emptying the held set made the very next
    /// poll treat it as a new press.
    #[test]
    fn rebuilding_on_a_context_change_does_not_re_press_the_held_button() {
        let mut r = Repeat::default();
        r.rebuild(&set(&["video"]), 0.0);
        assert!(!r.fire("video", 10.0), "the player opened and shut in one press");
        // And a genuinely new press still arrives.
        assert!(r.fire("back", 10.0));
    }

    /// Six presses to get home from the wrong end of a seven-long list is how
    /// a good control becomes an annoying one.
    #[test]
    fn two_seconds_of_a_button_is_a_hold_and_fires_once() {
        let mut h = Hold::default();
        let held = set(&["pictures"]);
        assert!(h.poll(&held, &["pictures"], 0.0).fired.is_empty());
        assert!(h.poll(&held, &["pictures"], HOLD_MS - 1.0).fired.is_empty());

        let done = h.poll(&held, &["pictures"], HOLD_MS);
        assert_eq!(done.fired, ["pictures"]);

        let after = h.poll(&held, &["pictures"], HOLD_MS + 500.0);
        assert!(after.fired.is_empty(), "the hold fired again while the button stayed down");
        assert!(after.consumed.contains("pictures"), "the release would fire it as a press");
    }

    /// A tap is unaffected: it is only consumed once the hold has happened.
    #[test]
    fn a_tap_is_not_swallowed_by_the_hold() {
        let mut h = Hold::default();
        let out = h.poll(&set(&["pictures"]), &["pictures"], 0.0);
        assert!(out.consumed.is_empty(), "a tap was eaten by the hold");
        h.poll(&BTreeSet::new(), &["pictures"], 100.0);
        // And letting go resets the clock rather than banking the time.
        let out = h.poll(&set(&["pictures"]), &["pictures"], HOLD_MS);
        assert!(out.fired.is_empty(), "two taps added up to a hold");
    }
}

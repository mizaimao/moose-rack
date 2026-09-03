//! Four controllers, four players.
//!
//! RetroArch assigns a connected pad to the next free port by itself, so two
//! identical controllers usually work with no help. Two *different* ones often
//! do not, and the reason is autoconfig: a pad is bound from the profile
//! RetroArch has for that exact device on that exact input driver, and a
//! controller with no profile gets nothing. Player one is fine because this app
//! already resolves a profile for the pad the frontend can see; players two to
//! four were never considered at all.
//!
//! Mirroring copies player one's bindings onto the other ports. It is right far
//! more often than it is wrong — the second pad on a desk is usually the same
//! model as the first, and a spare Xbox pad and a spare 8BitDo report the same
//! indices anyway — but it is wrong for a genuinely different device, so it is
//! a setting rather than an assumption. It only ever *adds* bindings: a port
//! whose own controller RetroArch already knows keeps what autoconfig gave it,
//! because these lines are written before the user's own config, not after.

use crate::padprofile::{PadProfile, Physical};

/// How many ports to enable.
///
/// Four because that is what the consoles in this library had — the NES and
/// Mega Drive with a multitap, the N64 and Saturn natively — and because the
/// cost of an unused port is zero. RetroArch defaults to five, which is not a
/// number any of this hardware has.
pub const MAX_PLAYERS: u8 = 4;

/// Every button worth copying to another port.
///
/// The face buttons, shoulders, stick clicks, Start/Select and the d-pad. Not
/// the triggers as axes and not the sticks: those come from `analog_dpad_mode`
/// and from the core, and copying an axis binding across ports is how a
/// half-pressed trigger on one pad starts moving another player.
const MIRRORED: &[Physical] = &[
    Physical::A,
    Physical::B,
    Physical::X,
    Physical::Y,
    Physical::LB,
    Physical::RB,
    Physical::L3,
    Physical::R3,
    Physical::Select,
    Physical::Start,
    Physical::Up,
    Physical::Down,
    Physical::Left,
    Physical::Right,
];

/// Config lines for multi-player support.
///
/// `mirror` copies player one's buttons onto ports two to four. Without a
/// profile there is nothing to copy and only the port count is written — which
/// is still worth doing, because a fifth port RetroArch offers and no console
/// here has is a port a second player's pad can get lost in.
pub fn config_lines(profile: Option<&PadProfile>, mirror: bool) -> String {
    let mut out = String::from(
        "\n# ---- Players ----\n\
         # Four ports, which is what the hardware in this library had. RetroArch\n\
         # defaults to five, and a port no console has is one a second player's\n\
         # pad can be assigned to and then appear dead.\n",
    );
    out.push_str(&format!("input_max_users = \"{MAX_PLAYERS}\"\n"));

    out.push_str(
        "\n# Left stick doubles as the d-pad, on every port.\n\
         #\n\
         # Every console here predates the analog stick, so the stick is\n\
         # otherwise dead in every game -- and it is where a thumb naturally\n\
         # sits on a modern pad. Mode 1 is \"Left Analog\": the d-pad keeps\n\
         # working, the stick simply also reports it, so nothing is taken away.\n",
    );
    for p in 1..=MAX_PLAYERS {
        out.push_str(&format!("input_player{p}_analog_dpad_mode = \"1\"\n"));
    }

    let Some(profile) = profile.filter(|_| mirror) else {
        return out;
    };

    out.push_str(&format!(
        "\n# Players 2-{MAX_PLAYERS} bound like player 1 ({}).\n\
         #\n\
         # RetroArch binds a pad from its own autoconfig profile for that exact\n\
         # device on that exact driver, and a controller it has no profile for\n\
         # gets nothing at all -- which looks like the port being dead. Copying\n\
         # player 1 is right whenever the other pads are the same model, which\n\
         # is the usual case, and is a setting because it is not always true.\n\
         #\n\
         # These are written before your own config, so a port whose controller\n\
         # RetroArch does know keeps what it was given.\n",
        if profile.device.is_empty() { "unknown pad" } else { &profile.device },
    ));

    for p in 2..=MAX_PLAYERS {
        for button in MIRRORED {
            if let Some(bind) = profile.get(*button) {
                out.push_str(&bind.line(&format!("player{p}_{}", button.retropad())));
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile with the buttons an ordinary pad reports.
    fn profile() -> PadProfile {
        PadProfile::parse(
            "input_device = \"Xbox Wireless Controller\"\n\
             input_driver = \"mfi\"\n\
             input_a_btn = \"1\"\n\
             input_b_btn = \"0\"\n\
             input_x_btn = \"3\"\n\
             input_y_btn = \"2\"\n\
             input_l_btn = \"4\"\n\
             input_r_btn = \"5\"\n\
             input_start_btn = \"9\"\n\
             input_select_btn = \"8\"\n\
             input_up_btn = \"h0up\"\n\
             input_down_btn = \"h0down\"\n\
             input_left_btn = \"h0left\"\n\
             input_right_btn = \"h0right\"\n",
        )
    }

    /// Five ports is RetroArch's default and no console in this library has
    /// five. A pad assigned to the port that does not exist looks broken.
    #[test]
    fn four_ports_are_enabled() {
        let out = config_lines(None, true);
        assert!(out.contains("input_max_users = \"4\""), "{out}");
    }

    /// The stick standing in for the d-pad was written for player one only, so
    /// the second player's stick did nothing in a game where the first
    /// player's worked.
    #[test]
    fn the_stick_stands_in_for_the_dpad_on_every_port() {
        let out = config_lines(None, false);
        for p in 1..=4 {
            assert!(
                out.contains(&format!("input_player{p}_analog_dpad_mode = \"1\"")),
                "player {p} missing:\n{out}"
            );
        }
    }

    #[test]
    fn mirroring_binds_the_other_three_ports_like_the_first() {
        let out = config_lines(Some(&profile()), true);
        for p in 2..=4 {
            // b is RetroPad's own name for the bottom face button.
            assert!(out.contains(&format!("input_player{p}_b_btn = \"0\"")), "{out}");
            assert!(out.contains(&format!("input_player{p}_start_btn = \"9\"")), "{out}");
            assert!(out.contains(&format!("input_player{p}_up_btn = \"h0up\"")), "{out}");
        }
        // Player one is left alone: it is bound elsewhere, from the same
        // profile, and writing it twice would only invite the two to drift.
        assert!(!out.contains("input_player1_b_btn"), "{out}");
    }

    /// Off means off. Someone with two different controllers turns this off
    /// precisely because RetroArch's own profile for the second one is right
    /// and player one's indices are not.
    #[test]
    fn without_mirroring_no_other_port_is_bound() {
        let out = config_lines(Some(&profile()), false);
        for p in 2..=4 {
            assert!(!out.contains(&format!("input_player{p}_b_btn")), "{out}");
        }
        // The port count and the stick still apply: those are true whatever
        // the controllers are.
        assert!(out.contains("input_max_users"));
        assert!(out.contains("input_player3_analog_dpad_mode"));
    }

    /// Nothing to copy is not a failure. It happens on the first launch after
    /// a fresh RetroArch install, before any profile exists.
    #[test]
    fn no_profile_still_enables_the_ports() {
        let out = config_lines(None, true);
        assert!(out.contains("input_max_users = \"4\""));
        assert!(!out.contains("input_player2_b_btn"));
    }

    /// Triggers are reported as an axis on most pads, and an axis copied to
    /// another port means a half-pressed trigger on one controller moves a
    /// different player.
    #[test]
    fn analog_triggers_are_not_copied_across_ports() {
        let with_axis = PadProfile::parse(
            "input_device = \"Pad\"\n\
             input_b_btn = \"0\"\n\
             input_l2_axis = \"+2\"\n\
             input_r2_axis = \"+5\"\n",
        );
        let out = config_lines(Some(&with_axis), true);
        assert!(out.contains("input_player2_b_btn"), "{out}");
        assert!(!out.contains("_l2_axis"), "a trigger axis was copied:\n{out}");
        assert!(!out.contains("_r2_axis"), "{out}");
    }
}

//! Systems played on a keyboard: the keyboard goes to the game.
//!
//! RetroArch binds thirty keyboard hotkeys out of the box, and a DOS game uses
//! most of the keys they sit on. Space is fast-forward and DOOM's use key, Esc
//! quits RetroArch, F1 opens its menu, P pauses, R rewinds, F goes full screen.
//! Found by playing DOOM (docs/dos.md): the game ran several times too fast and
//! RetroArch's log said `FAST_FORWARD` every time the player opened a door.
//!
//! The pad hotkeys in `padprofile` do not help here. They gate RetroArch's
//! actions behind a pad button, and a DOS game is mostly played with no pad.
//!
//! So for these systems every keyboard hotkey is unbound, the menu moves to
//! F12, which nothing claims by default, and the mouse is grabbed whenever the
//! window has focus. RetroArch does not capture the mouse in a window
//! otherwise, and DOOM's mouse did nothing at all.

/// RetroArch 1.20.0's keyboard hotkeys that have a key by default, with that
/// key.
///
/// From `config.def.keybinds.h` (the desktop branch, `retro_keybinds_1`) and
/// named as `configuration.c` writes them. Every one of these is a key a DOS
/// game might want, which is the only reason it is listed.
pub const DEFAULT_HOTKEYS: &[(&str, &str)] = &[
    ("input_menu_toggle", "f1"),
    ("input_exit_emulator", "escape"),
    ("input_reset", "h"),
    ("input_toggle_fast_forward", "space"),
    ("input_hold_fast_forward", "l"),
    ("input_hold_slowmotion", "e"),
    ("input_rewind", "r"),
    ("input_pause_toggle", "p"),
    ("input_frame_advance", "k"),
    ("input_audio_mute", "f9"),
    ("input_volume_up", "add"),
    ("input_volume_down", "subtract"),
    ("input_load_state", "f4"),
    ("input_save_state", "f2"),
    ("input_state_slot_increase", "f7"),
    ("input_state_slot_decrease", "f6"),
    ("input_shader_toggle", "comma"),
    ("input_shader_next", "m"),
    ("input_shader_prev", "n"),
    ("input_cheat_toggle", "u"),
    ("input_cheat_index_plus", "y"),
    ("input_cheat_index_minus", "t"),
    ("input_screenshot", "f8"),
    ("input_grab_mouse_toggle", "f11"),
    ("input_game_focus_toggle", "scroll_lock"),
    ("input_toggle_fullscreen", "f"),
    ("input_desktop_menu_toggle", "f5"),
    ("input_fps_toggle", "f3"),
    ("input_netplay_game_watch", "i"),
    ("input_netplay_player_chat", "tilde"),
];

/// Where the menu goes instead of F1.
pub const MENU_KEY: &str = "f12";

/// Systems whose games are played on a keyboard.
const SYSTEMS: &[&str] = &["dos"];

pub fn applies(platform: &str) -> bool {
    SYSTEMS.contains(&platform)
}

/// Launch config handing the keyboard to the game, or nothing for a system
/// played on a pad.
pub fn config_lines(platform: &str) -> String {
    if !applies(platform) {
        return String::new();
    }
    let mut s = String::from(
        "\n# A keyboard game: RetroArch's keyboard hotkeys are off so every key\n\
         # reaches the game, the menu is on F12, and the mouse is grabbed.\n",
    );
    for (action, _) in DEFAULT_HOTKEYS {
        if *action != "input_menu_toggle" {
            s.push_str(&format!("{action} = \"nul\"\n"));
        }
    }
    s.push_str(&format!("input_menu_toggle = \"{MENU_KEY}\"\n"));
    s.push_str("input_auto_mouse_grab = \"true\"\n");
    s
}

pub fn describe(platform: &str) -> Option<String> {
    applies(platform)
        .then(|| "keyboard and mouse go to the game; RetroArch's menu is on F12".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dos_game_gets_every_key_but_the_menu() {
        let lines = config_lines("dos");
        for (action, key) in DEFAULT_HOTKEYS {
            if *action == "input_menu_toggle" {
                continue;
            }
            assert!(
                lines.contains(&format!("{action} = \"nul\"\n")),
                "{action} would still take {key} from the game"
            );
        }
        assert!(lines.contains("input_menu_toggle = \"f12\"\n"));
        assert!(lines.contains("input_auto_mouse_grab = \"true\"\n"));
    }

    #[test]
    fn the_menu_key_is_one_nothing_else_has() {
        // Moving the menu onto a key another default hotkey sits on would
        // leave that hotkey unbound below and the menu fighting nothing -- but
        // the day the list changes, it would be the menu that went missing.
        assert!(DEFAULT_HOTKEYS.iter().all(|(_, key)| *key != MENU_KEY));
    }

    #[test]
    fn a_pad_system_is_left_alone() {
        assert_eq!(config_lines("snes"), "");
        assert_eq!(describe("snes"), None);
    }
}

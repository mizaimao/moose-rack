//! Playing light gun games with the mouse.
//!
//! Two separate things have to be true before a gun game works, and having
//! only one of them is why this normally looks broken:
//!
//! 1. **The mouse has to be bound to the gun controls.** RetroArch keeps the
//!    light gun binds (`gun_trigger`, `gun_offscreen_shot`, `gun_start`, …)
//!    apart from the pad binds, and ships them unbound. Aiming works out of the
//!    box because the pointer position is read directly, but the trigger does
//!    nothing. Those binds are in `retroarch.rs`'s launch overrides, always on:
//!    they cost nothing when no gun is plugged in.
//!
//! 2. **The core has to be told a gun is plugged in.** Cores default every port
//!    to a joypad. Until the port is switched to the console's gun the game
//!    sees no gun at all, however the mouse is bound.
//!
//! Step 2 is what this module does, and it is opt-in per system because it is
//! not free: on most of these consoles the gun goes in **port 2**, which is the
//! port the second player's pad would otherwise occupy. Turning it on for a
//! whole platform would quietly break every two-player game on it. So it is a
//! switch in Systems, next to the core and shader choices, off by default.
//!
//! ## Where the numbers come from
//!
//! libretro device ids are built from a base and a subclass:
//!
//! ```text
//! RETRO_DEVICE_LIGHTGUN            = 4
//! RETRO_DEVICE_SUBCLASS(base, id)  = ((id + 1) << 8) | base
//! ```
//!
//! so a core's first light gun is `(1 << 8) | 4` = 260 and its second is 516.
//! Every console below has exactly one gun, and the cores that emulate it
//! declare it as their first subclass — the Zapper, the Super Scope, the
//! Menacer, the GunCon. Hence 260 throughout, and the one plain `4` for MAME,
//! which exposes the generic light gun rather than a named peripheral.

/// Which port the gun goes in, and what to call it when reporting back.
struct Gun {
    /// 1-based, matching `input_libretro_device_pN`.
    port: u8,
    device: u32,
    name: &'static str,
}

/// The gun for a platform, if this project knows of one.
///
/// Keyed on the platform rather than the core because the peripheral is a
/// property of the console: the NES had a Zapper whichever emulator you use.
fn gun_for(platform: &str) -> Option<Gun> {
    let g = match platform {
        // The Zapper plugged into the second controller port.
        "nes" | "famicom" => Gun { port: 2, device: 260, name: "Zapper" },
        // The Super Scope was a receiver in port 2 as well.
        "snes" | "sfam" | "sfc" => Gun { port: 2, device: 260, name: "Super Scope" },
        // The Menacer, on the Mega Drive's second port.
        "genesis" | "megadrive" | "sms" | "mastersystem" => {
            Gun { port: 2, device: 260, name: "Menacer" }
        }
        // The GunCon was a PlayStation controller, so it takes a whole port —
        // and gun games are one-player, so it takes the first.
        "psx" => Gun { port: 1, device: 260, name: "GunCon" },
        "saturn" => Gun { port: 1, device: 260, name: "Stunner" },
        // Arcade cabinets had the gun wired in as player 1's controls.
        "arcade" | "mame" | "naomi" => Gun { port: 1, device: 4, name: "light gun" },
        _ => return None,
    };
    Some(g)
}

/// Whether a system can be switched to a gun at all.
///
/// Systems offers the switch only where it would do something.
pub fn supported(platform: &str) -> bool {
    gun_for(platform).is_some()
}

/// What the gun is called on this system, for the Systems row and the launch
/// notes. `None` when there is no gun to offer.
pub fn label(platform: &str) -> Option<&'static str> {
    gun_for(platform).map(|g| g.name)
}

/// Launch config putting the gun in its port, or nothing when the system has
/// no gun or the switch is off.
pub fn config_lines(platform: &str, enabled: bool) -> String {
    if !enabled {
        return String::new();
    }
    let Some(g) = gun_for(platform) else {
        return String::new();
    };
    format!(
        "\n# Light gun on, from Systems. The {} sits in port {} — while this is\n\
         # on, that port is a gun and not a second pad.\n\
         input_libretro_device_p{} = \"{}\"\n",
        g.name, g.port, g.port, g.device
    )
}

/// A line for the launch summary, so it is obvious why player two stopped
/// working if the switch was left on.
pub fn describe(platform: &str, enabled: bool) -> Option<String> {
    if !enabled {
        return None;
    }
    let g = gun_for(platform)?;
    Some(format!(
        "light gun: {} in port {} — aim with the mouse, left button fires",
        g.name, g.port
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_writes_nothing() {
        assert_eq!(config_lines("nes", false), "");
        assert_eq!(describe("nes", false), None);
    }

    /// The switch must be inert on a system with no gun, or a stale config
    /// entry would put a gun device on a console that has none.
    #[test]
    fn a_system_with_no_gun_writes_nothing_even_when_enabled() {
        assert!(!supported("gb"));
        assert_eq!(config_lines("gb", true), "");
        assert_eq!(describe("gb", true), None);
    }

    /// The port matters more than the device id: writing the gun to port 1 on a
    /// console whose gun lived in port 2 leaves the game reading a pad.
    #[test]
    fn console_guns_go_in_the_port_the_hardware_used() {
        assert!(config_lines("nes", true).contains("input_libretro_device_p2 = \"260\""));
        assert!(config_lines("snes", true).contains("input_libretro_device_p2 = \"260\""));
        // One-controller-port peripherals, and the arcade, take port 1.
        assert!(config_lines("psx", true).contains("input_libretro_device_p1 = \"260\""));
        assert!(config_lines("arcade", true).contains("input_libretro_device_p1 = \"4\""));
    }

    /// Both slugs for the same console have to behave alike, or the switch
    /// works on one and silently does nothing on the other.
    #[test]
    fn alternate_slugs_for_one_console_agree() {
        for (a, b) in [("nes", "famicom"), ("snes", "sfam"), ("genesis", "megadrive")] {
            assert_eq!(config_lines(a, true), config_lines(b, true), "{a} vs {b}");
        }
    }
}

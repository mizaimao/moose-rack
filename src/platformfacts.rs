//! What each console is, in a sentence.
//!
//! ES-DE's themes carry this: pick a system and the screen tells you who made
//! it, when it arrived, whether it is a console, a handheld or a cabinet, and
//! a line about what it was. The library here had none of it — a console was a
//! name, a picture and a number, and everything else about it lived in
//! Settings.
//!
//! A table in the binary rather than a file to ship: it is thirty lines of
//! facts that have not changed in twenty years, and a file is one more thing
//! to find, parse and fail to find.
//!
//! Release years are the *first* release anywhere, which is what ES-DE uses
//! and is the only unambiguous choice — the Mega Drive is 1988 in Japan, 1989
//! in America and 1990 in Europe, and picking the local one means the answer
//! depends on who is reading.

/// One console, as the preview pane shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub manufacturer: &'static str,
    /// First release anywhere.
    pub released: u16,
    /// Console, Handheld, Arcade — ES-DE's own three.
    pub hardware: &'static str,
    pub blurb: &'static str,
}

/// The facts for a platform slug, if this is one we know.
pub fn of(slug: &str) -> Option<Facts> {
    let f = |manufacturer, released, hardware, blurb| {
        Some(Facts { manufacturer, released, hardware, blurb })
    };
    match slug {
        "3do" => f(
            "The 3DO Company",
            1993,
            "Console",
            "A licensed standard rather than a machine: Panasonic, Sanyo and GoldStar each \
             built one. It launched at $699 and was outsold by everything.",
        ),
        "arcade" | "mame" => f(
            "Various",
            1978,
            "Arcade",
            "Coin-operated hardware, one board per game, built to be difficult enough to \
             keep the coins coming.",
        ),
        "dc" => f(
            "Sega",
            1998,
            "Console",
            "Sega's last console, and the first with a modem in the box. Discontinued \
             eighteen months into the PlayStation 2's life.",
        ),
        "famicom" | "nes" => f(
            "Nintendo",
            1983,
            "Console",
            "The machine that restarted the home games business after the 1983 crash. Sold \
             as the Family Computer in Japan and the NES everywhere else.",
        ),
        "gamegear" => f(
            "Sega",
            1990,
            "Handheld",
            "A backlit color handheld against the Game Boy's gray screen — and six AA \
             batteries for three hours of it.",
        ),
        "gb" => f(
            "Nintendo",
            1989,
            "Handheld",
            "A worse screen than its rivals and ten times the battery life. It sold for \
             fourteen years.",
        ),
        "gba" => f(
            "Nintendo",
            2001,
            "Handheld",
            "A 32-bit handheld with the Game Boy's library still in it, and for two years \
             no way to light the screen.",
        ),
        "gbc" => f(
            "Nintendo",
            1998,
            "Handheld",
            "Color, eight years into the Game Boy's life, and it still played every \
             original cartridge.",
        ),
        "mastersystem" => f(
            "Sega",
            1985,
            "Console",
            "Better hardware than the Famicom and almost no third-party games, because \
             Nintendo's contracts said so. Enormous in Brazil.",
        ),
        "megadrive" => f(
            "Sega",
            1988,
            "Console",
            "16 bits and an attitude to match. The machine that made the console war a \
             marketing campaign.",
        ),
        "n64" => f(
            "Nintendo",
            1996,
            "Console",
            "Cartridges when everyone else had moved to discs — expensive, and with no \
             loading times. It brought the analogue stick to a control pad.",
        ),
        "naomi" => f(
            "Sega",
            1998,
            "Arcade",
            "Dreamcast hardware in a cabinet, so arcade conversions came home nearly \
             unchanged.",
        ),
        "nds" => f(
            "Nintendo",
            2004,
            "Handheld",
            "Two screens, one of them a touchscreen, at a moment when nobody was asking \
             for either. The best-selling handheld ever made.",
        ),
        "neo-geo-pocket" => f(
            "SNK",
            1998,
            "Handheld",
            "A clicky microswitched stick on a handheld, and a fighting-game library to \
             use it on.",
        ),
        "neogeoaes" => f(
            "SNK",
            1990,
            "Console",
            "The arcade board sold for the living room, at $650 with $200 cartridges. \
             Identical hardware to the cabinet, which was the whole point.",
        ),
        "neogeocd" => f(
            "SNK",
            1994,
            "Console",
            "The Neo Geo's games on CD at a tenth of the cartridge price, and loading \
             times to match.",
        ),
        "ngc" => f(
            "Nintendo",
            2001,
            "Console",
            "A small purple cube with a handle, playing 8cm discs that were too small for \
             a DVD player to bother pirating.",
        ),
        "pcengine" => f(
            "NEC",
            1987,
            "Console",
            "An 8-bit CPU with 16-bit graphics, sold as the TurboGrafx-16 in America. \
             First console with a CD add-on.",
        ),
        "psp" => f(
            "Sony",
            2004,
            "Handheld",
            "Console-scale games on a handheld, on a proprietary disc nobody else ever \
             used.",
        ),
        "psx" => f(
            "Sony",
            1994,
            "Console",
            "Nintendo's cancelled CD add-on, built by the partner it walked away from. It \
             took the industry with it.",
        ),
        "saturn" => f(
            "Sega",
            1994,
            "Console",
            "Two CPUs, eight processors and a launch four months early that nobody was \
             ready for. Superb in Japan, lost everywhere else.",
        ),
        "sfc" | "snes" => f(
            "Nintendo",
            1990,
            "Console",
            "Mode 7, a sound chip by the man who later built the PlayStation's, and the \
             back catalog most people mean by \"retro\".",
        ),
        "wii" => f(
            "Nintendo",
            2006,
            "Console",
            "Weaker than both rivals and outsold both, by putting the controller in the \
             air.",
        ),
        "wonderswan" => f(
            "Bandai",
            1999,
            "Handheld",
            "Designed by the Game Boy's own creator, played held either way up, and ran \
             for forty hours on one AA battery.",
        ),
        "wonderswancolor" => f(
            "Bandai",
            2000,
            "Handheld",
            "Color, and the Final Fantasy remakes that were the reason to own one. Japan \
             only.",
        ),
        "g-and-w" => f(
            "Nintendo",
            1980,
            "Handheld",
            "One game per machine, on an LCD with the picture printed into it. It paid for \
             the Famicom, and gave the world the d-pad.",
        ),
        "pico8" => f(
            "Lexaloffle",
            2015,
            "Console",
            "A console that never existed, with limits invented on purpose: 128×128, \
             sixteen colors, and a cartridge you can fit in a PNG.",
        ),
        "easyrpg" => f(
            "ASCII / Enterbrain",
            1997,
            "Console",
            "Games made in RPG Maker 2000 and 2003, run by an open reimplementation of the \
             engine rather than the original.",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every platform this library can launch should say something about
    /// itself. A console with no facts draws a preview with three empty rows,
    /// which reads as a page that failed to load.
    #[test]
    fn every_platform_in_the_core_map_has_facts() {
        // The slugs `doctor` lists on the machine this was built for.
        const KNOWN: &[&str] = &[
            "3do", "arcade", "dc", "famicom", "gamegear", "gb", "gba", "gbc", "mame",
            "mastersystem", "megadrive", "n64", "naomi", "nds", "neo-geo-pocket", "neogeoaes",
            "nes", "ngc", "pcengine", "psp", "psx", "saturn", "sfc", "snes", "wii", "wonderswan",
            "wonderswancolor", "easyrpg", "g-and-w", "pico8",
        ];
        for slug in KNOWN {
            assert!(of(slug).is_some(), "{slug} has nothing to say about itself");
        }
    }

    /// Slugs that are the same machine share an entry rather than drifting into
    /// two descriptions of one console.
    #[test]
    fn the_same_machine_under_two_slugs_says_the_same_thing() {
        assert_eq!(of("nes"), of("famicom"));
        assert_eq!(of("snes"), of("sfc"));
        assert_eq!(of("arcade"), of("mame"));
    }

    /// A year nobody can check is worse than no year. These are the ones that
    /// are quoted everywhere and would be noticed.
    #[test]
    fn the_famous_years_are_right() {
        assert_eq!(of("nes").unwrap().released, 1983, "the Famicom is 1983");
        assert_eq!(of("snes").unwrap().released, 1990, "the Super Famicom is 1990");
        assert_eq!(of("psx").unwrap().released, 1994);
        assert_eq!(of("gb").unwrap().released, 1989);
        assert_eq!(of("n64").unwrap().released, 1996);
        assert_eq!(of("dc").unwrap().released, 1998);
    }

    /// ES-DE's three, and nothing else — the preview prints this word as it is.
    #[test]
    fn hardware_is_one_of_three_words() {
        for slug in ["nes", "gb", "arcade", "naomi", "psp", "pico8"] {
            let h = of(slug).unwrap().hardware;
            assert!(
                matches!(h, "Console" | "Handheld" | "Arcade"),
                "{slug} is a {h}, which is not one of the three"
            );
        }
    }
}

//! Sample library: real titles from Frank's shelves, so the grid reads like his
//! rather than like lorem ipsum. Years are North American release years.

struct RawPlatform {
    name: &'static str,
    short: &'static str,
    rgb: (u8, u8, u8),
    /// Typical size of one game, in MB, for the made-up sizes.
    typical_mb: u32,
    games: Vec<RawGame>,
}

struct RawGame {
    title: &'static str,
    year: u16,
}

fn games(list: &[(&'static str, u16)]) -> Vec<RawGame> {
    list.iter().map(|&(title, year)| RawGame { title, year }).collect()
}

fn raw() -> Vec<RawPlatform> {
    vec![
        RawPlatform {
            name: "PlayStation 2",
            short: "PS2",
            rgb: (52, 84, 196),
            typical_mb: 3800,
            games: games(&[
                ("Ratchet & Clank", 2002),
                ("Ratchet & Clank: Going Commando", 2003),
                ("Ratchet & Clank: Up Your Arsenal", 2004),
                ("Ratchet: Deadlocked", 2005),
                ("Sly Cooper and the Thievius Raccoonus", 2002),
                ("Sly 2: Band of Thieves", 2004),
                ("Sly 3: Honor Among Thieves", 2005),
                ("Jak and Daxter: The Precursor Legacy", 2001),
                ("Jak II", 2003),
                ("Jak 3", 2004),
                ("God of War", 2005),
                ("God of War II", 2007),
                ("Shadow of the Colossus", 2005),
                ("Ico", 2001),
                ("Okami", 2006),
                ("Kingdom Hearts", 2002),
                ("Kingdom Hearts II", 2006),
                ("Psychonauts", 2005),
                ("Katamari Damacy", 2004),
                ("Onimusha 3: Demon Siege", 2004),
                ("Onimusha: Dawn of Dreams", 2006),
                ("Metal Gear Solid 3: Snake Eater", 2004),
                ("Devil May Cry 3: Special Edition", 2006),
                ("Dark Cloud 2", 2003),
                ("Klonoa 2: Lunatea's Veil", 2001),
                ("Final Fantasy X", 2001),
                ("Dragon Quest VIII", 2005),
                ("Beyond Good & Evil", 2003),
                ("Burnout 3: Takedown", 2004),
                ("Gran Turismo 4", 2005),
            ]),
        },
        RawPlatform {
            name: "Xbox",
            short: "XBOX",
            rgb: (58, 140, 62),
            typical_mb: 3200,
            games: games(&[
                ("Jet Set Radio Future", 2002),
                ("Ninja Gaiden Black", 2005),
                ("Panzer Dragoon Orta", 2003),
                ("Crimson Skies: High Road to Revenge", 2003),
                ("Otogi: Myth of Demons", 2003),
                ("Otogi 2: Immortal Warriors", 2004),
                ("Halo: Combat Evolved", 2001),
                ("Star Wars: Knights of the Old Republic", 2003),
                ("Fable", 2004),
                ("Conker: Live & Reloaded", 2005),
            ]),
        },
        RawPlatform {
            name: "Super Nintendo",
            short: "SNES",
            rgb: (104, 74, 196),
            typical_mb: 3,
            games: games(&[
                ("Super Metroid", 1994),
                ("Chrono Trigger", 1995),
                ("The Legend of Zelda: A Link to the Past", 1992),
                ("Super Mario World", 1991),
                ("Donkey Kong Country", 1994),
                ("EarthBound", 1995),
                ("Final Fantasy III", 1994),
                ("Super Mario RPG", 1996),
            ]),
        },
        RawPlatform {
            name: "Game Boy Advance",
            short: "GBA",
            rgb: (86, 70, 170),
            typical_mb: 16,
            games: games(&[
                ("Metroid Fusion", 2002),
                ("Advance Wars", 2001),
                ("Golden Sun", 2001),
                ("Fire Emblem", 2003),
                ("The Legend of Zelda: The Minish Cap", 2005),
                ("Pokemon Emerald", 2005),
            ]),
        },
        RawPlatform {
            name: "Genesis",
            short: "MD",
            rgb: (178, 44, 44),
            typical_mb: 2,
            games: games(&[
                ("Sonic the Hedgehog 2", 1992),
                ("Streets of Rage 2", 1992),
                ("Gunstar Heroes", 1993),
                ("Shinobi III", 1993),
            ]),
        },
        RawPlatform {
            name: "Nintendo 64",
            short: "N64",
            rgb: (196, 98, 30),
            typical_mb: 32,
            games: games(&[
                ("Super Mario 64", 1996),
                ("The Legend of Zelda: Ocarina of Time", 1998),
                ("GoldenEye 007", 1997),
                ("Banjo-Kazooie", 1998),
                ("Mario Kart 64", 1997),
            ]),
        },
        RawPlatform {
            name: "Dreamcast",
            short: "DC",
            rgb: (214, 132, 20),
            typical_mb: 1000,
            games: games(&[
                ("Jet Set Radio", 2000),
                ("Shenmue", 2000),
                ("Soulcalibur", 1999),
                ("Crazy Taxi", 2000),
            ]),
        },
    ]
}

/// The sample in the app's shape. Sizes are made up from each console's
/// typical size, about a third of the games count as downloaded, and a few are
/// favorites, so every tab has something in it.
pub fn platforms() -> Vec<crate::model::Platform> {
    use crate::model::{Game, Platform, hash};
    const FAVORITES: [&str; 5] =
        ["Ratchet & Clank: Up Your Arsenal", "Sly 2: Band of Thieves", "Okami", "Jet Set Radio Future", "Super Metroid"];
    let mut id = 0;
    raw()
        .into_iter()
        .map(|p| Platform {
            slug: p.short.to_lowercase(),
            name: p.name.to_owned(),
            short: p.short.to_owned(),
            rgb: p.rgb,
            aspect: None,
            games: p
                .games
                .into_iter()
                .map(|g| {
                    id += 1;
                    Game {
                        id,
                        title: g.title.to_owned(),
                        year: Some(i32::from(g.year)),
                        bytes: u64::from(p.typical_mb) * (60 + hash(g.title) % 80) / 100 * 1_000_000,
                        installed: hash(g.title) % 3 == 0,
                        favorite: FAVORITES.contains(&g.title),
                        cover: None,
                    }
                })
                .collect(),
        })
        .collect()
}

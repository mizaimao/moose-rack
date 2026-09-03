//! Per-system artwork from an ES-DE theme, one file at a time.
//!
//! Themes name their pictures for the ES-DE system — `snes.svg`, `3do.png` —
//! and put them somewhere different in every repository. `data/icon-set-art.toml`
//! records where, surveyed from the repositories themselves.
//!
//! ## Why this exists at all
//!
//! The Icon sets tab first previewed a theme with the author's own screenshots,
//! which show the theme's whole interface — its menus, its game grids, its
//! backgrounds. None of that is what this app takes from a theme. It takes the
//! console pictures and nothing else, so the preview was advertising something
//! the download does not deliver.
//!
//! Showing the real console pictures instead means fetching them, and fetching
//! them used to mean cloning the repository: hundreds of megabytes to keep a
//! few hundred kilobytes, which is affordable once and impossible nine times
//! over for a tab whose entire purpose is looking before you download.
//!
//! Individual files over raw HTTP cost a few kilobytes each. That is what makes
//! previewing nine sets, and installing one, the same cheap operation.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Where each set keeps its art. Compiled in — see `data/icon-set-art.toml`.
pub const TABLE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/icon-set-art.toml"
));

/// One look a theme offers: a directory of per-system pictures.
#[derive(Debug, Clone, Deserialize)]
pub struct Look {
    /// Stable slug — the folder the pictures are kept in and the value written
    /// to `icons.style`.
    pub id: String,
    /// What the picker and the Select toast print.
    pub label: String,
    pub dir: String,
    pub ext: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetArt {
    /// `owner/repo` on GitHub.
    pub repo: String,
    pub branch: String,
    /// Everything this theme draws, in the order the Select button cycles it —
    /// hardware first, then controllers, then wordmarks.
    #[serde(default)]
    pub looks: Vec<Look>,
}

impl SetArt {
    /// Raw URL of one system's picture in one look.
    ///
    /// Spaces are percent-encoded: one theme's directory is literally
    /// `_inc/systems/artwork (modern)`, and an unencoded space makes the
    /// request a 400 rather than a miss.
    pub fn url(&self, look: &str, system: &str) -> Option<String> {
        let l = self.looks.iter().find(|l| l.id == look)?;
        Some(format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}.{}",
            self.repo,
            self.branch,
            encode_path(&l.dir),
            encode_path(system),
            l.ext
        ))
    }

    pub fn look(&self, id: &str) -> Option<&Look> {
        self.looks.iter().find(|l| l.id == id)
    }

    /// The look to preview a set with: the first, which the table orders
    /// hardware-first because that is where a designer's hand shows. Every
    /// theme's wordmark of the SNES is the same Nintendo logotype.
    pub fn best_look(&self) -> Option<&Look> {
        self.looks.first()
    }

    /// A stable summary of which directory each look came from.
    ///
    /// Written beside a downloaded set so a later run can tell whether the art
    /// on disk was fetched under the mapping now in force. The mapping has been
    /// wrong twice, and correcting it does nothing for art already downloaded:
    /// the pictures sit in folders named for what they were mistaken for.
    pub fn fingerprint(&self) -> String {
        self.looks
            .iter()
            .map(|l| format!("{}={}.{}", l.id, l.dir, l.ext))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// True when everything this set draws is a wordmark — the systems' names
    /// rather than the systems.
    pub fn wordmarks_only(&self) -> bool {
        !self.looks.is_empty() && self.looks.iter().all(|l| l.id.starts_with("styled-text"))
    }

    /// True when the set photographs the real console.
    pub fn has_hardware(&self) -> bool {
        self.looks.iter().any(|l| l.id.starts_with("hardware"))
    }
}

/// Only the characters a path segment must not carry literally. Deliberately
/// not a general-purpose encoder: these are repository paths, and mangling a
/// legitimate `(` or `-` would turn a working URL into a 404.
fn encode_path(s: &str) -> String {
    s.replace(' ', "%20")
}

/// The set the console grid draws from when nothing has been chosen.
///
/// Meringue: one of only ten sets that photograph the real console, and it
/// pairs them with wordmarks, so the whole rotation comes from one designer.
pub const DEFAULT_SET: &str = "meringue-es-de";

/// The nine sets asked for by name, listed before everything else.
///
/// A separate constant rather than an ordering column in the TOML: the table is
/// regenerated wholesale by the survey tool, and anything hand-maintained in
/// there would be lost on the next run.
pub const ORDER_FIRST: &[&str] = &[
    "codywheel-es-de",
    "diamond-es-de",
    "elegance-es-de",
    "elementerial-es-de",
    "iconic-es-de",
    "immersive-revisited-es-de",
    "meringue-es-de",
    "razor-es-de",
    "retromega-revisited-es-de",
];

/// The compiled-in table. Infallible by construction — the test below runs at
/// build time, so a malformed file fails CI rather than a user.
pub fn table() -> BTreeMap<String, SetArt> {
    toml::from_str(TABLE).expect("the embedded icon-set art table is valid TOML")
}

/// Every set, in the order the tab lists them: the nine by name, then the rest
/// alphabetically.
pub fn ordered() -> Vec<(String, SetArt)> {
    let mut all: Vec<(String, SetArt)> = table().into_iter().collect();
    all.sort_by_key(|(name, _)| {
        (
            ORDER_FIRST
                .iter()
                .position(|f| f == name)
                .unwrap_or(usize::MAX),
            name.clone(),
        )
    });
    all
}

/// What one set holds, for the tab to show without fetching anything.
/// Every icon set the shipped table describes.
///
/// The front ends offer these as a list to choose from; a set is a name in a
/// table, not something a person should be typing.
pub fn set_ids() -> Vec<String> {
    let Ok(table) = toml::from_str::<toml::Table>(TABLE) else {
        return Vec::new();
    };
    table.keys().cloned().collect()
}

pub fn of(set: &str) -> Option<SetArt> {
    table().remove(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses_and_every_set_offers_at_least_one_look() {
        let t = table();
        assert!(
            t.len() > 40,
            "only {} sets — did the survey run short?",
            t.len()
        );
        for (name, art) in &t {
            assert!(
                art.repo.contains('/'),
                "{name}: repo must be owner/repo, got {}",
                art.repo
            );
            assert!(!art.branch.is_empty(), "{name} has no branch");
            assert!(!art.looks.is_empty(), "{name} lists no looks at all");
            for l in &art.looks {
                assert!(
                    !l.id.is_empty() && !l.label.is_empty() && !l.dir.is_empty(),
                    "{name}: {l:?}"
                );
            }
        }
    }

    /// A look id is a folder name and the value written to `icons.style`, so
    /// two looks in one set sharing an id would overwrite each other on disk.
    #[test]
    fn look_ids_are_unique_within_a_set_and_safe_as_folder_names() {
        for (name, art) in table() {
            let mut seen = std::collections::BTreeSet::new();
            for l in &art.looks {
                assert!(
                    seen.insert(l.id.clone()),
                    "{name} has two looks called {}",
                    l.id
                );
                assert!(
                    l.id.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                    "{name}: {:?} is not usable as a folder name",
                    l.id
                );
            }
        }
    }

    /// The whole point of the change: a theme offers what it offers. Squeezing
    /// every one into three fixed kinds is what put controllers under
    /// "Hardware" and threw away six of Canvas's nine looks.
    #[test]
    fn sets_offer_as_many_looks_as_they_have() {
        let t = table();
        let counts: Vec<usize> = t.values().map(|a| a.looks.len()).collect();
        assert!(counts.contains(&1), "some sets draw one thing");
        assert!(
            counts.iter().any(|n| *n > 3),
            "and some draw more than three"
        );
        assert_eq!(t["iconic-es-de"].looks.len(), 2);
        assert!(t["canvas-es-de"].looks.len() >= 8, "Canvas draws nine");
    }

    /// Directories with spaces exist upstream, and an unencoded space makes the
    /// request a 400 rather than a miss. Built rather than taken from the table
    /// so a survey that drops the last such directory does not drop the guard.
    #[test]
    fn a_directory_with_a_space_survives_the_url() {
        let art: SetArt = toml::from_str(
            "repo = \"o/r\"\nbranch = \"main\"\n\
             [[looks]]\nid = \"hardware\"\nlabel = \"Hardware\"\n\
             dir = \"_inc/art (modern)\"\next = \"webp\"",
        )
        .unwrap();
        let u = art.url("hardware", "snes").unwrap();
        assert!(u.contains("art%20(modern)"), "space must be encoded: {u}");
        assert!(!u.contains("art (modern)"), "and not left raw: {u}");
    }

    /// A preview shows the console when there is one: every theme's wordmark of
    /// a given system is the same logotype and tells them apart not at all.
    #[test]
    fn a_preview_prefers_the_console() {
        assert_eq!(
            of("codywheel-es-de").unwrap().best_look().unwrap().id,
            "hardware"
        );
        assert_eq!(
            of("meringue-es-de").unwrap().best_look().unwrap().id,
            "hardware"
        );
        // Nothing else on offer.
        assert_eq!(
            of("razor-es-de").unwrap().best_look().unwrap().id,
            "styled-text"
        );
        assert_eq!(
            of("iconic-es-de").unwrap().best_look().unwrap().id,
            "controller"
        );
    }

    /// Sets that draw only the system's name. The tab says so on the card.
    #[test]
    fn the_wordmark_only_sets_are_flagged() {
        for set in ["diamond-es-de", "elegance-es-de", "razor-es-de"] {
            assert!(of(set).unwrap().wordmarks_only(), "{set} is wordmarks only");
        }
        for set in ["codywheel-es-de", "iconic-es-de", "meringue-es-de"] {
            assert!(
                !of(set).unwrap().wordmarks_only(),
                "{set} draws more than wordmarks"
            );
        }
    }

    /// The default set has to exist and photograph the real console — the last
    /// is why it was chosen over Iconic, which draws controllers.
    #[test]
    fn the_default_set_draws_the_hardware() {
        let art = of(DEFAULT_SET).expect("the default set must be in the table");
        assert!(art.has_hardware(), "the default draws the real console");
        assert!(art.looks.len() >= 2, "and has something to cycle to");
    }

    /// Ten of fifty-five photograph the hardware, which is why finding one by
    /// browsing was hopeless and the default is one of them.
    #[test]
    fn the_sets_that_draw_real_consoles_are_still_there() {
        let with_hw: Vec<String> = table()
            .into_iter()
            .filter(|(_, a)| a.has_hardware())
            .map(|(k, _)| k)
            .collect();
        assert!(
            with_hw.len() >= 8,
            "only {} sets draw hardware",
            with_hw.len()
        );
        for want in ["meringue-es-de", "codywheel-es-de", "playstation-x-es-de"] {
            assert!(
                with_hw.iter().any(|k| k == want),
                "{want} draws the console"
            );
        }
    }

    /// Every URL is an anonymous CDN fetch. `gh` builds this table offline;
    /// nothing at runtime needs an account, a token, or the rate-limited API.
    #[test]
    fn the_urls_need_no_account() {
        for (name, art) in table() {
            let u = art.url(&art.best_look().unwrap().id, "snes").unwrap();
            assert!(
                u.starts_with("https://raw.githubusercontent.com/"),
                "{name} builds {u}, which is not the anonymous raw host"
            );
            assert!(
                !u.contains("api.github.com"),
                "{name} uses the rate-limited API"
            );
            assert!(
                !u.contains('@') && !u.contains("token"),
                "{name} embeds a credential: {u}"
            );
        }
    }

    #[test]
    fn an_unknown_set_is_none_rather_than_a_panic() {
        assert!(of("no-such-theme").is_none());
    }

    /// The nine asked for by name lead the list, in the order they were asked
    /// for, and nothing is lost behind them.
    #[test]
    fn the_named_sets_come_first_and_the_rest_follow() {
        let all = ordered();
        assert_eq!(all.len(), table().len(), "ordering must not drop a set");
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            &names[..ORDER_FIRST.len()],
            ORDER_FIRST,
            "the nine, in order"
        );
        assert!(names[ORDER_FIRST.len()..].windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn every_named_set_is_in_the_table() {
        let t = table();
        for name in ORDER_FIRST {
            assert!(
                t.contains_key(*name),
                "{name} is named first but not in the table"
            );
        }
    }
}

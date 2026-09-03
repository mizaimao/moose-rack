//! The two TOML files that ship beside the app, checked as data.
//!
//! `config.example.toml` is the file the release notes tell a new user to copy,
//! and until this module existed nothing ever read it. A renamed field, a value
//! the app rejects, or a comment offering a setting that does not exist would
//! all reach a user before anyone noticed — and two did: the template
//! documented `list_art = "cartridge"` and `motion = "strobe"`, and neither is
//! a value the app accepts.
//!
//! So every value either file sets is checked against the code that consumes
//! it, and so is every alternative their comments offer.
//!
//! The template is compiled in; `config.toml` is *read from disk* instead. It
//! holds a server token, and `include_str!` would bake that into every binary.

/// The documented template, compiled in.
///
/// Embedded rather than read from beside the app because on Android there is no
/// "beside the app": the binary is inside an APK and the data directory is
/// private, so a user cannot put a file there and the release notes' "copy
/// config.example.toml to config.toml" is advice nobody can follow. See
/// [`seed_config`].
///
/// Only the *template* is compiled in. `config.toml` is still read from disk,
/// because it holds a server token and `include_str!` would bake that into
/// every binary.
pub const TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.toml"));

/// Write the template to `path` when nothing is there yet.
///
/// Returns whether it wrote one. A device with no config showed an empty
/// library, an empty screen and a message telling the user to copy a file into
/// a directory they cannot reach — measured on a fresh Retroid Pocket Mini V2,
/// where the whole app was a tab bar and nothing else.
///
/// Never overwrites: a config that exists is the user's, whatever is in it.
pub fn seed_config(path: &std::path::Path) -> bool {
    if path.exists() {
        return false;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, TEMPLATE).is_ok()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    const EXAMPLE: &str = super::TEMPLATE;

    /// The developer's own config.toml, when there is one. Absent in CI and in
    /// a fresh clone, which is why every test that uses it tolerates `None`
    /// rather than failing.
    fn current() -> Option<String> {
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml")).ok()
    }

    /// Values offered inside a section's comments — the five `[icons]` styles,
    /// the seven artwork types.
    ///
    /// Both sections list choices the same way, one per line as `#   <value>`
    /// followed by an optional description, so only the first word is a value.
    /// Taking every word instead read "console with a game" as three of them.
    fn documented_choices(toml: &str, section: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut inside = false;
        for line in toml.lines() {
            let t = line.trim_end();
            if t.starts_with('[') {
                inside = t == section;
                continue;
            }
            if !inside {
                continue;
            }
            if let Some(rest) = t.strip_prefix("#   ")
                && let Some(word) = rest.split_whitespace().next()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                out.insert(word.to_owned());
            }
        }
        out
    }

    /// Both files, so a fix applied to one is not forgotten in the other.
    fn both() -> Vec<(&'static str, String)> {
        let mut v = vec![("config.example.toml", EXAMPLE.to_owned())];
        if let Some(c) = current() {
            v.push(("config.toml", c));
        }
        v
    }

    /// A device with no config has to become a device with one.
    ///
    /// On Android nobody can copy a file into the app's private directory, so
    /// "copy config.example.toml to config.toml" is not a thing a user can do —
    /// and without it the app is a tab bar over an empty screen.
    #[test]
    fn a_missing_config_is_seeded_from_the_template_and_never_overwritten() {
        let dir = std::env::temp_dir().join("moose-rack-seed-config");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");

        assert!(super::seed_config(&path), "a missing config is written");
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, super::TEMPLATE);
        // And it is a config the app can actually load.
        assert!(crate::config::Config::load_from(&path).is_ok());

        std::fs::write(&path, "# mine\n").unwrap();
        assert!(!super::seed_config(&path), "an existing config is left alone");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
    }

    #[test]
    fn the_template_parses_as_a_config() {
        let cfg: crate::config::Config =
            toml::from_str(EXAMPLE).expect("config.example.toml must deserialize into Config");
        assert_eq!(cfg.library.local_root, "./library");
        assert_eq!(cfg.retroarch.installs.len(), 3, "the three probed installs");
        assert!(!cfg.achievements.enabled, "a template must not switch a feature on");
        assert!(cfg.server.token.is_none(), "a template must ship no credential");
    }

    /// A section renamed in `Config` and left alone in the template becomes a
    /// block that quietly does nothing. Serde is deliberately lenient about
    /// unknown keys so an old user config keeps loading; the file we ship has
    /// no such excuse.
    #[test]
    fn neither_file_has_a_section_the_app_ignores() {
        const KNOWN: &[&str] = &[
            "server", "library", "retroarch", "saves", "controllers", "theme", "cores",
            "shaders", "lightgun", "media", "icons", "appearance", "achievements", "cheevos",
            "scraper", "esde", "bindings", "picker_order",
        ];
        for (name, toml) in both() {
            let doc: toml::Value = toml::from_str(&toml).expect(name);
            for section in doc.as_table().unwrap().keys() {
                assert!(KNOWN.contains(&section.as_str()), "{name}: [{section}] is read by nothing");
            }
        }
    }

    #[test]
    fn every_artwork_type_the_files_name_is_one_the_app_accepts() {
        let valid: BTreeSet<&str> =
            crate::media::LIST_ART_CHOICES.iter().map(|(k, _)| *k).collect();
        for (name, toml) in both() {
            let doc: toml::Value = toml::from_str(&toml).expect(name);
            for key in ["list_art", "detail_art"] {
                if let Some(v) = doc.get("media").and_then(|m| m.get(key)).and_then(|v| v.as_str()) {
                    assert!(valid.contains(v), "{name}: {key} = {v:?} is not a choice the app offers");
                }
            }
            for word in documented_choices(&toml, "[media]") {
                assert!(
                    valid.contains(word.as_str()),
                    "{name}: the [media] comment offers {word:?}, which is not an artwork type"
                );
            }
        }
    }

    /// `icons.style` names a look — one the chosen set offers, or a folder in
    /// the shared pool.
    ///
    /// The pool is enumerated from disk and can hold anything an older build or
    /// a hand-copied theme left there, so a value that is not one of the set's
    /// looks cannot be rejected here: `consolegame` is a real pool look with
    /// pictures behind it, and an earlier version of this test failed on it.
    /// What is still worth checking is that the value is usable as a folder
    /// name at all — a stray space or capital is a folder that never resolves.
    #[test]
    fn the_chosen_look_is_a_set_look_or_a_usable_folder_name() {
        for (name, toml) in both() {
            let doc: toml::Value = toml::from_str(&toml).expect(name);
            let Some(icons) = doc.get("icons") else { continue };
            let Some(style) = icons.get("style").and_then(|v| v.as_str()) else { continue };
            assert!(!style.is_empty(), "{name}: icons.style is empty");
            assert!(
                style
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
                "{name}: icons.style = {style:?} is not usable as a folder name"
            );

            // When it *is* one of the chosen set's looks, say so — that is the
            // common case and the one worth naming in a failure.
            let set = icons.get("set").and_then(|v| v.as_str()).unwrap_or("");
            let set = if set.is_empty() { crate::iconart::DEFAULT_SET } else { set };
            if let Some(art) = crate::iconart::of(set)
                && art.look(style).is_none()
            {
                // Not a set look, so it has to be a pool folder. Nothing here
                // can see the disk; the app refuses an id with no folder.
                assert!(
                    !style.contains("styled-text-"),
                    "{name}: icons.style = {style:?} looks like a set look, but {set} \
                     offers {:?}",
                    art.looks.iter().map(|l| &l.id).collect::<Vec<_>>()
                );
            }
        }
    }

    /// Every core either file pins has to be one the ES-DE map knows, or the
    /// line is a typo that shows up as "this game will not launch".
    #[test]
    fn every_core_the_files_pin_is_a_core_that_exists() {
        let map = crate::coremap::CoreMap::embedded();
        for (name, toml) in both() {
            let doc: toml::Value = toml::from_str(&toml).expect(name);
            let Some(cores) = doc.get("cores") else { continue };
            for table in ["overrides", "per_game"] {
                let Some(t) = cores.get(table).and_then(|t| t.as_table()) else { continue };
                for (key, core) in t {
                    let core = core.as_str().unwrap_or_default();
                    assert!(
                        map.label_for(core).is_some(),
                        "{name}: cores.{table} sets {key} to {core:?}, which no ES-DE core is called"
                    );
                }
            }
        }
    }

    /// The header of both files claims they can be diffed, which holds only
    /// while their sections stay in the same relative order.
    #[test]
    fn the_two_files_stay_diffable() {
        fn sections(toml: &str) -> Vec<&str> {
            toml.lines().map(str::trim_end).filter(|l| l.starts_with('[')).collect()
        }
        let Some(mine) = current() else { return };
        let template = sections(EXAMPLE);
        let mut at = 0;
        let mut seen: Vec<&str> = Vec::new();
        for s in sections(&mine) {
            match template[at..].iter().position(|t| *t == s) {
                Some(i) => {
                    at += i + 1;
                    seen.push(s);
                }
                None => panic!(
                    "config.toml has {s} after {:?}, but config.example.toml does not — \
                     the two no longer diff",
                    seen.last()
                ),
            }
        }
    }

    /// The 154 rows moved out of config.toml are no longer in a file anybody
    /// reads, so a typo in one is invisible until a game refuses to start.
    ///
    /// Deliberately not asserting that each row differs from the platform
    /// default: that depends on which `[cores.overrides]` is in force — with
    /// `arcade = "fbneo"` a row pinning mame is doing work, and with
    /// `arcade = "mame"` the same row is a no-op. A property that flips with
    /// configuration is not an invariant, and the first draft of this test
    /// asserted it anyway and failed on 25pacman.
    #[test]
    fn every_core_in_the_shipped_arcade_table_exists() {
        let map = crate::coremap::CoreMap::embedded();
        let table = crate::config::arcade_core_map();
        assert!(table.len() > 100, "only {} rows", table.len());
        for (game, core) in &table {
            assert!(
                map.label_for(core).is_some(),
                "{game} pins {core:?}, which no ES-DE core is called"
            );
        }
    }
}

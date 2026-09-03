//! `knulli.conf`, read the way KNULLI reads it.
//!
//! Two rules matter and both have cost this project real time.
//!
//! **First-wins.** `knulli-settings-get` scans from the top of the file and
//! stops at the first match. A key repeated lower down is read by nothing.
//! `never-sleep` was appended at the bottom for weeks, under the value KNULLI
//! ships on line 319, and did nothing at all while the app reported it ON.
//!
//! **Scopes, most specific first.** A key can be written three ways:
//!
//! ```text
//! global.core=snes9x                        every system
//! snes.core=snes9x                          one system
//! snes["Chrono Trigger (USA).sfc"].core=…   one game
//! ```
//!
//! The game scope beats the system scope beats the global one. Within a
//! scope, first-wins still applies. Those two rules compose in the only
//! sensible way — find the most specific scope that mentions the key, then
//! take its first occurrence — but they are easy to get backwards, so the
//! tests below pin both directions.

use std::collections::HashMap;

/// How specific a line's scope is. Higher wins.
///
/// Ordering is the whole point of the type, so it is derived rather than
/// written out: `Global < System < Game`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Scope {
    Global,
    System,
    Game,
}

/// A parsed `knulli.conf`.
///
/// Only the winning value for each (scope, key) is kept — first-wins is
/// applied while parsing, so a later duplicate is dropped on the floor
/// exactly as `knulli-settings-get` would drop it.
#[derive(Debug, Default)]
pub struct Conf {
    /// `(scope, key) -> value`, first occurrence only.
    values: HashMap<(Scope, String), String>,
    /// The system this was resolved for, e.g. `gba`.
    system: String,
    /// The game key as EmulationStation writes it: the ROM's file name.
    game: String,
}

impl Conf {
    /// Parse the file for one system and one game.
    ///
    /// Lines for other systems and other games are discarded as they are
    /// read; there is no point carrying 13,000 lines of a config around to
    /// answer questions about one launch.
    pub fn parse(text: &str, system: &str, game: &str) -> Self {
        let mut conf = Self {
            values: HashMap::new(),
            system: system.to_string(),
            game: game.to_string(),
        };

        for line in text.lines() {
            let line = line.trim();
            // `#` is a comment. It is also how a disabled setting is written,
            // including the ones moose-patch comments out when it shadows a
            // key KNULLI set higher up — so a commented line is genuinely
            // inactive and must not be read back as a value.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((lhs, value)) = line.split_once('=') else {
                continue;
            };
            let (lhs, value) = (lhs.trim(), value.trim());

            let Some((scope, key)) = classify(lhs, system, game) else {
                continue;
            };

            // First-wins: only insert if this (scope, key) is untouched.
            conf.values
                .entry((scope, key.to_string()))
                .or_insert_with(|| value.to_string());
        }

        conf
    }

    /// The winning value for `key`, most specific scope first.
    pub fn get(&self, key: &str) -> Option<&str> {
        for scope in [Scope::Game, Scope::System, Scope::Global] {
            if let Some(v) = self.values.get(&(scope, key.to_string())) {
                return Some(v);
            }
        }
        None
    }

    /// The system this was parsed for.
    pub fn system(&self) -> &str {
        &self.system
    }

    /// The game key this was parsed for.
    ///
    /// Unused until config generation lands, which needs it for the per-game
    /// override file RetroArch reads.
    #[allow(dead_code)]
    pub fn game(&self) -> &str {
        &self.game
    }
}

/// Work out which scope a left-hand side belongs to, and the bare key.
///
/// Returns `None` for a line that belongs to some other system or game —
/// which is most of the file.
fn classify<'a>(lhs: &'a str, system: &str, game: &str) -> Option<(Scope, &'a str)> {
    if let Some(rest) = lhs.strip_prefix("global.") {
        return Some((Scope::Global, rest));
    }

    // A game-scoped line: `<system>["<game>"].<key>`. The quotes are part of
    // the format, and the game name inside them is the ROM's file name with
    // `=` and `#` stripped — see FileData::getConfigurationName() in
    // knulli-emulationstation, and Emulator.game_settings_name() in configgen.
    if let Some(rest) = lhs.strip_prefix(system)
        && let Some(rest) = rest.strip_prefix("[\"")
        && let Some((name, key)) = rest.split_once("\"]")
        && let Some(key) = key.strip_prefix('.')
    {
        return (name == sanitize_game(game)).then_some((Scope::Game, key));
    }

    // A system-scoped line: `<system>.<key>`.
    if let Some(rest) = lhs.strip_prefix(system)
        && let Some(key) = rest.strip_prefix('.')
    {
        return Some((Scope::System, key));
    }

    None
}

/// EmulationStation's rule for the name it writes between the brackets.
///
/// It strips `=` and `#` from the file name and nothing else — `=` because
/// it would end the key, `#` because it would start a comment. Anything else,
/// spaces and brackets and apostrophes included, is kept verbatim.
pub fn sanitize_game(file_name: &str) -> String {
    file_name.replace(['=', '#'], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# a comment
global.core=snes9x
global.ratio=auto
snes.core=snes9x-alt
snes["Chrono Trigger (USA).sfc"].core=bsnes
gba.core=vba-m
# gba.core=mgba          <- disabled, must not be read
snes.ratio=4/3
"#;

    #[test]
    fn game_scope_beats_system_beats_global() {
        let c = Conf::parse(SAMPLE, "snes", "Chrono Trigger (USA).sfc");
        assert_eq!(c.get("core"), Some("bsnes"), "game scope should win");

        let c = Conf::parse(SAMPLE, "snes", "Super Metroid (USA).sfc");
        assert_eq!(c.get("core"), Some("snes9x-alt"), "system scope should win");

        let c = Conf::parse(SAMPLE, "nes", "anything.nes");
        assert_eq!(c.get("core"), Some("snes9x"), "global is the fallback");
    }

    #[test]
    fn falls_through_to_global_when_the_system_is_silent() {
        // `ratio` is set globally and for snes, but not for gba.
        let c = Conf::parse(SAMPLE, "gba", "x.gba");
        assert_eq!(c.get("ratio"), Some("auto"));
        let c = Conf::parse(SAMPLE, "snes", "x.sfc");
        assert_eq!(c.get("ratio"), Some("4/3"));
    }

    #[test]
    fn commented_lines_are_not_values() {
        // The regression that mattered: moose-patch comments out a key it
        // shadows, and the commented original must stay inert.
        let c = Conf::parse(SAMPLE, "gba", "x.gba");
        assert_eq!(c.get("core"), Some("vba-m"), "the # line must not win");
    }

    #[test]
    fn first_wins_within_a_scope() {
        // This is the rule that broke never-sleep for weeks. KNULLI sets a
        // key high in the file; an append at the bottom is never read.
        let text = "\
system.batterysaver.extendedmode=suspend
system.batterysaver.extendedmode=none
";
        let c = Conf::parse(text, "gba", "x.gba");
        assert_eq!(
            c.get("system.batterysaver.extendedmode"),
            None,
            "unprefixed keys belong to no scope this launch cares about"
        );

        let text = "global.ratio=auto\nglobal.ratio=16/9\n";
        let c = Conf::parse(text, "gba", "x.gba");
        assert_eq!(c.get("ratio"), Some("auto"), "the first one wins, not the last");
    }

    #[test]
    fn a_more_specific_scope_wins_even_when_it_appears_later() {
        // First-wins is *within* a scope. It must not be confused with
        // file order across scopes, or a global line at the top would
        // shadow the system line below it.
        let text = "global.core=aaa\nsnes.core=bbb\n";
        let c = Conf::parse(text, "snes", "x.sfc");
        assert_eq!(c.get("core"), Some("bbb"));
    }

    #[test]
    fn other_systems_and_other_games_are_ignored() {
        let c = Conf::parse(SAMPLE, "gba", "x.gba");
        assert_eq!(c.get("core"), Some("vba-m"));
        // The snes game-scoped line must not leak into a gba launch.
        let c = Conf::parse(SAMPLE, "gba", "Chrono Trigger (USA).sfc");
        assert_eq!(c.get("core"), Some("vba-m"));
    }

    #[test]
    fn game_names_are_sanitized_the_way_es_writes_them() {
        assert_eq!(sanitize_game("Sonic = 2 #1.md"), "Sonic  2 1.md");
        let text = "md[\"Sonic  2 1.md\"].core=picodrive\n";
        let c = Conf::parse(text, "md", "Sonic = 2 #1.md");
        assert_eq!(c.get("core"), Some("picodrive"), "the key is the stripped name");
    }

    #[test]
    fn values_and_keys_are_trimmed_but_inner_spaces_kept() {
        let text = "  global.ratio  =  16/9  \n";
        let c = Conf::parse(text, "gba", "x.gba");
        assert_eq!(c.get("ratio"), Some("16/9"));
    }

    #[test]
    fn a_value_may_contain_an_equals_sign() {
        // Splitting on the *first* `=` matters: shader and path values
        // legitimately contain more of them.
        let text = "global.shader=crt/crt-pi.glslp?x=1\n";
        let c = Conf::parse(text, "gba", "x.gba");
        assert_eq!(c.get("shader"), Some("crt/crt-pi.glslp?x=1"));
    }

    #[test]
    fn junk_lines_do_not_panic_or_poison() {
        let text = "no equals sign here\n[section]\n\n\nglobal.core=ok\n";
        let c = Conf::parse(text, "gba", "x.gba");
        assert_eq!(c.get("core"), Some("ok"));
    }
}

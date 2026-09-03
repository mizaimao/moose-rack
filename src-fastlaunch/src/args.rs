//! The argv EmulationStation hands the launcher.
//!
//! ES runs, verbatim from `es_systems.cfg`:
//!
//! ```text
//! emulatorlauncher %CONTROLLERSCONFIG% -system %SYSTEM% -rom %ROM% \
//!                  -gameinfoxml %GAMEINFOXML% -systemname %SYSTEMNAME%
//! ```
//!
//! `%CONTROLLERSCONFIG%` expands to a run of `-p1index 0 -p1guid … -p1name …`
//! flags, one group per pad. We do not interpret those here — the fast path
//! reads the pad from `es_input.cfg` like configgen does — but they must be
//! carried through untouched so that handing the whole argv back to the
//! Python launcher is lossless.
//!
//! Parsing is deliberately forgiving. Anything surprising is not an error to
//! report; it is a reason to fall back, which the caller does by re-execing
//! the stock launcher with [`Args::argv`].

/// What we need out of the command line, plus the original argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `-system`, e.g. `gba`. The key into `knulli.conf` and the roms folder.
    pub system: Option<String>,
    /// `-rom`, the absolute path to the game.
    pub rom: Option<String>,
    /// `-systemname`, the pretty name, e.g. `Nintendo Game Boy Advance`.
    pub system_name: Option<String>,
    /// `-gameinfoxml`, the gamelist this game came from.
    pub game_info_xml: Option<String>,
    /// Everything as given, for the fallback exec.
    pub argv: Vec<String>,
}

impl Args {
    /// Parse an argv (without the program name).
    pub fn parse<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let argv: Vec<String> = args.into_iter().map(Into::into).collect();

        let mut out = Self {
            system: None,
            rom: None,
            system_name: None,
            game_info_xml: None,
            argv: argv.clone(),
        };

        let mut i = 0;
        while i < argv.len() {
            // Take the value only if there is one and it is not itself a
            // flag. A trailing `-rom` with nothing after it is malformed, and
            // the right answer is to leave the field empty and fall back
            // rather than to swallow the next flag as a path.
            let take = |slot: &mut Option<String>| {
                if let Some(v) = argv.get(i + 1)
                    && !v.starts_with('-')
                {
                    *slot = Some(v.clone());
                }
            };
            match argv[i].as_str() {
                "-system" => take(&mut out.system),
                "-rom" => take(&mut out.rom),
                "-systemname" => take(&mut out.system_name),
                "-gameinfoxml" => take(&mut out.game_info_xml),
                _ => {}
            }
            i += 1;
        }

        out
    }

    /// The ROM's file name, which is the key `knulli.conf` uses for a
    /// game-scoped setting.
    pub fn rom_file_name(&self) -> Option<&str> {
        let rom = self.rom.as_deref()?;
        // Deliberately not `Path::file_name`: the value is whatever ES passed,
        // and a trailing slash or an empty segment should read as "no name"
        // rather than as the parent directory.
        rom.rsplit('/').next().filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<&'static str> {
        vec![
            "-p1index", "0", "-p1guid", "030000005e0400008e02000014010000",
            "-p1name", "Miyoo Flip Gamepad",
            "-system", "gba",
            "-rom", "/userdata/roms/gba/Apotris.gba",
            "-gameinfoxml", "/userdata/roms/gba/gamelist.xml",
            "-systemname", "Nintendo Game Boy Advance",
        ]
    }

    #[test]
    fn reads_the_fields_es_passes() {
        let a = Args::parse(sample());
        assert_eq!(a.system.as_deref(), Some("gba"));
        assert_eq!(a.rom.as_deref(), Some("/userdata/roms/gba/Apotris.gba"));
        assert_eq!(a.system_name.as_deref(), Some("Nintendo Game Boy Advance"));
        assert_eq!(
            a.game_info_xml.as_deref(),
            Some("/userdata/roms/gba/gamelist.xml")
        );
    }

    #[test]
    fn keeps_the_whole_argv_for_the_fallback() {
        let a = Args::parse(sample());
        assert_eq!(a.argv.len(), sample().len(), "nothing may be dropped");
        assert_eq!(a.argv[0], "-p1index", "controller flags survive verbatim");
    }

    #[test]
    fn rom_file_name_is_the_conf_key() {
        let a = Args::parse(sample());
        assert_eq!(a.rom_file_name(), Some("Apotris.gba"));
    }

    #[test]
    fn rom_names_with_spaces_and_brackets_survive() {
        let a = Args::parse(vec![
            "-rom",
            "/userdata/roms/gba/007 - Everything or Nothing (USA, Europe) (En,Fr,De).gba",
        ]);
        assert_eq!(
            a.rom_file_name(),
            Some("007 - Everything or Nothing (USA, Europe) (En,Fr,De).gba")
        );
    }

    #[test]
    fn a_flag_with_no_value_stays_empty() {
        // `-rom` last, nothing after it.
        let a = Args::parse(vec!["-system", "gba", "-rom"]);
        assert_eq!(a.system.as_deref(), Some("gba"));
        assert_eq!(a.rom, None, "must not invent a value");
    }

    #[test]
    fn a_flag_followed_by_a_flag_stays_empty() {
        let a = Args::parse(vec!["-rom", "-system", "gba"]);
        assert_eq!(a.rom, None, "must not swallow the next flag as a path");
        assert_eq!(a.system.as_deref(), Some("gba"));
    }

    #[test]
    fn empty_argv_is_not_a_panic() {
        let a = Args::parse(Vec::<String>::new());
        assert_eq!(a.system, None);
        assert_eq!(a.rom_file_name(), None);
    }

    #[test]
    fn a_trailing_slash_reads_as_no_name() {
        let a = Args::parse(vec!["-rom", "/userdata/roms/gba/"]);
        assert_eq!(a.rom_file_name(), None);
    }
}

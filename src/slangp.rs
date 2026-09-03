//! Chaining one `.slangp` preset onto another.
//!
//! RetroArch loads a single shader preset at a time, so adding a strobe pass on
//! top of a CRT shader means generating a combined preset. A preset is a flat
//! `key = value` list where per-pass keys carry the pass index as a suffix
//! (`shader3`, `filter_linear3`, `alias3`), plus a `shaders = N` count. Merging
//! is therefore mechanical: renumber the second preset's passes to continue
//! after the first's.
//!
//! The one subtlety is paths. `shaderN` is relative to the directory holding
//! the preset that named it, so a merged file written anywhere else breaks
//! every one of them. They are made absolute during the merge, which also lets
//! the generated preset live in our own config directory rather than inside the
//! user's RetroArch install.

use std::path::{Path, PathBuf};

/// A parsed preset: the pass count, and every line that is not `shaders = N`.
#[derive(Debug, Clone)]
pub struct Preset {
    pub passes: usize,
    /// `(key_without_index, index, value)` for per-pass keys, and
    /// `(key, None, value)` for global ones like `parameters` or `textures`.
    entries: Vec<(String, Option<usize>, String)>,
    dir: PathBuf,
}

/// Keys whose value is a path relative to the preset's own directory.
const PATH_KEYS: &[&str] = &["shader"];

impl Preset {
    pub fn parse(text: &str, dir: &Path) -> Self {
        let mut passes = 0;
        let mut raw: Vec<(String, String)> = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_owned();
            let value = value.trim().trim_matches('"').to_owned();
            if key == "shaders" {
                passes = value.parse().unwrap_or(0);
                continue;
            }
            raw.push((key, value));
        }

        // Texture names have to be known before any key is split, because they
        // routinely end in a digit: crt-guest-advanced declares `SamplerLUT1`
        // through `SamplerLUT4`. Splitting those the usual way reads them as
        // pass 1 of a `SamplerLUT` key and renumbers them into nonsense.
        let textures: Vec<String> = raw
            .iter()
            .filter(|(k, _)| k == "textures")
            .flat_map(|(_, v)| v.split(';'))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let is_texture_key = |key: &str| {
            textures.iter().any(|t| {
                key == t
                    || key
                        .strip_prefix(t.as_str())
                        .is_some_and(|rest| rest.starts_with('_'))
            })
        };

        let entries = raw
            .into_iter()
            .map(|(key, value)| {
                if is_texture_key(&key) {
                    return (key, None, value);
                }
                // Split a trailing run of digits off the key: `filter_linear10`
                // is `filter_linear` for pass 10.
                let digits = key.len() - key.trim_end_matches(|c: char| c.is_ascii_digit()).len();
                if digits > 0 {
                    let (name, index) = key.split_at(key.len() - digits);
                    (name.to_owned(), index.parse().ok(), value)
                } else {
                    (key, None, value)
                }
            })
            .collect();

        Self { passes, entries, dir: dir.to_path_buf() }
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self::parse(&text, path.parent().unwrap_or(Path::new("."))))
    }

    /// Render this preset's passes, shifted by `offset`, with paths absolute.
    fn render(&self, offset: usize, out: &mut String) {
        // Global keys are skipped here and emitted by the caller: `parameters`
        // and `textures` lists have to be merged, not repeated.
        for (key, index, value) in &self.entries {
            let Some(i) = index else { continue };
            let value = if PATH_KEYS.contains(&key.as_str()) {
                absolute(&self.dir, value)
            } else {
                value.clone()
            };
            out.push_str(&format!("{key}{} = \"{value}\"\n", i + offset));
        }
    }

    /// Global (non-per-pass) entries, for merging.
    fn globals(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .filter(|(_, i, _)| i.is_none())
            .map(|(k, _, v)| (k.as_str(), v.as_str()))
    }

    /// Names listed in `textures = "A;B;C"`. Each has its own global key
    /// holding a path, which is relative to this preset's directory just as
    /// `shaderN` is — crt-guest-advanced ships four color LUTs this way.
    fn texture_names(&self) -> Vec<&str> {
        self.globals()
            .filter(|(k, _)| *k == "textures")
            .flat_map(|(_, v)| v.split(';'))
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Global entries with texture paths made absolute, ready to emit.
    fn globals_resolved(&self) -> Vec<(String, String)> {
        let textures = self.texture_names();
        self.globals()
            .filter(|(k, _)| *k != "textures" && *k != "parameters")
            .map(|(k, v)| {
                let value = if textures.contains(&k) && !k.contains('_') {
                    absolute(&self.dir, v)
                } else {
                    v.to_owned()
                };
                (k.to_owned(), value)
            })
            .collect()
    }
}

/// Resolve `value` against `dir` unless it is already absolute, always with
/// forward slashes.
///
/// `Path::join` uses the host separator, so on Windows this produced
/// `D:\RetroArch\crt` + `shaders/lut/x.png` = a path mixing both. RetroArch
/// accepts `/` on every platform and its own configs use it throughout, so
/// normalising gives one form that works everywhere and a generated preset
/// that reads the same wherever it was written.
fn absolute(dir: &Path, value: &str) -> String {
    let p = Path::new(value);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        // `..` segments are common in presets (`../stock.slang`) and RetroArch
        // resolves them itself, so leaving them in the joined path is fine.
        dir.join(p)
    };
    joined.to_string_lossy().replace('\\', "/")
}

/// Combine `base` with `extra` appended after it, as a single preset body.
///
/// Texture and parameter lists are unioned rather than overwritten: each is a
/// semicolon-separated list of names, and dropping one silently disables the
/// textures or exposed parameters it referred to.
/// Render one preset with every path in it made absolute.
///
/// A `.slangp` names its shaders relative to the directory holding it, so a
/// preset copied anywhere else refers to files that are not there — and
/// RetroArch fails to load it without saying so, which looks exactly like the
/// shader setting doing nothing. `handheld/ags001.slangp` says
/// `shader0 = shaders/mgba/ags001.slang`, and copied into RetroArch's config
/// directory that resolves to nothing at all.
///
/// The same treatment [`chain`] already gives the presets it merges, for the
/// case where there is only one.
pub fn standalone(base: &Preset) -> String {
    let mut out = String::from(
        "# Generated by moose-rack from the shader chosen in Settings.\n\
         # Paths are absolute: a preset is read relative to its own directory,\n\
         # and this one does not live beside the shaders it names.\n",
    );
    out.push_str(&format!("shaders = {}\n\n", base.passes));
    base.render(0, &mut out);

    for key in ["textures", "parameters"] {
        let merged: Vec<&str> = base
            .globals()
            .filter(|(k, _)| *k == key)
            .flat_map(|(_, v)| v.split(';'))
            .filter(|s| !s.is_empty())
            .collect();
        if !merged.is_empty() {
            out.push_str(&format!("\n{key} = \"{}\"\n", merged.join(";")));
        }
    }
    for (key, value) in base.globals_resolved() {
        out.push_str(&format!("{key} = \"{value}\"\n"));
    }
    out
}

pub fn chain(base: &Preset, extra: &Preset) -> String {
    let mut out = String::from(
        "# Generated by moose-rack: a motion pass chained onto a base preset.\n\
         # Regenerated on every launch — edit the shader choice in Settings.\n",
    );
    out.push_str(&format!("shaders = {}\n\n", base.passes + extra.passes));

    base.render(0, &mut out);
    out.push('\n');
    extra.render(base.passes, &mut out);

    for key in ["textures", "parameters"] {
        let merged: Vec<&str> = base
            .globals()
            .chain(extra.globals())
            .filter(|(k, _)| *k == key)
            .flat_map(|(_, v)| v.split(';'))
            .filter(|s| !s.is_empty())
            .collect();
        if !merged.is_empty() {
            out.push_str(&format!("\n{key} = \"{}\"\n", merged.join(";")));
        }
    }

    // Remaining globals: texture paths, per-texture flags like
    // `SamplerLUT1_linear`, and parameter value overrides. Texture paths are
    // resolved against whichever preset declared them — they are relative in
    // the same way `shaderN` is, and a preset written elsewhere cannot find
    // them otherwise. Base first, so the extra pass can override a shared name.
    for (key, value) in base.globals_resolved().into_iter().chain(extra.globals_resolved()) {
        out.push_str(&format!("{key} = \"{value}\"\n"));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
shaders = 2
shader0 = shaders/guest/stock.slang
filter_linear0 = false
scale_type0 = source
shader1 = shaders/guest/crt.slang
filter_linear1 = true
alias1 = CrtPass
textures = "Mask;Glow"
parameters = "brightness;contrast"
"#;

    const STROBE: &str = r#"
shaders = 1
shader0 = shaders/adaptive_strobe-koko.slang
wrap_mode0 = repeat
parameters = "_ADPT_STROBE_STR"
"#;

    fn chained() -> String {
        let base = Preset::parse(BASE, Path::new("/ra/shaders_slang/crt"));
        let extra = Preset::parse(STROBE, Path::new("/ra/shaders_slang/subframe-bfi"));
        chain(&base, &extra)
    }

    /// A preset copied out of the shader tree has to keep working, which means
    /// every path in it becoming absolute. This is what "the shader setting does
    /// nothing" was: `handheld/ags001.slangp` names `shaders/mgba/ags001.slang`,
    /// and RetroArch read that relative to wherever the copy landed.
    #[test]
    fn one_preset_alone_still_gets_absolute_paths() {
        let base = Preset::parse(BASE, Path::new("/ra/shaders_slang/crt"));
        let out = standalone(&base);
        assert!(out.contains("shaders = 2"));
        assert!(
            out.contains("/ra/shaders_slang/crt/shaders/guest/stock.slang"),
            "relative path survived: {out}"
        );
        assert!(!out.contains("= shaders/guest/"), "nothing may stay relative");
        assert!(out.contains("textures = \"Mask;Glow\""), "globals carried over");
    }

    /// The strobe pass has to run last, after the CRT pass — it modulates the
    /// finished image. Renumbering is what makes that happen.
    #[test]
    fn the_extra_pass_is_appended_after_the_base() {
        let out = chained();
        assert!(out.contains("shaders = 3"));
        assert!(out.contains("shader1 = \"/ra/shaders_slang/crt/shaders/guest/crt.slang\""));
        assert!(
            out.contains("shader2 = \"/ra/shaders_slang/subframe-bfi/shaders/adaptive_strobe-koko.slang\""),
            "the strobe pass becomes pass 2"
        );
    }

    /// Every per-pass key moves with its pass, not just `shaderN`. Leaving
    /// `alias1` or `filter_linear1` behind silently changes how the base
    /// shader renders.
    #[test]
    fn per_pass_keys_move_with_their_pass() {
        let out = chained();
        assert!(out.contains("alias1 = \"CrtPass\""));
        assert!(out.contains("filter_linear1 = \"true\""));
        assert!(out.contains("wrap_mode2 = \"repeat\""), "the strobe's own keys shift too");
    }

    /// Paths are relative to the preset that named them, and the two presets
    /// live in different directories. A merged file elsewhere only works if
    /// they are made absolute.
    #[test]
    fn paths_are_resolved_against_each_presets_own_directory() {
        let out = chained();
        assert!(!out.contains("= \"shaders/"), "no relative paths may survive: {out}");
    }

    #[test]
    fn an_already_absolute_path_is_left_alone() {
        let p = Preset::parse("shaders = 1\nshader0 = /abs/x.slang\n", Path::new("/ra"));
        assert!(chain(&p, &Preset::parse("shaders = 0\n", Path::new("/ra")))
            .contains("shader0 = \"/abs/x.slang\""));
    }

    /// Paths must come out with forward slashes on every host. `Path::join`
    /// uses the platform separator, which on Windows produced a path mixing
    /// both -- and made this suite fail there while passing on macOS.
    #[test]
    fn separators_are_normalized_whatever_the_host() {
        let base = Preset::parse(
            "shaders = 1\nshader0 = shaders/guest/crt.slang\n",
            // A Windows-shaped directory, so this is a real check there and a
            // meaningful one everywhere else.
            Path::new(r"D:\RetroArch\shaders\shaders_slang\crt"),
        );
        let out = chain(&base, &Preset::parse("shaders = 0", Path::new("/ra")));
        let line = out.lines().find(|l| l.starts_with("shader0")).unwrap();
        assert!(!line.contains('\\'), "no backslashes may survive: {line}");
        assert!(line.ends_with("crt/shaders/guest/crt.slang\""), "got {line}");
    }

    /// Both lists must survive. Keeping only the base's would drop the
    /// strobe's exposed parameters; keeping only the extra's would disable
    /// the CRT shader's textures.
    #[test]
    fn texture_and_parameter_lists_are_merged() {
        let out = chained();
        assert!(out.contains("textures = \"Mask;Glow\""));
        let params = out
            .lines()
            .find(|l| l.starts_with("parameters ="))
            .expect("a parameters line");
        for name in ["brightness", "contrast", "_ADPT_STROBE_STR"] {
            assert!(params.contains(name), "{name} missing from {params}");
        }
    }

    /// Two-digit indices must not be read as pass 1 followed by a stray 0.
    #[test]
    fn indices_past_nine_are_parsed_whole() {
        let p = Preset::parse("shaders = 12\nshader11 = a.slang\nalias11 = Last\n", Path::new("/r"));
        assert_eq!(p.passes, 12);
        let out = chain(&p, &Preset::parse("shaders = 1\nshader0 = s.slang\n", Path::new("/r")));
        assert!(out.contains("alias11 = \"Last\""));
        assert!(out.contains("shader12 = \"/r/s.slang\""), "the extra lands at 12, not 2");
    }

    /// Textures listed in `textures = ...` carry relative paths of their own.
    /// crt-guest-advanced ships four color LUTs that way, and leaving them
    /// relative means RetroArch cannot load them once the generated preset
    /// lives in a different directory — a silently wrong picture, not an error.
    #[test]
    fn texture_paths_are_made_absolute_too() {
        let base = Preset::parse(
            "shaders = 1\nshader0 = a.slang\ntextures = \"LUT1\"\n\
             LUT1 = shaders/lut/trinitron.png\nLUT1_linear = true\n",
            Path::new("/ra/crt"),
        );
        let out = chain(&base, &Preset::parse("shaders = 0", Path::new("/ra/bfi")));
        assert!(
            out.contains("LUT1 = \"/ra/crt/shaders/lut/trinitron.png\""),
            "the LUT path must be absolute: {out}"
        );
        assert!(out.contains("LUT1_linear = \"true\""), "flags are not paths");
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let p = Preset::parse("# a comment\n\nshaders = 1\nshader0 = x.slang\n", Path::new("/r"));
        assert_eq!(p.passes, 1);
    }
}

//! Per-platform video shaders.
//!
//! ES-DE leaves this to RetroArch; we set it per platform at launch instead,
//! so a CRT console gets a CRT mask and a handheld gets its LCD grid without
//! anyone opening RetroArch's menu.
//!
//! Presets are `.slangp` because this install runs the Vulkan driver. A GL
//! build would want `.glslp`; [`available`] checks the file exists before
//! offering it, so a missing format degrades to "no shader" rather than a
//! black screen.

use std::path::{Path, PathBuf};

use crate::retroarch::RetroArch;
use crate::slangp;

/// What kind of display the system had — decides which presets make sense.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// Plugged into a television. Wants scanlines, mask, curvature.
    Crt,
    /// Its own LCD panel. Wants a pixel grid, not scanlines.
    Handheld,
}

pub struct ShaderOption {
    /// Path under `shaders_slang/`, without the `.slangp` suffix.
    pub path: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub display: Display,
}

/// Curated presets, heaviest-first within each group.
///
/// Deliberately short: RetroArch ships 1,932 presets, and a list that long is
/// a worse experience than a considered dozen.
pub const CATALOGUE: &[ShaderOption] = &[
    // --- CRT ---
    ShaderOption { path: "crt/crt-guest-advanced", label: "CRT — Guest Advanced", note: "Best looking — 12 passes, heaviest by far", display: Display::Crt },
    ShaderOption { path: "crt/crt-guest-advanced-fast", label: "CRT — Guest Advanced (fast)", note: "The same look, 9 passes", display: Display::Crt },
    ShaderOption { path: "crt/crt-royale", label: "CRT — Royale", note: "Classic reference, heavy", display: Display::Crt },
    ShaderOption { path: "crt/crt-geom", label: "CRT — Geom", note: "Curvature, moderate cost", display: Display::Crt },
    ShaderOption { path: "crt/crt-hyllian", label: "CRT — Hyllian", note: "Sharp, good for 2D", display: Display::Crt },
    ShaderOption { path: "crt/crt-aperture", label: "CRT — Aperture", note: "Aperture grille, light", display: Display::Crt },
    ShaderOption { path: "crt/crt-lottes", label: "CRT — Lottes", note: "Warm, moderate", display: Display::Crt },
    ShaderOption { path: "crt/crt-easymode", label: "CRT — EasyMode", note: "Light and clean", display: Display::Crt },
    ShaderOption { path: "crt/zfast-crt", label: "CRT — zfast", note: "Cheapest scanlines", display: Display::Crt },
    // --- Handheld ---
    ShaderOption { path: "handheld/gameboy", label: "Game Boy DMG", note: "Green dot matrix", display: Display::Handheld },
    ShaderOption { path: "handheld/sameboy-lcd", label: "SameBoy LCD", note: "Accurate DMG panel", display: Display::Handheld },
    ShaderOption { path: "handheld/gameboy-pocket", label: "Game Boy Pocket", note: "Gray panel", display: Display::Handheld },
    ShaderOption { path: "handheld/gameboy-color-dot-matrix", label: "GBC dot matrix", note: "Color dot matrix", display: Display::Handheld },
    ShaderOption { path: "handheld/agb001", label: "GBA (AGB-001)", note: "Original, unlit — very dark", display: Display::Handheld },
    ShaderOption { path: "handheld/ags001", label: "GBA SP (AGS-001)", note: "Front-lit SP — the default", display: Display::Handheld },
    ShaderOption { path: "presets/handheld-plus-color-mod/lcd-grid-v2-sp101-color", label: "GBA SP backlit (AGS-101)", note: "Brightest — the backlit SP revision", display: Display::Handheld },
    ShaderOption { path: "handheld/gameboy-advance-dot-matrix", label: "GBA dot matrix", note: "Visible pixel grid", display: Display::Handheld },
    ShaderOption { path: "handheld/lcd1x_nds", label: "DS LCD", note: "Tuned for DS", display: Display::Handheld },
    ShaderOption { path: "handheld/lcd1x_psp", label: "PSP LCD", note: "Tuned for PSP", display: Display::Handheld },
    ShaderOption { path: "handheld/ds-hybrid-sabr", label: "DS hybrid (SABR)", note: "Smoothed DS upscale", display: Display::Handheld },
    ShaderOption { path: "handheld/lcd-grid-v2", label: "LCD grid v2", note: "Accurate generic grid", display: Display::Handheld },
    ShaderOption { path: "handheld/lcd1x", label: "LCD 1x", note: "Simple generic grid", display: Display::Handheld },
    ShaderOption { path: "handheld/zfast-lcd", label: "LCD — zfast", note: "Cheapest grid", display: Display::Handheld },
    ShaderOption { path: "handheld/dot", label: "Dot", note: "Plain dot matrix", display: Display::Handheld },
];

/// Which display a platform had. Everything not listed is treated as a TV
/// console, which is the safe default: a CRT mask on an unknown system looks
/// wrong, an LCD grid looks broken.
pub fn display_of(platform: &str) -> Display {
    match platform {
        "gb" | "gbc" | "gba" | "gamegear" | "wonderswan" | "wonderswancolor"
        | "neo-geo-pocket" | "nds" | "psp" => Display::Handheld,
        _ => Display::Crt,
    }
}

/// Shipped default per platform.
///
/// Handhelds get their own panel where a specific preset exists, since a
/// Game Boy grid on a PSP looks nothing like the real thing.
pub fn default_for(platform: &str) -> Option<&'static str> {
    Some(match platform {
        "gb" => "handheld/gameboy",
        "gbc" => "handheld/gameboy-color-dot-matrix",
        // ags001, not agb001: the AGB-001 is the *original* Game Boy Advance,
        // whose screen had no lighting at all, so an accurate recreation of it
        // is genuinely very dark — that is the panel, not a bug in the shader.
        // The AGS-001 is the front-lit SP, and its preset adds a second
        // lighting pass. Same family, same look, actually visible indoors.
        "gba" => "handheld/ags001",
        "nds" => "handheld/lcd1x_nds",
        "psp" => "handheld/lcd1x_psp",
        "gamegear" | "wonderswan" | "wonderswancolor" | "neo-geo-pocket" => "handheld/lcd1x",
        // Every television console. Which one depends on what is drawing it.
        //
        // `crt-guest-advanced` is twelve passes with large intermediate
        // buffers, and it is the right default on a desktop GPU. On a handheld
        // it is the wrong shape of expensive: the same frame, several times
        // over, on a chip also running the emulator.
        //
        // `crt-lottes` is one pass and was written to get the same idea — a
        // phosphor mask, scanline bloom, optional curvature — out of a single
        // draw. It is the closest thing to the look at a fraction of the cost,
        // which is exactly the trade a handheld wants.
        //
        // A default, not a rule: Settings -> Emulators sets the shader per
        // system and that choice wins over this everywhere.
        _ if cfg!(target_os = "android") => "crt/crt-lottes",
        _ => "crt/crt-guest-advanced",
    })
}

/// Strobe/BFI passes, which chain on top of whatever base shader is selected.
///
/// These are *shader* BFI: they use RetroArch's `video_shader_subframes` and
/// modulate brightness across subframes, rather than the frontend's
/// `video_black_frame_insertion`, which hard-requires the display refresh to be
/// an exact integer multiple of the content frame rate. That constraint is what
/// rules BFI out on a 144Hz monitor showing 60fps content (144/60 = 2.4).
///
/// Ordered best-first. The adaptive one compensates its own gain, so it costs
/// far less brightness than plain black-frame insertion — which matters on a
/// CRT preset that is already dark.
pub const MOTION: &[ShaderOption] = &[
    ShaderOption { path: "subframe-bfi/adaptive_strobe-koko", label: "Adaptive strobe", note: "Gain-compensated, keeps brightness", display: Display::Crt },
    ShaderOption { path: "subframe-bfi/120hz-smart-BFI", label: "Smart BFI (120Hz)", note: "Per-area cadence, needs 120Hz", display: Display::Crt },
    ShaderOption { path: "subframe-bfi/120hz-safe-BFI", label: "Safe BFI (120Hz)", note: "Flips cadence to spare the panel", display: Display::Crt },
    ShaderOption { path: "subframe-bfi/bfi-simple", label: "Simple BFI", note: "Plain black subframe, halves brightness", display: Display::Crt },
    ShaderOption { path: "subframe-bfi/crt-beam-simulator", label: "CRT beam simulator", note: "Rolling scan, wants 240Hz+", display: Display::Crt },
];

/// Subframes to render per content frame, from the display's refresh rate.
///
/// `video_shader_subframes` divides one content frame into N draws, so the
/// display has to be able to show them: at 120Hz a 60fps game gets 2, at 240Hz
/// it gets 4. Unlike frontend BFI a non-integer ratio is not fatal here — the
/// strobe cadence just beats slightly against the refresh — so 144Hz rounds
/// down to 2 rather than being refused.
pub fn subframes_for(refresh_hz: Option<f32>) -> u32 {
    const CONTENT_FPS: f32 = 60.0;
    match refresh_hz {
        Some(hz) if hz >= CONTENT_FPS * 2.0 => ((hz / CONTENT_FPS).floor() as u32).min(8),
        // Unknown refresh: 2 is right for every 120Hz+ panel and harmless on a
        // 60Hz one, where RetroArch simply has no spare subframe to draw.
        None => 2,
        Some(_) => 1,
    }
}

fn shader_root(ra: &RetroArch) -> PathBuf {
    ra.shaders_dir().join("shaders_slang")
}

/// Absolute path of a preset, if this install actually has it.
pub fn resolve(ra: &RetroArch, preset: &str) -> Option<PathBuf> {
    if preset.is_empty() || preset == "none" {
        return None;
    }
    let p = shader_root(ra).join(format!("{preset}.slangp"));
    p.is_file().then_some(p)
}

/// Catalog entries this install can actually load, for the given display.
pub fn available(ra: &RetroArch, display: Display) -> Vec<&'static ShaderOption> {
    CATALOGUE
        .iter()
        .filter(|o| o.display == display)
        .filter(|o| resolve(ra, o.path).is_some())
        .collect()
}

/// RetroArch config lines enabling `preset`, or disabling shaders entirely.
///
/// Written into the launch overrides rather than the user's config, so this
/// only ever affects games launched from here.
pub fn config_lines(ra: &RetroArch, preset: Option<&str>) -> String {
    match preset.and_then(|p| resolve(ra, p)) {
        Some(path) => format!(
            "\n# Per-platform shader chosen in this app.\n\
             video_shader_enable = \"true\"\nvideo_shader = \"{}\"\n",
            path.display()
        ),
        // Explicitly off: without this a shader set for the previous game
        // would persist into one that should have none.
        None => "\nvideo_shader_enable = \"false\"\n".to_owned(),
    }
}

/// Write a preset combining `base` with a motion pass, and return its path.
///
/// RetroArch loads one preset at a time, so a strobe layer on top of a CRT
/// shader has to be a generated file. It lands in `dir` (our own config
/// directory) rather than inside the RetroArch install, which is only possible
/// because [`crate::slangp`] rewrites every shader path to an absolute one.
pub fn write_chained(
    ra: &RetroArch,
    dir: &Path,
    base: Option<&str>,
    motion: &str,
) -> Option<PathBuf> {
    let motion_path = resolve(ra, motion)?;
    let extra = slangp::Preset::load(&motion_path).ok()?;

    let body = match base.and_then(|b| resolve(ra, b)) {
        Some(base_path) => {
            let base = slangp::Preset::load(&base_path).ok()?;
            slangp::chain(&base, &extra)
        }
        // No base shader: the motion pass is the whole chain.
        None => slangp::chain(&slangp::Preset::parse("shaders = 0", dir), &extra),
    };

    let out = dir.join("moose-motion.slangp");
    std::fs::write(&out, body).ok()?;
    // Absolute, always. The library folder is configured as "./library" and
    // writing that through to `video_shader` gave RetroArch a path it resolves
    // against somewhere of its own — so the preset was silently not found and
    // every CRT platform fell back to no shader at all. Handhelds were
    // unaffected, because the motion pass is CRT-only, which is exactly the
    // pattern the report described.
    Some(out.canonicalize().unwrap_or(out))
}

/// Preset for a platform: the user's choice if set, else the shipped default.
pub fn preset_for(
    overrides: &std::collections::BTreeMap<String, String>,
    platform: &str,
) -> Option<String> {
    if let Some(chosen) = overrides.get(platform) {
        return (chosen != "none" && !chosen.is_empty()).then(|| chosen.clone());
    }
    default_for(platform).map(str::to_owned)
}

/// Label for a preset path, for display.
pub fn label_of(preset: &str) -> &str {
    CATALOGUE
        .iter()
        .find(|o| o.path == preset)
        .map(|o| o.label)
        .unwrap_or(preset)
}

/// Every platform slug we ship a default for, paired with its display kind.
pub fn describe(platform: &str) -> (Display, Option<&'static str>) {
    (display_of(platform), default_for(platform))
}

/// URL of the slang shader pack, the same archive RetroArch's own
/// "Update Slang Shaders" fetches.
const SHADER_PACK: &str = "https://buildbot.libretro.com/assets/frontend/shaders_slang.zip";

/// Download the slang shader pack if this install has none.
///
/// Presets are useless without it: `resolve` returns None for every entry, so
/// shaders silently do nothing. The pack is a few megabytes and covers every
/// preset in the catalog, which is far simpler than fetching them one at a
/// time — and RetroArch itself ships no shaders in the base download.
///
/// Returns true when it downloaded something.
pub async fn ensure_pack(client: &reqwest::Client, ra: &RetroArch) -> anyhow::Result<bool> {
    use anyhow::{Context, bail};

    if shader_root(ra).is_dir() {
        return Ok(false);
    }
    let dest = ra.shaders_dir();
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;

    let resp = client.get(SHADER_PACK).send().await
        .with_context(|| format!("requesting {SHADER_PACK}"))?;
    if !resp.status().is_success() {
        bail!("{SHADER_PACK} -> HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await.context("reading the shader pack")?;

    // The archive already contains a `shaders_slang/` directory, so extracting
    // at shaders_dir() lands it exactly where shader_root expects.
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context("opening the shader pack")?;
    zip.extract(&dest).context("extracting the shader pack")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path written into `video_shader` has to be absolute.
    ///
    /// It was not, and the consequence was every CRT platform silently losing
    /// its shader: the library folder is configured as "./library", that went
    /// through as `./library/moose-motion.slangp`, and RetroArch resolved it
    /// against its own shaders directory —
    ///
    ///   <RetroArch>/shaders/./library/moose-motion.slangp
    ///
    /// — found nothing, and fell back to no shader at all. Nothing failed and
    /// nothing was logged at the app's end. Only the chained path took this
    /// route, and the motion pass is CRT-only, which is why the handhelds went
    /// on looking correct throughout.
    #[test]
    fn the_chained_preset_is_written_as_an_absolute_path() {
        // Relative on purpose, and relative to the working directory the test
        // runs in — which is what `library.local_root = "./library"` gives the
        // launcher in real use. A path under the system temp directory is
        // already absolute and would make this pass without proving anything.
        let dir = std::path::PathBuf::from("target/moose-rack-shader-chain-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shaders/shaders_slang/subframe-bfi")).unwrap();
        std::fs::write(
            dir.join("shaders/shaders_slang/subframe-bfi/plain.slangp"),
            "shaders = 1\nshader0 = \"a.slang\"\n",
        )
        .unwrap();
        assert!(dir.is_relative(), "the test itself has to hand over a relative path");

        let ra = crate::retroarch::RetroArch {
            root: dir.clone(),
            binary: dir.join("retroarch"),
            portable: false,
            system_override: None,
        };

        let out = write_chained(&ra, &dir, None, "subframe-bfi/plain")
            .expect("a chain should be written");
        assert!(
            out.is_absolute(),
            "video_shader would be written as {} — RetroArch resolves a relative \
             preset against its own shaders directory, not ours, so the shader \
             silently does not load",
            out.display()
        );
        assert!(out.is_file(), "{} was not actually written", out.display());
        let _ = std::fs::remove_dir_all(&dir);
    }


    /// The GBA default must be the *lit* panel. AGB-001 is the original Game
    /// Boy Advance, which had no screen lighting at all, so a faithful shader
    /// for it is close to unreadable on a desktop monitor. AGS-001 is the
    /// front-lit SP and its preset carries a second lighting pass.
    #[test]
    fn the_gba_default_is_a_lit_panel() {
        assert_eq!(default_for("gba"), Some("handheld/ags001"));
    }

    /// Every shipped default has to exist in the catalog, or the Settings
    /// list shows a shader the app itself chose but cannot name.
    #[test]
    fn every_default_is_in_the_catalog() {
        for platform in [
            "gb", "gbc", "gba", "nds", "psp", "gamegear", "wonderswan",
            "wonderswancolor", "neo-geo-pocket", "snes", "genesis", "arcade",
        ] {
            let preset = default_for(platform).expect("every platform has a default");
            assert!(
                CATALOGUE.iter().any(|o| o.path == preset),
                "{platform} defaults to {preset}, which is not in the catalog"
            );
        }
    }

    /// A default must match the display it is for: a CRT mask on a handheld
    /// looks wrong, and an LCD grid on a TV console looks broken.
    #[test]
    fn defaults_match_the_display_they_are_for() {
        for platform in ["gb", "gbc", "gba", "nds", "psp", "snes", "genesis", "n64"] {
            let preset = default_for(platform).unwrap();
            let entry = CATALOGUE.iter().find(|o| o.path == preset).unwrap();
            assert_eq!(
                entry.display,
                display_of(platform),
                "{platform} defaults to {preset}, which is for the other display kind"
            );
        }
    }

    /// An explicit choice wins over the shipped default, and "none" means none
    /// rather than falling back.
    #[test]
    fn an_override_beats_the_default() {
        let mut over = std::collections::BTreeMap::new();
        over.insert("gba".to_owned(), "handheld/lcd1x".to_owned());
        assert_eq!(preset_for(&over, "gba").as_deref(), Some("handheld/lcd1x"));

        over.insert("gba".to_owned(), "none".to_owned());
        assert_eq!(preset_for(&over, "gba"), None, "\"none\" must not fall back");
    }

    /// Selecting nothing has to emit an explicit disable: without it a shader
    /// set for the previous game persists into one that should have none.
    #[test]
    fn no_shader_disables_rather_than_omitting() {
        let dir = std::env::temp_dir().join("moose-rack-test-shaders");
        let ra = RetroArch {
            root: dir.clone(),
            binary: dir.join("retroarch"),
            portable: false,
            system_override: None,
        };
        assert!(config_lines(&ra, None).contains("video_shader_enable = \"false\""));
        // A preset this install does not have is the same as none, not a
        // reference to a missing file that RetroArch would fail to load.
        assert!(
            config_lines(&ra, Some("crt/crt-guest-advanced"))
                .contains("video_shader_enable = \"false\"")
        );
    }
}

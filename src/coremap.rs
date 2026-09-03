//! Reads `data/esde-core-map.json` — the platform → libretro core mapping
//! extracted from ES-DE (see `tools/extract_esde_cores.py`).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CoreMap {
    /// RomM platform slug -> default core stem, e.g. `"snes" -> "snes9x"`.
    pub default_core_by_server_platform: BTreeMap<String, String>,
    pub systems: BTreeMap<String, System>,
    /// ES-DE's app id -> the Android `package/Activity` pairs that run it.
    ///
    /// `"AETHERSX2-TURNIP" -> ["xyz.aethersx2.custom/xyz.aethersx2.android.EmulationActivity"]`.
    /// This is ES-DE's own table, taken from the same device export as the rest
    /// of this file, and it is how a standalone emulator is started on Android:
    /// there is no command line, only an Intent naming a component.
    ///
    /// More than one entry because a package can be forked — the list is in
    /// preference order and the first one installed wins.
    #[serde(default)]
    pub android_packages: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct System {
    /// What to call it on screen — "Sony PlayStation", not "psx".
    ///
    /// The table has carried this all along and nothing read it. A library
    /// scanned from a card knows only the directory name, so every console on
    /// the handheld was labelled `psx`, `gbc`, `dc`.
    #[serde(default)]
    pub fullname: Option<String>,
    pub server_platforms: Vec<String>,
    /// The file extensions ES-DE accepts for this system, `.sfc` and friends.
    ///
    /// Read only when walking into a subfolder. A top-level directory holds
    /// games and little else, but `arcade/fbneo` is battery files and
    /// `famicom/FCEUmm` is save states, and without this the recursion listed
    /// those as games.
    #[serde(default)]
    pub extensions: Vec<String>,
    pub emulators: Vec<Emulator>,
}

#[derive(Debug, Deserialize)]
pub struct Emulator {
    pub label: String,
    pub kind: String,
    /// Present only when `kind == "libretro"`.
    #[serde(default)]
    pub core: Option<String>,
    /// The core stem Android uses, when it differs from the desktop one.
    #[serde(default)]
    pub core_android: Option<String>,
    /// Per-platform file names for the core. Android's is the one that matters
    /// here: `opera_libretro_android.so`, not `opera_libretro.so`.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// ES-DE's app id for a standalone emulator, looked up in
    /// [`CoreMap::android_packages`]. Present only when `kind == "standalone"`.
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub default: bool,
}

/// How to start something on Android: a component, and what to hand it.
///
/// Two shapes, because Android has two. A libretro core runs inside RetroArch,
/// which takes the ROM, the full path to the core and a config file as Intent
/// extras. A standalone emulator is its own app and takes the ROM as the
/// Intent's data.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AndroidLaunch {
    /// `package/Activity`, ready for an explicit Intent.
    pub component: String,
    /// The core's file name, for RetroArch. `None` for a standalone emulator.
    pub core_file: Option<String>,
    pub label: String,
    /// True when this runs inside RetroArch rather than being its own app.
    pub libretro: bool,
}

/// Pick the core for a platform: an explicit config override, else the ES-DE
/// default, else any installed alternative.
///
/// `has_core` lets callers supply their own installed-check without this
/// module depending on the RetroArch locator.
pub fn resolve_core(
    map: &CoreMap,
    overrides: &std::collections::BTreeMap<String, String>,
    platform: &str,
    has_core: impl Fn(&str) -> bool,
) -> Option<String> {
    resolve_core_for(map, overrides, &Default::default(), platform, None, has_core)
}

/// As [`resolve_core`], but a per-game override wins over the platform one.
///
/// Arcade is why this exists: no single core runs a mixed romset, so the
/// platform default is a best guess and individual games need to escape it.
pub fn resolve_core_for(
    map: &CoreMap,
    overrides: &std::collections::BTreeMap<String, String>,
    per_game: &std::collections::BTreeMap<String, String>,
    platform: &str,
    fs_name: Option<&str>,
    has_core: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(fs_name) = fs_name
        && let Some(core) = per_game.get(&crate::config::game_key(platform, fs_name))
    {
        return Some(core.clone());
    }
    // An override is a deliberate choice; honour it even if not installed, so
    // the failure names the core the user asked for.
    if let Some(core) = overrides.get(platform) {
        return Some(core.clone());
    }
    if let Some(default) = map.default_core(platform)
        && has_core(default)
    {
        return Some(default.to_owned());
    }
    map.alternatives(platform)
        .into_iter()
        .find(|c| has_core(c))
        .map(str::to_owned)
}

/// The shipped core map, compiled into the binary.
///
/// A file next to the executable is not something a downloaded build can rely
/// on: the resource-copying code only ever knew the macOS `.app` layout, so a
/// loose Windows exe found nothing, failed to load the map, and — with no
/// console attached in a release build — exited without a word. 110 KB in the
/// binary buys a program that always starts.
pub const EMBEDDED: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/esde-core-map.json"));

impl CoreMap {
    /// The compiled-in map. Infallible by construction: it is validated at
    /// build time by the test below, so a malformed file fails CI, not a user.
    /// What a console directory is called on screen.
    ///
    /// Looked up by the *system* name — the directory on the card — and then by
    /// any RomM slug that maps to it, because a scan files games under the slug
    /// and the table is keyed by the system.
    pub fn display_name(&self, slug: &str) -> Option<&str> {
        if let Some(name) = self.systems.get(slug).and_then(|s| s.fullname.as_deref()) {
            return Some(name);
        }
        self.systems
            .values()
            .find(|sys| sys.server_platforms.iter().any(|p| p == slug))
            .and_then(|sys| sys.fullname.as_deref())
    }

    pub fn embedded() -> Self {
        serde_json::from_str(EMBEDDED).expect("the embedded core map is valid JSON")
    }

    /// Load from disk, falling back to the embedded copy.
    ///
    /// Disk wins so a user can edit the mapping without a rebuild; the
    /// fallback means a fresh install works before anything has been written.
    pub fn load_or_embedded(path: &Path) -> Self {
        Self::load(path).unwrap_or_else(|_| Self::embedded())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading core map at {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Default core stem for a RomM platform slug.
    pub fn default_core(&self, platform: &str) -> Option<&str> {
        self.default_core_by_server_platform
            .get(platform)
            .map(String::as_str)
    }

    /// Every libretro core ES-DE offers for a platform, in preference order.
    /// Used for "launch with a different core" later.
    pub fn alternatives(&self, platform: &str) -> Vec<&str> {
        let mut out = Vec::new();
        for system in self.systems.values() {
            if !system.server_platforms.iter().any(|p| p == platform) {
                continue;
            }
            for emu in &system.emulators {
                if emu.kind == "libretro"
                    && let Some(core) = emu.core.as_deref()
                    && !out.contains(&core)
                {
                    out.push(core);
                }
            }
        }
        out
    }

    /// Every RomM platform that lists `core` among its emulators.
    pub fn platforms_with_core(&self, core: &str) -> Vec<String> {
        let mut out = Vec::new();
        for system in self.systems.values() {
            let has = system
                .emulators
                .iter()
                .any(|e| e.core.as_deref() == Some(core));
            if has {
                for p in &system.server_platforms {
                    if !out.contains(p) {
                        out.push(p.clone());
                    }
                }
            }
        }
        out
    }

    /// Map an emulator's *display name* to a core stem.
    ///
    /// RetroArch names its save subdirectories after the core's display name,
    /// which differs from ES-DE's labels (`"MAME"` vs `"MAME - Current"`), so
    /// match on a normalized form of both the label and the stem itself.
    pub fn core_by_display_name(
        &self,
        display: &str,
        normalize: fn(&str) -> String,
    ) -> Option<String> {
        let want = normalize(display);
        if want.is_empty() {
            return None;
        }
        let mut fallback = None;
        for system in self.systems.values() {
            for emu in &system.emulators {
                let Some(core) = emu.core.as_deref() else {
                    continue;
                };
                if normalize(core) == want {
                    return Some(core.to_owned());
                }
                if normalize(&emu.label) == want {
                    fallback.get_or_insert_with(|| core.to_owned());
                }
            }
        }
        fallback
    }

    /// Human label for a core, for display.
    pub fn label_for(&self, core: &str) -> Option<&str> {
        self.systems
            .values()
            .flat_map(|s| &s.emulators)
            .find(|e| e.core.as_deref() == Some(core))
            .map(|e| e.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> CoreMap {
        serde_json::from_str(
            r#"{
              "default_core_by_server_platform": {"snes": "snes9x"},
              "systems": {"snes": {
                "server_platforms": ["snes"],
                "emulators": [
                  {"label": "Snes9x", "kind": "libretro", "core": "snes9x"},
                  {"label": "bsnes",  "kind": "libretro", "core": "bsnes"},
                  {"label": "Standalone", "kind": "standalone"}
                ]}}
            }"#,
        )
        .expect("test fixture matches the CoreMap schema")
    }

    /// A per-game pin beats the platform override, which beats the ES-DE
    /// default. This ordering is why arcade works at all: one platform, many
    /// romsets, no single core that runs them.
    /// The compiled-in map has to parse, since `embedded()` unwraps it. This
    /// is what makes that unwrap safe: a bad `data/esde-core-map.json` fails
    /// here rather than at a user's first launch.
    #[test]
    fn the_embedded_core_map_parses() {
        let m = CoreMap::embedded();
        assert!(!m.systems.is_empty(), "the embedded map has systems");
        assert!(
            m.default_core("snes").is_some(),
            "and a default for a platform that certainly has one"
        );
    }

    #[test]
    fn per_game_beats_override_beats_default() {
        let m = map();
        let none = BTreeMap::new();
        let over = BTreeMap::from([("snes".to_owned(), "bsnes".to_owned())]);
        let pinned = BTreeMap::from([(
            crate::config::game_key("snes", "Game.sfc"),
            "mesen".to_owned(),
        )]);

        assert_eq!(
            resolve_core_for(&m, &over, &pinned, "snes", Some("Game.sfc"), |_| true).as_deref(),
            Some("mesen"),
            "a per-game pin must win over everything"
        );
        assert_eq!(
            resolve_core_for(&m, &over, &none, "snes", Some("Game.sfc"), |_| true).as_deref(),
            Some("bsnes"),
            "with no pin, the platform override wins"
        );
        assert_eq!(
            resolve_core_for(&m, &none, &none, "snes", None, |_| true).as_deref(),
            Some("snes9x"),
            "with neither, the ES-DE default is used"
        );
    }

    /// A pin only applies to the game it names — a sibling on the same platform
    /// still resolves normally.
    #[test]
    fn a_pin_does_not_leak_to_other_games() {
        let m = map();
        let pinned = BTreeMap::from([(
            crate::config::game_key("snes", "Game.sfc"),
            "mesen".to_owned(),
        )]);
        assert_eq!(
            resolve_core_for(&m, &BTreeMap::new(), &pinned, "snes", Some("Other.sfc"), |_| true)
                .as_deref(),
            Some("snes9x")
        );
    }

    /// An override is honoured even when that core is absent, so the resulting
    /// error names the core the user asked for instead of quietly substituting.
    #[test]
    fn override_survives_a_missing_core() {
        let m = map();
        let over = BTreeMap::from([("snes".to_owned(), "bsnes".to_owned())]);
        assert_eq!(
            resolve_core_for(&m, &over, &BTreeMap::new(), "snes", None, |_| false).as_deref(),
            Some("bsnes")
        );
    }

    /// An uninstalled default falls through to an installed alternative rather
    /// than failing the launch.
    #[test]
    fn uninstalled_default_falls_back_to_an_alternative() {
        let m = map();
        let none = BTreeMap::new();
        assert_eq!(
            resolve_core_for(&m, &none, &none, "snes", None, |c| c == "bsnes").as_deref(),
            Some("bsnes")
        );
        assert_eq!(
            resolve_core_for(&m, &none, &none, "snes", None, |_| false),
            None,
            "with nothing installed there is nothing to resolve to"
        );
    }

    #[test]
    fn unknown_platform_resolves_to_nothing() {
        let m = map();
        let none = BTreeMap::new();
        assert_eq!(resolve_core_for(&m, &none, &none, "dreamcast", None, |_| true), None);
    }

    /// Standalone emulators are not cores and must never be handed to RetroArch.
    #[test]
    fn standalone_emulators_are_not_offered_as_cores() {
        assert_eq!(map().alternatives("snes"), vec!["snes9x", "bsnes"]);
    }
}

/// RetroArch's own component, in preference order.
///
/// Two names because a device may carry either build, and `aarch64` is the one
/// on every handheld worth the name.
pub const RETROARCH_COMPONENTS: &[&str] = &[
    "com.retroarch.aarch64/com.retroarch.browser.retroactivity.RetroActivityFuture",
    "com.retroarch/com.retroarch.browser.retroactivity.RetroActivityFuture",
];

impl CoreMap {
    /// Whether the table knows this platform at all.
    ///
    /// Distinct from "has nothing to run it with". An unmapped platform means
    /// the ES-DE export never had a system claiming it, which is a gap in the
    /// table; a mapped one with no libretro emulator means ES-DE really does
    /// run it in a standalone app. They need different things said about them,
    /// and [`Self::android_launches`] returns an empty list for both.
    pub fn knows_platform(&self, platform: &str) -> bool {
        self.systems
            .values()
            .any(|s| s.server_platforms.iter().any(|p| p == platform))
    }

    /// Everything that could run this platform on Android, best first.
    ///
    /// Mirrors how ES-DE decides: the system's own emulator list, in the order
    /// that file gives, with the one marked default first. Whether a component
    /// is actually installed is not decided here — the caller knows that, and
    /// this module deliberately does not reach for a package manager.
    ///
    /// A libretro entry becomes RetroArch plus a core file name; a standalone
    /// entry becomes whatever `android_packages` says runs it. An emulator with
    /// neither is skipped rather than guessed at.
    pub fn android_launches(&self, platform: &str) -> Vec<AndroidLaunch> {
        let Some(sys) = self
            .systems
            .values()
            .find(|s| s.server_platforms.iter().any(|p| p == platform))
        else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for e in &sys.emulators {
            if e.kind == "libretro" {
                // The Android file name if the table has one, otherwise built
                // from the stem — Android's cores are suffixed `_android.so`
                // and a desktop name will not resolve on the device.
                let file = e.files.get("android").cloned().or_else(|| {
                    let stem = e.core_android.as_deref().or(e.core.as_deref())?;
                    Some(format!("{stem}_libretro_android.so"))
                });
                let Some(core_file) = file else { continue };
                for component in RETROARCH_COMPONENTS {
                    out.push(AndroidLaunch {
                        component: (*component).to_owned(),
                        core_file: Some(core_file.clone()),
                        label: e.label.clone(),
                        libretro: true,
                    });
                }
            } else if let Some(app) = e.app.as_deref() {
                for component in self.android_packages.get(app).into_iter().flatten() {
                    out.push(AndroidLaunch {
                        component: component.clone(),
                        core_file: None,
                        label: e.label.clone(),
                        libretro: false,
                    });
                }
            }
        }
        // The default emulator first, keeping the file's order within each group.
        let default_label = sys.emulators.iter().find(|e| e.default).map(|e| e.label.clone());
        out.sort_by_key(|l| Some(&l.label) != default_label.as_ref());
        out
    }
}

#[cfg(test)]
mod android_tests {
    use super::*;

    fn map() -> CoreMap {
        CoreMap::load_or_embedded(std::path::Path::new("data/esde-core-map.json"))
    }

    /// RetroArch is offered for a platform with a libretro core, and with the
    /// Android core file rather than the desktop one.
    ///
    /// `opera_libretro.so` does not exist on a device — there it is
    /// `opera_libretro_android.so`, and RetroArch has wanted the full path to
    /// it rather than a bare name since the 2025-01-17 nightly.
    /// The emulator table is regenerated from ES-DE's `es_find_rules.xml`, and
    /// a bad regeneration is invisible: every entry still parses, the launch
    /// just silently finds nothing installed and falls back to RetroArch. That
    /// is exactly what the previous table did — eighteen emulators, not one of
    /// which matched what was actually on the device.
    #[test]
    fn every_android_component_is_a_package_and_an_activity() {
        let m = map();
        assert!(
            m.android_packages.len() > 100,
            "the table should carry ES-DE's full list, not the old eighteen: {}",
            m.android_packages.len()
        );
        for (emulator, components) in &m.android_packages {
            assert!(!components.is_empty(), "{emulator} has no component");
            for c in components {
                let (pkg, activity) =
                    c.split_once('/').unwrap_or_else(|| panic!("{emulator}: {c} is not package/Activity"));
                assert!(pkg.contains('.'), "{emulator}: {pkg} is not a package name");
                assert!(!activity.is_empty(), "{emulator}: {c} names no activity");
                assert!(!pkg.contains(' ') && !activity.contains(' '), "{emulator}: {c} has a space");
            }
        }
    }

    /// Every emulator in the table has to be declared in the Android manifest.
    ///
    /// These two are one fact kept in two files, and when they drift nothing
    /// says so. Since Android 11 a package the manifest does not declare cannot
    /// be seen at all — `getPackageInfo` throws for an app that is plainly
    /// installed — so an emulator in the table but not in `<queries>` is
    /// offered, reported as missing, and quietly skipped. That is the whole
    /// reason this app could only ever start RetroArch.
    ///
    /// Reads the manifest as text rather than parsing it: the assertion is
    /// about a name being present, and a parser here would be a second thing to
    /// keep right.
    #[test]
    fn every_emulator_in_the_table_is_visible_to_the_android_build() {
        let manifest = std::path::Path::new(
            "src-tauri/gen/android/app/src/main/AndroidManifest.xml",
        );
        let Ok(text) = std::fs::read_to_string(manifest) else {
            // A checkout without the generated Android project is not a failure;
            // `android init` has simply not been run.
            return;
        };
        let m = map();
        let mut undeclared: Vec<String> = Vec::new();
        for components in m.android_packages.values() {
            for c in components {
                let pkg = c.split('/').next().unwrap_or_default();
                if !text.contains(&format!("android:name=\"{pkg}\"")) {
                    undeclared.push(pkg.to_owned());
                }
            }
        }
        undeclared.sort();
        undeclared.dedup();
        assert!(
            undeclared.is_empty(),
            "these emulators are in the core map but invisible to Android — add them to \
             <queries> in AndroidManifest.xml: {undeclared:?}"
        );
    }

    /// The emulators actually installed on Frank's Thor, by the package name it
    /// reports. Each was missing from the old table, which is why standalone
    /// emulators could never be started.
    #[test]
    fn the_emulators_on_the_device_are_in_the_table() {
        let m = map();
        let all: Vec<&str> =
            m.android_packages.values().flatten().map(String::as_str).collect();
        for pkg in [
            "com.flycast.emulator",
            "com.github.stenzek.duckstation",
            "io.recompiled.redream",
            "me.magnum.melondualds",
            "org.azahar_emu.azahar",
            "info.cemu.cemu",
            "dev.eden.eden_emulator",
        ] {
            assert!(
                all.iter().any(|c| c.starts_with(&format!("{pkg}/"))),
                "{pkg} is installed on the device and the table does not know it"
            );
        }
    }

    /// "Nothing can run it" and "the table has never heard of it" are
    /// different answers, and only one of them is a gap worth filing. Both
    /// produce an empty `android_launches`, so the caller needs this to tell
    /// them apart before it writes a message about either.
    #[test]
    fn an_unmapped_platform_is_distinguishable_from_one_with_no_core() {
        let m = map();
        assert!(m.knows_platform("gb"), "gb is mapped and has a core");
        // Real slugs from Frank's library that the ES-DE export never claimed.
        for slug in ["pico8", "g-and-w", "wii", "easyrpg"] {
            assert!(!m.knows_platform(slug), "{slug} should be unmapped");
            assert!(m.android_launches(slug).is_empty(), "{slug}");
        }
    }

    #[test]
    fn a_libretro_platform_offers_retroarch_with_an_android_core() {
        let out = map().android_launches("3do");
        assert!(!out.is_empty(), "3do should have somewhere to run");
        let ra = out.iter().find(|l| l.libretro).expect("a libretro entry");
        assert!(ra.component.starts_with("com.retroarch"), "{}", ra.component);
        assert_eq!(ra.core_file.as_deref(), Some("opera_libretro_android.so"));
    }

    /// A standalone emulator becomes the component ES-DE uses for it, and
    /// carries no core.
    #[test]
    fn a_standalone_emulator_becomes_its_own_component() {
        let m = map();
        let found = m
            .systems
            .values()
            .flat_map(|s| s.server_platforms.clone())
            .flat_map(|p| m.android_launches(&p))
            .find(|l| !l.libretro);
        let Some(l) = found else {
            panic!("the export lists standalone emulators; none resolved")
        };
        assert!(l.core_file.is_none(), "a standalone app runs no core");
        assert!(l.component.contains('/'), "package/Activity expected: {}", l.component);
    }

    /// A platform nothing claims gets an empty list rather than a guess.
    #[test]
    fn an_unknown_platform_offers_nothing() {
        assert!(map().android_launches("not-a-platform").is_empty());
    }
}

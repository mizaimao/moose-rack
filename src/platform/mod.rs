// What differs between the devices this app runs on.
//
// The axis of difference is the *device*, not the front end. SDL-vs-webview is
// a rendering choice and answers none of these questions:
//
//   1. Where does the library live?
//   2. Where does RetroArch keep its own files?
//   3. Which system directories mean what, and which are not ours?
//   4. How do brightness and battery work?
//
// Before this module those were answered by `#[cfg(target_os = …)]` scattered
// across `src/`, which cannot express the targets that matter. KNULLI *is*
// `target_os = "linux"` while being nothing like desktop Linux, and Android
// falls through the same `not(any(macos, windows))` branch. Both compiled and
// both behaved wrong.
//
// Selection is by Cargo feature, with the host's `target_os` as the fallback
// when no scheme is named — so an ordinary build is unchanged and the two
// schemes `target_os` cannot reach are opt-in.

use std::path::{Path, PathBuf};

use crate::esde;

pub mod android;
pub mod knulli;
pub mod linux;
pub mod macos;
pub mod windows;

/// A screen backlight that can be read and set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brightness {
    /// Where the level is read and written, 0..=`max`.
    pub path: PathBuf,
    pub max: u32,
    /// A vendor CLI that already wraps the sysfs node, preferred when present
    /// because it also handles whatever else the device couples to backlight.
    pub helper: Option<&'static str>,
}

/// A wireless connection that can be read without spawning anything.
///
/// `/proc/net/wireless` is one line per interface and holds the link quality,
/// so a status bar can read it every couple of seconds for the cost of opening
/// a file. The SSID needs `iw`, which is a process — that belongs on a settings
/// screen, not in the corner of every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wifi {
    pub proc_wireless: PathBuf,
    pub interface: &'static str,
    /// The largest value the quality column reports, for turning it into bars.
    pub max_quality: u32,
    /// Lists and joins networks. A process, so it is only for settings.
    ///
    /// Called as `<helper> scanlist` to look, and `<helper> enable <ssid>
    /// <key>` to join — which saves the network and waits for an address, so it
    /// can take several seconds and must not be run from the draw loop.
    pub helper: Option<&'static str>,
    /// Reads one saved value, as `<settings> <key>`. `wifi.ssid` is the network
    /// the device is set to join.
    pub settings_get: Option<&'static str>,
}

/// A battery that can be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    /// Reads 0..=100.
    pub capacity: PathBuf,
    /// Each reads `1` when that supply is connected.
    pub charging: Vec<PathBuf>,
    pub helper: Option<&'static str>,
}

/// One target's answers.
///
/// Everything has a default that means "this device has nothing to say about
/// it", so a new scheme starts as a handful of lines rather than a wall of
/// stubs.
/// Where a save sits under the RetroArch root, and what the folder name means.
///
/// The two shapes are not a preference; they are what the two kinds of device
/// actually do, and a scanner that assumes one finds nothing on the other.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SaveLayout {
    /// RetroArch's own, and every desktop and Android install:
    /// `saves/<core display name>/Game.srm`, states in a sibling `states/`.
    /// The folder is the *core*, spelled the way RetroArch spells it —
    /// `PCSX-ReARMed`, not `pcsx_rearmed`.
    ByCore,
    /// Batocera and KNULLI: `saves/<system>/Game.srm`, where the folder is the
    /// **platform** and carries no core at all — configgen points
    /// `savefile_directory` at it per launch. Save states live in that same
    /// folder rather than a sibling `states/`, so the folder cannot say which
    /// a file is; the filename has to.
    BySystem,
}

pub trait Platform: Sync {
    /// The scheme's name, for diagnostics and for tests that assert which one
    /// a build selected.
    fn scheme(&self) -> &'static str;

    /// Where the app's own files live — config, cache, core map.
    ///
    /// `None` means "work it out from the executable", which is
    /// [`crate::datadir::choose`] and is right everywhere the executable sits
    /// beside its data. Android is the exception: `current_exe()` there is the
    /// zygote, `/system/bin/app_process64`, and nowhere near the app.
    fn data_root(&self) -> Option<PathBuf> {
        None
    }

    /// The library layout, where the device fixes it.
    ///
    /// `None` means the user says where it is. A handheld running a known OS
    /// image does not need asking.
    fn default_library(&self) -> Option<esde::Layout> {
        None
    }

    /// How this device arranges its save tree.
    ///
    /// RetroArch's own layout is the default and needs no override.
    fn save_layout(&self) -> SaveLayout {
        SaveLayout::ByCore
    }

    /// The directory holding `saves/` — and, on `ByCore` devices, `states/`.
    ///
    /// `None` means "no better idea than the config's default", which is a
    /// relative path beside the app. That is right for a portable desktop
    /// install and wrong for a handheld, where the save tree is somewhere
    /// fixed that the OS decided.
    fn saves_root(&self) -> Option<&'static str> {
        None
    }

    /// Install roots to search for RetroArch, in order.
    fn retroarch_roots(&self) -> &'static [&'static str];

    /// Where RetroArch keeps `retroarch.cfg`, `config/`, `autoconfig/` and
    /// `shaders/` — which is *not* always beside the binary, and on Batocera
    /// is not the XDG location either.
    fn retroarch_data_dir(&self, install_root: &Path) -> PathBuf;

    /// Directories holding libretro cores, searched after RetroArch's own.
    fn core_dirs(&self) -> &'static [&'static str] {
        &[]
    }

    /// ES-DE system directory -> RomM slug, for names this device uses that
    /// the shipped core map does not know.
    fn system_aliases(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// RomM slug -> the folder this device actually keeps saves in.
    ///
    /// Not the inverse of [`Platform::system_aliases`], and cannot be: the
    /// mapping is many-to-one. RomM separates `snes` from `sfam`; KNULLI has
    /// one `snes` folder and no `sfam` at all, so a save filed under the
    /// server's own slug lands where no emulator looks — which is precisely
    /// what happened to a Super Famicom save on the first real sync.
    ///
    /// Anything not listed keeps its slug, which is right far more often than
    /// not.
    fn save_folder(&self, server_slug: &str) -> String {
        server_slug.to_string()
    }

    /// The inverse: every RomM platform whose saves land in one folder.
    ///
    /// Needed because [`Platform::save_folder`] is many-to-one, and a scanner
    /// that only looks up the folder's own name cannot find a game filed under
    /// any of the others. A Super Famicom save sits in `saves/snes/` on this
    /// handheld and is `sfam` on the server; searching only `snes` for it
    /// finds nothing, which reads as the save being unmatched rather than as
    /// the search being too narrow.
    fn platforms_in_folder(&self, folder: &str) -> Vec<String> {
        vec![folder.to_string()]
    }

    /// Cores this device should use in preference to the shipped default.
    ///
    /// RomM platform slug -> libretro core stem. The shipped core map is ES-DE's
    /// and assumes a desktop; a quad A55 at 1.8 GHz does not run what a desktop
    /// runs, and the difference is the game being playable or not.
    fn default_cores(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// The device front end's own system table, when it has one.
    ///
    /// KNULLI's `es_systems.cfg` is where Ports and Tools are actually
    /// defined — each is a `<group>` of systems, every member with its own
    /// folder and its own extensions. Reading it is the difference between
    /// showing what the device shows and showing a guess.
    fn es_systems(&self) -> Option<PathBuf> {
        None
    }

    /// System directories to pass over in silence on this device.
    ///
    /// Distinct from "unknown": an unknown directory is reported as skipped so
    /// a missing alias can be found, whereas these are known and deliberately
    /// not wanted, so reporting them is noise.
    fn ignored_systems(&self) -> &'static [&'static str] {
        &[]
    }

    fn brightness(&self) -> Option<Brightness> {
        None
    }

    fn battery(&self) -> Option<Battery> {
        None
    }

    fn wifi(&self) -> Option<Wifi> {
        None
    }

    /// Where the device keeps front-end themes.
    ///
    /// Worth knowing because they carry console artwork — a KNULLI theme has
    /// `_inc/systems/<look>/<slug>.webp` for every system it draws, which is
    /// the same shape as the icon sets this app downloads. A handheld that has
    /// never synced has no artwork of ours and a folder full of somebody's.
    fn theme_dirs(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// The scheme this build selected.
///
/// A feature wins if one is set; otherwise the host's `target_os` decides, so
/// a plain `cargo build` behaves exactly as it did before this module existed.
pub fn current() -> &'static dyn Platform {
    #[cfg(feature = "knulli")]
    {
        &knulli::Knulli
    }
    #[cfg(all(feature = "android", not(feature = "knulli")))]
    {
        &android::Android
    }
    #[cfg(all(feature = "macos", not(any(feature = "knulli", feature = "android"))))]
    {
        &macos::MacOs
    }
    #[cfg(all(
        feature = "windows",
        not(any(feature = "knulli", feature = "android", feature = "macos"))
    ))]
    {
        &windows::Windows
    }
    #[cfg(all(
        feature = "linux",
        not(any(
            feature = "knulli",
            feature = "android",
            feature = "macos",
            feature = "windows"
        ))
    ))]
    {
        &linux::Linux
    }
    // No scheme named: fall back to the host, which is what every build did
    // before there were schemes.
    #[cfg(not(any(
        feature = "knulli",
        feature = "android",
        feature = "macos",
        feature = "windows",
        feature = "linux"
    )))]
    {
        #[cfg(target_os = "macos")]
        {
            &macos::MacOs
        }
        #[cfg(target_os = "windows")]
        {
            &windows::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            &linux::Linux
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every scheme has to answer the questions the app actually asks. A new
    /// one that returns an empty root list, or points its RetroArch data
    /// directory at the install root by accident, fails here rather than on
    /// the device.
    #[test]
    fn every_scheme_answers_the_basics() {
        let schemes: [&dyn Platform; 5] = [
            &macos::MacOs,
            &windows::Windows,
            &linux::Linux,
            &knulli::Knulli,
            &android::Android,
        ];
        for p in schemes {
            assert!(
                !p.scheme().is_empty(),
                "a scheme must be able to name itself"
            );
            assert!(
                !p.retroarch_roots().is_empty(),
                "{}: nowhere to look for RetroArch",
                p.scheme()
            );
        }
    }

    /// The five names are distinct, so a build cannot silently select the
    /// wrong one and still look right in a log line.
    #[test]
    fn a_handheld_names_its_own_save_root() {
        // `./Saves` is the portable-RetroArch answer and is wrong on a device
        // where the OS decided where saves go. Getting this wrong is silent:
        // the scan finds an empty directory and the sync reports nothing to do.
        assert_eq!(super::knulli::Knulli.saves_root(), Some("/userdata"));
        assert_eq!(
            super::knulli::Knulli.save_layout(),
            SaveLayout::BySystem,
            "and it files by platform, not by core"
        );
    }

    #[test]
    fn every_other_scheme_leaves_the_save_root_to_the_config() {
        // Only a device with a fixed, OS-chosen tree should override it.
        let schemes: [&dyn Platform; 4] = [
            &macos::MacOs,
            &windows::Windows,
            &linux::Linux,
            &android::Android,
        ];
        for scheme in schemes {
            assert_eq!(
                scheme.saves_root(),
                None,
                "{} should not be hardcoding a save root",
                scheme.scheme()
            );
            assert_eq!(scheme.save_layout(), SaveLayout::ByCore);
        }
    }

    #[test]
    fn every_save_folder_this_device_maps_to_actually_exists() {
        // The list is /userdata/roms as it really is on the Flip. A mapping to
        // a folder with no system behind it is a save nothing will ever read —
        // which is what `sfam` was until a real sync put a Super Famicom save
        // in a directory KNULLI does not have.
        const ON_THE_DEVICE: &[&str] = &[
            "snes", "snes-msu1", "nes", "fds", "fbneo", "mame", "neogeo", "ngp", "ngpc",
            "megadrive", "gba", "gb", "gbc", "psx", "n64", "dreamcast", "gamegear",
            "wswan", "wswanc", "gamecube", "mastersystem", "pcengine", "psp", "saturn",
            "3do", "nds",
        ];
        let device = &knulli::Knulli;
        // Every one of these is a platform_fs_slug this server really reports.
        for slug in [
            "sfc",
            "famicom",
            "arcade",
            "neogeoaes",
            "dc",
            "neo-geo-pocket",
            "wonderswan",
            "wonderswancolor",
            "ngc",
        ] {
            let folder = device.save_folder(slug);
            assert!(
                ON_THE_DEVICE.contains(&folder.as_str()),
                "{slug} maps to {folder}, which is not a system on this device"
            );
        }
        // And a system it really has keeps its own name.
        assert_eq!(device.save_folder("snes-msu1"), "snes-msu1");
        assert_eq!(device.save_folder("gba"), "gba");
    }

    #[test]
    fn what_a_folder_collects_is_what_gets_searched_in_it() {
        // The two halves have to agree or a save is written somewhere the
        // reader will not look for it. `Dear Boys (Japan)` is `sfam` on the
        // server, was filed into `saves/snes/` correctly, and then came back
        // unmatched because only `snes` was searched.
        let device = &knulli::Knulli;
        for slug in [
            "sfc", "famicom", "arcade", "neogeoaes", "dc", "neo-geo-pocket",
            "wonderswan", "wonderswancolor", "ngc", "snes", "gba", "megadrive",
        ] {
            let folder = device.save_folder(slug);
            assert!(
                device.platforms_in_folder(&folder).iter().any(|p| p == slug),
                "{slug} is written to {folder} but is not searched for there"
            );
        }
    }

    #[test]
    fn other_devices_look_only_where_they_wrote() {
        assert_eq!(macos::MacOs.platforms_in_folder("snes"), vec!["snes".to_string()]);
    }

    #[test]
    fn other_devices_keep_the_servers_slug() {
        // Only a device that renames systems should rename them.
        assert_eq!(macos::MacOs.save_folder("sfc"), "sfc");
        assert_eq!(android::Android.save_folder("arcade"), "arcade");
    }

    #[test]
    fn scheme_names_are_distinct() {
        let names = [
            macos::MacOs.scheme(),
            windows::Windows.scheme(),
            linux::Linux.scheme(),
            knulli::Knulli.scheme(),
            android::Android.scheme(),
        ];
        let mut sorted = names;
        sorted.sort_unstable();
        let before = sorted.len();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(deduped.len(), before, "duplicate scheme name in {names:?}");
    }

    /// KNULLI is `target_os = "linux"` and must not inherit desktop Linux's
    /// answers. This is the reason the module exists, so it is asserted
    /// directly rather than left implied.
    #[test]
    fn knulli_does_not_share_desktop_linuxs_retroarch_directory() {
        let root = Path::new("/usr");
        assert_ne!(
            knulli::Knulli.retroarch_data_dir(root),
            linux::Linux.retroarch_data_dir(root),
            "KNULLI keeps RetroArch's files outside XDG; sharing the desktop \
             answer points the app at a directory that does not exist"
        );
    }

    /// Measured on the device 2026-08-24: `/userdata/system/.config/retroarch`
    /// does not exist, `/userdata/system/configs/retroarch/` does.
    #[test]
    fn knulli_points_at_batoceras_own_retroarch_directory() {
        assert_eq!(
            knulli::Knulli.retroarch_data_dir(Path::new("/usr")),
            PathBuf::from("/userdata/system/configs/retroarch"),
        );
    }
}

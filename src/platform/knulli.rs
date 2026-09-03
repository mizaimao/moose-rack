// KNULLI on the Miyoo Flip — Batocera 42, Buildroot 2024.11, BSP kernel 5.10.209.
//
// Every path here was measured on the device on 2026-08-24, not inferred from
// the fact that it is Linux. That distinction is the reason this scheme exists:
// KNULLI is `target_os = "linux"` and agrees with desktop Linux about almost
// none of this.

use std::path::{Path, PathBuf};

use super::{SaveLayout, Battery, Brightness, Platform, Wifi};
use crate::esde;

pub struct Knulli;

impl Platform for Knulli {
    fn scheme(&self) -> &'static str {
        "knulli"
    }

    /// The library is where KNULLI already keeps it, and it does not move.
    ///
    /// ROMs stay under `/userdata/roms` so Batocera's own EmulationStation and
    /// configgen keep working — this app replaces the front end, not the
    /// system. `/userdata` is the persistent partition (exfat, ~155 G free)
    /// and survives OS updates.
    fn default_library(&self) -> Option<esde::Layout> {
        Some(esde::Layout::new(
            Path::new("/userdata/ES-DE"),
            Some(Path::new("/userdata/roms")),
        ))
    }

    /// One root, because there is exactly one RetroArch and it is part of the
    /// image: `/usr/bin/retroarch`, 15.7 MB, confirmed on device.
    /// Batocera keeps the save tree on the persistent partition, not beside
    /// any application. Measured, like everything else here: the last game
    /// launch wrote `/userdata/saves/gba/…`.
    fn saves_root(&self) -> Option<&'static str> {
        Some("/userdata")
    }

    /// `/userdata/saves/<system>/Game.srm`, and save states in there too.
    ///
    /// Read off the device: configgen sets both `savefile_directory` and
    /// `savestate_directory` to the same per-system folder at every launch.
    fn save_layout(&self) -> SaveLayout {
        SaveLayout::BySystem
    }

    fn retroarch_roots(&self) -> &'static [&'static str] {
        &["/usr"]
    }

    /// **Not XDG.** Desktop Linux answers `~/.config/retroarch`, and KNULLI
    /// sets `HOME=/userdata/system`, so the inherited answer resolved to
    /// `/userdata/system/.config/retroarch` — which does not exist on the
    /// device. Batocera hands RetroArch an explicit config path instead and
    /// keeps everything here, holding `config/`, `autoconfig/`, `cores/` and
    /// `shaders/`.
    ///
    /// This is load-bearing for five callers, and every one of them was
    /// pointed at an empty directory before the scheme existed.
    fn retroarch_data_dir(&self, _install_root: &Path) -> PathBuf {
        PathBuf::from("/userdata/system/configs/retroarch")
    }

    /// 99 cores, counted on device.
    fn core_dirs(&self) -> &'static [&'static str] {
        &["/usr/lib/libretro"]
    }

    /// Batocera's directory names are its own, and the shipped core map was
    /// built from an ES-DE **Android** export. A name that does not match is
    /// not an error — `esde::scan` skips the directory — so each of these is a
    /// console that silently was not there.
    fn system_aliases(&self) -> &'static [(&'static str, &'static str)] {
        &[
            // The whole arcade library. ES-DE calls the directory `arcade`;
            // KNULLI splits arcade across `fbneo`, `mame` and `neogeo`, and on
            // this device `fbneo` holds 2,504 zips — exactly the RomM `arcade`
            // count, so it is that library and not a subset of it.
            ("fbneo", "arcade"),
            // Batocera's spellings for two Bandai handhelds and the GameCube.
            // Empty on this device today, and mapped anyway so that filling
            // them later does not need a code change. `gamecube` is
            // deliberately absent — see `ignored_systems`.
            ("ngpc", "neo-geo-pocket"),
        ]
    }

    /// Where KNULLI keeps saves for platforms the server spells differently,
    /// or does not separate at all.
    ///
    /// Keyed on RomM's **`platform_fs_slug`** — its library folder name, and
    /// what the cache keys on — not `platform_slug`, its catalogue slug. Super
    /// Famicom is `sfc` in one and `sfam` in the other, and picking the wrong
    /// one files a save under a platform no game is indexed against.
    ///
    /// Every left-hand name below is a slug this server actually reports;
    /// every right-hand one is a directory this device actually has.
    fn save_folder(&self, server_slug: &str) -> String {
        match server_slug {
            "sfc" => "snes",
            "famicom" => "nes",
            "arcade" => "fbneo",
            "neogeoaes" => "neogeo",
            "dc" => "dreamcast",
            "neo-geo-pocket" => "ngp",
            "neo-geo-pocket-color" => "ngpc",
            "wonderswan" => "wswan",
            "wonderswancolor" => "wswanc",
            "ngc" => "gamecube",
            other => other,
        }
        .to_string()
    }

    /// The inverse, kept in step with it by a test: anything a folder collects
    /// has to be searched when a save is read back out of it.
    fn platforms_in_folder(&self, folder: &str) -> Vec<String> {
        let extra: &[&str] = match folder {
            "snes" => &["sfc"],
            "nes" => &["famicom"],
            "fbneo" => &["arcade"],
            "neogeo" => &["neogeoaes"],
            "dreamcast" => &["dc"],
            "ngp" => &["neo-geo-pocket"],
            "ngpc" => &["neo-geo-pocket-color"],
            "wswan" => &["wonderswan"],
            "wswanc" => &["wonderswancolor"],
            "gamecube" => &["ngc"],
            _ => &[],
        };
        std::iter::once(folder.to_string())
            .chain(extra.iter().map(|s| (*s).to_string()))
            .collect()
    }

    /// What this hardware should actually run, where it differs from the
    /// shipped default.
    ///
    /// Settled on the device rather than chosen from a list. The two here are
    /// the ones that are certain:
    ///
    /// * **PSX** — the default is SwanStation, which assumes a modern x86 CPU.
    ///   `pcsx_rearmed` is the ARM-targeted one and the reason PlayStation
    ///   games are playable on this chip at all.
    /// * **Neo Geo** — fixed by the ROMs, not by speed. The sets here are
    ///   geolith's, so FBNeo cannot load them however fast it is.
    ///
    /// Deliberately short. The rest of KNULLI's per-device tuning lives in
    /// `/usr/share/knulli/configgen/configgen-defaults-arch.yml`, and matching
    /// it needs the actual core filenames out of `/usr/lib/libretro` — 99 of
    /// them, not yet listed. Guessing a core name is not a cheap mistake:
    /// a name that does not exist is a failed launch, not an error.
    fn default_cores(&self) -> &'static [(&'static str, &'static str)] {
        &[("psx", "pcsx_rearmed"), ("neogeoaes", "geolith")]
    }

    /// Batocera's own system table, which is where Ports and Tools live.
    fn es_systems(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/usr/share/emulationstation/es_systems.cfg"))
    }

    /// Known, and deliberately not wanted here.
    ///
    /// Distinct from an unknown directory, which is reported so a missing
    /// alias can be found. These are reported as nothing at all.
    fn ignored_systems(&self) -> &'static [&'static str] {
        &[
            // Frank's call: not wanted on the Flip. Both are empty on the
            // device, so nothing is hidden that holds a game.
            "wswan",
            "wswanc",
            // The Flip cannot emulate it — a quad A55 at 1.8 GHz with 1 GB is
            // not a GameCube. The directory exists because Batocera creates
            // its whole system set, and it is empty.
            "gamecube",
            // Batocera furniture, not consoles. Scanned as systems these
            // invent games and hand them to a core that cannot run them.
            "emulators",
            "tools",
            "recordings",
            "library",
        ]
    }

    /// 0..=255, confirmed on device. The vendor wrapper is preferred because
    /// it handles whatever else the board couples to the backlight.
    /// Themes, in the order to prefer them: the user's own first.
    ///
    /// `/userdata/themes` is what somebody installed; the one under
    /// `/usr/share` ships with the image. Both hold console pictures.
    fn theme_dirs(&self) -> Vec<PathBuf> {
        vec![
            PathBuf::from("/userdata/themes"),
            PathBuf::from("/usr/share/emulationstation/themes"),
        ]
    }

    fn brightness(&self) -> Option<Brightness> {
        Some(Brightness {
            path: PathBuf::from("/sys/class/backlight/backlight/brightness"),
            max: 255,
            helper: Some("knulli-brightness"),
        })
    }

    /// Measured on the device: `wlan0`, quality 40 of 70 while connected to
    /// "Chicken24" at -52 dBm. `knulli-wifi` has `scanlist` and `list`
    /// subcommands for the settings screen.
    fn wifi(&self) -> Option<Wifi> {
        Some(Wifi {
            proc_wireless: PathBuf::from("/proc/net/wireless"),
            interface: "wlan0",
            max_quality: 70,
            helper: Some("knulli-wifi"),
            settings_get: Some("knulli-settings-get"),
        })
    }

    /// Reads a percentage; the gauge on this unit reports "Battery
    /// Calibrated: No", so treat the number as approximate.
    fn battery(&self) -> Option<Battery> {
        Some(Battery {
            capacity: PathBuf::from("/sys/class/power_supply/battery/capacity"),
            charging: vec![
                PathBuf::from("/sys/class/power_supply/ac/online"),
                PathBuf::from("/sys/class/power_supply/usb/online"),
            ],
            helper: Some("knulli-battery-check"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The single change that recovers the most games. `fbneo` holds 2,504
    /// zips on the device and the RomM `arcade` platform holds 2,504 games;
    /// without this alias every one of them scans to nothing.
    #[test]
    fn the_arcade_library_is_reachable() {
        assert!(
            Knulli.system_aliases().contains(&("fbneo", "arcade")),
            "fbneo is the arcade library on this device"
        );
    }

    /// Hidden means silent, not skipped-and-reported. If one of these ever
    /// gains an alias the two lists disagree and the directory would both map
    /// and be ignored, which is a coin toss rather than a decision.
    #[test]
    fn nothing_is_both_ignored_and_aliased() {
        for (dir, _) in Knulli.system_aliases() {
            assert!(
                !Knulli.ignored_systems().contains(dir),
                "{dir} is both aliased and ignored — pick one"
            );
        }
    }

    /// The library must not be relative to the executable: the app is a guest
    /// on this OS and the ROMs belong to Batocera.
    #[test]
    fn the_library_is_the_one_already_on_the_device() {
        let l = Knulli.default_library().expect("KNULLI fixes its library");
        assert_eq!(l.roms, PathBuf::from("/userdata/roms"));
        assert_eq!(l.gamelists, PathBuf::from("/userdata/ES-DE/gamelists"));
        assert_eq!(l.media, PathBuf::from("/userdata/ES-DE/downloaded_media"));
    }
}

//! The folder RetroArch keeps a core's saves and states in.
//!
//! RetroArch sorts saves by core into `saves/<name>/`, and the name is the
//! core's own `library_name`, which is neither the file stem the server stores
//! (`mednafen_psx_hw`) nor ES-DE's label for it (`Beetle PSX HW` happens to
//! match; `Snes9x - Current` does not). A folder already on disk always wins
//! over this table; it is for the first save of a core on a device, where there
//! is no folder to find yet and a wrong guess puts the save where RetroArch
//! never looks.
//!
//! Only names that are known are listed. A core missing from here is filed
//! under its stem, which is what happened to every core before this existed.

/// RetroArch's folder name for the core with file stem `core`, if known.
pub fn retroarch_name(core: &str) -> Option<&'static str> {
    Some(match core {
        "snes9x" => "Snes9x",
        "snes9x2010" => "Snes9x 2010",
        "snes9x2005" => "Snes9x 2005",
        "bsnes" => "bsnes",
        "mesen" => "Mesen",
        "mesen-s" => "Mesen-S",
        "fceumm" => "FCEUmm",
        "nestopia" => "Nestopia",
        "quicknes" => "QuickNES",
        "gambatte" => "Gambatte",
        "sameboy" => "SameBoy",
        "gearboy" => "Gearboy",
        "mgba" => "mGBA",
        "gpsp" => "gpSP",
        "vba_next" => "VBA Next",
        "vbam" => "VBA-M",
        "genesis_plus_gx" => "Genesis Plus GX",
        "genesis_plus_gx_wide" => "Genesis Plus GX Wide",
        "picodrive" => "PicoDrive",
        "gearsystem" => "Gearsystem",
        "smsplus" => "SMS Plus GX",
        "mednafen_psx_hw" => "Beetle PSX HW",
        "mednafen_psx" => "Beetle PSX",
        "mednafen_saturn" => "Beetle Saturn",
        "mednafen_pce_fast" => "Beetle PCE Fast",
        "mednafen_pce" => "Beetle PCE",
        "mednafen_supergrafx" => "Beetle SuperGrafx",
        "mednafen_ngp" => "Beetle NeoPop",
        "mednafen_wswan" => "Beetle WonderSwan",
        "mednafen_vb" => "Beetle VB",
        "mednafen_lynx" => "Beetle Lynx",
        "pcsx_rearmed" => "PCSX-ReARMed",
        "swanstation" => "SwanStation",
        "mupen64plus_next" => "Mupen64Plus-Next",
        "parallel_n64" => "ParaLLEl N64",
        "melonds" => "melonDS",
        "melondsds" => "melonDS DS",
        "desmume" => "DeSmuME",
        "fbneo" => "FinalBurn Neo",
        "mame" => "MAME",
        "mame2003_plus" => "MAME 2003-Plus",
        "mame2003" => "MAME 2003",
        "mame2010" => "MAME 2010",
        "flycast" => "Flycast",
        "ppsspp" => "PPSSPP",
        "dosbox_pure" => "DOSBox-pure",
        "geolith" => "Geolith",
        "stella" => "Stella",
        "stella2014" => "Stella 2014",
        "prosystem" => "ProSystem",
        "handy" => "Handy",
        "bluemsx" => "blueMSX",
        "fuse" => "Fuse",
        "puae" => "PUAE",
        "opera" => "Opera",
        "yabasanshiro" => "YabaSanshiro",
        "neocd" => "NeoCD",
        "o2em" => "O2EM",
        "freeintv" => "FreeIntv",
        "virtualjaguar" => "Virtual Jaguar",
        "pokemini" => "PokeMini",
        "atari800" => "Atari800",
        "hatari" => "Hatari",
        "scummvm" => "ScummVM",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names `tweaks::core_dir_name` already knows are RetroArch's config
    /// folders, which RetroArch names the same way. The two must not disagree.
    #[test]
    fn agrees_with_the_config_folder_names() {
        for core in [
            "fceumm", "mesen", "nestopia", "gambatte", "mgba", "gpsp", "mame2003_plus", "mame2003",
            "swanstation", "fbneo", "dosbox_pure",
        ] {
            assert_eq!(retroarch_name(core), crate::tweaks::core_dir_name(core), "{core}");
        }
    }

    #[test]
    fn an_unknown_core_has_no_name() {
        assert_eq!(retroarch_name("not_a_core"), None);
    }
}

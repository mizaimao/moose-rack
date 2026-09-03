use std::path::{Path, PathBuf};

use super::Platform;

pub struct MacOs;

impl Platform for MacOs {
    fn scheme(&self) -> &'static str {
        "macos"
    }

    fn retroarch_roots(&self) -> &'static [&'static str] {
        &[
            "/Applications",
            "~/Applications",
            "~/Data/Games/Emulators/RetroArch",
        ]
    }

    /// The bundle keeps nothing but itself; everything else is under
    /// Application Support, so the install root has no say.
    fn retroarch_data_dir(&self, _install_root: &Path) -> PathBuf {
        crate::util::expand_tilde("~/Library/Application Support/RetroArch")
    }

    fn core_dirs(&self) -> &'static [&'static str] {
        super::linux::DISTRO_CORE_DIRS
    }
}

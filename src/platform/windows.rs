use std::path::{Path, PathBuf};

use super::Platform;

pub struct Windows;

impl Platform for Windows {
    fn scheme(&self) -> &'static str {
        "windows"
    }

    fn retroarch_roots(&self) -> &'static [&'static str] {
        &[
            "C:/RetroArch-Win64",
            "C:/Program Files/RetroArch",
            "C:/Program Files (x86)/RetroArch",
            "~/RetroArch",
        ]
    }

    /// APPDATA when the environment has it; otherwise the install directory,
    /// which is where a portable unzip keeps everything anyway.
    fn retroarch_data_dir(&self, install_root: &Path) -> PathBuf {
        std::env::var_os("APPDATA")
            .map(|a| PathBuf::from(a).join("RetroArch"))
            .unwrap_or_else(|| install_root.to_path_buf())
    }

    fn core_dirs(&self) -> &'static [&'static str] {
        super::linux::DISTRO_CORE_DIRS
    }
}

use std::path::{Path, PathBuf};

use super::Platform;

/// Where distro packages and Flatpak drop libretro cores.
///
/// Shared with the macOS and Windows schemes because it always was: this list
/// was never gated, and on those hosts none of it exists, so it costs three
/// failed `is_dir` calls and changes nothing.
pub const DISTRO_CORE_DIRS: &[&str] = &[
    "/usr/lib/x86_64-linux-gnu/libretro",
    "/usr/lib/libretro",
    "/usr/local/lib/libretro",
];

pub struct Linux;

impl Platform for Linux {
    fn scheme(&self) -> &'static str {
        "linux"
    }

    fn retroarch_roots(&self) -> &'static [&'static str] {
        &["/usr", "/usr/local", "/app", "~/.local", "~/RetroArch"]
    }

    /// XDG, which is what RetroArch itself follows on desktop Linux — it
    /// ignores `portable.txt` there entirely.
    fn retroarch_data_dir(&self, _install_root: &Path) -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::util::expand_tilde("~/.config"))
            .join("retroarch")
    }

    fn core_dirs(&self) -> &'static [&'static str] {
        DISTRO_CORE_DIRS
    }
}

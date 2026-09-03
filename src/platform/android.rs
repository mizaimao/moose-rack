// Android 13 — the AYN Thor and the Retroid Pocket Mini V2.
//
// Nothing here has been run on a device. This scheme exists so the target
// compiles as itself rather than falling through the desktop-Linux branch and
// being quietly wrong; the values are the shape the port plan calls for, and
// Steps 6 to 8 are where they get tested. Anything asserted as *measured*
// belongs in the KNULLI scheme, not this one.

use std::path::{Path, PathBuf};

use super::Platform;

pub struct Android;

impl Platform for Android {
    fn scheme(&self) -> &'static str {
        "android"
    }

    /// `current_exe()` is the zygote here — `/system/bin/app_process64`, and
    /// nowhere near the app's data — so the walk that
    /// [`crate::datadir::choose`] does cannot work. The app's private files
    /// directory is the anchor instead, and it is handed in at startup rather
    /// than guessed, because only the Android runtime knows it.
    ///
    /// Returning `None` until Step 6 wires that through is deliberate: a wrong
    /// path here creates an empty `cache.sqlite3` and reports a library of
    /// nothing, which is the failure mode `datadir` was written to end.
    fn data_root(&self) -> Option<PathBuf> {
        None
    }

    /// RetroArch on Android is a separate app driven by an Intent, not a
    /// binary on a path we can search. The roots are only what the shared
    /// discovery code needs to not find; launching is Step 7's Kotlin plugin.
    fn retroarch_roots(&self) -> &'static [&'static str] {
        &["/data/data/com.retroarch"]
    }

    fn retroarch_data_dir(&self, _install_root: &Path) -> PathBuf {
        PathBuf::from("/data/data/com.retroarch/files")
    }

    fn core_dirs(&self) -> &'static [&'static str] {
        &["/data/data/com.retroarch/cores"]
    }
}

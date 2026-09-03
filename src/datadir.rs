// Where the app's files are, and how it finds them.
//
// Config, cache, core map and library are all addressed relative to one
// directory, and where that is depends on how the app was started. This used
// to live in the Tauri layer, which meant the SDL front end opened
// `cache.sqlite3` relative to whatever directory it happened to be launched
// from — and `Cache::open` creates the file, so instead of failing it made an
// empty database and reported a library of nothing.
//
// Nothing here is created and nothing is written to the home directory.

use std::path::{Path, PathBuf};

/// Find the data root and make it the working directory.
///
/// Config, cache, core map and library are all addressed relative to one
/// directory, but where that is depends on how the app was started:
///
/// * a config.toml the user put beside the executable, or in the cwd
/// * a dev checkout — the repo itself, found by walking up for the core map
/// * otherwise the executable's own directory
///
/// Nothing is created and nothing is written to the home directory. See
/// [`choose`], which holds the ordering and is where the tests are.
pub fn anchor() {
    // Android has none of the layouts below.
    //
    // There is no executable to sit beside — the process is `app_process64`
    // from /system/bin — no checkout to walk up to, and the working directory
    // is `/`, which is read-only. Falling through to `choose` finds nothing,
    // leaves the cwd alone, and the first relative path the app opens fails:
    //
    //     opening metadata cache: opening cache at cache.sqlite3
    //     Error code 14: Unable to open the database file
    //
    // which aborted the app before it drew anything. So Android gets the one
    // directory it does have: the private files dir every app is given, which
    // is writable, survives updates, and is removed when the app is
    // uninstalled.
    #[cfg(target_os = "android")]
    {
        match android_files_dir() {
            Some(root) => {
                // Created here, unlike every other branch. The directory
                // belongs to this app and Android does not make it for us —
                // and the alternative is the abort above.
                let _ = std::fs::create_dir_all(&root);
                if std::env::set_current_dir(&root).is_err() {
                    eprintln!("warning: could not enter {}", root.display());
                }
            }
            None => eprintln!("warning: could not read this app's package name"),
        }
        return;
    }

    #[cfg(not(target_os = "android"))]
    {
        anchor_by_layout();
    }
}

/// The private files directory Android gives every app: `/data/user/0/<pkg>/files`.
///
/// Read from `/proc/self/cmdline` rather than hardcoded or passed in, because
/// the package name is decided by tauri.conf.json's `identifier` and a second
/// copy of it here is a second thing to keep in step. cmdline is the process
/// name, which for an Android app *is* the package name.
///
/// `/data/user/0` rather than `/data/data`: the latter is a symlink to it that
/// exists for compatibility, and the real path is the one that keeps working
/// under multiple users and work profiles.
#[cfg(target_os = "android")]
fn android_files_dir() -> Option<PathBuf> {
    let cmdline = std::fs::read("/proc/self/cmdline").ok()?;
    // NUL-terminated, and argv[0] is all we want.
    let end = cmdline.iter().position(|&b| b == 0).unwrap_or(cmdline.len());
    let pkg = std::str::from_utf8(&cmdline[..end]).ok()?.trim();
    if pkg.is_empty() {
        return None;
    }
    Some(PathBuf::from("/data/user/0").join(pkg).join("files"))
}

/// The desktop rule: config beside the executable, else a checkout, else the
/// executable's own directory. See [`choose`].
#[cfg(not(target_os = "android"))]
fn anchor_by_layout() {
    let cwd = std::env::current_dir().ok();
    let exe = std::env::current_exe().ok();
    match choose(cwd.as_deref(), exe.as_deref(), &|p| p.is_file()) {
        Some(root) => {
            let _ = std::env::set_current_dir(&root);
        }
        None => eprintln!(
            "warning: could not locate the executable; leaving the working directory alone"
        ),
    }
}

const MARKER: &str = "data/esde-core-map.json";
const CONFIG: &str = "config.toml";

/// Decide the data root. Split out from [`anchor`] because the
/// *order* is the part that was wrong, and a function that calls
/// `set_current_dir` cannot be tested — the working directory is per-process,
/// so tests would fight each other.
///
/// `is_file` is injected for the same reason: the interesting cases are layouts
/// nobody has on disk (a Windows portable install, a `.app` in /Applications).
pub fn choose(
    cwd: Option<&Path>,
    exe: Option<&Path>,
    is_file: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let exe_dir = exe.and_then(app_dir);

    // 1. A config.toml beside the executable, or in the working directory.
    //
    // This is what a portable install looks like and what everyone expects of
    // one: unzip the exe, drop config.toml next to it, run it. Without this the
    // Windows build ignored that file completely — it anchored to %USERPROFILE%
    // \RomM and looked for the config there, so a config sitting right beside
    // the exe was never on any path it consulted.
    //
    // Checked before the marker search because it is the more specific signal:
    // a config.toml is somewhere a user deliberately put a file, whereas the
    // marker only says "a checkout is somewhere above us".
    for dir in [cwd, exe_dir.as_deref()].into_iter().flatten() {
        if is_file(&dir.join(CONFIG)) {
            return Some(dir.to_path_buf());
        }
    }

    // 2. A source checkout, if we are running from one.
    let ancestors = cwd
        .into_iter()
        .flat_map(Path::ancestors)
        .chain(exe.into_iter().flat_map(Path::ancestors));
    for root in ancestors {
        if is_file(&root.join(MARKER)) {
            return Some(root.to_path_buf());
        }
    }

    // 3. The executable's own directory, and nowhere else.
    //
    // The app lives where it was put. It does not create a folder in the home
    // directory, and nothing is written from here at all — the core map is
    // compiled into the binary (`CoreMap::load_or_embedded`), so no file has to
    // exist on disk before startup.
    exe_dir
}

/// The directory a user would say the app is "in".
///
/// On macOS the executable is buried at `Moose Rack.app/Contents/MacOS/`,
/// which is inside the signed bundle: writing there breaks the signature and is
/// wiped on update. The directory holding the `.app` is the equivalent of the
/// folder a loose `.exe` sits in, so that is what gets used.
pub fn app_dir(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let mut cur = dir;
    while let Some(parent) = cur.parent() {
        if cur.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
            return Some(parent.to_path_buf());
        }
        cur = parent;
    }
    Some(dir.to_path_buf())
}


#[cfg(test)]
mod tests {
    use super::*;

    /// holding the `.app` is the app's location as far as data goes — the
    /// equivalent of the folder a loose .exe sits in.
    #[test]
    fn a_macos_bundle_resolves_to_the_directory_holding_it() {
        assert_eq!(
            app_dir(Path::new("/Applications/Moose Rack.app/Contents/MacOS/moose-gui")),
            Some(PathBuf::from("/Applications"))
        );
        assert_eq!(
            app_dir(Path::new(
                "/Users/frank/Projects/moose-rack/Moose Rack.app/Contents/MacOS/moose-gui"
            )),
            Some(PathBuf::from("/Users/frank/Projects/moose-rack"))
        );
    }

    /// A loose executable — the Windows and Linux shape — anchors to its own
    /// directory. Unzip it anywhere, drop a config.toml beside it, run it.
    #[test]
    fn a_loose_executable_anchors_beside_itself() {
        assert_eq!(
            app_dir(Path::new("D:/Emulators/Moose Rack/moose-rack.exe")),
            Some(PathBuf::from("D:/Emulators/Moose Rack"))
        );
        assert_eq!(
            app_dir(Path::new("/opt/moose-rack/moose-rack")),
            Some(PathBuf::from("/opt/moose-rack"))
        );
    }

    /// A directory merely *containing* the string "app" is not a bundle; only a
    /// `.app` extension counts, or an install under /home/apps/ would anchor a
    /// level too high.
    #[test]
    fn only_a_dot_app_extension_counts_as_a_bundle() {
        assert_eq!(
            app_dir(Path::new("/home/frank/apps/moose-rack")),
            Some(PathBuf::from("/home/frank/apps"))
        );
        assert_eq!(
            app_dir(Path::new("/srv/appdata/moose-rack/moose-rack")),
            Some(PathBuf::from("/srv/appdata/moose-rack"))
        );
    }

    /// A set of paths that "exist", for driving `choose_data_root` over layouts
    /// nobody has on this disk.
    fn exists(paths: &[&str]) -> impl Fn(&Path) -> bool + use<> {
        let owned: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
        move |p: &Path| owned.iter().any(|o| Path::new(o) == p)
    }

    /// The reported bug, as a test: a portable Windows install with its config
    /// beside the exe, launched from a working directory that has nothing to do
    /// with it. This used to land on %USERPROFILE%\RomM, where the config was
    /// never on any path the app consulted.
    ///
    /// The checkout marker above the working directory is what gives this test
    /// teeth. Without it the answer is right either way — step 3 also returns
    /// the executable's directory — so the ordering would not actually be under
    /// test. A mutation run (delete the config check, watch this still pass)
    /// is what surfaced that.
    #[test]
    fn a_config_beside_the_executable_wins() {
        let root = choose(
            Some(Path::new("C:/Users/frank/checkout/sub")),
            Some(Path::new("D:/Emulators/Moose Rack/moose-rack.exe")),
            &exists(&[
                "D:/Emulators/Moose Rack/config.toml",
                // A checkout above the cwd, which wins if the config is
                // consulted second instead of first.
                "C:/Users/frank/checkout/data/esde-core-map.json",
            ]),
        );
        assert_eq!(root, Some(PathBuf::from("D:/Emulators/Moose Rack")));
    }

    /// The working directory is checked before the executable, so running from
    /// a configured folder uses that config rather than the app's own.
    #[test]
    fn the_working_directory_is_preferred_over_the_executables() {
        let root = choose(
            Some(Path::new("/srv/moose-rack-live")),
            Some(Path::new("/opt/moose-rack/moose-rack")),
            &exists(&["/srv/moose-rack-live/config.toml", "/opt/moose-rack/config.toml"]),
        );
        assert_eq!(root, Some(PathBuf::from("/srv/moose-rack-live")));
    }

    /// A config.toml is a deliberate act; the core map only says a checkout is
    /// somewhere above us. The specific signal has to win, or a developer with
    /// a checkout above their portable install gets the wrong data directory.
    #[test]
    fn a_config_beats_a_checkout_found_further_up() {
        let root = choose(
            Some(Path::new("/home/frank/Projects/moose-rack/portable")),
            Some(Path::new("/home/frank/Projects/moose-rack/portable/moose-rack")),
            &exists(&[
                "/home/frank/Projects/moose-rack/portable/config.toml",
                "/home/frank/Projects/moose-rack/data/esde-core-map.json",
            ]),
        );
        assert_eq!(
            root,
            Some(PathBuf::from("/home/frank/Projects/moose-rack/portable"))
        );
    }

    /// With no config anywhere, a checkout above the working directory is used
    /// — this is what makes `cargo run` from a subdirectory work.
    #[test]
    fn a_checkout_is_found_by_walking_up() {
        let root = choose(
            Some(Path::new("/home/frank/Projects/moose-rack/src-tauri")),
            Some(Path::new("/home/frank/Projects/moose-rack/target/debug/moose-gui")),
            &exists(&["/home/frank/Projects/moose-rack/data/esde-core-map.json"]),
        );
        assert_eq!(root, Some(PathBuf::from("/home/frank/Projects/moose-rack")));
    }

    /// Nothing configured and no checkout: the app lives where it was put. The
    /// previous behavior — inventing ~/Moose Rack and writing a core map into it —
    /// is what this asserts is gone.
    #[test]
    fn with_nothing_to_go_on_it_anchors_beside_the_app_not_in_home() {
        let root = choose(
            Some(Path::new("/")),
            Some(Path::new("/Applications/Moose Rack.app/Contents/MacOS/moose-gui")),
            &exists(&[]),
        );
        assert_eq!(
            root,
            Some(PathBuf::from("/Applications")),
            "the bundle resolves to the folder holding it, and never to $HOME"
        );
        assert!(
            !format!("{root:?}").contains("Moose Rack/"),
            "no invented data folder"
        );
    }

    /// Without an executable path there is nothing to anchor to, and leaving the
    /// working directory alone beats guessing.
    #[test]
    fn no_executable_and_no_markers_changes_nothing() {
        assert_eq!(choose(Some(Path::new("/tmp")), None, &exists(&[])), None);
    }
}

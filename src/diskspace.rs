//! How much room is left where the library lives.
//!
//! Needed before a bulk download, because the failure without it is the worst
//! kind: a transfer that runs for an hour, fills the disk, and leaves both a
//! half-written game and no space to fix it with. Asking first costs one system
//! call.
//!
//! Three implementations because there is no portable one in `std`. Each is the
//! ordinary way to ask on that platform, and the fallback is `None` rather than
//! a guess — a wrong number here is worse than no number, since the caller can
//! say "could not check" but cannot un-fill a disk.

use std::path::Path;

/// Bytes available to this user at `path`, or `None` if it cannot be read.
///
/// "Available" rather than "free": on Unix the two differ by the reserve set
/// aside for root, and writing into that reserve is not something an ordinary
/// process gets to do. Reporting the larger number would promise space that
/// does not exist.
pub fn available(path: &Path) -> Option<u64> {
    imp::available(path)
}

/// Whether `need` bytes fit, keeping a margin free.
///
/// The margin is not superstition. A disk taken to zero stops the things that
/// were not part of this download — the cache database cannot checkpoint, saves
/// cannot be written, and on a system disk the machine itself starts failing.
/// Leaving a couple of gigabytes means a bad estimate costs a failed copy
/// rather than a wedged machine.
pub const MARGIN: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Room for the download and the margin.
    Yes { available: u64 },
    /// Not enough. `short` is how much more would be needed.
    No { available: u64, short: u64 },
    /// The check itself failed. Deliberately distinct from `No`: refusing a
    /// download because a syscall failed would be a worse bug than the one this
    /// guards against.
    Unknown,
}

pub fn fits(path: &Path, need: u64) -> Fit {
    match available(path) {
        None => Fit::Unknown,
        Some(free) => {
            let want = need.saturating_add(MARGIN);
            if free >= want {
                Fit::Yes { available: free }
            } else {
                Fit::No { available: free, short: want - free }
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub fn available(path: &Path) -> Option<u64> {
        // statvfs needs a path that exists; the library folder may not yet, so
        // walk up until something does rather than reporting failure.
        let mut probe = path;
        loop {
            if probe.exists() {
                break;
            }
            probe = probe.parent()?;
        }
        let c = CString::new(probe.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c` is a valid NUL-terminated path and `stat` is only read
        // after the call reports success.
        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c.as_ptr(), &mut stat) != 0 {
                return None;
            }
            // f_bavail, not f_bfree: the latter includes blocks reserved for
            // root, which this process cannot write into.
            Some(stat.f_bavail as u64 * stat.f_frsize as u64)
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use std::process::Command;

    /// Asked of PowerShell rather than through a Win32 binding, to avoid a
    /// dependency for one number. The call is once per download, not per file.
    pub fn available(path: &Path) -> Option<u64> {
        let mut probe = path;
        loop {
            if probe.exists() {
                break;
            }
            probe = probe.parent()?;
        }
        let full = probe.canonicalize().ok()?;
        let s = full.to_string_lossy();
        // A canonicalised Windows path arrives as \\?\C:\... ; the drive letter
        // is what GetDiskFreeSpace wants.
        let drive = s.trim_start_matches(r"\\?\").chars().next()?;
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-PSDrive -Name {drive}).Free"
                ),
            ])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;
    pub fn available(_: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The margin is the point: a download that exactly fills the disk is not a
    /// download that fits.
    #[test]
    fn a_download_that_would_fill_the_disk_does_not_fit() {
        let free = 10 * 1024 * 1024 * 1024u64; // 10 GB
        // 9 GB plus a 2 GB margin is more than 10 GB, so this must be refused
        // even though the bytes alone would go in.
        assert!(matches!(check(free, 9 * 1024 * 1024 * 1024), Fit::No { .. }));
        assert!(matches!(check(free, 4 * 1024 * 1024 * 1024), Fit::Yes { .. }));
    }

    #[test]
    fn the_shortfall_says_how_much_more_is_needed() {
        let free = 1024u64;
        match check(free, 1024) {
            Fit::No { short, .. } => assert_eq!(short, MARGIN),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// A download larger than the address space must not wrap around into
    /// "fits". Saturating arithmetic is what stops that.
    #[test]
    fn an_absurd_request_is_refused_rather_than_overflowing() {
        assert!(matches!(check(u64::MAX - 1, u64::MAX), Fit::No { .. }));
    }

    /// Reading the real filesystem, since the whole point is the syscall.
    #[test]
    fn the_current_directory_reports_some_space() {
        let got = available(Path::new("."));
        assert!(got.is_some(), "no platform implementation answered");
        assert!(got.unwrap() > 0, "a writable disk with zero bytes available");
    }

    /// A path that does not exist yet is the normal case: the library folder is
    /// created by the download it is being checked for.
    #[test]
    fn a_folder_that_does_not_exist_yet_still_reports_its_disk() {
        let missing = std::env::current_dir().unwrap().join("no-such-folder-here/nested");
        assert!(available(&missing).is_some(), "should walk up to a real parent");
    }

    fn check(free: u64, need: u64) -> Fit {
        let want = need.saturating_add(MARGIN);
        if free >= want {
            Fit::Yes { available: free }
        } else {
            Fit::No { available: free, short: want - free }
        }
    }
}

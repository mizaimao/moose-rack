//! Small helpers shared across the CLI, TUI and GUI.
//!
//! Each of these existed in two or three places before; keeping one copy means
//! sizes are formatted identically everywhere and `~` expands the same way in
//! every config field.

use std::path::{Path, PathBuf};

/// Human-readable byte size, e.g. `1.5 GB`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    for (i, unit) in UNITS.iter().enumerate() {
        if value < 1024.0 || i == UNITS.len() - 1 {
            return format!("{value:.1} {unit}");
        }
        value /= 1024.0;
    }
    unreachable!()
}

/// Expand a leading `~/` against `$HOME`. Other paths pass through unchanged.
pub fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// Total size of everything under `dir`. Missing directories count as zero, so
/// callers can ask about a cache that does not exist yet.
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => dir_size(&entry.path()),
            Ok(_) => entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Build an HTTP client, installing the TLS backend on first use.
///
/// We select rustls without a bundled provider (`rustls-no-provider`) so the
/// build does not drag in `aws-lc-sys`, a large C library whose build script
/// needs a full cross toolchain and therefore breaks Windows and Linux builds.
/// The cost is that *something* must install a provider before the first
/// client is created — reqwest panics otherwise — so every client in this
/// project is built here.
/// Make sure rustls has a cipher implementation before anything builds a client.
///
/// Built with `rustls-no-provider`, so this is not optional and not lazy: a
/// client built without it panics on the first request, on whatever thread made
/// it. Separate from [`http_client`] because the front ends build blocking
/// clients of their own and need the same guarantee.
pub fn install_tls() {
    static TLS: std::sync::Once = std::sync::Once::new();
    TLS.call_once(|| {
        // Only errors if a provider is already installed, which is harmless.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Give a client builder the certificate roots it should trust.
///
/// A no-op everywhere except Android, where it is the difference between a
/// working HTTPS request and an abort.
///
/// reqwest verifies certificates through `rustls-platform-verifier`, which asks
/// the operating system. That is the right answer on macOS, Windows and Linux —
/// it picks up anything the user or their employer has installed. On Android
/// the trust store is only reachable through Java, so the verifier has to be
/// handed a JNI environment before the first request; without one it does not
/// fail the request, it panics the thread:
///
/// ```text
/// thread 'tokio-rt-worker' panicked at rustls-platform-verifier/src/android.rs:90
/// Expect rustls-platform-verifier to be initialized
/// ```
///
/// Tauri does not expose the Android context during setup, which is the whole
/// of tauri-apps/tauri#13267, so there is nowhere to do that initialisation
/// from. Android therefore carries Mozilla's root list in the binary instead.
///
/// What this costs: a certificate signed by something only the *device* trusts
/// is rejected on Android and accepted elsewhere. That is a smaller loss than
/// it sounds — since Android 7 an app ignores user-installed authorities anyway
/// unless it ships a network security config — but it is a real difference, and
/// a self-signed RomM server reached over https will not work on Android until
/// this is revisited. Plain http is unaffected, and so is any server with a
/// certificate from a public authority.
pub fn tls_roots(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    #[cfg(not(target_os = "android"))]
    {
        b
    }

    #[cfg(target_os = "android")]
    {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        // `builder_with_provider` rather than `builder`: this crate selects
        // rustls with no default provider, so the process-wide one installed by
        // `install_tls` is the only one there is.
        let cfg = rustls::ClientConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        b.use_preconfigured_tls(cfg)
    }
}

pub fn http_client(timeout: Option<std::time::Duration>) -> anyhow::Result<reqwest::Client> {
    install_tls();
    let mut b = tls_roots(reqwest::Client::builder())
        .user_agent(concat!("moose-rack/", env!("CARGO_PKG_VERSION")))
        // Bound *connecting* even when the request itself is unbounded.
        //
        // A server that is off the network blackholes the SYN rather than
        // refusing it, so without this the wait is the OS TCP timeout — over a
        // minute, and longer once reqwest retries. That is what left a launch
        // sitting on "checking saves with the server…" forever: `auto_sync`
        // was written to give up and offer "play anyway", and it never got the
        // chance to. A refused connection was always instant, which is why
        // this only ever showed up with the server switched off rather than
        // merely not listening.
        .connect_timeout(std::time::Duration::from_secs(5))
        // Between bytes, not for the whole transfer — a multi-gigabyte ROM
        // download resets this on every chunk, while a server that accepts the
        // connection and then says nothing still gives up.
        .read_timeout(std::time::Duration::from_secs(30));
    if let Some(d) = timeout {
        b = b.timeout(d);
    }
    b.build().map_err(Into::into)
}

/// Now, as the server writes it: `2026-08-14T09:30:00`.
///
/// Hand-rolled rather than pulled from a date library, because this is the only
/// date arithmetic in the project and the format has to match what the server
/// already stores. `last_played` is compared and sorted as text, so a locally
/// recorded session written as epoch seconds — or with a timezone suffix the
/// server never uses — would sort either above everything or below it, and the
/// "continue playing" row would be wrong in a way nothing announces.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_from_epoch(secs)
}

/// UTC, always. A local time would make a library carried between timezones
/// record sessions that appear to happen in the wrong order.
pub fn iso_from_epoch(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a calendar date.
///
/// Howard Hinnant's algorithm, which shifts the year to start in March so the
/// leap day lands at the end and the month-length arithmetic becomes a single
/// expression with no table and no special case for February.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A duration in words: "18 minutes", "2 h 05 m".
///
/// Seconds are dropped above a minute. Nobody reads "1 h 23 m 07 s", and the
/// figure is a wall-clock measurement of a window being open, which is not
/// accurate to the second in any sense that matters.
pub fn spell_duration(seconds: i64) -> String {
    match seconds {
        s if s < 60 => format!("{s} seconds"),
        s if s < 3600 => format!("{} minutes", s / 60),
        s => format!("{} h {:02} m", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_scales() {
        assert_eq!(human(0), "0.0 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(1_610_612_736), "1.5 GB");
    }

    #[test]
    fn tilde_only_expands_a_leading_home_reference() {
        assert_eq!(expand_tilde("/tmp/x"), PathBuf::from("/tmp/x"));
        assert_eq!(expand_tilde("relative/x"), PathBuf::from("relative/x"));
        // A bare "~" is not a prefix we expand, matching shell-free tools.
        assert_eq!(expand_tilde("~notuser/x"), PathBuf::from("~notuser/x"));
    }

    /// Checked against dates whose epoch seconds are known independently. The
    /// leap-year rules are the part that goes wrong quietly: 2000 is a leap
    /// year and 1900 is not, and an implementation that gets that backwards is
    /// correct for decades either side.
    #[test]
    fn the_clock_agrees_with_dates_whose_answers_are_known() {
        assert_eq!(iso_from_epoch(0), "1970-01-01T00:00:00");
        assert_eq!(iso_from_epoch(1_000_000_000), "2001-09-09T01:46:40");
        assert_eq!(iso_from_epoch(951_782_400), "2000-02-29T00:00:00");
        assert_eq!(iso_from_epoch(1_709_164_800), "2024-02-29T00:00:00");
        assert_eq!(iso_from_epoch(1_767_225_599), "2025-12-31T23:59:59");
        assert_eq!(iso_from_epoch(1_767_225_600), "2026-01-01T00:00:00");
    }

    /// `last_played` is sorted as text against timestamps the server wrote, so
    /// the format has to be identical: same width, same separator, no zone.
    #[test]
    fn a_locally_recorded_time_sorts_against_the_servers() {
        let ours = iso_from_epoch(1_767_225_600);
        let theirs = "2025-06-01T12:00:00";
        assert_eq!(ours.len(), theirs.len());
        assert!(ours.as_str() > theirs, "{ours} should sort after {theirs}");
        assert!(!ours.contains('Z') && !ours.contains('+'), "the server writes no zone");
    }

    #[test]
    fn durations_read_as_someone_would_say_them() {
        assert_eq!(spell_duration(12), "12 seconds");
        assert_eq!(spell_duration(600), "10 minutes");
        assert_eq!(spell_duration(3600), "1 h 00 m");
        assert_eq!(spell_duration(7_500), "2 h 05 m");
    }
}

/// A path the webview can load.
///
/// `canonicalize` on Windows returns a *verbatim* path — `\\?\C:\Users\...` —
/// which is a perfectly good path for the Windows API and not one that
/// survives being put through a URL, a scope match and a decode on the way to
/// an <img>. Every picture in this app arrives that way: covers, screenshots,
/// save-state thumbnails, and the console art the Appearance tab cycles
/// through. On macOS and Linux there is no such prefix and this returns what
/// it was given.
///
/// Kept as string surgery rather than a dependency: the prefix is four
/// characters, the UNC form is one more case, and both are stable parts of the
/// platform.
pub fn webview_path(path: &Path) -> String {
    let shown = path.display().to_string();
    if let Some(rest) = shown.strip_prefix(r"\\?\UNC\") {
        // \\?\UNC\server\share -> \\server\share
        return format!(r"\\{rest}");
    }
    match shown.strip_prefix(r"\\?\") {
        // Only a drive path: \\?\C:\x -> C:\x. Anything else keeps the prefix,
        // because without it that path does not name the same file.
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
        _ => shown,
    }
}

#[cfg(test)]
mod webview_path_tests {
    use super::*;

    /// The Windows form. `canonicalize` returns it, an <img> cannot use it, and
    /// the app has no way to notice from any other platform — which is exactly
    /// the shape of bug that gets reported as "the console pictures do not
    /// change on Windows" and reproduces nowhere.
    #[test]
    fn a_verbatim_drive_path_loses_its_prefix() {
        assert_eq!(webview_path(Path::new(r"\\?\C:\media\_platforms\snes.png")), r"C:\media\_platforms\snes.png");
        assert_eq!(webview_path(Path::new(r"\\?\D:\x.png")), r"D:\x.png");
    }

    /// A network path keeps being a network path.
    #[test]
    fn a_verbatim_unc_path_becomes_an_ordinary_one() {
        assert_eq!(webview_path(Path::new(r"\\?\UNC\nas\roms\x.png")), r"\\nas\roms\x.png");
    }

    /// Anything else is handed back as it came: a prefix that is not a drive
    /// and not UNC names a device, and dropping it would name a different file
    /// or nothing at all.
    #[test]
    fn everything_else_is_left_alone() {
        for p in [
            "/Users/frank/media/_platforms/snes.png",
            "./library/media/covers/1.png",
            r"C:\media\snes.png",
            r"\\?\Volume{2eca078d}\x.png",
        ] {
            assert_eq!(webview_path(Path::new(p)), p);
        }
    }
}

/// The wall clock, as `H:MM AM` in local time.
///
/// Through libc rather than a date crate: the only thing wanted is two numbers
/// in the corner of a handheld, and `chrono` is a dependency and a build for
/// four fields of a struct the C library already fills in. The device knows its
/// own zone — `/etc/timezone` on the Flip reads America/New_York — and
/// `localtime_r` is what reads it.
///
/// `None` if the clock cannot be read, which is a real answer: a handheld that
/// has been off the network since it was made has no idea what time it is, and
/// a wrong time in the corner is worse than no time.
///
/// Unix only, and deliberately. `localtime_r` is POSIX and does not exist in
/// the Windows C runtime, which is where this broke the build — the one caller
/// is the handheld's status corner, and there is no handheld running Windows.
/// A stub that guessed at a time would be worse than the missing clock this
/// returns.
#[cfg(not(unix))]
pub fn local_hhmm() -> Option<String> {
    None
}

#[cfg(unix)]
pub fn local_hhmm() -> Option<String> {
    // SAFETY: `localtime_r` fills a caller-owned `tm`, so there is no shared
    // buffer to race over — which is the reason it exists rather than
    // `localtime`.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        if now == -1 {
            return None;
        }
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return None;
        }
        // Twelve-hour, because that is what the corner of a device reads like
        // where this one lives. Midnight is 12 AM and noon is 12 PM — the two
        // a naive `hour % 12` gets wrong, and the two nobody notices until the
        // clock says 0:20.
        let suffix = if tm.tm_hour < 12 { "AM" } else { "PM" };
        let hour = match tm.tm_hour % 12 {
            0 => 12,
            h => h,
        };
        Some(format!("{hour}:{:02} {suffix}", tm.tm_min))
    }
}

#[cfg(test)]
mod clock_tests {
    /// The twelve-hour conversion, without waiting for the clock to come round.
    ///
    /// Midnight and noon are the two `hour % 12` gets wrong: both land on 0,
    /// and a corner that reads `0:20 AM` is the sort of thing that only shows
    /// up once a day.
    fn twelve(hour: i32) -> String {
        let suffix = if hour < 12 { "AM" } else { "PM" };
        let h = match hour % 12 {
            0 => 12,
            h => h,
        };
        format!("{h}:00 {suffix}")
    }

    #[test]
    fn midnight_and_noon_are_twelve_not_zero() {
        assert_eq!(twelve(0), "12:00 AM", "midnight read as zero");
        assert_eq!(twelve(12), "12:00 PM", "noon read as zero");
    }

    #[test]
    fn the_rest_of_the_day_reads_normally() {
        assert_eq!(twelve(1), "1:00 AM");
        assert_eq!(twelve(11), "11:00 AM");
        assert_eq!(twelve(13), "1:00 PM");
        assert_eq!(twelve(23), "11:00 PM");
    }
}

// What the device is, for the About page.
//
// KNULLI's own System Information page lists the board, the build, the kernel,
// the processor and its cores, how much memory and storage are in use, and the
// temperature. This gathers the same things, and it gathers them the same way
// KNULLI does — out of `/proc` and `/sys`, which are files, so there is no
// dependency and nothing to keep running.
//
// Every field is optional on purpose. A Mac has no `/sys/class/thermal` and no
// battery worth reading through this path, and a field that is not there is
// left out rather than shown as a zero. That is also why this returns pairs
// rather than a struct: the About page draws whatever came back.

use std::path::Path;

/// One line of the page.
pub type Fact = (&'static str, String);

/// Everything this machine will say about itself.
pub fn facts() -> Vec<Fact> {
    let mut out = Vec::new();
    let mut add = |label: &'static str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            out.push((label, v));
        }
    };
    add("Board", board());
    add("System", system());
    add("Kernel", kernel());
    add("Processor", cpu_model());
    add("Cores", cores().map(|n| n.to_string()));
    add("Memory", memory());
    add("Storage", storage());
    add("Temperature", temperature());
    add("Uptime", uptime());
    out
}

/// The model name, as the firmware reports it.
///
/// `/sys/firmware/devicetree/base/model` is what a device-tree machine calls
/// itself, and the Flip fills it in. The strings there are NUL-terminated, so
/// the trailing byte has to go or it prints as a stray box.
fn board() -> Option<String> {
    for path in [
        "/sys/firmware/devicetree/base/model",
        "/sys/devices/virtual/dmi/id/product_name",
    ] {
        if let Some(text) = read(path) {
            return Some(text);
        }
    }
    if cfg!(target_os = "macos") {
        return sysctl("hw.model");
    }
    None
}

/// The build this is running on: `batocera.version` on KNULLI, `os-release`
/// elsewhere, and the macOS version on a Mac.
fn system() -> Option<String> {
    if let Some(v) = read("/usr/share/batocera/batocera.version") {
        return Some(v.split_whitespace().take(3).collect::<Vec<_>>().join(" "));
    }
    if let Some(name) = read("/etc/os-release").and_then(|t| field(&t, "PRETTY_NAME=")) {
        return Some(name);
    }
    if cfg!(target_os = "macos") {
        return run("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}"));
    }
    None
}

fn kernel() -> Option<String> {
    read("/proc/sys/kernel/osrelease").or_else(|| run("uname", &["-r"]))
}

/// The processor's own name for itself.
///
/// ARM boards mostly leave `model name` out of `/proc/cpuinfo` and put the part
/// in `Hardware`, so both are tried — otherwise the Flip shows nothing here
/// while every desktop shows a name.
fn cpu_model() -> Option<String> {
    if let Some(text) = read("/proc/cpuinfo") {
        for key in ["model name\t: ", "Hardware\t: ", "Processor\t: "] {
            if let Some(v) = field(&text, key) {
                return Some(v);
            }
        }
    }
    if cfg!(target_os = "macos") {
        return sysctl("machdep.cpu.brand_string");
    }
    None
}

fn cores() -> Option<usize> {
    std::thread::available_parallelism().ok().map(|n| n.get())
}

/// Used and total, in whole megabytes.
///
/// "Available" rather than "free": Linux spends idle memory on cache and gives
/// it back on demand, so free is always near nothing and reporting it says the
/// machine is full when it is not.
fn memory() -> Option<String> {
    let text = read("/proc/meminfo")?;
    let kb = |key: &str| -> Option<u64> {
        field(&text, key)?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    };
    let total = kb("MemTotal:")?;
    let available = kb("MemAvailable:").or_else(|| kb("MemFree:"))?;
    Some(format!(
        "{} of {} used",
        mb(total.saturating_sub(available)),
        mb(total)
    ))
}

/// How full the card is.
///
/// `/userdata` on KNULLI, because that is the partition the ROMs and saves are
/// on and the only one whose fullness anybody can do anything about. The root
/// filesystem there is read-only and its number would mean nothing.
fn storage() -> Option<String> {
    let target = if Path::new("/userdata").is_dir() {
        "/userdata"
    } else {
        "/"
    };
    let out = run("df", &["-k", target])?;
    let line = out.lines().nth(1)?;
    let mut cols = line.split_whitespace().skip(1);
    let total: u64 = cols.next()?.parse().ok()?;
    let used: u64 = cols.next()?.parse().ok()?;
    Some(format!("{} of {} used", gb(used), gb(total)))
}

/// The hottest thermal zone, in whole degrees.
///
/// Millidegrees in the file; a board with several zones reports the highest,
/// since that is the one that throttles.
fn temperature() -> Option<String> {
    let zones = std::fs::read_dir("/sys/class/thermal").ok()?;
    let hottest = zones
        .flatten()
        .filter_map(|z| read(z.path().join("temp").to_str()?))
        .filter_map(|t| t.parse::<i64>().ok())
        .max()?;
    (hottest > 0).then(|| format!("{}°C", hottest / 1000))
}

fn uptime() -> Option<String> {
    let text = read("/proc/uptime")?;
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    let minutes = (seconds / 60.0) as u64;
    Some(match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m}m"),
    })
}

/// A `KEY=value` or `Key\t: value` line out of a file of them.
fn field(text: &str, key: &str) -> Option<String> {
    let line = text.lines().find(|l| l.starts_with(key))?;
    Some(line[key.len()..].trim().trim_matches('"').to_owned())
}

/// A file's contents, trimmed, with the NUL a device-tree string carries.
fn read(path: impl AsRef<Path>) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim_end_matches('\0').trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn sysctl(name: &str) -> Option<String> {
    run("sysctl", &["-n", name])
}

fn mb(kb: u64) -> String {
    format!("{} MB", kb / 1024)
}

fn gb(kb: u64) -> String {
    let g = kb as f64 / 1024.0 / 1024.0;
    if g < 10.0 {
        format!("{g:.1} GB")
    } else {
        format!("{g:.0} GB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field that is not there is left out, not shown as a zero. That is what
    /// lets one About page serve a Mac and a Flip.
    #[test]
    fn absent_fields_are_dropped_rather_than_zeroed() {
        let facts = facts();
        for (label, value) in &facts {
            assert!(!value.trim().is_empty(), "{label} came back blank");
        }
        // Cores and uptime-or-storage work everywhere this builds.
        assert!(
            facts.iter().any(|(l, _)| *l == "Cores"),
            "no field at all came back: {facts:?}"
        );
    }

    #[test]
    fn a_key_is_read_out_of_a_file_of_them() {
        let text = "NAME=\"Batocera\"\nPRETTY_NAME=\"KNULLI 42\"\n";
        assert_eq!(field(text, "PRETTY_NAME="), Some("KNULLI 42".to_owned()));
        assert_eq!(field(text, "VERSION="), None);
    }

    /// Memory is reported in megabytes and storage in gigabytes, because those
    /// are the units the numbers are legible in on a handheld.
    #[test]
    fn sizes_read_as_sizes() {
        assert_eq!(mb(1024 * 1024), "1024 MB");
        assert_eq!(gb(1024 * 1024 * 8), "8.0 GB");
        assert_eq!(gb(1024 * 1024 * 64), "64 GB");
    }
}

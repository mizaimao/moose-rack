// Joining a network, through the device's own helper.
//
// Every call here is a process, and two of them are slow: a scan sleeps for a
// second by design, and joining waits up to twenty for an address. Neither can
// happen on the draw loop, so both are run on a thread and the screen shows
// what it last heard.
//
// The interface is `knulli-wifi`, read off the device rather than guessed:
//
//     knulli-wifi scanlist        scan, then list — one SSID per line
//     knulli-wifi list            the same without scanning
//     knulli-wifi enable SSID KEY save it, reload connman, wait for an address
//     knulli-wifi get_route       the gateway, so "did it work" has an answer
//
// The saved network is `knulli-settings-get wifi.ssid`.

use std::sync::mpsc;

/// What the Wi-Fi screen is doing.
pub enum State {
    /// Nothing yet — a scan has not been asked for.
    Idle,
    /// A scan or a join is running on a thread.
    Busy(&'static str),
    /// Networks in range, and which one is saved.
    Networks {
        names: Vec<String>,
        joined: Option<String>,
    },
    /// It did not work, and why as far as the helper said.
    Failed(String),
}

impl State {
    /// The list, when there is one.
    pub fn names(&self) -> &[String] {
        match self {
            State::Networks { names, .. } => names,
            _ => &[],
        }
    }

    /// A line for the top of the screen.
    pub fn note(&self) -> String {
        match self {
            State::Idle => "Press A to scan.".to_owned(),
            State::Busy(what) => format!("{what}…"),
            State::Networks { names, joined } => match joined {
                Some(s) => format!("{} in range · saved: {s}", names.len()),
                None => format!("{} in range", names.len()),
            },
            State::Failed(why) => why.clone(),
        }
    }
}

/// A scan or a join, running off the draw loop.
pub struct Job(mpsc::Receiver<State>);

impl Job {
    /// Whatever the job last reported, if it has finished.
    pub fn poll(&self) -> Option<State> {
        self.0.try_recv().ok()
    }
}

/// Scan for networks. Takes a second or so.
pub fn scan() -> Job {
    run(
        |helper, get| {
            let names = lines(&output(helper, &["scanlist"])?);
            if names.is_empty() {
                return Some(State::Failed("No networks found.".into()));
            }
            let joined = get
                .and_then(|g| output(g, &["wifi.ssid"]))
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            Some(State::Networks { names, joined })
        },
        "Scanning",
    )
}

/// Join `ssid` with `key`, then scan again so the screen shows the result.
///
/// The helper waits for an address itself and reports failure by exit status,
/// so there is no need to poll for a connection here.
pub fn join(ssid: String, key: String) -> Job {
    run(
        move |helper, get| {
            if output(helper, &["enable", &ssid, &key]).is_none() {
                return Some(State::Failed(format!("Could not join {ssid}.")));
            }
            let names = lines(&output(helper, &["list"]).unwrap_or_default());
            let joined = get
                .and_then(|g| output(g, &["wifi.ssid"]))
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            Some(State::Networks { names, joined })
        },
        "Joining",
    )
}

/// Run `body` on a thread with this device's helper names, if it has any.
fn run(
    body: impl FnOnce(&str, Option<&str>) -> Option<State> + Send + 'static,
    doing: &'static str,
) -> Job {
    let (tx, rx) = mpsc::channel();
    let wifi = moose_rack::platform::current().wifi();
    let Some(helper) = wifi.as_ref().and_then(|w| w.helper) else {
        // No radio, or a build that has no idea how to reach one. Said plainly
        // rather than spinning on a scan that will never come back.
        let _ = tx.send(State::Failed("This device has no Wi-Fi controls.".into()));
        return Job(rx);
    };
    let get = wifi.as_ref().and_then(|w| w.settings_get);
    std::thread::spawn(move || {
        let out = body(helper, get).unwrap_or_else(|| State::Failed(format!("{doing} failed.")));
        let _ = tx.send(out);
    });
    Job(rx)
}

/// Run a command and take its output, or `None` if it failed.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One network per line, blanks dropped.
///
/// Split out so it can be tested against the device's real output without a
/// radio in the machine running the tests.
fn lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real output from `knulli-wifi list` on the Flip, 2026-08-25.
    ///
    /// Plain SSIDs, one per line — no columns, no signal, and names with spaces
    /// and hyphens in them, which is why the list is taken whole rather than
    /// split on whitespace.
    const REAL: &str = "Chicken24\nChicken50\nDIRECT-AB-HP OfficeJet Pro 6970\n\
                        HHK9G_2GEXT\nHawaii12\nVerizon_NQ9X36_2.4GHz\nXfinity Mobile\n\
                        xfinitywifi\n";

    #[test]
    fn the_real_output_parses_whole_names() {
        let got = lines(REAL);
        assert_eq!(got.len(), 8);
        assert_eq!(got[0], "Chicken24");
        assert_eq!(
            got[2], "DIRECT-AB-HP OfficeJet Pro 6970",
            "a name with spaces was split"
        );
        assert_eq!(got[6], "Xfinity Mobile");
    }

    /// Blank lines and trailing whitespace are not networks.
    #[test]
    fn blank_lines_are_not_networks() {
        assert!(lines("").is_empty());
        assert!(lines("\n\n  \n").is_empty());
        assert_eq!(lines("  Chicken24  \n\n"), ["Chicken24"]);
    }

    /// A build with no radio says so instead of pretending to scan.
    #[test]
    fn a_machine_with_no_radio_says_so() {
        // The tests run on a Mac, whose scheme offers no Wi-Fi.
        let job = scan();
        let state = job.poll().expect("an immediate answer");
        match state {
            State::Failed(why) => assert!(why.contains("no Wi-Fi")),
            _ => panic!("expected a plain refusal"),
        }
    }

    /// The heading says what is happening, in each state.
    #[test]
    fn the_heading_says_what_is_going_on() {
        assert!(State::Idle.note().contains("scan"));
        assert_eq!(State::Busy("Scanning").note(), "Scanning…");
        let joined = State::Networks {
            names: vec!["Chicken24".into(), "Hawaii12".into()],
            joined: Some("Chicken24".into()),
        };
        assert_eq!(joined.note(), "2 in range · saved: Chicken24");
        let anon = State::Networks {
            names: vec!["a".into()],
            joined: None,
        };
        assert_eq!(anon.note(), "1 in range");
    }
}

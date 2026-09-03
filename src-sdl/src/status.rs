// The clock, the signal and the battery, for the corner of the screen.
//
// Everything here is a file read, so it is cheap — but it is not *free*, and
// the loop runs at the display's refresh rate. Reading four sysfs files sixty
// times a second to draw a number that changes every few minutes is the kind of
// thing that shows up as battery drain and nothing else, so it is polled on a
// timer and cached in between.
//
// Which files those are is the platform's business, not this module's: see
// `moose_rack::platform`. On a Mac none of them exist and every field comes
// back `None`, which is why the preview needs `MOOSE_SDL_STATUS=fake` to show
// what the handheld will.

use moose_rack::platform;

/// How often to look. Battery moves in percent and Wi-Fi in bars; neither is
/// worth a read per frame.
const EVERY_MS: u64 = 2_000;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Status {
    /// `HH:MM`, or `None` when the clock cannot be read.
    pub clock: Option<String>,
    /// Signal as 0..=4 bars, or `None` when there is no wireless at all.
    pub bars: Option<u8>,
    /// Charge as a percentage.
    pub battery: Option<u8>,
    /// Whether something is plugged in.
    pub charging: bool,
    next_read: u64,
}

impl Status {
    /// Refresh if it is time to. `now` is the loop's millisecond clock.
    pub fn poll(&mut self, now: u64) {
        if now < self.next_read {
            return;
        }
        self.next_read = now + EVERY_MS;
        self.clock = moose_rack::util::local_hhmm();

        if std::env::var("MOOSE_SDL_STATUS").as_deref() == Ok("fake") {
            // The Mac has no battery node and no wlan0. Without this the
            // preview shows a clock and two gaps, and the corner cannot be
            // judged until it is on the device.
            self.bars = Some(3);
            self.battery = Some(72);
            self.charging = true;
            return;
        }

        let p = platform::current();
        self.bars = p.wifi().and_then(|w| bars_from_proc(&w));
        if let Some(b) = p.battery() {
            // The device's own helper first, and the file only if there is
            // none.
            //
            // Both usually read the same node, but "usually" is why the number
            // here and the number in KNULLI's own menu could disagree — the
            // helper knows about a percentage file this board may publish
            // instead, and about fuel-gauge nodes that need charge-now over
            // charge-full. Asking it is how the two agree by construction
            // rather than by our copying its arithmetic and hoping.
            self.battery = b
                .helper
                .and_then(ask_helper)
                .or_else(|| read_number(&b.capacity).map(|n| n.min(100) as u8));
            self.charging = b.charging.iter().any(|p| read_number(p) == Some(1));
        }
    }

    /// What the corner shows, right to left. Empty when there is nothing to say.
    pub fn parts(&self) -> Vec<Part> {
        let mut out = Vec::new();
        if let Some(clock) = &self.clock {
            out.push(Part::Text(clock.clone()));
        }
        if let Some(bars) = self.bars {
            out.push(Part::Wifi(bars));
        }
        if let Some(percent) = self.battery {
            out.push(Part::Battery {
                percent,
                charging: self.charging,
            });
            // The number as well as the picture. A cell that is a quarter full
            // and one that is a third look the same at this size, and "is it
            // worth starting something" is a question about the number.
            out.push(Part::Text(format!("{percent}%")));
        }
        out
    }
}

/// One thing in the corner.
///
/// Signal is not text. It used to be four block characters — the bars a phone
/// shows for a cellular network — and Wi-Fi has been drawn as arcs spreading
/// from a point for as long as anyone has drawn it. Nothing in a font does
/// that at four strengths, so it is a picture, made here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    /// 0 to 4 arcs lit.
    Wifi(u8),
    /// Charge as a percentage, and whether something is plugged in.
    Battery { percent: u8, charging: bool },
}

/// How wide and tall the signal picture is drawn, in pixels.
///
/// Odd width so the arcs have a centre column to be symmetrical about. Small,
/// because the whole corner is: on a 640-point panel the clock beside it is ten
/// points tall.
pub const WIFI_SIZE: (u32, u32) = (18, 14);

/// How wide and tall the battery is drawn, in pixels. Wider than tall, like a
/// battery, with a nub on the end.
pub const BATTERY_SIZE: (u32, u32) = (22, 12);

/// The battery symbol: a case, a nub, and as much of it filled as there is
/// charge. A bolt across it when something is plugged in.
///
/// Drawn rather than typed for the same reason the signal is: no font has a
/// battery at a hundred levels of fill, and the fill is the whole point.
pub fn battery_pixels(percent: u8, charging: bool) -> Vec<u8> {
    let (w, h) = BATTERY_SIZE;
    let (w, h) = (w as i32, h as i32);
    // The case, leaving room for the nub on the right.
    let case_w = w - 3;
    let wall = 1;
    let inner_w = case_w - wall * 2 - 2;
    let filled = (inner_w as f32 * percent as f32 / 100.0).round() as i32;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut a: f32 = 0.0;
            // The case outline.
            let on_case = x < case_w
                && (x < wall || x >= case_w - wall || y < wall || y >= h - wall);
            if on_case {
                a = 1.0;
            }
            // The nub, a third of the height, centred.
            if x >= case_w && x < w - 1 && y >= h / 3 && y < h - h / 3 {
                a = 1.0;
            }
            // The charge itself, inset by one from the wall so it never
            // touches it — a fill that meets the outline reads as a solid
            // block rather than as a level.
            let fill_x0 = wall + 1;
            if x >= fill_x0 && x < fill_x0 + filled && y > wall && y < h - wall - 1 {
                a = 1.0;
            }
            out.extend_from_slice(&[255, 255, 255, (a.clamp(0.0, 1.0) * 255.0) as u8]);
        }
    }
    // No bolt drawn into the cell.
    //
    // The first version carved one out of the fill with two cleared columns,
    // which at twenty-two pixels wide cut the charge into pieces and read as a
    // broken icon rather than as a battery charging. The colour carries it
    // instead: green when plugged in, grey when not, which is the one fact
    // worth reading across a room and needs no detail at all.
    let _ = charging;
    out
}

/// The signal symbol, as pixels: arcs spreading up from a dot.
///
/// Drawn rather than typed. Each arc is a band of radius around the dot, cut to
/// a wedge either side of vertical, and softened across a pixel at every edge —
/// at this size a hard edge is a staircase and there are only ten rows to make
/// the shape out of.
///
/// Unlit arcs stay, faintly. An icon that loses its outline as the signal drops
/// reads as a different icon rather than as the same one saying less.
pub fn wifi_pixels(bars: u8) -> Vec<u8> {
    let (w, h) = WIFI_SIZE;
    let (cx, cy) = ((w as f32 - 1.0) / 2.0, h as f32 - 1.0);
    // The dot, then three bands. Each is (inner, outer) radius.
    let bands = [(0.0, 1.6), (3.0, 4.3), (5.4, 6.7), (7.8, 9.1)];
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            // Sampled at the pixel's middle, or everything sits half a pixel
            // up and to the left of where it was meant to.
            let (dx, dy) = (x as f32 + 0.5 - cx, cy - (y as f32 + 0.5));
            let r = (dx * dx + dy * dy).sqrt();
            let mut alpha: f32 = 0.0;
            for (i, (inner, outer)) in bands.iter().enumerate() {
                // The wedge: 45 degrees either side of straight up. The dot is
                // the whole circle, having no direction to point in.
                if i > 0 && (dy <= 0.0 || dx.abs() > dy) {
                    continue;
                }
                // Coverage across one pixel at each edge of the band.
                let lit = smooth(r, inner - 0.5, inner + 0.5) * (1.0 - smooth(r, outer - 0.5, outer + 0.5));
                // An arc beyond the strength is drawn faintly rather than not
                // at all.
                let strength = if i == 0 || i as u8 <= bars { 1.0 } else { 0.22 };
                alpha = alpha.max(lit * strength);
            }
            let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
            out.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    out
}

/// 0 below `from`, 1 above `to`, an S-curve between.
fn smooth(v: f32, from: f32, to: f32) -> f32 {
    if to <= from {
        return if v >= to { 1.0 } else { 0.0 };
    }
    let t = ((v - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ask the device's own battery helper, if it has one.
///
/// A process, so it is only run on the same two-second timer as everything else
/// here — sixty of these a second would be sixty shells a second.
fn ask_helper(helper: &str) -> Option<u8> {
    let out = std::process::Command::new(helper).output().ok()?;
    if !out.status.success() {
        return None;
    }
    // The helper prints a report, not a number:
    //
    // ```text
    // Battery Status Report:
    //   Capacity:     84%
    //   Status:       Discharging
    // ```
    //
    // Parsing it for `i64` therefore failed, quietly fell back to the sysfs
    // node, and showed 27% next to KNULLI's 84% — which is the whole reason
    // this exists. The node is wrong on this board: it reads 27 while
    // `voltage_now` is 3.908 V, which on a lithium cell is about half. The
    // device knows better and keeps the real figure in `/tmp/battery.percent`,
    // which is the first thing the helper looks at.
    let text = String::from_utf8_lossy(&out.stdout);
    percent_in(&text)
}

/// The first percentage in a line of text, whatever else is on it.
fn percent_in(text: &str) -> Option<u8> {
    for line in text.lines() {
        let Some(cut) = line.find('%') else { continue };
        let digits: String = line[..cut]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.chars().rev().collect::<String>().parse::<i64>() {
            return Some(n.clamp(0, 100) as u8);
        }
    }
    // A helper that does print a bare number is fine too.
    text.trim().parse::<i64>().ok().map(|n| n.clamp(0, 100) as u8)
}

/// Read a whole number out of a one-line sysfs file.
fn read_number(path: &std::path::Path) -> Option<i64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Pull the link quality for one interface out of `/proc/net/wireless`.
///
/// The file is two header lines then one row per interface:
///
/// ```text
///  wlan0: 0000   40.   48.    0.  ...
/// ```
///
/// The third column is the quality — 40 of a possible 70 on the Flip while
/// connected. It carries a trailing dot, which is why it is trimmed rather than
/// parsed directly.
fn bars_from_proc(w: &platform::Wifi) -> Option<u8> {
    let text = std::fs::read_to_string(&w.proc_wireless).ok()?;
    let quality = quality_for(&text, w.interface)?;
    let fraction = quality as f32 / w.max_quality.max(1) as f32;
    Some((fraction * 4.0).round().clamp(0.0, 4.0) as u8)
}

/// Split out from the file read so it can be tested against a real capture
/// without a wireless card in the machine running the tests.
fn quality_for(text: &str, interface: &str) -> Option<u32> {
    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix(interface) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        // status, then quality.
        let mut fields = rest.split_whitespace();
        let _status = fields.next()?;
        let quality = fields.next()?.trim_end_matches('.');
        return quality.parse().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real capture from the Flip, connected to Wi-Fi on 2026-08-25.
    const PROC: &str = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n \
                        face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n \
                        wlan0: 0000   40.   48.    0.       0      0      0      0      0        0\n";

    /// The quality column is the third field and carries a trailing dot, which
    /// is what stops it parsing if it is read as-is.
    #[test]
    fn the_quality_is_read_from_a_real_capture() {
        assert_eq!(quality_for(PROC, "wlan0"), Some(40));
    }

    /// An interface that is not in the file is no signal, not a panic and not a
    /// zero — zero bars means connected and weak, which is a different thing.
    #[test]
    fn an_absent_interface_reports_nothing() {
        assert_eq!(quality_for(PROC, "wlan1"), None);
        assert_eq!(quality_for("", "wlan0"), None);
        assert_eq!(quality_for("nonsense\n", "wlan0"), None);
    }

    /// The header lines mention neither interface and must not be mistaken for
    /// one — "face |" begins a line and ends in a colon further along.
    #[test]
    fn the_header_is_not_read_as_an_interface() {
        assert_eq!(quality_for(PROC, "face"), None);
    }

    /// 40 of 70 is a bit over half, which is three bars of four.
    #[test]
    fn quality_becomes_bars() {
        let w = platform::Wifi {
            proc_wireless: "/nonexistent".into(),
            interface: "wlan0",
            max_quality: 70,
            helper: None,
            settings_get: None,
        };
        assert_eq!(bars_from_proc(&w), None, "a missing file is no signal");
        // The arithmetic, without the file.
        let bars = |q: f32| (q / 70.0 * 4.0).round() as u8;
        assert_eq!(bars(40.0), 2);
        assert_eq!(bars(70.0), 4);
        assert_eq!(bars(0.0), 0);
    }

    /// Charging shows a bolt; not charging shows the number alone.
    #[test]
    fn the_corner_says_what_it_knows_and_no_more() {
        let mut s = Status {
            clock: Some("00:20".into()),
            ..Status::default()
        };
        let text = |p: &Part| match p {
            Part::Text(t) => t.clone(),
            Part::Wifi(n) => format!("wifi{n}"),
            Part::Battery { percent, .. } => format!("battery{percent}"),
        };
        assert_eq!(
            s.parts().iter().map(text).collect::<Vec<_>>(),
            ["00:20"],
            "no radio and no battery means no room taken"
        );

        s.battery = Some(87);
        s.charging = true;
        s.bars = Some(4);
        let parts = s.parts();
        assert_eq!(text(&parts[0]), "00:20");
        // Four: the clock, the signal, the battery, and the battery's number.
        // Both of the last two, because a cell a quarter full and one a third
        // full are the same picture at this size.
        assert_eq!(parts.len(), 4, "{parts:?}");
        // Signal and charge are pictures, not words — see `Part::Wifi` and
        // `Part::Battery`.
        assert_eq!(parts[1], Part::Wifi(4));
        assert_eq!(
            parts[2],
            Part::Battery {
                percent: 87,
                charging: true
            }
        );
        assert_eq!(text(&parts[3]), "87%");

        // Charging is the icon's colour, not a bolt in the text.
        s.charging = false;
        assert_eq!(
            s.parts()[2],
            Part::Battery {
                percent: 87,
                charging: false
            }
        );
        assert_eq!(text(&s.parts()[3]), "87%");
    }

    /// Polling is on a timer. Sixty reads a second of four sysfs files, for a
    /// number that moves in percent, is battery spent to say so.
    #[test]
    fn it_does_not_read_the_files_every_frame() {
        let mut s = Status::default();
        s.poll(0);
        let first = s.next_read;
        assert!(first >= EVERY_MS, "the next read was not pushed out");
        s.clock = Some("wrong".into());
        s.poll(first - 1);
        assert_eq!(s.clock.as_deref(), Some("wrong"), "it read again too early");
        s.poll(first);
        assert_ne!(s.clock.as_deref(), Some("wrong"), "it never read again");
    }
}

#[cfg(test)]
mod battery {
    use super::percent_in;

    /// KNULLI's helper prints a report, and the number is in the middle of it.
    ///
    /// Read as a bare integer it fails, falls back to the sysfs node, and shows
    /// 27% beside the device's own 84%. That node is wrong on this board — it
    /// reads 27 at 3.908 V, which is about half a lithium cell — so falling
    /// back to it silently was the worst of the three possible outcomes.
    #[test]
    fn the_percentage_is_found_in_the_report() {
        let report = "Battery Status Report:\n  Capacity:     84%\n  Status:     Discharging\n";
        assert_eq!(percent_in(report), Some(84));
        assert_eq!(percent_in("7%"), Some(7));
        assert_eq!(percent_in("100%"), Some(100));
        // A helper that prints a bare number still works.
        assert_eq!(percent_in("55\n"), Some(55));
        // Nonsense is nothing, not a zero — zero is a flat battery.
        assert_eq!(percent_in("no battery here"), None);
        assert_eq!(percent_in(""), None);
        // Out of range is clamped rather than believed.
        assert_eq!(percent_in("Capacity: 140%"), Some(100));
    }
}

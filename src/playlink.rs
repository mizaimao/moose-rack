//! `moose-rack://play/<platform>/<file>`: the web page's Play button for a game
//! the browser can't run. The OS hands the address to the desktop app on the
//! same computer, which starts the game the way its own Play button would.
//! See `docs/server-and-client.md`.
//!
//! A game is named by platform and file name, not by the service's row id:
//! row ids are scan order and change when the library does.
//!
//! Any page on the web can fire one of these, so this parser is strict and the
//! caller only ever looks the result up in its own library. Nothing in a link
//! is run, opened or written anywhere.

/// The scheme registered with the OS, and the only one accepted.
pub const SCHEME: &str = "moose-rack";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PlayLink {
    pub platform: String,
    pub file: String,
}

/// The link for one game, percent-encoded the way the web page builds it.
pub fn build(platform: &str, file: &str) -> String {
    format!("{SCHEME}://play/{}/{}", encode(platform), encode(file))
}

/// Read a link, or `None` for anything that isn't exactly a play link.
pub fn parse(url: &str) -> Option<PlayLink> {
    let (scheme, rest) = url.trim().split_once("://")?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    // Some launchers add a trailing slash, and a query or fragment means
    // nothing here.
    let rest = rest.split(['?', '#']).next()?.trim_end_matches('/');
    let (action, rest) = rest.split_once('/')?;
    if !action.eq_ignore_ascii_case("play") {
        return None;
    }
    let (platform, file) = rest.split_once('/')?;
    let platform = decode(platform)?;
    let file = decode(file)?;
    let platform_ok = !platform.is_empty()
        && platform.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    // One file name, never a path: no separators, no `..`, no control bytes.
    let file_ok = !file.is_empty()
        && file != "."
        && file != ".."
        && !file.contains(['/', '\\'])
        && !file.chars().any(char::is_control);
    (platform_ok && file_ok).then_some(PlayLink { platform, file })
}

/// `encodeURIComponent`'s set: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )`.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(platform: &str, file: &str) -> Option<PlayLink> {
        Some(PlayLink { platform: platform.into(), file: file.into() })
    }

    #[test]
    fn a_built_link_reads_back() {
        for (p, f) in [
            ("xbox360", "Halo 3 (USA).iso"),
            ("ps2", "Ratchet & Clank - Going Commando (USA) (v2.00).chd"),
            ("snes", "Pokémon Stadium 100% (Japan) [T-En].sfc"),
            ("psx", "Final Fantasy VII (USA) (Disc 1).m3u"),
        ] {
            assert_eq!(parse(&build(p, f)), link(p, f), "{p}/{f}");
        }
    }

    #[test]
    fn matches_what_the_web_page_sends() {
        // encodeURIComponent("Ratchet & Clank (USA).chd")
        assert_eq!(
            parse("moose-rack://play/ps2/Ratchet%20%26%20Clank%20(USA).chd"),
            link("ps2", "Ratchet & Clank (USA).chd")
        );
        assert_eq!(build("ps2", "Ratchet & Clank (USA).chd"), "moose-rack://play/ps2/Ratchet%20%26%20Clank%20(USA).chd");
    }

    #[test]
    fn tolerates_what_launchers_add() {
        assert_eq!(parse("MOOSE-RACK://Play/gba/Golden%20Sun.gba/"), link("gba", "Golden Sun.gba"));
        assert_eq!(parse("moose-rack://play/gba/Golden%20Sun.gba?x=1#y"), link("gba", "Golden Sun.gba"));
        assert_eq!(parse("  moose-rack://play/gba/a.gba\n"), link("gba", "a.gba"));
    }

    #[test]
    fn refuses_anything_else() {
        for url in [
            "",
            "moose-rack://",
            "moose-rack://play",
            "moose-rack://play/gba",
            "moose-rack://play/gba/",
            "moose-rack://play//a.gba",
            "moose-rack://open/gba/a.gba",
            "https://play/gba/a.gba",
            "steam://play/gba/a.gba",
            "moose-rack://play/gba/sub/a.gba",
            "moose-rack://play/gba/..%2F..%2Fetc%2Fpasswd",
            "moose-rack://play/gba/a%2Fb.gba",
            "moose-rack://play/gba/a%5Cb.gba",
            "moose-rack://play/gba/..",
            "moose-rack://play/gba/a%0A.gba",
            "moose-rack://play/gba/a%ZZ.gba",
            "moose-rack://play/gba/a%E9.gba",
            "moose-rack://play/g%20ba/a.gba",
            "moose-rack://play/../a.gba",
        ] {
            assert_eq!(parse(url), None, "{url:?}");
        }
    }
}

//! Check `config.toml` against what the app reads now, and offer to update it.
//!
//! Every setting this app has ever renamed, retyped or removed still loads:
//! `[cheevos]` is read as `[achievements]`, a boolean `autofire` is read as the
//! shoulder it used to mean, `retroarch.root` stands in for `installs`. That
//! leniency is deliberate — a config written a year ago should not stop the app
//! starting — but it has a cost nobody sees. The file goes on saying
//! `password = "..."` long after passwords stopped being used, and the person
//! reading it has no way to tell which lines still do something.
//!
//! So this reports, in the file's own terms, which lines are stale and what
//! each one means now. Nothing is rewritten without being asked.
//!
//! ## Why detection is by content, not by a version number
//!
//! There has never been a version field, so every config in existence predates
//! one and would have to be assumed ancient. Reading the file for the shapes
//! that actually changed works on any config, including one edited by hand into
//! a state no released version ever wrote. A `version` line is *written* after
//! patching, which lets a later run skip the scan and gives the file a plain
//! statement of what it was last brought up to.
//!
//! ## The backup holds what was removed
//!
//! `config.toml.before-patch` is written before anything changes, and on an old
//! config that means a copy of the plain-text password this run just deleted.
//! `.gitignore` covers `/config.toml.*` for exactly that reason — a backup like
//! that has been committed to this repo once already. Delete it once the patch
//! looks right.
//!
//! ## What it will not do
//!
//! Guess. A finding is only offered as fixable when the new value follows from
//! the old one with no judgement: `password` had exactly one correct fate
//! (deletion), and a boolean `autofire` had exactly one meaning. Anything
//! needing a decision — which of a set's looks you want, which core a game
//! should use — is reported and left alone.

use std::fmt;

/// The shape `config.toml` is written in now. Bumped when a change lands that
/// this module knows how to make.
pub const CURRENT_VERSION: u32 = 1;

/// How bad a finding is, and therefore how it reads on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The line does nothing. Harmless, but misleading to read.
    Dead,
    /// The line still works through a compatibility path that will not last.
    Stale,
    /// The line is read and its value is wrong or unsafe.
    Wrong,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Dead => "dead",
            Severity::Stale => "stale",
            Severity::Wrong => "wrong",
        })
    }
}

/// What to do about a finding, when it can be done without guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Delete the line.
    Remove { table: String, key: String },
    /// Move a value to a different key, keeping it.
    Rename { table: String, from: String, to: String },
    /// Replace the value, keeping the key.
    SetValue { table: String, key: String, value: String },
    /// Rename a whole `[section]`.
    RenameTable { from: String, to: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    /// `achievements.password`, or `[cheevos]` for a whole section.
    pub what: String,
    /// One sentence a person can act on, in plain words.
    pub note: String,
    /// Present when the change follows from the old value with no judgement.
    pub fix: Option<Fix>,
}

impl Finding {
    fn new(severity: Severity, what: &str, note: &str, fix: Option<Fix>) -> Self {
        Self { severity, what: what.to_owned(), note: note.to_owned(), fix }
    }
}

/// The version a config says it was last brought up to, or 0 for one that has
/// never been stamped — which is every config written before this existed.
pub fn version_of(toml: &str) -> u32 {
    toml::from_str::<toml::Value>(toml)
        .ok()
        .and_then(|v| v.get("config_version").and_then(|v| v.as_integer()))
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

/// Everything about this config that no longer says what it used to.
///
/// Pure: takes the text, returns findings, touches nothing. That is what makes
/// it testable against configs from versions nobody has on disk any more.
pub fn inspect(toml: &str) -> Vec<Finding> {
    let Ok(doc) = toml::from_str::<toml::Value>(toml) else {
        return vec![Finding::new(
            Severity::Wrong,
            "the file",
            "This is not valid TOML, so none of it is being read — the app is \
             running entirely on defaults.",
            None,
        )];
    };
    let mut out = Vec::new();
    let get = |table: &str, key: &str| doc.get(table).and_then(|t| t.get(key));

    // ---- Credentials that are no longer used -------------------------------
    // Kept first because it is the only finding with a security cost: the file
    // is plain text and the value is a live password.
    for section in ["achievements", "cheevos"] {
        if get(section, "password").is_some() {
            out.push(Finding::new(
                Severity::Wrong,
                &format!("{section}.password"),
                "RetroAchievements passwords are not used any more and this one is sitting in \
                 plain text. Only the token is sent. Delete the line, and consider changing the \
                 password if this file has ever been shared or backed up.",
                Some(Fix::Remove { table: section.into(), key: "password".into() }),
            ));
        }
    }

    // ---- Sections and keys that were renamed -------------------------------
    if doc.get("cheevos").is_some() && doc.get("achievements").is_none() {
        out.push(Finding::new(
            Severity::Stale,
            "[cheevos]",
            "Still read, but the section is called [achievements] now. \"cheevos\" is \
             RetroArch's own spelling for the feature.",
            Some(Fix::RenameTable { from: "cheevos".into(), to: "achievements".into() }),
        ));
    }
    if get("retroarch", "root").is_some() {
        let has_installs = doc
            .get("retroarch")
            .and_then(|t| t.get("installs"))
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        out.push(Finding::new(
            Severity::Stale,
            "retroarch.root",
            if has_installs {
                "A single install path, superseded by [[retroarch.installs]] — which this file \
                 also has, so the root is being ignored."
            } else {
                "A single install path. [[retroarch.installs]] replaced it and can list several, \
                 tried in order."
            },
            None,
        ));
    }

    // ---- Values whose meaning changed --------------------------------------
    if let Some(v) = get("retroarch", "autofire") {
        if v.as_bool().is_some() {
            out.push(Finding::new(
                Severity::Stale,
                "retroarch.autofire",
                "Rapid fire was a yes/no switch and is now which shoulder holds it: \"off\", \
                 \"lb\" or \"rb\".",
                Some(Fix::SetValue {
                    table: "retroarch".into(),
                    key: "autofire".into(),
                    value: if v.as_bool() == Some(true) { "lb".into() } else { "off".into() },
                }),
            ));
        } else if matches!(v.as_str(), Some("a" | "bottom" | "y" | "top")) {
            out.push(Finding::new(
                Severity::Stale,
                "retroarch.autofire",
                "A face button was tried as the rapid-fire modifier and did not work: holding it \
                 reports a real press instead of the repeat. It is a shoulder now.",
                Some(Fix::SetValue {
                    table: "retroarch".into(),
                    key: "autofire".into(),
                    value: "lb".into(),
                }),
            ));
        }
    }

    // ---- Artwork: the styles that stopped being styles ---------------------
    if let Some(style) = get("icons", "style").and_then(|v| v.as_str()) {
        let set = get("icons", "set").and_then(|v| v.as_str()).unwrap_or("");
        let known_look = crate::iconart::of(if set.is_empty() { crate::iconart::DEFAULT_SET } else { set })
            .is_some_and(|a| a.look(style).is_some());
        // A pool folder is equally valid and cannot be checked from the text
        // alone, so only the two names that were definitely retired are called
        // out — anything else unknown is left be rather than guessed at.
        if !known_look && matches!(style, "systemart_legacy" | "consolegame") {
            out.push(Finding::new(
                Severity::Stale,
                "icons.style",
                "This was one of five fixed artwork kinds. The grid draws whatever the chosen \
                 set offers now, so the name only still works if you have that folder \
                 downloaded.",
                None,
            ));
        }
    }

    // ---- Per-game core overrides that ship with the app --------------------
    if let Some(t) = doc
        .get("cores")
        .and_then(|c| c.get("per_game"))
        .and_then(|t| t.as_table())
    {
        let shipped = crate::config::arcade_core_map();
        let dupes: Vec<&String> = t
            .keys()
            .filter(|k| shipped.get(*k).map(|v| Some(v.as_str()) == t[*k].as_str()).unwrap_or(false))
            .collect();
        if !dupes.is_empty() {
            out.push(Finding::new(
                Severity::Dead,
                "cores.per_game",
                &format!(
                    "{} of these repeat what the app already ships in \
                     data/arcade-core-map.toml, so they change nothing.",
                    dupes.len()
                ),
                None,
            ));
        }
    }

    // ---- Sections nothing reads --------------------------------------------
    for section in ["scraper"] {
        if doc.get(section).is_some() {
            out.push(Finding::new(
                Severity::Dead,
                &format!("[{section}]"),
                "Nothing reads this yet. Kept so the credentials and the explanation live with \
                 the rest of the configuration.",
                None,
            ));
        }
    }

    // Worst first: a plain-text password should not be the fourth line read.
    out.sort_by_key(|f| std::cmp::Reverse(f.severity));
    out
}

/// The findings that can be applied without a decision.
pub fn fixable(findings: &[Finding]) -> Vec<&Finding> {
    findings.iter().filter(|f| f.fix.is_some()).collect()
}

/// Apply every fixable finding to the file's text, and stamp the version.
///
/// Line-based, like the rest of this project's config writing: `toml`'s
/// serializer drops every comment, and this file is two thirds comments that
/// somebody wrote on purpose. A rewrite that silently deleted them would be a
/// worse outcome than the stale keys it fixed.
pub fn patch(toml: &str) -> (String, Vec<Finding>) {
    let findings = inspect(toml);
    let mut text = toml.to_owned();
    let mut applied = Vec::new();

    for f in &findings {
        let Some(fix) = &f.fix else { continue };
        let next = match fix {
            Fix::Remove { table, key } => remove_key(&text, table, key),
            Fix::Rename { table, from, to } => rename_key(&text, table, from, to),
            Fix::SetValue { table, key, value } => set_value(&text, table, key, value),
            Fix::RenameTable { from, to } => rename_table(&text, from, to),
        };
        if let Some(next) = next {
            text = next;
            applied.push(f.clone());
        }
    }

    if !applied.is_empty() || version_of(&text) != CURRENT_VERSION {
        text = stamp_version(&text, CURRENT_VERSION);
    }
    (text, applied)
}

/// Lines of one `[table]`, as a half-open range into `lines`.
fn table_span(lines: &[&str], table: &str) -> Option<(usize, usize)> {
    let header = format!("[{table}]");
    let start = lines.iter().position(|l| l.trim() == header)? + 1;
    let end = lines[start..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|i| start + i)
        .unwrap_or(lines.len());
    Some((start, end))
}

fn key_line(line: &str, key: &str) -> bool {
    let t = line.trim_start();
    t.strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

fn remove_key(toml: &str, table: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = toml.lines().collect();
    let (start, end) = table_span(&lines, table)?;
    let at = (start..end).find(|i| key_line(lines[*i], key))?;
    let mut out: Vec<&str> = lines.clone();
    out.remove(at);
    Some(join(&out, toml))
}

fn rename_key(toml: &str, table: &str, from: &str, to: &str) -> Option<String> {
    let lines: Vec<&str> = toml.lines().collect();
    let (start, end) = table_span(&lines, table)?;
    let at = (start..end).find(|i| key_line(lines[*i], from))?;
    let replaced = lines[at].replacen(from, to, 1);
    let mut out: Vec<String> = lines.iter().map(|s| (*s).to_owned()).collect();
    out[at] = replaced;
    Some(join_owned(&out, toml))
}

fn set_value(toml: &str, table: &str, key: &str, value: &str) -> Option<String> {
    let lines: Vec<&str> = toml.lines().collect();
    let (start, end) = table_span(&lines, table)?;
    let at = (start..end).find(|i| key_line(lines[*i], key))?;
    let indent: String = lines[at].chars().take_while(|c| c.is_whitespace()).collect();
    let mut out: Vec<String> = lines.iter().map(|s| (*s).to_owned()).collect();
    out[at] = format!("{indent}{key} = \"{value}\"");
    Some(join_owned(&out, toml))
}

fn rename_table(toml: &str, from: &str, to: &str) -> Option<String> {
    let header = format!("[{from}]");
    let lines: Vec<&str> = toml.lines().collect();
    let at = lines.iter().position(|l| l.trim() == header)?;
    let mut out: Vec<String> = lines.iter().map(|s| (*s).to_owned()).collect();
    out[at] = format!("[{to}]");
    Some(join_owned(&out, toml))
}

/// Record what the file was brought up to, at the top so it is the first thing
/// read, and above any `[section]` so it lands in the root table.
fn stamp_version(toml: &str, version: u32) -> String {
    let line = format!("config_version = {version}");
    let mut out: Vec<String> = Vec::new();
    let mut written = false;
    for l in toml.lines() {
        if l.trim_start().starts_with("config_version") {
            if !written {
                out.push(line.clone());
                written = true;
            }
            continue;
        }
        if !written && l.trim_start().starts_with('[') {
            out.push(line.clone());
            out.push(String::new());
            written = true;
        }
        out.push(l.to_owned());
    }
    if !written {
        out.push(line);
    }
    join_owned(&out, toml)
}

fn join(lines: &[&str], original: &str) -> String {
    join_owned(&lines.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(), original)
}

fn join_owned(lines: &[String], original: &str) -> String {
    let mut s = lines.join("\n");
    if original.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything a config could carry from an older build, in one file.
    const OLD: &str = r#"# My config, written a while ago.
[server]
url = "http://dev.lan"
token = "rmm_x"

[retroarch]
# Rapid fire on.
autofire = true
root = "~/RetroArch"

[cheevos]
enabled = true
username = "frank"
token = "abc"
password = "hunter2"

[icons]
style = "consolegame"
"#;

    #[test]
    fn a_plain_text_password_is_the_first_thing_reported() {
        let f = inspect(OLD);
        assert_eq!(f[0].severity, Severity::Wrong);
        assert_eq!(f[0].what, "cheevos.password");
        assert!(f[0].note.contains("plain text"));
    }

    #[test]
    fn the_old_shapes_are_all_found() {
        let found = inspect(OLD);
        let what: Vec<&str> = found.iter().map(|f| f.what.as_str()).collect();
        for want in ["cheevos.password", "[cheevos]", "retroarch.root", "retroarch.autofire"] {
            assert!(what.contains(&want), "{want} not reported; got {what:?}");
        }
    }

    /// The whole point of line-based editing: this file is mostly comments
    /// somebody wrote on purpose, and a serializer would drop every one.
    #[test]
    fn patching_keeps_the_comments_and_everything_it_did_not_touch() {
        let (out, applied) = patch(OLD);
        assert!(out.contains("# My config, written a while ago."));
        assert!(out.contains("# Rapid fire on."));
        assert!(out.contains(r#"url = "http://dev.lan""#));
        assert!(out.contains(r#"token = "rmm_x""#));
        assert!(!applied.is_empty());
    }

    #[test]
    fn the_password_is_gone_and_the_rest_of_the_section_is_not() {
        let (out, _) = patch(OLD);
        assert!(!out.contains("hunter2"), "the password must be removed");
        assert!(out.contains(r#"username = "frank""#), "and only the password");
        assert!(out.contains(r#"token = "abc""#));
    }

    #[test]
    fn a_boolean_autofire_becomes_the_shoulder_it_meant() {
        let (out, _) = patch(OLD);
        assert!(out.contains(r#"autofire = "lb""#), "true meant on, and on is LB now\n{out}");
        assert!(!out.contains("autofire = true"));
    }

    #[test]
    fn a_renamed_section_keeps_its_contents() {
        let (out, _) = patch(OLD);
        assert!(out.contains("[achievements]"));
        assert!(!out.contains("[cheevos]"));
        let cfg: crate::config::Config = toml::from_str(&out).expect("still parses");
        assert_eq!(cfg.achievements.username.as_deref(), Some("frank"));
    }

    /// Patching twice must not change anything the second time, or the offer
    /// would come back on every launch forever.
    #[test]
    fn patching_is_idempotent() {
        let (once, first) = patch(OLD);
        let (twice, second) = patch(&once);
        assert!(!first.is_empty(), "the first pass has work to do");
        assert!(second.is_empty(), "the second has none: {second:?}");
        assert_eq!(once, twice);
    }

    #[test]
    fn a_patched_file_records_what_it_was_brought_up_to() {
        let (out, _) = patch(OLD);
        assert_eq!(version_of(&out), CURRENT_VERSION);
        // Above the first section, or it would land inside one.
        let v = out.find("config_version").unwrap();
        let first_section = out.find("[server]").unwrap();
        assert!(v < first_section, "the stamp must be in the root table");
    }

    /// A current config is left completely alone — no findings, no rewrite.
    #[test]
    fn a_current_config_has_nothing_to_say_about_it() {
        let current = format!(
            "config_version = {CURRENT_VERSION}\n\n\
             [server]\nurl = \"http://x\"\ntoken = \"t\"\n\n\
             [achievements]\nenabled = false\nusername = \"\"\ntoken = \"\"\n"
        );
        assert!(inspect(&current).is_empty(), "{:?}", inspect(&current));
        let (out, applied) = patch(&current);
        assert!(applied.is_empty());
        assert_eq!(out, current, "an up-to-date file must not be rewritten");
    }

    /// Broken TOML is the one case where the app is running on defaults
    /// entirely, and saying so is more useful than any individual finding.
    #[test]
    fn unparseable_toml_says_so_rather_than_listing_nothing() {
        let f = inspect("[server\nurl = ");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Wrong);
        assert!(f[0].note.contains("defaults"));
        assert!(f[0].fix.is_none(), "nothing can be safely rewritten in a file that does not parse");
    }

    /// Anything needing a decision is reported and left alone.
    #[test]
    fn findings_that_need_a_choice_are_not_offered_as_fixes() {
        let f = inspect(OLD);
        let root = f.iter().find(|f| f.what == "retroarch.root").unwrap();
        assert!(root.fix.is_none(), "which install to keep is the user's call");
        let style = f.iter().find(|f| f.what == "icons.style").unwrap();
        assert!(style.fix.is_none(), "which look to draw is the user's call");
    }

    /// A key that appears in two sections must only be touched in the one named.
    #[test]
    fn editing_one_section_does_not_reach_into_another() {
        let two = "[a]\ntoken = \"keep\"\n\n[b]\ntoken = \"drop\"\n";
        let out = remove_key(two, "b", "token").unwrap();
        assert!(out.contains("keep"));
        assert!(!out.contains("drop"));
    }

    /// A key whose name is a prefix of another must not be mistaken for it.
    #[test]
    fn a_key_is_matched_whole_not_by_prefix() {
        let t = "[retroarch]\nautofire_hz = 6\nautofire = true\n";
        let out = set_value(t, "retroarch", "autofire", "rb").unwrap();
        assert!(out.contains("autofire_hz = 6"), "the hz line must survive\n{out}");
        assert!(out.contains(r#"autofire = "rb""#));
    }
}

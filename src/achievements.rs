//! RetroAchievements, delivered through the launch config layer.
//!
//! Nothing here talks to retroachievements.org — RetroArch implements the whole
//! protocol. What this does is own the *configuration*, in this project's
//! `config.toml`, and write it into the per-launch layer.
//!
//! Self-contained on purpose. The credentials are not read from the user's own
//! `retroarch.cfg` and nothing here depends on how RetroArch is set up: the
//! same `config.toml` produces the same achievement behavior against any
//! RetroArch install, including a fresh one that has never been logged in. That
//! is the same principle as the rest of the launch layer — this app's settings
//! live in this app's config, and the user's emulator settings are left alone.
//!
//! `cheevos_username` is required alongside the credential. RetroArch can hold
//! a token with an empty username, in which case the token authenticates
//! nothing and achievements stay silently off — which is exactly the state this
//! machine's `retroarch.cfg` was in, and precisely why inheriting from it is
//! not something to rely on.
//!
//! ## Hardcore mode
//!
//! `cheevos_hardcore_mode_enable` disables save states, fast-forward and
//! rewind, because those are how you would cheat an achievement. Four of the
//! gamepad hotkeys this app ships are exactly those functions, so turning
//! hardcore on silently breaks Select+LB, Select+RB, Select+RT and the save
//! slot d-pad bindings.
//!
//! It is therefore written explicitly on every launch rather than inherited.
//! An install with `cheevos_hardcore_mode_enable = "true"` sitting in its own
//! config — which is RetroArch's default once you enable achievements — would
//! otherwise take those hotkeys away with no indication why.

/// What the user configured under `[achievements]`.
#[derive(Debug, Default, Clone)]
pub struct Settings {
    pub enabled: bool,
    /// RetroAchievements account name. Required alongside a credential.
    pub username: Option<String>,
    /// Login token, which is what RetroArch stores after a successful login and
    /// what it prefers on subsequent runs. Either this or `password`.
    pub token: Option<String>,
    /// Account password. RetroArch exchanges it for a token on first use, so a
    /// token is the better thing to keep here once you have one.
    /// Disables save states, fast-forward and rewind. Off unless asked for.
    pub hardcore: bool,
    /// Unofficial/test achievement sets.
    pub test_unofficial: bool,
}

fn present(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl Settings {
    /// The credential to send, preferring a token over a password.
    pub fn credential(&self) -> Option<(&'static str, &str)> {
        // Token only. A password in a launch config is a password written to
        // disk in plain text on every launch, and RetroArch keeps a token
        // after its first login anyway — so this app stores what RetroArch
        // would have stored and never handles the other thing at all.
        present(&self.token).map(|t| ("cheevos_token", t))
    }

    /// True when there is enough to actually authenticate.
    ///
    /// Both halves are required. A username with no credential cannot log in,
    /// and a credential with no username authenticates nothing — RetroArch
    /// sends the pair.
    pub fn usable(&self) -> bool {
        self.enabled && present(&self.username).is_some() && self.credential().is_some()
    }

    /// Why it is not usable, for the launch note.
    fn missing(&self) -> &'static str {
        match (present(&self.username).is_some(), self.credential().is_some()) {
            (false, false) => "no username or token",
            (false, true) => "no username",
            (true, false) => "no token or password",
            (true, true) => "",
        }
    }
}

/// RetroArch settings for the generated launch config.
///
/// Returns an empty string when achievements are off, so the user's own
/// `retroarch.cfg` is left to decide — this app not being configured for
/// achievements must not turn off achievements someone set up themselves.
pub fn config_lines(s: &Settings) -> String {
    if !s.enabled {
        return String::new();
    }
    if !s.usable() {
        // Enabled but unusable. Say so in the file rather than writing a
        // half-configured login that fails on every launch.
        return format!(
            "\n# ---- RetroAchievements ----\n\
             # [achievements] enabled = true, but there is {}. Left untouched rather\n\
             # than written half-configured, which fails a login every launch.\n",
            s.missing()
        );
    }

    let mut out = String::from(
        "\n# ---- RetroAchievements ----\n\
         # Configured entirely from this project's config.toml, so achievements\n\
         # behave the same against any RetroArch install and nothing depends on\n\
         # how the emulator itself was set up.\n\
         cheevos_enable = \"true\"\n",
    );
    if let Some(u) = present(&s.username) {
        out.push_str(&format!("cheevos_username = \"{}\"\n", escape(u)));
    }
    if let Some((key, value)) = s.credential() {
        out.push_str(&format!("{key} = \"{}\"\n", escape(value)));
    }

    // Always explicit — see the module docs on why inheriting this is a trap.
    out.push_str(&format!(
        "\n# Hardcore disables save states, fast-forward and rewind, which are\n\
         # four of the gamepad hotkeys this app binds. Written on every launch\n\
         # so an install that has it on cannot take them away unannounced.\n\
         cheevos_hardcore_mode_enable = \"{}\"\n",
        s.hardcore
    ));
    if s.test_unofficial {
        out.push_str("cheevos_test_unofficial = \"true\"\n");
    }
    out
}

/// A line for the launch output, so it is visible that achievements are on and
/// what that costs.
pub fn describe(s: &Settings) -> Option<String> {
    if !s.enabled {
        return None;
    }
    if !s.usable() {
        return Some(format!("achievements: enabled but {} — see [achievements]", s.missing()));
    }
    let who = present(&s.username).unwrap_or("");
    Some(if s.hardcore {
        format!("achievements: {who} (HARDCORE — save states and fast-forward are disabled)")
    } else {
        format!("achievements: {who}")
    })
}

/// RetroArch config values are double-quoted, so a quote or backslash in one
/// would end the value early.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(username: Option<&str>) -> Settings {
        Settings {
            enabled: true,
            username: username.map(str::to_owned),
            token: Some("tok".to_owned()),
            ..Default::default()
        }
    }

    /// Off means untouched. Someone who set achievements up in RetroArch and
    /// never configured this app must not have them switched off by it.
    #[test]
    fn disabled_writes_nothing_at_all() {
        assert_eq!(config_lines(&Settings::default()), "");
        assert!(describe(&Settings::default()).is_none());
    }

    /// The exact state this machine's retroarch.cfg was in: a valid token, no
    /// username. The token authenticates nothing without one, so writing a
    /// half-configured login would fail on every launch with no explanation.
    #[test]
    fn a_token_without_a_username_is_refused_and_explained() {
        let mut s = on(None);
        s.token = Some("abc123".to_owned());
        assert!(!s.usable());

        let out = config_lines(&s);
        assert!(!out.contains("cheevos_enable = \"true\""), "must not half-enable");
        assert!(!out.contains("abc123"), "and must not write the token: {out}");
        assert!(describe(&s).unwrap().contains("no username"));

        // Whitespace is not a username either.
        s.username = Some("   ".to_owned());
        assert!(!s.usable());
    }

    /// A username with no credential cannot log in, and must be refused just as
    /// clearly as the other way round.
    #[test]
    fn a_username_without_a_credential_is_refused() {
        let mut s = on(Some("frank"));
        s.token = None;
        assert!(!s.usable());
        assert!(describe(&s).unwrap().contains("no token or password"));
        assert!(!config_lines(&s).contains("cheevos_enable = \"true\""));
    }

    /// The whole point of the redesign: everything needed is written by us, so
    /// the same config.toml behaves identically against any RetroArch install,
    /// including one that has never been logged in.
    #[test]
    fn the_login_is_written_in_full_and_inherits_nothing() {
        let out = config_lines(&on(Some("player1")));
        assert!(out.contains("cheevos_enable = \"true\""));
        assert!(out.contains("cheevos_username = \"player1\""));
        assert!(out.contains("cheevos_token = \"tok\""), "the credential is ours: {out}");
    }

    /// A token, and nothing else, ever reaches the launch config.
    ///
    /// Passwords were supported and are gone: a password in a launch config is
    /// a password written to disk in plain text on every single launch, and
    /// RetroArch stores a token after its own first login anyway. Restore the
    /// `cheevos_password` branch and this fails.
    #[test]
    fn only_a_token_is_ever_written() {
        let mut s = on(Some("frank"));
        assert_eq!(s.credential(), Some(("cheevos_token", "tok")));

        // With no token there is no credential at all — not a fallback to
        // something weaker.
        s.token = None;
        assert_eq!(s.credential(), None);
        assert!(!s.usable(), "usable with no credential");
        let out = config_lines(&s);
        assert!(!out.contains("cheevos_password"), "a password path came back: {out}");
        assert!(!out.contains("cheevos_token ="), "a token was written from nothing: {out}");
    }

    /// Hardcore is written on every launch whether on or off, because
    /// inheriting it lets an install silently disable four shipped hotkeys.
    #[test]
    fn hardcore_is_always_stated_never_inherited() {
        let out = config_lines(&on(Some("frank")));
        assert!(
            out.contains("cheevos_hardcore_mode_enable = \"false\""),
            "off must be written explicitly, not left to the user's config: {out}"
        );

        let mut hard = on(Some("frank"));
        hard.hardcore = true;
        assert!(config_lines(&hard).contains("cheevos_hardcore_mode_enable = \"true\""));
        assert!(
            describe(&hard).unwrap().contains("HARDCORE"),
            "the cost has to be visible at launch"
        );
    }

    /// A quote in a username would end the value early and corrupt every
    /// setting after it in the generated file.
    #[test]
    fn quotes_in_a_username_cannot_break_the_config() {
        let mut s = on(Some("odd\"name"));
        s.token = Some("tok\\en".to_owned());
        let out = config_lines(&s);
        assert!(out.contains(r#"cheevos_username = "odd\"name""#), "{out}");
        assert!(out.contains(r#"cheevos_token = "tok\\en""#), "{out}");
    }

    /// Every emitted line must parse as a RetroArch assignment or a comment.
    #[test]
    fn every_emitted_line_is_a_setting_or_a_comment() {
        let mut s = on(Some("frank"));
        s.token = Some("abc".to_owned());
        s.test_unofficial = true;
        for line in config_lines(&s).lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, _) = line.split_once(" = ").unwrap_or_else(|| panic!("malformed: {line}"));
            assert!(
                key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "key {key:?} has characters RetroArch will not accept"
            );
        }
    }
}

/// The result of asking RetroAchievements whether a login works.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Verified {
    pub ok: bool,
    pub user: Option<String>,
    /// Why not, when `ok` is false.
    pub error: Option<String>,
}

/// Ask RetroAchievements whether this token still works.
///
/// A token rather than a password, deliberately. A password would have to be
/// held somewhere to be checked with — in a config file, or in a field on
/// screen — and neither is worth it for a status light. The token is already
/// stored because RetroArch needs it, so checking it costs no new secret.
///
/// `r=patch` is the endpoint RetroArch calls when a game starts: it takes a
/// username, a token and a game id, and answers 401 when the token is not
/// good. Any game id does — the answer to "is this token valid" does not
/// depend on which one, and game 1 is as permanent as they get.
///
/// This is the same credential and the same host RetroArch uses, so a tick
/// here means the login the emulator will attempt. The Web API at
/// `retroachievements.org/API/` takes a different credential entirely, and
/// checking that would prove the account exists while RetroArch's own login
/// stayed broken.
pub async fn verify(username: &str, token: &str) -> Verified {
    fn q(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    let url = format!(
        "https://retroachievements.org/dorequest.php?r=patch&u={}&t={}&g=1",
        q(username),
        q(token)
    );
    // Through `tls_roots` and after `install_tls`, like every other client in
    // this project. This one is https and reaches a public server, so on
    // Android it is exactly the request that aborts without them.
    crate::util::install_tls();
    let Ok(client) = crate::util::tls_roots(reqwest::Client::builder())
        .user_agent(concat!("moose-rack/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return Verified { ok: false, user: None, error: Some("could not build an HTTP client".into()) };
    };
    match client.get(&url).send().await {
        Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED => Verified {
            ok: false,
            user: None,
            error: Some("the server rejected this username and token".into()),
        },
        Ok(r) => {
            let ok = r
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("Success").and_then(serde_json::Value::as_bool))
                .unwrap_or(false);
            Verified {
                ok,
                user: ok.then(|| username.to_owned()),
                error: (!ok).then(|| "the server did not accept the login".to_owned()),
            }
        }
        Err(e) => Verified {
            ok: false,
            user: None,
            error: Some(format!("could not reach retroachievements.org: {e}")),
        },
    }
}

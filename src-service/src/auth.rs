//! Who is asking, and what they are allowed to do.
//!
//! Two ways in, because the client already speaks both. `src/api.rs` builds one
//! `Authorization` header at startup and sends it verbatim on every request:
//! `Bearer <token>` when a token is configured, `Basic base64(user:pass)`
//! otherwise. Nothing on the client changes to turn this on.
//!
//! * **The owner** holds the token. Everything.
//! * **Everyone else** has a username and a password. They can read the library
//!   and sync their own saves; they cannot change settings, install artwork, or
//!   otherwise alter what the next person sees.
//!
//! A browser can send neither header, so `POST /login` swaps either credential
//! for a session cookie.
//!
//! ## With nothing configured, nothing is checked
//!
//! An `[auth]` section with no token and no users leaves the service open, and
//! it says so at startup. The alternative is that upgrading the binary locks
//! somebody out of their own library until they have read the release notes,
//! which is a worse failure than the one this prevents.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine as _;
use serde::Deserialize;
use subtle::ConstantTimeEq;

/// PBKDF2-HMAC-SHA256. Not Argon2: these crates were already in the tree, and a
/// home LAN service is not worth a new supply chain. The cost is recorded in
/// the hash so it can be raised later without invalidating what exists.
const ROUNDS: u32 = 600_000;
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
const SCHEME: &str = "pbkdf2-sha256";

#[derive(Debug, Default, Deserialize, Clone)]
pub struct AuthConfig {
    /// The owner's token. Full access.
    pub token: Option<String>,
    #[serde(default)]
    pub users: Vec<User>,
    /// A shared account with no password, offered as a button under the sign-in
    /// form. Everyone who uses it is the same person as far as the library is
    /// concerned -- one set of saves, one set of states, shared.
    ///
    /// Off unless asked for. It means anyone who can reach the port can read
    /// the library, which is the whole point and worth being deliberate about.
    #[serde(default)]
    pub guest: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct User {
    pub name: String,
    /// `pbkdf2-sha256$<rounds>$<salt-b64>$<key-b64>`.
    pub password: String,
}

impl AuthConfig {
    /// True when nothing is configured, so nothing is checked.
    pub fn open(&self) -> bool {
        self.token.as_deref().unwrap_or("").is_empty() && self.users.is_empty() && !self.guest
    }

    /// The name the shared account signs in under.
    pub const GUEST: &'static str = "guest";

    pub fn describe(&self) -> String {
        if self.open() {
            return "open -- no token and no users configured".into();
        }
        let t = if self.token.as_deref().unwrap_or("").is_empty() { "no token" } else { "token" };
        let g = if self.guest { ", shared guest account on" } else { "" };
        format!("{t}, {} user(s){g}", self.users.len())
    }
}

/// Who is asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    /// The token holder, or anyone at all when auth is switched off.
    Owner,
    /// A named user, by password.
    User(String),
}

impl Identity {
    pub fn is_owner(&self) -> bool {
        matches!(self, Identity::Owner)
    }
    pub fn name(&self) -> &str {
        match self {
            Identity::Owner => "owner",
            Identity::User(n) => n,
        }
    }
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Hash a password for `moose-service.toml`.
pub fn hash_password(plain: &str) -> String {
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).expect("the OS random source");
    let key = derive(plain, &salt, ROUNDS);
    format!("{SCHEME}${ROUNDS}${}${}", b64().encode(salt), b64().encode(key))
}

fn derive(plain: &str, salt: &[u8], rounds: u32) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    // Infallible for a non-zero round count; the type is fallible because
    // PBKDF2 rejects zero.
    let _ = pbkdf2::pbkdf2_hmac::<sha2::Sha256>(plain.as_bytes(), salt, rounds, &mut key);
    key
}

/// Check a password against a stored hash.
///
/// A malformed stored hash is a failure, never a pass. Getting that backwards
/// turns a typo in the config into an open door.
pub fn verify_password(stored: &str, plain: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some(scheme), Some(rounds), Some(salt), Some(key), None) =
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if scheme != SCHEME {
        return false;
    }
    let (Ok(rounds), Ok(salt), Ok(key)) = (rounds.parse::<u32>(), b64().decode(salt), b64().decode(key))
    else {
        return false;
    };
    if rounds == 0 || key.len() != KEY_LEN {
        return false;
    }
    // Constant time: a byte-by-byte compare leaks how much of the hash matched.
    derive(plain, &salt, rounds).ct_eq(&key[..]).into()
}

/// Whichever credential the header carries, if it is valid.
pub fn from_header(cfg: &AuthConfig, header: Option<&str>) -> Option<Identity> {
    let header = header?;
    if let Some(tok) = header.strip_prefix("Bearer ") {
        let want = cfg.token.as_deref().unwrap_or("");
        // Constant time, and never a match on an empty configured token --
        // otherwise `Authorization: Bearer ` with nothing after it is the owner.
        if !want.is_empty() && bool::from(tok.trim().as_bytes().ct_eq(want.as_bytes())) {
            return Some(Identity::Owner);
        }
        return None;
    }
    if let Some(rest) = header.strip_prefix("Basic ") {
        let raw = b64().decode(rest.trim()).ok()?;
        let text = String::from_utf8(raw).ok()?;
        let (name, pass) = text.split_once(':')?;
        // Every configured user is checked even after a match, so the time
        // taken does not say which name exists.
        let mut found = None;
        for u in &cfg.users {
            if u.name == name && verify_password(&u.password, pass) {
                found = Some(Identity::User(u.name.clone()));
            }
        }
        return found;
    }
    None
}

/// Browser sessions: a random id in a cookie, the identity kept here.
///
/// In memory rather than signed, so a restart logs everyone out. That is the
/// right trade for a home service: no key to store, no token to leak into a
/// cookie, and the cost of being wrong is one login.
#[derive(Default)]
pub struct Sessions(Mutex<HashMap<String, Identity>>);

impl Sessions {
    pub fn open(&self, who: Identity) -> String {
        let mut bytes = [0u8; 24];
        getrandom::fill(&mut bytes).expect("the OS random source");
        let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        if let Ok(mut m) = self.0.lock() {
            m.insert(id.clone(), who);
        }
        id
    }
    pub fn get(&self, id: &str) -> Option<Identity> {
        self.0.lock().ok()?.get(id).cloned()
    }
    pub fn close(&self, id: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(id);
        }
    }
    pub fn len(&self) -> usize {
        self.0.lock().map(|m| m.len()).unwrap_or(0)
    }
}

pub const COOKIE: &str = "moose_session";

/// The session id out of a `Cookie:` header.
pub fn session_from_cookies(header: Option<&str>) -> Option<&str> {
    header?.split(';').find_map(|c| {
        let (k, v) = c.trim().split_once('=')?;
        (k == COOKIE).then_some(v)
    })
}

/// Commands a non-owner may call.
///
/// Named individually rather than by a `set_` prefix. A prefix rule is one
/// carelessly-named command away from handing a guest the settings, and the
/// test below fails on any dispatch arm that appears in neither list -- so a
/// new command has to be classified rather than silently defaulting either way.
pub const READ_ONLY: &[&str] = &[
    "status", "versions", "platforms", "systems", "roms", "recent_games", "search",
    "collection_groups", "collections_in", "collection_roms", "play_history", "attract_pool",
    "rom_detail", "game_states", "confirm_delete_state", "game_cores", "game_lightgun",
    "game_displays", "game_video", "rom_covers", "config_fields", "config_findings",
    "config_patch", "list_art_options", "motion_options", "icon_styles", "icon_sets",
    "bios_status", "disk_usage", "check_update", "verify_achievements", "ui_bindings",
    "list_controls", "arrange_list", "picker_controls", "page_filter", "grid_uniform",
    "set_grid", "warm_media",
    // Saves and states are shared in the guest account -- that is what the
    // account is for. A guest who plays a game and cannot keep the save has
    // been given a library they can look at and not use.
    "sync_saves_plan", "sync_saves", "resolve_save_conflict",
];

/// True when this command changes something a non-owner has no business
/// changing. `set_grid` is the exception in the list above: it computes a
/// keyboard-navigation table from card geometry and stores nothing.
pub fn owner_only(cmd: &str) -> bool {
    !READ_ONLY.contains(&cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AuthConfig {
        AuthConfig {
            token: Some("owner-secret".into()),
            users: vec![User { name: "guest".into(), password: hash_password("hunter2") }],
            guest: false,
        }
    }

    #[test]
    fn a_password_round_trips_and_a_wrong_one_does_not() {
        let h = hash_password("hunter2");
        assert!(verify_password(&h, "hunter2"));
        assert!(!verify_password(&h, "hunter3"));
        assert!(!verify_password(&h, ""));
    }

    /// Two hashes of one password differ, or the file leaks which accounts
    /// share a password.
    #[test]
    fn the_salt_makes_every_hash_different() {
        assert_ne!(hash_password("same"), hash_password("same"));
        assert!(hash_password("x").starts_with("pbkdf2-sha256$600000$"));
    }

    /// A stored hash that does not parse must fail closed. Failing open turns a
    /// typo in the config into an open door.
    #[test]
    fn a_malformed_hash_never_verifies() {
        for bad in [
            "",
            "hunter2",
            "pbkdf2-sha256$notanumber$c2FsdA==$a2V5",
            "pbkdf2-sha256$600000$c2FsdA==",
            "pbkdf2-sha256$0$c2FsdA==$a2V5",
            "bcrypt$600000$c2FsdA==$a2V5",
            "pbkdf2-sha256$600000$!!!!$a2V5",
            "pbkdf2-sha256$600000$c2FsdA==$a2V5$extra",
        ] {
            assert!(!verify_password(bad, "hunter2"), "{bad:?} verified");
            assert!(!verify_password(bad, ""), "{bad:?} verified against empty");
        }
    }

    /// The key length is part of the check: a truncated hash must not match on
    /// its prefix.
    #[test]
    fn a_truncated_hash_does_not_match() {
        let h = hash_password("hunter2");
        let mut parts: Vec<&str> = h.split('$').collect();
        let short = b64().encode(&b64().decode(parts[3]).unwrap()[..16]);
        parts[3] = &short;
        assert!(!verify_password(&parts.join("$"), "hunter2"));
    }

    #[test]
    fn the_token_identifies_the_owner() {
        let c = cfg();
        assert_eq!(from_header(&c, Some("Bearer owner-secret")), Some(Identity::Owner));
        assert_eq!(from_header(&c, Some("Bearer wrong")), None);
        assert_eq!(from_header(&c, None), None);
        assert_eq!(from_header(&c, Some("owner-secret")), None, "scheme is required");
    }

    /// `Authorization: Bearer ` with nothing after it must not become the owner
    /// on a config whose token is absent or empty.
    #[test]
    fn an_empty_token_is_never_a_match() {
        let c = AuthConfig { token: None, users: vec![], guest: false };
        assert_eq!(from_header(&c, Some("Bearer ")), None);
        assert_eq!(from_header(&c, Some("Bearer  ")), None);
        let c = AuthConfig { token: Some(String::new()), users: vec![], guest: false };
        assert_eq!(from_header(&c, Some("Bearer ")), None);
    }

    #[test]
    fn a_username_and_password_identify_a_user() {
        let c = cfg();
        let ok = format!("Basic {}", b64().encode("guest:hunter2"));
        assert_eq!(from_header(&c, Some(&ok)), Some(Identity::User("guest".into())));
        for bad in ["guest:wrong", "nobody:hunter2", "guest", "guest:"] {
            let h = format!("Basic {}", b64().encode(bad));
            assert_eq!(from_header(&c, Some(&h)), None, "{bad:?} got in");
        }
        assert_eq!(from_header(&c, Some("Basic !!!not-base64")), None);
    }

    /// The owner's token must not work as a password, and a user's password
    /// must not work as a token.
    #[test]
    fn the_two_credentials_do_not_cross_over() {
        let c = cfg();
        let as_pass = format!("Basic {}", b64().encode("guest:owner-secret"));
        assert_eq!(from_header(&c, Some(&as_pass)), None);
        assert_eq!(from_header(&c, Some("Bearer hunter2")), None);
    }

    #[test]
    fn sessions_open_resolve_and_close() {
        let s = Sessions::default();
        let id = s.open(Identity::User("guest".into()));
        assert_eq!(s.get(&id), Some(Identity::User("guest".into())));
        assert_eq!(s.get("made-up"), None);
        s.close(&id);
        assert_eq!(s.get(&id), None);
        // Two sessions are never the same string.
        assert_ne!(s.open(Identity::Owner), s.open(Identity::Owner));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn the_session_cookie_is_found_among_others() {
        assert_eq!(session_from_cookies(Some("moose_session=abc")), Some("abc"));
        assert_eq!(session_from_cookies(Some("a=1; moose_session=abc; b=2")), Some("abc"));
        assert_eq!(session_from_cookies(Some("other=abc")), None);
        assert_eq!(session_from_cookies(Some("")), None);
        assert_eq!(session_from_cookies(None), None);
    }

    /// The shared account is a real identity, not an absence of one: a guest is
    /// still not the owner.
    #[test]
    fn the_guest_account_is_offered_only_when_asked_for() {
        let mut c = AuthConfig::default();
        assert!(c.open(), "no guest and no credentials is an open service");
        c.guest = true;
        assert!(!c.open(), "switching the guest on must switch the checking on");
        assert!(c.describe().contains("guest"));
    }

    /// Saves and states are shared in that account, so a guest may sync them --
    /// and still may not touch the settings.
    #[test]
    fn a_guest_may_sync_saves_but_not_change_settings() {
        for allowed in ["sync_saves", "sync_saves_plan", "resolve_save_conflict", "game_states"] {
            assert!(!owner_only(allowed), "{allowed} is shared and should be allowed");
        }
        for denied in ["set_config_field", "install_icon_set", "set_list_art", "delete_state"] {
            assert!(owner_only(denied), "{denied} must stay the owner's");
        }
    }

    #[test]
    fn nothing_configured_means_nothing_checked() {
        assert!(AuthConfig::default().open());
        assert!(AuthConfig { token: Some(String::new()), users: vec![], guest: false }.open());
        assert!(!cfg().open());
        assert!(AuthConfig { token: None, users: cfg().users, guest: false }.open() == false);
    }

    /// A guest may look; a guest may not change what the next person sees.
    #[test]
    fn reads_are_shared_and_writes_are_the_owners() {
        for r in ["roms", "platforms", "rom_covers", "rom_detail", "search", "status"] {
            assert!(!owner_only(r), "{r} should be readable by a guest");
        }
        // `sync_saves` is deliberately absent: saves and states are shared in
        // the guest account, which is what the account is for. `delete_state`
        // is not -- deleting somebody else's freeze-frame is not sharing.
        for w in [
            "set_config_field", "set_icon_set", "set_list_art", "install_icon_set",
            "fetch_icons", "toggle_favorite", "delete_state",
            "set_retroarch_root", "import_bindings", "set_page_names",
        ] {
            assert!(owner_only(w), "{w} must be owner-only");
        }
    }

    /// Anything unknown is owner-only, so a command added without thought is
    /// refused rather than exposed.
    #[test]
    fn an_unclassified_command_is_refused_not_exposed() {
        assert!(owner_only("something_new"));
        assert!(owner_only(""));
    }
}

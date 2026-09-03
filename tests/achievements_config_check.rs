
//! Reads the developer's real config.toml and reports what the app makes of
//! the achievements section. Ignored by default: it depends on a file that
//! is not in the repository.
#[test]
#[ignore]
fn the_local_config_is_usable_for_achievements() {
    let cfg = moose_rack::config::Config::load().expect("config.toml");
    let s = cfg.achievements.settings();
    assert!(s.usable(), "the app does not consider these credentials usable");
    assert!(moose_rack::achievements::config_lines(&s).contains("cheevos_token"),
            "the launch config is not using the token");
    assert!(!moose_rack::achievements::config_lines(&s).contains("cheevos_password"),
            "a password is still being written into the launch config");
}

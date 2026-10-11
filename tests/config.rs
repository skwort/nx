use nx::{Config, NtfyMode};
use std::path::Path;
use std::time::Duration;

#[test]
fn loads_system_and_daemon_defaults() {
    let config = Config::load(Path::new("tests/fixtures/config.toml")).unwrap();

    assert_eq!(config.system.flake.unwrap(), Path::new("/etc/nixos"));
    assert_eq!(config.system.configuration.as_deref(), Some("test-system"));
    assert_eq!(
        config.daemon.interval().unwrap(),
        Some(Duration::from_secs(6 * 60 * 60))
    );
    assert_eq!(
        config.daemon.startup_delay().unwrap(),
        Duration::from_secs(60)
    );
    assert_eq!(
        config.daemon.socket.unwrap(),
        Path::new("/tmp/nx-test.sock")
    );
    assert!(config.notifications.enabled);
    assert_eq!(
        config.notifications.view_command.unwrap(),
        ["kitty", "--hold", "nx", "update", "list"]
    );
}

#[test]
fn missing_config_uses_empty_defaults() {
    let config = Config::load(Path::new("tests/fixtures/does-not-exist.toml")).unwrap();

    assert!(config.system.flake.is_none());
    assert!(config.system.configuration.is_none());
    assert!(config.daemon.socket.is_none());
    assert!(config.daemon.interval().unwrap().is_none());
    assert_eq!(
        config.daemon.startup_delay().unwrap(),
        Duration::from_secs(60)
    );
    assert!(config.notifications.enabled);
    assert!(!config.notifications.ntfy.enabled);
}

#[test]
fn loads_ntfy_and_rejects_conflicting_token_sources() {
    let config: Config = toml::from_str(
        r#"
        [notifications]
        enabled = false
        [notifications.ntfy]
        enabled = true
        topic_url = "https://ntfy.example.com/updates"
        mode = "detailed"
        token_file = "/run/secrets/ntfy-token"
    "#,
    )
    .unwrap();
    assert!(!config.notifications.enabled);
    assert!(matches!(config.notifications.ntfy.mode, NtfyMode::Detailed));
    config.notifications.ntfy.validate().unwrap();

    let mut ntfy = config.notifications.ntfy;
    ntfy.token_env = Some("NX_NTFY_TOKEN".to_owned());
    assert!(ntfy.validate().is_err());
    ntfy.token_file = None;
    ntfy.topic_url = Some("file:///tmp/topic".to_owned());
    assert!(ntfy.validate().is_err());
}

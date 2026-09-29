use workspacer_hub::{Options, backend::configure_local_integrations};

#[test]
fn embedded_external_integrations_keep_host_identity_across_restarts() {
    let directory = tempfile::tempdir().unwrap();
    let mut first = Options::default();
    first.config_dir = Some(directory.path().into());
    configure_local_integrations(&mut first).unwrap();
    assert_eq!(first.token.len(), 32);
    assert!(first.listen.unwrap().ip().is_loopback());
    assert!(first.mcp_listen.unwrap().ip().is_loopback());
    let mut second = Options::default();
    second.config_dir = first.config_dir.clone();
    configure_local_integrations(&mut second).unwrap();
    assert_eq!(first.token, second.token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(directory.path().join("remote-token"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::write(directory.path().join("remote-token"), b"\n").unwrap();
    let mut third = Options::default();
    third.config_dir = first.config_dir;
    assert!(configure_local_integrations(&mut third).is_err());
    assert_eq!(
        std::fs::read(directory.path().join("remote-token")).unwrap(),
        b"\n"
    );
}

#[test]
fn existing_pairing_identity_wins_and_experimental_identity_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("remote-token"), "canonical-pairing").unwrap();
    std::fs::write(directory.path().join("hub.token"), "experimental-pairing").unwrap();
    let mut options = Options::default();
    options.config_dir = Some(directory.path().into());
    configure_local_integrations(&mut options).unwrap();
    assert_eq!(options.token, "canonical-pairing");
    let migrated = tempfile::tempdir().unwrap();
    std::fs::write(migrated.path().join("hub.token"), "experimental-pairing").unwrap();
    let mut options = Options::default();
    options.config_dir = Some(migrated.path().into());
    configure_local_integrations(&mut options).unwrap();
    assert_eq!(options.token, "experimental-pairing");
    assert_eq!(
        std::fs::read_to_string(migrated.path().join("remote-token"))
            .unwrap()
            .trim(),
        "experimental-pairing"
    );
    let lost = tempfile::tempdir().unwrap();
    std::fs::write(lost.path().join("tokens.json"), "[]").unwrap();
    let mut options = Options::default();
    options.config_dir = Some(lost.path().into());
    assert!(configure_local_integrations(&mut options).is_err());
    assert!(!lost.path().join("remote-token").exists());
}

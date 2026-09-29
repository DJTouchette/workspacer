//! Retired feature data is user data, not a migration input for the Rust backend.
use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use serde_json::json;
use std::path::Path;
use workspacer_hub::{Options, backend::Backend, client::Client};

fn assert_untouched(database: &Path, bytes: &[u8], artifact: &Path) {
    assert_eq!(std::fs::read(database).unwrap(), bytes);
    assert_eq!(std::fs::read(artifact).unwrap(), b"Retained user artifact");
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(
            !database
                .with_file_name(format!("intent-workspaces.sqlite{suffix}"))
                .exists(),
            "retired database acquired a {suffix} sidecar"
        );
    }
}

#[tokio::test]
async fn owned_startup_preserves_retired_intent_database_and_refuses_its_method() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let config = root.path().join("config");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    // Retain the old feature opt-in: it must not reactivate its storage owner.
    std::fs::write(config.join("config.yaml"), "ui:\n  mode: focus\n  intentWorkspaces: true\nagents:\n  checkProviderOnStartup: false\nusage:\n  pollOnBoot: false\n").unwrap();
    let database = config.join("intent-workspaces.sqlite");
    {
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute_batch("CREATE TABLE intent_workspaces (id TEXT PRIMARY KEY, snapshot TEXT); INSERT INTO intent_workspaces VALUES ('legacy', '{\"title\":\"Retained work\"}'); PRAGMA user_version=10;").unwrap();
        db.close().unwrap();
    }
    let bytes = std::fs::read(&database).unwrap();
    let artifact = config.join("intent-artifacts/legacy.txt");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, "Retained user artifact").unwrap();
    assert_untouched(&database, &bytes, &artifact);

    // Exercise two complete owned starts against the same persisted directory.
    // No global environment changes, external provider calls or real home paths.
    for _ in 0..2 {
        let mut options = Options::default();
        options.home_dir = Some(home.clone());
        options.config_dir = Some(config.clone());
        options.data_dir = Some(root.path().join("data"));
        let backend = Backend::start(
            ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: 0,
                api_port: 0,
                db_path: root.path().join("sessions.sqlite"),
            },
            EngineOptions {
                usage_poll_on_boot: Some(false),
            },
            options,
        )
        .await
        .unwrap();
        assert_untouched(&database, &bytes, &artifact);
        let client = Client::connect(&backend.handle()).await.unwrap();
        let current = client.call("config.get", json!({})).await.unwrap();
        assert_eq!(current["ui"]["mode"], "focus");
        let board = client
            .call("desktop.loadBriefBoard", json!({}))
            .await
            .unwrap();
        assert!(!board["lanes"].as_array().unwrap().is_empty());
        let error = client
            .call(
                "desktop.intentWorkspaceRequest",
                json!({"request":{"action":"list"}}),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("desktop.intentWorkspaceRequest"),
            "unexpected error: {error:#}"
        );
        // Rejection must not be a disconnected/broken backend masquerading as
        // removal: a current public method still succeeds on the same client.
        client.call("config.get", json!({})).await.unwrap();
        assert_untouched(&database, &bytes, &artifact);
        client.close();
        backend.shutdown().await.unwrap();
        assert_untouched(&database, &bytes, &artifact);
    }
}

#[test]
fn account_creation_is_visible_through_the_public_account_list() {
    // Profile root selection intentionally supports process environment in
    // production. Exercise it in a child so no test changes global HOME/PATH.
    const FIXTURE: &str = "WKS_ACCOUNT_LIST_FIXTURE";
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = std::path::PathBuf::from(root);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut options = Options::default();
            options.home_dir = Some(root.join("home"));
            options.config_dir = Some(root.join("config"));
            options.data_dir = Some(root.join("data"));
            let backend = Backend::start(
                ServeConfig {
                    host: "127.0.0.1".into(),
                    hook_port: 0,
                    api_port: 0,
                    db_path: root.join("sessions.db"),
                },
                EngineOptions {
                    usage_poll_on_boot: Some(false),
                },
                options,
            )
            .await
            .unwrap();
            let client = Client::connect(&backend.handle()).await.unwrap();
            let added = client
                .call("desktop.claudeProfilesAddAccount", json!({"name":"Second"}))
                .await
                .unwrap();
            let id = added["profile"]["id"].as_str().unwrap();
            assert!(!id.is_empty());
            assert_eq!(added["profile"]["name"], "Second");
            let accounts = client
                .call("desktop.claudeProfilesAccounts", json!({}))
                .await
                .unwrap();
            assert!(accounts.get(id).is_some_and(|row| row.is_object()));
            let dir = Path::new(added["profile"]["configDir"].as_str().unwrap());
            assert!(
                dir.canonicalize()
                    .unwrap()
                    .starts_with(root.join("home/.claude/accounts").canonicalize().unwrap())
            );
            assert!(!dir.join(".credentials.json").exists());
            client.close();
            backend.shutdown().await.unwrap();
        });
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("home/.claude")).unwrap();
    std::fs::create_dir(root.path().join("config")).unwrap();
    std::fs::write(
        root.path().join("config/config.yaml"),
        "agents:\n  checkProviderOnStartup: false\nusage:\n  pollOnBoot: false\n",
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "account_creation_is_visible_through_the_public_account_list",
            "--nocapture",
        ])
        .env(FIXTURE, root.path())
        .env("CLAUDE_CONFIG_DIR", root.path().join("home/.claude"))
        .env("HOME", root.path().join("home"))
        .env("USERPROFILE", root.path().join("home"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{Options, backend::Backend, client::Client};
fn tagged(settings: &Value) -> Vec<&str> {
    settings["hooks"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|groups| groups.as_array().unwrap())
        .flat_map(|group| group["hooks"].as_array().unwrap())
        .filter_map(|hook| hook["command"].as_str())
        .filter(|command| command.contains("# claudemon-hook"))
        .collect()
}
#[tokio::test]
async fn explicit_settings_path_preserves_user_hooks_status_line_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chosen/settings.json");
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    let untouched = dir.path().join("unselected-settings.json");
    std::fs::write(&untouched, "unchanged").unwrap();
    let original = json!({"theme":"light","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo user-hook"}]}]},"statusLine":{"type":"command","command":"printf user-status","padding":3}});
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    claudemon::daemon::init::run_at_with_port_quiet(path.clone(), 23456)
        .await
        .unwrap();
    let first = std::fs::read(&path).unwrap();
    claudemon::daemon::init::run_at_with_port_quiet(path.clone(), 23456)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first);
    claudemon::daemon::init::run_at_with_port_quiet(path.clone(), 24567)
        .await
        .unwrap();
    let settings: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(settings["theme"], "light");
    assert_eq!(
        settings["hooks"]["PreToolUse"][0],
        original["hooks"]["PreToolUse"][0]
    );
    assert_eq!(settings["statusLine"]["padding"], 3);
    assert_eq!(
        settings["statusLine"]["claudemonOriginalCommand"],
        "printf user-status"
    );
    let status = settings["statusLine"]["command"].as_str().unwrap();
    assert!(status.contains("127.0.0.1:24567/statusline"));
    assert_eq!(status.matches("printf user-status").count(), 1);
    assert!(
        tagged(&settings)
            .iter()
            .all(|command| command.contains("127.0.0.1:24567/hook"))
    );
    assert_eq!(std::fs::read_to_string(untouched).unwrap(), "unchanged");
}
#[tokio::test]
async fn backend_hooks_are_explicit_and_forward_to_the_owned_listener() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let settings = home.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, "{\"theme\":\"keep\"}").unwrap();
    let config = |port| ServeConfig {
        host: "127.0.0.1".into(),
        hook_port: port,
        api_port: 0,
        db_path: dir.path().join("sessions.db"),
    };
    let engine = || EngineOptions {
        usage_poll_on_boot: Some(false),
    };
    let mut options = Options::default();
    options.home_dir = Some(home.clone());
    let owner = Backend::start(config(0), engine(), options).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "{\"theme\":\"keep\"}"
    );
    owner.shutdown().await.unwrap();
    // Select an unused port, then require successful owned bind before any POST.
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let mut options = Options::default();
    options.home_dir = Some(home);
    options.claude_hook_settings = Some(settings.clone());
    let owner = Backend::start(config(port), engine(), options)
        .await
        .unwrap();
    let document: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    let commands = tagged(&document);
    assert!(!commands.is_empty());
    assert!(
        commands
            .iter()
            .all(|command| command.contains(&format!("127.0.0.1:{port}/hook")))
    );
    reqwest::Client::new().post(format!("http://127.0.0.1:{port}/hook")).json(&json!({"hook_event_name":"SessionStart","session_id":"native-hook-fixture","cwd":dir.path()})).send().await.unwrap().error_for_status().unwrap();
    let client = Client::connect(&owner.handle()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Ok(row) = client
                .call(
                    "sessions.snapshot",
                    json!({"sessionId":"native-hook-fixture"}),
                )
                .await
            {
                if row["sessionId"] == "native-hook-fixture" {
                    assert_eq!(row["status"], "active");
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    owner.shutdown().await.unwrap();
    // Match the old native launcher: malformed optional Claude settings warn,
    // but do not prevent unrelated stream/Codex capabilities from starting.
    std::fs::write(&settings, "{ malformed").unwrap();
    let mut options = Options::default();
    options.claude_hook_settings = Some(settings.clone());
    let owner = Backend::start(config(0), engine(), options).await.unwrap();
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ malformed");
    let client = Client::connect(&owner.handle()).await.unwrap();
    assert!(
        client
            .call("agents.list", json!({}))
            .await
            .unwrap()
            .is_array()
    );
    owner.shutdown().await.unwrap();
}

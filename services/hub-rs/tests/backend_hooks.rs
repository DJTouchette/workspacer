use claudemon::daemon::{ServeConfig, embedded::Options as EngineOptions};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{Options, backend::Backend, client::Client};
fn legacy(port: u16) -> Value {
    let hook = json!({"hooks":[{"type":"command","command":format!("curl -s -X POST http://127.0.0.1:{port}/hook -H \"content-type: application/json\" -d @- # claudemon-hook")}]});
    json!({
        "theme": "light",
        "hooks": {
            "PreToolUse": [{"matcher":"Bash","hooks":[{"type":"command","command":"echo user-hook"}]}, hook],
            "Stop": [hook],
        },
        "statusLine": {
            "type": "command",
            "command": format!("i=$(cat); printf '%s' \"$i\" | curl -s --max-time 2 -X POST http://127.0.0.1:{port}/statusline -H \"content-type: application/json\" -d @- >/dev/null 2>&1; printf '%s' \"$i\" | printf user-status # claudemon-statusline"),
            "claudemonOriginalCommand": "printf user-status",
            "padding": 3,
        },
    })
}
#[tokio::test]
async fn legacy_strip_keeps_user_hooks_and_status_line_and_touches_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chosen/settings.json");
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    let untouched = dir.path().join("unselected-settings.json");
    std::fs::write(&untouched, "unchanged").unwrap();
    std::fs::write(&path, serde_json::to_vec(&legacy(23456)).unwrap()).unwrap();
    assert!(
        claudemon::daemon::init::strip_legacy_global_at(path.clone())
            .await
            .unwrap()
    );
    let settings: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        settings,
        json!({
            "theme": "light",
            "hooks": {"PreToolUse": [{"matcher":"Bash","hooks":[{"type":"command","command":"echo user-hook"}]}]},
            "statusLine": {"type":"command","command":"printf user-status","padding":3},
        })
    );
    assert_eq!(std::fs::read_to_string(untouched).unwrap(), "unchanged");
}
#[tokio::test]
async fn backend_strips_legacy_hooks_without_writing_its_own_and_still_ingests_hooks() {
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
    // A clean file is never rewritten, with or without the option.
    let mut options = Options::default();
    options.home_dir = Some(home.clone());
    options.claude_hook_settings = Some(settings.clone());
    let owner = Backend::start(config(0), engine(), options).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "{\"theme\":\"keep\"}"
    );
    owner.shutdown().await.unwrap();

    let legacy_path = home.join(".claude/legacy-settings.json");
    std::fs::write(&legacy_path, serde_json::to_vec(&legacy(7890)).unwrap()).unwrap();
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let mut options = Options::default();
    options.home_dir = Some(home);
    options.claude_hook_settings = Some(legacy_path.clone());
    let owner = Backend::start(config(port), engine(), options)
        .await
        .unwrap();
    let text = std::fs::read_to_string(&legacy_path).unwrap();
    assert!(!text.contains("claudemon-hook"), "{text}");
    assert!(!text.contains("claudemon-statusline"), "{text}");
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
    // Malformed optional Claude settings warn, but do not prevent unrelated
    // stream/Codex capabilities from starting.
    let malformed = dir.path().join("malformed-settings.json");
    std::fs::write(&malformed, "{ malformed # claudemon-hook").unwrap();
    let mut options = Options::default();
    options.claude_hook_settings = Some(malformed.clone());
    let owner = Backend::start(config(0), engine(), options).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&malformed).unwrap(),
        "{ malformed # claudemon-hook"
    );
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

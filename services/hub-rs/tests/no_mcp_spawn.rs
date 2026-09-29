#![cfg(unix)]
use claudemon::{
    child_env::SanitizeChildEnvironment,
    daemon::{ServeConfig, embedded::Options as EngineOptions},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use workspacer_hub::{
    Options, Status,
    backend::Backend,
    client::Client,
    services::{config::Config, library::Library},
};
fn rows(root: &Path, id: &str) -> Vec<Value> {
    std::fs::read_to_string(root.join(format!("received-{id}.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
async fn until(mut ready: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !ready().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("no-MCP fixture timed out");
}
#[test]
fn deliberately_absent_facade_still_launches_receives_first_message_and_reaps_provider() {
    if let Some(root) = std::env::var_os("WKS_NO_MCP_FIXTURE") {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(root.into()));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("bin")).unwrap();
    std::fs::create_dir(root.path().join("home")).unwrap();
    let python = Command::new("python3")
        .args(["-I", "-S", "-c", "import sys; print(sys.executable)"])
        .scrub_host_authority()
        .output()
        .unwrap();
    assert!(python.status.success());
    let binary = root.path().join("bin/claude");
    std::fs::write(
        &binary,
        format!(
            "#!{}\n{}",
            String::from_utf8(python.stdout).unwrap().trim(),
            include_str!("fixtures/fake_claude_no_mcp.py")
        ),
    )
    .unwrap();
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.process_group(0);
    let mut child = command
        .args([
            "--exact",
            "deliberately_absent_facade_still_launches_receives_first_message_and_reaps_provider",
            "--nocapture",
        ])
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.path().join("bin").display()),
        )
        .env("HOME", root.path().join("home"))
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .env("WKS_NO_MCP_FIXTURE", root.path())
        .env("HUB_TOKEN", "fixture-owner")
        .env("WKS_MCP_TOKEN", "fixture-owner")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "no-MCP fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
async fn run(root: PathBuf) {
    let config_dir = root.join("config");
    std::fs::create_dir(&config_dir).unwrap();
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    let profile = root.join("profile");
    std::fs::create_dir(&profile).unwrap();
    Config::open(config_dir.join("config.yaml")).save(json!({"agents":{"binaries":{"claude":root.join("bin/claude")}},"claude":{"transport":"stream","defaultModel":"sonnet"}}),true).unwrap();
    std::fs::write(config_dir.join("claude-profiles.json"),serde_json::to_vec(&json!({"profiles":[{"id":"fixture","name":"Fixture","configDir":profile,"extraArgs":["--append-system-prompt","profile-without-facade"]}]})).unwrap()).unwrap();
    Library::new(config_dir.clone()).save(&json!({"scope":"global","id":"custom","kind":"mcp","mcp":{"type":"stdio","command":"fixture-mcp-never-executed"}})).unwrap();
    let tokens = config_dir.join("tokens.json");
    let mut options = Options::default();
    options.token = "fixture-owner".into();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.mcp_listen = None;
    options.scoped_tokens = Some(tokens.clone());
    options.config_dir = Some(config_dir.clone());
    options.data_dir = Some(root.join("data"));
    options.home_dir = Some(root.join("home"));
    let backend = Backend::start(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.join("state.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
        options,
    )
    .await
    .unwrap();
    assert!(matches!(
        *backend.handle().status().borrow(),
        Status::Ready {
            mcp_address: None,
            ..
        }
    ));
    until(async || {
        backend
            .handle()
            .health()
            .await
            .is_ok_and(|value| value["launchReady"] == true)
    })
    .await;
    assert_eq!(backend.handle().health().await.unwrap()["mcpReady"], false);
    let client = Client::connect(&backend.handle()).await.unwrap();
    let mut pids = Vec::new();
    for selected in [false, true] {
        let mut params = json!({"cwd":project,"provider":"claude","transport":"stream","profileId":"fixture","message":"no-mcp-first-message","trackTask":false});
        if selected {
            params["mcpItemIds"] = json!(["custom"]);
            params["resultSchema"] =
                json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}});
        }
        let receipt = client.call("agents.spawn", params).await.unwrap();
        assert_eq!(receipt["messageQueued"], true);
        let id = receipt["sessionId"].as_str().unwrap();
        until(async || {
            rows(&root, id).iter().any(|row| {
                row["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("no-mcp-first-message"))
            })
        })
        .await;
        let received = rows(&root, id);
        let ready = received.iter().find(|row| row["ready"] == true).unwrap();
        pids.push(ready["pid"].as_i64().unwrap());
        assert_eq!(
            ready["serverNames"],
            if selected {
                json!(["custom"])
            } else {
                json!([])
            }
        );
        if selected {
            assert!(received.iter().any(|row| {
                row["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("STRUCTURED RESULT CONTRACT"))
            }));
        }
        assert!(
            !tokens.exists(),
            "disabled facade minted or created a credential store"
        );
    }
    assert!(!project.join(".workspacer/skills").exists());
    assert!(
        client
            .call(
                "agents.spawn",
                json!({"cwd":project,"provider":"pi","message":"must refuse"})
            )
            .await
            .is_err()
    );
    backend.shutdown().await.unwrap();
    until(async || {
        pids.iter()
            .all(|pid| unsafe { libc::kill(*pid as i32, 0) } != 0)
    })
    .await;
    assert!(!tokens.exists());
}

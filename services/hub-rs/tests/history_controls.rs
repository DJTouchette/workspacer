#![cfg(unix)]
//! Conversation-history controls through the real bus, the embedded engine and
//! a deterministic fake Claude stream CLI (no model calls): the conversation
//! carries reasoning and user-message uuids, `claude.compact` marks a
//! compaction boundary, `claude.rewindFiles` returns the CLI's own verdict, and
//! `claude.revert` is refused for Claude with the alternative named.
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
use workspacer_hub::{Options, backend::Backend, client::Client, services::config::Config};

const TEST: &str = "history_controls_reach_the_stream_cli_through_the_bus";

#[test]
fn history_controls_reach_the_stream_cli_through_the_bus() {
    if let Some(root) = std::env::var_os("WKS_HISTORY_FIXTURE") {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(PathBuf::from(root)));
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
    let python = String::from_utf8(python.stdout).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let binary = root.path().join("bin/claude");
    std::fs::write(
        &binary,
        format!(
            "#!{}\n{}",
            python.trim(),
            include_str!("fixtures/fake_claude_history_session.py")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.process_group(0);
    let mut child = command
        .args(["--exact", TEST, "--nocapture"])
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.path().join("bin").display()),
        )
        .env("HOME", root.path().join("home"))
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .env("WKS_HISTORY_FIXTURE", root.path())
        .env("HUB_TOKEN", "fixture-owner")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
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
        "isolated history-controls fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn until(phase: &str, mut condition: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("history-controls fixture timed out: {phase}"));
}

async fn start(root: &Path) -> Backend {
    let config_dir = root.join("config");
    let mut options = Options::default();
    options.token = "fixture-owner".into();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.scoped_tokens = Some(config_dir.join("tokens.json"));
    options.config_dir = Some(config_dir);
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
    until("backend-launch-ready", async || {
        backend
            .handle()
            .health()
            .await
            .is_ok_and(|health| health["launchReady"] == true)
    })
    .await;
    backend
}

async fn items(client: &Client, id: &str) -> Vec<Value> {
    client
        .call("sessions.conversation", json!({"sessionId":id}))
        .await
        .ok()
        .and_then(|v| v["items"].as_array().cloned())
        .unwrap_or_default()
}

fn received(root: &Path, id: &str) -> Vec<Value> {
    std::fs::read_to_string(root.join(format!("received-{id}.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

async fn run(root: PathBuf) {
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(root.join("config")).unwrap();
    Config::open(root.join("config/config.yaml"))
        .save(
            json!({
                "agents":{"binaries":{"claude":root.join("bin/claude")}},
                "claude":{"transport":"stream","defaultModel":"sonnet"}}),
            true,
        )
        .unwrap();
    let backend = start(&root).await;
    let address = backend.handle().ready().await.unwrap().unwrap();
    let client = Client::connect_remote(&format!("ws://{address}/bus"), "fixture-owner")
        .await
        .unwrap();

    let receipt = client
        .call(
            "agents.spawn",
            json!({"cwd":project,"provider":"claude","transport":"stream","message":"hello"}),
        )
        .await
        .unwrap();
    let id = receipt["sessionId"].as_str().unwrap().to_owned();
    until("first reply", async || {
        items(&client, &id)
            .await
            .iter()
            .any(|i| i["kind"] == "assistant_text" && i["text"] == "done")
    })
    .await;
    let spawned: Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("argv-{id}.json"))).unwrap())
            .unwrap();
    assert!(
        spawned["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("--replay-user-messages")),
        "{spawned}"
    );
    assert_eq!(spawned["checkpointing"], "true");

    // The user item names its message by the uuid the CLI received, once; the
    // reasoning is its own item, once.
    let conversation = items(&client, &id).await;
    let users: Vec<&Value> = conversation
        .iter()
        .filter(|i| i["kind"] == "user_message")
        .collect();
    assert_eq!(users.len(), 1, "{conversation:?}");
    let uuid = users[0]["uuid"]
        .as_str()
        .expect("user item uuid")
        .to_owned();
    assert_eq!(received(&root, &id)[0]["uuid"], json!(uuid));
    let reasoning: Vec<&Value> = conversation
        .iter()
        .filter(|i| i["kind"] == "reasoning")
        .collect();
    assert_eq!(reasoning.len(), 1, "{conversation:?}");
    assert_eq!(reasoning[0]["text"], "Weighing the request.");
    assert!(
        reasoning[0]["item_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );

    // Compact: the CLI receives `/compact <instructions>` and the boundary
    // lands on the conversation and the row.
    let compacted = client
        .call(
            "claude.compact",
            json!({"sessionId":id,"instructions":"keep the API notes"}),
        )
        .await
        .unwrap();
    assert_eq!(compacted["ok"], true, "{compacted}");
    until("compaction boundary", async || {
        items(&client, &id).await.iter().any(|i| {
            i["kind"] == "compaction" && i["trigger"] == "manual" && i["pre_tokens"] == 25000
        })
    })
    .await;
    assert!(
        received(&root, &id)
            .iter()
            .any(|row| row["message"] == "/compact keep the API notes")
    );
    let row = client
        .call("sessions.snapshot", json!({"sessionId":id}))
        .await
        .unwrap();
    assert_ne!(row["compacting"], true, "{row}");

    // Rewind files: the CLI's own verdict comes back either way.
    let preview = client
        .call(
            "claude.rewindFiles",
            json!({"sessionId":id,"userMessageId":uuid,"preview":true}),
        )
        .await
        .unwrap();
    assert_eq!(
        preview["result"]["filesChanged"],
        json!(["a.txt"]),
        "{preview}"
    );
    let refused = client
        .call(
            "claude.rewindFiles",
            json!({"sessionId":id,"userMessageId":uuid}),
        )
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("No file checkpoint"),
        "{refused}"
    );
    let rewinds: Vec<Value> = received(&root, &id)
        .into_iter()
        .filter_map(|row| row.get("rewind").cloned())
        .collect();
    assert_eq!(rewinds.len(), 2);
    assert_eq!(rewinds[0]["user_message_id"], json!(uuid));
    assert_eq!(rewinds[0]["dry_run"], true);

    // Claude does not revert turns; malformed calls never reach the engine.
    let revert = client
        .call(
            "claude.revert",
            json!({"sessionId":id,"userMessageId":uuid}),
        )
        .await
        .unwrap_err();
    assert!(revert.to_string().contains("rewindFiles"), "{revert}");
    for (method, bad) in [
        ("claude.rewindFiles", json!({"sessionId":id})),
        ("claude.revert", json!({"sessionId":id})),
        (
            "claude.rewindFiles",
            json!({"sessionId":id,"userMessageId":"../x"}),
        ),
    ] {
        assert!(client.call(method, bad).await.is_err(), "{method}");
    }

    // Fork from before the second message: a NEW session resumes the source
    // with --fork-session at the entry before that message, pinned to its
    // own id, and shows only the history before the message.
    client
        .call(
            "agents.sendMessage",
            json!({"sessionId":id,"text":"second"}),
        )
        .await
        .unwrap();
    let mut second = String::new();
    until("second turn", async || {
        let conversation = items(&client, &id).await;
        let replied = conversation
            .iter()
            .filter(|i| i["kind"] == "assistant_text" && i["text"] == "done")
            .count()
            >= 2;
        if let Some(user) = conversation
            .iter()
            .find(|i| i["kind"] == "user_message" && i["text"] == "second")
        {
            second = user["uuid"].as_str().unwrap_or_default().to_owned();
        }
        replied && !second.is_empty()
    })
    .await;
    let forked = client
        .call(
            "agents.fork",
            json!({"sessionId":id,"atMessageId":second,"label":"forked"}),
        )
        .await
        .unwrap();
    let fork_id = forked["sessionId"].as_str().unwrap().to_owned();
    assert_ne!(fork_id, id);
    until("fork started", async || {
        root.join(format!("argv-{fork_id}.json")).exists()
    })
    .await;
    let spawned: Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("argv-{fork_id}.json"))).unwrap())
            .unwrap();
    let argv: Vec<&str> = spawned["argv"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let joined = argv.join(" ");
    // The entry just before "second" in the source chain: the `/compact`
    // message (a compaction turn has no assistant row in the fake).
    let sent: Vec<Value> = received(&root, &id)
        .into_iter()
        .filter(|row| row.get("message").is_some())
        .collect();
    let at = sent
        .iter()
        .position(|row| row["message"] == "second")
        .unwrap();
    assert_eq!(sent[at - 1]["message"], "/compact keep the API notes");
    let before = sent[at - 1]["uuid"].as_str().unwrap().to_owned();
    assert!(
        joined.contains(&format!(
            "--resume {id} --fork-session --session-id {fork_id} --resume-session-at {before}"
        )),
        "{joined}"
    );
    let seeded = items(&client, &fork_id).await;
    assert!(
        seeded
            .iter()
            .any(|i| i["kind"] == "assistant_text" && i["text"] == "done"),
        "{seeded:?}"
    );
    assert!(
        !seeded.iter().any(|i| i["text"] == "second"),
        "the forked-away message is not shown: {seeded:?}"
    );
    let row = client
        .call("sessions.snapshot", json!({"sessionId":fork_id}))
        .await
        .unwrap();
    assert_eq!(row["label"], "forked", "{row}");
    // Unknown sources and malformed ids are refused before any launch.
    for bad in [
        json!({"sessionId":"never-launched"}),
        json!({"sessionId":"../x"}),
        json!({"sessionId":id,"atMessageId":"a b"}),
        json!({}),
    ] {
        assert!(
            client.call("agents.fork", bad.clone()).await.is_err(),
            "{bad}"
        );
    }
    drop(client);
    backend.shutdown().await.unwrap();
}

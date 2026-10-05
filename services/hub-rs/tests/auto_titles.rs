#![cfg(unix)]
//! Hub-owned automatic session titles through the real bus, launch journal and
//! embedded engine, with deterministic fake provider CLIs: the agent is a fake
//! Claude stream session and the titler a fake `codex exec`. No model calls.
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

fn calls(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("titler-calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
async fn until(phase: &str, mut condition: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("auto-title fixture timed out: {phase}"));
}
async fn snapshot(client: &Client, id: &str) -> Value {
    client
        .call("sessions.snapshot", json!({"sessionId":id}))
        .await
        .unwrap_or(Value::Null)
}
async fn idle(client: &Client, id: &str) -> bool {
    snapshot(client, id).await["ambientState"] == "idle"
}

#[test]
fn hub_titles_sessions_with_the_configured_harness_and_never_over_a_user_name() {
    if let Some(root) = std::env::var_os("WKS_AUTO_TITLE_FIXTURE") {
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
    for (name, source) in [
        (
            "claude",
            include_str!("fixtures/fake_claude_title_session.py"),
        ),
        ("codex", include_str!("fixtures/fake_codex_titler.py")),
    ] {
        let binary = root.path().join("bin").join(name);
        std::fs::write(&binary, format!("#!{}\n{source}", python.trim())).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.process_group(0);
    let mut child = command
        .args([
            "--exact",
            "hub_titles_sessions_with_the_configured_harness_and_never_over_a_user_name",
            "--nocapture",
        ])
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.path().join("bin").display()),
        )
        .env("HOME", root.path().join("home"))
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .env("WKS_AUTO_TITLE_FIXTURE", root.path())
        .env("HUB_TOKEN", "fixture-owner")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
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
        "isolated auto-title fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
async fn connect(backend: &Backend) -> Client {
    let address = backend.handle().ready().await.unwrap().unwrap();
    Client::connect_remote(&format!("ws://{address}/bus"), "fixture-owner")
        .await
        .unwrap()
}

async fn run(root: PathBuf) {
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(root.join("config")).unwrap();
    // The agent runs on Claude; titles are pinned to Codex with an exact model.
    Config::open(root.join("config/config.yaml")).save(json!({
        "agents":{"binaries":{"claude":root.join("bin/claude"),"codex":root.join("bin/codex")},
            "autoTitle":{"enabled":true,"provider":"codex","models":{"codex":"gpt-5.4-mini-fixture"}}},
        "claude":{"transport":"stream","defaultModel":"sonnet"}}),true).unwrap();
    let backend = start(&root).await;
    let client = connect(&backend).await;
    let mut events = client.events();
    client
        .topics(["agent.snapshot".into()].into())
        .await
        .unwrap();
    let spawn = async |extra: Value| {
        let mut params = json!({"cwd":project,"provider":"claude","transport":"stream"});
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        let receipt = client.call("agents.spawn", params).await.unwrap();
        receipt["sessionId"].as_str().unwrap().to_owned()
    };

    // 1. Titled after the first answer, by the configured harness and model,
    //    from the user's own opening request plus the agent's reply.
    let titled = spawn(json!({"message":"please fix the login redirect loop on mobile\nit started yesterday","autoTitle":true})).await;
    let mut polls = 0;
    until("model title recorded", async || {
        let row = snapshot(&client, &titled).await;
        polls += 1;
        if polls % 100 == 0 {
            let conversation = client
                .call("sessions.conversation", json!({"sessionId":titled}))
                .await;
            eprintln!(
                "waiting for title: mode={} ambient={} label={} autoTitle={} calls={} conversation={:?}",
                row["mode"], row["ambientState"], row["label"], row["autoTitle"], calls(&root).len(), conversation
            );
        }
        row["label"] == "Fix the login redirect loop"
    })
    .await;
    let row = snapshot(&client, &titled).await;
    assert_eq!(row["autoTitle"]["state"], "titled", "{row}");
    assert_eq!(row["autoTitle"]["source"], "model");
    assert_eq!(row["autoTitle"]["provider"], "codex");
    assert_eq!(row["autoTitle"]["model"], "gpt-5.4-mini-fixture");
    assert!(row["autoTitle"].get("prompt").is_none(), "{row}");
    let first = calls(&root);
    assert_eq!(first.len(), 1, "{first:?}");
    let argv: Vec<&str> = first[0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        argv.windows(2)
            .any(|w| w == ["--model", "gpt-5.4-mini-fixture"]),
        "{argv:?}"
    );
    assert!(argv.contains(&"exec"));
    let stdin = first[0]["stdin"].as_str().unwrap();
    assert!(stdin.contains("User: please fix the login redirect loop on mobile"));
    assert!(stdin.contains("Assistant: I traced the redirect to the session cookie check."));
    // Clients learn it from the ordinary snapshot stream.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.data.as_ref().is_some_and(|row| {
                row["sessionId"] == titled.as_str() && row["label"] == "Fix the login redirect loop"
            }) {
                break;
            }
        }
    })
    .await
    .expect("label update was published as agent.snapshot");
    // Later turns never re-title.
    client
        .call(
            "agents.sendMessage",
            json!({"sessionId":titled,"text":"now also handle tablets"}),
        )
        .await
        .unwrap();
    until("titled session idles again", async || {
        idle(&client, &titled).await
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(calls(&root).len(), 1);

    // 2. A name the user gave at launch is never replaced or even paid for.
    let named =
        spawn(json!({"message":"rename me please","label":"My own name","autoTitle":true})).await;
    until("named session answers", async || {
        idle(&client, &named).await
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let row = snapshot(&client, &named).await;
    assert_eq!(row["label"], "My own name");
    assert!(row.get("autoTitle").is_none(), "{row}");
    assert_eq!(calls(&root).len(), 1);

    // 3. A model the titler CLI rejects is not swapped for another one: the
    //    session falls back to the first line of its request, and says so.
    client
        .call(
            "config.save",
            json!({"agents":{"autoTitle":{"models":{"codex":"gpt-reject-fixture"}}}}),
        )
        .await
        .unwrap();
    let failed =
        spawn(json!({"message":"tidy the flaky cache test\nsecond line","autoTitle":true})).await;
    until("fallback recorded", async || {
        snapshot(&client, &failed).await["autoTitle"]["state"] == "fallback"
    })
    .await;
    let row = snapshot(&client, &failed).await;
    assert_eq!(row["label"], "tidy the flaky cache test");
    assert_eq!(row["autoTitle"]["source"], "fallback");
    assert_eq!(row["autoTitle"]["reason"], "unsupported");
    assert_eq!(row["autoTitle"]["model"], "gpt-reject-fixture");
    let rejected = calls(&root);
    assert_eq!(rejected.len(), 2);
    assert!(
        rejected[1]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "gpt-reject-fixture")
    );

    // 4. Off means not now: nothing is called, the launch stays owed, and
    //    turning titles back on names it from the ORIGINAL request.
    client
        .call(
            "config.save",
            json!({"agents":{"autoTitle":{"enabled":false,"models":{"codex":"gpt-5.4-mini-fixture"}}}}),
        )
        .await
        .unwrap();
    let paused = spawn(json!({"message":"audit the billing export","autoTitle":true})).await;
    until("paused session answers", async || {
        idle(&client, &paused).await
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let row = snapshot(&client, &paused).await;
    assert_eq!(row["autoTitle"]["state"], "pending", "{row}");
    assert!(row.get("label").is_none() || row["label"] == "", "{row}");
    assert_eq!(calls(&root).len(), 2);
    client
        .call(
            "config.save",
            json!({"agents":{"autoTitle":{"enabled":true}}}),
        )
        .await
        .unwrap();
    client
        .call(
            "agents.sendMessage",
            json!({"sessionId":paused,"text":"and check the csv columns"}),
        )
        .await
        .unwrap();
    until("paused session titled after re-enable", async || {
        snapshot(&client, &paused).await["autoTitle"]["state"] == "titled"
    })
    .await;
    let last = calls(&root).pop().unwrap();
    assert!(
        last["stdin"]
            .as_str()
            .unwrap()
            .contains("User: audit the billing export")
    );

    // History (`name`, which every client reads as the row's label) shows
    // the same names.
    let recent = client.call("sessions.recent", json!({})).await.unwrap();
    let title_of = |id: &str| {
        recent
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["sessionId"] == id)
            .map(|row| row["name"].clone())
    };
    assert_eq!(
        title_of(&titled),
        Some(json!("Fix the login redirect loop"))
    );
    assert_eq!(title_of(&named), Some(json!("My own name")));
    assert_eq!(title_of(&failed), Some(json!("tidy the flaky cache test")));

    // 5. The owning hub keeps the names across a restart: they live in its
    //    launch journal (the projection over a reopened journal is pinned in
    //    tests/agent_lifecycle.rs). The fake agent writes no provider
    //    transcript, so the restarted daemon has no history rows to show.
    drop(client);
    backend.shutdown().await.unwrap();
    let backend = start(&root).await;
    let client = connect(&backend).await;
    let journal: Value =
        serde_json::from_slice(&std::fs::read(root.join("data/agent-launches.json")).unwrap())
            .unwrap();
    assert_eq!(
        journal[titled.as_str()]["metadata"]["autoTitle"]["title"],
        "Fix the login redirect loop"
    );
    assert_eq!(journal[named.as_str()]["metadata"]["label"], "My own name");
    assert!(
        journal[titled.as_str()]["metadata"]["autoTitle"]
            .get("prompt")
            .is_none()
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(calls(&root).len(), 3, "a restart must not re-title");
    drop(client);
    backend.shutdown().await.unwrap();
}

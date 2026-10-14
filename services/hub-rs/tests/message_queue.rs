#![cfg(unix)]
//! The hub-owned outbound message queue through the real bus, the embedded
//! engine and a deterministic fake Claude stream CLI (no model calls): queued
//! entries wait for the turn to end, go one per turn, never over an open
//! question, can be sent now, pause on a failed delivery and survive a restart.
use claudemon::{
    child_env::SanitizeChildEnvironment,
    daemon::{ServeConfig, embedded::Options as EngineOptions},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};
use workspacer_hub::{Options, backend::Backend, client::Client, services::config::Config};

const TEST: &str = "queued_messages_wait_for_turn_ends_and_survive_a_restart";

#[test]
fn queued_messages_wait_for_turn_ends_and_survive_a_restart() {
    if let Some(root) = std::env::var_os("WKS_QUEUE_FIXTURE") {
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
            include_str!("fixtures/fake_claude_queue_session.py")
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
        .env("WKS_QUEUE_FIXTURE", root.path())
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
        "isolated message-queue fixture failed:\n{}\n{}",
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
    .unwrap_or_else(|_| panic!("message-queue fixture timed out: {phase}"));
}

async fn snapshot(client: &Client, id: &str) -> Value {
    client
        .call("sessions.snapshot", json!({"sessionId":id}))
        .await
        .unwrap_or(Value::Null)
}

fn received(root: &Path, id: &str) -> Vec<String> {
    std::fs::read_to_string(root.join(format!("received-{id}.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        // A launch message arrives behind the hub's launch instructions.
        .filter_map(|row| {
            row["message"]
                .as_str()
                .and_then(|m| m.rsplit('\n').next())
                .map(str::to_owned)
        })
        .collect()
}

fn texts(queue: &Value) -> Vec<String> {
    queue
        .as_array()
        .unwrap_or_else(|| panic!("queue is not an array: {queue}"))
        .iter()
        .map(|entry| entry["text"].as_str().unwrap().to_owned())
        .collect()
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
    Config::open(root.join("config/config.yaml"))
        .save(
            json!({
                "agents":{"binaries":{"claude":root.join("bin/claude")}},
                "claude":{"transport":"stream","defaultModel":"sonnet"}}),
            true,
        )
        .unwrap();
    let backend = start(&root).await;
    let client = connect(&backend).await;
    let mut events = client.events();
    client
        .topics(["agent.snapshot".into()].into())
        .await
        .unwrap();
    let published: Arc<Mutex<Vec<Value>>> = Default::default();
    let sink = published.clone();
    let collector = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Some(data) = event.data {
                        sink.lock().unwrap().push(data);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    let published_queue = |id: &str| -> Option<Vec<String>> {
        published
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|row| row["sessionId"] == id && row.get("queuedMessages").is_some())
            .map(|row| texts(&row["queuedMessages"]))
    };
    let release = |name: &str| std::fs::write(root.join(format!("release-{name}")), b"").unwrap();

    let receipt = client
        .call(
            "agents.spawn",
            json!({"cwd":project,"provider":"claude","transport":"stream","message":"first hold:first"}),
        )
        .await
        .unwrap();
    let id = receipt["sessionId"].as_str().unwrap().to_owned();
    let mut polls = 0;
    until("first turn running", async || {
        let row = snapshot(&client, &id).await;
        polls += 1;
        if polls % 200 == 0 {
            eprintln!("waiting: received={:?} row={row}", received(&root, &id));
        }
        received(&root, &id) == ["first hold:first"] && row["mode"] == "responding"
    })
    .await;
    assert!(
        snapshot(&client, &id).await.get("queuedMessages").is_none(),
        "rows of sessions that never queued carry no queue field"
    );

    // 1. Added while busy: nothing is sent; the snapshot and the published
    //    agent.snapshot carry the queue in send order.
    let first = client
        .call(
            "sessions.queueAdd",
            json!({"sessionId":id,"text":"second hold:second"}),
        )
        .await
        .unwrap();
    let second_id = first["id"].as_str().unwrap().to_owned();
    assert!(second_id.starts_with("q-"), "{first}");
    assert_eq!(texts(&first["queue"]), ["second hold:second"]);
    assert!(first["queue"][0]["createdAt"].as_i64().unwrap() > 0);
    let added = client
        .call(
            "sessions.queueAdd",
            json!({"sessionId":id,"text":"third ask-question"}),
        )
        .await
        .unwrap();
    let third_id = added["id"].as_str().unwrap().to_owned();
    // Reorder and edit: the question entry goes first.
    let moved = client
        .call(
            "sessions.queueMove",
            json!({"sessionId":id,"id":third_id,"index":0}),
        )
        .await
        .unwrap();
    assert_eq!(
        texts(&moved["queue"]),
        ["third ask-question", "second hold:second"]
    );
    let edited = client
        .call(
            "sessions.queueUpdate",
            json!({"sessionId":id,"id":second_id,"text":"second edited hold:second"}),
        )
        .await
        .unwrap();
    assert_eq!(
        texts(&edited["queue"]),
        ["third ask-question", "second edited hold:second"]
    );
    for bad in [
        json!({"sessionId":id,"id":third_id,"index":2}),
        json!({"sessionId":id,"id":"q-missing","index":0}),
        json!({"sessionId":id,"id":third_id,"index":-1}),
    ] {
        assert!(client.call("sessions.queueMove", bad).await.is_err());
    }
    assert!(
        client
            .call(
                "sessions.queueAdd",
                json!({"sessionId":"never-seen","text":"x"})
            )
            .await
            .is_err()
    );
    assert!(
        client
            .call("sessions.queueAdd", json!({"sessionId":id,"text":""}))
            .await
            .is_err()
    );
    let row = snapshot(&client, &id).await;
    assert_eq!(
        texts(&row["queuedMessages"]),
        ["third ask-question", "second edited hold:second"]
    );
    assert_eq!(row["queuePaused"], false);
    until("queue change published", async || {
        published_queue(&id)
            == Some(vec![
                "third ask-question".into(),
                "second edited hold:second".into(),
            ])
    })
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        received(&root, &id),
        ["first hold:first"],
        "sent while busy"
    );

    // 2. The turn ends: exactly the head is sent. It opens a question, and an
    //    open question blocks the next entry.
    release("first");
    until("head sent at turn end", async || {
        received(&root, &id).len() == 2
    })
    .await;
    assert_eq!(
        received(&root, &id),
        ["first hold:first", "third ask-question"]
    );
    until("question open", async || {
        snapshot(&client, &id).await["mode"] == "question"
    })
    .await;
    assert_eq!(
        texts(&snapshot(&client, &id).await["queuedMessages"]),
        ["second edited hold:second"]
    );
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(received(&root, &id).len(), 2, "sent over an open question");

    // 3. Answering ends that turn; the next entry goes once.
    client
        .call(
            "claude.answer",
            json!({"sessionId":id,"answers":["Yes"],"answerKinds":["text"]}),
        )
        .await
        .unwrap();
    until("second sent after the answered turn", async || {
        received(&root, &id).len() == 3
    })
    .await;
    assert_eq!(received(&root, &id)[2], "second edited hold:second");
    until("drained queue published", async || {
        published_queue(&id) == Some(vec![])
    })
    .await;

    // 4. Send now: removed and sent immediately while the agent is busy; the
    //    rest still waits for a turn end.
    for text in ["fourth", "fifth"] {
        client
            .call("sessions.queueAdd", json!({"sessionId":id,"text":text}))
            .await
            .unwrap();
    }
    let queue = snapshot(&client, &id).await["queuedMessages"].clone();
    let fourth_id = queue[0]["id"].as_str().unwrap().to_owned();
    let sent = client
        .call(
            "sessions.queueSendNow",
            json!({"sessionId":id,"id":fourth_id,"delivery":"steer"}),
        )
        .await
        .unwrap();
    assert_eq!(texts(&sent["queue"]), ["fifth"]);
    assert!(
        client
            .call(
                "sessions.queueSendNow",
                json!({"sessionId":id,"id":fourth_id})
            )
            .await
            .is_err(),
        "an entry is sent at most once"
    );
    assert!(
        client
            .call(
                "sessions.queueSendNow",
                json!({"sessionId":id,"id":queue[1]["id"],"delivery":"later"})
            )
            .await
            .is_err()
    );
    // Idempotent remove.
    let removed = client
        .call(
            "sessions.queueRemove",
            json!({"sessionId":id,"id":fourth_id}),
        )
        .await
        .unwrap();
    assert_eq!(texts(&removed["queue"]), ["fifth"]);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(received(&root, &id).len(), 3);
    release("second");
    until("sent-now entry delivered", async || {
        received(&root, &id).len() >= 4
    })
    .await;
    assert_eq!(received(&root, &id)[3], "fourth");
    until("fifth after the next turn end", async || {
        received(&root, &id).len() == 5
    })
    .await;
    assert_eq!(received(&root, &id)[4], "fifth");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(received(&root, &id).len(), 5, "an entry was sent twice");

    // 5. A failed delivery pauses the queue with the error; nothing retries
    //    until the user acts. A hook-only session at its prompt has no input
    //    plumbing, so the daemon refuses the message.
    let hooks = backend.engine_endpoints().unwrap().hook_addr;
    let http = reqwest::Client::new();
    let orphan = "queue-orphan-session";
    for event in ["SessionStart", "UserPromptSubmit"] {
        http.post(format!("http://{hooks}/hook"))
            .json(&json!({"hook_event_name":event,"session_id":orphan,"cwd":project}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    until("orphan busy", async || {
        snapshot(&client, orphan).await["mode"] == "responding"
    })
    .await;
    client
        .call(
            "sessions.queueAdd",
            json!({"sessionId":orphan,"text":"cannot deliver"}),
        )
        .await
        .unwrap();
    http.post(format!("http://{hooks}/hook"))
        .json(&json!({"hook_event_name":"Stop","session_id":orphan,"cwd":project}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    until("failed send paused the queue", async || {
        snapshot(&client, orphan).await["queuePaused"] == true
    })
    .await;
    let row = snapshot(&client, orphan).await;
    assert_eq!(texts(&row["queuedMessages"]), ["cannot deliver"]);
    assert!(
        row["queueError"].as_str().is_some_and(|e| !e.is_empty()),
        "{row}"
    );
    until("the paused queue was published", async || {
        published.lock().unwrap().iter().any(|row| {
            row["sessionId"] == orphan
                && row["queuePaused"] == true
                && row["queueError"].is_string()
        })
    })
    .await;
    // A mutation resumes it: the head is retried (and fails again).
    let added = client
        .call(
            "sessions.queueAdd",
            json!({"sessionId":orphan,"text":"also queued"}),
        )
        .await
        .unwrap();
    assert_eq!(texts(&added["queue"]), ["cannot deliver", "also queued"]);
    until("retried and paused again", async || {
        let row = snapshot(&client, orphan).await;
        row["queuePaused"] == true
            && texts(&row["queuedMessages"]) == ["cannot deliver", "also queued"]
    })
    .await;
    // A send-now failure restores the entry and reports the error.
    let head = snapshot(&client, orphan).await["queuedMessages"][1]["id"].clone();
    assert!(
        client
            .call(
                "sessions.queueSendNow",
                json!({"sessionId":orphan,"id":head})
            )
            .await
            .is_err()
    );
    let row = snapshot(&client, orphan).await;
    assert_eq!(
        texts(&row["queuedMessages"]),
        ["cannot deliver", "also queued"]
    );

    // 6. Restart: the queue (and its paused state) is restored from the data
    //    directory and carried on the restored snapshot.
    let document: Value =
        serde_json::from_slice(&std::fs::read(root.join("data/message-queues.json")).unwrap())
            .unwrap();
    assert_eq!(
        texts(&document["sessions"][orphan]["entries"]),
        ["cannot deliver", "also queued"]
    );
    collector.abort();
    drop(client);
    backend.shutdown().await.unwrap();
    let backend = start(&root).await;
    let client = connect(&backend).await;
    until("queue restored after restart", async || {
        let row = snapshot(&client, orphan).await;
        row.get("queuedMessages")
            .is_some_and(|q| texts(q) == ["cannot deliver", "also queued"])
    })
    .await;
    let cleared = client
        .call("sessions.queueClear", json!({"sessionId":orphan}))
        .await
        .unwrap();
    assert_eq!(cleared["queue"], json!([]));
    let row = snapshot(&client, orphan).await;
    assert_eq!(row["queuedMessages"], json!([]));
    assert_eq!(row["queuePaused"], false);
    assert!(row.get("queueError").is_none(), "{row}");
    drop(client);
    backend.shutdown().await.unwrap();
}

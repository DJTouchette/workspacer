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
use workspacer_hub::{Options, backend::Backend, client::Client, services::config::Config};
// Diagnostic branch only: retain selected state, never argv/env/bearer fields.
static LAST_OBSERVATION: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);
fn observe(value: Value) {
    *LAST_OBSERVATION.lock().unwrap() = Some(value);
}
fn safe_error(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    let lower = text.to_ascii_lowercase();
    if ["token", "authorization", "bearer", "secret"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return "credential-related error text omitted".into();
    }
    text.chars().take(2000).collect()
}
fn selected_snapshot(value: &Value) -> Value {
    json!({"sessionId":value["sessionId"],"status":value["status"],
        "ambientState":value["ambientState"],"mode":value["mode"],"provider":value["provider"],
        "subagentIds":value["subagents"].as_array().into_iter().flatten().map(|row|row["id"].clone()).collect::<Vec<_>>()})
}
struct ObservedClient(Client);
impl std::ops::Deref for ObservedClient {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.0
    }
}
impl ObservedClient {
    async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let session = params["sessionId"].clone();
        let result = self.0.call(method, params).await;
        let state = match &result {
            Ok(value) if method == "sessions.snapshot" => selected_snapshot(value),
            Ok(value) => json!({"ok":value["ok"],"sessionId":value["sessionId"]}),
            Err(error) => json!({"error":safe_error(error)}),
        };
        observe(json!({"method":method,"sessionId":session,"reply":state}));
        result
    }
}
fn selected_log(row: &Value) -> Value {
    let message = row["message"].as_str().unwrap_or("");
    let markers: Vec<_> = [
        "parent-ready-fixture",
        "ask-numeric-fixture",
        "/effort high",
        "/effort low",
        "finish-child-fixture",
        "fixture milestone",
        "child-finished-fixture",
        "CORRECTION FROM YOUR DISPATCHER",
    ]
    .into_iter()
    .filter(|marker| message.contains(marker))
    .collect();
    json!({"ready":row["ready"],"pid":row["pid"],"session":row["session"],
        "scopedMcpCall":row["scopedMcpCall"],"progressMcpCall":row["progressMcpCall"],
        "numericAnswers":row["numericAnswers"],"messageBytes":message.len(),"messageMarkers":markers})
}
fn rows(root: &Path, id: &str) -> Vec<Value> {
    let rows: Vec<_> = std::fs::read_to_string(root.join(format!("received-{id}.jsonl")))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();
    observe(json!({"fixtureSession":id,"rowCount":rows.len(),
        "lastRows":rows.iter().rev().take(8).map(selected_log).collect::<Vec<_>>()}));
    rows
}
fn timed_out(phase: &str) -> ! {
    eprintln!(
        "[diag] phase={phase} last={}",
        LAST_OBSERVATION
            .lock()
            .unwrap()
            .as_ref()
            .unwrap_or(&Value::Null)
    );
    if let Some(root) = std::env::var_os("WKS_LOCAL_SPAWN_FIXTURE") {
        let root = PathBuf::from(root);
        let mut pids = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("received-") && name.ends_with(".jsonl"))
                && name != "codex-processes.jsonl"
            {
                continue;
            }
            let bytes = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let records: Vec<Value> = bytes
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            for row in &records {
                if let Some(pid) = row["pid"].as_i64().filter(|pid| *pid > 0) {
                    pids.insert(pid);
                }
            }
            eprintln!(
                "[diag] file={name} bytes={} records={} last={}",
                bytes.len(),
                records.len(),
                json!(
                    records
                        .iter()
                        .rev()
                        .take(8)
                        .map(selected_log)
                        .collect::<Vec<_>>()
                )
            );
        }
        if !pids.is_empty() {
            let ids = pids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            match Command::new("ps")
                .args(["-o", "pid=,ppid=,state=", "-p", &ids])
                .output()
            {
                Ok(output) => {
                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                        eprintln!("[diag] ps {line}");
                    }
                }
                Err(error) => eprintln!("[diag] ps failed: {}", safe_error(error)),
            }
        }
    }
    panic!("local launch fixture timed out in phase {phase}");
}
async fn until(phase: &str, mut condition: impl AsyncFnMut() -> bool) {
    eprintln!("[phase] begin {phase}");
    let began = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(20), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| timed_out(phase));
    eprintln!(
        "[phase] complete {phase} elapsed_ms={}",
        began.elapsed().as_millis()
    );
}
#[test]
fn owned_rpc_spawn_uses_scoped_mcp_and_routes_real_finish_wake_without_models() {
    if let Some(root) = std::env::var_os("WKS_LOCAL_SPAWN_FIXTURE") {
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
    let script = format!(
        "#!{}\n{}",
        python.trim(),
        include_str!("fixtures/fake_claude_stream.py")
    );
    let binary = root.path().join("bin/claude");
    std::fs::write(&binary, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let codex = root.path().join("bin/codex");
    std::fs::write(
        &codex,
        format!(
            "#!{}\n{}",
            python.trim(),
            include_str!("fixtures/fake_codex_subagent.py")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.process_group(0);
    let mut child = command
        .args([
            "--exact",
            "owned_rpc_spawn_uses_scoped_mcp_and_routes_real_finish_wake_without_models",
            "--nocapture",
        ])
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.path().join("bin").display()),
        )
        .env("HOME", root.path().join("home"))
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .env("WKS_LOCAL_SPAWN_FIXTURE", root.path())
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
    for line in String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.starts_with("[phase]") || line.starts_with("[diag]"))
    {
        eprintln!("{line}");
    }
    assert!(
        output.status.success(),
        "isolated local-spawn fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
async fn run(root: PathBuf) {
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    let config_dir = root.join("config");
    std::fs::create_dir(&config_dir).unwrap();
    let profile = root.join("profile");
    std::fs::create_dir(&profile).unwrap();
    Config::open(config_dir.join("config.yaml")).save(json!({"agents":{"binaries":{"claude":root.join("bin/claude"),"codex":root.join("bin/codex")}},"claude":{"transport":"stream","defaultModel":"sonnet"}}),true).unwrap();
    std::fs::write(
        config_dir.join("claude-profiles.json"),
        serde_json::to_vec(
            &json!({"profiles":[{"id":"isolated","name":"Isolated","configDir":profile}]}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut options = Options::default();
    options.token = "fixture-owner".into();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.scoped_tokens = Some(config_dir.join("tokens.json"));
    options.config_dir = Some(config_dir.clone());
    options.data_dir = Some(root.join("data"));
    options.home_dir = Some(home);
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
        let health = backend.handle().health().await;
        observe(match &health {
            Ok(value) => json!({"healthStatus":value["status"],"launchReady":value["launchReady"],"mcpReady":value["mcpReady"]}),
            Err(error) => json!({"healthError":safe_error(error)}),
        });
        health.is_ok_and(|health| health["launchReady"] == true)
    })
    .await;
    let address = backend.handle().ready().await.unwrap().unwrap();
    let client = ObservedClient(
        Client::connect_remote(&format!("ws://{address}/bus"), "fixture-owner")
            .await
            .unwrap(),
    );
    let mut events = client.events();
    client
        .topics(["agent.snapshot".into()].into())
        .await
        .unwrap();
    let parent=client.call("agents.spawn",json!({"cwd":project,"provider":"claude","transport":"stream","profileId":"isolated","message":"parent-ready-fixture","label":"Fixture parent"})).await.unwrap();
    assert_eq!(parent["messageQueued"], true);
    let parent_id = parent["sessionId"].as_str().unwrap().to_owned();
    until("parent-scoped-mcp-call", async || {
        rows(&root, &parent_id)
            .iter()
            .any(|row| row["scopedMcpCall"] == true)
    })
    .await;
    until("parent-initial-idle", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":parent_id}))
            .await
            .is_ok_and(|row| row["ambientState"] == "idle")
    })
    .await;
    client
        .call(
            "agents.sendMessage",
            json!({"sessionId":parent_id,"text":"ask-numeric-fixture"}),
        )
        .await
        .unwrap();
    until("parent-numeric-question", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":parent_id}))
            .await
            .is_ok_and(|row| row["mode"] == "question")
    })
    .await;
    client.call("claude.answer",json!({"sessionId":parent_id,"answers":["2","3","2"],"answerKinds":["text","text","option"]})).await.unwrap();
    until("numeric-answer-receipt", async || {
        rows(&root, &parent_id).iter().any(|row| {
            row["numericAnswers"]
                == json!({"Literal first":"2","Literal second":"3","Option control":"Blue"})
        })
    })
    .await;
    until("parent-after-question-idle", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":parent_id}))
            .await
            .is_ok_and(|row| row["ambientState"] == "idle")
    })
    .await;
    let child=client.call("agents.spawn",json!({"cwd":project,"provider":"claude","transport":"stream","profileId":"isolated","message":"finish-child-fixture","label":"Fixture child","parentSessionId":parent_id,"trackTask":false})).await.unwrap();
    assert_eq!(child["messageQueued"], true);
    let child_id = child["sessionId"].as_str().unwrap().to_owned();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.data.as_ref().is_some_and(|row| {
                row["sessionId"] == child_id && row["ambientState"] == "streaming"
            }) {
                break;
            }
        }
    })
    .await
    .unwrap();
    let snapshot = client
        .call("sessions.snapshot", json!({"sessionId":child_id}))
        .await
        .unwrap();
    assert_eq!(snapshot["label"], "Fixture child");
    assert_eq!(snapshot["parentSessionId"], parent_id);
    assert!(
        client
            .call("agents.close", json!({"sessionId":child_id}))
            .await
            .is_err()
    );
    std::fs::write(root.join(format!("release-{child_id}")), "go").unwrap();
    until("child-progress-receipt", async || {
        rows(&root, &child_id)
            .iter()
            .any(|row| row["progressMcpCall"] == true)
    })
    .await;
    until("parent-finish-wake-receipt", async || {
        rows(&root, &parent_id)
            .iter()
            .filter_map(|row| row["message"].as_str())
            .any(|message| {
                message.contains(&child_id) && message.contains("child-finished-fixture")
            })
    })
    .await;
    let parent_rows = rows(&root, &parent_id);
    assert!(
        parent_rows
            .iter()
            .filter_map(|row| row["message"].as_str())
            .any(|message| message.contains("fixture milestone"))
    );
    for id in [&parent_id, &child_id] {
        assert!(
            rows(&root, id)
                .iter()
                .any(|row| row["hostCredentialsAbsent"] == true)
        );
    }
    if let Ok(bytes) = std::fs::read(config_dir.join("dispatch-history.json")) {
        let history: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(history["requests"].as_array().is_none_or(Vec::is_empty));
    }
    let successor=client.call("agents.spawn",json!({"cwd":project,"provider":"claude","transport":"stream","profileId":"isolated","message":"parent-ready-fixture","manager":true,"label":"Successor"})).await.unwrap();
    let successor_id = successor["sessionId"].as_str().unwrap().to_owned();
    until("successor-initial-idle", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":successor_id}))
            .await
            .is_ok_and(|r| r["ambientState"] == "idle")
    })
    .await;
    let mode = client
        .call(
            "claude.setPermissionMode",
            json!({"sessionId":successor_id,"mode":"plan"}),
        )
        .await
        .unwrap();
    assert_eq!(mode, json!({"ok":true,"mode":"plan"}));
    let changed = client
        .call(
            "claude.setModel",
            json!({"sessionId":successor_id,"model":"opus[1m]"}),
        )
        .await
        .unwrap();
    assert_eq!(changed["ok"], true, "{changed}");
    assert_eq!(changed["requestedSelection"]["model"], "opus");
    assert_eq!(changed["requestedSelection"]["contextWindow"], 1_000_000);
    let effort = client
        .call(
            "claude.setEffort",
            json!({"sessionId":successor_id,"effort":"high"}),
        )
        .await
        .unwrap();
    assert_eq!(effort, json!({"ok":true,"effort":"high"}));
    until("successor-effort-high-receipt", async || {
        rows(&root, &successor_id)
            .iter()
            .any(|row| row["message"] == "/effort high")
    })
    .await;
    let controlled = client
        .call("sessions.snapshot", json!({"sessionId":successor_id}))
        .await
        .unwrap();
    assert_eq!(controlled["settings"]["effort"], "high");
    assert_eq!(controlled["livePermissionMode"], "plan");
    // A successful daemon change must not turn into a retryable RPC failure
    // just because the subsequent local journal cannot be replaced.
    let journal = root.join("data/agent-launches.json");
    let saved = root.join("data/agent-launches.saved");
    std::fs::rename(&journal, &saved).unwrap();
    std::fs::create_dir(&journal).unwrap();
    let accepted = client
        .call(
            "claude.setEffort",
            json!({"sessionId":successor_id,"effort":"low"}),
        )
        .await
        .unwrap();
    std::fs::remove_dir(&journal).unwrap();
    std::fs::rename(&saved, &journal).unwrap();
    assert_eq!(accepted["ok"], true);
    assert_eq!(accepted["effort"], "low");
    assert!(accepted["warning"].as_str().is_some());
    until(
        "successor-effort-low-after-bookkeeping-failure",
        async || {
            rows(&root, &successor_id)
                .iter()
                .any(|row| row["message"] == "/effort low")
        },
    )
    .await;
    assert_eq!(
        client
            .call(
                "claude.setEffort",
                json!({"sessionId":successor_id,"effort":"high"})
            )
            .await
            .unwrap()["ok"],
        true
    );
    let history = client.call("sessions.recent", json!({})).await.unwrap();
    assert!(
        history
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["sessionId"] == successor_id)
    );
    let handoff = client
        .call("claude.handoffBrief", json!({"sessionId":successor_id}))
        .await
        .unwrap();
    assert_eq!(handoff["ok"], true, "{handoff}");
    assert!(std::path::Path::new(handoff["path"].as_str().unwrap()).is_file());

    until("parent-before-close-idle", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":parent_id}))
            .await
            .is_ok_and(|r| r["ambientState"] == "idle")
    })
    .await;
    client
        .call("agents.close", json!({"sessionId":parent_id}))
        .await
        .unwrap();
    let orphans = client.call("agents.orphans", json!({})).await.unwrap();
    assert!(
        orphans["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["sessionId"] == parent_id && r["confirmedManager"] == false)
    );
    let adopted = client
        .call(
            "agents.reparent",
            json!({"fromSessionId":parent_id,"toSessionId":successor_id}),
        )
        .await
        .unwrap();
    assert!(
        adopted["moved"]
            .as_array()
            .unwrap()
            .contains(&json!(child_id))
    );
    assert_eq!(
        client
            .call("sessions.snapshot", json!({"sessionId":child_id}))
            .await
            .unwrap()["parentSessionId"],
        successor_id
    );
    until("reparented-child-idle", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":child_id}))
            .await
            .is_ok_and(|r| r["ambientState"] == "idle")
    })
    .await;
    // Driver-fed claudemon sessions discard their in-memory conversation on
    // teardown. Clone the observed original while it is available; a missing
    // original must remain an explicit refusal, never an invented dispatch.
    let manager_token = workspacer_hub::auth::load(&config_dir.join("tokens.json"))
        .unwrap()
        .iter()
        .find(|token| token.label == format!("session:{successor_id}"))
        .unwrap()
        .token
        .clone();
    let mcp_address = match *backend.handle().status().borrow() {
        workspacer_hub::Status::Ready {
            mcp_address: Some(address),
            ..
        } => address,
        _ => panic!("owned MCP unavailable"),
    };
    let http = reqwest::Client::new();
    let tool = async |name: &str, arguments: Value| {
        let envelope:Value=http.post(format!("http://{mcp_address}/mcp")).bearer_auth(&manager_token).header("accept","application/json, text/event-stream").header("MCP-Protocol-Version","2025-03-26").json(&json!({"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":name,"arguments":arguments}})).send().await.unwrap().json().await.unwrap();
        assert!(envelope.get("error").is_none(), "{name}: {envelope}");
        assert_ne!(envelope["result"]["isError"], true, "{name}: {envelope}");
        serde_json::from_str::<Value>(envelope["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    let context = tool("manager_context", json!({"tasks":[]})).await;
    assert_eq!(context["inbox"]["ok"], true, "{context}");
    assert_eq!(context["tasks"], json!([]));
    let redispatch = tool(
        "respawn_with",
        json!({"sessionId":child_id,"amendment":"Keep this correction exactly.","trackTask":false}),
    )
    .await;
    let retry_id = redispatch["sessionId"].as_str().unwrap().to_owned();
    assert_ne!(retry_id, child_id);
    assert_eq!(redispatch["clonedFrom"], child_id);
    assert_eq!(redispatch["taskTracking"], false);
    std::fs::write(root.join(format!("release-{retry_id}")), "go").unwrap();
    until("retry-progress-receipt", async || {
        rows(&root, &retry_id)
            .iter()
            .any(|row| row["progressMcpCall"] == true)
    })
    .await;
    let retry_snapshot = client
        .call("sessions.snapshot", json!({"sessionId":retry_id}))
        .await
        .unwrap();
    assert_eq!(retry_snapshot["parentSessionId"], successor_id);
    let received = rows(&root, &retry_id);
    assert!(
        received
            .iter()
            .filter_map(|row| row["message"].as_str())
            .any(|message| message.contains("finish-child-fixture")
                && message.contains("CORRECTION FROM YOUR DISPATCHER")
                && message.contains("Keep this correction exactly."))
    );
    let closed = client
        .call("agents.close", json!({"sessionId":child_id}))
        .await
        .unwrap();
    assert_eq!(closed["removed"], true);
    let retained = client
        .call("sessions.snapshot", json!({"sessionId":child_id}))
        .await
        .unwrap();
    assert_eq!(retained["label"], "Fixture child");
    assert_eq!(retained["parentSessionId"], successor_id);
    let tokens = workspacer_hub::auth::load(&config_dir.join("tokens.json")).unwrap();
    assert!(
        !tokens
            .iter()
            .any(|t| t.label == format!("session:{child_id}"))
    );
    assert!(
        tokens
            .iter()
            .any(|t| t.label == format!("session:{successor_id}"))
    );
    for _ in 0..3 {
        assert!(
            client
                .call("agents.list", json!({}))
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["sessionId"] != child_id)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let codex = client.call("agents.spawn",json!({"cwd":project,"provider":"codex","transport":"stream","model":"gpt-5.6-luna","message":"fixture subagent parent","trackTask":false})).await.unwrap();
    let codex_id = codex["sessionId"].as_str().unwrap();
    until("codex-parent-idle", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":codex_id}))
            .await
            .is_ok_and(|row| row["ambientState"] == "idle")
    })
    .await;
    let day = root.join("home/.codex/sessions/2026/09/30");
    std::fs::create_dir_all(&day).unwrap();
    for id in ["fixture-codex-child", "hidden-codex-child"] {
        let messages = [
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"inspect fixture"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"child fixture result"}]}}),
        ];
        std::fs::write(
            day.join(format!("rollout-2026-09-30-{id}.jsonl")),
            messages
                .into_iter()
                .map(|row| row.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }
    assert!(
        client
            .call(
                "sessions.subagentConversation",
                json!({"sessionId":codex_id,"agentId":"fixture-codex-child"})
            )
            .await
            .is_err(),
        "rollout must not be readable before parent exposure"
    );
    std::fs::write(root.join("expose-codex-child"), "go").unwrap();
    until("codex-child-exposed", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":codex_id}))
            .await
            .is_ok_and(|row| {
                row["subagents"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|row| row["id"] == "fixture-codex-child"))
            })
    })
    .await;
    let conversation = client
        .call(
            "sessions.subagentConversation",
            json!({"sessionId":codex_id,"agentId":"fixture-codex-child"}),
        )
        .await
        .unwrap();
    assert_eq!(conversation["session_id"], codex_id);
    assert_eq!(conversation["agent_id"], "fixture-codex-child");
    assert_eq!(conversation["seq"], 2);
    assert_eq!(conversation["items"][0]["text"], "inspect fixture");
    assert_eq!(conversation["items"][1]["text"], "child fixture result");
    for params in [
        json!({"sessionId":codex_id,"agentId":"hidden-codex-child"}),
        json!({"sessionId":codex_id}),
        json!({"sessionId":codex_id,"agentId":"../escape"}),
    ] {
        assert!(
            client
                .call("sessions.subagentConversation", params)
                .await
                .is_err()
        );
    }
    let mut pids = [&parent_id, &child_id, &successor_id, &retry_id]
        .into_iter()
        .flat_map(|id| rows(&root, id))
        .filter_map(|row| row["pid"].as_i64())
        .collect::<Vec<_>>();
    let codex_processes = std::fs::read_to_string(root.join("codex-processes.jsonl")).unwrap();
    let codex_pids: Vec<_> = codex_processes
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["pid"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert!(
        !codex_pids.is_empty(),
        "managed Codex fixture did not launch"
    );
    pids.extend(codex_pids);
    backend.shutdown().await.unwrap();
    until("all-provider-pids-reaped", async || {
        let probes:Vec<_> = pids.iter().map(|pid| {
            let result = unsafe {libc::kill(*pid as i32,0)};
            json!({"pid":pid,"killZero":result,"errno":if result==0 {None} else {std::io::Error::last_os_error().raw_os_error()}})
        }).collect();
        observe(json!({"pidProbes":probes}));
        probes.iter().all(|probe| probe["killZero"] != 0)
    })
    .await;
}

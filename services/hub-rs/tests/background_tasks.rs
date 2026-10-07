//! Background tasks over the real bus: an embedded claudemon drives a managed
//! stream session whose stand-in `claude` launches a real background shell
//! with the frame shapes the CLI 2.1.286 emits. The hub serves the task's log
//! (`sessions.taskOutput`) and stops it through the CLI's own control request
//! (`sessions.taskStop`), and the scoped tiers get exactly what the vocabulary
//! grants them.
#![cfg(unix)]
use claudemon::daemon::{
    ServeConfig,
    embedded::{Command, EmbeddedDaemon, Options as EngineOptions},
};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{
    Hub, Options,
    auth::{self, Scope},
    client::Client,
};

const SID: &str = "bg-hub";

/// The same stand-in the claudemon driver test uses: a real child whose
/// stdout is `<cwd>/<sid>/tasks/t1.output`, and a `stop_task` answered the
/// way the real CLI answered it.
fn write_stub(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("stub-claude");
    let script = r#"#!/bin/sh
SID=__SID__
out="$PWD/$SID/tasks/t1.output"
mkdir -p "$PWD/$SID/tasks"
while IFS= read -r line; do
  case "$line" in
    *'"type":"user"'*)
      (echo started; exec sleep 30) > "$out" 2>&1 &
      bg=$!
      printf '%s\n' "{\"type\":\"system\",\"subtype\":\"background_tasks_changed\",\"tasks\":[{\"task_id\":\"t1\",\"task_type\":\"local_bash\",\"description\":\"sleep 30\"}],\"session_id\":\"$SID\"}"
      printf '%s\n' "{\"type\":\"system\",\"subtype\":\"task_started\",\"task_id\":\"t1\",\"tool_use_id\":\"tu-bg\",\"description\":\"sleep 30\",\"is_backgrounded\":true,\"task_type\":\"local_bash\",\"session_id\":\"$SID\"}"
      printf '%s\n' "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"tool_use_id\":\"tu-bg\",\"type\":\"tool_result\",\"content\":\"Command running in background with ID: t1. Output is being written to: $out. You will be notified when it completes.\",\"is_error\":false}]},\"parent_tool_use_id\":null,\"session_id\":\"$SID\",\"tool_use_result\":{\"backgroundTaskId\":\"t1\"}}"
      printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"ok","duration_ms":1,"usage":{"input_tokens":1,"output_tokens":1},"total_cost_usd":0.01}'
      ;;
    *'"subtype":"stop_task"'*)
      rid=$(printf '%s' "$line" | sed 's/.*"request_id":"\([^"]*\)".*/\1/')
      kill "$bg" 2>/dev/null
      printf '%s\n' "{\"type\":\"system\",\"subtype\":\"background_tasks_changed\",\"tasks\":[],\"session_id\":\"$SID\"}"
      printf '%s\n' "{\"type\":\"system\",\"subtype\":\"task_notification\",\"task_id\":\"t1\",\"status\":\"stopped\",\"output_file\":\"$out\",\"summary\":\"sleep 30\",\"session_id\":\"$SID\"}"
      printf '%s\n' "{\"type\":\"control_response\",\"response\":{\"subtype\":\"success\",\"request_id\":\"$rid\",\"response\":{}}}"
      ;;
  esac
done
kill "$bg" 2>/dev/null
"#
    .replace("__SID__", SID);
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

async fn until(phase: &str, mut condition: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out: {phase}"));
}

fn task(row: &Value) -> Option<&Value> {
    row["background_task_list"]
        .as_array()?
        .iter()
        .find(|t| t["id"] == "t1")
}

#[tokio::test]
async fn task_log_and_stop_ride_the_bus_with_tiered_access() {
    let dir = tempfile::tempdir().unwrap();
    let stub = write_stub(dir.path());
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: dir.path().join("state.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    engine.ready().await.unwrap();
    let engine_client = engine.client();
    let _updates = engine_client.subscribe().unwrap();
    let mut options = Options::default();
    options.engine = Some(engine_client.clone());
    options.data_dir = Some(dir.path().join("hub"));
    options.config_dir = Some(dir.path().join("config"));
    std::fs::create_dir(dir.path().join("config")).unwrap();
    options.scoped_tokens = Some(dir.path().join("tokens.json"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();

    engine_client
        .request(Command::Request {
            method: "POST".into(),
            path: "/sessions/spawn-managed".into(),
            payload: Some(json!({
                "provider": "claude",
                "transport": "stream",
                "cwd": dir.path(),
                "bin": stub,
                "session_id": SID,
            })),
        })
        .await
        .unwrap();
    until("driver accepts a prompt", async || {
        engine_client
            .request(Command::Request {
                method: "POST".into(),
                path: format!("/sessions/{SID}/message"),
                payload: Some(json!({"text":"start a server"})),
            })
            .await
            .is_ok()
    })
    .await;
    until("the task list reaches the hub snapshot", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":SID}))
            .await
            .ok()
            .and_then(|row| task(&row).map(|t| t["hasOutput"] == true))
            .unwrap_or(false)
    })
    .await;
    let row = client
        .call("sessions.snapshot", json!({"sessionId":SID}))
        .await
        .unwrap();
    let t1 = task(&row).unwrap();
    assert_eq!(t1["taskType"], "local_bash");
    assert_eq!(t1["status"], "running");
    assert!(
        !row.to_string().contains(".output"),
        "the log path never rides the wire"
    );
    #[cfg(target_os = "linux")]
    assert!(t1["pid"].as_u64().is_some(), "a provable pid rides the row");

    let log = client
        .call(
            "sessions.taskOutput",
            json!({"sessionId":SID,"taskId":"t1","offset":0,"maxBytes":4096}),
        )
        .await
        .unwrap();
    assert_eq!(log["text"], "started\n");
    assert_eq!(log["running"], true);
    #[cfg(target_os = "linux")]
    assert_eq!(log["process"]["alive"], true);
    // Refused before claudemon is asked: not one plain id segment / bad ints.
    for bad in [
        json!({"sessionId":SID,"taskId":"../t1"}),
        json!({"sessionId":SID,"taskId":7}),
        json!({"sessionId":SID,"taskId":"t1","offset":-1}),
        json!({"sessionId":SID,"taskId":"t1","maxBytes":"lots"}),
        json!({"sessionId":SID,"taskId":"nope"}),
    ] {
        assert!(
            client
                .call("sessions.taskOutput", bad.clone())
                .await
                .is_err(),
            "{bad}"
        );
    }

    // Tiers: view reads logs but cannot stop; triage may stop; provider neither.
    let tokens = dir.path().join("tokens.json");
    for scope in [Scope::View, Scope::Provider] {
        let record = auth::mint(&tokens, scope, "tasks-fixture").unwrap();
        let scoped = Client::from_connection(
            hub.handle()
                .connect_authenticated(record.token, false)
                .await
                .unwrap(),
        );
        let read = scoped
            .call(
                "sessions.taskOutput",
                json!({"sessionId":SID,"taskId":"t1"}),
            )
            .await;
        let stop = scoped
            .call("sessions.taskStop", json!({"sessionId":SID,"taskId":"t1"}))
            .await;
        assert!(stop.is_err(), "{} may not stop tasks", scope.name());
        assert_eq!(read.is_ok(), scope == Scope::View, "{}", scope.name());
        scoped.close();
    }
    let record = auth::mint(&tokens, Scope::Triage, "tasks-fixture").unwrap();
    let triage = Client::from_connection(
        hub.handle()
            .connect_authenticated(record.token, false)
            .await
            .unwrap(),
    );
    let stopped = triage
        .call("sessions.taskStop", json!({"sessionId":SID,"taskId":"t1"}))
        .await
        .unwrap();
    assert_eq!(stopped["ok"], true);
    until("the stop lands on the row", async || {
        client
            .call("sessions.snapshot", json!({"sessionId":SID}))
            .await
            .ok()
            .and_then(|row| task(&row).map(|t| t["status"] == "stopped"))
            .unwrap_or(false)
    })
    .await;
    // A finished task cannot be stopped twice.
    assert!(
        triage
            .call("sessions.taskStop", json!({"sessionId":SID,"taskId":"t1"}))
            .await
            .is_err()
    );
    triage.close();

    let _ = engine_client
        .request(Command::Request {
            method: "POST".into(),
            path: format!("/sessions/{SID}/signal"),
            payload: Some(json!({"signal":"SIGTERM"})),
        })
        .await;
    hub.shutdown().unwrap();
    engine.shutdown().await.unwrap();
}

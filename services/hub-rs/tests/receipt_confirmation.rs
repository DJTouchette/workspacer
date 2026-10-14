//! An uncertain queue send settled by a delivery receipt, end to end: the
//! hub's queue delivers through a real embedded claudemon to a stand-in
//! stream CLI that reports `command_lifecycle` receipts for the uuid it was
//! sent; the acknowledgement is "lost" (the deliverer answers Uncertain), and
//! the receipt on the projected row turns `queueUncertain` into
//! `queueConfirmed` without resending or resuming.
#![cfg(unix)]
use claudemon::daemon::{
    ServeConfig,
    embedded::{Command, EmbeddedDaemon, Options as EngineOptions},
};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::services::{
    message_queue::{Delivery, Queues},
    snapshots::compat,
};

const SID: &str = "receipt-hub";

/// Reads user frames, logs them, and reports `started` then `completed` for
/// each frame's uuid, as CLI 2.1.299 does.
fn write_stub(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("stub-claude");
    let log = dir.join("frames.jsonl");
    let script = format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"type":"user"'*)
      printf '%s\n' "$line" >> '{log}'
      u=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
      printf '%s\n' "{{\"type\":\"command_lifecycle\",\"command_uuid\":\"$u\",\"state\":\"started\"}}"
      printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"result":"ok","duration_ms":1,"usage":{{"input_tokens":1,"output_tokens":1}},"total_cost_usd":0.0}}'
      printf '%s\n' "{{\"type\":\"command_lifecycle\",\"command_uuid\":\"$u\",\"state\":\"completed\"}}"
      ;;
  esac
done
"#,
        log = log.display()
    );
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

#[tokio::test]
async fn an_uncertain_send_is_confirmed_by_its_own_receipt() {
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
    let engine = engine.client();
    engine
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
    let row = || async {
        compat(
            engine
                .request(Command::Request {
                    method: "GET".into(),
                    path: format!("/sessions/{SID}"),
                    payload: None,
                })
                .await
                .unwrap(),
        )
    };

    // The driver takes messages once it is up.
    until("driver accepts a prompt", async || {
        engine
            .request(Command::Message {
                id: SID.into(),
                text: "warm up".into(),
                delivery: Default::default(),
                sender: Default::default(),
            })
            .await
            .is_ok()
    })
    .await;
    let queues = Queues::default();
    queues.add(SID, "hello").await.unwrap();
    queues.add(SID, "next").await.unwrap();
    let ready =
        json!({"sessionId":SID,"session_id":SID,"mode":"input","status":"active","pending":null});
    // Deliver for real, then lose the acknowledgement.
    let sent_uuid = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen = sent_uuid.clone();
    let client = engine.clone();
    let outcome = queues
        .flush(SID, &ready, move |entry| async move {
            let uuid = entry
                .message_uuid
                .clone()
                .expect("every attempt is stamped");
            *seen.lock().unwrap() = Some(uuid.clone());
            client
                .request(Command::IdentifiedMessage {
                    id: SID.into(),
                    text: entry.text,
                    delivery: Default::default(),
                    sender: Default::default(),
                    uuid,
                })
                .await
                .unwrap();
            Delivery::Uncertain("acknowledgement lost".into())
        })
        .await;
    let uuid = sent_uuid.lock().unwrap().clone().unwrap();
    // No row was observed during the send, so its outcome stays uncertain
    // until the receipt is seen.
    match outcome {
        Some(Delivery::Uncertain(_)) => {
            let enriched = queues.enrich(json!({"sessionId":SID}));
            assert_eq!(enriched["queueUncertain"]["messageUuid"], uuid.as_str());
            until("the receipt settles the uncertain send", async || {
                let row = row().await;
                queues.observe(&row);
                queues.confirm(SID).await
            })
            .await;
            let enriched = queues.enrich(json!({"sessionId":SID}));
            assert!(enriched.get("queueUncertain").is_none(), "{enriched}");
            assert_eq!(enriched["queueConfirmed"]["messageUuid"], uuid.as_str());
            assert_eq!(enriched["queuePaused"], true, "a receipt never resumes");
        }
        other => panic!("unexpected outcome {other:?}"),
    }
    // The CLI received exactly that uuid, once; nothing was resent.
    let frames: Vec<Value> = std::fs::read_to_string(dir.path().join("frames.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(frames.len(), 2, "the warm-up and the queued message, once");
    assert_eq!(frames[1]["uuid"], uuid.as_str());
    let row = row().await;
    assert!(
        row["messageReceipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["uuid"] == uuid.as_str() && r["state"] == "completed"),
        "{row}"
    );
    engine
        .request(Command::Request {
            method: "POST".into(),
            path: format!("/sessions/{SID}/signal"),
            payload: Some(json!({"signal":"SIGTERM"})),
        })
        .await
        .unwrap();
}

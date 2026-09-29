use claudemon::daemon::{
    ServeConfig,
    embedded::{EmbeddedDaemon, Options as EngineOptions},
};
use serde_json::json;
use std::{io::Write, time::Duration};
use workspacer_hub::{Hub, Options, client::Client};

// One real engine in this integration binary; all emitted content is a local
// transcript fixture under a credential-free account root. No provider CLI/API.
#[tokio::test]
async fn embedded_typed_streams_deliver_real_tail_deltas_and_status_without_polling() {
    let dir = tempfile::tempdir().unwrap();
    let account = dir.path().join("fixture-account");
    let project = account.join("projects/project");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("stream-fixture.jsonl");
    std::fs::write(&transcript, "").unwrap();
    claudemon::session::transcript::allow_root(account.join("projects"));
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
    let ready = engine.ready().await.unwrap();
    let engine_client = engine.client();
    let http = reqwest::Client::new();
    http.post(format!("http://{}/hook",ready.hook_addr)).json(&json!({"event":"SessionStart","session_id":"stream-fixture","cwd":dir.path(),"transcript_path":transcript})).send().await.unwrap().error_for_status().unwrap();
    let mut options = Options::default();
    options.engine = Some(engine_client.clone());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    let mut original = engine_client.subscribe_conversations().unwrap();
    let mut status_source = engine_client.subscribe_status_lines().unwrap();
    client
        .topics(
            [
                "agent.conversation.stream-fixture".into(),
                "agent.statusline".into(),
            ]
            .into(),
        )
        .await
        .unwrap();
    let handshake = tokio::time::timeout(Duration::from_secs(4), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handshake.topic, "agent.conversation.stream-fixture");
    assert_eq!(
        handshake.data.unwrap(),
        json!({"session_id":"stream-fixture","ready":true})
    );
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    writeln!(file,"{}",json!({"type":"assistant","message":{"id":"turn-one","role":"assistant","content":[{"type":"text","text":"real typed fixture delta"}]}})).unwrap();
    file.sync_all().unwrap();
    let delta = tokio::time::timeout(Duration::from_secs(5), original.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delta.session_id, "stream-fixture");
    assert!(!delta.items.is_empty());
    let forwarded = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(forwarded.topic, "agent.conversation.stream-fixture");
    assert_eq!(
        forwarded.data.unwrap(),
        serde_json::to_value(delta).unwrap()
    );
    http.post(format!("http://{}/statusline",ready.hook_addr)).json(&json!({"session_id":"stream-fixture","model":{"display_name":"fixture-model"},"cost":{"total_cost_usd":0.125},"context_window":{"context_window_size":200000,"used_percentage":25}})).send().await.unwrap().error_for_status().unwrap();
    let status = tokio::time::timeout(Duration::from_secs(3), status_source.recv())
        .await
        .unwrap()
        .unwrap();
    let forwarded = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(forwarded.topic, "agent.statusline");
    assert_eq!(
        forwarded.data.unwrap(),
        json!({"sessionId":"stream-fixture","statusLine":serde_json::to_value(status.status_line).unwrap()})
    );
    let snapshots = client.call("sessions.snapshots", json!({})).await.unwrap();
    let row = snapshots
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["sessionId"] == "stream-fixture")
        .unwrap();
    assert_eq!(row["statusLine"]["modelDisplay"], "fixture-model");
    client.topics(Default::default()).await.unwrap();
    client.call("sessions.snapshots", json!({})).await.unwrap();
    writeln!(file,"{}",json!({"type":"assistant","message":{"id":"turn-two","role":"assistant","content":[{"type":"text","text":"not demanded"}]}})).unwrap();
    file.sync_all().unwrap();
    tokio::time::timeout(Duration::from_secs(4), original.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(80), events.recv())
            .await
            .is_err()
    );
    drop(client);
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
    engine.shutdown().await.unwrap();
    assert!(engine_client.subscribe_conversations().is_err());
    assert!(engine_client.subscribe_status_lines().is_err());
}

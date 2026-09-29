use claudemon::daemon::{
    ServeConfig,
    embedded::{EmbeddedDaemon, Options as EngineOptions},
};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{Hub, Options, client::Client, protocol::Event};
async fn event(events: &mut tokio::sync::broadcast::Receiver<Event>, topic: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.topic == topic {
                return event.data.unwrap();
            }
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn real_artifact_producer_emits_transitions_preserves_hook_liveness_and_cannot_resurrect_closed_rows()
 {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = home.join(".claude/projects/project");
    let artifacts = project.join("workflow-fixture");
    std::fs::create_dir_all(&artifacts).unwrap();
    let transcript = project.join("workflow-fixture.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: dir.path().join("sessions.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    let ready = engine.ready().await.unwrap();
    let http = reqwest::Client::new();
    let hook = format!("http://{}/hook", ready.hook_addr);
    http.post(&hook).json(&json!({"event":"SessionStart","session_id":"workflow-fixture","cwd":dir.path(),"transcript_path":transcript})).send().await.unwrap().error_for_status().unwrap();
    let mut options = Options::default();
    options.home_dir = Some(home);
    options.config_dir = Some(dir.path().join("config"));
    options.data_dir = Some(dir.path().join("hub"));
    options.engine = Some(engine.client());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client.topics(["workflow.*".into()].into()).await.unwrap();
    let run = artifacts.join("subagents/workflows/wf_live");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("agent-worker.meta.json"), "{}").unwrap();
    let started = event(&mut events, "workflow.started").await;
    assert_eq!(started["sessionId"], "workflow-fixture");
    assert_eq!(started["runId"], "wf_live");
    std::fs::create_dir_all(artifacts.join("workflows")).unwrap();
    std::fs::write(artifacts.join("workflows/wf_live.json"),serde_json::to_vec(&json!({"status":"completed","workflowName":"Observed completion","workflowProgress":[{"type":"workflow_agent","agentId":"agent-worker","state":"done","tokens":4,"toolCalls":2}]})).unwrap()).unwrap();
    assert_eq!(
        event(&mut events, "workflow.completed").await["runId"],
        "wf_live"
    );
    assert_eq!(
        event(&mut events, "workflow.agent.finished").await["agentId"],
        "worker"
    );
    http.post(&hook).json(&json!({"event":"UserPromptSubmit","session_id":"workflow-fixture","cwd":dir.path(),"transcript_path":transcript})).send().await.unwrap().error_for_status().unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let row = client
                .call("sessions.snapshot", json!({"sessionId":"workflow-fixture"}))
                .await
                .unwrap();
            if row["ambientState"] == "streaming" {
                assert_eq!(row["status"], "active");
                assert_eq!(row["workflows"][0]["status"], "completed");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    http.post(&hook).json(&json!({"event":"Stop","session_id":"workflow-fixture","cwd":dir.path(),"transcript_path":transcript})).send().await.unwrap().error_for_status().unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let row = client
                .call("sessions.snapshot", json!({"sessionId":"workflow-fixture"}))
                .await
                .unwrap();
            if row["ambientState"] == "idle" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    client
        .call("agents.close", json!({"sessionId":"workflow-fixture"}))
        .await
        .unwrap();
    let late = artifacts.join("subagents/workflows/wf_late");
    std::fs::create_dir_all(&late).unwrap();
    std::fs::write(late.join("agent-late.meta.json"), "{}").unwrap();
    tokio::time::sleep(Duration::from_millis(2800)).await;
    assert!(
        !client
            .call("sessions.snapshots", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["sessionId"] == "workflow-fixture")
    );
    while let Ok(event) = events.try_recv() {
        assert_ne!(event.data.unwrap_or(Value::Null)["runId"], "wf_late");
    }
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
    engine.shutdown().await.unwrap();
}

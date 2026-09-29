//! One real engine per executable: no provider CLI, credentials, or singleton collisions.
use claudemon::{
    daemon::{
        ServeConfig,
        embedded::{EmbeddedDaemon, Options as EngineOptions},
    },
    session::{
        HookEvent,
        windows::{ModelSelection, PersistedModelSelection},
    },
    store::{Db, SpawnFacts},
};
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{Hub, Options, Status, client::Client};

fn selection(row: &Value) -> Value {
    json!({"requestedSelection":row["requestedSelection"],
        "requested_selection":row["requested_selection"],
        "requested_model":row["requested_model"],
        "resolvedContextWindow":row["resolvedContextWindow"],
        "resolved_context_window":row["resolved_context_window"]})
}

#[tokio::test]
async fn persisted_owner_selection_survives_seed_event_and_uncached_read_and_engine_failure_is_visible()
 {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    {
        let db = Db::open(&db_path).unwrap();
        for id in ["owned", "uncached"] {
            db.record_event_with_spawn_facts(
                &HookEvent {
                    event: "SessionStart".into(),
                    session_id: id.into(),
                    cwd: Some(dir.path().to_string_lossy().into_owned()),
                    timestamp: None,
                    payload: Default::default(),
                },
                SpawnFacts {
                    requested_selection: Some(&PersistedModelSelection {
                        selection: ModelSelection {
                            model: "opus".into(),
                            context_window: Some(1_000_000),
                        },
                        legacy_model: "opus[1m]".into(),
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path,
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    let ready = engine.ready().await.unwrap();
    let engine_client = engine.client();
    let http = reqwest::Client::new();
    http.post(format!("http://{}/hook", ready.hook_addr))
        .json(&json!({"hook_event_name":"SessionStart","session_id":"owned","cwd":dir.path()}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let mut options = Options::default();
    options.engine = Some(engine_client.clone());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let seed = client
        .call("sessions.snapshot", json!({"sessionId":"owned"}))
        .await
        .unwrap();
    let expected = json!({"requestedSelection":{"model":"opus","contextWindow":1000000},
        "requested_selection":{"model":"opus","context_window":1000000},
        "requested_model":"opus[1m]","resolvedContextWindow":1000000,"resolved_context_window":1000000});
    assert_eq!(selection(&seed), expected);
    let mut events = client.events();
    client
        .topics(["agent.snapshot".into()].into())
        .await
        .unwrap();
    http.post(format!("http://{}/hook", ready.hook_addr))
        .json(&json!({"hook_event_name":"UserPromptSubmit","session_id":"owned","cwd":dir.path()}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let Some(row) = event
                .data
                .filter(|row| row["sessionId"] == "owned" && row["ambientState"] == "streaming")
            {
                break row;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(selection(&event), expected);
    let listed = client.call("sessions.snapshots", json!({})).await.unwrap();
    assert_eq!(
        selection(
            listed
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["sessionId"] == "owned")
                .unwrap()
        ),
        expected
    );
    // Empty stopped history is excluded from the initial seed, so this identical
    // persisted selection must travel through the singular daemon fallback.
    assert!(
        !listed
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["sessionId"] == "uncached")
    );
    let fallback = client
        .call("sessions.snapshot", json!({"sessionId":"uncached"}))
        .await
        .unwrap();
    assert_eq!(selection(&fallback), expected);
    let mut status = hub.handle().status();
    engine.shutdown().await.unwrap();
    let failure = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Status::Failed(error) = status.borrow().clone() {
                break error;
            }
            status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        [
            "embedded session engine stopped",
            "embedded session update stream closed",
            "embedded live stream producer stopped",
            "embedded conversation producer closed",
            "embedded status-line producer closed"
        ]
        .contains(&failure.as_str()),
        "{failure}"
    );
    assert!(client.call("sessions.snapshots", json!({})).await.is_err());
    assert_eq!(hub.shutdown().unwrap_err().to_string(), failure);
    // A dead owner cannot initialize an apparently healthy empty fleet either.
    let mut options = Options::default();
    options.engine = Some(engine_client);
    let failed = Hub::start(options).unwrap();
    assert!(failed.ready().await.is_err());
    assert!(failed.shutdown().is_err());
}

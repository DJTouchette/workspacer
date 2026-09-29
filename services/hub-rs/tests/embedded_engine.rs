use claudemon::daemon::{
    ServeConfig,
    embedded::{Command, EmbeddedDaemon, Options as EngineOptions},
};
use serde_json::json;
use workspacer_hub::{Hub, Options, client::Client};

#[tokio::test]
async fn session_service_uses_owned_engine_without_a_go_process_or_loopback_client() {
    let dir = tempfile::tempdir().unwrap();
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
    let _updates = engine_client.subscribe().unwrap();
    let mut options = Options::default();
    options.engine = Some(engine_client.clone());
    options.data_dir = Some(dir.path().join("hub"));
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    options.home_dir = Some(home);
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let name_key = dir.path().to_string_lossy().into_owned();
    std::fs::write(
        config.join("tui-names.json"),
        serde_json::to_vec(&json!({name_key:"Renamed project"})).unwrap(),
    )
    .unwrap();
    options.config_dir = Some(config);
    let hub = Hub::start(options).unwrap();
    assert!(hub.ready().await.unwrap().is_none());
    let client = Client::connect(&hub.handle()).await.unwrap();
    assert_eq!(
        client.call("sessions.snapshots", json!({})).await.unwrap(),
        json!([])
    );
    assert!(
        client
            .call("sessions.snapshot", json!({"sessionId":"../other"}))
            .await
            .is_err()
    );
    let health = engine_client
        .request(Command::Request {
            method: "GET".into(),
            path: "/sessions?include_archived=true".into(),
            payload: None,
        })
        .await
        .unwrap();
    assert!(health.is_array());
    assert!(
        engine_client
            .request(Command::Request {
                method: "GET".into(),
                path: "http://external.example/".into(),
                payload: None
            })
            .await
            .is_err()
    );
    let http = reqwest::Client::new();
    for event in ["SessionStart", "Stop"] {
        http.post(format!("http://{}/hook", ready.hook_addr))
            .json(&json!({"hook_event_name":event,"session_id":"renamed-fixture","cwd":dir.path()}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop {
            if client
                .call("sessions.snapshot", json!({"sessionId":"renamed-fixture"}))
                .await
                .is_ok_and(|row| row["label"] == "Renamed project")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    hub.shutdown().unwrap();
    engine.shutdown().await.unwrap();
    assert!(engine_client.subscribe().is_err());
    assert!(client.call("sessions.snapshots", json!({})).await.is_err());
}

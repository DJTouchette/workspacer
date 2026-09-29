use serde_json::json;
use workspacer_hub::{Hub, Options, client::Client};

#[tokio::test]
async fn absent_and_owned_runtime_status_follow_actual_lifecycle() {
    use claudemon::daemon::{
        ServeConfig,
        embedded::{EmbeddedDaemon, Options as EngineOptions},
    };
    let root = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.config_dir = Some(root.path().join("catalog-config"));
    let catalog = Hub::start(options).unwrap();
    catalog.ready().await.unwrap();
    let client = Client::connect(&catalog.handle()).await.unwrap();
    assert_eq!(
        client
            .call("desktop.agentRuntimeStatus", json!({}))
            .await
            .unwrap(),
        json!({"hub":"ready","claudemon":"unknown","facade":"unknown"})
    );
    client.close();
    tokio::task::spawn_blocking(move || catalog.shutdown())
        .await
        .unwrap()
        .unwrap();

    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.path().join("daemon.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    engine.ready().await.unwrap();
    let mut options = Options::default();
    options.config_dir = Some(root.path().join("owned-config"));
    options.home_dir = Some(root.path().join("home"));
    options.engine = Some(engine.client());
    options.mcp_listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "host-status-fixture".into();
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let status = client
                .call("desktop.agentRuntimeStatus", json!({}))
                .await
                .unwrap();
            assert_eq!(status["hub"], "ready");
            assert_eq!(status["claudemon"], "ready");
            if status["facade"] == "ready" {
                break;
            }
            assert_eq!(status["facade"], "starting");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    engine.shutdown().await.unwrap();
    // An owned daemon disappearing must never remain advertised as ready.
    match client.call("desktop.agentRuntimeStatus", json!({})).await {
        Ok(status) => assert_eq!(status["claudemon"], "failed"),
        Err(error) => {
            let mut status = hub.handle().status();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while matches!(*status.borrow(), workspacer_hub::Status::Ready { .. }) {
                    status
                        .changed()
                        .await
                        .expect("runtime phase must report shutdown");
                }
            })
            .await
            .unwrap_or_else(|_| panic!("runtime stayed ready after read failed: {error}"));
        }
    }
    client.close();
    let _ = tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap();
}

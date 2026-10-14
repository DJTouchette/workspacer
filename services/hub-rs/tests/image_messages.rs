//! `agents.sendMessage` images through a real hub and embedded engine.
use base64::Engine;
use claudemon::{
    daemon::{
        ServeConfig,
        embedded::{Command, EmbeddedDaemon, Options as EngineOptions},
    },
    protocol::WrapperMessage,
};
use futures_util::SinkExt;
use serde_json::json;
use std::time::Duration;
use workspacer_hub::{Hub, Options, client::Client};

/// An upload from `files.upload` passes the hub's confinement, crosses the
/// embedded engine as `Command::Message.images`, and — this being a terminal
/// session, which cannot take images — comes back flagged as delivered as
/// path markers. Any other path is refused before the engine is asked.
#[tokio::test]
async fn uploaded_images_cross_the_engine_and_other_paths_never_do() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.path().join("engine.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    let ready = engine.ready().await.unwrap();
    let owned = engine.client();
    let (mut wrapper, _) = tokio_tungstenite::connect_async_with_config(
        format!("ws://{}/wrapper/image-fixture", ready.api_addr),
        None,
        true,
    )
    .await
    .unwrap();
    wrapper
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&WrapperMessage::Register {
                session_id: "image-fixture".into(),
                cwd: root.path().to_string_lossy().into_owned(),
                argv: vec!["inert-fixture".into()],
                cols: 80,
                rows: 24,
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while owned
            .request(Command::Request {
                method: "GET".into(),
                path: "/sessions/image-fixture".into(),
                payload: None,
            })
            .await
            .is_err()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut options = Options::default();
    options.engine = Some(owned.clone());
    options.data_dir = Some(root.path().join("hub"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let bus = Client::connect(&hub.handle()).await.unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .encode(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x02\0\0\0\x02\x08\x06\0\0\0");
    let upload = bus
        .call("files.upload", json!({"name":"shot.png","dataBase64":png}))
        .await
        .unwrap();
    let path = upload["path"].as_str().unwrap().to_owned();
    let sent = bus
        .call(
            "agents.sendMessage",
            json!({"sessionId":"image-fixture","text":"look","images":[{"path":path}]}),
        )
        .await;
    for refused in [
        json!([{"path":"/etc/passwd"}]),
        json!([{"path":root.path().join("m-1-abcdef01.png")}]),
    ] {
        let error = bus
            .call(
                "agents.sendMessage",
                json!({"sessionId":"image-fixture","text":"no","images":refused}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not a file uploaded"), "{error}");
    }
    let _ = std::fs::remove_file(&path);
    wrapper.close(None).await.unwrap();
    hub.shutdown().unwrap();
    engine.shutdown().await.unwrap();
    let sent = sent.unwrap();
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(sent["imagesAsMarkers"], true, "{sent}");
}

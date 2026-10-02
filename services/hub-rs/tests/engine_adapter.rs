//! Real registered session handlers and owned router, with an inert WS wrapper.
use base64::Engine;
use claudemon::{
    daemon::{
        ServeConfig,
        embedded::{Command, EmbeddedDaemon, Options as EngineOptions},
    },
    protocol::WrapperMessage,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use workspacer_hub::{Hub, Options, client::Client};

#[tokio::test]
async fn malformed_control_fields_never_become_gate_changes_or_pty_input() {
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
        format!("ws://{}/wrapper/adapter-fixture", ready.api_addr),
        None,
        true,
    )
    .await
    .unwrap();
    wrapper
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&WrapperMessage::Register {
                session_id: "adapter-fixture".into(),
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
        loop {
            if owned
                .request(Command::Request {
                    method: "GET".into(),
                    path: "/sessions/adapter-fixture".into(),
                    payload: None,
                })
                .await
                .is_ok()
            {
                break;
            }
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
    let floor = bus
        .call(
            "claude.gate",
            json!({"sessionId":"adapter-fixture","on":true}),
        )
        .await
        .unwrap();
    assert_eq!(floor["gate_enabled"], true);
    let mut failures = Vec::new();
    for bad in [json!("true"), json!(42), json!([]), json!({})] {
        let result = bus
            .call(
                "claude.gate",
                json!({"sessionId":"adapter-fixture","on":bad}),
            )
            .await;
        if result.is_ok() {
            failures.push(format!("gate accepted malformed on {bad}"));
        }
    }
    for params in [
        json!({"sessionId":"adapter-fixture"}),
        json!({"sessionId":"adapter-fixture","on":null}),
        json!({"sessionId":"adapter-fixture","on":false}),
    ] {
        assert_eq!(
            bus.call("claude.gate", params).await.unwrap()["gate_enabled"],
            false
        );
    }
    bus.call(
        "claude.answer",
        json!({"sessionId":"adapter-fixture","text":"positive-floor","opaque":{"keep":[42,null,{"flag":false}]}}),
    )
    .await
    .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(3), wrapper.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let first: WrapperMessage = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert!(
        matches!(first,WrapperMessage::Input {bytes} if base64::engine::general_purpose::STANDARD.decode(&bytes).unwrap()==b"positive-floor\r")
    );
    for (method, mut params, expected) in [
        ("sessions.transcript", json!({"cwd":42}), "cwd must be text"),
        (
            "sessions.conversation",
            json!({"sinceSeq":"1"}),
            "sinceSeq must be an integer",
        ),
        (
            "sessions.conversation",
            json!({"limit":-1}),
            "limit must be a non-negative integer",
        ),
        (
            "claude.approve",
            json!({"decision":"yes","reason":42}),
            "reason must be text",
        ),
        (
            "agents.sendMessage",
            json!({"text":"bad-sender","fromSessionId":42}),
            "fromSessionId must be text",
        ),
    ] {
        params["sessionId"] = json!("adapter-fixture");
        match bus.call(method, params).await {
            Err(error) if error.to_string().contains(expected) => (),
            other => failures.push(format!(
                "{method} did not reject its typed field before daemon request: {other:?}"
            )),
        }
    }
    for mut bad in [
        json!({"option":"2","text":"bad-option"}),
        json!({"option":1,"text":42}),
        json!({"answers":[42],"text":"bad-answers"}),
        json!({"answers":42,"text":"bad-array"}),
        json!({"answerKinds":[42],"text":"bad-kinds"}),
        json!({"option":1.5,"text":"bad-fraction"}),
    ] {
        bad["sessionId"] = json!("adapter-fixture");
        if bus.call("claude.answer", bad.clone()).await.is_ok() {
            failures.push(format!("answer accepted malformed carrier {bad}"));
        }
    }
    // A same-wrapper delivery marker proves the queue is drained, avoiding a
    // timing-only claim that malformed calls emitted no input.
    bus.call("claude.answer",json!({"sessionId":"adapter-fixture","option":null,"text":"delivery-marker","answers":null,"answerKinds":null})).await.unwrap();
    let observed = tokio::time::timeout(Duration::from_secs(3), async {
        let mut unexpected = Vec::new();
        loop {
            let frame = wrapper.next().await.unwrap().unwrap();
            let message: WrapperMessage = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            if let WrapperMessage::Input { bytes } = message {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bytes)
                    .unwrap();
                if bytes == b"delivery-marker\r" {
                    break unexpected;
                }
                unexpected.push(bytes);
            }
        }
    })
    .await
    .unwrap();
    if !observed.is_empty() {
        failures.push(format!("malformed calls delivered PTY input: {observed:?}"));
    }
    for (mut params, expected) in [
        (
            json!({"option":2,"text":"lower-priority","answers":[null],"answerKinds":[null]}),
            vec!["2\r"],
        ),
        (
            json!({"option":null,"text":"","answers":["lower-priority"]}),
            vec!["\r"],
        ),
        (
            json!({"text":null,"answers":[null,"array-value"],"answerKinds":[null,"text"]}),
            vec!["\r", "array-value\r"],
        ),
    ] {
        params["sessionId"] = json!("adapter-fixture");
        if let Err(error) = bus.call("claude.answer", params.clone()).await {
            failures.push(format!("valid Go-default answer refused {params}: {error}"));
            continue;
        }
        for expected in expected {
            let frame = tokio::time::timeout(Duration::from_secs(3), wrapper.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let message: WrapperMessage = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert!(
                matches!(message,WrapperMessage::Input {bytes} if base64::engine::general_purpose::STANDARD.decode(&bytes).unwrap()==expected.as_bytes())
            );
        }
    }
    bus.close();
    wrapper.close(None).await.unwrap();
    hub.shutdown().unwrap();
    engine.shutdown().await.unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

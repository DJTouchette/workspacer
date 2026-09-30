//! Actual socket paths retained from the editor and external-daemon spine.
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use workspacer_hub::{
    Hub, Options,
    plugins::{Manager, manifest::Manifest},
    protocol::Frame,
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
async fn frame(socket: &mut Socket) -> Frame {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return Frame::decode(text.as_bytes()).unwrap(),
                Message::Ping(_) => socket.flush().await.unwrap(),
                other => panic!("unexpected frame {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}
async fn send(socket: &mut Socket, value: serde_json::Value) {
    socket.send(Message::Text(value.to_string())).await.unwrap();
}

#[tokio::test]
async fn shipped_editor_pane_reads_ambient_paths_but_cannot_claim_host_authority() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let plugin_dir = tempfile::tempdir().unwrap();
    // This is the shipped non-Go editor asset, not a synthetic permissive manifest.
    let manifest_bytes = include_bytes!("../../../plugins/examples/editor/plugin.json");
    std::fs::write(plugin_dir.path().join("plugin.json"), manifest_bytes).unwrap();
    let manifest = Manifest::load(&plugin_dir.path().join("plugin.json")).unwrap();
    let mut options = Options::default();
    options.home_dir = Some(home.path().into());
    options.scoped_tokens = Some(home.path().join("tokens.json"));
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "owner-fixture".into();
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let mut manager = Manager::new(plugin_dir.path().into(), hub.handle(), String::new());
    manager.add(manifest).await.unwrap();
    let token = manager.pane_token("workspacer.editor").await.unwrap();
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/bus?token={token}"))
            .await
            .unwrap();
    assert_eq!(frame(&mut socket).await.op, "hello");
    for (id, path, contents) in [
        (
            "inside",
            project.path().join("inside.txt"),
            "inside project",
        ),
        (
            "outside",
            outside.path().join("outside.txt"),
            "ambient outside 🦀",
        ),
    ] {
        std::fs::write(&path, contents).unwrap();
        send(&mut socket, json!({"op":"call","id":id,"method":"fs.read","params":{"path":path,"cwd":project.path()}})).await;
        let reply = frame(&mut socket).await;
        assert_eq!(reply.op, "result", "{reply:?}");
        assert_eq!(reply.id, id);
        assert_eq!(reply.result.unwrap()["contents"], contents);
    }
    send(
        &mut socket,
        json!({"op":"register","methods":["fs.read","agents.spawn"]}),
    )
    .await;
    let refused = frame(&mut socket).await;
    assert_eq!(refused.op, "registered");
    assert!(refused.methods.is_empty());
    send(
        &mut socket,
        json!({"op":"call","id":"owner-only","method":"remote.tokensList","params":{}}),
    )
    .await;
    let refused = frame(&mut socket).await;
    assert_eq!(refused.op, "error", "{refused:?}");
    assert_eq!(refused.id, "owner-only");
    assert!(
        refused.error.contains("host") || refused.error.contains("owner"),
        "{}",
        refused.error
    );
    let (mut owner, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/bus?token=owner-fixture"))
            .await
            .unwrap();
    assert_eq!(frame(&mut owner).await.op, "hello");
    send(
        &mut owner,
        json!({"op":"call","id":"owner-floor","method":"remote.tokensList","params":{}}),
    )
    .await;
    let accepted = frame(&mut owner).await;
    assert_eq!(accepted.op, "result", "{accepted:?}");
    assert_eq!(accepted.result, Some(json!([])));
    owner.close(None).await.unwrap();
    manager
        .set_enabled("workspacer.editor", false)
        .await
        .unwrap();
    assert!(
        tokio_tungstenite::connect_async(format!("ws://{address}/bus?token={token}"))
            .await
            .is_err()
    );
    let _ = socket.close(None).await;
    manager.stop().await.unwrap();
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn external_sse_crosses_bridge_broker_and_websocket_with_stamped_envelope() {
    use axum::{
        Router,
        body::{Body, Bytes},
        response::Response,
        routing::get,
    };
    let (release, ready) = tokio::sync::watch::channel(false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route("/events", get(move || {
        let mut ready = ready.clone();
        async move {
            let stream = futures_util::stream::once(async move {
                if !*ready.borrow() { ready.changed().await.unwrap(); }
                Ok::<_, std::io::Error>(Bytes::from_static(b"event: session.update\ndata: {\"session_id\":\"spine\",\"event\":\"Stop\",\"state\":{\"mode\":\"input\"}}\n\n"))
            }).chain(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
            Response::builder().header("content-type", "text/event-stream").body(Body::from_stream(stream)).unwrap()
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut options = Options::default();
    options.control_plane_only = true;
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "spine-fixture".into();
    options.external_claudemon_url = Some(url.clone());
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/bus?token=spine-fixture"))
            .await
            .unwrap();
    assert_eq!(frame(&mut socket).await.op, "hello");
    send(&mut socket, json!({"op":"subscribe","topics":["agent.*"]})).await;
    assert_eq!(frame(&mut socket).await.op, "subscribed");
    release.send_replace(true);
    let received = frame(&mut socket).await;
    assert_eq!(received.op, "event");
    let event = received.event.unwrap();
    assert_eq!(event.topic, "agent.state_changed");
    assert_eq!(event.source, "claudemon");
    assert!(!event.id.is_empty());
    assert!(
        chrono::DateTime::parse_from_rfc3339(&event.time).is_ok(),
        "{}",
        event.time
    );
    assert_eq!(
        event.data,
        Some(json!({"sessionId":"spine","hookEvent":"Stop","mode":"input"}))
    );
    socket.close(None).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || hub.shutdown()),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    // Borrowed SSE ownership does not stop the external HTTP server.
    assert!(
        reqwest::get(format!("{url}/events"))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    server.abort();
    let _ = server.await;
}

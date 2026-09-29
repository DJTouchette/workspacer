use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{
    Hub, Options, Status,
    protocol::{Event, Frame, matches},
};

async fn recv(c: &mut workspacer_hub::Connection) -> Frame {
    tokio::time::timeout(Duration::from_secs(2), c.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn dropping_ui_does_not_stop_runtime_but_explicit_shutdown_closes_clients() {
    let hub = Hub::start(Options::default()).unwrap();
    assert_eq!(hub.ready().await.unwrap(), None);
    let handle = hub.handle();
    drop(handle.connect().await.unwrap());
    let mut client = handle.connect().await.unwrap();
    assert_eq!(recv(&mut client).await.op, "hello");
    let state = handle.status();
    hub.shutdown().unwrap();
    assert_eq!(*state.borrow(), Status::Stopped);
    assert!(client.recv().await.is_none());
    assert!(handle.connect().await.is_err());
}

#[tokio::test]
async fn shutdown_cancels_inflight_local_handler() {
    let (started, mut ready) = tokio::sync::mpsc::channel(1);
    let hub = Hub::start(Options::default().handler("fixture.wait", move |_, _| {
        let started = started.clone();
        async move {
            started.send(()).await.unwrap();
            std::future::pending::<anyhow::Result<Value>>().await
        }
    }))
    .unwrap();
    hub.ready().await.unwrap();
    let mut client = hub.handle().connect().await.unwrap();
    recv(&mut client).await;
    client
        .send(Frame {
            id: "one".into(),
            method: "fixture.wait".into(),
            ..Frame::op("call")
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), ready.recv())
        .await
        .unwrap();
    let before = std::time::Instant::now();
    hub.shutdown().unwrap();
    assert!(before.elapsed() < Duration::from_secs(2));
    assert!(client.recv().await.is_none());
}

#[tokio::test]
async fn listener_collision_is_reported_without_claiming_readiness() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut options = Options::default();
    options.listen = Some(listener.local_addr().unwrap());
    options.token = "fixture".into();
    let hub = Hub::start(options).unwrap();
    assert!(hub.ready().await.is_err());
    assert!(hub.shutdown().is_err());
}

#[tokio::test]
async fn slow_stream_consumer_gets_desync_without_blocking_rpc() {
    let mut options = Options::default().handler("fixture.echo", |_, p| async move { Ok(p) });
    options.event_buffer = 1;
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let mut client = hub.handle().connect().await.unwrap();
    recv(&mut client).await;
    client
        .send(Frame {
            topics: vec!["pty.bytes.*".into()],
            ..Frame::op("subscribe")
        })
        .unwrap();
    recv(&mut client).await;
    for _ in 0..3 {
        client
            .send(Frame {
                event: Some(Event::new(
                    "pty.bytes.one",
                    "fixture",
                    json!({"bytes":"abc"}),
                )),
                ..Frame::op("publish")
            })
            .unwrap();
    }
    client
        .send(Frame {
            id: "barrier".into(),
            method: "fixture.echo".into(),
            params: Some(json!(42)),
            ..Frame::op("call")
        })
        .unwrap();
    // Health is an actor barrier: all publishes have reached the bounded buffer.
    hub.handle().health().await.unwrap();
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(recv(&mut client).await);
    }
    assert!(
        seen.iter()
            .any(|f| f.id == "barrier" && f.result == Some(json!(42)))
    );
    assert!(seen.iter().any(|f| {
        f.event
            .as_ref()
            .is_some_and(|e| e.topic == "pty.desync" && e.data == Some(json!({"sessionId":"one"})))
    }));
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn provider_timeout_is_bounded_and_late_reply_cannot_complete_another_call() {
    let mut options = Options::default();
    options.call_timeout = Duration::from_millis(40);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let mut caller = hub.handle().connect().await.unwrap();
    recv(&mut caller).await;
    let mut provider = hub.handle().connect().await.unwrap();
    recv(&mut provider).await;
    provider
        .send(Frame {
            methods: vec!["fixture.wait".into()],
            ..Frame::op("register")
        })
        .unwrap();
    recv(&mut provider).await;
    caller
        .send(Frame {
            id: "one".into(),
            method: "fixture.wait".into(),
            ..Frame::op("call")
        })
        .unwrap();
    let old = recv(&mut provider).await;
    assert_eq!(
        recv(&mut caller).await,
        Frame::error(
            "one",
            "call timed out; outcome is unknown, do not automatically retry"
        )
    );
    caller
        .send(Frame {
            id: "two".into(),
            method: "fixture.wait".into(),
            ..Frame::op("call")
        })
        .unwrap();
    let new = recv(&mut provider).await;
    provider
        .send(Frame {
            id: old.id,
            result: Some(json!("late")),
            ..Frame::op("result")
        })
        .unwrap();
    provider
        .send(Frame {
            id: new.id,
            result: Some(json!("current")),
            ..Frame::op("result")
        })
        .unwrap();
    let result = recv(&mut caller).await;
    assert_eq!(result.id, "two");
    assert_eq!(result.result, Some(json!("current")));
    hub.shutdown().unwrap();
}

#[test]
fn wildcard_matches_dot_namespace_only() {
    for (pattern, topic, expected) in [
        ("agent.*", "agent", false),
        ("agent.*", "agentx.one", false),
        ("agent.*", "agent.state.changed", true),
        ("agent*", "agent.one", false),
        ("*", "anything", true),
    ] {
        assert_eq!(matches(pattern, topic), expected);
    }
}

#[test]
fn operation_budgets_match_the_legacy_provider_contract() {
    use workspacer_hub::protocol::provider_timeout;
    for (method, seconds) in [
        ("agents.spawn", 360),
        ("hub:worker/agents.spawn", 360),
        ("desktop.worktreeCreate", 360),
        ("claude.handoffAgentBrief", 180),
        ("desktop.managerRequestSend", 60),
        ("desktop.worktreeRemove", 60),
        ("sessions.snapshots", 30),
    ] {
        assert_eq!(
            provider_timeout(method, Duration::from_secs(30)),
            Duration::from_secs(seconds)
        );
        assert_eq!(
            provider_timeout(method, Duration::from_millis(10)),
            Duration::from_millis(10)
        );
        assert_eq!(
            provider_timeout(method, Duration::from_secs(600)),
            Duration::from_secs(600)
        );
    }
}

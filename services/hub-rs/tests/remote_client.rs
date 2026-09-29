use serde_json::json;
use std::{collections::BTreeSet, time::Duration};
use workspacer_hub::{Hub, Options, client::Client, protocol::Event};

#[tokio::test]
async fn prehello_close_keeps_typed_pause_identity_without_logging_query_credentials() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, protocol::CloseFrame};
    use workspacer_hub::client::DisconnectReason;
    for code in [4001u16, 4003] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: code.into(),
                    reason: "peer failure ws://peer/bus?token=fixture-query-secret&keep=1".into(),
                })))
                .await
                .unwrap();
            let _ = tokio::time::timeout(Duration::from_secs(2), socket.next()).await;
        });
        let error = Client::connect_remote(&format!("ws://{address}/bus"), "fixture-header-secret")
            .await
            .err()
            .expect("fixture must close before hello");
        let typed = error
            .downcast_ref::<DisconnectReason>()
            .expect("close classification retained");
        assert_eq!(typed.is_power_paused(), code == 4001);
        for rendered in [
            error.to_string(),
            format!("{error:#}"),
            format!("{error:?}"),
            format!("{typed:?}"),
        ] {
            assert!(!rendered.contains("fixture-query-secret"), "{rendered}");
            assert!(!rendered.contains("fixture-header-secret"), "{rendered}");
            assert!(
                rendered.contains("ws://peer/bus?token=REDACTED&keep=1"),
                "{rendered}"
            );
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn remote_calls_events_and_disconnect_preserve_the_same_client_contract() {
    let (started, mut starts) = tokio::sync::mpsc::channel(2);
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "remote-fixture-secret".into();
    let options = options
        .handler("fixture.echo", |_, params| async move { Ok(params) })
        .handler("fixture.wait", move |_, _| {
            let started = started.clone();
            async move {
                started.send(()).await.unwrap();
                std::future::pending::<anyhow::Result<serde_json::Value>>().await
            }
        });
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let url = format!("ws://{address}/bus");
    let rejected = Client::connect_remote(&url, "wrong-secret")
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(!rejected.contains("wrong-secret"));
    let client = Client::connect_remote(&url, "remote-fixture-secret")
        .await
        .unwrap();
    assert_eq!(
        client.call("fixture.echo", json!({"n":1})).await.unwrap(),
        json!({"n":1})
    );
    let mut events = client.events();
    client
        .topics(BTreeSet::from(["agent.*".into()]))
        .await
        .unwrap();
    // A reply after the subscribe frame establishes that the broker processed it.
    client.call("fixture.echo", json!(null)).await.unwrap();
    hub.handle()
        .publish_wait(Event::new("agent.updated", "fixture", json!({"id":"one"})))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.data.unwrap()["id"], "one");
    let caller = client.clone();
    let pending = tokio::spawn(async move { caller.call("fixture.wait", json!({})).await });
    tokio::time::timeout(Duration::from_secs(2), starts.recv())
        .await
        .unwrap()
        .unwrap();
    hub.shutdown().unwrap();
    tokio::time::timeout(Duration::from_secs(2), client.disconnected())
        .await
        .unwrap();
    let error = pending.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("outcome is unknown"), "{error}");
    assert!(
        client
            .call("fixture.echo", json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("not submitted")
    );
    assert!(starts.try_recv().is_err());
}

#[tokio::test]
async fn peer_tag_withholds_host_identity_even_with_a_host_token() {
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "remote-fixture-secret".into();
    let options = options.handler("fixture.identity", |caller, _| async move {
        Ok(json!({"host":caller.authenticated_host}))
    });
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let direct = Client::connect_remote(&format!("ws://{address}/bus"), "remote-fixture-secret")
        .await
        .unwrap();
    let peer = Client::connect_remote(
        &format!("ws://{address}/bus?peer=1"),
        "remote-fixture-secret",
    )
    .await
    .unwrap();
    assert_eq!(
        direct.call("fixture.identity", json!({})).await.unwrap()["host"],
        true
    );
    assert_eq!(
        peer.call("fixture.identity", json!({})).await.unwrap()["host"],
        false
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn concurrent_remote_replies_errors_and_live_subscription_changes_remain_correlated() {
    let (admitted, mut admissions) = tokio::sync::mpsc::channel(64);
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "correlation-token-with-query-characters-&?=+".into();
    let options = options
        .handler("fixture.waitForReply", move |_, params| {
            let admitted = admitted.clone();
            async move {
                let (reply, wait) = tokio::sync::oneshot::channel();
                admitted
                    .send((params["n"].as_u64().unwrap(), reply))
                    .await
                    .unwrap();
                let fail = wait.await.unwrap();
                if fail {
                    anyhow::bail!("fixture exact provider error");
                }
                Ok(params)
            }
        })
        .handler("fixture.barrier", |_, p| async move { Ok(p) });
    let hub = Hub::start(options).unwrap();
    let address = hub.ready().await.unwrap().unwrap();
    let client = Client::connect_remote(
        &format!("ws://{address}/bus?fixture=kept"),
        "correlation-token-with-query-characters-&?=+",
    )
    .await
    .unwrap();
    let mut pending = Vec::new();
    for n in 0..32u64 {
        let caller = client.clone();
        pending.push((
            n,
            tokio::spawn(async move { caller.call("fixture.waitForReply", json!({"n":n})).await }),
        ));
    }
    let mut replies = std::collections::BTreeMap::new();
    for _ in 0..32 {
        let (n, reply) = tokio::time::timeout(Duration::from_secs(3), admissions.recv())
            .await
            .unwrap()
            .unwrap();
        replies.insert(n, reply);
    }
    // Deliberately invert admission order; one provider error cannot poison
    // another outstanding result or consume the wrong correlation ID.
    for (n, reply) in replies.into_iter().rev() {
        reply.send(n == 7).unwrap();
    }
    for (n, reply) in pending {
        let reply = reply.await.unwrap();
        if n == 7 {
            assert!(
                reply
                    .unwrap_err()
                    .to_string()
                    .contains("fixture exact provider error")
            );
        } else {
            assert_eq!(reply.unwrap(), json!({"n":n}));
        }
    }
    let mut events = client.events();
    client
        .topics(BTreeSet::from(["fixture.old".into()]))
        .await
        .unwrap();
    client.call("fixture.barrier", json!(null)).await.unwrap();
    client
        .topics(BTreeSet::from(["fixture.new".into()]))
        .await
        .unwrap();
    client.call("fixture.barrier", json!(null)).await.unwrap();
    hub.handle()
        .publish_wait(Event::new(
            "fixture.old",
            "fixture",
            json!("must not arrive"),
        ))
        .await
        .unwrap();
    hub.handle()
        .publish_wait(Event::new("fixture.new", "fixture", json!("new topic")))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "fixture.new");
    client.close();
    client.disconnected().await;
    assert!(
        client
            .call("fixture.barrier", json!(null))
            .await
            .unwrap_err()
            .to_string()
            .contains("not submitted")
    );
    hub.shutdown().unwrap();
}

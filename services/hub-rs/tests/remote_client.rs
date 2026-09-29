use serde_json::json;
use std::{collections::BTreeSet, time::Duration};
use workspacer_hub::{Hub, Options, client::Client, protocol::Event};

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

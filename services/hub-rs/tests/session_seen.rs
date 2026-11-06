//! The hub's shared finished-turn marks over the real bus: an observed turn
//! end marks a session, `sessionSeen.mark` clears it for every client with a
//! `sessionSeen.changed` event, and the marks survive a restart.
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{Hub, Options, client::Client, protocol::Event};

fn options(dir: &std::path::Path) -> Options {
    let mut options = Options::default();
    options.data_dir = Some(dir.join("hub"));
    options.config_dir = Some(dir.join("config"));
    options
}

fn snapshot(id: &str, mode: &str) -> Event {
    Event::new(
        "agent.snapshot",
        "brain",
        json!({"sessionId":id,"session_id":id,"mode":mode,"status":"active"}),
    )
}

/// The observer subscribes asynchronously after `ready()`, so the working
/// row is re-sent until the open set has seen it; only then does the idle row
/// read as a transition.
async fn finish_turn(handle: &workspacer_hub::Handle, client: &Client, id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            handle
                .publish_wait(snapshot(id, "responding"))
                .await
                .unwrap();
            let open = client.call("sessionOpen.get", json!({})).await.unwrap();
            if open["open"].get(id).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        handle.publish_wait(snapshot(id, "input")).await.unwrap();
        loop {
            let doc = client.call("sessionSeen.get", json!({})).await.unwrap();
            if doc["unseen"].get(id).is_some() {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the finished turn was marked")
}

#[tokio::test]
async fn finished_turns_are_marked_cleared_by_any_client_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(options(dir.path())).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    let client = Client::connect(&handle).await.unwrap();
    let mut events = client.events();
    client
        .topics(["sessionSeen.changed".into()].into())
        .await
        .unwrap();
    assert_eq!(
        client.call("sessionSeen.get", json!({})).await.unwrap(),
        json!({"version":0,"unseen":{}})
    );

    // A row already idle when first seen marks nothing.
    handle
        .publish_wait(snapshot("idle", "input"))
        .await
        .unwrap();
    let doc = finish_turn(&handle, &client, "worked").await;
    assert!(doc["unseen"].get("idle").is_none());
    assert!(doc["unseen"]["worked"].is_i64());
    let changed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.topic == "sessionSeen.changed" {
                return event.data.unwrap();
            }
        }
    })
    .await
    .unwrap();
    assert!(changed["unseen"]["worked"].is_i64());

    // Another client opens it: the mark goes, for everyone.
    let other = Client::connect(&handle).await.unwrap();
    let doc = other
        .call("sessionSeen.mark", json!({"sessionId":"worked"}))
        .await
        .unwrap();
    assert!(doc["unseen"].get("worked").is_none());
    assert!(
        other
            .call("sessionSeen.mark", json!({"sessionId":""}))
            .await
            .is_err()
    );

    // A second finished turn survives a restart.
    finish_turn(&handle, &client, "later").await;
    drop((client, other));
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
    let hub = Hub::start(options(dir.path())).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let doc = client.call("sessionSeen.get", json!({})).await.unwrap();
    assert!(doc["unseen"]["later"].is_i64());
    assert!(doc["unseen"].get("worked").is_none());
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
}

//! The hub's shared sidebar set over the real bus (docs/session-roster.md):
//! observed sessions join it, an observed end marks rather than removes,
//! Hide (archive) removes and restore returns, `sessions.kept` is served from
//! it, and it survives a restart.
use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{Hub, Options, client::Client, protocol::Event};

fn options(dir: &std::path::Path) -> Options {
    let mut options = Options::default();
    options.data_dir = Some(dir.join("hub"));
    options.config_dir = Some(dir.join("config"));
    options
}

async fn until_doc(client: &Client, ok: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let doc = client.call("sessionOpen.get", json!({})).await.unwrap();
            if ok(&doc) {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("sessionOpen document converged")
}

/// The hub's observer subscribes asynchronously after `ready()` (in
/// production it then re-reads the roster; these tests have no provider), so
/// the first snapshot is re-sent until the observer has seen it.
async fn publish_until_observed(
    handle: &workspacer_hub::Handle,
    client: &Client,
    event: Event,
    ok: impl Fn(&Value) -> bool,
) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            handle.publish_wait(event.clone()).await.unwrap();
            let doc = client.call("sessionOpen.get", json!({})).await.unwrap();
            if ok(&doc) {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("observer saw the snapshot")
}

fn snapshot(id: &str, mode: &str) -> Event {
    Event::new(
        "agent.snapshot",
        "brain",
        json!({"sessionId":id,"session_id":id,"mode":mode,"status":if mode=="stopped"{"ended"}else{"active"}}),
    )
}

#[tokio::test]
async fn sidebar_set_is_hub_maintained_shared_and_persistent() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::start(options(dir.path())).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    let client = Client::connect(&handle).await.unwrap();
    let mut events = client.events();
    client
        .topics(["sessionOpen.changed".into()].into())
        .await
        .unwrap();
    let doc = until_doc(&client, |d| d["version"].is_i64()).await;
    assert_eq!(doc["open"], json!({}));
    assert_eq!(doc["ended"], json!({}));

    // Desktop main (or any provider) publishes rows; the hub observes them.
    let doc = publish_until_observed(&handle, &client, snapshot("live-1", "input"), |d| {
        d["open"].get("live-1").is_some()
    })
    .await;
    // The change was announced with the whole document.
    let announced = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.topic == "sessionOpen.changed" {
                return event.data.unwrap();
            }
        }
    })
    .await
    .unwrap();
    assert!(announced["open"].get("live-1").is_some());
    assert!(doc["version"].as_i64().unwrap() >= 1);
    assert_eq!(
        client.call("sessions.kept", json!({})).await.unwrap()["sessions"][0]["sessionId"],
        "live-1"
    );

    // Ending on purpose keeps it on the sidebar, marked ended (not Paused).
    handle
        .publish_wait(snapshot("live-1", "stopped"))
        .await
        .unwrap();
    let doc = until_doc(&client, |d| d["ended"].get("live-1").is_some()).await;
    assert!(doc["open"].get("live-1").is_some());
    assert_eq!(
        client.call("sessions.kept", json!({})).await.unwrap(),
        json!({"sessions":[]})
    );

    // Hide = archive: gone from the sidebar; restore brings it back ended.
    client
        .call(
            "sessionArchive.set",
            json!({"sessionId":"live-1","archived":true}),
        )
        .await
        .unwrap();
    let doc = until_doc(&client, |d| d["open"].get("live-1").is_none()).await;
    assert!(doc["ended"].get("live-1").is_none());
    client
        .call(
            "sessionArchive.set",
            json!({"sessionId":"live-1","archived":false}),
        )
        .await
        .unwrap();
    until_doc(&client, |d| {
        d["open"].get("live-1").is_some() && d["ended"].get("live-1").is_some()
    })
    .await;

    // Low-level pin/unpin.
    let pinned = client
        .call("sessionOpen.set", json!({"sessionId":"pin","open":true}))
        .await
        .unwrap();
    assert!(pinned["open"].get("pin").is_some());
    assert!(
        client
            .call("sessionOpen.set", json!({"sessionId":"pin"}))
            .await
            .is_err()
    );

    // A second session is live when the hub stops: it must stay Paused.
    handle
        .publish_wait(snapshot("paused-1", "responding"))
        .await
        .unwrap();
    let before = until_doc(&client, |d| d["open"].get("paused-1").is_some()).await;
    client.close();
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();

    let hub = Hub::start(options(dir.path())).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let after = until_doc(&client, |d| d["version"].is_i64()).await;
    assert_eq!(after["open"], before["open"]);
    assert_eq!(after["ended"], before["ended"]);
    let kept: Vec<_> = client.call("sessions.kept", json!({})).await.unwrap()["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["sessionId"].as_str().unwrap().to_owned())
        .collect();
    assert!(kept.contains(&"paused-1".to_owned()));
    assert!(!kept.contains(&"live-1".to_owned()));
    // Seen stopped for the first time in this run: still Paused, not ended.
    // (A probe session proves the observer is subscribed before we judge.)
    publish_until_observed(&hub.handle(), &client, snapshot("probe", "input"), |d| {
        d["open"].get("probe").is_some()
    })
    .await;
    hub.handle()
        .publish_wait(snapshot("paused-1", "stopped"))
        .await
        .unwrap();
    hub.handle()
        .publish_wait(snapshot("probe", "stopped"))
        .await
        .unwrap();
    let doc = until_doc(&client, |d| d["ended"].get("probe").is_some()).await;
    assert!(doc["ended"].get("paused-1").is_none());
    client.close();
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn view_reads_the_sidebar_and_triage_may_pin_snooze_clean_up_and_bulk_hide() {
    use workspacer_hub::auth::{self, Scope};
    let dir = tempfile::tempdir().unwrap();
    let mut options = options(dir.path());
    options.scoped_tokens = Some(dir.path().join("tokens.json"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let tokens = dir.path().join("tokens.json");
    for scope in [Scope::View, Scope::Triage, Scope::Provider] {
        let record = auth::mint(&tokens, scope, "sidebar-fixture").unwrap();
        let client = Client::from_connection(
            hub.handle()
                .connect_authenticated(record.token, false)
                .await
                .unwrap(),
        );
        let reads = [
            client.call("sessionOpen.get", json!({})).await.is_ok(),
            client.call("sessions.kept", json!({})).await.is_ok(),
        ];
        let writes = [
            client
                .call(
                    "sessionOpen.set",
                    json!({"sessionId":format!("pin-{}", scope.name()),"open":true}),
                )
                .await
                .is_ok(),
            client
                .call("sessionOpen.snoozeCleanup", json!({}))
                .await
                .is_ok(),
            client
                .call("sessionOpen.cleanup", json!({"dryRun":true}))
                .await
                .is_ok(),
            client
                .call(
                    "sessionArchive.setMany",
                    json!({"sessionIds":[format!("bulk-{}", scope.name())],"archived":true}),
                )
                .await
                .is_ok(),
        ];
        match scope {
            Scope::View => {
                assert_eq!(reads, [true, true]);
                assert_eq!(writes, [false; 4], "view may not change the sidebar");
            }
            Scope::Triage => {
                assert_eq!(reads, [true, true]);
                assert_eq!(writes, [true; 4]);
            }
            _ => {
                assert_eq!(reads, [false, false], "provider holds neither");
                assert_eq!(writes, [false; 4]);
            }
        }
        client.close();
    }
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cleanup_prompt_hides_ended_sessions_in_one_archive_change_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(
        dir.path().join("config/config.yaml"),
        "sessions:\n  cleanupThreshold: 3\n",
    )
    .unwrap();
    let hub = Hub::start(options(dir.path())).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    let client = Client::connect(&handle).await.unwrap();
    let mut events = client.events();
    client
        .topics(["sessionArchive.changed".into()].into())
        .await
        .unwrap();
    publish_until_observed(&handle, &client, snapshot("e1", "input"), |d| {
        d["open"].get("e1").is_some()
    })
    .await;
    for id in ["e1", "e2", "e3"] {
        handle.publish_wait(snapshot(id, "input")).await.unwrap();
        handle.publish_wait(snapshot(id, "stopped")).await.unwrap();
    }
    handle
        .publish_wait(snapshot("running", "responding"))
        .await
        .unwrap();
    let doc = until_doc(&client, |d| d["cleanup"]["ended"] == 3).await;
    assert_eq!(doc["cleanup"]["threshold"], 3);
    assert_eq!(doc["cleanup"]["suggested"], true);

    let dry = client
        .call("sessionOpen.cleanup", json!({"dryRun":true}))
        .await
        .unwrap();
    assert_eq!(dry["wouldHide"].as_array().unwrap().len(), 3);
    assert!(
        client
            .call("sessionOpen.cleanup", json!({"olderThanMs":-1}))
            .await
            .is_err()
    );
    // Nothing is a day old yet.
    let none = client
        .call("sessionOpen.cleanup", json!({"olderThanMs":86_400_000}))
        .await
        .unwrap();
    assert_eq!(none, json!({"hidden":[]}));

    let before = client.call("sessionArchive.get", json!({})).await.unwrap()["version"]
        .as_i64()
        .unwrap();
    let hidden = client.call("sessionOpen.cleanup", json!({})).await.unwrap()["hidden"].clone();
    let mut ids: Vec<_> = hidden
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    assert_eq!(ids, ["e1", "e2", "e3"]);
    let archive = client.call("sessionArchive.get", json!({})).await.unwrap();
    assert_eq!(archive["version"].as_i64().unwrap(), before + 1, "one bump");
    let mut changes = 0;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(200), events.recv()).await
    {
        if event.topic == "sessionArchive.changed" {
            changes += 1;
        }
    }
    assert_eq!(changes, 1, "one sessionArchive.changed");
    let doc = client.call("sessionOpen.get", json!({})).await.unwrap();
    assert!(doc["open"].get("e1").is_none());
    assert!(doc["open"].get("running").is_some(), "live never hidden");
    assert_eq!(doc["cleanup"]["ended"], 0);
    assert_eq!(doc["cleanup"]["suggested"], false);

    // Undo: one bulk restore puts them back on the sidebar, marked ended.
    let restored = client
        .call(
            "sessionArchive.setMany",
            json!({"sessionIds":hidden,"archived":false}),
        )
        .await
        .unwrap();
    assert_eq!(restored["version"].as_i64().unwrap(), before + 2);
    let doc = client.call("sessionOpen.get", json!({})).await.unwrap();
    for id in ["e1", "e2", "e3"] {
        assert!(doc["open"].get(id).is_some() && doc["ended"].get(id).is_some());
    }
    assert_eq!(doc["cleanup"]["ended"], 3);

    // Snooze: the prompt waits for ten more.
    let snoozed = client
        .call("sessionOpen.snoozeCleanup", json!({}))
        .await
        .unwrap();
    assert_eq!(snoozed["cleanup"]["snoozedAt"], 3);
    assert_eq!(snoozed["cleanup"]["suggested"], false);
    client.close();
    tokio::task::spawn_blocking(move || hub.shutdown())
        .await
        .unwrap()
        .unwrap();
}

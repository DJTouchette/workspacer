//! The hub's shared session archive over the real bus: one client archives,
//! every subscriber hears it, a restart keeps it, and nothing about the
//! session's lifecycle is touched along the way.
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use workspacer_hub::{
    Hub, Options,
    auth::{self, Scope},
    client::Client,
};

/// Lifecycle methods an archive must never reach. Each is an inert stand-in
/// that records the call, so "archive stopped or deleted it" fails loudly.
const LIFECYCLE: &[&str] = &[
    "agents.close",
    "claude.signal",
    "sessions.delete",
    "claude.gate",
];

fn options(dir: &std::path::Path, calls: &Arc<Mutex<Vec<String>>>) -> Options {
    let mut options = Options::default();
    options.data_dir = Some(dir.join("hub"));
    options.config_dir = Some(dir.join("config"));
    options.scoped_tokens = Some(dir.join("tokens.json"));
    for &method in LIFECYCLE {
        let calls = calls.clone();
        options = options.handler(method, move |_, _| {
            calls.lock().unwrap().push(method.to_string());
            async { Ok(json!({})) }
        });
    }
    options
}

async fn next_change(
    events: &mut tokio::sync::broadcast::Receiver<workspacer_hub::protocol::Event>,
) -> Value {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("sessionArchive.changed in time")
            .expect("event stream open");
        if event.topic == "sessionArchive.changed" {
            return event.data.unwrap();
        }
    }
}

#[tokio::test]
async fn archive_is_shared_live_persistent_and_never_a_lifecycle_action() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let hub = Hub::start(options(dir.path(), &calls)).unwrap();
    hub.ready().await.unwrap();

    // Two clients of one hub: "native" archives, "web" only listens.
    let native = Client::connect(&hub.handle()).await.unwrap();
    let web = Client::connect(&hub.handle()).await.unwrap();
    let mut web_events = web.events();
    web.topics(["sessionArchive.changed".into()].into())
        .await
        .unwrap();
    assert_eq!(
        web.call("sessionArchive.get", json!({})).await.unwrap(),
        json!({"version":0,"archived":{}})
    );

    let archived = native
        .call(
            "sessionArchive.set",
            json!({"sessionId":"sess-live","archived":true}),
        )
        .await
        .unwrap();
    assert_eq!(archived["version"], 1);
    assert!(archived["archived"]["sess-live"].as_i64().is_some());
    assert_eq!(next_change(&mut web_events).await, archived);
    assert_eq!(
        web.call("sessionArchive.get", json!({})).await.unwrap(),
        archived
    );

    // Re-archiving is a no-op: same document, no second event.
    assert_eq!(
        native
            .call(
                "sessionArchive.set",
                json!({"sessionId":"sess-live","archived":true}),
            )
            .await
            .unwrap(),
        archived
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), web_events.recv())
            .await
            .is_err()
    );
    assert!(
        native
            .call("sessionArchive.set", json!({"sessionId":"sess-live"}))
            .await
            .is_err()
    );

    // A restart (the web client reloading against a restarted hub) keeps it.
    native.close();
    web.close();
    hub.shutdown().unwrap();
    let hub = Hub::start(options(dir.path(), &calls)).unwrap();
    hub.ready().await.unwrap();
    let web = Client::connect(&hub.handle()).await.unwrap();
    let mut web_events = web.events();
    web.topics(["sessionArchive.changed".into()].into())
        .await
        .unwrap();
    assert_eq!(
        web.call("sessionArchive.get", json!({})).await.unwrap(),
        archived
    );

    // Restore from the web: the id leaves the document and listeners hear it.
    let restored = web
        .call(
            "sessionArchive.set",
            json!({"sessionId":"sess-live","archived":false}),
        )
        .await
        .unwrap();
    assert_eq!(restored, json!({"version":2,"archived":{}}));
    assert_eq!(next_change(&mut web_events).await, restored);

    assert!(
        calls.lock().unwrap().is_empty(),
        "archive reached a lifecycle method: {:?}",
        calls.lock().unwrap()
    );
    web.close();
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn view_tokens_read_the_archive_and_triage_tokens_may_change_it() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let hub = Hub::start(options(dir.path(), &calls)).unwrap();
    hub.ready().await.unwrap();
    let tokens = dir.path().join("tokens.json");
    for scope in [Scope::View, Scope::Triage, Scope::Provider] {
        let record = auth::mint(&tokens, scope, "archive-fixture").unwrap();
        let client = Client::from_connection(
            hub.handle()
                .connect_authenticated(record.token, false)
                .await
                .unwrap(),
        );
        let read = client.call("sessionArchive.get", json!({})).await;
        let write = client
            .call(
                "sessionArchive.set",
                json!({"sessionId":format!("from-{}", scope.name()),"archived":true}),
            )
            .await;
        match scope {
            Scope::View => {
                assert!(read.is_ok());
                assert!(write.is_err(), "view may not archive");
            }
            Scope::Triage => {
                assert!(read.is_ok());
                assert!(write.unwrap()["archived"]["from-triage"].is_i64());
            }
            _ => {
                assert!(read.is_err() && write.is_err(), "provider holds neither");
            }
        }
        client.close();
    }
    assert!(calls.lock().unwrap().is_empty());
    hub.shutdown().unwrap();
}

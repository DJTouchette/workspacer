use serde_json::json;
use std::{collections::BTreeSet, time::Duration};
use workspacer_hub::{Hub, Options, client::Client};

#[tokio::test]
async fn owned_polling_publishes_live_file_changes_and_releases_watches_on_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("appears.txt");
    let mut options = Options::default();
    options.home_dir = Some(directory.path().into());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client
        .topics(BTreeSet::from(["fs.changed".into()]))
        .await
        .unwrap();
    assert_eq!(
        client
            .call("fs.watch", json!({"path":path,"watchId":"fixture"}))
            .await
            .unwrap()["path"],
        json!(path)
    );
    client
        .call("fs.write", json!({"path":path,"contents":"created"}))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "fs.changed");
    assert_eq!(event.data, Some(json!({"path":path,"eventType":"rename"})));
    client
        .call("fs.unwatch", json!({"path":path,"watchId":"fixture"}))
        .await
        .unwrap();
    client
        .call(
            "fs.write",
            json!({"path":path,"contents":"longer change after unwatch"}),
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(650), events.recv())
            .await
            .is_err()
    );
    client.close();
    hub.shutdown().unwrap();
}

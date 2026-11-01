//! Bus-level coverage for the hub-owned `projectGroups.*` methods: installed on
//! a control-plane-only hub with a config directory (the Electron `--hub-only`
//! shape), mutations publish `projectGroups.changed {revision}`, and the store
//! is `<config_dir>/project-groups.json`.
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn project_groups_are_served_on_a_hub_only_host_and_mutations_publish_changes() {
    use workspacer_hub::{Hub, Options, client::Client};
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let mut options = Options::default();
    options.config_dir = Some(config.clone());
    options.control_plane_only = true;
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client
        .topics(["projectGroups.changed".into()].into())
        .await
        .unwrap();

    let empty = client.call("projectGroups.list", json!({})).await.unwrap();
    assert_eq!(empty, json!({"revision": 0, "groups": []}));

    let saved = client
        .call(
            "projectGroups.save",
            json!({"name": "Acme", "folders": ["/code/acme-api/", "/code/acme-web"], "color": "#5b8def", "icon": "A", "revision": 0}),
        )
        .await
        .unwrap();
    assert_eq!(saved["revision"], 1);
    assert_eq!(
        saved["folders"],
        json!(["/code/acme-api", "/code/acme-web"])
    );
    let id = saved["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("p_") && id.len() == 12, "{id}");
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "projectGroups.changed");
    assert_eq!(event.data.unwrap(), json!({"revision": 1}));

    let list = client.call("projectGroups.list", json!({})).await.unwrap();
    assert_eq!(list["revision"], 1);
    assert_eq!(list["groups"][0]["id"], id.as_str());
    assert_eq!(list["groups"][0]["name"], "Acme");
    assert!(config.join("project-groups.json").is_file());

    let conflict = client
        .call(
            "projectGroups.save",
            json!({"id": id, "name": "Renamed", "folders": ["/code/acme-api"], "revision": 0}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(conflict.contains("conflict:"), "{conflict}");
    let taken = client
        .call(
            "projectGroups.save",
            json!({"name": "Other", "folders": ["/code/acme-web"]}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(taken.contains("“Acme”"), "{taken}");

    let removed = client
        .call("projectGroups.remove", json!({"id": id, "revision": 1}))
        .await
        .unwrap();
    assert_eq!(removed, json!({"removed": true, "revision": 2}));
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "projectGroups.changed");
    assert_eq!(event.data.unwrap(), json!({"revision": 2}));
    hub.shutdown().unwrap();
}

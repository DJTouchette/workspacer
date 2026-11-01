//! Bus-level coverage for the hub-owned `scratch.*` methods: installed on a
//! control-plane-only hub (the Electron `--hub-only` shape), mutations publish
//! `scratch.changed`, and the directory is the one claudemon's spawn-time
//! `WORKSPACER_SCRATCH` names for the same project.
use serde_json::json;

#[tokio::test]
async fn scratch_is_served_on_a_hub_only_host_and_mutations_publish_changes() {
    use workspacer_hub::{Hub, Options, client::Client};
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = dir.path().join("shop");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let mut options = Options::default();
    options.home_dir = Some(home.clone());
    options.control_plane_only = true;
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client
        .topics(["scratch.changed".into()].into())
        .await
        .unwrap();
    let cwd = project.to_string_lossy().into_owned();

    let saved = client
        .call(
            "scratch.save",
            json!({"cwd": cwd, "path": "orders.sql", "content": "SELECT 1;", "purpose": "check orders", "status": "pending"}),
        )
        .await
        .unwrap();
    assert_eq!(saved["status"], "pending");
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "scratch.changed");
    assert_eq!(event.data.as_ref().unwrap()["key"], saved["project"]["key"]);
    assert_eq!(event.data.as_ref().unwrap()["pending"], 1);
    let pending = client.call("scratch.pending", json!({})).await.unwrap();
    assert_eq!(
        pending["projects"],
        json!([{"key": saved["project"]["key"], "root": saved["project"]["root"], "pending": 1}])
    );

    // The same directory an agent spawned in this project receives.
    let mut env = std::collections::HashMap::new();
    claudemon::scratch::inject_env_with_home(&mut env, &cwd, &home);
    assert_eq!(
        env[claudemon::scratch::ENV],
        saved["project"]["files"].as_str().unwrap()
    );

    let list = client
        .call("scratch.list", json!({"cwd": cwd}))
        .await
        .unwrap();
    assert_eq!(list["items"][0]["purpose"], "check orders");
    let read = client
        .call("scratch.read", json!({"cwd": cwd, "path": "orders.sql"}))
        .await
        .unwrap();
    assert_eq!(read["content"], "SELECT 1;");
    client
        .call(
            "scratch.mark",
            json!({"cwd": cwd, "path": "orders.sql", "status": "done", "note": "ran on prod"}),
        )
        .await
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "scratch.changed");
    assert_eq!(
        event.data.unwrap(),
        json!({"key": saved["project"]["key"], "pending": 0})
    );
    let pending = client.call("scratch.pending", json!({})).await.unwrap();
    assert_eq!(pending["projects"], json!([]));
    assert!(
        client
            .call(
                "scratch.save",
                json!({"cwd": cwd, "path": "../escape", "content": "x", "purpose": "p"})
            )
            .await
            .is_err()
    );
    hub.shutdown().unwrap();
}

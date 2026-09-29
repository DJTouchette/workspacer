use serde_json::{Value, json};
use std::time::Duration;
use workspacer_hub::{
    Caller, Hub, Options,
    client::Client,
    services::{
        layout::{Layout, scrub_boot_document},
        usage_prefs::UsagePrefs,
    },
};

fn caller(trusted: bool) -> Caller {
    Caller {
        call_id: 0,
        activity_seq: 0,
        federated: false,
        connection_id: 1,
        authenticated_host: trusted,
        trusted,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: String::new(),
    }
}

#[tokio::test]
async fn local_service_calls_events_and_persistence_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.data_dir = Some(dir.path().join("hub"));
    options.config_dir = Some(dir.path().join("config"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client
        .topics(["layout.changed".into()].into())
        .await
        .unwrap();
    let saved = client
        .call(
            "layout.set",
            json!({"data":{"agents":[],"url":"http://local/?busToken=secret&other=keep"}}),
        )
        .await
        .unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["data"]["url"], "http://local/?busToken=&other=keep");
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "layout.changed");
    assert_eq!(event.data, Some(saved.clone()));
    assert_eq!(
        client
            .call("usage.setPacingSchedule", json!({"schedule":" FIVE_DAY "}))
            .await
            .unwrap()["schedule"],
        "five_day"
    );
    for params in [
        json!({"schedule":"seven_day"}),
        json!({"curve":"calendar"}),
        json!({"url":"http://other"}),
    ] {
        assert!(
            client
                .call("usage.report", params)
                .await
                .unwrap_err()
                .to_string()
                .contains("no parameters accepted")
        );
    }
    client
        .call("config.save", json!({"ui":{"theme":"persisted"}}))
        .await
        .unwrap();
    let owner_saved = client
        .call(
            "desktop.saveConfig",
            json!({"partial":{"ui":{"fontSize":21}}}),
        )
        .await
        .unwrap();
    assert_eq!(owner_saved["ui"]["fontSize"], 21);
    assert!(
        client
            .call("desktop.saveConfig", json!({"ui":{"fontSize":23}}))
            .await
            .is_err()
    );
    hub.shutdown().unwrap();
    assert!(client.call("layout.get", json!({})).await.is_err());
    let mut options = Options::default();
    options.data_dir = Some(dir.path().join("hub"));
    options.config_dir = Some(dir.path().join("config"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    assert_eq!(client.call("layout.get", json!({})).await.unwrap(), saved);
    assert_eq!(
        client
            .call("usage.pacingSchedule", json!({}))
            .await
            .unwrap()["schedule"],
        "five_day"
    );
    assert_eq!(
        client.call("config.get", json!({})).await.unwrap()["ui"]["theme"],
        "persisted"
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn layout_versions_serialize_concurrent_writers_and_do_not_persist_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.json");
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let layout = std::sync::Arc::new(Layout::open(Some(path.clone()), hub.handle()));
    let mut threads = Vec::new();
    for n in 0..32 {
        let layout = layout.clone();
        threads.push(std::thread::spawn(move || {
            layout.set(&caller(true), json!({"data":{"n":n}})).unwrap()
        }));
    }
    let mut versions: Vec<_> = threads
        .into_iter()
        .map(|t| t.join().unwrap()["version"].as_i64().unwrap())
        .collect();
    versions.sort();
    assert_eq!(versions, (1..=32).collect::<Vec<_>>());
    let disk: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(disk["version"], 32);
    assert_eq!(disk, layout.get());
    hub.shutdown().unwrap();
}

#[test]
fn non_host_layout_scrubs_only_execution_bearing_agent_and_pane_fields() {
    let mut data = json!({"agents":[{"id":"a","skipPermissions":true,"launchIntegrationId":"p","tabs":[{"panes":[{"type":"terminal","shell":"evil","initialCommand":"command","title":"keep"}]}]}],"title":"shell"});
    scrub_boot_document(&mut data);
    assert!(data["agents"][0].get("skipPermissions").is_none());
    assert!(data["agents"][0].get("launchIntegrationId").is_none());
    assert_eq!(
        data["agents"][0]["tabs"][0]["panes"][0],
        json!({"type":"terminal","title":"keep"})
    );
    assert_eq!(
        data["agents"][0]["escalationScrubbed"],
        json!(["skipPermissions", "launchIntegrationId", "pane", "pane"])
    );
    assert_eq!(data["title"], "shell");
}

#[test]
fn preference_write_failure_does_not_adopt_a_value_that_will_disappear_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage-pacing.json");
    let prefs = UsagePrefs::open(Some(path.clone()));
    prefs
        .set(&caller(true), json!({"schedule":"five_day"}))
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        prefs
            .set(&caller(true), json!({"schedule":"seven_day"}))
            .is_err()
    );
    assert_eq!(prefs.get(Value::Null).unwrap()["schedule"], "five_day");
    assert!(
        prefs
            .set(&caller(false), json!({"schedule":"seven_day"}))
            .is_err()
    );
    assert!(
        prefs
            .set(&caller(true), json!({"schedule":"nonsense"}))
            .is_err()
    );
}

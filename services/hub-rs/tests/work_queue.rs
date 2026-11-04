//! Host work queue over the bus: docs/host-work-queue.md.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, time::Duration};
use workspacer_hub::{Hub, Options, client::Client};

/// A gate this machine always passes, whatever its memory and disk.
fn open_gate(extra: Value) -> Value {
    let mut config = json!({"maxCpuPressure":100,"maxMemoryPressure":100,"minAvailableMemoryMb":0,"minFreeDiskMb":0});
    for (key, value) in extra.as_object().unwrap() {
        config[key] = value.clone();
    }
    config
}
fn start(path: &Path) -> Hub {
    let mut options = Options::default();
    options.work_queue_file = Some(path.into());
    Hub::start(options).unwrap()
}
fn shell(command: &str) -> Value {
    json!({"kind":"shell","shell":{"command":command}})
}
async fn item(client: &Client, id: &Value) -> Value {
    let list = client.call("queue.list", json!({})).await.unwrap();
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| &item["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}
async fn wait_state(client: &Client, id: &Value, state: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let row = item(client, id).await;
            if row["state"] == state {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("item {id} never became {state}"))
}

#[tokio::test]
async fn shell_items_run_to_completion_with_exit_code_and_output_tail() {
    let directory = tempfile::tempdir().unwrap();
    let hub = start(&directory.path().join("queue.json"));
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client
        .topics(BTreeSet::from(["queue.changed".to_owned()]))
        .await
        .unwrap();
    client
        .call("queue.setConfig", open_gate(json!({})))
        .await
        .unwrap();
    let ok = client
        .call("queue.submit", shell("echo first; echo hello"))
        .await
        .unwrap();
    assert_eq!(ok["item"]["state"], "queued");
    assert_eq!(ok["item"]["source"]["kind"], "prompt");
    assert!(ok["host"].as_str().is_some_and(|h| !h.is_empty()));
    let done = wait_state(&client, &ok["item"]["id"], "done").await;
    assert_eq!(done["exitCode"], 0);
    assert!(done["detail"].as_str().unwrap().ends_with("hello"));
    assert!(done["startedAt"].as_i64().unwrap() >= done["enqueuedAt"].as_i64().unwrap());
    assert!(done["finishedAt"].as_i64().unwrap() >= done["startedAt"].as_i64().unwrap());
    let failing = client
        .call(
            "queue.submit",
            json!({"kind":"shell","label":"fails","shell":{"command":"echo broken >&2; exit 3","cwd":directory.path()}}),
        )
        .await
        .unwrap();
    let failed = wait_state(&client, &failing["item"]["id"], "failed").await;
    assert_eq!(failed["exitCode"], 3);
    assert_eq!(failed["detail"], "broken");
    assert_eq!(failed["label"], "fails");
    // Every transition was published.
    let mut seen = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(200), events.recv()).await
    {
        if event.topic == "queue.changed"
            && event.data.as_ref().unwrap()["item"]["id"] == ok["item"]["id"]
        {
            seen.push(
                event.data.unwrap()["item"]["state"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
    }
    assert_eq!(seen, ["queued", "running", "done"]);
    let resources = client.call("host.resources", json!({})).await.unwrap();
    assert_eq!(resources["queue"]["shellSlots"], 2);
    assert_eq!(resources["queue"]["shellRunning"], 0);
    assert_eq!(resources["queue"]["gate"]["open"], true);
    assert_eq!(resources["host"], ok["host"]);
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn shell_slots_hold_the_second_item_until_the_first_exits() {
    let directory = tempfile::tempdir().unwrap();
    let hub = start(&directory.path().join("queue.json"));
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let config = client
        .call("queue.setConfig", open_gate(json!({"maxShell":1})))
        .await
        .unwrap();
    assert_eq!(config["maxShell"], 1);
    assert_eq!(
        client.call("queue.config", json!({})).await.unwrap(),
        config
    );
    let release = directory.path().join("release");
    let blocker = client
        .call(
            "queue.submit",
            shell(&format!(
                "while [ ! -f '{}' ]; do sleep 0.05; done; echo released",
                release.display()
            )),
        )
        .await
        .unwrap();
    wait_state(&client, &blocker["item"]["id"], "running").await;
    let second = client
        .call("queue.submit", shell("echo second"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let waiting = item(&client, &second["item"]["id"]).await;
    assert_eq!(waiting["state"], "queued");
    assert_eq!(waiting["waitReason"], "no free shell slot (1/1 running)");
    let resources = client.call("host.resources", json!({})).await.unwrap();
    assert_eq!(resources["queue"]["shellRunning"], 1);
    assert_eq!(resources["queue"]["queued"], 1);
    // A running item cannot be cancelled.
    let refused = client
        .call("queue.cancel", json!({"id":blocker["item"]["id"]}))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("already running"), "{refused}");
    std::fs::write(&release, b"").unwrap();
    assert_eq!(
        wait_state(&client, &blocker["item"]["id"], "done").await["detail"],
        "released"
    );
    let second = wait_state(&client, &second["item"]["id"], "done").await;
    assert_eq!(second["waitReason"], "");
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn queued_items_cancel_and_stay_cancelled() {
    let directory = tempfile::tempdir().unwrap();
    let hub = start(&directory.path().join("queue.json"));
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    // No shell slots: everything waits.
    client
        .call("queue.setConfig", open_gate(json!({"maxShell":0})))
        .await
        .unwrap();
    let marker = directory.path().join("ran");
    let queued = client
        .call(
            "queue.submit",
            shell(&format!("touch '{}'", marker.display())),
        )
        .await
        .unwrap();
    let cancelled = client
        .call("queue.cancel", json!({"id":queued["item"]["id"]}))
        .await
        .unwrap();
    assert_eq!(cancelled["ok"], true);
    assert_eq!(cancelled["item"]["state"], "cancelled");
    assert!(
        client
            .call("queue.cancel", json!({"id":queued["item"]["id"]}))
            .await
            .is_err()
    );
    assert!(
        client
            .call("queue.cancel", json!({"id":"q_missing"}))
            .await
            .is_err()
    );
    client
        .call("queue.setConfig", json!({"maxShell":2}))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!marker.exists(), "a cancelled item never runs");
    assert_eq!(
        item(&client, &queued["item"]["id"]).await["state"],
        "cancelled"
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn a_restart_marks_running_items_unknown_and_keeps_queued_ones() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("queue.json");
    let hub = start(&path);
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    client
        .call("queue.setConfig", open_gate(json!({"maxShell":1})))
        .await
        .unwrap();
    let running = client
        .call("queue.submit", shell("sleep 30"))
        .await
        .unwrap();
    wait_state(&client, &running["item"]["id"], "running").await;
    let queued = client
        .call("queue.submit", shell("echo later"))
        .await
        .unwrap();
    drop(client);
    hub.shutdown().unwrap();

    let hub = start(&path);
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    // The fresh hub has default limits; hold shell work so the queued item
    // is still observable as queued.
    client
        .call("queue.setConfig", open_gate(json!({"maxShell":0})))
        .await
        .unwrap();
    let unknown = item(&client, &running["item"]["id"]).await;
    assert_eq!(unknown["state"], "unknown");
    assert_eq!(unknown["detail"], "hub restarted while running");
    assert!(unknown["finishedAt"].as_i64().unwrap() > 0);
    assert_eq!(
        item(&client, &queued["item"]["id"]).await["state"],
        "queued"
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn only_the_owner_submits_shell_work_or_changes_limits() {
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let operator = workspacer_hub::auth::mint(
        &tokens,
        workspacer_hub::auth::Scope::Operator,
        "remote operator",
    )
    .unwrap();
    let view =
        workspacer_hub::auth::mint(&tokens, workspacer_hub::auth::Scope::View, "viewer").unwrap();
    let mut options = Options::default();
    options.work_queue_file = Some(directory.path().join("queue.json"));
    options.scoped_tokens = Some(tokens);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let connect = |token: String| {
        let handle = hub.handle();
        async move { Client::from_connection(handle.connect_authenticated(token, false).await.unwrap()) }
    };
    let operator = connect(operator.token).await;
    let refused = operator
        .call("queue.submit", shell("echo nope"))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("requires the server owner"),
        "{refused}"
    );
    assert!(
        operator
            .call("queue.setConfig", json!({"maxShell":9}))
            .await
            .is_err()
    );
    // Reads are open to any authenticated caller, including view tokens.
    let viewer = connect(view.token).await;
    for method in ["queue.list", "queue.config", "host.resources"] {
        viewer.call(method, json!({})).await.unwrap();
    }
    assert!(
        viewer
            .call("queue.submit", shell("echo nope"))
            .await
            .is_err()
    );
    assert!(
        viewer
            .call(
                "queue.submit",
                json!({"kind":"spawn","spawn":{"cwd":directory.path(),"prompt":"go"}})
            )
            .await
            .is_err()
    );
    let owner = Client::connect(&hub.handle()).await.unwrap();
    assert_eq!(
        owner.call("queue.list", json!({})).await.unwrap()["items"],
        json!([])
    );
    assert_eq!(
        owner.call("queue.config", json!({})).await.unwrap()["maxShell"],
        2
    );
    hub.shutdown().unwrap();
}

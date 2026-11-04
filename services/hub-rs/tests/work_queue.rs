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
    let link = json!({"localPath":directory.path(),"peer":"mac","peerPath":"/Users/dj/app"});
    for method in ["projects.setLink", "projects.removeLink"] {
        let mut params = link.clone();
        if method == "projects.removeLink" {
            params.as_object_mut().unwrap().remove("peerPath");
        }
        let refused = operator.call(method, params).await.unwrap_err();
        assert!(
            refused.to_string().contains("requires the server owner"),
            "{refused}"
        );
    }
    // Reads are open to any authenticated caller, including view tokens.
    let viewer = connect(view.token).await;
    for method in [
        "queue.list",
        "queue.config",
        "host.resources",
        "projects.links",
        "projects.linkMatrix",
    ] {
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

/// A remote hub serving the queue with its own host token, and a local hub
/// linked to it three times: `box` (host token, dispatch), `ops` (an operator
/// token, dispatch) and `quiet` (host token, dispatch off).
struct TwoHubs {
    local: Hub,
    remote: Hub,
    owner: Client,
    remote_owner: Client,
    _dir: tempfile::TempDir,
}
async fn two_hubs() -> TwoHubs {
    two_hubs_advertising(None).await
}
/// `two_hubs`, with the remote answering `fleet.dispatchCapabilities` with
/// these directory choices (a hub without an execution host has none).
async fn two_hubs_advertising(cwds: Option<Value>) -> TwoHubs {
    use workspacer_hub::{
        auth::{self, Scope},
        federation::Peer,
    };
    let dir = tempfile::tempdir().unwrap();
    let tokens = dir.path().join("tokens.json");
    let operator = auth::mint(&tokens, Scope::Operator, "paired operator").unwrap();
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "remote-host-token".into();
    options.scoped_tokens = Some(tokens);
    std::fs::create_dir(dir.path().join("remote")).unwrap();
    options.work_queue_file = Some(dir.path().join("remote/queue.json"));
    if let Some(cwds) = cwds {
        options = options.handler("fleet.dispatchCapabilities", move |_, _| {
            let cwds = cwds.clone();
            async move {
                Ok(
                    json!({"protocol":2,"exactModel":true,"executes":true,"scope":"operator",
                    "providers":[],"cwds":cwds}),
                )
            }
        });
    }
    let remote = Hub::start(options).unwrap();
    let address = remote.ready().await.unwrap().unwrap();
    let remote_owner = Client::connect(&remote.handle()).await.unwrap();
    remote_owner
        .call("queue.setConfig", open_gate(json!({})))
        .await
        .unwrap();
    let peer = |name: &str, token: &str, dispatch: bool| Peer {
        name: name.into(),
        url: format!("ws://{address}/bus"),
        token: token.into(),
        dispatch,
    };
    let mut options = Options::default();
    std::fs::create_dir(dir.path().join("local")).unwrap();
    options.work_queue_file = Some(dir.path().join("local/queue.json"));
    options.federation_peers = vec![
        peer("box", "remote-host-token", true),
        peer("ops", &operator.token, true),
        peer("quiet", "remote-host-token", false),
    ];
    let local = Hub::start(options).unwrap();
    local.ready().await.unwrap();
    let owner = Client::connect(&local.handle()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let peers = owner.call("federation.peers", json!({})).await.unwrap();
            if peers
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["connected"] == true)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    TwoHubs {
        local,
        remote,
        owner,
        remote_owner,
        _dir: dir,
    }
}

#[tokio::test]
async fn shell_work_lands_on_a_host_token_peer_and_runs_there() {
    let hubs = two_hubs().await;
    let owner = &hubs.owner;
    owner
        .call("queue.setConfig", open_gate(json!({})))
        .await
        .unwrap();
    let watcher = Client::connect(&hubs.local.handle()).await.unwrap();
    let mut events = watcher.events();
    watcher
        .topics(BTreeSet::from(["queue.changed".to_owned()]))
        .await
        .unwrap();
    let mut job = shell("echo ran on the peer");
    job["host"] = json!("box");
    job["label"] = json!("remote shell");
    let landed = owner.call("queue.submit", job).await.unwrap();
    assert_eq!(landed["host"], "box");
    assert_eq!(landed["item"]["source"]["kind"], "remote");
    assert!(
        landed["item"]["source"]["origin"]
            .as_str()
            .is_some_and(|o| !o.is_empty())
    );
    let id = landed["item"]["id"].clone();
    // It runs on the peer, not here.
    let done = wait_state(&hubs.remote_owner, &id, "done").await;
    assert_eq!(done["detail"], "ran on the peer");
    assert!(done["updatedAt"].as_i64().unwrap() >= done["finishedAt"].as_i64().unwrap());
    assert_eq!(item(owner, &id).await, Value::Null);
    // Its events reach this hub under the configured peer name.
    let forwarded = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.data.as_ref().unwrap()["item"]["id"] == id {
                return event;
            }
        }
    })
    .await
    .unwrap();
    assert!(["box", "ops", "quiet"].contains(&forwarded.hub.as_str()));
    // queue.list fans out: one row per configured peer, by its name.
    let list = owner
        .call("queue.list", json!({"includePeers":true}))
        .await
        .unwrap();
    let peers = list["peers"].as_array().unwrap();
    let names: Vec<_> = peers.iter().map(|p| p["peer"].as_str().unwrap()).collect();
    assert_eq!(names, ["box", "ops", "quiet"]);
    let box_row = &peers[0];
    assert_eq!(box_row["ok"], true);
    assert!(
        box_row["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == id)
    );
    assert_eq!(
        owner.call("queue.list", json!({})).await.unwrap()["peers"],
        json!([])
    );
    // An operator-token pairing is not the host: shell is refused there,
    // and the refusal is a definite one.
    let mut refused = shell("echo nope");
    refused["host"] = json!("ops");
    let error = owner
        .call("queue.submit", refused)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("peer ops refused"), "{error}");
    assert!(error.contains("host credential"), "{error}");
    // Peers that are not dispatch-enabled or not configured are refused here.
    for (host, expected) in [
        ("quiet", "not dispatch-enabled"),
        ("nowhere", "unknown federation peer"),
    ] {
        let mut attempt = shell("echo nope");
        attempt["host"] = json!(host);
        let error = owner
            .call("queue.submit", attempt)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{host}: {error}");
    }
    // Remote cancel goes to the peer with this hub's credential.
    hubs.remote_owner
        .call("queue.setConfig", json!({"maxShell":0}))
        .await
        .unwrap();
    let mut held = shell("echo held");
    held["host"] = json!("box");
    let held = owner.call("queue.submit", held).await.unwrap();
    let cancelled = owner
        .call(
            "queue.cancel",
            json!({"id":held["item"]["id"],"host":"box"}),
        )
        .await
        .unwrap();
    assert_eq!(cancelled["item"]["state"], "cancelled");
    hubs.local.shutdown().unwrap();
    hubs.remote.shutdown().unwrap();
}

#[tokio::test]
async fn any_places_on_the_best_eligible_host_and_explains_it() {
    let hubs = two_hubs().await;
    let owner = &hubs.owner;
    // This machine's gate is open: a tie with the peers goes local.
    owner
        .call("queue.setConfig", open_gate(json!({})))
        .await
        .unwrap();
    let targets = owner
        .call("queue.targets", json!({"kind":"shell"}))
        .await
        .unwrap();
    let rows = targets["targets"].as_array().unwrap();
    let hosts: Vec<_> = rows.iter().map(|t| t["host"].as_str().unwrap()).collect();
    assert_eq!(hosts, ["local", "box", "ops", "quiet"]);
    assert_eq!(targets["best"], "local");
    assert_eq!(rows[3]["eligible"], false);
    assert!(
        rows[3]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("dispatch is not enabled for this peer"))
    );
    assert!(rows[1]["resources"]["queue"]["shellSlots"].is_u64());
    // A cwd only this machine has keeps local eligible; peers do not
    // advertise it, and that does not fail placement.
    let directory = tempfile::tempdir().unwrap();
    let mut here = shell("pwd");
    here["host"] = json!("any");
    here["shell"]["cwd"] = json!(directory.path());
    let landed = owner.call("queue.submit", here).await.unwrap();
    assert_ne!(landed["host"], "box");
    let ran = wait_state(owner, &landed["item"]["id"], "done").await;
    assert!(
        ran["detail"]
            .as_str()
            .unwrap()
            .ends_with(directory.path().file_name().unwrap().to_str().unwrap())
    );
    // Close this machine's gate: the first-named eligible peer wins.
    owner
        .call("queue.setConfig", json!({"minFreeDiskMb":1_073_741_824u64}))
        .await
        .unwrap();
    let targets = owner
        .call("queue.targets", json!({"kind":"shell"}))
        .await
        .unwrap();
    assert_eq!(targets["best"], "box");
    let local_reasons = targets["targets"][0]["reasons"].to_string();
    assert!(local_reasons.contains("gate closed"), "{local_reasons}");
    let mut anywhere = shell("echo placed");
    anywhere["host"] = json!("any");
    let landed = owner.call("queue.submit", anywhere).await.unwrap();
    assert_eq!(landed["host"], "box");
    wait_state(&hubs.remote_owner, &landed["item"]["id"], "done").await;
    hubs.local.shutdown().unwrap();
    hubs.remote.shutdown().unwrap();
}

#[tokio::test]
async fn a_job_with_host_local_enqueues_and_records_where_it_went() {
    let directory = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.work_queue_file = Some(directory.path().join("queue.json"));
    options.jobs_file = Some(directory.path().join("jobs.json"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    client
        .call("queue.setConfig", open_gate(json!({"maxShell":0})))
        .await
        .unwrap();
    let job = client
        .call(
            "jobs.upsert",
            json!({"name":"queued build","enabled":true,"host":"local","priority":3,
                "trigger":{"kind":"manual"},
                "action":{"kind":"shell","shell":{"command":"echo built"}}}),
        )
        .await
        .unwrap();
    assert_eq!(job["host"], "local");
    assert_eq!(job["priority"], 3);
    let history = |client: Client, id: Value| async move {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let runs =
                    client.call("jobs.history", json!({"id":id})).await.unwrap()["runs"].clone();
                if runs.as_array().is_some_and(|r| !r.is_empty()) {
                    return runs;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    };
    client
        .call("jobs.run", json!({"id":job["id"]}))
        .await
        .unwrap();
    let runs = history(client.clone(), job["id"].clone()).await;
    assert_eq!(runs[0]["status"], "ok");
    let detail = runs[0]["detail"].as_str().unwrap();
    let id = detail
        .strip_prefix("queued ")
        .and_then(|rest| rest.strip_suffix(" on local"))
        .unwrap_or_else(|| panic!("{detail}"));
    let queued = item(&client, &json!(id)).await;
    assert_eq!(queued["state"], "queued");
    assert_eq!(queued["priority"], 3);
    assert_eq!(
        queued["source"],
        json!({"kind":"job","jobId":job["id"],"origin":""})
    );
    // While that item waits, the next run is skipped rather than stacked.
    client
        .call("jobs.run", json!({"id":job["id"]}))
        .await
        .unwrap();
    let skipped = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let runs = client
                .call("jobs.history", json!({"id":job["id"]}))
                .await
                .unwrap()["runs"]
                .clone();
            if runs.as_array().unwrap().len() >= 2 {
                return runs[0].clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(skipped["status"], "skipped");
    assert!(
        skipped["detail"].as_str().unwrap().contains(id),
        "{skipped}"
    );
    // Once it runs to the end, the job may queue again.
    client
        .call("queue.setConfig", json!({"maxShell":2}))
        .await
        .unwrap();
    assert_eq!(
        wait_state(&client, &json!(id), "done").await["detail"],
        "built"
    );
    // A call action refuses a host.
    assert!(
        client
            .call(
                "jobs.upsert",
                json!({"name":"bad","host":"local","trigger":{"kind":"manual"},
                    "action":{"kind":"call","call":{"method":"sessions.list"}}}),
            )
            .await
            .is_err()
    );
    hub.shutdown().unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

/// The same project at different paths on two machines: the peer advertises
/// its copy with the same origin, a folder with only the same name, and the
/// owner links the latter by hand (docs/host-work-queue.md, "Project links").
#[tokio::test]
async fn project_links_translate_the_cwd_for_named_and_any_submits() {
    let paths = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(paths.path()).unwrap();
    let local_app = root.join("home/dj/code/app");
    let local_web = root.join("home/dj/web");
    let peer_app = root.join("Users/dj/app");
    let peer_web = root.join("Users/dj/sites/web");
    for dir in [&local_app, &local_web, &peer_app, &peer_web] {
        std::fs::create_dir_all(dir).unwrap();
    }
    git(&local_app, &["init", "-q"]);
    git(
        &local_app,
        &["remote", "add", "origin", "git@GitHub.com:DJ/app.git"],
    );
    let s = |p: &Path| p.to_str().unwrap().to_owned();
    let hubs = two_hubs_advertising(Some(json!([
        {"path": s(&peer_app), "name": "app", "remote": "github.com/DJ/app", "source": "project", "git": true},
        // An older peer's row: no name, no remote.
        {"path": s(&peer_web), "source": "active", "git": false},
    ])))
    .await;
    let owner = &hubs.owner;
    let watcher = Client::connect(&hubs.local.handle()).await.unwrap();
    let mut events = watcher.events();
    watcher
        .topics(BTreeSet::from(["projects.linksChanged".to_owned()]))
        .await
        .unwrap();
    let pwd_in = |cwd: &Path, host: &str| {
        let mut item = shell("pwd");
        item["host"] = json!(host);
        item["shell"]["cwd"] = json!(cwd);
        item
    };

    // Named peer, same origin at another path: the peer's path is used.
    let landed = owner
        .call("queue.submit", pwd_in(&local_app, "box"))
        .await
        .unwrap();
    assert_eq!(landed["host"], "box");
    assert_eq!(landed["item"]["shell"]["cwd"], s(&peer_app));
    assert_eq!(landed["item"]["source"]["originCwd"], s(&local_app));
    let ran = wait_state(&hubs.remote_owner, &landed["item"]["id"], "done").await;
    assert_eq!(ran["detail"], s(&peer_app));

    // queue.targets explains the translation per peer.
    let targets = owner
        .call("queue.targets", json!({"kind":"shell","cwd":local_app}))
        .await
        .unwrap();
    let rows = targets["targets"].as_array().unwrap();
    assert_eq!(rows[0]["host"], "local");
    assert_eq!(rows[0]["resolvedCwd"], s(&local_app));
    assert_eq!(rows[1]["host"], "box");
    assert_eq!(rows[1]["eligible"], true);
    assert_eq!(rows[1]["resolvedCwd"], s(&peer_app));
    assert_eq!(rows[1]["resolvedVia"], "remote");
    assert_eq!(rows[1]["suggestions"], json!([]));
    assert_eq!(rows[1]["cwds"][0]["remote"], "github.com/DJ/app");

    // `any` with this machine's gate closed: lands on a peer, translated.
    owner
        .call(
            "queue.setConfig",
            open_gate(json!({"minFreeDiskMb":1_073_741_824u64})),
        )
        .await
        .unwrap();
    let landed = owner
        .call("queue.submit", pwd_in(&local_app, "any"))
        .await
        .unwrap();
    assert_eq!(landed["host"], "box");
    assert_eq!(landed["item"]["shell"]["cwd"], s(&peer_app));
    assert_eq!(landed["item"]["source"]["originCwd"], s(&local_app));
    wait_state(&hubs.remote_owner, &landed["item"]["id"], "done").await;

    // Only the folder name matches: refused with a suggestion, both named
    // and through placement.
    let expected = format!(
        "cwd {} has no match on box (suggested: {} — link it in Hosts → Projects)",
        s(&local_web),
        s(&peer_web)
    );
    let refused = owner
        .call("queue.submit", pwd_in(&local_web, "box"))
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains(&expected), "{refused}");
    let targets = owner
        .call("queue.targets", json!({"kind":"shell","cwd":local_web}))
        .await
        .unwrap();
    let box_row = &targets["targets"][1];
    assert_eq!(box_row["eligible"], false);
    assert_eq!(box_row["resolvedCwd"], Value::Null);
    assert_eq!(
        box_row["suggestions"],
        json!([{"path": s(&peer_web), "why": "name"}])
    );
    assert!(
        box_row["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!(expected))
    );

    // The owner links it: now it is used, and the change is published.
    let saved = owner
        .call(
            "projects.setLink",
            json!({"localPath": local_web, "peer": "box", "peerPath": peer_web}),
        )
        .await
        .unwrap();
    assert_eq!(saved["links"][0]["localPath"], s(&local_web));
    assert_eq!(saved["links"][0]["peerPath"], s(&peer_web));
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.topic == "projects.linksChanged" {
                return event;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(event.data.unwrap(), saved);
    assert_eq!(
        owner.call("projects.links", json!({})).await.unwrap(),
        saved
    );
    let landed = owner
        .call("queue.submit", pwd_in(&local_web, "box"))
        .await
        .unwrap();
    assert_eq!(landed["item"]["shell"]["cwd"], s(&peer_web));
    assert_eq!(
        wait_state(&hubs.remote_owner, &landed["item"]["id"], "done").await["detail"],
        s(&peer_web)
    );
    let targets = owner
        .call("queue.targets", json!({"kind":"shell","cwd":local_web}))
        .await
        .unwrap();
    assert_eq!(targets["targets"][1]["resolvedVia"], "link");

    // The matrix: the linked local project against every configured peer.
    let matrix = owner.call("projects.linkMatrix", json!({})).await.unwrap();
    let web = matrix["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["path"] == s(&local_web))
        .unwrap_or_else(|| panic!("{matrix}"));
    assert_eq!(web["name"], "web");
    assert_eq!(web["remote"], Value::Null);
    let peers = web["peers"].as_array().unwrap();
    let names: Vec<_> = peers.iter().map(|p| p["peer"].as_str().unwrap()).collect();
    assert_eq!(names, ["box", "ops", "quiet"]);
    assert_eq!(peers[0]["connected"], true);
    assert_eq!(
        peers[0]["resolved"],
        json!({"path": s(&peer_web), "via": "link"})
    );
    assert_eq!(peers[0]["choices"].as_array().unwrap().len(), 2);
    assert_eq!(peers[0]["choices"][1]["name"], "");
    // `ops` has no saved link: name-only, so a suggestion.
    assert_eq!(peers[1]["resolved"], Value::Null);
    assert_eq!(
        peers[1]["suggestions"],
        json!([{"path": s(&peer_web), "why": "name"}])
    );
    let removed = owner
        .call(
            "projects.removeLink",
            json!({"localPath": local_web, "peer": "box"}),
        )
        .await
        .unwrap();
    assert_eq!(removed, json!({"links": []}));
    assert!(
        owner
            .call(
                "projects.setLink",
                json!({"localPath": local_web, "peer": "box", "peerPath": "relative"})
            )
            .await
            .is_err()
    );
    hubs.local.shutdown().unwrap();
    hubs.remote.shutdown().unwrap();
}

/// A peer that cannot list its directories (no execution host) still takes
/// a named submit: the cwd is forwarded as given and the peer decides.
#[tokio::test]
async fn a_peer_without_directory_choices_gets_the_cwd_as_given() {
    let hubs = two_hubs().await;
    let directory = tempfile::tempdir().unwrap();
    let mut item = shell("pwd");
    item["host"] = json!("box");
    item["shell"]["cwd"] = json!(directory.path());
    let landed = hubs.owner.call("queue.submit", item).await.unwrap();
    assert_eq!(landed["item"]["shell"]["cwd"], json!(directory.path()));
    assert_eq!(
        landed["item"]["source"]["originCwd"],
        json!(directory.path())
    );
    wait_state(&hubs.remote_owner, &landed["item"]["id"], "done").await;
    hubs.local.shutdown().unwrap();
    hubs.remote.shutdown().unwrap();
}

use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use workspacer_hub::{Caller, Hub, Options, client::Client, services::layout::Layout};

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
async fn persisted_defaults_and_invalid_documents_preserve_reference_load_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.json");
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    for (input, expected) in [
        (
            json!({"data":{"keep":9007199254740993_u64}}),
            json!({"version":0,"data":{"keep":9007199254740993_u64}}),
        ),
        (
            json!({"version":null,"data":[1,false]}),
            json!({"version":0,"data":[1,false]}),
        ),
        (json!({"version":7}), json!({"version":7,"data":null})),
        (Value::Null, json!({"version":0,"data":null})),
        (
            json!({"version":-2,"data":{"url":"http://h/?busToken=old-secret&keep=1"}}),
            json!({"version":-2,"data":{"url":"http://h/?busToken=&keep=1"}}),
        ),
        (
            json!({"version":"bad","data":"discard"}),
            json!({"version":0,"data":null}),
        ),
        (json!([1, 2]), json!({"version":0,"data":null})),
    ] {
        let bytes = serde_json::to_vec(&input).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let layout = Layout::open(Some(path.clone()), hub.handle());
        assert_eq!(layout.get(), expected, "{input}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "loading must not rewrite disk"
        );
    }
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn authority_scrubbing_roundtrips_opaque_fields_and_broadcasts_only_redacted_state() {
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    client
        .topics(["layout.changed".into()].into())
        .await
        .unwrap();
    let mut events = client.events();
    let layout = Layout::open(None, hub.handle());
    assert_eq!(layout.get(), json!({"version":0,"data":null}));
    assert!(layout.set(&caller(true), json!({})).is_err());
    let data = json!({"activeAgentId":"a","agents":[{"id":"a","name":"kept","cwd":"/","model":"opus","skipPermissions":true,"permissionMode":"plan","profileId":"work","mcpItemIds":["x"],"launchIntegrationId":"plugin","tabs":[{"id":"t","panes":[{"id":"p","type":"terminal","shell":"custom","initialCommand":"echo hi","title":"keep"},{"id":"q","type":"plugin","pluginId":"plugin","url":"https://h/?busToken=secret&other=keep"}]}]}]});
    let trusted = layout.set(&caller(true), json!({"data":data})).unwrap();
    assert_eq!(trusted["data"]["agents"][0]["profileId"], "work");
    assert_eq!(
        trusted["data"]["agents"][0]["tabs"][0]["panes"][0]["shell"],
        "custom"
    );
    let scrubbed = layout.set(&caller(false), json!({"data":data})).unwrap();
    let agent = &scrubbed["data"]["agents"][0];
    for key in [
        "skipPermissions",
        "permissionMode",
        "profileId",
        "mcpItemIds",
        "launchIntegrationId",
    ] {
        assert!(agent.get(key).is_none(), "{key}");
    }
    assert_eq!(
        agent["escalationScrubbed"],
        json!([
            "skipPermissions",
            "permissionMode",
            "profileId",
            "mcpItemIds",
            "launchIntegrationId",
            "pane",
            "pane",
            "pane"
        ])
    );
    assert_eq!(
        agent["tabs"][0]["panes"][0],
        json!({"id":"p","type":"terminal","title":"keep"})
    );
    assert_eq!(
        agent["tabs"][0]["panes"][1],
        json!({"id":"q","type":"plugin","url":"https://h/?busToken=&other=keep"})
    );
    assert_eq!(agent["name"], "kept");
    assert_eq!(agent["cwd"], "/");
    assert_eq!(agent["model"], "opus");
    for expected in [trusted, scrubbed] {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.topic, "layout.changed");
        assert_eq!(event.data, Some(expected));
        assert!(!serde_json::to_string(&event).unwrap().contains("secret"));
    }
    for data in [
        json!({"agents":"not-an-array","x":1}),
        json!([1, 2, 3]),
        Value::Null,
        json!({"globals":{"skipPermissions":true}}),
    ] {
        assert_eq!(
            layout.set(&caller(false), json!({"data":data})).unwrap()["data"],
            data
        );
    }
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn diagnostic_child() {
    let Some(root) = std::env::var_os("WORKSPACER_LAYOUT_DIAGNOSTIC_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let path = root.join("layout.json");
    match std::env::var("WORKSPACER_LAYOUT_DIAGNOSTIC_CASE")
        .unwrap()
        .as_str()
    {
        "missing" => {
            assert_eq!(Layout::open(Some(path), hub.handle()).get()["version"], 0);
        }
        "invalid" => {
            std::fs::write(&path, "{bad json").unwrap();
            assert_eq!(Layout::open(Some(path), hub.handle()).get()["version"], 0);
        }
        "unreadable" => {
            std::fs::create_dir(&path).unwrap();
            assert_eq!(Layout::open(Some(path), hub.handle()).get()["version"], 0);
        }
        "persist" => {
            let layout = Layout::open(Some(path.clone()), hub.handle());
            let client = Client::connect(&hub.handle()).await.unwrap();
            client
                .topics(["layout.changed".into()].into())
                .await
                .unwrap();
            let mut events = client.events();
            let first = layout.set(&caller(true), json!({"data":"old"})).unwrap();
            let backup = root.join("surviving-layout.json");
            std::fs::rename(&path, &backup).unwrap();
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("preserve"), "occupied").unwrap();
            let next = layout
                .set(&caller(true), json!({"data":"live-only"}))
                .unwrap();
            assert_eq!(next["version"], 2);
            assert_eq!(layout.get(), next);
            for expected in [first.clone(), next] {
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(2), events.recv())
                        .await
                        .unwrap()
                        .unwrap()
                        .data,
                    Some(expected)
                );
            }
            assert_eq!(Layout::open(Some(backup), hub.handle()).get(), first);
            assert_eq!(std::fs::read(path.join("preserve")).unwrap(), b"occupied");
            assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".tmp")
            }));
        }
        _ => panic!("unknown fixture"),
    }
    hub.shutdown().unwrap();
}

#[test]
fn disk_failures_are_audible_but_do_not_break_live_layout_sync() {
    for (case, diagnostic) in [
        ("missing", None),
        ("invalid", Some("persisted document is invalid")),
        ("unreadable", Some("could not read persisted document")),
        (
            "persist",
            Some("failed to persist version 2; live changes will be lost on restart"),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "diagnostic_child", "--nocapture"])
            .env("WORKSPACER_LAYOUT_DIAGNOSTIC_FIXTURE", dir.path())
            .env("WORKSPACER_LAYOUT_DIAGNOSTIC_CASE", case)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{case}: {stderr}\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        match diagnostic {
            Some(expected) => assert!(stderr.contains(expected), "{case}: {stderr}"),
            None => assert!(!stderr.contains("layout:"), "{stderr}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_disk_observer_never_sees_regression_or_partial_json() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.json");
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let layout = Arc::new(Layout::open(Some(path.clone()), hub.handle()));
    layout.set(&caller(true), json!({"data":null})).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(Barrier::new(2));
    let reader = {
        let stop = stop.clone();
        let ready = ready.clone();
        let path = path.clone();
        std::thread::spawn(move || {
            let mut version = 0;
            let mut observations = 0;
            ready.wait();
            while !stop.load(Ordering::Acquire) {
                let bytes = std::fs::read(&path).unwrap();
                let value: Value = serde_json::from_slice(&bytes).expect("partial JSON on disk");
                let next = value["version"].as_i64().unwrap();
                assert!(
                    next >= version,
                    "persisted version regressed: {version} -> {next}"
                );
                version = next;
                observations += 1;
                std::thread::yield_now();
            }
            observations
        })
    };
    ready.wait();
    let writers: Vec<_> = (0..8)
        .map(|writer| {
            let layout = layout.clone();
            std::thread::spawn(move || {
                for round in 0..300 {
                    layout
                        .set(
                            &caller(true),
                            json!({"data":{"writer":writer,"round":round}}),
                        )
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    stop.store(true, Ordering::Release);
    assert!(reader.join().unwrap() > 1);
    let disk: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(disk["version"], 2401);
    assert_eq!(disk, layout.get());
    assert_eq!(Layout::open(Some(path), hub.handle()).get(), disk);
    hub.shutdown().unwrap();
}

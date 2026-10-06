#![cfg(feature = "rust-hub")]
//! The Jobs view's reads and owner writes against a real, isolated Rust hub:
//! list, approve a proposed change, pause, run now, history and remove.
use serde_json::json;
use std::{sync::Arc, time::Duration};
use wks_native::{
    controller::{Command, Controller, View},
    features::{JobAction, Request, RequestState},
    host::{Mode, NativeHost, RustOptions},
    jobs,
};

async fn run(controller: &Controller, request: Request) -> RequestState {
    let key = request.key();
    let mut views = controller.views.clone();
    let before = views.borrow().requests.get(key).map_or(0, |s| s.number);
    controller.command(Command::Request(request)).unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let view: Arc<View> = views.borrow_and_update().clone();
            if let Some(state) = view.requests.get(key)
                && !state.loading
                && state.number > before
            {
                return state.clone();
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{key} never completed"))
}

#[tokio::test]
async fn jobs_view_reads_and_owner_writes_round_trip_through_the_hub() {
    let root = std::env::temp_dir().join(format!(
        "wks-native-jobs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut options = RustOptions::isolated(root.join("state")).unwrap();
    options.home_dir = root.join("home");
    std::fs::create_dir(&options.home_dir).unwrap();
    options.usage_poll_on_boot = Some(false);
    // What an agent leaves behind: a job, and a proposed change to it.
    std::fs::create_dir_all(&options.data_dir).unwrap();
    std::fs::write(
        options.data_dir.join("jobs.json"),
        json!({"jobs":[
            {"id":"job","name":"Echo","enabled":true,"trigger":{"kind":"manual"},
             "action":{"kind":"shell","shell":{"command":"echo one"}}},
            {"id":"change","name":"Echo twice","enabled":false,"proposedBy":"helper",
             "replaces":"job","trigger":{"kind":"manual"},
             "action":{"kind":"shell","shell":{"command":"echo two"}}}
        ]})
        .to_string(),
    )
    .unwrap();

    let host = NativeHost::start(Mode::Rust(options)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), host.ready())
        .await
        .unwrap()
        .unwrap();
    let controller = host.controller();
    let mut views = controller.views.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !views.borrow_and_update().connected {
            views.changed().await.unwrap();
        }
    })
    .await
    .unwrap();

    let listed = run(&controller, Request::Jobs).await;
    assert_eq!(listed.error, None);
    let rows = jobs::parse_list(&listed.value);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].change(), "the waiting proposal comes first");

    let approved = run(
        &controller,
        Request::JobAction(JobAction::Save(rows[0].approval())),
    )
    .await;
    assert_eq!(approved.error, None);
    let rows = jobs::parse_list(&approved.value);
    assert_eq!(rows.len(), 1, "the proposal row is consumed");
    assert_eq!(
        (rows[0].id.as_str(), rows[0].name.as_str()),
        ("job", "Echo twice")
    );
    assert_eq!(rows[0].action_summary(), "$ echo two");
    assert!(rows[0].enabled);

    let started = run(
        &controller,
        Request::JobAction(JobAction::Run("job".into())),
    )
    .await;
    assert_eq!(started.error, None);
    let history = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let state = run(&controller, Request::JobHistory { id: "job".into() }).await;
            let runs = jobs::parse_runs(&state.value);
            if !runs.is_empty() {
                return runs;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(history[0].label(), "ok");

    let paused = run(
        &controller,
        Request::JobAction(JobAction::Save(rows[0].with_enabled(false))),
    )
    .await;
    assert!(!jobs::parse_list(&paused.value)[0].enabled);

    let removed = run(
        &controller,
        Request::JobAction(JobAction::Remove("job".into())),
    )
    .await;
    assert!(jobs::parse_list(&removed.value).is_empty());
    host.shutdown().await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

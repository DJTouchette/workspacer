use anyhow::bail;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use workspacer_hub::services::{
    agent_lifecycle::{LaunchEngine, LaunchPreparation, Lifecycle, Operation, Phase},
    spawn_plan,
};

#[derive(Default)]
struct Fake {
    events: Mutex<Vec<String>>,
    live_rows: Mutex<Vec<Value>>,
    fail_spawn: AtomicBool,
    hang_prepare: AtomicBool,
    fail_revoke: AtomicBool,
    pause: tokio::sync::Notify,
    paused: AtomicBool,
}
impl LaunchPreparation for Fake {
    fn prepare<'a>(&'a self, _: &'a mut spawn_plan::Plan, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push("prepare".into());
            if self.hang_prepare.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            Ok(())
        })
    }
    fn revoke<'a>(&'a self, _: &'a str, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push("revoke".into());
            if self.fail_revoke.load(Ordering::SeqCst) {
                bail!("disk unavailable")
            }
            Ok(())
        })
    }
}
impl LaunchEngine for Fake {
    fn sessions(&self) -> Operation<'_, Value> {
        Box::pin(async { Ok(Value::Array(self.live_rows.lock().unwrap().clone())) })
    }
    fn spawn<'a>(&'a self, plan: &'a spawn_plan::Plan) -> Operation<'a, Value> {
        Box::pin(async move {
            self.events.lock().unwrap().push("spawn".into());
            if self.paused.load(Ordering::SeqCst) {
                self.pause.notified().await;
            }
            if self.fail_spawn.load(Ordering::SeqCst) {
                bail!("provider refused")
            }
            Ok(json!({"session_id":plan.session_id,"first_message_queued":true}))
        })
    }
    fn stop<'a>(&'a self, _: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push("stop".into());
            Ok(())
        })
    }
}
fn plan(home: &std::path::Path) -> spawn_plan::Plan {
    spawn_plan::resolve(&json!({"cwd":home,"message":"hello","label":"worker","parentSessionId":"parent","role":"reviewer"}), &json!({}), None, home, "child", false).unwrap()
}
#[tokio::test]
async fn launch_persists_attribution_before_provider_and_survives_waiter_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    fake.paused.store(true, Ordering::SeqCst);
    let journal = dir.path().join("launch.json");
    let service = Lifecycle::open(journal.clone(), fake.clone(), fake.clone()).unwrap();
    let launch = {
        let s = service.clone();
        let p = plan(dir.path());
        tokio::spawn(async move { s.launch(p).await })
    };
    for _ in 0..1000 {
        if fake.events.lock().unwrap().contains(&"spawn".into()) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(service.records()["child"].phase, Phase::Preparing);
    assert_eq!(
        Lifecycle::open(journal, fake.clone(), fake.clone())
            .unwrap()
            .records()["child"]
            .metadata["parentSessionId"],
        "parent"
    );
    launch.abort();
    fake.pause.notify_one();
    for _ in 0..1000 {
        if service.records()["child"].phase == Phase::Running {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        service.records()["child"].receipt.as_ref().unwrap()["messageQueued"],
        true
    );
    let row =
        service.enrich(json!({"sessionId":"child","mode":"input","settings":{"observed":true}}));
    assert_eq!(row["label"], "worker");
    assert_eq!(row["settings"]["observed"], true);
    assert_eq!(
        service.enrich(json!({"sessionId":"child","hub":"remote"}))["label"],
        Value::Null
    );
}
#[tokio::test]
async fn failed_launch_retains_retryable_revocation_and_fences_old_teardown() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    fake.fail_spawn.store(true, Ordering::SeqCst);
    fake.fail_revoke.store(true, Ordering::SeqCst);
    let service =
        Lifecycle::open(dir.path().join("launch.json"), fake.clone(), fake.clone()).unwrap();
    assert!(
        service
            .launch(plan(dir.path()))
            .await
            .unwrap_err()
            .to_string()
            .contains("cleanup")
    );
    let first = service.records()["child"].clone();
    assert!(first.revocation_pending);
    assert!(service.launch(plan(dir.path())).await.is_err());
    fake.fail_revoke.store(false, Ordering::SeqCst);
    assert!(service.stopped("child", &first.generation).await.unwrap());
    fake.fail_spawn.store(false, Ordering::SeqCst);
    service.launch(plan(dir.path())).await.unwrap();
    assert!(!service.stopped("child", &first.generation).await.unwrap());
    let second = service.records()["child"].clone();
    assert_eq!(second.phase, Phase::Running);
    assert!(service.stopped("child", &second.generation).await.unwrap());
    let count = fake.events.lock().unwrap().len();
    assert!(service.stopped("child", &second.generation).await.unwrap());
    assert_eq!(fake.events.lock().unwrap().len(), count);
}
#[tokio::test]
async fn invalid_cwd_has_no_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let service =
        Lifecycle::open(dir.path().join("launch.json"), fake.clone(), fake.clone()).unwrap();
    let mut p = plan(dir.path());
    p.request["cwd"] = json!(dir.path().join("absent"));
    assert!(service.launch(p).await.is_err());
    assert!(fake.events.lock().unwrap().is_empty());
    assert!(service.records().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn embedded_engine_launch_and_authoritative_stop_reconciliation() {
    use claudemon::daemon::{
        ServeConfig,
        embedded::{EmbeddedDaemon, Options as EngineOptions},
    };
    let dir = tempfile::tempdir().unwrap();
    let mut engine = EmbeddedDaemon::start_with_options(
        ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: dir.path().join("daemon.db"),
        },
        EngineOptions {
            usage_poll_on_boot: Some(false),
        },
    )
    .unwrap();
    engine.ready().await.unwrap();
    let engine_client = Arc::new(engine.client());
    let preparation = Arc::new(Fake::default());
    let service = Lifecycle::open(
        dir.path().join("journal.json"),
        engine_client.clone(),
        preparation.clone(),
    )
    .unwrap();
    let mut p = plan(dir.path());
    // A local shell fixture exercises the real PTY admission/registration/reaper;
    // no provider CLI, credentials, network model, or billable work is involved.
    p.request["argv"] = json!(["/bin/sh", "-c", "exec sleep 120"]);
    p.request.as_object_mut().unwrap().remove("first_message");
    let receipt = service.launch(p).await.unwrap();
    assert_eq!(receipt["sessionId"], "child");
    service.reconcile().await.unwrap();
    assert_eq!(service.records()["child"].phase, Phase::Running);
    engine_client.stop("child").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            service.reconcile().await.unwrap();
            if service.records()["child"].phase == Phase::Stopped {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(!service.records()["child"].revocation_pending);
    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn existing_engine_identity_is_rejected_before_rotating_its_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    fake.live_rows
        .lock()
        .unwrap()
        .push(json!({"session_id":"child","mode":"input"}));
    let service =
        Lifecycle::open(dir.path().join("journal.json"), fake.clone(), fake.clone()).unwrap();
    assert!(
        service
            .launch(plan(dir.path()))
            .await
            .unwrap_err()
            .to_string()
            .contains("already live")
    );
    assert!(service.records().is_empty());
    assert!(fake.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn close_interrupts_hung_preparation_and_revokes_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    fake.hang_prepare.store(true, Ordering::SeqCst);
    let service =
        Lifecycle::open(dir.path().join("launch.json"), fake.clone(), fake.clone()).unwrap();
    let launch = {
        let service = service.clone();
        let plan = plan(dir.path());
        tokio::spawn(async move { service.launch(plan).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if fake.events.lock().unwrap().contains(&"prepare".to_owned()) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), service.close())
        .await
        .unwrap();
    assert!(
        launch
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("closing")
    );
    assert_eq!(*fake.events.lock().unwrap(), vec!["prepare", "revoke"]);
    assert!(!service.records()["child"].engine_attempted);
}

#[tokio::test]
async fn manual_parent_transfer_updates_live_and_pending_but_preserves_ended_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("launch.json");
    let mut records = serde_json::Map::new();
    for (id, phase) in [
        ("live", "running"),
        ("pending", "preparing"),
        ("ended", "stopped"),
        ("successor", "running"),
    ] {
        records.insert(id.into(),json!({"generation":id,"provider":"claude","cwd":dir.path(),"metadata":{"parentSessionId":"old"},"phase":phase,"revocationPending":false,"receipt":null}));
    }
    std::fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();
    let fake = Arc::new(Fake::default());
    let service = Lifecycle::open(path.clone(), fake.clone(), fake.clone()).unwrap();
    assert_eq!(
        service
            .reparent_current_children(
                "old",
                "successor",
                &["live".into(), "successor".into()].into()
            )
            .await
            .unwrap(),
        ["live", "pending"]
    );
    let reopened = Lifecycle::open(path, fake.clone(), fake).unwrap();
    for id in ["live", "pending"] {
        assert_eq!(
            reopened.records()[id].metadata["parentSessionId"],
            "successor"
        );
    }
    for id in ["ended", "successor"] {
        assert_eq!(reopened.records()[id].metadata["parentSessionId"], "old");
    }
}

#[tokio::test]
async fn confirmed_live_controls_are_generation_fenced_and_cannot_change_authority() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let path = dir.path().join("launch.json");
    let service = Lifecycle::open(path.clone(), fake.clone(), fake.clone()).unwrap();
    let mut requested = plan(dir.path());
    requested.metadata["settings"]["effort"] = json!("low");
    service.launch(requested).await.unwrap();
    let generation = service.records()["child"].generation.clone();
    assert!(
        service
            .enrich(json!({"sessionId":"child"}))
            .get("liveEffort")
            .is_none()
    );
    let patch = json!({"settings":{"model":"confirmed-model","effort":"high","permissionMode":"plan","launchIntegrationId":"forged"},"requestedSelection":{"model":"confirmed-model"},"livePermissionMode":"plan","isWakeTarget":true,"parentSessionId":"forged"});
    assert!(
        !service
            .note_live_control("child", "stale", &patch)
            .await
            .unwrap()
    );
    assert!(
        service
            .enrich(json!({"sessionId":"child"}))
            .get("liveEffort")
            .is_none(),
        "a stale acknowledgement must not become a live observation"
    );
    assert!(
        service
            .note_live_control("child", &generation, &patch)
            .await
            .unwrap()
    );
    let row = service.enrich(json!({"sessionId":"child"}));
    assert_eq!(row["settings"]["model"], "confirmed-model");
    assert_eq!(row["liveEffort"], "high");
    // Fresh daemon snapshots do not carry the host's control acknowledgement.
    let next = service.enrich(json!({"sessionId":"child","mode":"input"}));
    assert_eq!(next["liveEffort"], "high");
    assert_eq!(row["livePermissionMode"], "plan");
    assert_eq!(row["parentSessionId"], "parent");
    assert!(row["settings"]["launchIntegrationId"].is_null());
    assert_ne!(row["isWakeTarget"], true);
    assert_eq!(
        Lifecycle::open(path.clone(), fake.clone(), fake.clone())
            .unwrap()
            .records()["child"]
            .metadata["requestedSelection"]["model"],
        "confirmed-model"
    );
    let restored = Lifecycle::open(path, fake.clone(), fake).unwrap();
    assert_eq!(
        restored.enrich(json!({"sessionId":"child"}))["liveEffort"],
        "high"
    );
    service.stopped("child", &generation).await.unwrap();
    assert!(
        !service
            .note_live_control("child", &generation, &patch)
            .await
            .unwrap()
    );
    service.close().await;
    assert!(
        !service
            .note_live_control("child", &generation, &patch)
            .await
            .unwrap()
    );
}

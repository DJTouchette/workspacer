use serde_json::{Value, json};
use workspacer_hub::services::manager_replacements::ReplacementState;
fn operation() -> Value {
    json!({"operationId":uuid::Uuid::new_v4().to_string(),"sourceSessionId":"old","successorSessionId":"new","paneId":"pane","workspaceId":"workspace","phase":"preparing","createdAt":1,"updatedAt":1,"committed":false,"bound":false,"artifactPath":"/project/.workspacer/handoff.json","workerIds":["worker"],"taskIds":["task"],"deliveries":[],"launch":{"options":{"manager":true,"toolScope":"operator","cwd":"/project","provider":"codex","transport":"stream"},"grants":"identity"},"metadata":[{"sessionId":"old","cwd":"/project","isWakeTarget":true},{"sessionId":"worker","cwd":"/project","parentSessionId":"old"}],"signatures":{},"finishes":{}})
}
#[test]
fn transfer_intent_routes_worker_wakes_to_held_successor_without_inventing_liveness() {
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("manager-replacements.json")).unwrap();
    let mut op = operation();
    op["transferIntent"] = true.into();
    state
        .edit(|d| {
            d.operations.push(op.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(state.worker_wake_target("old", "worker").unwrap(), "new");
    assert_eq!(state.automatic_wake_target("old").unwrap(), "old");
    assert!(state.parked_successor("new"));
    assert!(
        state
            .hold_message("new", "finished", &[("worker".into(), "sig".into())], None)
            .unwrap()
    );
    assert_eq!(state.signature("worker").as_deref(), Some("sig"));
    let actual=state.enrich(json!({"sessionId":"worker","status":"ended","mode":"stopped","usage":{"tokens":12},"parentSessionId":"old"}));
    assert_eq!(actual["parentSessionId"], "new");
    assert_eq!(actual["mode"], "stopped");
    assert_eq!(actual["status"], "ended");
    let remote = json!({"sessionId":"worker","hub":"peer","parentSessionId":"remote-owner"});
    assert_eq!(state.enrich(remote.clone()), remote);
}
#[test]
fn restart_marks_unacknowledged_delivery_uncertain_and_never_completes_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.json");
    let state = ReplacementState::open(path.clone()).unwrap();
    let mut op = operation();
    op["phase"] = "activating".into();
    op["committed"] = true.into();
    op["bound"] = true.into();
    op["deliveries"] = json!([{"id":"delivery","kind":"kickoff","text":"go","status":"sending"}]);
    let id = op["operationId"].as_str().unwrap().to_string();
    state
        .edit(|d| {
            d.operations.push(op);
            Ok(())
        })
        .unwrap();
    drop(state);
    let recovered = ReplacementState::open(path).unwrap();
    recovered.recover_status().unwrap();
    let op = recovered.get(&id).unwrap();
    assert_eq!(op["phase"], "recovery-required");
    assert_eq!(op["bound"], false);
    assert_eq!(op["deliveries"][0]["status"], "uncertain");
    assert!(!recovered.acknowledged("delivery"));
    assert!(recovered.assert_resume("old").is_err());
    assert!(recovered.assert_resume("new").is_err());
    assert_eq!(recovered.wake_target("old").unwrap(), "new");
}
#[test]
fn manual_owner_redirect_and_worker_metadata_beat_historical_lineage() {
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    let mut op = operation();
    op["committed"] = true.into();
    op["phase"] = "complete".into();
    state
        .edit(|d| {
            d.operations.push(op);
            Ok(())
        })
        .unwrap();
    state
        .note_manual_reparent(
            "new",
            "other",
            &["worker".into()],
            json!({"sessionId":"other","cwd":"/project","isWakeTarget":true}),
        )
        .unwrap();
    assert_eq!(state.wake_target("old").unwrap(), "other");
    assert_eq!(state.worker_wake_target("old", "worker").unwrap(), "other");
    assert_eq!(state.automatic_wake_target("old").unwrap(), "new");
}
#[test]
fn admission_guard_survives_fence_and_drops_exact_count() {
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    let guard = state.admit(&["old", "old"]).unwrap();
    assert_eq!(state.active_count("old"), 1);
    state
        .edit(|d| {
            d.operations.push(operation());
            Ok(())
        })
        .unwrap();
    assert!(state.admit(&["old"]).is_err());
    assert_eq!(state.active_count("old"), 1);
    drop(guard);
    assert_eq!(state.active_count("old"), 0);
}
#[test]
fn failed_journal_mutation_rolls_back_and_views_hide_authority_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.json");
    let state = ReplacementState::open(path.clone()).unwrap();
    state
        .edit(|d| {
            d.operations.push(operation());
            Ok(())
        })
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(
        state
            .edit(|d| {
                d.operations[0]["committed"] = "forged".into();
                Ok(())
            })
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
    let view = &state.views()[0];
    assert!(view.get("launch").is_none());
    assert!(view.get("metadata").is_none());
    assert!(view.get("signatures").is_none());
}

use anyhow::Result;
use futures_util::future::BoxFuture;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use workspacer_hub::services::{
    manager_replacements::{ReplacementHost, ReplacementService, SendOutcome, Timing, artifact},
    task_store::TaskStore,
};
struct Host {
    gate: Arc<Mutex<()>>,
    state: Arc<ReplacementState>,
    launch: Value,
    bound: AtomicBool,
    spawned: AtomicUsize,
    fail_transfer: AtomicBool,
    hold_spawn: AtomicBool,
    spawn_entered: tokio::sync::Notify,
    spawn_release: tokio::sync::Semaphore,
    transfer_attempts: AtomicUsize,
    close_changed: tokio::sync::Notify,
    sent: Mutex<Vec<(String, String)>>,
    closed: Mutex<Vec<String>>,
    outcomes: Mutex<Vec<SendOutcome>>,
    inventory: Mutex<Vec<Value>>,
}
impl ReplacementHost for Host {
    fn capture_gate(&self) -> Arc<Mutex<()>> {
        self.gate.clone()
    }
    fn source(&self, _: &str, _: Option<&str>) -> Result<Value> {
        Ok(self.launch.clone())
    }
    fn inventory(&self, _: &str) -> Result<Vec<Value>> {
        Ok(self.inventory.lock().unwrap().clone())
    }
    fn evidence(&self, _: &str, _: &[String]) -> Result<Value> {
        Ok(json!({"inFlightMessages":[],"signatures":{},"finishes":{}}))
    }
    fn settled(&self, _: &str) -> bool {
        true
    }
    fn bound(&self, _: &str, _: &str) -> bool {
        self.bound.load(Ordering::SeqCst)
    }
    fn receipt(&self, id: String) -> BoxFuture<'_, Result<String>> {
        Box::pin(async move {
            let op = self
                .state
                .records()
                .into_iter()
                .find(|o| o["sourceSessionId"] == id)
                .unwrap();
            let cwd = std::path::Path::new(op["launch"]["options"]["cwd"].as_str().unwrap());
            let brief = cwd.join(".workspacer/brief.md");
            std::fs::write(&brief, b"# Fleet checkpoint\n")?;
            let a = json!({"version":1,"operationId":op["operationId"],"sourceSessionId":op["sourceSessionId"],"cwd":op["launch"]["options"]["cwd"],"checkpoint":{"completed":true,"files":[{"path":brief,"sha256":artifact::hash(b"# Fleet checkpoint\n")}]},"workers":op["workerIds"].as_array().unwrap().iter().map(|id|json!({"sessionId":id,"instructions":"Continue original task"})).collect::<Vec<_>>(),"tasks":op["taskIds"].as_array().unwrap().iter().map(|id|json!({"taskId":id,"nextAction":"Inspect work"})).collect::<Vec<_>>(),"pendingDecisions":[],"facts":[],"nextAction":"Inspect task"});
            let bytes = serde_json::to_vec(&a)?;
            std::fs::write(op["artifactPath"].as_str().unwrap(), &bytes)?;
            Ok(format!(
                "```wks-manager-handoff\n{}\n```",
                json!({"operationId":op["operationId"],"sourceSessionId":op["sourceSessionId"],"sha256":artifact::hash(&bytes)})
            ))
        })
    }
    fn spawn(&self, _: String, _: Value) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.spawned.fetch_add(1, Ordering::SeqCst);
            self.spawn_entered.notify_one();
            if self.hold_spawn.load(Ordering::SeqCst) {
                self.spawn_release.acquire().await.unwrap().forget();
            }
            Ok(())
        })
    }
    fn validate_successor(&self, _: String, _: Value) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn reparent(&self, source: String, successor: String) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move {
            self.transfer_attempts.fetch_add(1, Ordering::SeqCst);
            if self.fail_transfer.swap(false, Ordering::SeqCst) {
                anyhow::bail!("worker transfer acknowledgement lost");
            }
            let mut ids = vec![];
            for m in self.inventory.lock().unwrap().iter_mut() {
                if m["parentSessionId"] == source {
                    m["parentSessionId"] = successor.clone().into();
                    ids.push(m["sessionId"].as_str().unwrap().to_string());
                }
            }
            Ok(ids)
        })
    }
    fn restore(&self, _: Vec<Value>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn send(
        &self,
        id: String,
        content: String,
        _: Option<Value>,
    ) -> BoxFuture<'_, Result<SendOutcome>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push((id, content));
            let mut outcomes = self.outcomes.lock().unwrap();
            Ok(if outcomes.is_empty() {
                SendOutcome::Accepted
            } else {
                outcomes.remove(0)
            })
        })
    }
    fn pause(&self, _: String) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn close(&self, id: String) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.closed.lock().unwrap().push(id);
            self.close_changed.notify_one();
            Ok(())
        })
    }
    fn kickoff(&self, op: &Value) -> Result<String> {
        Ok(workspacer_hub::services::manager_replacements::native::kickoff_message(op))
    }
    fn recover_finishes(&self, _: &Value) -> Result<()> {
        Ok(())
    }
    fn flush_finishes(&self, _: Vec<String>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn retry_request(&self, _: &str, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
}
fn service_fixture() -> (
    tempfile::TempDir,
    Arc<TaskStore>,
    Arc<Host>,
    Arc<ReplacementService>,
) {
    service_fixture_with_timing(Timing {
        preparation: Duration::from_secs(1),
        poll: Duration::from_millis(1),
        delivery: Duration::from_millis(200),
    })
}
fn service_fixture_with_timing(
    timing: Timing,
) -> (
    tempfile::TempDir,
    Arc<TaskStore>,
    Arc<Host>,
    Arc<ReplacementService>,
) {
    let dir = tempfile::tempdir().unwrap();
    // Real coordinator launch records carry the canonical host cwd. On macOS
    // tempdir's /var spelling is a symlink; on Windows avoid verbatim aliases.
    // The artifact verifier intentionally refuses linked candidate parents.
    let cwd = workspacer_hub::services::paths::canonicalize(dir.path()).unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    let tasks = Arc::new(TaskStore::open(dir.path().join("history.json")).unwrap());
    tasks.transaction(|h|{h.tasks.push(json!({"taskId":"task","ownerSessionId":"old","ownerLabel":"Old","projectCwd":cwd,"title":"Work","createdAt":"2026-09-28T00:00:00Z","attempts":[]}));Ok(())}).unwrap();
    let host = Arc::new(Host {
        gate: Arc::new(Mutex::new(())),
        state: state.clone(),
        launch: json!({"options":{"manager":true,"toolScope":"operator","cwd":cwd,"transport":"stream","provider":"codex","model":"gpt-5.6-sol"},"grants":"real-host-fingerprint"}),
        bound: AtomicBool::new(false),
        spawned: AtomicUsize::new(0),
        fail_transfer: AtomicBool::new(false),
        hold_spawn: AtomicBool::new(false),
        spawn_entered: tokio::sync::Notify::new(),
        spawn_release: tokio::sync::Semaphore::new(0),
        transfer_attempts: AtomicUsize::new(0),
        close_changed: tokio::sync::Notify::new(),
        sent: Mutex::new(vec![]),
        closed: Mutex::new(vec![]),
        outcomes: Mutex::new(vec![]),
        inventory: Mutex::new(vec![
            json!({"sessionId":"old","cwd":cwd,"isWakeTarget":true}),
            json!({"sessionId":"worker","cwd":cwd,"parentSessionId":"old"}),
        ]),
    });
    let service = ReplacementService::new(state, tasks.clone(), host.clone(), timing);
    (dir, tasks, host, service)
}
#[tokio::test]
async fn complete_handoff_commits_tasks_before_binding_and_requires_actual_view_ack() {
    let (_dir, tasks, host, service) = service_fixture();
    let manual = service.state.manual_admission(&["old"]).unwrap();
    let refused = service.request(json!({"action":"start","sourceSessionId":"old","paneId":"pane","workspaceId":"workspace"})).await;
    assert!(refused["error"].is_string());
    assert!(service.state.records().is_empty());
    drop(manual);
    let response=service.request(json!({"action":"start","sourceSessionId":"old","paneId":"pane","workspaceId":"workspace"})).await;
    assert!(response["error"].is_null(), "{response}");
    let id = response["operations"][0]["operationId"].as_str().unwrap();
    service.idle(id).await;
    let op = service.state.get(id).unwrap();
    assert_eq!(op["phase"], "binding", "{op}");
    assert_eq!(op["committed"], true);
    assert_eq!(
        tasks.task("task").unwrap().unwrap()["ownerSessionId"],
        op["successorSessionId"]
    );
    assert_eq!(host.spawned.load(Ordering::SeqCst), 1);
    assert!(host.closed.lock().unwrap().is_empty());
    let refused = service
        .request(json!({"action":"bind","operationId":id}))
        .await;
    assert!(refused["error"].is_string());
    assert_eq!(host.sent.lock().unwrap().len(), 1);
    host.bound.store(true, Ordering::SeqCst);
    service
        .request(json!({"action":"bind","operationId":id}))
        .await;
    service.idle(id).await;
    assert_eq!(service.state.get(id).unwrap()["phase"], "complete");
    assert_eq!(host.sent.lock().unwrap().len(), 2);
    assert_eq!(*host.closed.lock().unwrap(), vec!["old".to_string()]);
    service.close().await.unwrap();
}
#[tokio::test]
async fn uncertain_preparation_is_retained_and_never_automatically_replayed() {
    let (_dir, tasks, host, service) = service_fixture();
    host.outcomes
        .lock()
        .unwrap()
        .push(SendOutcome::Uncertain("transport interrupted".into()));
    let response=service.request(json!({"action":"start","sourceSessionId":"old","paneId":"pane","workspaceId":"workspace"})).await;
    let id = response["operations"][0]["operationId"].as_str().unwrap();
    service.idle(id).await;
    let op = service.state.get(id).unwrap();
    assert_eq!(op["phase"], "recovery-required");
    assert_eq!(op["deliveries"][0]["status"], "uncertain");
    service
        .request(json!({"action":"reconcile","operationId":id}))
        .await;
    assert_eq!(host.sent.lock().unwrap().len(), 1);
    assert_eq!(host.spawned.load(Ordering::SeqCst), 0);
    assert_eq!(
        tasks.task("task").unwrap().unwrap()["ownerSessionId"],
        "old"
    );
    service.close().await.unwrap();
}

#[test]
fn acknowledgement_and_hold_cannot_clear_a_newer_finish() {
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    let op = operation();
    let id = op["operationId"].as_str().unwrap().to_string();
    state
        .edit(|d| {
            d.operations.push(op);
            Ok(())
        })
        .unwrap();
    state
        .record_finish("old", "worker", "first reply", false)
        .unwrap();
    state
        .record_finish("old", "worker", "newer reply", false)
        .unwrap();
    assert!(
        !state
            .record_signature_for("worker", "first-sig", Some("first reply"))
            .unwrap()
    );
    assert_eq!(
        state.get(&id).unwrap()["finishes"]["worker"]["reply"],
        "newer reply"
    );
    state
        .hold_message(
            "old",
            "first message",
            &[("worker".into(), "first-sig".into())],
            None,
        )
        .unwrap();
    assert_eq!(
        state.get(&id).unwrap()["finishes"]["worker"]["reply"],
        "newer reply"
    );
    assert!(
        state
            .record_signature_for("worker", "newer-sig", Some("newer reply"))
            .unwrap()
    );
    assert!(
        state.get(&id).unwrap()["finishes"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn capture_gate_serializes_inflight_snapshot_with_acknowledgement() {
    use workspacer_hub::services::manager_replacements::{BeginDelivery, MessageTracker};
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    let tracker = MessageTracker::new(state.clone());
    let BeginDelivery::Send(delivery) = tracker
        .begin("old", "earlier message", &[], None, false)
        .unwrap()
    else {
        panic!("unexpected hold")
    };
    let gate = tracker.capture_gate.lock().unwrap();
    let evidence = tracker.evidence("old");
    let mut op = operation();
    op["deliveries"] = json!(
        evidence["inFlightMessages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| json!({"id":f["id"],"kind":"message","text":f["text"],"status":"sending"}))
            .collect::<Vec<_>>()
    );
    let id = op["operationId"].as_str().unwrap().to_string();
    let thread = std::thread::spawn(move || delivery.finish(&SendOutcome::Accepted).unwrap());
    state
        .edit(|d| {
            d.operations.push(op);
            Ok(())
        })
        .unwrap();
    drop(gate);
    thread.join().unwrap();
    assert_eq!(
        state.get(&id).unwrap()["deliveries"][0]["status"],
        "accepted"
    );
}
#[test]
fn handoff_artifact_rejects_changed_briefs_and_links() {
    let (dir, _, host, service) = service_fixture();
    let mut op = operation();
    op["launch"] = host.launch.clone();
    op["metadata"] = json!([]);
    op["workerIds"] = json!([]);
    op["taskIds"] = json!([]);
    let path = artifact::create_path(
        dir.path().to_str().unwrap(),
        op["operationId"].as_str().unwrap(),
    )
    .unwrap();
    op["artifactPath"] = json!(path);
    service
        .state
        .edit(|d| {
            d.operations.push(op.clone());
            Ok(())
        })
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let receipt = runtime.block_on(host.receipt("old".into())).unwrap();
    assert!(artifact::validate(&op, &receipt).is_ok());
    std::fs::write(dir.path().join(".workspacer/brief.md"), b"changed").unwrap();
    assert!(artifact::validate(&op, &receipt).is_err());
    #[cfg(unix)]
    {
        let brief = dir.path().join(".workspacer/brief.md");
        std::fs::remove_file(&brief).unwrap();
        let elsewhere = dir.path().join("outside.md");
        std::fs::write(&elsewhere, b"# Fleet checkpoint\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &brief).unwrap();
        assert!(artifact::validate(&op, &receipt).is_err());
    }
}

#[test]
fn finish_acknowledgement_is_fenced_by_stop_state_as_well_as_reply() {
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    let op = operation();
    let id = op["operationId"].as_str().unwrap().to_string();
    state
        .edit(|d| {
            d.operations.push(op);
            Ok(())
        })
        .unwrap();
    state
        .record_finish("old", "worker", "same reply", false)
        .unwrap();
    state
        .record_finish("old", "worker", "same reply", true)
        .unwrap();
    assert!(
        !state
            .record_signature_for_finish("worker", "earlier", "same reply", false)
            .unwrap()
    );
    assert_eq!(
        state.get(&id).unwrap()["finishes"]["worker"]["stopped"],
        true
    );
    assert!(
        state
            .record_signature_for_finish("worker", "latest", "same reply", true)
            .unwrap()
    );
}

#[tokio::test]
async fn interrupted_worker_transfer_rolls_task_ownership_forward_never_back() {
    // This tests a lost transfer acknowledgement, not setup I/O completing
    // within a one-second synthetic preparation deadline under Windows CI.
    let (_dir, tasks, host, service) = service_fixture_with_timing(Timing::default());
    host.fail_transfer.store(true, Ordering::SeqCst);
    let response=service.request(json!({"action":"start","sourceSessionId":"old","paneId":"pane","workspaceId":"workspace"})).await;
    let id = response["operations"][0]["operationId"].as_str().unwrap();
    service.idle(id).await;
    let interrupted = service.state.get(id).unwrap();
    assert_eq!(
        host.transfer_attempts.load(Ordering::SeqCst),
        1,
        "{interrupted}"
    );
    assert_eq!(interrupted["phase"], "recovery-required", "{interrupted}");
    assert_eq!(interrupted["transferIntent"], true);
    assert_eq!(interrupted["taskTransferCommitted"], true);
    assert_eq!(
        tasks.task("task").unwrap().unwrap()["ownerSessionId"],
        interrupted["successorSessionId"]
    );
    assert!(
        service
            .request(json!({"action":"cancel","operationId":id}))
            .await["error"]
            .is_string()
    );
    let reconciled = service
        .request(json!({"action":"reconcile","operationId":id}))
        .await;
    assert!(reconciled["error"].is_null(), "{reconciled}");
    assert_eq!(service.state.get(id).unwrap()["phase"], "binding");
    assert_eq!(host.spawned.load(Ordering::SeqCst), 1);
    service.close().await.unwrap();
}
#[tokio::test]
async fn late_successor_after_timeout_is_closed_without_kickoff() {
    let (_dir, tasks, host, service) = service_fixture_with_timing(Timing {
        delivery: Duration::from_millis(200),
        ..Timing::default()
    });
    host.hold_spawn.store(true, Ordering::SeqCst);
    let response=service.request(json!({"action":"start","sourceSessionId":"old","paneId":"pane","workspaceId":"workspace"})).await;
    let id = response["operations"][0]["operationId"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(15), host.spawn_entered.notified())
        .await
        .unwrap();
    service.idle(id).await;
    let failed = service.state.get(id).unwrap();
    assert_eq!(failed["phase"], "failed", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("Successor spawn timed out"),
        "{failed}"
    );
    assert_eq!(
        tasks.task("task").unwrap().unwrap()["ownerSessionId"],
        "old"
    );
    assert_eq!(
        host.closed.lock().unwrap().len(),
        1,
        "initial timeout cleanup"
    );
    // The candidate cannot complete before we have observed the timed-out
    // operation. Its completion receipt, not another wall-clock sleep, allows
    // the retained cleanup task to issue the second close.
    host.spawn_release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let notified = host.close_changed.notified();
            if host.closed.lock().unwrap().len() >= 2 {
                break;
            }
            notified.await;
        }
    })
    .await
    .unwrap();
    let closed = host.closed.lock().unwrap().clone();
    assert_eq!(closed.len(), 2);
    assert!(closed.iter().all(|id| failed["successorSessionId"] == *id));
    assert_eq!(host.sent.lock().unwrap().len(), 1);
    service.close().await.unwrap();
}

#[test]
fn fresh_host_metadata_can_remove_a_previous_manager_role() {
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    state.remember_child(json!({"sessionId":"manager","cwd":"/project","isWakeTarget":true,"dispatchAdmission":{"prompt":"must not be copied"}})).unwrap();
    state.remember_child(json!({"sessionId":"manager","cwd":"/project","isWakeTarget":false,"parentSessionId":"ordinary"})).unwrap();
    let metadata = state.metadata("manager").unwrap();
    assert_eq!(metadata["isWakeTarget"], false);
    assert_eq!(metadata["parentSessionId"], "ordinary");
    assert!(metadata.get("dispatchAdmission").is_none());
}

#[test]
fn manual_transfer_exclusively_fences_both_owners_and_releases_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let state = ReplacementState::open(dir.path().join("journal.json")).unwrap();
    let launch = state.admit(&["old"]).unwrap();
    assert!(state.manual_admission(&["old", "new"]).is_err());
    drop(launch);
    let transfer = state.manual_admission(&["old", "new"]).unwrap();
    assert!(state.admit(&["old"]).is_err());
    assert!(state.admit(&["new"]).is_err());
    assert!(state.manual_admission(&["new"]).is_err());
    assert!(state.admit(&["unrelated"]).is_ok());
    drop(transfer);
    assert!(state.admit(&["old", "new"]).is_ok());
}

#[tokio::test]
async fn binding_delivers_real_kickoff_then_exact_held_message_once_and_retires_predecessor() {
    use workspacer_hub::services::manager_replacements::{BeginDelivery, MessageTracker};
    let (_dir, tasks, host, service) = service_fixture();
    let response = service.request(json!({"action":"start","sourceSessionId":"old","paneId":"pane","workspaceId":"workspace"})).await;
    assert!(response["error"].is_null(), "{response}");
    let id = response["operations"][0]["operationId"].as_str().unwrap();
    service.idle(id).await;
    let op = service.state.get(id).unwrap();
    assert_eq!(op["phase"], "binding");
    let successor = op["successorSessionId"].as_str().unwrap();
    let tracker = MessageTracker::new(service.state.clone());
    let message = "Retain this exact queued message 🦀\nincluding its second line";
    assert!(matches!(
        tracker.begin("old", message, &[], None, false).unwrap(),
        BeginDelivery::Held
    ));
    assert_eq!(
        host.sent.lock().unwrap().len(),
        1,
        "binding must not deliver a held prompt early"
    );
    assert!(host.closed.lock().unwrap().is_empty());
    assert_eq!(
        tasks.task("task").unwrap().unwrap()["ownerSessionId"],
        successor
    );
    assert_eq!(
        host.inventory
            .lock()
            .unwrap()
            .iter()
            .find(|row| row["sessionId"] == "worker")
            .unwrap()["parentSessionId"],
        successor
    );
    host.bound.store(true, Ordering::SeqCst);
    service
        .request(json!({"action":"bind","operationId":id}))
        .await;
    service.idle(id).await;
    let completed = service.state.get(id).unwrap();
    assert_eq!(completed["phase"], "complete");
    let expected =
        workspacer_hub::services::manager_replacements::native::kickoff_message(&completed);
    assert!(expected.contains(&format!("HOST-OWNED MANAGER HANDOFF {id}")));
    assert!(expected.contains("Host committed worker AND task ownership"));
    assert!(expected.contains("predecessor old is audit history only"));
    // Source-compatible transfer annotation follows, without changing any
    // byte of the original message. It corrects stale parent IDs in quoted work.
    let expected_held = format!(
        "{message}\n\n[Host manager handoff {id}] Current manager/parentSessionId is {successor}. Earlier owner IDs in quoted instructions refer to predecessor. Preserve task IDs and pinned policy; inspect next_workflow_step before continuation. Do not adopt again or replay worker tasks."
    );

    {
        let sent = host.sent.lock().unwrap();
        let successor_messages: Vec<_> = sent
            .iter()
            .filter(|(target, _)| target == successor)
            .map(|(_, text)| text.as_str())
            .collect();
        assert_eq!(
            successor_messages,
            vec![expected.as_str(), expected_held.as_str()]
        );
        assert_eq!(
            sent.iter()
                .map(|(_, text)| text.matches(message).count())
                .sum::<usize>(),
            1
        );
    }
    assert_eq!(*host.closed.lock().unwrap(), vec!["old".to_owned()]);
    service
        .request(json!({"action":"bind","operationId":id}))
        .await;
    service.idle(id).await;
    assert_eq!(
        host.sent.lock().unwrap().len(),
        3,
        "repeat viewer acknowledgement must not replay kickoff or held text"
    );
    assert_eq!(
        service.state.get(id).unwrap()["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|delivery| delivery["text"] == expected_held && delivery["status"] == "accepted")
            .count(),
        1
    );
    service.close().await.unwrap();
}

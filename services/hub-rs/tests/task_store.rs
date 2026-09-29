use anyhow::bail;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use workspacer_hub::services::{
    manager_requests::{ManagerRequests, WorkflowPin},
    task_store::{OwnerLookup, TaskStore, dependency_state, revision},
};
fn task(id: &str) -> Value {
    json!({"taskId":id,"ownerSessionId":"manager","ownerLabel":"Manager","projectCwd":"/project","title":"Task","createdAt":"2026-09-28T00:00:00Z","attempts":[]})
}
fn pin() -> Value {
    json!({"hash":"pinned","templates":{},"definition":{"id":"selected","steps":[{"id":"implement","when":"always","template":"ship"}]},"steps":[{"id":"implement","state":"planned"}]})
}
fn owner() -> Value {
    json!({"sessionId":"manager","cwd":"/project","isWakeTarget":true,"status":"active"})
}
#[test]
fn atomic_multi_task_rollback_and_cross_instance_revision_cas() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.json");
    let a = Arc::new(TaskStore::open(path.clone()).unwrap());
    a.transaction(|h| {
        h.tasks.push(task("task"));
        Ok(())
    })
    .unwrap();
    assert_eq!(revision(&a.task("task").unwrap().unwrap()), 1);
    let old = std::fs::read(&path).unwrap();
    assert!(
        a.transaction::<()>(|h| {
            h.tasks[0]["title"] = "partial".into();
            h.tasks.push(task("new"));
            bail!("second mutation refused")
        })
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), old);
    let b = Arc::new(TaskStore::open(path.clone()).unwrap());
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [a.clone(), b.clone()]
        .into_iter()
        .enumerate()
        .map(|(n, s)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                s.mutate_owned(
                    "task",
                    "manager",
                    "/project",
                    1,
                    || Ok(()),
                    |t| {
                        t["title"] = format!("edit {n}").into();
                        Ok(())
                    },
                )
                .is_ok()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .map(|h| h.join().unwrap() as usize)
            .sum::<usize>(),
        1
    );
    assert_eq!(revision(&a.task("task").unwrap().unwrap()), 2);
    let before = std::fs::read(&path).unwrap();
    a.transaction(|_| Ok(())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
#[test]
fn durable_dispatch_reservation_fences_adoption_and_crash_views_are_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.json");
    let store = TaskStore::open(path.clone()).unwrap();
    let task = store
        .start_workflow(&owner(), "/project", "Work", pin())
        .unwrap();
    let id = task["taskId"].as_str().unwrap();
    let token = store
        .reserve_workflow_dispatch(id, revision(&task), "implement")
        .unwrap();
    assert!(store.adopt("manager", "successor").is_err());
    store.release_workflow_dispatch(id, "wrong").unwrap();
    assert!(store.adopt("manager", "successor").is_err());
    assert!(store.accept(json!({"owner":owner(),"taskId":id,"workflowStepId":"implement","sessionId":"worker","projectCwd":"/project","executionCwd":"/project"}),||Ok(())).is_err());
    let input = json!({"owner":owner(),"taskId":id,"workflowStepId":"implement","workflowReservationToken":token,"sessionId":"worker","projectCwd":"/project","executionCwd":"/project"});
    let receipt = store.accept(input.clone(), || Ok(())).unwrap().unwrap();
    assert!(store.adopt("manager", "successor").is_err());
    store.release_workflow_dispatch(id, &token).unwrap();
    assert_eq!(store.accept(input, || Ok(())).unwrap().unwrap(), receipt);
    assert!(receipt["dispatchId"].is_string());
    assert_eq!(
        store.task(id).unwrap().unwrap()["attempts"][0]["stale"],
        false
    );
    let reopened = TaskStore::open(path).unwrap();
    assert_eq!(
        reopened.task(id).unwrap().unwrap()["attempts"][0]["stale"],
        true
    );
    assert_eq!(
        reopened.task(id).unwrap().unwrap()["attempts"][0]["live"],
        false
    );
}
fn manager_fixture() -> (
    tempfile::TempDir,
    Arc<TaskStore>,
    ManagerRequests,
    Arc<Mutex<bool>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(TaskStore::open(dir.path().join("history.json")).unwrap());
    let live = Arc::new(Mutex::new(true));
    let flag = live.clone();
    let lookup: OwnerLookup = Arc::new(move |id| {
        if ["manager", "successor", "foreign"].contains(&id) && *flag.lock().unwrap() {
            let mut row = owner();
            row["sessionId"] = id.into();
            Some(row)
        } else {
            None
        }
    });
    let pins: WorkflowPin = Arc::new(|_, _| Ok(pin()));
    let service = ManagerRequests::new(store.clone(), lookup, pins);
    (dir, store, service, live)
}
fn submitted(service: &ManagerRequests, content: &str, status: &str) -> String {
    let c = service.prepare("manager", content, false).unwrap();
    let id = c["requestId"].as_str().unwrap();
    let d = service.begin_delivery("manager", id).unwrap().unwrap();
    service
        .finish_delivery(id, d["deliveryId"].as_str().unwrap(), status)
        .unwrap();
    id.into()
}
fn create(key: &str) -> Value {
    json!({"key":key,"kind":"create","title":key,"cwd":"/project","provenance":"explicit","reason":"User requested work"})
}
fn resolve(service: &ManagerRequests, id: &str, intents: Value) -> Value {
    service.handle(json!({"op":"resolveRequest","requestId":id,"expectedRevision":service.request("manager",id).unwrap()["revision"],"intents":intents}),"manager")
}
#[test]
fn request_delivery_uncertainty_never_replays_and_resolution_is_idempotent() {
    let (dir, store, service, _) = manager_fixture();
    let capture = service.prepare("manager", "two features", false).unwrap();
    let id = capture["requestId"].as_str().unwrap();
    service.begin_delivery("manager", id).unwrap().unwrap();
    assert!(service.begin_delivery("manager", id).unwrap().is_none());
    let first = resolve(&service, id, json!([create("one"), create("two")]));
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(first["tasks"][0]["revision"], 1);
    let same = resolve(&service, id, json!([create("one"), create("two")]));
    assert_eq!(same["tasks"], first["tasks"]);
    assert_eq!(
        resolve(&service, id, json!([create("different")]))["code"],
        "conflict"
    );
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("history.json")).unwrap()).unwrap();
    assert!(persisted["requests"][0].get("userContent").is_none());
    assert_eq!(store.list().unwrap().len(), 2);
    assert!(
        store
            .validate_admission(&json!({"owner":owner(),"projectCwd":"/project"}))
            .is_err()
    );
    assert!(
        store
            .validate_admission(&json!({"owner":owner(),"projectCwd":"/project","trackTask":false}))
            .is_ok()
    );
}
#[test]
fn multi_intent_conflict_and_reference_failure_retain_original_content() {
    let (_dir, store, service, _) = manager_fixture();
    let id = submitted(&service, "feature", "accepted");
    let made = resolve(&service, &id, json!([create("one")]));
    let task = &made["tasks"][0];
    let later = submitted(&service, "next feature", "accepted");
    let response = resolve(
        &service,
        &later,
        json!([create("new"),{"key":"edit","kind":"update","taskId":task["taskId"],"expectedTaskRevision":0,"cwd":"/project","reason":"Update","title":"changed"}]),
    );
    assert_eq!(response["code"], "conflict");
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(
        service.request("manager", &later).unwrap()["userContent"],
        "next feature"
    );
    let url = submitted(
        &service,
        "Read https://example.com/a and https://example.com/b",
        "accepted",
    );
    let refused = resolve(&service, &url, json!([create("ambiguous")]));
    assert_eq!(refused["ok"], false);
    assert!(
        service
            .request("manager", &url)
            .unwrap()
            .get("userContent")
            .is_some()
    );
}
#[test]
fn manager_owner_recheck_and_original_reference_binding() {
    let (_dir, store, service, live) = manager_fixture();
    let id = submitted(
        &service,
        "Fix https://github.com/org/repo/pull/42",
        "accepted",
    );
    let response = resolve(&service, &id, json!([create("feature")]));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["tasks"][0]["links"]["pullRequest"]["number"], "42");
    let t = &response["tasks"][0];
    let task_id = t["taskId"].as_str().unwrap();
    assert!(
        store
            .mutate_owned(
                task_id,
                "foreign",
                "/project",
                revision(t),
                || Ok(()),
                |_| Ok(())
            )
            .is_err()
    );
    *live.lock().unwrap() = false;
    assert_eq!(
        service.handle(json!({"op":"requestInbox"}), "manager")["ok"],
        false
    );
}
#[test]
fn outcome_acceptance_requires_exact_current_evidence_and_unblocks_dependents() {
    let (_dir, store, service, _) = manager_fixture();
    let id = submitted(&service, "two features", "accepted");
    let mut follow = create("second");
    follow["kind"] = "followUp".into();
    follow["dependsOnKeys"] = json!(["first"]);
    let made = resolve(&service, &id, json!([create("first"), follow]));
    assert_eq!(made["ok"], true, "{made}");
    let first = made["tasks"][0]["taskId"].as_str().unwrap();
    let second = made["tasks"][1]["taskId"].as_str().unwrap();
    assert_eq!(
        dependency_state(
            &store.task(second).unwrap().unwrap(),
            &store.snapshot().unwrap().tasks
        ),
        "blocked"
    );
    let token = store
        .reserve_workflow_dispatch(
            first,
            revision(&store.task(first).unwrap().unwrap()),
            "implement",
        )
        .unwrap();
    store.accept(json!({"owner":owner(),"taskId":first,"workflowStepId":"implement","workflowReservationToken":token,"sessionId":"worker","projectCwd":"/project","executionCwd":"/project"}),||Ok(())).unwrap();
    store.release_workflow_dispatch(first, &token).unwrap();
    store
        .validated("worker", "valid", None, Some(json!({"done":true})))
        .unwrap();
    let current = store.task(first).unwrap().unwrap();
    let accepted=service.handle(json!({"op":"acceptTaskOutcome","taskId":first,"cwd":"/project","expectedTaskRevision":revision(&current),"reason":"Verified tests and diff"}),"manager");
    assert_eq!(accepted["ok"], true, "{accepted}");
    assert_eq!(accepted["readyTasks"][0]["taskId"], second);
    store.validated("worker", "invalid", None, None).unwrap();
    assert_eq!(
        dependency_state(
            &store.task(second).unwrap().unwrap(),
            &store.snapshot().unwrap().tasks
        ),
        "blocked"
    );
}

#[test]
fn rejected_delivery_can_retry_but_unknown_cannot_and_pending_content_is_bounded() {
    let (_dir, _store, service, _) = manager_fixture();
    let id = submitted(&service, "retry me", "rejected");
    let retry = service.begin_delivery("manager", &id).unwrap().unwrap();
    service
        .finish_delivery(&id, retry["deliveryId"].as_str().unwrap(), "unknown")
        .unwrap();
    assert!(service.begin_delivery("manager", &id).unwrap().is_none());
    let large = "x".repeat(17 * 1024);
    let large_id = submitted(&service, &large, "accepted");
    let pending = service.handle(json!({"op":"requestInbox","view":"pending"}), "manager");
    let large_row = pending["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["host"]["requestId"] == large_id)
        .unwrap();
    assert_eq!(large_row["contentDeferred"], true);
    assert!(large_row.get("userContent").is_none());
    let exact = service.handle(
        json!({"op":"requestContent","requestId":large_id}),
        "manager",
    );
    assert_eq!(exact["userContent"]["text"], large);
    assert_eq!(exact["userContent"]["trust"], "user");
}
#[test]
fn source_mapping_rejects_identifier_substrings_and_foreign_links() {
    use workspacer_hub::services::task_store::references::map_request;
    let mut intent = create("feature");
    intent["references"] = json!([{"kind":"ticket","id":"TASK-1"}]);
    assert!(map_request("Please fix éTASK-1 and TASK-1-extra", &[intent.clone()]).is_err());
    assert!(map_request("Please fix (TASK-1)", &[intent.clone()]).is_ok());
    intent["references"] =
        json!([{"kind":"reference","label":"Ticket","url":"https://example.com/foreign"}]);
    assert!(map_request("Please see https://example.com/real", &[intent]).is_err());
}
#[test]
fn reference_updates_are_idempotent_and_blocked_by_dispatch_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let store = TaskStore::open(dir.path().join("history.json")).unwrap();
    let task = store
        .start_workflow(&owner(), "/project", "Work", pin())
        .unwrap();
    let id = task["taskId"].as_str().unwrap();
    let refs = json!([{"kind":"ticket","id":"TASK-1","url":"https://example.com/task"}]);
    let changed = store
        .update_references(
            id,
            "manager",
            "/project",
            revision(&task),
            Some(&refs),
            None,
            || Ok(()),
        )
        .unwrap();
    let repeated = store
        .update_references(
            id,
            "manager",
            "/project",
            revision(&changed),
            Some(&refs),
            None,
            || Ok(()),
        )
        .unwrap();
    assert_eq!(revision(&changed), revision(&repeated));
    assert_eq!(repeated["audit"].as_array().unwrap().len(), 1);
    let token = store
        .reserve_workflow_dispatch(id, revision(&repeated), "implement")
        .unwrap();
    let current = store.task(id).unwrap().unwrap();
    assert!(
        store
            .update_references(
                id,
                "manager",
                "/project",
                revision(&current),
                Some(&refs),
                None,
                || Ok(())
            )
            .is_err()
    );
    store.release_workflow_dispatch(id, &token).unwrap();
}

#[test]
fn unchanged_observation_poll_does_not_advance_cas_revision_or_write_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.json");
    let store = TaskStore::open(path.clone()).unwrap();
    let receipt=store.accept(json!({"owner":owner(),"sessionId":"worker","projectCwd":"/project","executionCwd":"/project"}),||Ok(())).unwrap().unwrap();
    let snapshot = json!({"sessionId":"worker","status":"active","ambientState":"idle","statusLine":{"totalInputTokens":0,"costUSD":0}});
    store.observe_batch(&[snapshot.clone()]).unwrap();
    let id = receipt["taskId"].as_str().unwrap();
    let first = store.task(id).unwrap().unwrap();
    let raw = std::fs::read(&path).unwrap();
    store.observe_batch(&[snapshot.clone()]).unwrap();
    assert_eq!(
        revision(&store.task(id).unwrap().unwrap()),
        revision(&first)
    );
    assert_eq!(std::fs::read(&path).unwrap(), raw);
    assert_eq!(first["attempts"][0]["metrics"]["inputTokens"], 0);
    let mut changed = snapshot;
    changed["statusLine"]["totalInputTokens"] = 1.into();
    store.observe_batch(&[changed]).unwrap();
    assert_eq!(
        revision(&store.task(id).unwrap().unwrap()),
        revision(&first) + 1
    );
}

#[test]
fn accepted_attempt_recovery_does_not_reapply_new_dispatch_policy() {
    let dir = tempfile::tempdir().unwrap();
    let store = TaskStore::open(dir.path().join("history.json")).unwrap();
    let input = json!({"owner":owner(),"sessionId":"worker","projectCwd":"/project","executionCwd":"/project"});
    let receipt = store.accept(input.clone(), || Ok(())).unwrap().unwrap();
    store
        .transaction(|h| {
            h.task_mut(receipt["taskId"].as_str().unwrap())?["cancelled"] = true.into();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store.accept(input.clone(), || Ok(())).unwrap().unwrap(),
        receipt
    );
    let mut foreign = input;
    foreign["projectCwd"] = "/other".into();
    assert!(store.accept(foreign, || Ok(())).is_err());
}

#[test]
fn host_capture_during_transfer_intent_cannot_be_stranded_after_task_adoption() {
    use workspacer_hub::services::manager_replacements::ReplacementState;
    let (dir, store, service, _) = manager_fixture();
    let state = ReplacementState::open(dir.path().join("manager-replacements.json")).unwrap();
    service.set_replacements(state.clone());
    let prior = service
        .prepare("manager", "before transfer", false)
        .unwrap();
    state.edit(|d|{d.operations.push(json!({"operationId":uuid::Uuid::new_v4().to_string(),"sourceSessionId":"manager","successorSessionId":"successor","phase":"transferring","transferIntent":true,"committed":false,"bound":false,"paneId":"pane","workspaceId":"workspace","workerIds":[],"taskIds":[],"launch":{"options":{"manager":true,"toolScope":"operator","cwd":"/project"}},"metadata":[],"signatures":{},"finishes":{},"deliveries":[]}));Ok(())}).unwrap();
    store.adopt("manager", "successor").unwrap();
    let fresh = service
        .prepare("manager", "between adoption and ACK", false)
        .unwrap();
    let id = fresh["requestId"].as_str().unwrap();
    assert_eq!(
        service.request("successor", id).unwrap()["ownerSessionId"],
        "successor"
    );
    assert_eq!(
        service.request("successor", id).unwrap()["sourceSessionId"],
        "manager"
    );
    assert!(service.begin_delivery("manager", id).unwrap().is_some());
    assert!(
        service
            .host_request("manager", prior["requestId"].as_str().unwrap())
            .is_ok()
    );
    assert_eq!(
        service.handle(json!({"op":"requestContent","requestId":id}), "manager")["ok"],
        false
    );
}

#[test]
fn paired_receipt_updates_only_admitted_remote_attempts_without_result_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.json");
    let store = TaskStore::open(path.clone()).unwrap();
    store.transaction(|h|{let mut t=task("remote");t["attempts"]=json!([
 {"sessionId":"paired-session","executionTarget":"paired","dispatchId":"remote-dispatch","metrics":{},"resultContract":"absent"},
 {"sessionId":"local-session","executionTarget":"local","dispatchId":"local-dispatch","metrics":{},"resultContract":"absent"}]);h.tasks.push(t);Ok(())}).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(store.observe_remote("unknown", "running", false).is_err());
    assert!(
        store
            .observe_remote("local-session", "ended", true)
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    store
        .observe_remote("paired-session", "running", false)
        .unwrap();
    let running = store.task("remote").unwrap().unwrap();
    assert_eq!(running["attempts"][0]["live"], true);
    assert_eq!(running["attempts"][0]["stale"], false);
    let same = std::fs::read(&path).unwrap();
    store
        .observe_remote("paired-session", "running", false)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), same);
    store
        .observe_remote("paired-session", "idle", true)
        .unwrap();
    let ended = store.task("remote").unwrap().unwrap();
    assert_eq!(ended["attempts"][0]["lifecycle"], "idle");
    assert_eq!(ended["attempts"][0]["live"], true);
    assert_eq!(ended["attempts"][0]["resultContract"], "absent");
    assert!(
        store
            .observe_remote("paired-session", "validated", false)
            .is_err()
    );
    drop(store);
    let reopened = TaskStore::open(path).unwrap();
    assert_eq!(
        reopened.task("remote").unwrap().unwrap()["attempts"][0]["stale"],
        true
    );
}

#[cfg(windows)]
#[test]
fn windows_project_aliases_preserve_task_owner_revision_and_distinct_project_guards() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("ProjectMixedCase");
    let other = dir.path().join("DifferentProject");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&other).unwrap();
    let canonical = workspacer_hub::services::paths::canonicalize(&project).unwrap();
    let canonical = canonical.to_str().unwrap();
    let plain = canonical.strip_prefix(r"\\?\").unwrap_or(canonical);
    let git_style = plain.replace('\\', "/").to_lowercase();
    let store = TaskStore::open(dir.path().join("history.json")).unwrap();
    let task = store
        .start_workflow(&owner(), plain, "Raw stored spelling", pin())
        .unwrap();
    let id = task["taskId"].as_str().unwrap();
    let history = store.snapshot().unwrap();
    assert!(history.owned(id, "manager", canonical).is_ok());
    assert!(history.owned(id, "manager", &git_style).is_ok());
    assert!(history.owned(id, "other-manager", canonical).is_err());
    assert!(
        history
            .owned(id, "manager", other.to_str().unwrap())
            .is_err()
    );
    let edited = store
        .mutate_owned(
            id,
            "manager",
            canonical,
            revision(&task),
            || Ok(()),
            |task| {
                task["title"] = json!("Updated through canonical spelling");
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        edited["projectCwd"], plain,
        "comparison must not silently rewrite persisted identity"
    );
    assert_eq!(revision(&edited), revision(&task) + 1);
    assert!(
        store
            .mutate_owned(
                id,
                "manager",
                &git_style,
                revision(&task),
                || Ok(()),
                |_| Ok(())
            )
            .is_err()
    );
}

//! Retained hand-edit and proposal invariants, with real files and owned runtime.
use super::*;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
fn owner() -> Caller {
    Caller {
        call_id: 0,
        activity_seq: 0,
        federated: false,
        connection_id: 1,
        authenticated_host: true,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: "fixture".into(),
    }
}
fn row(id: &str, minutes: i64) -> Value {
    json!({"id":id,"name":if id.is_empty(){"fixture"}else{id},"enabled":true,"trigger":{"kind":"interval","everyMinutes":minutes},"action":{"kind":"call","call":{"method":"fixture.job"}},"createdAt":1,"updatedAt":1})
}
fn write(path: &std::path::Path, rows: Value) {
    std::fs::write(path, serde_json::to_vec(&json!({"jobs":rows})).unwrap()).unwrap();
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reload_preserves_anchors_recovers_and_persists_duplicate_identity_repairs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("jobs.json");
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let (service, _) = Service::open(path.clone(), hub.handle());
    let base = DateTime::parse_from_rfc3339("2026-08-24T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut a = row("a", 30);
    let b = row("b", 60);
    write(&path, json!([a, b]));
    let mut state = service.state.lock().unwrap();
    service.reload(&mut state, base);
    let original = state.next.clone();
    assert_eq!(original["a"], base + chrono::Duration::minutes(30));
    let metadata = std::fs::metadata(&path).unwrap();
    let modified = metadata.modified().unwrap();
    a["name"] = json!("c"); // Same-size edit within the same filesystem timestamp.
    write(&path, json!([a, b]));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), metadata.len());
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    service.reload(&mut state, base + chrono::Duration::minutes(10));
    assert_eq!(state.next, original, "a rename moved a schedule");
    assert_eq!(
        state.jobs[0].name, "c",
        "same-size/mtime edit was not reloaded"
    );
    a["trigger"]["everyMinutes"] = json!(90);
    write(&path, json!([a, b]));
    service.reload(&mut state, base + chrono::Duration::minutes(10));
    assert_eq!(state.next["a"], base + chrono::Duration::minutes(100));
    assert_eq!(state.next["b"], original["b"]);
    let expected = state.next.clone();
    std::fs::write(&path, b"{\"jobs\":[").unwrap();
    service.reload(&mut state, base);
    assert_eq!(state.next, expected);
    std::fs::remove_file(&path).unwrap();
    service.reload(&mut state, base);
    assert_eq!(state.next, expected);
    let mut invalid = row("invalid", 1);
    invalid["trigger"] = json!({"kind":"daily","at":"not a time"});
    write(&path, json!([a, b, invalid]));
    service.reload(&mut state, base);
    assert_eq!(state.jobs.len(), 2);
    assert_eq!(state.next, expected);
    let mut duplicate = row("a", 60);
    duplicate["createdAt"] = json!(0);
    duplicate["updatedAt"] = json!(0);
    write(&path, json!([a, duplicate]));
    service.reload(&mut state, base);
    assert_eq!(state.jobs.len(), 2);
    assert_ne!(state.jobs[0].id, state.jobs[1].id);
    assert!(state.jobs[1].created_at > 0 && state.jobs[1].updated_at > 0);
    let repaired: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(repaired["jobs"][1]["id"], state.jobs[1].id);
    let ids: Vec<_> = state.jobs.iter().map(|j| j.id.clone()).collect();
    let hash = state.hash.clone();
    let anchors = state.next.clone();
    service.reload(&mut state, base + chrono::Duration::minutes(10));
    assert_eq!(state.hash, hash);
    assert_eq!(state.next, anchors);
    assert_eq!(
        state.jobs.iter().map(|j| j.id.clone()).collect::<Vec<_>>(),
        ids
    );
    write(&path, json!([]));
    service.reload(&mut state, base);
    assert!(state.jobs.is_empty() && state.next.is_empty());
    drop(state);
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn proposals_cannot_overwrite_approved_jobs_and_notify_the_review_target() {
    use crate::protocol::Frame;
    let root = tempfile::tempdir().unwrap();
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let mut subscriber = hub.handle().connect().await.unwrap();
    subscriber.recv().await.unwrap();
    subscriber
        .send(Frame {
            topics: vec!["notify.post".into()],
            ..Frame::op("subscribe")
        })
        .unwrap();
    subscriber.recv().await.unwrap();
    let (service, mut queued) = Service::open(root.path().join("jobs.json"), hub.handle());
    let mut initial = row("", 30);
    initial["name"] = json!("approved");
    let approved = service.call(&owner(), "jobs.upsert", initial).unwrap();
    let mut proposed = approved.clone();
    proposed["action"]["call"]["method"] = json!("fixture.unapproved");
    proposed["proposedBy"] = json!("fixture agent");
    let proposal = service.call(&owner(), "jobs.propose", proposed).unwrap();
    assert_ne!(proposal["id"], approved["id"]);
    assert_eq!(proposal["enabled"], false);
    let notification = tokio::time::timeout(Duration::from_secs(2), subscriber.recv())
        .await
        .unwrap()
        .unwrap()
        .event
        .unwrap();
    assert_eq!(notification.source, "jobs");
    let data = notification.data.unwrap();
    assert_eq!(data["paneType"], "settings");
    assert_eq!(data["paneSection"], "jobs");
    assert_eq!(data["level"], "info");
    let list = service.call(&owner(), "jobs.list", Value::Null).unwrap();
    assert_eq!(list["jobs"][0]["action"], approved["action"]);
    assert!(
        service
            .call(&owner(), "jobs.run", json!({"id":proposal["id"]}))
            .is_err()
    );
    for _ in 1..20 {
        service.call(&owner(), "jobs.propose", row("", 1)).unwrap();
    }
    assert!(
        service
            .call(&owner(), "jobs.propose", row("", 1))
            .unwrap_err()
            .to_string()
            .contains("waiting for review")
    );
    let mut allowed = proposal;
    allowed["enabled"] = json!(true);
    allowed["proposedBy"] = json!("");
    let allowed = service.call(&owner(), "jobs.upsert", allowed).unwrap();
    assert!(
        service
            .state
            .lock()
            .unwrap()
            .next
            .contains_key(allowed["id"].as_str().unwrap())
    );
    assert_eq!(
        service
            .call(&owner(), "jobs.run", json!({"id":allowed["id"]}))
            .unwrap()["started"],
        true
    );
    assert_eq!(
        queued.try_recv().unwrap().id,
        allowed["id"].as_str().unwrap()
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn real_scheduler_polls_hand_edits_without_read_rpc_and_preserves_them_on_write() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("jobs.json");
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let hub = crate::Hub::start(Options::default().handler("fixture.job", move |_, _| {
        let calls = observed.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok":true}))
        }
    }))
    .unwrap();
    hub.ready().await.unwrap();
    let (service, receiver) = Service::open(path.clone(), hub.handle());
    let base = DateTime::parse_from_rfc3339("2026-08-24T09:00:00Z")
        .unwrap()
        .timestamp_millis();
    let clock = Arc::new(AtomicI64::new(base));
    let ticking = clock.clone();
    let worker = service.clone();
    let task = tokio::spawn(async move {
        worker
            .run_with_clock(receiver, Duration::from_millis(10), move || {
                DateTime::from_timestamp_millis(ticking.load(Ordering::SeqCst)).unwrap()
            })
            .await
    });
    let mut hand = row("", 10);
    hand["name"] = json!("hand");
    for key in ["id", "createdAt", "updatedAt"] {
        hand.as_object_mut().unwrap().remove(key);
    }
    write(&path, json!([hand]));
    until(|| service.state.lock().unwrap().jobs.len() == 1).await;
    let hand_id = {
        let state = service.state.lock().unwrap();
        assert!(!state.jobs[0].id.is_empty());
        assert!(state.jobs[0].created_at > 0 && state.jobs[0].updated_at > 0);
        state.jobs[0].id.clone()
    };
    let normalized: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(normalized["jobs"][0]["id"], hand_id);
    assert_eq!(
        service.state.lock().unwrap().next[&hand_id].timestamp_millis(),
        base + 600_000
    );
    clock.store(base + 600_001, Ordering::SeqCst);
    until(|| {
        calls.load(Ordering::SeqCst) == 1
            && !service.state.lock().unwrap().running.contains(&hand_id)
    })
    .await;
    let mut edited = row(&hand_id, 240);
    write(&path, json!([edited]));
    until(|| {
        service
            .state
            .lock()
            .unwrap()
            .next
            .get(&hand_id)
            .is_some_and(|v| v.timestamp_millis() == base + 600_001 + 240 * 60_000)
    })
    .await;
    let extra = row("external", 90);
    write(&path, json!([edited, extra]));
    until(|| service.state.lock().unwrap().next.contains_key("external")).await;
    edited["name"] = json!("renamed by owner");
    service.call(&owner(), "jobs.upsert", edited).unwrap();
    let disk: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(
        disk["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["id"] == "external")
    );
    write(&path, json!([]));
    until(|| service.state.lock().unwrap().jobs.is_empty()).await;
    let before = calls.load(Ordering::SeqCst);
    clock.fetch_add(48 * 60 * 60_000, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(calls.load(Ordering::SeqCst), before);
    let mut manual = row("manual", 1);
    manual["trigger"] = json!({"kind":"manual"});
    write(&path, json!([manual]));
    assert_eq!(
        service
            .call(&owner(), "jobs.run", json!({"id":"manual"}))
            .unwrap()["started"],
        true
    );
    until(|| {
        calls.load(Ordering::SeqCst) == before + 1
            && !service.state.lock().unwrap().running.contains("manual")
    })
    .await;
    task.abort();
    let _ = task.await;
    hub.shutdown().unwrap();
}

#[test]
fn malformed_trigger_action_and_context_shapes_never_validate() {
    let base = row("fixture", 1);
    let mut cases = Vec::new();
    let mut unnamed = base.clone();
    unnamed["name"] = json!(" ");
    cases.push(unnamed);
    for trigger in [
        json!({"kind":"cron"}),
        json!({"kind":"interval","everyMinutes":0}),
        json!({"kind":"daily","at":"25:99"}),
        json!({"kind":"daily","at":"09:00","days":[7]}),
        json!({"kind":"once","once":"tomorrow"}),
    ] {
        let mut job = base.clone();
        job["trigger"] = trigger;
        cases.push(job);
    }
    for action in [
        json!({"kind":"unknown"}),
        json!({"kind":"spawn","spawn":{"cwd":"/fixture","prompt":""}}),
        json!({"kind":"spawn","spawn":{"cwd":" ","prompt":"task"}}),
        json!({"kind":"call","call":{"method":""}}),
        json!({"kind":"call","call":{"method":"jobs.run"}}),
        json!({"kind":"call","call":{"method":"hub:peer/method"}}),
        json!({"kind":"shell","shell":{"command":" "}}),
    ] {
        let mut job = base.clone();
        job["action"] = action;
        cases.push(job);
    }
    for step in [
        json!({"kind":"unknown"}),
        json!({"kind":"shell","shell":{"command":""}}),
        json!({"kind":"call","call":{"method":""}}),
        json!({"kind":"call","call":{"method":"jobs.list"}}),
        json!({"kind":"call","call":{"method":"hub:peer/method"}}),
        json!({"kind":"shell","shell":{"command":"fixture"},"skipUnlessMatch":"["}),
    ] {
        let mut job = base.clone();
        job["action"] =
            json!({"kind":"spawn","spawn":{"cwd":"/fixture","prompt":"task","context":[step]}});
        cases.push(job);
    }
    let mut oversized = base.clone();
    oversized["action"] = json!({"kind":"spawn","spawn":{"cwd":"/fixture","prompt":"task","context":vec![json!({"kind":"shell","shell":{"command":"fixture"}});5]}});
    cases.push(oversized);
    assert_eq!(cases.len(), 20);
    for case in cases {
        let job: Job = serde_json::from_value(case.clone()).unwrap();
        assert!(validate(&job).is_err(), "accepted {case}");
    }
}
#[test]
fn daily_schedules_keep_go_gap_and_repeated_hour_normalization() {
    let zone = chrono_tz::America::New_York;
    for (after, at, expected) in [
        ("2026-03-08T05:00:00Z", "02:30", "2026-03-08T06:30:00Z"),
        ("2026-03-08T06:45:00Z", "02:30", "2026-03-09T06:30:00Z"),
        ("2026-11-01T04:00:00Z", "01:30", "2026-11-01T05:30:00Z"),
        ("2026-11-01T05:45:00Z", "01:30", "2026-11-02T06:30:00Z"),
    ] {
        let after = DateTime::parse_from_rfc3339(after)
            .unwrap()
            .with_timezone(&Utc);
        let expected = DateTime::parse_from_rfc3339(expected)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            next_run(
                &Trigger {
                    kind: "daily".into(),
                    at: at.into(),
                    ..Default::default()
                },
                after,
                &zone
            ),
            Some(expected)
        );
    }
}

#[tokio::test]
async fn explicit_null_files_clear_but_missing_or_malformed_files_keep_the_schedule() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("jobs.json");
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let (service, _) = Service::open(path.clone(), hub.handle());
    for clear in ["null", r#"{"jobs":null}"#, r#"{"jobs":[]}"#, "{}"] {
        write(&path, json!([row("stable", 30)]));
        assert_eq!(
            service.call(&owner(), "jobs.list", Value::Null).unwrap()["jobs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let next = service.state.lock().unwrap().next.clone();
        for broken in ["", " ", r#"{"jobs":["#, "false", r#"{"Jobs":[]}"#] {
            std::fs::write(&path, broken).unwrap();
            service.call(&owner(), "jobs.list", Value::Null).unwrap();
            assert_eq!(
                service.state.lock().unwrap().next,
                next,
                "{broken:?} changed schedule"
            );
        }
        std::fs::remove_file(&path).unwrap();
        service.call(&owner(), "jobs.list", Value::Null).unwrap();
        assert_eq!(service.state.lock().unwrap().next, next);
        std::fs::write(&path, clear).unwrap();
        assert_eq!(
            service.call(&owner(), "jobs.list", Value::Null).unwrap(),
            json!({"jobs":[]})
        );
        assert!(service.state.lock().unwrap().next.is_empty());
    }
    write(&path, json!([null, row("valid", 30)]));
    let jobs = service.call(&owner(), "jobs.list", Value::Null).unwrap();
    assert_eq!(jobs["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(jobs["jobs"][0]["id"], "valid");
    hub.shutdown().unwrap();
}
#[tokio::test]
async fn nullable_job_fields_follow_go_zero_values_and_wrong_types_remain_errors() {
    let root = tempfile::tempdir().unwrap();
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let (service, _) = Service::open(root.path().join("jobs.json"), hub.handle());
    let mut spec = json!({"id":null,"name":"daily","enabled":true,"proposedBy":null,"createdAt":null,"updatedAt":null,"trigger":{"kind":"daily","at":"09:00","days":null,"everyMinutes":null,"once":null},"action":{"kind":"call","call":{"method":"fixture"}}});
    let saved = service.call(&owner(), "jobs.upsert", spec.clone()).unwrap();
    assert!(!saved["id"].as_str().unwrap().is_empty());
    assert!(saved["createdAt"].as_i64().unwrap() > 0);
    assert!(saved.get("proposedBy").is_none());
    assert!(
        service.call(&owner(), "jobs.list", Value::Null).unwrap()["jobs"][0]
            .get("nextRunAt")
            .is_some()
    );
    spec["enabled"] = Value::Null;
    spec["trigger"]["days"] = json!([null, 1]);
    let disabled = service.call(&owner(), "jobs.upsert", spec.clone()).unwrap();
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["trigger"]["days"], json!([0, 1]));
    spec["enabled"] = json!("yes");
    assert!(service.call(&owner(), "jobs.upsert", spec.clone()).is_err());
    spec["enabled"] = json!(true);
    spec["trigger"]["days"] = json!(["Monday"]);
    assert!(service.call(&owner(), "jobs.upsert", spec).is_err());
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn empty_optional_run_fields_survive_persistence_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("jobs.json");
    let hub = crate::Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let (service, _) = Service::open(path.clone(), hub.handle());
    {
        let mut state = service.state.lock().unwrap();
        service.record(
            &mut state,
            Run {
                job_id: "silent-shell".into(),
                started_at: 1,
                finished_at: 2,
                status: "ok".into(),
                detail: String::new(),
            },
        );
        service.record(
            &mut state,
            Run {
                job_id: "old-empty".into(),
                status: "ok".into(),
                ..Default::default()
            },
        );
    }
    let raw: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("jobs-history.json")).unwrap())
            .unwrap();
    assert!(raw["runs"]["silent-shell"][0].get("detail").is_none());
    assert!(raw["runs"]["old-empty"][0].get("finishedAt").is_none());
    let (reopened, _) = Service::open(path, hub.handle());
    for id in ["silent-shell", "old-empty"] {
        let history = reopened
            .call(&owner(), "jobs.history", json!({"id":id}))
            .unwrap();
        assert_eq!(history["runs"].as_array().unwrap().len(), 1, "lost {id}");
        assert_eq!(history["runs"][0]["status"], "ok");
    }
    let run: Run = serde_json::from_value(
        json!({"jobId":null,"status":null,"detail":null,"startedAt":null,"finishedAt":null}),
    )
    .unwrap();
    assert!(run.job_id.is_empty() && run.status.is_empty() && run.detail.is_empty());
    assert_eq!(run.started_at, 0);
    assert_eq!(run.finished_at, 0);
    hub.shutdown().unwrap();
}

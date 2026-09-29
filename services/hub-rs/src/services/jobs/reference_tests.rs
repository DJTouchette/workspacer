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
    a["name"] = json!("renamed");
    write(&path, json!([a, b]));
    service.reload(&mut state, base + chrono::Duration::minutes(10));
    assert_eq!(state.next, original, "a rename moved a schedule");
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
    write(&path, json!([row("hand", 10)]));
    until(|| service.state.lock().unwrap().next.contains_key("hand")).await;
    assert_eq!(
        service.state.lock().unwrap().next["hand"].timestamp_millis(),
        base + 600_000
    );
    clock.store(base + 600_001, Ordering::SeqCst);
    until(|| {
        calls.load(Ordering::SeqCst) == 1 && !service.state.lock().unwrap().running.contains("hand")
    })
    .await;
    let mut edited = row("hand", 240);
    write(&path, json!([edited]));
    until(|| {
        service
            .state
            .lock()
            .unwrap()
            .next
            .get("hand")
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
    task.abort();
    let _ = task.await;
    hub.shutdown().unwrap();
}

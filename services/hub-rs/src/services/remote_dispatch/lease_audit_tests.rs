use super::*;

fn owner() -> Caller {
    let mut owner = caller();
    owner.federated = true;
    owner.authenticated_host = false;
    owner.token_id = "verified-lease-owner".into();
    owner
}
fn arguments(owner: &Caller) -> Value {
    json!({"remoteOrigin":{"protocol":PROTOCOL,"dispatchId":"abcdefghijklmnop","ownerKey":owner.token_id},"cwd":"Q:\\remote-only\\repo","provider":"claude"})
}

#[tokio::test]
async fn lease_rejects_each_changed_binding_before_first_execution_and_remains_single_use() {
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    for mutation in ["owner", "cwd", "provider", "expired", "protocol"] {
        let root = tempfile::tempdir().unwrap();
        let execution = FakeExecution::new();
        let owner = owner();
        let valid = arguments(&owner);
        let mut receiver =
            Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
        receiver
            .prepare(owner.clone(), valid.clone())
            .await
            .unwrap();
        let mut changed = valid.clone();
        let mut changed_owner = owner.clone();
        match mutation {
            "owner" => {
                changed_owner.token_id = "different-authenticated-owner".into();
                changed["remoteOrigin"]["ownerKey"] = json!(changed_owner.token_id);
            }
            "cwd" => changed["cwd"] = json!("Q:\\different\\repo"),
            "provider" => changed["provider"] = json!("codex"),
            "protocol" => changed["remoteOrigin"]["protocol"] = json!(1),
            "expired" => {
                receiver.close().await;
                drop(receiver);
                let path = root.path().join("remote-dispatch-worker.json");
                let mut rows: Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                rows[0]["lease"]["expires"] = json!(1);
                std::fs::write(path, serde_json::to_vec(&rows).unwrap()).unwrap();
                receiver =
                    Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            receiver.spawn(changed_owner, changed).await.is_err(),
            "{mutation}"
        );
        assert_eq!(execution.starts.load(Ordering::SeqCst), 0, "{mutation}");
        if mutation != "expired" {
            // A rejected mismatched request must not consume the genuine lease.
            receiver.spawn(owner.clone(), valid.clone()).await.unwrap();
            assert!(receiver.spawn(owner.clone(), valid.clone()).await.is_err());
            receiver.close().await;
            drop(receiver);
            receiver = Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
            assert!(receiver.spawn(owner.clone(), valid.clone()).await.is_err());
            assert_eq!(execution.starts.load(Ordering::SeqCst), 1, "{mutation}");
        }
        receiver.close().await;
    }
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn journal_failure_prevents_execution_and_publication_and_terminal_ack_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let execution = FakeExecution::new();
    let owner = owner();
    let args = arguments(&owner);
    let receiver = Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
    receiver.prepare(owner.clone(), args.clone()).await.unwrap();
    let path = root.path().join("remote-dispatch-worker.json");
    let backup = root.path().join("saved-journal.json");
    let obstruct = || {
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
    };
    let restore = || {
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&backup, &path).unwrap();
    };
    obstruct();
    assert!(receiver.spawn(owner.clone(), args.clone()).await.is_err());
    assert_eq!(execution.starts.load(Ordering::SeqCst), 0);
    restore();
    let result = receiver.spawn(owner.clone(), args).await.unwrap();
    let session = result["sessionId"].as_str().unwrap();
    let viewer = crate::client::Client::connect(&hub.handle()).await.unwrap();
    let mut events = viewer.events();
    viewer
        .topics(["agent.dispatch.update".into()].into())
        .await
        .unwrap();
    obstruct();
    assert!(
        receiver
            .report(
                session,
                Kind::WorkerFinished,
                json!({"label":"worker","needsDecision":true})
            )
            .await
            .is_err()
    );
    assert!(
        events.try_recv().is_err(),
        "a nondurable receipt was published"
    );
    restore();
    for (seq, kind) in [
        (1, Kind::Progress),
        (2, Kind::Blocked),
        (3, Kind::WorkerFinished),
    ] {
        assert!(
            receiver
                .report(
                    session,
                    kind,
                    json!({"label":"worker","note":"phase finished","needsDecision":true})
                )
                .await
                .unwrap()
        );
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        let data = event.data.unwrap();
        assert_eq!(data["seq"], seq);
        assert_eq!(data["entry"]["sessionId"], session);
        assert_eq!(data["entry"]["label"], "worker");
        assert_eq!(data["entry"]["note"], "phase finished");
        assert_eq!(data["entry"]["needsDecision"], true);
        assert!(data["entry"].get("SessionID").is_none());
    }
    receiver.forget(session);
    receiver.close().await;
    drop(receiver);
    let receiver = Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
    let replay = receiver
        .replay(&owner, json!({"dispatchId":"abcdefghijklmnop"}))
        .await
        .unwrap();
    assert_eq!(replay["seq"], 3);
    assert_eq!(replay["sessionId"], session);
    assert!(
        !receiver
            .report(session, Kind::Progress, json!({"label":"late"}))
            .await
            .unwrap()
    );
    let mut other = owner.clone();
    other.token_id = "other-owner".into();
    assert!(
        receiver
            .replay(
                &other,
                json!({"dispatchId":"abcdefghijklmnop","ackedSeq":3})
            )
            .await
            .is_err()
    );
    receiver
        .replay(
            &owner,
            json!({"dispatchId":"abcdefghijklmnop","ackedSeq":3}),
        )
        .await
        .unwrap();
    receiver.close().await;
    drop(receiver);
    let stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(stored[0]["ackedAt"].as_i64().unwrap() > 0);
    assert_eq!(stored[0]["last"]["seq"], 3);
    assert_eq!(stored[0]["last"]["final"], true);
    let receiver = Receiver::open(root.path().into(), hub.handle(), execution).unwrap();
    assert_eq!(
        receiver
            .replay(&owner, json!({"dispatchId":"abcdefghijklmnop"}))
            .await
            .unwrap()["seq"],
        3
    );
    receiver.close().await;
    drop(viewer);
    hub.shutdown().unwrap();
}

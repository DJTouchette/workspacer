use super::*;
use serde_json::json;
#[test]
fn receipt_shapes_and_provider_readiness_fail_closed() {
    assert!(valid_id("abcdefghijklmnop"));
    assert!(!valid_id("../abcdefghijklmnop"));
    let entry = sanitize_entry(
        &json!({"label":" A\n B ","sessionId":"spoof","reviewEvidenceId":"local","fullReply":"answer"}),
        "worker",
    );
    assert_eq!(entry["sessionId"], "worker");
    assert_eq!(entry["label"], "A B");
    assert!(entry.get("reviewEvidenceId").is_none());
    assert_eq!(
        readiness::login_from_output("claude", br#"{"loggedIn":true}"#, false),
        None
    );
    assert_eq!(
        readiness::login_from_output("codex", b"Not logged in", true),
        Some(false)
    );
}
#[test]
fn legacy_journals_are_discriminated_without_overwriting_each_other() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join("remote-dispatches.json");
    std::fs::write(&legacy, br#"[{"id":"abcdefghijklmnop"}]"#).unwrap();
    let worker = Journal::<WorkerRecord>::open_legacy(
        root.path().join("worker.json"),
        legacy.clone(),
        "id",
        "dispatchId",
        |r| Ok(r.id.clone()),
    )
    .unwrap();
    assert_eq!(worker.list().len(), 1);
    let origin = Journal::<OriginRecord>::open_legacy(
        root.path().join("origin.json"),
        legacy.clone(),
        "dispatchId",
        "id",
        |r| Ok(r.dispatch_id.clone()),
    )
    .unwrap();
    assert!(origin.list().is_empty());
    assert!(legacy.exists());
}

use crate::{Caller, Handle, Hub, Options, protocol::Event, services::agent_lifecycle::Operation};
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
struct FakeExecution {
    starts: AtomicUsize,
    launches: Mutex<Vec<Value>>,
    uncertain: AtomicBool,
    paused: AtomicBool,
    release: tokio::sync::Notify,
    allocations: AtomicUsize,
    cleanups: AtomicUsize,
    allocation_failed: AtomicBool,
    cleanup_failed: AtomicBool,
}
impl FakeExecution {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            starts: AtomicUsize::new(0),
            launches: Mutex::new(vec![]),
            uncertain: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
            allocations: AtomicUsize::new(0),
            cleanups: AtomicUsize::new(0),
            allocation_failed: AtomicBool::new(false),
            cleanup_failed: AtomicBool::new(false),
        })
    }
}
impl Execution for FakeExecution {
    fn capabilities(&self) -> Operation<'_, Capabilities> {
        Box::pin(async {
            Ok(Capabilities {
                protocol: PROTOCOL,
                exact_model: true,
                executes: true,
                scope: "local".into(),
                providers: vec![Provider {
                    provider: "claude".into(),
                    found: true,
                    authenticated: Some(true),
                    note: String::new(),
                }],
                cwds: vec![Directory {
                    path: "Q:\\remote-only\\repo".into(),
                    source: "fixture".into(),
                    git: true,
                }],
                unsupported_reason: None,
            })
        })
    }
    fn canonical_directory<'a>(&'a self, cwd: &'a str) -> Operation<'a, String> {
        Box::pin(async move { Ok(cwd.into()) })
    }
    fn allocate<'a>(
        &'a self,
        _repo: &'a str,
        _cwd: &'a str,
        _branch: &'a str,
    ) -> Operation<'a, ()> {
        Box::pin(async move {
            self.allocations.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(
                !self.allocation_failed.load(Ordering::SeqCst),
                "allocation interrupted"
            );
            Ok(())
        })
    }
    fn cleanup<'a>(&'a self, _repo: &'a str, _cwd: &'a str, _branch: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.cleanups.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(
                !self.cleanup_failed.load(Ordering::SeqCst),
                "dirty worktree"
            );
            Ok(())
        })
    }
    fn spawn<'a>(
        &'a self,
        caller: Caller,
        admission: RemoteAdmission,
        params: Value,
    ) -> Operation<'a, Value> {
        Box::pin(async move {
            assert!(caller.federated);
            assert_eq!(admission.repo_cwd(), "Q:\\remote-only\\repo");
            self.launches.lock().unwrap().push(params);
            self.starts.fetch_add(1, Ordering::SeqCst);
            if self.paused.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            if self.uncertain.load(Ordering::SeqCst) {
                anyhow::bail!("acknowledgement lost after execution")
            }
            Ok(json!({"sessionId":admission.session_id(),"messageQueued":true}))
        })
    }
}
#[derive(Default)]
struct Sink {
    sent: Mutex<Vec<Update>>,
    uncertain: AtomicBool,
}
impl Delivery for Sink {
    fn recipient<'a>(&'a self, owner: &'a str) -> Operation<'a, Option<String>> {
        Box::pin(async move { Ok((owner == "local-owner").then(|| owner.into())) })
    }
    fn deliver<'a>(
        &'a self,
        recipient: &'a str,
        _record: &'a OriginRecord,
        update: &'a Update,
    ) -> Operation<'a, ()> {
        Box::pin(async move {
            assert_eq!(recipient, "local-owner");
            self.sent.lock().unwrap().push(update.clone());
            if self.uncertain.load(Ordering::SeqCst) {
                anyhow::bail!("wake acknowledgement unknown")
            };
            Ok(())
        })
    }
}
fn caller() -> Caller {
    Caller {
        call_id: 1,
        activity_seq: 0,
        connection_id: 1,
        federated: false,
        authenticated_host: true,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: String::new(),
    }
}
async fn pair(
    local: &Handle,
    address: std::net::SocketAddr,
    token: &str,
    dispatch: bool,
) -> crate::federation::Manager {
    let manager = crate::federation::Manager::start(
        local.clone(),
        vec![crate::federation::Peer {
            name: "worker".into(),
            url: format!("ws://{address}/bus"),
            token: token.into(),
            dispatch,
        }],
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !manager.peers()[0].connected {
            tokio::time::sleep(Duration::from_millis(10)).await
        }
    })
    .await
    .unwrap();
    manager
}
fn params() -> Value {
    json!({"cwd":"Q:\\remote-only\\repo","provider":"claude","dispatchOwnerSessionId":"local-owner","message":"work","exactModel":true})
}
#[tokio::test]
async fn two_hubs_bind_lease_to_link_identity_and_deliver_only_verified_receipts() {
    let root = tempfile::tempdir().unwrap();
    let origin_dir = tempfile::tempdir().unwrap();
    let execution = FakeExecution::new();
    let tokens = root.path().join("tokens.json");
    let operator = crate::auth::mint(&tokens, crate::auth::Scope::Operator, "link").unwrap();
    let viewer = crate::auth::mint(&tokens, crate::auth::Scope::View, "view-link").unwrap();
    let mut options = Options::default();
    options.listen = Some("127.0.0.1:0".parse().unwrap());
    options.token = "actual-host".into();
    options.scoped_tokens = Some(tokens);
    options.config_dir = Some(root.path().into());
    options.remote_dispatch_execution = Some(execution.clone());
    let remote = Hub::start(options).unwrap();
    let address = remote.ready().await.unwrap().unwrap();
    let local = Hub::start(Options::default()).unwrap();
    local.ready().await.unwrap();
    let mut manager = pair(&local.handle(), address, &operator.token, true).await;
    let sink = Arc::new(Sink::default());
    let origin = Origin::open(
        origin_dir.path().into(),
        local.handle(),
        Arc::new(manager.routes()),
        sink.clone(),
    )
    .unwrap();
    let result = origin
        .forward_sanitized(&caller(), "worker", params())
        .await
        .unwrap();
    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    assert_eq!(result["executionCwd"], "Q:\\remote-only\\repo");
    let id = result["dispatchId"].as_str().unwrap();
    let session = result["sessionId"].as_str().unwrap();
    let stamp = json!({"protocol":PROTOCOL,"dispatchId":id,"ownerKey":"forged"});
    assert!(
        manager
            .routes()
            .forward(
                "worker",
                "agents.spawn",
                json!({"remoteOrigin":stamp,"cwd":"Q:\\remote-only\\repo","provider":"claude"})
            )
            .await
            .is_err()
    );
    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    let update = Update {
        protocol: PROTOCOL,
        dispatch_id: id.into(),
        kind: Kind::WorkerFinished,
        session_id: session.into(),
        seq: 1,
        ts: now(),
        terminal: true,
        entry: sanitize_entry(
            &json!({"label":"worker","fullReply":"done","reviewEvidenceId":"spoof"}),
            session,
        ),
    };
    let mut event = Event::new(
        "agent.dispatch.update",
        "brain",
        serde_json::to_value(update).unwrap(),
    );
    assert!(!origin.update(&event).await.unwrap());
    event.hub = "wrong-peer".into();
    assert!(!origin.update(&event).await.unwrap());
    event.hub = String::new();
    let listener = crate::client::Client::connect(&local.handle())
        .await
        .unwrap();
    let mut events = listener.events();
    listener
        .topics(std::collections::BTreeSet::from([
            "agent.dispatch.update".into()
        ]))
        .await
        .unwrap();
    remote.handle().publish_wait(event).await.unwrap();
    let forwarded = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(forwarded.hub, "worker");
    assert!(origin.update(&forwarded).await.unwrap());
    assert!(!origin.update(&forwarded).await.unwrap());
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
    assert_eq!(origin.records()[0].state, "done");
    manager.shutdown().await;
    let mut limited = pair(&local.handle(), address, &viewer.token, true).await;
    let denied = Origin::open(
        origin_dir.path().join("view"),
        local.handle(),
        Arc::new(limited.routes()),
        sink.clone(),
    )
    .unwrap();
    assert!(
        denied
            .forward_sanitized(&caller(), "worker", params())
            .await
            .is_err()
    );
    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    limited.shutdown().await;
    let mut disabled = pair(&local.handle(), address, &operator.token, false).await;
    let denied = Origin::open(
        origin_dir.path().join("disabled"),
        local.handle(),
        Arc::new(disabled.routes()),
        sink,
    )
    .unwrap();
    assert!(
        denied
            .forward_sanitized(&caller(), "worker", params())
            .await
            .unwrap_err()
            .to_string()
            .contains("not enabled")
    );
    disabled.shutdown().await;
    local.shutdown().unwrap();
    remote.shutdown().unwrap();
}

#[tokio::test]
async fn uncertain_claim_survives_restart_and_cannot_be_replayed_or_reassigned() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let execution = FakeExecution::new();
    execution.uncertain.store(true, Ordering::SeqCst);
    let receiver = Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
    let id = "abcdefghijklmnop";
    let mut owner = caller();
    owner.federated = true;
    owner.token_id = "verified-link-fingerprint".into();
    owner.authenticated_host = false;
    let args = json!({"remoteOrigin":{"protocol":PROTOCOL,"dispatchId":id,"ownerKey":owner.token_id},"cwd":"Q:\\remote-only\\repo","provider":"claude"});
    receiver.prepare(owner.clone(), args.clone()).await.unwrap();
    assert!(
        receiver
            .spawn(owner.clone(), args.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("acknowledgement lost")
    );
    receiver.close().await;
    drop(receiver);
    let receiver = Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
    assert!(receiver.prepare(owner.clone(), args.clone()).await.is_err());
    assert!(receiver.spawn(owner.clone(), args).await.is_err());
    assert_eq!(receiver.sweep_at(i64::MAX).await.unwrap(), 0);
    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    let replay = receiver
        .replay(&owner, json!({"dispatchId":id}))
        .await
        .unwrap();
    let session = replay["sessionId"].as_str().unwrap();
    assert!(!session.is_empty());
    let mut other = owner.clone();
    other.token_id = "another-link".into();
    assert!(
        receiver
            .replay(&other, json!({"dispatchId":id}))
            .await
            .is_err()
    );
    assert!(
        receiver
            .report(
                session,
                Kind::Progress,
                json!({"label":"worker","note":"working"})
            )
            .await
            .unwrap()
    );
    assert!(
        receiver
            .replay(&owner, json!({"dispatchId":id,"ackedSeq":1}))
            .await
            .is_err()
    );
    assert!(
        receiver
            .report(
                session,
                Kind::WorkerFinished,
                json!({"label":"worker","fullReply":"done"})
            )
            .await
            .unwrap()
    );
    assert!(
        !receiver
            .report(
                session,
                Kind::WorkerFinished,
                json!({"label":"worker","fullReply":"different"})
            )
            .await
            .unwrap()
    );
    assert_eq!(
        receiver
            .replay(&owner, json!({"dispatchId":id}))
            .await
            .unwrap()["seq"],
        2
    );
    receiver
        .replay(&owner, json!({"dispatchId":id,"ackedSeq":2}))
        .await
        .unwrap();
    receiver.close().await;
    hub.shutdown().unwrap();
}
struct NoLink;
impl Link for NoLink {
    fn dispatch_enabled(&self, _: &str) -> bool {
        false
    }
    fn call<'a>(&'a self, _: &'a str, _: &'a str, _: Value) -> Operation<'a, Value> {
        Box::pin(async { Ok(json!({"state":"unknown"})) })
    }
}
#[tokio::test]
async fn unknown_wake_delivery_is_retained_across_restart_without_resend() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let sink = Arc::new(Sink::default());
    sink.uncertain.store(true, Ordering::SeqCst);
    let record = OriginRecord {
        dispatch_id: "abcdefghijklmnop".into(),
        peer: "worker".into(),
        owner_session_id: "local-owner".into(),
        session_id: Some("worker-session".into()),
        cwd: "remote".into(),
        provider: "claude".into(),
        model: String::new(),
        label: String::new(),
        local_session_id: None,
        opened_at: now(),
        acked_seq: 0,
        delivering_seq: None,
        last_update: None,
        state: "open".into(),
        note: None,
        result_schema: None,
    };
    std::fs::write(
        root.path().join("remote-dispatch-origin.json"),
        serde_json::to_vec(&vec![record]).unwrap(),
    )
    .unwrap();
    let origin = Origin::open(
        root.path().into(),
        hub.handle(),
        Arc::new(NoLink),
        sink.clone(),
    )
    .unwrap();
    let update = Update {
        protocol: PROTOCOL,
        dispatch_id: "abcdefghijklmnop".into(),
        kind: Kind::WorkerFinished,
        session_id: "worker-session".into(),
        seq: 1,
        ts: now(),
        terminal: true,
        entry: json!({"label":"worker","sessionId":"worker-session","fullReply":"retained"}),
    };
    let mut event = Event::new(
        "agent.dispatch.update",
        "brain",
        serde_json::to_value(update).unwrap(),
    );
    event.hub = "worker".into();
    assert!(origin.update(&event).await.is_err());
    drop(origin);
    sink.uncertain.store(false, Ordering::SeqCst);
    let origin = Origin::open(
        root.path().into(),
        hub.handle(),
        Arc::new(NoLink),
        sink.clone(),
    )
    .unwrap();
    assert!(!origin.update(&event).await.unwrap());
    origin.reconcile("worker").await.unwrap();
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
    let records = origin.records();
    assert_eq!(records[0].delivering_seq, Some(1));
    assert_eq!(records[0].state, "open");
    assert_eq!(
        records[0].last_update.as_ref().unwrap().entry["fullReply"],
        "retained"
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn owned_runtime_intercepts_remote_spawn_and_observes_return_channel() {
    let worker_root = tempfile::tempdir().unwrap();
    let origin_root = tempfile::tempdir().unwrap();
    let execution = FakeExecution::new();
    let mut worker_options = Options::default();
    worker_options.listen = Some("127.0.0.1:0".parse().unwrap());
    worker_options.token = "paired-fixture".into();
    worker_options.config_dir = Some(worker_root.path().into());
    worker_options.remote_dispatch_execution = Some(execution.clone());
    let worker = Hub::start(worker_options).unwrap();
    let address = worker.ready().await.unwrap().unwrap();
    let sink = Arc::new(Sink::default());
    let mut origin_options = Options::default();
    origin_options.config_dir = Some(origin_root.path().into());
    origin_options.remote_dispatch_delivery = Some(sink.clone());
    origin_options.federation_peers = vec![crate::federation::Peer {
        name: "worker".into(),
        url: format!("ws://{address}/bus"),
        token: "paired-fixture".into(),
        dispatch: true,
    }];
    let origin = Hub::start(origin_options).unwrap();
    origin.ready().await.unwrap();
    let owner = crate::client::Client::connect(&origin.handle())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while owner.call("federation.peers", Value::Null).await.unwrap()[0]["connected"] != true {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let response = owner
        .call("hub:worker/agents.spawn", params())
        .await
        .unwrap();
    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    let replay = owner
        .call(
            "hub:worker/agents.dispatchReplay",
            json!({"dispatchId":response["dispatchId"],"originKey":"forged"}),
        )
        .await
        .unwrap();
    assert_eq!(replay["state"], "running");
    assert_eq!(replay["sessionId"], response["sessionId"]);
    let update = Update {
        protocol: PROTOCOL,
        dispatch_id: response["dispatchId"].as_str().unwrap().into(),
        kind: Kind::WorkerFinished,
        session_id: response["sessionId"].as_str().unwrap().into(),
        seq: 1,
        ts: now(),
        terminal: true,
        entry: json!({"sessionId":response["sessionId"],"label":"worker","fullReply":"complete"}),
    };
    worker
        .handle()
        .publish_wait(Event::new(
            "agent.dispatch.update",
            "brain",
            serde_json::to_value(update).unwrap(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let rows: Vec<OriginRecord> = serde_json::from_slice(
                &std::fs::read(origin_root.path().join("remote-dispatch-origin.json")).unwrap(),
            )
            .unwrap();
            if rows[0].state == "done" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let rows: Vec<OriginRecord> = serde_json::from_slice(
        &std::fs::read(origin_root.path().join("remote-dispatch-origin.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(rows[0].state, "done");
    origin.shutdown().unwrap();
    worker.shutdown().unwrap();
}

#[tokio::test]
async fn cancelled_rpc_keeps_execution_owned_until_receiver_shutdown_finishes() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let execution = FakeExecution::new();
    execution.paused.store(true, Ordering::SeqCst);
    let receiver = Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
    let mut owner = caller();
    owner.federated = true;
    owner.token_id = "lease-owner".into();
    let args = json!({"remoteOrigin":{"protocol":2,"dispatchId":"abcdefghijklmnop","ownerKey":"lease-owner"},"cwd":"Q:\\remote-only\\repo","provider":"claude"});
    receiver.prepare(owner.clone(), args.clone()).await.unwrap();
    let admitted = receiver.clone();
    let caller_task = tokio::spawn(async move { admitted.spawn(owner, args).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while execution.starts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    caller_task.abort();
    let _ = caller_task.await;
    let closing = receiver.clone();
    let mut close = tokio::spawn(async move { closing.close().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut close)
            .await
            .is_err()
    );
    execution.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), close)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    hub.shutdown().unwrap();
}
#[tokio::test]
async fn incomplete_worktree_allocations_are_journaled_and_only_safe_unclaimed_expirations_cleaned()
{
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let execution = FakeExecution::new();
    execution.allocation_failed.store(true, Ordering::SeqCst);
    let receiver = Receiver::open(root.path().into(), hub.handle(), execution.clone()).unwrap();
    let mut owner = caller();
    owner.federated = true;
    owner.token_id = "lease-owner".into();
    let args = json!({"remoteOrigin":{"protocol":2,"dispatchId":"abcdefghijklmnop","ownerKey":"lease-owner"},"cwd":"Q:\\remote-only\\repo","provider":"claude","worktree":true});
    assert!(receiver.prepare(owner.clone(), args.clone()).await.is_err());
    assert_eq!(execution.allocations.load(Ordering::SeqCst), 1);
    execution.allocation_failed.store(false, Ordering::SeqCst);
    assert!(receiver.prepare(owner, args).await.is_err());
    assert_eq!(execution.allocations.load(Ordering::SeqCst), 1);
    execution.cleanup_failed.store(true, Ordering::SeqCst);
    assert_eq!(receiver.sweep_at(i64::MAX).await.unwrap(), 0);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 1);
    execution.cleanup_failed.store(false, Ordering::SeqCst);
    assert_eq!(receiver.sweep_at(i64::MAX).await.unwrap(), 1);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 2);
    receiver.close().await;
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn paired_workflow_books_local_proxy_before_remote_execution_and_unknown_is_not_retried() {
    let worker_root = tempfile::tempdir().unwrap();
    let local_root = tempfile::tempdir().unwrap();
    let execution = FakeExecution::new();
    let mut worker_options = Options::default();
    worker_options.listen = Some("127.0.0.1:0".parse().unwrap());
    worker_options.token = "paired-worker".into();
    worker_options.config_dir = Some(worker_root.path().into());
    worker_options.remote_dispatch_execution = Some(execution.clone());
    let worker = Hub::start(worker_options).unwrap();
    let address = worker.ready().await.unwrap().unwrap();
    std::fs::write(
        local_root.path().join("remote-server.json"),
        serde_json::to_vec(
            &json!({"mode":"workers","url":format!("http://{address}"),"token":"paired-worker"}),
        )
        .unwrap(),
    )
    .unwrap();
    let project = crate::services::paths::canonicalize(local_root.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut options = Options::default();
    options.config_dir = Some(local_root.path().into());
    options.remote_dispatch_delivery = Some(Arc::new(Sink::default()));
    options.session_snapshots.write().unwrap().insert(
        "local-owner".into(),
        json!({"sessionId":"local-owner","isWakeTarget":true,"status":"running","cwd":project}),
    );
    let projections = options.remote_proxy_snapshots.clone();
    let local = Hub::start(options).unwrap();
    local.ready().await.unwrap();
    let owner = crate::client::Client::connect(&local.handle())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while owner
            .call("fleet.dispatchTargets", Value::Null)
            .await
            .unwrap()["targets"][0]["ready"]
            != true
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let selection = owner
        .call(
            "fleet.selectDispatchModel",
            json!({"cwd":"Q:\\remote-only\\repo","role":"implementer"}),
        )
        .await
        .unwrap();
    assert!(selection["eligible"].is_boolean());
    if selection["eligible"] == true {
        assert_eq!(selection["provider"], "claude");
    }
    let task=owner.call("fleetWorkflows.request",json!({"op":"start","callerSessionId":"local-owner","cwd":project,"title":"Paired feature"})).await.unwrap()["task"].clone();
    assert!(task["taskId"].is_string(), "{task}");
    let next=owner.call("fleetWorkflows.request",json!({"op":"next","callerSessionId":"local-owner","taskId":task["taskId"],"cwd":project})).await.unwrap();
    let plan = &next["dispatch"];
    let params = json!({"executionTarget":"paired","remoteCwd":"Q:\\remote-only\\repo","cwd":project,"taskId":task["taskId"],"workflowStepId":plan["stepId"],"expectedTaskRevision":plan["expectedTaskRevision"],"parentSessionId":"local-owner","dispatchOwnerSessionId":"local-owner","stage":plan["stage"],"role":plan["role"],"template":plan["template"],"provider":"claude","model":"sonnet","templateParams":{"task":"Implement the scoped change"}});
    let result = owner.call("agents.spawn", params.clone()).await.unwrap();
    assert!(result["sessionId"].as_str().unwrap().starts_with("paired:"));
    assert_ne!(result["sessionId"], result["remoteSessionId"]);
    assert_ne!(result["dispatchId"], result["remoteDispatchId"]);
    assert_eq!(result["executionTarget"], "paired");
    let history = crate::services::task_store::TaskStore::open(
        local_root.path().join("dispatch-history.json"),
    )
    .unwrap();
    let task = history
        .task(task["taskId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    let attempt = &task["attempts"][0];
    assert_eq!(attempt["sessionId"], result["sessionId"]);
    assert_eq!(attempt["executionTarget"], "paired");
    assert_eq!(attempt["executionCwd"], result["executionCwd"]);
    assert!(task.get("dispatchReservation").is_none());
    assert!(owner.call("agents.spawn", params).await.is_err());
    assert_eq!(execution.starts.load(Ordering::SeqCst), 1);
    let launches = execution.launches.lock().unwrap();
    assert!(launches[0].get("taskId").is_none());
    assert!(launches[0].get("workflowStepId").is_none());
    assert!(
        launches[0]["message"]
            .as_str()
            .unwrap()
            .contains("STRUCTURED RESULT CONTRACT")
    );
    drop(launches);
    let proxy = projections
        .read()
        .unwrap()
        .get(result["sessionId"].as_str().unwrap())
        .cloned()
        .unwrap();
    assert_eq!(proxy["hub"], "@paired");
    assert_eq!(proxy["parentSessionId"], "local-owner");
    assert_eq!(proxy["isWakeTarget"], false);
    // A separately admitted task survives an execution acknowledgement loss.
    execution.uncertain.store(true, Ordering::SeqCst);
    let result=owner.call("agents.spawn",json!({"executionTarget":"paired","remoteCwd":"Q:\\remote-only\\repo","cwd":project,"parentSessionId":"local-owner","dispatchOwnerSessionId":"local-owner","provider":"claude","model":"sonnet","message":"another task"})).await;
    assert!(result.is_err());
    let state = history.snapshot().unwrap();
    assert_eq!(
        state
            .tasks
            .iter()
            .flat_map(|task| task["attempts"].as_array().unwrap())
            .count(),
        2
    );
    assert_eq!(execution.starts.load(Ordering::SeqCst), 2);
    let records: Vec<OriginRecord> = serde_json::from_slice(
        &std::fs::read(local_root.path().join("remote-dispatch-origin.json")).unwrap(),
    )
    .unwrap();
    assert!(records.iter().any(|record| {
        record.local_session_id.is_some()
            && record
                .note
                .as_ref()
                .is_some_and(|note| note.contains("acknowledgement"))
    }));
    local.shutdown().unwrap();
    worker.shutdown().unwrap();
}
#[test]
fn pairing_identity_changes_with_credential_and_rejects_url_authority() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("remote-server.json");
    std::fs::write(&file, br#"{"mode":"workers","url":"node","token":"first"}"#).unwrap();
    let first = super::paired::target(root.path()).unwrap().unwrap();
    assert_eq!(first.peer.url, "ws://node:7895/bus");
    std::fs::write(
        &file,
        br#"{"mode":"workers","url":"node","token":"second"}"#,
    )
    .unwrap();
    let second = super::paired::target(root.path()).unwrap().unwrap();
    assert_ne!(first.peer.name, second.peer.name);
    std::fs::write(
        &file,
        br#"{"mode":"workers","url":"http://user:password@node","token":"first"}"#,
    )
    .unwrap();
    assert!(super::paired::target(root.path()).is_err());
}

#[test]
fn paired_result_validation_uses_local_schema_and_rejects_peer_claimed_result_authority() {
    use crate::services::{
        config::Config, task_store::TaskStore, workflow_runtime::WorkflowRuntime,
        workflows::WorkflowStore,
    };
    let root = tempfile::tempdir().unwrap();
    let owner = json!({"sessionId":"manager","isWakeTarget":true,"status":"running"});
    let lookup_owner = owner.clone();
    let tasks = Arc::new(TaskStore::open(root.path().join("dispatch-history.json")).unwrap());
    let workflow = WorkflowRuntime::new(
        Arc::new(WorkflowStore::new(
            root.path().into(),
            Arc::new(Config::open(root.path().join("config.yaml"))),
        )),
        tasks.clone(),
        Arc::new(move |id| (id == "manager").then(|| lookup_owner.clone())),
    );
    let session = format!("paired:{}", uuid::Uuid::new_v4());
    let ids=tasks.accept(json!({"owner":owner,"projectCwd":root.path(),"sessionId":session,"executionCwd":"Q:\\remote","executionTarget":"paired","provider":"claude"}),||Ok(())).unwrap().unwrap();
    let record = OriginRecord {
        dispatch_id: "abcdefghijklmnop".into(),
        peer: "worker".into(),
        owner_session_id: "manager".into(),
        session_id: Some("remote-worker".into()),
        local_session_id: Some(session.clone()),
        cwd: "Q:\\remote".into(),
        provider: "claude".into(),
        model: String::new(),
        label: String::new(),
        opened_at: now(),
        acked_seq: 0,
        delivering_seq: None,
        last_update: None,
        state: "open".into(),
        note: None,
        result_schema: Some(
            json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}),
        ),
    };
    let mut update = Update {
        protocol: 2,
        dispatch_id: record.dispatch_id.clone(),
        kind: Kind::WorkerFinished,
        session_id: "remote-worker".into(),
        seq: 1,
        ts: now(),
        terminal: true,
        entry: json!({"sessionId":"remote-worker","label":"worker","fullReply":"no structured result","escalation":"{\"invented\":true}","result":"{\"ok\":true}"}),
    };
    let result = super::paired::result_entry(&record, &update, &workflow).unwrap();
    assert!(result.get("escalation").is_none());
    assert!(result.get("result").is_none());
    assert!(result["resultError"].is_string());
    assert_eq!(result["sessionId"], session);
    assert_eq!(
        tasks
            .task(ids["taskId"].as_str().unwrap())
            .unwrap()
            .unwrap()["attempts"][0]["resultContract"],
        "invalid"
    );
    update.entry["fullReply"] = "Done\n```wks-result\n{\"ok\":true}\n```".into();
    let result = super::paired::result_entry(&record, &update, &workflow).unwrap();
    assert!(result["result"].is_string());
    assert_eq!(
        tasks
            .task(ids["taskId"].as_str().unwrap())
            .unwrap()
            .unwrap()["attempts"][0]["resultContract"],
        "valid"
    );
}
#[tokio::test]
async fn paired_proxy_projection_ignores_peer_parent_and_host_capability_fields() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let session = format!("paired:{}", uuid::Uuid::new_v4());
    let record = OriginRecord {
        dispatch_id: "abcdefghijklmnop".into(),
        peer: "worker".into(),
        owner_session_id: "local-owner".into(),
        session_id: Some("remote-worker".into()),
        local_session_id: Some(session.clone()),
        cwd: "Q:\\remote".into(),
        provider: "claude".into(),
        model: String::new(),
        label: String::new(),
        opened_at: now(),
        acked_seq: 0,
        delivering_seq: None,
        last_update: None,
        state: "open".into(),
        note: None,
        result_schema: None,
    };
    let proxies = Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));
    super::paired::project_snapshot(&record,&json!({"sessionId":"local-owner","parentSessionId":"other-manager","isWakeTarget":true,"hub":"","_wksConfig":{"fullAccess":true},"settings":{"permissionMode":"bypassPermissions"},"cwd":"Z:\\still-remote","status":"running"}),&proxies,None,&hub.handle()).await.unwrap();
    let row = proxies.read().unwrap().get(&session).cloned().unwrap();
    assert_eq!(row["sessionId"], session);
    assert_eq!(row["parentSessionId"], "local-owner");
    assert_eq!(row["hub"], "@paired");
    assert_eq!(row["isWakeTarget"], false);
    assert!(row.get("settings").is_none());
    assert!(row.get("_wksConfig").is_none());
    assert_eq!(row["cwd"], "Z:\\still-remote");
    drop(root);
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn native_session_queries_include_proxies_without_granting_local_engine_identity() {
    let _engine_guard = crate::backend::ENGINE_TEST_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let options = Options::default();
    let local_rows = options.session_snapshots.clone();
    let session = format!("paired:{}", uuid::Uuid::new_v4());
    options.remote_proxy_snapshots.write().unwrap().insert(session.clone(),json!({"sessionId":session,"parentSessionId":"manager","hub":"@paired","isWakeTarget":false,"status":"running","cwd":"Q:\\remote-only"}));
    let backend = crate::backend::Backend::start(
        claudemon::daemon::ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: root.path().join("state.db"),
        },
        claudemon::daemon::embedded::Options {
            usage_poll_on_boot: Some(false),
        },
        options,
    )
    .await
    .unwrap();
    backend.handle().ready().await.unwrap();
    let client = crate::client::Client::connect(&backend.handle())
        .await
        .unwrap();
    let snapshot = client
        .call("sessions.snapshot", json!({"sessionId":session}))
        .await
        .unwrap();
    assert_eq!(snapshot["hub"], "@paired");
    assert_eq!(snapshot["cwd"], "Q:\\remote-only");
    let rows = client.call("agents.list", Value::Null).await.unwrap();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["sessionId"] == session)
    );
    assert!(!local_rows.read().unwrap().contains_key(&session));
    assert!(
        client
            .call("agents.close", json!({"sessionId":session}))
            .await
            .is_err()
    );
    backend.shutdown().await.unwrap();
}

#[tokio::test]
async fn legacy_paired_receipts_migrate_only_for_the_same_endpoint_and_credential() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let first = "a".repeat(64);
    let other = "b".repeat(64);
    let records:Vec<Value>=[&first,&other].iter().enumerate().map(|(index,hash)|json!({"dispatchId":format!("abcdefghijklmnop{index}"),"peer":format!("@paired:{hash}"),"ownerSessionId":"local-owner","localSessionId":format!("paired:{}",uuid::Uuid::new_v4()),"openedAt":now(),"state":"open","ackedSeq":0})).collect();
    let legacy = root.path().join("remote-dispatches.json");
    std::fs::write(&legacy, serde_json::to_vec(&records).unwrap()).unwrap();
    let origin = Origin::open(
        root.path().into(),
        hub.handle(),
        Arc::new(NoLink),
        Arc::new(Sink::default()),
    )
    .unwrap();
    origin
        .migrate_paired_peer(&format!("paired-{first}"))
        .unwrap();
    let rows = origin.records();
    assert!(rows.iter().any(|row| row.peer == format!("paired-{first}")));
    assert!(
        rows.iter()
            .any(|row| row.peer == format!("@paired:{other}"))
    );
    let original: Vec<Value> = serde_json::from_slice(&std::fs::read(legacy).unwrap()).unwrap();
    assert_eq!(original, records);
    hub.shutdown().unwrap();
}

#[path = "lease_audit_tests.rs"]
mod lease_audit_tests;

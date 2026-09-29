use super::*;
use crate::{Hub, client::Client, protocol::Frame};
use futures_util::future::BoxFuture;
use std::sync::{
    OnceLock,
    atomic::{AtomicI64, AtomicUsize, Ordering},
};

struct Source {
    native: source::NativeSources,
    clock: Arc<AtomicI64>,
    reads: AtomicUsize,
    jobs: Mutex<Vec<JobInfo>>,
}
impl EvidenceSource for Source {
    fn read(&self) -> BoxFuture<'_, Result<Evidence>> {
        Box::pin(async {
            // Match NativeSources: timestamp at read start, before provider I/O.
            // A read already in flight when the synthetic clock crosses the
            // demand cutoff must not be retimestamped as a fresh later sample.
            let sampled_at = self.clock.load(Ordering::SeqCst);
            let mut evidence = self.native.read().await?;
            evidence.now_ms = sampled_at;
            evidence.jobs = Ok(self.jobs.lock().unwrap().clone());
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(evidence)
        })
    }
}
struct Rig {
    hub: Hub,
    client: Client,
    watcher: Arc<Watcher>,
    source: Arc<Source>,
    provider: Option<tokio::task::JoinHandle<()>>,
    clock: Arc<AtomicI64>,
}
impl Rig {
    async fn new(rows: Option<Value>) -> Self {
        let slot: Arc<OnceLock<Arc<Watcher>>> = Arc::new(OnceLock::new());
        let clock = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp_millis()));
        let handler_slot = slot.clone();
        let handler_clock = clock.clone();
        let mut options = Options::default().handler("fleet.quiescence", move |caller, params| {
            let watcher = handler_slot.get().unwrap().clone();
            let now = handler_clock.load(Ordering::SeqCst);
            async move { watcher.answer(&caller, &params, now) }
        });
        options.listen = Some("127.0.0.1:0".parse().unwrap());
        options.token = "watcher-fixture-owner".into();
        let hub = Hub::start(options).unwrap();
        let address = hub.ready().await.unwrap().unwrap();
        let provider = if let Some(rows) = rows {
            let mut connection = hub.handle().connect().await.unwrap();
            assert_eq!(connection.recv().await.unwrap().op, "hello");
            connection
                .send(Frame {
                    methods: vec!["sessions.snapshots".into()],
                    ..Frame::op("register")
                })
                .unwrap();
            assert_eq!(connection.recv().await.unwrap().op, "registered");
            Some(tokio::spawn(async move {
                while let Some(frame) = connection.recv().await {
                    if frame.op == "call" {
                        assert_eq!(frame.method, "sessions.snapshots");
                        connection
                            .send(Frame {
                                id: frame.id,
                                result: Some(rows.clone()),
                                ..Frame::op("result")
                            })
                            .unwrap();
                    }
                }
            }))
        } else {
            None
        };
        let source = Arc::new(Source {
            native: source::NativeSources {
                engine: None,
                hub: hub.handle(),
                jobs: None,
                federation: Default::default(),
                coordinator: None,
                lifecycle: None,
                workflow: None,
                replacements: None,
                terminals: None,
            },
            clock: clock.clone(),
            reads: AtomicUsize::new(0),
            jobs: Mutex::new(vec![]),
        });
        let watcher = Watcher::new(source.clone(), Tunables::default(), PowerConfig::default());
        assert!(slot.set(watcher.clone()).is_ok());
        let client =
            Client::connect_remote(&format!("ws://{address}/bus"), "watcher-fixture-owner")
                .await
                .unwrap();
        Self {
            hub,
            client,
            watcher,
            source,
            provider,
            clock,
        }
    }
    fn run(&self) -> tokio::task::JoinHandle<Result<()>> {
        let watcher = self.watcher.clone();
        let hub = self.hub.handle();
        let clock = self.clock.clone();
        tokio::spawn(async move {
            watcher
                .run_fleet_with_clock(hub, Duration::from_millis(5), move || {
                    clock.load(Ordering::SeqCst)
                })
                .await
        })
    }
    async fn answer(&self) -> Value {
        self.client
            .call("fleet.quiescence", json!({}))
            .await
            .unwrap()
    }
    async fn next_sample(&self) {
        let before = self.source.reads.load(Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.source.reads.load(Ordering::SeqCst) <= before {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn close(self, task: tokio::task::JoinHandle<Result<()>>) {
        self.watcher.close();
        task.await.unwrap().unwrap();
        self.client.close();
        if let Some(provider) = self.provider {
            provider.abort();
            let _ = provider.await;
        }
        tokio::task::spawn_blocking(move || self.hub.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}
fn has(value: &Value, kind: &str) -> bool {
    value["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| blocker["kind"] == kind)
}

#[tokio::test]
async fn watcher_wire_answer_is_cold_stale_and_sampler_runs_only_during_demand() {
    let rig = Rig::new(Some(json!([]))).await;
    let task = rig.run();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(rig.source.reads.load(Ordering::SeqCst), 0);
    let cold = rig.answer().await;
    assert_eq!(cold["quiescent"], false);
    assert!(cold["since"].is_null());
    assert!(cold["dwellSeconds"].as_i64().unwrap() > 0);
    assert!(has(&cold, "stale-sample"));
    rig.next_sample().await;
    let asked_at = rig.clock.load(Ordering::SeqCst);
    assert!(rig.watcher.sampling(asked_at + 5 * 60_000));
    assert!(rig.watcher.sampling(asked_at + 15 * 60_000));
    assert!(!rig.watcher.sampling(asked_at + 15 * 60_000 + 1));
    rig.clock
        .store(asked_at + 15 * 60_000 + 1, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let stopped = rig.source.reads.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(rig.source.reads.load(Ordering::SeqCst), stopped);
    assert!(has(&rig.answer().await, "stale-sample"));
    rig.next_sample().await;
    rig.close(task).await;
}

#[tokio::test]
async fn watcher_reads_registered_provider_and_refuses_unknown_or_unavailable_fleet() {
    for (rows, expected) in [
        (
            Some(json!([{"session_id":"worker","mode":"responding"}])),
            "session-working",
        ),
        (
            Some(json!([{"session_id":"worker","mode":"unknown"}])),
            "session-unknown",
        ),
        (Some(json!({"not":"a session array"})), "fleet-unreadable"),
        (None, "fleet-unreadable"),
    ] {
        let rig = Rig::new(rows).await;
        let task = rig.run();
        rig.answer().await;
        rig.next_sample().await;
        let answer = rig.answer().await;
        assert_eq!(answer["quiescent"], false);
        assert!(has(&answer, expected), "{answer}");
        if expected == "session-unknown" {
            let detail = answer["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["kind"] == expected)
                .unwrap()["detail"]
                .as_str()
                .unwrap();
            assert!(detail.contains("TERMINAL") && detail.contains("SPAWNING"));
        }
        rig.close(task).await;
    }
}

#[tokio::test]
async fn watcher_excludes_infrastructure_and_asks_but_counts_later_same_connection_work() {
    let rig = Rig::new(Some(json!([{"session_id":"idle","mode":"input"}]))).await;
    let internal = rig.hub.handle().connect_service().await.unwrap();
    let initial = rig.source.read().await.unwrap();
    assert!(
        evaluate(&initial, Tunables::default(), &BTreeMap::new())
            .iter()
            .any(|b| b.kind == "client-active")
    );
    assert!(initial.clients.as_ref().unwrap().iter().any(|c| c.internal));
    assert!(initial.clients.as_ref().unwrap().iter().any(|c| c.provider));
    let task = rig.run();
    rig.answer().await;
    rig.next_sample().await;
    assert!(!has(&rig.answer().await, "client-active"));
    let asked = rig.watcher.activity.lock().unwrap().asked.clone();
    rig.client
        .call("sessions.snapshots", json!({}))
        .await
        .unwrap();
    rig.next_sample().await;
    let reading =
        serde_json::to_value(rig.watcher.fleet.latest(rig.clock.load(Ordering::SeqCst))).unwrap();
    assert!(has(&reading, "client-active"), "{reading}");
    let clients = rig.hub.handle().quiescence_clients().await.unwrap();
    assert!(clients.iter().any(|c| {
        asked
            .get(&c.connection_id)
            .is_some_and(|seq| c.activity_seq > *seq)
    }));
    assert!(
        rig.client
            .call("fleet.quiescence", json!({"sessions":[]}))
            .await
            .is_err()
    );
    drop(internal);
    rig.client.close();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !rig.watcher.activity.lock().unwrap().asked.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    rig.close(task).await;
}

#[tokio::test]
async fn watcher_job_evidence_protects_spawn_schedule_but_exempts_advisory_shell_poller() {
    let rig = Rig::new(Some(json!([]))).await;
    let due = rig.clock.load(Ordering::SeqCst) + 120_000;
    *rig.source.jobs.lock().unwrap() = vec![
        JobInfo {
            id: "spawn".into(),
            name: "review".into(),
            action_kind: "spawn".into(),
            next_run_ms: Some(due),
            running: false,
        },
        JobInfo {
            id: "poll".into(),
            name: "poller".into(),
            action_kind: "shell".into(),
            next_run_ms: Some(due),
            running: false,
        },
    ];
    let task = rig.run();
    rig.answer().await;
    rig.next_sample().await;
    let answer = rig.answer().await;
    assert!(has(&answer, "job-due-soon"));
    assert!(
        answer["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["id"] != "poll")
    );
    rig.close(task).await;
}

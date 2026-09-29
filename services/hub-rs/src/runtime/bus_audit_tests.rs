//! Direct retained bus invariants, with actor barriers rather than timeout-only
//! negatives. No provider process or external service is used.
use super::*;
use crate::auth::{Record, Scope};

struct Mailbox {
    reliable: mpsc::Receiver<Frame>,
    events: mpsc::Receiver<Frame>,
    closed: watch::Receiver<bool>,
}
fn core() -> Core {
    Core {
        federation: Default::default(),
        peers: HashMap::new(),
        providers: BTreeMap::new(),
        missing_reported: BTreeSet::new(),
        pending: HashMap::new(),
        demand_counts: BTreeMap::new(),
        seq: 0,
        event_seq: 0,
        options: Options::default(),
        jobs: tokio::task::JoinSet::new(),
        last_revalidation: Instant::now(),
    }
}
fn peer(core: &mut Core, id: u64, identity: Identity, capacity: usize) -> Mailbox {
    let (reliable, rx) = mpsc::channel(capacity);
    let (events, event_rx) = mpsc::channel(capacity);
    let (closed, closed_rx) = watch::channel(false);
    core.peers.insert(
        id,
        Peer {
            facade_access: None,
            delegated: false,
            wants_caller_context: false,
            close_code: Arc::new(AtomicU16::new(0)),
            network: Arc::new(AtomicBool::new(false)),
            close_done: watch::channel(false).1,
            local_facade: false,
            internal: false,
            activity_seq: 1,
            last_active_ms: 1,
            last_interaction_ms: 1,
            reports_interaction: false,
            identity,
            credential: None,
            reliable,
            events,
            closed,
            topics: vec![],
            demand: vec![],
            held_demand: BTreeSet::new(),
            desynced: Arc::new(Mutex::new(BTreeSet::new())),
        },
    );
    Mailbox {
        reliable: rx,
        events: event_rx,
        closed: closed_rx,
    }
}
fn scoped(scope: Scope, provides: Vec<String>) -> Identity {
    Identity {
        kind: Kind::Scoped(Record {
            scope: scope.name().into(),
            provides: Some(provides),
            ..Default::default()
        }),
        token_id: "verified-fixture".into(),
        federated: false,
    }
}
fn topics(core: &mut Core, id: u64, op: &str, topics: &[&str]) {
    core.frame(
        id,
        Frame {
            topics: topics.iter().map(|s| s.to_string()).collect(),
            ..Frame::op(op)
        },
    );
}
fn take(mail: &mut Mailbox) -> Frame {
    mail.reliable.try_recv().expect("expected reliable frame")
}
fn empty(mail: &mut Mailbox) {
    assert!(matches!(
        mail.reliable.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}
fn register(core: &mut Core, id: u64, method: &str) {
    core.frame(
        id,
        Frame {
            methods: vec![method.into()],
            ..Frame::op("register")
        },
    );
}
fn call(core: &mut Core, id: u64, correlation: &str, method: &str, params: Value) {
    core.frame(
        id,
        Frame {
            id: correlation.into(),
            method: method.into(),
            params: Some(params),
            ..Frame::op("call")
        },
    );
}

#[tokio::test]
async fn demand_replay_clear_entitlement_and_distinct_subscription_counts() {
    let mut core = core();
    let mut producer = peer(
        &mut core,
        1,
        scoped(Scope::Provider, vec!["sessions.conversation".into()]),
        32,
    );
    let mut a = peer(&mut core, 2, scoped(Scope::View, vec![]), 32);
    let mut b = peer(&mut core, 3, Identity::host("fixture"), 32);
    let mut snoop = peer(&mut core, 4, scoped(Scope::View, vec![]), 32);
    topics(
        &mut core,
        2,
        "subscribe",
        &[
            "*",
            "agent.*",
            "agent.conversation.*",
            "agent.conversation.a",
        ],
    );
    assert_eq!(take(&mut a).op, "subscribed");
    assert_eq!(core.demand_counts.len(), 1);
    assert_eq!(core.count_demand("agent.conversation.a"), 1);
    topics(&mut core, 1, "demand", &["agent.conversation."]);
    let replay = take(&mut producer);
    assert!(replay.demand);
    assert_eq!(replay.topic, "agent.conversation.a");
    topics(&mut core, 4, "demand", &["agent.conversation."]);
    empty(&mut snoop);
    topics(&mut core, 2, "subscribe", &["agent.conversation.a"]);
    take(&mut a);
    empty(&mut producer);
    topics(&mut core, 3, "subscribe", &["agent.conversation.a"]);
    take(&mut b);
    empty(&mut producer);
    assert_eq!(core.count_demand("agent.conversation.a"), 2);
    topics(&mut core, 2, "unsubscribe", &["agent.conversation.a"]);
    take(&mut a);
    empty(&mut producer);
    core.disconnect(3);
    let released = take(&mut producer);
    assert!(!released.demand);
    assert_eq!(released.topic, "agent.conversation.a");
    assert!(*b.closed.borrow());
    topics(&mut core, 1, "demand", &[]);
    topics(&mut core, 2, "subscribe", &["agent.conversation.b"]);
    take(&mut a);
    empty(&mut producer);
    topics(&mut core, 2, "subscribe", &["pty.bytes.secret"]);
    take(&mut a);
    assert_eq!(core.count_demand("pty.bytes.secret"), 0);
    topics(&mut core, 1, "demand", &[""]);
    empty(&mut producer);
    topics(&mut core, 1, "demand", &["agent.conversation."]);
    assert_eq!(take(&mut producer).topic, "agent.conversation.b");
    core.disconnect(2);
    assert!(!take(&mut producer).demand);
    empty(&mut snoop);
}

#[tokio::test]
async fn provider_forward_order_and_bounded_slow_consumers_do_not_stall_other_frames() {
    let mut core = core();
    let mut provider = peer(&mut core, 1, Identity::host("fixture"), 64);
    let mut caller = peer(&mut core, 2, Identity::host("fixture"), 64);
    register(&mut core, 1, "fixture.echo");
    take(&mut provider);
    for n in 0..40 {
        call(
            &mut core,
            2,
            &format!("c{n}"),
            "fixture.echo",
            json!({"seq":n}),
        );
    }
    for n in 0..40 {
        let f = take(&mut provider);
        assert_eq!(f.params.unwrap()["seq"], n);
        core.frame(
            1,
            Frame {
                id: f.id,
                result: Some(json!(n)),
                ..Frame::op("result")
            },
        );
        assert_eq!(take(&mut caller).id, format!("c{n}"));
    }
    // Saturation closes a wedged reliable consumer rather than buffering or
    // blocking the actor. Other peers must retain control-frame progress.
    let mut stuck = peer(&mut core, 3, Identity::host("fixture"), 1);
    register(&mut core, 3, "fixture.stuck");
    take(&mut stuck);
    call(&mut core, 2, "queued", "fixture.stuck", Value::Null);
    call(&mut core, 2, "overflow", "fixture.stuck", Value::Null);
    assert!(*stuck.closed.borrow());
    topics(&mut core, 2, "subscribe", &["agent.*"]);
    assert_eq!(take(&mut caller).op, "subscribed");
    core.sweep();
    assert!(!core.providers.contains_key("fixture.stuck"));
    let errors = [take(&mut caller), take(&mut caller)];
    assert!(errors.iter().all(|f| f.op == "error"));
    assert_eq!(
        errors.into_iter().map(|f| f.id).collect::<BTreeSet<_>>(),
        BTreeSet::from(["queued".into(), "overflow".into()])
    );
}

#[tokio::test]
async fn slow_caller_cannot_stall_sibling_replies_and_provider_errors_keep_correlation() {
    let mut core = core();
    let mut provider = peer(&mut core, 1, Identity::host("fixture"), 8);
    let slow = peer(&mut core, 2, Identity::host("fixture"), 1);
    let mut fast = peer(&mut core, 3, Identity::host("fixture"), 8);
    register(&mut core, 1, "fixture.echo");
    take(&mut provider);
    call(&mut core, 2, "slow1", "fixture.echo", Value::Null);
    let one = take(&mut provider);
    call(&mut core, 2, "slow2", "fixture.echo", Value::Null);
    let two = take(&mut provider);
    call(&mut core, 3, "fast", "fixture.echo", Value::Null);
    let three = take(&mut provider);
    for f in [one, two] {
        core.frame(
            1,
            Frame {
                id: f.id,
                result: Some(json!("slow")),
                ..Frame::op("result")
            },
        );
    }
    assert!(*slow.closed.borrow());
    core.frame(
        1,
        Frame {
            id: three.id,
            error: "exact provider error".into(),
            ..Frame::op("error")
        },
    );
    let result = take(&mut fast);
    assert_eq!(result.id, "fast");
    assert_eq!(result.error, "exact provider error");
}

#[tokio::test]
async fn missing_provider_child() {
    if std::env::var_os("WKS_TEST_MISSING_PROVIDER_DIAGNOSTIC").is_none() {
        return;
    }
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let client = crate::client::Client::connect(&hub.handle()).await.unwrap();
    for _ in 0..3 {
        assert!(
            client
                .call("fixture.absent", json!({"secret":"DO_NOT_LOG_THIS"}))
                .await
                .is_err()
        );
    }
    assert!(client.call("fixture.other", Value::Null).await.is_err());
    let mut provider = hub.handle().connect().await.unwrap();
    provider.recv().await.unwrap();
    provider
        .send(Frame {
            methods: vec!["fixture.absent".into()],
            ..Frame::op("register")
        })
        .unwrap();
    provider.recv().await.unwrap();
    drop(provider);
    hub.handle().health().await.unwrap();
    assert!(client.call("fixture.absent", Value::Null).await.is_err());
    hub.shutdown().unwrap();
}
#[test]
fn missing_provider_diagnostics_are_once_per_outage_and_never_log_parameters() {
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runtime::bus_audit_tests::missing_provider_child",
            "--nocapture",
        ])
        .env("WKS_TEST_MISSING_PROVIDER_DIAGNOSTIC", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(
        stderr.matches("NO PROVIDER for \"fixture.absent\"").count(),
        2,
        "{stderr}"
    );
    assert_eq!(
        stderr.matches("NO PROVIDER for \"fixture.other\"").count(),
        1,
        "{stderr}"
    );
    assert!(!stderr.contains("DO_NOT_LOG_THIS"));
}

#[tokio::test]
async fn passive_request_shapes_preserve_foreground_activity() {
    use crate::services::quiescence::passive_request;
    let cases = [
        ("desktop.providerReadiness", None, false),
        ("desktop.providerReadiness", Some(Value::Null), true),
        ("desktop.providerReadiness", Some(json!({})), true),
        (
            "desktop.providerReadiness",
            Some(json!({"CHECK": true})),
            false,
        ),
        (
            "desktop.providerReadiness",
            Some(json!({"CHECK": false})),
            true,
        ),
        (
            "desktop.providerReadiness",
            Some(json!({"check": "false"})),
            false,
        ),
        (
            "desktop.providerReadiness",
            Some(json!({"check": false, "CHECK": true})),
            false,
        ),
        (
            "desktop.managerReplacement",
            Some(json!({"REQUEST":{"ACTION":"list"}})),
            true,
        ),
        (
            "fleetWorkflows.request",
            Some(json!({"op":"list", "request": 3})),
            false,
        ),
        (
            "desktop.fleetWorkflowRequest",
            Some(json!({"op":3,"request":{"op":"list"}})),
            false,
        ),
        (
            "hub:remote/fleetWorkflows.request",
            Some(json!({"OP":"next"})),
            true,
        ),
        ("sessions.snapshots", None, true),
    ];
    let mut core = core();
    let mut mailbox = peer(&mut core, 1, Identity::host("fixture"), 64);
    core.peers.get_mut(&1).unwrap().reports_interaction = true;
    for (method, params, passive) in cases {
        assert_eq!(
            passive_request(method, params.as_ref()),
            passive,
            "{method} {params:?}"
        );
        core.peers.get_mut(&1).unwrap().last_interaction_ms = 1;
        core.frame(
            1,
            Frame {
                method: method.into(),
                params,
                id: "activity-case".into(),
                ..Frame::op("call")
            },
        );
        assert_eq!(core.peers[&1].last_interaction_ms == 1, passive, "{method}");
        let _ = mailbox.reliable.try_recv();
    }
    assert!(mailbox.events.try_recv().is_err());
}

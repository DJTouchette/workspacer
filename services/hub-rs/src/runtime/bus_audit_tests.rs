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
    assert!(!passive_request(
        "desktop.providerReadiness",
        Some(&json!({"checK":true}))
    ));
    assert!(passive_request(
        "desktop.managerReplacement",
        Some(&json!({"requeſt":{"action":"list"}}))
    ));
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
    core.peers.get_mut(&1).unwrap().last_active_ms = 1;
    topics(&mut core, 1, "subscribe", &["agent.snapshot"]);
    take(&mut mailbox);
    assert_eq!(
        core.peers[&1].last_active_ms, 1,
        "subscriptions are not foreground use"
    );
    call(&mut core, 1, "clock", "layout.get", Value::Null);
    take(&mut mailbox);
    assert!(
        core.peers[&1].last_active_ms > 1,
        "legacy clients count even passive calls"
    );
    core.peers.get_mut(&1).unwrap().last_active_ms = 1;
    core.frame(
        1,
        Frame {
            event: Some(Event::new("fixture.activity", "fixture", Value::Null)),
            ..Frame::op("publish")
        },
    );
    assert!(core.peers[&1].last_active_ms > 1);
    assert!(mailbox.events.try_recv().is_err());
}

#[test]
fn every_registered_topic_has_the_reference_identity_policy() {
    let vocabulary: Value =
        serde_json::from_str(include_str!("../../assets/hub-vocabulary.json")).unwrap();
    let rows = vocabulary["topics"].as_array().unwrap();
    assert!(rows.len() >= 36);
    let identities = [
        ("host", Identity::host("fixture")),
        ("view", scoped(Scope::View, vec![])),
        ("triage", scoped(Scope::Triage, vec![])),
        ("operator", scoped(Scope::Operator, vec![])),
        ("provider", scoped(Scope::Provider, vec!["*".into()])),
        (
            "plugin",
            Identity {
                kind: Kind::Plugin {
                    id: "fixtureplugin".into(),
                    provides: vec!["fixtureplugin.*".into()],
                },
                token_id: "plugin-fingerprint".into(),
                federated: false,
            },
        ),
    ];
    for row in rows {
        let pattern = row["Pattern"].as_str().unwrap();
        let topic = pattern
            .strip_suffix('*')
            .map(|prefix| format!("{prefix}fixture"))
            .unwrap_or_else(|| pattern.into());
        let disposition = row["Disposition"].as_str().unwrap();
        let method = row["Method"].as_str().unwrap();
        let publisher = row["Publisher"].as_str().unwrap();
        let dispatch = topic.starts_with("agent.dispatch.");
        for (name, identity) in &identities {
            let private_dispatch = dispatch && topic != "agent.dispatch.update";
            let consume = if private_dispatch {
                *name == "host"
            } else {
                match *name {
                    "host" | "operator" => true,
                    "provider" => false,
                    "plugin" => disposition != "host-only",
                    _ => {
                        disposition == "open-by-decision"
                            || (disposition == "guarded-by-capability"
                                && vocabulary["scopes"][*name].as_array().unwrap().iter().any(
                                    |grant| {
                                        let grant = grant.as_str().unwrap();
                                        grant == method
                                            || grant == "*"
                                            || grant
                                                .strip_suffix('*')
                                                .is_some_and(|prefix| method.starts_with(prefix))
                                    },
                                ))
                    }
                }
            };
            let publish = if dispatch {
                topic == "agent.dispatch.update" && matches!(*name, "host" | "provider")
            } else {
                matches!(*name, "host" | "operator")
                    || (*name == "provider" && !publisher.is_empty())
            };
            assert_eq!(
                identity.may_consume(&topic),
                consume,
                "consume {name}: {topic}"
            );
            assert_eq!(
                identity.may_publish(&topic),
                publish,
                "publish {name}: {topic}"
            );
        }
    }
    for (name, identity) in identities {
        let allowed = matches!(name, "host" | "operator" | "plugin");
        assert_eq!(
            identity.may_consume("unclassified.fixture"),
            allowed,
            "{name}"
        );
        assert_eq!(
            identity.may_publish("unclassified.fixture"),
            allowed,
            "{name}"
        );
    }
}

#[tokio::test]
async fn closed_identity_is_inert_before_physical_peer_eviction() {
    let mut core = core();
    let mut closed = peer(&mut core, 1, Identity::host("fixture"), 8);
    let mut provider = peer(&mut core, 2, Identity::host("fixture"), 8);
    topics(&mut core, 1, "subscribe", &["*"]);
    take(&mut closed);
    register(&mut core, 2, "fixture.echo");
    take(&mut provider);
    let sequence = core.peers[&1].activity_seq;
    core.peers[&1].closed.send_replace(true);
    core.publish(Event::new(
        "agent.snapshot",
        "fixture",
        json!({"private":true}),
    ));
    call(&mut core, 1, "revoked", "fixture.echo", Value::Null);
    assert!(closed.events.try_recv().is_err());
    empty(&mut provider);
    assert!(core.pending.is_empty());
    assert_eq!(core.peers[&1].activity_seq, sequence);
}

#[tokio::test]
async fn registered_private_desktop_names_remain_unreachable_even_to_owner() {
    async fn receive(connection: &mut Connection) -> Frame {
        tokio::time::timeout(Duration::from_secs(3), connection.recv())
            .await
            .unwrap()
            .unwrap()
    }
    let hub = Hub::start(Options::default()).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    let mut owner = handle.connect().await.unwrap();
    let mut provider = handle.connect().await.unwrap();
    receive(&mut owner).await;
    receive(&mut provider).await;
    provider
        .send(Frame {
            methods: vec![
                "desktop.internal.acceptSpawn".into(),
                "desktop.worktreeInfo".into(),
                "plugins.fixtureExtension".into(),
            ],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(receive(&mut provider).await.methods.len(), 3);
    owner
        .send(Frame {
            id: "private".into(),
            method: "desktop.internal.acceptSpawn".into(),
            params: Some(json!({"authenticatedHost":true})),
            ..Frame::op("call")
        })
        .unwrap();
    let refusal = receive(&mut owner).await;
    assert_eq!(refusal.op, "error");
    assert_eq!(refusal.id, "private");
    assert!(refusal.error.contains("desktop services"));
    for method in ["desktop.worktreeInfo", "plugins.fixtureExtension"] {
        owner
            .send(Frame {
                id: method.into(),
                method: method.into(),
                ..Frame::op("call")
            })
            .unwrap();
        let call = receive(&mut provider).await;
        assert_eq!(
            call.method, method,
            "private call must never have reached provider"
        );
        provider
            .send(Frame {
                id: call.id,
                result: Some(json!({"ok":true})),
                ..Frame::op("result")
            })
            .unwrap();
        let result = receive(&mut owner).await;
        assert_eq!(result.id, method);
        assert_eq!(result.result, Some(json!({"ok":true})));
    }
    handle
        .register_plugin(
            "fixture-plugin-token".into(),
            "fixtureplugin".into(),
            vec!["fixtureplugin.echo".into()],
        )
        .await
        .unwrap();
    let mut plugin = handle
        .connect_authenticated("fixture-plugin-token".into(), false)
        .await
        .unwrap();
    receive(&mut plugin).await;
    plugin
        .send(Frame {
            methods: vec!["fixtureplugin.echo".into()],
            ..Frame::op("register")
        })
        .unwrap();
    assert_eq!(
        receive(&mut plugin).await.methods,
        vec!["fixtureplugin.echo"]
    );
    owner
        .send(Frame {
            id: "extension".into(),
            method: "fixtureplugin.echo".into(),
            ..Frame::op("call")
        })
        .unwrap();
    let call = receive(&mut plugin).await;
    assert_eq!(call.method, "fixtureplugin.echo");
    plugin
        .send(Frame {
            id: call.id,
            result: Some(json!("plugin result")),
            ..Frame::op("result")
        })
        .unwrap();
    assert_eq!(
        receive(&mut owner).await.result,
        Some(json!("plugin result"))
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn canonical_key_refusal_precedes_both_local_and_qualified_execution() {
    let mut core = core();
    let mut caller = peer(&mut core, 1, scoped(Scope::Operator, vec![]), 16);
    for method in ["agents.spawn", "hub:worker/agents.spawn"] {
        for key in ["YoloGranted", "Model", "tasKId"] {
            call(&mut core, 1, "alias", method, json!({key:"value"}));
            let refusal = take(&mut caller);
            assert_eq!(refusal.id, "alias");
            assert!(
                refusal.error.contains("non-canonical param"),
                "{method}: {}",
                refusal.error
            );
            assert!(core.pending.is_empty());
        }
        // A canonical request reaches the distinct execution-service guard;
        // merely refusing every qualified request cannot satisfy this test.
        call(&mut core, 1, "canonical", method, json!({"model":"opus"}));
        assert!(
            take(&mut caller)
                .error
                .contains("requires a configured execution service")
        );
    }
}

#[tokio::test]
async fn live_inventory_uses_real_connection_ids_and_host_owned_infrastructure_flags() {
    async fn receive(connection: &mut Connection) -> Frame {
        tokio::time::timeout(Duration::from_secs(3), connection.recv())
            .await
            .unwrap()
            .unwrap()
    }
    let mut options = Options::default().handler("fixture.identity", |caller, _| async move {
        Ok(json!({"connectionId":caller.connection_id,"sequence":caller.activity_seq}))
    });
    options.token = "PRIVATE_OWNER_TOKEN".into();
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    let mut user = handle.connect().await.unwrap();
    let mut provider = handle.connect().await.unwrap();
    let internal = handle.connect_service().await.unwrap();
    receive(&mut user).await;
    receive(&mut provider).await;
    provider
        .send(Frame {
            methods: vec!["fixture.provider".into()],
            ..Frame::op("register")
        })
        .unwrap();
    receive(&mut provider).await;
    handle
        .register_plugin(
            "PRIVATE_PLUGIN_TOKEN".into(),
            "fixtureplugin".into(),
            vec![],
        )
        .await
        .unwrap();
    let plugin = handle
        .connect_authenticated("PRIVATE_PLUGIN_TOKEN".into(), false)
        .await
        .unwrap();
    user.send(serde_json::from_value(json!({"op":"activity","internal":true})).unwrap())
        .unwrap();
    user.send(Frame {
        id: "identity".into(),
        method: "fixture.identity".into(),
        params: Some(json!({"connectionId":999999,"internal":true})),
        ..Frame::op("call")
    })
    .unwrap();
    let identity = receive(&mut user).await.result.unwrap();
    assert_eq!(identity["connectionId"], user.id);
    assert!(identity["sequence"].as_u64().unwrap() > 1);
    let rows = handle.quiescence_clients().await.unwrap();
    assert!(
        rows.windows(2)
            .all(|pair| pair[0].connection_id < pair[1].connection_id)
    );
    let human = rows
        .iter()
        .find(|row| row.connection_id == user.id)
        .unwrap();
    assert!(!human.internal && !human.plugin && !human.provider);
    assert!(human.idle_active_ms > 0 && !human.label.is_empty());
    assert!(
        rows.iter()
            .find(|row| row.connection_id == provider.id)
            .unwrap()
            .provider
    );
    assert!(
        rows.iter()
            .find(|row| row.connection_id == plugin.id)
            .unwrap()
            .plugin
    );
    assert!(
        rows.iter()
            .find(|row| row.connection_id == internal.id)
            .unwrap()
            .internal
    );
    assert!(rows.iter().all(|row| !row.label.contains("PRIVATE_")));
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn hello_permission_passthrough_tracks_call_authority_not_provider_registration_grants() {
    let directory = tempfile::tempdir().unwrap();
    let tokens = directory.path().join("tokens.json");
    let operator = crate::auth::mint(&tokens, Scope::Operator, "operator").unwrap();
    let triage = crate::auth::mint(&tokens, Scope::Triage, "triage").unwrap();
    let provider = crate::auth::mint(&tokens, Scope::Provider, "provider").unwrap();
    let mut options = Options::default();
    options.token = "fixture-host".into();
    options.scoped_tokens = Some(tokens);
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let handle = hub.handle();
    handle
        .register_plugin("fixture-plugin".into(), "fixtureplugin".into(), vec![])
        .await
        .unwrap();
    for (token, federated, allowed) in [
        ("fixture-host".to_owned(), false, true),
        (operator.token, false, true),
        (triage.token, false, false),
        (provider.token, false, false),
        ("fixture-plugin".into(), false, true),
        ("fixture-host".into(), true, true),
    ] {
        let mut connection = handle
            .connect_authenticated(token.clone(), federated)
            .await
            .unwrap();
        let hello = tokio::time::timeout(Duration::from_secs(3), connection.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hello.op, "hello");
        assert_eq!(
            hello.spawn_full_access, allowed,
            "{} federated={federated}",
            hello.scope
        );
    }
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn disk_revalidation_closes_revoked_downgraded_and_facade_or_provider_narrowed_peers() {
    for change in [
        "view-revoked",
        "operator-revoked",
        "tier-downgraded",
        "facade-revoked",
        "provider-narrowed",
        "corrupt-store",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tokens.json");
        let scope = if change == "view-revoked" {
            Scope::View
        } else if change == "provider-narrowed" {
            Scope::Provider
        } else {
            Scope::Operator
        };
        let mut record = crate::auth::mint(&path, scope, "fixture").unwrap();
        record.facade_authority = change == "facade-revoked";
        crate::auth::save(&path, &[record.clone()]).unwrap();
        let mut core = core();
        core.options.scoped_tokens = Some(path.clone());
        let mut mailbox = peer(
            &mut core,
            1,
            Identity {
                kind: Kind::Scoped(record.clone()),
                token_id: crate::auth::fingerprint(&record.token),
                federated: false,
            },
            8,
        );
        core.peers.get_mut(&1).unwrap().credential = Some(record.token.clone());
        if scope == Scope::Provider {
            register(&mut core, 1, "fixture.provider");
            take(&mut mailbox);
        } else {
            topics(&mut core, 1, "subscribe", &["agent.snapshot"]);
            take(&mut mailbox);
            core.publish(Event::new(
                "agent.snapshot",
                "fixture",
                json!({"before":true}),
            ));
            assert_eq!(
                mailbox.events.try_recv().unwrap().event.unwrap().data,
                Some(json!({"before":true}))
            );
        }
        match change {
            "view-revoked" | "operator-revoked" => crate::auth::save(&path, &[]).unwrap(),
            "tier-downgraded" => {
                record.scope = "view".into();
                crate::auth::save(&path, &[record]).unwrap();
            }
            "facade-revoked" => {
                record.facade_authority = false;
                crate::auth::save(&path, &[record]).unwrap();
            }
            "provider-narrowed" => {
                record.provides = Some(vec![]);
                crate::auth::save(&path, &[record]).unwrap();
            }
            "corrupt-store" => std::fs::write(&path, b"{broken").unwrap(),
            _ => unreachable!(),
        }
        core.last_revalidation = Instant::now() - Duration::from_secs(6);
        core.sweep();
        assert!(*mailbox.closed.borrow(), "{change}");
        assert!(!core.peers.contains_key(&1), "{change}");
        assert!(!core.providers.values().any(|id| *id == 1), "{change}");
        core.publish(Event::new(
            "agent.snapshot",
            "fixture",
            json!({"after":true}),
        ));
        assert!(mailbox.events.try_recv().is_err(), "{change}");
        call(&mut core, 1, "revoked", "agents.list", Value::Null);
        assert!(core.pending.is_empty(), "{change}");
    }
}

#[tokio::test]
async fn editing_legacy_profile_metadata_neither_disconnects_nor_narrows_spawn_selection() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tokens.json");
    let mut record = crate::auth::mint(&path, Scope::Operator, "manager").unwrap();
    record
        .metadata
        .insert("profilesAllowed".into(), json!(["work"]));
    record.metadata.insert("yoloAllowed".into(), true.into());
    crate::auth::save(&path, &[record.clone()]).unwrap();
    let mut core = core();
    core.options.control_plane_only = true;
    core.options.scoped_tokens = Some(path.clone());
    let mut caller = peer(
        &mut core,
        1,
        Identity {
            kind: Kind::Scoped(record.clone()),
            token_id: crate::auth::fingerprint(&record.token),
            federated: false,
        },
        8,
    );
    core.peers.get_mut(&1).unwrap().credential = Some(record.token.clone());
    let mut provider = peer(&mut core, 2, Identity::host("fixture"), 8);
    register(&mut core, 2, "agents.spawn");
    take(&mut provider);
    for changed in [false, true] {
        if changed {
            record.metadata.insert("profilesAllowed".into(), json!([]));
            record.metadata.insert("yoloAllowed".into(), false.into());
            record
                .metadata
                .insert("role".into(), json!("legacy annotation"));
            crate::auth::save(&path, &[record.clone()]).unwrap();
            core.last_revalidation = Instant::now() - Duration::from_secs(6);
            core.sweep();
            assert!(!*caller.closed.borrow());
        }
        call(
            &mut core,
            1,
            "spawn",
            "agents.spawn",
            json!({"profileId":"work"}),
        );
        let forwarded = take(&mut provider);
        assert_eq!(forwarded.params.unwrap()["profileId"], "work");
        core.frame(
            2,
            Frame {
                id: forwarded.id,
                result: Some(json!({"ok":true})),
                ..Frame::op("result")
            },
        );
        assert_eq!(take(&mut caller).result, Some(json!({"ok":true})));
    }
}

#[path = "broker_tests.rs"]
mod broker_tests;

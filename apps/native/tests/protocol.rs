use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use wks_native::{
    bus::{Client, Config, Event},
    controller::{Action, Command, Controller, View},
    harness::event,
};

const DEADLINE: Duration = Duration::from_secs(5);

struct Frame {
    value: Value,
    send: mpsc::Sender<Message>,
}
impl Frame {
    async fn result(&self, value: Value) {
        self.send
            .send(Message::Text(
                json!({"op":"result", "id":self.value["id"], "result":value}).to_string(),
            ))
            .await
            .unwrap();
    }
    async fn event(&self, topic: &str, value: Value) {
        self.send.send(event(topic, value)).await.unwrap();
    }
}

struct Hub {
    config: Config,
    frames: mpsc::Receiver<Frame>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Hub {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Hub {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config =
            Config::new(format!("ws://{}/bus", listener.local_addr().unwrap()), None).unwrap();
        let (send, frames) = mpsc::channel(128);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let Ok(mut socket) = accept_async(stream).await else {
                    continue;
                };
                socket
                    .send(Message::Text(json!({"op":"hello"}).to_string()))
                    .await
                    .unwrap();
                let (tx, mut rx) = mpsc::channel(128);
                loop {
                    tokio::select! {
                        message = socket.next() => match message {
                            Some(Ok(Message::Text(text))) => {
                                let value = serde_json::from_str(&text).unwrap();
                                if send.send(Frame {value, send:tx.clone()}).await.is_err() { return; }
                            }
                            Some(Ok(Message::Ping(p))) => { let _ = socket.send(Message::Pong(p)).await; }
                            _ => break,
                        },
                        Some(message) = rx.recv() => {
                            let close = matches!(message, Message::Close(_));
                            if socket.send(message).await.is_err() || close { break; }
                        }
                    }
                }
            }
        });
        Self {
            config,
            frames,
            task,
        }
    }
    async fn frame(&mut self, op: &str, method: Option<&str>) -> Frame {
        timeout(DEADLINE, async {
            loop {
                let frame = self.frames.recv().await.unwrap();
                if !frame.send.is_closed()
                    && frame.value["op"] == op
                    && method.is_none_or(|m| frame.value["method"] == m)
                {
                    return frame;
                }
            }
        })
        .await
        .expect("expected protocol frame")
    }
}

async fn connected(events: &async_channel::Receiver<Event>) {
    timeout(DEADLINE, async {
        while !matches!(events.recv().await.unwrap(), Event::Connected) {}
    })
    .await
    .unwrap();
}

async fn view(controller: &Controller, predicate: impl Fn(&View) -> bool) -> Arc<View> {
    let mut views = controller.views.clone();
    timeout(DEADLINE, async {
        loop {
            let next = views.borrow_and_update().clone();
            if predicate(&next) {
                return next;
            }
            views.changed().await.unwrap();
        }
    })
    .await
    .expect("expected controller view")
}

fn session(id: &str) -> Value {
    json!({"sessionId":id,"mode":"input","transport":"stream","cwd":"/test"})
}
fn snapshot(seq: u64, text: &str) -> Value {
    json!({"seq":seq,"first_seq":1,"items":[{"kind":"assistant_text","text":text}]})
}

#[tokio::test]
async fn bounded_calls_timeout_and_false_success_is_an_error() {
    let mut hub = Hub::new().await;
    hub.config.call_timeout = Duration::from_millis(100);
    let (client, events) = Client::start(hub.config.clone());
    connected(&events).await;
    let c = client.clone();
    let call =
        tokio::spawn(async move { c.call("agents.sendMessage", json!({"text":"hello"})).await });
    let frame = hub.frame("call", None).await;
    frame
        .result(json!({"ok":false,"error":"session stopped"}))
        .await;
    assert!(
        call.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("session stopped")
    );
    let c = client.clone();
    let call = tokio::spawn(async move { c.call("agents.sendMessage", json!({})).await });
    let _frame = hub.frame("call", None).await;
    let error = call.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("unknown"), "{error}");
}

#[tokio::test]
async fn disconnect_fails_mutation_and_never_replays_it() {
    let mut hub = Hub::new().await;
    let (client, events) = Client::start(hub.config.clone());
    connected(&events).await;
    client
        .topics(BTreeSet::from(["agent.snapshot".into()]))
        .await
        .unwrap();
    let c = client.clone();
    let call =
        tokio::spawn(async move { c.call("agents.sendMessage", json!({"text":"once"})).await });
    let frame = hub.frame("call", None).await;
    frame.send.send(Message::Close(None)).await.unwrap();
    assert!(
        call.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("unknown")
    );
    connected(&events).await;
    let subscription = hub.frame("subscribe", None).await;
    assert_eq!(subscription.value["topics"], json!(["agent.snapshot"]));
    let c = client.clone();
    let read = tokio::spawn(async move { c.call("sessions.snapshots", json!({})).await });
    let next = hub.frame("call", None).await;
    assert_eq!(next.value["method"], "sessions.snapshots");
    next.result(json!([])).await;
    read.await.unwrap().unwrap();
}

#[tokio::test]
async fn controller_reconciles_snapshot_races_gaps_and_stale_selection() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet
        .event(
            "agent.snapshot",
            json!({"sessionId":"a","label":"Fresh label"}),
        )
        .await;
    fleet.result(json!([session("a"), session("b")])).await;
    let initial = hub.frame("call", Some("sessions.conversation")).await;
    assert_eq!(initial.value["params"]["sessionId"], "a");
    initial
        .event(
            "agent.conversation.a",
            json!({"seq":11,"items":[{"kind":"assistant_text","text":" world"}]}),
        )
        .await;
    initial.result(snapshot(10, "hello")).await;
    let next = view(&controller, |v| v.transcript.seq == Some(11)).await;
    assert_eq!(next.transcript.rows[0].text, "hello world");
    assert_eq!(next.sessions[0].label, "Fresh label");

    initial
        .event(
            "agent.conversation.a",
            json!({"seq":13,"items":[{"kind":"assistant_text","text":" gap"}]}),
        )
        .await;
    let resync = hub.frame("call", Some("sessions.conversation")).await;
    controller.command(Command::Select("b".into())).unwrap();
    let changed = hub.frame("call", Some("sessions.conversation")).await;
    assert_eq!(changed.value["params"]["sessionId"], "b");
    changed.result(snapshot(2, "Session B")).await;
    view(&controller, |v| {
        v.selected.as_deref() == Some("b") && v.transcript.seq == Some(2)
    })
    .await;
    resync.result(snapshot(13, "Stale session A")).await;
    changed
        .event(
            "agent.conversation.b",
            json!({"seq":3,"items":[{"kind":"assistant_text","text":" stays selected"}]}),
        )
        .await;
    let next = view(&controller, |v| v.transcript.seq == Some(3)).await;
    assert_eq!(next.transcript.rows[0].text, "Session B stays selected");
}

#[tokio::test]
async fn approval_contract_send_receipt_and_reconnect_reseed_use_real_websocket() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    let mut row = session("a");
    row["pendingApproval"] = json!({"toolName":"Bash"});
    fleet.result(json!([row])).await;
    let conversation = hub.frame("call", Some("sessions.conversation")).await;
    conversation.result(snapshot(1, "Start")).await;
    view(&controller, |v| v.transcript.seq == Some(1)).await;
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::Approve(true),
        })
        .unwrap();
    let approval = hub.frame("call", Some("claude.approve")).await;
    assert_eq!(
        approval.value["params"],
        json!({"sessionId":"a","decision":"yes"})
    );
    approval.result(json!({"ok":true})).await;
    view(&controller, |v| {
        v.receipt.as_ref().is_some_and(|r| r.error.is_none())
    })
    .await;
    // Close with read requests in flight; old results may not revive old state.
    approval.send.send(Message::Close(None)).await.unwrap();
    view(&controller, |v| !v.connected).await;
    let reseed = hub.frame("call", Some("sessions.snapshots")).await;
    reseed.result(json!([session("a")])).await;
    let conversation = hub.frame("call", Some("sessions.conversation")).await;
    conversation.result(snapshot(20, "After reconnect")).await;
    view(&controller, |v| v.connected && v.transcript.seq == Some(20)).await;
}

#[tokio::test]
async fn demo_fixture_completes_a_streaming_send_round_trip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config::new(format!("ws://{}/bus", listener.local_addr().unwrap()), None).unwrap();
    let server = tokio::spawn(wks_native::harness::serve(listener, 100, 100));
    let controller = Controller::start(config);
    view(&controller, |v| {
        v.connected && v.transcript.seq == Some(100)
    })
    .await;
    controller
        .command(Command::Act {
            session: "demo-0000".into(),
            action: Action::Send("integration test".into()),
        })
        .unwrap();
    let v = view(&controller, |v| {
        v.transcript.seq.is_some_and(|seq| seq > 102) && v.receipt.is_some()
    })
    .await;
    assert!(
        v.transcript
            .rows
            .iter()
            .any(|r| r.text == "integration test")
    );
    assert!(
        v.transcript
            .rows
            .back()
            .unwrap()
            .text
            .contains("Streaming native text")
    );
    assert!(!v.busy);
    server.abort();
}

#[tokio::test]
async fn reset_during_snapshot_requires_a_read_after_the_reset() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([session("a")])).await;
    let first = hub.frame("call", Some("sessions.conversation")).await;
    first
        .event(
            "agent.conversation.a",
            json!({"reset":true,"seq":1,"items":[{"kind":"assistant_text","text":"reset start"}]}),
        )
        .await;
    // This response may be newer than the reset, or from the old generation.
    // The client must not guess from these sequence numbers.
    first.result(snapshot(3, "newer than reset")).await;
    let after_reset = hub.frame("call", Some("sessions.conversation")).await;
    after_reset.result(snapshot(3, "newer than reset")).await;
    let v = view(&controller, |v| v.transcript.seq == Some(3)).await;
    assert_eq!(v.transcript.rows[0].text, "newer than reset");
}

#[tokio::test]
async fn readiness_closes_the_gap_after_the_initial_snapshot() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([session("a")])).await;
    let first = hub.frame("call", Some("sessions.conversation")).await;
    first.result(snapshot(1, "before subscription")).await;
    view(&controller, |v| v.transcript.seq == Some(1)).await;
    // No delta for seq2 reaches this client: it occurred before demand became
    // active, and the session is now idle. Ready must trigger recovery itself.
    first
        .event("agent.conversation.a", json!({"ready":true}))
        .await;
    let after_ready = hub.frame("call", Some("sessions.conversation")).await;
    after_ready
        .result(snapshot(2, "final answer during subscription gap"))
        .await;
    let v = view(&controller, |v| v.transcript.seq == Some(2)).await;
    assert_eq!(
        v.transcript.rows[0].text,
        "final answer during subscription gap"
    );
}

#[tokio::test]
async fn readiness_during_an_inflight_read_queues_a_new_read() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([session("a")])).await;
    let first = hub.frame("call", Some("sessions.conversation")).await;
    first
        .event("agent.conversation.a", json!({"ready":true}))
        .await;
    first
        .result(snapshot(1, "snapshot taken before demand"))
        .await;
    let after_ready = hub.frame("call", Some("sessions.conversation")).await;
    after_ready.result(snapshot(2, "after demand")).await;
    view(&controller, |v| v.transcript.seq == Some(2)).await;
}

#[tokio::test]
async fn a_reset_processed_after_a_completed_snapshot_cannot_overwrite_it() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([session("a")])).await;
    let first = hub.frame("call", Some("sessions.conversation")).await;
    first.result(snapshot(3, "newer snapshot")).await;
    view(&controller, |v| v.transcript.seq == Some(3)).await;
    first
        .event(
            "agent.conversation.a",
            json!({"reset":true,"seq":1,"items":[{"kind":"assistant_text","text":"older reset"}]}),
        )
        .await;
    let after_reset = hub.frame("call", Some("sessions.conversation")).await;
    after_reset.result(snapshot(3, "newer snapshot")).await;
    let v = view(&controller, |v| !v.loading && v.transcript.seq == Some(3)).await;
    assert_eq!(v.transcript.rows[0].text, "newer snapshot");
}

#[tokio::test]
// Tungstenite requires this unboxed HTTP response as its handshake callback error.
#[allow(clippy::result_large_err)]
async fn authentication_failure_is_visible_without_disclosing_the_credential() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config::new(
        format!("ws://{}/bus", listener.local_addr().unwrap()),
        Some("private-test-token".into()),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, _response| {
                assert_eq!(
                    request.headers()["Authorization"],
                    "Bearer private-test-token"
                );
                Err(tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(401)
                    .body(Some("unauthorized".into()))
                    .unwrap())
            },
        )
        .await;
    });
    let (_client, events) = Client::start(config);
    let Event::Disconnected(reason) = timeout(DEADLINE, events.recv()).await.unwrap().unwrap()
    else {
        panic!("expected authentication failure")
    };
    assert!(reason.contains("HTTP 401"));
    assert!(!reason.contains("private-test-token"));
    server.await.unwrap();
}

#[tokio::test]
async fn live_harness_targets_and_cleans_up_only_its_disposable_session() {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config::new(format!("ws://{}/bus", listener.local_addr().unwrap()), None).unwrap();
    let followup = Arc::new(AtomicBool::new(false));
    let mutations = Arc::new(Mutex::new(Vec::new()));
    let recorded = mutations.clone();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let followup = followup.clone();
            let mutations = mutations.clone();
            tokio::spawn(async move {
                let mut ws = accept_async(stream).await.unwrap();
                ws.send(Message::Text(json!({"op":"hello"}).to_string()))
                    .await
                    .unwrap();
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let f: Value = serde_json::from_str(&text).unwrap();
                    if f["op"] != "call" {
                        continue;
                    }
                    let method = f["method"].as_str().unwrap();
                    let result = match method {
                        "agents.spawn" => json!({"sessionId":"disposable"}),
                        "sessions.snapshots" => {
                            json!([session("aaa-existing"), session("disposable")])
                        }
                        "sessions.conversation" => {
                            if f["params"]["sessionId"] != "disposable" {
                                snapshot(99, "Unrelated existing conversation")
                            } else if followup.load(Ordering::SeqCst) {
                                snapshot(2, "NATIVE_FOLLOWUP_OK")
                            } else {
                                snapshot(1, "NATIVE_TEST_READY")
                            }
                        }
                        "agents.sendMessage" => {
                            mutations
                                .lock()
                                .unwrap()
                                .push((method.to_owned(), f["params"]["sessionId"].clone()));
                            followup.store(true, Ordering::SeqCst);
                            json!({"ok":true})
                        }
                        "claude.signal" => {
                            assert_eq!(f["params"]["signal"], "SIGTERM");
                            mutations
                                .lock()
                                .unwrap()
                                .push((method.to_owned(), f["params"]["sessionId"].clone()));
                            json!({"ok":true})
                        }
                        other => panic!("unexpected live harness method {other}"),
                    };
                    if ws
                        .send(Message::Text(
                            json!({"op":"result","id":f["id"],"result":result}).to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    let result = timeout(
        Duration::from_secs(10),
        wks_native::live::run(config, "claude".into(), std::env::temp_dir(), false),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result["session_id"], "disposable");
    assert_eq!(result["fresh_client_reseed_verified"], true);
    assert_eq!(
        *recorded.lock().unwrap(),
        vec![
            ("agents.sendMessage".into(), json!("disposable")),
            ("claude.signal".into(), json!("disposable"))
        ]
    );
    server.abort();
}

fn launch_request() -> wks_native::controller::NewSession {
    wks_native::controller::NewSession {
        provider: "codex".into(),
        cwd: "/work/project".into(),
        label: "My session".into(),
        model: String::new(),
        message: "Please inspect the project".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn create_session_uses_real_capability_and_survives_stale_fleet() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let old_fleet = hub.frame("call", Some("sessions.snapshots")).await;
    view(&controller, |v| v.connected).await;
    controller
        .command(Command::Create(launch_request()))
        .unwrap();
    let spawn = hub.frame("call", Some("agents.spawn")).await;
    assert_eq!(
        spawn.value["params"],
        json!({"provider":"codex","cwd":"/work/project",
        "label":"My session","transport":"stream","skipPermissions":false,"permissionMode":"ask","message":"Please inspect the project"})
    );
    // A second click while the acknowledgement is outstanding cannot launch twice.
    controller
        .command(Command::Create(launch_request()))
        .unwrap();
    spawn
        .result(json!({"sessionId":"new","messageQueued":true}))
        .await;
    let created = view(&controller, |v| v.spawn_receipt.is_some()).await;
    assert_eq!(created.selected.as_deref(), Some("new"));
    assert!(!created.creating);
    assert!(
        created
            .spawn_receipt
            .as_ref()
            .unwrap()
            .unsent_message
            .is_none()
    );
    old_fleet.result(json!([])).await;
    let conversation = hub.frame("call", Some("sessions.conversation")).await;
    conversation.result(snapshot(1, "Ready")).await;
    let ready = view(&controller, |v| !v.loading && !v.transcript.rows.is_empty()).await;
    assert_eq!(ready.selected.as_deref(), Some("new"));
    assert!(ready.sessions.iter().any(|s| s.id == "new"));
    while let Ok(frame) = hub.frames.try_recv() {
        assert_ne!(frame.value["method"], "agents.spawn", "duplicate launch");
    }
}

#[tokio::test]
async fn create_session_retains_unconfirmed_prompt_without_resending() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    controller
        .command(Command::Create(launch_request()))
        .unwrap();
    hub.frame("call", Some("agents.spawn"))
        .await
        .result(json!({"sessionId":"new","messageQueued":false}))
        .await;
    let created = view(&controller, |v| v.spawn_receipt.is_some()).await;
    let receipt = created.spawn_receipt.as_ref().unwrap();
    assert_eq!(
        receipt.unsent_message.as_deref(),
        Some("Please inspect the project")
    );
    assert_eq!(receipt.session.as_deref(), Some("new"));
    assert!(receipt.error.is_none());
    assert!(created.notice.contains("not confirmed"));
}

#[tokio::test]
async fn disconnected_spawn_is_reported_and_never_replayed() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    controller
        .command(Command::Create(launch_request()))
        .unwrap();
    let spawn = hub.frame("call", Some("agents.spawn")).await;
    spawn.send.send(Message::Close(None)).await.unwrap();
    let failed = view(&controller, |v| v.spawn_receipt.is_some()).await;
    assert!(!failed.creating);
    assert!(
        failed
            .spawn_receipt
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("unknown")
    );
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    while let Ok(frame) = hub.frames.try_recv() {
        assert_ne!(frame.value["method"], "agents.spawn");
    }
}

#[test]
fn creation_validates_hub_paths_without_checking_the_clients_filesystem() {
    let mut request = launch_request();
    for path in ["/remote/project", "C:\\Work\\project", "\\\\server\\share"] {
        request.cwd = path.into();
        assert!(request.params().is_ok());
    }
    for path in ["", "relative/project", "~/project", "C:relative"] {
        request.cwd = path.into();
        assert!(request.params().is_err());
    }
}

#[tokio::test]
async fn catalogs_are_scoped_and_late_provider_results_do_not_replace_selection() {
    use wks_native::launch::CatalogKey;
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    let claude = CatalogKey {
        provider: "claude".into(),
        cwd: String::new(),
    };
    let codex = CatalogKey {
        provider: "codex".into(),
        cwd: "/remote/project".into(),
    };
    controller
        .command(Command::LoadModels {
            key: claude.clone(),
            refresh: false,
        })
        .unwrap();
    let old = hub.frame("call", Some("claude.listModels")).await;
    controller
        .command(Command::LoadModels {
            key: codex.clone(),
            refresh: false,
        })
        .unwrap();
    let current = hub.frame("call", Some("providers.listModels")).await;
    assert_eq!(
        current.value["params"],
        json!({"provider":"codex","cwd":"/remote/project"})
    );
    current
        .result(json!([{"id":"exact-codex","label":"Codex model"}]))
        .await;
    let loaded = view(&controller, |v| {
        !v.catalog.loading && !v.catalog.models.is_empty()
    })
    .await;
    assert_eq!(loaded.catalog.key, codex);
    old.result(json!({"aliases":[{"model":"opus","contextWindow":1000000}]}))
        .await;
    controller
        .command(Command::LoadModels {
            key: codex.clone(),
            refresh: true,
        })
        .unwrap();
    let refresh = hub.frame("call", Some("providers.listModels")).await;
    refresh.result(json!([])).await;
    let failed = view(&controller, |v| v.catalog.error.is_some()).await;
    assert_eq!(failed.catalog.key, codex);
    assert_eq!(failed.catalog.models[0].id, "exact-codex");
    assert!(!failed.catalog.loading);
    controller
        .command(Command::LoadModels {
            key: claude,
            refresh: false,
        })
        .unwrap();
    hub.frame("call", Some("claude.listModels"))
        .await
        .result(json!({"aliases":[{"model":"sonnet","contextWindow":200000}]}))
        .await;
    let recovered = view(&controller, |v| {
        !v.catalog.loading && v.catalog.key.provider == "claude"
    })
    .await;
    assert_eq!(recovered.catalog.models[0].id, "sonnet");
    assert!(recovered.catalog.error.is_none());
}

#[tokio::test]
async fn model_discovery_requires_hub_directory_for_codex() {
    use wks_native::launch::CatalogKey;
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    controller
        .command(Command::LoadModels {
            key: CatalogKey {
                provider: "codex".into(),
                cwd: "relative".into(),
            },
            refresh: false,
        })
        .unwrap();
    let rejected = view(&controller, |v| v.catalog.error.is_some()).await;
    assert!(
        rejected
            .catalog
            .error
            .as_deref()
            .unwrap()
            .contains("absolute")
    );
    controller
        .command(Command::Create(launch_request()))
        .unwrap();
    let next = hub.frame("call", None).await;
    assert_eq!(
        next.value["method"], "agents.spawn",
        "invalid catalog cwd must not reach the hub"
    );
}

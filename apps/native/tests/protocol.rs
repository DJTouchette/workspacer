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
        Self::serving(false).await
    }
    /// Serves every accepted client at once, for several controllers on one hub.
    async fn shared() -> Self {
        Self::serving(true).await
    }
    async fn serving(concurrent: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config =
            Config::new(format!("ws://{}/bus", listener.local_addr().unwrap()), None).unwrap();
        let (send, frames) = mpsc::channel(128);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let connection = serve_connection(stream, send.clone());
                if concurrent {
                    tokio::spawn(connection);
                } else {
                    connection.await;
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

async fn serve_connection(stream: tokio::net::TcpStream, send: mpsc::Sender<Frame>) {
    let Ok(mut socket) = accept_async(stream).await else {
        return;
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
async fn session_loading_and_read_retry_follow_actual_replies() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    view(&controller, |v| v.connected && v.sessions_loading).await;
    fleet.result(json!([session("a")])).await;
    let read = hub.frame("call", Some("sessions.conversation")).await;
    view(&controller, |v| !v.sessions_loading && v.loading).await;
    read.send
        .send(Message::Text(
            json!({"op":"error", "id":read.value["id"], "error":"Read failed"}).to_string(),
        ))
        .await
        .unwrap();
    view(&controller, |v| {
        !v.loading && v.notice.starts_with("Conversation unavailable:")
    })
    .await;
    controller.command(Command::Refresh).unwrap();
    let retry_fleet = hub.frame("call", Some("sessions.snapshots")).await;
    let retry_read = hub.frame("call", Some("sessions.conversation")).await;
    view(&controller, |v| v.sessions_loading && v.loading).await;
    retry_fleet.result(json!([session("a")])).await;
    retry_read
        .result(snapshot(1, "Recovered conversation"))
        .await;
    let recovered = view(&controller, |v| {
        !v.sessions_loading && !v.loading && !v.transcript.rows.is_empty()
    })
    .await;
    assert!(recovered.notice.is_empty());
}

fn page(seq: u64, first: u64, count: u64) -> Value {
    let items: Vec<Value> = (first..first + count)
        .map(|n| json!({"kind":"user_message","text":format!("message {n}")}))
        .collect();
    json!({"seq":seq,"first_seq":1,"window_first_seq":first,"items":items})
}

#[tokio::test]
async fn conversations_open_on_the_newest_page_and_page_back_on_request() {
    use wks_native::controller::CONVERSATION_PAGE;
    let page_size = CONVERSATION_PAGE as u64;
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([session("a")]))
        .await;
    let read = hub.frame("call", Some("sessions.conversation")).await;
    assert_eq!(read.value["params"]["limit"], CONVERSATION_PAGE);
    // 500 retained items; the newest page starts at 301.
    read.result(page(500, 500 - page_size + 1, page_size)).await;
    let opened = view(&controller, |v| !v.loading && !v.transcript.rows.is_empty()).await;
    assert!(opened.transcript.has_older);
    assert_eq!(opened.transcript.rows.len(), CONVERSATION_PAGE);
    controller.command(Command::LoadOlder).unwrap();
    let older = hub.frame("call", Some("sessions.conversation")).await;
    assert_eq!(older.value["params"]["limit"], 2 * CONVERSATION_PAGE);
    view(&controller, |v| v.loading_older).await;
    older
        .result(page(500, 500 - 2 * page_size + 1, 2 * page_size))
        .await;
    let widened = view(&controller, |v| {
        !v.loading_older && v.transcript.rows.len() > CONVERSATION_PAGE
    })
    .await;
    assert!(widened.transcript.has_older);
    // Rows already on screen keep their keys, so the reader's anchor holds.
    let newest = opened.transcript.rows.back().unwrap().key;
    assert_eq!(widened.transcript.rows.back().unwrap().key, newest);
    controller.command(Command::LoadOlder).unwrap();
    let rest = hub.frame("call", Some("sessions.conversation")).await;
    assert_eq!(rest.value["params"]["limit"], 3 * CONVERSATION_PAGE);
    rest.result(page(500, 1, 500)).await;
    let all = view(&controller, |v| {
        !v.loading_older && v.transcript.rows.len() == 500
    })
    .await;
    assert!(!all.transcript.has_older, "the whole log is loaded");
    assert!(!all.transcript.omitted);
}

#[tokio::test]
async fn daemons_without_paging_read_completely_and_never_offer_older_pages() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([session("a")]))
        .await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(3, "An older daemon ignores limit"))
        .await;
    let v = view(&controller, |v| !v.loading && !v.transcript.rows.is_empty()).await;
    assert!(!v.transcript.has_older);
    controller.command(Command::LoadOlder).unwrap();
    // No read follows; the next frame is the next unrelated call, if any.
    assert!(
        timeout(
            Duration::from_millis(300),
            hub.frame("call", Some("sessions.conversation"))
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn subagent_view_reads_the_child_ignores_parent_deltas_and_returns() {
    use wks_native::controller::ChildTarget;
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let mut parent = session("a");
    parent["subagents"] = json!([{"id":"task-1","description":"Audit","status":"running"}]);
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([parent]))
        .await;
    let read = hub.frame("call", Some("sessions.conversation")).await;
    read.result(snapshot(5, "Parent reply")).await;
    view(&controller, |v| {
        v.transcript.rows.iter().any(|r| r.text == "Parent reply")
    })
    .await;

    controller
        .command(Command::ViewChild(Some(ChildTarget {
            parent: "a".into(),
            agent: "task-1".into(),
        })))
        .unwrap();
    let child = hub
        .frame("call", Some("sessions.subagentConversation"))
        .await;
    assert_eq!(child.value["params"]["sessionId"], "a");
    assert_eq!(child.value["params"]["agentId"], "task-1");
    child
        .result(
            json!({"session_id":"a","agent_id":"task-1","seq":2,"first_seq":1,
            "items":[{"kind":"assistant_text","text":"Child finding"}]}),
        )
        .await;
    let v = view(&controller, |v| {
        v.child.is_some() && v.transcript.rows.iter().any(|r| r.text == "Child finding")
    })
    .await;
    assert!(!v.transcript.rows.iter().any(|r| r.text == "Parent reply"));
    // Parent deltas while the child is shown never land in its transcript.
    child
        .event(
            "agent.conversation.a",
            json!({"session_id":"a","seq":6,"items":[{"kind":"assistant_text","text":"Parent moved on"}]}),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let v = controller.views.borrow().clone();
    assert!(
        !v.transcript
            .rows
            .iter()
            .any(|r| r.text == "Parent moved on")
    );

    controller.command(Command::ViewChild(None)).unwrap();
    let back = hub.frame("call", Some("sessions.conversation")).await;
    assert_eq!(back.value["params"]["sessionId"], "a");
    back.result(snapshot(6, "Parent moved on")).await;
    let v = view(&controller, |v| {
        v.child.is_none()
            && v.transcript
                .rows
                .iter()
                .any(|r| r.text == "Parent moved on")
    })
    .await;
    assert!(!v.transcript.rows.iter().any(|r| r.text == "Child finding"));
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
    assert!(
        v.notice.is_empty(),
        "routine send acknowledgements do not add a banner"
    );
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
                        "usage.report" => json!({"providers":[]}),
                        // Read on every connect; archiving is never a mutation here.
                        "sessionArchive.get" => json!({"version":0,"archived":{}}),
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

/// An unnamed launch asks the owning hub to name it (`autoTitle`), and the
/// name it later publishes is what the session shows. A typed label is sent
/// as the name instead and never asks for one (see the test above).
#[tokio::test]
async fn unnamed_launch_requests_a_hub_title_and_shows_it_when_published() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([])).await;
    view(&controller, |v| v.connected).await;
    let mut request = launch_request();
    request.label = "   ".into();
    controller.command(Command::Create(request)).unwrap();
    let spawn = hub.frame("call", Some("agents.spawn")).await;
    assert_eq!(spawn.value["params"]["autoTitle"], true);
    assert!(spawn.value["params"].get("label").is_none());
    spawn
        .result(json!({"sessionId":"new","messageQueued":true}))
        .await;
    spawn
        .event(
            "agent.snapshot",
            json!({"sessionId":"new","provider":"codex","cwd":"/work/project","mode":"responding","autoTitle":{"state":"pending"}}),
        )
        .await;
    let pending = view(&controller, |v| v.sessions.iter().any(|s| s.id == "new")).await;
    assert_eq!(
        pending
            .sessions
            .iter()
            .find(|s| s.id == "new")
            .unwrap()
            .label,
        ""
    );
    spawn
        .event(
            "agent.snapshot",
            json!({"sessionId":"new","mode":"input","label":"Inspect the project","autoTitle":{"state":"titled","source":"model"}}),
        )
        .await;
    view(&controller, |v| {
        v.sessions
            .iter()
            .any(|s| s.id == "new" && s.label == "Inspect the project")
    })
    .await;
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
        json!({"provider":"codex","cwd":"/remote/project","useHomeDirectory":false})
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
async fn model_discovery_allows_hub_home_before_project_and_rejects_relative_codex_directory() {
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
                cwd: String::new(),
            },
            refresh: false,
        })
        .unwrap();
    let initial = hub.frame("call", Some("providers.listModels")).await;
    assert_eq!(initial.value["params"]["cwd"], "");
    assert_eq!(initial.value["params"]["useHomeDirectory"], true);
    initial
        .result(json!([{"id":"initial-model", "label":"Initial model"}]))
        .await;
    let catalog = view(&controller, |v| {
        !v.catalog.loading && !v.catalog.models.is_empty()
    })
    .await;
    assert_eq!(catalog.catalog.models[0].id, "initial-model");
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

#[tokio::test]
async fn secondary_reads_are_superseded_and_never_cross_sessions() {
    use wks_native::features::Request;
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([session("a"), session("b")]))
        .await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(1, "a"))
        .await;
    view(&controller, |v| {
        v.selected.as_deref() == Some("a") && !v.loading
    })
    .await;
    controller
        .command(Command::Request(Request::Diff {
            cwd: "/test".into(),
            path: "old.rs".into(),
            staged: false,
            untracked: false,
        }))
        .unwrap();
    let old = hub.frame("call", Some("git.diff")).await;
    controller
        .command(Command::Request(Request::Diff {
            cwd: "/test".into(),
            path: "new.rs".into(),
            staged: true,
            untracked: false,
        }))
        .unwrap();
    let new = hub.frame("call", Some("git.diff")).await;
    assert_eq!(new.value["params"]["staged"], true);
    new.result(json!({"diff":"+new"})).await;
    old.result(json!({"diff":"+old"})).await;
    view(&controller, |v| {
        v.requests
            .get("diff")
            .is_some_and(|s| s.value["diff"] == "+new")
    })
    .await;
    controller.command(Command::Select("b".into())).unwrap();
    let selected = view(&controller, |v| v.selected.as_deref() == Some("b")).await;
    assert!(!selected.requests.contains_key("diff"));
}

#[tokio::test]
async fn provider_child_history_uses_parent_route_and_fences_old_child_reads() {
    use wks_native::features::Request;
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots")).await
        .result(json!([{"sessionId":"a","mode":"input","transport":"stream","cwd":"/test",
            "subagents":[{"id":"old-child"},{"id":"native-child"},{"id":"unavailable"},{"id":"pending-child"}]}, session("b")])).await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(1, "parent"))
        .await;
    view(&controller, |v| {
        v.selected.as_deref() == Some("a") && !v.loading
    })
    .await;
    let request = |agent: &str| {
        Command::Request(Request::SubagentHistory {
            session: "a".into(),
            agent: agent.into(),
        })
    };
    controller.command(request("old-child")).unwrap();
    let old = hub
        .frame("call", Some("sessions.subagentConversation"))
        .await;
    controller.command(request("native-child")).unwrap();
    let child = hub
        .frame("call", Some("sessions.subagentConversation"))
        .await;
    assert_eq!(
        child.value["params"],
        json!({"sessionId":"a","agentId":"native-child"})
    );
    child
        .result(
            json!({"session_id":"a","agent_id":"native-child","seq":4,"items":[
                {"kind":"user_message","text":"inspect"},
                {"kind":"tool_use","id":"t1","name":"exec_command","input":{"cmd":"pwd"}},
                {"kind":"tool_result","tool_use_id":"t1","content":"/test"},
                {"kind":"assistant_text","text":"child done"}
            ]}),
        )
        .await;
    old.result(snapshot(1, "stale child")).await;
    let loaded = view(&controller, |v| {
        v.requests
            .get("subagent-history")
            .is_some_and(|state| !state.loading && state.value["agent_id"] == "native-child")
    })
    .await;
    assert_eq!(loaded.selected.as_deref(), Some("a"));
    assert_eq!(loaded.transcript.rows[0].text, "parent");
    let rows = loaded.requests["subagent-history"].value["rows"]
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1]["tool"]["output"], "/test");
    assert_eq!(rows[2]["text"], "child done");

    controller.command(request("unavailable")).unwrap();
    hub.frame("call", Some("sessions.subagentConversation"))
        .await
        .result(Value::Null)
        .await;
    view(&controller, |v| {
        v.requests.get("subagent-history").is_some_and(|state| {
            state.error.as_deref() == Some("Child transcript is not available yet")
        })
    })
    .await;
    controller.command(request("pending-child")).unwrap();
    let pending = hub
        .frame("call", Some("sessions.subagentConversation"))
        .await;
    controller.command(Command::Select("b".into())).unwrap();
    let selected = view(&controller, |v| v.selected.as_deref() == Some("b")).await;
    assert!(!selected.requests.contains_key("subagent-history"));
    pending.result(snapshot(1, "stale after switch")).await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(1, "other parent"))
        .await;
    let switched = view(&controller, |v| {
        v.selected.as_deref() == Some("b") && !v.loading
    })
    .await;
    assert!(!switched.requests.contains_key("subagent-history"));
    assert_eq!(switched.transcript.rows[0].text, "other parent");
}

#[tokio::test]
async fn recent_session_can_be_read_without_launching_and_resume_is_explicit() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    controller
        .command(Command::OpenRecent(Box::new(wks_native::model::Session {
            id: "ended".into(),
            cwd: "/project".into(),
            state: "stopped".into(),
            provider: "codex".into(),
            ..Default::default()
        })))
        .unwrap();
    let history = hub.frame("call", Some("sessions.conversation")).await;
    assert_eq!(history.value["params"]["sessionId"], "ended");
    history.result(snapshot(1, "old work")).await;
    view(&controller, |v| {
        v.selected.as_deref() == Some("ended") && !v.loading
    })
    .await;
    controller
        .command(Command::Act {
            session: "ended".into(),
            action: Action::Send("new work".into()),
        })
        .unwrap();
    let rejected = view(&controller, |v| v.receipt.is_some()).await;
    assert!(rejected.receipt.as_ref().unwrap().error.is_some());
    let mut resume = launch_request();
    resume.resume_session_id = Some("ended".into());
    controller.command(Command::Create(resume)).unwrap();
    let spawn = hub.frame("call", Some("agents.spawn")).await;
    assert_eq!(spawn.value["params"]["resumeSessionId"], "ended");
    assert_eq!(spawn.value["params"]["permissionMode"], "ask");
}

#[tokio::test]
async fn end_model_and_question_controls_use_structured_contracts() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let mut row = session("a");
    row["pendingQuestions"] = json!([{"question":"Which?","options":[{"label":"One"}]}]);
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([row]))
        .await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(1, "a"))
        .await;
    view(&controller, |v| {
        v.selected.as_deref() == Some("a") && !v.loading
    })
    .await;
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::Answers(vec!["One".into()]),
        })
        .unwrap();
    let answer = hub.frame("call", Some("claude.answer")).await;
    assert_eq!(answer.value["params"]["answers"], json!(["One"]));
    assert_eq!(answer.value["params"]["answerKinds"], json!(["text"]));
    answer.result(json!({"ok":true})).await;
    view(&controller, |v| v.receipt.is_some() && !v.busy).await;
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::SetModel {
                model: "opus".into(),
                context_window: Some(1_000_000),
                effort: None,
            },
        })
        .unwrap();
    let model = hub.frame("call", Some("claude.setModel")).await;
    assert_eq!(model.value["params"]["contextWindow"], 1_000_000);
    assert!(
        model.value["params"].get("effort").is_none(),
        "effort never rides on the model call"
    );
    model
        .result(json!({"ok":true,"disposition":"queued"}))
        .await;
    view(&controller, |v| v.notice.contains("queued") && !v.busy).await;
    // Model and effort together: the model call first, then claude.setEffort,
    // which the hub routes per provider (Claude's /effort, Codex's thread).
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::SetModel {
                model: "gpt-5.5".into(),
                context_window: None,
                effort: Some("high".into()),
            },
        })
        .unwrap();
    hub.frame("call", Some("claude.setModel"))
        .await
        .result(json!({"ok":true,"model":"gpt-5.5"}))
        .await;
    let effort = hub.frame("call", Some("claude.setEffort")).await;
    assert_eq!(effort.value["params"]["effort"], "high");
    assert_eq!(effort.value["params"]["sessionId"], "a");
    effort
        .result(json!({"ok":false,"error":"this session can't take input right now (ended)"}))
        .await;
    view(&controller, |v| {
        v.notice.contains("model change was accepted") && v.notice.contains("ended") && !v.busy
    })
    .await;
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::SetEffort("low".into()),
        })
        .unwrap();
    let effort = hub.frame("call", Some("claude.setEffort")).await;
    assert_eq!(effort.value["params"]["effort"], "low");
    effort.result(json!({"ok":true,"effort":"low"})).await;
    view(&controller, |v| {
        v.receipt
            .as_ref()
            .is_some_and(|r| matches!(r.action, Action::SetEffort(_)) && r.error.is_none())
            && !v.busy
    })
    .await;
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::Terminate,
        })
        .unwrap();
    let end = hub.frame("call", Some("claude.signal")).await;
    assert_eq!(end.value["params"]["signal"], "SIGTERM");
}

#[tokio::test]
async fn attachment_upload_is_bound_to_original_session_and_checks_limits() {
    use wks_native::features::{AttachmentSource, Request};
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([session("a"), session("b")]))
        .await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(1, "a"))
        .await;
    view(&controller, |v| {
        v.selected.as_deref() == Some("a") && !v.loading
    })
    .await;
    controller
        .command(Command::Request(Request::Upload {
            session: "a".into(),
            source: AttachmentSource::Image {
                name: "screen.png".into(),
                bytes: Arc::new(vec![1, 2, 3]),
            },
        }))
        .unwrap();
    let upload = hub.frame("call", Some("files.upload")).await;
    assert_eq!(upload.value["params"]["dataBase64"], "AQID");
    controller.command(Command::Select("b".into())).unwrap();
    view(&controller, |v| v.selected.as_deref() == Some("b")).await;
    upload
        .result(json!({"path":"/remote/uploads/abc.png"}))
        .await;
    let result = view(&controller, |v| {
        v.requests.get("upload").is_some_and(|s| !s.loading)
    })
    .await;
    assert!(
        matches!(&result.requests["upload"].request, Request::Upload { session, .. } if session == "a")
    );
    assert_eq!(
        result.requests["upload"].value["path"],
        "/remote/uploads/abc.png"
    );
}

#[tokio::test]
async fn queued_bubbles_wait_for_a_new_authoritative_user_turn() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    let mut s = session("a");
    s["mode"] = json!("responding");
    fleet.result(json!([s])).await;
    let read = hub.frame("call", Some("sessions.conversation")).await;
    let old = json!({"seq":2,"first_seq":1,"items":[{"kind":"user_message","text":"again"},{"kind":"assistant_text","text":"Working"}]});
    read.result(old.clone()).await;
    view(&controller, |v| v.transcript.seq == Some(2)).await;
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::Send("again".into()),
        })
        .unwrap();
    let send = hub.frame("call", Some("agents.sendMessage")).await;
    let pending = view(&controller, |v| !v.pending_messages.is_empty()).await;
    assert!(pending.pending_messages[0].queued);
    assert!(!pending.pending_messages[0].accepted);
    send.result(json!({"ok":true,"queued":true})).await;
    let reread = hub.frame("call", Some("sessions.conversation")).await;
    reread.result(old).await;
    let accepted = view(&controller, |v| {
        v.pending_messages.first().is_some_and(|p| p.accepted) && !v.loading
    })
    .await;
    assert!(
        accepted.notice.is_empty(),
        "queued sends already have inline feedback"
    );
    assert_eq!(
        accepted.pending_messages.len(),
        1,
        "an identical historical send is not acknowledgement"
    );
    controller
        .command(Command::Act {
            session: "a".into(),
            action: Action::Send("again".into()),
        })
        .unwrap();
    let second = hub.frame("call", Some("agents.sendMessage")).await;
    second.result(json!({"ok":true,"queued":true})).await;
    hub.frame("call",Some("sessions.conversation")).await.result(json!({"seq":2,"first_seq":1,"items":[{"kind":"user_message","text":"again"},{"kind":"assistant_text","text":"Working"}]})).await;
    view(&controller, |v| {
        v.pending_messages.len() == 2 && v.pending_messages[1].accepted
    })
    .await;
    reread
        .event(
            "agent.conversation.a",
            json!({"seq":3,"items":[{"kind":"user_message","text":"again"}]}),
        )
        .await;
    view(&controller, |v| {
        v.transcript.seq == Some(3) && v.pending_messages.len() == 1
    })
    .await;
    reread
        .event(
            "agent.snapshot",
            json!({"sessionId":"a","label":"after first echo"}),
        )
        .await;
    let next = view(&controller, |v| {
        v.sessions.iter().any(|s| s.label == "after first echo")
    })
    .await;
    assert_eq!(
        next.pending_messages.len(),
        1,
        "the first echo cannot acknowledge a second identical send on the next frame"
    );
    reread
        .event(
            "agent.conversation.a",
            json!({"seq":4,"items":[{"kind":"user_message","text":"again"}]}),
        )
        .await;
    view(&controller, |v| {
        v.transcript.seq == Some(4) && v.pending_messages.is_empty()
    })
    .await;
}

#[tokio::test]
async fn native_card_diffs_use_the_owner_validated_desktop_service() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([session("a")]))
        .await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(1, "Ready"))
        .await;
    view(&controller, |v| v.transcript.seq == Some(1)).await;
    controller
        .command(Command::Request(wks_native::features::Request::CardDiff {
            session: "a".into(),
            path: "src/lib.rs".into(),
        }))
        .unwrap();
    let request = hub.frame("call", Some("desktop.htmlCardReadDiff")).await;
    assert_eq!(
        request.value["params"],
        json!({"ownerId":"a","target":"src/lib.rs"})
    );
    request
        .result(json!({"ok":false,"error":"Path is outside the owning project"}))
        .await;
    let v = view(&controller, |v| {
        v.requests
            .get("card-diff")
            .is_some_and(|r| r.error.is_some())
    })
    .await;
    assert!(
        v.requests["card-diff"]
            .error
            .as_ref()
            .unwrap()
            .contains("outside")
    );
}

#[tokio::test]
async fn ui_events_are_bounded_display_intents_and_terminal_requests_never_spawn_headlessly() {
    use wks_native::ui_requests::{Intent, MAX_PENDING};
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let subscription = loop {
        let frame = hub.frame("subscribe", None).await;
        if frame.value["topics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "facade.openTerminal")
        {
            break frame;
        }
    };
    for topic in wks_native::ui_requests::TOPICS {
        assert!(
            subscription.value["topics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == topic)
        );
    }
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([])).await;
    let payload = json!({"cwd":"/workspace","command":"echo visible only","label":"Checks","parentSessionId":"manager"});
    fleet.event("facade.openTerminal", payload.clone()).await;
    let pending = view(&controller, |v| v.ui_requests.len() == 1).await;
    assert!(
        matches!(&pending.ui_requests[0].intent,Intent::Terminal{command,..} if command=="echo visible only")
    );
    assert_eq!(pending.ui_requests[0].payload, payload);
    tokio::time::sleep(Duration::from_millis(60)).await;
    while let Ok(frame) = hub.frames.try_recv() {
        assert_ne!(frame.value["method"], "terminals.create");
        assert_ne!(frame.value["method"], "agents.spawn");
        assert_ne!(frame.value["method"], "sessions.terminalInput");
    }
    controller
        .command(Command::ConsumeUiRequest(pending.ui_requests[0].number))
        .unwrap();
    view(&controller, |v| v.ui_requests.is_empty()).await;
    for _ in 0..MAX_PENDING + 2 {
        fleet
            .event("command.open_spawn_dialog", json!({"cwd":"/project"}))
            .await;
    }
    let full = view(&controller, |v| !v.ui_request_warning.is_empty()).await;
    assert_eq!(full.ui_requests.len(), MAX_PENDING);
    assert!(!full.creating);
    assert!(full.spawn_receipt.is_none());
}

#[tokio::test]
async fn foreign_ui_requests_cannot_navigate_the_local_workspace() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([])).await;
    view(&controller, |v| v.connected).await;
    fleet.send.send(Message::Text(json!({"op":"event","event":{"type":"facade.openTerminal","hub":"other","data":{"cwd":"/remote","command":"no"}}}).to_string())).await.unwrap();
    fleet
        .event(
            "command.focus_agent",
            json!({"sessionId":"foreign","hub":"other"}),
        )
        .await;
    fleet
        .event("command.open_pane", json!({"paneType":"settings"}))
        .await;
    let v = view(&controller, |v| !v.ui_requests.is_empty()).await;
    assert_eq!(v.ui_requests.len(), 1);
    assert!(
        matches!(&v.ui_requests[0].intent,wks_native::ui_requests::Intent::OpenPane{pane_type,..} if pane_type=="settings")
    );
}

#[tokio::test]
async fn power_close_before_and_after_hello_requires_explicit_resume_and_never_replays() {
    use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
    for hello in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config =
            Config::new(format!("ws://{}/bus", listener.local_addr().unwrap()), None).unwrap();
        let (client, events) = Client::start(config);
        let (stream, _) = timeout(DEADLINE, listener.accept()).await.unwrap().unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let pending = if hello {
            socket
                .send(Message::Text(json!({"op":"hello"}).to_string()))
                .await
                .unwrap();
            connected(&events).await;
            let client = client.clone();
            let pending = tokio::spawn(async move {
                client
                    .call("agents.sendMessage", json!({"sessionId":"a","text":"once"}))
                    .await
            });
            loop {
                if let Some(Ok(Message::Text(text))) = socket.next().await {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["op"] == "call" {
                        assert_eq!(frame["method"], "agents.sendMessage");
                        break;
                    }
                }
            }
            Some(pending)
        } else {
            None
        };
        socket
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::Library(4001),
                reason: "machine stopping; reconnect only to wake".into(),
            })))
            .await
            .unwrap();
        timeout(DEADLINE, async {
            while !matches!(events.recv().await.unwrap(), Event::PowerPaused) {}
        })
        .await
        .unwrap();
        if let Some(pending) = pending {
            assert!(
                pending
                    .await
                    .unwrap()
                    .unwrap_err()
                    .to_string()
                    .contains("outcome unknown")
            );
        }
        assert!(client.call("fixture.read", json!({})).await.is_err());
        client
            .topics(BTreeSet::from(["agent.snapshot".into()]))
            .await
            .unwrap();
        assert!(
            timeout(Duration::from_millis(650), listener.accept())
                .await
                .is_err(),
            "paused client redialed without user intent"
        );
        client.resume_power_pause().unwrap();
        assert!(
            client.resume_power_pause().is_err(),
            "duplicate resume queued for a later pause"
        );
        let (stream, _) = timeout(DEADLINE, listener.accept()).await.unwrap().unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(Message::Text(json!({"op":"hello"}).to_string()))
            .await
            .unwrap();
        connected(&events).await;
        let frame = timeout(DEADLINE, async {
            loop {
                if let Some(Ok(Message::Text(text))) = socket.next().await {
                    break serde_json::from_str::<Value>(&text).unwrap();
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(frame["op"], "subscribe");
        assert_eq!(frame["topics"], json!(["agent.snapshot"]));
        assert!(
            timeout(Duration::from_millis(80), socket.next())
                .await
                .is_err(),
            "old mutation replayed after resume"
        );
        socket
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::Library(4001),
                reason: "second stop".into(),
            })))
            .await
            .unwrap();
        timeout(DEADLINE, async {
            while !matches!(events.recv().await.unwrap(), Event::PowerPaused) {}
        })
        .await
        .unwrap();
        assert!(
            timeout(Duration::from_millis(350), listener.accept())
                .await
                .is_err(),
            "a previous resume escaped into the next stop"
        );
        drop(client);
        drop(events);
    }
}

#[tokio::test]
async fn controller_pauses_all_host_reads_until_user_refresh() {
    use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = hub.frame("call", Some("sessions.snapshots")).await;
    fleet.result(json!([])).await;
    view(&controller, |v| v.connected).await;
    while hub.frames.try_recv().is_ok() {}
    fleet
        .send
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Library(4001),
            reason: "pause".into(),
        })))
        .await
        .unwrap();
    let paused = view(&controller, |v| v.power_paused).await;
    assert!(!paused.connected);
    assert!(paused.can_resume_power_pause);
    assert!(!paused.notice.contains("Reconnecting"));
    controller
        .command(Command::Request(wks_native::features::Request::Recent))
        .unwrap();
    view(&controller, |v| {
        v.requests.get("recent").is_some_and(|s| s.error.is_some())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        hub.frames.try_recv().is_err(),
        "maintenance or read request woke paused host"
    );
    controller.command(Command::Refresh).unwrap();
    let resumed = hub.frame("call", Some("sessions.snapshots")).await;
    resumed.result(json!([])).await;
    view(&controller, |v| v.connected && !v.power_paused).await;
    while hub.frames.try_recv().is_ok() {}
    resumed
        .send
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Library(4001),
            reason: "new stop episode".into(),
        })))
        .await
        .unwrap();
    let later = view(&controller, |v| {
        v.power_paused && v.power_pause_generation != paused.power_pause_generation
    })
    .await;
    assert!(later.power_pause_generation > paused.power_pause_generation);
    controller
        .command(Command::ResumePowerPause(paused.power_pause_generation))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(
        hub.frames.try_recv().is_err(),
        "a queued old GUI gesture woke a later stop episode"
    );
}

#[tokio::test]
async fn chat_file_links_read_on_the_session_hub_with_visible_errors() {
    use base64::Engine;
    use wks_native::{
        features::Request,
        links::{FileKind, Link, classify},
    };
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([session("a")]))
        .await;
    hub.frame("call", Some("sessions.conversation"))
        .await
        .result(snapshot(1, "See [main](src/main.rs:2)"))
        .await;
    view(&controller, |v| v.transcript.seq == Some(1)).await;
    let open = |raw: &str| {
        let Link::File(target) = classify("/test", raw) else {
            panic!("{raw} is a file link");
        };
        controller
            .command(Command::Request(Request::FilePreview {
                session: "a".into(),
                target,
            }))
            .unwrap();
    };
    let done = |number: u64| {
        move |v: &View| {
            v.requests
                .get("file-preview")
                .is_some_and(|r| !r.loading && r.number > number)
        }
    };

    // Source text: read by the hub (the session's machine), never locally.
    open("src/main.rs:2");
    let read = hub.frame("call", Some("fs.read")).await;
    assert_eq!(read.value["params"], json!({"path":"/test/src/main.rs"}));
    read.result(
        json!({"path":"/test/src/main.rs","contents":"fn main() {\n    start();\n}\n","size":28}),
    )
    .await;
    let v = view(&controller, done(0)).await;
    let state = &v.requests["file-preview"];
    assert!(state.error.is_none());
    assert_eq!(state.value["contents"], "fn main() {\n    start();\n}\n");
    let Request::FilePreview { target, .. } = &state.request else {
        unreachable!()
    };
    assert_eq!(
        (target.line, target.kind.clone()),
        (Some(2), FileKind::Text)
    );
    let last = state.number;

    // Missing files and hub limits come back as readable, visible errors.
    open("docs/gone.md");
    let read = hub.frame("call", Some("fs.read")).await;
    read.send
        .send(Message::Text(
            json!({"op":"error","id":read.value["id"],"error":"No such file or directory (os error 2)"})
                .to_string(),
        ))
        .await
        .unwrap();
    let v = view(&controller, done(last)).await;
    assert_eq!(
        v.requests["file-preview"].error.as_deref(),
        Some("No file at this path on the session's machine.")
    );
    let last = v.requests["file-preview"].number;

    open("big.log");
    hub.frame("call", Some("fs.read"))
        .await
        .result(json!({"contents":"x\n".repeat(60_000),"size":120_000}))
        .await;
    let v = view(&controller, done(last)).await;
    assert!(
        v.requests["file-preview"]
            .error
            .as_deref()
            .unwrap()
            .contains("lines")
    );
    let last = v.requests["file-preview"].number;

    // Images use the bounded image read and arrive as a PNG for the viewer.
    open("out/shot.png");
    let read = hub.frame("call", Some("fs.readImage")).await;
    assert_eq!(read.value["params"], json!({"path":"/test/out/shot.png"}));
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(40, 20)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    read.result(
        json!({"dataUrl":format!("data:image/png;base64,{data}"),"width":40,"height":20,"size":80}),
    )
    .await;
    let v = view(&controller, done(last)).await;
    let state = &v.requests["file-preview"];
    assert!(state.error.is_none(), "{:?}", state.error);
    assert_eq!(
        (
            state.value["width"].as_u64(),
            state.value["height"].as_u64()
        ),
        (Some(40), Some(20))
    );
    assert!(state.value["png"].as_str().is_some_and(|s| !s.is_empty()));
}

/// The hub's `config.save` contract as far as projects need it: a deep merge,
/// except that `projects` is replaced wholesale (hub-rs `merge_patch`).
fn save_config(state: &mut Value, patch: &Value) {
    fn merge(into: &mut Value, patch: &Value) {
        match (into.as_object_mut(), patch.as_object()) {
            (Some(into), Some(patch)) => {
                for (key, value) in patch {
                    merge(into.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
            _ => *into = patch.clone(),
        }
    }
    for (key, value) in patch.as_object().unwrap() {
        if key == "projects" {
            state[key] = value.clone();
        } else {
            merge(&mut state[key], value);
        }
    }
}

/// A pin (window A), a launch's recency touch (window A) and a legacy-only
/// removal (a second controller on the same hub) all accepted before the
/// hub answers anything. Without one serialized read→save→readback round,
/// every `config.get` reads the same map and each wholesale save erases the
/// others while all three report success.
#[tokio::test]
async fn overlapping_project_writes_queue_and_every_change_survives() {
    use wks_native::{features::Request, projects::Patch};
    let mut hub = Hub::shared().await;
    let a = Controller::start(hub.config.clone());
    let b = Controller::start(hub.config.clone());
    for _ in 0..2 {
        hub.frame("call", Some("sessions.snapshots"))
            .await
            .result(json!([]))
            .await;
    }
    view(&a, |v| v.connected).await;
    view(&b, |v| v.connected).await;

    let mut state = json!({
        "projects": {"/keep": {"label": "Retain", "color": "#336699"}},
        "directories": {"favourites": ["/legacy"], "recent": ["/legacy", "/old"]},
        "plugins": {"other": "untouched"}
    });
    a.command(Command::Request(Request::SaveProject {
        path: "/pin".into(),
        change: Patch::Pin(true),
    }))
    .unwrap();
    let first = hub.frame("call", Some("config.get")).await;
    a.command(Command::Request(Request::TouchProject {
        path: "/launch".into(),
        at: 100,
    }))
    .unwrap();
    b.command(Command::Request(Request::SaveProject {
        path: "/legacy".into(),
        change: Patch::Remove,
    }))
    .unwrap();
    // Give the unserialized ordering every chance to send its reads first.
    tokio::time::sleep(Duration::from_millis(200)).await;
    first.result(state.clone()).await;

    let mut log = vec!["get"];
    let mut saves = 0;
    while saves < 3 {
        let frame = timeout(DEADLINE, async {
            loop {
                let frame = hub.frame("call", None).await;
                let method = frame.value["method"].as_str().unwrap_or_default();
                if method.starts_with("config.") {
                    return frame;
                }
            }
        })
        .await
        .expect("project write round");
        match frame.value["method"].as_str().unwrap() {
            "config.get" => {
                log.push("get");
                frame.result(state.clone()).await;
            }
            "config.save" => {
                log.push("save");
                saves += 1;
                save_config(&mut state, &frame.value["params"]);
                frame.result(state.clone()).await;
            }
            other => panic!("unexpected {other}"),
        }
    }
    let done = |keys: &'static [&'static str]| {
        move |v: &View| {
            keys.iter()
                .all(|k| v.requests.get(k).is_some_and(|s| !s.loading))
        }
    };
    let a_view = view(&a, done(&["project-save", "project-touch"])).await;
    let b_view = view(&b, done(&["project-save"])).await;
    for (view, key) in [
        (&a_view, "project-save"),
        (&a_view, "project-touch"),
        (&b_view, "project-save"),
    ] {
        let receipt = &view.requests[key];
        assert!(receipt.error.is_none(), "{key}: {:?}", receipt.error);
    }
    assert!(matches!(
        a_view.requests["project-touch"].request,
        Request::TouchProject { .. }
    ));
    assert_eq!(state["projects"]["/pin"]["favourite"], true);
    assert_eq!(state["projects"]["/launch"]["lastOpened"], 100);
    assert_eq!(state["projects"]["/keep"]["label"], "Retain");
    assert_eq!(state["directories"]["favourites"], json!([]));
    assert_eq!(state["directories"]["recent"], json!(["/old"]));
    assert_eq!(state["plugins"]["other"], "untouched");
    assert_eq!(
        log,
        ["get", "save", "get", "save", "get", "save"],
        "each round reads only after the previous save answered"
    );
}

/// A save the hub skipped answers with the unchanged config. For a project
/// known only from the legacy arrays that config still lists it, so the
/// removal is reported as refused, not as "Removed".
#[tokio::test]
async fn skipped_legacy_only_removal_is_reported_as_refused() {
    use wks_native::{features::Request, projects::Patch};
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    let unchanged =
        json!({"projects": {}, "directories": {"favourites": ["/legacy"], "recent": ["/legacy"]}});
    controller
        .command(Command::Request(Request::SaveProject {
            path: "/legacy".into(),
            change: Patch::Remove,
        }))
        .unwrap();
    hub.frame("call", Some("config.get"))
        .await
        .result(unchanged.clone())
        .await;
    let save = hub.frame("call", Some("config.save")).await;
    assert_eq!(save.value["params"]["directories"]["favourites"], json!([]));
    save.result(unchanged).await;
    let done = view(&controller, |v| {
        v.requests.get("project-save").is_some_and(|s| !s.loading)
    })
    .await;
    let error = done.requests["project-save"]
        .error
        .clone()
        .unwrap_or_default();
    assert!(error.contains("did not save"), "{error}");
}

/// The newer-number refresh must not read pre-save data while a write is
/// pending. Exercise each visible mutation, including imported-key removal.
#[tokio::test]
async fn registry_refresh_waits_for_pin_unpin_and_remove_transactions() {
    use wks_native::{features::Request, projects::Patch};
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    let mut state =
        json!({"projects":{"/web/":{"lastOpened":10},"/keep":{"label":"Keep","workflowId":"wf"}}});
    let mut revision = 0;
    for change in [Patch::Pin(true), Patch::Pin(false), Patch::Remove] {
        controller
            .command(Command::Request(Request::SaveProject {
                path: "/web".into(),
                change: change.clone(),
            }))
            .unwrap();
        hub.frame("call", Some("config.get"))
            .await
            .result(state.clone())
            .await;
        let save = hub.frame("call", Some("config.save")).await;
        controller
            .command(Command::Request(Request::Projects))
            .unwrap();
        view(&controller, |v| {
            v.requests.get("projects").is_some_and(|s| s.loading)
        })
        .await;
        // Old code sends this read immediately, returning the pre-save map.
        assert!(
            timeout(
                Duration::from_millis(150),
                hub.frame("call", Some("config.get"))
            )
            .await
            .is_err(),
            "a newer request read stale config during an acknowledged write transaction"
        );
        save_config(&mut state, &save.value["params"]);
        save.result(state.clone()).await;
        hub.frame("call", Some("config.get"))
            .await
            .result(state.clone())
            .await;
        let done = view(&controller, |v| {
            ["projects", "project-save"]
                .iter()
                .all(|k| v.requests.get(k).is_some_and(|s| !s.loading))
        })
        .await;
        let written = &done.requests["project-save"];
        let refreshed = &done.requests["projects"];
        assert!(written.error.is_none() && refreshed.error.is_none());
        assert!(refreshed.number > written.number);
        let write_revision = written.value["revision"].as_u64().unwrap();
        let read_revision = refreshed.value["revision"].as_u64().unwrap();
        assert!(revision < write_revision && write_revision < read_revision);
        revision = read_revision;
        assert_eq!(
            done.project_registry.as_ref().unwrap()["projects"],
            state["projects"]
        );
        assert_eq!(state["projects"]["/keep"]["workflowId"], "wf");
        wks_native::projects::verify(&state, "/web", &change).unwrap();
    }
}

#[tokio::test]
async fn rapid_touches_keep_every_receipt_and_maximum_recency_across_writes() {
    use wks_native::{features::Request, projects::Patch};
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    hub.frame("call", Some("sessions.snapshots"))
        .await
        .result(json!([]))
        .await;
    view(&controller, |v| v.connected).await;
    let mut state =
        json!({"projects":{"/a/":{"label":"A","lastOpened":50},"/gone/":{"lastOpened":1}}});
    controller
        .command(Command::Request(Request::TouchProject {
            path: "/a".into(),
            at: 200,
        }))
        .unwrap();
    let first = hub.frame("call", Some("config.get")).await;
    for (path, at) in [("/b", 300), ("/a", 100), ("/b", 400), ("/refused", 500)] {
        controller
            .command(Command::Request(Request::TouchProject {
                path: path.into(),
                at,
            }))
            .unwrap();
    }
    controller
        .command(Command::Request(Request::SaveProject {
            path: "/b".into(),
            change: Patch::Pin(true),
        }))
        .unwrap();
    controller
        .command(Command::Request(Request::Projects))
        .unwrap();
    // Wait until the later request has been accepted, before the first reply.
    view(&controller, |v| {
        v.requests.get("projects").is_some_and(|s| s.loading)
    })
    .await;
    first.result(state.clone()).await;
    let mut saves = 0;
    let mut reads = 1;
    while saves < 6 || reads < 7 {
        let frame = hub.frame("call", None).await;
        match frame.value["method"].as_str().unwrap_or_default() {
            "config.get" => {
                reads += 1;
                frame.result(state.clone()).await;
            }
            "config.save" => {
                saves += 1;
                // One refusal must stay attached to /refused, not the next touch.
                if frame.value["params"]["projects"].get("/refused").is_none() {
                    save_config(&mut state, &frame.value["params"]);
                }
                frame.result(state.clone()).await;
            }
            _ => {}
        }
    }
    let done = view(&controller, |v| {
        v.project_touch_receipts.len() == 5
            && v.requests.get("projects").is_some_and(|s| !s.loading)
    })
    .await;
    let receipts: Vec<_> = done
        .project_touch_receipts
        .iter()
        .map(|r| (r.path.as_str(), r.at, r.error.is_some()))
        .collect();
    assert_eq!(
        receipts,
        [
            ("/a", 200, false),
            ("/b", 300, false),
            ("/a", 100, false),
            ("/b", 400, false),
            ("/refused", 500, true)
        ]
    );
    assert!(
        done.project_touch_receipts
            .iter()
            .map(|r| r.number)
            .collect::<Vec<_>>()
            .windows(2)
            .all(|w| w[0] < w[1])
    );
    assert_eq!(state["projects"]["/a/"]["lastOpened"], 200);
    assert_eq!(state["projects"]["/a/"]["label"], "A");
    assert_eq!(state["projects"]["/b"]["lastOpened"], 400);
    assert_eq!(state["projects"]["/b"]["favourite"], true);
    assert_eq!(
        done.project_registry.as_ref().unwrap()["projects"],
        state["projects"]
    );
    // Removal queued after a touch must remove its alias, without resurrecting
    // it from an older read. Neither operation starts or retries an agent.
    controller
        .command(Command::Request(Request::TouchProject {
            path: "/gone".into(),
            at: 600,
        }))
        .unwrap();
    let first = hub.frame("call", Some("config.get")).await;
    controller
        .command(Command::Request(Request::SaveProject {
            path: "/gone".into(),
            change: Patch::Remove,
        }))
        .unwrap();
    first.result(state.clone()).await;
    for i in 0..2 {
        if i == 1 {
            hub.frame("call", Some("config.get"))
                .await
                .result(state.clone())
                .await;
        }
        let save = hub.frame("call", Some("config.save")).await;
        save_config(&mut state, &save.value["params"]);
        save.result(state.clone()).await;
    }
    let done = view(&controller, |v| {
        v.project_touch_receipts.len() == 6
            && v.requests.get("project-save").is_some_and(|s| {
                !s.loading
                    && matches!(
                        s.request,
                        Request::SaveProject {
                            change: Patch::Remove,
                            ..
                        }
                    )
            })
    })
    .await;
    assert!(done.requests["project-save"].error.is_none());
    assert!(state["projects"].get("/gone/").is_none());
    assert_eq!(
        done.project_registry.as_ref().unwrap()["projects"],
        state["projects"]
    );
}

#[tokio::test]
async fn guarded_save_on_an_old_hub_never_falls_back_to_an_unconditional_write() {
    use wks_native::{backend::Backend, features::Request};
    let mut hub = Hub::new().await;
    let (backend, events) = Backend::connect(hub.config.clone());
    connected(&events).await;
    let save = tokio::spawn(async move {
        Request::SaveFile {
            session: "remote-session".into(),
            path: "/remote/project/source.rs".into(),
            contents: "mine".into(),
            base: "base".into(),
            force: false,
        }
        .run(&backend)
        .await
    });
    let frame = hub.frame("call", None).await;
    assert_eq!(frame.value["method"], "fs.compareWrite");
    assert_eq!(frame.value["params"]["path"], "/remote/project/source.rs");
    assert_eq!(frame.value["params"]["expected"], "base");
    frame
        .result(json!({"ok":false,"error":"unknown method fs.compareWrite"}))
        .await;
    assert!(timeout(DEADLINE, save).await.unwrap().unwrap().is_err());
    assert!(hub.frames.try_recv().is_err(), "no plain fs.write fallback");
}

/// Serve the fake hub until `done` says the expected calls arrived, answering
/// each call with `answer`. Returns every frame seen, for "never sent" checks.
async fn serve_until(
    hub: &mut Hub,
    mut answer: impl FnMut(&Value) -> Option<Value>,
    mut done: impl FnMut(&[Value]) -> bool,
) -> (Vec<Value>, mpsc::Sender<Message>) {
    let mut seen = Vec::new();
    let mut sender = None;
    timeout(DEADLINE, async {
        loop {
            let frame = hub.frames.recv().await.unwrap();
            if frame.send.is_closed() {
                continue;
            }
            sender = Some(frame.send.clone());
            if frame.value["op"] == "call"
                && let Some(result) = answer(&frame.value)
            {
                frame.result(result).await;
            }
            seen.push(frame.value);
            if done(&seen) {
                return;
            }
        }
    })
    .await
    .expect("expected hub frames");
    (seen, sender.unwrap())
}

fn called(seen: &[Value], method: &str) -> bool {
    seen.iter()
        .any(|f| f["op"] == "call" && f["method"] == method)
}

#[tokio::test]
async fn shared_archive_is_read_on_connect_followed_live_and_changed_only_by_its_own_call() {
    let mut hub = Hub::new().await;
    let controller = Controller::start(hub.config.clone());
    let fleet = || Some(json!([session("a"), session("b")]));
    let (mut seen, sender) = serve_until(
        &mut hub,
        |f| match f["method"].as_str().unwrap_or("") {
            "sessions.snapshots" => fleet(),
            "sessionArchive.get" => Some(json!({"version":3,"archived":{"a":1}})),
            _ => Some(json!({})),
        },
        |seen| {
            called(seen, "sessionArchive.get")
                && seen.iter().any(|f| {
                    f["op"] == "subscribe"
                        && f["topics"]
                            .as_array()
                            .is_some_and(|t| t.iter().any(|t| t == "sessionArchive.changed"))
                })
        },
    )
    .await;
    let read = view(&controller, |v| v.session_archive.is_some()).await;
    assert_eq!(read.session_archive.as_ref().unwrap()["version"], 3);

    // Another client (the web) archives b: the change arrives as an event.
    sender
        .send(event(
            "sessionArchive.changed",
            json!({"version":4,"archived":{"a":1,"b":2}}),
        ))
        .await
        .unwrap();
    view(&controller, |v| {
        v.session_archive
            .as_ref()
            .is_some_and(|d| d["version"] == 4)
    })
    .await;
    // A late, older document never undoes a newer one.
    sender
        .send(event(
            "sessionArchive.changed",
            json!({"version":2,"archived":{}}),
        ))
        .await
        .unwrap();
    sender
        .send(event("agent.snapshot", session("c")))
        .await
        .unwrap();
    let after = view(&controller, |v| v.sessions.iter().any(|s| s.id == "c")).await;
    assert_eq!(after.session_archive.as_ref().unwrap()["version"], 4);

    // Restoring b is exactly one sessionArchive.set — and nothing else.
    controller
        .command(Command::Request(
            wks_native::features::Request::SetArchive {
                session: "b".into(),
                archived: false,
            },
        ))
        .unwrap();
    let (more, _) = serve_until(
        &mut hub,
        |f| match f["method"].as_str().unwrap_or("") {
            "sessionArchive.set" => {
                assert_eq!(f["params"], json!({"sessionId":"b","archived":false}));
                Some(json!({"version":5,"archived":{"a":1}}))
            }
            _ => Some(json!({})),
        },
        |seen| called(seen, "sessionArchive.set"),
    )
    .await;
    seen.extend(more);
    let restored = view(&controller, |v| {
        v.session_archive
            .as_ref()
            .is_some_and(|d| d["version"] == 5)
            && !v.archive_receipts.is_empty()
    })
    .await;
    let receipt = restored.archive_receipts.back().unwrap();
    assert_eq!((receipt.session.as_str(), receipt.archived), ("b", false));
    assert!(receipt.error.is_none());

    // A reply that does not hold the change is a failure, never a success.
    controller
        .command(Command::Request(
            wks_native::features::Request::SetArchive {
                session: "c".into(),
                archived: true,
            },
        ))
        .unwrap();
    let (more, _) = serve_until(
        &mut hub,
        |f| match f["method"].as_str().unwrap_or("") {
            "sessionArchive.set" => Some(json!({"version":6,"archived":{"a":1}})),
            _ => Some(json!({})),
        },
        |seen| called(seen, "sessionArchive.set"),
    )
    .await;
    seen.extend(more);
    let refused = view(&controller, |v| v.archive_receipts.len() == 2).await;
    assert!(
        refused.archive_receipts[1]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("did not save"))
    );

    for method in [
        "claude.signal",
        "claude.gate",
        "agents.close",
        "sessions.delete",
    ] {
        assert!(!called(&seen, method), "archive sent {method}");
    }
}

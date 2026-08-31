#![allow(dead_code)] // wired into call sites incrementally as the TUI moves onto the bus

//! Hub-bus client for the TUI — a reconnecting WebSocket caller + event
//! subscriber, the Rust counterpart of the Go `internal/busclient`. It speaks
//! the hub protocol over `ws://<addr>/bus`:
//!
//!   call:      {"op":"call","id":..,"method":..,"params":..} → {"op":"result"|"error",..}
//!   subscribe: {"op":"subscribe","topics":[..]}              → {"op":"event","event":{type,data}}
//!
//! This is the seam that lets the TUI stop talking to claudemon directly and
//! mirror the desktop app: capability calls (spawn/message/approve/…) become
//! `call`s, and live updates (`agent.snapshot`, `pty.bytes.<id>`,
//! `agent.statusline`) arrive as events. Subscriptions are remembered and
//! re-sent on reconnect.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Result};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// A delivered bus event.
#[derive(Clone, Debug)]
pub struct BusEvent {
    pub topic: String,
    pub data: Value,
    /// Which peer hub the event came from, stamped on the ENVELOPE by the local
    /// hub's federation link (`hub: "<peerName>"`). `None` means local — the
    /// payload itself is never rewritten, so discriminating on this field is
    /// the only way to tell a remote `agent.snapshot` from a local one.
    pub hub: Option<String>,
}

/// Synthetic topic delivered once per successful (re)connect, before any real
/// frames. Not a hub topic — underscore-prefixed so it can never collide with
/// one. The app uses it to (re)seed call-derived state (the federated fleet):
/// events alone can't fix restart-blindness, and the bus client is otherwise
/// silent about its own connection state.
pub const TOPIC_BUS_CONNECTED: &str = "_bus.connected";

/// Synthetic topic carrying the hub's `hello` greeting — the TIER this
/// connection authenticated as (`scope`, plus the method allowlist a scoped
/// token holds). Not a real bus topic either; the hub sends `hello` once the
/// token resolves, and republishing it here is what lets the app gate a
/// control it would otherwise offer and have refused. `nodes.wake` is
/// host-authority only, so the remote-node surface needs this to know whether
/// to offer a wake at all — see [`crate::nodes::wake_affordance`].
pub const TOPIC_BUS_HELLO: &str = "_bus.hello";

enum Command {
    Call {
        method: String,
        params: Value,
        reply: oneshot::Sender<Result<Value>>,
    },
    Subscribe(Vec<String>),
}

/// A cloneable handle to the bus connection. Calls and subscriptions are
/// forwarded to the background task that owns the socket.
#[derive(Clone)]
pub struct BusClient {
    cmd_tx: mpsc::UnboundedSender<Command>,
}

impl BusClient {
    /// Connect (reconnecting in the background) and return the handle plus a
    /// receiver of every delivered event.
    pub fn connect(
        url: String,
        token: Option<String>,
    ) -> (BusClient, mpsc::UnboundedReceiver<BusEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        tokio::spawn(run(url, token, cmd_rx, event_tx));
        (BusClient { cmd_tx }, event_rx)
    }

    /// Invoke a capability and await its result.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Call {
                method: method.to_string(),
                params,
                reply,
            })
            .map_err(|_| anyhow!("bus client closed"))?;
        rx.await.map_err(|_| anyhow!("bus call dropped"))?
    }

    /// Subscribe to topics (exact, `ns.*`, or `*`). Remembered across reconnects.
    pub fn subscribe(&self, topics: Vec<String>) -> Result<()> {
        self.cmd_tx
            .send(Command::Subscribe(topics))
            .map_err(|_| anyhow!("bus client closed"))
    }
}

fn dial_url(url: &str, token: &Option<String>) -> String {
    match token {
        Some(t) if !t.is_empty() => {
            if url.contains('?') {
                format!("{url}&token={t}")
            } else {
                format!("{url}?token={t}")
            }
        }
        _ => url.to_string(),
    }
}

async fn run(
    url: String,
    token: Option<String>,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    event_tx: mpsc::UnboundedSender<BusEvent>,
) {
    let dial = dial_url(&url, &token);
    let mut topics: Vec<String> = Vec::new();
    let mut counter: u64 = 1;
    let mut backoff = Duration::from_secs(1);

    loop {
        let conn = connect_async(dial.as_str()).await;
        let ws = match conn {
            Ok((ws, _)) => ws,
            Err(_) => {
                tokio::time::sleep(backoff).await;
                if backoff < Duration::from_secs(10) {
                    backoff *= 2;
                }
                continue;
            }
        };
        backoff = Duration::from_secs(1);
        let (mut write, mut read) = ws.split();

        // Tell the app the socket is (back) up, so call-seeded state (the
        // federated fleet) can re-seed. Sent before any subscription so the
        // seed always precedes the events it will be reconciled against.
        let _ = event_tx.send(BusEvent {
            topic: TOPIC_BUS_CONNECTED.to_string(),
            data: Value::Null,
            hub: None,
        });

        // Re-prime subscriptions on every (re)connect.
        if !topics.is_empty() {
            let frame = json!({ "op": "subscribe", "topics": topics });
            let _ = write.send(Message::Text(frame.to_string())).await;
        }

        let mut pending: HashMap<String, oneshot::Sender<Result<Value>>> = HashMap::new();
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    None => return, // handle dropped → shut down
                    Some(Command::Subscribe(t)) => {
                        let frame = json!({ "op": "subscribe", "topics": t });
                        let _ = write.send(Message::Text(frame.to_string())).await;
                        topics.extend(t);
                    }
                    Some(Command::Call { method, params, reply }) => {
                        let id = format!("c{counter}");
                        counter += 1;
                        let frame = json!({ "op": "call", "id": id, "method": method, "params": params });
                        if write.send(Message::Text(frame.to_string())).await.is_err() {
                            let _ = reply.send(Err(anyhow!("bus write failed")));
                        } else {
                            pending.insert(id, reply);
                        }
                    }
                },
                // Any disconnect (None / error / close) drops out to reconnect.
                msg = read.next() => match msg {
                    Some(Ok(m)) if !m.is_close() => {
                        if let Message::Text(txt) = m {
                            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                                handle_frame(v, &mut pending, &event_tx);
                            }
                        }
                    }
                    _ => break,
                },
            }
        }

        // Disconnected — fail any in-flight calls so callers don't hang, then
        // loop back to reconnect.
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(anyhow!("bus disconnected")));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn handle_frame(
    v: Value,
    pending: &mut HashMap<String, oneshot::Sender<Result<Value>>>,
    event_tx: &mpsc::UnboundedSender<BusEvent>,
) {
    match v.get("op").and_then(|o| o.as_str()) {
        Some("result") => {
            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                if let Some(tx) = pending.remove(id) {
                    let _ = tx.send(Ok(v.get("result").cloned().unwrap_or(Value::Null)));
                }
            }
        }
        Some("error") => {
            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                if let Some(tx) = pending.remove(id) {
                    let err = v
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("bus error");
                    let _ = tx.send(Err(anyhow!(err.to_string())));
                }
            }
        }
        Some("event") => {
            if let Some(ev) = v.get("event") {
                let topic = ev
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let data = ev.get("data").cloned().unwrap_or(Value::Null);
                // The federation stamp lives on the envelope, not the payload;
                // absent or empty means the event is local.
                let hub = ev
                    .get("hub")
                    .and_then(|h| h.as_str())
                    .filter(|h| !h.is_empty())
                    .map(String::from);
                let _ = event_tx.send(BusEvent { topic, data, hub });
            }
        }
        // The greeting names the tier this token holds. A trusted (host or
        // operator) conn reports "operator"; a scoped one reports its own tier;
        // a plugin token reports none at all. Absent therefore means "not
        // operator", which is the safe reading for anything that spends money.
        Some("hello") => {
            let _ = event_tx.send(BusEvent {
                topic: TOPIC_BUS_HELLO.to_string(),
                data: v.clone(),
                hub: None,
            });
        }
        _ => {} // subscribed / unsubscribed acks
    }
}

/// Routes agent-driving capability calls through the bus when connected, else
/// straight to claudemon. The verbs here map 1:1 to brain capabilities with
/// simple `{sessionId, …}` params; spawn (argv) and raw PTY input still go
/// claudemon-direct (they need argv/bytes handling the bus path doesn't cover
/// yet). Cheap to clone — both fields are handles.
#[derive(Clone)]
pub struct Driver {
    pub claudemon: crate::claudemon::Claudemon,
    pub bus: Option<BusClient>,
    /// Transport for Claude sessions this TUI spawns (config `transport`,
    /// default stream). Stated on BOTH paths on purpose: the bus path used to
    /// send no transport at all, so the hub filled it in from the desktop's
    /// `claude.transport` — meaning the same TUI spawned a stream session when
    /// the desktop happened to own the hub and a PTY one when it didn't. Nobody
    /// chose that; it was a config default leaking across a seam.
    pub transport: crate::config::Transport,
    /// When set, the session this driver targets lives on a peer hub of that
    /// name: capability calls go out hub-qualified (`hub:<peer>/<method>`) and
    /// the claudemon fallback is refused — there is no local daemon that knows
    /// the session. `None` is the local path, unchanged.
    pub hub: Option<String>,
}

impl Driver {
    /// Qualify a capability name for the driver's hub, if any. The hub routes
    /// `hub:<peer>/<method>` over its federation link to that peer.
    fn method(&self, name: &str) -> String {
        match &self.hub {
            Some(peer) => format!("hub:{peer}/{name}"),
            None => name.to_string(),
        }
    }

    pub async fn message(&self, sid: &str, text: &str) -> Result<()> {
        match (&self.bus, &self.hub) {
            (Some(b), _) => b
                .call(
                    &self.method("agents.sendMessage"),
                    json!({ "sessionId": sid, "text": text }),
                )
                .await
                .map(|_| ()),
            (None, Some(_)) => Err(anyhow!("remote session but no hub bus connection")),
            (None, None) => self.claudemon.message(sid, text).await,
        }
    }

    pub async fn answer_text(&self, sid: &str, text: &str) -> Result<()> {
        // Remote: `claude.answer` is not reliably available across federation
        // (the peer's provider set varies) — the chosen text goes down the
        // ordinary message path instead, which every peer provides.
        if self.hub.is_some() {
            return self.message(sid, text).await;
        }
        match &self.bus {
            Some(b) => b
                .call("claude.answer", json!({ "sessionId": sid, "text": text }))
                .await
                .map(|_| ()),
            None => self.claudemon.answer_text(sid, text).await,
        }
    }

    pub async fn answer_all(&self, sid: &str, answers: Vec<String>) -> Result<()> {
        // Remote: no `claude.answer` — the collected answers travel as one
        // message, newline-joined in question order (the caller has already
        // resolved digit picks to their labels; see `remote_answers`).
        if self.hub.is_some() {
            return self.message(sid, &answers.join("\n")).await;
        }
        match &self.bus {
            Some(b) => b
                .call(
                    "claude.answer",
                    json!({ "sessionId": sid, "answers": answers }),
                )
                .await
                .map(|_| ()),
            None => self.claudemon.answer_all(sid, &answers).await,
        }
    }

    pub async fn answer_option(&self, sid: &str, option: u64) -> Result<()> {
        // Remote pick-by-number needs the option's TEXT (there is no
        // `claude.answer` across federation); the App resolves the label and
        // routes through `answer_text`, so reaching here remotely is a bug.
        if self.hub.is_some() {
            return Err(anyhow!(
                "remote session — answer with the option text, not a number"
            ));
        }
        match &self.bus {
            Some(b) => b
                .call(
                    "claude.answer",
                    json!({ "sessionId": sid, "option": option }),
                )
                .await
                .map(|_| ()),
            None => self.claudemon.answer_option(sid, option).await,
        }
    }

    pub async fn approve(&self, sid: &str, decision: &str, reason: Option<String>) -> Result<()> {
        match (&self.bus, &self.hub) {
            (Some(b), _) => {
                let mut params = json!({ "sessionId": sid, "decision": decision });
                if let Some(r) = reason {
                    params["reason"] = json!(r);
                }
                b.call(&self.method("claude.approve"), params)
                    .await
                    .map(|_| ())
            }
            (None, Some(_)) => Err(anyhow!("remote session but no hub bus connection")),
            (None, None) => self.claudemon.approve(sid, decision, reason).await,
        }
    }

    pub async fn signal(&self, sid: &str, signal: &str) -> Result<()> {
        match (&self.bus, &self.hub) {
            (Some(b), _) => b
                .call(
                    &self.method("claude.signal"),
                    json!({ "sessionId": sid, "signal": signal }),
                )
                .await
                .map(|_| ()),
            (None, Some(_)) => Err(anyhow!("remote session but no hub bus connection")),
            (None, None) => self.claudemon.signal(sid, signal).await,
        }
    }

    /// Spawn a fresh (or resumed) Claude agent and return its session id.
    ///
    /// Both paths now *state* the transport rather than letting anything infer
    /// it (see the field's note). On stream, claudemon builds the headless argv
    /// from typed fields, so the profile is decomposed into model/yolo/env/
    /// extra_args; on PTY the TUI hands over a full argv as it always has. Over
    /// the bus, the brain does whichever of those the transport implies.
    pub async fn spawn(
        &self,
        cwd: String,
        profile: &crate::profiles::Profile,
        resume_session_id: Option<String>,
    ) -> Result<String> {
        let stream = self.transport == crate::config::Transport::Stream;
        let profile_model = crate::profiles::profile_model(profile);
        match &self.bus {
            Some(b) => {
                let mut params = json!({
                    "cwd": cwd,
                    "profileId": profile.id,
                    "transport": self.transport.as_str(),
                });
                if let Some(rid) = &resume_session_id {
                    params["resumeSessionId"] = json!(rid);
                }
                if let Some(model) = profile_model {
                    crate::claudemon::add_model_wire_fields(&mut params, "claude", model);
                    if let Some(fields) = params.as_object_mut() {
                        if let Some(identity) = fields.remove("model_identity") {
                            fields.insert("modelIdentity".into(), identity);
                        }
                        if let Some(window) = fields.remove("context_window") {
                            fields.insert("contextWindow".into(), window);
                        }
                    }
                }
                let res = b.call("agents.spawn", params).await?;
                res.get("sessionId")
                    .and_then(|s| s.as_str())
                    .map(String::from)
                    .ok_or_else(|| anyhow!("agents.spawn returned no sessionId"))
            }
            None if stream => {
                let session_id = resume_session_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                self.claudemon
                    .spawn_claude_stream(
                        &cwd,
                        profile_model,
                        crate::profiles::profile_skips_permissions(profile),
                        &session_id,
                        resume_session_id.as_deref(),
                        &crate::profiles::stream_extra_args(profile),
                        &crate::profiles::build_env(profile),
                    )
                    .await
            }
            None => {
                let resume = resume_session_id.is_some();
                let session_id =
                    resume_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let argv = crate::profiles::build_argv(profile, None, false, &session_id, resume);
                let env = crate::profiles::build_env(profile);
                self.claudemon
                    .spawn(argv, cwd, env, &session_id, profile_model)
                    .await
            }
        }
    }

    /// Write raw keystroke bytes into a session's PTY. On the bus they go as
    /// base64 (sessions.terminalInput); claudemon-direct uses the byte endpoint.
    pub async fn terminal_input(&self, sid: &str, bytes: &[u8]) -> Result<()> {
        match &self.bus {
            Some(b) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                b.call(
                    "sessions.terminalInput",
                    json!({ "sessionId": sid, "bytesB64": b64 }),
                )
                .await
                .map(|_| ())
            }
            None => self.claudemon.input_bytes(sid, bytes).await,
        }
    }

    /// Resize a session's PTY to the pane grid.
    pub async fn resize(&self, sid: &str, cols: u16, rows: u16) -> Result<()> {
        match &self.bus {
            Some(b) => b
                .call(
                    "sessions.terminalResize",
                    json!({ "sessionId": sid, "cols": cols, "rows": rows }),
                )
                .await
                .map(|_| ()),
            None => self.claudemon.resize(sid, cols, rows).await,
        }
    }

    /// Live model/effort switch. On the bus this is `claude.setModel`, qualified
    /// for remote sessions; REST otherwise. The canonical pair is derived by the
    /// same Rust contract as spawn while the marker stays in `model` only.
    pub async fn set_model(
        &self,
        sid: &str,
        provider: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<crate::claudemon::ModelSwitchOutcome> {
        match &self.bus {
            Some(b) => {
                let mut params = json!({ "sessionId": sid });
                if let Some(m) = model {
                    let mut wire = json!({});
                    crate::claudemon::add_model_wire_fields(&mut wire, provider, m);
                    params["model"] = wire["model"].clone();
                    params["modelIdentity"] = wire["model_identity"].clone();
                    if let Some(window) = wire.get("context_window") {
                        params["contextWindow"] = window.clone();
                    }
                }
                if let Some(e) = effort {
                    params["effort"] = json!(e);
                }
                let value = check_ok(b.call(&self.method("claude.setModel"), params).await?)?;
                Ok(crate::claudemon::ModelSwitchOutcome::from_wire(&value))
            }
            None if self.hub.is_some() => Err(anyhow!("remote session but no hub bus connection")),
            None => self.claudemon.set_model(sid, provider, model, effort).await,
        }
    }

    /// Live permission-mode switch; returns the mode the daemon settled on. Bus
    /// path is `claude.setPermissionMode`, REST otherwise.
    pub async fn set_permission_mode(&self, sid: &str, mode: &str) -> Result<String> {
        match &self.bus {
            Some(b) => {
                let v = check_ok(
                    b.call(
                        "claude.setPermissionMode",
                        json!({ "sessionId": sid, "mode": mode }),
                    )
                    .await?,
                )?;
                Ok(v.get("mode")
                    .and_then(|m| m.as_str())
                    .unwrap_or(mode)
                    .to_string())
            }
            None => self.claudemon.set_permission_mode(sid, mode).await,
        }
    }

    /// Build a cross-provider handoff brief. Bus path is `claude.handoffBrief`,
    /// REST otherwise. Returns the markdown + persisted path.
    pub async fn handoff(&self, sid: &str) -> Result<crate::claudemon::HandoffBrief> {
        match &self.bus {
            Some(b) => {
                let v = check_ok(
                    b.call("claude.handoffBrief", json!({ "sessionId": sid }))
                        .await?,
                )?;
                Ok(crate::claudemon::HandoffBrief {
                    markdown: v
                        .get("markdown")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string(),
                    path: v.get("path").and_then(|p| p.as_str()).map(String::from),
                })
            }
            None => self.claudemon.handoff(sid).await,
        }
    }

    /// Spawn a managed (Codex/OpenCode/Pi) session and return its id. On the bus
    /// this rides `agents.spawn` with a `provider` param (which routes to the
    /// managed spawn path); note the bus path forces approvals *on* — a remote
    /// caller can't auto-bypass, so `yolo` is honoured only over REST.
    pub async fn spawn_managed(
        &self,
        provider: &str,
        cwd: &str,
        model: Option<&str>,
        effort: Option<&str>,
        yolo: bool,
    ) -> Result<String> {
        match &self.bus {
            Some(b) => {
                let mut params = json!({ "provider": provider, "cwd": cwd });
                if let Some(m) = model {
                    crate::claudemon::add_model_wire_fields(&mut params, provider, m);
                    if let Some(fields) = params.as_object_mut() {
                        if let Some(identity) = fields.remove("model_identity") {
                            fields.insert("modelIdentity".into(), identity);
                        }
                        if let Some(window) = fields.remove("context_window") {
                            fields.insert("contextWindow".into(), window);
                        }
                    }
                }
                if let Some(e) = effort {
                    params["effort"] = json!(e);
                }
                let res = check_ok(b.call("agents.spawn", params).await?)?;
                res.get("sessionId")
                    .and_then(|s| s.as_str())
                    .map(String::from)
                    .ok_or_else(|| anyhow!("agents.spawn returned no sessionId"))
            }
            None => {
                self.claudemon
                    .spawn_managed(provider, cwd, model, effort, yolo, "")
                    .await
            }
        }
    }
}

/// Fold a bus `call` result into a `Result`: a capability that returns
/// `{ ok:false, error }` (rather than raising a protocol error) becomes an `Err`
/// carrying the message, so a provider capability cliff reads the same over the
/// bus as over REST.
fn check_ok(v: Value) -> Result<Value> {
    if v.get("ok").and_then(|b| b.as_bool()) == Some(false) {
        let err = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("capability failed");
        return Err(anyhow!("{err}"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    // A minimal fake hub: answers a `call` with a result, and replies to a
    // `subscribe` by pushing one event on the subscribed topic.
    async fn fake_hub(listener: TcpListener) {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut write, mut read) = ws.split();
        let _ = write
            .send(Message::Text(json!({ "op": "hello" }).to_string()))
            .await;
        while let Some(Ok(msg)) = read.next().await {
            if let Message::Text(txt) = msg {
                let v: Value = serde_json::from_str(&txt).unwrap();
                match v.get("op").and_then(|o| o.as_str()) {
                    Some("call") => {
                        let id = v.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let reply = json!({ "op": "result", "id": id, "result": { "ok": true, "echo": v.get("params") } });
                        let _ = write.send(Message::Text(reply.to_string())).await;
                    }
                    Some("subscribe") => {
                        let ev = json!({ "op": "event", "event": { "type": "agent.snapshot", "data": { "session_id": "s1" } } });
                        let _ = write.send(Message::Text(ev.to_string())).await;
                    }
                    _ => {}
                }
            }
        }
    }

    #[tokio::test]
    async fn call_and_event_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(fake_hub(listener));

        let (client, mut events) = BusClient::connect(format!("ws://{addr}/bus"), None);

        let res = client
            .call("agents.list", json!({ "x": 1 }))
            .await
            .expect("call should succeed");
        assert_eq!(res["ok"], json!(true));
        assert_eq!(res["echo"]["x"], json!(1));

        client
            .subscribe(vec!["agent.snapshot".to_string()])
            .unwrap();
        // The synthetic connect notification always precedes real events.
        let ev = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("event within 3s")
            .expect("event channel open");
        assert_eq!(ev.topic, TOPIC_BUS_CONNECTED);
        // …followed by the hub's greeting, republished so the app can read the
        // tier it authenticated as before offering anything that spends money.
        let ev = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("event within 3s")
            .expect("event channel open");
        assert_eq!(ev.topic, TOPIC_BUS_HELLO);
        let ev = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("event within 3s")
            .expect("event channel open");
        assert_eq!(ev.topic, "agent.snapshot");
        assert_eq!(ev.data["session_id"], json!("s1"));
        assert_eq!(ev.hub, None, "no envelope stamp means local");
    }

    /// The greeting names the tier this token holds, and the app gates the
    /// remote-node wake on it (`nodes.wake` is host-authority only). A hello
    /// frame that arrived and was dropped would leave a scoped client offering
    /// a control the hub refuses — so the scope must survive the republish
    /// verbatim.
    #[tokio::test]
    async fn the_hello_greeting_is_republished_with_its_tier() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut write, _read) = ws.split();
            let _ = write
                .send(Message::Text(
                    json!({ "op": "hello", "scope": "triage", "methods": ["agents.list"] })
                        .to_string(),
                ))
                .await;
            // Hold the socket open so the client doesn't reconnect mid-assert.
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let (_client, mut events) = BusClient::connect(format!("ws://{addr}/bus"), None);
        assert_eq!(next_event(&mut events).await.topic, TOPIC_BUS_CONNECTED);
        let ev = next_event(&mut events).await;
        assert_eq!(ev.topic, TOPIC_BUS_HELLO);
        assert_eq!(ev.data["scope"], json!("triage"));
        assert_eq!(ev.hub, None, "the greeting is never a peer's");
    }

    // A fake hub that answers every call with `result` and records (method,
    // params) for the test to inspect.
    fn recording_hub(
        listener: TcpListener,
        result: Value,
    ) -> mpsc::UnboundedReceiver<(String, Value)> {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut write, mut read) = ws.split();
            while let Some(Ok(Message::Text(txt))) = read.next().await {
                let v: Value = serde_json::from_str(&txt).unwrap();
                if v.get("op").and_then(|o| o.as_str()) == Some("call") {
                    let id = v
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string();
                    let reply = json!({ "op": "result", "id": id, "result": result.clone() });
                    let _ = write.send(Message::Text(reply.to_string())).await;
                    let method = v
                        .get("method")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    let _ = tx.send((method, v.get("params").cloned().unwrap_or(Value::Null)));
                }
            }
        });
        rx
    }

    fn bus_driver(addr: std::net::SocketAddr) -> Driver {
        let (client, _events) = BusClient::connect(format!("ws://{addr}/bus"), None);
        Driver {
            transport: crate::config::Transport::default(),
            claudemon: crate::claudemon::Claudemon::new("http://unused".into()),
            bus: Some(client),
            hub: None,
        }
    }

    /// A driver whose session lives on peer hub `work`.
    fn remote_driver(addr: std::net::SocketAddr) -> Driver {
        let mut drv = bus_driver(addr);
        drv.hub = Some("work".to_string());
        drv
    }

    #[tokio::test]
    async fn driver_routes_message_to_capability() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({}));

        bus_driver(addr)
            .message("s1", "hello")
            .await
            .expect("message ok");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "agents.sendMessage");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["text"], json!("hello"));
    }

    #[tokio::test]
    async fn driver_routes_approval_and_answers_to_capabilities() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "ok": true }));
        let driver = bus_driver(addr);

        driver
            .approve("s1", "yes", Some("reviewed".into()))
            .await
            .expect("approve ok");
        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "claude.approve");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["decision"], json!("yes"));
        assert_eq!(params["reason"], json!("reviewed"));

        driver.answer_option("s1", 2).await.expect("answer ok");
        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "claude.answer");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["option"], json!(2));

        driver
            .answer_all("s1", vec!["2".into(), "free text".into()])
            .await
            .expect("multi-answer ok");
        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "claude.answer");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["answers"], json!(["2", "free text"]));
    }

    #[tokio::test]
    async fn driver_spawn_sends_profile_id_and_returns_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "sessionId": "spawned-1" }));

        let profile = crate::profiles::Profile {
            id: "work".into(),
            name: "Work".into(),
            config_dir: String::new(),
            extra_args: vec!["--model".into(), "opus[1m]".into()],
            is_default: false,
        };
        let sid = bus_driver(addr)
            .spawn("/tmp/proj".into(), &profile, None)
            .await
            .expect("spawn ok");
        assert_eq!(sid, "spawned-1");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "agents.spawn");
        assert_eq!(params["cwd"], json!("/tmp/proj"));
        assert_eq!(params["profileId"], json!("work"));
        assert_eq!(params["model"], json!("opus[1m]"));
        assert_eq!(params["modelIdentity"], json!("opus"));
        assert_eq!(params["contextWindow"], json!(1_000_000_u64));
    }

    #[tokio::test]
    async fn driver_routes_set_model_to_capability() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "ok": true, "model": "gpt-5" }));

        bus_driver(addr)
            .set_model("s1", "codex", Some("gpt-5"), Some("high"))
            .await
            .expect("set_model ok");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "claude.setModel");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["model"], json!("gpt-5"));
        assert_eq!(params["modelIdentity"], json!("gpt-5"));
        assert_eq!(params["effort"], json!("high"));
    }

    #[tokio::test]
    async fn remote_claude_model_switch_is_qualified_and_pair_aware() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(
            listener,
            json!({
                "ok": true,
                "queued": true,
                "disposition": "queued",
                "model": "opus[1m]",
                "requestedSelection": {"model": "opus", "contextWindow": 1_000_000_u64},
            }),
        );
        let mut driver = bus_driver(addr);
        driver.hub = Some("work".into());

        let outcome = driver
            .set_model("s1", "claude", Some("opus[1m]"), None)
            .await
            .expect("remote set_model ok");
        assert!(outcome.queued);
        assert_eq!(outcome.disposition, "queued");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "hub:work/claude.setModel");
        assert_eq!(params["model"], json!("opus[1m]"));
        assert_eq!(params["modelIdentity"], json!("opus"));
        assert_eq!(params["contextWindow"], json!(1_000_000_u64));
    }

    #[tokio::test]
    async fn driver_set_model_surfaces_capability_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _rx = recording_hub(listener, json!({ "ok": false, "error": "no model switch" }));

        let err = bus_driver(addr)
            .set_model("s1", "codex", Some("x"), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no model switch"), "got {err}");
    }

    #[tokio::test]
    async fn driver_set_permission_mode_returns_mode() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "ok": true, "mode": "plan" }));

        let mode = bus_driver(addr)
            .set_permission_mode("s1", "plan")
            .await
            .expect("permission mode ok");
        assert_eq!(mode, "plan");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "claude.setPermissionMode");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["mode"], json!("plan"));
    }

    /// The bus spawn used to send no transport, so the hub filled it in from the
    /// desktop's `claude.transport`: the same TUI got a stream session when the
    /// desktop owned the hub and a PTY one when it didn't. Stating it makes the
    /// TUI's spawns independent of who else is running.
    #[tokio::test]
    async fn driver_bus_spawn_states_its_transport() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "sessionId": "s-1" }));

        let profile = crate::profiles::Profile::default_profile();
        let sid = bus_driver(addr)
            .spawn("/w".into(), &profile, None)
            .await
            .expect("spawn ok");
        assert_eq!(sid, "s-1");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "agents.spawn");
        assert_eq!(params["transport"], json!("stream"));
        assert_eq!(params["cwd"], json!("/w"));
    }

    #[tokio::test]
    async fn driver_bus_spawn_honours_a_pty_config() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "sessionId": "s-2" }));

        let mut driver = bus_driver(addr);
        driver.transport = crate::config::Transport::Pty;
        let profile = crate::profiles::Profile::default_profile();
        driver.spawn("/w".into(), &profile, None).await.expect("ok");

        let (_, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(params["transport"], json!("pty"));
    }

    #[tokio::test]
    async fn driver_handoff_routes_and_parses_brief() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(
            listener,
            json!({ "ok": true, "markdown": "# b", "path": "/h/x.md" }),
        );

        let brief = bus_driver(addr).handoff("s1").await.expect("handoff ok");
        assert_eq!(brief.path.as_deref(), Some("/h/x.md"));

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "claude.handoffBrief");
        assert_eq!(params["sessionId"], json!("s1"));
    }

    #[tokio::test]
    async fn driver_spawn_managed_uses_agents_spawn_with_provider() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "sessionId": "m1" }));

        let sid = bus_driver(addr)
            .spawn_managed("codex", "/w", Some("gpt-5"), None, true)
            .await
            .expect("spawn_managed ok");
        assert_eq!(sid, "m1");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "agents.spawn");
        assert_eq!(params["provider"], json!("codex"));
        assert_eq!(params["cwd"], json!("/w"));
        assert_eq!(params["model"], json!("gpt-5"));
        assert_eq!(params["modelIdentity"], json!("gpt-5"));
        assert_eq!(params["model_identity"], Value::Null);
    }

    #[tokio::test]
    async fn driver_terminal_input_sends_base64_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({}));

        bus_driver(addr)
            .terminal_input("s1", &[1, 2, 3])
            .await
            .expect("input ok");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "sessions.terminalInput");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["bytesB64"], json!("AQID")); // base64([1,2,3])
    }

    // ── federation ──────────────────────────────────────────────────────────

    /// A hub-stamped envelope (a peer's event republished by the local hub)
    /// surfaces its hub on the BusEvent; an empty stamp reads as local.
    #[tokio::test]
    async fn events_carry_the_envelope_hub_stamp() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut write, mut read) = ws.split();
            while let Some(Ok(Message::Text(txt))) = read.next().await {
                let v: Value = serde_json::from_str(&txt).unwrap();
                if v.get("op").and_then(|o| o.as_str()) == Some("subscribe") {
                    for hub in [json!("work"), json!(""), Value::Null] {
                        let ev = json!({ "op": "event", "event": {
                            "id": "e1", "type": "agent.snapshot", "source": "hub",
                            "hub": hub, "data": { "sessionId": "r1" }
                        }});
                        let _ = write.send(Message::Text(ev.to_string())).await;
                    }
                }
            }
        });

        let (client, mut events) = BusClient::connect(format!("ws://{addr}/bus"), None);
        client.subscribe(vec!["agent.*".to_string()]).unwrap();

        assert_eq!(next_event(&mut events).await.topic, TOPIC_BUS_CONNECTED);
        let stamped = next_event(&mut events).await;
        assert_eq!(stamped.hub.as_deref(), Some("work"));
        assert_eq!(stamped.data["sessionId"], json!("r1"));
        // An empty or null stamp is not a hub — both are local.
        assert_eq!(next_event(&mut events).await.hub, None);
        assert_eq!(next_event(&mut events).await.hub, None);
    }

    async fn next_event(rx: &mut mpsc::UnboundedReceiver<BusEvent>) -> BusEvent {
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("event within 3s")
            .expect("event channel open")
    }

    async fn next_call(rx: &mut mpsc::UnboundedReceiver<(String, Value)>) -> (String, Value) {
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open")
    }

    #[tokio::test]
    async fn remote_driver_qualifies_message_approve_and_signal() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "ok": true }));
        let drv = remote_driver(addr);

        drv.message("r1", "hello").await.expect("message ok");
        drv.approve("r1", "yes", None).await.expect("approve ok");
        drv.signal("r1", "SIGINT").await.expect("signal ok");

        let (method, params) = next_call(&mut rx).await;
        assert_eq!(method, "hub:work/agents.sendMessage");
        assert_eq!(params["sessionId"], json!("r1"));
        assert_eq!(params["text"], json!("hello"));
        let (method, params) = next_call(&mut rx).await;
        assert_eq!(method, "hub:work/claude.approve");
        assert_eq!(params["decision"], json!("yes"));
        let (method, params) = next_call(&mut rx).await;
        assert_eq!(method, "hub:work/claude.signal");
        assert_eq!(params["signal"], json!("SIGINT"));
    }

    /// Across federation there is no `claude.answer`: answer text (and a
    /// collected multi-answer set) rides the peer's message path instead.
    #[tokio::test]
    async fn remote_driver_answers_via_the_message_path() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "ok": true }));
        let drv = remote_driver(addr);

        drv.answer_text("r1", "Option B").await.expect("answer ok");
        drv.answer_all("r1", vec!["Option A".into(), "free text".into()])
            .await
            .expect("multi-answer ok");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "hub:work/agents.sendMessage");
        assert_eq!(params["text"], json!("Option B"));
        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "hub:work/agents.sendMessage");
        assert_eq!(params["text"], json!("Option A\nfree text"));

        // Pick-by-number has no meaning in a chat message; the caller must
        // resolve the label first (the App does), so the driver refuses.
        assert!(drv.answer_option("r1", 2).await.is_err());
    }

    async fn reconnecting_subscribe_hub(
        listener: TcpListener,
        tx: mpsc::UnboundedSender<Vec<String>>,
    ) {
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (_write, mut read) = ws.split();
            while let Some(Ok(Message::Text(txt))) = read.next().await {
                let v: Value = serde_json::from_str(&txt).unwrap();
                if v.get("op").and_then(|o| o.as_str()) == Some("subscribe") {
                    let topics = v
                        .get("topics")
                        .and_then(|t| t.as_array())
                        .unwrap()
                        .iter()
                        .map(|t| t.as_str().unwrap().to_string())
                        .collect::<Vec<_>>();
                    let _ = tx.send(topics);
                    break;
                }
            }
        }
    }

    #[tokio::test]
    async fn subscriptions_are_reasserted_after_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(reconnecting_subscribe_hub(listener, tx));

        let (client, _events) = BusClient::connect(format!("ws://{addr}/bus"), None);
        client
            .subscribe(vec!["agent.snapshot".into(), "pty.bytes.s1".into()])
            .expect("subscribe ok");

        let first = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("first subscribe within 3s")
            .expect("recorder open");
        assert_eq!(first, vec!["agent.snapshot", "pty.bytes.s1"]);

        let second = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("re-subscribe within 3s")
            .expect("recorder open");
        assert_eq!(second, vec!["agent.snapshot", "pty.bytes.s1"]);
    }
}

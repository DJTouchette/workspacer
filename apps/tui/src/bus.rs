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
}

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
                let _ = event_tx.send(BusEvent { topic, data });
            }
        }
        _ => {} // hello / subscribed / unsubscribed acks
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
}

impl Driver {
    pub async fn message(&self, sid: &str, text: &str) -> Result<()> {
        match &self.bus {
            Some(b) => b
                .call(
                    "agents.sendMessage",
                    json!({ "sessionId": sid, "text": text }),
                )
                .await
                .map(|_| ()),
            None => self.claudemon.message(sid, text).await,
        }
    }

    pub async fn answer_text(&self, sid: &str, text: &str) -> Result<()> {
        match &self.bus {
            Some(b) => b
                .call("claude.answer", json!({ "sessionId": sid, "text": text }))
                .await
                .map(|_| ()),
            None => self.claudemon.answer_text(sid, text).await,
        }
    }

    pub async fn answer_option(&self, sid: &str, option: u64) -> Result<()> {
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
        match &self.bus {
            Some(b) => {
                let mut params = json!({ "sessionId": sid, "decision": decision });
                if let Some(r) = reason {
                    params["reason"] = json!(r);
                }
                b.call("claude.approve", params).await.map(|_| ())
            }
            None => self.claudemon.approve(sid, decision, reason).await,
        }
    }

    pub async fn signal(&self, sid: &str, signal: &str) -> Result<()> {
        match &self.bus {
            Some(b) => b
                .call(
                    "claude.signal",
                    json!({ "sessionId": sid, "signal": signal }),
                )
                .await
                .map(|_| ()),
            None => self.claudemon.signal(sid, signal).await,
        }
    }

    /// Spawn a fresh (or resumed) agent and return its session id. On the bus the
    /// brain builds the argv from the profile id (so the TUI doesn't); claudemon-
    /// direct builds it here, the way the TUI always has.
    pub async fn spawn(
        &self,
        cwd: String,
        profile: &crate::profiles::Profile,
        resume_session_id: Option<String>,
    ) -> Result<String> {
        match &self.bus {
            Some(b) => {
                let mut params = json!({ "cwd": cwd, "profileId": profile.id });
                if let Some(rid) = &resume_session_id {
                    params["resumeSessionId"] = json!(rid);
                }
                let res = b.call("agents.spawn", params).await?;
                res.get("sessionId")
                    .and_then(|s| s.as_str())
                    .map(String::from)
                    .ok_or_else(|| anyhow!("agents.spawn returned no sessionId"))
            }
            None => {
                let resume = resume_session_id.is_some();
                let session_id =
                    resume_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let argv = crate::profiles::build_argv(profile, None, false, &session_id, resume);
                let env = crate::profiles::build_env(profile);
                self.claudemon.spawn(argv, cwd, env, &session_id).await
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

    /// Live model/effort switch for a managed session. On the bus this is
    /// `claude.setModel`; a capability cliff (opencode/pi, or a PTY session) comes
    /// back as `{ ok:false, error }`, surfaced as an `Err`. REST otherwise.
    pub async fn set_model(
        &self,
        sid: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<()> {
        match &self.bus {
            Some(b) => {
                let mut params = json!({ "sessionId": sid });
                if let Some(m) = model {
                    params["model"] = json!(m);
                }
                if let Some(e) = effort {
                    params["effort"] = json!(e);
                }
                check_ok(b.call("claude.setModel", params).await?).map(|_| ())
            }
            None => self.claudemon.set_model(sid, model, effort).await,
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
                    params["model"] = json!(m);
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
        let ev = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("event within 3s")
            .expect("event channel open");
        assert_eq!(ev.topic, "agent.snapshot");
        assert_eq!(ev.data["session_id"], json!("s1"));
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
            claudemon: crate::claudemon::Claudemon::new("http://unused".into()),
            bus: Some(client),
        }
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
    async fn driver_spawn_sends_profile_id_and_returns_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "sessionId": "spawned-1" }));

        let profile = crate::profiles::Profile {
            id: "work".into(),
            name: "Work".into(),
            config_dir: String::new(),
            extra_args: vec![],
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
    }

    #[tokio::test]
    async fn driver_routes_set_model_to_capability() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut rx = recording_hub(listener, json!({ "ok": true, "model": "gpt-5" }));

        bus_driver(addr)
            .set_model("s1", Some("gpt-5"), Some("high"))
            .await
            .expect("set_model ok");

        let (method, params) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("call within 3s")
            .expect("recorder open");
        assert_eq!(method, "claude.setModel");
        assert_eq!(params["sessionId"], json!("s1"));
        assert_eq!(params["model"], json!("gpt-5"));
        assert_eq!(params["effort"], json!("high"));
    }

    #[tokio::test]
    async fn driver_set_model_surfaces_capability_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _rx = recording_hub(listener, json!({ "ok": false, "error": "no model switch" }));

        let err = bus_driver(addr)
            .set_model("s1", Some("x"), None)
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
}

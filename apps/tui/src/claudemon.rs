//! claudemon REST + SSE client — the TUI's source of truth.
//!
//! A standalone TUI can't use the hub-bus capabilities (`agents.list`, etc.):
//! those are registered by the Electron main process and absent when it isn't
//! running. claudemon, however, exposes the whole surface over plain HTTP on
//! loopback — list, transcript, approve/answer/message/signal, spawn, and a
//! `/events` SSE stream. Since it's all localhost we hand-roll the requests
//! over a TCP socket rather than pull in a full HTTP stack.

use anyhow::{anyhow, Result};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::types::Agent;

/// A chunk of raw PTY output for a session, streamed from `/sessions/:id/stream`.
pub struct PtyChunk {
    pub session_id: String,
    pub bytes: Vec<u8>,
}

/// How a PTY stream ended.
pub enum StreamEnd {
    /// Stream closed (daemon restart, EOF) — caller should reconnect.
    Disconnected,
    /// The session has no PTY attached (404) — e.g. an external session claudemon
    /// only observes via hooks. There's nothing to stream; don't retry.
    NoPty,
}

/// A model a managed provider can launch/switch to, from
/// `GET /providers/:provider/models`. An empty list is valid (the picker falls
/// back to free-text entry).
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub default: bool,
}

/// A cross-provider handoff brief from `POST /sessions/:id/handoff`: the
/// markdown a successor agent reads, plus the file it was persisted to (absent
/// when `no_persist`).
#[derive(Debug, Clone)]
pub struct HandoffBrief {
    /// The brief markdown. The daemon already persists it to `path`, and the TUI
    /// hands the successor the file path rather than the text, so this is carried
    /// for completeness / callers that want it inline.
    #[allow(dead_code)]
    pub markdown: String,
    pub path: Option<String>,
}

/// Connection-state / change notifications from the `/events` SSE stream.
#[derive(Debug)]
pub enum DaemonEvent {
    Connected,
    Disconnected,
    /// Some session changed — the app should re-pull the list.
    Changed,
}

#[derive(Clone)]
pub struct Claudemon {
    /// Base URL, e.g. `http://127.0.0.1:7891`.
    base: String,
}

impl Claudemon {
    pub fn new(base: String) -> Self {
        Claudemon { base }
    }

    // ── reads ───────────────────────────────────────────────────────────────

    pub async fn list(&self) -> Result<Vec<Agent>> {
        let v = self.get_json("/sessions").await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Parsed conversation (`{ items: [...] }`) — richer than `/transcript`
    /// (carries tool results / work-log items joined to their calls).
    pub async fn conversation(&self, session_id: &str) -> Result<Value> {
        self.get_json(&format!("/sessions/{session_id}/conversation"))
            .await
    }

    /// The session's current mode (unknown/input/responding/approval/...), or
    /// None if it can't be read. Used to wait until a fresh agent is ready.
    pub async fn session_mode(&self, session_id: &str) -> Option<String> {
        let v = self
            .get_json(&format!("/sessions/{session_id}"))
            .await
            .ok()?;
        v.get("mode")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
    }

    // ── control ───────────────────────────────────────────────────────────

    pub async fn approve(
        &self,
        session_id: &str,
        decision: &str,
        reason: Option<String>,
    ) -> Result<()> {
        let mut body = json!({ "decision": decision });
        if let Some(r) = reason {
            body["reason"] = json!(r);
        }
        self.post_ok(&format!("/sessions/{session_id}/approve"), &body)
            .await
    }

    pub async fn answer_option(&self, session_id: &str, option: u64) -> Result<()> {
        self.post_ok(
            &format!("/sessions/{session_id}/answer"),
            &json!({ "option": option }),
        )
        .await
    }

    pub async fn answer_text(&self, session_id: &str, text: &str) -> Result<()> {
        self.post_ok(
            &format!("/sessions/{session_id}/answer"),
            &json!({ "text": text }),
        )
        .await
    }

    /// Answer a multi-question set in one shot: one raw answer per question
    /// (a 1-indexed digit string for a pick, free text otherwise, or joined
    /// labels for a multi-select). The daemon maps digits to labels on both
    /// transports and types them sequentially into the PTY picker.
    pub async fn answer_all(&self, session_id: &str, answers: &[String]) -> Result<()> {
        self.post_ok(
            &format!("/sessions/{session_id}/answer"),
            &json!({ "answers": answers }),
        )
        .await
    }

    pub async fn message(&self, session_id: &str, text: &str) -> Result<()> {
        self.post_ok(
            &format!("/sessions/{session_id}/message"),
            &json!({ "text": text }),
        )
        .await
    }

    pub async fn signal(&self, session_id: &str, signal: &str) -> Result<()> {
        self.post_ok(
            &format!("/sessions/{session_id}/signal"),
            &json!({ "signal": signal }),
        )
        .await
    }

    // ── provider parity (model / permission-mode / handoff / managed spawn) ──

    /// Live model/effort switch for a *managed* session (`POST /sessions/:id/model`).
    /// PTY (claude) sessions 409 here — they switch via the `/model` slash command
    /// on the message path, so callers route those to `message` instead. Surfaces
    /// the daemon's `{ ok:false, error }` (409 for opencode/pi capability cliffs)
    /// as an `Err` with the message.
    pub async fn set_model(
        &self,
        session_id: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({});
        if let Some(m) = model {
            body["model"] = json!(m);
        }
        if let Some(e) = effort {
            body["effort"] = json!(e);
        }
        let (code, resp) = self
            .post_status(&format!("/sessions/{session_id}/model"), &body)
            .await?;
        ok_or_daemon_error(code, resp, "model switch failed").map(|_| ())
    }

    /// Live permission-mode switch (`POST /sessions/:id/permission-mode`). Returns
    /// the mode the daemon settled on. A capability cliff (managed yolo→ask when
    /// spawned in bypass, or a mode outside the shift+tab cycle) comes back as a
    /// 409 `{ ok:false, error }`, surfaced as an `Err`.
    pub async fn set_permission_mode(&self, session_id: &str, mode: &str) -> Result<String> {
        let (code, resp) = self
            .post_status(
                &format!("/sessions/{session_id}/permission-mode"),
                &json!({ "mode": mode }),
            )
            .await?;
        let resp = ok_or_daemon_error(code, resp, "permission-mode switch failed")?;
        Ok(resp
            .get("mode")
            .and_then(|m| m.as_str())
            .unwrap_or(mode)
            .to_string())
    }

    /// Build a cross-provider handoff brief (`POST /sessions/:id/handoff`),
    /// persisting it under `~/.workspacer/handoffs/`. Returns the markdown + path.
    pub async fn handoff(&self, session_id: &str) -> Result<HandoffBrief> {
        let (code, resp) = self
            .post_status(&format!("/sessions/{session_id}/handoff"), &json!({}))
            .await?;
        let resp = ok_or_daemon_error(code, resp, "handoff failed")?;
        Ok(HandoffBrief {
            markdown: resp
                .get("markdown")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string(),
            path: resp.get("path").and_then(|p| p.as_str()).map(String::from),
        })
    }

    /// Spawn a *managed* (adapter-driven) session — Codex / OpenCode / Pi
    /// (`POST /sessions/spawn-managed`). Returns the assigned session id.
    pub async fn spawn_managed(
        &self,
        provider: &str,
        cwd: &str,
        model: Option<&str>,
        effort: Option<&str>,
        yolo: bool,
        session_id: &str,
    ) -> Result<String> {
        let mut body = json!({ "provider": provider, "cwd": cwd, "yolo": yolo });
        if let Some(m) = model {
            body["model"] = json!(m);
        }
        if let Some(e) = effort {
            body["effort"] = json!(e);
        }
        if !session_id.is_empty() {
            body["session_id"] = json!(session_id);
        }
        let resp = self.post_json("/sessions/spawn-managed", &body).await?;
        resp.get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("spawn-managed response missing session_id"))
    }

    /// List the models a managed provider can launch with, live-queried from its
    /// CLI (`GET /providers/:provider/models`). An empty list is valid.
    pub async fn provider_models(&self, provider: &str, cwd: &str) -> Result<Vec<ProviderModel>> {
        let mut q = format!("/providers/{provider}/models");
        if !cwd.is_empty() {
            q.push_str(&format!("?cwd={}", encode(cwd)));
        }
        let v = self.get_json(&q).await?;
        let models = v
            .get("models")
            .cloned()
            .and_then(|m| serde_json::from_value(m).ok())
            .unwrap_or_default();
        Ok(models)
    }

    // ── terminal path (raw PTY) ─────────────────────────────────────────────

    /// Forward raw key bytes to the PTY. `bytes_b64` path sends them verbatim
    /// (no newline munging), which is what we want for a live terminal.
    pub async fn input_bytes(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.post_ok(
            &format!("/sessions/{session_id}/input"),
            &json!({ "bytes_b64": b64, "newline": false }),
        )
        .await
    }

    /// Tell claudemon to resize the PTY so Claude's TUI reflows to our pane.
    pub async fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()> {
        self.post_ok(
            &format!("/sessions/{session_id}/resize"),
            &json!({ "cols": cols, "rows": rows }),
        )
        .await
    }

    /// Stream the session's PTY bytes, sending each chunk to `sink`. The first
    /// frame is a snapshot replay of the current screen. Returns when the stream
    /// ends or errors (the caller reconnects).
    pub async fn read_pty_stream(
        &self,
        session_id: &str,
        sink: &mpsc::UnboundedSender<PtyChunk>,
    ) -> Result<StreamEnd> {
        const MAX_BUF: usize = 4 * 1024 * 1024;

        let (host, port) = split_host_port(&self.base);
        let mut stream = TcpStream::connect((host.as_str(), port)).await?;
        let req = format!(
            "GET /sessions/{session_id}/stream HTTP/1.1\r\nHost: {host}:{port}\r\n\
             Accept: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await?;
        stream.flush().await?;

        let mut buf: Vec<u8> = Vec::new();
        let mut headers_done = false;
        let mut tmp = [0u8; 16384];
        loop {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                return Ok(StreamEnd::Disconnected);
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.len() > MAX_BUF {
                return Err(anyhow!("stream buffer overflow"));
            }

            if !headers_done {
                match find(&buf, b"\r\n\r\n") {
                    Some(i) => {
                        // 404 → this session has no PTY (external/observed-only).
                        let headers = String::from_utf8_lossy(&buf[..i]);
                        let code = headers
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .and_then(|c| c.parse::<u16>().ok())
                            .unwrap_or(0);
                        if code == 404 {
                            return Ok(StreamEnd::NoPty);
                        }
                        if !(200..300).contains(&code) {
                            return Ok(StreamEnd::Disconnected);
                        }
                        buf.drain(..i + 4);
                        headers_done = true;
                    }
                    None => continue,
                }
            }

            for frame in drain_sse_frames(&mut buf) {
                let text = String::from_utf8_lossy(&frame);
                for line in text.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        if let Ok(bytes) =
                            base64::engine::general_purpose::STANDARD.decode(data.trim())
                        {
                            let _ = sink.send(PtyChunk {
                                session_id: session_id.to_string(),
                                bytes,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Spawn a Claude session in a fresh PTY. Returns the assigned session id.
    pub async fn spawn(
        &self,
        argv: Vec<String>,
        cwd: String,
        env: Map<String, Value>,
        session_id: &str,
    ) -> Result<String> {
        let mut body = json!({
            "argv": argv,
            "cwd": cwd,
            "cols": 120,
            "rows": 32,
            "env": Value::Object(env),
        });
        // Pin the id so it matches `--session-id` in argv (empty for shells).
        if !session_id.is_empty() {
            body["session_id"] = json!(session_id);
        }
        let resp = self.post_json("/sessions/spawn", &body).await?;
        resp.get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("spawn response missing session_id"))
    }

    // ── git (review pane) ───────────────────────────────────────────────────

    /// Branch + changed files for a work tree. Returns `(branch, files)`.
    pub async fn git_status(
        &self,
        cwd: &str,
    ) -> Result<(Option<String>, Vec<crate::types::FileStatus>)> {
        let v = self
            .get_json(&format!("/git/status?cwd={}", encode(cwd)))
            .await?;
        let branch = v.get("branch").and_then(|b| b.as_str()).map(str::to_string);
        let files = v
            .get("files")
            .cloned()
            .and_then(|f| serde_json::from_value(f).ok())
            .unwrap_or_default();
        Ok((branch, files))
    }

    /// Raw unified diff text for a path (or the whole tree when `path` is empty).
    /// `staged` selects index-vs-HEAD; `untracked` renders a new file as
    /// all-added (requires a path).
    pub async fn git_diff(
        &self,
        cwd: &str,
        path: &str,
        staged: bool,
        untracked: bool,
    ) -> Result<String> {
        let mut q = format!("/git/diff?cwd={}&staged={staged}", encode(cwd));
        if !path.is_empty() {
            q.push_str(&format!("&path={}", encode(path)));
        }
        if untracked {
            q.push_str("&untracked=true");
        }
        let v = self.get_json(&q).await?;
        Ok(v.get("diff")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string())
    }

    pub async fn git_stage(&self, cwd: &str, path: Option<&str>) -> Result<()> {
        let mut body = json!({ "cwd": cwd });
        if let Some(p) = path {
            body["path"] = json!(p);
        }
        self.git_action_ok("/git/stage", &body).await
    }

    pub async fn git_unstage(&self, cwd: &str, path: Option<&str>) -> Result<()> {
        let mut body = json!({ "cwd": cwd });
        if let Some(p) = path {
            body["path"] = json!(p);
        }
        self.git_action_ok("/git/unstage", &body).await
    }

    pub async fn git_commit(&self, cwd: &str, message: &str) -> Result<()> {
        self.git_action_ok("/git/commit", &json!({ "cwd": cwd, "message": message }))
            .await
    }

    pub async fn git_push(&self, cwd: &str) -> Result<()> {
        self.git_action_ok("/git/push", &json!({ "cwd": cwd }))
            .await
    }

    /// POST a git action and surface `{ ok:false, error }` as an `Err` so the UI
    /// can toast the git failure (e.g. "nothing to commit", push rejected).
    async fn git_action_ok(&self, path: &str, body: &Value) -> Result<()> {
        let resp = self.post_json(path, body).await?;
        if resp.get("ok").and_then(|b| b.as_bool()) == Some(false) {
            let err = resp
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("git failed");
            return Err(anyhow!("{}", err.trim()));
        }
        Ok(())
    }

    // ── HTTP plumbing ───────────────────────────────────────────────────────

    async fn get_json(&self, path: &str) -> Result<Value> {
        let (host, port) = split_host_port(&self.base);
        let mut stream = TcpStream::connect((host.as_str(), port)).await?;
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: application/json\r\n\
             Connection: close\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await?;
        stream.flush().await?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        parse_http_json(&raw)
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let (host, port) = split_host_port(&self.base);
        let mut stream = TcpStream::connect((host.as_str(), port)).await?;
        let data = serde_json::to_vec(body)?;
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        parse_http_json(&raw)
    }

    /// POST that only cares whether it succeeded.
    async fn post_ok(&self, path: &str, body: &Value) -> Result<()> {
        self.post_json(path, body).await.map(|_| ())
    }

    /// POST returning `(status_code, json_body)` *without* treating a non-2xx as
    /// an error — so callers can read a structured `{ ok:false, error }` body
    /// (the 409 capability cliffs for model / permission-mode) and surface the
    /// daemon's own message instead of a raw "claudemon 409: {...}" string.
    async fn post_status(&self, path: &str, body: &Value) -> Result<(u16, Value)> {
        let (host, port) = split_host_port(&self.base);
        let mut stream = TcpStream::connect((host.as_str(), port)).await?;
        let data = serde_json::to_vec(body)?;
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        parse_http_status(&raw)
    }
}

/// Fold a `(status, body)` from [`Claudemon::post_status`] into a `Result`: on a
/// 2xx return the body; otherwise pull `body.error` (the daemon's message) or
/// fall back to `fallback`.
fn ok_or_daemon_error(code: u16, body: Value, fallback: &str) -> Result<Value> {
    if (200..300).contains(&code) {
        Ok(body)
    } else {
        let msg = body
            .get("error")
            .and_then(|e| e.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback.to_string());
        Err(anyhow!("{msg}"))
    }
}

/// Subscribe to claudemon's `/events` SSE stream, emitting a `Changed` on every
/// `session.update` frame and surfacing connect/disconnect transitions. Owns
/// its own reconnect loop with backoff.
pub fn spawn_events(base: String) -> mpsc::UnboundedReceiver<DaemonEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut backoff = std::time::Duration::from_millis(500);
        loop {
            let was_connected = match sse_connect(&base, &tx).await {
                Ok(connected) => {
                    // Clean EOF — retry quickly.
                    backoff = std::time::Duration::from_millis(500);
                    connected
                }
                Err(_) => false,
            };
            // Only pulse Disconnected if we actually told the UI we were Connected.
            if was_connected {
                let _ = tx.send(DaemonEvent::Disconnected);
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(std::time::Duration::from_secs(8));
        }
    });
    rx
}

/// One authoritative statusLine tick for a session (model / context% / cost /
/// rate limits), streamed from `/statusline/stream`.
#[derive(Debug)]
pub struct StatusLineMsg {
    pub session_id: String,
    pub status_line: crate::types::StatusLine,
}

/// The `/statusline/stream` frame shape (claudemon's `StatusLineUpdate`).
#[derive(Deserialize)]
struct StatusLineFrame {
    session_id: String,
    status_line: crate::types::StatusLine,
}

/// Subscribe to `/statusline/stream`, emitting a [`StatusLineMsg`] per tick.
/// Owns its own reconnect loop with backoff, like [`spawn_events`].
pub fn spawn_status_lines(base: String) -> mpsc::UnboundedReceiver<StatusLineMsg> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut backoff = std::time::Duration::from_millis(500);
        loop {
            let _ = status_line_connect(&base, &tx).await;
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(std::time::Duration::from_secs(8));
        }
    });
    rx
}

async fn status_line_connect(base: &str, tx: &mpsc::UnboundedSender<StatusLineMsg>) -> Result<()> {
    const MAX_BUF: usize = 4 * 1024 * 1024;

    let (host, port) = split_host_port(base);
    let mut stream = TcpStream::connect((host.as_str(), port)).await?;
    let req = format!(
        "GET /statusline/stream HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\n\
         Connection: keep-alive\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut buf: Vec<u8> = Vec::new();
    let mut headers_done = false;
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_BUF {
            return Err(anyhow!("stream buffer overflow"));
        }

        if !headers_done {
            match find(&buf, b"\r\n\r\n") {
                Some(i) => {
                    let headers = String::from_utf8_lossy(&buf[..i]);
                    let code = headers
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|c| c.parse::<u16>().ok())
                        .unwrap_or(0);
                    if !(200..300).contains(&code) {
                        return Err(anyhow!("HTTP {code}"));
                    }
                    buf.drain(..i + 4);
                    headers_done = true;
                }
                None => continue,
            }
        }

        for frame in drain_sse_frames(&mut buf) {
            let text = String::from_utf8_lossy(&frame);
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    if let Ok(f) = serde_json::from_str::<StatusLineFrame>(data.trim()) {
                        let _ = tx.send(StatusLineMsg {
                            session_id: f.session_id,
                            status_line: f.status_line,
                        });
                    }
                }
            }
        }
    }
}

/// Hold one SSE connection, forwarding a `Changed` per data frame. Returns when
/// the stream ends or errors. Returns `Ok(true)` if `Connected` was emitted,
/// `Ok(false)` if the connection failed before that point.
async fn sse_connect(base: &str, tx: &mpsc::UnboundedSender<DaemonEvent>) -> Result<bool> {
    const MAX_BUF: usize = 4 * 1024 * 1024;

    let (host, port) = split_host_port(base);
    let mut stream = TcpStream::connect((host.as_str(), port)).await?;
    let req = format!(
        "GET /events HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\n\
         Connection: keep-alive\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;
    // NOTE: Connected is sent AFTER we confirm a 2xx response below.

    let mut buf: Vec<u8> = Vec::new();
    let mut headers_done = false;
    let mut was_connected = false;
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(was_connected); // EOF
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_BUF {
            return Err(anyhow!("stream buffer overflow"));
        }

        if !headers_done {
            match find(&buf, b"\r\n\r\n") {
                Some(i) => {
                    let headers = String::from_utf8_lossy(&buf[..i]);
                    let code = headers
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|c| c.parse::<u16>().ok())
                        .unwrap_or(0);
                    if !(200..300).contains(&code) {
                        return Err(anyhow!("HTTP {code}"));
                    }
                    buf.drain(..i + 4);
                    headers_done = true;
                    // Confirmed 2xx — safe to signal Connected.
                    let _ = tx.send(DaemonEvent::Connected);
                    was_connected = true;
                }
                None => continue,
            }
        }

        // SSE frames are separated by a blank line.
        for frame in drain_sse_frames(&mut buf) {
            let text = String::from_utf8_lossy(&frame);
            if text.lines().any(|l| l.starts_with("data:")) {
                let _ = tx.send(DaemonEvent::Changed);
            }
        }
    }
}

/// Drain all complete SSE frames (separated by `\n\n`) from `buf`, returning
/// them as a `Vec` of raw frame byte vectors. `buf` is left with any trailing
/// incomplete frame. Behaviour-preserving: identical to the previous inline
/// `while let Some(i) = find(&buf, b"\n\n")` loop used in both stream loops.
fn drain_sse_frames(buf: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    while let Some(i) = find(buf, b"\n\n") {
        let frame: Vec<u8> = buf.drain(..i).collect();
        buf.drain(..2); // consume the trailing \n\n
        frames.push(frame);
    }
    frames
}

/// Parse a buffered HTTP/1.1 response: check the status line, return the JSON
/// body (or `Null` for an empty body).
fn parse_http_json(raw: &[u8]) -> Result<Value> {
    let text = String::from_utf8_lossy(raw);
    let (headers, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed HTTP response"))?;
    let code = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&code) {
        return Err(anyhow!("claudemon {code}: {}", body.trim()));
    }
    Ok(serde_json::from_str(body.trim()).unwrap_or(Value::Null))
}

/// Like [`parse_http_json`] but returns the status code alongside the body
/// instead of erroring on non-2xx, so a caller can read a structured error body.
fn parse_http_status(raw: &[u8]) -> Result<(u16, Value)> {
    let text = String::from_utf8_lossy(raw);
    let (headers, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed HTTP response"))?;
    let code = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    Ok((
        code,
        serde_json::from_str(body.trim()).unwrap_or(Value::Null),
    ))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Minimal percent-encoding for a query-string *value*. Keeps the unreserved
/// set plus `/` (paths are common and `/` is legal in a query), encodes
/// everything else — enough to carry cwd/path values (spaces, `&`, `=`, `#`,
/// unicode) safely to claudemon's `Query` extractor.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pull `host` and `port` out of a `http://host:port` base.
fn split_host_port(base: &str) -> (String, u16) {
    let stripped = base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let mut parts = stripped.splitn(2, ':');
    let host = parts.next().unwrap_or("127.0.0.1").to_string();
    let port = parts.next().and_then(|p| p.parse().ok()).unwrap_or(7891);
    (host, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_single_frame() {
        let mut buf = b"data: hello\n\n".to_vec();
        let frames = drain_sse_frames(&mut buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], b"data: hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_multiple_frames() {
        let mut buf = b"data: a\n\ndata: b\n\n".to_vec();
        let frames = drain_sse_frames(&mut buf);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], b"data: a");
        assert_eq!(frames[1], b"data: b");
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_partial_frame_stays_in_buf() {
        let mut buf = b"data: incomplete".to_vec();
        let frames = drain_sse_frames(&mut buf);
        assert!(frames.is_empty());
        assert_eq!(buf, b"data: incomplete");
    }

    #[test]
    fn drain_one_complete_one_partial() {
        let mut buf = b"data: done\n\ndata: pending".to_vec();
        let frames = drain_sse_frames(&mut buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], b"data: done");
        assert_eq!(buf, b"data: pending");
    }

    #[test]
    fn http_status_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true}";
        assert_eq!(parse_http_json(raw).unwrap()["ok"], true);

        let err = b"HTTP/1.1 409 Conflict\r\n\r\nsession is not accepting input";
        assert!(parse_http_json(err).is_err());
    }

    #[test]
    fn host_port_parsing() {
        assert_eq!(
            split_host_port("http://127.0.0.1:7891"),
            ("127.0.0.1".into(), 7891)
        );
        assert_eq!(
            split_host_port("http://localhost:9/"),
            ("localhost".into(), 9)
        );
    }

    #[test]
    fn encode_preserves_paths_and_escapes_specials() {
        assert_eq!(encode("/home/u/rune-lang"), "/home/u/rune-lang");
        assert_eq!(encode("with space"), "with%20space");
        assert_eq!(encode("a&b=c#d"), "a%26b%3Dc%23d");
        assert_eq!(encode("src/lib.rs"), "src/lib.rs");
    }

    /// Live round-trip: spawn `cat` in a PTY, stream its output, send input,
    /// and confirm the echo comes back. Exercises spawn + read_pty_stream +
    /// input_bytes (base64 both directions). Run with:
    ///   cargo test live_pty -- --ignored --nocapture   (claudemon must be up)
    #[tokio::test]
    #[ignore]
    async fn live_pty() {
        use std::time::Duration;
        let cm = Claudemon::new("http://127.0.0.1:7891".into());
        let sid = cm
            .spawn(vec!["cat".into()], "/tmp".into(), Map::new(), "")
            .await
            .expect("spawn cat");

        let (tx, mut rx) = mpsc::unbounded_channel::<PtyChunk>();
        let cm2 = cm.clone();
        let s = sid.clone();
        let reader = tokio::spawn(async move {
            let _ = cm2.read_pty_stream(&s, &tx).await;
        });

        // Let the stream attach, then type a line `cat` will echo back.
        tokio::time::sleep(Duration::from_millis(300)).await;
        cm.input_bytes(&sid, b"PINGPONG\n").await.expect("input");

        let mut buf = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(c)) = tokio::time::timeout(Duration::from_millis(250), rx.recv()).await {
                buf.extend_from_slice(&c.bytes)
            }
        }
        reader.abort();
        let _ = cm.signal(&sid, "SIGTERM").await;

        let text = String::from_utf8_lossy(&buf);
        eprintln!(
            "pty bytes: {} | contains PINGPONG: {}",
            buf.len(),
            text.contains("PINGPONG")
        );
        assert!(
            text.contains("PINGPONG"),
            "expected echoed input in PTY stream"
        );
    }

    /// Live check against a running claudemon. Run with:
    ///   cargo test live_list -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_list() {
        let cm = Claudemon::new("http://127.0.0.1:7891".into());
        let agents = cm.list().await.expect("claudemon GET /sessions");
        eprintln!("claudemon reports {} session(s):", agents.len());
        for a in &agents {
            // Usage is now returned directly by GET /sessions — no transcript
            // fetch needed.
            let extra = match &a.usage {
                Some(u) => format!(
                    "{} · {}/{} ctx · ${:.2}",
                    u.model.as_deref().unwrap_or("?"),
                    u.context_tokens,
                    u.context_limit,
                    u.cost_usd
                ),
                None => "no usage".into(),
            };
            eprintln!(
                "  {} [{}] {} — {extra}",
                &a.session_id[..8.min(a.session_id.len())],
                a.state(),
                a.cwd_str()
            );
        }
    }

    // ── provider-parity client round-trips (against a one-shot mock server) ──

    /// A one-shot mock HTTP server: answers the next request with
    /// `status`/`resp_body`, then hands back the parsed request as
    /// `(method, path, json_body)` for assertions.
    async fn mock_server(
        status: u16,
        reason: &'static str,
        resp_body: &'static str,
    ) -> (String, tokio::task::JoinHandle<(String, String, Value)>) {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let header_end = loop {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break buf.len();
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = find(&buf, b"\r\n\r\n") {
                    break i;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let req_line = head.lines().next().unwrap_or("").to_string();
            let mut parts = req_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let content_len = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            let mut body_bytes = buf[(header_end + 4).min(buf.len())..].to_vec();
            while body_bytes.len() < content_len {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                body_bytes.extend_from_slice(&tmp[..n]);
            }
            let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
            (method, path, body)
        });
        (base, handle)
    }

    #[tokio::test]
    async fn set_model_posts_body_and_succeeds() {
        let (base, srv) = mock_server(200, "OK", r#"{"ok":true,"model":"gpt-5"}"#).await;
        Claudemon::new(base)
            .set_model("s1", Some("gpt-5"), Some("high"))
            .await
            .expect("ok");
        let (method, path, body) = srv.await.unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/sessions/s1/model");
        assert_eq!(body["model"], json!("gpt-5"));
        assert_eq!(body["effort"], json!("high"));
    }

    #[tokio::test]
    async fn set_model_409_surfaces_daemon_error() {
        let (base, srv) = mock_server(
            409,
            "Conflict",
            r#"{"ok":false,"error":"opencode has no model switch"}"#,
        )
        .await;
        let err = Claudemon::new(base)
            .set_model("s1", Some("x"), None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("opencode has no model switch"),
            "got {err}"
        );
        let _ = srv.await;
    }

    #[tokio::test]
    async fn set_permission_mode_returns_settled_mode() {
        let (base, srv) = mock_server(200, "OK", r#"{"ok":true,"mode":"plan"}"#).await;
        let mode = Claudemon::new(base)
            .set_permission_mode("s1", "plan")
            .await
            .expect("ok");
        assert_eq!(mode, "plan");
        let (_, path, body) = srv.await.unwrap();
        assert_eq!(path, "/sessions/s1/permission-mode");
        assert_eq!(body["mode"], json!("plan"));
    }

    #[tokio::test]
    async fn permission_mode_409_surfaces_error() {
        let (base, srv) =
            mock_server(409, "Conflict", r#"{"ok":false,"error":"session is busy"}"#).await;
        let err = Claudemon::new(base)
            .set_permission_mode("s1", "plan")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("session is busy"), "got {err}");
        let _ = srv.await;
    }

    #[tokio::test]
    async fn handoff_returns_markdown_and_path() {
        let (base, srv) = mock_server(
            200,
            "OK",
            r##"{"ok":true,"markdown":"# brief","path":"/h/x.md"}"##,
        )
        .await;
        let brief = Claudemon::new(base).handoff("s1").await.expect("ok");
        assert_eq!(brief.markdown, "# brief");
        assert_eq!(brief.path.as_deref(), Some("/h/x.md"));
        let (_, path, _) = srv.await.unwrap();
        assert_eq!(path, "/sessions/s1/handoff");
    }

    #[tokio::test]
    async fn spawn_managed_returns_session_id_and_sends_fields() {
        let (base, srv) = mock_server(200, "OK", r#"{"session_id":"m1","cwd":"/w"}"#).await;
        let sid = Claudemon::new(base)
            .spawn_managed("codex", "/w", Some("gpt-5"), Some("high"), true, "pin-1")
            .await
            .expect("ok");
        assert_eq!(sid, "m1");
        let (_, path, body) = srv.await.unwrap();
        assert_eq!(path, "/sessions/spawn-managed");
        assert_eq!(body["provider"], json!("codex"));
        assert_eq!(body["cwd"], json!("/w"));
        assert_eq!(body["yolo"], json!(true));
        assert_eq!(body["session_id"], json!("pin-1"));
    }

    #[tokio::test]
    async fn provider_models_lists_and_carries_cwd_query() {
        let (base, srv) = mock_server(
            200,
            "OK",
            r#"{"models":[{"id":"gpt-5","label":"GPT-5","default":true},{"id":"o3"}]}"#,
        )
        .await;
        let models = Claudemon::new(base)
            .provider_models("codex", "/w")
            .await
            .expect("ok");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5");
        assert!(models[0].default);
        assert_eq!(models[1].id, "o3");
        let (method, path, _) = srv.await.unwrap();
        assert_eq!(method, "GET");
        assert!(path.starts_with("/providers/codex/models"), "got {path}");
        assert!(path.contains("cwd="), "cwd query missing: {path}");
    }

    #[tokio::test]
    async fn answer_all_posts_the_raw_answers_array() {
        let (base, srv) = mock_server(200, "OK", r#"{"ok":true}"#).await;
        Claudemon::new(base)
            .answer_all("s1", &["2".to_string(), "free text".to_string()])
            .await
            .expect("ok");
        let (method, path, body) = srv.await.unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/sessions/s1/answer");
        assert_eq!(body["answers"], json!(["2", "free text"]));
    }
}

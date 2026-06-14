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
        self.get_json(&format!("/sessions/{session_id}/conversation")).await
    }

    /// The session's current mode (unknown/input/responding/approval/...), or
    /// None if it can't be read. Used to wait until a fresh agent is ready.
    pub async fn session_mode(&self, session_id: &str) -> Option<String> {
        let v = self.get_json(&format!("/sessions/{session_id}")).await.ok()?;
        v.get("mode").and_then(|m| m.as_str()).map(|s| s.to_string())
    }

    // ── control ───────────────────────────────────────────────────────────

    pub async fn approve(&self, session_id: &str, decision: &str, reason: Option<String>) -> Result<()> {
        let mut body = json!({ "decision": decision });
        if let Some(r) = reason {
            body["reason"] = json!(r);
        }
        self.post_ok(&format!("/sessions/{session_id}/approve"), &body).await
    }

    pub async fn answer_option(&self, session_id: &str, option: u64) -> Result<()> {
        self.post_ok(&format!("/sessions/{session_id}/answer"), &json!({ "option": option })).await
    }

    pub async fn answer_text(&self, session_id: &str, text: &str) -> Result<()> {
        self.post_ok(&format!("/sessions/{session_id}/answer"), &json!({ "text": text })).await
    }

    pub async fn message(&self, session_id: &str, text: &str) -> Result<()> {
        self.post_ok(&format!("/sessions/{session_id}/message"), &json!({ "text": text })).await
    }

    pub async fn signal(&self, session_id: &str, signal: &str) -> Result<()> {
        self.post_ok(&format!("/sessions/{session_id}/signal"), &json!({ "signal": signal })).await
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
                            let _ = sink.send(PtyChunk { session_id: session_id.to_string(), bytes });
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
    pub async fn git_status(&self, cwd: &str) -> Result<(Option<String>, Vec<crate::types::FileStatus>)> {
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
    pub async fn git_diff(&self, cwd: &str, path: &str, staged: bool, untracked: bool) -> Result<String> {
        let mut q = format!("/git/diff?cwd={}&staged={staged}", encode(cwd));
        if !path.is_empty() {
            q.push_str(&format!("&path={}", encode(path)));
        }
        if untracked {
            q.push_str("&untracked=true");
        }
        let v = self.get_json(&q).await?;
        Ok(v.get("diff").and_then(|d| d.as_str()).unwrap_or("").to_string())
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
        self.git_action_ok("/git/commit", &json!({ "cwd": cwd, "message": message })).await
    }

    pub async fn git_push(&self, cwd: &str) -> Result<()> {
        self.git_action_ok("/git/push", &json!({ "cwd": cwd })).await
    }

    /// POST a git action and surface `{ ok:false, error }` as an `Err` so the UI
    /// can toast the git failure (e.g. "nothing to commit", push rejected).
    async fn git_action_ok(&self, path: &str, body: &Value) -> Result<()> {
        let resp = self.post_json(path, body).await?;
        if resp.get("ok").and_then(|b| b.as_bool()) == Some(false) {
            let err = resp.get("error").and_then(|e| e.as_str()).unwrap_or("git failed");
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
}

/// Subscribe to claudemon's `/events` SSE stream, emitting a `Changed` on every
/// `session.update` frame and surfacing connect/disconnect transitions. Owns
/// its own reconnect loop with backoff.
pub fn spawn_events(base: String) -> mpsc::UnboundedReceiver<DaemonEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut backoff = std::time::Duration::from_millis(500);
        loop {
            if sse_connect(&base, &tx).await.is_ok() {
                // Clean EOF — retry quickly.
                backoff = std::time::Duration::from_millis(500);
            }
            let _ = tx.send(DaemonEvent::Disconnected);
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

        if !headers_done {
            match find(&buf, b"\r\n\r\n") {
                Some(i) => {
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
/// the stream ends or errors.
async fn sse_connect(base: &str, tx: &mpsc::UnboundedSender<DaemonEvent>) -> Result<()> {
    let (host, port) = split_host_port(base);
    let mut stream = TcpStream::connect((host.as_str(), port)).await?;
    let req = format!(
        "GET /events HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\n\
         Connection: keep-alive\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;
    let _ = tx.send(DaemonEvent::Connected);

    let mut buf: Vec<u8> = Vec::new();
    let mut headers_done = false;
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(()); // EOF
        }
        buf.extend_from_slice(&tmp[..n]);

        if !headers_done {
            match find(&buf, b"\r\n\r\n") {
                Some(i) => {
                    buf.drain(..i + 4);
                    headers_done = true;
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
        assert_eq!(split_host_port("http://127.0.0.1:7891"), ("127.0.0.1".into(), 7891));
        assert_eq!(split_host_port("http://localhost:9/"), ("localhost".into(), 9));
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
            match tokio::time::timeout(Duration::from_millis(250), rx.recv()).await {
                Ok(Some(c)) => buf.extend_from_slice(&c.bytes),
                _ => {}
            }
        }
        reader.abort();
        let _ = cm.signal(&sid, "SIGTERM").await;

        let text = String::from_utf8_lossy(&buf);
        eprintln!("pty bytes: {} | contains PINGPONG: {}", buf.len(), text.contains("PINGPONG"));
        assert!(text.contains("PINGPONG"), "expected echoed input in PTY stream");
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
}

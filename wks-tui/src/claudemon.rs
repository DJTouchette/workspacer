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

    pub async fn transcript(&self, session_id: &str) -> Result<Value> {
        self.get_json(&format!("/sessions/{session_id}/transcript")).await
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

            while let Some(i) = find(&buf, b"\n\n") {
                let frame: Vec<u8> = buf.drain(..i).collect();
                buf.drain(..2);
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
    pub async fn spawn(&self, argv: Vec<String>, cwd: String, env: Map<String, Value>) -> Result<String> {
        let body = json!({
            "argv": argv,
            "cwd": cwd,
            "cols": 120,
            "rows": 32,
            "env": Value::Object(env),
        });
        let resp = self.post_json("/sessions/spawn", &body).await?;
        resp.get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("spawn response missing session_id"))
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
        while let Some(i) = find(&buf, b"\n\n") {
            let frame: Vec<u8> = buf.drain(..i).collect();
            buf.drain(..2);
            let text = String::from_utf8_lossy(&frame);
            if text.lines().any(|l| l.starts_with("data:")) {
                let _ = tx.send(DaemonEvent::Changed);
            }
        }
    }
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
            .spawn(vec!["cat".into()], "/tmp".into(), Map::new())
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
            let usage = cm
                .transcript(&a.session_id)
                .await
                .ok()
                .and_then(|t| crate::usage::from_transcript(&t));
            let extra = match usage {
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

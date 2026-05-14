//! Minimal SSE consumer. Streams `data: {...}` frames from the daemon's
//! `/events` endpoint, decodes them into `SessionUpdate`, and forwards
//! to the TUI app over an mpsc channel.
//!
//! Reconnects with a short backoff on disconnect so the UI doesn't go
//! silently stale when the daemon restarts.

use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc;

use super::app::{parse_sse_data, AppEvent};

pub async fn run(url: String, tx: mpsc::UnboundedSender<AppEvent>) {
    let client = reqwest::Client::new();
    loop {
        let req = client
            .get(&url)
            .header("accept", "text/event-stream")
            .send()
            .await;
        let resp = match req {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let _ = tx.send(AppEvent::Toast(format!("sse {}", r.status())));
                let _ = tx.send(AppEvent::SseDisconnected);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Err(_) => {
                let _ = tx.send(AppEvent::SseDisconnected);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let _ = tx.send(AppEvent::SseConnected);
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();

        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else { break };
            buf.extend_from_slice(&chunk);
            while let Some(idx) = find_double_newline(&buf) {
                let frame: Vec<u8> = buf.drain(..idx).collect();
                buf.drain(..2); // consume the trailing \n\n
                if let Some(update) = parse_sse_frame(&frame) {
                    let _ = tx.send(AppEvent::Update(update));
                }
            }
        }

        let _ = tx.send(AppEvent::SseDisconnected);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn find_double_newline(haystack: &[u8]) -> Option<usize> {
    haystack.windows(2).position(|w| w == b"\n\n")
}

fn parse_sse_frame(bytes: &[u8]) -> Option<super::app::SessionUpdate> {
    let text = std::str::from_utf8(bytes).ok()?;
    // SSE frame: "event: name\ndata: {...}". Pick out the `data:` line(s).
    let mut data: String = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    parse_sse_data(&data)
}

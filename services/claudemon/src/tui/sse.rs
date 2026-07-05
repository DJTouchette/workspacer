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
            for frame in drain_sse_frames(&mut buf) {
                if let Some(update) = parse_sse_frame(&frame) {
                    let _ = tx.send(AppEvent::Update(Box::new(update)));
                }
            }
        }

        let _ = tx.send(AppEvent::SseDisconnected);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Drain all complete SSE frames (separated by `\n\n`) from `buf`, returning
/// them as a `Vec` of raw frame byte vectors. `buf` is left with any trailing
/// incomplete frame. Behaviour-preserving: identical to the previous inline
/// `while let Some(idx) = find_double_newline(&buf)` loop.
pub(crate) fn drain_sse_frames(buf: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    while let Some(idx) = find_double_newline(buf) {
        let frame: Vec<u8> = buf.drain(..idx).collect();
        buf.drain(..2); // consume the trailing \n\n
        frames.push(frame);
    }
    frames
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
}

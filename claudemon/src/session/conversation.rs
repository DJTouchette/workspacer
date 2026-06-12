//! Daemon-owned conversation parsing.
//!
//! Tails each session's JSONL transcript incrementally (byte offset + partial
//! line carry) and broadcasts structured `ConversationDelta`s, so clients
//! (Workspacer's Electron main, the web mirror) render the conversation
//! without ever re-reading or re-parsing the transcript themselves.
//!
//! This is the *content* channel. Hooks remain the *control* channel (mode,
//! approvals, questions); the statusLine stream remains the telemetry channel.

use std::io::SeekFrom;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::broadcast;

use super::transcript::{blocks, flatten_tool_result, Block};
use super::{SessionMode, SessionStore};

const CONV_BROADCAST_CAPACITY: usize = 1024;
const TAIL_INTERVAL: Duration = Duration::from_millis(400);
/// Keep draining a stopped session's transcript briefly — the final
/// assistant message can flush to disk after the Stop/SessionEnd hook fires.
const STOPPED_DRAIN_SECS: i64 = 30;

/// One structured event parsed out of the transcript, in timeline order.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationItem {
    UserMessage {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    AssistantText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    /// Token usage riding on an assistant message. `message_id` lets clients
    /// dedup the per-block repetition of one streamed message.
    Usage {
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        usage: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationDelta {
    pub session_id: String,
    /// Per-session sequence of the *last* item in this delta (1-based, counts
    /// items since the log was (re)built). Clients detect gaps by checking
    /// `seq == last_seq + items.len()` and resync from the snapshot endpoint.
    pub seq: u64,
    /// True when the log was rebuilt from scratch (transcript replaced or
    /// truncated). Clients must discard prior state and adopt `items` wholesale.
    pub reset: bool,
    pub items: Vec<ConversationItem>,
}

#[derive(Default)]
struct TailLog {
    path: String,
    offset: u64,
    /// Bytes after the last newline — kept as raw bytes so a UTF-8 sequence
    /// split across reads survives intact.
    partial: Vec<u8>,
    items: Vec<ConversationItem>,
    seq: u64,
}

/// Shared handle: the tailer task writes, API handlers read/subscribe.
#[derive(Clone)]
pub struct ConversationStore {
    logs: Arc<DashMap<String, TailLog>>,
    tx: broadcast::Sender<ConversationDelta>,
}

impl ConversationStore {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CONV_BROADCAST_CAPACITY);
        Self {
            logs: Arc::new(DashMap::new()),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ConversationDelta> {
        self.tx.subscribe()
    }

    /// Full parsed history + current seq for one session (for clients joining
    /// mid-session or recovering from a missed delta).
    pub fn snapshot(&self, session_id: &str) -> Option<(u64, Vec<ConversationItem>)> {
        self.logs
            .get(session_id)
            .map(|l| (l.seq, l.items.clone()))
    }
}

impl Default for ConversationStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the background tail loop: every tick, read whatever new bytes each
/// live session's transcript has gained and broadcast the parsed items.
pub fn spawn_tailer(sessions: SessionStore, conv: ConversationStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TAIL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            for state in sessions.list() {
                let Some(path) = state.transcript_path.clone() else {
                    continue;
                };
                if state.mode == SessionMode::Stopped {
                    let age = time::OffsetDateTime::now_utc() - state.updated_at;
                    if age.whole_seconds() > STOPPED_DRAIN_SECS {
                        continue;
                    }
                }
                if let Err(err) = tail_one(&conv, &state.session_id, &path).await {
                    tracing::debug!(?err, session = %state.session_id, "transcript tail failed");
                }
            }
        }
    });
}

/// Read new bytes for one session and broadcast a delta if anything parsed.
///
/// DashMap guards are never held across an await: we copy the cursor out,
/// do the file I/O, then re-acquire to commit. Only the tailer task mutates
/// logs, so the copy can't go stale.
async fn tail_one(conv: &ConversationStore, session_id: &str, path: &str) -> std::io::Result<()> {
    let (mut offset, mut partial, mut reset) = match conv.logs.get(session_id) {
        Some(l) if l.path == path => (l.offset, l.partial.clone(), false),
        // New session, or claude switched transcript files (e.g. resume).
        _ => (0, Vec::new(), true),
    };

    let len = tokio::fs::metadata(path).await?.len();
    if len < offset {
        // Truncated/replaced in place — rebuild from the top.
        offset = 0;
        partial.clear();
        reset = true;
    }
    if len == offset && !reset {
        return Ok(());
    }

    let mut buf = partial;
    if len > offset {
        let mut file = tokio::fs::File::open(path).await?;
        file.seek(SeekFrom::Start(offset)).await?;
        // Bound the read to the length we statted so `offset` stays consistent
        // even if the file grows while we read.
        let mut chunk = Vec::with_capacity((len - offset) as usize);
        (&mut file).take(len - offset).read_to_end(&mut chunk).await?;
        offset += chunk.len() as u64;
        buf.extend_from_slice(&chunk);
    }

    // Split off the trailing partial line (bytes after the last newline).
    let new_partial = match buf.iter().rposition(|&b| b == b'\n') {
        Some(idx) => buf.split_off(idx + 1),
        None => std::mem::take(&mut buf),
    };

    let complete = String::from_utf8_lossy(&buf);
    let mut items = Vec::new();
    for line in complete.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            items.extend(items_from_row(&value));
        }
    }

    let delta = {
        let mut entry = conv.logs.entry(session_id.to_string()).or_default();
        if reset {
            entry.items.clear();
            entry.seq = 0;
        }
        entry.path = path.to_string();
        entry.offset = offset;
        entry.partial = new_partial;
        if items.is_empty() && !reset {
            return Ok(());
        }
        entry.seq += items.len() as u64;
        entry.items.extend(items.iter().cloned());
        ConversationDelta {
            session_id: session_id.to_string(),
            seq: entry.seq,
            reset,
            items,
        }
    };
    let _ = conv.tx.send(delta);
    Ok(())
}

/// Parse one transcript row into zero or more conversation items.
///
/// Mirrors what clients used to derive themselves: user text, assistant text
/// per content block, tool_use starts, tool_results joined by id, and usage.
/// Thinking blocks and meta rows are skipped.
pub fn items_from_row(value: &Value) -> Vec<ConversationItem> {
    let mut out = Vec::new();
    if value.get("isMeta").and_then(Value::as_bool).unwrap_or(false) {
        return out;
    }
    let row_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let ts = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(msg) = value.get("message") else {
        return out;
    };

    match row_type {
        "user" => {
            let content = msg.get("content").unwrap_or(&Value::Null);
            let bs = blocks(content);
            let has_tool_result = bs.iter().any(|b| matches!(b, Block::ToolResult { .. }));
            if has_tool_result {
                // tool_result rows are API plumbing, not user messages —
                // surface them as results joined to their tool calls.
                for b in &bs {
                    if let Block::ToolResult {
                        content,
                        is_error,
                        tool_use_id: Some(tid),
                    } = b
                    {
                        out.push(ConversationItem::ToolResult {
                            tool_use_id: (*tid).to_string(),
                            content: flatten_tool_result(content),
                            is_error: *is_error,
                            timestamp: ts.clone(),
                        });
                    }
                }
            } else {
                let text = bs
                    .iter()
                    .filter_map(|b| match b {
                        Block::Text { text } => Some(*text),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    out.push(ConversationItem::UserMessage {
                        text,
                        timestamp: ts,
                    });
                }
            }
        }
        "assistant" => {
            if let Some(usage) = msg.get("usage") {
                out.push(ConversationItem::Usage {
                    model: msg.get("model").and_then(Value::as_str).map(str::to_owned),
                    usage: usage.clone(),
                    message_id: msg
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| value.get("uuid").and_then(Value::as_str).map(str::to_owned)),
                });
            }
            for b in blocks(msg.get("content").unwrap_or(&Value::Null)) {
                match b {
                    Block::Text { text } if !text.trim().is_empty() => {
                        out.push(ConversationItem::AssistantText {
                            text: text.trim().to_string(),
                            timestamp: ts.clone(),
                        });
                    }
                    Block::ToolUse { name, input, id } => {
                        out.push(ConversationItem::ToolUse {
                            id: id.map(str::to_owned).unwrap_or_default(),
                            name: name.to_string(),
                            input: input.clone(),
                            timestamp: ts.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_text_row_yields_user_message() {
        let row = json!({
            "type": "user",
            "timestamp": "2026-06-12T10:00:00Z",
            "message": { "role": "user", "content": "hello there" }
        });
        let items = items_from_row(&row);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::UserMessage { text, timestamp } => {
                assert_eq!(text, "hello there");
                assert_eq!(timestamp.as_deref(), Some("2026-06-12T10:00:00Z"));
            }
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_rows_are_results_not_user_messages() {
        let row = json!({
            "type": "user",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "tu_1", "content": "42 lines", "is_error": false }
            ]}
        });
        let items = items_from_row(&row);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::ToolResult { tool_use_id, content, is_error, .. } => {
                assert_eq!(tool_use_id, "tu_1");
                assert_eq!(content, "42 lines");
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn assistant_row_interlaces_usage_text_and_tool_use() {
        let row = json!({
            "type": "assistant",
            "timestamp": "2026-06-12T10:00:01Z",
            "message": {
                "role": "assistant",
                "id": "msg_1",
                "model": "claude-fable-5",
                "usage": { "input_tokens": 10, "output_tokens": 5 },
                "content": [
                    { "type": "thinking", "thinking": "hmm" },
                    { "type": "text", "text": "I'll read the file." },
                    { "type": "tool_use", "id": "tu_2", "name": "Read", "input": { "file_path": "/a.rs" } }
                ]
            }
        });
        let items = items_from_row(&row);
        assert_eq!(items.len(), 3, "usage + text + tool_use (thinking skipped)");
        assert!(matches!(&items[0], ConversationItem::Usage { model: Some(m), message_id: Some(id), .. } if m == "claude-fable-5" && id == "msg_1"));
        assert!(matches!(&items[1], ConversationItem::AssistantText { text, .. } if text == "I'll read the file."));
        assert!(matches!(&items[2], ConversationItem::ToolUse { id, name, .. } if id == "tu_2" && name == "Read"));
    }

    #[test]
    fn meta_and_summary_rows_are_skipped() {
        assert!(items_from_row(&json!({ "type": "user", "isMeta": true, "message": { "content": "x" } })).is_empty());
        assert!(items_from_row(&json!({ "type": "summary", "summary": "..." })).is_empty());
    }

    #[tokio::test]
    async fn tail_picks_up_appends_and_carries_partial_lines() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("claudemon-tail-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let path_str = path.to_string_lossy().to_string();

        let conv = ConversationStore::new();
        let mut rx = conv.subscribe();

        // First write: one complete row + the head of a second row.
        let row1 = r#"{"type":"user","message":{"role":"user","content":"first"}}"#;
        let row2 = r#"{"type":"user","message":{"role":"user","content":"second"}}"#;
        {
            let mut f = std::fs::File::create(&path).unwrap();
            write!(f, "{row1}\n{}", &row2[..20]).unwrap();
        }
        tail_one(&conv, "s1", &path_str).await.unwrap();
        let d1 = rx.try_recv().expect("first delta");
        assert!(d1.reset);
        assert_eq!(d1.seq, 1);
        assert_eq!(d1.items.len(), 1, "partial second row must not parse yet");

        // Second write: the rest of row 2.
        {
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            write!(f, "{}\n", &row2[20..]).unwrap();
        }
        tail_one(&conv, "s1", &path_str).await.unwrap();
        let d2 = rx.try_recv().expect("second delta");
        assert!(!d2.reset);
        assert_eq!(d2.seq, 2);
        assert!(matches!(&d2.items[0], ConversationItem::UserMessage { text, .. } if text == "second"));

        // Snapshot reflects the whole log.
        let (seq, items) = conv.snapshot("s1").unwrap();
        assert_eq!(seq, 2);
        assert_eq!(items.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}

use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_ROWS: usize = 2_000;
pub const MAX_TRANSCRIPT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ROW_BYTES: usize = 64 * 1024;

/// Deliberately project only sidebar/control fields: rich snapshots may carry
/// megabytes of conversation, which belongs to the selected transcript alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub id: String,
    pub label: String,
    pub cwd: String,
    pub state: String,
    pub transport: String,
    pub approval: Option<Value>,
    pub questions: Option<Value>,
}

impl Session {
    pub fn id_of(value: &Value) -> Option<&str> {
        value
            .get("sessionId")
            .or_else(|| value.get("session_id"))?
            .as_str()
            .filter(|s| !s.is_empty())
    }

    pub fn merge(&mut self, value: &Value) {
        for (target, names) in [
            (&mut self.id, &["sessionId", "session_id"][..]),
            (&mut self.label, &["label", "customName"][..]),
            (&mut self.cwd, &["cwd"][..]),
            (&mut self.state, &["mode", "ambientState"][..]),
            (&mut self.transport, &["transport"][..]),
        ] {
            if let Some(s) = names
                .iter()
                .find_map(|name| value.get(name).and_then(Value::as_str))
            {
                *target = s.to_owned();
            }
        }
        if value.get("status").and_then(Value::as_str) == Some("ended") {
            self.state = "stopped".into();
        }
        for (target, name) in [
            (&mut self.approval, "pendingApproval"),
            (&mut self.questions, "pendingQuestions"),
        ] {
            if let Some(v) = value.get(name) {
                *target = (!v.is_null()).then(|| v.clone());
            }
        }
        if let Some(pending) = value.get("pending") {
            if value.get("pendingApproval").is_none() {
                self.approval = (pending["kind"] == "approval").then(|| pending.clone());
            }
            if value.get("pendingQuestions").is_none() {
                self.questions =
                    (pending["kind"] == "question").then(|| pending["questions"].clone());
            }
        }
    }

    pub fn title(&self) -> &str {
        if !self.label.is_empty() {
            &self.label
        } else if !self.cwd.is_empty() {
            self.cwd
                .rsplit(['/', '\\'])
                .find(|s| !s.is_empty())
                .unwrap_or(&self.cwd)
        } else {
            &self.id
        }
    }

    pub fn stopped(&self) -> bool {
        self.state == "stopped" || self.state == "ended"
    }

    pub fn working(&self) -> bool {
        !self.stopped()
            && self.approval.is_none()
            && self.questions.is_none()
            && matches!(
                self.state.as_str(),
                "responding" | "working" | "thinking" | "streaming" | "running"
            )
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Item {
    #[serde(default, alias = "updatedAt")]
    pub timestamp: Option<String>,
    #[serde(default, alias = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub steps: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConversationSnapshot {
    pub seq: u64,
    #[serde(default)]
    pub first_seq: u64,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Delta {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub reset: bool,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: u64,
    pub role: &'static str,
    pub text: String,
    pub truncated: bool,
    pub tool: Option<ToolCall>,
    pub timestamp_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub result: Option<String>,
    pub is_error: bool,
    pub completed_at_ms: Option<i64>,
}

impl Row {
    pub fn same_content(&self, other: &Self) -> bool {
        self.role == other.role
            && self.text == other.text
            && self.truncated == other.truncated
            && self.tool == other.tool
            && self.timestamp_ms == other.timestamp_ms
    }

    fn retained_bytes(&self) -> usize {
        self.text.len()
            + self.tool.as_ref().map_or(0, |t| {
                t.id.len() + t.name.len() + t.result.as_ref().map_or(0, String::len)
            })
    }

    pub fn copy_text(&self) -> String {
        match &self.tool {
            Some(tool) => format!(
                "{}\n{}{}",
                tool.name,
                self.text,
                tool.result
                    .as_ref()
                    .map(|r| format!("\n\n{r}"))
                    .unwrap_or_default()
            ),
            None => self.text.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Changed,
    Unchanged,
    Gap,
}

/// Sequence numbers count raw events, not coalesced rows. Only a full snapshot
/// or a contiguous delta may advance the cursor. This type has no UI/runtime.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    pub rows: VecDeque<Arc<Row>>,
    pub seq: Option<u64>,
    pub bytes: usize,
    pub omitted: bool,
    pub revision: u64,
    next_key: u64,
}

impl Transcript {
    pub fn snapshot(&mut self, snapshot: ConversationSnapshot) {
        let previous = std::mem::take(&mut self.rows);
        self.bytes = 0;
        self.omitted = snapshot.first_seq > 1;
        for item in snapshot.items {
            self.push(item, false);
        }
        // Reseeding identical history must not discard text-view state or
        // measured row heights. Changed content still gets a fresh Arc.
        for (row, old) in self.rows.iter_mut().zip(&previous) {
            if row.same_content(old) {
                *row = old.clone();
            } else if row.role == old.role
                && match (&row.tool, &old.tool) {
                    (Some(a), Some(b)) => !a.id.is_empty() && a.id == b.id,
                    (None, None) => row.text.starts_with(&old.text),
                    _ => false,
                }
            {
                Arc::make_mut(row).key = old.key;
            }
        }
        self.seq = Some(snapshot.seq);
        self.revision += 1;
    }

    pub fn delta(&mut self, delta: Delta, streaming: bool) -> Fold {
        if delta.ready {
            return Fold::Unchanged;
        }
        if delta.reset {
            self.snapshot(ConversationSnapshot {
                seq: delta.seq,
                first_seq: 0,
                items: delta.items,
            });
            return Fold::Changed;
        }
        let Some(seq) = self.seq else {
            return Fold::Gap;
        };
        if delta.seq <= seq {
            return Fold::Unchanged;
        }
        if delta.seq.checked_sub(delta.items.len() as u64) != Some(seq) {
            return Fold::Gap;
        }
        for item in delta.items {
            self.push(item, streaming);
        }
        self.seq = Some(delta.seq);
        self.revision += 1;
        Fold::Changed
    }

    fn push(&mut self, item: Item, streaming: bool) {
        let timestamp_ms = item
            .timestamp
            .as_deref()
            .and_then(crate::timing::parse_timestamp);
        if item.kind == "tool_result"
            && !item.tool_use_id.is_empty()
            && let Some(row) = self
                .rows
                .iter_mut()
                .rev()
                .find(|r| r.tool.as_ref().is_some_and(|t| t.id == item.tool_use_id))
        {
            self.bytes -= row.retained_bytes();
            let row = Arc::make_mut(row);
            let mut output = item.content;
            row.truncated |= truncate(&mut output, MAX_ROW_BYTES / 2);
            let tool = row.tool.as_mut().unwrap();
            tool.result = Some(output);
            tool.is_error = item.is_error;
            tool.completed_at_ms = timestamp_ms.or(tool.completed_at_ms);
            self.bytes += row.retained_bytes();
            self.trim();
            return;
        }
        let mut metadata_truncated = false;
        let tool = (item.kind == "tool_use").then(|| {
            let mut id = item.id.clone();
            let mut name = item.name.clone();
            metadata_truncated |= truncate(&mut id, 1024);
            metadata_truncated |= truncate(&mut name, 1024);
            ToolCall {
                id,
                name,
                result: None,
                is_error: false,
                completed_at_ms: None,
            }
        });
        let (role, text) = match item.kind.as_str() {
            "user_message" => ("You", item.text),
            "assistant_text" => ("Assistant", item.text),
            "tool_use" => ("Tool", item.input.to_string()),
            "tool_result" => (
                if item.is_error {
                    "Tool error"
                } else {
                    "Tool result"
                },
                item.content,
            ),
            "slash_command" => ("Command", format!("/{}", item.name)),
            "command_output" => ("Command output", item.output),
            "plan" => (
                "Plan",
                serde_json::to_string_pretty(&item.steps).unwrap_or_default(),
            ),
            _ => return,
        };
        if text.is_empty() {
            return;
        }
        // Only stream transports coalesce fragments; PTY transcript blocks are
        // already complete. Accumulated provider chunks replace their prefix.
        if streaming
            && role == "Assistant"
            && let Some(last) = self.rows.back_mut().filter(|r| r.role == role)
        {
            self.bytes -= last.text.len();
            let last = Arc::make_mut(last);
            last.timestamp_ms = last.timestamp_ms.or(timestamp_ms);
            if text.starts_with(&last.text) && !last.truncated {
                last.text = text;
            } else {
                last.text.push_str(&text);
            }
            last.truncated |= truncate(&mut last.text, MAX_ROW_BYTES);
            self.bytes += last.text.len();
        } else {
            let mut text = text;
            let truncated = truncate(
                &mut text,
                if tool.is_some() {
                    MAX_ROW_BYTES / 2
                } else {
                    MAX_ROW_BYTES
                },
            ) | metadata_truncated;
            let row = Arc::new(Row {
                key: self.next_key,
                role,
                text,
                truncated,
                tool,
                timestamp_ms,
            });
            self.bytes += row.retained_bytes();
            self.rows.push_back(row);
            self.next_key += 1;
        }
        self.trim();
    }

    fn trim(&mut self) {
        while self.rows.len() > MAX_ROWS || self.bytes > MAX_TRANSCRIPT_BYTES {
            self.bytes -= self
                .rows
                .pop_front()
                .expect("over budget implies a row")
                .retained_bytes();
            self.omitted = true;
        }
    }
}

/// Preserve a UTF-8 boundary and the newest content. The UI labels clipping.
fn truncate(text: &mut String, limit: usize) -> bool {
    if text.len() <= limit {
        return false;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    // drain() would leave the original (possibly multi-megabyte) capacity
    // allocated behind a short string, defeating the retained-text budget.
    *text = text[start..].to_owned();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assistant(text: &str) -> Item {
        Item {
            kind: "assistant_text".into(),
            text: text.into(),
            ..Default::default()
        }
    }
    fn delta(seq: u64, text: &str) -> Delta {
        Delta {
            seq,
            items: vec![assistant(text)],
            ..Default::default()
        }
    }

    #[test]
    fn coalesced_snapshot_sequence_is_not_row_count() {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 100,
            first_seq: 1,
            items: vec![assistant("hello")],
        });
        assert_eq!(t.delta(delta(101, " world"), true), Fold::Changed);
        assert_eq!(t.rows[0].text, "hello world");
        assert_eq!(t.delta(delta(101, " world"), true), Fold::Unchanged);
        assert_eq!(t.delta(delta(103, "lost"), true), Fold::Gap);
        assert_eq!(t.seq, Some(101));
    }

    #[test]
    fn reset_can_move_sequence_backwards_and_accumulated_chunks_replace() {
        let mut t = Transcript::default();
        t.delta(
            Delta {
                seq: 40,
                reset: true,
                items: vec![assistant("old")],
                ..Default::default()
            },
            true,
        );
        t.delta(
            Delta {
                seq: 1,
                reset: true,
                items: vec![assistant("new")],
                ..Default::default()
            },
            true,
        );
        t.delta(delta(2, "new text"), true);
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].text, "new text");
    }

    #[test]
    fn bounded_history_preserves_utf8_and_published_rows() {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![assistant("hello")],
        });
        let before = t.clone();
        t.delta(delta(2, " world"), true);
        assert_eq!(before.rows[0].text, "hello");
        for _ in 0..3_000 {
            t.push(assistant(&"🦀".repeat(20_000)), false);
        }
        assert!(t.rows.len() <= MAX_ROWS);
        assert!(t.bytes <= MAX_TRANSCRIPT_BYTES);
        assert!(
            t.rows
                .iter()
                .all(|r| r.text.len() <= MAX_ROW_BYTES && r.truncated)
        );
        assert!(t.omitted);
        assert!(t.rows.iter().all(|r| r.text.capacity() <= MAX_ROW_BYTES));
    }

    #[test]
    fn tool_results_join_by_id_without_changing_wire_sequence_or_old_snapshots() {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 2,
            first_seq: 1,
            items: ["a", "b"]
                .map(|id| Item {
                    kind: "tool_use".into(),
                    id: id.into(),
                    name: "Read".into(),
                    input: json!({"file_path":format!("{id}.rs")}),
                    ..Default::default()
                })
                .into(),
        });
        let before = t.clone();
        assert_eq!(
            t.delta(
                Delta {
                    seq: 4,
                    items: vec![
                        Item {
                            kind: "tool_result".into(),
                            tool_use_id: "b".into(),
                            content: "failed".into(),
                            is_error: true,
                            ..Default::default()
                        },
                        Item {
                            kind: "tool_result".into(),
                            tool_use_id: "a".into(),
                            content: String::new(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                true
            ),
            Fold::Changed
        );
        assert_eq!(t.seq, Some(4));
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0].tool.as_ref().unwrap().result.as_deref(), Some(""));
        assert!(t.rows[1].tool.as_ref().unwrap().is_error);
        assert!(before.rows[1].tool.as_ref().unwrap().result.is_none());
        assert_eq!(
            t.bytes,
            t.rows.iter().map(|r| r.retained_bytes()).sum::<usize>()
        );
        t.push(
            Item {
                kind: "tool_result".into(),
                tool_use_id: "missing".into(),
                content: "orphan".into(),
                ..Default::default()
            },
            false,
        );
        assert_eq!(t.rows.back().unwrap().text, "orphan");
    }

    #[test]
    fn tool_inputs_results_and_metadata_share_the_retained_history_budget() {
        let mut t = Transcript::default();
        for i in 0..100 {
            t.push(
                Item {
                    kind: "tool_use".into(),
                    id: i.to_string(),
                    name: "Write".into(),
                    input: json!({"content":"🦀".repeat(40_000)}),
                    ..Default::default()
                },
                false,
            );
            t.push(
                Item {
                    kind: "tool_result".into(),
                    tool_use_id: i.to_string(),
                    content: "🦀".repeat(40_000),
                    ..Default::default()
                },
                false,
            );
        }
        assert!(t.bytes <= MAX_TRANSCRIPT_BYTES);
        assert!(t.omitted);
        assert!(t.rows.iter().all(|r| r.truncated
            && r.text.capacity() <= MAX_ROW_BYTES / 2
            && r.tool.as_ref().unwrap().result.as_ref().unwrap().capacity() <= MAX_ROW_BYTES / 2));
        assert_eq!(
            t.bytes,
            t.rows.iter().map(|r| r.retained_bytes()).sum::<usize>()
        );
    }

    #[test]
    fn reseeding_history_preserves_unchanged_rows_and_streaming_identity() {
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 2,
            first_seq: 1,
            items: vec![assistant("First"), assistant("Second")],
        });
        let before = t.clone();
        t.snapshot(ConversationSnapshot {
            seq: 3,
            first_seq: 1,
            items: vec![assistant("First"), assistant("Second grows")],
        });
        assert!(Arc::ptr_eq(&t.rows[0], &before.rows[0]));
        assert!(!Arc::ptr_eq(&t.rows[1], &before.rows[1]));
        assert_eq!(t.rows[1].key, before.rows[1].key);
        assert_eq!(before.rows[1].text, "Second");
    }

    #[test]
    fn message_times_keep_stream_start_and_join_tool_completion() {
        let start = "2026-09-27T12:00:00Z";
        let end = "2026-09-27T12:00:07Z";
        let mut t = Transcript::default();
        t.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: "assistant_text".into(),
                text: "Hello".into(),
                timestamp: Some(start.into()),
                ..Default::default()
            }],
        });
        t.delta(
            Delta {
                seq: 2,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: " world".into(),
                    timestamp: Some(end.into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            true,
        );
        assert_eq!(
            t.rows[0].timestamp_ms,
            crate::timing::parse_timestamp(start)
        );
        t.push(
            Item {
                kind: "tool_use".into(),
                id: "t1".into(),
                name: "Read".into(),
                timestamp: Some(start.into()),
                ..Default::default()
            },
            false,
        );
        t.push(
            Item {
                kind: "tool_result".into(),
                tool_use_id: "t1".into(),
                content: "result".into(),
                timestamp: Some(end.into()),
                ..Default::default()
            },
            false,
        );
        assert_eq!(
            t.rows[1].timestamp_ms,
            crate::timing::parse_timestamp(start)
        );
        assert_eq!(
            t.rows[1].tool.as_ref().unwrap().completed_at_ms,
            crate::timing::parse_timestamp(end)
        );
        let missing: Item =
            serde_json::from_value(json!({"kind":"user_message","text":"legacy"})).unwrap();
        assert!(missing.timestamp.is_none());
    }

    #[test]
    fn sparse_aliases_do_not_clear_missing_fields_but_null_clears_approval() {
        let mut s = Session::default();
        s.merge(&json!({"session_id":"s", "sessionId":"s", "label":"Build", "mode":"approval", "pendingApproval":{"toolName":"Bash"}}));
        s.merge(&json!({"sessionId":"s", "cwd":"/tmp/repo"}));
        assert!(s.approval.is_some());
        assert_eq!(s.title(), "Build");
        s.merge(&json!({"sessionId":"s", "pendingApproval":null, "mode":"input"}));
        assert!(s.approval.is_none());
    }
}

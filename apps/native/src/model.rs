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
    pub provider: String,
    pub model: String,
    pub context_window: Option<u64>,
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
            (
                &mut self.label,
                &["label", "customName", "name", "title"][..],
            ),
            (&mut self.provider, &["provider"][..]),
            (&mut self.model, &["model"][..]),
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
        if let Some(model) = value
            .pointer("/requestedSelection/model")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/settings/model").and_then(Value::as_str))
        {
            self.model = model.to_owned();
            self.context_window = if value.get("requestedSelection").is_some() {
                value
                    .pointer("/requestedSelection/contextWindow")
                    .and_then(Value::as_u64)
            } else {
                value
                    .pointer("/settings/contextWindow")
                    .and_then(Value::as_u64)
            };
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
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Item {
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

impl Item {
    pub fn display(self) -> Option<(&'static str, String)> {
        Some(match self.kind.as_str() {
            "user_message" => ("You", self.text),
            "assistant_text" => ("Assistant", self.text),
            "tool_use" => ("Tool", format!("{}\n{}", self.name, self.input)),
            "tool_result" => (
                if self.is_error {
                    "Tool error"
                } else {
                    "Tool result"
                },
                self.content,
            ),
            "slash_command" => ("Command", format!("/{}", self.name)),
            "command_output" => ("Command output", self.output),
            "plan" => (
                "Plan",
                serde_json::to_string_pretty(&self.steps).unwrap_or_default(),
            ),
            _ => return None,
        })
    }
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
        self.rows.clear();
        self.bytes = 0;
        self.omitted = snapshot.first_seq > 1;
        for item in snapshot.items {
            self.push(item, false);
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
        let Some((role, text)) = item.display() else {
            return;
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
            if text.starts_with(&last.text) && !last.truncated {
                last.text = text;
            } else {
                last.text.push_str(&text);
            }
            last.truncated |= truncate(&mut last.text, MAX_ROW_BYTES);
            self.bytes += last.text.len();
        } else {
            let mut text = text;
            let truncated = truncate(&mut text, MAX_ROW_BYTES);
            self.bytes += text.len();
            self.rows.push_back(Arc::new(Row {
                key: self.next_key,
                role,
                text,
                truncated,
            }));
            self.next_key += 1;
        }
        while self.rows.len() > MAX_ROWS || self.bytes > MAX_TRANSCRIPT_BYTES {
            self.bytes -= self
                .rows
                .pop_front()
                .expect("over budget implies a row")
                .text
                .len();
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

//! Wire and domain types, mirroring claudemon's REST API (`GET /sessions`,
//! `/sessions/:id`, `/sessions/:id/transcript`). claudemon is the source of
//! truth for a standalone TUI — the hub-bus capabilities the `/remote` client
//! uses are registered by the Electron app and absent when it isn't running.

use serde::Deserialize;
use serde_json::Value;

/// One live session, as returned by claudemon's `GET /sessions`.
#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// One of: unknown, input, responding, approval, question, stopped.
    #[serde(default)]
    pub mode: String,
    /// What Claude is blocked on, if anything. `skip_deserializing` on the
    /// daemon means it can be absent; we tolerate that.
    #[serde(default)]
    pub pending: Option<Pending>,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub last_event: Option<String>,
    /// Token/cost/context derived from the transcript (not part of the
    /// `/sessions` payload — filled in after listing). See [`crate::usage`].
    #[serde(skip)]
    pub usage: Option<crate::usage::Usage>,
}

/// Whatever Claude is waiting on, tagged by `kind` (matches claudemon's enum).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pending {
    Approval {
        #[serde(default)]
        tool: Option<String>,
        /// The raw permission-request hook payload, shown so the user can see
        /// exactly what the tool would do before approving.
        #[serde(default)]
        raw: Value,
    },
    Question {
        #[serde(default)]
        questions: Vec<Question>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Question {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl Agent {
    pub fn state(&self) -> &str {
        if self.mode.is_empty() {
            "unknown"
        } else {
            &self.mode
        }
    }

    /// True when the agent needs the user: an approval, a question, or a chat
    /// prompt awaiting the next message (matches the `/remote` semantics).
    pub fn is_waiting(&self) -> bool {
        matches!(self.mode.as_str(), "input" | "approval" | "question")
    }

    /// True when the agent is actively producing a turn.
    pub fn is_busy(&self) -> bool {
        self.mode == "responding"
    }

    pub fn cwd_str(&self) -> &str {
        self.cwd.as_deref().unwrap_or("")
    }

    /// Last path segments of the cwd — what the sidebar shows as a name.
    pub fn short_cwd(&self) -> String {
        let cwd = self.cwd_str();
        if cwd.is_empty() {
            return "(session)".into();
        }
        let parts: Vec<&str> = cwd.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
        if parts.len() <= 2 {
            cwd.to_string()
        } else {
            format!("…/{}", parts[parts.len() - 2..].join("/"))
        }
    }

    /// The pending approval as `(tool name, raw hook input)`, if any.
    pub fn approval(&self) -> Option<(&str, &Value)> {
        match &self.pending {
            Some(Pending::Approval { tool, raw, .. }) => {
                Some((tool.as_deref().unwrap_or("tool"), raw))
            }
            _ => None,
        }
    }

    pub fn questions(&self) -> Option<&[Question]> {
        match &self.pending {
            Some(Pending::Question { questions, .. }) => Some(questions),
            _ => None,
        }
    }

    pub fn has_question(&self) -> bool {
        self.questions().is_some_and(|q| !q.is_empty())
    }
}

/// A rendered transcript turn — a role plus its text/tool parts, after the
/// noise (tool results, thinking, system reminders) has been filtered out.
#[derive(Debug, Clone)]
pub struct Turn {
    pub role: Role,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum Part {
    Text(String),
    Tool { name: String, summary: String },
}

/// Parse claudemon's transcript payload (`{ messages: [...] }`) into renderable
/// turns. Content is a plain string or an array of content blocks.
pub fn turns_from_transcript(tx: &Value) -> Vec<Turn> {
    let messages = tx
        .get("messages")
        .or_else(|| tx.get("entries"))
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for m in &messages {
        let role = match m.get("role").and_then(|r| r.as_str()) {
            Some("assistant") => Role::Assistant,
            Some("user") => Role::User,
            _ => continue,
        };

        let blocks: Vec<Value> = match m.get("content") {
            Some(Value::Array(a)) => a.clone(),
            other => vec![serde_json::json!({
                "type": "text",
                "text": other.and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })],
        };

        let mut parts = Vec::new();
        for b in &blocks {
            let ty = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match ty {
                "text" => {
                    let text = b.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
                    if text.is_empty() || is_meta_noise(&text) {
                        continue;
                    }
                    parts.push(Part::Text(text));
                }
                "tool_use" if role == Role::Assistant => {
                    let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool").to_string();
                    let summary = tool_summary(b.get("input"));
                    parts.push(Part::Tool { name, summary });
                }
                _ => {}
            }
        }
        if !parts.is_empty() {
            out.push(Turn { role, parts });
        }
    }
    out
}

/// Slash-command echoes and injected reminders aren't real conversation.
fn is_meta_noise(text: &str) -> bool {
    const TAGS: [&str; 4] = [
        "<local-command",
        "<command-name",
        "<command-message",
        "<system-reminder",
    ];
    TAGS.iter().any(|t| text.starts_with(t))
}

/// A one-line gist of a tool call, drawn from whichever well-known field is
/// present.
fn tool_summary(input: Option<&Value>) -> String {
    let Some(obj) = input.and_then(|v| v.as_object()) else {
        return String::new();
    };
    const KEYS: [&str; 8] = [
        "file_path", "path", "command", "pattern", "query", "url", "prompt", "description",
    ];
    for k in KEYS {
        if let Some(s) = obj.get(k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return truncate(s, 64);
            }
        }
    }
    String::new()
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_sessions_list_shape() {
        // Exactly what claudemon's GET /sessions returns.
        let json = serde_json::json!([{
            "session_id": "abc",
            "cwd": "/home/u/proj",
            "mode": "responding",
            "pending": null,
            "started_at": "2026-06-04T03:00:00Z",
            "updated_at": "2026-06-04T03:00:10Z",
            "tool_calls": 3,
            "last_event": "PreToolUse"
        }]);
        let agents: Vec<Agent> = serde_json::from_value(json).unwrap();
        assert_eq!(agents.len(), 1);
        let a = &agents[0];
        assert_eq!(a.session_id, "abc");
        assert_eq!(a.state(), "responding");
        assert!(a.is_busy() && !a.is_waiting());
        assert_eq!(a.short_cwd(), "…/u/proj");
        assert!(a.approval().is_none() && a.questions().is_none());
    }

    #[test]
    fn parses_pending_approval() {
        let json = serde_json::json!({
            "session_id": "x", "mode": "approval",
            "pending": {"kind": "approval", "tool": "Bash",
                        "raw": {"tool_input": {"command": "ls -la"}}}
        });
        let a: Agent = serde_json::from_value(json).unwrap();
        assert!(a.is_waiting());
        let (tool, raw) = a.approval().expect("approval present");
        assert_eq!(tool, "Bash");
        assert_eq!(raw["tool_input"]["command"], "ls -la");
    }

    #[test]
    fn parses_pending_question() {
        let json = serde_json::json!({
            "session_id": "x", "mode": "question",
            "pending": {"kind": "question", "questions": [
                {"question": "Which?", "header": "Pick", "multi_select": false,
                 "options": [{"label": "A"}, {"label": "B", "description": "the b one"}]}
            ]}
        });
        let a: Agent = serde_json::from_value(json).unwrap();
        assert!(a.has_question());
        let qs = a.questions().unwrap();
        assert_eq!(qs[0].options.len(), 2);
        assert_eq!(qs[0].options[1].description.as_deref(), Some("the b one"));
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// What Claude Code is doing right now, as far as the daemon can tell.
///
/// Driven by hook events. `Approval` and `Question` are both paused states;
/// they override `Responding` because while a picker is up, Claude is waiting
/// on the user — it is not actively working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// No hook has fired yet. We're in TUI startup, OAuth, or first-run
    /// setup. The wrapper can still read/write bytes, but mode-specific
    /// endpoints don't know what to do.
    Unknown,
    /// Chat prompt is up. Ready to receive a user message.
    Input,
    /// Claude is producing a turn — streaming, thinking, or running a
    /// tool that didn't need approval.
    Responding,
    /// `PermissionRequest`-style pause. Claude is waiting on a yes/no
    /// (or yes/no/always) decision before running a tool.
    Approval,
    /// `AskUserQuestion` tool call is open. Claude is asking the user a
    /// free-form question with one or more multiple-choice options.
    Question,
    /// Session has ended.
    Stopped,
}

impl Default for SessionMode {
    fn default() -> Self {
        SessionMode::Unknown
    }
}

/// One question Claude is asking the user (mirrors the `AskUserQuestion`
/// tool input).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default)]
    pub options: Vec<PendingOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Whatever Claude is waiting on right now. Mirrors `mode` so clients
/// don't have to read two fields to know what UI to show.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pending {
    /// `PermissionRequest` — Claude wants permission to run a tool.
    Approval {
        tool: Option<String>,
        summary: Option<String>,
        raw: Value,
    },
    /// `AskUserQuestion` PreToolUse — assistant is asking the user.
    Question {
        questions: Vec<PendingQuestion>,
        raw: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub cwd: Option<String>,
    pub mode: SessionMode,
    #[serde(skip_deserializing)]
    pub pending: Option<Pending>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub tool_calls: u64,
    pub last_event: Option<String>,
}

impl SessionState {
    pub fn new(session_id: String, cwd: Option<String>) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            session_id,
            cwd,
            mode: SessionMode::Unknown,
            pending: None,
            started_at: now,
            updated_at: now,
            tool_calls: 0,
            last_event: None,
        }
    }

    pub fn apply(&mut self, event: &HookEvent) {
        self.updated_at = OffsetDateTime::now_utc();
        self.last_event = Some(event.event.clone());
        if self.cwd.is_none() {
            self.cwd = event.cwd.clone();
        }

        match event.event.as_str() {
            "SessionStart" => {
                self.mode = SessionMode::Input;
                self.pending = None;
            }
            "SessionEnd" => {
                self.mode = SessionMode::Stopped;
                self.pending = None;
            }

            "UserPromptSubmit" => {
                self.mode = SessionMode::Responding;
                self.pending = None;
            }

            "PreToolUse" => {
                self.tool_calls = self.tool_calls.saturating_add(1);
                let tool = event
                    .payload
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if tool == "AskUserQuestion" {
                    let raw = event
                        .payload
                        .get("tool_input")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let questions = raw
                        .get("questions")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Vec<PendingQuestion>>(v).ok())
                        .unwrap_or_default();
                    self.mode = SessionMode::Question;
                    self.pending = Some(Pending::Question { questions, raw });
                } else if self.mode != SessionMode::Approval
                    && self.mode != SessionMode::Question
                {
                    self.mode = SessionMode::Responding;
                }
            }
            "PostToolUse" | "PostToolUseFailure" => {
                self.mode = SessionMode::Responding;
                self.pending = None;
            }

            "PermissionRequest" => {
                let tool = event
                    .payload
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let summary = event
                    .payload
                    .get("summary")
                    .or_else(|| event.payload.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let raw = Value::Object(event.payload.clone());
                self.mode = SessionMode::Approval;
                self.pending = Some(Pending::Approval { tool, summary, raw });
            }

            "SubagentStart" | "SubagentStop" => {
                if self.mode != SessionMode::Approval
                    && self.mode != SessionMode::Question
                {
                    self.mode = SessionMode::Responding;
                }
            }

            "Stop" => {
                self.mode = SessionMode::Input;
                self.pending = None;
            }

            "Notification" => {}
            _ => {}
        }
    }
}

/// Raw inbound hook event. Common fields are typed; everything else lives
/// in `payload` so we don't have to track Claude Code's hook schema lock-step.
///
/// Real Claude Code hooks emit `hook_event_name`; our own synthetic events
/// and curl-based tests use `event`. Accept both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEvent {
    #[serde(alias = "hook_event_name")]
    pub event: String,
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub timestamp: Option<OffsetDateTime>,
    #[serde(flatten)]
    pub payload: serde_json::Map<String, Value>,
}

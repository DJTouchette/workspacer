use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// What Claude Code is doing right now, as far as the daemon can tell.
///
/// Driven by hook events. `Approval` overrides `Responding` because while
/// a permission picker is up, Claude is paused — it's not actively working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// No hook has fired yet. We're in early TUI startup, OAuth, or
    /// first-run setup screens. The wrapper can read/write bytes, but
    /// mode-specific endpoints don't know what to do.
    Unknown,
    /// Chat prompt is up. Ready to receive a user message.
    Input,
    /// Claude is producing a turn — streaming, thinking, or running a
    /// tool that didn't need approval.
    Responding,
    /// A `PermissionRequest` was received. Claude is paused at a picker.
    Approval,
    /// Session has ended.
    Stopped,
}

impl Default for SessionMode {
    fn default() -> Self {
        SessionMode::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub cwd: Option<String>,
    pub mode: SessionMode,
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
            // Lifecycle: SessionStart drops us at the chat prompt.
            "SessionStart" => self.mode = SessionMode::Input,
            "SessionEnd" => self.mode = SessionMode::Stopped,

            // User submitted a turn — Claude is now working.
            "UserPromptSubmit" => self.mode = SessionMode::Responding,

            // Tool calls. Don't downgrade Approval back to Responding here —
            // PermissionRequest may have fired before PreToolUse arrives via
            // a different code path, and we want Approval to be sticky until
            // the picker resolves (PostToolUse / Stop).
            "PreToolUse" => {
                self.tool_calls = self.tool_calls.saturating_add(1);
                if self.mode != SessionMode::Approval {
                    self.mode = SessionMode::Responding;
                }
            }
            "PostToolUse" | "PostToolUseFailure" => {
                // Tool resolved. Back to assistant streaming.
                self.mode = SessionMode::Responding;
            }

            // Permission picker is up.
            "PermissionRequest" => self.mode = SessionMode::Approval,

            // Subagent activity keeps us in Responding.
            "SubagentStart" | "SubagentStop" => {
                if self.mode != SessionMode::Approval {
                    self.mode = SessionMode::Responding;
                }
            }

            // Stop = the assistant turn is done. Back to the chat prompt.
            "Stop" => self.mode = SessionMode::Input,

            // Notifications surface info but don't change mode.
            "Notification" => {}

            _ => {}
        }
    }
}

/// Raw inbound hook event. Common fields are typed; everything else lives
/// in `payload` so we don't have to track Claude Code's hook schema lock-step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEvent {
    pub event: String,
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub timestamp: Option<OffsetDateTime>,
    #[serde(flatten)]
    pub payload: serde_json::Map<String, serde_json::Value>,
}

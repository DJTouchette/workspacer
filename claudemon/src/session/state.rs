use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Lifecycle status of a Claude Code session as far as the daemon can tell.
///
/// The daemon infers transitions from hook events. `WaitingForApproval` is
/// driven by `PermissionRequest` and the absence of a subsequent `Stop` or
/// `PostToolUse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Active,
    WaitingForApproval,
    Idle,
    Stopped,
}

impl Default for SessionStatus {
    fn default() -> Self {
        SessionStatus::Starting
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub cwd: Option<String>,
    pub status: SessionStatus,
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
            status: SessionStatus::Starting,
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
            "SessionStart" => self.status = SessionStatus::Active,
            "SessionEnd" | "Stop" => self.status = SessionStatus::Stopped,
            "PermissionRequest" => self.status = SessionStatus::WaitingForApproval,
            "PreToolUse" => {
                self.tool_calls = self.tool_calls.saturating_add(1);
                self.status = SessionStatus::Active;
            }
            "PostToolUse" | "PostToolUseFailure" => {
                if self.status == SessionStatus::WaitingForApproval {
                    self.status = SessionStatus::Active;
                }
            }
            "Notification" => {
                // Notifications don't change status by themselves.
            }
            _ => {}
        }
    }
}

/// Raw inbound hook event. Fields beyond the common ones are kept in `payload`
/// so we don't have to track Claude Code's hook schema lock-step.
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

//! Shared types for the classifier pipeline. The runtime side reads these
//! when loading inputs from SQLite and applies the actions back to the DB.

use serde::{Deserialize, Serialize};

use crate::session::HookEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Working,
    NeedsInput,
    Errored,
    Stuck,
    Done,
    Ended,
    Crashed,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Working => "working",
            SessionState::NeedsInput => "needs_input",
            SessionState::Errored => "errored",
            SessionState::Stuck => "stuck",
            SessionState::Done => "done",
            SessionState::Ended => "ended",
            SessionState::Crashed => "crashed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "working" => SessionState::Working,
            "needs_input" => SessionState::NeedsInput,
            "errored" => SessionState::Errored,
            "stuck" => SessionState::Stuck,
            "done" => SessionState::Done,
            "ended" => SessionState::Ended,
            "crashed" => SessionState::Crashed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    NeedsInput,
    Error,
    Stuck,
    Done,
    WorkingMilestone,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemKind::NeedsInput => "needs_input",
            ItemKind::Error => "error",
            ItemKind::Stuck => "stuck",
            ItemKind::Done => "done",
            ItemKind::WorkingMilestone => "working_milestone",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "needs_input" => ItemKind::NeedsInput,
            "error" => ItemKind::Error,
            "stuck" => ItemKind::Stuck,
            "done" => ItemKind::Done,
            "working_milestone" => ItemKind::WorkingMilestone,
            _ => return None,
        })
    }
}

/// Open inbox item for a session, as seen from the DB. The classifier uses
/// this to avoid creating duplicates when the same trigger fires twice.
#[derive(Debug, Clone)]
pub struct OpenItem {
    pub id: String,
    pub kind: ItemKind,
    #[allow(dead_code)]
    pub priority: i32,
}

/// What we know about a session at the moment a new event arrives.
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub state: SessionState,
    pub last_event_at_unix: i64,
}

/// One past event, distilled to the fields the classifier actually reads.
/// Keeping this narrow lets the SQL query for it stay cheap.
#[derive(Debug, Clone)]
pub struct RecentEvent {
    pub event_type: String,
    pub tool_name: Option<String>,
    pub tool_input_hash: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub struct Input {
    pub session: SessionSnapshot,
    pub recent_events: Vec<RecentEvent>,
    pub open_items: Vec<OpenItem>,
    pub event: HookEvent,
    pub event_row_id: i64,
    pub now_unix: i64,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub new_session_state: SessionState,
    pub actions: Vec<ItemAction>,
}

#[derive(Debug, Clone)]
pub enum ItemAction {
    Create(NewItem),
    Touch(String),
    ResolveAllForSession,
}

#[derive(Debug, Clone)]
pub struct NewItem {
    pub kind: ItemKind,
    pub priority: i32,
    pub triggering_event_id: Option<i64>,
    pub summary: String,
    pub next_action: Option<String>,
}

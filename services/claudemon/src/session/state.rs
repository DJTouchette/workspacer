use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// Every hook event name that claudemon handles (or is registered for).
///
/// The serialized form is PascalCase — identical to the string literals that
/// were previously used in `match event.event.as_str()` arms.  Adding a new
/// variant here is the single source of truth; `HOOK_EVENTS` in `init.rs`
/// derives the registration list from this enum via `REGISTERABLE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEventKind {
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Notification,
    Stop,
    SubagentStart,
    SubagentStop,
    PermissionRequest,
}

impl HookEventKind {
    /// Serialized (wire) name for this variant — identical to what serde would
    /// produce, but available at runtime without an allocating round-trip.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::Notification => "Notification",
            Self::Stop => "Stop",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::PermissionRequest => "PermissionRequest",
        }
    }

    /// The subset of variants that map to real Claude Code hook event names
    /// and must be registered in `~/.claude/settings.json`.
    ///
    /// `PostToolUseFailure` and `PermissionRequest` are NOT real registerable
    /// hooks — they are internal / forward-compat variants only.
    pub const REGISTERABLE: &'static [HookEventKind] = &[
        Self::SessionStart,
        Self::SessionEnd,
        Self::UserPromptSubmit,
        Self::PreToolUse,
        Self::Notification,
        Self::Stop,
        Self::SubagentStart,
        Self::SubagentStop,
    ];
}

/// What Claude Code is doing right now, as far as the daemon can tell.
///
/// Driven by hook events. `Approval` and `Question` are both paused states;
/// they override `Responding` because while a picker is up, Claude is waiting
/// on the user — it is not actively working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SessionMode {
    /// No hook has fired yet. We're in TUI startup, OAuth, or first-run
    /// setup. The wrapper can still read/write bytes, but mode-specific
    /// endpoints don't know what to do.
    #[default]
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

/// One question Claude is asking the user (mirrors the `AskUserQuestion`
/// tool input).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    /// The wire (AskUserQuestion tool input) spells this `multiSelect`;
    /// clients read the serialized snake_case form.
    #[serde(default, alias = "multiSelect")]
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

/// The agent's current plan / checklist, surfaced as first-class session state.
///
/// A single last-write-wins snapshot: whenever the agent rewrites its plan
/// (Claude Code's `TodoWrite`, Codex's `update_plan` / todo list), the whole
/// plan is replaced. Both the live SSE delta and a resync replay carry it (as a
/// `plan` conversation item), so clients keep the newest one they see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<PlanStep>,
    /// When the plan was last rewritten, in the same RFC3339 format the
    /// conversation items carry. Absent when the source event has no timestamp.
    #[serde(rename = "updatedAt", default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// One step of a [`Plan`]. Mirrors a Claude Code `TodoWrite` todo
/// (`content` / `status` / `activeForm`); Codex plan steps map onto the same
/// shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub content: String,
    pub status: PlanStatus,
    /// Present-tense label shown while the step is in progress (Claude's
    /// `activeForm`). Absent for providers that don't supply one.
    #[serde(
        rename = "activeForm",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub active_form: Option<String>,
}

/// Lifecycle of a [`PlanStep`]. Serializes to the wire vocabulary shared by
/// Claude's `TodoWrite` and Codex's plan tool (`pending` / `in_progress` /
/// `completed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

impl PlanStatus {
    /// Map a raw status string onto the enum, defensively — any unrecognized
    /// value (or a provider that spells "done" instead of "completed") lands on
    /// a sane default rather than being dropped. Aligns with the status
    /// vocabulary `transcript::summarize_todos` already recognizes.
    pub fn from_wire(s: &str) -> Self {
        match s {
            "in_progress" | "inprogress" | "in-progress" => Self::InProgress,
            "completed" | "complete" | "done" => Self::Completed,
            _ => Self::Pending,
        }
    }
}

/// Live telemetry from Claude Code's `statusLine` command.
///
/// This is a *different channel* from hooks: Claude pipes this JSON only to the
/// configured `statusLine` command (claudemon's forwarder posts a copy to
/// `/statusline`). It carries context-window %, cumulative cost, and the 5h/7d
/// rate-limit windows — none of which appear in hook payloads or the transcript.
/// Every field is optional because Claude omits some (e.g. `rate_limits` only
/// exists for Pro/Max accounts after the first API response).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusLine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_display: Option<String>,
    /// `context_window.used_percentage` (0–100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_output_tokens: Option<u64>,
    /// `cost.total_cost_usd` — Claude's own authoritative session cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// `rate_limits.five_hour.used_percentage` (0–100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour_pct: Option<f64>,
    /// Unix epoch seconds the 5h window resets at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour_resets_at: Option<i64>,
    /// `rate_limits.seven_day.used_percentage` (0–100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_pct: Option<f64>,
    /// Unix epoch seconds the 7d window resets at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_resets_at: Option<i64>,
    /// Monthly overage/credit window utilization (0–100). Sourced from Claude's
    /// stream `overage` `rateLimitType`; absent for the interactive statusLine
    /// (which carries only 5h/7d) and for providers without a monthly window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_pct: Option<f64>,
    /// Unix epoch seconds the monthly overage window resets at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_resets_at: Option<i64>,
    /// Human warning message when a window crosses its warning threshold
    /// (Claude's `status: allowed_warning`). Cleared when comfortable again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_warning: Option<String>,
    /// The monthly overage is disabled for lack of credits (Claude's
    /// `overageDisabledReason: out_of_credits`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overage_out_of_credits: Option<bool>,
    /// When the daemon received this line, so clients can age out stale data.
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub received_at: Option<OffsetDateTime>,
}

impl StatusLine {
    /// Extract the fields we care about from Claude Code's raw statusLine JSON.
    /// Tolerant of missing keys — anything absent stays `None`.
    pub fn from_claude_json(v: &Value) -> Self {
        let cw = v.get("context_window");
        let cost = v.get("cost");
        let rl = v.get("rate_limits");
        let five = rl.and_then(|r| r.get("five_hour"));
        let seven = rl.and_then(|r| r.get("seven_day"));
        // The interactive statusLine has historically carried only 5h/7d; read
        // `monthly` best-effort so we pick it up automatically if Claude ever
        // adds it, without depending on it.
        let monthly = rl.and_then(|r| r.get("monthly"));
        StatusLine {
            model_display: v
                .get("model")
                .and_then(|m| m.get("display_name"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            context_used_pct: cw
                .and_then(|c| c.get("used_percentage"))
                .and_then(Value::as_f64),
            context_window_size: cw
                .and_then(|c| c.get("context_window_size"))
                .and_then(Value::as_u64),
            total_input_tokens: cw
                .and_then(|c| c.get("total_input_tokens"))
                .and_then(Value::as_u64),
            total_output_tokens: cw
                .and_then(|c| c.get("total_output_tokens"))
                .and_then(Value::as_u64),
            cost_usd: cost
                .and_then(|c| c.get("total_cost_usd"))
                .and_then(Value::as_f64),
            five_hour_pct: five
                .and_then(|f| f.get("used_percentage"))
                .and_then(Value::as_f64),
            five_hour_resets_at: five
                .and_then(|f| f.get("resets_at"))
                .and_then(Value::as_i64),
            seven_day_pct: seven
                .and_then(|s| s.get("used_percentage"))
                .and_then(Value::as_f64),
            seven_day_resets_at: seven
                .and_then(|s| s.get("resets_at"))
                .and_then(Value::as_i64),
            monthly_pct: monthly
                .and_then(|m| m.get("used_percentage"))
                .and_then(Value::as_f64),
            monthly_resets_at: monthly
                .and_then(|m| m.get("resets_at"))
                .and_then(Value::as_i64),
            // The interactive statusLine JSON doesn't carry warning/overage
            // status — those ride the stream `rate_limit_event` only.
            rate_limit_warning: None,
            overage_out_of_credits: None,
            received_at: Some(OffsetDateTime::now_utc()),
        }
    }
}

/// How the daemon talks to a session's agent process.
///
/// `Pty` is the classic path: the agent's own TUI in a pseudo-terminal, state
/// reconstructed from hooks + screen scraping. `Stream` is the headless
/// stream-json path (`claude --print --input-format stream-json …`), where the
/// managed driver in `providers::claude_stream` owns the state machine via the
/// CLI's control protocol and hooks are enrichment-only (see
/// `SessionStore::ingest`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Pty,
    Stream,
}

/// How long a stopped session may sit idle before it's archived (hidden from
/// the default list but kept on disk and resumable). Seven days covers any
/// agent you'd realistically come back to.
pub const ARCHIVE_AFTER_SECONDS: i64 = 7 * 24 * 60 * 60;

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
    /// Absolute path to Claude's transcript JSONL, captured from the hook
    /// payload. Lets `/transcript` read the exact file even when the session id
    /// we expose (a spawn UUID) differs from Claude's own id that names the file.
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// Latest statusLine telemetry, fed by the `/statusline` forwarder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_line: Option<StatusLine>,
    /// Which agent backend drives this session: `"claude"` for the default
    /// hook + PTY sessions, or a managed adapter's name (`"codex"`, `"opencode"`,
    /// `"pi"`). Clients read this instead of guessing from spawn provenance.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Which transport drives this session: `"pty"` (default — every
    /// pre-existing row and the classic wrapper path) or `"stream"` (the
    /// headless stream-json driver). Additive and back-compatible like
    /// `provider`; serialized on every snapshot so clients can gate
    /// transport-specific affordances (e.g. no Term view for `stream`).
    #[serde(default)]
    pub transport: Transport,
    /// The agent's current plan / checklist, last-write-wins. Additive and
    /// back-compatible like `provider` — absent until the agent writes a plan,
    /// and omitted from the wire when empty. Fed by `SessionStore::set_plan`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
}

/// Serde default for [`SessionState::provider`] — the un-managed PTY path is
/// always Claude, and old persisted rows predate the field.
fn default_provider() -> String {
    "claude".to_string()
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
            transcript_path: None,
            status_line: None,
            provider: default_provider(),
            transport: Transport::default(),
            plan: None,
        }
    }

    /// Whether this session should be hidden from the default session list. A
    /// session is archived once it's stopped (no process attached) and has sat
    /// idle past [`ARCHIVE_AFTER_SECONDS`]. Live or recently-active sessions are
    /// never archived; archiving is purely a display filter — the row stays in
    /// SQLite and the session remains resumable.
    pub fn is_archived(&self, now_unix: i64) -> bool {
        self.mode == SessionMode::Stopped
            && now_unix.saturating_sub(self.updated_at.unix_timestamp()) > ARCHIVE_AFTER_SECONDS
    }

    pub fn apply(&mut self, event: &HookEvent) {
        self.updated_at = OffsetDateTime::now_utc();
        self.last_event = Some(event.event.clone());
        if self.cwd.is_none() {
            self.cwd = event.cwd.clone();
        }
        // Every Claude Code hook carries `transcript_path` — capture it so the
        // transcript endpoint reads the right file regardless of id aliasing.
        if let Some(tp) = event.payload.get("transcript_path").and_then(Value::as_str) {
            self.transcript_path = Some(tp.to_string());
        }

        // Parse the event name into a typed enum; unrecognised events are a no-op.
        let Ok(kind) =
            serde_json::from_value::<HookEventKind>(serde_json::Value::String(event.event.clone()))
        else {
            return;
        };

        match kind {
            HookEventKind::SessionStart => {
                self.mode = SessionMode::Input;
                self.pending = None;
            }
            HookEventKind::SessionEnd => {
                self.mode = SessionMode::Stopped;
                self.pending = None;
            }

            HookEventKind::UserPromptSubmit => {
                self.mode = SessionMode::Responding;
                self.pending = None;
            }

            HookEventKind::PreToolUse => {
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
                } else if self.mode != SessionMode::Approval && self.mode != SessionMode::Question {
                    self.mode = SessionMode::Responding;
                }
            }
            HookEventKind::PostToolUse | HookEventKind::PostToolUseFailure => {
                self.mode = SessionMode::Responding;
                self.pending = None;
            }

            HookEventKind::PermissionRequest => {
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

            HookEventKind::SubagentStart | HookEventKind::SubagentStop => {
                if self.mode != SessionMode::Approval && self.mode != SessionMode::Question {
                    self.mode = SessionMode::Responding;
                }
            }

            HookEventKind::Stop => {
                self.mode = SessionMode::Input;
                self.pending = None;
            }

            HookEventKind::Notification => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal HookEvent with an empty payload.
    fn make_event(name: &str) -> HookEvent {
        HookEvent {
            event: name.to_string(),
            session_id: "test-session".to_string(),
            cwd: None,
            timestamp: None,
            payload: serde_json::Map::new(),
        }
    }

    /// Build a HookEvent with a pre-populated JSON payload.
    fn make_event_with_payload(name: &str, payload: serde_json::Value) -> HookEvent {
        let map = match payload {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        HookEvent {
            event: name.to_string(),
            session_id: "test-session".to_string(),
            cwd: None,
            timestamp: None,
            payload: map,
        }
    }

    #[test]
    fn is_archived_only_for_stopped_and_stale_sessions() {
        let base = 1_000_000_000i64;
        let mut state = SessionState::new("s".into(), None);
        state.updated_at = OffsetDateTime::from_unix_timestamp(base).unwrap();

        // Stopped but recently active → not archived.
        state.mode = SessionMode::Stopped;
        assert!(!state.is_archived(base + 60));
        // Stopped and idle past the window → archived.
        assert!(state.is_archived(base + ARCHIVE_AFTER_SECONDS + 1));
        // Live (any non-stopped mode) is never archived, however old.
        state.mode = SessionMode::Input;
        assert!(!state.is_archived(base + ARCHIVE_AFTER_SECONDS + 1));
    }

    #[test]
    fn captures_transcript_path_from_hook() {
        let mut state = SessionState::new("spawn-uuid".into(), Some("/tmp".into()));
        let mut payload = serde_json::Map::new();
        payload.insert(
            "transcript_path".into(),
            Value::String("/home/u/.claude/projects/p/real-id.jsonl".into()),
        );
        let event = HookEvent {
            event: "SessionStart".into(),
            session_id: "claude-real-id".into(),
            cwd: Some("/tmp".into()),
            timestamp: None,
            payload,
        };
        state.apply(&event);
        assert_eq!(
            state.transcript_path.as_deref(),
            Some("/home/u/.claude/projects/p/real-id.jsonl")
        );
    }

    // ------------------------------------------------------------------ //
    // apply() — characterization tests for every event arm                //
    // ------------------------------------------------------------------ //

    #[test]
    fn apply_user_prompt_submit_sets_responding_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        // Precondition: put state in Approval to confirm the arm always overrides it.
        state.mode = SessionMode::Approval;
        state.pending = Some(Pending::Approval {
            tool: Some("Bash".into()),
            summary: None,
            raw: Value::Null,
        });

        state.apply(&make_event("UserPromptSubmit"));

        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.pending.is_none());
        assert_eq!(state.last_event.as_deref(), Some("UserPromptSubmit"));
    }

    #[test]
    fn apply_pre_tool_use_ask_user_question_sets_question_mode() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;

        let payload = serde_json::json!({
            "tool_name": "AskUserQuestion",
            "tool_input": {
                "questions": [
                    {
                        "question": "Which approach?",
                        "options": [{"label": "A"}, {"label": "B"}]
                    }
                ]
            }
        });
        state.apply(&make_event_with_payload("PreToolUse", payload));

        assert_eq!(state.mode, SessionMode::Question);
        assert!(state.pending.is_some());
        match state.pending.as_ref().unwrap() {
            Pending::Question { questions, .. } => {
                assert_eq!(questions.len(), 1);
                assert_eq!(questions[0].question, "Which approach?");
                assert_eq!(questions[0].options.len(), 2);
                assert_eq!(questions[0].options[0].label, "A");
            }
            other => panic!("expected Pending::Question, got {:?}", other),
        }
        // tool_calls counter incremented
        assert_eq!(state.tool_calls, 1);
    }

    #[test]
    fn apply_pre_tool_use_ask_user_question_empty_questions_still_sets_question_mode() {
        let mut state = SessionState::new("s".into(), None);
        let payload = serde_json::json!({
            "tool_name": "AskUserQuestion",
            "tool_input": {}
        });
        state.apply(&make_event_with_payload("PreToolUse", payload));

        assert_eq!(state.mode, SessionMode::Question);
        match state.pending.as_ref().unwrap() {
            Pending::Question { questions, .. } => {
                assert!(questions.is_empty(), "no questions parsed from empty input");
            }
            other => panic!("expected Pending::Question, got {:?}", other),
        }
    }

    #[test]
    fn apply_pre_tool_use_other_tool_sets_responding_when_not_blocked() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Input;

        let payload = serde_json::json!({ "tool_name": "Bash", "tool_input": {"command": "ls"} });
        state.apply(&make_event_with_payload("PreToolUse", payload));

        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.pending.is_none());
        assert_eq!(state.tool_calls, 1);
    }

    #[test]
    fn apply_pre_tool_use_other_tool_does_not_override_approval() {
        // When mode is Approval, a non-AskUserQuestion PreToolUse must NOT change the mode.
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Approval;
        state.pending = Some(Pending::Approval {
            tool: Some("Write".into()),
            summary: Some("overwrite /etc/passwd".into()),
            raw: Value::Null,
        });

        let payload = serde_json::json!({ "tool_name": "Read", "tool_input": {"file_path": "/x"} });
        state.apply(&make_event_with_payload("PreToolUse", payload));

        // Mode stays Approval; tool_calls still incremented.
        assert_eq!(state.mode, SessionMode::Approval);
        assert_eq!(state.tool_calls, 1);
    }

    #[test]
    fn apply_pre_tool_use_other_tool_does_not_override_question() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Question;

        let payload = serde_json::json!({ "tool_name": "Bash", "tool_input": {} });
        state.apply(&make_event_with_payload("PreToolUse", payload));

        assert_eq!(state.mode, SessionMode::Question);
        assert_eq!(state.tool_calls, 1);
    }

    #[test]
    fn apply_post_tool_use_sets_responding_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Approval;
        state.pending = Some(Pending::Approval {
            tool: Some("Bash".into()),
            summary: None,
            raw: Value::Null,
        });

        state.apply(&make_event("PostToolUse"));

        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.pending.is_none());
    }

    #[test]
    fn apply_post_tool_use_failure_also_sets_responding_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Question;
        state.apply(&make_event("PostToolUseFailure"));

        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.pending.is_none());
    }

    #[test]
    fn apply_permission_request_sets_approval_mode_with_tool_and_summary() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;

        let payload = serde_json::json!({
            "tool_name": "Write",
            "summary": "Overwrite config file"
        });
        state.apply(&make_event_with_payload("PermissionRequest", payload));

        assert_eq!(state.mode, SessionMode::Approval);
        match state.pending.as_ref().unwrap() {
            Pending::Approval { tool, summary, raw } => {
                assert_eq!(tool.as_deref(), Some("Write"));
                assert_eq!(summary.as_deref(), Some("Overwrite config file"));
                // raw is the whole payload wrapped in an Object
                assert!(raw.is_object());
                assert_eq!(raw.get("tool_name").and_then(Value::as_str), Some("Write"));
            }
            other => panic!("expected Pending::Approval, got {:?}", other),
        }
    }

    #[test]
    fn apply_permission_request_falls_back_to_message_field_for_summary() {
        let mut state = SessionState::new("s".into(), None);
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "message": "Run dangerous command"
        });
        state.apply(&make_event_with_payload("PermissionRequest", payload));

        assert_eq!(state.mode, SessionMode::Approval);
        match state.pending.as_ref().unwrap() {
            Pending::Approval { summary, .. } => {
                assert_eq!(summary.as_deref(), Some("Run dangerous command"));
            }
            other => panic!("expected Pending::Approval, got {:?}", other),
        }
    }

    #[test]
    fn apply_permission_request_no_tool_name_yields_none_tool() {
        let mut state = SessionState::new("s".into(), None);
        let payload = serde_json::json!({ "summary": "something" });
        state.apply(&make_event_with_payload("PermissionRequest", payload));

        assert_eq!(state.mode, SessionMode::Approval);
        match state.pending.as_ref().unwrap() {
            Pending::Approval { tool, .. } => assert!(tool.is_none()),
            other => panic!("expected Pending::Approval, got {:?}", other),
        }
    }

    #[test]
    fn apply_subagent_start_sets_responding_when_not_blocked() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Input;
        state.apply(&make_event("SubagentStart"));
        assert_eq!(state.mode, SessionMode::Responding);
    }

    #[test]
    fn apply_subagent_stop_sets_responding_when_not_blocked() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Input;
        state.apply(&make_event("SubagentStop"));
        assert_eq!(state.mode, SessionMode::Responding);
    }

    #[test]
    fn apply_subagent_start_does_not_override_approval() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Approval;
        state.apply(&make_event("SubagentStart"));
        assert_eq!(state.mode, SessionMode::Approval);
    }

    #[test]
    fn apply_subagent_stop_does_not_override_question() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Question;
        state.apply(&make_event("SubagentStop"));
        assert_eq!(state.mode, SessionMode::Question);
    }

    #[test]
    fn apply_stop_sets_input_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;
        state.pending = Some(Pending::Approval {
            tool: None,
            summary: None,
            raw: Value::Null,
        });

        state.apply(&make_event("Stop"));

        assert_eq!(state.mode, SessionMode::Input);
        assert!(state.pending.is_none());
    }

    #[test]
    fn apply_session_end_sets_stopped_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;
        state.pending = Some(Pending::Approval {
            tool: None,
            summary: None,
            raw: Value::Null,
        });

        state.apply(&make_event("SessionEnd"));

        assert_eq!(state.mode, SessionMode::Stopped);
        assert!(state.pending.is_none());
    }

    #[test]
    fn apply_notification_is_noop_for_mode() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;
        state.apply(&make_event("Notification"));
        // Mode must not change; last_event should be updated.
        assert_eq!(state.mode, SessionMode::Responding);
        assert_eq!(state.last_event.as_deref(), Some("Notification"));
    }

    #[test]
    fn apply_unknown_event_is_noop_for_mode() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Input;
        state.apply(&make_event("SomeFutureEvent"));
        assert_eq!(state.mode, SessionMode::Input);
        assert_eq!(state.last_event.as_deref(), Some("SomeFutureEvent"));
    }

    #[test]
    fn apply_always_updates_last_event_and_updated_at() {
        let mut state = SessionState::new("s".into(), None);
        let before = state.updated_at;
        // Sleep a tiny bit isn't needed — we just confirm the field is set.
        state.apply(&make_event("Stop"));
        assert_eq!(state.last_event.as_deref(), Some("Stop"));
        // updated_at must be >= before (monotone).
        assert!(state.updated_at >= before);
    }

    #[test]
    fn apply_sets_cwd_from_event_when_state_has_none() {
        let mut state = SessionState::new("s".into(), None);
        assert!(state.cwd.is_none());

        let mut event = make_event("Stop");
        event.cwd = Some("/project/dir".into());
        state.apply(&event);

        assert_eq!(state.cwd.as_deref(), Some("/project/dir"));
    }

    #[test]
    fn apply_does_not_overwrite_existing_cwd() {
        let mut state = SessionState::new("s".into(), Some("/original".into()));

        let mut event = make_event("Stop");
        event.cwd = Some("/new".into());
        state.apply(&event);

        assert_eq!(state.cwd.as_deref(), Some("/original"));
    }

    #[test]
    fn tool_calls_accumulate_across_pre_tool_use_events() {
        let mut state = SessionState::new("s".into(), None);
        for _ in 0..5 {
            let payload = serde_json::json!({ "tool_name": "Bash" });
            state.apply(&make_event_with_payload("PreToolUse", payload));
        }
        assert_eq!(state.tool_calls, 5);
    }

    #[test]
    fn apply_pre_tool_use_without_tool_name_field_increments_counter_and_sets_responding() {
        // When tool_name is absent, the arm sees "" which is not "AskUserQuestion",
        // so it falls through to the else-if branch.
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Input;
        let payload = serde_json::json!({});
        state.apply(&make_event_with_payload("PreToolUse", payload));
        assert_eq!(state.tool_calls, 1);
        assert_eq!(state.mode, SessionMode::Responding);
    }

    // ------------------------------------------------------------------ //
    // HookEventKind — serialization round-trip                            //
    // ------------------------------------------------------------------ //

    #[test]
    fn plan_status_from_wire_maps_defensively() {
        assert_eq!(PlanStatus::from_wire("pending"), PlanStatus::Pending);
        assert_eq!(PlanStatus::from_wire("in_progress"), PlanStatus::InProgress);
        assert_eq!(PlanStatus::from_wire("completed"), PlanStatus::Completed);
        // Codex/alt spellings.
        assert_eq!(PlanStatus::from_wire("done"), PlanStatus::Completed);
        assert_eq!(PlanStatus::from_wire("in-progress"), PlanStatus::InProgress);
        // Anything unrecognized falls back to Pending rather than being dropped.
        assert_eq!(PlanStatus::from_wire("garbage"), PlanStatus::Pending);
    }

    #[test]
    fn plan_status_serializes_to_wire_vocabulary() {
        assert_eq!(
            serde_json::to_string(&PlanStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(
            serde_json::to_string(&PlanStatus::Completed).unwrap(),
            "\"completed\""
        );
    }

    #[test]
    fn plan_serializes_with_camelcase_wire_fields() {
        let plan = Plan {
            steps: vec![PlanStep {
                content: "do it".into(),
                status: PlanStatus::InProgress,
                active_form: Some("Doing it".into()),
            }],
            updated_at: Some("2026-07-04T10:00:00Z".into()),
        };
        let v = serde_json::to_value(&plan).unwrap();
        assert_eq!(v["updatedAt"], "2026-07-04T10:00:00Z");
        assert_eq!(v["steps"][0]["content"], "do it");
        assert_eq!(v["steps"][0]["status"], "in_progress");
        assert_eq!(v["steps"][0]["activeForm"], "Doing it");
    }

    #[test]
    fn hook_event_kind_serializes_to_pascal_case_strings() {
        let cases: &[(HookEventKind, &str)] = &[
            (HookEventKind::SessionStart, "\"SessionStart\""),
            (HookEventKind::SessionEnd, "\"SessionEnd\""),
            (HookEventKind::UserPromptSubmit, "\"UserPromptSubmit\""),
            (HookEventKind::PreToolUse, "\"PreToolUse\""),
            (HookEventKind::PostToolUse, "\"PostToolUse\""),
            (HookEventKind::PostToolUseFailure, "\"PostToolUseFailure\""),
            (HookEventKind::Notification, "\"Notification\""),
            (HookEventKind::Stop, "\"Stop\""),
            (HookEventKind::SubagentStart, "\"SubagentStart\""),
            (HookEventKind::SubagentStop, "\"SubagentStop\""),
            (HookEventKind::PermissionRequest, "\"PermissionRequest\""),
        ];
        for (variant, expected_json) in cases {
            let serialized = serde_json::to_string(variant).unwrap();
            assert_eq!(
                &serialized, expected_json,
                "wrong serialization for {variant:?}"
            );
            // Also verify round-trip deserialization.
            let deserialized: HookEventKind = serde_json::from_str(&serialized).unwrap();
            assert_eq!(deserialized, *variant, "round-trip failed for {variant:?}");
        }
    }

    #[test]
    fn hook_event_kind_as_str_matches_serde() {
        let all = [
            HookEventKind::SessionStart,
            HookEventKind::SessionEnd,
            HookEventKind::UserPromptSubmit,
            HookEventKind::PreToolUse,
            HookEventKind::PostToolUse,
            HookEventKind::PostToolUseFailure,
            HookEventKind::Notification,
            HookEventKind::Stop,
            HookEventKind::SubagentStart,
            HookEventKind::SubagentStop,
            HookEventKind::PermissionRequest,
        ];
        for variant in all {
            let serde_str = serde_json::to_string(&variant).unwrap();
            // serde produces a quoted string; strip the quotes.
            let unquoted = serde_str.trim_matches('"');
            assert_eq!(
                variant.as_str(),
                unquoted,
                "as_str() diverges from serde for {variant:?}"
            );
        }
    }

    #[test]
    fn pending_question_accepts_both_multi_select_spellings() {
        // The AskUserQuestion tool input spells it camelCase; clients read the
        // serialized snake_case form.
        let camel: PendingQuestion = serde_json::from_value(serde_json::json!({
            "question": "Pick several", "multiSelect": true,
            "options": [{ "label": "a" }]
        }))
        .unwrap();
        assert!(camel.multi_select);
        let snake: PendingQuestion = serde_json::from_value(serde_json::json!({
            "question": "Pick several", "multi_select": true,
            "options": [{ "label": "a" }]
        }))
        .unwrap();
        assert!(snake.multi_select);
        assert!(serde_json::to_value(&camel)
            .unwrap()
            .get("multi_select")
            .and_then(serde_json::Value::as_bool)
            .unwrap());
    }
}

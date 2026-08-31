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
    PreCompact,
    PostCompact,
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
            Self::PreCompact => "PreCompact",
            Self::PostCompact => "PostCompact",
            Self::PermissionRequest => "PermissionRequest",
        }
    }

    /// The subset of variants that map to real Claude Code hook event names
    /// and must be registered in `~/.claude/settings.json`.
    ///
    /// `PostToolUseFailure` is NOT a real registerable hook — it is an internal
    /// variant only.
    ///
    /// `PermissionRequest` IS real (verified on CLI 2.1.237: registering it
    /// fires the command once per permission prompt, in step with the
    /// `can_use_tool` control requests the stream driver sees). It used to be
    /// excluded here as "forward-compat only", which meant `claudemon init`
    /// never installed it — and since it is the ONLY event that can set
    /// `SessionMode::Approval` for a PTY session (see `apply` below), a PTY
    /// agent's permission prompt produced no approvable record at all. The
    /// wire shapes — and this registration flag — are pinned in
    /// `contracts/permission-request-hook-cases.json`, read by
    /// `permission_request_contract_cases` below and by the desktop's
    /// `permissionRequestContract.test.ts`, so a CLI wording change breaks a
    /// test instead of approvals.
    pub const REGISTERABLE: &'static [HookEventKind] = &[
        Self::SessionStart,
        Self::SessionEnd,
        Self::UserPromptSubmit,
        Self::PreToolUse,
        Self::PostToolUse,
        Self::PermissionRequest,
        Self::Notification,
        Self::Stop,
        Self::SubagentStart,
        Self::SubagentStop,
        Self::PreCompact,
        Self::PostCompact,
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

/// Which feed a pending-slot write speaks for.
///
/// The slot holds one card, but a *managed* session can genuinely have two
/// requests outstanding at once, raised by two independent feeds:
///
/// * [`Self::Primary`] — the session's own state machine. For a PTY session
///   that is the hook feed ([`SessionState::apply`]); for a managed session it
///   is the driver task (`providers::{codex,opencode,pi,claude_stream}`), which
///   keeps its own FIFO of parked requests and re-surfaces the next one from
///   its decision arm. Exactly one of these exists per session.
/// * [`Self::Ask`] — the `AskUserQuestion` MCP shim (`daemon::mcp_ask`),
///   registered for codex, opencode and pi because they have no native
///   structured-question tool. It runs in an axum handler, entirely outside
///   the driver, and blocks an agent tool call for up to six hours.
///
/// Naming the feed is what lets the store keep the *older* request instead of
/// destroying it: a park displaces, a resolve releases only what it owns.
/// Before that, `mcp_ask` parked over a driver's approval card and then
/// cleared the slot when its question was answered — while the driver's FIFO
/// still held the approval and only re-surfaces it when a decision arrives for
/// a card the user can no longer see. That is the unresolvable-approval shape,
/// reached across two feeds rather than within one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingOwner {
    /// The session's own feed: the hook state machine (PTY) or the managed
    /// driver task (stream/codex/opencode/pi).
    Primary,
    /// The out-of-band `AskUserQuestion` MCP shim, `daemon::mcp_ask`.
    Ask,
}

/// What a managed-mode write intends to do with a session's single `pending`
/// slot, and which feed it speaks for.
///
/// This is the type form of the invariant that has produced the worst bugs on
/// this project (see `docs/unresolvable-approval-findings.md`): **a session's
/// pending slot may only be written by the feed that raised what is on it;
/// another feed may displace it, but must never destroy it.** Before this
/// existed, `set_managed_mode` took a bare `Option<Pending>` and every caller
/// re-derived the guard by hand —
///
/// ```ignore
/// if cur_mode != SessionMode::Approval && cur_mode != SessionMode::Question {
///     store.set_managed_mode(id, SessionMode::Responding, None);
/// }
/// ```
///
/// — which four sites simply forgot. Each omission produced the same
/// unanswerable shape: mode `Responding` (clients: "streaming") with
/// `pending: null`, while the CLI stayed blocked on an unanswered
/// `can_use_tool` that lives only inside the driver, so nothing outside could
/// repair it. The session wedged silently.
///
/// The first pass at this type said "exactly one feed owns the slot" and fenced
/// only [`Self::Keep`]. That claim was false for codex, opencode and pi, which
/// carry a second writer — the `AskUserQuestion` MCP shim, `daemon::mcp_ask` —
/// whose unattributed `Park`/`Resolve` pair could overwrite a driver's approval
/// card and then clear the slot, reaching the very shape above across two feeds.
/// So [`Park`](Self::Park) and [`Resolve`](Self::Resolve) now name their
/// [`PendingOwner`]: not another condition to remember, but a payload the
/// compiler demands at every call site, old and new.
///
/// Passing an intent instead of a value moves the whole guard into
/// [`SessionStore::set_managed_mode`](crate::session::SessionStore::set_managed_mode),
/// where it cannot be forgotten: a new call site must say which of the three
/// things it is doing and on whose behalf, and the store — not the caller —
/// decides what that is allowed to touch.
#[derive(Debug, Clone)]
pub enum PendingWrite {
    /// Park a request for the user (an approval card, a question picker) on
    /// behalf of `owner`. Always applied: the newest block is what the user is
    /// shown. If the *other* feed's request held the card it is displaced —
    /// kept, and restored the moment this one is released — so the two feeds
    /// can block on the user at the same time without either being lost.
    Park(PendingOwner, Pending),
    /// `owner`'s parked request is over — the user answered it, or a genuine
    /// turn boundary (a `result` frame, session start/stop) means nothing that
    /// feed raised can still be waiting.
    ///
    /// Clears only what `owner` owns. A feed that finds the *other* feed's
    /// request on the card leaves it, and the mode, exactly as it found them
    /// (dropping only its own displaced request, whose block is over). A feed
    /// releasing its own card restores the other's displaced request if there
    /// is one, and otherwise takes the requested mode.
    Resolve(PendingOwner),
    /// Liveness / enrichment only: a busy ping, a message written to stdin, a
    /// background-task count changing. Such a write says nothing about whether
    /// the user is still blocking the agent, so it is suppressed **entirely**
    /// while any request is parked — mode included, because reporting
    /// `Responding` for a session that is actually waiting on a human is the
    /// other half of the same bug. It needs no owner: it may never touch the
    /// slot, whoever sends it.
    Keep,
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
            "in_progress" | "inprogress" | "in-progress" | "inProgress" => Self::InProgress,
            "completed" | "complete" | "done" => Self::Completed,
            _ => Self::Pending,
        }
    }
}

/// Provider-neutral subagent row surfaced on session snapshots. Claude's richer
/// rows are still built in the desktop from hook/transcript artifacts; managed
/// providers use this daemon-owned shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub agent_type: String,
    pub status: SubagentStatus,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool_summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentUpdate {
    pub id: String,
    pub agent_type: Option<String>,
    pub status: SubagentStatus,
    pub description: Option<String>,
    pub tool_use_id: Option<String>,
    pub model: Option<String>,
    pub last_tool_name: Option<String>,
    pub last_tool_summary: Option<String>,
}

/// Session capabilities parsed from the stream `system/init` frame (stream
/// transport only — the PTY statusLine doesn't carry them). Static for the life
/// of the session; ride the StatusLine channel so they reach clients on the
/// first tick without a separate feed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Whether fast mode is active (`fast_mode_state`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_mode: Option<bool>,
    /// Active output style, e.g. "default" / "explanatory".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,
    /// Where the session's credentials come from ("none"/"user"/… — subscription
    /// vs API key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_source: Option<String>,
    /// Counts of the capabilities available to the session.
    #[serde(default)]
    pub mcp_servers: u32,
    #[serde(default)]
    pub skills: u32,
    #[serde(default)]
    pub plugins: u32,
    #[serde(default)]
    pub agents: u32,
    #[serde(default)]
    pub memory_files: u32,
    /// Itemized inventory behind the counts (names, paths, size estimates) —
    /// what the Context pane renders. Absent on PTY sessions and on stream
    /// frames that predate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory: Option<ContextInventory>,
}

/// One named thing loaded into the session's context: an MCP server, skill,
/// agent, plugin, or memory file. `bytes`/`est_tokens` are best-effort
/// estimates from the backing file on disk (~4 chars per token) — absent when
/// there is no file we can find (builtin agents, MCP tool schemas).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// MCP server connection status ("connected" / "pending" / "failed").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Plugin origin, e.g. "rust-analyzer-lsp@claude-plugins-official".
    ///
    /// For skills and agents this is the ORIGIN enrichment resolved them to —
    /// "project", "user", a plugin name, or "built-in" for the ones compiled
    /// into the CLI binary (deep-research, dataviz, verify, …), which have no
    /// file anywhere and so can never carry a path or a size. Without it every
    /// unresolved skill rendered as a bare word with no way to tell "shipped
    /// inside claude" apart from "we failed to find its file".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// One-line description from the item's frontmatter (`description:` in a
    /// SKILL.md / agent file). Absent for built-ins and for anything whose file
    /// we couldn't read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub est_tokens: Option<u64>,
}

/// Itemized inventory of what the stream `init` frame reports loaded into the
/// session: names straight off the frame, sizes enriched from disk where the
/// item is file-backed. Rides [`Capabilities`] so every client that already
/// sees the counts gets the detail for free.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextInventory {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<ContextItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<ContextItem>,
    /// Memory *files* — directories from the frame's `memory_paths` are
    /// expanded to the files inside them during enrichment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_files: Vec<ContextItem>,
    /// Builtin tool names available to the session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slash_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_code_version: Option<String>,
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
    /// Reasoning-effort level in force, when the provider confirms one (Codex's
    /// `thread/settings/updated`). Claude publishes its effective effort nowhere,
    /// so this stays absent there and the composer falls back to what it asked
    /// for. Rides the status line for the same reason `model_display` does: it's
    /// live provider truth, and it follows a change made in the provider's TUI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// `context_window.used_percentage` (0–100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_output_tokens: Option<u64>,
    /// The cache-read subset of `total_input_tokens`, when the provider reports
    /// one. Codex does (`cachedInputTokens` on both its wire shapes, and
    /// `cached_input_tokens` in the rollout file), and the daemon already used
    /// it to discount the cost estimate; carrying it here is what lets a client
    /// SHOW the split rather than only be billed by it.
    ///
    /// Claude's statusLine payload carries no cache figures at all, so this
    /// stays `None` there and the desktop reads the itemized transcript split
    /// (`Usage::cache`) instead. `None` means "not reported", never "nothing
    /// was cached".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    /// `cost.total_cost_usd` — Claude's own authoritative session cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// `rate_limits.five_hour.used_percentage` (0–100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour_pct: Option<f64>,
    /// Unix epoch seconds the 5h window resets at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour_resets_at: Option<i64>,
    /// How long the 5h window is, in minutes. See [`CLAUDE_FIVE_HOUR_WINDOW_MINUTES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour_window_minutes: Option<u64>,
    /// `rate_limits.seven_day.used_percentage` (0–100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_pct: Option<f64>,
    /// Unix epoch seconds the 7d window resets at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_resets_at: Option<i64>,
    /// How long the weekly window is, in minutes. See [`CLAUDE_SEVEN_DAY_WINDOW_MINUTES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_window_minutes: Option<u64>,
    /// Monthly overage/credit window utilization (0–100). Sourced from Claude's
    /// stream `overage` `rateLimitType`; absent for the interactive statusLine
    /// (which carries only 5h/7d) and for providers without a monthly window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_pct: Option<f64>,
    /// Unix epoch seconds the monthly overage window resets at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_resets_at: Option<i64>,
    /// How long the monthly overage window is, in minutes. Stays `None` for
    /// Claude: a calendar month is 28-31 days and no source spells it out, so
    /// clients show that window without a duration rather than a made-up one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_window_minutes: Option<u64>,
    /// Human warning message when a window crosses its warning threshold
    /// (Claude's `status: allowed_warning`). Cleared when comfortable again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_warning: Option<String>,
    /// The monthly overage is disabled for lack of credits (Claude's
    /// `overageDisabledReason: out_of_credits`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overage_out_of_credits: Option<bool>,
    /// Session capabilities from the stream `init` frame (stream only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
    /// When the daemon received this line, so clients can age out stale data.
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub received_at: Option<OffsetDateTime>,
}

/// Claude's account windows are named by their length rather than carrying it:
/// `five_hour` is 300 minutes and `seven_day` is 10080. Nothing on Claude's
/// wire spells the duration out, so the daemon stamps it from the window's own
/// name and every client reads the same `*_window_minutes` field that Codex
/// fills from its own `windowDurationMins`.
pub const CLAUDE_FIVE_HOUR_WINDOW_MINUTES: u64 = 300;
/// See [`CLAUDE_FIVE_HOUR_WINDOW_MINUTES`].
pub const CLAUDE_SEVEN_DAY_WINDOW_MINUTES: u64 = 10_080;

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
            // Claude's statusLine carries no effort field (verified: its stream
            // `init` frame doesn't either). Read it best-effort so we pick it up
            // automatically if that ever changes, without depending on it.
            effort: v.get("effort").and_then(Value::as_str).map(str::to_owned),
            // CLAMPED, because every client renders this straight — the TUI,
            // /m and remote.html all print `context_used_pct` without a bound
            // of their own, and the desktop multiplies it by the window to
            // DERIVE a token count for providers that report none. An
            // out-of-range percentage therefore does not stay a cosmetic
            // oddity: it becomes a session claiming to hold many times what it
            // can. `UsageAcc::status_line` already clamps the percentage it
            // computes for managed providers (`.min(100.0)`); this is the same
            // rule for the one it is handed rather than computes.
            context_used_pct: cw
                .and_then(|c| c.get("used_percentage"))
                .and_then(Value::as_f64)
                .filter(|p| p.is_finite())
                .map(|p| p.clamp(0.0, 100.0)),
            context_window_size: cw
                .and_then(|c| c.get("context_window_size"))
                .and_then(Value::as_u64),
            total_input_tokens: cw
                .and_then(|c| c.get("total_input_tokens"))
                .and_then(Value::as_u64),
            // Claude's statusLine payload carries no cache figures. Read the
            // field best-effort so we pick it up if that ever changes, and stay
            // `None` meanwhile. The desktop reads the itemized transcript
            // split for Claude, which the status line could not supply anyway.
            cached_input_tokens: cw
                .and_then(|c| c.get("cache_read_input_tokens"))
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
            five_hour_window_minutes: five.map(|_| CLAUDE_FIVE_HOUR_WINDOW_MINUTES),
            seven_day_pct: seven
                .and_then(|s| s.get("used_percentage"))
                .and_then(Value::as_f64),
            seven_day_resets_at: seven
                .and_then(|s| s.get("resets_at"))
                .and_then(Value::as_i64),
            seven_day_window_minutes: seven.map(|_| CLAUDE_SEVEN_DAY_WINDOW_MINUTES),
            monthly_pct: monthly
                .and_then(|m| m.get("used_percentage"))
                .and_then(Value::as_f64),
            monthly_resets_at: monthly
                .and_then(|m| m.get("resets_at"))
                .and_then(Value::as_i64),
            monthly_window_minutes: None,
            // The interactive statusLine JSON doesn't carry warning/overage
            // status or capabilities — those ride the stream events only.
            rate_limit_warning: None,
            overage_out_of_credits: None,
            capabilities: None,
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
    /// What the user is being asked for right now — the single displayed card.
    ///
    /// Private on purpose: this is the field whose unattributed writes wedged
    /// real sessions. Read it with [`SessionState::pending`]; write it only
    /// through [`SessionState::write_pending`] / [`SessionState::clear_pending`],
    /// which keep [`Self::pending_owner`] and [`Self::displaced_pending`] in
    /// step. A feed that cannot name itself cannot touch it.
    #[serde(skip_deserializing, rename = "pending")]
    pending_card: Option<Pending>,
    /// Which feed raised [`Self::pending_card`]; `None` when the slot is empty.
    /// Bookkeeping for the write funnel, never sent to clients.
    #[serde(skip)]
    pending_owner: Option<PendingOwner>,
    /// The other feed's request, displaced by the one currently on the card,
    /// with the mode it was parked under. Restored when the card is released.
    /// At most one, because each feed has at most one request outstanding.
    #[serde(skip)]
    displaced_pending: Option<(PendingOwner, Pending, SessionMode)>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub tool_calls: u64,
    /// How many user prompts this session has seen (`UserPromptSubmit` hooks).
    /// The "did the user actually talk to this agent" signal: a session spawned
    /// but never prompted stays at 0. Restored from the persisted event log on
    /// boot (see `SessionStore::hydrate`) and incremented live in `apply`.
    /// Additive/back-compatible like the fields below — old persisted JSON and
    /// pre-field rows deserialize to 0.
    #[serde(default)]
    pub user_prompts: u64,
    pub last_event: Option<String>,
    /// Absolute path to Claude's transcript JSONL, captured from the hook
    /// payload. Lets `/transcript` read the exact file even when the session id
    /// we expose (a spawn UUID) differs from Claude's own id that names the file.
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// The Claude config root this session was SPAWNED with — the value of
    /// `CLAUDE_CONFIG_DIR` in its spawn env, or the daemon's own default when
    /// the spawn set none. Absolute and un-normalized; `None` means the daemon
    /// did not spawn this session and genuinely does not know.
    ///
    /// This is the ATTRIBUTION KEY, and it exists because deriving it from the
    /// transcript path is a correctness hazard rather than a shortcut.
    /// `~/.claude/accounts/<name>/projects` is a SYMLINK to the shared
    /// `~/.claude/projects` (workspacer's "share memories, separate login"
    /// profile design), so both accounts write into ONE physical directory and
    /// the only thing that tells them apart is the *un-resolved* path string
    /// the CLI happened to use. One `realpath`/`canonicalize` anywhere upstream
    /// — ours or someone else's — collapses both logins onto the default
    /// account, silently and with no error, and the two accounts' usage merges.
    /// Recording the root at the source makes attribution independent of that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_root: Option<String>,
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
    /// Managed-provider subagent rows. Claude PTY/stream sessions still build
    /// their detailed rows in the desktop from hook/transcript artifacts; this
    /// list is for providers whose native machine interface reports subagent
    /// identities directly (Codex app-server).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubagentInfo>,
    /// Context compaction, driven by the `PreCompact`/`PostCompact` hooks.
    /// `compacting` is true between the two; `last_compact_at` (unix seconds) and
    /// `compaction_count` let clients badge a recently-compacted session and
    /// surface churn. Additive/back-compatible like the fields above.
    #[serde(default)]
    pub compacting: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compact_at: Option<i64>,
    #[serde(default)]
    pub compaction_count: u64,
    /// Live background tasks visible on the WIRE — any type: async subagents,
    /// `run_in_background` shells (a dev server, a poll loop), workflows. Fed
    /// by the stream driver's `background_tasks_changed` frames
    /// (`SessionStore::set_background_tasks`) and mirrored from
    /// `live_subagents` on the hook path. The MODE never rides on ambient
    /// tasks (holding "responding" for a background shell painted idle agents
    /// as working forever) — this count is how clients badge that work
    /// honestly instead. Additive/back-compatible: absent rows read 0.
    #[serde(default)]
    pub background_tasks: u32,
    /// Count of background subagents (async Agent/Task tool) currently running,
    /// tracked from the `SubagentStart`/`SubagentStop` hooks. While this is
    /// non-zero, a parent `Stop` does NOT idle the session — the parent's own
    /// turn can end while a subagent works on, and flipping to `Input` there
    /// would show idle mid-subagent. The real idle rides the last
    /// `SubagentStop`. Internal state-machine bookkeeping, not part of the wire
    /// snapshot (mirrors the stream driver's `bg_tasks_active`).
    #[serde(skip)]
    pub live_subagents: u32,
    /// True once the parent's own turn has ended (a `Stop` fired) while
    /// `live_subagents > 0`, so the draining `SubagentStop` knows to flip the
    /// mode to `Input`. Cleared on a new turn / session boundary.
    #[serde(skip)]
    pub parent_turn_ended: bool,
    /// The model string this session was *asked* for at spawn — the alias the
    /// caller picked (`opus[1m]`, `gpt-5-codex`), not the concrete id the
    /// provider ends up reporting. Recorded because Claude Code strips the
    /// `[1m]` marker from the `model` it writes into the transcript: without
    /// this the daemon cannot tell a 1M session from a 200k one until the
    /// session's context exceeds 200k, and every context gauge reads 5× too
    /// full until then (see `usage::Usage::resolve_window`). Updated by a live
    /// `/sessions/:id/model` switch. Additive/back-compatible: old persisted
    /// rows and every client that doesn't send a model deserialize to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_model: Option<String>,
    /// Canonical durable truth for the request. Published additively alongside
    /// `requested_model`, which remains the compatibility projection older
    /// readers know. Absence means the owner never recorded a selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_selection: Option<super::windows::ModelSelection>,
    /// Monotonic owner epoch for accepted live selections. Provider status
    /// frames carry no command id, so telemetry remains on the prior epoch
    /// until a frame is compatible with `pending_model_confirmation`.
    #[serde(skip)]
    pub model_selection_epoch: u64,
    /// Epoch of the model/window fields currently allowed through from provider
    /// telemetry. Kept separate from `model_selection_epoch` so delayed frames
    /// cannot silently inherit the newly accepted selection's provenance.
    #[serde(skip)]
    pub model_telemetry_epoch: u64,
    /// The accepted selection whose provider confirmation is still pending.
    /// Ephemeral by design; hydration may recreate the fence from durable owner
    /// truth without a schema change, but only with the same finite evidence
    /// budget as a live acceptance.
    #[serde(skip)]
    pub pending_model_confirmation: Option<super::windows::ModelSelection>,
    /// How many more incompatible provider status frames may have their
    /// model/window fields withheld. This is a frame-count bound, not a timer:
    /// every arriving status frame is evidence that the provider has advanced
    /// after the switch, even when its display name can never equal the request.
    /// Reaching zero releases the fence after suppressing that frame, so the
    /// next frame (and every later truthful divergence) is visible.
    #[serde(skip)]
    pub model_confirmation_suppressions_remaining: u8,
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
            pending_card: None,
            pending_owner: None,
            displaced_pending: None,
            started_at: now,
            updated_at: now,
            tool_calls: 0,
            user_prompts: 0,
            last_event: None,
            transcript_path: None,
            config_root: None,
            status_line: None,
            provider: default_provider(),
            transport: Transport::default(),
            plan: None,
            subagents: Vec::new(),
            compacting: false,
            last_compact_at: None,
            compaction_count: 0,
            background_tasks: 0,
            live_subagents: 0,
            parent_turn_ended: false,
            requested_model: None,
            requested_selection: None,
            model_selection_epoch: 0,
            model_telemetry_epoch: 0,
            pending_model_confirmation: None,
            model_confirmation_suppressions_remaining: 0,
        }
    }

    /// What the user is being asked for right now, or `None`. The single card
    /// every client renders — the displayed head of a slot that may have one
    /// more request from the other feed waiting behind it (see
    /// [`PendingOwner`]).
    pub fn pending(&self) -> Option<&Pending> {
        self.pending_card.as_ref()
    }

    /// Apply a [`PendingWrite`] to the slot and to `mode`, and report whether
    /// anything changed (a suppressed write has nothing to broadcast).
    ///
    /// **This is the only place the pending slot is ever written.** Every
    /// rule the slot has lives here rather than at its call sites, which is
    /// the whole point: the four wedged-session bugs were all a call site
    /// forgetting a rule it had to restate by hand.
    ///
    /// * [`PendingWrite::Keep`] — refused outright while any request is
    ///   parked, mode included.
    /// * [`PendingWrite::Park`] — always applied. The other feed's card, if it
    ///   held the slot, is displaced rather than dropped; this feed's own
    ///   displaced request is superseded (each feed has one outstanding
    ///   request at a time — the drivers' FIFOs surface only their head).
    /// * [`PendingWrite::Resolve`] — releases only what the named feed owns.
    ///   With the other feed's request on the card, the card and the mode are
    ///   left untouched and only this feed's displaced request is dropped.
    ///   Releasing its own card restores the other feed's displaced request,
    ///   under the mode it was parked with, instead of the requested mode.
    #[must_use]
    pub(crate) fn write_pending(&mut self, mode: SessionMode, write: PendingWrite) -> bool {
        match write {
            // A parked request is a PAUSE: the agent is blocked on the user,
            // not working. A liveness write knows nothing about that and must
            // not speak for it.
            PendingWrite::Keep
                if self.mode == SessionMode::Approval || self.mode == SessionMode::Question =>
            {
                false
            }
            PendingWrite::Keep => {
                self.mode = mode;
                self.pending_card = None;
                self.pending_owner = None;
                self.displaced_pending = None;
                true
            }
            PendingWrite::Park(owner, pending) => {
                if let (Some(held), Some(holder)) = (self.pending_card.take(), self.pending_owner) {
                    if holder != owner {
                        self.displaced_pending = Some((holder, held, self.mode));
                    }
                }
                // A displaced request is always the OTHER feed's — it is only
                // ever set from the card this feed just took over — so there
                // is never one of ours here to supersede.
                debug_assert!(
                    self.displaced_pending.as_ref().map(|(o, _, _)| *o) != Some(owner),
                    "a feed's own request cannot be the displaced one"
                );
                self.mode = mode;
                self.pending_card = Some(pending);
                self.pending_owner = Some(owner);
                true
            }
            PendingWrite::Resolve(owner) => {
                if matches!(self.pending_owner, Some(holder) if holder != owner) {
                    // Someone else's request is on the card. Our own block is
                    // over, so drop what we had displaced behind it — but the
                    // card, and the mode that reports it, are not ours to
                    // touch. Nothing visible changed.
                    if self.displaced_pending.as_ref().map(|(o, _, _)| *o) == Some(owner) {
                        self.displaced_pending = None;
                    }
                    return false;
                }
                self.pending_card = None;
                self.pending_owner = None;
                match self.displaced_pending.take() {
                    // The other feed is still blocked on the user: put its
                    // request back on the card instead of declaring the
                    // session free.
                    Some((holder, pending, parked_mode)) => {
                        self.mode = parked_mode;
                        self.pending_card = Some(pending);
                        self.pending_owner = Some(holder);
                    }
                    None => self.mode = mode,
                }
                true
            }
        }
    }

    /// Wipe the slot — card, owner and anything displaced — without asking who
    /// owns what. Only for the paths where the session itself is over or
    /// starting fresh (session start/end, a resumed row leaving `Stopped`, the
    /// ghost sweep): every feed's block really is void, so there is nothing to
    /// preserve for anyone.
    pub(crate) fn clear_pending(&mut self) {
        self.pending_card = None;
        self.pending_owner = None;
        self.displaced_pending = None;
    }

    /// Park a request from the session's own feed (hooks, or a driver) — the
    /// [`PendingOwner::Primary`] spelling of [`PendingWrite::Park`]. Public
    /// because it is also the only honest way to build a parked fixture: with
    /// the card private, a caller cannot mint one without naming a feed.
    pub fn park_pending(&mut self, mode: SessionMode, pending: Pending) {
        let _ = self.write_pending(mode, PendingWrite::Park(PendingOwner::Primary, pending));
    }

    /// Close out subagent rows still marked `Running` once the parent's own
    /// turn is over, and re-derive `background_tasks` from what survives.
    /// Returns whether anything changed.
    ///
    /// Daemon-tracked subagent rows are scoped to the tool call that spawned
    /// them (`tool_use_id`; Codex's `spawnAgent` and friends), and a tool call
    /// cannot outlive the turn it was issued in — so a row still `Running`
    /// after the parent has gone back to `Input` is stale by construction: its
    /// completion frame never arrived. Nothing else ever closes it, so left
    /// alone it is a *permanent* lie — the roll-up that asks "is this agent
    /// working" (`background_tasks`, and the desktop's
    /// `sessionHasBackgroundWork`) reads a dead child as live work and the
    /// parent shows busy forever, which also means its working→idle edge never
    /// fires and a dispatched worker never reports finished.
    ///
    /// Observed live before this existed: a codex session sat at `mode: input`
    /// with `background_tasks: 1` and one `Running` row for ten minutes across
    /// four further user turns — long after the agent itself had said in
    /// conversation that the subagent was done.
    ///
    /// Closing is the self-healing direction (the same asymmetry that keeps
    /// `local_bash` out of the busy-holding set in `claude_stream.rs`): if the
    /// provider does have more to say about that agent, the next
    /// `SessionStore::apply_subagent_update` re-opens the row as `Running` and
    /// clears `completed_at`. A wrong busy never self-corrects; a wrong idle
    /// does, on the next frame.
    ///
    /// No-op for providers that publish no subagent rows — Claude's live
    /// bookkeeping is `live_subagents`/`background_tasks` and its rows are
    /// built in the desktop, so `background_tasks` (which for a stream session
    /// counts background *shells* too) must not be re-derived here.
    pub fn close_stale_subagents(&mut self) -> bool {
        if self.subagents.is_empty() {
            return false;
        }
        let completed_at = OffsetDateTime::now_utc().unix_timestamp() * 1000;
        let mut changed = false;
        for sub in &mut self.subagents {
            if sub.status == SubagentStatus::Running {
                sub.status = SubagentStatus::Complete;
                sub.completed_at.get_or_insert(completed_at);
                changed = true;
            }
        }
        if changed {
            // Same derivation `apply_subagent_update` uses, so the wire count
            // and the rows can never disagree: nothing is running now.
            self.background_tasks = self
                .subagents
                .iter()
                .filter(|s| s.status == SubagentStatus::Running)
                .count() as u32;
        }
        changed
    }

    /// The Claude config root serving this session, `""` = the daemon's
    /// default. This is what keys the account-usage reading, so getting it
    /// wrong merges two logins' rate-limit gauges.
    ///
    /// Two sources, in this order, and the order is the whole point:
    ///
    /// 1. [`config_root`](Self::config_root) — what the session was actually
    ///    SPAWNED with. Robust: no amount of path canonicalization downstream
    ///    can change it, because it never came from a path.
    /// 2. the transcript path, as a fallback for sessions the daemon did not
    ///    spawn (`claudemon wrap` in a terminal that exported
    ///    `CLAUDE_CONFIG_DIR`, or a child orphaned by a restart).
    ///
    /// Source 2 is only correct while the path is un-resolved, and that is a
    /// property of the string, not something we can enforce: profile roots
    /// symlink `projects` at the shared `~/.claude/projects`, so a canonicalized
    /// path names the default account no matter which login wrote it. It used
    /// to be the ONLY source, which made every account's attribution one stray
    /// `realpath` away from silently merging. Source 1 is why it no longer is.
    ///
    /// A session with neither reads as the default root; it also has no status
    /// line yet, so nothing wrong gets patched in the interim.
    pub fn claude_config_root(&self) -> String {
        if let Some(root) = self.config_root.as_deref() {
            return super::account_usage::normalize_root(root);
        }
        self.transcript_path
            .as_deref()
            .and_then(super::account_usage::root_from_transcript)
            .map(|r| super::account_usage::normalize_root(&r))
            .unwrap_or_default()
    }

    /// Which account this session billed against, WITH its uncertainty.
    ///
    /// [`Self::claude_config_root`] must return a single `String` because it is
    /// a map key (one usage reading per account, one poll target per account),
    /// and a key cannot say "I don't know" — so it falls back to the default
    /// account, which is the right thing for a poll target and the wrong thing
    /// for a report. This is the reporting answer:
    ///
    ///   - spawn-recorded `config_root` → `Certain`, always. It came from the
    ///     spawn env, not from a path, so no amount of path resolution
    ///     downstream can change what it says.
    ///   - no stamp, a transcript path → whatever
    ///     [`account_usage::attribute_transcript`] can honestly conclude, which
    ///     is `Ambiguous` exactly when a profile shares this root's `projects`
    ///     directory.
    ///   - no stamp and no transcript → `Unknown`. A pre-v8 row or a session
    ///     the daemon did not spawn; NOT the default account.
    ///
    /// [`account_usage::attribute_transcript`]: super::account_usage::attribute_transcript
    pub fn claude_account_attribution(
        &self,
        roots: &[String],
    ) -> super::account_usage::RootAttribution {
        if let Some(root) = self.config_root.as_deref() {
            return super::account_usage::RootAttribution::Certain {
                account: super::account_usage::normalize_root(root),
            };
        }
        match self.transcript_path.as_deref() {
            Some(path) => super::account_usage::attribute_transcript(path, roots),
            None => super::account_usage::RootAttribution::Unknown,
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

    /// Whether this is a stopped session the user never actually used — spawned
    /// (a `SessionStart` fired, so it's on disk and rehydrates on boot) but with
    /// no prompt ever submitted and no tool ever run. There is no conversation to
    /// resume, so these are hidden from the default session list the same way
    /// archived rows are; they'd otherwise pile up as "stale sessions" on login.
    ///
    /// Scoped to `provider == "claude"`: both the PTY and stream transports run
    /// Claude Code's hooks, so `user_prompts`/`tool_calls` are trustworthy there.
    /// Codex/opencode/pi drive their state from native events (no hooks), so we
    /// have no such signal for them and must never gate them on it — and they're
    /// never persisted/rehydrated anyway, so this costs nothing.
    ///
    /// Both counters must be zero: `tool_calls` is persisted directly on the
    /// session row, so it still guards a genuinely-used session whose prompt
    /// events were dropped by a lagging persistence broadcast.
    pub fn is_empty_stopped(&self) -> bool {
        self.mode == SessionMode::Stopped
            && self.provider == "claude"
            && self.user_prompts == 0
            && self.tool_calls == 0
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
                self.clear_pending();
                self.live_subagents = 0;
                self.background_tasks = 0;
                self.parent_turn_ended = false;
            }
            HookEventKind::SessionEnd => {
                self.mode = SessionMode::Stopped;
                self.clear_pending();
                self.live_subagents = 0;
                self.background_tasks = 0;
                self.parent_turn_ended = false;
            }

            HookEventKind::UserPromptSubmit => {
                self.mode = SessionMode::Responding;
                self.user_prompts = self.user_prompts.saturating_add(1);
                self.clear_pending();
                // A fresh user turn supersedes any prior turn's background work.
                self.live_subagents = 0;
                self.background_tasks = 0;
                self.parent_turn_ended = false;
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
                    self.park_pending(SessionMode::Question, Pending::Question { questions, raw });
                } else if self.mode != SessionMode::Approval && self.mode != SessionMode::Question {
                    self.mode = SessionMode::Responding;
                }
            }
            HookEventKind::PostToolUse | HookEventKind::PostToolUseFailure => {
                self.mode = SessionMode::Responding;
                self.clear_pending();
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
                self.park_pending(
                    SessionMode::Approval,
                    Pending::Approval { tool, summary, raw },
                );
            }

            HookEventKind::SubagentStart => {
                self.live_subagents = self.live_subagents.saturating_add(1);
                // Wire mirror: hook-driven (PTY) sessions have no
                // background_tasks_changed frames, so the subagent count IS
                // their live-background-work count.
                self.background_tasks = self.live_subagents;
                if self.mode != SessionMode::Approval && self.mode != SessionMode::Question {
                    self.mode = SessionMode::Responding;
                }
            }

            HookEventKind::SubagentStop => {
                self.live_subagents = self.live_subagents.saturating_sub(1);
                self.background_tasks = self.live_subagents;
                if self.mode == SessionMode::Approval || self.mode == SessionMode::Question {
                    // A picker is up — never override it.
                } else if self.live_subagents == 0 && self.parent_turn_ended {
                    // The last background subagent drained after the parent's
                    // own turn had already ended: the real idle rides in now.
                    self.parent_turn_ended = false;
                    self.mode = SessionMode::Input;
                    self.clear_pending();
                } else {
                    // Parent still working, or more subagents outstanding.
                    self.mode = SessionMode::Responding;
                }
            }

            HookEventKind::Stop => {
                if self.live_subagents > 0 {
                    // The parent's own turn ended while a background subagent is
                    // still running — hold `Responding` rather than showing idle
                    // mid-subagent. The draining `SubagentStop` flips to `Input`.
                    self.parent_turn_ended = true;
                    if self.mode != SessionMode::Approval && self.mode != SessionMode::Question {
                        self.mode = SessionMode::Responding;
                    }
                } else {
                    self.mode = SessionMode::Input;
                    self.clear_pending();
                }
            }

            // Context compaction brackets: PreCompact starts it (Claude is busy
            // rewriting context, not idle), PostCompact ends it and records the
            // event so clients can badge a recently-compacted / churning session.
            HookEventKind::PreCompact => {
                self.compacting = true;
                if self.mode != SessionMode::Approval && self.mode != SessionMode::Question {
                    self.mode = SessionMode::Responding;
                }
            }
            HookEventKind::PostCompact => {
                self.compacting = false;
                self.last_compact_at = Some(OffsetDateTime::now_utc().unix_timestamp());
                self.compaction_count = self.compaction_count.saturating_add(1);
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

    /// ABSURD NUMBERS, at the producer.
    ///
    /// `context_used_pct` is rendered straight by the TUI, /m and remote.html,
    /// and the desktop MULTIPLIES it by the window to derive a token count for
    /// providers that report none. So an out-of-range percentage is not a
    /// cosmetic oddity — it becomes a session claiming to hold many times what
    /// it can. Clamped here, where the payload is read, rather than in each of
    /// the four places that consume it.
    #[test]
    fn context_percentage_is_clamped_to_a_percentage() {
        let pct = |v: serde_json::Value| {
            StatusLine::from_claude_json(&serde_json::json!({ "context_window": v }))
                .context_used_pct
        };
        assert_eq!(
            pct(serde_json::json!({ "used_percentage": 42.5 })),
            Some(42.5)
        );
        assert_eq!(
            pct(serde_json::json!({ "used_percentage": 4_300.0 })),
            Some(100.0),
            "a running total where a percentage was expected must not multiply the window by 43"
        );
        assert_eq!(
            pct(serde_json::json!({ "used_percentage": -5.0 })),
            Some(0.0)
        );
        // Non-finite is not a reading at all.
        assert_eq!(pct(serde_json::json!({ "used_percentage": "nope" })), None);
        assert_eq!(pct(serde_json::json!({})), None);
    }

    /// Claude never spells a window's length out, so the daemon stamps it from
    /// the window's own name, and only for the windows that actually reported.
    #[test]
    fn claude_status_line_stamps_window_lengths_it_knows() {
        let sl = StatusLine::from_claude_json(&serde_json::json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 11.0, "resets_at": 1787593199 },
                "seven_day": { "used_percentage": 2.0 }
            }
        }));
        assert_eq!(
            sl.five_hour_window_minutes,
            Some(CLAUDE_FIVE_HOUR_WINDOW_MINUTES)
        );
        assert_eq!(
            sl.seven_day_window_minutes,
            Some(CLAUDE_SEVEN_DAY_WINDOW_MINUTES)
        );
        // No monthly window in the payload, so nothing is claimed for it: not a
        // length, not a percentage.
        assert_eq!(sl.monthly_pct, None);
        assert_eq!(sl.monthly_window_minutes, None);
    }

    /// A statusLine with no rate_limits block at all (most accounts) must leave
    /// every window untouched rather than stamping lengths onto empty slots.
    #[test]
    fn claude_status_line_without_windows_stamps_nothing() {
        let sl = StatusLine::from_claude_json(&serde_json::json!({
            "model": { "display_name": "Opus" }
        }));
        assert_eq!(sl.five_hour_window_minutes, None);
        assert_eq!(sl.seven_day_window_minutes, None);
        assert_eq!(sl.monthly_window_minutes, None);
    }

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
    fn compaction_hooks_bracket_the_compacting_flag() {
        let mut state = SessionState::new("s".into(), None);
        assert!(!state.compacting);
        assert_eq!(state.compaction_count, 0);

        state.apply(&make_event("PreCompact"));
        assert!(state.compacting, "PreCompact starts compaction");

        state.apply(&make_event("PostCompact"));
        assert!(!state.compacting, "PostCompact ends compaction");
        assert_eq!(state.compaction_count, 1);
        assert!(state.last_compact_at.is_some());
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

    #[test]
    fn empty_stopped_gates_only_unused_claude_sessions() {
        // Spawned but never prompted → empty once stopped.
        let mut s = SessionState::new("s".into(), Some("/w".into()));
        s.apply(&make_event("SessionStart"));
        assert!(!s.is_empty_stopped(), "not stopped yet");
        s.apply(&make_event("SessionEnd"));
        assert!(
            s.is_empty_stopped(),
            "stopped claude session with no prompt and no tool call is empty"
        );

        // One real prompt → no longer empty, even after it stops.
        let mut used = SessionState::new("u".into(), Some("/w".into()));
        used.apply(&make_event("UserPromptSubmit"));
        used.apply(&make_event("SessionEnd"));
        assert_eq!(used.user_prompts, 1);
        assert!(
            !used.is_empty_stopped(),
            "a prompted session is never empty"
        );

        // A tool call alone also counts as real use (guards against a prompt
        // event lost to a lagging persistence broadcast).
        let mut tooled = SessionState::new("t".into(), Some("/w".into()));
        tooled.tool_calls = 1;
        tooled.mode = SessionMode::Stopped;
        assert!(!tooled.is_empty_stopped());

        // Non-claude managed providers have no hook signal — never gate them.
        let mut codex = SessionState::new("c".into(), Some("/w".into()));
        codex.provider = "codex".into();
        codex.mode = SessionMode::Stopped;
        assert!(
            !codex.is_empty_stopped(),
            "codex/opencode/pi lack the prompt signal and must not be gated"
        );
    }

    // ------------------------------------------------------------------ //
    // apply() — characterization tests for every event arm                //
    // ------------------------------------------------------------------ //

    /// The desktop, the web client and `/m` all read `pending` off this
    /// struct's JSON. Privatising the field and adding the ownership
    /// bookkeeping beside it must not have moved one byte of that wire shape:
    /// the card still serialises as `pending`, and the bookkeeping is not
    /// client state and must not leak.
    #[test]
    fn the_pending_card_still_serialises_under_its_own_name_and_nothing_else_does() {
        let mut state = SessionState::new("s".into(), None);
        state.park_pending(
            SessionMode::Approval,
            Pending::Approval {
                tool: Some("Bash".into()),
                summary: Some("ls".into()),
                raw: Value::Null,
            },
        );

        let wire = serde_json::to_value(&state).expect("serialize");
        assert_eq!(wire["pending"]["kind"], "approval");
        assert_eq!(wire["pending"]["tool"], "Bash");
        assert!(
            wire.get("pending_owner").is_none() && wire.get("displaced_pending").is_none(),
            "slot bookkeeping must never reach a client: {wire}"
        );
        assert!(
            wire.get("pending_card").is_none(),
            "the field rename must not have leaked: {wire}"
        );
    }

    #[test]
    fn apply_user_prompt_submit_sets_responding_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        // Precondition: put state in Approval to confirm the arm always overrides it.
        state.park_pending(
            SessionMode::Approval,
            Pending::Approval {
                tool: Some("Bash".into()),
                summary: None,
                raw: Value::Null,
            },
        );

        state.apply(&make_event("UserPromptSubmit"));

        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.pending().is_none());
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
        assert!(state.pending().is_some());
        match state.pending().as_ref().unwrap() {
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
        match state.pending().as_ref().unwrap() {
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
        assert!(state.pending().is_none());
        assert_eq!(state.tool_calls, 1);
    }

    #[test]
    fn apply_pre_tool_use_other_tool_does_not_override_approval() {
        // When mode is Approval, a non-AskUserQuestion PreToolUse must NOT change the mode.
        let mut state = SessionState::new("s".into(), None);
        state.park_pending(
            SessionMode::Approval,
            Pending::Approval {
                tool: Some("Write".into()),
                summary: Some("overwrite /etc/passwd".into()),
                raw: Value::Null,
            },
        );

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
        state.park_pending(
            SessionMode::Approval,
            Pending::Approval {
                tool: Some("Bash".into()),
                summary: None,
                raw: Value::Null,
            },
        );

        state.apply(&make_event("PostToolUse"));

        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.pending().is_none());
    }

    #[test]
    fn apply_post_tool_use_failure_also_sets_responding_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Question;
        state.apply(&make_event("PostToolUseFailure"));

        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.pending().is_none());
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
        match state.pending().as_ref().unwrap() {
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
        match state.pending().as_ref().unwrap() {
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
        match state.pending().as_ref().unwrap() {
            Pending::Approval { tool, .. } => assert!(tool.is_none()),
            other => panic!("expected Pending::Approval, got {:?}", other),
        }
    }

    /// The claudemon half of `contracts/permission-request-hook-cases.json`
    /// (the desktop half is `hookEventRouter.permissionRequestContract.test.ts`).
    /// Two readers pick different fields off this one frame, so a CLI rename
    /// must break a test rather than silently produce an unanswerable prompt.
    #[test]
    fn permission_request_contract_cases() {
        #[derive(serde::Deserialize)]
        struct Fixture {
            #[serde(rename = "hookEventName")]
            hook_event_name: String,
            registerable: bool,
            cases: Vec<Case>,
        }
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            payload: Value,
            claudemon: Expected,
        }
        #[derive(serde::Deserialize)]
        struct Expected {
            mode: SessionMode,
            #[serde(rename = "pendingKind")]
            pending_kind: String,
            tool: Option<String>,
            summary: Option<String>,
        }

        const FIXTURE: &str =
            include_str!("../../../../contracts/permission-request-hook-cases.json");
        let fixture: Fixture = serde_json::from_str(FIXTURE).expect("fixture parses");

        // The registration half: while this said `false` the hook was never
        // written into ~/.claude/settings.json and a PTY session's permission
        // prompt produced no approvable record at all.
        assert_eq!(
            fixture.hook_event_name,
            HookEventKind::PermissionRequest.as_str()
        );
        assert_eq!(
            fixture.registerable,
            HookEventKind::REGISTERABLE.contains(&HookEventKind::PermissionRequest),
            "contract and REGISTERABLE disagree about installing the PermissionRequest hook"
        );

        for case in &fixture.cases {
            let mut state = SessionState::new("s".into(), None);
            state.mode = SessionMode::Responding;
            state.apply(&make_event_with_payload(
                &fixture.hook_event_name,
                case.payload.clone(),
            ));

            assert_eq!(
                state.mode, case.claudemon.mode,
                "mode for case {:?}",
                case.name
            );
            match state.pending().as_ref() {
                Some(Pending::Approval { tool, summary, raw }) => {
                    assert_eq!(
                        case.claudemon.pending_kind, "approval",
                        "case {:?}",
                        case.name
                    );
                    assert_eq!(
                        tool.as_deref(),
                        case.claudemon.tool.as_deref(),
                        "tool for case {:?}",
                        case.name
                    );
                    assert_eq!(
                        summary.as_deref(),
                        case.claudemon.summary.as_deref(),
                        "summary for case {:?}",
                        case.name
                    );
                    // `raw` is what a client renders from, so the payload must
                    // survive the fold verbatim rather than being summarized away.
                    assert_eq!(raw, &case.payload, "raw for case {:?}", case.name);
                }
                other => panic!(
                    "expected Pending::Approval for case {:?}, got {:?}",
                    case.name, other
                ),
            }
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
    fn stop_holds_responding_while_a_background_subagent_runs() {
        // Parent launches a background subagent, then its own turn ends (Stop)
        // while the subagent is still working — the session must NOT go idle.
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;
        state.apply(&make_event("SubagentStart"));
        assert_eq!(state.live_subagents, 1);

        state.apply(&make_event("Stop"));
        assert_eq!(state.mode, SessionMode::Responding);
        assert!(state.parent_turn_ended);

        // The real idle rides in on the trailing SubagentStop.
        state.apply(&make_event("SubagentStop"));
        assert_eq!(state.mode, SessionMode::Input);
        assert_eq!(state.live_subagents, 0);
        assert!(!state.parent_turn_ended);
    }

    #[test]
    fn stop_idles_immediately_with_no_background_subagent() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;
        state.apply(&make_event("Stop"));
        assert_eq!(state.mode, SessionMode::Input);
        assert!(!state.parent_turn_ended);
    }

    #[test]
    fn subagent_finishing_mid_turn_keeps_parent_responding() {
        // A subagent that completes while the parent is still streaming (no Stop
        // yet) must not idle the parent.
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;
        state.apply(&make_event("SubagentStart"));
        state.apply(&make_event("SubagentStop"));
        assert_eq!(state.mode, SessionMode::Responding);
        assert!(!state.parent_turn_ended);
    }

    #[test]
    fn stop_waits_for_all_parallel_subagents_to_drain() {
        let mut state = SessionState::new("s".into(), None);
        state.mode = SessionMode::Responding;
        state.apply(&make_event("SubagentStart"));
        state.apply(&make_event("SubagentStart"));
        state.apply(&make_event("Stop"));
        assert_eq!(state.mode, SessionMode::Responding);

        state.apply(&make_event("SubagentStop"));
        // One still running — stay busy.
        assert_eq!(state.mode, SessionMode::Responding);
        state.apply(&make_event("SubagentStop"));
        // Last one drained — now idle.
        assert_eq!(state.mode, SessionMode::Input);
    }

    #[test]
    fn user_prompt_resets_pending_background_subagent_state() {
        let mut state = SessionState::new("s".into(), None);
        state.apply(&make_event("SubagentStart"));
        state.apply(&make_event("Stop"));
        assert!(state.parent_turn_ended);
        // A new turn supersedes any stuck background bookkeeping.
        state.apply(&make_event("UserPromptSubmit"));
        assert_eq!(state.live_subagents, 0);
        assert!(!state.parent_turn_ended);
        assert_eq!(state.mode, SessionMode::Responding);
    }

    #[test]
    fn apply_stop_sets_input_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        state.park_pending(
            SessionMode::Responding,
            Pending::Approval {
                tool: None,
                summary: None,
                raw: Value::Null,
            },
        );

        state.apply(&make_event("Stop"));

        assert_eq!(state.mode, SessionMode::Input);
        assert!(state.pending().is_none());
    }

    #[test]
    fn apply_session_end_sets_stopped_clears_pending() {
        let mut state = SessionState::new("s".into(), None);
        state.park_pending(
            SessionMode::Responding,
            Pending::Approval {
                tool: None,
                summary: None,
                raw: Value::Null,
            },
        );

        state.apply(&make_event("SessionEnd"));

        assert_eq!(state.mode, SessionMode::Stopped);
        assert!(state.pending().is_none());
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
        assert_eq!(PlanStatus::from_wire("inProgress"), PlanStatus::InProgress);
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

    /// THE SYMLINK ATTRIBUTION HAZARD, proved rather than asserted.
    ///
    /// Workspacer's "Add Claude Account" gives a second login its own config
    /// root but shares memories and transcripts by symlinking
    /// `<root>/projects` at the primary `~/.claude/projects`. So both accounts
    /// write their JSONL into ONE physical directory, and the ONLY thing that
    /// distinguishes them is the un-resolved path string the CLI happened to
    /// use. This test builds that exact layout on disk — a real symlink, not a
    /// mocked one — and shows both halves:
    ///
    ///   1. the fallback source IS lossy under canonicalization. A resolved
    ///      path from the `work` account names the PRIMARY root. No error, no
    ///      signal: the two accounts' usage merges and nothing says so.
    ///   2. attribution nevertheless survives, because the spawn-recorded
    ///      `config_root` never came from a path.
    ///
    /// Assertion (2) is the regression guard, and it FAILS on the previous
    /// implementation: `claude_config_root` read only `transcript_path`, so
    /// handed the canonicalized path it returned the primary root — silently
    /// billing the `work` account's tokens to the default account.
    #[cfg(unix)]
    #[test]
    fn account_attribution_survives_a_canonicalized_transcript_path() {
        let base = std::env::temp_dir().join(format!(
            "wks-symlink-attr-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = std::fs::remove_dir_all(&base);
        let primary = base.join(".claude");
        let shared_projects = primary.join("projects").join("-home-u-work");
        let work_root = primary.join("accounts").join("work");
        std::fs::create_dir_all(&shared_projects).unwrap();
        std::fs::create_dir_all(&work_root).unwrap();
        // The hazard itself: the profile's `projects` IS the primary's.
        std::os::unix::fs::symlink(primary.join("projects"), work_root.join("projects")).unwrap();
        let transcript = work_root
            .join("projects")
            .join("-home-u-work")
            .join("abc.jsonl");
        std::fs::write(&transcript, "").unwrap();

        let as_spelled = transcript.to_string_lossy().into_owned();
        let resolved = std::fs::canonicalize(&transcript)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // The symlink really did collapse the two roots into one directory.
        assert_ne!(
            as_spelled, resolved,
            "fixture must actually traverse a symlink"
        );

        // (1) The fallback source, on each spelling of the SAME file.
        let root_of = |p: &str| {
            super::super::account_usage::normalize_root(
                &super::super::account_usage::root_from_transcript(p).unwrap(),
            )
        };
        assert_eq!(
            root_of(&as_spelled),
            work_root.to_string_lossy(),
            "un-resolved, the path still names the account that wrote it",
        );
        assert_eq!(
            root_of(&resolved),
            primary.to_string_lossy(),
            "RESOLVED, the same file names the PRIMARY account — this is the \
             silent merge, and it is why the transcript path cannot be the \
             only source of attribution",
        );

        // (2) The regression guard. Same canonicalized path, but the session
        // remembers what it was spawned with.
        let mut merged = SessionState::new("s-merged".into(), None);
        merged.transcript_path = Some(resolved.clone());
        assert_eq!(
            merged.claude_config_root(),
            primary.to_string_lossy(),
            "without the spawn stamp there is nothing left to attribute with",
        );

        let mut stamped = SessionState::new("s-stamped".into(), None);
        stamped.transcript_path = Some(resolved);
        stamped.config_root = Some(work_root.to_string_lossy().into_owned());
        assert_eq!(
            stamped.claude_config_root(),
            work_root.to_string_lossy(),
            "the spawn-recorded root wins over any path, resolved or not",
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// The spawn env is the attribution source, and "no CLAUDE_CONFIG_DIR" is
    /// a real answer (the default account) rather than an absent one.
    #[test]
    fn spawn_env_names_the_account() {
        use std::collections::HashMap;
        let root_from_spawn_env = super::super::account_usage::root_from_spawn_env;
        assert_eq!(root_from_spawn_env(&HashMap::new()), "");
        let mut env = HashMap::new();
        env.insert("CLAUDE_CONFIG_DIR".to_string(), "  ".to_string());
        assert_eq!(root_from_spawn_env(&env), "", "blank is not a root");
        env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            "/home/u/.claude/accounts/work/".to_string(),
        );
        assert_eq!(
            root_from_spawn_env(&env),
            "/home/u/.claude/accounts/work",
            "trailing separator trimmed, so it keys the same as the poller's",
        );
    }
}

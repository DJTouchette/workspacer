//! Wire and domain types, mirroring claudemon's REST API (`GET /sessions`,
//! `/sessions/:id`, `/sessions/:id/transcript`). claudemon is the source of
//! truth for a standalone TUI — the hub-bus capabilities the `/remote` client
//! uses are registered by the Electron app and absent when it isn't running.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::Value;

// TWIN: `DRIFT_TOLERANCE` in desktop modelContextWindows.ts and the
// numerator/denominator pair in claudemon session/windows.rs.
const CONTEXT_WINDOW_DRIFT_TOLERANCE_NUM: u64 = 102;
const CONTEXT_WINDOW_DRIFT_TOLERANCE_DEN: u64 = 100;

/// Token/cost/context usage for a session, as returned by claudemon's
/// `GET /sessions` (and `GET /sessions/:id`) in the additive `usage` field.
/// Fields are optional so older daemon versions that omit the block still
/// deserialize cleanly.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    pub model: Option<String>,
    /// Input side of the latest turn — a point-in-time view of context fullness.
    #[serde(default)]
    pub context_tokens: u64,
    /// The session's context window, in tokens. `None` is the daemon saying it
    /// does NOT KNOW — a provider that reports no window, a model no table
    /// covers, or a turn-1 stream session that has not spoken yet. It is not
    /// zero, and it must never be rendered as a guess: every readout below
    /// omits the percentage rather than draw a bar against an invented
    /// denominator.
    ///
    /// `serde(default)` is load-bearing for VERSION SKEW, not just tidiness.
    /// A newer claudemon omits the key entirely when the window is unknown
    /// (`skip_serializing_if` on the Rust twin, `Usage::context_limit`), and a
    /// required `u64` here would fail the WHOLE `Usage` deserialize on the
    /// missing key — losing model, cost and tokens with it, not merely the
    /// window. An older daemon still sends a number and deserializes as before.
    #[serde(default)]
    pub context_limit: Option<u64>,
    /// Cumulative cost (USD) for the session.
    #[serde(default)]
    pub cost_usd: f64,
    /// Fresh / cache-write / cache-read split of the prompt side, cumulative.
    /// Absent when the provider reported no cache fields. That is "not
    /// reported", not "nothing was cached", so the detail pane omits the line
    /// rather than printing zeros.
    #[serde(default)]
    pub cache: Option<CacheSplit>,
}

/// The canonical model request its OWNER recorded for a session: the model
/// identity, and the context window asked for with it. Mirrors
/// `ModelSelection` in services/claudemon/src/session/windows.rs.
///
/// Both fields are optional and both are read by presence, because the pair is
/// the daemon's claim and this client only relays it. `model: Some, window:
/// None` is a real, common state — an identity was pinned and no window was
/// resolved for it yet — and it must stay distinguishable from a window this
/// client made up. The `contextWindow` alias absorbs the hub's camelCase
/// projection of the same fact.
/// Both halves are carried, neither is rendered yet: what a session is MEASURED
/// against is the resolved window (see [`Agent::owner_context_window`]), and the
/// request is the other half of the story a client needs before it can ever show
/// the two disagreeing. Relaying it faithfully now is what makes that possible
/// without another wire change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RequestedSelection {
    #[allow(dead_code)]
    #[serde(default)]
    pub model: Option<String>,
    #[allow(dead_code)]
    #[serde(default, alias = "contextWindow")]
    pub context_window: Option<u64>,
}

/// The prompt-cache split claudemon carries on `usage.cache`. Mirrors
/// `CacheSplit` in services/claudemon/src/session/usage.rs, field for field.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct CacheSplit {
    #[serde(default)]
    pub fresh: u64,
    #[serde(default)]
    pub write: u64,
    #[serde(default)]
    pub read: u64,
}

/// Claude's authoritative statusLine telemetry, streamed from claudemon's
/// `/statusline/stream`. Every field is optional (Claude omits some, and
/// rate-limit data only exists for Pro/Max accounts). Preferred over the
/// transcript-derived [`Usage`] when present — see [`derive_stats`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StatusLine {
    #[serde(default)]
    pub model_display: Option<String>,
    /// `context_window.used_percentage` (0–100).
    #[serde(default)]
    pub context_used_pct: Option<f64>,
    /// The denominator behind `context_used_pct`, when the provider reports
    /// one. Additive and optional so older daemon and peer payloads remain
    /// wire-compatible. The pair is rejected together when observed usage has
    /// disproved this window; see [`derive_stats`].
    #[serde(default)]
    pub context_window_size: Option<u64>,
    /// Claude's own authoritative session cost.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Cumulative input tokens, when the provider reports them.
    #[serde(default)]
    pub total_input_tokens: Option<u64>,
    /// The cache-read subset of `total_input_tokens`. Codex reports it; Claude's
    /// statusLine does not, and there the itemized [`Usage::cache`] is the
    /// source instead.
    #[serde(default)]
    pub cached_input_tokens: Option<u64>,
    /// 5h rate-limit window used %, 0–100 (Pro/Max only).
    #[serde(default)]
    pub five_hour_pct: Option<f64>,
    /// The 5h window's length in minutes. Codex reports its own figure per
    /// window; Claude's is stamped from the window's name. Absent when nothing
    /// reported one, in which case the slot's short name stands in.
    #[serde(default)]
    pub five_hour_window_minutes: Option<u64>,
    /// 7d rate-limit window used %, 0–100 (Pro/Max only).
    #[serde(default)]
    pub seven_day_pct: Option<f64>,
    /// The weekly window's length in minutes.
    #[serde(default)]
    pub seven_day_window_minutes: Option<u64>,
    /// Monthly overage/credit window used %, 0–100 (Claude stream `overage`).
    #[serde(default)]
    pub monthly_pct: Option<f64>,
    /// The monthly window's length in minutes. No source reports one today.
    #[serde(default)]
    pub monthly_window_minutes: Option<u64>,
    /// Human warning when a window crosses its threshold (stream only).
    #[serde(default)]
    pub rate_limit_warning: Option<String>,
    /// Monthly overage disabled for lack of credits (stream only).
    #[serde(default)]
    pub overage_out_of_credits: Option<bool>,
}

/// Model / context-% / cost for a session, resolving claudemon's authoritative
/// statusLine first and falling back to transcript-derived [`Usage`] — the
/// terminal analogue of the desktop `deriveSessionStats` precedence.
#[derive(Debug, Default, Clone)]
pub struct DerivedStats {
    pub model: Option<String>,
    pub context_pct: Option<f64>,
    pub cost: Option<f64>,
}

/// Compact token count: `142k`, `1.2M`, `12M`. Mirrors the desktop's
/// `fmtTokens` so the same session reads the same in both clients.
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        return if n >= 10_000_000 {
            format!("{m:.0}M")
        } else {
            format!("{m:.1}M")
        };
    }
    if n >= 1_000 {
        let k = (n as f64 / 1_000.0).round() as u64;
        // 999_500..=999_999 rounds to 1000k; carry it to M like the twin does.
        if k >= 1_000 {
            return "1.0M".to_string();
        }
        return format!("{k}k");
    }
    n.to_string()
}

/// A session's prompt-cache split, as a client should render it.
///
/// TWIN: `cacheBreakdown` in apps/desktop/src/renderer/src/lib/sessionStats.ts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheReport {
    /// Prompt tokens processed fresh.
    pub fresh: u64,
    /// Prompt tokens written into the cache. `None` when the provider counts
    /// cache reads but never itemizes writes. Codex reports a cached subset of
    /// its input and nothing at all about writes, and a 0 there would claim it
    /// wrote nothing.
    pub write: Option<u64>,
    /// Prompt tokens served back from the cache.
    pub read: u64,
}

impl CacheReport {
    /// Prompt tokens across every tier the provider reported.
    pub fn total(&self) -> u64 {
        self.fresh + self.write.unwrap_or(0) + self.read
    }

    /// Share of the prompt served from cache, 0-100. `None` when the reported
    /// tiers sum to zero. A hit rate over an empty prompt is not 0%, it is
    /// undefined, and printing 0% would read as a cache that never hit.
    pub fn hit_rate_pct(&self) -> Option<f64> {
        let total = self.total();
        (total > 0).then(|| self.read as f64 / total as f64 * 100.0)
    }
}

/// The prompt-cache split for a session, from whichever source actually has it.
///
/// Claude's itemized transcript split wins: it is the only source that separates
/// writes from reads. Codex has no such itemization, but its status line does
/// carry a cache-read subset of the cumulative input, which is enough for the
/// fresh/read halves. `None` when neither source reported anything, so callers
/// omit the readout rather than draw an all-zero one.
pub fn cache_report(agent: &Agent, sl: Option<&StatusLine>) -> Option<CacheReport> {
    if let Some(c) = agent.usage.as_ref().and_then(|u| u.cache) {
        return Some(CacheReport {
            fresh: c.fresh,
            write: Some(c.write),
            read: c.read,
        });
    }
    let sl = sl?;
    let (input, cached) = (sl.total_input_tokens?, sl.cached_input_tokens?);
    Some(CacheReport {
        fresh: input.saturating_sub(cached),
        write: None,
        read: cached.min(input),
    })
}

pub fn derive_stats(agent: &Agent, sl: Option<&StatusLine>) -> DerivedStats {
    let model = sl
        .and_then(|s| s.model_display.clone())
        .or_else(|| agent.usage.as_ref().and_then(|u| u.model.clone()));
    let usage_pct = || {
        let limit = agent.owner_context_window()?;
        agent.usage.as_ref().and_then(|u| {
            (limit > 0 && u.context_tokens > 0)
                .then(|| u.context_tokens as f64 / limit as f64 * 100.0)
        })
    };
    // TWIN: DRIFT_TOLERANCE in modelContextWindows.ts and the desktop
    // `deriveSessionStats` / `busContextLimit` consumers. A status percentage
    // and its denominator are one claim. If the session demonstrably holds
    // more than that denominator (past the same 2% rounding tolerance), reject
    // both and use only the daemon's already-resolved usage window. Do not
    // infer 1M from occupancy: a missing resolved limit stays unknown.
    let held = agent.usage.as_ref().map(|u| u.context_tokens).unwrap_or(0);
    let status_window_disproved = sl
        .and_then(|s| s.context_window_size)
        .filter(|window| *window > 0)
        .is_some_and(|window| {
            held > window.saturating_mul(CONTEXT_WINDOW_DRIFT_TOLERANCE_NUM)
                / CONTEXT_WINDOW_DRIFT_TOLERANCE_DEN
        });
    let context_pct = if status_window_disproved {
        usage_pct()
    } else {
        sl.and_then(|s| s.context_used_pct).or_else(usage_pct)
    };
    let cost = sl
        .and_then(|s| s.cost_usd)
        .filter(|c| *c > 0.0)
        .or_else(|| {
            agent
                .usage
                .as_ref()
                .map(|u| u.cost_usd)
                .filter(|c| *c > 0.0)
        });
    DerivedStats {
        model,
        context_pct,
        cost,
    }
}

/// The mode a Claude session can be in, mirroring claudemon's `SessionMode`.
///
/// Uses `#[serde(rename_all = "snake_case")]` to match the wire format claudemon
/// emits (e.g. `"input"`, `"approval"`). The `Unknown` catch-all variant absorbs
/// any future values so deserialization never fails on unknown modes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// No hook has fired yet, or the mode field was absent.
    #[default]
    Unknown,
    /// Chat prompt is up — waiting for the user to send a message.
    Input,
    /// Claude is actively producing a turn (streaming, thinking, or tool use).
    Responding,
    /// Paused waiting for a tool-permission yes/no/always decision.
    Approval,
    /// Claude asked the user a structured question via `AskUserQuestion`.
    Question,
    /// Session has ended.
    Stopped,
    /// A mode this client doesn't recognise yet — forward-compat catch-all.
    #[serde(other)]
    Other,
}

/// Serde default for [`Agent::provider`] — claudemon's un-managed PTY path is
/// Claude, and older daemons omit the field entirely, so an absent value means
/// Claude. Mirrors claudemon's own `default_provider`.
fn default_provider() -> String {
    "claude".to_string()
}

/// Serde default for [`Agent::transport`] — sessions are PTY-backed unless the
/// daemon says otherwise (it serializes `"stream"` for headless stream-json
/// sessions; older daemons omit the field entirely).
fn default_transport() -> String {
    "pty".to_string()
}

/// One live session, as returned by claudemon's `GET /sessions`.
#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Which agent backend owns this session, straight from claudemon's session
    /// list (`"claude"` | `"codex"` | `"copilot"` | `"opencode"` | `"pi"`). Authoritative over
    /// the TUI's local managed-spawn map — see [`App::provider_for`]. Serde
    /// defaults to `"claude"` so older daemons deserialize unchanged.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// How the daemon talks to the session: `"pty"` (a real terminal we can
    /// stream and type into) or `"stream"` (a headless stream-json adapter —
    /// no PTY exists, so the TUI renders the transcript only). Serde defaults
    /// to `"pty"` so older daemons deserialize unchanged.
    #[serde(default = "default_transport")]
    pub transport: String,
    /// The current session mode. Defaults to `AgentMode::Unknown` when the
    /// field is absent, and falls back to `AgentMode::Other` for unrecognised
    /// values so deserialization never panics on future daemon versions.
    #[serde(default)]
    pub mode: AgentMode,
    /// What Claude is blocked on, if anything. `skip_deserializing` on the
    /// daemon means it can be absent; we tolerate that.
    #[serde(default)]
    pub pending: Option<Pending>,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub last_event: Option<String>,
    /// Claude's transcript file for this session. Minus its `.jsonl` suffix this
    /// is the session's artifact directory, where its subagent and workflow
    /// progress is written (see [`crate::runs`]).
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// Token/cost/context/model as returned by claudemon's `/sessions` response.
    /// Absent when the daemon hasn't computed it yet (no assistant turns).
    #[serde(default)]
    pub usage: Option<Usage>,
    /// What the session's OWNER (the daemon running it) recorded as the model
    /// request: the identity, and the context window asked for alongside it.
    /// Absent on every row from a daemon that predates the field, and absent
    /// when nobody pinned a selection — which is why it is an `Option` of a
    /// struct whose own window is an `Option` too. This client never fills in
    /// either half: an unresolved window here means the owner resolved none,
    /// not 200k.
    ///
    /// Two spellings reach this client and both are the same fact: claudemon's
    /// own `requested_selection` (a direct `GET /sessions` row) and the hub's
    /// `requestedSelection` projection (a bus row, local or federated). The
    /// alias is what lets one struct read both, so a row cannot arrive with the
    /// selection silently dropped just because it came the other way round.
    #[serde(default, alias = "requestedSelection")]
    pub requested_selection: Option<RequestedSelection>,
    /// The window the owner's resolver settled on, in tokens — the same number
    /// behind [`Usage::context_limit`], published in its own right so a row
    /// carrying no usage block yet still says what the session is measured
    /// against. DISPLAY ONLY: a denominator, never evidence about what the
    /// provider claimed. When the provider's own status pair contradicts the
    /// tokens actually held, that contradiction is still settled against the
    /// RAW provider claim (see [`derive_stats`]), never against this.
    ///
    /// Aliased like the selection above, for the same reason: `GET /sessions`
    /// spells it `resolved_context_window`, the hub's projection spells it
    /// `resolvedContextWindow`.
    #[serde(default, alias = "resolvedContextWindow")]
    pub resolved_context_window: Option<u64>,
    /// Peer hub this session lives on (federation). `None` — the overwhelmingly
    /// common case, and everything claudemon returns — means local. Remote rows
    /// are built from hub-stamped `agent.snapshot` events / `sessions.snapshots`
    /// seeds (see `crate::federation`), never from claudemon, so this field
    /// deserializes to `None` on every claudemon payload.
    #[serde(default)]
    pub hub: Option<String>,
    /// True while this remote session's hub is unreachable: the row is kept as
    /// a tombstone ("hub offline") with actions disabled, rather than silently
    /// vanishing — which would read as "my agent died". Always false for local.
    #[serde(default)]
    pub hub_offline: bool,
    /// The session's own display label, when its home hub assigned one (the
    /// desktop snapshot's `label`). Used for remote rows, where the cwd names a
    /// filesystem on another machine.
    #[serde(default)]
    pub label: Option<String>,
    /// True for a state-only remote row (`sparse: true` on the wire): the
    /// headless brain's compat rows and the desktop's stopped layout-ghosts.
    /// Live status without the desktop's enrichment — folded OVER a richer row
    /// instead of replacing it (see `crate::federation::fold_row`), and the UI
    /// degrades gracefully where data is missing (no label → cwd-derived name,
    /// as everywhere else). Never set on claudemon payloads, so always false
    /// for local sessions.
    #[serde(default)]
    pub sparse: bool,
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

/// A content fingerprint of a pending question set, so stepper state
/// ([`crate::app::QuestionFlow`]) can detect when the set it tracks was
/// superseded by a *different* set of the same length — length alone would
/// silently carry old answers over to questions the user never saw.
pub fn question_fingerprint(qs: &[Question]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for q in qs {
        q.question.hash(&mut h);
        q.header.hash(&mut h);
        q.multi_select.hash(&mut h);
        for o in &q.options {
            o.label.hash(&mut h);
        }
        // Separator so option/question boundaries can't alias.
        u8::MAX.hash(&mut h);
    }
    h.finish()
}

impl Agent {
    pub fn state(&self) -> &str {
        match &self.mode {
            AgentMode::Unknown => "unknown",
            AgentMode::Input => "input",
            AgentMode::Responding => "responding",
            AgentMode::Approval => "approval",
            AgentMode::Question => "question",
            AgentMode::Stopped => "stopped",
            AgentMode::Other => "other",
        }
    }

    /// True when the agent needs the user: an approval, a question, or a chat
    /// prompt awaiting the next message (matches the `/remote` semantics).
    pub fn is_waiting(&self) -> bool {
        matches!(
            self.mode,
            AgentMode::Input | AgentMode::Approval | AgentMode::Question
        )
    }

    /// The window to MEASURE this session's context against, in tokens, or
    /// `None` when nothing resolved one.
    ///
    /// `usage.context_limit` first — it is the resolver's answer travelling
    /// with the tokens it belongs to — then the row's own
    /// `resolved_context_window`, which is the same resolver's answer on a row
    /// whose usage block has not been computed yet (or was mapped away). Both
    /// come from the OWNER; neither is inferred here.
    ///
    /// Deliberately NOT a source of truth about the provider: this is only ever
    /// a denominator for display. The request (`requested_selection`) is what
    /// was asked for and can differ; the status line's own window is raw
    /// provider evidence and stays authoritative for the contradiction check in
    /// [`derive_stats`].
    pub fn owner_context_window(&self) -> Option<u64> {
        self.usage
            .as_ref()
            .and_then(|u| u.context_limit)
            .or(self.resolved_context_window)
            .filter(|w| *w > 0)
    }

    /// True when this session belongs to a peer hub (federation) rather than
    /// the local claudemon.
    pub fn is_remote(&self) -> bool {
        self.hub.is_some()
    }

    /// `is_waiting`, minus tombstones: a session whose hub is offline can't be
    /// acted on, so it must not count toward "needs you" or be a `m`-jump stop.
    pub fn needs_you(&self) -> bool {
        self.is_waiting() && !self.hub_offline
    }

    /// True when the agent is actively producing a turn.
    pub fn is_busy(&self) -> bool {
        self.mode == AgentMode::Responding
    }

    /// True when the session has ended.
    pub fn is_stopped(&self) -> bool {
        self.mode == AgentMode::Stopped
    }

    /// True for headless stream-transport sessions — no PTY exists to attach,
    /// so the chat view is transcript-only.
    pub fn is_stream(&self) -> bool {
        self.transport == "stream"
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

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    /// A tool call. `result` is the (truncated) tool output once it lands,
    /// prefixed with `error: ` when the tool failed. `edits` carries the
    /// `(old_string, new_string)` pairs of an Edit/MultiEdit call so the
    /// renderer can show a compact colored diff under the row.
    Tool {
        name: String,
        summary: String,
        result: Option<String>,
        edits: Vec<(String, String)>,
        /// The file this call changed, when it changed one — what feeds the
        /// per-turn changed-files view.
        changed: Option<ChangedFile>,
    },
}

/// One step of the agent's plan (Claude's `TodoWrite`, Codex's `update_plan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub content: String,
    pub status: PlanStatus,
    /// Present-tense phrasing for the step being worked on right now.
    pub active_form: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

impl PlanStatus {
    fn parse(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            _ => Self::Pending,
        }
    }
}

/// The agent's current plan — a last-write-wins snapshot, replaced whole
/// whenever the agent rewrites it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<PlanStep>,
    pub updated_at: Option<String>,
}

impl Plan {
    pub fn done(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.status == PlanStatus::Completed)
            .count()
    }

    /// The step in flight, if any — what the agent says it's doing right now.
    pub fn current(&self) -> Option<&PlanStep> {
        self.steps
            .iter()
            .find(|s| s.status == PlanStatus::InProgress)
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

fn plan_from_item(item: &Value) -> Plan {
    let steps = item
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|st| {
                    let content = st.get("content").and_then(|c| c.as_str())?.trim();
                    if content.is_empty() {
                        return None;
                    }
                    Some(PlanStep {
                        content: content.to_string(),
                        status: PlanStatus::parse(
                            st.get("status").and_then(|s| s.as_str()).unwrap_or(""),
                        ),
                        active_form: st
                            .get("activeForm")
                            .and_then(|a| a.as_str())
                            .map(str::trim)
                            .filter(|a| !a.is_empty())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Plan {
        steps,
        updated_at: item
            .get("updatedAt")
            .and_then(|u| u.as_str())
            .map(str::to_string),
    }
}

/// Flatten a conversation's turns into searchable lines for content search:
/// non-empty text lines plus a compact `name summary result` line per tool.
/// Each line is clipped and the total is capped so a huge transcript can't blow
/// up the index.
pub fn search_lines(turns: &[Turn]) -> Vec<String> {
    const MAX_LINES: usize = 3000;
    const MAX_LEN: usize = 200;
    let clip = |s: &str| -> String { s.chars().take(MAX_LEN).collect() };
    let mut out = Vec::new();
    for t in turns {
        for p in &t.parts {
            match p {
                Part::Text(s) => {
                    for line in s.lines() {
                        let l = line.trim();
                        if !l.is_empty() {
                            out.push(clip(l));
                        }
                    }
                }
                Part::Tool {
                    name,
                    summary,
                    result,
                    ..
                } => {
                    let mut s = format!("{name} {summary}");
                    if let Some(r) = result {
                        s.push(' ');
                        s.push_str(r);
                    }
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(clip(s));
                    }
                }
            }
            if out.len() >= MAX_LINES {
                return out;
            }
        }
    }
    out
}

/// Parse claudemon's `/conversation` payload (`{ items: [...] }`) into renderable
/// turns. Items are a flat, `kind`-tagged stream (user_message / assistant_text
/// / tool_use / tool_result / usage); consecutive same-role items coalesce into
/// one turn, and a `tool_result` attaches back to its `tool_use` by id — so the
/// parsed view shows tool *output*, not just the call (richer than the old
/// transcript path).
///
/// `transport` is the session's wire transport (see [`Agent::transport`]) — see
/// [`ConvFold`], which this is a one-shot wrapper around.
pub fn turns_from_conversation(v: &Value, transport: &str) -> Vec<Turn> {
    let mut fold = ConvFold::new(transport == "stream");
    fold.adopt_snapshot(v);
    fold.into_turns()
}

/// A conversation folded *incrementally* — the state a delta feed needs.
///
/// The daemon's `/conversation/stream` sends `{session_id, seq, reset, items}`
/// as they happen, so the client can't re-derive the fold from scratch each
/// time: joining a tool result to its call, deduping replayed ids, and gluing
/// per-token text fragments all need what came before. This holds exactly that
/// and nothing more.
///
/// The open assistant text run stays *pending* rather than being committed on
/// every fragment: `pending_text` is what the renderer draws as the live tail,
/// and it lands in a real `Part::Text` when something else closes the run. That
/// is also what preserves the one-trim-at-the-end rule — a fragment is never
/// trimmed individually, because whitespace-only tokens are the glue between
/// words.
pub struct ConvFold {
    /// Stream sessions store per-token fragments (concatenated verbatim); PTY
    /// sessions store whole blocks (joined with a blank line).
    stream: bool,
    turns: Vec<Turn>,
    /// The open assistant text run, untrimmed and uncommitted.
    pending: String,
    /// tool_use id → (turn index, part index), so a later result can attach.
    tool_loc: HashMap<String, (usize, usize)>,
    /// Every tool_use id seen — a second occurrence is always a replay.
    seen_tool_ids: HashSet<String>,
    /// Sequence of the last item applied (claudemon's per-session counter).
    seq: u64,
    /// Bumped whenever `turns` structurally changes (a turn or part appended, a
    /// tool result attached, a reset). The render memo keys off this so a
    /// streamed token — which only touches `pending` — no longer invalidates the
    /// wrapped render of the entire committed conversation. See
    /// `App::transcript_cache`.
    commits: u64,
    plan: Option<Plan>,
}

impl Turn {
    /// Files this turn changed, one entry per file with the edits summed.
    pub fn changes(&self) -> Vec<ChangedFile> {
        merge_changes(self.parts.iter().filter_map(|p| match p {
            Part::Tool { changed, .. } => changed.clone(),
            _ => None,
        }))
    }
}

impl ConvFold {
    pub fn new(stream: bool) -> Self {
        Self {
            stream,
            turns: Vec::new(),
            pending: String::new(),
            tool_loc: HashMap::new(),
            seen_tool_ids: HashSet::new(),
            seq: 0,
            commits: 0,
            plan: None,
        }
    }

    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// Monotonic counter of structural changes to `turns` — the cache key for
    /// anything that renders the committed conversation.
    pub fn commits(&self) -> u64 {
        self.commits
    }

    /// The live, uncommitted assistant text — trimmed for display only.
    pub fn pending_text(&self) -> Option<&str> {
        let t = self.pending.trim();
        (!t.is_empty()).then_some(t)
    }

    /// Whether a tool call has produced its result yet.
    ///
    /// `false` for an id we've never seen at all, which is the honest answer for
    /// a caller asking "is this finished?" — an unseen call is not a finished one.
    pub fn tool_settled(&self, tool_use_id: &str) -> bool {
        self.tool_loc
            .get(tool_use_id)
            .and_then(|&(ti, pi)| self.turns.get(ti).and_then(|t| t.parts.get(pi)))
            .is_some_and(|p| {
                matches!(
                    p,
                    Part::Tool {
                        result: Some(_),
                        ..
                    }
                )
            })
    }

    /// Every file the session has changed, most recently touched first, with
    /// each file's edits summed across turns.
    ///
    /// This is the agent's account of its own work, not the work tree's: it
    /// includes a file the agent edited and later reverted, and excludes one
    /// you changed by hand.
    pub fn session_changes(&self) -> Vec<ChangedFile> {
        merge_changes(self.turns.iter().rev().flat_map(|t| t.changes()))
    }

    /// The turns that changed something, newest first, paired with their changes.
    pub fn changed_turns(&self) -> Vec<(usize, Vec<ChangedFile>)> {
        self.turns
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(i, t)| {
                let changes = t.changes();
                (!changes.is_empty()).then_some((i, changes))
            })
            .collect()
    }

    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref().filter(|p| !p.is_empty())
    }

    /// Consume the fold for the one-shot case: commit the open run and hand
    /// back the turns.
    pub fn into_turns(mut self) -> Vec<Turn> {
        self.flush_text();
        self.turns
    }

    /// Adopt a full snapshot (`GET /sessions/:id/conversation`), discarding
    /// whatever was folded before. `seq` follows the snapshot's own count so a
    /// later delta can be sequenced against it.
    pub fn adopt_snapshot(&mut self, v: &Value) {
        let items = v.get("items").and_then(|i| i.as_array());
        let count = items.map(|a| a.len()).unwrap_or(0) as u64;
        self.clear();
        if let Some(items) = items {
            self.apply_items(items);
        }
        // Take the snapshot's own `seq`. The item count is NOT it: the daemon
        // folds streamed assistant-text fragments into one retained item while
        // `seq` keeps counting every fragment, so on any stream session the two
        // diverge by hundreds within a turn. Deriving seq from the count left
        // `apply_delta`'s continuity check permanently unsatisfiable — every
        // delta read as a gap, every gap triggered a snapshot refetch, and the
        // refetch restored the same wrong seq. That is a refetch per token, not
        // merely a redundant one.
        //
        // `count` remains the fallback for a daemon too old to send `seq`.
        self.seq = v.get("seq").and_then(|s| s.as_u64()).unwrap_or(count);
    }

    /// Apply one `conversation.delta` frame.
    ///
    /// Returns `false` when the frame can't be sequenced onto what we have —
    /// `seq` must equal `last_seq + items.len()`, per the daemon's contract —
    /// meaning a frame was missed and the caller must resync from the snapshot
    /// endpoint. A `reset` frame is always applicable: it means the log was
    /// rebuilt, so prior state is void by definition.
    pub fn apply_delta(&mut self, seq: u64, reset: bool, items: &[Value]) -> bool {
        if reset {
            self.clear();
            self.apply_items(items);
            self.seq = seq;
            return true;
        }
        // Already seen (a reconnect replays the tail) — not a gap, just old.
        if seq <= self.seq {
            return true;
        }
        if seq != self.seq + items.len() as u64 {
            return false;
        }
        self.apply_items(items);
        self.seq = seq;
        true
    }

    fn clear(&mut self) {
        self.commits += 1;
        self.turns.clear();
        self.pending.clear();
        self.tool_loc.clear();
        self.seen_tool_ids.clear();
        self.seq = 0;
        // The plan deliberately survives a reset: it is last-write-wins session
        // state that happens to ride the conversation, not a transcript row, and
        // a rebuilt log replays it anyway.
    }

    /// Append a part to the open turn, starting a new turn on a role change.
    fn push(&mut self, role: Role, part: Part) -> (usize, usize) {
        if self.turns.last().map(|t| t.role) != Some(role) {
            self.turns.push(Turn {
                role,
                parts: Vec::new(),
            });
        }
        let ti = self.turns.len() - 1;
        self.turns[ti].parts.push(part);
        self.commits += 1;
        (ti, self.turns[ti].parts.len() - 1)
    }

    /// Commit the open text run as one part, trimmed exactly once.
    fn flush_text(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending);
        let text = text.trim();
        if !text.is_empty() {
            self.push(Role::Assistant, Part::Text(text.to_string()));
        }
    }

    fn apply_items(&mut self, items: &[Value]) {
        for item in items {
            match item.get("kind").and_then(|k| k.as_str()).unwrap_or("") {
                "user_message" => {
                    self.flush_text();
                    let text = item
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .trim();
                    if text.is_empty() || is_meta_noise(text) {
                        continue;
                    }
                    self.push(Role::User, Part::Text(text.to_string()));
                }
                "assistant_text" => {
                    let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if self.stream {
                        // Never trim or drop a fragment — whitespace-only tokens
                        // are the joining glue between words.
                        self.pending.push_str(text);
                    } else {
                        let block = text.trim();
                        if !block.is_empty() {
                            if !self.pending.is_empty() {
                                self.pending.push_str("\n\n");
                            }
                            self.pending.push_str(block);
                        }
                    }
                }
                "tool_use" => {
                    // Dedup by id first, so a replayed call vanishes without even
                    // breaking the coalescing of the text around it.
                    let id = item
                        .get("id")
                        .and_then(|i| i.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    if let Some(id) = &id {
                        if !self.seen_tool_ids.insert(id.clone()) {
                            continue;
                        }
                    }
                    self.flush_text();
                    let name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("tool")
                        .to_string();
                    let summary = tool_summary(item.get("input"));
                    let edits = edit_pairs(item.get("input"));
                    let changed = changed_file(item.get("input"));
                    let loc = self.push(
                        Role::Assistant,
                        Part::Tool {
                            name,
                            summary,
                            result: None,
                            edits,
                            changed,
                        },
                    );
                    if let Some(id) = id {
                        self.tool_loc.insert(id, loc);
                    }
                }
                "tool_result" => {
                    self.flush_text();
                    let tid = item
                        .get("tool_use_id")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    let content = item
                        .get("content")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .trim();
                    let is_error = item
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);
                    if content.is_empty() {
                        continue;
                    }
                    if let Some(&(ti, pi)) = self.tool_loc.get(tid) {
                        if let Some(Part::Tool { result, .. }) =
                            self.turns.get_mut(ti).and_then(|t| t.parts.get_mut(pi))
                        {
                            let snippet = truncate(content, 200);
                            *result = Some(if is_error {
                                format!("error: {snippet}")
                            } else {
                                snippet
                            });
                            self.commits += 1;
                        }
                    }
                }
                "usage" => {
                    // Ignored entirely — claudemon interleaves a usage item before
                    // every PTY assistant row's text, so treating it as a boundary
                    // would defeat the blank-line block coalescing above.
                }
                "plan" => {
                    // Session state, not a transcript row: last-write-wins, and
                    // it must NOT close the open text run (the agent rewrites its
                    // plan mid-message all the time).
                    self.plan = Some(plan_from_item(item));
                }
                "slash_command" => {
                    // A slash-command run. On stream sessions it arrives twice
                    // (driver send echo + transcript tailer parse of the CLI's
                    // echo row) — the daemon doesn't dedup, so drop an identical
                    // repeat here, mirroring the desktop applier.
                    self.flush_text();
                    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    let args = item.get("args").and_then(|a| a.as_str()).unwrap_or("");
                    let line = if args.is_empty() {
                        format!("/{name}")
                    } else {
                        format!("/{name} {args}")
                    };
                    let dup = self
                        .turns
                        .last()
                        .filter(|t| t.role == Role::User)
                        .is_some_and(|t| {
                            t.parts
                                .iter()
                                .any(|p| matches!(p, Part::Text(s) if *s == line))
                        });
                    if !dup {
                        self.push(Role::User, Part::Text(line));
                    }
                }
                "command_output" => {
                    // The command's local output — render like a tool row so the
                    // snippet/expansion affordances come for free.
                    self.flush_text();
                    let output = item
                        .get("output")
                        .and_then(|o| o.as_str())
                        .unwrap_or("")
                        .trim();
                    if output.is_empty() {
                        continue;
                    }
                    let is_error = item
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);
                    let snippet = truncate(output, 200);
                    self.push(
                        Role::Assistant,
                        Part::Tool {
                            name: "command output".to_string(),
                            summary: String::new(),
                            result: Some(if is_error {
                                format!("error: {snippet}")
                            } else {
                                snippet
                            }),
                            edits: Vec::new(),
                            changed: None,
                        },
                    );
                }
                _ => {
                    // Any future kinds — still a boundary between assistant
                    // messages, so close any open text run.
                    self.flush_text();
                }
            }
        }
    }
}

/// The `(old_string, new_string)` pairs of an edit-tool input, if any:
/// a top-level `old_string`/`new_string` pair (Claude Edit), plus every entry
/// of an `edits` array (MultiEdit); or, for managed providers whose edits
/// arrive as a unified patch (Codex `apply_patch` carries `diff`), the
/// removed/added line groups of that patch. Empty for every other tool shape.
/// A file a tool call changed, with the size of the change.
///
/// Derived from the call's input rather than from git: this is what the *agent*
/// did in this turn, which is a different question from what the work tree looks
/// like now. A file the agent edited and then reverted shows up here anyway, and
/// that is the point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

impl ChangedFile {
    /// Last two path segments — enough to tell files apart without eating the row.
    pub fn short_path(&self) -> String {
        let parts: Vec<&str> = self.path.rsplit('/').take(2).collect();
        parts.into_iter().rev().collect::<Vec<_>>().join("/")
    }
}

/// Sum a set of changes per file, newest-wins on order of appearance. Used for
/// both a turn's changes and a whole session's.
pub fn merge_changes(changes: impl IntoIterator<Item = ChangedFile>) -> Vec<ChangedFile> {
    let mut out: Vec<ChangedFile> = Vec::new();
    for c in changes {
        match out.iter_mut().find(|e| e.path == c.path) {
            // The same file touched twice in a turn is one entry, both edits counted.
            Some(existing) => {
                existing.added += c.added;
                existing.removed += c.removed;
            }
            None => out.push(c),
        }
    }
    out
}

/// The file an edit-shaped tool call changed, and by how much.
///
/// `None` for a call that changes nothing (a read, a search) or whose target we
/// can't name — a count with no path to attach it to is worse than nothing.
fn changed_file(input: Option<&Value>) -> Option<ChangedFile> {
    let obj = input?.as_object()?;
    let path = ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|p| !p.is_empty())?;

    // Write replaces a whole file: every line is added, and there is no old text
    // to diff against, so the edit-pair path below would report nothing.
    if let Some(content) = obj.get("content").and_then(Value::as_str) {
        if !obj.contains_key("old_string") {
            return Some(ChangedFile {
                path: path.to_string(),
                added: line_count(content),
                removed: 0,
            });
        }
    }

    let pairs = edit_pairs(input);
    if pairs.is_empty() {
        return None;
    }
    let (mut added, mut removed) = (0, 0);
    for (old, new) in &pairs {
        removed += line_count(old);
        added += line_count(new);
    }
    Some(ChangedFile {
        path: path.to_string(),
        added,
        removed,
    })
}

/// Lines in a chunk of text. Empty text is zero lines, not one — an empty
/// `old_string` means "inserted here", with nothing removed.
fn line_count(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    s.lines().count()
}

fn edit_pairs(input: Option<&Value>) -> Vec<(String, String)> {
    let Some(obj) = input.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let pair_of = |o: &serde_json::Map<String, Value>| -> Option<(String, String)> {
        let old = o.get("old_string").and_then(|v| v.as_str())?;
        let new = o.get("new_string").and_then(|v| v.as_str())?;
        (!old.is_empty() || !new.is_empty()).then(|| (old.to_string(), new.to_string()))
    };
    let mut out = Vec::new();
    if let Some(p) = pair_of(obj) {
        out.push(p);
    }
    if let Some(edits) = obj.get("edits").and_then(|e| e.as_array()) {
        out.extend(
            edits
                .iter()
                .filter_map(|e| e.as_object())
                .filter_map(pair_of),
        );
    }
    if out.is_empty() {
        if let Some(diff) = obj.get("diff").and_then(|v| v.as_str()) {
            out = unified_diff_pairs(diff);
        }
    }
    out
}

/// Fold a unified patch into `(removed, added)` line groups, one pair per
/// contiguous `-`/`+` run, so it renders through the same colored-diff path as
/// Claude's Edit pairs. Headers (`diff`/`index`/`---`/`+++`/`@@`) and context
/// lines act as group separators and are dropped.
fn unified_diff_pairs(diff: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut old, mut new) = (String::new(), String::new());
    let flush = |old: &mut String, new: &mut String, out: &mut Vec<(String, String)>| {
        if !old.is_empty() || !new.is_empty() {
            out.push((std::mem::take(old), std::mem::take(new)));
        }
    };
    for line in diff.lines() {
        if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("@@")
            || line.starts_with("*** ")
        {
            flush(&mut old, &mut new, &mut out);
            continue;
        }
        if let Some(rest) = line.strip_prefix('-') {
            // A `-` after `+` starts a new change group, keeping hunk order.
            if !new.is_empty() {
                flush(&mut old, &mut new, &mut out);
            }
            old.push_str(rest);
            old.push('\n');
        } else if let Some(rest) = line.strip_prefix('+') {
            new.push_str(rest);
            new.push('\n');
        } else {
            flush(&mut old, &mut new, &mut out);
        }
    }
    flush(&mut old, &mut new, &mut out);
    out
}

/// Slash-command echoes, injected reminders, and background-task notifications
/// (emitted by workflows) aren't real conversation.
fn is_meta_noise(text: &str) -> bool {
    const TAGS: [&str; 5] = [
        "<local-command",
        "<command-name",
        "<command-message",
        "<system-reminder",
        "<task-notification",
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
        "file_path",
        "path",
        "command",
        "pattern",
        "query",
        "url",
        "prompt",
        "description",
    ];
    for k in KEYS {
        match obj.get(k) {
            Some(Value::String(s)) if !s.is_empty() => return truncate(s, 64),
            // Codex's `shell`/`exec_command` sends the command as an argv
            // array rather than a string.
            Some(Value::Array(parts)) => {
                let joined = parts
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !joined.is_empty() {
                    return truncate(&joined, 64);
                }
            }
            _ => {}
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

/// Compact window length for a status readout: `5h`, `7d`, `90m`. `None` when
/// the provider did not report a length, so callers keep the slot's own name.
pub fn window_short(mins: Option<u64>) -> Option<String> {
    let m = mins.filter(|m| *m > 0)?;
    Some(if m % 1440 == 0 {
        format!("{}d", m / 1440)
    } else if m % 60 == 0 {
        format!("{}h", m / 60)
    } else {
        format!("{m}m")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_item(path: &str, old: &str, new: &str) -> Value {
        serde_json::json!({
            "kind": "tool_use", "id": format!("t-{path}-{old}"), "name": "Edit",
            "input": { "file_path": path, "old_string": old, "new_string": new }
        })
    }

    /// The counts come from the edit's own text, so they describe what the agent
    /// did — not what the work tree looks like now.
    #[test]
    fn an_edit_reports_its_file_and_line_counts() {
        let mut f = ConvFold::new(true);
        f.apply_delta(
            1,
            false,
            &[edit_item("/repo/src/a.rs", "one\ntwo", "ONE\nTWO\nTHREE")],
        );

        let changes = f.session_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "/repo/src/a.rs");
        assert_eq!((changes[0].added, changes[0].removed), (3, 2));
        assert_eq!(
            changes[0].short_path(),
            "src/a.rs",
            "enough to tell files apart"
        );
    }

    /// A Write has no old text to diff against, so the edit-pair path would
    /// report nothing at all for the commonest way a file gets created.
    #[test]
    fn a_write_counts_the_whole_file_as_added() {
        let mut f = ConvFold::new(true);
        f.apply_delta(
            1,
            false,
            &[serde_json::json!({
                "kind": "tool_use", "id": "w1", "name": "Write",
                "input": { "file_path": "/repo/new.rs", "content": "a\nb\nc" }
            })],
        );
        let changes = f.session_changes();
        assert_eq!((changes[0].added, changes[0].removed), (3, 0));
    }

    #[test]
    fn a_read_only_call_changes_nothing() {
        let mut f = ConvFold::new(true);
        f.apply_delta(
            1,
            false,
            &[serde_json::json!({
                "kind": "tool_use", "id": "r1", "name": "Read",
                "input": { "file_path": "/repo/a.rs" }
            })],
        );
        assert!(f.session_changes().is_empty(), "a read is not a change");
    }

    #[test]
    fn a_codex_patch_counts_through_the_unified_diff() {
        let mut f = ConvFold::new(true);
        f.apply_delta(
            1,
            false,
            &[serde_json::json!({
                "kind": "tool_use", "id": "p1", "name": "apply_patch",
                "input": {
                    "path": "src/a.rs",
                    "diff": "--- a/src/a.rs\n+++ b/src/a.rs\n@@\n context\n-let x = 1;\n+let x = 2;\n+let y = 3;\n"
                }
            })],
        );
        let changes = f.session_changes();
        assert_eq!(changes[0].path, "src/a.rs");
        assert_eq!((changes[0].added, changes[0].removed), (2, 1));
    }

    /// One file edited twice in a turn is one row with both edits counted —
    /// listing it twice would misreport the size of the change.
    #[test]
    fn repeat_edits_to_one_file_collapse_into_one_row() {
        let mut f = ConvFold::new(true);
        f.apply_delta(
            2,
            false,
            &[
                edit_item("/repo/a.rs", "x", "X"),
                edit_item("/repo/a.rs", "y", "Y\nZ"),
            ],
        );
        let turn_changes = f.turns()[0].changes();
        assert_eq!(turn_changes.len(), 1);
        assert_eq!((turn_changes[0].added, turn_changes[0].removed), (3, 2));
    }

    /// Turns are reported newest first: what just changed is the interesting part.
    #[test]
    fn changed_turns_are_newest_first_and_skip_turns_that_changed_nothing() {
        let mut f = ConvFold::new(true);
        f.apply_delta(1, false, &[edit_item("/repo/first.rs", "a", "A")]);
        // A turn with only prose changes nothing.
        f.apply_delta(
            2,
            false,
            &[serde_json::json!({ "kind": "user_message", "text": "go on" })],
        );
        f.apply_delta(3, false, &[edit_item("/repo/second.rs", "b", "B")]);

        let turns = f.changed_turns();
        assert_eq!(turns.len(), 2, "the prose turn is not listed");
        assert_eq!(turns[0].1[0].path, "/repo/second.rs", "newest first");
        assert_eq!(turns[1].1[0].path, "/repo/first.rs");

        // The session view merges them, most recently touched first.
        let session = f.session_changes();
        assert_eq!(session.len(), 2);
        assert_eq!(session[0].path, "/repo/second.rs");
    }

    #[test]
    fn edit_pairs_reads_codex_unified_patch() {
        // Codex `apply_patch` ToolUse input: no old_string/new_string, the
        // change arrives as a unified `diff` (concatenated per-file patches).
        let input = serde_json::json!({
            "path": "src/a.rs",
            "diff": "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@\n context\n-let x = 1;\n+let x = 2;\n context\n@@ -9,2 +9,3 @@\n+let added = true;\n",
            "changes": [{ "path": "src/a.rs", "kind": "update" }]
        });
        let pairs = edit_pairs(Some(&input));
        assert_eq!(
            pairs,
            vec![
                ("let x = 1;\n".to_string(), "let x = 2;\n".to_string()),
                (String::new(), "let added = true;\n".to_string()),
            ]
        );
    }

    #[test]
    fn edit_pairs_prefers_claude_shape_over_diff() {
        let input = serde_json::json!({
            "old_string": "a",
            "new_string": "b",
            "diff": "-x\n+y\n"
        });
        assert_eq!(
            edit_pairs(Some(&input)),
            vec![("a".to_string(), "b".to_string())]
        );
    }

    #[test]
    fn tool_summary_joins_argv_arrays() {
        // Codex `shell` sends the command as argv, not a string.
        let input = serde_json::json!({ "command": ["bash", "-c", "ls -la"] });
        assert_eq!(tool_summary(Some(&input)), "bash -c ls -la");
    }

    fn frag(text: &str) -> Value {
        serde_json::json!({ "kind": "assistant_text", "text": text })
    }

    /// The point of the fold being stateful: a token arrives, the open message
    /// grows, and nothing is re-derived. Fragments are glued verbatim — a
    /// whitespace-only token is the space between two words — and the whole run
    /// is trimmed exactly once, when something closes it.
    #[test]
    fn streamed_fragments_grow_the_open_message_without_committing_it() {
        let mut f = ConvFold::new(true);
        assert!(f.apply_delta(1, false, &[frag("Hel")]));
        assert_eq!(f.pending_text(), Some("Hel"));
        assert!(f.turns().is_empty(), "nothing committed mid-message");

        assert!(f.apply_delta(3, false, &[frag("lo,"), frag(" wor")]));
        assert!(f.apply_delta(4, false, &[frag("ld")]));
        assert_eq!(f.pending_text(), Some("Hello, world"));
        assert!(f.turns().is_empty());

        // A user message closes the run, committing it as one turn.
        assert!(f.apply_delta(
            5,
            false,
            &[serde_json::json!({ "kind": "user_message", "text": "next" })]
        ));
        assert_eq!(f.pending_text(), None);
        assert_eq!(f.turns().len(), 2);
        assert_eq!(
            f.turns()[0].parts,
            vec![Part::Text("Hello, world".into())],
            "glued verbatim, trimmed once"
        );
    }

    /// claudemon's contract: `seq` is the sequence of the LAST item in the
    /// frame, so a frame is applicable only when it continues from what we have.
    /// A hole means a frame was dropped and the client must resync — rendering a
    /// transcript with a gap in it would be worse than refetching.
    #[test]
    fn a_gap_in_the_sequence_is_reported_rather_than_papered_over() {
        let mut f = ConvFold::new(true);
        assert!(f.apply_delta(2, false, &[frag("a"), frag("b")]));

        // seq 5 after 2 means items 3 and 4 never arrived.
        assert!(
            !f.apply_delta(5, false, &[frag("e")]),
            "a gap must be reported"
        );
        // …and the fold is left untouched, so the resync has a clean base.
        assert_eq!(f.pending_text(), Some("ab"));

        // The very next contiguous frame still applies.
        assert!(f.apply_delta(3, false, &[frag("c")]));
        assert_eq!(f.pending_text(), Some("abc"));
    }

    /// A reconnect replays the tail, so already-seen frames must be idempotent
    /// rather than counted as gaps or duplicated into the transcript.
    #[test]
    fn a_replayed_frame_is_dropped_not_duplicated() {
        let mut f = ConvFold::new(true);
        assert!(f.apply_delta(1, false, &[frag("x")]));
        assert!(f.apply_delta(1, false, &[frag("x")]), "not a gap");
        assert_eq!(f.pending_text(), Some("x"), "and not applied twice");
    }

    /// `reset` means the log was rebuilt (transcript replaced or truncated), so
    /// prior state is void by definition and the frame always applies.
    #[test]
    fn a_reset_frame_replaces_everything_and_always_applies() {
        let mut f = ConvFold::new(true);
        f.apply_delta(2, false, &[frag("stale"), frag(" text")]);
        assert!(f.apply_delta(1, true, &[frag("fresh")]));
        assert_eq!(f.pending_text(), Some("fresh"));
        assert!(f.turns().is_empty());
    }

    /// A tool result can land in a later frame than its call, which is exactly
    /// what the retained id→location map is for.
    #[test]
    fn a_tool_result_attaches_to_a_call_from_an_earlier_frame() {
        let mut f = ConvFold::new(true);
        f.apply_delta(
            1,
            false,
            &[serde_json::json!({
                "kind": "tool_use", "id": "t1", "name": "Bash",
                "input": { "command": "ls" }
            })],
        );
        f.apply_delta(2, false, &[frag("thinking…")]);
        f.apply_delta(
            3,
            false,
            &[serde_json::json!({
                "kind": "tool_result", "tool_use_id": "t1", "content": "a.txt"
            })],
        );

        let tool = f
            .turns()
            .iter()
            .flat_map(|t| &t.parts)
            .find_map(|p| match p {
                Part::Tool { name, result, .. } if name == "Bash" => Some(result.clone()),
                _ => None,
            })
            .expect("the Bash call is in the transcript");
        assert_eq!(tool.as_deref(), Some("a.txt"));
    }

    /// The plan is session state that happens to ride the conversation: it must
    /// replace wholesale, and it must NOT close the open assistant message (the
    /// agent rewrites its plan mid-sentence all the time).
    #[test]
    fn a_plan_item_updates_the_plan_without_breaking_the_open_message() {
        let mut f = ConvFold::new(true);
        f.apply_delta(1, false, &[frag("working")]);
        f.apply_delta(
            2,
            false,
            &[serde_json::json!({
                "kind": "plan",
                "updatedAt": "2026-07-25T00:00:00Z",
                "steps": [
                    { "content": "one", "status": "completed" },
                    { "content": "two", "status": "in_progress", "activeForm": "doing two" },
                    { "content": "three", "status": "pending" }
                ]
            })],
        );
        f.apply_delta(3, false, &[frag(" on it")]);

        assert_eq!(f.pending_text(), Some("working on it"), "message intact");
        let plan = f.plan().expect("plan published");
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.done(), 1);
        assert_eq!(plan.current().map(|s| s.content.as_str()), Some("two"));
        assert_eq!(
            plan.current().and_then(|s| s.active_form.as_deref()),
            Some("doing two")
        );

        // Last-write-wins: a rewrite replaces the whole plan.
        f.apply_delta(
            4,
            false,
            &[serde_json::json!({
                "kind": "plan",
                "steps": [{ "content": "only", "status": "completed" }]
            })],
        );
        assert_eq!(f.plan().unwrap().steps.len(), 1);
        assert_eq!(f.plan().unwrap().done(), 1);
    }

    /// An empty plan reads as "no plan" rather than an empty panel.
    #[test]
    fn an_empty_plan_is_treated_as_absent() {
        let mut f = ConvFold::new(true);
        f.apply_delta(
            1,
            false,
            &[serde_json::json!({ "kind": "plan", "steps": [] })],
        );
        assert!(f.plan().is_none());
    }

    #[test]
    fn unified_diff_pairs_starts_new_group_when_minus_follows_plus() {
        // An add-then-remove hunk must stay two ordered pairs — the `-` line
        // after a `+` run starts a new group instead of merging into one
        // ("removed", "added") pair, which would invert the diff's hunk order.
        let input = serde_json::json!({ "diff": "+added\n-removed\n" });
        assert_eq!(
            edit_pairs(Some(&input)),
            vec![
                (String::new(), "added\n".to_string()),
                ("removed\n".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn unified_diff_pairs_keeps_deletion_only_groups() {
        // Pure deletions produce pairs with an empty new-side and must not be
        // dropped; a patch ending in `-` lines relies on the trailing flush.
        assert_eq!(
            unified_diff_pairs("@@\n-gone1\n ctx\n-gone2\n"),
            vec![
                ("gone1\n".to_string(), String::new()),
                ("gone2\n".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn tool_summary_falls_through_when_argv_join_is_empty() {
        // An empty argv array yields an empty join — the scan must continue to
        // a later usable key rather than returning the blank summary.
        let input = serde_json::json!({ "command": [], "description": "list files" });
        assert_eq!(tool_summary(Some(&input)), "list files");

        // Same for an argv of non-strings (`query` comes after `command` in
        // the KEYS scan, so this only passes if the empty join falls through).
        let input = serde_json::json!({ "command": [1, 2], "query": "/a" });
        assert_eq!(tool_summary(Some(&input)), "/a");
    }

    #[test]
    fn parses_live_sessions_list_shape() {
        // Exactly what claudemon's GET /sessions returns (with the new usage block).
        let json = serde_json::json!([{
            "session_id": "abc",
            "cwd": "/home/u/proj",
            "mode": "responding",
            "pending": null,
            "started_at": "2026-06-04T03:00:00Z",
            "updated_at": "2026-06-04T03:00:10Z",
            "tool_calls": 3,
            "last_event": "PreToolUse",
            "usage": {
                "model": "claude-sonnet-4-6",
                "context_tokens": 5200,
                "context_limit": 200000,
                "cost_usd": 0.042
            }
        }]);
        let agents: Vec<Agent> = serde_json::from_value(json).unwrap();
        assert_eq!(agents.len(), 1);
        let a = &agents[0];
        assert_eq!(a.session_id, "abc");
        assert_eq!(a.state(), "responding");
        assert!(a.is_busy() && !a.is_waiting());
        assert_eq!(a.short_cwd(), "…/u/proj");
        assert!(a.approval().is_none() && a.questions().is_none());
        // usage comes straight from the API now — no transcript fetch needed.
        let u = a.usage.as_ref().expect("usage present");
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(u.context_tokens, 5200);
        assert_eq!(u.context_limit, Some(200_000));
        assert!((u.cost_usd - 0.042).abs() < 1e-9);
    }

    #[test]
    fn parses_sessions_list_without_usage() {
        // Daemons that don't yet emit the usage block must still deserialize.
        let json = serde_json::json!([{
            "session_id": "abc",
            "mode": "responding",
        }]);
        let agents: Vec<Agent> = serde_json::from_value(json).unwrap();
        assert!(agents[0].usage.is_none());
    }

    #[test]
    fn provider_wire_field_parses_and_defaults_to_claude() {
        // Explicit provider is carried through verbatim.
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "x", "mode": "input", "provider": "codex"
        }))
        .unwrap();
        assert_eq!(a.provider, "codex");

        // Absent provider (older daemon / un-managed PTY) defaults to "claude".
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "y", "mode": "input"
        }))
        .unwrap();
        assert_eq!(a.provider, "claude");
    }

    #[test]
    fn derive_stats_prefers_statusline_then_falls_back_to_usage() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s",
            "mode": "responding",
            "usage": { "model": "claude-sonnet-4-6", "context_tokens": 50_000,
                       "context_limit": 200_000, "cost_usd": 1.0 }
        }))
        .unwrap();

        // No statusLine → transcript usage fallback (25% ctx, $1.00).
        let d = derive_stats(&agent, None);
        assert_eq!(d.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(d.context_pct, Some(25.0));
        assert_eq!(d.cost, Some(1.0));

        // statusLine present → its authoritative values win.
        let sl = StatusLine {
            model_display: Some("Opus 4.8".into()),
            context_used_pct: Some(73.0),
            context_window_size: Some(200_000),
            cost_usd: Some(12.5),
            ..Default::default()
        };
        let d = derive_stats(&agent, Some(&sl));
        assert_eq!(d.model.as_deref(), Some("Opus 4.8"));
        assert_eq!(d.context_pct, Some(73.0));
        assert_eq!(d.cost, Some(12.5));
    }

    #[test]
    fn derive_stats_rejects_a_disproved_200k_status_pair() {
        // This is the live Phase-2 specimen: the owner has resolved 1M from a
        // legacy `opus[1m]` request (or the equivalent canonical pair), while
        // Claude's raw status line still says 200K/100%. The same usage shape
        // also covers a native-1M Fable/Mythos model: clients consume the
        // owner's resolved window and never reverse-engineer its provenance.
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s",
            "mode": "responding",
            "usage": { "model": "claude-opus-5", "context_tokens": 356_380,
                       "context_limit": 1_000_000, "cost_usd": 1.0 }
        }))
        .unwrap();
        let sl: StatusLine = serde_json::from_value(serde_json::json!({
            "context_used_pct": 100.0,
            "context_window_size": 200_000
        }))
        .unwrap();

        assert_eq!(sl.context_window_size, Some(200_000));
        let pct = derive_stats(&agent, Some(&sl)).context_pct.unwrap();
        assert!((pct - 35.638).abs() < 1e-9, "pct={pct}");
    }

    #[test]
    fn derive_stats_preserves_a_truthful_200k_status_pair() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s",
            "mode": "responding",
            "usage": { "model": "claude-sonnet-5", "context_tokens": 90_000,
                       "context_limit": 200_000, "cost_usd": 1.0 }
        }))
        .unwrap();
        let sl = StatusLine {
            context_used_pct: Some(45.0),
            context_window_size: Some(200_000),
            ..Default::default()
        };

        assert_eq!(derive_stats(&agent, Some(&sl)).context_pct, Some(45.0));
    }

    #[test]
    fn derive_stats_does_not_promote_an_unknown_window() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s",
            "mode": "responding",
            "usage": { "model": "unknown-model", "context_tokens": 356_380,
                       "cost_usd": 1.0 }
        }))
        .unwrap();
        let sl = StatusLine {
            context_used_pct: Some(100.0),
            context_window_size: Some(200_000),
            ..Default::default()
        };

        assert_eq!(derive_stats(&agent, Some(&sl)).context_pct, None);
    }

    #[test]
    fn derive_stats_uses_the_same_two_percent_drift_boundary_as_desktop() {
        let at = |held: u64| {
            let agent: Agent = serde_json::from_value(serde_json::json!({
                "session_id": "s", "mode": "responding",
                "usage": { "context_tokens": held, "context_limit": 1_000_000 }
            }))
            .unwrap();
            let sl = StatusLine {
                context_used_pct: Some(100.0),
                context_window_size: Some(200_000),
                ..Default::default()
            };
            derive_stats(&agent, Some(&sl)).context_pct
        };

        assert_eq!(
            at(204_000),
            Some(100.0),
            "the exact boundary still trusts 200K"
        );
        assert_eq!(at(204_001), Some(20.4001), "one token past rejects it");
    }

    /// VERSION SKEW, the whole reason `context_limit` is an `Option` here.
    ///
    /// A newer claudemon OMITS `context_limit` when it does not know the
    /// window (it is `skip_serializing_if = "Option::is_none"` on the daemon's
    /// `Usage`). While this field was a required `u64`, the missing key failed
    /// the whole `Usage` deserialize on an old TUI — so an upgraded daemon did
    /// not merely hide one context meter, it blanked model, cost and tokens
    /// for every session at once. The block must survive the absence, and the
    /// window must read as unknown rather than as a zero-width one.
    #[test]
    fn usage_survives_a_daemon_that_omits_the_context_window() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s",
            "mode": "responding",
            // No `context_limit` key at all — the new daemon's honest unknown.
            "usage": { "model": "gpt-5-codex", "context_tokens": 12_000, "cost_usd": 0.25 }
        }))
        .unwrap();
        let u = agent.usage.as_ref().expect("usage still deserializes");
        assert_eq!(u.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(u.context_tokens, 12_000, "tokens survive the missing key");
        assert!((u.cost_usd - 0.25).abs() < 1e-9, "cost survives it too");
        assert_eq!(u.context_limit, None, "unknown, not zero");
        // And the readout omits the percentage rather than dividing by nothing.
        assert_eq!(derive_stats(&agent, None).context_pct, None);
    }

    // ── the owner's canonical model facts ───────────────────────────────────

    /// Both spellings deserialize into the same fields: claudemon's own
    /// snake_case (`GET /sessions`, the `--direct` path) and the hub's
    /// camelCase projection (a bus row). Nothing about the session changes
    /// because it arrived by a different road.
    #[test]
    fn owner_selection_parses_in_both_wire_spellings() {
        let snake: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "input",
            "requested_model": "opus[1m]",
            "requested_selection": { "model": "opus", "context_window": 1_000_000 },
            "resolved_context_window": 1_000_000
        }))
        .unwrap();
        let camel: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "input",
            "requestedSelection": { "model": "opus", "contextWindow": 1_000_000 },
            "resolvedContextWindow": 1_000_000
        }))
        .unwrap();

        for a in [&snake, &camel] {
            let sel = a.requested_selection.as_ref().expect("selection");
            assert_eq!(sel.model.as_deref(), Some("opus"));
            assert_eq!(sel.context_window, Some(1_000_000));
            assert_eq!(a.resolved_context_window, Some(1_000_000));
        }
    }

    /// Absence is the common case and must stay silent: a daemon that predates
    /// the slice, and a session nobody pinned a selection on. A sparse
    /// selection (identity, no window) is preserved as such — this client never
    /// completes the pair from the resolved window sitting beside it.
    #[test]
    fn owner_selection_tolerates_absence_and_sparseness() {
        let bare: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "input",
            "usage": { "model": "claude-opus-5", "context_tokens": 10, "context_limit": 200_000 }
        }))
        .unwrap();
        assert!(bare.requested_selection.is_none());
        assert_eq!(bare.resolved_context_window, None);
        assert_eq!(
            bare.owner_context_window(),
            Some(200_000),
            "the usage window still answers on its own"
        );

        let sparse: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "input",
            "requested_selection": { "model": "opus" },
            "resolved_context_window": 200_000
        }))
        .unwrap();
        let sel = sparse.requested_selection.as_ref().expect("selection");
        assert_eq!(sel.model.as_deref(), Some("opus"));
        assert_eq!(sel.context_window, None, "unresolved stays unresolved");
    }

    /// The resolved window is a DENOMINATOR, and only that. A row whose usage
    /// block has no window of its own measures against it; a row with one keeps
    /// using it (they are the same number from the same resolver, and the usage
    /// one travels with the tokens it belongs to).
    #[test]
    fn resolved_window_is_a_display_denominator_when_usage_has_none() {
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "responding",
            "resolved_context_window": 1_000_000,
            // The daemon's honest unknown: no `context_limit` key at all.
            "usage": { "model": "claude-opus-5", "context_tokens": 250_000, "cost_usd": 1.0 }
        }))
        .unwrap();
        assert_eq!(a.owner_context_window(), Some(1_000_000));
        assert_eq!(derive_stats(&a, None).context_pct, Some(25.0));

        // Nothing resolved anything: still unknown, never a guessed 200k.
        let unknown: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "responding",
            "usage": { "model": "unknown-model", "context_tokens": 250_000 }
        }))
        .unwrap();
        assert_eq!(unknown.owner_context_window(), None);
        assert_eq!(derive_stats(&unknown, None).context_pct, None);

        // A request is NOT a resolution: a window that was asked for but never
        // resolved cannot become a denominator.
        let asked: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "responding",
            "requested_selection": { "model": "opus", "context_window": 1_000_000 },
            "usage": { "model": "claude-opus-5", "context_tokens": 250_000 }
        }))
        .unwrap();
        assert_eq!(asked.owner_context_window(), None);
        assert_eq!(derive_stats(&asked, None).context_pct, None);
    }

    /// The early-1M specimen, with the resolved window as the ONLY source of
    /// the denominator: 356,380 tokens held while Claude's status line still
    /// says 200,000 at 100%. The raw provider claim is what gets disproved (2%
    /// drift), and the resolved window is what the readout then measures
    /// against — it never rescues the contradicted pair, and never suppresses
    /// the contradiction check either.
    #[test]
    fn resolved_window_does_not_soften_the_disproved_status_pair() {
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "responding",
            "requested_selection": { "model": "opus", "context_window": 1_000_000 },
            "resolved_context_window": 1_000_000,
            "usage": { "model": "claude-opus-5", "context_tokens": 356_380, "cost_usd": 1.0 }
        }))
        .unwrap();
        let sl: StatusLine = serde_json::from_value(serde_json::json!({
            "context_used_pct": 100.0,
            "context_window_size": 200_000
        }))
        .unwrap();

        let pct = derive_stats(&a, Some(&sl)).context_pct.unwrap();
        assert!((pct - 35.638).abs() < 1e-9, "pct={pct}");
        assert_eq!(
            sl.context_window_size,
            Some(200_000),
            "the provider's own claim is untouched evidence"
        );

        // The mirror image: a status pair the tokens do NOT disprove still
        // wins, even with a resolved window that disagrees with it.
        let truthful: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "responding",
            "resolved_context_window": 1_000_000,
            "usage": { "model": "claude-opus-5", "context_tokens": 90_000 }
        }))
        .unwrap();
        let sl = StatusLine {
            context_used_pct: Some(45.0),
            context_window_size: Some(200_000),
            ..Default::default()
        };
        assert_eq!(
            derive_stats(&truthful, Some(&sl)).context_pct,
            Some(45.0),
            "an uncontradicted provider percentage remains authoritative"
        );
    }

    #[test]
    fn cache_report_prefers_the_itemized_split_and_omits_what_is_unreported() {
        let agent: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s",
            "mode": "responding",
            "usage": { "model": "claude-opus-5", "context_tokens": 40_984,
                       "context_limit": 200_000, "cost_usd": 1.0,
                       "cache": { "fresh": 2, "write": 23_393, "read": 17_589 } }
        }))
        .unwrap();
        let c = cache_report(&agent, None).expect("itemized split");
        assert_eq!((c.fresh, c.write, c.read), (2, Some(23_393), 17_589));
        assert_eq!(c.total(), 40_984);
        let pct = c.hit_rate_pct().unwrap();
        assert!(
            (pct - 17_589.0 / 40_984.0 * 100.0).abs() < 1e-9,
            "pct={pct}"
        );

        // Codex: a cache-read subset of the input, and nothing about writes.
        // `write` stays None so the detail pane drops the figure rather than
        // claiming the session wrote nothing.
        let bare: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "s", "mode": "responding"
        }))
        .unwrap();
        let sl = StatusLine {
            total_input_tokens: Some(4_402_946),
            cached_input_tokens: Some(3_733_376),
            ..Default::default()
        };
        let c = cache_report(&bare, Some(&sl)).expect("codex split");
        assert_eq!((c.fresh, c.write, c.read), (669_570, None, 3_733_376));

        // Nothing reported → no readout at all.
        assert!(cache_report(&bare, None).is_none());
        assert!(cache_report(
            &bare,
            Some(&StatusLine {
                total_input_tokens: Some(50_000),
                ..Default::default()
            })
        )
        .is_none());

        // A reported-but-empty split has no hit rate: the denominator is zero,
        // which makes the share undefined rather than 0%.
        let empty = CacheReport {
            fresh: 0,
            write: Some(0),
            read: 0,
        };
        assert!(empty.hit_rate_pct().is_none());
    }

    #[test]
    fn conversation_groups_turns_and_attaches_tool_results() {
        let v = serde_json::json!({ "items": [
            { "kind": "user_message", "text": "do it" },
            { "kind": "assistant_text", "text": "on it" },
            { "kind": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"} },
            { "kind": "tool_result", "tool_use_id": "t1", "content": "a\nb\nc", "is_error": false },
            { "kind": "usage", "usage": {} },
            { "kind": "tool_use", "id": "t2", "name": "Edit", "input": {"file_path": "/x.rs"} },
            { "kind": "tool_result", "tool_use_id": "t2", "content": "boom", "is_error": true },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(
            turns.len(),
            2,
            "one user turn, one coalesced assistant turn"
        );
        assert_eq!(turns[0].role, Role::User);
        assert_eq!(turns[1].role, Role::Assistant);
        // assistant turn: text + 2 tools (usage skipped, results attached, not parts)
        let parts = &turns[1].parts;
        assert_eq!(parts.len(), 3);
        match &parts[1] {
            Part::Tool { name, result, .. } => {
                assert_eq!(name, "Bash");
                assert_eq!(result.as_deref(), Some("a\nb\nc"));
            }
            other => panic!("expected Bash tool, got {other:?}"),
        }
        match &parts[2] {
            Part::Tool { name, result, .. } => {
                assert_eq!(name, "Edit");
                assert_eq!(result.as_deref(), Some("error: boom"), "errors prefixed");
            }
            other => panic!("expected Edit tool, got {other:?}"),
        }
    }

    #[test]
    fn slash_command_items_render_as_user_command_line_plus_output_row() {
        let v = serde_json::json!({ "items": [
            // Stream driver echo + tailer parse of the same run — deduped.
            { "kind": "slash_command", "name": "context" },
            { "kind": "slash_command", "name": "context",
              "timestamp": "2026-07-14T10:00:01Z" },
            { "kind": "command_output", "output": "## Context Usage\n24.5k", "is_error": false },
        ]});
        let turns = turns_from_conversation(&v, "stream");
        assert_eq!(turns.len(), 2, "one user turn, one output row");
        assert_eq!(turns[0].role, Role::User);
        assert_eq!(turns[0].parts.len(), 1, "echo pair deduped");
        assert!(matches!(&turns[0].parts[0], Part::Text(t) if t == "/context"));
        match &turns[1].parts[0] {
            Part::Tool { name, result, .. } => {
                assert_eq!(name, "command output");
                assert_eq!(result.as_deref(), Some("## Context Usage\n24.5k"));
            }
            other => panic!("expected command output row, got {other:?}"),
        }
        // Args make it a distinct run, and stderr gets the error prefix.
        let v = serde_json::json!({ "items": [
            { "kind": "slash_command", "name": "btw", "args": "ready?" },
            { "kind": "command_output", "output": "nope", "is_error": true },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert!(matches!(&turns[0].parts[0], Part::Text(t) if t == "/btw ready?"));
        assert!(matches!(
            &turns[1].parts[0],
            Part::Tool { result: Some(r), .. } if r == "error: nope"
        ));
    }

    #[test]
    fn codex_conversation_folds_to_tool_parts_with_edits_and_argv_summary() {
        // Pins the claudemon→TUI wire contract for codex stream-transport tool
        // calls: arguments ride under `input`, `apply_patch` carries a string
        // `diff` (not just per-file `changes[]`), and `shell` sends argv.
        let v = serde_json::json!({ "items": [
            { "kind": "tool_use", "id": "c1", "name": "apply_patch",
              "input": {
                  "path": "src/a.rs",
                  "diff": "@@ -1,2 +1,2 @@\n-let x = 1;\n+let x = 2;\n",
                  "changes": [{ "path": "src/a.rs", "kind": "update" }]
              } },
            { "kind": "tool_use", "id": "c2", "name": "shell",
              "input": { "command": ["bash", "-c", "ls"] } },
            { "kind": "tool_result", "tool_use_id": "c2", "content": "a.rs", "is_error": false },
        ]});
        let turns = turns_from_conversation(&v, "stream");
        assert_eq!(turns.len(), 1, "one coalesced assistant turn");
        let parts = &turns[0].parts;
        assert_eq!(parts.len(), 2);
        match &parts[0] {
            Part::Tool {
                name,
                summary,
                result,
                edits,
                ..
            } => {
                assert_eq!(name, "apply_patch");
                assert_eq!(summary, "src/a.rs");
                assert_eq!(
                    edits,
                    &vec![("let x = 1;\n".to_string(), "let x = 2;\n".to_string())],
                    "the unified `diff` folds into colored-diff pairs"
                );
                assert!(result.is_none(), "the result belongs to the shell call");
            }
            other => panic!("expected apply_patch tool, got {other:?}"),
        }
        match &parts[1] {
            Part::Tool {
                name,
                summary,
                result,
                edits,
                ..
            } => {
                assert_eq!(name, "shell");
                assert_eq!(summary, "bash -c ls", "argv arrays join into a summary");
                assert_eq!(
                    result.as_deref(),
                    Some("a.rs"),
                    "result keyed by tool_use_id"
                );
                assert!(edits.is_empty());
            }
            other => panic!("expected shell tool, got {other:?}"),
        }
    }

    #[test]
    fn conversation_filters_injected_meta_user_text() {
        let v = serde_json::json!({ "items": [
            { "kind": "user_message", "text": "<system-reminder>noise</system-reminder>" },
            { "kind": "user_message", "text": "real" },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns.len(), 1);
        assert!(matches!(&turns[0].parts[0], Part::Text(t) if t == "real"));
    }

    #[test]
    fn transport_wire_field_parses_and_defaults_to_pty() {
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "x", "mode": "input", "transport": "stream"
        }))
        .unwrap();
        assert_eq!(a.transport, "stream");
        assert!(a.is_stream());

        // Absent transport (older daemon / PTY session) defaults to "pty".
        let a: Agent = serde_json::from_value(serde_json::json!({
            "session_id": "y", "mode": "input"
        }))
        .unwrap();
        assert_eq!(a.transport, "pty");
        assert!(!a.is_stream());
    }

    #[test]
    fn redelivered_tool_use_dedups_by_id() {
        // Transcript compaction / resume replays re-deliver the same call; ids
        // are globally unique, so the second occurrence folds into nothing.
        let v = serde_json::json!({ "items": [
            { "kind": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"} },
            { "kind": "tool_result", "tool_use_id": "toolu_1", "content": "ok" },
            { "kind": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"} },
            { "kind": "tool_use", "id": "toolu_2", "name": "Read", "input": {"file_path": "/f"} },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].parts.len(), 2, "the duplicate call is dropped");
        assert!(matches!(&turns[0].parts[0], Part::Tool { name, .. } if name == "Bash"));
        assert!(matches!(&turns[0].parts[1], Part::Tool { name, .. } if name == "Read"));
    }

    #[test]
    fn stream_transport_concats_fragments_verbatim() {
        // Stream sessions store per-token fragments; they must join into ONE
        // text part with no per-fragment trimming ("Hello" + " " + "world").
        let v = serde_json::json!({ "items": [
            { "kind": "assistant_text", "text": "Hel" },
            { "kind": "assistant_text", "text": "lo" },
            { "kind": "assistant_text", "text": " " },
            { "kind": "assistant_text", "text": "world" },
            { "kind": "assistant_text", "text": "\n" },
        ]});
        let turns = turns_from_conversation(&v, "stream");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].parts.len(), 1, "fragments coalesce into one part");
        assert!(
            matches!(&turns[0].parts[0], Part::Text(t) if t == "Hello world"),
            "verbatim concat, trimmed once at the end: {:?}",
            turns[0].parts[0]
        );
    }

    #[test]
    fn stream_fragments_split_on_intervening_items() {
        // A tool call between text runs is a real boundary — two text parts.
        let v = serde_json::json!({ "items": [
            { "kind": "assistant_text", "text": "first " },
            { "kind": "assistant_text", "text": "message" },
            { "kind": "tool_use", "id": "t1", "name": "Bash", "input": {} },
            { "kind": "assistant_text", "text": "second" },
        ]});
        let turns = turns_from_conversation(&v, "stream");
        let parts = &turns[0].parts;
        assert_eq!(parts.len(), 3);
        assert!(matches!(&parts[0], Part::Text(t) if t == "first message"));
        assert!(matches!(&parts[2], Part::Text(t) if t == "second"));
    }

    #[test]
    fn pty_transport_joins_whole_blocks_with_a_blank_line() {
        let v = serde_json::json!({ "items": [
            { "kind": "assistant_text", "text": "block one\n" },
            { "kind": "assistant_text", "text": "  block two  " },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns[0].parts.len(), 1, "blocks coalesce into one part");
        assert!(
            matches!(&turns[0].parts[0], Part::Text(t) if t == "block one\n\nblock two"),
            "got {:?}",
            turns[0].parts[0]
        );
    }

    #[test]
    fn usage_items_do_not_break_pty_block_coalescing() {
        // Real PTY conversations interleave a usage item before every
        // assistant row's text; it's ignored, not a text-run boundary.
        let v = serde_json::json!({ "items": [
            { "kind": "usage", "usage": {"input_tokens": 1} },
            { "kind": "assistant_text", "text": "para one" },
            { "kind": "usage", "usage": {"input_tokens": 2} },
            { "kind": "assistant_text", "text": "para two" },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        assert_eq!(turns[0].parts.len(), 1, "blocks coalesce across usage");
        assert!(
            matches!(&turns[0].parts[0], Part::Text(t) if t == "para one\n\npara two"),
            "got {:?}",
            turns[0].parts[0]
        );
    }

    #[test]
    fn edit_and_multiedit_inputs_carry_diff_pairs() {
        let v = serde_json::json!({ "items": [
            { "kind": "tool_use", "id": "e1", "name": "Edit",
              "input": {"file_path": "/a.rs", "old_string": "foo", "new_string": "bar"} },
            { "kind": "tool_use", "id": "e2", "name": "MultiEdit",
              "input": {"file_path": "/b.rs", "edits": [
                  {"old_string": "x", "new_string": "y"},
                  {"old_string": "p", "new_string": "q"}
              ]} },
            { "kind": "tool_use", "id": "e3", "name": "Bash", "input": {"command": "ls"} },
        ]});
        let turns = turns_from_conversation(&v, "pty");
        let parts = &turns[0].parts;
        match &parts[0] {
            Part::Tool { edits, .. } => assert_eq!(edits, &[("foo".into(), "bar".into())]),
            other => panic!("expected Edit tool, got {other:?}"),
        }
        match &parts[1] {
            Part::Tool { edits, .. } => assert_eq!(
                edits,
                &[("x".to_string(), "y".to_string()), ("p".into(), "q".into())]
            ),
            other => panic!("expected MultiEdit tool, got {other:?}"),
        }
        match &parts[2] {
            Part::Tool { edits, .. } => assert!(edits.is_empty(), "Bash carries no diff"),
            other => panic!("expected Bash tool, got {other:?}"),
        }
    }

    #[test]
    fn derive_stats_empty_when_nothing_known() {
        let agent: Agent =
            serde_json::from_value(serde_json::json!({ "session_id": "s" })).unwrap();
        let d = derive_stats(&agent, None);
        assert!(d.model.is_none() && d.context_pct.is_none() && d.cost.is_none());
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

    // ── agent-mode contract characterization ────────────────────────────────
    // These tests pin the is_waiting / is_busy / state() behavior for every
    // mode that claudemon can emit, now that mode is a typed AgentMode enum.

    fn agent_with_mode(mode: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "session_id": "test",
            "mode": mode
        }))
        .unwrap()
    }

    /// "input" — user's turn to type the next message.
    #[test]
    fn mode_input_is_waiting_not_busy() {
        let a = agent_with_mode("input");
        assert!(a.is_waiting(), "input must be waiting");
        assert!(!a.is_busy(), "input must not be busy");
        assert_eq!(a.state(), "input");
    }

    /// "approval" — Claude wants to run a tool and needs a y/n.
    #[test]
    fn mode_approval_is_waiting_not_busy() {
        let a = agent_with_mode("approval");
        assert!(a.is_waiting(), "approval must be waiting");
        assert!(!a.is_busy(), "approval must not be busy");
        assert_eq!(a.state(), "approval");
    }

    /// "question" — Claude asked the user a structured question.
    #[test]
    fn mode_question_is_waiting_not_busy() {
        let a = agent_with_mode("question");
        assert!(a.is_waiting(), "question must be waiting");
        assert!(!a.is_busy(), "question must not be busy");
        assert_eq!(a.state(), "question");
    }

    /// "responding" — Claude is actively generating a turn.
    #[test]
    fn mode_responding_is_busy_not_waiting() {
        let a = agent_with_mode("responding");
        assert!(a.is_busy(), "responding must be busy");
        assert!(!a.is_waiting(), "responding must not be waiting");
        assert_eq!(a.state(), "responding");
    }

    /// "stopped" — session is finished / Claude process exited.
    #[test]
    fn mode_stopped_is_neither_waiting_nor_busy() {
        let a = agent_with_mode("stopped");
        assert!(!a.is_waiting(), "stopped must not be waiting");
        assert!(!a.is_busy(), "stopped must not be busy");
        assert_eq!(a.state(), "stopped");
    }

    /// Absent mode field — #[serde(default)] yields AgentMode::Unknown, so
    /// state() returns "unknown".
    #[test]
    fn mode_empty_state_is_unknown() {
        let a: Agent = serde_json::from_value(serde_json::json!({"session_id": "t"})).unwrap();
        assert_eq!(
            a.state(),
            "unknown",
            "absent mode yields 'unknown' from state()"
        );
        assert!(!a.is_waiting());
        assert!(!a.is_busy());
    }

    /// An explicit empty string or an unrecognised mode (e.g. a future value
    /// the daemon emits) maps to AgentMode::Other via #[serde(other)], which
    /// state() renders as "other". It is neither waiting nor busy.
    #[test]
    fn mode_unknown_string_maps_to_other() {
        // Explicit empty string falls to #[serde(other)] => AgentMode::Other.
        let a_empty = agent_with_mode("");
        assert_eq!(a_empty.state(), "other");
        assert!(!a_empty.is_waiting());
        assert!(!a_empty.is_busy());

        // An arbitrary future mode string also maps to AgentMode::Other.
        let a = agent_with_mode("future_mode");
        assert_eq!(a.state(), "other");
        assert!(!a.is_waiting());
        assert!(!a.is_busy());
    }

    /// Exhaustive table confirming the three-way classification for all known
    /// daemon-emitted modes.  Each tuple: (mode, is_waiting, is_busy).
    #[test]
    fn mode_classification_table() {
        let cases: &[(&str, bool, bool)] = &[
            ("input", true, false),
            ("approval", true, false),
            ("question", true, false),
            ("responding", false, true),
            ("stopped", false, false),
        ];
        for (mode, want_waiting, want_busy) in cases {
            let a = agent_with_mode(mode);
            assert_eq!(
                a.is_waiting(),
                *want_waiting,
                "is_waiting mismatch for mode={mode:?}"
            );
            assert_eq!(
                a.is_busy(),
                *want_busy,
                "is_busy mismatch for mode={mode:?}"
            );
        }
    }

    /// The daemon folds streamed assistant text into one retained item while
    /// `seq` counts every fragment, so a snapshot's item count is not its
    /// sequence. Deriving it from the count made every subsequent delta read as
    /// a gap — and the resync that followed restored the same wrong number.
    #[test]
    fn adopt_snapshot_takes_the_daemons_seq_not_the_item_count() {
        let mut fold = ConvFold::new(true);
        fold.adopt_snapshot(&serde_json::json!({
            "seq": 51,
            "first_seq": 1,
            "items": [
                { "kind": "user_message", "text": "go" },
                { "kind": "assistant_text", "text": "a reply built from 50 chunks" }
            ]
        }));
        assert_eq!(
            fold.seq, 51,
            "not 2, which is what the item count would give"
        );

        // The next delta continues from 51 and must apply, not read as a gap.
        let applied = fold.apply_delta(
            52,
            false,
            &[serde_json::json!({ "kind": "assistant_text", "text": "more" })],
        );
        assert!(applied, "a contiguous delta must not trigger a resync");
    }

    /// A daemon too old to send `seq` still works off the item count.
    #[test]
    fn adopt_snapshot_falls_back_to_the_item_count() {
        let mut fold = ConvFold::new(true);
        fold.adopt_snapshot(&serde_json::json!({
            "items": [{ "kind": "user_message", "text": "go" }]
        }));
        assert_eq!(fold.seq, 1);
    }
}

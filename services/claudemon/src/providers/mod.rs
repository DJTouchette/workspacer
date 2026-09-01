//! Provider adapters for non-Claude coding agents. See
//! docs/multi-agent-providers.md.
//!
//! Each adapter drives an agent's native machine interface (OpenCode's
//! `serve` HTTP+SSE, Codex's `app-server` JSON-RPC) and translates its events
//! into claudemon's existing session model — `SessionState` (mode / pending),
//! the conversation delta stream, and the status line (model / usage / cost) —
//! so the hub bus, renderer, and Fleet Deck observe every provider identically
//! to a Claude session.
//!
//! The translation is split into a *pure* per-provider layer (native event →
//! [`AgentUpdate`]s, unit-tested) and a shared *apply* layer ([`apply_updates`])
//! that drives the stores. The live process/transport clients live in each
//! provider module.

pub mod claude_stream;
pub mod codex;
pub mod codex_rollout;
pub mod codex_usage;
pub mod copilot;
pub mod copilot_usage;
pub mod opencode;
pub mod pi;

/// Workspacer MCP facade wiring for a managed supervisor: the facade MCP server
/// URL to register with the provider, and the role instructions to prepend to
/// the agent's first turn. Both `None` for a normal (non-supervisor) agent.
#[derive(Clone, Default)]
pub struct Facade {
    pub mcp_url: Option<String>,
    pub instructions: Option<String>,
}

use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use portable_pty::PtySize;
use serde_json::Value;
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::protocol::{Signal, WrapperMessage};
use crate::session::conversation::ConversationItem;
use crate::session::state::{
    Capabilities, Pending, PendingOwner, PendingWrite, Plan, PlanStatus, PlanStep, SessionMode,
    StatusLine, SubagentUpdate,
};
use crate::session::store::WrapperHandle;
use crate::session::{ConversationStore, SessionStore};
use crate::wrapper::pty;

/// One selectable model for a managed provider, as surfaced by the spawn
/// dialog's model picker. `id` is the value passed back as the model override
/// (the provider's own id format); `label` is the human display name; `default`
/// marks the provider's out-of-the-box choice. `effort_levels` carries the
/// model-specific reasoning-effort ids when the provider reports them (Codex's
/// `model/list` does); an empty list means the provider supplied no metadata.
/// `default_effort` is the level that applies when the session passes no effort
/// override — Codex reports it per model (`defaultReasoningEffort`), which is
/// what lets the composer's "Default" row name the level it resolves to instead
/// of leaving it a blank. Populated by each provider's `list_models`
/// (live-queried from the CLI/server at pick time).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effort_levels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
    /// Raw installed-catalog capacity metadata. These are capability facts,
    /// never the active session denominator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_context_window_percent: Option<u64>,
}

// ── Model-list cache ─────────────────────────────────────────────────────────
//
// Listing a managed provider's models means shelling out (a throwaway
// `codex app-server`, `opencode models`, a Pi RPC), so we don't want to do it on
// every picker-open — and those interfaces are version-fragile, so we keep the
// last-known-good list to serve if a later query fails rather than showing an
// empty picker. Keyed by "<provider>:<bin>" so different binaries don't collide.

struct ModelCacheEntry {
    at: std::time::Instant,
    models: Vec<ModelInfo>,
}
static MODEL_CACHE: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, ModelCacheEntry>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
const MODEL_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// Cached models for `key`, if present and — when `max_age` is given — younger
/// than it. `None` age means "any age" (the stale last-known-good fallback).
fn model_cache_get(key: &str, max_age: Option<std::time::Duration>) -> Option<Vec<ModelInfo>> {
    let cache = MODEL_CACHE.lock().ok()?;
    let entry = cache.get(key)?;
    match max_age {
        Some(ttl) if entry.at.elapsed() > ttl => None,
        _ => Some(entry.models.clone()),
    }
}

fn model_cache_put(key: &str, models: &[ModelInfo]) {
    if let Ok(mut cache) = MODEL_CACHE.lock() {
        cache.insert(
            key.to_string(),
            ModelCacheEntry {
                at: std::time::Instant::now(),
                models: models.to_vec(),
            },
        );
    }
}

/// Wrap a provider's live model query with the shared cache: serve a fresh cache
/// hit without running `fetch`; on a miss run it and cache the result; if it
/// fails, serve the last-known-good cached list (never inventing ids) and only
/// error when we've never listed for this key. `fetch` is the query future — for
/// an `async fn` it's a no-op until awaited, so constructing it on a cache hit is
/// free.
pub(crate) async fn cached_or_fetch(
    key: String,
    fetch: impl std::future::Future<Output = anyhow::Result<Vec<ModelInfo>>>,
) -> anyhow::Result<Vec<ModelInfo>> {
    if let Some(models) = model_cache_get(&key, Some(MODEL_CACHE_TTL)) {
        return Ok(models);
    }
    match fetch.await {
        Ok(models) => {
            model_cache_put(&key, &models);
            Ok(models)
        }
        Err(err) => match model_cache_get(&key, None) {
            Some(models) => {
                tracing::warn!(
                    ?err,
                    key,
                    "model list failed; serving last-known-good cached models"
                );
                Ok(models)
            }
            None => Err(err),
        },
    }
}

/// A typed update distilled from one native provider event, in the common
/// vocabulary every adapter maps onto. Several can come from a single event
/// (e.g. a streamed text chunk is both "the agent is busy" and "here's text").
#[derive(Debug, Clone, PartialEq)]
pub enum AgentUpdate {
    /// The agent finished responding — session is ready for input.
    Idle,
    /// The agent is actively producing output.
    Busy,
    /// A permission/approval request is outstanding (waiting on the user). `id`
    /// is the provider's permission/request identifier, needed to forward the
    /// decision back (OpenCode permission reply); None when the transport
    /// already carries the id out of band (Codex JSON-RPC request id).
    PermissionPending {
        id: Option<String>,
        tool: Option<String>,
        summary: Option<String>,
        /// The provider's raw request payload, so the GUI can render the full
        /// approval detail (argv, diff, dialog fields) the way it does for
        /// Claude hook approvals. `Value::Null` when the adapter has no richer
        /// payload than the tool/summary it already carries.
        raw: Value,
    },
    /// A chunk of assistant text to append to the conversation.
    AssistantText(String),
    /// A user message echoed back by the server.
    UserText(String),
    /// A tool invocation.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// The result of a tool invocation, joined to its `ToolUse` by id so the GUI
    /// can render the call and its output as one card.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// Token/cost telemetry for the session.
    Usage {
        model: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        /// Cache-read subset of `input_tokens` (cumulative, like it). Only
        /// matters to providers without a native cost figure — cache reads
        /// bill at a much lower rate, so the estimate needs the split.
        cached_input_tokens: Option<u64>,
        cost_usd: Option<f64>,
        /// Tokens currently occupying the model's context window (the latest
        /// turn's total, including cache) — NOT cumulative like input/output.
        context_tokens: Option<u64>,
        /// The model's context window size, when the provider reports it.
        /// Absent → [`context_window_for`] falls back to a table by model id.
        context_window: Option<u64>,
    },
    /// The account's rolling rate-limit windows, when the provider reports them
    /// (Codex does; Claude's stream `rate_limit_event` does) — same meaning as
    /// the fields Claude's statusLine carries. `monthly` is the overage/credit
    /// window (Claude's `overage` `rateLimitType`); providers without one leave
    /// it `None`.
    RateLimits {
        five_hour_pct: Option<f64>,
        five_hour_resets_at: Option<i64>,
        /// The window's length in minutes, when it is known: Codex reports it
        /// per window, Claude's is implied by the window's name. `None` means
        /// "not reported", and clients render the window without a duration.
        five_hour_window_minutes: Option<u64>,
        seven_day_pct: Option<f64>,
        seven_day_resets_at: Option<i64>,
        seven_day_window_minutes: Option<u64>,
        monthly_pct: Option<f64>,
        monthly_resets_at: Option<i64>,
        monthly_window_minutes: Option<u64>,
    },
    /// Account rate-limit *status* (distinct from the utilization windows above,
    /// which Claude only sends near a limit). `warning` is a human message when a
    /// window crosses its warning threshold (`status: allowed_warning`), else
    /// `None` (which clears a prior warning). `out_of_credits` reflects the
    /// monthly overage being disabled for lack of credits. Latest-wins.
    RateLimitStatus {
        warning: Option<String>,
        out_of_credits: Option<bool>,
    },
    /// Session capabilities parsed from the stream `init` frame (fast mode,
    /// output style, MCP/skill/plugin/agent/memory counts). Set once per session.
    Capabilities(Capabilities),
    /// The reasoning-effort level now in force, as *confirmed by the provider*
    /// (Codex's `thread/settings/updated`). Latest-wins, and the only trustworthy
    /// effort signal there is: it also catches a change made in the provider's
    /// own TUI, which no request of ours would tell us about. Claude reports no
    /// equivalent — see the composer's optimistic path.
    Effort(String),
    /// A session-level error message.
    Error(String),
    /// The agent's current plan / checklist (Codex's `update_plan` / todo list).
    /// Last-write-wins full replacement — carried into the conversation store as
    /// a `plan` item by `apply_updates` (via `SessionStore::set_plan`), never as
    /// a `conversation_item`.
    Plan(Plan),
    /// Managed-provider subagent row update.
    Subagent(SubagentUpdate),
}

/// Parse a Codex `RateLimitSnapshot` — camelCase on the app-server wire
/// (`usedPercent`/`windowDurationMins`/`resetsAt`), snake_case in rollout
/// `event_msg`s (`used_percent`/`window_minutes`/`resets_at`) — into a
/// [`AgentUpdate::RateLimits`]. Each window is bucketed by its duration (≤12h →
/// the 5h slot, longer → the 7d slot), falling back to primary→5h /
/// secondary→7d when the duration is absent.
/// Open ANOTHER tool's SQLite database for reading — Codex's `state_5.sqlite`,
/// Copilot's `session-store.db`. Never ours; `store::Db` owns that one.
///
/// Read-only and WAL-aware FIRST, `immutable=1` only as a fallback, and the
/// order is load-bearing rather than a preference. `immutable=1` tells SQLite
/// to ignore the write-ahead log entirely, which is attractive (no locking, no
/// contention with the running CLI) and quietly catastrophic: measured
/// 2026-08-28, an immutable read of Copilot's `session-store.db` returned
/// **zero** `assistant_usage_events` rows while a WAL-aware read of the same
/// file returned all 56 — its 167 KB main file had last checkpointed two days
/// earlier and every event lived in a 3.3 MB uncheckpointed WAL. A UI shown
/// that zero would have reported "you have used nothing", which is the exact
/// class of lie this whole data layer exists to stop.
///
/// So: take the WAL. Concurrent readers are what WAL mode is for and they do
/// not block the writing CLI. The immutable fallback stays for the case that
/// genuinely needs it — a database on a read-only mount, where the `-shm` file
/// cannot be created — and a caller that gets it should treat missing rows as
/// unknown rather than as zero.
pub(crate) fn open_foreign_sqlite_readonly(path: &std::path::Path) -> Option<rusqlite::Connection> {
    if !path.is_file() {
        return None;
    }
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let display = path.display();
    rusqlite::Connection::open_with_flags(format!("file:{display}?mode=ro"), flags)
        .or_else(|_| {
            rusqlite::Connection::open_with_flags(
                format!("file:{display}?mode=ro&immutable=1"),
                flags,
            )
        })
        .ok()
}

pub(crate) fn rate_limits_from(v: &Value) -> Option<AgentUpdate> {
    fn window(w: &Value) -> (Option<f64>, Option<i64>, Option<u64>) {
        let pick = |keys: [&str; 2]| keys.iter().find_map(|k| w.get(*k));
        (
            pick(["usedPercent", "used_percent"]).and_then(Value::as_f64),
            pick(["resetsAt", "resets_at"]).and_then(Value::as_i64),
            pick(["windowDurationMins", "window_minutes"]).and_then(Value::as_u64),
        )
    }
    type Slot = (Option<f64>, Option<i64>, Option<u64>);
    let mut five: Slot = (None, None, None);
    let mut seven: Slot = (None, None, None);
    for key in ["primary", "secondary"] {
        let Some(w) = v.get(key).filter(|w| !w.is_null()) else {
            continue;
        };
        let (pct, resets, mins) = window(w);
        if pct.is_none() && resets.is_none() {
            continue;
        }
        let is_seven_day = mins.map_or(key == "secondary", |m| m > 720);
        if is_seven_day {
            seven = (pct, resets, mins);
        } else {
            five = (pct, resets, mins);
        }
    }
    if five == (None, None, None) && seven == (None, None, None) {
        return None;
    }
    Some(AgentUpdate::RateLimits {
        five_hour_pct: five.0,
        five_hour_resets_at: five.1,
        // Codex's own `windowDurationMins`: a "5h slot" window can be any
        // length up to 12h, so pass the reported figure rather than assuming.
        five_hour_window_minutes: five.2,
        seven_day_pct: seven.0,
        seven_day_resets_at: seven.1,
        seven_day_window_minutes: seven.2,
        // Codex reports only primary/secondary windows; no monthly overage.
        monthly_pct: None,
        monthly_resets_at: None,
        monthly_window_minutes: None,
    })
}

/// Parse a plan / todo-list payload from a provider event into a [`Plan`].
///
/// Codex surfaces the agent's plan in a few near-identical shapes depending on
/// the channel: the `update_plan` tool's arguments (`{ plan: [{ step, status
/// }] }`), an app-server `todoList`/`plan` item, or a rollout equivalent. This
/// reads whichever list key is present (`plan` / `steps` / `items` / `todos`)
/// and maps each entry defensively: step text from `content` / `step` / `text`
/// / `title`; status from an explicit `status` string (via
/// [`PlanStatus::from_wire`]) or a boolean `completed` flag. Returns `None` when
/// no recognizable step list is present, so it's safe to probe any item.
pub(crate) fn plan_from_value(v: &Value) -> Option<Plan> {
    let steps: Vec<PlanStep> = ["plan", "steps", "items", "todos"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_array))?
        .iter()
        .filter_map(plan_step_from_value)
        .collect();
    if steps.is_empty() {
        return None;
    }
    Some(Plan {
        steps,
        updated_at: None,
    })
}

fn plan_step_from_value(v: &Value) -> Option<PlanStep> {
    let content = ["content", "step", "text", "title"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))?
        .to_string();
    let status = v
        .get("status")
        .and_then(Value::as_str)
        .map(PlanStatus::from_wire)
        .or_else(|| {
            v.get("completed").and_then(Value::as_bool).map(|done| {
                if done {
                    PlanStatus::Completed
                } else {
                    PlanStatus::Pending
                }
            })
        })
        .unwrap_or(PlanStatus::Pending);
    let active_form = ["activeForm", "active_form"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_owned);
    Some(PlanStep {
        content,
        status,
        active_form,
    })
}

/// Context window size (tokens) for well-known model families, used when the
/// provider's own events don't report one. `None` for a model no row covers,
/// and the context meter simply doesn't render — a missing meter beats a wrong
/// one.
///
/// The KNOWLEDGE moved out from under this function: the hand-rolled
/// `if m.contains(...)` chain that used to live here was one of five parallel
/// window tables in this repo, and it disagreed with two of the others about
/// `gpt-5-codex`. It is now a call into [`crate::session::windows`], which is
/// pinned to contracts/model-context-windows.json alongside its TypeScript and
/// Go twins. The function survives because the call sites do.
pub fn context_window_for(model: &str) -> Option<u64> {
    crate::session::windows::window_for(model)
}

/// Window implied by a model string the session was *asked* for — the alias the
/// user picked (`opus[1m]`), not a concrete provider id. Claude Code strips the
/// `[1m]` marker from the `model` it writes into the transcript, so the request
/// is the only carrier of a 1M choice until the provider reports a window.
///
/// Deliberately narrower than [`context_window_for`]: it answers only "was 1M
/// asked for", and `None` means "says nothing", NOT "200k".
pub fn requested_context_window_for(model: &str) -> Option<u64> {
    crate::session::windows::requested_window_for(model)
}

/// Map an `AgentUpdate` to a conversation item, when it represents one.
pub fn conversation_item(update: &AgentUpdate) -> Option<ConversationItem> {
    // Managed adapters have no transcript rows to inherit timestamps from, so
    // stamp arrival time. Clients derive tool durations from the gap between a
    // tool_use and its tool_result — without stamps everything reads 0s.
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .ok();
    match update {
        AgentUpdate::AssistantText(text) => Some(ConversationItem::AssistantText {
            text: text.clone(),
            timestamp: now,
        }),
        AgentUpdate::UserText(text) => Some(ConversationItem::UserMessage {
            text: text.clone(),
            timestamp: now,
        }),
        AgentUpdate::ToolUse { id, name, input } => Some(ConversationItem::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
            timestamp: now,
        }),
        AgentUpdate::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(ConversationItem::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
            timestamp: now,
        }),
        _ => None,
    }
}

/// One rolling account window as a provider reported it. Every field is
/// optional on purpose: providers report different subsets, and "not reported"
/// has to stay distinguishable from zero all the way to the client.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct WindowReading {
    pub pct: Option<f64>,
    pub resets_at: Option<i64>,
    pub minutes: Option<u64>,
}

/// Running tally of model/usage/cost across a managed session, for the status
/// line. How input/output/cached_input/cost fold depends on what the provider's
/// wire figures mean, selected per-adapter via [`UsageAcc::additive`]:
///   - Default (max): the figures are session-CUMULATIVE running totals
///     (Claude, Codex), so the max is the latest total and the displayed
///     numbers never regress mid-turn.
///   - Additive (sum): the figures are PER-MESSAGE / per-step (OpenCode, Pi),
///     so summing them across the session gives the true totals — a max would
///     keep only the single biggest message and under-report.
///
/// Context occupancy (`context_tokens` / `context_window`) is the LATEST reading
/// in BOTH modes — never summed — because compaction legitimately shrinks it and
/// it feeds the context meter's occupancy, not a running total.
#[derive(Default)]
pub struct UsageAcc {
    model: Option<String>,
    input: Option<u64>,
    output: Option<u64>,
    cached_input: Option<u64>,
    cost: Option<f64>,
    context_tokens: Option<u64>,
    context_window: Option<u64>,
    five_hour_pct: Option<f64>,
    five_hour_resets_at: Option<i64>,
    five_hour_window_minutes: Option<u64>,
    seven_day_pct: Option<f64>,
    seven_day_resets_at: Option<i64>,
    seven_day_window_minutes: Option<u64>,
    monthly_pct: Option<f64>,
    monthly_resets_at: Option<i64>,
    monthly_window_minutes: Option<u64>,
    rate_limit_warning: Option<String>,
    overage_out_of_credits: Option<bool>,
    capabilities: Option<Capabilities>,
    /// Provider-confirmed reasoning effort (see `AgentUpdate::Effort`).
    effort: Option<String>,
    /// Estimate cost from the pricing table when no native cost arrives.
    /// Opt-in per adapter: only sound where the wire's input/output totals are
    /// session-cumulative AND never carry dollars (Codex). Claude's stream
    /// totals are per-request maxes — an estimate there would flash a wrong
    /// figure before the first `result` event's real `total_cost_usd`.
    estimate_costs: bool,
    /// Sum input/output/cached_input/cost across `merge` calls instead of taking
    /// the max. Opt-in per adapter: only for providers whose wire figures are
    /// per-message / per-step (OpenCode, Pi) rather than session-cumulative.
    /// Never affects context occupancy, which stays latest-wins in both modes.
    additive: bool,
}

impl UsageAcc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-fill the model from spawn settings so the status line names it even
    /// before (or without) the provider's own usage events carrying one. A
    /// later event that does carry a model still overrides this.
    pub fn seed_model(&mut self, model: Option<&str>) {
        if self.model.is_none() {
            self.model = model.map(str::to_owned);
        }
    }

    /// Turn on pricing-table cost estimation (see the field's caveats).
    pub fn estimate_costs(&mut self) {
        self.estimate_costs = true;
    }

    /// Fold token/cost figures additively instead of by max (see the field's
    /// caveats). Use only for adapters whose wire figures are per-message.
    pub fn additive(&mut self) {
        self.additive = true;
    }
    #[allow(clippy::too_many_arguments)]
    fn merge(
        &mut self,
        model: Option<String>,
        input: Option<u64>,
        output: Option<u64>,
        cached_input: Option<u64>,
        cost: Option<f64>,
        context_tokens: Option<u64>,
        context_window: Option<u64>,
    ) {
        if model.is_some() {
            self.model = model;
        }
        // Per-message adapters (additive) SUM these across the session; cumulative
        // adapters MAX so the latest running total wins and never regresses.
        if let Some(i) = input {
            self.input = Some(match self.input {
                Some(c) if self.additive => c + i,
                Some(c) => c.max(i),
                None => i,
            });
        }
        if let Some(o) = output {
            self.output = Some(match self.output {
                Some(c) if self.additive => c + o,
                Some(c) => c.max(o),
                None => o,
            });
        }
        if let Some(ci) = cached_input {
            self.cached_input = Some(match self.cached_input {
                Some(c) if self.additive => c + ci,
                Some(c) => c.max(ci),
                None => ci,
            });
        }
        if let Some(c) = cost {
            self.cost = Some(match self.cost {
                Some(p) if self.additive => p + c,
                Some(p) => p.max(c),
                None => c,
            });
        }
        // context_tokens / context_window stay latest-wins in BOTH modes — never
        // summed (see the type doc): they feed occupancy, and compaction shrinks it.
        if context_tokens.is_some() {
            self.context_tokens = context_tokens; // latest, not max — compaction shrinks it
        }
        if context_window.is_some() {
            self.context_window = context_window;
        }
    }

    /// Fold in a rate-limit reading — latest wins per field (they only move
    /// forward between readings; a lower % just means the window rolled).
    /// Field-wise, because the wire doesn't always pair them: Claude's
    /// `rate_limit_event` can carry `resetsAt` without `utilization`, and that
    /// reading must still land instead of being dropped.
    /// A window's length folds the same way.
    fn merge_rate_limits(
        &mut self,
        five: WindowReading,
        seven: WindowReading,
        monthly: WindowReading,
    ) {
        fn take<T: Copy>(dst: &mut Option<T>, src: Option<T>) {
            if src.is_some() {
                *dst = src;
            }
        }
        take(&mut self.five_hour_pct, five.pct);
        take(&mut self.five_hour_resets_at, five.resets_at);
        take(&mut self.five_hour_window_minutes, five.minutes);
        take(&mut self.seven_day_pct, seven.pct);
        take(&mut self.seven_day_resets_at, seven.resets_at);
        take(&mut self.seven_day_window_minutes, seven.minutes);
        take(&mut self.monthly_pct, monthly.pct);
        take(&mut self.monthly_resets_at, monthly.resets_at);
        take(&mut self.monthly_window_minutes, monthly.minutes);
    }

    /// Fold in a rate-limit *status* reading. Both fields are latest-wins,
    /// including clearing a warning back to `None` when a later event reports the
    /// window is comfortable again.
    fn merge_rate_limit_status(&mut self, warning: Option<String>, out_of_credits: Option<bool>) {
        self.rate_limit_warning = warning;
        if out_of_credits.is_some() {
            self.overage_out_of_credits = out_of_credits;
        }
    }

    /// Build the `StatusLine` for `SessionStore::apply_status_line` — the same
    /// shape Claude's own statusLine feeds, so the renderer's bottom bar (model
    /// · context meter · tokens · cost) renders identically for every provider.
    /// The context %, previously Claude-only, is computed from the latest
    /// context occupancy over the provider-reported window (falling back to
    /// [`context_window_for`] by model id).
    pub fn status_line(&self) -> StatusLine {
        let window = self
            .context_window
            .or_else(|| self.model.as_deref().and_then(context_window_for));
        let pct = match (self.context_tokens, window) {
            (Some(ctx), Some(win)) if win > 0 => {
                Some(((ctx as f64 / win as f64) * 100.0).min(100.0))
            }
            _ => None,
        };
        // No native cost (Codex — OpenAI's wire carries no dollars): estimate
        // from the cumulative token totals and the pricing table. Unknown
        // models stay blank rather than invent a figure.
        let cost = self.cost.or_else(|| {
            self.estimate_costs
                .then(|| {
                    crate::session::pricing::estimate_cost(
                        self.model.as_deref(),
                        self.input,
                        self.cached_input,
                        self.output,
                    )
                })
                .flatten()
        });
        // Providers that never send an explicit warning (Codex) still get one
        // synthesized from the utilization windows they do report.
        let warning = self.rate_limit_warning.clone().or_else(|| {
            synthesized_rate_limit_warning(&[
                ("5-hour", self.five_hour_pct),
                ("7-day", self.seven_day_pct),
                ("monthly", self.monthly_pct),
            ])
        });
        StatusLine {
            model_display: self.model.clone(),
            context_used_pct: pct,
            context_window_size: window,
            total_input_tokens: self.input,
            total_output_tokens: self.output,
            cached_input_tokens: self.cached_input,
            cost_usd: cost,
            five_hour_pct: self.five_hour_pct,
            five_hour_resets_at: self.five_hour_resets_at,
            five_hour_window_minutes: self.five_hour_window_minutes,
            seven_day_pct: self.seven_day_pct,
            seven_day_resets_at: self.seven_day_resets_at,
            seven_day_window_minutes: self.seven_day_window_minutes,
            monthly_pct: self.monthly_pct,
            monthly_resets_at: self.monthly_resets_at,
            monthly_window_minutes: self.monthly_window_minutes,
            rate_limit_warning: warning,
            overage_out_of_credits: self.overage_out_of_credits,
            capabilities: self.capabilities.clone(),
            effort: self.effort.clone(),
            received_at: Some(OffsetDateTime::now_utc()),
        }
    }
}

/// "You're close to your … usage limit — NN% used" once a window crosses 80%,
/// mirroring the phrasing Claude's own `rate_limit_event` warning uses so the
/// banner reads identically for every provider. Highest window wins. Clears
/// (returns `None`) once every window is back under the threshold.
fn synthesized_rate_limit_warning(windows: &[(&str, Option<f64>)]) -> Option<String> {
    const THRESHOLD: f64 = 80.0;
    windows
        .iter()
        .filter_map(|(label, pct)| pct.map(|p| (*label, p)))
        .filter(|(_, p)| *p >= THRESHOLD)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(label, p)| format!("You're close to your {label} usage limit — {p:.0}% used"))
}

/// Write a managed session's mode through the store's guarded funnel and sync
/// the driver's own `cur_mode` mirror to what the store actually holds.
///
/// Every adapter keeps a local `cur_mode` because it drives decisions the store
/// can't (which frame re-surfaces a queued approval, whether to bother writing
/// at all). That mirror is only safe if it follows the store rather than the
/// request: a [`PendingWrite::Keep`] write is suppressed while an approval or
/// question is parked, and a driver that then believed its own `Responding`
/// would go on to make every later decision on a mode the store never adopted.
///
/// An unregistered session (teardown race) has no state to read back, so the
/// mirror takes the requested mode — the same thing the pre-funnel code did.
pub(crate) fn set_mode(
    store: &SessionStore,
    session_id: &str,
    mode: SessionMode,
    write: PendingWrite,
    cur_mode: &mut SessionMode,
) {
    *cur_mode = match store.set_managed_mode(session_id, mode, write) {
        Some(state) => state.mode,
        None => mode,
    };
}

/// Mode bookkeeping for a user message that was just written to an agent's
/// stdin. Shared by every managed driver's send arm (claude stream, codex,
/// opencode, pi) — each had its own copy, and three of the four had the copy
/// aae765a3 had to fix.
///
/// A parked approval/question is a PAUSE: the CLI is blocked on the user, not
/// working. Reasserting `Responding` here must never demote it —
/// `set_managed_mode` would also drop the `pending` card (a `Responding` write
/// carries none), leaving a session that reports `responding` (clients:
/// "streaming") while the CLI is still blocked on an unanswered
/// `can_use_tool`, with no approval record left to answer. The parked request
/// only lives inside this driver, so nothing else can repair it: the session
/// wedges silently and its transcript stops growing.
///
/// This is the same invariant as `apply_updates`' `Busy` arm and the
/// `background_tasks_changed` paths in `handle_line`: a parked request may only
/// be written by the feed that raised it, and enrichment may not touch it at
/// all. It is no longer re-derived here — the write goes in as
/// [`PendingWrite::Keep`], which carries no owner precisely because it may
/// never touch the slot whoever sends it, and the store is what refuses it.
/// Only the state write is suppressed: the message itself already went down
/// stdin and still queues in the CLI.
///
/// Note "one feed" is not the same as "one request": on codex, opencode and pi
/// the `AskUserQuestion` MCP shim (`daemon::mcp_ask`) can have a question
/// blocking the agent at the same time as this driver has an approval, and the
/// store keeps both (see [`PendingOwner`]). A driver must therefore take its
/// `cur_mode` from what `set_managed_mode` returns, never from what it asked
/// for — releasing its own card can legitimately leave the session parked on
/// the other feed's.
pub(crate) fn note_user_send(store: &SessionStore, session_id: &str, cur_mode: &mut SessionMode) {
    if *cur_mode != SessionMode::Responding {
        set_mode(
            store,
            session_id,
            SessionMode::Responding,
            PendingWrite::Keep,
            cur_mode,
        );
    }
}

/// Drive the stores from a batch of translated updates. Shared by every
/// adapter. Mode changes are debounced (Approval always re-applies since its
/// `pending` can change); conversation items are pushed together; usage
/// refreshes the status line.
pub fn apply_updates(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    updates: Vec<AgentUpdate>,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
) {
    let mut items = Vec::new();
    // The mode this batch resolves to, paired with what it means to do to the
    // session's single pending slot. Carrying the intent (rather than a bare
    // `Option<Pending>`) is what lets the store refuse a liveness write that
    // would clobber a parked request — see [`PendingWrite`].
    let mut new_mode: Option<(SessionMode, PendingWrite)> = None;
    let mut usage_changed = false;
    // Latest plan in this batch (last-write-wins); applied after the item push
    // so its own conversation item lands just past the batch's items.
    let mut plan: Option<Plan> = None;
    let mut subagents = Vec::new();

    for update in &updates {
        match update {
            // A genuine turn end (a `result` frame) really is the CLI saying
            // the turn is over, so it is allowed to clear a parked request —
            // unlike `Busy`, which is only a liveness ping.
            AgentUpdate::Idle => {
                new_mode = Some((
                    SessionMode::Input,
                    PendingWrite::Resolve(PendingOwner::Primary),
                ))
            }
            AgentUpdate::Busy => {
                // A parked approval/question is a PAUSE: the agent is blocked
                // on the user, not working. `Busy` is a liveness ping and says
                // nothing about that, so it goes in as `Keep` and the store
                // drops it while a request is parked. Left unguarded (as it
                // was) it demoted the pause to `Responding` AND dropped the
                // card, leaving a session that reports "streaming" while the
                // CLI is still blocked on an unanswered `can_use_tool` with no
                // approval record to answer — unrepairable from outside,
                // because the daemon is the only holder of the parked request.
                //
                // The `new_mode.is_none()` check is a different thing and
                // stays: within one batch, an explicit `Idle`/`Approval`
                // outranks a liveness ping.
                if new_mode.is_none() {
                    new_mode = Some((SessionMode::Responding, PendingWrite::Keep));
                }
            }
            AgentUpdate::PermissionPending {
                tool, summary, raw, ..
            } => {
                // NOTE: surfacing the pending approval is accurate telemetry, but
                // forwarding the user's decision back to the provider's approval
                // API is a follow-up (Phase 4).
                new_mode = Some((
                    SessionMode::Approval,
                    PendingWrite::Park(
                        PendingOwner::Primary,
                        Pending::Approval {
                            tool: tool.clone(),
                            summary: summary.clone(),
                            raw: raw.clone(),
                        },
                    ),
                ));
            }
            AgentUpdate::Usage {
                model,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cost_usd,
                context_tokens,
                context_window,
            } => {
                acc.merge(
                    model.clone(),
                    *input_tokens,
                    *output_tokens,
                    *cached_input_tokens,
                    *cost_usd,
                    *context_tokens,
                    *context_window,
                );
                usage_changed = true;
            }
            AgentUpdate::RateLimits {
                five_hour_pct,
                five_hour_resets_at,
                five_hour_window_minutes,
                seven_day_pct,
                seven_day_resets_at,
                seven_day_window_minutes,
                monthly_pct,
                monthly_resets_at,
                monthly_window_minutes,
            } => {
                acc.merge_rate_limits(
                    WindowReading {
                        pct: *five_hour_pct,
                        resets_at: *five_hour_resets_at,
                        minutes: *five_hour_window_minutes,
                    },
                    WindowReading {
                        pct: *seven_day_pct,
                        resets_at: *seven_day_resets_at,
                        minutes: *seven_day_window_minutes,
                    },
                    WindowReading {
                        pct: *monthly_pct,
                        resets_at: *monthly_resets_at,
                        minutes: *monthly_window_minutes,
                    },
                );
                usage_changed = true;
            }
            AgentUpdate::RateLimitStatus {
                warning,
                out_of_credits,
            } => {
                acc.merge_rate_limit_status(warning.clone(), *out_of_credits);
                usage_changed = true;
            }
            AgentUpdate::Capabilities(caps) => {
                acc.capabilities = Some(caps.clone());
                usage_changed = true;
            }
            AgentUpdate::Effort(level) => {
                acc.effort = Some(level.clone());
                usage_changed = true;
            }
            AgentUpdate::Error(msg) => {
                // A managed provider (Codex/OpenCode/Pi) reported an agent-side
                // failure. Log it and surface it in the conversation so the GUI
                // shows it instead of silently swallowing it. The renderer only
                // renders known item kinds, so ride it in as assistant text with
                // a clear marker rather than a bespoke variant it would drop.
                tracing::warn!(session = %session_id, error = %msg, "managed session error");
                // The trailing newline is load-bearing, not cosmetic: the
                // conversation store COALESCES consecutive assistant_text
                // items, so an error followed by the turn's real reply ran
                // straight into it — "…no structured questions).OK" was the
                // live shape on a copilot session whose facade failed to
                // attach. `errorMarkerReason` (the wake path) reads only the
                // first line, so it is unaffected either way.
                items.push(ConversationItem::AssistantText {
                    text: format!("⚠️ Error: {msg}\n"),
                    timestamp: None,
                });
            }
            AgentUpdate::Plan(p) => plan = Some(p.clone()),
            AgentUpdate::Subagent(update) => subagents.push(update.clone()),
            AgentUpdate::AssistantText(_)
            | AgentUpdate::UserText(_)
            | AgentUpdate::ToolUse { .. }
            | AgentUpdate::ToolResult { .. } => {
                if let Some(item) = conversation_item(update) {
                    items.push(item);
                }
            }
        }
    }

    if !items.is_empty() {
        conv.push(session_id, items);
    }
    if let Some(plan) = plan {
        store.set_plan(conv, session_id, plan);
    }
    for subagent in subagents {
        store.apply_subagent_update(session_id, subagent);
    }
    if let Some((mode, write)) = new_mode {
        // Approval always re-applies: its `pending` payload can change even
        // when the mode doesn't (a second parked request replacing the card).
        if mode != *cur_mode || mode == SessionMode::Approval {
            set_mode(store, session_id, mode, write, cur_mode);
        }
    }
    if usage_changed {
        store.apply_status_line(session_id, acc.status_line());
    }
}
/// The profile half of a managed spawn: the harness's config-root environment
/// and the extra argv that go with it.
///
/// One type for every harness because it IS one primitive wearing three names.
/// `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `COPILOT_HOME` all say "read your
/// config from here", and `codex -p <preset>` rides the same argv channel as
/// Claude's `--settings`. The desktop resolves which name applies (see
/// `PROFILE_CAPS` in apps/desktop/src/main/shared/agentProfiles.ts) and sends
/// the resolved pair; the daemon does not need to know which harness it is
/// looking at in order to pass it on.
///
/// It exists because passing it on is exactly what the daemon was NOT doing.
/// `/sessions/spawn-managed` forwarded `env`/`extra_args` into the claude_stream
/// arm only, so a Codex or Copilot profile changed the Settings form, the spawn
/// picker and the payload on the wire — and nothing about the process that
/// actually ran. Threading one value through every arm is what makes a dropped
/// field a compile error instead of a silent no-op.
#[derive(Debug, Clone, Default)]
pub struct SpawnExtras {
    /// Merged on top of the daemon's own environment, never replacing it.
    pub env: HashMap<String, String>,
    /// Appended verbatim after the argv the daemon builds.
    pub extra_args: Vec<String>,
}

/// Spawn a PTY child and wire it into an already-registered session's byte
/// stream + input channel — the **Term half** of a hybrid managed agent. Output
/// is pumped through `record_output` (onto the session's byte broadcast); input
/// arrives via the `WrapperHandle` registered with `attach_pty`. Returns the
/// handle so the caller can kill the child when the session ends.
///
/// Shared by the hybrid adapters (OpenCode `attach`, Codex `resume --remote`):
/// each drives a structured GUI from its own machine interface *and* runs the
/// agent's native TUI in a PTY attached to the same live session, so the GUI and
/// Term are two views of one conversation.
pub(crate) fn spawn_attach_pty(
    store: &SessionStore,
    session_id: &str,
    argv: &[String],
    cwd: &str,
    env: &HashMap<String, String>,
) -> anyhow::Result<Arc<pty::PtyHandle>> {
    let handle = Arc::new(pty::spawn(
        argv,
        cwd,
        PtySize {
            cols: 120,
            rows: 32,
            pixel_width: 0,
            pixel_height: 0,
        },
        env,
    )?);

    // input pump: WrapperMessage (from POST /sessions/:id/input) -> PTY
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<WrapperMessage>();
    let pty_in = handle.clone();
    tokio::spawn(async move {
        while let Some(msg) = input_rx.recv().await {
            match msg {
                WrapperMessage::Input { bytes } => {
                    if let Ok(decoded) = B64.decode(bytes.as_bytes()) {
                        let _ = pty::write_bytes(&pty_in, &decoded).await;
                    }
                }
                WrapperMessage::Signal { signal } => match signal {
                    Signal::Sigint => {
                        let _ = pty::write_bytes(&pty_in, b"\x03").await;
                    }
                    other => {
                        let _ = pty::signal_child(&pty_in, other);
                    }
                },
                WrapperMessage::Resize { cols, rows } => {
                    let _ = pty::resize(&pty_in, cols, rows).await;
                }
                _ => {}
            }
        }
    });

    // output pump: PTY -> record_output -> byte broadcast (the Term view)
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    pty::start_reader(&handle, out_tx)?;
    let store_out = store.clone();
    let sid = session_id.to_string();
    let handle_for_reader = handle.clone();
    tokio::spawn(async move {
        while let Some(chunk) = out_rx.recv().await {
            store_out.record_output(&sid, &chunk).await;
        }
        // TUI exited (reader EOF) — reap it so it isn't left a zombie, but only
        // clear the registry slot if it still holds *this* TUI: a restart on the
        // same session id may already have attached its own.
        store_out.reap_pty_owned(&sid, &handle_for_reader);
    });

    // Register the TUI child so daemon shutdown kills it too (it's a portable-pty
    // child with no kill-on-drop, like the in-daemon PTY path).
    store.register_pty(session_id, handle.clone());
    store.attach_pty(session_id, WrapperHandle { tx: input_tx });
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window's length is data, not a convention: a Codex primary window is
    /// bucketed into the 5h slot at anything up to 12h, so the slot's name says
    /// nothing about how long the window actually is. The reported figure has to
    /// ride through, or a 90-minute window gets labelled "5 hours" downstream.
    #[test]
    fn codex_windows_carry_their_reported_length() {
        let snap = serde_json::json!({
            "primary": { "usedPercent": 40.0, "windowDurationMins": 90, "resetsAt": 1783121345 },
            "secondary": { "usedPercent": 3.0, "windowDurationMins": 10080, "resetsAt": 1783708145 }
        });
        let AgentUpdate::RateLimits {
            five_hour_window_minutes,
            seven_day_window_minutes,
            monthly_window_minutes,
            monthly_pct,
            ..
        } = rate_limits_from(&snap).expect("windows parse")
        else {
            panic!("expected RateLimits");
        };
        assert_eq!(five_hour_window_minutes, Some(90));
        assert_eq!(seven_day_window_minutes, Some(10_080));
        // Codex has no monthly window at all. Nothing may be invented for it:
        // an absent window must stay absent rather than become a 0% meter.
        assert_eq!(monthly_pct, None);
        assert_eq!(monthly_window_minutes, None);
    }

    /// A window the provider reports without a duration keeps `None`, so the
    /// client falls back to the slot's own name instead of a guessed length.
    #[test]
    fn codex_window_without_a_duration_reports_none() {
        let snap = serde_json::json!({
            "primary": { "usedPercent": 12.0, "resetsAt": 1783121345 }
        });
        let AgentUpdate::RateLimits {
            five_hour_pct,
            five_hour_window_minutes,
            ..
        } = rate_limits_from(&snap).expect("windows parse")
        else {
            panic!("expected RateLimits");
        };
        assert_eq!(five_hour_pct, Some(12.0));
        assert_eq!(five_hour_window_minutes, None);
    }

    #[test]
    fn model_cache_ttl_and_stale_fallback() {
        let key = "test-provider:test-bin-abc123"; // unique: MODEL_CACHE is global
        let models = vec![ModelInfo {
            id: "m1".into(),
            label: "M1".into(),
            default: true,
            effort_levels: vec![],
            default_effort: None,
            default_context_window: None,
            max_context_window: None,
            effective_context_window_percent: None,
        }];

        // Fresh entry is served both as a fresh hit and as last-known-good.
        model_cache_put(key, &models);
        assert!(
            model_cache_get(key, Some(MODEL_CACHE_TTL)).is_some(),
            "fresh hit"
        );
        assert!(model_cache_get(key, None).is_some(), "any-age hit");

        // An entry older than the TTL is NOT a fresh hit, but IS still available
        // as the stale fallback (what we serve when a live query fails).
        if let Some(old) = std::time::Instant::now().checked_sub(MODEL_CACHE_TTL * 2) {
            MODEL_CACHE
                .lock()
                .unwrap()
                .insert(key.into(), ModelCacheEntry { at: old, models });
            assert!(
                model_cache_get(key, Some(MODEL_CACHE_TTL)).is_none(),
                "stale is not a fresh hit"
            );
            assert!(
                model_cache_get(key, None).is_some(),
                "stale still served as last-known-good"
            );
        }

        // Unknown key → nothing cached.
        assert!(model_cache_get("test-provider:never-seen", None).is_none());
    }

    #[test]
    fn status_line_sets_only_known_fields() {
        let mut acc = UsageAcc::new();
        acc.merge(
            Some("m".into()),
            Some(10),
            Some(2),
            None,
            Some(0.5),
            None,
            None,
        );
        let sl = acc.status_line();
        assert_eq!(sl.model_display.as_deref(), Some("m"));
        assert_eq!(sl.total_input_tokens, Some(10));
        assert_eq!(sl.total_output_tokens, Some(2));
        assert_eq!(sl.cost_usd, Some(0.5));
        // Unknown model + no reported window → no context meter.
        assert!(sl.context_used_pct.is_none());
        assert!(sl.received_at.is_some());
    }

    #[test]
    fn status_line_estimates_cost_only_when_opted_in() {
        // Codex-shaped: cumulative tokens, no native cost, known model.
        // 2M input (1M cached) + 1M output on gpt-5-codex = 1.25 + 0.125 + 10.
        let mut acc = UsageAcc::new();
        acc.merge(
            Some("gpt-5-codex".into()),
            Some(2_000_000),
            Some(1_000_000),
            Some(1_000_000),
            None,
            None,
            None,
        );
        // Not opted in (Claude-style adapters) → no invented figure.
        assert!(acc.status_line().cost_usd.is_none());
        acc.estimate_costs();
        assert!((acc.status_line().cost_usd.unwrap() - 11.375).abs() < 1e-9);
        // A native cost, once it exists, always wins over the estimate.
        acc.merge(None, None, None, None, Some(2.0), None, None);
        assert_eq!(acc.status_line().cost_usd, Some(2.0));
    }

    #[test]
    fn status_line_synthesizes_rate_limit_warning_from_windows() {
        // Providers that never send an explicit warning (Codex) get one
        // synthesized once a window crosses 80% — highest window wins — and
        // it clears when the windows roll back under.
        let mut acc = UsageAcc::new();
        acc.merge_rate_limits(
            WindowReading {
                pct: Some(45.0),
                ..Default::default()
            },
            WindowReading {
                pct: Some(60.0),
                ..Default::default()
            },
            WindowReading::default(),
        );
        assert!(acc.status_line().rate_limit_warning.is_none());
        acc.merge_rate_limits(
            WindowReading {
                pct: Some(85.0),
                ..Default::default()
            },
            WindowReading {
                pct: Some(91.0),
                ..Default::default()
            },
            WindowReading::default(),
        );
        assert_eq!(
            acc.status_line().rate_limit_warning.as_deref(),
            Some("You're close to your 7-day usage limit — 91% used")
        );
        acc.merge_rate_limits(
            WindowReading {
                pct: Some(3.0),
                ..Default::default()
            },
            WindowReading {
                pct: Some(12.0),
                ..Default::default()
            },
            WindowReading::default(),
        );
        assert!(acc.status_line().rate_limit_warning.is_none());
        // An explicit provider warning always wins over the synthesized one.
        acc.merge_rate_limit_status(Some("explicit".into()), None);
        assert_eq!(
            acc.status_line().rate_limit_warning.as_deref(),
            Some("explicit")
        );
    }

    #[test]
    fn rate_limit_reading_without_utilization_still_lands() {
        // Claude's `rate_limit_event` can carry only `resetsAt` (no
        // `utilization`) — the reset time must reach the status line anyway,
        // and a later pct-only reading must not clobber it.
        let mut acc = UsageAcc::new();
        acc.merge_rate_limits(
            WindowReading {
                resets_at: Some(1_783_314_600),
                ..Default::default()
            },
            WindowReading::default(),
            WindowReading::default(),
        );
        acc.merge_rate_limits(
            WindowReading {
                pct: Some(19.0),
                ..Default::default()
            },
            WindowReading::default(),
            WindowReading::default(),
        );
        let sl = acc.status_line();
        assert_eq!(sl.five_hour_resets_at, Some(1_783_314_600));
        assert_eq!(sl.five_hour_pct, Some(19.0));
        assert!(sl.seven_day_pct.is_none());
    }

    #[test]
    fn monthly_overage_window_reaches_status_line() {
        // The `overage` window is a distinct bucket — it must not land in the
        // 5h/7d fields, and its reset time must survive a later pct-only read.
        let mut acc = UsageAcc::new();
        acc.merge_rate_limits(
            WindowReading::default(),
            WindowReading::default(),
            WindowReading {
                pct: Some(42.0),
                resets_at: Some(1_785_000_000),
                ..Default::default()
            },
        );
        acc.merge_rate_limits(
            WindowReading {
                pct: Some(19.0),
                ..Default::default()
            },
            WindowReading::default(),
            WindowReading::default(),
        );
        let sl = acc.status_line();
        assert_eq!(sl.monthly_pct, Some(42.0));
        assert_eq!(sl.monthly_resets_at, Some(1_785_000_000));
        assert_eq!(sl.five_hour_pct, Some(19.0));
    }

    #[test]
    fn status_line_computes_context_pct_from_reported_window() {
        let mut acc = UsageAcc::new();
        acc.merge(
            None,
            Some(10),
            Some(2),
            None,
            None,
            Some(50_000),
            Some(200_000),
        );
        let sl = acc.status_line();
        assert_eq!(sl.context_window_size, Some(200_000));
        assert!((sl.context_used_pct.unwrap() - 25.0).abs() < 0.001);
    }

    #[test]
    fn status_line_falls_back_to_window_table_by_model() {
        let mut acc = UsageAcc::new();
        acc.merge(
            Some("anthropic/claude-sonnet-4-5".into()),
            None,
            None,
            None,
            None,
            Some(100_000),
            None,
        );
        let sl = acc.status_line();
        assert_eq!(sl.context_window_size, Some(200_000));
        assert!((sl.context_used_pct.unwrap() - 50.0).abs() < 0.001);
        // Context % is capped at 100 even if occupancy overshoots the window.
        acc.merge(None, None, None, None, None, Some(999_999), None);
        assert!((acc.status_line().context_used_pct.unwrap() - 100.0).abs() < 0.001);
    }

    #[test]
    fn context_tokens_track_latest_not_max() {
        // Compaction shrinks the context — the meter must follow it down.
        let mut acc = UsageAcc::new();
        acc.merge(None, None, None, None, None, Some(150_000), Some(200_000));
        acc.merge(None, None, None, None, None, Some(30_000), None);
        let sl = acc.status_line();
        assert!((sl.context_used_pct.unwrap() - 15.0).abs() < 0.001);
    }

    #[test]
    fn additive_mode_sums_per_message_figures_while_default_maxes() {
        // OpenCode / Pi emit PER-MESSAGE figures. In additive mode the session
        // totals must be the SUM of the messages, not the single biggest one.
        let mut add = UsageAcc::new();
        add.additive();
        add.merge(
            None,
            Some(100),
            Some(10),
            None,
            Some(0.10),
            Some(1_000),
            None,
        );
        add.merge(
            None,
            Some(150),
            Some(20),
            None,
            Some(0.20),
            Some(1_200),
            None,
        );
        assert_eq!(add.input, Some(250), "additive input should sum 100+150");
        assert_eq!(add.output, Some(30), "additive output should sum 10+20");
        assert!(
            add.cost.map(|c| (c - 0.30).abs() < 1e-9).unwrap_or(false),
            "additive cost should sum 0.10+0.20, got {:?}",
            add.cost
        );
        // Context stays LATEST even in additive mode — never summed.
        assert_eq!(
            add.context_tokens,
            Some(1_200),
            "context_tokens must track latest, not sum, in additive mode"
        );

        // Default (Claude/Codex) adapters still MAX cumulative figures.
        let mut def = UsageAcc::new();
        def.merge(None, Some(100), Some(10), None, Some(0.10), None, None);
        def.merge(None, Some(150), Some(20), None, Some(0.20), None, None);
        assert_eq!(def.input, Some(150), "default input should max");
        assert_eq!(def.output, Some(20), "default output should max");
        assert!(
            def.cost.map(|c| (c - 0.20).abs() < 1e-9).unwrap_or(false),
            "default cost should max, got {:?}",
            def.cost
        );
    }

    #[test]
    fn context_window_table_matches_families() {
        assert_eq!(
            context_window_for("anthropic/claude-opus-4-8"),
            Some(200_000)
        );
        assert_eq!(context_window_for("claude-opus-4-8[1m]"), Some(1_000_000));
        // Fable/Mythos are 1M-native with no [1m] marker on the id.
        assert_eq!(context_window_for("claude-fable-5"), Some(1_000_000));
        assert_eq!(context_window_for("claude-mythos-1"), Some(1_000_000));
        assert_eq!(context_window_for("gpt-5-codex"), Some(272_000));
        assert_eq!(context_window_for("google/gemini-2.5-pro"), Some(1_048_576));
        assert_eq!(context_window_for("totally-unknown-model"), None);
    }

    #[test]
    fn requested_window_reads_the_1m_marker_off_bare_aliases() {
        // The marker is now a table row of its own, so the general lookup can
        // answer for the bare alias the composer actually sends too — it used
        // to need a "claude" in the id and returned None here.
        assert_eq!(context_window_for("opus[1m]"), Some(1_000_000));
        assert_eq!(requested_context_window_for("opus[1m]"), Some(1_000_000));
        assert_eq!(requested_context_window_for("sonnet[1m]"), Some(1_000_000));
        assert_eq!(
            requested_context_window_for("claude-opus-5[1m]"),
            Some(1_000_000)
        );
        assert_eq!(requested_context_window_for("fable"), Some(1_000_000));
        // An unmarked alias says NOTHING — not 200k. Callers keep whatever the
        // rates table resolved rather than have a coarse alias pin the window.
        assert_eq!(requested_context_window_for("opus"), None);
        assert_eq!(requested_context_window_for("claude-sonnet-5"), None);
        assert_eq!(requested_context_window_for(""), None);
    }

    #[test]
    fn error_update_surfaces_conversation_item() {
        // A managed-provider Error must reach the GUI as a conversation item,
        // not vanish into a log line.
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let mut mode = SessionMode::Unknown;
        let mut acc = UsageAcc::new();
        apply_updates(
            &store,
            &conv,
            "s-err",
            vec![AgentUpdate::Error("boom: model overloaded".into())],
            &mut mode,
            &mut acc,
        );
        let (_seq, items) = conv
            .snapshot("s-err")
            .expect("conversation exists for session");
        assert_eq!(items.len(), 1, "one item surfaced for the error");
        match &items[0] {
            ConversationItem::AssistantText { text, .. } => {
                assert!(
                    text.contains("boom: model overloaded"),
                    "error text carried through: {text}"
                );
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    /// The WRITER half of contracts/agent-error-marker-cases.json.
    ///
    /// There is no structured error field on the wire: an AgentUpdate::Error
    /// becomes an ordinary assistant turn prefixed with a marker (see the arm
    /// in apply_updates for why). The desktop's workerFailure.ts reads that
    /// prefix to tell a worker that DIED from one that FINISHED — a manager
    /// that cannot tell those apart records a crash as a landed outcome — so
    /// the prefix is a cross-process contract, and this pins our side of it
    /// against the same fixture the TypeScript reader's test loads.
    #[test]
    fn error_marker_matches_the_cross_language_contract() {
        const FIXTURE: &str = include_str!("../../../../contracts/agent-error-marker-cases.json");
        let spec: serde_json::Value =
            serde_json::from_str(FIXTURE).expect("agent-error-marker-cases.json parses");
        let marker = spec["marker"].as_str().expect("fixture has a marker");

        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let mut mode = SessionMode::Unknown;
        let mut acc = UsageAcc::new();
        apply_updates(
            &store,
            &conv,
            "s-marker",
            vec![AgentUpdate::Error("Credit balance is too low.".into())],
            &mut mode,
            &mut acc,
        );
        let (_seq, items) = conv.snapshot("s-marker").expect("conversation exists");
        match &items[0] {
            ConversationItem::AssistantText { text, .. } => {
                assert_eq!(
                    text.as_str(),
                    format!("{marker}Credit balance is too low.\n"),
                    "the error turn must be spelled exactly as the contract's marker + message; \
                     changing it here silently blinds the desktop's finished-vs-failed check"
                );
                // The trailing newline is the ONLY thing allowed past the
                // message, and it must not disturb the reader: the fixture's
                // own whitespace case pins that a padded turn still matches,
                // and workerFailure.errorMarkerReason trims and reads one line.
                assert_eq!(
                    text.trim_end(),
                    format!("{marker}Credit balance is too low."),
                    "nothing but trailing whitespace may follow the message"
                );
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }

        // Every fixture case the READER must call a failure has to be a string
        // this writer could actually have produced: marker-led. (The non-failure
        // cases are deliberately shapes we never emit.)
        for case in spec["cases"].as_array().expect("fixture has cases") {
            if case["failed"].as_bool() == Some(true) {
                let msg = case["finalMessage"].as_str().unwrap_or_default();
                assert!(
                    msg.trim_start().starts_with(marker),
                    "fixture case {:?} is marked failed but does not lead with the marker this \
                     writer emits",
                    case["name"]
                );
            }
        }
    }

    /// DEFECT 2 — "the session state lies". A `Busy` update (the CLI's
    /// `system`/`status:requesting` and `stream_event`/`message_start` frames
    /// both translate to one) arriving while an approval is parked used to
    /// call `set_managed_mode(Responding, None)`: the session then reported
    /// `responding` — which `agents.list` surfaces as `streaming` — while the
    /// CLI was still blocked on an unanswered `can_use_tool`, AND the pending
    /// card it needed to answer was dropped on the way through.
    #[test]
    fn busy_never_demotes_a_parked_approval_or_question() {
        for (mode, pending) in [
            (
                SessionMode::Approval,
                Pending::Approval {
                    tool: Some("Read".into()),
                    summary: Some("~/.workspacer/brief.md".into()),
                    raw: serde_json::json!({ "tool_name": "Read" }),
                },
            ),
            (
                SessionMode::Question,
                Pending::Question {
                    questions: vec![],
                    raw: serde_json::Value::Null,
                },
            ),
        ] {
            let store = SessionStore::new();
            let conv = ConversationStore::new();
            store.register_managed("s-block", "/tmp/proj", "claude");
            store.set_managed_mode(
                "s-block",
                mode,
                PendingWrite::Park(PendingOwner::Primary, pending),
            );
            let mut cur = mode;
            let mut acc = UsageAcc::new();

            apply_updates(
                &store,
                &conv,
                "s-block",
                vec![AgentUpdate::Busy],
                &mut cur,
                &mut acc,
            );

            let state = store.get("s-block").expect("session exists");
            assert_eq!(
                state.mode, mode,
                "a Busy ping must not report work while the agent is blocked on the user"
            );
            assert_eq!(cur, mode, "the driver's own mode tracker must agree");
            assert!(
                state.pending().is_some(),
                "the pending card must survive — without it the block is unanswerable"
            );
        }
    }

    /// The other half of the same guard: a batch that genuinely raises an
    /// approval still lands it, whichever order the updates arrive in, and a
    /// real turn end (`Idle`, from a `result` frame) still clears the pause.
    #[test]
    fn approval_still_lands_and_a_real_turn_end_still_clears_it() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed("s-mix", "/tmp/proj", "claude");
        let mut cur = SessionMode::Responding;
        let mut acc = UsageAcc::new();

        // Busy first, PermissionPending second — the pending still wins.
        apply_updates(
            &store,
            &conv,
            "s-mix",
            vec![
                AgentUpdate::Busy,
                AgentUpdate::PermissionPending {
                    id: None,
                    tool: Some("Read".into()),
                    summary: None,
                    raw: serde_json::json!({}),
                },
            ],
            &mut cur,
            &mut acc,
        );
        assert_eq!(cur, SessionMode::Approval);
        assert!(store.get("s-mix").is_some_and(|s| s.pending().is_some()));

        // A `result` frame is the CLI saying the turn is over — that must
        // still be able to release the pause, or a session could never leave
        // Approval after an interrupt.
        apply_updates(
            &store,
            &conv,
            "s-mix",
            vec![AgentUpdate::Idle],
            &mut cur,
            &mut acc,
        );
        assert_eq!(cur, SessionMode::Input);
        assert!(store.get("s-mix").is_some_and(|s| s.pending().is_none()));
    }

    #[test]
    fn apply_updates_plan_stores_state_and_pushes_conversation_item() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed("s-plan", "/tmp/proj", "codex");
        let mut mode = SessionMode::Input;
        let mut acc = UsageAcc::new();
        let plan = Plan {
            steps: vec![
                PlanStep {
                    content: "explore".into(),
                    status: PlanStatus::Completed,
                    active_form: None,
                },
                PlanStep {
                    content: "build".into(),
                    status: PlanStatus::InProgress,
                    active_form: None,
                },
            ],
            updated_at: None,
        };
        apply_updates(
            &store,
            &conv,
            "s-plan",
            vec![AgentUpdate::Plan(plan.clone())],
            &mut mode,
            &mut acc,
        );
        // Stored on the session state...
        assert_eq!(store.get("s-plan").and_then(|s| s.plan), Some(plan));
        // ...and pushed as a conversation item.
        let (_seq, items) = conv.snapshot("s-plan").expect("conversation exists");
        assert!(items
            .iter()
            .any(|i| matches!(i, ConversationItem::Plan { steps, .. } if steps.len() == 2)));
    }

    #[test]
    fn plan_from_value_reads_shapes_and_rejects_non_plans() {
        // `update_plan` args shape.
        let p = plan_from_value(&serde_json::json!({ "plan": [
            { "step": "a", "status": "in_progress" }
        ]}))
        .expect("plan shape parses");
        assert_eq!(p.steps.len(), 1);
        assert_eq!(p.steps[0].status, PlanStatus::InProgress);
        // todo-list shape with a boolean completed flag.
        let p = plan_from_value(&serde_json::json!({ "items": [
            { "text": "a", "completed": true }
        ]}))
        .expect("todo shape parses");
        assert_eq!(p.steps[0].status, PlanStatus::Completed);
        // A non-plan value is rejected (safe to probe any item).
        assert!(plan_from_value(&serde_json::json!({ "command": ["ls"] })).is_none());
        assert!(plan_from_value(&serde_json::json!({ "plan": [] })).is_none());
    }

    #[test]
    fn conversation_item_mapping() {
        assert!(matches!(
            conversation_item(&AgentUpdate::AssistantText("x".into())),
            Some(ConversationItem::AssistantText { .. })
        ));
        assert!(matches!(
            conversation_item(&AgentUpdate::UserText("x".into())),
            Some(ConversationItem::UserMessage { .. })
        ));
        assert!(conversation_item(&AgentUpdate::Idle).is_none());
        assert!(conversation_item(&AgentUpdate::Busy).is_none());
        // Plan is applied via set_plan, never mapped to a plain conversation item.
        assert!(conversation_item(&AgentUpdate::Plan(Plan {
            steps: vec![],
            updated_at: None,
        }))
        .is_none());
    }

    #[test]
    fn usage_acc_takes_max_and_latest_model() {
        let mut acc = UsageAcc::new();
        acc.merge(
            Some("a".into()),
            Some(100),
            Some(10),
            None,
            Some(0.1),
            None,
            None,
        );
        acc.merge(
            Some("b".into()),
            Some(80),
            Some(20),
            None,
            Some(0.2),
            None,
            None,
        );
        // model = latest, tokens/cost = max (never regress mid-turn).
        assert_eq!(acc.model.as_deref(), Some("b"));
        assert_eq!(acc.input, Some(100));
        assert_eq!(acc.output, Some(20));
        assert_eq!(acc.cost, Some(0.2));
    }
}

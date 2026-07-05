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

pub mod codex;
pub mod codex_rollout;
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
use crate::session::state::{Pending, Plan, PlanStatus, PlanStep, SessionMode, StatusLine};
use crate::session::store::WrapperHandle;
use crate::session::{ConversationStore, SessionStore};
use crate::wrapper::pty;

/// One selectable model for a managed provider, as surfaced by the spawn
/// dialog's model picker. `id` is the value passed back as the model override
/// (the provider's own id format); `label` is the human display name; `default`
/// marks the provider's out-of-the-box choice. Populated by each provider's
/// `list_models` (live-queried from the CLI/server at pick time).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub default: bool,
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
        cost_usd: Option<f64>,
        /// Tokens currently occupying the model's context window (the latest
        /// turn's total, including cache) — NOT cumulative like input/output.
        context_tokens: Option<u64>,
        /// The model's context window size, when the provider reports it.
        /// Absent → [`context_window_for`] falls back to a table by model id.
        context_window: Option<u64>,
    },
    /// The account's 5h/7d rate-limit windows, when the provider reports them
    /// (Codex does) — same meaning as the fields Claude's statusLine carries.
    RateLimits {
        five_hour_pct: Option<f64>,
        five_hour_resets_at: Option<i64>,
        seven_day_pct: Option<f64>,
        seven_day_resets_at: Option<i64>,
    },
    /// A session-level error message.
    Error(String),
    /// The agent's current plan / checklist (Codex's `update_plan` / todo list).
    /// Last-write-wins full replacement — carried into the conversation store as
    /// a `plan` item by `apply_updates` (via `SessionStore::set_plan`), never as
    /// a `conversation_item`.
    Plan(Plan),
}

/// Parse a Codex `RateLimitSnapshot` — camelCase on the app-server wire
/// (`usedPercent`/`windowDurationMins`/`resetsAt`), snake_case in rollout
/// `event_msg`s (`used_percent`/`window_minutes`/`resets_at`) — into a
/// [`AgentUpdate::RateLimits`]. Each window is bucketed by its duration (≤12h →
/// the 5h slot, longer → the 7d slot), falling back to primary→5h /
/// secondary→7d when the duration is absent.
pub(crate) fn rate_limits_from(v: &Value) -> Option<AgentUpdate> {
    fn window(w: &Value) -> (Option<f64>, Option<i64>, Option<u64>) {
        let pick = |keys: [&str; 2]| keys.iter().find_map(|k| w.get(*k));
        (
            pick(["usedPercent", "used_percent"]).and_then(Value::as_f64),
            pick(["resetsAt", "resets_at"]).and_then(Value::as_i64),
            pick(["windowDurationMins", "window_minutes"]).and_then(Value::as_u64),
        )
    }
    let mut five: (Option<f64>, Option<i64>) = (None, None);
    let mut seven: (Option<f64>, Option<i64>) = (None, None);
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
            seven = (pct, resets);
        } else {
            five = (pct, resets);
        }
    }
    if five == (None, None) && seven == (None, None) {
        return None;
    }
    Some(AgentUpdate::RateLimits {
        five_hour_pct: five.0,
        five_hour_resets_at: five.1,
        seven_day_pct: seven.0,
        seven_day_resets_at: seven.1,
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
/// provider's own events don't report one. Prefix/substring matched on the
/// model id (lowercased). Deliberately conservative: an unknown model returns
/// `None` and the context meter simply doesn't render — a missing meter beats a
/// wrong one.
pub fn context_window_for(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();
    // Order matters where families overlap (check the more specific first).
    if m.contains("gemini") {
        return Some(1_048_576);
    }
    if m.contains("gpt-4.1") {
        return Some(1_047_576);
    }
    if m.contains("claude") {
        // 1M-context variants advertise it in the id (e.g. `[1m]`).
        return Some(if m.contains("[1m]") || m.contains("-1m") {
            1_000_000
        } else {
            200_000
        });
    }
    if m.contains("gpt-5") || m.contains("codex") {
        return Some(272_000);
    }
    if m.contains("gpt-4o") {
        return Some(128_000);
    }
    if m.starts_with("o3") || m.starts_with("o4") || m.contains("/o3") || m.contains("/o4") {
        return Some(200_000);
    }
    if m.contains("grok") {
        return Some(256_000);
    }
    if m.contains("deepseek") {
        return Some(131_072);
    }
    if m.contains("kimi") || m.contains("qwen") {
        return Some(262_144);
    }
    None
}

/// Map an `AgentUpdate` to a conversation item, when it represents one.
pub fn conversation_item(update: &AgentUpdate) -> Option<ConversationItem> {
    match update {
        AgentUpdate::AssistantText(text) => Some(ConversationItem::AssistantText {
            text: text.clone(),
            timestamp: None,
        }),
        AgentUpdate::UserText(text) => Some(ConversationItem::UserMessage {
            text: text.clone(),
            timestamp: None,
        }),
        AgentUpdate::ToolUse { id, name, input } => Some(ConversationItem::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
            timestamp: None,
        }),
        AgentUpdate::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(ConversationItem::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: content.clone(),
            is_error: *is_error,
            timestamp: None,
        }),
        _ => None,
    }
}

/// Running tally of model/usage/cost across a managed session, for the status
/// line. Tokens/cost arrive as per-message or per-step partials, so we take the
/// max — the displayed totals never regress mid-turn. Context occupancy is the
/// LATEST reading (not max: compaction legitimately shrinks it).
#[derive(Default)]
pub struct UsageAcc {
    model: Option<String>,
    input: Option<u64>,
    output: Option<u64>,
    cost: Option<f64>,
    context_tokens: Option<u64>,
    context_window: Option<u64>,
    five_hour_pct: Option<f64>,
    five_hour_resets_at: Option<i64>,
    seven_day_pct: Option<f64>,
    seven_day_resets_at: Option<i64>,
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
    #[allow(clippy::too_many_arguments)]
    fn merge(
        &mut self,
        model: Option<String>,
        input: Option<u64>,
        output: Option<u64>,
        cost: Option<f64>,
        context_tokens: Option<u64>,
        context_window: Option<u64>,
    ) {
        if model.is_some() {
            self.model = model;
        }
        if let Some(i) = input {
            self.input = Some(self.input.map_or(i, |c| c.max(i)));
        }
        if let Some(o) = output {
            self.output = Some(self.output.map_or(o, |c| c.max(o)));
        }
        if let Some(c) = cost {
            self.cost = Some(self.cost.map_or(c, |p| p.max(c)));
        }
        if context_tokens.is_some() {
            self.context_tokens = context_tokens; // latest, not max — compaction shrinks it
        }
        if context_window.is_some() {
            self.context_window = context_window;
        }
    }

    /// Fold in a rate-limit reading — latest wins per window (they only move
    /// forward between readings; a lower % just means the window rolled).
    fn merge_rate_limits(
        &mut self,
        five_hour_pct: Option<f64>,
        five_hour_resets_at: Option<i64>,
        seven_day_pct: Option<f64>,
        seven_day_resets_at: Option<i64>,
    ) {
        if five_hour_pct.is_some() {
            self.five_hour_pct = five_hour_pct;
            self.five_hour_resets_at = five_hour_resets_at;
        }
        if seven_day_pct.is_some() {
            self.seven_day_pct = seven_day_pct;
            self.seven_day_resets_at = seven_day_resets_at;
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
        StatusLine {
            model_display: self.model.clone(),
            context_used_pct: pct,
            context_window_size: window,
            total_input_tokens: self.input,
            total_output_tokens: self.output,
            cost_usd: self.cost,
            five_hour_pct: self.five_hour_pct,
            five_hour_resets_at: self.five_hour_resets_at,
            seven_day_pct: self.seven_day_pct,
            seven_day_resets_at: self.seven_day_resets_at,
            received_at: Some(OffsetDateTime::now_utc()),
        }
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
    let mut new_mode: Option<SessionMode> = None;
    let mut pending: Option<Pending> = None;
    let mut usage_changed = false;
    // Latest plan in this batch (last-write-wins); applied after the item push
    // so its own conversation item lands just past the batch's items.
    let mut plan: Option<Plan> = None;

    for update in &updates {
        match update {
            AgentUpdate::Idle => new_mode = Some(SessionMode::Input),
            AgentUpdate::Busy => {
                if new_mode.is_none() {
                    new_mode = Some(SessionMode::Responding);
                }
            }
            AgentUpdate::PermissionPending {
                tool, summary, raw, ..
            } => {
                new_mode = Some(SessionMode::Approval);
                // NOTE: surfacing the pending approval is accurate telemetry, but
                // forwarding the user's decision back to the provider's approval
                // API is a follow-up (Phase 4).
                pending = Some(Pending::Approval {
                    tool: tool.clone(),
                    summary: summary.clone(),
                    raw: raw.clone(),
                });
            }
            AgentUpdate::Usage {
                model,
                input_tokens,
                output_tokens,
                cost_usd,
                context_tokens,
                context_window,
            } => {
                acc.merge(
                    model.clone(),
                    *input_tokens,
                    *output_tokens,
                    *cost_usd,
                    *context_tokens,
                    *context_window,
                );
                usage_changed = true;
            }
            AgentUpdate::RateLimits {
                five_hour_pct,
                five_hour_resets_at,
                seven_day_pct,
                seven_day_resets_at,
            } => {
                acc.merge_rate_limits(
                    *five_hour_pct,
                    *five_hour_resets_at,
                    *seven_day_pct,
                    *seven_day_resets_at,
                );
                usage_changed = true;
            }
            AgentUpdate::Error(msg) => {
                // A managed provider (Codex/OpenCode/Pi) reported an agent-side
                // failure. Log it and surface it in the conversation so the GUI
                // shows it instead of silently swallowing it. The renderer only
                // renders known item kinds, so ride it in as assistant text with
                // a clear marker rather than a bespoke variant it would drop.
                tracing::warn!(session = %session_id, error = %msg, "managed session error");
                items.push(ConversationItem::AssistantText {
                    text: format!("⚠️ Error: {msg}"),
                    timestamp: None,
                });
            }
            AgentUpdate::Plan(p) => plan = Some(p.clone()),
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
    if let Some(mode) = new_mode {
        if mode != *cur_mode || mode == SessionMode::Approval {
            store.set_managed_mode(session_id, mode, pending);
            *cur_mode = mode;
        }
    }
    if usage_changed {
        store.apply_status_line(session_id, acc.status_line());
    }
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
        &HashMap::new(),
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
    tokio::spawn(async move {
        while let Some(chunk) = out_rx.recv().await {
            store_out.record_output(&sid, &chunk).await;
        }
        // TUI exited (reader EOF) — reap it so it isn't left a zombie.
        store_out.reap_pty(&sid);
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

    #[test]
    fn model_cache_ttl_and_stale_fallback() {
        let key = "test-provider:test-bin-abc123"; // unique: MODEL_CACHE is global
        let models = vec![ModelInfo {
            id: "m1".into(),
            label: "M1".into(),
            default: true,
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
        acc.merge(Some("m".into()), Some(10), Some(2), Some(0.5), None, None);
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
    fn status_line_computes_context_pct_from_reported_window() {
        let mut acc = UsageAcc::new();
        acc.merge(None, Some(10), Some(2), None, Some(50_000), Some(200_000));
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
            Some(100_000),
            None,
        );
        let sl = acc.status_line();
        assert_eq!(sl.context_window_size, Some(200_000));
        assert!((sl.context_used_pct.unwrap() - 50.0).abs() < 0.001);
        // Context % is capped at 100 even if occupancy overshoots the window.
        acc.merge(None, None, None, None, Some(999_999), None);
        assert!((acc.status_line().context_used_pct.unwrap() - 100.0).abs() < 0.001);
    }

    #[test]
    fn context_tokens_track_latest_not_max() {
        // Compaction shrinks the context — the meter must follow it down.
        let mut acc = UsageAcc::new();
        acc.merge(None, None, None, None, Some(150_000), Some(200_000));
        acc.merge(None, None, None, None, Some(30_000), None);
        let sl = acc.status_line();
        assert!((sl.context_used_pct.unwrap() - 15.0).abs() < 0.001);
    }

    #[test]
    fn context_window_table_matches_families() {
        assert_eq!(
            context_window_for("anthropic/claude-opus-4-8"),
            Some(200_000)
        );
        assert_eq!(context_window_for("claude-opus-4-8[1m]"), Some(1_000_000));
        assert_eq!(context_window_for("gpt-5-codex"), Some(272_000));
        assert_eq!(context_window_for("google/gemini-2.5-pro"), Some(1_048_576));
        assert_eq!(context_window_for("totally-unknown-model"), None);
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
        acc.merge(Some("a".into()), Some(100), Some(10), Some(0.1), None, None);
        acc.merge(Some("b".into()), Some(80), Some(20), Some(0.2), None, None);
        // model = latest, tokens/cost = max (never regress mid-turn).
        assert_eq!(acc.model.as_deref(), Some("b"));
        assert_eq!(acc.input, Some(100));
        assert_eq!(acc.output, Some(20));
        assert_eq!(acc.cost, Some(0.2));
    }
}

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
use crate::session::state::{Pending, SessionMode, StatusLine};
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
static MODEL_CACHE: once_cell::sync::Lazy<std::sync::Mutex<std::collections::HashMap<String, ModelCacheEntry>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
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
        cache.insert(key.to_string(), ModelCacheEntry { at: std::time::Instant::now(), models: models.to_vec() });
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
                tracing::warn!(?err, key, "model list failed; serving last-known-good cached models");
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
    },
    /// A session-level error message.
    Error(String),
}

/// Build a `StatusLine` from usage telemetry, for `SessionStore::apply_status_line`.
pub fn status_from_usage(
    model: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cost_usd: Option<f64>,
) -> StatusLine {
    StatusLine {
        model_display: model,
        total_input_tokens: input_tokens,
        total_output_tokens: output_tokens,
        cost_usd,
        received_at: Some(OffsetDateTime::now_utc()),
        ..Default::default()
    }
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
        AgentUpdate::ToolResult { tool_use_id, content, is_error } => Some(ConversationItem::ToolResult {
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
/// max — the displayed totals never regress mid-turn.
#[derive(Default)]
pub struct UsageAcc {
    model: Option<String>,
    input: Option<u64>,
    output: Option<u64>,
    cost: Option<f64>,
}

impl UsageAcc {
    pub fn new() -> Self {
        Self::default()
    }
    fn merge(&mut self, model: Option<String>, input: Option<u64>, output: Option<u64>, cost: Option<f64>) {
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

    for update in &updates {
        match update {
            AgentUpdate::Idle => new_mode = Some(SessionMode::Input),
            AgentUpdate::Busy => {
                if new_mode.is_none() {
                    new_mode = Some(SessionMode::Responding);
                }
            }
            AgentUpdate::PermissionPending { tool, summary, .. } => {
                new_mode = Some(SessionMode::Approval);
                // NOTE: surfacing the pending approval is accurate telemetry, but
                // forwarding the user's decision back to the provider's approval
                // API is a follow-up (Phase 4).
                pending = Some(Pending::Approval {
                    tool: tool.clone(),
                    summary: summary.clone(),
                    raw: Value::Null,
                });
            }
            AgentUpdate::Usage { model, input_tokens, output_tokens, cost_usd } => {
                acc.merge(model.clone(), *input_tokens, *output_tokens, *cost_usd);
                usage_changed = true;
            }
            AgentUpdate::Error(msg) => {
                tracing::debug!(session = %session_id, error = %msg, "managed session error");
            }
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
    if let Some(mode) = new_mode {
        if mode != *cur_mode || mode == SessionMode::Approval {
            store.set_managed_mode(session_id, mode, pending);
            *cur_mode = mode;
        }
    }
    if usage_changed {
        store.apply_status_line(
            session_id,
            status_from_usage(acc.model.clone(), acc.input, acc.output, acc.cost),
        );
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
        PtySize { cols: 120, rows: 32, pixel_width: 0, pixel_height: 0 },
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
        let models = vec![ModelInfo { id: "m1".into(), label: "M1".into(), default: true }];

        // Fresh entry is served both as a fresh hit and as last-known-good.
        model_cache_put(key, &models);
        assert!(model_cache_get(key, Some(MODEL_CACHE_TTL)).is_some(), "fresh hit");
        assert!(model_cache_get(key, None).is_some(), "any-age hit");

        // An entry older than the TTL is NOT a fresh hit, but IS still available
        // as the stale fallback (what we serve when a live query fails).
        if let Some(old) = std::time::Instant::now().checked_sub(MODEL_CACHE_TTL * 2) {
            MODEL_CACHE.lock().unwrap().insert(key.into(), ModelCacheEntry { at: old, models });
            assert!(model_cache_get(key, Some(MODEL_CACHE_TTL)).is_none(), "stale is not a fresh hit");
            assert!(model_cache_get(key, None).is_some(), "stale still served as last-known-good");
        }

        // Unknown key → nothing cached.
        assert!(model_cache_get("test-provider:never-seen", None).is_none());
    }

    #[test]
    fn status_from_usage_sets_only_known_fields() {
        let sl = status_from_usage(Some("m".into()), Some(10), Some(2), Some(0.5));
        assert_eq!(sl.model_display.as_deref(), Some("m"));
        assert_eq!(sl.total_input_tokens, Some(10));
        assert_eq!(sl.total_output_tokens, Some(2));
        assert_eq!(sl.cost_usd, Some(0.5));
        assert!(sl.context_used_pct.is_none());
        assert!(sl.received_at.is_some());
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
    }

    #[test]
    fn usage_acc_takes_max_and_latest_model() {
        let mut acc = UsageAcc::new();
        acc.merge(Some("a".into()), Some(100), Some(10), Some(0.1));
        acc.merge(Some("b".into()), Some(80), Some(20), Some(0.2));
        // model = latest, tokens/cost = max (never regress mid-turn).
        assert_eq!(acc.model.as_deref(), Some("b"));
        assert_eq!(acc.input, Some(100));
        assert_eq!(acc.output, Some(20));
        assert_eq!(acc.cost, Some(0.2));
    }
}

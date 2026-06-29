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
pub mod opencode;

use serde_json::Value;
use time::OffsetDateTime;

use crate::session::conversation::ConversationItem;
use crate::session::state::{Pending, SessionMode, StatusLine};
use crate::session::{ConversationStore, SessionStore};

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
            AgentUpdate::AssistantText(_) | AgentUpdate::UserText(_) | AgentUpdate::ToolUse { .. } => {
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

#[cfg(test)]
mod tests {
    use super::*;

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

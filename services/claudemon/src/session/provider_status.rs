//! Live provider status a managed driver reports beside the mode state
//! machine: per-message delivery receipts, MCP server health, running hooks
//! and fast mode. Flattened into [`super::SessionState`], so every field is a
//! top-level snapshot key (`message_receipts`, `mcp_servers`, `active_hooks`,
//! `fast_mode`, `fast_mode_disabled_reason`). Runtime facts only: nothing here
//! is consulted by the mode state machine, and each field is omitted while it
//! carries nothing.
//!
//! The control requests that refresh or change these facts ride
//! [`StatusControl`] (one channel per driver, see
//! `SessionStore::register_managed_status_control`).

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// Receipts kept per session, newest last. The hub matches one uuid against
/// them seconds after it sent the message, so a short window is enough.
pub const RECEIPTS_KEPT: usize = 32;
/// Settled (`completed` / `cancelled`) receipts kept, newest.
pub const SETTLED_RECEIPTS_KEPT: usize = 10;
/// Receipts older than this are dropped (the hub settles an uncertain send
/// within seconds).
pub const RECEIPT_MAX_AGE_MS: i64 = 10 * 60 * 1000;
/// Running hooks kept per session (a runaway emitter cannot grow the row).
pub const ACTIVE_HOOKS_KEPT: usize = 32;
/// MCP servers kept per session.
pub const MCP_SERVERS_KEPT: usize = 128;
/// Bytes of any provider-reported string kept (names, errors, reasons).
const TEXT_KEPT: usize = 512;

/// What a provider reported about one message WE sent, by the uuid stamped on
/// it. Claude stream `command_lifecycle` vocabulary, verbatim:
/// `queued` (the CLI holds it), `started` (its turn began), `completed`
/// (its turn ended) and `cancelled` (an interrupt dropped it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReceipt {
    pub uuid: String,
    pub state: String,
    /// Epoch milliseconds of the latest state.
    pub at: i64,
}

impl MessageReceipt {
    /// Whether the agent has demonstrably read the message.
    pub fn delivered(&self) -> bool {
        matches!(self.state.as_str(), "started" | "completed")
    }
}

/// One MCP server's health, normalized to Claude's `McpServerStatus.status`
/// vocabulary: `connected` | `failed` | `needs-auth` | `pending` | `disabled`
/// (Codex statuses are mapped onto it, see `providers::codex_status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A hook that has started and not yet reported a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveHook {
    /// The provider's id for this run; its response names the same id.
    pub id: String,
    /// The hook's name (Claude `hook_name`; Codex has none, so its handler).
    pub name: String,
    /// The hook event (`PreToolUse`, `Stop`, Codex `preToolUse`, …).
    pub event: String,
    /// Epoch milliseconds, our clock.
    pub started_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_receipts: Vec<MessageReceipt>,
    /// `None` until the provider first reported its servers; `Some(vec![])`
    /// is a confirmed "no servers".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<McpServer>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_hooks: Vec<ActiveHook>,
    /// The CLI's own `fast_mode_state` (`on` | `off` | `cooldown`). Only ever
    /// a state the provider reported, never what we asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_mode: Option<String>,
    /// The CLI's `fast_mode_disabled_reason` beside it (`free`,
    /// `sdk_opt_in_required`, …); cleared whenever a report carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_mode_disabled_reason: Option<String>,
}

pub(crate) fn clip(s: &str) -> String {
    if s.len() <= TEXT_KEPT {
        return s.to_owned();
    }
    let mut end = TEXT_KEPT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

impl ProviderStatus {
    /// Record a receipt; `false` when nothing changed. A uuid keeps one row
    /// (its latest state), moved to the newest end.
    pub fn record_receipt(&mut self, uuid: &str, state: &str, at: i64) -> bool {
        if uuid.is_empty() || state.is_empty() {
            return false;
        }
        if let Some(pos) = self.message_receipts.iter().position(|r| r.uuid == uuid) {
            if self.message_receipts[pos].state == state {
                return false;
            }
            self.message_receipts.remove(pos);
        }
        self.message_receipts.push(MessageReceipt {
            uuid: clip(uuid),
            state: clip(state),
            at,
        });
        self.prune_receipts(at);
        true
    }

    /// Keep receipts useful, not historic: none older than
    /// [`RECEIPT_MAX_AGE_MS`], at most [`SETTLED_RECEIPTS_KEPT`] settled ones
    /// (`completed` / `cancelled`; a message still `queued`/`started` is kept),
    /// and never more than [`RECEIPTS_KEPT`].
    fn prune_receipts(&mut self, now: i64) {
        self.message_receipts
            .retain(|r| now.saturating_sub(r.at) <= RECEIPT_MAX_AGE_MS);
        let settled = |r: &MessageReceipt| matches!(r.state.as_str(), "completed" | "cancelled");
        let mut excess = self
            .message_receipts
            .iter()
            .filter(|r| settled(r))
            .count()
            .saturating_sub(SETTLED_RECEIPTS_KEPT);
        self.message_receipts.retain(|r| {
            if excess > 0 && settled(r) {
                excess -= 1;
                return false;
            }
            true
        });
        if self.message_receipts.len() > RECEIPTS_KEPT {
            let excess = self.message_receipts.len() - RECEIPTS_KEPT;
            self.message_receipts.drain(..excess);
        }
    }

    /// Replace the whole server list (a full report).
    pub fn set_mcp_servers(&mut self, mut servers: Vec<McpServer>) -> bool {
        servers.truncate(MCP_SERVERS_KEPT);
        if self.mcp_servers.as_ref() == Some(&servers) {
            return false;
        }
        self.mcp_servers = Some(servers);
        true
    }

    /// Update one server (an incremental report), adding it when new.
    pub fn upsert_mcp_server(&mut self, server: McpServer) -> bool {
        let servers = self.mcp_servers.get_or_insert_with(Vec::new);
        match servers.iter().position(|s| s.name == server.name) {
            Some(i) if servers[i] == server => false,
            Some(i) => {
                servers[i] = server;
                true
            }
            None if servers.len() >= MCP_SERVERS_KEPT => false,
            None => {
                servers.push(server);
                true
            }
        }
    }

    pub fn hook_started(&mut self, hook: ActiveHook) -> bool {
        if self.active_hooks.iter().any(|h| h.id == hook.id) {
            return false;
        }
        if self.active_hooks.len() >= ACTIVE_HOOKS_KEPT {
            self.active_hooks.remove(0);
        }
        self.active_hooks.push(hook);
        true
    }

    pub fn hook_finished(&mut self, id: &str) -> bool {
        let before = self.active_hooks.len();
        self.active_hooks.retain(|h| h.id != id);
        self.active_hooks.len() != before
    }

    pub fn set_fast_mode(&mut self, state: &str, reason: Option<&str>) -> bool {
        let state = Some(clip(state));
        let reason = reason.filter(|r| !r.is_empty()).map(clip);
        if self.fast_mode == state && self.fast_mode_disabled_reason == reason {
            return false;
        }
        self.fast_mode = state;
        self.fast_mode_disabled_reason = reason;
        true
    }

    /// Keep the status line's older `capabilities` view (`fast_mode`, the MCP
    /// server count and `inventory.mcp_servers`) in agreement with these
    /// newer, fresher fields: one source, two shapes.
    pub fn sync_capabilities(&self, caps: &mut super::state::Capabilities) {
        if let Some(state) = &self.fast_mode {
            caps.fast_mode = Some(state == "on");
        }
        let Some(servers) = &self.mcp_servers else {
            return;
        };
        caps.mcp_servers = servers.len() as u32;
        if let Some(inventory) = caps.inventory.as_mut() {
            let previous = std::mem::take(&mut inventory.mcp_servers);
            inventory.mcp_servers = servers
                .iter()
                .map(|server| {
                    let mut item = previous
                        .iter()
                        .find(|item| item.name == server.name)
                        .cloned()
                        .unwrap_or_else(|| super::state::ContextItem {
                            name: server.name.clone(),
                            ..Default::default()
                        });
                    item.status = Some(server.status.clone());
                    item
                })
                .collect();
        }
    }

    /// A session that stopped runs no hook.
    pub fn end(&mut self) {
        self.active_hooks.clear();
    }
}

/// An interrupt's receipt: the uuids of OUR messages the CLI listed as
/// pending when the interrupt landed. `still_queued` ones WILL run after the
/// interrupt (never resend them); `cancelled` ones will not (only with
/// `cancel_queued`, which Workspacer does not send today). `supported` is
/// false on a CLI without `interrupt_receipt_v1`, where both lists are empty
/// because nothing was reported, not because nothing was pending.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct InterruptReceipt {
    pub supported: bool,
    pub still_queued: Vec<String>,
    pub cancelled: Vec<String>,
}

/// A control request for a driver's live status (see module docs). Each
/// carries the reply its caller waits on, answered with the provider's own
/// verdict.
#[derive(Debug)]
pub enum StatusControl {
    /// Refresh the MCP server list from the provider.
    McpStatus {
        reply: oneshot::Sender<Result<Vec<McpServer>, String>>,
    },
    /// Reconnect one MCP server (Claude), or reload the MCP configuration
    /// (Codex, which has no per-server reconnect).
    McpReconnect {
        server: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Turn fast mode on or off. `Ok` means the provider accepted the
    /// setting, not that fast mode is on: the snapshot's `fast_mode` changes
    /// only when the provider reports it.
    FastMode {
        enabled: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Interrupt the current turn and report the receipt.
    Interrupt {
        reply: oneshot::Sender<Result<InterruptReceipt, String>>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_keep_one_row_per_uuid_newest_last_and_bounded() {
        let mut s = ProviderStatus::default();
        assert!(s.record_receipt("a", "queued", 1));
        assert!(
            !s.record_receipt("a", "queued", 2),
            "same state is no change"
        );
        assert!(s.record_receipt("b", "queued", 3));
        assert!(s.record_receipt("a", "started", 4));
        let order: Vec<_> = s
            .message_receipts
            .iter()
            .map(|r| (r.uuid.as_str(), r.state.as_str()))
            .collect();
        assert_eq!(order, [("b", "queued"), ("a", "started")]);
        assert!(s.message_receipts[1].delivered());
        assert!(!s.message_receipts[0].delivered());
        for i in 0..100 {
            s.record_receipt(&format!("q{i}"), "queued", 10);
        }
        assert_eq!(s.message_receipts.len(), RECEIPTS_KEPT);
        assert_eq!(s.message_receipts.last().unwrap().uuid, "q99");
        // Settled receipts: only the newest ten stay.
        let mut s = ProviderStatus::default();
        s.record_receipt("live", "started", 0);
        for i in 0..20 {
            s.record_receipt(&format!("u{i}"), "completed", i);
        }
        let settled: Vec<_> = s
            .message_receipts
            .iter()
            .filter(|r| r.state == "completed")
            .collect();
        assert_eq!(settled.len(), SETTLED_RECEIPTS_KEPT);
        assert_eq!(settled[0].uuid, "u10");
        assert!(
            s.message_receipts.iter().any(|r| r.uuid == "live"),
            "unsettled survive"
        );
        // …and nothing older than ten minutes.
        s.record_receipt("late", "queued", RECEIPT_MAX_AGE_MS + 100);
        assert_eq!(s.message_receipts.len(), 1);
        assert_eq!(s.message_receipts[0].uuid, "late");
        assert!(!s.record_receipt("", "queued", 0));
    }

    #[test]
    fn hooks_start_and_finish_by_id() {
        let mut s = ProviderStatus::default();
        let hook = |id: &str| ActiveHook {
            id: id.into(),
            name: "Stop".into(),
            event: "Stop".into(),
            started_at: 1,
        };
        assert!(s.hook_started(hook("h1")));
        assert!(!s.hook_started(hook("h1")));
        assert!(s.hook_started(hook("h2")));
        assert!(s.hook_finished("h1"));
        assert!(!s.hook_finished("h1"));
        assert_eq!(s.active_hooks.len(), 1);
        s.end();
        assert!(s.active_hooks.is_empty());
    }

    #[test]
    fn mcp_servers_replace_and_upsert() {
        let mut s = ProviderStatus::default();
        let server = |name: &str, status: &str| McpServer {
            name: name.into(),
            status: status.into(),
            error: None,
        };
        assert!(s.set_mcp_servers(vec![server("a", "pending")]));
        assert!(!s.set_mcp_servers(vec![server("a", "pending")]));
        assert!(s.upsert_mcp_server(server("a", "connected")));
        assert!(s.upsert_mcp_server(server("b", "failed")));
        assert!(!s.upsert_mcp_server(server("b", "failed")));
        assert_eq!(s.mcp_servers.as_ref().unwrap().len(), 2);
        assert!(s.set_mcp_servers(Vec::new()));
        assert_eq!(s.mcp_servers, Some(Vec::new()));
    }

    #[test]
    fn the_older_capabilities_view_follows_the_new_fields() {
        let mut caps = super::super::state::Capabilities {
            fast_mode: Some(true),
            mcp_servers: 3,
            inventory: Some(super::super::state::ContextInventory {
                mcp_servers: vec![super::super::state::ContextItem {
                    name: "a".into(),
                    source: Some("claudeai".into()),
                    status: Some("pending".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut s = ProviderStatus::default();
        s.sync_capabilities(&mut caps);
        assert_eq!(caps.mcp_servers, 3, "nothing reported, nothing changed");
        s.set_fast_mode("cooldown", None);
        s.set_mcp_servers(vec![
            McpServer {
                name: "a".into(),
                status: "connected".into(),
                error: None,
            },
            McpServer {
                name: "b".into(),
                status: "failed".into(),
                error: None,
            },
        ]);
        s.sync_capabilities(&mut caps);
        assert_eq!(caps.fast_mode, Some(false));
        assert_eq!(caps.mcp_servers, 2);
        let items = &caps.inventory.as_ref().unwrap().mcp_servers;
        assert_eq!(items[0].status.as_deref(), Some("connected"));
        assert_eq!(
            items[0].source.as_deref(),
            Some("claudeai"),
            "enrichment kept"
        );
        assert_eq!(items[1].name, "b");
    }

    #[test]
    fn fast_mode_tracks_state_and_reason_together() {
        let mut s = ProviderStatus::default();
        assert!(s.set_fast_mode("off", Some("sdk_opt_in_required")));
        assert!(!s.set_fast_mode("off", Some("sdk_opt_in_required")));
        assert!(s.set_fast_mode("off", None));
        assert_eq!(s.fast_mode_disabled_reason, None);
        assert!(s.set_fast_mode("on", Some("")));
        assert_eq!(s.fast_mode.as_deref(), Some("on"));
        assert_eq!(s.fast_mode_disabled_reason, None);
    }

    #[test]
    fn empty_status_serializes_to_nothing_and_flattens() {
        assert_eq!(
            serde_json::to_value(ProviderStatus::default()).unwrap(),
            serde_json::json!({})
        );
        let long = "é".repeat(400);
        assert!(clip(&long).len() <= 512);
    }
}

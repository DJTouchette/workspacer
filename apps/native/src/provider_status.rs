//! Live facts a managed provider reports beside a session's state: delivery
//! receipts for the messages we sent, MCP server health, hooks that are
//! running, the CLI's fast mode, and what an interrupt left queued.
//!
//! Every field is only what the provider reported (claudemon
//! `session::provider_status`); nothing here is inferred from what the client
//! asked for, and a field the row omits shows nothing.
use serde_json::Value;

/// Receipts, hooks and servers kept per session (the daemon keeps 32/32/128).
const MAX_RECEIPTS: usize = 64;
const MAX_HOOKS: usize = 32;
const MAX_SERVERS: usize = 128;
const MAX_TEXT: usize = 512;

/// One MCP server's health in Claude's vocabulary: `connected`, `failed`,
/// `needs-auth`, `pending` or `disabled`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub status: String,
    pub error: Option<String>,
}

impl McpServer {
    pub fn failed(&self) -> bool {
        self.status == "failed"
    }
    /// What the status reads as in a list.
    pub fn status_label(&self) -> &str {
        match self.status.as_str() {
            "connected" => "Connected",
            "failed" => "Failed",
            "needs-auth" => "Needs sign-in",
            "pending" => "Connecting…",
            "disabled" => "Disabled",
            other => other,
        }
    }
}

/// A hook that started and has not reported a result yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hook {
    pub id: String,
    pub name: String,
    pub event: String,
    /// Epoch milliseconds.
    pub started_at: Option<i64>,
}

/// What the provider said about one message we sent, by its uuid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Receipt {
    pub uuid: String,
    /// `queued`, `started`, `completed` or `cancelled`.
    pub state: String,
    pub at: Option<i64>,
}

impl Receipt {
    /// The words on the user's bubble.
    pub fn label(&self) -> Option<&'static str> {
        Some(match self.state.as_str() {
            "queued" => "Queued by the agent",
            "started" => "Started",
            "completed" => "Completed",
            "cancelled" => "Cancelled",
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// `None` until the provider first reports; `Some(empty)` is a confirmed
    /// "no servers".
    pub mcp_servers: Option<Vec<McpServer>>,
    pub hooks: Vec<Hook>,
    /// The CLI's own `on`, `off` or `cooldown`.
    pub fast_mode: Option<String>,
    pub fast_mode_disabled_reason: Option<String>,
    pub receipts: Vec<Receipt>,
}

fn text(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| crate::transcript::head(s, MAX_TEXT))
}

fn field<'a>(row: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| row.get(*name))
}

impl Status {
    /// Fold a snapshot row in. A key present replaces its fact (`null`
    /// clears it); a `full` row (one carrying the daemon's always-present
    /// state, so an omitted key means "nothing reported now") also clears
    /// what it omits. Partial rows leave the rest alone.
    pub fn merge(&mut self, row: &Value, full: bool) {
        match field(row, &["mcpServers", "mcp_servers"]) {
            Some(Value::Array(servers)) => {
                self.mcp_servers = Some(
                    servers
                        .iter()
                        .filter_map(|s| {
                            Some(McpServer {
                                name: text(s, &["name"])?,
                                status: text(s, &["status"]).unwrap_or_default(),
                                error: text(s, &["error"]),
                            })
                        })
                        .take(MAX_SERVERS)
                        .collect(),
                )
            }
            Some(_) => self.mcp_servers = None,
            None if full => self.mcp_servers = None,
            None => {}
        }
        match field(row, &["activeHooks", "active_hooks"]) {
            Some(hooks) => {
                self.hooks = hooks
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|h| {
                        Some(Hook {
                            id: text(h, &["id"]).unwrap_or_default(),
                            name: text(h, &["name"]).or_else(|| text(h, &["event"]))?,
                            event: text(h, &["event"]).unwrap_or_default(),
                            started_at: field(h, &["startedAt", "started_at"])
                                .and_then(Value::as_i64),
                        })
                    })
                    .take(MAX_HOOKS)
                    .collect()
            }
            None if full => self.hooks.clear(),
            None => {}
        }
        match field(row, &["fastMode", "fast_mode"]) {
            Some(mode) => self.fast_mode = mode.as_str().map(|m| crate::transcript::head(m, 32)),
            None if full => self.fast_mode = None,
            None => {}
        }
        match field(
            row,
            &["fastModeDisabledReason", "fast_mode_disabled_reason"],
        ) {
            Some(_) => {
                self.fast_mode_disabled_reason = text(
                    row,
                    &["fastModeDisabledReason", "fast_mode_disabled_reason"],
                )
            }
            None if full => self.fast_mode_disabled_reason = None,
            None => {}
        }
        match field(row, &["messageReceipts", "message_receipts"]) {
            Some(receipts) => {
                self.receipts = receipts
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| {
                        Some(Receipt {
                            uuid: text(r, &["uuid"])?,
                            state: text(r, &["state"])?,
                            at: r["at"].as_i64(),
                        })
                    })
                    .take(MAX_RECEIPTS)
                    .collect()
            }
            None if full => self.receipts.clear(),
            None => {}
        }
    }

    /// The latest receipt for a message, by its uuid.
    pub fn receipt(&self, uuid: &str) -> Option<&Receipt> {
        if uuid.is_empty() {
            return None;
        }
        self.receipts.iter().rev().find(|r| r.uuid == uuid)
    }

    pub fn failed_servers(&self) -> usize {
        self.mcp_servers
            .iter()
            .flatten()
            .filter(|s| s.failed())
            .count()
    }

    /// The fast-mode chip's words, when the CLI reported a state.
    pub fn fast_label(&self) -> Option<&'static str> {
        Some(match self.fast_mode.as_deref()? {
            "on" => "Fast",
            "cooldown" => "Fast · cooling down",
            "off" => "Fast off",
            _ => return None,
        })
    }
}

/// `claude.signal` SIGINT's receipt: which of our messages still run after
/// the interrupt, and which it dropped (uuids).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interrupt {
    pub supported: bool,
    pub still_queued: Vec<String>,
    pub cancelled: Vec<String>,
}

impl Interrupt {
    pub fn parse(reply: &Value) -> Option<Self> {
        let receipt = reply.get("interrupt").filter(|r| r.is_object())?;
        let ids = |names: &[&str]| -> Vec<String> {
            field(receipt, names)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(64)
                .map(|s| crate::transcript::head(s, 128))
                .collect()
        };
        Some(Self {
            supported: receipt["supported"].as_bool().unwrap_or(false),
            still_queued: ids(&["stillQueued", "still_queued"]),
            cancelled: ids(&["cancelled"]),
        })
    }

    /// The notice after Stop: which messages will still run, named by their
    /// text when the conversation shows them (`text_of`), else counted.
    pub fn notice(&self, text_of: impl Fn(&str) -> Option<String>) -> String {
        if !self.supported {
            return String::new();
        }
        let quote = |uuid: &String| {
            text_of(uuid)
                .map(|t| format!("“{}”", crate::transcript::head(t.trim(), 60)))
                .unwrap_or_else(|| "a message you sent".into())
        };
        let mut parts = vec!["Stopped.".to_owned()];
        match self.still_queued.len() {
            0 => {}
            1 => parts.push(format!(
                "{} will still run next.",
                quote(&self.still_queued[0])
            )),
            n => parts.push(format!(
                "{n} of your messages will still run: {}.",
                self.still_queued
                    .iter()
                    .take(3)
                    .map(quote)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
        match self.cancelled.len() {
            0 => {}
            1 => parts.push(format!("{} was cancelled.", quote(&self.cancelled[0]))),
            n => parts.push(format!("{n} queued messages were cancelled.")),
        }
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_report_servers_hooks_fast_mode_and_receipts() {
        let mut status = Status::default();
        status.merge(
            &json!({
                "mcpServers":[{"name":"github","status":"connected"},{"name":"db","status":"failed","error":"spawn ENOENT"}],
                "activeHooks":[{"id":"h1","name":"lint-on-save","event":"PostToolUse","startedAt":5}],
                "fastMode":"cooldown","fastModeDisabledReason":"free",
                "messageReceipts":[{"uuid":"u1","state":"queued","at":1},{"uuid":"u1","state":"started","at":2}]
            }),
            true,
        );
        assert_eq!(status.failed_servers(), 1);
        assert_eq!(
            status.mcp_servers.as_ref().unwrap()[1].error.as_deref(),
            Some("spawn ENOENT")
        );
        assert_eq!(status.hooks[0].name, "lint-on-save");
        assert_eq!(status.fast_label(), Some("Fast · cooling down"));
        assert_eq!(status.receipt("u1").unwrap().label(), Some("Started"));
        // A partial row keeps everything; a full one clears what it omits.
        status.merge(&json!({"mode":"input"}), false);
        assert_eq!(status.hooks.len(), 1);
        status.merge(&json!({"compacting":false}), true);
        assert!(status.hooks.is_empty() && status.fast_mode.is_none());
        assert!(status.mcp_servers.is_none() && status.receipts.is_empty());
    }

    #[test]
    fn an_interrupt_names_what_still_runs() {
        let reply =
            json!({"ok":true,"interrupt":{"supported":true,"stillQueued":["u2"],"cancelled":[]}});
        let interrupt = Interrupt::parse(&reply).unwrap();
        let notice = interrupt.notice(|uuid| (uuid == "u2").then(|| "run the tests".to_owned()));
        assert_eq!(notice, "Stopped. “run the tests” will still run next.");
        let two = Interrupt {
            supported: true,
            still_queued: vec!["a".into(), "b".into()],
            cancelled: vec!["c".into(), "d".into()],
        };
        assert_eq!(
            two.notice(|_| None),
            "Stopped. 2 of your messages will still run: a message you sent, a message you sent. 2 queued messages were cancelled."
        );
        assert!(Interrupt::parse(&json!({"ok":true,"interrupt":null})).is_none());
        assert_eq!(
            Interrupt {
                supported: false,
                ..Default::default()
            }
            .notice(|_| None),
            ""
        );
    }
}

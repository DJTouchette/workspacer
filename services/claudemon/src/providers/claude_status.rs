//! Claude stream live status: delivery receipts (`command_lifecycle`,
//! `msg_lifecycle_v1`), MCP server health (`system/init.mcp_servers`, the
//! `mcp_status` control request), running hooks (`system/hook_started` /
//! `hook_response`, with `--include-hook-events`), fast mode
//! (`fast_mode_state`, `apply_flag_settings {fastMode}`) and the interrupt
//! receipt (`interrupt_receipt_v1`).
//!
//! Rides beside the stream driver's state machine, never inside it: the
//! driver hands every stdout frame to [`ClaudeStatus::observe`] first, which
//! records what it reports into the row's
//! [`crate::session::provider_status::ProviderStatus`] and consumes only the
//! responses to its OWN control requests (ids `wks-status-<n>`, a namespace
//! the driver's `wks-<n>` requests never use).
//!
//! Wire shapes, captured live against CLI 2.1.299 (2026-10-14):
//! * `{"type":"command_lifecycle","command_uuid":<our user-frame uuid>,
//!   "state":"queued"|"started"|"completed"|"cancelled","uuid":…,"session_id":…}`
//! * `{"type":"system","subtype":"hook_started","hook_id","hook_name","hook_event",…}`
//!   then `hook_response` with the same `hook_id` plus `exit_code`, `stdout`,
//!   `stderr`, `output` and `outcome: "success"|"error"|"cancelled"`.
//! * `mcp_status` → `{"mcpServers":[{name,status,config,scope,source,error?,…}]}`;
//!   the init frame's `mcp_servers` is `[{name,status,source}]`.
//! * `interrupt` → `{"still_queued":[uuid…]}` (`cancelled` only with
//!   `cancel_queued`, which is never sent here).
//! * `apply_flag_settings {"settings":{"fastMode":true}}` → plain success. On
//!   haiku the next init still reported `fast_mode_state: "off"` (the reason
//!   moved from `sdk_opt_in_required` to none): success is acceptance, not
//!   "fast mode is on".
//! * `mcp_reconnect {"serverName":"nope"}` → error `Server not found: nope`.

use std::collections::HashMap;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use super::claude_stream::CliProtocol;
use crate::session::conversation::ConversationItem;
use crate::session::provider_status::{
    clip, ActiveHook, InterruptReceipt, McpServer, ProviderStatus, StatusControl,
};
use crate::session::{ConversationStore, SessionStore};

/// Request-id prefix of the status requests this module sends.
const ID_PREFIX: &str = "wks-status-";

/// The first CLI known to stream hook lifecycle frames live (they existed
/// from 2.1.169, but arrived in one batch after a SessionStart/Setup hook
/// until 2.1.204 restored live delivery — Agent SDK reference,
/// `SDKHookStartedMessage`). Older CLIs get no `--include-hook-events`.
pub(super) const HOOK_EVENTS_MIN_VERSION: (u32, u32, u32) = (2, 1, 204);

/// What a status request's eventual `control_response` resolves.
#[derive(Debug)]
enum Waiting {
    /// Every caller waiting on the one `mcp_status` in flight (empty for
    /// the driver's own refresh after a reconnect).
    McpStatus(Vec<oneshot::Sender<Result<Vec<McpServer>, String>>>),
    McpReconnect(oneshot::Sender<Result<(), String>>),
    FastMode(oneshot::Sender<Result<(), String>>),
    Interrupt(oneshot::Sender<Result<InterruptReceipt, String>>),
}

/// One stream session's status-request bookkeeping.
#[derive(Debug, Default)]
pub(super) struct ClaudeStatus {
    next: u64,
    waiting: HashMap<String, Waiting>,
}

impl ClaudeStatus {
    fn send(&mut self, out_tx: &mpsc::UnboundedSender<Value>, request: Value, waiting: Waiting) {
        self.next += 1;
        let request_id = format!("{ID_PREFIX}{}", self.next);
        self.waiting.insert(request_id.clone(), waiting);
        let _ = out_tx.send(json!({
            "type": "control_request", "request_id": request_id, "request": request
        }));
    }

    /// Write the control request `control` asks for.
    pub(super) fn request(
        &mut self,
        out_tx: &mpsc::UnboundedSender<Value>,
        control: StatusControl,
    ) {
        match control {
            // One `mcp_status` in flight per session: a caller arriving while
            // one is outstanding shares its answer.
            StatusControl::McpStatus { reply } => {
                let in_flight = self.waiting.values_mut().find_map(|w| match w {
                    Waiting::McpStatus(replies) => Some(replies),
                    _ => None,
                });
                match in_flight {
                    Some(replies) => replies.push(reply),
                    None => self.send(
                        out_tx,
                        json!({ "subtype": "mcp_status" }),
                        Waiting::McpStatus(vec![reply]),
                    ),
                }
            }
            StatusControl::McpReconnect { server, reply } => self.send(
                out_tx,
                json!({ "subtype": "mcp_reconnect", "serverName": server }),
                Waiting::McpReconnect(reply),
            ),
            StatusControl::FastMode { enabled, reply } => self.send(
                out_tx,
                json!({ "subtype": "apply_flag_settings", "settings": { "fastMode": enabled } }),
                Waiting::FastMode(reply),
            ),
            StatusControl::Interrupt { reply } => self.send(
                out_tx,
                json!({ "subtype": "interrupt" }),
                Waiting::Interrupt(reply),
            ),
        }
    }

    /// Record what one stdout frame says about the session's live status.
    /// Returns true when the frame was the answer to one of our own status
    /// requests (the driver must not see it); every other frame continues to
    /// the driver unchanged.
    pub(super) fn observe(
        &mut self,
        value: &Value,
        store: &SessionStore,
        conv: &ConversationStore,
        session_id: &str,
        protocol: &CliProtocol,
        out_tx: &mpsc::UnboundedSender<Value>,
    ) -> bool {
        let now = now_ms();
        match frame_status(value, protocol) {
            Some(StatusFrame::Receipt { uuid, state }) => {
                store.update_provider_status(session_id, |s| s.record_receipt(&uuid, &state, now));
            }
            Some(StatusFrame::Init { servers, fast_mode }) => {
                store.update_provider_status(session_id, |s| {
                    let mut changed = false;
                    if let Some(servers) = servers {
                        changed |= s.set_mcp_servers(servers);
                    }
                    if let Some((state, reason)) = &fast_mode {
                        changed |= s.set_fast_mode(state, reason.as_deref());
                    }
                    changed
                });
            }
            Some(StatusFrame::FastMode { state, reason }) => {
                store.update_provider_status(session_id, |s| {
                    s.set_fast_mode(&state, reason.as_deref())
                });
            }
            Some(StatusFrame::TurnEnded { fast_mode }) => {
                store.update_provider_status(session_id, |s| {
                    let mut changed = !s.active_hooks.is_empty();
                    s.active_hooks.clear();
                    if let Some((state, reason)) = &fast_mode {
                        changed |= s.set_fast_mode(state, reason.as_deref());
                    }
                    changed
                });
            }
            Some(StatusFrame::HookStarted(hook)) => {
                store.update_provider_status(session_id, |s| s.hook_started(hook));
            }
            Some(StatusFrame::HookFinished { id, failure }) => {
                store.update_provider_status(session_id, |s| s.hook_finished(&id));
                if let Some(text) = failure {
                    push_notice(conv, session_id, text);
                }
            }
            Some(StatusFrame::Response { request_id }) => {
                return self.answered(
                    &request_id,
                    value,
                    store,
                    conv,
                    session_id,
                    protocol,
                    out_tx,
                );
            }
            None => {}
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn answered(
        &mut self,
        request_id: &str,
        value: &Value,
        store: &SessionStore,
        conv: &ConversationStore,
        session_id: &str,
        protocol: &CliProtocol,
        out_tx: &mpsc::UnboundedSender<Value>,
    ) -> bool {
        let Some(waiting) = self.waiting.remove(request_id) else {
            // Ours by namespace but unknown (a duplicate): still not the
            // driver's to read.
            return request_id.starts_with(ID_PREFIX);
        };
        let response = value.get("response").unwrap_or(&Value::Null);
        let body = response.get("response");
        let error =
            (response.get("subtype").and_then(Value::as_str) != Some("success")).then(|| {
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("control request failed")
                    .to_string()
            });
        match (waiting, error) {
            (Waiting::McpStatus(reply), None) => {
                let servers = body
                    .map(|b| b.get("mcpServers").unwrap_or(b))
                    .map(mcp_servers_from)
                    .unwrap_or_default();
                store.update_provider_status(session_id, |s| s.set_mcp_servers(servers.clone()));
                for reply in reply {
                    let _ = reply.send(Ok(servers.clone()));
                }
            }
            (Waiting::McpStatus(reply), Some(error)) => {
                for reply in reply {
                    let _ = reply.send(Err(error.clone()));
                }
            }
            (Waiting::McpReconnect(reply), error) => {
                let ok = error.is_none();
                let _ = reply.send(error.map_or(Ok(()), Err));
                if ok {
                    // The reconnect changed a server's status: re-read them
                    // (unless a read is already on its way).
                    if !self
                        .waiting
                        .values()
                        .any(|w| matches!(w, Waiting::McpStatus(_)))
                    {
                        self.send(
                            out_tx,
                            json!({ "subtype": "mcp_status" }),
                            Waiting::McpStatus(Vec::new()),
                        );
                    }
                }
            }
            (Waiting::FastMode(reply), error) => {
                let _ = reply.send(error.map_or(Ok(()), Err));
            }
            (Waiting::Interrupt(reply), Some(error)) => {
                tracing::warn!(session = %session_id, error = %error, "claude stream: interrupt failed");
                let _ = reply.send(Err(error));
            }
            (Waiting::Interrupt(reply), None) => {
                let receipt = interrupt_receipt(body, protocol);
                if let Some(text) = interrupt_notice(&receipt) {
                    push_notice(conv, session_id, text);
                }
                let _ = reply.send(Ok(receipt));
            }
        }
        true
    }
}

fn push_notice(conv: &ConversationStore, session_id: &str, text: String) {
    conv.push(
        session_id,
        vec![ConversationItem::AssistantNotice {
            text,
            notice: true,
            timestamp: None,
        }],
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What one frame reports, if anything.
#[derive(Debug, PartialEq)]
enum StatusFrame {
    Receipt {
        uuid: String,
        state: String,
    },
    Init {
        servers: Option<Vec<McpServer>>,
        fast_mode: Option<(String, Option<String>)>,
    },
    FastMode {
        state: String,
        reason: Option<String>,
    },
    /// A main-thread `result`, with its `fast_mode_state` if it carried one.
    TurnEnded {
        fast_mode: Option<(String, Option<String>)>,
    },
    HookStarted(ActiveHook),
    HookFinished {
        id: String,
        failure: Option<String>,
    },
    /// A `control_response` to one of our status requests.
    Response {
        request_id: String,
    },
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// A reported `fast_mode_state` (`on` | `off` | `cooldown`) with its reason.
fn fast_mode_of(value: &Value) -> Option<(String, Option<String>)> {
    let state = text(value, "fast_mode_state")?;
    matches!(state, "on" | "off" | "cooldown").then(|| {
        (
            state.to_owned(),
            text(value, "fast_mode_disabled_reason").map(str::to_owned),
        )
    })
}

fn frame_status(value: &Value, protocol: &CliProtocol) -> Option<StatusFrame> {
    match text(value, "type")? {
        // Only receipts for messages WE sent: the CLI also enqueues its own
        // (task notifications, scheduled triggers).
        "command_lifecycle" => {
            let uuid = text(value, "command_uuid")?;
            let state = text(value, "state")?;
            protocol.sent_by_us(uuid).then(|| StatusFrame::Receipt {
                uuid: uuid.to_owned(),
                state: state.to_owned(),
            })
        }
        "system" => match text(value, "subtype")? {
            // Every turn's init: the servers' current status and fast mode.
            // A subagent's init is not the session's.
            "init" if value.get("parent_tool_use_id").is_none_or(Value::is_null) => {
                Some(StatusFrame::Init {
                    servers: value.get("mcp_servers").map(mcp_servers_from),
                    fast_mode: fast_mode_of(value),
                })
            }
            "hook_started" => Some(StatusFrame::HookStarted(ActiveHook {
                id: clip(text(value, "hook_id")?),
                name: clip(text(value, "hook_name").unwrap_or("hook")),
                event: clip(text(value, "hook_event").unwrap_or("")),
                started_at: now_ms(),
            })),
            "hook_response" => Some(StatusFrame::HookFinished {
                id: text(value, "hook_id")?.to_owned(),
                failure: hook_failure(value),
            }),
            _ => None,
        },
        // The turn is over: no hook of it is still running, whatever its
        // response said (or failed to say).
        "result" if value.get("parent_tool_use_id").is_none_or(Value::is_null) => {
            Some(StatusFrame::TurnEnded {
                fast_mode: fast_mode_of(value),
            })
        }
        "control_response" => {
            let response = value.get("response")?;
            let request_id = text(response, "request_id")?;
            if request_id.starts_with(ID_PREFIX) {
                return Some(StatusFrame::Response {
                    request_id: request_id.to_owned(),
                });
            }
            // The driver's own `initialize` answer carries fast mode too.
            response
                .get("response")
                .and_then(fast_mode_of)
                .map(|(state, reason)| StatusFrame::FastMode { state, reason })
        }
        _ => None,
    }
}

/// The notice a failed hook earns: `outcome: "error"` only (a cancelled hook
/// was stopped on purpose), with the exit code and the first stderr line.
fn hook_failure(value: &Value) -> Option<String> {
    if text(value, "outcome") != Some("error") {
        return None;
    }
    let name = text(value, "hook_name").unwrap_or("hook");
    let event = text(value, "hook_event").filter(|e| *e != name);
    let mut out = format!("Hook {name}");
    if let Some(event) = event {
        out.push_str(&format!(" ({event})"));
    }
    out.push_str(" failed");
    if let Some(code) = value.get("exit_code").and_then(Value::as_i64) {
        out.push_str(&format!(" with exit code {code}"));
    }
    let detail = text(value, "stderr")
        .or_else(|| text(value, "output"))
        .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty()));
    if let Some(detail) = detail {
        out.push_str(": ");
        out.push_str(&clip(detail));
    }
    Some(out)
}

/// `[{name, status, error?}]` (init frame, `mcp_status`), tolerating the
/// older name-keyed map shape.
pub(super) fn mcp_servers_from(list: &Value) -> Vec<McpServer> {
    let one = |name: &str, entry: &Value| McpServer {
        name: clip(name),
        status: clip(
            entry
                .get("status")
                .and_then(Value::as_str)
                .or_else(|| entry.as_str())
                .unwrap_or("pending"),
        ),
        error: text(entry, "error").map(clip),
    };
    match list {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| Some(one(text(item, "name")?, item)))
            .collect(),
        Value::Object(map) => map.iter().map(|(name, entry)| one(name, entry)).collect(),
        _ => Vec::new(),
    }
}

/// The receipt an interrupt's success carries, our uuids only.
fn interrupt_receipt(body: Option<&Value>, protocol: &CliProtocol) -> InterruptReceipt {
    InterruptReceipt {
        supported: protocol.has_capability("interrupt_receipt_v1")
            || body.is_some_and(|b| b.get("still_queued").is_some()),
        still_queued: protocol.ours(body.and_then(|b| b.get("still_queued"))),
        cancelled: protocol.ours(body.and_then(|b| b.get("cancelled"))),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        "message"
    } else {
        "messages"
    }
}

/// A notice only when the interrupt left OUR messages pending: they run
/// after it (or were cancelled), which the user would otherwise not know.
fn interrupt_notice(receipt: &InterruptReceipt) -> Option<String> {
    let (queued, cancelled) = (receipt.still_queued.len(), receipt.cancelled.len());
    match (queued, cancelled) {
        (0, 0) => None,
        (q, 0) => Some(format!(
            "Interrupted. {q} queued {} will still run.",
            plural(q)
        )),
        (0, c) => Some(format!(
            "Interrupted. {c} queued {} cancelled and will not run.",
            if c == 1 {
                "message was"
            } else {
                "messages were"
            }
        )),
        (q, c) => Some(format!(
            "Interrupted. {q} queued {} will still run; {c} {} cancelled.",
            plural(q),
            if c == 1 { "was" } else { "were" }
        )),
    }
}

/// `--include-hook-events` when the CLI (its version from the stream
/// driver's one per-spawn `cli_version` probe) streams hook lifecycle frames
/// live — see [`HOOK_EVENTS_MIN_VERSION`] — and the spawn's own args do not
/// already ask for them. An unknown version (probe failed) gets nothing: an
/// older CLI would refuse the flag.
pub(super) fn hook_events_args(version: Option<(u32, u32, u32)>, argv: &[String]) -> Vec<String> {
    if argv.iter().any(|a| a == "--include-hook-events")
        || version.is_none_or(|v| v < HOOK_EVENTS_MIN_VERSION)
    {
        return Vec::new();
    }
    vec!["--include-hook-events".into()]
}

/// The status a fresh driver life starts from: nothing from a previous life's
/// hooks is still running.
pub(super) fn reset(store: &SessionStore, session_id: &str) {
    store.update_provider_status(session_id, |s: &mut ProviderStatus| {
        let had = !s.active_hooks.is_empty();
        s.end();
        had
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol_with(sent: &[&str], caps: &[&str]) -> CliProtocol {
        let mut protocol = CliProtocol::default();
        let caps: Vec<Value> = caps.iter().map(|c| json!(c)).collect();
        protocol.observe_init(&json!({ "claude_code_version": "2.1.299", "capabilities": caps }));
        for uuid in sent {
            protocol.remember_sent((*uuid).to_owned());
        }
        protocol
    }

    #[test]
    fn lifecycle_frames_become_receipts_for_our_uuids_only() {
        let protocol = protocol_with(&["u1"], &[]);
        let frame = |uuid: &str, state: &str| json!({"type":"command_lifecycle","command_uuid":uuid,"state":state,"uuid":"x","session_id":"s"});
        assert_eq!(
            frame_status(&frame("u1", "started"), &protocol),
            Some(StatusFrame::Receipt {
                uuid: "u1".into(),
                state: "started".into()
            })
        );
        assert_eq!(frame_status(&frame("cli-own", "queued"), &protocol), None);
    }

    #[test]
    fn init_reports_servers_and_fast_mode() {
        let protocol = CliProtocol::default();
        let init = json!({"type":"system","subtype":"init",
            "mcp_servers":[{"name":"a","status":"pending","source":"claudeai"},{"name":"b","status":"needs-auth"}],
            "fast_mode_state":"off","fast_mode_disabled_reason":"sdk_opt_in_required"});
        let Some(StatusFrame::Init { servers, fast_mode }) = frame_status(&init, &protocol) else {
            panic!("init is a status frame");
        };
        let servers = servers.unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[1].status, "needs-auth");
        assert_eq!(
            fast_mode,
            Some(("off".into(), Some("sdk_opt_in_required".into())))
        );
        // An unknown fast-mode word is not a confirmed state.
        let odd = json!({"type":"result","fast_mode_state":"turbo"});
        assert_eq!(
            frame_status(&odd, &protocol),
            Some(StatusFrame::TurnEnded { fast_mode: None })
        );
        let result = json!({"type":"result","fast_mode_state":"cooldown"});
        assert_eq!(
            frame_status(&result, &protocol),
            Some(StatusFrame::TurnEnded {
                fast_mode: Some(("cooldown".into(), None))
            })
        );
        // A subagent's result ends nothing of the session's.
        let sub = json!({"type":"result","parent_tool_use_id":"tu","fast_mode_state":"on"});
        assert_eq!(frame_status(&sub, &protocol), None);
        // The driver's initialize answer carries it as well.
        let init_reply = json!({"type":"control_response","response":{"subtype":"success",
            "request_id":"wks-1","response":{"fast_mode_state":"on"}}});
        assert_eq!(
            frame_status(&init_reply, &protocol),
            Some(StatusFrame::FastMode {
                state: "on".into(),
                reason: None
            })
        );
    }

    #[test]
    fn hooks_start_finish_and_only_errors_earn_a_notice() {
        let protocol = CliProtocol::default();
        let started = json!({"type":"system","subtype":"hook_started","hook_id":"h1",
            "hook_name":"Stop","hook_event":"Stop","uuid":"x","session_id":"s"});
        match frame_status(&started, &protocol) {
            Some(StatusFrame::HookStarted(hook)) => {
                assert_eq!(
                    (hook.id.as_str(), hook.name.as_str(), hook.event.as_str()),
                    ("h1", "Stop", "Stop")
                );
            }
            other => panic!("{other:?}"),
        }
        // Captured live: a Stop hook exiting 1.
        let failed = json!({"type":"system","subtype":"hook_response","hook_id":"h1",
            "hook_name":"Stop","hook_event":"Stop","output":"stop-fail\n","stdout":"",
            "stderr":"stop-fail\n","exit_code":1,"outcome":"error"});
        assert_eq!(
            frame_status(&failed, &protocol),
            Some(StatusFrame::HookFinished {
                id: "h1".into(),
                failure: Some("Hook Stop failed with exit code 1: stop-fail".into())
            })
        );
        let ok = json!({"type":"system","subtype":"hook_response","hook_id":"h2",
            "hook_name":"PreToolUse:Bash","hook_event":"PreToolUse","exit_code":0,"outcome":"success"});
        assert_eq!(
            frame_status(&ok, &protocol),
            Some(StatusFrame::HookFinished {
                id: "h2".into(),
                failure: None
            })
        );
        let named =
            json!({"hook_name":"PreToolUse:Bash","hook_event":"PreToolUse","outcome":"error"});
        assert_eq!(
            hook_failure(&named).unwrap(),
            "Hook PreToolUse:Bash (PreToolUse) failed"
        );
        let cancelled = json!({"hook_name":"x","outcome":"cancelled"});
        assert_eq!(hook_failure(&cancelled), None);
        // Progress frames are not status.
        let progress = json!({"type":"system","subtype":"hook_progress","hook_id":"h1"});
        assert_eq!(frame_status(&progress, &protocol), None);
    }

    #[test]
    fn mcp_status_reply_and_legacy_map_shapes_parse() {
        let reply = json!([{"name":"claude.ai Linear","status":"failed","error":"boom","config":{"type":"claudeai-proxy"}}]);
        assert_eq!(
            mcp_servers_from(&reply),
            vec![McpServer {
                name: "claude.ai Linear".into(),
                status: "failed".into(),
                error: Some("boom".into())
            }]
        );
        let legacy = json!({"workspacer": {"status": "connected"}, "other": "pending"});
        let servers = mcp_servers_from(&legacy);
        assert_eq!(servers.len(), 2);
        assert!(servers
            .iter()
            .any(|s| s.name == "workspacer" && s.status == "connected"));
        assert!(servers
            .iter()
            .any(|s| s.name == "other" && s.status == "pending"));
    }

    #[test]
    fn interrupt_receipts_name_our_messages_and_notice_only_when_pending() {
        let protocol = protocol_with(&["mine-1", "mine-2"], &["interrupt_receipt_v1"]);
        let body = json!({"still_queued":["mine-1","cli-own"],"cancelled":["mine-2"]});
        let receipt = interrupt_receipt(Some(&body), &protocol);
        assert_eq!(
            receipt,
            InterruptReceipt {
                supported: true,
                still_queued: vec!["mine-1".into()],
                cancelled: vec!["mine-2".into()],
            }
        );
        assert_eq!(
            interrupt_notice(&receipt).unwrap(),
            "Interrupted. 1 queued message will still run; 1 was cancelled."
        );
        // Captured live: nothing pending.
        let empty = interrupt_receipt(Some(&json!({"still_queued":[]})), &protocol);
        assert!(empty.supported && interrupt_notice(&empty).is_none());
        // An older CLI answers with no receipt at all.
        let old = interrupt_receipt(None, &CliProtocol::default());
        assert!(!old.supported);
        assert!(interrupt_notice(&old).is_none());
        let two = InterruptReceipt {
            supported: true,
            still_queued: vec!["a".into(), "b".into()],
            cancelled: vec![],
        };
        assert_eq!(
            interrupt_notice(&two).unwrap(),
            "Interrupted. 2 queued messages will still run."
        );
    }

    #[tokio::test]
    async fn requests_use_their_own_namespace_and_answers_are_consumed() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed("s1", "/tmp", "claude");
        let protocol = protocol_with(&["mine"], &["interrupt_receipt_v1"]);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let mut status = ClaudeStatus::default();

        let (reply, verdict) = oneshot::channel();
        status.request(&out_tx, StatusControl::McpStatus { reply });
        let sent = out_rx.try_recv().unwrap();
        assert_eq!(sent["request"]["subtype"], "mcp_status");
        let id = sent["request_id"].as_str().unwrap().to_owned();
        assert!(id.starts_with(ID_PREFIX));
        let answer = json!({"type":"control_response","response":{"subtype":"success","request_id":id,
            "response":{"mcpServers":[{"name":"w","status":"connected"}]}}});
        assert!(status.observe(&answer, &store, &conv, "s1", &protocol, &out_tx));
        assert_eq!(verdict.await.unwrap().unwrap()[0].name, "w");
        assert_eq!(
            store
                .get("s1")
                .unwrap()
                .provider_status
                .mcp_servers
                .unwrap()[0]
                .status,
            "connected"
        );

        // A reconnect re-reads the servers on success.
        let (reply, verdict) = oneshot::channel();
        status.request(
            &out_tx,
            StatusControl::McpReconnect {
                server: "w".into(),
                reply,
            },
        );
        let sent = out_rx.try_recv().unwrap();
        assert_eq!(
            sent["request"],
            json!({"subtype":"mcp_reconnect","serverName":"w"})
        );
        let id = sent["request_id"].as_str().unwrap().to_owned();
        let ok =
            json!({"type":"control_response","response":{"subtype":"success","request_id":id}});
        assert!(status.observe(&ok, &store, &conv, "s1", &protocol, &out_tx));
        verdict.await.unwrap().unwrap();
        assert_eq!(
            out_rx.try_recv().unwrap()["request"]["subtype"],
            "mcp_status"
        );

        // Fast mode: the CLI's refusal is the caller's answer.
        let (reply, verdict) = oneshot::channel();
        status.request(
            &out_tx,
            StatusControl::FastMode {
                enabled: true,
                reply,
            },
        );
        let sent = out_rx.try_recv().unwrap();
        assert_eq!(
            sent["request"],
            json!({"subtype":"apply_flag_settings","settings":{"fastMode":true}})
        );
        let id = sent["request_id"].as_str().unwrap().to_owned();
        let refused = json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"nope"}});
        assert!(status.observe(&refused, &store, &conv, "s1", &protocol, &out_tx));
        assert_eq!(verdict.await.unwrap(), Err("nope".into()));

        // Interrupt: the receipt answers the caller and leaves a notice.
        let (reply, verdict) = oneshot::channel();
        status.request(&out_tx, StatusControl::Interrupt { reply });
        let sent = out_rx.try_recv().unwrap();
        let id = sent["request_id"].as_str().unwrap().to_owned();
        let receipt = json!({"type":"control_response","response":{"subtype":"success","request_id":id,
            "response":{"still_queued":["mine"]}}});
        assert!(status.observe(&receipt, &store, &conv, "s1", &protocol, &out_tx));
        assert_eq!(
            verdict.await.unwrap().unwrap().still_queued,
            vec!["mine".to_string()]
        );
        let (_, items) = conv.snapshot("s1").unwrap();
        assert!(items.iter().any(|item| matches!(item,
            ConversationItem::AssistantNotice { text, .. } if text.contains("will still run"))));

        // The driver's own responses and other frames pass through.
        let theirs = json!({"type":"control_response","response":{"subtype":"success","request_id":"wks-1"}});
        assert!(!status.observe(&theirs, &store, &conv, "s1", &protocol, &out_tx));
        let lifecycle = json!({"type":"command_lifecycle","command_uuid":"mine","state":"started"});
        assert!(!status.observe(&lifecycle, &store, &conv, "s1", &protocol, &out_tx));
        let receipts = store.get("s1").unwrap().provider_status.message_receipts;
        assert_eq!(receipts.last().unwrap().state, "started");

        // A failed hook: tracked while running, a notice when it fails.
        let started = json!({"type":"system","subtype":"hook_started","hook_id":"h","hook_name":"Stop","hook_event":"Stop"});
        status.observe(&started, &store, &conv, "s1", &protocol, &out_tx);
        assert_eq!(
            store.get("s1").unwrap().provider_status.active_hooks.len(),
            1
        );
        let failed = json!({"type":"system","subtype":"hook_response","hook_id":"h","hook_name":"Stop","hook_event":"Stop","outcome":"error","exit_code":2});
        status.observe(&failed, &store, &conv, "s1", &protocol, &out_tx);
        assert!(store
            .get("s1")
            .unwrap()
            .provider_status
            .active_hooks
            .is_empty());
        assert!(conv.snapshot("s1").unwrap().1.iter().any(|item| matches!(item,
            ConversationItem::AssistantNotice { text, .. } if text == "Hook Stop failed with exit code 2")));
    }

    #[cfg(unix)]
    struct Scratch(std::path::PathBuf);
    #[cfg(unix)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A stand-in `claude`: `--version` prints `version`; otherwise it logs
    /// argv and stdin, answers every control request (an interrupt with a
    /// receipt naming the last user uuid it read), and reports a `started`
    /// lifecycle for each user frame.
    #[cfg(unix)]
    fn stub(dir: &std::path::Path, version: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("claude");
        let log = dir.join("stdin.jsonl");
        let argv = dir.join("argv.txt");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo '{version} (Claude Code)'; exit 0; fi
printf '%s\n' "$@" > '{argv}'
seen=""
while IFS= read -r line; do
  printf '%s\n' "$line" >> '{log}'
  case "$line" in
    *'"type":"user"'*)
      u=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
      seen="\"$u\""
      printf '%s\n' "{{\"type\":\"command_lifecycle\",\"command_uuid\":\"$u\",\"state\":\"started\"}}"
      ;;
    *'"subtype":"interrupt"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
      printf '%s\n' "{{\"type\":\"control_response\",\"response\":{{\"subtype\":\"success\",\"request_id\":\"$id\",\"response\":{{\"still_queued\":[$seen]}}}}}}"
      ;;
    *'"type":"control_request"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
      printf '%s\n' "{{\"type\":\"control_response\",\"response\":{{\"subtype\":\"success\",\"request_id\":\"$id\",\"response\":{{\"mcpServers\":[{{\"name\":\"w\",\"status\":\"connected\"}}]}}}}}}"
      ;;
  esac
done
"#,
            log = log.display(),
            argv = argv.display(),
        );
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(unix)]
    async fn eventually(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..600 {
            if cond() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }

    #[cfg(unix)]
    fn scratch(tag: &str) -> Scratch {
        let dir =
            Scratch(std::env::temp_dir().join(format!("claudemon-{tag}-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(&dir.0).unwrap();
        dir
    }

    #[test]
    fn hook_events_flag_follows_the_cli_version() {
        let none: Vec<String> = Vec::new();
        assert!(hook_events_args(Some((2, 1, 203)), &none).is_empty());
        assert_eq!(
            hook_events_args(Some((2, 1, 204)), &none),
            vec!["--include-hook-events"]
        );
        assert_eq!(
            hook_events_args(Some((2, 1, 299)), &none),
            vec!["--include-hook-events"]
        );
        // A failed probe is not a reason to risk an unknown flag.
        assert!(hook_events_args(None, &none).is_empty());
        // The spawn's own args already ask: never twice.
        let asked = vec!["--include-hook-events".to_string()];
        assert!(hook_events_args(Some((2, 1, 299)), &asked).is_empty());
    }

    /// The whole path through a live driver: the caller's uuid on the frame,
    /// its receipt on the row, the status controls, and the interrupt receipt.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_live_driver_stamps_uuids_records_receipts_and_answers_status_controls() {
        use super::super::claude_stream::{spawn_session, SpawnConfig};
        use crate::session::{Delivery, MessageSender};
        let dir = scratch("live-status");
        let bin = stub(&dir.0, "2.1.299");
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let sid = "live-status";
        store.register_managed(sid, &dir.0.to_string_lossy(), "claude");
        store.set_transport(sid, crate::session::state::Transport::Stream);
        spawn_session(
            store.clone(),
            conv.clone(),
            SpawnConfig {
                session_id: sid.into(),
                cwd: dir.0.to_string_lossy().to_string(),
                bin: bin.to_string_lossy().to_string(),
                model: None,
                effort: None,
                permission_mode: None,
                resume: None,
                fork_session: false,
                resume_at: None,
                extra_args: vec![],
                env: HashMap::new(),
                yolo: false,
                facade: super::super::Facade::default(),
            },
        );
        assert!(eventually(|| store.has_status_control(sid)).await);
        assert!(eventually(|| dir.0.join("argv.txt").exists()).await);
        let argv = std::fs::read_to_string(dir.0.join("argv.txt")).unwrap_or_default();
        // Version-gated flags sit right after the fixed head, by name.
        let lines: Vec<&str> = argv.lines().collect();
        let replay = lines
            .iter()
            .position(|a| *a == "--replay-user-messages")
            .expect("replay flag");
        let hooks: Vec<usize> = (0..lines.len())
            .filter(|&i| lines[i] == "--include-hook-events")
            .collect();
        let session = lines
            .iter()
            .position(|a| *a == "--session-id")
            .expect("session id");
        assert_eq!(hooks.len(), 1, "{argv}");
        assert!(replay < hooks[0] && hooks[0] < session, "{argv}");

        let uuid = "6f1c0d52-0000-4000-8000-000000000001";
        assert!(matches!(
            store.submit_message_tracked(
                sid,
                "hi".into(),
                Delivery::Steer,
                MessageSender::Human,
                Some(uuid.into())
            ),
            crate::session::MessageOutcome::Sent
        ));
        assert!(
            eventually(|| store.get(sid).is_some_and(|s| s
                .provider_status
                .message_receipts
                .iter()
                .any(|r| r.uuid == uuid && r.delivered())))
            .await,
            "the started receipt lands on the row"
        );
        let log = std::fs::read_to_string(dir.0.join("stdin.jsonl")).unwrap();
        assert!(log.contains(&format!("\"uuid\":\"{uuid}\"")), "{log}");
        // An idle send: the hub's uuid is also the echoed user message's,
        // the id rewind and fork name it by.
        let echoed = conv
            .snapshot(sid)
            .map(|(_, items)| items)
            .unwrap_or_default()
            .into_iter()
            .find_map(|i| match i {
                crate::session::conversation::ConversationItem::UserMessage { uuid, .. } => uuid,
                _ => None,
            });
        assert_eq!(
            echoed.as_deref(),
            Some(uuid),
            "the echo carries the hub's uuid"
        );

        let (reply, verdict) = oneshot::channel();
        store
            .submit_status_control(sid, StatusControl::McpStatus { reply })
            .unwrap();
        assert_eq!(verdict.await.unwrap().unwrap()[0].name, "w");
        let (reply, verdict) = oneshot::channel();
        store
            .submit_status_control(
                sid,
                StatusControl::FastMode {
                    enabled: true,
                    reply,
                },
            )
            .unwrap();
        verdict.await.unwrap().unwrap();
        assert!(std::fs::read_to_string(dir.0.join("stdin.jsonl"))
            .unwrap()
            .contains(r#""fastMode":true"#));

        let (reply, verdict) = oneshot::channel();
        store
            .submit_status_control(sid, StatusControl::Interrupt { reply })
            .unwrap();
        let receipt = verdict.await.unwrap().unwrap();
        assert_eq!(receipt.still_queued, vec![uuid.to_string()]);

        store.terminate_managed(sid);
        assert!(eventually(|| !store.has_status_control(sid)).await);
    }

    #[tokio::test]
    async fn concurrent_mcp_status_requests_share_one_in_flight_read() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed("s1", "/tmp", "claude");
        let protocol = CliProtocol::default();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let mut status = ClaudeStatus::default();
        let (first, first_verdict) = oneshot::channel();
        let (second, second_verdict) = oneshot::channel();
        status.request(&out_tx, StatusControl::McpStatus { reply: first });
        status.request(&out_tx, StatusControl::McpStatus { reply: second });
        let sent = out_rx.try_recv().unwrap();
        assert!(
            out_rx.try_recv().is_err(),
            "the second caller sends nothing"
        );
        let id = sent["request_id"].as_str().unwrap().to_owned();
        let answer = json!({"type":"control_response","response":{"subtype":"success","request_id":id,
            "response":{"mcpServers":[{"name":"w","status":"connected"}]}}});
        assert!(status.observe(&answer, &store, &conv, "s1", &protocol, &out_tx));
        assert_eq!(first_verdict.await.unwrap().unwrap()[0].name, "w");
        assert_eq!(second_verdict.await.unwrap().unwrap()[0].name, "w");
        // Answered: the next caller starts a fresh read.
        let (third, _verdict) = oneshot::channel();
        status.request(&out_tx, StatusControl::McpStatus { reply: third });
        assert!(out_rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn a_turn_end_clears_hooks_that_never_responded() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        store.register_managed("s1", "/tmp", "claude");
        let protocol = CliProtocol::default();
        let (out_tx, _out_rx) = mpsc::unbounded_channel();
        let mut status = ClaudeStatus::default();
        let started = json!({"type":"system","subtype":"hook_started","hook_id":"h","hook_name":"Stop","hook_event":"Stop"});
        status.observe(&started, &store, &conv, "s1", &protocol, &out_tx);
        assert_eq!(
            store.get("s1").unwrap().provider_status.active_hooks.len(),
            1
        );
        let result = json!({"type":"result","subtype":"success","fast_mode_state":"off"});
        assert!(!status.observe(&result, &store, &conv, "s1", &protocol, &out_tx));
        let row = store.get("s1").unwrap();
        assert!(row.provider_status.active_hooks.is_empty());
        assert_eq!(row.provider_status.fast_mode.as_deref(), Some("off"));
    }
}

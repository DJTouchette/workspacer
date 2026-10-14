//! Codex app-server live status: MCP server health and running hooks, kept in
//! the row's [`crate::session::provider_status::ProviderStatus`] the same way
//! the Claude stream driver keeps them (see `claude_status`).
//!
//! The driver hands every JSON-RPC message to [`CodexStatus::observe`] first;
//! it consumes only the responses to its own requests (string ids
//! `wks-status-<n>`, which never collide with the driver's numeric ids or its
//! `workspacer-skill-roots` request).
//!
//! Wire shapes (observed live on codex-cli 0.153.4, 2026-10-14, plus the
//! generated `v2` schema):
//! * `mcpServer/startupStatus/updated {threadId, name, status:
//!   starting|ready|failed|cancelled, error, failureReason:
//!   reauthenticationRequired|null}` — per server, at thread start and after a
//!   reload.
//! * `mcpServerStatus/list {threadId, detail, limit}` →
//!   `{data:[{name, runtimeStatus, authStatus, tools, …}], nextCursor}`.
//!   `runtimeStatus` (`notStarted|starting|connected|authenticationRequired|
//!   failed|cancelled|disabled`) is null without `threadId`, and carries no
//!   error text. The reply waits for servers still starting.
//! * `config/mcpServer/reload` (no params) → `{}`, then fresh startup
//!   notifications. Codex has no per-server reconnect; this reloads all.
//! * `hook/started` / `hook/completed {threadId, turnId, run: {id, eventName,
//!   handlerType, status: running|completed|failed|blocked|stopped,
//!   statusMessage, sourcePath, entries:[{kind, text}], …}}` — schema only,
//!   not observed live.
//!
//! Statuses are normalized to Claude's vocabulary (`connected`, `failed`,
//! `needs-auth`, `pending`, `disabled`) so clients read one set.

use std::collections::HashMap;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use crate::session::conversation::ConversationItem;
use crate::session::provider_status::{clip, ActiveHook, McpServer, StatusControl};
use crate::session::{ConversationStore, SessionStore};

const ID_PREFIX: &str = "wks-status-";

#[derive(Debug)]
enum Waiting {
    /// Every caller sharing the one list request in flight.
    McpStatus(Vec<oneshot::Sender<Result<Vec<McpServer>, String>>>),
    McpReload(oneshot::Sender<Result<(), String>>),
}

#[derive(Debug, Default)]
pub(super) struct CodexStatus {
    next: u64,
    waiting: HashMap<String, Waiting>,
}

/// `mcpServer/startupStatus/updated`'s `status` (plus `failureReason`).
fn startup_status(status: &str, failure: Option<&str>) -> &'static str {
    match (status, failure) {
        (_, Some("reauthenticationRequired")) => "needs-auth",
        ("ready", _) => "connected",
        ("starting", _) => "pending",
        ("failed" | "cancelled", _) => "failed",
        _ => "pending",
    }
}

/// `mcpServerStatus/list`'s `runtimeStatus`.
fn runtime_status(status: &str) -> &'static str {
    match status {
        "connected" => "connected",
        "authenticationRequired" => "needs-auth",
        "failed" | "cancelled" => "failed",
        "disabled" => "disabled",
        _ => "pending",
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

impl CodexStatus {
    fn send(
        &mut self,
        out_tx: &mpsc::UnboundedSender<Value>,
        method: &str,
        params: Option<Value>,
        waiting: Waiting,
    ) {
        self.next += 1;
        let id = format!("{ID_PREFIX}{}", self.next);
        self.waiting.insert(id.clone(), waiting);
        let mut request = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            request["params"] = params;
        }
        let _ = out_tx.send(request);
    }

    /// Write the request `control` asks for. Fast mode and the receipt
    /// interrupt are Claude controls: refused here, with that reason.
    pub(super) fn request(
        &mut self,
        out_tx: &mpsc::UnboundedSender<Value>,
        thread_id: Option<&str>,
        control: StatusControl,
    ) {
        match control {
            StatusControl::McpStatus { reply } => {
                // One list request in flight per session; later callers share
                // its answer.
                if let Some(replies) = self.waiting.values_mut().find_map(|w| match w {
                    Waiting::McpStatus(replies) => Some(replies),
                    _ => None,
                }) {
                    replies.push(reply);
                    return;
                }
                // With the thread, the reply carries each server's runtime
                // status; without it, only the configured names.
                let mut params = json!({ "detail": "toolsAndAuthOnly", "limit": 100 });
                if let Some(tid) = thread_id {
                    params["threadId"] = json!(tid);
                }
                self.send(
                    out_tx,
                    "mcpServerStatus/list",
                    Some(params),
                    Waiting::McpStatus(vec![reply]),
                );
            }
            StatusControl::McpReconnect { reply, .. } => {
                self.send(
                    out_tx,
                    "config/mcpServer/reload",
                    None,
                    Waiting::McpReload(reply),
                );
            }
            StatusControl::FastMode { reply, .. } => {
                let _ = reply.send(Err("fast mode is a Claude setting; Codex has none".into()));
            }
            StatusControl::Interrupt { reply } => {
                let _ = reply.send(Err("interrupt receipts are a Claude stream feature".into()));
            }
        }
    }

    /// Record what one message reports. True when it answered one of our own
    /// requests (the driver must not see it).
    pub(super) fn observe(
        &mut self,
        value: &Value,
        store: &SessionStore,
        conv: &ConversationStore,
        session_id: &str,
        thread_id: Option<&str>,
    ) -> bool {
        if value.get("method").is_none() {
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                return false;
            };
            if !id.starts_with(ID_PREFIX) {
                return false;
            }
            if let Some(waiting) = self.waiting.remove(id) {
                self.answered(waiting, value, store, session_id);
            }
            return true;
        }
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        let params = value.get("params").unwrap_or(&Value::Null);
        // Another thread's (a subagent's) servers and hooks are not this
        // session's.
        let foreign = matches!(
            (text(params, "threadId"), thread_id),
            (Some(theirs), Some(ours)) if theirs != ours
        );
        if foreign {
            return false;
        }
        match method {
            "mcpServer/startupStatus/updated" => {
                if let Some(name) = text(params, "name") {
                    let server = McpServer {
                        name: clip(name),
                        status: startup_status(
                            text(params, "status").unwrap_or(""),
                            text(params, "failureReason"),
                        )
                        .to_owned(),
                        error: text(params, "error").map(clip),
                    };
                    store.update_provider_status(session_id, |s| s.upsert_mcp_server(server));
                }
            }
            "hook/started" => {
                if let Some(hook) = hook_of(params) {
                    store.update_provider_status(session_id, |s| s.hook_started(hook));
                }
            }
            "hook/completed" => {
                let run = params.get("run").unwrap_or(&Value::Null);
                if let Some(id) = text(run, "id") {
                    store.update_provider_status(session_id, |s| s.hook_finished(id));
                }
                if let Some(text) = hook_failure(run) {
                    conv.push(
                        session_id,
                        vec![ConversationItem::AssistantNotice {
                            text,
                            notice: true,
                            timestamp: None,
                        }],
                    );
                }
            }
            _ => {}
        }
        false
    }

    fn answered(
        &mut self,
        waiting: Waiting,
        value: &Value,
        store: &SessionStore,
        session_id: &str,
    ) {
        let error = value.get("error").map(|e| {
            e.get("message")
                .and_then(Value::as_str)
                .map_or_else(|| e.to_string(), str::to_owned)
        });
        match (waiting, error) {
            (Waiting::McpStatus(replies), Some(error)) => {
                for reply in replies {
                    let _ = reply.send(Err(error.clone()));
                }
            }
            (Waiting::McpStatus(reply), None) => {
                let known = store
                    .get(session_id)
                    .and_then(|s| s.provider_status.mcp_servers)
                    .unwrap_or_default();
                let servers = list_servers(value.pointer("/result/data"), &known);
                store.update_provider_status(session_id, |s| s.set_mcp_servers(servers.clone()));
                for reply in reply {
                    let _ = reply.send(Ok(servers.clone()));
                }
            }
            (Waiting::McpReload(reply), error) => {
                // Fresh startup notifications follow a successful reload.
                let _ = reply.send(error.map_or(Ok(()), Err));
            }
        }
    }
}

/// A list reply's servers. `runtimeStatus` is the truth when present; when it
/// is null, the status (and error) already learned from startup notifications
/// stands, else an unauthenticated server reads `needs-auth` and the rest
/// `pending`.
fn list_servers(data: Option<&Value>, known: &[McpServer]) -> Vec<McpServer> {
    data.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let name = text(entry, "name")?;
            let prior = known.iter().find(|s| s.name == name);
            let status = match text(entry, "runtimeStatus") {
                Some(runtime) => runtime_status(runtime).to_owned(),
                None => match prior {
                    Some(prior) => prior.status.clone(),
                    None if text(entry, "authStatus") == Some("notLoggedIn") => "needs-auth".into(),
                    None => "pending".into(),
                },
            };
            let error = prior
                .filter(|prior| prior.status == status)
                .and_then(|prior| prior.error.clone());
            Some(McpServer {
                name: clip(name),
                status,
                error,
            })
        })
        .collect()
}

fn hook_of(params: &Value) -> Option<ActiveHook> {
    let run = params.get("run")?;
    let event = text(run, "eventName").unwrap_or("");
    // Codex hooks have no name: the handler's file names it best.
    let name = text(run, "sourcePath")
        .and_then(|p| p.rsplit(['/', '\\']).next())
        .filter(|n| !n.is_empty())
        .or_else(|| text(run, "handlerType"))
        .unwrap_or(event);
    Some(ActiveHook {
        id: clip(text(run, "id")?),
        name: clip(name),
        event: clip(event),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    })
}

/// The notice a failed Codex hook earns: `status: "failed"` only (`blocked`
/// and `stopped` are a hook doing its job), with its first error entry.
fn hook_failure(run: &Value) -> Option<String> {
    if text(run, "status") != Some("failed") {
        return None;
    }
    let hook = hook_of(&json!({ "run": run }))?;
    let mut out = format!("Hook {}", hook.name);
    if !hook.event.is_empty() && hook.event != hook.name {
        out.push_str(&format!(" ({})", hook.event));
    }
    out.push_str(" failed");
    let detail = run
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|e| text(e, "kind") == Some("error"))
        .and_then(|e| text(e, "text"))
        .or_else(|| text(run, "statusMessage"));
    if let Some(detail) = detail {
        out.push_str(": ");
        out.push_str(&clip(detail.lines().next().unwrap_or(detail)));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (SessionStore, ConversationStore) {
        let store = SessionStore::new();
        store.register_managed("c1", "/tmp", "codex");
        (store, ConversationStore::new())
    }

    #[test]
    fn startup_notifications_upsert_normalized_servers() {
        let (store, conv) = setup();
        let mut status = CodexStatus::default();
        // Captured live (0.153.4): a server that cannot start.
        let failed = json!({"method":"mcpServer/startupStatus/updated","params":{"threadId":"t1",
            "name":"broken","status":"failed","error":"MCP client for `broken` failed to start","failureReason":null}});
        let starting = json!({"method":"mcpServer/startupStatus/updated","params":{"threadId":"t1",
            "name":"ok","status":"starting","error":null,"failureReason":null}});
        assert!(!status.observe(&starting, &store, &conv, "c1", Some("t1")));
        assert!(!status.observe(&failed, &store, &conv, "c1", Some("t1")));
        let servers = store
            .get("c1")
            .unwrap()
            .provider_status
            .mcp_servers
            .unwrap();
        assert_eq!(servers[0].status, "pending");
        assert_eq!(servers[1].status, "failed");
        assert!(servers[1]
            .error
            .as_deref()
            .unwrap()
            .contains("failed to start"));
        // Another thread's servers are ignored.
        let foreign = json!({"method":"mcpServer/startupStatus/updated","params":{"threadId":"t2","name":"x","status":"ready"}});
        status.observe(&foreign, &store, &conv, "c1", Some("t1"));
        assert_eq!(
            store
                .get("c1")
                .unwrap()
                .provider_status
                .mcp_servers
                .unwrap()
                .len(),
            2
        );
        assert_eq!(startup_status("ready", None), "connected");
        assert_eq!(
            startup_status("failed", Some("reauthenticationRequired")),
            "needs-auth"
        );
    }

    #[tokio::test]
    async fn list_and_reload_requests_are_answered_and_consumed() {
        let (store, conv) = setup();
        let mut status = CodexStatus::default();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        // Known from a startup notification: failed, with its error.
        status.observe(
            &json!({"method":"mcpServer/startupStatus/updated","params":{"name":"broken","status":"failed","error":"boom"}}),
            &store, &conv, "c1", None,
        );
        let (reply, verdict) = oneshot::channel();
        status.request(&out_tx, Some("t1"), StatusControl::McpStatus { reply });
        let sent = out_rx.try_recv().unwrap();
        assert_eq!(sent["method"], "mcpServerStatus/list");
        assert_eq!(sent["params"]["threadId"], "t1");
        let id = sent["id"].as_str().unwrap().to_owned();
        // Captured live shape (with threadId → runtimeStatus).
        let reply_frame = json!({"id": id, "result": {"data": [
            {"name":"broken","runtimeStatus":"failed","authStatus":"unsupported","tools":{}},
            {"name":"later","runtimeStatus":null,"authStatus":"notLoggedIn","tools":{}},
            {"name":"fine","runtimeStatus":"connected","authStatus":"bearerToken","tools":{}}
        ], "nextCursor": null}});
        assert!(status.observe(&reply_frame, &store, &conv, "c1", Some("t1")));
        let servers = verdict.await.unwrap().unwrap();
        assert_eq!(
            servers,
            vec![
                McpServer {
                    name: "broken".into(),
                    status: "failed".into(),
                    error: Some("boom".into())
                },
                McpServer {
                    name: "later".into(),
                    status: "needs-auth".into(),
                    error: None
                },
                McpServer {
                    name: "fine".into(),
                    status: "connected".into(),
                    error: None
                },
            ]
        );
        assert_eq!(
            store
                .get("c1")
                .unwrap()
                .provider_status
                .mcp_servers
                .unwrap(),
            servers
        );

        let (reply, verdict) = oneshot::channel();
        status.request(
            &out_tx,
            Some("t1"),
            StatusControl::McpReconnect {
                server: "broken".into(),
                reply,
            },
        );
        let sent = out_rx.try_recv().unwrap();
        assert_eq!(sent["method"], "config/mcpServer/reload");
        assert!(sent.get("params").is_none());
        let id = sent["id"].as_str().unwrap().to_owned();
        assert!(status.observe(
            &json!({"id": id, "error": {"code": -32601, "message": "nope"}}),
            &store,
            &conv,
            "c1",
            None
        ));
        assert_eq!(verdict.await.unwrap(), Err("nope".into()));

        // Claude-only controls are refused at once.
        let (reply, verdict) = oneshot::channel();
        status.request(
            &out_tx,
            None,
            StatusControl::FastMode {
                enabled: true,
                reply,
            },
        );
        assert!(verdict.await.unwrap().is_err());
        assert!(out_rx.try_recv().is_err());

        // The driver's own responses pass through.
        assert!(!status.observe(&json!({"id": 7, "result": {}}), &store, &conv, "c1", None));
        assert!(!status.observe(
            &json!({"id": "workspacer-skill-roots", "result": {}}),
            &store,
            &conv,
            "c1",
            None
        ));
    }

    #[test]
    fn hooks_track_and_failures_notice() {
        let (store, conv) = setup();
        let mut status = CodexStatus::default();
        let run = |status: &str| {
            json!({"id":"r1","eventName":"preToolUse","handlerType":"command",
            "sourcePath":"/home/u/.codex/hooks/guard.sh","status":status,"startedAt":1,
            "entries":[{"kind":"error","text":"denied by policy\nmore"}],"scope":"turn","executionMode":"sync","displayOrder":0})
        };
        status.observe(
            &json!({"method":"hook/started","params":{"threadId":"t1","run":run("running")}}),
            &store,
            &conv,
            "c1",
            Some("t1"),
        );
        let hooks = store.get("c1").unwrap().provider_status.active_hooks;
        assert_eq!(
            (hooks[0].name.as_str(), hooks[0].event.as_str()),
            ("guard.sh", "preToolUse")
        );
        status.observe(
            &json!({"method":"hook/completed","params":{"threadId":"t1","run":run("failed")}}),
            &store,
            &conv,
            "c1",
            Some("t1"),
        );
        assert!(store
            .get("c1")
            .unwrap()
            .provider_status
            .active_hooks
            .is_empty());
        let (_, items) = conv.snapshot("c1").unwrap();
        assert!(items.iter().any(|item| matches!(item,
            ConversationItem::AssistantNotice { text, .. } if text == "Hook guard.sh (preToolUse) failed: denied by policy")));
        assert_eq!(hook_failure(&run("blocked")), None);
    }
}

//! Codex adapter — translate `codex app-server` notifications into claudemon's
//! session model.
//!
//! Codex's app server speaks JSON-RPC 2.0 over stdio (newline-delimited JSON).
//! We `thread/start` (model + cwd) for a thread, `turn/start` each user prompt,
//! and consume the streamed notifications: `turn/started|completed|failed`,
//! `item/started|completed` (commandExecution / fileChange / mcpToolCall / …),
//! `item/agentMessage/delta` (streamed text), `thread/tokenUsage/updated`, and
//! the approval requests (`item/commandExecution/requestApproval`,
//! `item/fileChange/requestApproval`).
//!
//! The pure `translate(method, params)` is unit-tested; the live stdio client
//! needs a real `codex` binary to validate end-to-end.

use std::process::Stdio;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use super::{apply_updates, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::session::conversation::ConversationItem;
use crate::session::state::SessionMode;
use crate::session::{ConversationStore, SessionStore};

/// Translate one Codex app-server message (`method` + `params`) into typed
/// updates. Pure and total: unknown methods / missing fields yield an
/// empty/partial result.
pub fn translate(method: &str, params: &Value) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    match method {
        "turn/started" => out.push(AgentUpdate::Busy),
        "turn/completed" => out.push(AgentUpdate::Idle),
        "turn/failed" => {
            let msg = params
                .get("error")
                .and_then(|e| e.get("message").or_else(|| e.get("data")))
                .and_then(Value::as_str)
                .or_else(|| params.get("message").and_then(Value::as_str))
                .unwrap_or("turn failed");
            out.push(AgentUpdate::Error(msg.to_string()));
            out.push(AgentUpdate::Idle);
        }
        // Streamed assistant text. Reasoning/command-output deltas are skipped
        // (kept out of the conversation to avoid noise).
        "item/agentMessage/delta" => {
            if let Some(text) = params
                .get("delta")
                .and_then(Value::as_str)
                .or_else(|| params.get("text").and_then(Value::as_str))
            {
                if !text.is_empty() {
                    out.push(AgentUpdate::AssistantText(text.to_string()));
                }
            }
        }
        "item/started" | "item/completed" => {
            if let Some(item) = params.get("item") {
                translate_item(method, item, &mut out);
            }
        }
        "thread/tokenUsage/updated" => {
            let u = params.get("usage").unwrap_or(params);
            let input = u.get("input_tokens").and_then(Value::as_u64);
            let output = u.get("output_tokens").and_then(Value::as_u64);
            if input.is_some() || output.is_some() {
                out.push(AgentUpdate::Usage {
                    model: None,
                    input_tokens: input,
                    output_tokens: output,
                    cost_usd: None,
                });
            }
        }
        "item/commandExecution/requestApproval" => {
            let cmd = command_text(params);
            out.push(AgentUpdate::PermissionPending {
                id: None, // carried out of band as the JSON-RPC request id
                tool: Some("command".into()),
                summary: cmd,
            });
        }
        "item/fileChange/requestApproval" => {
            let path = params
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| params.get("item").and_then(|i| i.get("path")).and_then(Value::as_str))
                .map(str::to_owned);
            out.push(AgentUpdate::PermissionPending {
                id: None,
                tool: Some("file change".into()),
                summary: path,
            });
        }
        _ => {}
    }
    out
}

/// Map a started/completed `item` to a tool-use update. Only `item/started`
/// emits tool-uses (so a tool isn't recorded twice); assistant text arrives via
/// `item/agentMessage/delta`, so completed agentMessage items are not re-emitted.
fn translate_item(method: &str, item: &Value, out: &mut Vec<AgentUpdate>) {
    if method != "item/started" {
        return;
    }
    let ty = item
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| item.get("itemType").and_then(Value::as_str))
        .unwrap_or("");
    let id = item.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    match ty {
        "commandExecution" => {
            let input = item
                .get("command")
                .cloned()
                .map(|c| json!({ "command": c }))
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name: "shell".into(), input });
        }
        "fileChange" => {
            let input = json!({ "path": item.get("path").cloned().unwrap_or(Value::Null) });
            out.push(AgentUpdate::ToolUse { id, name: "apply_patch".into(), input });
        }
        "mcpToolCall" => {
            let name = item
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| item.get("name").and_then(Value::as_str))
                .unwrap_or("mcp")
                .to_string();
            let input = item.get("arguments").or_else(|| item.get("input")).cloned().unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        "webSearch" => {
            let input = item.get("query").cloned().map(|q| json!({ "query": q })).unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name: "web_search".into(), input });
        }
        _ => {}
    }
}

/// The command string for an approval request, whether it's a plain string or a
/// list of argv parts.
fn command_text(params: &Value) -> Option<String> {
    let cmd = params
        .get("command")
        .or_else(|| params.get("item").and_then(|i| i.get("command")))?;
    match cmd {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

// ── Model listing ────────────────────────────────────────────────────────────

/// List the models Codex offers, via the app-server's `model/list` JSON-RPC.
/// We boot a throwaway `codex app-server`, `initialize`, ask for the catalog,
/// then drop the process. Hidden models are skipped; the rest map to the picker
/// with their `displayName` as label and the server-flagged default marked.
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    let mut child = Command::new(bin)
        .arg("app-server")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} app-server`"))?;

    let mut stdin = child.stdin.take().context("codex app-server: no stdin")?;
    let stdout = child.stdout.take().context("codex app-server: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    write_msg(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "clientInfo": { "name": "workspacer", "version": "0.1" } }
        }),
    )
    .await?;
    write_msg(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "model/list", "params": {} }),
    )
    .await?;

    // Read until the response to id=2 arrives (or stdout closes). A short overall
    // timeout keeps a wedged binary from hanging the picker.
    let read = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if value.get("id").and_then(Value::as_u64) != Some(2) {
                continue;
            }
            let data = value
                .get("result")
                .and_then(|r| r.get("data"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let models = data
                .iter()
                .filter(|m| !m.get("hidden").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|m| {
                    let id = m
                        .get("model")
                        .or_else(|| m.get("id"))
                        .and_then(Value::as_str)?
                        .to_string();
                    let label = m
                        .get("displayName")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    let default = m.get("isDefault").and_then(Value::as_bool).unwrap_or(false);
                    Some(ModelInfo { id, label, default })
                })
                .collect::<Vec<_>>();
            return Ok(models);
        }
        anyhow::bail!("codex app-server closed before answering model/list");
    };

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), read)
        .await
        .context("timed out listing codex models")?;
    let _ = child.start_kill();
    result
}

// ── Live client ─────────────────────────────────────────────────────────────

/// Spawn and drive a Codex-managed session in the background. Returns
/// immediately; the session id is already registered in `store` by the caller.
pub fn spawn_session(
    store: SessionStore,
    conv: ConversationStore,
    session_id: String,
    cwd: String,
    model: Option<String>,
    bin: String,
    yolo: bool,
    facade: Facade,
) {
    tokio::spawn(async move {
        if let Err(err) = run_session(&store, &conv, &session_id, &cwd, model, &bin, yolo, &facade).await {
            tracing::warn!(?err, session = %session_id, "codex managed session ended with error");
        }
        store.deregister_managed(&session_id);
    });
}

async fn run_session(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    cwd: &str,
    model: Option<String>,
    bin: &str,
    yolo: bool,
    facade: &Facade,
) -> anyhow::Result<()> {
    let mut cmd = Command::new(bin);
    cmd.arg("app-server");
    // Register the workspacer MCP facade (supervisors) as a config override so
    // `codex app-server` exposes its tools.
    if let Some(mcp_url) = &facade.mcp_url {
        cmd.arg("-c")
            .arg(format!("mcp_servers.workspacer.url=\"{mcp_url}\""));
    }
    let mut child = cmd
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} app-server`"))?;

    let mut stdin = child.stdin.take().context("codex app-server: no stdin")?;
    let stdout = child.stdout.take().context("codex app-server: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    // Open a thread (handshake). Its id arrives in the response to id=1.
    let mut start_params = json!({ "cwd": cwd });
    if let Some(m) = &model {
        start_params["model"] = Value::String(m.clone());
    }
    write_msg(&mut stdin, &json!({ "jsonrpc": "2.0", "id": 1, "method": "thread/start", "params": start_params })).await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);

    let mut thread_id: Option<String> = None;
    let mut pending_prompts: Vec<String> = Vec::new();
    let mut req_id: u64 = 1;
    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // The JSON-RPC id of an approval request awaiting the user's decision
    // (non-YOLO). YOLO answers requests inline and never parks one here.
    let mut pending_approval: Option<Value> = None;
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = facade.instructions.clone();

    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        handle_message(
                            &value, store, conv, session_id, &mut stdin,
                            &mut thread_id, &mut pending_prompts, &mut req_id,
                            &mut cur_mode, &mut acc, yolo, &mut pending_approval,
                        ).await;
                    }
                }
                Ok(None) => break, // stdout closed → process gone
                Err(err) => return Err(err.into()),
            },
            msg = rx.recv() => match msg {
                Some(text) => {
                    // Echo the user's message verbatim, but prepend the role
                    // instructions (once) to what's actually sent to the agent.
                    conv.push(session_id, vec![ConversationItem::UserMessage { text: text.clone(), timestamp: None }]);
                    let sent = match pending_instructions.take() {
                        Some(instr) => format!("{instr}\n\n{text}"),
                        None => text,
                    };
                    if cur_mode != SessionMode::Responding {
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                    match &thread_id {
                        Some(tid) => {
                            req_id += 1;
                            let _ = send_turn(&mut stdin, req_id, tid, &sent).await;
                        }
                        // Thread not open yet — buffer the (already-wrapped) prompt.
                        None => pending_prompts.push(sent),
                    }
                }
                None => break, // managed input dropped → terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    if let Some(id) = pending_approval.take() {
                        let result = json!({ "decision": if approve { "accept" } else { "decline" } });
                        let _ = write_msg(&mut stdin, &json!({ "jsonrpc": "2.0", "id": id, "result": result })).await;
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                }
                None => break,
            },
            status = child.wait() => {
                tracing::info!(?status, session = %session_id, "codex app-server exited");
                break;
            }
        }
    }

    let _ = child.start_kill();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    value: &Value,
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    stdin: &mut ChildStdin,
    thread_id: &mut Option<String>,
    pending_prompts: &mut Vec<String>,
    req_id: &mut u64,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    yolo: bool,
    pending_approval: &mut Option<Value>,
) {
    // A response to one of our requests (the thread/start handshake).
    if value.get("id").is_some() && (value.get("result").is_some() || value.get("error").is_some()) {
        if thread_id.is_none() {
            if let Some(tid) = value
                .get("result")
                .and_then(|r| r.get("threadId").or_else(|| r.get("thread").and_then(|t| t.get("id"))))
                .and_then(Value::as_str)
            {
                *thread_id = Some(tid.to_string());
                // Flush any prompts that arrived before the thread opened.
                for text in std::mem::take(pending_prompts) {
                    *req_id += 1;
                    let _ = send_turn(stdin, *req_id, tid, &text).await;
                }
            }
        }
        return;
    }

    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = value.get("params").cloned().unwrap_or(Value::Null);

    let updates = translate(method, &params);
    if !updates.is_empty() {
        apply_updates(store, conv, session_id, updates, cur_mode, acc);
    }

    // Server→client *requests* (they carry an id) must be answered or the agent
    // blocks. For an approval request: YOLO accepts inline; otherwise we park the
    // request id and surface it, so the user's /approve decision is forwarded
    // (see the decision branch in run_session).
    if value.get("id").is_some() && method.ends_with("/requestApproval") {
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        if yolo {
            let _ = write_msg(stdin, &json!({ "jsonrpc": "2.0", "id": id, "result": { "decision": "accept" } })).await;
        } else {
            *pending_approval = Some(id);
        }
    }
}

async fn send_turn(stdin: &mut ChildStdin, id: u64, thread_id: &str, text: &str) -> anyhow::Result<()> {
    write_msg(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "turn/start",
            "params": { "threadId": thread_id, "input": [ { "type": "text", "text": text } ] }
        }),
    )
    .await
}

/// Write one JSON-RPC message as a single newline-delimited line.
async fn write_msg(stdin: &mut ChildStdin, value: &Value) -> anyhow::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_started_is_busy() {
        assert_eq!(translate("turn/started", &json!({})), vec![AgentUpdate::Busy]);
    }

    #[test]
    fn turn_completed_is_idle() {
        assert_eq!(translate("turn/completed", &json!({})), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn turn_failed_is_error_then_idle() {
        let p = json!({ "error": { "message": "nope" } });
        assert_eq!(
            translate("turn/failed", &p),
            vec![AgentUpdate::Error("nope".into()), AgentUpdate::Idle]
        );
    }

    #[test]
    fn agent_message_delta_is_assistant_text() {
        let p = json!({ "delta": "hi there" });
        assert_eq!(
            translate("item/agentMessage/delta", &p),
            vec![AgentUpdate::AssistantText("hi there".into())]
        );
    }

    #[test]
    fn command_execution_item_started_is_tool_use() {
        let p = json!({ "item": { "type": "commandExecution", "id": "i1", "command": ["bash", "-c", "ls"] } });
        assert_eq!(
            translate("item/started", &p),
            vec![AgentUpdate::ToolUse {
                id: "i1".into(),
                name: "shell".into(),
                input: json!({ "command": ["bash", "-c", "ls"] }),
            }]
        );
    }

    #[test]
    fn item_completed_does_not_double_emit_tool_use() {
        let p = json!({ "item": { "type": "commandExecution", "id": "i1", "command": "ls" } });
        assert!(translate("item/completed", &p).is_empty());
    }

    #[test]
    fn token_usage_maps_to_usage() {
        let p = json!({ "usage": { "input_tokens": 1000, "output_tokens": 200, "cached_input_tokens": 50 } });
        assert_eq!(
            translate("thread/tokenUsage/updated", &p),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(1000),
                output_tokens: Some(200),
                cost_usd: None,
            }]
        );
    }

    #[test]
    fn command_approval_request_is_pending_with_joined_argv() {
        let p = json!({ "command": ["rm", "-rf", "build"] });
        assert_eq!(
            translate("item/commandExecution/requestApproval", &p),
            vec![AgentUpdate::PermissionPending {
                id: None,
                tool: Some("command".into()),
                summary: Some("rm -rf build".into()),
            }]
        );
    }

    #[test]
    fn file_change_approval_request_is_pending_with_path() {
        let p = json!({ "path": "src/main.rs" });
        assert_eq!(
            translate("item/fileChange/requestApproval", &p),
            vec![AgentUpdate::PermissionPending {
                id: None,
                tool: Some("file change".into()),
                summary: Some("src/main.rs".into()),
            }]
        );
    }

    #[test]
    fn unknown_method_is_ignored() {
        assert!(translate("session/whatever", &json!({ "x": 1 })).is_empty());
        assert!(translate("item/reasoning/summaryTextDelta", &json!({ "delta": "thinking" })).is_empty());
    }
}

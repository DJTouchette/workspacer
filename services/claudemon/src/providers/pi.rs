//! Pi adapter — translate `pi --mode rpc` events into claudemon's session model.
//!
//! Pi (the `@mariozechner/pi-coding-agent` harness) exposes an RPC mode that
//! speaks strict LF-delimited JSONL over stdio: each line is a JSON object
//! tagged with a `type`. We send `{"type":"prompt","message":...}` for each
//! user turn and consume the streamed events: `agent_start`/`agent_end` and
//! `turn_start`/`turn_end` (lifecycle), `message_update` carrying an
//! `assistantMessageEvent` (streamed text deltas), `message_end`/`turn_end`
//! (whose message object carries token usage), and `tool_execution_start`
//! (tool calls).
//!
//! Pi's core auto-runs tools without an approval gate; approvals only appear
//! when a permission *extension* is loaded, which prompts via the bidirectional
//! Extension UI protocol (`extension_ui_request` with a `confirm`/`select`
//! dialog). We answer those: YOLO accepts inline; otherwise we surface the
//! request and forward the user's /approve decision as an `extension_ui_response`.
//!
//! The pure `translate(event)` is unit-tested; the live stdio client needs a
//! real `pi` binary to validate end-to-end.

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

/// List the models Pi can launch with, via its RPC `get_available_models`
/// command. We boot a throwaway `pi --mode rpc`, ask for the catalog, then drop
/// the process. Pi only returns models for providers the user has authed, so an
/// empty list (no login) is normal — the picker then falls back to free text.
/// Each model carries `provider` + `id`; we join them as `provider/id`, exactly
/// the form `--model` accepts.
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    let mut child = Command::new(bin)
        .arg("--mode")
        .arg("rpc")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} --mode rpc`"))?;

    let mut stdin = child.stdin.take().context("pi rpc: no stdin")?;
    let stdout = child.stdout.take().context("pi rpc: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    write_msg(&mut stdin, &json!({ "type": "get_available_models", "id": "models" })).await?;

    let read = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if value.get("command").and_then(Value::as_str) != Some("get_available_models") {
                continue;
            }
            let arr = value
                .get("data")
                .and_then(|d| d.get("models"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let models = arr
                .iter()
                .filter_map(|m| {
                    let provider = m.get("provider").and_then(Value::as_str)?;
                    let id = m.get("id").and_then(Value::as_str)?;
                    let full = format!("{provider}/{id}");
                    Some(ModelInfo { id: full.clone(), label: full, default: false })
                })
                .collect::<Vec<_>>();
            return Ok(models);
        }
        anyhow::bail!("pi rpc closed before answering get_available_models");
    };

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), read)
        .await
        .context("timed out listing pi models")?;
    let _ = child.start_kill();
    result
}

/// Translate one Pi RPC event into zero or more typed updates. Pure and total:
/// unknown event types and missing fields yield an empty/partial result rather
/// than an error. `extension_ui_request` is handled out of band (it needs a
/// reply), so it is intentionally *not* translated here.
pub fn translate(event: &Value) -> Vec<AgentUpdate> {
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    let mut out = Vec::new();

    match ty {
        // The agent loop started / a single LLM turn started → working.
        "agent_start" | "turn_start" => out.push(AgentUpdate::Busy),

        // The whole agent loop finished → ready for the next prompt. (A
        // `turn_end` is only *one* round within the loop; more turns may follow,
        // so it must NOT flip us back to Input — we only pull its usage below.)
        "agent_end" => out.push(AgentUpdate::Idle),

        "turn_end" => {
            if let Some(u) = event.get("message").and_then(usage_from) {
                out.push(u);
            }
        }

        // Streamed assistant output. Only text deltas land in the conversation;
        // thinking / tool-call deltas are skipped (tool calls arrive as their
        // own `tool_execution_start`).
        "message_update" => {
            out.push(AgentUpdate::Busy);
            if let Some(ev) = event.get("assistantMessageEvent") {
                if ev.get("type").and_then(Value::as_str) == Some("text_delta") {
                    if let Some(delta) = ev.get("delta").and_then(Value::as_str) {
                        if !delta.is_empty() {
                            out.push(AgentUpdate::AssistantText(delta.to_string()));
                        }
                    }
                }
            }
        }

        // End of one assistant message — carries final token usage. Text was
        // already streamed via `message_update`, so we don't re-emit it.
        "message_end" => {
            if let Some(u) = event.get("message").and_then(usage_from) {
                out.push(u);
            }
        }

        "tool_execution_start" => {
            let id = event
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = event
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let input = event.get("args").cloned().unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }

        "extension_error" => {
            if let Some(msg) = event.get("error").and_then(Value::as_str) {
                out.push(AgentUpdate::Error(msg.to_string()));
            }
        }

        _ => {}
    }

    out
}

/// Extract a `Usage` update from a Pi message object (`usage` + `model`). Pi's
/// unified API has used a few key spellings across versions, so we probe the
/// common ones defensively.
fn usage_from(message: &Value) -> Option<AgentUpdate> {
    let usage = message.get("usage");
    let pick = |keys: &[&str]| -> Option<u64> {
        let u = usage?;
        keys.iter().find_map(|k| u.get(*k).and_then(Value::as_u64))
    };
    let input = pick(&["input", "inputTokens", "input_tokens", "promptTokens", "prompt_tokens"]);
    let output = pick(&["output", "outputTokens", "output_tokens", "completionTokens", "completion_tokens"]);
    let cost = usage
        .and_then(|u| u.get("cost"))
        .and_then(Value::as_f64)
        .or_else(|| message.get("cost").and_then(Value::as_f64));
    let model = message.get("model").and_then(Value::as_str).map(str::to_owned);
    if input.is_none() && output.is_none() && cost.is_none() && model.is_none() {
        return None;
    }
    Some(AgentUpdate::Usage {
        model,
        input_tokens: input,
        output_tokens: output,
        cost_usd: cost,
    })
}

/// A short human summary for an Extension UI dialog request, for the approval card.
fn dialog_summary(req: &Value) -> Option<String> {
    req.get("message")
        .and_then(Value::as_str)
        .or_else(|| req.get("title").and_then(Value::as_str))
        .or_else(|| req.get("placeholder").and_then(Value::as_str))
        .map(str::to_owned)
}

// ── Live client ─────────────────────────────────────────────────────────────

/// Spawn and drive a Pi-managed session in the background. Returns immediately;
/// the session id is already registered in `store` by the caller.
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
            tracing::warn!(?err, session = %session_id, "pi managed session ended with error");
        }
        store.deregister_managed(&session_id);
        conv.forget(&session_id);
    });
}

/// Merge the workspacer MCP facade into the cwd's `.mcp.json` so Pi loads it as
/// a remote (HTTP) MCP server. Preserves any existing config; only sets
/// `mcpServers.workspacer`. Pi reads the standard `.mcp.json` automatically.
fn write_pi_mcp(cwd: &str, mcp_url: &str) {
    let path = std::path::Path::new(cwd).join(".mcp.json");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    root["mcpServers"]["workspacer"] = json!({
        "type": "http",
        "url": mcp_url,
    });
    if let Ok(text) = serde_json::to_string_pretty(&root) {
        if let Err(err) = std::fs::write(&path, text) {
            tracing::warn!(?err, "writing .mcp.json for the facade failed");
        }
    }
}

#[allow(clippy::too_many_arguments)]
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
    // Register the workspacer MCP facade (supervisors) before the agent boots.
    if let Some(mcp_url) = &facade.mcp_url {
        write_pi_mcp(cwd, mcp_url);
    }

    let mut cmd = Command::new(bin);
    cmd.arg("--mode").arg("rpc");
    if let Some(m) = &model {
        cmd.arg("--model").arg(m);
    }
    let mut child = cmd
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} --mode rpc`"))?;

    let mut stdin = child.stdin.take().context("pi rpc: no stdin")?;
    let stdout = child.stdout.take().context("pi rpc: no stdout")?;
    // Pi requires splitting on LF only; tokio's line reader does exactly that
    // (and trims a trailing CR), never on U+2028/U+2029 — safe per its spec.
    let mut lines = BufReader::new(stdout).lines();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);

    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // Extension UI requests awaiting the user's decision (non-YOLO), FIFO. We park
    // each whole request so we can echo its `id` + `method` in the reply; a queue
    // (not one slot) so concurrent requests don't drop each other and stall.
    let mut pending_approvals: std::collections::VecDeque<Value> = std::collections::VecDeque::new();
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = facade.instructions.clone();

    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        handle_message(
                            &value, store, conv, session_id, &mut stdin,
                            &mut cur_mode, &mut acc, yolo, &mut pending_approvals,
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
                    let _ = write_msg(&mut stdin, &json!({ "type": "prompt", "message": sent })).await;
                }
                None => break, // managed input dropped → terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    if let Some(req) = pending_approvals.pop_front() {
                        let _ = respond_ui(&mut stdin, &req, approve).await;
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                }
                None => break,
            },
            status = child.wait() => {
                tracing::info!(?status, session = %session_id, "pi rpc exited");
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
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    yolo: bool,
    pending_approvals: &mut std::collections::VecDeque<Value>,
) {
    let ty = value.get("type").and_then(Value::as_str).unwrap_or("");

    // Command acknowledgements (`{"type":"response", ...}`) carry no telemetry.
    if ty == "response" {
        return;
    }

    // Extension UI requests are server→client *requests* that must be answered
    // or the agent blocks. Dialog methods (confirm/select/input/editor) are the
    // approval seam; fire-and-forget methods (notify/setStatus/…) need no reply.
    if ty == "extension_ui_request" {
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        let is_dialog = matches!(method, "confirm" | "select" | "input" | "editor");
        if !is_dialog {
            return; // notify / setStatus / setWidget / setTitle / set_editor_text
        }
        if yolo {
            // Auto-accept so the agent keeps working without surfacing a card.
            let _ = respond_ui(stdin, value, true).await;
        } else {
            // Surface the approval and remember the request so /approve can
            // forward the user's decision (see the decision branch above).
            let updates = vec![AgentUpdate::PermissionPending {
                id: None,
                tool: Some(method.to_string()),
                summary: dialog_summary(value),
            }];
            apply_updates(store, conv, session_id, updates, cur_mode, acc);
            pending_approvals.push_back(value.clone());
        }
        return;
    }

    let updates = translate(value);
    if !updates.is_empty() {
        apply_updates(store, conv, session_id, updates, cur_mode, acc);
    }
}

/// Answer an Extension UI dialog request. `confirm` takes a boolean; `select`
/// resolves to an "allow"-ish option (or cancels on reject); `input`/`editor`
/// can't be synthesized meaningfully, so accept yields empty text and reject
/// cancels.
async fn respond_ui(stdin: &mut ChildStdin, req: &Value, approve: bool) -> anyhow::Result<()> {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("confirm");
    let mut msg = json!({ "type": "extension_ui_response", "id": id });
    match method {
        "confirm" => {
            msg["confirmed"] = json!(approve);
        }
        "select" if approve => {
            // Prefer an option whose label reads like an allow; fall back to the
            // first option offered.
            let opts = req.get("options").and_then(Value::as_array);
            let value = opts
                .and_then(|o| {
                    o.iter()
                        .filter_map(Value::as_str)
                        .find(|s| {
                            let l = s.to_ascii_lowercase();
                            l.contains("allow") || l.contains("yes") || l.contains("accept")
                        })
                })
                .or_else(|| opts.and_then(|o| o.first()).and_then(Value::as_str))
                .unwrap_or("Allow");
            msg["value"] = json!(value);
        }
        "input" | "editor" if approve => {
            msg["value"] = json!("");
        }
        _ => {
            msg["cancelled"] = json!(true);
        }
    }
    write_msg(stdin, &msg).await
}

/// Write one RPC message as a single LF-delimited JSON line.
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
    fn agent_start_is_busy() {
        assert_eq!(translate(&json!({ "type": "agent_start" })), vec![AgentUpdate::Busy]);
        assert_eq!(translate(&json!({ "type": "turn_start" })), vec![AgentUpdate::Busy]);
    }

    #[test]
    fn agent_end_is_idle() {
        assert_eq!(translate(&json!({ "type": "agent_end" })), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn turn_end_does_not_idle_but_pulls_usage() {
        // turn_end is one round inside the loop — must not flip to Input.
        let ev = json!({ "type": "turn_end", "message": { "usage": { "input": 10, "output": 2 } } });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(10),
                output_tokens: Some(2),
                cost_usd: None,
            }]
        );
        // No message → no updates (and still not Idle).
        assert!(translate(&json!({ "type": "turn_end" })).is_empty());
    }

    #[test]
    fn message_update_text_delta_is_busy_plus_text() {
        let ev = json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "text_delta", "contentIndex": 0, "delta": "Hello " }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Busy, AgentUpdate::AssistantText("Hello ".into())]
        );
    }

    #[test]
    fn message_update_non_text_delta_is_just_busy() {
        let ev = json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "thinking_delta", "delta": "hmm" }
        });
        assert_eq!(translate(&ev), vec![AgentUpdate::Busy]);
    }

    #[test]
    fn tool_execution_start_is_tool_use() {
        let ev = json!({
            "type": "tool_execution_start",
            "toolCallId": "call_123",
            "toolName": "bash",
            "args": { "command": "ls -la" }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::ToolUse {
                id: "call_123".into(),
                name: "bash".into(),
                input: json!({ "command": "ls -la" }),
            }]
        );
    }

    #[test]
    fn tool_execution_end_is_ignored() {
        let ev = json!({ "type": "tool_execution_end", "toolCallId": "call_123", "result": {} });
        assert!(translate(&ev).is_empty());
    }

    #[test]
    fn message_end_extracts_usage_and_model() {
        let ev = json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "model": "claude-sonnet-4-20250514",
                "usage": { "input": 1200, "output": 340, "cost": 0.0123 }
            }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: Some("claude-sonnet-4-20250514".into()),
                input_tokens: Some(1200),
                output_tokens: Some(340),
                cost_usd: Some(0.0123),
            }]
        );
    }

    #[test]
    fn usage_accepts_camelcase_token_keys() {
        let ev = json!({
            "type": "message_end",
            "message": { "usage": { "inputTokens": 5, "outputTokens": 7 } }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(5),
                output_tokens: Some(7),
                cost_usd: None,
            }]
        );
    }

    #[test]
    fn extension_error_maps_to_error() {
        let ev = json!({ "type": "extension_error", "error": "boom" });
        assert_eq!(translate(&ev), vec![AgentUpdate::Error("boom".into())]);
    }

    #[test]
    fn extension_ui_request_is_not_translated_here() {
        // Handled out of band (needs a reply) — translate stays silent.
        let ev = json!({ "type": "extension_ui_request", "id": "u1", "method": "confirm", "title": "Run?" });
        assert!(translate(&ev).is_empty());
    }

    #[test]
    fn unknown_and_malformed_events_are_ignored() {
        assert!(translate(&json!({ "type": "queue_update", "steering": [] })).is_empty());
        assert!(translate(&json!({})).is_empty());
        assert!(translate(&Value::Null).is_empty());
    }
}

//! OpenCode adapter — translate `opencode serve` events into claudemon's
//! session model.
//!
//! OpenCode exposes a headless HTTP server (`opencode serve`, default
//! 127.0.0.1:4096, OpenAPI 3.1) and a Server-Sent-Events stream at `GET /event`
//! whose frames are `{ "type": "<entity>.<action>", "properties": { … } }`
//! (e.g. `session.idle`, `message.part.updated`, `permission.updated`). The
//! stream carries 80+ event types and evolves quickly, so the translator below
//! is deliberately *defensive*: it recognizes the events that map onto our
//! model and ignores everything else (rather than failing on unknown shapes).
//!
//! The pure `translate` is unit-tested. The live client that spawns
//! `opencode serve`, creates a session, posts prompts, and pumps the SSE stream
//! through `translate` + the shared `apply_updates` needs a real `opencode`
//! binary to validate end-to-end.

use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{apply_updates, AgentUpdate, Facade, UsageAcc};
use crate::session::conversation::ConversationItem;
use crate::session::state::SessionMode;
use crate::session::{ConversationStore, SessionStore};

/// Translate one OpenCode SSE event into zero or more typed updates. Pure and
/// total: unknown event types and missing fields yield an empty/partial result
/// rather than an error.
pub fn translate(event: &Value) -> Vec<AgentUpdate> {
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    let props = event.get("properties").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();

    match ty {
        "session.idle" => out.push(AgentUpdate::Idle),

        "session.error" => {
            let msg = props
                .get("error")
                .and_then(|e| e.get("message").or_else(|| e.get("data")))
                .and_then(Value::as_str)
                .or_else(|| props.get("message").and_then(Value::as_str))
                .unwrap_or("session error");
            out.push(AgentUpdate::Error(msg.to_string()));
        }

        "permission.updated" | "permission.replied" => {
            let p = props.get("permission").unwrap_or(&props);
            let id = p
                .get("id")
                .or_else(|| p.get("permissionID"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let tool = p
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| p.get("type").and_then(Value::as_str))
                .map(str::to_owned);
            let summary = p
                .get("metadata")
                .and_then(|m| m.get("command"))
                .and_then(Value::as_str)
                .or_else(|| p.get("description").and_then(Value::as_str))
                .map(str::to_owned);
            out.push(AgentUpdate::PermissionPending { id, tool, summary });
        }

        // Both the streamed-part event and the whole-message event indicate the
        // agent is working; pull text / tool / usage out of whichever shape the
        // event carries.
        "message.updated" | "message.part.updated" => {
            out.push(AgentUpdate::Busy);
            if let Some(part) = props.get("part") {
                translate_part(part, &props, &mut out);
            }
            if let Some(info) = props.get("info").or_else(|| props.get("message")) {
                if let Some(u) = usage_from(info) {
                    out.push(u);
                }
            }
        }

        _ => {}
    }

    out
}

/// Map a message Part (`text` / `tool` / `step-finish` / …) to updates.
fn translate_part(part: &Value, props: &Value, out: &mut Vec<AgentUpdate>) {
    let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
    let role = part
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| props.get("role").and_then(Value::as_str));
    match kind {
        "text" => {
            if let Some(text) = part
                .get("delta")
                .and_then(Value::as_str)
                .or_else(|| part.get("text").and_then(Value::as_str))
            {
                if !text.is_empty() {
                    if role == Some("user") {
                        out.push(AgentUpdate::UserText(text.to_string()));
                    } else {
                        out.push(AgentUpdate::AssistantText(text.to_string()));
                    }
                }
            }
        }
        "tool" => {
            let id = part
                .get("callID")
                .and_then(Value::as_str)
                .or_else(|| part.get("id").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let name = part
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| part.get("name").and_then(Value::as_str))
                .unwrap_or("tool")
                .to_string();
            let input = part
                .get("state")
                .and_then(|s| s.get("input"))
                .or_else(|| part.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        "step-finish" | "step_finish" => {
            if let Some(u) = usage_from(part) {
                out.push(u);
            }
        }
        _ => {}
    }
}

/// Extract a `Usage` update from any object carrying `tokens` / `cost` /
/// `modelID` (a message info or a step-finish part).
fn usage_from(v: &Value) -> Option<AgentUpdate> {
    let tokens = v.get("tokens");
    let input = tokens.and_then(|t| t.get("input")).and_then(Value::as_u64);
    let output = tokens.and_then(|t| t.get("output")).and_then(Value::as_u64);
    let cost = v.get("cost").and_then(Value::as_f64);
    let model = v
        .get("modelID")
        .or_else(|| v.get("model"))
        .and_then(Value::as_str)
        .map(str::to_owned);
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

// ── Live client ─────────────────────────────────────────────────────────────

/// Spawn and drive an OpenCode-managed session in the background. Returns
/// immediately; the session's id is already registered in `store` by the
/// caller, so the UI shows it even while `opencode serve` is still booting.
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
            tracing::warn!(?err, session = %session_id, "opencode managed session ended with error");
        }
        store.deregister_managed(&session_id);
    });
}

/// Merge the workspacer MCP facade into the cwd's `opencode.json` so
/// `opencode serve` loads it as a remote MCP server. Preserves any existing
/// config; only sets `mcp.workspacer`.
fn write_opencode_mcp(cwd: &str, mcp_url: &str) {
    let path = std::path::Path::new(cwd).join("opencode.json");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    root["mcp"]["workspacer"] = serde_json::json!({
        "type": "remote",
        "url": mcp_url,
        "enabled": true,
    });
    if let Ok(text) = serde_json::to_string_pretty(&root) {
        if let Err(err) = std::fs::write(&path, text) {
            tracing::warn!(?err, "writing opencode.json for the facade failed");
        }
    }
}

/// POST a permission reply to OpenCode: `once` (allow this time) or `reject`.
/// Mirrors the SDK's `SessionPermissionService.Respond`.
fn reply_permission(client: &reqwest::Client, base: &str, oc_id: &str, perm_id: &str, approve: bool) {
    let url = format!("{base}/session/{oc_id}/permissions/{perm_id}");
    let body = serde_json::json!({ "response": if approve { "once" } else { "reject" } });
    let c = client.clone();
    tokio::spawn(async move {
        if let Err(err) = c.post(url).json(&body).send().await {
            tracing::warn!(?err, "opencode permission reply failed");
        }
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
    // Register the workspacer MCP facade (supervisors) before the server boots.
    if let Some(mcp_url) = &facade.mcp_url {
        write_opencode_mcp(cwd, mcp_url);
    }

    // Pick a free loopback port for this server instance (each managed session
    // gets its own `opencode serve`).
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("reserving a port for opencode serve")?;
        listener.local_addr()?.port()
    };

    let mut child = Command::new(bin)
        .arg("serve")
        .arg("--hostname")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} serve`"))?;

    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    wait_healthy(&client, &base).await?;

    // Create the OpenCode session (its own id, distinct from our canonical one).
    let oc_id: String = {
        let resp = client
            .post(format!("{base}/session"))
            .json(&serde_json::json!({}))
            .send()
            .await?
            .error_for_status()?;
        let v: Value = resp.json().await?;
        v.get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("opencode POST /session response missing `id`")?
    };

    // Route user prompts + approval decisions from the HTTP API to us.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);

    // Subscribe to the event stream.
    let resp = client
        .get(format!("{base}/event"))
        .send()
        .await?
        .error_for_status()?;
    let mut stream = resp.bytes_stream();

    let mut buf: Vec<u8> = Vec::new();
    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // The id of the permission currently awaiting the user's decision (non-YOLO).
    let mut pending_perm_id: Option<String> = None;
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = facade.instructions.clone();

    loop {
        tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => {
                    buf.extend_from_slice(&bytes);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=pos).collect();
                        let s = String::from_utf8_lossy(&line);
                        let s = s.trim_end_matches(['\r', '\n']);
                        let Some(data) = s.strip_prefix("data:") else { continue };
                        let data = data.trim();
                        if data.is_empty() { continue; }
                        let Ok(value) = serde_json::from_str::<Value>(data) else { continue };
                        let mut updates = translate(&value);
                        if updates.is_empty() { continue; }
                        // Pull any permission request out: YOLO auto-allows it and
                        // keeps working; otherwise we surface it and remember the id
                        // so the user's decision can be forwarded.
                        if let Some(idx) = updates.iter().position(|u| matches!(u, AgentUpdate::PermissionPending { .. })) {
                            if let AgentUpdate::PermissionPending { id, .. } = &updates[idx] {
                                let perm_id = id.clone();
                                if yolo {
                                    if let Some(pid) = perm_id {
                                        reply_permission(&client, &base, &oc_id, &pid, true);
                                    }
                                    updates.remove(idx); // don't surface Approval
                                } else {
                                    pending_perm_id = perm_id;
                                }
                            }
                        }
                        apply_updates(store, conv, session_id, updates, &mut cur_mode, &mut acc);
                    }
                }
                Some(Err(err)) => return Err(err.into()),
                None => break,
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
                    let c = client.clone();
                    let url = format!("{base}/session/{oc_id}/message");
                    let mut body = serde_json::json!({ "parts": [ { "type": "text", "text": sent } ] });
                    if let Some(m) = &model {
                        body["model"] = Value::String(m.clone());
                    }
                    tokio::spawn(async move {
                        if let Err(err) = c.post(url).json(&body).send().await {
                            tracing::warn!(?err, "opencode message POST failed");
                        }
                    });
                }
                None => break, // managed input dropped → session terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    if let Some(pid) = pending_perm_id.take() {
                        reply_permission(&client, &base, &oc_id, &pid, approve);
                        // The agent resumes (or stops on reject); reflect Responding.
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                }
                None => break,
            },
            status = child.wait() => {
                tracing::info!(?status, session = %session_id, "opencode serve exited");
                break;
            }
        }
    }

    let _ = child.start_kill();
    Ok(())
}

/// Poll `/global/health` until the server answers (or we give up after ~10s).
async fn wait_healthy(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    for _ in 0..50 {
        if let Ok(resp) = client.get(format!("{base}/global/health")).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("opencode serve did not become healthy in time")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn idle_maps_to_idle() {
        let ev = json!({ "type": "session.idle", "properties": { "sessionID": "s1" } });
        assert_eq!(translate(&ev), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn error_extracts_message() {
        let ev = json!({ "type": "session.error", "properties": { "error": { "message": "boom" } } });
        assert_eq!(translate(&ev), vec![AgentUpdate::Error("boom".into())]);
    }

    #[test]
    fn permission_pending_pulls_tool_and_command() {
        let ev = json!({
            "type": "permission.updated",
            "properties": {
                "id": "perm_1",
                "type": "bash",
                "title": "Bash",
                "metadata": { "command": "rm -rf build" }
            }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::PermissionPending {
                id: Some("perm_1".into()),
                tool: Some("Bash".into()),
                summary: Some("rm -rf build".into()),
            }]
        );
    }

    #[test]
    fn text_part_is_busy_plus_assistant_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "Hello" } }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Busy, AgentUpdate::AssistantText("Hello".into())]
        );
    }

    #[test]
    fn text_part_prefers_delta_over_full_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "Hello world", "delta": " world" } }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Busy, AgentUpdate::AssistantText(" world".into())]
        );
    }

    #[test]
    fn user_role_text_maps_to_user_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "hi", "role": "user" } }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Busy, AgentUpdate::UserText("hi".into())]
        );
    }

    #[test]
    fn tool_part_maps_to_tool_use() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": {
                "type": "tool", "tool": "bash", "callID": "c1",
                "state": { "input": { "command": "ls" } }
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::ToolUse {
                    id: "c1".into(),
                    name: "bash".into(),
                    input: json!({ "command": "ls" }),
                }
            ]
        );
    }

    #[test]
    fn step_finish_part_yields_usage() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": {
                "type": "step-finish",
                "tokens": { "input": 1200, "output": 340 },
                "cost": 0.0123,
                "modelID": "claude-sonnet-4"
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::Usage {
                    model: Some("claude-sonnet-4".into()),
                    input_tokens: Some(1200),
                    output_tokens: Some(340),
                    cost_usd: Some(0.0123),
                }
            ]
        );
    }

    #[test]
    fn message_info_usage_is_extracted() {
        let ev = json!({
            "type": "message.updated",
            "properties": { "info": {
                "role": "assistant",
                "tokens": { "input": 50, "output": 9 },
                "cost": 0.001
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::Usage {
                    model: None,
                    input_tokens: Some(50),
                    output_tokens: Some(9),
                    cost_usd: Some(0.001),
                }
            ]
        );
    }

    #[test]
    fn unknown_event_is_ignored() {
        let ev = json!({ "type": "installation.updated", "properties": {} });
        assert!(translate(&ev).is_empty());
        let ev2 = json!({ "type": "lsp.diagnostics", "properties": { "anything": 1 } });
        assert!(translate(&ev2).is_empty());
    }

    #[test]
    fn malformed_event_does_not_panic() {
        assert!(translate(&json!({})).is_empty());
        assert_eq!(translate(&json!({ "type": "message.part.updated" })), vec![AgentUpdate::Busy]);
        assert!(translate(&Value::Null).is_empty());
    }
}

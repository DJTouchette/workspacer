//! Codex adapter — translate `codex app-server` notifications into claudemon's
//! session model.
//!
//! Codex's app server speaks JSON-RPC 2.0. We run it as a WebSocket daemon
//! (`codex app-server --listen ws://127.0.0.1:<port>`) rather than over stdio,
//! because a plain-TCP ws endpoint is the one transport a live TUI
//! (`codex --remote ws://…`) and our RPC client can *share* — so the session is
//! a **hybrid** (GUI + Term), like the OpenCode `serve` + `attach` pairing.
//! (`--listen unix://` is gated in current Codex builds and the `remote-control`
//! daemon needs the standalone installer, so ws is the portable choice — and
//! works on Windows too, unlike the unix-socket paths.)
//!
//! Ownership is TUI-first: the native TUI (`codex --remote`, in a PTY = the Term
//! view) creates and runs the session's thread — a real, "running", resumable
//! rollout — and our RPC client discovers it (`thread/loaded/list`) and *rejoins*
//! it (`thread/resume`, which subscribes us to the live stream). The reverse (RPC
//! `thread/start` + TUI `resume`) fails: a just-started thread has no rollout, so
//! `resume` errors with "no rollout found for thread id …". Once rejoined we
//! `turn/start` each GUI prompt and consume the streamed notifications:
//! `turn/started|completed|failed`,
//! `item/started|completed` (commandExecution / fileChange / mcpToolCall / …),
//! `item/agentMessage/delta` (streamed text), `thread/tokenUsage/updated`, and
//! the approval requests (`item/commandExecution/requestApproval`,
//! `item/fileChange/requestApproval`).
//!
//! The pure `translate(method, params)` is unit-tested; the live ws client
//! needs a real `codex` binary to validate end-to-end.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::{apply_updates, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::protocol::Signal;
use crate::session::conversation::ConversationItem;
use crate::session::state::{Pending, SessionMode};
use crate::session::{ConversationStore, SessionStore};
use crate::wrapper::pty;

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
            // Current wire (`ThreadTokenUsage`): `{ tokenUsage: { total: {…},
            // last: {…}, modelContextWindow } }` where total/last are camelCase
            // `TokenUsageBreakdown`s. `total` is CUMULATIVE across the whole
            // thread; `last` is the most recent request — i.e. what's actually
            // occupying the context window right now. Older builds sent a flat
            // snake_case `usage` object; those spellings are kept as fallbacks.
            let tu = params
                .get("tokenUsage")
                .or_else(|| params.get("usage"))
                .unwrap_or(params);
            let pick = |v: &Value, keys: [&str; 2]| {
                keys.iter().find_map(|k| v.get(*k).and_then(Value::as_u64))
            };
            let total = tu.get("total");
            let last = tu.get("last");
            // Cumulative session totals, for the tokens readout.
            let input = total
                .and_then(|t| pick(t, ["inputTokens", "input_tokens"]))
                .or_else(|| pick(tu, ["input_tokens", "inputTokens"]));
            let output = total
                .and_then(|t| pick(t, ["outputTokens", "output_tokens"]))
                .or_else(|| pick(tu, ["output_tokens", "outputTokens"]));
            // Cache-read subset of the cumulative input — the cost estimate
            // bills it at the (10×-cheaper) cached rate.
            let cached_input = total
                .and_then(|t| pick(t, ["cachedInputTokens", "cached_input_tokens"]))
                .or_else(|| pick(tu, ["cached_input_tokens", "cachedInputTokens"]));
            if input.is_some() || output.is_some() {
                // Context occupancy: the LAST request's total (never the
                // cumulative one — that pins the meter at 100% within a few
                // turns). Only the legacy flat shape falls back to its own
                // total, which was per-turn there.
                let context_tokens = last
                    .and_then(|l| pick(l, ["totalTokens", "total_tokens"]))
                    .or_else(|| {
                        total.is_none().then(|| {
                            pick(tu, ["total_tokens", "totalTokens"])
                                .unwrap_or_else(|| input.unwrap_or(0) + output.unwrap_or(0))
                        })
                    });
                let context_window = [tu, params].iter().find_map(|v| {
                    v.get("modelContextWindow")
                        .or_else(|| v.get("model_context_window"))
                        .and_then(Value::as_u64)
                });
                out.push(AgentUpdate::Usage {
                    model: None,
                    input_tokens: input,
                    output_tokens: output,
                    cached_input_tokens: cached_input,
                    cost_usd: None,
                    context_tokens,
                    context_window,
                });
            }
        }
        // Thread settings changed — by our own `thread/settings/update` (live
        // model switch) or by the user in the TUI (`/model`). Either way the
        // model on the status line follows the thread's truth.
        "thread/settings/updated" => {
            let model = params
                .get("threadSettings")
                .and_then(|s| s.get("model"))
                .and_then(Value::as_str);
            if let Some(m) = model {
                out.push(AgentUpdate::Usage {
                    model: Some(m.to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    cached_input_tokens: None,
                    cost_usd: None,
                    context_tokens: None,
                    context_window: None,
                });
            }
        }
        // Account 5h/7d rate-limit windows (`RateLimitSnapshot`) — same meaning
        // as Claude's statusLine rate_limits, so they land in the same fields.
        "account/rateLimits/updated" => {
            let snap = params.get("rateLimits").unwrap_or(params);
            if let Some(u) = super::rate_limits_from(snap) {
                out.push(u);
            }
        }
        "item/commandExecution/requestApproval" => {
            let cmd = command_text(params);
            out.push(AgentUpdate::PermissionPending {
                id: None, // carried out of band as the JSON-RPC request id
                tool: Some("command".into()),
                summary: cmd,
                raw: params.clone(),
            });
        }
        "item/fileChange/requestApproval" => {
            let path = params
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| {
                    params
                        .get("item")
                        .and_then(|i| i.get("path"))
                        .and_then(Value::as_str)
                })
                .map(str::to_owned);
            out.push(AgentUpdate::PermissionPending {
                id: None,
                tool: Some("file change".into()),
                summary: path,
                raw: params.clone(),
            });
        }
        _ => {}
    }
    out
}

/// Map a started/completed `item` to tool updates. `item/started` emits the
/// [`AgentUpdate::ToolUse`] (so a tool isn't recorded twice) and
/// `item/completed` emits the matching [`AgentUpdate::ToolResult`] — the
/// completed `ThreadItem` carries the output fields (`aggregatedOutput` /
/// `exitCode` / mcp `result`), which never appear on the started one.
/// Assistant text arrives via `item/agentMessage/delta`, so completed
/// agentMessage items are not re-emitted.
fn translate_item(method: &str, item: &Value, out: &mut Vec<AgentUpdate>) {
    let ty = item
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| item.get("itemType").and_then(Value::as_str))
        .unwrap_or("");
    // Plan / todo-list updates arrive as a dedicated item and, unlike tool
    // uses, are meaningful on BOTH item/started and item/completed (the latter
    // carries the final statuses) — last-write-wins, so honor either. The exact
    // item type name isn't nailed down across Codex builds, so accept the
    // plausible spellings and lean on `plan_from_value` to confirm real steps.
    if matches!(ty, "todoList" | "todo_list" | "plan" | "planUpdate") {
        if let Some(plan) = super::plan_from_value(item) {
            out.push(AgentUpdate::Plan(plan));
        }
        return;
    }
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match method {
        "item/started" => translate_item_started(ty, id, item, out),
        "item/completed" => translate_item_completed(ty, id, item, out),
        _ => {}
    }
}

fn translate_item_started(ty: &str, id: String, item: &Value, out: &mut Vec<AgentUpdate>) {
    match ty {
        "commandExecution" => {
            let input = item
                .get("command")
                .cloned()
                .map(|c| json!({ "command": c }))
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse {
                id,
                name: "shell".into(),
                input,
            });
        }
        "fileChange" => {
            out.push(AgentUpdate::ToolUse {
                id,
                name: "apply_patch".into(),
                input: file_change_input(item),
            });
        }
        "mcpToolCall" => {
            let name = item
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| item.get("name").and_then(Value::as_str))
                .unwrap_or("mcp")
                .to_string();
            let input = item
                .get("arguments")
                .or_else(|| item.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        "webSearch" => {
            let input = item
                .get("query")
                .cloned()
                .map(|q| json!({ "query": q }))
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse {
                id,
                name: "web_search".into(),
                input,
            });
        }
        _ => {}
    }
}

/// Map a completed item to the `ToolResult` joined to the started `ToolUse` by
/// item id, normalizing the per-type output fields the same way the rollout
/// path's `function_output_text` does (plain display text + an error flag).
fn translate_item_completed(ty: &str, id: String, item: &Value, out: &mut Vec<AgentUpdate>) {
    // `CommandExecutionStatus` / `PatchApplyStatus` / `McpToolCallStatus`:
    // inProgress | completed | failed (| declined). A decline is surfaced as an
    // error so the card doesn't render as a silent success.
    let failed = matches!(
        item.get("status").and_then(Value::as_str).unwrap_or(""),
        "failed" | "declined"
    );
    match ty {
        "commandExecution" => {
            let mut content = item
                .get("aggregatedOutput")
                .or_else(|| item.get("output"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let exit_code = item.get("exitCode").and_then(Value::as_i64);
            if content.is_empty() {
                if let Some(code) = exit_code {
                    content = format!("exit code {code}");
                }
            }
            out.push(AgentUpdate::ToolResult {
                tool_use_id: id,
                content,
                is_error: failed || exit_code.is_some_and(|c| c != 0),
            });
        }
        "fileChange" => {
            // Summarize the patch outcome per file, mirroring the rollout
            // path's "Success. Updated the following files:\nM path" shape.
            let lines: Vec<String> = item
                .get("changes")
                .and_then(Value::as_array)
                .map(|changes| {
                    changes
                        .iter()
                        .filter_map(|c| {
                            let path = c.get("path").and_then(Value::as_str)?;
                            let mark = match c.get("kind").and_then(Value::as_str) {
                                Some("add") => "A",
                                Some("delete") => "D",
                                _ => "M",
                            };
                            Some(format!("{mark} {path}"))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let content = if failed {
                format!("Patch failed:\n{}", lines.join("\n"))
            } else if lines.is_empty() {
                "Success.".to_string()
            } else {
                format!(
                    "Success. Updated the following files:\n{}",
                    lines.join("\n")
                )
            };
            out.push(AgentUpdate::ToolResult {
                tool_use_id: id,
                content,
                is_error: failed,
            });
        }
        "mcpToolCall" => {
            let (content, is_error) = mcp_result_text(item);
            out.push(AgentUpdate::ToolResult {
                tool_use_id: id,
                content,
                is_error: failed || is_error,
            });
        }
        "webSearch" => {
            // No result payload on the wire — the empty result still marks the
            // call complete in the GUI card.
            out.push(AgentUpdate::ToolResult {
                tool_use_id: id,
                content: String::new(),
                is_error: false,
            });
        }
        _ => {}
    }
}

/// The ToolUse input for a `fileChange` item. Modern wire (`FileUpdateChange`):
/// `changes: [{ path, kind, diff }]` where `diff` is that file's unified patch.
/// Surface the first path as the headline `path`, the concatenated patches as
/// `diff` (what the GUI's inline diff renders), and the raw `changes` for
/// multi-file awareness. Older builds that sent a bare `path` still work.
fn file_change_input(item: &Value) -> Value {
    if let Some(changes) = item.get("changes").and_then(Value::as_array) {
        let path = changes
            .iter()
            .find_map(|c| c.get("path").and_then(Value::as_str));
        let diff = changes
            .iter()
            .filter_map(|c| c.get("diff").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let mut input = json!({ "path": path, "changes": changes });
        if !diff.is_empty() {
            input["diff"] = json!(diff);
        }
        return input;
    }
    json!({ "path": item.get("path").cloned().unwrap_or(Value::Null) })
}

/// Flatten a completed `mcpToolCall`'s `result` / `error` into display text +
/// an error flag. The result is an MCP `CallToolResult`: `content` is a list of
/// content items whose text parts carry `text`; `structuredContent` is the
/// typed alternative.
fn mcp_result_text(item: &Value) -> (String, bool) {
    if let Some(msg) = item
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
    {
        return (msg.to_string(), true);
    }
    let Some(result) = item.get("result").filter(|r| !r.is_null()) else {
        return (String::new(), false);
    };
    if let Some(parts) = result.get("content").and_then(Value::as_array) {
        let text = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return (text, false);
        }
    }
    if let Some(sc) = result.get("structuredContent").filter(|v| !v.is_null()) {
        return (sc.to_string(), false);
    }
    (result.to_string(), false)
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

/// An approval request parked while the user decides: the JSON-RPC request id
/// (what the decision must answer) together with the display fields, so a
/// request queued *behind* the surfaced one can be re-surfaced when it reaches
/// the head of the FIFO. Mirrors `claude_stream::ParkedCanUse`.
#[derive(Debug)]
struct ParkedApproval {
    id: Value,
    tool: Option<String>,
    summary: Option<String>,
    raw: Value,
}

/// Surface a parked approval as the session's pending card. The store's pending
/// is a single slot, so only the FIFO *head* is ever displayed — later requests
/// wait parked and are re-surfaced by [`resolve_approval`] when the head is
/// answered, keeping the displayed card and the answered request in sync.
fn surface_approval(
    store: &SessionStore,
    session_id: &str,
    cur_mode: &mut SessionMode,
    parked: &ParkedApproval,
) {
    store.set_managed_mode(
        session_id,
        SessionMode::Approval,
        Some(Pending::Approval {
            tool: parked.tool.clone(),
            summary: parked.summary.clone(),
            raw: parked.raw.clone(),
        }),
    );
    *cur_mode = SessionMode::Approval;
}

/// Answer the FIFO head of the parked approvals with the user's decision, then
/// surface the next parked request (parallel tool calls can park several) or —
/// when the queue is empty — return the session to Responding. Answering the
/// head (the request `surface_approval` displayed) is what guarantees the user
/// approves the card they actually saw.
fn resolve_approval(
    store: &SessionStore,
    session_id: &str,
    out_tx: &mpsc::UnboundedSender<Value>,
    pending_approvals: &mut VecDeque<ParkedApproval>,
    cur_mode: &mut SessionMode,
    approve: bool,
) {
    let Some(parked) = pending_approvals.pop_front() else {
        tracing::debug!(session = %session_id, "codex: decision with no parked approval — dropped");
        return;
    };
    let result = json!({ "decision": if approve { "accept" } else { "decline" } });
    let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": parked.id, "result": result }));
    match pending_approvals.front() {
        Some(next) => surface_approval(store, session_id, cur_mode, next),
        None => {
            store.set_managed_mode(session_id, SessionMode::Responding, None);
            *cur_mode = SessionMode::Responding;
        }
    }
}

// ── Model listing ────────────────────────────────────────────────────────────

/// List the models Codex offers (cached; see [`super::cached_or_fetch`]).
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    super::cached_or_fetch(format!("codex:{bin}"), fetch_models(bin, cwd)).await
}

/// Live query: boot a throwaway `codex app-server`, `initialize`, ask for the
/// catalog via `model/list`, then drop the process. Hidden models are skipped;
/// the rest map to the picker with their `displayName` as label and the
/// server-flagged default marked.
async fn fetch_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
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
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
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
#[allow(clippy::too_many_arguments)]
pub fn spawn_session(
    store: SessionStore,
    conv: ConversationStore,
    session_id: String,
    cwd: String,
    model: Option<String>,
    effort: Option<String>,
    bin: String,
    yolo: bool,
    headless: bool,
    resume_thread: Option<String>,
    facade: Facade,
) {
    tokio::spawn(async move {
        if let Err(err) = run_session(
            &store,
            &conv,
            &session_id,
            &cwd,
            model,
            effort,
            &bin,
            yolo,
            headless,
            resume_thread,
            &facade,
        )
        .await
        {
            tracing::warn!(?err, session = %session_id, "codex managed session ended with error");
        }
        store.deregister_managed(&session_id);
        conv.forget(&session_id);
    });
}

/// A connected JSON-RPC-over-WebSocket stream to a session's `codex app-server`.
type CodexWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Start `codex app-server --listen ws://…` for this session on a free loopback
/// port and connect the ws client. Returns the child, the ws stream, and the ws
/// URL (the TUI attaches to it via `--remote`). An error means the ws path is
/// unavailable for this Codex build — the caller falls back to the rollout hybrid.
async fn start_appserver(
    session_id: &str,
    cwd: &str,
    bin: &str,
    facade: &Facade,
    // Headless only: (model, effort) config overrides. The app-server is the
    // thread's creator there, so what hybrid mode sets on the TUI process goes
    // on the server instead. `None` in hybrid mode — the TUI owns the config.
    overrides: Option<(Option<String>, Option<String>)>,
) -> anyhow::Result<(tokio::process::Child, CodexWs, String)> {
    // Each managed session gets its own app-server, so threads/approvals are
    // isolated per pane.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("reserving a port for codex app-server")?;
        listener.local_addr()?.port()
    };
    let ws_url = format!("ws://127.0.0.1:{port}");
    let http_base = format!("http://127.0.0.1:{port}");

    let mut cmd = Command::new(bin);
    cmd.arg("app-server").arg("--listen").arg(&ws_url);
    // Register the workspacer MCP facade (supervisors) as a config override so
    // `codex app-server` exposes its tools.
    if let Some(mcp_url) = &facade.mcp_url {
        cmd.arg("-c")
            .arg(format!("mcp_servers.workspacer.url=\"{mcp_url}\""));
    }
    if let Some((model, effort)) = overrides {
        if let Some(m) = model {
            cmd.arg("-c").arg(format!("model={}", Value::String(m)));
        }
        if let Some(e) = effort {
            cmd.arg("-c")
                .arg(format!("model_reasoning_effort={}", Value::String(e)));
        }
    }
    // AskUserQuestion: the daemon serves a per-session MCP endpoint that parks
    // a structured question for the GUI and blocks until /answer resolves it —
    // Codex's stand-in for Claude's native tool. The generous tool timeout is
    // the point: a question can legitimately wait on the user for hours.
    if let Some(api_base) = crate::daemon::API_BASE.get() {
        cmd.arg("-c").arg(format!(
            "mcp_servers.workspacer_ask.url=\"{api_base}/mcp/ask/{session_id}\""
        ));
        cmd.arg("-c")
            .arg("mcp_servers.workspacer_ask.tool_timeout_sec=21600");
    }
    let child = cmd
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} app-server --listen {ws_url}`"))?;

    // Wait for the server's HTTP `/readyz` before opening the ws client.
    wait_ready(&http_base).await?;

    let (ws, _resp) = connect_async(&ws_url)
        .await
        .with_context(|| format!("connecting to codex app-server at {ws_url}"))?;
    Ok((child, ws, ws_url))
}

/// Fallback when the app-server ws path is unavailable: run the plain `codex` TUI
/// in a PTY (the Term view) and tail its rollout transcript for the GUI — the
/// same mechanism used on Windows. Less rich than the RPC path (approvals happen
/// in the Term; text lands in rollout-sized chunks rather than token deltas) but
/// robust and version-independent, so a Codex CLI that changed `app-server` /
/// `--remote` still gives a working pane instead of an empty one.
#[allow(clippy::too_many_arguments)]
async fn run_rollout_fallback(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    cwd: &str,
    model: Option<String>,
    effort: Option<String>,
    bin: &str,
    yolo: bool,
    // Prompts the user sent on the ws path that were never delivered (buffered
    // while waiting for the TUI's thread). Already echoed into the conversation
    // store and already instruction-wrapped — deliver them to the fallback TUI
    // instead of silently dropping a message the GUI shows as sent.
    initial_prompts: Vec<String>,
) -> anyhow::Result<()> {
    // Plain codex TUI (no `--remote`): it owns its own session and writes a rollout.
    let mut argv = vec![bin.to_string()];
    if let Some(m) = &model {
        argv.push("-c".to_string());
        argv.push(format!("model={}", Value::String(m.clone())));
    }
    if let Some(e) = &effort {
        argv.push("-c".to_string());
        argv.push(format!(
            "model_reasoning_effort={}",
            Value::String(e.clone())
        ));
    }
    if yolo {
        argv.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }
    let tui = super::spawn_attach_pty(store, session_id, &argv, cwd)
        .context("spawning fallback codex TUI")?;
    // Drive the GUI conversation from the rollout transcript.
    super::codex_rollout::spawn_tailer(
        store.clone(),
        conv.clone(),
        session_id.to_string(),
        cwd.to_string(),
    );

    // GUI-composer prompts arrive here; write them into the TUI's PTY (there's no
    // RPC channel in this mode — approvals and everything else happen in the Term).
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let mut tui_check = tokio::time::interval(std::time::Duration::from_secs(2));

    // Replay the undelivered ws-path prompts. They were already pushed into the
    // conversation store when first sent, so only the PTY write happens here. A
    // short grace period lets the fresh TUI bring up its composer (and enable
    // bracketed paste) before input lands — best-effort, like all PTY input.
    if !initial_prompts.is_empty() {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        for text in &initial_prompts {
            write_prompt(&tui, text).await;
        }
    }

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(text) => {
                    conv.push(session_id, vec![ConversationItem::UserMessage { text: text.clone(), timestamp: None }]);
                    write_prompt(&tui, &text).await;
                }
                None => break, // managed input dropped → terminated
            },
            _ = tui_check.tick() => {
                if pty::has_exited(&tui) {
                    tracing::info!(session = %session_id, "codex fallback TUI exited; tearing down");
                    break;
                }
            }
        }
    }
    let _ = pty::signal_child(&tui, Signal::Sigkill);
    Ok(())
}

/// Write one prompt into a fallback TUI's PTY as a bracketed paste + Enter
/// (same as the Claude PTY path), so the TUI submits it instead of folding the
/// CR into the paste.
async fn write_prompt(tui: &Arc<pty::PtyHandle>, text: &str) {
    let body = text.trim_end_matches(['\r', '\n']);
    let mut bytes = b"\x1b[200~".to_vec();
    bytes.extend_from_slice(body.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~\r");
    let _ = pty::write_bytes(tui, &bytes).await;
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    cwd: &str,
    model: Option<String>,
    effort: Option<String>,
    bin: &str,
    yolo: bool,
    headless: bool,
    // A prior life's app-server thread to `thread/resume` instead of starting
    // fresh — headless only (the TUI can't rejoin an arbitrary thread).
    resume_thread: Option<String>,
    facade: &Facade,
) -> anyhow::Result<()> {
    // Start the app-server + ws client. If that fails, the ws path is unavailable
    // for this Codex build (e.g. a version that dropped/renamed `app-server
    // --listen`, or won't bind/handshake) — degrade to the rollout hybrid rather
    // than leave the pane dead. The RPC path is preferred; this is the safety net.
    // Headless (stream-transport) sessions have no fallback: the rollout hybrid
    // is built around a TUI PTY, which is exactly what headless promises not to
    // spawn — so its unavailability is a hard error, like the Claude stream
    // driver.
    // For headless the app-server is the thread's creator, so the model/effort
    // overrides that hybrid mode sets on the TUI go on the server instead.
    let overrides = headless.then(|| (model.clone(), effort.clone()));
    let (mut child, ws_stream, ws_url) = match start_appserver(
        session_id, cwd, bin, facade, overrides,
    )
    .await
    {
        Ok(t) => t,
        Err(err) if headless => return Err(err),
        Err(err) => {
            tracing::warn!(?err, session = %session_id, "codex app-server ws path unavailable — falling back to the rollout hybrid (Term + transcript-tailed GUI)");
            return run_rollout_fallback(
                store,
                conv,
                session_id,
                cwd,
                model,
                effort,
                bin,
                yolo,
                Vec::new(),
            )
            .await;
        }
    };
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Serialize all outgoing JSON-RPC through one task that owns the ws sink, so
    // the several send sites (handshake, turns, approval replies) never contend
    // for the writer. Dropping `out_tx` (on return) closes the sink.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(v) = out_rx.recv().await {
            if ws_write.send(Message::Text(v.to_string())).await.is_err() {
                break;
            }
        }
        let _ = ws_write.close().await;
    });

    // The app server requires an `initialize` handshake before any other request.
    let _ = out_tx.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientInfo": { "name": "workspacer", "version": "0.1" } }
    }));

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);
    // Live model/effort switch (POST /sessions/:id/model): applied to the
    // running thread via `thread/settings/update`, so subsequent turns — from
    // the GUI or the TUI — use the new model. No restart, thread untouched.
    let (mtx, mut mrx) = mpsc::unbounded_channel::<crate::session::ModelSwitch>();
    store.register_managed_model_switch(session_id, mtx);
    // Structural interrupt (POST /sessions/:id/signal SIGINT): `turn/interrupt`
    // stops the running turn while keeping the thread alive — same semantics as
    // the Claude stream driver's `interrupt` control request, and it works for
    // clients with no PTY to Ctrl-C into (mobile remote, the inbox, wks-tui).
    let (itx, mut irx) = mpsc::unbounded_channel::<()>();
    store.register_managed_interrupt(session_id, itx);
    // Approval policy, live-switchable via `/permission-mode`: the adapter
    // mediates every approval request on this ws path, so flipping the flag
    // takes effect on the next request without touching the session. A
    // yolo-spawned TUI bypasses approvals at the source, though — that
    // direction needs a restart (`spawned_yolo` records it). Headless never
    // spawns anything in bypass mode — yolo there is pure adapter mediation —
    // so ask↔yolo stay live-switchable in BOTH directions (spawned_yolo=false).
    let yolo_live = Arc::new(AtomicBool::new(yolo));
    store.register_managed_yolo(session_id, yolo_live.clone(), yolo && !headless);

    // Hybrid: the native TUI OWNS the thread — bare `codex --remote` creates and
    // runs it (a real, "running", resumable rollout) — then we rejoin it over
    // RPC (below) to drive the GUI, exactly the validated owner/rejoiner split.
    // The reverse (RPC `thread/start` here + TUI `resume`) fails because a
    // just-started thread has no rollout yet: "no rollout found for thread id …".
    // Model / YOLO are set on the thread's creator (the TUI) as config
    // overrides. Kept so we can kill it when the session ends.
    // Headless (stream transport): no TUI at all — this client creates the
    // thread itself via `thread/start` below, the GUI is the only surface.
    let tui_pty = if headless {
        None
    } else {
        spawn_codex_tui(
            store,
            session_id,
            cwd,
            bin,
            &ws_url,
            model.as_deref(),
            effort.as_deref(),
            yolo,
        )
    };

    let mut thread_id: Option<String> = resume_thread;
    // Whether our `thread/resume` has actually taken (we're receiving the thread's
    // live stream). The first resume can land before the TUI's thread is "running"
    // and fail, so we keep retrying until this flips true.
    let mut subscribed = false;
    let mut pending_prompts: Vec<String> = Vec::new();
    // id 1 = initialize, 2 = thread/resume, 100 = thread/loaded/list poll,
    // 101 = thread/start (headless); the user's turns take ids from 3 up.
    let mut req_id: u64 = 2;
    // Headless bootstrap: no TUI to discover — this client starts the thread.
    // Sent right behind `initialize` (the out task serializes them in order);
    // `handle_message` picks the thread id off the id-101 response (or the
    // `thread/started` notification) and flushes any early prompts.
    // Resume: rejoin the prior life's persisted thread instead — the id-2
    // response handler flips `subscribed`, exactly like a hybrid rejoin. Its
    // conversation is pre-seeded from the rollout in spawn.rs, so the GUI
    // shows the history the app-server already has.
    if headless {
        match &thread_id {
            Some(tid) => {
                let _ = out_tx.send(json!({
                    "jsonrpc": "2.0", "id": 2, "method": "thread/resume",
                    "params": { "threadId": tid }
                }));
            }
            None => {
                let _ = out_tx.send(json!({
                    "jsonrpc": "2.0", "id": 101, "method": "thread/start",
                    "params": { "cwd": cwd }
                }));
            }
        }
    }
    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // OpenAI's wire never carries dollars; the token totals are cumulative,
    // so the status line prices them via the pricing table.
    acc.estimate_costs();
    // Codex's usage events never carry the model id — name it from the spawn
    // setting so the status line isn't blank (and the window-table fallback has
    // something to key on if the event omits `modelContextWindow`).
    acc.seed_model(model.as_deref());
    // Approval requests awaiting the user's decision (non-YOLO), FIFO — the
    // JSON-RPC request id plus the display fields. A queue (not a single slot)
    // so two requests arriving before the user answers don't drop the first and
    // deadlock the agent; only the head is surfaced as the store's pending, and
    // a decision answers that head (see `resolve_approval`). YOLO answers inline
    // and never parks one here.
    let mut pending_approvals: VecDeque<ParkedApproval> = VecDeque::new();
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = facade.instructions.clone();
    // A model/effort switch requested before the thread is joined can't be sent
    // yet (there's no thread to update), but the HTTP call already returned 200 —
    // stash it here and apply it the instant we subscribe rather than dropping it.
    let mut pending_switch: Option<crate::session::ModelSwitch> = None;
    // Poll `thread/loaded/list` until the TUI's thread appears, then rejoin it —
    // retrying the resume until we're actually subscribed. Bounded by a deadline
    // so a TUI that never creates a thread (or died at startup) can't busy-poll
    // for the daemon's life.
    let mut discover = tokio::time::interval(std::time::Duration::from_millis(300));
    // If we can't rejoin the TUI's thread within this window, the ws path is up
    // but its thread protocol drifted — fall back to the rollout hybrid rather
    // than sit on an empty GUI. Generous enough for a slow TUI cold-start.
    let discover_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut needs_fallback = false;
    // Watch the TUI (the thread owner) for death: if it exits while the app-server
    // lives, the session is dead and must tear down rather than run against a
    // dead thread. try_wait is cheap; a coarse tick is fine.
    let mut tui_check = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        tokio::select! {
            msg = ws_read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    // One ws frame may carry one JSON-RPC object or several
                    // newline-delimited ones; handle each line independently.
                    for line in text.split('\n') {
                        let line = line.trim();
                        if line.is_empty() { continue; }
                        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
                        handle_message(
                            &value, store, conv, session_id, &out_tx,
                            &mut thread_id, &mut subscribed, &mut pending_prompts, &mut req_id,
                            &mut cur_mode, &mut acc, &yolo_live, &mut pending_approvals,
                            &mut pending_switch, headless,
                        );
                    }
                }
                Some(Ok(Message::Close(_))) | None => break, // server gone
                Some(Ok(_)) => {} // ping/pong/binary — ignore
                Some(Err(err)) => return Err(err.into()),
            },
            // Drive discovery + rejoin until we're subscribed to the TUI's thread:
            // ask which threads are loaded (there's only ever one on this
            // per-session app-server) and (re)send `thread/resume` for it. The
            // first resume can precede the thread becoming "running", so we retry.
            // TUI-thread discovery is hybrid-only; headless started its own
            // thread and just waits for the id-101 response.
            _ = discover.tick(), if !subscribed && !needs_fallback && !headless => {
                if tokio::time::Instant::now() >= discover_deadline {
                    tracing::warn!(session = %session_id, "codex: couldn't rejoin the TUI thread in time; falling back to the rollout hybrid");
                    needs_fallback = true;
                    break;
                } else {
                    match &thread_id {
                        None => {
                            let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": 100, "method": "thread/loaded/list", "params": {} }));
                        }
                        Some(tid) => {
                            let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "thread/resume", "params": { "threadId": tid } }));
                        }
                    }
                }
            },
            _ = tui_check.tick(), if tui_pty.is_some() => {
                if tui_pty.as_ref().is_some_and(|h| pty::has_exited(h)) {
                    tracing::info!(session = %session_id, "codex TUI exited; tearing down session");
                    break;
                }
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
                            send_turn(&out_tx, req_id, tid, &sent);
                        }
                        // Thread not open yet — buffer the (already-wrapped) prompt.
                        None => pending_prompts.push(sent),
                    }
                }
                None => break, // managed input dropped → terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    resolve_approval(store, session_id, &out_tx, &mut pending_approvals, &mut cur_mode, approve);
                }
                None => break,
            },
            intr = irx.recv() => match intr {
                Some(()) => {
                    // Only meaningful once we're subscribed to the TUI's
                    // thread; before that there is no turn to interrupt.
                    if let Some(tid) = &thread_id {
                        if subscribed {
                            req_id += 1;
                            let _ = out_tx.send(json!({
                                "jsonrpc": "2.0", "id": req_id, "method": "turn/interrupt",
                                "params": { "threadId": tid }
                            }));
                        }
                    }
                }
                None => break,
            },
            switch = mrx.recv() => match switch {
                Some(sw) => {
                    // Settings live on the thread; partial params leave the
                    // rest untouched. Confirmation arrives as the
                    // `thread/settings/updated` notification (handled in
                    // `translate`), which refreshes the status-line model.
                    match &thread_id {
                        Some(tid) if subscribed => {
                            send_model_switch(&out_tx, &mut req_id, tid, &sw);
                        }
                        // Not joined yet — stash it; `handle_message` applies it
                        // the moment the resume subscribes us to the thread.
                        _ => {
                            tracing::debug!(session = %session_id, "model switch requested before thread join — queued");
                            merge_pending_switch(&mut pending_switch, sw);
                        }
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

    // Tear down the ws attempt (app-server + the `--remote` TUI) before any
    // fallback, so the rollout path starts from a clean slate.
    let _ = child.start_kill();
    if let Some(handle) = &tui_pty {
        let _ = pty::signal_child(handle, Signal::Sigkill);
    }

    // The thread protocol drifted (ws up, but we never rejoined): degrade to the
    // rollout hybrid so the pane still works — carrying any prompts that were
    // buffered while waiting for the thread, so a message the GUI already shows
    // as sent still reaches the agent.
    if needs_fallback {
        return run_rollout_fallback(
            store,
            conv,
            session_id,
            cwd,
            model,
            effort,
            bin,
            yolo,
            pending_prompts,
        )
        .await;
    }
    Ok(())
}

/// Launch the native Codex TUI in a PTY, connected over `--remote` to this
/// session's app-server. The TUI creates and owns the session's thread; the RPC
/// client rejoins it (see `run_session`), so the Term view and the RPC-driven GUI
/// are two views of one conversation. Best-effort: if it can't start, the GUI
/// still works and the Term is empty.
#[allow(clippy::too_many_arguments)]
fn spawn_codex_tui(
    store: &SessionStore,
    session_id: &str,
    cwd: &str,
    bin: &str,
    ws_url: &str,
    model: Option<&str>,
    effort: Option<&str>,
    yolo: bool,
) -> Option<Arc<pty::PtyHandle>> {
    let mut argv = vec![bin.to_string(), "--remote".to_string(), ws_url.to_string()];
    // Model / reasoning effort are config overrides on the thread's creator;
    // YOLO bypasses the approval/sandbox prompts so the shared thread doesn't
    // block on them.
    if let Some(m) = model {
        argv.push("-c".to_string());
        argv.push(format!("model={}", Value::String(m.to_string())));
    }
    if let Some(e) = effort {
        argv.push("-c".to_string());
        argv.push(format!(
            "model_reasoning_effort={}",
            Value::String(e.to_string())
        ));
    }
    if yolo {
        argv.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }
    match super::spawn_attach_pty(store, session_id, &argv, cwd) {
        Ok(h) => Some(h),
        Err(err) => {
            tracing::warn!(?err, session = %session_id, "codex TUI (--remote) failed; Term view unavailable");
            None
        }
    }
}

/// Poll the app-server's HTTP `/readyz` until it answers (or give up after ~10s).
async fn wait_ready(http_base: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(resp) = client.get(format!("{http_base}/readyz")).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    anyhow::bail!("codex app-server did not become ready in time")
}

#[allow(clippy::too_many_arguments)]
fn handle_message(
    value: &Value,
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    out_tx: &mpsc::UnboundedSender<Value>,
    thread_id: &mut Option<String>,
    subscribed: &mut bool,
    pending_prompts: &mut Vec<String>,
    req_id: &mut u64,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    yolo: &AtomicBool,
    pending_approvals: &mut VecDeque<ParkedApproval>,
    pending_switch: &mut Option<crate::session::ModelSwitch>,
    headless: bool,
) {
    // A response to one of our requests:
    //  - `thread/loaded/list` (id=100): `result.data` lists thread ids loaded in
    //    this per-session app-server — the one the TUI created. Rejoin it
    //    (`thread/resume`), which for a *running* thread subscribes us to its live
    //    stream so the GUI mirrors the TUI.
    //  - `thread/resume` (id=2): success means we're subscribed; an error (e.g. the
    //    thread wasn't "running" yet) is logged and the discover loop retries.
    //  - `thread/start` (id=101, headless only): we created the thread, so its
    //    success is both the thread id AND the subscription (the starter gets the
    //    live stream). Its failure is fatal for the pane — there is no TUI thread
    //    to fall back to — so surface it in the conversation.
    if value.get("id").is_some() && (value.get("result").is_some() || value.get("error").is_some())
    {
        let id = value.get("id").and_then(Value::as_u64);
        if id == Some(2) {
            if let Some(err) = value.get("error") {
                tracing::warn!(session = %session_id, error = %err, "codex thread/resume failed; retrying");
            } else {
                *subscribed = true;
                tracing::info!(session = %session_id, thread = ?thread_id, "codex: rejoined thread — GUI stream subscribed");
                if let (Some(sw), Some(tid)) = (pending_switch.take(), thread_id.as_deref()) {
                    send_model_switch(out_tx, req_id, tid, &sw);
                }
            }
        }
        if id == Some(101) {
            if let Some(err) = value.get("error") {
                tracing::error!(session = %session_id, error = %err, "codex thread/start failed — headless session has no thread");
                apply_updates(
                    store,
                    conv,
                    session_id,
                    vec![AgentUpdate::Error(format!(
                        "codex thread/start failed: {err}"
                    ))],
                    cur_mode,
                    acc,
                );
            } else if let Some(tid) = thread_id_of(value.get("result")) {
                *thread_id = Some(tid.clone());
                *subscribed = true;
                super::codex_rollout::record_thread(session_id, &tid);
                tracing::info!(session = %session_id, thread = %tid, "codex: headless thread started");
                if let Some(sw) = pending_switch.take() {
                    send_model_switch(out_tx, req_id, tid.as_str(), &sw);
                }
                for text in std::mem::take(pending_prompts) {
                    *req_id += 1;
                    send_turn(out_tx, *req_id, &tid, &text);
                }
            }
            return;
        }
        if thread_id.is_none() {
            if let Some(tid) = value
                .get("result")
                .and_then(|r| r.get("data"))
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
            {
                *thread_id = Some(tid.to_string());
                super::codex_rollout::record_thread(session_id, tid);
                tracing::info!(session = %session_id, thread = %tid, "codex: discovered TUI thread, resuming");
                let _ = out_tx.send(json!({
                    "jsonrpc": "2.0", "id": 2, "method": "thread/resume",
                    "params": { "threadId": tid }
                }));
                // Flush any prompts that arrived before the thread was found.
                for text in std::mem::take(pending_prompts) {
                    *req_id += 1;
                    send_turn(out_tx, *req_id, tid, &text);
                }
            }
        }
        return;
    }

    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = value.get("params").cloned().unwrap_or(Value::Null);

    // Headless belt-and-braces: if the id-101 response shape didn't carry the
    // thread id (wire drift), the `thread/started` notification names it.
    // Hybrid must NOT take this path — there the notification just means the
    // TUI's thread exists, not that we're subscribed to its stream.
    if headless && thread_id.is_none() && method == "thread/started" {
        if let Some(tid) = thread_id_of(Some(&params)) {
            *thread_id = Some(tid.clone());
            *subscribed = true;
            super::codex_rollout::record_thread(session_id, &tid);
            tracing::info!(session = %session_id, thread = %tid, "codex: headless thread started (via notification)");
            if let Some(sw) = pending_switch.take() {
                send_model_switch(out_tx, req_id, tid.as_str(), &sw);
            }
            for text in std::mem::take(pending_prompts) {
                *req_id += 1;
                send_turn(out_tx, *req_id, &tid, &text);
            }
        }
    }

    // Receiving any turn/item stream event proves the resume took, even if we
    // missed its response — stop the discover/retry loop.
    if !*subscribed && (method.starts_with("turn/") || method.starts_with("item/")) {
        *subscribed = true;
        if let (Some(sw), Some(tid)) = (pending_switch.take(), thread_id.as_deref()) {
            send_model_switch(out_tx, req_id, tid, &sw);
        }
    }

    let mut updates = translate(method, &params);
    // Approval cards must NOT flow through `apply_updates`: the store's pending
    // is a single slot, so a second request would overwrite the displayed card
    // while the decision channel answers FIFO — the user could approve a command
    // whose card they never saw. Strip the card fields out here and park them
    // WITH the request id below; only the queue head is ever surfaced.
    let mut approval_card: Option<(Option<String>, Option<String>, Value)> = None;
    updates.retain(|u| match u {
        AgentUpdate::PermissionPending {
            tool, summary, raw, ..
        } => {
            approval_card = Some((tool.clone(), summary.clone(), raw.clone()));
            false
        }
        _ => true,
    });
    if !updates.is_empty() {
        apply_updates(store, conv, session_id, updates, cur_mode, acc);
    }

    // Server→client *requests* (they carry an id) must be answered or the agent
    // blocks. For an approval request: YOLO accepts inline; otherwise we park the
    // request id + card and surface the FIFO head, so the user's /approve
    // decision answers the request that's actually on screen (see
    // `resolve_approval`, called from the decision branch in run_session).
    if value.get("id").is_some() && method.ends_with("/requestApproval") {
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        if yolo.load(Ordering::Relaxed) {
            let _ = out_tx
                .send(json!({ "jsonrpc": "2.0", "id": id, "result": { "decision": "accept" } }));
        } else {
            let (tool, summary, raw) = approval_card.take().unwrap_or((None, None, Value::Null));
            pending_approvals.push_back(ParkedApproval {
                id,
                tool,
                summary,
                raw,
            });
            if pending_approvals.len() == 1 {
                surface_approval(store, session_id, cur_mode, &pending_approvals[0]);
            }
        }
    }
}

/// The thread id wherever a `thread/start` result or `thread/started`
/// notification carries it: `{threadId}`, `{thread_id}`, or `{thread: {id}}`.
fn thread_id_of(v: Option<&Value>) -> Option<String> {
    let v = v?;
    v.get("threadId")
        .or_else(|| v.get("thread_id"))
        .or_else(|| v.get("thread").and_then(|t| t.get("id")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn send_turn(out_tx: &mpsc::UnboundedSender<Value>, id: u64, thread_id: &str, text: &str) {
    let _ = out_tx.send(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "turn/start",
        "params": { "threadId": thread_id, "input": [ { "type": "text", "text": text } ] }
    }));
}

/// Push a `thread/settings/update` for a live model/effort switch. Partial params
/// leave the untouched setting as is; confirmation returns via the
/// `thread/settings/updated` notification.
fn send_model_switch(
    out_tx: &mpsc::UnboundedSender<Value>,
    req_id: &mut u64,
    thread_id: &str,
    switch: &crate::session::ModelSwitch,
) {
    let mut params = json!({ "threadId": thread_id });
    if let Some(m) = &switch.model {
        params["model"] = json!(m);
    }
    if let Some(e) = &switch.effort {
        params["effort"] = json!(e);
    }
    *req_id += 1;
    let _ = out_tx.send(json!({
        "jsonrpc": "2.0",
        "id": *req_id,
        "method": "thread/settings/update",
        "params": params,
    }));
}

/// Fold a new switch into a stashed one (or start one), so a burst of switches
/// requested before the thread joins collapses to the latest value per field
/// rather than dropping all but one.
fn merge_pending_switch(
    pending: &mut Option<crate::session::ModelSwitch>,
    switch: crate::session::ModelSwitch,
) {
    match pending {
        Some(existing) => {
            if switch.model.is_some() {
                existing.model = switch.model;
            }
            if switch.effort.is_some() {
                existing.effort = switch.effort;
            }
        }
        None => *pending = Some(switch),
    }
}

/// Write one JSON-RPC message as a single newline-delimited line. Used by the
/// stdio-based `list_models` handshake (a throwaway `codex app-server`).
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
        assert_eq!(
            translate("turn/started", &json!({})),
            vec![AgentUpdate::Busy]
        );
    }

    #[test]
    fn turn_completed_is_idle() {
        assert_eq!(
            translate("turn/completed", &json!({})),
            vec![AgentUpdate::Idle]
        );
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
    fn item_completed_emits_tool_result_not_a_second_tool_use() {
        let p = json!({ "item": { "type": "commandExecution", "id": "i1", "command": "ls",
            "status": "completed", "aggregatedOutput": "a.txt\nb.txt\n", "exitCode": 0 } });
        assert_eq!(
            translate("item/completed", &p),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "i1".into(),
                content: "a.txt\nb.txt\n".into(),
                is_error: false,
            }]
        );
    }

    #[test]
    fn completed_command_with_nonzero_exit_is_error() {
        let p = json!({ "item": { "type": "commandExecution", "id": "i2", "command": "false",
            "status": "failed", "exitCode": 1 } });
        assert_eq!(
            translate("item/completed", &p),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "i2".into(),
                content: "exit code 1".into(),
                is_error: true,
            }]
        );
    }

    #[test]
    fn file_change_started_carries_paths_and_diff() {
        // Modern `FileChangeThreadItem`: no top-level path — the files live in
        // `changes: [{ path, kind, diff }]`.
        let p = json!({ "item": { "type": "fileChange", "id": "i3", "status": "inProgress",
            "changes": [
                { "path": "src/a.rs", "kind": "update", "diff": "@@ -1 +1 @@\n-old\n+new" },
                { "path": "src/b.rs", "kind": "add", "diff": "@@ -0,0 +1 @@\n+hello" }
            ] } });
        let updates = translate("item/started", &p);
        assert_eq!(updates.len(), 1);
        match &updates[0] {
            AgentUpdate::ToolUse { id, name, input } => {
                assert_eq!(id, "i3");
                assert_eq!(name, "apply_patch");
                assert_eq!(input["path"], "src/a.rs");
                let diff = input["diff"].as_str().unwrap();
                assert!(diff.contains("-old") && diff.contains("+hello"));
                assert_eq!(input["changes"].as_array().unwrap().len(), 2);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn file_change_completed_summarizes_files_and_flags_decline() {
        let changes = json!([
            { "path": "src/a.rs", "kind": "update", "diff": "@@\n+x" },
            { "path": "src/b.rs", "kind": "add", "diff": "@@\n+y" }
        ]);
        let ok = json!({ "item": { "type": "fileChange", "id": "i4", "status": "completed",
            "changes": changes } });
        assert_eq!(
            translate("item/completed", &ok),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "i4".into(),
                content: "Success. Updated the following files:\nM src/a.rs\nA src/b.rs".into(),
                is_error: false,
            }]
        );
        let declined = json!({ "item": { "type": "fileChange", "id": "i5", "status": "declined",
            "changes": changes } });
        match &translate("item/completed", &declined)[0] {
            AgentUpdate::ToolResult { is_error, .. } => assert!(is_error),
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn mcp_tool_call_completed_maps_result_text_and_error() {
        let ok = json!({ "item": { "type": "mcpToolCall", "id": "m1", "status": "completed",
            "server": "workspacer", "tool": "list_agents", "arguments": {},
            "result": { "content": [ { "type": "text", "text": "3 agents" } ] } } });
        assert_eq!(
            translate("item/completed", &ok),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "m1".into(),
                content: "3 agents".into(),
                is_error: false,
            }]
        );
        let err = json!({ "item": { "type": "mcpToolCall", "id": "m2", "status": "failed",
            "server": "workspacer", "tool": "list_agents", "arguments": {},
            "error": { "message": "server unavailable" } } });
        assert_eq!(
            translate("item/completed", &err),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "m2".into(),
                content: "server unavailable".into(),
                is_error: true,
            }]
        );
    }

    #[test]
    fn web_search_completed_emits_empty_result() {
        let p = json!({ "item": { "type": "webSearch", "id": "w1", "query": "rust patterns" } });
        assert_eq!(
            translate("item/completed", &p),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "w1".into(),
                content: String::new(),
                is_error: false,
            }]
        );
    }

    #[test]
    fn token_usage_maps_to_usage_legacy_flat_shape() {
        let p = json!({ "usage": { "input_tokens": 1000, "output_tokens": 200, "cached_input_tokens": 50,
            "total_tokens": 1250, "model_context_window": 272000 } });
        assert_eq!(
            translate("thread/tokenUsage/updated", &p),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(1000),
                output_tokens: Some(200),
                cached_input_tokens: Some(50),
                cost_usd: None,
                context_tokens: Some(1250),
                context_window: Some(272000),
            }]
        );
    }

    #[test]
    fn token_usage_thread_shape_uses_last_for_context() {
        // Modern `ThreadTokenUsage` wire: cumulative `total`, per-request
        // `last`. Tokens readout = total; context occupancy = last (using the
        // cumulative total here is the bug that pinned the meter at 100%).
        let p = json!({ "threadId": "t1", "tokenUsage": {
            "total": { "totalTokens": 4443142, "inputTokens": 4402946, "cachedInputTokens": 3733376,
                       "outputTokens": 40196, "reasoningOutputTokens": 17792 },
            "last": { "totalTokens": 132552, "inputTokens": 132153, "cachedInputTokens": 130432,
                      "outputTokens": 399, "reasoningOutputTokens": 99 },
            "modelContextWindow": 258400 } });
        assert_eq!(
            translate("thread/tokenUsage/updated", &p),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(4402946),
                output_tokens: Some(40196),
                cached_input_tokens: Some(3733376),
                cost_usd: None,
                context_tokens: Some(132552),
                context_window: Some(258400),
            }]
        );
    }

    #[test]
    fn thread_settings_updated_yields_model() {
        let p = json!({ "threadId": "t1", "threadSettings": { "model": "gpt-5.5-codex", "effort": "high" } });
        assert_eq!(
            translate("thread/settings/updated", &p),
            vec![AgentUpdate::Usage {
                model: Some("gpt-5.5-codex".into()),
                input_tokens: None,
                output_tokens: None,
                cached_input_tokens: None,
                cost_usd: None,
                context_tokens: None,
                context_window: None,
            }]
        );
    }

    #[test]
    fn account_rate_limits_map_to_windows() {
        let p = json!({ "rateLimits": {
            "primary": { "usedPercent": 19.0, "windowDurationMins": 300, "resetsAt": 1783121345 },
            "secondary": { "usedPercent": 3.0, "windowDurationMins": 10080, "resetsAt": 1783708145 } } });
        assert_eq!(
            translate("account/rateLimits/updated", &p),
            vec![AgentUpdate::RateLimits {
                five_hour_pct: Some(19.0),
                five_hour_resets_at: Some(1783121345),
                seven_day_pct: Some(3.0),
                seven_day_resets_at: Some(1783708145),
                monthly_pct: None,
                monthly_resets_at: None,
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
                raw: p.clone(),
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
                raw: p.clone(),
            }]
        );
    }

    #[test]
    fn todo_list_item_yields_plan_on_started_and_completed() {
        use crate::session::state::PlanStatus;
        let p = json!({ "item": { "type": "todoList", "id": "i1", "items": [
            { "text": "explore", "status": "completed" },
            { "text": "implement", "status": "in_progress" },
            { "text": "verify", "completed": false }
        ]}});
        for method in ["item/started", "item/completed"] {
            let updates = translate(method, &p);
            assert_eq!(updates.len(), 1, "{method} yields exactly one plan update");
            match &updates[0] {
                AgentUpdate::Plan(plan) => {
                    assert_eq!(plan.steps.len(), 3);
                    assert_eq!(plan.steps[0].content, "explore");
                    assert_eq!(plan.steps[0].status, PlanStatus::Completed);
                    assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
                    // boolean `completed: false` maps to Pending.
                    assert_eq!(plan.steps[2].status, PlanStatus::Pending);
                }
                other => panic!("expected Plan, got {other:?}"),
            }
        }
    }

    #[test]
    fn plan_item_with_step_status_shape_yields_plan() {
        use crate::session::state::PlanStatus;
        // The `update_plan`-style shape (`plan: [{ step, status }]`) surfaced as
        // a `plan` item.
        let p = json!({ "item": { "type": "plan", "plan": [
            { "step": "do the thing", "status": "pending" }
        ]}});
        let updates = translate("item/started", &p);
        assert_eq!(
            updates,
            vec![AgentUpdate::Plan(crate::session::state::Plan {
                steps: vec![crate::session::state::PlanStep {
                    content: "do the thing".into(),
                    status: PlanStatus::Pending,
                    active_form: None,
                }],
                updated_at: None,
            })]
        );
    }

    #[test]
    fn unknown_method_is_ignored() {
        assert!(translate("session/whatever", &json!({ "x": 1 })).is_empty());
        assert!(translate(
            "item/reasoning/summaryTextDelta",
            &json!({ "delta": "thinking" })
        )
        .is_empty());
    }

    #[test]
    fn merge_pending_switch_starts_and_overrides_per_field() {
        use crate::session::ModelSwitch;
        let mut pending: Option<ModelSwitch> = None;
        // First switch seeds the slot.
        merge_pending_switch(
            &mut pending,
            ModelSwitch {
                model: Some("gpt-5.5-codex".into()),
                effort: None,
            },
        );
        let p = pending.clone().unwrap();
        assert_eq!(p.model.as_deref(), Some("gpt-5.5-codex"));
        assert_eq!(p.effort, None);
        // A later switch that only sets effort keeps the earlier model.
        merge_pending_switch(
            &mut pending,
            ModelSwitch {
                model: None,
                effort: Some("high".into()),
            },
        );
        let p = pending.clone().unwrap();
        assert_eq!(p.model.as_deref(), Some("gpt-5.5-codex"));
        assert_eq!(p.effort.as_deref(), Some("high"));
        // A later switch that sets model overrides only the model.
        merge_pending_switch(
            &mut pending,
            ModelSwitch {
                model: Some("gpt-5.5".into()),
                effort: None,
            },
        );
        let p = pending.unwrap();
        assert_eq!(p.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(p.effort.as_deref(), Some("high"));
    }

    #[test]
    fn pending_switch_is_flushed_once_subscribed() {
        use crate::session::ModelSwitch;
        // Emulate the queue-and-apply path: a switch stashed before join is sent
        // as a thread/settings/update the moment we subscribe.
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        let mut pending: Option<ModelSwitch> = Some(ModelSwitch {
            model: Some("gpt-5.5-codex".into()),
            effort: Some("high".into()),
        });
        let mut req_id: u64 = 5;
        let thread_id = Some("t1".to_string());
        if let (Some(sw), Some(tid)) = (pending.take(), thread_id.as_deref()) {
            send_model_switch(&tx, &mut req_id, tid, &sw);
        }
        assert!(pending.is_none());
        let sent = rx
            .try_recv()
            .expect("a settings update should have been queued");
        assert_eq!(sent["method"], "thread/settings/update");
        assert_eq!(sent["id"], json!(6));
        assert_eq!(sent["params"]["threadId"], "t1");
        assert_eq!(sent["params"]["model"], "gpt-5.5-codex");
        assert_eq!(sent["params"]["effort"], "high");
    }

    /// The store's pending approval summary, for asserting which card is shown.
    fn pending_summary(store: &SessionStore, session_id: &str) -> Option<String> {
        match store.get(session_id).unwrap().pending {
            Some(Pending::Approval { summary, .. }) => summary,
            _ => None,
        }
    }

    #[test]
    fn headless_thread_start_response_bootstraps_and_flushes_prompts() {
        let store = SessionStore::new();
        store.register_managed("s", "/w", "codex");
        let conv = ConversationStore::new();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
        let mut thread_id: Option<String> = None;
        let mut subscribed = false;
        // A prompt sent before the thread existed must flush on bootstrap.
        let mut pending_prompts = vec!["hello".to_string()];
        let mut req_id = 2u64;
        let mut cur_mode = SessionMode::Input;
        let mut acc = UsageAcc::new();
        let yolo = AtomicBool::new(false);
        let mut pending_approvals: VecDeque<ParkedApproval> = VecDeque::new();
        let mut pending_switch = None;

        handle_message(
            &json!({ "jsonrpc": "2.0", "id": 101, "result": { "thread": { "id": "th-9" } } }),
            &store,
            &conv,
            "s",
            &out_tx,
            &mut thread_id,
            &mut subscribed,
            &mut pending_prompts,
            &mut req_id,
            &mut cur_mode,
            &mut acc,
            &yolo,
            &mut pending_approvals,
            &mut pending_switch,
            true,
        );
        assert_eq!(thread_id.as_deref(), Some("th-9"));
        assert!(subscribed);
        assert!(pending_prompts.is_empty());
        let sent = out_rx.try_recv().expect("flushed turn/start");
        assert_eq!(sent["method"], "turn/start");
        assert_eq!(sent["params"]["threadId"], "th-9");
        assert_eq!(sent["params"]["input"][0]["text"], "hello");
    }

    #[test]
    fn thread_started_notification_bootstraps_headless_but_never_hybrid() {
        // The `headless &&` gate is load-bearing: in hybrid mode this
        // notification only means the TUI's thread exists — we are NOT
        // subscribed to its stream until thread/resume succeeds. Dropping the
        // gate would mark hybrid subscribed with a silent, empty GUI pane.
        for headless in [true, false] {
            // Throwaway uuid id: the headless arm records a real sidecar under
            // ~/.workspacer/codex-threads (cleaned up via forget_thread).
            let sid = format!("wks-codex-test-{}", uuid::Uuid::new_v4());
            let store = SessionStore::new();
            store.register_managed(&sid, "/w", "codex");
            let conv = ConversationStore::new();
            let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
            let mut thread_id: Option<String> = None;
            let mut subscribed = false;
            let mut pending_prompts = vec!["hi".to_string()];
            let mut req_id = 2u64;
            let mut cur_mode = SessionMode::Input;
            let mut acc = UsageAcc::new();
            let yolo = AtomicBool::new(false);
            let mut pending_approvals: VecDeque<ParkedApproval> = VecDeque::new();
            let mut pending_switch = None;

            handle_message(
                &json!({ "jsonrpc": "2.0", "method": "thread/started",
                    "params": { "threadId": "th-n" } }),
                &store,
                &conv,
                &sid,
                &out_tx,
                &mut thread_id,
                &mut subscribed,
                &mut pending_prompts,
                &mut req_id,
                &mut cur_mode,
                &mut acc,
                &yolo,
                &mut pending_approvals,
                &mut pending_switch,
                headless,
            );

            if headless {
                // Wire-drift fallback: the id-101 response didn't carry the
                // thread id, so the notification bootstraps the session.
                assert_eq!(thread_id.as_deref(), Some("th-n"));
                assert!(subscribed);
                assert!(pending_prompts.is_empty(), "early prompt flushed");
                let sent = out_rx.try_recv().expect("flushed turn/start");
                assert_eq!(sent["method"], "turn/start");
                assert_eq!(sent["params"]["threadId"], "th-n");
                assert_eq!(sent["params"]["input"][0]["text"], "hi");
                super::super::codex_rollout::forget_thread(&sid);
            } else {
                // Hybrid must go through the id-100 discover → thread/resume
                // path; the notification alone changes nothing.
                assert_eq!(thread_id, None, "hybrid must not adopt the thread");
                assert!(!subscribed, "hybrid is not subscribed by notification");
                assert_eq!(pending_prompts, vec!["hi".to_string()]);
                assert!(out_rx.try_recv().is_err(), "nothing sent in hybrid");
            }
        }
    }

    #[test]
    fn headless_thread_start_error_surfaces_and_does_not_subscribe() {
        // Headless has no rollout fallback by design, so the id-101 error arm
        // is the pane's only death rattle: the failure must land in the
        // conversation, and the early `return` must keep the generic
        // thread_id-discovery block from misreading the error response.
        use crate::session::conversation::ConversationItem;
        let store = SessionStore::new();
        store.register_managed("s-err", "/w", "codex");
        let conv = ConversationStore::new();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
        let mut thread_id: Option<String> = None;
        let mut subscribed = false;
        let mut pending_prompts = vec!["hi".to_string()];
        let mut req_id = 2u64;
        let mut cur_mode = SessionMode::Input;
        let mut acc = UsageAcc::new();
        let yolo = AtomicBool::new(false);
        let mut pending_approvals: VecDeque<ParkedApproval> = VecDeque::new();
        let mut pending_switch = None;

        handle_message(
            &json!({ "jsonrpc": "2.0", "id": 101,
                "error": { "code": -1, "message": "boom" } }),
            &store,
            &conv,
            "s-err",
            &out_tx,
            &mut thread_id,
            &mut subscribed,
            &mut pending_prompts,
            &mut req_id,
            &mut cur_mode,
            &mut acc,
            &yolo,
            &mut pending_approvals,
            &mut pending_switch,
            true,
        );

        // The failure is surfaced in the conversation (rides as marked
        // assistant text — see apply_updates' Error arm)…
        let (_seq, items) = conv.snapshot("s-err").expect("error item recorded");
        assert!(
            items.iter().any(|i| matches!(
                i,
                ConversationItem::AssistantText { text, .. } if text.contains("thread/start failed")
            )),
            "conversation carries the thread/start failure: {items:?}"
        );
        // …and nothing pretends the session has a thread.
        assert_eq!(thread_id, None);
        assert!(!subscribed);
        assert_eq!(
            pending_prompts,
            vec!["hi".to_string()],
            "prompts are not flushed into a dead thread"
        );
        assert!(out_rx.try_recv().is_err(), "nothing emitted on the wire");
    }

    #[test]
    fn thread_id_of_reads_every_wire_shape() {
        assert_eq!(
            thread_id_of(Some(&json!({ "threadId": "a" }))).as_deref(),
            Some("a")
        );
        assert_eq!(
            thread_id_of(Some(&json!({ "thread_id": "b" }))).as_deref(),
            Some("b")
        );
        assert_eq!(
            thread_id_of(Some(&json!({ "thread": { "id": "c" } }))).as_deref(),
            Some("c")
        );
        assert!(thread_id_of(Some(&json!({ "other": 1 }))).is_none());
    }

    #[test]
    fn concurrent_approvals_surface_fifo_head_and_answer_it() {
        let store = SessionStore::new();
        store.register_managed("s", "/w", "codex");
        let conv = ConversationStore::new();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
        let mut thread_id = Some("t".to_string());
        let mut subscribed = true;
        let mut pending_prompts: Vec<String> = Vec::new();
        let mut req_id = 2u64;
        let mut cur_mode = SessionMode::Responding;
        let mut acc = UsageAcc::new();
        let yolo = AtomicBool::new(false);
        let mut pending_approvals: VecDeque<ParkedApproval> = VecDeque::new();
        let mut pending_switch = None;

        let approval_req = |id: u64, cmd: &str| {
            json!({
                "jsonrpc": "2.0", "id": id,
                "method": "item/commandExecution/requestApproval",
                "params": { "command": cmd }
            })
        };
        let mut handle =
            |v: &Value,
             cur_mode: &mut SessionMode,
             pending_approvals: &mut VecDeque<ParkedApproval>| {
                handle_message(
                    v,
                    &store,
                    &conv,
                    "s",
                    &out_tx,
                    &mut thread_id,
                    &mut subscribed,
                    &mut pending_prompts,
                    &mut req_id,
                    cur_mode,
                    &mut acc,
                    &yolo,
                    pending_approvals,
                    &mut pending_switch,
                    false,
                );
            };

        // Two approval requests arrive before the user answers either.
        handle(
            &approval_req(7, "rm -rf /tmp/x"),
            &mut cur_mode,
            &mut pending_approvals,
        );
        handle(
            &approval_req(8, "echo hi"),
            &mut cur_mode,
            &mut pending_approvals,
        );
        assert_eq!(pending_approvals.len(), 2);
        assert_eq!(cur_mode, SessionMode::Approval);
        // The DISPLAYED card is the FIFO head (first request) — a later request
        // must not overwrite it while the head is what a decision answers.
        assert_eq!(
            pending_summary(&store, "s").as_deref(),
            Some("rm -rf /tmp/x")
        );

        // Decision 1 (deny): answers the surfaced (first) request…
        resolve_approval(
            &store,
            "s",
            &out_tx,
            &mut pending_approvals,
            &mut cur_mode,
            false,
        );
        let sent = out_rx.try_recv().expect("first decision forwarded");
        assert_eq!(sent["id"], json!(7));
        assert_eq!(sent["result"]["decision"], "decline");
        // …and the second parked request re-surfaces instead of being dropped.
        assert_eq!(cur_mode, SessionMode::Approval);
        assert_eq!(pending_summary(&store, "s").as_deref(), Some("echo hi"));

        // Decision 2 (approve): answers the second request and resumes the turn.
        resolve_approval(
            &store,
            "s",
            &out_tx,
            &mut pending_approvals,
            &mut cur_mode,
            true,
        );
        let sent = out_rx.try_recv().expect("second decision forwarded");
        assert_eq!(sent["id"], json!(8));
        assert_eq!(sent["result"]["decision"], "accept");
        assert_eq!(cur_mode, SessionMode::Responding);
        assert!(pending_summary(&store, "s").is_none());
        assert!(pending_approvals.is_empty());

        // A stray decision with nothing parked is a no-op, not a panic.
        resolve_approval(
            &store,
            "s",
            &out_tx,
            &mut pending_approvals,
            &mut cur_mode,
            true,
        );
        assert!(out_rx.try_recv().is_err());
    }

    #[test]
    fn yolo_approval_is_answered_inline_and_never_parked() {
        let store = SessionStore::new();
        store.register_managed("s2", "/w", "codex");
        let conv = ConversationStore::new();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
        let mut thread_id = Some("t".to_string());
        let mut subscribed = true;
        let mut pending_prompts: Vec<String> = Vec::new();
        let mut req_id = 2u64;
        let mut cur_mode = SessionMode::Responding;
        let mut acc = UsageAcc::new();
        let yolo = AtomicBool::new(true);
        let mut pending_approvals: VecDeque<ParkedApproval> = VecDeque::new();
        let mut pending_switch = None;

        handle_message(
            &json!({
                "jsonrpc": "2.0", "id": 9,
                "method": "item/commandExecution/requestApproval",
                "params": { "command": "ls" }
            }),
            &store,
            &conv,
            "s2",
            &out_tx,
            &mut thread_id,
            &mut subscribed,
            &mut pending_prompts,
            &mut req_id,
            &mut cur_mode,
            &mut acc,
            &yolo,
            &mut pending_approvals,
            &mut pending_switch,
            false,
        );
        let sent = out_rx.try_recv().expect("yolo auto-accept");
        assert_eq!(sent["id"], json!(9));
        assert_eq!(sent["result"]["decision"], "accept");
        assert!(pending_approvals.is_empty());
        // The stripped PermissionPending must not have flipped the mode either.
        assert_eq!(cur_mode, SessionMode::Responding);
        assert!(pending_summary(&store, "s2").is_none());
    }
}

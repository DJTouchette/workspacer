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

use super::{apply_updates, note_user_send, set_mode, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::protocol::Signal;
use crate::session::conversation::ConversationItem;
use crate::session::state::{
    Pending, PendingOwner, PendingWrite, SessionMode, SubagentStatus, SubagentUpdate,
};
use crate::session::{ConversationStore, SessionStore};
use crate::wrapper::pty;
use super::SpawnExtras;

/// Translate one Codex app-server message (`method` + `params`) into typed
/// updates. Pure and total: unknown methods / missing fields yield an
/// empty/partial result.
pub fn translate(method: &str, params: &Value) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    match method {
        "turn/started" => out.push(AgentUpdate::Busy),
        // A FAILED turn arrives here, not on `turn/failed`. The app-server
        // protocol has no `turn/failed` notification at all (checked against
        // `codex app-server generate-json-schema`): every turn ends on
        // `turn/completed`, carrying `turn.status` ∈ {completed, interrupted,
        // failed, inProgress} and, when failed, a `turn.error.message`.
        // Translating this to a bare `Idle` discarded the whole failure — an
        // API-rejected turn (a wrong model id, an auth or quota refusal) left
        // the session sitting idle with no assistant text, no error, and
        // nothing anywhere saying why. That is what made a delivered-and-
        // rejected dispatch indistinguishable from a message that never
        // arrived. `Idle` still follows the error, so the turn is over either
        // way.
        "turn/completed" => {
            if let Some(msg) = turn_failure(params) {
                out.push(AgentUpdate::Error(msg));
            }
            out.push(AgentUpdate::Idle);
        }
        // `error` is the app-server's out-of-band failure notification
        // (`{ error: TurnError, threadId, turnId, willRetry }`) — a retryable
        // one is noise, so only a terminal one is surfaced. It does NOT end
        // the turn (the `turn/completed` above does), hence no `Idle` here.
        "error" => {
            if params.get("willRetry").and_then(Value::as_bool) != Some(true) {
                if let Some(msg) = error_message(params.get("error")) {
                    out.push(AgentUpdate::Error(msg));
                }
            }
        }
        // Kept for older app-servers that may still send it. Harmless where
        // the method does not exist, and the vocabulary is version-fragile
        // enough (see the model-list note in the providers doc) that dropping
        // a handler for a build we cannot check is not worth the saving.
        "turn/failed" => {
            let msg = error_message(params.get("error"))
                .or_else(|| {
                    params
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "turn failed".to_string());
            out.push(AgentUpdate::Error(msg));
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
        "thread/started" => {
            if let Some(update) = subagent_from_thread(params) {
                out.push(AgentUpdate::Subagent(update));
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
                // Context occupancy: the LAST request's INPUT side (never the
                // cumulative total — that pins the meter at 100% within a few
                // turns). Input-side (which already includes the cached subset)
                // rather than the request's totalTokens: the total also counts
                // output AND reasoning tokens, and reasoning doesn't carry
                // forward into the window — a high-effort turn inflated the
                // gauge by thousands of phantom tokens. Same convention as the
                // Claude paths (input + cache). Legacy flat shape falls back
                // through its own fields.
                let context_tokens = last
                    .and_then(|l| {
                        pick(l, ["inputTokens", "input_tokens"])
                            .or_else(|| pick(l, ["totalTokens", "total_tokens"]))
                    })
                    .or_else(|| {
                        total.is_none().then(|| {
                            input
                                .or_else(|| pick(tu, ["total_tokens", "totalTokens"]))
                                .unwrap_or_else(|| output.unwrap_or(0))
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
            // Codex echoes the whole `threadSettings` block, so this confirms an
            // effort switch we requested AND one the user made in the TUI.
            if let Some(effort) = params
                .get("threadSettings")
                .and_then(|s| s.get("effort"))
                .and_then(Value::as_str)
            {
                out.push(AgentUpdate::Effort(effort.to_string()));
            }
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
        "turn/plan/updated" => {
            if let Some(plan) = super::plan_from_value(params) {
                out.push(AgentUpdate::Plan(plan));
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
    if ty == "subAgentActivity" {
        if let Some(update) = subagent_from_activity(item) {
            out.push(AgentUpdate::Subagent(update));
        }
        return;
    }
    if ty == "collabAgentToolCall" {
        out.extend(
            subagents_from_collab_item(item)
                .into_iter()
                .map(AgentUpdate::Subagent),
        );
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
        "collabAgentToolCall" => {
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or("agent");
            out.push(AgentUpdate::ToolUse {
                id,
                name: if tool == "spawnAgent" {
                    "Agent".into()
                } else {
                    "codex_agent".into()
                },
                input: collab_input(item),
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
        "collabAgentToolCall" => {
            let (content, is_error) = collab_result_text(item);
            out.push(AgentUpdate::ToolResult {
                tool_use_id: id,
                content,
                is_error,
            });
        }
        _ => {}
    }
}

fn subagent_from_thread(params: &Value) -> Option<SubagentUpdate> {
    let thread = params.get("thread").unwrap_or(params);
    thread.get("parentThreadId").and_then(Value::as_str)?;
    let id = thread.get("id").and_then(Value::as_str)?.to_string();
    let agent_type = thread
        .get("agentRole")
        .and_then(Value::as_str)
        .or_else(|| thread.get("agentNickname").and_then(Value::as_str))
        .map(str::to_string);
    let description = thread
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| thread.get("preview").and_then(Value::as_str))
        .map(trimmed_summary);
    Some(SubagentUpdate {
        id,
        agent_type,
        status: SubagentStatus::Running,
        description,
        tool_use_id: None,
        model: None,
        last_tool_name: None,
        last_tool_summary: None,
    })
}

fn subagent_from_activity(item: &Value) -> Option<SubagentUpdate> {
    let id = item
        .get("agentThreadId")
        .and_then(Value::as_str)?
        .to_string();
    let kind = item.get("kind").and_then(Value::as_str).unwrap_or("");
    let agent_path = item.get("agentPath").and_then(Value::as_str);
    Some(SubagentUpdate {
        id,
        agent_type: agent_path.and_then(agent_type_from_path),
        status: match kind {
            "started" | "interacted" => SubagentStatus::Running,
            _ => SubagentStatus::Complete,
        },
        description: agent_path.map(trimmed_summary),
        tool_use_id: None,
        model: None,
        last_tool_name: Some(format!("subagent {kind}")),
        last_tool_summary: None,
    })
}

fn subagents_from_collab_item(item: &Value) -> Vec<SubagentUpdate> {
    let tool = item
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("agent")
        .to_string();
    let tool_use_id = item.get("id").and_then(Value::as_str).map(str::to_string);
    let prompt = item
        .get("prompt")
        .and_then(Value::as_str)
        .map(trimmed_summary);
    let model = item
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(states) = item.get("agentsStates").and_then(Value::as_object) {
        if !states.is_empty() {
            return states
                .iter()
                .map(|(id, state)| SubagentUpdate {
                    id: id.clone(),
                    agent_type: Some("codex".to_string()),
                    status: collab_agent_status(state),
                    description: prompt.clone(),
                    tool_use_id: (tool == "spawnAgent")
                        .then(|| tool_use_id.clone())
                        .flatten(),
                    model: model.clone(),
                    last_tool_name: Some(tool.clone()),
                    last_tool_summary: state
                        .get("message")
                        .and_then(Value::as_str)
                        .map(trimmed_summary),
                })
                .collect();
        }
    }
    item.get("receiverThreadIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| SubagentUpdate {
            id: id.to_string(),
            agent_type: Some("codex".to_string()),
            status: collab_tool_status(item),
            description: prompt.clone(),
            tool_use_id: (tool == "spawnAgent")
                .then(|| tool_use_id.clone())
                .flatten(),
            model: model.clone(),
            last_tool_name: Some(tool.clone()),
            last_tool_summary: None,
        })
        .collect()
}

fn collab_agent_status(state: &Value) -> SubagentStatus {
    match state.get("status").and_then(Value::as_str).unwrap_or("") {
        "pendingInit" | "running" | "inProgress" => SubagentStatus::Running,
        _ => SubagentStatus::Complete,
    }
}

fn collab_tool_status(item: &Value) -> SubagentStatus {
    match item.get("status").and_then(Value::as_str).unwrap_or("") {
        "inProgress" => SubagentStatus::Running,
        _ => SubagentStatus::Complete,
    }
}

fn collab_input(item: &Value) -> Value {
    let mut input = json!({
        "tool": item.get("tool").cloned().unwrap_or(Value::Null),
        "receiverThreadIds": item.get("receiverThreadIds").cloned().unwrap_or(Value::Null),
    });
    for key in ["prompt", "model", "reasoningEffort"] {
        if let Some(value) = item.get(key) {
            input[key] = value.clone();
        }
    }
    input
}

fn collab_result_text(item: &Value) -> (String, bool) {
    let status = item.get("status").and_then(Value::as_str).unwrap_or("");
    let tool = item.get("tool").and_then(Value::as_str).unwrap_or("agent");
    let mut lines = Vec::new();
    if let Some(states) = item.get("agentsStates").and_then(Value::as_object) {
        for (id, state) in states {
            let state_status = state
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let message = state
                .get("message")
                .and_then(Value::as_str)
                .map(trimmed_summary);
            lines.push(match message {
                Some(message) => format!("{id}: {state_status} — {message}"),
                None => format!("{id}: {state_status}"),
            });
        }
    }
    let content = if lines.is_empty() {
        format!("{tool} {status}")
    } else {
        lines.join("\n")
    };
    let failed = matches!(status, "failed" | "interrupted")
        || item
            .get("agentsStates")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|states| states.values())
            .any(|state| {
                matches!(
                    state.get("status").and_then(Value::as_str).unwrap_or(""),
                    "errored" | "notFound"
                )
            });
    (content, failed)
}

fn agent_type_from_path(path: &str) -> Option<String> {
    let last = path.rsplit('/').next().unwrap_or(path);
    let stem = last
        .strip_suffix(".md")
        .or_else(|| last.strip_suffix(".json"))
        .unwrap_or(last)
        .trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

fn trimmed_summary(s: &str) -> String {
    const MAX: usize = 180;
    let single_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= MAX {
        return single_line;
    }
    single_line.chars().take(MAX).collect()
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

/// Pull the human-readable message out of a `TurnError`
/// (`{ message, additionalDetails?, codexErrorInfo? }`). `data` is accepted as
/// an alias because a JSON-RPC error object spells the same thing that way,
/// and both shapes reach this helper.
fn error_message(err: Option<&Value>) -> Option<String> {
    let err = err?;
    let msg = err
        .get("message")
        .or_else(|| err.get("data"))
        .and_then(Value::as_str)?
        .trim();
    if msg.is_empty() {
        return None;
    }
    Some(msg.to_string())
}

/// The failure message of a `turn/completed` whose turn FAILED, or `None` for
/// a turn that completed or was interrupted.
///
/// Keyed on the error being present rather than on `status == "failed"`: the
/// status is a closed enum today, but an error attached under a spelling this
/// build does not know still has to be shown rather than swallowed. An
/// interrupt is not a failure and carries no error, so it stays quiet.
fn turn_failure(params: &Value) -> Option<String> {
    let turn = params.get("turn")?;
    let msg = error_message(turn.get("error"));
    if msg.is_none() && turn.get("status").and_then(Value::as_str) == Some("failed") {
        // Failed with no message attached — still worth saying so, since the
        // alternative is a session that answers nothing and explains nothing.
        return Some("turn failed".to_string());
    }
    msg
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
    set_mode(
        store,
        session_id,
        SessionMode::Approval,
        PendingWrite::Park(
            PendingOwner::Primary,
            Pending::Approval {
                tool: parked.tool.clone(),
                summary: parked.summary.clone(),
                raw: parked.raw.clone(),
            },
        ),
        cur_mode,
    );
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
        None => set_mode(
            store,
            session_id,
            SessionMode::Responding,
            PendingWrite::Resolve(PendingOwner::Primary),
            cur_mode,
        ),
    }
}

// ── Model listing ────────────────────────────────────────────────────────────

/// List the models Codex offers (cached; see [`super::cached_or_fetch`]).
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    super::cached_or_fetch(format!("codex:{bin}"), fetch_models(bin, cwd)).await
}

/// Live query: boot a throwaway `codex app-server`, `initialize`, ask for the
/// catalog via `model/list`, then drop the process. Hidden models are skipped;
/// the rest map to the picker with their `displayName` as label, the
/// server-flagged default marked, and that model's supported reasoning-effort
/// ids preserved. The latter is intentionally live metadata: Codex effort
/// ladders vary by model and evolve with the installed CLI.
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
                .filter_map(model_info_from_value)
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

/// Parse one Codex app-server `model/list` row. Kept separate from the process
/// handshake so the model-specific effort contract is easy to regression-test.
fn model_info_from_value(model: &Value) -> Option<ModelInfo> {
    let id = model
        .get("model")
        .or_else(|| model.get("id"))
        .and_then(Value::as_str)?
        .to_string();
    let label = model
        .get("displayName")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_string();
    let default = model
        .get("isDefault")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let effort_levels = model
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|effort| {
            effort
                .get("reasoningEffort")
                .or_else(|| effort.get("reasoning_effort"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    // The level a turn runs at when no `-c model_reasoning_effort=` is passed.
    // Per model, and it genuinely varies (verified against the installed CLI:
    // 'medium' for gpt-5.6-sol, 'xhigh' for gpt-5.5), so it can't be a constant.
    let default_effort = model
        .get("defaultReasoningEffort")
        .or_else(|| model.get("default_reasoning_effort"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(ModelInfo {
        id,
        label,
        default,
        effort_levels,
        default_effort,
    })
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
    // The profile's CODEX_HOME and its extra argv (`codex -p <preset>`).
    // Carried, not interpreted: the daemon does not decide what a Codex
    // profile means, it only stops dropping it. See `SpawnExtras`.
    extras: SpawnExtras,
) {
    // Claimed before the task starts: on a restart this driver's tail can
    // outlive its own lifetime, and must not tear down a successor.
    let generation = store.claim_generation(&session_id);
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
            &extras,
        )
        .await
        {
            tracing::warn!(?err, session = %session_id, "codex managed session ended with error");
        }
        // Both drops belong to this lifetime or to neither. Forgetting the
        // conversation unconditionally was the hole left beside the guarded
        // teardown: for a driver-fed provider there is no transcript to rebuild
        // from, so a superseded exit erased the successor's visible history for
        // good.
        if store.deregister_managed(&session_id, generation) {
            conv.forget(&session_id);
        }
    });
}

/// Shown in the conversation when a codex session degrades to the rollout
/// fallback, so the degradation is legible to whoever dispatched it rather than
/// being inferred from a missing structured result.
const DEGRADED_NOTICE: &str = "⚠️ Codex degraded to the rollout fallback: the \
app-server RPC path was unavailable, so this session runs as a terminal UI with \
its transcript tailed into the GUI. Workspacer MCP tools and role instructions \
are still attached; approvals happen in the Term view, and text arrives in \
transcript-sized chunks rather than token deltas.";

/// The same degradation, but for a session that was asked for HEADLESS. It says
/// the extra thing that matters there: a Term view now exists where the caller
/// was promised none, so approvals have somewhere to go — and on a client with
/// no terminal surface (mobile, the web app) they do not.
const DEGRADED_FROM_HEADLESS_NOTICE: &str = "⚠️ Codex could not start its \
app-server, so this HEADLESS session degraded to the rollout fallback: a \
terminal UI with its transcript tailed into the GUI. A Term view has appeared \
on this pane — approvals happen there, not as structured cards, and clients \
with no terminal (mobile, web) cannot answer them. Workspacer MCP tools and \
role instructions are still attached. Check that `codex app-server` runs in \
this directory.";

/// The `-c mcp_servers.*` config overrides that attach the facade to a codex
/// process: the workspacer MCP facade (tools at the session's tier, token on the
/// URL) and the daemon's per-session AskUserQuestion shim.
///
/// ONE list, shared by the app-server and the fallback TUI. They took different
/// paths before, which is how the fallback silently came up with no tools at
/// all — a divergence that could only be noticed by its absence.
fn facade_mcp_overrides(session_id: &str, facade: &Facade) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(mcp_url) = &facade.mcp_url {
        out.push((
            "mcp_servers.workspacer.url".to_string(),
            Value::String(mcp_url.clone()).to_string(),
        ));
    }
    // AskUserQuestion: the daemon serves a per-session MCP endpoint that parks a
    // structured question for the GUI and blocks until /answer resolves it —
    // Codex's stand-in for Claude's native tool. The generous tool timeout is
    // the point: a question can legitimately wait on the user for hours.
    if let Some(api_base) = crate::daemon::API_BASE.get() {
        out.push((
            "mcp_servers.workspacer_ask.url".to_string(),
            Value::String(format!("{api_base}/mcp/ask/{session_id}")).to_string(),
        ));
        out.push((
            "mcp_servers.workspacer_ask.tool_timeout_sec".to_string(),
            "21600".to_string(),
        ));
    }
    out
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
    extras: &SpawnExtras,
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
    if let Some((model, effort)) = overrides {
        if let Some(m) = model {
            cmd.arg("-c").arg(format!("model={}", Value::String(m)));
        }
        if let Some(e) = effort {
            cmd.arg("-c")
                .arg(format!("model_reasoning_effort={}", Value::String(e)));
        }
    }
    // The workspacer MCP facade + the daemon's AskUserQuestion shim.
    for (key, value) in facade_mcp_overrides(session_id, facade) {
        cmd.arg("-c").arg(format!("{key}={value}"));
    }
    // The profile's own argv, LAST, so a preset can override what we set above.
    cmd.args(&extras.extra_args);
    // …and its config root. `envs` MERGES onto the inherited environment rather
    // than replacing it, which is what a profile means: read your config from
    // here, keep everything else.
    cmd.envs(&extras.env);
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
///
/// It carries the FACADE with it. It used to take no `facade` at all, so a
/// dispatched worker that landed here came up looking healthy while missing its
/// workspacer MCP tools, its role instructions and its `wks-result` contract —
/// and nothing said so, which reads from the manager's side as a worker that
/// inexplicably ignored its result schema. A silent partial-capability session
/// is worse than a failed spawn: the degradation is now both repaired (the MCP
/// servers are registered as config overrides on the TUI, exactly as
/// `start_appserver` does, and the role instructions ride the first prompt) and
/// VISIBLE (a warn log plus a notice pushed into the session's conversation, so
/// `get_conversation` shows it).
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
    facade: &Facade,
    extras: &SpawnExtras,
    // Role instructions not yet delivered to the agent. Distinct from
    // `facade.instructions` because the ws path may already have consumed them:
    // `initial_prompts` below are ALREADY instruction-wrapped, so re-prepending
    // would send the contract twice.
    mut pending_instructions: Option<String>,
    // Prompts the user sent on the ws path that were never delivered (buffered
    // while waiting for the TUI's thread). Already echoed into the conversation
    // store and already instruction-wrapped — deliver them to the fallback TUI
    // instead of silently dropping a message the GUI shows as sent.
    initial_prompts: Vec<String>,
    // Whether the session that is degrading was asked for HEADLESS. Only the
    // wording of the ⚠️ notice changes: a headless caller was promised no
    // terminal, and one has just appeared, which is the part they need told.
    from_headless: bool,
) -> anyhow::Result<()> {
    // Plain codex TUI (no `--remote`): it owns its own session and writes a rollout.
    let argv = fallback_tui_argv(
        bin,
        model.as_deref(),
        effort.as_deref(),
        yolo,
        session_id,
        facade,
        &extras.extra_args,
    );
    // The degraded path is where a dropped profile would hide longest — the
    // pane works, so nothing looks wrong — so it carries CODEX_HOME too.
    let tui = super::spawn_attach_pty(store, session_id, &argv, cwd, &extras.env)
        .context("spawning fallback codex TUI")?;
    // Make the degradation visible. It is not a failure — the pane works — but
    // it is a different, thinner session than the one that was asked for, and
    // whoever dispatched it has to be able to see that.
    tracing::warn!(
        session = %session_id,
        facade = facade.mcp_url.is_some(),
        instructions = pending_instructions.is_some(),
        from_headless,
        "codex degraded to the rollout fallback (Term + transcript-tailed GUI): \
         structural approvals and token-level streaming are unavailable"
    );
    conv.push(
        session_id,
        vec![ConversationItem::AssistantText {
            text: if from_headless {
                DEGRADED_FROM_HEADLESS_NOTICE.to_string()
            } else {
                DEGRADED_NOTICE.to_string()
            },
            timestamp: None,
        }],
    );
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
                    // Echo verbatim, but prepend the role instructions (once) to
                    // what the agent actually receives — same contract as the ws
                    // path, which is what carries the `wks-result` schema.
                    conv.push(session_id, vec![ConversationItem::UserMessage { text: text.clone(), timestamp: None }]);
                    let sent = with_instructions(&mut pending_instructions, text);
                    write_prompt(&tui, &sent).await;
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

/// Argv for the fallback TUI. Same config overrides the app-server path applies
/// — model, reasoning effort, the bypass flag, and (the bit that used to go
/// missing) the facade's MCP servers.
fn fallback_tui_argv(
    bin: &str,
    model: Option<&str>,
    effort: Option<&str>,
    yolo: bool,
    session_id: &str,
    facade: &Facade,
    extra_args: &[String],
) -> Vec<String> {
    let mut argv = vec![bin.to_string()];
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
    // The workspacer MCP facade (and the daemon's AskUserQuestion shim) are
    // plain `-c mcp_servers.*` config overrides — the TUI takes them exactly
    // like the app-server does, so a fallback session keeps its tools.
    for (key, value) in facade_mcp_overrides(session_id, facade) {
        argv.push("-c".to_string());
        argv.push(format!("{key}={value}"));
    }
    // The profile's own argv last, so `codex -p <preset>` wins over the
    // defaults above rather than being shadowed by them.
    argv.extend(extra_args.iter().cloned());
    argv
}

/// Prepend the role instructions (once) to what the agent actually receives.
/// The user's own message is echoed verbatim into the conversation separately;
/// this is only the wire text. Taking the Option is what makes it once-only —
/// the brief rides the first prompt of the session and nothing after it.
fn with_instructions(pending: &mut Option<String>, text: String) -> String {
    match pending.take() {
        Some(instr) => format!("{instr}\n\n{text}"),
        None => text,
    }
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
    extras: &SpawnExtras,
) -> anyhow::Result<()> {
    // Start the app-server + ws client. If that fails, the ws path is unavailable
    // for this Codex build (e.g. a version that dropped/renamed `app-server
    // --listen`, or won't bind/handshake) — degrade to the rollout hybrid rather
    // than leave the pane dead. The RPC path is preferred; this is the safety net.
    //
    // HEADLESS TAKES THE SAME NET, and that changed with the headless default.
    // It used to be a hard error here on the grounds that headless promises no
    // PTY — defensible while headless was a deliberate opt-in, and untenable now
    // that it is what a plain `codex` spawn resolves to (config `codex.transport`
    // ships 'stream'): the failure mode would be a dead pane for every user
    // whose codex build cannot serve `app-server --listen`, on the default path.
    // It is also what makes it safe to send Windows down this path at all — the
    // ws app-server was chosen because plain-TCP ws works there (see the module
    // header), but that has never been verified on a real Windows box, so the
    // unverified leg must degrade rather than fail.
    //
    // The degradation is LOUD and it MOVES THE SESSION: the transport stamp goes
    // back to Pty (the pane's Term view is gated on that stamp — the desktop
    // reads `session.transport` as the authority, so this is what makes the
    // terminal actually appear), a ⚠️ notice lands in the conversation, and the
    // warn log names the error. Nothing about it is silent, and nothing about it
    // leaves the caller holding a session it cannot talk to.
    //
    // For headless the app-server is the thread's creator, so the model/effort
    // overrides that hybrid mode sets on the TUI go on the server instead.
    let overrides = headless.then(|| (model.clone(), effort.clone()));
    let (mut child, ws_stream, ws_url) = match start_appserver(
        session_id, cwd, bin, facade, overrides, extras,
    )
    .await
    {
        Ok(t) => t,
        Err(err) => {
            tracing::warn!(?err, session = %session_id, headless, "codex app-server ws path unavailable — falling back to the rollout hybrid (Term + transcript-tailed GUI)");
            if headless {
                // The session is no longer headless. Say so where every client
                // reads it, BEFORE the PTY exists, so no snapshot ever shows a
                // stream-stamped session that has a terminal.
                store.set_transport(session_id, crate::session::state::Transport::Pty);
            }
            return run_rollout_fallback(
                store,
                conv,
                session_id,
                cwd,
                model,
                effort,
                bin,
                yolo,
                facade,
                extras,
                // Nothing has been sent yet, so the whole role brief is still owed.
                facade.instructions.clone(),
                Vec::new(),
                headless,
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
    // The sandbox/approval posture that rides on every headless `turn/start`.
    // It shares `yolo_live` with the store, so a live `/permission-mode` flip
    // moves the REAL sandbox with the adapter's auto-accept instead of only
    // half of it.
    let policy = TurnPolicy::new(headless, yolo_live.clone());

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
            extras,
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
        // A bypassed headless session states the bypass to the app-server
        // itself — `sandbox` + `approvalPolicy` — instead of relying on the
        // adapter auto-accepting approval requests, which leaves codex's own
        // sandbox fully in force. RESUME MATTERS AS MUCH AS START: a resumed
        // thread never runs `thread/start`, so without the same params here a
        // resumed worker silently reverts to sandboxed. Non-bypassed sessions
        // send neither param and keep codex's resolved defaults.
        match &thread_id {
            Some(tid) => {
                let _ = out_tx.send(thread_resume_request(tid, headless, yolo));
            }
            None => {
                let _ = out_tx.send(thread_start_request(cwd, headless, yolo));
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
                            &mut cur_mode, &mut acc, &policy, &mut pending_approvals,
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
                    let sent = with_instructions(&mut pending_instructions, text);
                    note_user_send(store, session_id, &mut cur_mode);
                    match &thread_id {
                        Some(tid) => {
                            req_id += 1;
                            send_turn(&out_tx, req_id, tid, &sent, &policy);
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
            facade,
            extras,
            // Whatever the ws path had not yet delivered. `pending_prompts` are
            // already instruction-wrapped, so this is `None` once the first one
            // consumed the brief.
            pending_instructions,
            pending_prompts,
            // Hybrid-only: `needs_fallback` is set exclusively in the
            // TUI-thread discovery arm, which is gated on `!headless`.
            false,
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
    extras: &SpawnExtras,
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
    argv.extend(extras.extra_args.iter().cloned());
    match super::spawn_attach_pty(store, session_id, &argv, cwd, &extras.env) {
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
    policy: &TurnPolicy,
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
        // Only the ONE bootstrap `thread/start` uses id 101, and it completes
        // before any thread exists. Once a thread is established, an id-101
        // response can only be a user turn / live switch whose ever-incrementing
        // counter reached 101 — do NOT misroute it through the thread/start
        // handler (it would surface a spurious failure or re-capture a thread id).
        if id == Some(101) && thread_id.is_none() {
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
                    send_turn(out_tx, *req_id, &tid, &text, policy);
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
                    send_turn(out_tx, *req_id, tid, &text, policy);
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
                send_turn(out_tx, *req_id, &tid, &text, policy);
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
        if policy.bypassing() {
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
/// `pub(crate)` so daemon::heartbeat's throwaway app-server run shares it.
pub(crate) fn thread_id_of(v: Option<&Value>) -> Option<String> {
    let v = v?;
    v.get("threadId")
        .or_else(|| v.get("thread_id"))
        .or_else(|| v.get("thread").and_then(|t| t.get("id")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Codex's own sandbox vocabulary. `thread/start` and `thread/resume` take a
/// bare [`SandboxMode`] string under `sandbox`; `turn/start` takes a tagged
/// [`SandboxPolicy`] object under `sandboxPolicy`. Same concept, two shapes —
/// both read off codex 0.149's generated JSON schema (`codex app-server
/// generate-json-schema --out …`, `v2/ThreadStartParams.json` /
/// `v2/TurnStartParams.json`).
const SANDBOX_MODE_FULL: &str = "danger-full-access";
/// `AskForApproval`. `never` is codex's "don't ask me anything"; `on-request`
/// is its ordinary ask-when-needed posture.
const APPROVAL_NEVER: &str = "never";
const APPROVAL_ON_REQUEST: &str = "on-request";

/// Whether a *headless* thread should be created (`thread/start`) or rejoined
/// (`thread/resume`) with the bypass overrides, and which ones.
///
/// `None` means "send neither param", which is the whole point of the split: a
/// session spawned WITHOUT the bypass grant must keep whatever codex's own
/// config resolves for its cwd — untouched, exactly as before this existed.
///
/// Hybrid always answers `None`: there the native TUI creates the thread and
/// already carries `--dangerously-bypass-approvals-and-sandbox` itself, so the
/// bypass is real at the source and ours would be redundant.
fn thread_bypass_params(headless: bool, yolo: bool) -> Option<(&'static str, &'static str)> {
    (headless && yolo).then_some((SANDBOX_MODE_FULL, APPROVAL_NEVER))
}

/// Apply [`thread_bypass_params`] to a `thread/start` / `thread/resume` params
/// object in place.
fn apply_thread_bypass(params: &mut Value, headless: bool, yolo: bool) {
    if let Some((sandbox, approval)) = thread_bypass_params(headless, yolo) {
        params["sandbox"] = json!(sandbox);
        params["approvalPolicy"] = json!(approval);
    }
}

/// The headless bootstrap `thread/start` (id 101) — this client creates the
/// thread, so the bypass has to be stated here.
fn thread_start_request(cwd: &str, headless: bool, yolo: bool) -> Value {
    let mut params = json!({ "cwd": cwd });
    apply_thread_bypass(&mut params, headless, yolo);
    json!({ "jsonrpc": "2.0", "id": 101, "method": "thread/start", "params": params })
}

/// The headless `thread/resume` (id 2) — the resume path skips `thread/start`
/// entirely, so it needs the same overrides or a resumed bypassed worker comes
/// back sandboxed.
fn thread_resume_request(thread_id: &str, headless: bool, yolo: bool) -> Value {
    let mut params = json!({ "threadId": thread_id });
    apply_thread_bypass(&mut params, headless, yolo);
    json!({ "jsonrpc": "2.0", "id": 2, "method": "thread/resume", "params": params })
}

/// The sandbox/approval posture this session stamps on every `turn/start`.
///
/// Two flags, not one, because the directions are asymmetric. `yolo` is the
/// live `/permission-mode` state (the same `AtomicBool` the store flips).
/// `engaged` latches the first moment the session was ever bypassed — at spawn,
/// or later via a live `ask → yolo` flip.
///
/// Before the latch flips we send NO sandbox or approval params at all, so an
/// un-bypassed session keeps codex's own resolved defaults and is never widened
/// by us. After it flips we own the posture and must state it explicitly in
/// *both* directions: a `yolo → ask` flip that only cleared the adapter's
/// auto-accept would leave the thread sitting at `danger-full-access` while the
/// UI reported `ask` — the two halves diverging, which is worse than either.
///
/// Turn-scoped rather than live because a sandbox only governs what a turn
/// *executes*, and codex applies `turn/start`'s override "for this turn and
/// subsequent turns". A flip lands on the next turn; nothing runs between turns.
///
/// Hybrid stamps nothing, ever — see [`thread_bypass_params`].
struct TurnPolicy {
    headless: bool,
    yolo: Arc<AtomicBool>,
    engaged: Arc<AtomicBool>,
}

impl TurnPolicy {
    fn new(headless: bool, yolo: Arc<AtomicBool>) -> Self {
        let engaged = Arc::new(AtomicBool::new(yolo.load(Ordering::Relaxed)));
        Self {
            headless,
            yolo,
            engaged,
        }
    }

    /// Is the adapter currently auto-accepting approval requests?
    fn bypassing(&self) -> bool {
        self.yolo.load(Ordering::Relaxed)
    }

    /// `(sandboxPolicy, approvalPolicy)` for a `turn/start`, or `None` to send
    /// neither. Reads the live flag (and latches it), so it reflects a
    /// `/permission-mode` switch made since the last turn.
    fn turn_params(&self) -> Option<(Value, &'static str)> {
        if !self.headless {
            return None;
        }
        let yolo = self.bypassing();
        if yolo {
            self.engaged.store(true, Ordering::Relaxed);
        } else if !self.engaged.load(Ordering::Relaxed) {
            // Never bypassed — leave codex's defaults alone.
            return None;
        }
        Some(if yolo {
            (json!({ "type": "dangerFullAccess" }), APPROVAL_NEVER)
        } else {
            // Re-tightened after a bypass. We can't restore the unknown
            // pre-bypass default, so state codex's ordinary posture explicitly
            // — a narrowing from full access, and honest about what it is.
            (json!({ "type": "workspaceWrite" }), APPROVAL_ON_REQUEST)
        })
    }
}

/// Build a `turn/start` request. Headless sessions carry the sandbox/approval
/// posture on every turn (see [`TurnPolicy`]); hybrid carries neither.
fn turn_start_request(id: u64, thread_id: &str, text: &str, policy: &TurnPolicy) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "input": [ { "type": "text", "text": text } ]
    });
    if let Some((sandbox, approval)) = policy.turn_params() {
        params["sandboxPolicy"] = sandbox;
        params["approvalPolicy"] = json!(approval);
    }
    json!({ "jsonrpc": "2.0", "id": id, "method": "turn/start", "params": params })
}

fn send_turn(
    out_tx: &mpsc::UnboundedSender<Value>,
    id: u64,
    thread_id: &str,
    text: &str,
    policy: &TurnPolicy,
) {
    let _ = out_tx.send(turn_start_request(id, thread_id, text, policy));
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

    // ── Headless bypass: sandbox + approval policy on the wire ──────────────
    //
    // The bug these pin: `skipPermissions` on the headless (stream) transport
    // used to be pure adapter-side auto-accept of `requestApproval`, so a
    // "full access" worker still ran inside codex's own sandbox — fatal in a
    // fresh ship worktree, which is never in the user's trusted-projects list.
    // Field names/values verified against codex 0.149's generated JSON schema
    // (`v2/ThreadStartParams.json`, `v2/ThreadResumeParams.json`,
    // `v2/TurnStartParams.json`).

    fn policy_for(headless: bool, yolo: bool) -> TurnPolicy {
        TurnPolicy::new(headless, Arc::new(AtomicBool::new(yolo)))
    }

    // ── Live probe against a real `codex app-server` ───────────────────────
    //
    // `#[ignore]`d: it needs the codex CLI, network and an authenticated
    // account, so it never runs in CI. Run it by hand when touching the
    // sandbox/approval wire:
    //
    //   CODEX_BIN=$(which codex) cargo test -p claudemon --lib \
    //     codex_headless_bypass_really_escapes_the_sandbox -- --ignored --nocapture
    //
    // It drives the EXACT request builders the driver uses — not a re-typed
    // copy — creates a thread in a fresh directory that is NOT in the user's
    // codex trusted-projects list, and asks the agent to write a file there.
    // Under codex's default sandbox for an untrusted cwd that write is denied;
    // it succeeds only if the bypass genuinely reached the app-server.

    async fn live_probe(bin: &str, dir: &std::path::Path, yolo: bool) -> anyhow::Result<bool> {
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        let ws_url = format!("ws://127.0.0.1:{port}");
        let mut child = Command::new(bin)
            .arg("app-server")
            .arg("--listen")
            .arg(&ws_url)
            .current_dir(dir)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        wait_ready(&format!("http://127.0.0.1:{port}")).await?;
        let (ws, _) = connect_async(&ws_url).await?;
        let (mut w, mut r) = ws.split();

        async fn send(
            w: &mut futures_util::stream::SplitSink<CodexWs, Message>,
            v: Value,
        ) -> anyhow::Result<()> {
            w.send(Message::Text(v.to_string())).await?;
            Ok(())
        }
        send(
            &mut w,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "clientInfo": { "name": "workspacer-test", "version": "0" } } }),
        )
        .await?;
        let cwd = dir.to_string_lossy().to_string();
        send(&mut w, thread_start_request(&cwd, true, yolo)).await?;

        let mut thread: Option<String> = None;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(180);
        let target = dir.join("wks-sandbox-probe.txt");
        while tokio::time::Instant::now() < deadline {
            let Ok(Some(Ok(Message::Text(t)))) =
                tokio::time::timeout(std::time::Duration::from_secs(30), r.next()).await
            else {
                break;
            };
            for line in t.split('\n').map(str::trim).filter(|l| !l.is_empty()) {
                let Ok(v) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                eprintln!("<< {}", &line[..line.len().min(280)]);
                if v.get("id").and_then(Value::as_u64) == Some(101) {
                    assert!(
                        v.get("error").is_none(),
                        "the app-server REJECTED our thread/start params: {v}"
                    );
                    let tid = thread_id_of(v.get("result")).expect("thread id");
                    thread = Some(tid.clone());
                    let policy = policy_for(true, yolo);
                    send(
                        &mut w,
                        turn_start_request(
                            3,
                            &tid,
                            "Write the single word `ok` into ./wks-sandbox-probe.txt in the \
                         current directory, then reply DONE. Do not ask any questions.",
                            &policy,
                        ),
                    )
                    .await?;
                }
                // Non-bypassed control: an approval request means the sandbox is
                // in force. Deny it and stop — that IS the expected outcome.
                if v.get("method")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m.ends_with("/requestApproval"))
                {
                    let id = v.get("id").cloned().unwrap_or(Value::Null);
                    send(
                        &mut w,
                        json!({ "jsonrpc": "2.0", "id": id,
                        "result": { "decision": "denied" } }),
                    )
                    .await?;
                }
                if v.get("method")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m == "turn/completed" || m == "turn/failed")
                    && thread.is_some()
                {
                    let _ = child.start_kill();
                    return Ok(target.exists());
                }
            }
        }
        let _ = child.start_kill();
        Ok(target.exists())
    }

    #[tokio::test]
    #[ignore = "needs the codex CLI, network and an authenticated account"]
    async fn codex_headless_bypass_really_escapes_the_sandbox() {
        let bin = std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into());
        let root = std::env::temp_dir().join(format!("wks-codex-probe-{}", uuid::Uuid::new_v4()));
        let bypassed = root.join("bypassed");
        let control = root.join("control");
        std::fs::create_dir_all(&bypassed).unwrap();
        std::fs::create_dir_all(&control).unwrap();

        let got = live_probe(&bin, &bypassed, true)
            .await
            .expect("bypassed probe");
        let ctl = live_probe(&bin, &control, false)
            .await
            .expect("control probe");
        let _ = std::fs::remove_dir_all(&root);

        assert!(
            got,
            "BYPASSED headless session could not write in an untrusted cwd"
        );
        assert!(
            !ctl,
            "CONTROL (non-bypassed) session wrote without approval — \
            the sandbox was already off, so this probe proves nothing"
        );
    }

    // ── Rollout fallback keeps the facade, and says so ─────────────────────
    //
    // The bug these pin: `run_rollout_fallback` took no `facade` at all, so a
    // session that degraded here came up *working* but with no workspacer MCP
    // tools, no role instructions and no `wks-result` contract — silently. From
    // the dispatching manager's side that reads as a worker that ignored its
    // result schema.

    #[test]
    fn the_fallback_tui_registers_the_facade_mcp_server() {
        let facade = Facade {
            mcp_url: Some("http://127.0.0.1:9/mcp?t=tok".into()),
            instructions: None,
        };
        let argv = fallback_tui_argv("codex", Some("gpt-5.5"), None, true, "s1", &facade, &[]);
        assert!(
            argv.iter()
                .any(|a| a == "mcp_servers.workspacer.url=\"http://127.0.0.1:9/mcp?t=tok\""),
            "facade MCP server missing from the fallback TUI argv: {argv:?}"
        );
        // …and the rest of what the app-server path applies.
        assert!(argv.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
        assert!(argv.iter().any(|a| a == "model=\"gpt-5.5\""));
    }

    #[test]
    fn a_facade_less_session_registers_no_workspacer_server() {
        let argv = fallback_tui_argv("codex", None, None, false, "s1", &Facade::default(), &[]);
        assert!(!argv
            .iter()
            .any(|a| a.starts_with("mcp_servers.workspacer.url")));
        assert_eq!(argv[0], "codex");
    }

    /// The `wks-result` contract rides the role instructions, and the role
    /// instructions ride the FIRST prompt — once, never twice.
    #[test]
    fn role_instructions_ride_the_first_prompt_only() {
        let mut pending = Some("ROLE BRIEF".to_string());
        assert_eq!(
            with_instructions(&mut pending, "do the thing".into()),
            "ROLE BRIEF\n\ndo the thing"
        );
        assert_eq!(with_instructions(&mut pending, "next".into()), "next");
    }

    #[test]
    fn a_session_with_no_role_brief_sends_the_prompt_untouched() {
        assert_eq!(with_instructions(&mut None, "hi".into()), "hi");
    }

    #[test]
    fn headless_bypassed_thread_start_carries_the_sandbox_and_approval_override() {
        let req = thread_start_request("/tmp/wt", true, true);
        assert_eq!(req["method"], "thread/start");
        assert_eq!(req["params"]["cwd"], "/tmp/wt");
        assert_eq!(req["params"]["sandbox"], "danger-full-access");
        assert_eq!(req["params"]["approvalPolicy"], "never");
    }

    #[test]
    fn headless_unbypassed_thread_start_sends_neither_param() {
        let req = thread_start_request("/tmp/wt", true, false);
        // Byte-for-byte what we sent before the bypass existed: codex keeps
        // whatever its own config resolves for this cwd.
        assert_eq!(req["params"], json!({ "cwd": "/tmp/wt" }));
    }

    #[test]
    fn hybrid_thread_start_is_untouched_even_when_bypassed() {
        // Hybrid's bypass is real at the source (the thread-owning TUI carries
        // `--dangerously-bypass-approvals-and-sandbox`); ours must not intrude.
        assert_eq!(
            thread_start_request("/tmp/wt", false, true)["params"],
            json!({ "cwd": "/tmp/wt" })
        );
    }

    /// The resume path skips `thread/start` entirely — without this a resumed
    /// bypassed worker silently comes back sandboxed.
    #[test]
    fn headless_bypassed_thread_resume_carries_the_same_override() {
        let req = thread_resume_request("th_1", true, true);
        assert_eq!(req["method"], "thread/resume");
        assert_eq!(req["params"]["threadId"], "th_1");
        assert_eq!(req["params"]["sandbox"], "danger-full-access");
        assert_eq!(req["params"]["approvalPolicy"], "never");
    }

    #[test]
    fn headless_unbypassed_thread_resume_sends_neither_param() {
        assert_eq!(
            thread_resume_request("th_1", true, false)["params"],
            json!({ "threadId": "th_1" })
        );
    }

    #[test]
    fn headless_bypassed_turns_carry_the_sandbox_policy() {
        let req = turn_start_request(7, "th_1", "go", &policy_for(true, true));
        assert_eq!(req["method"], "turn/start");
        assert_eq!(req["params"]["input"][0]["text"], "go");
        assert_eq!(
            req["params"]["sandboxPolicy"],
            json!({ "type": "dangerFullAccess" })
        );
        assert_eq!(req["params"]["approvalPolicy"], "never");
    }

    #[test]
    fn unbypassed_turns_carry_no_sandbox_or_approval_params() {
        for p in [
            policy_for(true, false),
            policy_for(false, false),
            policy_for(false, true),
        ] {
            let req = turn_start_request(7, "th_1", "go", &p);
            assert!(req["params"].get("sandboxPolicy").is_none());
            assert!(req["params"].get("approvalPolicy").is_none());
        }
    }

    /// A live `/permission-mode` flip must move the REAL sandbox with the
    /// adapter's auto-accept — both directions — or the UI lies about what the
    /// session can do.
    #[test]
    fn live_permission_mode_flip_moves_the_sandbox_too() {
        let live = Arc::new(AtomicBool::new(true));
        let p = TurnPolicy::new(true, live.clone());
        assert_eq!(
            turn_start_request(1, "th", "a", &p)["params"]["sandboxPolicy"],
            json!({ "type": "dangerFullAccess" })
        );

        // yolo → ask: re-tighten explicitly, rather than leaving the thread at
        // danger-full-access while the UI reports "ask".
        live.store(false, Ordering::SeqCst);
        let req = turn_start_request(2, "th", "b", &p);
        assert_eq!(
            req["params"]["sandboxPolicy"],
            json!({ "type": "workspaceWrite" })
        );
        assert_eq!(req["params"]["approvalPolicy"], "on-request");

        // …and back again.
        live.store(true, Ordering::SeqCst);
        assert_eq!(
            turn_start_request(3, "th", "c", &p)["params"]["approvalPolicy"],
            "never"
        );
    }

    /// A session spawned WITHOUT the grant stays on codex's defaults until the
    /// grant actually arrives — we never widen it on our own, and we never
    /// tighten it either (which would be a behaviour change of its own).
    #[test]
    fn a_never_bypassed_session_is_never_stamped() {
        let live = Arc::new(AtomicBool::new(false));
        let p = TurnPolicy::new(true, live.clone());
        assert_eq!(
            turn_start_request(1, "th", "a", &p)["params"],
            json!({ "threadId": "th", "input": [ { "type": "text", "text": "a" } ] })
        );

        // Granted live: now — and only now — we own the posture, in both
        // directions, because we can no longer restore the unknown default.
        live.store(true, Ordering::SeqCst);
        assert_eq!(
            turn_start_request(2, "th", "b", &p)["params"]["sandboxPolicy"],
            json!({ "type": "dangerFullAccess" })
        );
        live.store(false, Ordering::SeqCst);
        assert_eq!(
            turn_start_request(3, "th", "c", &p)["params"]["sandboxPolicy"],
            json!({ "type": "workspaceWrite" })
        );
    }

    #[test]
    fn model_info_preserves_model_specific_effort_ids() {
        let row = json!({
            "model": "gpt-5.5",
            "displayName": "GPT-5.5",
            "isDefault": true,
            "defaultReasoningEffort": "xhigh",
            "supportedReasoningEfforts": [
                { "reasoningEffort": "low", "description": "Fast" },
                { "reasoningEffort": "medium", "description": "Balanced" },
                { "reasoningEffort": "high", "description": "Deep" },
                { "reasoningEffort": "xhigh", "description": "Extra deep" }
            ]
        });

        assert_eq!(
            model_info_from_value(&row),
            Some(ModelInfo {
                id: "gpt-5.5".into(),
                label: "GPT-5.5".into(),
                default: true,
                effort_levels: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
                default_effort: Some("xhigh".into()),
            })
        );
    }

    /// The default level is per model, not per CLI — verified against the
    /// installed binary, which reports 'medium' for gpt-5.6-sol and 'xhigh' for
    /// gpt-5.5. A row that omits it leaves the composer's "Default" row unnamed
    /// rather than guessing a level.
    #[test]
    fn model_info_default_effort_is_absent_when_the_row_omits_it() {
        let row = json!({ "model": "m", "displayName": "M" });
        assert_eq!(model_info_from_value(&row).unwrap().default_effort, None);
    }

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

    // REGRESSION. A turn the API rejected ends on `turn/completed` with
    // `turn.status: "failed"` and a `turn.error.message` — this build's
    // app-server has no `turn/failed` notification at all. Translating that to
    // a bare `Idle` threw the reason away, and the operator saw a codex
    // session that opened, produced no assistant turn and ended: exactly what
    // "the initial message never reached it" looks like, for a message that
    // was in fact delivered verbatim. This is the REAL payload, from the
    // rollout of the reproduction probe.
    #[test]
    fn a_failed_turn_on_turn_completed_surfaces_the_error() {
        let p = json!({
            "threadId": "01a0395f",
            "turn": {
                "id": "01a0395f-e9fd",
                "items": [],
                "status": "failed",
                "error": {
                    "message": "The 'opus[1m]' model is not supported when using Codex with a ChatGPT account.",
                    "codexErrorInfo": "other"
                }
            }
        });
        assert_eq!(
            translate("turn/completed", &p),
            vec![
                AgentUpdate::Error(
                    "The 'opus[1m]' model is not supported when using Codex with a ChatGPT account."
                        .into()
                ),
                AgentUpdate::Idle
            ]
        );
    }

    // A failed turn with no message attached still says something — silence is
    // the failure mode being fixed.
    #[test]
    fn a_failed_turn_with_no_message_still_reports_a_failure() {
        let p = json!({ "turn": { "id": "t", "items": [], "status": "failed" } });
        assert_eq!(
            translate("turn/completed", &p),
            vec![AgentUpdate::Error("turn failed".into()), AgentUpdate::Idle]
        );
    }

    // The ordinary paths stay quiet: a normal completion and an interrupt are
    // not errors, and must not start showing a ⚠️ item.
    #[test]
    fn a_completed_or_interrupted_turn_reports_no_error() {
        for status in ["completed", "interrupted"] {
            let p = json!({ "turn": { "id": "t", "items": [], "status": status } });
            assert_eq!(
                translate("turn/completed", &p),
                vec![AgentUpdate::Idle],
                "status {status} must not surface an error"
            );
        }
    }

    // The out-of-band `error` notification: terminal ones surface, retryable
    // ones stay quiet, and neither ends the turn (`turn/completed` does).
    #[test]
    fn a_terminal_error_notification_surfaces_without_ending_the_turn() {
        let p = json!({
            "threadId": "t", "turnId": "u", "willRetry": false,
            "error": { "message": "stream disconnected" }
        });
        assert_eq!(
            translate("error", &p),
            vec![AgentUpdate::Error("stream disconnected".into())]
        );
    }

    #[test]
    fn a_retryable_error_notification_is_not_surfaced() {
        let p = json!({
            "threadId": "t", "turnId": "u", "willRetry": true,
            "error": { "message": "transient" }
        });
        assert_eq!(translate("error", &p), vec![]);
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
                // Input side only — output/reasoning don't occupy the window.
                context_tokens: Some(1000),
                context_window: Some(272000),
            }]
        );
    }

    #[test]
    fn token_usage_thread_shape_uses_last_for_context() {
        // Modern `ThreadTokenUsage` wire: cumulative `total`, per-request
        // `last`. Tokens readout = total; context occupancy = last's INPUT
        // side (the cumulative total pinned the meter at 100%; last's
        // totalTokens counted output + reasoning, which don't carry forward).
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
                context_tokens: Some(132153),
                context_window: Some(258400),
            }]
        );
    }

    /// The notification echoes the whole settings block, so it confirms both
    /// fields — including an effort change made in Codex's own TUI, which is the
    /// only way we'd ever hear about that one.
    #[test]
    fn thread_settings_updated_yields_effort_and_model() {
        let p = json!({ "threadId": "t1", "threadSettings": { "model": "gpt-5.5-codex", "effort": "high" } });
        assert_eq!(
            translate("thread/settings/updated", &p),
            vec![
                AgentUpdate::Effort("high".into()),
                AgentUpdate::Usage {
                    model: Some("gpt-5.5-codex".into()),
                    input_tokens: None,
                    output_tokens: None,
                    cached_input_tokens: None,
                    cost_usd: None,
                    context_tokens: None,
                    context_window: None,
                }
            ]
        );
    }

    #[test]
    fn thread_settings_updated_with_only_effort_yields_only_effort() {
        let p = json!({ "threadId": "t1", "threadSettings": { "effort": "xhigh" } });
        assert_eq!(
            translate("thread/settings/updated", &p),
            vec![AgentUpdate::Effort("xhigh".into())]
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
                // Codex reports each window's own length; it rides through so
                // clients can say "5 hours" rather than assume the slot's name.
                five_hour_window_minutes: Some(300),
                seven_day_pct: Some(3.0),
                seven_day_resets_at: Some(1783708145),
                seven_day_window_minutes: Some(10_080),
                monthly_pct: None,
                monthly_resets_at: None,
                monthly_window_minutes: None,
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
    fn turn_plan_updated_yields_plan_with_camelcase_status() {
        use crate::session::state::PlanStatus;
        let p = json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "explanation": null,
            "plan": [
                { "step": "inspect schema", "status": "completed" },
                { "step": "wire updates", "status": "inProgress" },
                { "step": "run tests", "status": "pending" }
            ]
        });
        let updates = translate("turn/plan/updated", &p);
        assert_eq!(updates.len(), 1);
        match &updates[0] {
            AgentUpdate::Plan(plan) => {
                assert_eq!(plan.steps.len(), 3);
                assert_eq!(plan.steps[0].content, "inspect schema");
                assert_eq!(plan.steps[0].status, PlanStatus::Completed);
                assert_eq!(plan.steps[1].content, "wire updates");
                assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
                assert_eq!(plan.steps[2].content, "run tests");
                assert_eq!(plan.steps[2].status, PlanStatus::Pending);
            }
            other => panic!("expected Plan, got {other:?}"),
        }
    }

    #[test]
    fn collab_spawn_agent_yields_tool_card_and_subagent_row() {
        let p = json!({ "item": {
            "type": "collabAgentToolCall",
            "id": "call-1",
            "tool": "spawnAgent",
            "status": "inProgress",
            "senderThreadId": "parent",
            "receiverThreadIds": ["child-1"],
            "agentsStates": {},
            "prompt": "inspect the project",
            "model": "gpt-5.5-codex"
        }});
        let updates = translate("item/started", &p);
        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates[0],
            AgentUpdate::Subagent(crate::session::state::SubagentUpdate {
                id: "child-1".into(),
                agent_type: Some("codex".into()),
                status: crate::session::state::SubagentStatus::Running,
                description: Some("inspect the project".into()),
                tool_use_id: Some("call-1".into()),
                model: Some("gpt-5.5-codex".into()),
                last_tool_name: Some("spawnAgent".into()),
                last_tool_summary: None,
            })
        );
        match &updates[1] {
            AgentUpdate::ToolUse { id, name, input } => {
                assert_eq!(id, "call-1");
                assert_eq!(name, "Agent");
                assert_eq!(input["prompt"], "inspect the project");
                assert_eq!(input["receiverThreadIds"][0], "child-1");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn subagent_activity_completed_closes_row() {
        let p = json!({ "item": {
            "type": "subAgentActivity",
            "id": "activity-1",
            "agentPath": "/home/user/.codex/agents/reviewer.md",
            "agentThreadId": "child-1",
            "kind": "completed"
        }});
        assert_eq!(
            translate("item/completed", &p),
            vec![AgentUpdate::Subagent(
                crate::session::state::SubagentUpdate {
                    id: "child-1".into(),
                    agent_type: Some("reviewer".into()),
                    status: crate::session::state::SubagentStatus::Complete,
                    description: Some("/home/user/.codex/agents/reviewer.md".into()),
                    tool_use_id: None,
                    model: None,
                    last_tool_name: Some("subagent completed".into()),
                    last_tool_summary: None,
                }
            )]
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
        match store.get(session_id).unwrap().pending() {
            Some(Pending::Approval { summary, .. }) => summary.clone(),
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
        let policy = policy_for(true, false);
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
            &policy,
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
            let policy = policy_for(headless, false);
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
                &policy,
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
        let policy = policy_for(true, false);
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
            &policy,
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
    fn id_101_after_thread_started_is_not_treated_as_thread_start() {
        // `req_id` (user turns + live model/effort switches) only ever increments,
        // so a long-lived headless session eventually hands a turn the id 101 — the
        // very id used ONCE at bootstrap for `thread/start`. That turn's JSON-RPC
        // response must NOT be misrouted through the id-101 thread/start handler,
        // which would surface a spurious "thread/start failed" into the conversation.
        use crate::session::conversation::ConversationItem;
        let store = SessionStore::new();
        store.register_managed("s-101", "/w", "codex");
        let conv = ConversationStore::new();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
        // Thread already established and subscribed — bootstrap is long done.
        let mut thread_id = Some("t-live".to_string());
        let mut subscribed = true;
        let mut pending_prompts: Vec<String> = Vec::new();
        // The counter has climbed to 101 across ~98 prior turns/switches.
        let mut req_id = 101u64;
        let mut cur_mode = SessionMode::Responding;
        let mut acc = UsageAcc::new();
        let policy = policy_for(true, false);
        let mut pending_approvals: VecDeque<ParkedApproval> = VecDeque::new();
        let mut pending_switch = None;

        // A turn sent with id 101 fails; Codex replies with an error at id 101.
        handle_message(
            &json!({ "jsonrpc": "2.0", "id": 101,
                "error": { "code": -1, "message": "turn boom" } }),
            &store,
            &conv,
            "s-101",
            &out_tx,
            &mut thread_id,
            &mut subscribed,
            &mut pending_prompts,
            &mut req_id,
            &mut cur_mode,
            &mut acc,
            &policy,
            &mut pending_approvals,
            &mut pending_switch,
            true,
        );

        // The live thread must be left intact…
        assert_eq!(thread_id.as_deref(), Some("t-live"));
        assert!(subscribed);
        // …and NO spurious "thread/start failed" may land in the conversation.
        if let Some((_seq, items)) = conv.snapshot("s-101") {
            assert!(
                !items.iter().any(|i| matches!(
                    i,
                    ConversationItem::AssistantText { text, .. } if text.contains("thread/start failed")
                )),
                "id-101 turn response must not surface a thread/start failure: {items:?}"
            );
        }
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
        let policy = policy_for(false, false);
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
                    &policy,
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
        let policy = policy_for(false, true);
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
            &policy,
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

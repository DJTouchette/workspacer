//! Google Antigravity CLI (`agy`) adapter — drives `agy acp` (Agent Client
//! Protocol JSON-RPC 2.0 over stdio) and translates its session events into
//! claudemon's shared session model.
//!
//! # Transport: ACP (`agy acp`)
//!
//! `agy acp` speaks newline-delimited JSON-RPC 2.0 on stdin/stdout:
//!   - `initialize` → negotiates protocol v1 and advertises capabilities
//!     (`loadSession`, `promptCapabilities.image`, `_meta.antigravity.systemPrompt.append`).
//!   - `session/new` → creates a session with `cwd`, `mcpServers`, and optional
//!     `_meta.antigravity.systemPrompt.append`, returning `sessionId`, `models`,
//!     `modes` (`default`, `accept-edits`, `auto`, `plan`), and `configOptions`
//!     (`model`, `thought_level`, `mode`).
//!   - `session/load` → resumes a prior ACP session by ID and replays its history.
//!   - `session/prompt` → runs a turn (text + inline base64 image blocks); the
//!     response carries `stopReason` and per-turn `usage`.
//!   - `session/set_config_option` / `session/set_mode` → live model, thinking
//!     effort, and permission mode switches.
//!   - `session/cancel` → interrupts an active turn.
//!   - `session/request_permission` (agent → client request) → interactive tool
//!     approval parked in the session's pending approval card.
//!   - `session/update` (agent → client notification) → streaming text,
//!     thoughts/reasoning, tool calls & updates, plans, usage/context occupancy,
//!     and config/mode changes.
//!
//! All wire shapes below were captured live against Antigravity ACP Server
//! 1.5.13 (`agy.exe`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use directories::BaseDirs;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::background_process::BackgroundCommand;
use crate::child_env::SanitizeChildEnvironment;

use super::{apply_updates, note_user_send, set_mode, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::session::conversation::ConversationItem;
use crate::session::slash_commands::{from_acp_commands, SlashCommand};
use crate::session::state::{
    Pending, PendingOwner, PendingWrite, Plan, PlanStatus, PlanStep, SessionMode,
};
use crate::session::store::{
    ConfirmedModelSwitch, Delivery, ManagedDecision, ManagedPermissionSwitch, ManagedPrompt,
    ManagedSessionControl, SessionControlAction,
};
use crate::session::{ConversationStore, ModelSwitch, SessionStore};

/// Spawn-time configuration for an Antigravity ACP session.
#[derive(Clone)]
pub struct SpawnConfig {
    pub session_id: String,
    pub cwd: String,
    /// Resolved `agy` binary (falls back to `"agy"`).
    pub bin: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Initial permission mode (`default`, `acceptEdits`, `auto`, `plan`, `bypassPermissions`).
    pub permission_mode: Option<String>,
    /// Resume a prior Workspacer session via its persisted ACP session ID.
    pub resume: Option<String>,
    pub extra_args: Vec<String>,
    /// Extra skill directories (Workspacer role skills) exposed natively to
    /// `agy acp` via `--add-dir` + `.agents/skills.json`.
    pub skill_roots: Vec<String>,
    pub env: HashMap<String, String>,
    pub yolo: bool,
    pub facade: Facade,
}

/// Known effort suffixes appended to Antigravity `availableModels` IDs.
const EFFORT_SUFFIXES: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

// ── Session sidecar (Workspacer session_id → ACP sessionId) ──────────────────

fn sessions_dir_path() -> Option<PathBuf> {
    Some(
        BaseDirs::new()?
            .home_dir()
            .join(".workspacer")
            .join("antigravity-sessions"),
    )
}

fn sessions_dir() -> Option<PathBuf> {
    let dir = sessions_dir_path()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Persist which ACP session ID backs a Workspacer session (for resume).
pub fn record_acp_session(session_id: &str, acp_session_id: &str) {
    if !crate::daemon::api::valid_session_id(session_id) || acp_session_id.trim().is_empty() {
        return;
    }
    let Some(dir) = sessions_dir() else { return };
    let path = dir.join(format!("{session_id}.json"));
    let tmp = dir.join(format!("{session_id}.{}.tmp", std::process::id()));
    let body = json!({ "acp_session_id": acp_session_id }).to_string();
    if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Look up the ACP session ID recorded for a prior Workspacer session.
pub fn acp_session_for(session_id: &str) -> Option<String> {
    if !crate::daemon::api::valid_session_id(session_id) {
        return None;
    }
    let path = sessions_dir_path()?.join(format!("{session_id}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()?
        .get("acp_session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Materialize an app-owned wrapper directory containing `.agents/skills.json`
/// pointing at `skill_roots`, so `agy acp --add-dir <wrapper>` natively loads
/// Workspacer's per-role skills (e.g. `spawn-agent`, `handoff`, `checkpoint`)
/// without writing anything into the user's project directory.
fn prepare_skill_add_dir(skill_roots: &[String]) -> Option<PathBuf> {
    let base = BaseDirs::new()?
        .home_dir()
        .join(".workspacer")
        .join("antigravity-skill-roots");
    prepare_skill_add_dir_in(&base, skill_roots)
}

fn prepare_skill_add_dir_in(base: &std::path::Path, skill_roots: &[String]) -> Option<PathBuf> {
    let mut normalized: Vec<String> = Vec::new();
    for root in skill_roots {
        let trimmed = root.trim();
        if trimmed.is_empty() {
            continue;
        }
        let slash = trimmed.replace('\\', "/");
        if !normalized.contains(&slash) {
            normalized.push(slash);
        }
    }
    if normalized.is_empty() {
        return None;
    }
    // Stable FNV-1a 64-bit hash over the ordered root list so repeated launches
    // reuse the same wrapper directory cleanly.
    let mut hash: u64 = 0xcbf29ce484222325;
    for root in &normalized {
        for b in root.as_bytes() {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let wrapper = base.join(format!("{hash:016x}"));
    let agents_dir = wrapper.join(".agents");
    std::fs::create_dir_all(&agents_dir).ok()?;
    let manifest_path = agents_dir.join("skills.json");
    let entries: Vec<Value> = normalized
        .into_iter()
        .map(|p| json!({ "path": p }))
        .collect();
    let body = serde_json::to_string_pretty(&json!({ "entries": entries })).ok()?;
    if std::fs::read_to_string(&manifest_path).ok().as_deref() != Some(body.as_str()) {
        let tmp = agents_dir.join(format!("skills.{}.tmp", std::process::id()));
        if std::fs::write(&tmp, &body).is_ok() && std::fs::rename(&tmp, &manifest_path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
    }
    Some(wrapper)
}

// ── Mode normalization ───────────────────────────────────────────────────────

/// Convert Workspacer's permission mode spelling to Antigravity ACP's `modeId`.
/// Returns `(acp_mode_id, yolo_flag, canonical_wire_mode)`.
pub fn map_permission_mode(mode: &str) -> Option<(&'static str, bool, &'static str)> {
    match mode.trim() {
        "default" | "ask" => Some(("default", false, "default")),
        "acceptEdits" | "accept-edits" => Some(("accept-edits", false, "acceptEdits")),
        "auto" => Some(("auto", false, "auto")),
        "plan" => Some(("plan", false, "plan")),
        "bypassPermissions" | "yolo" => Some(("auto", true, "bypassPermissions")),
        _ => None,
    }
}

/// Convert an ACP `modeId` back to Workspacer's canonical observed permission mode.
pub fn observed_mode_from_acp(mode_id: &str, yolo_live: bool) -> &'static str {
    if yolo_live && mode_id != "plan" {
        return "bypassPermissions";
    }
    match mode_id.trim() {
        "accept-edits" | "acceptEdits" => "acceptEdits",
        "auto" => "auto",
        "plan" => "plan",
        _ => "default",
    }
}

/// Split a compound Antigravity model ID like `"gemini-4-neon-xhigh"` into
/// `("gemini-4-neon", "xhigh")` when it ends with a known effort suffix.
pub fn split_compound_model(model: &str) -> Option<(&str, &str)> {
    for suffix in EFFORT_SUFFIXES {
        let needle = format!("-{suffix}");
        if let Some(base) = model.strip_suffix(&needle) {
            if !base.is_empty() {
                return Some((base, suffix));
            }
        }
    }
    None
}

// ── Model catalog discovery ──────────────────────────────────────────────────

/// Parse `session/new`'s result into Workspacer's [`ModelInfo`] catalog.
///
/// Prefers `configOptions[id="model"]` (clean base model families) enriched
/// with per-model `effort_levels` derived from `models.availableModels`
/// (`<base>-<effort>` entries) and `default_effort` from
/// `configOptions[id="thought_level"]`. Falls back to `models.availableModels`
/// directly if `configOptions` omits the model selector.
pub fn parse_acp_models(result: &Value) -> Vec<ModelInfo> {
    let available: Vec<(&str, &str)> = result
        .get("models")
        .and_then(|m| m.get("availableModels"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|row| {
                    let id = row.get("modelId").and_then(Value::as_str)?.trim();
                    if id.is_empty() {
                        return None;
                    }
                    let name = row
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(id);
                    Some((id, name))
                })
                .collect()
        })
        .unwrap_or_default();

    let available_ids: HashSet<&str> = available.iter().map(|(id, _)| *id).collect();
    let config_options = result
        .get("configOptions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let model_opt = config_options
        .iter()
        .find(|o| o.get("id").and_then(Value::as_str) == Some("model"));
    let thought_opt = config_options
        .iter()
        .find(|o| o.get("id").and_then(Value::as_str) == Some("thought_level"));
    let current_thought = thought_opt
        .and_then(|o| o.get("currentValue").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    if let Some(options) = model_opt
        .and_then(|o| o.get("options"))
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty())
    {
        let current_model = model_opt
            .and_then(|o| o.get("currentValue").and_then(Value::as_str))
            .unwrap_or("");
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for opt in options {
            let Some(id) = opt
                .get("value")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let label = opt
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id)
                .to_owned();
            let is_default = id == current_model;
            let effort_levels: Vec<String> = EFFORT_SUFFIXES
                .iter()
                .filter(|suffix| available_ids.contains(format!("{id}-{suffix}").as_str()))
                .map(|s| (*s).to_owned())
                .collect();
            let default_effort = if is_default {
                current_thought
                    .as_ref()
                    .filter(|e| effort_levels.iter().any(|l| l == *e))
                    .cloned()
            } else {
                None
            };
            out.push(ModelInfo {
                id: id.to_owned(),
                label,
                default: is_default,
                effort_levels,
                default_effort,
                default_context_window: None,
                max_context_window: None,
                effective_context_window_percent: None,
            });
        }
        if !out.is_empty() {
            return out;
        }
    }

    // Fallback: flat list from `models.availableModels`.
    let current_id = result
        .get("models")
        .and_then(|m| m.get("currentModelId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut seen = HashSet::new();
    available
        .into_iter()
        .filter(|(id, _)| seen.insert((*id).to_owned()))
        .map(|(id, name)| ModelInfo {
            id: id.to_owned(),
            label: name.to_owned(),
            default: id == current_id,
            effort_levels: Vec::new(),
            default_effort: None,
            default_context_window: None,
            max_context_window: None,
            effective_context_window_percent: None,
        })
        .collect()
}

/// Live-query available Antigravity models via a short-lived `agy acp`
/// handshake (`initialize` → `session/new` → `session/close`), cached with the
/// shared provider model cache.
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    let key = format!("antigravity:{bin}");
    let bin = bin.to_owned();
    let cwd = cwd.to_owned();
    super::cached_or_refresh(key, async move { fetch_models(&bin, &cwd).await }).await
}

async fn fetch_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    let mut cmd = Command::new(bin);
    cmd.arg("acp")
        .current_dir(cwd)
        .scrub_host_authority()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .no_console_window();

    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to launch `{bin} acp`"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("agy acp stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("agy acp stdout unavailable"))?;
    let mut lines = BufReader::new(stdout).lines();

    let work = async {
        write_json_line(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": { "name": "workspacer", "version": env!("CARGO_PKG_VERSION") }
                }
            }),
        )
        .await?;
        let init_reply = read_rpc_reply(&mut lines, 1).await?;
        if let Some(err) = init_reply.get("error") {
            anyhow::bail!("agy acp initialize failed: {err}");
        }

        write_json_line(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session/new",
                "params": { "cwd": cwd, "mcpServers": [] }
            }),
        )
        .await?;
        let new_reply = read_rpc_reply(&mut lines, 2).await?;
        if let Some(err) = new_reply.get("error") {
            anyhow::bail!("agy acp session/new failed: {err}");
        }
        let result = new_reply.get("result").cloned().unwrap_or(Value::Null);
        let models = parse_acp_models(&result);
        if let Some(sid) = result.get("sessionId").and_then(Value::as_str) {
            let _ = write_json_line(
                &mut stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "session/close",
                    "params": { "sessionId": sid }
                }),
            )
            .await;
            let _ =
                tokio::time::timeout(Duration::from_secs(2), read_rpc_reply(&mut lines, 3)).await;
        }
        Ok::<Vec<ModelInfo>, anyhow::Error>(models)
    };

    let outcome = tokio::time::timeout(Duration::from_secs(20), work)
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for `{bin} acp` model catalog"))?;
    let _ = child.kill().await;
    outcome
}

async fn read_rpc_reply(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    expected_id: u64,
) -> anyhow::Result<Value> {
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if val.get("id").and_then(Value::as_u64) == Some(expected_id) {
            return Ok(val);
        }
    }
    anyhow::bail!("agy acp closed stdout before replying to request {expected_id}")
}

async fn write_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &Value,
) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}

// ── Pure translation ─────────────────────────────────────────────────────────

/// Per-session translation state for deduplicating tool cards and scoping
/// streamed thought/reasoning blocks.
#[derive(Debug, Default)]
pub struct TranslateState {
    pub current_model: Option<String>,
    pub last_context_tokens: Option<u64>,
    /// Ordered ACP `toolCallId`s of `invoke_subagent` calls, used to anchor
    /// discovered subagent rows onto their spawning tool card in the UI.
    pub subagent_tool_ids: Vec<String>,
    thought_seq: u64,
    active_thought_id: Option<String>,
    emitted_tool_uses: HashSet<String>,
    emitted_tool_results: HashSet<String>,
}

impl TranslateState {
    fn next_thought_id(&mut self) -> String {
        if let Some(id) = &self.active_thought_id {
            return id.clone();
        }
        self.thought_seq += 1;
        let id = format!("agy-thought-{}", self.thought_seq);
        self.active_thought_id = Some(id.clone());
        id
    }

    pub fn close_thought_block(&mut self) {
        self.active_thought_id = None;
    }
}

/// Extract text from an ACP `content` payload (either `{type:"text", text:"..."}`
/// or an array of content blocks).
fn extract_content_text(content: &Value) -> String {
    match content {
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn extract_tool_input(update: &Value) -> Value {
    if let Some(raw) = update.get("rawInput").filter(|v| !v.is_null()) {
        return raw.clone();
    }
    if let Some(args) = update.get("args").filter(|v| !v.is_null()) {
        return args.clone();
    }
    json!({})
}

fn extract_tool_output(update: &Value) -> String {
    if let Some(raw) = update.get("rawOutput").filter(|v| !v.is_null()) {
        if let Some(s) = raw.as_str() {
            return s.to_owned();
        }
        if let Some(s) = raw.get("output").and_then(Value::as_str) {
            return s.to_owned();
        }
        if let Some(s) = raw.get("error").and_then(Value::as_str) {
            return s.to_owned();
        }
        if let Some(s) = raw.get("content").and_then(Value::as_str) {
            return s.to_owned();
        }
        return raw.to_string();
    }
    if let Some(content) = update.get("content").filter(|v| !v.is_null()) {
        // ACP `tool_call_update.content` may be `[{type:"content", content:{type:"text", text:"..."}}]`
        if let Some(arr) = content.as_array() {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|entry| {
                    if let Some(inner) = entry.get("content") {
                        let t = extract_content_text(inner);
                        if !t.is_empty() {
                            return Some(t);
                        }
                    }
                    let t = extract_content_text(entry);
                    (!t.is_empty()).then_some(t)
                })
                .collect();
            if !parts.is_empty() {
                return parts.join("\n");
            }
        }
        let text = extract_content_text(content);
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

/// Metadata side-effects observed on a `session/update` notification that
/// update session state outside the `AgentUpdate` enum (such as observed
/// permission mode or session title).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UpdateMeta {
    pub observed_mode: Option<String>,
    pub title: Option<String>,
    pub model: Option<String>,
    pub slash_commands: Option<Vec<SlashCommand>>,
}

/// Translate one ACP `session/update` payload (`params.update`) into
/// [`AgentUpdate`]s and side-effect metadata. Pure and unit-tested.
pub fn translate_update(
    update: &Value,
    state: &mut TranslateState,
    replaying: bool,
) -> (Vec<AgentUpdate>, UpdateMeta) {
    let mut out = Vec::new();
    let mut meta = UpdateMeta::default();
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("");

    match kind {
        "agent_message_chunk" => {
            state.close_thought_block();
            let text = update
                .get("content")
                .map(extract_content_text)
                .unwrap_or_default();
            if !text.is_empty() {
                if !replaying {
                    out.push(AgentUpdate::Busy);
                }
                out.push(AgentUpdate::AssistantText(text));
            }
        }
        "agent_thought_chunk" => {
            let text = update
                .get("content")
                .map(extract_content_text)
                .unwrap_or_default();
            if !text.is_empty() {
                if !replaying {
                    out.push(AgentUpdate::Busy);
                }
                let item_id = state.next_thought_id();
                out.push(AgentUpdate::Reasoning {
                    item_id,
                    text,
                    complete: false,
                });
            }
        }
        "user_message_chunk" => {
            state.close_thought_block();
            // Live user prompts are recorded immediately via `note_user_send`;
            // `user_message_chunk` only arrives during `session/load` replay.
            if replaying {
                let text = update
                    .get("content")
                    .map(extract_content_text)
                    .unwrap_or_default();
                if !text.is_empty() {
                    out.push(AgentUpdate::UserText(text));
                }
            }
        }
        "tool_call" | "tool_call_update" => {
            state.close_thought_block();
            let id = update
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if !id.is_empty() {
                if !replaying {
                    out.push(AgentUpdate::Busy);
                }
                if state.emitted_tool_uses.insert(id.clone()) {
                    let name = update
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| update.get("title").and_then(Value::as_str))
                        .or_else(|| update.get("kind").and_then(Value::as_str))
                        .unwrap_or("tool")
                        .to_owned();
                    if name.eq_ignore_ascii_case("invoke_subagent") {
                        state.subagent_tool_ids.push(id.clone());
                    }
                    let input = extract_tool_input(update);
                    out.push(AgentUpdate::ToolUse {
                        id: id.clone(),
                        name,
                        input,
                    });
                }
                let status = update.get("status").and_then(Value::as_str).unwrap_or("");
                if matches!(status, "completed" | "failed")
                    && state.emitted_tool_results.insert(id.clone())
                {
                    let content = extract_tool_output(update);
                    out.push(AgentUpdate::ToolResult {
                        tool_use_id: id,
                        content,
                        is_error: status == "failed",
                    });
                }
            }
        }
        "plan" => {
            if let Some(entries) = update.get("entries").and_then(Value::as_array) {
                let steps: Vec<PlanStep> = entries
                    .iter()
                    .filter_map(|entry| {
                        let content = entry
                            .get("content")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())?
                            .to_owned();
                        let status = entry
                            .get("status")
                            .and_then(Value::as_str)
                            .map(PlanStatus::from_wire)
                            .unwrap_or(PlanStatus::Pending);
                        Some(PlanStep {
                            content,
                            status,
                            active_form: None,
                        })
                    })
                    .collect();
                if !steps.is_empty() {
                    out.push(AgentUpdate::Plan(Plan {
                        steps,
                        updated_at: None,
                    }));
                }
            }
        }
        "usage_update" => {
            let used = update.get("used").and_then(Value::as_u64);
            let size = update
                .get("size")
                .and_then(Value::as_u64)
                .filter(|s| *s > 0);
            if let Some(u) = used {
                state.last_context_tokens = Some(u);
            }
            if used.is_some() || size.is_some() {
                out.push(AgentUpdate::Usage {
                    model: state.current_model.clone(),
                    input_tokens: None,
                    output_tokens: None,
                    cached_input_tokens: None,
                    cost_usd: None,
                    context_tokens: used,
                    context_window: size,
                });
            }
        }
        "available_commands_update" => {
            meta.slash_commands = Some(from_acp_commands(update));
        }
        "current_mode_update" => {
            if let Some(mode_id) = update
                .get("currentModeId")
                .or_else(|| update.get("modeId"))
                .and_then(Value::as_str)
            {
                meta.observed_mode = Some(mode_id.to_owned());
            }
        }
        "config_option_update" => {
            if let Some(options) = update.get("configOptions").and_then(Value::as_array) {
                observe_config_options(options, state, &mut out, &mut meta);
            }
        }
        "session_info_update" => {
            if let Some(title) = update
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                meta.title = Some(title.to_owned());
            }
        }
        _ => {}
    }

    (out, meta)
}

fn observe_config_options(
    options: &[Value],
    state: &mut TranslateState,
    out: &mut Vec<AgentUpdate>,
    meta: &mut UpdateMeta,
) {
    for opt in options {
        let id = opt.get("id").and_then(Value::as_str).unwrap_or("");
        let val = opt
            .get("currentValue")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match (id, val) {
            ("model", Some(model)) => {
                state.current_model = Some(model.to_owned());
                meta.model = Some(model.to_owned());
                out.push(AgentUpdate::Usage {
                    model: Some(model.to_owned()),
                    input_tokens: None,
                    output_tokens: None,
                    cached_input_tokens: None,
                    cost_usd: None,
                    context_tokens: None,
                    context_window: None,
                });
            }
            ("thought_level", Some(effort)) => {
                out.push(AgentUpdate::Effort(effort.to_owned()));
            }
            ("mode", Some(mode)) => {
                meta.observed_mode = Some(mode.to_owned());
            }
            _ => {}
        }
    }
}

/// Extract per-turn token usage from a `session/prompt` JSON-RPC result.
pub fn extract_prompt_usage(result: &Value, current_model: Option<&str>) -> Option<AgentUpdate> {
    let usage = result.get("usage")?;
    let input = usage.get("inputTokens").and_then(Value::as_u64);
    let output = usage.get("outputTokens").and_then(Value::as_u64);
    let cached = usage.get("cachedReadTokens").and_then(Value::as_u64);
    let thought = usage
        .get("thoughtTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_output = output.map(|o| o.saturating_add(thought));
    if input.is_none() && total_output.is_none() && cached.is_none() {
        return None;
    }
    Some(AgentUpdate::Usage {
        model: current_model.map(str::to_owned),
        input_tokens: input,
        output_tokens: total_output,
        cached_input_tokens: cached,
        cost_usd: None,
        context_tokens: None,
        context_window: None,
    })
}

// ── Permission requests ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AcpPermOption {
    option_id: String,
    name: String,
    kind: String,
}

#[derive(Debug, Clone)]
struct ParkedPermission {
    rpc_id: Value,
    pending_id: String,
    tool: Option<String>,
    summary: Option<String>,
    raw: Value,
    options: Vec<AcpPermOption>,
    /// Mapping from `always_options[].id` (`"<pending_id>:<index>"`) to ACP `optionId`.
    always_map: Vec<(String, String)>,
}

fn parse_permission_request(rpc_id: Value, params: &Value) -> ParkedPermission {
    let pending_id = match &rpc_id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let tool_call = params.get("toolCall").unwrap_or(&Value::Null);
    let tool = tool_call
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| tool_call.get("kind").and_then(Value::as_str))
        .map(str::to_owned);
    let summary = tool_call
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let tool_input = extract_tool_input(tool_call);

    let options: Vec<AcpPermOption> = params
        .get("options")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let option_id = o.get("optionId").and_then(Value::as_str)?.to_owned();
                    let name = o
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&option_id)
                        .to_owned();
                    let kind = o
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    Some(AcpPermOption {
                        option_id,
                        name,
                        kind,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let mut always_options = Vec::new();
    let mut always_map = Vec::new();
    for opt in options.iter().filter(|o| o.kind == "allow_always") {
        let idx = always_options.len();
        let id = format!("{pending_id}:{idx}");
        always_options.push(json!({
            "id": id,
            "label": opt.name,
            "scope": "session",
            "kind": "allow_always",
        }));
        always_map.push((id, opt.option_id.clone()));
    }

    let raw = json!({
        "pending_id": pending_id,
        "tool_name": tool,
        "tool_input": tool_input,
        "title": summary,
        "always_options": always_options,
    });

    ParkedPermission {
        rpc_id,
        pending_id,
        tool,
        summary,
        raw,
        options,
        always_map,
    }
}

fn permission_outcome(perm: &ParkedPermission, decision: &ManagedDecision) -> Value {
    if decision.approve {
        if let Some(always_id) = decision.always_option.as_deref() {
            if let Some((_, opt_id)) = perm.always_map.iter().find(|(id, _)| id == always_id) {
                return json!({ "outcome": "selected", "optionId": opt_id });
            }
        }
        let chosen = perm
            .options
            .iter()
            .find(|o| o.kind == "allow_once")
            .or_else(|| perm.options.iter().find(|o| o.kind == "allow_always"))
            .or_else(|| perm.options.first());
        match chosen {
            Some(opt) => json!({ "outcome": "selected", "optionId": opt.option_id }),
            None => json!({ "outcome": "cancelled" }),
        }
    } else {
        let chosen = perm
            .options
            .iter()
            .find(|o| o.kind == "reject_once")
            .or_else(|| perm.options.iter().find(|o| o.kind == "reject_always"));
        match chosen {
            Some(opt) => json!({ "outcome": "selected", "optionId": opt.option_id }),
            None => json!({ "outcome": "cancelled" }),
        }
    }
}

// ── ACP request builders ─────────────────────────────────────────────────────

fn build_mcp_servers(facade: &Facade) -> Value {
    let Some(url) = facade
        .mcp_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!([]);
    };
    json!([{
        "name": "workspacer",
        "type": "http",
        "url": url,
        "headers": [],
    }])
}

fn build_session_new_params(cwd: &str, facade: &Facade) -> Value {
    let mut params = json!({
        "cwd": cwd,
        "mcpServers": build_mcp_servers(facade),
    });
    if let Some(instructions) = facade
        .instructions
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        params["_meta"] = json!({
            "antigravity": {
                "systemPrompt": {
                    "append": instructions,
                }
            }
        });
    }
    params
}

fn build_prompt_blocks(prompt: &ManagedPrompt) -> Vec<Value> {
    let mut blocks = Vec::new();
    for img in &prompt.images {
        if !img.data.is_empty() && !img.media_type.is_empty() {
            blocks.push(json!({
                "type": "image",
                "mimeType": img.media_type,
                "data": img.data,
            }));
        }
    }
    let text = if prompt.images.iter().any(|img| img.data.is_empty()) {
        crate::session::images::with_markers(
            prompt.text.clone(),
            &prompt
                .images
                .iter()
                .filter(|img| img.data.is_empty())
                .cloned()
                .collect::<Vec<_>>(),
        )
    } else {
        prompt.text.clone()
    };
    if !text.is_empty() || blocks.is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": text,
        }));
    }
    blocks
}

// ── Live driver ──────────────────────────────────────────────────────────────

struct QueuedPrompt {
    prompt: ManagedPrompt,
    is_compact: bool,
}

enum PendingRpc {
    Initialize,
    SessionNew,
    SessionLoad {
        acp_id: String,
    },
    InitialConfig,
    Prompt {
        is_compact: bool,
        pre_tokens: Option<u64>,
    },
    ModelSwitch {
        remaining_effort: Option<String>,
        fallback_compound: Option<(String, String)>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    EffortSwitch {
        reply: oneshot::Sender<Result<(), String>>,
    },
    PermissionMode {
        canonical: String,
        target_yolo: bool,
        reply: oneshot::Sender<Result<String, String>>,
    },
}

/// Spawn a managed Antigravity ACP session and drive it until termination.
pub fn spawn_session(store: SessionStore, conv: ConversationStore, cfg: SpawnConfig) {
    let generation = store.claim_generation(&cfg.session_id);
    tokio::spawn(async move {
        if let Err(err) = run_driver(store.clone(), conv.clone(), cfg.clone()).await {
            tracing::warn!(session = %cfg.session_id, ?err, "antigravity ACP driver failed");
            let mut mode = SessionMode::Responding;
            let mut acc = UsageAcc::new();
            apply_updates(
                &store,
                &conv,
                &cfg.session_id,
                vec![
                    AgentUpdate::Error(format!("Antigravity session error: {err:#}")),
                    AgentUpdate::Idle,
                ],
                &mut mode,
                &mut acc,
            );
        }
        let _ = store.deregister_managed(&cfg.session_id, generation);
    });
}

async fn run_driver(
    store: SessionStore,
    conv: ConversationStore,
    cfg: SpawnConfig,
) -> anyhow::Result<()> {
    let session_id = cfg.session_id.clone();

    let mut cmd = Command::new(&cfg.bin);
    cmd.arg("acp");
    if cfg.yolo {
        cmd.arg("--dangerously-skip-permissions");
    }
    if let Some(add_dir) = prepare_skill_add_dir(&cfg.skill_roots) {
        cmd.arg("--add-dir").arg(add_dir);
    }
    cmd.args(&cfg.extra_args)
        .current_dir(&cfg.cwd)
        .envs(&cfg.env)
        .scrub_host_authority()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .no_console_window();

    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to launch `{}`", cfg.bin))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("agy acp stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("agy acp stdout unavailable"))?;
    let stderr = child.stderr.take();

    // Drain stderr so the child never blocks on a full pipe; retain a bounded tail for diagnostics.
    let stderr_tail = Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(stderr) = stderr {
        let tail = stderr_tail.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                tracing::debug!(target: "antigravity_acp_stderr", "{line}");
                if let Ok(mut buf) = tail.lock() {
                    if !buf.is_empty() {
                        buf.push('\n');
                    }
                    buf.push_str(&line);
                    if buf.len() > 2048 {
                        let cut = buf.len() - 2048;
                        let boundary = buf.ceil_char_boundary(cut);
                        buf.drain(..boundary);
                    }
                }
            }
        });
    }

    let (prompt_tx, mut prompt_rx) = mpsc::unbounded_channel::<ManagedPrompt>();
    let (dec_tx, mut dec_rx) = mpsc::unbounded_channel::<ManagedDecision>();
    let (model_tx, mut model_rx) = mpsc::unbounded_channel::<ConfirmedModelSwitch>();
    let (perm_tx, mut perm_rx) = mpsc::unbounded_channel::<ManagedPermissionSwitch>();
    let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<ManagedSessionControl>();
    let (int_tx, mut int_rx) = mpsc::unbounded_channel::<()>();
    let yolo_live = Arc::new(AtomicBool::new(cfg.yolo));

    // Register store channels BEFORE booting so queued first messages and live
    // controls arrive cleanly even while `initialize` / `session/new` is in flight.
    store.register_managed_prompt(&session_id, prompt_tx);
    store.register_managed_decision(&session_id, dec_tx);
    store.register_managed_confirmed_model_switch(&session_id, model_tx);
    store.register_managed_permission_mode(&session_id, perm_tx);
    store.register_managed_session_control(&session_id, ctl_tx);
    store.register_managed_interrupt(&session_id, int_tx);
    store.register_managed_yolo(&session_id, yolo_live.clone(), cfg.yolo);

    // Seed initial observed permission mode before the first turn.
    let initial_mode = cfg
        .permission_mode
        .as_deref()
        .and_then(map_permission_mode)
        .unwrap_or_else(|| {
            if cfg.yolo {
                ("auto", true, "bypassPermissions")
            } else {
                ("default", false, "default")
            }
        });
    yolo_live.store(initial_mode.1 || cfg.yolo, Ordering::Relaxed);
    store.set_observed_permission_mode(&session_id, initial_mode.2);

    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    acc.additive();
    acc.estimate_costs();
    acc.seed_model(cfg.model.as_deref());

    let mut translate_state = TranslateState {
        current_model: cfg.model.clone(),
        ..Default::default()
    };

    let mut next_rpc_id: u64 = 1;
    let mut inflight: HashMap<u64, PendingRpc> = HashMap::new();
    let mut acp_session_id: Option<String> = None;
    let mut ready_for_prompts = false;
    let mut in_flight_prompt = false;
    let mut replaying = false;
    let mut suppress_replay = false;
    let mut terminated_by_us = false;
    let mut pending_prompts: VecDeque<QueuedPrompt> = VecDeque::new();
    let mut parked_perms: VecDeque<ParkedPermission> = VecDeque::new();

    // Send `initialize` immediately.
    let init_id = next_rpc_id;
    next_rpc_id += 1;
    inflight.insert(init_id, PendingRpc::Initialize);
    write_json_line(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": { "name": "workspacer", "version": env!("CARGO_PKG_VERSION") }
            }
        }),
    )
    .await?;

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut artifact_tick = tokio::time::interval(Duration::from_secs(1));
    artifact_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_artifact_stamps: Vec<(PathBuf, Option<super::antigravity_artifacts::FileStamp>)> =
        Vec::new();

    loop {
        // Dispatch the next queued prompt whenever ready and idle.
        if ready_for_prompts && !in_flight_prompt {
            if let (Some(sid), Some(queued)) =
                (acp_session_id.as_deref(), pending_prompts.pop_front())
            {
                let QueuedPrompt { prompt, is_compact } = queued;
                note_user_send(&store, &session_id, &mut cur_mode);
                if is_compact {
                    apply_updates(
                        &store,
                        &conv,
                        &session_id,
                        vec![AgentUpdate::Compacting(true)],
                        &mut cur_mode,
                        &mut acc,
                    );
                } else {
                    let display = if prompt.text.is_empty() && !prompt.images.is_empty() {
                        crate::session::images::with_markers(String::new(), &prompt.images)
                    } else {
                        prompt.text.clone()
                    };
                    apply_updates(
                        &store,
                        &conv,
                        &session_id,
                        vec![AgentUpdate::UserText(display)],
                        &mut cur_mode,
                        &mut acc,
                    );
                }
                translate_state.close_thought_block();
                set_mode(
                    &store,
                    &session_id,
                    SessionMode::Responding,
                    PendingWrite::Keep,
                    &mut cur_mode,
                );
                let blocks = build_prompt_blocks(&prompt);
                let rpc_id = next_rpc_id;
                next_rpc_id += 1;
                inflight.insert(
                    rpc_id,
                    PendingRpc::Prompt {
                        is_compact,
                        pre_tokens: translate_state.last_context_tokens,
                    },
                );
                in_flight_prompt = true;
                write_json_line(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": rpc_id,
                        "method": "session/prompt",
                        "params": {
                            "sessionId": sid,
                            "prompt": blocks,
                        }
                    }),
                )
                .await?;
            }
        }

        tokio::select! {
            line = stdout_lines.next_line() => {
                let Some(line) = line? else {
                    break;
                };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                    tracing::debug!(session = %session_id, line = trimmed, "ignoring non-JSON line from agy acp");
                    continue;
                };

                // 1. Agent → client JSON-RPC request (has both `method` and `id`).
                if let (Some(method), Some(req_id)) = (msg.get("method").and_then(Value::as_str), msg.get("id")) {
                    if method == "session/request_permission" {
                        let params = msg.get("params").unwrap_or(&Value::Null);
                        let perm = parse_permission_request(req_id.clone(), params);
                        if yolo_live.load(Ordering::Relaxed) {
                            let outcome = permission_outcome(&perm, &ManagedDecision::from(true));
                            write_json_line(
                                &mut stdin,
                                &json!({
                                    "jsonrpc": "2.0",
                                    "id": perm.rpc_id,
                                    "result": { "outcome": outcome },
                                }),
                            )
                            .await?;
                        } else {
                            let was_empty = parked_perms.is_empty();
                            parked_perms.push_back(perm);
                            if was_empty {
                                if let Some(head) = parked_perms.front() {
                                    apply_updates(
                                        &store,
                                        &conv,
                                        &session_id,
                                        vec![AgentUpdate::PermissionPending {
                                            id: Some(head.pending_id.clone()),
                                            tool: head.tool.clone(),
                                            summary: head.summary.clone(),
                                            raw: head.raw.clone(),
                                        }],
                                        &mut cur_mode,
                                        &mut acc,
                                    );
                                }
                            }
                        }
                    } else {
                        // Unknown server→client request: return MethodNotFound cleanly.
                        let _ = write_json_line(
                            &mut stdin,
                            &json!({
                                "jsonrpc": "2.0",
                                "id": req_id,
                                "error": { "code": -32601, "message": format!("unsupported client method: {method}") }
                            }),
                        )
                        .await;
                    }
                    continue;
                }

                // 2. Agent → client JSON-RPC notification (`method` without `id`).
                if let Some(method) = msg.get("method").and_then(Value::as_str) {
                    if method == "session/update" {
                        if let Some(update) = msg.get("params").and_then(|p| p.get("update")) {
                            let (updates, meta) = translate_update(update, &mut translate_state, replaying);
                            if let Some(mode_id) = meta.observed_mode {
                                let canonical = observed_mode_from_acp(
                                    &mode_id,
                                    yolo_live.load(Ordering::Relaxed),
                                );
                                store.set_observed_permission_mode(&session_id, canonical);
                            }
                            if let Some(commands) = meta.slash_commands {
                                store.set_slash_commands(&session_id, commands);
                            }
                            if let Some(title) = meta.title {
                                store.update_state(&session_id, |s| {
                                    if s.thread_name.as_deref() == Some(title.as_str()) {
                                        return false;
                                    }
                                    s.thread_name = Some(title.clone());
                                    true
                                });
                            }
                            if !updates.is_empty() && !(replaying && suppress_replay) {
                                apply_updates(
                                    &store,
                                    &conv,
                                    &session_id,
                                    updates,
                                    &mut cur_mode,
                                    &mut acc,
                                );
                            }
                        }
                    }
                    continue;
                }

                // 3. Response to one of our JSON-RPC requests (`id` without `method`).
                if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                    let Some(pending) = inflight.remove(&id) else {
                        continue;
                    };
                    let rpc_err = msg.get("error").map(format_rpc_error);
                    let result = msg.get("result").unwrap_or(&Value::Null);

                    match pending {
                        PendingRpc::Initialize => {
                            if let Some(err) = rpc_err {
                                anyhow::bail!("agy acp initialize failed: {err}");
                            }
                            // Attempt `session/load` on resume when we have a recorded ACP session ID.
                            let resume_acp = cfg
                                .resume
                                .as_deref()
                                .and_then(acp_session_for);
                            if let Some(acp_id) = resume_acp {
                                replaying = true;
                                suppress_replay = conv.has_conversation(&session_id);
                                let load_id = next_rpc_id;
                                next_rpc_id += 1;
                                inflight.insert(load_id, PendingRpc::SessionLoad { acp_id: acp_id.clone() });
                                write_json_line(
                                    &mut stdin,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "id": load_id,
                                        "method": "session/load",
                                        "params": {
                                            "sessionId": acp_id,
                                            "cwd": cfg.cwd,
                                            "mcpServers": build_mcp_servers(&cfg.facade),
                                        }
                                    }),
                                )
                                .await?;
                            } else {
                                if cfg.resume.is_some() {
                                    tracing::warn!(session = %session_id, "antigravity resume requested but no ACP session recorded — starting fresh");
                                }
                                let new_id = next_rpc_id;
                                next_rpc_id += 1;
                                inflight.insert(new_id, PendingRpc::SessionNew);
                                write_json_line(
                                    &mut stdin,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "id": new_id,
                                        "method": "session/new",
                                        "params": build_session_new_params(&cfg.cwd, &cfg.facade),
                                    }),
                                )
                                .await?;
                            }
                        }
                        PendingRpc::SessionLoad { acp_id } => {
                            replaying = false;
                            suppress_replay = false;
                            if let Some(err) = rpc_err {
                                tracing::warn!(session = %session_id, %err, "agy acp session/load failed; starting fresh session");
                                let new_id = next_rpc_id;
                                next_rpc_id += 1;
                                inflight.insert(new_id, PendingRpc::SessionNew);
                                write_json_line(
                                    &mut stdin,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "id": new_id,
                                        "method": "session/new",
                                        "params": build_session_new_params(&cfg.cwd, &cfg.facade),
                                    }),
                                )
                                .await?;
                            } else {
                                let resolved_id = result
                                    .get("sessionId")
                                    .and_then(Value::as_str)
                                    .unwrap_or(&acp_id)
                                    .to_owned();
                                acp_session_id = Some(resolved_id.clone());
                                record_acp_session(&session_id, &resolved_id);
                                apply_session_bootstrap(
                                    &store,
                                    &conv,
                                    &session_id,
                                    result,
                                    &cfg,
                                    &yolo_live,
                                    &mut stdin,
                                    &mut next_rpc_id,
                                    &mut inflight,
                                    &mut cur_mode,
                                    &mut acc,
                                    &mut translate_state,
                                    &resolved_id,
                                )
                                .await?;
                                ready_for_prompts = true;
                            }
                        }
                        PendingRpc::SessionNew => {
                            if let Some(err) = rpc_err {
                                anyhow::bail!("agy acp session/new failed: {err}");
                            }
                            let sid = result
                                .get("sessionId")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .ok_or_else(|| anyhow::anyhow!("agy acp session/new returned no sessionId"))?
                                .to_owned();
                            acp_session_id = Some(sid.clone());
                            record_acp_session(&session_id, &sid);
                            apply_session_bootstrap(
                                &store,
                                &conv,
                                &session_id,
                                result,
                                &cfg,
                                &yolo_live,
                                &mut stdin,
                                &mut next_rpc_id,
                                &mut inflight,
                                &mut cur_mode,
                                &mut acc,
                                &mut translate_state,
                                &sid,
                            )
                            .await?;
                            ready_for_prompts = true;
                        }
                        PendingRpc::InitialConfig => {
                            if let Some(err) = rpc_err {
                                tracing::warn!(session = %session_id, %err, "initial antigravity config option failed");
                            } else if let Some(opts) = result.get("configOptions").and_then(Value::as_array) {
                                let mut updates = Vec::new();
                                let mut meta = UpdateMeta::default();
                                observe_config_options(opts, &mut translate_state, &mut updates, &mut meta);
                                if let Some(mode_id) = meta.observed_mode {
                                    let canonical = observed_mode_from_acp(
                                        &mode_id,
                                        yolo_live.load(Ordering::Relaxed),
                                    );
                                    store.set_observed_permission_mode(&session_id, canonical);
                                }
                                if !updates.is_empty() {
                                    apply_updates(&store, &conv, &session_id, updates, &mut cur_mode, &mut acc);
                                }
                            }
                        }
                        PendingRpc::Prompt {
                            is_compact,
                            pre_tokens,
                        } => {
                            in_flight_prompt = false;
                            translate_state.close_thought_block();
                            // Cancel any stale parked permissions left over at turn end.
                            while let Some(perm) = parked_perms.pop_front() {
                                let _ = write_json_line(
                                    &mut stdin,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "id": perm.rpc_id,
                                        "result": { "outcome": { "outcome": "cancelled" } },
                                    }),
                                )
                                .await;
                            }
                            let mut updates = Vec::new();
                            if let Some(err) = rpc_err {
                                if is_compact {
                                    updates.push(AgentUpdate::Compacting(false));
                                }
                                updates.push(AgentUpdate::Error(err));
                            } else {
                                if let Some(usage) =
                                    extract_prompt_usage(result, translate_state.current_model.as_deref())
                                {
                                    updates.push(usage);
                                }
                                if is_compact {
                                    updates.push(AgentUpdate::Compacted {
                                        trigger: Some("manual".into()),
                                        pre_tokens,
                                        post_tokens: translate_state.last_context_tokens,
                                        legacy: false,
                                    });
                                }
                            }
                            if let Some(sid) = acp_session_id.as_deref() {
                                sync_brain_artifacts(
                                    &store,
                                    &session_id,
                                    sid,
                                    &translate_state.subagent_tool_ids,
                                    &mut last_artifact_stamps,
                                    true,
                                )
                                .await;
                            }
                            if pending_prompts.is_empty() {
                                updates.push(AgentUpdate::Idle);
                            }
                            if !updates.is_empty() {
                                apply_updates(&store, &conv, &session_id, updates, &mut cur_mode, &mut acc);
                            }
                        }
                        PendingRpc::ModelSwitch {
                            remaining_effort,
                            fallback_compound,
                            reply,
                        } => {
                            if let Some(err) = rpc_err {
                                // If caller passed a compound id (e.g. `gemini-4-neon-xhigh`), retry split.
                                if let (Some((base, eff)), Some(sid)) =
                                    (fallback_compound, acp_session_id.as_deref())
                                {
                                    let retry_id = next_rpc_id;
                                    next_rpc_id += 1;
                                    inflight.insert(
                                        retry_id,
                                        PendingRpc::ModelSwitch {
                                            remaining_effort: remaining_effort.or(Some(eff)),
                                            fallback_compound: None,
                                            reply,
                                        },
                                    );
                                    write_json_line(
                                        &mut stdin,
                                        &json!({
                                            "jsonrpc": "2.0",
                                            "id": retry_id,
                                            "method": "session/set_config_option",
                                            "params": {
                                                "sessionId": sid,
                                                "configId": "model",
                                                "value": base,
                                            }
                                        }),
                                    )
                                    .await?;
                                } else {
                                    let _ = reply.send(Err(err));
                                }
                            } else {
                                if let Some(opts) = result.get("configOptions").and_then(Value::as_array) {
                                    let mut updates = Vec::new();
                                    let mut meta = UpdateMeta::default();
                                    observe_config_options(opts, &mut translate_state, &mut updates, &mut meta);
                                    if !updates.is_empty() {
                                        apply_updates(&store, &conv, &session_id, updates, &mut cur_mode, &mut acc);
                                    }
                                }
                                if let (Some(effort), Some(sid)) = (remaining_effort, acp_session_id.as_deref()) {
                                    let eff_id = next_rpc_id;
                                    next_rpc_id += 1;
                                    inflight.insert(eff_id, PendingRpc::EffortSwitch { reply });
                                    write_json_line(
                                        &mut stdin,
                                        &json!({
                                            "jsonrpc": "2.0",
                                            "id": eff_id,
                                            "method": "session/set_config_option",
                                            "params": {
                                                "sessionId": sid,
                                                "configId": "thought_level",
                                                "value": effort,
                                            }
                                        }),
                                    )
                                    .await?;
                                } else {
                                    let _ = reply.send(Ok(()));
                                }
                            }
                        }
                        PendingRpc::EffortSwitch { reply } => {
                            if let Some(err) = rpc_err {
                                let _ = reply.send(Err(err));
                            } else {
                                if let Some(opts) = result.get("configOptions").and_then(Value::as_array) {
                                    let mut updates = Vec::new();
                                    let mut meta = UpdateMeta::default();
                                    observe_config_options(opts, &mut translate_state, &mut updates, &mut meta);
                                    if !updates.is_empty() {
                                        apply_updates(&store, &conv, &session_id, updates, &mut cur_mode, &mut acc);
                                    }
                                }
                                let _ = reply.send(Ok(()));
                            }
                        }
                        PendingRpc::PermissionMode {
                            canonical,
                            target_yolo,
                            reply,
                        } => {
                            if let Some(err) = rpc_err {
                                let _ = reply.send(Err(err));
                            } else {
                                yolo_live.store(target_yolo, Ordering::Relaxed);
                                store.set_observed_permission_mode(&session_id, &canonical);
                                // Auto-approve any already-parked requests when entering bypassPermissions.
                                if target_yolo {
                                    let had_parked = !parked_perms.is_empty();
                                    while let Some(perm) = parked_perms.pop_front() {
                                        let outcome = permission_outcome(&perm, &ManagedDecision::from(true));
                                        let _ = write_json_line(
                                            &mut stdin,
                                            &json!({
                                                "jsonrpc": "2.0",
                                                "id": perm.rpc_id,
                                                "result": { "outcome": outcome },
                                            }),
                                        )
                                        .await;
                                    }
                                    if had_parked {
                                        let next_mode = if in_flight_prompt {
                                            SessionMode::Responding
                                        } else {
                                            SessionMode::Input
                                        };
                                        set_mode(
                                            &store,
                                            &session_id,
                                            next_mode,
                                            PendingWrite::Resolve(PendingOwner::Primary),
                                            &mut cur_mode,
                                        );
                                    }
                                }
                                let _ = reply.send(Ok(canonical));
                            }
                        }
                    }
                }
            }

            prompt = prompt_rx.recv() => {
                let Some(prompt) = prompt else {
                    terminated_by_us = true;
                    if let Some(sid) = acp_session_id.as_deref() {
                        let close_id = next_rpc_id;
                        let _ = write_json_line(
                            &mut stdin,
                            &json!({
                                "jsonrpc": "2.0",
                                "id": close_id,
                                "method": "session/close",
                                "params": { "sessionId": sid }
                            }),
                        )
                        .await;
                    }
                    let _ = child.kill().await;
                    break;
                };
                if in_flight_prompt && prompt.delivery == Delivery::Now {
                    if let Some(sid) = acp_session_id.as_deref() {
                        let _ = write_json_line(
                            &mut stdin,
                            &json!({
                                "jsonrpc": "2.0",
                                "method": "session/cancel",
                                "params": { "sessionId": sid }
                            }),
                        )
                        .await;
                    }
                    pending_prompts.push_front(QueuedPrompt {
                        prompt,
                        is_compact: false,
                    });
                } else {
                    pending_prompts.push_back(QueuedPrompt {
                        prompt,
                        is_compact: false,
                    });
                }
            }

            Some(decision) = dec_rx.recv() => {
                let Some(head) = parked_perms.front() else {
                    continue;
                };
                if let Some(req_id) = decision.request_id.as_deref() {
                    if req_id != head.pending_id {
                        continue;
                    }
                }
                if let Some(always_id) = decision.always_option.as_deref() {
                    if !head.always_map.iter().any(|(id, _)| id == always_id) {
                        continue;
                    }
                }
                let head = parked_perms.pop_front().unwrap();
                let outcome = permission_outcome(&head, &decision);
                write_json_line(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": head.rpc_id,
                        "result": { "outcome": outcome },
                    }),
                )
                .await?;
                if !decision.approve {
                    if let Some(note) = decision
                        .note
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        let tool_name = head
                            .tool
                            .as_deref()
                            .or(head.summary.as_deref())
                            .unwrap_or("tool");
                        pending_prompts.push_back(QueuedPrompt {
                            prompt: ManagedPrompt {
                                text: format!("Denied {tool_name}: {note}"),
                                delivery: Delivery::Steer,
                                sender: crate::session::store::MessageSender::Human,
                                uuid: None,
                                images: Vec::new(),
                            },
                            is_compact: false,
                        });
                    }
                }
                if let Some(next) = parked_perms.front() {
                    apply_updates(
                        &store,
                        &conv,
                        &session_id,
                        vec![AgentUpdate::PermissionPending {
                            id: Some(next.pending_id.clone()),
                            tool: next.tool.clone(),
                            summary: next.summary.clone(),
                            raw: next.raw.clone(),
                        }],
                        &mut cur_mode,
                        &mut acc,
                    );
                } else {
                    let next_mode = if in_flight_prompt {
                        SessionMode::Responding
                    } else {
                        SessionMode::Input
                    };
                    set_mode(
                        &store,
                        &session_id,
                        next_mode,
                        PendingWrite::Resolve(PendingOwner::Primary),
                        &mut cur_mode,
                    );
                }
            }

            Some(ConfirmedModelSwitch { switch, reply }) = model_rx.recv() => {
                let Some(sid) = acp_session_id.as_deref() else {
                    let _ = reply.send(Err("antigravity session is still starting".into()));
                    continue;
                };
                match switch {
                    ModelSwitch { model: Some(model), effort } => {
                        let fallback = split_compound_model(&model)
                            .map(|(b, e)| (b.to_owned(), e.to_owned()));
                        let req_id = next_rpc_id;
                        next_rpc_id += 1;
                        inflight.insert(
                            req_id,
                            PendingRpc::ModelSwitch {
                                remaining_effort: effort,
                                fallback_compound: fallback,
                                reply,
                            },
                        );
                        write_json_line(
                            &mut stdin,
                            &json!({
                                "jsonrpc": "2.0",
                                "id": req_id,
                                "method": "session/set_config_option",
                                "params": {
                                    "sessionId": sid,
                                    "configId": "model",
                                    "value": model,
                                }
                            }),
                        )
                        .await?;
                    }
                    ModelSwitch { model: None, effort: Some(effort) } => {
                        let req_id = next_rpc_id;
                        next_rpc_id += 1;
                        inflight.insert(req_id, PendingRpc::EffortSwitch { reply });
                        write_json_line(
                            &mut stdin,
                            &json!({
                                "jsonrpc": "2.0",
                                "id": req_id,
                                "method": "session/set_config_option",
                                "params": {
                                    "sessionId": sid,
                                    "configId": "thought_level",
                                    "value": effort,
                                }
                            }),
                        )
                        .await?;
                    }
                    ModelSwitch { model: None, effort: None } => {
                        let _ = reply.send(Ok(()));
                    }
                }
            }

            Some(ManagedPermissionSwitch { mode, reply }) = perm_rx.recv() => {
                let Some((acp_mode, target_yolo, canonical)) = map_permission_mode(&mode) else {
                    let _ = reply.send(Err(format!("unsupported antigravity permission mode '{mode}'")));
                    continue;
                };
                if cfg.yolo && !target_yolo && acp_mode != "plan" {
                    let _ = reply.send(Err(
                        "the agent was started with approvals bypassed — restart to re-enable them".into()
                    ));
                    continue;
                }
                let Some(sid) = acp_session_id.as_deref() else {
                    let _ = reply.send(Err("antigravity session is still starting".into()));
                    continue;
                };
                let req_id = next_rpc_id;
                next_rpc_id += 1;
                inflight.insert(
                    req_id,
                    PendingRpc::PermissionMode {
                        canonical: canonical.to_owned(),
                        target_yolo,
                        reply,
                    },
                );
                write_json_line(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "method": "session/set_mode",
                        "params": {
                            "sessionId": sid,
                            "modeId": acp_mode,
                        }
                    }),
                )
                .await?;
            }

            Some(ManagedSessionControl { action, reply }) = ctl_rx.recv() => {
                match action {
                    SessionControlAction::Compact { instructions } => {
                        let args = instructions
                            .map(|i| i.trim().to_string())
                            .filter(|i| !i.is_empty());
                        let text = match &args {
                            Some(args) => format!("/compact {args}"),
                            None => "/compact".to_string(),
                        };
                        conv.push(
                            &session_id,
                            vec![ConversationItem::SlashCommand {
                                name: "compact".into(),
                                args,
                                timestamp: None,
                            }],
                        );
                        pending_prompts.push_back(QueuedPrompt {
                            prompt: ManagedPrompt {
                                text,
                                delivery: Delivery::Steer,
                                sender: crate::session::store::MessageSender::Human,
                                uuid: None,
                                images: Vec::new(),
                            },
                            is_compact: true,
                        });
                        let _ = reply.send(Ok(json!({ "queued": true })));
                    }
                    SessionControlAction::RewindFiles { .. } => {
                        let _ = reply.send(Err("file rewind is not supported by Antigravity".into()));
                    }
                    SessionControlAction::Revert { .. } => {
                        let _ = reply.send(Err("turn revert is not supported by Antigravity".into()));
                    }
                    SessionControlAction::ForkPoint { .. } => {
                        let _ = reply.send(Err("session fork is not supported by Antigravity".into()));
                    }
                }
            }

            Some(()) = int_rx.recv() => {
                if let Some(sid) = acp_session_id.as_deref() {
                    let _ = write_json_line(
                        &mut stdin,
                        &json!({
                            "jsonrpc": "2.0",
                            "method": "session/cancel",
                            "params": { "sessionId": sid }
                        }),
                    )
                    .await;
                }
            }

            _ = artifact_tick.tick(), if acp_session_id.is_some() => {
                if let Some(sid) = acp_session_id.as_deref() {
                    sync_brain_artifacts(
                        &store,
                        &session_id,
                        sid,
                        &translate_state.subagent_tool_ids,
                        &mut last_artifact_stamps,
                        false,
                    )
                    .await;
                }
            }
        }
    }

    // Reject any outstanding callers cleanly before exiting.
    for (_, pending) in inflight.drain() {
        match pending {
            PendingRpc::ModelSwitch { reply, .. } | PendingRpc::EffortSwitch { reply } => {
                let _ = reply.send(Err("antigravity session ended".into()));
            }
            PendingRpc::PermissionMode { reply, .. } => {
                let _ = reply.send(Err("antigravity session ended".into()));
            }
            _ => {}
        }
    }

    let status = child.wait().await.ok();
    if status.is_some_and(|s| !s.success()) && !terminated_by_us {
        let tail = stderr_tail
            .lock()
            .ok()
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        if !tail.is_empty() && !ready_for_prompts {
            anyhow::bail!("agy acp exited with {status:?}: {tail}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_session_bootstrap(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    result: &Value,
    cfg: &SpawnConfig,
    yolo_live: &Arc<AtomicBool>,
    stdin: &mut tokio::process::ChildStdin,
    next_rpc_id: &mut u64,
    inflight: &mut HashMap<u64, PendingRpc>,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    translate_state: &mut TranslateState,
    acp_sid: &str,
) -> anyhow::Result<()> {
    // Observe the server's initial model / effort / mode from `session/new` or `session/load`.
    if let Some(opts) = result.get("configOptions").and_then(Value::as_array) {
        let mut updates = Vec::new();
        let mut meta = UpdateMeta::default();
        observe_config_options(opts, translate_state, &mut updates, &mut meta);
        if let Some(mode_id) = meta.observed_mode {
            let canonical = observed_mode_from_acp(&mode_id, yolo_live.load(Ordering::Relaxed));
            store.set_observed_permission_mode(session_id, canonical);
        }
        if !updates.is_empty() {
            apply_updates(store, conv, session_id, updates, cur_mode, acc);
        }
    }

    // Apply spawn-time model & effort overrides if requested.
    let mut requested_model = cfg.model.clone().filter(|m| !m.trim().is_empty());
    let mut requested_effort = cfg.effort.clone().filter(|e| !e.trim().is_empty());
    if let Some(model) = requested_model.clone() {
        // If caller supplied a compound id (e.g. `gemini-4-neon-xhigh`), split it
        // only when it matches one of the base options in `configOptions`.
        let base_options: HashSet<&str> = result
            .get("configOptions")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|o| o.get("id").and_then(Value::as_str) == Some("model"))
            })
            .and_then(|o| o.get("options").and_then(Value::as_array))
            .map(|opts| {
                opts.iter()
                    .filter_map(|o| o.get("value").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        if !base_options.contains(model.as_str()) {
            if let Some((base, eff)) = split_compound_model(&model) {
                if base_options.is_empty() || base_options.contains(base) {
                    requested_model = Some(base.to_owned());
                    if requested_effort.is_none() {
                        requested_effort = Some(eff.to_owned());
                    }
                }
            }
        }
    }

    if let Some(model) = requested_model {
        if translate_state.current_model.as_deref() != Some(model.as_str()) {
            let id = *next_rpc_id;
            *next_rpc_id += 1;
            inflight.insert(id, PendingRpc::InitialConfig);
            write_json_line(
                stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "session/set_config_option",
                    "params": {
                        "sessionId": acp_sid,
                        "configId": "model",
                        "value": model,
                    }
                }),
            )
            .await?;
        }
    }
    if let Some(effort) = requested_effort {
        let id = *next_rpc_id;
        *next_rpc_id += 1;
        inflight.insert(id, PendingRpc::InitialConfig);
        write_json_line(
            stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/set_config_option",
                "params": {
                    "sessionId": acp_sid,
                    "configId": "thought_level",
                    "value": effort,
                }
            }),
        )
        .await?;
    }
    if let Some(mode_str) = cfg.permission_mode.as_deref() {
        if let Some((acp_mode, target_yolo, canonical)) = map_permission_mode(mode_str) {
            yolo_live.store(target_yolo || cfg.yolo, Ordering::Relaxed);
            store.set_observed_permission_mode(session_id, canonical);
            if acp_mode != "default" {
                let id = *next_rpc_id;
                *next_rpc_id += 1;
                inflight.insert(id, PendingRpc::InitialConfig);
                write_json_line(
                    stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": "session/set_mode",
                        "params": {
                            "sessionId": acp_sid,
                            "modeId": acp_mode,
                        }
                    }),
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn sync_brain_artifacts(
    store: &SessionStore,
    session_id: &str,
    acp_sid: &str,
    subagent_tool_ids: &[String],
    last_stamps: &mut Vec<(PathBuf, Option<super::antigravity_artifacts::FileStamp>)>,
    force: bool,
) {
    let Some(brain) = super::antigravity_artifacts::brain_root() else {
        return;
    };
    if !force && !last_stamps.is_empty() {
        let unchanged = last_stamps
            .iter()
            .all(|(path, prev)| super::antigravity_artifacts::file_stamp(path) == *prev);
        if unchanged {
            return;
        }
    }
    let acp_sid = acp_sid.to_owned();
    let anchors = subagent_tool_ids.to_vec();
    let Ok(snap) = tokio::task::spawn_blocking(move || {
        super::antigravity_artifacts::scan_session_in(&brain, &acp_sid, &anchors)
    })
    .await
    else {
        return;
    };
    *last_stamps = snap.stamps.clone();
    store.update_state(session_id, |s| {
        super::antigravity_artifacts::apply_snapshot(s, snap)
    });
}

fn format_rpc_error(err: &Value) -> String {
    let msg = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("ACP error");
    if let Some(data) = err
        .get("data")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        format!("{msg}: {data}")
    } else {
        msg.to_owned()
    }
}

// Suppress unused import warnings when Pending / PendingOwner are referenced via PendingWrite.
const _: fn() = || {
    let _ = std::any::type_name::<Pending>();
    let _ = std::any::type_name::<PendingOwner>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_acp_models_extracts_base_families_and_per_model_efforts() {
        let result = json!({
            "sessionId": "s1",
            "models": {
                "currentModelId": "gemini-4-neon-high",
                "availableModels": [
                    {"modelId": "gemini-4-neon-high", "name": "Gemini 4 Neon (High)"},
                    {"modelId": "gemini-4-neon-xhigh", "name": "Gemini 4 Neon (XHigh)"},
                    {"modelId": "gemini-4-neon-medium", "name": "Gemini 4 Neon (Medium)"},
                    {"modelId": "gemini-4-neon-low", "name": "Gemini 4 Neon (Low)"},
                    {"modelId": "gemini-pro-orchestrator-neon", "name": "Orchestrator (Neon)"},
                    {"modelId": "gemini-3.8-flash-high", "name": "Gemini 3.8 Flash (High)"},
                    {"modelId": "gemini-3.8-flash-medium", "name": "Gemini 3.8 Flash (Medium)"},
                    {"modelId": "gemini-3.8-flash-low", "name": "Gemini 3.8 Flash (Low)"}
                ]
            },
            "configOptions": [
                {
                    "id": "model",
                    "currentValue": "gemini-4-neon",
                    "options": [
                        {"value": "gemini-4-neon", "name": "Gemini 4 Neon"},
                        {"value": "gemini-pro-orchestrator-neon", "name": "Orchestrator (Neon)"},
                        {"value": "gemini-3.8-flash", "name": "Gemini 3.8 Flash"}
                    ]
                },
                {
                    "id": "thought_level",
                    "currentValue": "high",
                    "options": [
                        {"value": "low", "name": "Low"},
                        {"value": "medium", "name": "Medium"},
                        {"value": "high", "name": "High"},
                        {"value": "xhigh", "name": "Extra"}
                    ]
                }
            ]
        });

        let models = parse_acp_models(&result);
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "gemini-4-neon");
        assert_eq!(models[0].label, "Gemini 4 Neon");
        assert!(models[0].default);
        assert_eq!(
            models[0].effort_levels,
            vec!["low", "medium", "high", "xhigh"]
        );
        assert_eq!(models[0].default_effort.as_deref(), Some("high"));

        assert_eq!(models[1].id, "gemini-pro-orchestrator-neon");
        assert!(!models[1].default);
        assert!(models[1].effort_levels.is_empty());

        assert_eq!(models[2].id, "gemini-3.8-flash");
        assert_eq!(models[2].effort_levels, vec!["low", "medium", "high"]);
        assert_eq!(models[2].default_effort, None);
    }

    #[test]
    fn translate_update_maps_text_thoughts_tools_usage_and_plans() {
        let mut st = TranslateState {
            current_model: Some("gemini-3.8-flash".into()),
            ..Default::default()
        };

        let (u1, _) = translate_update(
            &json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "thinking"}}),
            &mut st,
            false,
        );
        assert!(matches!(
            u1.as_slice(),
            [AgentUpdate::Busy, AgentUpdate::Reasoning { text, .. }] if text == "thinking"
        ));

        let (u2, _) = translate_update(
            &json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "call_1",
                "name": "run_command",
                "status": "in_progress",
                "rawInput": {"CommandLine": "echo hi"}
            }),
            &mut st,
            false,
        );
        assert!(matches!(
            u2.as_slice(),
            [AgentUpdate::Busy, AgentUpdate::ToolUse { id, name, .. }] if id == "call_1" && name == "run_command"
        ));

        // Duplicate `pending` update does not re-emit ToolUse.
        let (u3, _) = translate_update(
            &json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call_1",
                "name": "run_command",
                "status": "pending",
                "rawInput": {"CommandLine": "echo hi"}
            }),
            &mut st,
            false,
        );
        assert_eq!(u3, vec![AgentUpdate::Busy]);

        let (u4, _) = translate_update(
            &json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call_1",
                "status": "completed",
                "rawOutput": {"output": "hi\n"}
            }),
            &mut st,
            false,
        );
        assert!(matches!(
            u4.as_slice(),
            [AgentUpdate::Busy, AgentUpdate::ToolResult { tool_use_id, content, is_error: false }]
            if tool_use_id == "call_1" && content == "hi\n"
        ));

        let (u5, _) = translate_update(
            &json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Done!"}}),
            &mut st,
            false,
        );
        assert_eq!(
            u5,
            vec![
                AgentUpdate::Busy,
                AgentUpdate::AssistantText("Done!".into())
            ]
        );

        let (u6, _) = translate_update(
            &json!({"sessionUpdate": "usage_update", "used": 14352, "size": 1048576}),
            &mut st,
            false,
        );
        assert!(matches!(
            u6.as_slice(),
            [AgentUpdate::Usage {
                context_tokens: Some(14352),
                context_window: Some(1048576),
                ..
            }]
        ));

        let (u7, _) = translate_update(
            &json!({"sessionUpdate": "plan", "entries": [
                {"content": "Step 1", "status": "completed"},
                {"content": "Step 2", "status": "in_progress"}
            ]}),
            &mut st,
            false,
        );
        assert!(matches!(
            u7.as_slice(),
            [AgentUpdate::Plan(plan)] if plan.steps.len() == 2 && plan.steps[0].status == PlanStatus::Completed
        ));
    }

    #[test]
    fn replay_completed_tool_call_emits_both_use_and_result() {
        let mut st = TranslateState::default();
        let (u, _) = translate_update(
            &json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "call_replay",
                "name": "run_command",
                "status": "completed",
                "rawInput": {"CommandLine": "ls"},
                "rawOutput": {"output": "a.rs"}
            }),
            &mut st,
            true,
        );
        assert!(matches!(
            u.as_slice(),
            [
                AgentUpdate::ToolUse { id, .. },
                AgentUpdate::ToolResult { tool_use_id, content, is_error: false }
            ] if id == "call_replay" && tool_use_id == "call_replay" && content == "a.rs"
        ));
    }

    #[test]
    fn permission_request_maps_allow_always_and_outcomes() {
        let perm = parse_permission_request(
            json!("1"),
            &json!({
                "sessionId": "s1",
                "toolCall": {
                    "toolCallId": "c1",
                    "name": "run_command",
                    "title": "echo hi",
                    "rawInput": {"CommandLine": "echo hi"}
                },
                "options": [
                    {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "always", "name": "Always allow echo", "kind": "allow_always"},
                    {"optionId": "deny", "name": "Deny", "kind": "reject_once"}
                ]
            }),
        );
        assert_eq!(perm.pending_id, "1");
        assert_eq!(perm.tool.as_deref(), Some("run_command"));
        assert_eq!(perm.raw["always_options"][0]["id"], "1:0");
        assert_eq!(perm.raw["always_options"][0]["label"], "Always allow echo");

        assert_eq!(
            permission_outcome(&perm, &ManagedDecision::from(true)),
            json!({"outcome": "selected", "optionId": "allow"})
        );
        assert_eq!(
            permission_outcome(
                &perm,
                &ManagedDecision {
                    approve: true,
                    always_option: Some("1:0".into()),
                    ..Default::default()
                }
            ),
            json!({"outcome": "selected", "optionId": "always"})
        );
        assert_eq!(
            permission_outcome(&perm, &ManagedDecision::from(false)),
            json!({"outcome": "selected", "optionId": "deny"})
        );
    }

    #[test]
    fn prompt_usage_folds_thought_tokens_into_output() {
        let upd = extract_prompt_usage(
            &json!({
                "stopReason": "end_turn",
                "usage": {
                    "inputTokens": 15893,
                    "outputTokens": 207,
                    "cachedReadTokens": 11500,
                    "thoughtTokens": 150
                }
            }),
            Some("gemini-3.8-flash"),
        )
        .unwrap();
        assert_eq!(
            upd,
            AgentUpdate::Usage {
                model: Some("gemini-3.8-flash".into()),
                input_tokens: Some(15893),
                output_tokens: Some(357),
                cached_input_tokens: Some(11500),
                cost_usd: None,
                context_tokens: None,
                context_window: None,
            }
        );
    }

    #[test]
    fn permission_mode_mappings_and_compound_split() {
        assert_eq!(
            map_permission_mode("default"),
            Some(("default", false, "default"))
        );
        assert_eq!(
            map_permission_mode("acceptEdits"),
            Some(("accept-edits", false, "acceptEdits"))
        );
        assert_eq!(
            map_permission_mode("accept-edits"),
            Some(("accept-edits", false, "acceptEdits"))
        );
        assert_eq!(map_permission_mode("auto"), Some(("auto", false, "auto")));
        assert_eq!(map_permission_mode("plan"), Some(("plan", false, "plan")));
        assert_eq!(
            map_permission_mode("bypassPermissions"),
            Some(("auto", true, "bypassPermissions"))
        );
        assert_eq!(observed_mode_from_acp("accept-edits", false), "acceptEdits");
        assert_eq!(observed_mode_from_acp("auto", true), "bypassPermissions");
        assert_eq!(observed_mode_from_acp("plan", true), "plan");
        assert_eq!(
            split_compound_model("gemini-4-neon-xhigh"),
            Some(("gemini-4-neon", "xhigh"))
        );
        assert_eq!(split_compound_model("gemini-pro-orchestrator-neon"), None);
    }

    #[test]
    fn translate_update_extracts_slash_commands_title_and_tracks_context_tokens() {
        let mut st = TranslateState::default();
        let (u1, m1) = translate_update(
            &json!({
                "sessionUpdate": "available_commands_update",
                "availableCommands": [
                    {"name": "compact", "description": "Compact context"},
                    {"name": "goal", "description": "Run until done"}
                ]
            }),
            &mut st,
            false,
        );
        assert!(u1.is_empty());
        let cmds = m1.slash_commands.expect("slash commands");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].insert, "/compact");

        let (u2, m2) = translate_update(
            &json!({"sessionUpdate": "session_info_update", "title": "Fix Parity Gaps"}),
            &mut st,
            false,
        );
        assert!(u2.is_empty());
        assert_eq!(m2.title.as_deref(), Some("Fix Parity Gaps"));

        let (_, _) = translate_update(
            &json!({"sessionUpdate": "usage_update", "used": 24500, "size": 1048576}),
            &mut st,
            false,
        );
        assert_eq!(st.last_context_tokens, Some(24500));
    }

    #[test]
    fn prepare_skill_add_dir_writes_valid_agents_skills_json() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(prepare_skill_add_dir_in(tmp.path(), &[]).is_none());
        let dir = prepare_skill_add_dir_in(
            tmp.path(),
            &[
                r"C:\Users\test\.workspacer\agent-skills\v1\workspacer\skills".into(),
                r"C:\Users\test\.workspacer\agent-skills\v1\workspacer\skills".into(),
            ],
        )
        .unwrap();
        let raw = std::fs::read_to_string(dir.join(".agents").join("skills.json")).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v,
            json!({
                "entries": [
                    { "path": "C:/Users/test/.workspacer/agent-skills/v1/workspacer/skills" }
                ]
            })
        );
    }
}

//! Antigravity CLI (`agy`) background workers (subagents) and background shell
//! tasks, observed from on-disk brain artifacts.
//!
//! `agy acp` does not stream subagent progress or background-task lifecycle
//! notifications over JSON-RPC, but writes structured logs under
//! `~/.gemini/antigravity-cli/brain/`:
//!
//! - `<acp_session_id>/.system_generated/logs/transcript.jsonl`:
//!   - `PLANNER_RESPONSE.tool_calls` lists `invoke_subagent` (`Role`, `TypeName`,
//!     `Prompt`, `Model`) and `run_command` (`CommandLine`, `toolSummary`).
//!   - `GENERIC` results emit:
//!     - `"Created the following subagent:\n{ \"conversationId\": \"<uuid>\", ... }"`
//!     - `"Tool is running as a background task with task id: <acp_sid>/task-<N>\n..."`
//!     - `"Task \"<acp_sid>/task-<N>\" cancelled."`
//!   - `SYSTEM_MESSAGE` steps emit completion wakes with
//!     `sender=<child_uuid>` (for subagents) or `sender=<acp_sid>/task-<N>`
//!     (for background shell tasks).
//! - `<child_uuid>/.system_generated/logs/transcript.jsonl`:
//!   - Full child conversation (`thinking`, `tool_calls`, `GENERIC` tool
//!     outputs, `model`, `input_tokens`, `cache_read_tokens`, `output_tokens`),
//!     powering both live telemetry on the parent card and drill-in child
//!     conversation replay via `GET /sessions/:id/subagents/:agent_id/conversation`.
//! - `<acp_session_id>/.system_generated/tasks/task-<N>.log`:
//!   - Live shell output served by `GET /sessions/:id/tasks/:task_id/output`.
//!
//! All paths are strictly derived from validated IDs inside `brain/` — never
//! from arbitrary URIs parsed out of transcript prose.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Result};
use directories::BaseDirs;
use serde_json::{json, Value};

use crate::session::background_tasks::{self as bg, BackgroundTask, TaskUsage};
use crate::session::conversation::ConversationItem;
use crate::session::state::{SessionState, SubagentInfo, SubagentStatus};

const MAX_CHILDREN: usize = 64;
const MAX_CHILD_BYTES: u64 = 128 * 1024 * 1024;
const MAX_REPLAY_ITEMS: usize = 2000;
const MAX_TEXT: usize = 512;

/// Strict identifier check for conversation UUIDs and `task-<N>` ids.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `~/.gemini/antigravity-cli/brain`.
pub fn brain_root() -> Option<PathBuf> {
    Some(
        BaseDirs::new()?
            .home_dir()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain"),
    )
}

pub fn transcript_path_in(brain: &Path, conv_id: &str) -> Option<PathBuf> {
    if !valid_id(conv_id) {
        return None;
    }
    Some(
        brain
            .join(conv_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl"),
    )
}

pub fn task_log_path_in(brain: &Path, acp_session_id: &str, task_id: &str) -> Option<PathBuf> {
    if !valid_id(acp_session_id) || !bg::valid_task_id(task_id) {
        return None;
    }
    Some(
        brain
            .join(acp_session_id)
            .join(".system_generated")
            .join("tasks")
            .join(format!("{task_id}.log")),
    )
}

/// Cheap file metadata fingerprint for avoiding redundant JSONL re-reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

pub fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

/// Snapshot of subagents and background tasks parsed from an Antigravity parent
/// transcript + its children's transcripts.
#[derive(Debug, Clone, Default)]
pub struct ArtifactSnapshot {
    pub subagents: Vec<SubagentInfo>,
    pub tasks: Vec<BackgroundTask>,
    /// File stamps of the parent + child transcripts observed during this scan.
    pub stamps: Vec<(PathBuf, Option<FileStamp>)>,
}

/// Parsed replay of a child subagent's conversation.
#[derive(Debug, Clone)]
pub struct ChildReplay {
    pub seq: usize,
    pub first_seq: usize,
    pub items: Vec<ConversationItem>,
}

fn clip(s: &str, limit: usize) -> String {
    let line = s.trim();
    if line.chars().count() <= limit {
        return line.to_owned();
    }
    line.chars().take(limit).collect()
}

fn parse_rfc3339_ms(ts: &str) -> Option<i64> {
    let parsed =
        time::OffsetDateTime::parse(ts.trim(), &time::format_description::well_known::Rfc3339)
            .ok()?;
    Some((parsed.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// Antigravity's `transcript.jsonl` encodes each `tool_calls[].args` value as a
/// JSON-encoded string (e.g. `"\"Quick Check\""`, `"10"`, `"true"`). Decode one
/// layer into a real JSON value so downstream tool previews work identically to
/// ACP wire events.
fn decode_transcript_arg(val: &Value) -> Value {
    match val {
        Value::String(s) => {
            serde_json::from_str::<Value>(s).unwrap_or_else(|_| Value::String(s.clone()))
        }
        other => other.clone(),
    }
}

fn decode_transcript_args(args: Option<&Value>) -> Value {
    let Some(Value::Object(map)) = args else {
        return json!({});
    };
    let mut out = serde_json::Map::with_capacity(map.len());
    for (k, v) in map {
        out.insert(k.clone(), decode_transcript_arg(v));
    }
    Value::Object(out)
}

fn arg_str(args: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn summarize_child_tool(name: &str, args: &Value) -> Option<String> {
    let candidate = match name {
        "run_command" => arg_str(args, &["toolSummary", "CommandLine", "command"]),
        "read_file" | "edit_file" | "write_file" => arg_str(args, &["Path", "path", "file_path"]),
        "search_web" => arg_str(args, &["query", "Query"]),
        "invoke_subagent" => arg_str(args, &["Role", "Prompt", "TypeName"]),
        "send_message" => arg_str(args, &["Message", "Recipient"]),
        _ => arg_str(
            args,
            &[
                "toolSummary",
                "Path",
                "path",
                "CommandLine",
                "query",
                "pattern",
                "Role",
                "Action",
            ],
        ),
    };
    candidate.map(|s| clip(s.lines().next().unwrap_or(&s), MAX_TEXT))
}

#[derive(Debug, Default)]
struct PendingSubagentMeta {
    role: Option<String>,
    agent_type: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Default)]
struct PendingCommandMeta {
    command: Option<String>,
    summary: Option<String>,
}

/// Extract `"conversationId": "<uuid>"` from a `Created the following subagent:`
/// `GENERIC` tool result.
fn extract_subagent_conversation_id(content: &str) -> Option<String> {
    let marker = "Created the following subagent:";
    let idx = content.find(marker)?;
    let rest = &content[idx + marker.len()..];
    let start = rest.find('{')?;
    // Scan for the matching closing brace of the JSON object.
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escape = false;
    let mut end = None;
    for (offset, ch) in rest[start..].char_indices() {
        if in_str {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    end = Some(start + offset + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let json_slice = &rest[start..end?];
    let parsed: Value = serde_json::from_str(json_slice).ok()?;
    let id = parsed
        .get("conversationId")
        .and_then(Value::as_str)
        .map(str::trim)?;
    valid_id(id).then(|| id.to_owned())
}

/// Extract `(full_task_id, short_task_id, description)` from a backgrounded
/// `run_command` `GENERIC` step:
/// `Tool is running as a background task with task id: <uuid>/task-<N>`.
fn extract_background_task_start(content: &str) -> Option<(String, String, Option<String>)> {
    let marker = "Tool is running as a background task with task id:";
    let idx = content.find(marker)?;
    let rest = &content[idx + marker.len()..];
    let full_id = rest.lines().next()?.trim();
    if full_id.is_empty() {
        return None;
    }
    let short_id = full_id.rsplit('/').next().unwrap_or(full_id).trim();
    if !bg::valid_task_id(short_id) {
        return None;
    }
    let description = content
        .find("Task Description:")
        .map(|d_idx| {
            let after = &content[d_idx + "Task Description:".len()..];
            let first_line = after.lines().next().unwrap_or("").trim();
            clip(first_line, 240)
        })
        .filter(|s| !s.is_empty());
    Some((full_id.to_owned(), short_id.to_owned(), description))
}

/// Extract `sender=<value>` from an Antigravity `<SYSTEM_MESSAGE>` `[Message]`
/// header line.
fn extract_system_message_sender(content: &str) -> Option<String> {
    let idx = content.find("sender=")?;
    let after = &content[idx + "sender=".len()..];
    let sender = after.split_whitespace().next()?.trim();
    (!sender.is_empty()).then(|| sender.to_owned())
}

/// Extract cancelled task id from `Task "<uuid>/task-<N>" cancelled.`
fn extract_cancelled_task_short_id(content: &str) -> Option<String> {
    let idx = content.find("Task \"")?;
    let after = &content[idx + "Task \"".len()..];
    let end = after.find("\" cancelled")?;
    let full_id = after[..end].trim();
    let short = full_id.rsplit('/').next().unwrap_or(full_id).trim();
    bg::valid_task_id(short).then(|| short.to_owned())
}

/// Extract exit code and a concise summary from a background task's completion
/// `<SYSTEM_MESSAGE>`.
fn parse_task_completion(content: &str) -> (&'static str, Option<String>) {
    if content.contains("cancelled") && !content.contains("Command exited with code") {
        return ("killed", Some("Cancelled".into()));
    }
    if let Some(idx) = content.find("Command exited with code ") {
        let rest = &content[idx + "Command exited with code ".len()..];
        let code_str: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if let Ok(code) = code_str.parse::<i32>() {
            if code == 0 {
                return ("completed", Some("Exited with code 0".into()));
            }
            return ("failed", Some(format!("Exited with code {code}")));
        }
    }
    ("completed", None)
}

#[derive(Debug, Default)]
struct ChildTelemetry {
    model: Option<String>,
    tokens: u64,
    tool_calls: u64,
    last_tool_name: Option<String>,
    last_tool_summary: Option<String>,
    has_events: bool,
}

fn scan_child_telemetry(path: &Path) -> ChildTelemetry {
    let mut out = ChildTelemetry::default();
    let Ok(file) = File::open(path) else {
        return out;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("PLANNER_RESPONSE") {
            continue;
        }
        out.has_events = true;
        if let Some(m) = v
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.model = Some(m.to_owned());
        }
        let input = v.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        let cache = v
            .get("cache_read_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = v.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        out.tokens = out
            .tokens
            .saturating_add(input)
            .saturating_add(cache)
            .saturating_add(output);

        if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let Some(name) = call.get("name").and_then(Value::as_str) else {
                    continue;
                };
                out.tool_calls = out.tool_calls.saturating_add(1);
                out.last_tool_name = Some(clip(name, 128));
                let decoded = decode_transcript_args(call.get("args"));
                out.last_tool_summary = summarize_child_tool(name, &decoded);
            }
        }
    }
    out
}

/// Scan an Antigravity session's parent transcript + its children's transcripts
/// under `brain` and produce the full `subagents` and `background_task_list`
/// snapshot. Pure filesystem read; safe to run on `spawn_blocking`.
pub fn scan_session_in(
    brain: &Path,
    acp_session_id: &str,
    acp_subagent_tool_ids: &[String],
) -> ArtifactSnapshot {
    let mut snap = ArtifactSnapshot::default();
    let Some(parent_path) = transcript_path_in(brain, acp_session_id) else {
        return snap;
    };
    snap.stamps
        .push((parent_path.clone(), file_stamp(&parent_path)));
    let Ok(file) = File::open(&parent_path) else {
        return snap;
    };

    let mut pending_subagent_meta: VecDeque<PendingSubagentMeta> = VecDeque::new();
    let mut pending_cmd_meta: VecDeque<PendingCommandMeta> = VecDeque::new();
    let mut next_tool_anchor_idx = 0usize;

    let mut subagents: Vec<SubagentInfo> = Vec::new();
    let mut tasks: Vec<BackgroundTask> = Vec::new();
    let now_ms = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let step_type = v.get("type").and_then(Value::as_str).unwrap_or("");
        let step_ts = v
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms)
            .unwrap_or(now_ms);

        match step_type {
            "PLANNER_RESPONSE" => {
                if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let name = call.get("name").and_then(Value::as_str).unwrap_or("");
                        let args = decode_transcript_args(call.get("args"));
                        match name {
                            "invoke_subagent" => {
                                pending_subagent_meta.push_back(PendingSubagentMeta {
                                    role: arg_str(&args, &["Role"]),
                                    agent_type: arg_str(&args, &["TypeName"]),
                                    prompt: arg_str(&args, &["Prompt"]),
                                    model: arg_str(&args, &["Model"]),
                                });
                            }
                            "run_command" => {
                                pending_cmd_meta.push_back(PendingCommandMeta {
                                    command: arg_str(&args, &["CommandLine", "command"]),
                                    summary: arg_str(&args, &["toolSummary"]),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            "GENERIC" => {
                let content = v.get("content").and_then(Value::as_str).unwrap_or("");
                if let Some(child_id) = extract_subagent_conversation_id(content) {
                    let meta = pending_subagent_meta.pop_front().unwrap_or_default();
                    let tool_use_id = acp_subagent_tool_ids.get(next_tool_anchor_idx).cloned();
                    next_tool_anchor_idx += 1;

                    let agent_type = meta
                        .role
                        .clone()
                        .or(meta.agent_type.clone())
                        .unwrap_or_else(|| "Subagent".into());
                    let description = meta
                        .prompt
                        .as_deref()
                        .map(|p| clip(p.lines().next().unwrap_or(p), MAX_TEXT))
                        .or_else(|| meta.role.clone());

                    if !subagents.iter().any(|s| s.id == child_id) && subagents.len() < MAX_CHILDREN
                    {
                        subagents.push(SubagentInfo {
                            id: child_id.clone(),
                            agent_type: clip(&agent_type, 256),
                            status: SubagentStatus::Running,
                            started_at: step_ts,
                            completed_at: None,
                            description: description.clone(),
                            tool_use_id: tool_use_id.clone(),
                            model: meta.model.clone(),
                            last_tool_name: None,
                            last_tool_summary: None,
                            tokens: None,
                            cost_usd: None,
                            tool_calls: None,
                        });
                    }
                    if !tasks.iter().any(|t| t.id == child_id) {
                        let mut task = BackgroundTask::new(&child_id, "local_agent", step_ts);
                        task.description = meta.role.or(description);
                        task.subagent_type = meta.agent_type.or_else(|| Some("Subagent".into()));
                        task.tool_use_id = tool_use_id;
                        task.subagent_id = Some(child_id);
                        task.has_output = false;
                        tasks.push(task);
                    }
                } else if let Some((_full_id, short_id, desc_from_out)) =
                    extract_background_task_start(content)
                {
                    let cmd_meta = pending_cmd_meta.pop_front().unwrap_or_default();
                    let log_path = task_log_path_in(brain, acp_session_id, &short_id);
                    if !tasks.iter().any(|t| t.id == short_id) {
                        let mut task = BackgroundTask::new(&short_id, "local_bash", step_ts);
                        task.description = cmd_meta
                            .summary
                            .or(desc_from_out)
                            .or_else(|| cmd_meta.command.as_deref().map(|c| clip(c, 240)));
                        task.command = cmd_meta.command;
                        task.has_output = log_path.is_some();
                        task.output_file = log_path;
                        tasks.push(task);
                    }
                } else if let Some(cancelled_id) = extract_cancelled_task_short_id(content) {
                    if let Some(task) = tasks.iter_mut().find(|t| t.id == cancelled_id) {
                        task.status = "killed".into();
                        task.ended_at.get_or_insert(step_ts);
                        task.summary = Some("Cancelled".into());
                    }
                } else if content.contains("Created At:") && !pending_cmd_meta.is_empty() {
                    // Synchronous run_command completed normally; pop its paired metadata so
                    // later backgrounded commands stay aligned.
                    if content.contains("Command exited with code")
                        || content.contains("Command output:")
                    {
                        let _ = pending_cmd_meta.pop_front();
                    }
                }
            }
            "SYSTEM_MESSAGE" => {
                let content = v.get("content").and_then(Value::as_str).unwrap_or("");
                if let Some(sender) = extract_system_message_sender(content) {
                    // Check subagent completion wake (`sender=<child_uuid>`).
                    if let Some(sub) = subagents.iter_mut().find(|s| s.id == sender) {
                        sub.status = SubagentStatus::Complete;
                        sub.completed_at.get_or_insert(step_ts);
                        if let Some(task) = tasks.iter_mut().find(|t| t.id == sender) {
                            task.status = "completed".into();
                            task.ended_at.get_or_insert(step_ts);
                        }
                    } else {
                        // Check background shell task completion (`sender=<acp_sid>/task-<N>`).
                        let short_id = sender.rsplit('/').next().unwrap_or(&sender);
                        if let Some(task) = tasks.iter_mut().find(|t| t.id == short_id) {
                            let (status, summary) = parse_task_completion(content);
                            task.status = status.into();
                            task.ended_at.get_or_insert(step_ts);
                            if summary.is_some() {
                                task.summary = summary;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Enrich every discovered subagent from its own child transcript on disk.
    for sub in &mut subagents {
        let Some(child_path) = transcript_path_in(brain, &sub.id) else {
            continue;
        };
        snap.stamps
            .push((child_path.clone(), file_stamp(&child_path)));
        let tel = scan_child_telemetry(&child_path);
        if tel.has_events {
            if let Some(model) = tel.model {
                sub.model = Some(model);
            }
            if tel.tokens > 0 {
                sub.tokens = Some(tel.tokens);
            }
            if tel.tool_calls > 0 {
                sub.tool_calls = Some(tel.tool_calls);
            }
            if let Some(tool) = tel.last_tool_name.clone() {
                sub.last_tool_name = Some(tool);
            }
            if let Some(summary) = tel.last_tool_summary.clone() {
                sub.last_tool_summary = Some(summary);
            }
        }
        // Mirror telemetry onto the matching `local_agent` BackgroundTask row.
        if let Some(task) = tasks.iter_mut().find(|t| t.id == sub.id) {
            if tel.last_tool_name.is_some() {
                task.last_tool_name = tel.last_tool_name;
            }
            if tel.has_events {
                let end = sub.completed_at.unwrap_or(now_ms);
                let duration_ms = end.saturating_sub(sub.started_at).max(0) as u64;
                task.usage = Some(TaskUsage {
                    total_tokens: tel.tokens,
                    tool_uses: tel.tool_calls,
                    duration_ms,
                });
            }
        }
    }

    bg::retain_bounded(&mut tasks, now_ms);
    snap.subagents = subagents;
    snap.tasks = tasks;
    snap
}

/// Apply an [`ArtifactSnapshot`] onto a live Antigravity `SessionState`.
/// Returns `true` if any visible field changed.
pub fn apply_snapshot(state: &mut SessionState, snap: ArtifactSnapshot) -> bool {
    let mut changed = false;
    if state.subagents != snap.subagents {
        state.subagents = snap.subagents;
        changed = true;
    }
    if state.background_task_list != snap.tasks {
        state.background_task_list = snap.tasks;
        changed = true;
    }
    let next_count = state.managed_background_count();
    if state.background_tasks != next_count {
        state.background_tasks = next_count;
        changed = true;
    }
    changed
}

/// Strip Antigravity's `<USER_REQUEST>` / `<ADDITIONAL_METADATA>` or
/// `[Message] ... content=...` wrappers for clean child-conversation display.
fn clean_child_prompt(step_type: &str, content: &str) -> String {
    if step_type == "SYSTEM_MESSAGE" {
        if let Some(idx) = content.find("content=") {
            let after = &content[idx + "content=".len()..];
            let trimmed = after
                .strip_suffix("</SYSTEM_MESSAGE>")
                .unwrap_or(after)
                .trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    if let (Some(start), Some(end)) = (
        content.find("<USER_REQUEST>"),
        content.find("</USER_REQUEST>"),
    ) {
        let inner = content[start + "<USER_REQUEST>".len()..end].trim();
        if !inner.is_empty() {
            return inner.to_owned();
        }
    }
    content.trim().to_owned()
}

/// Strip Antigravity's repeated `Created At: ...` / `Completed At: ...` lines
/// from `GENERIC` tool outputs so tool cards show the actual output cleanly.
fn clean_tool_output(content: &str) -> String {
    let lines: Vec<&str> = content
        .lines()
        .skip_while(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with("Created At:") || t.starts_with("Completed At:")
        })
        .collect();
    if lines.is_empty() {
        content.trim().to_owned()
    } else {
        lines.join("\n").trim().to_owned()
    }
}

/// Replay a child Antigravity subagent's conversation from
/// `~/.gemini/antigravity-cli/brain/<agent_id>/.system_generated/logs/transcript.jsonl`.
pub async fn replay_subagent(agent_id: &str) -> Result<ChildReplay> {
    let Some(brain) = brain_root() else {
        bail!("no home directory");
    };
    let id = agent_id.to_owned();
    tokio::task::spawn_blocking(move || replay_subagent_in(&brain, &id)).await?
}

/// [`replay_subagent`] against an explicit `brain/` directory for unit tests.
pub fn replay_subagent_in(brain: &Path, agent_id: &str) -> Result<ChildReplay> {
    let Some(path) = transcript_path_in(brain, agent_id) else {
        bail!("invalid subagent id");
    };
    let meta = std::fs::symlink_metadata(&path)?;
    if !meta.file_type().is_file() {
        bail!("subagent transcript is not a regular file");
    }
    if meta.len() > MAX_CHILD_BYTES {
        bail!("subagent transcript exceeds size bound");
    }
    let file = File::open(&path)?;
    let mut items: Vec<ConversationItem> = Vec::new();
    let mut pending_tool_ids: VecDeque<String> = VecDeque::new();

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let step_index = v.get("step_index").and_then(Value::as_u64).unwrap_or(0);
        let step_type = v.get("type").and_then(Value::as_str).unwrap_or("");
        let status = v.get("status").and_then(Value::as_str).unwrap_or("");
        let ts = v
            .get("created_at")
            .and_then(Value::as_str)
            .map(str::to_owned);

        match step_type {
            "USER_INPUT" | "SYSTEM_MESSAGE" => {
                if let Some(raw) = v.get("content").and_then(Value::as_str) {
                    let text = clean_child_prompt(step_type, raw);
                    if !text.is_empty() {
                        items.push(ConversationItem::UserMessage {
                            text,
                            uuid: None,
                            timestamp: ts,
                        });
                    }
                }
            }
            "PLANNER_RESPONSE" => {
                if let Some(thinking) = v
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    items.push(ConversationItem::Reasoning {
                        item_id: format!("agy-sub-thought-{step_index}"),
                        text: thinking.to_owned(),
                        timestamp: ts.clone(),
                    });
                }
                if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
                    for (idx, call) in calls.iter().enumerate() {
                        let Some(name) = call.get("name").and_then(Value::as_str) else {
                            continue;
                        };
                        let id = format!("agy-sub-call-{step_index}-{idx}");
                        let input = decode_transcript_args(call.get("args"));
                        pending_tool_ids.push_back(id.clone());
                        items.push(ConversationItem::ToolUse {
                            id,
                            name: name.to_owned(),
                            input,
                            timestamp: ts.clone(),
                        });
                    }
                }
                if let Some(text) = v
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    items.push(ConversationItem::AssistantText {
                        text: text.to_owned(),
                        timestamp: ts,
                    });
                }
            }
            "GENERIC" => {
                let raw = v.get("content").and_then(Value::as_str).unwrap_or("");
                let content = clean_tool_output(raw);
                if let Some(tool_use_id) = pending_tool_ids.pop_front() {
                    items.push(ConversationItem::ToolResult {
                        tool_use_id,
                        content,
                        is_error: status.eq_ignore_ascii_case("ERROR"),
                        timestamp: ts,
                    });
                }
            }
            _ => {}
        }
    }

    let total = items.len();
    let first_seq = total.saturating_sub(MAX_REPLAY_ITEMS);
    if first_seq > 0 {
        items.drain(..first_seq);
    }
    Ok(ChildReplay {
        seq: total,
        first_seq,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_parent_and_child_transcripts_for_subagents_and_background_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let brain = tmp.path();
        let parent_id = "9ccfacae-3593-4fbe-94a2-34b3a3d643b1";
        let child_id = "3c420f73-677d-49be-9db8-b6b052119ca5";

        let parent_logs = brain.join(parent_id).join(".system_generated").join("logs");
        let child_logs = brain.join(child_id).join(".system_generated").join("logs");
        let tasks_dir = brain
            .join(parent_id)
            .join(".system_generated")
            .join("tasks");
        std::fs::create_dir_all(&parent_logs).unwrap();
        std::fs::create_dir_all(&child_logs).unwrap();
        std::fs::create_dir_all(&tasks_dir).unwrap();
        std::fs::write(tasks_dir.join("task-42.log"), "building...\ndone\n").unwrap();

        std::fs::write(
            parent_logs.join("transcript.jsonl"),
            concat!(
                "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-11-20T20:15:19Z\",\"tool_calls\":[{\"name\":\"invoke_subagent\",\"args\":{\"Model\":\"\\\"flash_lite\\\"\",\"Prompt\":\"\\\"Inspect parser\\\"\",\"Role\":\"\\\"Code Researcher\\\"\",\"TypeName\":\"\\\"research\\\"\"}}]}\n",
                "{\"step_index\":2,\"type\":\"GENERIC\",\"created_at\":\"2026-11-20T20:15:23Z\",\"content\":\"Created At: 2026-11-20\\nCreated the following subagent:\\n{\\n  \\\"conversationId\\\":  \\\"3c420f73-677d-49be-9db8-b6b052119ca5\\\",\\n  \\\"logAbsoluteUri\\\": \\\"file:///ignored\\\"\\n}\\n\"}\n",
                "{\"step_index\":3,\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-11-20T20:15:24Z\",\"tool_calls\":[{\"name\":\"run_command\",\"args\":{\"CommandLine\":\"\\\"cargo test\\\"\",\"toolSummary\":\"\\\"Run tests\\\"\"}}]}\n",
                "{\"step_index\":4,\"type\":\"GENERIC\",\"status\":\"RUNNING\",\"created_at\":\"2026-11-20T20:15:25Z\",\"content\":\"Created At: 2026-11-20\\nTool is running as a background task with task id: 9ccfacae-3593-4fbe-94a2-34b3a3d643b1/task-42\\nTask Description: cargo test\\nTask logs are available at: file:///ignored\"}\n",
                "{\"step_index\":5,\"type\":\"SYSTEM_MESSAGE\",\"created_at\":\"2026-11-20T20:15:30Z\",\"content\":\"<SYSTEM_MESSAGE>\\n[Message] timestamp=2026-11-20T20:15:30Z sender=3c420f73-677d-49be-9db8-b6b052119ca5 priority=MESSAGE_PRIORITY_HIGH content=OK\\n</SYSTEM_MESSAGE>\"}\n"
            ),
        )
        .unwrap();

        std::fs::write(
            child_logs.join("transcript.jsonl"),
            concat!(
                "{\"step_index\":0,\"type\":\"SYSTEM_MESSAGE\",\"created_at\":\"2026-11-20T20:15:23Z\",\"content\":\"<SYSTEM_MESSAGE>\\n[Message] timestamp=2026-11-20T20:15:23Z sender=9ccfacae-3593-4fbe-94a2-34b3a3d643b1 priority=MESSAGE_PRIORITY_HIGH content=Inspect parser\\n</SYSTEM_MESSAGE>\"}\n",
                "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-11-20T20:15:24Z\",\"model\":\"gemini-3.5-flash-lite\",\"input_tokens\":5000,\"cache_read_tokens\":1000,\"output_tokens\":200,\"thinking\":\"Checking files\",\"tool_calls\":[{\"name\":\"read_file\",\"args\":{\"Path\":\"\\\"src/lib.rs\\\"\"}}]}\n",
                "{\"step_index\":2,\"type\":\"GENERIC\",\"status\":\"DONE\",\"created_at\":\"2026-11-20T20:15:25Z\",\"content\":\"Created At: 2026-11-20\\nCompleted At: 2026-11-20\\nfn main() {}\"}\n",
                "{\"step_index\":3,\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-11-20T20:15:26Z\",\"model\":\"gemini-3.5-flash-lite\",\"input_tokens\":1000,\"cache_read_tokens\":4000,\"output_tokens\":50,\"content\":\"Looks good!\"}\n"
            ),
        )
        .unwrap();

        let snap = scan_session_in(brain, parent_id, &["call_anchor_1".into()]);
        assert_eq!(snap.subagents.len(), 1);
        let sub = &snap.subagents[0];
        assert_eq!(sub.id, child_id);
        assert_eq!(sub.agent_type, "Code Researcher");
        assert_eq!(sub.description.as_deref(), Some("Inspect parser"));
        assert_eq!(sub.tool_use_id.as_deref(), Some("call_anchor_1"));
        assert_eq!(sub.status, SubagentStatus::Complete);
        assert_eq!(sub.model.as_deref(), Some("gemini-3.5-flash-lite"));
        assert_eq!(sub.tokens, Some(11250));
        assert_eq!(sub.tool_calls, Some(1));
        assert_eq!(sub.last_tool_name.as_deref(), Some("read_file"));
        assert_eq!(sub.last_tool_summary.as_deref(), Some("src/lib.rs"));

        assert_eq!(snap.tasks.len(), 2);
        let agent_task = snap.tasks.iter().find(|t| t.id == child_id).unwrap();
        assert_eq!(agent_task.task_type, "local_agent");
        assert_eq!(agent_task.status, "completed");
        assert_eq!(agent_task.subagent_id.as_deref(), Some(child_id));
        assert!(!agent_task.has_output);

        let shell_task = snap.tasks.iter().find(|t| t.id == "task-42").unwrap();
        assert_eq!(shell_task.task_type, "local_bash");
        assert_eq!(shell_task.status, "running");
        assert_eq!(shell_task.description.as_deref(), Some("Run tests"));
        assert_eq!(shell_task.command.as_deref(), Some("cargo test"));
        assert!(shell_task.has_output);
        assert_eq!(
            shell_task.output_file.as_deref(),
            Some(tasks_dir.join("task-42.log").as_path())
        );

        // Replay child conversation.
        let replay = replay_subagent_in(brain, child_id).unwrap();
        assert_eq!(replay.seq, 5);
        assert!(matches!(
            &replay.items[0],
            ConversationItem::UserMessage { text, .. } if text == "Inspect parser"
        ));
        assert!(matches!(
            &replay.items[1],
            ConversationItem::Reasoning { text, .. } if text == "Checking files"
        ));
        assert!(matches!(
            &replay.items[2],
            ConversationItem::ToolUse { name, input, .. }
            if name == "read_file" && input["Path"] == "src/lib.rs"
        ));
        assert!(matches!(
            &replay.items[3],
            ConversationItem::ToolResult { content, is_error: false, .. } if content == "fn main() {}"
        ));
        assert!(matches!(
            &replay.items[4],
            ConversationItem::AssistantText { text, .. } if text == "Looks good!"
        ));
    }

    #[test]
    fn managed_background_count_does_not_double_count_linked_local_agent_task() {
        let mut state = SessionState::new("s1".into(), None);
        state.provider = "antigravity".into();
        let snap = ArtifactSnapshot {
            subagents: vec![SubagentInfo {
                id: "child-1".into(),
                agent_type: "Researcher".into(),
                status: SubagentStatus::Running,
                started_at: 1000,
                completed_at: None,
                description: None,
                tool_use_id: None,
                model: None,
                last_tool_name: None,
                last_tool_summary: None,
                tokens: None,
                cost_usd: None,
                tool_calls: None,
            }],
            tasks: vec![
                {
                    let mut t = BackgroundTask::new("child-1", "local_agent", 1000);
                    t.subagent_id = Some("child-1".into());
                    t
                },
                BackgroundTask::new("task-1", "local_bash", 1000),
            ],
            stamps: vec![],
        };
        assert!(apply_snapshot(&mut state, snap));
        // 1 running subagent + 1 running bash task = 2 (NOT 3).
        assert_eq!(state.background_tasks, 2);
    }
}

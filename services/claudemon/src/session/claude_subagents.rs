//! Claude children are provider artifacts, never independently adopted sessions.
use super::{
    conversation::{self, ApiErrorMarking, ConversationItem},
    state::{HookEvent, SessionState, SubagentInfo, SubagentStatus},
    transcript,
};
use anyhow::{bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

pub const MAX_CHILD_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CHILDREN: usize = 128;
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
fn now() -> i64 {
    milliseconds(time::OffsetDateTime::now_utc())
}
fn milliseconds(stamp: time::OffsetDateTime) -> i64 {
    (stamp.unix_timestamp_nanos() / 1_000_000) as i64
}
fn text(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}
fn trusted(path: &Path) -> bool {
    transcript::allowed_roots()
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| path.starts_with(root))
        || transcript::path_is_allowed(path)
}

/// Enrichment only: never mutate mode, pending ownership or background counts.
pub fn hook(state: &mut SessionState, event: &HookEvent) -> bool {
    if state.provider != "claude" {
        return false;
    }
    let Some(id) = event
        .payload
        .get("agent_id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
    else {
        return false;
    };
    let payload = Value::Object(event.payload.clone());
    let position = state.subagents.iter().position(|s| s.id == id);
    if position.is_none() && event.event != "SubagentStart" {
        return false;
    }
    if position.is_none() {
        if state.subagents.len() >= MAX_CHILDREN {
            return false;
        }
        state.subagents.push(SubagentInfo {
            id: id.into(),
            agent_type: text(payload["agent_type"].as_str().unwrap_or("Agent"), 256),
            status: SubagentStatus::Running,
            started_at: now(),
            completed_at: None,
            description: None,
            tool_use_id: None,
            model: None,
            last_tool_name: None,
            last_tool_summary: None,
            tokens: None,
            cost_usd: None,
            tool_calls: None,
        });
    }
    let child = state.subagents.iter_mut().find(|s| s.id == id).unwrap();
    let previous = child.clone();
    if event.event == "SubagentStart" && child.status == SubagentStatus::Complete {
        child.status = SubagentStatus::Running;
        child.started_at = now();
        child.completed_at = None;
    }
    if event.event == "SubagentStop" {
        child.status = SubagentStatus::Complete;
        child.completed_at.get_or_insert(now());
    }
    if let Some(tool) = payload["tool_use_id"].as_str().filter(|s| {
        event.event == "SubagentStart"
            && s.len() <= 512
            && !s.is_empty()
            && s.bytes().all(|b| b.is_ascii_graphic())
    }) {
        child.tool_use_id = Some(tool.into());
    }
    if let Some(model) = payload["model"].as_str() {
        child.model = Some(text(model, 256));
    }
    if event.event == "PreToolUse" {
        if let Some(tool) = payload["tool_name"].as_str() {
            child.last_tool_name = Some(text(tool, 256));
            child.last_tool_summary = Some(text(
                &transcript::summarize_tool_input(tool, &payload["tool_input"]),
                512,
            ));
        }
    }
    position.is_none() || *child != previous
}

/// The stream transport's own subagent lifecycle: Claude Code emits
/// `task_started` / `task_progress` / `task_updated` / `task_notification`
/// frames for every Agent/Task subagent, foreground or background, and the
/// task id IS the agent id hooks and `subagents/agent-<id>.jsonl` use. Unlike
/// hooks these need no shell or settings file, so a stream session's children
/// appear even where hook delivery fails (Windows without Git Bash runs the
/// curl hook under PowerShell; a profile's config dir carries no hooks).
/// Enrichment only, like [`hook`]: never mode, pending or background counts.
pub fn stream_frame(state: &mut SessionState, frame: &Value) -> bool {
    if state.provider != "claude" || frame["type"] != "system" {
        return false;
    }
    let subtype = frame["subtype"].as_str().unwrap_or("");
    if !matches!(
        subtype,
        "task_started" | "task_progress" | "task_updated" | "task_notification"
    ) {
        return false;
    }
    let Some(id) = frame["task_id"].as_str().filter(|id| valid_id(id)) else {
        return false;
    };
    let position = state.subagents.iter().position(|s| s.id == id);
    if position.is_none() {
        // Only an agent's start opens a row: shells and workflows are tasks,
        // not children, and a late frame for an unknown id carries no type.
        if subtype != "task_started"
            || frame["task_type"] != "local_agent"
            || state.subagents.len() >= MAX_CHILDREN
        {
            return false;
        }
        state.subagents.push(SubagentInfo {
            id: id.into(),
            agent_type: "Agent".into(),
            status: SubagentStatus::Running,
            started_at: now(),
            completed_at: None,
            description: None,
            tool_use_id: None,
            model: None,
            last_tool_name: None,
            last_tool_summary: None,
            tokens: None,
            cost_usd: None,
            tool_calls: None,
        });
    }
    let child = state.subagents.iter_mut().find(|s| s.id == id).unwrap();
    let previous = child.clone();
    if let Some(kind) = frame["subagent_type"].as_str().filter(|s| !s.is_empty()) {
        child.agent_type = text(kind, 256);
    }
    if let Some(tool) = frame["tool_use_id"]
        .as_str()
        .filter(|s| s.len() <= 512 && !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic()))
    {
        child.tool_use_id.get_or_insert_with(|| tool.into());
    }
    let usage = &frame["usage"];
    if let Some(calls) = usage["tool_uses"].as_u64() {
        child.tool_calls = Some(calls);
    }
    // The transcript artifact scan sums exact per-message usage; the frame's
    // running total only stands in until that scan has reported.
    if let Some(tokens) = usage["total_tokens"].as_u64() {
        child.tokens.get_or_insert(tokens);
    }
    match subtype {
        "task_started" => {
            if let Some(description) = frame["description"].as_str() {
                child.description = Some(text(description, 180));
            }
            if child.status == SubagentStatus::Complete {
                child.status = SubagentStatus::Running;
                child.started_at = now();
                child.completed_at = None;
            }
        }
        // A progress frame's description is the agent's latest activity
        // ("Reading README.md"), not its name.
        "task_progress" => {
            if let Some(tool) = frame["last_tool_name"].as_str() {
                child.last_tool_name = Some(text(tool, 256));
            }
            if let Some(activity) = frame["description"].as_str() {
                child.last_tool_summary = Some(text(activity, 512));
            }
        }
        _ => {
            let status = frame["patch"]["status"]
                .as_str()
                .or_else(|| frame["status"].as_str())
                .unwrap_or("");
            if matches!(
                status,
                "completed" | "failed" | "killed" | "stopped" | "error" | "cancelled"
            ) {
                child.status = SubagentStatus::Complete;
                let ended = frame["patch"]["end_time"].as_i64().filter(|t| *t > 0);
                child.completed_at.get_or_insert(ended.unwrap_or_else(now));
            }
        }
    }
    position.is_none() || *child != previous
}

/// Exact-session lookup only. Never fall back to a nearby/latest transcript.
pub async fn discover_parent(state: &SessionState) -> Option<String> {
    if state.provider != "claude" || !valid_id(&state.session_id) {
        return None;
    }
    let cwd = state.cwd.as_deref()?;
    let project = transcript::project_dir_name(cwd)?;
    for root in transcript::allowed_roots() {
        let candidate = root
            .join(&project)
            .join(format!("{}.jsonl", state.session_id));
        let Ok(path) = tokio::fs::canonicalize(candidate).await else {
            continue;
        };
        let Ok(root) = tokio::fs::canonicalize(root).await else {
            continue;
        };
        let expected = root
            .join(&project)
            .join(format!("{}.jsonl", state.session_id));
        if path == expected && path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

async fn child_path(main: &str, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        bail!("invalid child id");
    }
    let main = tokio::fs::canonicalize(main).await?;
    if !trusted(&main) || main.extension().is_none_or(|e| e != "jsonl") {
        bail!("untrusted parent transcript");
    }
    let base = main.with_extension("");
    let directory = tokio::fs::canonicalize(base.join("subagents")).await?;
    // A symlinked child directory must not redirect to another parent or root.
    if directory != base.join("subagents") {
        bail!("redirected child directory");
    }
    let expected = directory.join(format!("agent-{id}.jsonl"));
    let path = tokio::fs::canonicalize(&expected).await?;
    if path != expected || !trusted(&path) {
        bail!("redirected child transcript");
    }
    Ok(path)
}

pub struct Replay {
    pub items: Vec<ConversationItem>,
    pub seq: usize,
    pub first_seq: usize,
}
pub async fn replay(state: &SessionState, id: &str) -> Result<Replay> {
    if state.provider != "claude" || !state.subagents.iter().any(|s| s.id == id) {
        bail!("child not exposed on parent");
    }
    let main = state
        .transcript_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("parent transcript unavailable"))?;
    let path = child_path(main, id).await?;
    let rows = read_rows(&path).await?;
    let mut items = Vec::new();
    for mut row in rows {
        // In the child's own transcript sidechain rows are its main timeline.
        row["isSidechain"] = Value::Bool(false);
        items.extend(conversation::items_from_row(&row, ApiErrorMarking::Tailer));
    }
    let seq = items.len();
    if items.len() > 2000 {
        items.drain(..items.len() - 2000);
    }
    let first_seq = if items.is_empty() {
        0
    } else {
        seq - items.len() + 1
    };
    Ok(Replay {
        items,
        seq,
        first_seq,
    })
}

async fn read_rows(path: &Path) -> Result<Vec<Value>> {
    let file = tokio::fs::File::open(path).await?;
    if !file.metadata().await?.is_file() || file.metadata().await?.len() > MAX_CHILD_BYTES {
        bail!("child transcript too large");
    }
    let mut bytes = Vec::new();
    file.take(MAX_CHILD_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_CHILD_BYTES {
        bail!("child transcript too large");
    }
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(Value::is_object)
        .collect())
}

/// Claude's sidecar names the parent Agent tool that actually spawned this
/// child. It is optional; a missing or refused file leaves transcript fallback.
async fn read_metadata(transcript_path: &Path) -> Result<Value> {
    const MAX_METADATA_BYTES: u64 = 64 * 1024;
    let expected = transcript_path.with_extension("meta.json");
    let path = tokio::fs::canonicalize(&expected).await?;
    if path != expected || !trusted(&path) {
        bail!("redirected child metadata");
    }
    let file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.len() > MAX_METADATA_BYTES {
        bail!("child metadata too large");
    }
    let mut bytes = Vec::new();
    file.take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        bail!("child metadata too large");
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    if !value.is_object() {
        bail!("invalid child metadata");
    }
    Ok(value)
}

pub async fn artifacts(state: &SessionState) -> Result<Vec<SubagentInfo>> {
    let Some(main) = state.transcript_path.as_deref() else {
        return Ok(vec![]);
    };
    let main = tokio::fs::canonicalize(main).await?;
    if !trusted(&main) {
        bail!("untrusted parent transcript");
    }
    let mut directory = tokio::fs::read_dir(main.with_extension("").join("subagents")).await?;
    let mut children = Vec::new();
    let mut examined = 0usize;
    while children.len() < MAX_CHILDREN && examined < MAX_CHILDREN * 2 {
        let Some(entry) = directory.next_entry().await? else {
            break;
        };
        examined += 1;
        let filename = entry.file_name();
        let Some(id) = filename
            .to_str()
            .and_then(|s| s.strip_prefix("agent-"))
            .and_then(|s| s.strip_suffix(".jsonl"))
        else {
            continue;
        };
        let Ok(path) = child_path(main.to_str().unwrap_or_default(), id).await else {
            continue;
        };
        let Ok(rows) = read_rows(&path).await else {
            continue;
        };
        let mut child = state
            .subagents
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .unwrap_or(SubagentInfo {
                id: id.into(),
                agent_type: "Agent".into(),
                status: SubagentStatus::Running,
                started_at: 0,
                completed_at: None,
                description: None,
                tool_use_id: None,
                model: None,
                last_tool_name: None,
                last_tool_summary: None,
                tokens: None,
                cost_usd: None,
                tool_calls: None,
            });
        if let Ok(metadata) = read_metadata(&path).await {
            if let Some(kind) = metadata["agentType"].as_str() {
                child.agent_type = text(kind, 256);
            }
            if let Some(description) = metadata["description"].as_str() {
                child.description = Some(text(description, 180));
            }
            if let Some(anchor) = metadata["toolUseId"].as_str().filter(|s| {
                !s.is_empty() && s.len() <= 512 && s.bytes().all(|b| b.is_ascii_graphic())
            }) {
                child.tool_use_id = Some(anchor.into());
            }
            // A requested model in metadata is not runtime model evidence.
        }
        let mut tokens = 0u64;
        let mut tools = std::collections::BTreeSet::new();
        let mut usage_seen = std::collections::BTreeSet::new();
        let mut usage_reported = false;
        for row in &rows {
            if let Some(stamp) = row["timestamp"].as_str().and_then(|s| {
                time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
            }) {
                let stamp = milliseconds(stamp);
                child.started_at = if child.started_at == 0 {
                    stamp
                } else {
                    child.started_at.min(stamp)
                };
            }
            if let Some(model) = row.pointer("/message/model").and_then(Value::as_str) {
                child.model = Some(text(model, 256));
            }
            if child.description.is_none() && row["type"] == "user" {
                child.description = transcript::blocks(&row["message"]["content"])
                    .iter()
                    .find_map(|block| {
                        if let transcript::Block::Text { text } = block {
                            Some(text.chars().take(180).collect())
                        } else {
                            None
                        }
                    });
            }
            if let Some(usage) = row.pointer("/message/usage") {
                usage_reported |= usage.is_object();
                let key = row
                    .pointer("/message/id")
                    .or_else(|| row.get("uuid"))
                    .and_then(Value::as_str);
                if usage.is_object() && key.is_none_or(|k| usage_seen.insert(k.to_owned())) {
                    for name in [
                        "input_tokens",
                        "output_tokens",
                        "cache_read_input_tokens",
                        "cache_creation_input_tokens",
                    ] {
                        tokens = tokens.saturating_add(usage[name].as_u64().unwrap_or_default());
                    }
                }
            }
            for block in transcript::blocks(&row["message"]["content"]) {
                if let transcript::Block::ToolUse { id, name, input } = block {
                    if let Some(id) = id.filter(|s| s.len() <= 512) {
                        tools.insert(id.to_owned());
                    }
                    child.last_tool_name = Some(text(name, 256));
                    child.last_tool_summary =
                        Some(text(&transcript::summarize_tool_input(name, input), 512));
                }
            }
        }
        child.tokens = usage_reported.then_some(tokens);
        let transcript = transcript::Transcript {
            messages: rows
                .iter()
                .filter_map(|row| {
                    let role = row["type"].as_str()?;
                    let mut raw = row.clone();
                    raw["isSidechain"] = Value::Bool(false);
                    Some(transcript::TranscriptMessage {
                        role: role.into(),
                        content: row["message"]["content"].clone(),
                        raw,
                    })
                })
                .collect(),
            ..Default::default()
        };
        child.cost_usd = super::usage::from_transcript(&transcript).map(|usage| usage.cost_usd);
        child.tool_calls = Some(tools.len() as u64);
        // Parent Input is not a child completion signal: detached workers can
        // outlive that turn. Only an explicit final child stop is authoritative.
        if let Some(last) = rows
            .last()
            .filter(|row| row["type"] == "assistant" && row["message"]["stop_reason"] == "end_turn")
        {
            child.status = SubagentStatus::Complete;
            child.completed_at = last["timestamp"]
                .as_str()
                .and_then(|s| {
                    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                        .ok()
                })
                .map(milliseconds)
                .or(child.completed_at);
        }
        children.push(child);
    }
    Ok(children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hook_event(name: &str, payload: Value) -> HookEvent {
        HookEvent {
            event: name.into(),
            session_id: "parent".into(),
            cwd: Some("/project".into()),
            timestamp: None,
            payload: payload.as_object().unwrap().clone(),
        }
    }

    /// Frames captured from Claude Code 2.1.286 (`claude -p --output-format
    /// stream-json`) for one foreground Agent call. No hooks were involved.
    fn foreground_agent_frames() -> Vec<Value> {
        vec![
            json!({"type":"system","subtype":"task_started","task_id":"a3b3fa98f26be5951","tool_use_id":"toolu_0189BCcra1oujC1cG3tBgLCw","description":"read readme","subagent_type":"general-purpose","is_backgrounded":false,"spawn_depth":1,"task_type":"local_agent","prompt":"Read README.md and reply with its first line only."}),
            json!({"type":"system","subtype":"task_progress","task_id":"a3b3fa98f26be5951","tool_use_id":"toolu_0189BCcra1oujC1cG3tBgLCw","description":"Reading README.md","subagent_type":"general-purpose","usage":{"total_tokens":15618,"tool_uses":1,"duration_ms":1657},"last_tool_name":"Read"}),
            json!({"type":"system","subtype":"task_updated","task_id":"a3b3fa98f26be5951","patch":{"status":"completed","end_time":1_791_408_091_015_i64}}),
            json!({"type":"system","subtype":"task_notification","task_id":"a3b3fa98f26be5951","tool_use_id":"toolu_0189BCcra1oujC1cG3tBgLCw","status":"completed","summary":"hello world","usage":{"total_tokens":17130,"tool_uses":1,"duration_ms":2975}}),
        ]
    }

    #[test]
    fn stream_frames_open_run_and_close_a_foreground_child_without_hooks() {
        let mut state = SessionState::new("parent".into(), Some("/project".into()));
        state.provider = "claude".into();
        let mode = state.mode;
        let frames = foreground_agent_frames();
        assert!(stream_frame(&mut state, &frames[0]));
        let child = &state.subagents[0];
        assert_eq!(child.id, "a3b3fa98f26be5951");
        assert_eq!(child.agent_type, "general-purpose");
        assert_eq!(child.status, SubagentStatus::Running);
        assert_eq!(child.description.as_deref(), Some("read readme"));
        assert_eq!(
            child.tool_use_id.as_deref(),
            Some("toolu_0189BCcra1oujC1cG3tBgLCw")
        );
        assert!(stream_frame(&mut state, &frames[1]));
        let child = &state.subagents[0];
        assert_eq!(child.description.as_deref(), Some("read readme"));
        assert_eq!(child.last_tool_name.as_deref(), Some("Read"));
        assert_eq!(
            child.last_tool_summary.as_deref(),
            Some("Reading README.md")
        );
        assert_eq!(child.tool_calls, Some(1));
        assert_eq!(child.tokens, Some(15618));
        assert!(stream_frame(&mut state, &frames[2]));
        assert_eq!(state.subagents[0].status, SubagentStatus::Complete);
        assert_eq!(state.subagents[0].completed_at, Some(1791408091015));
        // The notification repeats the end; nothing a client sees changes.
        assert!(!stream_frame(&mut state, &frames[3]));
        assert_eq!(state.subagents.len(), 1);
        assert_eq!(state.mode, mode);
        assert_eq!(state.background_tasks, 0);
    }

    #[test]
    fn stream_frames_ignore_shells_unknown_ids_and_other_providers() {
        let mut state = SessionState::new("parent".into(), None);
        state.provider = "claude".into();
        let shell = json!({"type":"system","subtype":"task_started","task_id":"b1","task_type":"local_bash","description":"npm test","is_backgrounded":true});
        assert!(!stream_frame(&mut state, &shell));
        let orphan = json!({"type":"system","subtype":"task_progress","task_id":"never-started","description":"x"});
        assert!(!stream_frame(&mut state, &orphan));
        let hostile = json!({"type":"system","subtype":"task_started","task_id":"../escape","task_type":"local_agent"});
        assert!(!stream_frame(&mut state, &hostile));
        assert!(state.subagents.is_empty());
        let mut codex = SessionState::new("c".into(), None);
        codex.provider = "codex".into();
        assert!(!stream_frame(&mut codex, &foreground_agent_frames()[0]));
    }

    #[test]
    fn stream_frames_and_hooks_converge_on_one_row() {
        let store = super::super::SessionStore::new();
        store.register_managed("parent", "/project", "claude");
        store.set_transport("parent", super::super::state::Transport::Stream);
        store.set_background_tasks("parent", 3);
        let mut updates = store.subscribe();
        let frames = foreground_agent_frames();
        assert!(store.observe_claude_stream_subagents("parent", &frames[0]));
        assert!(updates.try_recv().is_ok(), "a new child is broadcast");
        // The hook for the same agent enriches the stream's row.
        store.ingest(hook_event(
            "SubagentStart",
            json!({"agent_id":"a3b3fa98f26be5951","agent_type":"general-purpose","model":"claude-haiku-4-5"}),
        ));
        assert!(store.observe_claude_stream_subagents("parent", &frames[2]));
        store.ingest(hook_event(
            "SubagentStop",
            json!({"agent_id":"a3b3fa98f26be5951"}),
        ));
        let state = store.get("parent").unwrap();
        assert_eq!(state.subagents.len(), 1);
        assert_eq!(
            state.subagents[0].model.as_deref(),
            Some("claude-haiku-4-5")
        );
        assert_eq!(state.subagents[0].status, SubagentStatus::Complete);
        assert_eq!(state.subagents[0].completed_at, Some(1791408091015));
        assert_eq!(state.background_tasks, 3, "counts are not ours to touch");
        assert!(!store.observe_claude_stream_subagents("parent", &json!({"type":"assistant"})));
    }

    #[tokio::test]
    async fn metadata_anchors_actual_child_and_preserves_subsecond_timestamps() {
        let root = std::env::temp_dir().join(format!("claude-meta-{}", uuid::Uuid::new_v4()));
        let project = root.join("projects/-project");
        let directory = project.join("parent/subagents");
        std::fs::create_dir_all(&directory).unwrap();
        transcript::allow_root(root.join("projects"));
        let main = project.join("parent.jsonl");
        std::fs::write(&main, "").unwrap();
        let child_path = directory.join("agent-child.jsonl");
        let rows = [
            json!({"type":"user","timestamp":"2026-10-01T10:00:00.125Z","message":{"content":"Inspect actual parser"}}),
            json!({"type":"assistant","timestamp":"2026-10-01T10:00:00.875Z","message":{"model":"claude-sonnet-4-6","stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}),
        ];
        std::fs::write(
            &child_path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let mut state = SessionState::new("parent".into(), Some("/project".into()));
        state.transcript_path = Some(main.canonicalize().unwrap().to_string_lossy().into_owned());
        let fallback = artifacts(&state).await.unwrap();
        assert_eq!(
            fallback[0].tool_use_id, None,
            "missing metadata must retain usable child fallback"
        );
        assert_eq!(
            fallback[0].description.as_deref(),
            Some("Inspect actual parser")
        );
        assert_eq!(
            fallback[0].completed_at.unwrap() - fallback[0].started_at,
            750
        );
        let metadata_path = directory.join("agent-child.meta.json");
        std::fs::write(&metadata_path,json!({"agentType":"Explore","description":"Bounded parser audit","toolUseId":"toolu-parent-agent","model":"requested-wrong-model"}).to_string()).unwrap();
        let child = artifacts(&state).await.unwrap();
        assert_eq!(child[0].agent_type, "Explore");
        assert_eq!(
            child[0].description.as_deref(),
            Some("Bounded parser audit")
        );
        assert_eq!(child[0].tool_use_id.as_deref(), Some("toolu-parent-agent"));
        assert_eq!(child[0].model.as_deref(), Some("claude-sonnet-4-6"));
        std::fs::write(&metadata_path,json!({"agentType":"a".repeat(1000),"description":"d".repeat(1000),"toolUseId":"x".repeat(513)}).to_string()).unwrap();
        let bounded = artifacts(&state).await.unwrap();
        assert_eq!(bounded[0].agent_type.len(), 256);
        assert_eq!(bounded[0].description.as_ref().unwrap().len(), 180);
        assert_eq!(
            bounded[0].tool_use_id, None,
            "oversized identity cannot be clipped into another anchor"
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(&metadata_path)
            .unwrap()
            .set_len(64 * 1024 + 1)
            .unwrap();
        assert_eq!(artifacts(&state).await.unwrap()[0].tool_use_id, None);
        #[cfg(unix)]
        {
            std::fs::remove_file(&metadata_path).unwrap();
            let other = directory.join("agent-other.meta.json");
            std::fs::write(&other, json!({"toolUseId":"wrong-parent-call"}).to_string()).unwrap();
            std::os::unix::fs::symlink(&other, &metadata_path).unwrap();
            assert_eq!(
                artifacts(&state).await.unwrap()[0].tool_use_id,
                None,
                "same-directory redirect must not attach another child's metadata"
            );
            std::fs::remove_file(&child_path).unwrap();
            let other_transcript = directory.join("other.jsonl");
            std::fs::write(&other_transcript, rows[1].to_string()).unwrap();
            std::os::unix::fs::symlink(&other_transcript, &child_path).unwrap();
            assert!(
                artifacts(&state).await.unwrap().is_empty(),
                "a redirected JSONL must not substitute another child's sidecar identity"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_hooks_enrich_children_without_claiming_pending_or_counts() {
        let store = super::super::SessionStore::new();
        store.register_managed("parent", "/project", "claude");
        store.set_transport("parent", super::super::state::Transport::Stream);
        store.set_managed_mode(
            "parent",
            super::super::SessionMode::Approval,
            super::super::state::PendingWrite::Park(
                super::super::state::PendingOwner::Primary,
                super::super::state::Pending::Approval {
                    tool: Some("Read".into()),
                    summary: None,
                    raw: Value::Null,
                },
            ),
        );
        store.set_background_tasks("parent", 7);
        store.ingest(hook_event(
            "SessionStart",
            json!({"transcript_path":"/parent.jsonl"}),
        ));
        for name in [
            "SubagentStart",
            "SubagentStart",
            "PreToolUse",
            "SubagentStop",
        ] {
            store.ingest(hook_event(name, json!({"agent_id":"child","agent_type":"Explore","tool_name":"Read","tool_input":{"file_path":"src/lib.rs"}})));
        }
        store.ingest(hook_event(
            "PreToolUse",
            json!({"agent_id":"child","tool_name":"Read","transcript_path":"/child.jsonl"}),
        ));
        let state = store.get("parent").unwrap();
        assert_eq!(state.mode, super::super::SessionMode::Approval);
        assert!(state.pending().is_some());
        assert_eq!(state.background_tasks, 7);
        assert_eq!(state.transcript_path.as_deref(), Some("/parent.jsonl"));
        assert_eq!(state.subagents.len(), 1);
        assert_eq!(state.subagents[0].status, SubagentStatus::Complete);
        assert_eq!(state.subagents[0].last_tool_name.as_deref(), Some("Read"));
        store.ingest(hook_event(
            "SubagentStart",
            json!({"agent_id":"detached","agent_type":"Explore"}),
        ));
        store.set_managed_mode(
            "parent",
            super::super::SessionMode::Input,
            super::super::state::PendingWrite::Resolve(super::super::state::PendingOwner::Primary),
        );
        let idle = store.get("parent").unwrap();
        assert_eq!(
            idle.subagents
                .iter()
                .find(|s| s.id == "detached")
                .unwrap()
                .status,
            SubagentStatus::Running
        );
        assert_eq!(idle.background_tasks, 7);
        let observed = idle.clone();
        store.ingest(hook_event("SubagentStop", json!({"agent_id":"detached"})));
        store.ingest(hook_event("PreToolUse",json!({"agent_id":"child","tool_name":"Bash","tool_input":{"command":"newer activity"}})));
        store.enrich_claude_artifacts(&observed, "/parent.jsonl", observed.subagents.clone());
        let merged = store.get("parent").unwrap();
        assert_eq!(
            merged
                .subagents
                .iter()
                .find(|s| s.id == "detached")
                .unwrap()
                .status,
            SubagentStatus::Complete
        );
        assert_eq!(
            merged
                .subagents
                .iter()
                .find(|s| s.id == "child")
                .unwrap()
                .last_tool_name
                .as_deref(),
            Some("Bash")
        );
    }

    #[tokio::test]
    async fn exact_parent_discovery_and_confined_child_replay_keep_raw_blocks() {
        // Transcript roots are process-global, and discovery searches every
        // registered root: a session id and project shared with another test
        // let this test find that test's transcript. Both are unique here.
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let parent = format!("parent{unique}");
        let cwd = format!("/project{unique}");
        let root = std::env::temp_dir().join(format!("claude-child-{unique}"));
        let projects = root.join("projects");
        let project = projects.join(transcript::project_dir_name(&cwd).unwrap());
        let child_dir = project.join(&parent).join("subagents");
        std::fs::create_dir_all(&child_dir).unwrap();
        transcript::allow_root(projects);
        let main = project.join(format!("{parent}.jsonl"));
        std::fs::write(&main, "").unwrap();
        let rows = [
            json!({"type":"user","isSidechain":true,"message":{"content":[{"type":"text","text":"Inspect parser"}]}}),
            json!({"type":"assistant","isSidechain":true,"message":{"id":"msg-1","model":"claude-sonnet-4-6","usage":{"input_tokens":10,"output_tokens":3},"content":[{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"src/lib.rs"}}]}}),
            json!({"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"read","content":"source"}]}}),
            json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"Checked"}]}}),
        ];
        std::fs::write(
            child_dir.join("agent-child.jsonl"),
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let mut state = SessionState::new(parent.clone(), Some(cwd.clone()));
        state.mode = super::super::SessionMode::Input;
        state.transcript_path = discover_parent(&state).await;
        assert_eq!(
            Path::new(state.transcript_path.as_deref().unwrap()),
            main.canonicalize().unwrap()
        );
        assert!(
            replay(&state, "child").await.is_err(),
            "artifact alone is not replay membership"
        );
        state.subagents = artifacts(&state).await.unwrap();
        assert_eq!(state.subagents.len(), 1);
        assert_eq!(state.subagents[0].tokens, Some(13));
        assert_eq!(
            state.subagents[0].model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(state.subagents[0].tool_calls, Some(1));
        assert_eq!(
            state.subagents[0].status,
            SubagentStatus::Running,
            "an idle parent is not evidence that its child stopped"
        );
        assert_eq!(state.subagents[0].completed_at, None);
        let replay = replay(&state, "child").await.unwrap();
        assert!(replay
            .items
            .iter()
            .any(|item| matches!(item,ConversationItem::ToolUse {id,..} if id=="read")));
        assert!(replay.items.iter().any(|item| matches!(item,ConversationItem::ToolResult {tool_use_id,..} if tool_use_id=="read")));
        assert!(super::replay(&state, "../child").await.is_err());
        let zero = json!({"type":"assistant","message":{"id":"zero","model":"claude-sonnet-4-6","usage":{"input_tokens":0,"output_tokens":0},"content":[]}});
        std::fs::write(child_dir.join("agent-child.jsonl"), zero.to_string()).unwrap();
        let zero = artifacts(&state).await.unwrap();
        assert_eq!(zero[0].tokens, Some(0));
        assert_eq!(zero[0].cost_usd, Some(0.0));
        let rows = (0..2005).map(|i| json!({"type":"assistant","message":{"content":[{"type":"text","text":format!("Row {i}")}]}}).to_string()).collect::<Vec<_>>();
        std::fs::write(
            child_dir.join("agent-child.jsonl"),
            format!("null\n[]\n{}", rows.join("\n")),
        )
        .unwrap();
        let absent = artifacts(&state).await.unwrap();
        assert_eq!(absent[0].tokens, None);
        assert_eq!(absent[0].cost_usd, None);
        assert_eq!(absent[0].started_at, 0);
        let trimmed = super::replay(&state, "child").await.unwrap();
        assert_eq!(trimmed.seq, 2005);
        assert_eq!(trimmed.first_seq, 6);
        assert_eq!(trimmed.items.len(), 2000);
        std::fs::OpenOptions::new()
            .write(true)
            .open(child_dir.join("agent-child.jsonl"))
            .unwrap()
            .set_len(MAX_CHILD_BYTES + 1)
            .unwrap();
        assert!(super::replay(&state, "child").await.is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(child_dir.join("agent-child.jsonl")).unwrap();
            std::os::unix::fs::symlink(&main, child_dir.join("agent-child.jsonl")).unwrap();
            assert!(super::replay(&state, "child").await.is_err());
            let other = project.join("other.jsonl");
            std::fs::write(&other, "{}").unwrap();
            std::fs::remove_file(&main).unwrap();
            std::os::unix::fs::symlink(&other, &main).unwrap();
            assert!(
                discover_parent(&state).await.is_none(),
                "an exact filename symlink must not adopt another session"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

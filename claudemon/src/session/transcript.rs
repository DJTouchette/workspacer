//! Reader for Claude Code's on-disk JSONL transcripts at
//! `~/.claude/projects/<encoded-cwd>/*.jsonl`.
//!
//! Each JSONL row is shaped like:
//!
//! ```jsonc
//! {
//!   "type": "user" | "assistant" | "summary" | ...,
//!   "message": { "role": "...", "content": <string | array of blocks> },
//!   "uuid": "...", "parentUuid": "...", ...
//! }
//! ```
//!
//! Content blocks (when `content` is an array) carry one of:
//!   - `{type:"text", text:"..."}`
//!   - `{type:"tool_use", name:"Bash", input:{...}, id:"..."}`
//!   - `{type:"tool_result", tool_use_id:"...", content:..., is_error?:bool}`
//!   - `{type:"thinking", thinking:"..."}`
//!
//! We surface them as `Value` and let the renderer interpret. That keeps
//! us forward-compatible with new block kinds.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[allow(dead_code)]
pub fn projects_dir() -> Option<PathBuf> {
    let base = BaseDirs::new()?;
    Some(base.home_dir().join(".claude").join("projects"))
}

/// Claude encodes `/foo/bar` as `-foo-bar` and `C:\foo` as `C--foo`.
#[allow(dead_code)]
pub fn encoded_cwd(cwd: &str) -> String {
    cwd.replace(['/', '\\', ':'], "-")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptMessage {
    /// "user" | "assistant" | "summary" | ...
    pub role: String,
    /// Raw `message.content` — either a plain string (early Claude format)
    /// or an array of block objects with a `type` discriminator.
    #[serde(default)]
    pub content: Value,
    /// The whole original JSONL row, for clients that want everything.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub raw: Value,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Transcript {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub messages: Vec<TranscriptMessage>,
}

/// Roles we display by default. Everything else (summary, meta entries)
/// is filtered out unless the client opts in.
const RENDERABLE_ROLES: &[&str] = &["user", "assistant"];

/// Best-effort: find the most-recently-modified JSONL for a given cwd and
/// parse it into a flat list of messages. Returns an empty transcript if
/// nothing is found.
pub fn read_for_cwd(cwd: &str) -> Result<Transcript> {
    read_for_cwd_and_session(cwd, None)
}

/// Find and parse the transcript for a cwd, preferring the exact session
/// transcript file when the caller knows the Claude session id.
pub fn read_for_session(cwd: &str, session_id: &str) -> Result<Transcript> {
    read_for_cwd_and_session(cwd, Some(session_id))
}

fn read_for_cwd_and_session(cwd: &str, session_id: Option<&str>) -> Result<Transcript> {
    let Some(root) = projects_dir() else {
        return Ok(Transcript::default());
    };
    let dir = root.join(encoded_cwd(cwd));
    if !dir.exists() {
        return Ok(Transcript::default());
    }

    if let Some(session_id) = session_id {
        let path = dir.join(format!("{session_id}.jsonl"));
        if path.exists() {
            return read_transcript_file(path);
        }

        if let Some(path) = find_session_file(&root, session_id)? {
            return read_transcript_file(path);
        }
    }

    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry
            .metadata()?
            .modified()
            .unwrap_or(std::time::UNIX_EPOCH);
        match &newest {
            Some((best, _)) if *best >= mtime => {}
            _ => newest = Some((mtime, path)),
        }
    }

    let Some((_, path)) = newest else {
        return Ok(Transcript::default());
    };

    read_transcript_file(path)
}

fn read_transcript_file(path: PathBuf) -> Result<Transcript> {
    let text = fs::read_to_string(&path)?;
    let messages = parse_jsonl(&text);

    Ok(Transcript {
        path: Some(path.to_string_lossy().into_owned()),
        messages,
    })
}

fn find_session_file(root: &std::path::Path, session_id: &str) -> Result<Option<PathBuf>> {
    let filename = format!("{session_id}.jsonl");
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path().join(&filename);
        if path.exists() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Parse one transcript's worth of JSONL text into messages. Exposed for
/// tests and for callers that already have the bytes on hand.
pub fn parse_jsonl(text: &str) -> Vec<TranscriptMessage> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(msg) = extract_message(&value) {
            if RENDERABLE_ROLES.contains(&msg.role.as_str()) {
                out.push(msg);
            }
        }
    }
    out
}

fn extract_message(value: &Value) -> Option<TranscriptMessage> {
    // Real Claude transcripts: `type` is the role discriminator
    // ("user"/"assistant"/"summary"/...), and `message.content` carries
    // the payload. Older / synthetic formats put role + content directly
    // at the top level; accept both.
    let role = value
        .get("type")
        .or_else(|| value.get("role"))
        .or_else(|| value.get("message").and_then(|m| m.get("role")))
        .and_then(Value::as_str)?
        .to_string();
    let content = value
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| value.get("content"))
        .cloned()
        .unwrap_or(Value::Null);
    Some(TranscriptMessage {
        role,
        content,
        raw: value.clone(),
    })
}

// ----- block convenience helpers ----------------------------------------

/// Walk an assistant/user `content` value yielding blocks the renderer
/// cares about. Strings are normalized into a single Text block.
pub fn blocks(content: &Value) -> Vec<Block<'_>> {
    match content {
        Value::String(s) => vec![Block::Text { text: s.as_str() }],
        Value::Array(arr) => arr.iter().filter_map(Block::from_value).collect(),
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub enum Block<'a> {
    Text {
        text: &'a str,
    },
    ToolUse {
        name: &'a str,
        input: &'a Value,
        id: Option<&'a str>,
    },
    ToolResult {
        content: &'a Value,
        is_error: bool,
        tool_use_id: Option<&'a str>,
    },
    Thinking {
        text: &'a str,
    },
}

impl<'a> Block<'a> {
    fn from_value(v: &'a Value) -> Option<Self> {
        let kind = v.get("type")?.as_str()?;
        match kind {
            "text" => Some(Block::Text {
                text: v.get("text").and_then(Value::as_str).unwrap_or(""),
            }),
            "tool_use" => Some(Block::ToolUse {
                name: v.get("name").and_then(Value::as_str).unwrap_or("?"),
                input: v.get("input").unwrap_or(&Value::Null),
                id: v.get("id").and_then(Value::as_str),
            }),
            "tool_result" => Some(Block::ToolResult {
                content: v.get("content").unwrap_or(&Value::Null),
                is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                tool_use_id: v.get("tool_use_id").and_then(Value::as_str),
            }),
            "thinking" => Some(Block::Thinking {
                text: v.get("thinking").and_then(Value::as_str).unwrap_or(""),
            }),
            _ => None,
        }
    }
}

/// Produce a one-line summary of a tool's input that's friendlier than
/// raw JSON. Falls back to a compact JSON repr.
pub fn summarize_tool_input(tool: &str, input: &Value) -> String {
    let pick = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let s = match tool {
        "Agent" => first_nonempty(input, &["description", "prompt", "task"]).to_string(),
        "AskUserQuestion" => summarize_question(input),
        "Bash" => pick("command").to_string(),
        "CronCreate" => first_nonempty(input, &["name", "prompt", "schedule"]).to_string(),
        "CronDelete" => format_id("cancel", input),
        "CronList" => "list scheduled tasks".to_string(),
        "Read" => pick("file_path").to_string(),
        "EnterPlanMode" => "enter plan mode".to_string(),
        "EnterWorktree" => first_nonempty(input, &["path", "name"]).to_string(),
        "ExitPlanMode" => first_nonempty(input, &["plan", "summary"]).to_string(),
        "ExitWorktree" => "exit worktree".to_string(),
        "Write" | "Edit" | "MultiEdit" => pick("file_path").to_string(),
        "Grep" => {
            let p = pick("pattern");
            let path = input.get("path").and_then(Value::as_str).unwrap_or(".");
            format!("/{p}/  in {path}")
        }
        "Glob" => pick("pattern").to_string(),
        "ListMcpResourcesTool" => first_nonempty(input, &["server", "cursor"]).to_string(),
        "LSP" => {
            first_nonempty(input, &["query", "symbol", "file_path", "path", "action"]).to_string()
        }
        "Monitor" => pick("command").to_string(),
        "NotebookEdit" => {
            first_nonempty(input, &["notebook_path", "file_path", "cell_id"]).to_string()
        }
        "PowerShell" => pick("command").to_string(),
        "PushNotification" => first_nonempty(input, &["message", "title", "body"]).to_string(),
        "ReadMcpResourceTool" => first_nonempty(input, &["uri", "server"]).to_string(),
        "RemoteTrigger" => first_nonempty(input, &["name", "action", "id"]).to_string(),
        "SendMessage" => first_nonempty(input, &["message", "recipient", "agent_id"]).to_string(),
        "ShareOnboardingGuide" => first_nonempty(input, &["path", "file_path"]).to_string(),
        "Skill" => first_nonempty(input, &["skill", "name", "command", "prompt"]).to_string(),
        "TaskGet" => format_id("get task", input),
        "TaskList" => "list tasks".to_string(),
        "TaskOutput" => format_id("task output", input),
        "TaskStop" => format_id("stop task", input),
        "WebFetch" => pick("url").to_string(),
        "WebSearch" => pick("query").to_string(),
        "ToolSearch" => {
            let q = pick("query");
            if q.is_empty() {
                "search tools".to_string()
            } else {
                format!("search {q}")
            }
        }
        "TaskCreate" => {
            let active = pick("activeForm");
            let desc = pick("description");
            if !active.is_empty() {
                active.to_string()
            } else {
                first_line(desc).to_string()
            }
        }
        "TaskUpdate" => {
            let status = pick("status");
            let active = pick("activeForm");
            let id = input
                .get("taskId")
                .or_else(|| input.get("id"))
                .and_then(Value::as_i64)
                .map(|n| format!("#{n}"))
                .unwrap_or_default();
            match (id.is_empty(), status.is_empty(), active.is_empty()) {
                (false, false, false) => format!("{id} {status}: {active}"),
                (false, false, true) => format!("{id} {status}"),
                (false, true, false) => format!("{id}: {active}"),
                (true, false, false) => format!("{status}: {active}"),
                (true, false, true) => status.to_string(),
                _ => active.to_string(),
            }
        }
        "TeamCreate" => first_nonempty(input, &["name", "description"]).to_string(),
        "TeamDelete" => first_nonempty(input, &["team_id", "id", "name"]).to_string(),
        "TodoWrite" => summarize_todos(input),
        _ if tool.starts_with("mcp__") => summarize_mcp_tool(tool, input),
        _ => {
            // Generic: compact JSON, trimmed.
            serde_json::to_string(input).unwrap_or_default()
        }
    };
    let s = s.trim();
    if s.is_empty() {
        format!("{tool}()")
    } else {
        s.to_string()
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

fn first_nonempty<'a>(input: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|k| {
            input
                .get(*k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("")
}

fn format_id(action: &str, input: &Value) -> String {
    let id = value_to_string(
        input
            .get("taskId")
            .or_else(|| input.get("id"))
            .or_else(|| input.get("cronId"))
            .or_else(|| input.get("team_id")),
    );
    if id.is_empty() {
        action.to_string()
    } else if id.starts_with('#') {
        format!("{action} {id}")
    } else {
        format!("{action} #{id}")
    }
}

fn value_to_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn summarize_question(input: &Value) -> String {
    input
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|questions| questions.first())
        .and_then(|q| q.get("question"))
        .and_then(Value::as_str)
        .or_else(|| input.get("question").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

fn summarize_todos(input: &Value) -> String {
    let Some(todos) = input.get("todos").and_then(Value::as_array) else {
        return "update todos".to_string();
    };
    let total = todos.len();
    let done = todos
        .iter()
        .filter(|t| {
            t.get("status")
                .and_then(Value::as_str)
                .is_some_and(|s| s == "completed" || s == "done")
        })
        .count();
    format!("{done}/{total} todos complete")
}

fn summarize_mcp_tool(tool: &str, input: &Value) -> String {
    let short = tool.rsplit("__").next().unwrap_or(tool);
    let detail = first_nonempty(
        input,
        &[
            "query",
            "path",
            "file_path",
            "uri",
            "name",
            "title",
            "id",
            "command",
            "url",
        ],
    );
    if detail.is_empty() {
        short.to_string()
    } else {
        format!("{short} {detail}")
    }
}

/// Render a tool result `content` into a short plain-text representation.
pub fn flatten_tool_result(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|b| {
                let kind = b.get("type").and_then(Value::as_str)?;
                if kind == "text" {
                    b.get("text").and_then(Value::as_str).map(str::to_owned)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(v: serde_json::Value) -> String {
        serde_json::to_string(&v).unwrap()
    }

    #[test]
    fn encodes_windows_cwd_like_claude_project_folder() {
        assert_eq!(
            encoded_cwd(r"C:\Users\DamienTouchette\work\leroy"),
            "C--Users-DamienTouchette-work-leroy"
        );
    }

    #[test]
    fn parses_user_string_content() {
        let jsonl = line(json!({
            "type": "user",
            "message": { "role": "user", "content": "hello claude" }
        }));
        let msgs = parse_jsonl(&jsonl);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        let blocks = blocks(&msgs[0].content);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], Block::Text { text } if *text == "hello claude"));
    }

    #[test]
    fn parses_assistant_blocks_with_tool_use() {
        let jsonl = line(json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "Let me run that." },
                    { "type": "tool_use", "name": "Bash", "id": "abc",
                      "input": { "command": "ls -la" } }
                ]
            }
        }));
        let msgs = parse_jsonl(&jsonl);
        assert_eq!(msgs.len(), 1);
        let blocks = blocks(&msgs[0].content);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], Block::Text { text } if *text == "Let me run that."));
        match &blocks[1] {
            Block::ToolUse { name, input, id } => {
                assert_eq!(*name, "Bash");
                assert_eq!(*id, Some("abc"));
                assert_eq!(summarize_tool_input(name, input), "ls -la");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn parses_tool_result_in_user_turn() {
        let jsonl = line(json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [
                    { "type": "tool_result", "tool_use_id": "abc",
                      "content": "main.rs\nCargo.toml" }
                ]
            }
        }));
        let msgs = parse_jsonl(&jsonl);
        assert_eq!(msgs.len(), 1);
        let blocks = blocks(&msgs[0].content);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::ToolResult {
                content,
                is_error,
                tool_use_id,
            } => {
                assert!(!is_error);
                assert_eq!(*tool_use_id, Some("abc"));
                assert_eq!(flatten_tool_result(content), "main.rs\nCargo.toml");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn filters_out_summary_rows() {
        let jsonl = [
            line(json!({"type": "summary", "summary": "old session"})),
            line(json!({"type": "user", "message": {"role": "user", "content": "hi"}})),
        ]
        .join("\n");
        let msgs = parse_jsonl(&jsonl);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn summarize_tool_input_known_tools() {
        assert_eq!(
            summarize_tool_input("Bash", &json!({"command": "echo X"})),
            "echo X"
        );
        assert_eq!(
            summarize_tool_input("Read", &json!({"file_path": "/x/y.rs"})),
            "/x/y.rs"
        );
        assert_eq!(
            summarize_tool_input("Grep", &json!({"pattern": "foo", "path": "/tmp"})),
            "/foo/  in /tmp"
        );
        // Unknown tool falls back to JSON
        let s = summarize_tool_input("Unknown", &json!({"a": 1}));
        assert!(s.contains("\"a\":1"));
    }

    #[test]
    fn accepts_legacy_flat_format() {
        let jsonl = line(json!({
            "role": "user",
            "content": "hello"
        }));
        let msgs = parse_jsonl(&jsonl);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }
}

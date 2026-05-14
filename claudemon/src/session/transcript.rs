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

/// Claude encodes `/foo/bar` as `-foo-bar`.
#[allow(dead_code)]
pub fn encoded_cwd(cwd: &str) -> String {
    cwd.replace(['/', '\\'], "-")
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
    let Some(root) = projects_dir() else {
        return Ok(Transcript::default());
    };
    let dir = root.join(encoded_cwd(cwd));
    if !dir.exists() {
        return Ok(Transcript::default());
    }

    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry.metadata()?.modified().unwrap_or(std::time::UNIX_EPOCH);
        match &newest {
            Some((best, _)) if *best >= mtime => {}
            _ => newest = Some((mtime, path)),
        }
    }

    let Some((_, path)) = newest else {
        return Ok(Transcript::default());
    };

    let text = fs::read_to_string(&path)?;
    let messages = parse_jsonl(&text);

    Ok(Transcript {
        path: Some(path.to_string_lossy().into_owned()),
        messages,
    })
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
    },
    ToolResult {
        content: &'a Value,
        is_error: bool,
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
            }),
            "tool_result" => Some(Block::ToolResult {
                content: v.get("content").unwrap_or(&Value::Null),
                is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
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
        "Bash" => pick("command").to_string(),
        "Read" => pick("file_path").to_string(),
        "Write" | "Edit" | "MultiEdit" => pick("file_path").to_string(),
        "Grep" => {
            let p = pick("pattern");
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(".");
            format!("/{p}/  in {path}")
        }
        "Glob" => pick("pattern").to_string(),
        "WebFetch" => pick("url").to_string(),
        "WebSearch" => pick("query").to_string(),
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
            Block::ToolUse { name, input } => {
                assert_eq!(*name, "Bash");
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
            Block::ToolResult { content, is_error } => {
                assert!(!is_error);
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

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

/// Extra transcript roots learned at spawn time: a session launched with a
/// profile's `CLAUDE_CONFIG_DIR` writes its JSONL under *that* dir's
/// `projects/`, not `~/.claude/projects`. `read_at` confines reads to the known
/// roots, so those dirs have to be registered or profile sessions would lose
/// their transcripts. Grows only from spawn payloads the daemon itself acted on
/// — never from a hook's `transcript_path`, which is the untrusted input the
/// confinement exists to bound.
static EXTRA_ROOTS: once_cell::sync::Lazy<std::sync::Mutex<Vec<PathBuf>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(Vec::new()));

/// Register a spawn's `CLAUDE_CONFIG_DIR` (if it set one) as a transcript root.
pub fn allow_spawn_env(env: &std::collections::HashMap<String, String>) {
    let Some(dir) = env.get("CLAUDE_CONFIG_DIR").map(|d| d.trim()) else {
        return;
    };
    if dir.is_empty() {
        return;
    }
    allow_root(PathBuf::from(dir).join("projects"));
}

/// Add one directory to the set [`read_at`] will read from.
pub fn allow_root(root: impl Into<PathBuf>) {
    let root = root.into();
    if let Ok(mut roots) = EXTRA_ROOTS.lock() {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
}

/// Every directory a transcript may legitimately be read from: the default
/// projects dir plus any profile config dir a spawn declared.
fn allowed_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = EXTRA_ROOTS.lock().map(|r| r.clone()).unwrap_or_default();
    if let Some(dir) = projects_dir() {
        roots.push(dir);
    }
    roots
}

/// The one containment predicate for a `transcript_path`. That field arrives on
/// the unauthenticated hook ingress and is stored verbatim on the session, so
/// every consumer of it has to ask this — a guard bolted onto one reader leaves
/// the others (the conversation tailer, the usage fold) as open a primitive as
/// before.
///
/// Two ways in. A path under a registered root ([`allowed_roots`]) is the
/// ordinary case. The second exists because those roots are learned from spawn
/// payloads and therefore die with the process: `claudemon wrap` run in a
/// terminal that exported `CLAUDE_CONFIG_DIR` never tells the daemon about it,
/// and a claude child orphaned by a daemon restart keeps posting hooks naming a
/// profile the new process never registered. Refusing those is silent — zeroed
/// usage, an empty transcript — so a path is also accepted on *shape*: it must
/// sit inside the `projects/` directory of a directory that is itself a Claude
/// config dir. `/etc/passwd`, `~/.claude/history.jsonl` and a private key are
/// none of those, so the arbitrary-file read stays shut.
pub fn path_is_allowed(path: &std::path::Path) -> bool {
    let roots = allowed_roots();
    // A machine with no home dir and no spawns has nothing to confine to.
    if roots.is_empty() {
        return true;
    }
    roots.iter().any(|root| is_within(root, path)) || under_a_claude_config_dir(path)
}

/// True if `path` is `<dir>/projects/**` and `<dir>` looks like a Claude config
/// dir — named `.claude`, or holding the `settings.json` / `.credentials.json`
/// Claude Code writes into whatever `CLAUDE_CONFIG_DIR` points at. Lexical apart
/// from probing the candidate root, and any `..` in the path disqualifies it
/// outright: a real transcript path never has one, and allowing them would let a
/// forged hook use a genuine profile as a springboard.
fn under_a_claude_config_dir(path: &std::path::Path) -> bool {
    use std::path::Component;
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        if dir.file_name().is_some_and(|name| name == "projects") {
            if let Some(root) = dir.parent() {
                if root.file_name().is_some_and(|name| name == ".claude")
                    || root.join("settings.json").is_file()
                    || root.join(".credentials.json").is_file()
                {
                    return true;
                }
            }
        }
        cursor = dir.parent();
    }
    false
}

/// Claude encodes `/foo/bar` as `-foo-bar` and `C:\foo` as `C--foo`.
///
/// A TRAILING separator is dropped first: a real cwd never carries one, and
/// keeping it would produce a second project folder for the same directory. That
/// strip is not cosmetic here — this is the THIRD copy of an encoder whose two
/// other owners (`claudeProjectDirName` in services/hub/cmd/brain/discovery.go
/// and in apps/desktop/src/main/services/claudeSessionList.ts) both bind
/// themselves to *this* function by name in their comments, while this one had
/// neither the strip nor the refusal they grew. It disagreed with them on 18 of
/// 25 probe inputs. Held to the `projectDirNames` block of
/// contracts/path-containment-cases.json by the test at the bottom of this file.
pub fn encoded_cwd(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    trimmed.replace(['/', '\\', ':'], "-")
}

/// [`encoded_cwd`] plus the one thing the encoding does not give you on its own:
/// the guarantee that the result is a PLAIN COMPONENT. `None` means refuse.
///
/// The encoder rewrites only `/`, `\` and `:`, so `.` and `..` survive verbatim
/// and become real path components: `~/.claude/projects` joined with `".."` is
/// `~/.claude`, one level out of the transcript sandbox, where `history.jsonl`
/// is the user's entire prompt history. `""` is the same shape one level down —
/// it names the projects dir itself — and `"/"` encodes to `"-"`, which is a
/// legal component but names no project.
///
/// The two other copies refuse exactly `""`, `"."` and `".."`. This one is only
/// reachable through [`read_for_cwd`], which currently has no callers, so the
/// escape was latent rather than live — but "unreachable today" is not a
/// property a shared encoder should rely on.
pub fn project_dir_name(cwd: &str) -> Option<String> {
    let name = encoded_cwd(cwd);
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(name)
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

/// Parse the transcript at an exact absolute path (the `transcript_path` from
/// the hook). The authoritative read — no cwd guessing.
///
/// Confined by [`path_is_allowed`], like the cwd-driven lookups below.
/// `transcript_path` arrives from the (unauthenticated) hook ingress and is
/// stored verbatim on the session, so without this an attacker who can post one
/// hook points it at any file on disk and reads it back through
/// `GET /sessions/:id/transcript`. A path the predicate refuses is not read.
pub fn read_at(path: &str) -> Result<Transcript> {
    let path = PathBuf::from(path);
    if !path_is_allowed(&path) {
        anyhow::bail!(
            "refusing to read transcript outside the claude projects root: {}",
            path.display()
        );
    }
    read_transcript_file(path)
}

fn read_for_cwd_and_session(cwd: &str, session_id: Option<&str>) -> Result<Transcript> {
    let Some(root) = projects_dir() else {
        return Ok(Transcript::default());
    };
    // Refuse a cwd that does not encode to one plain component, rather than
    // joining it: see project_dir_name.
    let Some(name) = project_dir_name(cwd) else {
        return Ok(Transcript::default());
    };
    let dir = root.join(name);

    if let Some(session_id) = session_id {
        // Exact match only. We must NOT fall back to "newest jsonl in cwd" here:
        // when several sessions share a cwd that returns a *different* session's
        // transcript. Showing nothing beats showing the wrong conversation.
        let path = dir.join(format!("{session_id}.jsonl"));
        // Defense-in-depth: the API boundary already rejects traversal-shaped
        // ids, but never read a path the id pushed outside the projects root.
        if is_within(&root, &path) && path.exists() {
            return read_transcript_file(path);
        }
        if let Some(path) = find_session_file(&root, session_id)? {
            return read_transcript_file(path);
        }
        return Ok(Transcript::default());
    }

    // No session id known: best-effort newest jsonl in the cwd dir.
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
    // A machine that has never run Claude Code has no projects dir at all —
    // that's "no transcript", not an error (the API contract is an empty
    // transcript for sessions with nothing on disk).
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path().join(&filename);
        // Defense-in-depth: a session id carrying `..`/separators must not let
        // `join` climb out of the project subdirectory it's scanning.
        if is_within(root, &path) && path.exists() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// True if `path`, resolved lexically (so `..` segments are accounted for),
/// stays under `root`. Purely lexical — no `canonicalize`, so it doesn't depend
/// on the file existing and can't be fooled into touching disk while checking.
fn is_within(root: &std::path::Path, path: &std::path::Path) -> bool {
    use std::path::Component;
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let mut depth: i32 = 0;
    for comp in rel.components() {
        match comp {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            // An absolute/root/prefix segment in the tail means it escaped.
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
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
    fn read_at_refuses_a_path_outside_the_projects_root() {
        // `transcript_path` comes off the unauthenticated hook ingress, so a
        // forged hook must not turn GET /sessions/:id/transcript into an
        // arbitrary file read.
        if projects_dir().is_none() {
            return; // no home dir to confine to; read_at is unrestricted by design
        }
        let err = read_at("/etc/passwd").unwrap_err();
        assert!(
            err.to_string().contains("refusing to read transcript"),
            "got: {err}"
        );
        // Traversal back out of the root is caught lexically too.
        let escape = projects_dir()
            .unwrap()
            .join("../../.ssh/id_rsa")
            .to_string_lossy()
            .into_owned();
        assert!(read_at(&escape).is_err());
    }

    #[test]
    fn a_spawns_config_dir_becomes_a_transcript_root() {
        // Profile spawns (`CLAUDE_CONFIG_DIR`) write their JSONL outside
        // ~/.claude/projects; the confinement must follow them there.
        let dir = std::env::temp_dir().join(format!("wks-cfgdir-{}", std::process::id()));
        let root = dir.join("projects");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("sess.jsonl");
        fs::write(&file, line(json!({ "type": "summary" })) + "\n").unwrap();
        let path = file.to_string_lossy().into_owned();

        assert!(read_at(&path).is_err(), "unknown root is refused");
        allow_spawn_env(&std::collections::HashMap::from([(
            "CLAUDE_CONFIG_DIR".to_string(),
            dir.to_string_lossy().into_owned(),
        )]));
        assert!(read_at(&path).is_ok(), "registered root is readable");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_profile_transcript_is_readable_without_a_spawn_having_registered_it() {
        // `claudemon wrap` in a terminal that exported CLAUDE_CONFIG_DIR never
        // tells the daemon about that dir, and a daemon restart forgets every
        // root it did learn. Both used to zero the session's usage silently.
        // The dir is claude-shaped (it holds the settings.json Claude Code
        // writes), so it is accepted on shape without any allow_root call.
        let dir = std::env::temp_dir().join(format!(
            "wks-profile-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = dir.join("projects").join("-home-me-work");
        fs::create_dir_all(&root).unwrap();
        fs::write(dir.join("settings.json"), "{}").unwrap();
        let file = root.join("sess.jsonl");
        fs::write(
            &file,
            line(json!({
                "type": "assistant",
                "message": { "role": "assistant", "content": "hi" }
            })) + "\n",
        )
        .unwrap();

        let tx = read_at(&file.to_string_lossy()).expect("profile transcript reads");
        assert_eq!(tx.messages.len(), 1);

        // The shape rule is not a way back out: a real profile must not become a
        // springboard to anything above its projects dir.
        let escape = dir
            .join("projects")
            .join("../../../../etc/passwd")
            .to_string_lossy()
            .into_owned();
        assert!(read_at(&escape).is_err(), "traversal out of a profile root");
        // And a claude-shaped dir does not expose the files beside `projects/`.
        assert!(
            !under_a_claude_config_dir(&dir.join("settings.json")),
            "only paths under projects/ are transcripts"
        );

        let _ = fs::remove_dir_all(&dir);
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

    // ------------------------------------------------------------------ //
    // summarize_tool_input — characterization per tool family             //
    // ------------------------------------------------------------------ //

    // --- File-path tools ---

    #[test]
    fn summarize_read_returns_file_path() {
        assert_eq!(
            summarize_tool_input("Read", &json!({"file_path": "/home/user/main.rs"})),
            "/home/user/main.rs"
        );
    }

    #[test]
    fn summarize_write_returns_file_path() {
        assert_eq!(
            summarize_tool_input(
                "Write",
                &json!({"file_path": "/tmp/out.txt", "content": "hi"})
            ),
            "/tmp/out.txt"
        );
    }

    #[test]
    fn summarize_edit_returns_file_path() {
        assert_eq!(
            summarize_tool_input("Edit", &json!({"file_path": "/src/lib.rs"})),
            "/src/lib.rs"
        );
    }

    #[test]
    fn summarize_multi_edit_returns_file_path() {
        assert_eq!(
            summarize_tool_input("MultiEdit", &json!({"file_path": "/x.rs"})),
            "/x.rs"
        );
    }

    #[test]
    fn summarize_file_path_tool_empty_path_returns_tool_unit() {
        // When file_path is "" the trimmed result is empty, so the fallback fires.
        assert_eq!(
            summarize_tool_input("Read", &json!({"file_path": ""})),
            "Read()"
        );
    }

    // --- Search tools ---

    #[test]
    fn summarize_grep_formats_pattern_and_path() {
        assert_eq!(
            summarize_tool_input("Grep", &json!({"pattern": "TODO", "path": "/repo"})),
            "/TODO/  in /repo"
        );
    }

    #[test]
    fn summarize_grep_uses_dot_when_path_missing() {
        assert_eq!(
            summarize_tool_input("Grep", &json!({"pattern": "foo"})),
            "/foo/  in ."
        );
    }

    #[test]
    fn summarize_glob_returns_pattern() {
        assert_eq!(
            summarize_tool_input("Glob", &json!({"pattern": "**/*.rs"})),
            "**/*.rs"
        );
    }

    #[test]
    fn summarize_web_search_returns_query() {
        assert_eq!(
            summarize_tool_input("WebSearch", &json!({"query": "rust async traits"})),
            "rust async traits"
        );
    }

    #[test]
    fn summarize_web_fetch_returns_url() {
        assert_eq!(
            summarize_tool_input("WebFetch", &json!({"url": "https://example.com"})),
            "https://example.com"
        );
    }

    // --- Command / Bash tools ---

    #[test]
    fn summarize_bash_returns_command() {
        assert_eq!(
            summarize_tool_input("Bash", &json!({"command": "cargo build --release"})),
            "cargo build --release"
        );
    }

    #[test]
    fn summarize_monitor_returns_command() {
        assert_eq!(
            summarize_tool_input("Monitor", &json!({"command": "tail -f /var/log/syslog"})),
            "tail -f /var/log/syslog"
        );
    }

    #[test]
    fn summarize_power_shell_returns_command() {
        assert_eq!(
            summarize_tool_input("PowerShell", &json!({"command": "Get-Process"})),
            "Get-Process"
        );
    }

    // --- Agent / Task tools ---

    #[test]
    fn summarize_agent_prefers_description_over_prompt() {
        assert_eq!(
            summarize_tool_input(
                "Agent",
                &json!({"description": "Fix the bug", "prompt": "Please fix it"})
            ),
            "Fix the bug"
        );
    }

    #[test]
    fn summarize_agent_falls_back_to_prompt_when_description_missing() {
        assert_eq!(
            summarize_tool_input("Agent", &json!({"prompt": "Do the thing"})),
            "Do the thing"
        );
    }

    #[test]
    fn summarize_agent_falls_back_to_task_when_both_absent() {
        assert_eq!(
            summarize_tool_input("Agent", &json!({"task": "Run tests"})),
            "Run tests"
        );
    }

    #[test]
    fn summarize_agent_all_empty_returns_unit() {
        assert_eq!(summarize_tool_input("Agent", &json!({})), "Agent()");
    }

    #[test]
    fn summarize_task_create_prefers_active_form() {
        assert_eq!(
            summarize_tool_input(
                "TaskCreate",
                &json!({"activeForm": "Review PR", "description": "Long desc"})
            ),
            "Review PR"
        );
    }

    #[test]
    fn summarize_task_create_falls_back_to_first_line_of_description() {
        assert_eq!(
            summarize_tool_input(
                "TaskCreate",
                &json!({"description": "Write tests\nfor everything"})
            ),
            "Write tests"
        );
    }

    #[test]
    fn summarize_task_create_empty_yields_unit() {
        assert_eq!(
            summarize_tool_input("TaskCreate", &json!({})),
            "TaskCreate()"
        );
    }

    #[test]
    fn summarize_task_update_all_three_fields() {
        assert_eq!(
            summarize_tool_input(
                "TaskUpdate",
                &json!({"taskId": 42, "status": "done", "activeForm": "Ship it"})
            ),
            "#42 done: Ship it"
        );
    }

    #[test]
    fn summarize_task_update_id_and_status_only() {
        assert_eq!(
            summarize_tool_input("TaskUpdate", &json!({"taskId": 7, "status": "in_progress"})),
            "#7 in_progress"
        );
    }

    #[test]
    fn summarize_task_update_id_and_active_form_only() {
        assert_eq!(
            summarize_tool_input("TaskUpdate", &json!({"taskId": 3, "activeForm": "Draft"})),
            "#3: Draft"
        );
    }

    #[test]
    fn summarize_task_update_status_and_active_form_no_id() {
        assert_eq!(
            summarize_tool_input(
                "TaskUpdate",
                &json!({"status": "blocked", "activeForm": "Waiting on review"})
            ),
            "blocked: Waiting on review"
        );
    }

    #[test]
    fn summarize_task_update_status_only() {
        assert_eq!(
            summarize_tool_input("TaskUpdate", &json!({"status": "cancelled"})),
            "cancelled"
        );
    }

    #[test]
    fn summarize_task_update_active_form_only() {
        assert_eq!(
            summarize_tool_input("TaskUpdate", &json!({"activeForm": "Just a note"})),
            "Just a note"
        );
    }

    #[test]
    fn summarize_task_list_is_constant() {
        assert_eq!(summarize_tool_input("TaskList", &json!({})), "list tasks");
    }

    #[test]
    fn summarize_task_get_with_id() {
        assert_eq!(
            summarize_tool_input("TaskGet", &json!({"taskId": 99})),
            "get task #99"
        );
    }

    #[test]
    fn summarize_task_stop_with_id() {
        assert_eq!(
            summarize_tool_input("TaskStop", &json!({"id": 5})),
            "stop task #5"
        );
    }

    #[test]
    fn summarize_cron_create_returns_name() {
        assert_eq!(
            summarize_tool_input("CronCreate", &json!({"name": "daily-backup"})),
            "daily-backup"
        );
    }

    #[test]
    fn summarize_cron_delete_formats_id() {
        assert_eq!(
            summarize_tool_input("CronDelete", &json!({"cronId": "abc-123"})),
            "cancel #abc-123"
        );
    }

    #[test]
    fn summarize_cron_list_is_constant() {
        assert_eq!(
            summarize_tool_input("CronList", &json!({})),
            "list scheduled tasks"
        );
    }

    // --- mcp__ prefix family ---

    #[test]
    fn summarize_mcp_tool_uses_last_segment_plus_first_detail_field() {
        // mcp__Neon__run_sql → short name "run_sql", detail from "query"
        assert_eq!(
            summarize_tool_input("mcp__Neon__run_sql", &json!({"query": "SELECT 1"})),
            "run_sql SELECT 1"
        );
    }

    #[test]
    fn summarize_mcp_tool_falls_back_to_name_field_when_query_missing() {
        assert_eq!(
            summarize_tool_input("mcp__SomeSrv__do_thing", &json!({"name": "my-resource"})),
            "do_thing my-resource"
        );
    }

    #[test]
    fn summarize_mcp_tool_no_detail_returns_short_name_only() {
        assert_eq!(
            summarize_tool_input("mcp__Neon__list_projects", &json!({})),
            "list_projects"
        );
    }

    #[test]
    fn summarize_mcp_tool_uses_uri_field() {
        assert_eq!(
            summarize_tool_input(
                "mcp__FS__read_resource",
                &json!({"uri": "file:///etc/hosts"})
            ),
            "read_resource file:///etc/hosts"
        );
    }

    #[test]
    fn summarize_mcp_tool_uses_path_field() {
        assert_eq!(
            summarize_tool_input("mcp__FS__stat", &json!({"path": "/var/log"})),
            "stat /var/log"
        );
    }

    #[test]
    fn summarize_mcp_tool_uses_url_field() {
        assert_eq!(
            summarize_tool_input(
                "mcp__Browser__navigate",
                &json!({"url": "https://example.com"})
            ),
            "navigate https://example.com"
        );
    }

    #[test]
    fn summarize_mcp_tool_uses_command_field() {
        assert_eq!(
            summarize_tool_input("mcp__Shell__exec", &json!({"command": "make test"})),
            "exec make test"
        );
    }

    // --- Unknown / fallback ---

    #[test]
    fn summarize_unknown_tool_compact_json() {
        let result = summarize_tool_input("UnknownTool", &json!({"key": "val"}));
        // Should contain the JSON key-value pair
        assert!(
            result.contains("\"key\""),
            "expected key in output: {result}"
        );
        assert!(
            result.contains("\"val\""),
            "expected val in output: {result}"
        );
    }

    #[test]
    fn summarize_unknown_tool_null_input_returns_unit() {
        // serde_json::to_string(null) = "null", which is non-empty, so no unit fallback
        let result = summarize_tool_input("MyTool", &json!(null));
        assert_eq!(result, "null");
    }

    #[test]
    fn summarize_unknown_tool_empty_object_returns_unit() {
        // serde_json::to_string({}) = "{}", non-empty; tool() unit only fires when trimmed is ""
        let result = summarize_tool_input("MyTool", &json!({}));
        assert_eq!(result, "{}");
    }

    // --- AskUserQuestion ---

    #[test]
    fn summarize_ask_user_question_returns_first_question_text() {
        assert_eq!(
            summarize_tool_input(
                "AskUserQuestion",
                &json!({
                    "questions": [{"question": "Which strategy?", "options": []},
                                  {"question": "Second?", "options": []}]
                })
            ),
            "Which strategy?"
        );
    }

    #[test]
    fn summarize_ask_user_question_flat_fallback() {
        // When there's no questions array, fall back to top-level "question" field.
        assert_eq!(
            summarize_tool_input("AskUserQuestion", &json!({"question": "Are you sure?"})),
            "Are you sure?"
        );
    }

    #[test]
    fn summarize_ask_user_question_empty_input_returns_unit() {
        assert_eq!(
            summarize_tool_input("AskUserQuestion", &json!({})),
            "AskUserQuestion()"
        );
    }

    // --- TodoWrite ---

    #[test]
    fn summarize_todo_write_counts_completed_todos() {
        let input = json!({
            "todos": [
                {"status": "completed", "text": "Write tests"},
                {"status": "pending", "text": "Deploy"},
                {"status": "done", "text": "Review PR"},
            ]
        });
        assert_eq!(
            summarize_tool_input("TodoWrite", &input),
            "2/3 todos complete"
        );
    }

    #[test]
    fn summarize_todo_write_no_todos_key_returns_update_string() {
        assert_eq!(
            summarize_tool_input("TodoWrite", &json!({})),
            "update todos"
        );
    }

    #[test]
    fn summarize_todo_write_empty_list() {
        assert_eq!(
            summarize_tool_input("TodoWrite", &json!({"todos": []})),
            "0/0 todos complete"
        );
    }

    // --- Misc named tools ---

    #[test]
    fn summarize_tool_search_with_query() {
        assert_eq!(
            summarize_tool_input("ToolSearch", &json!({"query": "select:Read"})),
            "search select:Read"
        );
    }

    #[test]
    fn summarize_tool_search_without_query_returns_constant() {
        assert_eq!(
            summarize_tool_input("ToolSearch", &json!({"query": ""})),
            "search tools"
        );
    }

    #[test]
    fn summarize_tool_search_no_query_field_returns_constant() {
        assert_eq!(
            summarize_tool_input("ToolSearch", &json!({})),
            "search tools"
        );
    }

    #[test]
    fn summarize_send_message_prefers_message_field() {
        assert_eq!(
            summarize_tool_input(
                "SendMessage",
                &json!({"message": "Hello agent", "recipient": "AgentB"})
            ),
            "Hello agent"
        );
    }

    #[test]
    fn summarize_push_notification_prefers_message_field() {
        assert_eq!(
            summarize_tool_input("PushNotification", &json!({"message": "Build done"})),
            "Build done"
        );
    }

    #[test]
    fn summarize_enter_plan_mode_is_constant() {
        assert_eq!(
            summarize_tool_input("EnterPlanMode", &json!({})),
            "enter plan mode"
        );
    }

    #[test]
    fn summarize_exit_plan_mode_prefers_plan_field() {
        assert_eq!(
            summarize_tool_input("ExitPlanMode", &json!({"plan": "Refactor auth module"})),
            "Refactor auth module"
        );
    }

    #[test]
    fn summarize_enter_worktree_returns_path() {
        assert_eq!(
            summarize_tool_input("EnterWorktree", &json!({"path": "/worktrees/feat"})),
            "/worktrees/feat"
        );
    }

    #[test]
    fn summarize_exit_worktree_is_constant() {
        assert_eq!(
            summarize_tool_input("ExitWorktree", &json!({})),
            "exit worktree"
        );
    }

    #[test]
    fn summarize_remote_trigger_prefers_name() {
        assert_eq!(
            summarize_tool_input("RemoteTrigger", &json!({"name": "deploy", "action": "run"})),
            "deploy"
        );
    }

    #[test]
    fn summarize_team_create_returns_name() {
        assert_eq!(
            summarize_tool_input("TeamCreate", &json!({"name": "alpha-team"})),
            "alpha-team"
        );
    }

    #[test]
    fn summarize_team_delete_returns_team_id() {
        assert_eq!(
            summarize_tool_input("TeamDelete", &json!({"team_id": "tid-99"})),
            "tid-99"
        );
    }

    #[test]
    fn summarize_notebook_edit_returns_notebook_path() {
        assert_eq!(
            summarize_tool_input(
                "NotebookEdit",
                &json!({"notebook_path": "/nb/analysis.ipynb"})
            ),
            "/nb/analysis.ipynb"
        );
    }

    #[test]
    fn summarize_read_mcp_resource_tool_returns_uri() {
        assert_eq!(
            summarize_tool_input(
                "ReadMcpResourceTool",
                &json!({"uri": "resource://my/thing"})
            ),
            "resource://my/thing"
        );
    }

    #[test]
    fn summarize_list_mcp_resources_tool_returns_server() {
        assert_eq!(
            summarize_tool_input("ListMcpResourcesTool", &json!({"server": "neon"})),
            "neon"
        );
    }

    #[test]
    fn summarize_skill_returns_skill_field() {
        assert_eq!(
            summarize_tool_input("Skill", &json!({"skill": "run"})),
            "run"
        );
    }

    #[test]
    fn summarize_lsp_prefers_query_then_symbol() {
        assert_eq!(
            summarize_tool_input(
                "LSP",
                &json!({"query": "SessionState", "symbol": "something"})
            ),
            "SessionState"
        );
        assert_eq!(
            summarize_tool_input("LSP", &json!({"symbol": "apply"})),
            "apply"
        );
    }

    #[test]
    fn summarize_task_output_with_id() {
        assert_eq!(
            summarize_tool_input("TaskOutput", &json!({"taskId": 12})),
            "task output #12"
        );
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

    /// The `projectDirNames` block of contracts/path-containment-cases.json.
    ///
    /// `encoded_cwd` is the THIRD copy of this encoder, and the other two bind
    /// themselves to it by name in their comments — discovery.go's says "Must
    /// match claudemon's encoded_cwd", claudeSessionList.ts's names this exact
    /// file. Nothing held it to either: this copy had neither the trailing-
    /// separator strip nor the ''/'.'/'..'  refusal they grew, and disagreed
    /// with them on 18 of 25 spellings — including all six the fixture refuses.
    /// The fixture's own owner note listed two owners; there were always three.
    #[test]
    fn project_dir_names_match_the_shared_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/path-containment-cases.json");
        let Ok(raw) = fs::read_to_string(&path) else {
            eprintln!("contract fixture not reachable from this checkout; skipping");
            return;
        };
        let fx: serde_json::Value = serde_json::from_str(&raw).expect("fixture is valid JSON");
        let cases = fx["projectDirNames"]["cases"]
            .as_array()
            .expect("projectDirNames.cases");
        assert!(
            !cases.is_empty(),
            "the projectDirNames block is empty — this loader holds nothing to anything"
        );
        let mut refusals = 0;
        for c in cases {
            let cwd = c["cwd"].as_str().expect("cwd");
            let why = c["why"].as_str().unwrap_or("");
            match c["expect"].as_str() {
                Some(want) => assert_eq!(
                    project_dir_name(cwd).as_deref(),
                    Some(want),
                    "project_dir_name({cwd:?})\n  why: {why}"
                ),
                None => {
                    refusals += 1;
                    assert_eq!(
                        project_dir_name(cwd),
                        None,
                        "project_dir_name({cwd:?}) must REFUSE\n  why: {why}"
                    );
                }
            }
        }
        assert!(
            refusals > 0,
            "no refusal cases ran — the escape half of this block is not being exercised"
        );
    }

    /// The WIRING, as far as it can be asserted without repointing $HOME (which
    /// `projects_dir` reads through `BaseDirs`, and which these tests share a
    /// process for): `read_for_cwd_and_session` consults `project_dir_name` and
    /// returns an empty transcript rather than joining a refused name onto the
    /// projects root.
    #[test]
    fn read_for_cwd_returns_nothing_for_a_refused_cwd() {
        for cwd in ["..", ".", "", "/"] {
            let t = read_for_cwd(cwd).expect("must not error");
            if project_dir_name(cwd).is_none() {
                assert!(
                    t.messages.is_empty(),
                    "read_for_cwd({cwd:?}) returned messages for a cwd the encoder refuses — the refusal is not wired into the join"
                );
            }
        }
    }
}

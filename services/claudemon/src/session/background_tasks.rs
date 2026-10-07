//! Per-session BACKGROUND TASK LIST for stream-transport Claude sessions.
//!
//! The stream driver already folds `system/background_tasks_changed` into a
//! bare count (`SessionState::background_tasks`) and into the busy/idle hold
//! for agent-type tasks. This module is the additive enrichment beside that:
//! what each task IS, its status and progress, where its output lives, and —
//! on Linux, when it can be proven — which process runs it. Nothing here feeds
//! the mode state machine; the busy derivation in `claude_stream.rs` stays the
//! single owner of that.
//!
//! Frame shapes (captured from CLI 2.1.286, fixtures under
//! `providers/testdata/claude-stream-bg-tasks*.jsonl`):
//!
//! * `system/background_tasks_changed` — the full live set
//!   `tasks: [{task_id, task_type, description, ambient?}]`. Arrives BEFORE the
//!   matching `task_started`.
//! * `system/task_started` — `task_id, tool_use_id, description, task_type,
//!   is_backgrounded, subagent_type?, spawn_depth?, prompt?, workflow_name?,
//!   owned_by_subagent?, ambient?`.
//! * `system/task_progress` — `task_id, description, usage{total_tokens,
//!   tool_uses, duration_ms}, last_tool_name?, summary?` (agent tasks).
//! * `system/task_updated` — `task_id, patch{status?, end_time?, description?,
//!   error?, is_backgrounded?}`.
//! * `system/task_notification` — `task_id, status, output_file, summary,
//!   usage?, reason?`. The terminal frame for a task.
//! * The background Bash `tool_result` user frame carries
//!   `tool_use_result.backgroundTaskId` and names the output file in its text
//!   (`Output is being written to: <path>`) — the only place the path appears
//!   while the task is still running.
//!
//! The task status vocabulary is the CLI's: `pending`, `running`, `completed`,
//! `failed`, `killed`, `stopped`. Unknown values pass through verbatim.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Hard cap on rows kept per session. Running tasks are never evicted for a
/// finished one; finished tasks go oldest-first.
pub const MAX_TASKS: usize = 40;
/// Finished tasks older than this are dropped on the next change.
pub const FINISHED_RETENTION_MS: i64 = 2 * 60 * 60 * 1000;
/// Default and maximum bytes one output read returns.
pub const DEFAULT_READ_BYTES: u64 = 64 * 1024;
pub const MAX_READ_BYTES: u64 = 512 * 1024;
/// Bound on wire-visible free text (descriptions are whole shell commands).
const MAX_TEXT: usize = 2_000;

/// Token/tool/time counters the CLI reports for agent tasks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUsage {
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub tool_uses: u64,
    #[serde(default)]
    pub duration_ms: u64,
}

/// One background task as clients see it on the session snapshot
/// (`background_task_list`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTask {
    pub id: String,
    /// `local_bash`, `local_agent`, `in_process_teammate`, `remote_agent`,
    /// `local_workflow`, or whatever a newer CLI names.
    pub task_type: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Unix ms when the daemon first saw the task.
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    /// The CLI's one-line result/status summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TaskUsage>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ambient: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owned_by_subagent: bool,
    /// For `local_agent`: the id of the matching row in `subagents`, whose
    /// transcript the existing child view already renders. The task id IS
    /// the subagent id (`subagents/agent-<task_id>.jsonl`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_id: Option<String>,
    /// Whether `GET /sessions/:id/tasks/:task_id/output` can serve a log.
    #[serde(default)]
    pub has_output: bool,
    /// The task's root process, only when proven (see [`discover_process`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// The CLI-named output file. Never on the wire and never taken from a
    /// client: reads resolve a task id to this path daemon-side.
    #[serde(skip)]
    pub output_file: Option<PathBuf>,
    /// Proof material for `pid`, re-checked before every use.
    #[serde(skip)]
    pub process: Option<VerifiedProcess>,
}

impl BackgroundTask {
    fn new(id: &str, task_type: &str, now: i64) -> Self {
        Self {
            id: id.to_string(),
            task_type: task_type.to_string(),
            status: "running".into(),
            description: None,
            started_at: now,
            ended_at: None,
            tool_use_id: None,
            subagent_type: None,
            workflow_name: None,
            summary: None,
            last_tool_name: None,
            error: None,
            usage: None,
            ambient: false,
            owned_by_subagent: false,
            subagent_id: (task_type == "local_agent").then(|| id.to_string()),
            has_output: false,
            pid: None,
            output_file: None,
            process: None,
        }
    }

    pub fn is_running(&self) -> bool {
        !is_terminal(&self.status)
    }
}

pub fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "killed" | "stopped")
}

/// One fact parsed off a stream frame.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskEvent {
    /// The live set: `(id, type, description, ambient)`.
    LiveSet(Vec<(String, String, Option<String>, bool)>),
    Started {
        id: String,
        task_type: String,
        description: Option<String>,
        tool_use_id: Option<String>,
        subagent_type: Option<String>,
        workflow_name: Option<String>,
        backgrounded: Option<bool>,
        owned_by_subagent: bool,
        ambient: bool,
    },
    Progress {
        id: String,
        description: Option<String>,
        usage: Option<TaskUsage>,
        last_tool_name: Option<String>,
        summary: Option<String>,
    },
    Updated {
        id: String,
        status: Option<String>,
        ended_at: Option<i64>,
        description: Option<String>,
        error: Option<String>,
    },
    Notification {
        id: String,
        status: Option<String>,
        summary: Option<String>,
        usage: Option<TaskUsage>,
        reason: Option<String>,
        output_file: Option<PathBuf>,
    },
    /// The output file of a task, learned from its launch `tool_result`.
    Output { id: String, output_file: PathBuf },
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| bounded(s, MAX_TEXT))
}

fn bounded(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Task ids go into a filename (`<id>.output`) and a URL segment; the CLI mints
/// short alphanumerics, so anything else is refused outright.
pub fn valid_task_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn usage(v: &Value) -> Option<TaskUsage> {
    let u = v.get("usage")?.as_object()?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(TaskUsage {
        total_tokens: n("total_tokens"),
        tool_uses: n("tool_uses"),
        duration_ms: n("duration_ms"),
    })
}

/// Accept a CLI-named output path only in the CLI's own layout:
/// `…/<claude session id>/tasks/<task id>.output`, absolute. Anything else is
/// not this task's log and is ignored rather than trusted.
pub fn confined_output_path(raw: &str, task_id: &str, claude_session: &str) -> Option<PathBuf> {
    let path = PathBuf::from(raw.trim());
    if !path.is_absolute() || !valid_task_id(task_id) || claude_session.is_empty() {
        return None;
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let tasks = path.parent()?;
    let session = tasks.parent()?;
    (name == format!("{task_id}.output")
        && tasks.file_name()?.to_str()? == "tasks"
        && session.file_name()?.to_str()? == claude_session)
        .then_some(path)
}

/// Pull the output path out of a background-launch tool_result's text.
fn launch_output_path(frame: &Value) -> Option<String> {
    let content = frame.pointer("/message/content")?.as_array()?;
    for block in content {
        let body = match block.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        for marker in ["Output is being written to: ", "output_file: "] {
            if let Some(rest) = body.split(marker).nth(1) {
                // The path is followed by a sentence period: "….output. You…".
                let token: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
                let path = token.trim_end_matches(['.', ',', ')']);
                if path.ends_with(".output") {
                    return Some(path.to_string());
                }
            }
        }
    }
    None
}

/// Parse one stdout frame into task facts. Pure; unrelated frames yield none.
pub fn parse_frame(v: &Value) -> Vec<TaskEvent> {
    let session = v.get("session_id").and_then(Value::as_str).unwrap_or("");
    match v.get("type").and_then(Value::as_str) {
        Some("system") => {}
        Some("user") => {
            // A background Bash launch result: names the task and its file.
            let Some(id) = v
                .pointer("/tool_use_result/backgroundTaskId")
                .and_then(Value::as_str)
                .filter(|id| valid_task_id(id))
            else {
                return Vec::new();
            };
            return launch_output_path(v)
                .and_then(|raw| confined_output_path(&raw, id, session))
                .map(|output_file| {
                    vec![TaskEvent::Output {
                        id: id.to_string(),
                        output_file,
                    }]
                })
                .unwrap_or_default();
        }
        _ => return Vec::new(),
    }
    let id = || {
        v.get("task_id")
            .and_then(Value::as_str)
            .filter(|id| valid_task_id(id))
            .map(str::to_string)
    };
    match v.get("subtype").and_then(Value::as_str).unwrap_or("") {
        "background_tasks_changed" => {
            let live = v
                .get("tasks")
                .and_then(Value::as_array)
                .map(|tasks| {
                    tasks
                        .iter()
                        .filter_map(|t| {
                            let id = t
                                .get("task_id")
                                .and_then(Value::as_str)
                                .filter(|id| valid_task_id(id))?;
                            Some((
                                id.to_string(),
                                text(t, "task_type").unwrap_or_else(|| "unknown".into()),
                                text(t, "description"),
                                t.get("ambient").and_then(Value::as_bool).unwrap_or(false),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            vec![TaskEvent::LiveSet(live)]
        }
        "task_started" => id()
            .map(|id| {
                vec![TaskEvent::Started {
                    id,
                    task_type: text(v, "task_type").unwrap_or_else(|| "unknown".into()),
                    description: text(v, "description"),
                    tool_use_id: text(v, "tool_use_id"),
                    subagent_type: text(v, "subagent_type"),
                    workflow_name: text(v, "workflow_name"),
                    backgrounded: v.get("is_backgrounded").and_then(Value::as_bool),
                    owned_by_subagent: v
                        .get("owned_by_subagent")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    ambient: v.get("ambient").and_then(Value::as_bool).unwrap_or(false),
                }]
            })
            .unwrap_or_default(),
        "task_progress" => id()
            .map(|id| {
                vec![TaskEvent::Progress {
                    id,
                    description: text(v, "description"),
                    usage: usage(v),
                    last_tool_name: text(v, "last_tool_name"),
                    summary: text(v, "summary"),
                }]
            })
            .unwrap_or_default(),
        "task_updated" => id()
            .map(|id| {
                let patch = v.get("patch").unwrap_or(&Value::Null);
                vec![TaskEvent::Updated {
                    id,
                    status: text(patch, "status"),
                    ended_at: patch.get("end_time").and_then(Value::as_i64),
                    description: text(patch, "description"),
                    error: text(patch, "error"),
                }]
            })
            .unwrap_or_default(),
        "task_notification" => id()
            .map(|id| {
                let output_file = v
                    .get("output_file")
                    .and_then(Value::as_str)
                    .and_then(|raw| confined_output_path(raw, &id, session));
                vec![TaskEvent::Notification {
                    status: text(v, "status"),
                    summary: text(v, "summary"),
                    usage: usage(v),
                    reason: text(v, "reason"),
                    output_file,
                    id,
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Fold one event into a session's task list. Returns whether anything a
/// client can see changed. `now` is unix ms.
pub fn apply(tasks: &mut Vec<BackgroundTask>, event: TaskEvent, now: i64) -> bool {
    let changed = match event {
        TaskEvent::LiveSet(live) => {
            let mut changed = false;
            for (id, task_type, description, ambient) in live {
                match tasks.iter_mut().find(|t| t.id == id) {
                    Some(task) => {
                        if task.description.is_none() && description.is_some() {
                            task.description = description;
                            changed = true;
                        }
                    }
                    None => {
                        let mut task = BackgroundTask::new(&id, &task_type, now);
                        task.description = description;
                        task.ambient = ambient;
                        tasks.push(task);
                        changed = true;
                    }
                }
            }
            changed
        }
        TaskEvent::Started {
            id,
            task_type,
            description,
            tool_use_id,
            subagent_type,
            workflow_name,
            backgrounded,
            owned_by_subagent,
            ambient,
        } => {
            let known = tasks.iter().any(|t| t.id == id);
            // A foreground task is not background work; it only joins the list
            // once the live set names it (a later Ctrl+B-style backgrounding).
            if !known && backgrounded == Some(false) {
                return false;
            }
            if !known {
                tasks.push(BackgroundTask::new(&id, &task_type, now));
            }
            let task = tasks.iter_mut().find(|t| t.id == id).expect("just ensured");
            let before = task.clone();
            if task.task_type == "unknown" {
                task.task_type = task_type;
                task.subagent_id = (task.task_type == "local_agent").then(|| task.id.clone());
            }
            task.description = description.or(task.description.take());
            task.tool_use_id = tool_use_id.or(task.tool_use_id.take());
            task.subagent_type = subagent_type.or(task.subagent_type.take());
            task.workflow_name = workflow_name.or(task.workflow_name.take());
            task.owned_by_subagent |= owned_by_subagent;
            task.ambient |= ambient;
            *task != before
        }
        TaskEvent::Progress {
            id,
            description,
            usage,
            last_tool_name,
            summary,
        } => {
            let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
                return false;
            };
            let before = task.clone();
            // An agent task's progress `description` is its latest activity
            // ("Running Echo the letter a"), not its name: keep it as summary.
            if summary.is_some() {
                task.summary = summary;
            } else if description.is_some() && description != task.description {
                task.summary = description;
            }
            task.usage = usage.or(task.usage);
            task.last_tool_name = last_tool_name.or(task.last_tool_name.take());
            *task != before
        }
        TaskEvent::Updated {
            id,
            status,
            ended_at,
            description,
            error,
        } => {
            let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
                return false;
            };
            let before = task.clone();
            if let Some(status) = status {
                task.status = status;
            }
            if let Some(end) = ended_at {
                task.ended_at = Some(end);
            } else if is_terminal(&task.status) && task.ended_at.is_none() {
                task.ended_at = Some(now);
            }
            task.description = description.or(task.description.take());
            task.error = error.or(task.error.take());
            *task != before
        }
        TaskEvent::Notification {
            id,
            status,
            summary,
            usage,
            reason,
            output_file,
        } => {
            let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
                return false;
            };
            let before = task.clone();
            if let Some(status) = status {
                task.status = status;
            }
            if is_terminal(&task.status) && task.ended_at.is_none() {
                task.ended_at = Some(now);
            }
            task.summary = summary.or(task.summary.take());
            task.usage = usage.or(task.usage);
            task.error = reason.or(task.error.take());
            if let Some(path) = output_file {
                set_output(task, path);
            }
            *task != before
        }
        TaskEvent::Output { id, output_file } => {
            let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
                return false;
            };
            let before = task.clone();
            set_output(task, output_file);
            *task != before || before.output_file != task.output_file
        }
    };
    if changed {
        share_tasks_dir(tasks);
    }
    retain_bounded(tasks, now) || changed
}

fn set_output(task: &mut BackgroundTask, path: PathBuf) {
    // An agent's output file is a symlink to its own transcript, which the
    // child view already renders; it is never served as a log.
    task.has_output = task.task_type != "local_agent";
    task.output_file = Some(path);
}

/// Every task of a session writes into the same `…/tasks/` directory, and the
/// CLI names the file after the task id. Once one task has revealed that
/// directory, siblings that have not (agents' teammates, workflows, a task
/// whose launch result we never saw) get `<dir>/<id>.output` — derived from a
/// CLI-named path, never invented. A read still refuses it unless it exists
/// as a regular file.
fn share_tasks_dir(tasks: &mut [BackgroundTask]) {
    let Some(dir) = tasks
        .iter()
        .find_map(|t| t.output_file.as_deref().and_then(Path::parent))
        .map(Path::to_path_buf)
    else {
        return;
    };
    for task in tasks.iter_mut() {
        if task.output_file.is_none() && task.task_type != "local_agent" {
            task.output_file = Some(dir.join(format!("{}.output", task.id)));
            task.has_output = true;
        }
    }
}

/// Enforce the retention policy. Returns whether a row was dropped.
pub fn retain_bounded(tasks: &mut Vec<BackgroundTask>, now: i64) -> bool {
    let before = tasks.len();
    tasks.retain(|t| {
        t.is_running() || t.ended_at.unwrap_or(t.started_at) + FINISHED_RETENTION_MS > now
    });
    while tasks.len() > MAX_TASKS {
        let victim = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.is_running())
            .min_by_key(|(_, t)| t.ended_at.unwrap_or(t.started_at))
            .map(|(i, _)| i)
            .unwrap_or(0);
        tasks.remove(victim);
    }
    tasks.len() != before
}

/// A driver life ended (child exited, daemon teardown, or a resume replaced
/// it): nothing it launched is still running. Returns whether anything changed.
pub fn end_all(tasks: &mut [BackgroundTask], now: i64) -> bool {
    let mut changed = false;
    for task in tasks.iter_mut().filter(|t| t.is_running()) {
        task.status = "stopped".into();
        task.ended_at.get_or_insert(now);
        task.pid = None;
        task.process = None;
        changed = true;
    }
    changed
}

// ── Output reads ────────────────────────────────────────────────────────────

/// One bounded slice of a task's output file (snake_case like every other
/// claudemon HTTP body).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutputChunk {
    /// Byte offset `text` starts at.
    pub offset: u64,
    /// Where the next follow-up read should start.
    pub next_offset: u64,
    /// File size at read time.
    pub size: u64,
    pub text: String,
    /// The file shrank below the requested offset (rewritten): the client
    /// should replace, not append.
    pub reset: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OutputError {
    /// No output file is known for the task (agents, or not yet announced).
    NoOutput,
    /// The file is absent, not a regular file, or a symlink.
    Unavailable(String),
}

/// Read up to `max` bytes of `path` from `offset` (None = the tail). Refuses
/// symlinks and anything that is not a regular file; the caller already
/// resolved `path` from the daemon's own task row.
pub fn read_output(path: &Path, offset: Option<u64>, max: u64) -> Result<OutputChunk, OutputError> {
    use std::io::{Read, Seek, SeekFrom};
    let max = max.clamp(1, MAX_READ_BYTES);
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| OutputError::Unavailable(format!("output file: {e}")))?;
    if !meta.file_type().is_file() {
        return Err(OutputError::Unavailable(
            "output is not a regular file".into(),
        ));
    }
    let mut file =
        open_no_follow(path).map_err(|e| OutputError::Unavailable(format!("output file: {e}")))?;
    let size = file
        .metadata()
        .map_err(|e| OutputError::Unavailable(e.to_string()))?
        .len();
    let (start, reset, tail) = match offset {
        Some(o) if o <= size => (o, false, false),
        Some(_) => (size.saturating_sub(max), true, true),
        None => (size.saturating_sub(max), false, true),
    };
    let len = (size - start).min(max);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| OutputError::Unavailable(e.to_string()))?;
    let mut buf = vec![0u8; len as usize];
    file.read_exact(&mut buf)
        .map_err(|e| OutputError::Unavailable(e.to_string()))?;
    // A tail read can start mid-character: skip leading continuation bytes.
    let mut lead = 0;
    if tail && start > 0 {
        while lead < buf.len() && lead < 3 && (buf[lead] & 0xC0) == 0x80 {
            lead += 1;
        }
    }
    // Never end mid-character: hand the partial sequence to the next read.
    let mut end = buf.len();
    if let Err(e) = std::str::from_utf8(&buf[lead..end]) {
        if e.error_len().is_none() {
            end = lead + e.valid_up_to();
        }
    }
    let text = String::from_utf8_lossy(&buf[lead..end]).into_owned();
    Ok(OutputChunk {
        offset: start + lead as u64,
        next_offset: start + end as u64,
        size,
        text,
        reset,
    })
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    // symlink_metadata above already refused a link; there is no portable
    // no-follow open flag here.
    std::fs::File::open(path)
}

// ── Process verification (Linux only) ───────────────────────────────────────

/// A background task's root process, as proven at discovery time: a DIRECT
/// child of the session's own `claude` process whose stdout is that task's
/// own output file, started no earlier than shortly before the task appeared.
/// `start_ticks` pins the identity so a recycled pid cannot pass
/// [`verify_process`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedProcess {
    pub pid: u32,
    pub start_ticks: u64,
}

/// Live resource numbers for a verified task process tree.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ProcessSample {
    pub pid: u32,
    pub alive: bool,
    /// Processes in the tree rooted at `pid` (the shell plus what it runs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    /// Total CPU seconds consumed by the live tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_seconds: Option<f64>,
    /// CPU over the interval since this task's previous sample, 100 = one core.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
}

#[cfg(target_os = "linux")]
mod procfs {
    use super::*;
    use std::collections::HashSet;

    /// `/proc/<pid>/stat` fields this module needs. USER_HZ is 100 for the
    /// /proc ABI on every Linux architecture we ship.
    pub(super) const TICKS_PER_SEC: f64 = 100.0;

    pub(super) struct Stat {
        pub ppid: u32,
        pub cpu_ticks: u64,
        pub start_ticks: u64,
    }

    pub(super) fn stat(pid: u32) -> Option<Stat> {
        let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // `comm` is parenthesised and may contain spaces: split after the last ')'.
        let rest = &raw[raw.rfind(')')? + 2..];
        let f: Vec<&str> = rest.split_whitespace().collect();
        // rest[0] is field 3 (state); field N is f[N - 3].
        let n = |i: usize| f.get(i - 3).and_then(|s| s.parse::<u64>().ok());
        Some(Stat {
            ppid: n(4)? as u32,
            cpu_ticks: n(14)? + n(15)?,
            start_ticks: n(22)?,
        })
    }

    pub(super) fn rss_bytes(pid: u32) -> Option<u64> {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb * 1024)
    }

    pub(super) fn children(pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        if let Ok(threads) = std::fs::read_dir(format!("/proc/{pid}/task")) {
            for t in threads.flatten() {
                if let Ok(list) = std::fs::read_to_string(t.path().join("children")) {
                    out.extend(
                        list.split_whitespace()
                            .filter_map(|p| p.parse::<u32>().ok()),
                    );
                }
            }
        }
        if out.is_empty() {
            // Kernels without CONFIG_PROC_CHILDREN: scan for the parent link.
            if let Ok(all) = std::fs::read_dir("/proc") {
                for e in all.flatten() {
                    if let Some(child) = e.file_name().to_str().and_then(|s| s.parse().ok()) {
                        if stat(child).is_some_and(|s| s.ppid == pid) {
                            out.push(child);
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub(super) fn stdout_is(pid: u32, file: &Path) -> bool {
        let Ok(target) = std::fs::read_link(format!("/proc/{pid}/fd/1")) else {
            return false;
        };
        target == file || std::fs::canonicalize(file).is_ok_and(|c| c == target)
    }

    /// Wall-clock unix ms of a process start, from boot time + start ticks.
    pub(super) fn started_ms(start_ticks: u64) -> Option<i64> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let btime: i64 = stat
            .lines()
            .find_map(|l| l.strip_prefix("btime "))?
            .trim()
            .parse()
            .ok()?;
        Some(btime * 1000 + (start_ticks as f64 / TICKS_PER_SEC * 1000.0) as i64)
    }

    pub(super) fn tree(root: u32) -> Vec<u32> {
        let mut seen = HashSet::new();
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            if seen.len() >= 512 || !seen.insert(p) {
                continue;
            }
            stack.extend(children(p));
        }
        seen.into_iter().collect()
    }
}

/// How far before the daemon first saw a task its process may have started:
/// the CLI spawns the shell, then emits the live-set frame we stamp from.
const START_SLACK_MS: i64 = 10_000;

/// Find the task's root process. Proven, never guessed: exactly one direct
/// child of `claude_pid` must have the task's own output file as stdout, and
/// it must have started no earlier than [`START_SLACK_MS`] before the task
/// was first seen. Anything ambiguous yields `None`. Linux only.
pub fn discover_process(claude_pid: u32, task: &BackgroundTask) -> Option<VerifiedProcess> {
    #[cfg(target_os = "linux")]
    {
        let file = task.output_file.as_deref()?;
        let mut found = procfs::children(claude_pid)
            .into_iter()
            .filter(|&pid| procfs::stdout_is(pid, file));
        let pid = found.next()?;
        if found.next().is_some() {
            return None;
        }
        let stat = procfs::stat(pid)?;
        if stat.ppid != claude_pid {
            return None;
        }
        let started = procfs::started_ms(stat.start_ticks)?;
        if started < task.started_at - START_SLACK_MS {
            return None;
        }
        Some(VerifiedProcess {
            pid,
            start_ticks: stat.start_ticks,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (claude_pid, task);
        None
    }
}

/// Re-prove a previously verified process right before using it: same parent,
/// same start time (defeats pid reuse), stdout still the task's file.
pub fn verify_process(claude_pid: u32, task: &BackgroundTask, vp: VerifiedProcess) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Some(file) = task.output_file.as_deref() else {
            return false;
        };
        procfs::stat(vp.pid)
            .is_some_and(|s| s.ppid == claude_pid && s.start_ticks == vp.start_ticks)
            && procfs::stdout_is(vp.pid, file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (claude_pid, task, vp);
        false
    }
}

/// Previous CPU sample per (session, task), for a percentage between reads.
type CpuSamples = std::collections::HashMap<(String, String), (u64, std::time::Instant)>;
static CPU_SAMPLES: std::sync::Mutex<Option<CpuSamples>> = std::sync::Mutex::new(None);

/// Sample a task's process tree. `alive: false` when the proof no longer
/// holds (it exited, or the pid now names something else).
pub fn sample_process(
    session_id: &str,
    claude_pid: u32,
    task: &BackgroundTask,
    vp: VerifiedProcess,
) -> ProcessSample {
    let dead = ProcessSample {
        pid: vp.pid,
        alive: false,
        processes: None,
        rss_bytes: None,
        cpu_seconds: None,
        cpu_percent: None,
    };
    if !verify_process(claude_pid, task, vp) {
        return dead;
    }
    #[cfg(target_os = "linux")]
    {
        let tree = procfs::tree(vp.pid);
        let ticks: u64 = tree
            .iter()
            .filter_map(|&p| procfs::stat(p))
            .map(|s| s.cpu_ticks)
            .sum();
        let rss: u64 = tree.iter().filter_map(|&p| procfs::rss_bytes(p)).sum();
        let now = std::time::Instant::now();
        let key = (session_id.to_string(), task.id.clone());
        let mut guard = CPU_SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(Default::default);
        if map.len() > 256 {
            map.clear();
        }
        let cpu_percent = map.insert(key, (ticks, now)).and_then(|(prev, at)| {
            let secs = now.duration_since(at).as_secs_f64();
            (secs >= 0.2).then(|| {
                let used = ticks.saturating_sub(prev) as f64 / procfs::TICKS_PER_SEC;
                (used / secs * 1000.0).round() / 10.0
            })
        });
        ProcessSample {
            pid: vp.pid,
            alive: true,
            processes: Some(tree.len() as u32),
            rss_bytes: Some(rss),
            cpu_seconds: Some(ticks as f64 / procfs::TICKS_PER_SEC),
            cpu_percent,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = session_id;
        dead
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CAPTURE: &str =
        include_str!("../providers/testdata/claude-stream-bg-tasks-capture.jsonl");
    const FAILED: &str =
        include_str!("../providers/testdata/claude-stream-bg-tasks-failed-capture.jsonl");

    fn replay(capture: &str) -> Vec<BackgroundTask> {
        let mut tasks = Vec::new();
        let mut now = 1_000;
        for line in capture.lines().filter(|l| !l.trim().is_empty()) {
            let v: Value = serde_json::from_str(line).unwrap();
            for ev in parse_frame(&v) {
                apply(&mut tasks, ev, now);
            }
            now += 10;
        }
        tasks
    }

    /// A scratch directory under the shared test dir, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(prefix: &str) -> Self {
            let dir = crate::testtmp::dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl std::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn task<'a>(tasks: &'a [BackgroundTask], id: &str) -> &'a BackgroundTask {
        tasks.iter().find(|t| t.id == id).unwrap()
    }

    /// The real CLI 2.1.286 capture: two background shells and an async
    /// agent; the `sleep 300` shell was stopped through the `stop_task`
    /// control request (its `control_response` is in the fixture).
    #[test]
    fn real_capture_builds_the_task_list() {
        let tasks = replay(CAPTURE);
        assert_eq!(tasks.len(), 3);

        let ticks = task(&tasks, "bgznz35pp");
        assert_eq!(ticks.task_type, "local_bash");
        assert_eq!(ticks.status, "completed");
        assert_eq!(
            ticks.description.as_deref(),
            Some("for i in $(seq 1 8); do echo tick $i; sleep 1; done")
        );
        assert_eq!(
            ticks.tool_use_id.as_deref(),
            Some("toolu_01NoHxNBJ4KUV1aeBMrPxmKe")
        );
        assert!(ticks.has_output);
        assert_eq!(
            ticks.output_file.as_deref(),
            Some(Path::new(
                "/tmp/claude-1000/-home-user-scratch/332f16fa-bae7-4c72-b04a-4ed5cab2541d/tasks/bgznz35pp.output"
            ))
        );
        assert_eq!(ticks.ended_at, Some(1791393569890));
        assert!(ticks.summary.as_deref().unwrap().contains("exit code 0"));
        assert_eq!(ticks.subagent_id, None);

        let agent = task(&tasks, "ac4f6aefe4f43bd44");
        assert_eq!(agent.task_type, "local_agent");
        assert_eq!(agent.status, "completed");
        assert_eq!(agent.subagent_type.as_deref(), Some("general-purpose"));
        assert_eq!(agent.subagent_id.as_deref(), Some("ac4f6aefe4f43bd44"));
        assert!(!agent.has_output, "an agent's log is its child transcript");
        assert_eq!(agent.summary.as_deref(), Some("done"));
        assert_eq!(agent.usage.unwrap().total_tokens, 14543);

        let sleeper = task(&tasks, "b0z6rkl23");
        // task_updated said `killed`; the trailing notification says `stopped`.
        assert_eq!(sleeper.status, "stopped");
        assert!(sleeper.ended_at.is_some());
    }

    #[test]
    fn real_capture_tracks_progress_and_failure() {
        let tasks = replay(FAILED);
        let shell = task(&tasks, "bi3t30yt2");
        assert_eq!(shell.status, "failed");
        assert!(shell.summary.as_deref().unwrap().contains("exit code 3"));
        assert!(shell.has_output);

        let agent = task(&tasks, "a263f3ad82ae01d7d");
        assert_eq!(agent.status, "completed");
        assert_eq!(agent.last_tool_name.as_deref(), Some("Bash"));
        assert_eq!(agent.usage.unwrap().tool_uses, 3);
        // The final notification summary replaces the progress activity line.
        assert_eq!(agent.summary.as_deref(), Some("done"));
    }

    #[test]
    fn progress_frames_carry_activity_while_running() {
        let mut tasks = Vec::new();
        let mut lines = FAILED.lines();
        // Replay up to (and including) the first task_progress.
        for line in lines.by_ref() {
            let v: Value = serde_json::from_str(line).unwrap();
            let is_progress = v["subtype"] == "task_progress";
            for ev in parse_frame(&v) {
                apply(&mut tasks, ev, 5);
            }
            if is_progress {
                break;
            }
        }
        let agent = task(&tasks, "a263f3ad82ae01d7d");
        assert!(agent.is_running());
        assert_eq!(agent.description.as_deref(), Some("three echoes"));
        assert_eq!(agent.summary.as_deref(), Some("Running Echo the letter a"));
        assert_eq!(agent.usage.unwrap().tool_uses, 1);
    }

    #[test]
    fn running_shell_learns_its_output_file_from_the_launch_result() {
        let mut tasks = Vec::new();
        for line in CAPTURE.lines().take(3) {
            for ev in parse_frame(&serde_json::from_str(line).unwrap()) {
                apply(&mut tasks, ev, 5);
            }
        }
        let t = task(&tasks, "bgznz35pp");
        assert!(t.is_running());
        assert!(t.has_output);
        assert!(t
            .output_file
            .as_deref()
            .unwrap()
            .ends_with("tasks/bgznz35pp.output"));
    }

    #[test]
    fn output_paths_outside_the_cli_layout_are_ignored() {
        let sid = "s-1";
        let ok = confined_output_path("/tmp/claude-1/x/s-1/tasks/abc.output", "abc", sid);
        assert!(ok.is_some());
        for bad in [
            "/etc/passwd",
            "relative/s-1/tasks/abc.output",
            "/tmp/claude-1/x/s-1/tasks/other.output",
            "/tmp/claude-1/x/OTHER/tasks/abc.output",
            "/tmp/claude-1/x/s-1/notes/abc.output",
            "/tmp/claude-1/x/s-1/tasks/../tasks/abc.output",
        ] {
            assert_eq!(confined_output_path(bad, "abc", sid), None, "{bad}");
        }
        assert_eq!(
            confined_output_path("/a/s-1/tasks/..output", "..", sid),
            None
        );
        // A notification naming somebody else's file never becomes a log.
        let mut tasks = Vec::new();
        apply(
            &mut tasks,
            TaskEvent::LiveSet(vec![("abc".into(), "local_bash".into(), None, false)]),
            1,
        );
        for ev in parse_frame(&json!({"type":"system","subtype":"task_notification",
            "task_id":"abc","status":"completed","output_file":"/etc/shadow","session_id":sid}))
        {
            apply(&mut tasks, ev, 2);
        }
        assert!(!tasks[0].has_output);
        assert_eq!(tasks[0].output_file, None);
    }

    #[test]
    fn foreground_tasks_stay_out_until_backgrounded() {
        let mut tasks = Vec::new();
        let started = |bg: bool| {
            parse_frame(
                &json!({"type":"system","subtype":"task_started","task_id":"f1",
                "task_type":"local_bash","description":"npm test","is_backgrounded":bg}),
            )
        };
        for ev in started(false) {
            assert!(!apply(&mut tasks, ev, 1));
        }
        assert!(tasks.is_empty());
        for ev in started(true) {
            assert!(apply(&mut tasks, ev, 2));
        }
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn siblings_inherit_the_revealed_tasks_dir() {
        let mut tasks = Vec::new();
        apply(
            &mut tasks,
            TaskEvent::LiveSet(vec![
                ("wf1".into(), "local_workflow".into(), None, false),
                ("ag1".into(), "local_agent".into(), None, false),
                ("sh1".into(), "local_bash".into(), None, false),
            ]),
            1,
        );
        apply(
            &mut tasks,
            TaskEvent::Output {
                id: "sh1".into(),
                output_file: "/t/s/tasks/sh1.output".into(),
            },
            2,
        );
        assert_eq!(
            task(&tasks, "wf1").output_file.as_deref(),
            Some(Path::new("/t/s/tasks/wf1.output"))
        );
        assert_eq!(task(&tasks, "ag1").output_file, None);
    }

    #[test]
    fn retention_caps_rows_and_never_evicts_running_tasks_first() {
        let mut tasks = Vec::new();
        let live: Vec<_> = (0..MAX_TASKS + 5)
            .map(|i| (format!("t{i}"), "local_bash".to_string(), None, false))
            .collect();
        apply(&mut tasks, TaskEvent::LiveSet(live), 1);
        // Finish the first five, then add one more running task.
        for i in 0..5 {
            apply(
                &mut tasks,
                TaskEvent::Updated {
                    id: format!("t{i}"),
                    status: Some("completed".into()),
                    ended_at: Some(10 + i),
                    description: None,
                    error: None,
                },
                20,
            );
        }
        assert!(tasks.len() <= MAX_TASKS);
        assert!(tasks.iter().filter(|t| t.is_running()).count() >= MAX_TASKS - 5);
        // Finished rows age out.
        retain_bounded(&mut tasks, 20 + FINISHED_RETENTION_MS + 100);
        assert!(tasks.iter().all(BackgroundTask::is_running));
    }

    #[test]
    fn end_all_stops_running_rows_only() {
        let mut tasks = replay(CAPTURE);
        apply(
            &mut tasks,
            TaskEvent::LiveSet(vec![("new1".into(), "local_bash".into(), None, false)]),
            5,
        );
        assert!(end_all(&mut tasks, 9));
        assert_eq!(task(&tasks, "new1").status, "stopped");
        assert_eq!(task(&tasks, "bgznz35pp").status, "completed");
        assert!(!end_all(&mut tasks, 10));
    }

    #[test]
    fn wire_shape_is_camel_case_and_hides_paths() {
        let tasks = replay(CAPTURE);
        let v = serde_json::to_value(task(&tasks, "bgznz35pp")).unwrap();
        assert_eq!(v["taskType"], "local_bash");
        assert_eq!(v["hasOutput"], true);
        assert_eq!(v["toolUseId"], "toolu_01NoHxNBJ4KUV1aeBMrPxmKe");
        assert!(v.get("outputFile").is_none() && v.get("output_file").is_none());
        let back: BackgroundTask = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, "bgznz35pp");
    }

    #[test]
    fn reads_are_bounded_follow_able_and_utf8_safe() {
        let dir = Scratch::new("bg-task-read");
        let path = dir.join("t1.output");
        std::fs::write(&path, "héllo\nworld\n").unwrap();
        let all = read_output(&path, Some(0), 1024).unwrap();
        assert_eq!(all.text, "héllo\nworld\n");
        assert_eq!(all.next_offset, all.size);
        // A cut inside `é` (2 bytes at offset 1..3) hands the split char on.
        let first = read_output(&path, Some(0), 2).unwrap();
        assert_eq!(first.text, "h");
        assert_eq!(first.next_offset, 1);
        let next = read_output(&path, Some(first.next_offset), 2).unwrap();
        assert_eq!(next.text, "é");
        // Tail mode starts mid-file and never begins inside a character.
        let tail = read_output(&path, None, 11).unwrap();
        assert_eq!(tail.text, "llo\nworld\n");
        // Follow: append and read from the previous end.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"more\n"))
            .unwrap();
        let more = read_output(&path, Some(all.next_offset), 1024).unwrap();
        assert_eq!(more.text, "more\n");
        // A shrunken file resets the client.
        std::fs::write(&path, "x").unwrap();
        let reset = read_output(&path, Some(all.next_offset), 1024).unwrap();
        assert!(reset.reset);
        assert_eq!(reset.text, "x");
    }

    #[cfg(unix)]
    #[test]
    fn reads_refuse_symlinks_and_directories() {
        let dir = Scratch::new("bg-task-link");
        let secret = dir.join("secret");
        std::fs::write(&secret, "no").unwrap();
        let link = dir.join("t1.output");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(matches!(
            read_output(&link, None, 10),
            Err(OutputError::Unavailable(_))
        ));
        assert!(matches!(
            read_output(&dir.0, None, 10),
            Err(OutputError::Unavailable(_))
        ));
        assert!(matches!(
            read_output(&dir.join("missing.output"), None, 10),
            Err(OutputError::Unavailable(_))
        ));
    }

    /// A real child whose stdout is a task-shaped file is discovered and
    /// re-verified; a pid whose stdout is anything else never is.
    #[cfg(target_os = "linux")]
    #[test]
    fn process_discovery_requires_the_task_file_as_stdout() {
        let dir = Scratch::new("bg-task-proc");
        let tasks_dir = dir.join("sess").join("tasks");
        std::fs::create_dir_all(&tasks_dir).unwrap();
        let file = tasks_dir.join("tk1.output");
        let out = std::fs::File::create(&file).unwrap();
        // This test process plays the `claude` parent.
        let me = std::process::id();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(out)
            .spawn()
            .unwrap();
        let mut task = BackgroundTask::new("tk1", "local_bash", now_ms());
        task.output_file = Some(file.clone());
        let vp = discover_process(me, &task).expect("the child is provable");
        assert_eq!(vp.pid, child.id());
        assert!(verify_process(me, &task, vp));
        let sample = sample_process("s", me, &task, vp);
        assert!(sample.alive);
        assert!(sample.rss_bytes.unwrap() > 0);
        // A different task file proves nothing.
        let mut other = task.clone();
        other.output_file = Some(tasks_dir.join("tk2.output"));
        assert_eq!(discover_process(me, &other), None);
        // Wrong parent proves nothing.
        assert_eq!(discover_process(1, &task), None);
        // A forged start time (pid reuse) fails re-verification.
        assert!(!verify_process(
            me,
            &task,
            VerifiedProcess {
                pid: vp.pid,
                start_ticks: vp.start_ticks + 1
            }
        ));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!verify_process(me, &task, vp));
        assert!(!sample_process("s", me, &task, vp).alive);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_discovery_refuses_a_process_older_than_the_task() {
        let dir = Scratch::new("bg-task-old");
        let file = dir.join("tk1.output");
        let out = std::fs::File::create(&file).unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(out)
            .spawn()
            .unwrap();
        // The task claims to have appeared a minute after the process started.
        let mut task = BackgroundTask::new("tk1", "local_bash", now_ms() + 60_000);
        task.output_file = Some(file);
        assert_eq!(discover_process(std::process::id(), &task), None);
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[cfg(target_os = "linux")]
    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }
}

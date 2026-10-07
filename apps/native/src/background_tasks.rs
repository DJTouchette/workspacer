//! A session's background tasks as the hub reports them, and the bounded,
//! follow-able log of one of them.
//!
//! Claude's stream transport reports every task it runs beside the
//! conversation — `run_in_background` shells, async subagents, teammates,
//! cloud agents and workflows — on the session snapshot as
//! `background_task_list`, next to the plain `background_tasks` count. PTY
//! sessions carry only the count. A shell's (or workflow's) log is read by id
//! through `sessions.taskOutput`; an agent's transcript is the existing child
//! view. A pid appears only when the daemon proved it.
use serde_json::Value;

/// Rows kept per session (the daemon keeps 40).
pub const MAX_TASKS: usize = 40;
/// Log text kept in memory; older output is dropped from the front.
pub const MAX_LOG_BYTES: usize = 1024 * 1024;
/// Bytes asked for per read: the first read shows the tail, follow-ups
/// catch up in chunks of this size.
pub const READ_BYTES: u64 = 256 * 1024;
/// Free text kept per field (descriptions are whole shell commands).
const MAX_TEXT: usize = 1_000;

/// What kind of work a task is, from the CLI's `task_type`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// `local_bash`: a `run_in_background` shell.
    Shell,
    /// `local_agent`: an async Agent/Task subagent.
    Agent,
    /// `in_process_teammate`.
    Teammate,
    /// `remote_agent`: a cloud agent.
    Remote,
    /// `local_workflow`.
    Workflow,
    #[default]
    Other,
}

impl Kind {
    pub fn of(task_type: &str) -> Self {
        match task_type {
            "local_bash" => Self::Shell,
            "local_agent" => Self::Agent,
            "in_process_teammate" => Self::Teammate,
            "remote_agent" => Self::Remote,
            "local_workflow" => Self::Workflow,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Agent => "Subagent",
            Self::Teammate => "Teammate",
            Self::Remote => "Cloud agent",
            Self::Workflow => "Workflow",
            Self::Other => "Task",
        }
    }
}

/// Token/tool/time counters reported for agent tasks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub tokens: u64,
    pub tool_uses: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Task {
    pub id: String,
    pub kind: Kind,
    /// The CLI's own spelling, for kinds this client does not know.
    pub task_type: String,
    /// `pending`, `running`, `completed`, `failed`, `killed`, `stopped`.
    pub status: String,
    pub description: String,
    pub summary: String,
    pub error: String,
    pub last_tool: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub usage: Option<Usage>,
    /// Proven by the daemon; never guessed here.
    pub pid: Option<u32>,
    pub has_output: bool,
    /// The provider-native child this task is (async agents).
    pub subagent_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub ambient: bool,
}

fn text(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .map(|s| crate::transcript::head(s.trim(), MAX_TEXT))
        .unwrap_or_default()
}

fn id(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_owned)
}

impl Task {
    pub fn from_value(value: &Value) -> Option<Self> {
        let task_id = id(value, "id")?;
        let task_type = text(value, "taskType");
        let usage = value.get("usage").filter(|u| u.is_object()).map(|u| Usage {
            tokens: u["totalTokens"].as_u64().unwrap_or(0),
            tool_uses: u["toolUses"].as_u64().unwrap_or(0),
            duration_ms: u["durationMs"].as_u64().unwrap_or(0),
        });
        Some(Self {
            id: task_id,
            kind: Kind::of(&task_type),
            task_type,
            status: text(value, "status"),
            description: text(value, "description"),
            summary: text(value, "summary"),
            error: text(value, "error"),
            last_tool: text(value, "lastToolName"),
            started_at: value["startedAt"].as_i64(),
            ended_at: value["endedAt"].as_i64(),
            usage,
            pid: value["pid"]
                .as_u64()
                .and_then(|p| u32::try_from(p).ok())
                .filter(|p| *p > 0),
            has_output: value["hasOutput"].as_bool().unwrap_or(false),
            subagent_id: id(value, "subagentId"),
            tool_use_id: id(value, "toolUseId"),
            ambient: value["ambient"].as_bool().unwrap_or(false),
        })
    }

    /// Still working: anything but a terminal status.
    pub fn running(&self) -> bool {
        !matches!(
            self.status.as_str(),
            "completed" | "failed" | "killed" | "stopped"
        )
    }

    pub fn failed(&self) -> bool {
        self.status == "failed"
    }

    /// The row's name: what it runs, else its kind.
    pub fn title(&self) -> String {
        if self.description.is_empty() {
            self.kind.label().to_owned()
        } else {
            self.description.clone()
        }
    }

    pub fn status_label(&self) -> &str {
        match self.status.as_str() {
            "" | "running" => "Running",
            "pending" => "Starting",
            "completed" => "Done",
            "failed" => "Failed",
            "killed" | "stopped" => "Stopped",
            other => other,
        }
    }

    /// How long it ran (or has been running at `now`, unix ms).
    pub fn elapsed_ms(&self, now: i64) -> Option<i64> {
        let start = self.started_at?;
        let end = if self.running() { now } else { self.ended_at? };
        Some((end - start).max(0))
    }

    /// Opens a log in the panel (agents open their own conversation).
    pub fn shows_log(&self) -> bool {
        self.has_output && self.kind != Kind::Agent
    }
}

/// Parse a snapshot's `background_task_list`: running work first (oldest
/// first, as it was started), then finished tasks newest first.
pub fn parse(list: &Value) -> Vec<Task> {
    let mut tasks: Vec<Task> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Task::from_value)
        .take(MAX_TASKS)
        .collect();
    tasks.sort_by(|a, b| {
        b.running().cmp(&a.running()).then_with(|| {
            if a.running() {
                a.started_at.cmp(&b.started_at)
            } else {
                b.ended_at
                    .or(b.started_at)
                    .cmp(&a.ended_at.or(a.started_at))
            }
        })
    });
    tasks
}

pub fn running(tasks: &[Task]) -> usize {
    tasks.iter().filter(|t| t.running()).count()
}

/// The re-verified process behind a shell task, as of the latest read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub alive: bool,
    pub cpu_percent: Option<f64>,
    pub rss_bytes: Option<u64>,
    pub processes: Option<u32>,
}

impl Process {
    fn from_value(value: &Value) -> Option<Self> {
        let pid = u32::try_from(value["pid"].as_u64()?).ok()?;
        Some(Self {
            pid,
            alive: value["alive"].as_bool().unwrap_or(false),
            cpu_percent: value["cpu_percent"].as_f64().filter(|c| c.is_finite()),
            rss_bytes: value["rss_bytes"].as_u64(),
            processes: value["processes"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok()),
        })
    }
}

/// One task's log as this client holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Log {
    pub session: String,
    pub task: String,
    pub text: String,
    /// Where the next read starts; `None` before the first read (which asks
    /// for the tail).
    pub next_offset: Option<u64>,
    /// File size at the latest read.
    pub size: u64,
    /// Earlier output exists that this client does not hold.
    pub truncated: bool,
    /// The task finished and everything was read.
    pub done: bool,
    pub running: bool,
    pub status: String,
    pub process: Option<Process>,
}

impl Log {
    pub fn new(session: &str, task: &str) -> Self {
        Self {
            session: session.to_owned(),
            task: task.to_owned(),
            running: true,
            ..Default::default()
        }
    }

    /// More is already written than this client has read.
    pub fn behind(&self) -> bool {
        self.next_offset.is_some_and(|next| next < self.size)
    }

    /// Fold one `sessions.taskOutput` reply in. A reply for another task, or
    /// one that does not continue from where this log stands (a stale or
    /// duplicated read), changes nothing; the next read resumes from
    /// `next_offset`. Returns whether the log changed.
    pub fn apply(&mut self, reply: &Value) -> bool {
        if reply["task_id"].as_str() != Some(self.task.as_str())
            || reply["session_id"]
                .as_str()
                .is_some_and(|s| s != self.session)
        {
            return false;
        }
        let (Some(offset), Some(next)) = (reply["offset"].as_u64(), reply["next_offset"].as_u64())
        else {
            return false;
        };
        let chunk = reply["text"].as_str().unwrap_or("");
        let reset = reply["reset"].as_bool().unwrap_or(false);
        let before = self.clone();
        match self.next_offset {
            None => {
                self.text = chunk.to_owned();
                self.truncated = offset > 0;
            }
            Some(_) if reset => {
                self.text = chunk.to_owned();
                self.truncated = offset > 0;
            }
            Some(current) if offset == current => self.text.push_str(chunk),
            Some(_) => return false,
        }
        self.next_offset = Some(next);
        self.size = reply["size"].as_u64().unwrap_or(next);
        self.done = reply["done"].as_bool().unwrap_or(false);
        self.running = reply["running"].as_bool().unwrap_or(false);
        self.status = reply["status"].as_str().unwrap_or("").to_owned();
        self.process = reply.get("process").and_then(Process::from_value);
        if self.text.len() > MAX_LOG_BYTES {
            // Drop whole lines from the front where possible.
            let mut cut = self.text.len() - MAX_LOG_BYTES;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            let cut = self.text[cut..]
                .find('\n')
                .map(|nl| cut + nl + 1)
                .filter(|c| *c < self.text.len())
                .unwrap_or(cut);
            self.text.drain(..cut);
            self.truncated = true;
        }
        *self != before
    }

    /// Display lines (a trailing newline does not add an empty last line).
    pub fn lines(&self) -> Vec<&str> {
        let body = self.text.strip_suffix('\n').unwrap_or(&self.text);
        if body.is_empty() {
            return Vec::new();
        }
        body.split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect()
    }
}

/// `1.2 MB`-style size.
pub fn bytes_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: &str, kind: &str, status: &str, started: i64, ended: Option<i64>) -> Value {
        json!({"id":id,"taskType":kind,"status":status,"startedAt":started,"endedAt":ended,
            "description":format!("{id} work"),"hasOutput":kind != "local_agent"})
    }

    #[test]
    fn parses_the_daemon_rows_and_orders_running_first() {
        let list = json!([
            row("done-old", "local_bash", "completed", 1, Some(5)),
            row("run-late", "local_bash", "running", 30, None),
            row("done-new", "local_workflow", "failed", 2, Some(50)),
            row("run-early", "local_agent", "running", 10, None),
            {"taskType":"local_bash"},
        ]);
        let tasks = parse(&list);
        let order: Vec<_> = tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(order, ["run-early", "run-late", "done-new", "done-old"]);
        assert_eq!(running(&tasks), 2);
        assert_eq!(tasks[0].kind, Kind::Agent);
        assert!(!tasks[0].shows_log(), "agents open their own conversation");
        assert!(tasks[1].shows_log());
        assert!(tasks[2].failed());
        assert_eq!(tasks[2].status_label(), "Failed");
        assert_eq!(tasks[3].elapsed_ms(1_000), Some(4));
        assert_eq!(tasks[0].elapsed_ms(110), Some(100));
    }

    #[test]
    fn reads_every_wire_field_and_survives_unknown_kinds() {
        let task = Task::from_value(&json!({
            "id":"bgz","taskType":"local_bash","status":"running","description":"npm run dev",
            "startedAt":7,"pid":4242,"hasOutput":true,"toolUseId":"toolu_1","ambient":true,
            "usage":{"totalTokens":9,"toolUses":2,"durationMs":30},"lastToolName":"Bash",
            "summary":"listening on :3000","subagentId":null
        }))
        .unwrap();
        assert_eq!(task.pid, Some(4242));
        assert_eq!(task.title(), "npm run dev");
        assert_eq!(task.usage.unwrap().tool_uses, 2);
        assert!(task.ambient);
        let future = Task::from_value(&json!({"id":"x","taskType":"mcp_task"})).unwrap();
        assert_eq!(future.kind, Kind::Other);
        assert_eq!(future.title(), "Task");
        assert!(future.running(), "an unknown status is still working");
        // A zero or out-of-range pid is not a pid.
        assert_eq!(
            Task::from_value(&json!({"id":"x","pid":0})).unwrap().pid,
            None
        );
        assert_eq!(
            Task::from_value(&json!({"id":"x","pid":-3})).unwrap().pid,
            None
        );
    }

    fn reply(offset: u64, next: u64, text: &str) -> Value {
        json!({"session_id":"s","task_id":"t","offset":offset,"next_offset":next,
            "size":next,"text":text,"running":true,"status":"running","done":false})
    }

    #[test]
    fn the_log_follows_and_ignores_stale_or_foreign_reads() {
        let mut log = Log::new("s", "t");
        assert!(log.apply(&reply(100, 106, "tail\nx")));
        assert!(
            log.truncated,
            "a tail read starting mid-file has earlier output"
        );
        assert_eq!(log.next_offset, Some(106));
        assert!(log.apply(&reply(106, 110, "yz\n")));
        assert_eq!(log.text, "tail\nxyz\n");
        assert_eq!(log.lines(), ["tail", "xyz"]);
        // A duplicate of the previous read changes nothing.
        assert!(!log.apply(&reply(106, 110, "yz\n")));
        // Another task's reply changes nothing.
        let mut foreign = reply(110, 112, "no");
        foreign["task_id"] = json!("other");
        assert!(!log.apply(&foreign));
        // A rewritten file replaces the text.
        let mut reset = reply(0, 3, "new");
        reset["reset"] = json!(true);
        assert!(log.apply(&reset));
        assert_eq!(log.text, "new");
        assert!(!log.truncated);
        // Finished and fully read.
        let mut end = reply(3, 3, "");
        end["running"] = json!(false);
        end["done"] = json!(true);
        end["status"] = json!("completed");
        end["process"] = json!({"pid":12,"alive":false});
        assert!(log.apply(&end));
        assert!(log.done && !log.running);
        assert_eq!(log.process.unwrap().pid, 12);
    }

    #[test]
    fn the_log_stays_bounded_and_whole_lined() {
        let mut log = Log::new("s", "t");
        let line = "0123456789abcdef\n";
        let big = line.repeat(MAX_LOG_BYTES / line.len() + 100);
        log.apply(&reply(0, big.len() as u64, &big));
        assert!(log.text.len() <= MAX_LOG_BYTES);
        assert!(
            log.text.starts_with("0123"),
            "the cut lands on a line start"
        );
        assert!(log.truncated);
    }

    #[test]
    fn behind_means_more_was_written_than_read() {
        let mut log = Log::new("s", "t");
        assert!(!log.behind());
        let mut r = reply(0, 10, "0123456789");
        r["size"] = json!(500);
        log.apply(&r);
        assert!(log.behind());
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(bytes_label(512), "512 B");
        assert_eq!(bytes_label(1536), "1.5 KB");
        assert_eq!(bytes_label(3 * 1024 * 1024), "3.0 MB");
    }
}

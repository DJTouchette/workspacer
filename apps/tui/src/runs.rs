//! Subagent and workflow progress, read from Claude Code's on-disk artifacts.
//!
//! claudemon does not model either: it tails a session's sub-agent transcripts
//! only to get their token usage into the session's totals, and workflow runs
//! never reach it at all. The desktop's `workflowWatcher` reads the files
//! directly, and so does this — the TUI is a native process on the same machine,
//! already reading profiles, the agent library and session names off disk.
//!
//! Layout (sessionDir = the session's `transcript_path` minus `.jsonl`):
//!
//! ```text
//! <sessionDir>/subagents/agent-<id>.meta.json          { agentType, description, toolUseId }
//! <sessionDir>/subagents/agent-<id>.jsonl              transcript, appended live
//! <sessionDir>/subagents/workflows/wf_<runId>/         one dir per run, created at run start
//!     agent-<id>.meta.json / agent-<id>.jsonl          the run's agents
//!     journal.jsonl                                    { type: started|result, agentId, … }
//! <sessionDir>/workflows/scripts/<name>-wf_<id>.js     script copy, written at run start
//! <sessionDir>/workflows/wf_<runId>.json               rich final state, written ONCE at the end
//! ```
//!
//! So a *live* run is assembled from its dir, and the final state file is
//! adopted as authoritative the moment it appears. One deliberate gap while
//! live: phase titles live in the script's `export const meta` literal, which
//! the desktop reads by evaluating the JS. There is no JS engine here, so a live
//! run shows its name (from the script's filename, which carries it) and its
//! agents, and gains phases when the final file lands. Guessing at phases by
//! regexing a JS object literal would be worse than not showing them.
//!
//! Everything here is best-effort: a missing, partial or malformed file yields
//! less detail, never an error. These artifacts are written by another process
//! while we read them.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where an agent is in its life. Ordered so a sort surfaces live work first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RunState {
    Running,
    Queued,
    Done,
    Failed,
}

impl RunState {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Running => "◐",
            Self::Queued => "○",
            Self::Done => "✓",
            Self::Failed => "×",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "failed" => Self::Failed,
            "done" | "completed" => Self::Done,
            _ => Self::Queued,
        }
    }
}

/// A plain Agent-tool subagent — one `Task`/`Agent` call the session made.
#[derive(Debug, Clone)]
pub struct Subagent {
    pub id: String,
    /// The subagent type (`Explore`, `general-purpose`, …), from its meta file.
    pub agent_type: Option<String>,
    pub description: Option<String>,
    /// The `tool_use` id of the call that spawned it. This is the join back to
    /// the conversation: the call is finished exactly when its result has landed,
    /// which the fold already knows — no second source of truth for status.
    pub tool_use_id: Option<String>,
    /// Set once the caller resolves `tool_use_id` against the conversation.
    pub state: RunState,
    /// Last tool the subagent invoked, from the tail of its transcript.
    pub last_tool: Option<String>,
}

/// One agent inside a workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowAgent {
    pub id: String,
    /// Only known once the final state file lands (live agents have ids only).
    pub label: Option<String>,
    pub phase: Option<String>,
    pub model: Option<String>,
    pub state: RunState,
    pub last_tool: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowPhase {
    pub title: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowRun {
    pub run_id: String,
    /// From the script filename while live; from the final state file after.
    pub name: Option<String>,
    pub state: RunState,
    /// Empty while the run is live — see the module note.
    pub phases: Vec<WorkflowPhase>,
    pub agents: Vec<WorkflowAgent>,
    pub total_tokens: Option<u64>,
    pub duration_ms: Option<u64>,
}

impl WorkflowRun {
    pub fn done(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| matches!(a.state, RunState::Done | RunState::Failed))
            .count()
    }

    pub fn running(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.state == RunState::Running)
            .count()
    }
}

/// Everything on disk for one session.
#[derive(Debug, Clone, Default)]
pub struct SessionRuns {
    pub subagents: Vec<Subagent>,
    pub workflows: Vec<WorkflowRun>,
}

impl SessionRuns {
    pub fn is_empty(&self) -> bool {
        self.subagents.is_empty() && self.workflows.is_empty()
    }
}

/// The session's artifact directory: its transcript path minus `.jsonl`.
/// `None` for a path that doesn't have that shape, rather than inventing one.
pub fn session_dir(transcript_path: &str) -> Option<PathBuf> {
    let trimmed = transcript_path.strip_suffix(".jsonl")?;
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// Read a session's subagents and workflow runs. Cheap enough to call on a
/// refresh tick: it stats a couple of directories and reads the small metadata
/// files, and only ever reads a bounded tail of a transcript.
pub fn read(transcript_path: &str) -> SessionRuns {
    let Some(dir) = session_dir(transcript_path) else {
        return SessionRuns::default();
    };
    let subagents_root = dir.join("subagents");
    SessionRuns {
        subagents: read_plain_subagents(&subagents_root),
        workflows: read_workflows(&dir, &subagents_root),
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Agent ids appear both bare and `agent-`-prefixed depending on the writer.
fn strip_agent_prefix(id: &str) -> &str {
    id.strip_prefix("agent-").unwrap_or(id)
}

/// `agent-<id>.meta.json` → `<id>`.
fn meta_id(file_name: &str) -> Option<&str> {
    file_name
        .strip_prefix("agent-")?
        .strip_suffix(".meta.json")
        .filter(|s| !s.is_empty())
}

/// The last tool an agent invoked, from a bounded tail of its transcript.
///
/// Reads at most the final 16 KiB rather than the whole file: these grow to
/// megabytes, and all we want is the most recent `tool_use` name. A partial
/// first line from cutting mid-file simply fails to parse and is skipped.
fn last_tool_from_transcript(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 16 * 1024;
    let len = fs::metadata(path).ok()?.len();
    if len == 0 {
        return None;
    }
    // Seek to the tail rather than reading the file and slicing: the runs
    // overlay re-reads every subagent transcript in the directory once a
    // second, so reading whole multi-hundred-KB files to look at their last
    // 16 KiB churned megabytes per second on a background thread — and the cost
    // grew with transcript size instead of staying constant, which is exactly
    // what the bound was supposed to prevent.
    let mut file = fs::File::open(path).ok()?;
    let start = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::with_capacity(TAIL.min(len) as usize);
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut last = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        // Assistant rows carry content blocks; a tool_use block names the tool.
        let content = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array);
        for block in content.into_iter().flatten() {
            if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                if let Some(name) = str_field(block, "name") {
                    last = Some(name);
                }
            }
        }
    }
    last
}

/// Plain Agent-tool subagents. State is left `Running` here and resolved by the
/// caller against the conversation (see [`Subagent::tool_use_id`]).
fn read_plain_subagents(root: &Path) -> Vec<Subagent> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(id) = meta_id(&name) else { continue };
        let meta = read_json(&entry.path()).unwrap_or(Value::Null);
        out.push(Subagent {
            id: id.to_string(),
            agent_type: str_field(&meta, "agentType"),
            description: str_field(&meta, "description"),
            tool_use_id: str_field(&meta, "toolUseId"),
            state: RunState::Running,
            last_tool: last_tool_from_transcript(&root.join(format!("agent-{id}.jsonl"))),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn read_workflows(dir: &Path, subagents_root: &Path) -> Vec<WorkflowRun> {
    let workflows_dir = dir.join("workflows");
    let mut runs: HashMap<String, WorkflowRun> = HashMap::new();

    // Live runs: one dir per run under subagents/workflows/.
    if let Ok(entries) = fs::read_dir(subagents_root.join("workflows")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let run_id = name.to_string_lossy().to_string();
            if !run_id.starts_with("wf_") {
                continue;
            }
            let run = read_live_run(&run_id, &entry.path(), &workflows_dir);
            runs.insert(run_id, run);
        }
    }

    // Final state files are authoritative — they replace the live view wholesale.
    if let Ok(entries) = fs::read_dir(&workflows_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(run_id) = name.strip_suffix(".json").filter(|s| s.starts_with("wf_")) else {
                continue;
            };
            if let Some(run) = read_final_run(run_id, &entry.path()) {
                runs.insert(run_id.to_string(), run);
            }
        }
    }

    let mut out: Vec<WorkflowRun> = runs.into_values().collect();
    // Live runs first, then the most recent finished ones.
    out.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| b.run_id.cmp(&a.run_id)));
    out
}

/// A run still in flight: its agents come from the run dir, their states from
/// the journal, and its name from the script copy's filename.
fn read_live_run(run_id: &str, run_dir: &Path, workflows_dir: &Path) -> WorkflowRun {
    let mut agents: HashMap<String, WorkflowAgent> = HashMap::new();

    if let Ok(entries) = fs::read_dir(run_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(id) = meta_id(&name) else { continue };
            let transcript = run_dir.join(format!("agent-{id}.jsonl"));
            let started = fs::metadata(&transcript)
                .map(|m| m.len() > 0)
                .unwrap_or(false);
            let meta = read_json(&entry.path()).unwrap_or(Value::Null);
            agents.insert(
                id.to_string(),
                WorkflowAgent {
                    id: id.to_string(),
                    // Live agents have no label; the final file names them.
                    label: str_field(&meta, "description"),
                    phase: None,
                    model: str_field(&meta, "model"),
                    state: if started {
                        RunState::Running
                    } else {
                        RunState::Queued
                    },
                    last_tool: last_tool_from_transcript(&transcript),
                },
            );
        }
    }

    // The journal is the authority on start/finish while live.
    if let Ok(text) = fs::read_to_string(run_dir.join("journal.jsonl")) {
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(id) = str_field(&v, "agentId") else {
                continue;
            };
            let Some(agent) = agents.get_mut(strip_agent_prefix(&id)) else {
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("started") if agent.state == RunState::Queued => {
                    agent.state = RunState::Running;
                }
                Some("result") => agent.state = RunState::Done,
                _ => {}
            }
        }
    }

    let mut agents: Vec<WorkflowAgent> = agents.into_values().collect();
    agents.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| a.id.cmp(&b.id)));

    WorkflowRun {
        run_id: run_id.to_string(),
        name: script_name(workflows_dir, run_id),
        state: RunState::Running,
        phases: Vec::new(),
        agents,
        total_tokens: None,
        duration_ms: None,
    }
}

/// The workflow's name, carried by the script copy's filename
/// (`<name>-wf_<id>.js`) — no JS evaluation needed for the part that matters.
fn script_name(workflows_dir: &Path, run_id: &str) -> Option<String> {
    let suffix = format!("-{run_id}.js");
    let entries = fs::read_dir(workflows_dir.join("scripts")).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(stem) = name.strip_suffix(&suffix) {
            if !stem.is_empty() {
                return Some(stem.to_string());
            }
        }
    }
    None
}

/// The authoritative record written once at completion. Agents come from the
/// flat `workflowProgress` list, which interleaves `workflow_phase` and
/// `workflow_agent` entries.
fn read_final_run(run_id: &str, path: &Path) -> Option<WorkflowRun> {
    let v = read_json(path)?;
    let phases = v
        .get("phases")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    Some(WorkflowPhase {
                        title: str_field(p, "title")?,
                        detail: str_field(p, "detail"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let agents = v
        .get("workflowProgress")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|e| e.get("type").and_then(Value::as_str) == Some("workflow_agent"))
                .map(|e| WorkflowAgent {
                    id: str_field(e, "agentId").unwrap_or_default(),
                    label: str_field(e, "label"),
                    phase: str_field(e, "phaseTitle"),
                    model: str_field(e, "model"),
                    state: RunState::parse(e.get("state").and_then(Value::as_str).unwrap_or("")),
                    last_tool: str_field(e, "lastToolName"),
                })
                .collect()
        })
        .unwrap_or_default();

    Some(WorkflowRun {
        run_id: run_id.to_string(),
        name: str_field(&v, "workflowName"),
        state: RunState::parse(v.get("status").and_then(Value::as_str).unwrap_or("")),
        phases,
        agents,
        total_tokens: v.get("totalTokens").and_then(Value::as_u64),
        duration_ms: v.get("durationMs").and_then(Value::as_u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch session dir, laid out the way Claude Code writes one. Named by
    /// the test so parallel runs can't collide.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("wks-runs-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Fixture { root }
        }

        /// The `transcript_path` a session would report for this fixture.
        fn transcript(&self) -> String {
            format!("{}/sess.jsonl", self.root.display())
        }

        fn dir(&self) -> PathBuf {
            self.root.join("sess")
        }

        fn write(&self, rel: &str, body: &str) {
            let p = self.dir().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// One assistant transcript row carrying a tool_use block.
    fn tool_row(name: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"{name}"}}]}}}}"#
        )
    }

    #[test]
    fn session_dir_is_the_transcript_path_without_its_suffix() {
        assert_eq!(
            session_dir("/p/abc.jsonl"),
            Some(PathBuf::from("/p/abc")),
            "the artifact dir sits beside the transcript"
        );
        // A path we don't recognise yields nothing rather than a guess.
        assert_eq!(session_dir("/p/abc"), None);
        assert_eq!(session_dir(".jsonl"), None);
        assert_eq!(session_dir(""), None);
    }

    #[test]
    fn a_session_with_no_artifacts_reads_empty() {
        let fx = Fixture::new("empty");
        assert!(read(&fx.transcript()).is_empty());
        // A path with no session dir at all must not panic either.
        assert!(read("/nope/missing.jsonl").is_empty());
    }

    #[test]
    fn plain_subagents_come_from_their_meta_files() {
        let fx = Fixture::new("plain");
        fx.write(
            "subagents/agent-a1.meta.json",
            r#"{"agentType":"Explore","description":"Trace the boot flow","toolUseId":"toolu_9"}"#,
        );
        fx.write(
            "subagents/agent-a1.jsonl",
            &format!("{}\n{}\n", tool_row("Grep"), tool_row("Read")),
        );
        // A meta file with nothing in it still yields a row — the agent exists.
        fx.write("subagents/agent-a2.meta.json", "{}");

        let runs = read(&fx.transcript());
        assert_eq!(runs.subagents.len(), 2);
        let a1 = &runs.subagents[0];
        assert_eq!(a1.id, "a1");
        assert_eq!(a1.agent_type.as_deref(), Some("Explore"));
        assert_eq!(a1.description.as_deref(), Some("Trace the boot flow"));
        assert_eq!(a1.tool_use_id.as_deref(), Some("toolu_9"));
        assert_eq!(
            a1.last_tool.as_deref(),
            Some("Read"),
            "the most recent tool, not the first"
        );
        assert_eq!(runs.subagents[1].agent_type, None);
        assert_eq!(runs.subagents[1].last_tool, None, "no transcript yet");
    }

    #[test]
    fn malformed_metadata_degrades_instead_of_failing() {
        let fx = Fixture::new("malformed");
        fx.write("subagents/agent-bad.meta.json", "{ not json");
        fx.write("subagents/agent-bad.jsonl", "half a line");
        // Not a meta file at all — must be ignored, not misparsed.
        fx.write("subagents/notes.txt", "hello");

        let runs = read(&fx.transcript());
        assert_eq!(runs.subagents.len(), 1);
        assert_eq!(runs.subagents[0].id, "bad");
        assert_eq!(runs.subagents[0].agent_type, None);
    }

    /// A live run: agents from the dir, states from the journal, and the name off
    /// the script copy's filename (no JS evaluation).
    #[test]
    fn a_live_workflow_run_is_assembled_from_its_dir() {
        let fx = Fixture::new("live");
        let run = "subagents/workflows/wf_abc123";
        fx.write(&format!("{run}/agent-q1.meta.json"), "{}");
        fx.write(&format!("{run}/agent-r1.meta.json"), "{}");
        fx.write(&format!("{run}/agent-r1.jsonl"), &tool_row("Bash"));
        fx.write(&format!("{run}/agent-d1.meta.json"), "{}");
        fx.write(&format!("{run}/agent-d1.jsonl"), &tool_row("Edit"));
        fx.write(
            &format!("{run}/journal.jsonl"),
            "{\"type\":\"started\",\"agentId\":\"agent-r1\"}\n\
             {\"type\":\"started\",\"agentId\":\"d1\"}\n\
             {\"type\":\"result\",\"agentId\":\"agent-d1\",\"result\":\"ok\"}\n",
        );
        fx.write("workflows/scripts/find-bugs-wf_abc123.js", "// script");

        let runs = read(&fx.transcript());
        assert_eq!(runs.workflows.len(), 1);
        let wf = &runs.workflows[0];
        assert_eq!(wf.run_id, "wf_abc123");
        assert_eq!(
            wf.name.as_deref(),
            Some("find-bugs"),
            "the name rides the script filename"
        );
        assert_eq!(wf.state, RunState::Running);
        assert!(wf.phases.is_empty(), "phases only land with the final file");
        assert_eq!(wf.agents.len(), 3);

        let state_of = |id: &str| wf.agents.iter().find(|a| a.id == id).unwrap().state;
        // Journalled result → done, whichever id spelling the journal used.
        assert_eq!(state_of("d1"), RunState::Done);
        // Started, no result → running.
        assert_eq!(state_of("r1"), RunState::Running);
        // Registered but never started → queued.
        assert_eq!(state_of("q1"), RunState::Queued);
        assert_eq!(wf.done(), 1);
        assert_eq!(wf.running(), 1);
        // Live work sorts first.
        assert_eq!(wf.agents[0].state, RunState::Running);
    }

    /// The final state file is authoritative and replaces the live view: it is
    /// the only place phases, labels and models exist.
    #[test]
    fn the_final_state_file_supersedes_the_live_run_dir() {
        let fx = Fixture::new("final");
        let run = "subagents/workflows/wf_xyz";
        fx.write(&format!("{run}/agent-a.meta.json"), "{}");
        fx.write(
            "workflows/wf_xyz.json",
            r#"{
              "runId": "wf_xyz",
              "workflowName": "hub-bug-hunt",
              "status": "completed",
              "durationMs": 412108,
              "totalTokens": 1263590,
              "phases": [
                { "title": "Find", "detail": "one finder per module" },
                { "title": "Verify" }
              ],
              "workflowProgress": [
                { "type": "workflow_phase", "index": 1, "title": "Find" },
                { "type": "workflow_agent", "agentId": "a", "label": "find:bus",
                  "phaseTitle": "Find", "model": "opus", "state": "done",
                  "lastToolName": "StructuredOutput" },
                { "type": "workflow_agent", "agentId": "b", "label": "verify:bus",
                  "phaseTitle": "Verify", "state": "failed" }
              ]
            }"#,
        );

        let runs = read(&fx.transcript());
        assert_eq!(runs.workflows.len(), 1, "not one live + one final");
        let wf = &runs.workflows[0];
        assert_eq!(wf.name.as_deref(), Some("hub-bug-hunt"));
        assert_eq!(wf.state, RunState::Done);
        assert_eq!(wf.total_tokens, Some(1_263_590));
        assert_eq!(wf.duration_ms, Some(412_108));
        assert_eq!(wf.phases.len(), 2);
        assert_eq!(
            wf.phases[0].detail.as_deref(),
            Some("one finder per module")
        );
        assert_eq!(wf.phases[1].detail, None);
        // Only workflow_agent entries become agents — phase markers share the list.
        assert_eq!(wf.agents.len(), 2);
        assert_eq!(wf.agents[0].label.as_deref(), Some("find:bus"));
        assert_eq!(wf.agents[0].phase.as_deref(), Some("Find"));
        assert_eq!(wf.agents[0].model.as_deref(), Some("opus"));
        assert_eq!(wf.agents[1].state, RunState::Failed);
        assert_eq!(wf.done(), 2, "a failure is finished work too");
    }

    #[test]
    fn runs_sort_live_first_then_newest() {
        let fx = Fixture::new("sort");
        fx.write("subagents/workflows/wf_live/agent-a.meta.json", "{}");
        fx.write(
            "workflows/wf_old.json",
            r#"{"status":"completed","workflowName":"old"}"#,
        );
        let runs = read(&fx.transcript());
        assert_eq!(runs.workflows.len(), 2);
        assert_eq!(runs.workflows[0].run_id, "wf_live", "live work first");
    }

    /// The tail bound is the point: the runs overlay re-reads every subagent
    /// transcript in the directory once a second, so this must cost the same
    /// whether the file is 20 KB or 20 MB.
    #[test]
    fn last_tool_reads_only_the_tail_of_a_large_transcript() {
        let fx = Fixture::new("tail");
        let row = |tool: &str| {
            serde_json::json!({
                "type": "assistant",
                "message": { "content": [{ "type": "tool_use", "name": tool }] }
            })
            .to_string()
        };
        // An early tool call, then >16 KiB of filler, then the recent one. The
        // early name is only reachable by a caller that read the whole file.
        let filler = serde_json::json!({ "type": "user", "message": {
            "content": "x".repeat(1024)
        }})
        .to_string();
        let mut body = String::new();
        body.push_str(&row("AncientTool"));
        body.push('\n');
        for _ in 0..40 {
            body.push_str(&filler);
            body.push('\n');
        }
        body.push_str(&row("RecentTool"));
        body.push('\n');
        fx.write("subagents/agent-a.jsonl", &body);

        let path = fx.dir().join("subagents/agent-a.jsonl");
        assert!(
            fs::metadata(&path).unwrap().len() > 16 * 1024,
            "fixture must exceed the tail window to be meaningful"
        );
        assert_eq!(
            last_tool_from_transcript(&path),
            Some("RecentTool".to_string()),
            "the most recent tool_use in the tail wins"
        );
    }

    #[test]
    fn last_tool_handles_a_short_transcript_and_a_partial_first_line() {
        let fx = Fixture::new("short");
        // Deliberately leading with a truncated row: cutting mid-file leaves one,
        // and it must be skipped rather than aborting the scan.
        fx.write(
            "subagents/agent-b.jsonl",
            "{\"type\":\"assist\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\"}]}}\n",
        );
        let path = fx.dir().join("subagents/agent-b.jsonl");
        assert_eq!(last_tool_from_transcript(&path), Some("Bash".to_string()));
    }

    #[test]
    fn last_tool_is_none_for_an_empty_or_missing_transcript() {
        let fx = Fixture::new("empty");
        fx.write("subagents/agent-c.jsonl", "");
        assert_eq!(
            last_tool_from_transcript(&fx.dir().join("subagents/agent-c.jsonl")),
            None
        );
        assert_eq!(
            last_tool_from_transcript(&fx.dir().join("subagents/nope.jsonl")),
            None
        );
    }
}

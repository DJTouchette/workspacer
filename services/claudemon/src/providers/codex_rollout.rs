//! Codex "hybrid" GUI half — tail a Codex TUI session's rollout JSONL and fold
//! it into the shared session model, so a PTY-backed Codex session lights up the
//! GUI conversation view exactly like a Claude session does from its transcript.
//!
//! A hybrid Codex agent is spawned as a normal PTY (`codex` TUI — the Term view,
//! via `/sessions/spawn`). Codex persists that session incrementally to
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`. We discover that
//! file (newest rollout whose `session_meta.cwd` matches, created right after the
//! spawn), then tail it, translating each line into the same [`AgentUpdate`]s the
//! app-server adapter emits — so conversation / mode / usage all flow through the
//! unchanged `apply_updates` path under our canonical id.
//!
//! We parse the `response_item` records (assistant/user messages, `function_call`
//! and `function_call_output`) for a rich GUI — the same structured items the
//! app-server RPC would stream, including tool cards and their results — plus the
//! `event_msg` `task_started`/`task_complete` (busy/idle) and `token_count`
//! (usage). This is the Windows path: the Codex app-server *daemon* that would let
//! a TUI and an RPC client share one live thread is Unix-only, so on Windows the
//! rollout file is the only live structured channel out of the TUI session.

use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::BaseDirs;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::{apply_updates, AgentUpdate, UsageAcc};
use crate::session::state::SessionMode;
use crate::session::{ConversationStore, SessionStore};

/// Translate one rollout JSONL line into typed updates. Pure and total.
///
/// Two record families matter:
///   - `response_item` — the structured turn stream (the same items the
///     app-server RPC emits): assistant/user `message`s, `function_call`s and
///     their `function_call_output`s. This is what makes the GUI rich (tool
///     cards + results), matching what the RPC path would show.
///   - `event_msg` — high-level session events; we take only `task_started` /
///     `task_complete` (busy/idle) and `token_count` (usage). Its
///     `user_message` / `agent_message` are intentionally ignored — the same
///     text already arrives as `response_item` messages, so honoring both would
///     duplicate every turn.
///
/// `turn_context` yields only the model name; `session_meta` and reasoning
/// items yield nothing.
pub fn translate(value: &Value) -> Vec<AgentUpdate> {
    match value.get("type").and_then(Value::as_str) {
        Some("response_item") => translate_response_item(value.get("payload").unwrap_or(value)),
        Some("event_msg") => translate_event_msg(value.get("payload").unwrap_or(value)),
        // `turn_context` is otherwise ignored, but it's the only rollout record
        // naming the model — harvest it so the status line isn't blank.
        Some("turn_context") => {
            let model = value
                .get("payload")
                .unwrap_or(value)
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match model {
                Some(m) => vec![AgentUpdate::Usage {
                    model: Some(m),
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                    context_tokens: None,
                    context_window: None,
                }],
                None => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// Collect the text from a Codex message item's `content` array (elements like
/// `{ "type": "input_text" | "output_text" | "text", "text": "…" }`).
fn message_text(payload: &Value) -> String {
    match payload.get("content") {
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn translate_response_item(payload: &Value) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    match payload.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => {
            let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
            let text = message_text(payload);
            if text.trim().is_empty() {
                return out;
            }
            match role {
                "user" => out.push(AgentUpdate::UserText(text)),
                "assistant" => out.push(AgentUpdate::AssistantText(text)),
                _ => {} // system / developer instructions — not conversation
            }
        }
        "function_call" => {
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let id = payload
                .get("call_id")
                .or_else(|| payload.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            // `arguments` is a JSON string; parse it so the GUI can render fields
            // (command, path, …). Fall back to the raw string if it isn't JSON.
            let mut input = payload
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .or_else(|| payload.get("arguments").cloned())
                .unwrap_or(Value::Null);
            // Codex's plan tool is a function call named `update_plan` whose
            // arguments carry `{ plan: [{ step, status }] }`. Surface it as a
            // first-class plan in addition to the tool card, mirroring how the
            // Claude path treats `TodoWrite`.
            if name == "update_plan" {
                if let Some(plan) = super::plan_from_value(&input) {
                    out.push(AgentUpdate::Plan(plan));
                }
            }
            // An apply_patch function call carries the raw patch text (as the
            // whole arguments string, or under `input`/`patch`) — normalize to
            // `{ path, diff }` so the GUI can name the file and show the diff.
            if name == "apply_patch" {
                if let Some(patch) = patch_text_of(&input) {
                    input = apply_patch_input(&patch);
                }
            }
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        // Codex ≥0.14x emits apply_patch as a *custom* tool call: `input` is the
        // raw "*** Begin Patch …" text (not JSON). Without this arm the rollout
        // path drops every file edit on the floor.
        "custom_tool_call" => {
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let id = payload
                .get("call_id")
                .or_else(|| payload.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let raw = payload.get("input").and_then(Value::as_str).unwrap_or("");
            let input = if name == "apply_patch" {
                apply_patch_input(raw)
            } else {
                serde_json::json!({ "input": raw })
            };
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        "function_call_output" | "custom_tool_call_output" => {
            let tool_use_id = payload
                .get("call_id")
                .or_else(|| payload.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let (content, is_error) = function_output_text(payload.get("output"));
            out.push(AgentUpdate::ToolResult {
                tool_use_id,
                content,
                is_error,
            });
        }
        _ => {} // reasoning, etc. — skipped
    }
    out
}

/// Pull the raw patch text out of an apply_patch call's parsed arguments: the
/// whole value when it's a string, else an `input` / `patch` string field.
fn patch_text_of(input: &Value) -> Option<String> {
    match input {
        Value::String(s) => Some(s.clone()),
        Value::Object(map) => map
            .get("input")
            .or_else(|| map.get("patch"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

/// Build the ToolUse input for an apply_patch call from its raw patch text
/// (`*** Begin Patch\n*** Update File: path\n…`): the touched paths give the
/// GUI a `path` headline, and the whole patch rides as `diff` (rendered as the
/// inline diff, same key the app-server path uses).
fn apply_patch_input(patch: &str) -> Value {
    let mut paths: Vec<String> = Vec::new();
    for line in patch.lines() {
        for prefix in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                paths.push(rest.trim().to_string());
            }
        }
    }
    let mut input = serde_json::json!({ "diff": patch });
    if let Some(first) = paths.first() {
        input["path"] = serde_json::json!(first);
    }
    if paths.len() > 1 {
        input["paths"] = serde_json::json!(paths);
    }
    input
}

/// Flatten a `function_call_output.output` / `custom_tool_call_output.output`
/// (a string, an object that may carry `content`/`text`/`output` and a success
/// flag, or — custom tool calls — a string that itself encodes such an object,
/// e.g. `"{\"output\":\"…\",\"metadata\":{\"exit_code\":0}}"`) into display
/// text + an error flag.
fn function_output_text(output: Option<&Value>) -> (String, bool) {
    fn object_text(map: &serde_json::Map<String, Value>) -> (String, bool) {
        let is_error = map
            .get("success")
            .and_then(Value::as_bool)
            .map(|ok| !ok)
            .or_else(|| {
                map.get("metadata")
                    .and_then(|m| m.get("exit_code"))
                    .and_then(Value::as_i64)
                    .map(|code| code != 0)
            })
            .unwrap_or(false);
        let text = ["content", "output", "text"]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_str))
            .map(str::to_string)
            .unwrap_or_else(|| Value::Object(map.clone()).to_string());
        (text, is_error)
    }
    match output {
        Some(Value::String(s)) => {
            // custom_tool_call_output (and Codex's shell exec) wrap their
            // envelope in the string: `{"output":"…","metadata":{"exit_code":…}}`.
            // Only that exact shape is unwrapped — any other JSON-object string
            // is a legitimate tool result (e.g. an MCP tool that answers with a
            // JSON payload) and must be shown verbatim: flattening it would drop
            // fields, and reading a `success: false` field in it as a failure
            // would invent errors the tool never reported.
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(s) {
                let looks_like_envelope = map.get("output").is_some_and(Value::is_string)
                    && map.get("metadata").is_some_and(Value::is_object);
                if looks_like_envelope {
                    return object_text(&map);
                }
            }
            (s.clone(), false)
        }
        Some(Value::Object(map)) => object_text(map),
        Some(other) => (other.to_string(), false),
        None => (String::new(), false),
    }
}

fn translate_event_msg(payload: &Value) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    match payload.get("type").and_then(Value::as_str).unwrap_or("") {
        "task_started" => out.push(AgentUpdate::Busy),
        "task_complete" => out.push(AgentUpdate::Idle),
        "token_count" => {
            let info = payload.get("info");
            let usage = info.and_then(|i| i.get("total_token_usage"));
            let input = usage
                .and_then(|u| u.get("input_tokens"))
                .and_then(Value::as_u64);
            let output = usage
                .and_then(|u| u.get("output_tokens"))
                .and_then(Value::as_u64);
            if input.is_some() || output.is_some() {
                // Context occupancy comes from `last_token_usage` — the most
                // recent request, i.e. what's actually in the window.
                // `total_token_usage` is CUMULATIVE across the session and
                // pins the context meter at 100% within a few turns.
                let context_tokens = info
                    .and_then(|i| i.get("last_token_usage"))
                    .and_then(|l| l.get("total_tokens"))
                    .and_then(Value::as_u64)
                    .or_else(|| Some(input.unwrap_or(0) + output.unwrap_or(0)));
                let context_window = info
                    .and_then(|i| i.get("model_context_window"))
                    .and_then(Value::as_u64);
                out.push(AgentUpdate::Usage {
                    model: None,
                    input_tokens: input,
                    output_tokens: output,
                    cost_usd: None,
                    context_tokens,
                    context_window,
                });
            }
            // The same event carries the account's rate-limit windows.
            if let Some(u) = payload.get("rate_limits").and_then(super::rate_limits_from) {
                out.push(u);
            }
        }
        _ => {} // user_message / agent_message ignored (see response_item messages)
    }
    out
}

/// `$CODEX_HOME` (if set) else `<home>/.codex`. The home dir comes from
/// `directories::BaseDirs` — the same resolver the rest of claudemon uses — so
/// this works on Windows (USERPROFILE/known folders), where `$HOME` is normally
/// unset. Reading `$HOME` directly here meant Codex's GUI transcript never
/// populated on Windows (the rollout was never discovered).
fn codex_home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(h));
    }
    BaseDirs::new().map(|b| b.home_dir().join(".codex"))
}

/// Compare two filesystem paths for equality, tolerant of the differences that
/// crop up between the cwd we spawn Codex with and the one it records in
/// `session_meta`: separator style (`\` vs `/`), a trailing separator, and — on
/// Windows — case. Without this the exact-string match in `discover_rollout`
/// can miss the right rollout and leave the GUI view empty.
fn paths_eq(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        let t = s.replace('\\', "/");
        let t = t.trim_end_matches('/');
        if cfg!(windows) {
            t.to_lowercase()
        } else {
            t.to_string()
        }
    }
    norm(a) == norm(b)
}

/// Read the `session_meta.cwd` from the first JSON line of a rollout file.
async fn rollout_cwd(path: &Path) -> Option<String> {
    let content = tokio::fs::read_to_string(path).await.ok()?;
    let first = content.lines().find(|l| !l.trim().is_empty())?;
    let v: Value = serde_json::from_str(first).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    v.get("payload")
        .and_then(|p| p.get("cwd"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Collect every `rollout-*.jsonl` under `sessions_root` (recurses the
/// YYYY/MM/DD layout), with its modified time. Bounded, best-effort.
fn collect_rollouts(sessions_root: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![sessions_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
                .unwrap_or(false)
            {
                if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                    out.push((path, modified));
                }
            }
        }
    }
    out
}

/// Find the rollout file for a session we just spawned in `cwd`: the newest
/// rollout, modified at/after `since`, whose `session_meta.cwd` matches.
///
/// Polls until the rollout appears or the session ends. Codex can take well over
/// the old 20s budget to write its first `session_meta` — a cold start pays for
/// model + MCP startup and a large base-instruction flush — and if we gave up
/// first, the rollout showed up orphaned and the GUI view stayed empty forever
/// (the Term/PTY still worked). Waiting is cheap (250ms sleeps), and we bail the
/// instant the session is deregistered so a dead or never-recording session
/// can't poll forever. The ceiling is a backstop, not the expected path.
async fn discover_rollout(
    store: &SessionStore,
    session_id: &str,
    cwd: &str,
    since: std::time::SystemTime,
) -> Option<PathBuf> {
    let sessions_root = codex_home()?.join("sessions");
    // A small grace window before `since` guards against coarse mtime / clock skew.
    let cutoff = since.checked_sub(Duration::from_secs(5)).unwrap_or(since);
    // ~3 min backstop at 250ms/iter — long enough for any realistic cold start,
    // bounded so a session that never records eventually stops scanning.
    for _ in 0..720 {
        store.get(session_id)?;
        let mut candidates: Vec<(PathBuf, std::time::SystemTime)> =
            collect_rollouts(&sessions_root)
                .into_iter()
                .filter(|(_, m)| *m >= cutoff)
                .collect();
        // Newest first, so the first cwd match is the most recent session.
        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        for (path, _) in candidates {
            if rollout_cwd(&path).await.is_some_and(|c| paths_eq(&c, cwd)) {
                return Some(path);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    None
}

/// Spawn the rollout tailer for a hybrid Codex session. Returns immediately; the
/// session id is already registered (as a PTY) by the caller. Ends when the
/// session is deregistered or the daemon shuts down.
pub fn spawn_tailer(store: SessionStore, conv: ConversationStore, session_id: String, cwd: String) {
    tokio::spawn(async move {
        let since = std::time::SystemTime::now();
        let Some(path) = discover_rollout(&store, &session_id, &cwd, since).await else {
            tracing::warn!(session = %session_id, %cwd, "codex rollout not found; GUI view will stay empty");
            return;
        };
        tracing::info!(session = %session_id, path = %path.display(), "tailing codex rollout");
        if let Err(err) = tail(&store, &conv, &session_id, &path).await {
            tracing::warn!(?err, session = %session_id, "codex rollout tail ended with error");
        }
    });
}

/// Fold a freshly-read chunk into the carried partial-line bytes, returning
/// every now-complete line. The carry is the ONE mechanism for lines caught
/// mid-write (the read offset always advances past every byte read) — carrying
/// the partial AND rewinding the offset would replay those bytes twice and
/// corrupt the record. It is raw *bytes*, not a `String`, because a poll can
/// also land mid-UTF-8-character: decoding happens only on complete lines, and
/// lossily, so one bad byte degrades a single line instead of killing the
/// tailer. Same pattern as the transcript tailer in `session::conversation`.
fn drain_complete_lines(carry: &mut Vec<u8>, chunk: &[u8]) -> Vec<String> {
    carry.extend_from_slice(chunk);
    let Some(idx) = carry.iter().rposition(|&b| b == b'\n') else {
        return Vec::new(); // still no complete line — keep carrying
    };
    let rest = carry.split_off(idx + 1);
    let complete = std::mem::replace(carry, rest);
    String::from_utf8_lossy(&complete)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Tail a rollout file from the beginning, folding each new line into the stores
/// until the session is gone. Reads any already-written lines first (so a
/// resumed session replays its history into the GUI), then polls for appends by
/// tracking the byte offset.
async fn tail(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    path: &Path,
) -> anyhow::Result<()> {
    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // The session is ready for input until a turn starts (the TUI is idle on
    // launch); reflect that so the GUI composer's /message isn't mode-gated away.
    store.set_managed_mode(session_id, SessionMode::Input, None);

    let mut offset: u64 = 0;
    // Bytes of a trailing record caught mid-write (see `drain_complete_lines`).
    let mut carry: Vec<u8> = Vec::new();
    loop {
        // The session vanished (deregistered / exited) — stop tailing.
        if store.get(session_id).is_none() {
            break;
        }
        let mut file = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(_) => break, // rollout removed
        };
        let len = file.metadata().await.map(|m| m.len()).unwrap_or(offset);
        if len > offset {
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            // Bound the read to the length we statted so `offset` stays
            // consistent even if the file grows while we read.
            let mut chunk = Vec::with_capacity((len - offset) as usize);
            (&mut file)
                .take(len - offset)
                .read_to_end(&mut chunk)
                .await?;
            offset += chunk.len() as u64;
            for line in drain_complete_lines(&mut carry, &chunk) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                    let updates = translate(&value);
                    if !updates.is_empty() {
                        apply_updates(store, conv, session_id, updates, &mut cur_mode, &mut acc);
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(payload: Value) -> Value {
        json!({ "type": "event_msg", "payload": payload })
    }
    fn item(payload: Value) -> Value {
        json!({ "type": "response_item", "payload": payload })
    }

    #[test]
    fn non_conversational_lines_are_ignored() {
        assert!(
            translate(&json!({ "type": "session_meta", "payload": { "cwd": "/x" } })).is_empty()
        );
        assert!(translate(&json!({ "type": "turn_context", "payload": {} })).is_empty());
        // reasoning items are not surfaced
        assert!(translate(&item(json!({ "type": "reasoning", "summary": [] }))).is_empty());
        // event_msg messages are dropped (the same text comes via response_item)
        assert!(translate(&ev(json!({ "type": "agent_message", "message": "dup" }))).is_empty());
        assert!(translate(&ev(json!({ "type": "user_message", "message": "dup" }))).is_empty());
    }

    #[test]
    fn task_started_completed_map_to_busy_idle() {
        assert_eq!(
            translate(&ev(json!({ "type": "task_started" }))),
            vec![AgentUpdate::Busy]
        );
        assert_eq!(
            translate(&ev(json!({ "type": "task_complete" }))),
            vec![AgentUpdate::Idle]
        );
    }

    #[test]
    fn response_item_messages_map_to_text() {
        assert_eq!(
            translate(&item(json!({ "type": "message", "role": "user",
                "content": [{ "type": "input_text", "text": "hi" }] }))),
            vec![AgentUpdate::UserText("hi".into())]
        );
        assert_eq!(
            translate(&item(json!({ "type": "message", "role": "assistant",
                "content": [{ "type": "output_text", "text": "OK" }] }))),
            vec![AgentUpdate::AssistantText("OK".into())]
        );
        // system/developer messages and empty content are not conversation
        assert!(translate(&item(json!({ "type": "message", "role": "system",
            "content": [{ "type": "text", "text": "rules" }] })))
        .is_empty());
        assert!(translate(&item(
            json!({ "type": "message", "role": "assistant", "content": [] })
        ))
        .is_empty());
    }

    #[test]
    fn function_call_maps_to_tool_use_with_parsed_args() {
        assert_eq!(
            translate(&item(json!({ "type": "function_call", "name": "shell",
                "call_id": "c1", "arguments": "{\"command\":\"ls\"}" }))),
            vec![AgentUpdate::ToolUse {
                id: "c1".into(),
                name: "shell".into(),
                input: json!({ "command": "ls" }),
            }]
        );
    }

    #[test]
    fn update_plan_function_call_yields_plan_and_tool_use() {
        use crate::session::state::PlanStatus;
        let v = item(
            json!({ "type": "function_call", "name": "update_plan", "call_id": "c1",
            "arguments": "{\"explanation\":\"go\",\"plan\":[{\"step\":\"a\",\"status\":\"completed\"},{\"step\":\"b\",\"status\":\"in_progress\"}]}" }),
        );
        let updates = translate(&v);
        // A plan update PLUS the tool card (parallel to Claude's TodoWrite).
        assert_eq!(updates.len(), 2);
        match &updates[0] {
            AgentUpdate::Plan(plan) => {
                assert_eq!(plan.steps.len(), 2);
                assert_eq!(plan.steps[0].status, PlanStatus::Completed);
                assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
            }
            other => panic!("expected Plan first, got {other:?}"),
        }
        assert!(matches!(&updates[1], AgentUpdate::ToolUse { name, .. } if name == "update_plan"));
    }

    #[test]
    fn function_call_output_maps_to_tool_result() {
        assert_eq!(
            translate(&item(
                json!({ "type": "function_call_output", "call_id": "c1", "output": "done" })
            )),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "c1".into(),
                content: "done".into(),
                is_error: false
            }]
        );
        // object output with a success=false flag is an error
        assert_eq!(
            translate(&item(
                json!({ "type": "function_call_output", "call_id": "c2",
                "output": { "content": "boom", "success": false } })
            )),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "c2".into(),
                content: "boom".into(),
                is_error: true
            }]
        );
    }

    #[test]
    fn custom_apply_patch_call_yields_path_and_diff() {
        // The shape Codex ≥0.14x actually writes: a custom_tool_call whose
        // `input` is the raw patch text.
        let patch =
            "*** Begin Patch\n*** Update File: app/models/user.rb\n@@\n-old\n+new\n*** End Patch";
        let v = item(json!({ "type": "custom_tool_call", "name": "apply_patch",
            "call_id": "c9", "input": patch, "status": "completed" }));
        assert_eq!(
            translate(&v),
            vec![AgentUpdate::ToolUse {
                id: "c9".into(),
                name: "apply_patch".into(),
                input: json!({ "diff": patch, "path": "app/models/user.rb" }),
            }]
        );
    }

    #[test]
    fn custom_apply_patch_multi_file_lists_all_paths() {
        let patch = "*** Begin Patch\n*** Add File: a.rs\n+x\n*** Delete File: b.rs\n*** End Patch";
        let v = item(json!({ "type": "custom_tool_call", "name": "apply_patch",
            "call_id": "c10", "input": patch }));
        match &translate(&v)[0] {
            AgentUpdate::ToolUse { input, .. } => {
                assert_eq!(input["path"], "a.rs");
                assert_eq!(input["paths"], json!(["a.rs", "b.rs"]));
                assert_eq!(input["diff"], patch);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn function_call_apply_patch_normalizes_wrapped_patch_text() {
        // Some builds route apply_patch through a function_call whose JSON
        // arguments wrap the patch under `input`.
        let v = item(
            json!({ "type": "function_call", "name": "apply_patch", "call_id": "c11",
            "arguments": "{\"input\":\"*** Begin Patch\\n*** Update File: src/x.ts\\n@@\\n+hi\\n*** End Patch\"}" }),
        );
        match &translate(&v)[0] {
            AgentUpdate::ToolUse { name, input, .. } => {
                assert_eq!(name, "apply_patch");
                assert_eq!(input["path"], "src/x.ts");
                assert!(input["diff"].as_str().unwrap().contains("+hi"));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn custom_tool_call_output_unwraps_json_envelope() {
        // The output string itself encodes {"output": …, "metadata": {exit_code}}.
        let v = item(json!({ "type": "custom_tool_call_output", "call_id": "c9",
            "output": "{\"output\":\"Success. Updated the following files:\\nM app/models/user.rb\\n\",\"metadata\":{\"exit_code\":0,\"duration_seconds\":0.0}}" }));
        assert_eq!(
            translate(&v),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "c9".into(),
                content: "Success. Updated the following files:\nM app/models/user.rb\n".into(),
                is_error: false,
            }]
        );
        // Non-zero exit_code in the envelope flags the error.
        let err = item(json!({ "type": "custom_tool_call_output", "call_id": "c12",
            "output": "{\"output\":\"boom\",\"metadata\":{\"exit_code\":1}}" }));
        assert_eq!(
            translate(&err),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "c12".into(),
                content: "boom".into(),
                is_error: true,
            }]
        );
    }

    #[test]
    fn function_call_output_plain_json_string_passes_through_verbatim() {
        // An MCP / generic function tool may legitimately answer with a JSON
        // object *string*. It is NOT the codex exec envelope (no `output` +
        // `metadata`), so it must be shown verbatim — not flattened to one
        // field, and a `success: false` field inside it must not be read as a
        // tool failure.
        let payload = "{\"success\":false,\"reason\":\"no deploy yet\"}";
        let v = item(json!({ "type": "function_call_output", "call_id": "c20",
            "output": payload }));
        assert_eq!(
            translate(&v),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "c20".into(),
                content: payload.into(),
                is_error: false,
            }]
        );
        // A JSON string with a `text` field is still not the envelope: keep the
        // full payload, don't collapse it to the `text` field.
        let rich = "{\"text\":\"3 issues\",\"issues\":[1,2,3]}";
        let v = item(json!({ "type": "function_call_output", "call_id": "c21",
            "output": rich }));
        assert_eq!(
            translate(&v),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "c21".into(),
                content: rich.into(),
                is_error: false,
            }]
        );
    }

    #[test]
    fn drain_complete_lines_reassembles_record_split_across_reads() {
        // Simulates the tailer's poll catching a JSONL record mid-write: the
        // first read ends inside the record, the second delivers the rest. The
        // record must come out whole, exactly once.
        let record = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"OK"}]}}"#;
        let full = format!("{record}\n");
        let (a, b) = full.as_bytes().split_at(30);

        let mut carry = Vec::new();
        assert!(
            drain_complete_lines(&mut carry, a).is_empty(),
            "no complete line yet"
        );
        let lines = drain_complete_lines(&mut carry, b);
        assert_eq!(lines, vec![record.to_string()]);
        assert!(carry.is_empty(), "nothing left carried");

        // And the reassembled line parses + translates like any other.
        let value: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(
            translate(&value),
            vec![AgentUpdate::AssistantText("OK".into())]
        );
    }

    #[test]
    fn drain_complete_lines_survives_read_boundary_inside_utf8_char() {
        // The read boundary lands inside a multi-byte character ("é" = 2 bytes).
        // The old read_line-based tailer returned InvalidData here and died for
        // the rest of the session; the byte carry must reassemble it losslessly.
        let full = "{\"k\":\"café\"}\n".as_bytes().to_vec();
        let split = full.iter().position(|&b| b == 0xC3).unwrap() + 1; // mid-'é'
        let (a, b) = full.split_at(split);

        let mut carry = Vec::new();
        assert!(drain_complete_lines(&mut carry, a).is_empty());
        assert_eq!(
            drain_complete_lines(&mut carry, b),
            vec!["{\"k\":\"café\"}".to_string()]
        );
    }

    #[test]
    fn drain_complete_lines_handles_multiple_lines_and_trailing_partial() {
        let mut carry = Vec::new();
        let lines = drain_complete_lines(&mut carry, b"{\"a\":1}\n{\"b\":2}\n{\"c\"");
        assert_eq!(
            lines,
            vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]
        );
        assert_eq!(carry, b"{\"c\"");
        assert_eq!(
            drain_complete_lines(&mut carry, b":3}\n"),
            vec!["{\"c\":3}".to_string()]
        );
    }

    #[test]
    fn paths_eq_tolerates_separators_and_trailing_slash() {
        assert!(paths_eq("/work/repo", "/work/repo/"));
        assert!(paths_eq("C:\\work\\repo", "C:/work/repo"));
        assert!(!paths_eq("/work/repo", "/work/other"));
    }

    #[cfg(windows)]
    #[test]
    fn paths_eq_is_case_insensitive_on_windows() {
        assert!(paths_eq("C:\\Work\\Repo", "c:/work/repo"));
    }

    #[test]
    fn token_count_context_comes_from_last_not_cumulative() {
        // `total_token_usage` is cumulative across the session; the context
        // meter must read `last_token_usage` (what the latest request actually
        // held) or it pins at 100% within a few turns.
        let e = ev(json!({
            "type": "token_count",
            "info": { "total_token_usage": { "input_tokens": 4402946, "output_tokens": 40196, "total_tokens": 4443142 },
                      "last_token_usage": { "input_tokens": 132153, "output_tokens": 399, "total_tokens": 132552 },
                      "model_context_window": 258400 },
            "rate_limits": {
                "primary": { "used_percent": 19.0, "window_minutes": 300, "resets_at": 1783121345 },
                "secondary": { "used_percent": 3.0, "window_minutes": 10080, "resets_at": 1783708145 } }
        }));
        assert_eq!(
            translate(&e),
            vec![
                AgentUpdate::Usage {
                    model: None,
                    input_tokens: Some(4402946),
                    output_tokens: Some(40196),
                    cost_usd: None,
                    context_tokens: Some(132552),
                    context_window: Some(258400),
                },
                AgentUpdate::RateLimits {
                    five_hour_pct: Some(19.0),
                    five_hour_resets_at: Some(1783121345),
                    seven_day_pct: Some(3.0),
                    seven_day_resets_at: Some(1783708145),
                    monthly_pct: None,
                    monthly_resets_at: None,
                },
            ]
        );
    }

    #[test]
    fn token_count_without_last_falls_back_to_in_plus_out() {
        let e = ev(json!({
            "type": "token_count",
            "info": { "total_token_usage": { "input_tokens": 1000, "output_tokens": 200, "total_tokens": 1200 },
                      "model_context_window": 272000 }
        }));
        assert_eq!(
            translate(&e),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(1000),
                output_tokens: Some(200),
                cost_usd: None,
                context_tokens: Some(1200),
                context_window: Some(272000),
            }]
        );
    }

    #[test]
    fn turn_context_yields_model() {
        let v = json!({ "type": "turn_context", "payload": { "model": "gpt-5.5", "cwd": "/w" } });
        assert_eq!(
            translate(&v),
            vec![AgentUpdate::Usage {
                model: Some("gpt-5.5".into()),
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
                context_tokens: None,
                context_window: None,
            }]
        );
    }
}

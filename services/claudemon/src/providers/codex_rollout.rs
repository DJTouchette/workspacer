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
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

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
            let input = payload
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .or_else(|| payload.get("arguments").cloned())
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        "function_call_output" => {
            let tool_use_id = payload
                .get("call_id")
                .or_else(|| payload.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let (content, is_error) = function_output_text(payload.get("output"));
            out.push(AgentUpdate::ToolResult { tool_use_id, content, is_error });
        }
        _ => {} // reasoning, etc. — skipped
    }
    out
}

/// Flatten a `function_call_output.output` (a string, or an object that may carry
/// `content`/`text` and a success flag) into display text + an error flag.
fn function_output_text(output: Option<&Value>) -> (String, bool) {
    match output {
        Some(Value::String(s)) => (s.clone(), false),
        Some(Value::Object(map)) => {
            let is_error = map
                .get("success")
                .and_then(Value::as_bool)
                .map(|ok| !ok)
                .unwrap_or(false);
            let text = map
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| map.get("text").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| Value::Object(map.clone()).to_string());
            (text, is_error)
        }
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
            let input = usage.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64);
            let output = usage.and_then(|u| u.get("output_tokens")).and_then(Value::as_u64);
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
        if cfg!(windows) { t.to_lowercase() } else { t.to_string() }
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
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
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
    let cutoff = since
        .checked_sub(Duration::from_secs(5))
        .unwrap_or(since);
    // ~3 min backstop at 250ms/iter — long enough for any realistic cold start,
    // bounded so a session that never records eventually stops scanning.
    for _ in 0..720 {
        store.get(session_id)?;
        let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = collect_rollouts(&sessions_root)
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
    let mut leftover = String::new();
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
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                offset += n as u64;
                // A trailing partial line (no newline yet) — stash and retry next poll.
                if !line.ends_with('\n') {
                    leftover = std::mem::take(&mut line);
                    offset -= leftover.len() as u64;
                    break;
                }
                let full = if leftover.is_empty() {
                    line.clone()
                } else {
                    format!("{}{}", std::mem::take(&mut leftover), line)
                };
                let trimmed = full.trim();
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
        assert!(translate(&json!({ "type": "session_meta", "payload": { "cwd": "/x" } })).is_empty());
        assert!(translate(&json!({ "type": "turn_context", "payload": {} })).is_empty());
        // reasoning items are not surfaced
        assert!(translate(&item(json!({ "type": "reasoning", "summary": [] }))).is_empty());
        // event_msg messages are dropped (the same text comes via response_item)
        assert!(translate(&ev(json!({ "type": "agent_message", "message": "dup" }))).is_empty());
        assert!(translate(&ev(json!({ "type": "user_message", "message": "dup" }))).is_empty());
    }

    #[test]
    fn task_started_completed_map_to_busy_idle() {
        assert_eq!(translate(&ev(json!({ "type": "task_started" }))), vec![AgentUpdate::Busy]);
        assert_eq!(translate(&ev(json!({ "type": "task_complete" }))), vec![AgentUpdate::Idle]);
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
            "content": [{ "type": "text", "text": "rules" }] }))).is_empty());
        assert!(translate(&item(json!({ "type": "message", "role": "assistant", "content": [] }))).is_empty());
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
    fn function_call_output_maps_to_tool_result() {
        assert_eq!(
            translate(&item(json!({ "type": "function_call_output", "call_id": "c1", "output": "done" }))),
            vec![AgentUpdate::ToolResult { tool_use_id: "c1".into(), content: "done".into(), is_error: false }]
        );
        // object output with a success=false flag is an error
        assert_eq!(
            translate(&item(json!({ "type": "function_call_output", "call_id": "c2",
                "output": { "content": "boom", "success": false } }))),
            vec![AgentUpdate::ToolResult { tool_use_id: "c2".into(), content: "boom".into(), is_error: true }]
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

//! Codex "hybrid" GUI half — tail a Codex TUI session's rollout JSONL and fold
//! it into the shared session model, so a PTY-backed Codex session lights up the
//! GUI conversation view exactly like a Claude session does from its transcript.
//!
//! A hybrid Codex agent is spawned as a normal PTY (`codex` TUI — the Term view,
//! via `/sessions/spawn`). Codex persists that session incrementally to
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`. We discover that
//! file (newest rollout whose `session_meta.cwd` matches, created right after the
//! spawn), then tail it, translating each `event_msg` line into the same
//! [`AgentUpdate`]s the app-server adapter emits — so conversation / mode / usage
//! all flow through the unchanged `apply_updates` path under our canonical id.
//!
//! The `event_msg` stream is the clean, high-level view (one `agent_message` per
//! turn rather than token deltas); the parallel `response_item` lines are
//! redundant for our purposes (and include developer/environment noise), so we
//! ignore everything that isn't an `event_msg`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::BaseDirs;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

use super::{apply_updates, AgentUpdate, UsageAcc};
use crate::session::state::SessionMode;
use crate::session::{ConversationStore, SessionStore};

/// Translate one rollout JSONL line into typed updates. Pure and total: only
/// `event_msg` lines map onto our model; everything else (session_meta,
/// turn_context, response_item) yields nothing.
pub fn translate(value: &Value) -> Vec<AgentUpdate> {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return Vec::new();
    }
    let payload = value.get("payload").unwrap_or(value);
    let ty = payload.get("type").and_then(Value::as_str).unwrap_or("");
    let mut out = Vec::new();
    match ty {
        "task_started" => out.push(AgentUpdate::Busy),
        "task_complete" => out.push(AgentUpdate::Idle),
        "user_message" => {
            if let Some(text) = payload.get("message").and_then(Value::as_str) {
                if !text.is_empty() {
                    out.push(AgentUpdate::UserText(text.to_string()));
                }
            }
        }
        "agent_message" => {
            if let Some(text) = payload.get("message").and_then(Value::as_str) {
                if !text.is_empty() {
                    out.push(AgentUpdate::AssistantText(text.to_string()));
                }
            }
        }
        "token_count" => {
            let usage = payload
                .get("info")
                .and_then(|i| i.get("total_token_usage"));
            let input = usage.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64);
            let output = usage.and_then(|u| u.get("output_tokens")).and_then(Value::as_u64);
            if input.is_some() || output.is_some() {
                out.push(AgentUpdate::Usage {
                    model: None,
                    input_tokens: input,
                    output_tokens: output,
                    cost_usd: None,
                });
            }
        }
        _ => {}
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
/// rollout, modified at/after `since`, whose `session_meta.cwd` matches. Polls
/// for up to ~20s (the TUI writes `session_meta` within the first second, but
/// model/MCP startup can delay the very first flush).
async fn discover_rollout(cwd: &str, since: std::time::SystemTime) -> Option<PathBuf> {
    let sessions_root = codex_home()?.join("sessions");
    // A small grace window before `since` guards against coarse mtime / clock skew.
    let cutoff = since
        .checked_sub(Duration::from_secs(5))
        .unwrap_or(since);
    for _ in 0..80 {
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
        let Some(path) = discover_rollout(&cwd, since).await else {
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

    #[test]
    fn non_event_lines_are_ignored() {
        assert!(translate(&json!({ "type": "session_meta", "payload": { "cwd": "/x" } })).is_empty());
        assert!(translate(&json!({ "type": "response_item", "payload": { "type": "message" } })).is_empty());
        assert!(translate(&json!({ "type": "turn_context", "payload": {} })).is_empty());
    }

    #[test]
    fn task_started_completed_map_to_busy_idle() {
        assert_eq!(translate(&ev(json!({ "type": "task_started" }))), vec![AgentUpdate::Busy]);
        assert_eq!(translate(&ev(json!({ "type": "task_complete" }))), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn user_and_agent_messages_map_to_text() {
        assert_eq!(
            translate(&ev(json!({ "type": "user_message", "message": "hi" }))),
            vec![AgentUpdate::UserText("hi".into())]
        );
        assert_eq!(
            translate(&ev(json!({ "type": "agent_message", "message": "OK", "phase": "final_answer" }))),
            vec![AgentUpdate::AssistantText("OK".into())]
        );
    }

    #[test]
    fn empty_messages_are_skipped() {
        assert!(translate(&ev(json!({ "type": "agent_message", "message": "" }))).is_empty());
        assert!(translate(&ev(json!({ "type": "user_message" }))).is_empty());
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
    fn token_count_maps_to_usage() {
        let e = ev(json!({
            "type": "token_count",
            "info": { "total_token_usage": { "input_tokens": 1000, "output_tokens": 200, "total_tokens": 1200 } }
        }));
        assert_eq!(
            translate(&e),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(1000),
                output_tokens: Some(200),
                cost_usd: None,
            }]
        );
    }
}

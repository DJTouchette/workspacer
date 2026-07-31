//! One-shot headless prompts — a single `claude --print` turn whose answer is
//! returned as text, with no session left behind.
//!
//! This exists for the little jobs *about* a session rather than in it: naming
//! an agent after its first exchange (the desktop's auto-title), and whatever
//! else wants a sentence out of a cheap model. The caller could shell out to
//! `claude --print` itself — but a real headless `claude` fires the user's
//! Claude Code hooks against `/hook`, and `SessionStore::ingest` would register
//! a stray session for every one of them. A ghost row per titled agent is
//! exactly the failure keep-warm already solved, so this reuses that machinery:
//! pin `--session-id <uuid>`, mark the uuid via [`SessionStore::mark_heartbeat`]
//! so `ingest` drops its hooks wholesale, unmark once the child has exited.
//!
//! Same division of labor as spawns and heartbeats: the client resolves the
//! launcher argv (PATH/nvm quirks live there), the daemon runs it.

use std::process::Stdio;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::api::ApiState;

/// Cheapest model that can write a sentence; callers normally pass their own.
const DEFAULT_MODEL: &str = "haiku";
/// Whole-run ceiling: spawn + one small turn + exit.
const DEFAULT_TIMEOUT_SECS: u64 = 45;
const MAX_TIMEOUT_SECS: u64 = 120;
/// Prompts here are summaries of an opening exchange, not documents.
const MAX_PROMPT_CHARS: usize = 8_000;
/// A one-shot answers in a sentence; anything past this is a runaway.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
pub struct OneshotRequest {
    /// Resolved launcher argv, e.g. `["claude"]` or `["cmd.exe","/c","claude"]`.
    pub argv: Vec<String>,
    /// Model alias/id. Absent or empty ⇒ `haiku`.
    #[serde(default)]
    pub model: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct OneshotResponse {
    pub ok: bool,
    /// The model's raw stdout, untouched — callers own their own sanitizing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(OneshotResponse {
            ok: false,
            text: None,
            error: Some(msg.to_string()),
        }),
    )
        .into_response()
}

/// Run the child and read its stdout. Errors carry stderr's tail — a failing
/// `claude` (not logged in, unknown model) says why there.
async fn run_claude_print(
    argv: &[String],
    model: &str,
    prompt: &str,
    session_id: &str,
) -> anyhow::Result<String> {
    use anyhow::Context;

    let (bin, base) = argv.split_first().context("argv must be non-empty")?;
    let mut cmd = Command::new(bin);
    // The prompt goes on STDIN, never in argv. It carries text an agent wrote,
    // and argv[0] is whatever the client resolved — on Windows that can be
    // `cmd.exe /c claude`, where a quote in the prompt ends cmd's quoting and
    // the rest is a command. stdin has no such grammar, so the whole class is
    // gone regardless of the launcher.
    cmd.args(base)
        .args(["--print", "--model", model, "--session-id", session_id]);
    let mut child = cmd
        // Home, not a repo: a one-shot has no project context to pick up.
        .current_dir(super::heartbeat::home_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawning claude for one-shot")?;

    // Write the prompt and CLOSE stdin before reading: `--print` reads to EOF,
    // so holding the handle open would deadlock exactly like an undrained pipe.
    {
        let mut stdin = child.stdin.take().context("one-shot: no stdin")?;
        stdin.write_all(prompt.as_bytes()).await?;
        let _ = stdin.shutdown().await;
    }

    // Both pipes drain CONCURRENTLY. Reading stdout to EOF first would hang any
    // child that fills the 64 KiB stderr buffer: it blocks in write(2), so it
    // never exits, so stdout never reaches EOF, and the whole call burns its
    // timeout instead of returning the answer it already produced.
    let stdout = child.stdout.take().context("one-shot: no stdout")?;
    let stderr = child.stderr.take().context("one-shot: no stderr")?;
    let read_capped = |r: tokio::process::ChildStdout| async move {
        let mut buf = Vec::new();
        let _ = r.take(MAX_OUTPUT_BYTES as u64).read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    };
    let read_capped_err = |r: tokio::process::ChildStderr| async move {
        let mut buf = Vec::new();
        let _ = r.take(MAX_OUTPUT_BYTES as u64).read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).to_string()
    };
    let (out, err_text) = tokio::join!(read_capped(stdout), read_capped_err(stderr));
    let status = child.wait().await.context("waiting for claude")?;
    if !status.success() {
        let tail: String = err_text
            .trim()
            .chars()
            .rev()
            .take(300)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        anyhow::bail!("claude exited {status}: {tail}");
    }
    Ok(out)
}

/// POST /oneshot — one headless turn, text back, no session recorded.
pub async fn handle(State(state): State<ApiState>, Json(req): Json<OneshotRequest>) -> Response {
    if req.argv.is_empty() {
        return bad_request("argv must be non-empty");
    }
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return bad_request("prompt must be non-empty");
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return bad_request("prompt too long");
    }
    let model = match req.model.as_deref() {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => DEFAULT_MODEL.to_string(),
    };
    let timeout_secs = req
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .min(MAX_TIMEOUT_SECS);

    // Pin + suppress: the hooks this run fires must not become a session.
    let sid = uuid::Uuid::new_v4().to_string();
    state.store.mark_heartbeat(&sid);
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        run_claude_print(&req.argv, &model, prompt, &sid),
    )
    .await;
    // The child has exited (or been killed on timeout via kill_on_drop), so
    // every hook it fires has already arrived; stop suppressing the id.
    state.store.unmark_heartbeat(&sid);

    match outcome {
        Ok(Ok(text)) => Json(OneshotResponse {
            ok: true,
            text: Some(text),
            error: None,
        })
        .into_response(),
        Ok(Err(err)) => Json(OneshotResponse {
            ok: false,
            text: None,
            error: Some(format!("{err:#}")),
        })
        .into_response(),
        Err(_) => Json(OneshotResponse {
            ok: false,
            text: None,
            error: Some(format!("timed out after {timeout_secs}s")),
        })
        .into_response(),
    }
}

//! Keep-warm heartbeats — daemon-owned minimal Claude turns that start a
//! subscription 5-hour rate-limit window.
//!
//! The desktop's keep-warm scheduler decides WHEN to warm (config lives in the
//! app); this module owns HOW. A heartbeat speaks the exact same headless
//! stream-json contract the managed stream adapter uses (`--print
//! --input-format stream-json --output-format stream-json`), so there is one
//! place in the codebase that owns the CLI wire contract — if it changes (or
//! moves to an API), the adapter and heartbeats adapt together. Event parsing
//! is shared with the adapter via [`claude_stream::translate`], which is how
//! the ping learns the new window's `resets_at` from its own `rate_limit_event`.
//!
//! Heartbeats are deliberately NOT sessions: they are recorded in their own
//! `heartbeats` table (see store schema v3), so a warm ping can never surface
//! in the sidebar, the recent list, the fleet, or anything else that renders
//! sessions. `POST /heartbeat` runs one; `GET /heartbeats` lists them.

use std::process::Stdio;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::providers::claude_stream::{translate, StreamTotals};
use crate::providers::AgentUpdate;
use crate::store::{Db, HeartbeatRow};

use super::api::ApiState;

/// The cheapest thing that starts a window. `haiku` is the CLI alias for the
/// newest (and cheapest) Haiku — never expanded to a dated id, per repo policy.
const DEFAULT_MODEL: &str = "haiku";
const PING_PROMPT: &str = "Reply with exactly: ok";
/// Whole-run ceiling: spawn + one Haiku turn + exit.
const PING_TIMEOUT_SECS: u64 = 120;
const DEFAULT_LIST_LIMIT: usize = 50;

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    /// Resolved `claude` launcher argv — same division of labor as session
    /// spawns: the client resolves the binary (PATH/nvm quirks live there),
    /// the daemon runs it. E.g. `["claude"]` or `["cmd.exe","/c","claude"]`.
    pub argv: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// POST /heartbeat — run one warm ping, record it, return the stored row.
pub async fn handle(State(state): State<ApiState>, Json(req): Json<HeartbeatRequest>) -> Response {
    if req.argv.is_empty() {
        return (StatusCode::BAD_REQUEST, "argv must be non-empty").into_response();
    }
    let model = req.model.unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let started = std::time::Instant::now();
    let at = time::OffsetDateTime::now_utc().unix_timestamp();

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(PING_TIMEOUT_SECS),
        run_ping(&req.argv, &model),
    )
    .await;
    let (ok, resets_at, error) = match outcome {
        Ok(Ok(resets_at)) => (true, resets_at, None),
        Ok(Err(err)) => (false, None, Some(format!("{err:#}"))),
        Err(_) => (
            false,
            None,
            Some(format!("timed out after {PING_TIMEOUT_SECS}s")),
        ),
    };

    let row = HeartbeatRow {
        id: 0,
        at,
        ok,
        model,
        resets_at,
        duration_ms: Some(started.elapsed().as_millis() as i64),
        error,
    };
    match state.db.insert_heartbeat(&row) {
        Ok(stored) => {
            tracing::info!(ok = stored.ok, resets_at = ?stored.resets_at, "heartbeat");
            Json(stored).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("recording heartbeat: {err:#}"),
        )
            .into_response(),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    pub limit: Option<usize>,
}

/// GET /heartbeats — newest first.
pub async fn list(State(db): State<Db>, Query(q): Query<ListQuery>) -> Response {
    match db.list_heartbeats(q.limit.unwrap_or(DEFAULT_LIST_LIMIT)) {
        Ok(rows) => Json(rows).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("listing heartbeats: {err:#}"),
        )
            .into_response(),
    }
}

/// Spawn one headless turn, return the new 5h window's reset when reported.
///
/// stdin gets a single user message then EOF — in stream-json input mode the
/// CLI runs the queued turn and exits, which is exactly the lifecycle we want.
async fn run_ping(argv: &[String], model: &str) -> anyhow::Result<Option<i64>> {
    use anyhow::Context;

    let (bin, base) = argv.split_first().expect("argv checked non-empty");
    let mut child = Command::new(bin)
        .args(base)
        .args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            model,
        ])
        .current_dir(home_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawning claude for heartbeat")?;

    let mut stdin = child.stdin.take().context("heartbeat: no stdin")?;
    let stdout = child.stdout.take().context("heartbeat: no stdout")?;
    let msg = json!({
        "type": "user",
        "message": { "role": "user", "content": [ { "type": "text", "text": PING_PROMPT } ] }
    });
    stdin.write_all(format!("{msg}\n").as_bytes()).await?;
    let _ = stdin.shutdown().await;
    drop(stdin);

    // Read the turn's events with the adapter's own parser; the window reset
    // arrives as AgentUpdate::RateLimits from the CLI's rate_limit_event.
    let mut resets_at: Option<i64> = None;
    let mut saw_result = false;
    let mut totals = StreamTotals::default();
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        for update in translate(&value, &mut totals) {
            if let AgentUpdate::RateLimits {
                five_hour_resets_at: Some(t),
                ..
            } = update
            {
                resets_at = Some(t);
            }
        }
        if value.get("type").and_then(Value::as_str) == Some("result") {
            saw_result = true;
            break;
        }
    }

    let status = child.wait().await.context("waiting for heartbeat child")?;
    if !status.success() && !saw_result {
        anyhow::bail!("claude exited {status} before completing the turn");
    }
    Ok(resets_at)
}

fn home_dir() -> std::path::PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

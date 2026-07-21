//! Keep-warm heartbeats — daemon-owned minimal turns that start a
//! subscription 5-hour rate-limit window (Claude *and* Codex — both meter
//! usage in windows that only start with the first message).
//!
//! The desktop's keep-warm scheduler decides WHEN to warm (config lives in the
//! app); this module owns HOW, per provider, through the exact wire contract
//! each adapter already maintains:
//!
//!  - **claude**: the managed stream adapter's headless contract (`--print
//!    --input-format stream-json --output-format stream-json`), events parsed
//!    with the shared [`claude_stream::translate`] — the new window's reset
//!    arrives as the turn's own `rate_limit_event`.
//!  - **codex**: a throwaway stdio `codex app-server` (the same handshake the
//!    model-list query uses): `initialize` → `thread/start` → `turn/start`,
//!    reading until `turn/completed`; the window lands as an
//!    `account/rateLimits/updated` snapshot parsed by the adapter's shared
//!    [`rate_limits_from`].
//!
//! One owner per wire contract: if a CLI changes (or goes API-based), fixing
//! its adapter fixes heartbeats with it.
//!
//! Heartbeats are deliberately NOT sessions: they are recorded in their own
//! `heartbeats` table (see store schema v3/v4), so a warm ping can never
//! surface in the sidebar, the recent list, the fleet, or anything else that
//! renders sessions. `POST /heartbeat` runs one; `GET /heartbeats` lists them.

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
use crate::providers::{rate_limits_from, AgentUpdate};
use crate::store::{Db, HeartbeatRow};

use super::api::ApiState;

/// The cheapest thing that starts a Claude window. `haiku` is the CLI alias
/// for the newest (and cheapest) Haiku — never expanded to a dated id.
const CLAUDE_DEFAULT_MODEL: &str = "haiku";
const PING_PROMPT: &str = "Reply with exactly: ok";
/// Whole-run ceiling: spawn + one minimal turn + exit.
const PING_TIMEOUT_SECS: u64 = 120;
const DEFAULT_LIST_LIMIT: usize = 50;

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    /// Resolved launcher argv — same division of labor as session spawns: the
    /// client resolves the binary (PATH/nvm quirks live there), the daemon
    /// runs it. E.g. `["claude"]`, `["codex"]`, or `["cmd.exe","/c","claude"]`.
    pub argv: Vec<String>,
    /// 'claude' (default) or 'codex'.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// POST /heartbeat — run one warm ping, record it, return the stored row.
pub async fn handle(State(state): State<ApiState>, Json(req): Json<HeartbeatRequest>) -> Response {
    if req.argv.is_empty() {
        return (StatusCode::BAD_REQUEST, "argv must be non-empty").into_response();
    }
    let provider = req.provider.as_deref().unwrap_or("claude").to_string();
    let model = match (&req.model, provider.as_str()) {
        (Some(m), _) => m.clone(),
        (None, "claude") => CLAUDE_DEFAULT_MODEL.to_string(),
        // Codex: no forced model — the account default runs the tiny turn.
        (None, _) => String::new(),
    };
    let started = std::time::Instant::now();
    let at = time::OffsetDateTime::now_utc().unix_timestamp();

    let run = async {
        match provider.as_str() {
            "claude" => run_ping_claude(&req.argv, &model).await,
            "codex" => run_ping_codex(&req.argv, &model).await,
            other => anyhow::bail!("unsupported heartbeat provider '{other}'"),
        }
    };
    let outcome =
        tokio::time::timeout(std::time::Duration::from_secs(PING_TIMEOUT_SECS), run).await;
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
        provider,
        model,
        resets_at,
        duration_ms: Some(started.elapsed().as_millis() as i64),
        error,
    };
    match state.db.insert_heartbeat(&row) {
        Ok(stored) => {
            tracing::info!(
                ok = stored.ok,
                provider = %stored.provider,
                resets_at = ?stored.resets_at,
                "heartbeat"
            );
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

/// Spawn one headless Claude turn, return the new 5h window's reset when
/// reported.
///
/// stdin gets a single user message then EOF — in stream-json input mode the
/// CLI runs the queued turn and exits, which is exactly the lifecycle we want.
async fn run_ping_claude(argv: &[String], model: &str) -> anyhow::Result<Option<i64>> {
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

/// Spawn one Codex turn through a throwaway stdio `codex app-server` — the
/// same handshake the adapter's model-list query uses. Returns the 5h reset
/// from the run's `account/rateLimits/updated` snapshot, when one arrives.
async fn run_ping_codex(argv: &[String], model: &str) -> anyhow::Result<Option<i64>> {
    use anyhow::Context;

    let (bin, base) = argv.split_first().expect("argv checked non-empty");
    let mut child = Command::new(bin)
        .args(base)
        .arg("app-server")
        .current_dir(home_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} app-server` for heartbeat"))?;

    let mut stdin = child.stdin.take().context("codex heartbeat: no stdin")?;
    let stdout = child.stdout.take().context("codex heartbeat: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    let write = |v: Value| {
        let mut line = v.to_string();
        line.push('\n');
        line
    };
    stdin
        .write_all(
            write(json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "clientInfo": { "name": "workspacer", "version": "0.1" } }
            }))
            .as_bytes(),
        )
        .await?;
    stdin
        .write_all(
            write(json!({
                "jsonrpc": "2.0", "id": 2, "method": "thread/start",
                "params": { "cwd": home_dir().to_string_lossy() }
            }))
            .as_bytes(),
        )
        .await?;
    stdin.flush().await?;

    let mut thread_id: Option<String> = None;
    let mut resets_at: Option<i64> = None;
    let mut turn_sent = false;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // JSON-RPC errors to our requests are fatal (bad thread, bad turn).
        if value.get("id").is_some() {
            if let Some(err) = value.get("error") {
                anyhow::bail!("codex heartbeat rpc error: {err}");
            }
        }
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        let params = value.get("params").unwrap_or(&Value::Null);

        // The thread id arrives via the thread/start result or notification.
        if thread_id.is_none() {
            thread_id =
                crate::providers::codex::thread_id_of(value.get("result")).or_else(
                    || match method {
                        "thread/started" => crate::providers::codex::thread_id_of(Some(params)),
                        _ => None,
                    },
                );
            if let Some(tid) = &thread_id {
                if !turn_sent {
                    turn_sent = true;
                    if !model.is_empty() {
                        stdin
                            .write_all(
                                write(json!({
                                    "jsonrpc": "2.0", "id": 3, "method": "thread/settings/update",
                                    "params": { "threadId": tid, "model": model }
                                }))
                                .as_bytes(),
                            )
                            .await?;
                    }
                    stdin
                        .write_all(
                            write(json!({
                                "jsonrpc": "2.0", "id": 4, "method": "turn/start",
                                "params": { "threadId": tid,
                                            "input": [ { "type": "text", "text": PING_PROMPT } ] }
                            }))
                            .as_bytes(),
                        )
                        .await?;
                    stdin.flush().await?;
                }
            }
        }

        // The 5h window snapshot — parsed by the adapter's shared helper.
        if method == "account/rateLimits/updated" {
            let snap = params.get("rateLimits").unwrap_or(params);
            if let Some(AgentUpdate::RateLimits {
                five_hour_resets_at: Some(t),
                ..
            }) = rate_limits_from(snap)
            {
                resets_at = Some(t);
            }
        }
        match method {
            "turn/completed" => break,
            "turn/failed" => {
                let msg = params
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("turn failed");
                anyhow::bail!("codex heartbeat turn failed: {msg}");
            }
            _ => {}
        }
    }
    if !turn_sent {
        anyhow::bail!("codex app-server closed before starting a thread");
    }
    let _ = child.start_kill();
    Ok(resets_at)
}

fn home_dir() -> std::path::PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

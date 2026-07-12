//! `POST /sessions/spawn` — spawn a command in a PTY *inside* the daemon.
//!
//! The daemon pre-assigns a UUID, registers it as a "pending spawn" keyed by
//! cwd, and wires up the PTY's reader/writer to the same per-session output
//! buffer + bytes broadcast that `wrapper_ws` uses for external wrappers.
//!
//! When claude later posts `SessionStart` with its own session_id, the store
//! aliases that id to ours (see `SessionStore::ingest`), so every endpoint
//! that takes a session_id keeps working — clients only ever see the id we
//! handed back from this endpoint.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use portable_pty::PtySize;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::protocol::WrapperMessage;
use crate::session::store::WrapperHandle;
use crate::session::{ConversationStore, SessionStore};
use crate::wrapper::pty;

#[derive(Debug, Deserialize)]
pub struct SpawnPayload {
    /// Command + args, e.g. `["claude", "--resume", "..."]`.
    pub argv: Vec<String>,
    /// Working directory for the child.
    pub cwd: String,
    /// PTY dimensions. Defaults to 80x24 if omitted.
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// Extra env vars merged on top of the daemon's environment.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Caller-pinned session id. When the caller launches
    /// `claude --session-id <uuid>` it passes the same uuid here, so our id,
    /// claude's id, and the transcript filename (`<uuid>.jsonl`) all agree — no
    /// cwd-based alias guessing, correct even with many sessions in one cwd.
    #[serde(default)]
    pub session_id: Option<String>,
    /// When set (e.g. `"codex"`), this PTY is a managed agent's own TUI and we
    /// additionally tail its rollout transcript to drive the GUI conversation
    /// view — a "hybrid" session that has both a terminal and a structured GUI.
    #[serde(default)]
    pub rollout_provider: Option<String>,
}

pub async fn handle(
    State(store): State<SessionStore>,
    State(conv): State<ConversationStore>,
    Json(payload): Json<SpawnPayload>,
) -> impl IntoResponse {
    if payload.argv.is_empty() {
        return (StatusCode::BAD_REQUEST, "argv must not be empty").into_response();
    }

    // Prefer the caller-pinned id (matches `claude --session-id <uuid>`); fall
    // back to a fresh one for callers that don't pin (e.g. plain shells).
    let session_id = payload
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let cwd = payload.cwd.clone();
    let cols = payload.cols.unwrap_or(80);
    let rows = payload.rows.unwrap_or(24);

    let pty_handle = match pty::spawn(
        &payload.argv,
        &cwd,
        PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        },
        &payload.env,
    ) {
        Ok(h) => Arc::new(h),
        Err(err) => {
            tracing::error!(?err, "spawn failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawn failed: {err}"),
            )
                .into_response();
        }
    };

    // daemon → child input pump (mpsc<WrapperMessage> → PTY)
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<WrapperMessage>();
    let pty_for_input = pty_handle.clone();
    let input_task = tokio::spawn(async move {
        while let Some(msg) = input_rx.recv().await {
            match msg {
                WrapperMessage::Input { bytes } => {
                    if let Ok(decoded) = B64.decode(bytes.as_bytes()) {
                        if let Err(err) = pty::write_bytes(&pty_for_input, &decoded).await {
                            // A failed write means input (possibly a chat send
                            // the store already reported delivered) was lost —
                            // make it visible instead of vanishing.
                            tracing::warn!(?err, len = decoded.len(), "PTY input write failed");
                        }
                    }
                }
                WrapperMessage::Signal { signal } => match signal {
                    // Interactive interrupt: Ctrl-C byte through the tty.
                    crate::protocol::Signal::Sigint => {
                        let _ = pty::write_bytes(&pty_for_input, b"\x03").await;
                    }
                    // Terminate / kill: real process signal so a runaway session stops.
                    other => {
                        if let Err(err) = pty::signal_child(&pty_for_input, other) {
                            tracing::warn!(?err, "signal delivery failed");
                        }
                    }
                },
                WrapperMessage::Resize { cols, rows } => {
                    let _ = pty::resize(&pty_for_input, cols, rows).await;
                }
                _ => {}
            }
        }
    });

    // child → store output pump (PTY → output buffer + bytes broadcast)
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    if let Err(err) = pty::start_reader(&pty_handle, out_tx) {
        tracing::error!(?err, "start_reader failed");
        // Kill the child and abort the input pump so nothing leaks.
        let _ = pty::signal_child(&pty_handle, crate::protocol::Signal::Sigkill);
        input_task.abort();
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not start PTY reader",
        )
            .into_response();
    }
    let store_for_reader = store.clone();
    let session_for_reader = session_id.clone();
    let cwd_for_reader = cwd.clone();
    tokio::spawn(async move {
        while let Some(chunk) = out_rx.recv().await {
            store_for_reader
                .record_output(&session_for_reader, &chunk)
                .await;
        }
        // Reader EOF — child exited. Reap it (so it doesn't linger as a zombie)
        // and make sure we don't leak a pending spawn entry if SessionStart never
        // fired (e.g. claude crashed at startup).
        store_for_reader.reap_pty(&session_for_reader);
        store_for_reader.drop_pending_spawn(&session_for_reader, &cwd_for_reader);
        tracing::info!(session = %session_for_reader, "in-daemon PTY reader ended");
    });

    store.register_pty(&session_id, pty_handle.clone());
    store.register_spawn(&session_id, &cwd, WrapperHandle { tx: input_tx });
    store.note_term_size(&session_id, cols, rows);
    tracing::info!(%session_id, %cwd, argv=?payload.argv, "spawned in-daemon PTY");

    // Hybrid agents (e.g. Codex): the PTY above is the agent's own TUI (the Term
    // view); additionally tail its rollout transcript so the GUI conversation
    // view is populated from the same live session.
    if payload.rollout_provider.as_deref() == Some("codex") {
        crate::providers::codex_rollout::spawn_tailer(
            store.clone(),
            conv.clone(),
            session_id.clone(),
            cwd.clone(),
        );
    }

    Json(json!({ "session_id": session_id, "cwd": cwd })).into_response()
}

/// `POST /sessions/spawn-managed` — spawn a *managed* (adapter-driven) session.
///
/// Unlike `/sessions/spawn` (a PTY), this runs a provider's own machine
/// interface and translates its events into the session model. Currently only
/// `opencode` (drives `opencode serve` + its `/event` SSE). The session id is
/// registered up front and returned immediately; the adapter boots in the
/// background, so the UI shows the agent while the server starts.
#[derive(Debug, Deserialize)]
pub struct SpawnManagedPayload {
    /// Provider backend: `opencode`, `codex`, `pi`, or `claude` (the headless
    /// stream-json transport — the PTY path stays on `/sessions/spawn`).
    pub provider: String,
    /// Working directory for the agent.
    pub cwd: String,
    /// Optional model override (provider-specific id).
    #[serde(default)]
    pub model: Option<String>,
    /// Optional reasoning-effort level. Codex maps it to the
    /// `model_reasoning_effort` config override; other providers ignore it.
    #[serde(default)]
    pub effort: Option<String>,
    /// Resolved launcher binary (the desktop resolves it on PATH); falls back to
    /// the provider name.
    #[serde(default)]
    pub bin: Option<String>,
    /// YOLO / skip-approvals: auto-approve every command and file change instead
    /// of surfacing them for the user's decision.
    #[serde(default)]
    pub yolo: bool,
    /// Workspacer MCP facade URL to register with the provider (supervisors).
    #[serde(default)]
    pub mcp: Option<String>,
    /// Role instructions to prepend to the agent's first turn (supervisors).
    #[serde(default)]
    pub instructions: Option<String>,
    /// Caller-pinned session id, so every client converges on one card.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Codex only: `"stream"` runs headless (GUI-only, no native TUI PTY — the
    /// daemon starts the thread itself via `thread/start`), mirroring Claude's
    /// stream transport. Anything else (or absent) is the default hybrid.
    #[serde(default)]
    pub transport: Option<String>,
    /// Claude only: initial permission mode, in the CLI's own vocabulary
    /// (`acceptEdits`, `plan`, `bypassPermissions`, …) — `--permission-mode`.
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Claude only: resume this prior session (`--resume <id>`) instead of
    /// starting fresh with a pinned id.
    #[serde(default)]
    pub resume: Option<String>,
    /// Claude only: extra argv appended verbatim (escape hatch for CLI flags
    /// the payload doesn't model).
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Claude only: extra env vars merged on top of the daemon's environment
    /// (e.g. a Claude profile's `CLAUDE_CONFIG_DIR`) — same semantics as
    /// `/sessions/spawn`'s `env`.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

pub async fn handle_managed(
    State(store): State<SessionStore>,
    State(conv): State<ConversationStore>,
    Json(payload): Json<SpawnManagedPayload>,
) -> impl IntoResponse {
    if !matches!(
        payload.provider.as_str(),
        "opencode" | "codex" | "pi" | "claude"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            format!("unsupported managed provider: {}", payload.provider),
        )
            .into_response();
    }
    // Resuming a claude stream session keeps the CLI's *prior* session id (see
    // the claude_stream module contract — `--resume` is not re-pinnable), so an
    // unpinned resume must reuse that id as the row id: otherwise every hook
    // arrives under the prior id and drives a stale/ghost PTY row while the
    // stream row never sees its transcript_path.
    let session_id = payload
        .session_id
        .or_else(|| payload.resume.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let bin = payload.bin.unwrap_or_else(|| payload.provider.clone());

    store.register_managed(&session_id, &payload.cwd, &payload.provider);
    let facade = crate::providers::Facade {
        mcp_url: payload.mcp.clone(),
        instructions: payload.instructions.clone(),
    };
    match payload.provider.as_str() {
        // Claude over the headless stream-json transport (2nd claude
        // transport; the PTY path on `/sessions/spawn` is untouched). The
        // transport is stamped *before* the driver starts so `ingest`'s hooks
        // guard and the first snapshot already see it.
        "claude" => {
            store.set_transport(&session_id, crate::session::state::Transport::Stream);
            crate::providers::claude_stream::spawn_session(
                store.clone(),
                conv.clone(),
                crate::providers::claude_stream::SpawnConfig {
                    session_id: session_id.clone(),
                    cwd: payload.cwd.clone(),
                    bin,
                    model: payload.model.clone(),
                    effort: payload.effort.clone(),
                    permission_mode: payload.permission_mode.clone(),
                    resume: payload.resume.clone(),
                    extra_args: payload.extra_args.clone(),
                    env: payload.env.clone(),
                    yolo: payload.yolo,
                    facade,
                },
            );
        }
        "opencode" => crate::providers::opencode::spawn_session(
            store.clone(),
            conv.clone(),
            session_id.clone(),
            payload.cwd.clone(),
            payload.model.clone(),
            bin,
            payload.yolo,
            facade,
        ),
        "codex" => {
            // Resume: rejoin the prior life's app-server thread (persisted in
            // the codex-threads sidecar) and pre-seed the conversation from its
            // rollout, so the pane shows the history immediately. Resume is
            // headless-only — the TUI can't rejoin an arbitrary thread — so it
            // forces the stream transport.
            let resume_thread = payload
                .resume
                .as_deref()
                .and_then(crate::providers::codex_rollout::thread_for);
            if payload.resume.is_some() && resume_thread.is_none() {
                tracing::warn!(session = %session_id, "codex resume requested but no thread recorded — starting fresh");
            }
            let headless =
                payload.transport.as_deref() == Some("stream") || resume_thread.is_some();
            if headless {
                // Stamped before the driver starts (like claude-stream above)
                // so every snapshot/frame gates the pane GUI-only from the
                // session's first instant.
                store.set_transport(&session_id, crate::session::state::Transport::Stream);
            }
            if let Some(tid) = &resume_thread {
                // Seed only when the conversation isn't already resident (a
                // resume in the same daemon life would otherwise duplicate it).
                let empty = conv
                    .snapshot(&session_id)
                    .is_none_or(|(_, items)| items.is_empty());
                if empty {
                    if let Some(path) = crate::providers::codex_rollout::rollout_for_thread(tid) {
                        let items = crate::providers::codex_rollout::replay_conversation(&path);
                        if !items.is_empty() {
                            conv.push(&session_id, items);
                        }
                    }
                }
            }
            crate::providers::codex::spawn_session(
                store.clone(),
                conv.clone(),
                session_id.clone(),
                payload.cwd.clone(),
                payload.model.clone(),
                payload.effort.clone(),
                bin,
                payload.yolo,
                headless,
                resume_thread,
                facade,
            )
        }
        "pi" => crate::providers::pi::spawn_session(
            store.clone(),
            conv.clone(),
            session_id.clone(),
            payload.cwd.clone(),
            payload.model.clone(),
            bin,
            payload.yolo,
            facade,
        ),
        _ => unreachable!(),
    }
    tracing::info!(%session_id, provider = %payload.provider, cwd = %payload.cwd, "spawned managed session");

    Json(json!({ "session_id": session_id, "cwd": payload.cwd })).into_response()
}

/// Query params for `GET /providers/:provider/models`.
#[derive(Debug, Deserialize)]
pub struct ProviderModelsQuery {
    /// Working directory to run the provider CLI in (it reads project/global
    /// config + auth from there). Defaults to the daemon's cwd.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Resolved launcher binary (the desktop resolves it on PATH); falls back to
    /// the provider name.
    #[serde(default)]
    pub bin: Option<String>,
}

/// `GET /providers/:provider/models` — list the models a managed provider can
/// launch with, live-queried from its own CLI/server (so the picker always
/// matches what the installed binary actually offers). Returns
/// `{ "models": [{ "id", "label", "default" }] }`; an empty list is valid (e.g.
/// Pi with no authed providers) and the UI falls back to free-text entry.
pub async fn handle_provider_models(
    Path(provider): Path<String>,
    Query(q): Query<ProviderModelsQuery>,
) -> impl IntoResponse {
    let bin = q.bin.unwrap_or_else(|| provider.clone());
    let cwd = q.cwd.unwrap_or_else(|| ".".to_string());
    let result = match provider.as_str() {
        "opencode" => crate::providers::opencode::list_models(&bin, &cwd).await,
        "codex" => crate::providers::codex::list_models(&bin, &cwd).await,
        "pi" => crate::providers::pi::list_models(&bin, &cwd).await,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                format!("unsupported managed provider: {other}"),
            )
                .into_response();
        }
    };
    match result {
        Ok(models) => Json(json!({ "models": models })).into_response(),
        Err(err) => {
            tracing::warn!(?err, %provider, "listing provider models failed");
            (StatusCode::BAD_GATEWAY, format!("{err}")).into_response()
        }
    }
}

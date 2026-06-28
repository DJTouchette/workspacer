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

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
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
}

pub async fn handle(
    State(store): State<SessionStore>,
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
        PtySize { cols, rows, pixel_width: 0, pixel_height: 0 },
        &payload.env,
    ) {
        Ok(h) => Arc::new(h),
        Err(err) => {
            tracing::error!(?err, "spawn failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("spawn failed: {err}"))
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
                        let _ = pty::write_bytes(&pty_for_input, &decoded).await;
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
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not start PTY reader")
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
        // Reader EOF — child exited. Make sure we don't leak a pending spawn
        // entry if SessionStart never fired (e.g. claude crashed at startup).
        store_for_reader.drop_pending_spawn(&session_for_reader, &cwd_for_reader);
        tracing::info!(session = %session_for_reader, "in-daemon PTY reader ended");
    });

    store.register_spawn(&session_id, &cwd, WrapperHandle { tx: input_tx });
    tracing::info!(%session_id, %cwd, argv=?payload.argv, "spawned in-daemon PTY");

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
    /// Provider backend. Only `opencode` is supported today.
    pub provider: String,
    /// Working directory for the agent.
    pub cwd: String,
    /// Optional model override (provider-specific id).
    #[serde(default)]
    pub model: Option<String>,
    /// Resolved launcher binary (the desktop resolves it on PATH); falls back to
    /// the provider name.
    #[serde(default)]
    pub bin: Option<String>,
    /// Caller-pinned session id, so every client converges on one card.
    #[serde(default)]
    pub session_id: Option<String>,
}

pub async fn handle_managed(
    State(store): State<SessionStore>,
    State(conv): State<ConversationStore>,
    Json(payload): Json<SpawnManagedPayload>,
) -> impl IntoResponse {
    if payload.provider != "opencode" {
        return (
            StatusCode::BAD_REQUEST,
            format!("unsupported managed provider: {}", payload.provider),
        )
            .into_response();
    }
    let session_id = payload
        .session_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let bin = payload.bin.unwrap_or_else(|| "opencode".to_string());

    store.register_managed(&session_id, &payload.cwd);
    crate::providers::opencode::spawn_session(
        store.clone(),
        conv.clone(),
        session_id.clone(),
        payload.cwd.clone(),
        payload.model.clone(),
        bin,
    );
    tracing::info!(%session_id, cwd = %payload.cwd, "spawned managed opencode session");

    Json(json!({ "session_id": session_id, "cwd": payload.cwd })).into_response()
}

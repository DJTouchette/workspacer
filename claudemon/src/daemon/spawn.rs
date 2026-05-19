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
use crate::session::SessionStore;
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
}

pub async fn handle(
    State(store): State<SessionStore>,
    Json(payload): Json<SpawnPayload>,
) -> impl IntoResponse {
    if payload.argv.is_empty() {
        return (StatusCode::BAD_REQUEST, "argv must not be empty").into_response();
    }

    let session_id = Uuid::new_v4().to_string();
    let cwd = payload.cwd.clone();
    let cols = payload.cols.unwrap_or(80);
    let rows = payload.rows.unwrap_or(24);

    // pty::spawn currently reads std::env::vars() unconditionally; we apply
    // overrides by mutating the process env. That's racy if two spawns happen
    // at once with overlapping keys, but the alternative is wider changes to
    // pty::spawn. For now we accept the trade-off and document it.
    let _env_guard = ApplyEnv::apply(&payload.env);

    let pty_handle = match pty::spawn(
        &payload.argv,
        &cwd,
        PtySize { cols, rows, pixel_width: 0, pixel_height: 0 },
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
    tokio::spawn(async move {
        while let Some(msg) = input_rx.recv().await {
            match msg {
                WrapperMessage::Input { bytes } => {
                    if let Ok(decoded) = B64.decode(bytes.as_bytes()) {
                        let _ = pty::write_bytes(&pty_for_input, &decoded).await;
                    }
                }
                WrapperMessage::Signal { signal } if signal == "SIGINT" => {
                    let _ = pty::write_bytes(&pty_for_input, b"\x03").await;
                }
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

/// Temporarily apply env overrides to the process environment. Restored on
/// drop. Used because `pty::spawn` reads `std::env::vars()` to populate the
/// child's environment.
struct ApplyEnv {
    restore: Vec<(String, Option<String>)>,
}

impl ApplyEnv {
    fn apply(overrides: &HashMap<String, String>) -> Self {
        let mut restore = Vec::with_capacity(overrides.len());
        for (k, v) in overrides {
            restore.push((k.clone(), std::env::var(k).ok()));
            std::env::set_var(k, v);
        }
        Self { restore }
    }
}

impl Drop for ApplyEnv {
    fn drop(&mut self) {
        for (k, prev) in self.restore.drain(..) {
            match prev {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
    }
}

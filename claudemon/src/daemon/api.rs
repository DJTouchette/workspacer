use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tower_http::cors::CorsLayer;

use crate::protocol::WrapperMessage;
use crate::session::{transcript, SessionMode, SessionStore};
use crate::store::items::{ItemAction, ItemBroadcaster, ItemChange, ListFilter};
use crate::store::Db;

/// Bundled state for the v2 API router. FromRef lets each handler extract
/// just the slice it needs, so v1 handlers keep their `State<SessionStore>`
/// signature.
#[derive(Clone)]
pub struct ApiState {
    pub store: SessionStore,
    pub db: Db,
    pub items: ItemBroadcaster,
}

impl FromRef<ApiState> for SessionStore {
    fn from_ref(state: &ApiState) -> Self {
        state.store.clone()
    }
}

impl FromRef<ApiState> for Db {
    fn from_ref(state: &ApiState) -> Self {
        state.db.clone()
    }
}

impl FromRef<ApiState> for ItemBroadcaster {
    fn from_ref(state: &ApiState) -> Self {
        state.items.clone()
    }
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/sessions", get(list_sessions))
        .route("/sessions/spawn", post(crate::daemon::spawn::handle))
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id/input", post(post_input))
        .route("/sessions/:id/message", post(post_message))
        .route("/sessions/:id/approve", post(post_approve))
        .route("/sessions/:id/answer", post(post_answer))
        .route("/sessions/:id/decide", post(post_decide))
        .route("/sessions/:id/gate", post(post_gate))
        .route("/sessions/:id/signal", post(post_signal))
        .route("/sessions/:id/resize", post(post_resize))
        .route("/sessions/:id/output", get(get_output))
        .route("/sessions/:id/stream", get(stream_bytes))
        .route("/sessions/:id/transcript", get(get_transcript))
        .route("/events", get(event_stream))
        .route("/hooks/stream", get(hook_stream))
        .route("/items", get(list_items))
        .route("/items/stream", get(items_stream))
        .route("/items/:id", get(get_item_by_id))
        .route("/items/:id/action", post(post_item_action))
        .route("/wrapper/:id", get(crate::daemon::wrapper_ws::upgrade))
        .route("/git/status", get(crate::daemon::git::get_status))
        .route("/git/diff", get(crate::daemon::git::get_diff))
        .route("/git/stage", post(crate::daemon::git::post_stage))
        .route("/git/unstage", post(crate::daemon::git::post_unstage))
        .route("/git/commit", post(crate::daemon::git::post_commit))
        .route("/git/push", post(crate::daemon::git::post_push))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn list_sessions(State(store): State<SessionStore>) -> impl IntoResponse {
    Json(store.list())
}

async fn get_session(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match store.get(&id) {
        Some(state) => Json(state).into_response(),
        None => (StatusCode::NOT_FOUND, "session not found").into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct InputPayload {
    /// Plain text (will be sent verbatim).
    text: Option<String>,
    /// base64-encoded bytes (alternative to `text`).
    bytes_b64: Option<String>,
    /// If true and `text` is set, append `\n`.
    #[serde(default = "default_true")]
    newline: bool,
}

fn default_true() -> bool {
    true
}

async fn post_input(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<InputPayload>,
) -> impl IntoResponse {
    let Some(handle) = store.wrapper(&id) else {
        return (StatusCode::NOT_FOUND, "no wrapper attached to that session").into_response();
    };

    let raw = if let Some(b64) = payload.bytes_b64 {
        match B64.decode(b64.as_bytes()) {
            Ok(v) => v,
            Err(_) => return (StatusCode::BAD_REQUEST, "bad base64").into_response(),
        }
    } else if let Some(mut text) = payload.text {
        if payload.newline && !text.ends_with('\n') {
            text.push('\n');
        }
        text.into_bytes()
    } else {
        return (StatusCode::BAD_REQUEST, "expected `text` or `bytes_b64`").into_response();
    };

    if handle
        .tx
        .send(WrapperMessage::Input { bytes: B64.encode(&raw) })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    Json(json!({ "ok": true, "bytes": raw.len() })).into_response()
}

#[derive(Debug, Deserialize)]
struct MessagePayload {
    text: String,
}

async fn post_message(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<MessagePayload>,
) -> impl IntoResponse {
    let Some(state) = store.get(&id) else {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    };
    if state.mode != SessionMode::Input {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "session is not accepting chat input",
                "mode": state.mode,
                "expected": "input",
            })),
        )
            .into_response();
    }
    let Some(handle) = store.wrapper(&id) else {
        return (StatusCode::NOT_FOUND, "no wrapper attached to that session").into_response();
    };

    // Carriage return is what Claude Code's input field treats as submit.
    let mut bytes = payload.text.into_bytes();
    if !bytes.last().is_some_and(|b| *b == b'\r' || *b == b'\n') {
        bytes.push(b'\r');
    }
    if handle
        .tx
        .send(WrapperMessage::Input { bytes: B64.encode(&bytes) })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    Json(json!({ "ok": true, "bytes": bytes.len() })).into_response()
}

#[derive(Debug, Deserialize)]
struct ApprovePayload {
    /// `"yes"` → `{decision:"approve"}`, `"no"` → `{decision:"block"}`.
    /// `"always"` is treated as `"yes"` for hook purposes — hooks don't
    /// have a "remember this" channel; persistence is a TUI-only concept.
    decision: Option<String>,
    /// Optional reason returned to Claude with a block (shows up in the
    /// assistant's context as "the tool was blocked because ...").
    reason: Option<String>,
}

async fn post_approve(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<ApprovePayload>,
) -> impl IntoResponse {
    let Some(state) = store.get(&id) else {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    };
    if state.mode != SessionMode::Approval {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "session is not awaiting approval",
                "mode": state.mode,
                "expected": "approval",
            })),
        )
            .into_response();
    }
    let hook_decision = match payload.decision.as_deref() {
        Some("yes") | Some("always") => json!({"decision": "approve"}),
        Some("no") => {
            let mut obj = json!({"decision": "block"});
            if let Some(reason) = payload.reason {
                obj["reason"] = json!(reason);
            }
            obj
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "expected `decision` (yes|no|always)",
            )
                .into_response();
        }
    };
    if !store.resolve_decision(&id, hook_decision.clone()) {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "no parked decision to resolve",
                "hint": "enable the gate first: POST /sessions/:id/gate {\"on\":true}",
            })),
        )
            .into_response();
    }
    Json(json!({ "ok": true, "decision": hook_decision })).into_response()
}

#[derive(Debug, Deserialize)]
struct DecidePayload {
    /// Raw hook decision body returned to Claude Code verbatim, e.g.
    /// `{"decision":"approve"}` or `{"continue":false,"reason":"..."}`.
    /// Bypasses /approve's opinionated mapping for callers that need
    /// fine-grained control.
    body: Value,
}

async fn post_decide(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<DecidePayload>,
) -> impl IntoResponse {
    if !store.resolve_decision(&id, payload.body.clone()) {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "no parked decision to resolve",
                "hint": "the gate must be on for this session AND a PreToolUse hook must have arrived",
            })),
        )
            .into_response();
    }
    Json(json!({ "ok": true, "body": payload.body })).into_response()
}

#[derive(Debug, Deserialize)]
struct GatePayload {
    on: bool,
}

async fn post_gate(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<GatePayload>,
) -> impl IntoResponse {
    if store.get(&id).is_none() {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    }
    store.set_gate(&id, payload.on);
    Json(json!({
        "ok": true,
        "session_id": id,
        "gate_enabled": payload.on,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct AnswerPayload {
    /// 1-indexed option for the current (or only) question.
    option: Option<u8>,
    /// Free-form text answer (when the picker has an "Other" / text entry).
    text: Option<String>,
    /// For multi-question prompts: one answer per question in order. Each
    /// entry is either an option number (as a string like `"2"`) or
    /// free-form text. Sent back-to-back with `\r` between.
    answers: Option<Vec<String>>,
}

async fn post_answer(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<AnswerPayload>,
) -> impl IntoResponse {
    let Some(state) = store.get(&id) else {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    };
    if state.mode != crate::session::SessionMode::Question {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "session is not asking a question",
                "mode": state.mode,
                "expected": "question",
            })),
        )
            .into_response();
    }
    let Some(handle) = store.wrapper(&id) else {
        return (StatusCode::NOT_FOUND, "no wrapper attached").into_response();
    };

    let mut bytes: Vec<u8> = Vec::new();
    if let Some(answers) = payload.answers {
        for ans in answers {
            bytes.extend_from_slice(ans.as_bytes());
            bytes.push(b'\r');
        }
    } else if let Some(opt) = payload.option {
        if !(1..=9).contains(&opt) {
            return (StatusCode::BAD_REQUEST, "option must be 1-9").into_response();
        }
        bytes.extend_from_slice(opt.to_string().as_bytes());
        bytes.push(b'\r');
    } else if let Some(text) = payload.text {
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(b'\r');
    } else {
        return (
            StatusCode::BAD_REQUEST,
            "expected `option`, `text`, or `answers`",
        )
            .into_response();
    }

    if handle
        .tx
        .send(WrapperMessage::Input { bytes: B64.encode(&bytes) })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    Json(json!({ "ok": true, "bytes": bytes.len() })).into_response()
}

#[derive(Debug, Deserialize)]
struct SignalPayload {
    signal: String,
}

async fn post_signal(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<SignalPayload>,
) -> impl IntoResponse {
    let Some(handle) = store.wrapper(&id) else {
        return (StatusCode::NOT_FOUND, "no wrapper attached").into_response();
    };
    if handle
        .tx
        .send(WrapperMessage::Signal { signal: payload.signal.clone() })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    Json(json!({ "ok": true, "signal": payload.signal })).into_response()
}

#[derive(Debug, Deserialize)]
struct ResizePayload {
    cols: u16,
    rows: u16,
}

async fn post_resize(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<ResizePayload>,
) -> impl IntoResponse {
    let Some(handle) = store.wrapper(&id) else {
        return (StatusCode::NOT_FOUND, "no wrapper attached").into_response();
    };
    if handle
        .tx
        .send(WrapperMessage::Resize { cols: payload.cols, rows: payload.rows })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    Json(json!({ "ok": true, "cols": payload.cols, "rows": payload.rows })).into_response()
}

async fn get_output(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match store.output_snapshot(&id).await {
        Some(bytes) => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Json(json!({ "bytes": bytes.len(), "text": text })).into_response()
        }
        None => (StatusCode::NOT_FOUND, "no buffer for that session").into_response(),
    }
}

async fn stream_bytes(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some((snapshot, rx)) = store.snapshot_and_subscribe(&id).await else {
        return (StatusCode::NOT_FOUND, "no buffer for that session").into_response();
    };

    // First event replays the ring buffer so reconnecting/attaching clients
    // see prior terminal output. Empty snapshot → no replay event.
    let replay = (!snapshot.is_empty()).then(|| {
        Ok::<_, Infallible>(Event::default().event("pty.bytes").data(B64.encode(&snapshot)))
    });

    let live = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(chunk) => Some(Ok::<_, Infallible>(
            Event::default().event("pty.bytes").data(B64.encode(&chunk)),
        )),
        Err(_) => None,
    });

    let stream = futures::stream::iter(replay).chain(live);

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

async fn get_transcript(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(state) = store.get(&id) else {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    };
    let Some(cwd) = state.cwd.clone() else {
        return Json(transcript::Transcript::default()).into_response();
    };
    match transcript::read_for_session(&cwd, &id) {
        Ok(t) => Json(t).into_response(),
        Err(err) => {
            tracing::warn!(?err, "transcript read failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "transcript read failed").into_response()
        }
    }
}

async fn event_stream(
    State(store): State<SessionStore>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = store.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(update) => match serde_json::to_string(&update) {
            Ok(json) => Some(Ok(Event::default().event("session.update").data(json))),
            Err(err) => {
                tracing::warn!(?err, "failed to serialize session update");
                None
            }
        },
        Err(err) => {
            tracing::warn!(?err, "sse subscriber lagged");
            None
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn hook_stream(
    State(store): State<SessionStore>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = store.subscribe_hooks();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(hook) => match serde_json::to_string(&hook) {
            Ok(json) => Some(Ok(Event::default().event("hook").data(json))),
            Err(err) => {
                tracing::warn!(?err, "failed to serialize hook event");
                None
            }
        },
        Err(err) => {
            tracing::warn!(?err, "hook sse subscriber lagged");
            None
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

// ---- v2 items API ----------------------------------------------------------

async fn list_items(
    State(db): State<Db>,
    Query(filter): Query<ListFilter>,
) -> impl IntoResponse {
    match tokio::task::spawn_blocking(move || db.list_items(filter))
        .await
        .unwrap_or_else(|err| Err(anyhow::anyhow!(err)))
    {
        Ok(items) => Json(json!({ "items": items })).into_response(),
        Err(err) => {
            tracing::warn!(?err, "list_items failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "list_items failed").into_response()
        }
    }
}

async fn get_item_by_id(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match tokio::task::spawn_blocking(move || db.get_item(&id))
        .await
        .unwrap_or_else(|err| Err(anyhow::anyhow!(err)))
    {
        Ok(Some(item)) => Json(item).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such item").into_response(),
        Err(err) => {
            tracing::warn!(?err, "get_item failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "get_item failed").into_response()
        }
    }
}

async fn post_item_action(
    State(db): State<Db>,
    State(items): State<ItemBroadcaster>,
    Path(id): Path<String>,
    Json(action): Json<ItemAction>,
) -> impl IntoResponse {
    let id_for_task = id.clone();
    let action_for_task = action.clone();
    let db_for_task = db.clone();
    let result = tokio::task::spawn_blocking(move || {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        db_for_task.apply_item_action(&id_for_task, &action_for_task, now)
    })
    .await
    .unwrap_or_else(|err| Err(anyhow::anyhow!(err)));

    match result {
        Ok(updated) => {
            broadcast_post_action(&items, &updated);
            Json(updated).into_response()
        }
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("no such item") {
                (StatusCode::NOT_FOUND, msg).into_response()
            } else {
                tracing::warn!(?err, "item action failed");
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

fn broadcast_post_action(items: &ItemBroadcaster, updated: &crate::store::items::ItemRow) {
    if updated.state == "resolved" {
        items.send(ItemChange::ItemResolved {
            id: updated.id.clone(),
            session_id: updated.session_id.clone(),
        });
    } else {
        items.send(ItemChange::ItemChanged { item: updated.clone() });
    }
}

async fn items_stream(
    State(items): State<ItemBroadcaster>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = items.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(change) => match serde_json::to_string(&change) {
            Ok(json) => Some(Ok(Event::default().event("item").data(json))),
            Err(err) => {
                tracing::warn!(?err, "failed to serialize item change");
                None
            }
        },
        Err(err) => {
            tracing::warn!(?err, "item sse subscriber lagged");
            None
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

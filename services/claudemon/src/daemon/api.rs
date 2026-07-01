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
use crate::session::{transcript, usage, ConversationStore, MessageOutcome, SessionMode, SessionStore};
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
    pub conv: ConversationStore,
}

impl FromRef<ApiState> for SessionStore {
    fn from_ref(state: &ApiState) -> Self {
        state.store.clone()
    }
}

impl FromRef<ApiState> for ConversationStore {
    fn from_ref(state: &ApiState) -> Self {
        state.conv.clone()
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
        .route("/sessions/spawn-managed", post(crate::daemon::spawn::handle_managed))
        .route("/providers/:provider/models", get(crate::daemon::spawn::handle_provider_models))
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
        .route("/sessions/:id/conversation", get(get_conversation))
        .route("/conversation/stream", get(conversation_stream))
        .route("/events", get(event_stream))
        .route("/hooks/stream", get(hook_stream))
        .route("/statusline/stream", get(status_line_stream))
        .route("/items", get(list_items))
        .route("/items/stream", get(items_stream))
        .route("/items/:id", get(get_item_by_id))
        .route("/items/:id/action", post(post_item_action))
        .route("/wrapper/:id", get(crate::daemon::wrapper_ws::upgrade))
        .route("/health", get(|| async { "ok" }))
        // Bound request bodies (tool inputs, messages) so a hostile or buggy
        // local client can't push an unbounded payload through the fanout + DB.
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
struct ListSessionsQuery {
    /// Include archived (stopped + long-idle) sessions in the response. Off by
    /// default so the list shows only live and recently-active agents; the UI
    /// opts in (`?include_archived=true`) to browse older ones.
    #[serde(default)]
    include_archived: bool,
}

async fn list_sessions(
    State(store): State<SessionStore>,
    Query(q): Query<ListSessionsQuery>,
) -> impl IntoResponse {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let states = store.list();
    let include_archived = q.include_archived;
    // `usage_for_path` reads each session's transcript from disk — doing it inline
    // would block a runtime worker (and its SSE streams) across N file reads, so
    // run the whole fold on the blocking pool.
    let sessions: Vec<Value> = tokio::task::spawn_blocking(move || {
        states
            .into_iter()
            .filter_map(|state| {
                let archived = state.is_archived(now);
                // Default list hides archived sessions; they're still reachable
                // via `?include_archived=true` and the per-session endpoint.
                if archived && !include_archived {
                    return None;
                }
                let u = usage::usage_for_path(state.transcript_path.as_deref());
                let mut v = serde_json::to_value(&state).unwrap_or(Value::Null);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("usage".to_string(), serde_json::to_value(&u).unwrap_or(Value::Null));
                    obj.insert("archived".to_string(), Value::Bool(archived));
                }
                Some(v)
            })
            .collect()
    })
    .await
    .unwrap_or_default();
    Json(sessions)
}

async fn get_session(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match store.get(&id) {
        Some(state) => {
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let archived = state.is_archived(now);
            let u = usage::usage_for_path(state.transcript_path.as_deref());
            let mut v = serde_json::to_value(&state).unwrap_or(Value::Null);
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "usage".to_string(),
                    serde_json::to_value(&u).unwrap_or(Value::Null),
                );
                obj.insert("archived".to_string(), Value::Bool(archived));
            }
            Json(v).into_response()
        }
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
    // The store owns the policy: send now when the prompt is up, otherwise
    // queue and flush on the next `Input` transition (so the first message after
    // spawn doesn't race the TUI's cold start). See `SessionStore::submit_message`.
    match store.submit_message(&id, payload.text) {
        MessageOutcome::Sent => Json(json!({ "ok": true })).into_response(),
        MessageOutcome::Queued => Json(json!({ "ok": true, "queued": true })).into_response(),
        MessageOutcome::Rejected(mode) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "session is not accepting chat input",
                "mode": mode,
                "expected": "input",
            })),
        )
            .into_response(),
        MessageOutcome::NoSession => {
            (StatusCode::NOT_FOUND, "session not found").into_response()
        }
        MessageOutcome::NoWrapper => {
            (StatusCode::NOT_FOUND, "no wrapper attached to that session").into_response()
        }
        MessageOutcome::WrapperGone => {
            (StatusCode::GONE, "wrapper disconnected").into_response()
        }
    }
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
    // Managed (adapter-driven) sessions don't use Claude's parked-hook gateway —
    // route the decision to the provider adapter, which forwards it to the
    // agent's own approval API.
    let approve = matches!(payload.decision.as_deref(), Some("yes") | Some("always"));
    if store.submit_managed_decision(&id, approve) {
        return Json(json!({ "ok": true, "managed": true, "approve": approve })).into_response();
    }
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
    signal: crate::protocol::Signal,
}

async fn post_signal(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<SignalPayload>,
) -> impl IntoResponse {
    // Managed (adapter-driven) sessions have no Claude-style PTY lifecycle: a
    // terminate/kill must stop the driver loop (which then kills the provider
    // server + TUI), not just poke the attached TUI. SIGINT still forwards to the
    // TUI below as an interactive interrupt.
    if store.is_managed(&id)
        && matches!(payload.signal, crate::protocol::Signal::Sigterm | crate::protocol::Signal::Sigkill)
    {
        return if store.terminate_managed(&id) {
            Json(json!({ "ok": true, "signal": payload.signal })).into_response()
        } else {
            (StatusCode::NOT_FOUND, "no managed session").into_response()
        };
    }
    let Some(handle) = store.wrapper(&id) else {
        return (StatusCode::NOT_FOUND, "no wrapper attached").into_response();
    };
    if handle
        .tx
        .send(WrapperMessage::Signal { signal: payload.signal })
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

    // Drive the receiver directly (not via BroadcastStream) so a lag can `await`
    // a fresh snapshot to repaint from. First event replays the ring buffer so
    // reconnecting/attaching clients see prior terminal output.
    let mut rx = rx;
    let stream = async_stream::stream! {
        if !snapshot.is_empty() {
            yield Ok::<_, Infallible>(Event::default().event("pty.bytes").data(B64.encode(&snapshot)));
        }
        loop {
            match rx.recv().await {
                Ok(chunk) => {
                    yield Ok(Event::default().event("pty.bytes").data(B64.encode(&chunk)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // The client fell behind and the ring dropped `n` chunks. Raw
                    // bytes have no seq to resync, and a dropped mid-escape chunk
                    // corrupts the terminal silently. Recover client-transparently:
                    // full terminal reset (RIS, `\x1bc`) + the current buffer, so
                    // the terminal clears and repaints instead of rendering a hole.
                    tracing::warn!(session = %id, dropped = n, "byte stream lagged; repainting from snapshot");
                    let repaint = store.output_snapshot(&id).await.unwrap_or_default();
                    let mut bytes = b"\x1bc".to_vec();
                    bytes.extend_from_slice(&repaint);
                    yield Ok(Event::default().event("pty.bytes").data(B64.encode(&bytes)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

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
    // Authoritative path captured from the hook — read the exact file. Falls
    // through to cwd-based resolution only if it isn't known yet or is missing.
    // Reading/parsing the JSONL is blocking file I/O; run it off the async
    // runtime so a large transcript doesn't stall a worker + its SSE streams.
    let cwd = state.cwd.clone();
    let tp = state.transcript_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Authoritative path captured from the hook — read the exact file. Falls
        // through to cwd-based resolution only if it isn't known or is missing.
        if let Some(tp) = tp.as_deref() {
            if std::path::Path::new(tp).exists() {
                return transcript::read_at(tp);
            }
        }
        match cwd {
            Some(cwd) => transcript::read_for_session(&cwd, &id),
            None => Ok(transcript::Transcript::default()),
        }
    })
    .await;
    match result {
        Ok(Ok(t)) => Json(t).into_response(),
        Ok(Err(err)) => {
            tracing::warn!(?err, "transcript read failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "transcript read failed").into_response()
        }
        Err(err) => {
            tracing::warn!(?err, "transcript read task panicked");
            (StatusCode::INTERNAL_SERVER_ERROR, "transcript read failed").into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConversationQuery {
    /// Return only items *after* this sequence number (1-based). Lets a client
    /// poll cheap incremental deltas — e.g. a supervisor digesting just the new
    /// turns since it last looked, instead of the whole transcript every time.
    since: Option<u64>,
}

/// Parsed conversation snapshot for one session: item history + the sequence
/// number of the last item, so a client can join the delta stream without gaps.
/// With `?since=N`, only items after sequence N are returned (the `seq` field is
/// still the latest, so the client advances its cursor to it).
async fn get_conversation(
    State(conv): State<ConversationStore>,
    Path(id): Path<String>,
    Query(q): Query<ConversationQuery>,
) -> impl IntoResponse {
    let (seq, mut items) = conv.snapshot(&id).unwrap_or((0, Vec::new()));
    if let Some(since) = q.since {
        let skip = items_skip(seq, items.len(), since);
        items.drain(0..skip);
    }
    Json(json!({ "session_id": id, "seq": seq, "items": items }))
}

/// How many leading items to drop so only those with sequence > `since` remain,
/// given a window of `len` items ending at sequence `seq`. The first item's
/// sequence is `seq - len + 1`. Clamped to `[0, len]`.
fn items_skip(seq: u64, len: usize, since: u64) -> usize {
    let first_seq = seq.saturating_sub(len as u64).saturating_add(1);
    (since.saturating_add(1).saturating_sub(first_seq) as usize).min(len)
}

/// Global SSE feed of conversation deltas across all sessions — the content
/// counterpart to `/hooks/stream`. Each frame is a `ConversationDelta`.
async fn conversation_stream(
    State(conv): State<ConversationStore>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = conv.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(delta) => match serde_json::to_string(&delta) {
            Ok(json) => Some(Ok(Event::default().event("conversation.delta").data(json))),
            Err(err) => {
                tracing::warn!(?err, "failed to serialize conversation delta");
                None
            }
        },
        Err(err) => {
            tracing::warn!(?err, "conversation sse subscriber lagged");
            None
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
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

async fn status_line_stream(
    State(store): State<SessionStore>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = store.subscribe_status_lines();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(update) => match serde_json::to_string(&update) {
            Ok(json) => Some(Ok(Event::default().event("statusline").data(json))),
            Err(err) => {
                tracing::warn!(?err, "failed to serialize statusline update");
                None
            }
        },
        Err(err) => {
            tracing::warn!(?err, "statusline sse subscriber lagged");
            None
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

// ---- v2 items API (EXPERIMENTAL / PARKED) -----------------------------------
// These routes serve the classifier-driven inbox. They have no live client
// today — kept as future substrate. See docs/production-inventory.md §6.1.

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

#[cfg(test)]
mod tests {
    use super::items_skip;

    #[test]
    fn items_skip_window() {
        // 5 items, seq=5, no resets → first item's seq is 1.
        assert_eq!(items_skip(5, 5, 0), 0); // since 0 → keep all
        assert_eq!(items_skip(5, 5, 3), 3); // since 3 → keep items 4,5
        assert_eq!(items_skip(5, 5, 5), 5); // since 5 → keep none
        assert_eq!(items_skip(5, 5, 9), 5); // since beyond seq → keep none
        // A trimmed window (e.g. after items were consumed): seq=10, len=4 →
        // first item's seq is 7.
        assert_eq!(items_skip(10, 4, 6), 0); // since older than window → keep all
        assert_eq!(items_skip(10, 4, 8), 2); // since 8 → keep items 9,10
        assert_eq!(items_skip(0, 0, 0), 0); // empty
    }
}

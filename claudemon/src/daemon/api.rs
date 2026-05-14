use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Path, State},
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

pub fn router(store: SessionStore) -> Router {
    Router::new()
        .route("/sessions", get(list_sessions))
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id/input", post(post_input))
        .route("/sessions/:id/message", post(post_message))
        .route("/sessions/:id/approve", post(post_approve))
        .route("/sessions/:id/answer", post(post_answer))
        .route("/sessions/:id/decide", post(post_decide))
        .route("/sessions/:id/gate", post(post_gate))
        .route("/sessions/:id/signal", post(post_signal))
        .route("/sessions/:id/output", get(get_output))
        .route("/sessions/:id/stream", get(stream_bytes))
        .route("/sessions/:id/transcript", get(get_transcript))
        .route("/events", get(event_stream))
        .route("/wrapper/:id", get(crate::daemon::wrapper_ws::upgrade))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .with_state(store)
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
    let Some(rx) = store.subscribe_bytes(&id) else {
        return (StatusCode::NOT_FOUND, "no buffer for that session").into_response();
    };
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(chunk) => Some(Ok::<_, Infallible>(
            Event::default().event("pty.bytes").data(B64.encode(&chunk)),
        )),
        Err(_) => None,
    });
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
    match transcript::read_for_cwd(&cwd) {
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

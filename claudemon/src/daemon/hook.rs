use std::time::Duration;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};

use crate::session::{HookEvent, SessionStore};

/// How long we hold a PreToolUse hook open waiting for a decision when the
/// session has its gate enabled. Claude Code's own hook timeout is 60s by
/// default, so anything under that is safe — but long values can stall a
/// session if no client is listening, hence the per-session opt-in.
const DECISION_TIMEOUT: Duration = Duration::from_secs(30);

pub fn router(store: SessionStore) -> Router {
    Router::new()
        .route("/hook", post(receive))
        .route("/health", axum::routing::get(health))
        .with_state(store)
}

async fn health() -> &'static str {
    "ok"
}

async fn receive(
    State(store): State<SessionStore>,
    Json(event): Json<HookEvent>,
) -> impl IntoResponse {
    tracing::debug!(event = %event.event, session = %event.session_id, "hook received");
    let event_kind = event.event.clone();
    let payload_snapshot = event.payload.clone();
    let tool = payload_snapshot
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    // ingest() resolves alias / binds pending spawn by cwd; use the canonical
    // session_id it returns for everything below (gate lookup, park_decision)
    // so a /sessions/spawn → /gate flow works even though Claude's hook arrives
    // with a different session_id.
    let state = store.ingest(event);
    let session_id = state.session_id.clone();

    // Park PreToolUse responses if the session has the gate enabled. Claude
    // Code reads our stdout for the decision and uses it to allow/block the
    // tool call. Other event types don't take a decision — we return an
    // empty object so Claude doesn't trip on unexpected JSON keys.
    //
    // AskUserQuestion is a tool that asks the user something; the hook
    // response can't supply the answer, only allow/block the tool's
    // invocation. Don't park it — let it pass through so Claude's UI
    // shows the picker, and a client can call /answer to inject the
    // user's choice. Approval gating is for "should this tool run at all"
    // events (Bash, Edit, Write, …).
    let is_ask_question = tool.as_deref() == Some("AskUserQuestion");
    if event_kind == "PreToolUse" && store.gate_enabled(&session_id) && !is_ask_question {
        let raw = Value::Object(payload_snapshot);
        let rx = store.park_decision(&session_id, tool, raw);
        match tokio::time::timeout(DECISION_TIMEOUT, rx).await {
            Ok(Ok(decision)) => {
                tracing::info!(session = %session_id, ?decision, "decision resolved");
                return (StatusCode::OK, Json(decision)).into_response();
            }
            Ok(Err(_)) => {
                tracing::debug!(session = %session_id, "decision channel dropped; passthrough");
            }
            Err(_) => {
                tracing::debug!(session = %session_id, "decision timed out; passthrough");
            }
        }
        store.clear_pending_decision(&session_id);
    }

    (StatusCode::OK, Json(json!({}))).into_response()
}

/// Decision payload returned to Claude Code in the hook response body.
/// Shape: `{"decision":"approve"|"block","reason":"..."}`. Empty object
/// means "no decision, fall through to Claude's normal flow."
pub type HookDecision = Value;

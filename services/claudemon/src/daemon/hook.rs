use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

use crate::session::{HookEvent, SessionStore, Transport};

/// How long we hold a PreToolUse hook open waiting for a decision when the
/// session has its gate enabled. Claude Code's own hook timeout is 60s by
/// default, so anything under that is safe — but long values can stall a
/// session if no client is listening, hence the per-session opt-in.
const DECISION_TIMEOUT: Duration = Duration::from_secs(30);

pub fn router(store: SessionStore) -> Router {
    router_with_host(store, None)
}

/// Build the hook ingress router with the same `Host` allowlist the API router
/// enforces. The hook port is a write surface — everything posted here lands in
/// the session state machine, the broadcast fanout and SQLite — so it needs the
/// DNS-rebinding guard just as much: without it a malicious page can rebind its
/// own hostname to 127.0.0.1 and POST forged hook events at port 7890 while the
/// API port refuses the identical request.
pub fn router_with_host(store: SessionStore, bind_host: Option<String>) -> Router {
    let allowed_hosts = crate::daemon::api::AllowedHosts::new(bind_host);
    Router::new()
        .route("/hook", post(receive))
        .route("/hook/:kind", post(receive_named))
        .route("/statusline", post(receive_status_line))
        .route("/health", axum::routing::get(health))
        // Cap hook/statusline payloads: they're cloned, broadcast to every
        // subscriber, and persisted to SQLite, so an unbounded body is a DoS seam.
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            allowed_hosts,
            crate::daemon::api::host_guard,
        ))
        .with_state(store)
}

async fn health() -> &'static str {
    "ok"
}

async fn receive(
    State(store): State<SessionStore>,
    Json(event): Json<HookEvent>,
) -> impl IntoResponse {
    process(&store, event).await
}

/// `POST /hook/:kind` — per-event subroutes from v2 spec §13. Kind comes from
/// the URL path (`session_start`, `pre_tool`, …) and overrides any `event`
/// field in the body, so callers don't need to repeat the event name. The
/// rest of the body is the same `HookEvent` shape `/hook` accepts.
async fn receive_named(
    State(store): State<SessionStore>,
    Path(kind): Path<String>,
    Json(mut body): Json<Value>,
) -> impl IntoResponse {
    let Some(event_name) = subroute_to_event(&kind) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown hook kind", "kind": kind})),
        )
            .into_response();
    };
    if let Some(obj) = body.as_object_mut() {
        obj.insert("event".to_string(), Value::String(event_name.to_string()));
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "body must be a JSON object"})),
        )
            .into_response();
    }
    match serde_json::from_value::<HookEvent>(body) {
        Ok(event) => process(&store, event).await,
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid hook event", "detail": err.to_string()})),
        )
            .into_response(),
    }
}

/// `POST /statusline` — receives the JSON Claude Code pipes to its `statusLine`
/// command (forwarded by claudemon's wrapper command, see `init.rs`). This is a
/// separate channel from hooks: it's the only source of context-window %,
/// cumulative cost, and the 5h/7d rate-limit windows. We attach it to the
/// session and return an empty body — Claude ignores the forwarder's output
/// (the user's own statusLine script still renders the terminal line).
async fn receive_status_line(
    State(store): State<SessionStore>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let matched = store.ingest_status_line(&body).is_some();
    tracing::debug!(matched, "statusline received");
    (StatusCode::OK, Json(json!({})))
}

async fn process(store: &SessionStore, event: HookEvent) -> axum::response::Response {
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
    // Managed (adapter-driven) sessions — including claude's stream transport,
    // whose headless CLI still runs the user's hooks — own approvals in their
    // driver (`can_use_tool` control requests), and `ingest` above already kept
    // this hook enrichment-only. Parking here would force-flip the mode behind
    // the driver's back and block the CLI on a decision `/approve` can never
    // route back to this parked hook (it goes to the driver's decision channel
    // instead) — so the gate applies to PTY sessions only.
    let driver_owned = state.transport == Transport::Stream || store.is_managed(&session_id);
    if event_kind == "PreToolUse"
        && !driver_owned
        && store.gate_enabled(&session_id)
        && !is_ask_question
    {
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

/// Map a v2-spec hook subroute slug to the Claude Code event name it stands
/// for. Returning `None` produces a 404 so typos don't silently land an
/// event with an unrecognized kind.
fn subroute_to_event(kind: &str) -> Option<&'static str> {
    match kind {
        "session_start" => Some("SessionStart"),
        "session_end" => Some("SessionEnd"),
        "pre_tool" => Some("PreToolUse"),
        "post_tool" => Some("PostToolUse"),
        "tool_fail" => Some("PostToolUseFailure"),
        "permission" => Some("PermissionRequest"),
        "notification" => Some("Notification"),
        "stop" => Some("Stop"),
        "stop_fail" => Some("StopFailure"),
        "subagent_stop" => Some("SubagentStop"),
        "user_prompt_submit" => Some("UserPromptSubmit"),
        _ => None,
    }
}

/// Decision payload returned to Claude Code in the hook response body.
/// Shape: `{"decision":"approve"|"block","reason":"..."}`. Empty object
/// means "no decision, fall through to Claude's normal flow."
pub type HookDecision = Value;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt; // for `oneshot`

    /// Dispatch one request through the hook router (layers included) and return
    /// the status plus body bytes.
    async fn request(req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let resp = router(SessionStore::new())
            .oneshot(req)
            .await
            .expect("router responds");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("collect body");
        (status, bytes.to_vec())
    }

    #[tokio::test]
    async fn host_guard_rejects_non_loopback_host() {
        // Same DNS-rebinding defense the API router has: the hook port ingests
        // straight into the session state machine, so a rebound page must not be
        // able to post forged events at it.
        let req = Request::builder()
            .method("POST")
            .uri("/hook")
            .header("content-type", "application/json")
            .header(header::HOST, "evil.example.com")
            .body(Body::from(
                json!({ "event": "SessionStart", "session_id": "s1" }).to_string(),
            ))
            .unwrap();
        let (status, body) = request(req).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, b"host not allowed");
    }

    #[tokio::test]
    async fn host_guard_allows_loopback_host() {
        let req = Request::builder()
            .uri("/health")
            .header(header::HOST, "127.0.0.1:7890")
            .body(Body::empty())
            .unwrap();
        let (status, body) = request(req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ok");
    }

    #[test]
    fn known_subroutes_map_to_event_names() {
        assert_eq!(subroute_to_event("session_start"), Some("SessionStart"));
        assert_eq!(subroute_to_event("pre_tool"), Some("PreToolUse"));
        assert_eq!(subroute_to_event("tool_fail"), Some("PostToolUseFailure"));
        assert_eq!(subroute_to_event("permission"), Some("PermissionRequest"));
    }

    #[test]
    fn unknown_subroute_is_none() {
        assert_eq!(subroute_to_event("not_a_hook"), None);
        assert_eq!(subroute_to_event(""), None);
    }
}

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{FromRef, Path, Query, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::protocol::WrapperMessage;
use crate::session::{
    transcript, usage, ConversationStore, MessageOutcome, PermissionMode, PermissionSwitchError,
    SessionMode, SessionStore,
};
use crate::store::Db;

/// Bundled state for the v2 API router. FromRef lets each handler extract
/// just the slice it needs, so v1 handlers keep their `State<SessionStore>`
/// signature.
#[derive(Clone)]
pub struct ApiState {
    pub store: SessionStore,
    pub db: Db,
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

/// Which extra `Host` header value (beyond loopback) the API accepts. When the
/// daemon is bound to a concrete non-loopback address (`--host 192.168.1.5`),
/// that address is a legitimate `Host`; the wildcard binds (`0.0.0.0`, `::`)
/// carry no meaningful host name, so only loopback is accepted for those and
/// remote access is expected to go through the hub bus, not this API directly.
#[derive(Clone, Default)]
struct AllowedHosts {
    extra: Option<String>,
}

impl AllowedHosts {
    fn new(bind_host: Option<String>) -> Self {
        // A wildcard bind names no host, so it adds nothing to the allowlist.
        let extra = bind_host.filter(|h| h != "0.0.0.0" && h != "::" && !h.is_empty());
        Self { extra }
    }

    /// Accept the request's `Host` iff it is loopback (127.0.0.0/8, ::1,
    /// `localhost`) or exactly the configured concrete bind host.
    fn permits(&self, host_header: &str) -> bool {
        let host = host_without_port(host_header);
        if host_is_loopback(host) {
            return true;
        }
        match &self.extra {
            Some(extra) => host == host_without_port(extra),
            None => false,
        }
    }
}

/// Strip a trailing `:port` from a `Host`/authority, handling bracketed IPv6
/// (`[::1]:7891` → `::1`).
fn host_without_port(h: &str) -> &str {
    if let Some(rest) = h.strip_prefix('[') {
        // `[::1]:7891` → `::1`
        return rest.split(']').next().unwrap_or(h);
    }
    h.rsplit_once(':').map(|(host, _)| host).unwrap_or(h)
}

fn host_is_loopback(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// True if an `Origin` header names a loopback web origin
/// (`http(s)://localhost|127.0.0.1|[::1]` with any port). Everything else —
/// arbitrary websites, `null`/opaque origins — is rejected, so a page the user
/// happens to be visiting cannot make cross-origin calls to the local daemon.
fn is_loopback_origin(origin: &HeaderValue) -> bool {
    let Ok(s) = origin.to_str() else {
        return false;
    };
    let Some(rest) = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
    else {
        return false;
    };
    let host = if let Some(v6) = rest.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        rest.split(['/', ':']).next().unwrap_or("")
    };
    host_is_loopback(host)
}

/// CORS for the local API. No legitimate browser context calls this daemon
/// directly (the desktop renderer reaches it through the Electron main process
/// and the hub bus; the web client goes through the hub) — non-browser clients
/// (Electron main, wks-tui, hub, brain) send no `Origin` and are unaffected by
/// CORS. We therefore reflect only loopback origins so that a random website
/// cannot drive a preflighted cross-origin mutation (spawn/commit/push/signal).
/// Credentials are never reflected (the API uses none).
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            is_loopback_origin(origin)
        }))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE])
        .allow_credentials(false)
}

/// Reject requests whose `Host` header is neither loopback nor the configured
/// bind host. CORS alone can't stop a DNS-rebinding attack (after rebinding,
/// the malicious page and the daemon share an origin, so no preflight fires);
/// pinning `Host` to expected values closes that hole for the side-effecting
/// endpoints. Requests with no `Host` at all (non-browser clients that omit it)
/// pass through — the rebinding vector requires a browser, which always sends one.
async fn host_guard(State(allowed): State<AllowedHosts>, req: Request, next: Next) -> Response {
    match req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
    {
        Some(host) if !allowed.permits(host) => {
            (StatusCode::FORBIDDEN, "host not allowed").into_response()
        }
        _ => next.run(req).await,
    }
}

/// Reject session ids that could escape their on-disk transcript/handoff roots.
/// Real ids are UUIDs or provider tokens: ASCII alphanumerics plus `-` `_` `.`.
/// Anything with a path separator, a `..` segment, or other bytes is a traversal
/// attempt (`../../etc/passwd`) and is refused before it reaches the filesystem.
/// axum percent-decodes the path segment first, so `%2e%2e%2f` is caught here too.
fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.contains("..")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn router(state: ApiState) -> Router {
    router_with_host(state, None)
}

/// Build the API router, accepting `Host` headers for loopback plus (optionally)
/// the concrete address the daemon is bound to. `router` defaults to loopback-only.
pub fn router_with_host(state: ApiState, bind_host: Option<String>) -> Router {
    let allowed_hosts = AllowedHosts::new(bind_host);
    Router::new()
        .route("/sessions", get(list_sessions))
        .route("/sessions/spawn", post(crate::daemon::spawn::handle))
        .route(
            "/sessions/spawn-managed",
            post(crate::daemon::spawn::handle_managed),
        )
        .route(
            "/providers/:provider/models",
            get(crate::daemon::spawn::handle_provider_models),
        )
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id/input", post(post_input))
        .route("/sessions/:id/message", post(post_message))
        .route("/sessions/:id/approve", post(post_approve))
        .route("/sessions/:id/answer", post(post_answer))
        .route("/sessions/:id/decide", post(post_decide))
        .route("/sessions/:id/gate", post(post_gate))
        .route("/sessions/:id/signal", post(post_signal))
        .route("/sessions/:id/permission-mode", post(post_permission_mode))
        .route("/sessions/:id/model", post(post_model))
        .route("/sessions/:id/resize", post(post_resize))
        .route("/sessions/:id/output", get(get_output))
        .route("/sessions/:id/stream", get(stream_bytes))
        .route("/sessions/:id/transcript", get(get_transcript))
        .route("/sessions/:id/conversation", get(get_conversation))
        .route("/sessions/:id/handoff", post(post_handoff))
        .route("/conversation/stream", get(conversation_stream))
        .route("/events", get(event_stream))
        .route("/hooks/stream", get(hook_stream))
        .route("/statusline/stream", get(status_line_stream))
        .route("/wrapper/:id", get(crate::daemon::wrapper_ws::upgrade))
        // MCP streamable-HTTP server (POST-only; axum answers GET with 405) —
        // gives MCP-speaking agents (Codex) an AskUserQuestion tool that
        // parks a structured question in the GUI. See daemon::mcp_ask.
        .route("/mcp/ask/:session_id", post(crate::daemon::mcp_ask::handle))
        .route("/health", get(|| async { "ok" }))
        // Bound request bodies (tool inputs, messages) so a hostile or buggy
        // local client can't push an unbounded payload through the fanout + DB.
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        // Loopback-only CORS (see `cors_layer`) replaces the previous
        // `CorsLayer::permissive()`, which let any website drive the daemon.
        .layer(cors_layer())
        // Host-header guard runs outermost (added last), so a DNS-rebinding
        // request is refused before it can reach a handler or the CORS layer.
        .layer(middleware::from_fn_with_state(allowed_hosts, host_guard))
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
                    obj.insert(
                        "usage".to_string(),
                        serde_json::to_value(&u).unwrap_or(Value::Null),
                    );
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
        .send(WrapperMessage::Input {
            bytes: B64.encode(&raw),
        })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    // Raw keystrokes mean the composer is no longer exclusively ours — any
    // in-flight submit-verify ladder must stand down (see `note_client_input`).
    store.note_client_input(&id);
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
    // The store owns the policy: every live mode accepts the message — sent
    // through the settle+verify pipeline when the prompt is up, queued and
    // flushed on the next `Input` transition otherwise (cold start, mid-turn,
    // or an open approval/question dialog). Only a stopped session rejects.
    // See `SessionStore::submit_message`.
    match store.submit_message(&id, payload.text) {
        MessageOutcome::Sent => Json(json!({ "ok": true })).into_response(),
        MessageOutcome::Queued => Json(json!({ "ok": true, "queued": true })).into_response(),
        MessageOutcome::Rejected(mode) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "session has ended and cannot accept chat input",
                "mode": mode,
                "expected": "input",
            })),
        )
            .into_response(),
        MessageOutcome::NoSession => (StatusCode::NOT_FOUND, "session not found").into_response(),
        MessageOutcome::NoWrapper => {
            (StatusCode::NOT_FOUND, "no wrapper attached to that session").into_response()
        }
        MessageOutcome::WrapperGone => (StatusCode::GONE, "wrapper disconnected").into_response(),
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
    if payload.option.is_none() && payload.text.is_none() && payload.answers.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "expected `option`, `text`, or `answers`",
        )
            .into_response();
    }
    // Managed stream sessions resolve the parked AskUserQuestion structurally
    // (the driver answers the CLI's `can_use_tool` with the chosen options) —
    // there are no picker keystrokes to type.
    if store.submit_managed_answer(
        &id,
        crate::session::ManagedAnswer {
            option: payload.option,
            text: payload.text.clone(),
            answers: payload.answers.clone(),
        },
    ) {
        return Json(json!({ "ok": true, "managed": true })).into_response();
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
        .send(WrapperMessage::Input {
            bytes: B64.encode(&bytes),
        })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    // Picker keystrokes count as client input for the submit-verify ladder.
    store.note_client_input(&id);
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
        && matches!(
            payload.signal,
            crate::protocol::Signal::Sigterm | crate::protocol::Signal::Sigkill
        )
    {
        return if store.terminate_managed(&id) {
            Json(json!({ "ok": true, "signal": payload.signal })).into_response()
        } else {
            (StatusCode::NOT_FOUND, "no managed session").into_response()
        };
    }
    // SIGINT on a driver with a structural interrupt (the stream transport's
    // `interrupt` control request) — the Ctrl-C equivalent: stop the current
    // turn, keep the session alive.
    if matches!(payload.signal, crate::protocol::Signal::Sigint) && store.interrupt_managed(&id) {
        return Json(json!({ "ok": true, "signal": payload.signal })).into_response();
    }
    let Some(handle) = store.wrapper(&id) else {
        return (StatusCode::NOT_FOUND, "no wrapper attached").into_response();
    };
    if handle
        .tx
        .send(WrapperMessage::Signal {
            signal: payload.signal,
        })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    Json(json!({ "ok": true, "signal": payload.signal })).into_response()
}

#[derive(Debug, Deserialize)]
struct PermissionModePayload {
    mode: String,
}

/// Live permission-mode switch, no restart, conversation untouched.
///
/// PTY (claude) sessions: the daemon presses Shift+Tab (the TUI's own mode
/// cycle) and verifies each step against the reconstructed screen until the
/// target mode's footer marker shows (`SessionStore::set_permission_mode`).
/// Managed sessions (codex over the app-server ws): flips the adapter's
/// auto-approve flag — modes are `ask`/`yolo`, and yolo→ask is only possible
/// when the provider wasn't spawned in bypass mode
/// (`SessionStore::set_managed_permission_mode`).
async fn post_permission_mode(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<PermissionModePayload>,
) -> impl IntoResponse {
    // Stream-transport sessions speak Claude's own mode vocabulary through the
    // control protocol (`set_permission_mode`), so the switch is structural and
    // confirmed by the CLI — no ask/yolo indirection, no keystrokes.
    if store.has_managed_permission_mode(&id) {
        if !stream_permission_mode_valid(&payload.mode) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": format!("unknown permission mode '{}'", payload.mode) })),
            )
                .into_response();
        }
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if !store.submit_managed_permission_mode(
            &id,
            crate::session::ManagedPermissionSwitch {
                mode: payload.mode.clone(),
                reply: reply_tx,
            },
        ) {
            return (StatusCode::GONE, "session driver disconnected").into_response();
        }
        return match tokio::time::timeout(Duration::from_secs(10), reply_rx).await {
            Ok(Ok(Ok(mode))) => Json(json!({ "ok": true, "mode": mode })).into_response(),
            Ok(Ok(Err(err))) => (
                StatusCode::CONFLICT,
                Json(json!({ "ok": false, "error": err })),
            )
                .into_response(),
            // Driver died or the CLI never answered — unverified.
            _ => (
                StatusCode::CONFLICT,
                Json(json!({ "ok": false, "error": "the agent did not confirm the mode switch" })),
            )
                .into_response(),
        };
    }
    let result = if store.is_managed(&id) {
        if payload.mode != "ask" && payload.mode != "yolo" {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": format!("unknown managed permission mode '{}' (expected 'ask' or 'yolo')", payload.mode) })),
            )
                .into_response();
        }
        store
            .set_managed_permission_mode(&id, &payload.mode)
            .map(str::to_string)
    } else {
        let Some(target) = PermissionMode::parse(&payload.mode) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": format!("unknown permission mode '{}'", payload.mode) })),
            )
                .into_response();
        };
        store
            .set_permission_mode(&id, target)
            .await
            .map(|m| m.as_str().to_string())
    };
    match result {
        Ok(mode) => Json(json!({ "ok": true, "mode": mode })).into_response(),
        Err(err) => {
            let (status, msg, current) = match err {
                PermissionSwitchError::NoSession => {
                    (StatusCode::NOT_FOUND, "session not found".to_string(), None)
                }
                PermissionSwitchError::NoWrapper => (
                    StatusCode::NOT_FOUND,
                    "no wrapper attached to that session".to_string(),
                    None,
                ),
                PermissionSwitchError::Managed => (
                    StatusCode::CONFLICT,
                    "this session's permission policy is frozen at spawn — restart to change it"
                        .to_string(),
                    None,
                ),
                PermissionSwitchError::Busy(mode) => (
                    StatusCode::CONFLICT,
                    format!("session is busy ({mode:?}) — try again when the dialog is resolved"),
                    None,
                ),
                PermissionSwitchError::Unavailable(current) => (
                    StatusCode::CONFLICT,
                    format!(
                        "mode '{}' is not in this session's shift+tab cycle",
                        payload.mode
                    ),
                    Some(current.as_str().to_string()),
                ),
                PermissionSwitchError::Unverified(current) => (
                    StatusCode::CONFLICT,
                    "the TUI did not acknowledge the mode cycle keystroke".to_string(),
                    Some(current.as_str().to_string()),
                ),
                PermissionSwitchError::ManagedUnavailable { current } => (
                    StatusCode::CONFLICT,
                    "the agent was started with approvals bypassed — restart to re-enable them"
                        .to_string(),
                    Some(current.to_string()),
                ),
            };
            let mut body = json!({ "ok": false, "error": msg });
            if let Some(current) = current {
                body["mode"] = json!(current);
            }
            (status, Json(body)).into_response()
        }
    }
}

/// The permission modes a stream-transport session accepts: Claude's four
/// canonical modes plus the newer CLI spellings (`--permission-mode` choices
/// in 2.1.x). Validated here so an obvious typo 400s instead of riding to the
/// CLI; the CLI still gets the final say via its control response.
fn stream_permission_mode_valid(mode: &str) -> bool {
    PermissionMode::parse(mode).is_some() || matches!(mode, "auto" | "manual" | "dontAsk")
}

#[derive(Debug, Deserialize)]
struct ModelSwitchPayload {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
}

/// Live model/effort switch for a managed session, no restart, conversation
/// untouched. Codex over the app-server ws applies it to the running thread
/// via `thread/settings/update`; the claude stream driver sends the control
/// protocol's `set_model` (verified live in 2.1.201 — the next turn runs the
/// new model). Providers without a switch channel (opencode/pi, codex rollout
/// fallback) get a 409 so the caller can offer the restart path. PTY (claude)
/// sessions switch through their own `/model` slash command on the message
/// path — this endpoint is the managed-provider counterpart.
async fn post_model(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(payload): Json<ModelSwitchPayload>,
) -> impl IntoResponse {
    if payload.model.is_none() && payload.effort.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "provide `model` and/or `effort`" })),
        )
            .into_response();
    }
    if !store.is_managed(&id) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "PTY sessions switch via the /model slash command on the message path" })),
        )
            .into_response();
    }
    match store.set_managed_model(
        &id,
        crate::session::ModelSwitch {
            model: payload.model.clone(),
            effort: payload.effort.clone(),
        },
    ) {
        Ok(()) => Json(json!({ "ok": true, "model": payload.model, "effort": payload.effort }))
            .into_response(),
        Err(err) => (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": err })),
        )
            .into_response(),
    }
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
        .send(WrapperMessage::Resize {
            cols: payload.cols,
            rows: payload.rows,
        })
        .is_err()
    {
        return (StatusCode::GONE, "wrapper disconnected").into_response();
    }
    store.note_term_size(&id, payload.cols, payload.rows);
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

#[derive(Debug, Deserialize)]
struct TranscriptQuery {
    /// Fallback working directory for sessions this daemon isn't tracking:
    /// with `?cwd=`, an unknown id is resolved against the on-disk JSONL at
    /// `~/.claude/projects/<encoded-cwd>/<id>.jsonl` instead of 404ing, so
    /// clients can fetch historical transcripts recorded before the daemon
    /// started (e.g. a timeline replaying an old session).
    cwd: Option<String>,
}

async fn get_transcript(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> impl IntoResponse {
    // The id is interpolated into a `<projects>/<cwd>/<id>.jsonl` path below —
    // refuse traversal-shaped ids before any filesystem work.
    if !valid_session_id(&id) {
        return (StatusCode::BAD_REQUEST, "invalid session id").into_response();
    }
    let Some(state) = store.get(&id) else {
        // Not a live session. With a cwd hint, serve the historical transcript
        // straight from disk (read_for_session confines the read to the
        // projects root); without one, unknown stays 404.
        let Some(cwd) = q.cwd.filter(|c| !c.is_empty()) else {
            return (StatusCode::NOT_FOUND, "session not found").into_response();
        };
        let result =
            tokio::task::spawn_blocking(move || transcript::read_for_session(&cwd, &id)).await;
        return match result {
            Ok(Ok(t)) => Json(t).into_response(),
            Ok(Err(err)) => {
                tracing::warn!(?err, "historical transcript read failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "transcript read failed").into_response()
            }
            Err(err) => {
                tracing::warn!(?err, "historical transcript read task panicked");
                (StatusCode::INTERNAL_SERVER_ERROR, "transcript read failed").into_response()
            }
        };
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
    let (seq, items) = conv.snapshot(&id).unwrap_or((0, Vec::new()));
    // Codex restart durability: the ws adapter's conversation lives in daemon
    // memory, so a restarted daemon serves Stopped codex rows empty — but the
    // codex-threads sidecar knows which rollout backed the session, and the
    // rollout is the durable transcript. Replay it once, lazily, on first read.
    let (seq, mut items) = if items.is_empty() {
        match crate::providers::codex_rollout::thread_for(&id)
            .and_then(|tid| crate::providers::codex_rollout::rollout_for_thread(&tid))
            .map(|path| crate::providers::codex_rollout::replay_conversation(&path))
            .filter(|replayed| !replayed.is_empty())
        {
            Some(replayed) => {
                conv.push(&id, replayed);
                conv.snapshot(&id).unwrap_or((0, Vec::new()))
            }
            None => (seq, items),
        }
    } else {
        (seq, items)
    };
    if let Some(since) = q.since {
        let skip = items_skip(seq, items.len(), since);
        items.drain(0..skip);
    }
    Json(json!({ "session_id": id, "seq": seq, "items": items }))
}

#[derive(Debug, Default, Deserialize)]
struct HandoffPayload {
    /// Skip writing the brief under `~/.workspacer/handoffs/` (default writes).
    #[serde(default)]
    no_persist: bool,
}

/// Build a cross-provider handoff brief from the session's conversation —
/// the markdown a successor agent (any harness) reads to take the work over.
/// Persists to `~/.workspacer/handoffs/` unless `no_persist`; the response
/// carries both the markdown and the file path. Works for stopped sessions
/// too, as long as their conversation is still tailed/cached.
async fn post_handoff(
    State(store): State<SessionStore>,
    State(conv): State<ConversationStore>,
    Path(id): Path<String>,
    Json(payload): Json<HandoffPayload>,
) -> impl IntoResponse {
    // `id` becomes part of the persisted brief's filename under
    // `~/.workspacer/handoffs/` — reject traversal-shaped ids up front.
    if !valid_session_id(&id) {
        return (StatusCode::BAD_REQUEST, "invalid session id").into_response();
    }
    let (_, items) = conv.snapshot(&id).unwrap_or((0, Vec::new()));
    if items.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "no conversation recorded for that session" })),
        )
            .into_response();
    }
    let state = store.get(&id);
    let markdown = crate::session::handoff::build_brief(&id, state.as_ref(), &items);
    let path = if payload.no_persist {
        None
    } else {
        match crate::session::handoff::persist_brief(&id, &markdown) {
            Ok(p) => Some(p.to_string_lossy().into_owned()),
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "ok": false, "error": format!("could not write handoff brief: {err}") })),
                )
                    .into_response();
            }
        }
    };
    Json(json!({ "ok": true, "markdown": markdown, "path": path })).into_response()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::conversation::ConversationItem;
    use crate::session::store::WrapperHandle;
    use crate::session::ModelSwitch;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt; // for `oneshot`

    /// A wrapper handle plus the receiver, so a test can assert exactly which
    /// input/signal/resize frames a handler forwarded to the child. Mirrors the
    /// store-side `handle_with_rx` helper.
    fn wrapper() -> (WrapperHandle, mpsc::UnboundedReceiver<WrapperMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (WrapperHandle { tx }, rx)
    }

    /// Decode the next `Input` frame's bytes, asserting one is present.
    fn next_input(rx: &mut mpsc::UnboundedReceiver<WrapperMessage>) -> Vec<u8> {
        match rx.try_recv().expect("expected a wrapper frame") {
            WrapperMessage::Input { bytes } => B64.decode(bytes).expect("valid base64"),
            other => panic!("expected Input frame, got {other:?}"),
        }
    }

    /// Build an `ApiState` backed by a throwaway on-disk SQLite db and empty
    /// in-memory stores. Each call gets a fresh db file so tests don't share
    /// state. This is the seam future handler tests build on: register whatever
    /// the handler reads (sessions, conversation) on `state.store`/`state.conv`
    /// before dispatching a request.
    fn test_state() -> ApiState {
        let mut db_path = std::env::temp_dir();
        db_path.push(format!("claudemon-api-test-{}.db", uuid::Uuid::new_v4()));
        ApiState {
            store: SessionStore::new(),
            db: Db::open(&db_path).expect("open test db"),
            conv: ConversationStore::new(),
        }
    }

    /// Dispatch one request through the full router (layers + routing) and
    /// return the status plus the response body as bytes. Uses tower's
    /// `oneshot`, the idiomatic axum handler-test path — no socket is bound.
    async fn request(state: ApiState, req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let resp = router(state).oneshot(req).await.expect("router responds");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("collect body");
        (status, bytes.to_vec())
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    fn post_json(uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let (status, body) = request(test_state(), get("/health")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ok");
    }

    #[tokio::test]
    async fn list_sessions_empty_is_empty_array() {
        let (status, body) = request(test_state(), get("/sessions")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!([]));
    }

    #[tokio::test]
    async fn list_sessions_includes_a_registered_session_with_provider() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let (status, body) = request(state, get("/sessions")).await;
        assert_eq!(status, StatusCode::OK);
        let arr = serde_json::from_slice::<Value>(&body).unwrap();
        let sessions = arr.as_array().expect("array");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["session_id"], "sess-1");
        // The additive provider field is surfaced on the wire.
        assert_eq!(sessions[0]["provider"], "codex");
    }

    #[tokio::test]
    async fn get_unknown_session_is_404() {
        let (status, body) = request(test_state(), get("/sessions/does-not-exist")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"session not found");
    }

    #[tokio::test]
    async fn post_model_on_unswitchable_session_conflicts() {
        // A managed session with no model-switch channel registered (opencode/pi
        // or the codex rollout fallback) can't switch live: the route surfaces
        // the store's refusal as 409 rather than a silent success.
        let state = test_state();
        state
            .store
            .register_managed("sess-1", "/tmp/proj", "opencode");
        let req = post_json("/sessions/sess-1/model", json!({ "model": "gpt-5.5" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].is_string());
    }

    // --- list / read routes -------------------------------------------------

    #[tokio::test]
    async fn list_sessions_hides_and_reveals_via_include_archived() {
        // A fresh session is never archived, so it shows in both views; the
        // point here is that the query param parses and both paths return it.
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let (status, body) = request(state.clone(), get("/sessions?include_archived=true")).await;
        assert_eq!(status, StatusCode::OK);
        let arr = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 1);
        // The archived flag is surfaced additively on each row.
        assert_eq!(arr[0]["archived"], false);
    }

    #[tokio::test]
    async fn get_session_returns_the_registered_state() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let (status, body) = request(state, get("/sessions/sess-1")).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["session_id"], "sess-1");
        assert_eq!(v["provider"], "codex");
        // usage + archived are decorated onto the base state.
        assert_eq!(v["archived"], false);
        assert!(v.get("usage").is_some());
    }

    #[tokio::test]
    async fn get_session_surfaces_a_set_plan() {
        use crate::session::state::{Plan, PlanStatus, PlanStep};
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        state.store.set_plan(
            &state.conv,
            "sess-1",
            Plan {
                steps: vec![
                    PlanStep {
                        content: "explore".into(),
                        status: PlanStatus::Completed,
                        active_form: None,
                    },
                    PlanStep {
                        content: "build".into(),
                        status: PlanStatus::InProgress,
                        active_form: Some("Building".into()),
                    },
                ],
                updated_at: Some("2026-07-04T10:00:00Z".into()),
            },
        );
        let (status, body) = request(state, get("/sessions/sess-1")).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        // The plan auto-serializes on the session state, in the fixed wire shape.
        assert_eq!(v["plan"]["updatedAt"], "2026-07-04T10:00:00Z");
        assert_eq!(v["plan"]["steps"][0]["content"], "explore");
        assert_eq!(v["plan"]["steps"][0]["status"], "completed");
        assert_eq!(v["plan"]["steps"][1]["status"], "in_progress");
        assert_eq!(v["plan"]["steps"][1]["activeForm"], "Building");
    }

    #[tokio::test]
    async fn get_output_unknown_session_is_404() {
        let (status, body) = request(test_state(), get("/sessions/nope/output")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"no buffer for that session");
    }

    #[tokio::test]
    async fn get_output_registered_session_is_empty_buffer() {
        // A managed session gets an (empty) output buffer at registration, so the
        // viewer-attach path works uniformly even though it never emits bytes.
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let (status, body) = request(state, get("/sessions/sess-1/output")).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["bytes"], 0);
        assert_eq!(v["text"], "");
    }

    #[tokio::test]
    async fn get_transcript_unknown_session_is_404() {
        let (status, body) = request(test_state(), get("/sessions/nope/transcript")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"session not found");
    }

    #[tokio::test]
    async fn get_transcript_unknown_session_with_cwd_reads_disk_not_404() {
        // An untracked id + a cwd hint is the historical-transcript path: the
        // handler goes to the on-disk projects dir instead of 404ing. Nothing
        // exists there for this cwd, so it serves an empty transcript.
        let (status, body) = request(
            test_state(),
            get("/sessions/nope/transcript?cwd=/definitely/not/a/project"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["messages"], json!([]));
    }

    #[tokio::test]
    async fn get_transcript_traversal_id_with_cwd_is_rejected() {
        // valid_session_id must gate the disk-fallback path too.
        let (status, _) = request(
            test_state(),
            get("/sessions/..%2F..%2Fetc/transcript?cwd=/tmp"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_transcript_with_no_file_returns_empty_transcript() {
        // No transcript on disk for this id → the reader falls through to a
        // default (empty) Transcript rather than erroring.
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let (status, body) = request(state, get("/sessions/sess-1/transcript")).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert!(v.is_object(), "transcript is a JSON object");
    }

    #[tokio::test]
    async fn get_conversation_unknown_session_is_empty_snapshot() {
        // Conversation never 404s — an unknown session is just an empty log.
        let (status, body) = request(test_state(), get("/sessions/nope/conversation")).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["session_id"], "nope");
        assert_eq!(v["seq"], 0);
        assert_eq!(v["items"], json!([]));
    }

    #[tokio::test]
    async fn get_conversation_since_filters_to_the_delta() {
        let state = test_state();
        state.conv.push(
            "sess-1",
            vec![
                ConversationItem::UserMessage {
                    text: "one".into(),
                    timestamp: None,
                },
                ConversationItem::AssistantText {
                    text: "two".into(),
                    timestamp: None,
                },
                ConversationItem::UserMessage {
                    text: "three".into(),
                    timestamp: None,
                },
            ],
        );
        // Full snapshot: seq 3, all three items.
        let (status, body) = request(state.clone(), get("/sessions/sess-1/conversation")).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["seq"], 3);
        assert_eq!(v["items"].as_array().unwrap().len(), 3);
        // `?since=2` advances the cursor: only the 3rd item remains, seq unchanged.
        let (status, body) = request(state, get("/sessions/sess-1/conversation?since=2")).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["seq"], 3);
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["text"], "three");
    }

    // The env lock is deliberately held across the awaits: it serializes the
    // process-global CODEX_HOME override, and nothing awaited here ever takes
    // the same lock, so there is no deadlock to guard against.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn get_conversation_replays_codex_rollout_once_not_per_request() {
        use crate::providers::codex_rollout;
        // Restart durability: an empty in-memory conversation whose session
        // maps (via the codex-threads sidecar) to an on-disk rollout is
        // replayed into the store — exactly once, even though the GUI polls
        // this endpoint. Re-appending per request would duplicate the visible
        // history unboundedly.
        let _env = codex_rollout::codex_home_test_lock();
        let codex_home =
            std::env::temp_dir().join(format!("claudemon-api-codex-home-{}", uuid::Uuid::new_v4()));
        let day = codex_home
            .join("sessions")
            .join("2026")
            .join("07")
            .join("10");
        std::fs::create_dir_all(&day).unwrap();
        let thread = format!("th-{}", uuid::Uuid::new_v4());
        let rollout = day.join(format!("rollout-2026-07-10-{thread}.jsonl"));
        let lines = [
            json!({ "type": "response_item", "payload": { "type": "message", "role": "user",
                "content": [{ "type": "input_text", "text": "hi" }] } })
            .to_string(),
            json!({ "type": "response_item", "payload": { "type": "message", "role": "assistant",
                "content": [{ "type": "output_text", "text": "OK" }] } })
            .to_string(),
        ];
        std::fs::write(&rollout, lines.join("\n")).unwrap();
        let prev = std::env::var_os("CODEX_HOME");
        std::env::set_var("CODEX_HOME", &codex_home);

        // Throwaway uuid session ids — the sidecar lives in the real
        // ~/.workspacer/codex-threads (cleaned up via forget_thread).
        let sid = format!("wks-api-replay-{}", uuid::Uuid::new_v4());
        codex_rollout::record_thread(&sid, &thread);
        let state = test_state();

        // First read replays the rollout, advancing seq past the items.
        let (status, body) =
            request(state.clone(), get(&format!("/sessions/{sid}/conversation"))).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["seq"], 2);
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["text"], "hi");
        assert_eq!(items[1]["text"], "OK");

        // Second read serves the SAME snapshot — no re-append.
        let (status, body) =
            request(state.clone(), get(&format!("/sessions/{sid}/conversation"))).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["seq"], 2, "replay pushed exactly once");
        assert_eq!(v["items"].as_array().unwrap().len(), 2);

        // A session with a live in-memory conversation is never touched by
        // replay, even though its sidecar points at the same rollout.
        let sid_live = format!("wks-api-replay-{}", uuid::Uuid::new_v4());
        codex_rollout::record_thread(&sid_live, &thread);
        state.conv.push(
            &sid_live,
            vec![ConversationItem::AssistantText {
                text: "in-memory".into(),
                timestamp: None,
            }],
        );
        let (status, body) = request(
            state.clone(),
            get(&format!("/sessions/{sid_live}/conversation")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["seq"], 1);
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "non-empty conversation is left alone");
        assert_eq!(items[0]["text"], "in-memory");

        match prev {
            Some(val) => std::env::set_var("CODEX_HOME", val),
            None => std::env::remove_var("CODEX_HOME"),
        }
        codex_rollout::forget_thread(&sid);
        codex_rollout::forget_thread(&sid_live);
        let _ = std::fs::remove_dir_all(&codex_home);
    }

    // --- /input -------------------------------------------------------------

    #[tokio::test]
    async fn post_input_without_wrapper_is_404() {
        let req = post_json("/sessions/nope/input", json!({ "text": "hi" }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"no wrapper attached to that session");
    }

    #[tokio::test]
    async fn post_input_with_neither_text_nor_bytes_is_400() {
        let state = test_state();
        let (h, _rx) = wrapper();
        state.store.register_wrapper("sess-1", "/w", h);
        let req = post_json("/sessions/sess-1/input", json!({ "newline": true }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, b"expected `text` or `bytes_b64`");
    }

    #[tokio::test]
    async fn post_input_bad_base64_is_400() {
        let state = test_state();
        let (h, _rx) = wrapper();
        state.store.register_wrapper("sess-1", "/w", h);
        let req = post_json(
            "/sessions/sess-1/input",
            json!({ "bytes_b64": "not!base64!" }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, b"bad base64");
    }

    #[tokio::test]
    async fn post_input_text_forwards_a_frame() {
        let state = test_state();
        let (h, mut rx) = wrapper();
        state.store.register_wrapper("sess-1", "/w", h);
        let req = post_json(
            "/sessions/sess-1/input",
            json!({ "text": "hi", "newline": false }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["bytes"], 2);
        assert_eq!(next_input(&mut rx), b"hi");
    }

    // --- /message -----------------------------------------------------------

    #[tokio::test]
    async fn post_message_unknown_session_is_404() {
        let req = post_json("/sessions/nope/message", json!({ "text": "hi" }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"session not found");
    }

    #[tokio::test]
    async fn post_message_on_stopped_session_conflicts() {
        // register + deregister leaves a resumable but Stopped session (no managed
        // input channel), which is the only mode that rejects a chat message.
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        state.store.deregister_managed("sess-1");
        let req = post_json("/sessions/sess-1/message", json!({ "text": "hi" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert!(v["error"].is_string());
        assert_eq!(v["mode"], "stopped");
        assert_eq!(v["expected"], "input");
    }

    #[tokio::test]
    async fn post_message_on_managed_session_forwards_prompt() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        state.store.register_managed_input("sess-1", tx);
        let req = post_json(
            "/sessions/sess-1/message",
            json!({ "text": "do the thing" }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ok"], true);
        assert_eq!(rx.try_recv().unwrap(), "do the thing");
    }

    // --- /approve -----------------------------------------------------------

    #[tokio::test]
    async fn post_approve_unknown_session_is_404() {
        let req = post_json("/sessions/nope/approve", json!({ "decision": "yes" }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"session not found");
    }

    #[tokio::test]
    async fn post_approve_when_not_awaiting_approval_conflicts() {
        // A live session sitting at the prompt (Input) is not awaiting approval.
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let req = post_json("/sessions/sess-1/approve", json!({ "decision": "yes" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["expected"], "approval");
    }

    #[tokio::test]
    async fn post_approve_bad_decision_is_400() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        state
            .store
            .set_managed_mode("sess-1", SessionMode::Approval, None);
        let req = post_json("/sessions/sess-1/approve", json!({ "decision": "maybe" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, b"expected `decision` (yes|no|always)");
    }

    #[tokio::test]
    async fn post_approve_routes_to_managed_decision_channel() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        state
            .store
            .set_managed_mode("sess-1", SessionMode::Approval, None);
        let (tx, mut rx) = mpsc::unbounded_channel::<bool>();
        state.store.register_managed_decision("sess-1", tx);
        let req = post_json("/sessions/sess-1/approve", json!({ "decision": "yes" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["managed"], true);
        assert_eq!(v["approve"], true);
        assert!(rx.try_recv().unwrap());
    }

    // --- /decide ------------------------------------------------------------

    #[tokio::test]
    async fn post_decide_with_no_parked_decision_conflicts() {
        // Nothing parked (no gate, no PreToolUse) → the resolve fails as 409.
        let req = post_json(
            "/sessions/nope/decide",
            json!({ "body": { "decision": "approve" } }),
        );
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert!(v["error"].is_string());
    }

    #[tokio::test]
    async fn post_decide_resolves_a_parked_decision() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        // Park a decision the handler will resolve.
        let mut parked = state
            .store
            .park_decision("sess-1", Some("Bash".into()), json!({}));
        let req = post_json(
            "/sessions/sess-1/decide",
            json!({ "body": { "decision": "approve" } }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ok"], true);
        // The awaiter received exactly the body we posted.
        assert_eq!(parked.try_recv().unwrap(), json!({ "decision": "approve" }));
    }

    // --- /gate --------------------------------------------------------------

    #[tokio::test]
    async fn post_gate_unknown_session_is_404() {
        let req = post_json("/sessions/nope/gate", json!({ "on": true }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"session not found");
    }

    #[tokio::test]
    async fn post_gate_toggles_the_session_gate() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let req = post_json("/sessions/sess-1/gate", json!({ "on": true }));
        let (status, body) = request(state.clone(), req).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["gate_enabled"], true);
        assert!(state.store.gate_enabled("sess-1"));
    }

    // --- /answer ------------------------------------------------------------

    #[tokio::test]
    async fn post_answer_unknown_session_is_404() {
        let req = post_json("/sessions/nope/answer", json!({ "option": 1 }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"session not found");
    }

    #[tokio::test]
    async fn post_answer_when_not_asking_conflicts() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let req = post_json("/sessions/sess-1/answer", json!({ "option": 1 }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["expected"], "question");
    }

    #[tokio::test]
    async fn post_answer_option_out_of_range_is_400() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        state
            .store
            .set_managed_mode("sess-1", SessionMode::Question, None);
        let (h, _rx) = wrapper();
        state.store.attach_pty("sess-1", h);
        let req = post_json("/sessions/sess-1/answer", json!({ "option": 10 }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, b"option must be 1-9");
    }

    #[tokio::test]
    async fn post_answer_option_forwards_keystrokes() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        state
            .store
            .set_managed_mode("sess-1", SessionMode::Question, None);
        let (h, mut rx) = wrapper();
        state.store.attach_pty("sess-1", h);
        let req = post_json("/sessions/sess-1/answer", json!({ "option": 2 }));
        let (status, _body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        // Picker answer is the option digit plus a submitting CR.
        assert_eq!(next_input(&mut rx), b"2\r");
    }

    // --- /signal ------------------------------------------------------------

    #[tokio::test]
    async fn post_signal_without_wrapper_is_404() {
        let req = post_json("/sessions/nope/signal", json!({ "signal": "SIGINT" }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"no wrapper attached");
    }

    #[tokio::test]
    async fn post_signal_forwards_to_wrapper() {
        let state = test_state();
        let (h, mut rx) = wrapper();
        state.store.register_wrapper("sess-1", "/w", h);
        let req = post_json("/sessions/sess-1/signal", json!({ "signal": "SIGINT" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ok"], true);
        match rx.try_recv().expect("a signal frame") {
            WrapperMessage::Signal { signal } => {
                assert_eq!(signal, crate::protocol::Signal::Sigint)
            }
            other => panic!("expected Signal frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn post_signal_terminate_stops_managed_session() {
        // A managed SIGTERM tears down the driver loop instead of poking a TUI.
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        state.store.register_managed_input("sess-1", tx);
        let req = post_json("/sessions/sess-1/signal", json!({ "signal": "SIGTERM" }));
        let (status, body) = request(state.clone(), req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ok"], true);
        // The prompt channel was dropped, so the session is no longer managed.
        assert!(!state.store.is_managed("sess-1"));
    }

    // --- /resize ------------------------------------------------------------

    #[tokio::test]
    async fn post_resize_without_wrapper_is_404() {
        let req = post_json("/sessions/nope/resize", json!({ "cols": 80, "rows": 24 }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"no wrapper attached");
    }

    #[tokio::test]
    async fn post_resize_forwards_and_records_size() {
        let state = test_state();
        let (h, mut rx) = wrapper();
        state.store.register_wrapper("sess-1", "/w", h);
        let req = post_json(
            "/sessions/sess-1/resize",
            json!({ "cols": 120, "rows": 40 }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["cols"], 120);
        assert_eq!(v["rows"], 40);
        match rx.try_recv().expect("a resize frame") {
            WrapperMessage::Resize { cols, rows } => {
                assert_eq!((cols, rows), (120, 40))
            }
            other => panic!("expected Resize frame, got {other:?}"),
        }
    }

    // --- /permission-mode ---------------------------------------------------

    /// Mark an already-registered session as managed (adapter-driven) by giving
    /// it a prompt channel — `is_managed` keys off this map, not `register_managed`.
    /// The receiver is returned so the caller keeps it alive if it matters.
    fn mark_managed(store: &SessionStore, id: &str) -> mpsc::UnboundedReceiver<String> {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input(id, tx);
        rx
    }

    #[tokio::test]
    async fn post_permission_mode_unknown_session_is_404() {
        // Non-managed path: a valid mode string on an unknown session → NoSession,
        // which the handler renders as a JSON error body (not plain text).
        let req = post_json(
            "/sessions/nope/permission-mode",
            json!({ "mode": "default" }),
        );
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("session not found"));
    }

    #[tokio::test]
    async fn post_permission_mode_unknown_mode_is_400() {
        let req = post_json("/sessions/nope/permission-mode", json!({ "mode": "bogus" }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("bogus"));
    }

    #[tokio::test]
    async fn post_permission_mode_managed_unknown_mode_is_400() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let _rx = mark_managed(&state.store, "sess-1");
        let req = post_json(
            "/sessions/sess-1/permission-mode",
            json!({ "mode": "plan" }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert!(v["error"].as_str().unwrap().contains("ask"));
    }

    #[tokio::test]
    async fn post_permission_mode_managed_yolo_flips_live_flag() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let _rx = mark_managed(&state.store, "sess-1");
        let live = Arc::new(AtomicBool::new(false));
        state
            .store
            .register_managed_yolo("sess-1", live.clone(), false);
        let req = post_json(
            "/sessions/sess-1/permission-mode",
            json!({ "mode": "yolo" }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["mode"], "yolo");
        assert!(live.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn post_permission_mode_managed_ask_conflicts_when_spawned_yolo() {
        // Spawned in bypass mode: yolo→ask can't work, surfaced as 409.
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let _rx = mark_managed(&state.store, "sess-1");
        let live = Arc::new(AtomicBool::new(true));
        state.store.register_managed_yolo("sess-1", live, true);
        let req = post_json("/sessions/sess-1/permission-mode", json!({ "mode": "ask" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], false);
    }

    // --- /model -------------------------------------------------------------

    #[tokio::test]
    async fn post_model_without_fields_is_400() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let req = post_json("/sessions/sess-1/model", json!({}));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], false);
    }

    #[tokio::test]
    async fn post_model_on_pty_session_conflicts() {
        // A non-managed (PTY) session switches models via the /model slash command
        // on the message path, not this endpoint.
        let state = test_state();
        let (h, _rx) = wrapper();
        state.store.register_wrapper("sess-1", "/w", h);
        let req = post_json("/sessions/sess-1/model", json!({ "model": "opus" }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert!(v["error"].as_str().unwrap().contains("slash command"));
    }

    #[tokio::test]
    async fn post_model_routes_to_switch_channel() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let _in = mark_managed(&state.store, "sess-1");
        let (tx, mut rx) = mpsc::unbounded_channel::<ModelSwitch>();
        state.store.register_managed_model_switch("sess-1", tx);
        let req = post_json(
            "/sessions/sess-1/model",
            json!({ "model": "gpt-5.5", "effort": "high" }),
        );
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["model"], "gpt-5.5");
        let switch = rx.try_recv().unwrap();
        assert_eq!(switch.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(switch.effort.as_deref(), Some("high"));
    }

    // --- /handoff -----------------------------------------------------------

    #[tokio::test]
    async fn post_handoff_with_no_conversation_is_404() {
        let req = post_json("/sessions/nope/handoff", json!({ "no_persist": true }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], false);
    }

    #[tokio::test]
    async fn post_handoff_builds_a_brief_from_the_conversation() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        state.conv.push(
            "sess-1",
            vec![
                ConversationItem::UserMessage {
                    text: "add a test".into(),
                    timestamp: None,
                },
                ConversationItem::AssistantText {
                    text: "done".into(),
                    timestamp: None,
                },
            ],
        );
        // no_persist keeps the test off disk (~/.workspacer/handoffs).
        let req = post_json("/sessions/sess-1/handoff", json!({ "no_persist": true }));
        let (status, body) = request(state, req).await;
        assert_eq!(status, StatusCode::OK);
        let v = serde_json::from_slice::<Value>(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["markdown"].as_str().unwrap().contains("add a test"));
        assert_eq!(v["path"], Value::Null);
    }

    // --- spawn validation ---------------------------------------------------

    #[tokio::test]
    async fn spawn_empty_argv_is_400() {
        let req = post_json("/sessions/spawn", json!({ "argv": [], "cwd": "/tmp" }));
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, b"argv must not be empty");
    }

    #[tokio::test]
    async fn spawn_missing_required_field_is_4xx() {
        // No `cwd` → the JSON body fails to deserialize; axum rejects with a 4xx
        // (422) long before any PTY work.
        let req = post_json("/sessions/spawn", json!({ "argv": ["claude"] }));
        let (status, _body) = request(test_state(), req).await;
        assert!(status.is_client_error(), "got {status}");
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn spawn_managed_bad_provider_is_400() {
        let req = post_json(
            "/sessions/spawn-managed",
            json!({ "provider": "bogus", "cwd": "/tmp" }),
        );
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(String::from_utf8_lossy(&body).contains("unsupported managed provider"));
    }

    #[tokio::test]
    async fn spawn_managed_missing_provider_is_4xx() {
        let req = post_json("/sessions/spawn-managed", json!({ "cwd": "/tmp" }));
        let (status, _body) = request(test_state(), req).await;
        assert!(status.is_client_error(), "got {status}");
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

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

    // --- CORS + Host guard --------------------------------------------------

    /// Dispatch through the full router and hand back the raw response so a test
    /// can inspect headers (CORS) rather than just status + body.
    async fn response(state: ApiState, req: Request<Body>) -> Response {
        router(state).oneshot(req).await.expect("router responds")
    }

    fn preflight(uri: &str, origin: &str) -> Request<Body> {
        Request::builder()
            .method("OPTIONS")
            .uri(uri)
            .header(header::ORIGIN, origin)
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "content-type")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn cors_preflight_from_foreign_origin_gets_no_allow_origin() {
        // A random website's preflight for a JSON POST to a mutation route must
        // not be reflected — without an allow-origin header the browser never
        // sends the actual request.
        let resp = response(
            test_state(),
            preflight("/sessions/sess-1/message", "http://evil.example.com"),
        )
        .await;
        assert!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none(),
            "foreign origin must not be allowed"
        );
    }

    #[tokio::test]
    async fn cors_preflight_from_loopback_origin_is_allowed() {
        let resp = response(
            test_state(),
            preflight("/sessions/sess-1/message", "http://localhost:5173"),
        )
        .await;
        let acao = resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok());
        assert_eq!(acao, Some("http://localhost:5173"));
    }

    #[tokio::test]
    async fn cors_actual_post_from_foreign_origin_gets_no_allow_origin() {
        let req = Request::builder()
            .method("POST")
            .uri("/sessions/nope/message")
            .header(header::ORIGIN, "http://evil.example.com")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "text": "hi" }).to_string()))
            .unwrap();
        let resp = response(test_state(), req).await;
        assert!(resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test]
    async fn host_guard_rejects_non_loopback_host() {
        // DNS-rebinding defense: a mutation with a foreign Host is refused before
        // it reaches the handler, even though CORS wouldn't see a preflight.
        let req = Request::builder()
            .method("POST")
            .uri("/sessions/nope/message")
            .header("content-type", "application/json")
            .header(header::HOST, "evil.example.com")
            .body(Body::from(json!({ "text": "hi" }).to_string()))
            .unwrap();
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, b"host not allowed");
    }

    #[tokio::test]
    async fn host_guard_allows_loopback_host() {
        let req = Request::builder()
            .uri("/health")
            .header(header::HOST, "127.0.0.1:7891")
            .body(Body::empty())
            .unwrap();
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ok");
    }

    #[test]
    fn allowed_hosts_permits_loopback_and_configured_only() {
        let a = AllowedHosts::new(Some("192.168.1.5".into()));
        assert!(a.permits("127.0.0.1:7891"));
        assert!(a.permits("localhost:7891"));
        assert!(a.permits("[::1]:7891"));
        assert!(a.permits("192.168.1.5:7891"));
        assert!(!a.permits("evil.example.com"));
        // A wildcard bind names no host, so only loopback is accepted.
        let wild = AllowedHosts::new(Some("0.0.0.0".into()));
        assert!(wild.permits("127.0.0.1"));
        assert!(!wild.permits("10.0.0.9:7891"));
    }

    #[test]
    fn loopback_origin_predicate_matches_only_loopback() {
        assert!(is_loopback_origin(&HeaderValue::from_static(
            "http://localhost:5173"
        )));
        assert!(is_loopback_origin(&HeaderValue::from_static(
            "http://127.0.0.1:7891"
        )));
        assert!(is_loopback_origin(&HeaderValue::from_static(
            "https://[::1]:7891"
        )));
        assert!(!is_loopback_origin(&HeaderValue::from_static(
            "http://evil.example.com"
        )));
        // Opaque/`null` origins (sandboxed iframes) are not loopback.
        assert!(!is_loopback_origin(&HeaderValue::from_static("null")));
    }

    // --- session-id path-traversal guard ------------------------------------

    #[test]
    fn valid_session_id_accepts_ids_rejects_traversal() {
        assert!(valid_session_id("abc12345-1234-5678-9abc-def012345678"));
        assert!(valid_session_id("sess-1"));
        assert!(valid_session_id("rollout.2026.jsonl_id")); // dots/underscores ok
        assert!(!valid_session_id(""));
        assert!(!valid_session_id("../../etc/passwd"));
        assert!(!valid_session_id(".."));
        assert!(!valid_session_id("a/b"));
        assert!(!valid_session_id("a\\b"));
        assert!(!valid_session_id("foo..bar")); // any `..` segment is refused
    }

    #[tokio::test]
    async fn get_transcript_with_traversal_id_is_rejected() {
        // `%2e%2e%2f…` decodes to `../…`. Whether the router refuses the encoded
        // separators (404) or the handler's id guard does (400), the request is
        // rejected with a 4xx and never reaches the filesystem. A plain id like
        // `sess-1` still 200s (see get_transcript_with_no_file_returns_empty).
        let (status, _body) = request(
            test_state(),
            get("/sessions/..%2f..%2f..%2f..%2fetc%2fpasswd/transcript"),
        )
        .await;
        assert!(status.is_client_error(), "got {status}");
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn get_transcript_with_dotdot_segment_id_is_400() {
        // A single-segment id carrying `..` routes cleanly to the handler, so the
        // id guard itself is exercised and returns 400.
        let (status, body) = request(test_state(), get("/sessions/..evil/transcript")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, b"invalid session id");
    }

    #[tokio::test]
    async fn post_handoff_with_dotdot_segment_id_is_400() {
        let (status, body) = request(
            test_state(),
            post_json("/sessions/..evil/handoff", json!({ "no_persist": true })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, b"invalid session id");
    }
}

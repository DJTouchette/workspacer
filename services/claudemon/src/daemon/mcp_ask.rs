//! Minimal MCP (Model Context Protocol) streamable-HTTP endpoint exposing one
//! tool: `AskUserQuestion`.
//!
//! Why this exists: Claude Code has a native AskUserQuestion tool whose
//! PreToolUse hook the daemon already turns into `Pending::Question` (the GUI
//! renders a picker and answers via `POST /sessions/:id/answer`). Other
//! agents — Codex in particular — have no such tool, so their questions arrive
//! as plain prose the user has to answer by typing. Registering this endpoint
//! as an MCP server in the agent's config gives any MCP-speaking provider the
//! same structured question channel: the tool call lands here, we park the
//! session in `SessionMode::Question` (which the GUI already knows how to
//! render), block until `/answer` resolves it through the store's managed
//! answer channel, and hand the chosen options back as the tool result.
//!
//! Transport notes: this is the "streamable HTTP" MCP transport in its
//! simplest legal form — one JSON-RPC request per POST, one JSON response, no
//! SSE stream (we never need server-initiated messages). The session identity
//! rides in the URL (`/mcp/ask/:session_id`) rather than an MCP header so a
//! spawner can point each agent at its own session with nothing but a URL.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::session::state::{Pending, PendingQuestion};
use crate::session::{ManagedAnswer, SessionMode, SessionStore};

/// How long a question may sit unanswered before we give up and unblock the
/// agent. Deliberately generous — the whole point is that the user may be
/// away; a real answer hours later still beats the agent guessing. The drop
/// guard keeps a timeout from wedging the session in `Question`.
const ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Fallback protocol version when the client doesn't state one. Matches the
/// MCP revision this endpoint was written against.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

/// JSON-RPC 2.0 error codes (the only ones we emit).
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// `POST /mcp/ask/:session_id` — the whole MCP server. One JSON-RPC message
/// in, one out; notifications get an empty 202 per the streamable-HTTP spec.
pub async fn handle(
    State(store): State<SessionStore>,
    Path(session_id): Path<String>,
    Json(req): Json<Value>,
) -> Response {
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let id = req.get("id").cloned().filter(|v| !v.is_null());

    // Notifications (no id) never get a JSON-RPC response — the transport
    // acknowledges them with an empty 202. `notifications/initialized` is the
    // one we expect, but any id-less message takes this path.
    let Some(id) = id else {
        return StatusCode::ACCEPTED.into_response();
    };

    match method {
        "initialize" => rpc_result(id, initialize_result(&req)),
        "tools/list" => rpc_result(id, json!({ "tools": [ask_user_question_tool()] })),
        "tools/call" => tools_call(&store, &session_id, id, &req).await,
        _ => rpc_error(id, METHOD_NOT_FOUND, &format!("method not found: {method}")),
    }
}

/// The `initialize` result. We echo whatever protocol version the client
/// asked for — this endpoint's surface (tools only, single-response POSTs) is
/// valid under every published revision, so agreeing is always safe.
fn initialize_result(req: &Value) -> Value {
    let version = req
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "workspacer-ask",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

/// The one tool we serve. The input schema mirrors Claude Code's native
/// AskUserQuestion exactly, so questions parse straight into
/// [`PendingQuestion`] and render in the GUI identically to Claude's.
fn ask_user_question_tool() -> Value {
    json!({
        "name": "AskUserQuestion",
        "description": "Ask the user one or more multiple-choice questions through the workspacer GUI and wait for their answer. The call blocks until the user responds, so use it whenever a decision is genuinely the user's to make (ambiguous requirements, destructive actions, a fork in the road you should not pick alone) rather than guessing. Each question offers a short list of options; the result reports the option the user chose (or their free-form text) per question.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string" },
                            "header": { "type": "string" },
                            "multiSelect": { "type": "boolean" },
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string" },
                                        "description": { "type": "string" }
                                    },
                                    "required": ["label"]
                                }
                            }
                        },
                        "required": ["question", "options"]
                    }
                }
            },
            "required": ["questions"]
        }
    })
}

/// Restores the session out of `Question` when the ask ends for *any* reason.
///
/// The await below can end four ways: answered, timed out, channel closed, or
/// the HTTP request dropped mid-flight (agent killed, connection reset). The
/// first three run code we control, but the fourth just drops the future — so
/// the restore lives in `Drop`, and the completed paths call [`finish`]
/// (Self::finish) which restores once and defuses the drop arm. Without this,
/// an aborted ask would wedge the session in `Question` with a picker no one
/// can satisfy.
struct QuestionGuard {
    store: SessionStore,
    session_id: String,
    tx: mpsc::UnboundedSender<ManagedAnswer>,
    defused: bool,
}

impl QuestionGuard {
    /// Restore `Responding`/no-pending and drop our answer channel (only if
    /// it's still ours — see `unregister_managed_answer_if`), then defuse.
    fn finish(&mut self) {
        if self.defused {
            return;
        }
        self.defused = true;
        self.store
            .unregister_managed_answer_if(&self.session_id, &self.tx);
        self.store
            .set_managed_mode(&self.session_id, SessionMode::Responding, None);
    }
}

impl Drop for QuestionGuard {
    fn drop(&mut self) {
        self.finish();
    }
}

/// `tools/call` — parse the questions, park the session in `Question`, block
/// until the GUI answers (or we give up), and report the choices.
async fn tools_call(store: &SessionStore, session_id: &str, id: Value, req: &Value) -> Response {
    let tool = req.pointer("/params/name").and_then(Value::as_str);
    if tool != Some("AskUserQuestion") {
        return rpc_error(
            id,
            INVALID_PARAMS,
            &format!("unknown tool: {}", tool.unwrap_or("(none)")),
        );
    }
    let arguments = req
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or(Value::Null);
    // The wire spells `multiSelect`; PendingQuestion's serde alias absorbs it.
    let questions: Vec<PendingQuestion> = match arguments
        .get("questions")
        .cloned()
        .ok_or_else(|| "missing required argument: questions".to_string())
        .and_then(|q| serde_json::from_value(q).map_err(|e| format!("invalid questions: {e}")))
    {
        Ok(q) => q,
        Err(msg) => return rpc_error(id, INVALID_PARAMS, &msg),
    };
    if questions.is_empty() {
        return rpc_error(id, INVALID_PARAMS, "questions must not be empty");
    }
    if store.get(session_id).is_none() {
        return rpc_error(
            id,
            INVALID_PARAMS,
            &format!("unknown session: {session_id}"),
        );
    }

    // Park the question: register the structural answer channel first so an
    // instant `/answer` (racing the mode broadcast) can't miss it, then flip
    // the mode — the broadcast is what makes the GUI render the picker.
    let (tx, mut rx) = mpsc::unbounded_channel::<ManagedAnswer>();
    store.register_managed_answer(session_id, tx.clone());
    let mut guard = QuestionGuard {
        store: store.clone(),
        session_id: session_id.to_string(),
        tx,
        defused: false,
    };
    store.set_managed_mode(
        session_id,
        SessionMode::Question,
        Some(Pending::Question {
            questions: questions.clone(),
            raw: arguments,
        }),
    );

    match tokio::time::timeout(ANSWER_TIMEOUT, rx.recv()).await {
        Ok(Some(answer)) => {
            guard.finish();
            rpc_result(
                id,
                json!({
                    "content": [{ "type": "text", "text": summarize(&questions, &answer) }],
                    "isError": false,
                }),
            )
        }
        // Channel closed (session torn down mid-question) or nobody answered
        // in time. Tool-level failures are MCP *results* with isError, not
        // JSON-RPC errors — the agent should read and react, not crash.
        Ok(None) => {
            guard.finish();
            tool_error(id, "the session was closed before the user answered")
        }
        Err(_) => {
            guard.finish();
            tool_error(id, "timed out waiting for the user to answer")
        }
    }
}

/// Render the user's answer as one line per question, resolving option
/// numbers to their labels so the agent reads the choice, not an index.
///
/// `ManagedAnswer` carries either `answers` (one entry per question: an
/// option number as a string, or free text) or a single `option`/`text` for
/// the one-question case — the same vocabulary `/answer` accepts.
fn summarize(questions: &[PendingQuestion], answer: &ManagedAnswer) -> String {
    let mut lines = Vec::with_capacity(questions.len());
    for (i, q) in questions.iter().enumerate() {
        let raw = match &answer.answers {
            Some(answers) => answers.get(i).cloned(),
            // Single-answer shape only addresses the first question.
            None if i == 0 => answer
                .option
                .map(|o| o.to_string())
                .or_else(|| answer.text.clone()),
            None => None,
        };
        let chosen = match raw {
            // An option number in range means "that option's label";
            // anything else is free text and passes through verbatim.
            Some(r) => match r.trim().parse::<usize>() {
                Ok(n) if (1..=q.options.len()).contains(&n) => q.options[n - 1].label.clone(),
                _ => r,
            },
            None => "(no answer)".to_string(),
        };
        lines.push(format!("{}: {}", q.question, chosen));
    }
    lines.join("\n")
}

/// A tool-level failure: JSON-RPC success carrying `isError: true` content.
fn tool_error(id: Value, message: &str) -> Response {
    rpc_result(
        id,
        json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
    )
}

fn rpc_result(id: Value, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::api::{router, ApiState};
    use crate::session::ConversationStore;
    use crate::store::Db;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    /// Mirror of api.rs's test_state: a throwaway on-disk db + fresh stores.
    fn test_state() -> ApiState {
        let mut db_path = std::env::temp_dir();
        db_path.push(format!(
            "claudemon-mcp-ask-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        ApiState {
            store: SessionStore::new(),
            db: Db::open(&db_path).expect("open test db"),
            conv: ConversationStore::new(),
        }
    }

    async fn request(state: ApiState, req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let resp = router(state).oneshot(req).await.expect("router responds");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("collect body");
        (status, bytes.to_vec())
    }

    fn post_rpc(session_id: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(format!("/mcp/ask/{session_id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn rpc(state: ApiState, session_id: &str, body: Value) -> Value {
        let (status, bytes) = request(state, post_rpc(session_id, body)).await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_slice(&bytes).expect("json-rpc response body")
    }

    #[tokio::test]
    async fn get_is_method_not_allowed() {
        let req = Request::builder()
            .uri("/mcp/ask/sess-1")
            .body(Body::empty())
            .unwrap();
        let (status, _) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn initialize_echoes_version_and_names_the_server() {
        let v = rpc(
            test_state(),
            "sess-1",
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2024-11-05", "capabilities": {} },
            }),
        )
        .await;
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(v["result"]["serverInfo"]["name"], "workspacer-ask");
        assert!(v["result"]["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn initialize_without_version_uses_the_default() {
        let v = rpc(
            test_state(),
            "sess-1",
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        )
        .await;
        assert_eq!(v["result"]["protocolVersion"], DEFAULT_PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn initialized_notification_is_202() {
        let req = post_rpc(
            "sess-1",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        );
        let (status, body) = request(test_state(), req).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn tools_list_offers_ask_user_question() {
        let v = rpc(
            test_state(),
            "sess-1",
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .await;
        let tools = v["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "AskUserQuestion");
        assert_eq!(
            tools[0]["inputSchema"]["required"],
            json!(["questions"]),
            "schema requires questions"
        );
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() {
        let v = rpc(
            test_state(),
            "sess-1",
            json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }),
        )
        .await;
        assert_eq!(v["error"]["code"], METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn tools_call_missing_questions_is_invalid_params() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");
        let v = rpc(
            state,
            "sess-1",
            json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": { "name": "AskUserQuestion", "arguments": {} },
            }),
        )
        .await;
        assert_eq!(v["error"]["code"], INVALID_PARAMS);
        assert!(v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("questions"));
    }

    #[tokio::test]
    async fn tools_call_unknown_session_is_an_error() {
        let v = rpc(
            test_state(),
            "nope",
            json!({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": { "name": "AskUserQuestion", "arguments": {
                    "questions": [{ "question": "?", "options": [{ "label": "a" }] }],
                }},
            }),
        )
        .await;
        assert_eq!(v["error"]["code"], INVALID_PARAMS);
        assert!(v["error"]["message"].as_str().unwrap().contains("nope"));
    }

    #[tokio::test]
    async fn tools_call_blocks_until_answered_then_reports_the_choice() {
        let state = test_state();
        state.store.register_managed("sess-1", "/tmp/proj", "codex");

        // The call blocks on the answer channel, so dispatch it as a task…
        let call_state = state.clone();
        let call = tokio::spawn(async move {
            rpc(
                call_state,
                "sess-1",
                json!({
                    "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                    "params": { "name": "AskUserQuestion", "arguments": {
                        "questions": [{
                            "question": "Which db?",
                            "header": "Database",
                            "multiSelect": false,
                            "options": [
                                { "label": "sqlite", "description": "file-backed" },
                                { "label": "postgres" }
                            ],
                        }],
                    }},
                }),
            )
            .await
        });

        // …wait for it to park the session in Question…
        let mode = |s: &ApiState| s.store.get("sess-1").map(|st| st.mode);
        for _ in 0..200 {
            if mode(&state) == Some(SessionMode::Question) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let pending = state.store.get("sess-1").unwrap();
        assert_eq!(pending.mode, SessionMode::Question, "question is parked");

        // …answer through the same store path `POST /sessions/:id/answer` uses.
        assert!(state.store.submit_managed_answer(
            "sess-1",
            ManagedAnswer {
                option: Some(2),
                text: None,
                answers: None,
            },
        ));

        let v = call.await.expect("call task");
        assert_eq!(v["result"]["isError"], false);
        assert_eq!(
            v["result"]["content"][0]["text"], "Which db?: postgres",
            "option 2 resolves to its label"
        );
        // The ask must leave the session out of Question — back to Responding.
        assert_eq!(mode(&state), Some(SessionMode::Responding));
    }

    #[test]
    fn summarize_maps_multi_answers_and_free_text() {
        let questions: Vec<PendingQuestion> = serde_json::from_value(json!([
            { "question": "Which db?", "options": [{ "label": "sqlite" }, { "label": "postgres" }] },
            { "question": "Name?", "options": [{ "label": "auto" }] },
        ]))
        .unwrap();
        let answer = ManagedAnswer {
            option: None,
            text: None,
            answers: Some(vec!["1".into(), "call it claudemon".into()]),
        };
        assert_eq!(
            summarize(&questions, &answer),
            "Which db?: sqlite\nName?: call it claudemon"
        );
    }
}

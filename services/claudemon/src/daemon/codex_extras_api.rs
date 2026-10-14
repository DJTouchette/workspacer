//! HTTP handlers for the Codex app-server extras (registered in `api.rs`):
//! review, thread goals, thread naming and Windows sandbox setup. Each one
//! hands a [`CodexOp`] to the session's live codex ws driver and answers with
//! Codex's own verdict — the JSON-RPC result, or its error text (a 0.153
//! build without a method answers "unsupported" here, never a hang).

use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use super::api::valid_session_id;
use crate::session::codex_controls::{CodexControl, CodexOp};
use crate::session::SessionStore;

/// Longest free text accepted for a goal objective, review instructions or a
/// thread name. Codex has its own limits; this only bounds what we relay.
const MAX_TEXT: usize = 4_000;

fn refuse(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(json!({ "ok": false, "error": error.into() }))).into_response()
}

fn bounded_text(body: &Value, key: &str, max: usize) -> Result<Option<String>, String> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                Err(format!("{key} must not be empty"))
            } else if s.len() > max {
                Err(format!("{key} is longer than {max} bytes"))
            } else {
                Ok(Some(s.to_string()))
            }
        }
        Some(_) => Err(format!("{key} must be text")),
    }
}

/// Codex's `ReviewTarget`, re-built from only the fields each variant names
/// so nothing else the caller sent reaches the app-server.
pub(crate) fn review_target(target: &Value) -> Result<Value, String> {
    let text = |key: &str, max: usize| -> Result<String, String> {
        bounded_text(target, key, max)?.ok_or_else(|| format!("target.{key} is required"))
    };
    match target.get("type").and_then(Value::as_str) {
        Some("uncommittedChanges") => Ok(json!({ "type": "uncommittedChanges" })),
        Some("baseBranch") => Ok(json!({ "type": "baseBranch", "branch": text("branch", 255)? })),
        Some("commit") => {
            let sha = text("sha", 64)?;
            if !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("target.sha must be a hex commit id".into());
            }
            let mut out = json!({ "type": "commit", "sha": sha });
            if let Some(title) = bounded_text(target, "title", 300)? {
                out["title"] = json!(title);
            }
            Ok(out)
        }
        Some("custom") => Ok(json!({
            "type": "custom",
            "instructions": text("instructions", MAX_TEXT)?,
        })),
        _ => {
            Err("target.type must be uncommittedChanges, baseBranch, commit or custom".to_string())
        }
    }
}

/// Codex `ThreadGoalStatus` values a client may set.
const GOAL_STATUSES: &[&str] = &[
    "active",
    "paused",
    "blocked",
    "usageLimited",
    "budgetLimited",
    "complete",
];

pub(crate) fn goal_op(body: &Value) -> Result<CodexOp, String> {
    let objective = bounded_text(body, "objective", MAX_TEXT)?;
    let status = bounded_text(body, "status", 32)?;
    if let Some(status) = &status {
        if !GOAL_STATUSES.contains(&status.as_str()) {
            return Err(format!(
                "status must be one of {}",
                GOAL_STATUSES.join(", ")
            ));
        }
    }
    let budget = body.get("tokenBudget").or_else(|| body.get("token_budget"));
    let token_budget = match budget {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => match v.as_i64().filter(|n| *n > 0) {
            Some(n) => Some(Some(n)),
            None => return Err("tokenBudget must be a positive integer or null".into()),
        },
    };
    if objective.is_none() && status.is_none() && token_budget.is_none() {
        return Err("setGoal needs an objective, a status or a tokenBudget".into());
    }
    Ok(CodexOp::SetGoal {
        objective,
        status,
        token_budget,
    })
}

async fn run(store: &SessionStore, id: &str, op: CodexOp) -> Response {
    if !valid_session_id(id) {
        return (StatusCode::BAD_REQUEST, "invalid session id").into_response();
    }
    if store.get(id).is_none() {
        return refuse(StatusCode::NOT_FOUND, "session not found");
    }
    let Some(control) = store.codex_control(id) else {
        return refuse(
            StatusCode::NOT_IMPLEMENTED,
            "this needs a live Codex app-server session",
        );
    };
    let (reply, verdict) = tokio::sync::oneshot::channel();
    if control.tx.send(CodexControl { op, reply }).is_err() {
        return refuse(StatusCode::GONE, "session driver is gone");
    }
    match tokio::time::timeout(Duration::from_secs(20), verdict).await {
        Ok(Ok(Ok(result))) => Json(json!({ "ok": true, "result": result })).into_response(),
        Ok(Ok(Err(error))) => refuse(StatusCode::BAD_GATEWAY, error),
        Ok(Err(_)) => refuse(StatusCode::GONE, "session driver is gone"),
        Err(_) => refuse(StatusCode::GATEWAY_TIMEOUT, "Codex did not answer"),
    }
}

/// `POST /sessions/:id/review {target, delivery?}` → `review/start`.
pub(crate) async fn post_review(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let target = match review_target(body.get("target").unwrap_or(&Value::Null)) {
        Ok(target) => target,
        Err(error) => return refuse(StatusCode::BAD_REQUEST, error),
    };
    let delivery = match body.get("delivery").and_then(Value::as_str) {
        None => None,
        Some(d @ ("inline" | "detached")) => Some(d.to_string()),
        Some(_) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "delivery must be inline or detached",
            )
        }
    };
    run(&store, &id, CodexOp::Review { target, delivery }).await
}

/// `POST /sessions/:id/goal {objective?, status?, tokenBudget?}` →
/// `thread/goal/set`.
pub(crate) async fn post_goal(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match goal_op(&body) {
        Ok(op) => run(&store, &id, op).await,
        Err(error) => refuse(StatusCode::BAD_REQUEST, error),
    }
}

/// `POST /sessions/:id/goal/clear` → `thread/goal/clear`.
pub(crate) async fn post_goal_clear(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
) -> Response {
    run(&store, &id, CodexOp::ClearGoal).await
}

/// `POST /sessions/:id/thread-name {name}` → `thread/name/set`.
pub(crate) async fn post_thread_name(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match bounded_text(&body, "name", 200) {
        Ok(Some(name)) => run(&store, &id, CodexOp::SetThreadName { name }).await,
        Ok(None) => refuse(StatusCode::BAD_REQUEST, "name is required"),
        Err(error) => refuse(StatusCode::BAD_REQUEST, error),
    }
}

/// `POST /sessions/:id/windows-sandbox/setup {mode?}` →
/// `windowsSandbox/setupStart`. `mode` is Codex's `elevated` (the default,
/// what Codex's own TUI recommends) or `unelevated`.
pub(crate) async fn post_windows_sandbox_setup(
    State(store): State<SessionStore>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or(Value::Null);
    let mode = match body.get("mode").and_then(Value::as_str) {
        None => "elevated".to_string(),
        Some(m @ ("elevated" | "unelevated")) => m.to_string(),
        Some(_) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "mode must be elevated or unelevated",
            )
        }
    };
    run(&store, &id, CodexOp::WindowsSandboxSetup { mode }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_targets_keep_only_their_own_fields() {
        assert_eq!(
            review_target(&json!({"type":"uncommittedChanges","extra":1})).unwrap(),
            json!({"type":"uncommittedChanges"})
        );
        assert_eq!(
            review_target(&json!({"type":"baseBranch","branch":" main "})).unwrap(),
            json!({"type":"baseBranch","branch":"main"})
        );
        assert_eq!(
            review_target(&json!({"type":"commit","sha":"abc123","title":"Fix"})).unwrap(),
            json!({"type":"commit","sha":"abc123","title":"Fix"})
        );
        assert_eq!(
            review_target(&json!({"type":"custom","instructions":"look at auth"})).unwrap(),
            json!({"type":"custom","instructions":"look at auth"})
        );
        for bad in [
            json!(null),
            json!({"type":"everything"}),
            json!({"type":"baseBranch"}),
            json!({"type":"commit","sha":"not hex!"}),
            json!({"type":"custom","instructions":""}),
            json!({"type":"custom","instructions":7}),
        ] {
            assert!(review_target(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn goal_params_distinguish_absent_null_and_set_budgets() {
        assert_eq!(
            goal_op(&json!({"objective":"ship it"})).unwrap(),
            CodexOp::SetGoal {
                objective: Some("ship it".into()),
                status: None,
                token_budget: None
            }
        );
        assert_eq!(
            goal_op(&json!({"tokenBudget":null})).unwrap(),
            CodexOp::SetGoal {
                objective: None,
                status: None,
                token_budget: Some(None)
            }
        );
        assert_eq!(
            goal_op(&json!({"status":"paused","tokenBudget":5000})).unwrap(),
            CodexOp::SetGoal {
                objective: None,
                status: Some("paused".into()),
                token_budget: Some(Some(5000))
            }
        );
        for bad in [
            json!({}),
            json!({"status":"done"}),
            json!({"tokenBudget":-1}),
            json!({"tokenBudget":"lots"}),
            json!({"objective":""}),
        ] {
            assert!(goal_op(&bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn ops_on_a_session_without_a_codex_driver_are_not_implemented() {
        let store = SessionStore::new();
        store.register_managed("s1", "/tmp", "claude");
        let reply = run(&store, "s1", CodexOp::ClearGoal).await;
        assert_eq!(reply.status(), StatusCode::NOT_IMPLEMENTED);
        let reply = run(&store, "nope", CodexOp::ClearGoal).await;
        assert_eq!(reply.status(), StatusCode::NOT_FOUND);
    }
}

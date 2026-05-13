use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use serde_json::json;

use crate::session::{HookEvent, SessionStore};

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
    let state = store.ingest(event);
    (StatusCode::OK, Json(json!({ "ok": true, "status": state.status })))
}

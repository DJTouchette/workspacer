//! Optional network adapter. The embedded runtime never calls this unless a
//! host explicitly requests a listener.
pub(crate) mod policy;
mod web;
use crate::{Connection, Handle, protocol::Frame};
use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

#[derive(Clone)]
struct ServerState {
    hub: Handle,
    token: String,
    scoped_tokens: Option<std::path::PathBuf>,
    policy: policy::Policy,
}
#[derive(Default, Deserialize)]
struct Credential {
    #[serde(default)]
    token: String,
    #[serde(default)]
    peer: String,
    #[serde(default)]
    identity: String,
}

#[derive(Clone)]
pub(crate) struct Credentials {
    pub token: String,
    pub scoped_tokens: Option<std::path::PathBuf>,
}
impl Credentials {
    fn operator(&self, headers: &HeaderMap, query: &str) -> bool {
        let token = if let Some(header) = headers
            .get("authorization")
            .filter(|h| !h.as_bytes().is_empty())
        {
            header
                .to_str()
                .ok()
                .and_then(|header| header.strip_prefix("Bearer "))
                .unwrap_or("")
        } else {
            query
        };
        self.token.is_empty()
            || token == self.token
            || self
                .scoped_tokens
                .as_ref()
                .and_then(|path| crate::auth::Store { path: path.clone() }.lookup(token))
                .is_some_and(|record| record.scope() == Some(crate::auth::Scope::Operator))
    }
}

fn presented<'a>(headers: &'a HeaderMap, query: &'a Credential) -> &'a str {
    if let Some(header) = headers
        .get("authorization")
        .filter(|header| !header.as_bytes().is_empty())
    {
        return header
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or("");
    }
    &query.token
}
fn authorized(state: &ServerState, headers: &HeaderMap, query: &Credential) -> bool {
    let token = presented(headers, query);
    // Never print the presented credential or request URI in diagnostics.
    (!state.token.is_empty() && crate::auth::credential_eq(&state.token, &token))
        || state
            .scoped_tokens
            .as_ref()
            .and_then(|path| crate::auth::Store { path: path.clone() }.lookup(token))
            .is_some_and(|r| r.scope() == Some(crate::auth::Scope::Operator))
}

async fn health(
    State(state): State<ServerState>,
    Query(query): Query<Credential>,
    headers: HeaderMap,
) -> Response {
    if !state.policy.host(&headers, None) {
        return (StatusCode::FORBIDDEN, "host not allowed").into_response();
    }
    if !authorized(&state, &headers, &query) {
        return Json(json!({"status":"ok"})).into_response();
    }
    match state.hub.health().await {
        Ok(value) => Json(value).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "hub stopped").into_response(),
    }
}

async fn bus(
    State(state): State<ServerState>,
    Query(query): Query<Credential>,
    headers: HeaderMap,
    socket: Option<axum::extract::ConnectInfo<policy::Socket>>,
    ws: WebSocketUpgrade,
) -> Response {
    let local = socket.and_then(|socket| socket.0.local);
    if !state.policy.host(&headers, local) || !state.policy.origin(&headers, local) {
        return (StatusCode::FORBIDDEN, "host or origin not allowed").into_response();
    }
    let connection = match state
        .hub
        .connect_authenticated_context(
            presented(&headers, &query).into(),
            query.peer == "1",
            query.identity == "1",
        )
        .await
    {
        Ok(connection) => connection,
        Err(_) => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    };
    ws.max_message_size(64 << 20)
        .max_frame_size(64 << 20)
        .on_upgrade(move |socket| connected(socket, connection))
        .into_response()
}

async fn connected(socket: WebSocket, mut connection: Connection) {
    connection.mark_network_transport();
    let (mut writer, mut reader) = socket.split();
    loop {
        let outgoing = tokio::select! {
            frame = connection.recv() => match frame { Some(f) => f, None => break },
            incoming = reader.next() => match incoming {
                Some(Ok(Message::Text(text))) => match Frame::decode(text.as_bytes()) {
                    Ok(frame) => { if connection.send(frame).is_err() { break; } continue; }
                    Err(_) => Frame::error("", "bad frame: invalid JSON frame"),
                },
                Some(Ok(Message::Binary(bytes))) => match Frame::decode(&bytes) {
                    Ok(frame) => { if connection.send(frame).is_err() { break; } continue; }
                    Err(_) => Frame::error("", "bad frame: invalid JSON frame"),
                },
                Some(Ok(Message::Ping(bytes))) => {
                    if !matches!(tokio::time::timeout(Duration::from_secs(5), writer.send(Message::Pong(bytes))).await, Ok(Ok(()))) { break; }
                    continue;
                }
                Some(Ok(Message::Pong(_))) => continue,
                _ => break,
            },
        };
        let Ok(text) = serde_json::to_string(&outgoing) else {
            break;
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(5), writer.send(Message::Text(text))).await,
            Ok(Ok(()))
        ) {
            break;
        }
    }
    if let Some(code) = connection.close_code() {
        let frame = axum::extract::ws::CloseFrame {
            code,
            reason: "machine stopping; reconnect only to wake".into(),
        };
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            writer.send(Message::Close(Some(frame))),
        )
        .await;
    }
    let _ = tokio::time::timeout(Duration::from_millis(100), writer.close()).await;
}

pub(crate) async fn serve(
    listener: tokio::net::TcpListener,
    hub: Handle,
    token: String,
    scoped_tokens: Option<std::path::PathBuf>,
    plugins: Option<Router>,
    webapp_dir: Option<std::path::PathBuf>,
    trusted_hosts: Vec<String>,
) -> anyhow::Result<()> {
    let policy = policy::Policy::new(listener.local_addr()?.ip(), &trusted_hosts)?;
    let assets = web::router(
        Credentials {
            token: token.clone(),
            scoped_tokens: scoped_tokens.clone(),
        },
        webapp_dir,
    )?;
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/bus", get(bus))
        .with_state(ServerState {
            hub,
            token,
            scoped_tokens,
            policy: policy.clone(),
        })
        .merge(assets);
    if let Some(plugins) = plugins {
        router = router.merge(plugins);
    }
    router = router.layer(axum::middleware::from_fn_with_state(policy, policy::guard));
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<policy::Socket>(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

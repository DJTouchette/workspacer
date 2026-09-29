//! Legacy HTTP+SSE framing around the rmcp service engine. Every POST carries
//! freshly checked HTTP identity; session IDs never confer authority.
use super::{Adapter, Gate, Identity};
use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use futures_util::Stream;
use rmcp::{
    RoleServer, ServiceExt,
    model::{ClientJsonRpcMessage, GetExtensions, ServerJsonRpcMessage},
    transport::Transport,
};
use std::{
    collections::HashMap,
    convert::Infallible,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, watch};
const QUEUE: usize = 64;
const MAX_SESSIONS: usize = 1024;
struct Session {
    fingerprint: String,
    host: bool,
    scope: crate::auth::Scope,
    guest: bool,
    incoming: mpsc::Sender<ClientJsonRpcMessage>,
    outgoing: mpsc::WeakSender<ServerJsonRpcMessage>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
#[derive(Clone)]
struct Registry {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    adapter: Adapter,
    gate: Gate,
}
pub(super) struct LegacySse {
    registry: Registry,
}
impl LegacySse {
    pub(super) fn new(adapter: Adapter, gate: Gate) -> Self {
        Self {
            registry: Registry {
                sessions: Arc::new(Mutex::new(HashMap::new())),
                adapter,
                gate,
            },
        }
    }
    /// Merge before the existing authentication middleware.
    pub(super) fn router(&self) -> Router {
        Router::new()
            .route("/sse", get(open).post(message))
            .with_state(self.registry.clone())
    }
    pub(super) async fn shutdown(&self) {
        let sessions: Vec<_> = self
            .registry
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, s)| s)
            .collect();
        for session in &sessions {
            let _ = session.stop.send(true);
        }
        for mut session in sessions {
            if tokio::time::timeout(Duration::from_secs(3), &mut session.task)
                .await
                .is_err()
            {
                session.task.abort();
                let _ = session.task.await;
            }
        }
    }
}
impl Drop for LegacySse {
    fn drop(&mut self) {
        for (_, session) in self.registry.sessions.lock().unwrap().drain() {
            let _ = session.stop.send(true);
        }
    }
}
fn still_valid(gate: &Gate, identity: &Identity) -> bool {
    if let Some(lease) = &identity.ephemeral {
        return lease.valid();
    }
    if !gate.token.is_empty() && crate::auth::credential_eq(&gate.token, &identity.token) {
        return identity.scope == crate::auth::Scope::Operator;
    }
    gate.store
        .as_ref()
        .and_then(|path| crate::auth::Store { path: path.clone() }.lookup(&identity.token))
        .is_some_and(|record| record.scope() == Some(identity.scope))
}
struct ChannelTransport {
    incoming: mpsc::Receiver<ClientJsonRpcMessage>,
    outgoing: Option<mpsc::Sender<ServerJsonRpcMessage>>,
}
impl Transport<RoleServer> for ChannelTransport {
    type Error = io::Error;
    fn send(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let sender = self.outgoing.clone();
        async move {
            let Some(sender) = sender else {
                return Err(io::ErrorKind::BrokenPipe.into());
            };
            match tokio::time::timeout(Duration::from_secs(5), sender.send(message)).await {
                Ok(Ok(())) => Ok(()),
                _ => Err(io::ErrorKind::BrokenPipe.into()),
            }
        }
    }
    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        self.incoming.recv().await
    }
    async fn close(&mut self) -> Result<(), Self::Error> {
        self.incoming.close();
        self.outgoing.take();
        Ok(())
    }
}
async fn drive(
    adapter: Adapter,
    gate: Gate,
    identity: Identity,
    transport: ChannelTransport,
    mut stop: watch::Receiver<bool>,
) {
    let initialize = adapter.serve(transport);
    tokio::pin!(initialize);
    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    let mut recheck = tokio::time::interval(Duration::from_secs(1));
    let running = loop {
        tokio::select! {
            _=stop.changed()=>return,
            _=&mut deadline=>return,
            _=recheck.tick()=>if !still_valid(&gate,&identity){return},
            result=&mut initialize=>match result{Ok(service)=>break service,Err(_)=>return},
        }
    };
    let cancel = running.cancellation_token();
    let waiting = running.waiting();
    tokio::pin!(waiting);
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            _=recheck.tick()=>if !still_valid(&gate,&identity){break},
            _=&mut waiting=>return,
        }
    }
    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), &mut waiting).await;
}
struct Events {
    registry: Registry,
    id: String,
    endpoint: Option<String>,
    messages: mpsc::Receiver<ServerJsonRpcMessage>,
}
impl Stream for Events {
    type Item = Result<Event, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(endpoint) = self.endpoint.take() {
            return Poll::Ready(Some(Ok(Event::default().event("endpoint").data(endpoint))));
        }
        match self.messages.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(message)) => match serde_json::to_string(&message) {
                Ok(text) => Poll::Ready(Some(Ok(Event::default().event("message").data(text)))),
                Err(_) => Poll::Ready(None),
            },
        }
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        if let Some(session) = self.registry.sessions.lock().unwrap().remove(&self.id) {
            let _ = session.stop.send(true);
        }
    }
}
async fn open(State(registry): State<Registry>, request: Request) -> Response {
    let Some(identity) = request.extensions().get::<Identity>().cloned() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !still_valid(&registry.gate, &identity) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut sessions = registry.sessions.lock().unwrap();
    if sessions.len() >= MAX_SESSIONS {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("sessionid", &id);
    if !request
        .headers()
        .contains_key(axum::http::header::AUTHORIZATION)
        && request
            .uri()
            .query()
            .is_some_and(|q| url::form_urlencoded::parse(q.as_bytes()).any(|(k, _)| k == "t"))
    {
        query.append_pair("t", &identity.token);
    }
    let endpoint = format!("/sse?{}", query.finish());
    let (incoming, receiver) = mpsc::channel(QUEUE);
    let (sender, messages) = mpsc::channel(QUEUE);
    let (stop, cancel) = watch::channel(false);
    let outgoing = sender.downgrade();
    let task = tokio::spawn(drive(
        registry.adapter.clone(),
        registry.gate.clone(),
        identity.clone(),
        ChannelTransport {
            incoming: receiver,
            outgoing: Some(sender),
        },
        cancel,
    ));
    sessions.insert(
        id.clone(),
        Session {
            fingerprint: crate::auth::fingerprint(&identity.token),
            host: identity.ephemeral.is_none()
                && !identity.token.is_empty()
                && crate::auth::credential_eq(&registry.gate.token, &identity.token),
            scope: identity.scope,
            guest: identity
                .ephemeral
                .as_ref()
                .is_some_and(|lease| lease.is_guest()),
            incoming,
            outgoing,
            stop,
            task,
        },
    );
    drop(sessions);
    Sse::new(Events {
        registry,
        id: id.clone(),
        endpoint: Some(endpoint),
        messages,
    })
    .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
    .into_response()
}
async fn message(State(registry): State<Registry>, request: Request) -> Response {
    let Some(identity) = request.extensions().get::<Identity>().cloned() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !still_valid(&registry.gate, &identity) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let id = request.uri().query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == "sessionid")
            .map(|(_, v)| v.into_owned())
    });
    let Some(id) = id else {
        return (StatusCode::BAD_REQUEST, "sessionid required").into_response();
    };
    let (sender, outgoing) = {
        let sessions = registry.sessions.lock().unwrap();
        let Some(session) = sessions.get(&id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if session.fingerprint != crate::auth::fingerprint(&identity.token)
            || session.host
                != (identity.ephemeral.is_none()
                    && !identity.token.is_empty()
                    && crate::auth::credential_eq(&registry.gate.token, &identity.token))
            || session.scope != identity.scope
            || session.guest
                != identity
                    .ephemeral
                    .as_ref()
                    .is_some_and(|lease| lease.is_guest())
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        (session.incoming.clone(), session.outgoing.clone())
    };
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 64 << 20).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    if let Some((id, error)) = super::raw_preferences::validate(&bytes) {
        let Ok(id) = serde_json::from_value::<rmcp::model::RequestId>(id) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let result =
            rmcp::model::CallToolResult::error(vec![rmcp::model::ContentBlock::text(error)]);
        let response = ServerJsonRpcMessage::response(result.into(), id);
        return match outgoing.upgrade().map(|sender| sender.try_send(response)) {
            Some(Ok(())) => StatusCode::ACCEPTED.into_response(),
            Some(Err(mpsc::error::TrySendError::Full(_))) => {
                StatusCode::TOO_MANY_REQUESTS.into_response()
            }
            _ => StatusCode::GONE.into_response(),
        };
    }
    let mut message: ClientJsonRpcMessage = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid MCP message").into_response(),
    };
    match &mut message {
        ClientJsonRpcMessage::Request(request) => {
            request.request.extensions_mut().insert(parts);
        }
        ClientJsonRpcMessage::Notification(notification) => {
            notification.notification.extensions_mut().insert(parts);
        }
        _ => {}
    }
    match sender.try_send(message) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(mpsc::error::TrySendError::Full(_)) => StatusCode::TOO_MANY_REQUESTS.into_response(),
        Err(_) => StatusCode::GONE.into_response(),
    }
}

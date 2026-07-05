//! WebSocket endpoint wrappers connect to.
//!
//! Protocol: the wrapper opens `GET /wrapper/:id` (Upgrade: websocket),
//! then sends a `Register` frame, then streams `Output` chunks. The daemon
//! sends `Input`/`Signal`/`Resize` back via `WrapperHandle`.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures_util::{
    sink::SinkExt,
    stream::{SplitSink, SplitStream, StreamExt},
};
use tokio::sync::mpsc;

use crate::protocol::WrapperMessage;
use crate::session::store::WrapperHandle;
use crate::session::SessionStore;

pub async fn upgrade(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    State(store): State<SessionStore>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, id, store))
}

async fn handle(socket: WebSocket, _id_from_path: String, store: SessionStore) {
    let (mut sink, mut stream): (SplitSink<WebSocket, Message>, SplitStream<WebSocket>) =
        socket.split();

    // First frame must be Register.
    let register = match stream.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<WrapperMessage>(&text) {
            Ok(WrapperMessage::Register {
                session_id,
                cwd,
                cols,
                rows,
                ..
            }) => (session_id, cwd, cols, rows),
            Ok(other) => {
                tracing::warn!(?other, "wrapper opened WS without Register first");
                return;
            }
            Err(err) => {
                tracing::warn!(?err, "decode wrapper register");
                return;
            }
        },
        _ => return,
    };
    let (session_id, cwd, cols, rows) = register;

    let (tx, mut rx) = mpsc::unbounded_channel::<WrapperMessage>();
    store.register_wrapper(&session_id, &cwd, WrapperHandle { tx });
    store.note_term_size(&session_id, cols, rows);
    tracing::info!(%session_id, %cwd, "wrapper registered");

    // daemon → wrapper pump
    let outbound = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = match serde_json::to_string(&msg) {
                Ok(t) => t,
                Err(err) => {
                    tracing::warn!(?err, "encode daemon→wrapper msg");
                    continue;
                }
            };
            if sink.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    // wrapper → daemon pump
    let store_for_inbound = store.clone();
    let session_for_inbound = session_id.clone();
    while let Some(frame) = stream.next().await {
        let Ok(Message::Text(text)) = frame else {
            continue;
        };
        let msg: WrapperMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!(?err, "decode wrapper msg");
                continue;
            }
        };
        match msg {
            WrapperMessage::Output { bytes } => {
                if let Ok(decoded) = B64.decode(bytes.as_bytes()) {
                    store_for_inbound
                        .record_output(&session_for_inbound, &decoded)
                        .await;
                }
            }
            WrapperMessage::Exited { code } => {
                tracing::info!(session = %session_for_inbound, ?code, "wrapper exited");
                break;
            }
            _ => {} // daemon→wrapper variants ignored on this direction
        }
    }

    outbound.abort();
    store.deregister_wrapper(&session_id);
}

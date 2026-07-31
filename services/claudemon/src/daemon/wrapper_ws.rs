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
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
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
    headers: HeaderMap,
    State(store): State<SessionStore>,
) -> Response {
    // CORS never applies to a WebSocket upgrade, so the Origin policy has to be
    // enforced by hand here — otherwise any page in any browser on this machine
    // can open ws://127.0.0.1:7891/wrapper/<id> and drive a session's PTY.
    if !origin_allowed(&headers) {
        tracing::warn!(
            origin = ?headers.get(header::ORIGIN),
            "refused cross-site wrapper websocket"
        );
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    ws.on_upgrade(move |socket| handle(socket, id, store))
        .into_response()
}

/// Same-origin policy for `/wrapper/:id`, a port of the hub bus's
/// `originAllowed` (services/hub/internal/bus/bus.go) so the two WebSocket
/// ingresses agree:
///
///   - No `Origin` → allow. Wrappers are native clients (`claudemon wrap`,
///     tokio-tungstenite) and send none; only a browser's same-origin policy is
///     being enforced here.
///   - Loopback origin, any port → allow. A local dev renderer served on another
///     localhost port is legitimate; an attacker's page is never served from the
///     victim's own loopback.
///   - Origin host == the `Host` the client dialed → allow (same origin).
///   - Anything else is a cross-site browser origin: refused before the upgrade,
///     so no socket, no Register, no work.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    // Malformed or opaque ("null", sandboxed iframes) origins fail closed.
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return false;
    }
    if host_is_loopback(host_without_port(authority)) {
        return true;
    }
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|host| host.eq_ignore_ascii_case(authority))
}

/// Strip a trailing `:port` from an authority, handling bracketed IPv6
/// (`[::1]:7891` → `::1`). Mirrors the API router's helper of the same name.
fn host_without_port(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(authority);
    }
    authority
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(authority)
}

fn host_is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

async fn handle(socket: WebSocket, id_from_path: String, store: SessionStore) {
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

    // The id in the URL is part of the request the caller is authorized for;
    // a Register naming a *different* session would let one connection claim a
    // session it never dialed. The real wrapper always dials `/wrapper/<its own
    // id>` and repeats it in the frame, so a mismatch is never legitimate.
    if session_id != id_from_path {
        tracing::warn!(
            %session_id,
            path_id = %id_from_path,
            "wrapper Register session_id does not match the dialed path; closing"
        );
        return;
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<WrapperMessage>();
    if store
        .register_wrapper(&session_id, &cwd, WrapperHandle { tx })
        .is_none()
    {
        // First-registration-wins: the incumbent keeps the session. Bail out
        // *before* the pumps start so nothing this connection sends is recorded
        // against a session it doesn't own — and, critically, without running
        // the deregister at the bottom of this function, which would tear the
        // live wrapper's buffers down.
        tracing::warn!(%session_id, "refused wrapper register: session already has a live wrapper");
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    #[test]
    fn origin_policy_matches_the_bus() {
        let with = |origin: &str, host: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::ORIGIN, origin.parse().unwrap());
            h.insert(header::HOST, host.parse().unwrap());
            origin_allowed(&h)
        };
        assert!(
            origin_allowed(&HeaderMap::new()),
            "no Origin → native client"
        );
        assert!(with("http://127.0.0.1:5173", "127.0.0.1:7891"));
        assert!(with("http://localhost:5173", "127.0.0.1:7891"));
        assert!(with("https://[::1]:5173", "127.0.0.1:7891"));
        // Same-origin over a shared (tailscale/LAN) host the client dialed.
        assert!(with("http://box.tail:7891", "box.tail:7891"));
        assert!(!with("http://evil.example.com", "127.0.0.1:7891"));
        assert!(!with("null", "127.0.0.1:7891"));
        assert!(!with("http://box.tail.evil:7891", "box.tail:7891"));
    }

    /// Serve the wrapper route on an ephemeral loopback port and hand back its
    /// URL plus the store the handler registers into.
    async fn serve() -> (String, SessionStore) {
        let store = SessionStore::new();
        let app = Router::new()
            .route("/wrapper/:id", get(upgrade))
            .with_state(store.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("ws://{addr}/wrapper"), store)
    }

    /// Dial the wrapper endpoint with an `Origin`, the way a browser page would.
    /// Returns the handshake's HTTP status when the daemon refused it.
    async fn connect_with_origin(base: &str, origin: &str) -> Result<(), StatusCode> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut req = format!("{base}/s1").into_client_request().unwrap();
        req.headers_mut()
            .insert(header::ORIGIN, origin.parse().unwrap());
        match tokio_tungstenite::connect_async(req).await {
            Ok(_) => Ok(()),
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Err(resp.status()),
            Err(err) => panic!("unexpected handshake failure: {err:?}"),
        }
    }

    #[tokio::test]
    async fn cross_site_origin_is_refused_before_the_upgrade() {
        // The drive-by case: a page the user is merely visiting opens
        // ws://127.0.0.1:7891/wrapper/<id>. No socket may result.
        let (base, _store) = serve().await;
        assert_eq!(
            connect_with_origin(&base, "http://evil.example.com").await,
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[tokio::test]
    async fn loopback_origin_still_upgrades() {
        // A dev renderer served from another localhost port is legitimate.
        let (base, _store) = serve().await;
        assert_eq!(
            connect_with_origin(&base, "http://localhost:5173").await,
            Ok(())
        );
    }

    fn register_frame(session_id: &str) -> WsMessage {
        WsMessage::Text(
            serde_json::to_string(&WrapperMessage::Register {
                session_id: session_id.to_string(),
                cwd: "/w".to_string(),
                argv: vec!["claude".to_string()],
                cols: 80,
                rows: 24,
            })
            .unwrap(),
        )
    }

    /// Poll until the store has (or drops) a wrapper for `id`, or give up.
    async fn wait_for_wrapper(store: &SessionStore, id: &str, want: bool) -> bool {
        for _ in 0..200 {
            if store.wrapper(id).is_some() == want {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        false
    }

    #[tokio::test]
    async fn second_register_cannot_displace_a_live_wrapper() {
        let (base, store) = serve().await;
        let (mut first, _) = tokio_tungstenite::connect_async(format!("{base}/s1"))
            .await
            .expect("first wrapper connects");
        first.send(register_frame("s1")).await.unwrap();
        assert!(
            wait_for_wrapper(&store, "s1", true).await,
            "first registered"
        );

        let (mut second, _) = tokio_tungstenite::connect_async(format!("{base}/s1"))
            .await
            .expect("second wrapper connects");
        second.send(register_frame("s1")).await.unwrap();
        // The hijacker's socket is closed without ever being wired up.
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(frame) = second.next().await {
                if matches!(frame, Ok(WsMessage::Close(_)) | Err(_)) {
                    break;
                }
            }
        })
        .await;
        assert!(closed.is_ok(), "the refused connection must be dropped");

        // The incumbent still owns the session: input routed through the store
        // lands on the FIRST socket.
        let handle = store.wrapper("s1").expect("session still has a wrapper");
        handle
            .tx
            .send(WrapperMessage::Input {
                bytes: B64.encode("hi"),
            })
            .expect("incumbent channel is alive");
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), first.next())
            .await
            .expect("first wrapper receives")
            .expect("frame")
            .expect("ok frame");
        let WsMessage::Text(text) = got else {
            panic!("expected a text frame, got {got:?}");
        };
        assert!(text.contains(&B64.encode("hi")), "got: {text}");
    }

    #[tokio::test]
    async fn register_for_a_different_session_than_the_path_is_refused() {
        let (base, store) = serve().await;
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("{base}/s1"))
            .await
            .expect("connects");
        ws.send(register_frame("s2")).await.unwrap();
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(frame) = ws.next().await {
                if matches!(frame, Ok(WsMessage::Close(_)) | Err(_)) {
                    break;
                }
            }
        })
        .await;
        assert!(closed.is_ok(), "mismatched Register must close the socket");
        assert!(store.wrapper("s1").is_none(), "path id not registered");
        assert!(store.wrapper("s2").is_none(), "claimed id not registered");
    }
}

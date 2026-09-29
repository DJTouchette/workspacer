//! Typed caller with in-process and optional remote transports. Calls are never replayed.
use crate::{
    Connection, Handle,
    protocol::{Event, Frame},
};
use anyhow::{Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};

pub const MACHINE_STOP_CLOSE_CODE: u16 = 4001;
/// A received machine-stop close is a pause, not an invitation to retry a dial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisconnectReason {
    RemoteClose { code: Option<u16>, reason: String },
    TransportLost,
    InvalidFrame,
    LocalClosed,
}
impl DisconnectReason {
    pub fn is_power_paused(&self) -> bool {
        matches!(
            self,
            Self::RemoteClose {
                code: Some(MACHINE_STOP_CLOSE_CODE),
                ..
            }
        )
    }
}
impl std::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RemoteClose { code, reason } => {
                write!(f, "peer closed connection ({code:?}): {reason}")
            }
            Self::TransportLost => f.write_str("peer transport lost"),
            Self::InvalidFrame => f.write_str("peer sent an invalid protocol frame"),
            Self::LocalClosed => f.write_str("local caller closed connection"),
        }
    }
}
impl std::error::Error for DisconnectReason {}

enum Transport {
    Embedded(Connection),
    Socket(Box<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>),
}
impl Transport {
    async fn send(&mut self, frame: Frame) -> Result<()> {
        match self {
            Self::Embedded(connection) => connection.send(frame),
            Self::Socket(socket) => tokio::time::timeout(
                Duration::from_secs(5),
                socket.send(Message::Text(serde_json::to_string(&frame)?)),
            )
            .await
            .map_err(|_| anyhow!("peer write timed out; outcome is unknown"))?
            .map_err(|_| anyhow!("peer connection lost; outcome is unknown")),
        }
    }
    fn failure_reason(&self) -> DisconnectReason {
        match self {
            Self::Embedded(connection) => match connection.close_code() {
                Some(code) => DisconnectReason::RemoteClose {
                    code: Some(code),
                    reason: if code == MACHINE_STOP_CLOSE_CODE {
                        "machine stopping; reconnect only to wake".into()
                    } else {
                        String::new()
                    },
                },
                None => DisconnectReason::TransportLost,
            },
            Self::Socket(_) => DisconnectReason::TransportLost,
        }
    }
    async fn recv(&mut self) -> std::result::Result<Frame, DisconnectReason> {
        match self {
            Self::Embedded(connection) => match connection.recv().await {
                Some(frame) => Ok(frame),
                None => Err(self.failure_reason()),
            },
            Self::Socket(socket) => loop {
                match socket.next().await {
                    Some(Ok(Message::Text(text))) => {
                        return serde_json::from_str(&text)
                            .map_err(|_| DisconnectReason::InvalidFrame);
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        return serde_json::from_slice(&bytes)
                            .map_err(|_| DisconnectReason::InvalidFrame);
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let reason = DisconnectReason::RemoteClose {
                            code: frame.as_ref().map(|f| u16::from(f.code)),
                            reason: frame.map(|f| f.reason.into_owned()).unwrap_or_default(),
                        };
                        // Preserve the close reason even if the final acknowledgement
                        // cannot flush; importantly, never convert4001 to generic loss.
                        let _ = tokio::time::timeout(Duration::from_secs(1), socket.flush()).await;
                        return Err(reason);
                    }
                    Some(Ok(Message::Ping(_))) => {
                        if !matches!(
                            tokio::time::timeout(Duration::from_secs(5), socket.flush()).await,
                            Ok(Ok(()))
                        ) {
                            return Err(DisconnectReason::TransportLost);
                        }
                    }
                    Some(Ok(Message::Pong(_))) => (),
                    _ => return Err(DisconnectReason::TransportLost),
                }
            },
        }
    }
}

#[derive(Clone)]
pub struct Client {
    tx: mpsc::Sender<Request>,
    events: broadcast::Sender<Event>,
    stop: watch::Sender<bool>,
    disconnected: watch::Receiver<Option<DisconnectReason>>,
}
enum Request {
    Call {
        method: String,
        params: Value,
        reply: oneshot::Sender<Result<Value>>,
    },
    Topics {
        topics: BTreeSet<String>,
        reply: oneshot::Sender<Result<()>>,
    },
    Publish {
        event: Event,
        reply: oneshot::Sender<Result<()>>,
    },
}
impl Client {
    pub(crate) async fn connect_service(hub: &Handle) -> Result<Self> {
        Ok(Self::from_connection(hub.connect_service().await?))
    }

    pub async fn connect(hub: &Handle) -> Result<Self> {
        let connection = hub.connect().await?;
        Ok(Self::from_connection(connection))
    }
    pub fn from_connection(connection: Connection) -> Self {
        Self::from_transport(Transport::Embedded(connection), true)
    }
    fn from_transport(connection: Transport, cancel_calls: bool) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let (events, _) = broadcast::channel(256);
        let (stop, mut stopped) = watch::channel(false);
        let (finished, disconnected) = watch::channel(None);
        let sink = events.clone();
        tokio::spawn(async move {
            let reason = tokio::select! {
                reason = run(connection, rx, sink, cancel_calls) => reason,
                _ = stopped.changed() => DisconnectReason::LocalClosed,
            };
            finished.send_replace(Some(reason));
        });
        Self {
            tx,
            events,
            stop,
            disconnected,
        }
    }
    /// Close this connection and every clone, including in-flight calls.
    pub fn close(&self) {
        self.stop.send_replace(true);
    }
    /// Establish one authenticated remote connection. The owner decides when
    /// to reconnect and reassert subscriptions; pending mutations are not retried.
    pub async fn connect_remote(address: &str, token: &str) -> Result<Self> {
        Self::connect_remote_inner(address, token, false)
            .await
            .map(|(client, _)| client)
    }
    /// Negotiate a server-stamped identity without changing the legacy hello
    /// contract. Old peers provide None; authority must never be inferred from
    /// scope alone when this proof is absent.
    pub async fn connect_remote_with_identity(
        address: &str,
        token: &str,
    ) -> Result<(Self, Option<crate::protocol::ProviderCaller>)> {
        let mut address = url::Url::parse(address).map_err(|_| anyhow!("invalid peer URL"))?;
        let pairs: Vec<_> = address
            .query_pairs()
            .filter(|(key, _)| key != "identity")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        address.set_query(None);
        address
            .query_pairs_mut()
            .extend_pairs(pairs)
            .append_pair("identity", "1");
        let (client, hello) = Self::connect_remote_inner(address.as_str(), token, true).await?;
        let identity = if hello.caller_context_version == 1 {
            hello
                .provider_caller
                .filter(|identity| identity.version == 1 && identity.scope == hello.scope)
        } else {
            None
        };
        Ok((client, identity))
    }
    async fn connect_remote_inner(
        address: &str,
        token: &str,
        negotiated: bool,
    ) -> Result<(Self, Frame)> {
        let mut request = address
            .into_client_request()
            .map_err(|_| anyhow!("invalid peer URL"))?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| anyhow!("invalid peer credential"))?,
        );
        let limits = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
            max_message_size: Some(64 * 1024 * 1024),
            max_frame_size: Some(64 * 1024 * 1024),
            ..Default::default()
        };
        let socket = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async_with_config(request, Some(limits), true),
        )
        .await
        .map_err(|_| anyhow!("peer handshake timed out"))?
        .map_err(|_| anyhow!("peer handshake failed"))?
        .0;
        let mut transport = Transport::Socket(Box::new(socket));
        let hello = tokio::time::timeout(Duration::from_secs(5), transport.recv())
            .await
            .map_err(|_| anyhow!("peer hello timed out"))??;
        if hello.op != "hello" {
            bail!("peer did not send hello");
        }
        let cancel_calls = negotiated && hello.caller_context_version == 1;
        Ok((Self::from_transport(transport, cancel_calls), hello))
    }
    pub fn disconnect_reason(&self) -> Option<DisconnectReason> {
        self.disconnected.borrow().clone()
    }
    pub async fn disconnected_reason(&self) -> DisconnectReason {
        let mut state = self.disconnected.clone();
        loop {
            if let Some(reason) = state.borrow().clone() {
                return reason;
            }
            if state.changed().await.is_err() {
                return DisconnectReason::TransportLost;
            }
        }
    }
    pub async fn disconnected(&self) {
        let _ = self.disconnected_reason().await;
    }
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_with_timeout(method, params, Duration::from_secs(30))
            .await
    }
    /// Dropping/timing out a waiter requests per-call cancellation on embedded
    /// or explicitly negotiated transports. Accepted effects may still finish;
    /// absence of a reply never establishes that a mutation did not execute.
    pub async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let (reply, result) = oneshot::channel();
        self.tx
            .try_send(Request::Call {
                method: method.into(),
                params,
                reply,
            })
            .map_err(|_| anyhow!("hub caller unavailable or full; call was not submitted"))?;
        tokio::time::timeout(timeout, result)
            .await
            .map_err(|_| {
                anyhow!("hub call timed out; outcome is unknown, do not automatically retry")
            })?
            .map_err(|_| anyhow!("hub connection lost; outcome is unknown"))?
    }
    /// Submit an event once. Success confirms transport submission only, not
    /// broker authorization, UI delivery or an effect acknowledgement.
    pub async fn publish(&self, event: Event) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.tx
            .try_send(Request::Publish { event, reply })
            .map_err(|_| anyhow!("hub caller unavailable or full; event was not submitted"))?;
        tokio::time::timeout(Duration::from_secs(10), result)
            .await
            .map_err(|_| {
                anyhow!("event submission timed out; outcome unknown, do not replay automatically")
            })?
            .map_err(|_| anyhow!("hub connection lost; event submission outcome unknown"))?
    }
    pub async fn topics(&self, topics: BTreeSet<String>) -> Result<()> {
        if topics.len() > 256 {
            bail!("too many topic patterns");
        }
        let (reply, result) = oneshot::channel();
        self.tx
            .try_send(Request::Topics { topics, reply })
            .map_err(|_| {
                anyhow!("hub caller unavailable or full; subscription was not submitted")
            })?;
        result.await.map_err(|_| anyhow!("hub connection lost"))?
    }
}
async fn run(
    mut connection: Transport,
    mut commands: mpsc::Receiver<Request>,
    events: broadcast::Sender<Event>,
    cancel_calls: bool,
) -> DisconnectReason {
    let mut pending = HashMap::<String, oneshot::Sender<Result<Value>>>::new();
    let mut sequence = 0u64;
    let mut topics = BTreeSet::new();
    let mut sweep = tokio::time::interval(Duration::from_millis(25));
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let reason = 'connection: loop {
        let abandoned: Vec<_> = pending
            .iter()
            .filter(|(_, reply)| reply.is_closed())
            .map(|(id, _)| id.clone())
            .collect();
        for id in abandoned {
            pending.remove(&id);
            if cancel_calls
                && connection
                    .send(Frame {
                        id,
                        caller_context_version: 1,
                        ..Frame::op("cancel")
                    })
                    .await
                    .is_err()
            {
                break 'connection connection.failure_reason();
            }
        }
        tokio::select! {
            biased;
            _=sweep.tick()=>(),
            frame=connection.recv()=>match frame {
                Ok(frame) if frame.op=="event"=>{if let Some(event)=frame.event{let _=events.send(event);}},
                Ok(frame) if frame.op=="result" || frame.op=="error"=>{
                    if let Some(reply)=pending.remove(&frame.id){let _=reply.send(if frame.op=="error" {Err(anyhow!(frame.error))} else {Ok(frame.result.unwrap_or(Value::Null))});}
                }
                Ok(_)=>(),Err(reason)=>break reason,
            },
            request=commands.recv()=>match request {
                Some(Request::Call {method,params,reply})=>{
                    if reply.is_closed(){continue;}
                    if pending.len()>=256 {let _=reply.send(Err(anyhow!("too many pending calls; call was not submitted")));continue;}
                    sequence+=1;let id=sequence.to_string();
                    match connection.send(Frame {id:id.clone(),method,params:Some(params),..Frame::op("call")}).await{
                        Ok(())=>{pending.insert(id,reply);},Err(error)=>{let _=reply.send(Err(error));break connection.failure_reason();}
                    }
                }
                Some(Request::Publish{event,reply})=>{
                    if reply.is_closed(){continue;}
                    let result=connection.send(Frame{event:Some(event),..Frame::op("publish")}).await;
                    let failed=result.is_err();let _=reply.send(result);
                    if failed {break connection.failure_reason();}
                },
                Some(Request::Topics {topics:new,reply})=>{
                    let remove:Vec<_>=topics.difference(&new).cloned().collect();let add:Vec<_>=new.difference(&topics).cloned().collect();
                    let result=async {
                        if !remove.is_empty(){connection.send(Frame {topics:remove,..Frame::op("unsubscribe")}).await?;}
                        if !add.is_empty(){connection.send(Frame {topics:add,..Frame::op("subscribe")}).await?;}
                        anyhow::Ok(())
                    }.await;
                    if result.is_ok(){topics=new;}
                    let failed=result.is_err();
                    let _=reply.send(result);
                    if failed { break connection.failure_reason(); }
                }
                None=>break DisconnectReason::LocalClosed,
            },
        }
    };
    for (_, reply) in pending {
        let _ = reply.send(Err(anyhow::Error::new(reason.clone()).context(
            "hub connection lost; outcome is unknown, do not automatically retry",
        )));
    }
    reason
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::{Hub, Options};
    #[tokio::test]
    async fn dropped_call_cancels_only_that_waiter_and_preserves_sibling_and_connection() {
        let hub = Hub::start(Options::default()).unwrap();
        hub.ready().await.unwrap();
        let mut provider = hub.handle().connect().await.unwrap();
        provider.recv().await.unwrap();
        provider
            .send(Frame {
                methods: vec!["fixture.wait".into()],
                wants_caller_context: true,
                ..Frame::op("register")
            })
            .unwrap();
        provider.recv().await.unwrap();
        let client = Client::connect(&hub.handle()).await.unwrap();
        let first = client.clone();
        let first = tokio::spawn(async move { first.call("fixture.wait", Value::Null).await });
        let a = provider.recv().await.unwrap();
        assert_eq!(a.op, "call");
        let second = client.clone();
        let second = tokio::spawn(async move { second.call("fixture.wait", Value::Null).await });
        let b = provider.recv().await.unwrap();
        assert_eq!(b.op, "call");
        first.abort();
        let _ = first.await;
        let cancel = tokio::time::timeout(Duration::from_secs(2), provider.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cancel.op, "cancel");
        assert_eq!(cancel.id, a.id);
        assert_eq!(cancel.caller_context_version, 1);
        provider
            .send(Frame {
                id: a.id,
                result: Some(Value::String("late abandoned result".into())),
                ..Frame::op("result")
            })
            .unwrap();
        provider
            .send(Frame {
                id: b.id,
                result: Some(Value::String("sibling".into())),
                ..Frame::op("result")
            })
            .unwrap();
        assert_eq!(second.await.unwrap().unwrap(), "sibling");
        assert!(client.disconnect_reason().is_none());
        client.close();
        hub.shutdown().unwrap();
    }
    #[tokio::test]
    async fn remote_timeout_emits_cancel_only_after_explicit_negotiation() {
        for negotiated in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                socket
                    .send(Message::Text(
                        serde_json::to_string(&Frame {
                            scope: "operator".into(),
                            caller_context_version: if negotiated { 1 } else { 0 },
                            ..Frame::op("hello")
                        })
                        .unwrap(),
                    ))
                    .await
                    .unwrap();
                let request: Frame =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(request.op, "call");
                let next = tokio::time::timeout(Duration::from_millis(150), socket.next()).await;
                if negotiated {
                    let cancel: Frame =
                        serde_json::from_str(next.unwrap().unwrap().unwrap().to_text().unwrap())
                            .unwrap();
                    assert_eq!(cancel.op, "cancel");
                    assert_eq!(cancel.id, request.id);
                } else {
                    assert!(
                        next.is_err(),
                        "legacy peer received an additive protocol frame"
                    );
                }
            });
            let address = format!("ws://{address}");
            let client = if negotiated {
                Client::connect_remote_with_identity(&address, "fixture")
                    .await
                    .unwrap()
                    .0
            } else {
                Client::connect_remote(&address, "fixture").await.unwrap()
            };
            assert!(
                client
                    .call_with_timeout("fixture.wait", Value::Null, Duration::from_millis(30))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("outcome is unknown")
            );
            server.await.unwrap();
            client.close();
        }
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;
    use crate::{Hub, Options};
    #[tokio::test]
    async fn received_stop_reason_survives_remote_and_embedded_disconnect() {
        let mut options = Options::default();
        options.listen = Some("127.0.0.1:0".parse().unwrap());
        options.token = "fixture".into();
        let hub = Hub::start(options).unwrap();
        let address = hub.ready().await.unwrap().unwrap();
        let embedded = Client::connect(&hub.handle()).await.unwrap();
        let remote = Client::connect_remote(&format!("ws://{address}/bus"), "fixture")
            .await
            .unwrap();
        hub.handle().disconnect_for_machine_stop().await.unwrap();
        for client in [embedded, remote] {
            let reason = tokio::time::timeout(Duration::from_secs(2), client.disconnected_reason())
                .await
                .unwrap();
            assert!(reason.is_power_paused());
            assert_eq!(client.disconnect_reason(), Some(reason));
            client.close();
            assert!(client.disconnect_reason().unwrap().is_power_paused());
        }
        hub.shutdown().unwrap();
    }
    #[tokio::test]
    async fn power_pause_before_hello_is_a_typed_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket
                .send(Message::Close(Some(
                    tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: MACHINE_STOP_CLOSE_CODE.into(),
                        reason: "machine stopping".into(),
                    },
                )))
                .await
                .unwrap();
            let _ = socket.next().await;
        });
        let error = match Client::connect_remote(&format!("ws://{address}"), "fixture").await {
            Ok(_) => panic!("unexpected ready connection"),
            Err(error) => error,
        };
        assert!(
            error
                .downcast_ref::<DisconnectReason>()
                .unwrap()
                .is_power_paused()
        );
        server.await.unwrap();
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::{
        Hub, Options,
        auth::{self, Scope},
    };
    #[tokio::test]
    async fn negotiated_hello_distinguishes_host_operator_and_delegated_facade() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tokens.json");
        let ordinary = auth::mint(&path, Scope::Operator, "ordinary").unwrap();
        let delegated = auth::mint(&path, Scope::Operator, "delegated").unwrap();
        auth::update_records(&path, |records| {
            records
                .iter_mut()
                .find(|r| r.token == delegated.token)
                .unwrap()
                .facade_authority = true;
            Ok(())
        })
        .unwrap();
        let mut options = Options::default();
        options.listen = Some("127.0.0.1:0".parse().unwrap());
        options.token = "root-owner".into();
        options.scoped_tokens = Some(path);
        let hub = Hub::start(options).unwrap();
        let address = hub.ready().await.unwrap().unwrap();
        let url = format!("ws://{address}/bus?identity=0&identity=0");
        for (token, host, assertion) in [
            ("root-owner", true, true),
            (ordinary.token.as_str(), false, false),
            (delegated.token.as_str(), false, true),
        ] {
            let (client, proof) = Client::connect_remote_with_identity(&url, token)
                .await
                .unwrap();
            let proof = proof.expect("negotiated identity");
            assert_eq!(proof.scope, "operator");
            assert_eq!(proof.authenticated_host, host);
            assert_eq!(proof.may_assert_session, assertion);
            assert!(
                !proof.federated,
                "identity negotiation must not add peer provenance"
            );
            assert_eq!(proof.version, 1);
            client.close();
        }
        hub.shutdown().unwrap();
    }
    #[tokio::test]
    async fn old_peer_without_negotiated_identity_remains_none() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket=tokio_tungstenite::accept_hdr_async(stream,|request:&tokio_tungstenite::tungstenite::handshake::server::Request,response:tokio_tungstenite::tungstenite::handshake::server::Response|{let query=request.uri().query().unwrap();let pairs:Vec<_>=url::form_urlencoded::parse(query.as_bytes()).collect();assert_eq!(pairs.iter().filter(|(key,_)|key=="identity").count(),1);assert!(pairs.iter().any(|(key,value)|key=="identity"&&value=="1"));assert!(!pairs.iter().any(|(key,_)|key=="peer"));Ok(response)}).await.unwrap();
            socket
                .send(Message::Text(
                    serde_json::json!({"op":"hello","scope":"operator","methods":[]}).to_string(),
                ))
                .await
                .unwrap();
            let _ = socket.next().await;
        });
        let (client, identity) = Client::connect_remote_with_identity(
            &format!("ws://{address}/bus?identity=0"),
            "fixture",
        )
        .await
        .unwrap();
        assert!(identity.is_none());
        client.close();
        server.await.unwrap();
    }
}

#[cfg(test)]
mod publish_tests {
    use super::*;
    use crate::{Hub, Options};
    #[tokio::test]
    async fn remote_event_submission_is_once_and_closed_transport_does_not_queue_replay() {
        let mut options =
            Options::default().handler("fixture.barrier", |_, _| async { Ok(Value::Null) });
        options.listen = Some("127.0.0.1:0".parse().unwrap());
        options.token = "owner".into();
        let hub = Hub::start(options).unwrap();
        let address = hub.ready().await.unwrap().unwrap();
        let watch = Client::connect_remote(&format!("ws://{address}/bus"), "owner")
            .await
            .unwrap();
        let mut events = watch.events();
        watch.topics(["agent.updated".into()].into()).await.unwrap();
        watch.call("fixture.barrier", Value::Null).await.unwrap();
        let publisher = Client::connect_remote(&format!("ws://{address}/bus"), "owner")
            .await
            .unwrap();
        let mut event = Event::new("agent.updated", "fixture", serde_json::json!({"once":true}));
        event.hub = "forged".into();
        publisher.publish(event).await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.data, Some(serde_json::json!({"once":true})));
        assert!(event.hub.is_empty());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err()
        );
        hub.handle().disconnect_for_machine_stop().await.unwrap();
        publisher.disconnected().await;
        assert!(
            publisher
                .publish(Event::new("agent.updated", "fixture", Value::Null))
                .await
                .unwrap_err()
                .to_string()
                .contains("not submitted")
        );
        hub.shutdown().unwrap();
    }
}

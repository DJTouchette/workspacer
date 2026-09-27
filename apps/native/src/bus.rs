//! One socket owner, bounded queues, expiring calls, and no mutation replay.
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, timeout},
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};

pub const CALL_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct Config {
    pub url: String,
    pub token: Option<String>,
    pub call_timeout: Duration,
}

impl Config {
    pub fn new(url: String, token: Option<String>) -> Result<Self> {
        let parsed = url::Url::parse(&url).map_err(|_| anyhow!("Invalid hub URL"))?;
        if !matches!(parsed.scheme(), "ws" | "wss") || parsed.host_str().is_none() {
            bail!("Hub URL must use ws:// or wss://");
        }
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            bail!(
                "Use HUB_TOKEN or --token-file for credentials; hub URL must have no credentials, query or fragment"
            );
        }
        if let Some(token) = &token {
            tokio_tungstenite::tungstenite::http::HeaderValue::from_bytes(
                format!("Bearer {token}").as_bytes(),
            )
            .map_err(|_| anyhow!("Hub token contains invalid header characters"))?;
        }
        Ok(Self {
            url,
            token,
            call_timeout: CALL_TIMEOUT,
        })
    }
}

#[derive(Debug)]
pub enum Event {
    Connected,
    Disconnected(String),
    Data {
        topic: String,
        data: Value,
        hub: Option<String>,
    },
}

enum Command {
    Call {
        method: String,
        params: Value,
        reply: oneshot::Sender<Result<Value>>,
        expires: Instant,
    },
    Topics(BTreeSet<String>),
}

#[derive(Clone)]
pub struct Client {
    commands: mpsc::Sender<Command>,
    connected: Arc<AtomicBool>,
    call_timeout: Duration,
}

impl Client {
    /// Must be called within a Tokio runtime. Dropping all handles stops the
    /// actor even while dialing or waiting to reconnect.
    pub fn start(config: Config) -> (Self, async_channel::Receiver<Event>) {
        let (commands, rx) = mpsc::channel(64);
        let (events, incoming) = async_channel::bounded(256);
        let connected = Arc::new(AtomicBool::new(false));
        let client = Self {
            commands,
            connected: connected.clone(),
            call_timeout: config.call_timeout,
        };
        tokio::spawn(run(config, rx, events, connected));
        (client, incoming)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        if !self.connected.load(Ordering::Acquire) {
            bail!("Hub disconnected; request was not sent");
        }
        let (reply, result) = oneshot::channel();
        self.commands
            .try_send(Command::Call {
                method: method.into(),
                params,
                reply,
                expires: Instant::now() + self.call_timeout,
            })
            .map_err(|_| anyhow!("Hub busy; request was not sent"))?;
        timeout(self.call_timeout, result)
            .await
            .map_err(|_| {
                anyhow!("Request timed out; outcome unknown. Check the session before retrying")
            })?
            .map_err(|_| anyhow!("Hub disconnected; request outcome unknown"))?
    }

    pub async fn topics(&self, topics: BTreeSet<String>) -> Result<()> {
        self.commands
            .send(Command::Topics(topics))
            .await
            .map_err(|_| anyhow!("Hub closed"))
    }
}

type Pending = HashMap<String, (Instant, oneshot::Sender<Result<Value>>)>;

fn offline_command(cmd: Option<Command>, topics: &mut BTreeSet<String>) -> bool {
    match cmd {
        None => false,
        Some(Command::Topics(new)) => {
            *topics = new;
            true
        }
        Some(Command::Call { reply, .. }) => {
            let _ = reply.send(Err(anyhow!("Hub disconnected; request was not sent")));
            true
        }
    }
}

async fn run(
    config: Config,
    mut commands: mpsc::Receiver<Command>,
    events: async_channel::Sender<Event>,
    connected: Arc<AtomicBool>,
) {
    let mut topics = BTreeSet::new();
    let mut counter = 0u64;
    let mut backoff = Duration::from_millis(250);
    loop {
        let mut request = match config.url.as_str().into_client_request() {
            Ok(r) => r,
            Err(_) => return,
        };
        if let Some(token) = &config.token {
            let Ok(header) = format!("Bearer {token}").parse() else {
                return;
            };
            request.headers_mut().insert("Authorization", header);
        }
        let limits = WebSocketConfig {
            max_message_size: Some(MAX_FRAME_BYTES),
            max_frame_size: Some(MAX_FRAME_BYTES),
            ..Default::default()
        };
        let dial = timeout(
            Duration::from_secs(10),
            connect_async_with_config(request, Some(limits), true),
        );
        tokio::pin!(dial);
        let result = loop {
            tokio::select! {
                result = &mut dial => break result,
                cmd = commands.recv() => if !offline_command(cmd, &mut topics) { return; },
                _ = events.closed() => return,
            }
        };
        let (connection, reason) = match result {
            Ok(Ok(connection)) => (
                Some(connection),
                "Connection closed; displayed state may be stale".to_owned(),
            ),
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => (
                None,
                format!(
                    "Hub rejected the connection (HTTP {})",
                    response.status().as_u16()
                ),
            ),
            Ok(Err(tokio_tungstenite::tungstenite::Error::Tls(_))) => {
                (None, "TLS handshake failed".to_owned())
            }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Io(error))) => (
                None,
                format!("Network connection failed ({})", error.kind()),
            ),
            Ok(Err(_)) => (None, "WebSocket handshake failed".to_owned()),
            Err(_) => (None, "Connection attempt timed out".to_owned()),
        };
        if let Some((mut socket, _)) = connection {
            let mut pending = Pending::new();
            let mut sent_topics = BTreeSet::new();
            let mut ready = false;
            let mut last_received = Instant::now();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
            heartbeat.tick().await;
            loop {
                let deadline = pending
                    .values()
                    .map(|(at, _)| *at)
                    .min()
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
                tokio::select! {
                    _ = events.closed() => return,
                    _ = tokio::time::sleep_until(deadline), if !pending.is_empty() => {
                        pending.retain(|_, (at, reply)| *at > Instant::now() && !reply.is_closed());
                    }
                    _ = heartbeat.tick() => {
                        if last_received.elapsed() > Duration::from_secs(60) { break; }
                        if socket.send(Message::Ping(Vec::new())).await.is_err() { break; }
                    }
                    cmd = commands.recv() => match cmd {
                        None => return,
                        Some(Command::Topics(new)) => {
                            topics = new;
                            if ready {
                                let removed: Vec<_> = sent_topics.difference(&topics).cloned().collect();
                                let added: Vec<_> = topics.difference(&sent_topics).cloned().collect();
                                if !removed.is_empty() && socket.send(Message::Text(json!({"op":"unsubscribe", "topics":removed}).to_string())).await.is_err() { break; }
                                if !added.is_empty() && socket.send(Message::Text(json!({"op":"subscribe", "topics":added}).to_string())).await.is_err() { break; }
                                sent_topics = topics.clone();
                            }
                        }
                        Some(Command::Call { method, params, reply, expires }) => {
                            if !ready || expires <= Instant::now() || reply.is_closed() {
                                let _ = reply.send(Err(anyhow!("Request expired or hub disconnected; request was not sent")));
                                continue;
                            }
                            if pending.len() >= 64 {
                                let _ = reply.send(Err(anyhow!("Too many pending requests; request was not sent")));
                                continue;
                            }
                            counter += 1;
                            let id = counter.to_string();
                            let frame = json!({"op":"call", "id":id, "method":method, "params":params});
                            pending.insert(id, (expires, reply));
                            if !matches!(timeout(Duration::from_secs(5), socket.send(Message::Text(frame.to_string()))).await, Ok(Ok(()))) { break; }
                        }
                    },
                    message = socket.next() => {
                        last_received = Instant::now();
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                let Ok(v) = serde_json::from_str::<Value>(&text) else { break; };
                                match v["op"].as_str() {
                                    Some("hello") if !ready => {
                                        ready = true;
                                        backoff = Duration::from_millis(250);
                                        connected.store(true, Ordering::Release);
                                        if socket.send(Message::Text(json!({"op":"subscribe", "topics":topics}).to_string())).await.is_err() { break; }
                                        sent_topics = topics.clone();
                                        if events.try_send(Event::Connected).is_err() { break; }
                                    }
                                    Some("result" | "error") => {
                                        if let Some((_, reply)) = v["id"].as_str().and_then(|id| pending.remove(id)) {
                                            let result = if v["op"] == "error" { Err(anyhow!(v["error"].as_str().unwrap_or("Hub rejected request").to_owned())) }
                                                else if v["result"]["ok"] == false { Err(anyhow!(v["result"]["error"].as_str().unwrap_or("Operation failed").to_owned())) }
                                                else { Ok(v["result"].clone()) };
                                            let _ = reply.send(result);
                                        }
                                    }
                                    Some("event") if ready => {
                                        let event = &v["event"];
                                        if let Some(topic) = event["type"].as_str() {
                                            let outgoing = Event::Data { topic: topic.into(), data: event["data"].clone(), hub: event["hub"].as_str().filter(|s| !s.is_empty()).map(str::to_owned) };
                                            // Overflow deliberately reconnects and reseeds. Dropping a
                                            // fragment silently would leave an apparently live stale UI.
                                            if events.try_send(outgoing).is_err() { break; }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Some(Ok(Message::Ping(bytes))) => if socket.send(Message::Pong(bytes)).await.is_err() { break; },
                            Some(Ok(Message::Pong(_))) => {}
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                            _ => {}
                        }
                    }
                }
            }
            connected.store(false, Ordering::Release);
            for (_, (_, reply)) in pending {
                let _ = reply.send(Err(anyhow!(
                    "Hub disconnected; request outcome unknown. Check the session before retrying"
                )));
            }
        }
        connected.store(false, Ordering::Release);
        if events.send(Event::Disconnected(reason)).await.is_err() {
            return;
        }
        let wait = tokio::time::sleep(backoff);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                cmd = commands.recv() => if !offline_command(cmd, &mut topics) { return; },
                _ = events.closed() => return,
            }
        }
        backoff = (backoff * 2).min(Duration::from_secs(8));
    }
}

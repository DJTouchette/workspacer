//! Shared native backend boundary. Hub projections and launch setup stay intact;
//! an owned local engine can execute session controls through in-process channels.
use crate::{
    bus::{Client, Config, Event},
    controller::Action,
    launch::CatalogKey,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};

#[derive(Clone)]
pub struct Backend {
    hub: HubClient,
}

#[derive(Clone)]
enum HubClient {
    Remote(Client),
    #[cfg(feature = "rust-hub")]
    Embedded(workspacer_hub::client::Client),
}

impl HubClient {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match self {
            Self::Remote(client) => client.call(method, params).await,
            #[cfg(feature = "rust-hub")]
            Self::Embedded(client) => client.call(method, params).await,
        }
    }
    async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        match self {
            Self::Remote(client) => client.call_with_timeout(method, params, timeout).await,
            #[cfg(feature = "rust-hub")]
            Self::Embedded(client) => client.call_with_timeout(method, params, timeout).await,
        }
    }
    async fn topics(&self, topics: BTreeSet<String>) -> Result<()> {
        match self {
            Self::Remote(client) => client.topics(topics).await,
            #[cfg(feature = "rust-hub")]
            Self::Embedded(client) => client.topics(topics).await,
        }
    }
}

impl Backend {
    pub fn can_resume_power_pause(&self) -> bool {
        matches!(&self.hub, HubClient::Remote(_))
    }
    pub fn resume_power_pause(&self) -> Result<()> {
        match &self.hub {
            HubClient::Remote(client) => client.resume_power_pause(),
            #[cfg(feature = "rust-hub")]
            HubClient::Embedded(_) => anyhow::bail!(
                "Local connection is paused; restart it through its owning host when ready"
            ),
        }
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let value = self
            .hub
            .call_with_timeout(method, params, Duration::from_secs(90))
            .await?;
        anyhow::ensure!(
            value.get("ok") != Some(&Value::Bool(false)),
            "{}",
            value["error"].as_str().unwrap_or("Request was refused")
        );
        Ok(value)
    }
    pub fn connect(config: Config) -> (Self, async_channel::Receiver<Event>) {
        let (hub, events) = Client::start(config);
        (
            Self {
                hub: HubClient::Remote(hub),
            },
            events,
        )
    }

    #[cfg(feature = "rust-hub")]
    pub async fn in_process(
        handle: &workspacer_hub::Handle,
    ) -> Result<(Self, async_channel::Receiver<Event>)> {
        use tokio::sync::broadcast::error::RecvError;
        let client = workspacer_hub::client::Client::connect(handle).await?;
        let mut incoming = client.events();
        let mut status = handle.status();
        let (sender, events) = async_channel::bounded(256);
        sender.send(Event::Connected).await?;
        let close_client = client.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _=sender.closed()=>break,
                    reason=close_client.disconnected_reason()=>{
                        forward_embedded_close(&sender,&mut status,reason).await;
                        break;
                    },
                    changed = status.changed() => {
                        if changed.is_err() || !matches!(*status.borrow(), workspacer_hub::Status::Ready { .. }) {
                            let _ = sender.send(Event::Disconnected("Local Rust backend stopped".into())).await;
                            break;
                        }
                    }
                    event = incoming.recv() => match event {
                        Ok(event) => {
                            if sender.send(Event::Data { topic: event.topic, data: event.data.unwrap_or(Value::Null), hub:(!event.hub.is_empty()).then_some(event.hub) }).await.is_err() {break;}
                        }
                        Err(RecvError::Lagged(_)) => {
                            // Reuse the controller's reconnect reconciliation;
                            // a lost snapshot must not leave an ended row live.
                            if sender.send(Event::Disconnected("Local event stream requires reconciliation".into())).await.is_err() {break;}
                            if sender.send(Event::Connected).await.is_err() {break;}
                        }
                        Err(RecvError::Closed) => {
                            let reason=close_client.disconnected_reason().await;
                            forward_embedded_close(&sender,&mut status,reason).await;
                            break;
                        },
                    },
                }
            }
        });
        Ok((
            Self {
                hub: HubClient::Embedded(client),
            },
            events,
        ))
    }

    pub async fn snapshots(&self) -> Result<Value> {
        self.hub.call("sessions.snapshots", json!({})).await
    }

    pub async fn conversation(&self, id: &str) -> Result<Value> {
        self.hub
            .call("sessions.conversation", json!({"sessionId":id}))
            .await
    }

    pub async fn models(&self, key: &CatalogKey) -> Result<Value> {
        let (method, params) = if key.provider == "claude" {
            ("claude.listModels", json!({}))
        } else {
            (
                "providers.listModels",
                json!({"provider":key.provider,"cwd":key.cwd,
                    "useHomeDirectory": key.provider == "codex" && key.cwd.is_empty()}),
            )
        };
        self.hub
            .call_with_timeout(method, params, Duration::from_secs(60))
            .await
    }

    pub async fn spawn(&self, params: Value) -> Result<Value> {
        // Never bypass facade injection, worktrees, skills, or owner registration.
        self.hub
            .call_with_timeout("agents.spawn", params, Duration::from_secs(360))
            .await
    }

    pub async fn topics(&self, topics: BTreeSet<String>) -> Result<()> {
        self.hub.topics(topics).await
    }

    pub async fn action(&self, id: &str, action: &Action, _stream: bool) -> Result<Value> {
        let (method, params) = action.wire(id);
        self.call(method, params).await
    }
}

#[cfg(feature = "rust-hub")]
async fn forward_embedded_close(
    sender: &async_channel::Sender<Event>,
    status: &mut tokio::sync::watch::Receiver<workspacer_hub::Status>,
    reason: workspacer_hub::client::DisconnectReason,
) {
    if !reason.is_power_paused() {
        let _ = sender
            .send(Event::Disconnected("Local hub connection closed".into()))
            .await;
        return;
    }
    if sender.send(Event::PowerPaused).await.is_err() {
        return;
    }
    // A paused viewer is not a request to shut down its embedded engine. Keep
    // the controller channel alive without polling or constructing a new peer.
    loop {
        tokio::select! {
            _=sender.closed()=>return,
            changed=status.changed()=>{if changed.is_err()||!matches!(*status.borrow(),workspacer_hub::Status::Ready{..}){return;}}
        }
    }
}

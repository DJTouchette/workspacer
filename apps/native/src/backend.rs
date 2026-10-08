//! Shared native backend boundary. Hub projections and launch setup stay intact;
//! an owned local engine can execute session controls through in-process channels.
use crate::{
    bus::{Client, Config, Event},
    controller::Action,
    launch::CatalogKey,
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, LazyLock, Mutex, Weak},
    time::Duration,
};

#[derive(Clone)]
pub struct Backend {
    hub: HubClient,
    project_writes: Arc<tokio::sync::Mutex<u64>>,
}

/// One project transaction lock and snapshot revision per hub in this process.
/// Every window shares the host's controller, but a second controller (another host or fixture)
/// on the same bus URL must still queue behind the first: each write replaces
/// `config.projects` wholesale, so two overlapping read-patch-save rounds
/// would silently drop one change. Weak entries free a lock with its last
/// backend. Other processes are outside this lock (see `crate::projects`).
static PROJECT_WRITES: LazyLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<u64>>>>> =
    LazyLock::new(Default::default);

fn project_writes_for(url: &str) -> Arc<tokio::sync::Mutex<u64>> {
    let key = url::Url::parse(url).map_or_else(|_| url.to_owned(), |u| u.to_string());
    let mut locks = PROJECT_WRITES.lock().unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(0));
    locks.insert(key, Arc::downgrade(&lock));
    lock
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
    /// Serializes one complete project transaction (fresh `config.get`,
    /// patch, `config.save`, readback) against every other on this hub. FIFO,
    /// shared with registry reads. The guarded counter orders snapshots by
    /// transaction completion, independently of request-start numbers. Waiting
    /// happens on the backend runtime, never the UI thread; a dropped job
    /// releases its turn.
    pub async fn project_write(&self) -> tokio::sync::OwnedMutexGuard<u64> {
        self.project_writes.clone().lock_owned().await
    }
    pub fn connect(config: Config) -> (Self, async_channel::Receiver<Event>) {
        let project_writes = project_writes_for(&config.url);
        let (hub, events) = Client::start(config);
        (
            Self {
                hub: HubClient::Remote(hub),
                project_writes,
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
                    _ = sender.closed() => break,
                    reason = close_client.disconnected_reason() => {
                        forward_embedded_close(&sender, &mut status, reason).await;
                        break;
                    }
                    changed = status.changed() => {
                        let ready = matches!(*status.borrow(), workspacer_hub::Status::Ready { .. });
                        if changed.is_err() || !ready {
                            let stopped = Event::Disconnected("Local Rust backend stopped".into());
                            let _ = sender.send(stopped).await;
                            break;
                        }
                    }
                    event = incoming.recv() => match event {
                        Ok(event) => {
                            let data = Event::Data {
                                topic: event.topic,
                                data: event.data.unwrap_or(Value::Null),
                                hub: (!event.hub.is_empty()).then_some(event.hub),
                            };
                            if sender.send(data).await.is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(_)) => {
                            // Reuse the controller's reconnect reconciliation;
                            // a lost snapshot must not leave an ended row live.
                            let lagged =
                                Event::Disconnected("Local event stream requires reconciliation".into());
                            if sender.send(lagged).await.is_err()
                                || sender.send(Event::Connected).await.is_err()
                            {
                                break;
                            }
                        }
                        Err(RecvError::Closed) => {
                            let reason = close_client.disconnected_reason().await;
                            forward_embedded_close(&sender, &mut status, reason).await;
                            break;
                        }
                    },
                }
            }
        });
        Ok((
            Self {
                hub: HubClient::Embedded(client),
                // An owned hub has exactly one in-process backend.
                project_writes: Default::default(),
            },
            events,
        ))
    }

    /// Account usage windows for every provider; see [`crate::usage`].
    pub async fn usage_report(&self) -> Result<Value> {
        self.hub.call("usage.report", json!({})).await
    }

    pub async fn snapshots(&self) -> Result<Value> {
        self.hub.call("sessions.snapshots", json!({})).await
    }

    /// One session's row by id, including one the fleet list no longer shows.
    pub async fn snapshot(&self, id: &str) -> Result<Value> {
        self.hub
            .call("sessions.snapshot", json!({"sessionId": id}))
            .await
    }

    /// The newest `limit` items, or everything retained when `None`.
    pub async fn conversation(&self, id: &str, limit: Option<usize>) -> Result<Value> {
        let mut params = json!({"sessionId":id});
        if let Some(limit) = limit {
            params["limit"] = json!(limit);
        }
        self.hub.call("sessions.conversation", params).await
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

    /// Ask the owning hub for a handoff brief. The agent tier waits up to
    /// 150 s for the source's own brief, so this outlives the usual budget
    /// (the hub's own forwarding budget for it is 180 s). A refusal is an
    /// `ok:false` reply, read by [`crate::handoff::written`], not an error.
    pub async fn handoff_brief(&self, id: &str, brief: crate::handoff::Brief) -> Result<Value> {
        let wait = match brief {
            crate::handoff::Brief::Agent => 180,
            crate::handoff::Brief::Mechanical => 90,
        };
        self.hub
            .call_with_timeout(
                brief.method(),
                json!({"sessionId":id}),
                Duration::from_secs(wait),
            )
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
        let mut reply = self.call(method, params).await?;
        // Model then effort: the effort call only follows an accepted model
        // change, and its refusal is reported without hiding that the model
        // change went through.
        if let Action::SetModel {
            effort: Some(effort),
            ..
        } = action
            && reply["ok"] != false
        {
            let (method, params) = Action::SetEffort(effort.clone()).wire(id);
            let effort_reply = self.call(method, params).await;
            match effort_reply {
                Ok(value) if value["ok"] != false => reply["effort"] = value["effort"].clone(),
                Ok(value) => {
                    let error = value["error"].as_str().unwrap_or("refused");
                    return Ok(effort_after_model(format!("was refused: {error}")));
                }
                Err(error) => return Ok(effort_after_model(format!("failed: {error}"))),
            }
        }
        Ok(reply)
    }
}

/// The reply for a model change the hub accepted whose effort change then
/// did not go through.
fn effort_after_model(outcome: String) -> Value {
    serde_json::json!({
        "ok": false,
        "modelApplied": true,
        "error": format!("The model change was accepted, but the effort change {outcome}"),
    })
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

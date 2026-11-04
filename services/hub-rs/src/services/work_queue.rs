//! The host work queue: agent prompts and shell scripts admitted only when a
//! slot of their kind is free and the resource gate (`host_resources`) is
//! open. Contract: `docs/host-work-queue.md`.
//!
//! The queue is a durable 0600 JSON file next to `jobs.json`, rewritten
//! atomically on every state change. On start, items persisted as `running`
//! become `unknown`: their outcome is uncertain and they are never re-run.
//!
//! Identity: `source.kind == "remote"` is stamped here from the caller, never
//! taken from params. A federation peer reaches this hub by dialing its
//! `/bus?peer=1` with a credential this hub issued (`federation::run_link`,
//! `server::bus`), so the destination sees a `Caller` with `federated: true`,
//! `authenticated_host: false` (even for the host token: `Identity::
//! authenticated_host` requires `!federated`), `trusted: true` only for the
//! host token or an operator-scoped token, `scope` from that token, and
//! `token_id` = the fingerprint of the presented credential. The peer's *name*
//! is not known to the destination; `source.origin` is the submitter's own
//! (unverified, display-only) label, while `submitter` keeps the credential
//! fingerprint that may later cancel the item.
//!
//! Shell items are host code: only the server owner may submit one, or a
//! federation peer whose credential is this hub's own host credential (the
//! owner deliberately paired that peer with the host token). An operator or
//! scoped token never qualifies, federated or not.
//!
//! Placement (`queue.targets`, `host: "<peer>" | "any"`), the peer fan-out of
//! `queue.list` and remote cancel live in `routing`; scoring is the pure
//! `placement::evaluate`.
mod model;
pub mod placement;
mod routing;
mod spawn_watch;
#[cfg(test)]
mod tests;

use super::host_resources::{self, Monitor};
use crate::{
    Caller, Handle, Options, client::Client, federation::Routes, protocol::Event,
    services::remote_dispatch::Execution,
};
use anyhow::{Result, anyhow, bail};
pub use model::{
    CANCELLED, DONE, FAILED, Item, QUEUED, QueueConfig, RUNNING, ShellSpec, Source, SpawnSpec,
    UNKNOWN,
};
pub use routing::{Host, parse_host, peer_name_shaped};
use serde_json::{Value, json};
use spawn_watch::Signal;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const PUBLISH_EVERY: Duration = Duration::from_secs(15);
const RECHECK_EVERY: Duration = Duration::from_secs(60);

/// The exact owner check `jobs.*` uses (jobs.rs `Service::call`).
pub fn is_owner(caller: &Caller) -> bool {
    caller.authenticated_host && caller.trusted && caller.scope == "operator"
}
/// Whether this caller may start agents, i.e. what the bus requires of an
/// `agents.spawn` call: the host or an operator credential (`trusted`, local
/// or federated), or a scoped tier that lists `agents.spawn` (none does
/// today). Plugins are not admitted here: their spawns go through their own
/// launch path.
pub fn may_spawn(caller: &Caller) -> bool {
    caller.trusted
        || (caller.plugin_id.is_empty()
            && crate::auth::Scope::parse(&caller.scope).is_ok_and(|scope| {
                scope
                    .methods()
                    .iter()
                    .any(|m| m == "*" || m == "agents.*" || m == "agents.spawn")
            }))
}

/// Where the `queue.*` limits live: the shared config file when this hub
/// owns one, otherwise process memory (bare test and embedded hubs).
pub(crate) enum Limits {
    Shared(Arc<super::config::Config>),
    Memory(Mutex<Value>),
}
impl Limits {
    fn get(&self) -> QueueConfig {
        match self {
            Self::Shared(config) => QueueConfig::from_section(&config.get()["queue"]),
            Self::Memory(value) => QueueConfig::from_section(&value.lock().unwrap()),
        }
    }
    fn save(&self, patch: Value) -> Result<QueueConfig> {
        match self {
            Self::Shared(config) => Ok(QueueConfig::from_section(
                &config.save(json!({ "queue": patch }), true)?["queue"],
            )),
            Self::Memory(value) => {
                let mut value = value.lock().unwrap();
                *value = super::config::deep_merge(&value, &patch);
                Ok(QueueConfig::from_section(&value))
            }
        }
    }
}

struct Watch {
    session_id: String,
    seen_working: bool,
    strikes: u32,
}
struct State {
    items: Vec<Item>,
    /// Running spawn items whose session is known, by item id.
    watches: BTreeMap<String, Watch>,
    /// Running shell/spawn items that a task is executing in this process.
    active: BTreeSet<String>,
    gate_open: Option<bool>,
}

/// What the queue learns about its hub beyond the file and the monitor.
#[derive(Default)]
pub(crate) struct Wiring {
    /// `auth::fingerprint` of this hub's host token (empty when the hub has
    /// none). A federated caller presenting exactly this credential is the
    /// owner's own pairing and may submit shell items.
    pub host_fingerprint: String,
    /// The remote-dispatch readiness probe, for this host's provider logins
    /// and directory choices in `queue.targets`.
    pub execution: Option<Arc<dyn Execution>>,
}

pub struct WorkQueue {
    path: PathBuf,
    hub: Handle,
    monitor: Arc<Monitor>,
    limits: Limits,
    state: Mutex<State>,
    wake: tokio::sync::Notify,
    host_fingerprint: String,
    execution: Option<Arc<dyn Execution>>,
    /// Set once the federation manager has started (it starts after the
    /// queue is installed).
    federation: OnceLock<Routes>,
    /// The latest `host.resources` each peer published, with when it arrived.
    peer_resources: Mutex<BTreeMap<String, (Instant, Value)>>,
}

impl WorkQueue {
    /// Load (or start) the queue file. Running items from a previous process
    /// become `unknown`. A damaged file is renamed aside (never overwritten)
    /// and the queue starts empty, so one bad write cannot stop the hub.
    pub(crate) fn open(
        path: PathBuf,
        hub: Handle,
        monitor: Arc<Monitor>,
        limits: Limits,
        wiring: Wiring,
    ) -> Result<Arc<Self>> {
        let parsed = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .ok()
                .filter(Value::is_object)
                .and_then(|value| match &value["items"] {
                    Value::Null => Some(Vec::new()),
                    items => serde_json::from_value::<Vec<Item>>(items.clone()).ok(),
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut items = match parsed {
            Some(items) => items,
            None => {
                let aside = path.with_extension(format!("damaged-{}.json", now()));
                std::fs::rename(&path, &aside)?;
                eprintln!(
                    "work queue: {} was unreadable; kept as {} and starting empty",
                    path.display(),
                    aside.display()
                );
                Vec::new()
            }
        };
        items.retain(|item| !item.id.is_empty());
        let recovered = model::recover(&mut items, now());
        let queue = Arc::new(Self {
            path,
            hub,
            monitor,
            limits,
            state: Mutex::new(State {
                items,
                watches: BTreeMap::new(),
                active: BTreeSet::new(),
                gate_open: None,
            }),
            wake: tokio::sync::Notify::new(),
            host_fingerprint: wiring.host_fingerprint,
            execution: wiring.execution,
            federation: OnceLock::new(),
            peer_resources: Mutex::new(BTreeMap::new()),
        });
        if recovered {
            queue.persist(&queue.state.lock().unwrap())?;
        }
        Ok(queue)
    }

    /// Give the queue the federation routes once the manager is running.
    pub(crate) fn attach_federation(&self, routes: Routes) {
        let _ = self.federation.set(routes);
    }
    /// A federation peer presenting this hub's own host credential: the owner
    /// paired that peer with the host token on purpose. Not the owner (the
    /// caller is still federated), but trusted with host code.
    pub fn is_host_credential(&self, caller: &Caller) -> bool {
        caller.federated
            && caller.trusted
            && caller.scope == "operator"
            && caller.plugin_id.is_empty()
            && !self.host_fingerprint.is_empty()
            && caller.token_id == self.host_fingerprint
    }
    /// This hub's name on the wire.
    pub fn host(&self) -> &str {
        self.monitor.host()
    }
    /// Every item, in listing order (running, queued by queue order, then
    /// terminal newest first).
    pub fn snapshot(&self) -> Vec<Item> {
        let state = self.state.lock().unwrap();
        model::listing(&state.items).into_iter().cloned().collect()
    }
    /// The effective `queue.*` limits.
    pub fn config(&self) -> QueueConfig {
        self.limits.get()
    }
    /// The local `queue.list` answer. Peer fan-out fills `peers`.
    pub fn list(&self) -> Value {
        let items: Vec<Value> = self.snapshot().iter().map(Item::wire).collect();
        json!({"host": self.host(), "items": items, "peers": []})
    }
    /// The `host.resources` object: the latest sample, slot usage and gate.
    pub fn resources(&self) -> Value {
        let config = self.limits.get();
        let sample = self.monitor.latest();
        let reasons = host_resources::gate_reasons(&sample, &config.thresholds());
        let state = self.state.lock().unwrap();
        let count = |kind: &str, wanted: &str| {
            state
                .items
                .iter()
                .filter(|i| i.state == wanted && (kind.is_empty() || i.kind == kind))
                .count()
        };
        let mut value = sample.wire();
        value["host"] = json!(self.host());
        value["sampledAt"] = json!(sample.sampled_at);
        value["queue"] = json!({
            "agentSlots": config.max_agents,
            "agentsRunning": count("spawn", RUNNING),
            "shellSlots": config.max_shell,
            "shellRunning": count("shell", RUNNING),
            "queued": count("", QUEUED),
            "gate": {"open": reasons.is_empty(), "reasons": reasons},
        });
        value
    }

    /// Enqueue an item on this host. Checks the caller's authority for the
    /// item's kind and stamps `source` from the caller's identity.
    pub fn submit_local(&self, caller: &Caller, params: &Value) -> Result<Item> {
        let mut item = model::parse_submit(params)?;
        let host_code = is_owner(caller) || self.is_host_credential(caller);
        match item.kind.as_str() {
            "shell" if !host_code => {
                bail!(
                    "queue.submit of a shell item requires the server owner (or a federation peer paired with this hub's own host credential)"
                )
            }
            "spawn" if !may_spawn(caller) => {
                bail!("queue.submit of a spawn item requires the authority to call agents.spawn")
            }
            _ => {}
        }
        if item.spawn.as_ref().is_some_and(|s| s.full_access) && !host_code {
            bail!(
                "spawn.fullAccess requires the server owner (or a federation peer paired with this hub's own host credential)"
            );
        }
        model::check_local_paths(&item)?;
        if caller.federated {
            item.source.kind = "remote".into();
            item.submitter = caller.token_id.clone();
        } else {
            if item.source.kind == "remote" {
                item.source.kind = "prompt".into();
            }
            item.source.origin.clear();
        }
        item.id = format!("q_{}", &uuid::Uuid::new_v4().simple().to_string()[..20]);
        item.enqueued_at = now();
        item.updated_at = item.enqueued_at;
        {
            let mut state = self.state.lock().unwrap();
            state.items.push(item.clone());
            model::retain(&mut state.items);
            if let Err(error) = self.persist(&state) {
                state.items.retain(|i| i.id != item.id);
                return Err(error.context("queue item was not saved"));
            }
        }
        self.changed(&item);
        self.wake.notify_one();
        Ok(item)
    }
    /// `submit_local` with the server owner's authority, for hub-internal
    /// submitters whose own owner check already ran (the job scheduler: only
    /// the owner can store or approve a job).
    pub fn submit_as_owner(&self, params: &Value) -> Result<Item> {
        self.submit_local(&owner_caller(), params)
    }
    /// Cancel a queued item. The owner may cancel anything; a federation peer
    /// may cancel only the remote items its own credential submitted.
    pub fn cancel(&self, caller: &Caller, id: &str) -> Result<Item> {
        let mut state = self.state.lock().unwrap();
        let index = state
            .items
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| anyhow!("no queue item {id:?}"))?;
        let item = &state.items[index];
        let own_remote = caller.federated
            && item.source.kind == "remote"
            && !caller.token_id.is_empty()
            && caller.token_id == item.submitter;
        if !is_owner(caller) && !own_remote {
            bail!("queue.cancel requires the server owner");
        }
        match item.state.as_str() {
            QUEUED => {}
            RUNNING => bail!("queue item {id:?} is already running and cannot be cancelled"),
            other => bail!("queue item {id:?} is already {other}"),
        }
        let previous = state.items[index].clone();
        let item = &mut state.items[index];
        item.state = CANCELLED.into();
        item.finished_at = now();
        item.updated_at = item.finished_at;
        item.wait_reason.clear();
        item.detail = "cancelled".into();
        let item = item.clone();
        if let Err(error) = self.persist(&state) {
            state.items[index] = previous;
            return Err(error.context("cancellation was not saved"));
        }
        drop(state);
        self.changed(&item);
        self.wake.notify_one();
        Ok(item)
    }
    /// Apply a partial `queue.*` patch (owner only); returns the effective
    /// limits.
    pub fn set_config(&self, caller: &Caller, patch: &Value) -> Result<QueueConfig> {
        if !is_owner(caller) {
            bail!("queue.setConfig requires the server owner");
        }
        let patch = QueueConfig::validate_patch(patch)?;
        let config = self.limits.save(patch)?;
        self.wake.notify_one();
        Ok(config)
    }

    /// One RPC. Blocking (file writes); run off the async workers.
    pub fn call(&self, caller: &Caller, method: &str, params: Value) -> Result<Value> {
        let params = if params.is_null() { json!({}) } else { params };
        match method {
            "host.resources" => Ok(self.resources()),
            "queue.list" => {
                if !params["includePeers"].is_null() && !params["includePeers"].is_boolean() {
                    bail!("includePeers must be a boolean");
                }
                Ok(self.list())
            }
            "queue.config" => Ok(self.config().wire()),
            "queue.setConfig" => Ok(self.set_config(caller, &params)?.wire()),
            "queue.submit" => {
                if parse_host(&params)? != Host::Local {
                    bail!("host routing needs the async dispatcher (WorkQueue::dispatch)");
                }
                let item = self.submit_local(caller, &params)?;
                Ok(json!({"host": self.host(), "item": item.wire()}))
            }
            "queue.cancel" => {
                let id = params["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| anyhow!("queue.cancel requires {{id}}"))?;
                if parse_host(&params)? != Host::Local {
                    bail!("host routing needs the async dispatcher (WorkQueue::dispatch)");
                }
                Ok(json!({"ok": true, "item": self.cancel(caller, id)?.wire()}))
            }
            _ => bail!("unknown queue method {method}"),
        }
    }
    /// One RPC as the bus handlers run it: host routing and peer fan-out are
    /// async; everything else is the blocking `call` off the async workers.
    pub async fn dispatch(
        self: Arc<Self>,
        caller: Caller,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let params = if params.is_null() { json!({}) } else { params };
        match method {
            "queue.submit" => self.submit_routed(&caller, params).await,
            "queue.cancel" => self.cancel_routed(&caller, params).await,
            "queue.targets" => self.targets(&caller, &params).await,
            "queue.list" if params["includePeers"] == true => {
                Ok(self.list_with_peers(&caller).await)
            }
            _ => {
                let method = method.to_owned();
                tokio::task::spawn_blocking(move || self.call(&caller, &method, params)).await?
            }
        }
    }
    /// `submit_routed` with the server owner's authority, for the job
    /// scheduler (only the owner can store or approve a job). Returns where it
    /// landed (`local` or the peer name) and the item's id.
    pub async fn submit_routed_as_owner(
        self: &Arc<Self>,
        params: Value,
    ) -> Result<(String, String)> {
        let result = self.submit_routed(&owner_caller(), params).await?;
        let host = match result["host"].as_str() {
            Some(host) if host == self.host() => "local".to_owned(),
            Some(host) => host.to_owned(),
            None => "local".to_owned(),
        };
        let id = result["item"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("queue.submit returned no item id"))?
            .to_owned();
        Ok((host, id))
    }

    fn persist(&self, state: &State) -> Result<()> {
        super::atomic_json(
            &self.path,
            &json!({"version": 1, "items": state.items}),
            true,
        )
    }
    fn changed(&self, item: &Item) {
        let _ = self.hub.publish(Event::new(
            "queue.changed",
            "queue",
            json!({"item": item.wire()}),
        ));
    }
    fn publish_resources(&self) {
        let _ = self
            .hub
            .publish(Event::new("host.resources", "queue", self.resources()));
    }
    /// Apply `update` to one item, persist and publish. Returns the updated
    /// item (None when the item is gone).
    fn update(&self, id: &str, update: impl FnOnce(&mut Item)) -> Option<Item> {
        let mut state = self.state.lock().unwrap();
        let item = state.items.iter_mut().find(|i| i.id == id)?;
        update(item);
        item.updated_at = now().max(item.updated_at + 1);
        let item = item.clone();
        if item.is_terminal() {
            state.watches.remove(id);
            state.active.remove(id);
            model::retain(&mut state.items);
        }
        if let Err(error) = self.persist(&state) {
            eprintln!("work queue: failed to persist {id}: {error}");
        }
        drop(state);
        self.changed(&item);
        Some(item)
    }
    fn finish(&self, id: &str, outcome: &str, exit_code: Option<i32>, detail: String) {
        self.update(id, |item| {
            item.state = outcome.into();
            item.exit_code = exit_code;
            item.detail = model::clip(&detail, model::DETAIL_LIMIT);
            item.finished_at = now();
            item.wait_reason.clear();
        });
        self.wake.notify_one();
    }
    /// Re-evaluate the gate after a new sample; publish when it flips.
    fn gate_changed(&self) -> bool {
        let config = self.limits.get();
        let open =
            host_resources::gate_reasons(&self.monitor.latest(), &config.thresholds()).is_empty();
        let mut state = self.state.lock().unwrap();
        let changed = state.gate_open.is_some_and(|was| was != open);
        state.gate_open = Some(open);
        changed
    }
    /// One admission pass: mark admitted items running and refresh the wait
    /// reasons of the rest. Returns the started items.
    fn admit(&self) -> Vec<Item> {
        let config = self.limits.get();
        let gate = host_resources::gate_reasons(&self.monitor.latest(), &config.thresholds());
        let mut state = self.state.lock().unwrap();
        let plan = model::plan(&state.items, &config, &gate);
        let mut changed = Vec::new();
        let at = now();
        for item in state.items.iter_mut().filter(|i| i.state == QUEUED) {
            if plan.start.contains(&item.id) {
                item.state = RUNNING.into();
                item.started_at = at;
                item.updated_at = at.max(item.updated_at + 1);
                item.wait_reason.clear();
                changed.push(item.clone());
            } else {
                let reason = plan.waits.get(&item.id).cloned().unwrap_or_default();
                if item.wait_reason != reason {
                    item.wait_reason = reason;
                    item.updated_at = at.max(item.updated_at + 1);
                    changed.push(item.clone());
                }
            }
        }
        if changed.is_empty() {
            return Vec::new();
        }
        for item in &changed {
            if item.state == RUNNING {
                state.active.insert(item.id.clone());
            }
        }
        if let Err(error) = self.persist(&state) {
            // Not durable: do not start anything this pass.
            eprintln!("work queue: failed to persist admission: {error}");
            for item in state.items.iter_mut() {
                if plan.start.contains(&item.id) {
                    item.state = QUEUED.into();
                    item.started_at = 0;
                }
            }
            for id in &plan.start {
                state.active.remove(id);
            }
            return Vec::new();
        }
        drop(state);
        for item in &changed {
            self.changed(item);
        }
        changed.into_iter().filter(|i| i.state == RUNNING).collect()
    }
    /// A watched session's snapshot row.
    fn observe(&self, row: &Value) {
        let session = row["sessionId"].as_str().unwrap_or("");
        if session.is_empty() || row["hub"].as_str().is_some_and(|hub| !hub.is_empty()) {
            return;
        }
        let release = {
            let mut state = self.state.lock().unwrap();
            let Some((id, watch)) = state
                .watches
                .iter_mut()
                .find(|(_, watch)| watch.session_id == session)
            else {
                return;
            };
            match spawn_watch::classify(row, watch.seen_working) {
                Signal::Wait => None,
                Signal::Working => {
                    watch.seen_working = true;
                    watch.strikes = 0;
                    None
                }
                Signal::Release { failed, detail } => Some((id.clone(), failed, detail)),
            }
        };
        if let Some((id, failed, detail)) = release {
            self.finish(&id, if failed { FAILED } else { DONE }, None, detail);
        }
    }
    async fn recheck(self: &Arc<Self>, client: &Client) {
        let watched: Vec<(String, String)> = {
            let state = self.state.lock().unwrap();
            state
                .watches
                .iter()
                .map(|(id, watch)| (id.clone(), watch.session_id.clone()))
                .collect()
        };
        for (id, session) in watched {
            let row = client
                .call_with_timeout(
                    "sessions.snapshot",
                    json!({"sessionId": session}),
                    Duration::from_secs(10),
                )
                .await
                .ok();
            let release = {
                let mut state = self.state.lock().unwrap();
                let Some(watch) = state.watches.get_mut(&id) else {
                    continue;
                };
                let (signal, strikes) =
                    spawn_watch::recheck(row.as_ref(), watch.seen_working, watch.strikes);
                watch.strikes = strikes;
                match signal {
                    Signal::Working => {
                        watch.seen_working = true;
                        None
                    }
                    Signal::Wait => None,
                    Signal::Release { failed, detail } => Some((failed, detail)),
                }
            };
            if let Some((failed, detail)) = release {
                let queue = self.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    queue.finish(&id, if failed { FAILED } else { DONE }, None, detail)
                })
                .await;
            }
        }
    }
    fn start(self: &Arc<Self>, item: Item, client: &Client, tasks: &mut tokio::task::JoinSet<()>) {
        let queue = self.clone();
        if item.kind == "shell" {
            tasks.spawn(async move {
                let spec = item.shell.clone().unwrap_or_default();
                let (outcome, exit_code, detail) = match run_shell(&spec).await {
                    Ok((output, code, ok)) => (
                        if ok { DONE } else { FAILED },
                        code,
                        model::tail(&output, model::DETAIL_LIMIT),
                    ),
                    Err(error) => (FAILED, None, error.to_string()),
                };
                let _ = tokio::task::spawn_blocking(move || {
                    queue.finish(&item.id, outcome, exit_code, detail)
                })
                .await;
            });
        } else {
            let client = client.clone();
            tasks.spawn(async move {
                let result = start_spawn(&item, &client).await;
                let _ = tokio::task::spawn_blocking(move || match result {
                    Ok(session) => {
                        let attached = queue.update(&item.id, |stored| {
                            stored.session_id = session.clone();
                            stored.detail = format!("spawned {session}");
                        });
                        if attached.is_some_and(|i| i.state == RUNNING) {
                            queue.state.lock().unwrap().watches.insert(
                                item.id.clone(),
                                Watch {
                                    session_id: session,
                                    seen_working: false,
                                    strikes: 0,
                                },
                            );
                        }
                    }
                    Err(error) => queue.finish(&item.id, FAILED, None, error.to_string()),
                })
                .await;
            });
        }
    }

    async fn run(self: Arc<Self>) -> Result<()> {
        self.hub.ready().await?;
        let client = Client::connect_service(&self.hub).await?;
        let mut events = client.events();
        client
            .topics(BTreeSet::from([
                "agent.snapshot".to_owned(),
                "host.resources".to_owned(),
            ]))
            .await?;
        let mut tasks = tokio::task::JoinSet::new();
        let mut sample = tokio::time::interval(host_resources::SAMPLE_EVERY);
        let mut publish = tokio::time::interval(PUBLISH_EVERY);
        let mut recheck = tokio::time::interval(RECHECK_EVERY);
        for timer in [&mut sample, &mut publish, &mut recheck] {
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        }
        recheck.tick().await;
        loop {
            tokio::select! {
                _ = sample.tick() => {
                    let queue = self.clone();
                    let flipped = tokio::task::spawn_blocking(move || {
                        queue.monitor.sample();
                        queue.gate_changed()
                    }).await?;
                    if flipped {
                        self.publish_resources();
                    }
                }
                _ = publish.tick() => {
                    self.publish_resources();
                    continue;
                }
                _ = recheck.tick() => {
                    self.recheck(&client).await;
                    continue;
                }
                _ = self.wake.notified() => {}
                event = events.recv() => {
                    match event {
                        Ok(event) => {
                            if event.topic == "host.resources" {
                                // A peer's sample, forwarded by federation.
                                if let Some(value) = event.data {
                                    self.note_peer_resources(&event.hub, value);
                                }
                            } else if event.topic == "agent.snapshot" && let Some(row) = event.data {
                                let queue = self.clone();
                                tokio::task::spawn_blocking(move || queue.observe(&row)).await?;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            self.recheck(&client).await;
                        }
                        Err(_) => bail!("work queue lost its bus connection"),
                    }
                    continue;
                }
                Some(_) = tasks.join_next() => continue,
            }
            let queue = self.clone();
            let started = tokio::task::spawn_blocking(move || queue.admit()).await?;
            for item in started {
                self.start(item, &client, &mut tasks);
            }
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The identity of hub-internal submitters whose own owner check already ran.
fn owner_caller() -> Caller {
    Caller {
        call_id: 0,
        activity_seq: 0,
        federated: false,
        connection_id: 0,
        authenticated_host: true,
        trusted: true,
        scope: "operator".into(),
        plugin_id: String::new(),
        token_id: String::new(),
    }
}

/// Run a shell item like a jobs shell action: `/bin/sh -c` (`cmd /C` on
/// Windows) under the bounded process owner, killed on timeout. Returns the
/// combined output, the exit code and whether it succeeded.
async fn run_shell(spec: &ShellSpec) -> Result<(String, Option<i32>, bool)> {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = tokio::process::Command::new("cmd.exe");
        command.args(["/D", "/S", "/C"]);
        command
            .as_std_mut()
            .raw_arg(format!("\"{}\"", spec.command));
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", &spec.command]);
        command
    };
    if !spec.cwd.is_empty() {
        command.current_dir(&spec.cwd);
    }
    let timeout = spec.timeout_sec.clamp(1, model::MAX_SHELL_TIMEOUT_SEC);
    let output = super::owned_process::capture_combined(
        &mut command,
        64 * 1024 * 1024,
        Duration::from_secs(timeout),
    )
    .await?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
        output.status.success(),
    ))
}

/// The `agents.spawn` parameters for a spawn item: only the fields any
/// `agents.spawn` caller may set. The prompt rides the spawn as `message`
/// (see jobs.rs `perform`), so it is queued before the wrapper registers.
fn spawn_params(item: &Item) -> Value {
    let spec = item.spawn.clone().unwrap_or_default();
    let mut params = json!({"cwd": spec.cwd, "label": item.label, "message": spec.prompt});
    // `fullAccess` is the job scheduler's `agents.jobFullAccess`, mapped the
    // way an inline spawn job maps it: the provider's own approval policy
    // instead of the item's permission mode.
    if spec.full_access {
        params["skipPermissions"] = json!(true);
    }
    if let Some(window) = spec.context_window {
        params["contextWindow"] = json!(window);
    }
    for (key, value) in [
        ("provider", &spec.provider),
        ("model", &spec.model),
        ("effort", &spec.effort),
        ("permissionMode", &spec.permission_mode),
    ] {
        if !value.is_empty() && !(key == "permissionMode" && spec.full_access) {
            params[key] = json!(value);
        }
    }
    if item.source.kind == "job" && !item.source.job_id.is_empty() {
        params["jobId"] = json!(item.source.job_id);
    }
    params
}

/// Spawn the agent and make sure its prompt is delivered. Returns the session.
async fn start_spawn(item: &Item, client: &Client) -> Result<String> {
    let params = spawn_params(item);
    let prompt = params["message"].as_str().unwrap_or("").to_owned();
    let result = client
        .call_with_timeout("agents.spawn", params, Duration::from_secs(900))
        .await?;
    let id = result["sessionId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("agents.spawn returned no sessionId"))?
        .to_owned();
    if result["messageQueued"] == true {
        return Ok(id);
    }
    if result.get("messageQueued").is_some() {
        // The host tried and could not confirm delivery; a blind resend
        // could hand the agent its task twice.
        bail!("spawned {id} but the host did not confirm the prompt was queued");
    }
    // An older host that ignores `message`: send it, waiting out the wrapper
    // registration instead of losing the race.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut delay = Duration::from_millis(250);
    loop {
        match client
            .call(
                "agents.sendMessage",
                json!({"sessionId": id, "text": prompt}),
            )
            .await
        {
            Ok(_) => return Ok(id),
            Err(e)
                if e.to_string().contains("no wrapper attached")
                    && tokio::time::Instant::now() + delay < deadline =>
            {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(4));
            }
            Err(e) => bail!("spawned {id} but prompt failed: {e}"),
        }
    }
}

/// The queue file: explicit, else next to `jobs.json`, else in the data dir.
fn queue_path(options: &Options) -> Option<PathBuf> {
    options
        .work_queue_file
        .clone()
        .or_else(|| {
            options
                .jobs_file
                .as_ref()
                .map(|jobs| jobs.with_file_name("queue.json"))
        })
        .or_else(|| options.data_dir.as_ref().map(|d| d.join("queue.json")))
        .filter(|path| !path.as_os_str().is_empty())
}

pub(crate) fn install(
    mut options: Options,
    hub: Handle,
) -> Result<(Options, Option<tokio::task::JoinHandle<Result<()>>>)> {
    let Some(path) = queue_path(&options) else {
        return Ok((options, None));
    };
    let state_dir = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let monitor = Arc::new(Monitor::new(state_dir));
    let limits = match options.shared_config.clone() {
        Some(config) => Limits::Shared(config),
        None => Limits::Memory(Mutex::new(json!({}))),
    };
    let wiring = Wiring {
        host_fingerprint: crate::auth::fingerprint(&options.token),
        execution: options.remote_dispatch_execution.clone(),
    };
    let queue = WorkQueue::open(path, hub, monitor, limits, wiring)?;
    options.work_queue = Some(queue.clone());
    for method in [
        "host.resources",
        "queue.list",
        "queue.submit",
        "queue.cancel",
        "queue.config",
        "queue.setConfig",
        "queue.targets",
    ] {
        let queue = queue.clone();
        options = options.handler(method, move |caller, params| {
            queue.clone().dispatch(caller, method, params)
        });
    }
    Ok((options, Some(tokio::spawn(queue.run()))))
}

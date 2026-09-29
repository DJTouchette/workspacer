use super::{
    Capabilities, Journal, Kind, Lease, PROTOCOL, RemoteOrigin, Update, WorkerRecord, now,
    owned_worktree, sanitize_entry, valid_id,
};
use crate::{Caller, Handle, Options, protocol::Event, services::agent_lifecycle::Operation};
use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
/// Non-cloneable admission issued only after the receiver durably consumes a
/// credential-bound lease. Dropping it never makes the lease reusable.
pub struct RemoteAdmission {
    session: String,
    dispatch: String,
    lease: Lease,
}
impl RemoteAdmission {
    pub fn session_id(&self) -> &str {
        &self.session
    }
    pub fn dispatch_id(&self) -> &str {
        &self.dispatch
    }
    pub fn provider(&self) -> &str {
        &self.lease.provider
    }
    pub fn cwd(&self) -> &str {
        &self.lease.cwd
    }
    pub fn repo_cwd(&self) -> &str {
        &self.lease.repo
    }
    pub fn worktree(&self) -> bool {
        self.lease.worktree
    }
    pub fn branch(&self) -> &str {
        &self.lease.branch
    }
    pub fn origin(&self) -> Value {
        json!({"protocol":PROTOCOL,"dispatchId":self.dispatch})
    }
}
pub trait Execution: Send + Sync + 'static {
    fn capabilities(&self) -> Operation<'_, Capabilities>;
    fn canonical_directory<'a>(&'a self, cwd: &'a str) -> Operation<'a, String>;
    fn allocate<'a>(&'a self, repo: &'a str, cwd: &'a str, branch: &'a str) -> Operation<'a, ()>;
    /// Must refuse active or modified worktrees. Only generated unclaimed
    /// destinations are passed here; this is not an arbitrary directory delete.
    fn cleanup<'a>(&'a self, repo: &'a str, cwd: &'a str, branch: &'a str) -> Operation<'a, ()>;
    fn spawn<'a>(
        &'a self,
        caller: Caller,
        admission: RemoteAdmission,
        params: Value,
    ) -> Operation<'a, Value>;
    fn blocked<'a>(&'a self, _session: &'a str) -> Operation<'a, Option<bool>> {
        Box::pin(async { Ok(None) })
    }
}
pub struct Receiver {
    root: PathBuf,
    journal: Journal<WorkerRecord>,
    by_session: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    execution: Arc<dyn Execution>,
    hub: Handle,
    operations: tokio::sync::Mutex<()>,
    closing: AtomicBool,
    active: AtomicUsize,
    idle: tokio::sync::Notify,
    stop: tokio::sync::watch::Sender<bool>,
}
struct Active(Arc<Receiver>);
impl Drop for Active {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Prepare {
    remote_origin: RemoteOrigin,
    cwd: String,
    provider: String,
    #[serde(default)]
    worktree: bool,
}
impl Receiver {
    pub fn open(root: PathBuf, hub: Handle, execution: Arc<dyn Execution>) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&root)?;
        let root = crate::services::paths::canonicalize(&std::path::absolute(root)?)?;
        let journal = Journal::open_legacy(
            root.join("remote-dispatch-worker.json"),
            root.join("remote-dispatches.json"),
            "id",
            "dispatchId",
            |row: &WorkerRecord| {
                if !valid_id(&row.id)
                    || row
                        .lease
                        .as_ref()
                        .is_some_and(|lease| lease.owner.is_empty() || lease.provider.is_empty())
                    || row.seq > super::MAX_SEQ
                    || row.acked_at < 0
                {
                    bail!("invalid remote dispatch journal identity")
                }
                if row.lease.as_ref().is_some_and(|lease| {
                    lease.worktree && PathBuf::from(&lease.cwd) != owned_worktree(&root, &row.id)
                }) {
                    bail!("invalid remote dispatch journal destination")
                }
                if let Some(update) = &row.last {
                    update.validate()?;
                    if update.dispatch_id != row.id
                        || update.session_id != row.session
                        || update.seq > row.seq
                    {
                        bail!("invalid retained dispatch update")
                    }
                }
                Ok(row.id.clone())
            },
        )?;
        let mut sessions = std::collections::BTreeMap::new();
        for row in journal.list() {
            if !row.session.is_empty() && sessions.insert(row.session, row.id).is_some() {
                bail!("duplicate remote dispatch session")
            }
        }
        let (stop, _) = tokio::sync::watch::channel(false);
        Ok(Arc::new(Self {
            root,
            journal,
            by_session: std::sync::Mutex::new(sessions),
            execution,
            hub,
            operations: tokio::sync::Mutex::new(()),
            closing: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            idle: tokio::sync::Notify::new(),
            stop,
        }))
    }
    fn enter(self: &Arc<Self>) -> Result<Active> {
        if self.closing.load(Ordering::Acquire) {
            bail!("remote admission is closing")
        };
        self.active.fetch_add(1, Ordering::AcqRel);
        if self.closing.load(Ordering::Acquire) {
            if self.active.fetch_sub(1, Ordering::AcqRel) == 1 {
                self.idle.notify_waiters();
            }
            bail!("remote admission is closing")
        };
        Ok(Active(self.clone()))
    }
    fn origin(caller: &Caller, origin: &RemoteOrigin) -> Result<()> {
        origin.validate()?;
        if !caller.federated || caller.token_id.is_empty() || origin.owner_key != caller.token_id {
            bail!("authenticated paired origin required")
        };
        Ok(())
    }
    pub async fn prepare(self: &Arc<Self>, caller: Caller, params: Value) -> Result<Value> {
        let active = self.enter()?;
        tokio::spawn(async move { active.0.prepare_owned(caller, params).await }).await?
    }
    async fn prepare_owned(&self, caller: Caller, params: Value) -> Result<Value> {
        let p: Prepare = serde_json::from_value(params)?;
        Self::origin(&caller, &p.remote_origin)?;
        let _operation = self.operations.lock().await;
        let capabilities = self.execution.capabilities().await?;
        if !capabilities.executes || capabilities.protocol != PROTOCOL {
            bail!("execution host cannot run dispatched work and report back")
        }
        if !capabilities.providers.iter().any(|provider| {
            provider.provider == p.provider
                && provider.found
                && provider.authenticated == Some(true)
        }) {
            bail!("provider is not authenticated on this execution host")
        }
        let directory = capabilities
            .cwds
            .iter()
            .find(|directory| directory.path == p.cwd)
            .ok_or_else(|| {
                anyhow!("cwd must be an existing choice returned by this execution host")
            })?;
        let canonical = self.execution.canonical_directory(&p.cwd).await?;
        if canonical != p.cwd {
            bail!("remote cwd must be canonical")
        }
        if p.worktree && !directory.git {
            bail!("worktree allocation failed: isolated worktree requires an actual repository root; no worker started")
        }
        let id = &p.remote_origin.dispatch_id;
        if let Some(prior) = self.journal.get(id) {
            let lease = prior
                .lease
                .ok_or_else(|| anyhow!("dispatch has no reusable admission lease"))?;
            if lease.owner != caller.token_id
                || lease.repo != canonical
                || lease.provider != p.provider
                || lease.worktree != p.worktree
                || lease.claimed
                || !lease.prepared
                || lease.expires <= now()
            {
                bail!("dispatch already admitted or lease does not match; do not spawn again")
            };
            return Ok(lease.view());
        }
        let lease = Lease {
            owner: caller.token_id,
            repo: canonical.clone(),
            cwd: if p.worktree {
                owned_worktree(&self.root, id)
                    .to_string_lossy()
                    .into_owned()
            } else {
                canonical
            },
            provider: p.provider,
            worktree: p.worktree,
            branch: if p.worktree {
                format!("wks/paired-{id}")
            } else {
                String::new()
            },
            expires: now() + 300_000,
            claimed: false,
            prepared: !p.worktree,
        };
        self.journal.change(|rows| {
            if rows.len() >= 1024 {
                if let Some(oldest) = rows
                    .values()
                    .filter(|row| row.acked_at > 0)
                    .min_by_key(|row| row.acked_at)
                    .map(|row| row.id.clone())
                {
                    rows.remove(&oldest);
                }
            }
            if rows.len() >= 1024 {
                bail!("remote dispatch journal is full; no worker started")
            };
            rows.insert(
                id.clone(),
                WorkerRecord {
                    id: id.clone(),
                    session: String::new(),
                    seq: 0,
                    last: None,
                    lease: Some(lease.clone()),
                    acked_at: 0,
                },
            );
            Ok(())
        })?;
        self.by_session
            .lock()
            .unwrap()
            .retain(|_, dispatch| self.journal.get(dispatch).is_some());
        if lease.worktree {
            // Persist the cleanup responsibility before filesystem allocation.
            self.execution
                .allocate(&lease.repo, &lease.cwd, &lease.branch)
                .await?;
            self.journal.change(|rows| {
                rows.get_mut(id).unwrap().lease.as_mut().unwrap().prepared = true;
                Ok(())
            })?;
        }
        Ok(lease.view())
    }
    pub async fn spawn(self: &Arc<Self>, caller: Caller, params: Value) -> Result<Value> {
        let active = self.enter()?;
        tokio::spawn(async move { active.0.spawn_owned(caller, params).await }).await?
    }
    async fn spawn_owned(&self, caller: Caller, params: Value) -> Result<Value> {
        let origin: RemoteOrigin = serde_json::from_value(
            params
                .get("remoteOrigin")
                .cloned()
                .ok_or_else(|| anyhow!("remote origin required"))?,
        )?;
        Self::origin(&caller, &origin)?;
        for key in [
            "manager",
            "resumeSessionId",
            "retrySourceSessionId",
            "profileId",
            "mcpItemIds",
            "launchIntegrationId",
            "template",
            "workflowStepId",
            "taskId",
            "sessionId",
            "session_id",
        ] {
            if params
                .get(key)
                .is_some_and(|value| !value.is_null() && value != false && value != "")
            {
                bail!(
                    "remote workers require a fresh session without local process/workflow configuration"
                )
            }
        }
        let admission = {
            let _operation = self.operations.lock().await;
            let mut row = self
                .journal
                .get(&origin.dispatch_id)
                .ok_or_else(|| anyhow!("remote dispatch requires an unused matching lease"))?;
            let lease = row
                .lease
                .as_mut()
                .ok_or_else(|| anyhow!("remote dispatch has no admission lease"))?;
            if lease.owner != caller.token_id
                || params["cwd"] != lease.cwd
                || params["provider"] != lease.provider
                || lease.claimed
                || !lease.prepared
                || lease.expires <= now()
            {
                bail!(
                    "remote dispatch requires an unused matching lease; do not retry uncertain spawn"
                )
            }
            if self.execution.canonical_directory(&lease.cwd).await? != lease.cwd
                || self.execution.canonical_directory(&lease.repo).await? != lease.repo
            {
                bail!("remote dispatch directory changed after preparation")
            }
            lease.claimed = true;
            row.session = uuid::Uuid::new_v4().to_string();
            self.journal.change(|rows| {
                rows.insert(row.id.clone(), row.clone());
                Ok(())
            })?;
            self.by_session
                .lock()
                .unwrap()
                .insert(row.session.clone(), row.id.clone());
            RemoteAdmission {
                session: row.session,
                dispatch: row.id,
                lease: row.lease.unwrap(),
            }
        };
        let expected = admission.session.clone();
        let result = self.execution.spawn(caller, admission, params).await?;
        if result["sessionId"] != expected {
            bail!("remote spawn response changed admitted session; outcome unknown, do not retry")
        }
        Ok(result)
    }
    pub fn dispatch_for(&self, session: &str) -> Option<String> {
        self.by_session.lock().unwrap().get(session).cloned()
    }
    pub fn forget(&self, session: &str) {
        self.by_session.lock().unwrap().remove(session);
    }
    pub async fn report(&self, session: &str, kind: Kind, entry: Value) -> Result<bool> {
        let Some(id) = self.dispatch_for(session) else {
            return Ok(false);
        };
        let update = self.journal.change(|rows| {
            let Some(row) = rows.get_mut(&id) else {
                return Ok(None);
            };
            if row.last.as_ref().is_some_and(|last| last.terminal) {
                return Ok(None);
            };
            row.seq = row
                .seq
                .checked_add(1)
                .filter(|seq| *seq <= super::MAX_SEQ)
                .ok_or_else(|| anyhow!("dispatch sequence exhausted"))?;
            let update = Update {
                protocol: PROTOCOL,
                dispatch_id: id.clone(),
                kind,
                session_id: session.into(),
                seq: row.seq,
                ts: now(),
                terminal: kind.terminal(),
                entry: sanitize_entry(&entry, session),
            };
            row.last = Some(update.clone());
            Ok(Some(update))
        })?;
        if let Some(update) = update {
            self.hub
                .publish_wait(Event::new(
                    "agent.dispatch.update",
                    "brain",
                    serde_json::to_value(update)?,
                ))
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub async fn replay(&self, caller: &Caller, params: Value) -> Result<Value> {
        let id = params["dispatchId"]
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or_else(|| anyhow!("dispatchId is malformed"))?;
        if caller.token_id.is_empty() {
            bail!("authenticated dispatch origin required")
        }
        let Some(row) = self.journal.get(id) else {
            return Ok(json!({"state":"unknown","dispatchId":id}));
        };
        if row
            .lease
            .as_ref()
            .is_none_or(|lease| lease.owner != caller.token_id)
        {
            bail!("dispatch unavailable for this connection")
        }
        if let Some(seq) = params
            .get("ackedSeq")
            .filter(|seq| !seq.is_null())
            .map(|seq| {
                seq.as_u64()
                    .ok_or_else(|| anyhow!("invalid acknowledged sequence"))
            })
            .transpose()?
        {
            if seq > 0 {
                self.journal.change(|rows| {
                    let row = rows
                        .get_mut(id)
                        .ok_or_else(|| anyhow!("dispatch unavailable"))?;
                    if !row
                        .last
                        .as_ref()
                        .is_some_and(|last| last.terminal && last.seq == seq)
                    {
                        bail!("terminal acknowledgement does not match")
                    };
                    row.acked_at = now();
                    Ok(())
                })?;
                return Ok(json!({"state":"acknowledged"}));
            }
        }
        if let Some(last) = row.last {
            if last.kind == Kind::Blocked
                && self.execution.blocked(&row.session).await? == Some(false)
            {
                return Ok(json!({"state":"running","dispatchId":id,"sessionId":row.session}));
            }
            self.hub
                .publish_wait(Event::new(
                    "agent.dispatch.update",
                    "brain",
                    serde_json::to_value(&last)?,
                ))
                .await?;
            return Ok(
                json!({"state":"replayed","dispatchId":id,"sessionId":row.session,"kind":last.kind,"seq":last.seq}),
            );
        }
        Ok(json!({"state":"running","dispatchId":id,"sessionId":row.session}))
    }
    pub async fn sweep_at(&self, time: i64) -> Result<usize> {
        let _operation = self.operations.lock().await;
        let mut removed = 0;
        for row in self.journal.list() {
            let Some(lease) = &row.lease else { continue };
            if lease.claimed || lease.expires > time {
                continue;
            }
            if lease.worktree {
                if PathBuf::from(&lease.cwd) != owned_worktree(&self.root, &row.id) {
                    bail!("unsafe dispatch cleanup destination")
                };
                if self
                    .execution
                    .cleanup(&lease.repo, &lease.cwd, &lease.branch)
                    .await
                    .is_err()
                {
                    continue;
                }
            }
            self.journal.change(|rows| {
                rows.remove(&row.id);
                Ok(())
            })?;
            removed += 1;
        }
        Ok(removed)
    }
    pub async fn run(self: Arc<Self>) {
        let mut stop = self.stop.subscribe();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        while !*stop.borrow() {
            tokio::select! {_=stop.changed()=>{},_=tick.tick()=>{let _=self.sweep_at(now()).await;}}
        }
    }
    pub fn begin_close(&self) {
        self.closing.store(true, Ordering::Release);
        self.stop.send_replace(true);
    }
    pub async fn close(&self) {
        self.closing.store(true, Ordering::Release);
        self.stop.send_replace(true);
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                break;
            }
            notified.await;
        }
        let _operation = self.operations.lock().await;
    }
    pub fn handlers(self: &Arc<Self>, mut options: Options) -> Options {
        let native = options.spawn_coordinator.is_some();
        for method in [
            "fleet.dispatchCapabilities",
            "agents.dispatchPrepare",
            "agents.dispatchReplay",
        ] {
            let service = self.clone();
            options = options.handler(method, move |caller, params| {
                let service = service.clone();
                async move {
                    match method {
                        "fleet.dispatchCapabilities" => {
                            let mut caps = service.execution.capabilities().await?;
                            caps.scope = caller.scope.clone();
                            if native && service.hub.health().await?["launchReady"] != true {
                                caps.executes = false;
                                caps.unsupported_reason = Some(
                                    "Execution runtime is not ready to accept workers.".into(),
                                );
                            }
                            if caller.scope != "operator" || !caller.plugin_id.is_empty() {
                                caps.executes = false;
                                caps.unsupported_reason = Some(
                                    "The paired credential does not authorize worker execution."
                                        .into(),
                                );
                            }
                            Ok(serde_json::to_value(caps)?)
                        }
                        "agents.dispatchPrepare" => service.prepare(caller, params).await,
                        _ => service.replay(&caller, params).await,
                    }
                }
            });
        }
        options
    }
}

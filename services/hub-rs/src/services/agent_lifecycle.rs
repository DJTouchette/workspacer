//! Durable launch attribution and generation-fenced teardown.
//!
//! Admission (remote dispatch, workflows, worktrees and plugin ownership) must
//! precede this module. A `LaunchPreparation` implementation is mandatory: the
//! coordinator cannot silently launch without the configured facade/skills.
use super::{atomic_json, spawn_plan::Plan};
use anyhow::{Result, anyhow, bail};
use claudemon::daemon::embedded::{Command, EmbeddedClient};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
};

fn has_label(metadata: &Value) -> bool {
    metadata["label"]
        .as_str()
        .is_some_and(|s| !s.trim().is_empty())
}

pub type Operation<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Implementations own exact facade health checks, credentials, instruction and
/// MCP injection. Revoke must be idempotent and generation scoped, including
/// partially completed preparation. No bearer is written to this journal.
pub trait LaunchPreparation: Send + Sync + 'static {
    /// Sweep old session credentials only against an authoritative current live
    /// set. Needed when upgrading a store that predates the launch journal.
    fn sweep<'a>(&'a self, _live: &'a std::collections::BTreeSet<String>) -> Operation<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn prepare<'a>(&'a self, plan: &'a mut Plan, generation: &'a str) -> Operation<'a, ()>;
    fn revoke<'a>(&'a self, session: &'a str, generation: &'a str) -> Operation<'a, ()>;
}
pub trait LaunchEngine: Send + Sync + 'static {
    fn sessions(&self) -> Operation<'_, Value> {
        Box::pin(async { bail!("engine session reconciliation is unavailable") })
    }
    fn spawn<'a>(&'a self, plan: &'a Plan) -> Operation<'a, Value>;
    fn stop<'a>(&'a self, session: &'a str) -> Operation<'a, ()>;
    fn message<'a>(&'a self, _session: &'a str, _content: &'a str) -> Operation<'a, ()> {
        Box::pin(async { bail!("initial message delivery is unavailable") })
    }
}
impl LaunchEngine for EmbeddedClient {
    fn sessions(&self) -> Operation<'_, Value> {
        Box::pin(async move { self.request(Command::Sessions).await })
    }
    fn spawn<'a>(&'a self, plan: &'a Plan) -> Operation<'a, Value> {
        Box::pin(async move {
            self.request(Command::Request {
                method: "POST".into(),
                path: plan.endpoint.into(),
                payload: Some(plan.request.clone()),
            })
            .await
        })
    }
    fn message<'a>(&'a self, session: &'a str, content: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.request(Command::Message {
                id: session.into(),
                text: content.into(),
            })
            .await?;
            Ok(())
        })
    }
    fn stop<'a>(&'a self, session: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            self.request(Command::Request {
                method: "POST".into(),
                path: format!("/sessions/{session}/signal"),
                payload: Some(json!({"signal":"SIGTERM"})),
            })
            .await?;
            Ok(())
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Preparing,
    Running,
    Stopped,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRecord {
    pub generation: String,
    pub provider: String,
    pub cwd: String,
    pub metadata: Value,
    pub phase: Phase,
    /// Credentials still require eventual teardown; true is normal while running.
    pub revocation_pending: bool,
    pub receipt: Option<Value>,
    #[serde(default)]
    pub engine_attempted: bool,
    #[serde(default)]
    pub definitive_rejection: bool,
}

pub struct Lifecycle {
    path: PathBuf,
    rows: Mutex<BTreeMap<String, LaunchRecord>>,
    engine: Arc<dyn LaunchEngine>,
    preparation: Arc<dyn LaunchPreparation>,
    // Serializes all side effects, including stop versus resume. This is kept
    // separate from the read lock so snapshots can observe Preparing metadata.
    operations: tokio::sync::Mutex<()>,
    closing: std::sync::atomic::AtomicBool,
    stopping: tokio::sync::Notify,
}
impl Lifecycle {
    pub fn open(
        path: PathBuf,
        engine: Arc<dyn LaunchEngine>,
        preparation: Arc<dyn LaunchPreparation>,
    ) -> Result<Arc<Self>> {
        let rows = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Arc::new(Self {
            path,
            rows: Mutex::new(rows),
            engine,
            preparation,
            operations: tokio::sync::Mutex::new(()),
            closing: std::sync::atomic::AtomicBool::new(false),
            stopping: tokio::sync::Notify::new(),
        }))
    }
    /// Serializes explicit session teardown with launch and resume.
    pub(crate) async fn mutation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.operations.lock().await
    }
    pub fn records(&self) -> BTreeMap<String, LaunchRecord> {
        self.rows.lock().unwrap().clone()
    }
    fn update(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, LaunchRecord>) -> Result<()>,
    ) -> Result<()> {
        let mut rows = self.rows.lock().unwrap();
        let mut next = rows.clone();
        change(&mut next)?;
        atomic_json(&self.path, &serde_json::to_value(&next)?, true)?;
        *rows = next;
        Ok(())
    }
    /// Persist only daemon-confirmed live controls for the exact launch generation.
    pub async fn note_live_control(
        &self,
        session: &str,
        generation: &str,
        patch: &Value,
    ) -> Result<bool> {
        let _operation = self.operations.lock().await;
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(false);
        }
        let mut changed = false;
        self.update(|rows| {
            let Some(row) = rows
                .get_mut(session)
                .filter(|r| r.generation == generation && r.phase == Phase::Running)
            else {
                return Ok(());
            };
            for key in ["model", "effort", "permissionMode"] {
                if let Some(value) = patch["settings"][key].as_str() {
                    if !row.metadata["settings"].is_object() {
                        row.metadata["settings"] = json!({});
                    }
                    row.metadata["settings"][key] = value.into();
                    if key == "effort" {
                        // Only this confirmed-control path supplies liveEffort;
                        // configured/requested launch effort is not observation.
                        row.metadata["liveEffort"] = value.into();
                    }
                }
            }
            if let Some(value) = patch["livePermissionMode"].as_str() {
                row.metadata["livePermissionMode"] = value.into();
            }
            if patch["requestedSelection"].is_object() {
                row.metadata["requestedSelection"] = patch["requestedSelection"].clone();
            }
            changed = true;
            Ok(())
        })?;
        Ok(changed)
    }
    /// The pending automatic-title request for `session`, when one is owed:
    /// `(generation, provider, opening request)`. Only a launch that opted in
    /// with no user label carries one (see `spawn_plan`).
    pub fn auto_title_pending(&self, session: &str) -> Option<(String, String, String)> {
        let rows = self.rows.lock().unwrap();
        let row = rows.get(session)?;
        (row.metadata["autoTitle"]["state"] == "pending" && !has_label(&row.metadata)).then(|| {
            (
                row.generation.clone(),
                row.provider.clone(),
                row.metadata["autoTitle"]["prompt"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
            )
        })
    }
    /// Commit an automatic title for exactly the launch generation that asked
    /// for it. A resumed/replaced generation, a title already recorded, or a
    /// user label (launch label) all win over a late result: nothing is
    /// written and `false` is returned.
    pub async fn note_auto_title(
        &self,
        session: &str,
        generation: &str,
        outcome: Value,
    ) -> Result<bool> {
        let _operation = self.operations.lock().await;
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(false);
        }
        let mut changed = false;
        self.update(|rows| {
            let Some(row) = rows.get_mut(session).filter(|r| {
                r.generation == generation
                    && r.metadata["autoTitle"]["state"] == "pending"
                    && !has_label(&r.metadata)
            }) else {
                return Ok(());
            };
            row.metadata["autoTitle"] = outcome;
            changed = true;
            Ok(())
        })?;
        Ok(changed)
    }
    /// Overlay attribution only. Persisted records never assert process liveness
    /// and never enrich a remote session with coincidentally identical identity.
    pub fn enrich(&self, mut snapshot: Value) -> Value {
        if snapshot["hub"].as_str().is_some_and(|hub| !hub.is_empty()) {
            return snapshot;
        }
        let id = snapshot["sessionId"]
            .as_str()
            .or_else(|| snapshot["session_id"].as_str())
            .unwrap_or("");
        if let Some(record) = self.rows.lock().unwrap().get(id) {
            for key in [
                "label",
                "parentSessionId",
                "isWakeTarget",
                "routing",
                "settings",
                "livePermissionMode",
                "liveEffort",
                "requestedSelection",
                "taskId",
                "dispatchId",
                "escalationScrubbed",
                "stage",
                "workflowStepId",
                "afterDispatchId",
                "resultSchema",
                "remoteOrigin",
                "autoTitle",
            ] {
                if let Some(value) = record.metadata.get(key) {
                    if key == "settings" {
                        if !snapshot[key].is_object() {
                            snapshot[key] = json!({});
                        }
                        if let Some(values) = value.as_object() {
                            for (key, value) in values {
                                snapshot["settings"][key] = value.clone();
                            }
                        }
                    } else if key == "autoTitle" {
                        // State and result only; the recorded opening request
                        // stays in the private journal.
                        let mut value = value.clone();
                        if let Some(map) = value.as_object_mut() {
                            map.remove("prompt");
                        }
                        snapshot[key] = value;
                    } else {
                        snapshot[key] = value.clone();
                    }
                }
            }
        }
        snapshot
    }
    /// Own the operation in a task: disconnecting an RPC waiter cannot abandon
    /// a minted credential or a launched process midway through bookkeeping.
    pub async fn launch(self: &Arc<Self>, plan: Plan) -> Result<Value> {
        let service = self.clone();
        tokio::spawn(async move { service.launch_owned(plan).await }).await?
    }
    /// Keep the creation reservation alive across metadata/token preparation and
    /// the daemon's own reference-counted admission. The task owns its lifetime
    /// even when the client abandons the reply.
    pub async fn launch_reserved(
        self: &Arc<Self>,
        plan: Plan,
        reservation: super::worktrees::Reservation,
    ) -> Result<Value> {
        if plan.request["cwd"].as_str().map(std::path::Path::new) != Some(reservation.cwd.as_path())
        {
            bail!("worktree reservation differs from launch cwd");
        }
        let service = self.clone();
        tokio::spawn(async move {
            let _reservation = reservation;
            service.launch_owned(plan).await
        })
        .await?
    }
    /// Attach the committed task receipt without allowing a late admission
    /// callback to overwrite attribution from a resumed generation.
    pub async fn bind_dispatch(
        &self,
        session: &str,
        operation: &str,
        binding: &Value,
    ) -> Result<()> {
        let _operation = self.operations.lock().await;
        self.update(|rows| {
            let row = rows
                .get_mut(session)
                .ok_or_else(|| anyhow!("launch metadata unavailable"))?;
            if row.metadata["launchOperationId"] != operation {
                bail!("launch generation changed before task binding");
            }
            for key in ["taskId", "dispatchId"] {
                if let Some(value) = binding.get(key) {
                    row.metadata[key] = value.clone();
                    if let Some(receipt) = row.receipt.as_mut() {
                        receipt[key] = value.clone();
                    }
                }
            }
            Ok(())
        })
    }
    pub async fn finish_dispatch(&self, session: &str, operation: &str) -> Result<()> {
        let _operation = self.operations.lock().await;
        self.update(|rows| {
            let row = rows
                .get_mut(session)
                .ok_or_else(|| anyhow!("launch metadata unavailable"))?;
            if row.metadata["launchOperationId"] != operation {
                bail!("launch generation changed before admission completion");
            }
            let metadata = row
                .metadata
                .as_object_mut()
                .ok_or_else(|| anyhow!("invalid launch metadata"))?;
            metadata.remove("dispatchAdmission");
            metadata.remove("workflowReservationToken");
            Ok(())
        })
    }
    /// Persist child parentage after task ownership has committed. Only local
    /// lifecycle records are stored here, including pending launch attribution.
    pub async fn reparent_children(&self, from: &str, to: &str) -> Result<Vec<String>> {
        if from.is_empty() || to.is_empty() || from == to {
            bail!("invalid manager ownership transfer");
        }
        let _operation = self.operations.lock().await;
        let mut changed = Vec::new();
        self.update(|rows| {
            for (id, row) in rows {
                if id != to && row.metadata["parentSessionId"] == from {
                    row.metadata["parentSessionId"] = json!(to);
                    changed.push(id.clone());
                }
            }
            Ok(())
        })?;
        Ok(changed)
    }
    /// Manual adoption moves only current live rows and genuine pending launches.
    pub async fn reparent_current_children(
        &self,
        from: &str,
        to: &str,
        live: &std::collections::BTreeSet<String>,
    ) -> Result<Vec<String>> {
        let _operation = self.operations.lock().await;
        let mut changed = Vec::new();
        self.update(|rows| {
            for (id, row) in rows {
                if id != to
                    && row.metadata["parentSessionId"] == from
                    && (live.contains(id) || row.phase == Phase::Preparing)
                {
                    row.metadata["parentSessionId"] = json!(to);
                    changed.push(id.clone());
                }
            }
            Ok(())
        })?;
        Ok(changed)
    }
    /// Stop admission and join operations already accepted before engine teardown.
    pub fn request_close(&self) {
        self.closing
            .store(true, std::sync::atomic::Ordering::Release);
        self.stopping.notify_waiters();
    }
    async fn cancelled(&self) {
        let notified = self.stopping.notified();
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        notified.await;
    }
    async fn phase<T>(&self, name: &str, future: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::select! {
            biased;
            _=self.cancelled()=>bail!("launch service is closing during {name}"),
            result=tokio::time::timeout(std::time::Duration::from_secs(30),future)=>result.map_err(|_|anyhow!("launch {name} timed out"))?,
        }
    }
    pub async fn close(&self) {
        self.request_close();
        let _operation = self.operations.lock().await;
    }
    async fn launch_owned(&self, mut plan: Plan) -> Result<Value> {
        let _operation = self.operations.lock().await;
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            bail!("agent launch service is closing");
        }
        validate(&plan)?;
        let _worktree_admission =
            claudemon::daemon::WorktreeAdmission::acquire(plan.request["cwd"].as_str().unwrap())?;
        let inventory = self
            .phase("engine inventory", self.engine.sessions())
            .await?;
        let rows = inventory
            .as_array()
            .ok_or_else(|| anyhow!("daemon session inventory must be an array"))?;
        for row in rows {
            let id = row["session_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow!("daemon session inventory lacks identity"))?;
            if id == plan.session_id && row["mode"] != "stopped" {
                bail!("session is already live in the execution engine");
            }
        }
        let session = plan.session_id.clone();
        let endpoint = plan.endpoint;
        let provider = plan.provider.clone();
        let cwd = plan.request["cwd"].clone();
        let generation = uuid::Uuid::new_v4().to_string();
        self.update(|rows| {
            if rows.get(&session).is_some_and(|r| {
                matches!(r.phase, Phase::Preparing | Phase::Running) || r.revocation_pending
            }) {
                bail!("session launch already active or awaiting credential revocation");
            }
            rows.insert(
                session.clone(),
                LaunchRecord {
                    generation: generation.clone(),
                    provider: plan.provider.clone(),
                    cwd: plan.request["cwd"].as_str().unwrap().into(),
                    metadata: plan.metadata.clone(),
                    phase: Phase::Preparing,
                    revocation_pending: true,
                    receipt: None,
                    engine_attempted: false,
                    definitive_rejection: false,
                },
            );
            Ok(())
        })?;
        let mut engine_started = false;
        let mut engine_attempted = false;
        let result = async {
            self.phase(
                "preparation",
                self.preparation.prepare(&mut plan, &generation),
            )
            .await?;
            // Preparation may add arguments and environment, but must not switch
            // the identity or endpoint whose attribution was preregistered.
            validate(&plan)?;
            if plan.session_id != session
                || plan.endpoint != endpoint
                || plan.provider != provider
                || plan.request["cwd"] != cwd
            {
                bail!("preparation changed launch identity or destination");
            }
            self.update(|rows| {
                rows.get_mut(&session).unwrap().metadata = plan.metadata.clone();
                Ok(())
            })?;
            self.update(|rows| {
                rows.get_mut(&session).unwrap().engine_attempted = true;
                Ok(())
            })?;
            engine_attempted = true;
            let response = self
                .phase("engine admission", self.engine.spawn(&plan))
                .await?;
            engine_started = true;
            if response["session_id"] != session {
                bail!("spawn response changed session identity");
            }
            let receipt = plan.receipt(&response)?;
            self.update(|rows| {
                let row = rows.get_mut(&session).unwrap();
                row.phase = Phase::Running;
                row.receipt = Some(receipt.clone());
                Ok(())
            })?;
            Ok(receipt)
        }
        .await;
        match result {
            Ok(mut receipt) => {
                // A definitive spawn acknowledgement may say the engine did not
                // queue its initial prompt (including older response shapes).
                // Deliver once inside this owned launch task. Never enter this
                // branch after a lost spawn acknowledgement, and never turn a
                // message failure into a failed spawn inviting another process.
                if let Some(message) = plan.request["first_message"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                {
                    if receipt["messageQueued"] != true {
                        match self
                            .phase("initial message", self.engine.message(&session, message))
                            .await
                        {
                            Ok(()) => {
                                receipt["messageQueued"] = json!(true);
                            }
                            Err(error) => {
                                receipt["messageQueued"] = json!(false);
                                receipt["messageError"] = json!(format!(
                                    "Initial message acknowledgement unavailable; do not retry automatically: {error}"
                                ));
                            }
                        }
                        if let Err(error) = self.update(|rows| {
                            rows.get_mut(&session).unwrap().receipt = Some(receipt.clone());
                            Ok(())
                        }) {
                            receipt["messageError"] = json!(format!(
                                "Initial message receipt could not be retained; do not retry automatically: {error}"
                            ));
                        }
                    }
                }
                Ok(receipt)
            }
            Err(error) => {
                let definitive_rejection = error
                    .downcast_ref::<claudemon::daemon::embedded::CommandRejected>()
                    .is_some()
                    && !engine_started;
                let error = if engine_attempted && !definitive_rejection {
                    anyhow!(
                        "launch admission may have executed for session {session}: {error}; inspect its outcome before retrying"
                    )
                } else {
                    error
                };
                let stop = if engine_started || (engine_attempted && !definitive_rejection) {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        self.engine.stop(&session),
                    )
                    .await
                    .map_err(|_| anyhow!("engine cleanup timed out"))
                    .and_then(|r| r)
                } else {
                    Ok(())
                };
                let revoke = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.preparation.revoke(&session, &generation),
                )
                .await
                .map_err(|_| anyhow!("credential cleanup timed out"))
                .and_then(|r| r);
                let journal = self.update(|rows| {
                    let row = rows.get_mut(&session).unwrap();
                    row.phase = if stop.is_ok() {
                        Phase::Failed
                    } else {
                        Phase::Running
                    };
                    row.revocation_pending = revoke.is_err();
                    row.definitive_rejection = definitive_rejection;
                    Ok(())
                });
                let details = [stop.err(), revoke.err(), journal.err()]
                    .into_iter()
                    .flatten()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>();
                if details.is_empty() {
                    Err(error)
                } else {
                    Err(anyhow!("{error}; cleanup: {}", details.join("; ")))
                }
            }
        }
    }
    /// Reconcile from a fresh daemon inventory while holding the launch lock.
    /// This avoids treating a queued old stopped event as evidence against a
    /// resumed generation. Missing/stopped rows revoke credentials; active rows
    /// recover a launch interrupted between engine acceptance and journal commit.
    /// A failed inventory request changes nothing and is retried by the observer.
    pub async fn reconcile(&self) -> Result<()> {
        let _operation = self.operations.lock().await;
        let inventory = self
            .phase("engine inventory", self.engine.sessions())
            .await?;
        let rows = inventory
            .as_array()
            .ok_or_else(|| anyhow!("daemon session inventory must be an array"))?;
        let mut active = std::collections::BTreeSet::new();
        for row in rows {
            let id = row["session_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("daemon session inventory lacks identity"))?;
            if row["mode"] != "stopped" {
                active.insert(id.to_owned());
            }
        }
        let mut failures = Vec::new();
        for (session, row) in self.records() {
            if self.closing.load(std::sync::atomic::Ordering::Acquire) {
                bail!("launch service closing during reconciliation");
            }
            if active.contains(&session) {
                if row.phase == Phase::Preparing {
                    self.update(|rows| {
                        rows.get_mut(&session).unwrap().phase = Phase::Running;
                        Ok(())
                    })?;
                }
                continue;
            }
            if row.phase != Phase::Stopped {
                self.update(|rows| {
                    rows.get_mut(&session).unwrap().phase = Phase::Stopped;
                    Ok(())
                })?;
            }
            if row.revocation_pending {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.preparation.revoke(&session, &row.generation),
                )
                .await
                .map_err(|_| anyhow!("credential reconciliation timed out"))
                .and_then(|r| r)
                {
                    Ok(()) => self.update(|rows| {
                        rows.get_mut(&session).unwrap().revocation_pending = false;
                        Ok(())
                    })?,
                    Err(error) => failures.push(format!("{session}: {error}")),
                }
            }
        }
        if let Err(error) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.preparation.sweep(&active),
        )
        .await
        .map_err(|_| anyhow!("credential sweep timed out"))
        .and_then(|r| r)
        {
            failures.push(error.to_string());
        }
        if !failures.is_empty() {
            bail!("credential reconciliation failed: {}", failures.join("; "));
        }
        Ok(())
    }
    /// The observer must supply the generation it saw, never guess it from a
    /// late stopped update after resume. Duplicate observations retry revocation.
    pub async fn stopped(&self, session: &str, generation: &str) -> Result<bool> {
        let _operation = self.operations.lock().await;
        let Some(row) = self.records().get(session).cloned() else {
            return Ok(false);
        };
        if row.generation != generation {
            return Ok(false);
        }
        self.update(|rows| {
            rows.get_mut(session).unwrap().phase = Phase::Stopped;
            Ok(())
        })?;
        if row.revocation_pending {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.preparation.revoke(session, generation),
            )
            .await
            .map_err(|_| anyhow!("credential cleanup timed out"))??;
            self.update(|rows| {
                rows.get_mut(session).unwrap().revocation_pending = false;
                Ok(())
            })?;
        }
        Ok(true)
    }
}
fn validate(plan: &Plan) -> Result<()> {
    if plan.session_id.is_empty()
        || !plan
            .session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || matches!(plan.session_id.as_str(), "." | "..")
    {
        bail!("invalid session identifier");
    }
    if plan.request["session_id"] != plan.session_id {
        bail!("request session identity differs from launch plan");
    }
    if !matches!(plan.endpoint, "/sessions/spawn" | "/sessions/spawn-managed") {
        bail!("invalid launch endpoint");
    }
    let cwd = plan.request["cwd"]
        .as_str()
        .ok_or_else(|| anyhow!("launch requires cwd"))?;
    if !std::path::Path::new(cwd).is_absolute() || !std::fs::metadata(cwd)?.is_dir() {
        bail!("launch cwd must be an existing absolute directory");
    }
    Ok(())
}

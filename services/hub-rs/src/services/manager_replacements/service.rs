use super::{ReplacementState, artifact, remove, saved_finish_delivery, text};
use crate::services::task_store::TaskStore;
use anyhow::{Context, Result, bail};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone, Debug)]
pub enum SendOutcome {
    Accepted,
    Rejected { status: u16, reason: String },
    Uncertain(String),
}
/// The host verifies actual engine/session/facade identity. Every operation is
/// bounded by the coordinator; absence/timeout is never an acceptance receipt.
pub trait ReplacementHost: Send + Sync + 'static {
    fn capture_gate(&self) -> Arc<Mutex<()>>;
    fn source(&self, id: &str, pane: Option<&str>) -> Result<Value>;
    fn inventory(&self, id: &str) -> Result<Vec<Value>>;
    fn evidence(&self, id: &str, workers: &[String]) -> Result<Value>;
    fn settled(&self, id: &str) -> bool;
    fn bound(&self, pane: &str, session: &str) -> bool;
    fn receipt(&self, id: String) -> BoxFuture<'_, Result<String>>;
    fn spawn(&self, id: String, launch: Value) -> BoxFuture<'_, Result<()>>;
    fn validate_successor(&self, id: String, launch: Value) -> BoxFuture<'_, Result<()>>;
    fn reparent(&self, source: String, successor: String) -> BoxFuture<'_, Result<Vec<String>>>;
    fn restore(&self, metadata: Vec<Value>) -> BoxFuture<'_, Result<()>>;
    fn send(
        &self,
        id: String,
        message: String,
        source_request: Option<Value>,
    ) -> BoxFuture<'_, Result<SendOutcome>>;
    fn pause(&self, id: String) -> BoxFuture<'_, Result<()>>;
    fn close(&self, id: String) -> BoxFuture<'_, Result<()>>;
    fn kickoff(&self, operation: &Value) -> Result<String>;
    fn recover_finishes(&self, operation: &Value) -> Result<()>;
    fn flush_finishes(&self, parents: Vec<String>) -> BoxFuture<'_, Result<()>>;
    fn retry_request(&self, target: &str, request_id: &str) -> Result<Option<Value>>;
}
#[derive(Clone, Copy)]
pub struct Timing {
    pub preparation: Duration,
    pub poll: Duration,
    pub delivery: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            preparation: Duration::from_secs(180),
            poll: Duration::from_millis(250),
            delivery: Duration::from_secs(15),
        }
    }
}
#[derive(Clone, Copy)]
enum Action {
    Prepare,
    Activate,
}
pub struct ReplacementService {
    pub state: Arc<ReplacementState>,
    tasks: Arc<TaskStore>,
    host: Arc<dyn ReplacementHost>,
    timing: Timing,
    running: Mutex<BTreeSet<String>>,
    joins: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    initialized: tokio::sync::Mutex<bool>,
    candidates: Mutex<Vec<tokio::task::AbortHandle>>,
}
fn words(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
fn same_ids(a: &[String], b: &[String]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort();
    b.sort();
    a == b
}
impl ReplacementService {
    pub fn new(
        state: Arc<ReplacementState>,
        tasks: Arc<TaskStore>,
        host: Arc<dyn ReplacementHost>,
        timing: Timing,
    ) -> Arc<Self> {
        Arc::new(Self {
            state,
            tasks,
            host,
            timing,
            running: Mutex::new(BTreeSet::new()),
            joins: Mutex::new(vec![]),
            initialized: tokio::sync::Mutex::new(false),
            candidates: Mutex::new(vec![]),
        })
    }
    async fn bounded<T>(
        &self,
        stage: &str,
        future: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::time::timeout(self.timing.delivery, future)
            .await
            .with_context(|| format!("Timed out {stage}; inspect handoff state before retrying"))?
    }
    fn tasks(&self, id: &str) -> Result<Vec<String>> {
        Ok(self
            .tasks
            .snapshot()?
            .tasks
            .iter()
            .filter(|t| t["ownerSessionId"] == id)
            .map(|t| text(&t["taskId"]).into())
            .collect())
    }
    fn projects(&self, id: &str) -> Result<Vec<String>> {
        Ok(self
            .tasks
            .snapshot()?
            .tasks
            .iter()
            .filter(|t| t["ownerSessionId"] == id)
            .map(|t| text(&t["projectCwd"]).to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
    fn ready(&self, id: &str) -> Result<bool> {
        Ok(self.state.active_count(id) == 0
            && !self
                .tasks
                .snapshot()?
                .tasks
                .iter()
                .any(|t| t["ownerSessionId"] == id && t.get("dispatchReservation").is_some()))
    }
    pub async fn initialize(self: &Arc<Self>) -> Result<()> {
        let mut initialized = self.initialized.lock().await;
        if *initialized {
            return Ok(());
        }
        self.state.recover_status()?;
        self.bounded(
            "restoring manager metadata",
            self.host.restore(self.state.recovery_metadata()),
        )
        .await?;
        for op in self.state.records() {
            let id = text(&op["operationId"]);
            if op["transferIntent"] == true && op["phase"] != "complete" {
                self.reconcile(id).await?;
            } else if op["phase"] == "recovery-required" {
                let owner = if op["committed"] == true {
                    &op["successorSessionId"]
                } else {
                    &op["sourceSessionId"]
                };
                let _ = self
                    .bounded(
                        "pausing uncertain owner",
                        self.host.pause(text(owner).into()),
                    )
                    .await;
            }
            self.host.recover_finishes(&self.state.get(id)?)?;
        }
        *initialized = true;
        Ok(())
    }
    pub fn start(self: &Arc<Self>, source: &str, pane: &str, workspace: &str) -> Result<String> {
        let gate = self.host.capture_gate();
        let _capture = gate.lock().unwrap();
        if source.is_empty()
            || pane.is_empty()
            || workspace.is_empty()
            || source.len() > 128
            || pane.len() > 256
        {
            bail!("Source session and owning pane required");
        }
        if let Some(previous) = self.state.records().iter().find(|o| {
            o["sourceSessionId"] == source
                && (!matches!(text(&o["phase"]), "failed" | "cancelled")
                    || o["deliveries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|d| matches!(text(&d["status"]), "uncertain" | "pending")))
        }) {
            if previous["paneId"] != pane {
                bail!("Manager handoff belongs to another pane");
            }
            return Ok(text(&previous["operationId"]).into());
        }
        let _admission = self.state.handoff_admission(source)?;
        let launch = self.host.source(source, Some(pane))?;
        validate_launch(&launch)?;
        let metadata = self.host.inventory(source)?;
        let workers: Vec<_> = metadata
            .iter()
            .filter(|m| m["parentSessionId"] == source)
            .map(|m| text(&m["sessionId"]).to_string())
            .collect();
        let evidence = self.host.evidence(source, &workers)?;
        let tasks = self.tasks(source)?;
        let projects = self.projects(source)?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = artifact::create_path(text(&launch["options"]["cwd"]), &id)?;
        let now = chrono::Utc::now().timestamp_millis();
        self.state.edit(|data|{
            if data.operations.len()>=32{bail!("Replacement journal capacity reached; audit history retained");}
            for m in &metadata{data.pending_metadata.remove(text(&m["sessionId"]));}
            let deliveries:Vec<_>=evidence["inFlightMessages"].as_array().into_iter().flatten().map(|frame|{let mut d=json!({"id":frame["id"],"kind":"message","text":if frame.get("sourceRequest").is_some(){json!("")}else{frame["text"].clone()},"status":"sending"});if let Some(source)=frame.get("sourceRequest"){d["sourceRequest"]=source.clone();}d}).collect();
            data.operations.push(json!({"operationId":id,"sourceSessionId":source,"successorSessionId":uuid::Uuid::new_v4().to_string(),"paneId":pane,"workspaceId":workspace,"phase":"preparing","createdAt":now,"updatedAt":now,"committed":false,"bound":false,"artifactPath":path,"workerIds":workers,"taskIds":tasks,"deliveries":deliveries,"launch":launch,"projectCwds":projects,"metadata":metadata,"signatures":evidence.get("signatures").cloned().unwrap_or(json!({})),"finishes":evidence.get("finishes").cloned().unwrap_or(json!({}))}));Ok(())
        })?;
        self.run(&id, Action::Prepare);
        Ok(id)
    }
    fn run(self: &Arc<Self>, id: &str, action: Action) {
        let mut running = self.running.lock().unwrap();
        if !running.insert(id.into()) {
            return;
        }
        let service = self.clone();
        let id = id.to_string();
        let join = tokio::spawn(async move {
            let result = match action {
                Action::Prepare => service.prepare(&id).await,
                Action::Activate => service.activate(&id).await,
            };
            if let Err(error) = result {
                if let Err(failure) = service.fail(&id, error.to_string()).await {
                    *service.state.last_error.lock().unwrap() = Some(failure.to_string());
                }
            }
            service.running.lock().unwrap().remove(&id);
        });
        self.joins.lock().unwrap().push(join);
    }
    pub async fn idle(&self, id: &str) {
        while self.running.lock().unwrap().contains(id) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    pub async fn close(&self) -> Result<()> {
        for candidate in self.candidates.lock().unwrap().drain(..) {
            candidate.abort();
        }
        let joins = std::mem::take(&mut *self.joins.lock().unwrap());
        for join in &joins {
            join.abort();
        }
        for join in joins {
            let _ = join.await;
        }
        self.running.lock().unwrap().clear();
        self.state.recover_status()
    }
    async fn prepare(self: &Arc<Self>, id: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.timing.preparation;
        loop {
            let op = self.state.get(id)?;
            if op["phase"] != "preparing" {
                return Ok(());
            }
            if self.ready(text(&op["sourceSessionId"]))? {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("Pending dispatch did not settle before handoff timeout");
            }
            tokio::time::sleep(self.timing.poll).await;
        }
        let mut op = self.state.get(id)?;
        self.bounded(
            "draining earlier completions",
            self.host
                .flush_finishes(vec![text(&op["sourceSessionId"]).into()]),
        )
        .await?;
        op = self.state.get(id)?;
        if op["phase"] != "preparing" {
            return Ok(());
        }
        if let Some(error) = self.state.error() {
            bail!(error);
        }
        let metadata = self.host.inventory(text(&op["sourceSessionId"]))?;
        let workers: Vec<_> = metadata
            .iter()
            .filter(|m| m["parentSessionId"] == op["sourceSessionId"])
            .map(|m| text(&m["sessionId"]).to_string())
            .collect();
        let tasks = self.tasks(text(&op["sourceSessionId"]))?;
        let projects = self.projects(text(&op["sourceSessionId"]))?;
        self.state.change(id, |o| {
            o["metadata"] = json!(metadata);
            o["workerIds"] = json!(workers);
            o["taskIds"] = json!(tasks);
            o["projectCwds"] = json!(projects);
            Ok(())
        })?;
        op = self.state.get(id)?;
        let preparation = if let Some(delivery) = op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["kind"] == "preparation")
        {
            text(&delivery["id"]).to_string()
        } else {
            self.state
                .delivery(id, "preparation", &artifact::preparation_prompt(&op))?
        };
        if !self
            .deliver(id, &preparation, text(&op["sourceSessionId"]))
            .await?
        {
            return Ok(());
        }
        let mut validated = None;
        while tokio::time::Instant::now() < deadline {
            op = self.state.get(id)?;
            if op["phase"] != "preparing" {
                return Ok(());
            }
            let receipt = self
                .bounded(
                    "reading handoff receipt",
                    self.host.receipt(text(&op["sourceSessionId"]).into()),
                )
                .await?;
            if self.host.settled(text(&op["sourceSessionId"]))
                && receipt.contains("```wks-manager-handoff")
                && receipt.contains(id)
            {
                let input = op.clone();
                let checked =
                    tokio::task::spawn_blocking(move || artifact::validate(&input, &receipt))
                        .await??;
                if self.host.settled(text(&op["sourceSessionId"])) {
                    validated = Some(checked);
                    break;
                }
            }
            tokio::time::sleep(self.timing.poll).await;
        }
        let (raw, hash) =
            validated.context("Checkpoint/handoff timed out; old manager remains available")?;
        self.verify_source(&op)?;
        let sealed = Path::new(text(&op["artifactPath"]))
            .parent()
            .unwrap()
            .join("validated-handoff.json");
        write_bytes(&sealed, raw.as_bytes())?;
        self.state.change(id, |o| {
            o["sealedArtifactPath"] = json!(sealed);
            o["artifact"] = raw.into();
            o["artifactHash"] = hash.into();
            o["phase"] = "spawning".into();
            Ok(())
        })?;
        let host = self.host.clone();
        let candidate = text(&op["successorSessionId"]).to_string();
        let launch = op["launch"].clone();
        let mut spawn = tokio::spawn(async move { host.spawn(candidate, launch).await });
        self.candidates.lock().unwrap().push(spawn.abort_handle());
        match tokio::time::timeout(self.timing.delivery, &mut spawn).await {
            Ok(result) => result??,
            Err(_) => {
                let service = self.clone();
                let operation = id.to_string();
                let successor = text(&op["successorSessionId"]).to_string();
                let cleanup = tokio::spawn(async move {
                    if spawn.await.is_ok() {
                        if service
                            .state
                            .get(&operation)
                            .is_ok_and(|o| matches!(text(&o["phase"]), "failed" | "cancelled"))
                        {
                            let _ = service.host.close(successor).await;
                        }
                    }
                });
                self.joins.lock().unwrap().push(cleanup);
                bail!(
                    "Successor spawn timed out; pinned late candidate will be closed without kickoff"
                );
            }
        }
        if self.state.get(id)?["phase"] != "spawning" {
            self.bounded(
                "closing parked successor",
                self.host.close(text(&op["successorSessionId"]).into()),
            )
            .await?;
            return Ok(());
        }
        self.bounded(
            "verifying successor",
            self.host
                .validate_successor(text(&op["successorSessionId"]).into(), op["launch"].clone()),
        )
        .await?;
        if self.state.get(id)?["phase"] != "spawning" {
            self.bounded(
                "closing parked successor",
                self.host.close(text(&op["successorSessionId"]).into()),
            )
            .await?;
            return Ok(());
        }
        self.verify_source(&op)?;
        self.state.change(id,|o|{
            let successor=o["successorSessionId"].clone();if !o["metadata"].as_array().unwrap().iter().any(|m|m["sessionId"]==successor){let options=&o["launch"]["options"];let m=json!({"sessionId":successor,"cwd":options["cwd"],"provider":options["provider"],"isWakeTarget":true,"label":options["label"].as_str().unwrap_or("Fleet Manager"),"transport":"stream","settings":{"model":options["model"],"contextWindow":options["contextWindow"],"effort":options["effort"],"permissionMode":options["permissionMode"]}});o["metadata"].as_array_mut().unwrap().push(m);}
            o["phase"]="transferring".into();o["transferIntent"]=true.into();Ok(())
        })?;
        self.transfer(id).await?;
        self.state.change(id, |o| {
            o["committed"] = true.into();
            o["phase"] = "binding".into();
            Ok(())
        })?;
        Ok(())
    }
    fn verify_source(&self, op: &Value) -> Result<()> {
        let source = text(&op["sourceSessionId"]);
        if self.host.source(source, Some(text(&op["paneId"])))? != op["launch"]
            || !self.ready(source)?
            || !same_ids(&self.tasks(source)?, &words(&op["taskIds"]))
            || !same_ids(
                &self
                    .host
                    .inventory(source)?
                    .iter()
                    .filter(|m| m["parentSessionId"] == source)
                    .map(|m| text(&m["sessionId"]).to_string())
                    .collect::<Vec<_>>(),
                &words(&op["workerIds"]),
            )
        {
            bail!("Source lifecycle identity or fleet changed before ownership commit");
        }
        Ok(())
    }
    async fn transfer(&self, id: &str) -> Result<()> {
        let op = self.state.get(id)?;
        let source = text(&op["sourceSessionId"]);
        let successor = text(&op["successorSessionId"]);
        if let Err(error) = self.tasks.adopt(source, successor) {
            if op["taskTransferCommitted"] != true && op["committed"] != true {
                self.state.change(id, |o| {
                    o["transferIntent"] = false.into();
                    Ok(())
                })?;
            }
            return Err(error.context("Task transaction refused before ownership changed"));
        }
        self.state.change(id, |o| {
            o["taskTransferCommitted"] = true.into();
            Ok(())
        })?;
        self.bounded(
            "transferring worker metadata",
            self.host.reparent(source.into(), successor.into()),
        )
        .await?;
        Ok(())
    }
    pub async fn deliver(&self, id: &str, delivery_id: &str, target: &str) -> Result<bool> {
        let op = self.state.get(id)?;
        let delivery = op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == delivery_id)
            .cloned()
            .context("Unknown handoff delivery")?;
        if matches!(text(&delivery["status"]), "accepted" | "reconciled") {
            return Ok(true);
        }
        if delivery["status"] != "pending" {
            return Ok(false);
        }
        let mut message = text(&delivery["text"]).to_string();
        if delivery["kind"] == "message"
            && delivery.get("sourceRequest").is_none()
            && op["committed"] == true
            && op["successorSessionId"] == target
        {
            message.push_str(&format!("\n\n[Host manager handoff {id}] Current manager/parentSessionId is {target}. Earlier owner IDs in quoted instructions refer to predecessor. Preserve task IDs and pinned policy; inspect next_workflow_step before continuation. Do not adopt again or replay worker tasks."));
        }
        self.state.change(id, |o| {
            let d = o["deliveries"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|d| d["id"] == delivery_id)
                .unwrap();
            d["status"] = "sending".into();
            d["text"] = message.clone().into();
            Ok(())
        })?;
        let deadline = tokio::time::Instant::now() + self.timing.delivery;
        let outcome = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break SendOutcome::Uncertain("Timed out awaiting daemon acknowledgement".into());
            }
            let sent = tokio::time::timeout(
                remaining,
                self.host.send(
                    target.into(),
                    message.clone(),
                    delivery.get("sourceRequest").cloned(),
                ),
            )
            .await;
            let outcome = match sent {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => SendOutcome::Uncertain(error.to_string()),
                Err(_) => {
                    SendOutcome::Uncertain("Timed out awaiting daemon acknowledgement".into())
                }
            };
            if matches!(outcome, SendOutcome::Rejected { status: 404, .. })
                && delivery.get("sourceRequest").is_none()
                && tokio::time::Instant::now() + self.timing.poll < deadline
            {
                tokio::time::sleep(self.timing.poll).await;
                continue;
            }
            break outcome;
        };
        let accepted = matches!(outcome, SendOutcome::Accepted);
        let uncertain = matches!(outcome, SendOutcome::Uncertain(_));
        self.state.change(id, |o| {
            let d = o["deliveries"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|d| d["id"] == delivery_id)
                .unwrap();
            match outcome {
                SendOutcome::Accepted => {
                    d["status"] = "accepted".into();
                    remove(d, "error");
                }
                SendOutcome::Rejected { reason, .. } => {
                    d["status"] = "pending".into();
                    d["error"] = reason.into();
                    o["phase"] = "recovery-required".into();
                    o["error"] =
                        "Daemon explicitly rejected message; retained for inspection/retry".into();
                }
                SendOutcome::Uncertain(reason) => {
                    d["status"] = "uncertain".into();
                    d["error"] = reason.into();
                    o["phase"] = "recovery-required".into();
                    o["error"] = "Acknowledgement uncertain; no automatic replay".into();
                }
            }
            Ok(())
        })?;
        if uncertain {
            let _ = self
                .bounded("pausing uncertain delivery", self.host.pause(target.into()))
                .await;
        }
        Ok(accepted)
    }
    async fn release(&self, id: &str, target: &str) -> Result<bool> {
        loop {
            let op = self.state.get(id)?;
            let next = op["deliveries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| {
                    d["kind"] == "message"
                        && !matches!(text(&d["status"]), "accepted" | "reconciled")
                })
                .map(|d| text(&d["id"]).to_string());
            let Some(next) = next else {
                return Ok(true);
            };
            if !self.deliver(id, &next, target).await? {
                return Ok(false);
            }
        }
    }
    async fn fail(&self, id: &str, error: String) -> Result<()> {
        let op = self.state.get(id)?;
        let uncertain = op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| matches!(text(&d["status"]), "sending" | "uncertain"));
        let intent = op["transferIntent"] == true;
        self.state.change(id, |o| {
            o["error"] = error.clone().into();
            for d in o["deliveries"].as_array_mut().unwrap() {
                if d["status"] == "sending" {
                    d["status"] = "uncertain".into();
                    d["error"] = error.clone().into();
                }
            }
            o["phase"] = if intent || uncertain {
                "recovery-required"
            } else {
                "failed"
            }
            .into();
            Ok(())
        })?;
        if !intent {
            let _ = self
                .bounded(
                    "closing parked successor",
                    self.host.close(text(&op["successorSessionId"]).into()),
                )
                .await;
            if uncertain {
                let _ = self
                    .bounded(
                        "pausing predecessor",
                        self.host.pause(text(&op["sourceSessionId"]).into()),
                    )
                    .await;
            }
            self.release(id, text(&op["sourceSessionId"])).await?;
        } else {
            let _ = self
                .bounded(
                    "pausing successor",
                    self.host.pause(text(&op["successorSessionId"]).into()),
                )
                .await;
        }
        Ok(())
    }
    async fn cancel(&self, id: &str) -> Result<()> {
        let op = self.state.get(id)?;
        if op["transferIntent"] == true {
            bail!("Ownership transfer began; reconcile successor instead of cancelling");
        }
        if op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| matches!(text(&d["status"]), "sending" | "uncertain"))
        {
            bail!("Resolve uncertain delivery before cancelling");
        }
        self.state.change(id, |o| {
            o["phase"] = "cancelled".into();
            o["error"] = "Handoff cancelled; old manager retained".into();
            Ok(())
        })?;
        let _ = self
            .bounded(
                "closing parked successor",
                self.host.close(text(&op["successorSessionId"]).into()),
            )
            .await;
        self.release(id, text(&op["sourceSessionId"])).await?;
        Ok(())
    }
    async fn bind(self: &Arc<Self>, id: &str) -> Result<()> {
        let op = self.state.get(id)?;
        if op["committed"] != true {
            bail!("Ownership not committed");
        }
        if !self
            .host
            .bound(text(&op["paneId"]), text(&op["successorSessionId"]))
        {
            bail!("Waiting for same pane to attach successor");
        }
        self.state.change(id, |o| {
            o["bound"] = true.into();
            Ok(())
        })?;
        if op["phase"] == "binding" {
            self.run(id, Action::Activate);
        }
        Ok(())
    }
    async fn activate(&self, id: &str) -> Result<()> {
        let op = self.state.get(id)?;
        if op["bound"] != true
            || op["deliveries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["status"] == "uncertain")
        {
            return Ok(());
        }
        self.bounded(
            "verifying successor",
            self.host
                .validate_successor(text(&op["successorSessionId"]).into(), op["launch"].clone()),
        )
        .await?;
        self.state.change(id, |o| {
            o["phase"] = "activating".into();
            Ok(())
        })?;
        let kickoff = if let Some(d) = op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["kind"] == "kickoff")
        {
            text(&d["id"]).into()
        } else {
            self.state
                .delivery(id, "kickoff", &self.host.kickoff(&op)?)?
        };
        if !self
            .deliver(id, &kickoff, text(&op["successorSessionId"]))
            .await?
        {
            return Ok(());
        }
        let parents = vec![
            text(&op["sourceSessionId"]).into(),
            text(&op["successorSessionId"]).into(),
        ];
        self.bounded(
            "draining completion wakes",
            self.host.flush_finishes(parents.clone()),
        )
        .await?;
        if !self.release(id, text(&op["successorSessionId"])).await? {
            return Ok(());
        }
        if self.state.get(id)?["finishes"]
            .as_object()
            .is_some_and(|f| !f.is_empty())
        {
            bail!("Recorded completions remain undelivered; inspect retained evidence");
        }
        self.bounded(
            "retiring predecessor",
            self.host.close(text(&op["sourceSessionId"]).into()),
        )
        .await?;
        self.bounded(
            "draining completion wakes",
            self.host.flush_finishes(parents),
        )
        .await?;
        if !self.release(id, text(&op["successorSessionId"])).await? {
            return Ok(());
        }
        self.state.change(id, |o| {
            if o["finishes"].as_object().is_some_and(|f| !f.is_empty()) {
                bail!("Recorded completions remain undelivered");
            }
            o["retired"] = true.into();
            o["phase"] = "complete".into();
            remove(o, "error");
            Ok(())
        })?;
        Ok(())
    }
    async fn reconcile(self: &Arc<Self>, id: &str) -> Result<()> {
        let running = self.running.lock().unwrap().contains(id);
        if running {
            return Ok(());
        }
        let op = self.state.get(id)?;
        if op["transferIntent"] == true {
            self.bounded(
                "verifying successor",
                self.host.validate_successor(
                    text(&op["successorSessionId"]).into(),
                    op["launch"].clone(),
                ),
            )
            .await?;
            self.transfer(id).await?;
            self.host.recover_finishes(&self.state.get(id)?)?;
            self.state.change(id, |o| {
                o["committed"] = true.into();
                o["phase"] = if o["deliveries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["status"] == "uncertain")
                {
                    "recovery-required"
                } else {
                    "binding"
                }
                .into();
                Ok(())
            })?;
            let current = self.state.get(id)?;
            if current["bound"] == true && current["phase"] == "binding" {
                self.run(id, Action::Activate);
            }
        } else if !op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| matches!(text(&d["status"]), "uncertain" | "sending"))
        {
            self.cancel(id).await?;
        }
        Ok(())
    }
    async fn resolve_delivery(self: &Arc<Self>, request: &Value) -> Result<()> {
        if request["acknowledgeDuplicateRisk"] != true {
            bail!("Explicit duplicate-risk acknowledgement required");
        }
        let id = text(&request["operationId"]);
        let running = self.running.lock().unwrap().contains(id);
        if running {
            bail!("Wait for current handoff action to settle");
        }
        let mut op = self.state.get(id)?;
        let delivery_id = text(&request["deliveryId"]);
        if !op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == delivery_id)
        {
            if let Some(worker) = delivery_id.strip_prefix("finish:") {
                if let Some(evidence) = op["finishes"].get(worker) {
                    let delivery = saved_finish_delivery(worker, evidence);
                    let expected = evidence.clone();
                    self.state.change(id, |o| {
                        if o["finishes"][worker] != expected {
                            bail!(
                                "Finish evidence changed; inspect latest reply before resolution"
                            );
                        }
                        o["deliveries"].as_array_mut().unwrap().push(delivery);
                        o["finishes"].as_object_mut().unwrap().remove(worker);
                        Ok(())
                    })?;
                    op = self.state.get(id)?;
                }
            }
        }
        let original = op["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == delivery_id)
            .context("Unknown retained delivery")?
            .clone();
        if !matches!(text(&original["status"]), "uncertain" | "pending") {
            bail!("Delivery not awaiting resolution");
        }
        let retry = request["resolution"] == "retry";
        if !retry && request["resolution"] != "accepted" {
            bail!("Invalid delivery resolution");
        }
        let target = if op["committed"] == true {
            text(&op["successorSessionId"])
        } else {
            text(&op["sourceSessionId"])
        };
        let source = if retry {
            if let Some(source) = original.get("sourceRequest") {
                Some(self.host.retry_request(target,text(&source["requestId"]))?.context("Only definitively rejected inbox delivery may retry; unknown must not replay")?)
            } else {
                None
            }
        } else {
            original.get("sourceRequest").cloned()
        };
        let next = if retry {
            uuid::Uuid::new_v4().to_string()
        } else {
            delivery_id.into()
        };
        self.state.change(id,|o|{let d=o["deliveries"].as_array_mut().unwrap().iter_mut().find(|d|d["id"]==delivery_id).unwrap();d["status"]="reconciled".into();d["error"]=if retry{"User explicitly authorized another attempt; original may still arrive"}else{"User inspected and accounted for delivery"}.into();if retry{let mut next_delivery=json!({"id":next,"kind":original["kind"],"text":original["text"],"status":"pending","error":format!("Explicit retry of {delivery_id}; may duplicate work")});if let Some(source)=source{next_delivery["sourceRequest"]=source;}o["deliveries"].as_array_mut().unwrap().push(next_delivery);}Ok(())})?;
        if retry {
            self.deliver(id, &next, target).await?;
        }
        let current = self.state.get(id)?;
        let status = current["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == next)
            .unwrap();
        if op["transferIntent"] != true
            && original["kind"] == "preparation"
            && status["status"] != "uncertain"
        {
            self.state.change(id, |o| {
                o["phase"] = "preparing".into();
                remove(o, "error");
                Ok(())
            })?;
            self.run(id, Action::Prepare);
        } else {
            self.reconcile(id).await?;
        }
        Ok(())
    }
    pub async fn request(self: &Arc<Self>, request: Value) -> Value {
        let result = async {
            if serde_json::to_vec(&request)?.len() > 4096 {
                bail!("Replacement request too large");
            }
            self.initialize().await?;
            let action = text(&request["action"]);
            if matches!(
                action,
                "start" | "cancel" | "reconcile" | "resolve-delivery"
            ) {
                self.state.clear_error();
            }
            match action {
                "list" => {}
                "start" => {
                    self.start(
                        text(&request["sourceSessionId"]),
                        text(&request["paneId"]),
                        text(&request["workspaceId"]),
                    )?;
                }
                "cancel" => self.cancel(text(&request["operationId"])).await?,
                "bind" => self.bind(text(&request["operationId"])).await?,
                "reconcile" => self.reconcile(text(&request["operationId"])).await?,
                "resolve-delivery" => self.resolve_delivery(&request).await?,
                _ => bail!("Unknown replacement action"),
            };
            Ok::<_, anyhow::Error>(())
        }
        .await;
        json!({"available":true,"operations":self.state.views(),"error":result.err().map(|e|e.to_string()).or_else(||self.state.error())})
    }
}
fn validate_launch(launch: &Value) -> Result<()> {
    let options = &launch["options"];
    if options["manager"] != true
        || options["toolScope"] != "operator"
        || options["transport"] != "stream"
        || text(&options["cwd"]).is_empty()
        || text(&launch["grants"]).is_empty()
        || options
            .get("launchIntegrationId")
            .is_some_and(|v| !text(v).is_empty())
    {
        bail!(
            "Automatic replacement requires reproducible local stream manager and authenticated action tools"
        );
    }
    Ok(())
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("artifact parent absent")?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

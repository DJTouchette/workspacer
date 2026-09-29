//! Concrete embedded-daemon adapter. Viewer bindings are the authenticated
//! owner's attached-pane map, matching the former headless companion protocol.
use super::{
    MessageTracker, ReplacementHost, ReplacementService, ReplacementState, SendOutcome, Timing,
    messages,
};
use crate::{
    Options, auth,
    services::{
        agent_lifecycle::{Lifecycle, Phase},
        agent_spawn::SpawnCoordinator,
        launch_instructions,
        manager_requests::ManagerRequests,
        snapshots,
        task_store::text,
    },
};
use anyhow::{Context, Result, bail};
use claudemon::daemon::embedded::{Command, CommandRejected, EmbeddedClient};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
/// Hooks address coalesced completions that have not reached MessageTracker yet.
/// Implementations must not hold their own lock while invoking tracked sends.
pub trait WakeControl: Send + Sync + 'static {
    fn evidence(&self, parent: &str, workers: &[String]) -> Result<Value>;
    fn flush(&self, parents: Vec<String>) -> BoxFuture<'_, Result<()>>;
    fn recover(&self, operation: &Value) -> Result<()>;
}
pub struct NativeHost {
    hub: crate::Handle,
    engine: EmbeddedClient,
    coordinator: Arc<SpawnCoordinator>,
    lifecycle: Arc<Lifecycle>,
    state: Arc<ReplacementState>,
    requests: Arc<ManagerRequests>,
    rows: Arc<RwLock<BTreeMap<String, Value>>>,
    tokens: PathBuf,
    bindings: Mutex<BTreeMap<String, String>>,
    pub tracker: Arc<MessageTracker>,
    wakes: Arc<dyn WakeControl>,
    review: Option<Arc<crate::services::fleet_review::ReviewStore>>,
}
impl NativeHost {
    pub fn new(
        options: &Options,
        wakes: Arc<dyn WakeControl>,
        hub: crate::Handle,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            hub,
            engine: options
                .engine
                .clone()
                .context("Replacement requires embedded engine")?,
            coordinator: options
                .spawn_coordinator
                .clone()
                .context("Replacement requires admitted spawn coordinator")?,
            lifecycle: options
                .launch_lifecycle
                .clone()
                .context("Replacement requires launch authority")?,
            state: options
                .replacements
                .clone()
                .context("Replacement journal unavailable")?,
            requests: options
                .workflow_runtime
                .as_ref()
                .context("Replacement requires task ownership")?
                .requests
                .clone(),
            rows: options.session_snapshots.clone(),
            tokens: options
                .scoped_tokens
                .clone()
                .context("Replacement requires session credential store")?,
            bindings: Mutex::new(BTreeMap::new()),
            tracker: options
                .message_tracker
                .clone()
                .context("Replacement requires shared message tracker")?,
            wakes,
            review: options.review_store.clone(),
        }))
    }
    pub fn set_viewer_bindings(&self, value: &Value) -> Result<()> {
        let pairs = value
            .as_array()
            .context("Invalid manager viewer bindings")?;
        if pairs.len() > 256 {
            bail!("Invalid manager viewer bindings");
        }
        let mut next = BTreeMap::new();
        for pair in pairs {
            let pair = pair
                .as_array()
                .filter(|p| p.len() == 2)
                .context("Invalid viewer identity")?;
            let pane = pair[0]
                .as_str()
                .filter(|v| v.encode_utf16().count() <= 256)
                .context("Invalid viewer pane")?;
            let id = pair[1]
                .as_str()
                .filter(|v| v.encode_utf16().count() <= 256)
                .context("Invalid viewer session")?;
            next.insert(pane.into(), id.into());
        }
        *self.bindings.lock().unwrap() = next;
        Ok(())
    }
    fn enrich(&self, row: Value) -> Value {
        self.state.enrich(self.lifecycle.enrich(row))
    }
    fn snapshot(&self, id: &str) -> Option<Value> {
        self.rows
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .map(|row| self.enrich(row))
    }
    fn fingerprint(&self, id: &str) -> Result<String> {
        let records = auth::load(&self.tokens)?;
        if !records.iter().any(|r| {
            r.label == format!("session:{id}")
                && r.scope == "operator"
                && r.metadata.get("role").is_some_and(|v| v == "manager")
        }) {
            bail!("Manager authenticated action-tool identity unavailable");
        }
        Ok(format!(
            "{:x}",
            Sha256::digest(b"{\"scope\":\"operator\",\"role\":\"manager\"}")
        ))
    }
    async fn fresh_rows(&self) -> Result<Vec<Value>> {
        let value = self.engine.request(Command::Sessions).await?;
        Ok(value
            .as_array()
            .context("Daemon sessions unavailable")?
            .iter()
            .map(|r| self.enrich(snapshots::compat(r.clone())))
            .collect())
    }
    async fn publish_ids(&self, ids: &[String]) -> Result<()> {
        let rows = self.fresh_rows().await?;
        // Factory recovery runs before the broker actor starts consuming commands.
        // No client exists yet; initial snapshots already apply this overlay.
        // Waiting for publication acknowledgement here would deadlock startup.
        if matches!(*self.hub.status().borrow(), crate::Status::Starting) {
            return Ok(());
        }
        for row in rows {
            if ids.iter().any(|id| row["sessionId"] == *id) {
                self.hub
                    .publish_wait(crate::protocol::Event::new("agent.snapshot", "brain", row))
                    .await?;
            }
        }
        Ok(())
    }
    fn selection(
        model: &str,
        context: Option<u64>,
    ) -> Result<crate::model_selection::ModelSelection> {
        crate::model_selection::normalize_model_selection(model, context)
            .map_err(|e| anyhow::anyhow!("invalid model selection: {e:?}"))
    }
}
impl ReplacementHost for NativeHost {
    fn capture_gate(&self) -> Arc<Mutex<()>> {
        self.tracker.capture_gate.clone()
    }
    fn source(&self, id: &str, pane: Option<&str>) -> Result<Value> {
        let row = self
            .snapshot(id)
            .context("Manager not present in authoritative local session snapshots")?;
        let mut launch = self
            .state
            .launch(id)
            .context("Manager launch settings not recorded")?;
        if row["status"] == "ended"
            || !text(&row["hub"]).is_empty()
            || row["isWakeTarget"] != true
            || row["transport"] != "stream"
            || pane.is_some_and(|pane| !self.bound(pane, id))
        {
            bail!("Live local stream manager and attached owner viewer are required");
        }
        if !text(&launch["options"]["launchIntegrationId"]).is_empty() {
            bail!("Automatic replacement unavailable for custom launch integrations");
        }
        if launch["configuration"] != self.coordinator.launch_configuration(&launch["options"])? {
            bail!("Manager profile configuration changed; launch cannot be reproduced");
        }
        let model = row["requestedSelection"]["model"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| row["settings"]["model"].as_str().filter(|m| !m.is_empty()))
            .or_else(|| {
                launch["options"]["model"]
                    .as_str()
                    .filter(|m| !m.is_empty())
            })
            .or_else(|| row["usage"]["model"].as_str())
            .filter(|m| !m.is_empty())
            .context("Manager model not recorded; wait for first configured turn")?;
        let context = if row["requestedSelection"].is_object() {
            row["requestedSelection"]["contextWindow"].as_u64()
        } else if row["settings"].get("contextWindow").is_some() {
            row["settings"]["contextWindow"].as_u64()
        } else {
            launch["options"]["contextWindow"].as_u64()
        };
        let selection = Self::selection(model, context)?;
        let permission = row
            .get("livePermissionMode")
            .filter(|v| !v.is_null())
            .or_else(|| {
                row["settings"]
                    .get("permissionMode")
                    .filter(|v| !v.is_null())
            })
            .or_else(|| launch["options"].get("permissionMode"))
            .cloned()
            .unwrap_or(Value::Null);
        let effort = row
            .get("liveEffort")
            .filter(|v| !v.is_null())
            .or_else(|| row["statusLine"].get("effort").filter(|v| !v.is_null()))
            .or_else(|| launch["options"].get("effort"))
            .cloned();
        launch["grants"] = self.fingerprint(id)?.into();
        let options = &mut launch["options"];
        options["cwd"] = row["cwd"].clone();
        options["provider"] = row["provider"].clone();
        options["manager"] = true.into();
        options["toolScope"] = "operator".into();
        options["transport"] = "stream".into();
        for key in ["label", "parentSessionId"] {
            if let Some(v) = row.get(key) {
                options[key] = v.clone();
            } else if key == "parentSessionId" {
                super::remove(options, key);
            }
        }
        options["model"] = selection.model.clone().into();
        options["modelIdentity"] = selection.model.into();
        if let Some(window) = selection.context_window {
            options["contextWindow"] = window.into();
        } else {
            super::remove(options, "contextWindow");
        }
        if let Some(effort) = effort {
            options["effort"] = effort;
        }
        options["permissionMode"] = permission.clone();
        options["skipPermissions"] =
            (permission == "yolo" || permission == "bypassPermissions").into();
        Ok(launch)
    }
    fn inventory(&self, id: &str) -> Result<Vec<Value>> {
        let rows: Vec<_> = self.rows.read().unwrap().values().cloned().collect();
        let mut result = Vec::new();
        let mut seen = BTreeSet::new();
        for row in rows {
            let row = self.enrich(row);
            if !text(&row["hub"]).is_empty()
                || row["sessionId"] != id && row["parentSessionId"] != id
            {
                continue;
            }
            let mut metadata = json!({});
            for key in [
                "sessionId",
                "cwd",
                "label",
                "parentSessionId",
                "isWakeTarget",
                "provider",
                "transport",
                "settings",
                "resultSchema",
                "routing",
            ] {
                if let Some(v) = row.get(key) {
                    metadata[key] = v.clone();
                }
            }
            seen.insert(text(&row["sessionId"]).to_string());
            result.push(super::metadata_only(&metadata));
        }
        for (session, record) in self.lifecycle.records() {
            if seen.contains(&session) || !matches!(record.phase, Phase::Preparing | Phase::Running)
            {
                continue;
            }
            let mut metadata = record.metadata.clone();
            metadata["sessionId"] = session.into();
            metadata["cwd"] = record.cwd.into();
            metadata["provider"] = record.provider.into();
            let metadata = self.state.enrich(metadata);
            if metadata["sessionId"] == id || metadata["parentSessionId"] == id {
                result.push(super::metadata_only(&metadata));
            }
        }
        Ok(result)
    }
    fn evidence(&self, id: &str, workers: &[String]) -> Result<Value> {
        let mut evidence = self.tracker.evidence(id);
        let wakes = self.wakes.evidence(id, workers)?;
        for worker in workers {
            if let Some(signature) = self.state.signature(worker) {
                evidence["signatures"][worker] = signature.into();
            }
        }
        for (k, v) in wakes["signatures"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.iter())
        {
            evidence["signatures"][k] = v.clone();
        }
        evidence["finishes"] = wakes.get("finishes").cloned().unwrap_or(json!({}));
        Ok(evidence)
    }
    fn settled(&self, id: &str) -> bool {
        self.snapshot(id).is_some_and(|s| {
            s["status"] != "ended"
                && s["ambientState"] == "idle"
                && (s["pendingApproval"].is_null() || s["pendingApproval"] == false)
                && s["pendingQuestions"].as_array().is_none_or(Vec::is_empty)
                && s["activeToolCalls"].as_array().is_none_or(Vec::is_empty)
                && s["backgroundTasks"].as_u64().unwrap_or(0) == 0
        })
    }
    fn bound(&self, pane: &str, id: &str) -> bool {
        self.bindings
            .lock()
            .unwrap()
            .get(pane)
            .is_some_and(|s| s == id)
    }
    fn receipt(&self, id: String) -> BoxFuture<'_, Result<String>> {
        Box::pin(async move {
            let conversation = self
                .engine
                .request(Command::Conversation { id, since: None })
                .await?;
            let items = conversation["items"]
                .as_array()
                .context("Manager conversation unavailable")?;
            let start = items
                .iter()
                .rposition(|i| i["kind"] == "user_message")
                .map(|i| i + 1)
                .unwrap_or(0);
            Ok(items[start..]
                .iter()
                .filter(|i| i["kind"] == "assistant_text")
                .filter_map(|i| i["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"))
        })
    }
    fn spawn(&self, id: String, launch: Value) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let op = self
                .state
                .related(&id)
                .filter(|o| o["successorSessionId"] == id)
                .context("Pinned successor operation unavailable")?;
            self.coordinator
                .spawn_replacement(text(&op["operationId"]), &id, launch)
                .await?;
            Ok(())
        })
    }
    fn validate_successor(&self, id: String, launch: Value) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let rows = self.fresh_rows().await?;
            let row = rows
                .iter()
                .find(|s| s["sessionId"] == id)
                .context("Successor not present in daemon snapshot")?;
            let actual = self
                .state
                .launch(&id)
                .context("Successor launch not recorded")?;
            if row["status"] == "ended"
                || row["isWakeTarget"] != true
                || row["cwd"] != launch["options"]["cwd"]
                || row["provider"] != launch["options"]["provider"]
                || !text(&row["hub"]).is_empty()
                || self.fingerprint(&id)? != launch["grants"]
                || self.coordinator.launch_configuration(&launch["options"])?
                    != launch["configuration"]
            {
                bail!("Successor identity/liveness/lifecycle fingerprint differs from source");
            }
            if self
                .state
                .related(&id)
                .is_some_and(|o| o["transferIntent"] != true)
                && row["user_prompts"].as_u64() != Some(0)
            {
                bail!("Successor must report zero user prompts before transfer");
            }
            if actual["options"]["effort"] != launch["options"]["effort"]
                || actual["options"]["permissionMode"] != launch["options"]["permissionMode"]
                || Self::selection(
                    text(&actual["options"]["model"]),
                    actual["options"]["contextWindow"].as_u64(),
                )? != Self::selection(
                    text(&launch["options"]["model"]),
                    launch["options"]["contextWindow"].as_u64(),
                )?
            {
                bail!("Successor launch selection changed");
            }
            Ok(())
        })
    }
    fn reparent(&self, source: String, successor: String) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move {
            if let Some(review) = self.review.clone() {
                let source = source.clone();
                let successor = successor.clone();
                tokio::task::spawn_blocking(move || review.adopt_owner(&source, &successor))
                    .await??;
            }
            let workers = self
                .lifecycle
                .reparent_children(&source, &successor)
                .await?;
            let mut changed = workers.clone();
            // A retry after a lost publication ACK may find no remaining old
            // parents. Re-publish every already-transferred child as well.
            changed.extend(
                self.lifecycle
                    .records()
                    .into_iter()
                    .filter(|(_, r)| r.metadata["parentSessionId"] == successor)
                    .map(|(id, _)| id),
            );
            changed.push(source);
            changed.push(successor);
            self.publish_ids(&changed).await?;
            Ok(workers)
        })
    }
    fn restore(&self, metadata: Vec<Value>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let ids: Vec<_> = metadata
                .iter()
                .map(|m| text(&m["sessionId"]).to_string())
                .collect();
            self.publish_ids(&ids).await
        })
    }
    fn send(
        &self,
        id: String,
        message: String,
        source: Option<Value>,
    ) -> BoxFuture<'_, Result<SendOutcome>> {
        Box::pin(async move {
            Ok(messages::send_engine(
                &self.engine,
                &self.tracker,
                &id,
                &message,
                &[],
                source,
                true,
                Some(&self.requests),
            )
            .await?
            .outcome)
        })
    }
    fn pause(&self, id: String) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.engine.request(Command::Interrupt { id }).await?;
            Ok(())
        })
    }
    fn close(&self, id: String) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let rows = self.fresh_rows().await?;
            if rows
                .iter()
                .find(|s| s["sessionId"] == id)
                .is_none_or(|s| s["status"] == "ended")
            {
                return Ok(());
            }
            match self
                .engine
                .request(Command::Request {
                    method: "POST".into(),
                    path: format!("/sessions/{id}/signal"),
                    payload: Some(json!({"signal":"SIGTERM"})),
                })
                .await
            {
                Ok(_) => Ok(()),
                Err(e)
                    if e.downcast_ref::<CommandRejected>()
                        .is_some_and(|e| e.status == 404) =>
                {
                    Ok(())
                }
                Err(e) => Err(e),
            }
        })
    }
    fn kickoff(&self, op: &Value) -> Result<String> {
        Ok(format!(
            "{}\n\nThe user says:\n\nHOST-OWNED MANAGER HANDOFF {}. Your fresh manager session is {}; predecessor {} is audit history only. Host committed worker AND task ownership. Do not adopt, resume or terminate predecessor, or use shared handoff.md. Use validated handoff retained at {}, SHA256 {}. Preserve pending decisions; take next action within existing authority.\n{}",
            launch_instructions::manager_doctrine(),
            text(&op["operationId"]),
            text(&op["successorSessionId"]),
            text(&op["sourceSessionId"]),
            op["sealedArtifactPath"]
                .as_str()
                .unwrap_or(text(&op["artifactPath"])),
            text(&op["artifactHash"]),
            text(&op["artifact"])
        ))
    }
    fn recover_finishes(&self, op: &Value) -> Result<()> {
        self.wakes.recover(op)
    }
    fn flush_finishes(&self, parents: Vec<String>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.wakes.flush(parents.clone()).await?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while self.tracker.has_inflight(&parents) {
                if tokio::time::Instant::now() >= deadline {
                    bail!("Earlier message acknowledgements remain in flight");
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Ok(())
        })
    }
    fn retry_request(&self, target: &str, request_id: &str) -> Result<Option<Value>> {
        if self.requests.request(target, request_id)?["delivery"] != "rejected" {
            return Ok(None);
        }
        Ok(self
            .requests
            .begin_delivery(target, request_id)?
            .map(|d| json!({"requestId":request_id,"deliveryId":d["deliveryId"]})))
    }
}
/// Call only once the concrete wake actor exists. The owner browser must submit
/// its attached viewer bindings; no layout guess is treated as acknowledgement.
pub fn install(
    options: Options,
    wakes: Arc<dyn WakeControl>,
    hub: crate::Handle,
) -> Result<(Options, Arc<ReplacementService>)> {
    let host = NativeHost::new(&options, wakes, hub)?;
    let service = ReplacementService::new(
        host.state.clone(),
        options.workflow_runtime.as_ref().unwrap().tasks.clone(),
        host.clone(),
        Timing::default(),
    );
    let handle = service.clone();
    let options = options.handler("desktop.managerReplacement", move |caller, params| {
        let host = host.clone();
        let service = handle.clone();
        async move {
            if !caller.authenticated_host || !caller.trusted {
                bail!("desktop.managerReplacement requires authenticated owner authority");
            }
            host.set_viewer_bindings(&params["bindings"])?;
            let result = service.request(params["request"].clone()).await;
            let ids: Vec<_> = service
                .state
                .records()
                .iter()
                .flat_map(|o| {
                    [
                        text(&o["sourceSessionId"]).to_string(),
                        text(&o["successorSessionId"]).to_string(),
                    ]
                })
                .collect();
            host.publish_ids(&ids).await?;
            Ok(result)
        }
    });
    Ok((options, service))
}

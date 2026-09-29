//! Paired workers book their task attempt on the origin before executing on the
//! peer. The local proxy is observation only and never a local engine session.
use super::{Capabilities, Origin, OriginRecord, PROTOCOL, origin::Booking};
use crate::{
    Caller, Handle, Options,
    federation::{Peer, Routes},
    services::{
        agent_lifecycle::Operation,
        dispatch_templates,
        library::Library,
        paths, task_store,
        workflow_runtime::{Admission, WorkflowRuntime},
    },
};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
#[derive(Clone)]
pub(crate) struct Target {
    pub peer: Peer,
    pub host: String,
}
pub(crate) fn target(root: &Path) -> Result<Option<Target>> {
    let bytes = match std::fs::read(root.join("remote-server.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let setting: Value = serde_json::from_slice(&bytes)?;
    if setting["mode"] != "workers" {
        return Ok(None);
    }
    let raw = text(&setting, "url").trim();
    anyhow::ensure!(!raw.is_empty(), "paired worker address is empty");
    let scheme = raw.contains("://");
    let mut url = url::Url::parse(&if scheme {
        raw.into()
    } else {
        format!("http://{raw}")
    })?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https" | "ws" | "wss") && url.host_str().is_some(),
        "invalid paired worker address"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "pairing uses the saved token, not URL credentials"
    );
    if !scheme && url.port().is_none() {
        url.set_port(Some(7895))
            .map_err(|_| anyhow!("invalid paired worker port"))?;
    }
    let secure = matches!(url.scheme(), "https" | "wss");
    url.set_scheme(if secure { "wss" } else { "ws" })
        .map_err(|_| anyhow!("invalid paired worker scheme"))?;
    url.set_path("/bus");
    url.set_query(None);
    url.set_fragment(None);
    let token = text(&setting, "token").to_string();
    anyhow::ensure!(!token.is_empty(), "paired worker credential is empty");
    use sha2::{Digest, Sha256};
    let identity = format!(
        "{:x}",
        Sha256::digest(format!("{}\0{}", url, token).as_bytes())
    );
    let host = url[url::Position::BeforeHost..url::Position::AfterPort].into();
    Ok(Some(Target {
        peer: Peer {
            name: format!("paired-{identity}"),
            url: url.into(),
            token,
            dispatch: true,
        },
        host,
    }))
}
pub(crate) struct Paired {
    origin: Arc<Origin>,
    routes: Routes,
    target: Target,
    workflow: Arc<WorkflowRuntime>,
    library: Library,
    hub: Handle,
    proxies: Arc<std::sync::RwLock<std::collections::BTreeMap<String, Value>>>,
    replacements: Option<Arc<crate::services::manager_replacements::ReplacementState>>,
    closing: AtomicBool,
    active: AtomicUsize,
    idle: tokio::sync::Notify,
}
struct Active(Arc<Paired>);
impl Drop for Active {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}
impl Paired {
    pub(crate) fn install(
        mut options: Options,
        origin: Arc<Origin>,
        routes: Routes,
        hub: Handle,
    ) -> Result<Options> {
        let (Some(target), Some(workflow), Some(directory)) = (
            options.paired_target.clone(),
            options.workflow_runtime.clone(),
            options.config_dir.clone(),
        ) else {
            return Ok(options);
        };
        origin.migrate_paired_peer(&target.peer.name)?;
        let service = Arc::new(Self {
            origin,
            routes,
            target,
            workflow,
            library: Library::new(directory),
            hub,
            proxies: options.remote_proxy_snapshots.clone(),
            replacements: options.replacements.clone(),
            closing: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            idle: tokio::sync::Notify::new(),
        });
        for method in ["fleet.dispatchTargets", "fleet.selectDispatchModel"] {
            let service = service.clone();
            options = options.handler(method, move |_, params| {
                let service = service.clone();
                async move {
                    if method == "fleet.dispatchTargets" {
                        service.targets().await
                    } else {
                        service.select(params).await
                    }
                }
            });
        }
        options.paired_dispatch = Some(service);
        Ok(options)
    }
    async fn capabilities(&self) -> Result<Capabilities> {
        let caps: Capabilities = serde_json::from_value(
            self.routes
                .forward(
                    &self.target.peer.name,
                    "fleet.dispatchCapabilities",
                    Value::Null,
                )
                .await?,
        )?;
        anyhow::ensure!(
            caps.protocol == PROTOCOL && caps.executes,
            "paired host cannot execute this dispatch protocol"
        );
        Ok(caps)
    }
    pub async fn targets(&self) -> Result<Value> {
        let result = self.capabilities().await;
        Ok(match result {
            Ok(caps) => {
                json!({"targets":[{"name":"paired","host":self.target.host,"connected":true,"hasToken":true,"ready":true,"readiness":"ready","protocol":caps.protocol,"providers":caps.providers,"cwds":caps.cwds}],"linkedButNotEnabled":[],"note":"Use executionTarget=paired and keep cwd as the local task project; remoteCwd must be an advertised remote directory."})
            }
            Err(error) => {
                json!({"targets":[{"name":"paired","host":self.target.host,"connected":false,"hasToken":true,"ready":false,"readiness":error.to_string(),"providers":[],"cwds":[]}],"linkedButNotEnabled":[]})
            }
        })
    }
    pub async fn select(&self, params: Value) -> Result<Value> {
        let caps = self.capabilities().await?;
        anyhow::ensure!(
            caps.cwds.iter().any(|cwd| cwd.path == params["cwd"]),
            "select an advertised remote cwd"
        );
        anyhow::ensure!(
            caps.providers
                .iter()
                .any(|provider| (text(&params, "provider").is_empty()
                    || provider.provider == params["provider"])
                    && provider.found
                    && provider.authenticated == Some(true)),
            "no authenticated remote provider is available for this selection"
        );
        let mut wire = json!({});
        for key in [
            "role",
            "provider",
            "cwd",
            "difficulty",
            "risk",
            "decisionDensity",
            "previousProvider",
            "requireIndependentFamily",
            "profile",
            "forecastDemandBeforeResetPct",
            "expectedWork",
        ] {
            if let Some(value) = params.get(key) {
                wire[key] = value.clone();
            }
        }
        let mut decision = self
            .routes
            .forward(&self.target.peer.name, "routing.select", wire)
            .await?;
        if decision["eligible"] == true
            && !caps.providers.iter().any(|provider| {
                provider.provider == decision["provider"]
                    && provider.found
                    && provider.authenticated == Some(true)
            })
        {
            decision["eligible"] = false.into();
            let reason = "The routing policy selected a provider without confirmed login on the execution host; no worker was started.";
            if let Some(reasons) = decision["reason"].as_array_mut() {
                reasons.push(reason.into());
            } else {
                decision["reason"] = json!([reason]);
            }
        }
        Ok(decision)
    }
    pub async fn spawn(self: &Arc<Self>, caller: Caller, params: Value) -> Result<Value> {
        anyhow::ensure!(
            !self.closing.load(Ordering::Acquire),
            "paired admission is closing"
        );
        self.active.fetch_add(1, Ordering::AcqRel);
        let active = Active(self.clone());
        anyhow::ensure!(
            !self.closing.load(Ordering::Acquire),
            "paired admission is closing"
        );
        tokio::spawn(async move { active.0.spawn_owned(caller, params).await }).await?
    }
    pub async fn close(&self) {
        self.closing.store(true, Ordering::Release);
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                break;
            }
            notified.await;
        }
    }
    async fn spawn_owned(&self, caller: Caller, mut params: Value) -> Result<Value> {
        anyhow::ensure!(
            !caller.federated
                && caller.plugin_id.is_empty()
                && params["executionTarget"] == "paired",
            "paired launch requires a local caller"
        );
        let owner_id = text(&params, "dispatchOwnerSessionId").to_owned();
        anyhow::ensure!(
            !owner_id.is_empty() && params["parentSessionId"] == owner_id,
            "paired dispatch requires authenticated parent attribution"
        );
        let owner = self
            .workflow
            .owner_snapshot(&owner_id)
            .ok_or_else(|| anyhow!("paired dispatch owner is unavailable"))?;
        task_store::manager(&owner)?;
        let _lineage = self
            .replacements
            .as_ref()
            .map(|state| state.admit(&[&owner_id]))
            .transpose()?;
        for key in [
            "manager",
            "resumeSessionId",
            "retrySourceSessionId",
            "profileId",
            "mcpItemIds",
            "pluginTools",
            "launchIntegrationId",
        ] {
            if params
                .get(key)
                .is_some_and(|v| !v.is_null() && v != false && v != "")
            {
                bail!("paired workers require a fresh session without local process configuration")
            }
        }
        let project = paths::canonicalize(Path::new(text(&params, "cwd")))?;
        anyhow::ensure!(project.is_dir(), "local task project is unavailable");
        let project = project.to_string_lossy().into_owned();
        params["cwd"] = project.clone().into();
        let mut tracking = json!({"owner":owner,"projectCwd":project});
        for key in [
            "trackTask",
            "taskId",
            "stage",
            "afterDispatchId",
            "workflowStepId",
        ] {
            if let Some(value) = params.get(key) {
                tracking[key] = value.clone();
            }
        }
        self.workflow.tasks.validate_admission(&tracking)?;
        // Probe before a workflow reservation mutates the local task. The peer
        // revalidates its own credentials and directory when preparing the lease.
        let caps = self.capabilities().await?;
        anyhow::ensure!(
            caps.cwds.iter().any(|cwd| cwd.path == params["remoteCwd"]),
            "choose an advertised remote cwd"
        );
        anyhow::ensure!(
            caps.providers
                .iter()
                .any(|provider| provider.provider == params["provider"]
                    && provider.found
                    && provider.authenticated == Some(true)),
            "selected provider is not authenticated on the peer"
        );
        let admission = if !text(&params, "workflowStepId").is_empty() {
            Some(self.workflow.admit(&params, &owner_id)?)
        } else {
            None
        };
        if let Some(admission) = &admission {
            params = admission.params.clone();
            tracking["workflowReservationToken"] = admission.token.clone().into();
        }
        let template = if let Some(admission) = &admission {
            Some(admission.template.clone())
        } else if !text(&params, "template").is_empty() {
            anyhow::ensure!(
                text(&params, "message").trim().is_empty(),
                "pass template or message, not both"
            );
            let items = self
                .library
                .list(&json!({"cwd":project,"id":params["template"]}))?;
            let item = items
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["scope"] != "claude")
                .ok_or_else(|| anyhow!("dispatch template unavailable"))?
                .clone();
            anyhow::ensure!(
                item["kind"] == "dispatch",
                "only dispatch templates are supported"
            );
            Some(item)
        } else {
            None
        };
        if let Some(template) = &template {
            if admission.is_some() || params.get("resultSchema").is_none_or(Value::is_null) {
                params["resultSchema"] = template["resultSchema"].clone();
            }
            dispatch_templates::render(text(template, "body"), &params["templateParams"], "", "")?;
        }
        let source = Arc::new(Source {
            session: format!("paired:{}", uuid::Uuid::new_v4()),
            params: params.clone(),
            template,
            tracking,
            workflow: self.workflow.clone(),
            owner: owner_id.clone(),
            admission,
            booked: AtomicBool::new(false),
            host: self.target.host.clone(),
            proxies: self.proxies.clone(),
            hub: self.hub.clone(),
        });
        let mut wire = json!({"cwd":params["remoteCwd"],"dispatchOwnerSessionId":owner_id});
        for key in [
            "provider",
            "model",
            "modelIdentity",
            "contextWindow",
            "effort",
            "role",
            "capability",
            "decisionId",
            "exactModel",
            "toolScope",
            "message",
            "label",
            "resultSchema",
            "worktree",
        ] {
            if let Some(value) = params.get(key) {
                wire[key] = value.clone();
            }
        }
        let result = self
            .origin
            .forward_booked(&caller, &self.target.peer.name, wire, Some(source.clone()))
            .await;
        if let Ok(receipt) = &result {
            if let Some(record) = self
                .origin
                .records()
                .iter()
                .find(|record| record.dispatch_id == receipt["remoteDispatchId"])
            {
                let snapshot = self
                    .routes
                    .forward(
                        &self.target.peer.name,
                        "sessions.snapshot",
                        json!({"sessionId":record.session_id}),
                    )
                    .await
                    .unwrap_or(json!({"status":"starting"}));
                let _ = project_snapshot(
                    record,
                    &snapshot,
                    &self.proxies,
                    Some(&self.workflow),
                    &self.hub,
                )
                .await;
            }
        }
        if !source.booked.load(Ordering::Acquire) {
            if let Some(admission) = &source.admission {
                self.workflow
                    .tasks
                    .release_workflow_dispatch(&admission.task_id, &admission.token)?;
            }
        }
        result
    }
}
struct Source {
    session: String,
    params: Value,
    template: Option<Value>,
    tracking: Value,
    workflow: Arc<WorkflowRuntime>,
    owner: String,
    admission: Option<Admission>,
    booked: AtomicBool,
    host: String,
    proxies: Arc<std::sync::RwLock<std::collections::BTreeMap<String, Value>>>,
    hub: Handle,
}
impl Booking for Source {
    fn local_session_id(&self) -> &str {
        &self.session
    }
    fn source_root(&self) -> &str {
        text(&self.params, "cwd")
    }
    fn prepare<'a>(
        &'a self,
        record: &'a OriginRecord,
        prepared: &'a Value,
    ) -> Operation<'a, Value> {
        Box::pin(async move {
            let owner = self
                .workflow
                .owner_snapshot(&self.owner)
                .ok_or_else(|| anyhow!("paired manager ended before admission"))?;
            task_store::manager(&owner)?;
            let message = if let Some(template) = &self.template {
                dispatch_templates::render(
                    text(template, "body"),
                    &self.params["templateParams"],
                    text(prepared, "cwd"),
                    text(prepared, "repo"),
                )?
            } else {
                text(&self.params, "message").into()
            };
            let worktree = json!({"requested":self.params["worktree"]==true,"allocated":prepared["worktree"],"fallback":false,"branch":prepared["branch"]});
            let mut input = self.tracking.clone();
            input["owner"] = owner;
            input["sessionId"] = self.session.clone().into();
            input["executionCwd"] = prepared["cwd"].clone();
            input["executionTarget"] = "paired".into();
            input["executionHost"] = self.host.clone().into();
            input["worktree"] = worktree.clone();
            input["title"] = self.params["label"].clone();
            for (key, source) in [
                ("provider", "provider"),
                ("requestedProvider", "provider"),
                ("requestedModel", "model"),
                ("role", "role"),
            ] {
                input[key] = self.params[source].clone();
            }
            let ids = self.workflow.tasks.accept(input, || {
                task_store::manager(
                    &self
                        .workflow
                        .owner_snapshot(&self.owner)
                        .unwrap_or(Value::Null),
                )
            })?;
            self.booked.store(true, Ordering::Release);
            if let Some(admission) = &self.admission {
                self.workflow
                    .tasks
                    .release_workflow_dispatch(&admission.task_id, &admission.token)?;
            }
            let proxy = json!({"sessionId":self.session,"parentSessionId":self.owner,"isWakeTarget":false,"hub":"@paired","status":"starting","cwd":prepared["cwd"],"provider":self.params["provider"],"label":self.params["label"],"executionTarget":"paired","remoteDispatchId":record.dispatch_id});
            self.proxies
                .write()
                .unwrap()
                .insert(self.session.clone(), proxy.clone());
            let mut event = crate::protocol::Event::new("agent.snapshot", "brain", proxy);
            event.hub = "@paired".into();
            self.hub.publish_wait(event).await?;
            let mut receipt = ids.unwrap_or(json!({}));
            receipt["renderedMessage"] = message.into();
            receipt["worktreeResult"] = worktree;
            if self.params["trackTask"] == false {
                receipt["taskTracking"] = false.into();
            }
            Ok(receipt)
        })
    }
}

pub(crate) fn result_entry(
    record: &OriginRecord,
    update: &super::Update,
    workflow: &WorkflowRuntime,
) -> Result<Value> {
    let Some(session) = &record.local_session_id else {
        return Ok(update.entry.clone());
    };
    let mut entry = update.entry.clone();
    entry["sessionId"] = session.clone().into();
    if update.terminal {
        for key in ["escalation", "escalationError", "result", "resultError"] {
            entry.as_object_mut().unwrap().remove(key);
        }
        let reply = entry["fullReply"]
            .as_str()
            .or_else(|| entry["lastReply"].as_str())
            .unwrap_or("")
            .to_owned();
        // Remote assertions are text, never local validated result authority.
        if let Some(escalation) = crate::services::worker_results::read_escalation(&reply) {
            if let Some(value) = escalation.json {
                entry["escalation"] = value.into();
            }
            if let Some(error) = escalation.error {
                entry["escalationError"] = error.into();
            }
        }
        if entry.get("escalation").is_none() {
            if let Some(schema) = &record.result_schema {
                let result = crate::services::worker_results::read_result(&reply, schema);
                if let Some(value) = result.json {
                    entry["result"] = value.into();
                }
                if let Some(error) = result.error {
                    entry["resultError"] = error.into();
                }
            }
        }
        let (contract, outcome) = if let Some(escalation) = entry["escalation"].as_str() {
            ("escalated", serde_json::from_str(escalation).ok())
        } else if let Some(result) = entry["result"].as_str() {
            ("valid", serde_json::from_str(result).ok())
        } else if entry["resultError"].is_string() {
            ("invalid", None)
        } else {
            ("absent", None)
        };
        workflow.tasks.validated(session, contract, None, outcome)?;
    }
    let lifecycle = if update.kind == super::Kind::Blocked {
        "needs-decision"
    } else if update.terminal {
        if entry["stopped"] == true {
            "ended"
        } else {
            "idle"
        }
    } else {
        "running"
    };
    if workflow.tasks.snapshot()?.tasks.iter().any(|task| {
        task["attempts"].as_array().is_some_and(|attempts| {
            attempts
                .iter()
                .any(|attempt| attempt["sessionId"] == *session)
        })
    }) {
        workflow
            .tasks
            .observe_remote(session, lifecycle, update.terminal)?;
    }
    Ok(entry)
}
pub(crate) async fn project_snapshot(
    record: &OriginRecord,
    snapshot: &Value,
    proxies: &Arc<std::sync::RwLock<std::collections::BTreeMap<String, Value>>>,
    workflow: Option<&WorkflowRuntime>,
    hub: &Handle,
) -> Result<()> {
    let Some(session) = &record.local_session_id else {
        return Ok(());
    };
    let mut proxy = json!({"sessionId":session,"parentSessionId":record.owner_session_id,"hub":"@paired","isWakeTarget":false,"status":"running","cwd":record.cwd,"provider":record.provider,"label":record.label,"executionTarget":"paired","remoteSessionId":record.session_id});
    for key in [
        "status",
        "ambientState",
        "cwd",
        "label",
        "provider",
        "mode",
        "transport",
        "usage",
        "statusLine",
        "pendingApproval",
        "pendingQuestions",
        "lastAssistantMessage",
        "lastReply",
    ] {
        if let Some(value) = snapshot.get(key) {
            proxy[key] = value.clone();
        }
    }
    proxy["connectionState"] = "connected".into();
    if let Some(workflow) = workflow {
        let lifecycle = if proxy["status"] == "ended" {
            "ended"
        } else if proxy["ambientState"] == "idle" {
            "idle"
        } else if ["waiting_approval", "waiting_input"].contains(&text(&proxy, "ambientState")) {
            "needs-decision"
        } else {
            "running"
        };
        let _ = workflow
            .tasks
            .observe_remote(session, lifecycle, record.state == "done");
    }
    proxies
        .write()
        .unwrap()
        .insert(session.clone(), proxy.clone());
    let mut event = crate::protocol::Event::new("agent.snapshot", "brain", proxy);
    event.hub = "@paired".into();
    hub.publish_wait(event).await
}

pub(crate) async fn project_retained(
    record: &OriginRecord,
    proxies: &Arc<std::sync::RwLock<std::collections::BTreeMap<String, Value>>>,
    workflow: Option<&WorkflowRuntime>,
    hub: &Handle,
) -> Result<()> {
    let Some(update) = &record.last_update else {
        return Ok(());
    };
    let mut snapshot = update.entry.clone();
    snapshot["status"] = if update.entry["stopped"] == true {
        "ended"
    } else {
        "running"
    }
    .into();
    snapshot["ambientState"] = if update.terminal {
        "idle"
    } else if update.kind == super::Kind::Blocked {
        if update.entry["blockedOn"] == "approval" {
            "waiting_approval"
        } else {
            "waiting_input"
        }
    } else {
        "working"
    }
    .into();
    project_snapshot(record, &snapshot, proxies, workflow, hub).await
}
pub(crate) async fn mark_offline(
    record: &OriginRecord,
    proxies: &Arc<std::sync::RwLock<std::collections::BTreeMap<String, Value>>>,
    hub: &Handle,
) -> Result<()> {
    let Some(session) = &record.local_session_id else {
        return Ok(());
    };
    let snapshot = {
        let mut rows = proxies.write().unwrap();
        let Some(row) = rows.get_mut(session) else {
            return Ok(());
        };
        row["connectionState"] = "offline".into();
        row.clone()
    };
    let mut event = crate::protocol::Event::new("agent.snapshot", "brain", snapshot);
    event.hub = "@paired".into();
    hub.publish_wait(event).await
}

//! Concrete local spawn orchestration. Network adapters must apply the shared
//! provenance sanitizer before calling `spawn_sanitized`. Remote dispatch and
//! plugin launch callbacks remain explicit admission adapters, not fallbacks.
use super::{
    agent_lifecycle::Lifecycle,
    config::Config,
    dispatch_templates,
    library::Library,
    paths,
    profiles::Profiles,
    routing::RoutingService,
    spawn_plan::{self, Plan},
    task_store,
    workflow_runtime::{Admission, WorkflowRuntime},
    worktrees::{Reservation, Worktrees},
};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
struct ReplacementClaim {
    operation: String,
    session: String,
    launch: Value,
}
pub struct SpawnCoordinator {
    config: Arc<Config>,
    profiles: Profiles,
    library: Library,
    home: PathBuf,
    pub lifecycle: Arc<Lifecycle>,
    pub workflow: Arc<WorkflowRuntime>,
    worktrees: Arc<Worktrees>,
    routing: Arc<RoutingService>,
    replacements: Mutex<Option<Arc<super::manager_replacements::ReplacementState>>>,
    launch_integrations: Mutex<Option<(crate::Handle, Arc<crate::plugins::launch::Preparation>)>>,
    review: Mutex<Option<Arc<super::fleet_review::ReviewStore>>>,
    accepting: Mutex<bool>,
    active: AtomicUsize,
    idle: tokio::sync::Notify,
    stopping: tokio::sync::Notify,
}
impl SpawnCoordinator {
    pub fn new(
        directory: PathBuf,
        home: PathBuf,
        config: Arc<Config>,
        lifecycle: Arc<Lifecycle>,
        workflow: Arc<WorkflowRuntime>,
        worktrees: Arc<Worktrees>,
        routing: Arc<RoutingService>,
    ) -> Arc<Self> {
        let review = worktrees.review_store();
        Arc::new(Self {
            config,
            profiles: Profiles::new(directory.clone()),
            library: Library::new(directory),
            home,
            lifecycle,
            workflow,
            worktrees,
            routing,
            replacements: Mutex::new(None),
            launch_integrations: Mutex::new(None),
            review: Mutex::new(review),
            accepting: Mutex::new(true),
            active: AtomicUsize::new(0),
            idle: tokio::sync::Notify::new(),
            stopping: tokio::sync::Notify::new(),
        })
    }
    pub fn set_review_store(&self, review: Arc<super::fleet_review::ReviewStore>) {
        *self.review.lock().unwrap() = Some(review);
    }
    pub fn set_replacements(&self, state: Arc<super::manager_replacements::ReplacementState>) {
        *self.replacements.lock().unwrap() = Some(state);
    }
    pub fn set_launch_integrations(
        &self,
        hub: crate::Handle,
        preparation: Arc<crate::plugins::launch::Preparation>,
    ) {
        *self.launch_integrations.lock().unwrap() = Some((hub, preparation));
    }
    pub async fn spawn_for(
        self: &Arc<Self>,
        caller: crate::Caller,
        params: Value,
    ) -> Result<Value> {
        self.spawn_admitted(params, Some(caller), None, None).await
    }
    /// Only the lease receiver can construct this non-cloneable admission.
    /// A failed/unknown execution never makes its durable lease reusable.
    pub async fn spawn_remote(
        self: &Arc<Self>,
        caller: crate::Caller,
        admission: super::remote_dispatch::RemoteAdmission,
        input: Value,
    ) -> Result<Value> {
        if !input.is_object()
            || input["provider"] != admission.provider()
            || input["cwd"] != admission.cwd()
        {
            bail!("remote launch differs from its consumed lease");
        }
        let mut params = json!({"provider":admission.provider(),"cwd":admission.cwd(),"transport":"stream","manager":false,"trackTask":false,"toolScope":"operator"});
        // Remote provenance is deliberately not a local parent authority. Shared
        // caller sanitization has already attenuated model and permission fields.
        for key in [
            "message",
            "label",
            "model",
            "modelIdentity",
            "effort",
            "contextWindow",
            "exactModel",
            "permissionMode",
            "skipPermissions",
            "fullAccess",
            "role",
            "capability",
            "decisionId",
            "resultSchema",
            "escalationScrubbed",
        ] {
            if let Some(value) = input.get(key) {
                params[key] = value.clone();
            }
        }
        self.spawn_admitted(params, Some(caller), None, Some(admission))
            .await
    }
    pub fn launch_configuration(&self, options: &Value) -> Result<String> {
        let profile = self.profiles.get(text(options, "profileId"));
        let encoded = format!(
            "{{\"profile\":{},\"launchIntegrationId\":{}}}",
            serde_json::to_string(&profile)?,
            serde_json::to_string(options.get("launchIntegrationId").unwrap_or(&Value::Null))?
        );
        Ok(format!("{:x}", Sha256::digest(encoded.as_bytes())))
    }
    pub async fn spawn_replacement(
        self: &Arc<Self>,
        operation: &str,
        successor: &str,
        launch: Value,
    ) -> Result<Value> {
        let claim = ReplacementClaim {
            operation: operation.into(),
            session: successor.into(),
            launch,
        };
        self.validate_replacement(&claim)?;
        if self
            .lifecycle
            .records()
            .get(successor)
            .is_some_and(|record| record.engine_attempted)
        {
            bail!(
                "successor launch was already attempted; reconcile its outcome instead of launching again"
            );
        }
        let mut params = claim.launch["options"].clone();
        // Replacement is a reproduction of the recorded manager selection.
        // A changed ceiling must refuse it, never silently choose a substitute.
        params["exactModel"] = json!(true);
        self.spawn_admitted(params, None, Some(claim), None).await
    }
    fn validate_replacement(&self, claim: &ReplacementClaim) -> Result<()> {
        let state = self
            .replacements
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow!("manager replacement runtime unavailable"))?;
        let record = state.get(&claim.operation)?;
        let options = &claim.launch["options"];
        if record["phase"] != "spawning"
            || record["successorSessionId"] != claim.session
            || record["launch"] != claim.launch
            || options["manager"] != true
            || options["transport"] != "stream"
            || options["toolScope"] != "operator"
            || claim.launch["configuration"] != self.launch_configuration(options)?
            || claim.launch["grants"]
                != format!(
                    "{:x}",
                    Sha256::digest(b"{\"scope\":\"operator\",\"role\":\"manager\"}")
                )
        {
            bail!("manager successor does not match the authorized spawning operation");
        }
        for key in [
            "message",
            "resumeSessionId",
            "taskId",
            "workflowStepId",
            "afterDispatchId",
            "retrySourceSessionId",
            "launchIntegrationId",
        ] {
            if !text(options, key).is_empty() {
                bail!(
                    "replacement manager must start fresh without tasks, integration or first message"
                );
            }
        }
        if claim.session.is_empty() {
            bail!("replacement session identity required");
        }
        Ok(())
    }
    /// Own all admitted work, including checkout/setup, beyond RPC cancellation.
    pub async fn spawn_sanitized(self: &Arc<Self>, params: Value) -> Result<Value> {
        self.spawn_admitted(params, None, None, None).await
    }
    async fn spawn_admitted(
        self: &Arc<Self>,
        params: Value,
        caller: Option<crate::Caller>,
        replacement: Option<ReplacementClaim>,
        remote: Option<super::remote_dispatch::RemoteAdmission>,
    ) -> Result<Value> {
        {
            let accepting = self.accepting.lock().unwrap();
            if !*accepting {
                bail!("spawn coordinator is closing");
            }
            self.active.fetch_add(1, Ordering::AcqRel);
        }
        struct Active(Arc<SpawnCoordinator>);
        impl Drop for Active {
            fn drop(&mut self) {
                if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
                    self.0.idle.notify_waiters();
                }
            }
        }
        let active = Active(self.clone());
        tokio::spawn(async move {
            let result = active
                .0
                .spawn_owned(params, caller, replacement, remote)
                .await;
            drop(active);
            result
        })
        .await?
    }
    pub fn active_count(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
    pub async fn close(&self) {
        *self.accepting.lock().unwrap() = false;
        self.stopping.notify_waiters();
        self.lifecycle.request_close();
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
    async fn cancelled(&self) {
        let notified = self.stopping.notified();
        if !*self.accepting.lock().unwrap() {
            return;
        }
        notified.await;
    }
    async fn preparation<T>(
        &self,
        future: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::select! {
            biased;
            _=self.cancelled()=>bail!("spawn coordinator is closing"),
            result=tokio::time::timeout(std::time::Duration::from_secs(900),future)=>result.map_err(|_|anyhow!("spawn preparation timed out"))?,
        }
    }
    fn dispatch_owner(&self, params: &Value) -> Value {
        let parent = text(params, "parentSessionId");
        if !parent.is_empty() && text(params, "dispatchOwnerSessionId") == parent {
            self.workflow.owner_snapshot(parent).unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    }
    fn tracking_input(&self, params: &Value, project: &str) -> Value {
        let mut input = json!({"owner":self.dispatch_owner(params),"projectCwd":project});
        for key in [
            "trackTask",
            "taskId",
            "stage",
            "afterDispatchId",
            "retrySourceSessionId",
            "workflowStepId",
        ] {
            if let Some(value) = params.get(key) {
                input[key] = value.clone();
            }
        }
        input
    }
    fn routed(&self, params: &Value, config: &Value, id: &str, project: &str) -> Result<Plan> {
        let profile = self.profiles.get(text(params, "profileId"));
        let fleet = self
            .workflow
            .owner_snapshot(text(params, "parentSessionId"))
            .is_some_and(|p| p["isWakeTarget"] == true && p["status"] != "ended");
        let plan = spawn_plan::resolve(params, config, profile.as_ref(), &self.home, id, fleet)?;
        let mut resolved_params = effective(params, &plan, project);
        let mut scrubbed = self.routing.sanitize_spawn(&mut resolved_params)?;
        if scrubbed
            .iter()
            .any(|key| matches!(key.as_str(), "model" | "provider"))
        {
            for key in ["modelIdentity", "contextWindow"] {
                if resolved_params
                    .as_object_mut()
                    .unwrap()
                    .remove(key)
                    .is_some()
                {
                    scrubbed.push(key.into());
                }
            }
        }
        let mut resolved = if scrubbed.is_empty() {
            plan
        } else {
            spawn_plan::resolve(
                &resolved_params,
                config,
                profile.as_ref(),
                &self.home,
                id,
                fleet,
            )?
        };
        // A profile CLI pin may win again at resolution. Never silently launch
        // that model after a ceiling changed it at the named-field boundary.
        let mut final_effective = effective(&resolved_params, &resolved, project);
        if !self
            .routing
            .sanitize_spawn(&mut final_effective)?
            .is_empty()
        {
            bail!(
                "profile/provider arguments override the configured routing ceiling; no substitute was launched"
            );
        }
        if let Some(upstream) = params["escalationScrubbed"].as_array() {
            for field in upstream.iter().filter_map(Value::as_str) {
                if !scrubbed.iter().any(|key| key == field) {
                    scrubbed.push(field.into());
                }
            }
        }
        if !scrubbed.is_empty() {
            resolved.metadata["escalationScrubbed"] = json!(scrubbed);
        }
        Ok(resolved)
    }
    /// Replay only bookkeeping for positively acknowledged launches. Never
    /// repeat provider execution or infer first-message delivery from liveness.
    pub async fn recover_tracking(&self) -> Result<()> {
        let mut failures = Vec::new();
        for (session, record) in self.lifecycle.records() {
            let Some(mut input) = record.metadata.get("dispatchAdmission").cloned() else {
                continue;
            };
            let operation = text(&record.metadata, "launchOperationId");
            let token = text(&record.metadata, "workflowReservationToken");
            let task = text(&record.metadata, "taskId");
            let result = async {
                if record.receipt.is_none() {
                    if record.engine_attempted && !record.definitive_rejection {
                        return Ok(());
                    }
                    if !matches!(
                        record.phase,
                        super::agent_lifecycle::Phase::Stopped
                            | super::agent_lifecycle::Phase::Failed
                    ) {
                        return Ok(());
                    }
                } else {
                    let owner = text(&record.metadata, "parentSessionId");
                    input["owner"] = self.workflow.owner_snapshot(owner).unwrap_or(Value::Null);
                    let receipt = self.workflow.tasks.accept(input, || {
                        if !token.is_empty() {
                            task_store::manager(
                                &self.workflow.owner_snapshot(owner).unwrap_or(Value::Null),
                            )?;
                        }
                        Ok(())
                    })?;
                    if let Some(receipt) = receipt {
                        self.lifecycle
                            .bind_dispatch(&session, operation, &receipt)
                            .await?;
                    }
                }
                if !token.is_empty() {
                    self.workflow.tasks.release_workflow_dispatch(task, token)?;
                }
                self.lifecycle.finish_dispatch(&session, operation).await
            }
            .await;
            if let Err(error) = result {
                failures.push(format!("{session}: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            bail!(
                "launch bookkeeping recovery failed: {}",
                failures.join("; ")
            )
        }
    }
    async fn spawn_owned(
        &self,
        mut params: Value,
        caller: Option<crate::Caller>,
        replacement: Option<ReplacementClaim>,
        remote: Option<super::remote_dispatch::RemoteAdmission>,
    ) -> Result<Value> {
        let map = params
            .as_object_mut()
            .ok_or_else(|| anyhow!("spawn parameters must be an object"))?;
        // Internal claims originate only from this operation, never wire JSON.
        map.remove("workflowReservationToken");
        map.remove("projectCwd");
        let previous = self
            .lifecycle
            .records()
            .get(text(&params, "resumeSessionId"))
            .cloned();
        if let Some(previous) = &previous {
            let prior_integration = text(&previous.metadata["settings"], "launchIntegrationId");
            if !prior_integration.is_empty() && text(&params, "launchIntegrationId").is_empty() {
                bail!(
                    "resuming this session requires an explicit authenticated-owner launch integration selection; the previous proxy will not be silently dropped"
                );
            }
            for (key, value) in [
                ("provider", json!(previous.provider)),
                (
                    "profileId",
                    previous.metadata["settings"]["profileId"].clone(),
                ),
                (
                    "mcpItemIds",
                    previous.metadata["settings"]["mcpItemIds"].clone(),
                ),
                ("cwd", json!(previous.cwd)),
                ("transport", previous.metadata["transport"].clone()),
                ("label", previous.metadata["label"].clone()),
                (
                    "parentSessionId",
                    previous.metadata["parentSessionId"].clone(),
                ),
                ("manager", previous.metadata["isWakeTarget"].clone()),
                ("resultSchema", previous.metadata["resultSchema"].clone()),
            ] {
                if params.get(key).is_none_or(Value::is_null) && !value.is_null() {
                    params[key] = value;
                }
            }
            for key in ["role", "capability", "decisionId"] {
                if params.get(key).is_none_or(Value::is_null) {
                    if let Some(value) = previous.metadata["routing"].get(key) {
                        params[key] = value.clone();
                    }
                }
            }
        }
        let replacements = self.replacements.lock().unwrap().clone();
        let _lineage_guard = if let Some(state) = &replacements {
            state.assert_resume(text(&params, "resumeSessionId"))?;
            Some(state.admit(&[
                text(&params, "resumeSessionId"),
                text(&params, "dispatchOwnerSessionId"),
                text(&params, "parentSessionId"),
            ])?)
        } else {
            None
        };
        for key in ["remoteOrigin", "executionTarget", "targetHub"] {
            if params
                .get(key)
                .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
            {
                bail!("{key} requires its complete launch admission adapter");
            }
        }
        if params
            .get("launchIntegrationId")
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            bail!("invalid launch integration selection");
        }
        let integration = text(&params, "launchIntegrationId").to_owned();
        if !integration.is_empty()
            && (params["launchIntegrationGranted"] != true || caller.is_none())
        {
            bail!("launch integrations require an authenticated pending owner call");
        }
        for key in ["worktree", "trackTask"] {
            if params
                .get(key)
                .is_some_and(|v| !v.is_null() && !v.is_boolean())
            {
                bail!("{key} must be a boolean");
            }
        }
        if params
            .get("template")
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            bail!("template must be a string");
        }
        if params
            .get("templateParams")
            .is_some_and(|v| !v.is_null() && !v.is_object())
        {
            bail!("templateParams must be an object");
        }
        let requested =
            text(&params, "cwd").trim_matches([' ', '\t', '\r', '\n', '\u{b}', '\u{c}']);
        let project = paths::canonicalize(if requested.is_empty() {
            &self.home
        } else {
            std::path::Path::new(requested)
        })?;
        if !project.is_dir() {
            bail!("launch cwd must be an existing directory");
        }
        let project = project.to_string_lossy().into_owned();
        params["cwd"] = json!(project);
        let routing_project = remote
            .as_ref()
            .map(|r| r.repo_cwd().to_owned())
            .unwrap_or_else(|| {
                previous
                    .as_ref()
                    .filter(|prior| prior.cwd == project)
                    .and_then(|prior| prior.metadata["projectCwd"].as_str())
                    .unwrap_or(&project)
                    .to_owned()
            });
        if let Some(remote) = &remote {
            if project != remote.cwd()
                || paths::canonicalize(std::path::Path::new(remote.repo_cwd()))?.to_string_lossy()
                    != remote.repo_cwd()
            {
                bail!("leased remote directory changed before launch");
            }
        }
        let config = self.config.get();
        let operation = uuid::Uuid::new_v4().to_string();
        let id = if let Some(remote) = &remote {
            remote.session_id().to_owned()
        } else if let Some(claim) = &replacement {
            self.validate_replacement(claim)?;
            claim.session.clone()
        } else if text(&params, "resumeSessionId").is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            text(&params, "resumeSessionId").into()
        };
        self.workflow
            .tasks
            .validate_admission(&self.tracking_input(&params, &project))?;
        // Validate provider/profile/model before any checkout or reservation.
        let preliminary = self.routed(&params, &config, &id, &routing_project)?;
        if let Some(remote) = &remote {
            if preliminary.provider != remote.provider() {
                bail!("routing cannot replace the leased remote provider");
            }
        }
        let integrations = self.launch_integrations.lock().unwrap().clone();
        let _integration_lease = if !integration.is_empty() {
            let (hub, preparation) = integrations
                .as_ref()
                .ok_or_else(|| anyhow!("launch integration runtime unavailable"))?;
            let permit = self
                .preparation(hub.begin_launch_preparation(
                    caller.as_ref().unwrap(),
                    id.clone(),
                    integration.clone(),
                ))
                .await?;
            Some(preparation.admit(permit)?)
        } else {
            None
        };
        let mut admission: Option<Admission> = None;
        let mut template = None;
        if !text(&params, "workflowStepId").is_empty() {
            let owner = text(&params, "dispatchOwnerSessionId").to_owned();
            let claim = self.workflow.admit(&params, &owner)?;
            template = Some(claim.template.clone());
            params = claim.params.clone();
            admission = Some(claim);
        } else if !text(&params, "template").is_empty() {
            if !text(&params, "message").trim().is_empty() {
                bail!("agents.spawn: pass template OR message, not both");
            }
            let items = self
                .library
                .list(&json!({"cwd":project,"id":params["template"]}))?;
            let item = items
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["scope"] != "claude")
                .ok_or_else(|| {
                    anyhow!("agents.spawn: requested library template is unavailable")
                })?;
            if item["kind"] != "dispatch" {
                bail!("only dispatch templates render into a spawn");
            }
            dispatch_templates::render(text(item, "body"), &params["templateParams"], "", "")?;
            if !params.as_object().unwrap().contains_key("resultSchema") {
                if let Some(schema) = item.get("resultSchema") {
                    params["resultSchema"] = schema.clone();
                }
            }
            template = Some(item.clone());
        } else if params["templateParams"]
            .as_object()
            .is_some_and(|p| !p.is_empty())
        {
            bail!("templateParams was passed without a template to fill");
        }
        let result=async {
            if let Some(template)=&template{if admission.is_some()&&!template["resultSchema"].is_null(){params["resultSchema"]=template["resultSchema"].clone();}}
            if let Some(schema)=params.get("resultSchema").filter(|v|!v.is_null()){super::worker_results::check_schema(schema).map_err(|error|anyhow!(error))?;}
            let mut reservation:Option<Reservation>=None;let mut worktree=None;let mut review_allocation=None;let mut execution=project.clone();
            if params["worktree"]==true{
                let created=self.preparation(self.worktrees.create_reserved(json!({"repoCwd":project,"name":params["label"]}))).await;
                if !*self.accepting.lock().unwrap(){bail!("spawn coordinator is closing");}
                let result=match created{Ok(created)=>{reservation=created.reservation;created.result},Err(error)=>json!({"ok":false,"error":error.to_string()})};
                if result["ok"]==true{execution=text(&result,"path").into();review_allocation=result.get("reviewAllocation").cloned();}
                let mut metadata=json!({"projectCwd":project,"executionCwd":execution,"requested":true,"allocated":result["ok"]==true,"fallback":result["ok"]!=true});
                for key in ["branch","error","setup"]{if let Some(value)=result.get(key){metadata[key]=value.clone();}}
                if admission.is_some()&&result["ok"]!=true{bail!("Workflow ship step requires successful worktree allocation");}
                worktree=Some(metadata);
            }
            if let Some(template)=&template{params["message"]=json!(dispatch_templates::render(text(template,"body"),&params["templateParams"],&execution,&project)?);}
            // Recheck effective selection after slow setup; ceilings apply to the
            // original project, not the allocated path outside that project's tree.
            let mut plan=self.routed(&params,&config,&id,&routing_project)?;plan.request["cwd"]=json!(execution);
            if let Some(remote)=&remote {
                if plan.session_id != remote.session_id() || plan.provider != remote.provider() || execution != remote.cwd() {bail!("resolved launch differs from the consumed remote lease");}
                plan.metadata["remoteOrigin"]=remote.origin();
                plan.metadata["projectCwd"]=json!(remote.repo_cwd());
                let mut instructions=plan.request["instructions"].as_str().unwrap_or("").to_owned();
                if !instructions.contains(super::worker_results::ESCALATION_CONTRACT) {instructions.push_str("\n\n");instructions.push_str(super::worker_results::ESCALATION_CONTRACT);}
                plan.request["instructions"]=json!(instructions);
                if remote.worktree() {worktree=Some(json!({"projectCwd":remote.repo_cwd(),"executionCwd":remote.cwd(),"requested":true,"allocated":true,"fallback":false,"branch":remote.branch()}));}
            }
            if let Some(previous)=&previous {
                for key in ["taskId","dispatchId","workflowStepId","afterDispatchId","stage"] {
                    if plan.metadata.get(key).is_none() {if let Some(value)=previous.metadata.get(key){plan.metadata[key]=value.clone();}}
                }
                if plan.metadata["settings"].get("model").is_none() {if let Some(model)=previous.metadata["settings"].get("model"){plan.metadata["settings"]["model"]=model.clone();}}
            }
            plan.metadata["transport"]=json!(if plan.endpoint=="/sessions/spawn" {"pty"}else{plan.request["transport"].as_str().unwrap_or("stream")});
            let mut tracking=self.tracking_input(&params,&project);
            tracking["sessionId"]=json!(id);tracking["title"]=params["label"].clone();tracking["executionCwd"]=json!(execution);tracking["requestedProvider"]=params["provider"].clone();tracking["provider"]=json!(plan.provider);tracking["requestedModel"]=plan.request["model_identity"].clone();tracking["role"]=params["role"].clone();
            if let Some(worktree)=&worktree{tracking["worktree"]=worktree.clone();}
            if let Some(admission)=&admission{tracking["workflowReservationToken"]=json!(admission.token);}
            if text(&params,"resumeSessionId").is_empty(){plan.metadata["dispatchAdmission"]=tracking.clone();}
            if !integration.is_empty() {
                plan.metadata["settings"]["launchIntegrationId"]=json!(integration);
                plan.metadata["launchIntegrationResume"]=json!(!text(&params,"resumeSessionId").is_empty());
            }
            plan.metadata["launchOperationId"]=json!(operation);
            if let Some(admission)=&admission { plan.metadata["workflowReservationToken"]=json!(admission.token); }
            plan.metadata["projectCwd"]=remote.as_ref().map(|r|json!(r.repo_cwd())).or_else(||previous.as_ref().and_then(|prior|prior.metadata.get("projectCwd")).cloned()).unwrap_or(json!(project));if let Some(worktree)=&worktree{plan.metadata["worktree"]=worktree.clone();}
            if let Some(claim)=&replacement {self.validate_replacement(claim)?;}
            if let Some(state)=&replacements {
                let mut metadata=plan.metadata.clone();metadata["sessionId"]=json!(id);metadata["cwd"]=json!(execution);metadata["provider"]=json!(plan.provider);
                state.remember_child(metadata)?;
            }
            if let Some(allocation)=&review_allocation {
                plan.metadata["reviewAllocation"]=allocation.clone();
                let review={self.review.lock().unwrap().clone()};
                if let Some(review)=review {let owner=text(&plan.metadata,"parentSessionId");if !owner.is_empty(){
                    if let Err(error)=review.register(owner,&id,allocation.clone()){eprintln!("review allocation could not be retained: {error}");}
                }}
            }
            let receipt=if let Some(reservation)=reservation{self.lifecycle.launch_reserved(plan.clone(),reservation).await}else{self.lifecycle.launch(plan.clone()).await};
            let mut receipt=receipt?;
            if params["manager"]==true {
                if let Some(state)=&replacements {
                    let mut options=json!({"manager":true,"toolScope":"operator","provider":plan.provider,"transport":plan.metadata["transport"],"cwd":execution,"permissionMode":plan.metadata["settings"]["permissionMode"],"skipPermissions":plan.full_access});
                    for key in ["label","parentSessionId","profileId","mcpItemIds","launchIntegrationId","contextWindow"] {if let Some(value)=params.get(key){options[key]=value.clone();}}
                    for (target,source) in [("model","model_identity"),("contextWindow","context_window"),("effort","effort")] {if let Some(value)=plan.request.get(source){options[target]=value.clone();}}
                    if options.get("model").is_none(){if let Some(value)=plan.request.get("model"){options["model"]=value.clone();}}
                    let launch=json!({"configuration":self.launch_configuration(&options)?,"options":options,"grants":format!("{:x}",Sha256::digest(b"{\"scope\":\"operator\",\"role\":\"manager\"}"))});
                    if let Err(error)=state.remember_launch(&id,launch){receipt["historyError"]=json!(error.to_string());}
                }
            }
            if let Some(worktree)=worktree{receipt["worktree"]=worktree;}
            if template.is_some(){let message=text(&params,"message");let (echo,truncated)=truncate(message,16_000);receipt["renderedMessage"]=json!(echo);if truncated{receipt["renderedMessageTruncated"]=json!(true);}}
            if params["trackTask"]==false{receipt["taskTracking"]=json!(false);}
            else if text(&params,"resumeSessionId").is_empty(){
                let mut input=tracking;
                input["owner"]=self.dispatch_owner(&params);
                let accepted=self.workflow.tasks.accept(input,||{
                    let owner=self.dispatch_owner(&params);
                    if admission.is_some(){task_store::manager(&owner)?;}Ok(())
                });
                match accepted{
                    Ok(Some(ids))=>{
                        for(key,value)in ids.as_object().unwrap(){receipt[key]=value.clone();}
                        if let Err(error)=self.lifecycle.bind_dispatch(&id,&operation,&ids).await {
                            receipt["dispatchMetadataUnavailable"]=json!(true);
                            receipt["dispatchHistoryError"]=json!(error.to_string());
                        }
                    },
                    Ok(None)=>(),
                    Err(error)=>{
                        // Provider acceptance is irreversible. Preserve the receipt
                        // and uncertainty instead of reporting a failed spawn that
                        // would encourage the caller to launch a duplicate worker.
                        receipt["dispatchHistoryUnavailable"]=json!(true);receipt["dispatchHistoryError"]=json!(error.to_string());
                        if admission.is_some(){receipt["workflowAdmissionPending"]=json!(true);}return Ok(receipt);
                    }
                }
            }
            if let Some(admission)=&admission{if let Err(error)=self.workflow.tasks.release_workflow_dispatch(&admission.task_id,&admission.token){receipt["workflowAdmissionPending"]=json!(true);receipt["dispatchHistoryError"]=json!(error.to_string());}}
            if receipt["workflowAdmissionPending"]!=true && receipt["dispatchMetadataUnavailable"]!=true {
                if let Err(error)=self.lifecycle.finish_dispatch(&id,&operation).await {receipt["dispatchMetadataUnavailable"]=json!(true);receipt["dispatchHistoryError"]=json!(error.to_string());}
            }
            Ok(receipt)
        }.await;
        if result.is_err() {
            if let Some(admission) = &admission {
                let attempted = self
                    .lifecycle
                    .records()
                    .get(&id)
                    .is_some_and(|r| r.engine_attempted && !r.definitive_rejection);
                if !attempted {
                    if let Some(state) = &replacements {
                        let _ = state.forget_unclaimed_metadata(&id);
                    }
                    if let Err(error) = self
                        .workflow
                        .tasks
                        .release_workflow_dispatch(&admission.task_id, &admission.token)
                    {
                        return Err(anyhow!(
                            "{}; admission cleanup failed: {error}",
                            result.unwrap_err()
                        ));
                    }
                } else {
                    return Err(anyhow!(
                        "{}; launch admission may have executed for session {id}; reservation retained, do not retry blindly",
                        result.unwrap_err()
                    ));
                }
            }
        }
        result
    }
}
fn effective(params: &Value, plan: &Plan, project: &str) -> Value {
    let mut output = params.clone();
    output["provider"] = json!(plan.provider);
    output["cwd"] = json!(project);
    for (target, source) in [
        ("model", "model"),
        ("modelIdentity", "model_identity"),
        ("contextWindow", "context_window"),
        ("effort", "effort"),
    ] {
        if let Some(value) = plan.request.get(source) {
            output[target] = value.clone();
        } else {
            output.as_object_mut().unwrap().remove(target);
        }
    }
    if plan.endpoint == "/sessions/spawn" {
        if let Some(argv) = plan.request["argv"].as_array() {
            for (index, arg) in argv.iter().enumerate() {
                if arg == "--effort" {
                    if let Some(value) = argv.get(index + 1).filter(|v| v.is_string()) {
                        output["effort"] = value.clone();
                    }
                } else if let Some(value) = arg.as_str().and_then(|s| s.strip_prefix("--effort=")) {
                    output["effort"] = json!(value);
                }
            }
        }
    }
    output
}
fn truncate(value: &str, max: usize) -> (String, bool) {
    let mut units = 0;
    let mut out = String::new();
    for c in value.chars() {
        let len = c.len_utf16();
        if units + len > max {
            return (out, true);
        }
        out.push(c);
        units += len;
    }
    (out, false)
}

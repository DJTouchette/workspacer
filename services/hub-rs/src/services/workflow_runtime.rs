//! Manager-owned workflow interpretation and durable dispatch reservation.
use super::{
    dispatch_templates,
    manager_requests::ManagerRequests,
    task_store::{self, History, OwnerLookup, TaskStore},
    workflows::WorkflowStore,
};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn pending(task: &Value) -> Option<usize> {
    task["workflow"]["steps"]
        .as_array()?
        .iter()
        .position(|r| !task_store::terminal(&r["state"]))
}
fn evidence(task: &Value, index: usize) -> Option<String> {
    let pin = &task["workflow"];
    let step = &pin["definition"]["steps"][index];
    for key in ["independentOf", "repairOf"] {
        let id = text(step, key);
        if id.is_empty() {
            continue;
        }
        let run = pin["steps"].as_array()?.iter().find(|r| r["id"] == id);
        let found = run.is_some_and(|run| {
            run.get("outcome").is_some()
                && task["attempts"].as_array().is_some_and(|a| {
                    a.iter().any(|a| {
                        a["dispatchId"] == run["dispatchId"]
                            && a["sessionId"] == run["sessionId"]
                            && a["workflowStepId"] == id
                            && a["resultContract"] == "valid"
                    })
                })
        });
        if !found {
            return Some(format!(
                "Required evidence from step {id} is unavailable for {}",
                text(step, "label")
            ));
        }
    }
    None
}
pub fn dispatch_plan(task: &Value, tasks: &[Value]) -> Option<Value> {
    if !task["workflow"].is_object()
        || task.get("dispatchReservation").is_some()
        || task_store::dependency_state(task, tasks) != "ready"
    {
        return None;
    }
    let index = pending(task)?;
    let pin = &task["workflow"];
    let run = &pin["steps"][index];
    let step = &pin["definition"]["steps"][index];
    if run["state"] != "planned"
        || ((step["when"] == "material_risk" || !text(step, "repairOf").is_empty())
            && run["decision"] != true)
        || evidence(task, index).is_some()
    {
        return None;
    }
    let previous = pin["steps"]
        .as_array()?
        .iter()
        .take(index)
        .rev()
        .find(|r| r.get("dispatchId").is_some());
    let independent = if !text(step, "independentOf").is_empty() {
        pin["steps"]
            .as_array()?
            .iter()
            .find(|r| r["id"] == step["independentOf"])
    } else if step["kind"] == "review" {
        previous
    } else {
        None
    };
    let provider = independent
        .and_then(|r| {
            task["attempts"]
                .as_array()?
                .iter()
                .find(|a| a["dispatchId"] == r["dispatchId"])
        })
        .and_then(|a| a["provider"].as_str());
    let mut plan = json!({"taskId":task["taskId"],"cwd":task["projectCwd"],"stepId":step["id"],"expectedTaskRevision":task_store::revision(task),"role":step["role"],"stage":step["stage"],"template":step["template"],"params":pin["templates"][text(step,"template")]["params"],"toolScope":"operator"});
    if let Some(previous) = previous {
        plan["afterDispatchId"] = previous["dispatchId"].clone();
    }
    if let Some(provider) = provider {
        plan["previousProvider"] = json!(provider);
    }
    Some(plan)
}
pub fn instructions(task: &Value, tasks: &[Value]) -> String {
    let id = text(task, "taskId");
    let cwd = text(task, "projectCwd");
    let state = task_store::dependency_state(task, tasks);
    if state != "ready" {
        return format!(
            "Task {id} is {state}. Await explicit accepted dependency evidence; do not dispatch or bypass its pinned workflow."
        );
    }
    if !task["workflow"].is_object() {
        return format!(
            "Task {id} has no pinned workflow. Dispatch with spawn_agent using taskId={id}, cwd={cwd}, parentSessionId={} and the task-specific message/role. Honor explicit model choices; do not invent mandatory workflow steps or review.",
            text(task, "ownerSessionId")
        );
    }
    let pin = &task["workflow"];
    let definition = &pin["definition"];
    let policy = if definition["steps"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s["kind"] == "review"))
    {
        "Independent review required"
    } else {
        "Independent review omitted by selected policy"
    };
    let header = format!(
        "Fleet workflow {} ({}@{}, snapshot {}). {policy}. Task {id}, project {cwd}. Host-user waivers are explicit skips, never passing results.",
        text(definition, "name"),
        text(definition, "id"),
        definition["revision"],
        text(pin, "hash")
    );
    if task.get("dispatchReservation").is_some() {
        return format!(
            "{header}\nStep {} is being dispatched. Its host must finish or recover that dispatch before launching another worker.",
            text(&task["dispatchReservation"], "stepId")
        );
    }
    let Some(index) = pending(task) else {
        return format!(
            "{header}\nAll configured steps returned valid result contracts or explicit skips. This is NOT a passing verdict: inspect each reported outcome before accepting the task."
        );
    };
    let run = &pin["steps"][index];
    let step = &definition["steps"][index];
    if run["state"] != "planned" {
        return format!(
            "{header}\nStep {} is {}. Do not infer completion from idle/ended or launch another step. Escalate failed/blocked results; no automatic retries. {}",
            text(step, "id"),
            text(run, "state"),
            text(run, "reason")
        );
    }
    if (step["when"] == "material_risk" || !text(step, "repairOf").is_empty())
        && run.get("decision").is_none()
    {
        return format!(
            "{header}\nConditional decision required. Call dispatch_workflow_step with taskId={id}, cwd={cwd}, stepId={}, expectedTaskRevision={}, run=true/false and a concrete reason. run=false records only the skip. Fill templateParams when running.",
            text(step, "id"),
            task_store::revision(task)
        );
    }
    if let Some(missing) = evidence(task, index) {
        return format!("{header}\n{missing}. Do not dispatch or invent artifacts.");
    }
    format!(
        "{header}\nNext: {}. Call dispatch_workflow_step with taskId={id}, cwd={cwd}, stepId={}, expectedTaskRevision={} and templateParams {}. Honor explicit modelSelection; otherwise use automatic routing. Preserve isolation and delivery policy. {}\nStep instructions: {}\nAfter dispatch end your turn; the host wakes you. No polling or automatic launches.",
        text(step, "label"),
        text(step, "id"),
        task_store::revision(task),
        pin["templates"][text(step, "template")]["params"],
        if step["kind"] == "review" {
            "Give a fresh reviewer criteria, diff and closing handoff only; never the implementer's reasoning or transcript."
        } else {
            ""
        },
        text(step, "instructions")
    )
}
pub struct WorkflowRuntime {
    pub definitions: Arc<WorkflowStore>,
    pub tasks: Arc<TaskStore>,
    pub requests: Arc<ManagerRequests>,
    owner: OwnerLookup,
    replacements: Arc<Mutex<Option<Arc<super::manager_replacements::ReplacementState>>>>,
}
pub struct Admission {
    pub task_id: String,
    pub token: String,
    pub params: Value,
    pub template: Value,
}
impl Admission {
    pub fn render(&self, execution_cwd: &str) -> Result<Value> {
        let mut params = self.params.clone();
        params["message"] = json!(dispatch_templates::render(
            text(&self.template, "body"),
            &params["templateParams"],
            execution_cwd,
            text(&params, "cwd")
        )?);
        params["resultSchema"] = self.template["resultSchema"].clone();
        params["projectCwd"] = params["cwd"].clone();
        params["cwd"] = json!(execution_cwd);
        Ok(params)
    }
}
impl WorkflowRuntime {
    pub fn new(definitions: Arc<WorkflowStore>, tasks: Arc<TaskStore>, owner: OwnerLookup) -> Self {
        let defs = definitions.clone();
        let pin = Arc::new(move |cwd: &str, id: Option<&str>| defs.pin_for(cwd, id));
        let replacements: Arc<Mutex<Option<Arc<super::manager_replacements::ReplacementState>>>> =
            Arc::new(Mutex::new(None));
        let gate = replacements.clone();
        let raw_owner = owner.clone();
        let request_owner: OwnerLookup = Arc::new(move |id| {
            if gate
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|state| state.assert_available(id).is_err())
            {
                return None;
            }
            raw_owner(id)
        });
        let requests = Arc::new(ManagerRequests::new(tasks.clone(), request_owner, pin));
        Self {
            definitions,
            tasks,
            requests,
            owner,
            replacements,
        }
    }
    pub fn set_replacements(&self, state: Arc<super::manager_replacements::ReplacementState>) {
        self.requests.set_replacements(state.clone());
        *self.replacements.lock().unwrap() = Some(state);
    }
    pub fn owner_snapshot(&self, session: &str) -> Option<Value> {
        (self.owner)(session)
    }
    fn manager(&self, caller: &str) -> Result<Value> {
        if let Some(state) = self.replacements.lock().unwrap().as_ref() {
            state.assert_available(caller)?;
        }
        let owner = (self.owner)(caller)
            .ok_or_else(|| anyhow!("Workflow requires a live local manager"))?;
        task_store::manager(&owner)?;
        Ok(owner)
    }
    fn owned<'a>(
        &self,
        history: &'a History,
        id: &str,
        caller: &str,
        cwd: &str,
        workflow: bool,
    ) -> Result<&'a Value> {
        self.manager(caller)?;
        let task = history.owned(id, caller, cwd)?;
        if workflow && !task["workflow"].is_object() {
            bail!("Workflow task unavailable or belongs to another manager/project");
        }
        Ok(task)
    }
    fn task(&self, id: &str, caller: &str, cwd: &str, workflow: bool) -> Result<Value> {
        let history = self.tasks.snapshot()?;
        Ok(self.owned(&history, id, caller, cwd, workflow)?.clone())
    }
    fn projection(&self, task: Value) -> Result<Value> {
        let tasks = self.tasks.list()?;
        let mut out = json!({"ok":true,"instructions":instructions(&task,&tasks),"task":task});
        if let Some(dispatch) = dispatch_plan(&out["task"], &tasks) {
            out["dispatch"] = dispatch;
        }
        Ok(out)
    }
    /// The desktop capability wrapper resolves both project selectors before
    /// invoking the shared workflow service. Preserve that boundary in-process.
    /// Read-only exact-record lookups need no filesystem: archived projects may
    /// be offline, and this reads manager-owned history rather than project data.
    fn normalize_request(
        &self,
        input: &Value,
        caller: &str,
        historical_reads: bool,
    ) -> Result<Value> {
        if !input.is_object() || serde_json::to_vec(input)?.len() > 100 * 1024 {
            bail!("Invalid or oversized workflow request");
        }
        let mut request = input.clone();
        let preserve = historical_reads
            && matches!(text(input, "op"), "next" | "taskReferences")
            && self.tasks.task(text(input, "taskId"))?.is_some_and(|task| {
                task["ownerSessionId"] == caller && task["projectCwd"] == input["cwd"]
            });
        fn normalize(value: &mut Value) -> Result<()> {
            if value.is_null() || value.as_str() == Some("") {
                return Ok(());
            }
            let cwd = value
                .as_str()
                .ok_or_else(|| anyhow!("Workflow project cwd must be absolute"))?;
            *value = json!(
                super::paths::canonicalize(std::path::Path::new(cwd))?
                    .to_string_lossy()
                    .into_owned()
            );
            Ok(())
        }
        if !preserve && let Some(cwd) = request.get_mut("cwd") {
            normalize(cwd)?;
        }
        if let Some(intents) = request.get_mut("intents").and_then(Value::as_array_mut) {
            for intent in intents {
                if let Some(cwd) = intent.get_mut("cwd") {
                    normalize(cwd)?;
                }
            }
        }
        Ok(request)
    }
    pub fn request(&self, request: &Value, caller: &str) -> Value {
        let request = match self.normalize_request(request, caller, true) {
            Ok(request) => request,
            Err(error) => return json!({"ok":false,"code":"unavailable","error":error.to_string()}),
        };
        let request = &request;
        let op = text(request, "op");
        if let Some(state) = self.replacements.lock().unwrap().as_ref() {
            if !caller.is_empty() {
                if let Err(error) = state.assert_available(caller) {
                    return json!({"ok":false,"code":"unavailable","error":error.to_string()});
                }
                if op == "resolveRequest" && state.active_count(caller) > 0 {
                    return json!({"ok":false,"code":"unavailable","error":"A dispatch is being admitted; resolve the inbox request after it settles"});
                }
            }
        }
        if matches!(
            op,
            "list"
                | "get"
                | "validate"
                | "create"
                | "update"
                | "clone"
                | "disable"
                | "delete"
                | "select"
        ) {
            return self.definitions.request(request);
        }
        if matches!(
            op,
            "requestInbox" | "requestContent" | "resolveRequest" | "acceptTaskOutcome"
        ) {
            let mut response = self.requests.handle(request.clone(), caller);
            if response["ok"] == true && matches!(op, "resolveRequest" | "acceptTaskOutcome") {
                let ids = response[if op == "resolveRequest" {
                    "tasks"
                } else {
                    "readyTasks"
                }]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t["taskId"].as_str())
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>();
                response["nextActionsRemaining"] = json!(ids.len().saturating_sub(4));
                response["nextActions"]=json!(ids.into_iter().take(4).map(|id|{
                    let task=self.tasks.task(&id).ok().flatten().filter(|t|t["ownerSessionId"]==caller);match task.and_then(|t|{let tasks=self.tasks.list().ok()?;let mut action=json!({"taskId":id,"cwd":t["projectCwd"],"revision":task_store::revision(&t),"instructions":instructions(&t,&tasks)});if let Some(dispatch)=dispatch_plan(&t,&tasks){action["dispatch"]=dispatch;}Some(action)}){Some(action)=>action,None=>json!({"taskId":id,"unavailable":true})}
                }).collect::<Vec<_>>());
            }
            return response;
        }
        let result = (|| -> Result<Value> {
            if !request.is_object() || serde_json::to_vec(request)?.len() > 100 * 1024 {
                bail!("Invalid or oversized workflow request");
            }
            let cwd = text(request, "cwd");
            if !std::path::Path::new(cwd).is_absolute() {
                bail!("Workflow project cwd must be absolute");
            }
            let id = text(request, "taskId");
            if op == "start" {
                let owner = self.manager(caller)?;
                if dispatch_templates::trim_js(text(request, "title")).is_empty() || !id.is_empty()
                {
                    bail!("New workflow requires a live local manager, project cwd and title");
                }
                let pin = self.definitions.pin_for(cwd, None)?;
                return self.projection(self.tasks.start_workflow(
                    &owner,
                    cwd,
                    text(request, "title"),
                    pin,
                )?);
            }
            if matches!(op, "taskReferences" | "setTaskReferences") {
                let mut task = self.task(id, caller, cwd, false)?;
                if op == "setTaskReferences" {
                    let expected = request["expectedTaskRevision"].as_u64().ok_or_else(|| {
                        anyhow!("Task reference updates require expectedTaskRevision")
                    })?;
                    task = self.tasks.update_references(
                        id,
                        caller,
                        cwd,
                        expected,
                        request.get("upsert"),
                        request.get("remove"),
                        || {
                            self.manager(caller)?;
                            Ok(())
                        },
                    )?;
                }
                return Ok(
                    json!({"ok":true,"references":task.get("links").cloned().unwrap_or(json!({})),"taskRevision":task_store::revision(&task),"task":task}),
                );
            }
            if matches!(op, "next" | "decide" | "prepareDispatch") {
                if op != "next" {
                    self.tasks.transaction(|history| {
                        let task = self.owned(history, id, caller, cwd, true)?;
                        if task.get("dispatchReservation").is_some() {
                            bail!("Step dispatch in progress");
                        }
                        if op == "prepareDispatch" {
                            let expected = request["expectedTaskRevision"]
                                .as_u64()
                                .ok_or_else(|| anyhow!("Dispatch requires expectedTaskRevision"))?;
                            if task_store::revision(task) != expected {
                                bail!("conflict: task changed");
                            }
                            if task_store::dependency_state(task, &history.tasks) != "ready" {
                                bail!("Task dependencies or cancellation prevent dispatch");
                            }
                            let index = pending(task)
                                .ok_or_else(|| anyhow!("Workflow has no pending step"))?;
                            if task["workflow"]["steps"][index]["state"] != "planned"
                                || task["workflow"]["steps"][index]["id"] != request["stepId"]
                            {
                                bail!("Requested workflow step is not available for dispatch");
                            }
                            if request["run"] != false {
                                let step = &task["workflow"]["definition"]["steps"][index];
                                dispatch_templates::render(
                                    text(
                                        &task["workflow"]["templates"][text(step, "template")],
                                        "body",
                                    ),
                                    &request["templateParams"],
                                    "",
                                    "",
                                )?;
                            }
                        }
                        if op == "decide" || request.get("run").is_some() {
                            decision(
                                history.task_mut(id)?,
                                text(request, "stepId"),
                                request["run"]
                                    .as_bool()
                                    .ok_or_else(|| anyhow!("Decision requires run"))?,
                                text(request, "reason"),
                            )?;
                        }
                        Ok(())
                    })?;
                }
                let task = self.task(id, caller, cwd, op != "next")?;
                let mut response = self.projection(task)?;
                if op == "prepareDispatch" {
                    response["taskRevision"] = json!(task_store::revision(&response["task"]));
                    if request["run"] == false {
                        response.as_object_mut().unwrap().remove("dispatch");
                        response["skipped"] = json!(true);
                    } else if response.get("dispatch").is_none() {
                        return Ok(
                            json!({"ok":false,"code":"ineligible","error":response["instructions"]}),
                        );
                    }
                    response.as_object_mut().unwrap().remove("task");
                }
                return Ok(response);
            }
            bail!("Unknown workflow operation")
        })();
        match result {
            Ok(value) => value,
            Err(error) => {
                let conflict = error.to_string().starts_with("conflict:");
                let mut response = json!({"ok":false,"code":if conflict{"conflict"}else{"unavailable"},"error":error.to_string()});
                if conflict {
                    if let Ok(task) =
                        self.task(text(request, "taskId"), caller, text(request, "cwd"), false)
                    {
                        response["currentRevision"] = json!(task_store::revision(&task));
                        response["references"] = task.get("links").cloned().unwrap_or(json!({}));
                    }
                }
                response
            }
        }
    }
    /// Reserve under the task transaction BEFORE any worktree/token/provider IO.
    /// A process crash retains the reservation as uncertainty, not permission to
    /// retry. Only the matching token can finish or explicitly release it.
    pub fn admit(&self, params: &Value, caller: &str) -> Result<Admission> {
        let params = self.normalize_request(params, caller, false)?;
        let params = &params;
        let id = text(params, "taskId");
        let cwd = text(params, "cwd");
        let mut output = params.clone();
        let token = uuid::Uuid::new_v4().to_string();
        let mut template = Value::Null;
        self.tasks.transaction(|history| {
            let task = self.owned(history, id, caller, cwd, true)?;
            let plan = dispatch_plan(task, &history.tasks)
                .ok_or_else(|| anyhow!("Workflow step is not eligible; call next_workflow_step"))?;
            if params.get("expectedTaskRevision").is_some_and(|revision| {
                revision.as_u64() != Some(task_store::revision(task))
            }) {
                bail!("conflict: task changed before dispatch");
            }
            let fresh = params["manager"] != true
                && text(params, "resumeSessionId").is_empty()
                && text(params, "retrySourceSessionId").is_empty();
            let lineage_matches = params["parentSessionId"] == task["ownerSessionId"]
                && params["workflowStepId"] == plan["stepId"]
                && params["stage"] == plan["stage"]
                && params["role"] == plan["role"]
                && params["template"] == plan["template"]
                && params.get("afterDispatchId") == plan.get("afterDispatchId");
            let selected = !text(params, "provider").is_empty()
                && ["decisionId", "model", "modelIdentity"].iter().any(|key| !text(params,key).is_empty());
            let no_overrides = text(params, "message").is_empty()
                && params.get("resultSchema").is_none_or(Value::is_null);
            if !fresh || !lineage_matches || !selected || !no_overrides {
                bail!("Workflow dispatch requires exact next-step metadata, a routed or explicit model, and a fresh session; message/schema overrides are not accepted");
            }
            let index = pending(task).unwrap();
            let step = &task["workflow"]["definition"]["steps"][index];
            template = task["workflow"]["templates"][text(step,"template")].clone();
            let mut values = params.get("templateParams").filter(|v| !v.is_null()).cloned().unwrap_or(json!({}));
            dispatch_templates::render(text(&template,"body"), &values, "", "")?;
            if !text(step,"instructions").is_empty() {
                if !template["params"].as_array().is_some_and(|p| p.iter().any(|p| p["name"] == "task")) {
                    bail!("Step instructions require a template task input");
                }
                values["task"] = json!(format!("{}\n\n{}",text(&values,"task"),text(step,"instructions")));
            }
            if template["params"].as_array().is_some_and(|params| params.iter().any(|p| p["name"] == "delivery")) {
                values["delivery"] = json!(self.definitions.delivery(cwd)?);
            }
            output["dispatchOwnerSessionId"] = json!(caller);
            output["templateParams"] = values;
            output["toolScope"] = json!("operator");
            output["worktree"] = json!(!matches!(text(step,"kind"), "research" | "review" | "validate"));
            history.task_mut(id)?["dispatchReservation"] = json!({
                "stepId":params["workflowStepId"], "token":token, "createdAt":crate::protocol::now()
            });
            Ok(())
        })?;
        Ok(Admission {
            task_id: id.into(),
            token,
            params: output,
            template,
        })
    }
}
fn decision(task: &mut Value, id: &str, run: bool, reason: &str) -> Result<()> {
    let index = pending(task).ok_or_else(|| anyhow!("Workflow step unavailable"))?;
    let current = &task["workflow"]["steps"][index];
    let definition = &task["workflow"]["definition"]["steps"][index];
    if current["id"] != id || current["state"] != "planned" || current.get("decision").is_some() {
        bail!("Only the next planned conditional step accepts a decision");
    }
    if definition["when"] != "material_risk" && text(definition, "repairOf").is_empty() {
        bail!("Required step cannot be skipped");
    }
    if dispatch_templates::trim_js(reason).is_empty() || reason.encode_utf16().count() > 2000 {
        bail!("A decision requires a reason (at most 2000 characters)");
    }
    let current = &mut task["workflow"]["steps"][index];
    current["decision"] = json!(run);
    current["reason"] = json!(reason);
    if !run {
        current["state"] = json!("skipped");
    }
    Ok(())
}

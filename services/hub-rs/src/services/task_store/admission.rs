use super::*;
const STAGES: &[&str] = &[
    "scout",
    "implement",
    "review",
    "fix",
    "validate",
    "land",
    "other",
];
fn same_project(task: &Value, cwd: &Value) -> bool {
    same_cwd(text(&task["projectCwd"]), text(cwd))
        || task["attempts"].as_array().into_iter().flatten().any(|a| {
            same_cwd(text(&a["executionCwd"]), text(cwd)) && a["worktree"]["allocated"] == true
        })
}
fn retry_source(state: &History, input: &Value) -> Option<(String, Value)> {
    if text(&input["retrySourceSessionId"]).is_empty() || manager(&input["owner"]).is_err() {
        return None;
    }
    for task in &state.tasks {
        if task["ownerSessionId"] != input["owner"]["sessionId"]
            || !same_project(task, &input["projectCwd"])
            || input.get("taskId").is_some_and(|id| *id != task["taskId"])
            || task.get("workflow").is_some() && text(&input["workflowStepId"]).is_empty()
        {
            continue;
        }
        if let Some(attempt) = task["attempts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["sessionId"] == input["retrySourceSessionId"])
        {
            return Some((text(&task["taskId"]).into(), attempt.clone()));
        }
    }
    None
}
pub fn validate_admission(state: &History, input: &Value) -> Result<()> {
    if input.get("trackTask").is_some_and(|v| !v.is_boolean()) {
        bail!("trackTask must be boolean");
    }
    if input["trackTask"] == false {
        if [
            "taskId",
            "workflowStepId",
            "afterDispatchId",
            "retrySourceSessionId",
        ]
        .iter()
        .any(|k| !text(&input[*k]).is_empty())
        {
            bail!("Untracked dispatch must omit task, workflow and retry links");
        }
        return Ok(());
    }
    if input.get("stage").is_some()
        && !STAGES.contains(&text(&input["stage"]))
        && ["taskId", "workflowStepId", "afterDispatchId"]
            .iter()
            .any(|k| !text(&input[*k]).is_empty())
    {
        bail!("Unknown dispatch stage");
    }
    let owner = &input["owner"];
    if manager(owner).is_err() {
        if !text(&input["taskId"]).is_empty() || !text(&input["afterDispatchId"]).is_empty() {
            bail!("Task links require live local manager");
        }
        return Ok(());
    }
    let task = state.tasks.iter().find(|t| t["taskId"] == input["taskId"]);
    if let Some(task) = task {
        if dependency_state(task, &state.tasks) != "ready" {
            bail!("Task cancelled or waiting for accepted dependency evidence");
        }
        if task["sources"].as_array().is_some_and(|sources| {
            !sources.is_empty()
                && !sources.iter().any(|s| {
                    state.requests.iter().any(|r| {
                        r["requestId"] == s["requestId"]
                            && r["ownerSessionId"] == owner["sessionId"]
                            && r["intents"].as_array().into_iter().flatten().any(|i| {
                                i["key"] == s["intentKey"]
                                    && i["taskId"] == task["taskId"]
                                    && same_cwd(text(&i["cwd"]), text(&task["projectCwd"]))
                            })
                    })
                })
        }) {
            bail!("Task has no committed source request for this owner/project");
        }
    }
    if text(&input["taskId"]).is_empty()
        && retry_source(state, input).is_none()
        && state
            .requests
            .iter()
            .any(|r| r["ownerSessionId"] == owner["sessionId"] && r.get("intents").is_some())
    {
        bail!(
            "Resolve source request and use its taskId before dispatch, or explicit trackTask:false"
        );
    }
    if !text(&input["taskId"]).is_empty()
        && task.is_none_or(|t| {
            t["ownerSessionId"] != owner["sessionId"] || !same_project(t, &input["projectCwd"])
        })
    {
        bail!("Task belongs to another manager/project or is unavailable");
    }
    if let Some(task) = task {
        if task.get("workflow").is_some() {
            let steps = task["workflow"]["steps"]
                .as_array()
                .context("Invalid pinned steps")?;
            if !steps.iter().any(|s| s["id"] == input["workflowStepId"]) {
                bail!("Workflow-bound task requires explicit workflowStepId");
            }
            if steps
                .iter()
                .find(|s| !terminal(&s["state"]))
                .is_none_or(|s| s["id"] != input["workflowStepId"])
            {
                bail!("Workflow step is not next");
            }
        } else if !text(&input["workflowStepId"]).is_empty() {
            bail!("Workflow step requires pinned task");
        }
        if !text(&input["afterDispatchId"]).is_empty()
            && !task["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["dispatchId"] == input["afterDispatchId"])
        {
            bail!("Predecessor must belong to same task");
        }
    } else if !text(&input["workflowStepId"]).is_empty()
        || !text(&input["afterDispatchId"]).is_empty()
    {
        bail!("Workflow and predecessor links require a task");
    }
    Ok(())
}
impl TaskStore {
    pub fn validate_admission(&self, input: &Value) -> Result<()> {
        validate_admission(&self.snapshot()?, input)
    }
    /// Input owner, retry provenance and sessionId must come from host admission.
    pub fn accept(
        &self,
        input: Value,
        authorize: impl FnOnce() -> Result<()>,
    ) -> Result<Option<Value>> {
        let accepted=self.transaction(|state|{
            authorize()?;
            if input.get("trackTask").is_some_and(|v|!v.is_boolean()){bail!("trackTask must be boolean");}
            if input["trackTask"]==false||manager(&input["owner"]).is_err(){validate_admission(state,&input)?;return Ok(None);}
            let session=bounded_id(&input["sessionId"])?;
            for task in &state.tasks {
                if let Some(a)=task["attempts"].as_array().unwrap().iter().find(|a|a["sessionId"]==session) {
                    if task["ownerSessionId"]!=input["owner"]["sessionId"] || !same_project(task,&input["projectCwd"])
                        || input.get("taskId").is_some_and(|v|!text(v).is_empty()&&*v!=task["taskId"])
                        || input.get("workflowStepId").is_some_and(|v|!text(v).is_empty()&&*v!=a["workflowStepId"]) {
                        bail!("Recorded session attempt belongs to a different manager/project/task/step");
                    }
                    return Ok(Some(json!({"taskId":task["taskId"],"dispatchId":a["dispatchId"]})));
                }
            }
            validate_admission(state,&input)?;
            let source=retry_source(state,&input);let mut id=source.as_ref().map(|(id,_)|id.clone()).unwrap_or_else(||text(&input["taskId"]).into());let at=now();
            if id.is_empty(){id=uuid::Uuid::new_v4().to_string();state.tasks.push(json!({"taskId":id,"ownerSessionId":input["owner"]["sessionId"],"ownerLabel":input["owner"].get("label").unwrap_or(&input["owner"]["sessionId"]),"projectCwd":input["projectCwd"],"title":input["title"].as_str().unwrap_or("Unclassified dispatch").chars().take(300).collect::<String>(),"createdAt":at,"attempts":[]}));}
            let bound=state.task(&id)?;
            if bound.get("workflow").is_some() {
                let reservation=&bound["dispatchReservation"];
                let token=text(&input["workflowReservationToken"]);
                if token.is_empty() || reservation["token"]!=token || reservation["stepId"]!=input["workflowStepId"] {
                    bail!("Workflow attempt requires its host-owned dispatch reservation");
                }
                let run=bound["workflow"]["steps"].as_array().unwrap().iter().find(|s|s["id"]==input["workflowStepId"]);
                if !run.is_some_and(|s|s["state"]=="planned") { bail!("Workflow step is no longer planned"); }
            }
            let dispatch=uuid::Uuid::new_v4().to_string();let mut attempt=json!({"dispatchId":dispatch,"sessionId":session,"kind":if source.is_some(){"retry"}else{"fresh"},"acceptedAt":at,"observedAt":at,"executionCwd":input["executionCwd"],"lifecycle":"starting","stale":false,"live":true,"resultContract":"absent","metrics":{}});
            for key in ["workflowStepId","afterDispatchId","executionTarget","executionHost","requestedProvider","provider","requestedModel","role","worktree"]{if let Some(v)=input.get(key){attempt[key]=v.clone();}}
            if STAGES.contains(&text(&input["stage"])){attempt["stage"]=input["stage"].clone();}else if let Some((_,source))=&source{if let Some(stage)=source.get("stage"){attempt["stage"]=stage.clone();}}
            if let Some((_,source))=&source{attempt["retryOfDispatchId"]=source["dispatchId"].clone();attempt["afterDispatchId"]=source["dispatchId"].clone();}
            if attempt.get("executionTarget").is_none() && attempt["worktree"]["allocated"]==true && attempt["worktree"]["fallback"]!=true {
                let requested=Path::new(text(&attempt["executionCwd"]));
                if let Ok(canonical)=super::super::paths::canonicalize(requested) {
                    if canonical==requested {if let Ok((dev,ino))=super::directory::identity(&canonical){attempt["worktree"]["directoryIdentity"]=json!({"dev":dev,"ino":ino});}}
                }
            }
            let task=state.task_mut(&id)?;task["attempts"].as_array_mut().unwrap().push(attempt);
            if let Some(step)=task.get_mut("workflow").and_then(|v|v.get_mut("steps")).and_then(Value::as_array_mut).into_iter().flatten().find(|s|s["id"]==input["workflowStepId"]){step["state"]="dispatched".into();step["sessionId"]=session.into();step["dispatchId"]=dispatch.clone().into();remove(step,"reason");}
            Ok(Some(json!({"taskId":id,"dispatchId":dispatch,"_observedAt":at})))
        })?;
        let mut accepted = accepted;
        if let Some(receipt) = accepted.as_mut() {
            if let Some(at) = receipt
                .get("_observedAt")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                self.fresh
                    .lock()
                    .unwrap()
                    .insert(text(&input["sessionId"]).into(), at);
                remove(receipt, "_observedAt");
            }
        }
        Ok(accepted)
    }
    pub fn observe_batch(&self, snapshots: &[Value]) -> Result<()> {
        let mut observed = self.observations.lock().unwrap();
        let snapshots: Vec<Value> = snapshots
            .iter()
            .filter(|s| text(&s["hub"]).is_empty())
            .filter_map(|s| {
                let mut reading = json!({});
                for key in [
                    "sessionId",
                    "status",
                    "ambientState",
                    "pendingApproval",
                    "pendingQuestions",
                    "usage",
                    "statusLine",
                ] {
                    if let Some(v) = s.get(key) {
                        reading[key] = v.clone();
                    }
                }
                (observed.get(text(&s["sessionId"])) != Some(&reading)).then_some(reading)
            })
            .collect();
        if snapshots.is_empty() {
            return Ok(());
        }

        let fresh=self.transaction(|state|{let mut fresh=Vec::new();let at=now();
            for s in &snapshots{if !text(&s["hub"]).is_empty(){continue;}let id=text(&s["sessionId"]);
                for task in &mut state.tasks{
                    let Some(a)=task["attempts"].as_array_mut().unwrap().iter_mut().find(|a|a["sessionId"]==id && a["executionTarget"]!="paired")else{continue;};
                    let lifecycle=if s["status"]=="ended"{"ended"}else if !s["pendingApproval"].is_null()&&s["pendingApproval"]!=false||s["pendingQuestions"].as_array().is_some_and(|a|!a.is_empty())||s["pendingQuestions"]["length"].as_u64().unwrap_or(0)>0{"needs-decision"}else if s["status"]=="starting"{"starting"}else if s["ambientState"]=="idle"{"idle"}else{"running"};
                    a["observedAt"]=at.clone().into();a["stale"]=false.into();a["live"]=(s["status"]!="ended").into();a["lifecycle"]=lifecycle.into();fresh.push((id.to_string(),at.clone()));
                    if s["status"]=="ended"{if a.get("endedAt").is_none(){a["endedAt"]=at.clone().into();}}else{remove(a,"endedAt");}
                    if let Ok(start)=chrono::DateTime::parse_from_rfc3339(text(&a["acceptedAt"])){a["metrics"]["wallMs"]=(chrono::Utc::now().signed_duration_since(start).num_milliseconds().max(0)).into();}
                    let sl=&s["statusLine"];let usage=&s["usage"];
                    for key in ["totalInputTokens","totalOutputTokens","costUSD"]{let input=sl.get(key).filter(|v|!v.is_null()).or_else(||usage.get(key).filter(|v|v.as_f64().is_some_and(|n|n>0.)));if let Some(v)=input.filter(|v|v.as_f64().is_some_and(|n|n>=0.&&n.is_finite())){a["metrics"][match key{"totalInputTokens"=>"inputTokens","totalOutputTokens"=>"outputTokens",_=>key}]=v.clone();}}
                    if let Some(cache)=usage.get("cache"){a["metrics"]["cache"]=cache.clone();}if sl["cachedInputTokens"].as_f64().is_some_and(|n|n>=0.){a["metrics"]["cachedInputTokens"]=sl["cachedInputTokens"].clone();}
                    let model=sl.get("modelDisplay").filter(|v|!text(v).is_empty()).or_else(||usage.get("model")).filter(|v|!text(v).is_empty());if let Some(model)=model{a["reportedModel"]=model.clone();}
                    if sl["contextHealth"].is_object(){a["metrics"]["context"]=json!({"used":sl["contextHealth"]["usedTokens"],"limit":sl["contextHealth"]["windowTokens"],"observedAt":sl["contextHealth"]["observedAt"]});}
                    else if let(Some(pct),Some(window))=(sl["contextUsedPct"].as_f64(),sl["contextWindowSize"].as_f64()){if pct>=0.&&window>=0.&&sl["receivedAt"].is_string(){a["metrics"]["context"]=json!({"used":(pct.min(100.)*window/100.).round(),"limit":window,"observedAt":sl["receivedAt"]});}}
                    let absent=a["resultContract"]=="absent";
                    if absent{for step in task.get_mut("workflow").and_then(|v|v.get_mut("steps")).and_then(Value::as_array_mut).into_iter().flatten().filter(|s|s["sessionId"]==id&&s["state"]!="waived"){
                        step["state"]=match lifecycle{"needs-decision"=>"blocked","ended"=>"failed",_=>"dispatched"}.into();if lifecycle=="needs-decision"{step["reason"]="Worker needs a decision".into();}else if lifecycle=="ended"{step["reason"]="Worker ended without validated result".into();}else{remove(step,"reason");}
                    }}
                }
            }Ok(fresh)
        })?;
        for (id, _) in &fresh {
            if let Some(snapshot) = snapshots.iter().find(|s| s["sessionId"] == *id) {
                observed.insert(id.clone(), snapshot.clone());
            }
        }
        self.fresh.lock().unwrap().extend(fresh);
        Ok(())
    }
}
fn bounded_id(v: &Value) -> Result<&str> {
    let s = v.as_str().context("sessionId required")?;
    if s.is_empty() || s.len() > 256 {
        bail!("invalid sessionId");
    }
    Ok(s)
}

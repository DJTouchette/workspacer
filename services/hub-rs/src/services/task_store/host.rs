use super::*;
impl TaskStore {
    pub fn read_for_host(&self, sessions: &[Value]) -> Result<Value> {
        let state = self.snapshot()?;
        let mut owners: Vec<_> = sessions.iter().filter(|s| manager(s).is_ok()).collect();
        owners.sort_by_key(|s| std::cmp::Reverse(s["startedAt"].as_i64().unwrap_or(0)));
        let tasks: Vec<_> = state
            .tasks
            .iter()
            .rev()
            .map(|t| {
                let mut t = self.view(t);
                for a in t["attempts"].as_array_mut().unwrap() {
                    if let Some(actual) = sessions
                        .iter()
                        .find(|s| s["sessionId"] == a["sessionId"] && text(&s["hub"]).is_empty())
                    {
                        if actual["status"] == "ended" {
                            a["stale"] = false.into();
                            a["live"] = false.into();
                            a["lifecycle"] = "ended".into();
                        } else {
                            a["live"] = true.into();
                        }
                    }
                }
                t
            })
            .collect();
        let requests:Vec<_>=state.requests.iter().filter(|r|owners.iter().any(|o|o["sessionId"]==r["ownerSessionId"])).map(|r|json!({"ownerSessionId":r["ownerSessionId"],"requestId":r["requestId"],"delivery":r["delivery"],"resolved":r.get("intents").is_some()})).collect();
        let mut out = json!({"available":true,"tasks":tasks,"requests":requests});
        if let Some(owner) = owners.first() {
            out["currentOwnerSessionId"] = owner["sessionId"].clone();
        }
        Ok(out)
    }
    pub fn edit_by_host(
        &self,
        request: Value,
        session: &OwnerLookup,
        busy: impl Fn(&str) -> bool,
    ) -> Value {
        let id = text(&request["taskId"]).to_string();
        let outcome=self.transaction(|state|{
            let expected=request["expectedTaskRevision"].as_u64().filter(|r|*r<=9_007_199_254_740_991).context("Invalid task revision")?;let task=state.task_mut(&id)?;if revision(task)!=expected{bail!("conflict: task changed; reload before editing");}
            match text(&request["action"]){
                "links"=>{task["links"]=references::validate(&request["links"])?;link_audit(task,"host-user","Task references edited by you");},
                "waive"=>{
                    let step_id=text(&request["stepId"]);if task["dispatchReservation"]["stepId"]==step_id||task.get("dispatchReservation").is_none()&&busy(&id){bail!("A step is being dispatched; wait for it to start");}
                    let steps=task["workflow"]["steps"].as_array().context("Step unavailable")?;let at=steps.iter().position(|s|s["id"]==step_id).context("Step unavailable")?;let run=&steps[at];
                    if terminal(&run["state"])||run["state"]=="dispatched"||!matches!(text(&run["state"]),"planned"|"failed"|"blocked"){bail!("Step is finished, dispatched or unknown");}
                    let has_session=!text(&run["sessionId"]).is_empty();let has_dispatch=!text(&run["dispatchId"]).is_empty();
                    if has_session||has_dispatch||run["state"]!="planned"{
                        let attempt=task["attempts"].as_array().unwrap().iter().find(|a|a["dispatchId"]==run["dispatchId"]&&a["sessionId"]==run["sessionId"]&&a["workflowStepId"]==run["id"]).context("Worker status unknown")?;
                        let current=session(text(&attempt["sessionId"])).context("Exact attached worker has not been verified stopped")?;if current["sessionId"]!=attempt["sessionId"]||!text(&current["hub"]).is_empty()||current["status"]!="ended"{bail!("Exact attached worker has not been verified stopped");}
                        if steps[..at].iter().any(|s|!terminal(&s["state"])){bail!("An earlier step is unfinished");}
                    }
                    let reason=request["reason"].as_str().unwrap_or("Skipped for this task by you");if reason.trim().is_empty()||reason.encode_utf16().count()>2000{bail!("Use a reason of1-2000 characters");}
                    let audit_id=uuid::Uuid::new_v4().to_string();let run=&mut task["workflow"]["steps"][at];run["state"]="waived".into();run["waiverId"]=audit_id.clone().into();
                    if !task["audit"].is_array(){task["audit"]=json!([]);}task["audit"].as_array_mut().unwrap().push(json!({"id":audit_id,"actor":"host-user","action":"waive","stepId":step_id,"reason":reason.trim(),"createdAt":now()}));cap_link_audit(task);
                },_=>bail!("Unknown task action"),
            }Ok(())
        });
        match outcome {
            Ok(()) => json!({"ok":true,"task":self.task(&id).ok().flatten()}),
            Err(e) => {
                json!({"ok":false,"code":if e.to_string().starts_with("conflict:"){"conflict"}else{"ineligible"},"error":e.to_string(),"task":self.task(&id).ok().flatten()})
            }
        }
    }
    pub fn update_references(
        &self,
        id: &str,
        owner: &str,
        cwd: &str,
        expected: u64,
        upsert: Option<&Value>,
        removals: Option<&Value>,
        authorize: impl FnOnce() -> Result<()>,
    ) -> Result<Value> {
        self.mutate_owned(id, owner, cwd, expected, authorize, |task| {
            if task.get("dispatchReservation").is_some() {
                bail!("Task dispatch is in progress");
            }
            let links = references::apply(task.get("links"), upsert, removals)?;
            if task.get("links").unwrap_or(&json!({})) != &links {
                task["links"] = links;
                link_audit(
                    task,
                    "manager",
                    "Task references recorded by manager from conversation",
                );
            }
            Ok(())
        })
    }
    pub fn open_target(&self, request: &Value) -> Result<Value> {
        let task = self
            .task(text(&request["taskId"]))?
            .context("Task unavailable")?;
        if request["kind"] == "worktree" {
            let attempt = task["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["dispatchId"] == request["dispatchId"])
                .context("Attempt unavailable")?;
            if attempt["worktree"]["allocated"] != true
                || attempt["worktree"]["fallback"] == true
                || attempt["executionTarget"] == "paired"
                || !Path::new(text(&attempt["executionCwd"])).is_absolute()
            {
                bail!("No recorded allocated local worktree");
            }
            let path = Path::new(text(&attempt["executionCwd"]));
            let canonical = super::super::paths::canonicalize(path)?;
            let (dev, ino) = super::directory::identity(&canonical)?;
            let identity = &attempt["worktree"]["directoryIdentity"];
            if canonical != path
                || identity["dev"].as_u64() != Some(dev)
                || identity["ino"].as_u64() != Some(ino)
            {
                bail!("Recorded worktree moved or identity unavailable");
            }
            return Ok(json!({"kind":"worktree","target":canonical}));
        }
        if request["kind"] != "url" {
            bail!("Unknown task target");
        }
        let links = references::validate(task.get("links").unwrap_or(&json!({})))?;
        let category = text(&request["reference"]);
        let value = if category == "pullRequest" {
            &links["pullRequest"]["url"]
        } else if ["tickets", "references"].contains(&category) {
            let index = request["index"]
                .as_u64()
                .context("Invalid reference index")?;
            links[category]
                .as_array()
                .and_then(|a| a.get(index as usize))
                .and_then(|r| r.get("url"))
                .context("Reference URL unavailable")?
        } else {
            bail!("Unknown reference kind");
        };
        Ok(json!({"kind":"url","target":references::url(value)?}))
    }
}
fn link_audit(task: &mut Value, actor: &str, reason: &str) {
    if !task["audit"].is_array() {
        task["audit"] = json!([]);
    }
    task["audit"].as_array_mut().unwrap().push(json!({"id":uuid::Uuid::new_v4().to_string(),"actor":actor,"action":"links","reason":reason,"createdAt":now()}));
    cap_link_audit(task)
}
fn cap_link_audit(task: &mut Value) {
    if let Some(a) = task.get_mut("audit").and_then(Value::as_array_mut) {
        let mut count = 0;
        a.reverse();
        a.retain(|e| {
            if e["action"] == "waive" {
                true
            } else if e["action"] == "links" {
                count += 1;
                count <= 40
            } else {
                false
            }
        });
        a.reverse();
    }
}

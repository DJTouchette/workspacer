use serde_json::{Value, json};
pub(super) fn op(name: &str) -> Option<&'static str> {
    Some(match name {
        "list_workflows" => "list",
        "get_workflow" => "get",
        "validate_workflow" => "validate",
        "create_workflow" => "create",
        "update_workflow" => "update",
        "clone_workflow" => "clone",
        "disable_workflow" => "disable",
        "delete_workflow" => "delete",
        "select_default_workflow" | "select_project_workflow" => "select",
        "start_workflow" => "start",
        "next_workflow_step" => "next",
        "decide_workflow_step" => "decide",
        "list_manager_requests" => "requestInbox",
        "get_manager_request" => "requestContent",
        "resolve_manager_request" => "resolveRequest",
        "accept_task_outcome" => "acceptTaskOutcome",
        "get_task_references" => "taskReferences",
        "update_task_references" => "setTaskReferences",
        _ => return None,
    })
}
pub(super) fn prepare(name: &str, params: &mut Value, session: &str) -> Result<(), String> {
    if session.is_empty() {
        return Err("Fleet workflows require a local authenticated session; use server Settings for host management".into());
    }
    if name == "select_project_workflow" && params["cwd"].as_str().unwrap_or("").is_empty() {
        return Err("project selection requires cwd".into());
    }
    if name == "select_default_workflow" && !params["cwd"].as_str().unwrap_or("").is_empty() {
        return Err("global selection must omit cwd".into());
    }
    if name == "update_task_references" {
        if params["expectedTaskRevision"].as_u64().is_none() {
            return Err("expectedTaskRevision is required; call get_task_references first".into());
        }
        if !["upsert", "remove"].iter().any(|key| {
            params[key]
                .as_array()
                .is_some_and(|entries| !entries.is_empty())
        }) {
            return Err("supply at least one upsert or remove entry".into());
        }
    }
    let op = op(name).ok_or_else(|| "unknown workflow tool".to_owned())?;
    let map = params.as_object_mut().unwrap();
    map.remove("compact");
    map.insert("op".into(), op.into());
    map.insert("callerSessionId".into(), session.into());
    if op == "select" {
        map.entry("workflowId").or_insert(Value::Null);
    }
    Ok(())
}
pub(super) fn compact(mut value: Value) -> Value {
    fn strip(task: &mut Value) -> bool {
        let mut omitted = false;
        for template in task
            .get_mut("workflow")
            .and_then(|workflow| workflow.get_mut("templates"))
            .and_then(Value::as_object_mut)
            .into_iter()
            .flat_map(|templates| templates.values_mut())
        {
            if let Some(template) = template.as_object_mut() {
                omitted |= template.remove("body").is_some();
            }
        }
        omitted
    }
    let mut omitted = value.get_mut("task").is_some_and(strip);
    for task in value
        .get_mut("tasks")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        omitted |= strip(task);
    }
    if omitted {
        value["omitted"] = json!([
            "workflow template bodies; use next_workflow_step with compact:false for the full pinned task"
        ]);
    }
    value
}

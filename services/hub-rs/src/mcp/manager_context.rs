use crate::client::Client;
use anyhow::{Result, bail};
use futures_util::future::{BoxFuture, join_all};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
type Read = Arc<dyn Fn(Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;
pub(super) async fn call(client: &Client, params: Value, session: &str) -> Result<Value> {
    let client = client.clone();
    read(
        params,
        session,
        Arc::new(move |params| {
            let client = client.clone();
            Box::pin(async move { client.call("fleetWorkflows.request", params).await })
        }),
    )
    .await
}
async fn read(params: Value, session: &str, request: Read) -> Result<Value> {
    if session.is_empty() {
        bail!("manager_context requires an authenticated local manager");
    }
    let empty = Vec::new();
    let tasks = match params.get("tasks") {
        None | Some(Value::Null) => &empty,
        Some(Value::Array(tasks)) => tasks,
        _ => bail!("tasks must be an array"),
    };
    if tasks.len() > 4 {
        bail!("manager_context accepts at most four tasks");
    }
    let mut seen = BTreeSet::new();
    for task in tasks {
        let id = task["taskId"].as_str().unwrap_or("");
        if id.trim().is_empty()
            || task["cwd"].as_str().unwrap_or("").trim().is_empty()
            || !seen.insert(id)
        {
            bail!("tasks require unique taskId values and cwd");
        }
    }
    let mut calls = vec![json!({"op":"requestInbox","view":"pending","callerSessionId":session})];
    calls.extend(tasks.iter().map(|task|json!({"op":"next","callerSessionId":session,"taskId":task["taskId"],"cwd":task["cwd"]})));
    let results = join_all(calls.into_iter().map(|params| {
        let request = request.clone();
        async move {
            match request(params).await {
                Ok(value) => value,
                Err(error) => json!({"ok":false,"error":error.to_string()}),
            }
        }
    }))
    .await;
    Ok(
        json!({"inbox":results[0],"tasks":results[1..].iter().zip(tasks).map(|(response,request)|task_context(response.clone(),request)).collect::<Vec<_>>()}),
    )
}
fn task_context(mut response: Value, request: &Value) -> Value {
    let fail = |error: &str| json!({"ok":false,"taskId":request["taskId"],"cwd":request["cwd"],"error":error});
    if !response.is_object() {
        return fail("Unreadable task context");
    }
    if response["ok"] != true {
        response["taskId"] = request["taskId"].clone();
        response["cwd"] = request["cwd"].clone();
        return response;
    }
    let task = &response["task"];
    if !task.is_object() || task.as_object().unwrap().is_empty() {
        return fail("Missing task context");
    }
    if task["taskId"] != request["taskId"] {
        return fail("Mismatched task context");
    }
    let mut result = json!({"ok":true,"taskId":request["taskId"],"cwd":request["cwd"]});
    for key in [
        "title",
        "revision",
        "cancelled",
        "dependsOn",
        "acceptedOutcome",
        "links",
    ] {
        if let Some(value) = task.get(key) {
            result[key] = value.clone();
        }
    }
    let workflow = task["workflow"].as_object();
    if let Some(workflow) = workflow {
        for key in ["steps", "hash"] {
            if let Some(value) = workflow.get(key) {
                result[key] = value.clone();
            }
        }
    }
    if workflow.is_none_or(|workflow| workflow.is_empty())
        && let Some(attempts) = task["attempts"].as_array()
    {
        let start = attempts.len().saturating_sub(4);
        if start > 0 {
            result["attemptsRemaining"] = json!(start);
        }
        result["attempts"] = json!(&attempts[start..]);
    }
    for key in ["instructions", "dispatch"] {
        if let Some(value) = response.get(key) {
            result[key] = value.clone();
        }
    }
    let bytes = serde_json::to_vec(&result).unwrap().len();
    if bytes > 24 * 1024 {
        return json!({"ok":true,"taskId":request["taskId"],"cwd":request["cwd"],"contentDeferred":true,"bytes":bytes,"instructions":format!("Read next_workflow_step for task {} with compact:true; this task's evidence exceeds the context batch budget.",request["taskId"].as_str().unwrap_or(""))});
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn batch_preserves_partial_errors_verified_identity_and_requested_order() {
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let call: Read = Arc::new(move |params| {
            let barrier = barrier.clone();
            Box::pin(async move {
                barrier.wait().await;
                assert_eq!(params["callerSessionId"], "verified");
                match params["op"].as_str().unwrap() {
                    "requestInbox" => {
                        assert_eq!(params["view"], "pending");
                        Ok(json!({"ok":true,"requests":[{"id":"request"}]}))
                    }
                    "next" => {
                        if params["taskId"] == "denied" {
                            bail!("task belongs to another manager");
                        }
                        Ok(
                            json!({"ok":true,"task":{"taskId":params["taskId"],"revision":7,"workflow":{"hash":"h","steps":[{"id":"review"}],"templates":{"review":{"body":"large repeated instructions"}}}}}),
                        )
                    }
                    _ => panic!("unexpected mutation"),
                }
            })
        });
        let value=tokio::time::timeout(std::time::Duration::from_secs(2),read(json!({"callerSessionId":"forged","tasks":[{"taskId":"owned","cwd":"/repo"},{"taskId":"denied","cwd":"/other"}]}),"verified",call)).await.expect("all independent reads must overlap").unwrap();
        assert_eq!(value["inbox"]["requests"][0]["id"], "request");
        assert_eq!(value["tasks"][0]["revision"], 7);
        assert_eq!(value["tasks"][0]["hash"], "h");
        assert!(value["tasks"][0].get("workflow").is_none());
        assert_eq!(value["tasks"][1]["taskId"], "denied");
        assert_eq!(value["tasks"][1]["ok"], false);
    }
    #[tokio::test]
    async fn invalid_batches_refuse_before_any_host_call() {
        let call: Read = Arc::new(|_| panic!("invalid request reached host"));
        assert!(read(json!({}), "", call.clone()).await.is_err());
        for tasks in [
            json!([{"taskId":"x","cwd":"/a"},{"taskId":"x","cwd":"/b"}]),
            json!([{"taskId":"x","cwd":" "}]),
            json!([{}, {}, {}, {}, {}]),
        ] {
            assert!(
                read(json!({"tasks":tasks}), "verified", call.clone())
                    .await
                    .is_err()
            );
        }
    }
    #[test]
    fn task_evidence_is_preserved_or_explicitly_deferred() {
        let request = json!({"taskId":"t","cwd":"/repo"});
        let response = json!({"ok":true,"task":{"taskId":"t","attempts":[0,1,2,3,4,5],"revision":18446744073709551615_u64,"acceptedOutcome":{"evidence":"kept"}}});
        let result = task_context(response, &request);
        assert_eq!(result["attempts"], json!([2, 3, 4, 5]));
        assert_eq!(result["attemptsRemaining"], 2);
        assert_eq!(result["revision"], json!(18446744073709551615_u64));
        assert_eq!(result["acceptedOutcome"]["evidence"], "kept");
        let result = task_context(
            json!({"ok":true,"task":{"taskId":"t","acceptedOutcome":{"evidence":"x".repeat(25000)}}}),
            &request,
        );
        assert_eq!(result["contentDeferred"], true);
        assert!(result["bytes"].as_u64().unwrap() > 24576);
        assert_eq!(
            task_context(json!({"ok":true,"task":{"taskId":"other"}}), &request)["ok"],
            false
        );
    }
}

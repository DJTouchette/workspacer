//! One explicit pinned step, with no automatic retry after admission begins.
use crate::{client::Client, model_selection::normalize_model_input};
use anyhow::{Result, bail};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::Arc;
type Call = Arc<dyn Fn(String, Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;
type Launch = Arc<dyn Fn(Value) -> BoxFuture<'static, Result<Value>> + Send + Sync>;
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn failure(phase: &str, error: impl ToString, uncertain: bool) -> Value {
    json!({"ok":false,"phase":phase,"error":error.to_string(),"admissionUncertain":uncertain,"instructions":if uncertain{"Spawn admission or delivery may be uncertain. Do not repeat this call; inspect manager_context, list_agents and list_dispatches to reconcile the existing worker first."}else{"No worker was dispatched; an explicit conditional decision may already be recorded. Refresh task state before retrying."}})
}
fn route_receipt(decision: &Value) -> Value {
    let mut value = json!({});
    for key in [
        "decisionId",
        "eligible",
        "provider",
        "model",
        "effort",
        "capability",
        "mode",
        "independentFamily",
        "reason",
    ] {
        value[key] = decision[key].clone();
    }
    value
}
pub(super) async fn call(client: &Client, params: Value, session: &str) -> Result<Value> {
    let bus = client.clone();
    let launch_client = client.clone();
    dispatch(
        params,
        session,
        Arc::new(move |method, params| {
            let bus = bus.clone();
            Box::pin(async move { bus.call(&method, params).await })
        }),
        Arc::new(move |params| {
            let client = launch_client.clone();
            Box::pin(async move { super::spawn::call(&client, "agents.spawn", params).await })
        }),
    )
    .await
}
async fn dispatch(input: Value, session: &str, call: Call, launch: Launch) -> Result<Value> {
    if session.is_empty() {
        bail!("Workflow dispatch requires an authenticated local manager");
    }
    let task = text(&input, "taskId");
    let cwd = text(&input, "cwd");
    let step = text(&input, "stepId");
    let Some(revision) = input["expectedTaskRevision"]
        .as_u64()
        .filter(|_| !task.trim().is_empty() && !cwd.trim().is_empty() && !step.trim().is_empty())
    else {
        bail!("taskId, cwd, stepId and a nonnegative expectedTaskRevision are required")
    };
    let execution = text(&input, "executionTarget");
    if !matches!(execution, "" | "paired") {
        bail!("executionTarget must be paired or omitted");
    }
    if (execution == "paired") != (!text(&input, "remoteCwd").trim().is_empty()) {
        bail!("paired requires remoteCwd; local dispatch must omit it");
    }
    if input.get("run").is_some_and(|value| !value.is_null())
        && text(&input, "reason").trim().is_empty()
    {
        bail!("run requires a reason");
    }
    let watch = input
        .get("watchContextUsedPct")
        .filter(|value| !value.is_null());
    if let Some(watch) = watch {
        if !execution.is_empty()
            || watch
                .as_f64()
                .is_none_or(|value| !value.is_finite() || value <= 0. || value > 100.)
        {
            bail!("watchContextUsedPct must be in (0,100] and is local-only");
        }
    }
    let choice = input.get("modelSelection").filter(|value| !value.is_null());
    if let Some(choice) = choice {
        if input.get("routing").is_some_and(|value| !value.is_null()) {
            bail!(
                "Use modelSelection for an explicit model or routing for automatic selection, not both"
            );
        }
        if text(choice, "provider").trim().is_empty() || text(choice, "model").trim().is_empty() {
            bail!("Explicit model selection requires provider and model");
        }
        let window = match choice.get("contextWindow") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                anyhow::anyhow!("Invalid explicit model selection: invalid-context-window")
            })?),
        };
        normalize_model_input(
            text(choice, "provider"),
            Some(text(choice, "model")),
            None,
            window,
        )
        .map_err(|error| anyhow::anyhow!("Invalid explicit model selection: {}", error.code()))?;
    }
    if execution == "paired" && input["run"] != false {
        let targets = match call("fleet.dispatchTargets".into(), json!({})).await {
            Ok(value) => value,
            Err(error) => return Ok(failure("prepare", error, false)),
        };
        let wanted_provider = choice
            .map(|value| text(value, "provider"))
            .unwrap_or_else(|| text(&input["routing"], "provider"));
        let available = targets["targets"].as_array().is_some_and(|targets| {
            targets.iter().any(|target| {
                target["name"] == "paired"
                    && target["ready"] == true
                    && target["cwds"].as_array().is_some_and(|cwds| {
                        cwds.iter().any(|cwd| cwd["path"] == input["remoteCwd"])
                    })
                    && target["providers"].as_array().is_some_and(|providers| {
                        providers.iter().any(|provider| {
                            provider["found"] == true
                                && provider["authenticated"] == true
                                && (wanted_provider.is_empty()
                                    || provider["provider"] == wanted_provider)
                        })
                    })
            })
        });
        if !available {
            return Ok(failure(
                "prepare",
                "The paired target, selected remote directory and authenticated provider are not ready; no workflow decision or worker dispatch was submitted",
                false,
            ));
        }
    }
    let mut prepare = json!({"op":"prepareDispatch","callerSessionId":session,"taskId":task,"cwd":cwd,"stepId":step,"expectedTaskRevision":revision,"templateParams":input["templateParams"]});
    if input["run"].is_boolean() {
        prepare["run"] = input["run"].clone();
        prepare["reason"] = input["reason"].clone();
    }
    let prepared = match call("fleetWorkflows.request".into(), prepare).await {
        Ok(value) => value,
        Err(error) => return Ok(failure("prepare", error, false)),
    };
    if !prepared.is_object() || !prepared["ok"].is_boolean() {
        return Ok(failure(
            "prepare",
            "Host returned an unreadable dispatch plan",
            false,
        ));
    }
    if prepared["ok"] != true || prepared["skipped"] == true {
        return Ok(prepared);
    }
    let plan = &prepared["dispatch"];
    if plan["taskId"] != task
        || text(plan, "cwd").trim().is_empty()
        || plan["stepId"] != step
        || plan["expectedTaskRevision"]
            .as_u64()
            .is_none_or(|value| value < revision)
        || ["role", "stage", "template"]
            .iter()
            .any(|key| text(plan, key).is_empty())
        || !matches!(text(plan, "toolScope"), "view" | "operator")
    {
        return Ok(failure(
            "prepare",
            "Host returned an incomplete or mismatched dispatch plan",
            false,
        ));
    }
    let decision = if let Some(choice) = choice {
        choice.clone()
    } else {
        let mut route = json!({"role":plan["role"],"cwd":plan["cwd"],"previousProvider":plan["previousProvider"],"profileId":input["profileId"],"ticketId":format!("{task}:{step}")});
        for key in [
            "provider",
            "profile",
            "difficulty",
            "risk",
            "decisionDensity",
            "requireIndependentFamily",
            "forecastDemandBeforeResetPct",
            "expectedWork",
        ] {
            if let Some(value) = input["routing"].get(key) {
                route[key] = value.clone();
            }
        }
        let method = if execution == "paired" {
            route["cwd"] = input["remoteCwd"].clone();
            "fleet.selectDispatchModel"
        } else {
            "routing.select"
        };
        let independent = route["requireIndependentFamily"] == true;
        let decision = match call(method.into(), route).await {
            Ok(value) => value,
            Err(error) => return Ok(failure("routing", error, false)),
        };
        if !decision["eligible"].is_boolean() {
            return Ok(failure(
                "routing",
                format!("Router returned no eligibility decision: {decision}"),
                false,
            ));
        }
        if decision["eligible"] != true {
            return Ok(
                json!({"ok":false,"phase":"routing","admitted":false,"routing":route_receipt(&decision)}),
            );
        }
        if ["provider", "model", "capability", "decisionId"]
            .iter()
            .any(|key| text(&decision, key).is_empty())
            || decision["role"] != plan["role"]
            || (independent && decision["independentFamily"] != true)
        {
            return Ok(failure(
                "routing",
                "Router returned an incomplete or mismatched eligible decision",
                false,
            ));
        }
        decision
    };
    let mut spawn = json!({"cwd":plan["cwd"],"taskId":plan["taskId"],"workflowStepId":plan["stepId"],"expectedTaskRevision":plan["expectedTaskRevision"],"parentSessionId":session,"stage":plan["stage"],"role":plan["role"],"template":plan["template"]});
    for (source, key) in [
        (plan, "afterDispatchId"),
        (plan, "toolScope"),
        (&input, "templateParams"),
    ] {
        if let Some(value) = source.get(key).filter(|value| !value.is_null()) {
            spawn[key] = value.clone();
        }
    }
    for key in ["provider", "model", "effort", "capability", "decisionId"] {
        if let Some(value) = decision.get(key) {
            spawn[key] = value.clone();
        }
    }
    for key in [
        "label",
        "profileId",
        "skipPermissions",
        "executionTarget",
        "remoteCwd",
    ] {
        if let Some(value) = input.get(key) {
            spawn[key] = value.clone();
        }
    }
    if let Some(choice) = choice {
        spawn["exactModel"] = true.into();
        if let Some(window) = choice.get("contextWindow") {
            spawn["contextWindow"] = window.clone();
        }
    }
    super::spawn::prepare(&mut spawn, session)?;
    let mut receipt = match launch(spawn).await {
        Ok(value) => value,
        Err(error) => return Ok(failure("spawn", error, true)),
    };
    if !receipt.is_object() {
        return Ok(failure(
            "spawn",
            format!("Unreadable spawn receipt: {receipt}"),
            true,
        ));
    }
    if execution.is_empty() && text(&receipt, "sessionId").is_empty() {
        return Ok(failure(
            "spawn",
            format!("No local sessionId in spawn receipt: {receipt}"),
            true,
        ));
    }
    if receipt
        .as_object_mut()
        .unwrap()
        .remove("renderedMessage")
        .is_some()
    {
        receipt["renderedMessageOmitted"] = true.into();
    }
    receipt["selection"] = if let Some(choice) = choice {
        json!({"source":"explicit","provider":choice["provider"],"model":choice["model"],"effort":text(choice,"effort"),"contextWindow":choice["contextWindow"]})
    } else {
        route_receipt(&decision)
    };
    if let Some(watch) = watch {
        let id = text(&receipt, "sessionId");
        if id.is_empty() {
            receipt["watchError"]="No confirmed sessionId; inspect admission before arming a watch. Do not repeat the spawn.".into();
        } else {
            match call(
                "agents.notifyWhen".into(),
                json!({"sessionId":id,"notifySessionId":session,"contextUsedPct":watch}),
            )
            .await
            {
                Ok(value) => receipt["watch"] = value,
                Err(error) => {
                    receipt["watchError"] =
                        format!("{error}; the worker was spawned. Retry only notify_when.").into()
                }
            }
        }
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    fn input() -> Value {
        json!({"taskId":"task","cwd":"/alias","stepId":"review","expectedTaskRevision":7})
    }
    fn plan() -> Value {
        json!({"ok":true,"dispatch":{"taskId":"task","cwd":"/canonical","stepId":"review","expectedTaskRevision":8,"role":"reviewer","stage":"review","template":"pinned","toolScope":"view","previousProvider":"claude"}})
    }
    #[tokio::test]
    async fn host_pinned_fields_and_single_spawn_survive_watch_failure() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let call: Call = Arc::new(move |method, params| {
            let captured = captured.clone();
            Box::pin(async move {
                captured
                    .lock()
                    .unwrap()
                    .push((method.clone(), params.clone()));
                match method.as_str() {
                    "fleetWorkflows.request" => {
                        assert_eq!(params["callerSessionId"], "manager");
                        Ok(plan())
                    }
                    "routing.select" => {
                        assert_eq!(params["cwd"], "/canonical");
                        assert_eq!(params["role"], "reviewer");
                        Ok(
                            json!({"eligible":true,"role":"reviewer","provider":"codex","model":"gpt-fixture","decisionId":"decision","capability":"worker.review"}),
                        )
                    }
                    "agents.notifyWhen" => bail!("telemetry unavailable"),
                    _ => panic!("unexpected method"),
                }
            })
        });
        let spawned = Arc::new(Mutex::new(Vec::new()));
        let observed = spawned.clone();
        let launch: Launch = Arc::new(move |params| {
            let observed = observed.clone();
            Box::pin(async move {
                observed.lock().unwrap().push(params);
                Ok(
                    json!({"sessionId":"worker","renderedMessage":"full task","warnings":["kept"],"messageQueued":true}),
                )
            })
        });
        let mut params = input();
        params["watchContextUsedPct"] = 80.into();
        let result = dispatch(params, "manager", call, launch).await.unwrap();
        assert_eq!(spawned.lock().unwrap().len(), 1);
        let spawn = spawned.lock().unwrap()[0].clone();
        assert_eq!(spawn["expectedTaskRevision"], 8);
        assert_eq!(spawn["cwd"], "/canonical");
        assert_eq!(spawn["dispatchOwnerSessionId"], "manager");
        assert_eq!(spawn["template"], "pinned");
        assert!(spawn.get("afterDispatchId").is_none());
        assert!(spawn.get("templateParams").is_none());
        assert_eq!(result["warnings"], json!(["kept"]));
        assert_eq!(result["renderedMessageOmitted"], true);
        assert!(
            result["watchError"]
                .as_str()
                .unwrap()
                .contains("Retry only notify_when")
        );
        assert_eq!(calls.lock().unwrap().len(), 3);
    }
    #[tokio::test]
    async fn skip_and_mismatched_plan_never_route_or_spawn() {
        for response in [
            json!({"ok":true,"skipped":true}),
            json!({"ok":true,"dispatch":{"taskId":"other"}}),
        ] {
            let call: Call = Arc::new(move |method, _| {
                assert_eq!(method, "fleetWorkflows.request");
                let value = response.clone();
                Box::pin(async move { Ok(value) })
            });
            let result = dispatch(
                input(),
                "manager",
                call,
                Arc::new(|_| panic!("unadmitted spawn")),
            )
            .await
            .unwrap();
            assert!(result["skipped"] == true || result["phase"] == "prepare");
        }
    }
    #[tokio::test]
    async fn exact_choice_bypasses_router_and_unknown_admission_never_retries() {
        let mut params = input();
        params["modelSelection"] =
            json!({"provider":"claude","model":"sonnet","contextWindow":200000});
        let call: Call = Arc::new(|method, _| {
            assert_eq!(method, "fleetWorkflows.request");
            Box::pin(async { Ok(plan()) })
        });
        let count = Arc::new(Mutex::new(0));
        let attempts = count.clone();
        let launch: Launch = Arc::new(move |params| {
            let attempts = attempts.clone();
            Box::pin(async move {
                assert_eq!(params["exactModel"], true);
                assert_eq!(params["contextWindow"], 200000);
                *attempts.lock().unwrap() += 1;
                bail!("acknowledgement lost")
            })
        });
        let result = dispatch(params, "manager", call, launch).await.unwrap();
        assert_eq!(result["admissionUncertain"], true);
        assert_eq!(result["phase"], "spawn");
        assert_eq!(*count.lock().unwrap(), 1);
    }
    #[tokio::test]
    async fn conflicting_inputs_are_refused_before_host_mutations() {
        for change in [
            json!({"executionTarget":"other"}),
            json!({"remoteCwd":"/remote"}),
            json!({"run":false}),
            json!({"watchContextUsedPct":0}),
            json!({"modelSelection":{"provider":"claude","model":"sonnet"},"routing":{}}),
        ] {
            let mut params = input();
            params
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            assert!(
                dispatch(
                    params,
                    "manager",
                    Arc::new(|_, _| panic!("invalid request reached host")),
                    Arc::new(|_| panic!("invalid launch"))
                )
                .await
                .is_err()
            );
        }
    }
    #[tokio::test]
    async fn disconnected_pairing_refuses_before_workflow_decision_but_explicit_skip_needs_no_worker()
     {
        let mut params = input();
        params["executionTarget"] = "paired".into();
        params["remoteCwd"] = "X:/remote/repo".into();
        let call: Call = Arc::new(|method, _| {
            assert_eq!(method, "fleet.dispatchTargets");
            Box::pin(async { Ok(json!({"targets":[]})) })
        });
        let result = dispatch(
            params.clone(),
            "manager",
            call,
            Arc::new(|_| panic!("offline pair admitted a spawn")),
        )
        .await
        .unwrap();
        assert_eq!(result["phase"], "prepare");
        assert_eq!(result["admissionUncertain"], false);
        params["run"] = false.into();
        params["reason"] = "not needed".into();
        let call: Call = Arc::new(|method, params| {
            assert_eq!(method, "fleetWorkflows.request");
            assert_eq!(params["run"], false);
            Box::pin(async { Ok(json!({"ok":true,"skipped":true})) })
        });
        assert_eq!(
            dispatch(
                params,
                "manager",
                call,
                Arc::new(|_| panic!("skipped step dispatched"))
            )
            .await
            .unwrap()["skipped"],
            true
        );
    }
}

use crate::client::Client;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::time::Duration;
pub(super) fn prepare(params: &mut Value, session: &str) -> Result<()> {
    if !session.is_empty() {
        params["parentSessionId"] = session.into();
    }
    params["dispatchOwnerSessionId"] = session.into();
    if params["trackTask"] == false
        && [
            "taskId",
            "workflowStepId",
            "afterDispatchId",
            "retrySourceSessionId",
        ]
        .iter()
        .any(|key| params[key].as_str().is_some_and(|s| !s.is_empty()))
    {
        bail!("trackTask:false must omit task, workflow and retry links");
    }
    if params["hub"].as_str().is_some_and(|peer| !peer.is_empty())
        && params["workflowStepId"]
            .as_str()
            .is_some_and(|step| !step.is_empty())
    {
        bail!("Fleet workflow execution is local only");
    }
    Ok(())
}
fn needs_config(params: &Value) -> bool {
    params["skipPermissions"].is_null()
        || (matches!(
            params["provider"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "" | "claude"
        ) && ["model", "modelIdentity"]
            .iter()
            .all(|key| params[*key].as_str().unwrap_or("").is_empty())
            && params["contextWindow"].is_null())
}
// Resolve the facade defaults BEFORE forwarding so central routing sees the
// effective selection, even when the execution provider lives on another node.
fn resolve_defaults(params: &mut Value, config: &Value) -> Result<()> {
    use crate::model_selection::{context_for_new_spawn, normalize_model_input};
    let provider = params["provider"]
        .as_str()
        .unwrap_or("claude")
        .trim()
        .to_ascii_lowercase();
    let provider = if provider.is_empty() {
        "claude"
    } else {
        &provider
    };
    let model = params["model"].as_str().unwrap_or("");
    let identity = params["modelIdentity"].as_str().unwrap_or("");
    let window = if params["contextWindow"].is_null() {
        None
    } else {
        Some(
            params["contextWindow"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| anyhow::anyhow!("spawn_agent: invalid-context-window"))?,
        )
    };
    let window = context_for_new_spawn(provider, window, false);
    let selection =
        if provider == "claude" && model.is_empty() && identity.is_empty() && window.is_none() {
            let model = config["claude"]["defaultModel"].as_str().unwrap_or("");
            if model.trim().is_empty() {
                None
            } else {
                normalize_model_input(
                    "claude",
                    Some(model),
                    None,
                    config["claude"]["contextWindow"].as_u64(),
                )
                .ok()
                .flatten()
            }
        } else {
            normalize_model_input(
                provider,
                (!model.trim().is_empty()).then_some(model),
                (!identity.trim().is_empty()).then_some(identity),
                window,
            )
            .map_err(|e| anyhow::anyhow!("spawn_agent: invalid model selection ({})", e.code()))?
        };
    if window.is_some() {
        params["contextWindow"] = json!(window);
    }
    if let Some(selection) = selection {
        params["model"] = selection.legacy_model.into();
        params["modelIdentity"] = selection.selection.model.into();
        params["contextWindow"] = json!(selection.selection.context_window);
    }
    if params["skipPermissions"].is_null() {
        params["skipPermissions"] = json!(
            config["claude"]["skipPermissionsDefault"] == true
                || matches!(
                    config["claude"]["defaultPermissionMode"].as_str(),
                    Some("bypassPermissions" | "yolo")
                )
        );
    }
    Ok(())
}
pub(super) async fn call(client: &Client, method: &str, mut params: Value) -> Result<Value> {
    let config = if needs_config(&params) {
        client
            .call("config.get", json!({}))
            .await
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    resolve_defaults(&mut params, &config)?;
    if params["exactModel"] == true && method.starts_with("hub:") {
        let peer = method.split_once('/').unwrap().0;
        let capabilities = client
            .call_with_timeout(
                &format!("{peer}/fleet.dispatchCapabilities"),
                json!({}),
                Duration::from_secs(10),
            )
            .await?;
        if capabilities["exactModel"] != true {
            bail!(
                "Update the peer stack to honor an exact model choice; no substitute was launched"
            );
        }
    }
    let mut receipt = client
        .call_with_timeout(
            method,
            params.clone(),
            crate::protocol::provider_timeout(method, Duration::from_secs(30)),
        )
        .await?;
    if params["trackTask"] == false
        && (receipt["taskId"].as_str().is_some_and(|id| !id.is_empty())
            || receipt["dispatchHistoryUnavailable"] == true)
    {
        bail!(
            "Host did not confirm the requested no-task dispatch; a worker may already exist. Do not repeat the spawn. Receipt: {receipt}"
        );
    }
    let message = params["message"].as_str().unwrap_or("");
    if !message.trim().is_empty() && receipt["messageQueued"] != true {
        if receipt.get("messageQueued").is_some() {
            bail!(
                "Spawn was accepted but its first message was not confirmed; do not repeat the spawn or blindly retry delivery. Receipt: {receipt}"
            );
        }
        let Some(id) = receipt["sessionId"].as_str().filter(|id| !id.is_empty()) else {
            bail!(
                "Spawn may have started but no session identity was returned; do not repeat it. Receipt: {receipt}"
            );
        };
        let send = if let Some((peer, _)) = method
            .split_once('/')
            .filter(|_| method.starts_with("hub:"))
        {
            format!("{peer}/agents.sendMessage")
        } else {
            "agents.sendMessage".into()
        };
        if let Err(error) = client
            .call(&send, json!({"sessionId":id,"text":message}))
            .await
        {
            bail!(
                "Spawned session:{id}, but first-message delivery was not confirmed ({error}); do not repeat the spawn. Receipt: {receipt}"
            );
        }
        receipt["messageQueued"] = true.into();
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_explicit_before_remote_routing_and_provider_specific() {
        let config = json!({"claude":{"defaultModel":"opus[1m]","skipPermissionsDefault":true}});
        let mut params = json!({});
        resolve_defaults(&mut params, &config).unwrap();
        assert_eq!(params["model"], "opus[1m]");
        assert_eq!(params["modelIdentity"], "opus");
        assert_eq!(params["contextWindow"], 1_000_000);
        assert_eq!(params["skipPermissions"], true);
        let mut params = json!({"provider":"codex","skipPermissions":false});
        resolve_defaults(&mut params, &config).unwrap();
        assert_eq!(params["skipPermissions"], false);
        assert_ne!(params["model"], "opus[1m]");
        assert_eq!(
            params["contextWindow"],
            crate::model_selection::DEFAULT_CODEX_CONTEXT_WINDOW
        );
        for provider in ["opencode", "pi", "copilot"] {
            let mut params = json!({"provider":provider});
            resolve_defaults(&mut params, &config).unwrap();
            assert!(params["model"].is_null());
            assert!(params["contextWindow"].is_null());
        }
        let mut params = json!({"provider":"claude","model":"sonnet","skipPermissions":false});
        resolve_defaults(&mut params, &config).unwrap();
        assert_eq!(params["model"], "sonnet");
        assert_eq!(params["skipPermissions"], false);
        let mut params = json!({});
        resolve_defaults(&mut params, &Value::Null).unwrap();
        assert_eq!(params["skipPermissions"], false);
        assert!(params["model"].is_null());
        assert!(resolve_defaults(&mut json!({"contextWindow":-1}), &config).is_err());
    }
    #[test]
    fn session_parent_is_authoritative_and_untracked_requests_cannot_smuggle_links() {
        let mut params = json!({"parentSessionId":"other","message":"task"});
        prepare(&mut params, "caller").unwrap();
        assert_eq!(params["parentSessionId"], "caller");
        assert_eq!(params["dispatchOwnerSessionId"], "caller");
        assert!(prepare(&mut json!({"trackTask":false,"taskId":"task"}), "caller").is_err());
        assert!(
            prepare(
                &mut json!({"hub":"worker","workflowStepId":"review"}),
                "caller"
            )
            .is_err()
        );
    }
}

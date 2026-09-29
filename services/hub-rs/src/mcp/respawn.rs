use crate::client::Client;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
const HEADING: &str = "\n\n--- CORRECTION FROM YOUR DISPATCHER (this supersedes anything above it that conflicts) ---\n\n";
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn first<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .copied()
        .find(|value| !value.trim().is_empty())
        .unwrap_or("")
}
fn original(conversation: &Value) -> Option<&str> {
    conversation["items"]
        .as_array()?
        .iter()
        .filter(|item| item["kind"] == "user_message" || item["type"] == "user_message")
        .find_map(|item| item["text"].as_str().filter(|text| !text.trim().is_empty()))
}
fn compose(input: &Value, snapshot: &Value, conversation: &Value, session: &str) -> Result<Value> {
    let source = text(input, "sessionId");
    if source.trim().is_empty() {
        bail!("respawn_with requires sessionId");
    }
    let amendment = text(input, "amendment");
    if amendment.trim().is_empty() {
        bail!(
            "respawn_with requires an amendment — what the successor must do differently. Cloning a scope-creeping worker with no correction just repeats it; use spawn_agent for a plain new dispatch."
        );
    }
    if !snapshot.is_object() {
        bail!("respawn_with: session {source} returned a snapshot this tool could not read");
    }
    let task=original(conversation).ok_or_else(||anyhow::anyhow!("respawn_with: {source} has no first user message to clone — it was never given a task. Use spawn_agent to dispatch fresh."))?;
    let mut params = json!({"retrySourceSessionId":source,"provider":snapshot["provider"],"transport":snapshot["transport"],"cwd":first(&[text(input,"cwd"),text(snapshot,"cwd")]),"effort":first(&[text(input,"effort"),text(&snapshot["settings"],"effort")]),"parentSessionId":snapshot["parentSessionId"],"role":first(&[text(input,"role"),text(&snapshot["routing"],"role")]),"capability":first(&[text(input,"capability"),text(&snapshot["routing"],"capability")]),"message":format!("{task}{HEADING}{amendment}")});
    let label = if text(snapshot, "label").trim().is_empty() {
        String::new()
    } else {
        format!("{} (redispatch)", text(snapshot, "label"))
    };
    params["label"] = first(&[text(input, "label"), &label]).into();
    for key in ["toolScope", "worktree", "trackTask", "hub"] {
        if let Some(value) = input.get(key) {
            params[key] = value.clone();
        }
    }
    if snapshot["resultSchema"].is_object() {
        params["resultSchema"] = snapshot["resultSchema"].clone();
    }
    let model_override = !text(input, "model").trim().is_empty()
        || !text(input, "modelIdentity").trim().is_empty()
        || input
            .get("contextWindow")
            .is_some_and(|value| !value.is_null());
    if model_override {
        for key in ["model", "modelIdentity", "contextWindow"] {
            if let Some(value) = input.get(key) {
                params[key] = value.clone();
            }
        }
    } else if !text(&snapshot["requestedSelection"], "model")
        .trim()
        .is_empty()
    {
        params["modelIdentity"] = snapshot["requestedSelection"]["model"].clone();
        if let Some(window) = snapshot["requestedSelection"].get("contextWindow") {
            params["contextWindow"] = window.clone();
        }
    } else if let Some(model) = snapshot["settings"].get("model") {
        params["model"] = model.clone();
    }
    if !text(input, "model").is_empty() || !text(input, "modelIdentity").is_empty() {
        params["exactModel"] = true.into();
        if text(input, "capability").is_empty() {
            params.as_object_mut().unwrap().remove("capability");
        }
    }
    if input["trackTask"] == false {
        params
            .as_object_mut()
            .unwrap()
            .remove("retrySourceSessionId");
    }
    params["skipPermissions"] = matches!(
        first(&[
            text(snapshot, "livePermissionMode"),
            text(&snapshot["settings"], "permissionMode")
        ]),
        "bypassPermissions" | "yolo"
    )
    .into();
    // These values are settings on an immutable launch selection, not a copy of
    // caller-supplied host authority or the predecessor's routing decision ID.
    super::spawn::prepare(&mut params, session)?;
    Ok(params)
}
pub(super) async fn call(client: &Client, input: Value, session: &str) -> Result<Value> {
    let source = text(&input, "sessionId");
    if source.trim().is_empty() {
        bail!("respawn_with requires sessionId");
    }
    if text(&input, "amendment").trim().is_empty() {
        bail!("respawn_with requires an amendment — what the successor must do differently");
    }
    let peer = text(&input, "hub");
    let route = |method: &str| {
        if peer.is_empty() {
            method.into()
        } else {
            format!("hub:{peer}/{method}")
        }
    };
    let snapshot = client
        .call(&route("sessions.snapshot"), json!({"sessionId":source}))
        .await
        .with_context(|| format!("respawn_with: could not read session {source}"))?;
    let conversation = client
        .call(&route("sessions.conversation"), json!({"sessionId":source}))
        .await
        .with_context(|| format!("respawn_with: could not read {source}'s conversation"))?;
    let mut params = compose(&input, &snapshot, &conversation, session)?;
    params.as_object_mut().unwrap().remove("hub");
    let mut receipt = super::spawn::call(client, &route("agents.spawn"), params.clone()).await?;
    if text(&receipt, "sessionId").is_empty() {
        bail!(
            "respawn_with: the spawn returned no sessionId; a worker may already exist. Inspect list_agents before attempting another dispatch. Receipt: {receipt}"
        );
    }
    receipt["clonedFrom"] = source.into();
    for key in ["cwd", "label", "role", "capability"] {
        receipt[key] = params[key].clone();
    }
    receipt["note"]="The successor was sent the original task plus your correction. Its live provider permission mode was preserved explicitly.".into();
    if input["trackTask"] == false {
        receipt["taskTracking"] = false.into();
    }
    Ok(receipt)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn conversation() -> Value {
        json!({"items":[{"kind":"user_message","text":" "},{"kind":"assistant_text","text":"not the task"},{"kind":"user_message","text":"original task\nexactly"},{"kind":"user_message","text":"later followup"}]})
    }
    fn snapshot() -> Value {
        json!({"cwd":"/repo/worktree","provider":"claude","transport":"stream","label":"fix","parentSessionId":"original-parent","livePermissionMode":"default","settings":{"permissionMode":"yolo","model":"stale","effort":"high"},"requestedSelection":{"model":"sonnet","contextWindow":1000000},"routing":{"role":"reviewer","capability":"reviewer","decisionId":"do-not-clone"},"resultSchema":{"type":"object"}})
    }
    #[test]
    fn clone_preserves_received_task_contract_selection_and_live_permissions() {
        let result = compose(
            &json!({"sessionId":"old","amendment":"only change the parser"}),
            &snapshot(),
            &conversation(),
            "verified-parent",
        )
        .unwrap();
        assert_eq!(
            result["message"],
            format!("original task\nexactly{HEADING}only change the parser")
        );
        assert_eq!(result["cwd"], "/repo/worktree");
        assert_eq!(result["skipPermissions"], false);
        assert_eq!(result["modelIdentity"], "sonnet");
        assert_eq!(result["contextWindow"], 1000000);
        assert!(result.get("model").is_none());
        assert!(result.get("decisionId").is_none());
        assert_eq!(result["parentSessionId"], "verified-parent");
        assert_eq!(result["dispatchOwnerSessionId"], "verified-parent");
        assert_eq!(result["retrySourceSessionId"], "old");
        assert_eq!(result["role"], "reviewer");
        assert_eq!(result["resultSchema"]["type"], "object");
        assert_eq!(result["label"], "fix (redispatch)");
    }
    #[test]
    fn explicit_model_and_untracked_retry_do_not_inherit_stale_links() {
        let result=compose(&json!({"sessionId":"old","amendment":"redo narrowly","trackTask":false,"modelIdentity":"opus","contextWindow":200000}),&snapshot(),&conversation(),"manager").unwrap();
        assert_eq!(result["exactModel"], true);
        assert_eq!(result["modelIdentity"], "opus");
        assert_eq!(result["contextWindow"], 200000);
        assert!(result.get("capability").is_none());
        assert!(result.get("retrySourceSessionId").is_none());
        for mode in ["bypassPermissions", "yolo"] {
            let mut snapshot = snapshot();
            snapshot["livePermissionMode"] = mode.into();
            assert_eq!(
                compose(
                    &json!({"sessionId":"old","amendment":"redo"}),
                    &snapshot,
                    &conversation(),
                    ""
                )
                .unwrap()["skipPermissions"],
                true
            );
        }
    }
    #[test]
    fn missing_original_or_correction_cannot_become_a_new_task() {
        assert!(
            compose(
                &json!({"sessionId":"old","amendment":"redo"}),
                &snapshot(),
                &json!({"items":[]}),
                "manager"
            )
            .is_err()
        );
        assert!(
            compose(
                &json!({"sessionId":"old","amendment":" "}),
                &snapshot(),
                &conversation(),
                "manager"
            )
            .is_err()
        );
    }
}

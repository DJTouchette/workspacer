use crate::client::Client;
use anyhow::Result;
use serde_json::{Value, json};
fn mode(snapshot: &Value) -> &str {
    ["livePermissionMode", "permissionMode", "permission_mode"]
        .iter()
        .filter_map(|key| snapshot[*key].as_str())
        .find(|value| !value.is_empty())
        .or_else(|| {
            snapshot["settings"]["permissionMode"]
                .as_str()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("unknown")
}
pub(super) async fn call(client: &Client, mut params: Value) -> Result<Value> {
    let peer = params["hub"].as_str().unwrap_or("").to_owned();
    params.as_object_mut().unwrap().remove("hub");
    let route = |method: &str| {
        if peer.is_empty() {
            method.into()
        } else {
            format!("hub:{peer}/{method}")
        }
    };
    let mut result = client.call(&route("claude.gate"), params.clone()).await?;
    if !result.is_object() {
        result = json!({"ok":true});
    }
    let snapshot = client
        .call(
            &route("sessions.snapshot"),
            json!({"sessionId":params["sessionId"]}),
        )
        .await
        .unwrap_or(Value::Null);
    let mode = mode(&snapshot);
    result["permissionMode"] = mode.into();
    result["note"]=if params["on"]==true{format!("gate on: tool calls now pause for your approval. The gate is separate from the session's Claude permission mode (currently {mode}).")}else{format!("gate off: workspacer no longer holds tool calls, but the session still prompts per its Claude permission mode (currently {mode}) — the gate and the permission mode are separate; only a bypass permission mode stops prompting.")}.into();
    Ok(result)
}

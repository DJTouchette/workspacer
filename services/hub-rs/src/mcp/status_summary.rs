use crate::client::Client;
use anyhow::{Result, bail};
use serde_json::{Value, json};
const METHOD: &str = "agents.summarizeStatus";
fn unavailable(reason: &str) -> Value {
    json!({"contract":"agent-status-summary/v1","status":"unavailable","reason":reason,"activity":null,"progress":null,"blocker":null,"nextStep":null,"earliestRetainedTask":null,"latestExplicitProgress":null,"source":null,"provider":null,"model":null,"cached":false,"unknowns":[reason]})
}
fn valid(value: &Value) -> bool {
    let text = value.to_string();
    if text.len() > 12000
        || text.chars().count() > 3000
        || value.as_object().is_none_or(|value| value.len() != 14)
    {
        return false;
    }
    if value["contract"] != "agent-status-summary/v1"
        || !matches!(
            value["status"].as_str(),
            Some("ok" | "disabled" | "unavailable")
        )
    {
        return false;
    }
    for (keys, limit) in [
        (
            &[
                "activity",
                "progress",
                "blocker",
                "nextStep",
                "earliestRetainedTask",
                "latestExplicitProgress",
            ][..],
            240,
        ),
        (&["provider", "reason"][..], 80),
        (&["model"][..], 120),
    ] {
        for key in keys {
            if value.get(*key).is_none_or(|value| {
                !value.is_null()
                    && !value
                        .as_str()
                        .is_some_and(|value| value.chars().count() <= limit)
            }) {
                return false;
            }
        }
    }
    if !value["cached"].is_boolean()
        || value["unknowns"].as_array().is_none_or(|items| {
            items.len() > 5
                || items.iter().any(|item| {
                    item.as_str()
                        .is_none_or(|value| value.chars().count() > 240)
                })
        })
    {
        return false;
    }
    let Some(source) = value.get("source") else {
        return false;
    };
    if !source.is_null() {
        if source.as_object().is_none_or(|source| source.len() != 6) {
            return false;
        }
        for key in ["throughSeq", "firstSeq"] {
            if source[key].as_u64().is_none() {
                return false;
            }
        }
        for key in ["headTruncated", "tailTruncated", "textTruncated"] {
            if !source[key].is_boolean() {
                return false;
            }
        }
        if source.get("timestamp").is_none_or(|value| {
            !value.is_null() && value.as_str().is_none_or(|stamp| stamp.len() > 40)
        }) {
            return false;
        }
    }
    true
}
fn normalize(method: &str, response: Result<Value>) -> Result<Value> {
    let value = match response {
        Ok(value) => value,
        Err(error) => {
            let raw = error.to_string();
            let prefix = method
                .strip_prefix("hub:")
                .and_then(|name| name.split_once('/'))
                .map(|(peer, _)| format!("hub:{peer}: "));
            let message = prefix
                .as_ref()
                .and_then(|prefix| raw.strip_prefix(prefix))
                .unwrap_or(&raw);
            if [
                format!("no provider for {method}"),
                format!("no provider for {METHOD}"),
                format!("unknown method: {method}"),
                format!("unknown method: {METHOD}"),
            ]
            .contains(&message.to_owned())
            {
                unavailable("desktop-summary-unavailable")
            } else {
                let lower = raw.to_lowercase();
                if [
                    "permission",
                    "denied",
                    "not allowed",
                    "forbidden",
                    "may not call",
                    "scope",
                ]
                .iter()
                .any(|marker| lower.contains(marker))
                {
                    bail!("permission denied");
                }
                unavailable("source-unavailable")
            }
        }
    };
    Ok(if valid(&value) {
        value
    } else {
        unavailable("invalid-summary-response")
    })
}
pub(super) async fn call(client: &Client, params: Value) -> Result<Value> {
    let peer = params["hub"].as_str().unwrap_or("");
    let method = if peer.is_empty() {
        METHOD.into()
    } else {
        format!("hub:{peer}/{METHOD}")
    };
    normalize(
        &method,
        client
            .call(&method, json!({"sessionId":params["sessionId"]}))
            .await,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_unsupported_errors_degrade_and_provider_output_stays_bounded() {
        assert_eq!(
            normalize(
                "hub:peer/agents.summarizeStatus",
                Err(anyhow::anyhow!(
                    "hub:peer: no provider for agents.summarizeStatus"
                ))
            )
            .unwrap()["reason"],
            "desktop-summary-unavailable"
        );
        assert!(
            normalize(
                METHOD,
                Err(anyhow::anyhow!(
                    "permission denied mentioning no provider for agents.summarizeStatus"
                ))
            )
            .is_err()
        );
        assert_eq!(
            normalize(METHOD, Err(anyhow::anyhow!("sensitive provider stderr"))).unwrap()["reason"],
            "source-unavailable"
        );
        let mut value = unavailable("fixture");
        assert!(valid(&value));
        value["rawTranscript"] = json!("must not escape");
        assert_eq!(
            normalize(METHOD, Ok(value)).unwrap()["reason"],
            "invalid-summary-response"
        );
        let mut value = unavailable("fixture");
        value["source"] = json!({"throughSeq":9007199254740993_u64,"firstSeq":1,"headTruncated":false,"tailTruncated":false,"textTruncated":false,"timestamp":null});
        assert!(valid(&value));
        value["activity"] = json!("🦀".repeat(241));
        assert!(!valid(&value));
    }
}

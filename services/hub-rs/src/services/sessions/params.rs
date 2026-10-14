//! Validate the typed brain caller fields before querying or mutating the engine.
use anyhow::{Result, bail};
use serde_json::Value;

pub(super) fn validate(method: &str, params: &mut Value) -> Result<()> {
    let texts: &[&str] = match method {
        "sessions.transcript" => &["cwd"],
        "claude.approve" => &["reason"],
        "agents.sendMessage" => &["fromSessionId"],
        "claude.answer" => &["text"],
        "claude.mcpReconnect" => &["server"],
        _ => &[],
    };
    for key in texts {
        if params
            .get(*key)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            bail!("{key} must be text");
        }
    }
    if method == "agents.sendMessage"
        && let Some(delivery) = params.get("delivery").filter(|value| !value.is_null())
        && !delivery
            .as_str()
            .is_some_and(|value| claudemon::session::Delivery::from_wire(value).is_some())
    {
        bail!("delivery must be \"steer\" or \"now\"");
    }
    let integer = match method {
        "sessions.conversation" => Some("sinceSeq"),
        "claude.answer" => Some("option"),
        _ => None,
    };
    if let Some(key) = integer {
        if params
            .get(key)
            .is_some_and(|value| !value.is_null() && value.as_i64().is_none())
        {
            bail!("{key} must be an integer");
        }
    }
    if method == "sessions.taskOutput" {
        for key in ["offset", "maxBytes"] {
            if params
                .get(key)
                .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
            {
                bail!("{key} must be a non-negative integer");
            }
        }
    }
    if matches!(method, "sessions.taskOutput" | "sessions.taskStop")
        && params
            .get("taskId")
            .is_some_and(|value| !value.is_null() && !value.is_string())
    {
        bail!("taskId must be text");
    }
    if method == "sessions.conversation"
        && params
            .get("limit")
            .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
    {
        bail!("limit must be a non-negative integer");
    }
    if method == "claude.setFastMode" && !params.get("enabled").is_some_and(Value::is_boolean) {
        bail!("enabled must be a boolean");
    }
    if method == "claude.gate"
        && params
            .get("on")
            .is_some_and(|value| !value.is_null() && !value.is_boolean())
    {
        bail!("on must be a boolean");
    }
    if method == "claude.answer" {
        for key in ["answers", "answerKinds"] {
            if let Some(value) = params.get(key).filter(|value| !value.is_null()) {
                let Some(values) = value.as_array() else {
                    bail!("{key} must be an array of text");
                };
                if values
                    .iter()
                    .any(|value| !value.is_null() && !value.is_string())
                {
                    bail!("{key} must be an array of text");
                }
            }
        }
        // Go's []string decoder maps a null element to its zero-value string.
        // Do this only for these known DTO lists, after every field validated.
        for key in ["answers", "answerKinds"] {
            if let Some(values) = params.get_mut(key).and_then(Value::as_array_mut) {
                for value in values {
                    if value.is_null() {
                        *value = Value::String(String::new());
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate;
    use serde_json::json;

    #[test]
    fn send_message_delivery_is_steer_now_or_absent() {
        for ok in [
            json!({"sessionId":"s","text":"hi"}),
            json!({"sessionId":"s","text":"hi","delivery":null}),
            json!({"sessionId":"s","text":"hi","delivery":"steer"}),
            json!({"sessionId":"s","text":"hi","delivery":"now"}),
        ] {
            let mut params = ok.clone();
            assert!(validate("agents.sendMessage", &mut params).is_ok(), "{ok}");
        }
        for bad in [
            json!({"delivery":"later"}),
            json!({"delivery":"Now"}),
            json!({"delivery":""}),
            json!({"delivery":true}),
            json!({"delivery":["now"]}),
        ] {
            let mut params = bad.clone();
            let error = validate("agents.sendMessage", &mut params).unwrap_err();
            assert!(error.to_string().contains("delivery must be"), "{bad}");
        }
        // The field means nothing to other methods and is not policed there.
        assert!(validate("claude.approve", &mut json!({"delivery":"later"})).is_ok());
    }
}

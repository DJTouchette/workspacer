//! Validate the typed brain caller fields before querying or mutating the engine.
use anyhow::{Result, bail};
use serde_json::Value;

pub(super) fn validate(method: &str, params: &mut Value) -> Result<()> {
    let texts: &[&str] = match method {
        "sessions.transcript" => &["cwd"],
        "claude.approve" => &["reason"],
        "agents.sendMessage" => &["fromSessionId"],
        "claude.answer" => &["text"],
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

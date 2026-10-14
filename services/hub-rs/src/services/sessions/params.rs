//! Validate the typed brain caller fields before querying or mutating the engine.
use anyhow::{Result, bail};
use serde_json::Value;

pub(super) fn validate(method: &str, params: &mut Value) -> Result<()> {
    let texts: &[&str] = match method {
        "sessions.transcript" => &["cwd"],
        "claude.approve" => &["reason", "alwaysOption", "requestId"],
        "agents.sendMessage" => &["fromSessionId"],
        "claude.answer" => &["text", "requestId"],
        "claude.compact" => &["instructions"],
        "claude.rewindFiles" => &["userMessageId"],
        "claude.revert" => &["userMessageId", "beforeTurnId"],
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
    // An explicit always grant names the card it was offered on: the option
    // goes only with a plain "yes", and always with that card's `requestId`.
    if method == "claude.approve"
        && params
            .get("alwaysOption")
            .is_some_and(|value| value.as_str().is_some_and(|v| !v.is_empty()))
    {
        if params.get("decision").and_then(Value::as_str) != Some("yes") {
            bail!("alwaysOption requires decision \"yes\"");
        }
        if !params
            .get("requestId")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
        {
            bail!("alwaysOption requires the card's requestId");
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
    if method == "agents.sendMessage"
        && let Some(images) = params.get("images").filter(|value| !value.is_null())
    {
        let Some(images) = images.as_array() else {
            bail!("images must be an array of {{path}}");
        };
        if images.len() > claudemon::session::images::MAX_IMAGES {
            bail!(
                "a message may carry at most {} images",
                claudemon::session::images::MAX_IMAGES
            );
        }
        if images.iter().any(|image| {
            !image.as_object().is_some_and(|image| {
                image.len() == 1 && image.get("path").is_some_and(Value::is_string)
            })
        }) {
            bail!("images must be an array of {{path}}");
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
    if method == "claude.rewindFiles"
        && params
            .get("preview")
            .is_some_and(|value| !value.is_null() && !value.is_boolean())
    {
        bail!("preview must be a boolean");
    }
    if method == "claude.rewindFiles"
        && params["userMessageId"]
            .as_str()
            .is_none_or(|id| id.is_empty())
    {
        bail!("claude.rewindFiles requires {{ sessionId, userMessageId }}");
    }
    if method == "claude.revert"
        && ["userMessageId", "beforeTurnId"]
            .iter()
            .all(|key| params[*key].as_str().is_none_or(|id| id.is_empty()))
    {
        bail!("claude.revert requires {{ sessionId, userMessageId | beforeTurnId }}");
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

    #[test]
    fn history_controls_name_their_target_with_typed_ids() {
        for (method, ok) in [
            ("claude.compact", json!({"sessionId":"s"})),
            (
                "claude.compact",
                json!({"sessionId":"s","instructions":"keep the API notes"}),
            ),
            (
                "claude.rewindFiles",
                json!({"sessionId":"s","userMessageId":"u"}),
            ),
            (
                "claude.rewindFiles",
                json!({"sessionId":"s","userMessageId":"u","preview":true}),
            ),
            (
                "claude.revert",
                json!({"sessionId":"s","userMessageId":"u"}),
            ),
            ("claude.revert", json!({"sessionId":"s","beforeTurnId":"t"})),
        ] {
            assert!(validate(method, &mut ok.clone()).is_ok(), "{method} {ok}");
        }
        for (method, bad) in [
            ("claude.compact", json!({"sessionId":"s","instructions":7})),
            ("claude.rewindFiles", json!({"sessionId":"s"})),
            (
                "claude.rewindFiles",
                json!({"sessionId":"s","userMessageId":""}),
            ),
            (
                "claude.rewindFiles",
                json!({"sessionId":"s","userMessageId":3}),
            ),
            (
                "claude.rewindFiles",
                json!({"sessionId":"s","userMessageId":"u","preview":"yes"}),
            ),
            ("claude.revert", json!({"sessionId":"s"})),
            (
                "claude.revert",
                json!({"sessionId":"s","beforeTurnId":["t"]}),
            ),
        ] {
            assert!(
                validate(method, &mut bad.clone()).is_err(),
                "{method} {bad}"
            );
        }
    }

    #[test]
    fn card_bindings_and_always_options_must_be_text() {
        for (method, key) in [
            ("claude.approve", "alwaysOption"),
            ("claude.approve", "requestId"),
            ("claude.answer", "requestId"),
        ] {
            for bad in [json!(3), json!(["x"]), json!({"id":"x"}), json!(true)] {
                let error = validate(method, &mut json!({ key: bad })).unwrap_err();
                assert!(error.to_string().contains("must be text"), "{method} {key}");
            }
            let mut ok = json!({ key: "req-1:0" });
            if key == "alwaysOption" {
                ok["decision"] = json!("yes");
                ok["requestId"] = json!("req-1");
            }
            assert!(validate(method, &mut ok).is_ok(), "{method} {key}");
        }
    }

    #[test]
    fn an_always_option_needs_a_yes_and_the_card_id() {
        let error = |params: serde_json::Value| {
            validate("claude.approve", &mut params.clone())
                .unwrap_err()
                .to_string()
        };
        assert!(error(json!({"decision":"yes","alwaysOption":"r:0"})).contains("requestId"));
        assert!(
            error(json!({"decision":"yes","alwaysOption":"r:0","requestId":""}))
                .contains("requestId")
        );
        for decision in ["always", "no"] {
            assert!(
                error(json!({"decision":decision,"alwaysOption":"r:0","requestId":"r"}))
                    .contains("decision")
            );
        }
        assert!(
            validate(
                "claude.approve",
                &mut json!({"decision":"yes","alwaysOption":"r:0","requestId":"r"})
            )
            .is_ok()
        );
        // Without an option nothing changes: the legacy shapes still pass.
        for decision in ["yes", "no", "always"] {
            assert!(validate("claude.approve", &mut json!({ "decision": decision })).is_ok());
        }
    }

    #[test]
    fn send_message_images_are_a_bounded_list_of_paths() {
        for ok in [
            json!({"text":"hi"}),
            json!({"text":"hi","images":null}),
            json!({"text":"hi","images":[]}),
            json!({"text":"hi","images":[{"path":"/tmp/x/m-1-abcdef01.png"}]}),
        ] {
            assert!(
                validate("agents.sendMessage", &mut ok.clone()).is_ok(),
                "{ok}"
            );
        }
        for bad in [
            json!({"images":"/etc/passwd"}),
            json!({"images":["/etc/passwd"]}),
            json!({"images":[{"path":7}]}),
            json!({"images":[{"path":"/a","cwd":"/"}]}),
            json!({"images":[{}]}),
            json!({"images":vec![json!({"path":"/a"}); 9]}),
        ] {
            assert!(
                validate("agents.sendMessage", &mut bad.clone()).is_err(),
                "{bad}"
            );
        }
    }
}

use serde::Deserialize;
use serde_json::{Value, value::RawValue};

pub(super) fn validate(bytes: &[u8]) -> Option<(Value, String)> {
    // Identify only the strict admin calls. Other tools retain their existing
    // JSON semantics; the MCP SDK handles malformed envelopes.
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value["method"] != "tools/call"
        || !matches!(
            value["params"]["name"].as_str(),
            Some(
                "routing_preferences_validate"
                    | "routing_preferences_save"
                    | "routing_preferences_reset"
            )
        )
    {
        return None;
    }
    #[derive(Deserialize)]
    struct Envelope<'a> {
        #[serde(borrow)]
        params: Arguments<'a>,
    }
    #[derive(Deserialize)]
    struct Arguments<'a> {
        #[serde(default, borrow, deserialize_with = "crate::protocol::raw_present")]
        arguments: Option<&'a RawValue>,
    }
    let result = serde_json::from_slice::<Envelope<'_>>(bytes)
        .map_err(anyhow::Error::from)
        .and_then(|raw| {
            crate::services::routing::validate_preferences_raw(
                raw.params
                    .arguments
                    .map(|arguments| arguments.get().as_bytes())
                    .unwrap_or(b"{}"),
            )
        });
    result
        .err()
        .map(|error| (value["id"].clone(), error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_keys_survive_until_the_strict_admin_validator() {
        let raw=br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"routing_preferences_save","arguments":{"baseRevision":"one","patch":{"activeProfile":"safe","activeProfile":"changed"}}}}"#;
        assert!(validate(raw).is_some());
        let raw=br#"{"id":1,"method":"tools/call","params":{"name":"save_config","arguments":{"arbitrary":null}}}"#;
        assert!(validate(raw).is_none());
        assert!(crate::protocol::Frame::decode(br#"{"op":"call","method":"hub:worker/routing.preferences.save","params":{"patch":{"x":1,"x":2}}}"#).is_err());
    }
}

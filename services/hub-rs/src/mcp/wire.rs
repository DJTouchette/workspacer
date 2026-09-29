//! Explicit per-tool wire projections captured from the original Go input types.
//! Schema optionality is insufficient: *bool(false) and *int(0) must survive.
//! Schema validation precedes this projection. Explicit Rust canonical-context
//! null semantics are applied by the reviewed generator override, not guessed.
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};
#[derive(Deserialize)]
struct Contract {
    inputs: BTreeMap<String, Input>,
}
#[derive(Deserialize)]
struct Input {
    rule: Option<Rule>,
}
#[derive(Deserialize)]
struct Rule {
    #[serde(default)]
    omit: String,
    #[serde(default)]
    fields: BTreeMap<String, Rule>,
    item: Option<Box<Rule>>,
}
fn contract() -> &'static Contract {
    static RULES: OnceLock<Contract> = OnceLock::new();
    RULES.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/mcp-effective-wire.json"))
            .expect("reviewed MCP wire contract")
    })
}
fn omitted(value: &Value, rule: &Rule) -> bool {
    match rule.omit.as_str() {
        "nil" => value.is_null(),
        "empty" => {
            value.is_null()
                || value.as_array().is_some_and(Vec::is_empty)
                || value.as_object().is_some_and(serde_json::Map::is_empty)
        }
        "zero" => value == false || value == "" || value.as_f64() == Some(0.),
        _ => false,
    }
}
fn walk(value: &mut Value, rule: &Rule) {
    if let Some(map) = value.as_object_mut() {
        for (key, field) in &rule.fields {
            if map.get(key).is_some_and(|v| omitted(v, field)) {
                map.remove(key);
            } else if let Some(value) = map.get_mut(key) {
                walk(value, field);
            }
        }
    }
    if let (Some(values), Some(item)) = (value.as_array_mut(), &rule.item) {
        for value in values {
            walk(value, item)
        }
    }
}
pub(super) fn project(name: &str, value: &mut Value) {
    let input = contract()
        .inputs
        .get(name)
        .expect("built-in tool has explicit wire policy");
    if let Some(rule) = &input.rule {
        walk(value, rule)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn explicit_pointer_zero_false_and_empty_string_survive_value_omitempty() {
        let mut value = json!({"skipPermissions":false,"contextWindow":0,"model":"","worktree":false,"templateParams":{"task":""}});
        project("spawn_agent", &mut value);
        assert_eq!(
            value,
            json!({"skipPermissions":false,"contextWindow":0,"templateParams":{"task":""}})
        );
        let mut value = json!({"sessionId":"s","option":0,"text":"","answers":[]});
        project("answer", &mut value);
        assert_eq!(value, json!({"sessionId":"s","option":0,"text":""}));
        let mut value =
            json!({"sessionId":"s","tokens":0,"usd":0.0,"idleSeconds":-0.0,"contextUsedPct":80});
        project("notify_when", &mut value);
        assert_eq!(value, json!({"sessionId":"s","contextUsedPct":80}));
        let mut value = json!({"role":"scout","forecastDemandBeforeResetPct":0,"expectedWork":[{"phase":"implementation","count":0}]});
        project("select_model", &mut value);
        assert_eq!(value["forecastDemandBeforeResetPct"], 0);
        assert_eq!(value["expectedWork"][0]["count"], 0);
    }
    #[test]
    fn pointer_object_presence_and_opaque_maps_are_not_erased() {
        let mut value = json!({"routing":{},"modelSelection":{"provider":"codex","model":"chosen","effort":""},"run":false,"skipPermissions":false,"expectedTaskRevision":0,"templateParams":{}});
        project("dispatch_workflow_step", &mut value);
        assert_eq!(value["routing"], json!({}));
        assert_eq!(value["run"], false);
        assert_eq!(value["skipPermissions"], false);
        assert_eq!(value["expectedTaskRevision"], 0);
        assert!(value.get("templateParams").is_none());
        assert!(value["modelSelection"].get("effort").is_none());
        for name in [
            "save_config",
            "save_layout",
            "save_library",
            "save_saved_session",
            "update_profile",
            "propose_job",
        ] {
            let mut value = json!({"keepFalse":false,"keepZero":0,"empty":"","map":{},"array":[],"explicitNull":null});
            let original = value.clone();
            project(name, &mut value);
            assert_eq!(value, original, "{name}");
        }
        let mut value = json!({"requestId":"r","expectedRevision":0,"intents":[{"key":"new","workflowId":null,"replacePullRequest":false,"references":[]}]});
        project("resolve_manager_request", &mut value);
        assert_eq!(
            value["intents"][0],
            json!({"key":"new","workflowId":null,"replacePullRequest":false,"references":[]})
        );
    }
    #[test]
    fn canonical_context_null_is_an_explicit_rust_override_of_the_go_capture() {
        for name in ["spawn_agent", "respawn_with", "set_model"] {
            let mut value = json!({"contextWindow":null});
            project(name, &mut value);
            assert_eq!(value.get("contextWindow"), Some(&Value::Null), "{name}");
        }
        let mut value = json!({"modelSelection":{"provider":"codex","model":"gpt-fixture","contextWindow":null}});
        project("dispatch_workflow_step", &mut value);
        assert_eq!(
            value["modelSelection"].get("contextWindow"),
            Some(&Value::Null)
        );
    }
    #[test]
    fn every_builtin_schema_has_reviewed_wire_policy() {
        let captured: Value =
            serde_json::from_str(include_str!("../../assets/mcp-tools.json")).unwrap();
        let names: std::collections::BTreeSet<_> = captured
            .as_object()
            .unwrap()
            .values()
            .flat_map(|tools| tools.as_array().unwrap())
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.len() >= 100);
        assert_eq!(
            names,
            contract().inputs.keys().map(String::as_str).collect()
        );
    }
}

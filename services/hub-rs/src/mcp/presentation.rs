//! Rust-host presentation changes over the immutable Go catalog/help capture.
//! Keep schema shape and the original oracle intact; only descriptions change.
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolEdit {
    tool: String,
    pointer: String,
    before: String,
    after: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceEdit {
    topic: String,
    before: String,
    after: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edits {
    tools: Vec<ToolEdit>,
    guidance: Vec<GuidanceEdit>,
}
fn edits() -> &'static Edits {
    static EDITS: OnceLock<Edits> = OnceLock::new();
    EDITS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/mcp-rust-presentation.json"))
            .expect("reviewed MCP presentation overlay")
    })
}
pub(super) fn tools(mut catalog: Value) -> Value {
    for edit in &edits().tools {
        assert!(
            edit.pointer.ends_with("/description"),
            "presentation must not change schema constraints"
        );
        let mut matched = 0;
        for tool in catalog
            .as_object_mut()
            .unwrap()
            .values_mut()
            .flat_map(|scope| scope.as_array_mut().unwrap())
        {
            if tool["name"] != edit.tool {
                continue;
            }
            let text = tool
                .pointer_mut(&edit.pointer)
                .expect("captured schema description");
            let original = text.as_str().expect("description string");
            assert!(
                original.contains(&edit.before),
                "review MCP presentation overlay after reference capture changes"
            );
            *text = original.replace(&edit.before, &edit.after).into();
            matched += 1;
        }
        assert!(
            matched > 0,
            "MCP presentation edit must name a captured tool"
        );
    }
    catalog
}
pub(super) fn guidance(guidance: &mut BTreeMap<String, String>) {
    for edit in &edits().guidance {
        let original = guidance.get_mut(&edit.topic).expect("captured help topic");
        assert!(
            original.contains(&edit.before),
            "review MCP guidance overlay after reference capture changes"
        );
        *original = original.replace(&edit.before, &edit.after);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_overlay_preserves_all_tool_names_and_schema_constraints() {
        fn without_descriptions(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    map.remove("description");
                    for v in map.values_mut() {
                        without_descriptions(v);
                    }
                }
                Value::Array(rows) => {
                    for row in rows {
                        without_descriptions(row)
                    }
                }
                _ => (),
            }
        }
        let original: Value =
            serde_json::from_str(include_str!("../../assets/mcp-tools.json")).unwrap();
        let effective = tools(original.clone());
        let generated: Value =
            serde_json::from_str(include_str!("../../assets/mcp-effective-tools.json")).unwrap();
        assert_eq!(
            effective, generated,
            "portable generator must agree with the independent description overlay"
        );
        let mut baseline = original;
        let mut actual = effective.clone();
        without_descriptions(&mut baseline);
        without_descriptions(&mut actual);
        assert_eq!(baseline, actual);
        let spawn = effective["operator"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "spawn_agent")
            .unwrap();
        for field in ["resultSchema", "template", "workflowStepId"] {
            let description = spawn["inputSchema"]["properties"][field]["description"]
                .as_str()
                .unwrap();
            assert!(
                description.contains("standalone") && description.contains("embedded"),
                "{description}"
            );
            assert!(!description.contains("headless brain declines"));
        }
        let raw: Value = serde_json::from_str(include_str!("../../assets/mcp-help.json")).unwrap();
        let mut texts: BTreeMap<String, String> =
            serde_json::from_value(raw["guidance"].clone()).unwrap();
        guidance(&mut texts);
        let generated: Value =
            serde_json::from_str(include_str!("../../assets/mcp-effective-help.json")).unwrap();
        assert_eq!(serde_json::to_value(&texts).unwrap(), generated["guidance"]);
        assert!(texts["workflows"].contains("owning Rust host"));
        assert!(!texts["spawn"].contains("headless declines arbitrary"));
        assert!(
            texts["observe"].contains("headless/old peers"),
            "on-demand status summaries still require their desktop provider"
        );
    }
}

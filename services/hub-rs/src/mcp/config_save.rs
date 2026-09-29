//! A second facade-local wholesale-map check, independent of SDK schema handling.
use serde_json::Value;
const PATHS: &[&[&str]] = &[
    &["ui", "customThemes"],
    &["claude", "budgets"],
    &["projects"],
];
pub(super) fn invalid_wholesale(value: &Value) -> Option<String> {
    for path in PATHS {
        let mut current = value;
        for (index, key) in path.iter().enumerate() {
            let Some(next) = current.as_object().and_then(|map| map.get(*key)) else {
                break;
            };
            if index + 1 == path.len() && !next.is_object() {
                return Some(path.join("."));
            }
            current = next;
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn wholesale_guard_is_independent_of_schema_and_matches_shared_contract() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../../../contracts/wholesale-config-paths.json"
        ))
        .unwrap();
        let expected: std::collections::BTreeSet<_> = contract["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let actual: std::collections::BTreeSet<_> = PATHS.iter().map(|p| p.join(".")).collect();
        assert!(!expected.is_empty());
        assert_eq!(actual, expected);
        for (value, path) in [
            (json!({"projects":"{}"}), "projects"),
            (json!({"projects":null}), "projects"),
            (json!({"projects":[]}), "projects"),
            (json!({"claude":{"budgets":0}}), "claude.budgets"),
            (json!({"ui":{"customThemes":"nord"}}), "ui.customThemes"),
        ] {
            assert_eq!(invalid_wholesale(&value).as_deref(), Some(path));
        }
        for value in [
            json!({"ui":{"theme":"nord"}}),
            json!({"projects":{}}),
            json!({"projects":{"/repo":{}}}),
            json!({"supervisor":{"budgets":"ordinary string"}}),
            json!({"ui":"nord"}),
        ] {
            assert_eq!(invalid_wholesale(&value), None);
        }
    }
}

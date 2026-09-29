use serde_json::Value;
use workspacer_hub::{model_selection::foreign_manager_model, services::models::catalog};
#[test]
fn claude_model_catalog_matches_shared_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/claude-model-catalog-cases.json"
    ))
    .unwrap();
    assert!(fixture["cases"].as_array().unwrap().len() >= 7);
    for case in fixture["cases"].as_array().unwrap() {
        let live: Vec<_> = case["live"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            catalog(&case["config"], &live),
            case["expected"],
            "{}",
            case["name"]
        );
    }
}
#[test]
fn manager_model_vocabulary_ownership_matches_shared_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/model-vocabulary-ownership-cases.json"
    ))
    .unwrap();
    assert!(fixture["ownershipCases"].as_array().unwrap().len() >= 20);
    for case in fixture["ownershipCases"].as_array().unwrap() {
        for provider in ["claude", "codex", "copilot", "opencode", "pi"] {
            let owners = case["owners"].as_array().unwrap();
            let expected = !owners.is_empty() && !owners.iter().any(|v| v == provider);
            assert_eq!(
                foreign_manager_model(provider, case["model"].as_str().unwrap()),
                expected,
                "{}: {provider}",
                case["name"]
            );
        }
    }
}

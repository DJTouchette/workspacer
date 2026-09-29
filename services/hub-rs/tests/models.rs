use serde_json::{Value, json};
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

#[test]
fn context_without_a_claude_manager_model_is_refused_or_pruned_on_read() {
    use workspacer_hub::model_selection::manager_preferences;
    for models in [Value::Null, json!({"claude":""})] {
        let input = json!({"managerModels":models,"managerContextWindows":{"claude":1000000,"codex":272000}});
        assert!(
            manager_preferences(&input, true)
                .unwrap_err()
                .to_string()
                .starts_with("invalid-context-window")
        );
        let loaded = manager_preferences(&input, false).unwrap();
        assert!(loaded["managerContextWindows"].get("claude").is_none());
        assert_eq!(loaded["managerContextWindows"]["codex"], 272000);
    }
    let nullable = json!({"managerContextWindows":{"claude":null}});
    for strict in [false, true] {
        assert_eq!(
            manager_preferences(&nullable, strict).unwrap()["managerContextWindows"].get("claude"),
            Some(&Value::Null)
        );
    }
    for provider in ["copilot", "unknown"] {
        let input = json!({"managerContextWindows":{provider:1000000}});
        assert!(
            manager_preferences(&input, true)
                .unwrap_err()
                .to_string()
                .starts_with("unsupported-context-window")
        );
        assert_eq!(
            manager_preferences(&input, false).unwrap()["managerContextWindows"],
            json!({})
        );
    }
}

#[test]
fn model_selection_is_idempotent_and_fresh_context_defaults_never_rewrite_resume() {
    use workspacer_hub::model_selection::{
        DEFAULT_CODEX_CONTEXT_WINDOW, context_for_new_spawn, normalize_model_selection,
    };
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/model-context-windows.json"
    ))
    .unwrap();
    let defaults = fixture["providerContextDefaults"].as_array().unwrap();
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0]["provider"], "codex");
    assert_eq!(
        defaults[0]["freshContextWindow"],
        DEFAULT_CODEX_CONTEXT_WINDOW
    );
    assert_eq!(
        context_for_new_spawn(" CoDeX ", None, false),
        Some(DEFAULT_CODEX_CONTEXT_WINDOW)
    );
    assert_eq!(context_for_new_spawn("codex", None, true), None);
    assert_eq!(
        context_for_new_spawn("codex", Some(272000), false),
        Some(272000)
    );
    assert_eq!(
        context_for_new_spawn("codex", Some(272000), true),
        Some(272000)
    );
    assert_eq!(context_for_new_spawn("claude", None, false), None);
    let cases = fixture["selectionCases"].as_array().unwrap();
    assert!(cases.len() >= 10);
    let mut successful = 0;
    for case in cases.iter().filter(|case| case["error"].is_null()) {
        let once = normalize_model_selection(
            case["model"].as_str().unwrap(),
            case["contextWindow"].as_u64(),
        )
        .unwrap();
        assert!(!once.model.to_lowercase().ends_with("[1m]"));
        assert!(!once.model.to_lowercase().ends_with("-1m"));
        let twice = normalize_model_selection(&once.model, once.context_window).unwrap();
        assert_eq!(once.model, twice.model);
        assert_eq!(once.context_window, twice.context_window);
        successful += 1;
    }
    assert!(
        successful > 0,
        "no successful selection exercised idempotence"
    );
}

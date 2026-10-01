use serde_json::{Value, json};
use workspacer_hub::{model_selection::foreign_manager_model, services::models::catalog};
#[test]
fn claude_model_catalog_matches_shared_contract() {
    #[path = "support/sweepguard.rs"]
    mod sweepguard;
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/claude-model-catalog-cases.json"
    ))
    .unwrap();
    let mut tally = sweepguard::Tally::default();
    for case in fixture["cases"].as_array().unwrap() {
        let live: Vec<_> = case["live"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        tally.ran("other");
        assert_eq!(
            catalog(&case["config"], &live),
            case["expected"],
            "{}",
            case["name"]
        );
    }
    tally.require_every("Claude model catalog", 7).unwrap();
}

#[test]
fn provider_detection_and_spawn_override_are_isolated_from_parent_environment() {
    use workspacer_hub::services::models::{check_all, resolve_binary, resolve_spawn_binary};
    if std::env::var("WKS_PROVIDER_PARITY_CHILD").as_deref() == Ok("1") {
        let root = std::path::PathBuf::from(std::env::var_os("WKS_PROVIDER_PARITY_ROOT").unwrap());
        let rows = check_all(&json!({}));
        assert_eq!(rows.as_array().unwrap().len(), 5);
        for row in rows.as_array().unwrap() {
            if row["provider"] == "codex" {
                assert_eq!(row["found"], true);
                assert_eq!(row["resolvedPath"], json!(root.join("codex")));
            } else {
                assert_eq!(row["found"], false);
                assert!(row.get("resolvedPath").unwrap().is_null());
            }
        }
        assert_eq!(resolve_spawn_binary("claude", &json!({})), "escaped claude");
        assert_eq!(resolve_binary("claude", &json!({})), "claude");
        assert_eq!(resolve_spawn_binary("copilot", &json!({})), "copilot");
        let config = json!({"agents":{"binaries":{"claude":" configured claude ","pi":root.join("codex"),"opencode":root}}});
        assert_eq!(resolve_spawn_binary("claude", &config), "configured claude");
        let rows = check_all(&config);
        assert_eq!(rows[4]["found"], true);
        assert_eq!(rows[4]["resolvedPath"], json!(root.join("codex")));
        assert_eq!(rows[3]["found"], false);
        assert!(rows[3]["resolvedPath"].is_null());
        let plan = workspacer_hub::services::spawn_plan::resolve(
            &json!({"provider":"claude","transport":"pty","cwd":root}),
            &json!({}),
            None,
            &root,
            "fixture",
            false,
        )
        .unwrap();
        assert_eq!(plan.request["argv"][0], "escaped claude");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("codex"), "fixture, never executed").unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "provider_detection_and_spawn_override_are_isolated_from_parent_environment",
            "--nocapture",
        ])
        .env("WKS_PROVIDER_PARITY_CHILD", "1")
        .env("WKS_PROVIDER_PARITY_ROOT", root.path())
        .env("PATH", root.path())
        .env("WKS_CLAUDE_BIN", " escaped claude ")
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
async fn provider_rpc_refuses_bad_admission_and_reports_missing_engine() {
    let root = tempfile::tempdir().unwrap();
    let mut options = workspacer_hub::Options::default();
    options.home_dir = Some(root.path().into());
    options.config_dir = Some(root.path().join("config"));
    let hub = workspacer_hub::Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = workspacer_hub::client::Client::connect(&hub.handle())
        .await
        .unwrap();
    for provider in ["codex", "copilot", "opencode", "pi"] {
        let error = client
            .call(
                "providers.listModels",
                json!({"provider":provider,"cwd":root.path()}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no execution engine"), "{error}");
    }
    for params in [
        json!({"provider":"claude","cwd":root.path()}),
        json!({"provider":"codex"}),
        json!({"provider":"codex","cwd":"relative"}),
    ] {
        assert!(client.call("providers.listModels", params).await.is_err());
    }
    client.close();
    hub.shutdown().unwrap();
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

use serde_json::json;
use workspacer_hub::services::{config::defaults, profiles::Profile, spawn_plan::resolve};
#[test]
fn plan_preserves_first_message_ack_and_codex_context_omission_vs_null() {
    let dir = tempfile::tempdir().unwrap();
    let config = defaults();
    let plan = resolve(
        &json!({"provider":"codex","cwd":dir.path(),"message":"task"}),
        &config,
        None,
        dir.path(),
        "new-session",
        false,
    )
    .unwrap();
    assert_eq!(plan.request["context_window"], 1000000);
    assert_eq!(plan.request["transport"], "stream");
    assert_eq!(plan.request["first_message"], "task");
    assert_eq!(
        plan.receipt(&json!({"session_id":"new-session","first_message_queued":false}))
            .unwrap()["messageQueued"],
        false
    );
    for extra in [
        json!({"contextWindow":null}),
        json!({"resumeSessionId":"old"}),
    ] {
        let mut request = json!({"provider":"codex","cwd":dir.path()});
        request
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let plan = resolve(&request, &config, None, dir.path(), "new", false).unwrap();
        assert!(plan.request.get("context_window").is_none());
    }
}
#[test]
fn profiles_and_manager_preferences_select_the_executed_model_not_a_shadowed_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = defaults();
    config["agents"]["managerProvider"] = json!("codex");
    config["agents"]["managerModels"] = json!({"codex":"gpt-5-codex"});
    config["agents"]["managerContextWindows"] = json!({"codex":null});
    let plan = resolve(
        &json!({"manager":true,"cwd":dir.path()}),
        &config,
        None,
        dir.path(),
        "manager",
        false,
    )
    .unwrap();
    assert_eq!(plan.provider, "codex");
    assert_eq!(plan.request["model"], "gpt-5-codex");
    assert!(plan.request.get("context_window").is_none());
    let profile = Profile {
        extra_args: vec!["--model".into(), "sonnet[1m]".into()],
        ..Default::default()
    };
    let plan=resolve(&json!({"provider":"claude","cwd":dir.path(),"model":"opus","modelIdentity":"opus","contextWindow":200000}),&config,Some(&profile),dir.path(),"claude",false).unwrap();
    assert_eq!(plan.request["model_identity"], "sonnet");
    assert_eq!(plan.request["context_window"], 1000000);
}
#[test]
fn explicit_permission_false_wins_and_unsupported_provider_or_conflicting_exact_model_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = defaults();
    config["claude"]["skipPermissionsDefault"] = json!(true);
    let plan = resolve(
        &json!({"cwd":dir.path(),"skipPermissions":false}),
        &config,
        None,
        dir.path(),
        "session",
        false,
    )
    .unwrap();
    assert!(!plan.full_access);
    assert!(
        resolve(
            &json!({"provider":"pi"}),
            &config,
            None,
            dir.path(),
            "session",
            false
        )
        .is_err()
    );
    assert!(
        resolve(
            &json!({"exactModel":true,"escalationScrubbed":["model"]}),
            &config,
            None,
            dir.path(),
            "session",
            false
        )
        .is_err()
    );
}

#[test]
fn malformed_launch_fields_never_turn_into_provider_defaults() {
    let dir = tempfile::tempdir().unwrap();
    for request in [
        json!({"model":17}),
        json!({"skipPermissions":"false"}),
        json!({"manager":"true"}),
        json!({"cols":-1}),
    ] {
        assert!(resolve(&request, &defaults(), None, dir.path(), "session", false).is_err());
    }
}

#[test]
fn worker_contracts_are_additive_and_invalid_result_schema_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let planned=workspacer_hub::services::spawn_plan::resolve(&serde_json::json!({"provider":"codex","cwd":dir.path(),"parentSessionId":"manager","taskId":"task","workflowStepId":"implement","resultSchema":{"type":"object"},"message":"do the task"}),&serde_json::json!({}),None,dir.path(),"worker",false).unwrap();
    let instructions = planned.request["instructions"].as_str().unwrap();
    assert!(instructions.contains("wks-result"));
    assert!(instructions.contains("wks-escalation"));
    assert_eq!(planned.request["first_message"], "do the task");
    assert_eq!(planned.metadata["taskId"], "task");
    assert!(
        workspacer_hub::services::spawn_plan::resolve(
            &serde_json::json!({"cwd":dir.path(),"resultSchema":[]}),
            &serde_json::json!({}),
            None,
            dir.path(),
            "worker",
            false
        )
        .is_err()
    );
}

#[test]
fn retained_transport_precedence_and_first_message_receipts_cover_both_engine_routes() {
    let dir = tempfile::tempdir().unwrap();
    for (provider, requested, configured, want) in [
        ("codex", Some("pty"), Some("stream"), "pty"),
        ("codex", None, Some("pty"), "pty"),
        ("codex", None, None, "stream"),
        ("claude", None, None, "stream"),
        ("codex", None, Some("sideways"), "stream"),
        ("claude", None, Some("sideways"), "pty"),
    ] {
        let mut config = defaults();
        if let Some(value) = configured {
            config[provider]["transport"] = json!(value);
        }
        let mut params = json!({"provider":provider,"cwd":dir.path(),"parentSessionId":"parent","message":"ship the thing"});
        if let Some(value) = requested {
            params["transport"] = json!(value);
        }
        let plan = resolve(&params, &config, None, dir.path(), "child", false).unwrap();
        assert_eq!(
            if provider == "codex" {
                plan.request["transport"].as_str().unwrap()
            } else if plan.endpoint == "/sessions/spawn" {
                "pty"
            } else {
                "stream"
            },
            want,
            "{provider}/{requested:?}/{configured:?}"
        );
        assert_eq!(
            plan.endpoint,
            if provider == "claude" && want == "pty" {
                "/sessions/spawn"
            } else {
                "/sessions/spawn-managed"
            }
        );
        assert_eq!(plan.request["first_message"], "ship the thing");
        assert!(
            !plan.request["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("ship the thing")
        );
        for acknowledged in [None, Some(false), Some(true)] {
            let mut response = json!({"session_id":"child"});
            if let Some(value) = acknowledged {
                response["first_message_queued"] = json!(value);
            }
            assert_eq!(
                plan.receipt(&response).unwrap()["messageQueued"],
                acknowledged.unwrap_or(false)
            );
        }
        params.as_object_mut().unwrap().remove("message");
        let plan = resolve(&params, &config, None, dir.path(), "child", false).unwrap();
        assert!(
            plan.receipt(&json!({"session_id":"child","first_message_queued":true}))
                .unwrap()
                .get("messageQueued")
                .is_none()
        );
    }
}
#[test]
fn retained_permission_default_vocabulary_and_explicit_false_precedence() {
    let dir = tempfile::tempdir().unwrap();
    for mode in [
        "",
        "default",
        "plan",
        "acceptEdits",
        "made-up-mode",
        "bypassPermissions",
        "yolo",
    ] {
        for configured_skip in [false, true] {
            for explicit in [None, Some(false), Some(true)] {
                let mut config = defaults();
                config["claude"]["skipPermissionsDefault"] = json!(configured_skip);
                config["claude"]["defaultPermissionMode"] = json!(mode);
                let mut params = json!({"cwd":dir.path()});
                if let Some(value) = explicit {
                    params["skipPermissions"] = json!(value);
                }
                let plan = resolve(&params, &config, None, dir.path(), "child", false).unwrap();
                assert_eq!(
                    plan.full_access,
                    explicit
                        .unwrap_or(configured_skip || matches!(mode, "bypassPermissions" | "yolo")),
                    "{mode}/{configured_skip}/{explicit:?}"
                );
            }
        }
    }
}
#[test]
fn retained_manager_provider_selection_and_resume_defaults_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    for configured in ["codex", "copilot", "claude", "gpt6"] {
        let mut config = defaults();
        config["agents"]["managerProvider"] = json!(configured);
        let params = json!({"cwd":dir.path(),"manager":true});
        let plan = resolve(&params, &config, None, dir.path(), "manager", false).unwrap();
        assert_eq!(
            plan.provider,
            if configured == "gpt6" {
                "claude"
            } else {
                configured
            }
        );
        assert_eq!(plan.metadata["isWakeTarget"], true);
        let explicit = resolve(
            &json!({"cwd":dir.path(),"manager":true,"provider":"claude","transport":"pty"}),
            &config,
            None,
            dir.path(),
            "manager",
            false,
        )
        .unwrap();
        assert_eq!(explicit.provider, "claude");
        assert_eq!(explicit.endpoint, "/sessions/spawn");
        let worker = resolve(
            &json!({"cwd":dir.path()}),
            &config,
            None,
            dir.path(),
            "worker",
            false,
        )
        .unwrap();
        assert_eq!(worker.provider, "claude");
        assert_eq!(worker.metadata["isWakeTarget"], false);
    }
    for provider in ["codex", "claude"] {
        let mut config = defaults();
        config["agents"]["managerProvider"] = json!(provider);
        config["agents"]["managerModels"] = json!({"claude":"sonnet","codex":"gpt-5-codex"});
        config["agents"]["managerEfforts"] = json!({"claude":"max","codex":"xhigh"});
        config["agents"]["managerContextWindows"] = json!({"claude":200000,"codex":400000});
        config["claude"]["defaultModel"] = json!("opus");
        config["claude"]["contextWindow"] = json!(1000000);
        let plan=resolve(&json!({"cwd":dir.path(),"manager":true,"transport":"stream","resumeSessionId":"old-manager"}),&config,None,dir.path(),"unused",false).unwrap();
        assert_eq!(plan.request["resume"], "old-manager");
        for key in ["model", "model_identity", "effort", "context_window"] {
            assert!(
                plan.request.get(key).is_none(),
                "{provider} resume inherited current {key}: {}",
                plan.request
            );
        }
    }
}

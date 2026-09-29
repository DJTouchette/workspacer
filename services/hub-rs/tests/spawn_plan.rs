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

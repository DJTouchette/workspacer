use serde_json::{Value, json};
use std::sync::Arc;
use workspacer_hub::services::{
    config::Config,
    library::Library,
    workflows::{WorkflowStore, validate_definition},
};
fn fixture_cwd(dir: &tempfile::TempDir) -> String {
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    workspacer_hub::services::paths::canonicalize(&project)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}
fn setup() -> (tempfile::TempDir, Arc<Config>, WorkflowStore) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Arc::new(Config::open(dir.path().join("config.yaml")));
    let service = WorkflowStore::new(dir.path().to_owned(), cfg.clone());
    (dir, cfg, service)
}
#[test]
fn definitions_seed_immutably_clone_and_enforce_revisions() {
    let (_dir, _cfg, service) = setup();
    assert_eq!(service.list().unwrap().len(), 3);
    assert!(!service.request(&json!({"op":"disable","id":"implement-review","expectedRevision":1}))["ok"].as_bool().unwrap());
    let cloned = service
        .mutate(
            "clone",
            "implement-review",
            Some(1),
            &Value::Null,
            "Customized",
        )
        .unwrap()
        .unwrap();
    let id = cloned["id"].as_str().unwrap();
    let mut edited = cloned.clone();
    edited["name"] = json!("Changed");
    let updated = service
        .mutate("update", id, Some(1), &edited, "")
        .unwrap()
        .unwrap();
    assert_eq!(updated["revision"], 2);
    let conflict =
        service.request(&json!({"op":"update","id":id,"expectedRevision":1,"definition":edited}));
    assert_eq!(conflict["code"], "conflict");
    assert_eq!(conflict["currentRevision"], 2);
    service
        .mutate("delete", id, Some(2), &Value::Null, "")
        .unwrap();
    assert_eq!(service.list().unwrap().len(), 3);
}
#[test]
fn selection_uses_cas_and_generic_config_cannot_change_it() {
    let (dir, cfg, service) = setup();
    let cwd = fixture_cwd(&dir);
    let selected = service
        .select(None, Some("direct-implementation"), 0)
        .unwrap();
    assert_eq!(selected["selectionRevision"], 1);
    assert!(
        cfg.save(
            json!({"agents":{"defaultWorkflowId":"implement-review"}}),
            true
        )
        .is_err()
    );
    let conflict = service
        .request(&json!({"op":"select","workflowId":"implement-review","expectedRevision":0}));
    assert_eq!(conflict["code"], "conflict");
    assert_eq!(conflict["currentRevision"], 1);
    service
        .select(Some(&cwd), Some("implement-review"), 1)
        .unwrap();
    assert_eq!(
        service.pin_for(&cwd, None).unwrap()["definition"]["id"],
        "implement-review"
    );
    service.select(Some(&cwd), None, 2).unwrap();
    assert_eq!(
        service.pin_for(&cwd, None).unwrap()["definition"]["id"],
        "direct-implementation"
    );
}
#[test]
fn task_pins_keep_exact_template_snapshot_when_library_changes() {
    let (dir, _cfg, service) = setup();
    let cwd = fixture_cwd(&dir);
    let pin = service.pin_for(&cwd, None).unwrap();
    let original = pin["templates"]["ship-task"]["body"].clone();
    Library::new(dir.path().to_owned()).save(&json!({"scope":"global","id":"ship-task","kind":"dispatch","title":"Changed","body":"{{task}} changed","resultSchema":{"type":"object"}})).unwrap();
    let next = service.pin_for(&cwd, None).unwrap();
    assert_ne!(pin["hash"], next["hash"]);
    assert_eq!(pin["templates"]["ship-task"]["body"], original);
    assert_eq!(next["steps"][0]["state"], "planned");
}
#[test]
fn malformed_policy_and_corrupt_store_never_silently_relax_review() {
    let (dir, _cfg, service) = setup();
    let mut definition = service
        .list()
        .unwrap()
        .into_iter()
        .find(|d| d["id"] == "implement-review")
        .unwrap();
    definition["steps"][1]["when"] = json!("material_risk");
    assert!(validate_definition(&definition).is_err());
    definition["steps"][1]["when"] = json!("always");
    definition["steps"][1]["independentOf"] = json!("later-step");
    assert!(validate_definition(&definition).is_err());
    std::fs::write(
        dir.path().join("workflow-definitions.json"),
        "{\"version\":2,\"seeded\":[],\"definitions\":[]}",
    )
    .unwrap();
    assert!(service.list().is_err());
    assert!(
        std::fs::read_to_string(dir.path().join("workflow-definitions.json"))
            .unwrap()
            .contains("\"version\":2")
    );
}

#[test]
fn ordinary_config_roundtrip_preserves_global_and_project_workflow_selections() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("config.yaml");
    std::fs::write(&file, "agents:\n  defaultWorkflowId: research\n  workflowSelectionRevision: 3\nprojects:\n  /repo:\n    workflowId: custom\n").unwrap();
    let config = Config::open(file.clone());
    let before = config.get();
    config.save(json!({"projects":{"/repo":{"label":"After native save"}},"agents":{"fleetRoot":"/work","workflowSelectionRevision":3}}), true).unwrap();
    config
        .save(
            serde_json::from_str(r#"{"agents":{"workflowSelectionRevision":3.0}}"#).unwrap(),
            true,
        )
        .unwrap();
    let reopened = Arc::new(Config::open(file));
    let restored = reopened.get();
    assert_eq!(restored["agents"]["defaultWorkflowId"], "research");
    assert_eq!(restored["agents"]["workflowSelectionRevision"], 3);
    assert_eq!(restored["projects"]["/repo"]["workflowId"], "custom");
    assert_eq!(restored["projects"]["/repo"]["label"], "After native save");
    assert_eq!(restored["agents"]["fleetRoot"], "/work");
    assert_eq!(before["agents"]["defaultWorkflowId"], "research");
    assert!(before["projects"]["/repo"].get("label").is_none());
    assert_eq!(
        restored["agents"]["workflowSelectionRevision"].as_u64(),
        Some(3)
    );
    let selections = WorkflowStore::new(root.path().into(), reopened);
    assert!(
        selections
            .select(None, Some("implement-review"), 0)
            .is_err()
    );
    assert_eq!(
        selections
            .select(None, Some("implement-review"), 3)
            .unwrap()["selectionRevision"],
        4
    );
}

#[test]
fn generic_config_cannot_remove_or_type_confuse_selected_workflows_and_refusal_is_atomic() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("config.yaml");
    std::fs::write(&file, "agents:\n  defaultWorkflowId: research\n  workflowSelectionRevision: 3\nprojects:\n  /repo:\n    workflowId: custom\n").unwrap();
    let config = Config::open(file.clone());
    let before = config.get();
    let bytes = std::fs::read(&file).unwrap();
    for owner in [false, true] {
        for patch in [
            json!({"agents":{"defaultWorkflowId":"other"}}),
            json!({"agents":{"workflowSelectionRevision":"3"}}),
            json!({"agents":{"workflowSelectionRevision":4}}),
            json!({"agents":"replacement"}),
            json!({"agents":[]}),
            json!({"projects":{}}),
            json!({"projects":{"/repo":null}}),
            json!({"projects":{"/repo":"replacement"}}),
            json!({"projects":{"/repo":{"workflowId":{"evil":true}}}}),
            json!({"projects":{"/repo":{"workflowId":"other"}}}),
        ] {
            assert!(
                config.save(patch.clone(), owner).is_err(),
                "owner={owner}: {patch}"
            );
            assert_eq!(config.get(), before, "owner={owner}: {patch}");
            assert_eq!(
                std::fs::read(&file).unwrap(),
                bytes,
                "owner={owner}: {patch}"
            );
        }
    }
}

#[test]
fn revision_equivalence_preserves_exact_integers_without_rounding_or_coercing_ids() {
    use workspacer_hub::services::config::{defaults, merge_patch};
    for (old, echoed, expected) in [
        (json!(0), json!(0.0), 0u64),
        (json!(3.0), json!(3), 3),
        (
            json!(9_007_199_254_740_991u64),
            json!(9_007_199_254_740_991f64),
            9_007_199_254_740_991,
        ),
        (
            json!(9_007_199_254_740_993u64),
            json!(9_007_199_254_740_993u64),
            9_007_199_254_740_993,
        ),
    ] {
        let mut current = defaults();
        current["agents"]["workflowSelectionRevision"] = old;
        let merged = merge_patch(
            &current,
            json!({"agents":{"workflowSelectionRevision":echoed}}),
            true,
        )
        .unwrap();
        assert_eq!(
            merged["agents"]["workflowSelectionRevision"].as_u64(),
            Some(expected)
        );
    }
    for (old, echoed) in [
        (json!(3), json!("3")),
        (json!(3), json!(3.5)),
        (json!(3), json!(4.0)),
        (json!(3), json!(true)),
        (json!(3), Value::Null),
        (
            json!(9_007_199_254_740_993u64),
            json!(9_007_199_254_740_992f64),
        ),
        (
            json!(9_007_199_254_740_992f64),
            json!(9_007_199_254_740_992f64),
        ),
        (json!(-1), json!(-1.0)),
    ] {
        let mut current = defaults();
        current["agents"]["workflowSelectionRevision"] = old;
        assert!(
            merge_patch(
                &current,
                json!({"agents":{"workflowSelectionRevision":echoed}}),
                true
            )
            .is_err()
        );
    }
    for old in [None, Some(Value::Null)] {
        let mut current = defaults();
        if let Some(old) = old {
            current["agents"]["workflowSelectionRevision"] = old;
        }
        let merged = merge_patch(
            &current,
            json!({"agents":{"workflowSelectionRevision":null}}),
            true,
        )
        .unwrap();
        assert_eq!(
            merged["agents"].get("workflowSelectionRevision"),
            current["agents"].get("workflowSelectionRevision")
        );
    }
    let mut current = defaults();
    current["agents"]["defaultWorkflowId"] = json!(3);
    assert!(merge_patch(&current, json!({"agents":{"defaultWorkflowId":3.0}}), true).is_err());
    current["projects"] = json!({"/repo":{"workflowId":3}});
    assert!(
        merge_patch(
            &current,
            json!({"projects":{"/repo":{"workflowId":3.0}}}),
            true
        )
        .is_err()
    );
}

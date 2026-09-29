use serde_json::{Value, json};
use std::sync::Arc;
use workspacer_hub::services::{
    config::Config,
    library::Library,
    workflows::{WorkflowStore, validate_definition},
};
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
    let (_dir, cfg, service) = setup();
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
        .select(Some("/project"), Some("implement-review"), 1)
        .unwrap();
    assert_eq!(
        service.pin_for("/project", None).unwrap()["definition"]["id"],
        "implement-review"
    );
    service.select(Some("/project"), None, 2).unwrap();
    assert_eq!(
        service.pin_for("/project", None).unwrap()["definition"]["id"],
        "direct-implementation"
    );
}
#[test]
fn task_pins_keep_exact_template_snapshot_when_library_changes() {
    let (dir, _cfg, service) = setup();
    let pin = service.pin_for("/project", None).unwrap();
    let original = pin["templates"]["ship-task"]["body"].clone();
    Library::new(dir.path().to_owned()).save(&json!({"scope":"global","id":"ship-task","kind":"dispatch","title":"Changed","body":"{{task}} changed","resultSchema":{"type":"object"}})).unwrap();
    let next = service.pin_for("/project", None).unwrap();
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

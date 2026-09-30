use serde_json::{Value, json};
use std::collections::BTreeMap;
use workspacer_capability_source_check::{policy::check_spawn_keys, scan};
fn inputs() -> (Value, Value) {
    (
        serde_json::from_str(include_str!("../../../contracts/spawn-parameter-keys.json")).unwrap(),
        serde_json::from_str(include_str!(
            "../../../services/hub-rs/assets/hub-vocabulary.json"
        ))
        .unwrap(),
    )
}
fn source(keys: &[String], extra: &str) -> workspacer_capability_source_check::Report {
    let reads = keys
        .iter()
        .map(|k| format!("let _=params.get({k:?});"))
        .collect::<String>();
    scan(BTreeMap::from([("src/lib.rs".into(),format!("fn install(options:Options){{options.handler(\"agents.spawn\",|_,params|async move{{{reads}{extra} Ok(())}});}}"))])).unwrap()
}
#[test]
fn canonical_spawn_registry_preserves_history_and_fails_actual_new_read_or_removed_key() {
    let (contract, historical) = inputs();
    let keys: Vec<_> = contract["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(check_spawn_keys(&source(&keys, ""), &contract, &historical).is_empty());
    let novel = source(&keys, "let _=params.get(\"newProviderFlag\");");
    assert!(
        check_spawn_keys(&novel, &contract, &historical)
            .iter()
            .any(|e| e.contains("newProviderFlag"))
    );
    let mut missing = contract.clone();
    missing["keys"]
        .as_array_mut()
        .unwrap()
        .retain(|v| v != "targetHub");
    assert!(
        check_spawn_keys(&source(&keys, ""), &missing, &historical)
            .iter()
            .any(|e| e.contains("targetHub"))
    );
    assert!(
        check_spawn_keys(&source(&[], ""), &contract, &historical)
            .iter()
            .any(|e| e.contains("population collapsed"))
    );
}
#[test]
fn canonical_spawn_registry_rejects_unreviewed_reservations_duplicates_and_lost_history() {
    let (contract, historical) = inputs();
    let keys: Vec<_> = contract["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let report = source(&keys, "");
    for field in [
        "intents",
        "op",
        "projectCwd",
        "targetHub",
        "workflowReservationToken",
    ] {
        let mut mutant = contract.clone();
        mutant["reservations"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(!check_spawn_keys(&report, &mutant, &historical).is_empty());
    }
    let mut mutant = contract.clone();
    mutant["reservations"]["targetHub"] = json!("");
    assert!(!check_spawn_keys(&report, &mutant, &historical).is_empty());
    let mut mutant = contract.clone();
    mutant["keys"][0] = mutant["keys"][1].clone();
    assert!(!check_spawn_keys(&report, &mutant, &historical).is_empty());
    let mut mutant = contract.clone();
    mutant["keys"]
        .as_array_mut()
        .unwrap()
        .retain(|v| v != "model");
    assert!(
        check_spawn_keys(&report, &mutant, &historical)
            .iter()
            .any(|e| e.contains("historical spawn key omitted: model"))
    );
}

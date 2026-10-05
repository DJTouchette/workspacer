use std::{collections::BTreeMap, path::PathBuf};
use workspacer_capability_source_check::{
    Index, read_sources, scan,
    spawn_support::{self, Policy},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn policy() -> Policy {
    serde_json::from_slice(
        &std::fs::read(root().join("contracts/spawn-parameter-support.json")).unwrap(),
    )
    .unwrap()
}
#[test]
fn current_support_has_exact_source_closure_and_inverse_exceptions() {
    let sources = read_sources(&root()).unwrap();
    let report = scan(sources.clone()).unwrap();
    let index = Index::parse(sources.clone()).unwrap();
    let policy = policy();
    assert_eq!(policy.cases.len(), 51);
    assert_eq!(
        spawn_support::check(&report, &index, &policy),
        Vec::<String>::new()
    );
    let keys: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("contracts/spawn-parameter-keys.json")).unwrap(),
    )
    .unwrap();
    for row in &policy.cases {
        assert!(
            keys["keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|key| key == &row.name)
        );
    }
    let mut middleware = sources;
    let source = middleware
        .get_mut("services/hub-rs/src/services/workflow_runtime.rs")
        .unwrap();
    *source = source.replace(
        "let params = &params;",
        "let params = &params; params.get(\"futureField\");",
    );
    assert!(
        spawn_support::check(&report, &Index::parse(middleware).unwrap(), &policy)
            .iter()
            .any(|e| e.contains("source closure"))
    );
    let mut changed = report;
    changed
        .methods
        .get_mut("agents.spawn")
        .unwrap()
        .fields
        .insert("futureField".into());
    assert!(
        spawn_support::check(&changed, &index, &policy)
            .iter()
            .any(|e| e.contains("source closure"))
    );
}
#[test]
fn stale_directional_and_semantic_exceptions_cannot_hide_changes() {
    let baseline = policy();
    assert!(spawn_support::structure(&baseline).is_empty());
    let mut p = baseline.clone();
    p.differences
        .desktop_only
        .insert("model".into(), "obsolete decline".into());
    assert!(!spawn_support::structure(&p).is_empty());
    let mut p = baseline.clone();
    p.cases
        .iter_mut()
        .find(|r| r.name == "remoteCwd")
        .unwrap()
        .rust
        .observed = true;
    assert!(!spawn_support::structure(&p).is_empty());
    let mut p = baseline.clone();
    p.cases
        .iter_mut()
        .find(|r| r.name == "model")
        .unwrap()
        .rust
        .disposition = "refused".into();
    assert!(!spawn_support::structure(&p).is_empty());
    let mut p = baseline.clone();
    p.differences
        .semantics
        .get_mut("targetHub")
        .unwrap()
        .desktop = "supported".into();
    assert!(!spawn_support::structure(&p).is_empty());
    let mut p = baseline.clone();
    p.cases[0].rust.reason.clear();
    assert!(!spawn_support::structure(&p).is_empty());
    let mut p = baseline;
    p.mirrored_supported.pop();
    assert!(!spawn_support::structure(&p).is_empty());
}
fn workflow(body: &str) -> Index {
    Index::parse(BTreeMap::from([(
        "services/hub-rs/src/services/workflow_runtime.rs".into(),
        format!("struct WorkflowRuntime;impl WorkflowRuntime{{fn admit(params:Value){{{body}}}}}"),
    )]))
    .unwrap()
}
#[test]
fn middleware_new_fields_and_dynamic_keys_are_not_a_fixed_allowlist() {
    assert!(
        spawn_support::workflow_roots(&workflow("params.get(\"futureField\");"))
            .unwrap()
            .contains("futureField")
    );
    assert!(spawn_support::workflow_roots(&workflow("params.get(computed());")).is_err());
    let fields = spawn_support::workflow_roots(&workflow(
        "[\"a\",\"b\"].iter().any(|key|text(params,key));",
    ))
    .unwrap();
    assert_eq!(fields.into_iter().collect::<Vec<_>>(), ["a", "b"]);
    assert!(
        spawn_support::workflow_roots(&workflow(
            "[\"a\"].iter().any(|key|text(params,key));params.get(other);"
        ))
        .is_err()
    );
}
#[test]
fn removed_implementation_cannot_be_replaced_by_comment_or_string() {
    let sources = read_sources(&root()).unwrap();
    let report = scan(sources.clone()).unwrap();
    let mut changed = sources;
    let source = changed
        .get_mut("services/hub-rs/src/services/spawn_plan.rs")
        .unwrap();
    let anchor = "request[\"model\"] = json!(model);";
    assert!(source.contains(anchor));
    *source=source.replace(anchor,"let decoy = \"request[\\\"model\\\"] = json!(model);\"; // request[\"model\"] = json!(model);");
    let errors = spawn_support::check(&report, &Index::parse(changed).unwrap(), &policy());
    assert!(
        errors
            .iter()
            .any(|e| e.contains("support evidence changed model:")),
        "{errors:?}"
    );
}

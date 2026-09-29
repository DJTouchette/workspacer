use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};
use workspacer_capability_source_check::{
    Bound, Report,
    policy::{Policy, Surface},
    reference::Reference,
};
fn setup() -> (Policy, Surface, serde_json::Value, Report) {
    let names: Vec<_> = (0..100).map(|i| format!("actor.m{i}")).collect();
    let decisions: BTreeMap<_, _> = names
        .iter()
        .map(|m| (m.clone(), "reviewed inert fixture".to_string()))
        .collect();
    let p:Policy=serde_json::from_value(json!({"pathParameters":{},"methodDecisions":decisions,"inertMethods":{},"parameterDecisions":{"actor.m0":{"path":{"kind":"path","reason":"canonical host path"}}},"dangerousNames":{"path":"path"},"pathNamespaces":[],"opaqueDecisions":{"actor.m1":{"overrides":"fixed-file pricing map"}}})).unwrap();
    let s: Surface = serde_json::from_value(
        json!({"full":&names[..40],"catalog":[],"architecturalRetirements":{}}),
    )
    .unwrap();
    let mut r = Report {
        source_files: 10,
        methods: names
            .iter()
            .map(|m| (m.clone(), Bound::default()))
            .collect(),
        ..Default::default()
    };
    r.methods
        .get_mut("actor.m0")
        .unwrap()
        .fields
        .insert("path".into());
    let opaque = r.methods.get_mut("actor.m1").unwrap();
    opaque.opaque.insert("writer".into());
    opaque.opaque_paths.insert("overrides".into());
    for (m, f) in [
        ("terminals.create", "shell"),
        ("sessions.load", "filename"),
        ("claude.profiles.update", "configDir"),
        ("sessions.terminalInput", "bytesB64"),
        ("layouts.save", "name"),
        ("sessions.save", "name"),
    ] {
        r.methods
            .entry(m.into())
            .or_default()
            .fields
            .insert(f.into());
    }
    (
        p,
        s,
        json!({"params":{"path":"path"},"stems":["path","command"]}),
        r,
    )
}
#[test]
fn rejects_missing_binding_decision_new_dangerous_spelling_and_population_collapse() {
    let (mut p, s, v, mut r) = setup();
    assert!(p.check(&r, &s, &v).errors.is_empty());
    p.parameter_decisions.remove("actor.m0");
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s.contains("unclassified caller binding actor.m0.path"))
    );
    r.methods
        .get_mut("actor.m2")
        .unwrap()
        .fields
        .insert("launchCommand".into());
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s.contains("unreviewed dangerous spelling actor.m2.launchCommand"))
    );
    r.source_files = 0;
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s.contains("population collapsed"))
    );
}
#[test]
fn unrelated_parameter_or_opaque_decision_cannot_excuse_whole_payload() {
    let (mut p, s, v, r) = setup();
    p.opaque_decisions
        .get_mut("actor.m1")
        .unwrap()
        .remove("overrides");
    p.opaque_decisions
        .get_mut("actor.m1")
        .unwrap()
        .insert("name".into(), "inert label".into());
    p.parameter_decisions.insert(
        "actor.m1".into(),
        serde_json::from_value(json!({"name":{"kind":"inert","reason":"display label"}})).unwrap(),
    );
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s == "opaque caller path lacks decision actor.m1:overrides")
    );
}
#[test]
fn legacy_reference_checks_each_binding_and_source_digest() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut reference: Reference =
        serde_json::from_slice(include_bytes!("../go-reference.json")).unwrap();
    let mut r = Report::default();
    for (method, b) in &reference.methods {
        r.methods.insert(
            method.clone(),
            Bound {
                fields: b.dangerous.iter().cloned().collect(),
                ..Default::default()
            },
        );
    }
    assert!(reference.check(&root, &r).is_empty());
    r.methods
        .get_mut("sessions.terminalInput")
        .unwrap()
        .fields
        .remove("bytesB64");
    assert!(
        reference
            .check(&root, &r)
            .iter()
            .any(|s| s == "missing original Go caller binding sessions.terminalInput.bytesB64")
    );
    reference.scanner_sha256 = "tampered".into();
    assert!(
        reference.check(&root, &r).iter().any(
            |s| s.contains("provenance changed: services/hub/cmd/brain/capspec_params_test.go")
        )
    );
    reference.dangerous_bindings = 1;
    assert!(
        reference
            .check(&root, &r)
            .iter()
            .any(|s| s.contains("population changed"))
    );
}

#[test]
fn source_specific_spellings_need_real_fields_and_valid_kinds() {
    let (mut p, s, v, mut r) = setup();
    p.source_parameter_decisions.insert(
        "actor.m2".into(),
        serde_json::from_value(
            json!({"launchCommand":{"kind":"executable","reason":"explicit caller argument"}}),
        )
        .unwrap(),
    );
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s.contains("source decision lacks actual binding"))
    );
    r.methods
        .get_mut("actor.m2")
        .unwrap()
        .fields
        .insert("launchCommand".into());
    assert!(p.check(&r, &s, &v).errors.is_empty());
    p.source_parameter_decisions
        .get_mut("actor.m2")
        .unwrap()
        .get_mut("launchCommand")
        .unwrap()
        .kind = "unknown".into();
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s.contains("unreviewed dangerous spelling"))
    );
}
#[test]
fn validation_only_key_review_cannot_excuse_payload_storage_or_forwarding() {
    let (mut p, s, v, mut r) = setup();
    p.opaque_decisions.clear();
    p.inspection_decisions.insert(
        "actor.m1".into(),
        BTreeMap::from([("overrides".into(), "key validation only".into())]),
    );
    let b = r.methods.get_mut("actor.m1").unwrap();
    b.opaque.clear();
    b.opaque.insert("fixture Value map/array overrides".into());
    b.key_inspections.insert("overrides".into());
    assert!(p.check(&r, &s, &v).errors.is_empty());
    r.methods
        .get_mut("actor.m1")
        .unwrap()
        .opaque_transforms
        .insert("overrides".into());
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s == "opaque caller path lacks decision actor.m1:overrides")
    );
    let b = r.methods.get_mut("actor.m1").unwrap();
    b.opaque_transforms.clear();
    b.key_inspections.clear();
    assert!(
        p.check(&r, &s, &v)
            .errors
            .iter()
            .any(|s| s.contains("opaque caller path lacks decision"))
    );
}

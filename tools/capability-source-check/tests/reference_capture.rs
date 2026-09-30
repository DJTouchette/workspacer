use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use workspacer_capability_source_check::{Bound, Report, read_sources, reference::Reference};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "wks-reference-capture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let actual = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for file in [
            "tools/capability-source-check/go-reference.json",
            "tools/capability-source-check/go-reference-provenance.json",
            "apps/desktop/tests/fixtures/capability-parameter-vocabulary.json",
        ] {
            write(&path, file, &std::fs::read(actual.join(file)).unwrap());
        }
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn write(root: &Path, file: &str, bytes: &[u8]) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
fn original() -> Reference {
    serde_json::from_slice(include_bytes!("../go-reference.json")).unwrap()
}
fn report(r: &Reference) -> Report {
    let mut report = Report::default();
    for (m, b) in &r.methods {
        report.methods.insert(
            m.clone(),
            Bound {
                fields: b.dangerous.iter().cloned().collect(),
                ..Default::default()
            },
        );
    }
    report
}
#[test]
fn only_sealed_historical_absence_is_allowed_and_present_bytes_still_verify() {
    let fixture = Fixture::new();
    let reference = original();
    let report = report(&reference);
    assert!(!fixture.0.join("services/hub").exists());
    assert!(reference.check(&fixture.0, &report).is_empty());
    let (path, _) = reference.sources.first_key_value().unwrap();
    write(&fixture.0, path, b"changed original");
    assert!(
        reference
            .check(&fixture.0, &report)
            .iter()
            .any(|e| e.contains("provenance changed"))
    );
}
#[test]
fn corrupt_or_missing_capture_and_manifest_never_authorize_absence() {
    for file in [
        "tools/capability-source-check/go-reference.json",
        "tools/capability-source-check/go-reference-provenance.json",
    ] {
        for corrupt in [false, true] {
            let fixture = Fixture::new();
            let reference = original();
            let report = report(&reference);
            if corrupt {
                write(&fixture.0, file, b"{}");
            } else {
                std::fs::remove_file(fixture.0.join(file)).unwrap();
            }
            let errors = reference.check(&fixture.0, &report);
            assert!(!errors.is_empty());
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("Go reference unavailable:")),
                "{errors:?}"
            );
        }
    }
}
#[test]
fn unknown_missing_source_and_binding_mutations_are_not_part_of_the_capture() {
    let fixture = Fixture::new();
    let mut reference = original();
    let report = report(&reference);
    reference
        .sources
        .insert("services/hub/unknown.go".into(), "0".repeat(64));
    let errors = reference.check(&fixture.0, &report);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("differs from sealed capture"))
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("unavailable: services/hub/unknown.go"))
    );
    let mut reference = original();
    reference
        .methods
        .get_mut("sessions.terminalInput")
        .unwrap()
        .dangerous[0] = "invented".into();
    assert!(
        reference
            .check(&fixture.0, &report)
            .iter()
            .any(|e| e.contains("differs from sealed capture"))
    );
}
#[test]
fn captured_mode_still_requires_live_vocabulary_and_current_rust_owners() {
    let fixture = Fixture::new();
    let reference = original();
    let report = report(&reference);
    std::fs::remove_file(
        fixture
            .0
            .join("apps/desktop/tests/fixtures/capability-parameter-vocabulary.json"),
    )
    .unwrap();
    assert!(
        reference
            .check(&fixture.0, &report)
            .iter()
            .any(|e| e.contains("live vocabulary unavailable"))
    );
    assert!(
        read_sources(&fixture.0).is_err(),
        "missing Rust checkout must not become a captured reference"
    );
    assert!(
        reference
            .check(&fixture.0, &Report::default())
            .iter()
            .any(|e| e.contains("missing original Go caller binding"))
    );
}
#[test]
fn explicit_historical_verification_refuses_unpinned_or_incomplete_checkout() {
    let fixture = Fixture::new();
    let reference = original();
    let errors = reference.verify_historical(&fixture.0, &fixture.0);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("does not match pinned provenance"))
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Go reference unavailable"))
    );
    std::fs::remove_file(
        fixture
            .0
            .join("apps/desktop/tests/fixtures/capability-parameter-vocabulary.json"),
    )
    .unwrap();
    assert!(
        reference
            .verify_historical(&fixture.0, &fixture.0)
            .iter()
            .any(|e| e.contains("historical vocabulary unavailable"))
    );
}

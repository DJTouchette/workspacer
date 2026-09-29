#[path = "support/repo.rs"]
mod repo;
use std::fs;
use std::path::Path;

#[test]
fn root_discovery_never_converts_missing_markers_into_a_skip() {
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path();
    assert!(repo::checked_root(&root.join("absent")).is_err());
    assert!(repo::checked_root(root).is_err());
    fs::create_dir_all(root.join("services/hub-rs")).unwrap();
    fs::write(root.join("services/hub-rs/Cargo.toml"), "[package]\n").unwrap();
    assert!(repo::checked_root(root).unwrap_err().contains("Makefile"));
    fs::write(root.join("Makefile"), "all:\n").unwrap();
    assert_eq!(
        repo::checked_root(root).unwrap(),
        root.canonicalize().unwrap()
    );
    fs::remove_file(root.join("services/hub-rs/Cargo.toml")).unwrap();
    assert!(repo::checked_root(root).unwrap_err().contains("Cargo.toml"));
    fs::create_dir(root.join("services/hub-rs/Cargo.toml")).unwrap();
    assert!(
        repo::checked_root(root).is_err(),
        "a directory is not a marker file"
    );
}

#[test]
fn runtime_fixture_reads_observe_edits_and_refuse_missing_files() {
    let root = repo::root();
    assert!(repo::read(&root, Path::new("contracts/path-containment-cases.json")).is_ok());
    assert!(
        repo::read(&root, Path::new("contracts/no-such-fixture.json"))
            .unwrap_err()
            .contains("cannot be skipped")
    );
    let sandbox = tempfile::tempdir().unwrap();
    fs::create_dir_all(sandbox.path().join("services/hub-rs")).unwrap();
    fs::write(
        sandbox.path().join("services/hub-rs/Cargo.toml"),
        "[package]\n",
    )
    .unwrap();
    fs::write(sandbox.path().join("Makefile"), "all:\n").unwrap();
    let fixture = sandbox.path().join("fixture.json");
    fs::write(&fixture, "first").unwrap();
    assert_eq!(
        repo::read(sandbox.path(), Path::new("fixture.json")).unwrap(),
        b"first"
    );
    fs::write(&fixture, "edited").unwrap();
    assert_eq!(
        repo::read(sandbox.path(), Path::new("fixture.json")).unwrap(),
        b"edited"
    );
    fs::rename(&fixture, fixture.with_extension("moved")).unwrap();
    assert!(repo::read(sandbox.path(), Path::new("fixture.json")).is_err());
    fs::remove_file(sandbox.path().join("Makefile")).unwrap();
    assert!(
        repo::read(sandbox.path(), Path::new("fixture.moved"))
            .unwrap_err()
            .contains("Makefile")
    );
}

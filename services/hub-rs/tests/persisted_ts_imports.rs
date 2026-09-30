//! Independent current TypeScript-writer bytes; not Rust-produced seed data and
//! not a claim about every historical version or unknown root-envelope field.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use workspacer_hub::services::{
    fleet_review::ReviewStore,
    manager_replacements::ReplacementState,
    task_store::{TaskStore, revision},
};
fn capture(name: &str) -> (Vec<u8>, Value) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = root.join("services/hub-rs/assets/persisted-ts");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["origin"], "current-retained-typescript-writers");
    assert_eq!(manifest["artifacts"].as_object().unwrap().len(), 3);
    assert_eq!(manifest["sources"].as_object().unwrap().len(), 6);
    for (source, expected) in manifest["sources"].as_object().unwrap() {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(root.join(source)).unwrap())
            ),
            expected.as_str().unwrap(),
            "writer source drift: {source}"
        );
    }
    let bytes = std::fs::read(directory.join(name)).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        manifest["artifacts"][name].as_str().unwrap(),
        "raw TS writer bytes drifted"
    );
    let value = serde_json::from_slice(&bytes).unwrap();
    (bytes, value)
}
#[test]
fn typescript_task_history_import_keeps_ids_metrics_and_owner_cas_across_rewrite() {
    let (bytes, original) = capture("dispatch-history.json");
    assert!(original.get("requests").is_none());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dispatch-history.json");
    std::fs::write(&file, &bytes).unwrap();
    let store = TaskStore::open(file.clone()).unwrap();
    let tasks = store.list().unwrap();
    assert_eq!(tasks.len(), 2);
    let id = original["tasks"][0]["taskId"].as_str().unwrap();
    let imported = store.task(id).unwrap().unwrap();
    assert_eq!(imported["attempts"][0]["sessionId"], "completed-worker");
    assert_eq!(imported["attempts"][0]["metrics"]["inputTokens"], 123);
    assert_eq!(imported["attempts"][0]["resultContract"], "valid");
    for task in &tasks {
        assert_eq!(task["attempts"][0]["live"], false);
        assert_eq!(task["attempts"][0]["stale"], true);
    }
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "read/import must not rewrite history"
    );
    assert!(
        store
            .mutate_owned(
                id,
                "foreign",
                "/writer-project",
                revision(&imported),
                || Ok(()),
                |_| Ok(())
            )
            .is_err()
    );
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "refusal must preserve bytes"
    );
    store
        .mutate_owned(
            id,
            "manager",
            "/writer-project",
            revision(&imported),
            || Ok(()),
            |task| {
                task["title"] = json!("reviewed title");
                Ok(())
            },
        )
        .unwrap();
    drop(store);
    let reopened = TaskStore::open(file).unwrap();
    let raw = reopened.snapshot().unwrap();
    assert_eq!(
        raw.tasks[0]["attempts"], original["tasks"][0]["attempts"],
        "nested retained execution evidence must survive an unrelated title edit"
    );
    assert_eq!(raw.tasks[1], original["tasks"][1]);
    assert_eq!(raw.tasks[0]["taskId"], id);
    assert_eq!(raw.tasks[0]["title"], "reviewed title");
    assert!(reopened.requests("foreign").unwrap().is_empty());
}
#[test]
fn typescript_git_review_import_survives_absent_worktree_and_preserves_access() {
    let (bytes, original) = capture("fleet-review.json");
    assert!(original.get("readers").is_none());
    let record = &original["records"][0];
    assert!(
        !Path::new(record["allocatedCwd"].as_str().unwrap()).exists(),
        "capture generator removes its isolated worktree"
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fleet-review.json");
    std::fs::write(&file, &bytes).unwrap();
    let store = ReviewStore::new(dir.path().into(), dir.path().join("home"));
    let mut request = json!({"ownerSessionId":"manager","workerSessionId":"review-worker","evidenceId":record["id"],"file":"result.txt"});
    let response = store.read(&request);
    assert_eq!(response["ok"], true);
    assert_eq!(response["evidence"], *record);
    assert!(
        response["evidence"]["files"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("+after retained review")
    );
    request["ownerSessionId"] = json!("foreign");
    assert_eq!(store.read(&request)["ok"], false);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "reads/refusals do not rewrite immutable capture"
    );
    store.adopt_owner("manager", "successor").unwrap();
    drop(store);
    let reopened = ReviewStore::new(dir.path().into(), dir.path().join("home"));
    for owner in ["manager", "successor"] {
        request["ownerSessionId"] = json!(owner);
        assert_eq!(reopened.read(&request)["evidence"], *record);
    }
    request["ownerSessionId"] = json!("foreign");
    assert_eq!(reopened.read(&request)["ok"], false);
    let saved: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(saved["records"], original["records"]);
}
#[test]
fn typescript_replacement_import_recovers_uncertain_delivery_without_replay() {
    let (bytes, original) = capture("manager-replacements.json");
    assert!(original.get("pendingSignatures").is_none());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("manager-replacements.json");
    std::fs::write(&file, &bytes).unwrap();
    let id = original["operations"][0]["operationId"].as_str().unwrap();
    let state = ReplacementState::open(file.clone()).unwrap();
    assert_eq!(state.get(id).unwrap(), original["operations"][0]);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "opening alone must not rewrite"
    );
    state.recover_status().unwrap();
    let recovered = state.get(id).unwrap();
    assert_eq!(recovered["phase"], "recovery-required");
    assert_eq!(recovered["bound"], false);
    assert_eq!(recovered["deliveries"][0]["status"], "uncertain");
    assert_eq!(recovered["launch"], original["operations"][0]["launch"]);
    assert_eq!(recovered["metadata"], original["operations"][0]["metadata"]);
    assert_eq!(recovered["finishes"], original["operations"][0]["finishes"]);
    assert!(!state.acknowledged("delivery"));
    assert!(state.assert_resume("old").is_err());
    assert!(state.assert_resume("new").is_err());
    assert_eq!(state.wake_target("old").unwrap(), "new");
    assert_eq!(state.launch("old").unwrap(), original["launches"]["old"]);
    assert_eq!(
        state.metadata("queued-child").unwrap(),
        original["pendingMetadata"]["queued-child"]
    );
    drop(state);
    let again = ReplacementState::open(file).unwrap();
    again.recover_status().unwrap();
    let repeat = again.get(id).unwrap();
    assert_eq!(repeat["deliveries"], recovered["deliveries"]);
    assert_eq!(repeat["phase"], "recovery-required");
    assert!(!again.acknowledged("delivery"));
}

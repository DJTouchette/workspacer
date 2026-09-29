use serde_json::{Value, json};
use workspacer_hub::services::stores::{Stores, slug};

#[test]
fn filename_slug_contract_matches_all_three_existing_variants() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../contracts/filename-slug-cases.json")).unwrap();
    assert!(
        fixture["cases"].as_array().unwrap().len() >= 17,
        "slug corpus was reduced"
    );
    for case in fixture["cases"].as_array().unwrap() {
        for variant in ["library", "layout", "session"] {
            let actual = slug(case["input"].as_str().unwrap(), variant);
            assert_eq!(
                json!(actual),
                case["expect"][variant],
                "{} ({variant})",
                case["name"]
            );
            assert_eq!(slug(&actual, variant), actual);
        }
    }
}

#[test]
fn colliding_session_names_keep_identity_and_saved_agents_are_scrubbed() {
    let dir = tempfile::tempdir().unwrap();
    let stores = Stores::new(dir.path().into());
    let first=stores.call("sessions.save",json!({"name":"Feature: Auth","agents":[{"id":"one","skipPermissions":true,"tabs":[{"panes":[{"shell":"arbitrary"}]}]},false]})).unwrap();
    let second = stores
        .call("sessions.save", json!({"name":"Feature Auth","agents":[]}))
        .unwrap();
    assert_ne!(first, second);
    let again = stores
        .call("sessions.save", json!({"name":"Feature Auth","agents":[]}))
        .unwrap();
    assert_eq!(again, second);
    let loaded = stores
        .call("sessions.load", json!({"filename":first}))
        .unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../../contracts/session-schema.json")).unwrap();
    assert_eq!(loaded["schemaVersion"], schema["version"]);
    assert!(loaded["agents"][0].get("skipPermissions").is_none());
    assert!(
        loaded["agents"][0]["tabs"][0]["panes"][0]
            .get("shell")
            .is_none()
    );
    assert_eq!(
        stores
            .call("sessions.list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn bad_yaml_is_quarantined_once_with_literal_filename_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("broken[.yaml"), "unreadable: [\n").unwrap();
    let stores = Stores::new(dir.path().into());
    for _ in 0..3 {
        assert_eq!(stores.call("sessions.list", json!({})).unwrap(), json!([]));
    }
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 2);
    assert_eq!(
        std::fs::read(path.join("broken[.yaml")).unwrap(),
        b"unreadable: [\n"
    );
}

#[test]
#[cfg(unix)]
fn collision_at_a_symlink_slot_never_falls_back_to_overwriting_the_first_session() {
    let dir = tempfile::tempdir().unwrap();
    let stores = Stores::new(dir.path().into());
    let first = stores
        .call("sessions.save", json!({"name":"Feature: Auth","agents":[]}))
        .unwrap();
    let path = dir.path().join("sessions").join(first.as_str().unwrap());
    let before = std::fs::read(&path).unwrap();
    let outside = dir.path().join("outside");
    std::fs::write(&outside, "untouched").unwrap();
    std::os::unix::fs::symlink(&outside, dir.path().join("sessions/feature-auth-2.yaml")).unwrap();
    assert!(
        stores
            .call("sessions.save", json!({"name":"Feature Auth","agents":[]}))
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
}

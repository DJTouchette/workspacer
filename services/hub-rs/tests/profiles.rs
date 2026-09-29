use serde_json::{Value, json};
use workspacer_hub::services::profiles::{Profile, Profiles};

fn seed(case: &Value) -> (tempfile::TempDir, Profiles) {
    let dir = tempfile::tempdir().unwrap();
    if !case["file"].is_null() {
        std::fs::write(
            dir.path().join("claude-profiles.json"),
            json!({"profiles":case["file"]}).to_string(),
        )
        .unwrap();
    }
    let profiles = Profiles::new(dir.path().into());
    (dir, profiles)
}
fn disk(dir: &std::path::Path) -> Vec<Profile> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(dir.join("claude-profiles.json")).unwrap()).unwrap();
    serde_json::from_value(value["profiles"].clone()).unwrap()
}

#[test]
fn shared_profile_store_contract_covers_list_add_and_mutation() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/claude-profiles-cases.json"
    ))
    .unwrap();
    for (block, minimum) in [("list", 4), ("add", 2), ("mutate", 2)] {
        assert!(fixture[block].as_array().unwrap().len() >= minimum);
    }
    for case in fixture["list"].as_array().unwrap() {
        let (dir, profiles) = seed(case);
        let expected: Vec<Profile> = serde_json::from_value(case["expectedList"].clone()).unwrap();
        assert_eq!(profiles.list(), expected, "{}", case["name"]);
        let expected: Vec<Profile> = serde_json::from_value(case["expectedFile"].clone()).unwrap();
        assert_eq!(disk(dir.path()), expected, "{}", case["name"]);
    }
    for case in fixture["add"].as_array().unwrap() {
        let (dir, profiles) = seed(case);
        let mut added = profiles
            .call("claude.profiles.add", case["add"].clone())
            .unwrap();
        let id = added["id"].as_str().unwrap().to_owned();
        assert_eq!(id.len(), 36);
        added.as_object_mut().unwrap().remove("id");
        assert_eq!(added, case["expectedAdded"], "{}", case["name"]);
        let ids: Vec<_> = disk(dir.path())
            .iter()
            .map(|p| {
                if p.id == id {
                    "<added>".to_owned()
                } else {
                    p.id.clone()
                }
            })
            .collect();
        assert_eq!(json!(ids), case["expectedFileIds"]);
    }
    for case in fixture["mutate"].as_array().unwrap() {
        let (dir, profiles) = seed(case);
        profiles.list();
        if let Some(id) = case["updateId"].as_str() {
            let result = profiles.call(
                "claude.profiles.update",
                json!({"id":id,"updates":case["update"]}),
            );
            assert_eq!(result.is_ok(), case["expectFound"].as_bool().unwrap());
        }
        if let Some(id) = case["removeId"].as_str() {
            profiles
                .call("claude.profiles.remove", json!({"id":id}))
                .unwrap();
        }
        if let Some(expected) = case.get("expectedFileIds") {
            let ids: Vec<_> = disk(dir.path()).into_iter().map(|p| p.id).collect();
            assert_eq!(json!(ids), *expected);
        }
    }
}

#[test]
fn harness_normalization_keeps_identity_specific_metadata_separate() {
    let mut profile = Profile {
        provider: "codex".into(),
        preset: " foo /bar ".into(),
        mcp_item_ids: vec!["claude-server".into()],
        token_env_var: "TOKEN".into(),
        ..Default::default()
    };
    profile.normalize();
    assert_eq!(profile.preset, "foobar");
    assert!(profile.mcp_item_ids.is_empty());
    assert!(profile.token_env_var.is_empty());
    profile.provider = "copilot".into();
    profile.weight = 99.0;
    profile.token_env_var = " TOKEN_VAR ".into();
    profile.normalize();
    assert_eq!(profile.weight, 0.0);
    assert_eq!(profile.token_env_var, "TOKEN_VAR");
    assert!(profile.preset.is_empty());
}

#[test]
fn fractional_desktop_weights_survive_read_and_update_without_losing_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claude-profiles.json");
    let original = json!({"profiles":[{"id":"fractional","name":"Weighted account","weight":0.5},{"id":"second","name":"Other account","weight":2}]});
    std::fs::write(&path, original.to_string()).unwrap();
    let profiles = Profiles::new(directory.path().into());
    let rows = profiles.call("claude.profiles.list", json!({})).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["weight"], 0.5);
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
        original
    );
    let updated = profiles
        .call(
            "desktop.claudeProfilesUpdate",
            json!({"id":"fractional","updates":{"weight":0.25}}),
        )
        .unwrap();
    assert_eq!(updated["weight"], 0.25);
    assert_eq!(profiles.list().len(), 2);
    std::fs::write(&path, b"{broken").unwrap();
    assert!(profiles.call("claude.profiles.list", json!({})).is_err());
    assert!(
        profiles
            .call(
                "claude.profiles.add",
                json!({"name":"must not replace data"})
            )
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
}

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
    #[path = "support/sweepguard.rs"]
    mod sweepguard;
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/claude-profiles-cases.json"
    ))
    .unwrap();
    let mut list = sweepguard::Tally::default();
    let mut add = sweepguard::Tally::default();
    let mut mutate = sweepguard::Tally::default();
    for case in fixture["list"].as_array().unwrap() {
        let (dir, profiles) = seed(case);
        let expected: Vec<Profile> = serde_json::from_value(case["expectedList"].clone()).unwrap();
        list.ran("other");
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
        add.ran("other");
        assert_eq!(id.len(), 36);
        assert_eq!(uuid::Uuid::parse_str(&id).unwrap().get_version_num(), 4);
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
        mutate.ran("other");
        if let Some(id) = case["updateId"].as_str() {
            let result = profiles.call(
                "claude.profiles.update",
                json!({"id":id,"updates":case["update"]}),
            );
            assert_eq!(result.is_ok(), case["expectFound"].as_bool().unwrap());
            if case["expectFound"] == true {
                assert_eq!(result.unwrap()["name"], case["update"]["name"]);
            }
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
    list.require_every("profile list", 4).unwrap();
    add.require_every("profile add", 2).unwrap();
    mutate.require_every("profile mutate", 2).unwrap();
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
fn profile_wire_lists_never_become_null_and_environment_uses_provider_home() {
    use workspacer_hub::services::profiles::environment;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claude-profiles.json");
    std::fs::write(&path, r#"{"profiles":[{"id":"old","name":"Old","isDefault":true,"extraArgs":null,"mcpItemIds":null}]}"#).unwrap();
    let profiles = Profiles::new(root.path().into());
    let rows = profiles.call("claude.profiles.list", json!({})).unwrap();
    for field in ["extraArgs", "mcpItemIds"] {
        assert_eq!(rows[0][field], json!([]));
    }
    let added = profiles
        .call("claude.profiles.add", json!({"name":"New"}))
        .unwrap();
    assert_eq!(added["configDir"], "");
    assert_eq!(added["extraArgs"], json!([]));
    assert_eq!(added["mcpItemIds"], json!([]));
    assert_eq!(added["isDefault"], false);
    for (provider, key) in [
        ("", "CLAUDE_CONFIG_DIR"),
        ("codex", "CODEX_HOME"),
        ("copilot", "COPILOT_HOME"),
    ] {
        let profile = Profile {
            provider: provider.into(),
            config_dir: " ~/account ".into(),
            ..Default::default()
        };
        let values = environment(&profile, root.path());
        assert_eq!(values.len(), 1);
        assert_eq!(values[key], root.path().join("account").to_string_lossy());
    }
    assert!(environment(&Profile::default(), root.path()).is_empty());
}

#[test]
fn profile_config_directory_respects_platform_override_and_home_fallback() {
    if let Ok(mode) = std::env::var("WKS_PROFILE_CONFIG_CHILD") {
        let root = std::path::PathBuf::from(std::env::var_os("WKS_PROFILE_CONFIG_ROOT").unwrap());
        let expected = if mode == "override" {
            root.join(if cfg!(windows) { "appdata" } else { "xdg" })
                .join("workspacer")
        } else {
            root.join("home").join(if cfg!(windows) {
                "AppData/Roaming/workspacer"
            } else {
                ".config/workspacer"
            })
        };
        assert_eq!(workspacer_hub::cli::config_directory().unwrap(), expected);
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for mode in ["override", "fallback"] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "profile_config_directory_respects_platform_override_and_home_fallback",
                "--nocapture",
            ])
            .env("WKS_PROFILE_CONFIG_CHILD", mode)
            .env("WKS_PROFILE_CONFIG_ROOT", root.path())
            .env("HOME", root.path().join("home"))
            .env("USERPROFILE", root.path().join("home"));
        if mode == "override" {
            child
                .env("APPDATA", root.path().join("appdata"))
                .env("XDG_CONFIG_HOME", root.path().join("xdg"));
        } else {
            child.env_remove("APPDATA").env_remove("XDG_CONFIG_HOME");
        }
        assert!(child.status().unwrap().success());
    }
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

#[test]
fn nullable_legacy_profile_scalars_use_zero_values_without_dropping_the_store() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("claude-profiles.json");
    std::fs::write(&path,json!({"profiles":[{"id":"nullable","name":null,"configDir":null,"extraArgs":null,"mcpItemIds":null,"isDefault":null,"weight":null,"provider":null,"preset":null,"tokenEnvVar":null}]}).to_string()).unwrap();
    let profiles = Profiles::new(root.path().into());
    let rows = profiles.call("claude.profiles.list", json!({})).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(
        rows[0],
        json!({"id":"nullable","name":"","configDir":"","extraArgs":[],"mcpItemIds":[],"isDefault":false,"weight":0})
    );
    profiles
        .call(
            "claude.profiles.update",
            json!({"id":"nullable","updates":{"name":"Preserved"}}),
        )
        .unwrap();
    assert_eq!(disk(root.path())[0].name, "Preserved");
    assert_eq!(
        serde_json::from_value::<Profile>(json!({"id":null}))
            .unwrap()
            .id,
        ""
    );
    for malformed in [
        json!({"name":17}),
        json!({"isDefault":"false"}),
        json!({"provider":[]}),
    ] {
        assert!(serde_json::from_value::<Profile>(malformed).is_err());
    }
}

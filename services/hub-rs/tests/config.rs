use serde_json::{Value, json};
use workspacer_hub::{
    model_selection::{
        ModelSelection, claude_argv_model, manager_preferences, normalize_model_input,
        normalize_model_selection,
    },
    services::config::{
        Config, deep_merge, defaults, drop_host_trusted, merge_patch, strip_top_level_block,
    },
};

#[test]
fn shared_deep_merge_and_host_trusted_contracts() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../contracts/deepmerge-cases.json")).unwrap();
    assert!(fixture["cases"].as_array().unwrap().len() >= 10);
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(
            deep_merge(&case["target"], &case["source"]),
            case["expected"],
            "{}",
            case["name"]
        );
    }
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/host-trusted-config-cases.json"
    ))
    .unwrap();
    assert!(fixture["cases"].as_array().unwrap().len() >= 13);
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(
            drop_host_trusted(case["partial"].clone()),
            case["expected"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn shared_wholesale_contract_refuses_malformed_values_without_partial_application() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/wholesale-config-paths.json"
    ))
    .unwrap();
    assert!(fixture["valueCases"].as_array().unwrap().len() >= 11);
    assert_eq!(
        fixture["paths"],
        json!(["ui.customThemes", "claude.budgets", "projects"])
    );
    let mut covered = std::collections::BTreeSet::new();
    let mut refusals = 0;
    for case in fixture["valueCases"].as_array().unwrap() {
        let path = case["path"].as_str().unwrap();
        covered.insert(path);
        assert!(case["expect"] == "refuse" || case["expect"] == "accept");
        if case["expect"] == "refuse" {
            refusals += 1;
        }
        let mut partial = json!({});
        let mut current = defaults();
        // Keep the whole-document wipe guard distinct from wholesale-map rules.
        current["ui"]["theme"] = "contract-fixture".into();
        let keys: Vec<_> = path.split('.').collect();
        if keys.len() == 1 {
            partial[keys[0]] = case["value"].clone();
            current[keys[0]] = case["current"].clone();
        } else {
            partial[keys[0]] = json!({keys[1]:case["value"]});
            current[keys[0]][keys[1]] = case["current"].clone();
        }
        let result = merge_patch(&current, partial.clone(), false);
        if case["expect"] == "refuse" {
            assert!(result.is_err(), "{}", case["name"]);
        } else {
            let result = result.unwrap();
            let pointer = format!("/{}", path.replace('.', "/"));
            assert_eq!(
                result.pointer(&pointer).unwrap(),
                &case["expected"],
                "{}",
                case["name"]
            );
        }
        let directory = tempfile::tempdir().unwrap();
        let path_on_disk = directory.path().join("config.yaml");
        std::fs::write(&path_on_disk, serde_yaml::to_string(&current).unwrap()).unwrap();
        let config = Config::open(path_on_disk.clone());
        let before = std::fs::read(&path_on_disk).unwrap();
        let saved = config.save(partial, false);
        if case["expect"] == "refuse" {
            assert!(saved.is_err(), "{}", case["name"]);
            assert_eq!(std::fs::read(&path_on_disk).unwrap(), before);
        } else {
            let saved = saved.unwrap();
            let pointer = format!("/{}", path.replace('.', "/"));
            assert_eq!(saved.pointer(&pointer).unwrap(), &case["expected"]);
            assert_eq!(
                Config::open(path_on_disk).get().pointer(&pointer).unwrap(),
                &case["expected"]
            );
        }
    }
    assert_eq!(
        covered,
        ["ui.customThemes", "claude.budgets", "projects"]
            .into_iter()
            .collect()
    );
    assert!(refusals >= 7, "wholesale refusal cases disappeared");
}

#[test]
fn shared_model_selection_and_manager_contracts() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/model-context-windows.json"
    ))
    .unwrap();
    for (block, minimum) in [
        ("selectionCases", 10),
        ("inputCases", 12),
        ("claudeArgvCases", 4),
        ("managerPreferenceCases", 6),
    ] {
        assert!(
            fixture[block].as_array().unwrap().len() >= minimum,
            "{block} corpus was reduced"
        );
    }
    for case in fixture["selectionCases"].as_array().unwrap() {
        let result = normalize_model_selection(
            case["model"].as_str().unwrap(),
            case["contextWindow"].as_u64(),
        );
        if let Some(error) = case["error"].as_str() {
            assert_eq!(result.unwrap_err().code(), error, "{}", case["name"]);
        } else {
            let selected = result.unwrap();
            assert_eq!(json!(selected.model), case["expectedModel"]);
            assert_eq!(
                json!(selected.context_window),
                case["expectedContextWindow"]
            );
        }
    }
    for case in fixture["inputCases"].as_array().unwrap() {
        let result = normalize_model_input(
            case["provider"].as_str().unwrap(),
            case["model"].as_str(),
            case["modelIdentity"].as_str(),
            case["contextWindow"].as_u64(),
        );
        if let Some(error) = case["error"].as_str() {
            assert_eq!(result.unwrap_err().code(), error, "{}", case["name"]);
        } else if case["expectedModel"].is_null() {
            assert!(result.unwrap().is_none());
        } else {
            let selected = result.unwrap().unwrap();
            assert_eq!(json!(selected.selection.model), case["expectedModel"]);
            assert_eq!(
                json!(selected.selection.context_window),
                case["expectedContextWindow"]
            );
            assert_eq!(json!(selected.legacy_model), case["expectedLegacyModel"]);
        }
    }
    for case in fixture["claudeArgvCases"].as_array().unwrap() {
        let result = claude_argv_model(&ModelSelection {
            model: case["model"].as_str().unwrap().into(),
            context_window: case["contextWindow"].as_u64(),
        });
        if let Some(error) = case["error"].as_str() {
            assert_eq!(result.unwrap_err().code(), error);
        } else {
            assert_eq!(json!(result.unwrap()), case["expected"]);
        }
    }
    for case in fixture["managerPreferenceCases"].as_array().unwrap() {
        let result = manager_preferences(&case["agents"], true);
        if let Some(error) = case["error"].as_str() {
            assert!(
                result.unwrap_err().to_string().starts_with(error),
                "{}",
                case["name"]
            );
        } else {
            let got = result.unwrap();
            for (key, expected) in [
                ("managerModels", "expectedModels"),
                ("managerEfforts", "expectedEfforts"),
                ("managerContextWindows", "expectedContexts"),
            ] {
                assert_eq!(
                    got.get(key).cloned().unwrap_or(json!({})),
                    case[expected],
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn unreadable_config_stays_recoverable_and_missing_loaded_config_is_not_reseeded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "ui: [broken\n").unwrap();
    let config = Config::open(path.clone());
    let changed = config.save(json!({"ui":{"theme":"dark"}}), false).unwrap();
    assert_eq!(changed["ui"]["theme"], "dark");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ui: [broken\n");
    std::fs::write(&path, "ui:\n  theme: custom\nprojects: {}\n").unwrap();
    assert_eq!(config.reload()["ui"]["theme"], "custom");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(config.reload()["ui"]["theme"], "custom");
    config.save(json!({"ui":{"fontSize":16}}), false).unwrap();
    assert!(!path.exists());
}

#[test]
fn invalid_document_shapes_keep_original_bytes_and_quarantine_only_recoverable_documents() {
    for (raw, backups) in [
        ("", 0),
        ("# still editing\n", 0),
        ("null\n", 0),
        ("[one, two]\n", 1),
        ("scalar\n", 1),
        ("ui: [broken\n", 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, raw).unwrap();
        let config = Config::open(path.clone());
        for _ in 0..3 {
            config
                .save(json!({"ui":{"theme":"memory-only"}}), false)
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        let copies: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config.yaml.broken-")
            })
            .collect();
        assert_eq!(copies.len(), backups, "{raw:?}");
        for copy in copies {
            assert_eq!(std::fs::read_to_string(copy.path()).unwrap(), raw);
        }
        std::fs::write(&path, "ui:\n  theme: repaired\n").unwrap();
        assert_eq!(config.reload()["ui"]["theme"], "repaired");
        config.save(json!({"ui":{"fontSize":20}}), false).unwrap();
        assert_eq!(Config::open(path).get()["ui"]["fontSize"], 20);
    }
}

#[test]
fn two_writers_refresh_before_merge_and_explicit_context_null_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    let first = Config::open(path.clone());
    let second = Config::open(path.clone());
    first.save(json!({"ui":{"theme":"my-theme"},"agents":{"managerContextWindows":{"claude":1000000,"codex":272000}}}),false).unwrap();
    second
        .save(
            json!({"ui":{"fontSize":19},"agents":{"managerContextWindows":{"codex":null}}}),
            false,
        )
        .unwrap();
    let restored = Config::open(path).get();
    assert_eq!(restored["ui"]["theme"], "my-theme");
    assert_eq!(restored["ui"]["fontSize"], 19);
    assert_eq!(
        restored["agents"]["managerContextWindows"]["claude"],
        1000000
    );
    assert!(restored["agents"]["managerContextWindows"]["codex"].is_null());
}

#[test]
fn retired_block_pruning_preserves_unrelated_comments_and_quoted_keys() {
    let text = "# keep\r\nui:\r\n  theme: dark\r\nsupervisor:\r\n  old: true\r\n\r\nagents: {}\r\n";
    assert_eq!(
        strip_top_level_block(text, "supervisor"),
        "# keep\r\nui:\r\n  theme: dark\r\nagents: {}\r\n"
    );
    let text = "'supervisor': { old: true }\nui: {}\n";
    assert_eq!(strip_top_level_block(text, "supervisor"), text);
}

#[test]
fn retired_key_cleanup_preserves_disk_bytes_and_unknown_keys_across_reload() {
    for (raw, expected) in [
        (
            "ui:\n  theme: nord\nsupervisor:\n  provider: claude\n",
            "ui:\n  theme: nord\n",
        ),
        (
            "ui:\n  theme: nord\nsupervisor:\n  provider: claude",
            "ui:\n  theme: nord",
        ),
        (
            "supervisor: {provider: claude}\nui:\n  theme: nord\n",
            "ui:\n  theme: nord\n",
        ),
        ("supervisor:\nui:\n  theme: nord\n", "ui:\n  theme: nord\n"),
        (
            "ui:\r\n  theme: nord\r\nsupervisor:\r\n  provider: claude\r\n",
            "ui:\r\n  theme: nord\r\n",
        ),
        (
            "\"supervisor\": {provider: claude}\nui: {}\n",
            "\"supervisor\": {provider: claude}\nui: {}\n",
        ),
        (
            "supervisor:\n  provider: claude\n",
            "supervisor:\n  provider: claude\n",
        ),
        (
            "# keep above\nui:\n  theme: 'nord' # inline\n\n# keep below\nsupervisor:\n  # remove inside\n  models:\n    coordinator: opus\n\nplugin: {custom: true}\n",
            "# keep above\nui:\n  theme: 'nord' # inline\n\n# keep below\nplugin: {custom: true}\n",
        ),
        (
            "agents:\n  supervisor: {custom: true}\nsupervisorLoop: {enabled: true}\n",
            "agents:\n  supervisor: {custom: true}\nsupervisorLoop: {enabled: true}\n",
        ),
        ("# only a comment\n", "# only a comment\n"),
        ("", ""),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, raw).unwrap();
        let config = Config::open(path.clone());
        let loaded = config.get();
        assert!(loaded.get("supervisor").is_none(), "{raw}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected, "{raw}");
        if raw.contains("supervisorLoop") {
            assert_eq!(loaded["supervisorLoop"]["enabled"], true);
            assert_eq!(loaded["agents"]["supervisor"]["custom"], true);
        }
        if raw.contains("plugin:") {
            assert_eq!(loaded["plugin"]["custom"], true);
        }
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        config.reload();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    }
}

#[test]
fn malformed_wholesale_save_refuses_all_changes_and_preserves_disk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../contracts/wholesale-config-paths.json"
    ))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    let config = Config::open(path.clone());
    config
        .save(
            json!({"ui":{"theme":"before"},"projects":{"existing":{"label":"Keep"}}}),
            false,
        )
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    for case in fixture["valueCases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["expect"] == "refuse")
    {
        let mut patch = json!({"ui":{"theme":"must-not-apply"}});
        let parts: Vec<_> = case["path"].as_str().unwrap().split('.').collect();
        if parts.len() == 1 {
            patch[parts[0]] = case["value"].clone();
        } else {
            if !patch[parts[0]].is_object() {
                patch[parts[0]] = json!({});
            }
            patch[parts[0]][parts[1]] = case["value"].clone();
        }
        assert!(config.save(patch, false).is_err(), "{}", case["name"]);
        assert_eq!(config.get()["ui"]["theme"], "before");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[tokio::test]
async fn config_bus_reports_refused_maps_and_roundtrips_unknown_object_settings() {
    use workspacer_hub::{Hub, Options, client::Client};
    let dir = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.config_dir = Some(dir.path().join("config"));
    options.data_dir = Some(dir.path().join("data"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let saved = client.call("config.save", json!({"ui":{"theme":"before"},"pluginSettings":{"fullAccess":true,"provider":"claude"},"projects":{"a":{"label":"A","yolo":true},"b":{"label":"B"}}})).await.unwrap();
    assert_eq!(
        saved["pluginSettings"],
        json!({"fullAccess":true,"provider":"claude"})
    );
    let path = dir.path().join("config/config.yaml");
    let before = std::fs::read(&path).unwrap();
    let error = client
        .call(
            "config.save",
            json!({"ui":{"theme":"must-not-apply"},"projects":"{}"}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("projects"), "{error:#}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let surviving = client.call("config.get", json!({})).await.unwrap();
    assert_eq!(surviving["ui"]["theme"], "before");
    assert_eq!(surviving["projects"].as_object().unwrap().len(), 2);
    client
        .call(
            "config.save",
            json!({"projects":{"a":{"label":"A","yolo":true}}}),
        )
        .await
        .unwrap();
    let restored = Config::open(path).get();
    assert_eq!(restored["projects"], json!({"a":{"label":"A","yolo":true}}));
    assert_eq!(restored["pluginSettings"], saved["pluginSettings"]);
    client.close();
    hub.shutdown().unwrap();
}

#[test]
fn first_load_warns_about_missing_established_config_but_seeds_both_cases() {
    const CHILD: &str = "WORKSPACER_CONFIG_LOSS_TEST_PATH";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = std::path::PathBuf::from(path);
        let config = Config::open(path.clone());
        assert!(path.is_file());
        assert!(config.get().is_object());
        assert_eq!(
            config.save(json!({"ui":{"theme":"saved"}}), false).unwrap()["ui"]["theme"],
            "saved"
        );
        return;
    }
    for established in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        if established {
            std::fs::write(dir.path().join("remote-token"), "fixture").unwrap();
            std::fs::write(dir.path().join("tokens.json"), "[]").unwrap();
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "first_load_warns_about_missing_established_config_but_seeds_both_cases",
                "--nocapture",
            ])
            .env(CHILD, dir.path().join("config.yaml"))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert_eq!(stderr.contains("STATE LOSS"), established, "{stderr}");
        if established {
            assert!(stderr.contains("config.yaml"));
        }
    }
}

#[test]
fn owner_config_save_still_requires_the_workflow_selection_api() {
    let mut current = defaults();
    current["agents"]["defaultWorkflowId"] = json!("existing");
    for owner in [false, true] {
        assert!(
            merge_patch(
                &current,
                json!({"agents":{"defaultWorkflowId":"forged"}}),
                owner
            )
            .is_err()
        );
    }
}

#[test]
fn integral_config_numbers_are_compatible_with_json_and_yaml_writers() {
    let patch:Value=serde_json::from_str(r#"{"claude":{"defaultModel":"opus","contextWindow":1e6},"agents":{"managerContextWindows":{"codex":272000.0}}}"#).unwrap();
    let saved = merge_patch(&defaults(), patch, false).unwrap();
    assert_eq!(saved["claude"]["contextWindow"], 1000000);
    assert_eq!(saved["agents"]["managerContextWindows"]["codex"], 272000);
    assert!(merge_patch(&defaults(), json!({"claude":{"contextWindow":1.5}}), false).is_err());
}

#[test]
fn save_merges_external_bytes_even_with_identical_timestamp_and_length() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "ui:\n  theme: old-one\n").unwrap();
    let config = Config::open(path.clone());
    assert_eq!(config.get()["ui"]["theme"], "old-one");
    let original = std::fs::metadata(&path).unwrap();
    std::fs::write(&path, "ui:\n  theme: new-one\n").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap()))
        .unwrap();
    let replaced = std::fs::metadata(&path).unwrap();
    assert_eq!(original.len(), replaced.len());
    assert_eq!(original.modified().unwrap(), replaced.modified().unwrap());
    let saved = config.save(json!({"ui":{"fontSize":19}}), false).unwrap();
    assert_eq!(saved["ui"]["theme"], "new-one");
    assert_eq!(Config::open(path).get()["ui"]["fontSize"], 19);
}

#[test]
fn lock_timeout_keeps_prior_value_and_recovers_after_release_or_stale_holder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    let lock = dir.path().join("config.yaml.lock");
    let config = Config::open(path.clone());
    config
        .save(json!({"ui":{"theme":"before"}}), false)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    std::fs::write(&lock, "other writer\n").unwrap();
    let started = std::time::Instant::now();
    let denied = config.save(json!({"ui":{"theme":"after"}}), false).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(denied["ui"]["theme"], "before");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(lock.exists(), "a live competing lock must be left alone");
    std::fs::remove_file(&lock).unwrap();
    assert_eq!(
        config.save(json!({"ui":{"theme":"after"}}), false).unwrap()["ui"]["theme"],
        "after"
    );
    assert!(!lock.exists());
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../contracts/config-lock.json")).unwrap();
    assert_eq!(contract["lockFileSuffix"], ".lock");
    let stale_ms = contract["staleMs"].as_u64().unwrap();
    std::fs::write(&lock, "crashed writer\n").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(
            std::time::SystemTime::now() - std::time::Duration::from_millis(stale_ms + 1000),
        ))
        .unwrap();
    assert_eq!(
        config
            .save(json!({"ui":{"theme":"recovered"}}), false)
            .unwrap()["ui"]["theme"],
        "recovered"
    );
    assert!(!lock.exists());
}

#[cfg(unix)]
#[test]
fn atomic_config_save_keeps_existing_readers_on_complete_prior_bytes() {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    let config = Config::open(path.clone());
    config
        .save(json!({"ui":{"theme":"before"}}), false)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut reader = std::fs::File::open(&path).unwrap();
    let old_inode = reader.metadata().unwrap().ino();
    config.save(json!({"ui":{"theme":"after"}}), false).unwrap();
    assert_ne!(std::fs::metadata(&path).unwrap().ino(), old_inode);
    let mut still_readable = Vec::new();
    reader.read_to_end(&mut still_readable).unwrap();
    assert_eq!(still_readable, before);
    assert_eq!(Config::open(path).get()["ui"]["theme"], "after");
}

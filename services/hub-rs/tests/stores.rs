use serde_json::{Value, json};
use workspacer_hub::services::stores::{Stores, slug};
#[path = "support/sweepguard.rs"]
mod sweepguard;

#[test]
fn store_writes_and_deletes_use_resolved_alias_targets() {
    for kind in ["sessions", "layouts"] {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join(kind);
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target.yaml");
        let alias = directory.join("alias.yaml");
        std::fs::write(&target, "id: alias\nname: alias\nagents: []\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &alias)
            .expect("Windows contract CI must provide symlink privilege");
        let resolved =
            workspacer_hub::services::paths::selected_path(&directory, "alias.yaml").unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
        assert!(
            !std::fs::symlink_metadata(&resolved)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let service = Stores::new(root.path().into());
        service
            .call(
                &format!("{kind}.save"),
                json!({"id":"alias","name":"alias","agents":[]}),
            )
            .unwrap();
        assert!(
            std::fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let bytes = std::fs::read_to_string(&target).unwrap();
        assert!(
            bytes.contains(if kind == "sessions" {
                "schemaVersion"
            } else {
                "createdAt"
            }),
            "{bytes}"
        );
        service
            .call(
                &format!("{kind}.delete"),
                json!({"id":"alias","filename":"alias.yaml"}),
            )
            .unwrap();
        assert!(!target.exists());
        assert!(
            std::fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn filename_slug_contract_matches_all_three_existing_variants() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../contracts/filename-slug-cases.json")).unwrap();
    assert_eq!(
        fixture["owners"]["services/hub-rs/src/services/stores.rs"],
        json!(["layout", "session"]),
        "Rust's slug owner must retain every written filename variant"
    );
    assert_eq!(
        fixture["owners"]["services/hub-rs/src/services/library.rs"],
        json!(["library"])
    );
    assert!(
        fixture["cases"].as_array().unwrap().len() >= 17,
        "slug corpus was reduced"
    );
    let mut tally = sweepguard::Tally::default();
    for case in fixture["cases"].as_array().unwrap() {
        for variant in ["library", "layout", "session"] {
            let implementation = |input: &str| {
                if variant == "library" {
                    workspacer_hub::services::library::slug(input)
                } else {
                    slug(input, variant)
                }
            };
            let actual = implementation(case["input"].as_str().unwrap());
            assert_eq!(
                json!(actual),
                case["expect"][variant],
                "{} ({variant})",
                case["name"]
            );
            assert_eq!(implementation(&actual), actual);
        }
        tally.ran("other");
    }
    tally.require_every("filename slug variants", 17).unwrap();
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
    for kind in ["sessions", "layouts"] {
        for name in ["default.yaml", "broken[.yaml", "a*.yaml", "q?.yaml"] {
            if cfg!(windows) && name.contains(['*', '?']) {
                continue;
            }
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(kind);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join(name), "unreadable: [\n").unwrap();
            std::fs::write(path.join("zzz.yaml.broken-decoy"), "decoy").unwrap();
            let stores = Stores::new(dir.path().into());
            for _ in 0..3 {
                assert_eq!(
                    stores.call(&format!("{kind}.list"), json!({})).unwrap(),
                    json!([])
                );
                // Millisecond backup names must not hide repeated copies.
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let prefix = format!("{name}.broken-");
            let copies: Vec<_> = std::fs::read_dir(&path)
                .unwrap()
                .map(Result::unwrap)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
                .collect();
            assert_eq!(copies.len(), 1, "{kind}/{name}");
            assert_eq!(std::fs::read(copies[0].path()).unwrap(), b"unreadable: [\n");
            assert_eq!(std::fs::read(path.join(name)).unwrap(), b"unreadable: [\n");
        }
    }
}

#[cfg(unix)]
#[test]
fn saved_sessions_and_layouts_replace_inodes_without_leaving_temporary_files() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let stores = Stores::new(dir.path().into());
    for (kind, name) in [("sessions", "Work"), ("layouts", "My Layout")] {
        let method = format!("{kind}.save");
        let input = json!({"name":name,"agents":[]});
        let first = stores.call(&method, input.clone()).unwrap();
        let filename = if kind == "sessions" {
            first.as_str().unwrap().to_owned()
        } else {
            format!("{}.yaml", first["id"].as_str().unwrap())
        };
        let folder = dir.path().join(kind);
        let path = folder.join(filename);
        let inode = std::fs::metadata(&path).unwrap().ino();
        stores.call(&method, input).unwrap();
        assert_ne!(std::fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(std::fs::read_dir(folder).unwrap().count(), 1);
    }
}

#[test]
fn saved_state_crud_counts_sorting_and_unidentified_files_match_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let stores = Stores::new(dir.path().into());
    let saved = stores
        .call("layouts.save", json!({"name":"My Layout","agents":[]}))
        .unwrap();
    assert_eq!(saved["id"], "my-layout");
    assert!(!saved["createdAt"].as_str().unwrap().is_empty());
    std::fs::write(dir.path().join("layouts/junk.yaml"), "name: junk\n").unwrap();
    assert_eq!(
        stores
            .call("layouts.list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    stores
        .call("layouts.delete", json!({"id":"My Layout"}))
        .unwrap();
    assert_eq!(stores.call("layouts.list", json!({})).unwrap(), json!([]));
    for (id, year) in [("old", 2020), ("new", 2024)] {
        std::fs::write(
            dir.path().join(format!("layouts/{id}.yaml")),
            format!("id: {id}\ncreatedAt: '{year}-01-01T00:00:00.000Z'\nagents: []\n"),
        )
        .unwrap();
    }
    assert_eq!(
        stores.call("layouts.list", json!({})).unwrap()[0]["id"],
        "new"
    );
    let filename = stores.call("sessions.save", json!({"name":"Work","activeAgentId":"a","agents":[{"id":"a","tabs":[{"panes":[{},{}]}]},{"id":"global","global":true,"tabs":[{"panes":[{}]}]}]})).unwrap();
    let list = stores.call("sessions.list", json!({})).unwrap();
    assert_eq!(list[0]["paneCount"], 3);
    assert_eq!(list[0]["agentCount"], 1);
    assert!(!list[0]["timestamp"].as_str().unwrap().is_empty());
    assert_eq!(
        stores
            .call("sessions.load", json!({"filename":filename}))
            .unwrap()["activeAgentId"],
        "a"
    );
    stores
        .call("sessions.delete", json!({"filename":filename}))
        .unwrap();
    assert!(
        stores
            .call("sessions.load", json!({"filename":filename}))
            .unwrap()
            .is_null()
    );
    assert_eq!(stores.call("sessions.list", json!({})).unwrap(), json!([]));
    for (name, document, count) in [
        (
            "legacy",
            json!({"tabs":[{"panes":[{}]},{"panes":[{},{}]}]}),
            3,
        ),
        ("flat", json!({"panes":[{},{}]}), 2),
    ] {
        std::fs::write(
            dir.path().join(format!("sessions/{name}.yaml")),
            serde_yaml::to_string(&document).unwrap(),
        )
        .unwrap();
        let list = stores.call("sessions.list", json!({})).unwrap();
        let row = list
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap();
        assert_eq!(row["paneCount"], count);
    }
    let unknown = dir.path().join("sessions/default.yaml");
    std::fs::write(&unknown, "{{{ not yaml").unwrap();
    assert_ne!(
        stores
            .call("sessions.save", json!({"name":"Default"}))
            .unwrap(),
        "default.yaml"
    );
    assert_eq!(std::fs::read_to_string(unknown).unwrap(), "{{{ not yaml");
    let config = dir.path().join("config.yaml");
    std::fs::write(&config, "retained").unwrap();
    assert!(
        stores
            .call("layouts.save", json!({"id":"../config","agents":[]}))
            .is_err()
    );
    stores
        .call("layouts.delete", json!({"id":"../config"}))
        .unwrap();
    assert_eq!(std::fs::read_to_string(config).unwrap(), "retained");
}

#[test]
fn store_read_write_delete_and_quarantine_respect_resolved_entry_boundaries() {
    fn link(target: &std::path::Path, path: &std::path::Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, path).unwrap();
    }
    for kind in ["sessions", "layouts"] {
        for unresolved in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let folder = dir.path().join(kind);
            let outside = dir.path().join(format!("{kind}-backup"));
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::create_dir_all(&outside).unwrap();
            let victim = outside.join("victim.yaml");
            std::fs::write(&victim, "name: precious\nagents: []\n").unwrap();
            let planted = folder.join("precious.yaml");
            link(if unresolved { &planted } else { &victim }, &planted);
            let stores = Stores::new(dir.path().into());
            assert_eq!(
                stores.call(&format!("{kind}.list"), json!({})).unwrap(),
                json!([])
            );
            if kind == "sessions" {
                assert!(
                    stores
                        .call("sessions.load", json!({"filename":"precious.yaml"}))
                        .unwrap()
                        .is_null()
                );
            }
            assert!(
                stores
                    .call(
                        &format!("{kind}.save"),
                        json!({"name":"precious","id":"precious","agents":[]})
                    )
                    .is_err()
            );
            stores
                .call(
                    &format!("{kind}.delete"),
                    json!({"filename":"precious.yaml","id":"precious"}),
                )
                .unwrap();
            assert!(
                std::fs::symlink_metadata(&planted)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(
                std::fs::read_to_string(&victim).unwrap(),
                "name: precious\nagents: []\n"
            );
            // Invalid victim bytes must never be copied into the visible store.
            std::fs::write(&victim, "[unparseable secret").unwrap();
            assert_eq!(
                stores.call(&format!("{kind}.list"), json!({})).unwrap(),
                json!([])
            );
            assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        }
    }
    // An alias that resolves inside a store remains a valid catalog entry.
    let dir = tempfile::tempdir().unwrap();
    let stores = Stores::new(dir.path().into());
    stores
        .call("layouts.save", json!({"name":"Real","agents":[]}))
        .unwrap();
    link(
        &dir.path().join("layouts/real.yaml"),
        &dir.path().join("layouts/alias.yaml"),
    );
    assert_eq!(
        stores
            .call("layouts.list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
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
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, dir.path().join("sessions/feature-auth-2.yaml")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&outside, dir.path().join("sessions/feature-auth-2.yaml"))
        .unwrap();
    assert!(
        stores
            .call("sessions.save", json!({"name":"Feature Auth","agents":[]}))
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
}

#[test]
fn selected_session_filename_shared_contract() {
    use workspacer_hub::services::paths;
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/path-containment-cases.json"
    ))
    .unwrap();
    let rows = corpus["sessionFilenames"]["cases"].as_array().unwrap();
    let mut tally = sweepguard::Tally::default();
    for row in rows {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let at = |name: &str| root.join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
        let sessions = at("config/workspacer/sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        for sub in row["tree"]["dirs"].as_array().into_iter().flatten() {
            std::fs::create_dir_all(at(sub.as_str().unwrap())).unwrap();
        }
        for (name, text) in row["tree"]["files"].as_object().into_iter().flatten() {
            let path = at(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text.as_str().unwrap()).unwrap();
        }
        for (name, target) in row["tree"]["symlinks"].as_object().into_iter().flatten() {
            let link = at(name);
            let target = at(target.as_str().unwrap());
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, link).unwrap();
            #[cfg(windows)]
            if target.is_dir() {
                std::os::windows::fs::symlink_dir(target, link).unwrap();
            } else {
                std::os::windows::fs::symlink_file(target, link).unwrap();
            }
        }
        let result = paths::selected_path(&sessions, row["filename"].as_str().unwrap());
        tally.ran(row["expect"].as_str().unwrap());
        if row["expect"] == "accept" {
            assert_eq!(
                result.unwrap(),
                at(row["resolvesTo"].as_str().unwrap()),
                "{}",
                row["name"]
            );
        } else {
            let error = result.unwrap_err().to_string();
            let expected = match row["refusedBy"].as_str().unwrap() {
                "not-a-basename" => "basename",
                "escapes-sessions-dir" => "escapes selected object",
                other => panic!("unknown refusal {other}"),
            };
            assert!(error.contains(expected), "{}: {error}", row["name"]);
            let stores = Stores::new(at("config/workspacer"));
            let params = json!({"filename":row["filename"]});
            if row["filename"] == "" {
                // The public envelope rejects absence before the underlying
                // store resolver (the legacy direct helper returned null).
                for method in ["sessions.load", "sessions.delete"] {
                    assert_eq!(
                        stores.call(method, params.clone()).unwrap_err().to_string(),
                        format!("{method} requires {{ filename }}")
                    );
                }
            } else {
                assert!(
                    stores
                        .call("sessions.load", params.clone())
                        .unwrap()
                        .is_null()
                );
                stores.call("sessions.delete", params).unwrap();
            }
            for (name, text) in row["tree"]["files"].as_object().into_iter().flatten() {
                assert_eq!(
                    std::fs::read_to_string(at(name)).unwrap(),
                    text.as_str().unwrap()
                );
            }
        }
    }
    tally
        .require_corpus("selected session filenames", 12, 3, 9)
        .unwrap();
}

#[test]
fn saved_boot_documents_report_exact_removed_fields_and_keep_usable_records() {
    let root = tempfile::tempdir().unwrap();
    let stores = Stores::new(root.path().into());
    let agents = json!([
        {"id":"kept","cwd":"/project","provider":"claude","model":"opus","note":"permissionMode is ordinary prose","escalationScrubbed":["forged"],"skipPermissions":false,"permissionMode":null,"profileId":"profile","mcpItemIds":[],"launchIntegrationId":"plugin","tabs":[false,{"id":"tab","panes":[false,{"id":"pane","type":"terminal","title":"keep","shell":"fixture-shell","initialCommand":"fixture-command","pluginId":"fixture-plugin","url":"https://example.invalid/view"}]}]},
        {"id":"clean","model":"sonnet","escalationScrubbed":["stale"]}, false, null
    ]);
    for kind in ["sessions", "layouts"] {
        let saved = stores
            .call(
                &format!("{kind}.save"),
                json!({"name":"scrub fixture","agents":agents}),
            )
            .unwrap();
        let loaded = if kind == "sessions" {
            stores
                .call("sessions.load", json!({"filename":saved}))
                .unwrap()
        } else {
            stores.call("layouts.list", json!({})).unwrap()[0].clone()
        };
        let row = &loaded["agents"][0];
        assert_eq!(
            row["escalationScrubbed"],
            json!([
                "skipPermissions",
                "permissionMode",
                "profileId",
                "mcpItemIds",
                "launchIntegrationId",
                "pane.shell",
                "pane.initialCommand",
                "pane.pluginId"
            ])
        );
        for key in [
            "skipPermissions",
            "permissionMode",
            "profileId",
            "mcpItemIds",
            "launchIntegrationId",
        ] {
            assert!(row.get(key).is_none());
        }
        let pane = &row["tabs"][1]["panes"][1];
        assert_eq!(
            pane,
            &json!({"id":"pane","type":"terminal","title":"keep","url":"https://example.invalid/view"})
        );
        assert_eq!(row["note"], "permissionMode is ordinary prose");
        assert_eq!(row["model"], "opus");
        assert_eq!(loaded["agents"][1], json!({"id":"clean","model":"sonnet"}));
        assert_eq!(loaded["agents"][2], false);
        assert!(loaded["agents"][3].is_null());
    }
}

#[test]
fn restore_field_lists_stay_in_agreement_with_desktop_and_drive_real_scrubbing() {
    use workspacer_hub::services::layout::scrub_saved_document;
    let desktop = include_str!("../../../apps/desktop/src/main/lib/bootDocumentScrub.ts");
    let rust = include_str!("../src/services/layout.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    let arrays = regex::Regex::new(r"(?s)for key in \[(.*?)\]").unwrap();
    let quoted = regex::Regex::new(r#"["']([A-Za-z][A-Za-z0-9]*)["']"#).unwrap();
    let rust_lists: Vec<Vec<String>> = arrays
        .captures_iter(rust)
        .map(|capture| {
            quoted
                .captures_iter(&capture[1])
                .map(|field| field[1].to_owned())
                .collect()
        })
        .collect();
    assert_eq!(
        rust_lists.len(),
        2,
        "review structural scrub field declarations"
    );
    let mut lists = Vec::new();
    for (index, name, minimum) in [
        (0, "SPAWN_ESCALATION_KEYS", 5),
        (1, "PANE_ESCALATION_KEYS", 3),
    ] {
        let fields = desktop
            .split_once(&format!("export const {name} = ["))
            .unwrap()
            .1
            .split_once(']')
            .unwrap()
            .0;
        let fields: Vec<String> = quoted
            .captures_iter(fields)
            .map(|field| field[1].to_owned())
            .collect();
        assert!(fields.len() >= minimum);
        assert_eq!(rust_lists[index], fields, "{name}");
        lists.push(fields);
    }
    let mut agent = json!({"id":"kept","tabs":[{"panes":[{"id":"pane"}]}]});
    for key in &lists[0] {
        agent[key] = Value::Null;
    }
    for key in &lists[1] {
        agent["tabs"][0]["panes"][0][key] = Value::Null;
    }
    let mut document = json!({"agents":[agent]});
    scrub_saved_document(&mut document);
    for key in &lists[0] {
        assert!(document["agents"][0].get(key).is_none());
    }
    for key in &lists[1] {
        assert!(
            document["agents"][0]["tabs"][0]["panes"][0]
                .get(key)
                .is_none()
        );
    }
    let expected: Vec<_> = lists[0]
        .iter()
        .cloned()
        .chain(lists[1].iter().map(|key| format!("pane.{key}")))
        .collect();
    assert_eq!(document["agents"][0]["escalationScrubbed"], json!(expected));
}

#[cfg(unix)]
#[test]
fn quarantine_copy_keeps_original_bytes_private_without_changing_the_source() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sessions");
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("private.yaml");
    let bytes = b"secret: [unreadable\n";
    std::fs::write(&source, bytes).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
    let service = Stores::new(root.path().into());
    assert_eq!(service.call("sessions.list", json!({})).unwrap(), json!([]));
    let copies: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("private.yaml.broken-")
        })
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(std::fs::read(copies[0].path()).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(copies[0].path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(source).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

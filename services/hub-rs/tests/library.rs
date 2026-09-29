use serde_json::{Value, json};
#[path = "support/sweepguard.rs"]
mod sweepguard;
use workspacer_hub::services::{
    dispatch_templates,
    library::{Library, parse, slug},
};

#[test]
fn global_library_remains_visible_through_a_symlinked_config_root() {
    let root = tempfile::tempdir().unwrap();
    let actual = root.path().join("real-config");
    let alias = root.path().join("config-alias");
    std::fs::create_dir_all(&actual).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&actual, &alias)
        .expect("Windows contract CI must provide symlink privilege");
    let service = Library::new(alias);
    service.save(&json!({"scope":"global","id":"linked-config","title":"Linked config","kind":"prompt","body":"kept"})).unwrap();
    let rows = service
        .list(&json!({"cwd":root.path().join("project"),"id":"linked-config"}))
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["title"], "Linked config");
    assert!(actual.join("library/linked-config.md").is_file());
}

#[test]
fn every_library_walker_refuses_escaped_aliases_and_retains_ordinary_items() {
    for folder in [
        ".workspacer/library",
        ".claude/agents",
        ".claude/commands",
        ".claude/skills",
    ] {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        let directory = cwd.join(folder);
        let skill = folder.ends_with("skills");
        let bad = directory.join(if skill {
            "escaped/SKILL.md"
        } else {
            "escaped.md"
        });
        let good = directory.join(if skill {
            "ordinary/SKILL.md"
        } else {
            "ordinary.md"
        });
        std::fs::create_dir_all(bad.parent().unwrap()).unwrap();
        std::fs::create_dir_all(good.parent().unwrap()).unwrap();
        let target = root.path().join("outside.md");
        std::fs::write(
            &target,
            "---\ntitle: secret\nname: secret\n---\nNEVER-EXPOSE-THIS\n",
        )
        .unwrap();
        std::fs::write(
            &good,
            "---\ntitle: ordinary-control\nname: ordinary-control\n---\nallowed\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &bad).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &bad)
            .expect("Windows contract CI must provide symlink privilege");
        let rows = Library::new(root.path().join("config"))
            .list(&json!({"cwd":cwd}))
            .unwrap();
        let text = rows.to_string();
        assert!(!text.contains("NEVER-EXPOSE-THIS"), "{folder}: {text}");
        assert!(text.contains("ordinary-control"), "{folder}: {text}");
    }
}

#[test]
fn library_writes_use_resolved_targets_without_replacing_in_store_aliases() {
    for (scope, kind, folder) in [
        ("project", "prompt", ".workspacer/library"),
        ("claude", "agent", ".claude/agents"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        let directory = cwd.join(folder);
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target.md");
        let alias = directory.join("alias.md");
        std::fs::write(&target, "old bytes").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &alias)
            .expect("Windows contract CI must provide symlink privilege");
        let service = Library::new(root.path().join("config"));
        service.save(&json!({"scope":scope,"kind":kind,"cwd":cwd,"id":"alias","title":"Alias","body":"changed target"})).unwrap();
        assert!(
            std::fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("changed target")
        );
    }
}
#[test]
fn template_parameter_shared_contract_and_strict_rendering() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/dispatch-template-params-cases.json"
    ))
    .unwrap();
    assert!(
        corpus["owners"]["services/hub-rs/src/services/dispatch_templates.rs"]
            .as_str()
            .is_some_and(|owner| !owner.is_empty())
    );
    let cases = corpus["cases"].as_array().unwrap();
    assert!(cases.len() >= 18, "dispatch parameter corpus shrank");
    let mut executed = sweepguard::Tally::default();
    for case in cases {
        assert_eq!(
            json!(dispatch_templates::parameters(
                case["template"].as_str().unwrap()
            )),
            case["expect"],
            "{}",
            case["name"]
        );
        executed.ran("other");
    }
    executed
        .require_every("dispatch parameter cases", 18)
        .unwrap();
    let text = "{{task}} in {{cwd}} from {{projectCwd}}; {{delivery:open a PR}}";
    assert_eq!(
        dispatch_templates::render(text, &json!({"task":"implement"}), "/worktree", "/source")
            .unwrap(),
        "implement in /worktree from /source; open a PR"
    );
    assert!(dispatch_templates::render(text, &json!({"task":" "}), "", "").is_err());
    assert!(dispatch_templates::render(text, &json!({"task":"x","cwd":"spoof"}), "", "").is_err());
    assert!(dispatch_templates::render(text, &json!({"task":"x","typo":"x"}), "", "").is_err());
    assert!(dispatch_templates::render("{{x:fallback}} {{x}}", &json!({}), "", "").is_err());
}
#[test]
fn library_seeding_merge_edit_secret_roundtrip_and_selected_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let config = dir.path().join("config");
    let service = Library::new(config.clone());
    let initial = service.list(&json!({})).unwrap();
    assert_eq!(initial.as_array().unwrap().len(), 8);
    service
        .remove(&json!({"scope":"global","id":"summarize-and-plan"}))
        .unwrap();
    assert!(
        service
            .list(&json!({"id":"summarize-and-plan"}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let global=service.save(&json!({"scope":"global","id":"example","title":"Global","kind":"mcp","mcp":{"type":"stdio","command":"fixture","env":{"KEY":"private-value"}},"body":"description"})).unwrap();
    assert_eq!(global["mcp"]["env"]["KEY"], "__WKS_SECRET__");
    let mut edited = global;
    edited["title"] = json!("Edited");
    service.save(&edited).unwrap();
    let selected = service.selected_mcp(&project, &["example".into()]).unwrap();
    assert_eq!(selected["example"]["env"]["KEY"], "private-value");
    service.save(&json!({"scope":"project","cwd":project,"id":"example","title":"Project","kind":"mcp","mcp":{"type":"http","url":"http://localhost/fixture","headers":{"Authorization":"project-secret"}}})).unwrap();
    let items = service
        .list(&json!({"cwd":project,"id":"example"}))
        .unwrap();
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["title"], "Project");
    assert_eq!(
        items[0]["mcp"]["headers"]["Authorization"],
        "__WKS_SECRET__"
    );
    assert_eq!(
        service.selected_mcp(&project, &["example".into()]).unwrap()["example"]["headers"]["Authorization"],
        "project-secret"
    );
    assert!(service.list(&json!({"kind":"dispatchh"})).is_err());
}
#[test]
fn claude_basename_and_unknown_frontmatter_survive_edit() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("project");
    let skill = cwd.join(".claude/skills/My.Skill");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: Name\nallowed-tools: Read\nmodel: opus\n---\n\n  indentation\n",
    )
    .unwrap();
    let sibling = cwd.join(".claude/skills/my-skill");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(
        sibling.join("SKILL.md"),
        "---\nname: Other\n---\n\nuntouched\n",
    )
    .unwrap();
    let service = Library::new(dir.path().join("config"));
    let all = service.list(&json!({"cwd":cwd,"kind":"skill"})).unwrap();
    let ids: std::collections::BTreeSet<_> = all
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["scope"] == "claude")
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains("My.Skill") && ids.contains("my-skill"));
    let listed = service
        .list(&json!({"cwd":cwd,"kind":"skill","id":"My.Skill"}))
        .unwrap();
    assert_eq!(listed[0]["body"], "  indentation\n");
    let mut updated = listed[0].clone();
    updated["cwd"] = json!(cwd);
    updated["title"] = json!("Updated");
    service.save(&updated).unwrap();
    let (meta, _) = parse(&std::fs::read_to_string(skill.join("SKILL.md")).unwrap());
    assert_eq!(meta["allowed-tools"], "Read");
    assert_eq!(meta["model"], "opus");
    assert!(
        service
            .save(&json!({"scope":"claude","cwd":cwd,"id":"../../escape"}))
            .is_err()
    );
    assert!(
        service
            .remove(&json!({"scope":"claude","cwd":cwd,"id":"My.Skill","origin":"plugin:example"}))
            .is_err()
    );
    service
        .remove(&json!({"scope":"claude","cwd":cwd,"id":"My.Skill"}))
        .unwrap();
    assert!(!skill.exists());
    assert_eq!(
        std::fs::read_to_string(sibling.join("SKILL.md")).unwrap(),
        "---\nname: Other\n---\n\nuntouched\n"
    );
    let saved = service.save(&json!({"scope":"claude","kind":"skill","cwd":cwd,"id":"my-skill","title":"Edited","body":"new"})).unwrap();
    assert_eq!(
        saved["path"],
        json!(workspacer_hub::services::paths::canonicalize(&sibling.join("SKILL.md")).unwrap())
    );
    for id in ["..", "a/b", "../../.."] {
        assert!(service.save(&json!({"scope":"claude","kind":"skill","cwd":cwd,"id":id,"title":"Refused","body":"new"})).is_err());
    }
    assert_eq!(slug("Hello.World"), "hello-world");
}

#[test]
#[cfg(any(unix, windows))]
fn selected_library_directory_cannot_redirect_into_another_project_or_config_store() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let library = cwd.join(".workspacer/library");
    std::fs::create_dir_all(library.parent().unwrap()).unwrap();
    let service = Library::new(root.path().join("config"));
    for target in [
        root.path().join("other-project"),
        root.path().join("config/sessions"),
    ] {
        std::fs::create_dir_all(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &library).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target, &library)
            .expect("Windows contract CI must provide symlink privilege");
        assert!(service.save(&json!({"scope":"project","cwd":cwd,"id":"pwn","title":"T","kind":"prompt","body":"OWNED"})).is_err());
        assert!(!target.join("pwn.md").exists());
        #[cfg(unix)]
        std::fs::remove_file(&library).unwrap();
        #[cfg(windows)]
        std::fs::remove_dir(&library).unwrap();
    }
    service.save(&json!({"scope":"project","cwd":cwd,"id":"notes","title":"T","kind":"prompt","body":"ok"})).unwrap();
    assert!(library.join("notes.md").is_file());
    service
        .remove(&json!({"scope":"project","cwd":cwd,"id":"notes"}))
        .unwrap();
    assert!(!library.join("notes.md").exists());
}
#[test]
fn derived_symlinks_cannot_escape_semantic_library_roots() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("project");
    let lib = cwd.join(".workspacer/library");
    std::fs::create_dir_all(&lib).unwrap();
    let secret = dir.path().join("secret.md");
    std::fs::write(&secret, "never expose").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, lib.join("escape.md")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&secret, lib.join("escape.md"))
        .expect("Windows contract CI must provide symlink privilege");
    let service = Library::new(dir.path().join("config"));
    assert!(
        service
            .list(&json!({"cwd":cwd,"id":"escape"}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        service
            .save(&json!({"scope":"project","cwd":cwd,"id":"escape","body":"overwritten"}))
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(secret).unwrap(), "never expose");
}

#[tokio::test]
async fn bus_library_mutations_publish_changes_and_persist() {
    use workspacer_hub::{Hub, Options, client::Client};
    let dir = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.config_dir = Some(dir.path().join("config"));
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client
        .topics(["library.changed".into()].into())
        .await
        .unwrap();
    client
        .call(
            "library.save",
            json!({"scope":"global","id":"bus","title":"Bus","body":"persistent"}),
        )
        .await
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.topic, "library.changed");
    assert_eq!(
        client
            .call("library.list", json!({"id":"bus"}))
            .await
            .unwrap()[0]["body"],
        "persistent\n"
    );
    assert!(
        client
            .call("library.save", json!({"scope":"global","body":["wrong"]}))
            .await
            .is_err()
    );
    hub.shutdown().unwrap();
}

#[tokio::test]
async fn owned_runtime_observes_external_library_edits_and_joins_polling() {
    use std::time::Duration;
    use workspacer_hub::{Hub, Options, client::Client};
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let mut options = Options::default();
    options.config_dir = Some(config.clone());
    options = options.handler("sessions.snapshots", |_, _| async { Ok(json!([])) });
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let mut events = client.events();
    client
        .topics(["library.changed".into()].into())
        .await
        .unwrap();
    client.call("library.list", json!({})).await.unwrap();
    let path = config.join("library/edited-outside-hub.md");
    tokio::time::timeout(Duration::from_secs(7), async {
        for n in 0.. {
            std::fs::write(
                &path,
                format!("---\ntitle: External edit {n}\n---\neditor content\n"),
            )
            .unwrap();
            if let Ok(Ok(event)) =
                tokio::time::timeout(Duration::from_millis(150), events.recv()).await
            {
                assert_eq!(event.topic, "library.changed");
                assert_eq!(event.data, Some(json!({})));
                break;
            }
        }
    })
    .await
    .unwrap();
    let listed = client
        .call("library.list", json!({"id":"edited-outside-hub"}))
        .await
        .unwrap();
    assert_eq!(listed[0]["body"], "editor content\n");
    drop(client);
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || hub.shutdown()),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    // The runtime joined its watcher before releasing the owner.
    std::fs::remove_dir_all(config).unwrap();
}

#[tokio::test]
async fn dispatch_parameters_are_derived_on_save_and_list_and_filters_only_narrow() {
    use workspacer_hub::{Hub, Options, client::Client};
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let mut options = Options::default();
    options.config_dir = Some(config.clone());
    let hub = Hub::start(options).unwrap();
    hub.ready().await.unwrap();
    let client = Client::connect(&hub.handle()).await.unwrap();
    let expected = json!([{"name":"task","required":true},{"name":"delivery","required":false,"default":"open a PR"}]);
    for (id, kind, title) in [
        ("parity-ship", "dispatch", "Ship"),
        ("parity-scout", "dispatch", "Scout"),
        ("parity-notes", "prompt", "Notes"),
    ] {
        let saved=client.call("library.save",json!({"scope":"global","id":id,"title":title,"kind":kind,"body":"SHIP: {{task}}\nDeliver: {{delivery:open a PR}} in {{cwd}} from {{projectCwd}}","params":[{"name":"forged"}]})).await.unwrap();
        if kind == "dispatch" {
            assert_eq!(saved["params"], expected);
        } else {
            assert!(saved.get("params").is_none());
        }
        let source =
            std::fs::read_to_string(config.join("library").join(format!("{id}.md"))).unwrap();
        let (metadata, _) = parse(&source);
        assert!(
            metadata.get("params").is_none(),
            "derived parameters must not become authored frontmatter"
        );
    }
    let all = client.call("library.list", json!({})).await.unwrap();
    assert_eq!(
        client
            .call("library.list", json!({"kind":"","id":""}))
            .await
            .unwrap(),
        all
    );
    let dispatched = client
        .call("library.list", json!({"kind":"dispatch"}))
        .await
        .unwrap();
    let expected_filtered: Vec<_> = all
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == "dispatch")
        .cloned()
        .collect();
    assert_eq!(dispatched, json!(expected_filtered));
    let ours: Vec<_> = dispatched
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["id"].as_str())
        .filter(|id| id.starts_with("parity-"))
        .collect();
    assert_eq!(ours, ["parity-scout", "parity-ship"]);
    let ship = client
        .call("library.list", json!({"id":"parity-ship"}))
        .await
        .unwrap();
    assert_eq!(ship.as_array().unwrap().len(), 1);
    assert_eq!(ship[0]["params"], expected);
    for filter in [
        json!({"id":"parity-ship","kind":"prompt"}),
        json!({"id":"does-not-exist"}),
    ] {
        assert_eq!(
            client.call("library.list", filter).await.unwrap(),
            json!([])
        );
    }
    assert!(
        client
            .call("library.list", json!({"kind":"dispatchh"}))
            .await
            .is_err()
    );
    drop(client);
    hub.shutdown().unwrap();
}

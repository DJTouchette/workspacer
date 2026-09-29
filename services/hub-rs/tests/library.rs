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
        "global",
        ".workspacer/library",
        ".claude/agents",
        ".claude/commands",
        ".claude/skills",
    ] {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        let directory = if folder == "global" {
            root.path().join("config/library")
        } else {
            cwd.join(folder)
        };
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

#[test]
fn removing_a_skill_selects_its_directory_not_the_target_of_its_markdown_alias() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let selected = cwd.join(".claude/skills/selected");
    let other = cwd.join(".claude/skills/other");
    std::fs::create_dir_all(&selected).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("SKILL.md"), "other skill").unwrap();
    std::fs::write(other.join("keep.txt"), "other resources").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(other.join("SKILL.md"), selected.join("SKILL.md")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(other.join("SKILL.md"), selected.join("SKILL.md"))
        .expect("Windows contract CI must provide symlink privilege");
    Library::new(root.path().join("config"))
        .remove(&json!({"scope":"claude","kind":"skill","cwd":cwd,"id":"selected"}))
        .unwrap();
    assert!(!selected.exists(), "the selected skill must be removed");
    assert_eq!(
        std::fs::read_to_string(other.join("SKILL.md")).unwrap(),
        "other skill"
    );
    assert_eq!(
        std::fs::read_to_string(other.join("keep.txt")).unwrap(),
        "other resources"
    );
}

#[test]
fn malformed_dispatch_schema_cannot_overwrite_an_existing_library_item() {
    let root = tempfile::tempdir().unwrap();
    let service = Library::new(root.path().join("config"));
    let valid = json!({"scope":"global","id":"kept","title":"Kept","kind":"dispatch","body":"{{task}}","resultSchema":{"type":"object"}});
    let saved = service.save(&valid).unwrap();
    let path = std::path::Path::new(saved["path"].as_str().unwrap());
    let bytes = std::fs::read(path).unwrap();
    for schema in [json!([]), json!("object"), json!(42), json!(false)] {
        let mut invalid = valid.clone();
        invalid["resultSchema"] = schema;
        invalid["body"] = json!("corrupted");
        assert!(
            service.save(&invalid).is_err(),
            "malformed schema accepted: {invalid}"
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let mut cleared = valid;
    cleared["resultSchema"] = Value::Null;
    assert!(
        service
            .save(&cleared)
            .unwrap()
            .get("resultSchema")
            .is_none()
    );
}

#[test]
fn library_seed_upgrade_preserves_edits_and_deletions_and_all_starter_contracts() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let service = Library::new(config.clone());
    let first = service.list(&json!({})).unwrap();
    let rows = first.as_array().unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[0]["title"], "Careful refactor (skill)");
    let ids: std::collections::BTreeSet<_> =
        rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 8);
    assert!(!ids.contains(""));
    for (id, role) in [
        ("ship-task", "implementer"),
        ("review-task", "reviewer"),
        ("scout-task", "scout"),
        ("two-explanations", "diagnostician"),
    ] {
        let row = rows.iter().find(|r| r["id"] == id).unwrap();
        assert_eq!(row["kind"], "dispatch");
        assert!(row["resultSchema"].is_object());
        assert!(
            row["description"]
                .as_str()
                .unwrap()
                .contains(&format!("role \"{role}\""))
        );
    }
    for (id, phrases) in [
        (
            "ship-task",
            vec![
                "{{task}}",
                "Read what a check actually printed rather than trusting its exit code",
                "Do not judge whether your own work is correct",
                "HANDOFF",
                "Leave your plan and your reasoning out of it",
            ],
        ),
        (
            "review-task",
            vec![
                "{{task}}",
                "{{handoff}}",
                "session that never saw it being done",
                "plan, its reasoning or its transcript",
                "Rank what you find by severity",
                "Do not fix it",
            ],
        ),
    ] {
        let row = rows.iter().find(|r| r["id"] == id).unwrap();
        for phrase in phrases {
            assert!(
                row["body"].as_str().unwrap().contains(phrase),
                "{id}: {phrase}"
            );
        }
    }
    let mcp = rows.iter().find(|r| r["kind"] == "mcp").unwrap();
    assert_eq!(mcp["mcp"]["command"], "npx");
    assert_eq!(mcp["mcp"]["args"].as_array().unwrap().len(), 2);
    let stamps: Vec<_> = rows
        .iter()
        .map(|r| {
            let p = std::path::PathBuf::from(r["path"].as_str().unwrap());
            let m = std::fs::metadata(&p).unwrap().modified().unwrap();
            (p, m)
        })
        .collect();
    assert_eq!(service.list(&json!({})).unwrap(), first);
    for (p, m) in &stamps {
        assert_eq!(std::fs::metadata(p).unwrap().modified().unwrap(), *m);
    }
    for (p, _) in stamps {
        std::fs::remove_file(p).unwrap();
    }
    assert_eq!(service.list(&json!({})).unwrap(), json!([]));
    // Upgrade without a marker: missing old starters represent deletion;
    // new dispatch starters are additive and existing edited bytes remain exact.
    let old = root.path().join("old");
    std::fs::create_dir_all(old.join("library")).unwrap();
    let edited = b"---\ntitle: Mine\nkind: prompt\n---\n\nedited by hand\n";
    for id in ["summarize-and-plan", "careful-refactor"] {
        std::fs::write(old.join(format!("library/{id}.md")), edited).unwrap();
    }
    Library::new(old.clone()).list(&json!({})).unwrap();
    for id in ["summarize-and-plan", "careful-refactor"] {
        assert_eq!(
            std::fs::read(old.join(format!("library/{id}.md"))).unwrap(),
            edited
        );
    }
    for id in ["context7-mcp", "make-workspacer-plugin"] {
        assert!(!old.join(format!("library/{id}.md")).exists());
    }
    for id in ["ship-task", "review-task", "scout-task", "two-explanations"] {
        assert!(old.join(format!("library/{id}.md")).is_file());
    }
}

#[test]
fn claude_agent_and_command_remove_only_the_named_file_and_never_a_directory_tree() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let service = Library::new(root.path().join("config"));
    for kind in ["agent", "command", "skill"] {
        service.save(&json!({"scope":"claude","kind":kind,"cwd":cwd,"id":"same","title":"Same","body":"kept"})).unwrap();
    }
    service
        .remove(&json!({"scope":"claude","kind":"agent","cwd":cwd,"id":"same"}))
        .unwrap();
    assert!(!cwd.join(".claude/agents/same.md").exists());
    assert!(cwd.join(".claude/commands/same.md").is_file());
    assert!(cwd.join(".claude/skills/same/SKILL.md").is_file());
    service
        .remove(&json!({"scope":"claude","kind":"command","cwd":cwd,"id":"same"}))
        .unwrap();
    assert!(!cwd.join(".claude/commands/same.md").exists());
    assert!(cwd.join(".claude/skills/same/SKILL.md").is_file());
    let tree = cwd.join(".claude/agents/tree.md");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("keep"), "kept").unwrap();
    assert!(
        service
            .remove(&json!({"scope":"claude","kind":"agent","cwd":cwd,"id":"tree"}))
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(tree.join("keep")).unwrap(), "kept");
}

#[test]
fn library_mcp_empty_secrets_and_unbacked_placeholders_remain_distinct() {
    let root = tempfile::tempdir().unwrap();
    let service = Library::new(root.path().join("config"));
    let saved=service.save(&json!({"scope":"global","id":"secret","title":"Secret","kind":"mcp","mcp":{"type":"http","url":" https://example.test/mcp ","env":{"TOKEN":"real-secret","EMPTY":""},"headers":{"Authorization":"real-header","X-Trace":""}}})).unwrap();
    assert_eq!(saved["mcp"]["env"]["TOKEN"], "__WKS_SECRET__");
    assert_eq!(saved["mcp"]["env"]["EMPTY"], "");
    assert_eq!(saved["mcp"]["headers"]["Authorization"], "__WKS_SECRET__");
    assert_eq!(saved["mcp"]["headers"]["X-Trace"], "");
    assert_eq!(saved["mcp"]["url"], "https://example.test/mcp");
    let mut edited = saved.clone();
    edited["mcp"]["env"]["NEW"] = json!("__WKS_SECRET__");
    service.save(&edited).unwrap();
    let bytes = std::fs::read_to_string(saved["path"].as_str().unwrap()).unwrap();
    assert!(bytes.contains("real-secret") && bytes.contains("real-header"));
    assert!(!bytes.contains("__WKS_SECRET__"));
    let new=service.save(&json!({"scope":"global","id":"new","title":"New","kind":"mcp","mcp":{"headers":{"Authorization":"__WKS_SECRET__"}}})).unwrap();
    assert!(
        !std::fs::read_to_string(new["path"].as_str().unwrap())
            .unwrap()
            .contains("__WKS_SECRET__")
    );
    assert!(
        !service
            .list(&json!({}))
            .unwrap()
            .to_string()
            .contains("real-secret")
    );
}

#[test]
fn hand_edited_library_metadata_keeps_public_description_and_tags_typed() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let service = Library::new(config.clone());
    std::fs::create_dir_all(config.join("library")).unwrap();
    for (id, metadata, description, tags) in [
        (
            "mixed",
            "description: 17\ntags: [one, 42, false, two, null]",
            None,
            Some(json!(["one", "two"])),
        ),
        (
            "scalar",
            "description: {wrong: shape}\ntags: wrong",
            None,
            None,
        ),
        (
            "ordinary",
            "description: readable\ntags: [one]",
            Some("readable"),
            Some(json!(["one"])),
        ),
    ] {
        std::fs::write(
            config.join(format!("library/{id}.md")),
            format!("---\ntitle: {id}\n{metadata}\n---\nbody\n"),
        )
        .unwrap();
        let rows = service.list(&json!({"id":id})).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(
            rows[0].get("description"),
            description.map(|s| json!(s)).as_ref()
        );
        assert_eq!(rows[0].get("tags"), tags.as_ref());
    }
}

#[test]
fn malformed_mcp_frontmatter_does_not_become_a_partial_launch_configuration() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let service = Library::new(config.clone());
    std::fs::create_dir_all(config.join("library")).unwrap();
    for field in [
        "type: [wrong]",
        "command: [wrong]",
        "url: {wrong: shape}",
        "args: scalar",
        "args: [good, {wrong: shape}]",
        "env: scalar",
        "env: {TOKEN: [wrong]}",
        "headers: []",
        "headers: {Authorization: [wrong]}",
    ] {
        std::fs::write(
            config.join("library/malformed.md"),
            format!("---\ntitle: Broken\nkind: mcp\nmcp:\n  {field}\n---\nnotes\n"),
        )
        .unwrap();
        let listed = service.list(&json!({"id":"malformed"})).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert!(listed[0].get("mcp").is_none(), "{field}: {}", listed[0]);
        assert!(
            service
                .selected_mcp(root.path(), &["malformed".into()])
                .unwrap()
                .is_empty(),
            "{field}"
        );
    }
}

#[test]
fn both_mutating_scopes_refuse_other_project_redirects_and_retain_the_victim() {
    for (scope, kind, folder) in [
        ("project", "prompt", ".workspacer/library"),
        ("claude", "skill", ".claude/skills"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        let other = root.path().join("other-project");
        let link = cwd.join(folder);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let victim = other.join(if scope == "project" {
            "keep.md"
        } else {
            "keep"
        });
        if scope == "project" {
            std::fs::write(&victim, "precious").unwrap();
        } else {
            std::fs::create_dir_all(&victim).unwrap();
            std::fs::write(victim.join("SKILL.md"), "precious").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&other, &link)
            .expect("Windows contract CI must provide symlink privilege");
        let service = Library::new(root.path().join("config"));
        assert!(service.save(&json!({"scope":scope,"kind":kind,"cwd":cwd,"id":"pwn","title":"T","body":"owned"})).is_err());
        assert!(
            service
                .remove(&json!({"scope":scope,"kind":kind,"cwd":cwd,"id":"keep"}))
                .is_err()
        );
        assert!(!other.join("pwn").exists() && !other.join("pwn.md").exists());
        assert_eq!(
            std::fs::read_to_string(if scope == "project" {
                victim
            } else {
                victim.join("SKILL.md")
            })
            .unwrap(),
            "precious"
        );
    }
}

#[test]
fn dispatch_frontmatter_keeps_schema_but_never_projects_spawn_arguments() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("library")).unwrap();
    std::fs::write(config.join("library/sneaky.md"),"---\ntitle: Sneaky\nkind: dispatch\ntoolScope: operator\nskipPermissions: true\ncwd: /\nmodel: opus[1m]\nworktree: true\nparams: [{name: forged}]\nresultSchema:\n  type: object\n  required: [commit]\n---\ndo {{task}}\n").unwrap();
    let listed = Library::new(config).list(&json!({"id":"sneaky"})).unwrap();
    let row = &listed[0];
    assert_eq!(row["kind"], "dispatch");
    assert_eq!(row["resultSchema"]["required"], json!(["commit"]));
    assert_eq!(row["params"], json!([{"name":"task","required":true}]));
    for key in ["toolScope", "skipPermissions", "cwd", "model", "worktree"] {
        assert!(row.get(key).is_none(), "{key}");
    }
}

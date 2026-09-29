use serde_json::{Value, json};
use workspacer_hub::services::{
    dispatch_templates,
    library::{Library, parse, slug},
};
#[test]
fn template_parameter_shared_contract_and_strict_rendering() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../contracts/dispatch-template-params-cases.json"
    ))
    .unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        assert_eq!(
            json!(dispatch_templates::parameters(
                case["template"].as_str().unwrap()
            )),
            case["expect"],
            "{}",
            case["name"]
        );
    }
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
    let service = Library::new(dir.path().join("config"));
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
    assert_eq!(slug("Hello.World"), "hello-world");
}
#[cfg(unix)]
#[test]
fn derived_symlinks_cannot_escape_semantic_library_roots() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("project");
    let lib = cwd.join(".workspacer/library");
    std::fs::create_dir_all(&lib).unwrap();
    let secret = dir.path().join("secret.md");
    std::fs::write(&secret, "never expose").unwrap();
    std::os::unix::fs::symlink(&secret, lib.join("escape.md")).unwrap();
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

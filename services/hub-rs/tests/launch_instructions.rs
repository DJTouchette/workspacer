use workspacer_hub::services::launch_instructions::{
    MANAGER_PLUGIN, ORDINARY_PLUGIN, bundle_root, launch, manager_doctrine, materialize,
    prepare_skills, skill_version,
};

fn desktop_bundle() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../apps/desktop/src/main/services/agentSkillPlugins.generated.json"
    ))
    .unwrap()
}

fn quoted(path: &std::path::Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

#[test]
fn bundle_matches_desktop_and_materializes_every_file_idempotently() {
    let home = tempfile::tempdir().unwrap();
    let bundle = desktop_bundle();
    assert_eq!(skill_version(), bundle["version"].as_str().unwrap());
    let root = materialize(home.path()).unwrap();
    assert_eq!(root, bundle_root(home.path()));
    for (relative, body) in bundle["files"].as_object().unwrap() {
        assert_eq!(
            std::fs::read_to_string(root.join(relative)).unwrap(),
            body.as_str().unwrap(),
            "{relative}"
        );
    }
    // App-owned and content-addressed: an altered or missing file is restored.
    let spawn = root.join("workspacer/skills/spawn-agent/SKILL.md");
    std::fs::write(&spawn, "tampered").unwrap();
    std::fs::remove_file(root.join("workspacer-fleet/skills/standup/SKILL.md")).unwrap();
    assert_eq!(materialize(home.path()).unwrap(), root);
    assert_eq!(
        std::fs::read_to_string(&spawn).unwrap(),
        bundle["files"]["workspacer/skills/spawn-agent/SKILL.md"]
            .as_str()
            .unwrap()
    );
    assert!(
        root.join("workspacer-fleet/skills/standup/SKILL.md")
            .is_file()
    );
    let leftovers = std::fs::read_dir(root.join("workspacer/skills/spawn-agent"))
        .unwrap()
        .count();
    assert_eq!(leftovers, 1, "no temporary files remain");
}

#[test]
fn claude_takes_the_role_plugin_by_plugin_dir_and_codex_by_skill_roots() {
    let cwd = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let root = bundle_root(home.path());

    let claude = launch("one", "claude", cwd.path(), home.path(), false);
    let plugin = root.join(ORDINARY_PLUGIN);
    assert_eq!(
        claude.skills.args,
        vec!["--plugin-dir".to_string(), plugin.to_string_lossy().into()]
    );
    assert!(claude.skills.skill_roots.is_empty());
    assert!(claude.text.contains(&quoted(&plugin.join("skills"))));
    assert!(
        claude
            .text
            .contains("spawn-agent (before spawning child agents)")
    );

    let codex = launch("two", "codex", cwd.path(), home.path(), true);
    let fleet = root.join(MANAGER_PLUGIN);
    assert!(codex.skills.args.is_empty());
    assert_eq!(
        codex.skills.skill_roots,
        vec![fleet.join("skills").to_string_lossy().into_owned()]
    );
    assert!(codex.text.contains(manager_doctrine()));
    assert!(
        codex
            .text
            .contains("standup (for an on-demand fleet status digest)")
    );
    // A manager never sees the ordinary plugin, nor the reverse.
    assert!(!codex.text.contains("spawn-agent"));
    assert!(!claude.text.contains("standup"));
    for dir in [".workspacer", ".claude", ".agents"] {
        assert!(!cwd.path().join(dir).exists(), "{dir}");
    }
}

#[test]
fn pointer_harnesses_get_each_file_and_pi_gets_nothing() {
    let cwd = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let skills = bundle_root(home.path())
        .join(ORDINARY_PLUGIN)
        .join("skills");
    for provider in ["copilot", "opencode"] {
        let launch = prepare_skills(provider, cwd.path(), home.path(), false);
        assert!(launch.args.is_empty() && launch.skill_roots.is_empty());
        for name in [
            "spawn-agent",
            "project-brief",
            "scheduled-jobs",
            "workspacer-response-cards",
        ] {
            assert!(
                launch
                    .instruction
                    .contains(&quoted(&skills.join(name).join("SKILL.md")))
            );
        }
    }
    let pi = launch("pi", "pi", cwd.path(), home.path(), false);
    assert_eq!(pi.skills, Default::default());
    assert!(!pi.text.contains("skills"));
}

#[test]
fn legacy_cleanup_removes_only_exact_project_copies() {
    let bundle = desktop_bundle();
    let body = |rel: &str| bundle["files"][rel].as_str().unwrap().to_owned();
    for manager in [false, true] {
        let cwd = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let write = |rel: &str, text: &str| {
            let path = cwd.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            ".claude/skills/spawn-agent/SKILL.md",
            &body("workspacer/skills/spawn-agent/SKILL.md"),
        );
        write(
            ".agents/skills/workspacer-response-cards/references/schema.md",
            &body("workspacer/skills/workspacer-response-cards/references/schema.md"),
        );
        write(".claude/skills/project-brief/SKILL.md", "user owned");
        write(
            ".workspacer/skills/8480e9fb4e9c36ed/scheduled-jobs/SKILL.md",
            &body("workspacer/skills/scheduled-jobs/SKILL.md"),
        );
        write(".workspacer/skills/feedface/spawn-agent/SKILL.md", "edited");
        write(".workspacer/brief.md", "keep");
        launch("session", "codex", cwd.path(), home.path(), manager);
        assert!(!cwd.path().join(".claude/skills/spawn-agent").exists());
        assert!(!cwd.path().join(".agents").join("skills").exists());
        assert_eq!(
            std::fs::read_to_string(cwd.path().join(".claude/skills/project-brief/SKILL.md"))
                .unwrap(),
            "user owned"
        );
        assert!(
            !cwd.path()
                .join(".workspacer/skills/8480e9fb4e9c36ed")
                .exists()
        );
        assert_eq!(
            std::fs::read_to_string(
                cwd.path()
                    .join(".workspacer/skills/feedface/spawn-agent/SKILL.md")
            )
            .unwrap(),
            "edited"
        );
        assert!(cwd.path().join(".workspacer/brief.md").is_file());
    }
}

#[cfg(unix)]
#[test]
fn symlinks_are_never_followed() {
    use std::os::unix::fs::symlink;
    // A symlinked bundle directory refuses materialization; the launch still
    // proceeds without skills rather than writing through the link.
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), home.path().join(".workspacer")).unwrap();
    assert!(materialize(home.path()).is_err());
    let cwd = tempfile::tempdir().unwrap();
    assert_eq!(
        prepare_skills("claude", cwd.path(), home.path(), false),
        Default::default()
    );
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);

    // Cleanup never follows a symlinked cwd or legacy parent.
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let exact = outside.path().join("skills/spawn-agent/SKILL.md");
    std::fs::create_dir_all(exact.parent().unwrap()).unwrap();
    let body = include_str!("../../../apps/desktop/assets/skills/spawn-agent/SKILL.md");
    std::fs::write(&exact, body).unwrap();
    symlink(outside.path(), cwd.path().join(".agents")).unwrap();
    let links = tempfile::tempdir().unwrap();
    let alias = links.path().join("alias");
    symlink(cwd.path(), &alias).unwrap();
    launch("manager", "codex", cwd.path(), home.path(), true);
    launch("alias", "codex", &alias, home.path(), false);
    assert_eq!(std::fs::read_to_string(exact).unwrap(), body);
}

#[test]
fn manager_doctrine_carries_the_selected_fleet_policy() {
    assert!(manager_doctrine().contains("SELECTED FLEET POLICY:"));
}

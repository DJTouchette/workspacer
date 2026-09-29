use workspacer_hub::services::launch_instructions::{
    install_skills, instructions, manager_doctrine, skill_version,
};
#[test]
fn generated_skills_match_desktop_hash_and_preserve_user_content() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    use sha2::{Digest, Sha256};
    let desktop: serde_json::Value = serde_json::from_str(include_str!(
        "../../../apps/desktop/src/main/services/agentCollaborationSkills.generated.json"
    ))
    .unwrap();
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&desktop).unwrap())
    );
    assert_eq!(skill_version(), &hash[..16]);
    let root = install_skills(dir.path(), home.path()).unwrap();
    assert!(root.join("spawn-agent/SKILL.md").is_file());
    assert_eq!(install_skills(dir.path(), home.path()).unwrap(), root);
    let user = root.join("spawn-agent/SKILL.md");
    std::fs::write(&user, "user-owned").unwrap();
    assert!(install_skills(dir.path(), home.path()).is_err());
    assert_eq!(std::fs::read_to_string(user).unwrap(), "user-owned");
    let text = instructions("one", "claude", dir.path(), home.path(), false);
    assert!(!text.contains("provides two project skills"));
    assert!(install_skills(home.path(), home.path()).is_err());
}
#[test]
fn manager_instructions_do_not_install_ordinary_skills() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let text = instructions("manager", "codex", dir.path(), home.path(), true);
    assert!(text.contains(manager_doctrine()));
    assert!(!dir.path().join(".workspacer").exists());
    assert!(manager_doctrine().contains("SELECTED FLEET POLICY:"));
}
#[cfg(unix)]
#[test]
fn symlinked_destination_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(".workspacer")).unwrap();
    assert!(install_skills(dir.path(), home.path()).is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn all_asset_bytes_and_pointer_only_instructions_match_the_bundle() {
    let cwd = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let files: serde_json::Value = serde_json::from_str(include_str!(
        "../../../apps/desktop/src/main/services/agentCollaborationSkills.generated.json"
    ))
    .unwrap();
    let text = instructions("ordinary", "codex", cwd.path(), home.path(), false);
    let root = install_skills(cwd.path(), home.path()).unwrap();
    for (relative, body) in files.as_object().unwrap() {
        assert_eq!(
            std::fs::read_to_string(root.join(relative)).unwrap(),
            body.as_str().unwrap()
        );
        assert!(text.contains(&format!("{:?}", root.join(relative))));
        assert!(!text.contains(body.as_str().unwrap()));
    }
    let pi = tempfile::tempdir().unwrap();
    assert!(
        !instructions("pi", "pi", pi.path(), home.path(), false)
            .contains("provides two project skills")
    );
    assert!(!pi.path().join(".workspacer").exists());
    assert!(install_skills(std::path::Path::new("relative"), home.path()).is_err());
    let root = cwd.path().ancestors().last().unwrap();
    assert!(install_skills(root, home.path()).is_err());
}

#[test]
fn legacy_cleanup_removes_only_exact_provider_copies_even_for_managers() {
    let files: serde_json::Value = serde_json::from_str(include_str!(
        "../../../apps/desktop/src/main/services/agentCollaborationSkills.generated.json"
    ))
    .unwrap();
    for (provider, native) in [("", ".claude"), ("claude", ".claude"), ("codex", ".agents")] {
        for manager in [false, true] {
            let cwd = tempfile::tempdir().unwrap();
            let home = tempfile::tempdir().unwrap();
            let exact = cwd.path().join(native).join("skills/spawn-agent/SKILL.md");
            let custom = cwd
                .path()
                .join(native)
                .join("skills/project-brief/SKILL.md");
            std::fs::create_dir_all(exact.parent().unwrap()).unwrap();
            std::fs::create_dir_all(custom.parent().unwrap()).unwrap();
            std::fs::write(&exact, files["spawn-agent/SKILL.md"].as_str().unwrap()).unwrap();
            std::fs::write(&custom, "user owned").unwrap();
            let text = instructions("session", provider, cwd.path(), home.path(), manager);
            assert!(!exact.exists());
            assert_eq!(std::fs::read_to_string(custom).unwrap(), "user owned");
            assert_eq!(text.contains("provides two project skills"), !manager);
            assert_eq!(cwd.path().join(".workspacer").exists(), !manager);
        }
    }
}

#[cfg(unix)]
#[test]
fn symlink_cwd_and_legacy_parent_are_never_followed() {
    use std::os::unix::fs::symlink;
    let cwd = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let links = tempfile::tempdir().unwrap();
    let alias = links.path().join("alias");
    symlink(cwd.path(), &alias).unwrap();
    assert!(install_skills(&alias, home.path()).is_err());
    let outside = tempfile::tempdir().unwrap();
    let exact = outside.path().join("skills/spawn-agent/SKILL.md");
    std::fs::create_dir_all(exact.parent().unwrap()).unwrap();
    let body = include_str!("../../../apps/desktop/assets/skills/spawn-agent/SKILL.md");
    std::fs::write(&exact, body).unwrap();
    symlink(outside.path(), cwd.path().join(".agents")).unwrap();
    instructions("manager", "codex", cwd.path(), home.path(), true);
    assert_eq!(std::fs::read_to_string(exact).unwrap(), body);
}

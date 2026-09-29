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

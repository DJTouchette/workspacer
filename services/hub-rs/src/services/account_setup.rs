//! A new login shares the primary account's assets, never its credentials.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
const DIRS: &[&str] = &[
    "projects", "skills", "agents", "commands", "plugins", "todos",
];
const FILES: &[&str] = &[
    "settings.json",
    "settings.local.json",
    "CLAUDE.md",
    "keybindings.json",
];
fn slug(name: &str) -> String {
    let value = regex::Regex::new("[^a-z0-9]+")
        .unwrap()
        .replace_all(&name.to_ascii_lowercase(), "-")
        .trim_matches('-')
        .to_owned();
    if value.is_empty() {
        "account".into()
    } else {
        value
    }
}
mod platform;
use platform::{link_dir, link_file};
pub(crate) fn create(name: &str, primary: &Path, home: &Path) -> Result<Value> {
    let source = super::profile_accounts::claude_json_path(primary, home);
    let primary = super::paths::canonicalize(&super::profile_accounts::lexical_root(primary))?;
    let accounts = primary.join("accounts");
    std::fs::create_dir_all(&accounts)?;
    // Never let an existing accounts symlink relocate credentials outside the
    // primary's protected configuration tree.
    if !super::paths::contained(&super::paths::canonicalize(&accounts)?, &primary) {
        bail!("account directory escapes the primary configuration root");
    }
    let base = slug(name);
    let mut ordinal = 1u32;
    let dir = loop {
        let dir = accounts.join(if ordinal == 1 {
            base.clone()
        } else {
            format!("{base}-{ordinal}")
        });
        match std::fs::create_dir(&dir) {
            Ok(()) => break dir,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                ordinal = ordinal
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("account suffix exhausted"))?
            }
            Err(e) => return Err(e.into()),
        }
    };
    let mut shared = Vec::new();
    let mut warnings = Vec::new();
    for &entry in DIRS {
        let target = primary.join(entry);
        if !target.exists() {
            if entry != "projects" {
                continue;
            }
            std::fs::create_dir_all(&target)?;
        }
        match link_dir(&target, &dir.join(entry)) {
            Ok(()) => shared.push(entry),
            Err(e) => warnings.push(format!("could not share {entry}/: {e}")),
        }
    }
    for &entry in FILES {
        let target = primary.join(entry);
        if !target.exists() {
            if entry != "settings.json" {
                continue;
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut file) => file.write_all(b"{}\n")?,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.into()),
            }
        }
        let link = dir.join(entry);
        let result=link_file(&target,&link).or_else(|_|std::fs::hard_link(&target,&link)).or_else(|_|{
            std::fs::copy(&target,&link).map(|_|warnings.push(format!("{entry} could not be linked and was copied — future edits to the primary copy won't propagate to this account")))
        });
        match result {
            Ok(()) => shared.push(entry),
            Err(e) => warnings.push(format!("could not share {entry}: {e}")),
        }
    }
    let original = super::profile_accounts::read(&source);
    let mut seed = json!({"hasCompletedOnboarding":true});
    for key in ["theme", "projects"] {
        if let Some(value) = original.get(key) {
            seed[key] = value.clone();
        }
    }
    super::config::atomic_bytes(
        &dir.join(".claude.json"),
        serde_json::to_vec_pretty(&seed)?.as_slice(),
    )?;
    Ok(json!({"dir":dir,"shared":shared,"warnings":warnings}))
}
pub(crate) fn primary(home: &Path) -> PathBuf {
    super::profile_accounts::root(&super::profiles::Profile::default(), home)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_links_shared_assets_but_seeds_only_nonidentity_keys() {
        let home = tempfile::tempdir().unwrap();
        let primary = home.path().join(".claude");
        std::fs::create_dir(&primary).unwrap();
        std::fs::write(home.path().join(".claude.json"),r#"{"theme":"dark","projects":{"/repo":{"hasTrustDialogAccepted":true}},"oauthAccount":{"accessToken":"secret"},"userID":"secret-id"}"#).unwrap();
        std::fs::write(primary.join(".credentials.json"), "private").unwrap();
        let first = create("Work Account!", &primary, home.path()).unwrap();
        std::fs::create_dir(home.path().join("x")).unwrap();
        let dotted = home.path().join("x/../.claude");
        let second = create("Work Account!", &dotted, home.path()).unwrap();
        let dir = Path::new(first["dir"].as_str().unwrap());
        assert!(dir.ends_with("work-account"));
        assert!(Path::new(second["dir"].as_str().unwrap()).ends_with("work-account-2"));
        assert!(dir.join("projects").is_dir());
        assert!(!dir.join(".credentials.json").exists());
        let seed = super::super::profile_accounts::read(&dir.join(".claude.json"));
        assert_eq!(seed["hasCompletedOnboarding"], true);
        assert_eq!(seed["theme"], "dark");
        let dotted_seed = super::super::profile_accounts::read(
            &Path::new(second["dir"].as_str().unwrap()).join(".claude.json"),
        );
        assert_eq!(dotted_seed["theme"], "dark");
        assert!(seed.get("oauthAccount").is_none());
        assert!(seed.get("userID").is_none());
        std::fs::write(primary.join("settings.json"), "{\"hooks\":{}}").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json")).unwrap(),
            "{\"hooks\":{}}"
        );
    }
}

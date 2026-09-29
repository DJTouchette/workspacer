//! Immutable project skills and trusted manager doctrine, shared with desktop.
use anyhow::{Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};
#[derive(Deserialize)]
struct Assets {
    version: String,
    files: BTreeMap<String, String>,
    manager: String,
}
fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/launch-instructions.json"))
            .expect("generated launch assets")
    })
}
pub fn manager_doctrine() -> &'static str {
    &assets().manager
}
pub fn skill_version() -> &'static str {
    &assets().version
}
fn directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        bail!("skill directory is not an owned directory");
    }
    Ok(())
}
/// A conflict fails the whole preflight, preserving user content. Installation
/// failure should omit the pointer, as desktop does, rather than block a launch.
fn safe_project(cwd: &Path, home: &Path) -> Result<PathBuf> {
    if !cwd.is_absolute() || fs::symlink_metadata(cwd)?.file_type().is_symlink() {
        bail!("unsafe skill cwd");
    }
    let cwd = fs::canonicalize(cwd)?;
    if cwd.parent().is_none() || fs::canonicalize(home).ok().as_ref() == Some(&cwd) {
        bail!("skills cannot be installed in home or filesystem root");
    }
    if !fs::metadata(&cwd)?.is_dir() {
        bail!("skill cwd is not a directory");
    }
    Ok(cwd)
}
pub fn install_skills(cwd: &Path, home: &Path) -> Result<PathBuf> {
    let cwd = safe_project(cwd, home)?;
    let mut root = cwd.clone();
    for part in [".workspacer", "skills", skill_version()] {
        root.push(part);
        directory(&root)?;
    }
    for (relative, content) in &assets().files {
        let relative = Path::new(relative);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            bail!("invalid bundled skill path");
        }
        let path = root.join(relative);
        // Validate/create every parent before any content is written.
        let mut parent = root.clone();
        if let Some(parts) = relative.parent() {
            for part in parts.components() {
                parent.push(part);
                directory(&parent)?;
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if !meta.is_file()
                    || meta.file_type().is_symlink()
                    || fs::read(&path)? != content.as_bytes()
                {
                    bail!("preexisting skill differs from bundled asset");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    for (relative, content) in &assets().files {
        let path = root.join(relative);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes())?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let meta = fs::symlink_metadata(&path)?;
                if !meta.is_file()
                    || meta.file_type().is_symlink()
                    || fs::read(path)? != content.as_bytes()
                {
                    bail!("skill changed during installation");
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(root)
}
// Older releases installed these exact assets into harness discovery roots.
// Remove only matching regular files, including before a manager launch; leave
// user edits and symlinked parents alone. This is migration cleanup, not a purge.
fn remove_legacy_skills(provider: &str, cwd: &Path, home: &Path) {
    let native = match provider {
        "" | "claude" => ".claude",
        "codex" => ".agents",
        _ => return,
    };
    let Ok(cwd) = safe_project(cwd, home) else {
        return;
    };
    for (relative, content) in &assets().files {
        let relative = Path::new(relative);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            continue;
        }
        let mut parent = cwd.clone();
        let parts = Path::new(native).join("skills").join(relative);
        let Some(dirs) = parts.parent() else { continue };
        if !dirs.components().all(|part| {
            parent.push(part);
            fs::symlink_metadata(&parent).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        }) {
            continue;
        }
        let path = cwd.join(parts);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
            && fs::read(&path).is_ok_and(|body| body == content.as_bytes())
            && fs::remove_file(&path).is_ok()
        {
            let _ = fs::remove_dir(parent);
        }
    }
}
pub fn instructions(
    session: &str,
    provider: &str,
    cwd: &Path,
    home: &Path,
    manager: bool,
) -> String {
    remove_legacy_skills(provider, cwd, home);
    let mut parts = vec![format!("You are running inside Workspacer session {session} with access to the local workspacer MCP facade."),"Use the workspacer MCP tools when they are relevant to the task. Your tool scope for this session is operator.".into()];
    if manager {
        parts.push(manager_doctrine().into());
    } else if provider != "pi" {
        if let Ok(root) = install_skills(cwd, home) {
            parts.push(format!("Workspacer provides two project skills: read {:?} before spawning child agents, and {:?} before maintaining the project brief.",root.join("spawn-agent/SKILL.md"),root.join("project-brief/SKILL.md")));
        }
    }
    parts.join("\n")
}

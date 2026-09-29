//! Model-authored card targets stay within their owning project's selected file.
use super::{files, fleet_review::selected_secret, paths};
use crate::Options;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn resolve(target: &str, cwd: &Path, config: &Path, home: &Path) -> Result<PathBuf> {
    if target.trim().is_empty() {
        bail!("no file given");
    }
    let root = paths::canonicalize(cwd)
        .map_err(|_| anyhow!("this project directory could not be resolved"))?;
    let target = Path::new(target);
    let joined = if target.is_absolute() {
        target.to_owned()
    } else {
        cwd.join(target)
    };
    let target =
        paths::canonicalize(&joined).map_err(|_| anyhow!("that path could not be resolved"))?;
    if !paths::contained(&target, &root) {
        bail!("that file is outside this project");
    }
    if selected_secret(&target, config, home) {
        bail!("that file holds credentials or agent configuration");
    }
    match std::fs::metadata(&target) {
        Ok(m) if m.is_file() => Ok(target),
        Ok(_) => bail!("that path is not a file"),
        Err(_) => bail!("that file no longer exists"),
    }
}
async fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut command = tokio::process::Command::new("git");
    command.current_dir(cwd);
    for item in files::GIT_NO_EXEC {
        command.args(["-c", item]);
    }
    command.args(args);
    let output =
        super::owned_process::capture(&mut command, 256 * 1024, Duration::from_secs(10)).await?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}
pub async fn read(target: &str, cwd: &Path, config: &Path, home: &Path) -> Value {
    let result = async {
        let root = paths::canonicalize(Path::new(
            git(cwd, &["rev-parse", "--show-toplevel"]).await?.trim(),
        ))
        .map_err(|_| anyhow!("Project is not a git worktree"))?;
        let target = target.to_owned();
        let cwd = cwd.to_owned();
        let config = config.to_owned();
        let home = home.to_owned();
        let (path, after) = tokio::task::spawn_blocking(move || -> Result<_> {
            let path = resolve(&target, &cwd, &config, &home)?;
            if std::fs::metadata(&path)?.len() > 256 * 1024 {
                bail!("Card diffs are limited to 256 KiB per file");
            }
            let bytes = files::bounded_bytes(&path, 256 * 1024)?;
            if bytes.contains(&0) {
                bail!("File is too large or binary");
            }
            if resolve(&target, &cwd, &config, &home)? != path {
                bail!("File moved during read");
            }
            Ok((path, String::from_utf8_lossy(&bytes).into_owned()))
        })
        .await??;
        if !paths::contained(&path, &root) {
            bail!("File is outside this git worktree");
        }
        let rel = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let entry = git(
            &root,
            &["--literal-pathspecs", "ls-tree", "-z", "HEAD", "--", &rel],
        )
        .await?;
        let before = if entry.is_empty() {
            String::new()
        } else {
            if !entry.starts_with("100") {
                bail!("Baseline is not a regular file");
            }
            let before = git(
                &root,
                &[
                    "show",
                    "--no-ext-diff",
                    "--no-textconv",
                    &format!("HEAD:{rel}"),
                ],
            )
            .await?;
            if before.contains('\0') {
                bail!("Baseline is binary");
            }
            before
        };
        Ok::<_, anyhow::Error>(json!({"ok":true,"path":path,"before":before,"after":after}))
    }
    .await;
    result.unwrap_or_else(|error| json!({"ok":false,"error":error.to_string()}))
}
pub(crate) fn install(mut options: Options) -> Options {
    if let (Some(config), Some(home)) = (options.config_dir.clone(), options.home_dir.clone()) {
        let lookup = super::local_lookup(&options);
        options = options.handler("desktop.htmlCardReadDiff", move |caller, params| {
            let config = config.clone();
            let home = home.clone();
            let lookup = lookup.clone();
            async move {
                if !caller.authenticated_host {
                    bail!("desktop services require authenticated owner authority");
                }
                let id = text(&params, "ownerId");
                let owner = lookup(id)
                    .filter(|s| s["status"] != "ended" && text(s, "hub").is_empty())
                    .ok_or_else(|| anyhow!("Owning session unavailable"))?;
                let cwd = if text(&owner, "liveCwd").is_empty() {
                    text(&owner, "cwd")
                } else {
                    text(&owner, "liveCwd")
                };
                let result = read(text(&params, "target"), Path::new(cwd), &config, &home).await;
                let current = lookup(id);
                if !current.as_ref().is_some_and(|s| {
                    s["status"] != "ended"
                        && text(s, "hub").is_empty()
                        && (if text(s, "liveCwd").is_empty() {
                            text(s, "cwd")
                        } else {
                            text(s, "liveCwd")
                        }) == cwd
                }) {
                    return Ok(
                        json!({"ok":false,"error":"Owning session changed while reading the diff"}),
                    );
                }
                Ok(result)
            }
        });
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(cwd: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[tokio::test]
    async fn cards_snapshot_head_and_working_bytes_with_literal_path_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        let config = dir.path().join("config");
        std::fs::create_dir(&cwd).unwrap();
        command(&cwd, &["init", "-q"]);
        command(&cwd, &["config", "user.email", "fixture@example.invalid"]);
        command(&cwd, &["config", "user.name", "Fixture"]);
        std::fs::write(cwd.join("[literal].txt"), "before\n").unwrap();
        command(&cwd, &["add", "."]);
        command(&cwd, &["commit", "-qm", "baseline"]);
        std::fs::write(cwd.join("[literal].txt"), "after\n").unwrap();
        let result = read("[literal].txt", &cwd, &config, dir.path()).await;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["before"], "before\n");
        assert_eq!(result["after"], "after\n");
        std::fs::write(cwd.join("new.txt"), "new\n").unwrap();
        assert_eq!(
            read("new.txt", &cwd, &config, dir.path()).await["before"],
            ""
        );
        std::fs::write(cwd.join("binary"), b"a\0b").unwrap();
        assert_eq!(read("binary", &cwd, &config, dir.path()).await["ok"], false);
        std::fs::write(cwd.join("large"), vec![b'x'; 256 * 1024 + 1]).unwrap();
        assert_eq!(read("large", &cwd, &config, dir.path()).await["ok"], false);
        assert_eq!(
            read(".git/config", &cwd, &config, dir.path()).await["ok"],
            false
        );
        assert_eq!(
            read("gone.txt", &cwd, &config, dir.path()).await["ok"],
            false
        );
    }
    #[test]
    fn selected_paths_deny_credentials_and_resolve_symlink_before_parent() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        let config = dir.path().join("config");
        std::fs::create_dir(&cwd).unwrap();
        for name in [".bus-token", ".settings.json", ".mcp.json", ".gitconfig"] {
            std::fs::write(cwd.join(name), "secret").unwrap();
            assert!(resolve(name, &cwd, &config, dir.path()).is_err());
        }
        let allowed = cwd.join(".claude/skills/hooks/SKILL.md");
        std::fs::create_dir_all(allowed.parent().unwrap()).unwrap();
        std::fs::write(&allowed, "instructions").unwrap();
        assert!(resolve(allowed.to_str().unwrap(), &cwd, &config, dir.path()).is_ok());
        let inside = config.join("library/public.md");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        std::fs::write(&inside, "public").unwrap();
        assert!(!selected_secret(&inside, &config, dir.path()));
        assert!(selected_secret(
            &config.join("tokens.json"),
            &config,
            dir.path()
        ));
        #[cfg(unix)]
        {
            let outside = dir.path().join("outside/nested");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.parent().unwrap().join("private"), "secret").unwrap();
            std::os::unix::fs::symlink(&outside, cwd.join("link")).unwrap();
            assert!(resolve("link/../private", &cwd, &config, dir.path()).is_err());
        }
    }
}

//! Execution-host observations. Probe output stays private, bounded and never
//! becomes an authentication claim based on a credential file's mere presence.
use super::{Capabilities, Directory, Execution, PROTOCOL, Provider, RemoteAdmission};
use crate::{
    Caller,
    services::{agent_lifecycle::Operation, config::Config, files::GIT_NO_EXEC, models, paths},
};
use anyhow::{Result, anyhow, bail};
use claudemon::daemon::{
    WorktreeMaintenance,
    embedded::{Command as EngineCommand, EmbeddedClient},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub type Spawn =
    Arc<dyn Fn(Caller, RemoteAdmission, Value) -> Operation<'static, Value> + Send + Sync>;
pub struct Native {
    config: Arc<Config>,
    engine: EmbeddedClient,
    spawn: Spawn,
}
impl Native {
    pub fn new(config: Arc<Config>, engine: EmbeddedClient, spawn: Spawn) -> Arc<Self> {
        Arc::new(Self {
            config,
            engine,
            spawn,
        })
    }
    async fn sessions(&self) -> Result<Value> {
        tokio::time::timeout(
            Duration::from_secs(5),
            self.engine.request(EngineCommand::Sessions),
        )
        .await
        .map_err(|_| anyhow!("execution host session inventory timed out"))?
    }
}
pub fn login_from_output(provider: &str, bytes: &[u8], success: bool) -> Option<bool> {
    if provider == "claude" {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let logged = value["loggedIn"].as_bool()?;
        return if logged && !success {
            None
        } else {
            Some(logged)
        };
    }
    if provider == "codex" {
        let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
        if text.contains("not logged in") {
            return Some(false);
        }
        if success && text.contains("logged in") {
            return Some(true);
        }
    }
    None
}
struct Output {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
async fn capture(
    command: &mut tokio::process::Command,
    timeout: Duration,
    limit: u64,
) -> Result<Output> {
    let output = super::super::owned_process::capture_limits(
        command,
        limit as usize,
        limit as usize,
        timeout,
    )
    .await?;
    if output.stdout.len().saturating_add(output.stderr.len()) > limit as usize {
        bail!("command output exceeded limit");
    }
    Ok(Output {
        success: output.status.success(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}
async fn login(binary: &str, provider: &str) -> Option<bool> {
    let args = match provider {
        "claude" => ["auth", "status"],
        "codex" => ["login", "status"],
        _ => return None,
    };
    let mut command = tokio::process::Command::new(binary);
    command.args(args);
    let output = capture(&mut command, Duration::from_secs(3), 64 * 1024)
        .await
        .ok()?;
    let mut bytes = output.stdout;
    bytes.extend(output.stderr);
    login_from_output(provider, &bytes, output.success)
}
async fn git(repo: &str, args: &[&str]) -> Result<Output> {
    let mut command = tokio::process::Command::new("git");
    command.current_dir(repo).arg("--no-replace-objects");
    for option in GIT_NO_EXEC {
        command.args(["-c", option]);
    }
    command.args(["-c", "core.hooksPath=/dev/null"]).args(args);
    capture(&mut command, Duration::from_secs(15), 1024 * 1024).await
}
async fn git_text(repo: &str, args: &[&str]) -> Result<String> {
    let output = git(repo, args).await?;
    if !output.success {
        bail!("execution-host Git operation failed")
    };
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
fn directory_choices(config: &Value, sessions: &Value) -> Result<Vec<Directory>> {
    let rows = sessions
        .as_array()
        .ok_or_else(|| anyhow!("invalid execution-host session inventory"))?;
    let mut directories = BTreeMap::new();
    let mut insert = |raw: &str, source: &str| {
        let Ok(path) = paths::canonicalize(Path::new(raw)) else {
            return;
        };
        if !path.is_dir() {
            return;
        }
        directories.entry(path).or_insert_with(|| source.to_owned());
    };
    // Resolve identities before deduplicating; configured project provenance
    // wins even when an active row uses another symlink/case spelling.
    if let Some(projects) = config["projects"].as_object() {
        for raw in projects.keys() {
            insert(raw, "project");
        }
    }
    for row in rows {
        if super::super::snapshots::live(row) && row.get("hub").is_none_or(Value::is_null) {
            if let Some(cwd) = row["cwd"].as_str() {
                insert(cwd, "active");
            }
        }
    }
    Ok(directories
        .into_iter()
        .map(|(path, source)| Directory {
            git: path.join(".git").exists(),
            path: path.to_string_lossy().into_owned(),
            source,
        })
        .collect())
}
impl Execution for Native {
    fn capabilities(&self) -> Operation<'_, Capabilities> {
        Box::pin(async move {
            let config = self.config.get();
            let binaries = models::check_all(&config);
            let statuses = binaries.as_array().cloned().unwrap_or_default();
            let providers =
                futures_util::future::join_all(statuses.into_iter().map(|status| async move {
                    let provider = status["provider"].as_str().unwrap_or("").to_owned();
                    let found = status["found"] == true;
                    let authenticated = if !found {
                        Some(false)
                    } else if let Some(binary) = status["resolvedPath"].as_str() {
                        login(binary, &provider).await
                    } else {
                        None
                    };
                    Provider {
                        provider,
                        found,
                        authenticated,
                        note: if !found {
                            "not installed on this machine"
                        } else {
                            match authenticated {
                                Some(true) => {
                                    "installed; provider login status confirmed on this host"
                                }
                                Some(false) => "installed but not logged in on this host",
                                None => "installed; this host could not confirm a usable login",
                            }
                        }
                        .into(),
                    }
                }))
                .await;
            let sessions = self.sessions().await?;
            let cwds = directory_choices(&config, &sessions)?;
            Ok(Capabilities {
                protocol: PROTOCOL,
                exact_model: true,
                executes: true,
                scope: "full".into(),
                providers,
                cwds,
                unsupported_reason: None,
            })
        })
    }
    fn canonical_directory<'a>(&'a self, cwd: &'a str) -> Operation<'a, String> {
        Box::pin(async move {
            let path = paths::canonicalize(Path::new(cwd))?;
            if !path.is_dir() {
                bail!("execution cwd is unavailable")
            };
            Ok(path.to_string_lossy().into_owned())
        })
    }
    fn allocate<'a>(&'a self, repo: &'a str, cwd: &'a str, branch: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            if self.canonical_directory(repo).await? != repo
                || paths::canonicalize(Path::new(
                    &git_text(repo, &["rev-parse", "--show-toplevel"]).await?,
                ))? != PathBuf::from(repo)
            {
                bail!("remote worktree allocation requires the selected repository root")
            }
            if Path::new(cwd).exists() {
                bail!("remote worktree destination already exists")
            };
            std::fs::create_dir_all(
                Path::new(cwd)
                    .parent()
                    .ok_or_else(|| anyhow!("worktree parent missing"))?,
            )?;
            if !git(repo, &["worktree", "add", "-b", branch, "--", cwd, "HEAD"])
                .await?
                .success
            {
                bail!("remote isolated worktree allocation failed; no worker started")
            };
            Ok(())
        })
    }
    fn cleanup<'a>(&'a self, repo: &'a str, cwd: &'a str, branch: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            if !Path::new(cwd).exists() {
                return Ok(());
            }
            if self.canonical_directory(cwd).await? != cwd
                || self.canonical_directory(repo).await? != repo
            {
                bail!("dispatch cleanup path changed")
            }
            let own = git_text(cwd, &["rev-parse", "--absolute-git-dir"]).await?;
            let _maintenance = WorktreeMaintenance::acquire_git_dir(Path::new(&own))?;
            let sessions = self.sessions().await?;
            for session in sessions
                .as_array()
                .ok_or_else(|| anyhow!("invalid authoritative session inventory"))?
            {
                if session["mode"] == "stopped" {
                    continue;
                }
                let path = session["cwd"]
                    .as_str()
                    .ok_or_else(|| anyhow!("live session lacks cwd; cleanup refused"))?;
                if paths::canonicalize(Path::new(path))? == PathBuf::from(cwd) {
                    bail!("dispatch worktree still in use")
                }
            }
            if !git_text(
                cwd,
                &[
                    "status",
                    "--porcelain",
                    "--ignored=matching",
                    "--untracked-files=all",
                ],
            )
            .await?
            .is_empty()
            {
                bail!("dispatch worktree contains changes; cleanup deferred")
            }
            if !git(repo, &["worktree", "remove", "--force", "--", cwd])
                .await?
                .success
            {
                bail!("dispatch worktree cleanup failed")
            }
            if !branch.is_empty() {
                let _ = git(repo, &["branch", "-d", "--", branch]).await;
            }
            Ok(())
        })
    }
    fn spawn<'a>(
        &'a self,
        caller: Caller,
        admission: RemoteAdmission,
        params: Value,
    ) -> Operation<'a, Value> {
        (self.spawn)(caller, admission, params)
    }
    fn blocked<'a>(&'a self, session: &'a str) -> Operation<'a, Option<bool>> {
        Box::pin(async move {
            let sessions = self.sessions().await?;
            let Some(row) = sessions
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["session_id"] == session))
            else {
                return Ok(None);
            };
            let state = row["ambient_state"]
                .as_str()
                .or_else(|| row["ambientState"].as_str());
            Ok(state.map(|state| matches!(state, "waiting_approval" | "waiting_input")))
        })
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use serde_json::json;
    #[cfg(unix)]
    #[tokio::test]
    async fn actual_login_probe_uses_provider_arguments_and_bounded_output() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("auth-fixture");
        let write = |body: &str| {
            std::fs::write(&binary, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let name = binary.to_str().unwrap();
        write("[ \"$1 $2\" = 'auth status' ] || exit 17\nprintf '%s' '{\"loggedIn\":true}'");
        assert_eq!(login(name, "claude").await, Some(true));
        write(
            "[ \"$1 $2\" = 'login status' ] || exit 17\nprintf '%s' 'Logged in using fixture' >&2",
        );
        assert_eq!(login(name, "codex").await, Some(true));
        write("printf '%s' '{\"loggedIn\":true}'\nexit 1");
        assert_eq!(login(name, "claude").await, None);
        write("printf '%s' 'Not logged in' >&2\nexit 1");
        assert_eq!(login(name, "codex").await, Some(false));
        // No actual provider, login, credentials, or network is involved.
        write(
            "printf '%s' 'Logged in'\ni=0\nwhile [ $i -lt 7000 ]; do printf '0123456789'; i=$((i + 1)); done",
        );
        assert_eq!(login(name, "codex").await, None);
        assert_eq!(login("/missing/workspacer-fixture", "claude").await, None);
        assert_eq!(login(name, "copilot").await, None);
    }
    #[test]
    fn retained_login_vectors_keep_unknown_and_negative_separate() {
        for (provider, raw, success, want) in [
            ("codex", "Not logged in", true, Some(false)),
            ("codex", "Not logged in", false, Some(false)),
            ("codex", "Logged in using ChatGPT", true, Some(true)),
            ("codex", "Logged in using ChatGPT", false, None),
            ("claude", r#"{"loggedIn":true}"#, true, Some(true)),
            ("claude", r#"{"loggedIn":true}"#, false, None),
            ("claude", r#"{"loggedIn":false}"#, true, Some(false)),
            (
                "claude",
                r#"{"oauthAccount":{"email":"stale@example.invalid"}}"#,
                true,
                None,
            ),
            ("claude", "unrecognized command", false, None),
            ("claude", "[true]", true, None),
            ("claude", r#"{"loggedIn":"true"}"#, true, None),
            ("copilot", "Logged in", true, None),
        ] {
            assert_eq!(
                login_from_output(provider, raw.as_bytes(), success),
                want,
                "{provider}: {raw}"
            );
        }
    }
    #[test]
    fn cwd_menu_excludes_shells_stopped_and_remote_rows_and_keeps_project_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        for name in [
            "project", "active", "shell", "stopped", "archived", "remote", "bad", "ended",
        ] {
            std::fs::create_dir(root.join(name)).unwrap();
        }
        std::fs::write(root.join("project/.git"), "gitdir: fixture").unwrap();
        let config = json!({"projects":{(root.join("project").to_str().unwrap()):{},(root.join("missing").to_str().unwrap()):{}}});
        let rows = json!([
            {"cwd":root.join("project"),"mode":"input"},
            {"cwd":root.join("active"),"mode":"approval"},
            {"cwd":root.join("shell"),"mode":"unknown"},
            {"cwd":root.join("stopped"),"mode":"stopped"},
            {"cwd":root.join("archived"),"mode":"input","archived":true},
            {"cwd":root.join("remote"),"mode":"input","hub":"peer"},
            {"cwd":root.join("bad"),"mode":true},
            {"cwd":root.join("ended"),"status":"ended"}
        ]);
        let selected = directory_choices(&config, &rows).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].path, root.join("active").to_str().unwrap());
        assert_eq!(selected[0].source, "active");
        assert!(!selected[0].git);
        assert_eq!(selected[1].source, "project");
        assert!(selected[1].git);
        assert!(directory_choices(&config, &json!({})).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("project"), root.join("z-project-alias")).unwrap();
            let config = json!({"projects":{(root.join("z-project-alias").to_str().unwrap()):{}}});
            let selected = directory_choices(
                &config,
                &json!([{ "cwd":root.join("project"), "mode":"input"}]),
            )
            .unwrap();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].path, root.join("project").to_str().unwrap());
            assert_eq!(selected[0].source, "project");
        }
    }
}

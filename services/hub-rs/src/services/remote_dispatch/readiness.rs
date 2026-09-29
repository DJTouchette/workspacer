//! Execution-host observations. Probe output stays private, bounded and never
//! becomes an authentication claim based on a credential file's mere presence.
use super::{Capabilities, Directory, Execution, PROTOCOL, Provider, RemoteAdmission};
use crate::{
    Caller,
    services::{agent_lifecycle::Operation, config::Config, files::GIT_NO_EXEC, models, paths},
};
use anyhow::{Result, anyhow, bail};
use claudemon::{
    child_env::SanitizeChildEnvironment,
    daemon::{
        WorktreeMaintenance,
        embedded::{Command as EngineCommand, EmbeddedClient},
    },
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::io::AsyncReadExt;
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
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .scrub_host_authority();
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    #[cfg(unix)]
    let mut group = Group(child.id().unwrap());
    #[cfg(windows)]
    let _job = child
        .raw_handle()
        .and_then(|handle| crate::plugins::supervisor::windows_job::Job::assign_raw(handle).ok());
    let mut stdout = child.stdout.take().unwrap().take(limit + 1);
    let mut stderr = child.stderr.take().unwrap().take(limit + 1);
    let result = tokio::time::timeout(timeout, async {
        let mut out = vec![];
        let mut err = vec![];
        let (a, b) = tokio::try_join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err))?;
        if a + b > limit as usize {
            bail!("command output exceeded limit")
        }
        let status = child.wait().await?;
        anyhow::Ok(Output {
            success: status.success(),
            stdout: out,
            stderr: err,
        })
    })
    .await;
    #[cfg(unix)]
    group.kill();
    let _ = child.start_kill();
    let _ = child.wait().await;
    result.map_err(|_| anyhow!("execution-host command timed out"))?
}
#[cfg(unix)]
struct Group(u32);
#[cfg(unix)]
impl Group {
    fn kill(&mut self) {
        if self.0 != 0 {
            unsafe {
                libc::kill(-(self.0 as i32), libc::SIGKILL);
            }
            self.0 = 0;
        }
    }
}
#[cfg(unix)]
impl Drop for Group {
    fn drop(&mut self) {
        self.kill();
    }
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
            let mut directories = BTreeMap::new();
            if let Some(projects) = config["projects"].as_object() {
                for path in projects.keys() {
                    directories.insert(path.clone(), "project");
                }
            }
            let sessions = self.sessions().await?;
            for session in sessions
                .as_array()
                .ok_or_else(|| anyhow!("invalid execution-host session inventory"))?
            {
                if session["mode"] == "stopped"
                    || session.get("hub").is_some_and(|hub| !hub.is_null())
                {
                    continue;
                }
                if let Some(cwd) = session["cwd"].as_str().filter(|cwd| !cwd.is_empty()) {
                    directories.entry(cwd.into()).or_insert("active");
                }
            }
            let mut cwds = vec![];
            for (directory, source) in directories {
                let Ok(path) = paths::canonicalize(Path::new(&directory)) else {
                    continue;
                };
                if !path.is_dir() {
                    continue;
                }
                cwds.push(Directory {
                    path: path.to_string_lossy().into_owned(),
                    source: source.into(),
                    git: path.join(".git").exists(),
                });
            }
            cwds.sort_by(|a, b| a.path.cmp(&b.path));
            cwds.dedup_by(|a, b| a.path == b.path);
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

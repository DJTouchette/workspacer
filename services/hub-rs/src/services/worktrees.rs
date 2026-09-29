//! Host-owned worktree allocation, dependency linking and conservative removal.
use super::{atomic_json, config::Config, files::GIT_NO_EXEC, paths};
use anyhow::{Result, anyhow, bail};
use claudemon::daemon::{
    WorktreeAdmission, WorktreeMaintenance,
    embedded::{Command as EngineCommand, EmbeddedClient},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
}
async fn capture(command: &mut tokio::process::Command, timeout: Duration) -> Result<Output> {
    let output = super::owned_process::capture(command, 16 * 1024 * 1024, timeout).await?;
    Ok(Output {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}
async fn git(cwd: &Path, args: &[&str]) -> Result<Output> {
    let mut command = tokio::process::Command::new("git");
    command.current_dir(cwd).arg("--no-replace-objects");
    for key in GIT_NO_EXEC {
        command.args(["-c", key]);
    }
    // Allocation must not execute repository hooks during checkout.
    command.args(["-c", "core.hooksPath=/dev/null"]).args(args);
    capture(&mut command, Duration::from_secs(15)).await
}
async fn git_text(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = git(cwd, args).await?;
    if !output.ok {
        bail!("{}", output.stderr.trim());
    }
    Ok(output.stdout.trim().into())
}
pub async fn info(cwd: &Path) -> Value {
    if !cwd.is_absolute() || std::fs::read_dir(cwd).is_err() {
        return json!({"isRepo":false,"directory":"invalid"});
    }
    match git(cwd, &["rev-parse", "--show-toplevel"]).await {
        Ok(top) if top.ok && !top.stdout.trim().is_empty() => {
            let mut branch = git_text(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
                .await
                .unwrap_or_default();
            if branch == "HEAD" {
                branch = git_text(cwd, &["rev-parse", "--short", "HEAD"])
                    .await
                    .unwrap_or_default();
            }
            let mut result = json!({"isRepo":true,"directory":"accessible","gitStatus":"repo","root":top.stdout.trim()});
            if !branch.is_empty() {
                result["branch"] = json!(branch);
            }
            result
        }
        Ok(top) => {
            json!({"isRepo":false,"directory":"accessible","gitStatus":if top.stderr.to_lowercase().contains("not a git repository") {"non-git"} else {"unknown"}})
        }
        Err(_) => json!({"isRepo":false,"directory":"accessible","gitStatus":"unknown"}),
    }
}
async fn linked(cwd: &Path) -> Result<Option<(PathBuf, PathBuf)>> {
    let root = PathBuf::from(git_text(cwd, &["rev-parse", "--show-toplevel"]).await?);
    let marker = std::fs::symlink_metadata(root.join(".git"))?;
    if marker.is_dir() {
        return Ok(None);
    }
    if !marker.is_file() || marker.file_type().is_symlink() {
        bail!("invalid linked-worktree Git marker");
    }
    let own = std::fs::canonicalize(git_text(cwd, &["rev-parse", "--absolute-git-dir"]).await?)?;
    let common = std::fs::canonicalize(
        git_text(
            cwd,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?,
    )?;
    Ok((own != common).then_some((own, common)))
}
pub struct Reservation {
    pub cwd: PathBuf,
    _admission: WorktreeAdmission,
}
pub struct Created {
    pub result: Value,
    pub reservation: Option<Reservation>,
}
pub struct Worktrees {
    home: PathBuf,
    config: Arc<Config>,
    engine: Option<EmbeddedClient>,
    review: std::sync::Mutex<Option<Arc<super::fleet_review::ReviewStore>>>,
    // Holds a candidate through checkout and metadata registration. Returned
    // admission reservations then fence cleanup until daemon registration.
    operations: tokio::sync::Mutex<()>,
}
impl Worktrees {
    pub fn new(home: PathBuf, config: Arc<Config>, engine: Option<EmbeddedClient>) -> Arc<Self> {
        Arc::new(Self {
            home,
            config,
            engine,
            review: std::sync::Mutex::new(None),
            operations: tokio::sync::Mutex::new(()),
        })
    }
    pub fn set_review_store(&self, review: Arc<super::fleet_review::ReviewStore>) {
        *self.review.lock().unwrap() = Some(review);
    }
    pub fn review_store(&self) -> Option<Arc<super::fleet_review::ReviewStore>> {
        self.review.lock().unwrap().clone()
    }
    pub fn root(&self) -> Result<PathBuf> {
        let config = self.config.get();
        let value = config["agents"]["worktreeRoot"]
            .as_str()
            .unwrap_or("")
            .trim();
        paths::canonicalize(
            if value.is_empty() {
                self.home.join(".workspacer/worktrees")
            } else {
                PathBuf::from(value)
            }
            .as_path(),
        )
    }
    pub async fn create(self: &Arc<Self>, params: Value) -> Result<Value> {
        let service = self.clone();
        // Cancellation of the caller does not abandon checkout or setup hooks.
        tokio::spawn(async move { Ok(service.create_reserved(params).await?.result) }).await?
    }
    pub async fn create_reserved(&self, params: Value) -> Result<Created> {
        let _operation = self.operations.lock().await;
        let cwd = paths::canonicalize(Path::new(
            params["repoCwd"]
                .as_str()
                .ok_or_else(|| anyhow!("repoCwd is required"))?,
        ))?;
        let root = self.root()?;
        if let Some(requested) = params["rootOverride"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
        {
            if paths::canonicalize(Path::new(requested))? != root {
                bail!("Worktree destination must match the server configuration");
            }
        }
        let inspected = info(&cwd).await;
        let Some(project) = inspected["root"].as_str() else {
            return Ok(Created {
                result: json!({"ok":false,"error":format!("{} is not inside a git repository",cwd.display())}),
                reservation: None,
            });
        };
        let project = std::fs::canonicalize(project)?;
        let source_lease = if let Some((own, _)) = linked(&project).await? {
            Some(WorktreeMaintenance::acquire_git_dir(&own)?)
        } else {
            None
        };
        let parent = root.join(
            project
                .file_name()
                .ok_or_else(|| anyhow!("repository has no directory name"))?,
        );
        std::fs::create_dir_all(&parent)?;
        if std::fs::canonicalize(&parent)?
            != std::fs::canonicalize(&root)?.join(project.file_name().unwrap())
        {
            bail!("worktree destination contains a symlink");
        }
        let slug = slug(params["name"].as_str().unwrap_or(""));
        for attempt in 0..3 {
            let name = if attempt == 0 {
                slug.clone()
            } else {
                format!("{slug}-{}", &uuid::Uuid::new_v4().simple().to_string()[..4])
            };
            let destination = parent.join(&name);
            let branch = format!("wks/{name}");
            if std::fs::symlink_metadata(&destination).is_ok() {
                continue;
            }
            let result = git(
                &project,
                &[
                    "worktree",
                    "add",
                    "-b",
                    &branch,
                    destination
                        .to_str()
                        .ok_or_else(|| anyhow!("worktree path is not Unicode"))?,
                ],
            )
            .await?;
            if !result.ok {
                if result.stderr.to_lowercase().contains("already exists") {
                    continue;
                }
                return Ok(Created {
                    result: json!({"ok":false,"error":result.stderr.trim()}),
                    reservation: None,
                });
            }
            let (own, common) = linked(&destination)
                .await?
                .ok_or_else(|| anyhow!("allocated path is not a linked worktree"))?;
            let admission = WorktreeAdmission::acquire(destination.to_str().unwrap())?
                .ok_or_else(|| anyhow!("allocated path has no admission fence"))?;
            let reservation = Reservation {
                cwd: destination.clone(),
                _admission: admission,
            };
            // The path now has the daemon-shared reservation. Slow setup hooks
            // need not serialize allocations in other repositories/worktrees.
            drop(_operation);
            let mut allocation = json!({"version":1,"cwd":destination,"root":root,"createdAt":crate::protocol::now()});
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let meta = std::fs::metadata(&destination)?;
                allocation["dev"] = json!(meta.dev());
                allocation["ino"] = json!(meta.ino());
            }
            atomic_json(&own.join("workspacer-allocation.json"), &allocation, true)?;
            let base = git_text(&destination, &["rev-parse", "--verify", "HEAD^{commit}"]).await?;
            let review = json!({"projectRoot":project,"allocatedCwd":std::fs::canonicalize(&destination)?,"commonDir":common,"branch":branch,"baseCommit":base});
            let _ = link_dependencies(&project, &destination).await;
            let setup = run_setup(
                &resolve_setup(&self.config.get(), &[&cwd, &project]),
                &project,
                &destination,
                Duration::from_secs(300),
            )
            .await;
            drop(source_lease);
            let mut result =
                json!({"ok":true,"path":destination,"branch":branch,"reviewAllocation":review});
            if let Some(setup) = setup {
                result["setup"] = setup;
            }
            return Ok(Created {
                result,
                reservation: Some(reservation),
            });
        }
        Ok(Created {
            result: json!({"ok":false,"error":"could not find a free worktree name (tried 3 candidates)"}),
            reservation: None,
        })
    }
    pub async fn remove(&self, cwd: &Path) -> Result<Value> {
        let _operation = self.operations.lock().await;
        let root = self.root()?;
        let cwd = match std::fs::canonicalize(cwd) {
            Ok(path) => path,
            Err(_) => return Ok(json!({"ok":false,"skipped":true})),
        };
        let root = match std::fs::canonicalize(&root) {
            Ok(root) => root,
            Err(_) => return Ok(json!({"ok":false,"skipped":true})),
        };
        if (paths::contained(&cwd, &root) && paths::contained(&root, &cwd))
            || !paths::contained(&cwd, &root)
        {
            return Ok(json!({"ok":false,"skipped":true}));
        }
        let Some((own, common)) = linked(&cwd).await? else {
            return Ok(json!({"ok":false,"skipped":true}));
        };
        let _lease = match WorktreeMaintenance::acquire_git_dir(&own) {
            Ok(lease) => lease,
            Err(error) => return Ok(json!({"ok":false,"skipped":true,"error":error.to_string()})),
        };
        let Some(engine) = &self.engine else {
            return Ok(
                json!({"ok":false,"skipped":true,"error":"Live session state is unavailable"}),
            );
        };
        let sessions = engine.request(EngineCommand::Sessions).await?;
        let sessions = sessions
            .as_array()
            .ok_or_else(|| anyhow!("invalid daemon session inventory"))?;
        for row in sessions {
            if row["mode"] == "stopped" {
                continue;
            }
            let Some(path) = row["cwd"].as_str() else {
                bail!("live session lacks cwd; cleanup refused");
            };
            let path = std::fs::canonicalize(path)?;
            if paths::contained(&path, &cwd) {
                return Ok(
                    json!({"ok":false,"skipped":true,"error":"Worktree still has a live agent"}),
                );
            }
        }
        if let Some(review) = self.review_store() {
            review.before_remove(&cwd).await?;
        }
        let main = common
            .parent()
            .ok_or_else(|| anyhow!("Git common directory has no parent"))?;
        let result = git(
            main,
            &[
                "--git-dir",
                common.to_str().unwrap(),
                "worktree",
                "remove",
                cwd.to_str().unwrap(),
            ],
        )
        .await?;
        if !result.ok {
            return Ok(json!({"ok":false,"skipped":true,"error":result.stderr.trim()}));
        }
        Ok(json!({"ok":true,"removed":cwd}))
    }
}
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || "._-".contains(c) {
            slug.push(c);
            dash = false;
        } else if !dash {
            slug.push('-');
            dash = true;
        }
    }
    let value = slug
        .trim_matches(['-', '.'])
        .chars()
        .take(40)
        .collect::<String>();
    if value.is_empty() {
        "agent".into()
    } else {
        value
    }
}
#[derive(Clone, Debug)]
pub struct Setup {
    pub raw: String,
    pub command: Option<String>,
    pub error: Option<String>,
}
fn lookup<'a>(map: &'a Value, dir: &Path) -> Option<&'a Value> {
    let key = dir
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    if let Some(value) = map.get(&key) {
        return Some(value);
    }
    #[cfg(any(windows, target_os = "macos"))]
    if let Some(map) = map.as_object() {
        return map
            .iter()
            .find(|(k, _)| k.to_lowercase() == key.to_lowercase())
            .map(|(_, v)| v);
    }
    None
}
pub fn resolve_setup(config: &Value, dirs: &[&Path]) -> Vec<Setup> {
    for dir in dirs {
        let Some(commands) = lookup(&config["projects"], dir)
            .and_then(|p| p["worktreeSetup"].as_array())
            .filter(|c| !c.is_empty())
        else {
            continue;
        };
        let scripts = lookup(&config["scripts"], dir).and_then(Value::as_array);
        return commands
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|raw| {
                if let Some(reference) = raw.strip_prefix("script:").filter(|r| !r.is_empty()) {
                    let command = scripts
                        .and_then(|scripts| scripts.iter().find(|s| s["name"] == reference.trim()))
                        .and_then(|s| s["command"].as_str())
                        .map(str::to_owned);
                    Setup {
                        raw: raw.into(),
                        error: command.is_none().then(|| {
                            format!("no script named {:?} for this project", reference.trim())
                        }),
                        command,
                    }
                } else {
                    Setup {
                        raw: raw.into(),
                        command: Some(raw.into()),
                        error: None,
                    }
                }
            })
            .collect();
    }
    vec![]
}
pub async fn run_setup(
    commands: &[Setup],
    source: &Path,
    worktree: &Path,
    timeout: Duration,
) -> Option<Value> {
    if commands.is_empty() {
        return None;
    }
    let mut ran = Vec::new();
    for (index, entry) in commands.iter().enumerate() {
        let error = if let Some(command) = &entry.command {
            // Paths are environment data, not shell source. Native expansion
            // preserves quoted arguments without executing punctuation in cwd.
            #[cfg(unix)]
            let text = command.clone();
            #[cfg(windows)]
            let text = command
                .replace("${SOURCE}", "!SOURCE!")
                .replace("$SOURCE", "!SOURCE!")
                .replace("${WORKTREE}", "!WORKTREE!")
                .replace("$WORKTREE", "!WORKTREE!");
            #[cfg(unix)]
            let mut process = {
                let mut p = tokio::process::Command::new("/bin/sh");
                p.args(["-c", &text]);
                p
            };
            #[cfg(windows)]
            let mut process = {
                let mut p = tokio::process::Command::new("cmd.exe");
                p.args(["/V:ON", "/C", &text]);
                p
            };
            process
                .current_dir(worktree)
                .env("SOURCE", source)
                .env("WORKTREE", worktree);
            match capture(&mut process, timeout).await {
                Ok(out) if out.ok => None,
                Ok(out) => Some(if out.stderr.trim().is_empty() {
                    "command failed".into()
                } else {
                    out.stderr.trim().into()
                }),
                Err(error) => Some(error.to_string()),
            }
        } else {
            Some(
                entry
                    .error
                    .clone()
                    .unwrap_or_else(|| "unresolvable entry".into()),
            )
        };
        if let Some(error) = error {
            return Some(
                json!({"ran":ran,"failed":{"command":entry.raw,"error":error},"skipped":commands[index+1..].iter().map(|c|c.raw.clone()).collect::<Vec<_>>()}),
            );
        }
        ran.push(entry.raw.clone());
    }
    Some(json!({"ran":ran,"skipped":[]}))
}
pub async fn discover_dependencies(source: &Path) -> Result<Vec<PathBuf>> {
    let tree = git(source, &["ls-tree", "-r", "--name-only", "-z", "HEAD"]).await?;
    if !tree.ok {
        bail!("dependency discovery requires a readable Git HEAD");
    }
    let mut parents = BTreeSet::from([PathBuf::new()]);
    for file in tree.stdout.split('\0').filter(|s| !s.is_empty()) {
        if file
            .split('/')
            .any(|p| p.starts_with('.') || p == "node_modules")
        {
            continue;
        }
        let mut path = Path::new(file).parent();
        while let Some(parent) = path {
            parents.insert(parent.to_owned());
            path = parent.parent();
        }
    }
    Ok(parents
        .into_iter()
        .map(|p| p.join("node_modules"))
        .filter(|p| source.join(p).is_dir())
        .collect())
}
pub async fn link_dependencies(source: &Path, destination: &Path) -> Result<Vec<PathBuf>> {
    let mut linked = Vec::new();
    for relative in discover_dependencies(source).await? {
        let path = destination.join(&relative);
        if std::fs::symlink_metadata(&path).is_ok() || !path.parent().is_some_and(Path::is_dir) {
            continue;
        }
        #[cfg(unix)]
        let result = std::os::unix::fs::symlink(source.join(&relative), &path);
        #[cfg(windows)]
        let result = std::os::windows::fs::symlink_dir(source.join(&relative), &path);
        if result.is_err() {
            continue;
        }
        if git(
            destination,
            &["check-ignore", "-q", relative.to_str().unwrap()],
        )
        .await
        .is_ok_and(|r| r.ok)
        {
            linked.push(relative);
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(linked)
}

pub(crate) fn install(mut options: crate::Options, config: Arc<Config>) -> crate::Options {
    let Some(home) = options.home_dir.clone() else {
        return options;
    };
    let service = Worktrees::new(home, config, options.engine.clone());
    if let Some(review) = options.review_store.clone() {
        service.set_review_store(review);
    }
    options.worktrees = Some(service.clone());
    for method in [
        "desktop.worktreeInfo",
        "desktop.worktreeCreate",
        "desktop.worktreeRemove",
    ] {
        let service = service.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            async move {
                if !caller.authenticated_host {
                    bail!("desktop services require the authenticated server owner's connection");
                }
                match method {
                    "desktop.worktreeInfo" => Ok(info(Path::new(
                        params["cwd"]
                            .as_str()
                            .ok_or_else(|| anyhow!("cwd is required"))?,
                    ))
                    .await),
                    "desktop.worktreeCreate" => service.create(params).await,
                    _ => {
                        service
                            .remove(Path::new(
                                params["cwd"]
                                    .as_str()
                                    .ok_or_else(|| anyhow!("cwd is required"))?,
                            ))
                            .await
                    }
                }
            }
        });
    }
    options
}

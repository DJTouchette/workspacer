//! Pull requests on GitHub and Azure DevOps, behind one `GitProvider` trait.
//!
//! The provider for a working directory comes from its project's `git`
//! settings in config.yaml (`projects.<path>.git.provider`: `github`,
//! `azure-devops`, or absent for automatic) and otherwise from the git
//! remote's host. Credentials come from the provider CLIs (see
//! `credentials`). Bus methods:
//!
//! - `pr.providers` — each provider's CLI and sign-in state
//! - `pr.detect {cwd}` — the project's provider, remotes, repository, branch
//! - `pr.forBranch {cwd}` — the open pull request for the current branch
//! - `pr.get {url}`, `pr.threads {url}` — details and discussion
//! - `pr.reply {url, threadId, body}`, `pr.comment {url, body}`,
//!   `pr.resolve {url, threadId, resolved}`
//!
//! Linking a session (`pr.link`, `pr.unlink`) lives with the session service,
//! which republishes the session's snapshot with its `pullRequest`.
pub(crate) mod azure;
pub(crate) mod credentials;
pub(crate) mod github;
mod http;
pub(crate) mod links;
pub(crate) mod model;
pub(crate) mod remote;

use crate::Options;
use anyhow::{Result, anyhow, bail};
use credentials::Credentials;
use model::{Comment, PullRequest, Threads};
use remote::{Kind, PrRef, Repo};
use serde_json::{Value, json};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

/// What every hosted git service offers for pull requests.
pub(crate) trait GitProvider: Send + Sync {
    fn pull_request(&self, pr: &PrRef) -> impl Future<Output = Result<PullRequest>> + Send;
    /// The open pull request whose source is `branch`, if any.
    fn for_branch(
        &self,
        repo: &Repo,
        branch: &str,
    ) -> impl Future<Output = Result<Option<PullRequest>>> + Send;
    fn threads(&self, pr: &PrRef) -> impl Future<Output = Result<Threads>> + Send;
    fn reply(
        &self,
        pr: &PrRef,
        thread: &str,
        body: &str,
    ) -> impl Future<Output = Result<Comment>> + Send;
    /// A new top-level comment on the pull request.
    fn comment(&self, pr: &PrRef, body: &str) -> impl Future<Output = Result<Comment>> + Send;
    fn set_resolved(
        &self,
        pr: &PrRef,
        thread: &str,
        resolved: bool,
    ) -> impl Future<Output = Result<()>> + Send;
}

/// Runs `$body` with `$provider` bound to the implementation for `$kind`.
macro_rules! with_provider {
    ($service:expr, $kind:expr, |$provider:ident| $body:expr) => {
        match $kind {
            Kind::Github => {
                let $provider = &$service.github;
                $body
            }
            Kind::AzureDevops => {
                let $provider = &$service.azure;
                $body
            }
        }
    };
}

/// A project's `git` settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProjectGit {
    /// The project's key in config.yaml, when the directory belongs to one.
    pub project: Option<String>,
    pub provider: Option<Kind>,
    /// The remote to read (default: `origin`, else the first).
    pub remote: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Detected {
    pub settings: ProjectGit,
    /// Every remote, with the repository it names when recognised.
    pub remotes: Vec<(String, String, Option<Repo>)>,
    pub repo: Option<Repo>,
    pub branch: Option<String>,
    /// Why there is no repository, when there is none.
    pub problem: Option<String>,
}

impl Detected {
    fn wire(&self) -> Value {
        let mut candidates: Vec<Kind> = self
            .remotes
            .iter()
            .filter_map(|(_, _, repo)| repo.as_ref().map(|r| r.kind))
            .collect();
        candidates.dedup();
        json!({
            "project": self.settings.project,
            "configured": self.settings.provider,
            "remote": self.settings.remote,
            "remotes": self.remotes.iter().map(|(name, url, repo)| json!({
                "name": name, "url": url, "provider": repo.as_ref().map(|r| r.kind),
            })).collect::<Vec<_>>(),
            "candidates": candidates,
            "provider": self.repo.as_ref().map(|r| r.kind),
            "repo": self.repo.as_ref().map(Repo::display),
            "branch": self.branch,
            "problem": self.problem,
        })
    }
}

pub(crate) struct Service {
    github: github::GitHub,
    azure: azure::Azure,
    credentials: Arc<Credentials>,
    config: Option<Arc<super::config::Config>>,
    pub(crate) links: Arc<links::Links>,
    /// Comments this hub posted (from the app or an agent): never news to
    /// the agent that is watching the pull request.
    posted: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

/// `pullRequests` in config.yaml; both default on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    /// Link a session to the open pull request for its branch.
    pub auto_link: bool,
    /// Tell an idle agent about new comments on its linked pull request.
    pub notify: bool,
}

impl Service {
    pub(crate) fn new(config: Arc<super::config::Config>, links: Arc<links::Links>) -> Self {
        let credentials = Arc::new(Credentials::cli());
        Self {
            github: github::GitHub::new(credentials.clone(), None),
            azure: azure::Azure::new(credentials.clone(), None),
            credentials,
            config: Some(config),
            links,
            posted: Default::default(),
        }
    }

    /// Both providers against a local server, with fixed credentials.
    #[cfg(test)]
    pub(crate) fn for_tests(base: &str, links: Arc<links::Links>) -> Self {
        let credentials = Arc::new(Credentials::fixed(
            [
                (Kind::Github, credentials::Auth::Bearer("gh-token".into())),
                (
                    Kind::AzureDevops,
                    credentials::Auth::Basic("ado-token".into()),
                ),
            ]
            .into(),
        ));
        Self {
            github: github::GitHub::new(credentials.clone(), Some(format!("{base}/github"))),
            azure: azure::Azure::new(credentials.clone(), Some(format!("{base}/azure"))),
            credentials,
            config: None,
            links,
            posted: Default::default(),
        }
    }

    pub(crate) fn settings(&self) -> Settings {
        let config = self.config.as_ref().map(|c| c.get()).unwrap_or_default();
        let section = &config["pullRequests"];
        Settings {
            auto_link: section["autoLink"] != false,
            notify: section["notifyAgent"] != false,
        }
    }

    pub(crate) fn posted_by_us(&self, comment: &str) -> bool {
        self.posted.lock().unwrap().contains(comment)
    }

    fn remember_posted(&self, comment: &Comment) {
        if !comment.id.is_empty() {
            self.posted.lock().unwrap().insert(comment.id.clone());
        }
    }

    /// The `git` settings of the project that contains `cwd` (the deepest
    /// one, when projects nest).
    pub(crate) fn project_git(&self, cwd: &Path) -> ProjectGit {
        let Some(config) = &self.config else {
            return ProjectGit::default();
        };
        project_git(&config.get(), cwd)
    }

    pub(crate) async fn detect(&self, cwd: &Path) -> Result<Detected> {
        let settings = self.project_git(cwd);
        let names = super::git::read(cwd, &["remote"])
            .await?
            .ok_or_else(|| anyhow!("{} is not a git repository", cwd.display()))?;
        let mut remotes = Vec::new();
        for name in names.lines().map(str::trim).filter(|n| !n.is_empty()) {
            if let Some(url) = super::git::read(cwd, &["remote", "get-url", name]).await? {
                let repo = remote::parse_remote(&url, settings.provider);
                remotes.push((name.to_owned(), url, repo));
            }
        }
        let chosen = settings
            .remote
            .as_deref()
            .and_then(|wanted| remotes.iter().find(|(name, ..)| name == wanted))
            .or_else(|| remotes.iter().find(|(name, ..)| name == "origin"))
            .or_else(|| remotes.first());
        let (repo, problem) = match (chosen, settings.provider) {
            (None, _) => (None, Some("This repository has no remotes".to_owned())),
            (Some((name, _, None)), _) => (
                None,
                Some(format!(
                    "Remote `{name}` is not a GitHub or Azure DevOps repository"
                )),
            ),
            (Some((name, _, Some(repo))), Some(wanted)) if repo.kind != wanted => (
                None,
                Some(format!(
                    "The project uses {} but remote `{name}` is on {}",
                    wanted.label(),
                    repo.kind.label()
                )),
            ),
            (Some((_, _, Some(repo))), _) => (Some(repo.clone()), None),
        };
        let branch = super::git::read(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await?
            .filter(|b| !b.is_empty());
        Ok(Detected {
            settings,
            remotes,
            repo,
            branch,
            problem,
        })
    }

    /// The open pull request for the branch checked out in `cwd`.
    pub(crate) async fn for_cwd(&self, cwd: &Path) -> Result<(Detected, Option<PullRequest>)> {
        let detected = self.detect(cwd).await?;
        let (Some(repo), Some(branch)) = (&detected.repo, &detected.branch) else {
            return Ok((detected, None));
        };
        let found = with_provider!(self, repo.kind, |p| p.for_branch(repo, branch).await)?;
        Ok((detected, found))
    }

    /// Parses a pull request URL; `hint` lets a GitHub Enterprise URL through.
    pub(crate) fn parse(&self, url: &str, hint: Option<Kind>) -> Result<PrRef> {
        remote::parse_pull_request(url, hint)
            .ok_or_else(|| anyhow!("not a GitHub or Azure DevOps pull request URL: {url}"))
    }

    pub(crate) async fn pull_request(&self, pr: &PrRef) -> Result<PullRequest> {
        with_provider!(self, pr.repo.kind, |p| p.pull_request(pr).await)
    }

    pub(crate) async fn threads(&self, pr: &PrRef) -> Result<Threads> {
        with_provider!(self, pr.repo.kind, |p| p.threads(pr).await)
    }

    pub(crate) async fn reply(&self, pr: &PrRef, thread: &str, body: &str) -> Result<Comment> {
        let comment = with_provider!(self, pr.repo.kind, |p| p.reply(pr, thread, body).await)?;
        self.remember_posted(&comment);
        Ok(comment)
    }

    pub(crate) async fn comment(&self, pr: &PrRef, body: &str) -> Result<Comment> {
        let comment = with_provider!(self, pr.repo.kind, |p| p.comment(pr, body).await)?;
        self.remember_posted(&comment);
        Ok(comment)
    }

    pub(crate) async fn set_resolved(
        &self,
        pr: &PrRef,
        thread: &str,
        resolved: bool,
    ) -> Result<()> {
        with_provider!(self, pr.repo.kind, |p| p
            .set_resolved(pr, thread, resolved)
            .await)
    }

    async fn providers(&self) -> Value {
        let (github, azure) = tokio::join!(
            credentials::status(&self.credentials, Kind::Github, "github.com"),
            credentials::status(&self.credentials, Kind::AzureDevops, "dev.azure.com"),
        );
        json!([github, azure])
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match method {
            "pr.providers" => Ok(self.providers().await),
            "pr.detect" => Ok(self.detect(&cwd(&params)?).await?.wire()),
            "pr.forBranch" => {
                let (detected, found) = self.for_cwd(&cwd(&params)?).await?;
                Ok(json!({"detected": detected.wire(), "pullRequest": found}))
            }
            _ => {
                let pr = self.parse(text(&params, "url")?, hint(&params)?)?;
                match method {
                    "pr.get" => Ok(json!(self.pull_request(&pr).await?)),
                    "pr.threads" => {
                        let threads = self.threads(&pr).await?;
                        let mut value = json!(threads);
                        value["unresolved"] = json!(threads.unresolved());
                        Ok(value)
                    }
                    "pr.reply" => Ok(json!(
                        self.reply(&pr, text(&params, "threadId")?, body(&params)?)
                            .await?
                    )),
                    "pr.comment" => Ok(json!(self.comment(&pr, body(&params)?).await?)),
                    "pr.resolve" => {
                        let resolved = params["resolved"]
                            .as_bool()
                            .ok_or_else(|| anyhow!("resolved must be true or false"))?;
                        self.set_resolved(&pr, text(&params, "threadId")?, resolved)
                            .await?;
                        Ok(json!({"ok": true}))
                    }
                    _ => bail!("unknown pull request method {method}"),
                }
            }
        }
    }
}

pub(crate) fn install(mut options: Options, service: Arc<Service>) -> Options {
    for method in [
        "pr.providers",
        "pr.detect",
        "pr.forBranch",
        "pr.get",
        "pr.threads",
        "pr.reply",
        "pr.comment",
        "pr.resolve",
    ] {
        let service = service.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            async move { service.call(method, params).await }
        });
    }
    options
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params[key]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow!("{key} is required"))
}

fn body(params: &Value) -> Result<&str> {
    let body = text(params, "body")?;
    if body.len() > 64 * 1024 {
        bail!("comment is too long");
    }
    Ok(body)
}

fn hint(params: &Value) -> Result<Option<Kind>> {
    match params["provider"].as_str() {
        None => Ok(None),
        Some(id) => Kind::from_id(id)
            .map(Some)
            .ok_or_else(|| anyhow!("unknown provider {id}")),
    }
}

/// An ambient host path: canonicalized once, and that spelling is what git
/// runs in.
fn cwd(params: &Value) -> Result<PathBuf> {
    let path = PathBuf::from(text(params, "cwd")?);
    if !path.is_absolute() {
        bail!("cwd must be an absolute directory");
    }
    let path = super::paths::canonicalize(&path)?;
    if !path.is_dir() {
        bail!("cwd must be a directory");
    }
    Ok(path)
}

/// Reads `projects.<key>.git` for the project containing `cwd`.
pub(crate) fn project_git(config: &Value, cwd: &Path) -> ProjectGit {
    // Slashes either way and (on Windows) any case name the same directory.
    let fold = |path: &str| -> String {
        let text = path.replace('\\', "/");
        let text = text.trim_end_matches('/');
        if cfg!(windows) {
            text.to_ascii_lowercase()
        } else {
            text.to_owned()
        }
    };
    let target = fold(&cwd.to_string_lossy());
    let best = config["projects"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| {
            let root = fold(key);
            !root.is_empty()
                && (target == root
                    || target
                        .strip_prefix(&root)
                        .is_some_and(|rest| rest.starts_with('/')))
        })
        .max_by_key(|(key, _)| key.len());
    let Some((key, project)) = best else {
        return ProjectGit::default();
    };
    let git = &project["git"];
    ProjectGit {
        project: Some(key.clone()),
        provider: git["provider"].as_str().and_then(Kind::from_id),
        remote: git["remote"]
            .as_str()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned),
    }
}

#[cfg(test)]
mod tests;

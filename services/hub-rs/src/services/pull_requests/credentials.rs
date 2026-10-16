//! Credentials come from the provider CLIs the user already signed in with:
//! `gh auth token` for GitHub, and `az account get-access-token` (or the
//! `AZURE_DEVOPS_EXT_PAT` the Azure DevOps extension also honours) for Azure
//! DevOps. They are held in memory only, and dropped on a 401 so the next
//! call asks the CLI again.
use super::remote::Kind;
use crate::services::{owned_process, provider_utilities::tools::on_path};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Azure DevOps' Entra ID application, the resource its tokens are for.
const AZURE_DEVOPS_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";
const CLI_TIMEOUT: Duration = Duration::from_secs(30);
/// gh tokens do not expire on their own; re-read now and then in case the
/// user switched accounts.
const GITHUB_TOKEN_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Auth {
    Bearer(String),
    Basic(String),
}

impl Auth {
    pub(crate) fn header(&self) -> String {
        match self {
            Auth::Bearer(token) => format!("Bearer {token}"),
            Auth::Basic(token) => format!("Basic {token}"),
        }
    }
}

pub(crate) struct Credentials {
    cache: Mutex<HashMap<(Kind, String), (Auth, Instant)>>,
    /// Tests hand out fixed tokens instead of running the CLIs.
    fixed: Option<HashMap<Kind, Auth>>,
}

impl Credentials {
    pub(crate) fn cli() -> Self {
        Self {
            cache: Default::default(),
            fixed: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixed(tokens: HashMap<Kind, Auth>) -> Self {
        Self {
            cache: Default::default(),
            fixed: Some(tokens),
        }
    }

    pub(crate) async fn get(&self, kind: Kind, host: &str) -> Result<Auth> {
        if let Some(fixed) = &self.fixed {
            return fixed
                .get(&kind)
                .cloned()
                .ok_or_else(|| anyhow!("not signed in to {}", kind.label()));
        }
        let key = (kind, host.to_owned());
        if let Some((auth, until)) = self.cache.lock().unwrap().get(&key)
            && Instant::now() < *until
        {
            return Ok(auth.clone());
        }
        let (auth, ttl) = match kind {
            Kind::Github => (github_token(host).await?, GITHUB_TOKEN_TTL),
            Kind::AzureDevops => azure_token().await?,
        };
        self.cache
            .lock()
            .unwrap()
            .insert(key, (auth.clone(), Instant::now() + ttl));
        Ok(auth)
    }

    /// The provider refused the token: ask the CLI again next time.
    pub(crate) fn forget(&self, kind: Kind, host: &str) {
        self.cache.lock().unwrap().remove(&(kind, host.to_owned()));
    }
}

/// The CLI each provider signs in with.
pub(crate) fn cli(kind: Kind) -> &'static str {
    match kind {
        Kind::Github => "gh",
        Kind::AzureDevops => "az",
    }
}

pub(crate) fn install_hint(kind: Kind) -> &'static str {
    match kind {
        Kind::Github => "Install the GitHub CLI (https://cli.github.com), then run `gh auth login`",
        Kind::AzureDevops => {
            "Install the Azure CLI (https://aka.ms/azcli), then run `az login` (or set AZURE_DEVOPS_EXT_PAT)"
        }
    }
}

/// Is the CLI installed, and is the user signed in? Never fails: problems
/// are reported in the returned value.
pub(crate) async fn status(credentials: &Credentials, kind: Kind, host: &str) -> Value {
    let path = on_path(cli(kind));
    let pat = kind == Kind::AzureDevops && azure_pat().is_some();
    let mut value = json!({
        "id": kind.id(),
        "label": kind.label(),
        "cli": cli(kind),
        "available": path.is_some() || pat,
        "install": install_hint(kind),
    });
    if let Some(path) = path {
        value["path"] = json!(path.to_string_lossy());
    }
    if value["available"] == true {
        match credentials.get(kind, host).await {
            Ok(_) => value["signedIn"] = json!(true),
            Err(error) => {
                value["signedIn"] = json!(false);
                value["error"] = json!(error.to_string());
            }
        }
    }
    value
}

async fn github_token(host: &str) -> Result<Auth> {
    let gh = on_path("gh").ok_or_else(|| anyhow!(install_hint(Kind::Github)))?;
    let mut command = tokio::process::Command::new(gh);
    command.args(["auth", "token", "--hostname", host]);
    let token = run(command, "gh").await?;
    if token.is_empty() {
        bail!("GitHub CLI is not signed in to {host}; run `gh auth login --hostname {host}`");
    }
    Ok(Auth::Bearer(token))
}

fn azure_pat() -> Option<String> {
    std::env::var("AZURE_DEVOPS_EXT_PAT")
        .ok()
        .map(|pat| pat.trim().to_owned())
        .filter(|pat| !pat.is_empty())
}

async fn azure_token() -> Result<(Auth, Duration)> {
    if let Some(pat) = azure_pat() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
        return Ok((Auth::Basic(encoded), Duration::from_secs(60 * 60)));
    }
    let az = on_path("az").ok_or_else(|| anyhow!(install_hint(Kind::AzureDevops)))?;
    let mut command = azure_command(az);
    command.args([
        "account",
        "get-access-token",
        "--resource",
        AZURE_DEVOPS_RESOURCE,
        "--output",
        "json",
    ]);
    let output = run(command, "az").await?;
    let value: Value = serde_json::from_str(&output).context("az returned unreadable JSON")?;
    let token = value["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow!("az returned no access token; run `az login`"))?;
    // Newer CLIs report epoch seconds; refresh a few minutes early.
    let ttl = value["expires_on"]
        .as_i64()
        .or_else(|| value["expires_on"].as_str().and_then(|s| s.parse().ok()))
        .map(|epoch| epoch - chrono::Utc::now().timestamp() - 300)
        .filter(|left| *left > 0)
        .map_or(Duration::from_secs(30 * 60), |left| {
            Duration::from_secs(left as u64)
        });
    Ok((Auth::Bearer(token.to_owned()), ttl))
}

/// `az` on Windows is `az.cmd`, a shim around the CLI's bundled Python. Run
/// that Python directly so no shell ever parses the arguments.
fn azure_command(az: PathBuf) -> tokio::process::Command {
    let is_shim = az
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let python = az
        .parent()
        .and_then(|bin| bin.parent())
        .map(|root| root.join("python.exe"))
        .filter(|python| python.is_file());
    match (is_shim, python) {
        (true, Some(python)) => {
            let mut command = tokio::process::Command::new(python);
            command.args(["-IBm", "azure.cli"]);
            command
        }
        // Only constant arguments ever reach this command.
        (true, None) => {
            let mut command = tokio::process::Command::new("cmd");
            command.arg("/C").arg(az);
            command
        }
        (false, _) => tokio::process::Command::new(az),
    }
}

async fn run(mut command: tokio::process::Command, name: &str) -> Result<String> {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command.env("GH_PROMPT_DISABLED", "1");
    command.stdin(std::process::Stdio::null());
    let output = owned_process::capture(&mut command, 1024 * 1024, CLI_TIMEOUT)
        .await
        .with_context(|| format!("could not run {name}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("no error output");
        bail!("{name} failed: {line}");
    }
    Ok(stdout)
}

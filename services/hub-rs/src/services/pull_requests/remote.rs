//! Which hosted repository a git remote or a pull request URL names.
//!
//! GitHub remotes are recognised on github.com, or on any host when the
//! project says it is GitHub (Enterprise). Azure DevOps remotes are recognised
//! in every spelling the service hands out: `dev.azure.com` (with or without
//! the `org@` user), the legacy `<org>.visualstudio.com` (with or without
//! `DefaultCollection`), and both SSH forms (`ssh.dev.azure.com:v3/…` and
//! `vs-ssh.visualstudio.com:v3/…`).
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    Github,
    AzureDevops,
}

impl Kind {
    pub(crate) const ALL: [Kind; 2] = [Kind::Github, Kind::AzureDevops];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Kind::Github => "github",
            Kind::AzureDevops => "azure-devops",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Github => "GitHub",
            Kind::AzureDevops => "Azure DevOps",
        }
    }
}

/// A hosted repository. For GitHub `owner` is the user or organisation; for
/// Azure DevOps it is the organisation and `project` holds the project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Repo {
    pub kind: Kind,
    pub host: String,
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub name: String,
}

impl Repo {
    /// "owner/name", or "org/project/name" for Azure DevOps.
    pub(crate) fn display(&self) -> String {
        match &self.project {
            Some(project) => format!("{}/{}/{}", self.owner, project, self.name),
            None => format!("{}/{}", self.owner, self.name),
        }
    }

    pub(crate) fn pull_request_url(&self, number: u64) -> String {
        match self.kind {
            Kind::Github => format!(
                "https://{}/{}/{}/pull/{number}",
                self.host, self.owner, self.name
            ),
            Kind::AzureDevops => format!(
                "https://{}/{}/{}/_git/{}/pullrequest/{number}",
                self.host,
                encode(&self.owner),
                encode(self.project.as_deref().unwrap_or_default()),
                encode(&self.name)
            ),
        }
    }
}

/// One pull request on a hosted repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrRef {
    pub repo: Repo,
    pub number: u64,
}

impl PrRef {
    pub(crate) fn url(&self) -> String {
        self.repo.pull_request_url(self.number)
    }
}

const AZURE_HOST: &str = "dev.azure.com";

/// The repository a remote URL names. `hint` is the project's configured
/// provider: it lets a GitHub Enterprise host count as GitHub.
pub(crate) fn parse_remote(remote: &str, hint: Option<Kind>) -> Option<Repo> {
    let (host, segments) = split(remote)?;
    azure(&host, &segments, false)
        .or_else(|| github(&host, &segments, hint, false).map(|(repo, _)| repo))
}

/// The pull request a web URL names.
pub(crate) fn parse_pull_request(url: &str, hint: Option<Kind>) -> Option<PrRef> {
    let (host, segments) = split(url)?;
    if let Some(repo) = azure(&host, &segments, true) {
        let at = segments
            .iter()
            .position(|s| s.eq_ignore_ascii_case("pullrequest"))?;
        let number = segments.get(at + 1)?.parse().ok()?;
        return Some(PrRef { repo, number });
    }
    let (repo, rest) = github(&host, &segments, hint, true)?;
    match rest {
        [pull, number, ..] if pull == "pull" => Some(PrRef {
            repo,
            number: number.parse().ok()?,
        }),
        _ => None,
    }
}

/// Host (lowercase, without port or user) and decoded path segments of an
/// http(s), ssh or scp-style (`user@host:path`) URL.
fn split(raw: &str) -> Option<(String, Vec<String>)> {
    let raw = raw.trim();
    let normalized = if raw.contains("://") {
        raw.to_owned()
    } else {
        // scp-like: [user@]host:path, never a Windows drive or a local path.
        let (authority, path) = raw.split_once(':')?;
        if authority.is_empty() || authority.contains('/') || authority.len() == 1 {
            return None;
        }
        format!("ssh://{authority}/{}", path.trim_start_matches('/'))
    };
    let url = url::Url::parse(&normalized).ok()?;
    if !matches!(url.scheme(), "https" | "http" | "ssh" | "git") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let segments = url
        .path_segments()?
        .filter(|s| !s.is_empty())
        .map(decode)
        .collect();
    Some((host, segments))
}

fn azure(host: &str, segments: &[String], web: bool) -> Option<Repo> {
    let repo = |org: &str, project: &str, name: &str| {
        Some(Repo {
            kind: Kind::AzureDevops,
            host: AZURE_HOST.into(),
            owner: org.to_owned(),
            project: Some(project.to_owned()),
            name: name.trim_end_matches(".git").to_owned(),
        })
    };
    // ssh.dev.azure.com:v3/org/project/repo, org@vs-ssh.visualstudio.com:v3/…
    if !web && (host == "ssh.dev.azure.com" || host == "vs-ssh.visualstudio.com") {
        return match segments {
            [v3, org, project, name, ..] if v3 == "v3" => repo(org, project, name),
            _ => None,
        };
    }
    let (org, rest) = if host == AZURE_HOST {
        let (org, rest) = segments.split_first()?;
        (org.as_str(), rest)
    } else {
        let org = host.strip_suffix(".visualstudio.com")?;
        let rest = match segments {
            [collection, rest @ ..] if collection.eq_ignore_ascii_case("DefaultCollection") => rest,
            rest => rest,
        };
        (org, rest)
    };
    match rest {
        [project, git, name, ..] if git == "_git" => repo(org, project, name),
        // A repository named like its project: dev.azure.com/org/_git/repo.
        [git, name, ..] if git == "_git" => repo(org, name, name),
        _ => None,
    }
}

fn github<'a>(
    host: &str,
    segments: &'a [String],
    hint: Option<Kind>,
    web: bool,
) -> Option<(Repo, &'a [String])> {
    let known = host == "github.com" || host == "www.github.com" || host == "ssh.github.com";
    if !known && hint != Some(Kind::Github) {
        return None;
    }
    let [owner, name, rest @ ..] = segments else {
        return None;
    };
    if !web && !rest.is_empty() {
        return None;
    }
    let host = if known { "github.com" } else { host };
    Some((
        Repo {
            kind: Kind::Github,
            host: host.into(),
            owner: owner.clone(),
            project: None,
            name: name.trim_end_matches(".git").to_owned(),
        },
        rest,
    ))
}

fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = segment.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes a path segment (Azure DevOps project names carry spaces).
pub(crate) fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(owner: &str, name: &str) -> Repo {
        Repo {
            kind: Kind::Github,
            host: "github.com".into(),
            owner: owner.into(),
            project: None,
            name: name.into(),
        }
    }

    fn ado(org: &str, project: &str, name: &str) -> Repo {
        Repo {
            kind: Kind::AzureDevops,
            host: AZURE_HOST.into(),
            owner: org.into(),
            project: Some(project.into()),
            name: name.into(),
        }
    }

    #[test]
    fn github_remotes_in_every_spelling() {
        for remote in [
            "https://github.com/DJTouchette/workspacer.git",
            "https://github.com/DJTouchette/workspacer",
            "https://user:token@github.com/DJTouchette/workspacer/",
            "git@github.com:DJTouchette/workspacer.git",
            "ssh://git@ssh.github.com:443/DJTouchette/workspacer.git",
        ] {
            assert_eq!(
                parse_remote(remote, None),
                Some(gh("DJTouchette", "workspacer")),
                "{remote}"
            );
        }
    }

    #[test]
    fn enterprise_hosts_need_the_project_to_say_github() {
        let remote = "git@git.example.com:team/app.git";
        assert_eq!(parse_remote(remote, None), None);
        let repo = parse_remote(remote, Some(Kind::Github)).unwrap();
        assert_eq!(
            (repo.host.as_str(), repo.display()),
            ("git.example.com", "team/app".into())
        );
    }

    #[test]
    fn azure_remotes_in_every_spelling() {
        let expected = ado("contoso", "Fabrikam Fiber", "web-app");
        for remote in [
            "https://dev.azure.com/contoso/Fabrikam%20Fiber/_git/web-app",
            "https://contoso@dev.azure.com/contoso/Fabrikam%20Fiber/_git/web-app",
            "https://contoso.visualstudio.com/Fabrikam%20Fiber/_git/web-app",
            "https://contoso.visualstudio.com/DefaultCollection/Fabrikam%20Fiber/_git/web-app",
            "git@ssh.dev.azure.com:v3/contoso/Fabrikam%20Fiber/web-app",
            "contoso@vs-ssh.visualstudio.com:v3/contoso/Fabrikam%20Fiber/web-app",
        ] {
            assert_eq!(
                parse_remote(remote, None),
                Some(expected.clone()),
                "{remote}"
            );
        }
        assert_eq!(
            parse_remote("https://dev.azure.com/contoso/_git/tools", None),
            Some(ado("contoso", "tools", "tools"))
        );
    }

    #[test]
    fn local_paths_and_other_hosts_are_not_repositories() {
        for remote in [
            "C:\\work\\repo",
            "/home/me/repo",
            "../repo",
            "https://gitlab.com/group/project.git",
        ] {
            assert_eq!(parse_remote(remote, None), None, "{remote}");
        }
    }

    #[test]
    fn pull_request_urls_round_trip() {
        let github = parse_pull_request(
            "https://github.com/DJTouchette/workspacer/pull/42/files",
            None,
        )
        .unwrap();
        assert_eq!(github.number, 42);
        assert_eq!(
            github.url(),
            "https://github.com/DJTouchette/workspacer/pull/42"
        );
        let azure = parse_pull_request(
            "https://contoso.visualstudio.com/Fabrikam%20Fiber/_git/web-app/pullrequest/7?_a=files",
            None,
        )
        .unwrap();
        assert_eq!(azure.repo, ado("contoso", "Fabrikam Fiber", "web-app"));
        assert_eq!(
            azure.url(),
            "https://dev.azure.com/contoso/Fabrikam%20Fiber/_git/web-app/pullrequest/7"
        );
        assert_eq!(parse_pull_request(&azure.url(), None), Some(azure));
        assert_eq!(
            parse_pull_request("https://github.com/DJTouchette/workspacer/issues/3", None),
            None
        );
    }
}

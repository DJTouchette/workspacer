//! A session's linked pull request (the hub's `pullRequest` snapshot field)
//! and the discussion `pr.threads` returns, for GitHub and Azure DevOps alike.
use serde_json::Value;

/// Where a link came from: a person, the session's branch, or the agent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    User,
    Branch,
    Agent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    /// `github` or `azure-devops`.
    pub provider: String,
    /// "owner/name", or "org/project/name" on Azure DevOps.
    pub repo: String,
    pub number: u64,
    /// Empty until the provider has answered once.
    pub title: String,
    /// `open`, `draft`, `merged` or `closed`; empty when not yet read.
    pub state: String,
    pub source_branch: String,
    pub source: Source,
    /// Unresolved review threads, when the hub has counted them.
    pub unresolved: Option<u32>,
}

fn text(value: &Value, key: &str, limit: usize) -> String {
    crate::transcript::head(value[key].as_str().unwrap_or("").trim(), limit)
}

impl Link {
    pub fn parse(value: &Value) -> Option<Self> {
        let url = text(value, "url", 2048);
        if url.is_empty() {
            return None;
        }
        Some(Self {
            url,
            provider: text(value, "provider", 32),
            repo: text(value, "repo", 300),
            number: value["number"].as_u64().unwrap_or_default(),
            title: text(value, "title", 500),
            state: text(value, "state", 16),
            source_branch: text(value, "sourceBranch", 300),
            source: match value["source"].as_str() {
                Some("branch") => Source::Branch,
                Some("agent") => Source::Agent,
                _ => Source::User,
            },
            unresolved: value["unresolved"]
                .as_u64()
                .map(|n| n.min(u32::MAX as u64) as u32),
        })
    }

    /// `#123`, or `!123` the way Azure DevOps names pull requests.
    pub fn short(&self) -> String {
        if self.provider == "azure-devops" {
            format!("!{}", self.number)
        } else {
            format!("#{}", self.number)
        }
    }

    pub fn provider_label(&self) -> &'static str {
        provider_label(&self.provider)
    }

    pub fn state_label(&self) -> &'static str {
        match self.state.as_str() {
            "draft" => "Draft",
            "merged" => "Merged",
            "closed" => "Closed",
            "open" => "Open",
            _ => "Linked",
        }
    }
}

/// Where `#number` goes in the repository of a linked pull request: the
/// issue (GitHub redirects it to a pull request of that number), or the work
/// item on Azure DevOps, where `#` names work items. `None` when the link's
/// address has neither shape.
pub fn reference_url(link: &Link, number: u64) -> Option<String> {
    match link.provider.as_str() {
        "github" => {
            let (base, _) = link.url.split_once("/pull/")?;
            Some(format!("{base}/issues/{number}"))
        }
        "azure-devops" => {
            let (base, _) = link.url.split_once("/_git/")?;
            Some(format!("{base}/_workitems/edit/{number}"))
        }
        _ => None,
    }
}

pub fn provider_label(provider: &str) -> &'static str {
    match provider {
        "github" => "GitHub",
        "azure-devops" => "Azure DevOps",
        _ => "Pull request",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub resolvable: bool,
    pub resolved: bool,
    pub outdated: bool,
    pub comments: Vec<Comment>,
}

impl Thread {
    /// `src/lib.rs:12`, `src/lib.rs`, or "Conversation".
    pub fn location(&self) -> String {
        match (&self.path, self.line) {
            (Some(path), Some(line)) => format!("{path}:{line}"),
            (Some(path), None) => path.clone(),
            _ => "Conversation".into(),
        }
    }

    pub fn open(&self) -> bool {
        self.resolvable && !self.resolved
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Threads {
    pub threads: Vec<Thread>,
    pub truncated: bool,
}

/// Most threads and comments kept from one `pr.threads` reply.
const MAX_THREADS: usize = 300;
const MAX_COMMENTS: usize = 100;

impl Threads {
    pub fn parse(value: &Value) -> Self {
        let threads = value["threads"]
            .as_array()
            .map(|threads| {
                threads
                    .iter()
                    .take(MAX_THREADS)
                    .map(|t| Thread {
                        id: text(t, "id", 200),
                        path: Some(text(t, "path", 1000)).filter(|p| !p.is_empty()),
                        line: t["line"].as_u64(),
                        resolvable: t["resolvable"] == true,
                        resolved: t["resolved"] == true,
                        outdated: t["outdated"] == true,
                        comments: t["comments"]
                            .as_array()
                            .map(|comments| {
                                comments
                                    .iter()
                                    .take(MAX_COMMENTS)
                                    .map(|c| Comment {
                                        id: text(c, "id", 200),
                                        author: text(c, "author", 200),
                                        body: text(c, "body", 64 * 1024),
                                        created_at: Some(text(c, "createdAt", 64))
                                            .filter(|t| !t.is_empty()),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .filter(|t| !t.id.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            threads,
            truncated: value["truncated"] == true,
        }
    }

    pub fn unresolved(&self) -> usize {
        self.threads.iter().filter(|t| t.open()).count()
    }
}

/// The message "Send to agent" puts in the composer: each thread's place
/// and its comments, quoted, with what to do about them.
pub fn agent_brief(link: &Link, threads: &[&Thread]) -> String {
    let mut out = format!(
        "Please address these review comments on {} {} ({}):\n",
        link.provider_label(),
        link.short(),
        link.url
    );
    for thread in threads {
        out.push_str(&format!(
            "\n- {} (thread {})\n",
            thread.location(),
            thread.id
        ));
        for comment in &thread.comments {
            let author = if comment.author.is_empty() {
                "someone"
            } else {
                &comment.author
            };
            for (i, line) in comment.body.lines().enumerate() {
                if i == 0 {
                    out.push_str(&format!("  > {author}: {line}\n"));
                } else {
                    out.push_str(&format!("  > {line}\n"));
                }
            }
        }
    }
    out.push_str(
        "\nWhen one is handled, reply on its thread and resolve it with the pull request tools.",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn references_resolve_in_the_linked_repository() {
        let github = Link::parse(&json!({"url":"https://github.com/o/r/pull/3","provider":"github","repo":"o/r","number":3})).unwrap();
        assert_eq!(
            reference_url(&github, 42).as_deref(),
            Some("https://github.com/o/r/issues/42")
        );
        let ado = Link::parse(&json!({"url":"https://dev.azure.com/o/p/_git/r/pullrequest/7","provider":"azure-devops","repo":"o/p/r","number":7})).unwrap();
        assert_eq!(
            reference_url(&ado, 42).as_deref(),
            Some("https://dev.azure.com/o/p/_workitems/edit/42")
        );
        let odd = Link::parse(&json!({"url":"https://example.com/x","provider":"github"})).unwrap();
        assert_eq!(reference_url(&odd, 1), None);
    }

    #[test]
    fn a_link_reads_the_snapshot_field() {
        let link = Link::parse(
            &json!({"url":"https://dev.azure.com/o/p/_git/r/pullrequest/7",
            "provider":"azure-devops","repo":"o/p/r","number":7,"title":"Fix","state":"draft",
            "source":"agent","unresolved":2}),
        )
        .unwrap();
        assert_eq!(link.short(), "!7");
        assert_eq!(
            (link.state_label(), link.source, link.unresolved),
            ("Draft", Source::Agent, Some(2))
        );
        assert!(Link::parse(&json!({"url":""})).is_none());
    }

    #[test]
    fn threads_parse_and_brief_quotes_every_comment() {
        let threads = Threads::parse(&json!({"truncated":true,"threads":[
            {"id":"conversation","resolvable":false,"resolved":false,"comments":[
                {"id":"1","author":"lead","body":"Looks close"}]},
            {"id":"T1","path":"src/lib.rs","line":12,"resolvable":true,"resolved":false,
             "comments":[{"id":"2","author":"rev","body":"Rename this\nand add a test"}]},
            {"id":"T2","resolvable":true,"resolved":true,"comments":[]}
        ]}));
        assert!(threads.truncated);
        assert_eq!(threads.unresolved(), 1);
        assert_eq!(threads.threads[1].location(), "src/lib.rs:12");
        let link = Link {
            url: "https://github.com/o/r/pull/3".into(),
            provider: "github".into(),
            number: 3,
            ..Default::default()
        };
        let brief = agent_brief(&link, &[&threads.threads[1]]);
        assert!(brief.contains("GitHub #3"));
        assert!(brief.contains("- src/lib.rs:12 (thread T1)"));
        assert!(brief.contains("  > rev: Rename this\n  > and add a test"));
    }
}

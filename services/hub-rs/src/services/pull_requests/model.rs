//! The provider-neutral shape of a pull request and its discussion, as the
//! bus returns it. GitHub and Azure DevOps both map onto these.
use super::remote::Kind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullRequest {
    pub provider: Kind,
    pub url: String,
    /// "owner/name" (GitHub) or "org/project/name" (Azure DevOps).
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub state: State,
    pub source_branch: String,
    pub target_branch: String,
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Comment {
    pub id: String,
    pub author: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A discussion on the pull request: a review thread anchored to a file and
/// line, or the general conversation (no `path`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Thread {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    /// Whether the provider can mark this thread resolved at all.
    pub resolvable: bool,
    pub resolved: bool,
    /// The code it was left on has since changed.
    pub outdated: bool,
    pub comments: Vec<Comment>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Threads {
    pub threads: Vec<Thread>,
    /// The provider had more than one page; only the first is shown.
    pub truncated: bool,
}

impl Threads {
    pub(crate) fn comment_ids(&self) -> impl Iterator<Item = String> + '_ {
        self.threads
            .iter()
            .flat_map(|t| t.comments.iter().map(|c| c.id.clone()))
            .filter(|id| !id.is_empty())
    }

    pub(crate) fn unresolved(&self) -> usize {
        self.threads
            .iter()
            .filter(|t| t.resolvable && !t.resolved)
            .count()
    }
}

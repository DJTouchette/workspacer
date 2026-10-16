//! Which pull request each session is linked to, persisted in
//! `<config>/pr-links.json` and keyed by session id, so adopted and external
//! sessions can be linked as well as the ones Workspacer launched.
//!
//! An explicit unlink is remembered (a link with no `url`): automatic linking
//! by branch or by the agent never overrides a choice the user made.
use super::{
    model::PullRequest,
    remote::{self, Kind},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Mutex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Source {
    /// Linked or unlinked by a person.
    User,
    /// The session's branch has this pull request open.
    Branch,
    /// The agent opened (or named) it.
    Agent,
}

impl Source {
    pub(crate) fn from_wire(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "branch" => Some(Self::Branch),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Link {
    /// None: the user unlinked it, and automatic linking stays off.
    #[serde(default)]
    pub url: Option<String>,
    pub source: Source,
    pub linked_at: i64,
    /// The last details read from the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    /// Comment ids the agent has already been told about.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub seen: BTreeSet<String>,
}

impl Link {
    pub(crate) fn new(url: String, source: Source, pull_request: Option<PullRequest>) -> Self {
        Self {
            url: Some(url),
            source,
            linked_at: chrono::Utc::now().timestamp_millis(),
            pull_request,
            unresolved: None,
            checked_at: None,
            seen: BTreeSet::new(),
        }
    }

    pub(crate) fn unlinked() -> Self {
        Self {
            url: None,
            source: Source::User,
            linked_at: chrono::Utc::now().timestamp_millis(),
            pull_request: None,
            unresolved: None,
            checked_at: None,
            seen: BTreeSet::new(),
        }
    }

    /// The snapshot's `pullRequest` field.
    pub(crate) fn wire(&self) -> Option<Value> {
        let url = self.url.as_ref()?;
        let mut value = json!({"url": url, "source": self.source});
        if let Some(pr) = &self.pull_request {
            value["provider"] = json!(pr.provider);
            value["repo"] = json!(pr.repo);
            value["number"] = json!(pr.number);
            value["title"] = json!(pr.title);
            value["state"] = json!(pr.state);
            value["sourceBranch"] = json!(pr.source_branch);
        } else if let Some(parsed) = remote::parse_pull_request(url, Some(Kind::Github))
            .or_else(|| remote::parse_pull_request(url, None))
        {
            value["provider"] = json!(parsed.repo.kind);
            value["repo"] = json!(parsed.repo.display());
            value["number"] = json!(parsed.number);
        }
        if let Some(unresolved) = self.unresolved {
            value["unresolved"] = json!(unresolved);
        }
        Some(value)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    sessions: BTreeMap<String, Link>,
}

pub(crate) struct Links {
    path: Option<PathBuf>,
    state: Mutex<BTreeMap<String, Link>>,
}

impl Links {
    pub(crate) fn open(path: PathBuf) -> Self {
        let sessions = std::fs::read(&path)
            .ok()
            .and_then(|bytes| match serde_json::from_slice::<File>(&bytes) {
                Ok(file) => Some(file.sessions),
                Err(error) => {
                    eprintln!("pull request links were unreadable and start empty: {error}");
                    None
                }
            })
            .unwrap_or_default();
        Self {
            path: Some(path),
            state: Mutex::new(sessions),
        }
    }

    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self {
            path: None,
            state: Default::default(),
        }
    }

    pub(crate) fn get(&self, session: &str) -> Option<Link> {
        self.state.lock().unwrap().get(session).cloned()
    }

    pub(crate) fn set(&self, session: &str, link: Link) -> Result<()> {
        self.update(|sessions| {
            sessions.insert(session.to_owned(), link);
        })
    }

    /// Change a link in place; a no-op when the session has none.
    pub(crate) fn modify(&self, session: &str, change: impl FnOnce(&mut Link)) -> Result<()> {
        self.update(|sessions| {
            if let Some(link) = sessions.get_mut(session) {
                change(link);
            }
        })
    }

    fn update(&self, change: impl FnOnce(&mut BTreeMap<String, Link>)) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.clone();
        change(&mut next);
        if next == *state {
            return Ok(());
        }
        if let Some(path) = &self.path {
            let bytes = serde_json::to_vec_pretty(&File {
                sessions: next.clone(),
            })?;
            crate::services::config::atomic_bytes(path, &bytes)?;
        }
        *state = next;
        Ok(())
    }

    /// Adds `pullRequest` to a session snapshot that has a link.
    pub(crate) fn enrich(&self, mut row: Value) -> Value {
        let Some(id) = row["sessionId"].as_str() else {
            return row;
        };
        let wire = self.state.lock().unwrap().get(id).and_then(Link::wire);
        if let (Some(wire), Some(object)) = (wire, row.as_object_mut()) {
            object.insert("pullRequest".into(), wire);
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_persist_and_an_unlink_is_remembered_but_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pr-links.json");
        let links = Links::open(path.clone());
        links
            .set(
                "a",
                Link::new("https://github.com/o/r/pull/5".into(), Source::Agent, None),
            )
            .unwrap();
        links.set("b", Link::unlinked()).unwrap();
        let reopened = Links::open(path);
        let a = reopened.enrich(json!({"sessionId": "a"}));
        assert_eq!(a["pullRequest"]["number"], 5);
        assert_eq!(a["pullRequest"]["repo"], "o/r");
        assert_eq!(a["pullRequest"]["source"], "agent");
        let b = reopened.enrich(json!({"sessionId": "b"}));
        assert!(b.get("pullRequest").is_none());
        assert_eq!(reopened.get("b").unwrap().source, Source::User);
    }
}

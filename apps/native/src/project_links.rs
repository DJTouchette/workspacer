//! Project links: the same project on different machines
//! (`docs/host-work-queue.md`, "Project links").
//!
//! Paths differ between machines, so the hub translates a local project
//! folder into each peer's own path: a link the owner saved, the same path,
//! or the one peer folder with the same git remote. Anything weaker (the
//! same folder name, or several folders sharing the remote) is only a
//! suggestion the owner accepts by saving a link.
//!
//! Everything here is parsed leniently. Older hubs send `queue.targets`
//! cwds as plain strings, with no `name` or `remote`: the name is derived
//! from the path and the remote is unknown (`None`), never "no remote".
use serde_json::Value;

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").trim().to_owned()
}

/// The final component of a path, for either separator: `"app"` for
/// `/Users/dj/code/app/` or `C:\code\app`.
pub fn basename(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_owned()
}

/// One directory a host offers (`queue.targets[].cwds`, a link matrix
/// cell's `choices`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directory {
    pub path: String,
    /// The final path component (derived when the hub did not send it).
    pub name: String,
    /// The normalized `origin` URL; `None` when there is none or the hub
    /// did not say (older hubs).
    pub remote: Option<String>,
    pub git: Option<bool>,
    /// Where the host found it (`project`, `live`, …); empty when unsaid.
    pub source: String,
}

impl Directory {
    /// A plain path string (older hubs) or a directory object; `None` for
    /// anything without a path.
    pub fn parse(value: &Value) -> Option<Self> {
        if let Some(path) = value.as_str() {
            let path = path.trim();
            return (!path.is_empty()).then(|| Self::from_path(path));
        }
        let path = text(value, "path");
        if path.is_empty() {
            return None;
        }
        let name = match text(value, "name") {
            name if name.is_empty() => basename(&path),
            name => name,
        };
        Some(Self {
            name,
            remote: value["remote"]
                .as_str()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_owned),
            git: value["git"].as_bool(),
            source: text(value, "source"),
            path,
        })
    }

    pub fn from_path(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            name: basename(path),
            ..Default::default()
        }
    }

    pub fn parse_list(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Self::parse)
            .collect()
    }
}

/// How a local folder was matched to a peer's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    /// A link the owner saved.
    Link,
    /// The same path exists there.
    Exact,
    /// The one folder there with the same git remote.
    Remote,
    /// A way this client does not know; shown as the hub spelled it.
    Other(String),
}

impl Via {
    /// `None` for null or empty (nothing was used).
    pub fn parse(value: &Value) -> Option<Self> {
        let via = value.as_str()?.trim();
        Some(match via {
            "" => return None,
            "link" => Self::Link,
            "exact" => Self::Exact,
            "remote" => Self::Remote,
            other => Self::Other(other.to_owned()),
        })
    }

    /// The Hosts → Projects wording: `"linked"`, `"matched by git
    /// remote"`, `"same path"`.
    pub fn label(&self) -> &str {
        match self {
            Self::Link => "linked",
            Self::Exact => "same path",
            Self::Remote => "matched by git remote",
            Self::Other(other) => other,
        }
    }

    /// The launch picker's shorter wording: `"linked"`, `"git remote"`,
    /// `"same path"`.
    pub fn short(&self) -> &str {
        match self {
            Self::Remote => "git remote",
            other => other.label(),
        }
    }
}

/// A peer folder that may be the same project, never used without a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub path: String,
    /// `name` or `remote` (several folders share the remote).
    pub why: String,
}

impl Suggestion {
    pub fn parse_list(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                // Tolerate a bare path string.
                let (path, why) = match s.as_str() {
                    Some(path) => (path.trim().to_owned(), String::new()),
                    None => (text(s, "path"), text(s, "why")),
                };
                (!path.is_empty()).then_some(Self { path, why })
            })
            .collect()
    }

    /// `"same name"`, `"same git remote"`.
    pub fn label(&self) -> &str {
        match self.why.as_str() {
            "name" => "same name",
            "remote" => "same git remote",
            "" => "suggested",
            other => other,
        }
    }
}

/// A saved link (`project-links.json`, keyed by local path and peer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub local_path: String,
    pub peer: String,
    pub peer_path: String,
    pub created_at: Option<i64>,
}

/// `projects.links`, `projects.setLink`, `projects.removeLink` replies and
/// the `projects.linksChanged` event: `{links: [...]}`. `None` when there
/// is no `links` array.
pub fn parse_links(value: &Value) -> Option<Vec<Link>> {
    Some(
        value["links"]
            .as_array()?
            .iter()
            .filter_map(|l| {
                let (local_path, peer, peer_path) =
                    (text(l, "localPath"), text(l, "peer"), text(l, "peerPath"));
                (!local_path.is_empty() && !peer.is_empty() && !peer_path.is_empty()).then(|| {
                    Link {
                        local_path,
                        peer,
                        peer_path,
                        created_at: l["createdAt"].as_i64().filter(|&at| at > 0),
                    }
                })
            })
            .collect(),
    )
}

/// What the hub used for a (project, peer) pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub path: String,
    pub via: Option<Via>,
}

/// One peer's column for one local project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerCell {
    pub peer: String,
    pub connected: bool,
    pub resolved: Option<Resolved>,
    pub suggestions: Vec<Suggestion>,
    /// Every folder the peer offers (empty while it is unreachable).
    pub choices: Vec<Directory>,
}

/// One local project's row of `projects.linkMatrix`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub path: String,
    pub name: String,
    pub remote: Option<String>,
    pub peers: Vec<PeerCell>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matrix {
    pub projects: Vec<Project>,
}

impl Matrix {
    /// `None` when there is no `projects` array.
    pub fn parse(value: &Value) -> Option<Self> {
        let projects = value["projects"].as_array()?;
        Some(Self {
            projects: projects
                .iter()
                .filter_map(|p| {
                    let dir = Directory::parse(p)?;
                    Some(Project {
                        peers: p["peers"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| {
                                let peer = text(c, "peer");
                                (!peer.is_empty()).then(|| PeerCell {
                                    connected: c["connected"] == true,
                                    resolved: {
                                        let r = &c["resolved"];
                                        let path = text(r, "path");
                                        (!path.is_empty()).then(|| Resolved {
                                            path,
                                            via: Via::parse(&r["via"]),
                                        })
                                    },
                                    suggestions: Suggestion::parse_list(&c["suggestions"]),
                                    choices: Directory::parse_list(&c["choices"]),
                                    peer,
                                })
                            })
                            .collect(),
                        path: dir.path,
                        name: dir.name,
                        remote: dir.remote,
                    })
                })
                .collect(),
        })
    }

    /// Every peer named in any row, in first-seen order.
    pub fn peers(&self) -> Vec<String> {
        let mut peers: Vec<String> = Vec::new();
        for cell in self.projects.iter().flat_map(|p| &p.peers) {
            if !peers.contains(&cell.peer) {
                peers.push(cell.peer.clone());
            }
        }
        peers
    }
}

/// What a matrix cell shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellState {
    /// The owner's saved link is in use.
    Linked(String),
    /// Matched automatically: same path, or the one same-remote folder.
    Matched { path: String, via: Via },
    /// Nothing used; these may be it.
    Suggested(Vec<Suggestion>),
    /// Nothing used and nothing suggested.
    Missing,
}

/// A cell as the Hosts → Projects section renders it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellView {
    pub state: CellState,
    /// The saved link for this pair, if any; it can be removed even when
    /// its path is gone from the peer (resolution then fell through).
    pub saved: Option<Link>,
    pub offline: bool,
}

impl CellView {
    pub fn new(project: &Project, cell: &PeerCell, links: &[Link]) -> Self {
        let saved = links
            .iter()
            .find(|l| l.local_path == project.path && l.peer == cell.peer)
            .cloned();
        let state = match &cell.resolved {
            Some(Resolved {
                path,
                via: Some(Via::Link),
            }) => CellState::Linked(path.clone()),
            Some(Resolved { path, via }) => CellState::Matched {
                path: path.clone(),
                via: via.clone().unwrap_or(Via::Other("matched".into())),
            },
            None if !cell.suggestions.is_empty() => CellState::Suggested(cell.suggestions.clone()),
            None => CellState::Missing,
        };
        Self {
            state,
            saved,
            offline: !cell.connected,
        }
    }

    /// Why a saved link is not in use (the hub fell through because its
    /// folder is gone from the peer): `"Saved link /x is not on mac now."`
    pub fn stale_note(&self, peer: &str) -> Option<String> {
        let saved = self.saved.as_ref()?;
        (!matches!(self.state, CellState::Linked(_)))
            .then(|| format!("Saved link {} is not on {peer} now.", saved.peer_path))
    }
}

/// `projects.linkMatrix` and `projects.links`, read together: each part is
/// the method's result or `{"error": "..."}`.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub matrix: Result<Matrix, String>,
    pub links: Result<Vec<Link>, String>,
}

impl Snapshot {
    pub fn parse(value: &Value) -> Self {
        fn part<T>(value: &Value, parse: impl Fn(&Value) -> Option<T>) -> Result<T, String> {
            if let Some(error) = value.get("error").and_then(Value::as_str)
                && value.as_object().is_some_and(|m| m.len() == 1)
            {
                return Err(error.to_owned());
            }
            parse(value).ok_or_else(|| "The hub returned an unexpected reply.".to_owned())
        }
        Self {
            matrix: part(&value["matrix"], Matrix::parse),
            links: part(&value["links"], parse_links),
        }
    }

    /// The hub predates project links.
    pub fn unsupported(&self) -> bool {
        matches!(&self.matrix, Err(e) if crate::hosts::unsupported(e))
    }

    /// The saved links, empty when they could not be read.
    pub fn saved(&self) -> &[Link] {
        self.links.as_deref().unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn directories_parse_strings_and_objects() {
        let dirs = Directory::parse_list(&json!([
            "/srv/api/",
            "",
            {"path": "/Users/dj/code/app", "name": "app", "remote": "github.com/dj/app",
             "git": true, "source": "project"},
            {"path": "C:\\code\\web", "remote": null},
            {"name": "no path"},
            42
        ]));
        assert_eq!(dirs.len(), 3);
        assert_eq!(dirs[0].path, "/srv/api/");
        assert_eq!(dirs[0].name, "api", "derived from an old hub's string");
        assert_eq!(dirs[0].remote, None, "unknown, not empty");
        assert_eq!(dirs[1].remote.as_deref(), Some("github.com/dj/app"));
        assert_eq!(dirs[1].git, Some(true));
        assert_eq!(dirs[1].source, "project");
        assert_eq!(dirs[2].name, "web");
        assert_eq!(basename("/"), "");
        assert_eq!(basename("app"), "app");
    }

    #[test]
    fn via_and_suggestion_labels() {
        assert_eq!(Via::parse(&json!("link")), Some(Via::Link));
        assert_eq!(Via::parse(&json!(null)), None);
        assert_eq!(Via::parse(&json!("")), None);
        assert_eq!(Via::Link.label(), "linked");
        assert_eq!(Via::Remote.label(), "matched by git remote");
        assert_eq!(Via::Remote.short(), "git remote");
        assert_eq!(Via::Exact.label(), "same path");
        assert_eq!(Via::parse(&json!("alias")).unwrap().label(), "alias");
        let s = Suggestion::parse_list(&json!([
            {"path": "/a", "why": "name"},
            {"path": "/b", "why": "remote"},
            "/c",
            {"why": "name"}
        ]));
        let labels: Vec<_> = s.iter().map(Suggestion::label).collect();
        assert_eq!(labels, ["same name", "same git remote", "suggested"]);
    }

    #[test]
    fn links_parse_from_replies_and_events() {
        let links = parse_links(&json!({"links": [
            {"localPath": "/home/dj/app", "peer": "mac", "peerPath": "/Users/dj/app", "createdAt": 5},
            {"localPath": "/home/dj/x", "peer": "", "peerPath": "/y"}
        ]}))
        .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].created_at, Some(5));
        assert!(parse_links(&json!({})).is_none());
        assert_eq!(parse_links(&json!({"links": []})), Some(vec![]));
    }

    fn matrix() -> Value {
        json!({"projects": [
            {"path": "/home/dj/app", "name": "app", "remote": "github.com/dj/app", "peers": [
                {"peer": "mac", "connected": true,
                 "resolved": {"path": "/Users/dj/app", "via": "link"},
                 "suggestions": [], "choices": [{"path": "/Users/dj/app", "remote": null}]},
                {"peer": "gpu", "connected": true,
                 "resolved": {"path": "/srv/app", "via": "remote"}, "suggestions": [],
                 "choices": ["/srv/app"]}
            ]},
            {"path": "/home/dj/web", "peers": [
                {"peer": "mac", "connected": true, "resolved": null,
                 "suggestions": [{"path": "/Users/dj/web", "why": "name"}],
                 "choices": [{"path": "/Users/dj/web", "name": "web", "remote": null}]},
                {"peer": "laptop", "connected": false, "resolved": null, "suggestions": [],
                 "choices": []}
            ]}
        ]})
    }

    #[test]
    fn matrix_cells_say_how_each_peer_matched() {
        let snapshot = Snapshot::parse(&json!({
            "matrix": matrix(),
            "links": {"links": [
                {"localPath": "/home/dj/app", "peer": "mac", "peerPath": "/Users/dj/app"},
                {"localPath": "/home/dj/web", "peer": "laptop", "peerPath": "/Users/dj/web-old"}
            ]}
        }));
        assert!(!snapshot.unsupported());
        let m = snapshot.matrix.as_ref().unwrap();
        assert_eq!(m.peers(), ["mac", "gpu", "laptop"]);
        assert_eq!(m.projects[1].name, "web", "derived");
        let cell = |r: usize, c: usize| {
            CellView::new(&m.projects[r], &m.projects[r].peers[c], snapshot.saved())
        };
        let linked = cell(0, 0);
        assert_eq!(linked.state, CellState::Linked("/Users/dj/app".into()));
        assert!(linked.saved.is_some());
        assert_eq!(linked.stale_note("mac"), None);
        assert_eq!(
            cell(0, 1).state,
            CellState::Matched {
                path: "/srv/app".into(),
                via: Via::Remote
            }
        );
        assert!(cell(0, 1).saved.is_none());
        match cell(1, 0).state {
            CellState::Suggested(s) => assert_eq!(s[0].path, "/Users/dj/web"),
            other => panic!("{other:?}"),
        }
        let offline = cell(1, 1);
        assert!(offline.offline);
        assert_eq!(offline.state, CellState::Missing);
        assert_eq!(
            offline.stale_note("laptop").unwrap(),
            "Saved link /Users/dj/web-old is not on laptop now."
        );
    }

    #[test]
    fn older_hubs_and_failed_parts() {
        let old = Snapshot::parse(&json!({
            "matrix": {"error": "no provider for projects.linkMatrix"},
            "links": {"error": "no provider for projects.links"}
        }));
        assert!(old.unsupported());
        assert!(old.saved().is_empty());
        let flaky = Snapshot::parse(&json!({
            "matrix": {"error": "timed out"},
            "links": {"links": []}
        }));
        assert!(!flaky.unsupported());
        assert!(flaky.matrix.is_err());
        assert!(Snapshot::parse(&json!({})).matrix.is_err());
    }
}

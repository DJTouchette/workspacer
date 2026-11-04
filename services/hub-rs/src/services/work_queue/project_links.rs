//! Project links: the same project on different machines
//! (docs/host-work-queue.md, "Project links"). Paths differ between hosts, so
//! the origin translates the submitter's local cwd into a peer's own path
//! before checking eligibility or forwarding. [`resolve`] is the pure rule;
//! [`Links`] is the owner's saved (`localPath`, `peer`) → `peerPath` table in
//! `project-links.json` beside `queue.json`.
//!
//! A peer's advertised choices only choose a path; the destination still
//! re-checks that the folder exists before it accepts the item.
use super::routing::peer_name_shaped;
use crate::services::{paths, remote_dispatch::Directory};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex};

/// At most this many saved links.
pub const MAX_LINKS: usize = 1000;
/// Longest accepted path, in bytes.
const MAX_PATH: usize = 4096;

/// How a cwd was resolved for a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// The owner's saved link.
    Link,
    /// The same path exists on the peer.
    Exact,
    /// The only peer choice with the same normalized `origin`.
    Remote,
}
impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Exact => "exact",
            Self::Remote => "remote",
        }
    }
}

/// A peer choice offered but not used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub path: String,
    /// `remote` (an ambiguous origin match) or `name` (same folder name).
    pub why: &'static str,
}

/// What [`resolve`] decided for one (cwd, peer).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    /// The peer path to use; `None` when nothing may be used.
    pub path: Option<String>,
    pub via: Option<Via>,
    /// Only when nothing was used: remote matches first, then name matches.
    pub suggestions: Vec<Suggestion>,
    /// Diagnostics for `reasons` (a saved link whose path vanished).
    pub notes: Vec<String>,
}
impl Resolution {
    pub fn suggestions_wire(&self) -> Value {
        json!(
            self.suggestions
                .iter()
                .map(|s| json!({"path": s.path, "why": s.why}))
                .collect::<Vec<_>>()
        )
    }
    /// `{path, via, suggestions}`.
    pub fn wire(&self) -> Value {
        json!({
            "path": self.path,
            "via": self.via.map(Via::as_str),
            "suggestions": self.suggestions_wire(),
        })
    }
    /// The refusal/ineligibility text for a cwd with no match on `peer`.
    pub fn no_match(&self, cwd: &str, peer: &str) -> String {
        let mut text = format!("cwd {cwd} has no match on {peer}");
        if !self.suggestions.is_empty() {
            let paths: Vec<&str> = self.suggestions.iter().map(|s| s.path.as_str()).collect();
            text.push_str(&format!(
                " (suggested: {} — link it in Hosts → Projects)",
                paths.join(", ")
            ));
        }
        text
    }
}

/// Paths compare as strings, ignoring trailing separators (never the root's).
pub fn same_path(a: &str, b: &str) -> bool {
    fn trim(p: &str) -> &str {
        let t = p.trim_end_matches(['/', '\\']);
        if t.is_empty() {
            &p[..p.len().min(1)]
        } else {
            t
        }
    }
    trim(a) == trim(b)
}

/// Resolve local cwd `cwd` (with its normalized `remote`, if it is a
/// repository root that has one) for a peer advertising `choices`, given the
/// owner's saved `link` for (`cwd`, peer). First match wins:
///
/// 1. `link`, while that path is still among the choices (a vanished one is
///    noted and resolution continues);
/// 2. `exact`: `cwd` itself is a choice;
/// 3. `remote`: exactly one choice has the same remote (two or more are
///    ambiguous: none is used, all become suggestions);
/// 4. nothing, with same-`name` choices as suggestions. A name match is never
///    used without a saved link.
pub fn resolve(
    cwd: &str,
    remote: Option<&str>,
    link: Option<&str>,
    choices: &[Directory],
) -> Resolution {
    let mut out = Resolution::default();
    let find = |path: &str| choices.iter().find(|c| same_path(&c.path, path));
    if let Some(link) = link {
        if let Some(choice) = find(link) {
            out.path = Some(choice.path.clone());
            out.via = Some(Via::Link);
            return out;
        }
        out.notes.push(format!(
            "saved link {link} is no longer among this host's directory choices"
        ));
    }
    if let Some(choice) = find(cwd) {
        out.path = Some(choice.path.clone());
        out.via = Some(Via::Exact);
        return out;
    }
    let remote = remote.filter(|r| !r.is_empty());
    let by_remote: Vec<&Directory> = match remote {
        Some(remote) => choices
            .iter()
            .filter(|c| c.remote.as_deref() == Some(remote))
            .collect(),
        None => Vec::new(),
    };
    if let [only] = by_remote.as_slice() {
        out.path = Some(only.path.clone());
        out.via = Some(Via::Remote);
        return out;
    }
    for choice in &by_remote {
        out.suggestions.push(Suggestion {
            path: choice.path.clone(),
            why: "remote",
        });
    }
    let name = Directory::name_of(cwd);
    if !name.is_empty() {
        for choice in choices {
            if choice.display_name() == name
                && !out.suggestions.iter().any(|s| s.path == choice.path)
            {
                out.suggestions.push(Suggestion {
                    path: choice.path.clone(),
                    why: "name",
                });
            }
        }
    }
    out
}

/// The lookup key for a local path: canonical when it exists, else as given
/// (a link may outlive its local folder and must still be removable).
pub fn local_key(path: &str) -> String {
    let path = path.trim();
    match paths::canonicalize(std::path::Path::new(path)) {
        Ok(canonical) => canonical.to_string_lossy().into_owned(),
        Err(_) => {
            let trimmed = path.trim_end_matches(['/', '\\']);
            if trimmed.is_empty() {
                path.to_owned()
            } else {
                trimmed.to_owned()
            }
        }
    }
}

/// Absolute on any OS a peer may run: `/x`, `C:\x`, `C:/x` or `\\server\x`.
pub fn absolute_anywhere(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub local_path: String,
    pub peer: String,
    pub peer_path: String,
    #[serde(default)]
    pub created_at: i64,
}

/// The saved links, persisted atomically (0600) on every change.
pub struct Links {
    path: PathBuf,
    rows: Mutex<Vec<Link>>,
}

fn field(params: &Value, key: &str) -> Result<String> {
    let raw = params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{key} is required and must be a non-empty string"))?;
    let text = raw.trim();
    if text.is_empty() {
        bail!("{key} is required and must be a non-empty string");
    }
    if raw.len() > MAX_PATH || raw.chars().any(char::is_control) {
        bail!("{key} is too long or contains control characters");
    }
    Ok(text.to_owned())
}
fn known(params: &Value, method: &str, keys: &[&str]) -> Result<()> {
    let map = params
        .as_object()
        .ok_or_else(|| anyhow!("{method} takes an object"))?;
    for key in map.keys() {
        if !keys.contains(&key.as_str()) {
            bail!("unknown {method} field {key:?}");
        }
    }
    Ok(())
}
fn local_path(params: &Value) -> Result<String> {
    let path = field(params, "localPath")?;
    if !std::path::Path::new(&path).is_absolute() {
        bail!("localPath must be an absolute path on this host");
    }
    Ok(local_key(&path))
}
fn peer(params: &Value) -> Result<String> {
    let peer = field(params, "peer")?;
    if !peer_name_shaped(&peer) || peer.len() > 128 || matches!(peer.as_str(), "local" | "any") {
        bail!("peer must be a federation peer name");
    }
    Ok(peer)
}

impl Links {
    /// Load the file (missing is empty). A damaged file is renamed aside and
    /// the table starts empty, like the queue file.
    pub fn open(path: PathBuf) -> Result<Self> {
        let rows = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes)
                .ok()
                .filter(Value::is_object)
                .and_then(|v| match &v["links"] {
                    Value::Null => Some(Vec::new()),
                    links => serde_json::from_value::<Vec<Link>>(links.clone()).ok(),
                }) {
                Some(rows) => rows,
                None => {
                    let aside = path.with_extension(format!(
                        "damaged-{}.json",
                        chrono::Utc::now().timestamp_millis()
                    ));
                    std::fs::rename(&path, &aside)?;
                    eprintln!(
                        "project links: {} was unreadable; kept as {} and starting empty",
                        path.display(),
                        aside.display()
                    );
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            rows: Mutex::new(rows),
        })
    }
    pub fn list(&self) -> Vec<Link> {
        self.rows.lock().unwrap().clone()
    }
    /// `{links: [...]}`.
    pub fn wire(rows: &[Link]) -> Value {
        json!({ "links": rows })
    }
    /// The saved peer path for a local cwd, looked up by its [`local_key`].
    pub fn get(&self, local: &str, peer: &str) -> Option<String> {
        let key = local_key(local);
        self.rows
            .lock()
            .unwrap()
            .iter()
            .find(|l| l.peer == peer && same_path(&l.local_path, &key))
            .map(|l| l.peer_path.clone())
    }
    fn change(
        &self,
        update: impl FnOnce(&mut Vec<Link>) -> Result<bool>,
    ) -> Result<(bool, Vec<Link>)> {
        let mut rows = self.rows.lock().unwrap();
        let mut next = rows.clone();
        if !update(&mut next)? {
            return Ok((false, next));
        }
        next.sort_by(|a, b| (&a.local_path, &a.peer).cmp(&(&b.local_path, &b.peer)));
        super::super::atomic_json(&self.path, &Self::wire(&next), true)
            .map_err(|error| error.context("project links were not saved"))?;
        *rows = next.clone();
        Ok((true, next))
    }
    /// `projects.setLink {localPath, peer, peerPath}`. Replaces the link for
    /// (`localPath`, `peer`). `peerPath` need not be among the peer's current
    /// choices (it may be offline). Returns whether anything changed.
    pub fn set(&self, params: &Value, now: i64) -> Result<(bool, Vec<Link>)> {
        known(
            params,
            "projects.setLink",
            &["localPath", "peer", "peerPath"],
        )?;
        let local_path = local_path(params)?;
        let peer = peer(params)?;
        let peer_path = field(params, "peerPath")?;
        if !absolute_anywhere(&peer_path) {
            bail!("peerPath must be an absolute path");
        }
        self.change(|rows| {
            if let Some(row) = rows
                .iter_mut()
                .find(|l| l.peer == peer && l.local_path == local_path)
            {
                if row.peer_path == peer_path {
                    return Ok(false);
                }
                row.peer_path = peer_path;
                row.created_at = now;
                return Ok(true);
            }
            if rows.len() >= MAX_LINKS {
                bail!("at most {MAX_LINKS} project links may be saved");
            }
            rows.push(Link {
                local_path,
                peer,
                peer_path,
                created_at: now,
            });
            Ok(true)
        })
    }
    /// `projects.removeLink {localPath, peer}`. Removing a link that is not
    /// there changes nothing and is not an error.
    pub fn remove(&self, params: &Value) -> Result<(bool, Vec<Link>)> {
        known(params, "projects.removeLink", &["localPath", "peer"])?;
        let local_path = local_path(params)?;
        let peer = peer(params)?;
        self.change(|rows| {
            let before = rows.len();
            rows.retain(|l| !(l.peer == peer && same_path(&l.local_path, &local_path)));
            Ok(rows.len() != before)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &str, remote: Option<&str>) -> Directory {
        Directory {
            path: path.into(),
            name: Directory::name_of(path),
            remote: remote.map(Into::into),
            source: "project".into(),
            git: remote.is_some(),
        }
    }
    const GH: Option<&str> = Some("github.com/dj/app");

    #[test]
    fn a_saved_link_wins_over_exact_and_remote_matches() {
        let choices = [
            dir("/home/dj/code/app", GH),
            dir("/Users/dj/app", GH),
            dir("/Users/dj/other", None),
        ];
        let r = resolve("/home/dj/code/app", GH, Some("/Users/dj/other/"), &choices);
        assert_eq!(r.path.as_deref(), Some("/Users/dj/other"));
        assert_eq!(r.via, Some(Via::Link));
        assert!(r.suggestions.is_empty() && r.notes.is_empty());
        assert_eq!(
            r.wire(),
            json!({"path":"/Users/dj/other","via":"link","suggestions":[]})
        );
    }

    #[test]
    fn a_stale_link_is_reported_and_resolution_falls_through() {
        let choices = [dir("/Users/dj/app", GH)];
        let r = resolve("/home/dj/app", GH, Some("/Users/dj/gone"), &choices);
        assert_eq!(r.path.as_deref(), Some("/Users/dj/app"));
        assert_eq!(r.via, Some(Via::Remote));
        assert_eq!(
            r.notes,
            ["saved link /Users/dj/gone is no longer among this host's directory choices"]
        );
        // Falling all the way through still keeps the note.
        let r = resolve("/home/dj/app", None, Some("/Users/dj/gone"), &[]);
        assert_eq!(r.path, None);
        assert_eq!(r.notes.len(), 1);
    }

    #[test]
    fn the_same_path_on_the_peer_is_an_exact_match() {
        let choices = [dir("/work/repo", None), dir("/elsewhere/repo", GH)];
        let r = resolve("/work/repo", GH, None, &choices);
        assert_eq!(
            (r.path.as_deref(), r.via),
            (Some("/work/repo"), Some(Via::Exact))
        );
        // Trailing separators do not matter.
        let r = resolve("/work/repo/", None, None, &choices);
        assert_eq!(r.via, Some(Via::Exact));
    }

    #[test]
    fn a_unique_remote_match_is_used_automatically() {
        let choices = [
            dir("/Users/dj/code/app", GH),
            dir("/Users/dj/code/web", Some("github.com/dj/web")),
            dir("/Users/dj/scratch/app", None),
        ];
        let r = resolve("/home/dj/app", GH, None, &choices);
        assert_eq!(r.path.as_deref(), Some("/Users/dj/code/app"));
        assert_eq!(r.via, Some(Via::Remote));
        assert!(r.suggestions.is_empty());
    }

    #[test]
    fn an_ambiguous_remote_is_never_used_and_every_match_is_suggested() {
        let choices = [
            dir("/Users/dj/app", GH),
            dir("/Users/dj/worktrees/feature", GH),
            dir("/Users/dj/old/app", None),
        ];
        let r = resolve("/home/dj/app", GH, None, &choices);
        assert_eq!(r.path, None);
        assert_eq!(r.via, None);
        assert_eq!(
            r.suggestions,
            [
                Suggestion {
                    path: "/Users/dj/app".into(),
                    why: "remote"
                },
                Suggestion {
                    path: "/Users/dj/worktrees/feature".into(),
                    why: "remote"
                },
                Suggestion {
                    path: "/Users/dj/old/app".into(),
                    why: "name"
                },
            ]
        );
        assert_eq!(
            r.no_match("/home/dj/app", "mac"),
            "cwd /home/dj/app has no match on mac (suggested: /Users/dj/app, /Users/dj/worktrees/feature, /Users/dj/old/app — link it in Hosts → Projects)"
        );
        assert_eq!(
            r.wire()["suggestions"][2],
            json!({"path":"/Users/dj/old/app","why":"name"})
        );
    }

    #[test]
    fn a_name_match_is_only_a_suggestion() {
        let choices = [dir("/Users/dj/app", Some("github.com/someone/app"))];
        for remote in [GH, None] {
            let r = resolve("/a/app", remote, None, &choices);
            assert_eq!(r.path, None);
            assert_eq!(
                r.suggestions,
                [Suggestion {
                    path: "/Users/dj/app".into(),
                    why: "name"
                }]
            );
            assert_eq!(
                r.no_match("/a/app", "mac"),
                "cwd /a/app has no match on mac (suggested: /Users/dj/app — link it in Hosts → Projects)"
            );
        }
        let r = resolve("/a/unrelated", GH, None, &choices);
        assert_eq!(r, Resolution::default());
        assert_eq!(
            r.no_match("/a/unrelated", "mac"),
            "cwd /a/unrelated has no match on mac"
        );
    }

    #[test]
    fn an_older_peer_without_names_or_remotes_still_matches_by_path_and_name() {
        let old = |path: &str| Directory {
            path: path.into(),
            source: "project".into(),
            git: true,
            ..Default::default()
        };
        let choices = [old("C:\\Users\\dj\\app"), old("/srv/app")];
        let r = resolve("/home/dj/app", GH, None, &choices);
        assert_eq!(r.path, None);
        assert_eq!(r.suggestions.len(), 2);
        assert!(r.suggestions.iter().all(|s| s.why == "name"));
        let r = resolve("/srv/app", GH, None, &choices);
        assert_eq!(r.via, Some(Via::Exact));
        let r = resolve("/home/dj/app", GH, Some("C:\\Users\\dj\\app"), &choices);
        assert_eq!(r.via, Some(Via::Link));
    }

    #[test]
    fn links_persist_privately_replace_by_key_and_validate() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let file = root.join("project-links.json");
        let links = Links::open(file.clone()).unwrap();
        assert!(links.list().is_empty());
        let local = root.join("app");
        std::fs::create_dir(&local).unwrap();
        let local_s = local.to_str().unwrap();
        let (changed, rows) = links
            .set(
                &json!({"localPath": format!("{local_s}/"), "peer": "mac", "peerPath": "/Users/dj/app"}),
                7,
            )
            .unwrap();
        assert!(changed);
        assert_eq!(rows[0].local_path, local_s);
        assert_eq!(rows[0].created_at, 7);
        // Same value again: no change, nothing published by the caller.
        let (changed, _) = links
            .set(
                &json!({"localPath": local_s, "peer": "mac", "peerPath": "/Users/dj/app"}),
                9,
            )
            .unwrap();
        assert!(!changed);
        links
            .set(
                &json!({"localPath": local_s, "peer": "mac", "peerPath": "C:\\dj\\app"}),
                10,
            )
            .unwrap();
        links
            .set(
                &json!({"localPath": "/gone/app", "peer": "box", "peerPath": "/srv/app"}),
                11,
            )
            .unwrap();
        assert_eq!(links.get(local_s, "mac").as_deref(), Some("C:\\dj\\app"));
        assert_eq!(links.get("/gone/app/", "box").as_deref(), Some("/srv/app"));
        assert_eq!(links.get(local_s, "box"), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let reopened = Links::open(file.clone()).unwrap();
        assert_eq!(reopened.list().len(), 2);
        assert_eq!(reopened.list()[1].peer_path, "C:\\dj\\app");
        let on_disk: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(on_disk["links"][0]["localPath"], "/gone/app");
        assert_eq!(on_disk["links"][0]["peerPath"], "/srv/app");
        assert_eq!(on_disk["links"][0]["createdAt"], 11);
        let (removed, rows) = reopened
            .remove(&json!({"localPath": "/gone/app", "peer": "box"}))
            .unwrap();
        assert!(removed);
        assert_eq!(rows.len(), 1);
        let (removed, _) = reopened
            .remove(&json!({"localPath": "/gone/app", "peer": "box"}))
            .unwrap();
        assert!(!removed);
        for bad in [
            json!({"localPath": "relative", "peer": "mac", "peerPath": "/x"}),
            json!({"localPath": "/a", "peer": "mac", "peerPath": "relative"}),
            json!({"localPath": "/a", "peer": "a b", "peerPath": "/x"}),
            json!({"localPath": "/a", "peer": "local", "peerPath": "/x"}),
            json!({"localPath": "/a", "peer": "any", "peerPath": "/x"}),
            json!({"localPath": "/a", "peer": "mac"}),
            json!({"localPath": "/a", "peer": "mac", "peerPath": "/x\n"}),
            json!({"localPath": "/a", "peer": "mac", "peerPath": 3}),
            json!({"localPath": "/a", "peer": "mac", "peerPath": "/x", "extra": 1}),
            json!(["/a"]),
        ] {
            assert!(reopened.set(&bad, 1).is_err(), "{bad}");
        }
        assert!(reopened.remove(&json!({"localPath": "/a"})).is_err());
    }

    #[test]
    fn a_damaged_links_file_is_kept_aside() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("project-links.json");
        std::fs::write(&file, b"{not json").unwrap();
        let links = Links::open(file.clone()).unwrap();
        assert!(links.list().is_empty());
        assert!(!file.exists());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn absolute_paths_are_recognized_for_any_peer_os() {
        for path in ["/x", "C:\\x", "c:/x", "\\\\server\\share"] {
            assert!(absolute_anywhere(path), "{path}");
        }
        for path in ["x", "C:x", "./x", "~/x", ""] {
            assert!(!absolute_anywhere(path), "{path}");
        }
        assert!(same_path("/", "/"));
        assert!(same_path("/a/", "/a"));
        assert!(!same_path("/a", "/b"));
    }
}

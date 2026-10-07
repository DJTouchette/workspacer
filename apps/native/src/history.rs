//! Session history, grouped by project and searchable.
//!
//! `sessions.recent` answers a flat list of every session the hub knows
//! (newest first) with its folder but no lineage. Here each row is filed
//! under the project it belongs to, the way the desktop History pane does
//! (`apps/desktop/src/renderer/src/lib/sessionHistoryGroups.ts`), with two
//! additions the native client can make from what it already holds:
//!
//! - a session started in a subfolder of a saved project counts toward that
//!   project (the most specific saved project wins), and
//! - a worktree checkout counts toward the repository it was made from: the
//!   live fleet's lineage when it knows the session, else the path itself
//!   (Claude Code's `<repo>/.claude/worktrees/…`, or the hub's
//!   `<worktreeRoot>/<repo name>/…` matched to a project or folder of that
//!   name). Worktrees of an unknown repository still share one group.
//!
//! Anything else is grouped by its own folder. Groups and their rows are
//! ordered by most recent activity.
use crate::model::Session;
use crate::projects::{self, KnownProject, Source, WorktreeRepo};

/// One history row as the screen shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub session: Session,
    /// The name shown for the row (a device rename, else the session's own).
    pub title: String,
    /// Epoch ms of the session's last activity; 0 when the hub did not say.
    pub updated_at: i64,
    /// The folder of the session's nearest non-worktree ancestor, when the
    /// live fleet knows its lineage.
    pub home: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    /// A project the hub's registry or this device saved.
    Project,
    /// A folder only sessions know about.
    Folder,
    /// Worktree checkouts of a repository no project or folder names.
    Worktrees,
    /// Sessions the hub reported without a folder.
    NoFolder,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Stable identity for collapse and "show more" state.
    pub key: String,
    pub name: String,
    /// The directory the group stands for (empty for [`GroupKind::NoFolder`]).
    pub path: String,
    pub kind: GroupKind,
    /// Rows to show, newest first.
    pub entries: Vec<Entry>,
    /// Rows in the group before any search.
    pub total: usize,
    /// Newest activity among all the group's rows.
    pub updated_at: i64,
}

/// Groups `entries` by project. `projects` is [`projects::list`]'s output;
/// `worktree_root` is the hub's configured `agents.worktreeRoot` (or empty).
pub fn group(entries: Vec<Entry>, projects: &[KnownProject], worktree_root: &str) -> Vec<Group> {
    let saved: Vec<&KnownProject> = projects.iter().filter(|p| !p.worktree).collect();
    // Folders sessions ran in directly: a home for worktrees of a repository
    // that is not a saved project.
    let mut folders: Vec<String> = Vec::new();
    for entry in &entries {
        let dir = entry.home.as_deref().unwrap_or(&entry.session.cwd);
        if !dir.trim().is_empty()
            && projects::worktree_repo(dir, worktree_root).is_none()
            && !folders.iter().any(|f| projects::same_dir(f, dir))
        {
            folders.push(projects::project_key(dir));
        }
    }
    let mut groups: Vec<Group> = Vec::new();
    for entry in entries {
        let dir = entry.home.as_deref().unwrap_or(&entry.session.cwd);
        let (path, name, kind) = place(dir, &saved, &folders, worktree_root);
        let key = format!("{kind:?}:{path}");
        let ix = match groups.iter().position(|g| g.key == key) {
            Some(ix) => ix,
            None => {
                groups.push(Group {
                    key,
                    name,
                    path,
                    kind,
                    entries: Vec::new(),
                    total: 0,
                    updated_at: i64::MIN,
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[ix];
        group.updated_at = group.updated_at.max(entry.updated_at);
        group.total += 1;
        group.entries.push(entry);
    }
    for group in &mut groups {
        // Stable: the hub's own (newest-first) order breaks ties.
        group
            .entries
            .sort_by_key(|e| std::cmp::Reverse(e.updated_at));
    }
    groups.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then((a.kind == GroupKind::NoFolder).cmp(&(b.kind == GroupKind::NoFolder)))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
    groups
}

/// The group a directory files under: its path, display name and kind.
fn place(
    dir: &str,
    saved: &[&KnownProject],
    folders: &[String],
    worktree_root: &str,
) -> (String, String, GroupKind) {
    if dir.trim().is_empty() {
        return (String::new(), "No folder".into(), GroupKind::NoFolder);
    }
    let project = |p: &KnownProject| (p.path.clone(), p.title().to_owned(), GroupKind::Project);
    // A worktree the user saved as a project is that project.
    if let Some(p) = saved.iter().find(|p| projects::same_dir(&p.path, dir)) {
        return project(p);
    }
    let dir = match projects::worktree_repo(dir, worktree_root) {
        None => projects::project_key(dir),
        Some(WorktreeRepo::Dir(repo)) => repo,
        Some(WorktreeRepo::Named { name, dir }) => {
            // Saved projects are listed pinned, then most recently opened
            // first: the likeliest of several same-named checkouts wins.
            if let Some(p) = saved.iter().find(|p| projects::basename(&p.path) == name) {
                return project(p);
            }
            if let Some(folder) = folders.iter().find(|f| projects::basename(f) == name) {
                return (folder.clone(), name, GroupKind::Folder);
            }
            return (dir, name, GroupKind::Worktrees);
        }
    };
    // The most specific saved project holding the folder. Folders only the
    // live fleet knows are matched exactly, never as parents.
    let holder = saved
        .iter()
        .filter(|p| {
            projects::same_dir(&p.path, &dir)
                || (p.source != Source::Sessions && contains(&p.path, &dir))
        })
        .max_by_key(|p| projects::project_key(&p.path).len());
    if let Some(p) = holder {
        return project(p);
    }
    let name = projects::basename(&dir).to_owned();
    (dir, name, GroupKind::Folder)
}

/// Whether `dir` is strictly inside `parent`.
fn contains(parent: &str, dir: &str) -> bool {
    let parent = projects::project_key(parent);
    let dir = projects::project_key(dir);
    let prefix = if parent.ends_with('/') {
        parent.clone()
    } else {
        format!("{parent}/")
    };
    dir.len() > prefix.len()
        && dir.is_char_boundary(prefix.len())
        && projects::same_dir(&dir[..prefix.len()], &prefix)
}

/// Whether a row answers `query`: every whitespace-separated term must
/// appear, case-insensitively, in its title, project name, folder, project
/// path, provider, model or id.
pub fn matches(entry: &Entry, group: &Group, query: &str) -> bool {
    let s = &entry.session;
    let haystack = [
        entry.title.as_str(),
        group.name.as_str(),
        group.path.as_str(),
        s.cwd.as_str(),
        s.provider.as_str(),
        s.model.as_str(),
        s.runtime_model.as_str(),
        &s.display_model(),
        s.id.as_str(),
    ]
    .join("\n")
    .to_lowercase();
    query
        .split_whitespace()
        .all(|term| haystack.contains(&term.to_lowercase()))
}

/// Keeps the rows that answer `query` and drops groups left empty. Each
/// group's `total` still counts all of its rows, for "3 of 12".
pub fn search(groups: Vec<Group>, query: &str) -> Vec<Group> {
    if query.trim().is_empty() {
        return groups;
    }
    groups
        .into_iter()
        .filter_map(|mut group| {
            let entries = std::mem::take(&mut group.entries);
            group.entries = entries
                .into_iter()
                .filter(|e| matches(e, &group, query))
                .collect();
            (!group.entries.is_empty()).then_some(group)
        })
        .collect()
}

/// "just now", "5m ago", "3h ago", "2d ago" for an epoch-ms time; empty when
/// the time is unknown.
pub fn age(at_ms: i64, now_ms: i64) -> String {
    if at_ms <= 0 {
        return String::new();
    }
    match (now_ms - at_ms).max(0) / 1000 {
        s if s < 60 => "just now".into(),
        s if s < 3_600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3_600),
        s => format!("{}d ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, cwd: &str, updated_at: i64) -> Entry {
        Entry {
            session: Session {
                id: id.into(),
                cwd: cwd.into(),
                provider: "claude".into(),
                ..Default::default()
            },
            title: format!("Session {id}"),
            updated_at,
            home: None,
        }
    }

    fn registry(paths: &[(&str, Option<&str>)]) -> serde_json::Value {
        let projects: serde_json::Map<_, _> = paths
            .iter()
            .map(|(path, label)| {
                let mut entry = json!({"lastOpened": 1});
                if let Some(label) = label {
                    entry["label"] = json!(label);
                }
                (path.to_string(), entry)
            })
            .collect();
        json!({"projects": projects, "favourites": [], "recent": [], "configured": []})
    }

    fn ids(group: &Group) -> Vec<&str> {
        group
            .entries
            .iter()
            .map(|e| e.session.id.as_str())
            .collect()
    }

    #[test]
    fn rows_file_under_their_saved_project_including_subfolders() {
        let registry = registry(&[("/work/app", Some("App")), ("/work/app/web", None)]);
        let known = projects::list(Some(&registry), &[], &[]);
        let groups = group(
            vec![
                entry("a", "/work/app", 10),
                entry("b", "/work/app/crates/core", 30),
                entry("c", "/work/app/web/src", 20),
                entry("d", "/work/app/web", 5),
            ],
            &known,
            "",
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "App");
        assert_eq!(groups[0].path, "/work/app");
        assert_eq!(groups[0].kind, GroupKind::Project);
        assert_eq!(ids(&groups[0]), ["b", "a"]);
        // The most specific saved project wins.
        assert_eq!(groups[1].name, "web");
        assert_eq!(ids(&groups[1]), ["c", "d"]);
    }

    #[test]
    fn hub_worktrees_join_the_project_their_repository_is_named_after() {
        let registry = registry(&[("/home/u/Work/workspacer", None)]);
        let mut known = projects::list(Some(&registry), &[], &[]);
        let root = "/home/u/.workspacer/worktrees";
        let groups = group(
            vec![
                entry("a", "/home/u/Work/workspacer", 1),
                entry(
                    "w1",
                    "/home/u/.workspacer/worktrees/workspacer/fix-sidebar",
                    5,
                ),
                entry("w2", "/srv/trees/workspacer/other", 3),
            ],
            &known,
            root,
        );
        assert_eq!(groups.len(), 2, "{groups:#?}");
        assert_eq!(groups[0].path, "/home/u/Work/workspacer");
        assert_eq!(ids(&groups[0]), ["w1", "a"]);
        // A configured root other than the default is recognised too.
        let groups = group(
            vec![entry("w2", "/srv/trees/workspacer/other", 3)],
            &known,
            "/srv/trees",
        );
        assert_eq!(groups[0].path, "/home/u/Work/workspacer");
        assert_eq!(groups[0].kind, GroupKind::Project);

        // No saved project: a folder sessions ran in by that name is home.
        known.clear();
        let groups = group(
            vec![
                entry("a", "/elsewhere/workspacer", 1),
                entry("w1", "/home/u/.workspacer/worktrees/workspacer/fix", 5),
            ],
            &known,
            root,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].path, "/elsewhere/workspacer");
        assert_eq!(groups[0].kind, GroupKind::Folder);
        assert_eq!(groups[0].total, 2);

        // Nothing names the repository: its worktrees still share a group.
        let groups = group(
            vec![
                entry("w1", "/home/u/.workspacer/worktrees/tool/a", 5),
                entry("w2", "/home/u/.workspacer/worktrees/tool/b", 4),
            ],
            &known,
            root,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "tool");
        assert_eq!(groups[0].path, "/home/u/.workspacer/worktrees/tool");
        assert_eq!(groups[0].kind, GroupKind::Worktrees);
    }

    #[test]
    fn claude_code_worktrees_and_live_lineage_find_their_repository() {
        let registry = registry(&[("/work/app", Some("App"))]);
        let known = projects::list(Some(&registry), &[], &[]);
        let mut managed = entry("child", "/trees/app-x/worker", 9);
        managed.home = Some("/work/app".into());
        let groups = group(
            vec![
                entry("cc", "/work/app/.claude/worktrees/feature", 4),
                managed,
                entry("p", "/work/app", 1),
            ],
            &known,
            "/trees",
        );
        assert_eq!(groups.len(), 1, "{groups:#?}");
        assert_eq!(ids(&groups[0]), ["child", "cc", "p"]);
    }

    #[test]
    fn unknown_folders_group_by_themselves_and_saved_worktrees_stay_projects() {
        let registry = registry(&[(
            "/home/u/.workspacer/worktrees/app/pinned",
            Some("Pinned tree"),
        )]);
        let known = projects::list(Some(&registry), &[], &[]);
        let groups = group(
            vec![
                entry("a", "/tmp/scratch", 3),
                entry("b", "/tmp/scratch/", 2),
                entry("c", "/tmp/scratch/deeper", 1),
                entry("t", "/home/u/.workspacer/worktrees/app/pinned", 4),
                entry("n", "", 7),
            ],
            &known,
            "",
        );
        let summary: Vec<_> = groups
            .iter()
            .map(|g| (g.name.as_str(), g.kind, g.total))
            .collect();
        assert_eq!(
            summary,
            [
                ("No folder", GroupKind::NoFolder, 1),
                ("Pinned tree", GroupKind::Project, 1),
                ("scratch", GroupKind::Folder, 2),
                // Unknown folders never absorb each other's subfolders.
                ("deeper", GroupKind::Folder, 1),
            ]
        );
    }

    #[test]
    fn groups_and_rows_are_ordered_by_latest_activity() {
        let groups = group(
            vec![
                entry("old", "/b", 1),
                entry("a1", "/a", 5),
                entry("new", "/b", 9),
                entry("a2", "/a", 7),
                entry("tie1", "/c", 0),
                entry("tie2", "/c", 0),
            ],
            &[],
            "",
        );
        let order: Vec<_> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
        assert_eq!(ids(&groups[0]), ["new", "old"]);
        assert_eq!(ids(&groups[1]), ["a2", "a1"]);
        // Unknown times keep the hub's order.
        assert_eq!(ids(&groups[2]), ["tie1", "tie2"]);
        assert_eq!(groups[0].updated_at, 9);
    }

    #[test]
    fn search_matches_title_project_folder_provider_and_model() {
        let registry = registry(&[("/work/app", Some("Storefront"))]);
        let known = projects::list(Some(&registry), &[], &[]);
        let mut codex = entry("x", "/work/app/api", 2);
        codex.session.provider = "codex".into();
        codex.session.model = "gpt-5.5-codex".into();
        codex.title = "Fix the checkout".into();
        let mut other = entry("y", "/srv/notes", 1);
        other.title = "Write release notes".into();
        let groups = group(vec![codex, other], &known, "");
        let hits = |q: &str| -> Vec<String> {
            search(groups.clone(), q)
                .iter()
                .flat_map(|g| g.entries.iter().map(|e| e.session.id.clone()))
                .collect()
        };
        assert_eq!(hits("CHECKOUT"), ["x"]);
        assert_eq!(hits("storefront"), ["x"], "project name");
        assert_eq!(hits("app/api"), ["x"], "folder");
        assert_eq!(hits("codex"), ["x"], "provider");
        assert_eq!(hits("gpt-5.5"), ["x"], "model");
        assert_eq!(hits("release notes"), ["y"], "terms in any order");
        assert_eq!(hits("notes storefront"), Vec::<String>::new());
        assert_eq!(hits("  "), ["x", "y"]);
        let found = search(groups, "checkout");
        assert_eq!(found.len(), 1, "groups without a match hide");
        assert_eq!((found[0].entries.len(), found[0].total), (1, 1));
    }

    #[test]
    fn ages_read_at_a_glance() {
        let now = 10 * 86_400_000;
        assert_eq!(age(0, now), "");
        assert_eq!(age(now - 5_000, now), "just now");
        assert_eq!(age(now - 5 * 60_000, now), "5m ago");
        assert_eq!(age(now - 3 * 3_600_000, now), "3h ago");
        assert_eq!(age(now - 2 * 86_400_000, now), "2d ago");
    }

    #[test]
    fn worktree_paths_name_their_repository() {
        use WorktreeRepo::*;
        assert_eq!(
            projects::worktree_repo("/r/app/.claude/worktrees/x", ""),
            Some(Dir("/r/app".into()))
        );
        assert_eq!(
            projects::worktree_repo("/h/.workspacer/worktrees/app/x/sub", ""),
            Some(Named {
                name: "app".into(),
                dir: "/h/.workspacer/worktrees/app".into()
            })
        );
        assert_eq!(
            projects::worktree_repo("C:\\trees\\app\\x", "c:/trees"),
            Some(Named {
                name: "app".into(),
                dir: "C:/trees/app".into()
            })
        );
        assert_eq!(projects::worktree_repo("/trees", "/trees"), None);
        assert_eq!(projects::worktree_repo("/work/app", "/trees"), None);
        assert_eq!(projects::worktree_repo("/.claude/worktrees/", ""), None);
    }
}

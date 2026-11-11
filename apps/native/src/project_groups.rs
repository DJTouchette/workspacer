//! Projects as named groups of folders (`docs/features/projects.md`).
//!
//! The hub owns them in `<configDir>/project-groups.json` and serves them
//! through `projectGroups.list/save/remove`; the per-folder registry
//! (`crate::projects`, `config.yaml` `projects:`) is untouched and still
//! carries each folder's own identity and settings. A folder belongs to at
//! most one group, and a folder in no group is still shown as a one-folder
//! project, so nothing a user had before changes until they make a group.
//!
//! Membership is by folder: a cwd is in the group whose folder contains it
//! (the longest folder wins), and a worktree checkout is in the group of the
//! repository it was made from.
use anyhow::{Result, ensure};
use serde_json::{Value, json};

use crate::model::Session;
use crate::projects::{
    self, KnownProject, MAX_ICON_CHARS, MAX_LABEL_CHARS, Source, WorktreeRepo, project_key,
    same_dir,
};

/// Folders one project may hold (the hub enforces the same bound).
pub const MAX_FOLDERS: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// Normalised (`project_key`) absolute folders, in the user's order.
    pub folders: Vec<String>,
    /// `#rrggbb`.
    pub color: Option<u32>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Groups {
    pub revision: u64,
    pub groups: Vec<Group>,
}

fn parse_color(value: &Value) -> Option<u32> {
    let hex = value.as_str()?.trim().strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok())?
}

/// A `projectGroups.list` reply (or a save/remove, which answers with one).
pub fn parse(value: &Value) -> Option<Groups> {
    let rows = value.get("groups")?.as_array()?;
    let groups = rows
        .iter()
        .filter_map(|row| {
            let id = row["id"].as_str().filter(|s| !s.is_empty())?.to_owned();
            let name = row["name"].as_str().map(str::trim).unwrap_or("").to_owned();
            let folders: Vec<String> = row["folders"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(project_key)
                .filter(|f| !f.is_empty())
                .collect();
            Some(Group {
                name: if name.is_empty() { id.clone() } else { name },
                id,
                folders,
                color: parse_color(&row["color"]),
                icon: row["icon"]
                    .as_str()
                    .map(str::trim)
                    .filter(|i| !i.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect();
    Some(Groups {
        revision: value["revision"].as_u64().unwrap_or(0),
        groups,
    })
}

/// Whether `path` is `folder` or inside it.
pub fn within(path: &str, folder: &str) -> bool {
    let (path, folder) = (project_key(path), project_key(folder));
    if folder.is_empty() || path.is_empty() {
        return false;
    }
    if same_dir(&path, &folder) {
        return true;
    }
    let prefix = if folder.ends_with('/') {
        folder.len()
    } else {
        folder.len() + 1
    };
    path.len() > prefix
        && path.is_char_boundary(prefix)
        && path[..prefix].ends_with('/')
        && same_dir(&path[..prefix], &folder)
}

impl Group {
    pub fn title(&self) -> &str {
        &self.name
    }

    /// The identity `project_mark` draws: the group's name, colour and icon,
    /// with a stable hue seed (`group:<id>`) when no colour is set.
    pub fn as_known(&self) -> KnownProject {
        KnownProject {
            path: format!("group:{}", self.id),
            label: Some(self.name.clone()),
            color: self.color,
            icon: self.icon.clone(),
            favicon: None,
            icon_file: None,
            favourite: false,
            last_opened: None,
            sessions: 0,
            live_sessions: 0,
            source: Source::Hub,
            worktree: false,
            configured: true,
            git: Default::default(),
        }
    }

    /// The length of this group's longest folder containing `path`.
    fn depth(&self, path: &str) -> Option<usize> {
        self.folders
            .iter()
            .filter(|f| within(path, f))
            .map(|f| f.len())
            .max()
    }

    pub fn holds(&self, path: &str) -> bool {
        self.depth(path).is_some()
    }

    pub fn matches(&self, query: &str) -> bool {
        let haystack = format!("{} {}", self.name, self.folders.join(" ")).to_lowercase();
        query
            .split_whitespace()
            .all(|term| haystack.contains(&term.to_lowercase()))
    }
}

impl Groups {
    pub fn get(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The group a path is in: the longest containing folder wins. A
    /// worktree checkout resolves to its repository first.
    pub fn of_path(&self, path: &str, worktree_root: &str) -> Option<&Group> {
        let direct = self
            .groups
            .iter()
            .filter_map(|g| g.depth(path).map(|d| (d, g)))
            .max_by_key(|(d, _)| *d)
            .map(|(_, g)| g);
        if direct.is_some() {
            return direct;
        }
        match projects::worktree_repo(path, worktree_root)? {
            WorktreeRepo::Dir(repo) => self.of_path(&repo, ""),
            // Only the repository's folder NAME survives in the hub's own
            // worktree layout: match a group folder by that name.
            WorktreeRepo::Named { name, .. } => self
                .groups
                .iter()
                .find(|g| g.folders.iter().any(|f| projects::basename(f) == name)),
        }
    }

    /// The group a session is in: its own folder, else (a worker in an
    /// isolated checkout) the nearest ancestor session's.
    pub fn of_session(
        &self,
        sessions: &[Session],
        at: usize,
        worktree_root: &str,
    ) -> Option<&Group> {
        let session = sessions.get(at)?;
        if !session.cwd.is_empty()
            && let Some(group) = self.of_path(&session.cwd, worktree_root)
        {
            return Some(group);
        }
        crate::navigation::ancestors(sessions, at)
            .into_iter()
            .map(|a| sessions[a].cwd.as_str())
            .filter(|cwd| !cwd.is_empty())
            .find_map(|cwd| self.of_path(cwd, worktree_root))
    }

    /// Whether `path` is one of some group's folders (exactly).
    pub fn grouped(&self, path: &str) -> bool {
        self.groups
            .iter()
            .any(|g| g.folders.iter().any(|f| same_dir(f, path)))
    }
}

/// What the project editor holds; `params` is the `projectGroups.save` call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    /// `None` creates a project.
    pub id: Option<String>,
    pub name: String,
    pub folders: Vec<String>,
    /// `#rrggbb` or empty.
    pub color: String,
    pub icon: String,
}

impl Draft {
    pub fn of(group: &Group) -> Self {
        Self {
            id: Some(group.id.clone()),
            name: group.name.clone(),
            folders: group.folders.clone(),
            color: group.color.map(|c| format!("#{c:06x}")).unwrap_or_default(),
            icon: group.icon.clone().unwrap_or_default(),
        }
    }

    /// Adds a folder unless the draft already holds it; `false` when not added.
    pub fn add_folder(&mut self, path: &str) -> bool {
        let key = project_key(path);
        if key.is_empty() || self.folders.iter().any(|f| same_dir(f, &key)) {
            return false;
        }
        self.folders.push(key);
        true
    }

    pub fn remove_folder(&mut self, path: &str) {
        self.folders.retain(|f| !same_dir(f, path));
    }

    /// The checks the hub also makes, so the editor can say what is wrong
    /// before sending.
    pub fn params(&self, revision: u64) -> Result<Value> {
        let name = self.name.trim();
        ensure!(!name.is_empty(), "Give the project a name.");
        ensure!(
            name.chars().count() <= MAX_LABEL_CHARS,
            "A project name is at most {MAX_LABEL_CHARS} characters."
        );
        ensure!(
            !self.folders.is_empty(),
            "Add at least one folder to the project."
        );
        ensure!(
            self.folders.len() <= MAX_FOLDERS,
            "A project holds at most {MAX_FOLDERS} folders."
        );
        for folder in &self.folders {
            ensure!(
                crate::launch::absolute_directory(folder),
                "{folder} is not an absolute folder path."
            );
        }
        let icon = self.icon.trim();
        ensure!(
            icon.chars().count() <= MAX_ICON_CHARS,
            "An icon is an emoji or up to {MAX_ICON_CHARS} letters."
        );
        let color = self.color.trim();
        ensure!(
            color.is_empty() || parse_color(&json!(color)).is_some(),
            "A colour is written #rrggbb."
        );
        let mut params = json!({"name": name, "folders": self.folders, "revision": revision});
        if let Some(id) = &self.id {
            params["id"] = json!(id);
        }
        if !color.is_empty() {
            params["color"] = json!(color.to_ascii_lowercase());
        }
        if !icon.is_empty() {
            params["icon"] = json!(icon);
        }
        Ok(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed() -> Value {
        json!({"revision": 4, "groups": [
            {"id":"p_acme","name":"Acme","folders":["/w/acme-api/","/w/acme-web"],"color":"#5B8DEF","icon":"🚀"},
            {"id":"p_mono","name":"Mono","folders":["/w/mono","/w/mono/tools/cli"]},
            {"id":"","name":"broken","folders":["/x"]}
        ]})
    }

    fn session(id: &str, cwd: &str, parent: Option<&str>) -> Session {
        Session {
            id: id.into(),
            cwd: cwd.into(),
            parent_session_id: parent.unwrap_or("").into(),
            ..Default::default()
        }
    }

    #[test]
    fn parses_groups_and_skips_rows_without_an_id() {
        let groups = parse(&listed()).unwrap();
        assert_eq!(groups.revision, 4);
        assert_eq!(groups.groups.len(), 2);
        let acme = &groups.groups[0];
        assert_eq!(acme.folders, ["/w/acme-api", "/w/acme-web"]);
        assert_eq!(acme.color, Some(0x5b8def));
        assert_eq!(acme.as_known().mark(), "🚀");
        assert!(parse(&json!({"jobs":[]})).is_none());
    }

    #[test]
    fn a_path_is_in_the_group_of_its_longest_containing_folder() {
        let groups = parse(&listed()).unwrap();
        let of = |p: &str| groups.of_path(p, "").map(|g| g.id.as_str());
        assert_eq!(of("/w/acme-api"), Some("p_acme"));
        assert_eq!(of("/w/acme-web/src/app"), Some("p_acme"));
        assert_eq!(
            of("/w/acme-webby"),
            None,
            "a sibling with a shared prefix is not inside"
        );
        assert_eq!(of("/w/mono/tools/cli/src"), Some("p_mono"));
        assert_eq!(of("/elsewhere"), None);
        assert!(within("/w/a", "/w/a/") && within("/w/a/b", "/w/a") && !within("/w/ab", "/w/a"));
        assert!(within("/anything", "/"));
    }

    #[test]
    fn worktrees_and_workers_resolve_to_their_repositorys_group() {
        let groups = parse(&listed()).unwrap();
        let of = |p: &str, root: &str| groups.of_path(p, root).map(|g| g.id.as_str());
        assert_eq!(of("/w/acme-api/.claude/worktrees/fix", ""), Some("p_acme"));
        assert_eq!(
            of(
                "/home/me/.workspacer/worktrees/acme-web/fix-cart",
                "/home/me/.workspacer/worktrees"
            ),
            Some("p_acme")
        );
        let sessions = vec![
            session("m", "/w/acme-api", None),
            session("w", "/tmp/elsewhere", Some("m")),
            session("x", "/tmp/elsewhere", None),
        ];
        assert_eq!(
            groups.of_session(&sessions, 1, "").map(|g| g.id.as_str()),
            Some("p_acme")
        );
        assert_eq!(groups.of_session(&sessions, 2, ""), None);
        assert!(groups.grouped("/w/acme-web/") && !groups.grouped("/w/acme-web/src"));
    }

    #[test]
    fn drafts_check_what_the_hub_checks_and_send_only_what_is_set() {
        let mut draft = Draft::default();
        assert!(draft.params(0).unwrap_err().to_string().contains("name"));
        draft.name = "  Acme  ".into();
        assert!(draft.params(0).unwrap_err().to_string().contains("folder"));
        assert!(draft.add_folder("/w/api/"));
        assert!(
            !draft.add_folder("/w/api"),
            "the same folder is not added twice"
        );
        assert!(draft.add_folder("/w/web"));
        draft.remove_folder("/w/web/");
        assert_eq!(
            draft.params(3).unwrap(),
            json!({"name":"Acme","folders":["/w/api"],"revision":3})
        );
        draft.color = "#ABCDEF".into();
        draft.icon = "🚀".into();
        draft.id = Some("p_1".into());
        assert_eq!(
            draft.params(3).unwrap(),
            json!({"id":"p_1","name":"Acme","folders":["/w/api"],"revision":3,"color":"#abcdef","icon":"🚀"})
        );
        draft.color = "blue".into();
        assert!(draft.params(3).is_err());
        draft.color.clear();
        draft.folders = vec!["relative/path".into()];
        assert!(draft.params(3).is_err());
        let group = parse(&listed()).unwrap().groups[0].clone();
        assert_eq!(Draft::of(&group).color, "#5b8def");
    }
}

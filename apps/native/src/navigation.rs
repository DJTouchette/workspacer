//! Client-local settings and exact-path project grouping. No hub mutations.
use crate::{appearance::preference_path, model::Session};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
}
impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Device-local typography; empty families select the platform default.
    pub interface_font: String,
    pub code_font: String,
    pub text_size: u8,
    pub twelve_hour_clock: bool,
    /// One work card per turn: assistant notes between tool calls join it.
    pub merge_turn_tools: bool,
    /// Interface size in percent, on top of the OS display scale.
    pub interface_scale: u16,
    pub sidebar_width: f32,
    pub vim_navigation: bool,
    /// Composer submit key: Enter sends and Shift+Enter adds a line, instead
    /// of the default Ctrl/Cmd+Enter to send and Enter for a new line.
    pub enter_sends: bool,
    pub keep_running: bool,
    pub notifications: bool,
    /// Review shows its right-hand panel (changed files, tree, commits).
    pub review_panel: bool,
    /// Chrome changes shape at once instead of on springs (the title
    /// island's reveal and notices); fades stay.
    pub reduce_motion: bool,
    /// Client-local session organization, scoped by hub identity.
    pub names: BTreeMap<String, BTreeMap<String, String>>,
    pub archived: BTreeMap<String, Vec<String>>,
    /// Finished children cleared from the sidebar, by hub identity. Separate
    /// from the shared archive: this device's child view only.
    pub cleared_children: BTreeMap<String, BTreeMap<String, crate::child_agents::ClearMark>>,
    /// Sessions that were open, by hub identity, with when this device first
    /// saw each open. One still listed here after the app restarts was open
    /// when it closed, so it stays in the sidebar to pick back up until it is
    /// resumed, archived, or ends while the app is running.
    pub kept_open: BTreeMap<String, BTreeMap<String, i64>>,
    pub default_provider: Provider,
    pub default_claude_access: crate::launch::Permission,
    pub default_codex_access: crate::launch::Permission,
    /// Bookmarks are scoped to the hub endpoint so remote paths do not cross hosts.
    pub projects: BTreeMap<String, Vec<String>>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            interface_font: "Inter".into(),
            code_font: "JetBrains Mono".into(),
            text_size: 15,
            twelve_hour_clock: false,
            merge_turn_tools: false,
            interface_scale: 100,
            sidebar_width: 304.,
            vim_navigation: true,
            enter_sends: false,
            keep_running: false,
            notifications: true,
            review_panel: true,
            reduce_motion: false,
            names: BTreeMap::new(),
            archived: BTreeMap::new(),
            cleared_children: BTreeMap::new(),
            kept_open: BTreeMap::new(),
            default_provider: Provider::Claude,
            default_claude_access: crate::launch::Permission::Ask,
            default_codex_access: crate::launch::Permission::Ask,
            projects: BTreeMap::new(),
        }
    }
}
impl Settings {
    pub fn default_access(&self, provider: &str) -> crate::launch::Permission {
        use crate::launch::Permission;
        let access = if provider == "claude" {
            self.default_claude_access
        } else {
            self.default_codex_access
        };
        if Permission::choices(provider).contains(&access) {
            access
        } else {
            Permission::Ask
        }
    }
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut settings: Self = serde_json::from_slice(&bytes)?;
                settings.text_size = settings.text_size.clamp(12, 20);
                settings.sidebar_width = sidebar_width(settings.sidebar_width, 2000.);
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
    pub fn bookmarks(&self, hub: &str) -> &[String] {
        self.projects
            .get(hub)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
    pub fn add_project(&mut self, hub: &str, path: &str) -> anyhow::Result<()> {
        let path = path.trim();
        crate::controller::NewSession {
            provider: "claude".into(),
            cwd: path.into(),
            label: String::new(),
            model: String::new(),
            message: String::new(),
            ..Default::default()
        }
        .params()?;
        let paths = self.projects.entry(hub.to_owned()).or_default();
        if !paths.iter().any(|p| p == path) {
            paths.push(path.to_owned());
            paths.sort();
        }
        Ok(())
    }
}
pub fn settings_path() -> Option<PathBuf> {
    Some(preference_path()?.with_file_name("native-settings.json"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub path: String,
    pub sessions: usize,
    pub saved: bool,
}
impl Project {
    pub fn title(&self) -> &str {
        self.path
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or(&self.path)
    }
}
pub fn projects(sessions: &[Session], saved: &[String]) -> Vec<Project> {
    let mut groups = BTreeMap::<String, Project>::new();
    for path in saved.iter().filter(|p| !p.is_empty()) {
        groups.entry(path.clone()).or_insert(Project {
            path: path.clone(),
            sessions: 0,
            saved: true,
        });
    }
    for session in sessions.iter().filter(|s| !s.cwd.is_empty()) {
        groups
            .entry(session.cwd.clone())
            .or_insert(Project {
                path: session.cwd.clone(),
                sessions: 0,
                saved: false,
            })
            .sessions += 1;
    }
    groups.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interface_scale_steps_through_offered_sizes() {
        assert_eq!(step_interface_scale(100, 1), 110);
        assert_eq!(step_interface_scale(100, -1), 90);
        assert_eq!(step_interface_scale(200, 1), 200);
        assert_eq!(step_interface_scale(70, -1), 70);
        assert_eq!(step_interface_scale(105, 1), 110);
        assert_eq!(step_interface_scale(105, -1), 100);
    }

    #[test]
    fn clock_preference_is_backwards_compatible_and_round_trips() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(!old.twelve_hour_clock);
        assert!(!old.merge_turn_tools);
        assert_eq!(old.interface_scale, 100);
        let selected = Settings {
            twelve_hour_clock: true,
            merge_turn_tools: true,
            ..old
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&selected).unwrap()).unwrap();
        assert!(restored.twelve_hour_clock);
        assert!(restored.merge_turn_tools);
    }

    #[test]
    fn projects_keep_same_names_distinct_and_include_empty_bookmarks() {
        let sessions = vec![
            Session {
                cwd: "/a/app".into(),
                ..Default::default()
            },
            Session {
                cwd: "/b/app".into(),
                ..Default::default()
            },
            Session {
                cwd: "/a/app".into(),
                ..Default::default()
            },
        ];
        let rows = projects(&sessions, &["/empty".into(), "/a/app".into()]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].sessions, 2);
        assert!(rows[0].saved);
        assert_eq!(rows[2].sessions, 0);
        assert_eq!(rows[0].title(), rows[1].title());
    }
    #[test]
    fn bookmarks_validate_deduplicate_and_stay_with_their_hub() {
        let mut settings = Settings::default();
        assert!(settings.add_project("hub-a", "relative").is_err());
        assert!(settings.add_project("hub-a", "/bad\0path").is_err());
        settings.add_project("hub-a", " /work/app ").unwrap();
        settings.add_project("hub-a", "/work/app").unwrap();
        settings.add_project("hub-b", "C:\\work\\app").unwrap();
        assert_eq!(settings.bookmarks("hub-a"), ["/work/app"]);
        assert_eq!(settings.bookmarks("hub-b"), ["C:\\work\\app"]);
    }
    #[test]
    fn settings_defaults_and_persistence() {
        let path = std::env::temp_dir().join(format!(
            "native-settings-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut settings = Settings::load(&path).unwrap();
        assert!(settings.vim_navigation);
        settings.vim_navigation = false;
        settings.default_provider = Provider::Codex;
        settings.default_claude_access = crate::launch::Permission::AcceptEdits;
        settings.default_codex_access = crate::launch::Permission::FullAccess;
        settings.interface_font = "Adwaita Sans".into();
        settings.code_font = "Adwaita Mono".into();
        settings.text_size = 17;
        settings.add_project("hub", "/app").unwrap();
        settings.save(&path).unwrap();
        let read = Settings::load(&path).unwrap();
        assert!(!read.vim_navigation);
        assert_eq!(read.default_provider, Provider::Codex);
        assert_eq!(
            read.default_access("claude"),
            crate::launch::Permission::AcceptEdits
        );
        assert_eq!(
            read.default_access("codex"),
            crate::launch::Permission::FullAccess
        );
        assert_eq!(read.interface_font, "Adwaita Sans");
        assert_eq!(read.code_font, "Adwaita Mono");
        assert_eq!(read.text_size, 17);
        assert_eq!(read.bookmarks("hub"), ["/app"]);
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().default_access("claude"),
            crate::launch::Permission::Ask
        );
        assert_eq!(
            Settings::load(&path).unwrap().default_access("codex"),
            crate::launch::Permission::Ask
        );
        std::fs::write(&path, br#"{"default_codex_access":"plan"}"#).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().default_access("codex"),
            crate::launch::Permission::Ask
        );
        std::fs::write(&path, b"{}").unwrap();
        assert!(Settings::load(&path).unwrap().vim_navigation);
        assert_eq!(Settings::load(&path).unwrap().interface_font, "Inter");
        std::fs::write(
            &path,
            br#"{"text_size":255,"interface_font":"","code_font":""}"#,
        )
        .unwrap();
        let read = Settings::load(&path).unwrap();
        assert_eq!(read.text_size, 20);
        assert!(read.interface_font.is_empty());
        assert!(read.code_font.is_empty());
        std::fs::write(&path, b"{").unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}

// Keep enough room for the conversation at small window sizes.
/// Interface sizes offered in Settings and stepped by Ctrl/Cmd + / −.
pub const INTERFACE_SCALES: [u16; 9] = [70, 80, 90, 100, 110, 125, 150, 175, 200];

/// The next offered size above (`step > 0`) or below `current`.
pub fn step_interface_scale(current: u16, step: i32) -> u16 {
    let (first, last) = (
        INTERFACE_SCALES[0],
        INTERFACE_SCALES[INTERFACE_SCALES.len() - 1],
    );
    if step > 0 {
        INTERFACE_SCALES
            .into_iter()
            .find(|s| *s > current)
            .unwrap_or(last)
    } else {
        INTERFACE_SCALES
            .into_iter()
            .rev()
            .find(|s| *s < current)
            .unwrap_or(first)
    }
}

pub fn sidebar_width(preferred: f32, viewport: f32) -> f32 {
    let preferred = if preferred.is_finite() {
        preferred
    } else {
        304.
    };
    preferred.clamp(200., (viewport * 0.4).clamp(200., 520.))
}

/// The ancestors of `ix`, nearest first, following `parent_session_id`
/// through `sessions`. Stops at a missing parent or a cycle.
pub fn ancestors(sessions: &[Session], ix: usize) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut current = ix;
    while let Some(parent) = sessions.iter().position(|s| {
        !sessions[current].parent_session_id.is_empty()
            && s.id == sessions[current].parent_session_id
    }) {
        if parent == ix || chain.contains(&parent) {
            break;
        }
        chain.push(parent);
        current = parent;
    }
    chain
}

/// Filters apply to whole lineages, not single rows: spawned workers run in
/// their own worktree folders, so a project or search that matches the parent
/// would otherwise drop its children (and a match on a child would show it
/// with no parent). A row is kept when `eligible` allows it and it, or one of
/// its ancestors, `matches`; the eligible ancestors of every kept row are kept
/// too, as context, so nesting survives filtering.
pub fn lineage_filter(
    sessions: &[Session],
    eligible: impl Fn(usize) -> bool,
    matches: impl Fn(usize) -> bool,
) -> Vec<usize> {
    let chains: Vec<_> = (0..sessions.len())
        .map(|ix| ancestors(sessions, ix))
        .collect();
    let mut keep = vec![false; sessions.len()];
    for ix in (0..sessions.len()).filter(|&ix| eligible(ix)) {
        if matches(ix) || chains[ix].iter().any(|&a| eligible(a) && matches(a)) {
            keep[ix] = true;
            for &a in chains[ix].iter().filter(|&&a| eligible(a)) {
                keep[a] = true;
            }
        }
    }
    (0..sessions.len()).filter(|&ix| keep[ix]).collect()
}

/// Which sessions a clear hides. A cleared session that still has a shown,
/// uncleared descendant stays as that descendant's context, so clearing never
/// detaches live work from its lineage.
pub fn cleared_hidden(
    sessions: &[Session],
    shown: impl Fn(usize) -> bool,
    cleared: impl Fn(usize) -> bool,
) -> Vec<bool> {
    let cleared: Vec<bool> = (0..sessions.len()).map(cleared).collect();
    let mut hidden = cleared.clone();
    for ix in (0..sessions.len()).filter(|&ix| shown(ix) && !cleared[ix]) {
        for ancestor in ancestors(sessions, ix) {
            hidden[ancestor] = false;
        }
    }
    hidden
}

/// Stable parent-first order; missing/filtered parents become roots. A visited
/// set also keeps malformed cycles visible without recursing indefinitely.
pub fn session_tree(sessions: &[Session], visible: &[usize]) -> Vec<(usize, usize)> {
    let ids: BTreeMap<_, _> = visible
        .iter()
        .map(|&ix| (sessions[ix].id.as_str(), ix))
        .collect();
    let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut roots = Vec::new();
    for &ix in visible {
        match ids.get(sessions[ix].parent_session_id.as_str()).copied() {
            Some(parent) if parent != ix => children.entry(parent).or_default().push(ix),
            _ => roots.push(ix),
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for root in roots.into_iter().chain(visible.iter().copied()) {
        let mut stack = vec![(root, 0)];
        while let Some((ix, depth)) = stack.pop() {
            if !seen.insert(ix) {
                continue;
            }
            result.push((ix, depth));
            if let Some(children) = children.get(&ix) {
                stack.extend(children.iter().rev().map(|&child| (child, depth + 1)));
            }
        }
    }
    result
}

#[cfg(test)]
mod sidebar_tests {
    use super::*;
    fn session(id: &str, parent: &str) -> Session {
        Session {
            id: id.into(),
            parent_session_id: parent.into(),
            ..Default::default()
        }
    }
    #[test]
    fn children_and_grandchildren_follow_parent_even_when_newest_first() {
        let sessions = vec![
            session("grandchild", "child"),
            session("child", "root"),
            session("other", ""),
            session("root", ""),
        ];
        assert_eq!(
            session_tree(&sessions, &[0, 1, 2, 3]),
            vec![(2, 0), (3, 0), (1, 1), (0, 2)]
        );
        assert_eq!(
            session_tree(&sessions, &[0, 1, 2]),
            vec![(1, 0), (0, 1), (2, 0)]
        );
    }
    #[test]
    fn a_cleared_child_stays_as_context_for_uncleared_descendants() {
        let sessions = vec![
            session("root", ""),
            session("child", "root"),
            session("grandchild", "child"),
            session("sibling", "root"),
        ];
        let all = |_| true;
        assert_eq!(
            cleared_hidden(&sessions, all, |ix| ix == 1 || ix == 3),
            vec![false, false, false, true],
            "the live grandchild keeps its cleared parent"
        );
        assert_eq!(
            cleared_hidden(&sessions, all, |ix| ix == 1 || ix == 2),
            vec![false, true, true, false]
        );
        // An archived (not shown) grandchild does not hold its parent.
        assert_eq!(
            cleared_hidden(&sessions, |ix| ix != 2, |ix| ix == 1),
            vec![false, true, false, false]
        );
    }
    fn located(id: &str, parent: &str, cwd: &str, label: &str) -> Session {
        Session {
            id: id.into(),
            parent_session_id: parent.into(),
            cwd: cwd.into(),
            label: label.into(),
            ..Default::default()
        }
    }

    /// The fleet shape seen 2026-10-04: a manager in the repo checkout and
    /// workers in isolated worktrees, listed newest first.
    fn fleet() -> Vec<Session> {
        let trees = "/home/u/.workspacer/worktrees/workspacer";
        vec![
            located(
                "w3",
                "mgr",
                &format!("{trees}/native-themes"),
                "Native themes",
            ),
            located(
                "w2",
                "mgr",
                &format!("{trees}/native-controls"),
                "Native controls",
            ),
            located(
                "w1",
                "mgr",
                &format!("{trees}/native-editor"),
                "Native editor",
            ),
            located("other", "", "/home/u/Work/other", "Other project"),
            located("mgr", "", "/home/u/Work/worky/workspacer", "Fleet manager"),
        ]
    }

    #[test]
    fn workers_in_worktrees_nest_under_their_manager_through_filters() {
        let sessions = fleet();
        let all = lineage_filter(&sessions, |_| true, |_| true);
        assert_eq!(
            session_tree(&sessions, &all),
            vec![(3, 0), (4, 0), (0, 1), (1, 1), (2, 1)]
        );
        // The project filter on the manager's checkout keeps its workers.
        let project = lineage_filter(
            &sessions,
            |_| true,
            |ix| crate::projects::same_dir(&sessions[ix].cwd, "/home/u/Work/worky/workspacer"),
        );
        assert_eq!(
            session_tree(&sessions, &project),
            vec![(4, 0), (0, 1), (1, 1), (2, 1)]
        );
        // Searching one worker shows it under its manager, not its siblings.
        let search = lineage_filter(
            &sessions,
            |_| true,
            |ix| sessions[ix].label.contains("controls"),
        );
        assert_eq!(session_tree(&sessions, &search), vec![(4, 0), (1, 1)]);
        // An ineligible (archived) manager leaves its workers as roots.
        let archived = lineage_filter(&sessions, |ix| ix != 4, |_| true);
        assert_eq!(
            session_tree(&sessions, &archived),
            vec![(0, 0), (1, 0), (2, 0), (3, 0)]
        );
        assert_eq!(ancestors(&sessions, 0), vec![4]);
    }

    #[test]
    fn lineage_filter_survives_cycles() {
        let sessions = vec![session("a", "b"), session("b", "a"), session("c", "c")];
        assert_eq!(ancestors(&sessions, 0), vec![1]);
        assert_eq!(ancestors(&sessions, 2), Vec::<usize>::new());
        assert_eq!(
            lineage_filter(&sessions, |_| true, |ix| ix == 0),
            vec![0, 1]
        );
    }

    #[test]
    fn orphans_self_links_and_cycles_remain_visible_once() {
        let sessions = vec![
            session("a", "b"),
            session("b", "a"),
            session("self", "self"),
            session("orphan", "missing"),
        ];
        let rows = session_tree(&sessions, &[0, 1, 2, 3]);
        assert_eq!(rows, vec![(2, 0), (3, 0), (0, 0), (1, 1)]);
    }
    #[test]
    fn width_defaults_and_limits_leave_room_for_content() {
        assert_eq!(Settings::default().sidebar_width, 304.);
        assert_eq!(sidebar_width(f32::NAN, 1200.), 304.);
        assert_eq!(sidebar_width(520., 720.), 288.);
        assert_eq!(sidebar_width(10., 1200.), 200.);
        assert_eq!(
            serde_json::from_str::<Settings>("{}")
                .unwrap()
                .sidebar_width,
            304.
        );
    }
}

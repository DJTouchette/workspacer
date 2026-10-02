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
    pub sidebar_width: f32,
    pub reading: BTreeMap<String, crate::reading::Bookmark>,
    pub vim_navigation: bool,
    pub keep_running: bool,
    pub notifications: bool,
    /// Client-local session organization, scoped by hub identity.
    pub names: BTreeMap<String, BTreeMap<String, String>>,
    pub archived: BTreeMap<String, Vec<String>>,
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
            sidebar_width: 304.,
            reading: BTreeMap::new(),
            vim_navigation: true,
            keep_running: false,
            notifications: true,
            names: BTreeMap::new(),
            archived: BTreeMap::new(),
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
    fn clock_preference_is_backwards_compatible_and_round_trips() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(!old.twelve_hour_clock);
        let selected = Settings {
            twelve_hour_clock: true,
            ..old
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&selected).unwrap()).unwrap();
        assert!(restored.twelve_hour_clock);
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
pub fn sidebar_width(preferred: f32, viewport: f32) -> f32 {
    let preferred = if preferred.is_finite() {
        preferred
    } else {
        304.
    };
    preferred.clamp(200., (viewport * 0.4).clamp(200., 520.))
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

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
    pub vim_navigation: bool,
    pub keep_running: bool,
    pub notifications: bool,
    /// Client-local session organization, scoped by hub identity.
    pub names: BTreeMap<String, BTreeMap<String, String>>,
    pub archived: BTreeMap<String, Vec<String>>,
    pub default_provider: Provider,
    /// Bookmarks are scoped to the hub endpoint so remote paths do not cross hosts.
    pub projects: BTreeMap<String, Vec<String>>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            vim_navigation: true,
            keep_running: false,
            notifications: true,
            names: BTreeMap::new(),
            archived: BTreeMap::new(),
            default_provider: Provider::Claude,
            projects: BTreeMap::new(),
        }
    }
}
impl Settings {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
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
        settings.add_project("hub", "/app").unwrap();
        settings.save(&path).unwrap();
        let read = Settings::load(&path).unwrap();
        assert!(!read.vim_navigation);
        assert_eq!(read.default_provider, Provider::Codex);
        assert_eq!(read.bookmarks("hub"), ["/app"]);
        std::fs::write(&path, b"{}").unwrap();
        assert!(Settings::load(&path).unwrap().vim_navigation);
        std::fs::write(&path, b"{").unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}

//! The library: reusable prompts / skills / agents, loaded from disk like the
//! Electron app's Library pane. Sources (all read-only here):
//!   - `~/.config/workspacer/library/*.md`  — workspacer prompts (frontmatter)
//!   - `~/.claude/skills/<id>/SKILL.md`      — Claude skills
//!   - `~/.claude/agents/<id>.md`            — Claude subagent definitions
//!
//! Each item's `body` is the markdown after the frontmatter; the command
//! palette can insert it into the focused agent.

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct LibraryItem {
    pub title: String,
    /// "prompt" | "skill" | "agent".
    pub kind: String,
    pub description: Option<String>,
    pub body: String,
}

/// Load every library item from the user's global locations. Best-effort: any
/// missing directory or unreadable file is just skipped.
pub fn load() -> Vec<LibraryItem> {
    let mut items = Vec::new();
    let Some(dirs) = directories::BaseDirs::new() else { return items };
    let home = dirs.home_dir();

    load_md_dir(&dirs.config_dir().join("workspacer").join("library"), "prompt", &mut items);
    load_skills(&home.join(".claude").join("skills"), &mut items);
    load_agents(&home.join(".claude").join("agents"), &mut items);

    items.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.title.cmp(&b.title)));
    items
}

/// Flat directory of `*.md` prompt/library files.
fn load_md_dir(dir: &Path, default_kind: &str, out: &mut Vec<LibraryItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let (fm, body) = parse_frontmatter(&text);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("untitled");
        out.push(LibraryItem {
            title: fm.get("title").cloned().unwrap_or_else(|| stem.to_string()),
            kind: fm.get("kind").cloned().unwrap_or_else(|| default_kind.to_string()),
            description: fm.get("description").cloned(),
            body,
        });
    }
}

/// `~/.claude/skills/<id>/SKILL.md`.
fn load_skills(dir: &Path, out: &mut Vec<LibraryItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let skill = entry.path().join("SKILL.md");
        let Ok(text) = std::fs::read_to_string(&skill) else { continue };
        let (fm, body) = parse_frontmatter(&text);
        let id = entry.file_name().to_string_lossy().to_string();
        out.push(LibraryItem {
            title: fm.get("name").cloned().unwrap_or(id),
            kind: "skill".into(),
            description: fm.get("description").cloned(),
            body,
        });
    }
}

/// `~/.claude/agents/<id>.md`.
fn load_agents(dir: &Path, out: &mut Vec<LibraryItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let (fm, body) = parse_frontmatter(&text);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("agent");
        out.push(LibraryItem {
            title: fm.get("name").cloned().unwrap_or_else(|| stem.to_string()),
            kind: "agent".into(),
            description: fm.get("description").cloned(),
            body,
        });
    }
}

/// Split leading `---` YAML frontmatter from the markdown body. Only scalar
/// `key: value` lines are parsed (enough for title/kind/name/description); the
/// body is everything after the closing `---`.
pub fn parse_frontmatter(text: &str) -> (HashMap<String, String>, String) {
    let mut map = HashMap::new();
    let trimmed = text.strip_prefix('\u{feff}').unwrap_or(text); // tolerate BOM
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (map, text.to_string());
    };
    // Find the closing fence at the start of a line.
    let Some(end) = rest.find("\n---") else {
        return (map, text.to_string());
    };
    let front = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']).to_string();
    for line in front.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            if !v.is_empty() {
                map.insert(k.trim().to_string(), v);
            }
        }
    }
    (map, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\ntitle: My Prompt\nkind: prompt\ndescription: \"does a thing\"\n---\nHello {{x}}\nbody line 2\n";
        let (fm, body) = parse_frontmatter(text);
        assert_eq!(fm.get("title").unwrap(), "My Prompt");
        assert_eq!(fm.get("kind").unwrap(), "prompt");
        assert_eq!(fm.get("description").unwrap(), "does a thing");
        assert_eq!(body, "Hello {{x}}\nbody line 2\n");
    }

    #[test]
    fn no_frontmatter_returns_whole_body() {
        let (fm, body) = parse_frontmatter("just text\n");
        assert!(fm.is_empty());
        assert_eq!(body, "just text\n");
    }
}

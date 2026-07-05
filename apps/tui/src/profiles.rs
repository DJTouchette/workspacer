//! Claude profiles, read straight from the file the Electron app writes:
//! `~/.config/workspacer/claude-profiles.json`. A profile carries a custom
//! `CLAUDE_CONFIG_DIR` and extra CLI args (which is where `--model` and
//! `--dangerously-skip-permissions` live), so spawning only needs a cwd + a
//! chosen profile.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    #[serde(rename = "configDir", default)]
    pub config_dir: String,
    #[serde(rename = "extraArgs", default)]
    pub extra_args: Vec<String>,
    #[serde(rename = "isDefault", default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<Profile>,
}

impl Profile {
    fn default_profile() -> Self {
        Profile {
            id: "default".into(),
            name: "Default".into(),
            config_dir: String::new(),
            extra_args: Vec::new(),
            is_default: true,
        }
    }
}

/// Load the configured profiles, always returning at least a synthetic
/// "Default" so the spawn picker is never empty. The default is ordered first.
pub fn load() -> Vec<Profile> {
    let mut profiles = read_file().unwrap_or_default();
    if !profiles.iter().any(|p| p.is_default) {
        profiles.insert(0, Profile::default_profile());
    }
    profiles.sort_by_key(|p| !p.is_default); // defaults first
    profiles
}

fn read_file() -> Option<Vec<Profile>> {
    let dirs = directories::BaseDirs::new()?;
    let path = dirs
        .config_dir()
        .join("workspacer")
        .join("claude-profiles.json");
    let text = std::fs::read_to_string(path).ok()?;
    let parsed: ProfilesFile = serde_json::from_str(&text).ok()?;
    if parsed.profiles.is_empty() {
        None
    } else {
        Some(parsed.profiles)
    }
}

/// Build the argv claudemon should execute for a fresh Claude session, mirroring
/// the Electron app's `buildClaudeArgv`: base binary, then the profile's extra
/// args, then `--model` / skip-permissions unless the profile already pins them.
///
/// `session_id` pins `--session-id <uuid>` so claude names its transcript
/// `<uuid>.jsonl` — the same id we hand to claudemon and track here. Without it,
/// claudemon would have to guess the transcript by cwd and could serve the wrong
/// one when several agents share a directory. Pass "" to skip (non-claude spawns).
///
/// When `resume` is set, the same id is passed as `--resume <uuid>` instead so
/// claude reopens that transcript (its conversation) rather than starting blank.
/// `--resume` and `--session-id` are mutually exclusive, so resume wins.
pub fn build_argv(
    profile: &Profile,
    model: Option<&str>,
    skip_permissions: bool,
    session_id: &str,
    resume: bool,
) -> Vec<String> {
    let claude = std::env::var("WKS_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let mut argv = vec![claude];
    argv.extend(profile.extra_args.iter().cloned());

    let pins_model = profile
        .extra_args
        .iter()
        .any(|a| a == "--model" || a.starts_with("--model="));
    if let Some(m) = model {
        let m = m.trim();
        if !m.is_empty() && !pins_model {
            argv.push("--model".into());
            argv.push(m.into());
        }
    }

    let already_skips = profile
        .extra_args
        .iter()
        .any(|a| a == "--dangerously-skip-permissions");
    if skip_permissions && !already_skips {
        argv.push("--dangerously-skip-permissions".into());
    }

    if resume {
        if !session_id.is_empty() {
            argv.push("--resume".into());
            argv.push(session_id.into());
        }
    } else {
        let pins_id = profile
            .extra_args
            .iter()
            .any(|a| a == "--session-id" || a.starts_with("--session-id="));
        if !session_id.is_empty() && !pins_id {
            argv.push("--session-id".into());
            argv.push(session_id.into());
        }
    }
    argv
}

/// The env overrides a profile implies — currently just `CLAUDE_CONFIG_DIR`,
/// with a leading `~` expanded.
pub fn build_env(profile: &Profile) -> serde_json::Map<String, serde_json::Value> {
    let mut env = serde_json::Map::new();
    if !profile.config_dir.is_empty() {
        env.insert(
            "CLAUDE_CONFIG_DIR".into(),
            serde_json::Value::String(expand_tilde(&profile.config_dir)),
        );
    }
    env
}

pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix('~') {
        if let Some(dirs) = directories::BaseDirs::new() {
            return format!("{}{}", dirs.home_dir().display(), rest);
        }
    }
    p.to_string()
}

/// The cwd to spawn with: tilde-expanded and trailing-slash-stripped. The strip
/// matters — tab-completion leaves a trailing `/` on directories, but Claude
/// reports its cwd without one, and claudemon aliases a spawn to Claude's
/// session by exact cwd match. A mismatched slash means the agent shows up
/// twice (one row with the terminal, one with only the hook state).
pub fn normalize_cwd(p: &str) -> String {
    let mut s = expand_tilde(p.trim());
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_uses_resume_flag_not_session_id() {
        let p = Profile::default_profile();
        let argv = build_argv(&p, None, false, "abc-123", true);
        assert!(argv.windows(2).any(|w| w == ["--resume", "abc-123"]));
        assert!(!argv.iter().any(|a| a == "--session-id"));
    }

    #[test]
    fn fresh_spawn_uses_session_id_not_resume() {
        let p = Profile::default_profile();
        let argv = build_argv(&p, None, false, "abc-123", false);
        assert!(argv.windows(2).any(|w| w == ["--session-id", "abc-123"]));
        assert!(!argv.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn normalize_strips_trailing_slashes() {
        assert_eq!(normalize_cwd("/home/u/backshop/"), "/home/u/backshop");
        assert_eq!(normalize_cwd("/home/u/backshop///"), "/home/u/backshop");
        assert_eq!(normalize_cwd("  /home/u/backshop/  "), "/home/u/backshop");
        assert_eq!(normalize_cwd("/home/u/backshop"), "/home/u/backshop");
        assert_eq!(normalize_cwd("/"), "/"); // root preserved
    }
}

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
    pub(crate) fn default_profile() -> Self {
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
    let path = crate::config::config_dir()?.join("claude-profiles.json");
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

/// A profile's extra argv, minus the flags the stream transport's own builder
/// owns.
///
/// `build_argv` above composes the whole PTY command line, so a profile's flags
/// go in verbatim. The stream path is the other way round: claudemon builds the
/// headless argv itself from typed payload fields (`model`, `yolo`,
/// `session_id`/`resume`) and appends `extra_args` after them. Passing those
/// same flags through as extra argv would hand the CLI each one twice — and
/// `--session-id` twice with different values is not a cosmetic problem.
///
/// Everything else passes through: that's the escape hatch profiles exist for.
pub fn stream_extra_args(profile: &Profile) -> Vec<String> {
    /// Flags claudemon's stream builder emits, with the value-taking ones
    /// needing their following token dropped too.
    const OWNED_WITH_VALUE: [&str; 3] = ["--model", "--session-id", "--resume"];
    const OWNED_BARE: [&str; 1] = ["--dangerously-skip-permissions"];

    let mut out = Vec::with_capacity(profile.extra_args.len());
    let mut skip_value = false;
    for arg in &profile.extra_args {
        if skip_value {
            skip_value = false;
            continue;
        }
        // `--flag=value` carries its value inline, so nothing follows to drop.
        let name = arg.split('=').next().unwrap_or(arg.as_str());
        if OWNED_WITH_VALUE.contains(&name) {
            skip_value = !arg.contains('=');
            continue;
        }
        if OWNED_BARE.contains(&arg.as_str()) {
            continue;
        }
        out.push(arg.clone());
    }
    out
}

/// Whether a profile already pins bypass mode in its own argv — the stream path
/// expresses that as the payload's `yolo` flag instead of a raw arg.
pub fn profile_skips_permissions(profile: &Profile) -> bool {
    profile
        .extra_args
        .iter()
        .any(|a| a == "--dangerously-skip-permissions")
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

/// The ASCII whitespace set every caller-string trim on this seam strips: space,
/// tab, and the four vertical/form controls. Spelled as a literal set because no
/// language's built-in trim is portable across the three copies — Rust's
/// `str::trim` is Unicode White_Space (it strips U+0085 NEL and U+00A0 NBSP),
/// Go's `strings.TrimSpace` strips U+0085 but not U+FEFF, and JS `.trim()`
/// strips U+FEFF but not U+0085. Every one of those code points is an ordinary
/// character in a filename, which is what it is on the filesystem.
///
/// TWINS: `asciiWhitespace` in cmd/brain/profiles.go, `TRIM_SET` in spawnCwd.ts.
const ASCII_WHITESPACE: [char; 6] = [' ', '\t', '\n', '\u{b}', '\u{c}', '\r'];

/// The ONE normalization a spawn / terminal working directory gets before it is
/// handed to claudemon. Trims the ASCII whitespace set, strips trailing
/// separators, and falls back to the home directory only when nothing is left.
///
/// The strip matters: tab-completion leaves a trailing `/` on directories, but
/// Claude reports its cwd without one, and claudemon aliases a spawn to Claude's
/// own session by EXACT cwd match. A mismatched slash means the agent shows up
/// twice (one row with the terminal, one with only the hook state).
///
/// It deliberately does NOT expand `~` any more. This is the THIRD copy of a
/// rule contracts/path-containment-cases.json owns, and it was registered
/// nowhere: it diverged from the two declared owners on 7 of the block's 14
/// cases — the tilde expansion BINDING DECISION 1 exists to kill, Rust's Unicode
/// trim eating U+0085 and U+00A0, only `/` stripped and not `\\`, and `""`
/// answering `""` where the twins answer the home directory. What comes out of
/// here becomes claudemon's stored session cwd, which is exactly what the
/// brain's `agentCwds()` lifts into the fs.* allow-list, so a divergence here is
/// a divergence in what every bus client may read.
///
/// Tilde expansion for a path a HUMAN typed belongs in the dialog that took the
/// keystrokes (see `submit_spawn`), not in the seam normalizer — the same split
/// the other two copies make.
///
/// TWINS: `normalizeCwd` in cmd/brain/profiles.go, `normalizeSpawnCwd` in
/// apps/desktop/src/main/lib/spawnCwd.ts. The fixture's `spawnCwds` block holds
/// all three together.
pub fn normalize_cwd(p: &str) -> String {
    let trimmed = p.trim_matches(|c| ASCII_WHITESPACE.contains(&c));
    let mut s = trimmed.to_string();
    while s.len() > 1 && (s.ends_with('/') || s.ends_with('\\')) {
        s.pop();
    }
    if s.is_empty() {
        // A terminal has to open SOMEWHERE, and this is the only fallback.
        return directories::BaseDirs::new()
            .map(|d| d.home_dir().display().to_string())
            .unwrap_or_default();
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

    fn profile_with(extra: &[&str]) -> Profile {
        Profile {
            extra_args: extra.iter().map(|s| s.to_string()).collect(),
            ..Profile::default_profile()
        }
    }

    /// On the stream transport claudemon builds the headless argv itself and
    /// appends `extra_args` after it, so anything it already emits has to come
    /// out of the profile's own flags or the CLI gets it twice — and two
    /// different `--session-id`s is not a cosmetic problem.
    #[test]
    fn stream_extra_args_drops_the_flags_the_daemon_emits() {
        let p = profile_with(&[
            "--model",
            "opus",
            "--session-id",
            "pinned-elsewhere",
            "--dangerously-skip-permissions",
            "--add-dir",
            "/srv",
        ]);
        // Only the flag the payload can't express survives, with its value.
        assert_eq!(stream_extra_args(&p), vec!["--add-dir", "/srv"]);
    }

    #[test]
    fn stream_extra_args_handles_the_inline_value_form() {
        // `--model=opus` carries its value, so no following token to drop —
        // dropping one would eat the next real flag.
        let p = profile_with(&["--model=opus", "--verbose"]);
        assert_eq!(stream_extra_args(&p), vec!["--verbose"]);
    }

    #[test]
    fn stream_extra_args_passes_everything_else_through() {
        let p = profile_with(&[
            "--append-system-prompt",
            "be terse",
            "--settings",
            "/s.json",
        ]);
        assert_eq!(
            stream_extra_args(&p),
            vec![
                "--append-system-prompt",
                "be terse",
                "--settings",
                "/s.json"
            ]
        );
    }

    #[test]
    fn bypass_in_a_profile_becomes_the_yolo_flag_not_an_arg() {
        let bypass = profile_with(&["--dangerously-skip-permissions"]);
        assert!(profile_skips_permissions(&bypass));
        assert!(!stream_extra_args(&bypass)
            .iter()
            .any(|a| a == "--dangerously-skip-permissions"));

        assert!(!profile_skips_permissions(&profile_with(&["--verbose"])));
    }

    #[test]
    fn normalize_strips_trailing_slashes() {
        assert_eq!(normalize_cwd("/home/u/backshop/"), "/home/u/backshop");
        assert_eq!(normalize_cwd("/home/u/backshop///"), "/home/u/backshop");
        assert_eq!(normalize_cwd("  /home/u/backshop/  "), "/home/u/backshop");
        assert_eq!(normalize_cwd("/home/u/backshop"), "/home/u/backshop");
        assert_eq!(normalize_cwd("/"), "/"); // root preserved
    }

    /// The THIRD copy of the spawn-cwd normalizer, held to the same fixture as
    /// the other two.
    ///
    /// `contracts/path-containment-cases.json`'s `spawnCwds` block named exactly
    /// two owners — cmd/brain/profiles.go and spawnCwd.ts — and this function
    /// was neither, while sitting on the same seam: `submit_spawn` feeds it
    /// straight to claudemon's `/sessions/spawn`, and claudemon's stored session
    /// cwd is what the brain's `agentCwds()` lifts into the fs.* allow-list every
    /// bus client is confined to. It diverged on 7 of the 14 cases (tilde
    /// expansion, Rust's Unicode trim eating U+0085 and U+00A0, only `/`
    /// stripped, and `""` answering `""` instead of the home directory).
    ///
    /// `include_str!` rather than a runtime read, exactly as
    /// services/claudemon/src/session/pricing.rs consumes its fixture: it puts
    /// the file in this crate's rebuild inputs, so editing a case recompiles the
    /// test instead of leaving a cached pass behind.
    #[test]
    fn spawn_cwds_contract_cases() {
        const FIXTURE: &str = include_str!("../../../contracts/path-containment-cases.json");
        #[derive(serde::Deserialize)]
        struct Case {
            #[serde(rename = "in")]
            input: String,
            out: String,
            why: String,
        }
        #[derive(serde::Deserialize)]
        struct Block {
            cases: Vec<Case>,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            #[serde(rename = "spawnCwds")]
            spawn_cwds: Block,
        }
        let fx: Fixture = serde_json::from_str(FIXTURE).expect("the containment fixture parses");
        assert!(
            fx.spawn_cwds.cases.len() >= 14,
            "the spawnCwds block shrank to {} cases — a silently emptied block agrees with everything",
            fx.spawn_cwds.cases.len()
        );
        let home = directories::BaseDirs::new()
            .map(|d| d.home_dir().display().to_string())
            .expect("this process has a home directory");
        for c in &fx.spawn_cwds.cases {
            // ${HOME} is the block's only token.
            let want = c.out.replace("${HOME}", &home);
            let got = normalize_cwd(&c.input);
            assert_eq!(
                got, want,
                "normalize_cwd({:?}) = {:?}, the contract says {:?}\n  why: {}",
                c.input, got, want, c.why
            );
        }
    }
}

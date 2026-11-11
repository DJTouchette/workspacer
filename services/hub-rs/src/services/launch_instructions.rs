//! Per-session agent skills and trusted manager doctrine, shared with desktop.
//!
//! The skills are bundled from `apps/desktop/assets/skills` (see
//! scripts/generate-rust-launch-assets.py) and written once to a
//! content-addressed `~/.workspacer/agent-skills/<version>/`, laid out as one
//! Claude Code plugin per role (`workspacer`, `workspacer-fleet`). A launch then
//! hands its role's plugin to that session only: Claude via `--plugin-dir`,
//! Codex via claudemon's `skill_roots` (the app-server's `skills/extraRoots/set`),
//! other harnesses through an instruction line naming each SKILL.md. Nothing is
//! written into the project or a harness discovery root.
//!
//! Twin: apps/desktop/src/main/services/agentSkillPlugins.ts.
use anyhow::{Result, bail};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};
#[derive(Deserialize)]
struct Templates {
    native: String,
    pointer: String,
}
#[derive(Deserialize)]
struct Assets {
    version: String,
    instructions: BTreeMap<String, Templates>,
    files: BTreeMap<String, String>,
    manager: String,
}
fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/launch-instructions.json"))
            .expect("generated launch assets")
    })
}
pub fn manager_doctrine() -> &'static str {
    &assets().manager
}
pub fn skill_version() -> &'static str {
    &assets().version
}
pub const ORDINARY_PLUGIN: &str = "workspacer";
pub const MANAGER_PLUGIN: &str = "workspacer-fleet";

/// How a launch hands its role's skills to the harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loading {
    PluginDir,
    SkillRoots,
    Pointer,
}
pub fn loading(provider: &str) -> Loading {
    match provider {
        "" | "claude" => Loading::PluginDir,
        "codex" => Loading::SkillRoots,
        _ => Loading::Pointer,
    }
}

/// One launch's skills: Claude argv, Codex skill roots and the instruction
/// line that names the files (also the fallback when a plugin does not load).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillLaunch {
    pub args: Vec<String>,
    pub skill_roots: Vec<String>,
    pub instruction: String,
}

pub fn bundle_root(home: &Path) -> PathBuf {
    home.join(".workspacer")
        .join("agent-skills")
        .join(skill_version())
}

fn owned_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        bail!("skill directory is not an owned directory");
    }
    Ok(())
}

fn relative_parts(relative: &str) -> Result<Vec<&str>> {
    let parts: Vec<&str> = relative.split('/').collect();
    if Path::new(relative)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || parts.iter().any(|p| p.is_empty())
    {
        bail!("invalid bundled skill path");
    }
    Ok(parts)
}

/// Write every bundled file under the versioned root, verifying what is already
/// there. The root is app-owned and content-addressed, so a missing or altered
/// file is restored atomically (temporary file + rename); a symlink anywhere in
/// the tree refuses the whole root.
pub fn materialize(home: &Path) -> Result<PathBuf> {
    let root = bundle_root(home);
    let mut dir = home.to_path_buf();
    for part in [".workspacer", "agent-skills", skill_version()] {
        dir.push(part);
        owned_directory(&dir)?;
    }
    for (relative, content) in &assets().files {
        let parts = relative_parts(relative)?;
        let mut path = root.clone();
        for part in &parts[..parts.len() - 1] {
            path.push(part);
            owned_directory(&path)?;
        }
        path.push(parts[parts.len() - 1]);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => bail!("bundled skill is a symlink"),
            Ok(meta) if meta.is_file() && fs::read(&path)? == content.as_bytes() => continue,
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        let tmp = path.with_file_name(format!(
            "{}.{}.{}.tmp",
            parts[parts.len() - 1],
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path)?;
    }
    Ok(root)
}

fn quoted(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

/// The instruction line for `plugin`, with `{dir}` / `{skill:<name>}` bound to
/// JSON-quoted absolute paths (same substitution as the desktop twin).
pub fn skill_instruction(plugin_dir: &Path, plugin: &str, native: bool) -> String {
    let skills = plugin_dir.join("skills");
    let templates = &assets().instructions[plugin];
    let template = if native {
        &templates.native
    } else {
        &templates.pointer
    };
    let mut out = String::new();
    let mut rest = template.as_str();
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let key = &rest[start + 1..start + end];
        if key == "dir" {
            out.push_str(&quoted(&skills));
        } else if let Some(name) = key.strip_prefix("skill:") {
            out.push_str(&quoted(&skills.join(name).join("SKILL.md")));
        } else {
            out.push_str(&rest[start..=start + end]);
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

/// Prepare one launch's role skills. Pi gets nothing (no MCP bridge, so the
/// tool-driven skills would be decorative). A materialization failure omits
/// the skills rather than blocking the launch, as desktop does.
pub fn prepare_skills(provider: &str, cwd: &Path, home: &Path, manager: bool) -> SkillLaunch {
    remove_legacy_skills(cwd, home);
    if provider == "pi" {
        return SkillLaunch::default();
    }
    let root = match materialize(home) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("could not prepare Workspacer agent skills: {error}");
            return SkillLaunch::default();
        }
    };
    let plugin = if manager {
        MANAGER_PLUGIN
    } else {
        ORDINARY_PLUGIN
    };
    let plugin_dir = root.join(plugin);
    let loading = loading(provider);
    SkillLaunch {
        args: if loading == Loading::PluginDir {
            vec![
                "--plugin-dir".into(),
                plugin_dir.to_string_lossy().into_owned(),
            ]
        } else {
            vec![]
        },
        skill_roots: if loading == Loading::SkillRoots {
            vec![plugin_dir.join("skills").to_string_lossy().into_owned()]
        } else {
            vec![]
        },
        instruction: skill_instruction(&plugin_dir, plugin, loading != Loading::Pointer),
    }
}

fn safe_project(cwd: &Path, home: &Path) -> Result<PathBuf> {
    if !cwd.is_absolute() || fs::symlink_metadata(cwd)?.file_type().is_symlink() {
        bail!("unsafe skill cwd");
    }
    let cwd = fs::canonicalize(cwd)?;
    if cwd.parent().is_none() || fs::canonicalize(home).ok().as_ref() == Some(&cwd) {
        bail!("skills cannot be cleaned in home or filesystem root");
    }
    Ok(cwd)
}

fn real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// skill-relative path ("spawn-agent/SKILL.md") → every bundled body for it.
fn skill_files() -> &'static BTreeMap<String, BTreeSet<String>> {
    static FILES: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    FILES.get_or_init(|| {
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (relative, body) in &assets().files {
            if let Some((_, rest)) = relative.split_once("/skills/") {
                map.entry(rest.into()).or_default().insert(body.clone());
            }
        }
        map
    })
}

/// Remove exact bundled copies under `skills_root/<skill>/…` (one `<hash>`
/// level first when `hashed`), then the directories that leaves empty.
fn remove_exact_copies(skills_root: &Path, hashed: bool) {
    if !real_dir(skills_root) {
        return;
    }
    let bases: Vec<PathBuf> = if hashed {
        fs::read_dir(skills_root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| real_dir(path))
            .collect()
    } else {
        vec![skills_root.to_path_buf()]
    };
    for base in bases {
        for (relative, bodies) in skill_files() {
            let parts: Vec<&str> = relative.split('/').collect();
            let mut dir = base.clone();
            if !parts[..parts.len() - 1].iter().all(|part| {
                dir.push(part);
                real_dir(&dir)
            }) {
                continue;
            }
            let path = dir.join(parts[parts.len() - 1]);
            let exact = fs::symlink_metadata(&path)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                && fs::read_to_string(&path).is_ok_and(|body| bodies.contains(&body));
            if !exact || fs::remove_file(&path).is_err() {
                continue;
            }
            let mut current = dir;
            while current.starts_with(skills_root) && fs::remove_dir(&current).is_ok() {
                if current == skills_root {
                    break;
                }
                current.pop();
            }
        }
    }
}

/// Older releases installed these assets into the project: native discovery
/// roots (`.claude/skills`, `.agents/skills`) and a pointer copy under
/// `.workspacer/skills/<hash>/`. Remove only byte-identical regular files and
/// the directories that leaves empty; never follow a symlinked parent.
fn remove_legacy_skills(cwd: &Path, home: &Path) {
    let Ok(cwd) = safe_project(cwd, home) else {
        return;
    };
    for native in [".claude", ".agents"] {
        if real_dir(&cwd.join(native)) {
            remove_exact_copies(&cwd.join(native).join("skills"), false);
        }
    }
    if real_dir(&cwd.join(".workspacer")) {
        remove_exact_copies(&cwd.join(".workspacer").join("skills"), true);
    }
}

/// The launch's private instructions plus its skill delivery.
pub struct LaunchInstructions {
    pub text: String,
    pub skills: SkillLaunch,
}

pub fn launch(
    session: &str,
    provider: &str,
    cwd: &Path,
    home: &Path,
    manager: bool,
) -> LaunchInstructions {
    let skills = prepare_skills(provider, cwd, home, manager);
    let mut parts = vec![format!("You are running inside Workspacer session {session} with access to the local workspacer MCP facade."),"Use the workspacer MCP tools when they are relevant to the task. Your tool scope for this session is operator.".into()];
    if manager {
        parts.push(manager_doctrine().into());
    }
    if !skills.instruction.is_empty() {
        parts.push(skills.instruction.clone());
    }
    LaunchInstructions {
        text: parts.join("\n"),
        skills,
    }
}

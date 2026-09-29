//! Shared config writer. A path is required so tests and embedded runtimes do
//! not accidentally open the user's global configuration.
use crate::model_selection::{config_window, manager_preferences, normalize_model_selection};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

pub fn defaults() -> Value {
    serde_json::from_str(include_str!("../../assets/config-defaults.json"))
        .expect("valid compiled config defaults")
}

pub fn deep_merge(target: &Value, source: &Value) -> Value {
    let mut output = target.clone();
    if let (Some(target), Some(source)) = (output.as_object_mut(), source.as_object()) {
        for (key, value) in source {
            if key == "__proto__" || value.is_null() {
                continue;
            }
            if value.is_object() && target.get(key).is_some_and(Value::is_object) {
                target.insert(key.clone(), deep_merge(&target[key], value));
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    }
    output
}

pub fn drop_host_trusted(mut partial: Value) -> Value {
    if let Some(root) = partial.as_object_mut() {
        root.remove("updates");
        root.remove("scripts");
        for (section, key) in [
            ("agents", "binaries"),
            ("claude", "profiles"),
            ("terminal", "shell"),
            ("terminal", "shells"),
            ("editor", "terminalCommand"),
        ] {
            if let Some(map) = root.get_mut(section).and_then(Value::as_object_mut) {
                map.remove(key);
            }
        }
    }
    partial
}

pub fn merge_patch(current: &Value, mut partial: Value, owner: bool) -> Result<Value> {
    if !partial.is_object() {
        bail!("config patch must be an object");
    }
    if !owner {
        partial = drop_host_trusted(partial);
    }
    preserve_workflows(current, &mut partial)?;
    let mut merged = deep_merge(current, &partial);
    for pointer in ["/ui/customThemes", "/claude/budgets", "/projects"] {
        if let Some(value) = partial.pointer(pointer) {
            if !value.is_object() {
                bail!("wholesale config path must be an object: {pointer}");
            }
            if let Some(destination) = merged.pointer_mut(pointer) {
                *destination = value.clone();
            }
        }
    }
    normalize_claude(&mut merged, &partial)?;
    normalize_manager(&mut merged, &partial, true)?;
    Ok(merged)
}

fn preserve_workflows(current: &Value, partial: &mut Value) -> Result<()> {
    let old = &current["agents"];
    if partial
        .get("agents")
        .is_some_and(|a| !a.is_null() && !a.is_object())
        && (!old["defaultWorkflowId"].is_null() || !old["workflowSelectionRevision"].is_null())
    {
        bail!("workflow selections cannot be removed by replacing agents");
    }
    for key in ["defaultWorkflowId", "workflowSelectionRevision"] {
        if partial["agents"].get(key).is_some_and(|v| v != &old[key]) {
            bail!("use the desktop Fleet workflow selection API with expectedRevision");
        }
    }
    if let Some(projects) = partial.get_mut("projects").and_then(Value::as_object_mut) {
        for (cwd, project) in projects.iter_mut() {
            let old = &current["projects"][cwd];
            if !old["workflowId"].is_null() && !project.is_object() {
                bail!("set project workflow to inherit before replacing selected project");
            }
            if project
                .get("workflowId")
                .is_some_and(|v| v != &old["workflowId"])
            {
                bail!("use the desktop Fleet workflow selection API with expectedRevision");
            }
            if let (Some(selected), Some(map)) = (old.get("workflowId"), project.as_object_mut()) {
                map.insert("workflowId".into(), selected.clone());
            }
        }
        if let Some(old) = current["projects"].as_object() {
            for (cwd, project) in old {
                if project.get("workflowId").is_some() && !projects.contains_key(cwd) {
                    bail!("set project workflow to inherit before removing selected project");
                }
            }
        }
    }
    Ok(())
}

fn normalize_claude(config: &mut Value, source: &Value) -> Result<()> {
    let Some(claude) = config.get_mut("claude").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    let patch = &source["claude"];
    let model = patch
        .get("defaultModel")
        .or_else(|| claude.get("defaultModel"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("claude.defaultModel must be a string"))?;
    if model.trim().is_empty() {
        claude.insert("defaultModel".into(), json!(""));
        claude.insert("contextWindow".into(), Value::Null);
    } else {
        let window = patch.get("contextWindow").or_else(|| {
            if patch.get("defaultModel").is_none() {
                claude.get("contextWindow")
            } else {
                None
            }
        });
        let window = match window {
            None | Some(Value::Null) => None,
            Some(v) => config_window(v)?,
        };
        let selection = normalize_model_selection(model, window).map_err(|e| anyhow!(e.code()))?;
        claude.insert("defaultModel".into(), json!(selection.model));
        claude.insert("contextWindow".into(), json!(selection.context_window));
    }
    if let Some(seen) = claude.get("seenModels").and_then(Value::as_array) {
        let canonical: std::collections::BTreeSet<_> = seen
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|s| normalize_model_selection(s, None).ok().map(|s| s.model))
            .collect();
        claude.insert("seenModels".into(), json!(canonical));
    }
    Ok(())
}

fn normalize_manager(config: &mut Value, source: &Value, strict: bool) -> Result<()> {
    let Some(agents) = config.get_mut("agents").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if let Some(patch) = source["agents"]["managerContextWindows"].as_object() {
        let map = agents.entry("managerContextWindows").or_insert(json!({}));
        if let Some(map) = map.as_object_mut() {
            for (key, value) in patch {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    if source["agents"]["statusSummary"].get("model") == Some(&Value::Null)
        && let Some(summary) = agents
            .get_mut("statusSummary")
            .and_then(Value::as_object_mut)
    {
        summary.insert("model".into(), Value::Null);
    }
    let canonical = manager_preferences(&Value::Object(agents.clone()), strict)?;
    for (key, value) in canonical.as_object().unwrap() {
        agents.insert(key.clone(), value.clone());
    }
    Ok(())
}

pub struct Config {
    path: PathBuf,
    state: Mutex<State>,
}
struct State {
    current: Value,
    loaded: bool,
    blocked: bool,
    stamp: Option<(SystemTime, u64)>,
    broken: Vec<u8>,
}
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

#[derive(Debug)]
pub struct SelectionConflict {
    pub current_revision: u64,
}
impl std::fmt::Display for SelectionConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Workflow changed; reload revision {} before saving",
            self.current_revision
        )
    }
}
impl std::error::Error for SelectionConflict {}

impl Config {
    pub fn open(path: PathBuf) -> Self {
        let mut state = State {
            current: defaults(),
            loaded: false,
            blocked: false,
            stamp: None,
            broken: vec![],
        };
        refresh(&path, &mut state);
        Self {
            path,
            state: Mutex::new(state),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn get(&self) -> Value {
        let mut state = self.state.lock().unwrap();
        if stamp(&self.path) != state.stamp {
            refresh(&self.path, &mut state);
        }
        state.current.clone()
    }
    pub fn reload(&self) -> Value {
        let mut state = self.state.lock().unwrap();
        refresh(&self.path, &mut state);
        state.current.clone()
    }
    /// The only selection writer; ordinary config patches keep this state fixed.
    /// Definition availability is validated by the workflow service first.
    pub fn select_workflow(
        &self,
        expected_revision: u64,
        cwd: Option<&str>,
        workflow_id: Option<&str>,
    ) -> Result<Value> {
        if cwd.is_none() && workflow_id.is_none() {
            bail!("Global selection requires an enabled workflow id");
        }
        if cwd.is_some_and(|cwd| !Path::new(cwd).is_absolute())
            || workflow_id.is_some_and(|id| id.trim().is_empty())
        {
            bail!("Invalid workflow selection");
        }
        let mut state = self.state.lock().unwrap();
        let _guard = ConfigLock::take(&self.path)?;
        for _ in 0..5 {
            refresh(&self.path, &mut state);
            if state.blocked {
                bail!("Workflow selection cannot replace unreadable configuration");
            }
            let current = state.current["agents"]["workflowSelectionRevision"]
                .as_u64()
                .unwrap_or(0);
            if current != expected_revision {
                return Err(SelectionConflict {
                    current_revision: current,
                }
                .into());
            }
            if current >= 9_007_199_254_740_991 {
                bail!("Workflow selection revision exhausted");
            }
            let mut next = state.current.clone();
            if !next["agents"].is_object() {
                bail!("Invalid agents configuration");
            }
            if let Some(cwd) = cwd {
                if !next["projects"].is_object() {
                    next["projects"] = serde_json::json!({});
                }
                if !next["projects"][cwd].is_object() {
                    next["projects"][cwd] = serde_json::json!({});
                }
                if let Some(id) = workflow_id {
                    next["projects"][cwd]["workflowId"] = id.into();
                } else {
                    next["projects"][cwd]
                        .as_object_mut()
                        .unwrap()
                        .remove("workflowId");
                }
            } else {
                next["agents"]["defaultWorkflowId"] = workflow_id.unwrap().into();
            }
            next["agents"]["workflowSelectionRevision"] = (current + 1).into();
            if stamp(&self.path) != state.stamp {
                continue;
            }
            write_config(&self.path, &next)?;
            state.current = next;
            state.stamp = stamp(&self.path);
            return Ok(state.current.clone());
        }
        bail!("Configuration changed during workflow selection")
    }
    pub fn save(&self, partial: Value, owner: bool) -> Result<Value> {
        self.save_using(partial, owner, || {}, write_config)
    }
    fn save_using(
        &self,
        partial: Value,
        owner: bool,
        before_stamp_check: impl Fn(),
        write: impl Fn(&Path, &Value) -> Result<()>,
    ) -> Result<Value> {
        let mut state = self.state.lock().unwrap();
        let _guard = match ConfigLock::take(&self.path) {
            Ok(lock) => lock,
            Err(e) => {
                eprintln!("config save skipped: {e}");
                return Ok(state.current.clone());
            }
        };
        for _ in 0..5 {
            refresh(&self.path, &mut state);
            let merged = merge_patch(&state.current, partial.clone(), owner)?;
            if state.blocked {
                state.current = merged;
                return Ok(state.current.clone());
            }
            before_stamp_check();
            if stamp(&self.path) != state.stamp {
                continue;
            }
            if let Err(e) = write(&self.path, &merged) {
                eprintln!("config write failed: {e}");
                return Ok(state.current.clone());
            }
            state.current = merged;
            state.stamp = stamp(&self.path);
            return Ok(state.current.clone());
        }
        eprintln!("config save skipped: file changed during all five attempts");
        Ok(state.current.clone())
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn failed_write_never_reports_or_caches_unpersisted_settings_and_releases_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, "ui:\n  theme: before\n").unwrap();
        let config = Config::open(path.clone());
        let prior = fs::read(&path).unwrap();
        let called = std::cell::Cell::new(false);
        let result = config
            .save_using(
                serde_json::json!({"ui":{"theme":"after"}}),
                false,
                || {},
                |_, _| {
                    called.set(true);
                    bail!("fixture atomic write failure")
                },
            )
            .unwrap();
        assert!(called.get());
        assert_eq!(result["ui"]["theme"], "before");
        assert_eq!(config.get()["ui"]["theme"], "before");
        assert_eq!(fs::read(&path).unwrap(), prior);
        assert!(!dir.path().join("config.yaml.lock").exists());
        assert_eq!(
            config
                .save(serde_json::json!({"ui":{"theme":"recovered"}}), false)
                .unwrap()["ui"]["theme"],
            "recovered"
        );
    }

    #[test]
    fn stamp_retry_merges_an_outside_write_and_bounds_continuous_churn() {
        for continuous in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.yaml");
            let config = Config::open(path.clone());
            config
                .save(json!({"projects":{"existing":{"label":"Keep"}}}), false)
                .unwrap();
            let attempts = std::cell::Cell::new(0);
            let writes = std::cell::Cell::new(0);
            let result = config
                .save_using(
                    json!({"ui":{"theme":"requested"}}),
                    false,
                    || {
                        let attempt = attempts.get();
                        attempts.set(attempt + 1);
                        if attempt == 0 || continuous {
                            let mut outside: Value =
                                serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
                            outside["projects"]["outsider"] =
                                json!({"label":"x".repeat(attempt + 1)});
                            // Different length guarantees a changed stamp even on coarse filesystems.
                            write_config(&path, &outside).unwrap();
                        }
                    },
                    |path, next| {
                        writes.set(writes.get() + 1);
                        write_config(path, next)
                    },
                )
                .unwrap();
            let disk = Config::open(path).get();
            assert_eq!(disk["projects"]["existing"]["label"], "Keep");
            assert!(disk["projects"].get("outsider").is_some());
            assert!(!dir.path().join("config.yaml.lock").exists());
            if continuous {
                assert_eq!(attempts.get(), 5);
                assert_eq!(writes.get(), 0);
                assert_ne!(result["ui"]["theme"], "requested");
                assert_ne!(disk["ui"]["theme"], "requested");
                assert_eq!(
                    config
                        .save(json!({"ui":{"theme":"recovered"}}), false)
                        .unwrap()["ui"]["theme"],
                    "recovered"
                );
            } else {
                assert_eq!(attempts.get(), 2);
                assert_eq!(writes.get(), 1);
                assert_eq!(result["ui"]["theme"], "requested");
                assert_eq!(disk["ui"]["theme"], "requested");
            }
        }
    }

    #[test]
    fn bare_defaults_cannot_replace_populated_state_but_seed_and_normal_saves_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(&path, &defaults()).unwrap();
        write_config(&path, &defaults()).unwrap();
        let original = "ui:\n  theme: custom\nclaude:\n  skipPermissionsDefault: true\nprojects:\n  custom: {name: Keep}\nonboardingDismissed: true\n";
        fs::write(&path, original).unwrap();
        let mut migrated = defaults();
        migrated["keybindings"] = json!({"mode":"vim","leader":"space"});
        migrate_keys(&mut migrated);
        assert_eq!(migrated, defaults());
        assert!(
            write_config(&path, &migrated)
                .unwrap_err()
                .to_string()
                .contains("bare defaults")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let config = Config::open(path.clone());
        config
            .save(json!({"ui":{"theme":"changed"}}), false)
            .unwrap();
        let restored = Config::open(path).get();
        assert_eq!(restored["ui"]["theme"], "changed");
        assert_eq!(restored["claude"]["skipPermissionsDefault"], true);
        assert_eq!(restored["projects"]["custom"]["name"], "Keep");
        assert_eq!(restored["onboardingDismissed"], true);
    }
}

fn refresh(path: &Path, state: &mut State) {
    state.blocked = false;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if state.loaded {
                state.blocked = true;
            } else {
                if let (Some(directory), Some(name)) = (path.parent(), path.file_name())
                    && crate::state_loss::suspected(directory, name)
                {
                    eprintln!(
                        "STATE LOSS: {} is missing, but {} still holds the rest of this install. Reseeding factory defaults — projects, agents.binaries, transport, budgets and keybindings are all back to their shipped values, and this process will run on them. If the file is recoverable (a backup, or a volume that failed to mount), restore it and restart before anything writes over the seed.",
                        path.display(),
                        directory.display()
                    );
                }
                state.current = defaults();
                if let Err(e) = write_config(path, &state.current) {
                    eprintln!("config default seed failed: {e}");
                }
            }
            state.loaded = true;
            state.stamp = stamp(path);
            return;
        }
        Err(e) => {
            eprintln!("config read failed; persistence blocked: {e}");
            state.blocked = true;
            state.loaded = true;
            state.current = defaults();
            state.stamp = stamp(path);
            return;
        }
    };
    let parsed = serde_yaml::from_slice::<Value>(&bytes);
    let mut parsed = match parsed {
        Ok(value) if value.is_object() => value,
        result => {
            // YAML scalars/arrays are also malformed configuration documents.
            // Empty/comment-only YAML decodes as null and is protected without
            // creating a meaningless backup, matching the existing writers.
            let backup_needed = match &result {
                Err(_) => true,
                Ok(value) => !value.is_null(),
            };
            if backup_needed && state.broken != bytes {
                let backup = PathBuf::from(format!(
                    "{}.broken-{}",
                    path.display(),
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                ));
                if fs::write(backup, &bytes).is_ok() {
                    state.broken = bytes;
                }
            }
            state.blocked = true;
            state.loaded = true;
            state.current = defaults();
            state.stamp = stamp(path);
            return;
        }
    };
    state.broken.clear();
    let had_orphan = parsed
        .as_object_mut()
        .unwrap()
        .remove("supervisor")
        .is_some();
    if had_orphan && let Ok(text) = std::str::from_utf8(&bytes) {
        let pruned = strip_top_level_block(text, "supervisor");
        if pruned != text
            && serde_yaml::from_str::<Value>(&pruned).ok().as_ref() == Some(&parsed)
            && let Err(e) = atomic_bytes(path, pruned.as_bytes())
        {
            eprintln!("config retired-key cleanup failed: {e}");
        }
    }
    let mut merged = deep_merge(&defaults(), &parsed);
    if normalize_claude(&mut merged, &parsed).is_err()
        && let Some(claude) = merged.get_mut("claude").and_then(Value::as_object_mut)
    {
        claude.insert(
            "defaultModel".into(),
            defaults()["claude"]["defaultModel"].clone(),
        );
        claude.insert(
            "contextWindow".into(),
            defaults()["claude"]["contextWindow"].clone(),
        );
    }
    if let Err(e) = normalize_manager(&mut merged, &parsed, false) {
        eprintln!("config manager normalization failed: {e}");
    }
    let before = merged.clone();
    migrate_keys(&mut merged);
    if before != merged
        && let Err(e) = write_config(path, &merged)
    {
        eprintln!("config migration write failed: {e}");
    }
    state.current = merged;
    state.loaded = true;
    state.stamp = stamp(path);
}

fn write_config(path: &Path, value: &Value) -> Result<()> {
    if value == &defaults()
        && let Ok(bytes) = fs::read(path)
        && let Ok(existing) = serde_yaml::from_slice::<Value>(&bytes)
        && existing.is_object()
        && existing != *value
    {
        bail!("refusing to replace a populated config.yaml with bare defaults");
    }
    atomic_bytes(path, serde_yaml::to_string(value)?.as_bytes())
}
pub(crate) fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub(crate) struct ConfigLock {
    path: PathBuf,
}
impl ConfigLock {
    pub(crate) fn take(path: &Path) -> Result<Self> {
        let lock = PathBuf::from(format!("{}.lock", path.display()));
        fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
            {
                Ok(mut file) => {
                    writeln!(file, "{} {}", std::process::id(), crate::protocol::now())?;
                    return Ok(Self { path: lock });
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::AlreadyExists
                        || (cfg!(windows)
                            && (e.kind() == std::io::ErrorKind::PermissionDenied
                                || e.raw_os_error() == Some(32))) => {}
                Err(e) => return Err(e.into()),
            }
            if fs::metadata(&lock)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(10))
                && fs::remove_file(&lock).is_ok()
            {
                continue;
            }
            if Instant::now() >= deadline {
                bail!("config.yaml is locked by another process");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn strip_top_level_block(text: &str, key: &str) -> String {
    let head = regex::Regex::new(&format!(r"^{}:(\s|$)", regex::escape(key))).unwrap();
    let lines: Vec<_> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !head.is_match(lines[i].trim_end_matches('\r')) {
            out.push(lines[i]);
            i += 1;
            continue;
        }
        i += 1;
        let mut pending = Vec::new();
        while i < lines.len() {
            let line = lines[i].trim_end_matches('\r');
            if line.trim().is_empty() {
                pending.push(lines[i]);
                i += 1;
                continue;
            }
            if line.starts_with([' ', '\t']) {
                pending.clear();
                i += 1;
                continue;
            }
            break;
        }
        if i >= lines.len() {
            out.extend(pending);
        }
    }
    out.join("\n")
}
fn migrate_keys(config: &mut Value) {
    let defaults = defaults();
    let Some(kb) = config.get_mut("keybindings").and_then(Value::as_object_mut) else {
        return;
    };
    if kb.contains_key("mode")
        || kb.contains_key("leader")
        || kb
            .get("prefix")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
    {
        *kb = defaults["keybindings"].as_object().unwrap().clone();
        return;
    }
    let Some(shortcuts) = kb.get_mut("shortcuts").and_then(Value::as_object_mut) else {
        return;
    };
    shortcuts.remove("cycle-view");
    for (key, old) in [
        ("new-terminal", "prefix n t"),
        ("new-claude", "prefix n c"),
        ("new-browser", "prefix n b"),
        ("prev-tab", "prefix t ["),
        ("next-tab", "prefix t ]"),
        ("move-tab-left", "prefix t ,"),
        ("move-tab-right", "prefix t ."),
        ("rename-tab", "prefix t r"),
        ("close-pane", "prefix t w"),
        ("split", "prefix p s"),
        ("quick-split", "prefix p c"),
        ("nav-left", "prefix p h"),
        ("nav-down", "prefix p j"),
        ("nav-up", "prefix p k"),
        ("nav-right", "prefix p l"),
    ] {
        if shortcuts.get(key).and_then(Value::as_str) == Some(old)
            && let Some(value) = defaults["keybindings"]["shortcuts"].get(key)
        {
            shortcuts.insert(key.into(), value.clone());
        }
    }
}

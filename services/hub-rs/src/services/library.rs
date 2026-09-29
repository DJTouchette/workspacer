//! Markdown/YAML library storage. Derived files stay in semantic library roots;
//! public results mask MCP credentials, while launch resolution reads originals.
use super::{
    atomic_json,
    config::{ConfigLock, atomic_bytes},
    dispatch_templates, paths,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};
const SECRET: &str = "__WKS_SECRET__";
const KINDS: &[&str] = &["prompt", "skill", "agent", "mcp", "command", "dispatch"];
pub struct Library {
    directory: PathBuf,
}
fn validate(input: &Value) -> Result<()> {
    if !input.is_object() && !input.is_null() {
        bail!("library parameters must be an object");
    }
    for key in [
        "cwd",
        "scope",
        "id",
        "title",
        "kind",
        "description",
        "action",
        "origin",
        "body",
    ] {
        if input
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            bail!("{key} must be a string");
        }
    }
    if let Some(tags) = input.get("tags").filter(|v| !v.is_null()) {
        if !tags
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
        {
            bail!("tags must be an array of strings");
        }
    }
    if let Some(mcp) = input.get("mcp").filter(|v| !v.is_null()) {
        if !mcp.is_object() {
            bail!("mcp must be an object");
        }
        for key in ["type", "command", "url"] {
            if mcp.get(key).is_some_and(|v| !v.is_null() && !v.is_string()) {
                bail!("mcp.{key} must be a string");
            }
        }
        if mcp.get("args").is_some_and(|v| {
            !v.is_null() && !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
        }) {
            bail!("mcp.args must contain strings");
        }
        for key in ["env", "headers"] {
            if mcp.get(key).is_some_and(|v| {
                !v.is_null()
                    && !v
                        .as_object()
                        .is_some_and(|m| m.values().all(Value::is_string))
            }) {
                bail!("mcp.{key} must map strings to strings");
            }
        }
    }
    Ok(())
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
pub fn slug(text: &str) -> String {
    static BAD: OnceLock<regex::Regex> = OnceLock::new();
    let text = text.to_ascii_lowercase();
    let slug = BAD
        .get_or_init(|| regex::Regex::new("[^a-z0-9_-]+").unwrap())
        .replace_all(&text, "-");
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "item".into()
    } else {
        slug.into()
    }
}
fn basename(id: &str) -> Result<&str> {
    if id.is_empty() || matches!(id, "." | "..") || id.contains(['/', '\\']) {
        bail!("invalid library item id");
    }
    Ok(id)
}
pub fn parse(raw: &str) -> (Value, String) {
    static FRONT: OnceLock<regex::Regex> = OnceLock::new();
    if let Some(capture) = FRONT
        .get_or_init(|| regex::Regex::new(r"(?s)^---\r?\n(.*?)\r?\n---\r?\n?(.*)$").unwrap())
        .captures(raw)
    {
        if let Ok(value) = serde_yaml::from_str::<Value>(&capture[1]) {
            if value.is_object() || value.is_null() {
                return (
                    if value.is_null() { json!({}) } else { value },
                    capture[2].into(),
                );
            }
        }
    }
    (json!({}), raw.into())
}
fn strip_leading_blank(body: &str) -> String {
    let mut start = 0;
    for (index, ch) in body.char_indices() {
        if !ch.is_whitespace() {
            break;
        }
        if ch == '\n' {
            start = index + 1;
        }
    }
    body[start..].to_owned()
}
fn serialize(metadata: &Value, body: &str) -> Result<String> {
    Ok(format!(
        "---\n{}\n---\n\n{}\n",
        serde_yaml::to_string(metadata)?.trim_end_matches('\n'),
        body.trim_end_matches([' ', '\t', '\r', '\n', '\u{b}', '\u{c}'])
    ))
}
fn kind(value: &Value) -> &str {
    let value = value.as_str().unwrap_or("");
    if matches!(value, "skill" | "agent" | "mcp" | "dispatch") {
        value
    } else {
        "prompt"
    }
}
fn clean_mcp(value: &Value) -> Option<Value> {
    let map = value.as_object()?;
    let mut result = json!({});
    for key in ["type", "command", "url"] {
        if let Some(value) = map.get(key).and_then(Value::as_str) {
            let value = if key == "type" { value } else { value.trim() };
            if !value.is_empty() {
                result[key] = json!(value);
            }
        }
    }
    if let Some(args) = map
        .get("args")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
    {
        if args.iter().all(Value::is_string) {
            result["args"] = json!(args);
        } else {
            return None;
        }
    }
    for key in ["env", "headers"] {
        if let Some(values) = map
            .get(key)
            .and_then(Value::as_object)
            .filter(|a| !a.is_empty())
        {
            if values.values().all(Value::is_string) {
                result[key] = json!(values);
            } else {
                return None;
            }
        }
    }
    Some(result)
}
fn redact(mut item: Value) -> Value {
    if item["kind"] == "mcp" {
        for key in ["env", "headers"] {
            if let Some(values) = item["mcp"][key].as_object_mut() {
                for value in values.values_mut() {
                    if value.as_str().is_some_and(|s| !s.is_empty()) {
                        *value = json!(SECRET);
                    }
                }
            }
        }
    }
    item
}
fn restore(mut incoming: Value, stored: &Value) -> Value {
    for key in ["env", "headers"] {
        if let Some(map) = incoming[key].as_object_mut() {
            map.retain(|name, value| {
                if value != SECRET {
                    return true;
                }
                if let Some(previous) = stored[key]
                    .get(name)
                    .filter(|v| v.is_string() && *v != SECRET)
                {
                    *value = previous.clone();
                    true
                } else {
                    false
                }
            });
        }
    }
    incoming
}
impl Library {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    fn global(&self) -> PathBuf {
        self.directory.join("library")
    }
    fn guard(&self, path: &Path, cwd: Option<&Path>, file: bool) -> Result<PathBuf> {
        let path = paths::canonicalize(path)?;
        let global = paths::canonicalize(&self.global())?;
        let allowed = path.starts_with(&global)
            || cwd.is_some_and(|cwd| {
                path.starts_with(cwd.join(".workspacer/library"))
                    || path.starts_with(cwd.join(".claude"))
            });
        if !allowed {
            bail!("library item is outside the selected library directories");
        }
        // A markdown alias must never expose a provider credential/config file.
        if file
            && !path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("md"))
        {
            bail!("library item must resolve to a markdown file");
        }
        Ok(path)
    }
    fn read(&self, path: &Path, cwd: Option<&Path>) -> Result<(PathBuf, Value, String)> {
        let path = self.guard(path, cwd, true)?;
        let metadata = std::fs::metadata(&path)?;
        if !metadata.is_file() || metadata.len() > 5 * 1024 * 1024 {
            bail!("library item is not a bounded regular file");
        }
        let raw = String::from_utf8(super::files::bounded_bytes(&path, 5 * 1024 * 1024)?)?;
        let (metadata, body) = parse(&raw);
        Ok((path, metadata, strip_leading_blank(&body)))
    }
    fn read_dir(&self, dir: &Path, scope: &str, cwd: Option<&Path>) -> Vec<Value> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return vec![];
        };
        let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|e| e.file_name());
        let mut result = Vec::new();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.to_lowercase().ends_with(".md") || entry.file_type().is_ok_and(|t| t.is_dir())
            {
                continue;
            }
            let Ok((path, data, body)) = self.read(&entry.path(), cwd) else {
                continue;
            };
            let id = slug(&name[..name.len() - 3]);
            let title = if string(&data, "title").is_empty() {
                &id
            } else {
                string(&data, "title")
            };
            let kind = kind(&data["kind"]);
            let mut item = json!({"id":id,"scope":scope,"title":title,"kind":kind,"editable":true,"body":body,"path":path});
            for key in ["description", "tags"] {
                if let Some(value) = data.get(key).filter(|v| !v.is_null()) {
                    item[key] = value.clone();
                }
            }
            if matches!(string(&data, "action"), "insert" | "spawn" | "copy") {
                item["action"] = data["action"].clone();
            }
            if kind == "mcp" {
                if let Some(mcp) = clean_mcp(&data["mcp"]) {
                    item["mcp"] = mcp;
                }
            }
            if kind == "dispatch" {
                item["params"] = json!(dispatch_templates::parameters(&body));
                if data["resultSchema"].is_object() {
                    item["resultSchema"] = data["resultSchema"].clone();
                }
            }
            result.push(item);
        }
        result
    }
    fn claude(&self, cwd: &Path) -> Vec<Value> {
        let mut result = Vec::new();
        for (directory, kind) in [
            ("skills", "skill"),
            ("agents", "agent"),
            ("commands", "command"),
        ] {
            let Ok(entries) = std::fs::read_dir(cwd.join(".claude").join(directory)) else {
                continue;
            };
            let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                let (path, id) = if kind == "skill" {
                    if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                        continue;
                    }
                    (entry.path().join("SKILL.md"), name)
                } else {
                    if !name.to_lowercase().ends_with(".md") {
                        continue;
                    }
                    (entry.path(), name[..name.len() - 3].into())
                };
                if let Ok((path, data, body)) = self.read(&path, Some(cwd)) {
                    result.push(json!({"id":id,"scope":"claude","kind":kind,"title":if string(&data,"name").is_empty(){&id}else{string(&data,"name")},"description":string(&data,"description"),"origin":"project","editable":true,"body":body,"path":path}));
                }
            }
        }
        result
    }
    fn seed(&self) -> Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let _lock = ConfigLock::take(&self.directory.join("library-seeded.json"))?;
        let marker = self.directory.join("library-seeded.json");
        let existing = std::fs::read(&marker)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(|v| {
                v["seeded"]
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string))
            });
        let mut seeded: BTreeSet<String> = existing
            .as_ref()
            .and_then(|v| v["seeded"].as_array())
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if existing.is_none()
            && std::fs::read_dir(self.global()).is_ok_and(|e| {
                e.filter_map(Result::ok).any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .ends_with(".md")
                })
            })
        {
            seeded.extend(
                [
                    "summarize-and-plan",
                    "careful-refactor",
                    "context7-mcp",
                    "make-workspacer-plugin",
                ]
                .map(str::to_owned),
            );
        }
        let before = seeded.clone();
        let starters: Vec<Value> =
            serde_json::from_str(include_str!("../../assets/library-starters.json"))?;
        std::fs::create_dir_all(self.global())?;
        for starter in starters {
            let id = string(&starter, "id");
            if seeded.contains(id) {
                continue;
            }
            let path = self.guard(&self.global().join(format!("{id}.md")), None, true)?;
            if std::fs::symlink_metadata(&path).is_err() {
                let mut item = starter["item"].clone();
                let body = string(&item, "body").to_owned();
                item.as_object_mut().unwrap().remove("body");
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                file.write_all(serialize(&item, &body)?.as_bytes())?;
                file.sync_all()?;
            }
            seeded.insert(id.into());
        }
        if seeded != before || existing.is_none() {
            atomic_json(&marker, &json!({"seeded":seeded}), false)?;
        }
        Ok(())
    }
    pub fn list(&self, params: &Value) -> Result<Value> {
        validate(params)?;
        let filter = string(params, "kind");
        if !filter.is_empty() && !KINDS.contains(&filter) {
            bail!("library.list: unknown kind {filter:?}");
        }
        let cwd = if string(params, "cwd").is_empty() {
            None
        } else {
            Some(paths::canonicalize(Path::new(string(params, "cwd")))?)
        };
        self.seed()?;
        let mut items = BTreeMap::new();
        for item in self.read_dir(&self.global(), "global", cwd.as_deref()) {
            items.insert(string(&item, "id").to_owned(), item);
        }
        if let Some(cwd) = &cwd {
            for item in self.read_dir(&cwd.join(".workspacer/library"), "project", Some(cwd)) {
                items.insert(string(&item, "id").to_owned(), item);
            }
            for item in self.claude(cwd) {
                items.insert(
                    format!("claude:{}:{}", string(&item, "kind"), string(&item, "id")),
                    item,
                );
            }
        }
        let mut items = items
            .into_values()
            .filter(|i| {
                (filter.is_empty() || i["kind"] == filter)
                    && (string(params, "id").is_empty() || i["id"] == params["id"])
            })
            .map(redact)
            .collect::<Vec<_>>();
        items.sort_by(|a, b| string(a, "title").cmp(string(b, "title")));
        Ok(json!(items))
    }
    fn destination(&self, input: &Value) -> Result<(PathBuf, String, Option<PathBuf>, String)> {
        let scope = string(input, "scope");
        if !matches!(scope, "global" | "project" | "claude") {
            bail!("invalid library scope");
        }
        if scope == "claude" && string(input, "origin").starts_with("plugin:") {
            bail!("plugin library items are read-only — copy into the project to edit");
        }
        if scope == "claude" && !matches!(string(input, "origin"), "" | "project") {
            bail!("library origin is outside the selected project");
        }
        let cwd = if scope == "global" {
            None
        } else {
            Some(paths::canonicalize(&if string(input, "cwd").is_empty() {
                std::env::current_dir()?
            } else {
                PathBuf::from(string(input, "cwd"))
            })?)
        };
        let id = if scope == "claude" && !string(input, "id").is_empty() {
            basename(string(input, "id"))?.into()
        } else {
            slug(if string(input, "id").is_empty() {
                string(input, "title")
            } else {
                string(input, "id")
            })
        };
        let kind = if scope == "claude" {
            match string(input, "kind") {
                "agent" => "agent",
                "command" => "command",
                _ => "skill",
            }
        } else {
            kind(&input["kind"])
        }
        .to_owned();
        let path = match (scope, kind.as_str()) {
            ("global", _) => self.global().join(format!("{id}.md")),
            ("project", _) => cwd
                .as_ref()
                .unwrap()
                .join(".workspacer/library")
                .join(format!("{id}.md")),
            ("claude", "skill") => cwd
                .as_ref()
                .unwrap()
                .join(".claude/skills")
                .join(&id)
                .join("SKILL.md"),
            ("claude", "agent") => cwd
                .as_ref()
                .unwrap()
                .join(".claude/agents")
                .join(format!("{id}.md")),
            _ => cwd
                .as_ref()
                .unwrap()
                .join(".claude/commands")
                .join(format!("{id}.md")),
        };
        Ok((self.guard(&path, cwd.as_deref(), true)?, id, cwd, kind))
    }
    pub fn save(&self, input: &Value) -> Result<Value> {
        validate(input)?;
        let (path, id, cwd, kind) = self.destination(input)?;
        let _lock = ConfigLock::take(&path)?;
        let old = self
            .read(&path, cwd.as_deref())
            .ok()
            .map(|(_, m, _)| m)
            .unwrap_or(json!({}));
        let mut meta = if input["scope"] == "claude" {
            old.clone()
        } else {
            json!({})
        };
        let title = string(input, "title");
        let body = string(input, "body");
        meta[if input["scope"] == "claude" {
            "name"
        } else {
            "title"
        }] = json!(title);
        if !string(input, "description").is_empty() {
            meta["description"] = input["description"].clone();
        } else {
            meta.as_object_mut().unwrap().remove("description");
        }
        if input["scope"] != "claude" {
            meta["kind"] = json!(kind);
            for key in ["tags", "action"] {
                if let Some(value) = input.get(key).filter(|v| !v.is_null()) {
                    meta[key] = value.clone();
                }
            }
            if kind == "dispatch" && input["resultSchema"].is_object() {
                meta["resultSchema"] = input["resultSchema"].clone();
            }
            if kind == "mcp" {
                if let Some(mcp) = clean_mcp(&input["mcp"]) {
                    meta["mcp"] = restore(mcp, &old["mcp"]);
                }
            }
        }
        atomic_bytes(&path, serialize(&meta, body)?.as_bytes())?;
        let mut result = json!({"id":id,"scope":input["scope"],"title":title,"kind":kind,"editable":true,"body":body,"path":path});
        for key in ["description", "tags", "action", "mcp", "resultSchema"] {
            if let Some(value) = meta.get(key) {
                result[key] = value.clone();
            }
        }
        if kind == "dispatch" {
            result["params"] = json!(dispatch_templates::parameters(body));
        }
        if input["scope"] == "claude" {
            result["origin"] = json!("project");
        }
        Ok(redact(result))
    }
    pub fn remove(&self, input: &Value) -> Result<Value> {
        validate(input)?;
        if string(input, "id").is_empty() {
            bail!("library.remove requires scope and id");
        }
        let (path, _, cwd, kind) = self.destination(input)?;
        let path = if input["scope"] == "claude" && kind == "skill" {
            self.guard(path.parent().unwrap(), cwd.as_deref(), false)?
        } else {
            path
        };
        let result = if input["scope"] == "claude" && kind == "skill" {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        Ok(json!({"ok":true}))
    }
    pub fn selected_mcp(&self, cwd: &Path, ids: &[String]) -> Result<BTreeMap<String, Value>> {
        let cwd = paths::canonicalize(cwd)?;
        let wanted: BTreeSet<_> = ids
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && *s != "workspacer")
            .collect();
        let mut selected = BTreeMap::new();
        for item in self
            .read_dir(&self.global(), "global", Some(&cwd))
            .into_iter()
            .chain(self.read_dir(&cwd.join(".workspacer/library"), "project", Some(&cwd)))
        {
            let id = string(&item, "id");
            if wanted.contains(id) && item["kind"] == "mcp" && !item["mcp"].is_null() {
                selected.insert(id.into(), item["mcp"].clone());
            }
        }
        Ok(selected)
    }
}
pub(crate) fn install(
    mut options: crate::Options,
    directory: PathBuf,
    hub: crate::Handle,
) -> crate::Options {
    let service = std::sync::Arc::new(Library::new(directory));
    options.library_watcher = Some(super::library_watch::Watcher::new(
        service.clone(),
        options.session_snapshots.clone(),
        options.engine.is_some(),
    ));
    for method in ["library.list", "library.save", "library.remove"] {
        let service = service.clone();
        let hub = hub.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            let hub = hub.clone();
            async move {
                let result = tokio::task::spawn_blocking(move || match method {
                    "library.list" => service.list(&params),
                    "library.save" => service.save(&params),
                    _ => service.remove(&params),
                })
                .await??;
                if method != "library.list" {
                    if let Err(error) = hub
                        .publish_wait(crate::protocol::Event::new(
                            "library.changed",
                            "brain",
                            json!({}),
                        ))
                        .await
                    {
                        eprintln!(
                            "library mutation committed but change notification failed: {error}"
                        );
                    }
                }
                Ok(result)
            }
        });
    }
    options
}

#[cfg(test)]
#[path = "../../tests/support/sweepguard.rs"]
mod sweepguard;

#[cfg(test)]
mod selected_directory_contract {
    use super::*;
    fn cases() -> Vec<Value> {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../../contracts/path-containment-cases.json"
        ))
        .unwrap();
        corpus["libraryItemDirs"]["cases"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn run_cases(
        rows: &[Value],
        create_link: impl Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
    ) -> sweepguard::Tally {
        let mut tally = sweepguard::Tally::default();
        'case: for row in rows {
            let dir = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(dir.path()).unwrap();
            for sub in ["home", "config/workspacer/library", "outside"] {
                std::fs::create_dir_all(root.join(sub)).unwrap();
            }
            for sub in row["tree"]["dirs"].as_array().into_iter().flatten() {
                std::fs::create_dir_all(root.join(sub.as_str().unwrap())).unwrap();
            }
            for (name, text) in row["tree"]["files"].as_object().into_iter().flatten() {
                let path = root.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text.as_str().unwrap()).unwrap();
            }
            for (name, target) in row["tree"]["symlinks"].as_object().into_iter().flatten() {
                let link = root.join(name);
                std::fs::create_dir_all(link.parent().unwrap()).unwrap();
                if let Err(error) = create_link(&root.join(target.as_str().unwrap()), &link) {
                    tally.skip(&format!("{}: needsSymlinks ({error})", row["name"]));
                    continue 'case;
                }
            }
            let service = Library::new(root.join("config/workspacer"));
            let cwd = root.join(row["cwd"].as_str().unwrap());
            // The current implementation combines the historical root/dir gates;
            // assert the real semantic refusal, never manufacture old layer names.
            let result =
                service.guard(&root.join(row["item"].as_str().unwrap()), Some(&cwd), false);
            tally.ran(row["expect"].as_str().unwrap());
            if row["expect"] == "accept" {
                assert_eq!(
                    result.unwrap(),
                    root.join(row["resolvesTo"].as_str().unwrap()),
                    "{}",
                    row["name"]
                );
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("outside the selected library directories"),
                    "{}",
                    row["name"]
                );
            }
        }
        tally
    }
    fn create_link(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "directory symlinks unavailable",
            ))
        }
    }
    #[test]
    fn selected_library_item_directories_match_corpus() {
        run_cases(&cases(), create_link)
            .require_corpus("selected library directories", 7, 3, 4)
            .unwrap();
    }
    #[test]
    fn unavailable_symlink_privilege_cannot_make_library_corpus_green() {
        let tally = run_cases(&cases(), |_, _| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "simulated host without symlink privilege",
            ))
        });
        assert_eq!(
            (tally.enumerated(), tally.executed(), tally.skipped),
            (7, 5, 2)
        );
        let error = tally
            .require_corpus("selected library directories", 7, 3, 4)
            .unwrap_err();
        for expected in [
            "2 deny cases",
            "needsSymlinks",
            "simulated host",
            "2 case(s) skipped",
        ] {
            assert!(error.contains(expected), "{error}");
        }
    }
}

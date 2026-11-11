//! Project groups: named sets of folders ("Acme" = api + web + infra).
//!
//! Design: `docs/features/projects.md`. Groups live in their own hub-owned file,
//! `<config_dir>/project-groups.json`, and NOT inside `config.yaml`: both config
//! writers deep-merge saves, so a client sending back a stale copy would
//! resurrect a deleted group or undo a rename. Only these methods write the
//! file, under the config lock and a revision compare-and-set:
//!
//! ```text
//! {"version":1,"revision":7,"groups":{"p_3f9c2a1b7d":{"name":"Acme","folders":[...],
//!   "color":"#5b8def","icon":"🚀","createdAt":"...","updatedAt":"..."}}}
//! ```
//!
//! A folder belongs to at most one group. Folders are membership labels only:
//! they are normalised like the native `project_key` (forward slashes, no
//! trailing slash) and compared, never opened, joined or canonicalized.
use super::config::ConfigLock;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub const FILE: &str = "project-groups.json";
const VERSION: u64 = 1;
const MAX_NAME: usize = 80;
const MAX_FOLDERS: usize = 32;
const MAX_FOLDER: usize = 1024;
const MAX_ICON: usize = 8;

pub struct ProjectGroups {
    path: PathBuf,
}

struct Store {
    revision: u64,
    groups: Map<String, Value>,
}

impl ProjectGroups {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn list(&self, params: &Value) -> Result<Value> {
        keys(params, &[])?;
        let store = read_store(&self.path)?;
        Ok(json!({"revision": store.revision, "groups": sorted(&store.groups)}))
    }

    /// Create (no `id`) or replace one group. Returns the saved group plus the
    /// new revision; `color`/`icon` left out are cleared, like the other fields
    /// a save replaces.
    pub fn save(&self, params: &Value) -> Result<(Value, u64)> {
        keys(
            params,
            &["id", "name", "folders", "color", "icon", "revision"],
        )?;
        let id = string(params, "id")?;
        let name = name(params)?;
        let folders = folders(params)?;
        let color = color(params)?;
        let icon = icon(params)?;
        let expected = revision(params)?;

        let _lock = self.lock()?;
        let mut store = read_store(&self.path)?;
        check_revision(&store, expected)?;
        if let Some(id) = id
            && !store.groups.contains_key(id)
        {
            bail!("there is no project {id}; leave id out to create a new one");
        }
        for folder in &folders {
            if let Some((_, other)) = store.groups.iter().find(|(other_id, group)| {
                Some(other_id.as_str()) != id
                    && group["folders"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .any(|existing| same_folder(&normalise(existing), folder))
            }) {
                let other = other["name"].as_str().unwrap_or_default();
                bail!("{folder} is already in the project “{other}”; remove it there first");
            }
        }
        let id = match id {
            Some(id) => id.to_owned(),
            None => new_id(&store.groups)?,
        };
        let now = crate::protocol::now();
        let mut group = store
            .groups
            .get(&id)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        group.insert("name".into(), json!(name));
        group.insert("folders".into(), json!(folders));
        set_optional(&mut group, "color", color);
        set_optional(&mut group, "icon", icon);
        group.entry("createdAt").or_insert_with(|| json!(now));
        group.insert("updatedAt".into(), json!(now));
        store.groups.insert(id.clone(), Value::Object(group));
        store.revision += 1;
        write_store(&self.path, &store)?;

        let mut result = view(&id, &store.groups[&id]);
        result["revision"] = json!(store.revision);
        Ok((result, store.revision))
    }

    /// Remove one group. Its folders and their settings are
    /// untouched: they simply become one-folder projects again.
    pub fn remove(&self, params: &Value) -> Result<(Value, u64)> {
        keys(params, &["id", "revision"])?;
        let id = string(params, "id")?
            .filter(|id| !id.is_empty())
            .context("id is required: the project to remove")?;
        let expected = revision(params)?;
        let _lock = self.lock()?;
        let mut store = read_store(&self.path)?;
        check_revision(&store, expected)?;
        if store.groups.remove(id).is_none() {
            bail!("there is no project {id}");
        }
        store.revision += 1;
        write_store(&self.path, &store)?;
        Ok((
            json!({"removed": true, "revision": store.revision}),
            store.revision,
        ))
    }

    fn lock(&self) -> Result<ConfigLock> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        ConfigLock::take(&self.path)
    }
}

// ---- store ----------------------------------------------------------------

/// A missing file is the empty store. Anything else that does not parse is an
/// error, and because every write reads first, a damaged file is never
/// overwritten.
fn read_store(path: &Path) -> Result<Store> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Store {
                revision: 0,
                groups: Map::new(),
            });
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let damaged = |why: &str| anyhow::anyhow!("{} {why}; fix or move it aside", path.display());
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| damaged("is not valid JSON"))?;
    if value["version"].as_u64() != Some(VERSION) {
        return Err(damaged("is not a version 1 project groups file"));
    }
    let revision = value["revision"]
        .as_u64()
        .ok_or_else(|| damaged("has no revision number"))?;
    let groups = value["groups"]
        .as_object()
        .ok_or_else(|| damaged("has no groups object"))?;
    for group in groups.values() {
        let valid = group["name"].is_string()
            && group["folders"]
                .as_array()
                .is_some_and(|folders| folders.iter().all(Value::is_string));
        if !valid {
            return Err(damaged("has a group without a name and folder list"));
        }
    }
    Ok(Store {
        revision,
        groups: groups.clone(),
    })
}

fn write_store(path: &Path, store: &Store) -> Result<()> {
    super::atomic_json(
        path,
        &json!({"version": VERSION, "revision": store.revision, "groups": store.groups}),
        false,
    )
}

fn check_revision(store: &Store, expected: Option<u64>) -> Result<()> {
    match expected {
        Some(expected) if expected != store.revision => bail!(
            "conflict: project groups changed (now revision {}, this edit was made against {expected}); reload and try again",
            store.revision
        ),
        _ => Ok(()),
    }
}

fn new_id(groups: &Map<String, Value>) -> Result<String> {
    use rand::RngCore;
    loop {
        let mut bytes = [0u8; 5];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| anyhow::anyhow!("random source unavailable"))?;
        let id = format!(
            "p_{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        if !groups.contains_key(&id) {
            return Ok(id);
        }
    }
}

fn view(id: &str, group: &Value) -> Value {
    let field = |key: &str| group.get(key).cloned().unwrap_or(Value::Null);
    json!({
        "id": id,
        "name": field("name"),
        "folders": field("folders"),
        "color": field("color"),
        "icon": field("icon"),
        "createdAt": field("createdAt"),
        "updatedAt": field("updatedAt"),
    })
}

/// By name, case-insensitively, then by id so equal names order stably.
fn sorted(groups: &Map<String, Value>) -> Vec<Value> {
    let mut rows: Vec<(String, &String, &Value)> = groups
        .iter()
        .map(|(id, group)| {
            (
                group["name"].as_str().unwrap_or_default().to_lowercase(),
                id,
                group,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    rows.into_iter()
        .map(|(_, id, group)| view(id, group))
        .collect()
}

fn set_optional(group: &mut Map<String, Value>, key: &str, value: Option<String>) {
    match value {
        Some(value) => {
            group.insert(key.into(), json!(value));
        }
        None => {
            group.remove(key);
        }
    }
}

// ---- folders --------------------------------------------------------------

/// The native `project_key` spelling: trimmed, forward slashes, no trailing
/// slash (a lone `/` and a drive root `C:/` stay roots).
fn normalise(folder: &str) -> String {
    let key = folder.trim().replace('\\', "/");
    let trimmed = key.trim_end_matches('/');
    if trimmed.is_empty() {
        return if key.is_empty() { key } else { "/".into() };
    }
    if trimmed.len() == 2 && trimmed.ends_with(':') && key.len() > 2 {
        return format!("{trimmed}/");
    }
    trimmed.to_owned()
}

/// Drive-letter and UNC spellings name a case-insensitive filesystem.
fn windows_shaped(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with("//")
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

fn absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/')
}

/// Both sides already normalised. Exact on Linux POSIX paths; case-folded for
/// Windows-shaped paths and on the case-insensitive desktop filesystems.
fn same_folder(a: &str, b: &str) -> bool {
    a == b
        || ((windows_shaped(a) || cfg!(any(windows, target_os = "macos")))
            && a.to_lowercase() == b.to_lowercase())
}

fn folders(params: &Value) -> Result<Vec<String>> {
    let raw = match params.get("folders") {
        Some(Value::Array(raw)) => raw,
        None | Some(Value::Null) => bail!("folders is required: one or more absolute folder paths"),
        Some(_) => bail!("folders must be an array of absolute paths"),
    };
    if raw.len() > MAX_FOLDERS {
        bail!("a project holds at most {MAX_FOLDERS} folders");
    }
    let mut folders: Vec<String> = Vec::new();
    for folder in raw {
        let folder = folder
            .as_str()
            .context("folders must be an array of absolute paths")?;
        if folder.chars().count() > MAX_FOLDER || folder.chars().any(char::is_control) {
            bail!("folder paths are at most {MAX_FOLDER} characters with no control characters");
        }
        let folder = normalise(folder);
        if !absolute(&folder) {
            bail!("folder {folder:?} is not an absolute path");
        }
        if folder.split('/').any(|part| part == "." || part == "..") {
            bail!("folder {folder} must not contain '.' or '..' components");
        }
        if !folders.iter().any(|seen| same_folder(seen, &folder)) {
            folders.push(folder);
        }
    }
    if folders.is_empty() {
        bail!("folders is required: one or more absolute folder paths");
    }
    Ok(folders)
}

// ---- parameters -----------------------------------------------------------

/// Unknown keys are refused, and so is a key that differs from a known one only
/// by case: the checked value must be the used value.
fn keys(params: &Value, allowed: &[&str]) -> Result<()> {
    let Some(object) = params.as_object() else {
        if params.is_null() {
            return Ok(());
        }
        bail!("projectGroups parameters must be an object");
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            if allowed.iter().any(|a| a.eq_ignore_ascii_case(key)) {
                bail!("parameter {key} differs from a known parameter only by case");
            }
            bail!("unknown parameter {key}");
        }
    }
    Ok(())
}
fn string<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => bail!("{key} must be a string"),
    }
}
fn revision(params: &Value) -> Result<Option<u64>> {
    match params.get("revision") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .context("revision must be a non-negative integer"),
    }
}
fn name(params: &Value) -> Result<String> {
    let name = string(params, "name")?.unwrap_or_default().trim();
    if name.is_empty() {
        bail!("name is required");
    }
    if name.chars().count() > MAX_NAME {
        bail!("name is at most {MAX_NAME} characters");
    }
    if name.chars().any(char::is_control) {
        bail!("name contains control characters");
    }
    Ok(name.to_owned())
}
fn color(params: &Value) -> Result<Option<String>> {
    let Some(color) = string(params, "color")?
        .map(str::trim)
        .filter(|c| !c.is_empty())
    else {
        return Ok(None);
    };
    let hex = color.strip_prefix('#').unwrap_or_default();
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("color must be #rrggbb");
    }
    Ok(Some(color.to_ascii_lowercase()))
}
fn icon(params: &Value) -> Result<Option<String>> {
    let Some(icon) = string(params, "icon")?
        .map(str::trim)
        .filter(|i| !i.is_empty())
    else {
        return Ok(None);
    };
    if icon.chars().count() > MAX_ICON || icon.chars().any(char::is_control) {
        bail!("icon is at most {MAX_ICON} characters with no control characters");
    }
    Ok(Some(icon.to_owned()))
}

// ---- bus ------------------------------------------------------------------

pub(crate) fn install(
    mut options: crate::Options,
    config_dir: PathBuf,
    hub: crate::Handle,
) -> crate::Options {
    let service = Arc::new(ProjectGroups::new(config_dir.join(FILE)));
    for method in [
        "projectGroups.list",
        "projectGroups.save",
        "projectGroups.remove",
    ] {
        let service = service.clone();
        let hub = hub.clone();
        options = options.handler(method, move |_caller, params| {
            let service = service.clone();
            let hub = hub.clone();
            async move {
                let (result, changed) = tokio::task::spawn_blocking(move || match method {
                    "projectGroups.list" => service.list(&params).map(|r| (r, None)),
                    "projectGroups.save" => service.save(&params).map(|(r, n)| (r, Some(n))),
                    _ => service.remove(&params).map(|(r, n)| (r, Some(n))),
                })
                .await??;
                if let Some(revision) = changed
                    && let Err(error) = hub
                        .publish_wait(crate::protocol::Event::new(
                            "projectGroups.changed",
                            "brain",
                            json!({"revision": revision}),
                        ))
                        .await
                {
                    eprintln!("project groups change committed but notification failed: {error}");
                }
                Ok(result)
            }
        });
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        path: PathBuf,
        groups: ProjectGroups,
    }
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config").join(FILE);
        Fixture {
            groups: ProjectGroups::new(path.clone()),
            path,
            _tmp: tmp,
        }
    }
    impl Fixture {
        fn save(&self, params: Value) -> Result<Value> {
            self.groups.save(&params).map(|(r, _)| r)
        }
        fn list(&self) -> Value {
            self.groups.list(&json!({})).unwrap()
        }
    }

    #[test]
    fn a_missing_file_lists_empty_and_creates_nothing() {
        let f = fixture();
        assert_eq!(f.list(), json!({"revision": 0, "groups": []}));
        assert!(!f.path.exists());
        assert_eq!(f.groups.list(&Value::Null).unwrap()["revision"], 0);
    }

    #[test]
    fn create_list_update_and_rename() {
        let f = fixture();
        let acme = f
            .save(json!({"name": "  Acme ", "folders": ["/code/acme-api", "/code/acme-web/"], "color": "#5B8DEF", "icon": "🚀"}))
            .unwrap();
        let id = acme["id"].as_str().unwrap().to_owned();
        assert_eq!(id.len(), 12);
        assert!(id.starts_with("p_"));
        assert!(
            id[2..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_eq!(acme["name"], "Acme");
        assert_eq!(acme["folders"], json!(["/code/acme-api", "/code/acme-web"]));
        assert_eq!(acme["color"], "#5b8def");
        assert_eq!(acme["icon"], "🚀");
        assert_eq!(acme["revision"], 1);
        assert!(acme["createdAt"].is_string());

        f.save(json!({"name": "beta", "folders": ["/code/beta"]}))
            .unwrap();
        let list = f.list();
        assert_eq!(list["revision"], 2);
        let names: Vec<_> = list["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            ["Acme", "beta"],
            "sorted by name, case-insensitively"
        );
        assert_eq!(list["groups"][1]["color"], Value::Null);

        let renamed = f
            .save(json!({"id": id, "name": "Zeta", "folders": ["/code/acme-api"], "revision": 2}))
            .unwrap();
        assert_eq!(renamed["id"], id.as_str());
        assert_eq!(renamed["revision"], 3);
        assert_eq!(renamed["createdAt"], acme["createdAt"]);
        assert_eq!(renamed["color"], Value::Null, "a save replaces the group");
        let list = f.list();
        assert_eq!(list["groups"][1]["name"], "Zeta");
        assert_eq!(list["groups"][1]["folders"], json!(["/code/acme-api"]));
        assert_eq!(list["groups"].as_array().unwrap().len(), 2);

        let stored: Value = serde_json::from_slice(&std::fs::read(&f.path).unwrap()).unwrap();
        assert_eq!(stored["version"], 1);
        assert_eq!(stored["revision"], 3);
        assert_eq!(stored["groups"][&id]["name"], "Zeta");
    }

    #[test]
    fn folders_are_normalised_deduplicated_and_must_be_absolute() {
        let f = fixture();
        let saved = f
            .save(json!({"name": "Win", "folders": [" C:\\code\\api\\ ", "C:/code/api", "/", "C:\\", "//server/share/"]}))
            .unwrap();
        assert_eq!(
            saved["folders"],
            json!(["C:/code/api", "/", "C:/", "//server/share"])
        );
        let upper = f
            .save(json!({"name": "Dup", "folders": ["c:/CODE/api"]}))
            .unwrap_err()
            .to_string();
        assert!(
            upper.contains("“Win”"),
            "drive paths compare case-insensitively: {upper}"
        );
        for bad in ["code/api", "C:code", "~/code", "/code/../etc", ""] {
            let error = f
                .save(json!({"name": "Bad", "folders": [bad]}))
                .unwrap_err()
                .to_string();
            assert!(error.contains("folder"), "{bad:?}: {error}");
        }
        let none = f
            .save(json!({"name": "Bad", "folders": []}))
            .unwrap_err()
            .to_string();
        assert!(none.contains("folders is required"), "{none}");
        let many: Vec<String> = (0..33).map(|i| format!("/code/{i}")).collect();
        let too_many = f
            .save(json!({"name": "Big", "folders": many}))
            .unwrap_err()
            .to_string();
        assert!(too_many.contains("at most 32"), "{too_many}");
    }

    #[test]
    fn a_folder_in_another_group_is_refused_with_that_groups_name() {
        let f = fixture();
        let acme = f
            .save(json!({"name": "Acme", "folders": ["/x/api"]}))
            .unwrap();
        let error = f
            .save(json!({"name": "Other", "folders": ["/x/web", "/x/api/"]}))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "/x/api is already in the project “Acme”; remove it there first"
        );
        // The group's own folders are not a conflict when it is re-saved, and a
        // nested folder is a different folder (longest prefix wins).
        f.save(json!({"id": acme["id"], "name": "Acme", "folders": ["/x/api", "/x/web"]}))
            .unwrap();
        f.save(json!({"name": "Nested", "folders": ["/x/api/sub"]}))
            .unwrap();
        assert_eq!(f.list()["revision"], 3);
    }

    #[test]
    fn a_stale_revision_is_a_conflict_and_changes_nothing() {
        let f = fixture();
        let acme = f
            .save(json!({"name": "Acme", "folders": ["/a"], "revision": 0}))
            .unwrap();
        let stale = f
            .save(json!({"id": acme["id"], "name": "Renamed", "folders": ["/a"], "revision": 0}))
            .unwrap_err()
            .to_string();
        assert!(stale.starts_with("conflict:"), "{stale}");
        let stale_remove = f
            .groups
            .remove(&json!({"id": acme["id"], "revision": 5}))
            .unwrap_err()
            .to_string();
        assert!(stale_remove.starts_with("conflict:"), "{stale_remove}");
        let list = f.list();
        assert_eq!(list["revision"], 1);
        assert_eq!(list["groups"][0]["name"], "Acme");
        let bad = f
            .save(json!({"name": "A", "folders": ["/b"], "revision": "1"}))
            .unwrap_err()
            .to_string();
        assert!(bad.contains("revision must be"), "{bad}");
    }

    #[test]
    fn remove_deletes_only_the_group() {
        let f = fixture();
        let acme = f.save(json!({"name": "Acme", "folders": ["/a"]})).unwrap();
        f.save(json!({"name": "Beta", "folders": ["/b"]})).unwrap();
        let (removed, revision) = f
            .groups
            .remove(&json!({"id": acme["id"], "revision": 2}))
            .unwrap();
        assert_eq!(removed, json!({"removed": true, "revision": 3}));
        assert_eq!(revision, 3);
        let list = f.list();
        assert_eq!(list["groups"].as_array().unwrap().len(), 1);
        assert_eq!(list["groups"][0]["name"], "Beta");
        let again = f
            .groups
            .remove(&json!({"id": acme["id"]}))
            .unwrap_err()
            .to_string();
        assert!(again.contains("no project"), "{again}");
        let missing = f.groups.remove(&json!({})).unwrap_err().to_string();
        assert!(missing.contains("id is required"), "{missing}");
        // Its folder is free again.
        f.save(json!({"name": "New", "folders": ["/a"]})).unwrap();
        let unknown = f
            .save(json!({"id": "p_0000000000", "name": "X", "folders": ["/z"]}))
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("no project p_0000000000"), "{unknown}");
    }

    #[test]
    fn a_malformed_file_is_reported_and_never_overwritten() {
        for body in [
            "{not json",
            r#"{"version":2,"revision":1,"groups":{}}"#,
            r#"{"version":1,"groups":{}}"#,
            r#"{"version":1,"revision":1,"groups":[]}"#,
            r#"{"version":1,"revision":1,"groups":{"p_1":{"name":"A","folders":"/a"}}}"#,
        ] {
            let f = fixture();
            std::fs::create_dir_all(f.path.parent().unwrap()).unwrap();
            std::fs::write(&f.path, body).unwrap();
            let list = f.groups.list(&json!({})).unwrap_err().to_string();
            assert!(list.contains(FILE), "{body}: {list}");
            assert!(
                f.save(json!({"name": "A", "folders": ["/a"]})).is_err(),
                "{body}"
            );
            assert!(f.groups.remove(&json!({"id": "p_1"})).is_err(), "{body}");
            assert_eq!(std::fs::read_to_string(&f.path).unwrap(), body);
        }
    }

    #[test]
    fn parameters_are_strict() {
        let f = fixture();
        let unknown = f
            .save(json!({"name": "A", "folders": ["/a"], "owner": "me"}))
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("unknown parameter owner"), "{unknown}");
        let case = f
            .save(json!({"name": "A", "folders": ["/a"], "Folders": ["/b"]}))
            .unwrap_err()
            .to_string();
        assert!(case.contains("only by case"), "{case}");
        let list = f
            .groups
            .list(&json!({"all": true}))
            .unwrap_err()
            .to_string();
        assert!(list.contains("unknown parameter"), "{list}");
        let remove = f
            .groups
            .remove(&json!({"ID": "p_1"}))
            .unwrap_err()
            .to_string();
        assert!(remove.contains("only by case"), "{remove}");
        for (params, expect) in [
            (json!({"name": " ", "folders": ["/a"]}), "name is required"),
            (
                json!({"name": "a".repeat(81), "folders": ["/a"]}),
                "at most 80",
            ),
            (
                json!({"name": "a\u{7}b", "folders": ["/a"]}),
                "control characters",
            ),
            (
                json!({"name": "A", "folders": ["/a"], "color": "blue"}),
                "#rrggbb",
            ),
            (
                json!({"name": "A", "folders": ["/a"], "color": "#12345g"}),
                "#rrggbb",
            ),
            (
                json!({"name": "A", "folders": ["/a"], "icon": "123456789"}),
                "icon is at most",
            ),
            (json!({"name": "A", "folders": "/a"}), "array"),
            (json!({"name": "A", "folders": [1]}), "array"),
            (
                json!({"name": 1, "folders": ["/a"]}),
                "name must be a string",
            ),
        ] {
            let error = f.save(params.clone()).unwrap_err().to_string();
            assert!(error.contains(expect), "{params}: {error}");
        }
        assert!(!f.path.exists(), "no refused save wrote the file");
        let eight = f
            .save(json!({"name": "A", "folders": ["/a"], "icon": "12345678", "color": ""}))
            .unwrap();
        assert_eq!(eight["icon"], "12345678");
        assert_eq!(eight["color"], Value::Null);
    }
}

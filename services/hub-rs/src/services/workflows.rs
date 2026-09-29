//! Strict workflow definitions and immutable task pins. Definition changes do
//! not rewrite an existing task's pinned template, policy, or result contract.
use super::{
    atomic_json,
    config::{Config, SelectionConflict},
    library::Library,
};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
pub const DEFAULT_ID: &str = "implement-review";
const ROLES: &[&str] = &[
    "mechanical",
    "complex_fixer",
    "scout",
    "diagnostician",
    "implementer",
    "reviewer",
    "deep_reviewer",
    "fixer",
    "validator",
    "judge",
    "supervisor",
];
const KINDS: &[&str] = &[
    "research",
    "implement",
    "review",
    "repair",
    "validate",
    "land",
];
const STAGES: &[&str] = &[
    "scout",
    "implement",
    "review",
    "fix",
    "validate",
    "land",
    "other",
];
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let map = value
        .as_object()
        .ok_or_else(|| anyhow!("Expected an object"))?;
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!("Unknown workflow field: {key}");
        }
    }
    Ok(())
}
fn bounded_text(value: &Value, max: usize, empty: bool) -> Result<()> {
    if !value.as_str().is_some_and(|s| {
        s.encode_utf16().count() <= max
            && (empty || !super::dispatch_templates::trim_js(s).is_empty())
    }) {
        bail!(
            "Expected {} text, at most {max} characters",
            if empty { "optional" } else { "nonempty" }
        );
    }
    Ok(())
}
fn slug(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 64
            && s.as_bytes()[0].is_ascii_lowercase()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}
pub fn validate_definition(value: &Value) -> Result<Value> {
    keys(
        value,
        &["id", "revision", "name", "description", "enabled", "steps"],
    )?;
    if !slug(&value["id"])
        || !value["revision"]
            .as_u64()
            .is_some_and(|r| (1..=9_007_199_254_740_991).contains(&r))
    {
        bail!("Invalid workflow id/revision");
    }
    bounded_text(&value["name"], 120, false)?;
    bounded_text(&value["description"], 1000, true)?;
    let steps = value["steps"]
        .as_array()
        .filter(|s| (1..=8).contains(&s.len()))
        .ok_or_else(|| anyhow!("Workflow needs enabled and 1–8 steps"))?;
    if !value["enabled"].is_boolean() {
        bail!("Workflow needs enabled and 1–8 steps");
    }
    let mut seen = BTreeMap::new();
    let mut repairs = BTreeSet::new();
    for step in steps {
        keys(
            step,
            &[
                "id",
                "label",
                "kind",
                "stage",
                "role",
                "when",
                "template",
                "instructions",
                "independentOf",
                "repairOf",
            ],
        )?;
        if !slug(&step["id"]) || seen.contains_key(text(step, "id")) {
            bail!("Step ids must be unique slugs");
        }
        bounded_text(&step["label"], 120, false)?;
        bounded_text(&step["instructions"], 8000, true)?;
        bounded_text(&step["template"], 128, false)?;
        if !KINDS.contains(&text(step, "kind"))
            || !STAGES.contains(&text(step, "stage"))
            || !ROLES.contains(&text(step, "role"))
            || !matches!(text(step, "when"), "always" | "material_risk")
        {
            bail!("Unknown step kind, stage, role or condition");
        }
        if step["kind"] == "review" && step["when"] != "always" {
            bail!("A configured independent review is required");
        }
        for key in ["independentOf", "repairOf"] {
            if step
                .get(key)
                .is_some_and(|v| !v.is_null() && !v.is_string())
            {
                bail!("{key} must name an earlier step");
            }
        }
        let independent = text(step, "independentOf");
        if !independent.is_empty()
            && (step["kind"] != "review"
                || !seen
                    .get(independent)
                    .is_some_and(|kind| matches!(*kind, "implement" | "repair")))
        {
            bail!("Independent review must reference an earlier implementation/repair");
        }
        let repair = text(step, "repairOf");
        if !repair.is_empty() {
            if step["kind"] != "repair"
                || seen.get(repair) != Some(&"review")
                || !repairs.insert(repair.to_owned())
            {
                bail!("Repair must reference one earlier review, at most once per review");
            }
        }
        seen.insert(text(step, "id").to_owned(), text(step, "kind"));
    }
    if serde_json::to_string(value)?.encode_utf16().count() > 64 * 1024 {
        bail!("Workflow exceeds 64 KiB");
    }
    Ok(value.clone())
}
pub fn selections(config: &Value) -> Value {
    let projects = config["projects"]
        .as_object()
        .into_iter()
        .flat_map(|p| p.iter())
        .filter_map(|(cwd, p)| p.get("workflowId").map(|id| (cwd.clone(), id.clone())))
        .collect::<serde_json::Map<_, _>>();
    json!({"defaultId":config["agents"]["defaultWorkflowId"].as_str().unwrap_or(DEFAULT_ID),"selectionRevision":config["agents"]["workflowSelectionRevision"].as_u64().unwrap_or(0),"projects":projects})
}
#[derive(Debug)]
pub struct WorkflowConflict {
    pub current_revision: u64,
}
impl std::fmt::Display for WorkflowConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Workflow changed; reload revision {} before saving",
            self.current_revision
        )
    }
}
impl std::error::Error for WorkflowConflict {}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u64,
    seeded: Vec<String>,
    definitions: Vec<Value>,
}
pub struct WorkflowStore {
    path: PathBuf,
    library: Library,
    config: Arc<Config>,
}
impl WorkflowStore {
    pub fn new(directory: PathBuf, config: Arc<Config>) -> Self {
        Self {
            path: directory.join("workflow-definitions.json"),
            library: Library::new(directory),
            config,
        }
    }
    fn starters() -> Vec<Value> {
        serde_json::from_str(include_str!("../../assets/workflow-starters.json"))
            .expect("generated workflow starter definitions")
    }
    fn read(&self) -> Result<Document> {
        let mut document: Document = match std::fs::read(&self.path) {
            Ok(bytes) => {
                if bytes.len() > 2 * 1024 * 1024 {
                    bail!("Workflow store exceeds 2 MiB");
                }
                serde_json::from_slice(&bytes)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Document {
                version: 1,
                seeded: vec![],
                definitions: vec![],
            },
            Err(error) => return Err(error.into()),
        };
        if document.version != 1 || document.definitions.len() > 100 {
            bail!("Invalid workflow store/version");
        }
        let mut ids = BTreeSet::new();
        for definition in &document.definitions {
            validate_definition(definition)?;
            if !ids.insert(text(definition, "id")) {
                bail!("Duplicate workflow ids");
            }
        }
        for definition in Self::starters() {
            let id = text(&definition, "id").to_owned();
            if !document.seeded.contains(&id) {
                if !document.definitions.iter().any(|d| d["id"] == id) {
                    document.definitions.push(definition);
                }
                document.seeded.push(id);
            }
        }
        Ok(document)
    }
    fn transaction<T>(&self, change: impl FnOnce(&mut Document) -> Result<T>) -> Result<T> {
        std::fs::create_dir_all(self.path.parent().unwrap())?;
        let _lock = crate::auth::StoreLock::take(&self.path.with_extension("rust"))?;
        let mut document = self.read()?;
        let result = change(&mut document)?;
        if document.definitions.len() > 100
            || serde_json::to_vec(&document)?.len() > 2 * 1024 * 1024
        {
            bail!("Workflow store is full");
        }
        atomic_json(&self.path, &serde_json::to_value(document)?, true)?;
        Ok(result)
    }
    pub fn list(&self) -> Result<Vec<Value>> {
        self.transaction(|doc| Ok(doc.definitions.clone()))
    }
    pub fn templates(&self) -> Result<Vec<Value>> {
        Ok(self.library.list(&json!({"kind":"dispatch"}))?.as_array().unwrap().iter().map(|item|json!({"id":item["id"],"body":item["body"],"resultSchema":item["resultSchema"],"params":item["params"]})).collect())
    }
    pub fn validate(&self, value: &Value) -> Result<Value> {
        Self::validate_templates(value, &self.templates()?)
    }
    fn validate_templates(value: &Value, templates: &[Value]) -> Result<Value> {
        let definition = validate_definition(value)?;
        for step in definition["steps"].as_array().unwrap() {
            let template = templates
                .iter()
                .find(|t| t["id"] == step["template"])
                .ok_or_else(|| {
                    anyhow!("Dispatch template unavailable: {}", text(step, "template"))
                })?;
            if !text(step, "instructions").is_empty()
                && !template["params"]
                    .as_array()
                    .is_some_and(|p| p.iter().any(|p| p["name"] == "task"))
            {
                bail!(
                    "Template {} needs a task input to carry step instructions",
                    text(template, "id")
                );
            }
            if super::worker_results::check_schema(&template["resultSchema"]).is_err() {
                bail!("Template {} needs a result contract", text(template, "id"));
            }
        }
        Ok(definition)
    }
    pub fn mutate(
        &self,
        op: &str,
        id: &str,
        expected: Option<u64>,
        value: &Value,
        name: &str,
    ) -> Result<Option<Value>> {
        if !matches!(op, "create" | "update" | "clone" | "disable" | "delete") {
            bail!("Unknown workflow mutation");
        }
        self.transaction(|doc| {
            let index = doc.definitions.iter().position(|d| d["id"] == id);
            let old = index.map(|i| doc.definitions[i].clone());
            if op != "create" && old.is_none() {
                bail!("Workflow unavailable");
            }
            if let Some(old) = &old {
                let revision = old["revision"].as_u64().unwrap();
                if Some(revision) != expected {
                    return Err(WorkflowConflict {
                        current_revision: revision,
                    }
                    .into());
                }
            }
            if matches!(op, "update" | "disable" | "delete")
                && Self::starters().iter().any(|d| d["id"] == id)
            {
                bail!("Shipped starters are immutable; clone to customize");
            }
            let selected = selections(&self.config.get());
            let is_selected = |id: &str| {
                selected["defaultId"] == id
                    || selected["projects"]
                        .as_object()
                        .unwrap()
                        .values()
                        .any(|v| v == id)
            };
            if matches!(op, "delete" | "disable") && is_selected(id) {
                bail!("Select another workflow globally and in projects first");
            }
            if op == "delete" {
                doc.definitions.remove(index.unwrap());
                return Ok(None);
            }
            let mut next = match op {
                "clone" | "disable" => old.clone().unwrap(),
                _ => value.clone(),
            };
            if !next.is_object() {
                bail!("Expected an object");
            }
            match op {
                "clone" => {
                    next["id"] = json!(format!("workflow-{}", uuid::Uuid::new_v4()));
                    next["revision"] = json!(1);
                    next["name"] = json!(if name.is_empty() {
                        format!("{} copy", text(old.as_ref().unwrap(), "name"))
                    } else {
                        name.into()
                    });
                    next["enabled"] = json!(true);
                }
                "update" | "disable" => {
                    next["id"] = json!(id);
                    next["revision"] =
                        json!(old.as_ref().unwrap()["revision"].as_u64().unwrap() + 1);
                    if op == "disable" {
                        next["enabled"] = json!(false);
                    }
                }
                _ => {
                    next["revision"] = json!(1);
                }
            }
            let next = if op == "disable" {
                validate_definition(&next)?
            } else {
                self.validate(&next)?
            };
            if next["enabled"] == false && is_selected(text(&next, "id")) {
                bail!("Select another workflow globally and in projects first");
            }
            if op == "create" && doc.definitions.iter().any(|d| d["id"] == next["id"]) {
                bail!("Workflow id already exists");
            }
            if matches!(op, "update" | "disable") {
                doc.definitions[index.unwrap()] = next.clone();
            } else {
                doc.definitions.push(next.clone());
            }
            Ok(Some(next))
        })
    }
    pub fn pin(&self, definition: &Value) -> Result<Value> {
        let available = self.templates()?;
        let definition = Self::validate_templates(definition, &available)?;
        let mut templates = serde_json::Map::new();
        for step in definition["steps"].as_array().unwrap() {
            let template = available
                .iter()
                .find(|t| t["id"] == step["template"])
                .ok_or_else(|| anyhow!("Pinned dispatch template disappeared"))?;
            templates.insert(text(template, "id").into(), template.clone());
        }
        let mut pin = json!({"definition":definition,"templates":templates});
        let encoded = serde_json::to_vec(&pin)?;
        if encoded.len() > 256 * 1024 {
            bail!("Pinned workflow exceeds 256 KiB");
        }
        pin["hash"] = json!(format!("{:x}", Sha256::digest(encoded)));
        pin["steps"] = json!(
            definition["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| json!({"id":s["id"],"state":"planned"}))
                .collect::<Vec<_>>()
        );
        Ok(pin)
    }
    pub fn project_key(&self, cwd: &str) -> Result<String> {
        let canonical = super::paths::canonicalize(Path::new(cwd))?;
        let config = self.config.get();
        Ok(config["projects"]
            .as_object()
            .into_iter()
            .flat_map(|p| p.keys())
            .find(|key| {
                super::paths::canonicalize(Path::new(key)).is_ok_and(|path| path == canonical)
            })
            .cloned()
            .unwrap_or_else(|| canonical.to_string_lossy().into()))
    }
    pub fn delivery(&self, cwd: &str) -> Result<&'static str> {
        let key = self.project_key(cwd)?;
        Ok(
            if self.config.get()["projects"][&key]["delivery"] == "local" {
                "Commit on the isolated branch for an approved local merge. Do not push or merge without authority."
            } else {
                "Open a pull request for review only when authorized by the task. Do not merge."
            },
        )
    }
    pub fn pin_for(&self, cwd: &str, id: Option<&str>) -> Result<Value> {
        let selected = selections(&self.config.get());
        let key = self.project_key(cwd)?;
        let id = id
            .or_else(|| selected["projects"][&key].as_str())
            .unwrap_or_else(|| text(&selected, "defaultId"));
        self.transaction(|doc| {
            let definition = doc
                .definitions
                .iter()
                .find(|d| d["id"] == id && d["enabled"] == true)
                .ok_or_else(|| anyhow!("Workflow unavailable or disabled"))?;
            self.pin(definition)
        })
    }
    pub fn select(&self, cwd: Option<&str>, id: Option<&str>, revision: u64) -> Result<Value> {
        let target = id
            .map(str::to_owned)
            .unwrap_or_else(|| text(&selections(&self.config.get()), "defaultId").into());
        let key = cwd.map(|cwd| self.project_key(cwd)).transpose()?;
        self.transaction(|doc| {
            if !doc
                .definitions
                .iter()
                .any(|d| d["id"] == target && d["enabled"] == true)
            {
                bail!("Workflow unavailable or disabled");
            }
            self.config.select_workflow(revision, key.as_deref(), id)
        })?;
        self.catalog()
    }
    pub fn catalog(&self) -> Result<Value> {
        let mut catalog = selections(&self.config.get());
        catalog["available"] = json!(true);
        catalog["definitions"] = json!(self.list()?);
        catalog["templates"] = json!(self.templates()?);
        Ok(catalog)
    }
    /// Definition-only operations. Manager/task operations are owned by the
    /// task service; no anonymous request can assert manager identity here.
    pub fn request(&self, request: &Value) -> Value {
        let result = (|| -> Result<Value> {
            if !request.is_object() || serde_json::to_vec(request)?.len() > 100 * 1024 {
                bail!("Invalid or oversized workflow request");
            }
            match text(request, "op") {
                "list" => Ok(json!({"ok":true,"catalog":self.catalog()?})),
                "get" => Ok(
                    json!({"ok":true,"definition":self.list()?.into_iter().find(|d|d["id"]==request["id"]).ok_or_else(||anyhow!("Workflow unavailable"))?}),
                ),
                "validate" => {
                    Ok(json!({"ok":true,"definition":self.validate(&request["definition"])?}))
                }
                "create" | "update" | "clone" | "disable" | "delete" => {
                    let definition = self.mutate(
                        text(request, "op"),
                        text(request, "id"),
                        request["expectedRevision"].as_u64(),
                        &request["definition"],
                        text(request, "name"),
                    )?;
                    let mut result = json!({"ok":true});
                    if let Some(definition) = definition {
                        result["definition"] = definition;
                    }
                    Ok(result)
                }
                "select" => {
                    let cwd = request["cwd"].as_str().filter(|s| !s.is_empty());
                    if cwd.is_some_and(|p| !Path::new(p).is_absolute()) {
                        bail!("Workflow project cwd must be absolute");
                    }
                    let revision = request["expectedRevision"]
                        .as_u64()
                        .ok_or_else(|| anyhow!("Workflow selection requires expectedRevision"))?;
                    Ok(
                        json!({"ok":true,"catalog":self.select(cwd,request["workflowId"].as_str(),revision)?}),
                    )
                }
                _ => bail!("Workflow task operation requires authenticated manager admission"),
            }
        })();
        match result {
            Ok(value) => value,
            Err(error) => {
                let revision = error
                    .downcast_ref::<WorkflowConflict>()
                    .map(|e| e.current_revision)
                    .or_else(|| {
                        error
                            .downcast_ref::<SelectionConflict>()
                            .map(|e| e.current_revision)
                    });
                let mut result = json!({"ok":false,"code":if revision.is_some(){"conflict"}else{"unavailable"},"error":error.to_string()});
                if let Some(revision) = revision {
                    result["currentRevision"] = json!(revision);
                }
                result
            }
        }
    }
}

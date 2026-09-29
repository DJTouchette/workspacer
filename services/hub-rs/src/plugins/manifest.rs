use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub api_version: String,
    pub server: Option<Server>,
    pub ui: String,
    pub provides: Vec<String>,
    pub settings: Vec<Setting>,
    pub tools: Vec<Tool>,
    pub install: Vec<String>,
    pub disabled: bool,
    pub source: String,
    #[serde(skip)]
    pub dir: PathBuf,
    #[serde(flatten)]
    pub contributions: BTreeMap<String, Value>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    pub command: String,
    pub args: Vec<String>,
    pub port: u16,
    pub health: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Setting {
    pub key: String,
    pub label: String,
    pub scope: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: Option<Value>,
    pub options: Vec<String>,
    pub secret: bool,
    #[serde(flatten)]
    pub metadata: BTreeMap<String, Value>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub method: String,
    pub input_schema: Option<Value>,
}
impl Setting {
    pub fn validate_value(&self, v: &Value) -> Result<()> {
        let valid = match self.kind.as_str() {
            "boolean" => v.is_boolean(),
            "number" => v.is_number(),
            "string" => v.is_string(),
            "select" => v
                .as_str()
                .is_some_and(|s| self.options.iter().any(|o| o == s)),
            _ => false,
        };
        if !valid {
            bail!("setting {:?} expects {}", self.key, self.kind);
        }
        Ok(())
    }
}
pub fn relative_path(raw: &str) -> Result<PathBuf> {
    let raw = raw.replace('\\', "/");
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains(':')
        || raw.split('/').any(|s| s == "..")
        || raw.split('/').all(|s| s.is_empty() || s == ".")
    {
        bail!("expected a relative subdirectory or file path");
    }
    Ok(PathBuf::from(raw))
}
impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let mut m: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        m.dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        m.disabled = m.dir.join(".disabled").exists();
        m.source = std::fs::read_to_string(m.dir.join(".install-source"))
            .unwrap_or_default()
            .trim()
            .into();
        m.validate()?;
        Ok(m)
    }
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || self.id == "."
            || self.id == ".."
        {
            bail!("invalid plugin id");
        }
        static RESERVED: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
        let reserved = RESERVED.get_or_init(|| {
            let vocabulary: Value =
                serde_json::from_str(include_str!("../../assets/hub-vocabulary.json"))
                    .expect("hub vocabulary");
            vocabulary["methods"]
                .as_array()
                .expect("methods")
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|method| method.split('.').next())
                .map(str::to_owned)
                .collect()
        });
        if reserved.contains(self.id.split('.').next().unwrap_or("")) {
            bail!("plugin cannot claim a core/provider namespace");
        }
        if self.api_version != "1" {
            bail!("unsupported apiVersion");
        }
        if self.server.as_ref().is_some_and(|s| s.command.is_empty()) {
            bail!("server.command required");
        }
        if !self.ui.is_empty() {
            relative_path(&self.ui)?;
        }
        let prefix = format!("{}.", self.id);
        for p in &self.provides {
            if !p.starts_with(&prefix)
                || p.len() == prefix.len()
                || (p != &(prefix.clone() + "*") && p.contains('*'))
            {
                bail!("provides must be in plugin namespace");
            }
        }
        let mut keys = BTreeSet::new();
        for s in &self.settings {
            if s.key.is_empty() || !keys.insert(&s.key) {
                bail!("empty or duplicate setting key");
            }
            if !["", "global", "project"].contains(&s.scope.as_str())
                || !["boolean", "number", "string", "select"].contains(&s.kind.as_str())
            {
                bail!("invalid setting scope/type");
            }
            if s.kind == "select" && s.options.is_empty() {
                bail!("select requires options");
            }
            if s.secret
                && (s.kind != "string"
                    || s.scope == "project"
                    || s.default.as_ref().is_some_and(|v| !v.is_null() && v != ""))
            {
                bail!("secret must be global string with no default");
            }
        }
        let mut names = BTreeSet::new();
        for t in &self.tools {
            if t.name.is_empty()
                || t.name.len() > 64
                || !t.name.as_bytes()[0].is_ascii_lowercase()
                || !t
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
                || !names.insert(&t.name)
            {
                bail!("invalid or duplicate tool name");
            }
            if t.description.trim().is_empty()
                || !self
                    .provides
                    .iter()
                    .any(|p| crate::protocol::matches(p, &t.method))
            {
                bail!("tool requires description and declared provides method");
            }
            if t.input_schema
                .as_ref()
                .is_some_and(|v| v.get("type") != Some(&Value::String("object".into())))
            {
                bail!("tool inputSchema must declare object");
            }
        }
        for (field, key) in [("panes", "type"), ("widgets", "id")] {
            if let Some(items) = self.contributions.get(field).and_then(Value::as_array) {
                if !items.is_empty() && self.server.is_none() && self.ui.is_empty() {
                    bail!("UI contributions require server or ui");
                }
                let mut seen = BTreeSet::new();
                for item in items {
                    let id = item.get(key).and_then(Value::as_str).unwrap_or("");
                    if id.is_empty() || !seen.insert(id) {
                        bail!("empty or duplicate contribution");
                    }
                    if field == "widgets" {
                        if let Some(sizes) = item.get("sizes").and_then(Value::as_array) {
                            if sizes.iter().any(|s| {
                                !s.as_str()
                                    .is_some_and(|s| ["small", "medium", "large"].contains(&s))
                            }) {
                                bail!("unknown widget size");
                            }
                        }
                    }
                }
            }
        }
        if let Some(li) = self.contributions.get("launchIntegration") {
            let method = li
                .get("prepareMethod")
                .and_then(Value::as_str)
                .unwrap_or("");
            let agents = li.get("agents").and_then(Value::as_array);
            if self.server.is_none()
                || li.get("version") != Some(&Value::from(1))
                || !self.provides.iter().any(|p| p == method)
                || !method.starts_with(&prefix)
                || method.contains('*')
                || method.contains(' ')
                || agents.is_none_or(|a| a.is_empty())
            {
                bail!("invalid launchIntegration");
            }
            let mut seen = BTreeSet::new();
            for a in agents.unwrap() {
                let a = a.as_str().unwrap_or("");
                if a.is_empty() || a.trim() != a || !seen.insert(a) {
                    bail!("invalid launchIntegration agents");
                }
            }
        }
        Ok(())
    }
}

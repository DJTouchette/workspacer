//! Shared test policy, not runtime authority. Source gaps fail independently of
//! vocabulary membership; a registered name is not a proof about its payload.
use crate::Report;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub path_parameters: BTreeMap<String, String>,
    pub method_decisions: BTreeMap<String, String>,
    pub inert_methods: BTreeMap<String, String>,
    pub parameter_decisions: BTreeMap<String, BTreeMap<String, Decision>>,
    pub dangerous_names: BTreeMap<String, String>,
    pub path_namespaces: Vec<String>,
    #[serde(default)]
    pub opaque_decisions: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub inspection_decisions: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub source_parameter_decisions: BTreeMap<String, BTreeMap<String, Decision>>,
}
#[derive(Deserialize)]
pub struct Decision {
    pub kind: String,
    pub reason: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    pub full: Vec<String>,
    pub catalog: Vec<String>,
    pub architectural_retirements: BTreeMap<String, serde_json::Value>,
}
#[derive(Debug, Serialize)]
pub struct Check {
    pub methods: usize,
    pub dangerous_bindings: usize,
    pub opaque_methods: usize,
    pub errors: Vec<String>,
}
fn folded(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c == 'ſ' {
                's'
            } else {
                let mut lower = c.to_lowercase();
                let first = lower.next().unwrap_or(c);
                if lower.next().is_none() { first } else { c }
            }
        })
        .collect()
}
fn words(value: &str) -> Vec<String> {
    let chars: Vec<_> = value.chars().collect();
    let mut result = Vec::new();
    let mut current = String::new();
    for (i, c) in chars.iter().copied().enumerate() {
        if matches!(c, '_' | '-' | '.' | ' ') || c.is_numeric() {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            continue;
        }
        if c.is_uppercase()
            && (i == 0
                || !chars[i - 1].is_uppercase()
                || chars.get(i + 1).is_some_and(|c| c.is_lowercase()))
            && !current.is_empty()
        {
            result.push(std::mem::take(&mut current));
        }
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        result.push(current)
    }
    result
}
impl Policy {
    fn classified(&self, method: &str) -> bool {
        self.path_parameters.contains_key(method)
            || self.method_decisions.contains_key(method)
            || self.inert_methods.contains_key(method)
    }
    fn decision(&self, method: &str, path: &str) -> bool {
        let leaf = path.rsplit('.').next().unwrap_or(path);
        if self
            .path_parameters
            .get(method)
            .is_some_and(|p| folded(p) == folded(leaf))
        {
            return true;
        }
        self.parameter_decisions
            .get(method)
            .into_iter()
            .chain(self.source_parameter_decisions.get(method))
            .any(|rows| {
                rows.iter().any(|(name, d)| {
                    matches!(
                        d.kind.as_str(),
                        "path"
                            | "filename"
                            | "executable"
                            | "argv"
                            | "shell"
                            | "env"
                            | "url"
                            | "port"
                            | "id"
                            | "regex"
                            | "permission"
                            | "inert"
                    ) && !d.reason.trim().is_empty()
                        && (folded(name) == folded(path) || folded(name) == folded(leaf))
                })
            })
    }
    pub fn check(
        &self,
        report: &Report,
        surface: &Surface,
        vocabulary: &serde_json::Value,
    ) -> Check {
        let mut out = Check {
            methods: 0,
            dangerous_bindings: 0,
            opaque_methods: 0,
            errors: Vec::new(),
        };
        if report.source_files == 0 || report.methods.len() < 100 {
            out.errors
                .push("source/registration population collapsed".into())
        }
        for issue in &report.unresolved_registrations {
            out.errors.push(format!("unresolved registration: {issue}"))
        }
        let stems: BTreeSet<_> = vocabulary["stems"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .collect();
        if stems.is_empty() {
            out.errors
                .push("empty dangerous-name stem vocabulary".into())
        }
        let golden: BTreeMap<String, String> =
            serde_json::from_value(vocabulary["params"].clone()).unwrap_or_default();
        if golden != self.dangerous_names || golden.is_empty() {
            out.errors
                .push("dangerous-name policy differs from independent golden".into())
        }
        let methods: BTreeSet<_> = surface.full.iter().chain(&surface.catalog).collect();
        if methods.len() < 40 {
            out.errors.push("brain surface population collapsed".into())
        }
        for method in methods {
            if surface.architectural_retirements.contains_key(method) {
                continue;
            }
            let Some(bound) = report.methods.get(method) else {
                out.errors.push(format!("missing actual handler {method}"));
                continue;
            };
            out.methods += 1;
            if !self.classified(method) {
                out.errors.push(format!("unclassified capability {method}"))
            }
            for field in &bound.fields {
                let leaf = field.rsplit('.').next().unwrap_or(field);
                let known = self
                    .dangerous_names
                    .keys()
                    .any(|name| folded(name) == folded(leaf));
                if known {
                    out.dangerous_bindings += 1;
                    if !self.decision(method, field) {
                        out.errors
                            .push(format!("unclassified caller binding {method}.{field}"))
                    }
                } else if words(leaf).iter().any(|word| stems.contains(word.as_str()))
                    && !self.decision(method, field)
                {
                    out.errors
                        .push(format!("unreviewed dangerous spelling {method}.{field}"))
                }
            }
            if let Some(rows) = self.source_parameter_decisions.get(method) {
                for field in rows.keys() {
                    if !bound
                        .fields
                        .iter()
                        .any(|actual| folded(actual) == folded(field))
                    {
                        out.errors.push(format!(
                            "source decision lacks actual binding {method}.{field}"
                        ));
                    }
                }
            }
            let path_inert = self.inert_methods.contains_key(method)
                && self.path_namespaces.iter().any(|p| method.starts_with(p));
            if path_inert && (!bound.fields.is_empty() || !bound.opaque.is_empty()) {
                out.errors
                    .push(format!("inert path method binds caller payload {method}"))
            }
            if !bound.opaque.is_empty() {
                out.opaque_methods += 1;
            }
            for path in &bound.opaque_paths {
                let key_only = self.inspection_decisions.get(method).is_some_and(|rows| {
                    rows.iter().any(|(reviewed, reason)| {
                        !reason.trim().is_empty()
                            && (reviewed == "$"
                                || reviewed == path
                                || path.starts_with(&format!("{reviewed}.")))
                            && bound.key_inspections.contains(reviewed)
                            && !bound.opaque_transforms.iter().any(|transformed| {
                                reviewed == "$"
                                    || reviewed == transformed
                                    || transformed.starts_with(&format!("{reviewed}."))
                            })
                    })
                });
                if !key_only
                    && !self.opaque_decisions.get(method).is_some_and(|rows| {
                        rows.iter().any(|(reviewed, reason)| {
                            !reason.trim().is_empty()
                                && (reviewed == "$"
                                    || reviewed == path
                                    || path.starts_with(&format!("{reviewed}.")))
                        })
                    })
                {
                    out.errors
                        .push(format!("opaque caller path lacks decision {method}:{path}"));
                }
            }
            for issue in &bound.unresolved {
                out.errors.push(format!("unresolved {method}: {issue}"))
            }
        }
        for (method, key) in [
            ("terminals.create", "shell"),
            ("sessions.load", "filename"),
            ("claude.profiles.update", "configDir"),
            ("sessions.terminalInput", "bytesB64"),
            ("layouts.save", "name"),
            ("sessions.save", "name"),
        ] {
            if !report
                .methods
                .get(method)
                .is_some_and(|b| b.fields.iter().any(|p| p.rsplit('.').next() == Some(key)))
            {
                out.errors
                    .push(format!("missing binding canary {method}.{key}"))
            }
        }
        if out.opaque_methods == 0 {
            out.errors.push("opaque payload coverage collapsed".into())
        }
        out
    }
}

/// Runtime spelling ownership is independent of the historical Go snapshot.
/// All traced roots are reserved, including stripped or conservatively read keys;
/// membership never means that a provider accepts that field or its value.
pub fn check_spawn_keys(
    report: &Report,
    contract: &serde_json::Value,
    historical: &serde_json::Value,
) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(rows) = contract["keys"].as_array() else {
        return vec!["spawn key contract lacks keys".into()];
    };
    let keys: BTreeSet<_> = rows.iter().filter_map(serde_json::Value::as_str).collect();
    if rows.len() != 52 || keys.len() != 52 {
        errors.push("spawn key registry must contain exactly52 unique reviewed keys".into());
    }
    if keys.iter().any(|key| {
        !key.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            || !key.bytes().all(|b| b.is_ascii_alphanumeric())
    }) {
        errors.push("spawn key registry contains malformed canonical spelling".into());
    }
    if keys
        .iter()
        .map(|key| key.to_ascii_lowercase())
        .collect::<BTreeSet<_>>()
        .len()
        != keys.len()
    {
        errors.push("spawn key registry has ambiguous case-folded names".into());
    }
    let old: BTreeSet<_> = historical["spawnKeys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect();
    if old.len() != 46 {
        errors.push("historical spawn key population changed".into());
    }
    for key in &old {
        if !keys.contains(key) {
            errors.push(format!("historical spawn key omitted: {key}"));
        }
    }
    let additions: BTreeSet<_> = keys.difference(&old).copied().collect();
    let reservations = contract["reservations"].as_object();
    let reviewed: BTreeSet<_> = reservations
        .into_iter()
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();
    if additions != reviewed {
        errors
            .push("spawn key reservations do not exactly explain the historical additions".into());
    }
    for (key, reason) in reservations.into_iter().flatten() {
        if !reason.as_str().is_some_and(|s| !s.trim().is_empty()) {
            errors.push(format!("spawn key reservation lacks review: {key}"));
        }
    }
    let Some(bound) = report.methods.get("agents.spawn") else {
        errors.push("agents.spawn source binding missing".into());
        return errors;
    };
    let roots: BTreeSet<_> = bound
        .fields
        .iter()
        .filter_map(|p| p.split('.').next())
        .collect();
    if roots.len() < 40 {
        errors.push(format!(
            "agents.spawn source root population collapsed: {}",
            roots.len()
        ));
    }
    for field in roots {
        if !keys.contains(field) {
            errors.push(format!(
                "agents.spawn caller root missing canonical spelling: {field}"
            ));
        }
    }
    errors
}

use super::manifest::Manifest;
use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
pub const SECRET_PLACEHOLDER: &str = "__WKS_SECRET__";
fn overlay(m: &Manifest) -> Map<String, Value> {
    std::fs::read(m.dir.join(".settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn merged(m: &Manifest) -> Map<String, Value> {
    let persisted = overlay(m);
    let mut out = Map::new();
    for s in &m.settings {
        if let Some(v) = persisted.get(&s.key).or(s.default.as_ref()) {
            out.insert(s.key.clone(), v.clone());
        }
    }
    out
}
pub fn redacted(m: &Manifest) -> Value {
    let mut out = merged(m);
    for s in &m.settings {
        if s.secret
            && out
                .get(&s.key)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        {
            out.insert(s.key.clone(), Value::String(SECRET_PLACEHOLDER.into()));
        }
    }
    Value::Object(out)
}
pub fn update(m: &Manifest, partial: &Map<String, Value>) -> Result<Value> {
    let mut values = overlay(m);
    for (k, v) in partial {
        let def = m
            .settings
            .iter()
            .find(|s| &s.key == k)
            .ok_or_else(|| anyhow!("unknown setting key {k}"))?;
        if def.secret && v == SECRET_PLACEHOLDER {
            continue;
        }
        if v.is_null() {
            values.remove(k);
        } else {
            def.validate_value(v)?;
            values.insert(k.clone(), v.clone());
        }
    }
    crate::services::atomic_json(&m.dir.join(".settings.json"), &Value::Object(values), true)?;
    Ok(redacted(m))
}

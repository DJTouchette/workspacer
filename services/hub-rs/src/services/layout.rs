use crate::{Caller, Handle, protocol::Event};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex};

pub struct Layout {
    document: Mutex<Value>,
    path: Option<PathBuf>,
    hub: Handle,
}
impl Layout {
    pub fn open(path: Option<PathBuf>, hub: Handle) -> Self {
        let mut document = json!({"version":0, "data":null});
        if let Some(path) = &path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value)
                        if (value.is_object() || value.is_null())
                            && (value["version"].is_null()
                                || value["version"].as_i64().is_some()) =>
                    {
                        document = json!({"version":value["version"].as_i64().unwrap_or(0), "data":value["data"]});
                        redact_tokens(&mut document["data"]);
                    }
                    _ => eprintln!("layout: persisted document is invalid; starting empty"),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => eprintln!("layout: could not read persisted document: {e}"),
            }
        }
        Self {
            document: Mutex::new(document),
            path,
            hub,
        }
    }
    pub fn get(&self) -> Value {
        self.document.lock().unwrap().clone()
    }
    pub fn set(&self, caller: &Caller, params: Value) -> Result<Value> {
        let mut data = params
            .get("data")
            .cloned()
            .ok_or_else(|| anyhow!("layout.set requires {{ data }}"))?;
        if !caller.trusted {
            scrub_boot_document(&mut data);
        }
        redact_tokens(&mut data);
        let mut document = self.document.lock().unwrap();
        let version = document["version"]
            .as_i64()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow!("layout version overflow"))?;
        *document = json!({"version":version,"data":data});
        if let Some(path) = &self.path
            && let Err(e) = super::atomic_json(path, &document, false)
        {
            eprintln!(
                "layout: failed to persist version {version}; live changes will be lost on restart: {e}"
            );
        }
        if let Err(error) = self
            .hub
            .publish(Event::new("layout.changed", "hub", document.clone()))
        {
            eprintln!(
                "layout: version {version} is committed but its change event could not be queued: {error}"
            );
        }
        Ok(document.clone())
    }
}

pub fn redact_tokens(value: &mut Value) {
    match value {
        Value::String(text) => {
            let mut offset = 0;
            while let Some(index) = text[offset..].find("busToken=") {
                let start = offset + index + "busToken=".len();
                let end = text[start..]
                    .find(['&', '"', '\\'])
                    .map(|i| start + i)
                    .unwrap_or(text.len());
                text.replace_range(start..end, "");
                offset = start;
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact_tokens),
        Value::Object(values) => {
            let original = std::mem::take(values);
            for (key, mut value) in original {
                let mut key = Value::String(key);
                redact_tokens(&mut key);
                redact_tokens(&mut value);
                let Value::String(key) = key else {
                    unreachable!()
                };
                values.insert(key, value);
            }
        }
        _ => (),
    }
}

/// The desktop may execute restored agents/panes. Keep the legacy structural
/// scrub at this persisted-document boundary, including explicit receipts.
pub fn scrub_boot_document(value: &mut Value) {
    scrub_document(value, true);
}
pub fn scrub_saved_document(value: &mut Value) {
    scrub_document(value, false);
}
fn scrub_document(value: &mut Value, strict_shape: bool) {
    let Some(agents) = value.get_mut("agents").and_then(Value::as_array_mut) else {
        return;
    };
    if strict_shape && agents.iter().any(|a| !a.is_object() && !a.is_null()) {
        return;
    }
    for agent in agents {
        let Some(agent) = agent.as_object_mut() else {
            continue;
        };
        agent.remove("escalationScrubbed");
        let mut dropped = Vec::new();
        for key in [
            "skipPermissions",
            "permissionMode",
            "profileId",
            "mcpItemIds",
            "launchIntegrationId",
        ] {
            if agent.remove(key).is_some() {
                dropped.push(key);
            }
        }
        if let Some(tabs) = agent.get_mut("tabs").and_then(Value::as_array_mut)
            && (!strict_shape || tabs.iter().all(|t| t.is_object() || t.is_null()))
        {
            for tab in tabs {
                if let Some(panes) = tab.get_mut("panes").and_then(Value::as_array_mut)
                    && (!strict_shape || panes.iter().all(|p| p.is_object() || p.is_null()))
                {
                    for pane in panes {
                        if let Some(pane) = pane.as_object_mut() {
                            for key in ["shell", "initialCommand", "pluginId"] {
                                if pane.remove(key).is_some() {
                                    dropped.push(if strict_shape {
                                        "pane"
                                    } else {
                                        match key {
                                            "shell" => "pane.shell",
                                            "initialCommand" => "pane.initialCommand",
                                            "pluginId" => "pane.pluginId",
                                            _ => unreachable!(),
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        if !dropped.is_empty() {
            agent.insert("escalationScrubbed".into(), json!(dropped));
        }
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::*;
    #[test]
    fn bearer_urls_are_redacted_in_keys_and_values_at_every_depth() {
        let mut document = json!({"ws://host/bus?busToken=key-secret&keep=1":{"url":"ws://host/bus?busToken=value-secret","nested":[{"busToken=nested-secret":true}]},"untouched":"hello"});
        redact_tokens(&mut document);
        let serialized = document.to_string();
        assert!(!serialized.contains("secret"));
        assert_eq!(
            document["ws://host/bus?busToken=&keep=1"]["url"],
            "ws://host/bus?busToken="
        );
        assert_eq!(document["untouched"], "hello");
    }
}

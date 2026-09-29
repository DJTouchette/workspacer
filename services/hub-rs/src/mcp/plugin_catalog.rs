use crate::{Handle, client::Client};
use anyhow::{Result, anyhow};
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn sanitized_name(id: &str) -> String {
    regex::Regex::new("[^a-z0-9]+")
        .unwrap()
        .replace_all(&id.to_lowercase(), "_")
        .trim_matches('_')
        .to_owned()
}
fn parse_tools(rows: &Value) -> Result<BTreeMap<String, (Tool, String)>> {
    let rows = rows
        .as_array()
        .ok_or_else(|| anyhow!("invalid plugin catalog"))?;
    let mut tools = BTreeMap::new();
    for row in rows {
        let Some(id) = row["pluginId"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        let prefix = sanitized_name(id);
        let Some(defs) = row["tools"].as_array() else {
            continue;
        };
        for def in defs {
            let (Some(name), Some(method), Some(description)) = (
                def["name"].as_str(),
                def["method"].as_str(),
                def["description"].as_str(),
            ) else {
                continue;
            };
            if !method.starts_with(&format!("{id}.")) {
                continue;
            }
            let name = format!("{prefix}_{name}");
            if super::catalogs()
                .values()
                .flatten()
                .any(|tool| tool.name == name)
            {
                continue;
            }
            let schema = def
                .get("inputSchema")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| json!({"type":"object"}));
            if schema["type"] != "object" {
                continue;
            }
            let tool: Tool = serde_json::from_value(
                json!({"name":name,"description":description,"inputSchema":schema}),
            )?;
            if tools.insert(name, (tool, method.to_owned())).is_some() {
                return Err(anyhow!("plugin tool names collide after normalization"));
            }
        }
    }
    Ok(tools)
}

#[derive(Default)]
pub(super) struct Catalog {
    tools: RwLock<BTreeMap<String, (Tool, String)>>,
    ready: Arc<AtomicBool>,
    upstream: Option<Arc<crate::provider_relay::UpstreamCaller>>,
}
impl Catalog {
    pub fn with_readiness(
        ready: Arc<AtomicBool>,
        upstream: Option<Arc<crate::provider_relay::UpstreamCaller>>,
    ) -> Self {
        Self {
            tools: RwLock::new(BTreeMap::new()),
            ready,
            upstream,
        }
    }
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn tool(&self, name: &str) -> Option<(Tool, String)> {
        self.tools.read().unwrap().get(name).cloned()
    }
    pub fn tools(&self) -> Vec<Tool> {
        self.tools
            .read()
            .unwrap()
            .values()
            .map(|(tool, _)| tool.clone())
            .collect()
    }
    pub async fn refresh(&self, hub: &Handle) -> Result<()> {
        let client = if let Some(upstream) = &self.upstream {
            upstream.client().await?
        } else {
            Client::connect_service(hub).await?
        };
        let rows = client
            .call_with_timeout("plugins.tools", Value::Null, Duration::from_secs(10))
            .await?;
        self.replace(rows)
    }
    fn replace(&self, rows: Value) -> Result<()> {
        let tools = parse_tools(&rows)?;
        *self.tools.write().unwrap() = tools;
        self.ready.store(true, Ordering::Release);
        Ok(())
    }
    pub async fn run(&self, hub: Handle) -> Result<()> {
        hub.ready().await?;
        let mut upstream_state = self.upstream.as_ref().map(|upstream| upstream.state());
        loop {
            if self.refresh(&hub).await.is_err() {
                self.ready.store(false, Ordering::Release);
            }
            tokio::select! {
                _=tokio::time::sleep(Duration::from_secs(15))=>(),
                changed=async{upstream_state.as_mut().unwrap().changed().await},if upstream_state.is_some()=>{changed.map_err(|_|anyhow!("upstream caller state closed"))?;self.ready.store(false,Ordering::Release);}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: &str, name: &str) -> Value {
        json!({"pluginId":id,"tools":[{"name":name,"description":"fixture","method":format!("{id}.{name}")}]})
    }
    #[test]
    fn names_and_default_schema_match_the_go_plugin_bridge() {
        for (input, expected) in [
            ("djtouchette.jira", "djtouchette_jira"),
            ("A.B-C", "a_b_c"),
            ("..x..", "x"),
        ] {
            assert_eq!(sanitized_name(input), expected);
        }
        let catalog = Catalog::default();
        catalog.replace(json!([row("acme", "echo")])).unwrap();
        let (tool, method) = catalog.tool("acme_echo").unwrap();
        assert_eq!(method, "acme.echo");
        assert_eq!(tool.input_schema.get("type"), Some(&json!("object")));
        assert!(catalog.ready());
    }
    #[test]
    fn malformed_schemas_foreign_methods_and_builtin_shadows_are_not_advertised() {
        let mut wrong_schema = row("wrong", "schema");
        wrong_schema["tools"][0]["inputSchema"] = json!({"type":"array"});
        let mut foreign = row("foreign", "method");
        foreign["tools"][0]["method"] = "agents.spawn".into();
        let mut valid = row("acme", "echo");
        let schema = json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false});
        valid["tools"][0]["inputSchema"] = schema.clone();
        let catalog = Catalog::default();
        catalog
            .replace(json!([wrong_schema, foreign, row("spawn", "agent"), valid]))
            .unwrap();
        assert_eq!(catalog.tools().len(), 1);
        assert_eq!(
            &*catalog.tool("acme_echo").unwrap().0.input_schema,
            schema.as_object().unwrap()
        );
        assert!(catalog.tool("spawn_agent").is_none());
    }
    #[test]
    fn collisions_do_not_partially_replace_and_successful_removal_retires_tools() {
        let catalog = Catalog::default();
        catalog.replace(json!([row("stable", "echo")])).unwrap();
        let error = catalog
            .replace(json!([row("one.two", "echo"), row("one-two", "echo")]))
            .unwrap_err();
        assert!(error.to_string().contains("collide"));
        assert!(catalog.tool("stable_echo").is_some());
        assert!(catalog.tool("one_two_echo").is_none());
        catalog.replace(json!([])).unwrap();
        assert!(catalog.ready());
        assert!(catalog.tools().is_empty());
        assert!(catalog.tool("stable_echo").is_none());
    }
}

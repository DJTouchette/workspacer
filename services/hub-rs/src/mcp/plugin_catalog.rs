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
        let rows = rows
            .as_array()
            .ok_or_else(|| anyhow!("invalid plugin catalog"))?;
        let mut tools = BTreeMap::new();
        for row in rows {
            let Some(id) = row["pluginId"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            let prefix = regex::Regex::new("[^a-z0-9]+")
                .unwrap()
                .replace_all(&id.to_lowercase(), "_")
                .trim_matches('_')
                .to_owned();
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

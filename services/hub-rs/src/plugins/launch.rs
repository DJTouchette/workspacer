//! Optional plugin launch patches, authorized by a live owner spawn's opaque
//! broker permit. No JSON field can manufacture this in-memory lease.
mod codex;
use super::manifest::Manifest;
use crate::{
    Handle, LaunchPermit,
    services::{
        agent_lifecycle::{LaunchPreparation, Operation},
        spawn_plan::Plan,
    },
};
use anyhow::{Result, anyhow, bail};
pub use codex::{Provider, provider_from_config, routing_config_args};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Patch {
    pub env: BTreeMap<String, String>,
    pub args: Vec<String>,
}
pub fn validate_patch(value: Value) -> Result<Patch> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("Invalid launch integration response"))?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "env" | "args"))
    {
        bail!("Unsupported launch integration field")
    }
    let mut patch = Patch::default();
    if let Some(env) = object.get("env") {
        let env = env
            .as_object()
            .filter(|env| env.len() <= 64)
            .ok_or_else(|| anyhow!("Invalid launch environment"))?;
        for (key, value) in env {
            let Some(value) = value.as_str() else {
                bail!("Invalid launch environment entry")
            };
            if key.is_empty()
                || key.len() > 128
                || !key.as_bytes()[0].is_ascii_alphabetic() && key.as_bytes()[0] != b'_'
                || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                || ["__proto__", "constructor", "prototype"].contains(&key.as_str())
                || value.contains('\0')
                || value.encode_utf16().count() > 32768
            {
                bail!("Invalid launch environment entry")
            }
            patch.env.insert(key.clone(), value.into());
        }
    }
    if let Some(args) = object.get("args") {
        let args = args
            .as_array()
            .filter(|args| args.len() <= 64)
            .ok_or_else(|| anyhow!("Invalid launch arguments"))?;
        for arg in args {
            let Some(arg) = arg
                .as_str()
                .filter(|arg| !arg.contains('\0') && arg.encode_utf16().count() <= 8192)
            else {
                bail!("Invalid launch arguments")
            };
            patch.args.push(arg.into());
        }
    }
    Ok(patch)
}
/// The shape mirrors the TS contract; this additional admission check keeps
/// a trusted integration from invalidating the host's resolved model/identity
/// receipt or replacing its facade injection after routing policy ran.
fn validate_overlay(patch: &Patch, provider: &str) -> Result<()> {
    for key in patch.env.keys() {
        let key = key.to_ascii_uppercase();
        if [
            "PATH",
            "PATHEXT",
            "COMSPEC",
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "ANTHROPIC_MODEL",
            "OPENAI_MODEL",
            "CODEX_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_EFFORT_LEVEL",
            "HUB_TOKEN",
            "WORKSPACER_PARENT_PID",
            "NODE_OPTIONS",
            "NODE_PATH",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
        ]
        .contains(&key.as_str())
            || key.starts_with("ANTHROPIC_DEFAULT_")
            || key.starts_with("WKS_")
        {
            bail!("launch integration cannot override host-owned routing or identity environment")
        }
    }
    let mut index = 0;
    while index < patch.args.len() {
        let arg = &patch.args[index];
        let flag = arg.split('=').next().unwrap_or(arg);
        if [
            "--model",
            "--effort",
            "--context-window",
            "--context_window",
            "--session-id",
            "--session_id",
            "--resume",
            "--resume-session-id",
            "--resume-session",
            "--session",
            "--conversation-id",
            "--thread-id",
            "--fork-session",
            "fork",
            "--agent",
            "--agents",
            "--output-format",
            "--input-format",
            "--transport",
            "--listen",
            "resume",
            "--continue",
            "--cwd",
            "--cd",
            "--directory",
            "--worktree",
            "--profile",
            "--settings",
            "--setting-sources",
            "--provider",
            "--permission-mode",
            "--dangerously-skip-permissions",
            "--dangerously-bypass-approvals-and-sandbox",
            "--full-auto",
            "--mcp-config",
            "--strict-mcp-config",
            "--allowedTools",
            "--disallowedTools",
            "--tools",
            "--append-system-prompt",
            "--system-prompt",
        ]
        .contains(&flag)
            || arg.starts_with("-C")
            || arg.starts_with("-r")
            || arg.starts_with("-w")
            || arg.starts_with("-m")
            || arg.starts_with("-p")
        {
            bail!("launch integration cannot override host-owned launch arguments")
        }
        let config = if arg == "-c" || arg == "--config" {
            index += 1;
            Some(
                patch
                    .args
                    .get(index)
                    .ok_or_else(|| anyhow!("Missing integration config override"))?
                    .as_str(),
            )
        } else {
            arg.strip_prefix("--config=")
                .or_else(|| arg.strip_prefix("-c").filter(|s| !s.is_empty()))
        };
        if let Some(config) = config {
            if provider != "codex" {
                bail!("launch integration cannot override host-owned launch configuration")
            };
            let key = config.split('=').next().unwrap_or("").trim();
            if key
                .split('.')
                .next()
                .unwrap_or("")
                .contains(['\\', '[', ']'])
            {
                bail!("ambiguous integration configuration key is not allowed")
            }
            let root = key
                .split('.')
                .next()
                .unwrap_or("")
                .trim_matches(['\'', '"']);
            if [
                "model",
                "model_provider",
                "model_reasoning_effort",
                "model_context_window",
                "model_max_output_tokens",
                "profile",
                "profiles",
                "cwd",
                "approval_policy",
                "sandbox_mode",
                "mcp_servers",
            ]
            .contains(&root)
            {
                bail!(
                    "launch integration cannot override host-owned routing or facade configuration"
                )
            }
        }
        index += 1;
    }
    Ok(())
}
trait Host: Send + Sync {
    fn check<'a>(&'a self, permit: &'a LaunchPermit, finish: bool) -> Operation<'a, ()>;
    fn manifest<'a>(&'a self, id: &'a str) -> Operation<'a, Manifest>;
    fn call<'a>(&'a self, method: &'a str, context: Value) -> Operation<'a, Value>;
}
struct HubHost(Handle);
struct OwnedClient(crate::client::Client);
impl Drop for OwnedClient {
    fn drop(&mut self) {
        self.0.close();
    }
}
impl Host for HubHost {
    fn check<'a>(&'a self, permit: &'a LaunchPermit, finish: bool) -> Operation<'a, ()> {
        Box::pin(self.0.check_launch_preparation(permit, finish))
    }
    fn manifest<'a>(&'a self, id: &'a str) -> Operation<'a, Manifest> {
        Box::pin(async move {
            let client = OwnedClient(crate::client::Client::connect(&self.0).await?);
            let value = client
                .0
                .call_with_timeout("plugins.manifests", Value::Null, Duration::from_secs(3))
                .await?;
            let manifests: Vec<Manifest> = serde_json::from_value(value)?;
            manifests
                .into_iter()
                .find(|manifest| manifest.id == id && !manifest.disabled)
                .ok_or_else(|| anyhow!("selected launch integration is unavailable"))
        })
    }
    fn call<'a>(&'a self, method: &'a str, context: Value) -> Operation<'a, Value> {
        Box::pin(async move {
            let client = OwnedClient(crate::client::Client::connect(&self.0).await?);
            client
                .0
                .call_with_timeout(method, context, Duration::from_secs(20))
                .await
        })
    }
}
struct Entry {
    permit: LaunchPermit,
    generation: Option<String>,
}
pub struct Preparation {
    inner: Arc<dyn LaunchPreparation>,
    host: Arc<dyn Host>,
    probe: Arc<dyn codex::Probe>,
    pending: Mutex<BTreeMap<String, Entry>>,
}
pub(crate) struct Lease {
    owner: Arc<Preparation>,
    permit: LaunchPermit,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let removed = self.owner.remove(&self.permit);
        if removed {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let host = self.owner.host.clone();
                let permit = self.permit.clone();
                runtime.spawn(async move {
                    let _ = host.check(&permit, true).await;
                });
            }
        }
    }
}
impl Preparation {
    pub fn new(inner: Arc<dyn LaunchPreparation>, hub: Handle) -> Arc<Self> {
        Arc::new(Self {
            inner,
            host: Arc::new(HubHost(hub)),
            probe: Arc::new(codex::NativeProbe),
            pending: Mutex::new(BTreeMap::new()),
        })
    }
    pub(crate) fn admit(self: &Arc<Self>, permit: LaunchPermit) -> Result<Lease> {
        let mut pending = self.pending.lock().unwrap();
        if permit.nonce.is_empty() || permit.session_id.is_empty() || permit.plugin_id.is_empty() {
            bail!("invalid launch permit")
        }
        if pending.len() >= 128 {
            bail!("too many pending launch preparations")
        }
        if pending.contains_key(&permit.session_id) {
            bail!("session already has a pending launch integration")
        }
        pending.insert(
            permit.session_id.clone(),
            Entry {
                permit: permit.clone(),
                generation: None,
            },
        );
        Ok(Lease {
            owner: self.clone(),
            permit,
        })
    }
    fn remove(&self, permit: &LaunchPermit) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if pending
            .get(&permit.session_id)
            .is_some_and(|entry| entry.permit == *permit)
        {
            pending.remove(&permit.session_id);
            true
        } else {
            false
        }
    }
    async fn selected(&self, id: &str, agent: &str) -> Result<(Manifest, String)> {
        let manifest = self.host.manifest(id).await?;
        let contribution = manifest
            .contributions
            .get("launchIntegration")
            .ok_or_else(|| anyhow!("selected launch integration is unavailable"))?;
        let method = contribution["prepareMethod"].as_str().unwrap_or("");
        if manifest.id != id
            || manifest.disabled
            || manifest.server.is_none()
            || contribution["version"] != 1
            || !contribution["agents"]
                .as_array()
                .is_some_and(|agents| agents.iter().any(|supported| supported == agent))
            || !method.starts_with(&format!("{id}."))
            || !manifest.provides.iter().any(|provided| provided == method)
        {
            bail!("Plugin is unavailable or does not support {agent}")
        }
        let method = method.to_owned();
        Ok((manifest, method))
    }
    async fn prepare_selected(
        &self,
        plan: &mut Plan,
        generation: &str,
        id: &str,
        permit: &LaunchPermit,
    ) -> Result<()> {
        if !["claude", "codex"].contains(&plan.provider.as_str()) {
            bail!("This host supports launch integrations for Claude and Codex only")
        }
        self.host.check(permit, false).await?;
        self.inner.prepare(plan, generation).await?;
        self.host.check(permit, false).await?;
        let (manifest, method) = self.selected(id, &plan.provider).await?;
        let cwd = plan.request["cwd"]
            .as_str()
            .filter(|cwd| !cwd.is_empty())
            .ok_or_else(|| anyhow!("launch cwd required"))?;
        let mut context = json!({"version":1,"agent":plan.provider,"cwd":cwd,"resume":plan.metadata["launchIntegrationResume"].as_bool().unwrap_or_else(||plan.request["resume"].as_str().is_some_and(|s|!s.is_empty()))});
        if let Some(model) = plan.request["model"]
            .as_str()
            .filter(|model| !model.is_empty())
        {
            context["model"] = model.into();
        }
        if plan.provider == "codex" {
            context["provider"] = serde_json::to_value(self.probe.read(plan).await?)?;
        }
        self.host.check(permit, false).await?;
        let patch = validate_patch(self.host.call(&method, context).await?)?;
        validate_overlay(&patch, &plan.provider)?;
        self.host.check(permit, false).await?;
        let (current, current_method) = self.selected(id, &plan.provider).await?;
        if current_method != method
            || current.contributions.get("launchIntegration")
                != manifest.contributions.get("launchIntegration")
        {
            bail!("launch integration changed during preparation")
        }
        // Validate the existing destination before consuming authority; changes
        // remain local until the entire response and owner proof are accepted.
        let mut env = plan
            .request
            .get("env")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let env_map = env
            .as_object_mut()
            .ok_or_else(|| anyhow!("launch environment must be an object"))?;
        for (key, value) in patch.env {
            #[cfg(windows)]
            env_map.retain(|existing, _| !existing.eq_ignore_ascii_case(&key));
            env_map.insert(key, value.into());
        }
        let key = if plan.endpoint == "/sessions/spawn" {
            "argv"
        } else {
            "extra_args"
        };
        let mut args = plan.request.get(key).cloned().unwrap_or_else(|| json!([]));
        let args_vec = args
            .as_array_mut()
            .ok_or_else(|| anyhow!("launch arguments must be an array"))?;
        args_vec.extend(patch.args.into_iter().map(Value::String));
        self.host.check(permit, true).await?;
        plan.request["env"] = env;
        plan.request[key] = args;
        Ok(())
    }
}
impl LaunchPreparation for Preparation {
    fn sweep<'a>(&'a self, live: &'a std::collections::BTreeSet<String>) -> Operation<'a, ()> {
        self.inner.sweep(live)
    }
    fn prepare<'a>(&'a self, plan: &'a mut Plan, generation: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            let selected = plan.metadata["settings"]
                .get("launchIntegrationId")
                .filter(|value| !value.is_null());
            let Some(selected) = selected else {
                if self.pending.lock().unwrap().contains_key(&plan.session_id) {
                    bail!("launch integration selection missing from admitted plan")
                };
                return self.inner.prepare(plan, generation).await;
            };
            let id = selected
                .as_str()
                .filter(|id| !id.is_empty() && id.trim() == *id && id.encode_utf16().count() <= 200)
                .ok_or_else(|| anyhow!("Invalid launch integration selection"))?
                .to_owned();
            let permit = {
                let mut pending = self.pending.lock().unwrap();
                let entry = pending
                    .get_mut(&plan.session_id)
                    .ok_or_else(|| anyhow!("launch integration has no active owner permit"))?;
                if entry.permit.plugin_id != id || entry.generation.is_some() {
                    bail!("launch integration permit does not match this pending launch")
                };
                entry.generation = Some(generation.into());
                entry.permit.clone()
            };
            let result = self.prepare_selected(plan, generation, &id, &permit).await;
            self.remove(&permit);
            if result.is_err() {
                let _ = self.host.check(&permit, true).await;
            }
            result.map_err(|error|anyhow!("[WKS_LAUNCH_INTEGRATION] Launch integration {id}: {error}. Restore the plugin/service or select None for a new launch."))
        })
    }
    fn revoke<'a>(&'a self, session: &'a str, generation: &'a str) -> Operation<'a, ()> {
        Box::pin(async move {
            let permit = {
                let mut pending = self.pending.lock().unwrap();
                if pending
                    .get(session)
                    .is_some_and(|entry| entry.generation.as_deref() == Some(generation))
                {
                    pending.remove(session).map(|entry| entry.permit)
                } else {
                    None
                }
            };
            if let Some(permit) = permit {
                let _ = self.host.check(&permit, true).await;
            }
            self.inner.revoke(session, generation).await
        })
    }
}
#[cfg(test)]
mod tests;

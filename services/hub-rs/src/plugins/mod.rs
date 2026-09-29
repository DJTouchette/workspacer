//! Trusted local plugins: persistent identities/settings and owned sidecars.
pub mod http;
pub mod install;
pub mod launch;
pub mod manifest;
pub mod settings;
pub mod supervisor;
use crate::{Caller, Handle, Options, protocol::Event};
use anyhow::{Result, anyhow, bail};
use manifest::Manifest;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use supervisor::{Factory, NativeFactory, Spec, Supervisor, Timing};
struct Loaded {
    manifest: Manifest,
    token: String,
    panes: Vec<String>,
    process: Option<Supervisor>,
}
pub struct Manager {
    root: PathBuf,
    hub: Handle,
    bus_url: String,
    plugins: BTreeMap<String, Loaded>,
    factory: Arc<dyn Factory>,
    stream_logs: bool,
    sidecar_node: Option<String>,
}
impl Manager {
    pub fn new(root: PathBuf, hub: Handle, bus_url: String) -> Self {
        Self {
            root,
            hub,
            bus_url,
            plugins: BTreeMap::new(),
            factory: Arc::new(NativeFactory),
            stream_logs: false,
            sidecar_node: None,
        }
    }
    pub fn set_sidecar_node(&mut self, runtime: Option<String>) {
        self.sidecar_node = runtime;
    }
    pub fn set_stream_logs(&mut self, enabled: bool) {
        self.stream_logs = enabled;
    }
    pub fn with_factory(mut self, factory: Arc<dyn Factory>) -> Self {
        self.factory = factory;
        self
    }
    pub async fn load(&mut self) -> Vec<String> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return vec![],
            Err(e) => return vec![e.to_string()],
        };
        let mut errors = vec![];
        for entry in entries {
            let result = async {
                let e = entry?;
                if e.file_name().to_string_lossy().starts_with('.') {
                    return Ok(());
                }
                let p = e.path().join("plugin.json");
                if !p.exists() {
                    return Ok(());
                }
                self.add(Manifest::load(&p)?).await
            }
            .await;
            if let Err(e) = result {
                errors.push(format!("{e:#}"));
            }
        }
        errors
    }
    pub async fn add(&mut self, m: Manifest) -> Result<()> {
        m.validate()?;
        // A dev manifest can change its id. A directory still has one process
        // owner: retire any old identity before registering its replacement.
        if let Ok(directory) = m.dir.canonicalize() {
            let aliases: Vec<_> = self
                .plugins
                .iter()
                .filter(|(id, loaded)| {
                    *id != &m.id
                        && loaded.manifest.dir.canonicalize().ok().as_ref() == Some(&directory)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in aliases {
                self.remove(&id).await?;
            }
        }
        // Same-id reload publishes loaded only, matching the legacy event API.
        self.remove_internal(&m.id, false).await?;
        let loaded = Loaded {
            manifest: m.clone(),
            token: if m.disabled {
                String::new()
            } else {
                token(&m.dir)?
            },
            panes: vec![],
            process: None,
        };
        let stable_token = loaded.token.clone();
        self.plugins.insert(m.id.clone(), loaded);
        if !m.disabled {
            self.hub
                .register_plugin(stable_token.clone(), m.id.clone(), m.provides.clone())
                .await?;
            if let Some(server) = &m.server {
                let mut env = BTreeMap::from([
                    ("HUB_TOKEN".into(), stable_token.clone()),
                    ("HUB_URL".into(), self.bus_url.clone()),
                    (
                        "WKS_SETTINGS".into(),
                        serde_json::to_string(&settings::merged(&m))?,
                    ),
                ]);
                env.insert("WORKSPACER_HUB_URL".into(), self.bus_url.clone());
                let (command, runtime_env) =
                    install::runtime_command(&server.command, self.sidecar_node.as_deref());
                env.extend(runtime_env);
                let hub = self.hub.clone();
                let id = m.id.clone();
                match Supervisor::start_observed(
                    Spec {
                        command,
                        args: server.args.iter().map(|a| install::platform(a)).collect(),
                        directory: m.dir.clone(),
                        health_url: (!server.health.is_empty() && server.port > 0)
                            .then(|| format!("http://127.0.0.1:{}{}", server.port, server.health)),
                        env,
                        log: self.stream_logs.then(|| {
                            let hub = self.hub.clone();
                            let id = m.id.clone();
                            Arc::new(move |stream: &str, line: &str| {
                                let _ = hub.publish(Event::new(
                                    "plugin.log",
                                    "supervisor",
                                    json!({"name":id,"stream":stream,"line":line}),
                                ));
                            }) as supervisor::LogSink
                        }),
                    },
                    self.factory.clone(),
                    Timing::default(),
                    Arc::new(move |status| {
                        let topic = format!("sidecar.{}", status.state.name());
                        let mut data = serde_json::to_value(status).expect("supervisor status");
                        data["name"] = id.clone().into();
                        let _ = hub.publish(Event::new(topic, "supervisor", data));
                    }),
                ) {
                    Ok(p) => self.plugins.get_mut(&m.id).unwrap().process = Some(p),
                    Err(e) => {
                        self.remove(&m.id).await?;
                        return Err(e);
                    }
                }
            }
        }
        let _ = self
            .hub
            .publish(Event::new("plugin.loaded", "hub", serde_json::to_value(m)?));
        Ok(())
    }
    pub async fn remove(&mut self, id: &str) -> Result<Option<PathBuf>> {
        self.remove_internal(id, true).await
    }
    async fn remove_internal(&mut self, id: &str, announce: bool) -> Result<Option<PathBuf>> {
        let Some(l) = self.plugins.get_mut(id) else {
            return Ok(None);
        };
        if let Some(p) = &mut l.process {
            p.stop();
        }
        // Retain tracked credentials until every revoke completes. If this
        // future is cancelled, the next remove/stop safely resumes cleanup.
        for token in l
            .panes
            .iter()
            .chain((!l.token.is_empty()).then_some(&l.token))
        {
            self.hub.revoke_plugin(token.clone()).await?;
        }
        let directory = self.plugins.remove(id).map(|l| l.manifest.dir);
        if announce {
            let _ = self
                .hub
                .publish(Event::new("plugin.unloaded", "hub", json!({"id":id})));
        }
        Ok(directory)
    }
    pub async fn stop(&mut self) -> Result<()> {
        let ids: Vec<_> = self.plugins.keys().cloned().collect();
        for id in ids {
            self.remove(&id).await?;
        }
        Ok(())
    }
    pub fn list(&self) -> Vec<Manifest> {
        self.plugins.values().map(|l| l.manifest.clone()).collect()
    }
    fn manifest(&self, id: &str) -> Result<Manifest> {
        self.plugins
            .get(id)
            .map(|l| l.manifest.clone())
            .ok_or_else(|| anyhow!("plugin {id:?} is not loaded"))
    }
    pub fn settings(&self, id: &str) -> Result<Value> {
        Ok(settings::redacted(&self.manifest(id)?))
    }
    pub async fn set_settings(
        &mut self,
        id: &str,
        values: &serde_json::Map<String, Value>,
    ) -> Result<Value> {
        let m = self.manifest(id)?;
        let result = settings::update(&m, values)?;
        self.hub.publish(Event::new(
            "plugin.settings.changed",
            "hub",
            json!({"id":id,"values":result}),
        ))?;
        if m.server.is_some() && !m.disabled {
            self.add(m).await?;
        }
        Ok(result)
    }
    pub async fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<Manifest> {
        let mut m = self.manifest(id)?;
        let path = m.dir.join(".disabled");
        if enabled {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        } else {
            std::fs::write(path, b"")?;
        }
        m.disabled = !enabled;
        self.add(m.clone()).await?;
        Ok(m)
    }
    pub async fn pane_token(&mut self, id: &str) -> Result<String> {
        let m = self.manifest(id)?;
        if m.disabled {
            bail!("plugin disabled")
        }
        let token = random_token();
        self.plugins.get_mut(id).unwrap().panes.push(token.clone());
        self.hub
            .register_plugin(token.clone(), id.into(), m.provides)
            .await?;
        Ok(token)
    }
    pub async fn revoke_pane(&mut self, token: &str) -> Result<()> {
        self.hub.revoke_plugin(token.into()).await?;
        for l in self.plugins.values_mut() {
            l.panes.retain(|t| t != token);
        }
        Ok(())
    }
    /// Canonicalize both roots so a symlink cannot serve credentials outside UI.
    pub fn ui_file(&self, id: &str, path: &str) -> Result<PathBuf> {
        let m = self.manifest(id)?;
        if m.disabled || m.ui.is_empty() {
            bail!("plugin UI unavailable")
        };
        let dir = m.dir.canonicalize()?;
        let root = dir.join(manifest::relative_path(&m.ui)?).canonicalize()?;
        if !root.starts_with(&dir) || root == dir {
            bail!("UI root escapes plugin")
        };
        let file = root
            .join(manifest::relative_path(if path.is_empty() {
                "index.html"
            } else {
                path
            })?)
            .canonicalize()?;
        if !file.starts_with(&root) || !file.is_file() {
            bail!("UI file escapes root")
        };
        Ok(file)
    }
    pub fn tools(&self) -> Value {
        json!(
            self.plugins
                .values()
                .filter(|l| !l.manifest.disabled && !l.manifest.tools.is_empty())
                .map(|l| json!({"pluginId":l.manifest.id,"tools":l.manifest.tools}))
                .collect::<Vec<_>>()
        )
    }
}
fn random_token() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
fn token(dir: &Path) -> Result<String> {
    let path = dir.join(".bus-token");
    if let Ok(s) = std::fs::read_to_string(&path) {
        if !s.trim().is_empty() {
            return Ok(s.trim().into());
        }
    }
    let token = random_token();
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(token.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(token)
}
pub type SharedManager = Arc<tokio::sync::Mutex<Manager>>;
pub fn handlers(mut options: Options, manager: SharedManager) -> Options {
    for method in [
        "plugins.list",
        "plugins.manifests",
        "plugins.tools",
        "plugins.settings",
        "plugins.setSettings",
        "plugins.setEnabled",
        "plugins.reload",
        "plugins.paneToken",
        "plugins.revokePaneToken",
    ] {
        let manager = manager.clone();
        options = options.handler(method, move |caller: Caller, params| {
            let manager = manager.clone();
            async move {
                let mut m = manager.lock().await;
                if method == "plugins.list" || method == "plugins.manifests" {
                    if !caller.authenticated_host {
                        bail!("full plugin manifests require host identity");
                    }
                    return Ok(serde_json::to_value(m.list())?);
                }
                if method == "plugins.tools" {
                    return Ok(m.tools());
                }
                let id = params
                    .get("pluginId")
                    .or_else(|| params.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !caller.authenticated_host
                    && !(method == "plugins.settings" && caller.plugin_id == id)
                {
                    bail!("plugin management requires host identity")
                }
                match method {
                    "plugins.settings" => m.settings(id),
                    "plugins.setSettings" => {
                        m.set_settings(
                            id,
                            params
                                .get("values")
                                .and_then(Value::as_object)
                                .ok_or_else(|| anyhow!("values must be object"))?,
                        )
                        .await
                    }
                    "plugins.setEnabled" => Ok(serde_json::to_value(
                        m.set_enabled(
                            id,
                            params
                                .get("enabled")
                                .and_then(Value::as_bool)
                                .ok_or_else(|| anyhow!("enabled must be boolean"))?,
                        )
                        .await?,
                    )?),
                    "plugins.paneToken" => Ok(json!(m.pane_token(id).await?)),
                    "plugins.revokePaneToken" => {
                        m.revoke_pane(
                            params
                                .get("token")
                                .and_then(Value::as_str)
                                .ok_or_else(|| anyhow!("token required"))?,
                        )
                        .await?;
                        Ok(Value::Null)
                    }
                    _ => {
                        let manifest = m.manifest(id)?;
                        m.add(Manifest::load(&manifest.dir.join("plugin.json"))?)
                            .await?;
                        Ok(Value::Null)
                    }
                }
            }
        });
    }
    options
}

//! Ownership for the complete local Rust process: claudemon engine and hub.
//! Both the GUI host and standalone executable can use this owner.
mod ownership;
use crate::{Handle, Hub, Options};
use anyhow::Result;
use claudemon::daemon::{
    ServeConfig,
    embedded::{EmbeddedDaemon, Options as EngineOptions},
};

/// External plugins and provider MCP tools need listeners even when the GUI
/// itself calls the backend in memory. Identity persists across GUI restarts.
pub fn configure_local_integrations(options: &mut Options) -> Result<()> {
    use std::io::Write;
    let directory = options
        .config_dir
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("local integrations require a config directory"))?;
    std::fs::create_dir_all(directory)?;
    let historical = crate::cli::config_directory().ok().and_then(|standard| {
        crate::services::state_paths::historical_directory(directory, &standard)
    });
    if options.push_dir.is_none() {
        options.push_dir = historical.clone();
    }
    if options.data_dir.is_none() {
        options.data_dir = Some(historical.unwrap_or_else(|| directory.clone()));
    }
    if options.jobs_file.is_none() {
        options.jobs_file = options
            .data_dir
            .as_ref()
            .map(|directory| directory.join("jobs.json"));
    }
    if options.token.is_empty() {
        let path = directory.join("remote-token");
        match std::fs::read_to_string(&path) {
            Ok(token) => options.token = token.trim().to_owned(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Preserve credentials from the earlier experimental Rust
                // mode; the canonical pairing identity is remote-token.
                let token = match std::fs::read_to_string(directory.join("hub.token")) {
                    Ok(token) => token.trim().to_owned(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        crate::cli::load_or_create_host_token(directory, false)?
                    }
                    Err(error) => return Err(error.into()),
                };
                anyhow::ensure!(
                    !token.is_empty(),
                    "existing host credential is empty; refusing to rotate identity"
                );
                let mut file = tempfile::NamedTempFile::new_in(directory)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.as_file()
                        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
                }
                writeln!(file, "{token}")?;
                file.as_file().sync_all()?;
                match file.persist_noclobber(&path) {
                    Ok(_) => options.token = token,
                    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                        options.token = std::fs::read_to_string(&path)?.trim().to_owned();
                    }
                    Err(error) => return Err(error.error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }
        anyhow::ensure!(
            !options.token.is_empty(),
            "existing host credential is empty; refusing to rotate identity"
        );
    }
    options.listen.get_or_insert("127.0.0.1:0".parse()?);
    options.mcp_listen.get_or_insert("127.0.0.1:0".parse()?);
    options
        .scoped_tokens
        .get_or_insert_with(|| directory.join("tokens.json"));
    options
        .plugins_dir
        .get_or_insert_with(|| directory.join("plugins"));
    Ok(())
}

/// Match native/launcher precedence without changing process-global env.
pub fn configured_usage_polling(config_dir: Option<&std::path::Path>) -> Option<bool> {
    let explicit_environment = std::env::var_os("WORKSPACER_USAGE_POLL_ON_BOOT").is_some();
    let raw = if explicit_environment {
        String::new()
    } else {
        std::fs::read_to_string(config_dir?.join("config.yaml")).ok()?
    };
    usage_poll_setting(&raw, explicit_environment)
}
fn usage_poll_setting(raw: &str, explicit_environment: bool) -> Option<bool> {
    if explicit_environment {
        return None;
    }
    let config: serde_yaml::Value = serde_yaml::from_str(raw).ok()?;
    config.get("usage")?.get("pollOnBoot")?.as_bool()
}

pub struct Backend {
    hub: Option<Hub>,
    engine: Option<EmbeddedDaemon>,
    database_lease: Option<ownership::DatabaseLease>,
}
impl Backend {
    pub async fn start(
        config: ServeConfig,
        engine_options: EngineOptions,
        options: Options,
    ) -> Result<Self> {
        let mut owner = Self::prepare(config, engine_options)?;
        if let Err(error) = owner.initialize(options).await {
            return match owner.shutdown().await {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(error.context(format!("backend startup cleanup also failed: {cleanup:#}")))
                }
            };
        }
        Ok(owner)
    }
    /// Keep ownership outside a cancellable readiness future. A host that
    /// cancels initialize can still explicitly join every resource it started.
    pub fn prepare(mut config: ServeConfig, engine_options: EngineOptions) -> Result<Self> {
        let database_lease = ownership::DatabaseLease::take(&config.db_path)?;
        if let Some(lease) = &database_lease {
            config.db_path = lease.database.clone();
        }
        Ok(Self {
            hub: None,
            engine: Some(EmbeddedDaemon::start_with_options(config, engine_options)?),
            database_lease,
        })
    }
    pub async fn initialize(&mut self, mut options: Options) -> Result<()> {
        anyhow::ensure!(self.hub.is_none(), "backend is already initialized");
        let started = std::time::Instant::now();
        let stage = |name: &str| {
            tracing::info!(
                stage = name,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "backend startup"
            )
        };
        let engine = self.engine.as_mut().expect("engine owner present");
        let endpoints = engine.ready().await?;
        stage("engine");
        if let Some(path) = options.claude_hook_settings.clone() {
            if let Err(error) =
                claudemon::daemon::init::run_at_with_port_quiet(path, endpoints.hook_addr.port())
                    .await
            {
                eprintln!("hook initialization failed: {error:#}; continuing");
            }
        }
        stage("hooks");
        options.engine = Some(engine.client());
        let wait_for_facade = options.mcp_listen.is_some();
        self.hub = Some(Hub::start(options)?);
        self.hub.as_ref().unwrap().ready().await?;
        stage("hub");
        if wait_for_facade {
            let handle = self.hub.as_ref().unwrap().handle();
            tokio::time::timeout(std::time::Duration::from_secs(60), async {
                loop {
                    if handle.health().await?["mcpReady"] == true {
                        return anyhow::Ok(());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            })
            .await
            .map_err(|_| anyhow::anyhow!("owned MCP facade catalog readiness timed out"))??;
            stage("facade");
        }
        Ok(())
    }
    pub fn handle(&self) -> Handle {
        self.hub.as_ref().expect("backend is running").handle()
    }
    /// Actual bound engine endpoints after readiness; no listener is inferred
    /// from configured ports or exposed as a caller-controlled capability.
    pub fn engine_endpoints(&self) -> Option<claudemon::daemon::embedded::ReadyInfo> {
        let status = self.engine.as_ref()?.client().status().borrow().clone();
        match status {
            claudemon::daemon::embedded::Status::Ready(ready) => Some(ready),
            _ => None,
        }
    }
    pub async fn shutdown(mut self) -> Result<()> {
        let hub_result = if let Some(hub) = self.hub.take() {
            match tokio::task::spawn_blocking(move || hub.shutdown()).await {
                Ok(result) => result,
                Err(error) => Err(error.into()),
            }
        } else {
            Ok(())
        };
        let engine_result = if let Some(engine) = self.engine.take() {
            engine.shutdown().await
        } else {
            Ok(())
        };
        if hub_result.is_ok() && engine_result.is_ok() {
            drop(self.database_lease.take());
        }
        // Drop deliberately retains the OS lock on an error/cancelled join.
        hub_result?;
        engine_result
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        // Admission must close before stopping the engine. GUI quit should use
        // explicit shutdown to await the daemon's PTY reaper as well.
        drop(self.hub.take());
        if let Some(engine) = self.engine.as_mut() {
            engine.request_shutdown();
        }
        if let Some(lease) = self.database_lease.take() {
            // A requested shutdown is not evidence that late engine/host work
            // finished. The OS releases this reservation when the process exits.
            std::mem::forget(lease);
        }
    }
}

/// Claudemon deliberately permits only one embedded engine per process.
#[cfg(test)]
pub(crate) static ENGINE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod usage_tests {
    use super::usage_poll_setting;
    #[test]
    fn idle_usage_polling_honors_config_without_overriding_explicit_environment() {
        for (raw, expected) in [
            ("usage:\n  pollOnBoot: false\n", Some(false)),
            ("usage:\n  pollOnBoot: true\n", Some(true)),
            ("", None),
            ("{", None),
            ("usage: false", None),
            ("usage:\n  pollOnBoot: 'false'\n", None),
            ("usage: {}", None),
        ] {
            assert_eq!(usage_poll_setting(raw, false), expected);
            assert_eq!(usage_poll_setting(raw, true), None);
        }
    }
}

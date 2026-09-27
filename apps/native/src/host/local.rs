use super::*;
use claudemon::daemon::{
    ServeConfig,
    embedded::{EmbeddedDaemon, Options as EngineOptions, Status as EngineStatus},
};
use serde::Deserialize;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, ChildStdin, Command as Process},
};

#[derive(Clone, Debug)]
pub struct LocalOptions {
    pub services_dir: Option<PathBuf>,
    pub database: PathBuf,
    pub hub_port: u16,
    pub mcp_port: u16,
    pub hook_port: u16,
    pub api_port: u16,
    pub no_plugins: bool,
}

impl LocalOptions {
    pub fn default_database() -> Result<PathBuf> {
        let dirs = directories::BaseDirs::new().context("cannot locate native data directory")?;
        Ok(dirs.data_local_dir().join("workspacer/native/state.db"))
    }
}

fn usage_poll_setting(raw: &str, explicit_environment: bool) -> Option<bool> {
    if explicit_environment {
        return None;
    }
    let value: serde_yaml::Value = serde_yaml::from_str(raw).ok()?;
    value.get("usage")?.get("pollOnBoot")?.as_bool()
}

fn configured_usage_polling() -> Option<bool> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    // Match the shared Go/desktop config root, including on macOS where
    // BaseDirs::config_dir() would instead choose Library/Application Support.
    let (variable, fallback) = if cfg!(target_os = "windows") {
        ("APPDATA", home.join("AppData/Roaming"))
    } else {
        ("XDG_CONFIG_HOME", home.join(".config"))
    };
    let base = std::env::var_os(variable)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or(fallback);
    let raw = std::fs::read_to_string(base.join("workspacer/config.yaml")).ok()?;
    usage_poll_setting(
        &raw,
        std::env::var_os("WORKSPACER_USAGE_POLL_ON_BOOT").is_some(),
    )
}

struct Binaries {
    launcher: PathBuf,
    hub: PathBuf,
    brain: PathBuf,
    mcp: PathBuf,
}
impl Binaries {
    fn find(options: &LocalOptions) -> Result<Self> {
        let find = |name: &str| -> Result<PathBuf> {
            let name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
            if let Some(dir) = &options.services_dir {
                let path = dir.join(&name);
                anyhow::ensure!(
                    path.is_file(),
                    "Missing {}. Build the service binaries or choose --services-dir.",
                    path.display()
                );
                return Ok(path);
            }
            let mut dirs = Vec::new();
            if let Some(parent) = std::env::current_exe()?.parent() {
                dirs.push(parent.to_owned());
            }
            dirs.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            dirs.into_iter()
                .map(|dir| dir.join(&name))
                .find(|path| path.is_file())
                .ok_or_else(|| {
                    anyhow!("Missing {name}. Build the service binaries and use --services-dir.")
                })
        };
        // Require the complete launch/tool stack; do not silently start reduced mode.
        Ok(Self {
            launcher: find("workspacer")?,
            hub: find("hub")?,
            brain: find("brain")?,
            mcp: find("mcp")?,
        })
    }
}

struct LocalServices {
    engine: Option<EmbeddedDaemon>,
    gateway: Option<OwnedGateway>,
}

// Child::wait closes Child.stdin before waiting. Keep the parentwatch pipe
// outside Child so observing process exit cannot accidentally request shutdown.
struct OwnedGateway {
    child: Child,
    stdin: Option<ChildStdin>,
}
impl OwnedGateway {
    fn spawn(process: &mut Process) -> Result<Self> {
        let mut child = process.stdin(Stdio::piped()).kill_on_drop(true).spawn()?;
        let stdin = child.stdin.take();
        Ok(Self { child, stdin })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Banner {
    bus_url: String,
    claudemon_url: String,
    token: String,
}

impl LocalServices {
    fn new() -> Self {
        Self {
            engine: None,
            gateway: None,
        }
    }

    async fn initialize(
        &mut self,
        options: &LocalOptions,
    ) -> Result<(Backend, async_channel::Receiver<crate::bus::Event>, Ready)> {
        anyhow::ensure!(
            options.hub_port != 0 && options.mcp_port != 0 && options.hub_port != options.mcp_port,
            "Choose distinct nonzero hub and MCP ports"
        );
        let bins = Binaries::find(options)?;
        // Refuse an incumbent before touching the engine database or workers.
        // The supervisor repeats this check to cover the subsequent bind race.
        for port in [options.hub_port, options.mcp_port] {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await
                .with_context(|| format!("Port {port} is occupied. Use --bus to attach to an existing hub, or choose separate local ports and a database"))?;
            drop(listener);
        }
        self.engine = Some(EmbeddedDaemon::start_with_options(
            ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: options.hook_port,
                api_port: options.api_port,
                db_path: options.database.clone(),
            },
            EngineOptions {
                usage_poll_on_boot: configured_usage_polling(),
            },
        )?);
        let engine = self.engine.as_mut().expect("engine started");
        let endpoints = engine.ready().await?;
        let mut process = Process::new(&bins.launcher);
        process
            .arg("serve")
            .arg("--external-claudemon")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--claudemon-api-port")
            .arg(endpoints.api_addr.port().to_string())
            .arg("--claudemon-hook-port")
            .arg(endpoints.hook_addr.port().to_string())
            .arg("--hub-port")
            .arg(options.hub_port.to_string())
            .arg("--mcp-port")
            .arg(options.mcp_port.to_string())
            .arg("--hub-bin")
            .arg(&bins.hub)
            .arg("--brain-bin")
            .arg(&bins.brain)
            .arg("--mcp-bin")
            .arg(&bins.mcp)
            .arg("--json")
            .env("WORKSPACER_PARENT_PID", std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if options.no_plugins {
            process.arg("--plugins-dir").arg("");
        }
        self.gateway =
            Some(OwnedGateway::spawn(&mut process).context("starting native hub services")?);
        let gateway = self.gateway.as_mut().expect("gateway started");
        let stdout = gateway
            .child
            .stdout
            .take()
            .context("hub ready stream missing")?;
        let mut stdout = BufReader::new(stdout).take(65536);
        let mut line = String::new();
        stdout
            .read_line(&mut line)
            .await
            .context("reading hub readiness")?;
        anyhow::ensure!(
            line.ends_with('\n'),
            "Hub services exited before readiness; see service diagnostics"
        );
        // Never include this line in diagnostics: it contains the host token.
        let banner: Banner =
            serde_json::from_str(&line).map_err(|_| anyhow!("Invalid hub readiness response"))?;
        let bus_url = format!("ws://127.0.0.1:{}/bus", options.hub_port);
        let engine_api = format!("http://{}", endpoints.api_addr);
        anyhow::ensure!(
            banner.bus_url == bus_url
                && banner.claudemon_url == engine_api
                && !banner.token.is_empty(),
            "Hub services did not attach to the expected embedded runtime"
        );
        let (backend, events) = Backend::connect(Config::new(bus_url.clone(), Some(banner.token))?);
        let backend = backend.with_local(engine.client());
        // The launcher banner precedes brain registration. Wait for the real
        // session capability, not merely an open WebSocket listener.
        loop {
            if let Some(exit) = gateway.child.try_wait()? {
                anyhow::bail!("Hub services exited during startup: {exit}");
            }
            if backend.snapshots().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok((
            backend,
            events,
            Ready {
                bus_url,
                engine_api: Some(engine_api),
                engine_hook: Some(format!("http://{}", endpoints.hook_addr)),
            },
        ))
    }

    async fn shutdown(&mut self) -> Result<()> {
        let mut failure = None;
        if let Some(mut gateway) = self.gateway.take() {
            // Parentwatch turns stdin EOF into ordered brain/facade/hub shutdown.
            drop(gateway.stdin.take());
            match tokio::time::timeout(Duration::from_secs(20), gateway.child.wait()).await {
                Ok(Ok(_)) => {}
                result => {
                    let _ = gateway.child.start_kill();
                    let _ =
                        tokio::time::timeout(Duration::from_secs(5), gateway.child.wait()).await;
                    failure = Some(anyhow!(
                        "Hub services did not shut down cleanly: {result:?}"
                    ));
                }
            }
        }
        if let Some(engine) = self.engine.take() {
            if let Err(error) = engine.shutdown().await {
                failure = Some(error.context("stopping embedded engine"));
            }
        }
        if let Some(error) = failure {
            Err(error)
        } else {
            Ok(())
        }
    }
}

pub(super) async fn run(
    options: LocalOptions,
    commands: mpsc::Receiver<Command>,
    views: watch::Sender<Arc<View>>,
    status: watch::Sender<Status>,
    mut stopping: oneshot::Receiver<()>,
) -> Result<()> {
    views.send_replace(Arc::new(View {
        notice: "Starting local engine and hub services…".into(),
        ..Default::default()
    }));
    let mut services = LocalServices::new();
    let result = async {
        let ready = tokio::select! {
            _ = &mut stopping => return Ok(()),
            ready = tokio::time::timeout(Duration::from_secs(90), services.initialize(&options)) =>
                ready.context("Local backend startup timed out")??,
        };
        let (backend, events, info) = ready;
        status.send_replace(Status::Ready(info));
        let mut engine_status = services.engine.as_ref().expect("ready engine").client().status();
        let engine_ended = async {
            loop {
                let current = engine_status.borrow_and_update().clone();
                match current {
                    EngineStatus::Failed(error) => return Err(anyhow!("Embedded engine failed: {error}")),
                    EngineStatus::Stopped => return Err(anyhow!("Embedded engine stopped unexpectedly")),
                    _ => {},
                }
                engine_status.changed().await.context("Embedded engine closed")?;
            }
        };
        tokio::select! {
            _ = &mut stopping => Ok(()),
            _ = Controller::run(backend, events, commands, views.clone()) => Ok(()),
            result = engine_ended => result,
            exit = services.gateway.as_mut().expect("ready gateway").child.wait() => Err(anyhow!("Hub services stopped unexpectedly: {exit:?}")),
        }
    }.await;
    let cleanup = services.shutdown().await;
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("Local backend cleanup also failed: {cleanup:#}")))
        }
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn watching_gateway_exit_keeps_parentwatch_stdin_open() {
        let mut process = Process::new("/bin/sh");
        process.args(["-c", "cat >/dev/null"]);
        let mut gateway = OwnedGateway::spawn(&mut process).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), gateway.child.wait())
                .await
                .is_err()
        );
        drop(gateway.stdin.take());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), gateway.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn idle_usage_polling_honors_config_without_overriding_explicit_environment() {
        assert_eq!(
            usage_poll_setting("usage:\n  pollOnBoot: false\n", false),
            Some(false)
        );
        assert_eq!(
            usage_poll_setting("usage:\n  pollOnBoot: true\n", false),
            Some(true)
        );
        for raw in [
            "",
            "{",
            "usage: false",
            "usage:\n  pollOnBoot: 'false'\n",
            "usage: {}",
        ] {
            assert_eq!(usage_poll_setting(raw, false), None);
        }
        assert_eq!(
            usage_poll_setting("usage:\n  pollOnBoot: true\n", true),
            None
        );
        assert_eq!(
            usage_poll_setting("usage:\n  pollOnBoot: false\n", true),
            None
        );
    }
    #[test]
    fn missing_service_bundle_fails_before_starting_an_engine() {
        let options = LocalOptions {
            services_dir: Some(std::path::Path::new("/nonexistent/native-services").into()),
            database: PathBuf::from("unused.db"),
            hub_port: 7895,
            mcp_port: 7897,
            hook_port: 0,
            api_port: 0,
            no_plugins: true,
        };
        assert!(Binaries::find(&options).is_err());
    }
}

use super::*;
use std::path::PathBuf;

/// Backend stores are isolated from an existing Go stack. Provider accounts
/// and the owner's manual Claude hook integration still use their chosen home.
#[derive(Clone, Debug)]
pub struct RustOptions {
    pub database: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub home_dir: PathBuf,
    /// An explicit host override; None follows the shared config/environment.
    pub usage_poll_on_boot: Option<bool>,
}

impl RustOptions {
    pub fn default_directory() -> Result<PathBuf> {
        let base = directories::BaseDirs::new().context("Cannot locate native data directory")?;
        Ok(base.data_local_dir().join("Workspacer Native Rust Preview"))
    }
    pub fn isolated(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        let directory = std::fs::canonicalize(directory)?;
        Ok(Self {
            database: directory.join("state.db"),
            config_dir: directory.join("config"),
            data_dir: directory.join("hub"),
            home_dir: directories::BaseDirs::new()
                .context("Cannot locate home directory")?
                .home_dir()
                .into(),
            usage_poll_on_boot: None,
        })
    }
}

pub(super) async fn run(
    options: RustOptions,
    commands: mpsc::Receiver<Command>,
    views: watch::Sender<Arc<View>>,
    status: watch::Sender<Status>,
    mut stopping: oneshot::Receiver<()>,
) -> Result<()> {
    views.send_replace(Arc::new(View {
        notice: "Starting Rust backend…".into(),
        ..Default::default()
    }));
    let mut hub_options = workspacer_hub::Options::default();
    hub_options.config_dir = Some(options.config_dir);
    hub_options.peers_file = hub_options
        .config_dir
        .as_ref()
        .map(|dir| dir.join("peers.json"));
    hub_options.data_dir = Some(options.data_dir);
    hub_options.jobs_file = hub_options
        .data_dir
        .as_ref()
        .map(|directory| directory.join("jobs.json"));
    // Like the previous native launcher, the desktop owner opts into the
    // user's manual/PTY Claude hook integration. Backend stores stay isolated.
    hub_options.claude_hook_settings = Some(options.home_dir.join(".claude/settings.json"));
    hub_options.home_dir = Some(options.home_dir);
    workspacer_hub::backend::configure_local_integrations(&mut hub_options)?;
    let usage_poll_on_boot = options.usage_poll_on_boot.or_else(|| {
        workspacer_hub::backend::configured_usage_polling(hub_options.config_dir.as_deref())
    });
    let mut owner = workspacer_hub::backend::Backend::prepare(
        claudemon::daemon::ServeConfig {
            host: "127.0.0.1".into(),
            hook_port: 0,
            api_port: 0,
            db_path: options.database,
        },
        claudemon::daemon::embedded::Options { usage_poll_on_boot },
    )?;
    let result=async {
        tokio::select! {
            _=&mut stopping=>return Ok(()),
            ready=tokio::time::timeout(std::time::Duration::from_secs(90),owner.initialize(hub_options))=>ready.context("Rust backend startup timed out")??,
        }
        let handle = owner.handle();
        let (backend,events)=Backend::in_process(&handle).await?;
        let engine=owner.engine_endpoints().context("Owned engine stopped before native readiness")?;
        let mut owned_listeners=vec![engine.api_addr,engine.hook_addr];
        if let workspacer_hub::Status::Ready{address,mcp_address,..}=handle.status().borrow().clone(){owned_listeners.extend(address);owned_listeners.extend(mcp_address);}
        else{anyhow::bail!("Owned hub stopped before native readiness");}
        status.send_replace(Status::Ready(Ready {bus_url:"in-process".into(),engine_api:Some(format!("http://{}",engine.api_addr)),engine_hook:Some(format!("http://{}",engine.hook_addr)),owned_listeners}));
        let mut hub_status=handle.status();
        tokio::select! {
            _=&mut stopping=>Ok(()),
            _=Controller::run(backend,events,commands,views.clone())=>Ok(()),
            result=async {loop{
                match hub_status.borrow_and_update().clone(){
                    workspacer_hub::Status::Failed(error)=>return Err(anyhow!("Rust hub failed: {error}")),
                    workspacer_hub::Status::Stopped=>return Err(anyhow!("Rust hub stopped unexpectedly")),_=>(),
                }
                hub_status.changed().await.context("Rust hub status closed")?;
            }}=>result,
        }
    }.await;
    let cleanup = owner.shutdown().await;
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("Rust backend cleanup also failed: {cleanup:#}")))
        }
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

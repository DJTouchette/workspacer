use super::{CommandLine, ServeArgs, print_json};
use anyhow::{Result, bail};
use serde_json::json;
use std::{
    io::Write,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Debug)]
pub struct ServePlan {
    pub hub_only: bool,
    pub config: PathBuf,
    pub home: PathBuf,
    pub database: PathBuf,
    pub listen: SocketAddr,
    pub api_port: u16,
    pub hook_port: u16,
    pub mcp: Option<SocketAddr>,
}
pub fn plan_serve(args: &CommandLine, serve: &ServeArgs) -> Result<ServePlan> {
    if !serve.hub_only
        && (serve.claudemon_api_port != 7891 || serve.claudemon_hook_port != 7890)
        && serve.claudemon_db_path.is_none()
    {
        bail!(
            "alternate claudemon ports require explicit --claudemon-db-path; refusing accidental shared database"
        )
    }
    if !serve.hub_only && (serve.claudemon_api_port == 0 || serve.claudemon_hook_port == 0) {
        bail!(
            "launcher claudemon ports must be nonzero so hook configuration names a stable endpoint"
        )
    }
    let host: IpAddr = if args.host == "localhost" {
        "127.0.0.1".parse()?
    } else {
        args.host.trim_matches(['[', ']']).parse()?
    };
    anyhow::ensure!(
        !serve.hub_only || serve.upstream.is_none(),
        "hub-only and upstream worker modes are separate roles"
    );
    anyhow::ensure!(
        serve.upstream.is_none()
            || serve.provider_scope != crate::provider_relay::Scope::Full
            || !serve.no_mcp,
        "full upstream workers require the local MCP facade"
    );
    anyhow::ensure!(
        serve.upstream.is_none()
            || (serve.jobs_file.is_none()
                && serve.peers_file.is_none()
                && serve.plugins_dir.is_none()
                && serve.examples_dir.is_none()
                && serve.nodes_file.is_none()
                && serve.push_dir.is_none()),
        "upstream worker mode uses central jobs, peers, plugins, nodes and push; configure those on the hub"
    );
    Ok(ServePlan {
        hub_only: serve.hub_only,
        config: args.directory()?,
        home: match &serve.home_dir {
            Some(home) => home.clone(),
            None if serve.hub_only => super::identity::home_directory().unwrap_or_default(),
            None => super::identity::home_directory()?,
        },
        database: serve
            .claudemon_db_path
            .clone()
            .unwrap_or_else(claudemon::store::default_db_path),
        listen: SocketAddr::new(host, args.hub_port),
        api_port: serve.claudemon_api_port,
        hook_port: serve.claudemon_hook_port,
        mcp: (!serve.no_mcp).then(|| SocketAddr::from(([127, 0, 0, 1], serve.mcp_port))),
    })
}
fn push_directory(selected: &Path, explicit: Option<&Path>) -> PathBuf {
    if let Some(explicit) = explicit {
        return explicit.to_path_buf();
    }
    if let Ok(standard) = super::identity::config_directory() {
        if let Some(historical) =
            crate::services::state_paths::historical_directory(selected, &standard)
        {
            return historical;
        }
    }
    selected.to_path_buf()
}
fn discover_webapp() -> Option<PathBuf> {
    let binary = std::env::current_exe().ok()?.canonicalize().ok()?;
    let directory = binary.parent()?;
    [directory.join("web"), directory.join("../web")]
        .into_iter()
        .find(|path| path.join("index.html").is_file())
}
fn discover_examples() -> Option<PathBuf> {
    if let Ok(executable) = std::env::current_exe().and_then(|path| path.canonicalize()) {
        if let Some(parent) = executable.parent() {
            let path = parent.join("examples");
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    let path = PathBuf::from("/usr/local/share/workspacer/examples");
    path.is_dir().then_some(path)
}
fn preflight(plan: &ServePlan) -> Result<()> {
    let mut addresses = vec![plan.listen];
    if !plan.hub_only {
        addresses.extend([
            SocketAddr::from(([127, 0, 0, 1], plan.api_port)),
            SocketAddr::from(([127, 0, 0, 1], plan.hook_port)),
        ]);
    }
    addresses.extend(plan.mcp);
    let mut listeners = vec![];
    for address in addresses {
        listeners.push(std::net::TcpListener::bind(address).map_err(|error| {
            anyhow::anyhow!("cannot bind {address}: {error}; refusing to kill an existing service")
        })?);
    }
    Ok(())
}
enum Owner {
    Hub(Option<crate::Hub>),
    Backend(crate::backend::Backend),
}
impl Owner {
    async fn initialize(&mut self, options: crate::Options) -> Result<()> {
        match self {
            Self::Hub(hub) => {
                *hub = Some(crate::Hub::start(options)?);
                hub.as_ref().unwrap().ready().await?;
                Ok(())
            }
            Self::Backend(backend) => backend.initialize(options).await,
        }
    }
    fn handle(&self) -> crate::Handle {
        match self {
            Self::Hub(hub) => hub.as_ref().unwrap().handle(),
            Self::Backend(backend) => backend.handle(),
        }
    }
    async fn shutdown(self) -> Result<()> {
        match self {
            Self::Hub(Some(hub)) => tokio::task::spawn_blocking(move || hub.shutdown()).await?,
            Self::Hub(None) => Ok(()),
            Self::Backend(backend) => backend.shutdown().await,
        }
    }
}
fn secret_file(path: &Path) -> Result<String> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| anyhow::anyhow!("upstream credential file is unreadable"))?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= 64 * 1024,
        "upstream credential file is invalid"
    );
    let token = std::fs::read_to_string(path)
        .map_err(|_| anyhow::anyhow!("upstream credential file is unreadable"))?
        .trim()
        .into();
    Ok(token)
}
fn relay_config(serve: &ServeArgs) -> Result<Option<crate::provider_relay::Config>> {
    let Some(url) = &serve.upstream else {
        return Ok(None);
    };
    let token = match &serve.upstream_token_file {
        Some(path) => secret_file(path)?,
        None => std::env::var("HUB_TOKEN").unwrap_or_default(),
    };
    let caller_token = match &serve.upstream_caller_token_file {
        Some(path) => Some(secret_file(path)?),
        None => std::env::var("WKS_MCP_HUB_TOKEN")
            .ok()
            .filter(|token| !token.is_empty()),
    };
    let config = crate::provider_relay::Config {
        url: url.clone(),
        token,
        scope: serve.provider_scope,
        node_id: if serve.node_id.is_empty() {
            std::env::var("WKS_NODE_ID")
                .unwrap_or_default()
                .trim()
                .into()
        } else {
            serve.node_id.clone()
        },
        caller_token,
        last_exit_file: std::env::var_os("WKS_LAST_EXIT_FILE")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("WKS_DATA")
                    .filter(|path| !path.is_empty())
                    .map(|path| PathBuf::from(path).join("state/last-exit.json"))
            }),
    };
    config.validate()?;
    Ok(Some(config))
}
pub(super) async fn run(
    args: &CommandLine,
    serve: &ServeArgs,
    dev: Option<(PathBuf, bool, u64)>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32> {
    let plan = plan_serve(args, serve)?;
    preflight(&plan)?;
    let relay = relay_config(serve)?;
    let supplied = if relay.is_some() {
        args.token.clone().unwrap_or_default()
    } else {
        args.credential()?
    };
    let token = if supplied.is_empty() {
        super::load_or_create_host_token(
            &plan.config,
            serve.allow_new_token
                || std::env::var("WORKSPACER_ALLOW_NEW_TOKEN").is_ok_and(|v| v == "1"),
        )?
    } else {
        supplied
    };
    let temporary = if dev.is_some() {
        Some(
            tempfile::Builder::new()
                .prefix("wks-plugin-dev-")
                .tempdir()?,
        )
    } else {
        None
    };
    let mut options = crate::Options::default();
    options.external_claudemon_url = serve.external_claudemon.clone();
    options.mcp_static_token = match &serve.mcp_token {
        Some(token) => Some(token.clone()),
        None => std::env::var_os("WKS_MCP_TOKEN")
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("WKS_MCP_TOKEN must contain valid text"))
            })
            .transpose()?,
    };
    options.mcp_untokened =
        untokened_override(serve.untokened, std::env::var_os("WKS_MCP_UNTOKENED"))?;
    options.network_admin_socket = std::env::var_os("WKS_NETWORK_ADMIN_SOCKET")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    options.network_admin_token_file = std::env::var_os("WKS_NETWORK_ADMIN_TOKEN_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    options.uploads_to_worker = serve.uploads_to_worker
        || std::env::var("WKS_UPLOAD_PROVIDER").is_ok_and(|value| value == "worker");
    options.machine_power_provider = if relay.is_some() {
        None
    } else {
        match crate::services::machine_power::standalone_from_environment() {
            Ok(provider) => provider,
            Err(_) => {
                writeln!(err, "Machine power configuration invalid; stop disabled")?;
                None
            }
        }
    };
    options.control_plane_only = serve.hub_only;
    options.provider_relay = relay;
    options.nodes_file = serve.nodes_file.clone();
    options.nodes_keep_failed_wakes_running = serve.nodes_keep_failed_wakes_running;
    options.listen = Some(plan.listen);
    options.mcp_listen = plan.mcp;
    options.token = token.clone();
    options.config_dir = Some(plan.config.clone());
    options.scoped_tokens = Some(args.tokens_path()?);
    let hub_state = serve.data_dir.clone().unwrap_or_else(|| {
        super::config_directory()
            .ok()
            .and_then(|standard| {
                crate::services::state_paths::historical_directory(&plan.config, &standard)
            })
            .unwrap_or_else(|| plan.config.clone())
    });
    options.data_dir = Some(hub_state.clone());
    options.home_dir = (!plan.home.as_os_str().is_empty()).then(|| plan.home.clone());
    options.jobs_file = Some(
        serve
            .jobs_file
            .clone()
            .unwrap_or(hub_state.join("jobs.json")),
    );
    if serve.no_jobs {
        options.jobs_file = None;
    }
    options.peers_file = Some(
        serve
            .peers_file
            .clone()
            .unwrap_or(plan.config.join("peers.json")),
    );
    options.plugin_origin = serve.plugin_origin.clone();
    options.trusted_hosts = if serve.trusted_host.is_empty() {
        std::env::var("HUB_TRUSTED_HOSTS")
            .ok()
            .into_iter()
            .flat_map(|hosts| hosts.split(',').map(str::to_string).collect::<Vec<_>>())
            .collect()
    } else {
        serve.trusted_host.clone()
    };
    options.webapp_dir = serve
        .webapp_dir
        .clone()
        .or_else(|| {
            std::env::var_os("WORKSPACER_WEBAPP_DIR")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
        .or_else(discover_webapp);
    options.push_dir = Some(push_directory(&plan.config, serve.push_dir.as_deref()));
    options.plugin_examples_dir = serve.examples_dir.clone().or_else(discover_examples);
    options.plugins_stream_logs = dev.is_some();
    options.sidecar_node = serve
        .sidecar_node
        .clone()
        .or_else(|| std::env::var("WORKSPACER_SIDECAR_NODE").ok())
        .filter(|s| !s.is_empty());
    options.plugins_dir = temporary
        .as_ref()
        .map(|dir| dir.path().to_path_buf())
        .or_else(|| {
            Some(
                serve
                    .plugins_dir
                    .clone()
                    .unwrap_or(plan.config.join("plugins")),
            )
        })
        .filter(|p| !p.as_os_str().is_empty());
    if serve.upstream.is_some() {
        options.jobs_file = None;
        options.plugins_dir = None;
        options.plugin_examples_dir = None;
        options.push_dir = Some(PathBuf::new());
        options.nodes_file = Some(PathBuf::new());
        options.peers_file = None;
    }
    let plugin_root = options.plugins_dir.clone();
    if let Some((source, _, _)) = &dev {
        crate::plugins::manifest::Manifest::load(&source.join("plugin.json"))?;
        options.plugin_examples_dir = None;
    }
    if !serve.hub_only && !serve.no_claudemon_init {
        match tokio::time::timeout(
            Duration::from_secs(10),
            claudemon::daemon::init::run_with_port_quiet(false, plan.hook_port),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => writeln!(err, "hook initialization failed: {error:#}; continuing")?,
            Err(_) => writeln!(err, "hook initialization timed out; continuing")?,
        }
    }
    let engine_options = claudemon::daemon::embedded::Options {
        usage_poll_on_boot: crate::backend::configured_usage_polling(Some(&plan.config)),
    };
    let mut shutdown_signal = Box::pin(signal());
    let mut parent_signal = Box::pin(super::parent::gone());
    let mut backend = if serve.hub_only {
        Owner::Hub(None)
    } else {
        Owner::Backend(crate::backend::Backend::prepare(
            claudemon::daemon::ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: plan.hook_port,
                api_port: plan.api_port,
                db_path: plan.database.clone(),
            },
            engine_options,
        )?)
    };
    let initialized = tokio::select! {
        result=tokio::time::timeout(Duration::from_secs(20),backend.initialize(options))=>result.map_err(|_|anyhow::anyhow!("backend readiness timed out")).and_then(|result|result),
        result=&mut shutdown_signal=>{let cleanup=backend.shutdown().await;return result.and(cleanup).map(|_|0)},
        result=&mut parent_signal=>{let cleanup=backend.shutdown().await;return result.and(cleanup).map(|_|0)},
    };
    if let Err(error) = initialized {
        let _ = backend.shutdown().await;
        return Err(error);
    }
    let handle = backend.handle();
    let snapshot = handle.status().borrow().clone();
    let (address, mcp) = match snapshot {
        crate::Status::Ready {
            address: Some(address),
            mcp_address,
        } => (address, mcp_address),
        _ => {
            let _ = backend.shutdown().await;
            bail!("backend has no ready listener")
        }
    };
    let local_address = crate::net_address::dial_addr(address);
    let interfaces = crate::net_address::local_ipv4s();
    let advertised = crate::net_address::advertise_addr(address, &interfaces);
    let advertised_mcp =
        mcp.map(|address| crate::net_address::advertise_addr(address, &interfaces));
    let result:Result<()>=async {
    if let Some(mcp)=mcp {tokio::select!{result=wait_mcp(mcp)=>result?,result=&mut shutdown_signal=>return result,result=&mut parent_signal=>return result}}

    if let Some((source,_,_))=&dev {reload(&format!("http://{local_address}"),&token,source).await?;}
    let banner = json!({"service":"workspacer-rust","hubUrl":format!("http://{advertised}"),"busUrl":format!("ws://{advertised}/bus"),"mobileUrl":format!("http://{advertised}/m"),"remoteUrl":format!("http://{advertised}/remote"),"claudemonUrl":(!serve.hub_only).then(||format!("http://127.0.0.1:{}",plan.api_port)),"mcpUrl":advertised_mcp.map(|address|format!("http://{address}/mcp")),"token":token,"database":(!serve.hub_only).then_some(&plan.database),"mode":if serve.hub_only{"hub-only"}else if serve.upstream.is_some(){"worker"}else{"standalone"},"migrationComplete":false});
    if serve.quiet {} else if args.json {
        print_json(out, &banner)?
    } else {
        writeln!(
            out,
            "Workspacer Rust ready\nHub: ws://{advertised}/bus\nMobile: http://{advertised}/m\nPairing token: {token}"
        )?;
        if let Some(mcp) = advertised_mcp {
            writeln!(out, "MCP: http://{mcp}/mcp")?
        }
    }
    let mut status = handle.status();
    let ended = async {
        loop {
            match status.borrow_and_update().clone() {
                crate::Status::Failed(error) => bail!("backend failed: {error}"),
                crate::Status::Stopped => bail!("backend stopped unexpectedly"),
                _ => {}
            }
            status.changed().await?;
        }
    };
    let watch = async {
        if let Some((source, build, poll)) = dev {
            watch_plugin(
                &source,
                build,
                poll,
                plugin_root.as_ref().unwrap(),
                &format!("http://{local_address}"),
                &token,
                err,
                serve.sidecar_node.clone().or_else(|| std::env::var("WORKSPACER_SIDECAR_NODE").ok()),
            )
            .await
        } else {
            std::future::pending::<Result<()>>().await
        }
    };
    tokio::select! {result=&mut shutdown_signal=>result,result=&mut parent_signal=>result,result=ended=>result,result=watch=>result}
    }.await;
    let cleanup =
        tokio::select! {result=backend.shutdown()=>result,_=signal()=>std::process::exit(130)};
    drop(temporary);
    result.and(cleanup)?;
    Ok(0)
}
async fn signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {result=tokio::signal::ctrl_c()=>result?,_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
async fn build_dev(source: &Path, runtime: Option<String>) -> Result<()> {
    let source = source.to_path_buf();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _guard = CancelBuild(cancel.clone());
    tokio::task::spawn_blocking(move || {
        crate::plugins::install::build_in_place_with_runtime(&source, &cancel, runtime.as_deref())
    })
    .await?
}
struct CancelBuild(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelBuild {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}
async fn reload(base: &str, token: &str, source: &Path) -> Result<()> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?
        .post(format!("{base}/plugins/reload"))
        .bearer_auth(token)
        .json(&json!({"dir":source.canonicalize()?}))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
async fn wait_mcp(address: SocketAddr) -> Result<()> {
    let address = crate::net_address::dial_addr(address);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(response) = client.get(format!("http://{address}/health")).send().await {
                if let Ok(value) = response.json::<serde_json::Value>().await {
                    if value["hubConnected"] == true && value["pluginCatalogReady"] == true {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("MCP initial plugin catalog readiness timed out"))?;
    Ok(())
}
fn stamp(root: &Path) -> Result<Vec<(PathBuf, u64, std::time::SystemTime)>> {
    fn walk(root: &Path, out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>) -> Result<()> {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Ok(());
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            if [
                ".git",
                "node_modules",
                "target",
                ".bus-token",
                ".settings.json",
            ]
            .iter()
            .any(|s| name == *s)
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                walk(&entry.path(), out)?
            } else if meta.is_file() {
                out.push((entry.path(), meta.len(), meta.modified()?));
            }
        }
        Ok(())
    }
    let mut out = vec![];
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}
async fn watch_plugin(
    source: &Path,
    build: bool,
    poll: u64,
    _root: &Path,
    base: &str,
    token: &str,
    err: &mut dyn Write,
    runtime: Option<String>,
) -> Result<()> {
    let bus = crate::client::Client::connect_remote(
        &format!("{}/bus", base.replacen("http", "ws", 1)),
        token,
    )
    .await?;
    let mut events = bus.events();
    bus.topics(
        ["plugin.*".to_owned(), "sidecar.*".to_owned()]
            .into_iter()
            .collect(),
    )
    .await?;
    let mut previous = stamp(source)?;
    let mut interval = tokio::time::interval(Duration::from_millis(poll.max(50)));
    loop {
        tokio::select! {
            event=events.recv()=>match event{
                Ok(event)=>{let data=event.data.unwrap_or_default();if event.topic=="plugin.log"{writeln!(err,"[{}] {}",data["name"].as_str().unwrap_or("plugin"),data["line"].as_str().unwrap_or(""))?;}else if event.topic.starts_with("sidecar."){writeln!(err,"[{}] {} {}",data["name"].as_str().unwrap_or("plugin"),event.topic,data["err"].as_str().unwrap_or(""))?;}},
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{},Err(_)=>bail!("plugin event stream closed"),
            },
            _=interval.tick()=>{
                let current=stamp(source)?;if current==previous{continue}previous=current;
                if build {if let Err(error)=build_dev(source, runtime.clone()).await{writeln!(err,"plugin build failed; keeping current sidecar: {error:#}")?;continue}}
                if let Err(error)=reload(base,token,source).await{writeln!(err,"plugin reload failed: {error:#}")?;}else{writeln!(err,"reloaded {}",source.display())?;}previous=stamp(source)?;
            }
        }
    }
}

fn untokened_override(
    flag: Option<crate::mcp::UntokenedAccess>,
    environment: Option<std::ffi::OsString>,
) -> Result<Option<crate::mcp::UntokenedAccess>> {
    if flag.is_some() {
        return Ok(flag);
    }
    let Some(value) = environment.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let value = value
        .into_string()
        .map_err(|_| anyhow::anyhow!("WKS_MCP_UNTOKENED must be deny, view, or operator"))?;
    Ok(Some(value.parse()?))
}
#[cfg(test)]
mod access_tests {
    #[test]
    fn untokened_override_is_explicit_and_invalid_environment_refuses() {
        use crate::mcp::UntokenedAccess::{Deny, View};
        assert_eq!(super::untokened_override(None, None).unwrap(), None);
        assert_eq!(
            super::untokened_override(None, Some("".into())).unwrap(),
            None
        );
        assert_eq!(
            super::untokened_override(None, Some("view".into())).unwrap(),
            Some(View)
        );
        assert_eq!(
            super::untokened_override(Some(Deny), Some("operator".into())).unwrap(),
            Some(Deny)
        );
        assert!(super::untokened_override(None, Some("yes".into())).is_err());
    }
}

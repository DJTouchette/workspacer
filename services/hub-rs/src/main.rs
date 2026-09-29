use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};
use workspacer_hub::{Hub, Options};

#[derive(Parser)]
#[command(about = "Experimental Rust hub core; not yet a replacement for workspacer serve")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7895")]
    listen: SocketAddr,
    #[arg(long)]
    mcp_listen: Option<SocketAddr>,
    /// Read an existing host credential. This program never rotates identity.
    #[arg(long,required_unless_present="mcp_inventory")]
    token_file: Option<PathBuf>,
    /// Print reference facade mapping gaps without starting services.
    #[arg(long)]
    mcp_inventory:bool,
    /// Shared tokens.json store for scoped clients and providers.
    #[arg(long)]
    tokens_file: Option<PathBuf>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    config_dir: Option<PathBuf>,
    /// Embed claudemon with this database instead of requiring an external engine.
    #[arg(long)]
    database: Option<PathBuf>,
    #[arg(long)]
    home_dir: Option<PathBuf>,
    #[arg(long)]
    jobs_file: Option<PathBuf>,
    /// Existing peers.json. Defaults to config-dir/peers.json; empty disables.
    #[arg(long)]
    peers_file: Option<PathBuf>,
    /// Plugin directory. Defaults to config-dir/plugins; empty disables.
    #[arg(long)]
    plugins_dir: Option<PathBuf>,
    #[arg(long, default_value = "")]
    plugin_origin: String,
    #[arg(long)]
    plugin_examples_dir: Option<PathBuf>,
    #[arg(long="trusted-host",value_delimiter=',')]
    trusted_hosts:Vec<String>,
    #[arg(long)]
    webapp_dir:Option<PathBuf>,
    #[arg(long)]
    push_dir:Option<PathBuf>,
}

enum Owner {
    Hub(Hub),
    Backend(workspacer_hub::backend::Backend),
}
impl Owner {
    fn handle(&self) -> workspacer_hub::Handle {
        match self {
            Self::Hub(h) => h.handle(),
            Self::Backend(b) => b.handle(),
        }
    }
    async fn shutdown(self) -> anyhow::Result<()> {
        match self {
            Self::Hub(h) => h.shutdown(),
            Self::Backend(b) => b.shutdown().await,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.mcp_inventory {
        println!("{}",serde_json::to_string_pretty(&workspacer_hub::mcp::migration_inventory())?);
        return Ok(());
    }
    let token = std::fs::read_to_string(args.token_file.expect("clap requires token-file"))?.trim().to_owned();
    let mut options = Options::default();
    options.listen = Some(args.listen);
    options.mcp_listen = args.mcp_listen;
    options.token = token;
    options.scoped_tokens = args.tokens_file;
    options.data_dir = args.data_dir;
    options.config_dir = args.config_dir;
    if options.scoped_tokens.is_none() {
        options.scoped_tokens = options.config_dir.as_ref().map(|directory| directory.join("tokens.json"));
    }
    options.home_dir = args.home_dir;
    options.plugin_origin = args.plugin_origin;
    options.trusted_hosts=if args.trusted_hosts.is_empty(){std::env::var("HUB_TRUSTED_HOSTS").unwrap_or_default().split(',').map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned).collect()}else{args.trusted_hosts};
    options.webapp_dir=args.webapp_dir.or_else(||std::env::var_os("WORKSPACER_WEBAPP_DIR").map(PathBuf::from));
    options.push_dir=args.push_dir;
    options.plugin_examples_dir = args.plugin_examples_dir;
    options.plugins_dir = args.plugins_dir.or_else(|| options.config_dir.as_ref().map(|dir| dir.join("plugins")))
        .filter(|path| !path.as_os_str().is_empty());
    options.peers_file = args.peers_file.or_else(|| options.config_dir.as_ref().map(|dir| dir.join("peers.json")))
        .filter(|path| !path.as_os_str().is_empty());
    options.jobs_file = args
        .jobs_file
        .or_else(|| {
            options
                .data_dir
                .as_ref()
                .map(|directory| directory.join("jobs.json"))
        })
        .filter(|path| !path.as_os_str().is_empty());
    let usage_poll_on_boot =
        workspacer_hub::backend::configured_usage_polling(options.config_dir.as_deref());
    let owner = if let Some(database) = args.database {
        Owner::Backend(
            workspacer_hub::backend::Backend::start(
                claudemon::daemon::ServeConfig {
                    host: "127.0.0.1".into(),
                    hook_port: 0,
                    api_port: 0,
                    db_path: database,
                },
                claudemon::daemon::embedded::Options { usage_poll_on_boot },
                options,
            )
            .await?,
        )
    } else {
        Owner::Hub(Hub::start(options)?)
    };
    let address = owner.handle().ready().await?.expect("listener requested");
    println!(
        "{}",
        serde_json::json!({"service":"rust-hub-core", "address": address.to_string(), "migrationComplete":false})
    );
    let mut status = owner.handle().status();
    let ended = async {
        loop {
            match status.borrow_and_update().clone() {
                workspacer_hub::Status::Failed(error) => anyhow::bail!("backend failed: {error}"),
                workspacer_hub::Status::Stopped => anyhow::bail!("backend stopped unexpectedly"),
                _ => (),
            }
            status.changed().await?;
        }
    };
    let result: anyhow::Result<()> =
        tokio::select! {result=process_signal()=>result,result=ended=>result};
    let cleanup = owner.shutdown().await;
    result.and(cleanup)
}

// Only the standalone executable owns process signals.
async fn process_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {result=tokio::signal::ctrl_c()=>result?,_=terminate.recv()=>()}
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

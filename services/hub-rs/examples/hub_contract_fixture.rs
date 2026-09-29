//! Real Rust bus/MCP/HTTP adapters with explicitly fake execution boundaries.
//! Built only with `--features test-support`; never included in runtime bundles.
use anyhow::{Context, Result};
#[path = "support/paired.rs"]
mod paired;
use clap::{Parser, ValueEnum};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use workspacer_hub::{
    Hub, Options, Status,
    auth::{self, Scope},
    services::routing,
    test_support,
};
#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Browser,
    Mcp,
    Paired,
}
#[derive(Parser)]
struct Args {
    #[arg(long, value_enum)]
    mode: Mode,
    #[arg(long)]
    root: PathBuf,
    #[arg(long, default_value = "127.0.0.1:0")]
    listen: std::net::SocketAddr,
    #[arg(long, default_value = "dispatch-chain-synthetic-host")]
    token: String,
    #[arg(long)]
    tokens_file: Option<PathBuf>,
    #[arg(long)]
    layout_file: Option<PathBuf>,
    #[arg(long)]
    push_dir: Option<PathBuf>,
    #[arg(long)]
    peers_file: Option<PathBuf>,
    #[arg(long)]
    jobs_file: Option<PathBuf>,
    #[arg(long)]
    nodes_file: Option<PathBuf>,
    #[arg(long)]
    webapp_dir: Option<PathBuf>,
}
fn contained(root: &std::path::Path, path: &std::path::Path) -> Result<()> {
    anyhow::ensure!(
        path.is_absolute()
            && workspacer_hub::services::paths::canonicalize(path)?.starts_with(root),
        "fixture state path escaped its scratch root"
    );
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    anyhow::ensure!(args.root.is_absolute(), "absolute scratch root required");
    let root = args.root.canonicalize()?;
    if matches!(args.mode, Mode::Paired) {
        return paired::run(&root).await;
    }
    let config = root.join("config/workspacer");
    std::fs::create_dir_all(&config)?;
    let tokens = args.tokens_file.unwrap_or(config.join("tokens.json"));
    contained(&root, &tokens)?;
    let mut options = Options::default();
    options.listen = Some(args.listen);
    options.token = args.token;
    options.scoped_tokens = Some(tokens.clone());
    options.control_plane_only = true;
    options.plugins_dir = Some(root.join("plugins"));
    options.data_dir = Some(match args.layout_file {
        Some(path) => {
            contained(&root, &path)?;
            anyhow::ensure!(
                path.file_name().is_some_and(|name| name == "layout.json"),
                "fixture layout filename must be layout.json"
            );
            path.parent().context("layout parent")?.into()
        }
        None => root.join("hub-state"),
    });
    options.push_dir = Some(args.push_dir.unwrap_or(root.join("push")));
    options.peers_file = args.peers_file;
    options.jobs_file = args.jobs_file;
    options.nodes_file = args.nodes_file;
    options.webapp_dir = args.webapp_dir;
    for path in [
        &options.data_dir,
        &options.push_dir,
        &options.peers_file,
        &options.jobs_file,
        &options.nodes_file,
    ]
    .into_iter()
    .flatten()
    {
        contained(&root, path)?;
    }
    if let Some(path) = &options.nodes_file {
        let nodes: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        for node in nodes.as_array().context("fixture nodes must be an array")? {
            if node["fly"].is_object() {
                let url = url::Url::parse(
                    node["fly"]["baseUrl"]
                        .as_str()
                        .context("fake cloud endpoint required")?,
                )?;
                anyhow::ensure!(
                    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
                    "fixture cannot contact a real cloud endpoint"
                );
            }
        }
    }
    if matches!(args.mode, Mode::Mcp) {
        anyhow::ensure!(
            std::env::var("WKS_DISPATCH_CHAIN_FIXTURE").as_deref() == Ok("1"),
            "fixture opt-in required"
        );
        for label in [
            "session:manager-current",
            "session:manager-other",
            "pairing",
        ] {
            auth::mint(&tokens, Scope::Operator, label)?;
        }
        options.mcp_listen = Some("127.0.0.1:0".parse()?);
        options = options.plugin_token("dispatch-chain-synthetic-plugin", "fixture.plugin", vec![]);
        std::fs::write(
            config.join("routing.yaml"),
            "active_profile: codex_only\nceilings:\n  default: {max_capability: frontier, max_tool_scope: view}\n",
        )?;
        let routing = Arc::new(routing::RoutingService::open(config)?);
        options = test_support::after_services(options, move |options| {
            let configured = test_support::with_routing(options, routing.clone());
            let routing = routing.clone();
            configured.handler("routing.select", move |_, params| {
                let routing = routing.clone();
                async move {
                    routing.select(
                        params,
                        &json!({"providers":[]}),
                        chrono::Utc::now().timestamp(),
                    )
                }
            })
        });
    }
    let hub = Hub::start(options)?;
    let bus = hub.ready().await?.context("bus listener missing")?;
    let facade = match *hub.handle().status().borrow() {
        Status::Ready { mcp_address, .. } => mcp_address,
        _ => None,
    };
    println!(
        "{}",
        json!({"busURL":format!("ws://{bus}/bus"),"facadeURL":facade.map(|address|format!("http://{address}"))})
    );
    wait_for_parent().await;
    hub.shutdown()
}
async fn wait_for_parent() {
    let (eof, closed) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
        let _ = eof.send(());
    });
    tokio::select! {_=closed=>{},_=tokio::signal::ctrl_c()=>{},_=tokio::time::sleep(Duration::from_secs(600))=>{}}
}

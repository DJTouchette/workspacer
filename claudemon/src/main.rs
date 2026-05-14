use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("claudemon=info")),
        )
        .init();

    let cli = claudemon::cli::Cli::parse();
    claudemon::cli::dispatch(cli).await
}

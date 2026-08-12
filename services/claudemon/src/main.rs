use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// See Cargo.toml: glibc arenas never return the peak of a large transient
/// allocation, and this daemon serializes whole conversations on demand.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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

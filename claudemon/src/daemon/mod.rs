pub mod api;
pub mod hook;
pub mod init;
pub mod spawn;
pub mod wrapper_ws;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::session::{HookEvent, SessionStore};
use crate::store::Db;

pub struct ServeConfig {
    pub host: String,
    pub hook_port: u16,
    pub api_port: u16,
    pub db_path: PathBuf,
}

pub async fn run(cfg: ServeConfig) -> Result<()> {
    let store = SessionStore::new();
    let db = Db::open(&cfg.db_path)
        .with_context(|| format!("opening db at {}", cfg.db_path.display()))?;
    tracing::info!(db = %cfg.db_path.display(), "sqlite store ready");

    // Persistence runs out-of-band: subscribe to the raw-hook broadcast and
    // write each event to SQLite without blocking the hook handler's response.
    spawn_persistence_task(db.clone(), store.subscribe_hooks());

    let hook_addr: SocketAddr = format!("{}:{}", cfg.host, cfg.hook_port).parse()?;
    let api_addr: SocketAddr = format!("{}:{}", cfg.host, cfg.api_port).parse()?;

    let hook_listener = TcpListener::bind(hook_addr)
        .await
        .with_context(|| format!("binding hook server to {hook_addr}"))?;
    let api_listener = TcpListener::bind(api_addr)
        .await
        .with_context(|| format!("binding api server to {api_addr}"))?;

    tracing::info!(%hook_addr, "hook server listening");
    tracing::info!(%api_addr, "api server listening");

    let hook_app = hook::router(store.clone());
    let api_app = api::router(store);

    let hook_task = tokio::spawn(async move {
        if let Err(err) = axum::serve(hook_listener, hook_app).await {
            tracing::error!(?err, "hook server crashed");
        }
    });
    let api_task = tokio::spawn(async move {
        if let Err(err) = axum::serve(api_listener, api_app).await {
            tracing::error!(?err, "api server crashed");
        }
    });

    tokio::select! {
        _ = hook_task => {},
        _ = api_task => {},
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutting down");
        }
    }

    Ok(())
}

fn spawn_persistence_task(
    db: Db,
    mut rx: tokio::sync::broadcast::Receiver<HookEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let db = db.clone();
                    // Run the synchronous sqlite write on the blocking pool so
                    // we don't tie up an async worker on file I/O.
                    let result = tokio::task::spawn_blocking(move || db.record_event(&event))
                        .await
                        .unwrap_or_else(|join_err| Err(anyhow::anyhow!(join_err)));
                    if let Err(err) = result {
                        tracing::warn!(?err, "persisting hook event failed");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "persistence task lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!("hook broadcast closed; persistence task exiting");
                    break;
                }
            }
        }
    });
}

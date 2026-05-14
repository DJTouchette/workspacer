pub mod api;
pub mod hook;
pub mod init;
pub mod wrapper_ws;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::session::SessionStore;

pub async fn run(host: &str, hook_port: u16, api_port: u16) -> Result<()> {
    let store = SessionStore::new();

    let hook_addr: SocketAddr = format!("{host}:{hook_port}").parse()?;
    let api_addr: SocketAddr = format!("{host}:{api_port}").parse()?;

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

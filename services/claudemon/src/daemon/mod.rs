pub mod api;
pub mod hook;
pub mod init;
pub mod spawn;
pub mod wrapper_ws;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::session::{conversation, ConversationStore, HookEvent, SessionStore};
use crate::store::Db;

/// How many recent sessions to restore into the live list on startup. Newest
/// first; the rest stay in the DB. Generous enough to cover any realistic set
/// of open agents without flooding the UI with stale history.
const SESSION_HYDRATE_LIMIT: usize = 100;

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

    // Repopulate the in-memory list from the DB so sessions survive a daemon
    // restart: prior agents reappear (as stopped — no process is attached, so
    // they show as resumable, not live) and can be resumed with
    // `claude --resume <id>`. Bounded to the most-recent window. Nothing is
    // deleted; stale ones come back archived (see `SessionState::is_archived`)
    // so they stay out of the default list but remain reachable.
    match db.load_recent_sessions(SESSION_HYDRATE_LIMIT) {
        Ok(sessions) if !sessions.is_empty() => {
            let count = sessions.len();
            store.hydrate(sessions);
            tracing::info!(count, "hydrated prior sessions from db");
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(?err, "hydrating sessions from db failed"),
    }

    // Persistence runs out-of-band: subscribe to the raw-hook broadcast and
    // write each event to SQLite without blocking the hook handler's response.
    spawn_persistence_task(db.clone(), store.subscribe_hooks());

    // Transcript tailer: daemon-owned conversation parsing. Streams structured
    // deltas to clients so they never re-read the JSONL themselves.
    let conv = ConversationStore::new();
    conversation::spawn_tailer(store.clone(), conv.clone());

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
    // Retained past the `store` move into ApiState so shutdown can kill the PTY
    // children the daemon spawned (they have no kill-on-drop).
    let store_for_shutdown = store.clone();
    let api_app = api::router_with_host(
        api::ApiState { store, db, conv },
        // Accept the daemon's own bind address as a valid Host (loopback is
        // always accepted); wildcard binds add nothing (see `AllowedHosts`).
        Some(cfg.host.clone()),
    );

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

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = hook_task => {},
            _ = api_task => {},
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, shutting down");
            }
            _ = wait_for_parent_exit() => {
                tracing::info!("parent process exited; shutting down");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = hook_task => {},
            _ = api_task => {},
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
            }
            _ = wait_for_parent_exit() => {
                tracing::info!("parent process exited; shutting down");
            }
        }
    }

    // Kill the PTY children we spawned so they don't outlive the daemon (and the
    // launcher). Managed-provider children use kill_on_drop and are reaped as the
    // runtime tears down; the portable-pty children are not, so kill them here.
    store_for_shutdown.kill_all_ptys();

    Ok(())
}

/// Resolves when our parent process exits, so a daemon launched by the desktop
/// app never outlives it (no orphaned listeners holding ports 7890/7891).
///
/// Detection is via stdin EOF: the launcher hands us a stdin pipe and holds the
/// write end open for its whole life; when it dies — even on a force-kill the OS
/// can't notify us about — the kernel closes the pipe and our read returns EOF.
/// Cross-platform (Win/Mac/Linux), no polling, no process-handle APIs.
///
/// Gated on `WORKSPACER_PARENT_PID` (set by the launcher): when it's unset — a
/// manual `claudemon serve` from a terminal — this never resolves, so the
/// daemon keeps running. We discard any bytes; only EOF matters.
async fn wait_for_parent_exit() {
    use tokio::io::AsyncReadExt;
    if std::env::var_os("WORKSPACER_PARENT_PID").is_none() {
        std::future::pending::<()>().await;
        return;
    }
    let mut stdin = tokio::io::stdin();
    let mut buf = [0u8; 256];
    loop {
        match stdin.read(&mut buf).await {
            Ok(0) | Err(_) => break, // parent closed the pipe (exited)
            Ok(_) => {}              // ignore anything the parent writes
        }
    }
}

fn spawn_persistence_task(db: Db, mut rx: tokio::sync::broadcast::Receiver<HookEvent>) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let db_inner = db.clone();
                    // Run the synchronous sqlite write on the blocking pool so
                    // we don't tie up an async worker on file I/O.
                    let result = tokio::task::spawn_blocking(move || db_inner.record_event(&event))
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

pub mod api;
pub mod git;
pub mod hook;
pub mod init;
pub mod spawn;
pub mod wrapper_ws;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::session::{HookEvent, SessionStore};
use crate::store::items::{ItemBroadcaster, ItemChange};
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
    let items_broadcaster = ItemBroadcaster::new();

    // Persistence runs out-of-band: subscribe to the raw-hook broadcast and
    // write each event to SQLite without blocking the hook handler's response.
    spawn_persistence_task(
        db.clone(),
        items_broadcaster.clone(),
        store.subscribe_hooks(),
    );
    // Idle sweep: promote silent working sessions to stuck per spec §11.
    spawn_idle_sweep(db.clone(), items_broadcaster.clone());

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
    let api_app = api::router(api::ApiState {
        store,
        db,
        items: items_broadcaster,
    });

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
    items: ItemBroadcaster,
    mut rx: tokio::sync::broadcast::Receiver<HookEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let db_inner = db.clone();
                    let session_id = event.session_id.clone();
                    let now = time::OffsetDateTime::now_utc().unix_timestamp();
                    // Run the synchronous sqlite write on the blocking pool so
                    // we don't tie up an async worker on file I/O.
                    let result = tokio::task::spawn_blocking(move || {
                        db_inner.record_and_classify(&event, now)
                    })
                    .await
                    .unwrap_or_else(|join_err| Err(anyhow::anyhow!(join_err)));
                    match result {
                        Ok(outcome) => {
                            if !outcome.created_item_ids.is_empty() {
                                tracing::info!(
                                    items = ?outcome.created_item_ids,
                                    state = outcome.new_session_state.as_str(),
                                    "classifier created items"
                                );
                            }
                            broadcast_classifier_outcome(&db, &items, &session_id, &outcome).await;
                        }
                        Err(err) => tracing::warn!(?err, "persisting hook event failed"),
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

/// Fetch the affected rows and broadcast item changes downstream. Fire-and-forget;
/// a failure here means subscribers miss a notification but doesn't roll back
/// the DB write that just landed.
async fn broadcast_classifier_outcome(
    db: &Db,
    items: &ItemBroadcaster,
    session_id: &str,
    outcome: &crate::store::ClassifyOutcome,
) {
    for id in &outcome.created_item_ids {
        if let Some(row) = fetch_item(db, id).await {
            items.send(ItemChange::ItemCreated { item: row });
        }
    }
    for id in &outcome.touched_item_ids {
        if let Some(row) = fetch_item(db, id).await {
            items.send(ItemChange::ItemChanged { item: row });
        }
    }
    for id in &outcome.unsnoozed_item_ids {
        if let Some(row) = fetch_item(db, id).await {
            items.send(ItemChange::ItemChanged { item: row });
        }
    }
    for id in &outcome.resolved_item_ids {
        items.send(ItemChange::ItemResolved {
            id: id.clone(),
            session_id: session_id.to_string(),
        });
    }
}

async fn fetch_item(db: &Db, id: &str) -> Option<crate::store::items::ItemRow> {
    let db = db.clone();
    let id = id.to_string();
    tokio::task::spawn_blocking(move || db.get_item(&id))
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten()
}

/// Tick on a 30s interval and promote silent working sessions to stuck.
/// Cheap enough to run in the background indefinitely — one indexed scan
/// against `sessions` per tick plus at most a handful of single-row writes.
fn spawn_idle_sweep(db: Db, items: ItemBroadcaster) {
    const TICK: std::time::Duration = std::time::Duration::from_secs(30);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the first immediate tick at startup.
        interval.tick().await;
        loop {
            interval.tick().await;
            let db_inner = db.clone();
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let result = tokio::task::spawn_blocking(move || db_inner.idle_sweep(now))
                .await
                .unwrap_or_else(|err| Err(anyhow::anyhow!(err)));
            match result {
                Ok(hits) if !hits.is_empty() => {
                    tracing::info!(hit_count = hits.len(), "idle sweep promoted sessions to stuck");
                    for hit in hits {
                        if let Some(row) = fetch_item(&db, &hit.item_id).await {
                            items.send(ItemChange::ItemCreated { item: row });
                        }
                    }
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(?err, "idle sweep failed"),
            }
        }
    });
}

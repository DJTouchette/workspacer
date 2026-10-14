//! `sessions.queue*` bus methods and the auto-flush worker over the hub-owned
//! message queue (`services::message_queue`). Delivery always goes through the
//! ordinary `agents.sendMessage` path of this service.
use super::{Sessions, segment, snapshots};
use crate::protocol::Event;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::sync::Arc;

/// How often queued sessions are re-evaluated without a new row (startup,
/// and the stale-turn fallback).
const TICK: std::time::Duration = std::time::Duration::from_secs(2);

fn text_param<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    match params.get(key) {
        Some(Value::String(text)) => Ok(text),
        None | Some(Value::Null) => bail!("{key} is required"),
        Some(_) => bail!("{key} must be text"),
    }
}

fn entry_param(params: &Value) -> Result<&str> {
    let id = text_param(params, "id")?;
    if !super::super::message_queue::valid_id(id) {
        bail!("invalid queued message id");
    }
    Ok(id)
}

impl Sessions {
    /// One queue method. Every mutation for a session runs under that
    /// session's queue lock, which delivery also holds.
    pub(super) async fn queue_call(&self, method: &str, params: Value) -> Result<Value> {
        let id = segment(params["sessionId"].as_str().unwrap_or(""))?.to_owned();
        if self.remote_proxies.read().unwrap().contains_key(&id)
            || self
                .rows
                .read()
                .unwrap()
                .get(&id)
                .is_some_and(|row| row["hub"].as_str().is_some_and(|hub| !hub.is_empty()))
        {
            bail!("queue messages for a remote session on its owning hub");
        }
        let lock = self.queue.session_lock(&id);
        let _serial = lock.lock().await;
        let mut reply = json!({});
        match method {
            "sessions.queueAdd" => {
                if !self.rows.read().unwrap().contains_key(&id) {
                    bail!("unknown session");
                }
                let text = text_param(&params, "text")?;
                let entry = self
                    .queue
                    .add(&id, text, params.get("attachments").cloned())?;
                reply["id"] = json!(entry);
            }
            "sessions.queueUpdate" => {
                self.queue
                    .update(&id, entry_param(&params)?, text_param(&params, "text")?)?;
            }
            "sessions.queueRemove" => self.queue.remove(&id, entry_param(&params)?),
            "sessions.queueMove" => {
                let index = params["index"]
                    .as_u64()
                    .ok_or_else(|| anyhow!("index must be a non-negative integer"))?;
                self.queue
                    .move_to(&id, entry_param(&params)?, index as usize)?;
            }
            "sessions.queueClear" => self.queue.clear(&id),
            "sessions.queueSendNow" => {
                let delivery = match params.get("delivery") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(mode)) if matches!(mode.as_str(), "steer" | "now") => {
                        Some(mode.clone())
                    }
                    Some(_) => bail!("delivery must be 'steer' or 'now'"),
                };
                let row = self.rows.read().unwrap().get(&id).cloned();
                let (index, entry) = self.queue.take(&id, entry_param(&params)?, row.as_ref())?;
                self.queue.persist().await?;
                if let Err(error) = self.deliver(&id, &entry, delivery).await {
                    self.queue.restore(&id, index, entry, None);
                    if let Err(error) = self.queue.persist().await {
                        eprintln!("message queue was not persisted: {error}");
                    }
                    self.queue_changed(&id).await;
                    return Err(error);
                }
            }
            _ => bail!("unknown queue method"),
        }
        if method != "sessions.queueSendNow" {
            self.queue.persist().await?;
        }
        self.queue_changed(&id).await;
        // Any mutation resumes a paused queue; let the worker look again.
        self.queue.kick(&id);
        reply["queue"] = self.queue.list(&id);
        Ok(reply)
    }

    async fn deliver(
        &self,
        id: &str,
        entry: &super::super::message_queue::Entry,
        delivery: Option<String>,
    ) -> Result<Value> {
        let mut params = json!({"sessionId":id,"text":entry.text});
        if let Some(attachments) = &entry.attachments {
            params["attachments"] = attachments.clone();
        }
        if let Some(delivery) = delivery {
            params["delivery"] = json!(delivery);
        }
        self.call("agents.sendMessage", params).await
    }

    /// Re-publish the session's snapshot after a queue change.
    async fn queue_changed(&self, id: &str) {
        let Some(row) = self.rows.read().unwrap().get(id).cloned() else {
            return;
        };
        let row = self.enrich(row);
        if snapshots::visible(&row, &self.layout(), time::OffsetDateTime::now_utc())
            && let Err(error) = self
                .hub
                .publish_wait(Event::new("agent.snapshot", "brain", row))
                .await
        {
            eprintln!("queued-message snapshot was not published: {error}");
        }
    }

    /// Send the head entry if the session is ready and its last sent entry's
    /// turn has ended.
    async fn flush_queue(&self, id: &str) {
        let lock = self.queue.session_lock(id);
        let _serial = lock.lock().await;
        let Some(row) = self.rows.read().unwrap().get(id).cloned() else {
            return;
        };
        let Some(entry) = self.queue.take_head(id, &row) else {
            return;
        };
        // Removed (and persisted) before the send: an interrupted delivery is
        // uncertain, and is never replayed automatically after a restart.
        if let Err(error) = self.queue.persist().await {
            eprintln!("message queue was not persisted: {error}");
        }
        if let Err(error) = self.deliver(id, &entry, None).await {
            self.queue.restore(id, 0, entry, Some(error.to_string()));
            if let Err(error) = self.queue.persist().await {
                eprintln!("message queue was not persisted: {error}");
            }
        }
        self.queue_changed(id).await;
    }

    /// Auto-flush worker: woken by observed rows and by a periodic tick.
    pub(super) async fn run_queue(
        self: Arc<Self>,
        mut kicks: tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut flushes = tokio::task::JoinSet::new();
        loop {
            let ids = tokio::select! {
                kick = kicks.recv() => match kick {
                    Some(id) => vec![id],
                    None => return,
                },
                _ = tick.tick() => {
                    // Re-observing the current row runs the stale-turn
                    // fallback and kicks any session that may now send.
                    for id in self.queue.pending_sessions() {
                        let row = self.rows.read().unwrap().get(&id).cloned();
                        if let Some(row) = row {
                            self.queue.observe(&row);
                        }
                    }
                    Vec::new()
                }
                Some(_) = flushes.join_next(), if !flushes.is_empty() => Vec::new(),
            };
            for id in ids {
                let service = self.clone();
                flushes.spawn(async move { service.flush_queue(&id).await });
            }
        }
    }

    /// The row about to be published feeds the queue's turn watch.
    pub(super) fn queue_observe(&self, row: &Value) {
        self.queue.observe(row);
    }

    /// Prune queues to the sessions the hub retains.
    pub(super) fn queue_retain(&self, ids: &std::collections::BTreeSet<String>) {
        if self.queue.retain(ids) {
            let queue = self.queue.clone();
            tokio::spawn(async move {
                if let Err(error) = queue.persist().await {
                    eprintln!("message queue was not persisted: {error}");
                }
            });
        }
    }

    pub(super) async fn queue_forget(&self, id: &str) {
        if self.queue.forget(id)
            && let Err(error) = self.queue.persist().await
        {
            eprintln!("message queue was not persisted: {error}");
        }
    }
}

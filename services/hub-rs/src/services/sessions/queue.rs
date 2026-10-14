//! `sessions.queue*` bus methods and the auto-flush worker over the hub-owned
//! message queue (`services::message_queue`). Delivery always goes through the
//! same internal send path as `agents.sendMessage` (`send::send_message`),
//! as a human's message: the queue only ever holds what a person typed.
use super::super::message_queue::{Delivery, Entry, QueueConflict, valid_id};
use super::{Sessions, segment, send::MessageOutcome, snapshots};
use crate::protocol::Event;
use anyhow::{Result, anyhow, bail};
use claudemon::session::Delivery as SendDelivery;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

/// How often queued sessions are re-evaluated without a new row (startup,
/// and the stale-turn fallback).
const TICK: Duration = Duration::from_secs(2);
/// How often queues are pruned against the daemon's FULL inventory.
const PRUNE_EVERY: Duration = Duration::from_secs(10 * 60);

fn text_param<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    match params.get(key) {
        Some(Value::String(text)) => Ok(text),
        None | Some(Value::Null) => bail!("{key} is required"),
        Some(_) => bail!("{key} must be text"),
    }
}

fn entry_param(params: &Value) -> Result<&str> {
    let id = text_param(params, "id")?;
    if !valid_id(id) {
        bail!("invalid queued message id");
    }
    Ok(id)
}

/// Sort a send's typed outcome into the queue's vocabulary: a definite
/// rejection (safe to put back and retry on the user's say-so) or an
/// uncertain outcome (never retried). The send path decides which is which
/// structurally (see `send::MessageOutcome`), so no error wording is matched.
pub(super) fn classify(outcome: MessageOutcome) -> Delivery {
    match outcome {
        MessageOutcome::Delivered(_) => Delivery::Sent,
        MessageOutcome::Rejected(error) => Delivery::Rejected(error.to_string()),
        MessageOutcome::Uncertain(error) => Delivery::Uncertain(error.to_string()),
    }
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
        let outcome = match method {
            "sessions.queueAdd" => {
                // A queued entry is replayed later as plain text; images ride
                // only an immediate agents.sendMessage.
                if params.get("attachments").is_some_and(|a| !a.is_null())
                    || params.get("images").is_some_and(|a| !a.is_null())
                {
                    bail!(
                        "queued messages cannot carry images or attachments; send them with agents.sendMessage"
                    );
                }
                if !self.rows.read().unwrap().contains_key(&id) {
                    bail!("unknown session");
                }
                let entry = self.queue.add(&id, text_param(&params, "text")?).await?;
                reply["id"] = json!(entry);
                Ok(())
            }
            "sessions.queueUpdate" => {
                // The text the editor started from: a change made elsewhere
                // since then is refused rather than overwritten.
                let expected = match params.get("expectedText") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(text)) => Some(text.as_str()),
                    Some(_) => bail!("expectedText must be text"),
                };
                self.queue
                    .update(
                        &id,
                        entry_param(&params)?,
                        text_param(&params, "text")?,
                        expected,
                    )
                    .await
            }
            "sessions.queueResume" => self.queue.resume_sending(&id).await,
            "sessions.queueRemove" => self.queue.remove(&id, entry_param(&params)?).await,
            "sessions.queueMove" => {
                let index = params["index"]
                    .as_u64()
                    .ok_or_else(|| anyhow!("index must be a non-negative integer"))?;
                self.queue
                    .move_to(&id, entry_param(&params)?, index as usize)
                    .await
            }
            "sessions.queueClear" => self.queue.clear(&id).await,
            "sessions.queueSendNow" => {
                let delivery = match params.get("delivery") {
                    None | Some(Value::Null) => SendDelivery::default(),
                    Some(Value::String(mode)) => match SendDelivery::from_wire(mode) {
                        Some(delivery) => delivery,
                        None => bail!("delivery must be 'steer' or 'now'"),
                    },
                    Some(_) => bail!("delivery must be 'steer' or 'now'"),
                };
                let entry = entry_param(&params)?.to_owned();
                let row = self.rows.read().unwrap().get(&id).cloned();
                self.queue
                    .send_now(&id, &entry, row.as_ref(), |entry| {
                        self.deliver(&id, entry, delivery)
                    })
                    .await
            }
            _ => bail!("unknown queue method"),
        };
        // Publish whatever was committed, including a send-now's bookkeeping.
        self.queue_changed(&id).await;
        if let Err(error) = &outcome
            && let Some(conflict) = error.downcast_ref::<QueueConflict>()
        {
            // A typed refusal: the caller keeps its edit and sees the newer
            // text, then chooses to overwrite or discard. Nothing changed.
            return Ok(json!({
                "ok": false,
                "error": conflict.to_string(),
                "conflict": {"currentText": conflict.current},
                "queue": self.queue.list(&id),
            }));
        }
        outcome?;
        // A resume (or send-now) may have lifted a pause, and any change may
        // make the head sendable; let the worker look again.
        self.queue.kick(&id);
        reply["queue"] = self.queue.list(&id);
        Ok(reply)
    }

    /// Deliver one entry through the `agents.sendMessage` send path, as the
    /// human who queued it (no sending session), with `delivery` honoured.
    async fn deliver(&self, id: &str, entry: Entry, delivery: SendDelivery) -> Delivery {
        classify(
            self.send_message(id, Some(&entry.text), delivery, None, &[])
                .await,
        )
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
        if self
            .queue
            .flush(id, &row, |entry| {
                self.deliver(id, entry, SendDelivery::Steer)
            })
            .await
            .is_some()
        {
            self.queue_changed(id).await;
        }
    }

    /// Drop queues of sessions neither the launch journal nor the daemon's
    /// FULL inventory (archived and empty rows included) still knows.
    async fn prune_queues(&self) {
        if self.queue.sessions().is_empty() {
            return;
        }
        let inventory = match self
            .request(
                "GET",
                "/sessions?include_archived=true&include_empty=true".into(),
                None,
            )
            .await
        {
            Ok(inventory) => inventory,
            Err(error) => {
                eprintln!("message queue pruning skipped: {error}");
                return;
            }
        };
        let Some(rows) = inventory.as_array() else {
            return;
        };
        let mut keep: BTreeSet<String> = rows
            .iter()
            .filter_map(|row| row["session_id"].as_str().map(str::to_owned))
            .collect();
        keep.extend(self.rows.read().unwrap().keys().cloned());
        if let Some(lifecycle) = &self.lifecycle {
            keep.extend(lifecycle.session_ids());
        }
        if let Err(error) = self.queue.prune(&keep).await {
            eprintln!("message queue was not persisted: {error}");
        }
    }

    /// Auto-flush worker: woken by observed rows and by a periodic tick.
    pub(super) async fn run_queue(
        self: Arc<Self>,
        mut kicks: tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut prune = tokio::time::interval(PRUNE_EVERY);
        prune.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
                _ = prune.tick() => {
                    self.prune_queues().await;
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

    pub(super) async fn queue_forget(&self, id: &str) {
        if let Err(error) = self.queue.forget(id).await {
            eprintln!("message queue was not persisted: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_follows_the_typed_outcome_not_the_wording() {
        assert_eq!(
            classify(MessageOutcome::Delivered(json!({"ok":true}))),
            Delivery::Sent
        );
        // Wording that once read as "uncertain" is a rejection when the send
        // path says so, and vice versa.
        assert!(matches!(
            classify(MessageOutcome::Rejected(anyhow!("connection reset"))),
            Delivery::Rejected(reason) if reason == "connection reset"
        ));
        assert!(matches!(
            classify(MessageOutcome::Uncertain(anyhow!(
                "message delivery rejected: looks definite"
            ))),
            Delivery::Uncertain(_)
        ));
    }
}

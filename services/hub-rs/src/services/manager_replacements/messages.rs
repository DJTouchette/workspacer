//! One in-flight registry for host sends. The capture gate makes snapshotting
//! earlier sends plus installing a handoff journal operation atomic with ACKs.
use super::{ReplacementState, SendOutcome};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Clone)]
struct Flight {
    target: String,
    frame: Value,
}
pub struct MessageTracker {
    pub capture_gate: Arc<Mutex<()>>,
    state: Arc<ReplacementState>,
    flights: Mutex<BTreeMap<String, Flight>>,
}
pub enum BeginDelivery {
    Held,
    Send(TrackedDelivery),
}
pub struct TrackedDelivery {
    tracker: Arc<MessageTracker>,
    id: String,
    finished: bool,
}
impl MessageTracker {
    pub fn new(state: Arc<ReplacementState>) -> Arc<Self> {
        Arc::new(Self {
            capture_gate: Arc::new(Mutex::new(())),
            state,
            flights: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn begin(
        self: &Arc<Self>,
        target: &str,
        message: &str,
        signatures: &[(String, String)],
        source: Option<Value>,
        outbox: bool,
    ) -> Result<BeginDelivery> {
        let _capture = self.capture_gate.lock().unwrap();
        if !outbox
            && self
                .state
                .hold_message(target, message, signatures, source.clone())?
        {
            return Ok(BeginDelivery::Held);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut frame = json!({"id":id,"text":message,"signatures":signatures});
        if let Some(source) = source {
            frame["sourceRequest"] = source;
        }
        self.flights.lock().unwrap().insert(
            id.clone(),
            Flight {
                target: target.into(),
                frame,
            },
        );
        Ok(BeginDelivery::Send(TrackedDelivery {
            tracker: self.clone(),
            id,
            finished: false,
        }))
    }
    /// Caller holds capture_gate while the returned evidence is installed in the
    /// journal. Do not acquire it again here: the handoff start owns that interval.
    pub fn evidence(&self, target: &str) -> Value {
        let flights = self.flights.lock().unwrap();
        let rows: Vec<_> = flights
            .values()
            .filter(|f| f.target == target)
            .map(|f| f.frame.clone())
            .collect();
        let mut signatures = json!({});
        for row in &rows {
            for pair in row["signatures"].as_array().into_iter().flatten() {
                if let (Some(worker), Some(signature)) = (pair[0].as_str(), pair[1].as_str()) {
                    signatures[worker] = signature.into();
                }
            }
        }
        json!({"inFlightMessages":rows,"signatures":signatures,"finishes":{}})
    }
    pub fn has_inflight(&self, targets: &[String]) -> bool {
        self.flights
            .lock()
            .unwrap()
            .values()
            .any(|f| targets.contains(&f.target))
    }
    fn finish(&self, id: &str, outcome: &SendOutcome) -> Result<()> {
        let _capture = self.capture_gate.lock().unwrap();
        let flight = self.flights.lock().unwrap().get(id).cloned();
        if let Some(flight) = flight {
            let (accepted, rejected, error) = match outcome {
                SendOutcome::Accepted => (true, false, None),
                SendOutcome::Rejected { reason, .. } => (false, true, Some(reason.as_str())),
                SendOutcome::Uncertain(error) => (false, false, Some(error.as_str())),
            };
            self.state
                .note_inflight_message(&flight.target, id, accepted, error, rejected)?;
            if accepted {
                for pair in flight.frame["signatures"].as_array().into_iter().flatten() {
                    if let (Some(worker), Some(signature)) = (pair[0].as_str(), pair[1].as_str()) {
                        self.state.record_signature(worker, signature)?;
                    }
                }
            }
            self.flights.lock().unwrap().remove(id);
        }
        Ok(())
    }
}
impl TrackedDelivery {
    pub fn finish(mut self, outcome: &SendOutcome) -> Result<()> {
        self.tracker.finish(&self.id, outcome)?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for TrackedDelivery {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.tracker.finish(
                &self.id,
                &SendOutcome::Uncertain("Host send interrupted before acknowledgement".into()),
            );
        }
    }
}

pub struct DeliveryResult {
    pub held: bool,
    pub outcome: SendOutcome,
}
impl DeliveryResult {
    pub fn accepted(&self) -> bool {
        matches!(self.outcome, SendOutcome::Accepted)
    }
    pub fn wire(&self) -> Value {
        match &self.outcome {
            SendOutcome::Accepted => json!({"ok":true,"held":self.held}),
            SendOutcome::Rejected { status, reason } => {
                json!({"ok":false,"status":status,"delivery":"rejected","error":reason})
            }
            SendOutcome::Uncertain(reason) => {
                json!({"ok":false,"delivery":"unknown","error":reason})
            }
        }
    }
}
/// The common manager-bound transport. Normal sends may be held durably; only
/// the handoff coordinator uses outbox=true to drain those journaled messages.
pub async fn send_engine(
    engine: &claudemon::daemon::embedded::EmbeddedClient,
    tracker: &Arc<MessageTracker>,
    target: &str,
    message: &str,
    signatures: &[(String, String)],
    source_request: Option<Value>,
    outbox: bool,
    inbox: Option<&crate::services::manager_requests::ManagerRequests>,
) -> Result<DeliveryResult> {
    use anyhow::{Context, bail};
    use claudemon::daemon::embedded::{Command, CommandRejected};
    let resolved_target = if source_request.is_some() {
        inbox
            .context("Source request delivery requires manager inbox")?
            .host_target(target)?
    } else {
        target.to_string()
    };
    let target = resolved_target.as_str();
    let mut content = message.to_string();
    if let Some(source) = &source_request {
        let inbox = inbox.context("Source request delivery requires manager inbox")?;
        let request = inbox.request(target, source["requestId"].as_str().unwrap_or(""))?;
        let attempt = request["attempts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["deliveryId"] == source["deliveryId"])
            .context("Request delivery attempt unavailable")?;
        if attempt["status"] == "accepted" || request.get("intents").is_some() {
            return Ok(DeliveryResult {
                held: false,
                outcome: SendOutcome::Accepted,
            });
        }
        if attempt["status"] != "pending" {
            bail!(
                "Only pending source delivery may send; unknown must not replay, rejected requires new attempt"
            );
        }
        content = request["userContent"]
            .as_str()
            .context("Unresolved request content unavailable")?
            .into();
        if request["bootstrap"] == true {
            content = format!(
                "{}\n\nThe user says:\n\n{}",
                crate::services::launch_instructions::manager_doctrine(),
                content.trim()
            );
        }
    }
    let tracked =
        match tracker.begin(target, &content, signatures, source_request.clone(), outbox)? {
            BeginDelivery::Held => {
                return Ok(DeliveryResult {
                    held: true,
                    outcome: SendOutcome::Accepted,
                });
            }
            BeginDelivery::Send(delivery) => delivery,
        };
    if let Some(source) = &source_request {
        inbox.unwrap().finish_delivery(
            source["requestId"].as_str().unwrap_or(""),
            source["deliveryId"].as_str().unwrap_or(""),
            "unknown",
        )?;
    }
    let outcome = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        engine.request(Command::Message {
            id: target.into(),
            text: content,
        }),
    )
    .await
    {
        Ok(Ok(_)) => SendOutcome::Accepted,
        Ok(Err(error)) => {
            if let Some(rejected) = error.downcast_ref::<CommandRejected>() {
                SendOutcome::Rejected {
                    status: rejected.status,
                    reason: rejected.message.clone(),
                }
            } else {
                SendOutcome::Uncertain(error.to_string())
            }
        }
        Err(_) => SendOutcome::Uncertain("Daemon message acknowledgement timed out".into()),
    };
    if let Some(source) = &source_request {
        inbox.unwrap().finish_delivery(
            source["requestId"].as_str().unwrap_or(""),
            source["deliveryId"].as_str().unwrap_or(""),
            match &outcome {
                SendOutcome::Accepted => "accepted",
                SendOutcome::Rejected { .. } => "rejected",
                SendOutcome::Uncertain(_) => "unknown",
            },
        )?;
    }
    tracked.finish(&outcome)?;
    Ok(DeliveryResult {
        held: false,
        outcome,
    })
}

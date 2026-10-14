//! The one internal message-send path behind `agents.sendMessage` and the
//! hub-owned message queue. It reports a typed [`MessageOutcome`] so callers
//! that must not replay an uncertain send (the queue) decide from structure,
//! never from error wording.
use super::Sessions;
use anyhow::anyhow;
use claudemon::{
    daemon::embedded::CommandRejected,
    session::{Delivery, MessageSender},
};
use serde_json::{Value, json};

/// What became of one message send.
#[derive(Debug)]
pub(super) enum MessageOutcome {
    /// The engine acknowledged it (or the tracker durably held it).
    Delivered(Value),
    /// Definitely not delivered: refused before anything reached the engine,
    /// or explicitly refused by the daemon (`CommandRejected`).
    Rejected(anyhow::Error),
    /// May have been delivered (timeout, transport loss, bookkeeping failure
    /// around the send). Never safe to replay automatically.
    Uncertain(anyhow::Error),
}

impl MessageOutcome {
    /// The `agents.sendMessage` wire result: the outcome's own error either way.
    pub(super) fn into_result(self) -> anyhow::Result<Value> {
        match self {
            Self::Delivered(value) => Ok(value),
            Self::Rejected(error) | Self::Uncertain(error) => Err(error),
        }
    }
}

impl Sessions {
    /// Send `text` to session `id`. `from` names a sending session: present,
    /// the message is agent-to-agent (sender `Agent`, attributed with a
    /// sender header); absent, it comes from a person (sender `Human`) — only
    /// a person's `now` may claim `origin: {kind: "human"}`.
    ///
    /// The caller has already validated `id` (`segment`) and the wire params.
    pub(super) async fn send_message(
        &self,
        id: &str,
        text: Option<&str>,
        delivery: Delivery,
        from: Option<&str>,
    ) -> MessageOutcome {
        let Some(text) = text.filter(|s| !s.is_empty()) else {
            return MessageOutcome::Rejected(anyhow!(
                "agents.sendMessage requires {{ sessionId, text }}"
            ));
        };
        let from = from.filter(|s| !s.is_empty());
        let sender = if from.is_some() {
            MessageSender::Agent
        } else {
            MessageSender::Human
        };
        let text = match from {
            Some(from) => {
                let label = self.sender_label(from);
                format!(
                    "{}{text}",
                    super::super::fleet_messages::sender_header(
                        from,
                        label.as_deref().unwrap_or("")
                    )
                )
            }
            None => text.to_owned(),
        };
        if let Some(tracker) = &self.message_tracker {
            let target = match &self.replacements {
                // Pure routing lookup: a failure here sent nothing.
                Some(state) => match state.wake_target(id) {
                    Ok(target) => target,
                    Err(error) => return MessageOutcome::Rejected(error),
                },
                None => id.into(),
            };
            let result = match super::super::manager_replacements::messages::send_engine_with(
                &self.engine,
                tracker,
                &target,
                &text,
                &[],
                None,
                false,
                self.requests.as_deref(),
                delivery,
                sender,
            )
            .await
            {
                Ok(result) => result,
                // The tracker's own bookkeeping failed somewhere around the
                // send (hold journal, finish record): it may have gone out.
                Err(error) => return MessageOutcome::Uncertain(error),
            };
            return match result.outcome {
                super::super::manager_replacements::SendOutcome::Accepted => {
                    MessageOutcome::Delivered(result.wire())
                }
                super::super::manager_replacements::SendOutcome::Rejected { reason, .. } => {
                    MessageOutcome::Rejected(anyhow!("message delivery rejected: {reason}"))
                }
                super::super::manager_replacements::SendOutcome::Uncertain(reason) => {
                    MessageOutcome::Uncertain(anyhow!(
                        "message delivery outcome is unknown; do not retry automatically: {reason}"
                    ))
                }
            };
        }
        match self
            .request(
                "POST",
                format!("/sessions/{id}/message"),
                Some(json!({"text":text,"delivery":delivery,"sender":sender})),
            )
            .await
        {
            Ok(_) => MessageOutcome::Delivered(json!({"ok":true})),
            Err(error) if error.downcast_ref::<CommandRejected>().is_some() => {
                MessageOutcome::Rejected(error)
            }
            Err(error) => MessageOutcome::Uncertain(error),
        }
    }
}

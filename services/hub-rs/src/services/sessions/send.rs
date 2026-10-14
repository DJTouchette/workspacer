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

/// The `images` of an `agents.sendMessage`: `[{path}]` naming files this
/// host's `files.upload` wrote. Each path must be one of those uploads
/// (confined by [`claudemon::session::images::confine`]: directly inside the
/// private upload directory, under the hub-composed name, a regular file) —
/// a caller can never point the agent at any other file this way. The shape
/// was checked in `params`; the daemon re-checks, sniffs and size-caps the
/// bytes when it loads them.
pub(super) async fn confined_images_off_thread(images: &Value) -> anyhow::Result<Vec<String>> {
    if images.as_array().is_none_or(Vec::is_empty) {
        return Ok(Vec::new());
    }
    // Canonicalizing and stat-ing are file I/O: off the async workers.
    let images = images.clone();
    tokio::task::spawn_blocking(move || confined_images(&images)).await?
}

/// [`confined_images_off_thread`]'s blocking check.
pub(super) fn confined_images(images: &Value) -> anyhow::Result<Vec<String>> {
    let Some(images) = images.as_array() else {
        return Ok(Vec::new());
    };
    images
        .iter()
        .map(|image| {
            let path = image["path"].as_str().unwrap_or("");
            claudemon::session::images::confine(path)
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| anyhow!(error))
        })
        .collect()
}

impl Sessions {
    /// Send `text` to session `id`. `from` names a sending session: present,
    /// the message is agent-to-agent (sender `Agent`, attributed with a
    /// sender header); absent, it comes from a person (sender `Human`) — only
    /// a person's `now` may claim `origin: {kind: "human"}`.
    ///
    /// `images` are already-confined upload paths ([`confined_images`]).
    ///
    /// The caller has already validated `id` (`segment`) and the wire params.
    pub(super) async fn send_message(
        &self,
        id: &str,
        text: Option<&str>,
        delivery: Delivery,
        from: Option<&str>,
        images: &[String],
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
                images,
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
        let mut body = json!({"text":text,"delivery":delivery,"sender":sender});
        if !images.is_empty() {
            body["images"] = images.iter().map(|path| json!({"path":path})).collect();
        }
        match self
            .request("POST", format!("/sessions/{id}/message"), Some(body))
            .await
        {
            Ok(reply) if reply["imagesAsMarkers"] == true => {
                MessageOutcome::Delivered(json!({"ok":true,"imagesAsMarkers":true}))
            }
            Ok(_) => MessageOutcome::Delivered(json!({"ok":true})),
            Err(error) if error.downcast_ref::<CommandRejected>().is_some() => {
                MessageOutcome::Rejected(error)
            }
            Err(error) => MessageOutcome::Uncertain(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::confined_images;
    use serde_json::json;

    #[test]
    fn images_must_name_this_hosts_uploads() {
        assert!(confined_images(&json!(null)).unwrap().is_empty());
        assert!(confined_images(&json!([])).unwrap().is_empty());
        let dir = claudemon::session::images::upload_dir();
        for refused in [
            json!([{"path": "/etc/passwd"}]),
            json!([{"path": dir.join("m-1-abcdef01.txt")}]),
            json!([{"path": dir.join("m-1-ffffffff.png")}]),
            json!([{"path": "relative/m-1-abcdef01.png"}]),
        ] {
            let error = confined_images(&refused).unwrap_err().to_string();
            // (A missing upload directory is refused as such.)
            assert!(
                error.contains("not a file uploaded") || error.contains("upload directory"),
                "{refused}: {error}"
            );
        }
    }
}

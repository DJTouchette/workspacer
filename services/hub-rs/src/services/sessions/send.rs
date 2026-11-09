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

/// How long after resuming an idle-stopped session a message that raced the
/// resume keeps retrying definite refusals while the new driver comes up.
const RESUME_SEND_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);

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
    ///
    /// `uuid`, when given, is stamped on the message's provider frame (Claude
    /// stream), so its delivery receipts (`message_receipts`) can be matched
    /// even when this send's own outcome is lost. A delivered result reports
    /// the stamped uuid as `messageUuid` whenever the driver stamps one.
    pub(super) async fn send_message(
        &self,
        id: &str,
        text: Option<&str>,
        delivery: Delivery,
        from: Option<&str>,
        uuid: Option<&str>,
        images: &[String],
    ) -> MessageOutcome {
        let Some(text) = text.filter(|s| !s.is_empty()) else {
            return MessageOutcome::Rejected(anyhow!(
                "agents.sendMessage requires {{ sessionId, text }}"
            ));
        };
        let from = from.filter(|s| !s.is_empty());
        let text = self.attributed(text, from);
        let outcome = self
            .send_once(id, &text, delivery, from, uuid, images)
            .await;
        // Only a definite refusal can mean "this session was idle-stopped":
        // an uncertain send may have been delivered and is never replayed.
        if !matches!(outcome, MessageOutcome::Rejected(_)) || !images.is_empty() {
            return outcome;
        }
        match self.resume_idle(id, &text, delivery, from, uuid).await {
            Some(resumed) => resumed,
            None => outcome,
        }
    }

    /// `text` as the target reads it: an agent's message carries a sender
    /// header naming who wrote it.
    fn attributed(&self, text: &str, from: Option<&str>) -> String {
        match from {
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
        }
    }

    /// Whether claudemon (authoritative, not this hub's projection, which can
    /// lag a resume by a snapshot) holds `id` stopped by idle shutdown.
    async fn idle_stopped(&self, id: &str) -> Option<Value> {
        let row = self
            .request("GET", format!("/sessions/{id}"), None)
            .await
            .ok()?;
        (super::super::roster::ended(&row) && super::super::session_open::idle_stopped(&row))
            .then_some(row)
    }

    /// A message to a session the engine stopped for idleness resumes it —
    /// same id, same conversation, the launch's own settings — with the
    /// message as its first prompt, so idle shutdown is invisible to whoever
    /// sends: the native composer, the hub queue, a child's completion wake,
    /// a job. `None` when the session is not idle-stopped (the original
    /// refusal stands).
    ///
    /// Single-flight: a message that waited behind another's resume finds the
    /// session already live and is sent normally, retrying only definite
    /// refusals while the resumed driver comes up (claudemon refuses input
    /// until then rather than queue it).
    async fn resume_idle(
        &self,
        id: &str,
        text: &str,
        delivery: Delivery,
        from: Option<&str>,
        uuid: Option<&str>,
    ) -> Option<MessageOutcome> {
        // A replaced manager routes elsewhere; never revive its old life.
        if let Some(state) = &self.replacements {
            if state.wake_target(id).ok().as_deref() != Some(id) {
                return None;
            }
        }
        let mut flight = self.idle_resumes.lock().await;
        let now = std::time::Instant::now();
        flight.retain(|_, at| now.duration_since(*at) < RESUME_SEND_WINDOW);
        let Some(row) = self.idle_stopped(id).await else {
            // Resumed while this message waited (or never idle-stopped): send
            // into the new life once its driver takes input.
            if !flight.contains_key(id) {
                return None;
            }
            let deadline = tokio::time::Instant::now() + RESUME_SEND_WINDOW;
            loop {
                let outcome = self.send_once(id, text, delivery, from, uuid, &[]).await;
                if !matches!(outcome, MessageOutcome::Rejected(_))
                    || tokio::time::Instant::now() >= deadline
                {
                    return Some(outcome);
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        };
        let mut params = json!({"resumeSessionId": id, "message": text});
        // A launch record (this hub launched it) supplies everything else; the
        // row covers a session it did not.
        for key in ["provider", "cwd", "transport"] {
            if let Some(value) = row[key].as_str().filter(|v| !v.is_empty()) {
                params[key] = json!(value);
            }
        }
        let resumed = async {
            let client = crate::client::Client::connect_service(&self.hub).await?;
            client.call("agents.spawn", params).await
        }
        .await;
        Some(match resumed {
            Ok(reply) if reply["messageQueued"] == true => {
                flight.insert(id.to_owned(), std::time::Instant::now());
                tracing::info!(session = %id, "idle-stopped session resumed by a message");
                MessageOutcome::Delivered(json!({"ok":true,"resumed":true}))
            }
            Ok(_) => {
                flight.insert(id.to_owned(), std::time::Instant::now());
                MessageOutcome::Rejected(anyhow!(
                    "the paused session was resumed but did not accept the message; send it again"
                ))
            }
            // A spawn that errored may still have started (timeout): never
            // claim the message was refused outright.
            Err(error) => MessageOutcome::Uncertain(anyhow!(
                "resuming the paused session failed; check it before resending: {error:#}"
            )),
        })
    }

    /// One send attempt, no resume. `text` is already attributed.
    async fn send_once(
        &self,
        id: &str,
        text: &str,
        delivery: Delivery,
        from: Option<&str>,
        uuid: Option<&str>,
        images: &[String],
    ) -> MessageOutcome {
        let sender = if from.is_some() {
            MessageSender::Agent
        } else {
            MessageSender::Human
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
                text,
                &[],
                None,
                false,
                self.requests.as_deref(),
                delivery,
                sender,
                uuid,
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
        let mut payload = json!({"text":text,"delivery":delivery,"sender":sender});
        if let Some(uuid) = uuid {
            payload["uuid"] = json!(uuid);
        }
        if !images.is_empty() {
            payload["images"] = images.iter().map(|path| json!({"path":path})).collect();
        }
        match self
            .request("POST", format!("/sessions/{id}/message"), Some(payload))
            .await
        {
            Ok(reply) => {
                let mut wire = json!({"ok":true});
                if let Some(uuid) = reply["message_uuid"].as_str() {
                    wire["messageUuid"] = json!(uuid);
                }
                if reply["imagesAsMarkers"] == true {
                    wire["imagesAsMarkers"] = json!(true);
                }
                MessageOutcome::Delivered(wire)
            }
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

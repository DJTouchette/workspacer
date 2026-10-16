//! Linking a session to a pull request: `pr.link {sessionId, url, source?}`,
//! `pr.unlink {sessionId}` and `pr.session {sessionId}`. A change republishes
//! the session's snapshot, whose `pullRequest` field carries the link.
use super::super::pull_requests::{
    links::{Link, Source},
    remote::Kind,
};
use super::{Sessions, segment, snapshots};
use crate::protocol::Event;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

impl Sessions {
    pub(super) async fn pull_request_call(&self, method: &str, params: Value) -> Result<Value> {
        let service = self
            .pull_requests
            .as_ref()
            .ok_or_else(|| anyhow!("pull requests are not available on this hub"))?;
        let id = segment(params["sessionId"].as_str().unwrap_or(""))?.to_owned();
        let remote = self.remote_proxies.read().unwrap().contains_key(&id)
            || self
                .rows
                .read()
                .unwrap()
                .get(&id)
                .is_some_and(|row| row["hub"].as_str().is_some_and(|hub| !hub.is_empty()));
        if remote {
            bail!("link a remote session's pull request on its owning hub");
        }
        let existing = service.links.get(&id);
        match method {
            "pr.session" => Ok(json!({
                "pullRequest": existing.as_ref().and_then(Link::wire),
                "unlinked": existing.as_ref().is_some_and(|l| l.url.is_none()),
            })),
            "pr.unlink" => {
                service.links.set(&id, Link::unlinked())?;
                self.pull_request_changed(&id).await;
                Ok(json!({"ok": true}))
            }
            "pr.link" => {
                if !self.rows.read().unwrap().contains_key(&id) {
                    bail!("unknown session");
                }
                let source = match params["source"].as_str() {
                    None => Source::User,
                    Some(wire) => match Source::from_wire(wire) {
                        Some(source @ (Source::User | Source::Agent)) => source,
                        _ => bail!("source must be 'user' or 'agent'"),
                    },
                };
                let hint = params["provider"].as_str().and_then(Kind::from_id);
                let url = params["url"]
                    .as_str()
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .ok_or_else(|| anyhow!("url is required"))?;
                let pr = service.parse(url, hint)?;
                // The agent may link or move a link, but never undo a person's choice.
                if source != Source::User
                    && let Some(existing) = &existing
                    && existing.source == Source::User
                    && existing.url.as_deref() != Some(pr.url().as_str())
                {
                    return Ok(json!({
                        "ok": false,
                        "error": "the user chose this session's pull request; ask them to change it",
                        "pullRequest": existing.wire(),
                    }));
                }
                // Linking works offline; details fill in once the provider answers.
                let (details, warning) = match service.pull_request(&pr).await {
                    Ok(details) => (Some(details), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                let mut link = Link::new(pr.url(), source, details);
                if let Some(existing) = existing
                    && existing.url == link.url
                {
                    link.seen = existing.seen;
                    link.unresolved = existing.unresolved;
                }
                let wire = link.wire();
                service.links.set(&id, link)?;
                self.pull_request_changed(&id).await;
                Ok(json!({"ok": true, "pullRequest": wire, "warning": warning}))
            }
            _ => bail!("unknown pull request method {method}"),
        }
    }

    pub(super) async fn pull_request_changed(&self, id: &str) {
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
            eprintln!("pull request snapshot was not published: {error}");
        }
    }
}

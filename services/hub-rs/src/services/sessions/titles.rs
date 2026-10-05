//! Hub-owned automatic session titles.
//!
//! A launch that opted in (`agents.spawn` `autoTitle: true` with no label)
//! carries `autoTitle.state = "pending"` in the launch journal. Once its
//! opening exchange has an answer — or the session stopped without one — the
//! hub writes ONE title with the configured harness/model and records it on
//! exactly that launch generation. The owning hub does this whether or not a
//! client is watching, so every client (and a federated peer) sees the same
//! name, and it survives restarts because the journal does.
//!
//! The trigger matches Electron's `useAgentAutoTitle`: the first real user
//! message plus the first assistant text after it. Nothing here retries a
//! failed call: a failure records the first line of the request as a
//! `fallback` with its reason, never as a model title.
use super::Sessions;
use crate::services::provider_utilities::{Service as Utilities, TitleOutcome};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};
use tokio::sync::mpsc;

/// Concurrent title calls per hub (the shared generator also caps at 4).
const PARALLEL: usize = 4;

/// The opening exchange of a daemon conversation window: the first user
/// message and the first assistant text after it.
pub(super) fn opening(items: &Value) -> (Option<String>, Option<String>) {
    let items = items.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let text = |item: &Value| {
        item["text"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let Some(first) = items
        .iter()
        .position(|item| item["kind"] == "user_message" && text(item).is_some())
    else {
        return (None, None);
    };
    let reply = items[first + 1..]
        .iter()
        .filter(|item| item["kind"] == "assistant_text")
        .find_map(text);
    (text(&items[first]), reply)
}

/// What the journal records for an outcome.
pub(super) fn record(outcome: &TitleOutcome) -> Value {
    let mut value = outcome.wire();
    value["state"] = json!(match outcome.source {
        "model" => "titled",
        "fallback" => "fallback",
        _ => "skipped",
    });
    value["at"] = json!(chrono::Utc::now().timestamp_millis());
    value
}

impl Sessions {
    /// Queue a published row for a title check. Cheap and non-blocking: rows
    /// that do not owe a title are filtered before anything is sent.
    pub(super) fn offer_title(&self, row: &Value) {
        let Some(sender) = &self.titles else {
            return;
        };
        if row["autoTitle"]["state"] != "pending"
            || row["hub"].as_str().is_some_and(|hub| !hub.is_empty())
        {
            return;
        }
        if let Some(id) = row["sessionId"].as_str().filter(|s| !s.is_empty()) {
            // A full queue only delays: the next snapshot offers it again.
            let _ = sender.try_send(id.to_owned());
        }
    }

    /// One attempt for `id`: `None` when it is not ready (or no longer owed),
    /// otherwise the generation it was computed for and the outcome.
    async fn title_attempt(
        self: Arc<Self>,
        utilities: Arc<Utilities>,
        id: String,
    ) -> Option<(String, TitleOutcome)> {
        let lifecycle = self.lifecycle.clone()?;
        let (generation, provider, prompt) = lifecycle.auto_title_pending(&id)?;
        // Off means "not now", not "never": the launch stays pending, and
        // turning titles back on names it at its next turn boundary.
        if !utilities.titles_enabled() {
            return None;
        }
        let mode = self
            .rows
            .read()
            .unwrap()
            .get(&id)
            .and_then(|row| row["mode"].as_str().map(str::to_owned))?;
        let stopped = mode == "stopped";
        if !matches!(mode.as_str(), "input" | "approval" | "question" | "stopped") {
            return None;
        }
        let conversation = self
            .request("GET", format!("/sessions/{id}/conversation"), None)
            .await
            .ok()?;
        let (asked, reply) = opening(&conversation["items"]);
        // The launch's own message is authoritative: some providers prepend
        // instructions to the first turn the conversation records.
        let user = Some(prompt)
            .filter(|p| !p.trim().is_empty())
            .or(asked)
            .unwrap_or_default();
        if reply.is_none() && !stopped {
            return None;
        }
        if user.is_empty() {
            return stopped.then(|| {
                (
                    generation.clone(),
                    TitleOutcome {
                        title: None,
                        source: "none",
                        provider,
                        model: None,
                        reason: Some("empty"),
                    },
                )
            });
        }
        let outcome = utilities
            .suggest(&provider, &user, reply.as_deref().unwrap_or(""), true)
            .await;
        Some((generation, outcome))
    }

    async fn commit_title(&self, id: &str, generation: &str, outcome: TitleOutcome) {
        let Some(lifecycle) = &self.lifecycle else {
            return;
        };
        if outcome.source != "model" {
            eprintln!(
                "title: session {id} not titled by {} ({}){}",
                outcome.provider,
                outcome.reason.unwrap_or("failed"),
                if outcome.title.is_some() {
                    "; using the first line of the request"
                } else {
                    ""
                }
            );
        }
        match lifecycle
            .note_auto_title(id, generation, record(&outcome))
            .await
        {
            Ok(true) => {
                let _mutation = self.mutations.lock().await;
                let row = self.rows.read().unwrap().get(id).cloned();
                if let Some(row) = row {
                    if let Err(error) = self.publish(row).await {
                        eprintln!("title: snapshot publish failed for {id}: {error}");
                    }
                }
            }
            // Superseded: resumed, renamed or already named. Drop silently.
            Ok(false) => {}
            Err(error) => eprintln!("title: not recorded for {id}: {error}"),
        }
    }

    /// The title worker: bounded parallelism, one attempt per session at a
    /// time, and offers that arrive while full wait their turn instead of
    /// being dropped (an idle session may never publish again).
    pub(super) async fn run_titles(
        self: Arc<Self>,
        utilities: Arc<Utilities>,
        mut offers: mpsc::Receiver<String>,
    ) {
        let mut running = BTreeSet::<String>::new();
        let mut waiting = VecDeque::<String>::new();
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            while jobs.len() < PARALLEL {
                let Some(id) = waiting.pop_front() else {
                    break;
                };
                running.insert(id.clone());
                let (service, utilities) = (self.clone(), utilities.clone());
                jobs.spawn(async move {
                    let result = service.clone().title_attempt(utilities, id.clone()).await;
                    if let Some((generation, outcome)) = result {
                        service.commit_title(&id, &generation, outcome).await;
                    }
                    id
                });
            }
            tokio::select! {
                offer = offers.recv() => {
                    let Some(id) = offer else { break };
                    if !running.contains(&id) && !waiting.contains(&id) {
                        waiting.push_back(id);
                    }
                }
                Some(done) = jobs.join_next() => {
                    if let Ok(id) = done {
                        running.remove(&id);
                    }
                }
            }
        }
        jobs.abort_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_is_first_user_message_and_the_reply_after_it() {
        let items = json!([
            {"kind":"assistant_text","text":"banner before anything"},
            {"kind":"user_message","text":"  "},
            {"kind":"user_message","text":"Fix the login redirect"},
            {"kind":"tool_use","id":"t","name":"Read","input":{}},
            {"kind":"assistant_text","text":"Looking at the router"},
            {"kind":"user_message","text":"later"},
            {"kind":"assistant_text","text":"later reply"}
        ]);
        assert_eq!(
            opening(&items),
            (
                Some("Fix the login redirect".into()),
                Some("Looking at the router".into())
            )
        );
        assert_eq!(
            opening(&json!([{"kind":"user_message","text":"only"}])),
            (Some("only".into()), None)
        );
        assert_eq!(opening(&json!(null)), (None, None));
    }

    #[test]
    fn record_never_reports_a_fallback_as_a_model_title() {
        let outcome = |title: Option<&str>, source, reason| TitleOutcome {
            title: title.map(str::to_owned),
            source,
            provider: "codex".into(),
            model: Some("gpt-x".into()),
            reason,
        };
        let titled = record(&outcome(Some("Fix login"), "model", None));
        assert_eq!(titled["state"], "titled");
        assert_eq!(titled["model"], "gpt-x");
        assert!(titled.get("reason").is_none());
        let fallback = record(&outcome(Some("fix the login"), "fallback", Some("missing")));
        assert_eq!(fallback["state"], "fallback");
        assert_eq!(fallback["reason"], "missing");
        assert_eq!(
            record(&outcome(None, "none", Some("empty")))["state"],
            "skipped"
        );
    }
}

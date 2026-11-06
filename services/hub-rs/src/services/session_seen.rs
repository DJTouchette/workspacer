//! Hub-owned "finished, not yet looked at" marks: which sessions ended a
//! turn since a client last opened them.
//!
//! Like `sessionArchive` and `sessionOpen` this is shared VIEW state, so
//! opening a session on one client (native, desktop, mobile) clears it on
//! every other. It never starts, stops or changes a session.
//!
//! A mark is added only when this hub run SEES a turn end: a row whose mode
//! was working (`responding`, `thinking`, a tool…) and is now idle, waiting
//! for input, or stopped. Nothing is inferred from a row that is already idle
//! when first seen (an upgrade, a hub restart, a reconnect), so no old session
//! lights up. `sessionSeen.mark` removes a mark; a session that goes back to
//! work keeps its mark until it is opened or ends another turn.
//!
//! The document is `{version, unseen: {sessionId: finishedAtMs}}`. Every
//! accepted change bumps `version`, persists, and publishes the whole document
//! as `sessionSeen.changed`, so a client converges from either the event or a
//! `sessionSeen.get` after a reconnect.
use crate::{Handle, protocol::Event};
use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

/// Upper bound on marks; past it the oldest go first.
pub const MAX_UNSEEN: usize = 10_000;
/// Rows whose last mode this hub run remembers, to spot a turn ending.
const MAX_TRACKED: usize = 20_000;
const MAX_ID_BYTES: usize = 256;

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control)
}

fn row_id(row: &Value) -> Option<&str> {
    row["sessionId"]
        .as_str()
        .or_else(|| row["session_id"].as_str())
        .filter(|id| valid_id(id))
}

/// A row's mode as a turn sees it. `None` when the row does not say, so a
/// partial row never reads as a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Working,
    /// Waiting on the user's decision: the turn is paused, not over.
    Blocked,
    Done,
}

fn phase(row: &Value) -> Option<Phase> {
    let decision = |key: &str| row.get(key).is_some_and(|v| !v.is_null());
    if decision("pendingApproval") || decision("pendingQuestions") {
        return Some(Phase::Blocked);
    }
    let mode = row["mode"].as_str().or_else(|| row["state"].as_str())?;
    Some(match mode {
        "responding" | "working" | "thinking" | "streaming" | "running" | "tool" | "executing"
        | "compacting" => Phase::Working,
        "approval" | "waiting_approval" | "question" | "waiting_input" => Phase::Blocked,
        "input" | "idle" | "done" | "stopped" | "ended" => Phase::Done,
        _ => return None,
    })
}

pub struct SessionSeen {
    document: Mutex<Value>,
    path: Option<PathBuf>,
    hub: Option<Handle>,
    /// Each tracked session's last phase in this hub run.
    phases: Mutex<BTreeMap<String, Phase>>,
}

fn sanitize(unseen: &Value) -> Map<String, Value> {
    let mut entries: Vec<(String, i64)> = unseen
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, _)| valid_id(id))
        .filter_map(|(id, at)| Some((id.clone(), at.as_i64()?)))
        .collect();
    // Newest kept when a hand-edited file is over the cap.
    entries.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    entries
        .into_iter()
        .take(MAX_UNSEEN)
        .map(|(id, at)| (id, json!(at)))
        .collect()
}

impl SessionSeen {
    pub fn open(path: Option<PathBuf>, hub: Option<Handle>) -> Self {
        let mut document = json!({"version":0,"unseen":{}});
        if let Some(path) = &path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) if value.is_object() => {
                        document = json!({
                            "version": value["version"].as_i64().unwrap_or(0).max(0),
                            "unseen": sanitize(&value["unseen"]),
                        });
                    }
                    _ => eprintln!("session seen: persisted document is invalid; starting empty"),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => eprintln!("session seen: could not read persisted document: {e}"),
            }
        }
        Self {
            document: Mutex::new(document),
            path,
            hub,
            phases: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn get(&self) -> Value {
        self.document.lock().unwrap().clone()
    }

    /// Fold one observed session row in: a working → done transition marks
    /// the session unseen. Returns whether the document changed.
    pub fn observe(&self, row: &Value) -> bool {
        let Some(id) = row_id(row) else {
            return false;
        };
        let Some(now) = phase(row) else {
            return false;
        };
        let before = {
            let mut phases = self.phases.lock().unwrap();
            if phases.len() >= MAX_TRACKED && !phases.contains_key(id) {
                phases.clear();
            }
            phases.insert(id.to_owned(), now)
        };
        if !(before == Some(Phase::Working) && now == Phase::Done) {
            return false;
        }
        let at = chrono::Utc::now().timestamp_millis();
        self.change(|unseen| {
            if unseen.len() >= MAX_UNSEEN && !unseen.contains_key(id) {
                let oldest = unseen
                    .iter()
                    .min_by_key(|(_, at)| at.as_i64().unwrap_or(0))
                    .map(|(id, _)| id.clone());
                if let Some(oldest) = oldest {
                    unseen.remove(&oldest);
                }
            }
            unseen.insert(id.to_owned(), json!(at));
            true
        })
        .unwrap_or_else(|error| {
            eprintln!("session seen: could not record a finished turn: {error}");
            false
        })
    }

    /// `{sessionId}`: the user looked at this session; clear its mark.
    /// Idempotent: an unmarked session keeps the version and publishes nothing.
    pub fn mark(&self, params: Value) -> Result<Value> {
        let Some(id) = params["sessionId"].as_str() else {
            bail!("sessionSeen.mark requires {{ sessionId }}");
        };
        if !valid_id(id) {
            bail!("sessionSeen.mark: invalid sessionId");
        }
        self.change(|unseen| unseen.remove(id).is_some())?;
        Ok(self.get())
    }

    /// Apply `edit` to the marks; when it reports a change, bump the version,
    /// persist, then publish.
    fn change(&self, edit: impl FnOnce(&mut Map<String, Value>) -> bool) -> Result<bool> {
        let mut document = self.document.lock().unwrap();
        let mut next = document.clone();
        if !edit(next["unseen"].as_object_mut().expect("unseen map")) {
            return Ok(false);
        }
        next["version"] = json!(document["version"].as_i64().unwrap_or(0) + 1);
        if let Some(path) = &self.path {
            super::atomic_json(path, &next, false)?;
        }
        *document = next;
        if let Some(hub) = &self.hub
            && let Err(error) =
                hub.publish(Event::new("sessionSeen.changed", "hub", document.clone()))
        {
            eprintln!(
                "session seen: version {} is committed but its change event could not be queued: {error}",
                document["version"]
            );
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, mode: &str) -> Value {
        json!({"sessionId": id, "mode": mode})
    }

    #[test]
    fn only_an_observed_turn_end_marks_and_opening_clears_it() {
        let seen = SessionSeen::open(None, None);
        // Already idle when first seen: nothing is inferred.
        assert!(!seen.observe(&row("a", "input")));
        assert!(!seen.observe(&row("a", "responding")));
        assert!(seen.observe(&row("a", "input")));
        assert!(seen.get()["unseen"]["a"].is_i64());
        assert_eq!(seen.get()["version"], 1);
        // Repeats of the same idle row change nothing.
        assert!(!seen.observe(&row("a", "input")));
        let doc = seen.mark(json!({"sessionId": "a"})).unwrap();
        assert_eq!(doc, json!({"version": 2, "unseen": {}}));
        // Marking again is a no-op.
        assert_eq!(seen.mark(json!({"sessionId": "a"})).unwrap()["version"], 2);
    }

    #[test]
    fn a_decision_pauses_the_turn_and_partial_rows_are_ignored() {
        let seen = SessionSeen::open(None, None);
        seen.observe(&row("a", "thinking"));
        // An approval is not the end of the turn.
        assert!(!seen.observe(
            &json!({"sessionId":"a","mode":"input","pendingApproval":{"toolName":"Bash"}})
        ));
        // A row without a mode says nothing.
        assert!(!seen.observe(&json!({"sessionId":"a","usage":{}})));
        seen.observe(&row("a", "responding"));
        // Stopping mid-turn also ends it.
        assert!(seen.observe(&row("a", "stopped")));
        // Snake-case ids and states are read too.
        seen.observe(&json!({"session_id":"b","state":"working"}));
        assert!(seen.observe(&json!({"session_id":"b","state":"idle"})));
        assert_eq!(seen.get()["unseen"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn marks_persist_across_reopen_and_bad_input_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-seen.json");
        let seen = SessionSeen::open(Some(path.clone()), None);
        seen.observe(&row("a", "working"));
        seen.observe(&row("a", "done"));
        let reopened = SessionSeen::open(Some(path.clone()), None);
        assert_eq!(reopened.get(), seen.get());
        assert!(reopened.mark(json!({})).is_err());
        assert!(reopened.mark(json!({"sessionId": ""})).is_err());
        std::fs::write(&path, br#"{"version":-3,"unseen":{"ok":5,"bad":"x","":1}}"#).unwrap();
        assert_eq!(
            SessionSeen::open(Some(path), None).get(),
            json!({"version":0,"unseen":{"ok":5}})
        );
    }
}

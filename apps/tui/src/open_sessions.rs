//! The hub's shared sidebar set (`sessionOpen`, see docs/session-roster.md).
//!
//! `open` is every session the sidebar keeps; `ended` marks the open ones
//! that were ended on purpose (by the user or the agent). A stopped session
//! that is open and unmarked was stopped by its app (or host) closing: every
//! client shows it as Paused and resumable. One that is open and marked ended
//! stays listed as stopped. Nothing leaves the set except by archiving (Hide),
//! which the TUI does not write here. The TUI reads the document once per bus
//! (re)connect and follows `sessionOpen.changed`; it keeps no list of its own.
//!
//! Older hubs have no `sessionOpen`. They expose the open, unended ids as
//! `sessions.kept` instead, which carries no version and no marks.

use serde_json::Value;
use std::collections::HashSet;

/// The topic the hub publishes the whole document on after every change.
pub const TOPIC_CHANGED: &str = "sessionOpen.changed";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenSessions {
    /// The document version, for ordering a read against change events.
    /// `None` for the legacy `sessions.kept` answer, which has none.
    pub version: Option<i64>,
    /// Every session the shared sidebar keeps.
    pub ids: HashSet<String>,
    /// Open sessions ended on purpose: listed, but not Paused.
    pub ended: HashSet<String>,
}

impl OpenSessions {
    /// On the shared sidebar (Paused or Ended).
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Open and not ended on purpose: a stopped session reads as Paused.
    pub fn is_paused(&self, id: &str) -> bool {
        self.ids.contains(id) && !self.ended.contains(id)
    }

    /// `{version, open: {sessionId: sinceMs}, ended: {sessionId: endedAtMs}}`.
    /// `None` when `version`/`open` are malformed, so a bad payload never
    /// clears a good set. `ended` is read tolerantly: absent or malformed is
    /// no marks.
    pub fn from_document(doc: &Value) -> Option<Self> {
        let version = doc.get("version")?.as_i64()?;
        let open = doc.get("open")?.as_object()?;
        let ended = doc
            .get("ended")
            .and_then(Value::as_object)
            .map(|ended| ended.keys().filter(|id| !id.is_empty()).cloned().collect())
            .unwrap_or_default();
        Some(Self {
            version: Some(version),
            ids: open.keys().filter(|id| !id.is_empty()).cloned().collect(),
            ended,
        })
    }

    /// The legacy `sessions.kept` answer: `{sessions: [{sessionId, since}]}`,
    /// the open and unended ids, so every one of them reads as Paused.
    pub fn from_kept(doc: &Value) -> Self {
        let ids = doc
            .get("sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| row.get("sessionId").and_then(Value::as_str))
            .filter(|id| !id.is_empty())
            .map(String::from)
            .collect();
        Self {
            version: None,
            ids,
            ended: HashSet::new(),
        }
    }

    /// Whether `next` should replace `held`. A change event must be strictly
    /// newer, so a duplicate or reordered one is a no-op. A read may equal the
    /// held version, but never rolls back an event that landed while it was
    /// in flight. An unversioned (legacy) set never displaces a versioned one.
    pub fn supersedes(&self, held: Option<&OpenSessions>, from_event: bool) -> bool {
        let Some(held) = held else { return true };
        match (held.version, self.version) {
            (_, None) => held.version.is_none(),
            (None, Some(_)) => true,
            (Some(old), Some(new)) => {
                if from_event {
                    new > old
                } else {
                    new >= old
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(version: i64, ids: &[&str]) -> OpenSessions {
        let open: serde_json::Map<String, Value> =
            ids.iter().map(|id| ((*id).to_string(), json!(1))).collect();
        OpenSessions::from_document(&json!({"version": version, "open": open})).unwrap()
    }

    #[test]
    fn parses_the_document_and_rejects_malformed_ones() {
        let set = OpenSessions::from_document(&json!({"version": 3, "open": {"a": 10, "b": 20}}))
            .unwrap();
        assert_eq!(set.version, Some(3));
        assert!(set.contains("a") && set.contains("b") && !set.contains("c"));
        for bad in [
            json!(null),
            json!({"open": {}}),
            json!({"version": "3", "open": {}}),
            json!({"version": 3, "open": ["a"]}),
        ] {
            assert!(OpenSessions::from_document(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn ended_marks_are_tolerant_and_only_unmarked_open_ids_are_paused() {
        let set = OpenSessions::from_document(
            &json!({"version": 1, "open": {"p": 1, "e": 2}, "ended": {"e": 3, "gone": 4}}),
        )
        .unwrap();
        assert!(set.contains("p") && set.contains("e") && !set.contains("gone"));
        assert!(set.is_paused("p"));
        assert!(!set.is_paused("e"), "ended on purpose: listed, not Paused");
        assert!(
            !set.is_paused("gone"),
            "a mark alone does not list a session"
        );
        for ended in [json!(null), json!(["e"]), json!("e")] {
            let set = OpenSessions::from_document(
                &json!({"version": 1, "open": {"e": 1}, "ended": ended}),
            )
            .unwrap();
            assert!(set.is_paused("e"), "malformed ended is no marks");
        }
    }

    #[test]
    fn reads_the_legacy_kept_set_without_a_version() {
        let set = OpenSessions::from_kept(&json!({"sessions": [
            {"sessionId": "a", "since": 1}, {"sessionId": ""}, {"since": 2}
        ]}));
        assert_eq!(set.version, None);
        assert_eq!(set.ids, HashSet::from(["a".to_string()]));
        assert!(set.is_paused("a"), "legacy kept ids are open and unended");
        assert!(OpenSessions::from_kept(&json!({"nope": 1})).ids.is_empty());
    }

    #[test]
    fn events_must_be_newer_and_reads_never_roll_an_event_back() {
        let held = doc(5, &["a"]);
        assert!(doc(6, &[]).supersedes(Some(&held), true));
        assert!(
            !doc(5, &[]).supersedes(Some(&held), true),
            "duplicate event"
        );
        assert!(
            !doc(4, &[]).supersedes(Some(&held), true),
            "reordered event"
        );
        assert!(
            doc(5, &["a"]).supersedes(Some(&held), false),
            "read of the same version"
        );
        assert!(!doc(4, &[]).supersedes(Some(&held), false), "stale read");
        assert!(doc(1, &[]).supersedes(None, true));
    }

    #[test]
    fn a_legacy_set_never_displaces_a_versioned_one() {
        let legacy = OpenSessions::from_kept(&json!({"sessions": [{"sessionId": "a"}]}));
        assert!(!legacy.supersedes(Some(&doc(1, &[])), false));
        assert!(legacy.supersedes(Some(&legacy.clone()), false));
        assert!(doc(1, &[]).supersedes(Some(&legacy), true));
    }
}

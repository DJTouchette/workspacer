//! The hub's shared open set (`sessionOpen`, see docs/session-roster.md).
//!
//! The hub remembers which sessions some client had open and that were not
//! ended on purpose. A stopped session still in that set was stopped by its
//! app (or host) closing, so every client shows it as Paused and resumable
//! instead of hiding it as history. The TUI reads the document once per bus
//! (re)connect and follows `sessionOpen.changed`; it keeps no list of its own.
//!
//! Older hubs have no `sessionOpen`. They expose native's kept-open set as
//! `sessions.kept` instead, which carries no version and is used as is.

use serde_json::Value;
use std::collections::HashSet;

/// The topic the hub publishes the whole document on after every change.
pub const TOPIC_CHANGED: &str = "sessionOpen.changed";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenSessions {
    /// The document version, for ordering a read against change events.
    /// `None` for the legacy `sessions.kept` answer, which has none.
    pub version: Option<i64>,
    pub ids: HashSet<String>,
}

impl OpenSessions {
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// `{version, open: {sessionId: sinceMs}}`. `None` when malformed, so a
    /// bad payload never clears a good set.
    pub fn from_document(doc: &Value) -> Option<Self> {
        let version = doc.get("version")?.as_i64()?;
        let open = doc.get("open")?.as_object()?;
        Some(Self {
            version: Some(version),
            ids: open.keys().filter(|id| !id.is_empty()).cloned().collect(),
        })
    }

    /// The legacy `sessions.kept` answer: `{sessions: [{sessionId, since}]}`.
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
        Self { version: None, ids }
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
    fn reads_the_legacy_kept_set_without_a_version() {
        let set = OpenSessions::from_kept(&json!({"sessions": [
            {"sessionId": "a", "since": 1}, {"sessionId": ""}, {"since": 2}
        ]}));
        assert_eq!(set.version, None);
        assert_eq!(set.ids, HashSet::from(["a".to_string()]));
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

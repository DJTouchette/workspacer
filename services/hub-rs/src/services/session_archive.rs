//! Hub-owned session archive: which sessions the user tucked away.
//!
//! Archiving is a VIEW decision, shared by every client of this hub (native,
//! web, desktop) so archiving in one hides the session in all of them. It is
//! deliberately not a lifecycle operation: nothing here stops, signals or
//! forgets a session, and a running archived session keeps running. The
//! conversation, transcript and daemon row are untouched; restoring is
//! removing the id again.
//!
//! This is not claudemon's `archived` row field. That one is derived (stopped
//! and idle past seven days) and liveness checks read it as "not live"; this
//! set is explicit user intent and can hold a live session.
//!
//! The document is `{version, archived: {sessionId: archivedAtMs}}`. Every
//! accepted change bumps `version` and publishes the whole document as
//! `sessionArchive.changed`, so a client converges from either the event or a
//! `sessionArchive.get` after a reconnect.
use crate::{Handle, protocol::Event};
use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use std::{path::PathBuf, sync::Mutex};

/// Upper bound on archived ids, so a looping caller cannot grow the file
/// without limit. Far above the daemon's retained history.
pub const MAX_ARCHIVED: usize = 10_000;
const MAX_ID_BYTES: usize = 256;

pub struct SessionArchive {
    document: Mutex<Value>,
    path: Option<PathBuf>,
    hub: Option<Handle>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control)
}

/// Keep only well-formed `id → timestamp` entries from a persisted document.
fn sanitize(archived: &Value) -> Map<String, Value> {
    archived
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .filter(|(id, at)| valid_id(id) && at.as_i64().is_some())
                .take(MAX_ARCHIVED)
                .map(|(id, at)| (id.clone(), at.clone()))
                .collect()
        })
        .unwrap_or_default()
}

impl SessionArchive {
    pub fn open(path: Option<PathBuf>, hub: Option<Handle>) -> Self {
        let mut document = json!({"version":0,"archived":{}});
        if let Some(path) = &path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) if value.is_object() => {
                        document = json!({
                            "version": value["version"].as_i64().unwrap_or(0).max(0),
                            "archived": sanitize(&value["archived"]),
                        });
                    }
                    _ => {
                        eprintln!("session archive: persisted document is invalid; starting empty")
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => eprintln!("session archive: could not read persisted document: {e}"),
            }
        }
        Self {
            document: Mutex::new(document),
            path,
            hub,
        }
    }

    pub fn get(&self) -> Value {
        self.document.lock().unwrap().clone()
    }

    /// `{sessionId, archived}`: archive (true) or restore (false) one session.
    /// Idempotent — a no-op change keeps the version and publishes nothing.
    pub fn set(&self, params: Value) -> Result<Value> {
        let Some(id) = params["sessionId"].as_str() else {
            bail!("sessionArchive.set requires {{ sessionId, archived }}");
        };
        let Some(archived) = params["archived"].as_bool() else {
            bail!("sessionArchive.set requires a boolean archived");
        };
        if !valid_id(id) {
            bail!("sessionArchive.set: invalid sessionId");
        }
        let mut document = self.document.lock().unwrap();
        let present = document["archived"].get(id).is_some();
        if present == archived {
            return Ok(document.clone());
        }
        let mut next = document.clone();
        let entries = next["archived"].as_object_mut().expect("archive map");
        if archived {
            if entries.len() >= MAX_ARCHIVED {
                bail!("sessionArchive.set: archive is full ({MAX_ARCHIVED} sessions)");
            }
            entries.insert(id.into(), json!(chrono::Utc::now().timestamp_millis()));
        } else {
            entries.remove(id);
        }
        next["version"] = json!(document["version"].as_i64().unwrap_or(0) + 1);
        // Persist before acknowledging: an archive that would silently come
        // back after a restart is worse than a refused click.
        if let Some(path) = &self.path {
            super::atomic_json(path, &next, false)?;
        }
        *document = next;
        if let Some(hub) = &self.hub
            && let Err(error) = hub.publish(Event::new(
                "sessionArchive.changed",
                "hub",
                document.clone(),
            ))
        {
            eprintln!(
                "session archive: version {} is committed but its change event could not be queued: {error}",
                document["version"]
            );
        }
        Ok(document.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_and_restore_persist_across_reopen_and_keep_versions_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-archive.json");
        let archive = SessionArchive::open(Some(path.clone()), None);
        assert_eq!(archive.get(), json!({"version":0,"archived":{}}));

        let doc = archive
            .set(json!({"sessionId":"a","archived":true}))
            .unwrap();
        assert_eq!(doc["version"], 1);
        assert!(doc["archived"]["a"].as_i64().unwrap() > 0);
        // Idempotent: archiving again changes nothing.
        assert_eq!(
            archive
                .set(json!({"sessionId":"a","archived":true}))
                .unwrap(),
            doc
        );
        archive
            .set(json!({"sessionId":"b","archived":true}))
            .unwrap();

        let reopened = SessionArchive::open(Some(path.clone()), None);
        assert_eq!(reopened.get(), archive.get());
        assert_eq!(reopened.get()["version"], 2);

        let doc = reopened
            .set(json!({"sessionId":"a","archived":false}))
            .unwrap();
        assert_eq!(doc["version"], 3);
        assert!(doc["archived"].get("a").is_none());
        assert!(doc["archived"].get("b").is_some());
        assert_eq!(
            SessionArchive::open(Some(path), None).get()["archived"],
            json!({"b": doc["archived"]["b"]})
        );
    }

    #[test]
    fn invalid_requests_and_documents_change_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-archive.json");
        let archive = SessionArchive::open(Some(path.clone()), None);
        for params in [
            Value::Null,
            json!({}),
            json!({"sessionId":"a"}),
            json!({"sessionId":"a","archived":"yes"}),
            json!({"sessionId":"","archived":true}),
            json!({"sessionId":"a\nb","archived":true}),
            json!({"sessionId":"x".repeat(MAX_ID_BYTES + 1),"archived":true}),
        ] {
            assert!(archive.set(params).is_err());
        }
        assert!(!path.exists());
        assert_eq!(archive.get()["version"], 0);

        std::fs::write(
            &path,
            br#"{"version":4,"archived":{"ok":5,"bad":"x","":7}}"#,
        )
        .unwrap();
        assert_eq!(
            SessionArchive::open(Some(path.clone()), None).get(),
            json!({"version":4,"archived":{"ok":5}})
        );
        std::fs::write(&path, b"[]").unwrap();
        assert_eq!(
            SessionArchive::open(Some(path), None).get(),
            json!({"version":0,"archived":{}})
        );
    }

    #[test]
    fn memory_only_archive_still_works_for_the_running_hub() {
        let archive = SessionArchive::open(None, None);
        let doc = archive
            .set(json!({"sessionId":"live","archived":true}))
            .unwrap();
        assert_eq!(doc["version"], 1);
        assert!(archive.get()["archived"].get("live").is_some());
    }
}

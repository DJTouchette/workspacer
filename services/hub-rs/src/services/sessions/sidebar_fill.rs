//! Sidebar sessions claudemon has evicted from memory.
//!
//! The roster lists every id in the shared sidebar set (`sessionOpen`), but
//! the projection only holds the rows claudemon keeps in memory: stopped
//! sessions idle past seven days are evicted. Those are still answerable
//! from claudemon's permanent history (`GET /sessions/{id}`), so this cache
//! reads each missing sidebar id once and lists it as a stopped row. A
//! sidebar session therefore never vanishes because the engine let its
//! in-memory row go. A 404 means the id is genuinely unknown: not listed.
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::RwLock,
};

/// Reads per pass, so a large migrated sidebar fills over a few passes
/// instead of one burst.
pub(super) const MAX_READS_PER_PASS: usize = 64;
/// Concurrent reads within a pass.
pub(super) const CONCURRENCY: usize = 8;

#[derive(Default)]
pub(super) struct Evicted {
    /// `Some(row)` lists the id; `None` remembers a 404 until the sidebar
    /// changes.
    rows: RwLock<BTreeMap<String, Option<Value>>>,
    /// Sidebar version the negative entries were learned at.
    version: RwLock<Option<i64>>,
}

/// What one fill pass reads, and which cached ids it drops.
pub(super) fn plan(
    open: &BTreeSet<String>,
    resident: &BTreeSet<String>,
    cached: &BTreeMap<String, Option<Value>>,
) -> (Vec<String>, Vec<String>) {
    let read = open
        .iter()
        .filter(|id| !resident.contains(*id) && !cached.contains_key(*id))
        .take(MAX_READS_PER_PASS)
        .cloned()
        .collect();
    let drop = cached
        .keys()
        .filter(|id| !open.contains(*id) || resident.contains(*id))
        .cloned()
        .collect();
    (read, drop)
}

/// A definitive "no such session" from the engine.
pub(super) fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<claudemon::daemon::embedded::CommandRejected>()
        .is_some_and(|rejected| rejected.status == 404)
}

impl Evicted {
    /// Cached rows for ids still missing from the projection.
    pub(super) fn listed(&self, resident: &BTreeSet<String>) -> Vec<Value> {
        self.rows
            .read()
            .unwrap()
            .iter()
            .filter(|(id, _)| !resident.contains(*id))
            .filter_map(|(_, row)| row.clone())
            .collect()
    }

    /// One pass: forget a 404 learned under an older sidebar, drop ids that
    /// left the sidebar or came back into memory, then read missing ones.
    /// Transport errors are retried on the next pass.
    pub(super) async fn fill<F, Fut>(
        &self,
        version: i64,
        open: &BTreeSet<String>,
        resident: &BTreeSet<String>,
        read: F,
    ) where
        F: Fn(String) -> Fut,
        Fut: Future<Output = anyhow::Result<Value>>,
    {
        use futures_util::StreamExt;
        {
            let mut learned = self.version.write().unwrap();
            if *learned != Some(version) {
                self.rows.write().unwrap().retain(|_, row| row.is_some());
                *learned = Some(version);
            }
        }
        let (reads, drop) = plan(open, resident, &self.rows.read().unwrap());
        if !drop.is_empty() {
            let mut rows = self.rows.write().unwrap();
            for id in &drop {
                rows.remove(id);
            }
        }
        if reads.is_empty() {
            return;
        }
        let results: Vec<(String, anyhow::Result<Value>)> = futures_util::stream::iter(reads)
            .map(|id| {
                let pending = read(id.clone());
                async move { (id, pending.await) }
            })
            .buffer_unordered(CONCURRENCY)
            .collect()
            .await;
        let mut rows = self.rows.write().unwrap();
        for (id, result) in results {
            match result {
                Ok(row) if row["session_id"].as_str() == Some(id.as_str()) => {
                    rows.insert(id, Some(super::snapshots::compat(row)));
                }
                Ok(_) => {}
                Err(error) if is_not_found(&error) => {
                    rows.insert(id, None);
                }
                Err(error) => {
                    eprintln!("sidebar session {id} could not be read from history: {error}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ids(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    fn not_found() -> anyhow::Error {
        claudemon::daemon::embedded::CommandRejected {
            status: 404,
            message: "daemon returned 404 Not Found: session not found".into(),
        }
        .into()
    }

    #[test]
    fn plan_reads_missing_sidebar_ids_and_drops_stale_cache() {
        let cached = BTreeMap::from([
            ("gone".to_owned(), Some(json!({}))),
            ("back".to_owned(), Some(json!({}))),
            ("kept".to_owned(), None),
        ]);
        let (read, drop) = plan(
            &ids(&["live", "old", "back", "kept"]),
            &ids(&["live", "back"]),
            &cached,
        );
        assert_eq!(read, vec!["old".to_owned()]);
        assert_eq!(drop, vec!["back".to_owned(), "gone".to_owned()]);
    }

    #[tokio::test]
    async fn evicted_sidebar_ids_list_from_history_and_404s_are_not_listed() {
        let evicted = Evicted::default();
        let open = ids(&["old", "unknown", "flaky", "resident"]);
        let resident = ids(&["resident"]);
        evicted
            .fill(1, &open, &resident, |id| async move {
                match id.as_str() {
                    "old" => Ok(json!({"session_id":"old","mode":"stopped","history":true})),
                    "unknown" => Err(not_found()),
                    _ => Err(anyhow::anyhow!("engine busy")),
                }
            })
            .await;
        let listed = evicted.listed(&resident);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["sessionId"], "old");
        assert_eq!(listed[0]["mode"], "stopped");
        // A 404 is remembered (no re-read) under the same sidebar version;
        // a transport error is retried.
        let reads = std::sync::Mutex::new(Vec::new());
        evicted
            .fill(1, &open, &resident, |id| {
                reads.lock().unwrap().push(id.clone());
                async move { Err::<Value, _>(anyhow::anyhow!("still busy")) }
            })
            .await;
        assert_eq!(*reads.lock().unwrap(), vec!["flaky".to_owned()]);
        // A sidebar change re-asks about the 404.
        reads.lock().unwrap().clear();
        evicted
            .fill(2, &open, &resident, |id| {
                reads.lock().unwrap().push(id.clone());
                async move { Err::<Value, _>(not_found()) }
            })
            .await;
        assert_eq!(
            *reads.lock().unwrap(),
            vec!["flaky".to_owned(), "unknown".to_owned()]
        );
        // Back in memory, or off the sidebar: no longer served from here.
        evicted
            .fill(2, &ids(&["old"]), &ids(&["old"]), |_| async {
                Ok::<Value, anyhow::Error>(json!({}))
            })
            .await;
        assert!(evicted.listed(&BTreeSet::new()).is_empty());
    }
}

//! Observe external edits through the same redacted projections clients read.
//! One joined task owns polling; no file bytes are carried in change events.
use super::{library::Library, snapshots};
use crate::{Handle, client::Client, protocol::Event};
use anyhow::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
    time::Duration,
};

type Rows = Arc<RwLock<BTreeMap<String, Value>>>;
const PERIOD: Duration = Duration::from_secs(2);

pub(crate) struct Watcher {
    library: Arc<Library>,
    rows: Rows,
    owned_engine: bool,
    stopped: tokio::sync::watch::Sender<bool>,
}
fn roots<'a>(rows: impl Iterator<Item = &'a Value>) -> BTreeSet<String> {
    rows.filter(|row| snapshots::live(row) && row.get("hub").is_none_or(Value::is_null))
        .filter_map(|row| row["cwd"].as_str().filter(|cwd| !cwd.is_empty()))
        .map(str::to_owned)
        .chain([String::new()])
        .collect()
}
fn revision(library: &Library, roots: &BTreeSet<String>) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    for cwd in roots {
        let projection = match library.list(&json!({"cwd":cwd})) {
            Ok(projection) => projection,
            // A bad/stale project observation must not suppress global edits
            // or other valid projects. Global store failure remains observable
            // to the caller, which retains its previous complete sample.
            Err(_) if !cwd.is_empty() => continue,
            Err(error) => return Err(error),
        };
        // Frame both values so alternate path/JSON boundaries cannot collide.
        hash.update((cwd.len() as u64).to_le_bytes());
        hash.update(cwd.as_bytes());
        let data = serde_json::to_vec(&projection)?;
        hash.update((data.len() as u64).to_le_bytes());
        hash.update(data);
    }
    Ok(hash.finalize().into())
}
impl Watcher {
    pub(crate) fn new(library: Arc<Library>, rows: Rows, owned_engine: bool) -> Arc<Self> {
        Arc::new(Self {
            library,
            rows,
            owned_engine,
            stopped: tokio::sync::watch::channel(false).0,
        })
    }
    pub(crate) fn close(&self) {
        self.stopped.send_replace(true);
    }
    pub(crate) async fn run(self: Arc<Self>, hub: Handle) -> Result<()> {
        self.run_at(hub, PERIOD).await
    }
    async fn run_at(self: Arc<Self>, hub: Handle, period: Duration) -> Result<()> {
        let mut stopped = self.stopped.subscribe();
        if *stopped.borrow() {
            return Ok(());
        }
        tokio::select! { _ = stopped.changed() => return Ok(()), ready = hub.ready() => { ready?; } }
        let client = Client::connect(&hub).await?;
        let mut previous = None;
        let mut known_roots = BTreeSet::from([String::new()]);
        let mut ticker = tokio::time::interval(period);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { _ = stopped.changed() => return Ok(()), _ = ticker.tick() => {} }
            if self.owned_engine {
                known_roots = roots(self.rows.read().unwrap().values());
            } else {
                let observed = tokio::select! {
                    _ = stopped.changed() => return Ok(()),
                    result = client.call_with_timeout("sessions.snapshots", json!({}), Duration::from_secs(2)) => result,
                };
                if let Ok(Value::Array(rows)) = observed {
                    known_roots = roots(rows.iter());
                }
                // An unavailable source is not evidence that the known fleet
                // vanished. Global edits still refresh while inventory recovers.
            }
            let library = self.library.clone();
            let selected = known_roots.clone();
            // Await even on shutdown: dropping a blocking task would detach it.
            let sampled =
                tokio::task::spawn_blocking(move || revision(&library, &selected)).await?;
            if *stopped.borrow() {
                return Ok(());
            }
            let Ok(next) = sampled else {
                continue;
            };
            if previous.is_some_and(|old| old != next) {
                tokio::select! {
                    _ = stopped.changed() => return Ok(()),
                    result = hub.publish_wait(Event::new("library.changed", "brain", json!({}))) => { result?; }
                }
            }
            previous = Some(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roots_use_process_liveness_and_projection_revision_does_not_follow_secret_changes() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::new(dir.path().join("config"));
        let rows = vec![
            json!({"cwd":"/live","mode":"input"}),
            json!({"cwd":"/live","mode":"approval"}),
            json!({"cwd":"/stopped","mode":"stopped"}),
            json!({"cwd":"/shell","mode":"unknown"}),
            json!({"cwd":"/archived","mode":"input","archived":true}),
            json!({"cwd":"/remote","mode":"input","hub":"peer"}),
            json!({"cwd":"/bad","mode":true}),
            json!({"cwd":"/ended","status":"ended"}),
        ];
        assert_eq!(
            roots(rows.iter()),
            BTreeSet::from(["".into(), "/live".into()])
        );
        let selected = BTreeSet::from([String::new()]);
        library.save(&json!({"scope":"global","id":"mcp","kind":"mcp","title":"MCP","mcp":{"command":"fixture","env":{"KEY":"first"}}})).unwrap();
        let first = revision(&library, &selected).unwrap();
        let malformed = BTreeSet::from([String::new(), "relative/project".into()]);
        assert_eq!(first, revision(&library, &malformed).unwrap());
        library.save(&json!({"scope":"global","id":"mcp","kind":"mcp","title":"MCP","mcp":{"command":"fixture","env":{"KEY":"second"}}})).unwrap();
        assert_eq!(first, revision(&library, &selected).unwrap());
        library.save(&json!({"scope":"global","id":"mcp","kind":"mcp","title":"Changed","mcp":{"command":"fixture","env":{"KEY":"second"}}})).unwrap();
        assert_ne!(first, revision(&library, &selected).unwrap());
    }

    #[tokio::test]
    async fn external_edits_publish_empty_refreshes_and_stopped_projects_leave_polling() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(project.join(".workspacer/library")).unwrap();
        let library = Arc::new(Library::new(directory.path().join("config")));
        library.list(&json!({})).unwrap();
        let rows = Arc::new(RwLock::new(BTreeMap::new()));
        let mut options = crate::Options::default();
        options.control_plane_only = true;
        let hub = crate::Hub::start(options).unwrap();
        hub.ready().await.unwrap();
        let client = Client::connect(&hub.handle()).await.unwrap();
        let mut events = client.events();
        client
            .topics(["library.changed".into()].into())
            .await
            .unwrap();
        let watcher = Watcher::new(library, rows.clone(), true);
        let task = tokio::spawn(
            watcher
                .clone()
                .run_at(hub.handle(), Duration::from_millis(20)),
        );
        let global = directory.path().join("config/library/external.md");
        // Retry different external writes until the initial baseline has landed;
        // no test-only readiness hook or scheduler sleep establishes correctness.
        tokio::time::timeout(Duration::from_secs(3), async {
            for n in 0.. {
                std::fs::write(&global, format!("---\ntitle: Global {n}\n---\nbody\n")).unwrap();
                if let Ok(Ok(event)) =
                    tokio::time::timeout(Duration::from_millis(60), events.recv()).await
                {
                    assert_eq!(event.topic, "library.changed");
                    assert_eq!(event.data, Some(json!({})));
                    break;
                }
            }
        })
        .await
        .unwrap();
        // Let the last write settle, then prove stable projections are quiet.
        tokio::time::sleep(Duration::from_millis(80)).await;
        while events.try_recv().is_ok() {}
        assert!(
            tokio::time::timeout(Duration::from_millis(100), events.recv())
                .await
                .is_err()
        );
        rows.write()
            .unwrap()
            .insert("live".into(), json!({"cwd":project,"mode":"input"}));
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        let item = project.join(".workspacer/library/item.md");
        std::fs::write(&item, "---\ntitle: External\n---\nchanged\n").unwrap();
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        std::fs::remove_file(&item).unwrap();
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        rows.write().unwrap().get_mut("live").unwrap()["mode"] = json!("stopped");
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        std::fs::write(&item, "---\ntitle: Stopped project\n---\nquiet\n").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), events.recv())
                .await
                .is_err()
        );
        watcher.close();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(client);
        hub.shutdown().unwrap();
    }
}

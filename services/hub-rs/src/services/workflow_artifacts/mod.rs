//! Owned Claude artifact polling. Files contribute workflow content only;
//! daemon/session ownership remains authoritative for liveness and provenance.
mod state;
mod telemetry;
use super::{agent_lifecycle::Lifecycle, desktop_workflows::Artifacts, pricing::Pricing};
use crate::{Handle, Options};
use anyhow::Result;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::watch;
pub(crate) type Sink =
    Arc<dyn Fn(String, Value, Value, Vec<Value>) -> BoxFuture<'static, Result<()>> + Send + Sync>;
struct Entry {
    identity: Value,
    watch: state::Watch,
}
pub struct Service {
    artifacts: Artifacts,
    pricing: Pricing,
    rows: Arc<RwLock<BTreeMap<String, Value>>>,
    lifecycle: Option<Arc<Lifecycle>>,
    watches: Mutex<BTreeMap<String, Entry>>,
    cache: Mutex<BTreeMap<String, (Value, Value)>>,
    telemetry: Mutex<telemetry::Telemetry>,
    sink: Mutex<Option<Sink>>,
    stop: watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
/// Merge precisely the existing brain/desktop workflow projection fields.
pub fn merge(mut row: Value, update: &Value) -> Value {
    if !text(&row, "hub").is_empty() {
        return row;
    }
    if update["runs"].is_array() {
        row["workflows"] = update["runs"].clone();
    }
    if let Some(subs) = row["subagents"].as_array() {
        let kept: Vec<_> = subs
            .iter()
            .filter_map(|sub| {
                let id = text(sub, "id")
                    .strip_prefix("agent-")
                    .unwrap_or(text(sub, "id"));
                if update["workflowAgentIds"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&json!(id)))
                {
                    return None;
                }
                let mut sub = sub.clone();
                let activity = &update["subagentActivity"][id];
                for key in ["description", "toolUseId", "model", "lastToolName"] {
                    if !text(activity, key).is_empty() {
                        sub[key] = activity[key].clone();
                    }
                }
                for key in ["tokens", "costUSD", "toolCalls", "lastToolSummary"] {
                    if let Some(v) = activity.get(key) {
                        sub[key] = v.clone();
                    }
                }
                Some(sub)
            })
            .collect();
        row["subagents"] = json!(kept);
    }
    row
}
impl Service {
    pub fn identity(&self, row: &Value) -> Value {
        let id = text(row, "sessionId");
        let generation = self
            .lifecycle
            .as_ref()
            .and_then(|l| l.records().get(id).map(|r| r.generation.clone()));
        json!([
            id,
            row["provider"],
            row["cwd"],
            row["startedAt"],
            row["started_at"],
            row["created_at"],
            row["transcriptPath"],
            row["transcript_path"],
            generation
        ])
    }
    pub fn enrich(&self, row: Value) -> Value {
        let id = text(&row, "sessionId");
        let identity = self.identity(&row);
        let cache = self.cache.lock().unwrap();
        match cache.get(id).filter(|(expected, _)| *expected == identity) {
            Some((_, update)) => merge(row, update),
            None => row,
        }
    }
    pub(crate) fn set_sink(&self, sink: Sink) {
        *self.sink.lock().unwrap() = Some(sink);
    }
    fn scan(&self) -> Vec<(String, Value, Value, Vec<Value>)> {
        let rows = self.rows.read().unwrap().clone();
        let present: BTreeSet<_> = rows.keys().cloned().collect();
        let now = chrono::Utc::now().timestamp_millis();
        let mut watches = self.watches.lock().unwrap();
        let removed: Vec<_> = watches
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            watches.remove(&id);
            self.cache.lock().unwrap().remove(&id);
            self.telemetry.lock().unwrap().forget(&id);
        }
        let mut changes = Vec::new();
        for (id, row) in rows {
            if *self.stop.borrow() {
                break;
            }
            if !text(&row, "hub").is_empty()
                || (!text(&row, "provider").is_empty() && text(&row, "provider") != "claude")
            {
                continue;
            }
            let identity = self.identity(&row);
            if watches
                .get(&id)
                .is_some_and(|entry| entry.identity != identity)
            {
                watches.remove(&id);
                self.cache.lock().unwrap().remove(&id);
                self.telemetry.lock().unwrap().forget(&id);
            }
            if !watches.contains_key(&id) {
                let Some(path) = self.artifacts.locate(&id, &row) else {
                    continue;
                };
                let Some(root) = path.to_str().and_then(|p| p.strip_suffix(".jsonl")) else {
                    continue;
                };
                watches.insert(
                    id.clone(),
                    Entry {
                        identity: identity.clone(),
                        watch: state::Watch::new(root.into(), now),
                    },
                );
            }
            let entry = watches.get_mut(&id).unwrap();
            if row["status"] != "ended" && row["ambientState"] != "idle" {
                entry.watch.last_poke = now;
            }
            if !entry.watch.active(now) {
                continue;
            }
            if let Some(update) = entry.watch.refresh(&self.pricing, now) {
                self.cache
                    .lock()
                    .unwrap()
                    .insert(id.clone(), (identity.clone(), update.clone()));
                let events =
                    self.telemetry
                        .lock()
                        .unwrap()
                        .update(&id, text(&row, "cwd"), &update["runs"]);
                changes.push((id, identity, update, events));
            }
        }
        changes
    }
    pub fn start(self: &Arc<Self>) {
        let mut task = self.task.lock().unwrap();
        if task.is_some() {
            return;
        }
        let service = self.clone();
        let mut stop = self.stop.subscribe();
        *task = Some(tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_millis(2500));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {_=stop.changed()=>break,_=timer.tick()=>{let scanner=service.clone();let changes=match tokio::task::spawn_blocking(move||scanner.scan()).await{Ok(changes)=>changes,Err(error)=>{eprintln!("workflow artifact scan failed: {error}");continue;}};if *stop.borrow(){break;}let sink=service.sink.lock().unwrap().clone();if let Some(sink)=sink{for (id,identity,update,events) in changes{if *stop.borrow(){break;}if let Err(error)=sink(id,identity,update,events).await{eprintln!("workflow artifact publication failed: {error}");}}}}}
            }
        }));
    }
    pub async fn close(&self) {
        self.stop.send_replace(true);
        let task = self.task.lock().unwrap().take();
        if let Some(mut task) = task {
            if tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}
/// Construct before Sessions installs its sink; start after its initial seed.
pub(crate) fn prepare(mut options: Options, _hub: Handle) -> Options {
    if let (Some(home), Some(config)) = (options.home_dir.clone(), options.config_dir.clone()) {
        let (stop, _) = watch::channel(false);
        let artifacts = Artifacts::new(home.clone(), config, super::local_lookup(&options));
        options.workflow_artifacts = Some(Arc::new(Service {
            artifacts,
            pricing: Pricing::new(home),
            rows: options.session_snapshots.clone(),
            lifecycle: options.launch_lifecycle.clone(),
            watches: Default::default(),
            cache: Default::default(),
            telemetry: Default::default(),
            sink: Default::default(),
            stop,
            task: Default::default(),
        }));
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_changes_only_workflow_content_and_never_remote_authority() {
        let row = json!({"sessionId":"local","status":"ended","cwd":"/project","parentSessionId":"parent","subagents":[{"id":"agent-worker","status":"running"},{"id":"plain","status":"done"}]});
        let update = json!({"status":"active","cwd":"/forged","runs":[{"runId":"wf_one"}],"workflowAgentIds":["worker"],"subagentActivity":{"plain":{"tokens":4,"model":"observed","status":"running","parentSessionId":"forged"}}});
        let projected = merge(row.clone(), &update);
        assert_eq!(projected["status"], "ended");
        assert_eq!(projected["cwd"], "/project");
        assert_eq!(projected["parentSessionId"], "parent");
        assert_eq!(projected["subagents"].as_array().unwrap().len(), 1);
        assert_eq!(projected["subagents"][0]["status"], "done");
        assert_eq!(projected["subagents"][0]["tokens"], 4);
        let mut remote = row;
        remote["hub"] = "peer".into();
        assert_eq!(merge(remote.clone(), &update), remote);
    }
}

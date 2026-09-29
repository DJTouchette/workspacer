//! Polling survives atomic replacements and missing files without retaining
//! contents. Named browser leases expire; legacy references remain explicit.
mod sample;
use super::paths;
use crate::{Handle, Options, protocol::Event};
use anyhow::{Result, anyhow, bail};
use sample::Sample;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
struct Watch {
    refs: usize,
    leases: BTreeMap<String, i64>,
    sample: Option<Sample>,
}
pub struct Watches {
    files: Mutex<BTreeMap<PathBuf, Watch>>,
    stop: tokio::sync::watch::Sender<bool>,
}
fn observe(path: &Path) -> Result<Option<Sample>> {
    match sample::sample(path) {
        Ok(value) => Ok(Some(value)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
fn selection(params: &Value) -> Result<(PathBuf, String)> {
    let path = params["path"]
        .as_str()
        .ok_or_else(|| anyhow!("file watch requires path"))?;
    let id = match params.get("watchId") {
        None | Some(Value::Null) => "",
        Some(value) => value
            .as_str()
            .ok_or_else(|| anyhow!("watchId must be text"))?,
    };
    if id.len() > 128 {
        bail!("watchId is too long");
    }
    Ok((paths::canonicalize(Path::new(path))?, id.into()))
}
impl Watches {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            files: Default::default(),
            stop: tokio::sync::watch::channel(false).0,
        })
    }
    pub fn watch(&self, params: &Value, now: i64) -> Result<Value> {
        let (path, id) = selection(params)?;
        let sample = observe(&path)?;
        let mut files = self.files.lock().unwrap();
        if !files.contains_key(&path) && files.len() >= 1024 {
            bail!("too many watched files");
        }
        let watch = files.entry(path.clone()).or_insert_with(|| Watch {
            refs: 0,
            leases: Default::default(),
            sample,
        });
        if id.is_empty() {
            watch.refs = watch
                .refs
                .checked_add(1)
                .ok_or_else(|| anyhow!("too many file references"))?;
        } else {
            if !watch.leases.contains_key(&id) && watch.leases.len() >= 128 {
                bail!("too many file watchers");
            }
            watch.leases.insert(id, now.saturating_add(180_000));
        }
        Ok(json!({"ok":true,"path":path}))
    }
    pub fn unwatch(&self, params: &Value) -> Result<Value> {
        let (path, id) = selection(params)?;
        let mut files = self.files.lock().unwrap();
        if let Some(watch) = files.get_mut(&path) {
            if id.is_empty() {
                watch.refs = watch.refs.saturating_sub(1);
            } else {
                watch.leases.remove(&id);
            }
            if watch.refs == 0 && watch.leases.is_empty() {
                files.remove(&path);
            }
        }
        Ok(json!({"ok":true}))
    }
    pub fn poll(&self, now: i64) -> Vec<Value> {
        let mut files = self.files.lock().unwrap();
        let mut events = Vec::new();
        files.retain(|path, watch| {
            watch.leases.retain(|_, expires| *expires >= now);
            if watch.refs == 0 && watch.leases.is_empty() {
                return false;
            }
            // Canonical selection is frozen. A later link must not retarget it.
            if !paths::canonicalize(path).is_ok_and(|current| current == *path) {
                return false;
            }
            let Ok(current) = observe(path) else {
                return true;
            };
            let kind = match (&watch.sample, &current) {
                (None, None) => None,
                (Some(old), Some(new)) if old.identity != new.identity => Some("rename"),
                (Some(old), Some(new)) if old != new => Some("change"),
                (Some(_), Some(_)) => None,
                _ => Some("rename"),
            };
            watch.sample = current;
            if let Some(kind) = kind {
                events.push(json!({"path":path,"eventType":kind}));
            }
            true
        });
        events
    }
    pub fn close(&self) {
        self.stop.send_replace(true);
    }
    async fn run(self: Arc<Self>, hub: Handle) -> Result<()> {
        let mut stop = self.stop.subscribe();
        if *stop.borrow() {
            return Ok(());
        }
        tokio::select! {ready=hub.ready()=>{ready?;},_=stop.changed()=>return Ok(())};
        let mut timer = tokio::time::interval(Duration::from_millis(500));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        timer.tick().await;
        loop {
            tokio::select! {_=stop.changed()=>return Ok(()),_=timer.tick()=>{
                let observer=self.clone();let sample=tokio::task::spawn_blocking(move||observer.poll(chrono::Utc::now().timestamp_millis()));
                let events=tokio::select!{_=stop.changed()=>return Ok(()),events=sample=>events?};
                for event in events {tokio::select!{_=stop.changed()=>return Ok(()),sent=hub.publish_wait(Event::new("fs.changed","brain",event))=>{sent?;}}}
            }}
        }
    }
}
pub(crate) fn install(
    mut options: Options,
    hub: Handle,
) -> (
    Options,
    Option<Arc<Watches>>,
    Option<tokio::task::JoinHandle<Result<()>>>,
) {
    if options.home_dir.is_none() {
        return (options, None, None);
    }
    let watches = Watches::new();
    for method in ["fs.watch", "fs.unwatch"] {
        let service = watches.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    if method == "fs.watch" {
                        service.watch(&params, chrono::Utc::now().timestamp_millis())
                    } else {
                        service.unwatch(&params)
                    }
                })
                .await?
            }
        });
    }
    let service = watches.clone();
    let task = tokio::spawn(async move { service.run(hub).await });
    (options, Some(watches), Some(task))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_atomic_replacement_and_missing_file_are_distinct_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        let params = json!({"path":path});
        let watches = Watches::new();
        watches.watch(&params, 0).unwrap();
        watches.watch(&params, 0).unwrap();
        std::fs::write(&path, "first").unwrap();
        assert_eq!(watches.poll(1)[0]["eventType"], "rename");
        let before = std::fs::metadata(&path).unwrap();
        let mut replacement = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        use std::io::Write;
        replacement.write_all(b"other").unwrap();
        replacement
            .as_file()
            .set_permissions(before.permissions())
            .unwrap();
        replacement
            .as_file()
            .set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
            .unwrap();
        replacement.persist(&path).unwrap();
        assert_eq!(watches.poll(2)[0]["eventType"], "rename");
        assert!(watches.poll(3).is_empty());
        watches.unwatch(&params).unwrap();
        std::fs::write(&path, "changed length").unwrap();
        assert_eq!(watches.poll(4)[0]["eventType"], "change");
        watches.unwatch(&params).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(watches.poll(5).is_empty());
    }
    #[test]
    fn leases_renew_without_reference_leaks_and_expire() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing");
        let params = json!({"path":path,"watchId":"browser"});
        let watches = Watches::new();
        for now in [0, 1000, 2000] {
            watches.watch(&params, now).unwrap();
        }
        // Watches key by the canonical spelling (/var may alias /private/var).
        let canonical = crate::services::paths::canonicalize(&path).unwrap();
        assert_eq!(watches.files.lock().unwrap()[&canonical].refs, 0);
        assert_eq!(watches.files.lock().unwrap()[&canonical].leases.len(), 1);
        watches.poll(182000);
        assert_eq!(watches.files.lock().unwrap().len(), 1);
        watches.poll(182001);
        assert!(watches.files.lock().unwrap().is_empty());
        watches.watch(&params, 200000).unwrap();
        watches.unwatch(&params).unwrap();
        assert!(watches.files.lock().unwrap().is_empty());
    }
    #[test]
    fn path_and_lease_caps_allow_renewal_but_refuse_new_entries() {
        let directory = tempfile::tempdir().unwrap();
        let watches = Watches::new();
        let path = directory.path().join("watched");
        for index in 0..128 {
            watches
                .watch(&json!({"path":path,"watchId":format!("lease-{index}")}), 0)
                .unwrap();
        }
        watches
            .watch(&json!({"path":path,"watchId":"lease-0"}), 1)
            .unwrap();
        assert!(
            watches
                .watch(&json!({"path":path,"watchId":"one-too-many"}), 1)
                .unwrap_err()
                .to_string()
                .contains("too many file watchers")
        );
        for index in 1..1024 {
            watches
                .watch(
                    &json!({"path":directory.path().join(format!("path-{index}"))}),
                    0,
                )
                .unwrap();
        }
        watches.watch(&json!({"path":path}), 1).unwrap();
        assert!(
            watches
                .watch(&json!({"path":directory.path().join("extra")}), 1)
                .unwrap_err()
                .to_string()
                .contains("too many watched files")
        );
        assert!(
            watches
                .watch(&json!({"path":path,"watchId":"é".repeat(65)}), 1)
                .unwrap_err()
                .to_string()
                .contains("too long")
        );
    }
    #[cfg(unix)]
    #[test]
    fn swapped_symlink_drops_observer_without_external_metadata_event() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("observed");
        let outside = directory.path().join("other");
        std::fs::write(&path, "old").unwrap();
        std::fs::write(&outside, "outside").unwrap();
        let watches = Watches::new();
        watches.watch(&json!({"path":path}), 0).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(watches.poll(1).is_empty());
        assert!(watches.files.lock().unwrap().is_empty());
    }
}

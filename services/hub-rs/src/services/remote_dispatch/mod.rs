//! Durable, one-hop worker admissions and return receipts. Origin paths are
//! opaque; only the execution host may inspect or allocate its filesystem.
mod model;
mod origin;
pub(crate) mod paired;
pub mod readiness;
mod receiver;
pub mod return_channel;
pub(crate) mod runtime;
use anyhow::Result;
pub use model::*;
pub use origin::{Delivery, Link, Origin};
pub use receiver::{Execution, Receiver, RemoteAdmission};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};
struct Journal<T> {
    path: PathBuf,
    rows: Mutex<BTreeMap<String, T>>,
}
impl<T: Clone + Serialize + DeserializeOwned> Journal<T> {
    fn open(path: PathBuf, key: impl Fn(&T) -> Result<String>) -> Result<Self> {
        let rows: Vec<T> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        let mut indexed = BTreeMap::new();
        for row in rows {
            let id = key(&row)?;
            anyhow::ensure!(
                indexed.insert(id, row).is_none(),
                "duplicate dispatch journal identity"
            );
        }
        Ok(Self {
            path,
            rows: Mutex::new(indexed),
        })
    }
    fn open_legacy(
        path: PathBuf,
        legacy: PathBuf,
        identity_field: &str,
        other_field: &str,
        key: impl Fn(&T) -> Result<String>,
    ) -> Result<Self> {
        if path.exists() || !legacy.exists() {
            return Self::open(path, key);
        }
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&legacy)?)?;
        if rows.iter().all(|row| row.get(identity_field).is_some()) {
            let mut journal = Self::open(legacy, key)?;
            journal.path = path;
            journal.change(|_| Ok(()))?;
            return Ok(journal);
        }
        if rows.iter().all(|row| row.get(other_field).is_some()) {
            return Self::open(path, key);
        }
        anyhow::bail!("remote dispatch legacy journal has an unknown or mixed schema")
    }
    fn change<R>(&self, update: impl FnOnce(&mut BTreeMap<String, T>) -> Result<R>) -> Result<R> {
        let mut rows = self.rows.lock().unwrap();
        let mut next = rows.clone();
        let result = update(&mut next)?;
        super::atomic_json(
            &self.path,
            &serde_json::to_value(next.values().collect::<Vec<_>>())?,
            true,
        )?;
        *rows = next;
        Ok(result)
    }
    fn list(&self) -> Vec<T> {
        self.rows.lock().unwrap().values().cloned().collect()
    }
    fn get(&self, id: &str) -> Option<T> {
        self.rows.lock().unwrap().get(id).cloned()
    }
}
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub(crate) fn owned_worktree(root: &Path, id: &str) -> PathBuf {
    root.join("dispatch-worktrees").join(id)
}
#[cfg(test)]
mod tests;

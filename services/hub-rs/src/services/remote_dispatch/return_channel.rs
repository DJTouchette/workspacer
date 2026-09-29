//! Host-owned return transport; caller payloads cannot select this channel.
use super::{Kind, Receiver};
use anyhow::Result;
use futures_util::future::BoxFuture;
use serde_json::Value;
pub trait ReturnChannel: Send + Sync {
    fn owns(&self, session: &str) -> bool;
    fn report(&self, session: String, kind: Kind, entry: Value) -> BoxFuture<'_, Result<bool>>;
}
impl ReturnChannel for Receiver {
    fn owns(&self, session: &str) -> bool {
        self.dispatch_for(session).is_some()
    }
    fn report(&self, session: String, kind: Kind, entry: Value) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move { Receiver::report(self, &session, kind, entry).await })
    }
}

#[cfg(test)]
pub(crate) struct TestReturns {
    pub entries: std::sync::Mutex<Vec<(Kind, Value)>>,
}
#[cfg(test)]
impl TestReturns {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            entries: Default::default(),
        })
    }
}
#[cfg(test)]
impl ReturnChannel for TestReturns {
    fn owns(&self, session: &str) -> bool {
        session == "worker"
    }
    fn report(&self, session: String, kind: Kind, entry: Value) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            anyhow::ensure!(
                session == "worker",
                "only a known remote worker has a return channel"
            );
            let mut entries = self.entries.lock().unwrap();
            if entries.iter().any(|(kind, _)| kind.terminal()) {
                return Ok(false);
            }
            entries.push((kind, entry));
            Ok(true)
        })
    }
}

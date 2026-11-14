//! Periodic host chores of a backend that owns its engine: keep-warm pings
//! and worktree artifact cleanup. A hub that merely attaches to someone
//! else's engine never runs them; the owner of that engine does.
use super::{
    config::Config,
    keep_warm::{KeepWarm, OwnedEngine},
    worktrees::artifact_cleanup_scheduler::{Scheduler, owned_pass, process_in_flight},
};
use claudemon::daemon::embedded::EmbeddedClient;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Default)]
pub(crate) struct HostChores {
    keep_warm: Option<tokio::task::JoinHandle<()>>,
    cleanup: Option<Scheduler>,
}
impl HostChores {
    /// Call once the owned engine and hub are ready, inside a Tokio runtime.
    /// Both chores read `config.yaml` on every pass. Without a configuration
    /// directory there are no settings to honor and nothing starts.
    pub(crate) fn start(
        engine: EmbeddedClient,
        config_dir: Option<&Path>,
        home: Option<PathBuf>,
    ) -> Self {
        let Some(config_dir) = config_dir else {
            tracing::info!("host chores not started: the backend has no configuration directory");
            return Self::default();
        };
        let config = Arc::new(Config::open(config_dir.join("config.yaml")));
        let keep_warm = match OwnedEngine::new(engine.clone()) {
            Ok(owned) => {
                let config = config.clone();
                Some(tokio::spawn(
                    KeepWarm::new(owned, move || config.get()).run(),
                ))
            }
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "keep-warm unavailable");
                None
            }
        };
        // Cleanup proves a directory is the one it inspected by device and
        // inode. Where those are unavailable every worktree would be refused,
        // so the loop is not started rather than reporting that forever.
        let cleanup = if cfg!(unix) {
            Some(Scheduler::start(
                move || config.get(),
                home,
                process_in_flight(),
                owned_pass(engine),
            ))
        } else {
            tracing::info!("worktree artifact cleanup is not available on this platform");
            None
        };
        Self { keep_warm, cleanup }
    }
    #[cfg(test)]
    pub(crate) fn started(&self) -> bool {
        self.keep_warm.is_some() && self.cleanup.is_some() == cfg!(unix)
    }
    fn stop_keep_warm(&mut self) {
        if let Some(task) = self.keep_warm.take() {
            task.abort();
        }
    }
    /// Stop both loops and let a running cleanup pass release its lease.
    pub(crate) async fn shutdown(mut self) {
        self.stop_keep_warm();
        if let Some(cleanup) = self.cleanup.take() {
            cleanup.shutdown().await;
        }
    }
}
impl Drop for HostChores {
    fn drop(&mut self) {
        // The cleanup scheduler stops itself when dropped.
        self.stop_keep_warm();
    }
}

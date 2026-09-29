//! Explicitly opt-in fixture construction; excluded from production builds.
use crate::{
    Handle, Options,
    services::{
        remote_dispatch::{Execution, Receiver},
        routing::RoutingService,
    },
};
use anyhow::Result;
use std::sync::Arc;
pub type AfterServices = Arc<dyn Fn(Options) -> Options + Send + Sync>;
pub fn after_services(
    mut options: Options,
    configure: impl Fn(Options) -> Options + Send + Sync + 'static,
) -> Options {
    options.test_after_services = Some(Arc::new(configure));
    options
}
pub fn with_execution(mut options: Options, execution: Arc<dyn Execution>) -> Options {
    options.remote_dispatch_execution = Some(execution);
    options
}
pub fn with_routing(mut options: Options, routing: Arc<RoutingService>) -> Options {
    options.routing = Some(routing);
    options
}
pub async fn receiver(handle: &Handle) -> Result<Arc<Receiver>> {
    handle
        .test_receiver()
        .await?
        .ok_or_else(|| anyhow::anyhow!("fixture receiver not installed"))
}

pub fn wake_returns(
    wakes: Arc<crate::services::wakes::Wakes>,
    receiver: Arc<Receiver>,
) -> Arc<crate::services::wakes::Wakes> {
    crate::services::wakes::Wakes::fixture_remote(wakes, receiver)
}
pub fn progress_returns(
    progress: crate::services::progress::Progress,
    receiver: Arc<Receiver>,
) -> crate::services::progress::Progress {
    progress.fixture_remote(receiver)
}
pub async fn git(cwd: &std::path::Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = tokio::process::Command::new("git");
    command.current_dir(cwd);
    for value in crate::services::files::GIT_NO_EXEC {
        command.args(["-c", value]);
    }
    command.args(args);
    let output = crate::services::owned_process::capture(
        &mut command,
        1024 * 1024,
        std::time::Duration::from_secs(15),
    )
    .await?;
    anyhow::ensure!(
        output.status.success(),
        "fixture git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}

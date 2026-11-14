//! Automatic worktree artifact cleanup for a backend that owns its engine.
//!
//! The first pass runs one minute after readiness, then every 15 minutes.
//! Configuration is read each pass, so disabling needs no restart. One pass
//! runs at a time per process. Shutdown stops scheduling, refuses later state
//! loads and removals, and waits briefly for a running pass to release its
//! maintenance lease; it cannot interrupt a removal already in the kernel.
use super::artifact_cleanup::{
    CleanupReport, Options, cleanup_worktree_artifacts, load_worktree_cleanup_state,
};
use anyhow::{Result, anyhow, bail};
use claudemon::daemon::embedded::{EmbeddedClient, Status as EngineStatus};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const START_DELAY: Duration = Duration::from_secs(60);
pub const INTERVAL: Duration = Duration::from_secs(15 * 60);
/// How long shutdown waits for a running pass to notice and release its lease.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// `agents.artifactCleanup.minAgeHours`, default one hour. `requested` (a
/// command-line value) wins over the file.
pub fn min_age(config: &Value, requested: Option<f64>) -> Result<Duration> {
    let hours = match (
        requested,
        &config["agents"]["artifactCleanup"]["minAgeHours"],
    ) {
        (Some(hours), _) => Some(hours),
        (None, Value::Null) => Some(1.0),
        (None, value) => value.as_f64(),
    };
    hours
        .filter(|hours| hours.is_finite() && *hours >= 0.0)
        .and_then(|hours| Duration::try_from_secs_f64(hours * 60.0 * 60.0).ok())
        .ok_or_else(|| anyhow!("agents.artifactCleanup.minAgeHours must be a nonnegative number"))
}

/// `agents.worktreeRoot`, default `<home>/.workspacer/worktrees`. The home is
/// the backend's configured one, never read from the process environment here.
pub fn root(config: &Value, home: Option<&Path>) -> Result<PathBuf> {
    let configured = match &config["agents"]["worktreeRoot"] {
        Value::Null => "",
        Value::String(root) => root.trim(),
        _ => bail!("agents.worktreeRoot must be a string"),
    };
    if configured.is_empty() {
        return Ok(home
            .ok_or_else(|| {
                anyhow!("no home directory is configured for the default worktree root")
            })?
            .join(".workspacer")
            .join("worktrees"));
    }
    let root = PathBuf::from(configured);
    if !root.is_absolute() {
        bail!("agents.worktreeRoot must be an absolute path");
    }
    Ok(root)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    pub root: PathBuf,
    pub min_age: Duration,
}

/// The automatic policy; `None` when `agents.artifactCleanup.enabled` is set
/// to anything but true.
pub fn policy(config: &Value, home: Option<&Path>) -> Result<Option<Policy>> {
    let cleanup = &config["agents"]["artifactCleanup"];
    if cleanup
        .get("enabled")
        .is_some_and(|enabled| *enabled != Value::Bool(true))
    {
        return Ok(None);
    }
    Ok(Some(Policy {
        min_age: min_age(config, None)?,
        root: root(config, home)?,
    }))
}

/// Held for the whole of one pass, including by a pass that outlives the
/// scheduler that started it.
pub struct InFlight(Arc<AtomicBool>);
impl InFlight {
    pub fn take(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag.clone()))
    }
}
impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
/// One guard per process, as every owned backend in it shares one disk.
pub fn process_in_flight() -> Arc<AtomicBool> {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    FLAG.get_or_init(Default::default).clone()
}

pub struct Request {
    pub policy: Policy,
    /// Set by shutdown; the pass must load no further state and remove nothing.
    pub stop: Arc<AtomicBool>,
    pub in_flight: InFlight,
}
pub type Pass = BoxFuture<'static, Result<CleanupReport>>;

pub struct Scheduler {
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}
impl Scheduler {
    /// Must be called inside a Tokio runtime. `run` performs one pass.
    pub fn start(
        config: impl Fn() -> Value + Send + 'static,
        home: Option<PathBuf>,
        in_flight: Arc<AtomicBool>,
        run: impl Fn(Request) -> Pass + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn({
            let (stop, in_flight) = (stop.clone(), in_flight.clone());
            async move {
                tokio::time::sleep(START_DELAY).await;
                // The first tick completes immediately.
                let mut interval = tokio::time::interval(INTERVAL);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let Some(guard) = InFlight::take(&in_flight) else {
                        continue;
                    };
                    let policy = match policy(&config(), home.as_deref()) {
                        Ok(Some(policy)) => policy,
                        Ok(None) => continue,
                        Err(error) => {
                            tracing::warn!(error = %format!("{error:#}"), "worktree artifact cleanup unavailable");
                            continue;
                        }
                    };
                    let pass = run(Request {
                        policy,
                        stop: stop.clone(),
                        in_flight: guard,
                    });
                    match pass.await {
                        Ok(report) if report.is_quiet() => {}
                        Ok(report) => tracing::warn!(
                            report = %serde_json::to_string(&report).unwrap_or_default(),
                            "worktree artifact cleanup"
                        ),
                        Err(_) if stop.load(Ordering::Acquire) => {}
                        Err(error) => {
                            tracing::warn!(error = %format!("{error:#}"), "worktree artifact cleanup unavailable")
                        }
                    }
                }
            }
        });
        Self {
            stop,
            in_flight,
            task,
        }
    }
    /// No further pass starts; a running one loads no more state and removes
    /// nothing more.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.task.abort();
    }
    /// Stop, then give a running pass a bounded time to release its lease.
    pub async fn shutdown(self) {
        self.stop();
        let waited = tokio::time::timeout(SHUTDOWN_GRACE, async {
            while self.in_flight.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        });
        if waited.await.is_err() {
            tracing::warn!(
                "a worktree artifact cleanup pass is still running at shutdown; if the process exits now its .workspacer-maintenance.lock stays until removed by hand"
            );
        }
    }
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One applying pass against the owned engine, on a thread of its own: the
/// walks and the removal block, and must neither stall a shared runtime nor
/// be counted against its shutdown. State comes from the engine's loopback
/// API, whose health response carries the maintenance-support marker.
pub fn owned_pass(engine: EmbeddedClient) -> impl Fn(Request) -> Pass + Send + 'static {
    move |request| {
        let engine = engine.clone();
        let (done, report) = tokio::sync::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("artifact-cleanup".into())
            .spawn(move || {
                let Request {
                    policy,
                    stop,
                    in_flight,
                } = request;
                let pass = async {
                    let client = reqwest::Client::builder()
                        .no_proxy()
                        .redirect(reqwest::redirect::Policy::none())
                        .build()?;
                    let options = Options {
                        root: &policy.root,
                        apply: true,
                        min_age: policy.min_age,
                        stop: Some(&stop),
                    };
                    cleanup_worktree_artifacts(options, || async {
                        if stop.load(Ordering::Acquire) {
                            bail!("Artifact cleanup stopped");
                        }
                        // An engine that is not ready is unknown state, never
                        // an empty fleet.
                        let address = match &*engine.status().borrow() {
                            EngineStatus::Ready(ready) => {
                                crate::net_address::dial_addr(ready.api_addr)
                            }
                            _ => bail!("Daemon state unavailable; cleanup skipped"),
                        };
                        load_worktree_cleanup_state(&client, &format!("http://{address}")).await
                    })
                    .await
                };
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| runtime.block_on(pass));
                // Released before the report is observable.
                drop(in_flight);
                let _ = done.send(result);
            });
        Box::pin(async move {
            spawned?;
            report
                .await
                .map_err(|_| anyhow!("worktree artifact cleanup pass ended without a report"))?
        })
    }
}

// Fixture roots are POSIX absolute paths.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn report() -> CleanupReport {
        CleanupReport {
            root: "/worktrees".into(),
            apply: true,
            scanned_worktrees: 0,
            removed_bytes: 0,
            reclaimable_bytes: 0,
            artifacts: vec![],
            skipped: vec![],
            errors: vec![],
        }
    }
    /// A pass's policy and the stop flag it was handed.
    type Recorded = (Policy, Arc<AtomicBool>);
    #[derive(Clone, Default)]
    struct Runs(Arc<Mutex<Vec<Recorded>>>);
    impl Runs {
        fn count(&self) -> usize {
            self.0.lock().unwrap().len()
        }
        fn last(&self) -> Policy {
            self.0.lock().unwrap().last().unwrap().0.clone()
        }
        fn record(&self, request: &Request) {
            self.0
                .lock()
                .unwrap()
                .push((request.policy.clone(), request.stop.clone()));
        }
        /// A pass that completes at once with `result`.
        fn completing(
            &self,
            result: impl Fn(usize) -> Result<CleanupReport> + Send + 'static,
        ) -> impl Fn(Request) -> Pass + Send + 'static {
            let runs = self.clone();
            move |request| {
                runs.record(&request);
                let result = result(runs.count());
                Box::pin(async move { result })
            }
        }
    }
    fn configured(config: &Arc<Mutex<Value>>) -> impl Fn() -> Value + Send + 'static {
        let config = config.clone();
        move || config.lock().unwrap().clone()
    }
    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }
    async fn advance(duration: Duration) {
        // A scheduler started just now must arm its timer at the current
        // instant, and afterwards observe the new one and finish its pass.
        settle().await;
        tokio::time::advance(duration).await;
        settle().await;
    }

    #[tokio::test(start_paused = true)]
    async fn starts_after_the_startup_delay_and_uses_current_configuration_each_interval() {
        let config = Arc::new(Mutex::new(json!({"agents": {
            "worktreeRoot": "/first", "artifactCleanup": {"enabled": true, "minAgeHours": 2}
        }})));
        let runs = Runs::default();
        let _scheduler = Scheduler::start(
            configured(&config),
            Some("/home/fixture".into()),
            Default::default(),
            runs.completing(|_| Ok(report())),
        );
        advance(START_DELAY - Duration::from_millis(1)).await;
        assert_eq!(runs.count(), 0);
        advance(Duration::from_millis(1)).await;
        assert_eq!(
            runs.last(),
            Policy {
                root: "/first".into(),
                min_age: Duration::from_secs(7200)
            }
        );
        *config.lock().unwrap() = json!({"agents": {
            "worktreeRoot": "/second", "artifactCleanup": {"enabled": true, "minAgeHours": 4}
        }});
        advance(INTERVAL).await;
        assert_eq!(runs.count(), 2);
        assert_eq!(
            runs.last(),
            Policy {
                root: "/second".into(),
                min_age: Duration::from_secs(14_400)
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn honors_disabling_without_needing_a_restart() {
        let config = Arc::new(Mutex::new(
            json!({"agents": {"artifactCleanup": {"enabled": false}}}),
        ));
        let runs = Runs::default();
        let _scheduler = Scheduler::start(
            configured(&config),
            Some("/home/fixture".into()),
            Default::default(),
            runs.completing(|_| Ok(report())),
        );
        advance(START_DELAY).await;
        assert_eq!(runs.count(), 0);
        *config.lock().unwrap() = json!({"agents": {"artifactCleanup": {"enabled": true}}});
        advance(INTERVAL).await;
        assert_eq!(runs.count(), 1);
        // Defaults: one hour, under the backend's configured home.
        assert_eq!(
            runs.last(),
            Policy {
                root: "/home/fixture/.workspacer/worktrees".into(),
                min_age: Duration::from_secs(3600)
            }
        );
        *config.lock().unwrap() = json!({"agents": {"artifactCleanup": {"enabled": false}}});
        advance(INTERVAL).await;
        assert_eq!(runs.count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_overlap_runs_and_shutdown_prevents_further_state_reads_or_passes() {
        let runs = Runs::default();
        let (finish, finished) = tokio::sync::oneshot::channel::<()>();
        let finished = Arc::new(Mutex::new(Some(finished)));
        let in_flight: Arc<AtomicBool> = Default::default();
        let scheduler = Scheduler::start(
            || json!({}),
            Some("/home/fixture".into()),
            in_flight.clone(),
            {
                let runs = runs.clone();
                move |request| {
                    runs.record(&request);
                    let finished = finished.lock().unwrap().take();
                    // Like the real pass, the worker (not the scheduler's
                    // future) owns the in-flight guard until it finishes.
                    tokio::spawn(async move {
                        let _guard = request.in_flight;
                        if let Some(finished) = finished {
                            let _ = finished.await;
                        }
                    });
                    Box::pin(std::future::pending())
                }
            },
        );
        advance(START_DELAY + 2 * INTERVAL).await;
        assert_eq!(runs.count(), 1);
        assert!(in_flight.load(Ordering::Acquire));
        // A second scheduler in the process cannot start a concurrent pass.
        let other = Runs::default();
        let second = Scheduler::start(
            || json!({}),
            Some("/home/fixture".into()),
            in_flight.clone(),
            other.completing(|_| Ok(report())),
        );
        advance(START_DELAY + INTERVAL).await;
        assert_eq!(other.count(), 0);
        let stop = runs.0.lock().unwrap()[0].1.clone();
        assert!(!stop.load(Ordering::Acquire));
        scheduler.stop();
        assert!(stop.load(Ordering::Acquire), "the pass must refuse state");
        // The running pass still holds the guard after its scheduler stopped.
        advance(INTERVAL).await;
        assert_eq!(other.count(), 0);
        finish.send(()).unwrap();
        advance(INTERVAL).await;
        assert_eq!(runs.count(), 1);
        assert_eq!(other.count(), 1);
        second.shutdown().await;
        assert!(!in_flight.load(Ordering::Acquire));
        advance(2 * INTERVAL).await;
        assert_eq!(other.count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_failures_on_the_next_interval() {
        let runs = Runs::default();
        let in_flight: Arc<AtomicBool> = Default::default();
        let _scheduler = Scheduler::start(
            || json!({}),
            Some("/home/fixture".into()),
            in_flight.clone(),
            runs.completing(|count| {
                if count == 1 {
                    bail!("daemon unavailable")
                }
                Ok(report())
            }),
        );
        advance(START_DELAY).await;
        assert_eq!(runs.count(), 1);
        assert!(!in_flight.load(Ordering::Acquire));
        advance(INTERVAL).await;
        assert_eq!(runs.count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn invalid_configuration_never_starts_a_pass() {
        for agents in [
            json!({"artifactCleanup": {"minAgeHours": -1}}),
            json!({"artifactCleanup": {"minAgeHours": "2"}}),
            json!({"artifactCleanup": {"minAgeHours": 1e300}}),
            json!({"artifactCleanup": {"enabled": "yes"}}),
            json!({"artifactCleanup": {"enabled": null}}),
            json!({"worktreeRoot": "relative/worktrees"}),
            json!({"worktreeRoot": 7}),
        ] {
            let runs = Runs::default();
            let in_flight: Arc<AtomicBool> = Default::default();
            let config = json!({"agents": agents});
            let _scheduler = Scheduler::start(
                move || config.clone(),
                Some("/home/fixture".into()),
                in_flight.clone(),
                runs.completing(|_| Ok(report())),
            );
            advance(START_DELAY + INTERVAL).await;
            assert_eq!(runs.count(), 0, "{agents}");
            assert!(!in_flight.load(Ordering::Acquire), "{agents}");
        }
        // No configured home and no configured root: nowhere to look.
        let runs = Runs::default();
        let _scheduler = Scheduler::start(
            || json!({}),
            None,
            Default::default(),
            runs.completing(|_| Ok(report())),
        );
        advance(START_DELAY).await;
        assert_eq!(runs.count(), 0);
    }

    #[test]
    fn command_line_values_override_the_file_and_are_validated() {
        let config = json!({"agents": {"artifactCleanup": {"minAgeHours": 3}}});
        assert_eq!(min_age(&config, None).unwrap(), Duration::from_secs(10_800));
        assert_eq!(min_age(&config, Some(0.0)).unwrap(), Duration::ZERO);
        assert_eq!(
            min_age(&json!({}), None).unwrap(),
            Duration::from_secs(3600)
        );
        for invalid in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(min_age(&config, Some(invalid)).is_err());
        }
        assert_eq!(
            root(
                &json!({"agents": {"worktreeRoot": "  "}}),
                Some(Path::new("/home/x"))
            )
            .unwrap(),
            Path::new("/home/x/.workspacer/worktrees")
        );
    }
}

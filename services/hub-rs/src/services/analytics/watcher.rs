use super::*;
use claudemon::daemon::embedded::{Command, EmbeddedClient};
use std::time::Duration;
pub struct Watcher {
    service: Arc<Analytics>,
    engine: Option<EmbeddedClient>,
    profiles: Arc<super::super::profiles::Profiles>,
    home: PathBuf,
    cancelled: tokio::sync::watch::Sender<bool>,
    lifecycle: Option<Arc<super::super::agent_lifecycle::Lifecycle>>,
    replacements: Option<Arc<super::super::manager_replacements::ReplacementState>>,
}
impl Watcher {
    pub fn new(
        service: Arc<Analytics>,
        engine: Option<EmbeddedClient>,
        profiles: Arc<super::super::profiles::Profiles>,
        home: PathBuf,
    ) -> Arc<Self> {
        let (cancelled, _) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            service,
            engine,
            profiles,
            home,
            cancelled,
            lifecycle: None,
            replacements: None,
        })
    }
    /// The durable store this watcher feeds.
    pub fn store(&self) -> Arc<Analytics> {
        self.service.clone()
    }
    pub fn close(&self) {
        self.cancelled.send_replace(true);
    }
    fn roots(&self) -> Vec<PathBuf> {
        let default = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".claude"));
        let mut roots = vec![default.join("projects")];
        for p in self
            .profiles
            .list()
            .into_iter()
            .filter(|p| p.provider.is_empty() || p.provider == "claude")
        {
            if !p.config_dir.is_empty() {
                let root = if p.config_dir == "~" {
                    self.home.clone()
                } else if let Some(rest) = p.config_dir.strip_prefix("~/") {
                    self.home.join(rest)
                } else {
                    PathBuf::from(p.config_dir)
                };
                roots.push(root.join("projects"));
            }
        }
        roots
    }
    /// Record every session the engine knows into the analytics store, then
    /// answer `method` from it.
    pub async fn query(self: &Arc<Self>, method: &'static str, params: Value) -> Result<Value> {
        self.timed(method, params, Pass::Query).await
    }
    /// Every pass is logged by its cost: a slow one is one the disk feels.
    async fn timed(
        self: &Arc<Self>,
        method: &'static str,
        params: Value,
        pass: Pass,
    ) -> Result<Value> {
        let started = std::time::Instant::now();
        let sessions = std::sync::atomic::AtomicUsize::new(0);
        let result = self.observe(method, params, &sessions, pass).await;
        let took = started.elapsed();
        let sessions = sessions.into_inner();
        if took >= Duration::from_secs(1) {
            tracing::warn!(
                method,
                sessions,
                took_ms = took.as_millis() as u64,
                ok = result.is_ok(),
                "slow analytics pass"
            );
        } else if took >= Duration::from_millis(200) {
            tracing::info!(
                method,
                sessions,
                took_ms = took.as_millis() as u64,
                ok = result.is_ok(),
                "analytics pass"
            );
        } else {
            tracing::debug!(
                method,
                sessions,
                took_ms = took.as_millis() as u64,
                ok = result.is_ok(),
                "analytics pass"
            );
        }
        result
    }
    async fn observe(
        self: &Arc<Self>,
        method: &'static str,
        params: Value,
        sessions: &std::sync::atomic::AtomicUsize,
        pass: Pass,
    ) -> Result<Value> {
        let engine = self
            .engine
            .as_ref()
            .context("analytics session source unavailable")?;
        // The closing pass must not hold up shutdown behind a stuck engine.
        let budget = if matches!(pass, Pass::Final) { 2 } else { 15 };
        let raw = tokio::time::timeout(
            Duration::from_secs(budget),
            engine.request(Command::Request {
                method: "GET".into(),
                path: if matches!(pass, Pass::Background { archived: false } | Pass::Final) {
                    "/sessions?include_empty=true".into()
                } else {
                    "/sessions?include_archived=true&include_empty=true".into()
                },
                payload: None,
            }),
        )
        .await
        .context("analytics session source timed out")??;
        let rows = raw
            .as_array()
            .context("analytics session source returned invalid rows")?
            .iter()
            .map(|row| {
                super::super::snapshots::with_host_metadata(
                    super::super::snapshots::compat(row.clone()),
                    self.lifecycle.as_deref(),
                    self.replacements.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        sessions.store(rows.len(), std::sync::atomic::Ordering::Relaxed);
        let watcher = self.clone();
        tokio::task::spawn_blocking(move || {
            let observe = if matches!(pass, Pass::Query) {
                Analytics::observe_and_read
            } else {
                Analytics::observe_in_background
            };
            observe(
                &watcher.service,
                method,
                &params,
                &rows,
                &watcher.roots(),
                chrono::Utc::now().timestamp_millis(),
            )
        })
        .await?
    }
    /// Own and join this task. Query work is allowed to finish before stop returns;
    /// no detached blocking transcript fold can outlive the backend owner.
    pub async fn run(self: Arc<Self>, hub: crate::Handle) -> Result<()> {
        let mut cancelled = self.cancelled.subscribe();
        if *cancelled.borrow() {
            return Ok(());
        }
        tokio::select! {_=cancelled.changed()=>return Ok(()),r=hub.ready()=>{r?;}}
        let Some(engine) = &self.engine else {
            let _ = cancelled.changed().await;
            return Ok(());
        };
        let mut updates = engine.subscribe()?;
        // Engine updates arrive many times a second while agents work; a pass
        // follows them at most every ACTIVE_PASS, and otherwise every
        // IDLE_PASS. Archived sessions only change by being archived, so a
        // pass reads them every ARCHIVED_PASS. A failed pass retries after
        // RETRY_PASS rather than on the next tick.
        const ACTIVE_PASS: Duration = Duration::from_secs(5);
        const IDLE_PASS: Duration = Duration::from_secs(60);
        const ARCHIVED_PASS: Duration = Duration::from_secs(300);
        const RETRY_PASS: Duration = Duration::from_secs(30);
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut dirty = true;
        let mut last: Option<tokio::time::Instant> = None;
        let mut last_archived: Option<tokio::time::Instant> = None;
        let mut wait = ACTIVE_PASS;
        loop {
            tokio::select! {
             _=cancelled.changed()=>{
                // Unconditional: an update and the close can arrive together,
                // and an unchanged row costs a read, not a write.
                if let Err(error) = self.timed("analytics.recent", json!({"limit":1}), Pass::Final).await {
                    eprintln!("Analytics closing observation was not recorded: {error:#}");
                }
                return Ok(());
             },
             update=updates.recv()=>{match update{Ok(_)|Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>dirty=true,Err(tokio::sync::broadcast::error::RecvError::Closed)=>{if *cancelled.borrow(){return Ok(());}bail!("analytics engine update stream closed");}}},
             _=ticker.tick()=>{
                let since = last.map_or(Duration::MAX, |at| at.elapsed());
                if (dirty && since >= wait) || since >= IDLE_PASS {
                    let archived = last_archived.is_none_or(|at| at.elapsed() >= ARCHIVED_PASS);
                    last = Some(tokio::time::Instant::now());
                    match self.timed("analytics.recent", json!({"limit":1}), Pass::Background { archived }).await {
                        Ok(_) => {
                            dirty = false;
                            wait = ACTIVE_PASS;
                            if archived { last_archived = last; }
                        }
                        Err(error) => {
                            dirty = true;
                            wait = RETRY_PASS;
                            eprintln!("Analytics observation retained previous history: {error:#}");
                        }
                    }
                    if *cancelled.borrow(){return Ok(());}
                }
             }
            }
        }
    }
}
pub fn install(
    mut options: crate::Options,
    service: Arc<Analytics>,
    profiles: Arc<super::super::profiles::Profiles>,
    home: PathBuf,
) -> (crate::Options, Arc<Watcher>) {
    let mut watcher = Watcher::new(service, options.engine.clone(), profiles, home);
    let inner = Arc::get_mut(&mut watcher).expect("new watcher has one owner");
    inner.lifecycle = options.launch_lifecycle.clone();
    inner.replacements = options.replacements.clone();
    for method in ["analytics.summary", "analytics.recent"] {
        let watcher = watcher.clone();
        options = options.handler(method, move |_, params| {
            let watcher = watcher.clone();
            async move { watcher.query(method, params).await }
        });
    }
    (options, watcher)
}
/// What a pass is for: an explicit analytics query folds everything that
/// changed; the watcher's background pass is throttled and reads archived
/// sessions only when `archived`.
#[derive(Clone, Copy)]
enum Pass {
    Query,
    Background {
        archived: bool,
    },
    /// One last background pass as the hub closes, so a session that started
    /// or ended since the previous pass is still recorded.
    Final,
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A session that starts moments before the hub closes, inside the
    /// background pass interval, still reaches the durable record history
    /// falls back on.
    #[tokio::test]
    async fn a_session_started_just_before_shutdown_is_recorded() -> Result<()> {
        use claudemon::daemon::{
            ServeConfig,
            embedded::{EmbeddedDaemon, Options as EngineOptions},
        };
        let _single_engine = crate::backend::ENGINE_TEST_LOCK.lock().await;
        let dir = tempfile::tempdir()?;
        let mut engine = EmbeddedDaemon::start_with_options(
            ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: 0,
                api_port: 0,
                db_path: dir.path().join("state.db"),
            },
            EngineOptions {
                usage_poll_on_boot: Some(false),
            },
        )?;
        let ready = engine.ready().await?;
        let data = dir.path().join("data");
        let mut options = crate::Options::default();
        options.engine = Some(engine.client());
        options.config_dir = Some(data.clone());
        options.home_dir = Some(dir.path().join("home"));
        let hub = crate::Hub::start(options)?;
        hub.ready().await?;
        let record = data.join("headless-analytics.sqlite");
        let recorded = |id: &str| -> bool {
            Analytics::open(
                record.clone(),
                Arc::new(super::super::super::pricing::Pricing::new(
                    dir.path().into(),
                )),
            )
            .and_then(|a| a.sessions(100))
            .is_ok_and(|rows| rows.iter().any(|r| r["sessionId"] == id))
        };
        let http = reqwest::Client::new();
        let hook = |id: &'static str| {
            http.post(format!("http://{}/hook", ready.hook_addr))
                .json(&json!({"hook_event_name":"SessionStart","session_id":id,"cwd":dir.path()}))
                .send()
        };
        // Once `first` is recorded a pass has just run; the next regular one
        // is at least five seconds away.
        hook("first").await?.error_for_status()?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while !recorded("first") {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?;
        hook("short").await?.error_for_status()?;
        tokio::task::spawn_blocking(move || hub.shutdown()).await??;
        engine.shutdown().await?;
        assert!(recorded("short"));
        Ok(())
    }
}

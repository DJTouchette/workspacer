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
    pub async fn query(self: &Arc<Self>, method: &'static str, params: Value) -> Result<Value> {
        let engine = self
            .engine
            .as_ref()
            .context("analytics session source unavailable")?;
        let raw = tokio::time::timeout(
            Duration::from_secs(15),
            engine.request(Command::Request {
                method: "GET".into(),
                path: "/sessions?include_archived=true&include_empty=true".into(),
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
        let watcher = self.clone();
        tokio::task::spawn_blocking(move || {
            watcher.service.observe_and_read(
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
        let mut ticker = tokio::time::interval(Duration::from_secs(2));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut dirty = true;
        let mut since = 0u32;
        loop {
            tokio::select! {
             _=cancelled.changed()=>return Ok(()),
             update=updates.recv()=>{match update{Ok(_)|Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>dirty=true,Err(tokio::sync::broadcast::error::RecvError::Closed)=>{if *cancelled.borrow(){return Ok(());}bail!("analytics engine update stream closed");}}},
             _=ticker.tick()=>{since+=1;if dirty||since>=15{match self.query("analytics.recent",json!({"limit":1})).await{Ok(_)=>{dirty=false;since=0;},Err(error)=>{dirty=true;eprintln!("Analytics observation retained previous history: {error:#}");}}if *cancelled.borrow(){return Ok(());}}}
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

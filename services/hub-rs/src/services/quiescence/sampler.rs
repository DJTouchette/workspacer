use super::{power::PowerConfig, source::EvidenceSource, *};
use crate::{Caller, Handle, Options};
use std::{sync::Arc, time::Duration};
struct Activity {
    last_ask: Option<i64>,
    asked: BTreeMap<u64, u64>,
}
pub struct Watcher {
    fleet: Monitor,
    machine: Arc<super::super::machine_power::Controller>,
    source: Arc<dyn EvidenceSource>,
    activity: Arc<Mutex<Activity>>,
    cancelled: tokio::sync::watch::Sender<bool>,
}
impl Watcher {
    pub fn new(source: Arc<dyn EvidenceSource>, tun: Tunables, config: PowerConfig) -> Arc<Self> {
        Self::configured(
            source,
            tun,
            config,
            None,
            Arc::new(|| Box::pin(async { Ok(()) })),
        )
    }
    fn configured(
        source: Arc<dyn EvidenceSource>,
        tun: Tunables,
        config: PowerConfig,
        provider: Option<Arc<dyn super::super::machine_power::PowerProvider>>,
        disconnect: super::super::machine_power::Disconnect,
    ) -> Arc<Self> {
        let (cancelled, _) = tokio::sync::watch::channel(false);
        let activity = Arc::new(Mutex::new(Activity {
            last_ask: None,
            asked: BTreeMap::new(),
        }));
        let machine_source = Arc::new(IdleSource {
            inner: source.clone(),
            activity: activity.clone(),
        });
        let machine = super::super::machine_power::Controller::new(
            config,
            provider,
            machine_source,
            disconnect,
            Default::default(),
        );
        Arc::new(Self {
            fleet: Monitor::new(tun),
            machine,
            source,
            activity,
            cancelled,
        })
    }
    pub fn answer(&self, caller: &Caller, params: &Value, now_ms: i64) -> Result<Value> {
        if !params.is_null() && params != &json!({}) {
            bail!("fleet.quiescence accepts no parameters");
        }
        let mut activity = self.activity.lock().unwrap();
        activity.last_ask = Some(now_ms);
        if caller.connection_id != 0 {
            activity
                .asked
                .insert(caller.connection_id, caller.activity_seq);
        }
        drop(activity);
        Ok(serde_json::to_value(self.fleet.latest(now_ms))?)
    }
    pub fn power_info(&self, caller: &Caller, now_ms: i64) -> Result<Value> {
        Ok(self.machine.info(caller, now_ms))
    }
    fn sampling(&self, now_ms: i64) -> bool {
        self.activity
            .lock()
            .unwrap()
            .last_ask
            .is_some_and(|at| now_ms >= at && now_ms - at <= 15 * 60_000)
    }
    pub fn close(&self) {
        self.cancelled.send_replace(true);
        self.machine.close();
    }
    pub async fn run(self: Arc<Self>, hub: Handle, interval: Duration) -> Result<()> {
        let mut cancelled = self.cancelled.subscribe();
        if *cancelled.borrow() {
            return Ok(());
        }
        tokio::select! {r=hub.ready()=>{r?;},_=cancelled.changed()=>return Ok(())}
        let mut fleet = Box::pin(self.clone().run_fleet(hub, interval));
        let mut machine = Box::pin(self.machine.clone().run());
        tokio::select! {
            result=&mut fleet=>{self.machine.close();let other=machine.await;result?;other},
            result=&mut machine=>{self.close();let other=fleet.await;result?;other}
        }
    }
    async fn run_fleet(self: Arc<Self>, hub: Handle, interval: Duration) -> Result<()> {
        let mut cancelled = self.cancelled.subscribe();
        if *cancelled.borrow() {
            return Ok(());
        }
        tokio::select! {ready=hub.ready()=>{ready?;},_=cancelled.changed()=>return Ok(())};
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {_=cancelled.changed()=>return Ok(()),_=ticker.tick()=>{
                let now=chrono::Utc::now().timestamp_millis();let requested=self.sampling(now);if !requested{continue;}
                let read=tokio::select!{_=cancelled.changed()=>return Ok(()),read=tokio::time::timeout(Duration::from_secs(12),self.source.read())=>read};
                let evidence=match read{Ok(Ok(evidence))=>evidence,Ok(Err(error))=>{let mut unknown=Evidence::unknown(now);unknown.sessions=Err(error.to_string());unknown},Err(_)=>{let mut unknown=Evidence::unknown(now);unknown.sessions=Err("Fleet sampling deadline exceeded".into());unknown}};
                let asked={let mut activity=self.activity.lock().unwrap();if let Ok(clients)=&evidence.clients{activity.asked.retain(|id,_|clients.iter().any(|c|c.connection_id==*id));}activity.asked.clone()};
                if requested{self.fleet.observe(&evidence,&asked);}
            }}
        }
    }
}
pub fn install(
    mut options: Options,
    hub: Handle,
    routes: crate::federation::Routes,
) -> (Options, Arc<Watcher>, tokio::task::JoinHandle<Result<()>>) {
    let config = PowerConfig::from_environment();
    let tun = Tunables {
        keep_jobs_awake: config.requested_stop,
        ..Tunables::default()
    };
    let source = Arc::new(super::source::NativeSources {
        engine: options.engine.clone(),
        hub: hub.clone(),
        jobs: options.jobs_service.clone(),
        federation: routes,
        coordinator: options.spawn_coordinator.clone(),
        lifecycle: options.launch_lifecycle.clone(),
        workflow: options.workflow_runtime.clone(),
        replacements: options.replacements.clone(),
        terminals: options.terminals.clone(),
    });
    let disconnect_hub = hub.clone();
    let disconnect: super::super::machine_power::Disconnect = Arc::new(move || {
        let hub = disconnect_hub.clone();
        Box::pin(async move { hub.disconnect_for_machine_stop().await })
    });
    let watcher = Watcher::configured(
        source,
        tun,
        config,
        options.machine_power_provider.clone(),
        disconnect,
    );
    for method in ["fleet.quiescence", "machine.power", "machine.stop"] {
        if method != "machine.stop" && options.has_handler(method) {
            continue;
        }
        let watcher = watcher.clone();
        options = options.handler(method, move |caller, params| {
            let watcher = watcher.clone();
            async move {
                let now = chrono::Utc::now().timestamp_millis();
                if method == "fleet.quiescence" {
                    watcher.answer(&caller, &params, now)
                } else if method == "machine.stop" {
                    watcher.machine.manual_stop(&caller).await
                } else {
                    watcher.power_info(&caller, now)
                }
            }
        });
    }
    let running = watcher.clone();
    let task = tokio::spawn(async move { running.run(hub, Duration::from_secs(30)).await });
    (options, watcher, task)
}

struct IdleSource {
    inner: Arc<dyn EvidenceSource>,
    activity: Arc<Mutex<Activity>>,
}
impl EvidenceSource for IdleSource {
    fn read(&self) -> futures_util::future::BoxFuture<'_, Result<Evidence>> {
        Box::pin(async move {
            let mut evidence = self.inner.read().await?;
            if let Ok(clients) = &mut evidence.clients {
                let activity = self.activity.lock().unwrap();
                clients.retain(|client| {
                    activity.asked.get(&client.connection_id) != Some(&client.activity_seq)
                });
            }
            Ok(evidence)
        })
    }
}

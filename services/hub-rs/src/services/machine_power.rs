//! Power of this host only. The embedding host must explicitly supply authority;
//! neither OS detection nor inherited cloud variables enable library actions.
mod fly;
use super::quiescence::{
    Evidence, Monitor, Reading, Tunables, power::PowerConfig, source::EvidenceSource,
};
use anyhow::{Result, bail};
pub use fly::standalone_from_environment;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
pub trait PowerProvider: Send + Sync + 'static {
    fn check(&self) -> BoxFuture<'_, Result<()>>;
    fn stop(&self) -> BoxFuture<'_, Result<()>>;
}
pub type Disconnect = Arc<dyn Fn() -> BoxFuture<'static, Result<()>> + Send + Sync>;
#[derive(Clone, Copy)]
pub struct Timing {
    pub sample: Duration,
    pub receipt_delay: Duration,
    pub check_timeout: Duration,
    pub stop_timeout: Duration,
    pub read_timeout: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            sample: Duration::from_secs(30),
            receipt_delay: Duration::from_secs(1),
            check_timeout: Duration::from_secs(8),
            stop_timeout: Duration::from_secs(60),
            read_timeout: Duration::from_secs(12),
        }
    }
}
struct State {
    stopping: bool,
    manual: bool,
    error: String,
}
struct Manual {
    reply: oneshot::Sender<Result<Value>>,
}
pub struct Controller {
    config: PowerConfig,
    provider: Option<Arc<dyn PowerProvider>>,
    source: Arc<dyn EvidenceSource>,
    disconnect: Disconnect,
    monitor: Monitor,
    state: Mutex<State>,
    tx: mpsc::Sender<Manual>,
    rx: Mutex<Option<mpsc::Receiver<Manual>>>,
    cancelled: watch::Sender<bool>,
    timing: Timing,
}
pub fn trusted(caller: &crate::Caller) -> bool {
    caller.trusted && caller.scope == "operator"
}
/// Every future schedule blocks automatic stop. The host has no scheduled wake.
pub fn idle_inputs(mut evidence: Evidence) -> Evidence {
    if let Ok(jobs) = &mut evidence.jobs {
        for job in jobs {
            if job.running || job.next_run_ms.is_some() {
                job.action_kind = "call".into();
                job.running = true;
            }
        }
    }
    evidence
}
impl Controller {
    pub fn new(
        config: PowerConfig,
        provider: Option<Arc<dyn PowerProvider>>,
        source: Arc<dyn EvidenceSource>,
        disconnect: Disconnect,
        timing: Timing,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (cancelled, _) = watch::channel(false);
        Arc::new(Self {
            monitor: Monitor::new(Tunables {
                dwell_ms: config.idle_timeout_ms,
                client_idle_ms: timing.sample.as_millis().min(i64::MAX as u128) as i64,
                ..Default::default()
            }),
            config,
            provider,
            source,
            disconnect,
            state: Mutex::new(State {
                stopping: false,
                manual: false,
                error: String::new(),
            }),
            tx,
            rx: Mutex::new(Some(rx)),
            cancelled,
            timing,
        })
    }
    pub fn close(&self) {
        self.cancelled.send_replace(true);
    }
    pub fn info(&self, caller: &crate::Caller, now_ms: i64) -> Value {
        let state = self.state.lock().unwrap();
        let idle = if self.config.idle_timeout_ms > 0 {
            Some(self.monitor.latest(now_ms))
        } else {
            None
        };
        let mut info = self.config.info(idle);
        info["canStop"] = (self.provider.is_some() && trusted(caller)).into();
        info["stopping"] = state.stopping.into();
        info["error"] = state.error.clone().into();
        info["idleMode"] = if self.config.idle_timeout_ms <= 0 {
            "off"
        } else if self.config.requested_stop && self.provider.is_some() {
            "stop"
        } else {
            "observe"
        }
        .into();
        info
    }
    /// Parameters are deliberately ignored: no caller can choose a host, signal,
    /// credential, executable or endpoint. This matches the existing bus contract.
    pub async fn manual_stop(&self, caller: &crate::Caller) -> Result<Value> {
        if !trusted(caller) {
            bail!("machine.stop requires operator authority");
        }
        if self.provider.is_none() {
            bail!("machine power is not configured on this server");
        }
        if *self.cancelled.borrow() {
            bail!("machine power owner is stopping");
        }
        let (reply, answer) = oneshot::channel();
        self.tx
            .try_send(Manual { reply })
            .map_err(|_| anyhow::anyhow!("machine power request queue unavailable"))?;
        answer
            .await
            .map_err(|_| anyhow::anyhow!("machine power owner stopped before admission"))?
    }
    async fn sample(&self) -> Reading {
        let now = chrono::Utc::now().timestamp_millis();
        let evidence =
            match tokio::time::timeout(self.timing.read_timeout, self.source.read()).await {
                Ok(Ok(e)) => e,
                Ok(Err(_)) | Err(_) => Evidence::unknown(now),
            };
        self.monitor
            .observe(&idle_inputs(evidence), &BTreeMap::new())
    }
    async fn check(&self) -> bool {
        match &self.provider {
            Some(provider) => matches!(
                tokio::time::timeout(self.timing.check_timeout, provider.check()).await,
                Ok(Ok(()))
            ),
            None => false,
        }
    }
    fn can_auto_arm(&self) -> bool {
        let state = self.state.lock().unwrap();
        self.config.requested_stop
            && self.provider.is_some()
            && !state.stopping
            && state.error.is_empty()
    }
    fn arm(&self, manual: bool) {
        let mut state = self.state.lock().unwrap();
        state.stopping = true;
        state.manual = manual;
        state.error.clear();
    }
    fn cancel_arm(&self) {
        let mut state = self.state.lock().unwrap();
        state.stopping = false;
        state.manual = false;
    }
    fn fail_stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.stopping = false;
        state.error = "The stop request failed. The machine may still be running.".into();
    }
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let mut rx = self
            .rx
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| anyhow::anyhow!("machine power owner already running"))?;
        let mut cancelled = self.cancelled.subscribe();
        if *cancelled.borrow() {
            return Ok(());
        }
        let mut ticker = tokio::time::interval(self.timing.sample);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        let mut due: Option<tokio::time::Instant> = None;
        loop {
            let wait = async {
                if let Some(at) = due {
                    tokio::time::sleep_until(at).await
                } else {
                    std::future::pending::<()>().await
                }
            };
            tokio::select! {
             _=cancelled.changed()=>{self.cancel_arm();return Ok(());},
             command=rx.recv()=>{let Some(command)=command else{return Ok(());};
              if self.state.lock().unwrap().stopping {self.state.lock().unwrap().manual=true;let _=command.reply.send(Ok(json!({"accepted":true})));continue;}
              if !self.check().await{let _=command.reply.send(Err(anyhow::anyhow!("machine power provider unavailable; check the server's power configuration")));continue;}
              if *cancelled.borrow(){let _=command.reply.send(Err(anyhow::anyhow!("machine power owner is stopping")));return Ok(());}
              self.arm(true);due=Some(tokio::time::Instant::now()+self.timing.receipt_delay);let _=command.reply.send(Ok(json!({"accepted":true})));
             },
             _=ticker.tick(),if self.config.idle_timeout_ms>0=>{
              let reading=self.sample().await;
              if reading.quiescent&&self.can_auto_arm()&&self.check().await&&!*cancelled.borrow(){self.arm(false);due=Some(tokio::time::Instant::now()+self.timing.receipt_delay);}
             },
             _=wait=>{
              due=None;let manual=self.state.lock().unwrap().manual;
              if *cancelled.borrow(){self.cancel_arm();return Ok(());}
              if !manual&&!self.sample().await.quiescent{self.cancel_arm();continue;}
              if *cancelled.borrow(){self.cancel_arm();return Ok(());}
              if !matches!(tokio::time::timeout(self.timing.check_timeout,(self.disconnect)()).await,Ok(Ok(()))){self.fail_stop();continue;}
              // Provider calls are bounded and awaited. Successful acceptance remains
              // latched until host shutdown; transport loss never causes automatic replay.
              let outcome=tokio::time::timeout(self.timing.stop_timeout,self.provider.as_ref().expect("armed provider").stop()).await;
              if !matches!(outcome,Ok(Ok(()))){self.fail_stop();}
             }
            }
        }
    }
}

//! A separate outbound caller credential for the node's MCP facade. It never
//! borrows provider or owner authority, and reconnect never retries an RPC.
use crate::client::{Client, DisconnectReason};
use anyhow::{Result, anyhow};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;
pub struct UpstreamCaller {
    url: String,
    token: String,
    current: Mutex<Option<Client>>,
    layout: Arc<std::sync::RwLock<serde_json::Value>>,
    state: watch::Sender<State>,
    stop: watch::Sender<bool>,
    resume: watch::Sender<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Connecting,
    Connected,
    Disconnected,
    Paused,
    Rejected,
    Stopped,
}
impl UpstreamCaller {
    #[cfg(test)]
    pub(crate) fn new(url: String, token: String) -> Arc<Self> {
        Self::with_layout(
            url,
            token,
            Arc::new(std::sync::RwLock::new(serde_json::Value::Null)),
        )
    }
    pub(crate) fn with_layout(
        url: String,
        token: String,
        layout: Arc<std::sync::RwLock<serde_json::Value>>,
    ) -> Arc<Self> {
        let (state, _) = watch::channel(State::Connecting);
        let (stop, _) = watch::channel(false);
        let (resume, _) = watch::channel(0);
        Arc::new(Self {
            url,
            token,
            current: Mutex::new(None),
            layout,
            state,
            stop,
            resume,
        })
    }
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
    pub fn state(&self) -> watch::Receiver<State> {
        self.state.subscribe()
    }
    pub fn connected(&self) -> bool {
        *self.state.borrow() == State::Connected
    }
    pub fn paused(&self) -> bool {
        *self.state.borrow() == State::Paused
    }
    pub async fn client(&self) -> Result<Client> {
        let mut state = self.state.subscribe();
        tokio::time::timeout(Duration::from_secs(5),async{loop{
  if let Some(client)=self.current.lock().unwrap().clone(){return Ok(client)}
  match *state.borrow(){State::Paused=>return Err(anyhow!("upstream machine is paused; explicit resume is required")),State::Rejected=>return Err(anyhow!("upstream caller requires a scoped operator facade credential, not a provider or owner credential")),State::Stopped=>return Err(anyhow!("upstream caller stopped")),_=>{}}
  state.changed().await.map_err(|_|anyhow!("upstream caller stopped"))?;
 }}).await.map_err(|_|anyhow!("upstream caller unavailable; call was not submitted"))?
    }
    pub(crate) fn close(&self) {
        self.stop.send_replace(true);
        if let Some(client) = self.current.lock().unwrap().take() {
            client.close();
        }
    }
    pub(crate) fn resume(&self) {
        if self.connected() {
            return;
        }
        self.state.send_replace(State::Disconnected);
        self.resume
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
    }
    async fn refresh_layout(&self, client: &Client) {
        if let Ok(value) = client
            .call_with_timeout("layout.get", serde_json::json!({}), Duration::from_secs(2))
            .await
        {
            if value.is_object() {
                *self.layout.write().unwrap() = value;
            }
        }
    }
    pub(crate) async fn run(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.subscribe();
        let mut resume = self.resume.subscribe();
        let mut delay = Duration::from_secs(1);
        loop {
            if *stop.borrow() {
                break;
            }
            if self.paused() {
                tokio::select! {_=stop.changed()=>break,_=resume.changed()=>{}}
            }
            if *stop.borrow() {
                break;
            }
            self.state.send_replace(State::Connecting);
            let connected = tokio::select! {_=stop.changed()=>break,result=Client::connect_remote_with_identity(&self.url,&self.token)=>result};
            let mut paused = false;
            match connected {
                Ok((client, Some(proof)))
                    if proof.scope == "operator"
                        && proof.may_assert_session
                        && !proof.authenticated_host
                        && !proof.federated
                        && proof.plugin_id.is_empty()
                        && proof.token_id == crate::auth::fingerprint(&self.token) =>
                {
                    self.refresh_layout(&client).await;
                    *self.current.lock().unwrap() = Some(client.clone());
                    self.state.send_replace(State::Connected);
                    let started = tokio::time::Instant::now();
                    let mut layout_tick = tokio::time::interval_at(
                        tokio::time::Instant::now() + Duration::from_secs(5),
                        Duration::from_secs(5),
                    );
                    layout_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    let reason = loop {
                        tokio::select! {biased;_=stop.changed()=>break DisconnectReason::LocalClosed,reason=client.disconnected_reason()=>break reason,_=layout_tick.tick()=>self.refresh_layout(&client).await}
                    };
                    self.current.lock().unwrap().take();
                    client.close();
                    paused = reason.is_power_paused();
                    self.state.send_replace(if paused {
                        State::Paused
                    } else {
                        State::Disconnected
                    });
                    if started.elapsed() >= Duration::from_secs(30) {
                        delay = Duration::from_secs(1)
                    }
                }
                Ok((client, _)) => {
                    client.close();
                    self.state.send_replace(State::Rejected);
                }
                Err(error) => {
                    paused = error
                        .downcast_ref::<DisconnectReason>()
                        .is_some_and(DisconnectReason::is_power_paused);
                    self.state.send_replace(if paused {
                        State::Paused
                    } else {
                        State::Disconnected
                    });
                }
            }
            if *stop.borrow() {
                break;
            }
            if paused {
                continue;
            }
            tokio::select! {_=stop.changed()=>break,_=resume.changed()=>{},_=tokio::time::sleep(delay)=>{}}
            delay = (delay * 2).min(Duration::from_secs(30));
        }
        if let Some(client) = self.current.lock().unwrap().take() {
            client.close();
        }
        self.state.send_replace(State::Stopped);
        Ok(())
    }
}

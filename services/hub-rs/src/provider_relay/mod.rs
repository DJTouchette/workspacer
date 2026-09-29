//! Owned outbound capability provider for a remote worker. It replaces the Go
//! brain socket, not the engine: incoming calls retain broker-verified identity
//! and dispatch into the same local Rust services used by native/standalone.
mod caller;
mod last_exit;
mod methods;
use crate::{
    Handle, Options,
    client::{Client, DisconnectReason},
    protocol::{Event, Frame, ProviderCaller},
};
use anyhow::{Result, anyhow, bail};
pub use caller::UpstreamCaller;
use futures_util::{SinkExt, StreamExt};
pub(crate) use last_exit::path as last_exit_path;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, time::Instant};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Full,
    Catalog,
}
#[derive(Clone)]
pub struct Config {
    pub url: String,
    pub token: String,
    pub scope: Scope,
    pub node_id: String,
    pub last_exit_file: Option<PathBuf>,
    pub caller_token: Option<String>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        let url =
            url::Url::parse(&self.url).map_err(|_| anyhow!("invalid upstream provider URL"))?;
        anyhow::ensure!(
            matches!(url.scheme(), "ws" | "wss")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "upstream provider URL must be WS(S) without URL credentials, query or fragment"
        );
        anyhow::ensure!(
            !self.token.is_empty(),
            "upstream provider credential is required"
        );
        anyhow::ensure!(
            self.scope != Scope::Full
                || self
                    .caller_token
                    .as_ref()
                    .is_some_and(|token| !token.is_empty() && token != &self.token),
            "full worker relay requires a separate upstream caller credential"
        );
        anyhow::ensure!(
            self.node_id.is_empty() || crate::services::nodes::valid_id(&self.node_id),
            "invalid provider node identity"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    connected: bool,
    paused: bool,
    scope: Scope,
    node: String,
    registered_methods: Vec<String>,
    detail: String,
}
pub(crate) struct Relay {
    config: Config,
    hub: Handle,
    stop: watch::Sender<bool>,
    resume: watch::Sender<u64>,
    status: Mutex<Status>,
    last_exit: Option<crate::services::nodes::ExitRecord>,
    caller: Option<Arc<UpstreamCaller>>,
}
struct LocalCaller {
    client: Client,
    active: AtomicUsize,
    last: Mutex<Instant>,
}
impl Drop for LocalCaller {
    fn drop(&mut self) {
        self.client.close();
    }
}
struct Active(Arc<LocalCaller>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        *self.0.last.lock().unwrap() = Instant::now();
    }
}
struct Reply {
    generation: u64,
    attempt: u64,
    id: String,
    result: Result<Value>,
}
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
impl Relay {
    fn new(
        config: Config,
        hub: Handle,
        layout: Option<Arc<std::sync::RwLock<Value>>>,
    ) -> Result<Arc<Self>> {
        config.validate()?;
        let last_exit = config
            .last_exit_file
            .as_ref()
            .and_then(|path| last_exit::read(path));
        let caller = config
            .caller_token
            .as_ref()
            .filter(|token| !token.is_empty())
            .map(|token| {
                UpstreamCaller::with_layout(
                    config.url.clone(),
                    token.clone(),
                    layout.unwrap_or_else(|| Arc::new(std::sync::RwLock::new(Value::Null))),
                )
            });
        let status = Status {
            connected: false,
            paused: false,
            scope: config.scope,
            node: config.node_id.clone(),
            registered_methods: vec![],
            detail: "waiting for local backend".into(),
        };
        let (stop, _) = watch::channel(false);
        let (resume, _) = watch::channel(0);
        Ok(Arc::new(Self {
            config,
            hub,
            stop,
            resume,
            status: Mutex::new(status),
            last_exit,
            caller,
        }))
    }
    pub fn close(&self) {
        self.stop.send_replace(true);
        if let Some(caller) = &self.caller {
            caller.close();
        }
    }
    fn resume(&self) {
        if let Some(caller) = &self.caller {
            caller.resume();
        }
        let mut status = self.status.lock().unwrap();
        status.paused = false;
        status.detail = "explicit resume requested".into();
        drop(status);
        self.resume
            .send_modify(|version| *version = version.wrapping_add(1));
    }
    fn status(&self) -> Value {
        serde_json::to_value(self.status.lock().unwrap().clone()).unwrap()
    }
    async fn dial(&self) -> Result<Socket> {
        let mut request = self
            .config
            .url
            .as_str()
            .into_client_request()
            .map_err(|_| anyhow!("invalid upstream provider URL"))?;
        let mut token = format!("Bearer {}", self.config.token)
            .parse::<tokio_tungstenite::tungstenite::http::HeaderValue>()
            .map_err(|_| anyhow!("invalid upstream provider credential"))?;
        token.set_sensitive(true);
        request.headers_mut().insert("authorization", token);
        let config = WebSocketConfig {
            max_message_size: Some(64 << 20),
            max_frame_size: Some(64 << 20),
            ..WebSocketConfig::default()
        };
        tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async_with_config(request, Some(config), false),
        )
        .await
        .map_err(|_| anyhow!("upstream provider connection timed out"))?
        .map(|(socket, _)| socket)
        .map_err(|_| anyhow!("upstream provider connection failed"))
    }
    async fn caller(
        &self,
        proof: ProviderCaller,
        cache: &mut HashMap<ProviderCaller, Arc<LocalCaller>>,
    ) -> Result<Arc<LocalCaller>> {
        if let Some(caller) = cache.get(&proof) {
            return Ok(caller.clone());
        }
        cache.retain(|_, caller| {
            caller.active.load(Ordering::Acquire) > 0
                || caller.last.lock().unwrap().elapsed() < Duration::from_secs(60)
        });
        if cache.len() >= 128 {
            let oldest = cache
                .iter()
                .filter(|(_, caller)| caller.active.load(Ordering::Acquire) == 0)
                .min_by_key(|(_, caller)| *caller.last.lock().unwrap())
                .map(|(proof, _)| proof.clone());
            if let Some(oldest) = oldest {
                cache.remove(&oldest);
            }
        }
        anyhow::ensure!(cache.len() < 128, "provider caller capacity reached");
        let connection = self.hub.connect_provider_caller(proof.clone()).await?;
        let caller = Arc::new(LocalCaller {
            client: Client::from_connection(connection),
            active: AtomicUsize::new(0),
            last: Mutex::new(Instant::now()),
        });
        cache.insert(proof, caller.clone());
        Ok(caller)
    }
    async fn session(
        &self,
        socket: &mut Socket,
        generation: u64,
        offered: &BTreeMap<String, String>,
        events: &Client,
        stream: &mut tokio::sync::broadcast::Receiver<Event>,
        tasks: &mut tokio::task::JoinSet<Reply>,
    ) -> Result<()> {
        let hello = tokio::time::timeout(Duration::from_secs(10), receive(socket))
            .await
            .map_err(|_| anyhow!("upstream provider hello timed out"))??;
        anyhow::ensure!(hello.op == "hello", "upstream did not send hello");
        send(
            socket,
            &Frame {
                methods: offered.keys().cloned().collect(),
                wants_caller_context: true,
                ..Frame::op("register")
            },
        )
        .await?;
        let registered = tokio::time::timeout(Duration::from_secs(10), receive(socket))
            .await
            .map_err(|_| anyhow!("provider registration timed out"))??;
        anyhow::ensure!(
            registered.op == "registered" && registered.caller_context_version == 1,
            "upstream must support authenticated provider caller context"
        );
        let accepted: BTreeSet<_> = registered
            .methods
            .into_iter()
            .filter(|method| offered.contains_key(method))
            .collect();
        {
            let mut status = self.status.lock().unwrap();
            status.connected = true;
            status.paused = false;
            status.detail = if accepted.contains("brain.info") {
                String::new()
            } else {
                "upstream liveness slot belongs to another provider".into()
            };
            status.registered_methods = accepted.iter().cloned().collect();
        }
        let mut demand = BTreeSet::new();
        let base: BTreeSet<String> = methods::TOPICS
            .iter()
            .map(|topic| (*topic).into())
            .collect();
        events.topics(base.clone()).await?;
        if accepted.contains("sessions.conversation") {
            send(
                socket,
                &Frame {
                    topics: vec!["agent.conversation.".into()],
                    ..Frame::op("demand")
                },
            )
            .await?;
        }
        // Initial/reconnect fleet state is a read, not replay of any accepted mutation.
        if accepted.contains("sessions.snapshots") {
            if let Ok(rows) = events.call("sessions.snapshots", Value::Null).await {
                if let Some(rows) = rows.as_array() {
                    for row in rows {
                        let event = Event::new("agent.snapshot", "brain", row.clone());
                        send(
                            socket,
                            &Frame {
                                event: Some(event),
                                ..Frame::op("publish")
                            },
                        )
                        .await?;
                    }
                }
            }
        }
        let mut callers = HashMap::new();
        let mut active_ids = HashSet::new();
        let mut cancellations: HashMap<String, (u64, tokio::task::AbortHandle)> = HashMap::new();
        let mut attempt = 0u64;
        let mut closed_callers = BTreeSet::new();
        let mut stop = self.stop.subscribe();
        let mut caller_state = self.caller.as_ref().map(|caller| caller.state());
        let mut sweep = tokio::time::interval(Duration::from_secs(30));
        let result=async{loop{if *stop.borrow(){return Ok(())}if let Some(caller)=&self.caller{if caller.paused(){return Err(DisconnectReason::RemoteClose{code:Some(crate::client::MACHINE_STOP_CLOSE_CODE),reason:"upstream caller paused".into()}.into())}if !caller.connected(){bail!("upstream caller unavailable")}}tokio::select!{biased;
   _=stop.changed()=>return Ok(()),
   _=async{caller_state.as_mut().unwrap().changed().await},if caller_state.is_some()=>{if self.caller.as_ref().is_some_and(|caller|caller.paused()){return Err(DisconnectReason::RemoteClose{code:Some(crate::client::MACHINE_STOP_CLOSE_CODE),reason:"upstream caller paused".into()}.into())}if self.caller.as_ref().is_some_and(|caller|!caller.connected()){bail!("upstream caller unavailable")}},
   Some(reply)=tasks.join_next()=>{if let Ok(reply)=reply{if reply.generation==generation && cancellations.get(&reply.id).is_some_and(|(id,_)|*id==reply.attempt){cancellations.remove(&reply.id);active_ids.remove(&reply.id);let frame=match reply.result{Ok(value)=>Frame{id:reply.id,result:Some(value),..Frame::op("result")},Err(error)=>Frame::error(reply.id,error.to_string())};send(socket,&frame).await?;}}},
   incoming=receive(socket)=>{
    let frame=incoming?;match frame.op.as_str(){
     "call"=>{if frame.id.is_empty()||frame.id.len()>200{bail!("invalid upstream call identity")};if !accepted.contains(&frame.method){send(socket,&Frame::error(frame.id,"method was not registered by this provider")).await?;continue}
      if frame.method=="brain.info"{let mut info=json!({"provider":"brain","runtime":"rust","scope":self.config.scope});if !self.config.node_id.is_empty(){info["node"]=self.config.node_id.clone().into();}if let Some(exit)=&self.last_exit{info["lastExit"]=serde_json::to_value(exit)?;}send(socket,&Frame{id:frame.id,result:Some(info),..Frame::op("result")}).await?;continue}
      if tasks.len()>=64||active_ids.contains(&frame.id){send(socket,&Frame::error(frame.id,"provider is busy or call identity is already active")).await?;continue}
      let Some(proof)=frame.provider_caller else{send(socket,&Frame::error(frame.id,"broker-verified provider caller context is required")).await?;continue};if closed_callers.contains(&proof.connection_id){send(socket,&Frame::error(frame.id,"original caller disconnected; request was not admitted")).await?;continue}
      let caller=match self.caller(proof,&mut callers).await{Ok(caller)=>caller,Err(error)=>{send(socket,&Frame::error(frame.id,error.to_string())).await?;continue}};caller.active.fetch_add(1,Ordering::AcqRel);let active=Active(caller);let method=offered[&frame.method].clone();let timeout=crate::protocol::provider_timeout(&method,Duration::from_secs(30));let id=frame.id;active_ids.insert(id.clone());let params=frame.params.unwrap_or(Value::Null);
      attempt=attempt.checked_add(1).ok_or_else(||anyhow!("provider call generation exhausted"))?;let current=attempt;let key=id.clone();let task=tasks.spawn(async move{let result=active.0.client.call_with_timeout(&method,params,timeout).await;drop(active);Reply{generation,attempt:current,id,result}});cancellations.insert(key,(current,task));
     },
     "cancel"=>{if frame.caller_context_version!=1{bail!("invalid negotiated cancellation")};if let Some((_,task))=cancellations.remove(&frame.id){active_ids.remove(&frame.id);task.abort();}},
     "callerClosed"=>{if frame.caller_context_version!=1{bail!("invalid caller lifecycle frame")};let id=frame.id.parse::<u64>().map_err(|_|anyhow!("invalid caller lifecycle identity"))?;closed_callers.insert(id);callers.retain(|proof,caller|{if proof.connection_id==id{caller.client.close();false}else{true}});if closed_callers.len()>4096{closed_callers.pop_first();}},
     "demand"=>{if frame.topic.starts_with("agent.conversation.")&&!frame.topic.contains('*')&&frame.topic.len()<=256&&accepted.contains("sessions.conversation"){if frame.demand{if demand.len()<256-base.len(){demand.insert(frame.topic);}}else{demand.remove(&frame.topic);}let mut topics=base.clone();topics.extend(demand.iter().cloned());events.topics(topics).await?;}},
     "error"=>{},
     _=>{}
    }
   },
   event=stream.recv()=>match event{Ok(event)=>{if event.hub.is_empty()&&methods::event_allowed(&event.topic,&accepted,&demand){send(socket,&Frame{event:Some(event),..Frame::op("publish")}).await?;}},Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{if accepted.contains("sessions.snapshots"){if let Ok(rows)=events.call("sessions.snapshots",Value::Null).await{if let Some(rows)=rows.as_array(){for row in rows{send(socket,&Frame{event:Some(Event::new("agent.snapshot","brain",row.clone())),..Frame::op("publish")}).await?;}}}}let mut topics=base.clone();topics.extend(demand.iter().cloned());events.topics(BTreeSet::new()).await?;events.topics(topics).await?;},Err(_)=>bail!("local provider event stream closed")},
   _=sweep.tick()=>callers.retain(|_,caller|caller.active.load(Ordering::Acquire)>0||caller.last.lock().unwrap().elapsed()<Duration::from_secs(60)),
  }}}.await;
        // Close cached identities even while active calls hold Arc clones. Local
        // engine effects remain owned; only their abandoned reply waiters cancel.
        for caller in callers.values() {
            caller.client.close();
        }
        for (_, task) in cancellations.into_values() {
            task.abort();
        }
        callers.clear();
        let _ = events.topics(BTreeSet::new()).await;
        result
    }
    async fn run(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.subscribe();
        let mut resume = self.resume.subscribe();
        if *stop.borrow() {
            return Ok(());
        }
        tokio::select! {_=stop.changed()=>return Ok(()),ready=self.hub.ready()=>{ready?;}}
        loop {
            let health = self.hub.health().await?;
            if self.config.scope == Scope::Catalog
                || (health["launchReady"] == true
                    && self
                        .caller
                        .as_ref()
                        .is_some_and(|caller| caller.connected()))
            {
                break;
            }
            tokio::select! {_=stop.changed()=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
        }
        let health = self.hub.health().await?;
        let available = health["methodNames"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let offered = methods::offered(self.config.scope, &available);
        let events = Client::connect_service(&self.hub).await?;
        let mut stream = events.events();
        let mut tasks = tokio::task::JoinSet::new();
        let mut generation = 0u64;
        let mut delay = Duration::from_secs(1);
        loop {
            if *stop.borrow() {
                break;
            }
            if self.status.lock().unwrap().paused {
                tokio::select! {_=stop.changed()=>break,_=resume.changed()=>{},Some(_)=tasks.join_next()=>{continue}}
            }
            if *stop.borrow() {
                break;
            }
            if self.config.scope == Scope::Full
                && self
                    .caller
                    .as_ref()
                    .is_some_and(|caller| !caller.connected())
            {
                tokio::select! {_=stop.changed()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
                continue;
            }
            generation = generation.wrapping_add(1);
            let started = Instant::now();
            let result = tokio::select! {_=stop.changed()=>break,result=self.dial()=>result};
            let result = match result {
                Ok(mut socket) => {
                    let result = self
                        .session(
                            &mut socket,
                            generation,
                            &offered,
                            &events,
                            &mut stream,
                            &mut tasks,
                        )
                        .await;
                    let _ =
                        tokio::time::timeout(Duration::from_millis(100), socket.close(None)).await;
                    result
                }
                Err(error) => Err(error),
            };
            let paused = result
                .as_ref()
                .err()
                .and_then(|error| error.downcast_ref::<DisconnectReason>())
                .is_some_and(DisconnectReason::is_power_paused);
            {
                let mut status = self.status.lock().unwrap();
                status.connected = false;
                status.paused = paused;
                status.registered_methods.clear();
                status.detail = if paused {
                    "upstream is stopping; explicit resume is required"
                } else {
                    "upstream disconnected; calls are never replayed"
                }
                .into();
            }
            if *stop.borrow() {
                break;
            }
            if paused {
                continue;
            }
            if started.elapsed() >= Duration::from_secs(30) {
                delay = Duration::from_secs(1)
            }
            let until = Instant::now() + delay;
            loop {
                tokio::select! {_=stop.changed()=>break,_=resume.changed()=>break,_=tokio::time::sleep_until(until)=>break,Some(_)=tasks.join_next()=>{}}
            }
            delay = (delay * 2).min(Duration::from_secs(30));
        }
        events.close();
        if tokio::time::timeout(Duration::from_secs(5), async {
            while tasks.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        }
        let mut status = self.status.lock().unwrap();
        status.connected = false;
        status.detail = "stopped".into();
        Ok(())
    }
}
async fn send(socket: &mut Socket, frame: &Frame) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(serde_json::to_string(frame)?)),
    )
    .await
    .map_err(|_| anyhow!("provider write timed out; outcome unknown"))?
    .map_err(|_| anyhow!("provider connection lost; outcome unknown"))
}
async fn receive(socket: &mut Socket) -> Result<Frame> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => return Frame::decode(text.as_bytes()),
            Some(Ok(Message::Binary(bytes))) => return Frame::decode(&bytes),
            Some(Ok(Message::Ping(_))) => {
                tokio::time::timeout(Duration::from_secs(5), socket.flush())
                    .await
                    .map_err(|_| anyhow!("provider heartbeat timed out"))??;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(close))) => {
                let reason = DisconnectReason::RemoteClose {
                    code: close.as_ref().map(|close| u16::from(close.code)),
                    reason: close
                        .map(|close| close.reason.to_string())
                        .unwrap_or_default(),
                };
                let _ = tokio::time::timeout(Duration::from_secs(1), socket.flush()).await;
                return Err(reason.into());
            }
            _ => return Err(DisconnectReason::TransportLost.into()),
        }
    }
}
pub(crate) struct Owner {
    pub relay: Arc<Relay>,
    pub task: tokio::task::JoinHandle<Result<()>>,
    pub caller_task: Option<tokio::task::JoinHandle<Result<()>>>,
}
impl Owner {
    pub fn stop(&self) {
        self.relay.close();
    }
}
pub(crate) fn install(mut options: Options, hub: Handle) -> Result<(Options, Option<Owner>)> {
    let Some(config) = options.provider_relay.clone() else {
        return Ok((options, None));
    };
    let relay = Relay::new(config, hub, options.upstream_layout.clone())?;
    options.upstream_caller = relay.caller.clone();
    let caller_task = relay
        .caller
        .clone()
        .map(|caller| tokio::spawn(caller.run()));
    for method in ["providerRelay.status", "providerRelay.resume"] {
        let relay = relay.clone();
        options = options.handler(method, move |caller, _| {
            let relay = relay.clone();
            async move {
                anyhow::ensure!(
                    caller.authenticated_host,
                    "provider relay administration requires the actual host"
                );
                if method == "providerRelay.resume" {
                    relay.resume();
                }
                Ok(relay.status())
            }
        });
    }
    let running = relay.clone();
    let task = tokio::spawn(running.run());
    Ok((
        options,
        Some(Owner {
            relay,
            task,
            caller_task,
        }),
    ))
}
#[cfg(test)]
mod tests;

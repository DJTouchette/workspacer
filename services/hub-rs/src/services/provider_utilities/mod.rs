//! Small provider-derived UI facts: owned disposable inference, never agent sessions.
mod codex;
mod completion;
mod text;
mod tools;
use crate::{
    Options,
    services::{agent_lifecycle::Operation, config::Config},
};
use anyhow::Result;
use claudemon::daemon::embedded::EmbeddedClient;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
#[derive(Clone)]
struct Context {
    key: String,
    provider: String,
    binary: Option<PathBuf>,
    local: bool,
    enabled: bool,
}
trait Ping: Send + Sync {
    fn ping<'a>(
        &'a self,
        context: Context,
        home: PathBuf,
    ) -> Operation<'a, completion::Outcome<()>>;
}
struct NativePing;
impl Ping for NativePing {
    fn ping<'a>(
        &'a self,
        context: Context,
        home: PathBuf,
    ) -> Operation<'a, completion::Outcome<()>> {
        Box::pin(async move {
            let Some(binary) = context.binary else {
                return Ok(Err(text::Failure::Missing));
            };
            Ok(match context.provider.as_str() {
                "claude" => completion::claude_ping(&binary).await,
                "codex" => codex::ping(&binary, &home).await,
                _ => Err(text::Failure::Unsupported),
            })
        })
    }
}
struct Check {
    context: Context,
    automatic: bool,
    reply: oneshot::Sender<Value>,
}
struct Entry {
    value: Value,
    at: Option<Instant>,
    waiters: Vec<oneshot::Sender<Value>>,
}
pub struct Service {
    config: Arc<Config>,
    home: PathBuf,
    engine: Option<EmbeddedClient>,
    owned: bool,
    startup_delay: Duration,
    activated: std::sync::atomic::AtomicBool,
    activation: tokio::sync::Notify,
    ping: Arc<dyn Ping>,
    entries: Mutex<BTreeMap<String, Entry>>,
    requests: mpsc::Sender<Check>,
    receiver: Mutex<Option<mpsc::Receiver<Check>>>,
    stop: watch::Sender<bool>,
    titles: tokio::sync::Semaphore,
}
impl Service {
    fn new(config: Arc<Config>, home: PathBuf, engine: Option<EmbeddedClient>) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(32);
        let (stop, _) = watch::channel(false);
        Arc::new(Self {
            config,
            home,
            owned: engine.is_some(),
            engine,
            startup_delay: Duration::from_secs(2),
            activated: std::sync::atomic::AtomicBool::new(false),
            activation: tokio::sync::Notify::new(),
            ping: Arc::new(NativePing),
            entries: Mutex::new(BTreeMap::new()),
            requests: tx,
            receiver: Mutex::new(Some(rx)),
            stop,
            titles: tokio::sync::Semaphore::new(4),
        })
    }
    fn context(&self, provider: Option<&str>) -> Context {
        let config = self.config.get();
        let provider = provider
            .unwrap_or_else(|| {
                config["agents"]["managerProvider"]
                    .as_str()
                    .unwrap_or("claude")
            })
            .to_owned();
        let local =
            self.owned && (provider != "claude" || config["claude"]["transport"] == "stream");
        let binary = local
            .then(|| completion::binary(&provider, &config))
            .flatten();
        let key = json!([
            local,
            provider,
            binary,
            config["agents"],
            config["claude"],
            config["codex"]
        ])
        .to_string();
        Context {
            key,
            provider,
            binary,
            local,
            enabled: config["agents"]["checkProviderOnStartup"] != false,
        }
    }
    fn version(&self) -> String {
        let config = self.config.get();
        json!([config["agents"], config["claude"], config["codex"]]).to_string()
    }
    fn activate(&self) {
        if !self
            .activated
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            self.activation.notify_one();
        }
    }
    fn read(&self, provider: &str) -> Value {
        self.activate();
        let context = self.context(Some(provider));
        if !context.local {
            return json!({"state":"unsupported"});
        }
        self.entries
            .lock()
            .unwrap()
            .get(&context.key)
            .map(|entry| entry.value.clone())
            .unwrap_or_else(|| json!({"state":"unchecked"}))
    }
    async fn check(&self, provider: &str) -> Value {
        self.activate();
        let context = self.context(Some(provider));
        if !context.local {
            return json!({"state":"unsupported"});
        }
        if context.binary.is_none() {
            return json!({"state":"unchecked"});
        }
        let (reply, rx) = oneshot::channel();
        if self
            .requests
            .try_send(Check {
                context,
                automatic: false,
                reply,
            })
            .is_err()
        {
            return self.read(provider);
        }
        rx.await.unwrap_or_else(|_| json!({"state":"unchecked"}))
    }
    fn clear(&self) {
        let entries = std::mem::take(&mut *self.entries.lock().unwrap());
        for (_, entry) in entries {
            for waiter in entry.waiters {
                let _ = waiter.send(json!({"state":"unchecked"}));
            }
        }
    }
    pub fn close(&self) {
        self.stop.send_replace(true);
    }
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let mut receiver = self
            .receiver
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| anyhow::anyhow!("provider readiness already started"))?;
        let mut stop = self.stop.subscribe();
        let mut jobs = tokio::task::JoinSet::new();
        let mut version = self.version();
        let mut poll = tokio::time::interval(Duration::from_millis(200));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let startup = tokio::time::sleep(self.startup_delay);
        tokio::pin!(startup);
        let mut started = false;
        let mut armed = false;
        loop {
            if *stop.borrow() {
                break;
            }
            tokio::select! {biased;
                _=stop.changed()=>break,
                _=poll.tick()=>{let current=self.version();if current!=version{version=current;jobs.abort_all();while jobs.join_next().await.is_some(){}self.clear();}},
                _=self.activation.notified(),if !armed=>{armed=true;startup.as_mut().reset(Instant::now()+self.startup_delay);},
                _=&mut startup,if armed&&!started=>{started=true;let context=self.context(None);if context.enabled&&context.local&&context.binary.is_some(){let(reply,_)=oneshot::channel();self.admit(Check{context,automatic:true,reply},&mut jobs);}},
                request=receiver.recv()=>{let Some(request)=request else{break;};self.admit(request,&mut jobs);},
                Some(result)=jobs.join_next()=>{if let Ok((context,mut result))=result{if self.context(Some(&context.provider)).key!=context.key{result=json!({"state":"unchecked"});}if let Some(entry)=self.entries.lock().unwrap().get_mut(&context.key){entry.value=result.clone();entry.at=Some(Instant::now());for waiter in entry.waiters.drain(..){let _=waiter.send(result.clone());}}}},
            }
        }
        jobs.abort_all();
        while jobs.join_next().await.is_some() {}
        self.clear();
        Ok(())
    }
    fn admit(&self, request: Check, jobs: &mut tokio::task::JoinSet<(Context, Value)>) {
        let current = self.context(Some(&request.context.provider));
        if current.key != request.context.key
            || !current.local
            || current.binary.is_none()
            || request.automatic && !current.enabled
        {
            let _ = request.reply.send(json!({"state":"unchecked"}));
            return;
        }
        let key = current.key.clone();
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(&key) {
            if entry.at.is_none() {
                entry.waiters.push(request.reply);
                return;
            }
            if request.automatic
                || entry
                    .at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
            {
                let _ = request.reply.send(entry.value.clone());
                return;
            }
        }
        if jobs.len() >= 5 {
            let _ = request.reply.send(json!({"state":"unchecked"}));
            return;
        }
        entries.insert(
            key.clone(),
            Entry {
                value: json!({"state":"checking"}),
                at: None,
                waiters: vec![request.reply],
            },
        );
        let ping = self.ping.clone();
        let home = self.home.clone();
        jobs.spawn(async move {
            let state = match tokio::time::timeout(
                Duration::from_secs(20),
                ping.ping(current.clone(), home),
            )
            .await
            {
                Ok(Ok(Ok(()))) => "responding",
                Ok(Ok(Err(reason))) => reason.state(),
                Ok(Err(_)) => "error",
                Err(_) => "timeout",
            };
            (
                current,
                json!({"state":state,"checkedAt":chrono::Utc::now().timestamp_millis()}),
            )
        });
    }
    async fn title(&self, request: &Value) -> Result<Value> {
        anyhow::ensure!(request.is_object(), "title request must be an object");
        let config = self.config.get();
        if config["agents"]["autoTitle"]["enabled"] == false {
            return Ok(Value::Null);
        }
        let user = request["userMessage"].as_str().unwrap_or("").trim();
        if user.is_empty() {
            return Ok(Value::Null);
        }
        let fallback = text::fallback(user);
        let provider = request["provider"].as_str().unwrap_or("claude");
        let Ok(_permit) = self.titles.try_acquire() else {
            return Ok(json!(fallback));
        };
        let prompt = text::prompt(user, request["assistantReply"].as_str().unwrap_or(""));
        let output =
            completion::title(provider, &config, &self.home, self.engine.as_ref(), &prompt).await;
        Ok(json!(
            output
                .ok()
                .and_then(|raw| text::sanitize(&raw))
                .or(fallback)
        ))
    }
}
pub(crate) fn install(mut options: Options, config: Arc<Config>, home: PathBuf) -> Options {
    let service = Service::new(config, home, options.engine.clone());
    for method in [
        "desktop.agentSuggestTitle",
        "desktop.providerReadiness",
        "desktop.toolsStatus",
    ] {
        let service = service.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            async move {
                anyhow::ensure!(
                    caller.authenticated_host,
                    "desktop services require the authenticated server owner's connection"
                );
                match method {
                    "desktop.agentSuggestTitle" => service.title(&params["request"]).await,
                    "desktop.providerReadiness" => {
                        let provider = params["provider"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 64)
                            .ok_or_else(|| anyhow::anyhow!("provider required"))?;
                        Ok(if params["check"] == true {
                            service.check(provider).await
                        } else {
                            service.read(provider)
                        })
                    }
                    _ => Ok(tools::status()),
                }
            }
        });
    }
    options.provider_utilities = Some(service);
    options
}
#[cfg(test)]
mod tests;

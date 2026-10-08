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
/// The opening request as a title prompt can use it (same cap as the prompt).
pub fn clip_prompt(message: &str) -> String {
    text::clip(message.trim(), 1200)
}
/// Writes one title. The CLI implementation is the only one outside tests;
/// the seam exists so title triggering can be proven without a model call.
pub(crate) trait Generate: Send + Sync {
    fn generate<'a>(
        &'a self,
        provider: &'a str,
        model: Option<&'a str>,
        config: &'a Value,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = completion::Outcome<String>> + Send + 'a>>;
    /// Writes one handoff brief (see [`completion::Limits::BRIEF`]).
    fn brief<'a>(
        &'a self,
        _provider: &'a str,
        _model: Option<&'a str>,
        _config: &'a Value,
        _prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = completion::Outcome<String>> + Send + 'a>>
    {
        Box::pin(async { Err(text::Failure::Unsupported) })
    }
}
struct Cli {
    home: PathBuf,
    engine: Option<EmbeddedClient>,
}
impl Generate for Cli {
    fn generate<'a>(
        &'a self,
        provider: &'a str,
        model: Option<&'a str>,
        config: &'a Value,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = completion::Outcome<String>> + Send + 'a>>
    {
        Box::pin(completion::title(
            provider,
            model,
            config,
            &self.home,
            self.engine.as_ref(),
            prompt,
        ))
    }
    fn brief<'a>(
        &'a self,
        provider: &'a str,
        model: Option<&'a str>,
        config: &'a Value,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = completion::Outcome<String>> + Send + 'a>>
    {
        Box::pin(completion::oneshot(
            provider,
            model,
            config,
            &self.home,
            self.engine.as_ref(),
            prompt,
            completion::Limits::BRIEF,
        ))
    }
}
/// Who wrote (or was asked to write) a handoff summary, and what came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BriefOutcome {
    pub provider: String,
    pub model: Option<String>,
    /// The model's text, or why there is none (a [`text::Failure::reason`]).
    pub text: Result<String, &'static str>,
}
/// One title attempt, reported truthfully: a fallback (the first line of the
/// user's own message) is never presented as a model-written title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleOutcome {
    pub title: Option<String>,
    /// `model`, `fallback`, or `none` when there was nothing to title from.
    pub source: &'static str,
    pub provider: String,
    pub model: Option<String>,
    pub reason: Option<&'static str>,
}
impl TitleOutcome {
    pub fn wire(&self) -> Value {
        let mut value = json!({"title":self.title,"source":self.source,"provider":self.provider,"model":self.model});
        if let Some(reason) = self.reason {
            value["reason"] = json!(reason);
        }
        value
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
    briefs: tokio::sync::Semaphore,
    generator: Arc<dyn Generate>,
}
impl Service {
    fn new(config: Arc<Config>, home: PathBuf, engine: Option<EmbeddedClient>) -> Arc<Self> {
        let generator = Arc::new(Cli {
            home: home.clone(),
            engine: engine.clone(),
        });
        Self::with_generator(config, home, engine, generator)
    }
    pub(crate) fn with_generator(
        config: Arc<Config>,
        home: PathBuf,
        engine: Option<EmbeddedClient>,
        generator: Arc<dyn Generate>,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(32);
        let (stop, _) = watch::channel(false);
        Arc::new(Self {
            config,
            home,
            owned: engine.is_some(),
            startup_delay: Duration::from_secs(2),
            activated: std::sync::atomic::AtomicBool::new(false),
            activation: tokio::sync::Notify::new(),
            ping: Arc::new(NativePing),
            entries: Mutex::new(BTreeMap::new()),
            requests: tx,
            receiver: Mutex::new(Some(rx)),
            stop,
            titles: tokio::sync::Semaphore::new(4),
            briefs: tokio::sync::Semaphore::new(2),
            generator,
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
    /// Whether automatic titling is on in this hub's shared config.
    pub fn titles_enabled(&self) -> bool {
        self.config.get()["agents"]["autoTitle"]["enabled"] != false
    }
    /// Title an opening exchange with the configured harness and model.
    /// `wait` queues for a slot instead of degrading to the fallback at once.
    pub async fn suggest(
        &self,
        agent_provider: &str,
        user: &str,
        reply: &str,
        wait: bool,
    ) -> TitleOutcome {
        let config = self.config.get();
        let target = text::title_target(&config, agent_provider);
        let fallback = text::fallback(user);
        let outcome = |title: Option<String>, source, reason| TitleOutcome {
            title,
            source,
            provider: target.provider.clone(),
            model: target.model.clone(),
            reason,
        };
        let degraded = |reason| match &fallback {
            Some(title) => outcome(Some(title.clone()), "fallback", Some(reason)),
            None => outcome(None, "none", Some(reason)),
        };
        if user.trim().is_empty() {
            return outcome(None, "none", Some("empty"));
        }
        let _permit = if wait {
            match self.titles.acquire().await {
                Ok(permit) => permit,
                Err(_) => return degraded("busy"),
            }
        } else {
            match self.titles.try_acquire() {
                Ok(permit) => permit,
                Err(_) => return degraded("busy"),
            }
        };
        let prompt = text::prompt(user, reply);
        match self
            .generator
            .generate(&target.provider, target.model.as_deref(), &config, &prompt)
            .await
        {
            Ok(raw) => match text::sanitize(&raw) {
                Some(title) => outcome(Some(title), "model", None),
                None => degraded("empty"),
            },
            Err(failure) => degraded(failure.reason()),
        }
    }
    /// Have the cheap title harness and model write a handoff brief from a
    /// prepared prompt. Same target as titles (`agents.autoTitle`): the
    /// session's own harness unless one is configured, Haiku for Claude.
    /// Waits for one of two slots; the caller owns the overall deadline.
    pub async fn summarize_handoff(&self, agent_provider: &str, prompt: &str) -> BriefOutcome {
        let config = self.config.get();
        let target = text::title_target(&config, agent_provider);
        let outcome = |text| BriefOutcome {
            provider: target.provider.clone(),
            model: target.model.clone(),
            text,
        };
        let Ok(_permit) = self.briefs.acquire().await else {
            return outcome(Err("busy"));
        };
        match self
            .generator
            .brief(&target.provider, target.model.as_deref(), &config, prompt)
            .await
        {
            Ok(text) if !text.trim().is_empty() => outcome(Ok(text)),
            Ok(_) => outcome(Err(text::Failure::Empty.reason())),
            Err(failure) => outcome(Err(failure.reason())),
        }
    }
    async fn title(&self, request: &Value) -> Result<Value> {
        anyhow::ensure!(request.is_object(), "title request must be an object");
        if !self.titles_enabled() {
            return Ok(Value::Null);
        }
        let user = request["userMessage"].as_str().unwrap_or("").trim();
        if user.is_empty() {
            return Ok(Value::Null);
        }
        let provider = request["provider"].as_str().unwrap_or("claude");
        let outcome = self
            .suggest(
                provider,
                user,
                request["assistantReply"].as_str().unwrap_or(""),
                false,
            )
            .await;
        if outcome.source == "fallback" {
            eprintln!(
                "title: no {} title ({}); using the first line of the request",
                outcome.provider,
                outcome.reason.unwrap_or("failed")
            );
        }
        Ok(json!(outcome.title))
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

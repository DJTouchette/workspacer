//! Persisted Web Push identities and subscriptions. Delivery is bounded, owned,
//! and never retried automatically. Tests inject the transport; no live sends.
mod crypto;
mod watch;
use crate::{Caller, Handle, Options, client::Client, services::agent_lifecycle::Operation};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use web_push_native::p256::elliptic_curve::subtle::ConstantTimeEq;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Needs,
    Finished,
    Ended,
    Checkpoint,
    Test,
}
#[derive(Clone)]
struct Notification {
    kind: Kind,
    title: String,
    body: String,
    detail: String,
    session: String,
    ran_for: i64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    needs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished_after_sec: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoints: Option<bool>,
}
impl Preferences {
    fn wants(&self, event: &Notification) -> bool {
        match event.kind {
            Kind::Needs => self.needs.unwrap_or(true),
            Kind::Finished => {
                self.finished.unwrap_or(true)
                    && event.ran_for
                        >= self
                            .finished_after_sec
                            .filter(|seconds| *seconds >= 0)
                            .unwrap_or(60)
                            .saturating_mul(1000)
            }
            Kind::Ended => self.ended.unwrap_or(true),
            Kind::Checkpoint => self.checkpoints.unwrap_or(false),
            Kind::Test => true,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Subscription {
    endpoint: String,
    keys: SubscriptionKeys,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    token_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created: Option<String>,
    #[serde(default)]
    prefs: Preferences,
}
trait Sender: Send + Sync + 'static {
    fn send<'a>(
        &'a self,
        keys: &'a crypto::Keys,
        subscription: &'a Subscription,
        payload: Vec<u8>,
    ) -> Operation<'a, u16>;
}
struct Network {
    client: reqwest::Client,
}
impl Sender for Network {
    fn send<'a>(
        &'a self,
        keys: &'a crypto::Keys,
        subscription: &'a Subscription,
        payload: Vec<u8>,
    ) -> Operation<'a, u16> {
        Box::pin(async move {
            let request = crypto::request(keys, subscription, payload)?;
            let (parts, body) = request.into_parts();
            let response = self
                .client
                .post(parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(|_| anyhow!("push transport failed"))?;
            Ok(response.status().as_u16())
        })
    }
}
pub(crate) struct Manager {
    path: PathBuf,
    keys: crypto::Keys,
    subscriptions: Mutex<BTreeMap<String, Subscription>>,
    valid: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    sender: Arc<dyn Sender>,
    sends: Arc<tokio::sync::Semaphore>,
    broadcasts: Arc<tokio::sync::Semaphore>,
    stop: tokio::sync::watch::Sender<bool>,
}
impl Manager {
    fn open(
        root: PathBuf,
        valid: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        sender: Arc<dyn Sender>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&root)?;
        let _identity = crate::auth::StoreLock::take(&root.join("vapid.json"))?;
        let path = root.join("push-subscriptions.json");
        let list: Vec<Subscription> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(list.len() <= 512, "push subscription capacity exceeded");
        let mut subscriptions: BTreeMap<_, _> = list
            .into_iter()
            .map(|row| (row.endpoint.clone(), row))
            .collect();
        let key_path = root.join("vapid.json");
        let keys = std::fs::read(&key_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<crypto::Keys>(&bytes).ok())
            .filter(|keys| keys.validate().is_ok());
        let keys = if let Some(keys) = keys {
            keys
        } else {
            let lost = subscriptions.len();
            let keys = crypto::Keys::generate();
            if lost > 0 {
                eprintln!(
                    "push: VAPID key state loss; {lost} old subscriptions cannot receive and must subscribe again"
                );
                super::atomic_json(&path, &json!([]), true)?;
                subscriptions.clear();
            }
            super::atomic_json(&key_path, &serde_json::to_value(&keys)?, true)?;
            keys
        };
        let (stop, _) = tokio::sync::watch::channel(false);
        Ok(Arc::new(Self {
            path,
            keys,
            subscriptions: Mutex::new(subscriptions),
            valid,
            sender,
            sends: Arc::new(tokio::sync::Semaphore::new(8)),
            broadcasts: Arc::new(tokio::sync::Semaphore::new(32)),
            stop,
        }))
    }
    fn change<T>(
        &self,
        mutate: impl FnOnce(&mut BTreeMap<String, Subscription>) -> Result<T>,
    ) -> Result<T> {
        let mut current = self.subscriptions.lock().unwrap();
        let mut next = current.clone();
        let result = mutate(&mut next)?;
        super::atomic_json(
            &self.path,
            &serde_json::to_value(next.values().collect::<Vec<_>>())?,
            true,
        )?;
        *current = next;
        Ok(result)
    }
    fn subscribe(&self, caller: Caller, params: Value) -> Result<Value> {
        let mut row: Subscription = serde_json::from_value(params)?;
        crypto::endpoint(&row.endpoint)?;
        crypto::subscription_keys(&row.keys)?;
        anyhow::ensure!(row.endpoint.len() <= 8192, "push endpoint too long");
        row.token_id = caller.token_id;
        row.scope = caller.scope;
        row.created = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        self.change(|rows| {
            anyhow::ensure!(
                rows.contains_key(&row.endpoint) || rows.len() < 512,
                "push subscription capacity exceeded"
            );
            rows.insert(row.endpoint.clone(), row);
            Ok(())
        })?;
        Ok(json!({"ok":true}))
    }
    fn unsubscribe(&self, params: Value) -> Result<Value> {
        let endpoint = params["endpoint"].as_str().unwrap_or("");
        let auth = params["auth"].as_str().unwrap_or("");
        self.change(|rows| {
            if let Some(row) = rows.get(endpoint) {
                anyhow::ensure!(
                    row.keys.auth.is_empty()
                        || bool::from(row.keys.auth.as_bytes().ct_eq(auth.as_bytes())),
                    "push.unsubscribe auth does not match subscription"
                );
                rows.remove(endpoint);
            }
            Ok(())
        })?;
        Ok(json!({"ok":true}))
    }
    fn list(&self) -> Value {
        let rows: Vec<_> = self
            .subscriptions
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        json!({"subscriptions":rows.iter().map(|row|json!({"endpoint":row.endpoint,"tokenId":row.token_id,"scope":row.scope,"created":row.created.as_deref().unwrap_or("0001-01-01T00:00:00Z"),"revoked":!row.token_id.is_empty()&&!(self.valid)(&row.token_id)})).collect::<Vec<_>>()})
    }
    fn revoke(&self, params: Value) -> Result<Value> {
        let endpoint = params["endpoint"].as_str().unwrap_or("");
        let token = params["tokenId"].as_str().unwrap_or("");
        anyhow::ensure!(
            !endpoint.is_empty() || !token.is_empty(),
            "push.revoke requires endpoint or tokenId"
        );
        let removed = self.change(|rows| {
            let before = rows.len();
            rows.retain(|_, row| {
                !((!endpoint.is_empty() && row.endpoint == endpoint)
                    || (!token.is_empty() && row.token_id == token))
            });
            Ok(before - rows.len())
        })?;
        Ok(json!({"ok":true,"removed":removed}))
    }
    async fn send(&self, row: Subscription, payload: Vec<u8>) -> u16 {
        let Ok(_permit) = self.sends.clone().acquire_owned().await else {
            return 0;
        };
        if !row.token_id.is_empty() && !(self.valid)(&row.token_id) {
            return 0;
        }
        let status = match tokio::time::timeout(
            Duration::from_secs(10),
            self.sender.send(&self.keys, &row, payload),
        )
        .await
        {
            Ok(Ok(status)) => status,
            _ => 0,
        };
        if status == 404 || status == 410 {
            if let Err(error) = self.change(|rows| {
                if rows.get(&row.endpoint).is_some_and(|current| {
                    current.keys.auth == row.keys.auth && current.token_id == row.token_id
                }) {
                    rows.remove(&row.endpoint);
                }
                Ok(())
            }) {
                eprintln!("push: dead endpoint could not be pruned: {error}");
            }
        } else if !(200..300).contains(&status) {
            let host = url::Url::parse(&row.endpoint)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
                .unwrap_or_else(|| "push service".into());
            eprintln!("push: delivery to {host} failed (status {status})");
        }
        status
    }
    async fn broadcast(&self, event: Notification) -> Result<Value> {
        let _permit = self
            .broadcasts
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow!("push delivery queue is full"))?;
        let rows: Vec<_> = self
            .subscriptions
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect();
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|row| {
                (row.token_id.is_empty() || (self.valid)(&row.token_id)) && row.prefs.wants(&event)
            })
            .collect();
        let devices = rows.len();
        use futures_util::StreamExt;
        let statuses:Vec<u16>=futures_util::stream::iter(rows).map(|row|{let body=if !event.detail.is_empty()&&row.prefs.preview.unwrap_or(true){format!("{} — {}",event.body,event.detail)}else{event.body.clone()};let payload=serde_json::to_vec(&json!({"title":event.title,"body":body,"sessionId":event.session,"kind":event.kind})).unwrap();async move{self.send(row,payload).await}}).buffer_unordered(8).collect().await;
        Ok(
            json!({"ok":true,"devices":devices,"delivered":statuses.iter().filter(|status|(200..300).contains(*status)).count(),"gone":statuses.iter().filter(|status|**status==404||**status==410).count(),"failed":statuses.iter().filter(|status|!(200..300).contains(*status)&&**status!=404&&**status!=410).count()}),
        )
    }
    fn handlers(self: &Arc<Self>, mut options: Options) -> Options {
        for method in [
            "push.key",
            "push.subscribe",
            "push.unsubscribe",
            "push.list",
            "push.revoke",
            "push.test",
        ] {
            let service = self.clone();
            options = options.handler(method, move |caller, params| {
                let service = service.clone();
                async move {
                    match method {
                        "push.key" => Ok(json!({"publicKey":service.keys.public_key})),
                        "push.subscribe" => service.subscribe(caller, params),
                        "push.unsubscribe" => service.unsubscribe(params),
                        "push.list" => Ok(service.list()),
                        "push.revoke" => service.revoke(params),
                        _ => {
                            service
                                .broadcast(Notification {
                                    kind: Kind::Test,
                                    title: "Workspacer".into(),
                                    body: "Test notification — push is working".into(),
                                    detail: String::new(),
                                    session: String::new(),
                                    ran_for: 0,
                                })
                                .await
                        }
                    }
                }
            });
        }
        options
    }
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
    async fn run(self: Arc<Self>, hub: Handle) -> Result<()> {
        let mut stopping = self.stop.subscribe();
        tokio::select! {_=stopping.changed()=>return Ok(()),ready=hub.ready()=>{ready?;}}
        let client = Client::connect_service(&hub).await?;
        let mut events = client.events();
        client
            .topics(BTreeSet::from(["agent.snapshot".into()]))
            .await?;
        let mut watcher = watch::Watcher::default();
        let mut deliveries = tokio::task::JoinSet::new();
        loop {
            tokio::select! {biased;
             _=stopping.changed()=>{client.close();deliveries.abort_all();while deliveries.join_next().await.is_some(){}return Ok(())}
             Some(_)=deliveries.join_next()=>{},
             event=events.recv()=>match event{Ok(event)=>{if let Some(data)=event.data{for notification in watcher.snapshot(&data,chrono::Utc::now().timestamp_millis()){if deliveries.len()>=32{eprintln!("push: snapshot delivery queue full; notification skipped");continue}let manager=self.clone();deliveries.spawn(async move{if let Err(error)=manager.broadcast(notification).await{eprintln!("push: notification failed: {error}");}});}}},Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{},Err(_)=>bail!("push observer disconnected")}
            }
        }
    }
}
pub(crate) struct Observer {
    manager: Arc<Manager>,
    pub task: tokio::task::JoinHandle<Result<()>>,
}
impl Observer {
    pub fn stop(&self) {
        self.manager.stop();
    }
}
pub(crate) fn install(options: Options, hub: Handle) -> (Options, Option<Observer>) {
    let Some(root) = options
        .push_dir
        .clone()
        .or_else(|| options.config_dir.clone())
    else {
        return (options, None);
    };
    if root.as_os_str().is_empty() {
        return (options, None);
    }
    let host = crate::auth::fingerprint(&options.token);
    let scoped = options.scoped_tokens.clone();
    let valid = Arc::new(move |fingerprint: &str| {
        fingerprint.is_empty()
            || (!host.is_empty() && fingerprint == host)
            || scoped
                .as_ref()
                .and_then(|path| crate::auth::load(path).ok())
                .is_some_and(|records| {
                    records.iter().any(|record| {
                        record.scope().is_some()
                            && crate::auth::fingerprint(&record.token) == fingerprint
                    })
                })
    });
    let opened = (|| {
        let sender = Network {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .https_only(true)
                .build()?,
        };
        Manager::open(root, valid, Arc::new(sender))
    })();
    match opened {
        Ok(manager) => {
            let options = manager.handlers(options);
            let running = manager.clone();
            let task = tokio::spawn(async move { running.run(hub).await });
            (options, Some(Observer { manager, task }))
        }
        Err(error) => {
            eprintln!("push: disabled ({error})");
            (options, None)
        }
    }
}
#[cfg(test)]
mod tests;

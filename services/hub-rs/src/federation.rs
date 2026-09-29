//! Owned outbound peer links. Local routing authorization stays in the broker;
//! the destination additionally enforces the configured peer credential.
pub mod config;
use crate::{
    Handle,
    client::{Client, DisconnectReason},
    protocol::{Event, matches, provider_timeout},
};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinSet;

pub const FORWARD_TOPICS: [&str; 2] = ["agent.*", "workflow.*"];

pub fn load_peers(path: &std::path::Path) -> Result<Vec<Peer>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut peers: Vec<Peer> =
        serde_json::from_slice::<Option<Vec<Peer>>>(&bytes)?.unwrap_or_default();
    let mut names = HashSet::new();
    for peer in &mut peers {
        peer.name = peer.name.trim().to_owned();
        peer.url = peer.url.trim().to_owned();
        peer.validate()?;
        if !names.insert(peer.name.clone()) {
            bail!("duplicate peer name");
        }
    }
    Ok(peers)
}

// Deliberately no Debug: peer credentials must not reach diagnostic logs.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Peer {
    pub name: String,
    pub url: String,
    pub token: String,
    pub dispatch: bool,
}
impl Peer {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            bail!("peer name must use letters, digits, - or _");
        }
        let url = url::Url::parse(&self.url).map_err(|_| anyhow!("invalid peer URL"))?;
        if !matches!(url.scheme(), "ws" | "wss") || url.host_str().is_none() {
            bail!("peer URL must be ws:// or wss:// with a host");
        }
        Ok(())
    }
    pub fn link_url(&self) -> Result<String> {
        self.validate()?;
        let mut url = url::Url::parse(&self.url).map_err(|_| anyhow!("invalid peer URL"))?;
        // Replace existing peer flags: Go's query decoder uses the first value.
        let pairs: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key != "peer")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        url.set_query(None);
        url.query_pairs_mut()
            .extend_pairs(pairs)
            .append_pair("peer", "1");
        Ok(url.into())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub name: String,
    pub connected: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub power_paused: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub last_seen: i64,
    pub dispatch: bool,
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}
struct State {
    info: PeerInfo,
    client: Option<Client>,
    stopped: bool,
}
struct Link {
    peer: Peer,
    state: Mutex<State>,
    resume: tokio::sync::watch::Sender<u64>,
}
pub struct Manager {
    hub: Handle,
    routes: Routes,
    workers: HashMap<String, tokio::task::AbortHandle>,
    tasks: JoinSet<()>,
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.tasks.abort_all();
        for link in self.routes.links.lock().unwrap().iter() {
            close_link(link);
        }
    }
}
fn close_link(link: &Link) {
    let mut state = link.state.lock().unwrap();
    state.stopped = true;
    if let Some(client) = state.client.take() {
        client.close();
    }
    state.info.connected = false;
}
#[derive(Clone, Default)]
pub struct Routes {
    links: Arc<Mutex<Vec<Arc<Link>>>>,
}
impl Routes {
    pub fn dispatch_enabled(&self, peer: &str) -> bool {
        self.links
            .lock()
            .unwrap()
            .iter()
            .any(|link| link.peer.name == peer && link.peer.dispatch)
    }
    pub fn peers(&self) -> Vec<PeerInfo> {
        self.links
            .lock()
            .unwrap()
            .iter()
            .map(|link| link.state.lock().unwrap().info.clone())
            .collect()
    }
    /// Explicit host wake intent. Merely forwarding a call must never resume it.
    pub fn resume_peer(&self, peer: &str) -> Result<bool> {
        let link = self
            .links
            .lock()
            .unwrap()
            .iter()
            .find(|link| link.peer.name == peer)
            .cloned()
            .ok_or_else(|| anyhow!("unknown federation peer"))?;
        let mut state = link.state.lock().unwrap();
        if state.stopped {
            bail!("federation link is stopped");
        }
        let resumed = state.info.power_paused;
        if resumed {
            state.info.power_paused = false;
            link.resume
                .send_modify(|generation| *generation = generation.wrapping_add(1));
        }
        Ok(resumed)
    }
    pub async fn forward(&self, peer: &str, method: &str, params: Value) -> Result<Value> {
        let link = self
            .links
            .lock()
            .unwrap()
            .iter()
            .find(|link| link.peer.name == peer)
            .cloned()
            .ok_or_else(|| anyhow!("unknown federation peer"))?;
        let client = {
            let state = link.state.lock().unwrap();
            if state.info.power_paused {
                bail!(
                    "federation peer is paused after machine stop; explicit owner resume is required; call was not submitted"
                );
            }
            state
                .client
                .clone()
                .ok_or_else(|| anyhow!("federation peer disconnected; call was not submitted"))?
        };
        client
            .call_with_timeout(method, params, forwarding_budget(method))
            .await
    }
}
impl Manager {
    pub fn routes(&self) -> Routes {
        self.routes.clone()
    }
    pub fn start(hub: Handle, peers: Vec<Peer>) -> Result<Self> {
        let mut manager = Self {
            hub,
            routes: Routes::default(),
            workers: HashMap::new(),
            tasks: JoinSet::new(),
        };
        manager.replace(peers)?;
        Ok(manager)
    }
    /// Replace only changed links. Existing in-flight calls retain their old
    /// connection; closing it fails those calls instead of routing them elsewhere.
    pub fn replace(&mut self, peers: Vec<Peer>) -> Result<()> {
        let mut names = HashSet::new();
        for peer in &peers {
            peer.validate()?;
            if !names.insert(&peer.name) {
                bail!("duplicate peer name");
            }
        }
        while self.tasks.try_join_next().is_some() {}
        // Editing a name, dispatch flag or credential is not a wake request.
        // Carry the pause across replacement links for the same endpoint.
        let paused_endpoints: HashSet<String> = self
            .routes
            .links
            .lock()
            .unwrap()
            .iter()
            .filter(|link| link.state.lock().unwrap().info.power_paused)
            .map(|link| link.peer.url.clone())
            .collect();
        let mut previous: HashMap<_, _> = self
            .routes
            .links
            .lock()
            .unwrap()
            .iter()
            .map(|link| (link.peer.name.clone(), link.clone()))
            .collect();
        let mut next = Vec::new();
        for peer in peers {
            if let Some(old) = previous.remove(&peer.name) {
                if old.peer == peer {
                    next.push(old);
                    continue;
                }
                self.stop_link(&old);
            }
            let link = Arc::new(Link {
                state: Mutex::new(State {
                    info: PeerInfo {
                        name: peer.name.clone(),
                        connected: false,
                        power_paused: paused_endpoints.contains(&peer.url),
                        last_seen: 0,
                        dispatch: peer.dispatch,
                    },
                    client: None,
                    stopped: false,
                }),
                resume: tokio::sync::watch::channel(0).0,
                peer,
            });
            self.workers.insert(
                link.peer.name.clone(),
                self.tasks.spawn(run_link(self.hub.clone(), link.clone())),
            );
            next.push(link);
        }
        for old in previous.values() {
            self.stop_link(old);
        }
        *self.routes.links.lock().unwrap() = next;
        Ok(())
    }
    fn stop_link(&mut self, link: &Link) {
        if let Some(worker) = self.workers.remove(&link.peer.name) {
            worker.abort();
        }
        close_link(link);
        let last = link.state.lock().unwrap().info.last_seen;
        let last = if last == 0 {
            "0001-01-01T00:00:00Z".to_owned()
        } else {
            chrono::DateTime::from_timestamp_millis(last)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        let _ = self.hub.publish(Event::new(
            "hub.peer.disconnected",
            "federation",
            json!({"peer":link.peer.name,"lastSeen":last}),
        ));
    }
    pub fn peers(&self) -> Vec<PeerInfo> {
        self.routes.peers()
    }
    pub fn dispatch_enabled(&self, peer: &str) -> bool {
        self.routes
            .links
            .lock()
            .unwrap()
            .iter()
            .any(|link| link.peer.name == peer && link.peer.dispatch)
    }
    pub async fn forward(&self, peer: &str, method: &str, params: Value) -> Result<Value> {
        self.routes().forward(peer, method, params).await
    }
    pub async fn shutdown(&mut self) {
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
        for link in self.routes.links.lock().unwrap().iter() {
            close_link(link);
        }
        self.workers.clear();
    }
}

pub fn forwarding_budget(method: &str) -> Duration {
    provider_timeout(method, Duration::from_secs(30)) - Duration::from_secs(5)
}

fn forward_event(peer: &str, mut event: Event) -> Option<Event> {
    if !event.hub.is_empty()
        || !FORWARD_TOPICS
            .iter()
            .any(|pattern| matches(pattern, &event.topic))
    {
        return None;
    }
    event.hub = peer.into();
    event.id.clear();
    Some(event)
}

async fn pause(hub: &Handle, link: &Link) -> bool {
    let last_seen = {
        let mut state = link.state.lock().unwrap();
        if state.stopped {
            return false;
        }
        state.info.power_paused = true;
        state.info.connected = false;
        if let Some(client) = state.client.take() {
            client.close();
        }
        state.info.last_seen
    };
    let last_seen = if last_seen == 0 {
        "0001-01-01T00:00:00Z".into()
    } else {
        chrono::DateTime::from_timestamp_millis(last_seen)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    hub.publish_wait(Event::new(
        "hub.peer.disconnected",
        "federation",
        json!({"peer":link.peer.name,"lastSeen":last_seen,"powerPaused":true}),
    ))
    .await
    .is_ok()
}
async fn wait_unpaused(link: &Link) -> bool {
    let mut resume = link.resume.subscribe();
    loop {
        {
            let state = link.state.lock().unwrap();
            if state.stopped {
                return false;
            }
            if !state.info.power_paused {
                return true;
            }
        }
        if resume.changed().await.is_err() {
            return false;
        }
    }
}
async fn run_link(hub: Handle, link: Arc<Link>) {
    let address = link.peer.link_url().expect("validated peer URL");
    let mut backoff = Duration::from_millis(250);
    loop {
        if !wait_unpaused(&link).await {
            return;
        }
        let client = match Client::connect_remote(&address, &link.peer.token).await {
            Ok(client) => client,
            Err(error) => {
                if error
                    .downcast_ref::<DisconnectReason>()
                    .is_some_and(DisconnectReason::is_power_paused)
                {
                    if !pause(&hub, &link).await {
                        return;
                    }
                    continue;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
                continue;
            }
        };
        let mut events = client.events();
        if client
            .topics(BTreeSet::from(FORWARD_TOPICS.map(str::to_owned)))
            .await
            .is_err()
        {
            let reason = client.disconnected_reason().await;
            if reason.is_power_paused() {
                if !pause(&hub, &link).await {
                    return;
                }
            } else {
                client.close();
                tokio::time::sleep(backoff).await;
            }
            continue;
        }
        backoff = Duration::from_millis(250);
        {
            let mut state = link.state.lock().unwrap();
            if state.stopped {
                client.close();
                return;
            }
            state.client = Some(client.clone());
            state.info.connected = true;
            state.info.last_seen = chrono::Utc::now().timestamp_millis();
        }
        if hub
            .publish_wait(Event::new(
                "hub.peer.connected",
                "federation",
                json!({"peer":link.peer.name}),
            ))
            .await
            .is_err()
        {
            client.close();
            return;
        }
        let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
        let reason = loop {
            tokio::select! {
                biased;
                reason=client.disconnected_reason()=>break reason,
                event=events.recv()=>match event {
                    Ok(event)=>if let Some(event)=forward_event(&link.peer.name,event){if hub.publish_wait(event).await.is_err(){client.close();return;}},
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{
                        // Reseed fleet consumers on the existing socket. Closing
                        // it here could discard a queued4001 and turn repair into
                        // an unintended wake through a new network handshake.
                        if hub.publish_wait(Event::new("hub.peer.connected","federation",json!({"peer":link.peer.name,"resync":true}))).await.is_err(){client.close();return;}
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed)=>break client.disconnect_reason().unwrap_or(DisconnectReason::TransportLost),
                },
                _=heartbeat.tick()=>{link.state.lock().unwrap().info.last_seen=chrono::Utc::now().timestamp_millis();}
            }
        };
        if reason.is_power_paused() {
            if !pause(&hub, &link).await {
                return;
            }
            client.close();
            continue;
        }
        let last_seen = {
            let mut state = link.state.lock().unwrap();
            state.client = None;
            state.info.connected = false;
            chrono::DateTime::from_timestamp_millis(state.info.last_seen)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        client.close();
        client.disconnected().await;
        if hub
            .publish_wait(Event::new(
                "hub.peer.disconnected",
                "federation",
                json!({"peer":link.peer.name,"lastSeen":last_seen}),
            ))
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(backoff).await;
    }
}
/// Registration is internal; the public entry point independently checks actual
/// owner provenance rather than treating a scoped operator as wake authority.
pub(crate) fn install_resume(options: crate::Options, routes: Routes) -> crate::Options {
    options.handler("federation.resumePeer", move |caller, params| {
        let routes = routes.clone();
        async move {
            if !caller.authenticated_host
                || !caller.trusted
                || caller.federated
                || caller.scope != "operator"
            {
                bail!("resuming a paused federation link requires the server owner");
            }
            let peer = params["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("federation.resumePeer requires name"))?;
            Ok(json!({"ok":true,"resumed":routes.resume_peer(peer)?}))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forwarding_is_one_hop_and_excludes_remote_control_topics() {
        let event = Event::new("agent.updated", "brain", json!({"cwd":"/remote/path"}));
        let forwarded = forward_event("worker", event.clone()).unwrap();
        assert_eq!(forwarded.data, event.data);
        assert_eq!(forwarded.hub, "worker");
        assert!(forwarded.id.is_empty());
        assert!(forward_event("other", forwarded).is_none());
        assert!(forward_event("worker", Event::new("layout.changed", "hub", json!({}))).is_none());
    }
    #[test]
    fn peer_marker_cannot_be_shadowed_by_existing_query_or_fragment() {
        let peer = Peer {
            name: "worker".into(),
            url: "ws://localhost/bus?peer=0&x=1#fragment".into(),
            ..Peer::default()
        };
        let parsed = url::Url::parse(&peer.link_url().unwrap()).unwrap();
        assert_eq!(
            parsed
                .query_pairs()
                .filter(|(key, _)| key == "peer")
                .collect::<Vec<_>>(),
            vec![("peer".into(), "1".into())]
        );
        assert_eq!(forwarding_budget("agents.spawn"), Duration::from_secs(355));
        assert_eq!(forwarding_budget("agents.list"), Duration::from_secs(25));
    }
}

#[cfg(test)]
mod pause_tests {
    use super::*;
    use crate::{
        Hub, Options,
        auth::{self, Scope},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    async fn paused(client: &Client) -> Value {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let peers = client.call("federation.peers", json!({})).await.unwrap();
                if peers[0]["powerPaused"] == true {
                    return peers;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn two_hubs_pause_without_redial_and_only_owner_resumes_without_replaying_mutation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let mut options = Options::default()
            .handler("fixture.mutate", move |_, _| {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<Result<Value>>().await
                }
            })
            .handler("config.get", |_, _| async { Ok(json!({"ready":true})) });
        options.listen = Some("127.0.0.1:0".parse().unwrap());
        options.token = "peer-owner".into();
        let remote = Hub::start(options).unwrap();
        let address = remote.ready().await.unwrap().unwrap();
        let root = tempfile::tempdir().unwrap();
        let tokens = root.path().join("tokens.json");
        let operator = auth::mint(&tokens, Scope::Operator, "scoped-operator").unwrap();
        let mut options = Options::default();
        options.scoped_tokens = Some(tokens);
        options.federation_peers = vec![Peer {
            name: "worker".into(),
            url: format!("ws://{address}/bus"),
            token: "peer-owner".into(),
            dispatch: false,
        }];
        let local = Hub::start(options).unwrap();
        local.ready().await.unwrap();
        let owner = Client::connect(&local.handle()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if owner.call("federation.peers", json!({})).await.unwrap()[0]["connected"] == true
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let caller = owner.clone();
        let pending = tokio::spawn(async move {
            caller
                .call("hub:worker/fixture.mutate", json!({"once":true}))
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        remote.handle().disconnect_for_machine_stop().await.unwrap();
        assert!(
            pending
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("unknown")
        );
        paused(&owner).await;
        tokio::time::sleep(Duration::from_millis(850)).await;
        assert!(
            remote
                .handle()
                .quiescence_clients()
                .await
                .unwrap()
                .iter()
                .all(|c| c.internal || c.provider || c.plugin),
            "timer reconnected and could wake stopped machine"
        );
        assert!(
            owner
                .call("hub:worker/config.get", json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("paused")
        );
        let scoped = Client::from_connection(
            local
                .handle()
                .connect_authenticated(operator.token, false)
                .await
                .unwrap(),
        );
        assert!(
            scoped
                .call("federation.resumePeer", json!({"name":"worker"}))
                .await
                .is_err()
        );
        assert_eq!(
            owner
                .call("federation.resumePeer", json!({"name":"worker"}))
                .await
                .unwrap()["resumed"],
            true
        );
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if owner.call("hub:worker/config.get", json!({})).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a pending mutation was replayed"
        );
        assert!(
            owner.call("federation.peers", json!({})).await.unwrap()[0]
                .get("powerPaused")
                .is_none()
        );
        owner.close();
        scoped.close();
        local.shutdown().unwrap();
        remote.shutdown().unwrap();
    }
    #[tokio::test]
    async fn prehello_pause_survives_unchanged_peer_reload_until_explicit_resume() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{Message, protocol::CloseFrame};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let dials = Arc::new(AtomicUsize::new(0));
        let count = dials.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                socket
                    .send(Message::Close(Some(CloseFrame {
                        code: 4001.into(),
                        reason: "machine stopping".into(),
                    })))
                    .await
                    .unwrap();
                let _ = socket.next().await;
            }
        });
        let local = Hub::start(Options::default()).unwrap();
        local.ready().await.unwrap();
        let peer = Peer {
            name: "sleeping".into(),
            url: format!("ws://{address}"),
            token: "fixture".into(),
            dispatch: false,
        };
        let mut manager = Manager::start(local.handle(), vec![peer.clone()]).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !manager.peers()[0].power_paused {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        manager.replace(vec![peer.clone()]).unwrap();
        let mut edited = peer;
        edited.dispatch = true;
        edited.token = "rotated-owner-token".into();
        manager.replace(vec![edited]).unwrap();
        tokio::time::sleep(Duration::from_millis(850)).await;
        assert_eq!(dials.load(Ordering::SeqCst), 1);
        assert!(manager.routes().resume_peer("sleeping").unwrap());
        tokio::time::timeout(Duration::from_secs(3), async {
            while dials.load(Ordering::SeqCst) < 2 || !manager.peers()[0].power_paused {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(dials.load(Ordering::SeqCst), 2);
        manager.shutdown().await;
        server.abort();
        local.shutdown().unwrap();
    }
}

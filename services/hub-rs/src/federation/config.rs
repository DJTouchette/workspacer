//! Owner-only persisted peer configuration. Tokens are never returned to clients.
use super::{Peer, load_peers};
use crate::{Caller, Handle, Options};
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

fn owner(caller: &Caller) -> Result<()> {
    if !caller.authenticated_host || !caller.trusted || caller.scope != "operator" {
        bail!("peer configuration requires the server owner");
    }
    Ok(())
}
#[derive(Deserialize)]
struct Request {
    peers: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    name: String,
    url: String,
    token: Option<String>,
    #[serde(default)]
    dispatch: bool,
}

// Keep commit and enqueue in the same owned blocking operation. A request
// timeout may drop its JoinHandle, but cannot strand a completed disk write
// without publishing it. The save lock is released only after enqueue, so the
// actor's FIFO order is also the persistent commit order. Hub runtime shutdown
// joins blocking work with its existing bounded cleanup/uncertainty fence.
async fn commit_and_apply(
    hub: Handle,
    guard: tokio::sync::OwnedMutexGuard<()>,
    commit: impl FnOnce() -> Result<Vec<Peer>> + Send + 'static,
) -> Result<()> {
    let (publication, applied) = hub.reserve_peer_replacement().await?;
    tokio::task::spawn_blocking(move || -> Result<()> {
        let _guard = guard;
        let peers = commit()?;
        publication.publish(peers);
        Ok(())
    })
    .await
    .context(
        "peer configuration writer stopped; inspect persisted configuration before retrying",
    )??;
    applied.await.context("peer configuration was committed but live application was not confirmed before hub shutdown")??;
    Ok(())
}

pub(crate) fn install(
    mut options: Options,
    hub: Handle,
    path: Option<PathBuf>,
    fixed: Vec<Peer>,
) -> Options {
    let serial = Arc::new(tokio::sync::Mutex::new(()));
    let read_path = path.clone();
    let read_serial = serial.clone();
    options = options.handler("federation.peersConfig", move |caller, _| {
        let path = read_path.clone();
        let serial = read_serial.clone();
        async move {
            owner(&caller)?;
            let path = path.ok_or_else(|| anyhow!("peer configuration is disabled by the server launcher"))?;
            let guard = serial.lock_owned().await;
            tokio::task::spawn_blocking(move || {
                let _guard = guard;
                Ok(json!(load_peers(&path)?.iter().map(|peer| json!({
                    "name":peer.name,"url":peer.url,"hasToken":!peer.token.is_empty(),"dispatch":peer.dispatch
                })).collect::<Vec<_>>()))
            }).await?
        }
    });
    options.handler("federation.savePeersConfig", move |caller, params| {
        let (path, fixed, hub, serial) = (path.clone(), fixed.clone(), hub.clone(), serial.clone());
        async move {
            owner(&caller)?;
            let path = path
                .ok_or_else(|| anyhow!("peer configuration is disabled by the server launcher"))?;
            let request: Request = serde_json::from_value(params)?;
            if request.peers.len() > 64 {
                bail!("peers must be an array of at most 64 entries");
            }
            let guard = serial.lock_owned().await;
            commit_and_apply(hub, guard, move || -> Result<_> {
                let tokens: HashMap<_, _> = load_peers(&path)?
                    .into_iter()
                    .map(|peer| (peer.name, peer.token))
                    .collect();
                let mut names = HashSet::new();
                let mut peers = Vec::new();
                for entry in request.peers {
                    let name = entry.name.trim().to_owned();
                    let address = entry.url.trim().to_owned();
                    let url = url::Url::parse(&address).map_err(|_| anyhow!("invalid peer URL"))?;
                    if !url.username().is_empty()
                        || url.password().is_some()
                        || url.query().is_some()
                        || url.fragment().is_some()
                    {
                        bail!("peer URL must be a credential-free ws:// or wss:// address");
                    }
                    if name.len() > 128 || !names.insert(name.clone()) {
                        bail!("invalid or duplicate peer name");
                    }
                    let token = entry
                        .token
                        .map(|token| token.trim().to_owned())
                        .unwrap_or_else(|| tokens.get(&name).cloned().unwrap_or_default());
                    if token.len() > 4096 {
                        bail!("peer token is too long");
                    }
                    let peer = Peer {
                        name,
                        url: address,
                        token,
                        dispatch: entry.dispatch,
                    };
                    peer.validate()?;
                    peers.push(peer);
                }
                for peer in &fixed {
                    peer.validate()?;
                    if !names.insert(peer.name.clone()) {
                        bail!("duplicate peer name");
                    }
                }
                crate::services::atomic_json(&path, &serde_json::to_value(&peers)?, true)?;
                peers.extend(fixed);
                Ok(peers)
            })
            .await?;
            Ok(json!({"ok":true}))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hub, client::Client};
    use serde_json::Value;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    struct Gate {
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        released: Mutex<bool>,
        ready: Condvar,
        second_entered: std::sync::atomic::AtomicBool,
    }
    impl Gate {
        fn wait(&self) {
            if let Some(entered) = self.entered.lock().unwrap().take() {
                let _ = entered.send(());
            }
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.ready.wait(released).unwrap();
            }
        }
        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.ready.notify_all();
        }
    }
    struct ReleaseOnDrop(Arc<Gate>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    async fn fixture(
        path: PathBuf,
        timeout: Duration,
    ) -> (Hub, Client, Arc<Gate>, tokio::sync::oneshot::Receiver<()>) {
        let (entered, receiver) = tokio::sync::oneshot::channel();
        let gate = Arc::new(Gate {
            entered: Mutex::new(Some(entered)),
            released: Mutex::new(false),
            ready: Condvar::new(),
            second_entered: std::sync::atomic::AtomicBool::new(false),
        });
        let serial = Arc::new(tokio::sync::Mutex::new(()));
        let handle = Arc::new(Mutex::new(None::<Handle>));
        let mut options = Options::default();
        options.peers_file = Some(path.clone());
        options.call_timeout = timeout;
        let handler_handle = handle.clone();
        let handler_gate = gate.clone();
        options = options.handler("fixture.savePeers", move |caller, params| {
            let hub = handler_handle.lock().unwrap().clone().unwrap();
            let (path, serial, gate) = (path.clone(), serial.clone(), handler_gate.clone());
            async move {
                owner(&caller)?;
                let name = params["name"].as_str().context("name missing")?.to_owned();
                if name == "second" {
                    gate.second_entered
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
                let guard = serial.lock_owned().await;
                commit_and_apply(hub, guard, move || {
                    let peers = vec![Peer {
                        name: name.clone(),
                        url: "ws://127.0.0.1:0/bus".into(),
                        dispatch: true,
                        ..Default::default()
                    }];
                    crate::services::atomic_json(&path, &serde_json::to_value(&peers)?, true)?;
                    if name == "first" {
                        // Pause AFTER durable commit, while its caller can expire.
                        gate.wait();
                    }
                    Ok(peers)
                })
                .await?;
                Ok(json!({"ok":true}))
            }
        });
        let hub = Hub::start(options).unwrap();
        *handle.lock().unwrap() = Some(hub.handle());
        hub.ready().await.unwrap();
        let client = Client::connect(&hub.handle()).await.unwrap();
        (hub, client, gate, receiver)
    }
    async fn live(client: &Client, name: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(rows) = client.call("federation.peers", json!({})).await {
                    if rows[0]["name"] == name {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn committed_peer_update_applies_after_the_request_deadline() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let (hub, client, gate, entered) = fixture(path.clone(), Duration::from_millis(200)).await;
        let _release = ReleaseOnDrop(gate.clone());
        let caller = client.clone();
        let request = tokio::spawn(async move {
            caller
                .call("fixture.savePeers", json!({"name":"first"}))
                .await
        });
        entered.await.unwrap();
        assert_eq!(load_peers(&path).unwrap()[0].name, "first");
        assert!(
            request
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        gate.release();
        live(&client, "first").await;
        client.close();
        hub.shutdown().unwrap();
    }
    #[tokio::test]
    async fn caller_cancellation_and_concurrent_saves_preserve_commit_order() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let (hub, client, gate, entered) = fixture(path.clone(), Duration::from_secs(5)).await;
        let _release = ReleaseOnDrop(gate.clone());
        let first = client.clone();
        let first = tokio::spawn(async move {
            first
                .call("fixture.savePeers", json!({"name":"first"}))
                .await
        });
        entered.await.unwrap();
        let second = client.clone();
        let second = tokio::spawn(async move {
            second
                .call("fixture.savePeers", json!({"name":"second"}))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !gate
                .second_entered
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        first.abort();
        let _ = first.await;
        assert_eq!(load_peers(&path).unwrap()[0].name, "first");
        gate.release();
        assert_eq!(second.await.unwrap().unwrap(), json!({"ok":true}));
        live(&client, "second").await;
        assert_eq!(load_peers(&path).unwrap()[0].name, "second");
        client.close();
        hub.shutdown().unwrap();
    }
    #[tokio::test]
    async fn shutdown_joins_an_accepted_blocking_commit_before_a_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let (hub, client, gate, entered) = fixture(path.clone(), Duration::from_secs(5)).await;
        let _release = ReleaseOnDrop(gate.clone());
        let request = tokio::spawn(async move {
            client
                .call("fixture.savePeers", json!({"name":"first"}))
                .await
        });
        entered.await.unwrap();
        let shutting_down = tokio::task::spawn_blocking(move || hub.shutdown());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !shutting_down.is_finished(),
            "shutdown abandoned its blocking writer"
        );
        gate.release();
        shutting_down.await.unwrap().unwrap();
        let _ = request.await;
        let mut options = Options::default();
        options.peers_file = Some(path.clone());
        let restarted = Hub::start(options).unwrap();
        restarted.ready().await.unwrap();
        let caller = Client::connect(&restarted.handle()).await.unwrap();
        live(&caller, "first").await;
        let persisted: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted[0]["name"], "first");
        caller.close();
        restarted.shutdown().unwrap();
    }
}

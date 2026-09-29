//! Owner-only persisted peer configuration. Tokens are never returned to clients.
use super::{Peer, load_peers};
use crate::{Caller, Handle, Options};
use anyhow::{Result, anyhow, bail};
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
            // Keep the transaction lock inside the blocking task even if the
            // caller times out while storage is committing.
            let (guard, peers) = tokio::task::spawn_blocking(move || -> Result<_> {
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
                Ok((guard, peers))
            })
            .await??;
            let _guard = guard;
            hub.replace_peers(peers).await?;
            Ok(json!({"ok":true}))
        }
    })
}

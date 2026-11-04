//! Host routing on top of the local queue: `queue.targets`, `host: "<peer>"`
//! and `host: "any"` on submit, the peer fan-out of `queue.list`, remote
//! cancel, and the overlap lookup the job scheduler uses.
//!
//! Everything a peer reports here only chooses a target. The destination
//! re-checks admission and authority itself (`submit_local` on that hub), and
//! work is never forwarded a second hop: a federated caller may only submit to
//! the hub it is connected to.
use super::{
    QUEUED, RUNNING, WorkQueue, is_owner, may_spawn, model,
    placement::{self, Candidate, ProviderReady, Request},
};
use crate::{Caller, federation::Routes};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Budget for one peer `host.resources` read.
pub const RESOURCES_BUDGET: Duration = Duration::from_secs(5);
/// A cached peer sample younger than this is used instead of asking.
pub const RESOURCES_FRESH: Duration = Duration::from_secs(30);
/// Budget for one readiness read (provider login probes run in parallel and
/// are bounded at 3 s each on the answering host).
pub const CAPABILITIES_BUDGET: Duration = Duration::from_secs(10);
/// Per-peer budget of the `queue.list` fan-out and overlap lookups.
pub const LIST_BUDGET: Duration = Duration::from_secs(10);

/// Where a submission should go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    Local,
    Peer(String),
    Any,
}

/// A federation peer name has this shape (`federation::Peer::validate`).
pub fn peer_name_shaped(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// Read `params.host`. `""`/`"local"` and `"any"` are reserved words, so a
/// peer that happens to be named one of them cannot be addressed by it.
pub fn parse_host(params: &Value) -> Result<Host> {
    match params.get("host") {
        None | Some(Value::Null) => Ok(Host::Local),
        Some(Value::String(host)) => match host.as_str() {
            "" | "local" => Ok(Host::Local),
            "any" => Ok(Host::Any),
            peer if peer_name_shaped(peer) => Ok(Host::Peer(peer.to_owned())),
            _ => bail!("host must be \"\", \"local\", \"any\" or a federation peer name"),
        },
        Some(_) => bail!("host must be a string"),
    }
}

/// What an item asks of a host, for placement.
pub fn request_for(item: &model::Item) -> Request {
    match (&item.spawn, &item.shell) {
        (Some(spawn), _) => Request {
            kind: "spawn".into(),
            provider: spawn.provider.clone(),
            cwd: spawn.cwd.clone(),
        },
        (_, Some(shell)) => Request {
            kind: "shell".into(),
            provider: String::new(),
            cwd: shell.cwd.clone(),
        },
        _ => Request::default(),
    }
}

/// Is this a transport failure whose effect on the peer cannot be known?
fn outcome_unknown(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("outcome is unknown") || text.contains("outcome unknown")
}

/// Turn a failed forward into an error that says plainly whether the peer
/// may have acted on it. An uncertain outcome is never retried here.
fn forward_error(peer: &str, method: &str, error: anyhow::Error) -> anyhow::Error {
    if outcome_unknown(&error) {
        anyhow!(
            "{method} on peer {peer}: the outcome is unknown ({error}). It may or may not have taken effect there and is not retried automatically; check hub:{peer}/queue.list before trying again"
        )
    } else if error.to_string().contains("not submitted") {
        anyhow!("{method} on peer {peer} was not sent: {error}")
    } else {
        anyhow!("peer {peer} refused {method}: {error}")
    }
}

impl WorkQueue {
    fn routes(&self) -> Option<&Routes> {
        self.federation.get()
    }

    /// The origin-side authority to route an item anywhere: the same bar the
    /// destination applies to a local caller. The destination still checks
    /// its own caller (this hub's peer credential) independently.
    fn check_origin_authority(&self, caller: &Caller, item: &model::Item) -> Result<()> {
        match item.kind.as_str() {
            "shell" if !is_owner(caller) => {
                bail!("queue.submit of a shell item requires the server owner")
            }
            "spawn" if !may_spawn(caller) => {
                bail!("queue.submit of a spawn item requires the authority to call agents.spawn")
            }
            _ => {}
        }
        if item.spawn.as_ref().is_some_and(|s| s.full_access) && !is_owner(caller) {
            bail!("spawn.fullAccess requires the server owner");
        }
        Ok(())
    }

    /// `queue.submit` with host routing. Returns `{host, item}`: `host` is
    /// this hub's name for a local landing, else the peer name.
    pub async fn submit_routed(self: &Arc<Self>, caller: &Caller, params: Value) -> Result<Value> {
        let host = parse_host(&params)?;
        if host != Host::Local && caller.federated {
            bail!(
                "a federation peer may only submit to the hub it is connected to (host \"local\"); work is never forwarded a second hop"
            );
        }
        match host {
            Host::Local => self.submit_here(caller, params).await,
            Host::Peer(peer) => self.submit_peer(caller, &peer, params).await,
            Host::Any => {
                let item = model::parse_submit(&params)?;
                self.check_origin_authority(caller, &item)?;
                let (targets, best) = self.place(&request_for(&item), caller).await;
                match best {
                    Some(best) if best == "local" => self.submit_here(caller, params).await,
                    Some(peer) => self.submit_peer(caller, &peer, params).await,
                    None => {
                        let why: Vec<String> = targets
                            .iter()
                            .map(|t| {
                                let reasons: Vec<&str> = t.row["reasons"]
                                    .as_array()
                                    .map(|r| r.iter().filter_map(Value::as_str).collect())
                                    .unwrap_or_default();
                                format!("{}: {}", t.host, reasons.join(", "))
                            })
                            .collect();
                        bail!("no eligible host for this item ({})", why.join("; "))
                    }
                }
            }
        }
    }

    async fn submit_here(self: &Arc<Self>, caller: &Caller, params: Value) -> Result<Value> {
        let (queue, owned) = (self.clone(), caller.clone());
        let item =
            tokio::task::spawn_blocking(move || queue.submit_local(&owned, &params)).await??;
        Ok(json!({"host": self.host(), "item": item.wire()}))
    }

    /// Forward to `hub:<peer>/queue.submit` after checking the peer is
    /// configured, connected and dispatch-enabled.
    async fn submit_peer(
        self: &Arc<Self>,
        caller: &Caller,
        peer: &str,
        mut params: Value,
    ) -> Result<Value> {
        let item = model::parse_submit(&params)?;
        self.check_origin_authority(caller, &item)?;
        let routes = self
            .routes()
            .ok_or_else(|| anyhow!("unknown federation peer {peer:?}"))?;
        let info = routes
            .peers()
            .into_iter()
            .find(|info| info.name == peer)
            .ok_or_else(|| anyhow!("unknown federation peer {peer:?}"))?;
        if !info.dispatch {
            bail!("federation peer {peer} is not dispatch-enabled; nothing was submitted");
        }
        if !info.connected {
            bail!("federation peer {peer} is not connected; nothing was submitted");
        }
        params["host"] = json!("local");
        if !params["source"].is_object() {
            params["source"] = json!({});
        }
        params["source"]["kind"] = json!("remote");
        if params["source"]["origin"]
            .as_str()
            .is_none_or(str::is_empty)
        {
            params["source"]["origin"] = json!(self.host());
        }
        let mut result = routes
            .forward(peer, "queue.submit", params)
            .await
            .map_err(|error| forward_error(peer, "queue.submit", error))?;
        if !result["item"].is_object() {
            bail!(
                "queue.submit on peer {peer}: the outcome is unknown (the peer answered without an item); not retried automatically"
            );
        }
        result["host"] = json!(peer);
        Ok(result)
    }

    /// `queue.cancel` with host routing. Remote cancel is the owner's.
    pub async fn cancel_routed(self: &Arc<Self>, caller: &Caller, params: Value) -> Result<Value> {
        let id = params["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("queue.cancel requires {{id}}"))?
            .to_owned();
        match parse_host(&params)? {
            Host::Local => {
                let (queue, owned) = (self.clone(), caller.clone());
                let item = tokio::task::spawn_blocking(move || queue.cancel(&owned, &id)).await??;
                Ok(json!({"ok": true, "item": item.wire()}))
            }
            Host::Any => bail!("queue.cancel needs the host the item is on, not \"any\""),
            Host::Peer(peer) => {
                if !is_owner(caller) {
                    bail!("queue.cancel on a peer requires the server owner");
                }
                let routes = self
                    .routes()
                    .ok_or_else(|| anyhow!("unknown federation peer {peer:?}"))?;
                routes
                    .forward(&peer, "queue.cancel", json!({"id": id, "host": "local"}))
                    .await
                    .map_err(|error| forward_error(&peer, "queue.cancel", error))
            }
        }
    }

    /// `queue.list {includePeers: true}`: this hub's list plus one row per
    /// configured peer, each read concurrently within its own budget.
    pub async fn list_with_peers(self: &Arc<Self>, caller: &Caller) -> Value {
        let mut list = self.list();
        let Some(routes) = self.routes().filter(|_| !caller.federated) else {
            return list;
        };
        let peers = routes.peers();
        let rows = futures_util::future::join_all(peers.into_iter().map(|info| async move {
            let failed = |error: String| json!({"peer": info.name, "ok": false, "error": error, "items": []});
            if !info.connected {
                return failed("not connected".into());
            }
            match tokio::time::timeout(
                LIST_BUDGET,
                routes.forward(&info.name, "queue.list", json!({})),
            )
            .await
            {
                Ok(Ok(result)) => json!({
                    "peer": info.name,
                    "ok": true,
                    "items": result["items"].as_array().cloned().unwrap_or_default(),
                }),
                Ok(Err(error)) => failed(error.to_string()),
                Err(_) => failed(format!(
                    "no answer within {} s",
                    LIST_BUDGET.as_secs()
                )),
            }
        }))
        .await;
        list["peers"] = json!(rows);
        list
    }

    /// Remember a peer's `host.resources` (from a forwarded event).
    pub(crate) fn note_peer_resources(&self, peer: &str, resources: Value) {
        if peer.is_empty() || !resources.is_object() {
            return;
        }
        self.peer_resources
            .lock()
            .unwrap()
            .insert(peer.to_owned(), (Instant::now(), resources));
    }

    async fn peer_resources(&self, routes: &Routes, peer: &str) -> Result<Value> {
        if let Some((at, value)) = self.peer_resources.lock().unwrap().get(peer)
            && at.elapsed() < RESOURCES_FRESH
        {
            return Ok(value.clone());
        }
        let value = tokio::time::timeout(
            RESOURCES_BUDGET,
            routes.forward(peer, "host.resources", json!({})),
        )
        .await
        .map_err(|_| anyhow!("no answer within {} s", RESOURCES_BUDGET.as_secs()))??;
        if !value.is_object() {
            bail!("malformed host.resources answer");
        }
        self.note_peer_resources(peer, value.clone());
        Ok(value)
    }

    /// Provider readiness and directory choices, from the remote-dispatch
    /// readiness probe (`fleet.dispatchCapabilities` on a peer).
    async fn capabilities(
        &self,
        routes: Option<&Routes>,
        peer: Option<&str>,
    ) -> Result<(Vec<ProviderReady>, Vec<String>)> {
        let caps: crate::services::remote_dispatch::Capabilities = match (routes, peer) {
            (Some(routes), Some(peer)) => serde_json::from_value(
                tokio::time::timeout(
                    CAPABILITIES_BUDGET,
                    routes.forward(peer, "fleet.dispatchCapabilities", Value::Null),
                )
                .await
                .map_err(|_| anyhow!("no answer within {} s", CAPABILITIES_BUDGET.as_secs()))??,
            )?,
            _ => {
                let execution = self
                    .execution
                    .as_ref()
                    .ok_or_else(|| anyhow!("no readiness probe on this host"))?;
                tokio::time::timeout(CAPABILITIES_BUDGET, execution.capabilities())
                    .await
                    .map_err(|_| {
                        anyhow!("no answer within {} s", CAPABILITIES_BUDGET.as_secs())
                    })??
            }
        };
        Ok((
            caps.providers
                .into_iter()
                .map(|p| ProviderReady {
                    provider: p.provider,
                    found: p.found,
                    authenticated: p.authenticated,
                })
                .collect(),
            caps.cwds.into_iter().map(|d| d.path).collect(),
        ))
    }

    /// Observe this host and every configured peer (concurrently), then
    /// score them. A federated caller sees only this host.
    pub async fn place(
        self: &Arc<Self>,
        request: &Request,
        caller: &Caller,
    ) -> (Vec<placement::Target>, Option<String>) {
        let local = async {
            let mut candidate = Candidate {
                host: "local".into(),
                local: true,
                connected: true,
                dispatch: true,
                resources: Some(self.resources()),
                cwd_exists: std::path::Path::new(&request.cwd).is_absolute()
                    && std::path::Path::new(&request.cwd).is_dir(),
                ..Default::default()
            };
            match self.capabilities(None, None).await {
                Ok((providers, cwds)) => {
                    candidate.providers = Some(providers);
                    candidate.cwds = cwds;
                }
                Err(error) if self.execution.is_some() => {
                    candidate.notes.push(format!("readiness: {error}"));
                }
                Err(_) => {}
            }
            candidate
        };
        let routes = self.routes().filter(|_| !caller.federated);
        let peers = routes.map(Routes::peers).unwrap_or_default();
        let remote = futures_util::future::join_all(peers.into_iter().map(|info| async move {
            let mut candidate = Candidate {
                host: info.name.clone(),
                connected: info.connected,
                dispatch: info.dispatch,
                ..Default::default()
            };
            let Some(routes) = routes.filter(|_| info.connected) else {
                return candidate;
            };
            let (resources, capabilities) = futures_util::future::join(
                self.peer_resources(routes, &info.name),
                self.capabilities(Some(routes), Some(&info.name)),
            )
            .await;
            match resources {
                Ok(resources) => candidate.resources = Some(resources),
                Err(error) => candidate.notes.push(format!("resources: {error}")),
            }
            match capabilities {
                Ok((providers, cwds)) => {
                    candidate.providers = Some(providers);
                    candidate.cwds = cwds;
                }
                Err(error) => candidate.notes.push(format!("capabilities: {error}")),
            }
            candidate
        }));
        let (local, remote) = futures_util::future::join(local, remote).await;
        let mut candidates = vec![local];
        candidates.extend(remote);
        placement::evaluate(request, &candidates)
    }

    /// `queue.targets {kind?, provider?, cwd?}`.
    pub async fn targets(self: &Arc<Self>, caller: &Caller, params: &Value) -> Result<Value> {
        if let Some(map) = params.as_object() {
            for key in map.keys() {
                if !matches!(key.as_str(), "kind" | "provider" | "cwd") {
                    bail!("unknown queue.targets field {key:?}");
                }
            }
        }
        let text = |key: &str| -> Result<String> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(text)) => Ok(text.trim().to_owned()),
                Some(_) => bail!("{key} must be a string"),
            }
        };
        let kind = match text("kind")?.as_str() {
            "" | "spawn" => "spawn",
            "shell" => "shell",
            _ => bail!("kind must be \"spawn\" or \"shell\""),
        };
        let request = Request {
            kind: kind.into(),
            provider: text("provider")?,
            cwd: text("cwd")?,
        };
        let (targets, best) = self.place(&request, caller).await;
        Ok(placement::wire(&targets, best))
    }

    /// The job scheduler's overlap check: an item this job submitted that is
    /// still queued or running, as `(id, state, host)`. `last` is the item the
    /// scheduler last saw this job submit, as `(host, id)`. Local items are
    /// also found by their `source.jobId`, which survives a hub restart. An
    /// unreachable peer is an error: the caller treats it as still running.
    pub async fn job_item_active(
        self: &Arc<Self>,
        job_id: &str,
        last: Option<(&str, &str)>,
    ) -> Result<Option<(String, String, String)>> {
        let active = |item: &Value| matches!(item["state"].as_str(), Some(QUEUED | RUNNING));
        let mine = |item: &Value, id: Option<&str>| {
            id.is_some_and(|id| item["id"] == id)
                || (!job_id.is_empty() && item["source"]["jobId"] == job_id)
        };
        let last_local = last.filter(|(host, _)| *host == "local").map(|(_, id)| id);
        for item in self.snapshot() {
            let item = item.wire();
            if active(&item) && mine(&item, last_local) {
                return Ok(Some((
                    item["id"].as_str().unwrap_or("").into(),
                    item["state"].as_str().unwrap_or("").into(),
                    "local".into(),
                )));
            }
        }
        let Some((peer, id)) = last.filter(|(host, _)| *host != "local") else {
            return Ok(None);
        };
        let routes = self
            .routes()
            .ok_or_else(|| anyhow!("peer {peer} is no longer configured"))?;
        let list = tokio::time::timeout(LIST_BUDGET, routes.forward(peer, "queue.list", json!({})))
            .await
            .map_err(|_| anyhow!("no answer from {peer} within {} s", LIST_BUDGET.as_secs()))??;
        Ok(list["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| active(item) && mine(item, Some(id)))
            .map(|item| {
                (
                    item["id"].as_str().unwrap_or("").into(),
                    item["state"].as_str().unwrap_or("").into(),
                    peer.to_owned(),
                )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_parse_with_reserved_words_and_peer_shapes() {
        for (value, host) in [
            (json!({}), Host::Local),
            (json!({"host": null}), Host::Local),
            (json!({"host": ""}), Host::Local),
            (json!({"host": "local"}), Host::Local),
            (json!({"host": "any"}), Host::Any),
            (
                json!({"host": "build-box_2"}),
                Host::Peer("build-box_2".into()),
            ),
        ] {
            assert_eq!(parse_host(&value).unwrap(), host, "{value}");
        }
        for bad in [
            json!({"host": "hub:x/queue.submit"}),
            json!({"host": "a b"}),
            json!({"host": 3}),
        ] {
            assert!(parse_host(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn forward_failures_say_whether_the_outcome_is_known() {
        let unknown = forward_error(
            "box",
            "queue.submit",
            anyhow!("hub call timed out; outcome is unknown, do not automatically retry"),
        )
        .to_string();
        assert!(unknown.contains("the outcome is unknown"), "{unknown}");
        assert!(unknown.contains("not retried automatically"), "{unknown}");
        let lost = forward_error(
            "box",
            "queue.submit",
            anyhow!("peer connection lost; outcome is unknown"),
        );
        assert!(lost.to_string().contains("the outcome is unknown"));
        let unsent = forward_error(
            "box",
            "queue.submit",
            anyhow!("federation peer disconnected; call was not submitted"),
        )
        .to_string();
        assert!(unsent.contains("was not sent"), "{unsent}");
        let refused = forward_error(
            "box",
            "queue.submit",
            anyhow!("queue.submit of a shell item requires the server owner"),
        )
        .to_string();
        assert!(refused.starts_with("peer box refused"), "{refused}");
    }
}

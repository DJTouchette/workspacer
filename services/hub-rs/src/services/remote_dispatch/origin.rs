use super::{
    Capabilities, Journal, OriginRecord, PROTOCOL, Update, dispatch_id, now, sanitize_entry,
    valid_id,
};
use crate::{Caller, Handle, protocol::Event, services::agent_lifecycle::Operation};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
pub trait Link: Send + Sync + 'static {
    fn dispatch_enabled(&self, peer: &str) -> bool;
    fn call<'a>(&'a self, peer: &'a str, method: &'a str, params: Value) -> Operation<'a, Value>;
}
impl Link for crate::federation::Routes {
    fn dispatch_enabled(&self, peer: &str) -> bool {
        self.dispatch_enabled(peer)
    }
    fn call<'a>(&'a self, peer: &'a str, method: &'a str, params: Value) -> Operation<'a, Value> {
        Box::pin(self.forward(peer, method, params))
    }
}
pub trait Delivery: Send + Sync + 'static {
    /// Resolve only a live local parent, including authenticated succession.
    fn recipient<'a>(&'a self, owner: &'a str) -> Operation<'a, Option<String>>;
    /// Ok means the owner engine acknowledged this exact wake, not merely that
    /// bytes were written to a transport. Any error retains uncertain delivery.
    fn deliver<'a>(
        &'a self,
        recipient: &'a str,
        record: &'a OriginRecord,
        update: &'a Update,
    ) -> Operation<'a, ()>;
    /// Optional local durable receipt lookup. Never accept peer assertions here.
    fn confirmed<'a>(&'a self, _record: &'a OriginRecord, _seq: u64) -> Operation<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}
pub(crate) trait Booking: Send + Sync {
    fn local_session_id(&self) -> &str;
    /// Host-derived local task project; never taken from the remote wire payload.
    fn source_root(&self) -> &str;
    /// Runs after remote cwd preparation, before the one and only engine call.
    /// Returns trusted local receipt fields and the rendered message.
    fn prepare<'a>(&'a self, record: &'a OriginRecord, prepared: &'a Value)
    -> Operation<'a, Value>;
}
pub struct Origin {
    journal: Journal<OriginRecord>,
    link: Arc<dyn Link>,
    delivery: Arc<dyn Delivery>,
    hub: Handle,
    routing: Option<Arc<crate::services::routing::RoutingService>>,
    deliveries: tokio::sync::Mutex<()>,
    observing: tokio::sync::watch::Sender<bool>,
}
impl Origin {
    pub fn open(
        root: PathBuf,
        hub: Handle,
        link: Arc<dyn Link>,
        delivery: Arc<dyn Delivery>,
    ) -> Result<Arc<Self>> {
        Self::open_with_routing(root, hub, link, delivery, None)
    }
    pub(crate) fn open_with_routing(
        root: PathBuf,
        hub: Handle,
        link: Arc<dyn Link>,
        delivery: Arc<dyn Delivery>,
        routing: Option<Arc<crate::services::routing::RoutingService>>,
    ) -> Result<Arc<Self>> {
        let journal = Journal::open_legacy(
            root.join("remote-dispatch-origin.json"),
            root.join("remote-dispatches.json"),
            "dispatchId",
            "id",
            |record: &OriginRecord| {
                if !valid_id(&record.dispatch_id)
                    || record.peer.is_empty()
                    || record.owner_session_id.is_empty()
                    || record.acked_seq > super::MAX_SEQ
                    || !matches!(record.state.as_str(), "open" | "done" | "failed" | "lost")
                {
                    bail!("invalid origin dispatch journal")
                }
                if record.local_session_id.as_ref().is_some_and(|id| {
                    id.strip_prefix("paired:")
                        .is_none_or(|uuid| uuid::Uuid::parse_str(uuid).is_err())
                }) {
                    bail!("invalid paired proxy identity")
                }
                if let Some(update) = &record.last_update {
                    update.validate()?;
                    if update.dispatch_id != record.dispatch_id
                        || record
                            .session_id
                            .as_ref()
                            .is_some_and(|session| session != &update.session_id)
                    {
                        bail!("invalid retained remote evidence")
                    }
                }
                if record
                    .delivering_seq
                    .is_some_and(|seq| seq == 0 || seq > super::MAX_SEQ)
                {
                    bail!("invalid delivery receipt")
                }
                Ok(record.dispatch_id.clone())
            },
        )?;
        // Old 'lost' meant missing remote knowledge, never proof of no effect.
        if journal.list().iter().any(|row| row.state == "lost") {
            journal.change(|rows| {
                for row in rows.values_mut() {
                    if row.state == "lost" {
                        row.state = "open".into();
                        row.note =
                            Some("Remote outcome unknown; do not repeat spawn or wake".into());
                    }
                }
                Ok(())
            })?;
        }
        Ok(Arc::new(Self {
            journal,
            link,
            delivery,
            hub,
            routing,
            deliveries: tokio::sync::Mutex::new(()),
            observing: tokio::sync::watch::channel(true).0,
        }))
    }
    pub(crate) fn migrate_paired_peer(&self, peer: &str) -> Result<()> {
        let hash = peer
            .strip_prefix("paired-")
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| anyhow!("invalid paired connection identity"))?;
        let legacy = format!("@paired:{hash}");
        if self
            .journal
            .list()
            .iter()
            .any(|record| record.peer == legacy && record.local_session_id.is_some())
        {
            self.journal.change(|rows| {
                for record in rows.values_mut() {
                    if record.peer == legacy && record.local_session_id.is_some() {
                        record.peer = peer.into();
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    pub(crate) fn observer_ready(&self, ready: bool) {
        self.observing.send_replace(ready);
    }
    pub fn records(&self) -> Vec<OriginRecord> {
        let mut rows = self.journal.list();
        rows.sort_by_key(|row| std::cmp::Reverse(row.opened_at));
        rows
    }
    /// Called only at the broker's qualified-call boundary AFTER provenance
    /// sanitization. Source routing is checked before the first peer call;
    /// the peer still applies its own independent execution policy.
    pub(crate) async fn forward_sanitized(
        &self,
        caller: &Caller,
        peer: &str,
        params: Value,
    ) -> Result<Value> {
        self.forward_booked(caller, peer, params, None).await
    }
    pub(crate) async fn forward_booked(
        &self,
        caller: &Caller,
        peer: &str,
        mut params: Value,
        booking: Option<Arc<dyn Booking>>,
    ) -> Result<Value> {
        let mut observing = self.observing.subscribe();
        while !*observing.borrow() {
            observing.changed().await?;
        }
        if caller.federated || !caller.plugin_id.is_empty() {
            bail!("remote worker dispatch is one hop and unavailable to plugins")
        }
        if !self.link.dispatch_enabled(peer) {
            bail!("peer is not enabled for worker dispatch")
        }
        anyhow::ensure!(params.is_object(), "spawn parameters must be an object");
        if let Some(routing) = &self.routing {
            // The origin's policy is independent of the destination's. Apply
            // it before *any* peer call, including the ownerless forwarding
            // path and paired/booked dispatch which bypasses qualified RPC.
            let mut audit = routing.begin_origin_spawn_audit(caller);
            let remote_cwd = params.get("cwd").cloned();
            if let Some(booking) = &booking {
                params["cwd"] = json!(booking.source_root());
            }
            let checked = audit.check(&mut params);
            // Source policy/audit sees the local project for paired work, while
            // the peer must still receive the independently selected remote cwd.
            if let Some(cwd) = remote_cwd {
                params["cwd"] = cwd;
            } else {
                params.as_object_mut().unwrap().remove("cwd");
            }
            let mut scrubbed = checked?;
            if scrubbed
                .iter()
                .any(|field| matches!(field.as_str(), "model" | "provider"))
            {
                if let Some(map) = params.as_object_mut() {
                    for key in ["modelIdentity", "contextWindow"] {
                        if map.remove(key).is_some() {
                            scrubbed.push(key.into());
                        }
                    }
                }
            }
            audit.extend_scrubbed(&scrubbed);
            if !scrubbed.is_empty() {
                params["escalationScrubbed"] = json!(scrubbed);
            }
        }
        let map = params
            .as_object_mut()
            .ok_or_else(|| anyhow!("spawn parameters must be an object"))?;
        map.remove("remoteOrigin");
        let cwd = map
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.trim().is_empty())
            .ok_or_else(|| anyhow!("remote spawn requires an explicit remote cwd"))?
            .to_owned();
        let owner = map
            .get("dispatchOwnerSessionId")
            .and_then(Value::as_str)
            .filter(|owner| !owner.is_empty())
            .map(str::to_owned);
        let Some(owner) = owner else {
            return self.link.call(peer, "agents.spawn", params).await;
        };
        if self.delivery.recipient(&owner).await?.is_none() {
            bail!("remote dispatch requires a live local owner")
        }
        for key in [
            "manager",
            "resumeSessionId",
            "retrySourceSessionId",
            "profileId",
            "mcpItemIds",
            "launchIntegrationId",
            "template",
            "workflowStepId",
        ] {
            if params
                .get(key)
                .is_some_and(|value| !value.is_null() && value != false && value != "")
            {
                bail!("remote dispatch cannot forward local workflow or process configuration")
            }
        }
        let provider = params["provider"]
            .as_str()
            .filter(|p| !p.is_empty())
            .unwrap_or("claude")
            .to_owned();
        let caps: Capabilities = serde_json::from_value(
            self.link
                .call(peer, "fleet.dispatchCapabilities", Value::Null)
                .await?,
        )?;
        if caps.protocol != PROTOCOL || !caps.executes {
            bail!("remote worker dispatch unsupported; upgrade the older endpoint")
        }
        if params["exactModel"] == true && !caps.exact_model {
            bail!("remote endpoint cannot honor exact model selection")
        }
        if !caps.cwds.iter().any(|directory| directory.path == cwd) {
            bail!("choose an actual remote cwd advertised by the peer")
        }
        if !caps
            .providers
            .iter()
            .any(|p| p.provider == provider && p.found && p.authenticated == Some(true))
        {
            bail!("selected provider is not authenticated on the remote host")
        }
        if let Some(schema) = params
            .get("resultSchema")
            .filter(|schema| !schema.is_null())
        {
            crate::services::worker_results::check_schema(schema)
                .map_err(|error| anyhow!(error))?;
        }
        let id = dispatch_id()?;
        let record = OriginRecord {
            dispatch_id: id.clone(),
            peer: peer.into(),
            owner_session_id: owner,
            session_id: None,
            cwd: cwd.clone(),
            provider: provider.clone(),
            model: params["model"].as_str().unwrap_or("").into(),
            label: params["label"].as_str().unwrap_or("").into(),
            local_session_id: booking
                .as_ref()
                .map(|booking| booking.local_session_id().into()),
            opened_at: now(),
            acked_seq: 0,
            delivering_seq: None,
            last_update: None,
            state: "open".into(),
            note: None,
            result_schema: params
                .get("resultSchema")
                .filter(|schema| !schema.is_null())
                .cloned(),
        };
        self.journal.change(|rows| {
            if rows.values().filter(|row| row.state == "open").count() >= 256 {
                bail!("unresolved remote dispatch limit reached; no worker started")
            };
            while rows.len() >= 256 {
                let Some(oldest) = rows
                    .values()
                    .filter(|row| row.state == "done" || row.state == "failed")
                    .min_by_key(|row| row.opened_at)
                    .map(|row| row.dispatch_id.clone())
                else {
                    break;
                };
                rows.remove(&oldest);
            }
            rows.insert(id.clone(), record.clone());
            Ok(())
        })?;
        self.hub
            .publish_wait(Event::new(
                "agent.dispatch.opened",
                "federation",
                serde_json::to_value(&record)?,
            ))
            .await?;
        let result=async{
            let stamp=json!({"protocol":PROTOCOL,"dispatchId":id});
            let requested_worktree=params["worktree"]==true;
            let prepared=self.link.call(peer,"agents.dispatchPrepare",json!({"remoteOrigin":stamp,"cwd":cwd,"provider":provider,"worktree":requested_worktree})).await?;
            let actual=prepared["cwd"].as_str().filter(|cwd|!cwd.is_empty()).ok_or_else(||anyhow!("remote preparation returned no cwd"))?;
            if prepared["repo"]!=cwd||prepared["worktree"]!=requested_worktree||(!requested_worktree&&actual!=cwd){bail!("remote preparation changed the selected repository or isolation request")}
            let mut wire=json!({"remoteOrigin":stamp,"cwd":actual,"provider":provider,"transport":"stream","skipPermissions":false});
            for key in ["message","label","model","modelIdentity","contextWindow","effort","role","capability","decisionId","exactModel","toolScope"]{if let Some(value)=params.get(key){wire[key]=value.clone();}}
            let booked=if let Some(booking)=&booking { let booked=booking.prepare(&record,&prepared).await?;wire["message"]=booked["renderedMessage"].clone();Some(booked) }else{None};
            if let Some(schema)=&record.result_schema {let contract=crate::services::worker_results::result_contract(schema).map_err(|error|anyhow!(error))?;wire["message"]=format!("{}\n\n{contract}",wire["message"].as_str().unwrap_or("")).into();}
            let response=self.link.call(peer,"agents.spawn",wire.clone()).await?;
            let session=response["sessionId"].as_str().filter(|session|!session.is_empty()).ok_or_else(||anyhow!("remote admission uncertain; do not repeat spawn"))?;
            if (booking.is_some()||wire["message"].as_str().is_some_and(|message|!message.trim().is_empty()))&&response["messageQueued"]!=true{bail!("remote first-message delivery uncertain; do not repeat spawn or initial message")}
            self.attach(&id,session)?;
            self.hub.publish_wait(Event::new("agent.dispatch.registered","federation",json!({"dispatchId":id,"peer":peer,"sessionId":session}))).await?;
            let mut response=response;response["remoteDispatchId"]=id.clone().into();response["dispatchId"]=id.clone().into();response["hub"]=peer.into();response["executionCwd"]=actual.into();
            if let Some(booked)=booked {let remote_session=response["sessionId"].clone();response.as_object_mut().unwrap().extend(booked.as_object().unwrap().clone());response["sessionId"]=record.local_session_id.clone().unwrap().into();response["remoteSessionId"]=remote_session;response["executionTarget"]="paired".into();response.as_object_mut().unwrap().remove("hub");}
            Ok(response)
        }.await;
        if let Err(error) = &result {
            self.uncertain(&id, &error.to_string())?;
            let _=self.hub.publish_wait(Event::new("agent.dispatch.failed","federation",json!({"dispatchId":id,"peer":peer,"error":error.to_string(),"outcomeUnknown":true}))).await;
        }
        result
    }
    fn attach(&self, id: &str, session: &str) -> Result<()> {
        self.journal.change(|rows| {
            let record = rows
                .get_mut(id)
                .ok_or_else(|| anyhow!("unknown dispatch"))?;
            if record
                .session_id
                .as_ref()
                .is_some_and(|prior| prior != session)
                || record
                    .last_update
                    .as_ref()
                    .is_some_and(|update| update.session_id != session)
            {
                bail!("remote spawn receipt differs from authenticated callback session")
            };
            record.session_id = Some(session.into());
            Ok(())
        })
    }
    fn uncertain(&self, id: &str, note: &str) -> Result<()> {
        self.journal.change(|rows| {
            if let Some(record) = rows.get_mut(id) {
                if record.state == "open" {
                    record.note = Some(format!(
                        "Admission unknown; do not repeat spawn. {}",
                        note.chars().take(500).collect::<String>()
                    ));
                }
            }
            Ok(())
        })
    }
    pub fn reparent(&self, old: &str, new: &str) -> Result<Vec<String>> {
        if old.is_empty() || new.is_empty() || old == new {
            return Ok(vec![]);
        }
        self.journal.change(|rows| {
            let mut changed = vec![];
            for row in rows.values_mut() {
                if row.state == "open" && row.owner_session_id == old {
                    row.owner_session_id = new.into();
                    changed.push(row.dispatch_id.clone());
                }
            }
            Ok(changed)
        })
    }
    /// Only an envelope stamped by the trusted federation controller can reach
    /// delivery. A peer payload never names its local recipient or filesystem.
    pub async fn update(&self, event: &Event) -> Result<bool> {
        if event.topic != "agent.dispatch.update" || event.hub.is_empty() {
            return Ok(false);
        }
        let update: Update = match event.data.clone().map(serde_json::from_value).transpose() {
            Ok(Some(update)) => update,
            _ => return Ok(false),
        };
        if update.validate().is_err() {
            return Ok(false);
        }
        let _serial = self.deliveries.lock().await;
        self.accept_and_deliver(&event.hub, update).await
    }
    async fn accept_and_deliver(&self, peer: &str, mut update: Update) -> Result<bool> {
        let Some(record) = self.journal.get(&update.dispatch_id) else {
            return Ok(false);
        };
        if record.peer != peer
            || record.state != "open"
            || update.seq <= record.acked_seq
            || record
                .session_id
                .as_ref()
                .is_some_and(|session| session != &update.session_id)
        {
            return Ok(false);
        }
        if let Some(prior) = &record.last_update {
            if prior.session_id != update.session_id
                || update.seq < prior.seq
                || (prior.terminal && update.seq != prior.seq)
            {
                return Ok(false);
            }
            if update.seq == prior.seq {
                update = prior.clone();
            }
        }
        update.entry = sanitize_entry(&update.entry, &update.session_id);
        self.journal.change(|rows| {
            let record = rows.get_mut(&update.dispatch_id).unwrap();
            if record.session_id.is_none() {
                record.session_id = Some(update.session_id.clone());
            }
            if record
                .last_update
                .as_ref()
                .is_none_or(|prior| prior.seq < update.seq)
            {
                record.last_update = Some(update.clone());
            }
            Ok(())
        })?;
        // A prior interrupted send remains unknown even if the peer replays it.
        if record.delivering_seq.is_some() {
            return Ok(false);
        }
        let Some(recipient) = self.delivery.recipient(&record.owner_session_id).await? else {
            return Ok(false);
        };
        self.journal.change(|rows| {
            let record = rows.get_mut(&update.dispatch_id).unwrap();
            record.delivering_seq = Some(update.seq);
            record.note = Some(
                "Wake delivery in progress; interruption is unknown, never resend blindly".into(),
            );
            Ok(())
        })?;
        self.delivery.deliver(&recipient, &record, &update).await?;
        self.acknowledge(&update)?;
        if update.terminal {
            let _ = self
                .link
                .call(
                    peer,
                    "agents.dispatchReplay",
                    json!({"dispatchId":update.dispatch_id,"ackedSeq":update.seq}),
                )
                .await;
        }
        Ok(true)
    }
    fn acknowledge(&self, update: &Update) -> Result<()> {
        self.journal.change(|rows| {
            let record = rows
                .get_mut(&update.dispatch_id)
                .ok_or_else(|| anyhow!("unknown dispatch"))?;
            record.acked_seq = record.acked_seq.max(update.seq);
            record.delivering_seq = None;
            record.note = None;
            if update.terminal {
                record.state = "done".into();
            }
            if record.session_id.is_none() {
                record.session_id = Some(update.session_id.clone());
            }
            Ok(())
        })
    }
    pub async fn reconcile(&self, peer: &str) -> Result<()> {
        for record in self
            .journal
            .list()
            .into_iter()
            .filter(|record| record.peer == peer)
        {
            if record.state == "done" {
                let _ = self
                    .link
                    .call(
                        peer,
                        "agents.dispatchReplay",
                        json!({"dispatchId":record.dispatch_id,"ackedSeq":record.acked_seq}),
                    )
                    .await;
                continue;
            }
            if record.state != "open" {
                continue;
            }
            if let (Some(seq), Some(update)) = (record.delivering_seq, record.last_update.as_ref())
            {
                if update.seq == seq && self.delivery.confirmed(&record, seq).await? {
                    self.acknowledge(update)?;
                    continue;
                }
            }
            if record.delivering_seq.is_none() {
                if let Some(update) = record.last_update.clone() {
                    let _serial = self.deliveries.lock().await;
                    let _ = self.accept_and_deliver(peer, update).await?;
                }
            }
            match self.link.call(peer,"agents.dispatchReplay",json!({"dispatchId":record.dispatch_id})).await{
                Ok(reply) if reply["state"]=="unknown"=>self.journal.change(|rows|{if let Some(current)=rows.get_mut(&record.dispatch_id){if current.state=="open"{current.note=Some(if current.last_update.as_ref().is_some_and(|update|update.terminal){"Remote journal lost; terminal outcome retained locally, wake delivery unconfirmed. Do not repeat spawn or wake."}else{"Remote journal lost; worker outcome unknown. Do not repeat spawn."}.into());}}Ok(())})?,_=>{}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "origin_routing_tests.rs"]
mod routing_tests;

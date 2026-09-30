//! Session service over claudemon's in-process API and typed update stream.
mod params;
use super::{layout::Layout, snapshots};
use crate::{Handle, Options, protocol::Event};
use anyhow::{Result, anyhow, bail};
use claudemon::{
    daemon::embedded::{Command, EmbeddedClient},
    session::store::SessionUpdate,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use tokio::sync::broadcast;

struct Sessions {
    dismissals: super::agent_ops::Dismissals,
    confirmed: Arc<super::live_controls::ConfirmedControls>,
    tokens: Option<std::path::PathBuf>,
    config_dir: Option<std::path::PathBuf>,
    mutations: tokio::sync::Mutex<()>,
    engine: EmbeddedClient,
    rows: Arc<RwLock<BTreeMap<String, Value>>>,
    remote_proxies: Arc<RwLock<BTreeMap<String, Value>>>,
    layout: Option<Arc<Layout>>,
    upstream_layout: Option<Arc<RwLock<Value>>>,
    hub: Handle,
    lifecycle: Option<Arc<super::agent_lifecycle::Lifecycle>>,
    review: Option<Arc<super::fleet_review::ReviewStore>>,
    history: Option<Arc<super::task_store::TaskStore>>,
    replacements: Option<Arc<super::manager_replacements::ReplacementState>>,
    message_tracker: Option<Arc<super::manager_replacements::MessageTracker>>,
    requests: Option<Arc<super::manager_requests::ManagerRequests>>,
    wakes: Option<Arc<super::wakes::Wakes>>,
    workflow_artifacts: Option<Arc<super::workflow_artifacts::Service>>,
}

pub(crate) async fn install(
    mut options: Options,
    hub: Handle,
) -> Result<(Options, Option<tokio::task::JoinHandle<Result<()>>>)> {
    let Some(engine) = options.engine.clone() else {
        return Ok((options, None));
    };
    let updates = engine.subscribe()?;
    let service = Arc::new(Sessions {
        workflow_artifacts: options.workflow_artifacts.clone(),
        dismissals: Default::default(),
        confirmed: options.confirmed_controls.clone(),
        tokens: options.scoped_tokens.clone(),
        config_dir: options.config_dir.clone(),
        mutations: tokio::sync::Mutex::new(()),
        engine,
        rows: options.session_snapshots.clone(),
        remote_proxies: options.remote_proxy_snapshots.clone(),
        layout: options.layout.clone(),
        upstream_layout: options.upstream_layout.clone(),
        hub,
        lifecycle: options.launch_lifecycle.clone(),
        review: options.review_store.clone(),
        replacements: options.replacements.clone(),
        message_tracker: options.message_tracker.clone(),
        wakes: options.wakes.clone(),
        requests: options
            .workflow_runtime
            .as_ref()
            .map(|workflow| workflow.requests.clone()),
        history: options
            .workflow_runtime
            .as_ref()
            .map(|workflow| workflow.tasks.clone()),
    });
    if let Some(lifecycle) = &service.lifecycle {
        lifecycle.reconcile().await?;
    }
    service.seed(false).await?;
    if let Some(artifacts) = &service.workflow_artifacts {
        let weak = Arc::downgrade(&service);
        artifacts.set_sink(Arc::new(move |id, identity, update, events| {
            let weak = weak.clone();
            Box::pin(async move {
                if let Some(service) = weak.upgrade() {
                    service
                        .workflow_update(&id, &identity, &update, events)
                        .await?;
                }
                Ok(())
            })
        }));
    }
    for method in [
        "agents.close",
        "agents.orphans",
        "agents.reparent",
        "agents.list",
        "sessions.snapshots",
        "sessions.snapshot",
        "sessions.transcript",
        "sessions.conversation",
        "sessions.subagentConversation",
        "agents.sendMessage",
        "claude.approve",
        "claude.answer",
        "claude.signal",
        "claude.gate",
    ] {
        let service = service.clone();
        options = options.handler(method, move |_, params| {
            let service = service.clone();
            async move { service.call(method, params).await }
        });
    }
    let task = tokio::spawn(service.observe(updates));
    Ok((options, Some(task)))
}

fn segment(raw: &str) -> Result<&str> {
    if raw.is_empty()
        || raw == "."
        || raw == ".."
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        bail!("invalid session identifier");
    }
    Ok(raw)
}
impl Sessions {
    async fn request(&self, method: &str, path: String, payload: Option<Value>) -> Result<Value> {
        self.engine
            .request(Command::Request {
                method: method.into(),
                path,
                payload,
            })
            .await
    }
    fn generation(&self, id: &str) -> String {
        self.lifecycle
            .as_ref()
            .and_then(|l| l.records().get(id).map(|r| r.generation.clone()))
            .unwrap_or_default()
    }
    fn layout(&self) -> Value {
        if let Some(upstream) = &self.upstream_layout {
            return upstream.read().unwrap().clone();
        }
        self.layout.as_ref().map(|l| l.get()).unwrap_or(Value::Null)
    }
    async fn seed(&self, publish: bool) -> Result<()> {
        let _mutation = self.mutations.lock().await;
        let raw = if publish {
            self.request(
                "GET",
                "/sessions?include_archived=true&include_empty=true".into(),
                None,
            )
            .await?
        } else {
            self.engine.request(Command::Sessions).await?
        };
        let mut known: std::collections::BTreeSet<String> =
            self.rows.read().unwrap().keys().cloned().collect();
        if let Some(lifecycle) = &self.lifecycle {
            // A host-owned launch can finish while both its start and end
            // notifications are lost, before its first projected row exists.
            known.extend(lifecycle.records().into_keys());
        }
        let array = raw
            .as_array()
            .ok_or_else(|| anyhow!("daemon session seed is not an array"))?;
        let mut rows = BTreeMap::new();
        for row in array {
            if let Some(id) = row["session_id"].as_str() {
                // A lag recovery must recover known terminal edges without
                // importing unrelated retained history into the live fleet.
                if publish && !known.contains(id) && super::agent_ops::ended(row) {
                    continue;
                }
                if self.dismissals.allows(id, &self.generation(id)) {
                    let row = snapshots::compat(row.clone());
                    self.confirmed.observe(&row);
                    rows.insert(id.into(), self.confirmed.enrich(row));
                }
            }
        }
        let mut retained: std::collections::BTreeSet<String> = array
            .iter()
            .filter_map(|row| row["session_id"].as_str().map(str::to_owned))
            .collect();
        for row in rows.values() {
            let row = self.enrich(row.clone());
            if !super::agent_ops::ended(&row) {
                if let Some(parent) = row["parentSessionId"].as_str().filter(|s| !s.is_empty()) {
                    retained.insert(parent.into());
                }
            }
        }
        self.dismissals.retain_inventory(&retained);
        self.confirmed.retain(&rows.keys().cloned().collect());
        let old = std::mem::replace(&mut *self.rows.write().unwrap(), rows.clone());
        let mut observations: Vec<_> = rows.values().cloned().collect();
        for (id, row) in &old {
            if !rows.contains_key(id) {
                let mut row = row.clone();
                row["mode"] = json!("stopped");
                row["status"] = json!("ended");
                observations.push(row);
            }
        }
        self.record_history(observations).await;
        if !publish {
            if let Some(wakes) = &self.wakes {
                wakes.prime(
                    &rows
                        .values()
                        .cloned()
                        .map(|row| self.enrich(row))
                        .collect::<Vec<_>>(),
                );
            }
        }
        if publish {
            for (id, mut row) in old {
                if !rows.contains_key(&id) {
                    row["mode"] = json!("stopped");
                    row["status"] = json!("ended");
                    if let Some(wakes) = &self.wakes {
                        wakes.observe(
                            &self.enrich(row.clone()),
                            chrono::Utc::now().timestamp_millis(),
                        );
                    }
                    self.hub
                        .publish_wait(Event::new("agent.snapshot", "brain", self.enrich(row)))
                        .await?;
                }
            }
            for row in rows.values() {
                self.publish(row.clone()).await?;
            }
        }
        Ok(())
    }
    fn enrich(&self, row: Value) -> Value {
        let row = self.confirmed.enrich(row);
        let row = super::snapshots::with_host_metadata(
            row,
            self.lifecycle.as_deref(),
            self.replacements.as_deref(),
        );
        let row = if row.get("label").is_none() {
            let names = self
                .config_dir
                .as_ref()
                .filter(|path| path.is_absolute())
                .map(|path| super::profile_accounts::read(&path.join("tui-names.json")))
                .unwrap_or(Value::Null);
            snapshots::with_cwd_name(row, &names)
        } else {
            row
        };
        match &self.workflow_artifacts {
            Some(service) => service.enrich(row),
            None => row,
        }
    }
    fn sender_label(&self, id: &str) -> Option<String> {
        // Attribution names the recorded agent, never a cwd display rename.
        let row = snapshots::with_host_metadata(
            json!({"sessionId":id}),
            self.lifecycle.as_deref(),
            self.replacements.as_deref(),
        );
        row["label"]
            .as_str()
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
    }
    async fn workflow_update(
        &self,
        id: &str,
        identity: &Value,
        update: &Value,
        events: Vec<Value>,
    ) -> Result<()> {
        let _mutation = self.mutations.lock().await;
        let Some(artifacts) = &self.workflow_artifacts else {
            return Ok(());
        };
        if !self.dismissals.allows(id, &self.generation(id)) {
            return Ok(());
        }
        let next = {
            let mut rows = self.rows.write().unwrap();
            let Some(row) = rows.get_mut(id) else {
                return Ok(());
            };
            if !row["hub"].as_str().unwrap_or("").is_empty() || artifacts.identity(row) != *identity
            {
                return Ok(());
            }
            // Merge into the current row under the shared write lock, so other
            // observers' controls/status/usage cannot be overwritten by a scan.
            *row = super::workflow_artifacts::merge(row.clone(), update);
            row.clone()
        };
        self.publish(next).await?;
        for event in events {
            let Some(topic) = event["type"].as_str() else {
                continue;
            };
            self.hub
                .publish_wait(Event::new(topic, "brain", event["data"].clone()))
                .await?;
        }
        Ok(())
    }
    async fn record_history(&self, rows: Vec<Value>) {
        let Some(history) = self.history.clone() else {
            return;
        };
        let rows: Vec<_> = rows.into_iter().map(|row| self.enrich(row)).collect();
        match tokio::task::spawn_blocking(move || history.observe_batch(&rows)).await {
            Ok(Ok(())) => (),
            Ok(Err(error)) => eprintln!("task history observation was not persisted: {error}"),
            Err(error) => eprintln!("task history observation task failed: {error}"),
        }
    }
    async fn publish(&self, row: Value) -> Result<()> {
        let row = self.enrich(row);
        if let Some(wakes) = &self.wakes {
            wakes.observe(&row, chrono::Utc::now().timestamp_millis());
        }
        if snapshots::visible(&row, &self.layout(), time::OffsetDateTime::now_utc()) {
            self.hub
                .publish_wait(Event::new("agent.snapshot", "brain", row))
                .await?;
        }
        Ok(())
    }
    async fn observe(
        self: Arc<Self>,
        mut updates: broadcast::Receiver<SessionUpdate>,
    ) -> Result<()> {
        let mut status = self.engine.status();
        let mut lifecycle_retry = tokio::time::interval(std::time::Duration::from_secs(5));
        lifecycle_retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = lifecycle_retry.tick(), if self.lifecycle.is_some() || self.history.is_some() => {
                    if let Some(lifecycle) = &self.lifecycle {
                        if let Err(error) = lifecycle.reconcile().await {
                            eprintln!("session credential reconciliation failed; will retry: {error}");
                        }
                    }
                    let rows=self.rows.read().unwrap().values().cloned().collect();
                    self.record_history(rows).await;
                }
                changed=status.changed()=>{
                    if changed.is_err() || !matches!(*status.borrow(),claudemon::daemon::embedded::Status::Ready(_)){bail!("embedded session engine stopped");}
                }
                update=updates.recv()=>match update {
                    Ok(update)=>{
                        if update.event == "SessionStart" { self.confirmed.forget(&update.session_id); }
                        let path=format!("/sessions/{}",segment(&update.session_id)?);
                        match self.request("GET",path,None).await {
                            Ok(row)=>{
                                if row["mode"] == "stopped" {
                                    if let Some(lifecycle) = &self.lifecycle {
                                        if let Err(error) = lifecycle.reconcile().await {
                                            eprintln!("session credential revocation failed; will retry: {error}");
                                        }
                                    }
                                }
                                let _mutation = self.mutations.lock().await;
                                if !self.dismissals.allows(&update.session_id, &self.generation(&update.session_id)) { continue; }
                                let row=snapshots::compat(row);self.confirmed.observe(&row);let row=self.confirmed.enrich(row);self.rows.write().unwrap().insert(update.session_id,row.clone());self.record_history(vec![row.clone()]).await;self.publish(row).await?;}
                            Err(error)=>{eprintln!("session projection refresh failed: {error}");self.seed(true).await?;}
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_))=>self.seed(true).await?,
                    Err(broadcast::error::RecvError::Closed)=>bail!("embedded session update stream closed"),
                }
            }
        }
    }
    async fn close_agent(&self, params: Value) -> Result<Value> {
        let id = segment(params["sessionId"].as_str().unwrap_or(""))?;
        let _mutation = self.mutations.lock().await;
        let _lineage = self
            .replacements
            .as_ref()
            .map(|s| s.manual_admission(&[id]))
            .transpose()?;
        let _launch = match &self.lifecycle {
            Some(l) => Some(l.mutation_guard().await),
            None => None,
        };
        let generation = self.generation(id);
        let known = self.rows.read().unwrap().get(id).cloned();
        let removed = known.is_some();
        if known.is_none()
            && self.lifecycle.as_ref().is_some_and(|l| {
                l.records().get(id).is_some_and(|r| {
                    matches!(
                        r.phase,
                        super::agent_lifecycle::Phase::Preparing
                            | super::agent_lifecycle::Phase::Running
                    )
                })
            })
            && self.dismissals.allows(id, &generation)
        {
            bail!(
                "session launch has not reached the projection; retry close after its snapshot arrives"
            );
        }
        let mut daemon = "already-ended";
        let mut was_live = false;
        let mut label = None;
        if let Some(known) = known {
            let row = self.enrich(snapshots::compat(
                self.request("GET", format!("/sessions/{id}"), None).await?,
            ));
            super::agent_ops::validate_close(&row)?;
            was_live = !super::agent_ops::ended(&row);
            label = row.get("label").cloned();
            self.dismissals
                .dismiss(id, generation.clone(), self.enrich(known));
            self.rows.write().unwrap().remove(id);
            self.confirmed.forget(id);
            if let Some(wakes) = &self.wakes {
                wakes.forget(id);
            }
            if was_live {
                daemon = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.request(
                        "POST",
                        format!("/sessions/{id}/signal"),
                        Some(json!({"signal":"SIGTERM"})),
                    ),
                )
                .await
                {
                    Ok(Ok(_)) => "stopped",
                    _ => "failed",
                };
            }
        }
        if let Some(tokens) = &self.tokens {
            let tokens = tokens.clone();
            let label = format!("session:{id}");
            tokio::task::spawn_blocking(move || {
                crate::auth::update_records(&tokens, |records| {
                    records.retain(|record| {
                        record.label != label
                            || (!generation.is_empty()
                                && record
                                    .metadata
                                    .get("generation")
                                    .is_some_and(|g| g != &Value::String(generation.clone())))
                    });
                    Ok(())
                })
            })
            .await
            .map_err(|e| anyhow!("session closed but token cleanup failed: {e}"))??;
        }
        let mut result = json!({"ok":true,"removed":removed,"wasLive":was_live,"daemon":daemon,"note":"The session is gone from list_agents. Its desktop pane remains the user's to close."});
        if let Some(label) = label {
            result["label"] = label;
        }
        Ok(result)
    }
    async fn reparent(&self, params: Value) -> Result<Value> {
        let from = segment(params["fromSessionId"].as_str().unwrap_or(""))?;
        let to = segment(params["toSessionId"].as_str().unwrap_or(""))?;
        if from == to {
            bail!("successor is already the parent");
        }
        let _mutation = self.mutations.lock().await;
        let _lineage = self
            .replacements
            .as_ref()
            .map(|s| s.manual_admission(&[from, to]))
            .transpose()?;
        let _known_successor = self
            .rows
            .read()
            .unwrap()
            .get(to)
            .cloned()
            .map(|r| self.enrich(r))
            .ok_or_else(|| anyhow!("successor is not a known live manager"))?;
        let successor = self.enrich(snapshots::compat(
            self.request("GET", format!("/sessions/{to}"), None).await?,
        ));
        if super::agent_ops::ended(&successor)
            || successor["isWakeTarget"] != true
            || successor["hub"].as_str().is_some_and(|h| !h.is_empty())
        {
            bail!("successor must be a live local manager");
        }
        let lifecycle = self
            .lifecycle
            .as_ref()
            .ok_or_else(|| anyhow!("launch metadata unavailable"))?;
        if let Some(history) = &self.history {
            let history = history.clone();
            let from = from.to_owned();
            let to = to.to_owned();
            tokio::task::spawn_blocking(move || history.adopt(&from, &to)).await??;
        }
        if let Some(review) = self.review.clone() {
            let from = from.to_owned();
            let to = to.to_owned();
            tokio::task::spawn_blocking(move || review.adopt_owner(&from, &to)).await??;
        }
        let live: std::collections::BTreeSet<String> = self
            .rows
            .read()
            .unwrap()
            .iter()
            .filter(|(_, r)| !super::agent_ops::ended(r))
            .map(|(id, _)| id.clone())
            .collect();
        let mut planned: Vec<String> = lifecycle
            .records()
            .into_iter()
            .filter(|(id, r)| {
                id != to
                    && r.metadata["parentSessionId"] == from
                    && (live.contains(id) || r.phase == super::agent_lifecycle::Phase::Preparing)
            })
            .map(|(id, _)| id)
            .collect();
        for (id, row) in self.rows.read().unwrap().iter() {
            if id != to
                && row["parentSessionId"] == from
                && !super::agent_ops::ended(row)
                && row["hub"].as_str().is_none_or(str::is_empty)
                && !planned.contains(id)
            {
                planned.push(id.clone());
            }
        }
        // Commit the overlay before awaiting lifecycle persistence: after task
        // adoption, any interrupted metadata write must converge on the new owner.
        if let Some(state) = &self.replacements {
            state.note_manual_reparent(from, to, &planned, successor)?;
        }
        let mut changed = lifecycle.reparent_current_children(from, to, &live).await?;
        let mut moved = Vec::new();
        {
            let mut rows = self.rows.write().unwrap();
            for (id, row) in rows.iter_mut() {
                if id != to
                    && row["hub"].as_str().is_none_or(str::is_empty)
                    && (changed.contains(id) || row["parentSessionId"] == from)
                    && !super::agent_ops::ended(row)
                {
                    row["parentSessionId"] = json!(to);
                    moved.push(id.clone());
                    if !changed.contains(id) {
                        changed.push(id.clone());
                    }
                }
            }
        }
        let pending: Vec<_> = changed
            .into_iter()
            .filter(|id| !moved.contains(id))
            .collect();
        Ok(
            json!({"moved":moved,"pending":pending,"note":"Task ownership and local worker parentage transferred to the successor."}),
        )
    }
    async fn call(&self, method: &str, mut params: Value) -> Result<Value> {
        params::validate(method, &mut params)?;
        if method == "agents.orphans" {
            let mut rows: Vec<_> = self
                .rows
                .read()
                .unwrap()
                .values()
                .cloned()
                .map(|r| self.enrich(r))
                .collect();
            rows.extend(self.dismissals.tombstones());
            return Ok(super::agent_ops::orphans(&rows));
        }
        if method == "agents.close" {
            return self.close_agent(params).await;
        }
        if method == "agents.reparent" {
            return self.reparent(params).await;
        }

        if matches!(method, "agents.list" | "sessions.snapshots") {
            let layout = self.layout();
            let now = time::OffsetDateTime::now_utc();
            let mut rows: Vec<Value> = self
                .rows
                .read()
                .unwrap()
                .values()
                .filter(|row| snapshots::visible(row, &layout, now))
                .cloned()
                .map(|row| self.enrich(row))
                .collect();
            rows.extend(self.remote_proxies.read().unwrap().values().cloned());
            return Ok(Value::Array(rows));
        }
        if method == "sessions.snapshot" {
            if let Some(row) = self
                .remote_proxies
                .read()
                .unwrap()
                .get(params["sessionId"].as_str().unwrap_or(""))
                .cloned()
            {
                return Ok(row);
            }
        }
        let id = segment(params["sessionId"].as_str().unwrap_or(""))?;
        let root = format!("/sessions/{id}");
        match method {
            "sessions.snapshot" => {
                if let Some(row) = self.remote_proxies.read().unwrap().get(id).cloned() {
                    return Ok(row);
                }
                if let Some(row) = self.rows.read().unwrap().get(id).cloned() {
                    return Ok(self.enrich(row));
                }
                Ok(self.enrich(snapshots::compat(self.request("GET", root, None).await?)))
            }
            "sessions.transcript" => {
                let mut path = format!("{root}/transcript");
                if let Some(cwd) = params["cwd"].as_str().filter(|s| !s.is_empty()) {
                    path.push('?');
                    path.push_str(
                        &url::form_urlencoded::Serializer::new(String::new())
                            .append_pair("cwd", cwd)
                            .finish(),
                    );
                }
                self.request("GET", path, None).await
            }
            "sessions.conversation" => {
                let mut path = format!("{root}/conversation");
                if let Some(since) = params["sinceSeq"].as_i64() {
                    path.push_str(&format!("?since={since}"));
                }
                self.request("GET", path, None).await
            }
            "sessions.subagentConversation" => {
                self.request(
                    "GET",
                    format!(
                        "{root}/subagents/{}/conversation",
                        segment(params["agentId"].as_str().unwrap_or(""))?
                    ),
                    None,
                )
                .await
            }
            "agents.sendMessage" => {
                let text = params["text"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("agents.sendMessage requires {{ sessionId, text }}"))?;
                let mut text = text.to_owned();
                if let Some(from) = params["fromSessionId"].as_str().filter(|s| !s.is_empty()) {
                    let label = self.sender_label(from);
                    text = format!(
                        "{}{text}",
                        super::fleet_messages::sender_header(from, label.as_deref().unwrap_or(""))
                    );
                }
                if let Some(tracker) = &self.message_tracker {
                    let target = match &self.replacements {
                        Some(state) => state.wake_target(id)?,
                        None => id.into(),
                    };
                    let delivery = super::manager_replacements::messages::send_engine(
                        &self.engine,
                        tracker,
                        &target,
                        &text,
                        &[],
                        None,
                        false,
                        self.requests.as_deref(),
                    )
                    .await?;
                    return match delivery.outcome {
                        super::manager_replacements::SendOutcome::Accepted => Ok(delivery.wire()),
                        super::manager_replacements::SendOutcome::Rejected { reason, .. } => {
                            Err(anyhow!("message delivery rejected: {reason}"))
                        }
                        super::manager_replacements::SendOutcome::Uncertain(reason) => {
                            Err(anyhow!(
                                "message delivery outcome is unknown; do not retry automatically: {reason}"
                            ))
                        }
                    };
                }
                self.request(
                    "POST",
                    format!("{root}/message"),
                    Some(json!({"text":text})),
                )
                .await?;
                Ok(json!({"ok":true}))
            }
            "claude.approve" => {
                let decision = params["decision"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        anyhow!(
                            "claude.approve requires {{ sessionId, decision: 'yes'|'no'|'always' }}"
                        )
                    })?;
                let mut payload = json!({"decision":decision});
                if let Some(reason) = params["reason"]
                    .as_str()
                    .filter(|reason| !reason.is_empty())
                {
                    payload["reason"] = reason.into();
                }
                self.request("POST", format!("{root}/approve"), Some(payload))
                    .await?;
                Ok(json!({"ok":true}))
            }
            "claude.signal" => {
                let signal = params["signal"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("claude.signal requires {{ sessionId, signal }}"))?;
                self.request(
                    "POST",
                    format!("{root}/signal"),
                    Some(json!({"signal":signal})),
                )
                .await?;
                Ok(json!({"ok":true}))
            }
            "claude.gate" => {
                self.request(
                    "POST",
                    format!("{root}/gate"),
                    Some(json!({"on":params["on"].as_bool().unwrap_or(false)})),
                )
                .await
            }
            "claude.answer" => {
                let option = params["option"].as_i64();
                let text = params["text"].as_str();
                let answers = params["answers"].as_array();
                if option.is_none() && text.is_none() && answers.is_none_or(Vec::is_empty) {
                    bail!("claude.answer requires one of {{ option, text, answers }}");
                }
                let row = self.request("GET", root.clone(), None).await?;
                if row["transport"] == "stream" {
                    let mut payload = params.clone();
                    payload.as_object_mut().unwrap().remove("sessionId");
                    self.request("POST", format!("{root}/answer"), Some(payload))
                        .await?;
                } else {
                    let texts = if let Some(option) = option {
                        vec![option.to_string()]
                    } else if let Some(text) = text {
                        vec![text.into()]
                    } else {
                        answers
                            .unwrap()
                            .iter()
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_owned)
                                    .ok_or_else(|| anyhow!("answer must be text"))
                            })
                            .collect::<Result<Vec<_>>>()?
                    };
                    for text in texts {
                        self.request(
                            "POST",
                            format!("{root}/input"),
                            Some(json!({"text":format!("{text}\r"), "newline": false})),
                        )
                        .await?;
                    }
                }
                Ok(json!({"ok":true}))
            }
            _ => bail!("unknown session method"),
        }
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    struct NoPreparation;
    impl super::super::agent_lifecycle::LaunchPreparation for NoPreparation {
        fn prepare<'a>(
            &'a self,
            _: &'a mut super::super::spawn_plan::Plan,
            _: &'a str,
        ) -> super::super::agent_lifecycle::Operation<'a, ()> {
            Box::pin(async { bail!("test does not launch providers") })
        }
        fn revoke<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> super::super::agent_lifecycle::Operation<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }
    #[tokio::test]
    async fn typed_lag_reloads_canonical_terminal_state_without_importing_history() -> Result<()> {
        use claudemon::daemon::{
            ServeConfig,
            embedded::{EmbeddedDaemon, Options as EngineOptions},
        };
        let _single_engine = crate::backend::ENGINE_TEST_LOCK.lock().await;
        let dir = tempfile::tempdir()?;
        let mut engine = EmbeddedDaemon::start_with_options(
            ServeConfig {
                host: "127.0.0.1".into(),
                hook_port: 0,
                api_port: 0,
                db_path: dir.path().join("state.db"),
            },
            EngineOptions {
                usage_poll_on_boot: Some(false),
            },
        )?;
        let ready = engine.ready().await?;
        let client = engine.client();
        let mut actual_updates = client.subscribe()?;
        let hub = crate::Hub::start(Options::default())?;
        hub.ready().await?;
        let viewer = crate::client::Client::connect(&hub.handle()).await?;
        let mut events = viewer.events();
        viewer.topics(["agent.snapshot".into()].into()).await?;
        let rows = Arc::new(RwLock::new(BTreeMap::new()));
        let journal = dir.path().join("launches.json");
        std::fs::write(
            &journal,
            serde_json::to_vec(&json!({"owned-fast":{
                "generation":"owned-life", "provider":"claude", "cwd":dir.path(),
                "metadata":{"label":"Owned fast worker"}, "phase":"running",
                "revocationPending":false, "receipt":null
            }}))?,
        )?;
        let lifecycle = super::super::agent_lifecycle::Lifecycle::open(
            journal,
            Arc::new(client.clone()),
            Arc::new(NoPreparation),
        )?;
        let service = Arc::new(Sessions {
            dismissals: Default::default(),
            confirmed: Default::default(),
            tokens: None,
            config_dir: None,
            mutations: tokio::sync::Mutex::new(()),
            engine: client.clone(),
            rows: rows.clone(),
            remote_proxies: Default::default(),
            layout: None,
            upstream_layout: None,
            hub: hub.handle(),
            lifecycle: Some(lifecycle),
            review: None,
            history: None,
            replacements: None,
            message_tracker: None,
            requests: None,
            wakes: None,
            workflow_artifacts: None,
        });
        let http = reqwest::Client::new();
        for (id, event) in [("worker", "SessionStart"), ("worker", "UserPromptSubmit")] {
            http.post(format!("http://{}/hook", ready.hook_addr))
                .json(&json!({"hook_event_name":event,"session_id":id,"cwd":dir.path()}))
                .send()
                .await?
                .error_for_status()?;
        }
        service.seed(false).await?;
        assert!(rows.read().unwrap().contains_key("worker"));
        assert!(events.try_recv().is_err(), "initial seed must be silent");
        for (id, event) in [
            ("worker", "SessionEnd"),
            ("old-history", "SessionStart"),
            ("old-history", "SessionEnd"),
            ("owned-fast", "SessionStart"),
            ("owned-fast", "SessionEnd"),
        ] {
            http.post(format!("http://{}/hook", ready.hook_addr))
                .json(&json!({"hook_event_name":event,"session_id":id,"cwd":dir.path()}))
                .send()
                .await?
                .error_for_status()?;
        }
        let raw = service
            .request(
                "GET",
                "/sessions?include_archived=true&include_empty=true".into(),
                None,
            )
            .await?;
        assert!(
            raw.as_array()
                .unwrap()
                .iter()
                .any(|r| r["session_id"] == "old-history" && r["mode"] == "stopped")
        );
        // A bounded typed receiver has genuinely lost an edge. Its final queued
        // frame is deliberately stale: the observer must GET authoritative state.
        let stale = tokio::time::timeout(std::time::Duration::from_secs(2), actual_updates.recv())
            .await??;
        let (updates, receiver) = broadcast::channel(1);
        for _ in 0..4 {
            updates.send(stale.clone())?;
        }
        let observer = tokio::spawn(service.clone().observe(receiver));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let event = events.recv().await?;
                if event
                    .data
                    .as_ref()
                    .is_some_and(|r| r["sessionId"] == "worker" && r["status"] == "ended")
                {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await??;
        assert!(!rows.read().unwrap().contains_key("old-history"));
        assert_eq!(rows.read().unwrap()["worker"]["status"], "ended");
        assert_eq!(
            rows.read().unwrap()["owned-fast"]["status"],
            "ended",
            "owned launch cannot disappear when both lifecycle edges were lost"
        );
        observer.abort();
        let _ = observer.await;
        hub.shutdown()?;
        engine.shutdown().await?;
        Ok(())
    }
}

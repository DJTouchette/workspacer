//! Owned finish/block wake scheduling over the shared engine projection.
use super::{
    fleet_messages,
    manager_replacements::{ReplacementState, native::WakeControl},
    task_store::{OwnerLookup, TaskStore},
    worker_results,
};
use anyhow::{Result, anyhow};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
pub type Capture = Arc<dyn Fn(String) -> BoxFuture<'static, Result<Value>> + Send + Sync>;
pub type Delivery = Arc<
    dyn Fn(String, String, Vec<(String, String)>) -> BoxFuture<'static, Result<()>> + Send + Sync,
>;
pub type Inventory = Arc<dyn Fn() -> Vec<Value> + Send + Sync>;
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn ended(row: &Value) -> bool {
    row["status"] == "ended" || row["mode"] == "stopped"
}
fn idle(row: &Value) -> bool {
    ended(row) || matches!(text(row, "ambientState"), "" | "idle")
}
fn working(mode: &str) -> bool {
    matches!(mode, "thinking" | "streaming" | "background")
}
fn blocked(mode: &str) -> bool {
    matches!(mode, "waiting_approval" | "waiting_input")
}
fn label(row: &Value) -> String {
    if !text(row, "label").is_empty() {
        return text(row, "label").into();
    }
    let cwd = text(row, "cwd");
    if cwd.is_empty() {
        return "Agent".into();
    }
    cwd.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(cwd)
        .into()
}
#[derive(Clone)]
struct Pending {
    at: i64,
    workers: Vec<(String, u64)>,
}
#[derive(Clone)]
struct Block {
    at: i64,
    generation: u64,
}
#[derive(Clone)]
struct Action {
    remote: bool,
    kind: String,
    parent: String,
    workers: Vec<(String, u64)>,
}
#[derive(Default)]
struct State {
    previous: BTreeMap<String, String>,
    generations: BTreeMap<String, u64>,
    sequence: u64,
    groups: BTreeMap<(String, String, bool), Pending>,
    blocks: BTreeMap<String, Block>,
    signatures: BTreeMap<String, String>,
    finishes: BTreeMap<String, Value>,
}
pub struct Wakes {
    remote: Option<Arc<dyn super::remote_dispatch::return_channel::ReturnChannel>>,
    lookup: OwnerLookup,
    list: Inventory,
    capture: Capture,
    review: Mutex<Option<Arc<super::fleet_review::ReviewStore>>>,
    delivery: Delivery,
    history: Option<Arc<TaskStore>>,
    replacements: Option<Arc<ReplacementState>>,
    state: Mutex<State>,
    serial: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    stop: tokio::sync::watch::Sender<bool>,
}
fn schedule(
    state: &mut State,
    kind: &str,
    parent: &str,
    id: &str,
    generation: u64,
    now: i64,
    delay: i64,
) {
    schedule_for(state, kind, parent, id, generation, now, delay, false);
}
fn schedule_for(
    state: &mut State,
    kind: &str,
    parent: &str,
    id: &str,
    generation: u64,
    now: i64,
    delay: i64,
    remote: bool,
) {
    let pending = state
        .groups
        .entry((kind.into(), parent.into(), remote))
        .or_insert_with(|| Pending {
            at: now + delay,
            workers: Vec::new(),
        });
    if let Some(existing) = pending.workers.iter_mut().find(|(worker, _)| worker == id) {
        existing.1 = generation;
    } else {
        pending.workers.push((id.into(), generation));
    }
}
impl Wakes {
    #[cfg(feature = "test-support")]
    pub(crate) fn fixture_remote(
        mut service: Arc<Self>,
        receiver: Arc<super::remote_dispatch::Receiver>,
    ) -> Arc<Self> {
        Arc::get_mut(&mut service)
            .expect("fresh fixture wake observer")
            .remote = Some(receiver);
        service
    }
    pub fn new(
        lookup: OwnerLookup,
        list: Inventory,
        capture: Capture,
        delivery: Delivery,
        history: Option<Arc<TaskStore>>,
        replacements: Option<Arc<ReplacementState>>,
    ) -> Arc<Self> {
        let (stop, _) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            remote: None,
            lookup,
            list,
            capture,
            review: Mutex::new(None),
            delivery,
            history,
            replacements,
            state: Mutex::new(State::default()),
            serial: Mutex::new(BTreeMap::new()),
            stop,
        })
    }
    pub fn set_review_store(&self, review: Arc<super::fleet_review::ReviewStore>) {
        *self.review.lock().unwrap() = Some(review);
    }
    pub fn forget(&self, id: &str) {
        let mut state = self.state.lock().unwrap();
        state.previous.remove(id);
        state.generations.remove(id);
        state.blocks.remove(id);
        state.signatures.remove(id);
        state.finishes.remove(id);
        for pending in state.groups.values_mut() {
            pending.workers.retain(|(worker, _)| worker != id);
        }
        state
            .groups
            .retain(|(_, parent, _), pending| parent != id && !pending.workers.is_empty());
    }
    pub fn prime(&self, rows: &[Value]) {
        let mut state = self.state.lock().unwrap();
        for row in rows {
            let id = text(row, "sessionId");
            if !id.is_empty() {
                state
                    .previous
                    .insert(id.into(), text(row, "ambientState").into());
            }
        }
    }
    fn is_remote(&self, worker: &str) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|receiver| receiver.owns(worker))
    }
    fn target(&self, parent: &str, worker: &str) -> Result<String> {
        if self.is_remote(worker) {
            return Ok(parent.into());
        }
        match &self.replacements {
            Some(state) => state.worker_wake_target(parent, worker),
            None => Ok(parent.into()),
        }
    }
    fn available(&self, parent: &str) -> bool {
        self.replacements
            .as_ref()
            .is_some_and(|state| state.held(parent).is_some())
            || (self.lookup)(parent).is_some_and(|row| !ended(&row))
    }
    fn block_targets(&self, row: &Value) -> Vec<(String, bool)> {
        let id = text(row, "sessionId");
        let mode = text(row, "ambientState");
        let parent = self
            .target(text(row, "parentSessionId"), id)
            .unwrap_or_default();
        let remote = self.is_remote(id);
        let parent_available =
            remote || (!parent.is_empty() && parent != id && self.available(&parent));
        let mut targets = BTreeSet::new();
        if blocked(mode) {
            for manager in (self.list)() {
                if manager["isWakeTarget"] == true
                    && !ended(&manager)
                    && text(&manager, "sessionId") != id
                {
                    let target = self
                        .replacements
                        .as_ref()
                        .map(|state| state.wake_target(text(&manager, "sessionId")))
                        .unwrap_or_else(|| Ok(text(&manager, "sessionId").into()));
                    if let Ok(target) = target {
                        if self.available(&target) {
                            targets.insert((target, false));
                        }
                    }
                }
            }
            if parent_available {
                targets.insert((parent.clone(), remote));
            }
        }
        targets.into_iter().collect()
    }
    pub fn observe(&self, row: &Value, now: i64) {
        let id = text(row, "sessionId");
        if id.is_empty() || !text(row, "hub").is_empty() {
            return;
        }
        let mode = text(row, "ambientState");
        let parent = self
            .target(text(row, "parentSessionId"), id)
            .unwrap_or_default();
        let remote = self.is_remote(id);
        let parent_available =
            remote || (!parent.is_empty() && parent != id && self.available(&parent));
        let targets = if blocked(mode) {
            self.block_targets(row)
        } else {
            Vec::new()
        };
        let mut state = self.state.lock().unwrap();
        let previous = state.previous.insert(id.into(), mode.into());
        let continuous_block = previous.as_deref().is_some_and(blocked) && blocked(mode);
        if previous.as_deref() != Some(mode) && !continuous_block {
            state.sequence += 1;
            let generation = state.sequence;
            state.generations.insert(id.into(), generation);
            state.blocks.remove(id);
            for ((kind, _, _), group) in &mut state.groups {
                if kind == "blocked" {
                    group.workers.retain(|(worker, _)| worker != id);
                }
            }
            state.groups.retain(|_, group| !group.workers.is_empty());
            if blocked(mode) && !targets.is_empty() {
                state.blocks.insert(
                    id.into(),
                    Block {
                        at: now + 20000,
                        generation,
                    },
                );
            }
        }
        if previous.as_deref().is_some_and(working) && mode == "idle" && parent_available {
            schedule_for(
                &mut state,
                "worker-finished",
                &parent,
                id,
                0,
                now,
                1500,
                remote,
            );
        }
        if working(mode) {
            state.finishes.remove(id);
        }
    }
    fn due(&self, now: i64, limit: usize) -> Vec<Action> {
        // Look up current recipients outside the scheduler lock. Go's blocked
        // broadcast includes managers appearing during the survival window;
        // the initial recipient list is only a cheap admission check.
        let due_blocks: Vec<_> = {
            let state = self.state.lock().unwrap();
            state
                .blocks
                .iter()
                .filter(|(_, block)| block.at <= now)
                .map(|(id, block)| (id.clone(), block.clone()))
                .collect()
        };
        let recipients: Vec<_> = due_blocks
            .into_iter()
            .map(|(id, block)| {
                let targets = (self.lookup)(&id)
                    .filter(|row| !ended(row) && blocked(text(row, "ambientState")))
                    .map(|row| self.block_targets(&row))
                    .unwrap_or_default();
                (id, block, targets)
            })
            .collect();
        let mut state = self.state.lock().unwrap();
        for (id, block, targets) in recipients {
            // A clear/re-block during lookup invalidates this timer generation.
            if state
                .blocks
                .get(&id)
                .is_none_or(|current| current.generation != block.generation)
            {
                continue;
            }
            state.blocks.remove(&id);
            for (parent, remote) in targets {
                schedule_for(
                    &mut state,
                    "blocked",
                    &parent,
                    &id,
                    block.generation,
                    now,
                    1500,
                    remote,
                );
            }
        }
        let keys: Vec<_> = state
            .groups
            .iter()
            .filter(|(_, group)| group.at <= now)
            .take(limit)
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .map(|(kind, parent, remote)| {
                let pending = state
                    .groups
                    .remove(&(kind.clone(), parent.clone(), remote))
                    .unwrap();
                Action {
                    remote,
                    kind,
                    parent,
                    workers: pending.workers,
                }
            })
            .collect()
    }
    pub fn backstop(&self, now: i64) {
        let rows = (self.list)();
        let mut state = self.state.lock().unwrap();
        for child in &rows {
            if self.is_remote(text(child, "sessionId")) && idle(child) {
                let at = child["lastActivity"].as_i64().unwrap_or(0);
                if at > 0 && now.saturating_sub(at) > 180000 {
                    schedule_for(
                        &mut state,
                        "worker-finished",
                        text(child, "parentSessionId"),
                        text(child, "sessionId"),
                        0,
                        now,
                        0,
                        true,
                    );
                }
            }
        }
        for parent in &rows {
            if ended(parent) || parent["ambientState"] != "idle" {
                continue;
            }
            for child in &rows {
                if child["parentSessionId"] != parent["sessionId"]
                    || child["sessionId"] == parent["sessionId"]
                    || self.is_remote(text(child, "sessionId"))
                    || !idle(child)
                {
                    continue;
                }
                let at = child["lastActivity"].as_i64().unwrap_or(0);
                let parent_at = parent["lastActivity"].as_i64().unwrap_or(0);
                if at > parent_at && at > 0 && now.saturating_sub(at) > 180000 {
                    schedule(
                        &mut state,
                        "catch-up",
                        text(parent, "sessionId"),
                        text(child, "sessionId"),
                        0,
                        now,
                        0,
                    );
                }
            }
        }
    }
    async fn deliver_action(&self, action: Action) -> Result<()> {
        let serial = self
            .serial
            .lock()
            .unwrap()
            .entry(action.parent.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _serial = serial.lock().await;
        if !action.remote && !self.available(&action.parent) {
            return Ok(());
        }
        let parent = (self.lookup)(&action.parent);
        let ordinary = !action.remote
            && parent
                .as_ref()
                .is_none_or(|row| row["isWakeTarget"] != true);
        let mut completed = Vec::new();
        let mut escalated = Vec::new();
        for (id, generation) in action.workers {
            let Some(row) = (self.lookup)(&id) else {
                continue;
            };
            if action.kind == "blocked" {
                if ended(&row)
                    || !blocked(text(&row, "ambientState"))
                    || self.state.lock().unwrap().generations.get(&id) != Some(&generation)
                {
                    continue;
                }
                if ordinary && self.target(text(&row, "parentSessionId"), &id)? != action.parent {
                    continue;
                }
                completed.push((json!({"label":label(&row),"sessionId":id,"_wakeGeneration":generation,"blockedOn":if row["ambientState"]=="waiting_approval"{"approval"}else{"question"}}),String::new(),String::new()));
                continue;
            }
            if !idle(&row) {
                continue;
            }
            let target = self.target(text(&row, "parentSessionId"), &id)?;
            if target != action.parent {
                if !target.is_empty() {
                    schedule_for(
                        &mut self.state.lock().unwrap(),
                        &action.kind,
                        &target,
                        &id,
                        0,
                        chrono::Utc::now().timestamp_millis(),
                        1500,
                        action.remote,
                    );
                }
                continue;
            }
            let (has_task, reply) = final_turn(
                tokio::time::timeout(Duration::from_secs(10), (self.capture)(id.clone()))
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result),
            );
            if !has_task {
                continue;
            }
            let Some(current) = (self.lookup)(&id) else {
                continue;
            };
            if !idle(&current) {
                continue;
            }
            if current["lastActivity"] != row["lastActivity"]
                || self.target(text(&current, "parentSessionId"), &id)? != action.parent
            {
                schedule_for(
                    &mut self.state.lock().unwrap(),
                    &action.kind,
                    &action.parent,
                    &id,
                    0,
                    chrono::Utc::now().timestamp_millis(),
                    1500,
                    action.remote,
                );
                continue;
            }
            let mut entry = json!({"label":label(&current),"sessionId":id,"cwd":current["cwd"],"stopped":ended(&current),"_wakeActivity":current["lastActivity"]});
            if !reply.is_empty() {
                let excerpt = fleet_messages::excerpt(&reply);
                entry["lastReply"] = excerpt.clone().into();
                if excerpt != reply.trim() {
                    entry["fullReply"] = reply.clone().into();
                }
            }
            if let Some(reason) = failure_reason(&current, &reply) {
                entry["failed"] = reason.into();
            }
            let mut contract = "absent";
            let mut outcome = None;
            if let Some(escalation) = worker_results::read_escalation(&reply) {
                if let Some(json) = escalation.json {
                    entry["escalation"] = json.into();
                    contract = "escalated";
                }
                if let Some(error) = escalation.error {
                    entry["escalationError"] = error.into();
                }
            }
            if contract != "escalated" && current["resultSchema"].is_object() {
                let report = worker_results::read_result(&reply, &current["resultSchema"]);
                if let Some(json) = report.json {
                    outcome = serde_json::from_str(&json).ok();
                    entry["result"] = json.into();
                    contract = "valid";
                }
                if let Some(error) = report.error {
                    entry["resultError"] = error.into();
                    contract = "invalid";
                }
            }
            if !text(&entry, "failed").is_empty() {
                contract = "invalid";
            }
            let signature = format!(
                "{reply} {} {} {}",
                if ended(&current) { 1 } else { 0 },
                text(&entry, "failed"),
                if !text(&entry, "escalation").is_empty() {
                    "escalated"
                } else if !text(&entry, "escalationError").is_empty() {
                    "invalid-escalation"
                } else {
                    ""
                }
            );
            let prior = { self.state.lock().unwrap().signatures.get(&id).cloned() };
            let prior = prior.or_else(|| {
                self.replacements
                    .as_ref()
                    .and_then(|state| state.signature(&id))
            });
            if action.kind != "catch-up" && prior.as_deref() == Some(&signature) {
                continue;
            }
            let mut review_id = None;
            let review = { self.review.lock().unwrap().clone() };
            if !action.remote
                && let Some(review) = review
            {
                match review
                    .capture(
                        &action.parent,
                        &id,
                        if ended(&current) {
                            "session-ended"
                        } else {
                            "turn-ended"
                        },
                    )
                    .await
                {
                    Ok(id) => review_id = id,
                    Err(error) => eprintln!("finish review capture unavailable: {error}"),
                }
                // Git capture crosses an await: a resumed worker or ownership
                // transfer must not emit an obsolete finish/validation receipt.
                let Some(after) = (self.lookup)(&id) else {
                    continue;
                };
                let target = self.target(text(&after, "parentSessionId"), &id)?;
                if !idle(&after)
                    || after["lastActivity"] != current["lastActivity"]
                    || target != action.parent
                {
                    if idle(&after) && !target.is_empty() {
                        schedule_for(
                            &mut self.state.lock().unwrap(),
                            &action.kind,
                            &target,
                            &id,
                            0,
                            chrono::Utc::now().timestamp_millis(),
                            1500,
                            false,
                        );
                    }
                    continue;
                }
                if let Some(id) = &review_id {
                    entry["reviewEvidenceId"] = json!(id);
                }
            }
            self.state.lock().unwrap().finishes.insert(
                id.clone(),
                json!({"reply":reply,"stopped":ended(&current),"parentSessionId":action.parent}),
            );
            if !action.remote
                && let Some(state) = &self.replacements
            {
                state.record_finish(&action.parent, &id, &reply, ended(&current))?;
            }
            if !action.remote
                && let Some(history) = self.history.clone()
            {
                let session = id.clone();
                tokio::task::spawn_blocking(move || {
                    history.validated(&session, contract, review_id.as_deref(), outcome)
                })
                .await??;
            }
            let item = (entry, signature, reply);
            if contract == "escalated" {
                escalated.push(item);
            } else {
                completed.push(item);
            }
        }
        for (kind, items) in [
            ("worker-escalated", escalated),
            (action.kind.as_str(), completed),
        ] {
            let items: Vec<_> = items
                .into_iter()
                .filter(|(entry, _, _)| {
                    let id = text(entry, "sessionId");
                    let Some(row) = (self.lookup)(id) else {
                        return false;
                    };
                    if kind == "blocked" {
                        return !ended(&row)
                            && blocked(text(&row, "ambientState"))
                            && self.state.lock().unwrap().generations.get(id).copied()
                                == entry["_wakeGeneration"].as_u64();
                    }
                    let valid = idle(&row)
                        && row["lastActivity"] == entry["_wakeActivity"]
                        && self
                            .target(text(&row, "parentSessionId"), id)
                            .is_ok_and(|parent| parent == action.parent);
                    if !valid && idle(&row) {
                        if let Ok(parent) = self.target(text(&row, "parentSessionId"), id) {
                            schedule_for(
                                &mut self.state.lock().unwrap(),
                                kind,
                                &parent,
                                id,
                                0,
                                chrono::Utc::now().timestamp_millis(),
                                1500,
                                action.remote,
                            );
                        }
                    }
                    valid
                })
                .collect();
            if items.is_empty() || (!action.remote && !self.available(&action.parent)) {
                continue;
            }
            let signatures: Vec<_> = items
                .iter()
                .filter(|(_, sig, _)| !sig.is_empty())
                .map(|(entry, sig, _)| (text(entry, "sessionId").to_owned(), sig.clone()))
                .collect();
            if action.remote {
                let receiver = self
                    .remote
                    .as_ref()
                    .ok_or_else(|| anyhow!("remote wake return channel unavailable"))?;
                for (entry, signature, _) in items {
                    let id = text(&entry, "sessionId").to_owned();
                    let kind = match kind {
                        "blocked" => super::remote_dispatch::Kind::Blocked,
                        "worker-escalated" => super::remote_dispatch::Kind::WorkerEscalated,
                        _ => super::remote_dispatch::Kind::WorkerFinished,
                    };
                    if receiver.report(id.clone(), kind, entry).await? && kind.terminal() {
                        let mut state = self.state.lock().unwrap();
                        state.signatures.insert(id.clone(), signature);
                        state.finishes.remove(&id);
                    }
                }
                continue;
            }
            let mut entries: Vec<_> = items.iter().map(|(entry, _, _)| entry.clone()).collect();
            if !ordinary && kind != "blocked" {
                if let Some(history) = self.history.clone() {
                    // Read after validation commits; use current durable ownership,
                    // not a task pin cached when the worker was dispatched.
                    let tasks = tokio::task::spawn_blocking(move || history.list()).await??;
                    // The disk read yields. A resume, reparent or manager role
                    // change must not deliver the old task instructions.
                    if !self.available(&action.parent)
                        || (self.lookup)(&action.parent)
                            .is_none_or(|row| row["isWakeTarget"] != true)
                        || entries.iter().any(|entry| {
                            let id = text(entry, "sessionId");
                            (self.lookup)(id).is_none_or(|row| {
                                !idle(&row)
                                    || row["lastActivity"] != entry["_wakeActivity"]
                                    || !self
                                        .target(text(&row, "parentSessionId"), id)
                                        .is_ok_and(|parent| parent == action.parent)
                            })
                        })
                    {
                        continue;
                    }
                    for entry in &mut entries {
                        let session = text(entry, "sessionId");
                        let instructions: Vec<_> = tasks
                            .iter()
                            .filter(|task| {
                                task["ownerSessionId"] == action.parent
                                    && task["workflow"].is_object()
                                    && task["workflow"]["steps"].as_array().is_some_and(|steps| {
                                        steps.iter().any(|step| step["sessionId"] == session)
                                    })
                            })
                            .map(|task| super::workflow_runtime::instructions(task, &tasks))
                            .collect();
                        if !instructions.is_empty() {
                            entry["workflowInstructions"] = instructions.join("\n\n").into();
                        }
                    }
                }
            }
            let message = fleet_messages::build(kind, &entries, ordinary)?;
            (self.delivery)(action.parent.clone(), message, signatures.clone()).await?;
            if kind != "catch-up" && kind != "blocked" {
                for (entry, signature, reply) in items {
                    let id = text(&entry, "sessionId");
                    {
                        let mut state = self.state.lock().unwrap();
                        state.signatures.insert(id.into(), signature.clone());
                        if state
                            .finishes
                            .get(id)
                            .is_some_and(|finish| finish["reply"] == reply)
                        {
                            state.finishes.remove(id);
                        }
                    }
                    if let Some(state) = &self.replacements {
                        state.record_signature_for_finish(
                            id,
                            &signature,
                            &reply,
                            entry["stopped"] == true,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
    pub async fn tick(&self, now: i64) -> Result<()> {
        let mut first_error = None;
        for action in self.due(now, 16) {
            if let Err(error) = self.deliver_action(action).await {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.subscribe();
        if *stop.borrow() {
            return Ok(());
        }
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut backstop = tokio::time::interval(Duration::from_secs(120));
        backstop.tick().await;
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _=stop.changed()=>break,
                _=backstop.tick()=>self.backstop(chrono::Utc::now().timestamp_millis()),
                _=interval.tick()=>{let now=chrono::Utc::now().timestamp_millis();for action in self.due(now,16usize.saturating_sub(tasks.len())){let service=self.clone();tasks.spawn(async move{service.deliver_action(action).await});}},
                Some(result)=tasks.join_next()=>match result{Ok(Ok(()))=>(),Ok(Err(error))=>eprintln!("fleet wake was not confirmed: {error}"),Err(error)=>eprintln!("fleet wake task failed: {error}")},
            }
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        Ok(())
    }
    pub fn close(&self) {
        self.stop.send_replace(true);
    }
}
impl WakeControl for Wakes {
    fn evidence(&self, parent: &str, workers: &[String]) -> Result<Value> {
        let state = self.state.lock().unwrap();
        let signatures: BTreeMap<_, _> = state
            .signatures
            .iter()
            .filter(|(id, _)| workers.contains(id))
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        let finishes: BTreeMap<_, _> = state
            .finishes
            .iter()
            .filter(|(id, finish)| workers.contains(id) && finish["parentSessionId"] == parent)
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        Ok(json!({"signatures":signatures,"finishes":finishes}))
    }
    fn flush(&self, parents: Vec<String>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            for parent in parents {
                let actions = {
                    let mut state = self.state.lock().unwrap();
                    let keys: Vec<_> = state
                        .groups
                        .keys()
                        .filter(|(_, target, remote)| target == &parent && !remote)
                        .cloned()
                        .collect();
                    keys.into_iter()
                        .map(|(kind, parent, remote)| {
                            let group = state
                                .groups
                                .remove(&(kind.clone(), parent.clone(), remote))
                                .unwrap();
                            Action {
                                remote,
                                kind,
                                parent,
                                workers: group.workers,
                            }
                        })
                        .collect::<Vec<_>>()
                };
                for action in actions {
                    self.deliver_action(action).await?;
                }
                let serial = self.serial.lock().unwrap().get(&parent).cloned();
                if let Some(serial) = serial {
                    let _wait = serial.lock().await;
                }
            }
            Ok(())
        })
    }
    fn recover(&self, operation: &Value) -> Result<()> {
        let parent = if operation["committed"] == true {
            text(operation, "successorSessionId")
        } else {
            text(operation, "sourceSessionId")
        };
        let mut state = self.state.lock().unwrap();
        for (id, signature) in operation["signatures"]
            .as_object()
            .into_iter()
            .flat_map(|map| map.iter())
        {
            if let Some(signature) = signature.as_str() {
                state
                    .signatures
                    .entry(id.clone())
                    .or_insert_with(|| signature.into());
            }
        }
        let ids: BTreeSet<_> = operation["workerIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .chain(
                operation["finishes"]
                    .as_object()
                    .into_iter()
                    .flat_map(|map| map.keys().map(String::as_str)),
            )
            .collect();
        for id in ids {
            schedule(
                &mut state,
                "worker-finished",
                parent,
                id,
                0,
                chrono::Utc::now().timestamp_millis(),
                0,
            );
        }
        Ok(())
    }
}
fn final_turn(result: Result<Value>) -> (bool, String) {
    let Ok(value) = result else {
        return (true, String::new());
    };
    let Some(items) = value.get("items") else {
        return (false, String::new());
    };
    if items.is_null() {
        return (false, String::new());
    }
    let Some(items) = items.as_array() else {
        return (true, String::new());
    };
    let (mut task, mut reply) = (false, String::new());
    for item in items {
        match text(item, "kind") {
            "user_message" => task = true,
            "assistant_text" => reply = text(item, "text").into(),
            _ => (),
        }
    }
    (task, reply)
}
pub fn failure_reason(row: &Value, reply: &str) -> Option<String> {
    let raw = reply
        .trim_start_matches(super::progress::js_space)
        .strip_prefix("⚠️ Error: ")?
        .split('\n')
        .next()
        .unwrap_or("");
    let flat = super::progress::flatten_note(raw);
    let flattened = regex::Regex::new(r" [—–-]{1,2} ")
        .unwrap()
        .replace_all(&flat, " - ");
    let reason = if flattened.is_empty() {
        "the provider reported an error with no message".into()
    } else {
        fleet_messages::clip(&flattened, 200, "…")
    };
    let usage=regex::Regex::new(r"(?i)out of (?:usage )?credits|credit balance|insufficient credits?|\b(?:session|usage|weekly|monthly|daily) limit\b").unwrap();
    Some(
        if row["statusLine"]["overageOutOfCredits"] == true && usage.is_match(&reason) {
            format!("out of credits (overage disabled) - {reason}")
        } else {
            reason
        },
    )
}

pub(crate) fn install(mut options: crate::Options) -> crate::Options {
    let Some(engine) = options.engine.clone() else {
        return options;
    };
    let lookup = super::local_lookup(&options);
    let rows = options.session_snapshots.clone();
    let lifecycle = options.launch_lifecycle.clone();
    let replacements = options.replacements.clone();
    let list: Inventory = Arc::new(move || {
        let rows: Vec<_> = rows.read().unwrap().values().cloned().collect();
        rows.into_iter()
            .map(|row| {
                super::snapshots::with_host_metadata(
                    row,
                    lifecycle.as_deref(),
                    replacements.as_deref(),
                )
            })
            .collect()
    });
    let capture_engine = engine.clone();
    let capture: Capture = Arc::new(move |id| {
        let engine = capture_engine.clone();
        Box::pin(async move {
            let id = url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
            engine
                .request(claudemon::daemon::embedded::Command::Request {
                    method: "GET".into(),
                    path: format!("/sessions/{id}/conversation"),
                    payload: None,
                })
                .await
        })
    });
    let tracker = options.message_tracker.clone();
    let requests = options
        .workflow_runtime
        .as_ref()
        .map(|workflow| workflow.requests.clone());
    let delivery: Delivery = Arc::new(move |id, message, signatures| {
        let (engine, tracker, requests) = (engine.clone(), tracker.clone(), requests.clone());
        Box::pin(async move {
            if let Some(tracker) = tracker {
                let result = super::manager_replacements::messages::send_engine(
                    &engine,
                    &tracker,
                    &id,
                    &message,
                    &signatures,
                    None,
                    false,
                    requests.as_deref(),
                )
                .await?;
                return match result.outcome {
                    super::manager_replacements::SendOutcome::Accepted => Ok(()),
                    super::manager_replacements::SendOutcome::Rejected { reason, .. } => {
                        Err(anyhow!("wake rejected: {reason}"))
                    }
                    super::manager_replacements::SendOutcome::Uncertain(reason) => Err(anyhow!(
                        "wake outcome unknown; not replaying immediately: {reason}"
                    )),
                };
            }
            let id = url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
            engine
                .request(claudemon::daemon::embedded::Command::Request {
                    method: "POST".into(),
                    path: format!("/sessions/{id}/message"),
                    payload: Some(json!({"text":message})),
                })
                .await?;
            Ok(())
        })
    });
    let mut wakes = Wakes::new(
        lookup,
        list,
        capture,
        delivery,
        options
            .workflow_runtime
            .as_ref()
            .map(|workflow| workflow.tasks.clone()),
        options.replacements.clone(),
    );
    if let Some(review) = options.review_store.clone() {
        wakes.set_review_store(review);
    }
    Arc::get_mut(&mut wakes).unwrap().remote = options
        .remote_receiver
        .clone()
        .map(|receiver| receiver as Arc<dyn super::remote_dispatch::return_channel::ReturnChannel>);
    options.wakes = Some(wakes);
    options
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    use crate::services::remote_dispatch::{Kind, return_channel::TestReturns};
    fn fixture() -> (
        Arc<Wakes>,
        Arc<TestReturns>,
        Arc<Mutex<BTreeMap<String, Value>>>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let rows = Arc::new(Mutex::new(BTreeMap::from([
            (
                "worker".into(),
                json!({"sessionId":"worker","parentSessionId":"remote-parent","cwd":"/execution/repo","ambientState":"streaming","lastActivity":1}),
            ),
            // Deliberately shares the foreign parent's ID. A finish must not be
            // routed here just because two hubs chose the same session string.
            (
                "remote-parent".into(),
                json!({"sessionId":"remote-parent","isWakeTarget":true,"ambientState":"idle"}),
            ),
        ])));
        let lookup = rows.clone();
        let inventory = rows.clone();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let deliveries = sent.clone();
        let mut service = Wakes::new(
            Arc::new(move |id| lookup.lock().unwrap().get(id).cloned()),
            Arc::new(move || inventory.lock().unwrap().values().cloned().collect()),
            Arc::new(|_| {
                Box::pin(async {
                    Ok(
                        json!({"items":[{"kind":"user_message","text":"task"},{"kind":"assistant_text","text":"done"}]}),
                    )
                })
            }),
            Arc::new(move |id, _, _| {
                let deliveries = deliveries.clone();
                Box::pin(async move {
                    deliveries.lock().unwrap().push(id);
                    Ok(())
                })
            }),
            None,
            None,
        );
        let returns = TestReturns::new();
        Arc::get_mut(&mut service).unwrap().remote = Some(returns.clone());
        (service, returns, rows, sent)
    }
    #[tokio::test]
    async fn terminal_wake_uses_owned_remote_channel_despite_local_identity_collision() {
        let (service, returns, rows, sent) = fixture();
        let before = rows.lock().unwrap()["worker"].clone();
        service.prime(&[before]);
        let after = {
            let mut rows = rows.lock().unwrap();
            let worker = rows.get_mut("worker").unwrap();
            worker["ambientState"] = json!("idle");
            worker["lastActivity"] = json!(2);
            worker.clone()
        };
        service.observe(&after, 1000);
        service.tick(2500).await.unwrap();
        assert!(sent.lock().unwrap().is_empty());
        let entries = returns.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, Kind::WorkerFinished);
        assert_eq!(entries[0].1["lastReply"], "done");
        drop(entries);
        service.backstop(200_000);
        service.tick(200_000).await.unwrap();
        assert_eq!(returns.entries.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn remote_block_wakes_origin_and_local_managers_only_after_surviving_delay() {
        let (service, returns, rows, sent) = fixture();
        let blocked = {
            let mut rows = rows.lock().unwrap();
            let worker = rows.get_mut("worker").unwrap();
            worker["ambientState"] = json!("waiting_approval");
            worker.clone()
        };
        service.observe(&blocked, 1000);
        service.tick(20_999).await.unwrap();
        assert!(returns.entries.lock().unwrap().is_empty());
        service.tick(21_000).await.unwrap();
        service.tick(22_500).await.unwrap();
        let entries = returns.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, Kind::Blocked);
        assert_eq!(entries[0].1["blockedOn"], "approval");
        assert_eq!(*sent.lock().unwrap(), vec!["remote-parent"]);
    }
    #[tokio::test]
    async fn resumed_remote_activity_cancels_block_and_finished_notifications() {
        let (service, returns, rows, sent) = fixture();
        for (mode, at) in [
            ("waiting_approval", 1000),
            ("streaming", 2000),
            ("idle", 3000),
            ("streaming", 4000),
        ] {
            let row = {
                let mut rows = rows.lock().unwrap();
                let row = rows.get_mut("worker").unwrap();
                row["ambientState"] = json!(mode);
                row["lastActivity"] = json!(at);
                row.clone()
            };
            service.observe(&row, at);
        }
        service.tick(30_000).await.unwrap();
        assert!(returns.entries.lock().unwrap().is_empty());
        assert!(sent.lock().unwrap().is_empty());
    }
}

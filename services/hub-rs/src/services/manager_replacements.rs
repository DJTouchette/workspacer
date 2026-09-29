//! Durable manager lineage and handoff delivery authority. Metadata restoration
//! never implies process liveness, and an unacknowledged send is never replayed.
use super::task_store::{remove, text};
pub mod artifact;
pub mod messages;
pub mod native;
pub mod service;
use anyhow::{Context, Result, bail};
pub use messages::{BeginDelivery, MessageTracker};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub use service::{ReplacementHost, ReplacementService, SendOutcome, Timing};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub version: u32,
    pub operations: Vec<Value>,
    pub launches: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_metadata: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_signatures: BTreeMap<String, String>,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            version: 1,
            operations: vec![],
            launches: BTreeMap::new(),
            pending_metadata: BTreeMap::new(),
            pending_signatures: BTreeMap::new(),
        }
    }
}
pub struct ReplacementState {
    path: PathBuf,
    data: Mutex<Journal>,
    active: Mutex<BTreeMap<String, usize>>,
    manual: Mutex<BTreeSet<String>>,
    last_error: Mutex<Option<String>>,
}
pub fn metadata_only(value: &Value) -> Value {
    let mut out = json!({});
    for key in [
        "sessionId",
        "cwd",
        "label",
        "parentSessionId",
        "isWakeTarget",
        "provider",
        "transport",
        "settings",
        "resultSchema",
        "routing",
    ] {
        if let Some(value) = value.get(key) {
            out[key] = value.clone();
        }
    }
    out
}
fn related(data: &Journal, id: &str) -> Option<Value> {
    data.operations
        .iter()
        .rev()
        .find(|o| o["sourceSessionId"] == id || o["successorSessionId"] == id)
        .cloned()
}
fn held(data: &Journal, id: &str) -> Option<Value> {
    let o = related(data, id)?;
    if o["sourceSessionId"] == id && (o["committed"] == true || o["transferIntent"] == true) {
        return Some(o);
    }
    if o["successorSessionId"] == id && o["phase"] == "activating" && o["committed"] == true {
        return None;
    }
    if !matches!(text(&o["phase"]), "complete" | "failed" | "cancelled") {
        Some(o)
    } else {
        None
    }
}
fn metadata(data: &Journal, id: &str) -> Option<Value> {
    for op in data.operations.iter().rev() {
        if let Some(m) = op["metadata"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["sessionId"] == id)
        {
            let mut m = m.clone();
            if m["parentSessionId"] == op["sourceSessionId"]
                && (op["committed"] == true || op["transferIntent"] == true)
            {
                m["parentSessionId"] = op["successorSessionId"].clone();
            }
            return Some(m);
        }
    }
    if let Some(m) = data.pending_metadata.get(id) {
        return Some(m.clone());
    }
    let launch = data.launches.get(id)?;
    let options = &launch["options"];
    if options["manager"] != true || text(&options["cwd"]).is_empty() {
        return None;
    }
    let mut m = json!({"sessionId":id,"cwd":options["cwd"],"isWakeTarget":true,"settings":{}});
    for key in ["parentSessionId", "label", "provider", "transport"] {
        if let Some(v) = options.get(key) {
            m[key] = v.clone();
        }
    }
    for key in ["model", "contextWindow", "effort", "permissionMode"] {
        if let Some(v) = options.get(key) {
            m["settings"][key] = v.clone();
        }
    }
    Some(m)
}
fn target(data: &Journal, parent: &str, manual: bool) -> Result<String> {
    let mut current = parent.to_string();
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current.clone()) {
            bail!("Replacement ownership routing contains a cycle");
        }
        let redirect = if manual {
            data.operations
                .iter()
                .rev()
                .find_map(|o| o["ownerRedirects"][&current].as_str().map(str::to_string))
        } else {
            None
        };
        let next = redirect.or_else(|| {
            data.operations
                .iter()
                .find(|o| o["sourceSessionId"] == current && o["committed"] == true)
                .and_then(|o| o["successorSessionId"].as_str())
                .map(str::to_string)
        });
        match next {
            Some(next) => current = next,
            None => return Ok(current),
        }
    }
}
pub fn saved_finish_delivery(worker: &str, evidence: &Value) -> Value {
    json!({"id":format!("finish:{worker}"),"kind":"message","status":"pending","text":format!("Recorded completion evidence for session:{worker}. This is not an automatic workflow verdict; inspect task history before continuing.\n{}",text(&evidence["reply"])),"error":"Completion was recorded, but delivery is not confirmed. Inspect worker transcript before accounting for it or sending again."})
}
impl ReplacementState {
    pub fn open(path: PathBuf) -> Result<Arc<Self>> {
        let data = read(&path)?;
        Ok(Arc::new(Self {
            path,
            data: Mutex::new(data),
            active: Mutex::new(BTreeMap::new()),
            manual: Mutex::new(BTreeSet::new()),
            last_error: Mutex::new(None),
        }))
    }
    pub fn edit<T>(&self, change: impl FnOnce(&mut Journal) -> Result<T>) -> Result<T> {
        let mut current = self.data.lock().unwrap();
        std::fs::create_dir_all(
            self.path
                .parent()
                .context("journal needs parent directory")?,
        )?;
        let _lock =
            crate::auth::StoreLock::take(&PathBuf::from(format!("{}.rust", self.path.display())))?;
        let mut next = if !self.path.exists()
            && (!current.operations.is_empty()
                || !current.launches.is_empty()
                || !current.pending_metadata.is_empty())
        {
            // Losing the directory entry while this owner is live must not
            // erase its in-memory retirement/transfer authority on the next ACK.
            current.clone()
        } else {
            read(&self.path)?
        };
        let outcome: Result<T> = (|| {
            let result = change(&mut next)?;
            validate(&next)?;
            let value = serde_json::to_value(&next)?;
            if serde_json::to_vec(&value)?.len() > 8 * 1024 * 1024 {
                bail!("Replacement journal capacity reached; retained evidence was not discarded");
            }
            if !self.path.exists() || serde_json::to_value(&*current)? != value {
                super::atomic_json(&self.path, &value, true)?;
            }
            Ok(result)
        })();
        match outcome {
            Ok(result) => {
                *current = next;
                Ok(result)
            }
            Err(error) => {
                *self.last_error.lock().unwrap() = Some(error.to_string());
                Err(error)
            }
        }
    }
    pub fn records(&self) -> Vec<Value> {
        self.data.lock().unwrap().operations.clone()
    }
    pub fn views(&self) -> Vec<Value> {
        self.records()
            .into_iter()
            .map(|mut o| {
                let finishes = o["finishes"].as_object().cloned().unwrap_or_default();
                for key in [
                    "launch",
                    "projectCwds",
                    "ownerRedirects",
                    "metadata",
                    "signatures",
                    "finishes",
                    "artifact",
                    "artifactHash",
                    "transferIntent",
                    "retired",
                    "taskTransferCommitted",
                ] {
                    remove(&mut o, key);
                }
                let deliveries = o["deliveries"].as_array_mut().unwrap();
                for (worker, evidence) in finishes {
                    deliveries.push(saved_finish_delivery(&worker, &evidence));
                }
                o
            })
            .collect()
    }
    pub fn get(&self, id: &str) -> Result<Value> {
        self.data
            .lock()
            .unwrap()
            .operations
            .iter()
            .find(|o| o["operationId"] == id)
            .cloned()
            .context("Unknown manager replacement")
    }
    pub fn change<T>(&self, id: &str, change: impl FnOnce(&mut Value) -> Result<T>) -> Result<T> {
        self.edit(|data| {
            let op = data
                .operations
                .iter_mut()
                .find(|o| o["operationId"] == id)
                .context("Unknown manager replacement")?;
            let result = change(op)?;
            op["updatedAt"] = chrono::Utc::now().timestamp_millis().into();
            Ok(result)
        })
    }
    pub fn error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }
    pub fn clear_error(&self) {
        *self.last_error.lock().unwrap() = None;
    }
    pub fn launch(&self, id: &str) -> Option<Value> {
        self.data.lock().unwrap().launches.get(id).cloned()
    }
    pub fn remember_launch(&self, id: &str, launch: Value) -> Result<()> {
        self.edit(|data| {
            if !data.launches.contains_key(id) && data.launches.len() >= 128 {
                bail!("Manager launch history capacity reached");
            }
            data.launches.insert(id.into(), launch);
            Ok(())
        })
    }
    pub fn update_launch(&self, id: &str, patch: &Value) -> Result<()> {
        self.edit(|data| {
            if let Some(launch) = data.launches.get_mut(id) {
                for (k, v) in patch.as_object().context("Launch patch must be object")? {
                    launch["options"][k] = v.clone();
                }
            }
            Ok(())
        })
    }
    pub fn remember_child(&self, m: Value) -> Result<()> {
        let m = metadata_only(&m);
        if text(&m["cwd"]).is_empty() {
            return Ok(());
        }
        self.edit(|data| {
            let parent = text(&m["parentSessionId"]);
            let op = related(data, text(&m["sessionId"]))
                .or_else(|| related(data, parent))
                .or_else(|| {
                    data.operations
                        .iter()
                        .rev()
                        .find(|o| {
                            o["metadata"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|row| row["sessionId"] == m["sessionId"])
                        })
                        .cloned()
                })
                .or_else(|| {
                    data.operations
                        .iter()
                        .rev()
                        .find(|o| {
                            o["metadata"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|m| m["sessionId"] == parent && m["isWakeTarget"] == true)
                        })
                        .cloned()
                });
            let known = data
                .launches
                .get(parent)
                .is_some_and(|l| l["options"]["manager"] == true)
                || data
                    .pending_metadata
                    .get(parent)
                    .is_some_and(|m| m["isWakeTarget"] == true);
            if op.is_none()
                && m["isWakeTarget"] != true
                && !known
                && !data.pending_metadata.contains_key(text(&m["sessionId"]))
            {
                return Ok(());
            }
            if let Some(op) = op {
                let record = data
                    .operations
                    .iter_mut()
                    .find(|r| r["operationId"] == op["operationId"])
                    .unwrap();
                let rows = record["metadata"].as_array_mut().unwrap();
                if let Some(row) = rows.iter_mut().find(|r| r["sessionId"] == m["sessionId"]) {
                    *row = m;
                } else {
                    rows.push(m);
                }
            } else {
                data.pending_metadata
                    .insert(text(&m["sessionId"]).into(), m);
            }
            Ok(())
        })
    }
    pub fn forget_unclaimed_metadata(&self, id: &str) -> Result<()> {
        self.edit(|data| {
            data.pending_metadata.remove(id);
            Ok(())
        })
    }
    pub fn metadata(&self, id: &str) -> Option<Value> {
        metadata(&self.data.lock().unwrap(), id)
    }
    pub fn enrich(&self, mut snapshot: Value) -> Value {
        if !text(&snapshot["hub"]).is_empty() {
            return snapshot;
        }
        let id = snapshot["sessionId"]
            .as_str()
            .or_else(|| snapshot["session_id"].as_str())
            .unwrap_or("");
        if let Some(metadata) = self.metadata(id) {
            for key in [
                "label",
                "parentSessionId",
                "isWakeTarget",
                "settings",
                "resultSchema",
                "routing",
            ] {
                if let Some(value) = metadata.get(key) {
                    snapshot[key] = value.clone();
                }
            }
            for key in ["cwd", "provider", "transport"] {
                if snapshot.get(key).is_none_or(Value::is_null) {
                    if let Some(value) = metadata.get(key) {
                        snapshot[key] = value.clone();
                    }
                }
            }
        }
        snapshot
    }
    pub fn recovery_metadata(&self) -> Vec<Value> {
        let data = self.data.lock().unwrap();
        let mut ids: BTreeSet<_> = data
            .pending_metadata
            .keys()
            .chain(data.launches.keys())
            .cloned()
            .collect();
        for o in &data.operations {
            for m in o["metadata"].as_array().into_iter().flatten() {
                ids.insert(text(&m["sessionId"]).into());
            }
        }
        ids.iter().filter_map(|id| metadata(&data, id)).collect()
    }
    pub fn related(&self, id: &str) -> Option<Value> {
        related(&self.data.lock().unwrap(), id)
    }
    pub fn held(&self, id: &str) -> Option<Value> {
        held(&self.data.lock().unwrap(), id)
    }
    pub fn parked_successor(&self, id: &str) -> bool {
        self.related(id)
            .is_some_and(|o| o["successorSessionId"] == id && o["committed"] != true)
    }
    pub fn assert_available(&self, id: &str) -> Result<()> {
        if self.manual.lock().unwrap().contains(id) {
            bail!("Session {id} is fenced by a host ownership/close operation");
        }
        if let Some(op) = self.held(id) {
            bail!(
                "Manager {id} fenced by replacement {}; inspect handoff status",
                text(&op["operationId"])
            );
        }
        Ok(())
    }
    pub fn assert_resume(&self, id: &str) -> Result<()> {
        if let Some(op) = self.related(id) {
            let primary = op["successorSessionId"] == id
                && op["committed"] == true
                && op["phase"] == "complete"
                || op["sourceSessionId"] == id
                    && op["transferIntent"] != true
                    && matches!(text(&op["phase"]), "failed" | "cancelled");
            if !primary {
                bail!("Retired or incomplete manager handoff cannot resume stale context");
            }
        }
        Ok(())
    }
    pub fn admit(self: &Arc<Self>, ids: &[&str]) -> Result<AdmissionGuard> {
        let ids: BTreeSet<String> = ids
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let mut active = self.active.lock().unwrap();
        for id in &ids {
            self.assert_available(id)?;
        }
        for id in &ids {
            *active.entry(id.clone()).or_default() += 1;
        }
        Ok(AdmissionGuard {
            state: self.clone(),
            ids,
            manual: false,
        })
    }
    /// Serialize installation of the durable handoff fence with ordinary and
    /// manual admission. Already admitted launches drain during preparation.
    pub(crate) fn handoff_admission(&self, id: &str) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, usize>>> {
        let guard = self.active.lock().unwrap();
        self.assert_available(id)?;
        Ok(guard)
    }
    /// Check existing launches and fence new ones under the SAME activity lock.
    /// The owned guard may cross await points; journal attribution edits remain
    /// allowed while it is held so manual ownership can converge durably.
    pub fn manual_admission(self: &Arc<Self>, ids: &[&str]) -> Result<AdmissionGuard> {
        let ids: BTreeSet<String> = ids
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let mut active = self.active.lock().unwrap();
        for id in &ids {
            self.assert_available(id)?;
            if active.get(id).copied().unwrap_or(0) > 0 {
                bail!("Session {id} has an admitted launch/ownership operation in progress");
            }
        }
        let mut manual = self.manual.lock().unwrap();
        for id in &ids {
            manual.insert(id.clone());
            active.insert(id.clone(), 1);
        }
        Ok(AdmissionGuard {
            state: self.clone(),
            ids,
            manual: true,
        })
    }
    pub fn active_count(&self, id: &str) -> usize {
        self.active.lock().unwrap().get(id).copied().unwrap_or(0)
    }
    pub fn hold_message(
        &self,
        id: &str,
        message: &str,
        signatures: &[(String, String)],
        source_request: Option<Value>,
    ) -> Result<bool> {
        self.edit(|data|{
        let op=held(data,id).or_else(||related(data,id).filter(|o|o["phase"]=="activating"));let Some(op)=op else{return Ok(false);};if op["sourceSessionId"]==id&&op["phase"]=="complete"{bail!("Manager retired; send to successor {}",text(&op["successorSessionId"]));}
        let op=data.operations.iter_mut().find(|o|o["operationId"]==op["operationId"]).unwrap();if op["deliveries"].as_array().unwrap().len()>=256{bail!("Handoff message capacity reached; message was not accepted");}
        let mut delivery=json!({"id":uuid::Uuid::new_v4().to_string(),"kind":"message","text":if source_request.is_some(){""}else{message},"status":"pending"});if let Some(source)=source_request{delivery["sourceRequest"]=source;}op["deliveries"].as_array_mut().unwrap().push(delivery);
        for(worker,signature)in signatures{op["signatures"][worker]=signature.clone().into();}op["updatedAt"]=chrono::Utc::now().timestamp_millis().into();Ok(true)
    })
    }
    pub fn delivery(&self, id: &str, kind: &str, message: &str) -> Result<String> {
        if !["preparation", "kickoff", "message"].contains(&kind) {
            bail!("Invalid replacement delivery kind");
        }
        let delivery = uuid::Uuid::new_v4().to_string();
        self.change(id, |op| {
            op["deliveries"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":delivery,"kind":kind,"text":message,"status":"pending"}));
            Ok(())
        })?;
        Ok(delivery)
    }
    pub fn acknowledged(&self, id: &str) -> bool {
        self.records().iter().any(|o| {
            o["deliveries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["id"] == id && matches!(text(&d["status"]), "accepted" | "reconciled"))
        })
    }
    pub fn note_inflight_message(
        &self,
        parent: &str,
        id: &str,
        accepted: bool,
        error: Option<&str>,
        rejected: bool,
    ) -> Result<()> {
        self.edit(|data| {
            if let Some(op) = data.operations.iter_mut().rev().find(|o| {
                (o["sourceSessionId"] == parent || o["successorSessionId"] == parent)
                    && o["deliveries"].as_array().unwrap().iter().any(|d| {
                        d["id"] == id && matches!(text(&d["status"]), "sending" | "uncertain")
                    })
            }) {
                let d = op["deliveries"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|d| d["id"] == id)
                    .unwrap();
                d["status"] = if accepted {
                    "accepted"
                } else if rejected {
                    "pending"
                } else {
                    "uncertain"
                }
                .into();
                if let Some(e) = error {
                    d["error"] = e.into();
                }
                if !accepted {
                    op["phase"] = "recovery-required".into();
                    op["error"] = if rejected {
                        "Earlier message rejected; text retained"
                    } else {
                        "In-flight message acknowledgement uncertain; no automatic replay"
                    }
                    .into();
                }
            }
            Ok(())
        })
    }
    pub fn record_finish(
        &self,
        parent: &str,
        worker: &str,
        reply: &str,
        stopped: bool,
    ) -> Result<()> {
        self.edit(|data| {
            if let Some(op) = held(data, parent)
                .or_else(|| related(data, parent).filter(|o| o["phase"] == "activating"))
            {
                let current = data
                    .operations
                    .iter_mut()
                    .find(|o| o["operationId"] == op["operationId"])
                    .unwrap();
                current["finishes"][worker] = json!({"reply":reply,"stopped":stopped});
            }
            Ok(())
        })
    }
    pub fn clear_finish(&self, worker: &str, expected_reply: Option<&str>) -> Result<()> {
        self.edit(|data| {
            if let Some(op) = data.operations.iter_mut().rev().find(|o| {
                o["finishes"].get(worker).is_some()
                    && expected_reply.is_none_or(|r| o["finishes"][worker]["reply"] == r)
            }) {
                op["finishes"].as_object_mut().unwrap().remove(worker);
            }
            Ok(())
        })
    }
    pub fn signature(&self, worker: &str) -> Option<String> {
        let data = self.data.lock().unwrap();
        data.operations
            .iter()
            .rev()
            .find_map(|o| o["signatures"][worker].as_str().map(str::to_string))
            .or_else(|| data.pending_signatures.get(worker).cloned())
    }
    pub fn record_signature(&self, worker: &str, signature: &str) -> Result<()> {
        self.record_signature_for(worker, signature, None)
            .map(|_| ())
    }
    /// A send acknowledgement may only clear the exact finish it carried. A
    /// newer reply arriving during I/O remains durable for the next delivery.
    pub fn record_signature_for(
        &self,
        worker: &str,
        signature: &str,
        expected_reply: Option<&str>,
    ) -> Result<bool> {
        self.edit(|data| {
            let mut cleared = false;
            if let Some(op) = data.operations.iter_mut().rev().find(|o| {
                o["workerIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w == worker)
                    || o["metadata"].as_array().unwrap().iter().any(|m| {
                        m["sessionId"] == worker && !text(&m["parentSessionId"]).is_empty()
                    })
            }) {
                op["signatures"][worker] = signature.into();
                if expected_reply.is_some_and(|reply| op["finishes"][worker]["reply"] == reply) {
                    op["finishes"].as_object_mut().unwrap().remove(worker);
                    cleared = true;
                }
                data.pending_signatures.remove(worker);
            } else {
                data.pending_signatures
                    .insert(worker.into(), signature.into());
            }
            Ok(cleared)
        })
    }
    pub fn record_signature_for_finish(
        &self,
        worker: &str,
        signature: &str,
        expected_reply: &str,
        expected_stopped: bool,
    ) -> Result<bool> {
        self.edit(|data| {
            let mut cleared = false;
            if let Some(op) = data.operations.iter_mut().rev().find(|o| {
                o["workerIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w == worker)
                    || o["metadata"].as_array().unwrap().iter().any(|m| {
                        m["sessionId"] == worker && !text(&m["parentSessionId"]).is_empty()
                    })
            }) {
                op["signatures"][worker] = signature.into();
                if op["finishes"][worker]["reply"] == expected_reply
                    && op["finishes"][worker]["stopped"] == expected_stopped
                {
                    op["finishes"].as_object_mut().unwrap().remove(worker);
                    cleared = true;
                }
                data.pending_signatures.remove(worker);
            } else {
                data.pending_signatures
                    .insert(worker.into(), signature.into());
            }
            Ok(cleared)
        })
    }
    pub fn wake_target(&self, parent: &str) -> Result<String> {
        target(&self.data.lock().unwrap(), parent, true)
    }
    /// Host inbox capture follows durable transfer intent too: an input arriving
    /// between task adoption and final acknowledgement must not be stranded on
    /// the predecessor after the task-store transfer already ran.
    pub fn request_target(&self, parent: &str) -> Result<String> {
        let data = self.data.lock().unwrap();
        let mut current = parent.to_string();
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current.clone()) {
                bail!("Replacement request routing contains a cycle");
            }
            let redirect = data
                .operations
                .iter()
                .rev()
                .find_map(|o| o["ownerRedirects"][&current].as_str().map(str::to_string));
            let next = redirect.or_else(|| {
                data.operations
                    .iter()
                    .find(|o| {
                        o["sourceSessionId"] == current
                            && (o["committed"] == true || o["transferIntent"] == true)
                    })
                    .and_then(|o| o["successorSessionId"].as_str())
                    .map(str::to_string)
            });
            match next {
                Some(next) => current = next,
                None => return Ok(current),
            }
        }
    }
    pub fn automatic_wake_target(&self, parent: &str) -> Result<String> {
        target(&self.data.lock().unwrap(), parent, false)
    }
    pub fn worker_wake_target(&self, parent: &str, worker: &str) -> Result<String> {
        let data = self.data.lock().unwrap();
        let owner = metadata(&data, worker)
            .and_then(|m| m["parentSessionId"].as_str().map(str::to_string))
            .unwrap_or(parent.into());
        target(&data, &owner, false)
    }
    pub fn note_manual_reparent(
        &self,
        old: &str,
        new: &str,
        workers: &[String],
        destination: Value,
    ) -> Result<()> {
        self.edit(|data| {
            let mut workers: BTreeSet<_> = workers.iter().cloned().collect();
            for op in &data.operations {
                for m in op["metadata"].as_array().unwrap() {
                    if m["sessionId"] != new
                        && metadata(data, text(&m["sessionId"]))
                            .is_some_and(|m| m["parentSessionId"] == old)
                    {
                        workers.insert(text(&m["sessionId"]).into());
                    }
                }
            }
            let pending = data
                .pending_metadata
                .values()
                .any(|m| m["parentSessionId"] == old);
            if pending {
                for m in data.pending_metadata.values_mut() {
                    if m["parentSessionId"] == old && m["sessionId"] != new {
                        m["parentSessionId"] = new.into();
                    }
                }
                data.pending_metadata
                    .insert(new.into(), destination.clone());
            }
            for op in &mut data.operations {
                if let Some(r) = op["ownerRedirects"].as_object_mut() {
                    r.remove(new);
                }
                let affected = op["sourceSessionId"] == old
                    || op["successorSessionId"] == old
                    || op["metadata"].as_array().unwrap().iter().any(|m| {
                        workers.contains(text(&m["sessionId"]))
                            || m["isWakeTarget"] == true && m["sessionId"] == old
                    });
                if !affected {
                    continue;
                }
                let rows = op["metadata"].as_array_mut().unwrap();
                for m in rows.iter_mut() {
                    if workers.contains(text(&m["sessionId"])) {
                        m["parentSessionId"] = new.into();
                    }
                }
                if !rows.iter().any(|m| m["sessionId"] == new) {
                    rows.push(destination.clone());
                }
                if !op["ownerRedirects"].is_object() {
                    op["ownerRedirects"] = json!({});
                }
                op["ownerRedirects"][old] = new.into();
            }
            Ok(())
        })
    }
    /// Crash recovery records uncertainty before any external metadata restore.
    pub fn recover_status(&self) -> Result<()> {
        self.edit(|data| {
            for o in &mut data.operations {
                for d in o["deliveries"].as_array_mut().unwrap() {
                    if d["status"] == "sending" {
                        d["status"] = "uncertain".into();
                        d["error"] = "Host stopped before acknowledgement was recorded".into();
                    }
                }
                if !matches!(text(&o["phase"]), "complete" | "failed" | "cancelled") {
                    o["phase"] = "recovery-required".into();
                    o["bound"] = false.into();
                    o["error"] = "Host restarted during handoff; nothing was replayed".into();
                }
            }
            Ok(())
        })
    }
}
pub struct AdmissionGuard {
    state: Arc<ReplacementState>,
    ids: BTreeSet<String>,
    manual: bool,
}
impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        let mut active = self.state.active.lock().unwrap();
        if self.manual {
            let mut manual = self.state.manual.lock().unwrap();
            for id in &self.ids {
                manual.remove(id);
            }
        }
        for id in &self.ids {
            if let Some(n) = active.get_mut(id) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    active.remove(id);
                }
            }
        }
    }
}
fn read(path: &std::path::Path) -> Result<Journal> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Journal::default()),
        Err(e) => return Err(e.into()),
    };
    if bytes.len() > 8 * 1024 * 1024 {
        bail!("Replacement journal oversized");
    }
    let data: Journal = serde_json::from_slice(&bytes)?;
    validate(&data)?;
    Ok(data)
}
fn validate(data: &Journal) -> Result<()> {
    if data.version != 1 || data.launches.len() > 128 || data.operations.len() > 32 {
        bail!("Replacement journal format/capacity invalid");
    }
    let mut ids = BTreeSet::new();
    for op in &data.operations {
        if uuid::Uuid::parse_str(text(&op["operationId"])).is_err()
            || !ids.insert(text(&op["operationId"]))
            || text(&op["sourceSessionId"]).is_empty()
            || text(&op["successorSessionId"]).is_empty()
            || !matches!(
                text(&op["phase"]),
                "preparing"
                    | "spawning"
                    | "transferring"
                    | "binding"
                    | "activating"
                    | "complete"
                    | "failed"
                    | "cancelled"
                    | "recovery-required"
            )
            || !op["committed"].is_boolean()
            || !op["bound"].is_boolean()
            || !op["paneId"].is_string()
            || !op["workspaceId"].is_string()
            || op["launch"]["options"]["manager"] != true
            || op["launch"]["options"]["toolScope"] != "operator"
            || !op["launch"]["options"]["cwd"].is_string()
            || !op["signatures"].is_object()
            || !op["finishes"].is_object()
        {
            bail!("Replacement journal operation invalid; retained file requires inspection");
        }
        for key in ["workerIds", "taskIds"] {
            if !op[key]
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string))
            {
                bail!("Replacement ownership IDs invalid");
            }
        }
        if !op["metadata"].as_array().is_some_and(|a| {
            a.iter()
                .all(|m| m["sessionId"].is_string() && m["cwd"].is_string())
        }) {
            bail!("Replacement metadata invalid");
        }
        if !op["deliveries"].as_array().is_some_and(|a| {
            a.len() <= 256
                && a.iter().all(|d| {
                    d["id"].is_string()
                        && d["text"].is_string()
                        && matches!(
                            text(&d["status"]),
                            "pending" | "sending" | "accepted" | "uncertain" | "reconciled"
                        )
                        && matches!(text(&d["kind"]), "preparation" | "kickoff" | "message")
                })
        }) {
            bail!("Replacement deliveries invalid");
        }
    }
    Ok(())
}

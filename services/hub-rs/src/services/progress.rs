//! Bounded, one-way worker progress to its current parent. Session identity is
//! provided by the trusted facade, never selected by an untrusted bus caller.
use super::task_store::OwnerLookup;
use anyhow::{Result, anyhow, bail};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub type Deliver = Arc<dyn Fn(String, String) -> BoxFuture<'static, Result<()>> + Send + Sync>;
#[derive(Default)]
struct Budget {
    count: usize,
    last_at: i64,
    last_note: String,
}
pub struct Progress {
    budgets: Mutex<HashMap<String, Budget>>,
    lookup: OwnerLookup,
    deliver: Deliver,
    replacements: Option<Arc<super::manager_replacements::ReplacementState>>,
    remote: Option<Arc<dyn super::remote_dispatch::return_channel::ReturnChannel>>,
}
pub(super) fn js_space(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
pub fn flatten_note(note: &str) -> String {
    note.split(js_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub(super) fn label(row: &Value) -> String {
    if let Some(label) = row["label"].as_str().filter(|s| !s.is_empty()) {
        return label.into();
    }
    let cwd = row["cwd"].as_str().unwrap_or("");
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
pub fn message(row: &Value, note: &str, decision: bool) -> String {
    let entry = json!({"label":label(row),"sessionId":row["sessionId"],"cwd":row["cwd"],"note":note,"needsDecision":decision});
    super::fleet_messages::build("progress", &[entry], false).expect("known progress message kind")
}

impl Progress {
    #[cfg(feature = "test-support")]
    pub(crate) fn fixture_remote(mut self, receiver: Arc<super::remote_dispatch::Receiver>) -> Self {
        self.remote=Some(receiver);self
    }
    pub fn new(lookup: OwnerLookup, deliver: Deliver) -> Self {
        Self {
            budgets: Mutex::new(HashMap::new()),
            lookup,
            deliver,
            replacements: None,
            remote: None,
        }
    }
    pub fn with_replacements(
        mut self,
        state: Arc<super::manager_replacements::ReplacementState>,
    ) -> Self {
        self.replacements = Some(state);
        self
    }
    pub async fn report(&self, params: Value, now: i64) -> Result<Value> {
        let caller = params["callerSessionId"].as_str().unwrap_or("").trim();
        if caller.is_empty() {
            bail!(
                "report_progress: the host could not identify your session from your credential, so it cannot tell who dispatched you. This tool is for agents workspacer spawned; use send_message if you are driving the fleet."
            );
        }
        let note = flatten_note(params["note"].as_str().unwrap_or(""));
        if note.is_empty() {
            bail!("report_progress requires a non-empty note");
        }
        let length = note.encode_utf16().count();
        if length > 500 {
            bail!(
                "report_progress: note is {length} characters; the limit is 500. This is a progress LINE, not a report — say what changed for your manager's decision (a phase finished, the approach is wrong, the budget is running out) and leave the detail for your final message, which the finish wake delivers in full."
            );
        }
        let me = (self.lookup)(caller)
            .ok_or_else(|| anyhow!("report_progress: session {caller} is not a tracked session"))?;
        let remote = self
            .remote
            .as_ref()
            .filter(|receiver| receiver.owns(caller));
        let parent = me["parentSessionId"].as_str().unwrap_or("");
        if remote.is_none() && (parent.is_empty() || parent == caller) {
            bail!(
                "report_progress: you have no parent session — nothing dispatched you, so there is nobody to report to. Tell the user directly in your reply instead."
            );
        }
        let parent = match &self.replacements {
            _ if remote.is_some() => parent.into(),
            Some(state) => state.worker_wake_target(parent, caller)?,
            None => parent.into(),
        };
        let held = self
            .replacements
            .as_ref()
            .is_some_and(|state| state.held(&parent).is_some());
        if remote.is_none()
            && !held
            && (self.lookup)(&parent).is_none_or(|row| row["status"] == "ended")
        {
            bail!(
                "report_progress: your parent session ({parent}) has ended — there is nobody to receive this. Carry on and put it in your final message."
            );
        }
        {
            let mut budgets = self.budgets.lock().unwrap();
            let budget = budgets.entry(caller.into()).or_default();
            if budget.count >= 20 {
                bail!(
                    "report_progress: you have already sent 20 progress updates, which is the limit for one session. Stop reporting and finish the task — your final message reaches your manager in full."
                );
            }
            if note == budget.last_note {
                bail!(
                    "report_progress: that is the same note you just sent; it was NOT delivered again."
                );
            }
            let since = now.saturating_sub(budget.last_at);
            if budget.count > 0 && since < 60000 {
                bail!(
                    "report_progress: you reported {}s ago; updates are limited to one per 60s. This one was NOT delivered — carry on working and fold it into your next update.",
                    (since as f64 / 1000. + 0.5).floor()
                );
            }
            budget.count += 1;
            budget.last_at = now;
            budget.last_note = note.clone();
        }
        if let Some(receiver) = remote {
            let entry = json!({"label":label(&me),"sessionId":caller,"cwd":me["cwd"],"note":note,"needsDecision":params["needsDecision"]==true});
            if !receiver
                .report(caller.into(), super::remote_dispatch::Kind::Progress, entry)
                .await?
            {
                bail!(
                    "report_progress: this node could not publish to the machine that dispatched you. It was NOT delivered — carry on and fold it into your final message."
                );
            }
            return Ok(json!({"queuedTo":"dispatching-hub","route":"remote-dispatch"}));
        }
        let message = message(&me, &note, params["needsDecision"] == true);
        if let Some(state) = &self.replacements {
            if state.hold_message(&parent, &message, &[], None)? {
                return Ok(json!({"deliveredTo":parent}));
            }
        }
        let _admission = self
            .replacements
            .as_ref()
            .map(|state| state.admit(&[&parent]))
            .transpose()?;
        if (self.lookup)(&parent).is_none_or(|row| row["status"] == "ended") {
            bail!("report_progress: parent changed before delivery; the note was not sent");
        }
        (self.deliver)(parent.clone(), message).await?;
        Ok(json!({"deliveredTo":parent}))
    }
}
pub(crate) fn install(mut options: crate::Options) -> crate::Options {
    let Some(engine) = options.engine.clone() else {
        return options;
    };
    let lookup = super::local_lookup(&options);
    let deliver = engine_delivery(&options, engine);
    let mut service = Progress::new(lookup, deliver);
    service.remote = options
        .remote_receiver
        .clone()
        .map(|receiver| receiver as Arc<dyn super::remote_dispatch::return_channel::ReturnChannel>);
    if let Some(state) = options.replacements.clone() {
        service = service.with_replacements(state);
    }
    let service = Arc::new(service);
    options = options.handler("agents.reportProgress", move |_, params| {
        let service = service.clone();
        async move {
            service
                .report(params, chrono::Utc::now().timestamp_millis())
                .await
        }
    });
    options
}

pub(super) fn engine_delivery(
    options: &crate::Options,
    engine: claudemon::daemon::embedded::EmbeddedClient,
) -> Deliver {
    let tracker = options.message_tracker.clone();
    let requests = options
        .workflow_runtime
        .as_ref()
        .map(|workflow| workflow.requests.clone());
    let deliver: Deliver = Arc::new(move |id, text| {
        let engine = engine.clone();
        let tracker = tracker.clone();
        let requests = requests.clone();
        Box::pin(async move {
            if let Some(tracker) = tracker {
                let delivery = super::manager_replacements::messages::send_engine(
                    &engine,
                    &tracker,
                    &id,
                    &text,
                    &[],
                    None,
                    false,
                    requests.as_deref(),
                )
                .await?;
                return match delivery.outcome {
                    super::manager_replacements::SendOutcome::Accepted => Ok(()),
                    super::manager_replacements::SendOutcome::Rejected { reason, .. } => {
                        Err(anyhow!("progress delivery rejected: {reason}"))
                    }
                    super::manager_replacements::SendOutcome::Uncertain(reason) => Err(anyhow!(
                        "progress delivery outcome is unknown; do not retry automatically: {reason}"
                    )),
                };
            }
            let id = url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
            engine
                .request(claudemon::daemon::embedded::Command::Request {
                    method: "POST".into(),
                    path: format!("/sessions/{id}/message"),
                    payload: Some(json!({"text":text})),
                })
                .await?;
            Ok(())
        })
    });
    deliver
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    use crate::services::remote_dispatch::{Kind, return_channel::TestReturns};
    #[tokio::test]
    async fn remote_progress_uses_verified_worker_mapping_and_retains_budget() {
        let mut service = Progress::new(
            Arc::new(|id| {
                (id=="worker").then(||json!({"sessionId":"worker","parentSessionId":"foreign-parent","cwd":"/execution/repo"}))
            }),
            Arc::new(|_, _| panic!("remote progress must never reach local sender")),
        );
        let returns = TestReturns::new();
        service.remote = Some(returns.clone());
        let result = service
            .report(
                json!({"callerSessionId":"worker","note":"phase done"}),
                1000,
            )
            .await
            .unwrap();
        assert_eq!(
            result,
            json!({"queuedTo":"dispatching-hub","route":"remote-dispatch"})
        );
        assert!(
            service
                .report(
                    json!({"callerSessionId":"worker","note":"phase done"}),
                    61000
                )
                .await
                .is_err()
        );
        assert_eq!(returns.entries.lock().unwrap().len(), 1);
        assert_eq!(returns.entries.lock().unwrap()[0].0, Kind::Progress);
        assert!(
            service
                .report(
                    json!({"callerSessionId":"forged","note":"phase done"}),
                    1000
                )
                .await
                .is_err()
        );
    }
}

//! Host-owned job specifications, scheduling and invocation history.
use crate::{Caller, Handle, Options, client::Client, protocol::Event};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Datelike, Days, Local, NaiveTime, Offset, TimeZone, Timelike, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Trigger {
    pub kind: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub every_minutes: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub at: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<i32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub once: String,
}
fn is_zero(n: &i64) -> bool {
    *n == 0
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub action: Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub proposed_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}
impl Job {
    pub fn is_proposal(&self) -> bool {
        !self.proposed_by.trim().is_empty()
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Run {
    pub job_id: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub finished_at: i64,
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}
pub fn validate(job: &Job) -> Result<()> {
    check_keys(&job.action, &["kind", "spawn", "call", "shell"])?;
    if job.name.trim().is_empty() {
        bail!("job needs a name");
    }
    match job.trigger.kind.as_str() {
        "interval" => {
            if job.trigger.every_minutes < 1 {
                bail!("interval trigger needs everyMinutes >= 1");
            }
            if chrono::Duration::try_minutes(job.trigger.every_minutes).is_none() {
                bail!("interval exceeds supported duration");
            }
        }
        "daily" => {
            NaiveTime::parse_from_str(&job.trigger.at, "%H:%M")
                .map_err(|e| anyhow!("daily trigger needs at as HH:MM: {e}"))?;
            if job.trigger.days.iter().any(|d| !(0..=6).contains(d)) {
                bail!("daily trigger day out of range");
            }
        }
        "once" => {
            DateTime::parse_from_rfc3339(&job.trigger.once)
                .map_err(|e| anyhow!("once trigger needs an RFC3339 time: {e}"))?;
        }
        "manual" => (),
        _ => bail!("unknown trigger kind {:?}", job.trigger.kind),
    }
    match job.action["kind"].as_str().unwrap_or("") {
        "spawn" => {
            let spawn = &job.action["spawn"];
            check_keys(
                spawn,
                &[
                    "cwd",
                    "prompt",
                    "context",
                    "provider",
                    "model",
                    "effort",
                    "permissionMode",
                ],
            )?;
            for key in [
                "cwd",
                "prompt",
                "provider",
                "model",
                "effort",
                "permissionMode",
            ] {
                optional_type(spawn, key, Value::is_string, "a string")?;
            }
            if string(spawn, "cwd").trim().is_empty() || string(spawn, "prompt").trim().is_empty() {
                bail!("spawn action needs cwd and prompt");
            }
            if let Some(steps) = spawn.get("context").filter(|v| !v.is_null()) {
                let steps = steps
                    .as_array()
                    .ok_or_else(|| anyhow!("context must be an array"))?;
                if steps.len() > 4 {
                    bail!("at most 4 context steps (got {})", steps.len());
                }
                for (index, step) in steps.iter().enumerate() {
                    check_keys(
                        step,
                        &[
                            "kind",
                            "shell",
                            "call",
                            "skipIfEmpty",
                            "skipUnlessMatch",
                            "ignoreExitCode",
                        ],
                    )?;
                    for key in ["skipIfEmpty", "ignoreExitCode"] {
                        optional_type(step, key, Value::is_boolean, "a boolean")?;
                    }
                    optional_type(step, "skipUnlessMatch", Value::is_string, "a string")?;
                    validate_step(step).map_err(|e| anyhow!("context step {}: {e}", index + 1))?;
                    if !string(step, "skipUnlessMatch").is_empty() {
                        regex::Regex::new(string(step, "skipUnlessMatch")).map_err(|e| {
                            anyhow!(
                                "context step {} has an invalid skipUnlessMatch: {e}",
                                index + 1
                            )
                        })?;
                    }
                }
            }
        }
        "call" | "shell" => validate_step(&job.action)?,
        _ => bail!("unknown action kind {:?}", job.action["kind"]),
    }
    Ok(())
}
fn validate_step(action: &Value) -> Result<()> {
    match string(action, "kind") {
        "shell" => {
            check_keys(&action["shell"], &["command", "cwd"])?;
            optional_type(&action["shell"], "cwd", Value::is_string, "a string")?;
            if string(&action["shell"], "command").trim().is_empty() {
                bail!("shell action needs a command");
            }
        }
        "call" => {
            check_keys(&action["call"], &["method", "params"])?;
            let method = string(&action["call"], "method");
            if method.trim().is_empty() {
                bail!("call action needs a method");
            }
            if method.starts_with("jobs.") || method.starts_with("hub:") {
                bail!("call may not target {method:?}");
            }
        }
        _ => bail!("unknown context step kind"),
    }
    Ok(())
}
fn optional_type(
    value: &Value,
    key: &str,
    predicate: fn(&Value) -> bool,
    kind: &str,
) -> Result<()> {
    if value
        .get(key)
        .is_some_and(|value| !value.is_null() && !predicate(value))
    {
        bail!("{key} must be {kind}");
    }
    Ok(())
}
fn check_keys(value: &Value, known: &[&str]) -> Result<()> {
    if let Some(map) = value.as_object() {
        let mut seen = BTreeSet::new();
        for key in map.keys() {
            if !seen.insert(key.to_ascii_lowercase()) {
                bail!("ambiguous case-variant job field {key:?}");
            }
            if known
                .iter()
                .any(|canonical| canonical.eq_ignore_ascii_case(key) && *canonical != key)
            {
                bail!("non-canonical job field {key:?}");
            }
        }
    }
    Ok(())
}
fn check_job_keys(value: &Value) -> Result<()> {
    check_keys(
        value,
        &[
            "id",
            "name",
            "enabled",
            "trigger",
            "action",
            "proposedBy",
            "createdAt",
            "updatedAt",
        ],
    )?;
    check_keys(
        &value["trigger"],
        &["kind", "everyMinutes", "at", "days", "once"],
    )
}
fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

pub fn next_run<T: TimeZone>(
    trigger: &Trigger,
    after: DateTime<Utc>,
    zone: &T,
) -> Option<DateTime<Utc>> {
    match trigger.kind.as_str() {
        "interval" if trigger.every_minutes >= 1 => {
            after.checked_add_signed(chrono::Duration::try_minutes(trigger.every_minutes)?)
        }
        "once" => DateTime::parse_from_rfc3339(&trigger.once)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
        "daily" => {
            let time = NaiveTime::parse_from_str(&trigger.at, "%H:%M").ok()?;
            let base = after.with_timezone(zone).date_naive();
            for d in 0..8 {
                let date = base.checked_add_days(Days::new(d))?;
                let wall = date.and_hms_opt(time.hour(), time.minute(), 0)?;
                // Same two-lookup normalization as Go time.Date, including
                // wall times in a DST gap or repeated hour.
                let first = zone.offset_from_utc_datetime(&wall).fix().local_minus_utc();
                let candidate = wall.checked_sub_signed(chrono::Duration::seconds(first as i64))?;
                let offset = zone
                    .offset_from_utc_datetime(&candidate)
                    .fix()
                    .local_minus_utc();
                let utc = wall
                    .checked_sub_signed(chrono::Duration::seconds(offset as i64))?
                    .and_utc();
                if utc > after
                    && (trigger.days.is_empty()
                        || trigger.days.contains(
                            &(utc.with_timezone(zone).weekday().num_days_from_sunday() as i32),
                        ))
                {
                    return Some(utc);
                }
            }
            None
        }
        _ => None,
    }
}
pub fn fill_prompt(prompt: &str, outputs: &[String]) -> String {
    let mut filled = prompt.to_owned();
    let mut used = false;
    for (i, output) in outputs.iter().enumerate() {
        let marker = format!("{{{{output.{}}}}}", i + 1);
        if filled.contains(&marker) {
            filled = filled.replace(&marker, output);
            used = true;
        }
    }
    if let Some(last) = outputs.last()
        && filled.contains("{{output}}")
    {
        filled = filled.replace("{{output}}", last);
        used = true;
    }
    if !used {
        for (i, output) in outputs.iter().enumerate() {
            filled.push_str(&format!(
                "\n\n--- context step {} ---\n```\n{}\n```",
                i + 1,
                output
            ));
        }
    }
    filled
}
pub fn empty_output(output: &str) -> bool {
    matches!(output.trim(), "" | "{}" | "[]" | "null" | "\"\"")
}
fn head(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.into()
    } else {
        format!("{}…", String::from_utf8_lossy(&text.as_bytes()[..limit]))
    }
}
fn tail(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.len() <= limit {
        text.into()
    } else {
        format!(
            "…{}",
            String::from_utf8_lossy(&text.as_bytes()[text.len() - limit..])
        )
    }
}
fn elide(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.into()
    } else {
        let half = limit / 2;
        format!(
            "{}\n\n… {} characters elided …\n\n{}",
            String::from_utf8_lossy(&text.as_bytes()[..half]),
            text.len() - 2 * half,
            String::from_utf8_lossy(&text.as_bytes()[text.len() - half..])
        )
    }
}

struct State {
    jobs: Vec<Job>,
    next: BTreeMap<String, DateTime<Utc>>,
    running: BTreeSet<String>,
    history: BTreeMap<String, Vec<Run>>,
    hash: Option<Vec<u8>>,
}
pub struct Service {
    path: PathBuf,
    history_path: PathBuf,
    hub: Handle,
    state: Mutex<State>,
    queue: mpsc::UnboundedSender<Job>,
}
impl Service {
    pub fn open(path: PathBuf, hub: Handle) -> (Arc<Self>, mpsc::UnboundedReceiver<Job>) {
        let history_path = path.with_file_name("jobs-history.json");
        let history = std::fs::read(&history_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| serde_json::from_value(v["runs"].clone()).ok())
            .unwrap_or_default();
        let (queue, receiver) = mpsc::unbounded_channel();
        let service = Arc::new(Self {
            path,
            history_path,
            hub,
            state: Mutex::new(State {
                jobs: vec![],
                next: BTreeMap::new(),
                running: BTreeSet::new(),
                history,
                hash: None,
            }),
            queue,
        });
        service.reload(&mut service.state.lock().unwrap(), Utc::now());
        (service, receiver)
    }
    fn reload(&self, state: &mut State, now: DateTime<Utc>) {
        let raw = match std::fs::read(&self.path) {
            Ok(raw) => raw,
            Err(_) => return,
        };
        let hash = Sha256::digest(&raw).to_vec();
        if state.hash.as_ref() == Some(&hash) {
            return;
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct File {
            jobs: Vec<Job>,
        }
        let parsed = serde_json::from_slice::<Value>(&raw);
        if let Ok(value) = &parsed
            && let Some(rows) = value["jobs"].as_array()
        {
            for row in rows {
                if let Err(error) = check_job_keys(row) {
                    eprintln!("jobs: ambiguous spec; retaining last good schedule: {error}");
                    return;
                }
            }
        }
        let file = match serde_json::from_slice::<File>(&raw) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("jobs: unreadable spec; retaining last good schedule: {error}");
                return;
            }
        };
        let mut jobs = Vec::new();
        let mut ids = BTreeSet::new();
        let mut filled = false;
        for mut job in file.jobs {
            if let Err(error) = validate(&job) {
                eprintln!("jobs: invalid row {:?} skipped: {error}", job.name);
                continue;
            }
            if job.id.is_empty() || ids.contains(&job.id) {
                job.id = uuid::Uuid::new_v4().simple().to_string()[..24].into();
                filled = true;
            }
            ids.insert(job.id.clone());
            if job.created_at == 0 {
                job.created_at = now.timestamp_millis();
                filled = true;
            }
            if job.updated_at == 0 {
                job.updated_at = now.timestamp_millis();
                filled = true;
            }
            let unchanged = state.jobs.iter().any(|old| {
                old.id == job.id
                    && old.enabled == job.enabled
                    && old.proposed_by == job.proposed_by
                    && old.trigger == job.trigger
            });
            if !unchanged {
                reschedule(state, &job, now);
            }
            jobs.push(job);
        }
        state.jobs = jobs;
        state.next.retain(|id, _| ids.contains(id));
        state.hash = Some(hash);
        if filled {
            self.persist(state);
        }
    }
    fn persist(&self, state: &mut State) {
        let value = json!({"jobs":state.jobs});
        if let Err(error) = super::atomic_json(&self.path, &value, true) {
            eprintln!("jobs: failed to persist spec: {error}");
        } else if let Ok(raw) = std::fs::read(&self.path) {
            state.hash = Some(Sha256::digest(raw).to_vec());
        }
    }
    fn record(&self, state: &mut State, run: Run) {
        let rows = state.history.entry(run.job_id.clone()).or_default();
        rows.insert(0, run);
        rows.truncate(30);
        if let Err(error) =
            super::atomic_json(&self.history_path, &json!({"runs":state.history}), true)
        {
            eprintln!("jobs: failed to persist history: {error}");
        }
    }
    /// Read the scheduler's owned timing state without triggering file writes or
    /// a bus authorization round trip. An unreadable first load is unknown.
    pub fn schedule(&self) -> Result<Vec<super::quiescence::JobInfo>> {
        let state = self.state.lock().unwrap();
        if state.hash.is_none() {
            match std::fs::metadata(&self.path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
                Ok(_) => bail!("job specification has not produced a valid scheduler snapshot"),
            }
        }
        let mut rows: Vec<_> = state
            .jobs
            .iter()
            .map(|job| super::quiescence::JobInfo {
                id: job.id.clone(),
                name: job.name.clone(),
                action_kind: string(&job.action, "kind").into(),
                next_run_ms: state.next.get(&job.id).map(|at| at.timestamp_millis()),
                running: state.running.contains(&job.id),
            })
            .collect();
        for id in &state.running {
            if !rows.iter().any(|row| &row.id == id) {
                rows.push(super::quiescence::JobInfo {
                    id: id.clone(),
                    name: id.clone(),
                    action_kind: "unknown".into(),
                    next_run_ms: None,
                    running: true,
                });
            }
        }
        Ok(rows)
    }
    pub fn call(&self, caller: &Caller, method: &str, params: Value) -> Result<Value> {
        if !caller.authenticated_host || !caller.trusted || caller.scope != "operator" {
            bail!("{method} requires the server owner");
        }
        let mut state = self.state.lock().unwrap();
        let now = Utc::now();
        if method != "jobs.history" {
            self.reload(&mut state, now);
        }
        match method {
            "jobs.list" => {
                let jobs: Vec<_> = state
                    .jobs
                    .iter()
                    .map(|j| {
                        let mut v = json!(j);
                        if let Some(at) = state.next.get(&j.id) {
                            v["nextRunAt"] = json!(at.timestamp_millis());
                        }
                        if state.running.contains(&j.id) {
                            v["running"] = json!(true);
                        }
                        if let Some(run) = state.history.get(&j.id).and_then(|r| r.first()) {
                            v["lastRun"] = json!(run);
                        }
                        v
                    })
                    .collect();
                Ok(json!({"jobs":jobs}))
            }
            "jobs.upsert" | "jobs.propose" => {
                check_job_keys(&params)?;
                let mut job: Job = serde_json::from_value(params)?;
                let proposal = method == "jobs.propose";
                if proposal {
                    job.enabled = false;
                    job.id.clear();
                    if job.proposed_by.trim().is_empty() {
                        job.proposed_by = "an agent".into();
                    }
                    if state.jobs.iter().filter(|j| j.is_proposal()).count() >= 20 {
                        bail!(
                            "20 job proposals are already waiting for review — approve or remove some first"
                        );
                    }
                }
                validate(&job)?;
                job.updated_at = now.timestamp_millis();
                if job.id.is_empty() {
                    job.id = uuid::Uuid::new_v4().simple().to_string()[..24].into();
                    job.created_at = job.updated_at;
                    state.jobs.push(job.clone());
                } else {
                    let index = state
                        .jobs
                        .iter()
                        .position(|j| j.id == job.id)
                        .ok_or_else(|| anyhow!("no job {:?}", job.id))?;
                    job.created_at = state.jobs[index].created_at;
                    state.jobs[index] = job.clone();
                }
                reschedule(&mut state, &job, now);
                self.persist(&mut state);
                if proposal {
                    let _=self.hub.publish(Event::new("notify.post","jobs",json!({"title":format!("Job proposed: {}",job.name),"body":format!("{} suggested a job. It won't run until you approve it — click to read the trigger and the action.",job.proposed_by),"level":"info","key":format!("job-proposal-{}",job.id),"paneType":"settings","paneSection":"jobs"})));
                }
                Ok(json!(job))
            }
            "jobs.remove" | "jobs.run" | "jobs.history" => {
                let id = string(&params, "id");
                if id.is_empty() {
                    bail!("{method} requires {{id}}");
                }
                if method == "jobs.history" {
                    return Ok(json!({"runs":state.history.get(id).cloned().unwrap_or_default()}));
                }
                if method == "jobs.remove" {
                    state.jobs.retain(|j| j.id != id);
                    state.next.remove(id);
                    state.history.remove(id);
                    self.persist(&mut state);
                    let _ = super::atomic_json(
                        &self.history_path,
                        &json!({"runs":state.history}),
                        true,
                    );
                    return Ok(json!({"ok":true}));
                }
                let job = state
                    .jobs
                    .iter()
                    .find(|j| j.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow!("no job {id:?}"))?;
                if job.is_proposal() {
                    bail!(
                        "job {:?} is an unapproved proposal — approve it in Settings → Jobs first",
                        job.name
                    );
                }
                if !state.running.insert(id.into()) {
                    return Ok(json!({"started":false,"reason":"already running"}));
                }
                if self.queue.send(job).is_err() {
                    state.running.remove(id);
                    bail!("job executor is stopped; run was not started");
                }
                Ok(json!({"started":true}))
            }
            _ => bail!("unknown job method"),
        }
    }
    fn due(&self, now: DateTime<Utc>) -> Vec<Job> {
        let mut state = self.state.lock().unwrap();
        self.reload(&mut state, now);
        let mut due = Vec::new();
        let jobs = state.jobs.clone();
        for mut job in jobs {
            if !job.enabled
                || job.is_proposal()
                || !state.next.get(&job.id).is_some_and(|at| *at <= now)
            {
                continue;
            }
            if job.trigger.kind == "once" {
                job.enabled = false;
                if let Some(stored) = state.jobs.iter_mut().find(|j| j.id == job.id) {
                    stored.enabled = false;
                }
                state.next.remove(&job.id);
                self.persist(&mut state);
            } else {
                reschedule(&mut state, &job, now);
            }
            if !state.running.insert(job.id.clone()) {
                self.record(
                    &mut state,
                    Run {
                        job_id: job.id,
                        started_at: now.timestamp_millis(),
                        finished_at: now.timestamp_millis(),
                        status: "skipped".into(),
                        detail: "previous run still in progress".into(),
                    },
                );
                continue;
            }
            due.push(job);
        }
        due
    }
    async fn execute(self: Arc<Self>, job: Job, client: Client) {
        let started = Utc::now();
        let result =
            tokio::time::timeout(Duration::from_secs(15 * 60), perform(&job, &client)).await;
        let (status, detail) = match result {
            Ok(Ok(detail)) => ("ok", head(&detail, 2000)),
            Ok(Err(error)) if error.downcast_ref::<Skip>().is_some() => {
                ("skipped", head(&error.to_string(), 2000))
            }
            Ok(Err(error)) => ("error", head(&error.to_string(), 2000)),
            Err(_) => ("error", "job timed out after 15 minutes".into()),
        };
        if status == "error" {
            let _=self.hub.publish_wait(Event::new("notify.post","jobs",json!({"title":format!("Job failed: {}",job.name),"body":detail,"level":"error","key":format!("job-{}",job.id)}))).await;
        }
        let mut state = self.state.lock().unwrap();
        state.running.remove(&job.id);
        self.record(
            &mut state,
            Run {
                job_id: job.id,
                started_at: started.timestamp_millis(),
                finished_at: Utc::now().timestamp_millis(),
                status: status.into(),
                detail,
            },
        );
    }
    async fn run(self: Arc<Self>, mut receiver: mpsc::UnboundedReceiver<Job>) -> Result<()> {
        self.hub.ready().await?;
        let client = Client::connect_service(&self.hub).await?;
        let mut runs = tokio::task::JoinSet::new();
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.tick().await;
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let jobs = tokio::select! {_=tick.tick()=>self.due(Utc::now()),Some(job)=receiver.recv()=>vec![job],Some(_)=runs.join_next()=>continue};
            for job in jobs {
                let service = self.clone();
                let client = client.clone();
                runs.spawn(async move {
                    service.execute(job, client).await;
                });
            }
        }
    }
}
fn reschedule(state: &mut State, job: &Job, now: DateTime<Utc>) {
    state.next.remove(&job.id);
    if job.enabled && !job.is_proposal() {
        if let Some(at) = next_run(&job.trigger, now, &Local) {
            state.next.insert(job.id.clone(), at);
        }
    }
}
#[derive(Debug)]
struct Skip(String);
impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Skip {}
trait Runner: Send + Sync {
    fn invoke<'a>(&'a self, method: &'a str, params: Value) -> BoxFuture<'a, Result<Value>>;
    fn shell<'a>(&'a self, action: &'a Value) -> BoxFuture<'a, Result<(String, bool)>>;
}
impl Runner for Client {
    fn invoke<'a>(&'a self, method: &'a str, params: Value) -> BoxFuture<'a, Result<Value>> {
        Box::pin(self.call_with_timeout(method, params, Duration::from_secs(900)))
    }
    fn shell<'a>(&'a self, action: &'a Value) -> BoxFuture<'a, Result<(String, bool)>> {
        Box::pin(shell(action))
    }
}
async fn perform(job: &Job, client: &impl Runner) -> Result<String> {
    let action = &job.action;
    match string(action, "kind") {
        "shell" => {
            let (output, ok) = client.shell(&action["shell"]).await?;
            if !ok {
                bail!("shell exited nonzero — output: {}", tail(&output, 1000));
            }
            Ok(tail(&output, 2000))
        }
        "call" => Ok(client
            .invoke(
                string(&action["call"], "method"),
                action["call"].get("params").cloned().unwrap_or(json!({})),
            )
            .await?
            .to_string()),
        "spawn" => {
            let spawn = &action["spawn"];
            let mut outputs = Vec::new();
            if let Some(steps) = spawn["context"].as_array() {
                for step in steps {
                    let output = match string(step, "kind") {
                        "shell" => {
                            let (out, ok) = client.shell(&step["shell"]).await?;
                            if !ok && step["ignoreExitCode"] != true {
                                bail!("shell exited nonzero — output: {}", tail(&out, 1000));
                            }
                            out
                        }
                        "call" => client
                            .invoke(
                                string(&step["call"], "method"),
                                step["call"].get("params").cloned().unwrap_or(json!({})),
                            )
                            .await?
                            .to_string(),
                        _ => bail!("unknown context step kind"),
                    };
                    let output = output.trim();
                    if step["skipIfEmpty"] == true && empty_output(output) {
                        return Err(Skip("no output — nothing to send an agent".into()).into());
                    }
                    let pattern = string(step, "skipUnlessMatch");
                    if !pattern.is_empty() && !regex::Regex::new(pattern)?.is_match(output) {
                        return Err(Skip(format!("output did not match {pattern}")).into());
                    }
                    outputs.push(elide(output, 12000));
                }
            }
            let prompt = fill_prompt(string(spawn, "prompt"), &outputs);
            let mut params = json!({"cwd":spawn["cwd"],"label":job.name});
            for key in ["provider", "model", "effort", "permissionMode"] {
                if !string(spawn, key).is_empty() {
                    params[key] = spawn[key].clone();
                }
            }
            let result = client.invoke("agents.spawn", params).await?;
            let id = result["sessionId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("agents.spawn returned no sessionId"))?;
            client
                .invoke("agents.sendMessage", json!({"sessionId":id,"text":prompt}))
                .await
                .map_err(|e| anyhow!("spawned {id} but prompt failed: {e}"))?;
            Ok(format!("spawned {id}"))
        }
        _ => bail!("unknown action kind"),
    }
}
async fn shell(action: &Value) -> Result<(String, bool)> {
    let mut command = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", string(action, "command")]);
        c
    } else {
        let mut c = tokio::process::Command::new("/bin/sh");
        c.args(["-c", string(action, "command")]);
        c
    };
    if !string(action, "cwd").is_empty() {
        command.current_dir(string(action, "cwd"));
    }
    let output = super::owned_process::capture_combined(
        &mut command,
        64 * 1024 * 1024,
        Duration::from_secs(15 * 60),
    )
    .await?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.success(),
    ))
}
pub(crate) fn install(
    mut options: Options,
    hub: Handle,
) -> (Options, Option<tokio::task::JoinHandle<Result<()>>>) {
    let Some(path) = options.jobs_file.clone() else {
        return (options, None);
    };
    let (service, receiver) = Service::open(path, hub);
    options.jobs_service = Some(service.clone());
    for method in [
        "jobs.list",
        "jobs.upsert",
        "jobs.propose",
        "jobs.remove",
        "jobs.run",
        "jobs.history",
    ] {
        let service = service.clone();
        options = options.handler(method, move |caller, params| {
            let service = service.clone();
            async move {
                tokio::task::spawn_blocking(move || service.call(&caller, method, params)).await?
            }
        });
    }
    (options, Some(tokio::spawn(service.run(receiver))))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Recording {
        calls: Mutex<Vec<(String, Value)>>,
        output: String,
    }
    impl Runner for Recording {
        fn invoke<'a>(&'a self, method: &'a str, params: Value) -> BoxFuture<'a, Result<Value>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push((method.into(), params));
                Ok(match method {
                    "context.read" => json!({"count":1}),
                    "agents.spawn" => json!({"sessionId":"child"}),
                    _ => json!({"ok":true}),
                })
            })
        }
        fn shell<'a>(&'a self, action: &'a Value) -> BoxFuture<'a, Result<(String, bool)>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(("shell".into(), action.clone()));
                Ok((self.output.clone(), true))
            })
        }
    }
    #[tokio::test]
    async fn spawn_collects_all_context_before_launch_and_delivers_the_prompt_once() {
        let runner = Recording {
            calls: Mutex::new(vec![]),
            output: " diff \n".into(),
        };
        let job:Job=serde_json::from_value(json!({"name":"review","trigger":{"kind":"manual"},"action":{"kind":"spawn","spawn":{"cwd":"/project","provider":"codex","model":"chosen-model","prompt":"Build {{output.1}} / {{output}}","context":[{"kind":"shell","shell":{"command":"git diff"},"skipUnlessMatch":"diff"},{"kind":"call","call":{"method":"context.read"}}]}}})).unwrap();
        validate(&job).unwrap();
        assert_eq!(perform(&job, &runner).await.unwrap(), "spawned child");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "shell",
                "context.read",
                "agents.spawn",
                "agents.sendMessage"
            ]
        );
        assert_eq!(
            calls[2].1,
            json!({"cwd":"/project","label":"review","provider":"codex","model":"chosen-model"})
        );
        assert_eq!(
            calls[3].1,
            json!({"sessionId":"child","text":"Build diff / {\"count\":1}"})
        );
    }
    #[tokio::test]
    async fn context_veto_never_invokes_spawn() {
        let runner = Recording {
            calls: Mutex::new(vec![]),
            output: "no changes".into(),
        };
        let job:Job=serde_json::from_value(json!({"name":"review","trigger":{"kind":"manual"},"action":{"kind":"spawn","spawn":{"cwd":"/project","prompt":"task","context":[{"kind":"shell","shell":{"command":"inspect"},"skipUnlessMatch":"changed files"}]}}})).unwrap();
        assert!(
            perform(&job, &runner)
                .await
                .unwrap_err()
                .downcast_ref::<Skip>()
                .is_some()
        );
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }
    fn owner() -> Caller {
        Caller {
            call_id: 0,
            activity_seq: 0,
            federated: false,
            connection_id: 1,
            authenticated_host: true,
            trusted: true,
            scope: "operator".into(),
            plugin_id: String::new(),
            token_id: String::new(),
        }
    }
    #[tokio::test]
    async fn once_disarms_before_execution_and_interval_overlap_records_a_skip() {
        let directory = tempfile::tempdir().unwrap();
        let hub = crate::Hub::start(Options::default()).unwrap();
        hub.ready().await.unwrap();
        let (service, _) = Service::open(directory.path().join("jobs.json"), hub.handle());
        let once=service.call(&owner(),"jobs.upsert",json!({"name":"once","enabled":true,"trigger":{"kind":"once","once":"2020-01-01T00:00:00Z"},"action":{"kind":"call","call":{"method":"fixture"}}})).unwrap();
        assert_eq!(service.due(Utc::now()).len(), 1);
        assert!(service.due(Utc::now()).is_empty());
        assert!(
            !service
                .state
                .lock()
                .unwrap()
                .jobs
                .iter()
                .find(|j| j.id == once["id"])
                .unwrap()
                .enabled
        );
        let interval=service.call(&owner(),"jobs.upsert",json!({"name":"interval","enabled":true,"trigger":{"kind":"interval","everyMinutes":1},"action":{"kind":"call","call":{"method":"fixture"}}})).unwrap();
        let due = Utc::now() + chrono::Duration::seconds(61);
        assert_eq!(service.due(due).len(), 1);
        assert!(service.due(due + chrono::Duration::seconds(61)).is_empty());
        assert_eq!(
            service
                .call(&owner(), "jobs.history", json!({"id":interval["id"]}))
                .unwrap()["runs"][0]["status"],
            "skipped"
        );
        hub.shutdown().unwrap();
    }
}

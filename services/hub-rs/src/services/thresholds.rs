//! One-shot host observations. Cumulative usage never substitutes for a
//! correlated runtime context sample; epoch identity survives JSON precision.
use super::{progress::Deliver, task_store::OwnerLookup};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}
fn provider(value: &str) -> String {
    value.trim().to_lowercase()
}
fn ended(row: &Value) -> bool {
    row["status"] == "ended" || row["mode"] == "stopped"
}
fn unsupported(provider: &str) -> bool {
    matches!(provider, "opencode" | "pi")
}
fn pct(value: f64) -> String {
    format!("{:.1}", (value.clamp(0., 100.) * 10.).round() / 10.)
}
fn grouped(value: f64) -> String {
    let digits = (value as i64).to_string();
    let (sign, digits) = digits
        .strip_prefix('-')
        .map(|v| ("-", v))
        .unwrap_or(("", &digits));
    let mut result = sign.to_owned();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}
#[derive(Clone, Debug)]
struct Health {
    used: f64,
    window: f64,
    percent: f64,
    observed: String,
    epoch: String,
    provider: String,
}
fn health(row: &Value, now: i64) -> Option<Health> {
    let raw = row.get("status_line").filter(|v| !v.is_null());
    let (sample, used, window, percent, source, observed) = if let Some(raw) = raw {
        (
            raw.get("context_health")?,
            "used_tokens",
            "window_tokens",
            "used_pct",
            "window_source",
            "observed_at",
        )
    } else {
        (
            row["statusLine"].get("contextHealth")?,
            "usedTokens",
            "windowTokens",
            "usedPct",
            "windowSource",
            "observedAt",
        )
    };
    let sample_provider = provider(text(sample, "provider"));
    let owner = provider(text(row, "provider"));
    let used = sample[used].as_f64()?;
    let window = sample[window].as_f64()?;
    let percent = sample[percent].as_f64()?;
    let epoch = if let Some(epoch) = sample["epoch"].as_str() {
        if epoch.starts_with('0')
            || epoch.is_empty()
            || !epoch.bytes().all(|c| c.is_ascii_digit())
            || epoch.parse::<u64>().ok()? == 0
        {
            return None;
        }
        epoch.to_owned()
    } else {
        let epoch = sample["epoch"].as_u64()?;
        if epoch == 0 || epoch > 9_007_199_254_740_991 {
            return None;
        }
        epoch.to_string()
    };
    let observed = text(sample, observed);
    let at = chrono::DateTime::parse_from_rfc3339(observed)
        .ok()?
        .timestamp_millis();
    if text(sample, source) != "runtime"
        || sample_provider.is_empty()
        || (!owner.is_empty() && sample_provider != owner)
        || window <= 0.
        || used < 0.
        || used > window
        || !percent.is_finite()
        || !(0. ..=100.).contains(&percent)
        || (used / window * 100. - percent).abs() > 0.01
        || at > now.saturating_add(5000)
        || now.saturating_sub(at) > 120_000
    {
        return None;
    }
    Some(Health {
        used,
        window,
        percent,
        observed: observed.into(),
        epoch,
        provider: sample_provider,
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub id: String,
    pub session_id: String,
    pub watcher_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_used_pct: Option<f64>,
    armed_at: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    context_provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_epoch: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    state: String,
}
fn pick(row: &Value, raw: &str, camel: &str) -> f64 {
    row["status_line"][raw]
        .as_f64()
        .or_else(|| row["statusLine"][camel].as_f64())
        .unwrap_or(0.)
}
impl Watch {
    fn crossing(&mut self, row: &Value, now: i64) -> Option<String> {
        if let Some(threshold) = self.context_used_pct {
            let owner = provider(text(row, "provider"));
            let display = if owner.is_empty() { "unknown" } else { &owner };
            if !self.context_provider.is_empty()
                && !owner.is_empty()
                && owner != self.context_provider
            {
                return Some(format!(
                    "monitoring invalidated: contextUsedPct {}% watch crossed a provider/session boundary ({} → {display}); re-arm after a confirmed sample",
                    pct(threshold),
                    self.context_provider
                ));
            }
            if unsupported(&owner) {
                return Some(format!(
                    "monitoring invalidated: contextUsedPct {}% is unavailable for provider {display}; re-arm only on a provider with runtime context telemetry",
                    pct(threshold)
                ));
            }
            let h = health(row, now)?;
            if let Some(epoch) = &self.context_epoch {
                if *epoch != h.epoch {
                    return Some(format!(
                        "monitoring invalidated: contextUsedPct {}% watch crossed telemetry epoch {epoch} → {}; re-arm after the confirmed {} sample",
                        pct(threshold),
                        h.epoch,
                        h.provider
                    ));
                }
            }
            if self.context_provider.is_empty() {
                self.context_provider = h.provider.clone();
            }
            if self.context_epoch.is_none() {
                self.context_epoch = Some(h.epoch.clone());
            }
            return (h.percent>=threshold).then(||format!("contextUsedPct active context {}% ≥ {}% ({} / {} tokens; runtime-confirmed by {}; observed {}; epoch {})",pct(h.percent),pct(threshold),grouped(h.used),grouped(h.window),h.provider,h.observed,h.epoch));
        }
        if let Some(threshold) = self.tokens {
            let tokens = pick(row, "total_input_tokens", "totalInputTokens")
                + pick(row, "total_output_tokens", "totalOutputTokens");
            if tokens >= threshold {
                return Some(format!(
                    "tokens {} ≥ {}",
                    grouped(tokens),
                    grouped(threshold)
                ));
            }
        }
        if let Some(threshold) = self.usd {
            let cost = row["usage"]["costUSD"]
                .as_f64()
                .unwrap_or_else(|| pick(row, "cost_usd", "costUSD"));
            if cost >= threshold {
                return Some(format!("cost ${cost:.2} ≥ ${threshold:.2}"));
            }
        }
        if let Some(threshold) = self.idle_seconds {
            let last = row["lastActivity"]
                .as_i64()
                .filter(|v| *v > 0)
                .unwrap_or(now);
            let elapsed = now.saturating_sub(last) as f64 / 1000.;
            if elapsed >= threshold {
                let secs = elapsed.round() as i64;
                let threshold = threshold as i64;
                let state = text(row, "ambientState");
                return Some(if matches!(state, "" | "idle") {
                    format!("idle for {secs}s ≥ {threshold}s")
                } else {
                    format!("no activity for {secs}s ≥ {threshold}s (still reports {state})")
                });
            }
        }
        None
    }
}
#[derive(Default)]
struct State {
    sequence: u64,
    watches: BTreeMap<String, Watch>,
}
pub struct Thresholds {
    state: Mutex<State>,
    lookup: OwnerLookup,
    deliver: Deliver,
    stopped: tokio::sync::watch::Sender<bool>,
    replacements: Option<Arc<super::manager_replacements::ReplacementState>>,
}
impl Thresholds {
    pub fn new(lookup: OwnerLookup, deliver: Deliver) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            lookup,
            deliver,
            stopped: tokio::sync::watch::channel(false).0,
            replacements: None,
        })
    }
    pub fn arm(&self, params: &Value, now: i64) -> Result<Value> {
        let id = text(params, "sessionId");
        if id.is_empty() {
            bail!("agents.notifyWhen requires {{ sessionId }}");
        }
        let numeric = |name: &str| -> Result<Option<f64>> {
            let Some(value) = params.get(name).filter(|v| !v.is_null()) else {
                return Ok(None);
            };
            let value = value
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.)
                .ok_or_else(|| anyhow!("agents.notifyWhen: {name} must be a positive number"))?;
            Ok(Some(value))
        };
        let tokens = numeric("tokens")?;
        let usd = numeric("usd")?;
        let idle_seconds = numeric("idleSeconds")?;
        let context_used_pct = numeric("contextUsedPct")?;
        if let Some(context) = context_used_pct {
            if context > 100. {
                bail!("agents.notifyWhen: contextUsedPct must be a finite number in (0, 100]");
            }
            if tokens.is_some() || usd.is_some() || idle_seconds.is_some() {
                bail!(
                    "agents.notifyWhen: contextUsedPct is a single-purpose health predicate and cannot be combined with tokens, usd, or idleSeconds"
                );
            }
        }
        if tokens.is_none() && usd.is_none() && idle_seconds.is_none() && context_used_pct.is_none()
        {
            bail!(
                "agents.notifyWhen requires at least one threshold: tokens, usd, idleSeconds, or contextUsedPct"
            );
        }
        let target =
            (self.lookup)(id).ok_or_else(|| anyhow!("agents.notifyWhen: no such session {id}"))?;
        if ended(&target) {
            bail!("agents.notifyWhen: session {id} has already ended — nothing left to watch");
        }
        let owner = provider(text(&target, "provider"));
        let h = health(&target, now);
        if context_used_pct.is_some() {
            if unsupported(&owner) {
                bail!(
                    "agents.notifyWhen: contextUsedPct is unavailable for provider {owner}: it cannot emit a runtime context window"
                );
            }
            if !owner.is_empty()
                && !matches!(owner.as_str(), "claude" | "codex" | "copilot")
                && h.is_none()
            {
                bail!(
                    "agents.notifyWhen: contextUsedPct cannot wait for unknown provider {owner} without a fresh runtime context sample"
                );
            }
        }
        let watcher = text(params, "notifySessionId");
        let watcher = if watcher.is_empty() {
            text(&target, "parentSessionId")
        } else {
            watcher
        };
        if watcher.is_empty() {
            bail!(
                "agents.notifyWhen: no notifySessionId and the target has no parent session — pass your own session id as notifySessionId"
            );
        }
        if (self.lookup)(watcher).is_none_or(|row| ended(&row)) {
            bail!(
                "agents.notifyWhen: notifySessionId {watcher} is not a live session — a watch with no recipient would fire into nothing"
            );
        }
        let mut state = self.state.lock().unwrap();
        if state
            .watches
            .values()
            .filter(|w| w.watcher_session_id == watcher)
            .count()
            >= 20
        {
            bail!(
                "agents.notifyWhen: {watcher} already has 20 armed watches (max 20) — let some fire, or stop arming in a loop"
            );
        }
        state.sequence += 1;
        let mut watch = Watch {
            id: format!("w{}", state.sequence),
            session_id: id.into(),
            watcher_session_id: watcher.into(),
            tokens,
            usd,
            idle_seconds,
            context_used_pct,
            armed_at: now,
            context_provider: String::new(),
            context_epoch: None,
            state: String::new(),
        };
        if let Some(threshold) = context_used_pct {
            watch.context_provider = owner;
            if let Some(h) = h {
                watch.context_provider = h.provider;
                watch.context_epoch = Some(h.epoch);
                watch.state = if h.percent >= threshold {
                    "alreadySatisfied"
                } else {
                    "armed"
                }
                .into();
            } else {
                watch.state = "waitingForTelemetry".into();
            }
        }
        let result = serde_json::to_value(&watch)?;
        state.watches.insert(watch.id.clone(), watch);
        Ok(result)
    }
    pub async fn sweep(&self, now: i64) {
        let mut fired: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        {
            let mut state = self.state.lock().unwrap();
            state.watches.retain(|_,watch|{
            let Some(target)=(self.lookup)(&watch.session_id)else{return false};
            if ended(&target)||(self.lookup)(&watch.watcher_session_id).is_none_or(|row|ended(&row)){return false;}
            let Some(crossed)=watch.crossing(&target,now)else{return true};
            let label=super::progress::label(&target);
            fired.entry(watch.watcher_session_id.clone()).or_default().push(json!({"label":label,"sessionId":watch.session_id,"cwd":target["cwd"],"crossed":crossed}));false
        });
        }
        for (parent, mut entries) in fired {
            entries.sort_by(|a, b| text(a, "sessionId").cmp(text(b, "sessionId")));
            let Ok(message) = super::fleet_messages::build("threshold", &entries, false) else {
                continue;
            };
            let recipient = match &self.replacements {
                Some(state) => match state.automatic_wake_target(&parent) {
                    Ok(target) => target,
                    Err(_) => continue,
                },
                None => parent,
            };
            if let Some(state) = &self.replacements {
                match state.hold_message(&recipient, &message, &[], None) {
                    Ok(true) | Err(_) => continue,
                    Ok(false) => (),
                }
            }
            let admission = self
                .replacements
                .as_ref()
                .map(|state| state.admit(&[&recipient]))
                .transpose();
            let Ok(_admission) = admission else { continue };
            if (self.lookup)(&recipient).is_none_or(|row| ended(&row)) {
                continue;
            }
            if let Err(error) = (self.deliver)(recipient, message).await {
                eprintln!("threshold wake delivery failed; one-shot will not replay: {error}");
            }
        }
    }
    pub fn close(&self) {
        self.stopped.send_replace(true);
    }
    pub async fn run(self: Arc<Self>, hub: crate::Handle) -> Result<()> {
        let mut stop = self.stopped.subscribe();
        if *stop.borrow() {
            return Ok(());
        }
        tokio::select! {r=hub.ready()=>{r?;},_=stop.changed()=>return Ok(())};
        let mut timer = tokio::time::interval(Duration::from_secs(15));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        timer.tick().await;
        loop {
            tokio::select! {_=stop.changed()=>return Ok(()),_=timer.tick()=>{tokio::select!{_=stop.changed()=>return Ok(()),_=self.sweep(chrono::Utc::now().timestamp_millis())=>()}}}
        }
    }
}

pub(crate) fn install(
    mut options: crate::Options,
    hub: crate::Handle,
) -> (
    crate::Options,
    Option<Arc<Thresholds>>,
    Option<tokio::task::JoinHandle<Result<()>>>,
) {
    let Some(engine) = options.engine.clone() else {
        return (options, None, None);
    };
    let mut service = Thresholds::new(
        super::local_lookup(&options),
        super::progress::engine_delivery(&options, engine),
    );
    Arc::get_mut(&mut service).unwrap().replacements = options.replacements.clone();
    let handler = service.clone();
    options = options.handler("agents.notifyWhen", move |_, params| {
        let handler = handler.clone();
        async move { handler.arm(&params, chrono::Utc::now().timestamp_millis()) }
    });
    let running = service.clone();
    let task = tokio::spawn(async move { running.run(hub).await });
    (options, Some(service), Some(task))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(
        fail: bool,
    ) -> (
        Arc<Thresholds>,
        Arc<Mutex<BTreeMap<String, Value>>>,
        Arc<Mutex<Vec<(String, String)>>>,
    ) {
        let rows = Arc::new(Mutex::new(BTreeMap::from([
            (
                "worker".into(),
                json!({"sessionId":"worker","parentSessionId":"parent","provider":"codex","cwd":"/repo/leaf","ambientState":"streaming"}),
            ),
            (
                "parent".into(),
                json!({"sessionId":"parent","status":"idle"}),
            ),
        ])));
        let lookup = rows.clone();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let deliveries = sent.clone();
        let service = Thresholds::new(
            Arc::new(move |id| lookup.lock().unwrap().get(id).cloned()),
            Arc::new(move |id, text| {
                let deliveries = deliveries.clone();
                Box::pin(async move {
                    deliveries.lock().unwrap().push((id, text));
                    if fail {
                        bail!("fixture rejected");
                    }
                    Ok(())
                })
            }),
        );
        (service, rows, sent)
    }
    fn sample(epoch: Value) -> Value {
        json!({"provider":"codex","status_line":{"context_health":{"used_tokens":160,"window_tokens":200,"used_pct":80,"window_source":"runtime","observed_at":"2026-09-28T00:00:00Z","epoch":epoch,"provider":"codex"}}})
    }
    fn now() -> i64 {
        chrono::DateTime::parse_from_rfc3339("2026-09-28T00:00:00Z")
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn correlated_context_requires_fresh_owner_exact_epoch_and_runtime_pair() {
        let now = now();
        let row = sample(json!("1788888888888888901"));
        let first = health(&row, now).unwrap();
        let second = health(&sample(json!("1788888888888888902")), now).unwrap();
        assert_ne!(first.epoch, second.epoch);
        for epoch in [
            json!(1788888888888888901_u64),
            json!(0),
            json!("01"),
            json!("+1"),
            json!("18446744073709551616"),
        ] {
            assert!(health(&sample(epoch), now).is_none());
        }
        assert!(health(&sample(json!(9007199254740991_u64)), now).is_some());
        assert!(health(&row, now + 120001).is_none());
        assert!(health(&row, now - 5001).is_none());
        for (key, value) in [
            ("window_source", json!("catalog")),
            ("used_pct", json!(81)),
            ("used_tokens", json!(201)),
            ("provider", json!("claude")),
        ] {
            let mut row = row.clone();
            row["status_line"]["context_health"][key] = value;
            assert!(health(&row, now).is_none());
        }
        let mut compat = json!({"provider":"codex","statusLine":{"contextHealth":{"usedTokens":160,"windowTokens":200,"usedPct":80,"windowSource":"runtime","observedAt":"2026-09-28T00:00:00Z","epoch":"1","provider":"codex"}}});
        assert!(health(&compat, now).is_some());
        compat["status_line"] = json!({});
        assert!(
            health(&compat, now).is_none(),
            "present raw block forbids stale compatibility fallback"
        );
    }
    #[tokio::test]
    async fn shared_context_contract_does_not_launder_cumulative_usage() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../../contracts/context-health-cases.json"
        ))
        .unwrap();
        for block in ["formatPctCases", "unsupportedProviders", "cumulativeCodex"] {
            assert!(corpus["vocabulary"]["blocks"][block]["loaders"].as_array().unwrap().iter().any(|loader|loader == "services/hub-rs/src/services/thresholds.rs::shared_context_contract_does_not_launder_cumulative_usage"),"Rust watch consumer missing from {block} contract");
        }
        assert_eq!(corpus["formatPctCases"].as_array().unwrap().len(), 10);
        for case in corpus["formatPctCases"].as_array().unwrap() {
            assert_eq!(
                pct(case["input"].as_f64().unwrap()),
                case["expected"].as_str().unwrap()
            );
        }
        let (service, rows, sent) = fixture(false);
        assert!(
            !corpus["unsupportedProviders"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for case in corpus["unsupportedProviders"].as_array().unwrap() {
            rows.lock().unwrap().get_mut("worker").unwrap()["provider"] = case["provider"].clone();
            assert!(
                service
                    .arm(&json!({"sessionId":"worker","contextUsedPct":80}), now())
                    .is_err()
            );
            assert!(service.state.lock().unwrap().watches.is_empty());
        }
        assert_eq!(corpus["cumulativeCodex"].as_array().unwrap().len(), 1);
        let case = &corpus["cumulativeCodex"][0];
        {
            let mut rows = rows.lock().unwrap();
            let worker = rows.get_mut("worker").unwrap();
            worker["provider"] = json!("codex");
            worker["status_line"] = json!({"total_input_tokens":case["inputTokens"],"total_output_tokens":case["outputTokens"],"context_window_size":case["windowTokens"]});
        }
        assert_eq!(
            service
                .arm(
                    &json!({"sessionId":"worker","contextUsedPct":case["thresholdPct"]}),
                    now()
                )
                .unwrap()["state"],
            "waitingForTelemetry"
        );
        service.sweep(now()).await;
        assert!(sent.lock().unwrap().is_empty());
        assert_eq!(service.state.lock().unwrap().watches.len(), 1);
        rows.lock().unwrap().get_mut("worker").unwrap()["status_line"] =
            sample(json!("1788888888888888902"))["status_line"].clone();
        service.sweep(now()).await;
        assert_eq!(sent.lock().unwrap().len(), 1);
        assert!(
            sent.lock().unwrap()[0]
                .1
                .contains("runtime-confirmed by codex")
        );
        assert!(service.state.lock().unwrap().watches.is_empty());
    }
    #[tokio::test]
    async fn idle_streaming_raw_usage_and_one_shot_failure_coalesce() {
        let (service, rows, sent) = fixture(true);
        {
            let mut rows = rows.lock().unwrap();
            let worker = rows.get_mut("worker").unwrap();
            worker["lastActivity"] = json!(1000);
            worker["status_line"] =
                json!({"total_input_tokens":1001,"total_output_tokens":9,"cost_usd":2});
            worker["statusLine"] = json!({"totalInputTokens":1,"costUSD":0});
        }
        for params in [
            json!({"tokens":1000,"usd":1}),
            json!({"idleSeconds":10}),
            json!({"usd":1}),
        ] {
            let mut p = params;
            p["sessionId"] = json!("worker");
            service.arm(&p, 1000).unwrap();
        }
        service.sweep(12000).await;
        service.sweep(14000).await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "parent");
        assert!(sent[0].1.contains("tokens 1,010 ≥ 1,000"));
        assert!(
            sent[0]
                .1
                .contains("no activity for 11s ≥ 10s (still reports streaming)")
        );
        assert!(sent[0].1.contains("cost $2.00 ≥ $1.00"));
    }
    #[tokio::test]
    async fn lifetime_limits_and_dead_recipients_do_not_invent_deliveries() {
        let (service, rows, sent) = fixture(false);
        for params in [
            json!({}),
            json!({"tokens":0}),
            json!({"tokens":"1"}),
            json!({"contextUsedPct":101}),
            json!({"contextUsedPct":80,"usd":1}),
        ] {
            let mut p = params;
            p["sessionId"] = json!("worker");
            assert!(service.arm(&p, now()).is_err());
        }
        for _ in 0..20 {
            service
                .arm(&json!({"sessionId":"worker","tokens":1}), now())
                .unwrap();
        }
        assert!(
            service
                .arm(&json!({"sessionId":"worker","tokens":1}), now())
                .is_err()
        );
        rows.lock().unwrap().get_mut("parent").unwrap()["status"] = json!("ended");
        service.sweep(now()).await;
        assert!(sent.lock().unwrap().is_empty());
        assert!(service.state.lock().unwrap().watches.is_empty());
    }
    #[tokio::test]
    async fn epoch_transition_invalidates_instead_of_silently_rebinding() {
        let (service, rows, sent) = fixture(false);
        rows.lock().unwrap().get_mut("worker").unwrap()["status_line"] =
            sample(json!("1788888888888888901"))["status_line"].clone();
        assert_eq!(
            service
                .arm(&json!({"sessionId":"worker","contextUsedPct":90}), now())
                .unwrap()["state"],
            "armed"
        );
        rows.lock().unwrap().get_mut("worker").unwrap()["status_line"] =
            sample(json!("1788888888888888902"))["status_line"].clone();
        service.sweep(now()).await;
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0]
                .1
                .contains("1788888888888888901 → 1788888888888888902")
        );
        assert!(sent[0].1.contains("monitoring invalidated"));
    }
}

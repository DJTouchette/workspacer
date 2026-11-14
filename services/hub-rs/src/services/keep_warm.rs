//! Keep-warm: keeps subscription 5-hour rate-limit windows running.
//!
//! Claude and Codex subscriptions both meter usage in 5-hour windows that only
//! START when the first message of a window is sent. When `claude.keepWarm` is
//! enabled, this loop checks each configured provider's current 5h window and,
//! when it is expired/absent, fires ONE minimal turn so a fresh window is
//! already running. It never pings while a window is active.
//!
//! Modes (`claude.keepWarm.mode`): `auto` re-warms whenever a window lapses,
//! `interval` checks every `intervalHours`, `daily` checks once a day at
//! `dailyAt` (machine-local "HH:MM").
//!
//! The ping itself is a claudemon heartbeat (`POST /heartbeat`): the engine
//! runs the turn, records it in its heartbeats table (never a session) and
//! returns the new window's reset time. Window checks before pinging are both
//! sessionless: Claude from the engine's `GET /usage`, Codex from
//! `GET /usage/report` with a live local session's status line as fallback.
//! When both are silent, ping to be safe: a redundant ping costs a few tokens
//! and cannot reset an already-running window.
//!
//! Off by default; only an owned backend runs it. Failure is always soft and
//! log-only. Decision logic lives in [`logic`] (pure, unit-tested).
pub mod logic;

use anyhow::{Context, Result, bail};
use claudemon::daemon::embedded::{Command, EmbeddedClient, Status as EngineStatus};
use futures_util::future::BoxFuture;
use logic::{
    AccountUsageWire, KeepWarmConfig, ProviderWarmState, ScheduleState, day_key,
    five_hour_window_from_report, is_warmable_provider, provider_needs_check, schedule_due,
    window_active,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

const CHECK_INTERVAL: Duration = Duration::from_secs(60);
const FIVE_HOURS_MS: i64 = 5 * 60 * 60 * 1000;
/// The daemon caps a heartbeat run at 120s; allow a little slack on top.
const PING_TIMEOUT: Duration = Duration::from_secs(130);
const USAGE_TIMEOUT: Duration = Duration::from_secs(15);
/// After a failed ping, don't retry for this long (auto mode would otherwise
/// re-attempt every tick while e.g. offline).
const FAILURE_BACKOFF_MS: i64 = 10 * 60 * 1000;

/// What the loop needs from the owned engine. `None` always means "unknown".
pub(crate) trait Engine: Send + Sync {
    /// `GET /usage`: the default Claude login's account usage.
    fn claude_usage(&self) -> BoxFuture<'_, Option<Value>>;
    /// `GET /usage/report`: every provider's windows as their CLIs left them.
    fn usage_report(&self) -> BoxFuture<'_, Option<Value>>;
    /// Session rows of THIS engine. A federated peer's rate-limit window
    /// belongs to the peer machine's account and must never appear here.
    fn local_sessions(&self) -> BoxFuture<'_, Option<Vec<Value>>>;
    /// `POST /heartbeat`: one minimal turn; resolves to the stored row.
    fn heartbeat(&self, body: Value) -> BoxFuture<'_, Result<Value>>;
}

/// The owned embedded engine. Reads go through the in-process router; the
/// heartbeat uses the engine's own loopback listener because one run may take
/// two minutes, beyond the in-process command deadline, and must not occupy a
/// command slot that message delivery waits on.
pub(crate) struct OwnedEngine {
    client: EmbeddedClient,
    http: reqwest::Client,
}
impl OwnedEngine {
    pub(crate) fn new(client: EmbeddedClient) -> Result<Self> {
        Ok(Self {
            client,
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    async fn read(&self, path: &str) -> Option<Value> {
        let request = self.client.request(Command::Request {
            method: "GET".into(),
            path: path.into(),
            payload: None,
        });
        tokio::time::timeout(USAGE_TIMEOUT, request)
            .await
            .ok()?
            .ok()
    }
}
impl Engine for OwnedEngine {
    fn claude_usage(&self) -> BoxFuture<'_, Option<Value>> {
        Box::pin(self.read("/usage"))
    }
    fn usage_report(&self) -> BoxFuture<'_, Option<Value>> {
        Box::pin(self.read("/usage/report"))
    }
    fn local_sessions(&self) -> BoxFuture<'_, Option<Vec<Value>>> {
        Box::pin(async {
            match self.read("/sessions?state_only=true").await? {
                Value::Array(rows) => Some(rows),
                _ => None,
            }
        })
    }
    fn heartbeat(&self, body: Value) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move {
            let address = match &*self.client.status().borrow() {
                EngineStatus::Ready(ready) => crate::net_address::dial_addr(ready.api_addr),
                _ => bail!("the owned engine is not ready"),
            };
            let response = self
                .http
                .post(format!("http://{address}/heartbeat"))
                .timeout(PING_TIMEOUT)
                .json(&body)
                .send()
                .await?;
            if !response.status().is_success() {
                bail!("heartbeat endpoint returned {}", response.status());
            }
            response.json().await.context("invalid heartbeat response")
        })
    }
}

/// The launcher argv handed to the engine, resolved the way session spawns
/// resolve their binary: the client resolves, the daemon runs.
fn argv_for(provider: &str, config: &Value) -> Vec<String> {
    vec![if provider == "claude" {
        super::models::resolve_spawn_binary("claude", config)
    } else {
        super::models::resolve_binary(provider, config)
    }]
}

pub(crate) struct KeepWarm<E> {
    engine: E,
    config: Box<dyn Fn() -> Value + Send + Sync>,
    schedule: ScheduleState,
    providers: BTreeMap<String, ProviderWarmState>,
    warned_no_warmable_provider: bool,
}
impl<E: Engine> KeepWarm<E> {
    pub(crate) fn new(engine: E, config: impl Fn() -> Value + Send + Sync + 'static) -> Self {
        Self {
            engine,
            config: Box::new(config),
            schedule: ScheduleState::default(),
            providers: BTreeMap::new(),
            warned_no_warmable_provider: false,
        }
    }

    /// Check now, then every minute, until the task is cancelled. A tick that
    /// outlasts the period (a slow ping) skips the ticks it covered.
    pub(crate) async fn run(mut self) {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            self.tick(chrono::Local::now()).await;
        }
    }

    fn read_config(&mut self, config: &Value) -> Option<KeepWarmConfig> {
        let keep_warm = &config["claude"]["keepWarm"];
        if keep_warm["enabled"] != true {
            return None;
        }
        let configured: Vec<String> = match &keep_warm["providers"] {
            Value::Null => vec!["claude".into()],
            Value::Array(list) => list
                .iter()
                .filter_map(|provider| provider.as_str().map(str::to_owned))
                .collect(),
            _ => vec![],
        };
        let providers: Vec<String> = configured
            .iter()
            .filter(|provider| is_warmable_provider(provider))
            .cloned()
            .collect();
        if providers.is_empty() {
            // Enabled, but nothing warmable is configured (a hand-edited
            // config). Logged once, because the tick is every 60s and forever.
            if !self.warned_no_warmable_provider {
                self.warned_no_warmable_provider = true;
                tracing::warn!(
                    configured = %configured.join(", "),
                    "keep-warm is enabled, but no configured provider has a 5h window to warm"
                );
            }
            return None;
        }
        self.warned_no_warmable_provider = false;
        Some(KeepWarmConfig {
            enabled: true,
            providers,
            mode: match &keep_warm["mode"] {
                Value::Null => "auto".into(),
                Value::String(mode) => mode.clone(),
                // Not a mode; `schedule_due` never opens a slot for it.
                _ => String::new(),
            },
            interval_hours: match &keep_warm["intervalHours"] {
                Value::Null => 5.0,
                // A non-number is not positive, so the 5h fallback applies.
                other => other.as_f64().unwrap_or(f64::NAN),
            },
            daily_at: match &keep_warm["dailyAt"] {
                Value::Null => "08:00".into(),
                // A non-string is unparseable: daily mode does nothing.
                other => other.as_str().unwrap_or("").into(),
            },
        })
    }

    pub(crate) async fn tick<Tz: chrono::TimeZone>(&mut self, now: chrono::DateTime<Tz>) {
        let config = (self.config)();
        let Some(cfg) = self.read_config(&config) else {
            return;
        };
        if !schedule_due(&cfg, &self.schedule, &now) {
            return;
        }
        let now_ms = now.timestamp_millis();
        let due: Vec<&String> = cfg
            .providers
            .iter()
            .filter(|provider| {
                provider_needs_check(
                    &self.providers.get(*provider).copied().unwrap_or_default(),
                    now_ms,
                )
            })
            .collect();
        // Interval/daily slots are consumed once opened: "checked, all warm"
        // counts as this slot's check. (Auto mode has no slot to consume.)
        match cfg.mode.as_str() {
            "interval" => self.schedule.last_interval_check_ms = Some(now_ms),
            "daily" => self.schedule.last_daily_key = Some(day_key(&now)),
            _ => {}
        }
        for provider in due {
            self.evaluate(provider, now_ms, &config).await;
        }
    }

    /// Check one provider's window; ping when it's expired/unknown.
    async fn evaluate(&mut self, provider: &str, now_ms: i64, config: &Value) {
        let known = if provider == "claude" {
            self.engine
                .claude_usage()
                .await
                .map(|usage| AccountUsageWire::from_value(&usage))
        } else {
            self.codex_window(now_ms).await
        };
        match known {
            Some(usage) if window_active(&usage, now_ms) => {
                if let Some(resets) = usage.five_hour_resets_at {
                    self.providers
                        .entry(provider.into())
                        .or_default()
                        .assumed_resets_at_ms = Some((resets * 1000.0) as i64);
                }
                tracing::debug!(
                    provider,
                    "keep-warm: 5h window already active; no ping needed"
                );
                return;
            }
            Some(_) => tracing::info!(provider, "keep-warm: 5h window has lapsed; warming"),
            None => tracing::info!(
                provider,
                "keep-warm: window state unknown; pinging to be safe"
            ),
        }
        self.ping(provider, now_ms, config).await;
    }

    /// Codex's 5h window: the engine's on-disk report first, a live session's
    /// status line second. The report answers with no Codex session running.
    /// The status line is still consulted after a LAPSED disk reading because
    /// it only ever reports a window that is still running: it can add a
    /// window the newest rollout has not recorded yet, never take one away.
    async fn codex_window(&self, now_ms: i64) -> Option<AccountUsageWire> {
        let on_disk = self
            .engine
            .usage_report()
            .await
            .and_then(|report| five_hour_window_from_report(&report, "codex", now_ms));
        if on_disk.is_some_and(|window| window_active(&window, now_ms)) {
            return on_disk;
        }
        // `on_disk` is now either `{}` (definitely lapsed) or None (unreadable).
        self.codex_window_from_sessions(now_ms).await.or(on_disk)
    }

    /// The freshest 5h reset from live local Codex sessions' status lines. A
    /// session with a lapsed window is skipped, not reported as expired.
    async fn codex_window_from_sessions(&self, now_ms: i64) -> Option<AccountUsageWire> {
        let mut best: Option<f64> = None;
        for row in self.engine.local_sessions().await? {
            // Federation: a remote session's window belongs to the PEER
            // machine's account and must not drive the local pinger.
            let peer = !matches!(&row["hub"], Value::Null)
                && row["hub"].as_str().is_none_or(|hub| !hub.is_empty());
            if row["provider"] != "codex" || peer {
                continue;
            }
            let Some(resets) = row["status_line"]["five_hour_resets_at"].as_f64() else {
                continue;
            };
            if resets * 1000.0 > now_ms as f64 && best.is_none_or(|current| resets > current) {
                best = Some(resets);
            }
        }
        best.map(|resets| AccountUsageWire {
            five_hour_pct: None,
            five_hour_resets_at: Some(resets),
        })
    }

    /// One claudemon heartbeat: a minimal turn run by the engine.
    async fn ping(&mut self, provider: &str, now_ms: i64, config: &Value) {
        tracing::info!(provider, "keep-warm: requesting heartbeat from the engine");
        let mut body = json!({"provider": provider, "argv": argv_for(provider, config)});
        if provider == "claude" {
            body["model"] = json!("haiku");
        }
        let request = tokio::time::timeout(PING_TIMEOUT, self.engine.heartbeat(body));
        let resets_at = match request.await {
            Ok(Ok(row)) if row["ok"] == true => Some(row["resets_at"].as_f64()),
            Ok(Ok(row)) => {
                let error = row["error"].as_str().unwrap_or("unknown");
                tracing::warn!(provider, error, "keep-warm: heartbeat failed");
                None
            }
            Ok(Err(error)) => {
                tracing::warn!(provider, error = %format!("{error:#}"), "keep-warm: heartbeat request failed");
                None
            }
            Err(_) => {
                tracing::warn!(provider, "keep-warm: heartbeat request timed out");
                None
            }
        };
        let state = self.providers.entry(provider.into()).or_default();
        let Some(resets_at) = resets_at else {
            state.not_before_ms = Some(now_ms + FAILURE_BACKOFF_MS);
            return;
        };
        state.not_before_ms = None;
        let resets_at_ms = resets_at
            .map(|seconds| (seconds * 1000.0) as i64)
            .unwrap_or(now_ms + FIVE_HOURS_MS);
        state.assumed_resets_at_ms = Some(resets_at_ms);
        tracing::info!(provider, resets_at_ms, "keep-warm: window started");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Local, TimeZone};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Script {
        claude_usage: Option<Value>,
        usage_report: Option<Value>,
        sessions: Option<Vec<Value>>,
        heartbeat: Option<Value>,
        calls: Vec<String>,
        pings: Vec<Value>,
    }
    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<Script>>);
    impl Fake {
        fn set(&self, change: impl FnOnce(&mut Script)) {
            change(&mut self.0.lock().unwrap());
        }
        fn pings(&self) -> Vec<Value> {
            self.0.lock().unwrap().pings.clone()
        }
        fn calls(&self) -> Vec<String> {
            std::mem::take(&mut self.0.lock().unwrap().calls)
        }
    }
    impl Engine for Fake {
        fn claude_usage(&self) -> BoxFuture<'_, Option<Value>> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("usage".into());
            let value = script.claude_usage.clone();
            Box::pin(async move { value })
        }
        fn usage_report(&self) -> BoxFuture<'_, Option<Value>> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("report".into());
            let value = script.usage_report.clone();
            Box::pin(async move { value })
        }
        fn local_sessions(&self) -> BoxFuture<'_, Option<Vec<Value>>> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("sessions".into());
            let value = script.sessions.clone();
            Box::pin(async move { value })
        }
        fn heartbeat(&self, body: Value) -> BoxFuture<'_, Result<Value>> {
            let mut script = self.0.lock().unwrap();
            script.calls.push("heartbeat".into());
            script.pings.push(body);
            let value = script.heartbeat.clone();
            Box::pin(async move { value.ok_or_else(|| anyhow::anyhow!("engine unreachable")) })
        }
    }

    fn at(hh: u32, mm: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 7, 20, hh, mm, 0)
            .earliest()
            .unwrap()
    }
    fn seconds(now: DateTime<Local>, offset: i64) -> i64 {
        now.timestamp() + offset
    }
    fn service(keep_warm: Value) -> (KeepWarm<Fake>, Fake, Arc<Mutex<Value>>) {
        let engine = Fake::default();
        // Explicit binaries: the resolvers must not depend on this machine.
        let config = Arc::new(Mutex::new(json!({
            "claude": {"keepWarm": keep_warm},
            "agents": {"binaries": {"claude": "/fixture/claude", "codex": "/fixture/codex"}}
        })));
        let source = config.clone();
        (
            KeepWarm::new(engine.clone(), move || source.lock().unwrap().clone()),
            engine,
            config,
        )
    }
    fn codex_report(resets_at: i64) -> Value {
        json!({"providers": [{"provider": "codex", "accounts": [
            {"windows": {"five_hour": {"used_percent": {"state": "ok", "value": 67}, "resets_at": resets_at}}}
        ]}]})
    }

    #[tokio::test]
    async fn disabled_or_absent_configuration_never_touches_the_engine() {
        for keep_warm in [
            Value::Null,
            json!({}),
            json!({"enabled": false}),
            json!({"enabled": "true"}),
            json!({"enabled": true, "providers": ["copilot", "pi"]}),
            json!({"enabled": true, "providers": "claude"}),
            json!({"enabled": true, "mode": "sometimes"}),
            json!({"enabled": true, "mode": 5}),
            json!({"enabled": true, "mode": "daily", "dailyAt": 800}),
        ] {
            let (mut service, engine, _) = service(keep_warm.clone());
            service.tick(at(12, 0)).await;
            assert!(engine.calls().is_empty(), "{keep_warm}");
        }
    }

    #[tokio::test]
    async fn an_active_claude_window_is_remembered_and_never_pinged() {
        let (mut service, engine, _) = service(json!({"enabled": true}));
        let now = at(9, 0);
        engine.set(|script| {
            script.claude_usage =
                Some(json!({"five_hour_pct": 0, "five_hour_resets_at": seconds(now, 3600)}))
        });
        service.tick(now).await;
        assert_eq!(engine.calls(), ["usage"]);
        // Quiet until the remembered reset, then it looks again.
        service.tick(at(9, 59)).await;
        assert!(engine.calls().is_empty());
        // Utilization without a reset is active but leaves nothing to remember.
        engine.set(|script| script.claude_usage = Some(json!({"five_hour_pct": 19})));
        service.tick(at(10, 0)).await;
        service.tick(at(10, 1)).await;
        assert_eq!(engine.calls(), ["usage", "usage"]);
        assert!(engine.pings().is_empty());
    }

    #[tokio::test]
    async fn a_lapsed_or_unknown_claude_window_is_warmed_with_the_launch_argv_and_haiku() {
        let (mut service, engine, _) = service(json!({"enabled": true}));
        let now = at(9, 0);
        engine.set(|script| {
            script.claude_usage = Some(json!({"five_hour_pct": 0, "five_hour_resets_at": null}));
            script.heartbeat = Some(json!({"ok": true, "resets_at": seconds(now, 7200)}));
        });
        service.tick(now).await;
        assert_eq!(engine.calls(), ["usage", "heartbeat"]);
        assert_eq!(
            engine.pings(),
            [json!({"provider": "claude", "argv": ["/fixture/claude"], "model": "haiku"})]
        );
        // The returned reset keeps auto mode quiet until the window lapses.
        service.tick(at(10, 59)).await;
        assert!(engine.calls().is_empty());
        // Unknown usage pings to be safe; no reset in the row assumes now+5h.
        engine.set(|script| {
            script.claude_usage = None;
            script.heartbeat = Some(json!({"ok": true, "resets_at": null}));
        });
        service.tick(at(11, 0)).await;
        assert_eq!(engine.calls(), ["usage", "heartbeat"]);
        service.tick(at(15, 59)).await;
        assert!(engine.calls().is_empty());
        service.tick(at(16, 0)).await;
        assert_eq!(engine.calls(), ["usage", "heartbeat"]);
    }

    #[tokio::test]
    async fn a_failed_ping_backs_off_for_ten_minutes() {
        let (mut service, engine, _) = service(json!({"enabled": true}));
        // Unreachable engine, then a recorded failure, then an HTTP-level row
        // without `ok`: all three are failures.
        service.tick(at(9, 0)).await;
        assert_eq!(engine.calls(), ["usage", "heartbeat"]);
        service.tick(at(9, 9)).await;
        assert!(engine.calls().is_empty());
        engine.set(|script| script.heartbeat = Some(json!({"ok": false, "error": "offline"})));
        service.tick(at(9, 10)).await;
        assert_eq!(engine.calls(), ["usage", "heartbeat"]);
        service.tick(at(9, 19)).await;
        assert!(engine.calls().is_empty());
        engine.set(|script| script.heartbeat = Some(json!({"resets_at": 1})));
        service.tick(at(9, 20)).await;
        service.tick(at(9, 21)).await;
        assert_eq!(engine.calls(), ["usage", "heartbeat"]);
        // Success clears the backoff.
        engine.set(|script| script.heartbeat = Some(json!({"ok": true})));
        service.tick(at(9, 30)).await;
        assert_eq!(engine.calls(), ["usage", "heartbeat"]);
        service.tick(at(9, 40)).await;
        assert!(engine.calls().is_empty());
    }

    #[tokio::test]
    async fn codex_reads_the_report_first_and_local_status_lines_second() {
        let (mut service, engine, _) = service(json!({"enabled": true, "providers": ["codex"]}));
        let now = at(9, 0);
        // A running on-disk window answers alone.
        engine.set(|script| script.usage_report = Some(codex_report(seconds(now, 600))));
        service.tick(now).await;
        assert_eq!(engine.calls(), ["report"]);
        // Lapsed on disk (a stale 67% must not count): a live LOCAL session's
        // status line may still add a running window.
        let later = at(9, 30);
        engine.set(|script| {
            script.usage_report = Some(codex_report(seconds(later, -60)));
            script.sessions = Some(vec![
                json!({"provider": "claude", "status_line": {"five_hour_resets_at": seconds(later, 9000)}}),
                json!({"provider": "codex", "status_line": {"five_hour_resets_at": seconds(later, -5)}}),
                json!({"provider": "codex", "hub": "", "status_line": {"five_hour_resets_at": seconds(later, 300)}}),
                json!({"provider": "codex", "status_line": {"five_hour_resets_at": seconds(later, 120)}}),
                json!({"provider": "codex"}),
            ]);
        });
        service.tick(later).await;
        assert_eq!(engine.calls(), ["report", "sessions"]);
        assert!(engine.pings().is_empty());
        // The freshest reset (+300s) was remembered.
        service.tick(at(9, 34)).await;
        assert!(engine.calls().is_empty());
    }

    #[tokio::test]
    async fn a_peer_session_never_answers_for_the_local_codex_account() {
        let (mut service, engine, _) = service(json!({"enabled": true, "providers": ["codex"]}));
        let now = at(9, 0);
        engine.set(|script| {
            script.sessions = Some(vec![
                json!({"provider": "codex", "hub": "peer", "status_line": {"five_hour_resets_at": seconds(now, 3600)}}),
                json!({"provider": "codex", "hub": {"id": "peer"}, "status_line": {"five_hour_resets_at": seconds(now, 3600)}}),
            ]);
            script.heartbeat = Some(json!({"ok": true, "resets_at": seconds(now, 18_000)}));
        });
        service.tick(now).await;
        assert_eq!(engine.calls(), ["report", "sessions", "heartbeat"]);
        // Codex runs on the account default: no forced model.
        assert_eq!(
            engine.pings(),
            [json!({"provider": "codex", "argv": ["/fixture/codex"]})]
        );
    }

    #[tokio::test]
    async fn providers_lapse_independently_and_unwarmable_ones_are_dropped() {
        let (mut service, engine, _) =
            service(json!({"enabled": true, "providers": ["claude", "copilot", "codex"]}));
        let now = at(9, 0);
        engine.set(|script| {
            script.claude_usage = Some(json!({"five_hour_resets_at": seconds(now, 3600)}));
            script.heartbeat = Some(json!({"ok": true, "resets_at": seconds(now, 600)}));
        });
        service.tick(now).await;
        assert_eq!(engine.calls(), ["usage", "report", "sessions", "heartbeat"]);
        assert_eq!(engine.pings()[0]["provider"], "codex");
        // Only codex is due after its ten-minute window; claude stays quiet.
        engine.set(|script| script.usage_report = Some(codex_report(seconds(now, 7200))));
        service.tick(at(9, 10)).await;
        assert_eq!(engine.calls(), ["report"]);
    }

    #[tokio::test]
    async fn interval_and_daily_slots_are_consumed_and_configuration_is_read_each_tick() {
        let (mut service, engine, config) =
            service(json!({"enabled": true, "mode": "interval", "intervalHours": 2}));
        engine.set(|script| script.claude_usage = Some(json!({"five_hour_pct": 3})));
        service.tick(at(9, 0)).await;
        service.tick(at(10, 59)).await;
        assert_eq!(engine.calls(), ["usage"]);
        service.tick(at(11, 0)).await;
        assert_eq!(engine.calls(), ["usage"]);
        // Switching mode takes effect on the next tick, without a restart.
        config.lock().unwrap()["claude"]["keepWarm"] =
            json!({"enabled": true, "mode": "daily", "dailyAt": "11:30"});
        service.tick(at(11, 29)).await;
        assert!(engine.calls().is_empty());
        service.tick(at(11, 30)).await;
        service.tick(at(11, 31)).await;
        service.tick(at(23, 59)).await;
        assert_eq!(engine.calls(), ["usage"]);
        config.lock().unwrap()["claude"]["keepWarm"]["enabled"] = json!(false);
        service.tick(at(12, 0)).await;
        assert!(engine.calls().is_empty());
    }

    #[test]
    fn argv_reuses_the_launch_resolvers() {
        let config =
            json!({"agents": {"binaries": {"claude": " /opt/claude ", "codex": "/opt/codex"}}});
        assert_eq!(argv_for("claude", &config), ["/opt/claude"]);
        assert_eq!(argv_for("codex", &config), ["/opt/codex"]);
    }
}

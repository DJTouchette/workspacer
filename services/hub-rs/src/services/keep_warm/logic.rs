//! Pure decision logic for the keep-warm 5h-window pinger, separated from the
//! service loop so tests need neither a config file nor an engine.
//!
//! Two gates compose:
//!  - the SCHEDULE gate (per config mode: always / every N hours / daily at),
//!    one shared slot regardless of how many providers are warmed;
//!  - the PROVIDER gate (per provider: failure backoff plus a window already
//!    known or assumed to be running). Claude and Codex lapse independently.
use chrono::{DateTime, Datelike, TimeZone, Timelike};
use serde_json::Value;

/// The harnesses that meter a 5-hour subscription window a ping can START.
/// Copilot bills monthly requests; opencode and pi are bring-your-own-key.
pub const WARMABLE_PROVIDERS: [&str; 2] = ["claude", "codex"];

/// Whether a configured provider id is one keep-warm can actually ping.
pub fn is_warmable_provider(provider: &str) -> bool {
    WARMABLE_PROVIDERS.contains(&provider)
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeepWarmConfig {
    pub enabled: bool,
    /// Which subscription windows to warm: `claude` and/or `codex`.
    pub providers: Vec<String>,
    /// `auto`, `interval` or `daily`; anything else never opens a slot.
    pub mode: String,
    pub interval_hours: f64,
    pub daily_at: String,
}

/// Shared scheduling state: when the last interval/daily slot ran.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScheduleState {
    pub last_interval_check_ms: Option<i64>,
    pub last_daily_key: Option<String>,
}

/// Per-provider window state.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProviderWarmState {
    /// Reset time (epoch ms) of the window we know/assume is running.
    pub assumed_resets_at_ms: Option<i64>,
    /// Failure backoff: no ping attempts before this (epoch ms).
    pub not_before_ms: Option<i64>,
}

/// Local calendar key (YYYY-MM-DD) for daily-mode dedup.
pub fn day_key<Tz: TimeZone>(now: &DateTime<Tz>) -> String {
    format!("{}-{:02}-{:02}", now.year(), now.month(), now.day())
}

/// `^(\d{1,2}):(\d{2})$` over the trimmed text, as minutes after midnight.
/// Out-of-range values are kept as written: "25:00" simply never comes due.
fn daily_target_minutes(text: &str) -> Option<u32> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(hours) || hours.len() > 2 || !digits(minutes) || minutes.len() != 2 {
        return None;
    }
    Some(hours.parse::<u32>().ok()? * 60 + minutes.parse::<u32>().ok()?)
}

/// Whether this tick opens a check slot at all, per mode. `now` carries the
/// machine's local zone in production; daily mode compares local wall time.
pub fn schedule_due<Tz: TimeZone>(
    cfg: &KeepWarmConfig,
    state: &ScheduleState,
    now: &DateTime<Tz>,
) -> bool {
    match cfg.mode.as_str() {
        "auto" => true,
        "interval" => {
            let hours = if cfg.interval_hours > 0.0 {
                cfg.interval_hours
            } else {
                5.0
            };
            state.last_interval_check_ms.is_none_or(|last| {
                (now.timestamp_millis() - last) as f64 >= hours * 60.0 * 60.0 * 1000.0
            })
        }
        "daily" => {
            if state.last_daily_key.as_deref() == Some(day_key(now).as_str()) {
                return false;
            }
            // An unparseable time does nothing rather than guess.
            let Some(target) = daily_target_minutes(&cfg.daily_at) else {
                return false;
            };
            now.hour() * 60 + now.minute() >= target
        }
        _ => false,
    }
}

/// Whether a provider needs its window checked: not backing off, and no
/// window known/assumed to be running.
pub fn provider_needs_check(state: &ProviderWarmState, now_ms: i64) -> bool {
    if state.not_before_ms.is_some_and(|until| now_ms < until) {
        return false;
    }
    if state
        .assumed_resets_at_ms
        .is_some_and(|reset| now_ms < reset)
    {
        return false;
    }
    true
}

/// Account usage as served by claudemon's `GET /usage` (snake_case wire).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AccountUsageWire {
    pub five_hour_pct: Option<f64>,
    /// Epoch seconds.
    pub five_hour_resets_at: Option<f64>,
}
impl AccountUsageWire {
    /// Only numbers count; any other JSON type is the same as absent.
    pub fn from_value(value: &Value) -> Self {
        Self {
            five_hour_pct: value.get("five_hour_pct").and_then(Value::as_f64),
            five_hour_resets_at: value.get("five_hour_resets_at").and_then(Value::as_f64),
        }
    }
}

/// Whether a 5h window is currently running. The OAuth endpoint has been seen
/// reporting utilization without `resets_at`, so either signal counts.
pub fn window_active(usage: &AccountUsageWire, now_ms: i64) -> bool {
    if usage.five_hour_pct.is_some_and(|pct| pct > 0.0) {
        return true;
    }
    usage
        .five_hour_resets_at
        .is_some_and(|resets| resets * 1000.0 > now_ms as f64)
}

/// A provider's 5h window as claudemon's `GET /usage/report` sees it, in the
/// flat shape [`window_active`] consumes. Three outcomes:
///
///   `Some {five_hour_resets_at}` - a window is running; this is when it lapses.
///   `Some {}`                    - every readable reading has ALREADY rolled
///                                  over: a definite "no window is running".
///   `None`                       - nothing readable. Genuinely unknown.
///
/// The PERCENTAGE is deliberately not carried across. `used_percent` is the
/// last figure the provider wrote, and for a rolled-over window that is real
/// history and a false present; [`window_active`] treats any percentage above
/// zero as a live window, so forwarding it would suppress warming permanently.
/// Currency is decided from `resets_at` against `now_ms`, never from the
/// report's own `is_current`, which was computed at `generated_at`.
pub fn five_hour_window_from_report(
    report: &Value,
    provider: &str,
    now_ms: i64,
) -> Option<AccountUsageWire> {
    let entry = report
        .get("providers")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("provider").and_then(Value::as_str) == Some(provider))?;
    let mut best: Option<f64> = None;
    let mut lapsed = false;
    for account in entry
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // No reset time is no answer: a percentage alone cannot say which
        // window it describes.
        let Some(resets) = account
            .pointer("/windows/five_hour/resets_at")
            .and_then(Value::as_f64)
        else {
            continue;
        };
        if resets * 1000.0 > now_ms as f64 {
            if best.is_none_or(|current| resets > current) {
                best = Some(resets);
            }
        } else {
            lapsed = true;
        }
    }
    match best {
        Some(resets) => Some(AccountUsageWire {
            five_hour_pct: None,
            five_hour_resets_at: Some(resets),
        }),
        None => lapsed.then(AccountUsageWire::default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use serde_json::json;

    fn cfg(mode: &str, interval_hours: f64, daily_at: &str) -> KeepWarmConfig {
        KeepWarmConfig {
            enabled: true,
            providers: vec!["claude".into()],
            mode: mode.into(),
            interval_hours,
            daily_at: daily_at.into(),
        }
    }
    // Local-time constructor so daily-mode tests are TZ-independent.
    fn on(day: u32, hh: u32, mm: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 7, day, hh, mm, 0)
            .earliest()
            .expect("fixture local time exists")
    }
    fn at(hh: u32, mm: u32) -> DateTime<Local> {
        on(20, hh, mm)
    }
    fn now_ms() -> i64 {
        at(9, 0).timestamp_millis()
    }
    fn resets(seconds: i64) -> Option<AccountUsageWire> {
        Some(AccountUsageWire {
            five_hour_pct: None,
            five_hour_resets_at: Some(seconds as f64),
        })
    }

    // ---- scheduleDue ----

    #[test]
    fn auto_every_tick_opens_a_slot() {
        assert!(schedule_due(
            &cfg("auto", 5.0, "08:00"),
            &ScheduleState::default(),
            &at(3, 0)
        ));
    }

    #[test]
    fn interval_first_slot_is_immediate_then_respects_the_cadence() {
        let c = cfg("interval", 2.0, "08:00");
        let mut state = ScheduleState::default();
        assert!(schedule_due(&c, &state, &at(9, 0)));
        state.last_interval_check_ms = Some(at(9, 0).timestamp_millis());
        assert!(!schedule_due(&c, &state, &at(10, 59)));
        assert!(schedule_due(&c, &state, &at(11, 0)));
    }

    #[test]
    fn interval_a_non_positive_cadence_falls_back_to_5h_instead_of_spinning() {
        let c = cfg("interval", 0.0, "08:00");
        let state = ScheduleState {
            last_interval_check_ms: Some(at(9, 0).timestamp_millis()),
            ..Default::default()
        };
        assert!(!schedule_due(&c, &state, &at(9, 1)));
        assert!(schedule_due(&c, &state, &at(14, 0)));
        // NaN is what a non-numeric config value becomes; same fallback.
        let nan = cfg("interval", f64::NAN, "08:00");
        assert!(!schedule_due(&nan, &state, &at(9, 1)));
        assert!(schedule_due(&nan, &state, &at(14, 0)));
    }

    #[test]
    fn daily_opens_at_or_after_the_configured_time_once_per_day() {
        let c = cfg("daily", 5.0, "08:30");
        let mut state = ScheduleState::default();
        assert!(!schedule_due(&c, &state, &at(8, 29)));
        assert!(schedule_due(&c, &state, &at(8, 30)));
        state.last_daily_key = Some(day_key(&at(8, 30)));
        assert!(!schedule_due(&c, &state, &at(9, 0)));
        // Next day it's due again.
        assert!(schedule_due(&c, &state, &on(21, 8, 30)));
    }

    #[test]
    fn daily_an_unparseable_time_does_nothing_rather_than_guessing() {
        for text in [
            "morning",
            "",
            "8",
            "08:5",
            "008:30",
            "8:300",
            "-8:30",
            "٠٨:٣٠",
        ] {
            assert!(
                !schedule_due(
                    &cfg("daily", 5.0, text),
                    &ScheduleState::default(),
                    &at(12, 0)
                ),
                "{text:?}"
            );
        }
        // Surrounding whitespace and a one-digit hour are accepted.
        assert!(schedule_due(
            &cfg("daily", 5.0, " 8:30 "),
            &ScheduleState::default(),
            &at(12, 0)
        ));
    }

    #[test]
    fn an_unknown_mode_never_opens_a_slot() {
        assert!(!schedule_due(
            &cfg("hourly", 5.0, "08:00"),
            &ScheduleState::default(),
            &at(12, 0)
        ));
    }

    #[test]
    fn day_key_is_the_zero_padded_local_calendar_date() {
        assert_eq!(day_key(&at(8, 30)), "2026-07-20");
        assert_eq!(
            day_key(
                &Local
                    .with_ymd_and_hms(2026, 1, 5, 0, 0, 0)
                    .earliest()
                    .unwrap()
            ),
            "2026-01-05"
        );
    }

    // ---- providerNeedsCheck ----

    #[test]
    fn a_fresh_provider_needs_a_check() {
        assert!(provider_needs_check(
            &ProviderWarmState::default(),
            now_ms()
        ));
    }

    #[test]
    fn quiet_while_a_window_is_assumed_running_due_again_after_it_lapses() {
        let mut state = ProviderWarmState {
            assumed_resets_at_ms: Some(now_ms() + 60_000),
            ..Default::default()
        };
        assert!(!provider_needs_check(&state, now_ms()));
        state.assumed_resets_at_ms = Some(now_ms() - 1);
        assert!(provider_needs_check(&state, now_ms()));
    }

    #[test]
    fn failure_backoff_suppresses_checks_until_it_expires() {
        let mut state = ProviderWarmState {
            not_before_ms: Some(now_ms() + 1),
            ..Default::default()
        };
        assert!(!provider_needs_check(&state, now_ms()));
        state.not_before_ms = Some(now_ms());
        assert!(provider_needs_check(&state, now_ms()));
    }

    // ---- windowActive ----

    fn usage(pct: Option<f64>, resets_at: Option<f64>) -> AccountUsageWire {
        AccountUsageWire {
            five_hour_pct: pct,
            five_hour_resets_at: resets_at,
        }
    }

    #[test]
    fn nonzero_utilization_counts_even_without_resets_at() {
        assert!(window_active(&usage(Some(19.0), None), now_ms()));
    }

    #[test]
    fn a_future_reset_counts_even_at_zero_utilization() {
        let reset = now_ms() as f64 / 1000.0 + 3600.0;
        assert!(window_active(&usage(Some(0.0), Some(reset)), now_ms()));
    }

    #[test]
    fn zero_percent_with_a_past_or_absent_reset_means_no_window() {
        assert!(!window_active(&usage(Some(0.0), None), now_ms()));
        let past = now_ms() as f64 / 1000.0 - 10.0;
        assert!(!window_active(&usage(None, Some(past)), now_ms()));
        assert!(!window_active(&AccountUsageWire::default(), now_ms()));
    }

    #[test]
    fn the_usage_wire_reads_numbers_only() {
        assert_eq!(
            AccountUsageWire::from_value(&json!({"five_hour_pct":19,"five_hour_resets_at":null})),
            usage(Some(19.0), None)
        );
        assert_eq!(
            AccountUsageWire::from_value(&json!({"five_hour_pct":"19","five_hour_resets_at":"1"})),
            AccountUsageWire::default()
        );
        assert_eq!(
            AccountUsageWire::from_value(&json!([])),
            AccountUsageWire::default()
        );
    }

    // ---- fiveHourWindowFromReport ----

    fn sec(offset_ms: i64) -> i64 {
        (now_ms() + offset_ms).div_euclid(1000)
    }
    fn report(provider: &str, accounts: Value) -> Value {
        json!({"generated_at": now_ms() / 1000, "providers": [{"provider": provider, "accounts": accounts}]})
    }
    fn acct(five_hour: Value) -> Value {
        json!({"account": "", "label": "x", "windows": {"five_hour": five_hour}})
    }

    #[test]
    fn a_future_reset_is_a_running_window_and_feeds_window_active() {
        let window = five_hour_window_from_report(
            &report(
                "codex",
                json!([acct(json!({
                    "used_percent": {"state": "ok", "value": 12},
                    "resets_at": sec(3_600_000),
                    "is_current": true
                }))]),
            ),
            "codex",
            now_ms(),
        );
        assert_eq!(window, resets(sec(3_600_000)));
        assert!(window_active(&window.unwrap(), now_ms()));
    }

    // The trap this whole function exists to avoid: codex reads 67% against a
    // resets_at two days gone. The percentage is real history and a false
    // present; carrying it across would suppress warming permanently.
    #[test]
    fn a_high_percentage_on_a_rolled_over_window_is_not_a_running_window() {
        let window = five_hour_window_from_report(
            &report(
                "codex",
                json!([acct(json!({
                    "used_percent": {"state": "ok", "value": 67},
                    "resets_at": sec(-172_800_000),
                    "is_current": false
                }))]),
            ),
            "codex",
            now_ms(),
        );
        assert_eq!(window, Some(AccountUsageWire::default()));
        assert!(!window_active(&window.unwrap(), now_ms()));
    }

    #[test]
    fn distinguishes_definitely_lapsed_from_cannot_tell() {
        // No reset time at all: the report leaves is_current null, and so do we.
        assert_eq!(
            five_hour_window_from_report(
                &report(
                    "codex",
                    json!([acct(
                        json!({"used_percent": {"state": "ok", "value": 67}, "resets_at": null})
                    )])
                ),
                "codex",
                now_ms()
            ),
            None
        );
        // An unreadable window is unknown, not lapsed.
        assert_eq!(
            five_hour_window_from_report(
                &report(
                    "codex",
                    json!([acct(json!({"used_percent": {
                        "state": "unavailable",
                        "reason": "no local quota record"
                    }}))])
                ),
                "codex",
                now_ms()
            ),
            None
        );
    }

    #[test]
    fn takes_the_latest_reset_when_several_accounts_report_one() {
        assert_eq!(
            five_hour_window_from_report(
                &report(
                    "codex",
                    json!([
                        acct(json!({"resets_at": sec(-1000)})),
                        acct(json!({"resets_at": sec(600_000)})),
                        acct(json!({"resets_at": sec(3_600_000)}))
                    ])
                ),
                "codex",
                now_ms()
            ),
            resets(sec(3_600_000))
        );
    }

    #[test]
    fn a_provider_the_report_does_not_carry_is_unknown_not_lapsed() {
        assert_eq!(
            five_hour_window_from_report(
                &report("claude", json!([acct(json!({"resets_at": sec(-1)}))])),
                "codex",
                now_ms()
            ),
            None
        );
        assert_eq!(
            five_hour_window_from_report(&json!({}), "codex", now_ms()),
            None
        );
        assert_eq!(
            five_hour_window_from_report(&report("codex", json!([])), "codex", now_ms()),
            None
        );
    }

    // -----------------------------------------------------------------------
    // THE CROSS-LANGUAGE HALF.
    //
    // contracts/usage-window-currency-cases.json is the WINDOW-CURRENCY RULE:
    // a reading may be used only if resets_at is present and strictly after
    // the moment of the decision, and the wire's own `is_current` is a display
    // hint that is never the decision input. five_hour_window_from_report's
    // three returns ARE the three verdicts, and the percentage is dropped on
    // all of them.
    // -----------------------------------------------------------------------

    /// The floor is what stops a fixture edit that drops half the cases from
    /// reporting a clean sweep over the two that survived.
    const CURRENCY_CASE_FLOOR: usize = 11;
    const CURRENCY_VERDICT_FLOOR: [(&str, usize); 3] =
        [("current", 3), ("rolled-over", 4), ("unreadable", 3)];

    fn currency_fixture() -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/usage-window-currency-cases.json");
        serde_json::from_slice(&std::fs::read(&path).expect("currency fixture is readable"))
            .expect("currency fixture is JSON")
    }
    fn cases(fixture: &Value) -> &Vec<Value> {
        fixture["cases"].as_array().expect("fixture carries cases")
    }
    fn one_window(case: &Value, window: &Value) -> Value {
        json!({
            "generated_at": case["now"],
            "providers": [{
                "provider": "codex",
                "accounts": [{"account": "", "label": "x", "windows": {"five_hour": window}}]
            }]
        })
    }
    /// The reason vocabulary, re-derived from the case's own inputs.
    fn expected_reason(case: &Value) -> Option<&'static str> {
        if case["window"].is_null() {
            return Some("no-window-reading-at-all");
        }
        let Some(resets) = case["window"]["resets_at"].as_i64() else {
            return Some("no-reset-time-reported");
        };
        let now = case["now"].as_i64().unwrap();
        if resets == now {
            Some("reset-time-equals-now")
        } else if resets < now {
            Some("reset-time-has-passed")
        } else {
            None
        }
    }

    #[test]
    fn usage_window_currency_cross_language_contract() {
        let fixture = currency_fixture();
        // A fixture that stopped loading would otherwise pass vacuously.
        assert!(cases(&fixture).len() >= CURRENCY_CASE_FLOOR);
        for case in cases(&fixture) {
            let name = case["name"].as_str().unwrap();
            let now = case["now"].as_i64().unwrap();
            let now_ms = now * 1000;
            let got =
                five_hour_window_from_report(&one_window(case, &case["window"]), "codex", now_ms);
            let verdict = case["expect"].as_str().unwrap();
            match verdict {
                "current" => {
                    let window_resets = case["window"]["resets_at"].as_i64().unwrap();
                    assert_eq!(got, resets(window_resets), "{name}");
                    assert!(window_active(&got.unwrap(), now_ms), "{name}");
                }
                "rolled-over" => {
                    // A DEFINITE "no window is running": strictly more than
                    // "unknown", and what lets keep-warm ping knowingly.
                    assert_eq!(got, Some(AccountUsageWire::default()), "{name}");
                    assert!(!window_active(&got.unwrap(), now_ms), "{name}");
                }
                "unreadable" => assert_eq!(got, None, "{name}"),
                other => panic!("{name}: unknown verdict {other}"),
            }
            // The percentage never crosses, on ANY verdict.
            if let Some(got) = got {
                assert_eq!(got.five_hour_pct, None, "{name}");
            }
            // unknownBecause: derived from `now` and `window` by the shared rule.
            assert_eq!(
                case.get("unknownBecause").and_then(Value::as_str),
                expected_reason(case),
                "{name}: unknownBecause"
            );
            if verdict == "current" {
                let window_resets = case["window"]["resets_at"].as_i64().unwrap();
                let seconds = case["secondsToReset"].as_i64();
                assert_eq!(seconds, Some(window_resets - now), "{name}: secondsToReset");
                assert_eq!(
                    got.unwrap().five_hour_resets_at.unwrap() as i64 - now,
                    seconds.unwrap(),
                    "{name}"
                );
                assert!(seconds.unwrap() > 0, "{name}");
                // usedPercent is pinned iff the source could read one.
                let measured = &case["window"]["used_percent"];
                if measured["state"] == "ok" && measured["value"].is_number() {
                    assert_eq!(
                        case["usedPercent"], measured["value"],
                        "{name}: usedPercent"
                    );
                } else {
                    assert!(
                        case["usedPercent"].is_null(),
                        "{name}: an unreadable scalar has no percentage"
                    );
                }
            } else {
                // UNKNOWN has to mean NO NUMBER AT ALL.
                assert!(
                    case["usedPercent"].is_null(),
                    "{name}: usedPercent on a non-current case"
                );
                assert!(
                    case["secondsToReset"].is_null(),
                    "{name}: secondsToReset on a non-current case"
                );
            }
        }
    }

    #[test]
    fn the_currency_corpus_carries_every_verdict_not_eleven_of_one() {
        let fixture = currency_fixture();
        for (verdict, minimum) in CURRENCY_VERDICT_FLOOR {
            let counted = cases(&fixture)
                .iter()
                .filter(|case| case["expect"] == verdict)
                .count();
            assert!(
                counted >= minimum,
                "the corpus carries {counted} {verdict} case(s)"
            );
        }
    }

    #[test]
    fn every_unknown_because_is_in_the_fixtures_own_declared_vocabulary() {
        let fixture = currency_fixture();
        let declared: std::collections::BTreeSet<&str> = ["rolledOverReasons", "unreadableReasons"]
            .iter()
            .filter_map(|block| fixture["vocabulary"][block].as_object())
            .flat_map(|reasons| reasons.keys().map(String::as_str))
            .collect();
        assert!(!declared.is_empty(), "the vocabulary blocks were renamed");
        for case in cases(&fixture) {
            if let Some(reason) = case.get("unknownBecause").and_then(Value::as_str) {
                assert!(
                    declared.contains(reason),
                    "{} names an undeclared reason",
                    case["name"]
                );
            }
        }
    }

    #[test]
    fn the_wires_is_current_is_never_the_decision_input() {
        // Flipping the hint on every case and requiring the verdicts not to
        // move is what makes "display-only" checkable rather than a comment.
        let fixture = currency_fixture();
        let mut flipped = 0;
        for case in cases(&fixture) {
            if case["window"].is_null() {
                continue;
            }
            let now_ms = case["now"].as_i64().unwrap() * 1000;
            let before =
                five_hour_window_from_report(&one_window(case, &case["window"]), "codex", now_ms);
            let mut mutant = case["window"].clone();
            mutant["is_current"] = json!(match case["window"]["is_current"].as_bool() {
                Some(current) => !current,
                None => true,
            });
            flipped += 1;
            assert_eq!(
                five_hour_window_from_report(&one_window(case, &mutant), "codex", now_ms),
                before,
                "{}",
                case["name"]
            );
        }
        assert!(flipped >= CURRENCY_CASE_FLOOR - 1);
    }
}

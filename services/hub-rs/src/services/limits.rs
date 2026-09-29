//! Quota evidence is judged at the instant of use; cached `is_current` is never authority.
use chrono::{Datelike, Local, TimeZone, Timelike, Utc};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct Reading {
    state: &'static str,
    reason: &'static str,
    used: Option<f64>,
    reset: Option<i64>,
    minutes: Option<i64>,
    now: i64,
}
impl Reading {
    pub fn read(window: &Value, now: i64) -> Self {
        let mut r = Self {
            state: "unreadable",
            reason: "no-window-reading-at-all",
            used: None,
            reset: None,
            minutes: None,
            now,
        };
        if window.is_null() {
            return r;
        }
        let Some(reset) = window["resets_at"].as_i64() else {
            r.reason = "no-reset-time-reported";
            return r;
        };
        if reset <= now {
            r.state = "rolled-over";
            r.reason = if reset == now {
                "reset-time-equals-now"
            } else {
                "reset-time-has-passed"
            };
            return r;
        }
        r.state = "current";
        r.reason = "";
        r.reset = Some(reset);
        r.minutes = window["window_minutes"]
            .as_i64()
            .filter(|m| *m > 0 && *m <= 366 * 24 * 60);
        if window["used_percent"]["state"] == "ok" {
            r.used = window["used_percent"]["value"]
                .as_f64()
                .filter(|v| v.is_finite());
        }
        r
    }
    pub fn used(&self) -> Option<f64> {
        self.used
    }
    pub fn seconds(&self) -> Option<i64> {
        self.reset.and_then(|r| r.checked_sub(self.now))
    }
    pub fn health(&self, window: &Value, bands: &Value) -> &'static str {
        if self.state != "current" && window["used_percent"]["state"] == "unavailable" {
            return "unmetered";
        }
        let (Some(used), Some(yellow), Some(red)) = (
            self.used,
            bands["yellow_at_used_pct"].as_f64(),
            bands["red_at_used_pct"].as_f64(),
        ) else {
            return "unknown";
        };
        if yellow <= 0. || red < yellow {
            return "unknown";
        }
        if used >= 100. {
            "exhausted"
        } else if used >= red {
            "red"
        } else if used >= yellow {
            "yellow"
        } else {
            "green"
        }
    }
}
fn num(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap_or(0.)
}
fn weekday_seconds(start: i64, end: i64, zone: &str, weight: f64) -> Option<f64> {
    if end < start || end.checked_sub(start)? > 62 * 86400 {
        return None;
    }
    // Walk hour boundaries, including partial hours and DST transitions. Every zone's
    // local midnight is an hour boundary except historical subminute offsets.
    let mut t = start;
    let mut total = 0.;
    while t < end {
        let dt = Utc.timestamp_opt(t, 0).single()?;
        let (weekday, second, minute) = if zone == "local" {
            let local = dt.with_timezone(&Local);
            (local.weekday(), local.second(), local.minute())
        } else {
            let tz: chrono_tz::Tz = zone.parse().ok()?;
            let local = dt.with_timezone(&tz);
            (local.weekday(), local.second(), local.minute())
        };
        let next = (t + 3600 - (minute * 60 + second) as i64).min(end);
        total += (next - t) as f64
            * if weekday.num_days_from_monday() >= 5 {
                weight
            } else {
                1.
            };
        t = next;
    }
    Some(total)
}
pub fn pace(window: &Value, name: &str, account: &Value, cfg: &Value, now: i64) -> Value {
    let mut out = json!({"window":name,"state":"unknown","known":false,"usedPct":0.,"expectedPct":0.,"because":"quota evidence cannot establish a current pace"});
    if cfg["enabled"] != true {
        out["state"] = "disabled".into();
        out["because"] = "pacing is disabled".into();
        return out;
    }
    let conserve = num(cfg, "conserve_at_ratio");
    let block = num(cfg, "block_spend_down_at_ratio");
    if conserve <= 0. || block <= 0. || block > conserve || account["fresh"] == false {
        return out;
    }
    let r = Reading::read(window, now);
    let (Some(used), Some(seconds), Some(minutes)) = (r.used(), r.seconds(), r.minutes) else {
        return out;
    };
    let length = minutes * 60;
    if seconds > length {
        return out;
    }
    let elapsed = (length - seconds) as f64 / length as f64;
    out["elapsedPct"] = (elapsed * 100.).into();
    if elapsed * 100. < num(&cfg["bootstrap"], "min_elapsed_pct") {
        return out;
    }
    let mut expected = elapsed;
    let mut curve = "calendar";
    let mut note = String::new();
    let week = &cfg["seven_day"];
    if name == "seven_day" {
        let requested = week["curve"].as_str().unwrap_or("calendar");
        let weight = if requested == "five_day" {
            0.
        } else {
            num(week, "weekend_weight")
        };
        if requested == "five_day" || requested == "workdays" && weight > 0. {
            let zone = week["timezone"].as_str().unwrap_or("local");
            let reset = r.reset.unwrap();
            let start = reset - length;
            if let (Some(done), Some(total)) = (
                weekday_seconds(start, now, zone, weight),
                weekday_seconds(start, reset, zone, weight),
            ) {
                if total > 0. {
                    expected = done / total;
                    curve = requested;
                }
            } else {
                note = "; timezone/curve unavailable; calendar fallback".into();
            }
        }
        if requested != "five_day" && week["weekend"] == "reserve" {
            let reserve = num(week, "weekend_reserve_pct");
            if (0. ..100.).contains(&reserve) {
                expected *= 1. - reserve / 100.;
            }
        }
    }
    expected = (expected + num(&cfg["bootstrap"], "expected_offset_pct").max(0.) / 100.).min(1.);
    if expected <= 0. {
        return out;
    }
    let ratio = used / 100. / expected;
    let state = if ratio >= conserve {
        "overspending"
    } else if ratio >= block {
        "ahead"
    } else {
        "on_track"
    };
    out["known"] = true.into();
    out["state"] = state.into();
    out["curve"] = curve.into();
    out["usedPct"] = used.into();
    out["expectedPct"] = (expected * 100.).into();
    out["ratio"] = ratio.into();
    out["because"] = format!(
        "{name}: {used:.1}% used, {:.1}% elapsed, {ratio:.2}x {curve} pace{note}",
        elapsed * 100.
    )
    .into();
    out
}
fn severity(s: &str) -> usize {
    [
        "unmetered",
        "green",
        "unknown",
        "yellow",
        "red",
        "exhausted",
    ]
    .iter()
    .position(|x| *x == s)
    .unwrap_or(2)
}
pub fn capacity(
    matrix: &Value,
    report: &Value,
    provider: &str,
    requested: &str,
    now: i64,
) -> Value {
    let provider_row = report["providers"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["provider"] == provider));
    let accounts = provider_row.and_then(|p| p["accounts"].as_array());
    let account = accounts.and_then(|rows| {
        if !requested.is_empty() {
            rows.iter()
                .find(|r| r["account"].as_str() == Some(requested))
        } else {
            rows.iter()
                .find(|r| r["is_default"] == true && r["account"].is_string())
                .or_else(|| rows.iter().find(|r| r["account"].is_string()))
        }
    });
    let mut buckets = Vec::new();
    let mut paces = Vec::new();
    let mut health = "unmetered";
    if let Some(account) = account {
        for name in ["five_hour", "seven_day", "monthly"] {
            let w = &account["windows"][name];
            let r = Reading::read(w, now);
            let h = r.health(w, &matrix["thresholds"]["health"]);
            if severity(h) > severity(health) {
                health = h;
            }
            let mut b = json!({"id":format!("{provider}/{}/{name}",account["account"].as_str().unwrap_or("")),"window":name,"health":h,"metered":h!="unmetered","state":r.state,"reason":r.reason,"explain":format!("{provider} {name}: {h}; {}",r.reason),"source":account["source"]});
            if let Some(u) = r.used {
                b["usedPercent"] = u.into();
                b["remainingPercent"] = (100. - u).into();
            }
            if let Some(s) = r.seconds() {
                b["resetsInSeconds"] = s.into();
            }
            buckets.push(b);
            paces.push(pace(w, name, account, &matrix["thresholds"]["pacing"], now));
        }
    } else {
        health = "unknown";
    }
    let assumption = matrix["providers"][provider]["when_unknown"]
        .as_str()
        .unwrap_or("unknown");
    let assumption = if ["green", "yellow", "red", "exhausted", "unmetered"].contains(&assumption) {
        assumption
    } else {
        "unknown"
    };
    let effective = if health == "unknown" {
        assumption
    } else {
        health
    };
    let mut out = json!({"provider":provider,"account":account.and_then(|a| a["account"].as_str()).unwrap_or(requested),"accountKnown":account.is_some(),"health":health,"effectiveHealth":effective,"metered":buckets.iter().any(|b| b["metered"]==true),"buckets":buckets,"because":format!("observed {health}"),"inReport":provider_row.is_some()});
    if health == "unknown" {
        out["assumedHealth"] = assumption.into();
    }
    if matrix["thresholds"]["pacing"]["enabled"] == true {
        if let Some(worst) = paces
            .iter()
            .filter(|p| p["known"] == true)
            .max_by(|a, b| num(a, "ratio").total_cmp(&num(b, "ratio")))
        {
            out["pace"] = worst.clone();
        }
        out["paceWindows"] = paces.into();
    }
    if let Some(a) = account {
        out["observedAt"] = a["observed_at"].clone();
    }
    out
}
pub fn projection(report: &Value, cfg: &Value, now: i64) -> Value {
    let mut providers = Vec::new();
    for p in report["providers"].as_array().into_iter().flatten() {
        let mut accounts = Vec::new();
        for a in p["accounts"].as_array().into_iter().flatten() {
            let mut row = json!({});
            for key in [
                "account",
                "label",
                "is_default",
                "source",
                "observed_at",
                "fresh",
                "failure",
            ] {
                row[key] = a[key].clone();
            }
            row["windows"] = json!({});
            for name in ["five_hour", "seven_day", "monthly"] {
                let w = &a["windows"][name];
                if w.is_null() {
                    continue;
                }
                row["windows"][name] = w.clone();
                row["windows"][name]["pace"] = pace(w, name, a, cfg, now);
            }
            accounts.push(row);
        }
        providers.push(json!({"provider":p["provider"],"note":p["note"],"accounts":accounts}));
    }
    json!({"generated_at":report["generated_at"],"evaluated_at":now,"valid_until":now+60,"providers":providers})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_pacing_contract() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../../../contracts/usage-pacing-cases.json"
        ))
        .unwrap();
        fn merge(a: &mut Value, b: &Value) {
            if let Some(map) = b.as_object() {
                for (k, v) in map {
                    if v.is_object() {
                        merge(&mut a[k], v);
                    } else {
                        a[k] = v.clone();
                    }
                }
            }
        }
        for case in corpus["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let now = case["now"]
                .as_i64()
                .unwrap_or(corpus["now"].as_i64().unwrap());
            let mut cfg = corpus["config"].clone();
            merge(&mut cfg, &case["patch"]);
            let w = json!({"used_percent":{"state":case["measurement"].as_str().unwrap_or("ok"),"value":case["used"]},"resets_at":case["resetIn"].as_i64().map(|r|now+r),"window_minutes":case["minutes"]});
            let p = pace(
                &w,
                case["window"].as_str().unwrap_or("five_hour"),
                &json!({"fresh":case["fresh"]}),
                &cfg,
                now,
            );
            assert_eq!(p["state"], case["expect"]["state"], "{name}");
            assert_eq!(p["known"], case["expect"]["known"], "{name}");
            if let Some(ratio) = case["expect"]["ratio"].as_f64() {
                assert!(
                    (p["ratio"].as_f64().unwrap() - ratio).abs() < 1e-10,
                    "{name}: {p}"
                );
            }
            if let Some(curve) = case["expect"].get("curve") {
                assert_eq!(&p["curve"], curve, "{name}");
            }
            if let Some(health) = case["health"].as_str() {
                assert_eq!(
                    Reading::read(&w, now)
                        .health(&w, &json!({"yellow_at_used_pct":70,"red_at_used_pct":90})),
                    health,
                    "{name}"
                );
            }
        }
    }
    #[test]
    fn shared_currency_contract() {
        let data: Value = serde_json::from_str(include_str!(
            "../../../../contracts/usage-window-currency-cases.json"
        ))
        .unwrap();
        for case in data["cases"].as_array().unwrap() {
            let r = Reading::read(&case["window"], case["now"].as_i64().unwrap());
            assert_eq!(
                r.state,
                case["expect"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(json!(r.used()), case["usedPercent"], "{}", case["name"]);
            assert_eq!(
                json!(r.seconds()),
                case["secondsToReset"],
                "{}",
                case["name"]
            );
            if let Some(reason) = case["unknownBecause"].as_str() {
                assert_eq!(r.reason, reason);
            }
        }
    }
    #[test]
    fn account_null_cannot_become_default_and_stale_cannot_pace() {
        let matrix: Value = serde_yaml::from_str(include_str!("routing.default.yaml")).unwrap();
        let report = json!({"providers":[{"provider":"claude","accounts":[{"account":null,"is_default":true,"windows":{}}]}]});
        assert_eq!(
            capacity(&matrix, &report, "claude", "", 100)["accountKnown"],
            false
        );
        let window = json!({"used_percent":{"state":"ok","value":50},"resets_at":10000,"window_minutes":300});
        assert_eq!(
            pace(
                &window,
                "five_hour",
                &json!({"fresh":false}),
                &matrix["thresholds"]["pacing"],
                100
            )["known"],
            false
        );
    }
}

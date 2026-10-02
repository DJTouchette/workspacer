//! Account usage windows from the hub's `usage.report`: every provider, every
//! account, with no live session. Display rules twin desktop
//! `usagePacingRows` (lib/usagePacing.ts): unavailable windows and windows
//! that rolled over are skipped, a percentage is shown only when it was
//! measured for the window open now, and an account appears only once one of
//! its windows has a reading.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// "5h", "Week" or "Month".
    pub label: &'static str,
    pub pct: f64,
    pub resets_at: Option<i64>,
    /// Hub pacing verdict when known: "on_track", "ahead" or "overspending".
    pub pace: Option<String>,
    pub expected_pct: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    /// "claude", "codex", …
    pub provider: String,
    /// "Claude" for the default login, "Claude · work" for a profile.
    pub title: String,
    pub stale: bool,
    /// Why the last refresh failed, when it did; the reading may still stand.
    pub failure: Option<String>,
    pub windows: Vec<Window>,
}

impl Account {
    /// The window to show when there is room for one: the shortest, which
    /// is the one that blocks soonest (5h, else weekly, else monthly).
    pub fn primary(&self) -> &Window {
        &self.windows[0]
    }
}

/// When the hub evaluated the report (epoch seconds).
pub fn evaluated_at(report: &Value) -> Option<i64> {
    report["evaluated_at"]
        .as_i64()
        .or_else(|| report["generated_at"].as_i64())
}

/// "just now", "45s ago", "3m ago".
pub fn ago(at: i64, now: i64) -> String {
    match (now - at).max(0) {
        s if s < 10 => "just now".into(),
        s if s < 60 => format!("{s}s ago"),
        s if s < 3_600 => format!("{}m ago", s / 60),
        s => format!("{}h ago", s / 3_600),
    }
}

const WINDOWS: [(&str, &str); 3] = [
    ("five_hour", "5h"),
    ("seven_day", "Week"),
    ("monthly", "Month"),
];

fn provider_name(provider: &str) -> String {
    match provider {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "copilot" => "Copilot".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

pub fn accounts(report: &Value, now: i64) -> Vec<Account> {
    let mut out = Vec::new();
    for provider in report["providers"].as_array().into_iter().flatten() {
        let id = provider["provider"].as_str().unwrap_or_default();
        for account in provider["accounts"].as_array().into_iter().flatten() {
            let mut windows = Vec::new();
            for (key, label) in WINDOWS {
                let w = &account["windows"][key];
                if w.is_null() || w["used_percent"]["state"] == "unavailable" {
                    continue;
                }
                let resets_at = w["resets_at"].as_i64().filter(|r| *r > 0);
                if w["is_current"] == false || resets_at.is_some_and(|r| r <= now) {
                    continue;
                }
                // A percentage without a reset cannot be placed in the
                // current window, so it is history, not a reading.
                let Some(pct) = resets_at
                    .and(w["used_percent"]["value"].as_f64())
                    .filter(|_| w["used_percent"]["state"] == "ok")
                    .filter(|p| p.is_finite() && (0. ..=100.).contains(p))
                else {
                    continue;
                };
                let pace = &w["pace"];
                let known = pace["known"] == true
                    && matches!(
                        pace["state"].as_str(),
                        Some("on_track" | "ahead" | "overspending")
                    );
                windows.push(Window {
                    label,
                    pct,
                    resets_at,
                    pace: known.then(|| pace["state"].as_str().unwrap_or_default().to_owned()),
                    expected_pct: known.then(|| pace["expectedPct"].as_f64()).flatten(),
                });
            }
            if windows.is_empty() {
                continue;
            }
            let label = account["label"].as_str().unwrap_or_default();
            let title = if account["is_default"] == true || label.is_empty() || label == "default" {
                provider_name(id)
            } else {
                format!("{} · {label}", provider_name(id))
            };
            out.push(Account {
                provider: id.to_owned(),
                title,
                stale: account["fresh"] == false,
                failure: account["failure"]["detail"]
                    .as_str()
                    .filter(|d| !d.is_empty())
                    .map(|d| crate::transcript::head(d, 200)),
                windows,
            });
        }
    }
    out
}

/// "2h 14m", "3d 4h", "12m" until `resets_at`.
pub fn resets_in(resets_at: i64, now: i64) -> String {
    let secs = (resets_at - now).max(0);
    let (days, hours, minutes) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    match (days, hours) {
        (0, 0) => format!("{}m", minutes.max(1)),
        (0, h) => format!("{h}h {minutes}m"),
        (d, h) => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accounts_keep_only_current_measured_windows() {
        let now = 1_000_000;
        let ok = |pct: f64, reset: i64| json!({"used_percent":{"state":"ok","value":pct},"resets_at":reset,"is_current":true});
        let report = json!({"providers":[
            {"provider":"claude","accounts":[
                {"label":"default","is_default":true,"fresh":true,"windows":{
                    "five_hour": {"used_percent":{"state":"ok","value":42.0},"resets_at":now+3600,
                        "pace":{"known":true,"state":"on_track","expectedPct":40.0}},
                    "seven_day": ok(18.0, now + 86_400),
                    "monthly": {"used_percent":{"state":"unavailable","reason":"no extra usage"}}
                }},
                {"label":"work","is_default":false,"fresh":false,"windows":{
                    "five_hour": ok(90.0, now - 10),
                    "seven_day": {"used_percent":{"state":"ok","value":5.0},"resets_at":null},
                    "monthly": ok(61.0, now + 10)
                }},
                {"label":"empty","windows":{"five_hour":{"used_percent":{"state":"unknown","reason":"expired"}}}}
            ]},
            {"provider":"codex","accounts":[{"label":"","windows":{"five_hour": ok(7.0, now + 60)}}]}
        ]});
        let accounts = accounts(&report, now);
        assert_eq!(accounts.len(), 3, "the unmeasured account is not shown");
        assert_eq!(accounts[0].title, "Claude");
        assert_eq!(accounts[0].windows.len(), 2);
        assert_eq!(accounts[0].windows[0].pace.as_deref(), Some("on_track"));
        assert_eq!(accounts[0].windows[0].expected_pct, Some(40.));
        // A rolled-over window and a reset-less reading are dropped.
        assert_eq!(accounts[1].title, "Claude · work");
        assert!(accounts[1].stale);
        assert_eq!(accounts[1].windows.len(), 1);
        assert_eq!(accounts[1].windows[0].label, "Month");
        assert_eq!(accounts[2].title, "Codex");
        assert_eq!(accounts[0].primary().label, "5h");
        assert_eq!(
            accounts[1].primary().label,
            "Month",
            "the shortest window present"
        );
    }

    #[test]
    fn freshness_reads_naturally() {
        assert_eq!(ago(100, 105), "just now");
        assert_eq!(ago(100, 145), "45s ago");
        assert_eq!(ago(100, 100 + 180), "3m ago");
        assert_eq!(
            evaluated_at(&json!({"evaluated_at":7,"generated_at":3})),
            Some(7)
        );
        assert_eq!(evaluated_at(&json!({"generated_at":3})), Some(3));
    }

    #[test]
    fn reset_countdowns_read_naturally() {
        assert_eq!(resets_in(100, 100), "1m");
        assert_eq!(resets_in(100 + 8_040, 100), "2h 14m");
        assert_eq!(resets_in(100 + 3 * 86_400 + 4 * 3_600, 100), "3d 4h");
    }
}

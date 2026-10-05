//! Account usage windows from the hub's `usage.report`: every provider, every
//! account, with no live session. Unavailable windows and windows that rolled
//! over are skipped, and a percentage is shown only when it was measured for
//! the window open now. A login whose windows should have a reading but do not
//! (expired sign-in, failed poll, not polled yet) stays listed with that state
//! instead of silently vanishing; only providers that publish no windows at
//! all (and the hub's "unattributed" transcript bucket) are left out.
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
    /// Set when no window has a current reading: a short state ("Sign in
    /// again", "Refresh failed", "No reading yet") and the hub's reason.
    pub unmeasured: Option<Unmeasured>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unmeasured {
    pub state: &'static str,
    pub reason: String,
    /// A failure the user can act on, rather than a reading still to come.
    pub error: bool,
}

impl Account {
    /// The window to show when there is room for one: the shortest, which
    /// is the one that blocks soonest (5h, else weekly, else monthly).
    pub fn primary(&self) -> Option<&Window> {
        self.windows.first()
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
            // The transcript bucket aggregates sessions whose login is not
            // known; it is not an account and has no windows of its own.
            if account["source"] == "transcript" || account["label"] == "unattributed" {
                continue;
            }
            let failure_kind = account["failure"]["kind"].as_str().unwrap_or_default();
            let failure_detail = account["failure"]["detail"]
                .as_str()
                .filter(|d| !d.is_empty())
                .map(|d| crate::transcript::head(d, 200));
            let unmeasured = if windows.is_empty() {
                let states = WINDOWS
                    .iter()
                    .map(|(key, _)| &account["windows"][key]["used_percent"])
                    .filter(|w| !w.is_null())
                    .collect::<Vec<_>>();
                // Every window structurally unavailable (a plan with no
                // limits, a provider that publishes none) and nothing failed:
                // there is no usage to show, so the account is left out.
                if failure_detail.is_none() && states.iter().all(|w| w["state"] == "unavailable") {
                    continue;
                }
                let reason = failure_detail.clone().unwrap_or_else(|| {
                    states
                        .iter()
                        .find(|w| w["state"] != "unavailable")
                        .and_then(|w| w["reason"].as_str())
                        .map(|r| crate::transcript::head(r, 200))
                        .unwrap_or_else(|| {
                            "The hub has no current reading for this account.".into()
                        })
                });
                let reauth = failure_kind == "needs_reauth"
                    || reason.to_lowercase().starts_with("needsreauth");
                Some(if reauth {
                    Unmeasured {
                        state: "Sign in again",
                        reason,
                        error: true,
                    }
                } else if failure_detail.is_some() {
                    Unmeasured {
                        state: "Refresh failed",
                        reason,
                        error: true,
                    }
                } else {
                    Unmeasured {
                        state: "No reading yet",
                        reason,
                        error: false,
                    }
                })
            } else {
                None
            };
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
                failure: failure_detail,
                windows,
                unmeasured,
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
        assert_eq!(accounts.len(), 4);
        assert_eq!(accounts[0].title, "Claude");
        assert_eq!(accounts[0].windows.len(), 2);
        assert_eq!(accounts[0].windows[0].pace.as_deref(), Some("on_track"));
        assert_eq!(accounts[0].windows[0].expected_pct, Some(40.));
        assert_eq!(accounts[0].unmeasured, None);
        // A rolled-over window and a reset-less reading are dropped.
        assert_eq!(accounts[1].title, "Claude · work");
        assert!(accounts[1].stale);
        assert_eq!(accounts[1].windows.len(), 1);
        assert_eq!(accounts[1].windows[0].label, "Month");
        // An account with no current reading stays listed, explained.
        assert_eq!(accounts[2].title, "Claude · empty");
        assert!(accounts[2].primary().is_none());
        let state = accounts[2].unmeasured.as_ref().unwrap();
        assert_eq!((state.state, state.error), ("No reading yet", false));
        assert_eq!(state.reason, "expired");
        assert_eq!(accounts[3].title, "Codex");
        assert_eq!(accounts[0].primary().unwrap().label, "5h");
        assert_eq!(
            accounts[1].primary().unwrap().label,
            "Month",
            "the shortest window present"
        );
    }

    /// The shape claudemon reported on 2026-10-04 when the sidebar showed
    /// only Codex: a Claude login needing sign-in, the transcript bucket, a
    /// Codex plan with only a weekly window, and Copilot without any windows.
    #[test]
    fn unmeasured_logins_are_explicit_and_unmetered_ones_are_left_out() {
        let now = 1_000_000;
        let reauth = json!({"used_percent":{"state":"unknown","reason":"NeedsReauth: oauth token expired"},"resets_at":null});
        let none = |why: &str| json!({"used_percent":{"state":"unavailable","reason":why},"resets_at":null});
        let report = json!({"providers":[
            {"provider":"claude","accounts":[
                {"account":"","label":"default","is_default":true,"source":"oauth_poll","fresh":null,"failure":null,"windows":{
                    "five_hour":{"used_percent":{"state":"unknown","reason":"not polled yet"},"resets_at":null},
                    "seven_day":{"used_percent":{"state":"unknown","reason":"not polled yet"},"resets_at":null},
                    "monthly":none("extra usage is not enabled")}},
                {"account":"/home/u/.claude/accounts/work","label":"work","is_default":false,"source":"oauth_poll",
                    "failure":{"kind":"needs_reauth","detail":"oauth token expired","at":now - 60},
                    "windows":{"five_hour":reauth,"seven_day":reauth,"monthly":reauth}},
                {"account":null,"label":"unattributed","is_default":false,"source":"transcript","failure":null,
                    "windows":{"five_hour":{"used_percent":{"state":"unknown","reason":"no account"}}}}
            ]},
            {"provider":"codex","accounts":[{"label":"pro","is_default":true,"source":"disk","failure":null,"windows":{
                "five_hour":none("plan publishes no five-hour window"),
                "seven_day":{"used_percent":{"state":"ok","value":9.0},"resets_at":now + 86_400,"is_current":true},
                "monthly":none("Codex publishes no monthly window")}}]},
            {"provider":"copilot","accounts":[{"label":"copilot","is_default":true,"source":"disk","failure":null,"windows":{
                "five_hour":none("no quota record"),"seven_day":none("no quota record"),"monthly":none("no quota record")}}]}
        ]});
        let accounts = accounts(&report, now);
        let titles: Vec<_> = accounts.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["Claude", "Claude · work", "Codex"]);
        let claude = accounts[0].unmeasured.as_ref().unwrap();
        assert_eq!((claude.state, claude.error), ("No reading yet", false));
        assert_eq!(claude.reason, "not polled yet");
        let work = accounts[1].unmeasured.as_ref().unwrap();
        assert_eq!((work.state, work.error), ("Sign in again", true));
        assert_eq!(work.reason, "oauth token expired");
        assert_eq!(accounts[2].primary().unwrap().label, "Week");
        assert_eq!(accounts[2].unmeasured, None);
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

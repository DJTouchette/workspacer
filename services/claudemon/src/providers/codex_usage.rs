//! Codex account usage, read from disk with no live session.
//!
//! Codex reports its rate-limit windows only on the wire, inside the
//! `token_count` event of a running session — so a daemon that has not spawned
//! a Codex agent since boot knows nothing about the account, and a UI asking
//! "how much of my weekly window is left" gets silence. Silence renders as
//! zero, and zero is a lie.
//!
//! The same events are ALSO durably persisted: every session's
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl` records each `token_count`
//! verbatim, `rate_limits` included. This module reads the newest one back.
//!
//! Two sources, ranked, because they answer different questions:
//!
//!   1. the ROLLOUTS — the authority for the windows (`primary`/`secondary`
//!      percentages, `resets_at`, `plan_type`, credits). Plain JSONL, always
//!      readable, no lock to contend with.
//!   2. `state_5.sqlite` `threads.tokens_used` — the SECONDARY source, and only
//!      for cumulative token totals. Verified against a rollout on the author's
//!      machine 2026-08-28: thread `01a04992…` reads 3 886 013 in both, exactly.
//!      Best-effort: Codex holds this DB open in WAL mode, so a read may fail,
//!      and a failure here must degrade the totals to UNKNOWN rather than 0.
//!
//! STALENESS IS PART OF THE READING. A persisted percentage describes the
//! window that was open when it was written. Once that window's `resets_at` has
//! passed, the number is not a small number — it is a number about a window
//! that no longer exists. [`CodexWindow::is_current`] says which, and callers
//! must not render an expired window as a current utilization.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// One rate-limit window as Codex persists it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CodexWindow {
    pub used_percent: Option<f64>,
    /// Codex reports its own window length; a "5h" window is 300 minutes and
    /// the weekly one 10080, but both are read rather than assumed.
    pub window_minutes: Option<u64>,
    pub resets_at: Option<i64>,
}

impl CodexWindow {
    /// Whether this window is still the one being measured. A reading whose
    /// `resets_at` has passed describes a window that has since rolled over:
    /// the true current utilization is unknown (likely lower), and rendering
    /// the old percentage would overstate it. `None` when Codex reported no
    /// reset time — unknowable, so not claimed either way.
    pub fn is_current(&self, now: i64) -> Option<bool> {
        self.resets_at.map(|r| r > now)
    }
}

/// A whole account-level reading recovered from disk.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodexUsage {
    /// The shorter window Codex calls `primary` (300 minutes in every capture).
    pub five_hour: Option<CodexWindow>,
    /// The longer window Codex calls `secondary` (10080 minutes = 7 days).
    pub seven_day: Option<CodexWindow>,
    /// e.g. `"team"`, `"plus"`. Straight from the event; never inferred.
    pub plan_type: Option<String>,
    pub has_credits: Option<bool>,
    pub unlimited_credits: Option<bool>,
    pub credit_balance: Option<f64>,
    /// The rollout event's own timestamp (epoch seconds), which is how old this
    /// reading is. `None` if the line carried no parsable timestamp.
    pub observed_at: Option<i64>,
    /// Which rollout it came from — so a caller can say where the number is from.
    pub source: PathBuf,
    /// Cumulative tokens for the thread that rollout belongs to, from the same
    /// event's `total_token_usage`.
    pub thread_total_tokens: Option<u64>,
    /// Cumulative tokens across EVERY thread Codex has recorded, from
    /// `state_5.sqlite`. `None` means that DB could not be read — unknown, not
    /// zero. See [`all_thread_tokens`].
    pub all_thread_tokens: Option<u64>,
    /// How many threads that total covers. `None` for the same reason.
    pub thread_count: Option<u64>,
}

/// Why a disk read produced nothing. Distinguished so a caller can say which,
/// rather than collapsing all three into an empty gauge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexUsageError {
    /// No `$CODEX_HOME` / home directory could be resolved at all.
    NoCodexHome,
    /// Codex is installed but has never recorded a session here.
    NoRollouts,
    /// Rollouts exist, but none of the ones scanned carried a `rate_limits`
    /// block. Codex only emits one once a turn has actually billed, so a
    /// freshly created or purely local session legitimately has none.
    NoRateLimits,
}

impl std::fmt::Display for CodexUsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoCodexHome => "no codex home directory",
            Self::NoRollouts => "codex has recorded no sessions",
            Self::NoRateLimits => "no rollout carried a rate_limits block",
        })
    }
}

/// How many of the newest rollouts to look through before giving up. The
/// newest usually answers; the walk continues because the newest session can
/// legitimately be one that never billed a turn (`NoRateLimits` per file).
const MAX_ROLLOUTS_SCANNED: usize = 8;

/// How much of a rollout's tail to read looking for its last `token_count`.
/// Codex writes one per turn and they are the last thing a session logs, so the
/// tail is where they are; the largest rollout on the author's machine is 9.5
/// MB and reading it whole to find a line near its end would be waste. Widened
/// once (see [`WIDE_TAIL_BYTES`]) before the file is abandoned.
const TAIL_BYTES: u64 = 256 * 1024;
const WIDE_TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// The account's most recent reliable window reading, from disk alone.
///
/// Scans rollouts newest-first and returns the first that carries a
/// `rate_limits` block, which is the most recent one Codex actually wrote.
/// Whether that reading is still CURRENT is a separate question the caller
/// answers with [`CodexWindow::is_current`] — a stale reading is still real
/// information ("you were at 67% until 14:00") and is not silently discarded.
pub fn read_from_disk() -> Result<CodexUsage, CodexUsageError> {
    let home = super::codex_rollout::codex_home().ok_or(CodexUsageError::NoCodexHome)?;
    let mut rollouts = super::codex_rollout::collect_rollouts(&home.join("sessions"));
    if rollouts.is_empty() {
        return Err(CodexUsageError::NoRollouts);
    }
    rollouts.sort_by_key(|(_, m)| std::cmp::Reverse(*m));

    for (path, _) in rollouts.into_iter().take(MAX_ROLLOUTS_SCANNED) {
        let Some(line) = last_rate_limited_event(&path) else {
            continue;
        };
        let Some(usage) = usage_from_event(&line, &path) else {
            continue;
        };
        let (all_thread_tokens, thread_count) = all_thread_tokens(&home);
        return Ok(CodexUsage {
            all_thread_tokens,
            thread_count,
            ..usage
        });
    }
    Err(CodexUsageError::NoRateLimits)
}

/// Build a reading out of one rollout line. Separate from the file walk so the
/// shape can be tested against a verbatim capture with no filesystem at all.
pub fn usage_from_event(line: &Value, source: &Path) -> Option<CodexUsage> {
    let payload = line.get("payload").unwrap_or(line);
    let limits = payload.get("rate_limits")?;
    // Which window is which is decided by `window_minutes`, NOT by the
    // primary/secondary key — reusing the one classifier the live wire path
    // already uses, so an offline reading can never disagree with a live one.
    let super::AgentUpdate::RateLimits {
        five_hour_pct,
        five_hour_resets_at,
        five_hour_window_minutes,
        seven_day_pct,
        seven_day_resets_at,
        seven_day_window_minutes,
        ..
    } = super::rate_limits_from(limits)?
    else {
        return None;
    };
    let window = |pct: Option<f64>, resets: Option<i64>, mins: Option<u64>| {
        (pct.is_some() || resets.is_some()).then_some(CodexWindow {
            used_percent: pct,
            window_minutes: mins,
            resets_at: resets,
        })
    };
    let credits = limits.get("credits");
    Some(CodexUsage {
        five_hour: window(five_hour_pct, five_hour_resets_at, five_hour_window_minutes),
        seven_day: window(seven_day_pct, seven_day_resets_at, seven_day_window_minutes),
        plan_type: limits
            .get("plan_type")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        has_credits: credits
            .and_then(|c| c.get("has_credits"))
            .and_then(Value::as_bool),
        unlimited_credits: credits
            .and_then(|c| c.get("unlimited"))
            .and_then(Value::as_bool),
        credit_balance: credits
            .and_then(|c| c.get("balance"))
            .and_then(Value::as_f64),
        observed_at: line.get("timestamp").and_then(parse_timestamp),
        source: source.to_path_buf(),
        thread_total_tokens: payload
            .get("info")
            .and_then(|i| i.get("total_token_usage"))
            .and_then(|t| t.get("total_tokens"))
            .and_then(Value::as_u64),
        all_thread_tokens: None,
        thread_count: None,
    })
}

/// Epoch seconds from Codex's RFC3339 rollout timestamp (or a bare number).
fn parse_timestamp(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    time::OffsetDateTime::parse(v.as_str()?, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

/// The last line of `path` that is a `token_count` event carrying
/// `rate_limits`. Reads a bounded tail rather than the whole file, widening
/// once before giving up.
fn last_rate_limited_event(path: &Path) -> Option<Value> {
    for budget in [TAIL_BYTES, WIDE_TAIL_BYTES] {
        let text = read_tail(path, budget)?;
        let found = text
            .lines()
            .rev()
            // A tail can begin mid-line; a partial line simply fails to parse,
            // which is exactly the behaviour wanted — no special case needed.
            .filter(|l| l.contains("\"rate_limits\""))
            .find_map(|l| serde_json::from_str::<Value>(l).ok());
        if found.is_some() {
            return found;
        }
        // Whole file already read — widening cannot help.
        if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) <= budget {
            return None;
        }
    }
    None
}

/// The last `budget` bytes of a file as UTF-8, lossily. `None` if unreadable.
fn read_tail(path: &Path, budget: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(budget);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::with_capacity((len - from) as usize);
    f.take(budget).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Cumulative tokens across every thread Codex has recorded, from
/// `state_5.sqlite` — the SECONDARY source named in the task, and used only for
/// this total. Returns `(None, None)` on any failure, because Codex holds this
/// database open in WAL mode and a read can legitimately lose: an unreadable DB
/// means the total is UNKNOWN, and reporting 0 would claim the account had
/// spent nothing.
///
/// Read WAL-aware via [`super::open_foreign_sqlite_readonly`], which explains
/// why `immutable=1` is the fallback and not the default. Measured here on
/// 2026-08-28: the WAL-aware read returns 222 588 411 tokens over 54 threads,
/// the immutable one 221 939 543 over 53 — one uncheckpointed session. Codex's
/// gap is small; Copilot's is everything, which is why the helper is shared.
fn all_thread_tokens(codex_home: &Path) -> (Option<u64>, Option<u64>) {
    let db = codex_home.join("state_5.sqlite");
    if !db.is_file() {
        return (None, None);
    }
    let Some(conn) = super::open_foreign_sqlite_readonly(&db) else {
        return (None, None);
    };
    conn.query_row(
        "SELECT COALESCE(SUM(tokens_used), 0), COUNT(*) FROM threads",
        [],
        |r| {
            Ok((
                Some(r.get::<_, i64>(0)?.max(0) as u64),
                Some(r.get::<_, i64>(1)?.max(0) as u64),
            ))
        },
    )
    .unwrap_or((None, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// VERBATIM capture from `~/.codex/sessions/2026/08/28/rollout-2026-08-28
    /// T12-12-14-01a04992-….jsonl`, taken on 2026-08-28. Trimmed of nothing
    /// that this module reads.
    fn live_capture() -> Value {
        json!({
          "timestamp": "2026-08-28T19:02:06.134Z",
          "type": "event_msg",
          "payload": {
            "type": "token_count",
            "info": {
              "total_token_usage": {
                "input_tokens": 3857750, "cached_input_tokens": 3717888,
                "cache_write_input_tokens": 0, "output_tokens": 28263,
                "reasoning_output_tokens": 6929, "total_tokens": 3886013
              },
              "last_token_usage": { "total_tokens": 114909 },
              "model_context_window": 258400
            },
            "rate_limits": {
              "limit_id": "codex", "limit_name": null,
              "primary": { "used_percent": 67.0, "window_minutes": 300,
                           "resets_at": 1787953526 },
              "secondary": { "used_percent": 11.0, "window_minutes": 10080,
                             "resets_at": 1788492430 },
              "credits": { "has_credits": false, "unlimited": false, "balance": null },
              "individual_limit": null, "spend_control_reached": null,
              "plan_type": "team", "rate_limit_reached_type": null
            }
          }
        })
    }

    #[test]
    fn reads_every_field_the_live_shape_carries() {
        let u = usage_from_event(&live_capture(), Path::new("/tmp/r.jsonl")).unwrap();
        let five = u.five_hour.as_ref().unwrap();
        assert_eq!(five.used_percent, Some(67.0));
        assert_eq!(five.window_minutes, Some(300));
        assert_eq!(five.resets_at, Some(1787953526));
        let seven = u.seven_day.as_ref().unwrap();
        assert_eq!(seven.used_percent, Some(11.0));
        assert_eq!(seven.window_minutes, Some(10080));
        assert_eq!(u.plan_type.as_deref(), Some("team"));
        assert_eq!(u.has_credits, Some(false));
        assert_eq!(u.unlimited_credits, Some(false));
        assert_eq!(u.credit_balance, None, "a null balance is unknown, not 0");
        // 2026-08-28T19:02:06Z. Cross-checks against the live sqlite: this
        // thread's `threads.updated_at` is 1787943726 too.
        assert_eq!(u.observed_at, Some(1787943726));
        // Matches `threads.tokens_used` for this thread exactly (verified on
        // the live DB, 2026-08-28) — which is why the sqlite side is a
        // secondary source rather than a second opinion.
        assert_eq!(u.thread_total_tokens, Some(3886013));
    }

    /// The windows are classified by their reported length, not by the
    /// `primary`/`secondary` key. Codex's "5h slot" is configurable, so a key
    /// that carried a 7-day window would otherwise be filed as the short one.
    #[test]
    fn windows_are_classified_by_duration_not_by_key() {
        let v = json!({ "payload": { "rate_limits": {
            "primary": { "used_percent": 9.0, "window_minutes": 10080, "resets_at": 100 },
            "secondary": { "used_percent": 80.0, "window_minutes": 300, "resets_at": 50 }
        }}});
        let u = usage_from_event(&v, Path::new("/tmp/r.jsonl")).unwrap();
        assert_eq!(u.five_hour.unwrap().used_percent, Some(80.0));
        assert_eq!(u.seven_day.unwrap().used_percent, Some(9.0));
    }

    /// A persisted percentage is about the window that was open when it was
    /// written. Past its reset it describes a window that no longer exists, and
    /// a caller that renders it as the current figure overstates usage — so the
    /// reading carries the answer rather than making every caller derive it.
    #[test]
    fn an_expired_window_says_so_instead_of_passing_as_current() {
        let u = usage_from_event(&live_capture(), Path::new("/tmp/r.jsonl")).unwrap();
        let five = u.five_hour.unwrap();
        assert_eq!(five.is_current(1787953525), Some(true), "before the reset");
        assert_eq!(five.is_current(1787953526), Some(false), "at the reset");
        assert_eq!(five.is_current(1787999999), Some(false), "long after");
        // No reset time reported → not claimed either way.
        assert_eq!(
            CodexWindow {
                used_percent: Some(5.0),
                window_minutes: None,
                resets_at: None
            }
            .is_current(0),
            None,
        );
    }

    #[test]
    fn an_event_without_rate_limits_is_not_a_reading() {
        let v = json!({ "payload": { "type": "token_count", "info": {} } });
        assert!(usage_from_event(&v, Path::new("/tmp/r.jsonl")).is_none());
    }

    /// The tail read has to survive starting mid-line, which it will on every
    /// real file — and it has to find the LAST reading, not the first.
    #[test]
    fn the_tail_scan_finds_the_newest_reading_across_a_partial_first_line() {
        let dir = std::env::temp_dir().join(format!(
            "wks-codex-usage-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-test.jsonl");
        let mut body = String::new();
        // Junk large enough that a 256 KiB tail cannot start at a line break.
        body.push_str(&format!("{}\n", "x".repeat(300 * 1024)));
        for pct in [10.0, 42.0] {
            body.push_str(&format!(
                "{}\n",
                json!({ "timestamp": "2026-08-28T19:02:06.134Z", "payload": {
                    "type": "token_count",
                    "rate_limits": { "primary": { "used_percent": pct,
                                                  "window_minutes": 300,
                                                  "resets_at": 1 } } } })
            ));
        }
        std::fs::write(&path, &body).unwrap();

        let line = last_rate_limited_event(&path).expect("a reading is found");
        let u = usage_from_event(&line, &path).unwrap();
        assert_eq!(
            u.five_hour.unwrap().used_percent,
            Some(42.0),
            "the LAST reading in the file is the current one",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live smoke test against this machine's real `~/.codex`. Ignored by
    /// default — it asserts on whatever the user's account happens to say:
    /// `cargo test -p claudemon codex_disk_read_smoke -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the real ~/.codex"]
    fn codex_disk_read_smoke() {
        let u = read_from_disk().expect("a reading");
        println!("{u:#?}");
        assert!(u.five_hour.is_some() || u.seven_day.is_some());
    }
}

//! Account-level usage poller — the rate-limit gauges for stream sessions.
//!
//! The PTY transport learns the 5h/7d utilization from Claude's interactive
//! statusLine JSON, but the headless stream transport has no statusLine: its
//! only wire source is `rate_limit_event`, which in practice carries `resetsAt`
//! *without* `utilization` (verified against live CLI captures — the percent
//! only appears on warning-threshold events). So a pure-stream fleet never
//! learns how much of a window it has used.
//!
//! This module fills the gap from the account side: it fetches the same OAuth
//! usage endpoint Claude Code's own `/usage` screen reads, using the CLI's
//! stored credentials. The reading is *account*-scoped (the windows are shared
//! by every session on the account) — but "the account" is per CLAUDE config
//! root: a profile spawn with its own `CLAUDE_CONFIG_DIR` (workspacer's
//! second-account profiles) is a different login with different windows. So
//! the daemon keeps one reading PER config root in [`SessionStore`], keyed by
//! the root each session's transcript path reveals, and patches each Claude
//! session's status line from its own root's reading — see
//! `SessionStore::apply_status_line` / `set_account_usage`. A single global
//! copy here used to stamp the default account's gauges onto every session,
//! overwriting a second account's own wire-reported windows.
//!
//! Cost: zero tokens. This is an account-metadata query, not an inference
//! call. Failure is always soft — no credentials, an expired token, or a
//! non-200 just means the gauges stay wire-fed.

use anyhow::{Context, Result};
use serde_json::Value;
use time::OffsetDateTime;

/// One account-level rate-limit reading, in the same vocabulary as
/// [`super::state::StatusLine`]'s window fields.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountUsage {
    pub five_hour_pct: Option<f64>,
    pub five_hour_resets_at: Option<i64>,
    pub seven_day_pct: Option<f64>,
    pub seven_day_resets_at: Option<i64>,
    /// The monthly overage/credit window (`extra_usage`) — only reported while
    /// extra usage is enabled; a disabled overage would otherwise pin a noisy
    /// permanent 0% meter on every session.
    pub monthly_pct: Option<f64>,
    pub monthly_resets_at: Option<i64>,
    pub out_of_credits: Option<bool>,
    /// When the daemon fetched this. Readings age out — see [`Self::is_fresh`].
    pub fetched_at: OffsetDateTime,
}

/// How long a reading stays authoritative. Past this the poller has stopped
/// (no live Claude sessions) or is failing, and per-session wire data — even
/// stale — is better than a dead account snapshot.
const FRESH_FOR_SECS: i64 = 5 * 60;

impl AccountUsage {
    pub fn is_fresh(&self, now: OffsetDateTime) -> bool {
        (now - self.fetched_at).whole_seconds() < FRESH_FOR_SECS
    }
}

/// Parse the `/api/oauth/usage` response body. Shape (captured live,
/// 2026-07-15):
///
/// ```json
/// { "five_hour": { "utilization": 19.0, "resets_at": null, ... },
///   "seven_day": { "utilization": 3.0, "resets_at": null, ... },
///   "extra_usage": { "is_enabled": false, "utilization": 0.0,
///                    "disabled_reason": "out_of_credits", ... } }
/// ```
///
/// `utilization` is 0–100 (the same scale as the stream `rate_limit_event`).
/// `resets_at` was `null` in captures; accept either epoch seconds or an
/// RFC3339 string so a populated value lands whichever way it's spelled.
pub fn parse_usage_response(v: &Value) -> AccountUsage {
    let window = |name: &str| {
        let w = v.get(name);
        (
            w.and_then(|w| w.get("utilization")).and_then(Value::as_f64),
            w.and_then(|w| w.get("resets_at")).and_then(parse_resets_at),
        )
    };
    let (five_hour_pct, five_hour_resets_at) = window("five_hour");
    let (seven_day_pct, seven_day_resets_at) = window("seven_day");

    let extra = v.get("extra_usage");
    let extra_enabled = extra
        .and_then(|e| e.get("is_enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (monthly_pct, monthly_resets_at) = if extra_enabled {
        window("extra_usage")
    } else {
        (None, None)
    };
    let out_of_credits = extra
        .and_then(|e| e.get("disabled_reason"))
        .and_then(Value::as_str)
        .map(|r| r == "out_of_credits");

    AccountUsage {
        five_hour_pct,
        five_hour_resets_at,
        seven_day_pct,
        seven_day_resets_at,
        monthly_pct,
        monthly_resets_at,
        out_of_credits,
        fetched_at: OffsetDateTime::now_utc(),
    }
}

/// Epoch seconds from either a JSON number or an RFC3339 string.
fn parse_resets_at(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    let s = v.as_str()?;
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

/// The daemon's default Claude config root (`~/.claude`, or the daemon's own
/// `CLAUDE_CONFIG_DIR` override). Sessions running here use the empty-string
/// root key.
pub fn default_config_root() -> String {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return trimmed.trim_end_matches(['/', '\\']).to_string();
        }
    }
    dirs_home()
        .map(|h| h.join(".claude").to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The Claude config root a transcript path reveals: Claude Code writes every
/// transcript at `<CLAUDE_CONFIG_DIR>/projects/<slug>/<id>.jsonl`, so the root
/// is everything before the last `/projects/` component (the slug is a single
/// dash-flattened component and can never contain a separator). `None` when
/// the path has no `projects` component.
pub fn root_from_transcript(path: &str) -> Option<String> {
    // Match either separator; byte offsets are shared because the replacement
    // is 1:1, so we can slice the ORIGINAL string (Windows paths keep their
    // backslashes for later joins).
    let norm = path.replace('\\', "/");
    let idx = norm.rfind("/projects/")?;
    if idx == 0 {
        return None; // "/projects/x.jsonl" — no root before it
    }
    Some(path[..idx].to_string())
}

/// Canonical root key: trailing separators trimmed, and the daemon's default
/// root collapsed to `""` so sessions that predate their first hook (no
/// transcript yet → unknown root → default) share a key with sessions whose
/// transcript spells the default root out.
pub fn normalize_root(root: &str) -> String {
    let trimmed = root.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed == default_config_root() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

/// Why an account's usage could not be read. The distinction is load-bearing:
/// a UI that renders any of these as `0%` tells the user they have a full
/// window when the truth is that we do not know, and `NeedsReauth` is
/// specifically actionable in a way the others are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageFailure {
    /// Credentials exist, but their OAuth token has expired. The daemon
    /// deliberately does NOT refresh it — rotation is the CLI's job and racing
    /// it can invalidate the CLI's own stored token — so this persists until
    /// that account's CLI runs again. Measured live on 2026-08-28: the `work`
    /// profile's token expired 2026-08-20 and has stayed expired since.
    NeedsReauth,
    /// No readable credentials at all: an API-key / Bedrock setup, a config
    /// root that was never logged in, or (macOS, named roots only) a login
    /// that lives in the Keychain under a service entry we cannot name.
    NoCredentials,
    /// The endpoint could not be reached or refused the request — offline,
    /// timeout, 5xx, or an auth rejection the local expiry check did not catch.
    Unreachable,
}

/// A classified fetch failure. Carries the human detail as well as the kind so
/// a log line and a wire field can both say something true.
#[derive(Debug, Clone)]
pub struct UsageError {
    pub kind: UsageFailure,
    pub detail: String,
}

impl UsageError {
    fn new(kind: UsageFailure, detail: impl std::fmt::Display) -> Self {
        Self {
            kind,
            detail: detail.to_string(),
        }
    }
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.detail)
    }
}

impl std::error::Error for UsageError {}

/// The CLI's OAuth access token for one config root. `""` (the default root)
/// reads `~/.claude/.credentials.json` with a macOS-Keychain fallback; a named
/// root reads `<root>/.credentials.json` only — a second account's login on
/// macOS may live in the Keychain under an unknown service entry, in which
/// case this soft-fails and that account's gauges stay wire-fed.
///
/// Never refreshes: rotation is the CLI's job (racing it with our own refresh
/// could invalidate the CLI's stored token). An expired token is an `Err` and
/// the poll simply retries next tick — the CLI refreshes it on its next turn.
fn read_access_token(root: &str) -> std::result::Result<String, UsageError> {
    match read_credentials_json(root) {
        Ok(creds) => token_from_credentials(&creds),
        Err(file_err) => {
            #[cfg(target_os = "macos")]
            {
                if root.is_empty() {
                    if let Ok(out) = std::process::Command::new("security")
                        .args([
                            "find-generic-password",
                            "-s",
                            "Claude Code-credentials",
                            "-w",
                        ])
                        .output()
                    {
                        if out.status.success() {
                            if let Ok(creds) = serde_json::from_slice::<Value>(
                                String::from_utf8_lossy(&out.stdout).trim().as_bytes(),
                            ) {
                                return token_from_credentials(&creds);
                            }
                        }
                    }
                }
            }
            Err(UsageError::new(UsageFailure::NoCredentials, format!("{file_err:#}")))
        }
    }
}

fn read_credentials_json(root: &str) -> Result<Value> {
    let path = if root.is_empty() {
        dirs_home()
            .context("no home directory")?
            .join(".claude")
            .join(".credentials.json")
    } else {
        std::path::PathBuf::from(root).join(".credentials.json")
    };
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&raw).context("parsing credentials json")
}

fn dirs_home() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Extract + validate the token from the credentials document.
fn token_from_credentials(creds: &Value) -> std::result::Result<String, UsageError> {
    let oauth = creds.get("claudeAiOauth").ok_or_else(|| {
        UsageError::new(
            UsageFailure::NoCredentials,
            "no claudeAiOauth in credentials",
        )
    })?;
    // `expiresAt` is epoch milliseconds. A stale token would just 401; skip
    // the round-trip when we can tell locally — and report it as its own kind,
    // because "this account needs the CLI to log in again" is a different
    // thing to tell a user than "we could not reach the endpoint".
    if let Some(expires_ms) = oauth.get("expiresAt").and_then(Value::as_i64) {
        if expires_ms <= OffsetDateTime::now_utc().unix_timestamp() * 1000 {
            return Err(UsageError::new(
                UsageFailure::NeedsReauth,
                "oauth token expired (the CLI refreshes it on its next turn; \
                 the daemon deliberately does not)",
            ));
        }
    }
    oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| UsageError::new(UsageFailure::NoCredentials, "no accessToken in credentials"))
}

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// One fetch for one config root: credentials → GET → parsed reading.
pub async fn fetch_account_usage(
    client: &reqwest::Client,
    root: &str,
) -> std::result::Result<AccountUsage, UsageError> {
    let token = read_access_token(root)?;
    let resp = client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|err| UsageError::new(UsageFailure::Unreachable, format!("usage request failed: {err}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        // A 401/403 after our local expiry check passed still means the token
        // is no good — surface it as reauth, not as a network blip.
        let kind = if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            UsageFailure::NeedsReauth
        } else {
            UsageFailure::Unreachable
        };
        return Err(UsageError::new(
            kind,
            format!("usage endpoint returned {status}"),
        ));
    }
    let body: Value = resp.json().await.map_err(|err| {
        UsageError::new(
            UsageFailure::Unreachable,
            format!("parsing usage response: {err}"),
        )
    })?;
    Ok(parse_usage_response(&body))
}

/// Where the desktop records the Claude account profiles the user has set up.
/// Read-only, and read fresh on every discovery pass so a profile added while
/// the daemon is running is picked up without a restart. Owned by
/// `apps/desktop/src/main/services/claudeProfiles.ts`; the daemon only ever
/// looks at `profiles[].configDir`.
fn profiles_file() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        let dir = dir.trim();
        if !dir.is_empty() {
            return Some(
                std::path::PathBuf::from(dir)
                    .join("workspacer")
                    .join("claude-profiles.json"),
            );
        }
    }
    directories::BaseDirs::new().map(|d| {
        d.home_dir()
            .join(".config")
            .join("workspacer")
            .join("claude-profiles.json")
    })
}

/// Every Claude config root the daemon should poll, whether or not a session
/// is running on it. Normalized keys, `""` first (the default account).
///
/// The poller used to iterate `live_claude_config_roots()`, which is empty at
/// boot — so a cold daemon fetched nothing and every rate-limit gauge stayed
/// blank until a session happened to start. The fetch itself has no session
/// dependency; the gate existed only to keep an idle daemon from hammering the
/// endpoint, and that intent is preserved by the CADENCE (see
/// [`next_poll_delay`]) rather than by having nothing to poll.
///
/// Three sources, unioned:
///   1. the default root, always — it is the account almost everything runs on;
///   2. `configDir` of every profile in the desktop's `claude-profiles.json`;
///   3. `<default>/accounts/*` on disk, so a daemon running without the desktop
///      (`workspacer serve`, a headless node) still finds second logins.
///
/// A named root is only included if it actually has a `.credentials.json` —
/// polling a root that cannot possibly answer just manufactures failures.
pub fn configured_config_roots() -> Vec<String> {
    let mut roots: Vec<String> = vec![String::new()];
    let mut push = |root: &str| {
        let key = normalize_root(root);
        if key.is_empty() {
            return; // already covered by the default entry
        }
        if !std::path::Path::new(&key).join(".credentials.json").is_file() {
            return;
        }
        if !roots.contains(&key) {
            roots.push(key);
        }
    };

    if let Some(path) = profiles_file() {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(doc) = serde_json::from_str::<Value>(&raw) {
                if let Some(list) = doc.get("profiles").and_then(Value::as_array) {
                    for p in list {
                        if let Some(dir) = p.get("configDir").and_then(Value::as_str) {
                            push(dir);
                        }
                    }
                }
            }
        }
    }

    // `<default>/accounts/*` — the layout workspacer's "Add Claude Account"
    // creates. Enumerated lexically; `push` filters to the ones with a login.
    let accounts = std::path::PathBuf::from(default_config_root()).join("accounts");
    if let Ok(rd) = std::fs::read_dir(&accounts) {
        let mut dirs: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            if let Some(dir) = d.to_str() {
                push(dir);
            }
        }
    }
    roots
}

/// How often a root is re-read while a Claude session is live on the account.
const LIVE_INTERVAL_SECS: u64 = 60;
/// …and while nothing is running. Still polled — a boot readout is exactly what
/// an idle daemon is asked for — but 15× less often, which is what keeps the
/// original "don't hammer the API from an idle daemon" intent intact.
const IDLE_INTERVAL_SECS: u64 = 15 * 60;
/// Ceiling on the failure backoff. An expired profile token (the live case on
/// this machine since 2026-08-20) is not going to fix itself from our side, so
/// a failing root settles at one attempt an hour rather than one a minute.
const MAX_BACKOFF_SECS: u64 = 60 * 60;
/// How often the scheduler wakes to see whether any root is due. Cheap: no
/// file is read and no request is made unless something is actually due.
const SCHEDULER_TICK_SECS: u64 = 30;

/// Delay until a root's next attempt, given whether it has live sessions and
/// how many times in a row it has just failed. Doubling from the base interval,
/// capped — so a permanently broken root costs one request an hour, and a
/// healthy idle root costs four an hour.
pub fn next_poll_delay(live: bool, consecutive_failures: u32) -> std::time::Duration {
    let base = if live {
        LIVE_INTERVAL_SECS
    } else {
        IDLE_INTERVAL_SECS
    };
    let secs = if consecutive_failures == 0 {
        base
    } else {
        let shift = consecutive_failures.min(16);
        base.saturating_mul(1u64 << shift).min(MAX_BACKOFF_SECS)
    };
    std::time::Duration::from_secs(secs.min(MAX_BACKOFF_SECS))
}

/// Background poll loop.
///
/// Iterates CONFIGURED roots ([`configured_config_roots`]) unioned with the
/// roots of any live session, so a cold daemon with zero sessions still has
/// real gauges within a tick of boot. Each root keeps its own schedule: live
/// accounts every [`LIVE_INTERVAL_SECS`], idle ones every
/// [`IDLE_INTERVAL_SECS`], and a failing one backs off toward
/// [`MAX_BACKOFF_SECS`] (see [`next_poll_delay`]).
///
/// Every outcome is recorded, success or failure — `set_account_usage` for a
/// reading, `set_account_usage_error` for a classified failure — so a consumer
/// can tell "0% used" from "we could not ask" from "this account needs to log
/// in again". Silence was the old behaviour and it is indistinguishable from
/// zero at the surface, which is the thing that lies to the user.
///
/// [`SessionStore::set_account_usage`]: super::store::SessionStore::set_account_usage
pub fn spawn_poller(store: super::store::SessionStore) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut tick =
            tokio::time::interval(std::time::Duration::from_secs(SCHEDULER_TICK_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // root → (next attempt, consecutive failures). A root absent from the
        // map has never been attempted and is due immediately, which is what
        // makes the FIRST tick after boot do the work.
        let mut schedule: std::collections::HashMap<String, (tokio::time::Instant, u32)> =
            std::collections::HashMap::new();
        loop {
            tick.tick().await;
            let now = tokio::time::Instant::now();
            let live = store.live_claude_config_roots();
            let mut roots = configured_config_roots();
            for r in &live {
                if !roots.contains(r) {
                    roots.push(r.clone());
                }
            }
            // Forget roots that are no longer configured or live, so a removed
            // profile stops occupying the schedule.
            schedule.retain(|k, _| roots.contains(k));

            for root in roots {
                let due = schedule
                    .get(&root)
                    .map(|(at, _)| *at <= now)
                    .unwrap_or(true);
                if !due {
                    continue;
                }
                let is_live = live.contains(&root);
                let failures = match fetch_account_usage(&client, &root).await {
                    Ok(usage) => {
                        store.set_account_usage(&root, usage);
                        0
                    }
                    Err(err) => {
                        // Soft-fail by design — no credentials, an expired
                        // token, or being offline must never take the daemon
                        // down — but no longer SILENT: the classified failure
                        // is stored so callers can say which of the three it is.
                        tracing::debug!(kind = ?err.kind, root, detail = %err.detail,
                                        "account usage poll failed");
                        let prev = schedule.get(&root).map(|(_, f)| *f).unwrap_or(0);
                        store.set_account_usage_error(&root, err);
                        prev.saturating_add(1)
                    }
                };
                let delay = next_poll_delay(is_live, failures);
                schedule.insert(root, (tokio::time::Instant::now() + delay, failures));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_live_capture_shape() {
        // Verbatim structure from a live 2026-07-15 capture (values trimmed).
        let v = json!({
            "five_hour": { "utilization": 19.5, "resets_at": null,
                           "limit_dollars": null },
            "seven_day": { "utilization": 3.0, "resets_at": 1784191200 },
            "extra_usage": { "is_enabled": false, "monthly_limit": 12400,
                             "used_credits": 0.0, "utilization": 0.0,
                             "disabled_reason": "out_of_credits" }
        });
        let u = parse_usage_response(&v);
        assert_eq!(u.five_hour_pct, Some(19.5));
        assert_eq!(u.five_hour_resets_at, None);
        assert_eq!(u.seven_day_pct, Some(3.0));
        assert_eq!(u.seven_day_resets_at, Some(1784191200));
        // Disabled extra usage must NOT produce a permanent 0% monthly meter.
        assert_eq!(u.monthly_pct, None);
        assert_eq!(u.out_of_credits, Some(true));
    }

    #[test]
    fn enabled_extra_usage_feeds_the_monthly_window() {
        let v = json!({
            "five_hour": { "utilization": 50.0, "resets_at": "2026-07-15T22:00:00Z" },
            "extra_usage": { "is_enabled": true, "utilization": 12.0 }
        });
        let u = parse_usage_response(&v);
        assert_eq!(u.five_hour_resets_at, Some(1784152800));
        assert_eq!(u.monthly_pct, Some(12.0));
        assert_eq!(u.out_of_credits, None);
    }

    #[test]
    fn readings_age_out() {
        let mut u = parse_usage_response(&json!({}));
        assert!(u.is_fresh(OffsetDateTime::now_utc()));
        u.fetched_at = OffsetDateTime::now_utc() - time::Duration::seconds(FRESH_FOR_SECS + 1);
        assert!(!u.is_fresh(OffsetDateTime::now_utc()));
    }

    /// Live smoke test against the real endpoint with the CLI's stored
    /// credentials — network + account state, so ignored by default:
    /// `cargo test -p claudemon live_fetch_smoke -- --ignored`
    #[tokio::test]
    #[ignore = "network + real Claude credentials"]
    async fn live_fetch_smoke() {
        let u = fetch_account_usage(&reqwest::Client::new(), "")
            .await
            .expect("fetch should succeed");
        assert!(
            u.five_hour_pct.is_some(),
            "endpoint always reports a 5h utilization: {u:?}"
        );
    }

    #[test]
    fn transcript_paths_reveal_their_config_root() {
        // Default-root shape (the root itself is NOT special-cased here).
        assert_eq!(
            root_from_transcript("/home/u/.claude/projects/-home-u-work/abc.jsonl").as_deref(),
            Some("/home/u/.claude"),
        );
        // A second-account root nested inside the primary: the LAST /projects/
        // wins, so a slug can't fake a root and a root containing "projects"
        // earlier in the path stays intact.
        assert_eq!(
            root_from_transcript("/home/u/projects/.claude/accounts/work/projects/p/abc.jsonl")
                .as_deref(),
            Some("/home/u/projects/.claude/accounts/work"),
        );
        // Windows separators, original backslashes preserved in the slice.
        assert_eq!(
            root_from_transcript("C:\\Users\\u\\.claude\\accounts\\work\\projects\\p\\a.jsonl")
                .as_deref(),
            Some("C:\\Users\\u\\.claude\\accounts\\work"),
        );
        assert_eq!(root_from_transcript("/tmp/nothing-here.jsonl"), None);
    }

    #[test]
    fn normalize_collapses_default_and_trims() {
        assert_eq!(normalize_root(""), "");
        assert_eq!(normalize_root(&default_config_root()), "");
        assert_eq!(
            normalize_root("/home/u/.claude/accounts/work/"),
            "/home/u/.claude/accounts/work",
        );
    }

    #[test]
    fn token_validation() {
        // Valid, unexpired.
        let ok = json!({ "claudeAiOauth": {
            "accessToken": "tok", "expiresAt": (OffsetDateTime::now_utc().unix_timestamp() + 3600) * 1000 } });
        assert_eq!(token_from_credentials(&ok).unwrap(), "tok");
        // Expired → Err, no request goes out.
        let expired = json!({ "claudeAiOauth": { "accessToken": "tok", "expiresAt": 1000 } });
        assert!(token_from_credentials(&expired).is_err());
        // Missing token → Err.
        assert!(token_from_credentials(&json!({})).is_err());
    }
}

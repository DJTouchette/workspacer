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
//! by every session on the account), so the daemon keeps one global copy in
//! [`SessionStore`] and patches it into each Claude session's status line —
//! see `SessionStore::apply_status_line` / `set_account_usage`.
//!
//! Cost: zero tokens. This is an account-metadata query, not an inference
//! call. Failure is always soft — no credentials, an expired token, or a
//! non-200 just means the gauges stay wire-fed.

use anyhow::{bail, Context, Result};
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

/// The CLI's OAuth access token. Read from `~/.claude/.credentials.json`
/// (Linux/Windows); on macOS the CLI keeps it in the login Keychain instead,
/// so fall back to `security find-generic-password`.
///
/// Never refreshes: rotation is the CLI's job (racing it with our own refresh
/// could invalidate the CLI's stored token). An expired token is an `Err` and
/// the poll simply retries next tick — the CLI refreshes it on its next turn.
fn read_access_token() -> Result<String> {
    match read_credentials_json() {
        Ok(creds) => token_from_credentials(&creds),
        Err(file_err) => {
            #[cfg(target_os = "macos")]
            {
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
            Err(file_err)
        }
    }
}

fn read_credentials_json() -> Result<Value> {
    let path = dirs_home()
        .context("no home directory")?
        .join(".claude")
        .join(".credentials.json");
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&raw).context("parsing credentials json")
}

fn dirs_home() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Extract + validate the token from the credentials document.
fn token_from_credentials(creds: &Value) -> Result<String> {
    let oauth = creds
        .get("claudeAiOauth")
        .context("no claudeAiOauth in credentials")?;
    // `expiresAt` is epoch milliseconds. A stale token would just 401; skip
    // the round-trip when we can tell locally.
    if let Some(expires_ms) = oauth.get("expiresAt").and_then(Value::as_i64) {
        if expires_ms <= OffsetDateTime::now_utc().unix_timestamp() * 1000 {
            bail!("oauth token expired (CLI will refresh it on its next turn)");
        }
    }
    oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .context("no accessToken in credentials")
}

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// One fetch: credentials → GET → parsed reading.
pub async fn fetch_account_usage(client: &reqwest::Client) -> Result<AccountUsage> {
    let token = read_access_token()?;
    let resp = client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("usage request failed")?;
    if !resp.status().is_success() {
        bail!("usage endpoint returned {}", resp.status());
    }
    let body: Value = resp.json().await.context("parsing usage response")?;
    Ok(parse_usage_response(&body))
}

/// How often the poller re-reads the endpoint while a Claude session is live.
const POLL_INTERVAL_SECS: u64 = 60;

/// Background poll loop. Ticks every [`POLL_INTERVAL_SECS`]; skips the fetch
/// entirely while no live Claude session exists, so an idle daemon touches
/// neither the credentials file nor the network. Each successful reading goes
/// through [`SessionStore::set_account_usage`], which patches and re-broadcasts
/// every live Claude session's status line.
///
/// [`SessionStore::set_account_usage`]: super::store::SessionStore::set_account_usage
pub fn spawn_poller(store: super::store::SessionStore) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(POLL_INTERVAL_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if !store.has_live_claude_session() {
                continue;
            }
            match fetch_account_usage(&client).await {
                Ok(usage) => store.set_account_usage(usage),
                // Soft-fail by design: no credentials (API-key or Bedrock
                // setups), expired token, offline — the gauges just stay
                // wire-fed until the next tick succeeds.
                Err(err) => tracing::debug!(?err, "account usage poll failed"),
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
        let u = fetch_account_usage(&reqwest::Client::new())
            .await
            .expect("fetch should succeed");
        assert!(
            u.five_hour_pct.is_some(),
            "endpoint always reports a 5h utilization: {u:?}"
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

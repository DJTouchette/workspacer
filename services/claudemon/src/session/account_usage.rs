//! Account-level usage poller — the rate-limit gauges for stream sessions.
//!
//! The PTY transport learns the 5h/7d utilization from Claude's interactive
//! statusLine JSON, but the headless stream transport has no statusLine: its
//! only wire source is `rate_limit_event`, which in practice carries `resetsAt`
//! *without* `utilization` (verified against live CLI captures — the percent
//! only appears on warning-threshold events). So a pure-stream fleet never
//! learns how much of a window it has used.
//!
//! This module fills the gap from the account side: it asks the user's own
//! `claude` CLI, over the stream-json control protocol, for the same reading
//! its `/usage` screen shows (a `get_usage` control request — see
//! [`fetch_account_usage`]). The daemon never opens the CLI's credential store
//! and never talks to Anthropic itself: the unmodified CLI holds the login,
//! refreshes it, and makes the request. The reading is *account*-scoped (the
//! windows are shared
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
//! Cost: zero tokens. The probe process is never sent a user message, so no
//! inference call is made. Failure is always soft — no plan login, a CLI too
//! old to answer, or being offline just means the gauges stay wire-fed.

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
    /// Weekly windows the plan meters per model, as the CLI lists them
    /// (`model_scoped`). Empty when the reply names none.
    pub model_windows: Vec<ModelWindow>,
    /// When the daemon fetched this. Readings age out — see [`Self::is_fresh`].
    pub fetched_at: OffsetDateTime,
}

/// One per-model weekly window: the server's own label for the model bucket
/// (e.g. `Fable`) and how much of its week is used.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelWindow {
    pub name: String,
    pub pct: f64,
    pub resets_at: Option<i64>,
}

/// How long a reading stays authoritative. Past this the poller has stopped or
/// is failing, and per-session wire data — even stale — is better than a dead
/// account snapshot.
///
/// DERIVED FROM THE REFRESH CADENCE ON PURPOSE, and it must never be shorter
/// than it. This was a flat five minutes while [`IDLE_INTERVAL_SECS`] was
/// fifteen, so an idle daemon's reading spent two thirds of every refresh cycle
/// being declared stale — and because the stream transport's `rate_limit_event`
/// carries `resetsAt` without `utilization`, every status line rebuilt inside
/// that gap went out with `five_hour_pct: None` and blanked the meter for five
/// to fifteen minutes after an agent started. The ceiling is one idle cadence
/// plus one live cadence of grace (16 minutes), which is the OUTER BOUND the
/// gate exists for: a reading from a poller that has died or been failing for
/// longer than a full refresh cycle plus a minute stops being asserted, while a
/// reading from a healthy poller is never asserted-then-retracted between two
/// of its own polls. Raising the ceiling rather than lowering the idle interval
/// is deliberate — five-hour utilization does not move meaningfully in a
/// quarter hour, and the alternative quadruples the API call rate.
const FRESH_FOR_SECS: i64 = (IDLE_INTERVAL_SECS + LIVE_INTERVAL_SECS) as i64;

impl AccountUsage {
    pub fn is_fresh(&self, now: OffsetDateTime) -> bool {
        (now - self.fetched_at).whole_seconds() < FRESH_FOR_SECS
    }
}

/// Parse the plan windows: the `rate_limits` object of the CLI's `get_usage`
/// reply (the CLI passes the usage body through under that key). Shape
/// (captured live, 2026-07-15):
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
    let amount = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|n| n.is_finite());
    let ratio = |used: Option<f64>, limit: Option<f64>| match (used, limit) {
        (Some(used), Some(limit)) if used >= 0.0 && limit > 0.0 => {
            Some((used / limit * 100.0).clamp(0.0, 100.0))
        }
        _ => None,
    };
    // The monthly spend budget, when the reply carries one: exact minor-unit
    // amounts, comparable only in one currency and scale.
    let spend = v.get("spend").filter(|s| s["enabled"] == true);
    let spend_pct = spend.and_then(|s| {
        let (used, limit) = (&s["used"], &s["limit"]);
        let currency = |m: &Value| m["currency"].as_str().unwrap_or_default().trim().to_owned();
        (!currency(used).is_empty()
            && currency(used) == currency(limit)
            && used["exponent"].is_u64()
            && used["exponent"] == limit["exponent"])
            .then(|| {
                ratio(
                    amount(used.get("amount_minor")),
                    amount(limit.get("amount_minor")),
                )
            })
            .flatten()
    });
    let (monthly_pct, monthly_resets_at) = if spend_pct.is_some() || extra_enabled {
        // Else spent credits over the monthly limit, else the reply's own
        // rounded `utilization`.
        let field = |key: &str| amount(extra.and_then(|e| e.get(key)));
        let (reported, resets_at) = window("extra_usage");
        let pct = spend_pct
            .or_else(|| ratio(field("used_credits"), field("monthly_limit")))
            .or(reported.filter(|p| extra_enabled && *p >= 0.0));
        (pct, resets_at)
    } else {
        (None, None)
    };
    let model_windows = v
        .get("model_scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m["display_name"].as_str().map(str::trim)?;
            let pct = m["utilization"].as_f64().filter(|p| p.is_finite())?;
            (!name.is_empty()).then(|| ModelWindow {
                name: name.to_owned(),
                pct: pct.clamp(0.0, 100.0),
                resets_at: m.get("resets_at").and_then(parse_resets_at),
            })
        })
        .collect();
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
        model_windows,
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
///
/// LOSSY ON PURPOSE-RESOLVED PATHS. Workspacer's "Add Claude Account" profiles
/// share memories and transcripts by symlinking `<root>/projects` at the
/// primary `~/.claude/projects`, so both logins write into ONE physical
/// directory. This function reads the string it is given, which is the only
/// reason it can tell them apart: a canonicalized path names the primary
/// account no matter which login wrote it, and it does so silently — there is
/// no error to notice and no way to detect it after the fact.
///
/// So this is the FALLBACK source, for sessions the daemon did not spawn.
/// [`root_from_spawn_env`] is the primary one, and it cannot be defeated this
/// way because it never looks at a path. Never make this the sole source
/// again, and never `canonicalize`/`realpath` a transcript path before handing
/// it here.
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

/// The config root a SPAWN env selects: its `CLAUDE_CONFIG_DIR` if it set one,
/// otherwise the daemon's own default. Normalized, so `""` means "the default
/// account" whichever way the env spelled it.
///
/// This is the robust half of account attribution. The env var is a
/// *declaration*, not a path we later have to re-derive, so no amount of path
/// resolution downstream can change what it says. Contrast
/// [`root_from_transcript`], which is only correct while the path it is handed
/// is un-resolved — see its own note.
pub fn root_from_spawn_env(env: &std::collections::HashMap<String, String>) -> String {
    match env.get("CLAUDE_CONFIG_DIR").map(|d| d.trim()) {
        Some(dir) if !dir.is_empty() => normalize_root(dir),
        _ => String::new(),
    }
}

/// What a transcript path can honestly say about which account wrote it.
///
/// Three values, because the question has three answers and a `String` has
/// room for only two of them. [`Self::Ambiguous`] is the one that did not
/// exist before, and it is the whole point of this type — see
/// [`attribute_transcript`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RootAttribution {
    /// Exactly one configured account can have written this path.
    Certain { account: String },
    /// More than one can. The path resolves into a `projects` directory that
    /// several logins share, so which one wrote the file is not recoverable
    /// from the path — the candidates are listed rather than one being picked.
    Ambiguous {
        candidates: Vec<String>,
        reason: String,
    },
    /// The path is not a Claude transcript path at all (no `projects`
    /// component), so it names no account.
    Unknown,
}

impl RootAttribution {
    /// The account when there is exactly one, else `None`. Callers that must
    /// have a single key (a poll target, a cache key) use this and treat
    /// `None` as "the default account"; callers that REPORT attribution must
    /// match on the enum instead, or they re-introduce the merge.
    pub fn certain(&self) -> Option<&str> {
        match self {
            Self::Certain { account } => Some(account),
            _ => None,
        }
    }
}

/// The `projects` directory a normalized root key points at. `""` is the
/// daemon's default root.
fn root_projects_dir(key: &str) -> std::path::PathBuf {
    let root = if key.is_empty() {
        default_config_root()
    } else {
        key.to_string()
    };
    std::path::PathBuf::from(root).join("projects")
}

/// Whether two `projects` directories are the SAME physical directory.
///
/// This is the one place canonicalization belongs. Resolving a *transcript*
/// path destroys attribution ([`root_from_transcript`]); resolving the *roots*
/// is how we discover that the destruction is possible, which is the opposite
/// operation. A path we cannot resolve (a root that does not exist on this
/// machine) is not shared with anything, so a failed canonicalize is `false`.
fn same_physical_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Attribute a transcript path to an account, HONESTLY.
///
/// [`root_from_transcript`] answers this question by reading the path string,
/// which is correct exactly while nothing has resolved it. Workspacer's second
/// Claude accounts symlink `<root>/projects` at the primary's, so one
/// `realpath` anywhere upstream rewrites a profile's transcript path into one
/// that names the PRIMARY root — and the old answer was a confident, wrong
/// `String` with no way to tell it from a right one.
///
/// This does not try to undo that (it cannot: the information is gone from the
/// path). It makes the loss VISIBLE, which is the difference between a bug and
/// a reading:
///
///   - the derived root's `projects` dir is shared with no other configured
///     root → [`RootAttribution::Certain`]. This is every machine with a
///     single login, so nothing gets noisier for the common case.
///   - it IS shared, but the derived root's own `projects` is a symlink → still
///     `Certain`. A canonicalized path can never spell a root whose `projects`
///     is a symlink (canonicalizing is what replaces that component), so a path
///     that does spell it proves the writer used that config dir.
///   - it is shared and the derived root's `projects` is the real directory →
///     [`RootAttribution::Ambiguous`], listing every root that shares it. This
///     is precisely the canonicalized-profile-transcript case, and it is also
///     an honest description of a genuine primary-account transcript: from the
///     path alone the two are indistinguishable once the symlink exists.
///
/// `roots` is the configured account list ([`configured_config_roots`]),
/// normalized. Taken as a parameter so the behaviour can be tested against a
/// fixture tree rather than the machine's real `~/.claude`.
pub fn attribute_transcript(path: &str, roots: &[String]) -> RootAttribution {
    let Some(derived) = root_from_transcript(path) else {
        return RootAttribution::Unknown;
    };
    let key = normalize_root(&derived);
    let projects = root_projects_dir(&key);

    let mut candidates: Vec<String> = roots
        .iter()
        .map(|r| normalize_root(r))
        .filter(|r| *r != key && same_physical_dir(&root_projects_dir(r), &projects))
        .collect();
    if candidates.is_empty() {
        return RootAttribution::Certain { account: key };
    }
    // Spelling a root whose own `projects` is a symlink is proof the path was
    // never resolved, so it still names its writer.
    if std::fs::symlink_metadata(&projects)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return RootAttribution::Certain { account: key };
    }
    candidates.push(key);
    candidates.sort();
    candidates.dedup();
    RootAttribution::Ambiguous {
        reason: format!(
            "{} is shared by {} logged-in Claude accounts (a profile symlinks \
             its projects dir at this one), so a transcript path that resolves \
             here names all of them equally",
            projects.display(),
            candidates.len(),
        ),
        candidates,
    }
}

/// Why an account's usage could not be read. The distinction is load-bearing:
/// a UI that renders any of these as `0%` tells the user they have a full
/// window when the truth is that we do not know, and `NeedsReauth` is
/// specifically actionable in a way the others are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageFailure {
    /// The CLI said this account's login was rejected or has expired and it
    /// could not renew it. Fixed by signing in again with that account's CLI.
    NeedsReauth,
    /// The CLI reports no plan rate limits for this config root: an API-key /
    /// Bedrock / Vertex setup, or a root that was never signed in.
    NoCredentials,
    /// No reading could be obtained: the CLI is missing, too old to answer
    /// `get_usage`, did not answer in time, or could not reach the usage
    /// service (offline, 5xx).
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
    /// Build a classified failure without a failing fetch, so the shapes that
    /// consume one () can be tested against every kind rather
    /// than only against whichever the machine happens to produce.
    #[cfg(test)]
    pub fn for_test(kind: UsageFailure, detail: impl std::fmt::Display) -> Self {
        Self::new(kind, detail)
    }

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

fn dirs_home() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Whole-probe ceiling: CLI start, the usage request it makes, and the reply.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// How long a probe that has answered gets to exit after its stdin closes.
const PROBE_EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
/// The control `request_id` of the probe's one request.
const PROBE_REQUEST_ID: &str = "wks-usage-1";

/// An enterprise `managed-mcp.json` already excludes every other MCP config,
/// and the CLI refuses `--strict-mcp-config` beside it.
fn managed_mcp_config_present() -> bool {
    let path = if cfg!(target_os = "macos") {
        "/Library/Application Support/ClaudeCode/managed-mcp.json"
    } else if cfg!(windows) {
        "C:\\Program Files\\ClaudeCode\\managed-mcp.json"
    } else {
        "/etc/claude-code/managed-mcp.json"
    };
    std::path::Path::new(path).exists()
}

/// The probe's argv after the binary: the headless stream-json contract with
/// everything a usage read does not need turned off — no hooks, no tools, no
/// slash commands, no MCP servers, no transcript.
fn probe_args(session_id: &str, strict_mcp: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--session-id",
        session_id,
        "--settings",
        r#"{"disableAllHooks":true}"#,
        "--tools",
        "",
        "--disable-slash-commands",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if strict_mcp {
        args.extend(
            [
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
            ]
            .map(str::to_owned),
        );
    }
    args
}

/// Run one `claude` process for `root`, send it a single `get_usage` control
/// request and return the reply's payload (`SDKControlGetUsageResponse` in the
/// Agent SDK's typings; the SDK's own usage call sends this same request).
///
/// No user message is ever written, so the CLI makes no model call. `root`
/// selects the login the way every spawn does: `CLAUDE_CONFIG_DIR`, or the
/// daemon's own environment for the default root (`""`).
async fn probe_cli_usage(
    bin: &str,
    root: &str,
    session_id: &str,
) -> std::result::Result<Value, UsageError> {
    use crate::background_process::BackgroundCommand;
    use crate::child_env::SanitizeChildEnvironment;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let unreachable = |detail: String| UsageError::new(UsageFailure::Unreachable, detail);
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(probe_args(session_id, !managed_mcp_config_present()))
        // A neutral directory: no project settings, `.mcp.json` or CLAUDE.md.
        .current_dir(std::env::temp_dir())
        // claude.ai connectors are discovered outside the MCP config files.
        .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
        .env("CLAUDE_CODE_AUTO_CONNECT_IDE", "0")
        .env("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .scrub_host_authority()
        .no_console_window();
    if !root.is_empty() {
        cmd.env("CLAUDE_CONFIG_DIR", root);
    }
    let mut child = cmd
        .spawn()
        .map_err(|err| unreachable(format!("could not start `{bin}`: {err}")))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");

    let exchange = async {
        let request = serde_json::json!({
            "type": "control_request",
            "request_id": PROBE_REQUEST_ID,
            // `skip_behaviors`: only the plan windows are wanted, not the scan
            // of local transcripts behind the `/usage` screen's breakdown.
            "request": { "subtype": "get_usage", "skip_behaviors": true },
        });
        stdin
            .write_all(format!("{request}\n").as_bytes())
            .await
            .map_err(|err| unreachable(format!("writing to `{bin}`: {err}")))?;
        stdin
            .flush()
            .await
            .map_err(|err| unreachable(format!("writing to `{bin}`: {err}")))?;
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        loop {
            let line = lines
                .next_line()
                .await
                .map_err(|err| unreachable(format!("reading from `{bin}`: {err}")))?
                .ok_or_else(|| {
                    unreachable(format!("`{bin}` exited without answering get_usage"))
                })?;
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if frame["type"] == "control_response"
                && frame["response"]["request_id"] == PROBE_REQUEST_ID
            {
                return control_reply(&frame["response"]);
            }
        }
    };
    let outcome = match tokio::time::timeout(PROBE_TIMEOUT, exchange).await {
        Ok(outcome) => outcome,
        Err(_) => Err(unreachable(format!(
            "`{bin}` did not answer get_usage within {}s",
            PROBE_TIMEOUT.as_secs()
        ))),
    };
    // Closed stdin is the CLI's end of input; a probe that lingers is killed.
    drop(stdin);
    if tokio::time::timeout(PROBE_EXIT_GRACE, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    outcome
}

/// The payload of a `control_response` body, or its error classified.
fn control_reply(response: &Value) -> std::result::Result<Value, UsageError> {
    if response["subtype"] == "success" {
        return Ok(response["response"].clone());
    }
    let error = response["error"].as_str().unwrap_or("unknown error");
    let lower = error.to_ascii_lowercase();
    if lower.contains("unsupported control request") {
        return Err(UsageError::new(
            UsageFailure::Unreachable,
            "this Claude Code version cannot report plan usage (no get_usage); update Claude Code",
        ));
    }
    let kind = if ["401", "403", "unauthorized", "log in", "login", "expired"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        UsageFailure::NeedsReauth
    } else {
        UsageFailure::Unreachable
    };
    Err(UsageError::new(kind, format!("get_usage failed: {error}")))
}

/// Turn a `get_usage` payload into a reading. `rate_limits_available: false`
/// is the CLI saying plan limits do not apply to this login; a null
/// `rate_limits` beside `true` is the CLI failing to read them.
pub fn usage_from_reply(reply: &Value) -> std::result::Result<AccountUsage, UsageError> {
    if reply["rate_limits_available"] != true {
        return Err(UsageError::new(
            UsageFailure::NoCredentials,
            "no plan rate limits for this login (API key, cloud provider, or not signed in)",
        ));
    }
    match reply.get("rate_limits") {
        Some(limits) if limits.is_object() => Ok(parse_usage_response(limits)),
        _ => Err(UsageError::new(
            UsageFailure::Unreachable,
            "Claude Code could not read this account's plan usage",
        )),
    }
}

/// One reading for one config root, asked of the user's own `claude` CLI.
///
/// The probe is a real headless `claude`, so its session id is marked like a
/// keep-warm ping's: should a hook fire despite `disableAllHooks` (managed
/// policy hooks), [`SessionStore::ingest`] drops it and no ghost row appears.
///
/// [`SessionStore::ingest`]: super::store::SessionStore::ingest
pub async fn fetch_account_usage(
    store: &super::store::SessionStore,
    root: &str,
) -> std::result::Result<AccountUsage, UsageError> {
    let bin = crate::daemon::spawn::resolve_provider_bin("claude");
    read_account_usage(store, &bin, root).await
}

async fn read_account_usage(
    store: &super::store::SessionStore,
    bin: &str,
    root: &str,
) -> std::result::Result<AccountUsage, UsageError> {
    let session_id = uuid::Uuid::new_v4().to_string();
    store.mark_heartbeat(&session_id);
    let reply = probe_cli_usage(bin, root, &session_id).await;
    store.unmark_heartbeat(&session_id);
    usage_from_reply(&reply?)
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
        if !std::path::Path::new(&key)
            .join(".credentials.json")
            .is_file()
        {
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

/// The environment variable carrying the desktop's / launcher's
/// `usage.pollOnBoot` setting. `0` (or `false`/`off`/`no`) turns idle-root
/// discovery off; anything else, INCLUDING ABSENCE, leaves it on.
pub const POLL_ON_BOOT_ENV: &str = "WORKSPACER_USAGE_POLL_ON_BOOT";

/// Parse [`POLL_ON_BOOT_ENV`]. Split from the env read so the default is
/// testable without touching process state.
///
/// Absence is ON, and that is the compatibility contract in both directions: an
/// older desktop that has never heard of the key sets nothing and must keep the
/// shipped behaviour, and a spawn site that fails to compute the value must
/// over-poll rather than silently blank every gauge. Only an explicit falsy
/// spelling turns it off.
pub fn poll_on_boot_from_env(raw: Option<&str>) -> bool {
    match raw {
        None => true,
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
    }
}

/// [`poll_on_boot_from_env`] against this process's environment.
pub fn poll_on_boot_enabled() -> bool {
    poll_on_boot_from_env(std::env::var(POLL_ON_BOOT_ENV).ok().as_deref())
}

/// The roots ONE scheduler pass should poll.
///
/// Live roots are always included — the toggle is about an IDLE daemon, and a
/// running session's gauges must never regress because of it. `include_idle`
/// (the `usage.pollOnBoot` setting) decides whether the configured accounts
/// that have nothing running join them; with it off this is exactly the
/// pre-46fd339f behaviour, an empty list on a cold daemon and therefore no
/// requests at all.
pub fn roots_to_poll(live: &[String], include_idle: bool) -> Vec<String> {
    let mut roots = if include_idle {
        configured_config_roots()
    } else {
        Vec::new()
    };
    for r in live {
        if !roots.contains(r) {
            roots.push(r.clone());
        }
    }
    roots
}

/// How often a root is re-read while a Claude session is live on the account.
const LIVE_INTERVAL_SECS: u64 = 60;
/// …and while nothing is running. Still polled — a boot readout is exactly what
/// an idle daemon is asked for — but 15× less often, which is what keeps the
/// original "don't hammer the API from an idle daemon" intent intact.
pub(crate) const IDLE_INTERVAL_SECS: u64 = 15 * 60;
/// Ceiling on the failure backoff. A signed-out profile is not going to fix
/// itself from our side, so a failing root settles at one attempt an hour rather than one a minute.
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

/// One root's booked next attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Booking {
    /// When the root may next be polled.
    pub next: std::time::Instant,
    /// Consecutive failures behind the current backoff.
    pub failures: u32,
    /// Whether `next` was computed on the LIVE cadence. Kept so an idle→live
    /// transition can be recognised — see [`due_now`].
    pub live: bool,
}

/// Whether `root` may be polled now, re-arming it when it has just gone from
/// idle to live.
///
/// The booked instant is written only AFTER a poll runs, from the liveness at
/// that instant — so a root polled while idle is booked [`IDLE_INTERVAL_SECS`]
/// out, and a session starting a minute later used to wait out the rest of that
/// appointment. That wait is exactly the window in which a just-started agent
/// shows a blank usage meter, because the account reading it needs is the one
/// the poll would refresh. So a root whose booking was made on the idle cadence
/// and which now has a live session becomes due immediately: the first status
/// line after the session starts is patched from a reading taken within one
/// scheduler tick rather than up to a quarter of an hour later.
///
/// Only idle→live pulls an appointment forward. Live→idle leaves the booking
/// alone (it is already sooner than an idle one would be), and a root that is
/// already live is untouched, so this cannot turn into a poll every tick.
pub fn due_now(
    schedule: &mut std::collections::HashMap<String, Booking>,
    root: &str,
    is_live: bool,
    now: std::time::Instant,
) -> bool {
    match schedule.get_mut(root) {
        // Never attempted: due immediately, which is what makes the FIRST tick
        // after boot do the work.
        None => true,
        Some(b) => {
            if is_live && !b.live {
                b.next = now;
                b.live = true;
            }
            b.next <= now
        }
    }
}

/// A refresh that finished this recently answers the next one too.
const REFRESH_COALESCE: std::time::Duration = std::time::Duration::from_secs(10);

/// Re-read every Claude account NOW: the configured roots plus any with a live
/// session, probed side by side, each outcome recorded like a poll's. This is
/// what a person pressing Refresh asks for; the poller's cadence and failure
/// backoff do not apply, so an account that was signed in again shows up at
/// once instead of at its next hourly attempt.
///
/// Callers are serialized, and one that arrives within [`REFRESH_COALESCE`] of
/// the last finished refresh gets that result: repeated presses cannot stack
/// CLI processes.
pub async fn refresh_all(store: &super::store::SessionStore) {
    static LAST: std::sync::OnceLock<tokio::sync::Mutex<Option<std::time::Instant>>> =
        std::sync::OnceLock::new();
    let mut last = LAST.get_or_init(Default::default).lock().await;
    if last.is_some_and(|at| at.elapsed() < REFRESH_COALESCE) {
        return;
    }
    let roots = roots_to_poll(&store.live_claude_config_roots(), true);
    let bin = crate::daemon::spawn::resolve_provider_bin("claude");
    refresh_roots(store, &bin, &roots).await;
    *last = Some(std::time::Instant::now());
}

/// Probe `roots` side by side with `bin` and record every outcome.
async fn refresh_roots(store: &super::store::SessionStore, bin: &str, roots: &[String]) {
    let readings = futures::future::join_all(
        roots
            .iter()
            .map(|root| async move { (root, read_account_usage(store, bin, root).await) }),
    )
    .await;
    for (root, reading) in readings {
        let label = super::usage_report::label_for_root(root);
        match reading {
            Ok(usage) => {
                tracing::info!(root = %label, outcome = "ok", "account usage refresh");
                store.set_account_usage(root, usage);
            }
            Err(err) => {
                tracing::info!(
                    root = %label, outcome = "failed", kind = ?err.kind, detail = %err.detail,
                    "account usage refresh",
                );
                store.set_account_usage_error(root, err);
            }
        }
    }
}

/// Background poll loop.
///
/// Iterates CONFIGURED roots ([`configured_config_roots`]) unioned with the
/// roots of any live session, so a cold daemon with zero sessions still has
/// real gauges within a tick of boot. `include_idle_roots` is the operator's
/// `usage.pollOnBoot` setting (see [`roots_to_poll`]): with it off, only live
/// roots are polled and an idle daemon makes no requests. Each root keeps its
/// own schedule: live
/// accounts every [`LIVE_INTERVAL_SECS`], idle ones every
/// [`IDLE_INTERVAL_SECS`], and a failing one backs off toward
/// [`MAX_BACKOFF_SECS`] (see [`next_poll_delay`]). A root that goes idle→live
/// has its booking pulled forward to now ([`due_now`]).
///
/// Every poll outcome is logged at INFO — root label, success or failure kind,
/// and the instant observed — so poll history is reconstructible from the
/// daemon log. It is four probes an hour per idle root, so this is not
/// chatty, and its absence was what made "the reading is two hours old" an
/// unanswerable question.
///
/// Every outcome is recorded, success or failure — `set_account_usage` for a
/// reading, `set_account_usage_error` for a classified failure — so a consumer
/// can tell "0% used" from "we could not ask" from "this account needs to log
/// in again". Silence was the old behaviour and it is indistinguishable from
/// zero at the surface, which is the thing that lies to the user.
///
/// [`SessionStore::set_account_usage`]: super::store::SessionStore::set_account_usage
pub fn spawn_poller(store: super::store::SessionStore, include_idle_roots: bool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(SCHEDULER_TICK_SECS));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // root → booking. A root absent from the map has never been attempted
        // and is due immediately (see [`due_now`]).
        let mut schedule: std::collections::HashMap<String, Booking> =
            std::collections::HashMap::new();
        loop {
            tick.tick().await;
            let now = std::time::Instant::now();
            let live = store.live_claude_config_roots();
            let roots = roots_to_poll(&live, include_idle_roots);
            // Forget roots that are no longer configured or live, so a removed
            // profile stops occupying the schedule.
            schedule.retain(|k, _| roots.contains(k));

            for root in roots {
                let is_live = live.contains(&root);
                if !due_now(&mut schedule, &root, is_live, now) {
                    continue;
                }
                let label = super::usage_report::label_for_root(&root);
                let failures = match fetch_account_usage(&store, &root).await {
                    Ok(usage) => {
                        tracing::info!(
                            root = %label,
                            outcome = "ok",
                            live = is_live,
                            observed_at = usage.fetched_at.unix_timestamp(),
                            five_hour_pct = ?usage.five_hour_pct,
                            seven_day_pct = ?usage.seven_day_pct,
                            "account usage poll",
                        );
                        store.set_account_usage(&root, usage);
                        0
                    }
                    Err(err) => {
                        // Soft-fail by design — no plan login, an expired
                        // login, or being offline must never take the daemon
                        // down — but no longer SILENT: the classified failure
                        // is stored so callers can say which of the three it is.
                        tracing::info!(
                            root = %label,
                            outcome = "failed",
                            live = is_live,
                            kind = ?err.kind,
                            detail = %err.detail,
                            observed_at = OffsetDateTime::now_utc().unix_timestamp(),
                            "account usage poll",
                        );
                        let prev = schedule.get(&root).map(|b| b.failures).unwrap_or(0);
                        store.set_account_usage_error(&root, err);
                        prev.saturating_add(1)
                    }
                };
                let delay = next_poll_delay(is_live, failures);
                schedule.insert(
                    root,
                    Booking {
                        next: std::time::Instant::now() + delay,
                        failures,
                        live: is_live,
                    },
                );
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

    /// Live smoke test against the installed `claude` and its own login —
    /// network + account state, so ignored by default:
    /// `cargo test -p claudemon live_fetch_smoke -- --ignored`
    #[tokio::test]
    #[ignore = "network + a real signed-in Claude Code"]
    async fn live_fetch_smoke() {
        let u = fetch_account_usage(&crate::session::SessionStore::new(), "")
            .await
            .expect("fetch should succeed");
        assert!(
            u.five_hour_pct.is_some(),
            "a plan login always reports a 5h utilization: {u:?}"
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

    /// A fixture of the real hazard: `<base>/.claude/projects` is a real
    /// directory and `<base>/.claude/accounts/work/projects` is a SYMLINK at
    /// it, which is exactly what workspacer's "Add Claude Account" builds so
    /// the two logins share memories and transcripts.
    #[cfg(unix)]
    fn shared_projects_fixture(tag: &str) -> (std::path::PathBuf, String, String) {
        let base = crate::testtmp::dir().join(format!(
            "attr-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = std::fs::remove_dir_all(&base);
        let primary = base.join(".claude");
        let work = primary.join("accounts").join("work");
        std::fs::create_dir_all(primary.join("projects").join("-home-u-p")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::os::unix::fs::symlink(primary.join("projects"), work.join("projects")).unwrap();
        std::fs::write(
            work.join("projects").join("-home-u-p").join("abc.jsonl"),
            "",
        )
        .unwrap();
        (
            base,
            primary.to_string_lossy().into_owned(),
            work.to_string_lossy().into_owned(),
        )
    }

    /// THE REGRESSION GUARD for the symlink merge, on the fallback source.
    ///
    /// `root_from_transcript` handed a canonicalized profile transcript returns
    /// the PRIMARY root — a confident `String` indistinguishable from a right
    /// answer. This asserts the new attribution refuses to make that claim, so
    /// it FAILS against the old behaviour: the old code had exactly one answer
    /// for this input and it was `Certain(primary)`.
    #[cfg(unix)]
    #[test]
    fn a_canonicalized_profile_transcript_is_ambiguous_not_the_primary_account() {
        let (base, primary, work) = shared_projects_fixture("canon");
        let roots = vec![primary.clone(), work.clone()];
        let spelled = format!("{work}/projects/-home-u-p/abc.jsonl");
        let resolved = std::fs::canonicalize(&spelled)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_ne!(spelled, resolved, "the fixture must traverse the symlink");

        // What the OLD, string-only source says about the resolved path: the
        // primary. This is the silent merge, reproduced.
        assert_eq!(
            normalize_root(&root_from_transcript(&resolved).unwrap()),
            primary,
            "the old derivation still says primary — that is the hazard",
        );

        // What attribution says now. Not primary, not a guess: the two logins
        // that could equally have written it, and why.
        match attribute_transcript(&resolved, &roots) {
            RootAttribution::Ambiguous { candidates, reason } => {
                assert_eq!(candidates, {
                    let mut v = vec![primary.clone(), work.clone()];
                    v.sort();
                    v
                });
                assert!(reason.contains("shared"), "{reason}");
            }
            other => panic!(
                "a resolved profile transcript must not be attributed to one \
                 account; got {other:?}",
            ),
        }

        // And the un-resolved spelling is STILL certain — the guard must not
        // buy honesty by making the working case useless.
        assert_eq!(
            attribute_transcript(&spelled, &roots).certain(),
            Some(work.as_str()),
            "a path that spells the profile root was never resolved, so it \
             still names its writer",
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The common machine — one login, no profile — must not become ambiguous.
    #[cfg(unix)]
    #[test]
    fn a_single_account_machine_stays_certain() {
        let base = crate::testtmp::dir().join(format!("attr-solo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let primary = base.join(".claude");
        std::fs::create_dir_all(primary.join("projects").join("-p")).unwrap();
        let root = primary.to_string_lossy().into_owned();
        assert_eq!(
            attribute_transcript(
                &format!("{root}/projects/-p/a.jsonl"),
                std::slice::from_ref(&root),
            )
            .certain(),
            Some(root.as_str()),
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A path with no `projects` component names no account, and that is its
    /// own value — not the default account.
    #[test]
    fn a_non_transcript_path_attributes_to_unknown_not_to_the_default() {
        assert_eq!(
            attribute_transcript("/tmp/nothing-here.jsonl", &[String::new()]),
            RootAttribution::Unknown,
        );
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

    /// The `get_usage` payload as the Agent SDK types it
    /// (`SDKControlGetUsageResponse`): ISO reset times, credits beside the
    /// rounded utilization.
    #[test]
    fn reads_the_cli_usage_reply() {
        let u = usage_from_reply(&json!({
            "subscription_type": "max",
            "rate_limits_available": true,
            "rate_limits": {
                "five_hour": { "utilization": 42.0, "resets_at": "2026-11-10T20:00:00Z" },
                "seven_day": { "utilization": 7.5, "resets_at": null },
                "seven_day_opus": null,
                "extra_usage": { "is_enabled": true, "monthly_limit": 2000,
                                 "used_credits": 500.0, "utilization": 26.0 }
            },
            "behaviors": null
        }))
        .unwrap();
        assert_eq!(u.five_hour_pct, Some(42.0));
        assert_eq!(u.five_hour_resets_at, Some(1_794_340_800));
        assert_eq!(u.seven_day_pct, Some(7.5));
        assert_eq!(u.seven_day_resets_at, None);
        assert_eq!(
            u.monthly_pct,
            Some(25.0),
            "credits over limit, not the rounded figure"
        );
    }

    /// The spend budget (exact minor units) outranks the credit ratio, shows a
    /// monthly window even with `extra_usage` off, and is ignored when its two
    /// amounts are not comparable. Per-model weekly windows are listed as the
    /// server names them; one with no reading is left out.
    #[test]
    fn reads_the_spend_budget_and_per_model_windows() {
        let money = |amount: u64, currency: &str| json!({ "amount_minor": amount, "currency": currency, "exponent": 2 });
        let u = parse_usage_response(&json!({
            "extra_usage": { "is_enabled": false, "monthly_limit": 2000,
                             "used_credits": 500.0, "utilization": 26.0 },
            "spend": { "enabled": true, "used": money(1500, "USD"), "limit": money(5000, "USD") },
            "model_scoped": [
                { "display_name": "Fable", "utilization": 61.5,
                  "resets_at": "2026-11-10T20:00:00Z" },
                { "display_name": "Opus", "utilization": null, "resets_at": null }
            ]
        }));
        assert_eq!(u.monthly_pct, Some(30.0));
        assert_eq!(
            u.model_windows,
            vec![ModelWindow {
                name: "Fable".into(),
                pct: 61.5,
                resets_at: Some(1_794_340_800),
            }]
        );

        let mismatched = parse_usage_response(&json!({
            "extra_usage": { "is_enabled": true, "monthly_limit": 2000,
                             "used_credits": 500.0, "utilization": 26.0 },
            "spend": { "enabled": true, "used": money(1500, "USD"), "limit": money(5000, "EUR") }
        }));
        assert_eq!(mismatched.monthly_pct, Some(25.0), "falls back to credits");
        assert!(mismatched.model_windows.is_empty());

        let disabled = parse_usage_response(&json!({
            "spend": { "enabled": false, "used": money(1, "USD"), "limit": money(2, "USD") }
        }));
        assert_eq!(disabled.monthly_pct, None);
    }

    /// The failure kinds are distinguished where they are created. "This login
    /// has no plan limits" must never collapse into the same bucket as "we
    /// could not ask", and neither may read as 0%.
    #[test]
    fn failures_are_classified_not_merged() {
        // Captured live from a signed-out CLI 2.1.322.
        let no_plan = json!({ "subscription_type": null, "rate_limits_available": false,
                              "rate_limits": null, "behaviors": null });
        assert_eq!(
            usage_from_reply(&no_plan).unwrap_err().kind,
            UsageFailure::NoCredentials,
        );
        let unread = json!({ "rate_limits_available": true, "rate_limits": null });
        assert_eq!(
            usage_from_reply(&unread).unwrap_err().kind,
            UsageFailure::Unreachable,
        );
        let reply = |error: &str| {
            control_reply(&json!({ "subtype": "error", "request_id": "r", "error": error }))
                .unwrap_err()
        };
        // Captured live: an unknown subtype on CLI 2.1.322.
        let old = reply("Unsupported control request subtype: get_usage");
        assert_eq!(old.kind, UsageFailure::Unreachable);
        assert!(old.detail.contains("update Claude Code"), "{}", old.detail);
        assert_eq!(
            reply("OAuth token has expired").kind,
            UsageFailure::NeedsReauth
        );
        assert_eq!(reply("fetch failed").kind, UsageFailure::Unreachable);
    }

    #[test]
    fn the_probe_turns_off_everything_but_the_question() {
        let args = probe_args("sid-1", true).join(" ");
        assert!(args.starts_with("--print --input-format stream-json --output-format stream-json"));
        assert!(args.contains("--session-id sid-1"));
        assert!(args.contains("--no-session-persistence"));
        assert!(args.contains(r#"--settings {"disableAllHooks":true}"#));
        assert!(args.contains(r#"--strict-mcp-config --mcp-config {"mcpServers":{}}"#));
        // A managed MCP policy forbids the strict flag.
        assert!(!probe_args("sid-1", false).join(" ").contains("mcp"));
    }

    /// A throwaway directory, removed on drop.
    #[cfg(unix)]
    struct Scratch(std::path::PathBuf);
    #[cfg(unix)]
    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir()
                .join(format!("claudemon-usage-probe-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    #[cfg(unix)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A stand-in `claude` that records its argv, config root and stdin, and
    /// answers the first line it reads with `answer`.
    #[cfg(unix)]
    fn stub_cli(dir: &std::path::Path, answer: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("claude");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{d}/argv.txt'\n\
             printf '%s' \"$CLAUDE_CONFIG_DIR\" > '{d}/root.txt'\n\
             echo '{{\"type\":\"system\",\"subtype\":\"noise\"}}'\n\
             IFS= read -r line\nprintf '%s\\n' \"$line\" > '{d}/stdin.jsonl'\n{answer}\n",
            d = dir.display(),
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// End to end over a real pipe: one `get_usage` control request goes in,
    /// no user message does, the root travels as `CLAUDE_CONFIG_DIR`, and the
    /// reply's payload comes back.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_probe_asks_the_cli_and_never_sends_a_prompt() {
        let dir = Scratch::new();
        let bin = stub_cli(
            dir.path(),
            r#"echo '{"type":"control_response","response":{"subtype":"success","request_id":"wks-usage-1","response":{"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":12,"resets_at":null}}}}}'"#,
        );
        let reply = probe_cli_usage(&bin, "/profiles/work", "sid-9")
            .await
            .unwrap();
        assert_eq!(usage_from_reply(&reply).unwrap().five_hour_pct, Some(12.0));

        let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();
        assert_eq!(read("root.txt"), "/profiles/work");
        assert!(read("argv.txt").contains("sid-9"));
        let sent: Vec<Value> = read("stdin.jsonl")
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            sent,
            vec![
                json!({ "type": "control_request", "request_id": "wks-usage-1",
                         "request": { "subtype": "get_usage", "skip_behaviors": true } })
            ]
        );
    }

    /// A refresh records what each account's CLI said, good or bad, and a good
    /// reading retires that account's earlier failure.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_refresh_records_every_account_at_once() {
        let dir = Scratch::new();
        let bin = stub_cli(
            dir.path(),
            r#"echo '{"type":"control_response","response":{"subtype":"success","request_id":"wks-usage-1","response":{"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":33,"resets_at":null}}}}}'"#,
        );
        let store = crate::session::SessionStore::new();
        store.set_account_usage_error(
            "/profiles/work",
            UsageError::for_test(UsageFailure::NeedsReauth, "signed out"),
        );
        refresh_roots(&store, &bin, &[String::new(), "/profiles/work".into()]).await;
        for root in ["", "/profiles/work"] {
            assert_eq!(
                store.account_usage_for(root).and_then(|u| u.five_hour_pct),
                Some(33.0),
                "{root:?}"
            );
            assert!(store.account_usage_error_for(root).is_none(), "{root:?}");
        }

        let missing = dir.path().join("gone").to_string_lossy().into_owned();
        refresh_roots(&store, &missing, &[String::new()]).await;
        assert!(store.account_usage_error_for("").is_some());
    }

    /// A CLI that dies without answering (an unknown flag on an old build) or
    /// is not installed at all is a soft, classified failure.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_probe_that_cannot_answer_fails_soft() {
        let dir = Scratch::new();
        let bin = stub_cli(dir.path(), "exit 1");
        let err = probe_cli_usage(&bin, "", "sid").await.unwrap_err();
        assert_eq!(err.kind, UsageFailure::Unreachable);
        assert!(
            err.detail.contains("exited without answering"),
            "{}",
            err.detail
        );

        let missing = dir.path().join("no-such-claude");
        let err = probe_cli_usage(&missing.to_string_lossy(), "", "sid")
            .await
            .unwrap_err();
        assert_eq!(err.kind, UsageFailure::Unreachable);
        assert!(err.detail.contains("could not start"), "{}", err.detail);
    }

    /// The cadence that replaces the old live-sessions gate. An idle daemon
    /// still polls — a boot readout is exactly what it is asked for — just far
    /// less often, and a root that keeps failing settles at one attempt an
    /// hour rather than retrying every minute forever.
    #[test]
    fn poll_cadence_backs_off_and_is_capped() {
        assert_eq!(
            next_poll_delay(true, 0).as_secs(),
            LIVE_INTERVAL_SECS,
            "a live account is polled at the original interval",
        );
        assert_eq!(
            next_poll_delay(false, 0).as_secs(),
            IDLE_INTERVAL_SECS,
            "an idle daemon polls, but 15x less often",
        );
        assert_eq!(next_poll_delay(true, 1).as_secs(), LIVE_INTERVAL_SECS * 2);
        assert_eq!(next_poll_delay(true, 2).as_secs(), LIVE_INTERVAL_SECS * 4);
        // The expired-profile case: failing forever must not mean requesting
        // forever. Both live and idle settle at the ceiling.
        assert_eq!(next_poll_delay(true, 30).as_secs(), MAX_BACKOFF_SECS);
        assert_eq!(next_poll_delay(false, 30).as_secs(), MAX_BACKOFF_SECS);
        // …and no arithmetic overflow on the way there.
        assert!(next_poll_delay(false, u32::MAX).as_secs() <= MAX_BACKOFF_SECS);
    }

    /// THE defect, in one line: a reading may not be declared stale sooner than
    /// the schedule can possibly refresh it. `FRESH_FOR_SECS` was 300 while
    /// `IDLE_INTERVAL_SECS` was 900, so an idle root's reading was dead for two
    /// thirds of every refresh cycle and every status line rebuilt in that gap
    /// went out with no percentage. Nothing else ties the two constants
    /// together, so this test is the tie.
    #[test]
    fn a_reading_never_goes_stale_before_it_can_be_refreshed() {
        assert!(
            FRESH_FOR_SECS >= IDLE_INTERVAL_SECS as i64,
            "a reading is asserted for {FRESH_FOR_SECS}s but refreshed at worst \
             every {IDLE_INTERVAL_SECS}s — the difference is a window in which \
             every rebuilt status line blanks the meter",
        );
        assert!(
            FRESH_FOR_SECS >= LIVE_INTERVAL_SECS as i64,
            "…and the same must hold on the live cadence",
        );
        // But it is still a CEILING: a poller that has died must eventually
        // stop having its last reading asserted, so freshness may not be
        // unbounded or wider than a full refresh cycle plus a live one.
        assert!(
            FRESH_FOR_SECS <= (IDLE_INTERVAL_SECS + LIVE_INTERVAL_SECS) as i64,
            "a dead poller's reading has to age out within one refresh cycle \
             plus a live cadence of grace",
        );
    }

    /// A root polled while idle is booked a quarter of an hour out. When a
    /// session starts on it, the appointment must come forward to now — not be
    /// waited out, which is what made "start an agent, wait up to 15 minutes
    /// for your usage" the shipped behaviour.
    #[test]
    fn an_idle_root_that_goes_live_is_due_within_one_tick() {
        let now = std::time::Instant::now();
        let mut schedule = std::collections::HashMap::new();

        // Never seen: due immediately, which is what makes boot work.
        assert!(due_now(&mut schedule, "", false, now));

        // Polled while idle → booked IDLE_INTERVAL_SECS out, and not due.
        schedule.insert(
            String::new(),
            Booking {
                next: now + next_poll_delay(false, 0),
                failures: 0,
                live: false,
            },
        );
        let a_minute_later = now + std::time::Duration::from_secs(60);
        assert!(
            !due_now(&mut schedule, "", false, a_minute_later),
            "an idle root keeps its idle cadence",
        );

        // A session starts. The next scheduler tick must find it due.
        assert!(
            due_now(&mut schedule, "", true, a_minute_later),
            "an idle→live transition pulls the appointment forward to now",
        );
        assert!(schedule[""].live, "the booking is re-armed as live");

        // …and only once: still live on the following tick, it waits out the
        // live cadence like anything else rather than polling every tick.
        schedule.insert(
            String::new(),
            Booking {
                next: a_minute_later + next_poll_delay(true, 0),
                failures: 0,
                live: true,
            },
        );
        assert!(
            !due_now(
                &mut schedule,
                "",
                true,
                a_minute_later + std::time::Duration::from_secs(SCHEDULER_TICK_SECS),
            ),
            "a root that was already live must not become due every tick",
        );
    }

    /// Root discovery, against a synthetic HOME laid out the way workspacer's
    /// "Add Claude Account" does it. The point of the test is that the list is
    /// non-empty and complete WITHOUT any session existing — that is the whole
    /// deliverable.
    #[test]
    fn configured_roots_are_found_with_no_sessions_at_all() {
        let mut home = std::env::temp_dir();
        home.push(format!(
            "wks-roots-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let claude = home.join(".claude");
        let work = claude.join("accounts").join("work");
        let logged_out = claude.join("accounts").join("never-logged-in");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&logged_out).unwrap();
        std::fs::write(work.join(".credentials.json"), "{}").unwrap();

        // `default_config_root` reads CLAUDE_CONFIG_DIR, which is also what
        // scopes the accounts scan — so pointing it at the fixture is enough.
        let restore = std::env::var("CLAUDE_CONFIG_DIR").ok();
        std::env::set_var("CLAUDE_CONFIG_DIR", &claude);
        let roots = configured_config_roots();
        match restore {
            Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
            None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
        }

        assert_eq!(roots[0], "", "the default account is always polled, first",);
        assert!(
            roots.contains(&work.to_string_lossy().into_owned()),
            "a second login on disk is discovered without the desktop's help: {roots:?}",
        );
        assert!(
            !roots
                .iter()
                .any(|r| r == &logged_out.to_string_lossy().into_owned()),
            "a profile dir with no credentials is skipped rather than \
             manufacturing a failure every tick: {roots:?}",
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    // ── usage.pollOnBoot ────────────────────────────────────────────────────
    //
    // The toggle gates ONE thing: whether an idle daemon discovers the accounts
    // that have no session running. Everything else about the poller — the
    // cadence, the backoff, the booking — is untouched by it, so these tests
    // pin the union and the default and nothing else.

    #[test]
    fn the_toggle_suppresses_idle_roots_but_never_a_live_one() {
        let live = vec!["/home/u/.claude/accounts/work".to_string()];

        // Off: the pre-46fd339f behaviour. The default root is NOT added — that
        // union is the whole of what the setting buys — and the live root is
        // still there, so a running session's gauges cannot regress.
        let off = roots_to_poll(&live, false);
        assert_eq!(
            off, live,
            "with pollOnBoot off, only roots with a live session are polled",
        );
        assert!(
            !off.iter().any(|r| r.is_empty()),
            "the default root is an IDLE-discovery entry; off must not add it: {off:?}",
        );

        // On: the configured roots (always at least the default one, first)
        // plus the live root.
        let on = roots_to_poll(&live, true);
        assert_eq!(
            on[0], "",
            "with pollOnBoot on, the default account leads the pass: {on:?}",
        );
        assert!(
            on.contains(&live[0]),
            "a live root is polled under either setting: {on:?}",
        );
        assert!(
            on.len() > off.len(),
            "on must poll strictly more than off, or the flag does nothing: {on:?} vs {off:?}",
        );
    }

    #[test]
    fn an_idle_daemon_with_the_toggle_off_polls_nothing_at_all() {
        // The cold-boot shape: zero sessions. This is the request-count claim
        // the setting makes — not "fewer requests", none.
        assert!(
            roots_to_poll(&[], false).is_empty(),
            "an idle daemon with pollOnBoot off must make no account requests",
        );
        assert!(
            !roots_to_poll(&[], true).is_empty(),
            "an idle daemon with pollOnBoot on must still find the default account",
        );
    }

    #[test]
    fn a_live_root_is_not_polled_twice_when_it_is_also_configured() {
        // The default root is in configured_config_roots() unconditionally, so
        // a live session on the DEFAULT account is the collision case.
        let roots = roots_to_poll(&[String::new()], true);
        assert_eq!(
            roots.iter().filter(|r| r.is_empty()).count(),
            1,
            "the default root must appear once, not once per source: {roots:?}",
        );
    }

    #[test]
    fn an_absent_env_var_means_on() {
        // Compatibility in both directions: an older desktop sets nothing, and
        // a spawn site that could not compute the value sets nothing. Neither
        // may turn the feature off.
        assert!(poll_on_boot_from_env(None), "absent must default to ON");
        for on in ["1", "true", "yes", "on", "", "  ", "anything"] {
            assert!(
                poll_on_boot_from_env(Some(on)),
                "{on:?} is not one of the falsy spellings and must read as ON",
            );
        }
        for off in ["0", "false", "off", "no", "FALSE", " 0 "] {
            assert!(
                !poll_on_boot_from_env(Some(off)),
                "{off:?} must read as OFF"
            );
        }
    }

    #[test]
    fn a_cold_boot_with_no_env_var_set_reads_as_on() {
        // The same claim through the real environment read the daemon boot
        // path calls, so a wrong variable NAME cannot pass the parser test.
        let restore = std::env::var(POLL_ON_BOOT_ENV).ok();
        std::env::remove_var(POLL_ON_BOOT_ENV);
        let cold = poll_on_boot_enabled();
        std::env::set_var(POLL_ON_BOOT_ENV, "0");
        let off = poll_on_boot_enabled();
        std::env::set_var(POLL_ON_BOOT_ENV, "1");
        let on = poll_on_boot_enabled();
        match restore {
            Some(v) => std::env::set_var(POLL_ON_BOOT_ENV, v),
            None => std::env::remove_var(POLL_ON_BOOT_ENV),
        }
        assert!(
            cold,
            "a daemon spawned with no {POLL_ON_BOOT_ENV} polls on boot"
        );
        assert!(!off, "{POLL_ON_BOOT_ENV}=0 turns idle-root discovery off");
        assert!(on, "{POLL_ON_BOOT_ENV}=1 turns it back on");
    }
}

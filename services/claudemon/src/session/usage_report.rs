//! The usage wire shape: every provider, every account, with no live session.
//!
//! Everything the other modules in this layer learned has to leave the daemon
//! through one document, or each client re-invents the joins and re-invents the
//! lies. This is that document.
//!
//! # Three values, everywhere a number could be missing
//!
//! The bug this whole layer exists to stop is a blank rendering as `0`. A
//! `Option<f64>` cannot stop it, because it has room for only two of the three
//! answers a usage question actually has:
//!
//!   - **ZERO** — we asked, and the answer is nothing. `{"state":"ok",
//!     "value":0}`. A brand-new account really has spent $0.00.
//!   - **UNKNOWN** — we could not find out *right now*. An expired token, an
//!     unreadable database, a poller that has not run yet. Carries a reason and
//!     invites a retry. `{"state":"unknown","reason":"…"}`.
//!   - **UNAVAILABLE** — nobody can find out, and no retry will help. Copilot's
//!     quota headroom is the type case: the only endpoint that would answer
//!     refuses our only credential. `{"state":"unavailable","reason":"…"}`.
//!
//! [`Measured`] is that tag, and it is used for every scalar in the report,
//! including ones that are always known today — because "always known" is a
//! property of this month's provider, not of the shape.
//!
//! # Accounts, and the fourth value
//!
//! `account` is `null` when the daemon does not know which login a session
//! billed against — a row older than schema v8, a session it did not spawn, or
//! a transcript path whose account was destroyed by canonicalization (see
//! [`super::account_usage::attribute_transcript`]). `null` is UNKNOWN and must
//! never be folded into the default account, which is spelled `""`. Those two
//! are different claims and the difference is the whole of commit f3ef4fcb.
//!
//! # Where each provider's numbers come from
//!
//! | provider | windows | spend | needs a live session |
//! |----------|---------|-------|----------------------|
//! | claude   | OAuth `/api/oauth/usage`, polled per config root | estimated from transcripts | no |
//! | codex    | last `rate_limits` in the newest rollout on disk | not metered — UNAVAILABLE | no |
//! | copilot  | UNAVAILABLE — no local record, endpoint 403s | GitHub's own recorded charge | no |
//!
//! Not one of them needs a session running, which is the requirement: a desktop
//! that has just launched can render the whole thing.

use serde::Serialize;

use super::account_usage::{self, RootAttribution};
use super::state::SessionState;
use super::store::SessionStore;
use crate::providers::{codex_usage, copilot_usage};

/// A scalar that can be genuinely zero, temporarily unknown, or structurally
/// unavailable. See the module note — the three are not interchangeable and
/// collapsing any pair of them is the failure mode this layer was built to fix.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Measured<T> {
    /// A real reading. `value` may be `0`, and that is an answer.
    Ok { value: T },
    /// Not knowable right now. A retry may succeed.
    Unknown { reason: String },
    /// Not knowable at all, by anyone, from here. A retry will not succeed.
    Unavailable { reason: String },
}

impl<T> Measured<T> {
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self::Unknown {
            reason: reason.into(),
        }
    }
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }
    /// A reading, or `Unknown` with `reason` when there isn't one. The common
    /// shape at every boundary where the source hands us an `Option`.
    pub fn from_option(v: Option<T>, reason: impl Into<String>) -> Self {
        match v {
            Some(value) => Self::Ok { value },
            None => Self::unknown(reason),
        }
    }
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ok { value } => Some(value),
            _ => None,
        }
    }
}

/// One rate-limit window.
#[derive(Debug, Clone, Serialize)]
pub struct WindowReport {
    pub used_percent: Measured<f64>,
    pub resets_at: Option<i64>,
    pub window_minutes: Option<u64>,
    /// Whether the reading still describes the window that is open now.
    /// `Some(false)` means the window rolled over after the number was taken:
    /// the percentage is real history and a false present. `None` when the
    /// source reported no reset time, so it cannot be decided either way.
    pub is_current: Option<bool>,
}

impl WindowReport {
    fn missing(m: Measured<f64>) -> Self {
        Self {
            used_percent: m,
            resets_at: None,
            window_minutes: None,
            is_current: None,
        }
    }
}

/// The three windows every provider is asked about, so a client can lay out one
/// row of gauges without knowing which provider fills which.
#[derive(Debug, Clone, Serialize)]
pub struct WindowsReport {
    pub five_hour: WindowReport,
    pub seven_day: WindowReport,
    /// The monthly overage/credit window. Anthropic only, and only while extra
    /// usage is enabled — otherwise UNAVAILABLE, never a permanent 0% meter.
    pub monthly: WindowReport,
}

/// Token counts. Every field is [`Measured`] because providers disagree about
/// which of them they record at all: Copilot has the full split, Codex has only
/// a cumulative total, Claude's comes from transcript folding.
#[derive(Debug, Clone, Serialize)]
pub struct TokensReport {
    /// Fresh (uncached) input, where the source separates it. Copilot's
    /// `input_tokens` column CONFLATES all three prompt kinds, so this is read
    /// from its per-request price table instead — see [`copilot_usage`].
    pub input: Measured<u64>,
    pub cache_read: Measured<u64>,
    pub cache_write: Measured<u64>,
    pub output: Measured<u64>,
    pub reasoning: Measured<u64>,
    pub total: Measured<u64>,
}

impl TokensReport {
    fn all(reason: &str) -> Self {
        let r = || Measured::unavailable(reason);
        Self {
            input: r(),
            cache_read: r(),
            cache_write: r(),
            output: r(),
            reasoning: r(),
            total: r(),
        }
    }
}

/// How a cost figure was arrived at. A client that shows money should be able
/// to say whether the vendor said it or we worked it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    /// The vendor's own recorded charge for the request. Authoritative.
    VendorRecorded,
    /// Our price table applied to token counts we folded ourselves. Close, and
    /// known to run LOW where cache writes are involved — see the copilot
    /// module's measured comparison.
    Estimated,
    /// No cost exists to report (a flat-rate plan), or none could be computed.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpendReport {
    pub cost_usd: Measured<f64>,
    /// GitHub's own billing unit, exact and integral. Copilot only; the dollar
    /// figure beside it is derived from it at a rate the copilot module
    /// justifies. UNAVAILABLE for providers that do not use AIU.
    pub nano_aiu: Measured<u64>,
    pub basis: CostBasis,
}

/// One model's share of an account's usage.
#[derive(Debug, Clone, Serialize)]
pub struct ModelReport {
    pub model: String,
    pub requests: Measured<u64>,
    pub tokens: TokensReport,
    pub spend: SpendReport,
}

/// A classified failure, carried alongside whatever reading survived it — a
/// stale-but-real number plus "and the last refresh failed because the token
/// expired" is strictly more information than either alone.
#[derive(Debug, Clone, Serialize)]
pub struct FailureReport {
    pub kind: account_usage::UsageFailure,
    pub detail: String,
    pub at: i64,
}

/// Where an account's numbers came from, so a UI can caption them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSource {
    /// Anthropic's OAuth usage endpoint, polled by the daemon.
    OauthPoll,
    /// A file the provider's own CLI wrote. No network, no live session.
    Disk,
    /// Folded from transcripts the daemon can read.
    Transcript,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountReport {
    /// The account key. `""` is the default login — a real answer. `null` is
    /// UNKNOWN: the daemon cannot say which account this is, and a consumer
    /// must render it as unknown rather than adding it to the default.
    pub account: Option<String>,
    /// Short human name. `"default"`, a profile directory's basename, or
    /// `"unattributed"`.
    pub label: String,
    pub is_default: bool,
    pub source: ReportSource,
    /// When the underlying reading was taken (epoch seconds), if known.
    pub observed_at: Option<i64>,
    /// Whether that reading is recent enough to be treated as current. `None`
    /// when the source has no freshness notion (a disk fold is as fresh as the
    /// file).
    pub fresh: Option<bool>,
    pub failure: Option<FailureReport>,
    pub windows: WindowsReport,
    pub spend: SpendReport,
    pub tokens: TokensReport,
    pub models: Vec<ModelReport>,
    /// Live sessions the daemon currently attributes to this account.
    pub live_sessions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderReport {
    pub provider: String,
    pub accounts: Vec<AccountReport>,
    /// Why there are no accounts, when there are none. An empty list plus a
    /// reason is a reading; an empty list alone is indistinguishable from a
    /// provider nobody uses.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageReport {
    pub generated_at: i64,
    pub providers: Vec<ProviderReport>,
}

/// The whole report. Reads the store, the configured Claude roots, and both
/// on-disk provider records; makes no network request and needs no live
/// session, so a just-booted daemon answers it fully.
pub fn build(store: &SessionStore) -> UsageReport {
    let now = time::OffsetDateTime::now_utc();
    UsageReport {
        generated_at: now.unix_timestamp(),
        providers: vec![
            claude_report(store, now),
            codex_report(store),
            copilot_report(store),
        ],
    }
}

// ── Claude ───────────────────────────────────────────────────────────

/// Per-account Claude spend, folded from the transcripts of the sessions the
/// daemon knows about, bucketed by [`SessionState::claude_account_attribution`].
///
/// Sessions whose account is `Ambiguous` or `Unknown` go to the `None` bucket,
/// NOT to the default account. That is the point of the attribution work: a
/// canonicalized profile transcript used to land silently in the primary's
/// total, inflating one account and emptying another with no error anywhere.
fn claude_spend_by_account(
    store: &SessionStore,
    roots: &[String],
) -> std::collections::BTreeMap<Option<String>, ClaudeBucket> {
    let mut out: std::collections::BTreeMap<Option<String>, ClaudeBucket> = Default::default();
    for s in store.list().into_iter().filter(|s| s.provider == "claude") {
        let bucket = out.entry(account_bucket_key(&s, roots)).or_default();
        bucket.add(&s);
    }
    out
}

/// Which account row a session's spend belongs in — the one decision that
/// turns attribution into money.
///
/// `None` for BOTH `Ambiguous` and `Unknown`, and that is the whole point.
/// Folding either into the default account is the silent merge: it inflates
/// one login's spend and empties another's, with nothing anywhere saying so.
/// Pulled out of the fold so it can be tested without a store.
fn account_bucket_key(s: &SessionState, roots: &[String]) -> Option<String> {
    match s.claude_account_attribution(roots) {
        RootAttribution::Certain { account } => Some(account),
        RootAttribution::Ambiguous { .. } | RootAttribution::Unknown => None,
    }
}

#[derive(Debug, Default)]
struct ClaudeBucket {
    cost_usd: f64,
    context_tokens: u64,
    cache: super::usage::CacheSplit,
    /// model → (requests, cost, cache split)
    models: std::collections::BTreeMap<String, (u64, f64, super::usage::CacheSplit)>,
    live_sessions: u64,
}

impl ClaudeBucket {
    fn add(&mut self, s: &SessionState) {
        let u = super::usage::usage_for_session(s);
        self.cost_usd += u.cost_usd;
        self.context_tokens += u.context_tokens;
        let cache = u.cache.unwrap_or_default();
        self.cache.fresh += cache.fresh;
        self.cache.write += cache.write;
        self.cache.read += cache.read;
        if s.mode != super::state::SessionMode::Stopped {
            self.live_sessions += 1;
        }
        let model = u.model.unwrap_or_else(|| "unknown".to_string());
        let e = self.models.entry(model).or_default();
        e.0 += 1;
        e.1 += u.cost_usd;
        e.2.fresh += cache.fresh;
        e.2.write += cache.write;
        e.2.read += cache.read;
    }
}

fn label_for_root(root: &str) -> String {
    if root.is_empty() {
        return "default".to_string();
    }
    std::path::Path::new(root)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string())
}

fn claude_report(store: &SessionStore, now: time::OffsetDateTime) -> ProviderReport {
    // Configured roots, unioned with any root the store has an opinion about,
    // so an account that only ever produced a FAILURE still gets a row saying
    // so. A missing row and a failing row look identical to a user otherwise.
    let mut roots = account_usage::configured_config_roots();
    for r in store.known_account_roots() {
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    let mut spend = claude_spend_by_account(store, &roots);

    let mut accounts: Vec<AccountReport> = roots
        .iter()
        .map(|root| {
            let bucket = spend.remove(&Some(root.clone())).unwrap_or_default();
            let reading = store.account_usage_for(root);
            let failure = store
                .account_usage_error_for(root)
                .map(|f| FailureReport {
                    kind: f.kind,
                    detail: f.detail,
                    at: f.at.unix_timestamp(),
                });
            // Why a window is missing, phrased for whichever reason applies.
            // A classified failure is strictly better than "no reading yet",
            // and NeedsReauth in particular is actionable.
            let why: String = match (&failure, &reading) {
                (Some(f), _) => format!("{:?}: {}", f.kind, f.detail),
                (None, None) => "the account-usage poller has not produced a \
                                 reading for this account yet"
                    .to_string(),
                (None, Some(_)) => "the usage endpoint did not report this window".to_string(),
            };
            let window = |pct: Option<f64>, resets: Option<i64>| WindowReport {
                used_percent: Measured::from_option(pct, why.clone()),
                resets_at: resets,
                window_minutes: None,
                is_current: resets.map(|r| r > now.unix_timestamp()),
            };
            let windows = match &reading {
                Some(u) => WindowsReport {
                    five_hour: window(u.five_hour_pct, u.five_hour_resets_at),
                    seven_day: window(u.seven_day_pct, u.seven_day_resets_at),
                    monthly: match u.monthly_pct {
                        Some(_) => window(u.monthly_pct, u.monthly_resets_at),
                        // Not a gap in our reading: the account has extra usage
                        // switched off, so there is no monthly window to be at
                        // 0% of. Rendering one would pin a permanent empty
                        // meter on every session.
                        None => WindowReport::missing(Measured::unavailable(
                            "extra usage (monthly overage) is not enabled on this account",
                        )),
                    },
                },
                None => WindowsReport {
                    five_hour: WindowReport::missing(Measured::unknown(why.clone())),
                    seven_day: WindowReport::missing(Measured::unknown(why.clone())),
                    monthly: WindowReport::missing(Measured::unknown(why.clone())),
                },
            };
            AccountReport {
                account: Some(root.clone()),
                label: label_for_root(root),
                is_default: root.is_empty(),
                source: ReportSource::OauthPoll,
                observed_at: reading.as_ref().map(|u| u.fetched_at.unix_timestamp()),
                fresh: reading.as_ref().map(|u| u.is_fresh(now)),
                failure,
                windows,
                spend: SpendReport {
                    cost_usd: Measured::Ok {
                        value: bucket.cost_usd,
                    },
                    nano_aiu: Measured::unavailable("Anthropic does not bill in AIU"),
                    basis: CostBasis::Estimated,
                },
                tokens: claude_tokens(&bucket),
                models: claude_models(&bucket),
                live_sessions: bucket.live_sessions,
            }
        })
        .collect();

    // Whatever is left is genuinely unattributed. It gets its OWN row with a
    // null account rather than being added to the default — see the module note.
    if let Some(bucket) = spend.remove(&None) {
        accounts.push(unattributed_claude_account(bucket));
    }
    // Any leftover keys are roots that sessions name but that are not
    // configured (a profile whose credentials were removed). Same treatment as
    // any other account rather than being dropped on the floor.
    let leftovers: Vec<(Option<String>, ClaudeBucket)> = spend.into_iter().collect();
    for (key, bucket) in leftovers {
        let root = key.unwrap_or_default();
        accounts.push(AccountReport {
            account: Some(root.clone()),
            label: label_for_root(&root),
            is_default: root.is_empty(),
            source: ReportSource::Transcript,
            observed_at: None,
            fresh: None,
            failure: None,
            windows: WindowsReport {
                five_hour: WindowReport::missing(Measured::unknown(
                    "this account is not configured on this machine, so its \
                     windows are not polled",
                )),
                seven_day: WindowReport::missing(Measured::unknown(
                    "this account is not configured on this machine, so its \
                     windows are not polled",
                )),
                monthly: WindowReport::missing(Measured::unknown(
                    "this account is not configured on this machine, so its \
                     windows are not polled",
                )),
            },
            spend: SpendReport {
                cost_usd: Measured::Ok {
                    value: bucket.cost_usd,
                },
                nano_aiu: Measured::unavailable("Anthropic does not bill in AIU"),
                basis: CostBasis::Estimated,
            },
            tokens: claude_tokens(&bucket),
            models: claude_models(&bucket),
            live_sessions: bucket.live_sessions,
        });
    }

    ProviderReport {
        provider: "claude".to_string(),
        accounts,
        note: None,
    }
}

fn unattributed_claude_account(bucket: ClaudeBucket) -> AccountReport {
    const WHY: &str = "these sessions carry no spawn-recorded account, and their \
                       transcript paths cannot name one — a row older than schema \
                       v8, a session this daemon did not spawn, or a path whose \
                       account was erased by canonicalization";
    AccountReport {
        account: None,
        label: "unattributed".to_string(),
        is_default: false,
        source: ReportSource::Transcript,
        observed_at: None,
        fresh: None,
        failure: None,
        windows: WindowsReport {
            five_hour: WindowReport::missing(Measured::unknown(WHY)),
            seven_day: WindowReport::missing(Measured::unknown(WHY)),
            monthly: WindowReport::missing(Measured::unknown(WHY)),
        },
        spend: SpendReport {
            cost_usd: Measured::Ok {
                value: bucket.cost_usd,
            },
            nano_aiu: Measured::unavailable("Anthropic does not bill in AIU"),
            basis: CostBasis::Estimated,
        },
        tokens: claude_tokens(&bucket),
        models: claude_models(&bucket),
        live_sessions: bucket.live_sessions,
    }
}

fn claude_tokens(bucket: &ClaudeBucket) -> TokensReport {
    const NO_OUTPUT: &str = "the transcript fold tracks the prompt side per turn; \
                             a cumulative output total is not kept";
    TokensReport {
        input: Measured::Ok {
            value: bucket.cache.fresh,
        },
        cache_read: Measured::Ok {
            value: bucket.cache.read,
        },
        cache_write: Measured::Ok {
            value: bucket.cache.write,
        },
        output: Measured::unavailable(NO_OUTPUT),
        reasoning: Measured::unavailable(NO_OUTPUT),
        total: Measured::Ok {
            value: bucket.cache.fresh + bucket.cache.read + bucket.cache.write,
        },
    }
}

fn claude_models(bucket: &ClaudeBucket) -> Vec<ModelReport> {
    bucket
        .models
        .iter()
        .map(|(model, (requests, cost, cache))| ModelReport {
            model: model.clone(),
            requests: Measured::Ok { value: *requests },
            tokens: TokensReport {
                input: Measured::Ok { value: cache.fresh },
                cache_read: Measured::Ok { value: cache.read },
                cache_write: Measured::Ok { value: cache.write },
                output: Measured::unavailable("not folded per model"),
                reasoning: Measured::unavailable("not folded per model"),
                total: Measured::Ok {
                    value: cache.fresh + cache.read + cache.write,
                },
            },
            spend: SpendReport {
                cost_usd: Measured::Ok { value: *cost },
                nano_aiu: Measured::unavailable("Anthropic does not bill in AIU"),
                basis: CostBasis::Estimated,
            },
        })
        .collect()
}

// ── Codex ────────────────────────────────────────────────────────────

fn codex_report(store: &SessionStore) -> ProviderReport {
    const NOT_METERED: &str = "Codex records no per-request cost on disk; its plan \
                               is a rate limit, not a meter";
    let live = live_sessions_for(store, "codex");
    match codex_usage::read_from_disk() {
        Err(err) => ProviderReport {
            provider: "codex".to_string(),
            accounts: vec![],
            note: Some(format!("{err}")),
        },
        Ok(u) => {
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let window = |w: &Option<codex_usage::CodexWindow>| match w {
                Some(w) => WindowReport {
                    used_percent: Measured::from_option(
                        w.used_percent,
                        "the rollout's rate_limits block carried no percentage \
                         for this window",
                    ),
                    resets_at: w.resets_at,
                    window_minutes: w.window_minutes,
                    is_current: w.is_current(now),
                },
                None => WindowReport::missing(Measured::unknown(
                    "the most recent rollout carrying rate limits did not \
                     include this window",
                )),
            };
            let account = AccountReport {
                // Codex has one login per CODEX_HOME and records no account
                // identity in the rollout, so the home directory IS the key.
                account: Some(u.source.parent().map(codex_home_of).unwrap_or_default()),
                label: u.plan_type.clone().unwrap_or_else(|| "codex".to_string()),
                is_default: true,
                source: ReportSource::Disk,
                observed_at: u.observed_at,
                // A disk reading has no freshness of its own; whether it is
                // CURRENT is the window's `is_current`, which is the question
                // that actually matters and is answered per window.
                fresh: None,
                failure: None,
                windows: WindowsReport {
                    five_hour: window(&u.five_hour),
                    seven_day: window(&u.seven_day),
                    monthly: WindowReport::missing(Measured::unavailable(
                        "Codex publishes no monthly window",
                    )),
                },
                spend: SpendReport {
                    cost_usd: Measured::unavailable(NOT_METERED),
                    nano_aiu: Measured::unavailable("Codex does not bill in AIU"),
                    basis: CostBasis::None,
                },
                tokens: TokensReport {
                    total: Measured::from_option(
                        u.all_thread_tokens,
                        "Codex's state_5.sqlite could not be read (it is held \
                         open in WAL mode), so the cumulative total is unknown \
                         — it is NOT zero",
                    ),
                    ..TokensReport::all(
                        "state_5.sqlite records one cumulative token count per \
                         thread, with no input/output split",
                    )
                },
                models: vec![],
                live_sessions: live,
            };
            ProviderReport {
                provider: "codex".to_string(),
                accounts: vec![account],
                note: u
                    .has_credits
                    .map(|has| format!("credits: {}", if has { "available" } else { "none" })),
            }
        }
    }
}

/// `$CODEX_HOME` recovered from a rollout path. Rollouts live at
/// `<home>/sessions/YYYY/MM/DD/rollout-*.jsonl`, so from the containing
/// directory the home is FOUR ancestors up — `DD`, `MM`, `YYYY`, `sessions` —
/// and `ancestors()` counts itself as the zeroth, which is the off-by-one this
/// is spelled out to avoid. Verified against a live report: `nth(3)` returned
/// `~/.codex/sessions`.
fn codex_home_of(dir: &std::path::Path) -> String {
    dir.ancestors()
        .nth(4)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ── Copilot ──────────────────────────────────────────────────────────

fn copilot_report(store: &SessionStore) -> ProviderReport {
    let live = live_sessions_for(store, "copilot");
    let u = match copilot_usage::read_from_disk() {
        Ok(u) => u,
        Err(err) => {
            return ProviderReport {
                provider: "copilot".to_string(),
                accounts: vec![],
                note: Some(format!("{err}")),
            }
        }
    };
    // All three windows are the SAME structural refusal, and it is a refusal
    // rather than a gap: no local record exists and the endpoint that would
    // answer 403s our credential. Zero here would say "your plan is spent".
    let unavailable = || WindowReport::missing(Measured::unavailable(copilot_usage::QUOTA_UNAVAILABLE));
    let tokens = |m: &copilot_usage::CopilotModelUsage| TokensReport {
        input: Measured::from_option(
            m.fresh_input_tokens,
            "no request for this model carried a token_details_json blob, and \
             the input_tokens column conflates fresh input with cache traffic, \
             so the fresh split is unknown",
        ),
        cache_read: Measured::Ok {
            value: m.cache_read_tokens,
        },
        cache_write: Measured::Ok {
            value: m.cache_write_tokens,
        },
        output: Measured::Ok {
            value: m.output_tokens,
        },
        reasoning: Measured::Ok {
            value: m.reasoning_tokens,
        },
        total: Measured::Ok {
            value: m.input_tokens + m.output_tokens,
        },
    };
    let spend = |m: &copilot_usage::CopilotModelUsage| SpendReport {
        cost_usd: Measured::Ok { value: m.cost_usd },
        nano_aiu: Measured::Ok { value: m.nano_aiu },
        basis: CostBasis::VendorRecorded,
    };
    let account = AccountReport {
        // Copilot's store is per-machine, not per-login, and records no account
        // identity — so there is exactly one bucket and its key is the file.
        account: Some(u.source.to_string_lossy().into_owned()),
        label: "copilot".to_string(),
        is_default: true,
        source: ReportSource::Disk,
        observed_at: u.last_event_at.as_deref().and_then(parse_iso),
        fresh: None,
        failure: None,
        windows: WindowsReport {
            five_hour: unavailable(),
            seven_day: unavailable(),
            monthly: unavailable(),
        },
        spend: spend(&u.totals),
        tokens: tokens(&u.totals),
        models: u
            .by_model
            .iter()
            .map(|m| ModelReport {
                model: m.model.clone(),
                requests: Measured::Ok { value: m.requests },
                tokens: tokens(m),
                spend: spend(m),
            })
            .collect(),
        live_sessions: live,
    };
    ProviderReport {
        provider: "copilot".to_string(),
        accounts: vec![account],
        note: Some(format!(
            "{} requests over {} sessions, recorded by GitHub itself",
            u.totals.requests, u.sessions
        )),
    }
}

fn parse_iso(s: &str) -> Option<i64> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

fn live_sessions_for(store: &SessionStore, provider: &str) -> u64 {
    store
        .list()
        .iter()
        .filter(|s| s.provider == provider && s.mode != super::state::SessionMode::Stopped)
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The account key for Codex is its HOME, not the sessions directory
    /// underneath it — caught by reading a live report, so it is pinned here.
    #[test]
    fn the_codex_account_key_is_the_codex_home() {
        assert_eq!(
            codex_home_of(std::path::Path::new("/home/u/.codex/sessions/2026/08/28")),
            "/home/u/.codex",
        );
    }

    /// THE JOIN between attribution and the report, on the real hazard.
    ///
    /// A session whose transcript path resolves into a `projects` directory two
    /// logins share cannot be attributed, and its spend must land in the
    /// null-account row — NOT in the default account's total, which is where
    /// the old string-only derivation silently put it. `account: null` is the
    /// fourth value the module note describes and it is not a bucket: a
    /// consumer reads it as unknown.
    #[cfg(unix)]
    #[test]
    fn an_unattributable_session_lands_in_the_null_row_not_the_default() {
        let base = crate::testtmp::dir().join(format!("report-attr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let primary = base.join(".claude");
        let work = primary.join("accounts").join("work");
        std::fs::create_dir_all(primary.join("projects").join("-p")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::os::unix::fs::symlink(primary.join("projects"), work.join("projects")).unwrap();
        let transcript = work.join("projects").join("-p").join("a.jsonl");
        std::fs::write(&transcript, "").unwrap();
        let resolved = std::fs::canonicalize(&transcript)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let roots = vec![
            primary.to_string_lossy().into_owned(),
            work.to_string_lossy().into_owned(),
        ];

        // No spawn stamp — the case the fallback has to answer.
        let mut s = SessionState::new("s-resolved".into(), None);
        s.provider = "claude".into();
        s.transcript_path = Some(resolved);
        assert!(
            matches!(
                s.claude_account_attribution(&roots),
                RootAttribution::Ambiguous { .. }
            ),
            "fixture must produce the ambiguous case",
        );

        assert_eq!(
            account_bucket_key(&s, &roots),
            None,
            "an unattributable session must bucket as null, NOT as the primary \
             account — that fold is the silent merge this layer exists to stop",
        );

        // …while a session that DOES carry its spawn stamp still buckets to its
        // own account, so the honesty above is not bought with uselessness.
        let mut stamped = SessionState::new("s-stamped".into(), None);
        stamped.provider = "claude".into();
        stamped.config_root = Some(work.to_string_lossy().into_owned());
        assert_eq!(
            account_bucket_key(&stamped, &roots),
            Some(work.to_string_lossy().into_owned()),
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The three values are three distinct JSON documents. This is the whole
    /// contract a client codes against, so it is pinned literally rather than
    /// asserted field by field.
    #[test]
    fn zero_unknown_and_unavailable_serialize_differently() {
        let zero = serde_json::to_value(Measured::Ok { value: 0u64 }).unwrap();
        let unknown = serde_json::to_value(Measured::<u64>::unknown("token expired")).unwrap();
        let unavail = serde_json::to_value(Measured::<u64>::unavailable("403")).unwrap();
        assert_eq!(zero, serde_json::json!({ "state": "ok", "value": 0 }));
        assert_eq!(
            unknown,
            serde_json::json!({ "state": "unknown", "reason": "token expired" }),
        );
        assert_eq!(
            unavail,
            serde_json::json!({ "state": "unavailable", "reason": "403" }),
        );
        assert_ne!(zero, unknown);
        assert_ne!(unknown, unavail);
    }

    /// A report is produced with NO sessions at all — the requirement that
    /// started this work. Every provider appears, whether or not it has data.
    #[test]
    fn a_cold_daemon_still_reports_every_provider() {
        let store = SessionStore::new();
        let r = build(&store);
        let names: Vec<&str> = r.providers.iter().map(|p| p.provider.as_str()).collect();
        assert_eq!(names, vec!["claude", "codex", "copilot"]);
        // Claude always has at least the default account row, even cold.
        let claude = &r.providers[0];
        assert!(
            claude.accounts.iter().any(|a| a.is_default),
            "the default account must always have a row: {:?}",
            claude.accounts,
        );
        // …and its windows must be UNKNOWN, never a zero gauge.
        let default = claude.accounts.iter().find(|a| a.is_default).unwrap();
        assert!(
            matches!(default.windows.five_hour.used_percent, Measured::Unknown { .. }),
            "an unpolled window is unknown, not 0%: {:?}",
            default.windows.five_hour,
        );
    }

    /// A failing account still gets a row, and the row says WHICH failure.
    /// Silence and 0% are the two renderings this must never produce.
    #[test]
    fn a_reauth_failure_is_reported_as_unknown_with_its_kind() {
        let store = SessionStore::new();
        store.set_account_usage_error(
            "",
            account_usage::UsageError::for_test(
                account_usage::UsageFailure::NeedsReauth,
                "oauth token expired",
            ),
        );
        let r = build(&store);
        let default = r.providers[0]
            .accounts
            .iter()
            .find(|a| a.is_default)
            .unwrap();
        let f = default.failure.as_ref().expect("a classified failure");
        assert_eq!(f.kind, account_usage::UsageFailure::NeedsReauth);
        match &default.windows.five_hour.used_percent {
            Measured::Unknown { reason } => {
                assert!(reason.contains("NeedsReauth"), "{reason}");
                assert!(reason.contains("oauth token expired"), "{reason}");
            }
            other => panic!("a failed poll must be unknown, not {other:?}"),
        }
    }

    /// A real reading of 0% is ZERO — an account that has used nothing — and
    /// must not be confused with the unknown above.
    #[test]
    fn a_genuine_zero_reading_is_ok_zero_not_unknown() {
        let store = SessionStore::new();
        store.set_account_usage(
            "",
            account_usage::parse_usage_response(&serde_json::json!({
                "five_hour": { "utilization": 0.0 },
                "seven_day": { "utilization": 0.0 },
            })),
        );
        let r = build(&store);
        let default = r.providers[0]
            .accounts
            .iter()
            .find(|a| a.is_default)
            .unwrap();
        assert_eq!(
            default.windows.five_hour.used_percent,
            Measured::Ok { value: 0.0 },
        );
        // …while the monthly window is UNAVAILABLE, because extra usage is off:
        // a third value again, and not the same as either of the others.
        assert!(matches!(
            default.windows.monthly.used_percent,
            Measured::Unavailable { .. }
        ));
    }

    /// Copilot quota is the type case for UNAVAILABLE, and the reason travels
    /// with it so a client can explain a blank gauge instead of inventing one.
    #[test]
    fn copilot_quota_is_unavailable_with_the_403_reason() {
        let store = SessionStore::new();
        let r = build(&store);
        let copilot = r
            .providers
            .iter()
            .find(|p| p.provider == "copilot")
            .unwrap();
        // Skipped rather than failed on a machine with no Copilot install —
        // the shape is what is under test, and there is nothing to shape.
        let Some(account) = copilot.accounts.first() else {
            assert!(copilot.note.is_some(), "an empty provider must say why");
            return;
        };
        for w in [
            &account.windows.five_hour,
            &account.windows.seven_day,
            &account.windows.monthly,
        ] {
            match &w.used_percent {
                Measured::Unavailable { reason } => assert!(reason.contains("403"), "{reason}"),
                other => panic!("copilot quota must be unavailable, not {other:?}"),
            }
        }
        assert_eq!(account.spend.basis, CostBasis::VendorRecorded);
    }

    /// The whole document round-trips to JSON. Cheap, and it is the only thing
    /// that proves a sibling can actually build against the shape.
    #[test]
    fn the_report_serializes() {
        let store = SessionStore::new();
        let v = serde_json::to_value(build(&store)).unwrap();
        assert!(v.get("generated_at").is_some());
        assert_eq!(v["providers"].as_array().unwrap().len(), 3);
        assert_eq!(v["providers"][0]["provider"], "claude");
    }
}

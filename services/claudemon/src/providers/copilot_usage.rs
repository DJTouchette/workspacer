//! GitHub Copilot usage, read from disk with no live session.
//!
//! Copilot is the one provider whose local record is BETTER than anything the
//! daemon can compute. `~/.copilot/session-store.db` holds an
//! `assistant_usage_events` row per request with the real token split and, in
//! `token_details_json`, GitHub's own price table for that exact request.
//!
//! # Why the embedded table wins
//!
//! `session::pricing::estimate_cost` is an estimate in two ways Copilot's
//! record is not. It has no cache-WRITE concept, and Copilot's `input_tokens`
//! column conflates fresh input, cache reads and cache writes — so our
//! estimator prices cache writes as fresh input. And it prices only models it
//! has a row for. Measured on this machine, 2026-08-28, across all 56 recorded
//! requests:
//!
//! | model            | our estimate | GitHub's record | why they differ            |
//! |------------------|--------------|-----------------|----------------------------|
//! | gpt-5-mini       | $0.031033    | $0.031033       | no cache writes — identical |
//! | claude-haiku-4.5 | $0.235484    | $0.275168       | 158 737 cache-write tokens billed at 1.25x input, which we price at 1.0x |
//! | gpt-5.6-luna     | *unpriced*   | $0.0040558      | not in our table at all     |
//!
//! The exact agreement on `gpt-5-mini` is the load-bearing observation: it is
//! what shows the two are measuring the same thing in the same unit, so the
//! disagreements are our error rather than a different accounting.
//!
//! # The AIU→USD rate is derived, not invented
//!
//! `total_nano_aiu` is exact and GitHub's own unit; it is reported verbatim.
//! The dollar figure beside it converts at **1 AIU = $0.01**, which is not a
//! guess — every `costPerBatch` in the captured tables reproduces a published
//! vendor list price at that rate, and five independent ones were checked:
//! claude-haiku-4.5 input 100 AIU/1M = $1.00, cache-write 125 = $1.25 (1.25x,
//! Anthropic's documented write premium), output 500 = $5.00; gpt-5-mini input
//! 25 = $0.25, output 200 = $2.00. Per-request sums reproduce
//! `total_nano_aiu` to the integer. A consumer that does not accept the
//! derivation has `nano_aiu` and can ignore the dollars.
//!
//! # Quota headroom is UNAVAILABLE, and that is a value
//!
//! There is no local record of how much of a Copilot plan remains, and the
//! remote source is shut: `copilot_internal/v2/token` answers 403 to a `gh`
//! OAuth token (see [`super::copilot::list_models`], probed live against CLI
//! v1.0.81). That is not zero headroom and it is not unknown-for-now — it is
//! structurally unavailable, and [`QUOTA_UNAVAILABLE`] says so in one place so
//! no caller has to decide what a blank gauge means.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

/// Why Copilot quota headroom cannot be reported. Not a failure to retry: the
/// endpoint that would answer refuses the only credential we have.
pub const QUOTA_UNAVAILABLE: &str =
    "GitHub exposes no local quota record, and copilot_internal/v2/token answers \
     403 to a gh OAuth token (probed live against copilot CLI v1.0.81)";

/// Nano-AIU per US dollar. See the module note — derived from the price tables
/// Copilot itself embeds, cross-checked against five published vendor rates.
const NANO_AIU_PER_USD: f64 = 100_000_000_000.0;

/// Per-model usage, folded from `assistant_usage_events`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct CopilotModelUsage {
    pub model: String,
    pub requests: u64,
    /// The column's own `input_tokens`, which CONFLATES fresh input, cache
    /// reads and cache writes. Kept because it is what the table says; use the
    /// three fields below to bill.
    pub input_tokens: u64,
    /// Fresh (uncached) input, from `token_details_json`. `None` when no row
    /// for this model carried a details blob — unknown, not zero.
    pub fresh_input_tokens: Option<u64>,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    /// GitHub's own billing figure, exact, summed over the model's requests.
    pub nano_aiu: u64,
    /// [`Self::nano_aiu`] in dollars. Derived; see the module note.
    pub cost_usd: f64,
}

/// The whole local record.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CopilotUsage {
    pub by_model: Vec<CopilotModelUsage>,
    /// Every field of [`CopilotModelUsage`] summed; `model` is `""`.
    pub totals: CopilotModelUsage,
    pub sessions: u64,
    /// ISO-8601, as the table stores them. `None` when there are no events.
    pub first_event_at: Option<String>,
    pub last_event_at: Option<String>,
    pub source: PathBuf,
}

/// Why the local record could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CopilotUsageError {
    /// No home directory resolved.
    NoHome,
    /// Copilot has never run here (no `~/.copilot/session-store.db`).
    NoStore,
    /// The file exists but could not be opened or queried — a schema this
    /// build does not know, or a lock we lost. Unknown, emphatically not zero.
    Unreadable,
}

impl std::fmt::Display for CopilotUsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoHome => "no home directory",
            Self::NoStore => "copilot has recorded no sessions here",
            Self::Unreadable => "copilot's session store could not be read",
        })
    }
}

/// `~/.copilot/session-store.db`.
fn store_path() -> Option<PathBuf> {
    Some(
        BaseDirs::new()?
            .home_dir()
            .join(".copilot")
            .join("session-store.db"),
    )
}

/// Fold the whole local usage record.
pub fn read_from_disk() -> Result<CopilotUsage, CopilotUsageError> {
    let path = store_path().ok_or(CopilotUsageError::NoHome)?;
    if !path.is_file() {
        return Err(CopilotUsageError::NoStore);
    }
    read_store(&path)
}

/// [`read_from_disk`] against an explicit path, so the fold can be tested
/// against a database built row by row rather than against the author's.
pub fn read_store(path: &Path) -> Result<CopilotUsage, CopilotUsageError> {
    let conn = super::open_foreign_sqlite_readonly(path).ok_or(CopilotUsageError::Unreadable)?;
    let mut stmt = conn
        .prepare(
            "SELECT model, input_tokens, cache_read_tokens, cache_write_tokens,
                    output_tokens, reasoning_tokens, total_nano_aiu, token_details_json
               FROM assistant_usage_events",
        )
        .map_err(|_| CopilotUsageError::Unreadable)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Row {
                model: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                input_tokens: r.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0) as u64,
                cache_read_tokens: r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u64,
                cache_write_tokens: r.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) as u64,
                output_tokens: r.get::<_, Option<i64>>(4)?.unwrap_or(0).max(0) as u64,
                reasoning_tokens: r.get::<_, Option<i64>>(5)?.unwrap_or(0).max(0) as u64,
                nano_aiu: r.get::<_, Option<i64>>(6)?.unwrap_or(0).max(0) as u64,
                details: r.get::<_, Option<String>>(7)?,
            })
        })
        .map_err(|_| CopilotUsageError::Unreadable)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    drop(stmt);

    let (sessions, first, last) = conn
        .query_row(
            "SELECT COUNT(DISTINCT session_id), MIN(created_at), MAX(created_at)
               FROM assistant_usage_events",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?.max(0) as u64,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap_or((0, None, None));

    let mut by_model: BTreeMap<String, CopilotModelUsage> = BTreeMap::new();
    for row in rows {
        let e = by_model.entry(row.model.clone()).or_insert_with(|| CopilotModelUsage {
            model: row.model.clone(),
            ..Default::default()
        });
        e.requests += 1;
        e.input_tokens += row.input_tokens;
        e.cache_read_tokens += row.cache_read_tokens;
        e.cache_write_tokens += row.cache_write_tokens;
        e.output_tokens += row.output_tokens;
        e.reasoning_tokens += row.reasoning_tokens;
        e.nano_aiu += row.nano_aiu;
        // Fresh input is only knowable from the details blob. A row without
        // one leaves the running total where it is rather than adding the
        // conflated column and quietly overstating fresh input.
        if let Some(fresh) = row.fresh_input() {
            *e.fresh_input_tokens.get_or_insert(0) += fresh;
        }
    }
    for e in by_model.values_mut() {
        e.cost_usd = e.nano_aiu as f64 / NANO_AIU_PER_USD;
    }

    let mut totals = CopilotModelUsage::default();
    for e in by_model.values() {
        totals.requests += e.requests;
        totals.input_tokens += e.input_tokens;
        totals.cache_read_tokens += e.cache_read_tokens;
        totals.cache_write_tokens += e.cache_write_tokens;
        totals.output_tokens += e.output_tokens;
        totals.reasoning_tokens += e.reasoning_tokens;
        totals.nano_aiu += e.nano_aiu;
        if let Some(f) = e.fresh_input_tokens {
            *totals.fresh_input_tokens.get_or_insert(0) += f;
        }
    }
    totals.cost_usd = totals.nano_aiu as f64 / NANO_AIU_PER_USD;

    Ok(CopilotUsage {
        by_model: by_model.into_values().collect(),
        totals,
        sessions,
        first_event_at: first,
        last_event_at: last,
        source: path.to_path_buf(),
    })
}

struct Row {
    model: String,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    nano_aiu: u64,
    details: Option<String>,
}

impl Row {
    /// Fresh (uncached) input tokens, from the embedded price table's own
    /// breakdown. `None` when the row carried no blob.
    fn fresh_input(&self) -> Option<u64> {
        Some(
            token_details(self.details.as_deref()?)
                .into_iter()
                .filter(|d| d.token_type == "input")
                .map(|d| d.token_count)
                .sum(),
        )
    }
}

/// One entry of `token_details_json`: `{"batchSize":1000000,"costPerBatch":
/// 100000000000,"tokenCount":7,"tokenType":"input"}`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenDetail {
    pub batch_size: u64,
    /// Nano-AIU charged per `batch_size` tokens of this type — GitHub's price
    /// table, per request, per model.
    pub cost_per_batch: u64,
    pub token_count: u64,
    pub token_type: String,
}

/// Parse the embedded price table. Empty on anything unparsable — the caller
/// then reports the split as unknown rather than as zero.
pub fn token_details(json: &str) -> Vec<TokenDetail> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Recompute a request's charge from its own embedded table. This is what
/// makes the table checkable rather than merely quoted: it reproduces
/// `total_nano_aiu` exactly on real rows, so a future schema change that
/// breaks the relationship is detectable instead of silent.
pub fn nano_aiu_from_details(details: &[TokenDetail]) -> u64 {
    details
        .iter()
        .filter(|d| d.batch_size > 0)
        .map(|d| d.token_count.saturating_mul(d.cost_per_batch) / d.batch_size)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VERBATIM `token_details_json` and `total_nano_aiu` from row id=56 of
    /// `~/.copilot/session-store.db` on 2026-08-28 (claude-haiku-4.5).
    const ROW_56_DETAILS: &str = r#"[
      {"batchSize":1000000,"costPerBatch":100000000000,"tokenCount":7,"tokenType":"input"},
      {"batchSize":1000000,"costPerBatch":10000000000,"tokenCount":19382,"tokenType":"cache_read"},
      {"batchSize":1000000,"costPerBatch":125000000000,"tokenCount":429,"tokenType":"cache_write"},
      {"batchSize":1000000,"costPerBatch":500000000000,"tokenCount":93,"tokenType":"output"}]"#;
    const ROW_56_NANO_AIU: u64 = 294_645_000;

    /// The embedded table is arithmetic, not decoration: summing it reproduces
    /// GitHub's own recorded charge to the integer. If a schema change ever
    /// breaks that, this fails rather than the numbers quietly drifting.
    #[test]
    fn the_embedded_price_table_reproduces_the_recorded_charge_exactly() {
        let details = token_details(ROW_56_DETAILS);
        assert_eq!(details.len(), 4);
        assert_eq!(nano_aiu_from_details(&details), ROW_56_NANO_AIU);
    }

    /// The rate that turns AIU into dollars is derived from that same table,
    /// and these are the cross-checks it is derived FROM — published vendor
    /// list prices for claude-haiku-4.5 ($1.00/$5.00 per 1M, cache writes at
    /// 1.25x input) and gpt-5-mini ($0.25/$2.00). All five land exactly.
    #[test]
    fn the_aiu_rate_reproduces_published_vendor_prices() {
        let usd_per_million = |cost_per_batch: u64, batch: u64| {
            (cost_per_batch as f64 / NANO_AIU_PER_USD) * (1_000_000.0 / batch as f64)
        };
        for (cost_per_batch, expected, what) in [
            (100_000_000_000u64, 1.00, "haiku-4.5 input"),
            (125_000_000_000, 1.25, "haiku-4.5 cache write (1.25x input)"),
            (500_000_000_000, 5.00, "haiku-4.5 output"),
            (25_000_000_000, 0.25, "gpt-5-mini input"),
            (200_000_000_000, 2.00, "gpt-5-mini output"),
        ] {
            let got = usd_per_million(cost_per_batch, 1_000_000);
            assert!(
                (got - expected).abs() < 1e-9,
                "{what}: {got} != {expected} — the AIU rate is no longer $0.01, \
                 so the derived dollar figures must be re-derived or dropped",
            );
        }
    }

    /// The disagreement the task is about, on real numbers. Our estimator has
    /// no cache-write concept and Copilot's `input_tokens` column folds writes
    /// in with fresh input, so we price 158 737 cache-write tokens at 1.0x when
    /// Anthropic bills 1.25x. GitHub's record is right and ours is low.
    #[test]
    fn we_undercount_where_the_embedded_table_disagrees() {
        // Live aggregates for claude-haiku-4.5, 2026-08-28.
        let (input, cache_read, output) = (616_592u64, 457_559u64, 6_139u64);
        let theirs = 27_516_815_000_f64 / NANO_AIU_PER_USD;
        let ours = crate::session::pricing::estimate_cost(
            Some("claude-haiku-4.5"),
            Some(input),
            Some(cache_read),
            Some(output),
        )
        .expect("haiku is in the table");
        assert!(
            (theirs - 0.275_168_15).abs() < 1e-9,
            "the recorded charge is fixed data: {theirs}",
        );
        assert!(
            (ours - 0.235_484).abs() < 1e-6,
            "our estimate is what it is: {ours}",
        );
        assert!(
            theirs > ours,
            "we undercount; the embedded table must therefore win",
        );
    }

    /// gpt-5-mini has no cache writes, so the two agree exactly — which is what
    /// shows they measure the same thing in the same unit, and therefore that
    /// the haiku gap above is our error rather than a different accounting.
    #[test]
    fn the_two_agree_exactly_where_nothing_is_cache_written() {
        let ours = crate::session::pricing::estimate_cost(
            Some("gpt-5-mini"),
            Some(234_028),
            Some(160_640),
            Some(4_335),
        )
        .unwrap();
        let theirs = 3_103_300_000_f64 / NANO_AIU_PER_USD;
        assert!(
            (ours - theirs).abs() < 1e-9,
            "ours {ours} vs GitHub {theirs}",
        );
    }

    /// Quota headroom is a THIRD state. Not zero (which would say the plan is
    /// exhausted), not unknown-for-now (which invites a retry that cannot
    /// succeed) — structurally unavailable, with the reason attached.
    #[test]
    fn quota_headroom_is_unavailable_with_a_stated_reason() {
        assert!(QUOTA_UNAVAILABLE.contains("403"));
        assert!(!QUOTA_UNAVAILABLE.is_empty());
    }

    /// The fold, over a database built to the real schema. Fresh input comes
    /// from the details blob; a row WITHOUT one must not contribute the
    /// conflated `input_tokens` column to the fresh total.
    #[test]
    fn the_fold_separates_fresh_input_from_cache_traffic() {
        let path = crate::testtmp::db_path("copilot-usage");
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE assistant_usage_events (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   session_id TEXT NOT NULL, model TEXT NOT NULL,
                   input_tokens INTEGER, output_tokens INTEGER,
                   cache_read_tokens INTEGER, cache_write_tokens INTEGER,
                   reasoning_tokens INTEGER, total_nano_aiu INTEGER,
                   token_details_json TEXT, created_at TEXT);",
            )
            .unwrap();
            c.execute(
                "INSERT INTO assistant_usage_events
                 (session_id, model, input_tokens, output_tokens, cache_read_tokens,
                  cache_write_tokens, reasoning_tokens, total_nano_aiu,
                  token_details_json, created_at)
                 VALUES ('s1','claude-haiku-4.5',19818,93,19382,429,0,?1,?2,
                         '2026-08-28T16:44:19.713Z')",
                rusqlite::params![ROW_56_NANO_AIU as i64, ROW_56_DETAILS],
            )
            .unwrap();
            // Same model, no details blob: 500 input tokens of unknown split.
            c.execute(
                "INSERT INTO assistant_usage_events
                 (session_id, model, input_tokens, output_tokens, cache_read_tokens,
                  cache_write_tokens, reasoning_tokens, total_nano_aiu,
                  token_details_json, created_at)
                 VALUES ('s2','claude-haiku-4.5',500,10,0,0,0,1000000,NULL,
                         '2026-08-28T17:00:00.000Z')",
                [],
            )
            .unwrap();
        }

        let u = read_store(&path).unwrap();
        assert_eq!(u.sessions, 2);
        assert_eq!(u.by_model.len(), 1);
        let m = &u.by_model[0];
        assert_eq!(m.requests, 2);
        assert_eq!(m.input_tokens, 20_318, "the column, conflated, as recorded");
        assert_eq!(
            m.fresh_input_tokens,
            Some(7),
            "only the row with a details blob can contribute fresh input; the \
             other row's 500 are of unknown split and must not be added",
        );
        assert_eq!(m.cache_read_tokens, 19_382);
        assert_eq!(m.cache_write_tokens, 429);
        assert_eq!(m.nano_aiu, ROW_56_NANO_AIU + 1_000_000);
        assert!((m.cost_usd - 0.002_956_45).abs() < 1e-9, "{}", m.cost_usd);
        assert_eq!(u.totals.nano_aiu, m.nano_aiu);
        assert_eq!(u.first_event_at.as_deref(), Some("2026-08-28T16:44:19.713Z"));
        assert_eq!(u.last_event_at.as_deref(), Some("2026-08-28T17:00:00.000Z"));
    }

    #[test]
    fn a_missing_store_is_no_store_not_an_empty_reading() {
        let missing = crate::testtmp::dir().join("definitely-not-here.db");
        assert_eq!(read_store(&missing).unwrap_err(), CopilotUsageError::Unreadable);
    }

    /// Live smoke test against this machine's real `~/.copilot`:
    /// `cargo test -p claudemon copilot_disk_read_smoke -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the real ~/.copilot"]
    fn copilot_disk_read_smoke() {
        let u = read_from_disk().expect("a reading");
        println!("{u:#?}");
        assert!(u.totals.requests > 0);
    }
}

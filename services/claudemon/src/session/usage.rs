//! Per-session token/cost/context tracking, ported verbatim from the wks-tui
//! crate's `usage.rs` (which itself mirrors the Electron app's `modelUsage.ts`
//! + `claudeSessionStore.applyUsage`).
//!
//! Every assistant message in Claude Code's JSONL transcript carries a `usage`
//! block and a `model` id. We fold each assistant turn to produce cumulative
//! cost and a point-in-time view of context fullness.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::transcript::Transcript;

/// How a session's cumulative prompt tokens divided between fresh input, cache
/// writes and cache reads.
///
/// Only ever present once a provider has actually reported cache fields.
/// Absence means "not reported", never "zero". Clients must omit the readout
/// rather than draw a session that cached nothing.
///
/// TWIN: `CacheTokenSplit` in apps/desktop/src/main/services/modelUsage.ts. The
/// field names are identical on the wire, so the hub's camelCase projection
/// (cmd/brain/enrich.go) passes the object through unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheSplit {
    /// Prompt tokens processed fresh, at the full input rate.
    pub fresh: u64,
    /// Prompt tokens written into the cache this session.
    pub write: u64,
    /// Prompt tokens served back from the cache.
    pub read: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub model: Option<String>,
    /// Latest turn's input side — a point-in-time view of context fullness.
    pub context_tokens: u64,
    pub context_limit: u64,
    /// Cumulative cost over the session.
    pub cost_usd: f64,
    /// Cumulative fresh / cache-write / cache-read split of the prompt side.
    /// `None` until a turn arrives carrying cache fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheSplit>,
}

struct Rates {
    /// USD per million input tokens. Cache writes bill at a multiple of this
    /// (see `CACHE_WRITE_5M_MULTIPLIER` / `CACHE_WRITE_1H_MULTIPLIER`);
    /// cache-read = the table's cached rate (0.1× input when the entry has
    /// none).
    input: f64,
    output: f64,
    cached_input: Option<f64>,
    context_limit: u64,
}

const DEFAULT_RATES: Rates = Rates {
    input: 3.0,
    output: 15.0,
    cached_input: None,
    context_limit: 200_000,
};

/// Longest-prefix match on the transcript `model` id, via the shared pricing
/// table (built-ins + the user's ~/.workspacer/model-rates.json overrides).
/// Unknown models fall back to Sonnet-tier defaults, as before.
fn rates_for(model: Option<&str>) -> Rates {
    let Some(model) = model else {
        return DEFAULT_RATES;
    };
    match super::pricing::rates_for(model) {
        Some(r) => Rates {
            input: r.input,
            output: r.output,
            cached_input: r.cached_input,
            context_limit: r.context_limit.unwrap_or(200_000),
        },
        None => DEFAULT_RATES,
    }
}

/// Tokens occupying the window this turn: input + both cache tiers.
fn context_tokens_of(usage: &Value) -> u64 {
    usage_u64(usage, "input_tokens")
        + usage_u64(usage, "cache_creation_input_tokens")
        + usage_u64(usage, "cache_read_input_tokens")
}

/// Window implied by the transcript alone: the rates table for the model id,
/// with a last-resort retrospective promotion — the transcript model id lacks
/// the `[1m]` suffix, so a session whose window is otherwise unknown only
/// reveals 1M mode by exceeding the standard 200k. [`Usage::resolve_window`]
/// overrides this with real signals when the session has any.
fn context_limit_for(model: Option<&str>, observed: u64) -> u64 {
    let base = rates_for(model).context_limit;
    if base <= 200_000 && observed > 200_000 {
        1_000_000
    } else {
        base
    }
}

impl Usage {
    /// Replace the transcript-derived `context_limit` with what the *session*
    /// knows about its window — the transcript can't say, because Claude Code
    /// strips the `[1m]` marker from the `model` id it records.
    ///
    /// `reported` is the provider's own window (the status line's
    /// `context_window_size`: Claude's statusLine payload on the PTY path, the
    /// stream `result` frame's `modelUsage.*.contextWindow` on the managed
    /// one, `model_context_window` for Codex) — a fact, so it wins outright.
    /// `requested` is the model string the session was spawned with
    /// (`opus[1m]`), which still carries the marker and is known from token
    /// zero; it may only *raise* the window, never lower one the rates table
    /// (or a user override) already resolved higher.
    pub fn resolve_window(&mut self, reported: Option<u64>, requested: Option<&str>) {
        if let Some(w) = reported.filter(|w| *w > 0) {
            self.context_limit = w;
            return;
        }
        if let Some(w) = requested.and_then(crate::providers::requested_context_window_for) {
            self.context_limit = self.context_limit.max(w);
        }
    }
}

/// [`usage_for_path`] for a live session, with its window resolved from the
/// session's own signals (see [`Usage::resolve_window`]). This is what every
/// snapshot should use — `usage_for_path` alone can only guess the window.
pub fn usage_for_session(state: &super::state::SessionState) -> Usage {
    let mut u = usage_for_path(state.transcript_path.as_deref());
    u.resolve_window(
        state
            .status_line
            .as_ref()
            .and_then(|sl| sl.context_window_size),
        state.requested_model.as_deref(),
    );
    u
}

// ── Cache multipliers ────────────────────────────────────────────────
//
// A cache write costs more than fresh input because the write is kept alive for
// a chosen lifetime, and the price scales with that lifetime: 1.25× the base
// input rate at the 5-minute TTL, 2× at the 1-hour TTL. Reads cost 0.1×.
//
// These were a single hardcoded 1.25× until 2026-08-24, which is the 5-minute
// rate. Workspacer's own sessions are almost entirely 1-hour, so every displayed
// cost charged 1.25× for writes the account was billed 2× for: the real write
// cost is 1.6× what was shown. The transcript has carried the answer the whole
// time, in `usage.cache_creation`, which tags each write with its TTL.
//
// TWIN: apps/desktop/src/main/services/modelUsage.ts (turnCostUSD). The two are
// pinned to each other by contracts/model-pricing-cases.json's
// `cacheMultiplierCases` block. Edit one side and the other's test goes red.

/// Cache writes held for 5 minutes bill at 1.25× the base input rate.
pub const CACHE_WRITE_5M_MULTIPLIER: f64 = 1.25;
/// Cache writes held for 1 hour bill at 2× the base input rate.
pub const CACHE_WRITE_1H_MULTIPLIER: f64 = 2.0;
/// Cache reads bill at 0.1× the base input rate, absent a per-model rate.
pub const CACHE_READ_MULTIPLIER: f64 = 0.1;

/// Read a `usage` field as a `u64`, warning (and yielding 0) on a value that is
/// present but not a number.
fn usage_u64(usage: &Value, key: &str) -> u64 {
    match usage.get(key) {
        None => 0,
        Some(val) => match val.as_u64() {
            Some(n) => n,
            None => {
                tracing::warn!(field = key, raw = %val, "usage field present but not a valid u64; treating as 0");
                0
            }
        },
    }
}

/// The per-TTL cache-write tokens a turn reports, or `None` when it carries no
/// `cache_creation` block for them to come from.
fn cache_write_ttl_split(usage: &Value) -> Option<(u64, u64)> {
    let cc = usage.get("cache_creation")?;
    let m5 = cc.get("ephemeral_5m_input_tokens");
    let m1h = cc.get("ephemeral_1h_input_tokens");
    if m5.is_none() && m1h.is_none() {
        return None;
    }
    Some((
        m5.and_then(Value::as_u64).unwrap_or(0),
        m1h.and_then(Value::as_u64).unwrap_or(0),
    ))
}

/// USD cost of a turn's cache writes, weighted by the TTL split.
///
/// THE NO-SPLIT FALLBACK, stated rather than assumed: when a turn reports writes
/// but no `cache_creation` block, we cannot tell which lifetime was bought, and
/// we price the whole amount at the 1-hour rate. Defaulting to the cheaper 5m
/// rate is the exact bug this function exists to fix. It reads as a lower bill
/// than the account will see, and a cost readout that is too low is worse than
/// one that is too high. The same rule covers writes the split does not account
/// for (`cache_creation_input_tokens` larger than the two TTL fields sum to).
fn cache_write_cost_usd(input_rate: f64, usage: &Value) -> f64 {
    let total = usage_u64(usage, "cache_creation_input_tokens");
    let rate_5m = input_rate * CACHE_WRITE_5M_MULTIPLIER;
    let rate_1h = input_rate * CACHE_WRITE_1H_MULTIPLIER;
    let Some((m5, m1h)) = cache_write_ttl_split(usage) else {
        return total as f64 * rate_1h;
    };
    let unattributed = total.saturating_sub(m5.saturating_add(m1h));
    m5 as f64 * rate_5m + m1h as f64 * rate_1h + unattributed as f64 * rate_1h
}

/// The fresh / cache-write / cache-read split of one turn's prompt, or `None`
/// when the turn reports no cache fields at all.
///
/// `None` is the honest answer for a turn that says nothing about caching:
/// folding it in as three zeros would make a provider that does not itemize look
/// like one that cached nothing, and every surface downstream would draw a 0%
/// hit rate it has no basis for.
fn cache_split_of(usage: &Value) -> Option<CacheSplit> {
    let has_write = usage.get("cache_creation_input_tokens").is_some();
    let has_read = usage.get("cache_read_input_tokens").is_some();
    if !has_write && !has_read && usage.get("cache_creation").is_none() {
        return None;
    }
    Some(CacheSplit {
        fresh: usage_u64(usage, "input_tokens"),
        write: usage_u64(usage, "cache_creation_input_tokens"),
        read: usage_u64(usage, "cache_read_input_tokens"),
    })
}

/// USD cost of one turn. Cache writes bill per TTL (see
/// [`cache_write_cost_usd`]), reads 0.1×.
fn turn_cost_usd(model: Option<&str>, usage: &Value) -> f64 {
    let r = rates_for(model);
    let n = |k: &str| usage_u64(usage, k) as f64;
    let dollars = n("input_tokens") * r.input
        + cache_write_cost_usd(r.input, usage)
        + n("cache_read_input_tokens") * r.cached_input.unwrap_or(r.input * CACHE_READ_MULTIPLIER)
        + n("output_tokens") * r.output;
    dollars / 1_000_000.0
}

/// Fold a session's transcript (a raw JSON value shaped like
/// `{"messages": [{"role": "...", "raw": <jsonl-row>}]}`) into a [`Usage`].
///
/// This overload accepts the same `Value` shape the wks-tui crate produces,
/// and is kept for parity / testing.
#[cfg(test)]
fn from_transcript_value(tx: &Value) -> Option<Usage> {
    let messages = tx.get("messages")?.as_array()?;
    let mut usage = Usage::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut any = false;
    let mut peak_context: u64 = 0;

    for m in messages {
        if m.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let Some(raw) = m.get("raw") else { continue };
        let Some(msg) = raw.get("message") else {
            continue;
        };
        let Some(u) = msg.get("usage") else { continue };
        any = true;

        let sidechain = raw
            .get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Neutralize Claude's `<synthetic>` placeholder (see from_transcript).
        let raw_model = msg.get("model").and_then(|m| m.as_str());
        let placeholder = raw_model.is_some_and(|m| m.starts_with('<'));
        let row_model = raw_model.filter(|_| !placeholder);

        // Point-in-time context/model: main thread only, and never from a
        // placeholder row (see from_transcript).
        if !sidechain && !placeholder {
            usage.context_tokens = context_tokens_of(u);
            if let Some(model) = row_model {
                usage.model = Some(model.to_string());
            }
            peak_context = peak_context.max(usage.context_tokens);
            usage.context_limit = context_limit_for(usage.model.as_deref(), peak_context);
        }

        // Cumulative cost — once per distinct message id.
        let id = msg.get("id").and_then(|i| i.as_str()).unwrap_or("");
        if !id.is_empty() && !seen.insert(id.to_string()) {
            continue;
        }
        usage.cost_usd += turn_cost_usd(row_model.or(usage.model.as_deref()), u);
        if let Some(split) = cache_split_of(u) {
            let c = usage.cache.get_or_insert_with(CacheSplit::default);
            c.fresh += split.fresh;
            c.write += split.write;
            c.read += split.read;
        }
    }

    any.then_some(usage)
}

/// Fold a claudemon [`Transcript`] into a [`Usage`]. Context/model come from
/// the last *main-thread* assistant turn; cost accumulates across main and
/// sub-agent (isSidechain) turns alike — each priced at its own model's rates —
/// deduped by message id so streamed blocks of one message aren't
/// double-counted. Returns `None` if no assistant usage was found (empty
/// transcript, no usage blocks, etc.).
///
/// Each `TranscriptMessage.raw` is the whole JSONL row:
/// `{"type": "assistant", "message": {"id": "...", "model": "...", "usage": {...}}}`.
pub fn from_transcript(tx: &Transcript) -> Option<Usage> {
    let mut usage = Usage::default();
    let mut seen: HashSet<String> = HashSet::new();
    // 1M mode is a session-level property: once any main-thread turn exceeds the
    // 200k window the limit must stay promoted even when a later turn's context
    // is smaller (e.g. after auto-compaction). Track the high-water mark, matching
    // the TS reference's `session.peakContext`.
    let mut peak_context: u64 = 0;
    let any = fold_transcript(tx, &mut usage, &mut seen, &mut peak_context, false);
    any.then_some(usage)
}

/// Fold one transcript's assistant turns into `usage`, deduping cost by message
/// id via the caller-owned `seen` set and tracking the caller-owned
/// `peak_context` high-water mark. Returns whether any assistant usage block was
/// found. Splitting this out lets [`usage_for_path`] share ONE `seen` set across
/// the main transcript and every `subagents/*.jsonl` file, so a sub-agent turn
/// that appears both inline (isSidechain) in the main file and in its own
/// sidechain file is billed once — parity with desktop
/// `analyticsBackfill.recomputeSession`, which threads a single Set.
///
/// `force_sidechain` treats EVERY row of `tx` as a sub-agent turn regardless of
/// its own `isSidechain` flag — used for the per-agent `subagents/*.jsonl`
/// files, whose rows all belong to a sub-agent's run and often carry NO
/// `isSidechain` key at all (a sub-agent's own transcript does not know it is a
/// side thread of a parent). Without this, such a row folds as a main-thread
/// turn and clobbers the session's context gauge and reported model with the
/// sub-agent's — exact parity with the TS twin `foldTranscriptFile`, which
/// passes `forceSidechain=true` for every subagent file and computes
/// `sidechain = forceSidechain || row.isSidechain === true`.
fn fold_transcript(
    tx: &Transcript,
    usage: &mut Usage,
    seen: &mut HashSet<String>,
    peak_context: &mut u64,
    force_sidechain: bool,
) -> bool {
    let mut any = false;
    for m in &tx.messages {
        if m.role != "assistant" {
            continue;
        }
        // raw is the full JSONL row; the API message lives at raw.message.
        let Some(msg) = m.raw.get("message") else {
            continue;
        };
        let Some(u) = msg.get("usage") else { continue };
        any = true;

        let sidechain = force_sidechain
            || m.raw
                .get("isSidechain")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        // Neutralize Claude's `<synthetic>` placeholder (and any `<...>` marker):
        // it is not a real model, so it must not clobber the reported model nor
        // force default pricing — synthetic turns inherit the thread model.
        let raw_model = msg.get("model").and_then(|m| m.as_str());
        let placeholder = raw_model.is_some_and(|m| m.starts_with('<'));
        let row_model = raw_model.filter(|_| !placeholder);

        // Point-in-time context/model: main thread only — a sub-agent's turn
        // must not clobber the session's context gauge or reported model.
        // A placeholder row is skipped outright: it is the CLI answering itself
        // (e.g. "No response requested." to a resume kickoff) with all-zero
        // usage, which is not an observation of the window — folding it in
        // zeroes the gauge and makes a healthy session read as frozen.
        if !sidechain && !placeholder {
            usage.context_tokens = context_tokens_of(u);
            if let Some(model) = row_model {
                usage.model = Some(model.to_string());
            }
            *peak_context = (*peak_context).max(usage.context_tokens);
            usage.context_limit = context_limit_for(usage.model.as_deref(), *peak_context);
        }

        // Cumulative cost — once per distinct message id, at the row's own
        // model rates (sub-agents often run a different model). Streamed/replayed
        // blocks without a message.id fall back to the row-level uuid so they are
        // deduped rather than counted repeatedly (parity with the TS backfill).
        let id = msg
            .get("id")
            .and_then(|i| i.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| m.raw.get("uuid").and_then(|u| u.as_str()))
            .unwrap_or("");
        if !id.is_empty() && !seen.insert(id.to_string()) {
            continue;
        }
        usage.cost_usd += turn_cost_usd(row_model.or(usage.model.as_deref()), u);
        // The three prompt tiers, kept apart instead of only summed into
        // `context_tokens`. Folded here (after the dedup `continue`) so a
        // replayed message cannot double-count them.
        if let Some(split) = cache_split_of(u) {
            let c = usage.cache.get_or_insert_with(CacheSplit::default);
            c.fresh += split.fresh;
            c.write += split.write;
            c.read += split.read;
        }
    }
    any
}

/// Compute usage for a session given its `transcript_path`. Returns a zeroed
/// default if the path is `None`, the file doesn't exist, or contains no
/// assistant usage blocks.
///
/// Sub-agent transcripts (`<transcript-stem>/subagents/*.jsonl` — where
/// current Claude Code writes Task/teammate agents) fold in as cost/spend
/// only: their rows are `isSidechain`, so [`from_transcript`] keeps them off
/// the context gauge automatically.
/// Identity of one file folded into a usage figure: (path, len, mtime as ns).
/// Comparing these is a `stat` apiece, versus re-reading and re-parsing the
/// file — and a transcript that hasn't changed can't have changed its usage.
type FileStamp = (String, u64, i128);

/// Memo for [`usage_for_path`], keyed on the main transcript path.
///
/// Every `GET /sessions` re-derives usage for every session, and the desktop
/// polls that list on a 60s timer plus a four-fetch burst after each terminate.
/// Without this, a daemon with ~10 hook-bound sessions re-read and fully
/// re-parsed tens of megabytes of transcript per poll — allocating hundreds of
/// MB of `serde_json::Value` for data that had not changed.
///
/// Bounded because the key space is sessions, not requests; the cap is a
/// backstop against a long-lived daemon accumulating dead sessions' entries.
type UsageCache = HashMap<String, (Vec<FileStamp>, Usage)>;
static USAGE_CACHE: Lazy<Mutex<UsageCache>> = Lazy::new(|| Mutex::new(HashMap::new()));
const MAX_USAGE_CACHE: usize = 256;

/// The main transcript plus each `subagents/*.jsonl`, with their current
/// (len, mtime). Any change to any of them invalidates the memo — including a
/// file shrinking, which a length-only check would miss on a rewritten
/// transcript.
fn usage_inputs(path: &str) -> Vec<FileStamp> {
    let mut stamps: Vec<FileStamp> = Vec::new();
    let mut stamp = |p: String| {
        if let Ok(md) = std::fs::metadata(&p) {
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i128)
                .unwrap_or(0);
            stamps.push((p, md.len(), mtime));
        }
    };
    stamp(path.to_string());
    if let Some(stem) = path.strip_suffix(".jsonl") {
        if let Ok(rd) = std::fs::read_dir(format!("{stem}/subagents")) {
            let mut subs: Vec<String> = rd
                .flatten()
                .map(|ent| ent.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
                .filter_map(|p| p.to_str().map(str::to_string))
                .collect();
            // read_dir order is unspecified; sort so the fingerprint is stable.
            subs.sort();
            for p in subs {
                stamp(p);
            }
        }
    }
    stamps
}

pub fn usage_for_path(transcript_path: Option<&str>) -> Usage {
    let Some(path) = transcript_path else {
        return Usage::default();
    };

    let inputs = usage_inputs(path);
    if let Ok(cache) = USAGE_CACHE.lock() {
        if let Some((stamps, usage)) = cache.get(path) {
            if *stamps == inputs {
                return usage.clone();
            }
        }
    }

    // A confinement refusal is otherwise invisible here: `read_at`'s Err is
    // swallowed below and the session just reports zero cost and zero tokens
    // forever. Name the path, so someone staring at a session that bills nothing
    // has a thread to pull. Past the cache lookup, so it can't loop.
    if !super::transcript::path_is_allowed(std::path::Path::new(path)) {
        tracing::warn!(
            transcript = %path,
            "transcript is outside every known transcript root — this session's usage will read as zero"
        );
    }

    // ONE shared dedup set (and peak-context high-water mark) across the main
    // transcript and every `subagents/*.jsonl` file: a sub-agent turn that
    // appears both inline (isSidechain) in the main file and in its own
    // sidechain file must be billed once, not twice — parity with the desktop
    // analyticsBackfill.recomputeSession which threads a single Set. That
    // shared state is why this memoizes the whole fold rather than per file.
    let mut usage = Usage::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut peak_context: u64 = 0;
    if let Ok(tx) = super::transcript::read_at(path) {
        fold_transcript(&tx, &mut usage, &mut seen, &mut peak_context, false);
    }
    // Every `subagents/*.jsonl` file is a sub-agent's own transcript — force
    // sidechain so its turns count toward cost but never move the session's
    // context gauge or reported model, even when the file's rows carry no
    // `isSidechain` key (parity with TS `recomputeSession`, which folds these
    // with `forceSidechain=true`).
    for (p, _, _) in inputs.iter().skip(1) {
        if let Ok(tx) = super::transcript::read_at(p) {
            fold_transcript(&tx, &mut usage, &mut seen, &mut peak_context, true);
        }
    }

    if let Ok(mut cache) = USAGE_CACHE.lock() {
        if cache.len() >= MAX_USAGE_CACHE && !cache.contains_key(path) {
            cache.clear();
        }
        cache.insert(path.to_string(), (inputs, usage.clone()));
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::transcript::TranscriptMessage;

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build a TranscriptMessage whose raw field looks like a real JSONL row.
    fn assistant_msg(
        id: &str,
        model: &str,
        input: u64,
        cache_write: u64,
        cache_read: u64,
        output: u64,
    ) -> TranscriptMessage {
        TranscriptMessage {
            role: "assistant".into(),
            content: Value::Null,
            raw: serde_json::json!({
                "type": "assistant",
                "message": {
                    "id": id,
                    "model": model,
                    "usage": {
                        "input_tokens": input,
                        "cache_creation_input_tokens": cache_write,
                        "cache_read_input_tokens": cache_read,
                        "output_tokens": output
                    }
                }
            }),
        }
    }

    fn user_msg() -> TranscriptMessage {
        TranscriptMessage {
            role: "user".into(),
            content: serde_json::json!("hi"),
            raw: serde_json::json!({"type": "user", "message": {"role": "user", "content": "hi"}}),
        }
    }

    fn tx(messages: Vec<TranscriptMessage>) -> Transcript {
        Transcript {
            path: None,
            messages,
            ..Default::default()
        }
    }

    // ── pricing pin tests (mirror modelUsage.test.ts) ────────────────────────

    /// opus (4.5+): $5/M in, $25/M out
    #[test]
    fn pricing_opus_in5_out25() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-opus-4-8",
            1_000_000,
            0,
            0,
            1_000_000,
        )]);
        let u = from_transcript(&t).unwrap();
        // 1M input * $5 + 1M output * $25 = $30
        assert!((u.cost_usd - 30.0).abs() < 1e-9, "opus cost={}", u.cost_usd);
    }

    /// fable: $10/M in, $50/M out
    #[test]
    fn pricing_fable_in10_out50() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-fable-5",
            1_000_000,
            0,
            0,
            1_000_000,
        )]);
        let u = from_transcript(&t).unwrap();
        // 1M input * $10 + 1M output * $50 = $60
        assert!(
            (u.cost_usd - 60.0).abs() < 1e-9,
            "fable cost={}",
            u.cost_usd
        );
    }

    /// legacy opus 4.1/4.0: $15/M in, $75/M out (longest-prefix wins)
    #[test]
    fn pricing_legacy_opus_in15_out75() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-opus-4-1-20250805",
            1_000_000,
            0,
            0,
            1_000_000,
        )]);
        let u = from_transcript(&t).unwrap();
        assert!(
            (u.cost_usd - 90.0).abs() < 1e-9,
            "legacy opus cost={}",
            u.cost_usd
        );
    }

    /// sonnet: $3/M in, $15/M out
    #[test]
    fn pricing_sonnet_in3_out15() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-sonnet-4-6",
            1_000_000,
            0,
            0,
            1_000_000,
        )]);
        let u = from_transcript(&t).unwrap();
        // 1M input * $3 + 1M output * $15 = $18
        assert!(
            (u.cost_usd - 18.0).abs() < 1e-9,
            "sonnet cost={}",
            u.cost_usd
        );
    }

    /// haiku: $1/M in, $5/M out
    #[test]
    fn pricing_haiku_in1_out5() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-haiku-3-5",
            1_000_000,
            0,
            0,
            1_000_000,
        )]);
        let u = from_transcript(&t).unwrap();
        // 1M input * $1 + 1M output * $5 = $6
        assert!((u.cost_usd - 6.0).abs() < 1e-9, "haiku cost={}", u.cost_usd);
    }

    /// default/unknown model → falls back to sonnet rates ($3/$15)
    #[test]
    fn pricing_unknown_model_uses_default_3_15() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-unknown-future",
            1_000_000,
            0,
            0,
            1_000_000,
        )]);
        let u = from_transcript(&t).unwrap();
        // default = $3/$15 → $18
        assert!(
            (u.cost_usd - 18.0).abs() < 1e-9,
            "default cost={}",
            u.cost_usd
        );
    }

    /// Writes with no TTL split: the documented fallback is the 1-hour rate.
    ///
    /// `assistant_msg` builds a usage block with no `cache_creation`, which is
    /// exactly the turn that cannot say which lifetime it bought. This test used
    /// to assert $3.75 (the 5-minute rate) because that rate was hardcoded; the
    /// fallback is now the dearer one, because a cost that reads lower than the
    /// bill is the failure this whole change exists to remove.
    #[test]
    fn pricing_cache_write_without_ttl_split_falls_back_to_1h() {
        // sonnet: input=$3/M, so a 2× write = $6/M.
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-sonnet-4-6",
            0,
            1_000_000,
            0,
            0,
        )]);
        let u = from_transcript(&t).unwrap();
        assert!(
            (u.cost_usd - 6.0).abs() < 1e-9,
            "cache-write cost={}",
            u.cost_usd
        );
    }

    /// cache-read multiplier: 0.1× input rate
    #[test]
    fn pricing_cache_read_0_1x() {
        // sonnet: input=$3/M, so cache-read = $0.30/M
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-sonnet-4-6",
            0,
            0,
            1_000_000,
            0,
        )]);
        let u = from_transcript(&t).unwrap();
        assert!(
            (u.cost_usd - 0.30).abs() < 1e-9,
            "cache-read cost={}",
            u.cost_usd
        );
    }

    /// A turn whose cache writes carry a TTL split, for the multiplier tests.
    fn assistant_msg_ttl(
        id: &str,
        model: &str,
        cache_write: u64,
        ephemeral_5m: Option<u64>,
        ephemeral_1h: Option<u64>,
    ) -> TranscriptMessage {
        let mut usage = serde_json::json!({
            "input_tokens": 0,
            "cache_creation_input_tokens": cache_write,
            "cache_read_input_tokens": 0,
            "output_tokens": 0
        });
        // Both absent ⇒ no `cache_creation` block at all. That absence is the
        // no-split case and must not be faked with zeros: a pair of zeros is a
        // turn that split its writes and wrote none, which is a different turn.
        if ephemeral_5m.is_some() || ephemeral_1h.is_some() {
            let mut cc = serde_json::Map::new();
            if let Some(v) = ephemeral_5m {
                cc.insert("ephemeral_5m_input_tokens".into(), v.into());
            }
            if let Some(v) = ephemeral_1h {
                cc.insert("ephemeral_1h_input_tokens".into(), v.into());
            }
            usage["cache_creation"] = Value::Object(cc);
        }
        TranscriptMessage {
            role: "assistant".into(),
            content: Value::Null,
            raw: serde_json::json!({
                "type": "assistant",
                "message": { "id": id, "model": model, "usage": usage }
            }),
        }
    }

    /// THE BUG. A 1-hour cache write bills at 2× the input rate, not 1.25×.
    /// This project's sessions are almost entirely 1-hour, so the hardcoded
    /// 1.25× understated the write component of every displayed cost.
    #[test]
    fn pricing_cache_write_1h_is_2x() {
        // opus: input=$5/M ⇒ 1-hour write = $10/M.
        let t = tx(vec![assistant_msg_ttl(
            "m1",
            "claude-opus-4-8",
            1_000_000,
            Some(0),
            Some(1_000_000),
        )]);
        let u = from_transcript(&t).unwrap();
        assert!(
            (u.cost_usd - 10.0).abs() < 1e-9,
            "1h cache-write cost={}",
            u.cost_usd
        );
    }

    /// A 5-minute cache write bills at 1.25x, the rate that used to be applied
    /// to every write regardless of lifetime.
    #[test]
    fn pricing_cache_write_5m_is_1_25x() {
        let t = tx(vec![assistant_msg_ttl(
            "m1",
            "claude-opus-4-8",
            1_000_000,
            Some(1_000_000),
            Some(0),
        )]);
        let u = from_transcript(&t).unwrap();
        assert!(
            (u.cost_usd - 6.25).abs() < 1e-9,
            "5m cache-write cost={}",
            u.cost_usd
        );
    }

    /// A turn carrying both lifetimes pays each portion at its own rate. No
    /// single blended multiplier reproduces this, which is why the split is read
    /// rather than a rate being chosen for the turn as a whole.
    #[test]
    fn pricing_cache_write_mixed_ttl_prices_each_portion() {
        let t = tx(vec![assistant_msg_ttl(
            "m1",
            "claude-opus-4-8",
            1_000_000,
            Some(400_000),
            Some(600_000),
        )]);
        let u = from_transcript(&t).unwrap();
        // 400k * $6.25/M + 600k * $10/M = $2.50 + $6.00.
        assert!(
            (u.cost_usd - 8.5).abs() < 1e-9,
            "mixed cache-write cost={}",
            u.cost_usd
        );
    }

    /// Writes the TTL split does not account for take the same 1-hour fallback
    /// as a turn with no split at all.
    #[test]
    fn pricing_cache_write_unattributed_remainder_falls_back_to_1h() {
        let t = tx(vec![assistant_msg_ttl(
            "m1",
            "claude-opus-4-8",
            1_000_000,
            Some(250_000),
            Some(250_000),
        )]);
        let u = from_transcript(&t).unwrap();
        // 250k * $6.25/M + 250k * $10/M + the unattributed 500k * $10/M.
        assert!(
            (u.cost_usd - 9.0625).abs() < 1e-9,
            "unattributed cache-write cost={}",
            u.cost_usd
        );
    }

    /// The cache-multiplier half of `contracts/model-pricing-cases.json`, which
    /// a TS test (modelPricingContract.test.ts) reads row for row. This is the
    /// drift guard between `turn_cost_usd` here and `turnCostUSD` there.
    #[test]
    fn matches_shared_cache_multiplier_contract() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Case {
            name: String,
            model: String,
            cache_write_tokens: u64,
            ephemeral5m: Option<u64>,
            ephemeral1h: Option<u64>,
            cache_read_tokens: Option<u64>,
            input_tokens: Option<u64>,
            output_tokens: Option<u64>,
            #[serde(rename = "expectedUSD")]
            expected_usd: f64,
            #[allow(dead_code)]
            note: String,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Contract {
            cache_multiplier_cases: Vec<Case>,
        }

        const FIXTURE: &str = include_str!("../../../../contracts/model-pricing-cases.json");
        let contract: Contract =
            serde_json::from_str(FIXTURE).expect("model-pricing-cases.json parses");
        assert!(
            contract.cache_multiplier_cases.len() >= 9,
            "cacheMultiplierCases lost rows: {}",
            contract.cache_multiplier_cases.len()
        );
        for case in &contract.cache_multiplier_cases {
            let mut usage = serde_json::json!({
                "input_tokens": case.input_tokens.unwrap_or(0),
                "cache_creation_input_tokens": case.cache_write_tokens,
                "cache_read_input_tokens": case.cache_read_tokens.unwrap_or(0),
                "output_tokens": case.output_tokens.unwrap_or(0),
            });
            if case.ephemeral5m.is_some() || case.ephemeral1h.is_some() {
                let mut cc = serde_json::Map::new();
                if let Some(v) = case.ephemeral5m {
                    cc.insert("ephemeral_5m_input_tokens".into(), v.into());
                }
                if let Some(v) = case.ephemeral1h {
                    cc.insert("ephemeral_1h_input_tokens".into(), v.into());
                }
                usage["cache_creation"] = Value::Object(cc);
            }
            let got = turn_cost_usd(Some(&case.model), &usage);
            assert!(
                (got - case.expected_usd).abs() < 1e-9,
                "{}: expected ${}, got ${got}",
                case.name,
                case.expected_usd
            );
        }
    }

    /// The three prompt tiers survive the fold as distinct numbers, and a
    /// provider that reports none of them yields `None` rather than three zeros.
    #[test]
    fn cache_split_is_carried_and_absent_when_unreported() {
        let t = tx(vec![
            assistant_msg("m1", "claude-opus-4-8", 10, 400, 90, 5),
            assistant_msg("m2", "claude-opus-4-8", 2, 100, 900, 7),
        ]);
        let c = from_transcript(&t).unwrap().cache.expect("cache split");
        assert_eq!(c.fresh, 12);
        assert_eq!(c.write, 500);
        assert_eq!(c.read, 990);

        // A turn with no cache fields at all reports nothing, not zero.
        let bare = TranscriptMessage {
            role: "assistant".into(),
            content: Value::Null,
            raw: serde_json::json!({
                "type": "assistant",
                "message": {
                    "id": "m1",
                    "model": "claude-opus-4-8",
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }
            }),
        };
        assert!(from_transcript(&tx(vec![bare])).unwrap().cache.is_none());
    }

    /// 200k→1M context window heuristic: if context_tokens > 200_000, limit = 1_000_000
    #[test]
    fn context_window_200k_to_1m_promotion() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-opus-4-8",
            250_000,
            0,
            0,
            10,
        )]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(u.context_tokens, 250_000);
        assert_eq!(u.context_limit, 1_000_000, "should promote to 1M");
    }

    /// Fable/Mythos are 1M-native — the limit must be 1M from the first turn,
    /// not only after the observed context crosses the 200K promotion gate.
    #[test]
    fn context_window_fable_is_1m_native() {
        let t = tx(vec![assistant_msg("m1", "claude-fable-5", 1_000, 0, 0, 10)]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(u.context_limit, 1_000_000);
    }

    /// The regression: a `opus[1m]` session under 200k reported a 200k window,
    /// so every context gauge (and every `list_agents` row) read ~5× too full
    /// until the session actually crossed 200k. The spawn request carries the
    /// `[1m]` marker the transcript's model id drops — resolve from it.
    #[test]
    fn resolve_window_reads_1m_off_the_requested_model() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-opus-5",
            190_000,
            0,
            0,
            10,
        )]);
        let mut u = from_transcript(&t).unwrap();
        assert_eq!(u.context_limit, 200_000, "transcript alone cannot tell");
        u.resolve_window(None, Some("opus[1m]"));
        assert_eq!(u.context_limit, 1_000_000);
    }

    /// The provider's own window is a fact and outranks every inference —
    /// including the retrospective 200k→1M promotion.
    #[test]
    fn resolve_window_prefers_the_reported_window() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-opus-5",
            300_000,
            0,
            0,
            10,
        )]);
        let mut u = from_transcript(&t).unwrap();
        assert_eq!(u.context_limit, 1_000_000, "promoted by the fallback");
        u.resolve_window(Some(200_000), Some("opus[1m]"));
        assert_eq!(u.context_limit, 200_000);
    }

    /// The fix must not default everything to 1M: an unmarked request says
    /// nothing about the window, and a genuinely-200k session must keep
    /// reporting 200k or a real approaching limit would be hidden.
    #[test]
    fn resolve_window_leaves_a_200k_session_alone() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-opus-5",
            190_000,
            0,
            0,
            10,
        )]);
        let mut u = from_transcript(&t).unwrap();
        u.resolve_window(None, Some("opus"));
        assert_eq!(u.context_limit, 200_000);
        // A zero/absent reported window is "unknown", not "no window".
        u.resolve_window(Some(0), None);
        assert_eq!(u.context_limit, 200_000);
    }

    /// A coarse requested alias may raise the window, never lower one the
    /// rates table (or a user override) already resolved higher.
    #[test]
    fn resolve_window_never_lowers_a_1m_native_model() {
        let t = tx(vec![assistant_msg("m1", "claude-fable-5", 1_000, 0, 0, 10)]);
        let mut u = from_transcript(&t).unwrap();
        u.resolve_window(None, Some("opus"));
        assert_eq!(u.context_limit, 1_000_000);
    }

    /// At exactly 200_000 tokens the limit stays 200_000.
    #[test]
    fn context_window_at_200k_stays_200k() {
        let t = tx(vec![assistant_msg(
            "m1",
            "claude-sonnet-4-6",
            200_000,
            0,
            0,
            0,
        )]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(u.context_limit, 200_000);
    }

    /// Claude's `<synthetic>` placeholder model must be neutralized (parity with
    /// analyticsBackfill.ts `if (rowModel?.startsWith('<')) rowModel = null`).
    #[test]
    fn synthetic_placeholder_model_is_neutralized() {
        let t = tx(vec![
            assistant_msg("m1", "claude-opus-4-8", 1_000, 0, 0, 500),
            assistant_msg("m2", "<synthetic>", 0, 0, 0, 2_000),
        ]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(
            u.model.as_deref(),
            Some("claude-opus-4-8"),
            "synthetic row must not clobber the reported model"
        );
        // opus: $5/M in, $25/M out. m1 = 1000*5 + 500*25; synthetic m2 = 2000 out at opus $25/M.
        let expected = (1_000.0 * 5.0 + 500.0 * 25.0 + 2_000.0 * 25.0) / 1_000_000.0;
        assert!(
            (u.cost_usd - expected).abs() < 1e-12,
            "cost={} expected={} (synthetic turn should bill at opus $25/M, not sonnet $15/M)",
            u.cost_usd,
            expected
        );
    }

    /// ...and it must not zero the CONTEXT GAUGE either. The CLI answers a
    /// resume kickoff with a local `<synthetic>` row carrying all-zero usage;
    /// folding that in reported a healthy 96k-token session at 0 tokens on a
    /// `<synthetic>` model, which reads exactly like a wedged worker. A
    /// placeholder's zeros are not an observation of the window.
    #[test]
    fn synthetic_placeholder_does_not_zero_the_context_gauge() {
        let t = tx(vec![
            assistant_msg("m1", "claude-opus-4-8", 90_000, 0, 6_683, 500),
            assistant_msg("m2", "<synthetic>", 0, 0, 0, 0),
        ]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(
            u.context_tokens, 96_683,
            "a placeholder row must leave the last real turn's context standing"
        );
        assert_eq!(u.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(u.context_limit, 200_000);
    }

    /// 1M-mode is a session-level property: once any main-thread turn exceeds the
    /// 200k window the limit must stay promoted even when a later turn's context
    /// (e.g. after auto-compaction) falls back under 200k.
    #[test]
    fn context_window_1m_promotion_survives_compaction() {
        let t = tx(vec![
            assistant_msg("m1", "claude-opus-4-8", 250_000, 0, 0, 10),
            user_msg(),
            assistant_msg("m2", "claude-opus-4-8", 180_000, 0, 0, 10),
        ]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(u.context_tokens, 180_000, "gauge reflects latest turn");
        assert_eq!(
            u.context_limit, 1_000_000,
            "1M promotion must persist past compaction (session peak), not revert to 200k"
        );
    }

    /// per-message-id dedup: duplicate id must be counted only once
    #[test]
    fn per_message_id_dedup() {
        let t = tx(vec![
            user_msg(),
            assistant_msg("m1", "claude-sonnet-4-6", 100, 0, 1_000, 50),
            assistant_msg("m1", "claude-sonnet-4-6", 100, 0, 1_000, 50), // dup → skip
            assistant_msg("m2", "claude-sonnet-4-6", 200, 0, 5_000, 80),
        ]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-4-6"));
        // context = last turn's input + cache_read
        assert_eq!(u.context_tokens, 200 + 5_000);
        assert_eq!(u.context_limit, 200_000);
        // cost from m1 (once) + m2, sonnet rates (3 in / 15 out per Mtok, cache-read 0.1×)
        let expected = ((100.0 * 3.0 + 1_000.0 * 0.3 + 50.0 * 15.0)
            + (200.0 * 3.0 + 5_000.0 * 0.3 + 80.0 * 15.0))
            / 1_000_000.0;
        assert!(
            (u.cost_usd - expected).abs() < 1e-12,
            "cost={} expected={}",
            u.cost_usd,
            expected
        );
    }

    /// Streamed blocks of one logical message that carry a row-level `uuid`
    /// but NO `message.id` must be deduped by the row uuid — matching the TS
    /// backfill (`msg.id || row.uuid`). Otherwise the daemon counts the cost
    /// on every occurrence and diverges from the desktop path.
    #[test]
    fn dedup_falls_back_to_row_uuid_when_message_id_absent() {
        fn assistant_no_id(uuid: &str, model: &str, input: u64, output: u64) -> TranscriptMessage {
            TranscriptMessage {
                role: "assistant".into(),
                content: Value::Null,
                raw: serde_json::json!({
                    "type": "assistant",
                    "uuid": uuid,
                    "message": {
                        // no "id" field
                        "model": model,
                        "usage": {
                            "input_tokens": input,
                            "cache_creation_input_tokens": 0,
                            "cache_read_input_tokens": 0,
                            "output_tokens": output
                        }
                    }
                }),
            }
        }

        // Two streamed blocks of the SAME message (same row uuid, no message.id).
        let t = tx(vec![
            assistant_no_id("row-uuid-1", "claude-opus-4-8", 1_000_000, 1_000_000),
            assistant_no_id("row-uuid-1", "claude-opus-4-8", 1_000_000, 1_000_000),
        ]);
        let u = from_transcript(&t).unwrap();
        // Counted ONCE: 1M in * $5 + 1M out * $25 = $30 (not $60).
        assert!(
            (u.cost_usd - 30.0).abs() < 1e-9,
            "expected row-uuid dedup => $30, got {}",
            u.cost_usd
        );
    }

    /// Sub-agent (isSidechain) turns: cost counts at the sub-agent's own model
    /// rates, but context/model stay pinned to the main thread's last turn.
    #[test]
    fn sidechain_cost_counts_but_context_stays_main_thread() {
        let mut side = assistant_msg("sub1", "claude-haiku-4-5", 1_000_000, 0, 0, 1_000_000);
        side.raw["isSidechain"] = serde_json::json!(true);
        let t = tx(vec![
            assistant_msg("m1", "claude-fable-5", 1_000, 0, 0, 500),
            side, // runs after the main turn — must not clobber context/model
        ]);
        let u = from_transcript(&t).unwrap();
        assert_eq!(u.model.as_deref(), Some("claude-fable-5"));
        assert_eq!(u.context_tokens, 1_000, "sidechain must not move the gauge");
        // main fable turn (1k in, 500 out) + sidechain haiku turn (1M in, 1M out at $1/$5)
        let expected = (1_000.0 * 10.0 + 500.0 * 50.0) / 1_000_000.0 + (1.0 + 5.0);
        assert!(
            (u.cost_usd - expected).abs() < 1e-9,
            "cost={} expected={}",
            u.cost_usd,
            expected
        );
    }

    /// No assistant messages → None
    #[test]
    fn none_without_assistant_usage() {
        let t = tx(vec![user_msg()]);
        assert!(from_transcript(&t).is_none());
    }

    /// Empty transcript → None
    #[test]
    fn none_for_empty_transcript() {
        let t = tx(vec![]);
        assert!(from_transcript(&t).is_none());
    }

    /// usage_for_path with None → zeroed Usage
    #[test]
    fn usage_for_path_none_returns_default() {
        let u = usage_for_path(None);
        assert_eq!(u.context_tokens, 0);
        assert_eq!(u.cost_usd, 0.0);
        assert!(u.model.is_none());
    }

    /// usage_for_path with non-existent path → zeroed Usage (no panic)
    #[test]
    fn usage_for_path_missing_file_returns_default() {
        let u = usage_for_path(Some("/nonexistent/path/session.jsonl"));
        assert_eq!(u.context_tokens, 0);
        assert_eq!(u.cost_usd, 0.0);
    }

    /// idx 19: a sub-agent assistant turn that appears BOTH inline in the main
    /// transcript (isSidechain) and in a `subagents/*.jsonl` file must be billed
    /// once — the main fold and each subagent fold must share ONE dedup set
    /// (parity with desktop analyticsBackfill.recomputeSession). Regression: the
    /// daemon used a fresh `seen` per file and double-counted the echoed turn.
    #[test]
    fn subagent_turn_echoed_into_main_is_not_double_counted() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "wks-usage-dedup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // `read_at` reads only inside known transcript roots (see transcript.rs).
        super::super::transcript::allow_root(&dir);

        // One assistant row: opus, 1M output → $25. isSidechain so it never
        // touches the context gauge.
        let row = serde_json::json!({
            "type": "assistant",
            "isSidechain": true,
            "message": {
                "id": "sub-abc",
                "model": "claude-opus-4-8",
                "usage": {
                    "input_tokens": 0,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "output_tokens": 1_000_000
                }
            }
        });
        let line = serde_json::to_string(&row).unwrap();

        // The SAME row id lives inline in the main transcript AND in a subagent
        // sidechain file next to it (`<stem>/subagents/agent1.jsonl`).
        let main_path = dir.join("session.jsonl");
        std::fs::write(&main_path, format!("{line}\n")).unwrap();
        let sub_dir = dir.join("session").join("subagents");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(sub_dir.join("agent1.jsonl"), format!("{line}\n")).unwrap();

        let u = usage_for_path(Some(main_path.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);

        // Shared dedup ⇒ counted ONCE ($25), not twice ($50).
        assert!(
            (u.cost_usd - 25.0).abs() < 1e-9,
            "subagent turn echoed into main double-counted: cost={} (expected $25)",
            u.cost_usd
        );
    }

    /// A `subagents/*.jsonl` file is a sub-agent's OWN transcript and its rows
    /// carry no `isSidechain` key (the sub-agent doesn't know it is a side
    /// thread of a parent). Folding such a file must still keep those turns off
    /// the session's context gauge and reported model — otherwise the last
    /// sub-agent turn clobbers both. Parity with the TS twin
    /// `analyticsBackfill.recomputeSession`, which folds every subagent file
    /// with `forceSidechain=true`. Regression: the Rust fold honored the missing
    /// per-row flag as `false` and treated the subagent turn as main-thread.
    #[test]
    fn subagent_file_without_sidechain_flag_does_not_clobber_context() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "wks-usage-subforce-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        super::super::transcript::allow_root(&dir);

        // Main transcript: sonnet (200k-native), small context.
        let main_row = serde_json::json!({
            "type": "assistant",
            "message": {
                "id": "main-1",
                "model": "claude-sonnet-4-6",
                "usage": {
                    "input_tokens": 1_000,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "output_tokens": 10
                }
            }
        });
        // Subagent's OWN file — big context, different model, NO isSidechain key.
        let sub_row = serde_json::json!({
            "type": "assistant",
            "message": {
                "id": "sub-1",
                "model": "claude-haiku-4-5",
                "usage": {
                    "input_tokens": 500_000,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "output_tokens": 10
                }
            }
        });

        let main_path = dir.join("session.jsonl");
        std::fs::write(
            &main_path,
            format!("{}\n", serde_json::to_string(&main_row).unwrap()),
        )
        .unwrap();
        let sub_dir = dir.join("session").join("subagents");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(
            sub_dir.join("agent1.jsonl"),
            format!("{}\n", serde_json::to_string(&sub_row).unwrap()),
        )
        .unwrap();

        let u = usage_for_path(Some(main_path.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            u.context_tokens, 1_000,
            "subagent file turn clobbered the context gauge (expected main-thread 1000)"
        );
        assert_eq!(
            u.model.as_deref(),
            Some("claude-sonnet-4-6"),
            "subagent file turn clobbered the reported model"
        );
        assert_eq!(
            u.context_limit, 200_000,
            "subagent file's 500k context wrongly promoted the window to 1M"
        );
    }

    // ── value-based API (same shape as wks-tui) ──────────────────────────────

    fn assistant_value(id: &str, model: &str, input: u64, cache_read: u64, output: u64) -> Value {
        serde_json::json!({
            "role": "assistant",
            "raw": {"message": {
                "id": id, "model": model,
                "usage": {"input_tokens": input, "cache_read_input_tokens": cache_read,
                          "output_tokens": output}
            }}
        })
    }

    #[test]
    fn value_api_folds_usage_from_transcript() {
        let tx = serde_json::json!({"messages": [
            {"role": "user", "content": "hi"},
            assistant_value("m1", "claude-sonnet-4-6", 100, 1000, 50),
            assistant_value("m1", "claude-sonnet-4-6", 100, 1000, 50), // dup id — cost once
            assistant_value("m2", "claude-sonnet-4-6", 200, 5000, 80),
        ]});
        let u = from_transcript_value(&tx).unwrap();
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(u.context_tokens, 200 + 5000);
        assert_eq!(u.context_limit, 200_000);
        let expected = ((100.0 * 3.0 + 1000.0 * 0.3 + 50.0 * 15.0)
            + (200.0 * 3.0 + 5000.0 * 0.3 + 80.0 * 15.0))
            / 1_000_000.0;
        assert!((u.cost_usd - expected).abs() < 1e-12);
    }

    #[test]
    fn value_api_promotes_to_1m_window() {
        let tx = serde_json::json!({"messages": [
            assistant_value("m1", "claude-opus-4-8", 250_000, 0, 10),
        ]});
        let u = from_transcript_value(&tx).unwrap();
        assert_eq!(u.context_limit, 1_000_000);
    }

    #[test]
    fn value_api_ignores_synthetic_placeholder_rows() {
        let tx = serde_json::json!({"messages": [
            assistant_value("m1", "claude-sonnet-4-6", 200, 5_000, 80),
            assistant_value("m2", "<synthetic>", 0, 0, 0),
        ]});
        let u = from_transcript_value(&tx).unwrap();
        assert_eq!(u.context_tokens, 200 + 5_000);
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn value_api_none_without_assistant_usage() {
        let tx = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        assert!(from_transcript_value(&tx).is_none());
    }

    // ── memoization ─────────────────────────────────────────────────────────
    //
    // `GET /sessions` re-derives usage for EVERY session, and the desktop polls
    // that list every 60s plus a four-fetch burst after each terminate. Without
    // a memo a daemon with ~10 hook-bound sessions re-read and fully re-parsed
    // tens of megabytes per poll, for data that had not changed.

    /// One assistant row, as it appears in a real transcript JSONL. Cost is
    /// deduped by message id, so distinct turns need distinct ids.
    fn jsonl_row(id: &str, output_tokens: u64) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "id": id,
                "model": "claude-opus-4-8",
                "usage": {
                    "input_tokens": 1000,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "output_tokens": output_tokens
                }
            }
        })
        .to_string()
    }

    /// A temp path unique to this test binary + line, so tests don't collide.
    /// `read_at` only reads inside known transcript roots, so the fixture dir is
    /// registered as one — the same call a profile spawn makes for its own
    /// `CLAUDE_CONFIG_DIR`.
    fn temp_transcript(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("claudemon-usage-test-{tag}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        super::super::transcript::allow_root(&dir);
        dir.join("session.jsonl")
    }

    #[test]
    fn an_unchanged_transcript_is_not_re_parsed() {
        let path = temp_transcript("unchanged");
        // Both rows are the same length, so only the *content* differs — which
        // means a re-parse would be visible in the result while an unchanged
        // (len, mtime) stamp would not.
        std::fs::write(&path, format!("{}\n", jsonl_row("m1", 500))).expect("write");
        let p = path.to_str().unwrap();

        let first = usage_for_path(Some(p));
        assert!(first.cost_usd > 0.0, "baseline usage was computed");

        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, format!("{}\n", jsonl_row("m1", 999))).expect("rewrite");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .expect("restore mtime");

        let second = usage_for_path(Some(p));
        assert_eq!(
            second.cost_usd, first.cost_usd,
            "same (len, mtime) => memo hit, no re-read"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_changed_transcript_is_re_parsed() {
        let path = temp_transcript("changed");
        std::fs::write(&path, format!("{}\n", jsonl_row("m1", 500))).expect("write");
        let p = path.to_str().unwrap();
        let first = usage_for_path(Some(p));

        // Appending changes the length, so the stamp differs regardless of the
        // filesystem's mtime granularity.
        std::fs::write(
            &path,
            format!("{}\n{}\n", jsonl_row("m1", 500), jsonl_row("m2", 500)),
        )
        .expect("append");
        let second = usage_for_path(Some(p));
        assert!(
            second.cost_usd > first.cost_usd,
            "a grown transcript re-folds: {} then {}",
            first.cost_usd,
            second.cost_usd
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_truncated_transcript_is_re_parsed() {
        // A length-only check would miss a rewrite that shrinks the file.
        let path = temp_transcript("truncated");
        std::fs::write(
            &path,
            format!("{}\n{}\n", jsonl_row("m1", 500), jsonl_row("m2", 500)),
        )
        .expect("write");
        let p = path.to_str().unwrap();
        let first = usage_for_path(Some(p));

        std::fs::write(&path, format!("{}\n", jsonl_row("m1", 500))).expect("truncate");
        let second = usage_for_path(Some(p));
        assert!(
            second.cost_usd < first.cost_usd,
            "shrink invalidates the memo"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_transcript_stays_empty() {
        let usage = usage_for_path(Some("/nonexistent/does-not-exist.jsonl"));
        assert_eq!(usage.cost_usd, 0.0);
        assert_eq!(usage_for_path(None).cost_usd, 0.0);
    }
}

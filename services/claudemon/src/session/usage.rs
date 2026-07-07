//! Per-session token/cost/context tracking, ported verbatim from the wks-tui
//! crate's `usage.rs` (which itself mirrors the Electron app's `modelUsage.ts`
//! + `claudeSessionStore.applyUsage`).
//!
//! Every assistant message in Claude Code's JSONL transcript carries a `usage`
//! block and a `model` id. We fold each assistant turn to produce cumulative
//! cost and a point-in-time view of context fullness.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::transcript::Transcript;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub model: Option<String>,
    /// Latest turn's input side — a point-in-time view of context fullness.
    pub context_tokens: u64,
    pub context_limit: u64,
    /// Cumulative cost over the session.
    pub cost_usd: f64,
}

struct Rates {
    /// USD per million input tokens. cache-write = 1.25×, cache-read = 0.1×.
    input: f64,
    output: f64,
    context_limit: u64,
}

const DEFAULT_RATES: Rates = Rates {
    input: 3.0,
    output: 15.0,
    context_limit: 200_000,
};

/// Longest-prefix match on the transcript `model` id (mirrors modelUsage.ts).
/// Current list pricing (2026-06): Fable $10/$50, Opus 4.5+ $5/$25,
/// Sonnet $3/$15, Haiku $1/$5. Opus 4.1 and older kept the $15/$75 rates.
fn rates_for(model: Option<&str>) -> Rates {
    let Some(model) = model else {
        return DEFAULT_RATES;
    };
    // (prefix, input, output)
    const TABLE: [(&str, f64, f64); 7] = [
        ("claude-fable", 10.0, 50.0),
        ("claude-mythos", 10.0, 50.0),
        ("claude-opus", 5.0, 25.0),
        ("claude-opus-4-1", 15.0, 75.0),
        ("claude-opus-4-0", 15.0, 75.0),
        ("claude-sonnet", 3.0, 15.0),
        ("claude-haiku", 1.0, 5.0),
    ];
    let mut best: Option<Rates> = None;
    let mut best_len = 0usize;
    for (prefix, input, output) in TABLE {
        if model.starts_with(prefix) && prefix.len() > best_len {
            best = Some(Rates {
                input,
                output,
                context_limit: 200_000,
            });
            best_len = prefix.len();
        }
    }
    best.unwrap_or(DEFAULT_RATES)
}

/// Tokens occupying the window this turn: input + both cache tiers.
fn context_tokens_of(usage: &Value) -> u64 {
    let n = |k: &str| {
        let v = usage.get(k);
        match v {
            None => 0,
            Some(val) => match val.as_u64() {
                Some(n) => n,
                None => {
                    tracing::warn!(field = k, raw = %val, "usage field present but not a valid u64; treating as 0");
                    0
                }
            },
        }
    };
    n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens")
}

/// The transcript model id lacks the `[1m]` suffix, so we infer the 1M window
/// once a turn's context exceeds the standard 200k.
fn context_limit_for(model: Option<&str>, observed: u64) -> u64 {
    let base = rates_for(model).context_limit;
    if base <= 200_000 && observed > 200_000 {
        1_000_000
    } else {
        base
    }
}

/// USD cost of one turn. Cache writes cost 1.25× input, reads 0.1×.
fn turn_cost_usd(model: Option<&str>, usage: &Value) -> f64 {
    let r = rates_for(model);
    let n = |k: &str| {
        let v = usage.get(k);
        match v {
            None => 0_f64,
            Some(val) => match val.as_u64() {
                Some(n) => n as f64,
                None => {
                    tracing::warn!(field = k, raw = %val, "usage field present but not a valid u64; treating as 0");
                    0_f64
                }
            },
        }
    };
    let dollars = n("input_tokens") * r.input
        + n("cache_creation_input_tokens") * (r.input * 1.25)
        + n("cache_read_input_tokens") * (r.input * 0.1)
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
        let row_model = msg.get("model").and_then(|m| m.as_str());

        // Point-in-time context/model: main thread only (see from_transcript).
        if !sidechain {
            usage.context_tokens = context_tokens_of(u);
            if let Some(model) = row_model {
                usage.model = Some(model.to_string());
            }
            usage.context_limit = context_limit_for(usage.model.as_deref(), usage.context_tokens);
        }

        // Cumulative cost — once per distinct message id.
        let id = msg.get("id").and_then(|i| i.as_str()).unwrap_or("");
        if !id.is_empty() && !seen.insert(id.to_string()) {
            continue;
        }
        usage.cost_usd += turn_cost_usd(row_model.or(usage.model.as_deref()), u);
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

        let sidechain = m
            .raw
            .get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let row_model = msg.get("model").and_then(|m| m.as_str());

        // Point-in-time context/model: main thread only — a sub-agent's turn
        // must not clobber the session's context gauge or reported model.
        if !sidechain {
            usage.context_tokens = context_tokens_of(u);
            if let Some(model) = row_model {
                usage.model = Some(model.to_string());
            }
            usage.context_limit = context_limit_for(usage.model.as_deref(), usage.context_tokens);
        }

        // Cumulative cost — once per distinct message id, at the row's own
        // model rates (sub-agents often run a different model).
        let id = msg.get("id").and_then(|i| i.as_str()).unwrap_or("");
        if !id.is_empty() && !seen.insert(id.to_string()) {
            continue;
        }
        usage.cost_usd += turn_cost_usd(row_model.or(usage.model.as_deref()), u);
    }

    any.then_some(usage)
}

/// Compute usage for a session given its `transcript_path`. Returns a zeroed
/// default if the path is `None`, the file doesn't exist, or contains no
/// assistant usage blocks.
///
/// Sub-agent transcripts (`<transcript-stem>/subagents/*.jsonl` — where
/// current Claude Code writes Task/teammate agents) fold in as cost/spend
/// only: their rows are `isSidechain`, so [`from_transcript`] keeps them off
/// the context gauge automatically.
pub fn usage_for_path(transcript_path: Option<&str>) -> Usage {
    let Some(path) = transcript_path else {
        return Usage::default();
    };
    let mut usage = match super::transcript::read_at(path) {
        Ok(tx) => from_transcript(&tx).unwrap_or_default(),
        Err(_) => Usage::default(),
    };
    if let Some(stem) = path.strip_suffix(".jsonl") {
        if let Ok(rd) = std::fs::read_dir(format!("{stem}/subagents")) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let Some(p) = p.to_str() else { continue };
                if let Ok(tx) = super::transcript::read_at(p) {
                    if let Some(side) = from_transcript(&tx) {
                        usage.cost_usd += side.cost_usd;
                    }
                }
            }
        }
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

    /// cache-write multiplier: 1.25× input rate
    #[test]
    fn pricing_cache_write_1_25x() {
        // sonnet: input=$3/M, so cache-write = $3.75/M
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
            (u.cost_usd - 3.75).abs() < 1e-9,
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
    fn value_api_none_without_assistant_usage() {
        let tx = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        assert!(from_transcript_value(&tx).is_none());
    }
}

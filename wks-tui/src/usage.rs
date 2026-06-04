//! Per-agent token/cost/context tracking, ported from the Electron app's
//! `modelUsage.ts` + `claudeSessionStore.applyUsage`. claudemon's `/sessions`
//! doesn't carry usage — the desktop app derives it from the transcript JSONL,
//! where every assistant message has a `usage` block and a `model` id. We do
//! the same: parse the transcript and fold each assistant turn in.

use std::collections::HashSet;

use serde_json::Value;

#[derive(Debug, Clone, Default)]
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

const DEFAULT_RATES: Rates = Rates { input: 3.0, output: 15.0, context_limit: 200_000 };

/// Longest-prefix match on the transcript `model` id (mirrors modelUsage.ts).
fn rates_for(model: Option<&str>) -> Rates {
    let Some(model) = model else { return DEFAULT_RATES };
    // (prefix, input, output)
    const TABLE: [(&str, f64, f64); 3] = [
        ("claude-opus", 15.0, 75.0),
        ("claude-sonnet", 3.0, 15.0),
        ("claude-haiku", 1.0, 5.0),
    ];
    let mut best: Option<Rates> = None;
    let mut best_len = 0usize;
    for (prefix, input, output) in TABLE {
        if model.starts_with(prefix) && prefix.len() > best_len {
            best = Some(Rates { input, output, context_limit: 200_000 });
            best_len = prefix.len();
        }
    }
    best.unwrap_or(DEFAULT_RATES)
}

/// Tokens occupying the window this turn: input + both cache tiers.
fn context_tokens_of(usage: &Value) -> u64 {
    let n = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
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
    let n = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as f64;
    let dollars = n("input_tokens") * r.input
        + n("cache_creation_input_tokens") * (r.input * 1.25)
        + n("cache_read_input_tokens") * (r.input * 0.1)
        + n("output_tokens") * r.output;
    dollars / 1_000_000.0
}

/// Fold a session's transcript into a [`Usage`]. Context/model come from the
/// last assistant turn; cost accumulates, deduped by message id so streamed
/// blocks of one message aren't double-counted. Returns `None` if no assistant
/// usage was found.
pub fn from_transcript(tx: &Value) -> Option<Usage> {
    let messages = tx.get("messages")?.as_array()?;
    let mut usage = Usage::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut any = false;

    for m in messages {
        if m.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let Some(msg) = m.get("raw").and_then(|r| r.get("message")) else { continue };
        let Some(u) = msg.get("usage") else { continue };
        any = true;

        // Point-in-time: overwrite with the latest turn.
        usage.context_tokens = context_tokens_of(u);
        if let Some(model) = msg.get("model").and_then(|m| m.as_str()) {
            usage.model = Some(model.to_string());
        }
        usage.context_limit = context_limit_for(usage.model.as_deref(), usage.context_tokens);

        // Cumulative cost — once per distinct message id.
        let id = msg.get("id").and_then(|i| i.as_str()).unwrap_or("");
        if !id.is_empty() && !seen.insert(id.to_string()) {
            continue;
        }
        usage.cost_usd += turn_cost_usd(usage.model.as_deref(), u);
    }

    any.then_some(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(id: &str, model: &str, input: u64, cache_read: u64, output: u64) -> Value {
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
    fn folds_usage_from_transcript() {
        let tx = serde_json::json!({"messages": [
            {"role": "user", "content": "hi"},
            assistant("m1", "claude-sonnet-4-6", 100, 1000, 50),
            assistant("m1", "claude-sonnet-4-6", 100, 1000, 50), // dup id — cost once
            assistant("m2", "claude-sonnet-4-6", 200, 5000, 80),
        ]});
        let u = from_transcript(&tx).unwrap();
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-4-6"));
        // context = last turn's input + cache.
        assert_eq!(u.context_tokens, 200 + 5000);
        assert_eq!(u.context_limit, 200_000);
        // cost from m1 (once) + m2, sonnet rates (3 in / 15 out per Mtok, cache-read 0.1×).
        let expected = ((100.0 * 3.0 + 1000.0 * 0.3 + 50.0 * 15.0)
            + (200.0 * 3.0 + 5000.0 * 0.3 + 80.0 * 15.0))
            / 1_000_000.0;
        assert!((u.cost_usd - expected).abs() < 1e-12);
    }

    #[test]
    fn promotes_to_1m_window() {
        let tx = serde_json::json!({"messages": [
            assistant("m1", "claude-opus-4-8", 250_000, 0, 10),
        ]});
        let u = from_transcript(&tx).unwrap();
        assert_eq!(u.context_limit, 1_000_000);
    }

    #[test]
    fn none_without_assistant_usage() {
        let tx = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        assert!(from_transcript(&tx).is_none());
    }

    /// Validate against a real Claude transcript JSONL, wrapped the way
    /// claudemon's `/transcript` endpoint does (one message per line, with the
    /// full line object under `raw`). Run with:
    ///   WKS_TRANSCRIPT=/path/to/session.jsonl cargo test real_transcript -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_transcript() {
        let path = std::env::var("WKS_TRANSCRIPT").expect("set WKS_TRANSCRIPT");
        let text = std::fs::read_to_string(&path).unwrap();
        let messages: Vec<Value> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|entry| {
                let msg = entry.get("message")?;
                let role = msg.get("role")?.as_str()?.to_string();
                Some(serde_json::json!({
                    "role": role,
                    "content": msg.get("content").cloned().unwrap_or(Value::Null),
                    "raw": entry,
                }))
            })
            .collect();
        let tx = serde_json::json!({ "messages": messages });
        let u = from_transcript(&tx).expect("usage from real transcript");
        eprintln!(
            "model={:?} context={}/{} ({:.0}%) cost=${:.2}",
            u.model,
            u.context_tokens,
            u.context_limit,
            u.context_tokens as f64 / u.context_limit as f64 * 100.0,
            u.cost_usd
        );
        assert!(u.model.is_some());
        assert!(u.context_tokens > 0);
    }
}

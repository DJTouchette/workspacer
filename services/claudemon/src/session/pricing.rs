//! Per-model pricing: one built-in longest-prefix table for every provider,
//! plus user overrides from `~/.workspacer/model-rates.json`.
//!
//! The built-ins are list prices (USD per million tokens). They WILL drift as
//! vendors reprice — that's what the overrides file is for: any JSON object of
//! `{ "<model-prefix>": { "input": _, "output": _, "cached_input": _,
//! "context_limit": _ } }` participates in the same longest-prefix match and
//! beats a built-in of equal length. The file is re-read when its mtime
//! changes, so edits apply without restarting the daemon.
//!
//! Two consumers:
//!  - `usage.rs` (Claude JSONL costing) reads input/output/context_limit and
//!    keeps its own cache-tier multipliers for the fields Claude itemizes;
//!  - `UsageAcc::status_line` (managed providers) calls [`estimate_cost`] when
//!    an adapter reports cumulative tokens but no native cost figure (Codex —
//!    OpenAI's wire carries no dollars).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use directories::BaseDirs;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ModelRates {
    /// USD per million input tokens.
    pub input: f64,
    /// USD per million output tokens.
    pub output: f64,
    /// USD per million cache-read input tokens. Absent → provider default
    /// (Claude bills reads at 0.1× input; OpenAI entries set it explicitly).
    #[serde(default)]
    pub cached_input: Option<f64>,
    #[serde(default)]
    pub context_limit: Option<u64>,
}

const fn rates(input: f64, output: f64, cached_input: Option<f64>) -> ModelRates {
    ModelRates {
        input,
        output,
        cached_input,
        context_limit: None,
    }
}

/// Like [`rates`], for 1M-native models (Fable / Mythos) whose ids carry no
/// `[1m]` marker for the window to be inferred from.
const fn rates_1m(input: f64, output: f64, cached_input: Option<f64>) -> ModelRates {
    ModelRates {
        input,
        output,
        cached_input,
        context_limit: Some(1_000_000),
    }
}

/// List pricing as of 2026-07. Longest matching prefix wins; overrides from
/// `~/.workspacer/model-rates.json` win length ties.
const BUILTIN: &[(&str, ModelRates)] = &[
    // Anthropic (2026-06 list): Fable/Mythos $10/$50, Opus 4.5+ $5/$25,
    // Sonnet $3/$15, Haiku $1/$5; Opus 4.1 and older kept $15/$75.
    //
    // NOTE on the legacy Opus keys (kept in lock-step with modelUsage.ts's
    // MODEL_RATES): transcripts carry the *dated* ids. Opus 4.0 is
    // 'claude-opus-4-20250514' — which does NOT start with the 'claude-opus-4-0'
    // alias — so it needs its own 'claude-opus-4-20' prefix (dated 4.0 ids are
    // the only ids that continue "4-2" with a "0"). Opus 4.1 is
    // 'claude-opus-4-1-20250805', so the key carries a trailing '-'
    // ('claude-opus-4-1-') to pin it to the 4.1 generation only — a bare
    // 'claude-opus-4-1' prefix would also swallow 4.10–4.19
    // ('claude-opus-4-10…'), which are current generations and should price at
    // the generic 5/25. Claude 3 Opus ids ('claude-3-opus-20240229') don't
    // start with 'claude-opus' at all, hence the separate 'claude-3-opus' entry.
    // 1M-native — without the explicit limit the 200K default made
    // transcript-derived context % read 5× high on Fable/Mythos sessions.
    ("claude-fable", rates_1m(10.0, 50.0, None)),
    ("claude-mythos", rates_1m(10.0, 50.0, None)),
    ("claude-opus", rates(5.0, 25.0, None)),
    ("claude-opus-4-1-", rates(15.0, 75.0, None)),
    ("claude-opus-4-0", rates(15.0, 75.0, None)),
    ("claude-opus-4-20", rates(15.0, 75.0, None)),
    ("claude-3-opus", rates(15.0, 75.0, None)),
    ("claude-sonnet", rates(3.0, 15.0, None)),
    ("claude-haiku", rates(1.0, 5.0, None)),
    // OpenAI (Codex models). Cached input is billed at its own (much lower)
    // rate; the wire reports cached as a subset of input tokens.
    ("gpt-5", rates(1.25, 10.0, Some(0.125))),
    ("gpt-5-codex", rates(1.25, 10.0, Some(0.125))),
    ("gpt-5-mini", rates(0.25, 2.0, Some(0.025))),
    ("gpt-5-nano", rates(0.05, 0.40, Some(0.005))),
    ("gpt-5-pro", rates(15.0, 120.0, None)),
    ("codex-mini", rates(1.5, 6.0, Some(0.375))),
    ("o3", rates(2.0, 8.0, Some(0.5))),
    ("o4-mini", rates(1.1, 4.4, Some(0.275))),
];

/// The user's editable overrides file.
pub fn overrides_path() -> Option<PathBuf> {
    Some(
        BaseDirs::new()?
            .home_dir()
            .join(".workspacer")
            .join("model-rates.json"),
    )
}

/// mtime-keyed cache of the parsed overrides file, so the hot paths (usage
/// events, status-line assembly) cost one stat, not one parse.
type CachedOverrides = (Option<SystemTime>, HashMap<String, ModelRates>);
static OVERRIDES: Mutex<Option<CachedOverrides>> = Mutex::new(None);

fn overrides() -> HashMap<String, ModelRates> {
    let Some(path) = overrides_path() else {
        return HashMap::new();
    };
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut guard = OVERRIDES.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((cached_mtime, table)) = guard.as_ref() {
        if *cached_mtime == mtime {
            return table.clone();
        }
    }
    let table: HashMap<String, ModelRates> = match mtime {
        None => HashMap::new(), // no file (or unreadable) — built-ins only
        Some(_) => std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| match serde_json::from_str(&s) {
                Ok(t) => Some(t),
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "model-rates.json is invalid JSON; ignoring");
                    None
                }
            })
            .unwrap_or_default(),
    };
    *guard = Some((mtime, table.clone()));
    table
}

/// Longest-prefix match over built-ins + user overrides (overrides win ties).
/// None when the model is unknown to both — callers decide their own fallback
/// (Claude costing has a default rate; managed cost estimation stays blank
/// rather than invent numbers).
pub fn rates_for(model: &str) -> Option<ModelRates> {
    let user = overrides();
    rates_for_in(model, BUILTIN, &user)
}

fn rates_for_in(
    model: &str,
    builtin: &[(&str, ModelRates)],
    user: &HashMap<String, ModelRates>,
) -> Option<ModelRates> {
    let mut best: Option<ModelRates> = None;
    let mut best_len = 0usize;
    for (prefix, r) in builtin {
        if model.starts_with(prefix) && prefix.len() > best_len {
            best = Some(*r);
            best_len = prefix.len();
        }
    }
    for (prefix, r) in user {
        // `>=` so a user entry beats the built-in of the same prefix.
        if model.starts_with(prefix.as_str()) && prefix.len() >= best_len {
            best = Some(*r);
            best_len = prefix.len();
        }
    }
    best
}

/// Cumulative USD estimate from cumulative token totals, for providers whose
/// wire reports tokens but no dollars. `input` INCLUDES `cached_input` (the
/// Codex wire's shape); cache reads bill at the entry's `cached_input` rate,
/// falling back to 0.1× input. None when the model is unknown/absent — an
/// invented rate would be worse than a blank readout.
pub fn estimate_cost(
    model: Option<&str>,
    input: Option<u64>,
    cached_input: Option<u64>,
    output: Option<u64>,
) -> Option<f64> {
    let r = rates_for(model?)?;
    let input = input.unwrap_or(0);
    let cached = cached_input.unwrap_or(0).min(input);
    let fresh = (input - cached) as f64;
    let cached_rate = r.cached_input.unwrap_or(r.input * 0.1);
    let dollars =
        fresh * r.input + cached as f64 * cached_rate + output.unwrap_or(0) as f64 * r.output;
    Some(dollars / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins() {
        assert_eq!(
            rates_for_in("gpt-5-codex-preview", BUILTIN, &HashMap::new())
                .unwrap()
                .input,
            1.25
        );
        // gpt-5-mini must not fall back to the shorter gpt-5 entry.
        assert_eq!(
            rates_for_in("gpt-5-mini-2027", BUILTIN, &HashMap::new())
                .unwrap()
                .input,
            0.25
        );
        assert!(rates_for_in("gemini-3", BUILTIN, &HashMap::new()).is_none());
    }

    #[test]
    fn user_override_beats_builtin_of_same_prefix() {
        let mut user = HashMap::new();
        user.insert("gpt-5-codex".to_string(), rates(9.0, 90.0, Some(0.9)));
        let r = rates_for_in("gpt-5-codex", BUILTIN, &user).unwrap();
        assert_eq!(r.input, 9.0);
        // Unrelated models still hit the built-ins.
        assert_eq!(rates_for_in("gpt-5", BUILTIN, &user).unwrap().input, 1.25);
    }

    #[test]
    fn legacy_opus_ids_price_at_15_75_current_at_5_25() {
        // Regression guards for the drift the shared fixture caught:
        // dated Opus 4.0 and Claude 3 Opus must price at legacy 15/75, while
        // current dated Opus 4.x (incl. 4.10+) stay on the generic 5/25.
        let cases: &[(&str, f64, f64)] = &[
            // Legacy 15/75.
            ("claude-opus-4-1-20250805", 15.0, 75.0),
            ("claude-opus-4-20250514", 15.0, 75.0), // dated Opus 4.0
            ("claude-3-opus-20240229", 15.0, 75.0),
            // Current 5/25 — the trailing-dash pin must NOT swallow 4.10.
            ("claude-opus-4-10-20260601", 5.0, 25.0),
            ("claude-opus-4-8-20260101", 5.0, 25.0),
        ];
        for (model, input, output) in cases {
            let r = rates_for_in(model, BUILTIN, &HashMap::new())
                .unwrap_or_else(|| panic!("{model} must have a built-in rate"));
            assert_eq!(r.input, *input, "input rate for {model}");
            assert_eq!(r.output, *output, "output rate for {model}");
        }
    }

    /// Cross-language pricing contract: every id in the shared fixture
    /// (`contracts/model-pricing-cases.json`, also consumed by a TS test) must
    /// price identically here. This is the drift guard between `BUILTIN` and
    /// modelUsage.ts's `MODEL_RATES`. Loaded via `include_str!` so it's
    /// compile-time and CWD-independent.
    #[test]
    fn matches_shared_pricing_contract() {
        #[derive(serde::Deserialize)]
        struct Case {
            model: String,
            input: f64,
            output: f64,
            #[allow(dead_code)]
            note: Option<String>,
        }
        #[derive(serde::Deserialize)]
        struct Contract {
            cases: Vec<Case>,
        }

        const FIXTURE: &str = include_str!("../../../../contracts/model-pricing-cases.json");
        let contract: Contract =
            serde_json::from_str(FIXTURE).expect("model-pricing-cases.json parses");
        assert!(!contract.cases.is_empty(), "fixture has cases");
        for case in &contract.cases {
            let r = rates_for_in(&case.model, BUILTIN, &HashMap::new())
                .unwrap_or_else(|| panic!("no built-in rate for contract model {}", case.model));
            assert_eq!(r.input, case.input, "input rate for {}", case.model);
            assert_eq!(r.output, case.output, "output rate for {}", case.model);
        }
    }

    #[test]
    fn estimate_prices_cached_tokens_at_the_cached_rate() {
        // 1M fresh + 1M cached + 1M output on gpt-5-codex:
        // 1.25 + 0.125 + 10.0 = 11.375
        let c = estimate_cost(
            Some("gpt-5-codex"),
            Some(2_000_000),
            Some(1_000_000),
            Some(1_000_000),
        )
        .unwrap();
        assert!((c - 11.375).abs() < 1e-9);
        // Unknown model → None, not a made-up figure.
        assert!(estimate_cost(Some("mystery-lm"), Some(1000), None, Some(10)).is_none());
        assert!(estimate_cost(None, Some(1000), None, Some(10)).is_none());
    }
}

//! Prompt-cache warmth: when a session's cached prefix expires, and what the
//! next message costs once it has.
//!
//! Every API request that reads or writes the cache refreshes its lifetime, so
//! the prefix is alive until `last request + TTL`. After that, the next message
//! resends the whole context at the cache-WRITE price (Claude: 1.25× base input
//! for the 5-minute TTL, 2× for the 1-hour one) instead of the read price
//! (0.1×). A session left for an hour and resumed can cost 10-20× what the same
//! message would have cost warm, and nothing in the UI said so.
//!
//! Two feeds, same shape ([`RequestObservation`]):
//!  - the transcript fold in [`super::usage`] — every main-thread assistant row
//!    is one request, carrying its timestamp and the TTL it bought in
//!    `usage.cache_creation.ephemeral_{5m,1h}_input_tokens`. This is what
//!    covers PTY sessions and stopped / rehydrated rows.
//!  - live provider frames ([`crate::providers::AgentUpdate::ApiRequest`]),
//!    recorded on the session by `SessionStore::note_api_request`. This is the
//!    only feed Codex has: its wire carries no TTL, so its warmth is an
//!    ESTIMATE (OpenAI keeps an idle prefix for roughly 5-10 minutes; we take
//!    the long end) and says so with `estimated: true`.
//!
//! Clients compare `expires_at` against their own clock: the snapshot only
//! says when, never whether, so a row does not need republishing to go cold.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::Value;

use super::state::SessionState;
use super::usage::Usage;

/// Claude's default cache lifetime, and what an untagged write is assumed to
/// have bought. The SHORTER lifetime on purpose: guessing short warns a little
/// early, guessing long tells someone a cold cache is warm.
pub const CLAUDE_DEFAULT_TTL_SECS: u64 = 300;
/// The 1-hour lifetime a request buys by writing `ephemeral_1h_input_tokens`.
pub const CLAUDE_1H_TTL_SECS: u64 = 3600;
/// OpenAI reports no lifetime; an idle prefix lasts about 5-10 minutes.
pub const CODEX_ESTIMATED_TTL_SECS: u64 = 600;

/// One API request against the session's cached prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestObservation {
    /// When the request was made, Unix ms.
    pub at_ms: i64,
    /// The API message id. Claude writes one transcript row (and the stream
    /// one `assistant` frame) per content block, all sharing the message's id:
    /// they are one request, and the FIRST of them is closest to when it ran.
    pub message_id: Option<String>,
    /// The lifetime this request bought, or the last one known before it. A
    /// request that only READ the cache refreshes the lifetime the prefix
    /// already had, and says nothing itself.
    pub ttl_seconds: Option<u64>,
    /// Prompt tokens the request carried: fresh input + cache write + cache
    /// read. What a cold request has to send again.
    pub context_tokens: u64,
    pub model: Option<String>,
}

impl RequestObservation {
    /// This request folded on top of the previous one: blocks of the same
    /// message keep the message's first time, and an untagged request carries
    /// the lifetime forward.
    pub fn after(mut self, previous: Option<&RequestObservation>) -> Self {
        let Some(previous) = previous else {
            return self;
        };
        if self.message_id.is_some() && self.message_id == previous.message_id {
            self.at_ms = self.at_ms.min(previous.at_ms);
        }
        if self.ttl_seconds.is_none() {
            self.ttl_seconds = previous.ttl_seconds;
        }
        if self.model.is_none() {
            self.model = previous.model.clone();
        }
        self
    }
}

/// What the snapshot says about a session's prompt cache, as `prompt_cache`.
///
/// TWIN: hub-rs `snapshots::compat` projects this to camelCase `promptCache`;
/// the native client reads it in `wks_native::model::PromptCache`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PromptCache {
    /// How long the cached prefix outlives the last request.
    pub ttl_seconds: u64,
    /// The last request that read or wrote the cache, Unix ms.
    pub last_request_at: i64,
    /// When the cached prefix expires, Unix ms: `last_request_at + ttl`.
    pub expires_at: i64,
    /// Prompt tokens the next request resends: all of them, at the write
    /// price, once the cache has expired.
    pub context_tokens: u64,
    /// `true` when the lifetime is assumed rather than reported: always for
    /// Codex, and for Claude until a request tags its writes with a TTL.
    pub estimated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// USD to resend `context_tokens` with the cache expired. Absent when the
    /// model has no price (never a guess dressed as a figure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold_cost_usd: Option<f64>,
    /// USD for the same context read from a warm cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_cost_usd: Option<f64>,
}

/// The cache lifetime a request bought, from its `usage.cache_creation`
/// split; `None` when it wrote nothing it tagged.
pub fn ttl_of(usage: &Value) -> Option<u64> {
    let (m5, m1h) = super::usage::cache_write_ttl_split(usage)?;
    if m1h > 0 {
        Some(CLAUDE_1H_TTL_SECS)
    } else if m5 > 0 {
        Some(CLAUDE_DEFAULT_TTL_SECS)
    } else {
        None
    }
}

/// A transcript row's own `timestamp`, Unix ms.
pub fn row_timestamp_ms(row: &Value) -> Option<i64> {
    parse_ms(row.get("timestamp")?.as_str()?)
}

fn parse_ms(raw: &str) -> Option<i64> {
    let at =
        time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339).ok()?;
    Some((at.unix_timestamp_nanos() / 1_000_000) as i64)
}

pub fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// The session's prompt-cache picture, or `None` when there is nothing to
/// say: no request seen yet, or a provider whose caching we cannot describe.
pub fn derive(state: &SessionState, usage: &Usage) -> Option<PromptCache> {
    match state.provider.as_str() {
        "claude" => claude(state, usage),
        "codex" => codex(state),
        _ => None,
    }
}

/// The later of the transcript's and the live feed's last request, with the
/// lifetime whichever of them knows it.
fn claude(state: &SessionState, usage: &Usage) -> Option<PromptCache> {
    let folded = usage.last_request.as_ref();
    let live = state.last_api_request.as_ref();
    let (latest, other) = match (folded, live) {
        (Some(f), Some(l)) if l.at_ms > f.at_ms => (l, Some(f)),
        (Some(f), l) => (f, l),
        (None, Some(l)) => (l, None),
        (None, None) => return None,
    };
    if latest.context_tokens == 0 {
        return None;
    }
    let reported = latest
        .ttl_seconds
        .or_else(|| other.and_then(|o| o.ttl_seconds));
    let ttl = reported.unwrap_or(CLAUDE_DEFAULT_TTL_SECS);
    let model = latest
        .model
        .clone()
        .or_else(|| other.and_then(|o| o.model.clone()))
        .or_else(|| usage.model.clone());
    let (cold, warm) =
        super::usage::claude_resend_costs(model.as_deref(), latest.context_tokens, ttl);
    Some(PromptCache {
        ttl_seconds: ttl,
        last_request_at: latest.at_ms,
        expires_at: latest.at_ms + ttl as i64 * 1000,
        context_tokens: latest.context_tokens,
        estimated: reported.is_none(),
        model,
        cold_cost_usd: Some(cold),
        warm_cost_usd: Some(warm),
    })
}

/// Codex: always an estimate. Live requests while the daemon watched, else the
/// last `token_count` its rollout recorded (a stopped session after restart).
fn codex(state: &SessionState) -> Option<PromptCache> {
    let observed = state
        .last_api_request
        .clone()
        .or_else(|| codex_rollout_request(&state.session_id))?;
    if observed.context_tokens == 0 {
        return None;
    }
    let model = observed
        .model
        .clone()
        .or_else(|| state.status_line.as_ref()?.model_display.clone())
        .or_else(|| state.requested_model.clone());
    let (cold, warm) = match model.as_deref().and_then(super::pricing::rates_for) {
        Some(r) => {
            let tokens = observed.context_tokens as f64 / 1_000_000.0;
            // OpenAI has no write premium: cold is fresh input, warm the
            // cached-input rate (no cached rate = no discount to lose).
            (
                Some(tokens * r.input),
                Some(tokens * r.cached_input.unwrap_or(r.input)),
            )
        }
        None => (None, None),
    };
    let ttl = CODEX_ESTIMATED_TTL_SECS;
    Some(PromptCache {
        ttl_seconds: ttl,
        last_request_at: observed.at_ms,
        expires_at: observed.at_ms + ttl as i64 * 1000,
        context_tokens: observed.context_tokens,
        estimated: true,
        model,
        cold_cost_usd: cold,
        warm_cost_usd: warm,
    })
}

/// The last request a Codex rollout recorded: its final `token_count` event's
/// timestamp and last-request input, with the model its `turn_context` named.
pub fn last_rollout_request(text: &str) -> Option<RequestObservation> {
    let mut model: Option<String> = None;
    let mut last: Option<RequestObservation> = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = row.get("payload").unwrap_or(&row);
        match row.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                if let Some(m) = payload.get("model").and_then(Value::as_str) {
                    model = Some(m.to_owned());
                }
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("token_count") =>
            {
                let tokens = payload
                    .pointer("/info/last_token_usage/input_tokens")
                    .and_then(Value::as_u64);
                if let (Some(tokens), Some(at_ms)) = (tokens, row_timestamp_ms(&row)) {
                    last = Some(RequestObservation {
                        at_ms,
                        message_id: None,
                        ttl_seconds: None,
                        context_tokens: tokens,
                        model: model.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    last
}

/// (len, mtime ns) of a file, so an unchanged rollout is not re-read.
type Stamp = (u64, i128);
/// session id → its rollout (found once), and the last fold of that rollout.
type RolloutMemo = HashMap<String, (PathBuf, Stamp, Option<RequestObservation>)>;
static ROLLOUTS: Lazy<Mutex<RolloutMemo>> = Lazy::new(|| Mutex::new(HashMap::new()));
const MAX_ROLLOUTS: usize = 256;

fn stamp(path: &std::path::Path) -> Option<Stamp> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0);
    Some((md.len(), mtime))
}

fn codex_rollout_request(session_id: &str) -> Option<RequestObservation> {
    let known = ROLLOUTS
        .lock()
        .ok()
        .and_then(|memo| memo.get(session_id).cloned());
    let path = match &known {
        Some((path, ..)) => path.clone(),
        None => {
            let thread = crate::providers::codex_rollout::thread_for(session_id)?;
            crate::providers::codex_rollout::rollout_for_thread(&thread)?
        }
    };
    let now = stamp(&path)?;
    if let Some((_, was, observed)) = &known {
        if *was == now {
            return observed.clone();
        }
    }
    let observed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| last_rollout_request(&text));
    if let Ok(mut memo) = ROLLOUTS.lock() {
        if memo.len() >= MAX_ROLLOUTS && !memo.contains_key(session_id) {
            memo.clear();
        }
        memo.insert(session_id.to_owned(), (path, now, observed.clone()));
    }
    observed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::transcript::{Transcript, TranscriptMessage};
    use serde_json::json;

    /// A main-thread assistant row as current Claude Code writes it.
    fn row(id: &str, at: &str, fresh: u64, m5: u64, m1h: u64, read: u64) -> TranscriptMessage {
        TranscriptMessage {
            role: "assistant".into(),
            content: Value::Null,
            raw: json!({
                "type": "assistant",
                "timestamp": at,
                "uuid": format!("{id}-{at}"),
                "message": {
                    "id": id,
                    "model": "claude-opus-4-8",
                    "usage": {
                        "input_tokens": fresh,
                        "cache_creation_input_tokens": m5 + m1h,
                        "cache_read_input_tokens": read,
                        "output_tokens": 40,
                        "cache_creation": {
                            "ephemeral_5m_input_tokens": m5,
                            "ephemeral_1h_input_tokens": m1h
                        }
                    }
                }
            }),
        }
    }

    fn fold(rows: Vec<TranscriptMessage>) -> Usage {
        super::super::usage::from_transcript(&Transcript {
            messages: rows,
            ..Default::default()
        })
        .unwrap()
    }

    fn claude_state() -> SessionState {
        SessionState::new("s".into(), None)
    }

    fn ms(raw: &str) -> i64 {
        parse_ms(raw).unwrap()
    }

    #[test]
    fn a_1h_write_expires_an_hour_after_the_last_request() {
        let usage = fold(vec![
            row("m1", "2026-10-07T10:00:00.000Z", 3, 0, 20_000, 0),
            // A read-only hit later refreshes the hour it already had.
            row("m2", "2026-10-07T10:20:00.000Z", 5, 0, 0, 20_000),
        ]);
        let cache = derive(&claude_state(), &usage).unwrap();
        assert_eq!(cache.ttl_seconds, 3600);
        assert!(!cache.estimated);
        assert_eq!(cache.last_request_at, ms("2026-10-07T10:20:00.000Z"));
        assert_eq!(cache.expires_at, ms("2026-10-07T11:20:00.000Z"));
        assert_eq!(cache.context_tokens, 20_005);
        assert_eq!(cache.model.as_deref(), Some("claude-opus-4-8"));
        // Opus $5/M: cold = 2x write, warm = 0.1x read.
        let tokens = 20_005. / 1_000_000.;
        assert!((cache.cold_cost_usd.unwrap() - tokens * 10.).abs() < 1e-9);
        assert!((cache.warm_cost_usd.unwrap() - tokens * 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_5m_write_expires_five_minutes_after_and_costs_the_5m_rate() {
        let usage = fold(vec![row(
            "m1",
            "2026-10-07T10:00:00.000Z",
            10,
            600_000,
            0,
            24_000,
        )]);
        let cache = derive(&claude_state(), &usage).unwrap();
        assert_eq!(cache.ttl_seconds, 300);
        assert_eq!(cache.expires_at, ms("2026-10-07T10:05:00.000Z"));
        assert_eq!(cache.context_tokens, 624_010);
        let tokens = 624_010. / 1_000_000.;
        assert!((cache.cold_cost_usd.unwrap() - tokens * 5. * 1.25).abs() < 1e-9);
    }

    #[test]
    fn blocks_of_one_message_keep_its_first_time() {
        let usage = fold(vec![
            row("m1", "2026-10-07T10:00:00.000Z", 3, 0, 9_000, 0),
            row("m1", "2026-10-07T10:03:30.000Z", 3, 0, 9_000, 0),
        ]);
        let cache = derive(&claude_state(), &usage).unwrap();
        assert_eq!(cache.last_request_at, ms("2026-10-07T10:00:00.000Z"));
    }

    #[test]
    fn sidechain_and_placeholder_rows_do_not_keep_the_parent_warm() {
        let mut sub = row("s1", "2026-10-07T10:50:00.000Z", 3, 0, 9_000, 0);
        sub.raw["isSidechain"] = json!(true);
        let mut synthetic = row("x1", "2026-10-07T10:55:00.000Z", 0, 0, 0, 0);
        synthetic.raw["message"]["model"] = json!("<synthetic>");
        let usage = fold(vec![
            row("m1", "2026-10-07T10:00:00.000Z", 3, 0, 9_000, 0),
            sub,
            synthetic,
        ]);
        let cache = derive(&claude_state(), &usage).unwrap();
        assert_eq!(cache.last_request_at, ms("2026-10-07T10:00:00.000Z"));
    }

    #[test]
    fn untagged_writes_are_an_estimated_five_minutes() {
        let mut untagged = row("m1", "2026-10-07T10:00:00.000Z", 3, 0, 0, 0);
        untagged.raw["message"]["usage"] = json!({
            "input_tokens": 3, "cache_creation_input_tokens": 8_000,
            "cache_read_input_tokens": 0, "output_tokens": 1
        });
        let cache = derive(&claude_state(), &fold(vec![untagged])).unwrap();
        assert_eq!(cache.ttl_seconds, 300);
        assert!(cache.estimated);
    }

    #[test]
    fn a_stopped_session_reads_its_cache_from_the_transcript_on_disk() {
        let dir =
            std::env::temp_dir().join(format!("claudemon-prompt-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::session::transcript::allow_root(&dir);
        let path = dir.join("stopped.jsonl");
        let rows = [
            row("m1", "2026-10-07T08:00:00.000Z", 3, 0, 300_000, 0),
            row("m2", "2026-10-07T08:40:00.000Z", 7, 0, 1_000, 300_000),
        ];
        let text: String = rows
            .iter()
            .map(|r| format!("{}\n", r.raw))
            .collect::<String>();
        std::fs::write(&path, text).unwrap();
        // As a rehydrated row: stopped, no live feed, only the stored path.
        let mut state = claude_state();
        state.mode = crate::session::SessionMode::Stopped;
        state.transcript_path = Some(path.to_string_lossy().into_owned());
        let usage = super::super::usage::usage_for_session(&state);
        let cache = derive(&state, &usage).expect("transcript-backed cache");
        assert_eq!(cache.ttl_seconds, 3600);
        assert_eq!(cache.expires_at, ms("2026-10-07T09:40:00.000Z"));
        assert_eq!(cache.context_tokens, 301_007);
        assert!(!cache.estimated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_live_feed_wins_when_it_is_newer_and_borrows_the_transcript_ttl() {
        let usage = fold(vec![row("m1", "2026-10-07T10:00:00.000Z", 3, 0, 9_000, 0)]);
        let mut state = claude_state();
        state.last_api_request = Some(RequestObservation {
            at_ms: ms("2026-10-07T10:30:00.000Z"),
            message_id: Some("m2".into()),
            ttl_seconds: None,
            context_tokens: 12_000,
            model: None,
        });
        let cache = derive(&state, &usage).unwrap();
        assert_eq!(cache.last_request_at, ms("2026-10-07T10:30:00.000Z"));
        assert_eq!(cache.ttl_seconds, 3600, "the transcript knew the TTL");
        assert_eq!(cache.context_tokens, 12_000);
        assert_eq!(cache.model.as_deref(), Some("claude-opus-4-8"));
        // An older live sample loses to the transcript.
        state.last_api_request.as_mut().unwrap().at_ms = ms("2026-10-07T09:00:00.000Z");
        assert_eq!(
            derive(&state, &usage).unwrap().last_request_at,
            ms("2026-10-07T10:00:00.000Z")
        );
    }

    #[test]
    fn codex_is_an_estimate_from_its_last_request() {
        let mut state = claude_state();
        state.provider = "codex".into();
        assert_eq!(derive(&state, &Usage::default()), None, "nothing seen yet");
        state.last_api_request = Some(RequestObservation {
            at_ms: 1_000_000,
            message_id: None,
            ttl_seconds: None,
            context_tokens: 200_000,
            model: Some("gpt-5-codex".into()),
        });
        let cache = derive(&state, &Usage::default()).unwrap();
        assert!(cache.estimated);
        assert_eq!(cache.ttl_seconds, 600);
        assert_eq!(cache.expires_at, 1_000_000 + 600_000);
        // gpt-5-codex: $1.25/M fresh, $0.125/M cached.
        assert!((cache.cold_cost_usd.unwrap() - 0.25).abs() < 1e-9);
        assert!((cache.warm_cost_usd.unwrap() - 0.025).abs() < 1e-9);
    }

    #[test]
    fn a_codex_rollout_yields_its_last_token_count() {
        let text = [
            json!({"timestamp":"2026-10-07T10:00:00.000Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}),
            json!({"timestamp":"2026-10-07T10:01:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":50_000}}}}),
            json!({"timestamp":"2026-10-07T10:02:00.000Z","type":"event_msg","payload":{"type":"token_count","info":null}}),
            json!({"timestamp":"2026-10-07T10:09:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":61_000}}}}),
        ]
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>();
        let last = last_rollout_request(&text).unwrap();
        assert_eq!(last.at_ms, ms("2026-10-07T10:09:00.000Z"));
        assert_eq!(last.context_tokens, 61_000);
        assert_eq!(last.model.as_deref(), Some("gpt-5-codex"));
    }

    #[test]
    fn other_providers_say_nothing() {
        let mut state = claude_state();
        state.provider = "copilot".into();
        state.last_api_request = Some(RequestObservation {
            at_ms: 1,
            context_tokens: 5,
            ..Default::default()
        });
        assert_eq!(derive(&state, &Usage::default()), None);
    }

    #[test]
    fn the_wire_shape_is_snake_case_and_omits_unknown_prices() {
        let cache = PromptCache {
            ttl_seconds: 600,
            last_request_at: 1,
            expires_at: 600_001,
            context_tokens: 9,
            estimated: true,
            model: None,
            cold_cost_usd: None,
            warm_cost_usd: None,
        };
        assert_eq!(
            serde_json::to_value(&cache).unwrap(),
            json!({"ttl_seconds":600,"last_request_at":1,"expires_at":600_001,
                   "context_tokens":9,"estimated":true})
        );
    }
}

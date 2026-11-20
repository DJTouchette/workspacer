//! Google Antigravity CLI (`agy`) usage, read from disk with no live session.
//!
//! Antigravity CLI records every conversation step in
//! `~/.gemini/antigravity-cli/brain/<conversation-id>/.system_generated/logs/transcript.jsonl`.
//! Each `PLANNER_RESPONSE` line carries the exact per-request `model`,
//! `input_tokens`, `cache_read_tokens`, `output_tokens`, and `created_at`
//! timestamp.
//!
//! # Quota headroom is UNAVAILABLE
//!
//! Neither Antigravity's local files nor its ACP (`agy acp`) protocol expose
//! rate-limit window percentages or reset timestamps. Reporting `unavailable`
//! rather than `0%` or `unknown` keeps limit-aware routing honest and ensures
//! Antigravity never drags aggregate health to UNKNOWN.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde_json::Value;

/// Why Antigravity quota headroom cannot be reported.
pub const QUOTA_UNAVAILABLE: &str =
    "Antigravity CLI exposes no local or ACP quota window endpoint (probed live against agy 1.5.13)";

/// Per-model token usage folded from Antigravity CLI transcripts.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AntigravityModelUsage {
    pub model: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub output_tokens: u64,
}

impl AntigravityModelUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.output_tokens)
    }
}

/// Folded on-disk usage across all local Antigravity CLI conversations.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AntigravityUsage {
    pub by_model: Vec<AntigravityModelUsage>,
    pub totals: AntigravityModelUsage,
    pub sessions: u64,
    pub first_event_at: Option<String>,
    pub last_event_at: Option<String>,
    pub source: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AntigravityUsageError {
    NoHome,
    NoStore,
    Unreadable,
}

impl std::fmt::Display for AntigravityUsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoHome => "no home directory",
            Self::NoStore => "antigravity has recorded no sessions here",
            Self::Unreadable => "antigravity's transcript directory could not be read",
        })
    }
}

/// `~/.gemini/antigravity-cli/brain`.
fn brain_dir() -> Option<PathBuf> {
    Some(
        BaseDirs::new()?
            .home_dir()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain"),
    )
}

/// Fold all local Antigravity CLI transcripts.
pub fn read_from_disk() -> Result<AntigravityUsage, AntigravityUsageError> {
    let dir = brain_dir().ok_or(AntigravityUsageError::NoHome)?;
    if !dir.is_dir() {
        return Err(AntigravityUsageError::NoStore);
    }
    read_brain_dir(&dir)
}

/// [`read_from_disk`] against an explicit `brain/` directory for unit tests.
pub fn read_brain_dir(brain: &Path) -> Result<AntigravityUsage, AntigravityUsageError> {
    let entries = std::fs::read_dir(brain).map_err(|_| AntigravityUsageError::Unreadable)?;
    let mut by_model: BTreeMap<String, AntigravityModelUsage> = BTreeMap::new();
    let mut sessions: u64 = 0;
    let mut first_event_at: Option<String> = None;
    let mut last_event_at: Option<String> = None;

    for entry in entries.flatten() {
        let path = entry
            .path()
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        if !path.is_file() {
            continue;
        }
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let mut session_had_usage = false;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() || !trimmed.contains("_tokens") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            let input = v.get("input_tokens").and_then(Value::as_u64);
            let cache_read = v.get("cache_read_tokens").and_then(Value::as_u64);
            let output = v.get("output_tokens").and_then(Value::as_u64);
            if input.is_none() && cache_read.is_none() && output.is_none() {
                continue;
            }
            session_had_usage = true;
            let model = v
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown")
                .to_owned();
            let e = by_model
                .entry(model.clone())
                .or_insert_with(|| AntigravityModelUsage {
                    model,
                    ..Default::default()
                });
            e.requests += 1;
            e.input_tokens = e.input_tokens.saturating_add(input.unwrap_or(0));
            e.cache_read_tokens = e.cache_read_tokens.saturating_add(cache_read.unwrap_or(0));
            e.output_tokens = e.output_tokens.saturating_add(output.unwrap_or(0));

            if let Some(ts) = v
                .get("created_at")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                match &first_event_at {
                    None => first_event_at = Some(ts.to_owned()),
                    Some(cur) if ts < cur.as_str() => first_event_at = Some(ts.to_owned()),
                    _ => {}
                }
                match &last_event_at {
                    None => last_event_at = Some(ts.to_owned()),
                    Some(cur) if ts > cur.as_str() => last_event_at = Some(ts.to_owned()),
                    _ => {}
                }
            }
        }
        if session_had_usage {
            sessions += 1;
        }
    }

    if sessions == 0 {
        return Err(AntigravityUsageError::NoStore);
    }

    let mut totals = AntigravityModelUsage::default();
    for m in by_model.values() {
        totals.requests = totals.requests.saturating_add(m.requests);
        totals.input_tokens = totals.input_tokens.saturating_add(m.input_tokens);
        totals.cache_read_tokens = totals.cache_read_tokens.saturating_add(m.cache_read_tokens);
        totals.output_tokens = totals.output_tokens.saturating_add(m.output_tokens);
    }

    Ok(AntigravityUsage {
        by_model: by_model.into_values().collect(),
        totals,
        sessions,
        first_event_at,
        last_event_at,
        source: brain.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_per_model_tokens_and_sessions_from_transcript_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let s1 = tmp
            .path()
            .join("conv-1")
            .join(".system_generated")
            .join("logs");
        let s2 = tmp
            .path()
            .join("conv-2")
            .join(".system_generated")
            .join("logs");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::create_dir_all(&s2).unwrap();

        std::fs::write(
            s1.join("transcript.jsonl"),
            concat!(
                "{\"step_index\":0,\"type\":\"USER_INPUT\",\"created_at\":\"2026-11-20T18:00:00Z\",\"content\":\"hi\"}\n",
                "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-11-20T18:00:05Z\",\"model\":\"gemini-4-neon-high\",\"input_tokens\":1000,\"cache_read_tokens\":5000,\"output_tokens\":200}\n",
                "{\"step_index\":2,\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-11-20T18:01:00Z\",\"model\":\"gemini-4-neon-high\",\"input_tokens\":500,\"cache_read_tokens\":6000,\"output_tokens\":100}\n",
                "{malformed json line}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            s2.join("transcript.jsonl"),
            "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-11-20T19:00:00Z\",\"model\":\"gemini-3.8-flash-medium\",\"input_tokens\":300,\"cache_read_tokens\":0,\"output_tokens\":50}\n",
        )
        .unwrap();

        let u = read_brain_dir(tmp.path()).unwrap();
        assert_eq!(u.sessions, 2);
        assert_eq!(u.totals.requests, 3);
        assert_eq!(u.totals.input_tokens, 1800);
        assert_eq!(u.totals.cache_read_tokens, 11000);
        assert_eq!(u.totals.output_tokens, 350);
        assert_eq!(u.totals.total_tokens(), 13150);
        assert_eq!(u.first_event_at.as_deref(), Some("2026-11-20T18:00:05Z"));
        assert_eq!(u.last_event_at.as_deref(), Some("2026-11-20T19:00:00Z"));
        assert_eq!(u.by_model.len(), 2);
        assert_eq!(u.by_model[0].model, "gemini-3.8-flash-medium");
        assert_eq!(u.by_model[1].model, "gemini-4-neon-high");
        assert_eq!(u.by_model[1].requests, 2);
    }

    #[test]
    fn empty_brain_dir_reports_no_store() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            read_brain_dir(tmp.path()).unwrap_err(),
            AntigravityUsageError::NoStore
        );
    }
}

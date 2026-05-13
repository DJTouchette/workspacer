//! Reader for Claude Code's on-disk JSONL transcripts at
//! `~/.claude/projects/<encoded-cwd>/*.jsonl`.

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use directories::BaseDirs;
use serde::Serialize;

pub fn projects_dir() -> Option<PathBuf> {
    let base = BaseDirs::new()?;
    Some(base.home_dir().join(".claude").join("projects"))
}

/// Claude encodes `/foo/bar` as `-foo-bar`.
pub fn encoded_cwd(cwd: &str) -> String {
    cwd.replace(['/', '\\'], "-")
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptMessage {
    pub role: String,
    pub text: Option<String>,
    /// Tool calls or other structured payloads, kept as raw JSON.
    pub raw: serde_json::Value,
}

#[derive(Debug, Default, Serialize)]
pub struct Transcript {
    pub path: Option<String>,
    pub messages: Vec<TranscriptMessage>,
}

/// Best-effort: find the most-recently-modified JSONL for a given cwd and
/// parse it into a flat list of messages. Returns an empty transcript if
/// nothing is found (e.g. wrapper-only session with no Claude Code running).
pub fn read_for_cwd(cwd: &str) -> Result<Transcript> {
    let Some(root) = projects_dir() else {
        return Ok(Transcript::default());
    };
    let dir = root.join(encoded_cwd(cwd));
    if !dir.exists() {
        return Ok(Transcript::default());
    }

    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry.metadata()?.modified().unwrap_or(std::time::UNIX_EPOCH);
        match &newest {
            Some((best, _)) if *best >= mtime => {}
            _ => newest = Some((mtime, path)),
        }
    }

    let Some((_, path)) = newest else {
        return Ok(Transcript::default());
    };

    let text = fs::read_to_string(&path)?;
    let mut messages = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(msg) = extract_message(&value) {
            messages.push(msg);
        }
    }

    Ok(Transcript {
        path: Some(path.to_string_lossy().into_owned()),
        messages,
    })
}

/// Extract a flat message from one JSONL row. Claude's transcript format
/// nests content as either a string or an array of typed blocks; we collapse
/// text blocks into a single string and keep the original value under `raw`.
fn extract_message(value: &serde_json::Value) -> Option<TranscriptMessage> {
    let role = value
        .get("role")
        .or_else(|| value.get("type"))
        .and_then(|v| v.as_str())?
        .to_string();
    let text = match value.get("content") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Array(items)) => {
            let collected: String = items
                .iter()
                .filter_map(|i| {
                    if i.get("type").and_then(|t| t.as_str()) == Some("text") {
                        i.get("text").and_then(|t| t.as_str()).map(str::to_owned)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            if collected.is_empty() {
                None
            } else {
                Some(collected)
            }
        }
        _ => None,
    };
    Some(TranscriptMessage { role, text, raw: value.clone() })
}

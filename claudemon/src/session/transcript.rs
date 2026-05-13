//! Reader for Claude Code's on-disk JSONL transcripts at
//! `~/.claude/projects/<encoded-cwd>/*.jsonl`.
//!
//! Stub: returns the directory the daemon would scan. The full JSONL parser
//! (turns + tool calls, matching what Workspacer's `processTranscriptEntry`
//! does) lands once the API needs to serve transcript content.

use std::path::PathBuf;

use directories::BaseDirs;

#[allow(dead_code)]
pub fn projects_dir() -> Option<PathBuf> {
    let base = BaseDirs::new()?;
    Some(base.home_dir().join(".claude").join("projects"))
}

#[allow(dead_code)]
pub fn encoded_cwd(cwd: &str) -> String {
    cwd.replace(['/', '\\'], "-")
}

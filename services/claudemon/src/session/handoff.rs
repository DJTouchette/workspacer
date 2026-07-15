//! Cross-provider session handoff — distill a session's conversation into a
//! markdown brief a *different* agent can pick up from.
//!
//! The brief is provider-neutral: it's built from the shared
//! [`ConversationItem`] timeline (which every adapter and the claude
//! transcript tailer feed), so any harness can hand off to any other. It gets
//! persisted under `~/.workspacer/handoffs/` and the successor agent's first
//! message just points at the file — every harness can read a file, and the
//! brief stays inspectable after the fact.
//!
//! Deliberately deterministic (no model in the loop): metadata header, the
//! spine of user requests, files touched, and the tail of the exchange with
//! the final assistant message kept fattest — that's where "where we left
//! off" lives. An agent-authored brief can layer on top later; this one always
//! works, even for a session that's already dead.

use std::path::PathBuf;

use directories::BaseDirs;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::conversation::ConversationItem;
use super::state::{PlanStatus, SessionState};

/// Per-message caps (chars). The final assistant message gets the big one.
const USER_SPINE_CAP: usize = 240;
const RECENT_TEXT_CAP: usize = 1_600;
const FINAL_ASSISTANT_CAP: usize = 5_000;
const TOOL_INPUT_CAP: usize = 200;
/// Overall soft budget for the "recent exchange" section.
const RECENT_BUDGET: usize = 10_000;
const MAX_FILES: usize = 40;
const MAX_USER_SPINE: usize = 25;

/// Truncate on a char boundary with an ellipsis marker.
fn clip(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        return s.to_string();
    }
    let cut: String = s.chars().take(cap).collect();
    format!("{cut}… [truncated]")
}

/// Pull a file path out of a tool input, across harness vocabularies
/// (claude `file_path`, codex/opencode `path`/`filePath`).
fn tool_path(input: &Value) -> Option<String> {
    ["file_path", "path", "filePath", "notebook_path"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .map(str::to_owned)
}

/// One compact line for a tool call: `Bash: cargo test` / `Edit: src/main.rs`.
fn tool_line(name: &str, input: &Value) -> String {
    let detail = tool_path(input)
        .or_else(|| {
            input
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            input
                .get("pattern")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            input
                .get("query")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            let s = input.to_string();
            if s == "null" || s == "{}" {
                String::new()
            } else {
                s
            }
        });
    if detail.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {}", clip(&detail, TOOL_INPUT_CAP))
    }
}

/// Build the handoff brief. `state` enriches the header when the session is
/// still known to the store; the conversation alone is enough otherwise.
pub fn build_brief(
    session_id: &str,
    state: Option<&SessionState>,
    items: &[ConversationItem],
) -> String {
    let now = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    let mut out = String::with_capacity(16 * 1024);

    out.push_str("# Session handoff brief\n\n");
    out.push_str(
        "You are taking over an in-progress working session from another AI coding agent.\n",
    );
    out.push_str("Read this brief, then continue the work — do not start over, do not redo completed steps.\n\n");
    out.push_str(&format!("- Source session: `{session_id}`\n"));
    if let Some(s) = state {
        if let Some(cwd) = &s.cwd {
            out.push_str(&format!("- Working directory: `{cwd}`\n"));
        }
        if let Some(model) = s
            .status_line
            .as_ref()
            .and_then(|sl| sl.model_display.as_deref())
        {
            out.push_str(&format!("- Source agent model: {model}\n"));
        }
        out.push_str(&format!("- Tool calls made: {}\n", s.tool_calls));
    }
    out.push_str(&format!("- Brief generated: {now}\n"));

    // ── The spine: every user request, oldest → newest ──
    let user_msgs: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            ConversationItem::UserMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if !user_msgs.is_empty() {
        out.push_str("\n## What the user asked for (oldest first)\n\n");
        let skip = user_msgs.len().saturating_sub(MAX_USER_SPINE);
        if skip > 0 {
            out.push_str(&format!("_({skip} earlier requests omitted)_\n"));
        }
        for text in user_msgs.iter().skip(skip) {
            out.push_str(&format!(
                "- {}\n",
                clip(&text.replace('\n', " "), USER_SPINE_CAP)
            ));
        }
    }

    // ── Files touched (edit-shaped tools first, then anything with a path) ──
    let mut edited: Vec<String> = Vec::new();
    let mut read_only: Vec<String> = Vec::new();
    for item in items {
        if let ConversationItem::ToolUse { name, input, .. } = item {
            if let Some(p) = tool_path(input) {
                let lower = name.to_ascii_lowercase();
                let bucket = if lower.contains("edit")
                    || lower.contains("write")
                    || lower.contains("patch")
                    || lower.contains("filechange")
                {
                    &mut edited
                } else {
                    &mut read_only
                };
                if !bucket.contains(&p) {
                    bucket.push(p);
                }
            }
        }
    }
    // A file that was both read and edited is a *modified* file — the two
    // buckets were filled independently, so drop from read-only anything that
    // also landed in edited (else it's mislabeled "not modified").
    read_only.retain(|p| !edited.contains(p));
    if !edited.is_empty() {
        out.push_str("\n## Files modified\n\n");
        for p in edited.iter().take(MAX_FILES) {
            out.push_str(&format!("- `{p}`\n"));
        }
        if edited.len() > MAX_FILES {
            out.push_str(&format!("- _…and {} more_\n", edited.len() - MAX_FILES));
        }
    }
    if !read_only.is_empty() {
        out.push_str("\n## Files read/inspected (not modified)\n\n");
        let cap = MAX_FILES.min(read_only.len());
        for p in read_only.iter().take(cap) {
            out.push_str(&format!("- `{p}`\n"));
        }
        if read_only.len() > MAX_FILES {
            out.push_str(&format!("- _…and {} more_\n", read_only.len() - MAX_FILES));
        }
    }

    // ── The tail of the exchange, newest-heavy, within a char budget ──
    // Walk backwards collecting rendered blocks until the budget runs out,
    // then emit them in timeline order. The last assistant message gets the
    // big cap — it's usually the "state of the world" summary.
    let mut blocks: Vec<String> = Vec::new();
    let mut spent = 0usize;
    let mut last_assistant_seen = false;
    // Only the most recent plan is signal — earlier revisions are superseded.
    let mut plan_seen = false;
    for item in items.iter().rev() {
        let block = match item {
            ConversationItem::UserMessage { text, .. } => {
                format!("**User:**\n{}\n", clip(text, RECENT_TEXT_CAP))
            }
            ConversationItem::AssistantText { text, .. } => {
                let cap = if last_assistant_seen {
                    RECENT_TEXT_CAP
                } else {
                    FINAL_ASSISTANT_CAP
                };
                last_assistant_seen = true;
                format!("**Agent:**\n{}\n", clip(text, cap))
            }
            ConversationItem::ToolUse { name, input, .. } => {
                format!("- [tool] {}\n", tool_line(name, input))
            }
            ConversationItem::ToolResult { is_error, .. } => {
                if *is_error {
                    "- [tool result] ERROR\n".to_string()
                } else {
                    continue; // successful results add bulk, not signal
                }
            }
            ConversationItem::Usage { .. } => continue,
            ConversationItem::SlashCommand { name, args, .. } => match args {
                Some(args) => format!("**User ran:** /{name} {}\n", clip(args, RECENT_TEXT_CAP)),
                None => format!("**User ran:** /{name}\n"),
            },
            // Local command output is console noise to a successor agent.
            ConversationItem::CommandOutput { .. } => continue,
            ConversationItem::Plan { steps, .. } => {
                // Reverse iteration → the first plan we hit is the latest; skip
                // the superseded earlier ones.
                if plan_seen || steps.is_empty() {
                    continue;
                }
                plan_seen = true;
                let lines = steps
                    .iter()
                    .map(|s| {
                        let mark = match s.status {
                            PlanStatus::Completed => "[x]",
                            PlanStatus::InProgress => "[~]",
                            PlanStatus::Pending => "[ ]",
                        };
                        format!("  {mark} {}", s.content)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("**Plan:**\n{lines}\n")
            }
        };
        if spent + block.len() > RECENT_BUDGET && !blocks.is_empty() {
            break;
        }
        spent += block.len();
        blocks.push(block);
    }
    if !blocks.is_empty() {
        out.push_str("\n## Recent exchange (oldest first, ends where the session left off)\n\n");
        for b in blocks.iter().rev() {
            out.push_str(b);
            out.push('\n');
        }
    }

    out.push_str("\n## Your job\n\n");
    out.push_str(
        "Continue this work in the working directory above. Verify anything you depend on \
                  with your own tools (read the files, run the checks) rather than trusting this \
                  summary blindly — it is lossy. If the last exchange left an explicit next step, \
                  do that first.\n",
    );
    out
}

/// `~/.workspacer/handoffs`, created on demand.
pub fn handoffs_dir() -> Option<PathBuf> {
    let dir = BaseDirs::new()?
        .home_dir()
        .join(".workspacer")
        .join("handoffs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Persist a brief; returns the absolute path. Filename is sortable and names
/// the source session so stale briefs are self-explanatory.
pub fn persist_brief(session_id: &str, markdown: &str) -> std::io::Result<PathBuf> {
    let dir = handoffs_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;
    let ts = OffsetDateTime::now_utc()
        .format(
            &time::format_description::parse("[year][month][day]-[hour][minute][second]")
                .expect("static format"),
        )
        .unwrap_or_default();
    let sid: String = session_id.chars().take(8).collect();
    let path = dir.join(format!("{ts}-{sid}.md"));
    std::fs::write(&path, markdown)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str) -> ConversationItem {
        ConversationItem::UserMessage {
            text: text.into(),
            timestamp: None,
        }
    }
    fn agent(text: &str) -> ConversationItem {
        ConversationItem::AssistantText {
            text: text.into(),
            timestamp: None,
        }
    }
    fn tool(name: &str, input: Value) -> ConversationItem {
        ConversationItem::ToolUse {
            id: "t1".into(),
            name: name.into(),
            input,
            timestamp: None,
        }
    }

    #[test]
    fn brief_carries_spine_files_and_tail() {
        let items = vec![
            user("fix the login bug"),
            tool("Edit", json!({ "file_path": "src/login.rs" })),
            tool("Read", json!({ "file_path": "src/auth.rs" })),
            agent("Fixed the null check in login.rs; tests pass."),
        ];
        let brief = build_brief("abc12345-xyz", None, &items);
        assert!(brief.contains("fix the login bug"));
        assert!(brief.contains("`src/login.rs`"), "edited file listed");
        assert!(brief.contains("Files modified"));
        assert!(brief.contains("`src/auth.rs`"), "read file listed");
        assert!(brief.contains("Fixed the null check"));
        assert!(brief.contains("abc12345-xyz"));
    }

    #[test]
    fn file_read_then_edited_is_not_listed_as_read_only() {
        // A file that was both Read and Edited must appear only under "Files
        // modified", never also under "read/inspected (not modified)".
        let items = vec![
            tool("Read", json!({ "file_path": "a.rs" })),
            tool("Edit", json!({ "file_path": "a.rs" })),
        ];
        let brief = build_brief("s", None, &items);
        assert!(
            brief.contains("Files modified"),
            "should be listed as modified"
        );
        assert!(brief.contains("`a.rs`"));
        assert!(
            !brief.contains("not modified"),
            "an edited file must not also appear as read-only:\n{brief}"
        );
    }

    #[test]
    fn edit_shaped_codex_tools_count_as_modified() {
        let items = vec![tool("fileChange", json!({ "path": "lib/a.ts" }))];
        let brief = build_brief("s", None, &items);
        assert!(brief.contains("Files modified"));
        assert!(brief.contains("`lib/a.ts`"));
    }

    #[test]
    fn long_final_assistant_message_gets_the_big_cap() {
        let long = "x".repeat(3_000);
        let items = vec![user("go"), agent(&long)];
        let brief = build_brief("s", None, &items);
        // Would be clipped at RECENT_TEXT_CAP if it weren't the final message.
        assert!(brief.contains(&"x".repeat(2_500)));
    }
}

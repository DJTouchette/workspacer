//! `claudemon init` — emit the hook configuration the user (or a future
//! version of this command) can merge into `~/.claude/settings.json`.
//!
//! Today this only prints the recommended JSON snippet. A real implementation
//! will load the existing settings file, deep-merge our hook entries (without
//! clobbering user hooks), and write it back atomically.

use anyhow::Result;
use serde_json::json;

const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "Notification",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "PermissionRequest",
];

pub async fn run(_dry_run: bool) -> Result<()> {
    let hooks: serde_json::Map<String, serde_json::Value> = HOOK_EVENTS
        .iter()
        .map(|name| {
            (
                name.to_string(),
                json!([{
                    "command": "curl -s -X POST http://127.0.0.1:7890/hook -H 'content-type: application/json' -d @-"
                }]),
            )
        })
        .collect();

    let snippet = json!({ "hooks": hooks });
    println!("# Add to ~/.claude/settings.json (merge by hand for now):\n");
    println!("{}", serde_json::to_string_pretty(&snippet)?);
    println!("\n# Future: `claudemon init` will merge this automatically.");
    Ok(())
}

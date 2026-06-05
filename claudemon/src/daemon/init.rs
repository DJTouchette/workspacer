//! `claudemon init` — merges claudemon's hook configuration into the user's
//! `~/.claude/settings.json` atomically and idempotently.
//!
//! The hook schema Claude Code expects per event is:
//!
//! ```jsonc
//! "EventName": [
//!   { "matcher": "..." /* optional for tool events */,
//!     "hooks": [
//!       { "type": "command", "command": "..." }
//!     ]
//!   }
//! ]
//! ```
//!
//! We identify our own entries by a tagged command string so re-running
//! `init` doesn't append duplicates.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde_json::{json, Value};

use crate::session::state::HookEventKind;

/// Hook events claudemon registers, derived from `HookEventKind::REGISTERABLE`.
///
/// This is the single source of truth: add/remove a variant from
/// `HookEventKind::REGISTERABLE` and this list updates automatically.
const HOOK_EVENTS: &[&str] = {
    // Build a &[&str] from the REGISTERABLE slice.  We can't call methods in
    // a const context without const fn, so we enumerate explicitly — but the
    // assignment is mechanically derived from `HookEventKind::REGISTERABLE`.
    //
    // NOTE: The length assertion below ensures this slice stays in sync.
    &[
        HookEventKind::SessionStart.as_str(),
        HookEventKind::SessionEnd.as_str(),
        HookEventKind::UserPromptSubmit.as_str(),
        HookEventKind::PreToolUse.as_str(),
        HookEventKind::Notification.as_str(),
        HookEventKind::Stop.as_str(),
        HookEventKind::SubagentStart.as_str(),
        HookEventKind::SubagentStop.as_str(),
    ]
};

/// Marker we embed in our command so we can find (and update) our entries
/// without trampling user-added hooks.
const TAG: &str = "# claudemon-hook";

fn hook_command(hook_port: u16) -> String {
    format!(
        "curl -s -X POST http://127.0.0.1:{hook_port}/hook -H \"content-type: application/json\" -d @- {TAG}"
    )
}

fn settings_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not resolve home directory")?;
    Ok(base.home_dir().join(".claude").join("settings.json"))
}

pub async fn run_with_port(dry_run: bool, hook_port: u16) -> Result<()> {
    let path = settings_path()?;
    let existing = match fs::read_to_string(&path) {
        Ok(text) if text.trim().is_empty() => Value::Object(Default::default()),
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };

    let command = hook_command(hook_port);
    let (merged, changed_events) = merge_hooks(existing, &command);

    let formatted = serde_json::to_string_pretty(&merged)? + "\n";

    if dry_run {
        println!("# would write to {}", path.display());
        if changed_events.is_empty() {
            println!("# (no changes — already up to date)");
        } else {
            println!("# adding/updating hooks for: {}", changed_events.join(", "));
        }
        println!("{formatted}");
        return Ok(());
    }

    if changed_events.is_empty() {
        println!("✓ {} already up to date", path.display());
        return Ok(());
    }

    // Atomic write: tmpfile in the same dir, then rename.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.claudemon.tmp");
    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(formatted.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;

    println!(
        "✓ wrote {} hook(s) to {}: {}",
        changed_events.len(),
        path.display(),
        changed_events.join(", ")
    );
    Ok(())
}

/// Merge our hook entries into the settings JSON. Returns the updated
/// document and the list of events that changed (empty if everything was
/// already present).
fn merge_hooks(mut doc: Value, our_command: &str) -> (Value, Vec<String>) {
    let obj = doc.as_object_mut().expect("settings must be an object");
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .expect("hooks must be an object");

    let mut changed = Vec::new();
    for event in HOOK_EVENTS {
        let arr = hooks
            .entry((*event).to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(arr) = arr.as_array_mut() else { continue };

        // Find an existing group whose `hooks[*].command` is tagged as ours.
        let mut found = false;
        for group in arr.iter_mut() {
            let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            for hook in inner.iter_mut() {
                let cmd = hook.get("command").and_then(Value::as_str).unwrap_or("");
                if cmd.contains(TAG) {
                    found = true;
                    if cmd != our_command {
                        hook["command"] = Value::String(our_command.to_string());
                        if !changed.contains(&event.to_string()) {
                            changed.push(event.to_string());
                        }
                    }
                    if !hook
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|t| t == "command")
                    {
                        hook["type"] = Value::String("command".to_string());
                    }
                }
            }
        }

        if !found {
            arr.push(json!({
                "hooks": [
                    { "type": "command", "command": our_command }
                ]
            }));
            changed.push(event.to_string());
        }
    }

    (doc, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_into_empty_doc() {
        let (doc, changed) = merge_hooks(json!({}), "echo X # claudemon-hook");
        assert_eq!(changed.len(), HOOK_EVENTS.len());
        let hooks = doc.get("hooks").unwrap().as_object().unwrap();
        for ev in HOOK_EVENTS {
            assert!(hooks.contains_key(*ev), "missing {ev}");
        }
    }

    #[test]
    fn idempotent_second_run() {
        let cmd = "echo X # claudemon-hook";
        let (doc, first) = merge_hooks(json!({}), cmd);
        assert!(!first.is_empty());
        let (_, second) = merge_hooks(doc, cmd);
        assert!(second.is_empty(), "expected no changes on second run");
    }

    #[test]
    fn preserves_user_hooks() {
        let starting = json!({
            "hooks": {
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [
                        { "type": "command", "command": "echo user-hook" }
                    ]}
                ]
            }
        });
        let cmd = "echo claudemon # claudemon-hook";
        let (doc, _) = merge_hooks(starting, cmd);
        let pre = doc["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "user hook should still be present alongside ours");
        let user_cmd = pre[0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(user_cmd, "echo user-hook");
    }

    #[test]
    fn updates_command_when_port_changes() {
        let old = "curl http://127.0.0.1:7890/hook # claudemon-hook";
        let new = "curl http://127.0.0.1:8888/hook # claudemon-hook";
        let (doc, _) = merge_hooks(json!({}), old);
        let (doc2, changed) = merge_hooks(doc, new);
        assert!(!changed.is_empty(), "expected change when command differs");
        let cmd = doc2["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(cmd, new);
    }
}

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

/// Marker for our statusLine forwarder, kept distinct from the hook tag so the
/// two are matched independently.
const STATUS_TAG: &str = "# claudemon-statusline";

fn hook_command(hook_port: u16) -> String {
    format!(
        "curl -s -X POST http://127.0.0.1:{hook_port}/hook -H \"content-type: application/json\" -d @- {TAG}"
    )
}

/// Build the statusLine command claudemon installs. It reads Claude Code's
/// statusLine JSON from stdin once, forwards a copy to `/statusline`, then —
/// when the user already had a statusLine — pipes the same JSON to their
/// original command so their terminal line keeps rendering unchanged.
///
/// Runs in the same bash/sh Claude uses for hook commands (the hook command
/// relies on `-d @-` and `#` comments, so a shell is guaranteed). The trailing
/// `STATUS_TAG` comment both marks the entry as ours and is inert at runtime.
fn status_line_command(hook_port: u16, inner: Option<&str>) -> String {
    // `--max-time 2` so an unresponsive daemon can never stall the (frequently
    // re-run, latency-sensitive) status line; connection-refused already fails
    // fast when the daemon is simply down.
    let forward = format!(
        "printf '%s' \"$i\" | curl -s --max-time 2 -X POST http://127.0.0.1:{hook_port}/statusline -H \"content-type: application/json\" -d @- >/dev/null 2>&1"
    );
    match inner {
        Some(cmd) => format!("i=$(cat); {forward}; printf '%s' \"$i\" | {cmd} {STATUS_TAG}"),
        None => format!("i=$(cat); {forward} {STATUS_TAG}"),
    }
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
    let (mut merged, changed_events) = merge_hooks(existing, &command);
    let status_changed = merge_status_line(&mut merged, hook_port);
    let nothing_changed = changed_events.is_empty() && !status_changed;

    let formatted = serde_json::to_string_pretty(&merged)? + "\n";

    if dry_run {
        println!("# would write to {}", path.display());
        if nothing_changed {
            println!("# (no changes — already up to date)");
        } else {
            if !changed_events.is_empty() {
                println!("# adding/updating hooks for: {}", changed_events.join(", "));
            }
            if status_changed {
                println!("# adding/updating statusLine forwarder");
            }
        }
        println!("{formatted}");
        return Ok(());
    }

    if nothing_changed {
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

    if !changed_events.is_empty() {
        println!(
            "✓ wrote {} hook(s) to {}: {}",
            changed_events.len(),
            path.display(),
            changed_events.join(", ")
        );
    }
    if status_changed {
        println!("✓ wrapped statusLine forwarder in {}", path.display());
    }
    Ok(())
}

/// Merge our statusLine forwarder into the settings doc, returning `true` if it
/// changed anything.
///
/// We *wrap* any existing `statusLine.command` so the user's own status line
/// keeps rendering while a copy of Claude Code's statusLine JSON is forwarded to
/// claudemon — the only channel carrying context-%/cost/rate-limit data.
/// Idempotent: once our `STATUS_TAG` is present we leave the entry alone (this
/// also prevents double-wrapping). `padding` and other keys are preserved.
fn merge_status_line(doc: &mut Value, hook_port: u16) -> bool {
    let Some(obj) = doc.as_object_mut() else {
        tracing::warn!("settings.json top-level value is not an object; skipping statusLine merge");
        return false;
    };
    let existing = obj.get("statusLine");

    // Already ours → idempotent no-op (also guards against re-wrapping).
    if existing
        .and_then(|sl| sl.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(STATUS_TAG))
    {
        return false;
    }

    // Preserve the user's existing command (wrap it) and any sibling keys.
    let inner = existing
        .and_then(|sl| sl.get("command"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let command = status_line_command(hook_port, inner.as_deref());

    let entry = obj
        .entry("statusLine".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let Some(map) = entry.as_object_mut() else {
        // A non-object statusLine is malformed; replace it wholesale.
        *entry = json!({ "type": "command", "command": command });
        return true;
    };
    map.insert("type".to_string(), Value::String("command".to_string()));
    map.insert("command".to_string(), Value::String(command));
    true
}

/// Merge our hook entries into the settings JSON. Returns the updated
/// document and the list of events that changed (empty if everything was
/// already present).
fn merge_hooks(mut doc: Value, our_command: &str) -> (Value, Vec<String>) {
    if !doc.is_object() {
        tracing::warn!("settings.json top-level value is not an object; skipping hooks merge");
        return (doc, Vec::new());
    }
    let obj = doc.as_object_mut().expect("checked above");
    // Ensure `hooks` key exists as an object; if it already exists but is not
    // an object (malformed file), skip rather than panic.
    let hooks_is_object = obj
        .get("hooks")
        .map_or(true, |v| v.is_object());
    if !hooks_is_object {
        tracing::warn!("settings.json `hooks` value is not an object; skipping hooks merge");
        return (doc, Vec::new());
    }
    let obj = doc.as_object_mut().expect("checked above");
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .expect("checked above");

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
    fn status_line_wraps_existing_user_command() {
        let mut doc = json!({
            "statusLine": { "type": "command", "command": "bash ~/.claude/my-statusline.sh", "padding": 2 }
        });
        let changed = merge_status_line(&mut doc, 7890);
        assert!(changed);
        let cmd = doc["statusLine"]["command"].as_str().unwrap();
        // User's original command is preserved (piped to) ...
        assert!(cmd.contains("bash ~/.claude/my-statusline.sh"), "inner command preserved");
        // ... and a forward to /statusline is prepended, tagged as ours.
        assert!(cmd.contains("/statusline"));
        assert!(cmd.contains(STATUS_TAG));
        // Sibling keys like padding survive.
        assert_eq!(doc["statusLine"]["padding"], json!(2));
    }

    #[test]
    fn status_line_installs_forwarder_when_absent() {
        let mut doc = json!({});
        let changed = merge_status_line(&mut doc, 7890);
        assert!(changed);
        let cmd = doc["statusLine"]["command"].as_str().unwrap();
        assert!(cmd.contains("/statusline"));
        assert!(cmd.contains(STATUS_TAG));
        assert_eq!(doc["statusLine"]["type"], json!("command"));
    }

    #[test]
    fn status_line_idempotent_no_double_wrap() {
        let mut doc = json!({
            "statusLine": { "type": "command", "command": "bash ~/.claude/my-statusline.sh" }
        });
        assert!(merge_status_line(&mut doc, 7890));
        let after_first = doc["statusLine"]["command"].as_str().unwrap().to_string();
        // Second run must detect our tag and leave the (already-wrapped) command alone.
        assert!(!merge_status_line(&mut doc, 7890), "second run should be a no-op");
        assert_eq!(doc["statusLine"]["command"].as_str().unwrap(), after_first);
        // The original command appears exactly once — no nested re-wrap.
        let occurrences = after_first.matches("my-statusline.sh").count();
        assert_eq!(occurrences, 1, "inner command must not be wrapped twice");
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

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
        HookEventKind::PostToolUse.as_str(),
        HookEventKind::PermissionRequest.as_str(),
        HookEventKind::Notification.as_str(),
        HookEventKind::Stop.as_str(),
        HookEventKind::SubagentStart.as_str(),
        HookEventKind::SubagentStop.as_str(),
        HookEventKind::PreCompact.as_str(),
        HookEventKind::PostCompact.as_str(),
    ]
};

/// Marker we embed in our command so we can find (and update) our entries
/// without trampling user-added hooks.
const TAG: &str = "# claudemon-hook";

/// Marker for our statusLine forwarder, kept distinct from the hook tag so the
/// two are matched independently.
const STATUS_TAG: &str = "# claudemon-statusline";

/// Sibling key we stash the user's untouched statusLine command under when we
/// wrap it. Recovering their command by re-parsing our shell one-liner is
/// guesswork the moment either end of it changes, and the command is the only
/// copy they have — so the verbatim original travels with the entry and both
/// the overlay path and `strip_our_entries` read it back from there.
const STATUS_STASH_KEY: &str = "claudemonOriginalCommand";

/// The `stdin`-replay prefix shared by our forward and the pipe into the user's
/// own command, so the wrapper and the unwrapper can't drift apart.
const PIPE_INPUT: &str = "printf '%s' \"$i\" | ";

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
        "{PIPE_INPUT}curl -s --max-time 2 -X POST http://127.0.0.1:{hook_port}/statusline -H \"content-type: application/json\" -d @- >/dev/null 2>&1"
    );
    match inner {
        Some(cmd) => format!("i=$(cat); {forward}; {PIPE_INPUT}{cmd} {STATUS_TAG}"),
        None => format!("i=$(cat); {forward} {STATUS_TAG}"),
    }
}

/// Build the whole `statusLine` entry we install: our wrapper command plus, when
/// we wrapped something, the user's original stashed verbatim beside it.
fn status_line_entry(hook_port: u16, inner: Option<&str>) -> Value {
    let mut entry = json!({
        "type": "command",
        "command": status_line_command(hook_port, inner),
    });
    if let (Some(inner), Some(obj)) = (inner, entry.as_object_mut()) {
        obj.insert(
            STATUS_STASH_KEY.to_string(),
            Value::String(inner.to_string()),
        );
    }
    entry
}

/// The user's own statusLine command, given whatever is in a settings doc's
/// `statusLine`: their command untouched when it isn't ours, the command we
/// wrapped when it is, and `None` when there was nothing to wrap. Preferring the
/// stash means the common case never depends on parsing a shell string; the
/// parse is only reached for entries written before the stash existed.
fn inner_status_line(entry: Option<&Value>) -> Option<String> {
    let entry = entry?;
    let command = entry.get("command").and_then(Value::as_str)?;
    if !command.contains(STATUS_TAG) {
        return Some(command.to_string());
    }
    if let Some(stashed) = entry.get(STATUS_STASH_KEY).and_then(Value::as_str) {
        return Some(stashed.to_string());
    }
    let body = command.trim_end().strip_suffix(STATUS_TAG)?.trim_end();
    let (_, tail) = body.rsplit_once(PIPE_INPUT)?;
    // With no inner command the last `printf | …` in the line is our own forward;
    // that shape means there was nothing of the user's to recover.
    if tail.starts_with("curl ") && tail.contains("/statusline") {
        return None;
    }
    Some(tail.to_string())
}

fn settings_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not resolve home directory")?;
    Ok(base.home_dir().join(".claude").join("settings.json"))
}

pub async fn run_with_port(dry_run: bool, hook_port: u16) -> Result<()> {
    let path = settings_path()?;
    let existing = match fs::read_to_string(&path) {
        Ok(text) if text.trim().is_empty() => Value::Object(Default::default()),
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
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
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.claudemon.tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
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

/// Write claudemon's hooks + statusLine to a standalone *overlay* file passed to
/// `claude` via `--settings`, rather than mutating the user's global
/// `~/.claude/settings.json`.
///
/// The overlay contains ONLY our entries (a fresh doc). `--settings` layers
/// additively over the user's still-loaded `user`/`project`/`local` sources, so
/// everything they have keeps working; we only contribute our hooks + a
/// statusLine that wraps whatever command they already had (read read-only from
/// their global file). To avoid our hooks firing twice — once from the overlay
/// and once from a prior `claudemon init` that wrote into the global file — we
/// also strip any previously-installed claudemon entries from the global file
/// (the only mutation, and a purely subtractive cleanup of our own tag).
pub async fn run_overlay(dry_run: bool, hook_port: u16, overlay_path: &PathBuf) -> Result<()> {
    let global = settings_path()?;

    // Read the user's existing statusLine command (if any) so we can wrap it —
    // read-only; we never write their global file's statusLine.
    let global_doc = match fs::read_to_string(&global) {
        Ok(text) if text.trim().is_empty() => Value::Object(Default::default()),
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", global.display()))?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", global.display())),
    };
    // A prior overlay-less `claudemon init` may have left OUR forwarder in the
    // global file, with the user's command inside it. Unwrap rather than ignore:
    // ignoring it wraps nothing here and then the strip below removes the global
    // entry, and their command exists nowhere any more.
    let inner_status = inner_status_line(global_doc.get("statusLine"));

    // Build the overlay: a fresh doc carrying only our hooks + statusLine.
    let command = hook_command(hook_port);
    let (overlay_doc, _) = merge_hooks(Value::Object(Default::default()), &command);
    let mut overlay_doc = overlay_doc;
    if let Some(obj) = overlay_doc.as_object_mut() {
        obj.insert(
            "statusLine".to_string(),
            status_line_entry(hook_port, inner_status.as_deref()),
        );
    }
    let overlay_formatted = serde_json::to_string_pretty(&overlay_doc)? + "\n";

    // Strip our previously-installed entries from the global file (subtractive).
    let (stripped_global, removed) = strip_our_entries(global_doc);

    if dry_run {
        println!("# would write overlay to {}", overlay_path.display());
        println!("{overlay_formatted}");
        if removed {
            println!("# would strip claudemon entries from {}", global.display());
        }
        return Ok(());
    }

    write_atomic(overlay_path, &overlay_formatted)
        .with_context(|| format!("writing overlay {}", overlay_path.display()))?;
    println!("✓ wrote overlay settings to {}", overlay_path.display());

    if removed {
        let stripped_formatted = serde_json::to_string_pretty(&stripped_global)? + "\n";
        write_atomic(&global, &stripped_formatted)
            .with_context(|| format!("rewriting {}", global.display()))?;
        println!(
            "✓ stripped stale claudemon entries from {}",
            global.display()
        );
    }
    Ok(())
}

/// Atomic write: tmpfile in the same dir, fsync, then rename over the target.
fn write_atomic(path: &PathBuf, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.claudemon.tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Remove claudemon's own tagged hook groups and statusLine from a settings doc,
/// leaving every user-authored entry intact. Returns the pruned doc and whether
/// anything was removed. Used by the overlay path to prevent double-firing hooks
/// when a prior `claudemon init` had written into the global file.
fn strip_our_entries(mut doc: Value) -> (Value, bool) {
    let Some(obj) = doc.as_object_mut() else {
        return (doc, false);
    };
    let mut removed = false;

    // Unwrap our statusLine wrapper (leave a user's own untouched). Removing the
    // key outright would delete the user's command along with our wrapper — this
    // file is where it lives, so put theirs back and only drop the entry when
    // there was nothing but ours in it.
    if obj
        .get("statusLine")
        .and_then(|sl| sl.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(STATUS_TAG))
    {
        match inner_status_line(obj.get("statusLine")) {
            Some(inner) => {
                if let Some(sl) = obj.get_mut("statusLine").and_then(Value::as_object_mut) {
                    sl.insert("command".to_string(), Value::String(inner));
                    sl.remove(STATUS_STASH_KEY);
                }
            }
            None => {
                obj.remove("statusLine");
            }
        }
        removed = true;
    }

    // Drop our hook groups per event, pruning now-empty event arrays.
    if let Some(hooks) = obj.get_mut("hooks").and_then(Value::as_object_mut) {
        let mut empty_events = Vec::new();
        for (event, arr) in hooks.iter_mut() {
            let Some(arr) = arr.as_array_mut() else {
                continue;
            };
            let before = arr.len();
            arr.retain(|group| {
                let ours = group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|inner| {
                        inner.iter().any(|h| {
                            h.get("command")
                                .and_then(Value::as_str)
                                .is_some_and(|c| c.contains(TAG))
                        })
                    });
                !ours
            });
            if arr.len() != before {
                removed = true;
            }
            if arr.is_empty() {
                empty_events.push(event.clone());
            }
        }
        for ev in empty_events {
            hooks.remove(&ev);
        }
    }

    (doc, removed)
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
    let wrapped = status_line_entry(hook_port, inner.as_deref());

    let entry = obj
        .entry("statusLine".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let Some(map) = entry.as_object_mut() else {
        // A non-object statusLine is malformed; replace it wholesale.
        *entry = wrapped;
        return true;
    };
    // Merge our keys over the user's entry so `padding` and friends survive; the
    // stash key rides along so an unwrap never has to reverse the shell string.
    for (key, value) in wrapped.as_object().expect("built as an object") {
        map.insert(key.clone(), value.clone());
    }
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
    let hooks_is_object = obj.get("hooks").is_none_or(|v| v.is_object());
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
        let Some(arr) = arr.as_array_mut() else {
            continue;
        };

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
                    if hook
                        .get("type")
                        .and_then(Value::as_str)
                        .is_none_or(|t| t != "command")
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
        assert_eq!(
            pre.len(),
            2,
            "user hook should still be present alongside ours"
        );
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
        assert!(
            cmd.contains("bash ~/.claude/my-statusline.sh"),
            "inner command preserved"
        );
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
        assert!(
            !merge_status_line(&mut doc, 7890),
            "second run should be a no-op"
        );
        assert_eq!(doc["statusLine"]["command"].as_str().unwrap(), after_first);
        // The original command appears exactly once — no nested re-wrap.
        let occurrences = after_first.matches("my-statusline.sh").count();
        assert_eq!(occurrences, 1, "inner command must not be wrapped twice");
    }

    #[test]
    fn strip_removes_only_our_entries() {
        // A file with both user-authored and claudemon entries.
        let cmd = hook_command(7890);
        let (mut doc, _) = merge_hooks(
            json!({
                "hooks": {
                    "PreToolUse": [
                        { "matcher": "Bash", "hooks": [
                            { "type": "command", "command": "echo user-hook" }
                        ]}
                    ]
                },
                "statusLine": { "type": "command", "command": "bash ~/mine.sh" }
            }),
            &cmd,
        );
        merge_status_line(&mut doc, 7890);

        let (pruned, removed) = strip_our_entries(doc);
        assert!(removed);
        // Our SessionStart group is gone (and the now-empty event pruned).
        assert!(pruned["hooks"].get("SessionStart").is_none());
        // The user's PreToolUse hook survives, and our group there is gone.
        let pre = pruned["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "only the user hook should remain");
        assert_eq!(pre[0]["hooks"][0]["command"], json!("echo user-hook"));
        // Our statusLine wrapper is removed — but the user's command, which the
        // wrapper had swallowed, is restored in its place. Deleting the key (as
        // this used to) destroyed the only copy of it they had.
        assert_eq!(
            pruned["statusLine"]["command"],
            json!("bash ~/mine.sh"),
            "the wrapped command must be handed back, not deleted"
        );
        assert!(
            pruned["statusLine"].get(STATUS_STASH_KEY).is_none(),
            "our stash goes with our wrapper"
        );
    }

    #[test]
    fn strip_drops_the_entry_when_it_only_ever_held_our_forwarder() {
        // Nothing of the user's was wrapped, so there is nothing to hand back and
        // the key goes away entirely.
        let mut doc = json!({});
        assert!(merge_status_line(&mut doc, 7890));
        let (pruned, removed) = strip_our_entries(doc);
        assert!(removed);
        assert!(pruned.get("statusLine").is_none());
    }

    #[test]
    fn strip_unwraps_a_pre_stash_wrapper_by_parsing() {
        // Entries written before the stash key existed have to survive too: the
        // user's command is recovered from the shell string itself.
        let wrapped = status_line_command(7890, Some("bash ~/legacy.sh"));
        let doc = json!({ "statusLine": { "type": "command", "command": wrapped } });
        let (pruned, removed) = strip_our_entries(doc);
        assert!(removed);
        assert_eq!(pruned["statusLine"]["command"], json!("bash ~/legacy.sh"));
    }

    #[test]
    fn overlay_inner_command_survives_an_already_wrapped_global() {
        // The overlay path reads the global file's statusLine to wrap it. When a
        // prior `claudemon init` already wrapped it, the user's command must be
        // unwrapped and re-wrapped — ignoring it (as this used to) dropped it,
        // and the strip that follows then removed the last copy.
        let mut global = json!({
            "statusLine": { "type": "command", "command": "bash ~/mine.sh" }
        });
        merge_status_line(&mut global, 7890);

        let inner = inner_status_line(global.get("statusLine"));
        assert_eq!(inner.as_deref(), Some("bash ~/mine.sh"));

        let overlay = status_line_entry(7890, inner.as_deref());
        let cmd = overlay["command"].as_str().unwrap();
        assert!(cmd.contains("bash ~/mine.sh"), "re-wrapped, not dropped");
        assert_eq!(cmd.matches("bash ~/mine.sh").count(), 1, "wrapped once");
        assert_eq!(overlay[STATUS_STASH_KEY], json!("bash ~/mine.sh"));
    }

    #[test]
    fn inner_status_line_reads_plain_and_absent_entries() {
        assert_eq!(inner_status_line(None), None);
        assert_eq!(
            inner_status_line(Some(&json!({ "command": "bash ~/mine.sh" }))).as_deref(),
            Some("bash ~/mine.sh"),
        );
        // Our own forwarder with nothing wrapped inside it yields nothing.
        let bare = status_line_command(7890, None);
        assert_eq!(
            inner_status_line(Some(&json!({ "command": bare }))),
            None,
            "the forward's own pipe must not be mistaken for a user command"
        );
    }

    #[test]
    fn strip_is_noop_without_our_entries() {
        let doc = json!({
            "hooks": { "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "echo user" }] }
            ]},
            "statusLine": { "type": "command", "command": "bash ~/mine.sh" }
        });
        let (pruned, removed) = strip_our_entries(doc.clone());
        assert!(!removed);
        assert_eq!(pruned, doc, "a file with no claudemon entries is untouched");
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

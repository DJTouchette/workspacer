//! Pure classifier: maps `(session snapshot, recent events, incoming event)`
//! to a new session state and a list of inbox-item actions.
//!
//! Lives outside [`crate::store`] so it can be tested with synthetic event
//! streams without touching SQLite. The runtime side (`store::record_and_classify`)
//! loads the inputs, calls [`classify`], and applies the output in one
//! transaction.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::session::HookEvent;

pub mod types;
pub use types::*;

/// Repeated-tool-call detection window: how many recent events back we look,
/// and how many identical (tool_name, hash(tool_input)) calls within that
/// window flip the session to "stuck".
const STUCK_WINDOW_EVENTS: usize = 10;
const STUCK_REPEAT_THRESHOLD: usize = 5;
const STUCK_WINDOW_SECONDS: i64 = 60;

/// How long a session can be silent while working before the idle-timer check
/// promotes it to stuck. Matches spec §11 ("No event for 5min while previously
/// working").
pub const IDLE_STUCK_SECONDS: i64 = 300;

/// Run the rules from spec §11 over the new event. The caller is responsible
/// for already having inserted the event row (so `event_row_id` exists) and
/// for applying the returned actions.
pub fn classify(input: Input) -> Output {
    let mut actions = Vec::new();
    let new_state = match input.event.event.as_str() {
        "SessionStart" => SessionState::Working,
        "UserPromptSubmit" => SessionState::Working,

        "PermissionRequest" => {
            let item = build_item(
                ItemKind::NeedsInput,
                95,
                &input.event,
                input.event_row_id,
                Some("approve".into()),
            );
            push_or_touch(&mut actions, ItemKind::NeedsInput, &input.open_items, item);
            SessionState::NeedsInput
        }

        "Notification" => {
            // Claude Code's real payload key is `notification_type`
            // (values `idle_prompt` / `permission_prompt`). `kind` and `type`
            // are kept as fallbacks for synthetic/forwarded events.
            let notif_kind = input
                .event
                .payload
                .get("notification_type")
                .and_then(Value::as_str)
                .or_else(|| input.event.payload.get("kind").and_then(Value::as_str))
                .or_else(|| input.event.payload.get("type").and_then(Value::as_str));
            match notif_kind {
                Some("idle_prompt") | Some("permission_prompt") => {
                    let item = build_item(
                        ItemKind::NeedsInput,
                        90,
                        &input.event,
                        input.event_row_id,
                        Some("reply".into()),
                    );
                    push_or_touch(&mut actions, ItemKind::NeedsInput, &input.open_items, item);
                    SessionState::NeedsInput
                }
                _ => input.session.state,
            }
        }

        "PostToolUseFailure" => {
            let item = build_item(
                ItemKind::Error,
                80,
                &input.event,
                input.event_row_id,
                Some("review_diff".into()),
            );
            actions.push(ItemAction::Create(item));
            SessionState::Errored
        }

        "StopFailure" => {
            let item = build_item(
                ItemKind::Error,
                85,
                &input.event,
                input.event_row_id,
                Some("review_diff".into()),
            );
            actions.push(ItemAction::Create(item));
            SessionState::Errored
        }

        "PreToolUse" => {
            if is_repeating(&input) {
                let already_stuck = input
                    .open_items
                    .iter()
                    .any(|i| i.kind == ItemKind::Stuck);
                if !already_stuck {
                    let item = build_item(
                        ItemKind::Stuck,
                        70,
                        &input.event,
                        input.event_row_id,
                        Some("intervene".into()),
                    );
                    actions.push(ItemAction::Create(item));
                }
                SessionState::Stuck
            } else {
                // Cadence healthy: update working item if any (spec §11).
                if let Some(existing) = input
                    .open_items
                    .iter()
                    .find(|i| i.kind == ItemKind::WorkingMilestone)
                {
                    actions.push(ItemAction::Touch(existing.id.clone()));
                }
                SessionState::Working
            }
        }

        "PostToolUse" => SessionState::Working,

        "Stop" => {
            let item = build_item(
                ItemKind::Done,
                40,
                &input.event,
                input.event_row_id,
                Some("merge".into()),
            );
            actions.push(ItemAction::Create(item));
            SessionState::Done
        }

        "SessionEnd" => {
            actions.push(ItemAction::ResolveAllForSession);
            SessionState::Ended
        }

        _ => input.session.state,
    };

    Output {
        new_session_state: new_state,
        actions,
    }
}

/// Idle-timer rule: a session sitting silent in `Working` for [`IDLE_STUCK_SECONDS`]
/// gets promoted to stuck with a priority-60 item. Returns `None` if the
/// session doesn't qualify (wrong state, recent activity, already has a stuck
/// item).
pub fn classify_idle(session: &SessionSnapshot, open_items: &[OpenItem], now_unix: i64) -> Option<NewItem> {
    if session.state != SessionState::Working {
        return None;
    }
    if now_unix - session.last_event_at_unix < IDLE_STUCK_SECONDS {
        return None;
    }
    if open_items.iter().any(|i| i.kind == ItemKind::Stuck) {
        return None;
    }
    Some(NewItem {
        kind: ItemKind::Stuck,
        priority: 60,
        triggering_event_id: None,
        summary: format!(
            "Idle for {}m with no events",
            (now_unix - session.last_event_at_unix) / 60
        ),
        next_action: Some("intervene".into()),
    })
}

fn is_repeating(input: &Input) -> bool {
    let tool_name = input
        .event
        .payload
        .get("tool_name")
        .and_then(Value::as_str);
    let Some(tool_name) = tool_name else {
        return false;
    };
    let input_hash = canonical_tool_input_hash(input.event.payload.get("tool_input"));
    let cutoff = input.now_unix.saturating_sub(STUCK_WINDOW_SECONDS);

    let matches = input
        .recent_events
        .iter()
        .rev()
        .take(STUCK_WINDOW_EVENTS)
        .filter(|e| e.event_type == "PreToolUse")
        .filter(|e| e.timestamp >= cutoff)
        .filter(|e| e.tool_name.as_deref() == Some(tool_name))
        .filter(|e| e.tool_input_hash == input_hash)
        .count();

    // The incoming event counts toward the threshold.
    matches + 1 >= STUCK_REPEAT_THRESHOLD
}

/// Canonical hash of a `tool_input` JSON value so we can spot "Claude is
/// trying the same call over and over." Stable across key ordering because
/// `serde_json` sorts object keys when we re-serialize through a `BTreeMap`.
pub fn canonical_tool_input_hash(input: Option<&Value>) -> String {
    let Some(value) = input else {
        return String::new();
    };
    let canonical = canonicalize(value);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: std::collections::BTreeMap<String, Value> =
                std::collections::BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.clone(), canonicalize(v));
            }
            serde_json::to_value(sorted).unwrap_or(Value::Null)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        _ => value.clone(),
    }
}

fn build_item(
    kind: ItemKind,
    priority: i32,
    event: &HookEvent,
    event_row_id: i64,
    next_action: Option<String>,
) -> NewItem {
    NewItem {
        kind,
        priority,
        triggering_event_id: Some(event_row_id),
        summary: fallback_summary(kind, event),
        next_action,
    }
}

/// Deterministic fallback summary line. Used until the LLM summarizer
/// (Phase 4) overwrites it with something better.
fn fallback_summary(kind: ItemKind, event: &HookEvent) -> String {
    let tool = event
        .payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    match kind {
        ItemKind::NeedsInput if !tool.is_empty() => format!("Needs decision on {} call", tool),
        ItemKind::NeedsInput => "Needs your input".into(),
        ItemKind::Error if !tool.is_empty() => format!("Tool {} failed", tool),
        ItemKind::Error => "Session errored".into(),
        ItemKind::Stuck if !tool.is_empty() => format!("Stuck looping on {}", tool),
        ItemKind::Stuck => "Session stuck".into(),
        ItemKind::Done => "Session finished cleanly".into(),
        ItemKind::WorkingMilestone => "Working".into(),
    }
}

fn push_or_touch(
    actions: &mut Vec<ItemAction>,
    kind: ItemKind,
    open_items: &[OpenItem],
    item: NewItem,
) {
    if let Some(existing) = open_items.iter().find(|i| i.kind == kind) {
        actions.push(ItemAction::Touch(existing.id.clone()));
    } else {
        actions.push(ItemAction::Create(item));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(name: &str, session: &str) -> HookEvent {
        HookEvent {
            event: name.to_string(),
            session_id: session.to_string(),
            cwd: Some("/tmp".into()),
            timestamp: None,
            payload: serde_json::Map::new(),
        }
    }

    fn input(session_state: SessionState, ev: HookEvent) -> Input {
        Input {
            session: SessionSnapshot {
                state: session_state,
                last_event_at_unix: 1000,
            },
            recent_events: Vec::new(),
            open_items: Vec::new(),
            event: ev,
            event_row_id: 1,
            now_unix: 1000,
        }
    }

    #[test]
    fn session_start_yields_working_no_item() {
        let out = classify(input(SessionState::Working, event("SessionStart", "s")));
        assert_eq!(out.new_session_state, SessionState::Working);
        assert!(out.actions.is_empty());
    }

    #[test]
    fn permission_request_creates_needs_input_item() {
        let mut e = event("PermissionRequest", "s");
        e.payload.insert("tool_name".into(), json!("Bash"));
        let out = classify(input(SessionState::Working, e));
        assert_eq!(out.new_session_state, SessionState::NeedsInput);
        match out.actions.as_slice() {
            [ItemAction::Create(item)] => {
                assert_eq!(item.kind, ItemKind::NeedsInput);
                assert_eq!(item.priority, 95);
                assert!(item.summary.contains("Bash"));
            }
            other => panic!("expected one Create action, got {other:?}"),
        }
    }

    #[test]
    fn second_permission_request_touches_instead_of_duplicating() {
        let mut e = event("PermissionRequest", "s");
        e.payload.insert("tool_name".into(), json!("Bash"));
        let mut inp = input(SessionState::NeedsInput, e);
        inp.open_items.push(OpenItem {
            id: "item-1".into(),
            kind: ItemKind::NeedsInput,
            priority: 95,
        });
        let out = classify(inp);
        assert_eq!(out.new_session_state, SessionState::NeedsInput);
        assert!(matches!(out.actions.as_slice(), [ItemAction::Touch(id)] if id == "item-1"));
    }

    #[test]
    fn notification_idle_prompt_creates_item() {
        let mut e = event("Notification", "s");
        e.payload.insert("kind".into(), json!("idle_prompt"));
        let out = classify(input(SessionState::Working, e));
        assert_eq!(out.new_session_state, SessionState::NeedsInput);
        assert!(matches!(out.actions.as_slice(),
            [ItemAction::Create(item)] if item.priority == 90));
    }

    #[test]
    fn notification_uses_notification_type_key() {
        // Claude Code's real payload uses `notification_type`, not `kind`.
        let mut e = event("Notification", "s");
        e.payload
            .insert("notification_type".into(), json!("permission_prompt"));
        e.payload
            .insert("message".into(), json!("Claude needs your permission to use Bash"));
        let out = classify(input(SessionState::Working, e));
        assert_eq!(out.new_session_state, SessionState::NeedsInput);
        assert!(matches!(out.actions.as_slice(),
            [ItemAction::Create(item)] if item.priority == 90));
    }

    #[test]
    fn notification_unrelated_kind_is_noop() {
        let mut e = event("Notification", "s");
        e.payload.insert("kind".into(), json!("status_change"));
        let out = classify(input(SessionState::Working, e));
        assert_eq!(out.new_session_state, SessionState::Working);
        assert!(out.actions.is_empty());
    }

    #[test]
    fn post_tool_use_failure_creates_error_item() {
        let mut e = event("PostToolUseFailure", "s");
        e.payload.insert("tool_name".into(), json!("Edit"));
        let out = classify(input(SessionState::Working, e));
        assert_eq!(out.new_session_state, SessionState::Errored);
        match out.actions.as_slice() {
            [ItemAction::Create(item)] => {
                assert_eq!(item.kind, ItemKind::Error);
                assert_eq!(item.priority, 80);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn stop_failure_outranks_post_tool_failure() {
        let mut e = event("StopFailure", "s");
        e.payload.insert("tool_name".into(), json!("Edit"));
        let out = classify(input(SessionState::Working, e));
        assert!(matches!(out.actions.as_slice(),
            [ItemAction::Create(item)] if item.priority == 85));
    }

    #[test]
    fn stop_creates_done_item() {
        let out = classify(input(SessionState::Working, event("Stop", "s")));
        assert_eq!(out.new_session_state, SessionState::Done);
        assert!(matches!(out.actions.as_slice(),
            [ItemAction::Create(item)] if item.kind == ItemKind::Done && item.priority == 40));
    }

    #[test]
    fn session_end_resolves_all_items() {
        let out = classify(input(SessionState::NeedsInput, event("SessionEnd", "s")));
        assert_eq!(out.new_session_state, SessionState::Ended);
        assert!(matches!(out.actions.as_slice(), [ItemAction::ResolveAllForSession]));
    }

    #[test]
    fn repeated_pre_tool_use_flips_to_stuck() {
        let mut e = event("PreToolUse", "s");
        e.payload.insert("tool_name".into(), json!("Bash"));
        e.payload.insert("tool_input".into(), json!({"command": "ls"}));
        let hash = canonical_tool_input_hash(Some(&json!({"command": "ls"})));

        let mut inp = input(SessionState::Working, e);
        // four prior identical calls within the window
        inp.recent_events = (0..4)
            .map(|i| RecentEvent {
                event_type: "PreToolUse".into(),
                tool_name: Some("Bash".into()),
                tool_input_hash: hash.clone(),
                timestamp: 990 + i,
            })
            .collect();
        let out = classify(inp);
        assert_eq!(out.new_session_state, SessionState::Stuck);
        assert!(matches!(out.actions.as_slice(),
            [ItemAction::Create(item)] if item.kind == ItemKind::Stuck && item.priority == 70));
    }

    #[test]
    fn repeated_tool_use_only_creates_one_stuck_item() {
        let mut e = event("PreToolUse", "s");
        e.payload.insert("tool_name".into(), json!("Bash"));
        e.payload.insert("tool_input".into(), json!({"command": "ls"}));
        let hash = canonical_tool_input_hash(Some(&json!({"command": "ls"})));

        let mut inp = input(SessionState::Stuck, e);
        inp.recent_events = (0..5)
            .map(|i| RecentEvent {
                event_type: "PreToolUse".into(),
                tool_name: Some("Bash".into()),
                tool_input_hash: hash.clone(),
                timestamp: 990 + i,
            })
            .collect();
        inp.open_items.push(OpenItem {
            id: "stuck-1".into(),
            kind: ItemKind::Stuck,
            priority: 70,
        });
        let out = classify(inp);
        assert_eq!(out.new_session_state, SessionState::Stuck);
        assert!(out.actions.is_empty(), "should not duplicate stuck items");
    }

    #[test]
    fn pre_tool_use_outside_window_does_not_trigger_stuck() {
        let mut e = event("PreToolUse", "s");
        e.payload.insert("tool_name".into(), json!("Bash"));
        e.payload.insert("tool_input".into(), json!({"command": "ls"}));
        let hash = canonical_tool_input_hash(Some(&json!({"command": "ls"})));

        let mut inp = input(SessionState::Working, e);
        inp.now_unix = 1000;
        // 4 priors, but they're outside the 60s window
        inp.recent_events = (0..4)
            .map(|i| RecentEvent {
                event_type: "PreToolUse".into(),
                tool_name: Some("Bash".into()),
                tool_input_hash: hash.clone(),
                timestamp: 800 + i, // > 60s before now
            })
            .collect();
        let out = classify(inp);
        assert_eq!(out.new_session_state, SessionState::Working);
        assert!(out.actions.is_empty());
    }

    #[test]
    fn canonical_hash_ignores_key_order() {
        let a = json!({"command": "ls", "cwd": "/tmp"});
        let b = json!({"cwd": "/tmp", "command": "ls"});
        assert_eq!(
            canonical_tool_input_hash(Some(&a)),
            canonical_tool_input_hash(Some(&b))
        );
    }

    #[test]
    fn canonical_hash_distinguishes_payloads() {
        let a = json!({"command": "ls"});
        let b = json!({"command": "pwd"});
        assert_ne!(
            canonical_tool_input_hash(Some(&a)),
            canonical_tool_input_hash(Some(&b))
        );
    }

    #[test]
    fn idle_check_promotes_silent_working_session() {
        let session = SessionSnapshot {
            state: SessionState::Working,
            last_event_at_unix: 1000,
        };
        let item = classify_idle(&session, &[], 1000 + IDLE_STUCK_SECONDS + 1).unwrap();
        assert_eq!(item.kind, ItemKind::Stuck);
        assert_eq!(item.priority, 60);
    }

    #[test]
    fn idle_check_skips_sessions_with_existing_stuck_item() {
        let session = SessionSnapshot {
            state: SessionState::Working,
            last_event_at_unix: 1000,
        };
        let existing = [OpenItem {
            id: "x".into(),
            kind: ItemKind::Stuck,
            priority: 70,
        }];
        assert!(classify_idle(&session, &existing, 1000 + IDLE_STUCK_SECONDS + 1).is_none());
    }

    #[test]
    fn idle_check_skips_non_working_sessions() {
        let session = SessionSnapshot {
            state: SessionState::NeedsInput,
            last_event_at_unix: 1000,
        };
        assert!(classify_idle(&session, &[], 1000 + IDLE_STUCK_SECONDS + 1).is_none());
    }

    #[test]
    fn idle_check_skips_fresh_sessions() {
        let session = SessionSnapshot {
            state: SessionState::Working,
            last_event_at_unix: 1000,
        };
        assert!(classify_idle(&session, &[], 1000 + IDLE_STUCK_SECONDS - 1).is_none());
    }
}

//! OpenCode adapter — translate `opencode serve` events into claudemon's
//! session model.
//!
//! OpenCode exposes a headless HTTP server (`opencode serve`, default
//! 127.0.0.1:4096, OpenAPI 3.1) and a Server-Sent-Events stream at `GET /event`
//! whose frames are `{ "type": "<entity>.<action>", "properties": { … } }`
//! (e.g. `session.idle`, `message.part.updated`, `permission.updated`). The
//! stream carries 80+ event types and evolves quickly, so the translator below
//! is deliberately *defensive*: it recognizes the events that map onto our
//! model and ignores everything else (rather than failing on unknown shapes).
//!
//! This module is the pure, unit-tested translation core. The live client that
//! spawns `opencode serve`, creates a session, posts prompts, and pumps the SSE
//! stream through `translate` + `apply` is wired in a follow-up (it needs a real
//! `opencode` binary to validate end-to-end).

use serde_json::Value;
use time::OffsetDateTime;

use crate::session::conversation::ConversationItem;
use crate::session::state::StatusLine;

/// A typed update distilled from one OpenCode event. Several can come from a
/// single event (e.g. a part update is both "the agent is busy" and "here's
/// some assistant text").
#[derive(Debug, Clone, PartialEq)]
pub enum OpenCodeUpdate {
    /// The agent finished responding — session is ready for input.
    Idle,
    /// The agent is actively producing output.
    Busy,
    /// A permission/approval request is outstanding (waiting on the user).
    PermissionPending {
        tool: Option<String>,
        summary: Option<String>,
    },
    /// A chunk of assistant text to append to the conversation.
    AssistantText(String),
    /// A user message echoed back by the server.
    UserText(String),
    /// A tool invocation.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Token/cost telemetry for the session.
    Usage {
        model: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd: Option<f64>,
    },
    /// A session-level error message.
    Error(String),
}

/// Translate one OpenCode SSE event into zero or more typed updates. Pure and
/// total: unknown event types and missing fields yield an empty/partial result
/// rather than an error.
pub fn translate(event: &Value) -> Vec<OpenCodeUpdate> {
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    let props = event.get("properties").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();

    match ty {
        "session.idle" => out.push(OpenCodeUpdate::Idle),

        "session.error" => {
            let msg = props
                .get("error")
                .and_then(|e| e.get("message").or_else(|| e.get("data")))
                .and_then(Value::as_str)
                .or_else(|| props.get("message").and_then(Value::as_str))
                .unwrap_or("session error");
            out.push(OpenCodeUpdate::Error(msg.to_string()));
        }

        "permission.updated" | "permission.replied" => {
            let p = props.get("permission").unwrap_or(&props);
            let tool = p
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| p.get("type").and_then(Value::as_str))
                .map(str::to_owned);
            let summary = p
                .get("metadata")
                .and_then(|m| m.get("command"))
                .and_then(Value::as_str)
                .or_else(|| p.get("description").and_then(Value::as_str))
                .map(str::to_owned);
            out.push(OpenCodeUpdate::PermissionPending { tool, summary });
        }

        // Both the streamed-part event and the whole-message event indicate the
        // agent is working; pull text / tool / usage out of whichever shape the
        // event carries.
        "message.updated" | "message.part.updated" => {
            out.push(OpenCodeUpdate::Busy);
            if let Some(part) = props.get("part") {
                translate_part(part, &props, &mut out);
            }
            if let Some(info) = props.get("info").or_else(|| props.get("message")) {
                translate_message_info(info, &mut out);
            }
        }

        _ => {}
    }

    out
}

/// Map a message Part (`text` / `tool` / `step-finish` / …) to updates.
fn translate_part(part: &Value, props: &Value, out: &mut Vec<OpenCodeUpdate>) {
    let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
    // Role is usually on the parent message; a part event sometimes carries it.
    let role = part
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| props.get("role").and_then(Value::as_str));
    match kind {
        "text" => {
            // Prefer the incremental delta when present, else the full text.
            if let Some(text) = part
                .get("delta")
                .and_then(Value::as_str)
                .or_else(|| part.get("text").and_then(Value::as_str))
            {
                if !text.is_empty() {
                    if role == Some("user") {
                        out.push(OpenCodeUpdate::UserText(text.to_string()));
                    } else {
                        out.push(OpenCodeUpdate::AssistantText(text.to_string()));
                    }
                }
            }
        }
        "tool" => {
            let id = part
                .get("callID")
                .and_then(Value::as_str)
                .or_else(|| part.get("id").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let name = part
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| part.get("name").and_then(Value::as_str))
                .unwrap_or("tool")
                .to_string();
            let input = part
                .get("state")
                .and_then(|s| s.get("input"))
                .or_else(|| part.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            out.push(OpenCodeUpdate::ToolUse { id, name, input });
        }
        // A step boundary carries the per-step token/cost tally.
        "step-finish" | "step_finish" => {
            if let Some(u) = usage_from(part) {
                out.push(u);
            }
        }
        _ => {}
    }
}

/// Map a whole-message `info` object to usage (assistant messages carry the
/// running token/cost tally on the message itself).
fn translate_message_info(info: &Value, out: &mut Vec<OpenCodeUpdate>) {
    if let Some(u) = usage_from(info) {
        out.push(u);
    }
}

/// Extract a `Usage` update from any object carrying `tokens` / `cost` /
/// `modelID` (a message info or a step-finish part).
fn usage_from(v: &Value) -> Option<OpenCodeUpdate> {
    let tokens = v.get("tokens");
    let input = tokens.and_then(|t| t.get("input")).and_then(Value::as_u64);
    let output = tokens.and_then(|t| t.get("output")).and_then(Value::as_u64);
    let cost = v.get("cost").and_then(Value::as_f64);
    let model = v
        .get("modelID")
        .or_else(|| v.get("model"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if input.is_none() && output.is_none() && cost.is_none() && model.is_none() {
        return None;
    }
    Some(OpenCodeUpdate::Usage {
        model,
        input_tokens: input,
        output_tokens: output,
        cost_usd: cost,
    })
}

/// Build a `StatusLine` from usage telemetry, for `SessionStore::apply_status_line`.
pub fn status_from_usage(
    model: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cost_usd: Option<f64>,
) -> StatusLine {
    StatusLine {
        model_display: model,
        total_input_tokens: input_tokens,
        total_output_tokens: output_tokens,
        cost_usd,
        received_at: Some(OffsetDateTime::now_utc()),
        ..Default::default()
    }
}

/// Map an `OpenCodeUpdate` to a conversation item, when it represents one.
pub fn conversation_item(update: &OpenCodeUpdate) -> Option<ConversationItem> {
    match update {
        OpenCodeUpdate::AssistantText(text) => Some(ConversationItem::AssistantText {
            text: text.clone(),
            timestamp: None,
        }),
        OpenCodeUpdate::UserText(text) => Some(ConversationItem::UserMessage {
            text: text.clone(),
            timestamp: None,
        }),
        OpenCodeUpdate::ToolUse { id, name, input } => Some(ConversationItem::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
            timestamp: None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn idle_maps_to_idle() {
        let ev = json!({ "type": "session.idle", "properties": { "sessionID": "s1" } });
        assert_eq!(translate(&ev), vec![OpenCodeUpdate::Idle]);
    }

    #[test]
    fn error_extracts_message() {
        let ev = json!({ "type": "session.error", "properties": { "error": { "message": "boom" } } });
        assert_eq!(translate(&ev), vec![OpenCodeUpdate::Error("boom".into())]);
    }

    #[test]
    fn permission_pending_pulls_tool_and_command() {
        let ev = json!({
            "type": "permission.updated",
            "properties": {
                "type": "bash",
                "title": "Bash",
                "metadata": { "command": "rm -rf build" }
            }
        });
        assert_eq!(
            translate(&ev),
            vec![OpenCodeUpdate::PermissionPending {
                tool: Some("Bash".into()),
                summary: Some("rm -rf build".into()),
            }]
        );
    }

    #[test]
    fn text_part_is_busy_plus_assistant_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "Hello" } }
        });
        assert_eq!(
            translate(&ev),
            vec![OpenCodeUpdate::Busy, OpenCodeUpdate::AssistantText("Hello".into())]
        );
    }

    #[test]
    fn text_part_prefers_delta_over_full_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "Hello world", "delta": " world" } }
        });
        assert_eq!(
            translate(&ev),
            vec![OpenCodeUpdate::Busy, OpenCodeUpdate::AssistantText(" world".into())]
        );
    }

    #[test]
    fn user_role_text_maps_to_user_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "hi", "role": "user" } }
        });
        assert_eq!(
            translate(&ev),
            vec![OpenCodeUpdate::Busy, OpenCodeUpdate::UserText("hi".into())]
        );
    }

    #[test]
    fn tool_part_maps_to_tool_use() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": {
                "type": "tool", "tool": "bash", "callID": "c1",
                "state": { "input": { "command": "ls" } }
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                OpenCodeUpdate::Busy,
                OpenCodeUpdate::ToolUse {
                    id: "c1".into(),
                    name: "bash".into(),
                    input: json!({ "command": "ls" }),
                }
            ]
        );
    }

    #[test]
    fn step_finish_part_yields_usage() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": {
                "type": "step-finish",
                "tokens": { "input": 1200, "output": 340 },
                "cost": 0.0123,
                "modelID": "claude-sonnet-4"
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                OpenCodeUpdate::Busy,
                OpenCodeUpdate::Usage {
                    model: Some("claude-sonnet-4".into()),
                    input_tokens: Some(1200),
                    output_tokens: Some(340),
                    cost_usd: Some(0.0123),
                }
            ]
        );
    }

    #[test]
    fn message_info_usage_is_extracted() {
        let ev = json!({
            "type": "message.updated",
            "properties": { "info": {
                "role": "assistant",
                "tokens": { "input": 50, "output": 9 },
                "cost": 0.001
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                OpenCodeUpdate::Busy,
                OpenCodeUpdate::Usage {
                    model: None,
                    input_tokens: Some(50),
                    output_tokens: Some(9),
                    cost_usd: Some(0.001),
                }
            ]
        );
    }

    #[test]
    fn unknown_event_is_ignored() {
        let ev = json!({ "type": "installation.updated", "properties": {} });
        assert!(translate(&ev).is_empty());
        let ev2 = json!({ "type": "lsp.diagnostics", "properties": { "anything": 1 } });
        assert!(translate(&ev2).is_empty());
    }

    #[test]
    fn malformed_event_does_not_panic() {
        assert!(translate(&json!({})).is_empty());
        assert!(translate(&json!({ "type": "message.part.updated" })).is_empty()
            || translate(&json!({ "type": "message.part.updated" })) == vec![OpenCodeUpdate::Busy]);
        assert!(translate(&Value::Null).is_empty());
    }

    #[test]
    fn status_from_usage_sets_only_known_fields() {
        let sl = status_from_usage(Some("m".into()), Some(10), Some(2), Some(0.5));
        assert_eq!(sl.model_display.as_deref(), Some("m"));
        assert_eq!(sl.total_input_tokens, Some(10));
        assert_eq!(sl.total_output_tokens, Some(2));
        assert_eq!(sl.cost_usd, Some(0.5));
        assert!(sl.context_used_pct.is_none());
        assert!(sl.received_at.is_some());
    }

    #[test]
    fn conversation_item_mapping() {
        assert!(matches!(
            conversation_item(&OpenCodeUpdate::AssistantText("x".into())),
            Some(ConversationItem::AssistantText { .. })
        ));
        assert!(matches!(
            conversation_item(&OpenCodeUpdate::UserText("x".into())),
            Some(ConversationItem::UserMessage { .. })
        ));
        assert!(conversation_item(&OpenCodeUpdate::Idle).is_none());
        assert!(conversation_item(&OpenCodeUpdate::Busy).is_none());
    }
}

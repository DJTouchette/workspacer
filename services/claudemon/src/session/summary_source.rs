//! Fixed-budget, data-only projection. Never serializes arbitrary tool payloads.
use super::conversation::ConversationItem;
use serde::Serialize;
use std::collections::VecDeque;

pub const HEAD_MESSAGES: usize = 3;
pub const TAIL_EVENTS: usize = 24;
pub const EVENT_CHARS: usize = 800;
// Bytes are a stricter bound than JS UTF-16 characters, including JSON escaping.
pub const SOURCE_BYTES: usize = 5000;
pub const PROJECTION: &str = "agent-status-source/v1";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryEvent {
    seq: u64,
    kind: &'static str,
    text: String,
    timestamp: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummarySource {
    projection: &'static str,
    session_id: String,
    through_seq: u64,
    first_seq: u64,
    head_truncated: bool,
    tail_truncated: bool,
    text_truncated: bool,
    events: Vec<SummaryEvent>,
}

fn usable(item: &ConversationItem) -> Option<(&'static str, &str, &Option<String>)> {
    match item {
        ConversationItem::UserMessage { text, timestamp } => {
            Some(("user_message", text.as_str(), timestamp))
        }
        ConversationItem::AssistantText { text, timestamp } => {
            Some(("assistant_text", text.as_str(), timestamp))
        }
        // This tool's schema is {note, needsDecision?}; no suffix/substring matching.
        ConversationItem::ToolUse {
            name,
            input,
            timestamp,
            ..
        } if name == "report_progress" || name == "mcp__workspacer__report_progress" => input
            .get("note")
            .and_then(|v| v.as_str())
            .map(|t| ("progress", t, timestamp)),
        _ => None,
    }
    .filter(|(_, text, _)| !text.trim().is_empty())
}

pub fn project<'a>(
    session_id: &str,
    through_seq: u64,
    first_seq: u64,
    head_truncated: bool,
    items: impl Iterator<Item = (&'a ConversationItem, u64)>,
) -> SummarySource {
    let mut head = Vec::new();
    let mut tail = VecDeque::new();
    let mut count = 0;
    let mut text_truncated = false;
    for (item, seq) in items {
        let Some((kind, text, timestamp)) = usable(item) else {
            continue;
        };
        count += 1;
        let mut chars = text.trim().chars();
        let text: String = chars.by_ref().take(EVENT_CHARS).collect();
        text_truncated |= chars.next().is_some();
        let event = SummaryEvent {
            seq,
            kind,
            text,
            timestamp: timestamp.as_ref().filter(|t| t.len() <= 40).cloned(),
        };
        if kind != "progress" && head.len() < HEAD_MESSAGES {
            head.push(event.clone());
        }
        tail.push_back(event);
        if tail.len() > TAIL_EVENTS {
            tail.pop_front();
        }
    }
    let head_len = head.len();
    for event in tail {
        if !head.iter().any(|h| h.seq == event.seq) {
            head.push(event);
        }
    }
    head.sort_by_key(|e| e.seq);
    let mut source = SummarySource {
        projection: PROJECTION,
        session_id: session_id.to_owned(),
        through_seq,
        first_seq,
        head_truncated,
        tail_truncated: head.len() < count,
        text_truncated,
        events: head,
    };
    // Preserve the earliest task and newest activity. Remove oldest tail entries
    // first; when only the head remains, shrink text rather than lose the task.
    while serde_json::to_vec(&source)
        .expect("summary serializes")
        .len()
        > SOURCE_BYTES
    {
        source.tail_truncated = true;
        if source.events.len() > head_len + 1 {
            source.events.remove(head_len);
        } else if let Some(event) = source.events.iter_mut().max_by_key(|e| e.text.len()) {
            source.text_truncated = true;
            event.text = event
                .text
                .chars()
                .take(event.text.chars().count() / 2)
                .collect();
        } else {
            break;
        }
    }
    source
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn bounds_order_dedupe_and_exclusions() {
        let mut items = vec![ConversationItem::UserMessage {
            text: "task".into(),
            timestamp: None,
        }];
        for i in 0..80 {
            items.push(ConversationItem::AssistantText {
                text: format!("{i}:{}", "\"\\😀".repeat(2000)),
                timestamp: None,
            });
            items.push(ConversationItem::ToolResult {
                tool_use_id: "x".into(),
                content: "SECRET".repeat(10000),
                is_error: false,
                timestamp: None,
            });
        }
        items.push(ConversationItem::ToolUse {
            id: "p".into(),
            name: "report_progress".into(),
            input: json!({"note":"latest progress", "other":"SECRET"}),
            timestamp: None,
        });
        let s = project("s", items.len() as u64, 1, false, items.iter().zip(1..));
        let raw = serde_json::to_string(&s).unwrap();
        assert!(raw.len() <= SOURCE_BYTES);
        assert!(!raw.contains("SECRET"));
        assert_eq!(s.events[0].text, "task");
        assert_eq!(s.events.last().unwrap().text, "latest progress");
        assert!(s.events.len() <= HEAD_MESSAGES + TAIL_EVENTS);
        assert!(s.events.windows(2).all(|w| w[0].seq < w[1].seq));
        assert!(s
            .events
            .iter()
            .all(|e| e.text.chars().count() <= EVENT_CHARS));
        assert!(s.tail_truncated && s.text_truncated);
    }
    #[test]
    fn small_source_deduplicates_and_empty_is_explicit() {
        let items = [ConversationItem::UserMessage {
            text: "task".into(),
            timestamp: None,
        }];
        let s = project("s", 1, 1, true, items.iter().zip(1..));
        assert_eq!(s.events.len(), 1);
        assert!(s.head_truncated);
        assert!(!s.tail_truncated);
        let empty = project("s", 0, 0, false, std::iter::empty());
        assert!(empty.events.is_empty());
        assert_eq!(empty.projection, PROJECTION);
    }
}

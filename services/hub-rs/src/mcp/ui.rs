//! Event-backed UI actions retain the original fire-and-forget contract.
use crate::protocol::Event;
use serde_json::{Value, json};
pub(super) fn topic(name: &str) -> Option<&'static str> {
    Some(match name {
        "focus_agent" => "command.focus_agent",
        "open_pane" | "open_browser" => "command.open_pane",
        "open_plugin" => "command.open_plugin",
        "open_spawn_dialog" => "command.open_spawn_dialog",
        "open_guide" => "command.open_guide",
        "run_ui_action" => "command.run_action",
        _ => return None,
    })
}
pub(super) fn event(name: &str, params: &Value) -> Option<Event> {
    let topic = topic(name)?;
    let mut data = json!({});
    let fields: &[&str] = match name {
        "focus_agent" => &["sessionId"],
        "open_pane" => &["paneType", "cwd", "url"],
        "open_browser" => &["url"],
        "open_plugin" => &["type"],
        "open_spawn_dialog" => &["cwd"],
        "run_ui_action" => &["action", "digit"],
        _ => &[],
    };
    for key in fields {
        if let Some(value) = params.get(*key) {
            if matches!(*key, "cwd" | "url") && value == "" && name != "open_browser" {
                continue;
            }
            if *key == "digit" && value == 0 {
                continue;
            }
            data[*key] = value.clone();
        }
    }
    if name == "open_browser" {
        data["paneType"] = "browser".into();
    }
    Some(Event::new(topic, "mcp-facade", data))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_topics_and_payloads_do_not_forward_spoofed_envelopes() {
        let event=event("open_browser",&json!({"url":"https://example.test","topic":"agent.dispatch.update","source":"forged","hub":"forged"})).unwrap();
        assert_eq!(event.topic, "command.open_pane");
        assert_eq!(event.source, "mcp-facade");
        assert_eq!(
            event.data,
            Some(json!({"paneType":"browser","url":"https://example.test"}))
        );
        assert!(event.hub.is_empty());
        assert_eq!(
            self::event("open_guide", &json!({"source":"forged"}))
                .unwrap()
                .data,
            Some(json!({}))
        );
        assert!(self::event("unknown", &json!({})).is_none());
    }
}

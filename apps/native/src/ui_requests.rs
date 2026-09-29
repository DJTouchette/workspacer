//! Bounded, display-only bus intents. A controller is also used headlessly, so
//! receiving an event must never create a process or pretend a pane is visible.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
pub const TOPICS: &[&str] = &[
    "facade.openTerminal",
    "command.focus_agent",
    "command.open_pane",
    "command.open_spawn_dialog",
    "command.open_guide",
    "command.run_action",
    "command.open_plugin",
];
pub const MAX_PENDING: usize = 32;
pub const MAX_BYTES: usize = 64 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    FocusAgent(String),
    OpenPane {
        pane_type: String,
        cwd: String,
        url: String,
    },
    OpenSpawnDialog {
        cwd: String,
    },
    OpenGuide,
    RunAction {
        action: String,
        digit: Option<u8>,
    },
    OpenPlugin(String),
    Terminal {
        cwd: String,
        command: String,
        label: String,
        parent_session_id: String,
    },
}
#[derive(Clone, Debug)]
pub struct Request {
    pub number: u64,
    pub intent: Intent,
    pub payload: Value,
}
fn string(data: &Value, key: &str) -> Result<String> {
    match data.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(v) => {
            let text = v
                .as_str()
                .with_context(|| format!("UI request {key} must be text"))?;
            let limit = match key {
                "command" => MAX_BYTES,
                "cwd" | "url" => 16 * 1024,
                "action" | "paneType" | "type" => 128,
                _ => 512,
            };
            if text.len() > limit {
                bail!("UI request {key} is too long");
            }
            Ok(text.to_string())
        }
    }
}
pub fn parse(topic: &str, data: &Value) -> Result<Option<(Intent, Value)>> {
    if !TOPICS.contains(&topic) {
        return Ok(None);
    }
    if !data.is_null() && !data.is_object() {
        bail!("UI request must be an object");
    }
    if serde_json::to_vec(data)?.len() > MAX_BYTES {
        bail!("UI request exceeds 64 KiB");
    }
    let (intent, keys) = match topic {
        "facade.openTerminal" => (
            Intent::Terminal {
                cwd: string(data, "cwd")?,
                command: string(data, "command")?,
                label: string(data, "label")?,
                parent_session_id: string(data, "parentSessionId")?,
            },
            vec!["cwd", "command", "label", "parentSessionId"],
        ),
        "command.focus_agent" => {
            let id = string(data, "agentId")?;
            let id = if id.is_empty() {
                string(data, "sessionId")?
            } else {
                id
            };
            if id.is_empty() || id.len() > 256 {
                bail!("UI focus request requires a session ID");
            }
            (Intent::FocusAgent(id), vec!["agentId", "sessionId"])
        }
        "command.open_pane" => (
            Intent::OpenPane {
                pane_type: string(data, "paneType")?,
                cwd: string(data, "cwd")?,
                url: string(data, "url")?,
            },
            vec!["paneType", "cwd", "url"],
        ),
        "command.open_spawn_dialog" => (
            Intent::OpenSpawnDialog {
                cwd: string(data, "cwd")?,
            },
            vec!["cwd"],
        ),
        "command.open_guide" => (Intent::OpenGuide, vec![]),
        "command.open_plugin" => (Intent::OpenPlugin(string(data, "type")?), vec!["type"]),
        "command.run_action" => {
            let digit = match data.get("digit") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let n = v
                        .as_u64()
                        .filter(|n| (1..=9).contains(n))
                        .context("UI action digit must be 1 through 9")?;
                    Some(n as u8)
                }
            };
            (
                Intent::RunAction {
                    action: string(data, "action")?,
                    digit,
                },
                vec!["action", "digit"],
            )
        }
        _ => unreachable!(),
    };
    let mut payload = json!({});
    for key in keys {
        if let Some(value) = data.get(key) {
            payload[key] = value.clone();
        }
    }
    Ok(Some((intent, payload)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_metadata_is_retained_as_data_never_a_spawn_request() {
        let data = json!({"cwd":"/workspace","command":"echo hello\n","label":"checks","parentSessionId":"manager","shell":"spoofed-executable"});
        let (intent, payload) = parse("facade.openTerminal", &data).unwrap().unwrap();
        assert_eq!(
            intent,
            Intent::Terminal {
                cwd: "/workspace".into(),
                command: "echo hello\n".into(),
                label: "checks".into(),
                parent_session_id: "manager".into()
            }
        );
        assert!(payload.get("shell").is_none());
        assert_eq!(payload["command"], "echo hello\n");
    }
    #[test]
    fn bounded_typed_intents_do_not_accept_privileged_action_envelopes() {
        assert!(parse("command.focus_agent", &json!({"sessionId":4})).is_err());
        assert!(
            parse(
                "command.run_action",
                &json!({"action":"jump-tab","digit":99})
            )
            .is_err()
        );
        assert!(
            parse(
                "facade.openTerminal",
                &json!({"command":"x".repeat(MAX_BYTES)})
            )
            .is_err()
        );
        assert!(parse("command.approve", &json!({})).unwrap().is_none());
        let (_, p) = parse(
            "command.open_spawn_dialog",
            &json!({"cwd":"/project","message":"do work","skipPermissions":true}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(p, json!({"cwd":"/project"}));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    FocusAgent(String),
    Conversation,
    Settings,
    Recent,
    Changes(String),
    Inspector,
    Spawn { cwd: String, claude: bool },
    PreviousAgent,
    NextAgent,
    NextAttention,
    Unsupported(String),
}
pub fn effect(intent: &Intent) -> Effect {
    match intent {
  Intent::FocusAgent(id)=>Effect::FocusAgent(id.clone()),
  Intent::OpenSpawnDialog{cwd}=>Effect::Spawn{cwd:cwd.clone(),claude:false},
  Intent::OpenPane{pane_type,cwd,..}=>match pane_type.as_str(){
   "claude"=>Effect::Spawn{cwd:cwd.clone(),claude:true},"settings"=>Effect::Settings,
   "sessions"|"recentagents"=>Effect::Recent,"review"=>Effect::Changes(cwd.clone()),
   "agents"|"agentwatch"=>Effect::Conversation,"inspector"=>Effect::Inspector,
   "terminal"=>Effect::Unsupported("Terminal panes are unavailable in the native client. No shell or command was started; use a client with a terminal pane.".into()),
   other=>Effect::Unsupported(format!("The native client does not support the {other} pane.")),
  },
  Intent::Terminal{..}=>Effect::Unsupported("Terminal panes are unavailable in the native client. No shell or command was started; copy the request to retain its command and project details.".into()),
  Intent::OpenGuide=>Effect::Unsupported("The native client has no Workspacer Guide pane. Read native guide opens documentation for its supported controls.".into()),
  Intent::OpenPlugin(name)=>Effect::Unsupported(format!("The native client cannot render plugin pane {name}.")),
  Intent::RunAction{action,..}=>match action.as_str(){
   "prev-agent"=>Effect::PreviousAgent,"next-agent"=>Effect::NextAgent,"next-attention"=>Effect::NextAttention,
   "spawn-agent"=>Effect::Spawn{cwd:String::new(),claude:false},"new-claude"=>Effect::Spawn{cwd:String::new(),claude:true},
   "settings"=>Effect::Settings,"open-review"=>Effect::Changes(String::new()),"toggle-inspector"=>Effect::Inspector,
   "new-terminal"|"toggle-terminal"=>Effect::Unsupported("Terminal panes are unavailable in the native client. No shell was started.".into()),
   "toggle-help"=>effect(&Intent::OpenGuide),
   other=>Effect::Unsupported(format!("The native client does not apply the {other} UI action. Decision and session-control actions require their scoped controls.")),
  },
 }
}

#[cfg(test)]
mod effect_tests {
    use super::*;
    #[test]
    fn mappings_use_existing_views_and_never_treat_decisions_as_navigation() {
        assert_eq!(
            effect(&Intent::OpenSpawnDialog {
                cwd: "/repo".into()
            }),
            Effect::Spawn {
                cwd: "/repo".into(),
                claude: false
            }
        );
        assert_eq!(
            effect(&Intent::OpenPane {
                pane_type: "review".into(),
                cwd: "/different".into(),
                url: String::new()
            }),
            Effect::Changes("/different".into())
        );
        for action in [
            "fleet-approve-yes",
            "fleet-approve-no",
            "fleet-answer",
            "close-pane",
            "save-session",
            "move-tab",
            "run-shell",
        ] {
            assert!(matches!(
                effect(&Intent::RunAction {
                    action: action.into(),
                    digit: Some(2)
                }),
                Effect::Unsupported(_)
            ));
        }
        for pane in [
            "terminal",
            "browser",
            "editor",
            "plugin",
            "board",
            "analytics",
            "library",
        ] {
            assert!(matches!(
                effect(&Intent::OpenPane {
                    pane_type: pane.into(),
                    cwd: String::new(),
                    url: String::new()
                }),
                Effect::Unsupported(_)
            ));
        }
    }
}

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
    "notify.post",
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
    /// An agent's `notify` call, or a hub service's own notice (a proposed
    /// or failed job).
    Notify {
        title: String,
        body: String,
        /// The session a click should open, when the notice is about one.
        session_id: String,
        /// Shown in the window only: `silent` or `inAppOnly` was set.
        quiet: bool,
    },
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
                "body" => 4 * 1024,
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
        "notify.post" => {
            let flag = |key: &str| match data.get(key) {
                None | Some(Value::Null) => Ok(false),
                Some(Value::Bool(flag)) => Ok(*flag),
                Some(_) => Err(anyhow::anyhow!("Notification {key} must be true or false")),
            };
            let (title, body) = (string(data, "title")?, string(data, "body")?);
            if title.trim().is_empty() && body.trim().is_empty() {
                bail!("Notification has no text");
            }
            (
                Intent::Notify {
                    title,
                    body,
                    session_id: string(data, "sessionId")?,
                    quiet: flag("silent")? || flag("inAppOnly")?,
                },
                vec!["title", "body", "sessionId", "level", "url"],
            )
        }
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
    fn a_notification_is_shown_never_applied_as_navigation() {
        let data = json!({"title":"Job failed: nightly","body":"exit 2","level":"error","key":"job-1","sessionId":"abc","paneType":"settings","inAppOnly":true});
        let (intent, payload) = parse("notify.post", &data).unwrap().unwrap();
        assert_eq!(
            intent,
            Intent::Notify {
                title: "Job failed: nightly".into(),
                body: "exit 2".into(),
                session_id: "abc".into(),
                quiet: true
            }
        );
        assert!(payload.get("paneType").is_none());
        assert!(matches!(
            effect(&intent),
            Effect::Notify { quiet: true, .. }
        ));
        assert!(parse("notify.post", &json!({"title":" ","body":""})).is_err());
        assert!(parse("notify.post", &json!({"title":"x","silent":"yes"})).is_err());
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
    Spawn {
        cwd: String,
        claude: bool,
    },
    PreviousAgent,
    NextAgent,
    NextAttention,
    /// The selected agent's own terminal: shown (or toggled), never running
    /// a requested command.
    Terminal {
        toggle: bool,
    },
    /// A notice to show in the window, and as an OS notification while the
    /// window is in the background unless it is `quiet`.
    Notify {
        title: String,
        body: String,
        session: String,
        quiet: bool,
    },
    /// A command the hub asks to run in a terminal. Receiving it starts
    /// nothing: the window shows it and the person runs it, or does not.
    OfferCommand(TerminalRequest),
    /// A web page the hub asks to show. Offered, and opened in the system
    /// browser only on the person's click: a request alone never browses.
    OfferPage(String),
    /// Account usage, the native counterpart of the analytics pane.
    Usage,
    Unsupported(String),
}

/// `facade.openTerminal`: a command for a terminal in `cwd`, on behalf of
/// `parent_session_id` when it names a session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalRequest {
    pub cwd: String,
    pub command: String,
    pub label: String,
    pub parent_session_id: String,
}

/// What running a [`TerminalRequest`] types into a session's shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalInput {
    pub bytes: Vec<u8>,
    /// The text ends with Enter, so the shell runs it; otherwise it is left
    /// at the prompt for the person to finish.
    pub runs: bool,
    /// Why the text was left at the prompt although the request ended a line.
    pub held: Option<&'static str>,
}

impl TerminalRequest {
    /// The command as text a terminal can take: line ends normalised and
    /// every other control character dropped, so a request cannot smuggle an
    /// escape sequence (ending a bracketed paste, retitling the window) into
    /// the shell behind the command the person read.
    pub fn text(&self) -> String {
        self.command
            .replace("\r\n", "\n")
            .chars()
            .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
            .collect()
    }

    /// One line for the offer: the first line, shortened, and how many more.
    pub fn preview(&self) -> String {
        let text = self.text();
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let first = lines.next().unwrap_or_default().trim();
        let more = lines.count();
        let mut shown: String = first.chars().take(120).collect();
        if shown.len() < first.len() {
            shown.push('…');
        }
        match more {
            0 => shown,
            1 => format!("{shown} (+1 line)"),
            n => format!("{shown} (+{n} lines)"),
        }
    }

    /// What to type into a shell whose folder is `session_cwd`.
    ///
    /// The shell belongs to the session, so a request for another folder
    /// changes to it first, and only runs the command if that worked. That
    /// is one POSIX line (`cd -- '…' && command`); where it cannot be
    /// written safely (several lines, or a Windows path, whose shells do not
    /// share one quoting) the text is typed and left for the person to run.
    pub fn input(&self, session_cwd: &str) -> TerminalInput {
        let text = self.text();
        let wants_run = text.ends_with('\n');
        let body = text.trim_end_matches('\n');
        let same_folder = |a: &str, b: &str| {
            let trim = |p: &'_ str| p.trim_end_matches(['/', '\\']).to_owned();
            trim(a) == trim(b)
        };
        let elsewhere = !self.cwd.is_empty() && !same_folder(&self.cwd, session_cwd);
        let (typed, held) = if !elsewhere || body.is_empty() && !self.cwd.starts_with('/') {
            (body.to_owned(), None)
        } else if !self.cwd.starts_with('/') {
            (
                body.to_owned(),
                Some("It asks for another folder; change to it before running."),
            )
        } else {
            let cd = format!("cd -- '{}'", self.cwd.replace('\'', "'\\''"));
            if body.is_empty() {
                (cd, None)
            } else if body.contains('\n') {
                (
                    format!("{cd}\n{body}"),
                    Some("It has several lines for another folder; check them before running."),
                )
            } else {
                (format!("{cd} && {body}"), None)
            }
        };
        // A held text is typed as a paste would be, without its final Enter.
        let runs = wants_run && held.is_none() && !typed.is_empty();
        let mut bytes = typed.replace('\n', "\r").into_bytes();
        if held.is_some() {
            // Several lines must not run one by one either: only the first
            // is typed up to its end.
            if let Some(end) = bytes.iter().position(|b| *b == b'\r') {
                bytes.truncate(end);
            }
        }
        if runs {
            bytes.push(b'\r');
        }
        TerminalInput { bytes, runs, held }
    }
}
/// Only plain web addresses are offered to the browser: a request cannot
/// hand the system opener a file, a script or another application's scheme.
fn web_page(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|rest| {
        !rest.is_empty() && !rest.starts_with('/') && !url.chars().any(char::is_whitespace)
    })
}

pub fn effect(intent: &Intent) -> Effect {
    match intent {
  Intent::FocusAgent(id)=>Effect::FocusAgent(id.clone()),
  Intent::OpenSpawnDialog{cwd}=>Effect::Spawn{cwd:cwd.clone(),claude:false},
  Intent::OpenPane{pane_type,cwd,url}=>match pane_type.as_str(){
   "claude"=>Effect::Spawn{cwd:cwd.clone(),claude:true},"settings"=>Effect::Settings,
   "sessions"|"recentagents"=>Effect::Recent,"review"=>Effect::Changes(cwd.clone()),
   "inspector"=>Effect::Inspector,
   "agents"|"agentwatch"|"overview"=>Effect::Conversation,
   // A terminal pane is the selected agent's own shell here.
   "terminal"=>Effect::Terminal{toggle:false},
   "browser" if web_page(url)=>Effect::OfferPage(url.clone()),
   "browser"=>Effect::Unsupported("The native client has no browser pane, and the request named no web page to open instead.".into()),
   "analytics"=>Effect::Usage,
   other=>Effect::Unsupported(format!("The native client does not support the {other} pane.")),
  },
  Intent::Terminal{cwd,command,label,parent_session_id}=>Effect::OfferCommand(TerminalRequest{cwd:cwd.clone(),command:command.clone(),label:label.clone(),parent_session_id:parent_session_id.clone()}),
  Intent::Notify{title,body,session_id,quiet}=>Effect::Notify{title:title.clone(),body:body.clone(),session:session_id.clone(),quiet:*quiet},
  Intent::OpenGuide=>Effect::Unsupported("The native client has no Workspacer Guide pane. Read native guide opens documentation for its supported controls.".into()),
  Intent::OpenPlugin(name)=>Effect::Unsupported(format!("The native client cannot render plugin pane {name}.")),
  Intent::RunAction{action,..}=>match action.as_str(){
   "prev-agent"=>Effect::PreviousAgent,"next-agent"=>Effect::NextAgent,"next-attention"=>Effect::NextAttention,
   "spawn-agent"=>Effect::Spawn{cwd:String::new(),claude:false},"new-claude"=>Effect::Spawn{cwd:String::new(),claude:true},
   "settings"=>Effect::Settings,"open-review"=>Effect::Changes(String::new()),"toggle-inspector"=>Effect::Inspector,
   "new-terminal"=>Effect::Terminal{toggle:false},"toggle-terminal"=>Effect::Terminal{toggle:true},
   "toggle-help"=>effect(&Intent::OpenGuide),
   other=>Effect::Unsupported(format!("The native client does not apply the {other} UI action. Decision and session-control actions require their scoped controls.")),
  },
 }
}

#[cfg(test)]
mod effect_tests {
    use super::*;

    fn pane(pane_type: &str, url: &str) -> Effect {
        effect(&Intent::OpenPane {
            pane_type: pane_type.into(),
            cwd: String::new(),
            url: url.into(),
        })
    }

    #[test]
    fn panes_with_a_native_counterpart_open_it_and_pages_are_only_offered() {
        assert_eq!(pane("terminal", ""), Effect::Terminal { toggle: false });
        assert_eq!(pane("overview", ""), Effect::Conversation);
        assert_eq!(pane("analytics", ""), Effect::Usage);
        assert_eq!(
            pane("browser", "https://example.com/docs?q=1"),
            Effect::OfferPage("https://example.com/docs?q=1".into())
        );
        for url in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https:///nohost",
            "http://a b",
            "zed://open",
        ] {
            assert!(
                matches!(pane("browser", url), Effect::Unsupported(_)),
                "{url}"
            );
        }
    }

    fn request(cwd: &str, command: &str) -> TerminalRequest {
        TerminalRequest {
            cwd: cwd.into(),
            command: command.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_terminal_request_is_an_offer_and_types_only_what_was_shown() {
        let offered = effect(&Intent::Terminal {
            cwd: "/repo".into(),
            command: "cargo test\n".into(),
            label: "Checks".into(),
            parent_session_id: "manager".into(),
        });
        let Effect::OfferCommand(offered) = offered else {
            panic!("a terminal request must be offered, not applied");
        };
        assert_eq!(offered.preview(), "cargo test");

        // In the session's own folder the command is typed as sent: Enter
        // only where the request ended the line.
        let run = offered.input("/repo/");
        assert_eq!(
            (run.bytes.as_slice(), run.runs),
            (&b"cargo test\r"[..], true)
        );
        let typed = request("", "git status").input("/repo");
        assert_eq!(
            (typed.bytes.as_slice(), typed.runs),
            (&b"git status"[..], false)
        );

        // Another folder: change to it first, and run only if that worked.
        let moved = request("/it's here", "make\n").input("/repo");
        assert_eq!(moved.bytes, b"cd -- '/it'\\''s here' && make\r");
        assert!(moved.runs);
        // Only a folder: open the terminal there.
        assert_eq!(
            request("/other", "").input("/repo").bytes,
            b"cd -- '/other'"
        );

        // Where that cannot be one safe line, nothing runs by itself.
        let lines = request("/other", "rm -rf build\nmake\n").input("/repo");
        assert_eq!(lines.bytes, b"cd -- '/other'");
        assert!(!lines.runs && lines.held.is_some());
        let windows = request("C:\\work", "del /q build\r\n").input("D:\\repo");
        assert_eq!(windows.bytes, b"del /q build");
        assert!(!windows.runs && windows.held.is_some());

        // Control characters never reach the shell; previews count lines.
        let sneaky = request("", "echo ok\x1b[201~\x07\nsecond\n\nthird\n");
        assert_eq!(sneaky.text(), "echo ok[201~\nsecond\n\nthird\n");
        assert_eq!(sneaky.preview(), "echo ok[201~ (+2 lines)");
        assert_eq!(sneaky.input("").bytes, b"echo ok[201~\rsecond\r\rthird\r");
    }

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
        assert_eq!(
            effect(&Intent::RunAction {
                action: "toggle-terminal".into(),
                digit: None
            }),
            Effect::Terminal { toggle: true }
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
            "browser", "editor", "plugin", "board", "library", "ask", "context",
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

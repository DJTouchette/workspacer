//! "Continue with…": hand a session's work to a new agent of the other
//! provider. The owning hub writes a brief (`claude.handoffAgentBrief`, the
//! source agent's own, with its mechanical fallback; or `claude.handoffBrief`,
//! the mechanical digest) and the successor starts through the ordinary
//! `agents.spawn` path in the source's exact folder. The takeover message is
//! staged in the successor's composer for review, never sent for the user.
//!
//! A handoff is a new agent reading a file: it does not move the provider
//! session, its private context or a Fleet Manager's role, and it never stops
//! or rewrites the source.
use anyhow::{Result, bail, ensure};
use serde_json::Value;

use crate::{controller::NewSession, launch::Permission, model::Session};

/// Who writes the brief.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Brief {
    /// The source agent writes it (takes one turn); the hub falls back to the
    /// mechanical digest when it cannot or does not within its deadline.
    #[default]
    Agent,
    /// The hub's deterministic digest of the retained conversation.
    Mechanical,
}

impl Brief {
    pub fn method(self) -> &'static str {
        match self {
            Self::Agent => "claude.handoffAgentBrief",
            Self::Mechanical => "claude.handoffBrief",
        }
    }
}

/// The provider a session can continue with, or why it cannot. Native
/// launches admit Claude and Codex only, so each continues with the other.
pub fn target(session: &Session) -> Result<&'static str> {
    if session.wake_target {
        bail!(
            "Fleet Manager sessions are replaced through manager replacement, which moves their \
             workers and journal. An ordinary handoff would not, so it is not offered here."
        );
    }
    let target = match session.provider.as_str() {
        "claude" | "" => "codex",
        "codex" => "claude",
        _ => bail!("Continue with… is available for Claude and Codex sessions."),
    };
    ensure!(
        crate::launch::absolute_directory(&session.cwd),
        "This session has no project folder to continue in."
    );
    Ok(target)
}

pub fn provider_name(provider: &str) -> &'static str {
    if provider == "codex" {
        "Codex"
    } else {
        "Claude"
    }
}

/// Carry the source's access to the successor without ever widening it.
/// Bypass in either vocabulary stays full access (the same intent); a mode
/// the target does not offer, an unknown or live-only mode, or no report at
/// all falls back to asking.
pub fn carry_permission(target: &str, source_mode: &str) -> Permission {
    let carried = match source_mode.trim() {
        "bypassPermissions" | "yolo" => Permission::FullAccess,
        "acceptEdits" => Permission::AcceptEdits,
        "plan" => Permission::Plan,
        _ => Permission::Ask,
    };
    if Permission::choices(target).contains(&carried) {
        carried
    } else {
        Permission::Ask
    }
}

/// The takeover message staged in the successor's composer. Keeps the
/// desktop's wording (and its `handoff brief at ` phrase) so every client
/// hands off the same way.
pub fn successor_prompt(path: &str) -> String {
    format!(
        "You are taking over an in-progress session from another AI coding agent. \
         First read the handoff brief at {path}, then continue the work from where it left off — \
         don't start over or redo completed steps. Reply with a one-paragraph summary of the \
         state and your next step."
    )
}

/// A brief the hub reported as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// On the hub's machine; the successor runs there too.
    pub path: String,
    /// Why the source agent's own brief was replaced by the mechanical digest.
    pub fallback: Option<String>,
}

/// Read a brief reply. Anything short of `ok` with a path is a failure:
/// a successor is never started without a brief to read.
pub fn written(reply: &Value) -> Result<Written> {
    if reply["ok"] == false {
        bail!(
            "{}",
            reply["error"]
                .as_str()
                .filter(|e| !e.is_empty())
                .unwrap_or("The hub could not write the handoff brief")
        );
    }
    let Some(path) = reply["path"]
        .as_str()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        bail!("The hub wrote no handoff brief file");
    };
    ensure!(
        crate::launch::absolute_directory(path) && !path.contains('\n'),
        "The hub returned an unusable brief path"
    );
    Ok(Written {
        path: path.to_owned(),
        fallback: (reply["fallback"] == true).then(|| {
            reply["error"]
                .as_str()
                .filter(|e| !e.is_empty())
                .unwrap_or("The source agent did not write the brief")
                .to_owned()
        }),
    })
}

/// One requested handoff: the source and the successor's launch settings.
#[derive(Clone, Debug)]
pub struct Request {
    pub source: String,
    pub brief: Brief,
    pub successor: NewSession,
}

impl Request {
    /// Check the request against the source as the controller knows it.
    /// The successor must start in the source's exact folder, on the provider
    /// it continues with, and with no message sent on the user's behalf.
    pub fn validate(&self, source: &Session) -> Result<()> {
        let target = target(source)?;
        ensure!(
            self.successor.provider == target,
            "{} sessions continue with {}",
            provider_name(&source.provider),
            provider_name(target)
        );
        ensure!(
            self.successor.cwd == source.cwd,
            "The new agent must start in the same folder as this session"
        );
        ensure!(
            self.successor.message.trim().is_empty() && self.successor.resume_session_id.is_none(),
            "A handoff starts a fresh agent and stages its first message for review"
        );
        ensure!(
            self.brief == Brief::Mechanical || !source.stopped(),
            "An ended agent cannot write its own brief; use the quick summary"
        );
        self.successor.params().map(|_| ())
    }
}

/// Where a handoff is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Brief(Brief),
    Starting,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub number: u64,
    pub source: String,
    pub provider: String,
    pub stage: Stage,
}

/// A finished handoff, successful or not, for the window that asked.
#[derive(Clone, Debug)]
pub struct Receipt {
    pub number: u64,
    pub source: String,
    pub provider: String,
    pub successor: Option<String>,
    pub brief: Option<Written>,
    pub error: Option<String>,
}

impl Receipt {
    /// What the user is told: honest about a fallback brief and about a
    /// brief left behind when the successor could not start.
    pub fn summary(&self) -> String {
        let name = provider_name(&self.provider);
        let fallback = self
            .brief
            .as_ref()
            .and_then(|b| b.fallback.as_ref())
            .map(|why| {
                format!(
                    " The source agent did not write the brief ({}), so the hub's mechanical \
                     summary of its conversation was used instead.",
                    why.trim_end_matches('.')
                )
            })
            .unwrap_or_default();
        match (&self.successor, &self.error) {
            (Some(_), _) => format!(
                "{name} started in the same folder. Review the handoff message in the composer, \
                 then send it. The original session is unchanged.{fallback}"
            ),
            (None, Some(error)) => match &self.brief {
                Some(brief) => format!(
                    "The brief was written to {}, but {name} could not be started: {error}{}",
                    brief.path,
                    if crate::launch::uncertain_outcome(error) {
                        " Check your sessions before trying again."
                    } else {
                        ""
                    }
                ),
                None => format!("Could not prepare the handoff brief: {error}"),
            },
            (None, None) => format!("{name} was not started."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(provider: &str) -> Session {
        Session {
            id: "src".into(),
            provider: provider.into(),
            cwd: "/work/repo-wt".into(),
            state: "idle".into(),
            ..Default::default()
        }
    }

    fn request(source: &Session, provider: &str) -> Request {
        Request {
            source: source.id.clone(),
            brief: Brief::Agent,
            successor: NewSession {
                provider: provider.into(),
                cwd: source.cwd.clone(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn each_provider_continues_with_the_other_and_managers_are_gated() {
        assert_eq!(target(&session("claude")).unwrap(), "codex");
        assert_eq!(target(&session("")).unwrap(), "codex");
        assert_eq!(target(&session("codex")).unwrap(), "claude");
        assert!(target(&session("opencode")).is_err());
        let mut manager = session("claude");
        manager.wake_target = true;
        assert!(
            target(&manager)
                .unwrap_err()
                .to_string()
                .contains("manager replacement")
        );
        let mut folderless = session("codex");
        folderless.cwd = "relative".into();
        assert!(target(&folderless).is_err());
    }

    #[test]
    fn manager_flag_is_read_from_snapshots() {
        let mut s = Session::default();
        s.merge(&json!({"sessionId":"m","isWakeTarget":true,"settings":{"permissionMode":"plan"}}));
        assert!(s.wake_target);
        assert_eq!(s.permission_mode, "plan");
        s.merge(&json!({"livePermissionMode":"bypassPermissions"}));
        assert_eq!(s.permission_mode, "bypassPermissions");
        // A later delta without the flag keeps it.
        s.merge(&json!({"mode":"idle"}));
        assert!(s.wake_target);
    }

    #[test]
    fn access_is_carried_and_never_widened() {
        assert_eq!(
            carry_permission("codex", "bypassPermissions"),
            Permission::FullAccess
        );
        assert_eq!(carry_permission("claude", "yolo"), Permission::FullAccess);
        assert_eq!(carry_permission("codex", "acceptEdits"), Permission::Ask);
        assert_eq!(carry_permission("codex", "plan"), Permission::Ask);
        assert_eq!(carry_permission("claude", "plan"), Permission::Plan);
        assert_eq!(carry_permission("claude", "ask"), Permission::Ask);
        assert_eq!(carry_permission("claude", ""), Permission::Ask);
        assert_eq!(carry_permission("claude", "dontAsk"), Permission::Ask);
        assert_eq!(carry_permission("claude", "auto"), Permission::Ask);
    }

    #[test]
    fn brief_replies_are_success_only_with_an_absolute_path() {
        let ok = written(&json!({"ok":true,"path":"/h/.workspacer/handoffs/a.md"})).unwrap();
        assert_eq!(ok.path, "/h/.workspacer/handoffs/a.md");
        assert_eq!(ok.fallback, None);
        let fell = written(&json!({"ok":true,"path":"/h/b.md","fallback":true,
            "error":"Source agent did not write the brief before the deadline"}))
        .unwrap();
        assert!(fell.fallback.unwrap().contains("deadline"));
        assert!(written(&json!({"ok":false,"error":"mechanical fallback also failed"})).is_err());
        assert!(written(&json!({"ok":true,"path":null})).is_err());
        assert!(written(&json!({"ok":true,"path":""})).is_err());
        assert!(written(&json!({"ok":true,"path":"relative.md"})).is_err());
        // Hub fallback that produced no file reports ok:false with a reason.
        let err =
            written(&json!({"ok":false,"path":null,"fallback":true,"error":"x"})).unwrap_err();
        assert_eq!(err.to_string(), "x");
    }

    #[test]
    fn requests_stay_in_the_source_folder_and_send_nothing() {
        let source = session("claude");
        assert!(request(&source, "codex").validate(&source).is_ok());
        assert!(request(&source, "claude").validate(&source).is_err());
        let mut moved = request(&source, "codex");
        moved.successor.cwd = "/work/repo".into();
        assert!(moved.validate(&source).is_err());
        let mut sent = request(&source, "codex");
        sent.successor.message = "go".into();
        assert!(sent.validate(&source).is_err());
        let mut stopped = source.clone();
        stopped.state = "stopped".into();
        assert!(request(&stopped, "codex").validate(&stopped).is_err());
        let mut quick = request(&stopped, "codex");
        quick.brief = Brief::Mechanical;
        assert!(quick.validate(&stopped).is_ok());
    }

    #[test]
    fn prompt_names_the_brief_and_summaries_are_honest() {
        let prompt = successor_prompt("/h/.workspacer/handoffs/a.md");
        assert!(prompt.contains("handoff brief at /h/.workspacer/handoffs/a.md"));
        let brief = Written {
            path: "/h/a.md".into(),
            fallback: Some("Source agent could not accept the brief request".into()),
        };
        let done = Receipt {
            number: 1,
            source: "src".into(),
            provider: "codex".into(),
            successor: Some("new".into()),
            brief: Some(brief.clone()),
            error: None,
        };
        let text = done.summary();
        assert!(text.contains("Codex started") && text.contains("mechanical summary"));
        let orphaned = Receipt {
            successor: None,
            error: Some("launch admission may have executed".into()),
            ..done.clone()
        };
        let text = orphaned.summary();
        assert!(text.contains("/h/a.md") && text.contains("Check your sessions"));
        let unprepared = Receipt {
            brief: None,
            successor: None,
            error: Some("hub disconnected".into()),
            ..done
        };
        assert!(unprepared.summary().starts_with("Could not prepare"));
    }
}

//! "Continue with…": hand a session's work to a new agent of the other
//! provider, or "Start fresh from a summary" with a new agent of the same one.
//! The owning hub writes a brief (`claude.handoffAgentBrief`, the source
//! agent's own, with its mechanical fallback; `claude.handoffSummaryBrief`, a
//! cheap model's summary, with the same fallback; or `claude.handoffBrief`,
//! the mechanical digest) and the successor starts through the ordinary
//! `agents.spawn` path in the source's exact folder. The takeover message is
//! staged in the successor's composer for review, never sent for the user.
//!
//! Only the agent tier touches the source (it takes one turn). The summary
//! tier exists for a session whose prompt cache has gone cold: resuming it
//! just to describe itself would re-read its whole context at full price.
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
    /// A cheap model (the hub's title harness, Haiku for Claude) summarizes
    /// the deterministic digest and the conversation's tail. Nothing is sent
    /// to the source; the hub falls back to the mechanical digest on failure.
    Summary,
    /// The hub's deterministic digest of the retained conversation.
    Mechanical,
}

impl Brief {
    pub fn method(self) -> &'static str {
        match self {
            Self::Agent => "claude.handoffAgentBrief",
            Self::Summary => "claude.handoffSummaryBrief",
            Self::Mechanical => "claude.handoffBrief",
        }
    }

    /// Who writes it, for a fallback notice.
    fn author(self) -> &'static str {
        match self {
            Self::Agent => "The source agent",
            Self::Summary => "The summary model",
            Self::Mechanical => "The hub",
        }
    }
}

/// The other provider a session can continue with, or why it cannot. Native
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

/// Every provider a session can hand off to: the other one first ("Continue
/// with…"), then its own ("Start fresh from a summary": a new agent with an
/// empty context, for a session too long or too cold to resume cheaply).
pub fn targets(session: &Session) -> Result<[&'static str; 2]> {
    let other = target(session)?;
    Ok([other, same(session)])
}

/// The session's own provider, as a launch target.
pub fn same(session: &Session) -> &'static str {
    if session.provider == "codex" {
        "codex"
    } else {
        "claude"
    }
}

/// Whether `provider` starts the source's own kind of agent afresh.
pub fn fresh(session: &Session, provider: &str) -> bool {
    provider == same(session)
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
    /// Why the requested brief was replaced by the mechanical digest.
    pub fallback: Option<String>,
    /// The brief that was asked for.
    pub kind: Brief,
}

/// Read a brief reply. Anything short of `ok` with a path is a failure:
/// a successor is never started without a brief to read.
pub fn written(reply: &Value, kind: Brief) -> Result<Written> {
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
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{} did not write the brief", kind.author()))
        }),
        kind,
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
        let [other, same] = targets(source)?;
        ensure!(
            self.successor.provider == other || self.successor.provider == same,
            "{} sessions continue with {} or start fresh with {}",
            provider_name(same),
            provider_name(other),
            provider_name(same)
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
            self.brief != Brief::Agent || !source.stopped(),
            "An ended agent cannot write its own brief; use a summary"
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
            .and_then(|b| Some((b.kind, b.fallback.as_ref()?)))
            .map(|(kind, why)| {
                format!(
                    " {} did not write the brief ({}), so the hub's mechanical summary of its \
                     conversation was used instead.",
                    kind.author(),
                    why.trim_end_matches('.')
                )
            })
            .unwrap_or_default();
        // The lead says which kind of success it was; the window's notice
        // tone keys on it (a fallback brief stays a warning).
        let lead = if fallback.is_empty() {
            "Handoff ready"
        } else {
            "Handoff ready with a fallback brief"
        };
        match (&self.successor, &self.error) {
            (Some(_), _) => format!(
                "{lead}: {name} started in the same folder. Review the handoff message in the \
                 composer, then send it. The original session stays available with its \
                 history.{fallback}"
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
        let written = |reply: Value| super::written(&reply, Brief::Agent);
        let ok = written(json!({"ok":true,"path":"/h/.workspacer/handoffs/a.md"})).unwrap();
        assert_eq!(ok.path, "/h/.workspacer/handoffs/a.md");
        assert_eq!(ok.fallback, None);
        assert_eq!(ok.kind, Brief::Agent);
        let fell = written(json!({"ok":true,"path":"/h/b.md","fallback":true,
            "error":"Source agent did not write the brief before the deadline"}))
        .unwrap();
        assert!(fell.fallback.unwrap().contains("deadline"));
        assert!(written(json!({"ok":false,"error":"mechanical fallback also failed"})).is_err());
        assert!(written(json!({"ok":true,"path":null})).is_err());
        assert!(written(json!({"ok":true,"path":""})).is_err());
        assert!(written(json!({"ok":true,"path":"relative.md"})).is_err());
        // Hub fallback that produced no file reports ok:false with a reason.
        let err = written(json!({"ok":false,"path":null,"fallback":true,"error":"x"})).unwrap_err();
        assert_eq!(err.to_string(), "x");
        // A reasonless fallback names who did not write it.
        let summary = super::written(
            &json!({"ok":true,"path":"/h/c.md","fallback":true}),
            Brief::Summary,
        )
        .unwrap();
        assert_eq!(
            summary.fallback.as_deref(),
            Some("The summary model did not write the brief")
        );
    }

    #[test]
    fn requests_stay_in_the_source_folder_and_send_nothing() {
        let source = session("claude");
        assert!(request(&source, "codex").validate(&source).is_ok());
        // Starting fresh keeps the provider; nothing else is admitted.
        assert!(request(&source, "claude").validate(&source).is_ok());
        assert!(request(&source, "opencode").validate(&source).is_err());
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
        // An ended agent's summary is written without it, in either direction.
        for provider in ["codex", "claude"] {
            let mut summary = request(&stopped, provider);
            summary.brief = Brief::Summary;
            assert!(summary.validate(&stopped).is_ok(), "{provider}");
        }
        let mut fresh_agent = request(&stopped, "claude");
        fresh_agent.brief = Brief::Agent;
        assert!(fresh_agent.validate(&stopped).is_err());
    }

    #[test]
    fn summary_brief_has_its_own_method_and_same_provider_targets() {
        assert_eq!(Brief::Summary.method(), "claude.handoffSummaryBrief");
        assert_eq!(Brief::Mechanical.method(), "claude.handoffBrief");
        assert_eq!(Brief::Agent.method(), "claude.handoffAgentBrief");
        let mut stopped = session("claude");
        stopped.state = "stopped".into();
        assert_eq!(targets(&stopped).unwrap(), ["codex", "claude"]);
        assert_eq!(targets(&session("codex")).unwrap(), ["claude", "codex"]);
        assert_eq!(targets(&session("")).unwrap(), ["codex", "claude"]);
        assert!(fresh(&stopped, "claude") && !fresh(&stopped, "codex"));
        let mut manager = stopped.clone();
        manager.wake_target = true;
        assert!(targets(&manager).is_err(), "managers are never handed off");
        assert!(targets(&session("opencode")).is_err());
    }

    #[test]
    fn prompt_names_the_brief_and_summaries_are_honest() {
        let prompt = successor_prompt("/h/.workspacer/handoffs/a.md");
        assert!(prompt.contains("handoff brief at /h/.workspacer/handoffs/a.md"));
        let brief = Written {
            path: "/h/a.md".into(),
            fallback: Some("Source agent could not accept the brief request".into()),
            kind: Brief::Agent,
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
        assert!(text.starts_with("Handoff ready with a fallback brief: Codex started"));
        assert!(text.contains("The source agent did not write the brief"));
        assert!(text.contains("mechanical summary"));
        let summarized = Receipt {
            brief: Some(Written {
                fallback: Some("The summary model did not answer in time".into()),
                kind: Brief::Summary,
                ..brief.clone()
            }),
            ..done.clone()
        }
        .summary();
        assert!(summarized.starts_with("Handoff ready with a fallback brief"));
        assert!(summarized.contains("The summary model did not write the brief"));
        let clean = Receipt {
            brief: Some(Written {
                fallback: None,
                ..brief.clone()
            }),
            ..done.clone()
        }
        .summary();
        assert!(clean.starts_with("Handoff ready: Codex started"));
        // An authored brief takes a turn in the source: never claim it is untouched.
        assert!(text.contains("stays available with its history") && !text.contains("unchanged"));
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

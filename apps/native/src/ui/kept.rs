//! Paused sessions, from the hub's shared open set (`sessionOpen`). The hub
//! keeps that set for every client: a session stays open (and in the roster,
//! however long ago it stopped) until it is hidden, which here means
//! archived. One the hub saw end on purpose (by you or by its agent) is also
//! marked `ended` and reads Ended; one stopped by the app or host closing is
//! not, and reads Paused. This device keeps no list of its own and reads
//! nothing back by id. See docs/session-roster.md.
use super::*;
use wks_native::child_agents::Delegated;

/// Prefix of the notice a failed resume leaves on the conversation.
pub(super) const RESUME_FAILED: &str = "Could not resume this session: ";

impl Workspace {
    /// Stopped while open and not ended on purpose (the hub's open set lists
    /// it without an `ended` mark): shown as paused and resumable from its
    /// composer. A hub without the shared open set, or one whose set has not
    /// been read yet, pauses nothing.
    pub(super) fn paused(&self, session: &Session) -> bool {
        session.stopped()
            && matches!(session.provider.as_str(), "claude" | "codex" | "")
            && self
                .view
                .session_open
                .as_ref()
                .is_some_and(|open| wks_native::features::open_paused(open, &session.id))
    }

    /// The session's status label, with an open session that stopped (the
    /// app's or host's closing) reading as paused rather than ended.
    /// The session ended a turn no client of this hub has opened since, and
    /// is not busy again (a decision or a new turn outranks it).
    pub(super) fn finished_unseen(&self, session: &Session) -> bool {
        !session.busy()
            && !session.starting()
            && self
                .view
                .session_unseen
                .as_ref()
                .is_some_and(|doc| wks_native::features::unseen_since(doc, &session.id).is_some())
    }

    pub(super) fn status_of<'a>(&self, session: &'a Session, p: Palette) -> (&'a str, u32) {
        let delegated = self.delegated(session);
        // Agents still at work outrank Finished: the turn that ended only
        // handed the work off, and their results wake this session again. A
        // background shell can outlive every turn, so it never hides one.
        if delegated == Some(Delegated::Agents) {
            (Delegated::Agents.label(), p.busy)
        } else if self.view.connected && self.finished_unseen(session) {
            ("Finished", p.success)
        } else if self.paused(session) {
            ("Paused", p.accent)
        } else if let Some(delegated) = delegated {
            (delegated.label(), p.busy)
        } else {
            session_status(session, p)
        }
    }

    /// What a session that is not mid-turn is still waiting on: agents it
    /// handed work to, or background tasks. Nothing while offline, when the
    /// fleet is only the last one seen.
    pub(super) fn delegated(&self, session: &Session) -> Option<Delegated> {
        self.view
            .connected
            .then(|| wks_native::child_agents::delegated(session, &self.view.sessions))
            .flatten()
    }

    /// The session shows the working loader: mid-turn itself, or waiting on
    /// agents doing its work.
    pub(super) fn shows_working(&self, session: &Session) -> bool {
        self.view.connected
            && (session.working() || self.delegated(session) == Some(Delegated::Agents))
    }

    /// Resume a paused session with `text` as its next message: the same
    /// session continues (same id, same conversation) on its provider, model
    /// and access. Errors come back on the spawn receipt.
    pub(super) fn resume_with(&mut self, session: &Session, text: String, cx: &mut Context<Self>) {
        let provider = if session.provider == "codex" {
            "codex"
        } else {
            "claude"
        };
        let permission = match session.access_mode() {
            "bypassPermissions" | "yolo" | "full" => wks_native::launch::Permission::FullAccess,
            "acceptEdits" if provider == "claude" => wks_native::launch::Permission::AcceptEdits,
            "plan" if provider == "claude" => wks_native::launch::Permission::Plan,
            _ => wks_native::launch::Permission::Ask,
        };
        let request = NewSession {
            provider: provider.into(),
            cwd: session.cwd.clone(),
            label: session.label.clone(),
            model: session.model.clone(),
            message: text,
            context_window: None,
            permission,
            effort: session.effort.clone(),
            resume_session_id: Some(session.id.clone()),
        };
        match request
            .params()
            .and_then(|_| self.controller.command(Command::Create(request)))
        {
            Ok(()) => self.resuming = Some(session.id.clone()),
            Err(error) => self.extras.notice = format!("{RESUME_FAILED}{error}"),
        }
        cx.notify();
    }
}

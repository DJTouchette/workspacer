//! Paused sessions, from the hub's shared open set (`sessionOpen`). The hub
//! keeps that set for every client: a session stays open (and in the roster,
//! however long ago it stopped) until it is hidden, which here means
//! archived. One the hub saw end on purpose (by you or by its agent) is also
//! marked `ended` and reads Ended; one stopped by the app or host closing is
//! not, and reads Paused. This device keeps no list of its own and reads
//! nothing back by id. See docs/session-roster.md.
use super::*;

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
    pub(super) fn status_of<'a>(&self, session: &'a Session, p: Palette) -> (&'a str, u32) {
        if self.paused(session) {
            ("Paused", p.accent)
        } else {
            session_status(session, p)
        }
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

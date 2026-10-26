//! Sessions that were open when the app closed stay in the sidebar, with their
//! last content, until they are picked back up, archived, or end while the app
//! is running. The policy lives in `wks_native::kept`; this applies it to the
//! window's settings and hands the controller the ids to read.
use super::*;

/// Prefix of the notice a failed resume leaves on the conversation.
pub(super) const RESUME_FAILED: &str = "Could not resume this session: ";

impl Workspace {
    /// Fold one view into the kept set (see [`wks_native::kept::fold`]).
    pub(super) fn sync_kept_open(&mut self, view: &View, cx: &mut Context<Self>) {
        if !view.connected || view.sessions_loading && view.sessions.is_empty() {
            return;
        }
        let scope = self.project_scope.clone();
        let mut kept = self
            .settings
            .kept_open
            .get(&scope)
            .cloned()
            .unwrap_or_default();
        let before = kept.clone();
        let mut seen_open = std::mem::take(&mut self.seen_open);
        wks_native::kept::fold(
            &mut kept,
            &view.sessions,
            &mut seen_open,
            &view.kept_gone,
            |id| self.archived(id),
            // Only a hub that outlives the app can stop a session after it.
            self.last_closed.filter(|_| !self.extras.local_paths),
            timing::now_ms(),
        );
        self.seen_open = seen_open;
        if kept != before {
            if kept.is_empty() {
                self.settings.kept_open.remove(&scope);
            } else {
                self.settings.kept_open.insert(scope, kept.clone());
            }
            self.save_settings(cx);
        }
        // The controller only needs the list once the fleet read leaves one
        // out; after that it follows every change so it never reads a session
        // this device has let go.
        let ids = wks_native::kept::read_order(&kept);
        // Only a session carried over from an earlier run can need reading by
        // id; one seen open in this run that leaves the list is the hub's call.
        let missing = ids
            .iter()
            .any(|id| !self.seen_open.contains(id) && !view.sessions.iter().any(|s| &s.id == id));
        if ids != self.kept_sent && (missing || !self.kept_sent.is_empty()) {
            self.kept_sent = ids.clone();
            let _ = self.controller.command(Command::KeepSessions(ids));
        }
    }

    /// Stopped by the app closing rather than ended: shown as paused and
    /// resumable from its composer.
    pub(super) fn paused(&self, session: &Session) -> bool {
        session.stopped()
            && matches!(session.provider.as_str(), "claude" | "codex" | "")
            && self
                .settings
                .kept_open
                .get(&self.project_scope)
                .is_some_and(|kept| kept.contains_key(&session.id))
    }

    /// The session's status label, with a kept session that the app's closing
    /// stopped reading as paused rather than ended.
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

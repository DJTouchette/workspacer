//! Sessions that were open when the app closed stay in the sidebar, with their
//! last content, until they are picked back up, archived, or end while the app
//! is running. The hub's fleet list ages stopped sessions out after a day; this
//! device remembers which ones it was in the middle of (`Settings::kept_open`)
//! and has the controller read those by id.
use super::*;

/// Prefix of the notice a failed resume leaves on the conversation.
pub(super) const RESUME_FAILED: &str = "Could not resume this session: ";

impl Workspace {
    /// Fold one view into the kept set. A session seen open is kept; one that
    /// ends while this run watched it was ended on purpose (by you or by its
    /// agent) and is let go; one that is already stopped when first seen this
    /// run was stopped by the app closing, and stays.
    pub(super) fn sync_kept_open(&mut self, view: &View, cx: &mut Context<Self>) {
        if !view.connected || view.sessions_loading && view.sessions.is_empty() {
            return;
        }
        let now = timing::now_ms();
        let scope = self.project_scope.clone();
        let mut kept = self
            .settings
            .kept_open
            .get(&scope)
            .cloned()
            .unwrap_or_default();
        let before = kept.clone();
        for session in view.sessions.iter() {
            if session.stopped() {
                if self.seen_open.contains(&session.id) {
                    kept.remove(&session.id);
                }
            } else {
                self.seen_open.insert(session.id.clone());
                kept.entry(session.id.clone()).or_insert(now);
            }
        }
        kept.retain(|id, _| !self.archived(id));
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
        let ids: Vec<String> = kept.into_keys().collect();
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
        let permission = match session.permission_mode.as_str() {
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

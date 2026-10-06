//! Archiving: a visibility change only, shared through the hub's archive
//! where it has one and kept on this device where it does not.
use super::*;
use wks_native::features::Request;

impl Workspace {
    /// Hidden from the normal list. The hub's shared archive (so web and
    /// every other client agree) plus any device-only archive not yet moved
    /// there; an unconfirmed change shows as already made.
    pub(super) fn archived(&self, id: &str) -> bool {
        if let Some(&pending) = self.extras.archive_pending.get(id) {
            return pending;
        }
        self.locally_archived(id)
            || self
                .view
                .session_archive
                .as_ref()
                .is_some_and(|doc| wks_native::features::archive_contains(doc, id))
    }

    fn locally_archived(&self, id: &str) -> bool {
        self.settings
            .archived
            .get(&self.project_scope)
            .is_some_and(|ids| ids.iter().any(|s| s == id))
    }

    /// The connected hub keeps a shared archive (older hubs do not).
    fn hub_archive(&self) -> bool {
        self.view.connected && !self.demo && self.view.session_archive.is_some()
    }

    pub(super) fn archive_loading(&self) -> bool {
        !self.demo
            && self.view.connected
            && self.view.session_archive.is_none()
            && self
                .view
                .requests
                .get("archive")
                .is_some_and(|request| request.loading)
    }

    fn send_archive(&mut self, id: &str, archived: bool) {
        self.extras.archive_pending.insert(id.into(), archived);
        if self.extras.archive_inflight.contains_key(id) {
            return;
        }
        match self
            .controller
            .command(Command::Request(Request::SetArchive {
                session: id.into(),
                archived,
            })) {
            Ok(()) => {
                self.extras.archive_inflight.insert(id.into(), archived);
            }
            Err(error) => {
                self.extras.archive_pending.remove(id);
                self.extras.notice = error.to_string();
            }
        }
    }

    /// Settle sent archive changes and move device-only archives to the hub,
    /// so a session archived here before archives were shared hides on the
    /// web too. A device copy is dropped only once the hub holds it.
    pub(super) fn sync_archive(&mut self, next: &View, cx: &mut Context<Self>) {
        if !next.connected {
            // The backend fences old-connection receipts. Keep no optimistic
            // choice stranded forever waiting for one; reconnect reads truth.
            self.extras.archive_pending.clear();
            self.extras.archive_inflight.clear();
            self.extras.archive_migrating.clear();
        }
        let mut followups = Vec::new();
        for receipt in next
            .archive_receipts
            .iter()
            .filter(|r| r.number > self.extras.archive_receipt)
        {
            if self.extras.archive_inflight.get(&receipt.session) != Some(&receipt.archived) {
                continue;
            }
            self.extras.archive_inflight.remove(&receipt.session);
            if let Some(error) = &receipt.error {
                self.extras.archive_pending.remove(&receipt.session);
                let verb = if receipt.archived {
                    "archive"
                } else {
                    "restore"
                };
                self.extras.notice = format!("Could not {verb} the session: {error}");
            } else if let Some(&desired) = self.extras.archive_pending.get(&receipt.session) {
                if desired != receipt.archived {
                    followups.push((receipt.session.clone(), desired));
                } else {
                    self.extras.archive_pending.remove(&receipt.session);
                }
            }
        }
        for (id, archived) in followups {
            self.send_archive(&id, archived);
        }
        if let Some(last) = next.archive_receipts.back() {
            self.extras.archive_receipt = self.extras.archive_receipt.max(last.number);
        }
        let Some(doc) = next.session_archive.clone() else {
            return;
        };
        if !next.connected || self.demo {
            return;
        }
        let local = self
            .settings
            .archived
            .get(&self.project_scope)
            .cloned()
            .unwrap_or_default();
        let mut moved = false;
        for id in local {
            if wks_native::features::archive_contains(&doc, &id) {
                if let Some(ids) = self.settings.archived.get_mut(&self.project_scope) {
                    ids.retain(|s| s != &id);
                }
                moved = true;
            } else if !self.extras.archive_pending.contains_key(&id)
                && self.extras.archive_migrating.insert(id.clone())
            {
                self.send_archive(&id, true);
            }
        }
        if moved {
            self.settings.archived.retain(|_, ids| !ids.is_empty());
            self.save_settings(cx);
        }
    }

    /// Archive or restore. Only ever a visibility change: no stop, signal,
    /// selection change or forget is sent, and a running session keeps
    /// running. Shared through the hub when it supports it; otherwise kept on
    /// this device for this connection, as before.
    pub(super) fn toggle_archive(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.view.connected && self.view.session_archive.is_some() {
            self.extras.notice = "Reconnect to archive or restore this session.".into();
            cx.notify();
            return;
        }
        let archive = !self.archived(id);
        if !archive && self.locally_archived(id) {
            if let Some(ids) = self.settings.archived.get_mut(&self.project_scope) {
                ids.retain(|s| s != id);
            }
            self.settings.archived.retain(|_, ids| !ids.is_empty());
            self.save_settings(cx);
        }
        let shared = !archive
            && self
                .view
                .session_archive
                .as_ref()
                .is_some_and(|doc| wks_native::features::archive_contains(doc, id));
        if self.hub_archive() && (archive || shared || self.extras.archive_pending.contains_key(id))
        {
            self.extras.notice.clear();
            self.send_archive(id, archive);
        } else if archive {
            self.settings
                .archived
                .entry(self.project_scope.clone())
                .or_default()
                .push(id.into());
            self.save_settings(cx);
        }
        cx.notify();
    }
}
